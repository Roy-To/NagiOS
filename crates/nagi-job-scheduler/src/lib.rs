//! Host-only, provider-neutral background job orchestration contract.
//!
//! This crate is deliberately not a Nagi system service. It provides an
//! in-memory deterministic scheduler and adapter traits for contract tests;
//! it does not start during system boot, call host daemons, or perform domain
//! side effects. Persistent stores and runtime authority adapters remain
//! future integrations behind the traits below.

#![forbid(unsafe_code)]

mod authorization;
mod diagnostics;
mod model;
mod scheduler;
mod store;

pub use authorization::{
    AuthorizationError, AuthorizationErrorCode, CallerRef, JobAction, JobAuthorization, JobOwner,
    OwnerProfileRef, PrincipalRef,
};
pub use diagnostics::{
    BoundedJobEventRecorder, JobDiagnosticEvent, JobDiagnosticSink, JobDiagnosticSinkError,
    JobEventCode, JobEventReason, NoopJobDiagnosticSink, ProgressError, ProgressSummary,
    JOB_EVENT_SCHEMA_VERSION,
};
pub use model::{
    decode_job_record, encode_job_record, CheckpointRef, ConstraintError, DependencyFailurePolicy,
    DependencyRef, DurationMillis, ExecutionCondition, HandlerRef, IdempotencyKey, JobConstraints,
    JobFailureCode, JobId, JobPriority, JobRecord, JobRecordCodecError, JobRequest, JobState,
    JobTrigger, NetworkPolicy, OpaqueRef, ProgressCode, RecoveryPolicy, ResourceClass, RetryPolicy,
    RetryTerminalAction, ScheduleRef, TimestampMillis, TriggerKind, JOB_RECORD_SCHEMA_VERSION,
};
pub use scheduler::{
    CancellationReason, CancellationToken, Clock, DispatchResult, EnqueueResult, HandlerContext,
    HandlerOutcome, JobHandler, JobProviderRegistry, ManualClock, NetworkConditions, Scheduler,
    SchedulerConfig, SchedulerConfigError, SchedulerError, SystemConditions,
};
pub use store::{DedupeScope, InMemoryJobStore, InsertOutcome, JobStore, JobStoreError};

#[cfg(test)]
mod tests;
