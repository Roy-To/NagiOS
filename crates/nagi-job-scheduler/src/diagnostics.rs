use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Mutex;

use crate::model::{HandlerRef, JobId, ProgressCode, TimestampMillis};

pub const JOB_EVENT_SCHEMA_VERSION: u16 = 1;
pub const MAX_RECORDED_JOB_EVENTS: usize = 4_096;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobEventCode {
    EnqueueAccepted,
    EnqueueRejected,
    Deduplicated,
    DependencyBlocked,
    DependencyReleased,
    Started,
    StateTransition,
    Progress,
    RetryScheduled,
    RetryExhausted,
    PauseRequested,
    Paused,
    Resumed,
    CancellationRequested,
    CancellationCompleted,
    TimedOut,
    RecoveryResumed,
    RecoveryRequired,
    HandlerMissing,
    HandlerFailed,
    PermissionDenied,
    QueueSaturated,
    StoreFailure,
    StoreCorruption,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobEventReason {
    InvalidRequest,
    QueueCapacity,
    DependencyPending,
    DependencyFailed,
    RetryableFailure,
    RetryExhausted,
    PermissionDenied,
    MissingHandler,
    DeadlineExceeded,
    Timeout,
    Cancelled,
    UnsupportedPause,
    StoreUnavailable,
    CorruptRecord,
    UnsupportedSchema,
    UnsafeRecovery,
    CheckpointRejected,
    DiagnosticsUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressSummary {
    code: ProgressCode,
    completed: u64,
    total: Option<u64>,
}

impl ProgressSummary {
    pub fn new(
        code: ProgressCode,
        completed: u64,
        total: Option<u64>,
    ) -> Result<Self, ProgressError> {
        if total == Some(0) || total.is_some_and(|value| completed > value) {
            return Err(ProgressError::InvalidRange);
        }
        Ok(Self {
            code,
            completed,
            total,
        })
    }

    pub fn code(&self) -> &ProgressCode {
        &self.code
    }

    pub const fn completed(&self) -> u64 {
        self.completed
    }

    pub const fn total(&self) -> Option<u64> {
        self.total
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProgressError {
    InvalidRange,
    RateLimited,
    StoreUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobDiagnosticEvent {
    schema_version: u16,
    timestamp: TimestampMillis,
    code: JobEventCode,
    job_id: Option<JobId>,
    handler: Option<HandlerRef>,
    attempt: Option<u8>,
    correlation: Option<JobId>,
    reason: Option<JobEventReason>,
    progress: Option<ProgressSummary>,
}

#[derive(Default)]
pub(crate) struct JobDiagnosticDetails {
    pub(crate) job_id: Option<JobId>,
    pub(crate) handler: Option<HandlerRef>,
    pub(crate) attempt: Option<u8>,
    pub(crate) correlation: Option<JobId>,
    pub(crate) reason: Option<JobEventReason>,
    pub(crate) progress: Option<ProgressSummary>,
}

impl JobDiagnosticEvent {
    pub(crate) fn new(
        timestamp: TimestampMillis,
        code: JobEventCode,
        details: JobDiagnosticDetails,
    ) -> Self {
        Self {
            schema_version: JOB_EVENT_SCHEMA_VERSION,
            timestamp,
            code,
            job_id: details.job_id,
            handler: details.handler,
            attempt: details.attempt,
            correlation: details.correlation,
            reason: details.reason,
            progress: details.progress,
        }
    }

    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    pub const fn timestamp(&self) -> TimestampMillis {
        self.timestamp
    }

    pub const fn code(&self) -> JobEventCode {
        self.code
    }

    pub fn job_id(&self) -> Option<&JobId> {
        self.job_id.as_ref()
    }

    pub fn handler(&self) -> Option<&HandlerRef> {
        self.handler.as_ref()
    }

    pub const fn attempt(&self) -> Option<u8> {
        self.attempt
    }

    pub const fn reason(&self) -> Option<JobEventReason> {
        self.reason
    }

    pub fn progress(&self) -> Option<&ProgressSummary> {
        self.progress.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JobDiagnosticSinkError;

/// Narrow adapter pending an accepted shared runtime event vocabulary/sink.
/// Implementations receive bounded enum codes and references, never job input,
/// idempotency tokens, capabilities, or credentials.
pub trait JobDiagnosticSink: Send + Sync {
    fn record(&self, event: JobDiagnosticEvent) -> Result<(), JobDiagnosticSinkError>;
}

#[derive(Default)]
pub struct NoopJobDiagnosticSink;

impl JobDiagnosticSink for NoopJobDiagnosticSink {
    fn record(&self, _event: JobDiagnosticEvent) -> Result<(), JobDiagnosticSinkError> {
        Ok(())
    }
}

/// Host-test recorder with a fixed maximum capacity. Old events are evicted
/// from the front; sink failure never participates in scheduler decisions.
pub struct BoundedJobEventRecorder {
    capacity: usize,
    events: Mutex<VecDeque<JobDiagnosticEvent>>,
}

impl BoundedJobEventRecorder {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.clamp(1, MAX_RECORDED_JOB_EVENTS),
            events: Mutex::new(VecDeque::new()),
        }
    }

    pub fn snapshot(&self) -> Vec<JobDiagnosticEvent> {
        self.events
            .lock()
            .map(|events| events.iter().cloned().collect())
            .unwrap_or_default()
    }
}

impl Default for BoundedJobEventRecorder {
    fn default() -> Self {
        Self::new(256)
    }
}

impl JobDiagnosticSink for BoundedJobEventRecorder {
    fn record(&self, event: JobDiagnosticEvent) -> Result<(), JobDiagnosticSinkError> {
        let mut events = self.events.lock().map_err(|_| JobDiagnosticSinkError)?;
        if events.len() == self.capacity {
            events.pop_front();
        }
        events.push_back(event);
        Ok(())
    }
}
