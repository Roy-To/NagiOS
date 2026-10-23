//! Side-effect-free host contract for first-party automation drafts.
//! Admission is descriptive and never confers Platform execution authority.

pub const MAX_ACTIONS: usize = 16;
pub const MAX_CONDITIONS: usize = 16;
pub const MAX_RESOURCES: usize = 64;
pub const MAX_PERMISSIONS: usize = 32;
pub const MAX_IDENTIFIER_BYTES: usize = 128;
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

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_BYTES
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"._:-/".contains(&c))
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
    Pending,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunStatus {
    Success,
    Partial,
    Failed,
    Skipped,
    Pending,
}

/// Summarize final per-step outcomes, never attempts. A caller supplies at most
/// one outcome per step; this is not a durable Activity/history implementation.
pub fn summarize_run(expected_steps: usize, outcomes: &[StepOutcome]) -> RunStatus {
    if expected_steps > MAX_ACTIONS || outcomes.len() > expected_steps {
        return RunStatus::Failed;
    }
    if expected_steps == 0 {
        return RunStatus::Skipped;
    }
    let succeeded = outcomes
        .iter()
        .filter(|o| **o == StepOutcome::Succeeded)
        .count();
    if outcomes.contains(&StepOutcome::Failed) {
        return if succeeded > 0 {
            RunStatus::Partial
        } else {
            RunStatus::Failed
        };
    }
    if outcomes.len() < expected_steps || outcomes.contains(&StepOutcome::Pending) {
        return RunStatus::Pending;
    }
    if succeeded == expected_steps {
        RunStatus::Success
    } else if succeeded > 0 {
        RunStatus::Partial
    } else {
        RunStatus::Skipped
    }
}
