use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crate::authorization::{AuthorizationError, CallerRef, JobAction, JobAuthorization};
use crate::diagnostics::{
    JobDiagnosticDetails, JobDiagnosticEvent, JobDiagnosticSink, JobEventCode, JobEventReason,
    ProgressError, ProgressSummary,
};
use crate::model::{
    CheckpointRef, ConstraintError, DependencyFailurePolicy, DurationMillis, ExecutionCondition,
    HandlerRef, JobFailureCode, JobId, JobPriority, JobRecord, JobRequest, JobState, NetworkPolicy,
    RecoveryPolicy, RetryTerminalAction, StopReason, TimestampMillis,
};
use crate::store::{DedupeScope, InMemoryJobStore, InsertOutcome, JobStore, JobStoreError};

const DEFAULT_DEDUPE_WINDOW_MS: u64 = 86_400_000;
const DEFAULT_AGING_INTERVAL_MS: u64 = 5_000;
const DEFAULT_PROGRESS_INTERVAL_MS: u64 = 250;
const MAX_CONCURRENCY: usize = 256;
const MAX_QUEUE_CAPACITY: usize = 65_536;
const MAX_RECORD_CAPACITY: usize = 262_144;

pub trait Clock: Send + Sync {
    fn now(&self) -> TimestampMillis;
}

/// Monotonic virtual clock for host tests. It performs no host time reads.
#[derive(Clone, Default)]
pub struct ManualClock {
    now: Arc<AtomicU64>,
}

impl ManualClock {
    pub fn new(initial: TimestampMillis) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(initial.0)),
        }
    }

    pub fn advance(&self, duration: DurationMillis) -> TimestampMillis {
        let next = self
            .now
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                Some(current.saturating_add(duration.0))
            })
            .unwrap_or_else(|current| current);
        TimestampMillis(next.saturating_add(duration.0))
    }

    pub fn set(&self, now: TimestampMillis) -> Result<(), SchedulerError> {
        let mut current = self.now.load(Ordering::SeqCst);
        loop {
            if now.0 < current {
                return Err(SchedulerError::ClockWentBackwards);
            }
            match self
                .now
                .compare_exchange(current, now.0, Ordering::SeqCst, Ordering::SeqCst)
            {
                Ok(_) => return Ok(()),
                Err(actual) => current = actual,
            }
        }
    }
}

impl Clock for ManualClock {
    fn now(&self) -> TimestampMillis {
        TimestampMillis(self.now.load(Ordering::SeqCst))
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SystemConditions {
    pub network: NetworkConditions,
    pub system_idle: bool,
    pub external_power: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NetworkConditions {
    pub available: bool,
    pub unmetered: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulerConfig {
    global_concurrency: usize,
    per_owner_concurrency: usize,
    queue_capacity: usize,
    record_capacity: usize,
    dedupe_window: DurationMillis,
    priority_aging_interval: DurationMillis,
    progress_interval: DurationMillis,
}

impl SchedulerConfig {
    pub fn new(
        global_concurrency: usize,
        per_owner_concurrency: usize,
        queue_capacity: usize,
    ) -> Result<Self, SchedulerConfigError> {
        if global_concurrency == 0
            || per_owner_concurrency == 0
            || queue_capacity == 0
            || per_owner_concurrency > global_concurrency
            || global_concurrency > MAX_CONCURRENCY
            || queue_capacity > MAX_QUEUE_CAPACITY
        {
            return Err(SchedulerConfigError::InvalidLimits);
        }
        Ok(Self {
            global_concurrency,
            per_owner_concurrency,
            queue_capacity,
            record_capacity: queue_capacity
                .saturating_mul(16)
                .max(queue_capacity)
                .min(MAX_RECORD_CAPACITY),
            dedupe_window: DurationMillis(DEFAULT_DEDUPE_WINDOW_MS),
            priority_aging_interval: DurationMillis(DEFAULT_AGING_INTERVAL_MS),
            progress_interval: DurationMillis(DEFAULT_PROGRESS_INTERVAL_MS),
        })
    }

    pub fn with_dedupe_window(mut self, duration: DurationMillis) -> Self {
        self.dedupe_window = duration;
        self
    }

    pub fn with_record_capacity(mut self, capacity: usize) -> Result<Self, SchedulerConfigError> {
        if capacity < self.queue_capacity || capacity > MAX_RECORD_CAPACITY {
            return Err(SchedulerConfigError::InvalidLimits);
        }
        self.record_capacity = capacity;
        Ok(self)
    }

    pub fn with_priority_aging_interval(mut self, duration: DurationMillis) -> Self {
        self.priority_aging_interval = DurationMillis(duration.0.max(1));
        self
    }

    pub fn with_progress_interval(mut self, duration: DurationMillis) -> Self {
        self.progress_interval = DurationMillis(duration.0.max(1));
        self
    }

    pub const fn global_concurrency(&self) -> usize {
        self.global_concurrency
    }

    pub const fn per_owner_concurrency(&self) -> usize {
        self.per_owner_concurrency
    }

    pub const fn queue_capacity(&self) -> usize {
        self.queue_capacity
    }

    pub const fn record_capacity(&self) -> usize {
        self.record_capacity
    }
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self::new(4, 1, 256).expect("static scheduler defaults are valid")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerConfigError {
    InvalidLimits,
    ZeroDedupeWindow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    InvalidRequest(ConstraintError),
    Authorization(AuthorizationError),
    Store(JobStoreError),
    DeadlineAlreadyPassed,
    UnknownDependency,
    DependencyCycle,
    QueueFull,
    DuplicateJobId,
    NotFound,
    InvalidTransition,
    UnsupportedPause,
    MissingHandler,
    HandlerAlreadyRegistered,
    ClockWentBackwards,
    HandlerRegistryPoisoned,
    RuntimeStatePoisoned,
    InvalidDedupeWindow,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispatchResult {
    Idle,
    Completed(Box<JobRecord>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HandlerOutcome {
    Succeeded,
    Failed(JobFailureCode),
    Cancelled,
    Paused(Option<CheckpointRef>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancellationReason {
    Requested,
    TimedOut,
}

#[derive(Clone)]
pub struct CancellationToken {
    control: Arc<RunControl>,
}

impl CancellationToken {
    pub fn is_cancelled(&self) -> bool {
        self.reason().is_some()
    }

    pub fn reason(&self) -> Option<CancellationReason> {
        self.control.reason()
    }

    pub fn child_token(&self) -> Self {
        Self {
            control: Arc::new(RunControl::child(self.control.clone())),
        }
    }

    fn requested(control: Arc<RunControl>) -> Self {
        Self { control }
    }
}

impl TryFrom<u8> for CancellationReason {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Requested),
            2 => Ok(Self::TimedOut),
            _ => Err(()),
        }
    }
}

struct RunControl {
    stop: AtomicU8,
    pause: AtomicU8,
    parent: Option<Arc<RunControl>>,
}

impl Default for RunControl {
    fn default() -> Self {
        Self {
            stop: AtomicU8::new(0),
            pause: AtomicU8::new(0),
            parent: None,
        }
    }
}

impl RunControl {
    fn child(parent: Arc<Self>) -> Self {
        Self {
            parent: Some(parent),
            ..Self::default()
        }
    }

    fn reason(&self) -> Option<CancellationReason> {
        self.stop
            .load(Ordering::SeqCst)
            .try_into()
            .ok()
            .or_else(|| self.parent.as_ref().and_then(|parent| parent.reason()))
    }

    fn request_cancel(&self) {
        let _ = self
            .stop
            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst);
    }

    fn request_timeout(&self) {
        let _ = self
            .stop
            .compare_exchange(0, 2, Ordering::SeqCst, Ordering::SeqCst);
    }

    fn request_pause(&self) {
        self.pause.store(1, Ordering::SeqCst);
    }
}

pub struct HandlerContext {
    record: JobRecord,
    cancellation: CancellationToken,
    control: Arc<RunControl>,
    progress: Box<dyn FnMut(ProgressSummary) -> Result<(), ProgressError> + Send>,
    checkpoint: Box<dyn FnMut(CheckpointRef) -> Result<(), ProgressError> + Send>,
}

impl HandlerContext {
    pub fn record(&self) -> &JobRecord {
        &self.record
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }

    pub fn cancellation_reason(&self) -> Option<CancellationReason> {
        self.cancellation.reason()
    }

    pub fn pause_requested(&self) -> bool {
        self.control.pause.load(Ordering::SeqCst) != 0
    }

    pub fn report_progress(&mut self, summary: ProgressSummary) -> Result<(), ProgressError> {
        (self.progress)(summary)
    }

    /// Persists only an opaque provider-owned checkpoint reference. The
    /// provider remains responsible for validating and authorizing its data.
    pub fn publish_checkpoint(&mut self, checkpoint: CheckpointRef) -> Result<(), ProgressError> {
        (self.checkpoint)(checkpoint)
    }
}

pub trait JobHandler: Send + Sync {
    fn handler_ref(&self) -> HandlerRef;

    fn recovery_policy(&self, _record: &JobRecord) -> RecoveryPolicy {
        RecoveryPolicy::NonIdempotent
    }

    fn verify_checkpoint(&self, _checkpoint: &CheckpointRef) -> bool {
        false
    }

    fn supports_pause(&self) -> bool {
        false
    }

    fn execute(&self, context: &mut HandlerContext) -> HandlerOutcome;
}

#[derive(Default)]
pub struct JobProviderRegistry {
    providers: RwLock<HashMap<HandlerRef, Arc<dyn JobHandler>>>,
}

impl JobProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, provider: Arc<dyn JobHandler>) -> Result<(), SchedulerError> {
        let reference = provider.handler_ref();
        let mut providers = self
            .providers
            .write()
            .map_err(|_| SchedulerError::HandlerRegistryPoisoned)?;
        if providers.contains_key(&reference) {
            return Err(SchedulerError::HandlerAlreadyRegistered);
        }
        providers.insert(reference, provider);
        Ok(())
    }

    fn get(&self, reference: &HandlerRef) -> Result<Option<Arc<dyn JobHandler>>, SchedulerError> {
        self.providers
            .read()
            .map(|providers| providers.get(reference).cloned())
            .map_err(|_| SchedulerError::HandlerRegistryPoisoned)
    }
}

#[derive(Clone)]
pub struct Scheduler {
    store: Arc<dyn JobStore>,
    clock: Arc<dyn Clock>,
    authorization: Arc<dyn JobAuthorization>,
    diagnostics: Arc<dyn JobDiagnosticSink>,
    providers: Arc<JobProviderRegistry>,
    config: SchedulerConfig,
    conditions: Arc<Mutex<SystemConditions>>,
    runtime: Arc<Mutex<RuntimeState>>,
    id_sequence: Arc<AtomicU64>,
    dispatch_lock: Arc<Mutex<()>>,
}

#[derive(Default)]
struct RuntimeState {
    active: HashMap<JobId, Arc<RunControl>>,
    last_progress: HashMap<JobId, TimestampMillis>,
}

impl Scheduler {
    pub fn new(
        store: Arc<dyn JobStore>,
        clock: Arc<dyn Clock>,
        authorization: Arc<dyn JobAuthorization>,
        diagnostics: Arc<dyn JobDiagnosticSink>,
        providers: Arc<JobProviderRegistry>,
        config: SchedulerConfig,
    ) -> Result<Self, SchedulerConfigError> {
        if config.dedupe_window.0 == 0 {
            return Err(SchedulerConfigError::ZeroDedupeWindow);
        }
        Ok(Self {
            store,
            clock,
            authorization,
            diagnostics,
            providers,
            config,
            conditions: Arc::new(Mutex::new(SystemConditions::default())),
            runtime: Arc::new(Mutex::new(RuntimeState::default())),
            id_sequence: Arc::new(AtomicU64::new(1)),
            dispatch_lock: Arc::new(Mutex::new(())),
        })
    }

    pub fn host_reference(
        clock: Arc<dyn Clock>,
        authorization: Arc<dyn JobAuthorization>,
        diagnostics: Arc<dyn JobDiagnosticSink>,
        providers: Arc<JobProviderRegistry>,
        config: SchedulerConfig,
    ) -> Result<(Self, Arc<InMemoryJobStore>), SchedulerConfigError> {
        let store = Arc::new(InMemoryJobStore::new());
        let scheduler = Self::new(
            store.clone(),
            clock,
            authorization,
            diagnostics,
            providers,
            config,
        )?;
        Ok((scheduler, store))
    }

    pub fn set_conditions(&self, conditions: SystemConditions) -> Result<(), SchedulerError> {
        *self
            .conditions
            .lock()
            .map_err(|_| SchedulerError::RuntimeStatePoisoned)? = conditions;
        Ok(())
    }

    pub fn enqueue(
        &self,
        caller: &CallerRef,
        request: JobRequest,
    ) -> Result<EnqueueResult, SchedulerError> {
        request.validate().map_err(SchedulerError::InvalidRequest)?;
        let now = self.clock.now();
        if request
            .constraints()
            .deadline()
            .is_some_and(|deadline| deadline <= now)
        {
            self.emit(
                now,
                JobEventCode::EnqueueRejected,
                None,
                Some(request.handler().clone()),
                None,
                None,
                Some(JobEventReason::DeadlineExceeded),
                None,
            );
            return Err(SchedulerError::DeadlineAlreadyPassed);
        }

        let owner = self
            .authorization
            .owner_for(caller)
            .map_err(SchedulerError::Authorization)?;
        self.authorization
            .authorize_user(caller, &owner, JobAction::Enqueue, None)
            .map_err(SchedulerError::Authorization)?;
        self.authorization
            .authorize_priority(caller, &owner, request.handler(), request.priority())
            .map_err(SchedulerError::Authorization)?;

        let current = self.store.list().map_err(SchedulerError::Store)?;
        for dependency in request.dependencies() {
            if current
                .iter()
                .all(|record| record.id() != dependency.prerequisite())
            {
                return Err(SchedulerError::UnknownDependency);
            }
        }

        let sequence = self.id_sequence.fetch_add(1, Ordering::SeqCst);
        let id =
            JobId::new(format!("job-{sequence:016x}")).map_err(SchedulerError::InvalidRequest)?;
        let record = JobRecord::new(id, owner.clone(), request.clone(), now);
        let mut graph = current;
        graph.push(record.clone());
        if has_dependency_cycle(&graph) {
            return Err(SchedulerError::DependencyCycle);
        }

        let dedupe = request.idempotency_key().cloned().map(|key| {
            (
                DedupeScope::new(&owner, request.handler(), key),
                now.saturating_add(self.config.dedupe_window),
            )
        });
        match self.store.insert(
            record,
            dedupe,
            now,
            self.config.queue_capacity,
            self.config.record_capacity,
        ) {
            Ok(InsertOutcome::Inserted(record)) => {
                self.emit_for_record(now, JobEventCode::EnqueueAccepted, &record, None);
                Ok(EnqueueResult {
                    record,
                    deduplicated: false,
                })
            }
            Ok(InsertOutcome::Existing(record)) => {
                self.emit_for_record(now, JobEventCode::Deduplicated, &record, None);
                Ok(EnqueueResult {
                    record,
                    deduplicated: true,
                })
            }
            Err(JobStoreError::Capacity) => {
                self.emit(
                    now,
                    JobEventCode::QueueSaturated,
                    None,
                    Some(request.handler().clone()),
                    None,
                    None,
                    Some(JobEventReason::QueueCapacity),
                    None,
                );
                Err(SchedulerError::QueueFull)
            }
            Err(JobStoreError::DuplicateId) => Err(SchedulerError::DuplicateJobId),
            Err(error) => Err(SchedulerError::Store(error)),
        }
    }

    pub fn inspect(&self, caller: &CallerRef, id: &JobId) -> Result<JobRecord, SchedulerError> {
        let record = self
            .store
            .get(id)
            .map_err(SchedulerError::Store)?
            .ok_or(SchedulerError::NotFound)?;
        self.authorization
            .authorize_user(caller, record.owner(), JobAction::Inspect, Some(id))
            .map_err(SchedulerError::Authorization)?;
        Ok(record)
    }

    /// Returns records the adapter explicitly authorizes for inspection.
    /// Denied owners are omitted; store failures remain errors.
    pub fn list_visible(&self, caller: &CallerRef) -> Result<Vec<JobRecord>, SchedulerError> {
        let records = self.store.list().map_err(SchedulerError::Store)?;
        Ok(records
            .into_iter()
            .filter(|record| {
                self.authorization
                    .authorize_user(caller, record.owner(), JobAction::List, Some(record.id()))
                    .is_ok()
            })
            .collect())
    }

    pub fn cancel(&self, caller: &CallerRef, id: &JobId) -> Result<JobRecord, SchedulerError> {
        let now = self.clock.now();
        let mut record = self
            .store
            .get(id)
            .map_err(SchedulerError::Store)?
            .ok_or(SchedulerError::NotFound)?;
        self.authorization
            .authorize_user(caller, record.owner(), JobAction::Cancel, Some(id))
            .map_err(SchedulerError::Authorization)?;
        if record.state().is_terminal() || matches!(record.state(), JobState::Cancelled) {
            return Ok(record);
        }
        if matches!(record.state(), JobState::CancelRequested) {
            if let Ok(runtime) = self.runtime.lock() {
                if let Some(control) = runtime.active.get(id) {
                    control.request_cancel();
                }
            }
            return Ok(record);
        }
        if record.state().occupies_slot() {
            record
                .request_cancel(now)
                .map_err(|_| SchedulerError::InvalidTransition)?;
        } else {
            record
                .set_state(JobState::Cancelled, now)
                .map_err(|_| SchedulerError::InvalidTransition)?;
            record.mark_finished(now);
        }
        let saved = self.save_record(record)?;
        if matches!(saved.state(), JobState::CancelRequested) {
            if let Ok(runtime) = self.runtime.lock() {
                if let Some(control) = runtime.active.get(id) {
                    control.request_cancel();
                }
            }
        }
        self.emit_for_record(now, JobEventCode::CancellationRequested, &saved, None);
        Ok(saved)
    }

    pub fn pause(&self, caller: &CallerRef, id: &JobId) -> Result<JobRecord, SchedulerError> {
        let mut record = self
            .store
            .get(id)
            .map_err(SchedulerError::Store)?
            .ok_or(SchedulerError::NotFound)?;
        self.authorization
            .authorize_user(caller, record.owner(), JobAction::Pause, Some(id))
            .map_err(SchedulerError::Authorization)?;
        let provider = self
            .providers
            .get(record.handler())?
            .ok_or(SchedulerError::MissingHandler)?;
        if !provider.supports_pause() {
            return Err(SchedulerError::UnsupportedPause);
        }
        let now = self.clock.now();
        if matches!(record.state(), JobState::Paused) {
            return Ok(record);
        }
        if matches!(record.state(), JobState::PauseRequested) {
            if let Ok(runtime) = self.runtime.lock() {
                if let Some(control) = runtime.active.get(id) {
                    control.request_pause();
                }
            }
            return Ok(record);
        }
        if record.state().occupies_slot() {
            record
                .set_state(JobState::PauseRequested, now)
                .map_err(|_| SchedulerError::InvalidTransition)?;
        } else {
            record
                .set_state(JobState::Paused, now)
                .map_err(|_| SchedulerError::InvalidTransition)?;
        }
        let saved = self.save_record(record)?;
        if matches!(saved.state(), JobState::PauseRequested) {
            if let Ok(runtime) = self.runtime.lock() {
                if let Some(control) = runtime.active.get(id) {
                    control.request_pause();
                }
            }
        }
        self.emit_for_record(now, JobEventCode::PauseRequested, &saved, None);
        Ok(saved)
    }

    pub fn resume(&self, caller: &CallerRef, id: &JobId) -> Result<JobRecord, SchedulerError> {
        let mut record = self
            .store
            .get(id)
            .map_err(SchedulerError::Store)?
            .ok_or(SchedulerError::NotFound)?;
        self.authorization
            .authorize_user(caller, record.owner(), JobAction::Resume, Some(id))
            .map_err(SchedulerError::Authorization)?;
        if !matches!(record.state(), JobState::Paused) {
            return Err(SchedulerError::InvalidTransition);
        }
        let now = self.clock.now();
        record
            .set_state(JobState::Queued, now)
            .map_err(|_| SchedulerError::InvalidTransition)?;
        let saved = self.save_record(record)?;
        self.emit_for_record(now, JobEventCode::Resumed, &saved, None);
        Ok(saved)
    }

    /// Explicitly retries a recovery-required record after operator repair.
    /// The adapter must authorize this independently, and handler authority is
    /// checked again immediately before execution.
    pub fn retry_recovery(
        &self,
        caller: &CallerRef,
        id: &JobId,
    ) -> Result<JobRecord, SchedulerError> {
        let mut record = self
            .store
            .get(id)
            .map_err(SchedulerError::Store)?
            .ok_or(SchedulerError::NotFound)?;
        self.authorization
            .authorize_user(caller, record.owner(), JobAction::Recover, Some(id))
            .map_err(SchedulerError::Authorization)?;
        if !matches!(record.state(), JobState::RecoveryRequired(_)) {
            return Err(SchedulerError::InvalidTransition);
        }
        let now = self.clock.now();
        record
            .set_state(JobState::Queued, now)
            .map_err(|_| SchedulerError::InvalidTransition)?;
        let saved = self.save_record(record)?;
        self.emit_for_record(now, JobEventCode::StateTransition, &saved, None);
        Ok(saved)
    }

    /// Runs one eligible registered provider synchronously. Tests can advance
    /// the virtual clock or cancel from another thread while it cooperates.
    pub fn dispatch_one(&self) -> Result<DispatchResult, SchedulerError> {
        self.refresh_pending()?;
        let now = self.clock.now();
        let conditions = *self
            .conditions
            .lock()
            .map_err(|_| SchedulerError::RuntimeStatePoisoned)?;
        let selected = {
            let _dispatch = self
                .dispatch_lock
                .lock()
                .map_err(|_| SchedulerError::RuntimeStatePoisoned)?;
            let mut candidates = self
                .store
                .list()
                .map_err(SchedulerError::Store)?
                .into_iter()
                .filter(|record| matches!(record.state(), JobState::Queued))
                .filter(|record| self.conditions_allow(record, conditions))
                .collect::<Vec<_>>();
            candidates.sort_by(|left, right| {
                let left_priority =
                    effective_priority(left, now, self.config.priority_aging_interval);
                let right_priority =
                    effective_priority(right, now, self.config.priority_aging_interval);
                right_priority
                    .cmp(&left_priority)
                    .then_with(|| left.enqueue_sequence().cmp(&right.enqueue_sequence()))
            });
            let mut selected = None;
            for candidate in candidates {
                let Some(provider) = self.providers.get(candidate.handler())? else {
                    self.fail_record(
                        candidate,
                        JobFailureCode::MissingHandler,
                        JobEventCode::HandlerMissing,
                    )?;
                    continue;
                };
                let claimed = match self.store.claim(
                    candidate.id(),
                    candidate.revision(),
                    now,
                    self.config.global_concurrency,
                    self.config.per_owner_concurrency,
                ) {
                    Ok(Some(record)) => record,
                    Ok(None) => continue,
                    Err(JobStoreError::Conflict) => continue,
                    Err(error) => return Err(SchedulerError::Store(error)),
                };
                selected = Some((claimed, provider));
                break;
            }
            selected
        };

        let Some((claimed, provider)) = selected else {
            return Ok(DispatchResult::Idle);
        };

        let control = Arc::new(RunControl::default());
        self.runtime
            .lock()
            .map_err(|_| SchedulerError::RuntimeStatePoisoned)?
            .active
            .insert(claimed.id().clone(), control.clone());
        self.emit_for_record(now, JobEventCode::Started, &claimed, None);

        if self
            .authorization
            .authorize_handler(claimed.owner(), claimed.handler())
            .is_err()
        {
            let failed = self.complete(
                claimed,
                HandlerOutcome::Failed(JobFailureCode::PermissionDenied),
                &control,
            )?;
            self.remove_active(failed.id());
            self.emit_for_record(
                self.clock.now(),
                JobEventCode::PermissionDenied,
                &failed,
                Some(JobEventReason::PermissionDenied),
            );
            return Ok(DispatchResult::Completed(Box::new(failed)));
        }

        let context = self.make_handler_context(claimed.clone(), control.clone());
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let mut context = context;
            provider.execute(&mut context)
        }))
        .unwrap_or(HandlerOutcome::Failed(JobFailureCode::Permanent));
        let completed = self.complete(claimed, outcome, &control)?;
        self.remove_active(completed.id());
        Ok(DispatchResult::Completed(Box::new(completed)))
    }

    /// Reconciles persisted running records after a process restart. Safe
    /// idempotent/checkpointed work is queued for at-least-once replay;
    /// non-idempotent work remains recovery-required for explicit repair.
    pub fn recover_startup(&self) -> Result<Vec<JobRecord>, SchedulerError> {
        self.runtime
            .lock()
            .map_err(|_| SchedulerError::RuntimeStatePoisoned)?
            .active
            .clear();
        let records = self.store.list().map_err(SchedulerError::Store)?;
        if has_dependency_cycle(&records) {
            return Err(SchedulerError::DependencyCycle);
        }
        let now = self.clock.now();
        let mut recovered = Vec::new();
        for record in records.into_iter().filter(|record| {
            matches!(
                record.state(),
                JobState::Running
                    | JobState::CancelRequested
                    | JobState::PauseRequested
                    | JobState::Interrupted
            )
        }) {
            let interrupted = if matches!(record.state(), JobState::Interrupted) {
                record.clone()
            } else {
                let mut interrupted = record.clone();
                interrupted
                    .set_state(JobState::Interrupted, now)
                    .map_err(|_| SchedulerError::InvalidTransition)?;
                self.store
                    .compare_and_swap(record.id(), record.revision(), interrupted)
                    .map_err(SchedulerError::Store)?
            };
            let provider = self.providers.get(interrupted.handler())?;
            let policy = provider
                .as_ref()
                .map(|provider| provider.recovery_policy(&interrupted));
            let target = match (provider, policy) {
                (Some(_), Some(_)) if !interrupted.has_execution_capacity() => {
                    JobState::RecoveryRequired(JobFailureCode::Permanent)
                }
                (None, _) => JobState::RecoveryRequired(JobFailureCode::MissingHandler),
                (Some(_), Some(_))
                    if self
                        .authorization
                        .authorize_handler(interrupted.owner(), interrupted.handler())
                        .is_err() =>
                {
                    JobState::RecoveryRequired(JobFailureCode::PermissionDenied)
                }
                (Some(_), Some(RecoveryPolicy::Idempotent)) => JobState::Queued,
                (Some(provider), Some(RecoveryPolicy::VerifiedCheckpoint)) => {
                    if interrupted
                        .checkpoint_ref()
                        .is_some_and(|reference| provider.verify_checkpoint(reference))
                    {
                        JobState::Queued
                    } else {
                        JobState::RecoveryRequired(JobFailureCode::InvalidCheckpoint)
                    }
                }
                (Some(_), Some(RecoveryPolicy::NonIdempotent)) | (_, None) => {
                    JobState::RecoveryRequired(JobFailureCode::Permanent)
                }
            };
            let mut reconciled = interrupted.clone();
            reconciled
                .set_state(target.clone(), now)
                .map_err(|_| SchedulerError::InvalidTransition)?;
            if target.is_terminal() {
                reconciled.mark_finished(now);
            }
            let reconciled = self
                .store
                .compare_and_swap(interrupted.id(), interrupted.revision(), reconciled)
                .map_err(SchedulerError::Store)?;
            let code = if matches!(reconciled.state(), JobState::Queued) {
                JobEventCode::RecoveryResumed
            } else {
                JobEventCode::RecoveryRequired
            };
            let reason = if matches!(reconciled.state(), JobState::Queued) {
                None
            } else {
                Some(JobEventReason::UnsafeRecovery)
            };
            self.emit_for_record(now, code, &reconciled, reason);
            recovered.push(reconciled);
        }
        Ok(recovered)
    }

    fn refresh_pending(&self) -> Result<(), SchedulerError> {
        let records = self.store.list().map_err(SchedulerError::Store)?;
        if has_dependency_cycle(&records) {
            self.emit(
                self.clock.now(),
                JobEventCode::StoreCorruption,
                None,
                None,
                None,
                None,
                Some(JobEventReason::CorruptRecord),
                None,
            );
            return Err(SchedulerError::DependencyCycle);
        }
        let by_id = records
            .iter()
            .map(|record| (record.id().clone(), record.clone()))
            .collect::<HashMap<_, _>>();
        let now = self.clock.now();
        for mut record in records {
            if matches!(record.state(), JobState::Running | JobState::PauseRequested) {
                let timed_out = record
                    .started_at()
                    .zip(record.constraints().effective_runtime())
                    .is_some_and(|(start, runtime)| now.0 >= start.0.saturating_add(runtime.0));
                let deadline_passed = record
                    .constraints()
                    .deadline()
                    .is_some_and(|deadline| now >= deadline);
                if timed_out || deadline_passed {
                    record
                        .request_timeout(now)
                        .map_err(|_| SchedulerError::InvalidTransition)?;
                    let Some(saved) = self.save_record_if_current(record)? else {
                        // A handler completion or another refresh won this
                        // revision. The next refresh reads its committed state.
                        continue;
                    };
                    if let Ok(runtime) = self.runtime.lock() {
                        if let Some(control) = runtime.active.get(saved.id()) {
                            control.request_timeout();
                        }
                    }
                    self.emit_for_record(
                        now,
                        JobEventCode::TimedOut,
                        &saved,
                        Some(JobEventReason::Timeout),
                    );
                    continue;
                }
            }
            if record.state().is_terminal() || record.state().occupies_slot() {
                continue;
            }

            if record
                .constraints()
                .deadline()
                .is_some_and(|deadline| now >= deadline)
            {
                record
                    .set_state(JobState::TimedOut, now)
                    .map_err(|_| SchedulerError::InvalidTransition)?;
                record.mark_finished(now);
                if !self.save_transition_if_current(
                    record,
                    JobEventCode::TimedOut,
                    Some(JobEventReason::DeadlineExceeded),
                )? {
                    continue;
                }
                continue;
            }

            if matches!(
                record.state(),
                JobState::Paused | JobState::Interrupted | JobState::RecoveryRequired(_)
            ) {
                continue;
            }

            if matches!(record.state(), JobState::Queued) && !record.has_execution_capacity() {
                record
                    .set_state(JobState::RecoveryRequired(JobFailureCode::Permanent), now)
                    .map_err(|_| SchedulerError::InvalidTransition)?;
                if !self.save_transition_if_current(
                    record,
                    JobEventCode::RecoveryRequired,
                    Some(JobEventReason::UnsafeRecovery),
                )? {
                    continue;
                }
                continue;
            }

            if matches!(record.state(), JobState::RetryWait)
                && record.retry_at().is_some_and(|retry_at| now >= retry_at)
            {
                record
                    .set_state(JobState::Queued, now)
                    .map_err(|_| SchedulerError::InvalidTransition)?;
                let Some(saved) = self.save_record_if_current(record)? else {
                    continue;
                };
                record = saved;
                self.emit_for_record(now, JobEventCode::StateTransition, &record, None);
            }
            let dep_result = dependency_result(&record, &by_id);
            match dep_result {
                DependencyResult::Ready
                    if matches!(record.state(), JobState::WaitingOnDependency) =>
                {
                    record
                        .set_state(JobState::Queued, now)
                        .map_err(|_| SchedulerError::InvalidTransition)?;
                    if !self.save_transition_if_current(
                        record,
                        JobEventCode::DependencyReleased,
                        None,
                    )? {
                        continue;
                    }
                }
                DependencyResult::Pending if matches!(record.state(), JobState::Queued) => {
                    record
                        .set_state(JobState::WaitingOnDependency, now)
                        .map_err(|_| SchedulerError::InvalidTransition)?;
                    if !self.save_transition_if_current(
                        record,
                        JobEventCode::DependencyBlocked,
                        Some(JobEventReason::DependencyPending),
                    )? {
                        continue;
                    }
                }
                DependencyResult::PendingAndFailed => match record.dependency_failure_policy() {
                    DependencyFailurePolicy::CancelDependent => {
                        record
                            .set_state(JobState::Cancelled, now)
                            .map_err(|_| SchedulerError::InvalidTransition)?;
                        record.mark_finished(now);
                        if !self.save_transition_if_current(
                            record,
                            JobEventCode::StateTransition,
                            Some(JobEventReason::DependencyFailed),
                        )? {
                            continue;
                        }
                    }
                    DependencyFailurePolicy::Block | DependencyFailurePolicy::AllowFailure
                        if matches!(record.state(), JobState::Queued) =>
                    {
                        record
                            .set_state(JobState::WaitingOnDependency, now)
                            .map_err(|_| SchedulerError::InvalidTransition)?;
                        if !self.save_transition_if_current(
                            record,
                            JobEventCode::DependencyBlocked,
                            Some(JobEventReason::DependencyPending),
                        )? {
                            continue;
                        }
                    }
                    DependencyFailurePolicy::Block | DependencyFailurePolicy::AllowFailure => {}
                },
                DependencyResult::Failed
                    if matches!(
                        record.state(),
                        JobState::Queued | JobState::WaitingOnDependency
                    ) =>
                {
                    match record.dependency_failure_policy() {
                        DependencyFailurePolicy::CancelDependent => {
                            record
                                .set_state(JobState::Cancelled, now)
                                .map_err(|_| SchedulerError::InvalidTransition)?;
                            record.mark_finished(now);
                            if !self.save_transition_if_current(
                                record,
                                JobEventCode::StateTransition,
                                Some(JobEventReason::DependencyFailed),
                            )? {
                                continue;
                            }
                        }
                        DependencyFailurePolicy::Block => {
                            if matches!(record.state(), JobState::Queued) {
                                record
                                    .set_state(JobState::WaitingOnDependency, now)
                                    .map_err(|_| SchedulerError::InvalidTransition)?;
                                if !self.save_transition_if_current(
                                    record,
                                    JobEventCode::DependencyBlocked,
                                    Some(JobEventReason::DependencyFailed),
                                )? {
                                    continue;
                                }
                            }
                        }
                        DependencyFailurePolicy::AllowFailure => {
                            if matches!(record.state(), JobState::WaitingOnDependency) {
                                record
                                    .set_state(JobState::Queued, now)
                                    .map_err(|_| SchedulerError::InvalidTransition)?;
                                if !self.save_transition_if_current(
                                    record,
                                    JobEventCode::DependencyReleased,
                                    None,
                                )? {
                                    continue;
                                }
                            }
                        }
                    }
                }
                DependencyResult::Ready | DependencyResult::Pending | DependencyResult::Failed => {}
            }
        }
        Ok(())
    }

    fn conditions_allow(&self, record: &JobRecord, conditions: SystemConditions) -> bool {
        let network_ok = match record.constraints().network_policy() {
            NetworkPolicy::Any => true,
            NetworkPolicy::OfflineOnly => !conditions.network.available,
            NetworkPolicy::RequireNetwork => conditions.network.available,
            NetworkPolicy::RequireUnmeteredNetwork => {
                conditions.network.available && conditions.network.unmetered
            }
        };
        network_ok
            && record
                .constraints()
                .required_conditions()
                .iter()
                .all(|condition| match condition {
                    ExecutionCondition::SystemIdle => conditions.system_idle,
                    ExecutionCondition::ExternalPower => conditions.external_power,
                })
    }

    fn make_handler_context(&self, record: JobRecord, control: Arc<RunControl>) -> HandlerContext {
        let id = record.id().clone();
        let clock = self.clock.clone();
        let store = self.store.clone();
        let diagnostics = self.diagnostics.clone();
        let runtime = self.runtime.clone();
        let progress_interval = self.config.progress_interval;
        let progress_id = id.clone();
        let progress_handler = record.handler().clone();
        let progress_correlation = correlation_id(&record);
        let progress = move |summary: ProgressSummary| {
            let now = clock.now();
            {
                let mut state = runtime
                    .lock()
                    .map_err(|_| ProgressError::StoreUnavailable)?;
                if state
                    .last_progress
                    .get(&progress_id)
                    .is_some_and(|last| now.0 < last.0.saturating_add(progress_interval.0))
                {
                    return Err(ProgressError::RateLimited);
                }
                state.last_progress.insert(progress_id.clone(), now);
            }
            let mut current = store
                .get(&progress_id)
                .map_err(|_| ProgressError::StoreUnavailable)?
                .ok_or(ProgressError::StoreUnavailable)?;
            if !current.state().occupies_slot() {
                return Err(ProgressError::StoreUnavailable);
            }
            current.set_progress(summary.clone(), now);
            let saved = store
                .compare_and_swap(&progress_id, current.revision(), current)
                .map_err(|_| ProgressError::StoreUnavailable)?;
            let event = JobDiagnosticEvent::new(
                now,
                JobEventCode::Progress,
                JobDiagnosticDetails {
                    job_id: Some(saved.id().clone()),
                    handler: Some(progress_handler.clone()),
                    attempt: Some(saved.attempt_count()),
                    correlation: progress_correlation.clone(),
                    progress: Some(summary),
                    ..JobDiagnosticDetails::default()
                },
            );
            let _ = diagnostics.record(event);
            Ok(())
        };

        let checkpoint_id = id.clone();
        let checkpoint_store = self.store.clone();
        let checkpoint = move |reference: CheckpointRef| {
            let mut current = checkpoint_store
                .get(&checkpoint_id)
                .map_err(|_| ProgressError::StoreUnavailable)?
                .ok_or(ProgressError::StoreUnavailable)?;
            if !current.state().occupies_slot() {
                return Err(ProgressError::StoreUnavailable);
            }
            current.set_checkpoint(reference);
            let _ = checkpoint_store
                .compare_and_swap(&checkpoint_id, current.revision(), current)
                .map_err(|_| ProgressError::StoreUnavailable)?;
            Ok(())
        };

        HandlerContext {
            record,
            cancellation: CancellationToken::requested(control.clone()),
            control,
            progress: Box::new(progress),
            checkpoint: Box::new(checkpoint),
        }
    }

    fn complete(
        &self,
        claimed: JobRecord,
        outcome: HandlerOutcome,
        control: &Arc<RunControl>,
    ) -> Result<JobRecord, SchedulerError> {
        let now = self.clock.now();
        let mut record = self
            .store
            .get(claimed.id())
            .map_err(SchedulerError::Store)?
            .ok_or(SchedulerError::NotFound)?;
        let elapsed_timeout = record
            .started_at()
            .zip(record.constraints().effective_runtime())
            .is_some_and(|(start, runtime)| now.0 >= start.0.saturating_add(runtime.0));
        let deadline_passed = record
            .constraints()
            .deadline()
            .is_some_and(|deadline| now >= deadline);
        let token_reason = CancellationToken::requested(control.clone()).reason();

        let final_state = if elapsed_timeout
            || deadline_passed
            || token_reason == Some(CancellationReason::TimedOut)
        {
            JobState::TimedOut
        } else if record.stop_reason() == Some(StopReason::Cancelled) {
            match outcome {
                HandlerOutcome::Succeeded => JobState::Succeeded,
                _ => JobState::Cancelled,
            }
        } else {
            match outcome {
                HandlerOutcome::Succeeded => JobState::Succeeded,
                HandlerOutcome::Cancelled => JobState::Cancelled,
                HandlerOutcome::Paused(checkpoint) => {
                    if !matches!(record.state(), JobState::PauseRequested) {
                        JobState::Failed(JobFailureCode::Permanent)
                    } else {
                        if let Some(checkpoint) = checkpoint {
                            record.set_checkpoint(checkpoint);
                        }
                        JobState::Paused
                    }
                }
                HandlerOutcome::Failed(code) => {
                    let retryable = record.retry_policy().retryable().contains(&code)
                        && record.attempt_count() < record.retry_policy().max_attempts()
                        && !deadline_passed;
                    if retryable {
                        let delay = record.retry_policy().delay_after(record.attempt_count());
                        let retry_at = now.saturating_add(delay);
                        record
                            .set_state(JobState::RetryWait, now)
                            .map_err(|_| SchedulerError::InvalidTransition)?;
                        record.set_retry_at(retry_at);
                        let saved = self.save_record(record)?;
                        self.emit_for_record(
                            now,
                            JobEventCode::RetryScheduled,
                            &saved,
                            Some(JobEventReason::RetryableFailure),
                        );
                        return Ok(saved);
                    }
                    if record.attempt_count() >= record.retry_policy().max_attempts()
                        && record.retry_policy().exhausted_action()
                            == RetryTerminalAction::RecoveryRequired
                    {
                        JobState::RecoveryRequired(code)
                    } else {
                        JobState::Failed(
                            if record.attempt_count() >= record.retry_policy().max_attempts() {
                                JobFailureCode::Permanent
                            } else {
                                code
                            },
                        )
                    }
                }
            }
        };

        if matches!(final_state, JobState::Paused) {
            record
                .set_state(final_state, now)
                .map_err(|_| SchedulerError::InvalidTransition)?;
            let saved = self.save_record(record)?;
            self.emit_for_record(now, JobEventCode::Paused, &saved, None);
            return Ok(saved);
        }

        record
            .set_state(final_state.clone(), now)
            .map_err(|_| SchedulerError::InvalidTransition)?;
        if final_state.is_terminal() {
            record.mark_finished(now);
        }
        let saved = self.save_record(record)?;
        let code = match saved.state() {
            JobState::Cancelled => JobEventCode::CancellationCompleted,
            JobState::TimedOut => JobEventCode::TimedOut,
            JobState::Failed(_) => JobEventCode::HandlerFailed,
            JobState::RecoveryRequired(_) => JobEventCode::RecoveryRequired,
            _ => JobEventCode::StateTransition,
        };
        self.emit_for_record(now, code, &saved, None);
        Ok(saved)
    }

    fn fail_record(
        &self,
        mut record: JobRecord,
        code: JobFailureCode,
        event: JobEventCode,
    ) -> Result<(), SchedulerError> {
        let now = self.clock.now();
        record
            .set_state(JobState::Failed(code), now)
            .map_err(|_| SchedulerError::InvalidTransition)?;
        record.mark_finished(now);
        let saved = self.save_record(record)?;
        self.emit_for_record(now, event, &saved, Some(reason_for_failure(code)));
        Ok(())
    }

    fn save_record_if_current(
        &self,
        record: JobRecord,
    ) -> Result<Option<JobRecord>, SchedulerError> {
        match self.save_record(record) {
            Ok(saved) => Ok(Some(saved)),
            Err(SchedulerError::Store(JobStoreError::Conflict)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn save_transition_if_current(
        &self,
        record: JobRecord,
        event: JobEventCode,
        reason: Option<JobEventReason>,
    ) -> Result<bool, SchedulerError> {
        let now = self.clock.now();
        let Some(saved) = self.save_record_if_current(record)? else {
            return Ok(false);
        };
        self.emit_for_record(now, event, &saved, reason);
        Ok(true)
    }

    fn remove_active(&self, id: &JobId) {
        if let Ok(mut runtime) = self.runtime.lock() {
            runtime.active.remove(id);
            runtime.last_progress.remove(id);
        }
    }

    fn save_record(&self, record: JobRecord) -> Result<JobRecord, SchedulerError> {
        let id = record.id().clone();
        let revision = record.revision();
        self.store
            .compare_and_swap(&id, revision, record)
            .map_err(SchedulerError::Store)
    }

    fn emit_for_record(
        &self,
        now: TimestampMillis,
        code: JobEventCode,
        record: &JobRecord,
        reason: Option<JobEventReason>,
    ) {
        self.emit(
            now,
            code,
            Some(record.id().clone()),
            Some(record.handler().clone()),
            Some(record.attempt_count()),
            correlation_id(record),
            reason,
            record.progress().cloned(),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn emit(
        &self,
        now: TimestampMillis,
        code: JobEventCode,
        job_id: Option<JobId>,
        handler: Option<HandlerRef>,
        attempt: Option<u8>,
        correlation: Option<JobId>,
        reason: Option<JobEventReason>,
        progress: Option<ProgressSummary>,
    ) {
        let _ = self.diagnostics.record(JobDiagnosticEvent::new(
            now,
            code,
            JobDiagnosticDetails {
                job_id,
                handler,
                attempt,
                correlation,
                reason,
                progress,
            },
        ));
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnqueueResult {
    pub record: JobRecord,
    pub deduplicated: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DependencyResult {
    Ready,
    Pending,
    Failed,
    PendingAndFailed,
}

fn dependency_result(record: &JobRecord, records: &HashMap<JobId, JobRecord>) -> DependencyResult {
    let mut pending = false;
    let mut failed = false;
    for dependency in record.dependencies() {
        let Some(prerequisite) = records.get(dependency.prerequisite()) else {
            failed = true;
            continue;
        };
        if !prerequisite.state().is_terminal() {
            pending = true;
        } else if dependency.requires_success()
            && !matches!(prerequisite.state(), JobState::Succeeded)
        {
            failed = true;
        }
    }
    match (pending, failed) {
        (true, true) => DependencyResult::PendingAndFailed,
        (true, false) => DependencyResult::Pending,
        (false, true) => DependencyResult::Failed,
        (false, false) => DependencyResult::Ready,
    }
}

fn has_dependency_cycle(records: &[JobRecord]) -> bool {
    let ids = records
        .iter()
        .map(|record| record.id().clone())
        .collect::<std::collections::HashSet<_>>();
    if ids.len() != records.len() {
        return true;
    }
    let mut remaining = HashMap::<JobId, usize>::new();
    let mut dependents = HashMap::<JobId, Vec<JobId>>::new();
    for record in records {
        let prerequisites = record
            .dependencies()
            .iter()
            .filter(|dependency| ids.contains(dependency.prerequisite()))
            .collect::<Vec<_>>();
        remaining.insert(record.id().clone(), prerequisites.len());
        for dependency in prerequisites {
            dependents
                .entry(dependency.prerequisite().clone())
                .or_default()
                .push(record.id().clone());
        }
    }
    let mut ready = std::collections::VecDeque::from_iter(
        remaining
            .iter()
            .filter_map(|(id, count)| (*count == 0).then_some(id.clone())),
    );
    let mut removed = 0usize;
    while let Some(prerequisite) = ready.pop_front() {
        removed += 1;
        for dependent in dependents.get(&prerequisite).into_iter().flatten() {
            let count = remaining.get_mut(dependent).expect("known dependent node");
            *count -= 1;
            if *count == 0 {
                ready.push_back(dependent.clone());
            }
        }
    }
    removed != records.len()
}

fn effective_priority(record: &JobRecord, now: TimestampMillis, interval: DurationMillis) -> u8 {
    let waited = now.0.saturating_sub(record.created_at().0);
    record
        .priority()
        .rank()
        .saturating_add((waited / interval.0.max(1)).min(3) as u8)
        .min(JobPriority::Urgent.rank())
}

fn correlation_id(record: &JobRecord) -> Option<JobId> {
    record
        .correlation_ref()
        .and_then(|reference| JobId::new(reference.as_str().to_owned()).ok())
}

fn reason_for_failure(code: JobFailureCode) -> JobEventReason {
    match code {
        JobFailureCode::PermissionDenied => JobEventReason::PermissionDenied,
        JobFailureCode::DeadlineExceeded => JobEventReason::DeadlineExceeded,
        JobFailureCode::Timeout => JobEventReason::Timeout,
        JobFailureCode::MissingHandler => JobEventReason::MissingHandler,
        JobFailureCode::DependencyFailed => JobEventReason::DependencyFailed,
        JobFailureCode::InvalidCheckpoint => JobEventReason::CheckpointRejected,
        JobFailureCode::StoreUnavailable => JobEventReason::StoreUnavailable,
        JobFailureCode::UnsupportedSchema => JobEventReason::UnsupportedSchema,
        _ => JobEventReason::RetryExhausted,
    }
}
