use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

use crate::authorization::{JobOwner, OwnerProfileRef, PrincipalRef};
use crate::diagnostics::ProgressSummary;

pub const JOB_RECORD_SCHEMA_VERSION: u16 = 1;
pub const TRIGGER_SCHEMA_VERSION: u16 = 1;
const MAX_ID_BYTES: usize = 96;
const MAX_DEPENDENCIES: usize = 64;
const MAX_ATTEMPTS: u8 = 16;

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct TimestampMillis(pub u64);

impl TimestampMillis {
    pub const fn get(self) -> u64 {
        self.0
    }

    pub const fn saturating_add(self, duration: DurationMillis) -> Self {
        Self(self.0.saturating_add(duration.0))
    }
}

#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct DurationMillis(pub u64);

impl DurationMillis {
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct JobId(String);

impl JobId {
    pub fn new(value: impl Into<String>) -> Result<Self, ConstraintError> {
        let value = value.into();
        validate_token(&value, MAX_ID_BYTES, "job id")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for JobId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct OpaqueRef(String);

impl OpaqueRef {
    pub fn new(value: impl Into<String>) -> Result<Self, ConstraintError> {
        let value = value.into();
        validate_token(&value, MAX_ID_BYTES, "opaque reference")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for OpaqueRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ProgressCode(String);

impl ProgressCode {
    pub fn new(value: impl Into<String>) -> Result<Self, ConstraintError> {
        let value = value.into();
        validate_token(&value, 48, "progress code")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ProgressCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

macro_rules! opaque_ref_type {
    ($name:ident, $label:literal) => {
        #[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ConstraintError> {
                let value = value.into();
                validate_token(&value, MAX_ID_BYTES, $label)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(serde::de::Error::custom)
            }
        }
    };
}

opaque_ref_type!(CheckpointRef, "checkpoint reference");
opaque_ref_type!(ScheduleRef, "schedule reference");
opaque_ref_type!(IdempotencyKey, "idempotency key");

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HandlerRef {
    provider: OpaqueRef,
    name: OpaqueRef,
    version: u16,
}

impl HandlerRef {
    pub fn new(
        provider: OpaqueRef,
        name: OpaqueRef,
        version: u16,
    ) -> Result<Self, ConstraintError> {
        if version == 0 {
            return Err(ConstraintError::InvalidVersion);
        }
        Ok(Self {
            provider,
            name,
            version,
        })
    }

    pub fn provider(&self) -> &OpaqueRef {
        &self.provider
    }

    pub fn name(&self) -> &OpaqueRef {
        &self.name
    }

    pub const fn version(&self) -> u16 {
        self.version
    }
}

impl<'de> Deserialize<'de> for HandlerRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct RawHandlerRef {
            provider: OpaqueRef,
            name: OpaqueRef,
            version: u16,
        }
        let raw = RawHandlerRef::deserialize(deserializer)?;
        Self::new(raw.provider, raw.name, raw.version).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobTrigger {
    schema_version: u16,
    kind: TriggerKind,
}

impl JobTrigger {
    pub fn manual() -> Self {
        Self::new(TriggerKind::Manual)
    }

    pub fn lifecycle(event: OpaqueRef, event_version: u16) -> Result<Self, ConstraintError> {
        if event_version == 0 {
            return Err(ConstraintError::InvalidVersion);
        }
        Ok(Self::new(TriggerKind::Lifecycle {
            event,
            event_version,
        }))
    }

    pub fn scheduled(
        definition: ScheduleRef,
        definition_version: u16,
    ) -> Result<Self, ConstraintError> {
        if definition_version == 0 {
            return Err(ConstraintError::InvalidVersion);
        }
        Ok(Self::new(TriggerKind::Scheduled {
            definition,
            definition_version,
        }))
    }

    fn new(kind: TriggerKind) -> Self {
        Self {
            schema_version: TRIGGER_SCHEMA_VERSION,
            kind,
        }
    }

    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub fn kind(&self) -> &TriggerKind {
        &self.kind
    }

    pub(crate) fn validate(&self) -> Result<(), ConstraintError> {
        let version_valid = match &self.kind {
            TriggerKind::Manual => true,
            TriggerKind::Lifecycle { event_version, .. } => *event_version > 0,
            TriggerKind::Scheduled {
                definition_version, ..
            } => *definition_version > 0,
        };
        if self.schema_version != TRIGGER_SCHEMA_VERSION || !version_valid {
            Err(ConstraintError::UnsupportedTriggerVersion)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum TriggerKind {
    Manual,
    Lifecycle {
        event: OpaqueRef,
        event_version: u16,
    },
    Scheduled {
        definition: ScheduleRef,
        definition_version: u16,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobPriority {
    Low,
    Normal,
    High,
    Urgent,
}

impl JobPriority {
    pub(crate) const fn rank(self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Normal => 1,
            Self::High => 2,
            Self::Urgent => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceClass {
    Background,
    Normal,
    Interactive,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    Any,
    OfflineOnly,
    RequireNetwork,
    RequireUnmeteredNetwork,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionCondition {
    SystemIdle,
    ExternalPower,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobConstraints {
    deadline: Option<TimestampMillis>,
    timeout: Option<DurationMillis>,
    max_runtime: Option<DurationMillis>,
    resource_class: ResourceClass,
    network_policy: NetworkPolicy,
    required_conditions: BTreeSet<ExecutionCondition>,
}

impl Default for JobConstraints {
    fn default() -> Self {
        Self {
            deadline: None,
            timeout: None,
            max_runtime: None,
            resource_class: ResourceClass::Background,
            network_policy: NetworkPolicy::Any,
            required_conditions: BTreeSet::new(),
        }
    }
}

impl JobConstraints {
    pub fn with_deadline(mut self, deadline: TimestampMillis) -> Self {
        self.deadline = Some(deadline);
        self
    }

    pub fn with_timeout(mut self, timeout: DurationMillis) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub fn with_max_runtime(mut self, max_runtime: DurationMillis) -> Self {
        self.max_runtime = Some(max_runtime);
        self
    }

    pub fn with_resource_class(mut self, class: ResourceClass) -> Self {
        self.resource_class = class;
        self
    }

    pub fn with_network_policy(mut self, policy: NetworkPolicy) -> Self {
        self.network_policy = policy;
        self
    }

    pub fn require(mut self, condition: ExecutionCondition) -> Self {
        self.required_conditions.insert(condition);
        self
    }

    pub const fn deadline(&self) -> Option<TimestampMillis> {
        self.deadline
    }

    pub const fn timeout(&self) -> Option<DurationMillis> {
        self.timeout
    }

    pub const fn max_runtime(&self) -> Option<DurationMillis> {
        self.max_runtime
    }

    pub const fn resource_class(&self) -> ResourceClass {
        self.resource_class
    }

    pub const fn network_policy(&self) -> NetworkPolicy {
        self.network_policy
    }

    pub fn required_conditions(&self) -> &BTreeSet<ExecutionCondition> {
        &self.required_conditions
    }

    pub(crate) fn validate(&self) -> Result<(), ConstraintError> {
        if self.timeout.is_some_and(|value| value.0 == 0)
            || self.max_runtime.is_some_and(|value| value.0 == 0)
        {
            return Err(ConstraintError::ZeroRuntimeBound);
        }
        Ok(())
    }

    pub(crate) fn effective_runtime(&self) -> Option<DurationMillis> {
        match (self.timeout, self.max_runtime) {
            (Some(timeout), Some(maximum)) => Some(DurationMillis(timeout.0.min(maximum.0))),
            (Some(timeout), None) => Some(timeout),
            (None, Some(maximum)) => Some(maximum),
            (None, None) => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicy {
    max_attempts: u8,
    initial_delay: DurationMillis,
    max_delay: DurationMillis,
    multiplier_milli: u32,
    retryable: BTreeSet<JobFailureCode>,
    exhausted_action: RetryTerminalAction,
}

impl RetryPolicy {
    pub fn new(
        max_attempts: u8,
        initial_delay: DurationMillis,
        max_delay: DurationMillis,
        multiplier_milli: u32,
        retryable: impl IntoIterator<Item = JobFailureCode>,
        exhausted_action: RetryTerminalAction,
    ) -> Result<Self, ConstraintError> {
        let policy = Self {
            max_attempts,
            initial_delay,
            max_delay,
            multiplier_milli,
            retryable: retryable.into_iter().collect(),
            exhausted_action,
        };
        policy.validate()?;
        Ok(policy)
    }

    pub fn no_retry() -> Self {
        Self {
            max_attempts: 1,
            initial_delay: DurationMillis(0),
            max_delay: DurationMillis(0),
            multiplier_milli: 1_000,
            retryable: BTreeSet::new(),
            exhausted_action: RetryTerminalAction::Failed,
        }
    }

    pub const fn max_attempts(&self) -> u8 {
        self.max_attempts
    }

    pub fn retryable(&self) -> &BTreeSet<JobFailureCode> {
        &self.retryable
    }

    pub const fn exhausted_action(&self) -> RetryTerminalAction {
        self.exhausted_action
    }

    pub fn delay_after(&self, failed_attempt: u8) -> DurationMillis {
        let mut delay = self.initial_delay.0;
        for _ in 1..failed_attempt {
            delay = delay
                .saturating_mul(u64::from(self.multiplier_milli))
                .checked_div(1_000)
                .unwrap_or(u64::MAX)
                .min(self.max_delay.0);
        }
        DurationMillis(delay.min(self.max_delay.0))
    }

    pub(crate) fn validate(&self) -> Result<(), ConstraintError> {
        if self.max_attempts == 0 || self.max_attempts > MAX_ATTEMPTS {
            return Err(ConstraintError::InvalidAttemptBound);
        }
        if self.initial_delay.0 > self.max_delay.0 {
            return Err(ConstraintError::InvalidBackoffBounds);
        }
        if !(1_000..=8_000).contains(&self.multiplier_milli) {
            return Err(ConstraintError::InvalidBackoffMultiplier);
        }
        if self.retryable.len() > 16 {
            return Err(ConstraintError::TooManyRetryClasses);
        }
        if self.retryable.iter().any(|code| {
            !matches!(
                code,
                JobFailureCode::Transient | JobFailureCode::HandlerUnavailable
            )
        }) {
            return Err(ConstraintError::UnsafeRetryClass);
        }
        Ok(())
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::no_retry()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetryTerminalAction {
    Failed,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobFailureCode {
    Transient,
    HandlerUnavailable,
    Permanent,
    PermissionDenied,
    DeadlineExceeded,
    Timeout,
    DependencyFailed,
    MissingHandler,
    InvalidCheckpoint,
    StoreUnavailable,
    UnsupportedSchema,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryPolicy {
    NonIdempotent,
    Idempotent,
    VerifiedCheckpoint,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencyRef {
    prerequisite: JobId,
    require_success: bool,
}

impl DependencyRef {
    pub fn success(prerequisite: JobId) -> Self {
        Self {
            prerequisite,
            require_success: true,
        }
    }

    pub fn terminal(prerequisite: JobId) -> Self {
        Self {
            prerequisite,
            require_success: false,
        }
    }

    pub fn prerequisite(&self) -> &JobId {
        &self.prerequisite
    }

    pub const fn requires_success(&self) -> bool {
        self.require_success
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyFailurePolicy {
    /// Keep the dependent in `WaitingOnDependency` until an operator repairs
    /// or replaces the prerequisite through a future authorized API.
    Block,
    CancelDependent,
    AllowFailure,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "code")]
pub enum JobState {
    Queued,
    WaitingOnDependency,
    Running,
    CancelRequested,
    PauseRequested,
    Paused,
    RetryWait,
    Succeeded,
    Failed(JobFailureCode),
    Cancelled,
    TimedOut,
    Interrupted,
    RecoveryRequired(JobFailureCode),
}

impl JobState {
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed(_) | Self::Cancelled | Self::TimedOut
        )
    }

    pub const fn occupies_slot(&self) -> bool {
        matches!(
            self,
            Self::Running | Self::CancelRequested | Self::PauseRequested
        )
    }

    pub(crate) const fn can_transition_to(&self, next: &Self) -> bool {
        use JobState as S;
        match self {
            S::Queued => matches!(
                next,
                S::WaitingOnDependency
                    | S::Running
                    | S::CancelRequested
                    | S::Paused
                    | S::Failed(_)
                    | S::Cancelled
                    | S::TimedOut
                    | S::Interrupted
            ),
            S::WaitingOnDependency => matches!(
                next,
                S::Queued
                    | S::Paused
                    | S::Cancelled
                    | S::TimedOut
                    | S::Failed(_)
                    | S::RecoveryRequired(_)
            ),
            S::Running => matches!(
                next,
                S::CancelRequested
                    | S::PauseRequested
                    | S::RetryWait
                    | S::Succeeded
                    | S::Failed(_)
                    | S::Cancelled
                    | S::TimedOut
                    | S::Interrupted
                    | S::RecoveryRequired(_)
            ),
            S::CancelRequested => matches!(
                next,
                S::Succeeded | S::Failed(_) | S::Cancelled | S::TimedOut | S::Interrupted
            ),
            S::PauseRequested => matches!(
                next,
                S::CancelRequested
                    | S::Paused
                    | S::RetryWait
                    | S::Succeeded
                    | S::Failed(_)
                    | S::Cancelled
                    | S::TimedOut
                    | S::Interrupted
            ),
            S::Paused => matches!(next, S::Queued | S::Cancelled | S::TimedOut | S::Failed(_)),
            S::RetryWait => matches!(
                next,
                S::Queued
                    | S::WaitingOnDependency
                    | S::Paused
                    | S::Cancelled
                    | S::TimedOut
                    | S::Failed(_)
            ),
            S::Interrupted => matches!(
                next,
                S::Queued | S::RecoveryRequired(_) | S::TimedOut | S::Failed(_)
            ),
            S::RecoveryRequired(_) => {
                matches!(next, S::Queued | S::Cancelled | S::TimedOut | S::Failed(_))
            }
            S::Succeeded | S::Failed(_) | S::Cancelled | S::TimedOut => false,
        }
    }
}

/// A metadata-only request. There is intentionally no closure, credential,
/// arbitrary input blob, or executable payload field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobRequest {
    handler: HandlerRef,
    trigger: JobTrigger,
    priority: JobPriority,
    constraints: JobConstraints,
    retry_policy: RetryPolicy,
    dependencies: Vec<DependencyRef>,
    dependency_failure_policy: DependencyFailurePolicy,
    idempotency_key: Option<IdempotencyKey>,
    request_ref: Option<OpaqueRef>,
    correlation_ref: Option<OpaqueRef>,
}

impl JobRequest {
    pub fn new(handler: HandlerRef) -> Self {
        Self {
            handler,
            trigger: JobTrigger::manual(),
            priority: JobPriority::Normal,
            constraints: JobConstraints::default(),
            retry_policy: RetryPolicy::no_retry(),
            dependencies: Vec::new(),
            dependency_failure_policy: DependencyFailurePolicy::Block,
            idempotency_key: None,
            request_ref: None,
            correlation_ref: None,
        }
    }

    pub fn with_trigger(mut self, trigger: JobTrigger) -> Self {
        self.trigger = trigger;
        self
    }

    pub fn with_priority(mut self, priority: JobPriority) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_constraints(mut self, constraints: JobConstraints) -> Self {
        self.constraints = constraints;
        self
    }

    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    pub fn with_dependencies(
        mut self,
        dependencies: Vec<DependencyRef>,
        failure_policy: DependencyFailurePolicy,
    ) -> Self {
        self.dependencies = dependencies;
        self.dependency_failure_policy = failure_policy;
        self
    }

    pub fn with_idempotency_key(mut self, key: IdempotencyKey) -> Self {
        self.idempotency_key = Some(key);
        self
    }

    pub fn with_request_ref(mut self, reference: OpaqueRef) -> Self {
        self.request_ref = Some(reference);
        self
    }

    pub fn with_correlation_ref(mut self, reference: OpaqueRef) -> Self {
        self.correlation_ref = Some(reference);
        self
    }

    pub fn handler(&self) -> &HandlerRef {
        &self.handler
    }

    pub const fn priority(&self) -> JobPriority {
        self.priority
    }

    pub(crate) fn idempotency_key(&self) -> Option<&IdempotencyKey> {
        self.idempotency_key.as_ref()
    }

    pub(crate) fn dependencies(&self) -> &[DependencyRef] {
        &self.dependencies
    }

    pub(crate) fn constraints(&self) -> &JobConstraints {
        &self.constraints
    }

    pub(crate) fn validate(&self) -> Result<(), ConstraintError> {
        if self.dependencies.len() > MAX_DEPENDENCIES {
            return Err(ConstraintError::TooManyDependencies);
        }
        if self
            .dependencies
            .iter()
            .map(DependencyRef::prerequisite)
            .collect::<BTreeSet<_>>()
            .len()
            != self.dependencies.len()
        {
            return Err(ConstraintError::DuplicateDependency);
        }
        self.trigger.validate()?;
        self.constraints.validate()?;
        self.retry_policy.validate()?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobRecord {
    schema_version: u16,
    revision: u64,
    id: JobId,
    owner: JobOwner,
    handler: HandlerRef,
    trigger: JobTrigger,
    priority: JobPriority,
    constraints: JobConstraints,
    state: JobState,
    retry_policy: RetryPolicy,
    dependencies: Vec<DependencyRef>,
    dependency_failure_policy: DependencyFailurePolicy,
    idempotency_key: Option<IdempotencyKey>,
    request_ref: Option<OpaqueRef>,
    correlation_ref: Option<OpaqueRef>,
    checkpoint_ref: Option<CheckpointRef>,
    progress: Option<ProgressSummary>,
    attempt_count: u8,
    enqueue_sequence: u64,
    created_at: TimestampMillis,
    updated_at: TimestampMillis,
    started_at: Option<TimestampMillis>,
    finished_at: Option<TimestampMillis>,
    retry_at: Option<TimestampMillis>,
    cancellation_requested_at: Option<TimestampMillis>,
    stop_reason: Option<StopReason>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopReason {
    Cancelled,
    TimedOut,
}

impl JobRecord {
    pub(crate) fn new(
        id: JobId,
        owner: JobOwner,
        request: JobRequest,
        now: TimestampMillis,
    ) -> Self {
        Self {
            schema_version: JOB_RECORD_SCHEMA_VERSION,
            revision: 0,
            id,
            owner,
            handler: request.handler,
            trigger: request.trigger,
            priority: request.priority,
            constraints: request.constraints,
            state: if request.dependencies.is_empty() {
                JobState::Queued
            } else {
                JobState::WaitingOnDependency
            },
            retry_policy: request.retry_policy,
            dependencies: request.dependencies,
            dependency_failure_policy: request.dependency_failure_policy,
            idempotency_key: request.idempotency_key,
            request_ref: request.request_ref,
            correlation_ref: request.correlation_ref,
            checkpoint_ref: None,
            progress: None,
            attempt_count: 0,
            enqueue_sequence: 0,
            created_at: now,
            updated_at: now,
            started_at: None,
            finished_at: None,
            retry_at: None,
            cancellation_requested_at: None,
            stop_reason: None,
        }
    }

    pub fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub fn id(&self) -> &JobId {
        &self.id
    }

    pub fn owner(&self) -> &JobOwner {
        &self.owner
    }

    pub fn handler(&self) -> &HandlerRef {
        &self.handler
    }

    pub fn trigger(&self) -> &JobTrigger {
        &self.trigger
    }

    pub const fn priority(&self) -> JobPriority {
        self.priority
    }

    pub fn constraints(&self) -> &JobConstraints {
        &self.constraints
    }

    pub fn state(&self) -> &JobState {
        &self.state
    }

    pub const fn attempt_count(&self) -> u8 {
        self.attempt_count
    }

    pub fn dependencies(&self) -> &[DependencyRef] {
        &self.dependencies
    }

    pub(crate) const fn dependency_failure_policy(&self) -> DependencyFailurePolicy {
        self.dependency_failure_policy
    }

    pub(crate) fn correlation_ref(&self) -> Option<&OpaqueRef> {
        self.correlation_ref.as_ref()
    }

    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.retry_policy
    }

    pub fn request_ref(&self) -> Option<&OpaqueRef> {
        self.request_ref.as_ref()
    }

    pub fn checkpoint_ref(&self) -> Option<&CheckpointRef> {
        self.checkpoint_ref.as_ref()
    }

    pub fn progress(&self) -> Option<&ProgressSummary> {
        self.progress.as_ref()
    }

    pub const fn revision(&self) -> u64 {
        self.revision
    }

    pub const fn enqueue_sequence(&self) -> u64 {
        self.enqueue_sequence
    }

    pub const fn created_at(&self) -> TimestampMillis {
        self.created_at
    }

    pub const fn updated_at(&self) -> TimestampMillis {
        self.updated_at
    }

    pub const fn started_at(&self) -> Option<TimestampMillis> {
        self.started_at
    }

    pub const fn finished_at(&self) -> Option<TimestampMillis> {
        self.finished_at
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.state.is_terminal()
    }

    pub(crate) fn has_execution_capacity(&self) -> bool {
        self.attempt_count < MAX_ATTEMPTS
    }

    pub(crate) fn validate(&self) -> Result<(), ConstraintError> {
        if self.schema_version != JOB_RECORD_SCHEMA_VERSION {
            return Err(ConstraintError::UnsupportedSchemaVersion);
        }
        self.trigger.validate()?;
        if self.attempt_count > MAX_ATTEMPTS {
            return Err(ConstraintError::InvalidAttemptCount);
        }
        if self.dependencies.len() > MAX_DEPENDENCIES {
            return Err(ConstraintError::TooManyDependencies);
        }
        if self
            .dependencies
            .iter()
            .map(DependencyRef::prerequisite)
            .collect::<BTreeSet<_>>()
            .len()
            != self.dependencies.len()
        {
            return Err(ConstraintError::DuplicateDependency);
        }
        self.constraints.validate()?;
        self.retry_policy.validate()?;
        PrincipalRef::new(self.owner.principal().as_str().to_owned())
            .map_err(|_| ConstraintError::InvalidOwner)?;
        if let Some(profile) = self.owner.profile() {
            OwnerProfileRef::new(profile.as_str().to_owned())
                .map_err(|_| ConstraintError::InvalidOwner)?;
        }
        Ok(())
    }

    pub(crate) fn set_state(
        &mut self,
        state: JobState,
        now: TimestampMillis,
    ) -> Result<(), ConstraintError> {
        if !self.state.can_transition_to(&state) {
            return Err(ConstraintError::InvalidStateTransition);
        }
        self.state = state;
        self.updated_at = now;
        Ok(())
    }

    pub(crate) fn set_progress(&mut self, progress: ProgressSummary, now: TimestampMillis) {
        self.progress = Some(progress);
        self.updated_at = now;
    }

    pub(crate) fn start(&mut self, now: TimestampMillis) -> Result<(), ConstraintError> {
        if self.attempt_count >= MAX_ATTEMPTS {
            return Err(ConstraintError::InvalidAttemptCount);
        }
        self.set_state(JobState::Running, now)?;
        self.attempt_count = self.attempt_count.saturating_add(1);
        self.started_at = Some(now);
        self.retry_at = None;
        self.stop_reason = None;
        self.cancellation_requested_at = None;
        Ok(())
    }

    pub(crate) fn set_checkpoint(&mut self, checkpoint: CheckpointRef) {
        self.checkpoint_ref = Some(checkpoint);
    }

    pub(crate) fn set_retry_at(&mut self, retry_at: TimestampMillis) {
        self.retry_at = Some(retry_at);
    }

    pub(crate) fn retry_at(&self) -> Option<TimestampMillis> {
        self.retry_at
    }

    pub(crate) fn request_cancel(&mut self, now: TimestampMillis) -> Result<(), ConstraintError> {
        self.set_state(JobState::CancelRequested, now)?;
        self.cancellation_requested_at = Some(now);
        self.stop_reason = Some(StopReason::Cancelled);
        Ok(())
    }

    pub(crate) fn request_timeout(&mut self, now: TimestampMillis) -> Result<(), ConstraintError> {
        self.set_state(JobState::CancelRequested, now)?;
        self.cancellation_requested_at = Some(now);
        self.stop_reason = Some(StopReason::TimedOut);
        Ok(())
    }

    pub(crate) fn stop_reason(&self) -> Option<StopReason> {
        self.stop_reason
    }

    pub(crate) fn mark_finished(&mut self, now: TimestampMillis) {
        self.finished_at = Some(now);
        self.updated_at = now;
    }

    /// Store bookkeeping field used by atomic queue implementations.
    pub fn set_enqueue_sequence(&mut self, sequence: u64) {
        self.enqueue_sequence = sequence;
    }

    /// Store bookkeeping field used by compare-and-swap implementations.
    pub fn set_revision(&mut self, revision: u64) {
        self.revision = revision;
    }

    /// Atomically claimed records may be transitioned to a running attempt by
    /// a JobStore implementation that holds its transaction lock.
    pub fn start_attempt(&mut self, now: TimestampMillis) -> Result<(), ConstraintError> {
        self.start(now)
    }

    pub(crate) fn copy_identity_matches(&self, other: &Self) -> bool {
        self.id == other.id && self.owner == other.owner && self.handler == other.handler
    }

    #[cfg(test)]
    pub(crate) fn seed_state_for_test(&mut self, state: JobState) {
        if matches!(state, JobState::Running) {
            let _ = self.start(TimestampMillis(1));
        } else {
            self.state = state;
        }
        self.revision = self.revision.saturating_add(1);
    }

    #[cfg(test)]
    pub(crate) fn seed_dependency_for_test(&mut self, dependency: DependencyRef) {
        self.dependencies.push(dependency);
        self.state = JobState::WaitingOnDependency;
        self.revision = self.revision.saturating_add(1);
    }

    #[cfg(test)]
    pub(crate) fn seed_checkpoint_for_test(&mut self, checkpoint: CheckpointRef) {
        self.set_checkpoint(checkpoint);
        self.revision = self.revision.saturating_add(1);
    }
}

pub fn encode_job_record(record: &JobRecord) -> Result<Vec<u8>, JobRecordCodecError> {
    record
        .validate()
        .map_err(JobRecordCodecError::InvalidRecord)?;
    serde_json::to_vec(record).map_err(|_| JobRecordCodecError::Malformed)
}

pub fn decode_job_record(bytes: &[u8]) -> Result<JobRecord, JobRecordCodecError> {
    let record: JobRecord =
        serde_json::from_slice(bytes).map_err(|_| JobRecordCodecError::Malformed)?;
    record
        .validate()
        .map_err(JobRecordCodecError::InvalidRecord)?;
    Ok(record)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobRecordCodecError {
    Malformed,
    InvalidRecord(ConstraintError),
}

impl fmt::Display for JobRecordCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed => formatter.write_str("job record is malformed"),
            Self::InvalidRecord(error) => write!(formatter, "job record is invalid: {error}"),
        }
    }
}

impl std::error::Error for JobRecordCodecError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConstraintError {
    InvalidIdentifier,
    InvalidVersion,
    UnsupportedSchemaVersion,
    UnsupportedTriggerVersion,
    InvalidOwner,
    ZeroRuntimeBound,
    InvalidAttemptBound,
    InvalidAttemptCount,
    InvalidBackoffBounds,
    InvalidBackoffMultiplier,
    TooManyRetryClasses,
    UnsafeRetryClass,
    TooManyDependencies,
    DuplicateDependency,
    InvalidStateTransition,
}

impl fmt::Display for ConstraintError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentifier => {
                "identifier is empty, too long, or contains unsupported characters"
            }
            Self::InvalidVersion => "version must be non-zero",
            Self::UnsupportedSchemaVersion => "job record schema version is unsupported",
            Self::UnsupportedTriggerVersion => "job trigger schema version is unsupported",
            Self::InvalidOwner => "job owner reference is malformed",
            Self::ZeroRuntimeBound => "timeout and maximum runtime must be non-zero",
            Self::InvalidAttemptBound => "retry attempts must be between 1 and 16",
            Self::InvalidAttemptCount => "attempt count exceeds the retry policy bound",
            Self::InvalidBackoffBounds => "initial retry delay exceeds maximum retry delay",
            Self::InvalidBackoffMultiplier => "retry multiplier must be between 1x and 8x",
            Self::TooManyRetryClasses => "retry classification count exceeds the bound",
            Self::UnsafeRetryClass => {
                "permission, deadline, timeout, and permanent failures cannot retry"
            }
            Self::TooManyDependencies => "dependency count exceeds the bound",
            Self::DuplicateDependency => "duplicate prerequisite job ID",
            Self::InvalidStateTransition => "job state transition is not allowed",
        })
    }
}

impl std::error::Error for ConstraintError {}

fn validate_token(value: &str, max: usize, _label: &'static str) -> Result<(), ConstraintError> {
    if value.is_empty()
        || value.len() > max
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || value.as_bytes().iter().any(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b':' | b'-'))
        })
    {
        return Err(ConstraintError::InvalidIdentifier);
    }
    Ok(())
}
