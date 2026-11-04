//! Side-effect-free host contract for first-party automation drafts.
//! Admission is descriptive and never confers Platform execution authority.

pub const MAX_ACTIONS: usize = 16;
pub const MAX_CONDITIONS: usize = 16;
pub const MAX_RESOURCES: usize = 64;
pub const MAX_PERMISSIONS: usize = 32;
pub const MAX_IDENTIFIER_BYTES: usize = 128;
pub const MAX_JOB_IDENTIFIER_BYTES: usize = 96;
pub const MAX_CONDITION_VALUE_BYTES: usize = 1024;
pub const MAX_RETRIES: u8 = 3;

/// Opaque Action and Scope values belong to the existing Platform adapters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Draft<A, S> {
    pub enabled: bool,
    pub trigger: Trigger,
    pub conditions: Vec<Condition>,
    pub actions: Vec<A>,
    pub scope: S,
    pub error_policy: ErrorPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Trigger {
    Manual,
    Scheduled { unix_seconds: u64 },
    Interval { seconds: u64 },
    PlatformEvent { event: String },
    ResourceChange { resource: u64 },
    DeviceState { device: String, state: String },
}

/// Opaque predicates, not evaluated or assumed satisfied by this crate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Condition {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePolicy {
    Stop,
    ContinueSafeSteps,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ErrorPolicy {
    pub failure: FailurePolicy,
    /// Retries after the first attempt, never interpreted as unbounded.
    pub retries: u8,
    pub notify_user: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Effects {
    Local,
    External,
    Destructive,
    ExternalAndDestructive,
}

impl Effects {
    pub fn requires_confirmation(self) -> bool {
        self != Self::Local
    }
}

/// Authoritative metadata returned by the trusted Platform adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionInfo {
    pub action_id: String,
    pub effects: Effects,
    pub permissions: Vec<String>,
    pub resources: Vec<u64>,
    /// The adapter verified an explicit limited scope for this invocation.
    pub scope_restricted: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundaryError {
    UnknownAction,
    InvalidInput,
    Denied,
    Unavailable,
}

/// Read-only integration seam, deliberately without an executor hook.
///
/// Implementations must consult existing Platform registration, validate the
/// complete Action payload and scope for an authenticated caller, and reject
/// denied/unknown input. Metadata must come from trusted Platform descriptors,
/// not from claims in the draft. No capability acquisition or dispatch occurs.
pub trait ActionBoundary<A, S> {
    fn inspect(&self, action: &A, scope: &S) -> Result<ActionInfo, BoundaryError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationError {
    ActionCount,
    ConditionCount,
    InvalidTrigger,
    InvalidCondition { index: usize },
    RetryLimit,
    Action { index: usize, reason: BoundaryError },
    InvalidMetadata { index: usize },
    ScopeRestrictionRequired { index: usize },
}

/// Borrowing the draft keeps all Trigger/Conditions/Scope/ErrorPolicy visible.
#[derive(Debug)]
pub struct Preview<'a, A, S> {
    pub draft: &'a Draft<A, S>,
    pub actions: Vec<ActionInfo>,
    pub confirmation_required: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentifierError {
    Empty,
    TooLong,
    InvalidStart,
    InvalidCharacter,
}

/// An opaque reference to one approval-bound run, not an automation identity.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ApprovalRunRef(String);

impl ApprovalRunRef {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_core_identifier(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A per-run step identity using the existing 128-byte, slash-capable core
/// identifier grammar.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct StepId(String);

impl StepId {
    pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
        let value = value.into();
        validate_core_identifier(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Explicitly validated identifier for the standalone Job scheduler boundary.
/// This validates only; it does not compose, truncate, or normalize core IDs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulerJobId(String);

impl SchedulerJobId {
    pub fn new(value: &str) -> Result<Self, IdentifierError> {
        if value.is_empty() {
            return Err(IdentifierError::Empty);
        }
        if value.len() > MAX_JOB_IDENTIFIER_BYTES {
            return Err(IdentifierError::TooLong);
        }
        let bytes = value.as_bytes();
        if !bytes[0].is_ascii_alphanumeric() {
            return Err(IdentifierError::InvalidStart);
        }
        if bytes
            .iter()
            .any(|byte| !(byte.is_ascii_alphanumeric() || b"._:-".contains(byte)))
        {
            return Err(IdentifierError::InvalidCharacter);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_core_identifier(value: &str) -> Result<(), IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::Empty);
    }
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(IdentifierError::TooLong);
    }
    if !value
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || b"._:-/".contains(&c))
    {
        return Err(IdentifierError::InvalidCharacter);
    }
    Ok(())
}

fn identifier(value: &str) -> bool {
    validate_core_identifier(value).is_ok()
}

fn validate_shape<A, S>(draft: &Draft<A, S>) -> Result<(), ValidationError> {
    if draft.actions.is_empty() || draft.actions.len() > MAX_ACTIONS {
        return Err(ValidationError::ActionCount);
    }
    if draft.conditions.len() > MAX_CONDITIONS {
        return Err(ValidationError::ConditionCount);
    }
    if draft.error_policy.retries > MAX_RETRIES {
        return Err(ValidationError::RetryLimit);
    }
    let valid_trigger = match &draft.trigger {
        Trigger::Interval { seconds } => *seconds > 0,
        Trigger::PlatformEvent { event } => identifier(event),
        Trigger::DeviceState { device, state } => identifier(device) && identifier(state),
        _ => true,
    };
    if !valid_trigger {
        return Err(ValidationError::InvalidTrigger);
    }
    for (index, condition) in draft.conditions.iter().enumerate() {
        if !identifier(&condition.key)
            || condition.value.trim().is_empty()
            || condition.value.len() > MAX_CONDITION_VALUE_BYTES
            || condition.value.chars().any(char::is_control)
        {
            return Err(ValidationError::InvalidCondition { index });
        }
    }
    Ok(())
}

/// Validate a draft and describe its effects without executing anything.
/// A successful preview is not an authority grant and must not be cached as one.
pub fn preview<'a, A, S, B: ActionBoundary<A, S>>(
    draft: &'a Draft<A, S>,
    boundary: &B,
) -> Result<Preview<'a, A, S>, ValidationError> {
    validate_shape(draft)?;
    let mut actions = Vec::with_capacity(draft.actions.len());
    let mut confirmation_required = false;
    for (index, action) in draft.actions.iter().enumerate() {
        let info = boundary
            .inspect(action, &draft.scope)
            .map_err(|reason| ValidationError::Action { index, reason })?;
        if !identifier(&info.action_id)
            || info.permissions.len() > MAX_PERMISSIONS
            || info.resources.len() > MAX_RESOURCES
            || info.permissions.iter().any(|p| !identifier(p))
        {
            return Err(ValidationError::InvalidMetadata { index });
        }
        if info.effects.requires_confirmation() {
            if !info.scope_restricted {
                return Err(ValidationError::ScopeRestrictionRequired { index });
            }
            confirmation_required = true;
        }
        actions.push(info);
    }
    Ok(Preview {
        draft,
        actions,
        confirmation_required,
    })
}

/// Descriptive states only; none can be submitted to an Executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Admission {
    Disabled,
    PendingConfirmation,
    ReadyForPlatformValidation,
}

/// Check a user's run-once request. Disabled requests do not touch the adapter.
/// The runtime still must evaluate conditions and perform fresh deterministic
/// Platform validation, live permission checks and confirmation before dispatch.
pub fn request_run_once<A, S, B: ActionBoundary<A, S>>(
    draft: &Draft<A, S>,
    boundary: &B,
) -> Result<Admission, ValidationError> {
    if !draft.enabled {
        return Ok(Admission::Disabled);
    }
    let report = preview(draft, boundary)?;
    Ok(if report.confirmation_required {
        Admission::PendingConfirmation
    } else {
        Admission::ReadyForPlatformValidation
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepOutcome {
    Succeeded,
    Failed,
    Skipped,
    /// The step has no final result yet.
    Pending,
    Cancelled,
    TimedOut,
    RecoveryRequired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StepResult {
    pub approval_run_ref: ApprovalRunRef,
    pub step_id: StepId,
    pub outcome: StepOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunStatus {
    Success,
    Partial,
    Failed,
    Skipped,
    Pending,
    Cancelled,
    TimedOut,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SummaryError {
    TooManyExpectedSteps,
    DuplicateExpectedStep { index: usize },
    WrongApprovalRunRef { index: usize },
    UnregisteredStep { index: usize },
    DuplicateStepResult { index: usize },
    OutOfOrderStepResult { index: usize },
}

/// Summarize final per-step results for one approval-bound run.
///
/// Every returned result must identify the same approval run and exactly match
/// the expected step sequence. Identity, membership, duplicate, and ordering
/// errors are checked before outcome aggregation, even if a result is
/// RecoveryRequired. Once the result structure is valid, any RecoveryRequired
/// result takes precedence over all other outcomes, Pending, and missing
/// trailing results. Without RecoveryRequired, a missing trailing result or
/// any Pending result keeps the run Pending. This describes outcomes only and
/// is not durable deduplication or an Activity/history implementation.
pub fn summarize_run(
    approval_run_ref: &ApprovalRunRef,
    expected_steps: &[StepId],
    results: &[StepResult],
) -> Result<RunStatus, SummaryError> {
    if expected_steps.len() > MAX_ACTIONS {
        return Err(SummaryError::TooManyExpectedSteps);
    }
    for (index, step_id) in expected_steps.iter().enumerate() {
        if expected_steps[..index].contains(step_id) {
            return Err(SummaryError::DuplicateExpectedStep { index });
        }
    }
    for (index, result) in results.iter().enumerate() {
        if result.approval_run_ref != *approval_run_ref {
            return Err(SummaryError::WrongApprovalRunRef { index });
        }
        if !expected_steps.contains(&result.step_id) {
            return Err(SummaryError::UnregisteredStep { index });
        }
        if results[..index]
            .iter()
            .any(|previous| previous.step_id == result.step_id)
        {
            return Err(SummaryError::DuplicateStepResult { index });
        }
        if expected_steps.get(index) != Some(&result.step_id) {
            return Err(SummaryError::OutOfOrderStepResult { index });
        }
    }

    if expected_steps.is_empty() {
        return Ok(RunStatus::Skipped);
    }

    // RecoveryRequired is a run-level safety barrier. Preserve it even when
    // another step succeeded, failed, was cancelled, or is still pending.
    if results
        .iter()
        .any(|result| result.outcome == StepOutcome::RecoveryRequired)
    {
        return Ok(RunStatus::RecoveryRequired);
    }

    // Without a recovery barrier, unfinished ordinary work remains Pending.
    if results.len() < expected_steps.len()
        || results
            .iter()
            .any(|result| result.outcome == StepOutcome::Pending)
    {
        return Ok(RunStatus::Pending);
    }

    let succeeded = results
        .iter()
        .filter(|result| result.outcome == StepOutcome::Succeeded)
        .count();
    if succeeded == expected_steps.len() {
        return Ok(RunStatus::Success);
    }
    if succeeded > 0 {
        // Includes completed Succeeded + Skipped runs; skipped work is not a
        // successful result for every expected step.
        return Ok(RunStatus::Partial);
    }

    let mut failure_status = None;
    let mut mixed_failure_statuses = false;
    for result in results {
        let status = match result.outcome {
            StepOutcome::Failed => Some(RunStatus::Failed),
            StepOutcome::Cancelled => Some(RunStatus::Cancelled),
            StepOutcome::TimedOut => Some(RunStatus::TimedOut),
            StepOutcome::Succeeded
            | StepOutcome::Skipped
            | StepOutcome::Pending
            | StepOutcome::RecoveryRequired => None,
        };
        if let Some(status) = status {
            if failure_status.is_some_and(|previous| previous != status) {
                mixed_failure_statuses = true;
            } else {
                failure_status = Some(status);
            }
        }
    }

    if mixed_failure_statuses {
        Ok(RunStatus::Failed)
    } else if let Some(status) = failure_status {
        Ok(status)
    } else {
        Ok(RunStatus::Skipped)
    }
}
