use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use super::*;

#[derive(Default)]
struct TestAuthority {
    denied_actions: Mutex<HashSet<(String, JobAction)>>,
    deny_handler: AtomicBool,
    allow_urgent: AtomicBool,
    unavailable_priority: AtomicBool,
}

impl TestAuthority {
    fn deny_action(&self, caller: &str, action: JobAction) {
        self.denied_actions
            .lock()
            .unwrap()
            .insert((caller.to_owned(), action));
    }

    fn allow_action(&self, caller: &str, action: JobAction) {
        self.denied_actions
            .lock()
            .unwrap()
            .remove(&(caller.to_owned(), action));
    }
}

impl JobAuthorization for TestAuthority {
    fn owner_for(&self, caller: &CallerRef) -> Result<JobOwner, AuthorizationError> {
        Ok(JobOwner::from_authenticated_adapter(
            caller.principal().clone(),
            caller.profile().cloned(),
        ))
    }

    fn authorize_user(
        &self,
        caller: &CallerRef,
        owner: &JobOwner,
        action: JobAction,
        _job_id: Option<&JobId>,
    ) -> Result<(), AuthorizationError> {
        let denied = self
            .denied_actions
            .lock()
            .unwrap()
            .contains(&(caller.principal().as_str().to_owned(), action));
        if denied || caller.principal() != owner.principal() {
            Err(AuthorizationError::denied())
        } else {
            Ok(())
        }
    }

    fn authorize_priority(
        &self,
        _caller: &CallerRef,
        _owner: &JobOwner,
        _handler: &HandlerRef,
        priority: JobPriority,
    ) -> Result<(), AuthorizationError> {
        if self.unavailable_priority.load(Ordering::SeqCst) {
            Err(AuthorizationError::unavailable())
        } else if priority == JobPriority::Urgent && !self.allow_urgent.load(Ordering::SeqCst) {
            Err(AuthorizationError::denied())
        } else {
            Ok(())
        }
    }

    fn authorize_handler(
        &self,
        _owner: &JobOwner,
        _handler: &HandlerRef,
    ) -> Result<(), AuthorizationError> {
        if self.deny_handler.load(Ordering::SeqCst) {
            Err(AuthorizationError::denied())
        } else {
            Ok(())
        }
    }
}

enum HandlerMode {
    Script(Mutex<VecDeque<HandlerOutcome>>),
    WaitFirst(Arc<Barrier>),
    OutcomeUnknownAfterStop(Arc<Barrier>),
    OutcomeUnknownAfterDeadline(Arc<ManualClock>),
    PauseFirst(Arc<Barrier>),
    ReportProgress(Arc<Mutex<Vec<Result<(), ProgressError>>>>),
}

struct TestHandler {
    reference: HandlerRef,
    mode: HandlerMode,
    recovery: RecoveryPolicy,
    supports_pause: bool,
    checkpoint_valid: AtomicBool,
    calls: AtomicUsize,
}

impl TestHandler {
    fn scripted(reference: HandlerRef, outcomes: impl IntoIterator<Item = HandlerOutcome>) -> Self {
        Self {
            reference,
            mode: HandlerMode::Script(Mutex::new(outcomes.into_iter().collect())),
            recovery: RecoveryPolicy::NonIdempotent,
            supports_pause: false,
            checkpoint_valid: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        }
    }

    fn with_recovery(mut self, recovery: RecoveryPolicy) -> Self {
        self.recovery = recovery;
        self
    }

    fn with_pause(mut self, supports_pause: bool) -> Self {
        self.supports_pause = supports_pause;
        self
    }

    fn with_checkpoint_valid(self, valid: bool) -> Self {
        self.checkpoint_valid.store(valid, Ordering::SeqCst);
        self
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl JobHandler for TestHandler {
    fn handler_ref(&self) -> HandlerRef {
        self.reference.clone()
    }

    fn recovery_policy(&self, _record: &JobRecord) -> RecoveryPolicy {
        self.recovery
    }

    fn verify_checkpoint(&self, _checkpoint: &CheckpointRef) -> bool {
        self.checkpoint_valid.load(Ordering::SeqCst)
    }

    fn supports_pause(&self) -> bool {
        self.supports_pause
    }

    fn execute(&self, context: &mut HandlerContext) -> HandlerOutcome {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        match &self.mode {
            HandlerMode::Script(outcomes) => outcomes
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(HandlerOutcome::Succeeded),
            HandlerMode::WaitFirst(started) if call == 1 => {
                let child_work = context.cancellation_token().child_token().child_token();
                started.wait();
                while !child_work.is_cancelled() {
                    thread::yield_now();
                }
                match child_work.reason() {
                    Some(CancellationReason::TimedOut) => {
                        HandlerOutcome::Failed(JobFailureCode::Timeout)
                    }
                    Some(CancellationReason::Requested) => HandlerOutcome::Cancelled,
                    Some(CancellationReason::ClockUnavailable) => {
                        HandlerOutcome::Failed(JobFailureCode::Permanent)
                    }
                    None => HandlerOutcome::Succeeded,
                }
            }
            HandlerMode::OutcomeUnknownAfterStop(started) if call == 1 => {
                started.wait();
                while context.cancellation_token().reason().is_none() {
                    thread::yield_now();
                }
                HandlerOutcome::OutcomeUnknown(JobFailureCode::Transient)
            }
            HandlerMode::OutcomeUnknownAfterDeadline(clock) => {
                clock.advance(DurationMillis(1));
                HandlerOutcome::OutcomeUnknown(JobFailureCode::Transient)
            }
            HandlerMode::PauseFirst(started) if call == 1 => {
                started.wait();
                while !context.pause_requested() {
                    thread::yield_now();
                }
                HandlerOutcome::Paused(Some(CheckpointRef::new("checkpoint-1").unwrap()))
            }
            HandlerMode::ReportProgress(results) => {
                let code = ProgressCode::new("phase-a").unwrap();
                let first = ProgressSummary::new(code.clone(), 2, Some(10)).unwrap();
                let second = ProgressSummary::new(code, 3, Some(10)).unwrap();
                let first_result = context.report_progress(first);
                let second_result = context.report_progress(second);
                results
                    .lock()
                    .unwrap()
                    .extend([first_result, second_result]);
                HandlerOutcome::Succeeded
            }
            _ => HandlerOutcome::Succeeded,
        }
    }
}

#[derive(Default)]
struct FailingDiagnostics;

impl JobDiagnosticSink for FailingDiagnostics {
    fn record(&self, _event: JobDiagnosticEvent) -> Result<(), JobDiagnosticSinkError> {
        Err(JobDiagnosticSinkError)
    }
}

#[derive(Clone, Copy)]
enum ReentrantCall {
    DispatchIdle,
    RecoverEmpty,
    RecoverOneFenced,
    CancelCurrent,
    PauseCurrent,
}

struct ReentrantDiagnostics {
    events: Arc<BoundedJobEventRecorder>,
    scheduler: Arc<Mutex<Option<Scheduler>>>,
    trigger: JobEventCode,
    call: ReentrantCall,
    invoked: AtomicBool,
    completed: AtomicBool,
    worker: Mutex<Option<thread::JoinHandle<()>>>,
}

impl ReentrantDiagnostics {
    fn new(
        events: Arc<BoundedJobEventRecorder>,
        trigger: JobEventCode,
        call: ReentrantCall,
    ) -> Self {
        Self {
            events,
            scheduler: Arc::new(Mutex::new(None)),
            trigger,
            call,
            invoked: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            worker: Mutex::new(None),
        }
    }

    fn bind_scheduler(&self, scheduler: Scheduler) {
        *self.scheduler.lock().unwrap() = Some(scheduler);
    }

    fn finish_reentry(&self) -> bool {
        if let Some(worker) = self.worker.lock().unwrap().take() {
            worker
                .join()
                .expect("reentrant scheduler call should finish");
        }
        self.completed.load(Ordering::SeqCst)
    }

    fn detach_scheduler(&self) {
        self.scheduler.lock().unwrap().take();
    }
}

impl JobDiagnosticSink for ReentrantDiagnostics {
    fn record(&self, event: JobDiagnosticEvent) -> Result<(), JobDiagnosticSinkError> {
        let should_reenter = event.code() == self.trigger;
        let event_job_id = event.job_id().cloned();
        self.events.record(event)?;
        if !should_reenter || self.invoked.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let scheduler = self
            .scheduler
            .lock()
            .unwrap()
            .clone()
            .ok_or(JobDiagnosticSinkError)?;
        let call = self.call;
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let completed = match call {
                ReentrantCall::DispatchIdle => {
                    matches!(scheduler.dispatch_one(), Ok(DispatchResult::Idle))
                }
                ReentrantCall::RecoverEmpty => scheduler
                    .recover_startup()
                    .is_ok_and(|records| records.is_empty()),
                ReentrantCall::RecoverOneFenced => {
                    scheduler.recover_startup().is_ok_and(|records| {
                        records.len() == 1
                            && matches!(records[0].state(), JobState::RecoveryRequired(_))
                    })
                }
                ReentrantCall::CancelCurrent => event_job_id
                    .as_ref()
                    .is_some_and(|id| scheduler.cancel(&caller("user.alice"), id).is_ok()),
                ReentrantCall::PauseCurrent => event_job_id
                    .as_ref()
                    .is_some_and(|id| scheduler.pause(&caller("user.alice"), id).is_ok()),
            };
            let _ = sender.send(completed);
        });
        *self.worker.lock().unwrap() = Some(worker);

        let completed = receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap_or(false);
        self.completed.store(completed, Ordering::SeqCst);
        if completed {
            Ok(())
        } else {
            Err(JobDiagnosticSinkError)
        }
    }
}

struct PanickingDiagnostics {
    events: Arc<BoundedJobEventRecorder>,
    trigger: JobEventCode,
    panicked: AtomicBool,
}

impl JobDiagnosticSink for PanickingDiagnostics {
    fn record(&self, event: JobDiagnosticEvent) -> Result<(), JobDiagnosticSinkError> {
        let should_panic = event.code() == self.trigger;
        self.events.record(event)?;
        if should_panic && !self.panicked.swap(true, Ordering::SeqCst) {
            panic!("injected diagnostic sink panic");
        }
        Ok(())
    }
}

struct BlockingDiagnostics {
    events: Arc<BoundedJobEventRecorder>,
    order: Arc<Mutex<Vec<JobEventCode>>>,
    block_on: JobEventCode,
    entered: Arc<Barrier>,
    release: Arc<Barrier>,
    blocked: AtomicBool,
}

impl JobDiagnosticSink for BlockingDiagnostics {
    fn record(&self, event: JobDiagnosticEvent) -> Result<(), JobDiagnosticSinkError> {
        let code = event.code();
        self.events.record(event)?;
        self.order.lock().unwrap().push(code);
        if code == self.block_on && !self.blocked.swap(true, Ordering::SeqCst) {
            self.entered.wait();
            self.release.wait();
        }
        Ok(())
    }
}

fn reentrant_harness(
    trigger: JobEventCode,
    call: ReentrantCall,
) -> (Harness, Arc<ReentrantDiagnostics>) {
    let events = Arc::new(BoundedJobEventRecorder::new(32));
    let sink = Arc::new(ReentrantDiagnostics::new(events.clone(), trigger, call));
    let diagnostics: Arc<dyn JobDiagnosticSink> = sink.clone();
    let harness = harness_with_diagnostics(SchedulerConfig::default(), events, diagnostics);
    sink.bind_scheduler(harness.scheduler.clone());
    (harness, sink)
}

struct Harness {
    scheduler: Scheduler,
    store: Arc<InMemoryJobStore>,
    clock: Arc<ManualClock>,
    authority: Arc<TestAuthority>,
    providers: Arc<JobProviderRegistry>,
    events: Arc<BoundedJobEventRecorder>,
}

fn harness(config: SchedulerConfig) -> Harness {
    let events = Arc::new(BoundedJobEventRecorder::new(32));
    harness_with_diagnostics(config, events.clone(), events)
}

fn harness_with_diagnostics(
    config: SchedulerConfig,
    events: Arc<BoundedJobEventRecorder>,
    diagnostics: Arc<dyn JobDiagnosticSink>,
) -> Harness {
    let store = Arc::new(InMemoryJobStore::new());
    let clock = Arc::new(ManualClock::new(TimestampMillis(0)));
    let authority = Arc::new(TestAuthority::default());
    let providers = Arc::new(JobProviderRegistry::new());
    let scheduler = Scheduler::new(
        store.clone(),
        clock.clone(),
        authority.clone(),
        diagnostics,
        providers.clone(),
        config,
    )
    .unwrap();
    Harness {
        scheduler,
        store,
        clock,
        authority,
        providers,
        events,
    }
}

fn caller(id: &str) -> CallerRef {
    CallerRef::from_authenticated_adapter(PrincipalRef::new(id).unwrap(), None)
}

fn handler_ref(name: &str) -> HandlerRef {
    HandlerRef::new(
        OpaqueRef::new("test-provider").unwrap(),
        OpaqueRef::new(name).unwrap(),
        1,
    )
    .unwrap()
}

fn register(harness: &Harness, handler: TestHandler) -> Arc<TestHandler> {
    let handler = Arc::new(handler);
    harness.providers.register(handler.clone()).unwrap();
    handler
}

fn enqueue(harness: &Harness, caller: &CallerRef, handler: &HandlerRef) -> EnqueueResult {
    harness
        .scheduler
        .enqueue(caller, JobRequest::new(handler.clone()))
        .unwrap()
}

fn record(harness: &Harness, caller: &CallerRef, id: &JobId) -> JobRecord {
    harness.scheduler.inspect(caller, id).unwrap()
}

#[test]
fn records_are_versioned_validated_and_metadata_only() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler = handler_ref("index");
    register(
        &harness,
        TestHandler::scripted(handler.clone(), [HandlerOutcome::Succeeded]),
    );
    let request = JobRequest::new(handler)
        .with_request_ref(OpaqueRef::new("object-42").unwrap())
        .with_idempotency_key(IdempotencyKey::new("request-42").unwrap())
        .with_correlation_ref(OpaqueRef::new("corr-42").unwrap());
    let result = harness.scheduler.enqueue(&caller, request).unwrap();
    let encoded = encode_job_record(&result.record).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(json["schema_version"], JOB_RECORD_SCHEMA_VERSION);
    assert_eq!(json["handler"]["name"], "index");
    assert!(json.get("closure").is_none());
    assert!(json.get("credentials").is_none());
    assert!(json.get("input").is_none());
    assert!(!String::from_utf8(encoded.clone())
        .unwrap()
        .contains("secret-value"));
    assert_eq!(decode_job_record(&encoded).unwrap(), result.record);

    let mut unsupported = json;
    unsupported["schema_version"] = serde_json::json!(91);
    let error = decode_job_record(&serde_json::to_vec(&unsupported).unwrap()).unwrap_err();
    assert_eq!(
        error,
        JobRecordCodecError::InvalidRecord(ConstraintError::UnsupportedSchemaVersion)
    );
}

#[test]
fn retries_use_bounded_virtual_backoff_and_stop_at_attempt_limit() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler = handler_ref("retry");
    register(
        &harness,
        TestHandler::scripted(
            handler.clone(),
            [
                HandlerOutcome::Failed(JobFailureCode::Transient),
                HandlerOutcome::Failed(JobFailureCode::Transient),
                HandlerOutcome::Succeeded,
            ],
        ),
    );
    let retry = RetryPolicy::new(
        2,
        DurationMillis(100),
        DurationMillis(250),
        2_000,
        [JobFailureCode::Transient],
        RetryTerminalAction::Failed,
    )
    .unwrap();
    assert_eq!(retry.delay_after(1), DurationMillis(100));
    assert_eq!(retry.delay_after(2), DurationMillis(200));
    assert_eq!(retry.delay_after(3), DurationMillis(250));
    let result = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler.clone()).with_retry_policy(retry),
        )
        .unwrap();

    let first = harness.scheduler.dispatch_one().unwrap();
    assert!(matches!(first, DispatchResult::Completed(_)));
    assert_eq!(
        record(&harness, &caller, result.record.id()).state(),
        &JobState::RetryWait
    );
    assert_eq!(
        record(&harness, &caller, result.record.id()).attempt_count(),
        1
    );
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    harness.clock.advance(DurationMillis(100));
    let second = harness.scheduler.dispatch_one().unwrap();
    let DispatchResult::Completed(second_record) = second else {
        panic!("retry should execute after virtual backoff")
    };
    assert_eq!(
        second_record.state(),
        &JobState::Failed(JobFailureCode::Permanent)
    );
    assert_eq!(second_record.attempt_count(), 2);
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
}

#[test]
fn saturated_retry_deadline_at_max_remains_bounded() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler_ref = handler_ref("max-time-retry");
    let provider = register(
        &harness,
        TestHandler::scripted(
            handler_ref.clone(),
            [
                HandlerOutcome::Failed(JobFailureCode::Transient),
                HandlerOutcome::Failed(JobFailureCode::Transient),
                HandlerOutcome::Succeeded,
            ],
        ),
    );
    let retry = RetryPolicy::new(
        2,
        DurationMillis(10),
        DurationMillis(10),
        1_000,
        [JobFailureCode::Transient],
        RetryTerminalAction::Failed,
    )
    .unwrap();
    let before_max = TimestampMillis(u64::MAX - 5);
    let at_max = TimestampMillis(u64::MAX);
    harness.clock.set(before_max).unwrap();
    let queued = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler_ref).with_retry_policy(retry),
        )
        .unwrap()
        .record;

    assert!(matches!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Completed(_)
    ));
    let retry_wait = record(&harness, &caller, queued.id());
    assert_eq!(retry_wait.state(), &JobState::RetryWait);
    assert_eq!(retry_wait.retry_at(), Some(at_max));
    assert_eq!(retry_wait.attempt_count(), 1);
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );

    assert_eq!(harness.clock.advance(DurationMillis(10)), at_max);
    let DispatchResult::Completed(failed) = harness.scheduler.dispatch_one().unwrap() else {
        panic!("retry should execute at the saturated deadline")
    };
    assert_eq!(failed.state(), &JobState::Failed(JobFailureCode::Permanent));
    assert_eq!(failed.attempt_count(), 2);
    assert_eq!(provider.call_count(), 2);
    let events_after_failure = harness.events.snapshot();
    let retry_scheduled_count = events_after_failure
        .iter()
        .filter(|event| {
            event.job_id() == Some(queued.id()) && event.code() == JobEventCode::RetryScheduled
        })
        .count();
    assert_eq!(retry_scheduled_count, 1);

    for _ in 0..3 {
        assert_eq!(
            harness.scheduler.dispatch_one().unwrap(),
            DispatchResult::Idle
        );
        assert_eq!(harness.clock.advance(DurationMillis(1)), at_max);
        assert_eq!(provider.call_count(), 2);
        assert_eq!(harness.events.snapshot(), events_after_failure);
    }
    assert_eq!(harness.store.get(queued.id()).unwrap(), Some(*failed));
}

#[test]
fn saturated_retry_can_requeue_at_max_before_later_job_until_attempt_limit() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let retry_handler_ref = handler_ref("max-time-retry-before-later-job");
    let later_handler_ref = handler_ref("max-time-later-job");
    let retry_provider = register(
        &harness,
        TestHandler::scripted(
            retry_handler_ref.clone(),
            [
                HandlerOutcome::Failed(JobFailureCode::Transient),
                HandlerOutcome::Failed(JobFailureCode::Transient),
                HandlerOutcome::Failed(JobFailureCode::Transient),
            ],
        ),
    );
    let later_provider = register(
        &harness,
        TestHandler::scripted(later_handler_ref.clone(), [HandlerOutcome::Succeeded]),
    );
    let retry = RetryPolicy::new(
        3,
        DurationMillis(10),
        DurationMillis(10),
        1_000,
        [JobFailureCode::Transient],
        RetryTerminalAction::Failed,
    )
    .unwrap();
    let before_max = TimestampMillis(u64::MAX - 5);
    let at_max = TimestampMillis(u64::MAX);
    harness.clock.set(before_max).unwrap();
    let retry_job = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(retry_handler_ref)
                .with_priority(JobPriority::Normal)
                .with_retry_policy(retry),
        )
        .unwrap()
        .record;

    let DispatchResult::Completed(first_retry) = harness.scheduler.dispatch_one().unwrap() else {
        panic!("initial retry attempt should complete")
    };
    assert_eq!(first_retry.id(), retry_job.id());
    assert_eq!(first_retry.state(), &JobState::RetryWait);
    assert_eq!(first_retry.attempt_count(), 1);
    assert_eq!(first_retry.retry_at(), Some(at_max));
    assert_eq!(retry_provider.call_count(), 1);

    let later_job = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(later_handler_ref).with_priority(JobPriority::Normal),
        )
        .unwrap()
        .record;
    // Equal creation time and priority leave the older retry's queue sequence first.
    assert_eq!(retry_job.created_at(), later_job.created_at());
    assert_eq!(later_job.state(), &JobState::Queued);
    assert_eq!(later_provider.call_count(), 0);
    assert_eq!(harness.clock.advance(DurationMillis(10)), at_max);

    let DispatchResult::Completed(second_retry) = harness.scheduler.dispatch_one().unwrap() else {
        panic!("first saturated retry should complete")
    };
    assert_eq!(second_retry.id(), retry_job.id());
    assert_eq!(second_retry.state(), &JobState::RetryWait);
    assert_eq!(second_retry.attempt_count(), 2);
    assert_eq!(second_retry.retry_at(), Some(at_max));
    assert_eq!(retry_provider.call_count(), 2);
    assert_eq!(later_provider.call_count(), 0);

    let DispatchResult::Completed(exhausted_retry) = harness.scheduler.dispatch_one().unwrap()
    else {
        panic!("bounded retry should exhaust on the third attempt")
    };
    assert_eq!(exhausted_retry.id(), retry_job.id());
    assert_eq!(
        exhausted_retry.state(),
        &JobState::Failed(JobFailureCode::Permanent)
    );
    assert_eq!(exhausted_retry.attempt_count(), 3);
    assert_eq!(retry_provider.call_count(), 3);
    assert_eq!(later_provider.call_count(), 0);

    let DispatchResult::Completed(later_completed) = harness.scheduler.dispatch_one().unwrap()
    else {
        panic!("later queued job should run after the retry attempt bound")
    };
    assert_eq!(later_completed.id(), later_job.id());
    assert_eq!(later_completed.state(), &JobState::Succeeded);
    assert_eq!(later_provider.call_count(), 1);
}

#[test]
fn retry_attempt_counter_stops_at_max_policy_bound() {
    assert_eq!(
        RetryPolicy::new(
            17,
            DurationMillis(1),
            DurationMillis(1),
            1_000,
            [JobFailureCode::Transient],
            RetryTerminalAction::Failed,
        )
        .unwrap_err(),
        ConstraintError::InvalidAttemptBound
    );
    let retry = RetryPolicy::new(
        16,
        DurationMillis(1),
        DurationMillis(1),
        1_000,
        [JobFailureCode::Transient],
        RetryTerminalAction::Failed,
    )
    .unwrap();
    assert_eq!(retry.max_attempts(), 16);

    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler_ref = handler_ref("max-attempt-counter");
    let provider = register(
        &harness,
        TestHandler::scripted(
            handler_ref.clone(),
            (0..16).map(|_| HandlerOutcome::Failed(JobFailureCode::Transient)),
        ),
    );
    let before_max = TimestampMillis(u64::MAX - 1);
    let at_max = TimestampMillis(u64::MAX);
    harness.clock.set(before_max).unwrap();
    let queued = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler_ref).with_retry_policy(retry),
        )
        .unwrap()
        .record;

    for attempt in 1u8..=16 {
        let DispatchResult::Completed(record) = harness.scheduler.dispatch_one().unwrap() else {
            panic!("bounded retry attempt should complete")
        };
        assert_eq!(record.id(), queued.id());
        assert_eq!(record.attempt_count(), attempt);
        if attempt == 16 {
            assert_eq!(record.state(), &JobState::Failed(JobFailureCode::Permanent));
            assert_eq!(record.retry_at(), None);
        } else {
            assert_eq!(record.state(), &JobState::RetryWait);
            assert_eq!(record.retry_at(), Some(at_max));
        }
        assert_eq!(harness.clock.advance(DurationMillis(1)), at_max);
    }

    assert_eq!(provider.call_count(), 16);
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    let settled = harness.store.get(queued.id()).unwrap().unwrap();
    assert_eq!(settled.attempt_count(), 16);
    assert_eq!(
        settled.state(),
        &JobState::Failed(JobFailureCode::Permanent)
    );
    assert_eq!(provider.call_count(), 16);
}

#[test]
fn unknown_handler_outcome_requires_recovery_and_never_retries_after_restart() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler_ref = handler_ref("outcome-unknown");
    let handler = register(
        &harness,
        TestHandler::scripted(
            handler_ref.clone(),
            [HandlerOutcome::OutcomeUnknown(JobFailureCode::Transient)],
        )
        .with_recovery(RecoveryPolicy::Idempotent),
    );
    let retry = RetryPolicy::new(
        3,
        DurationMillis(10),
        DurationMillis(40),
        2_000,
        [JobFailureCode::Transient],
        RetryTerminalAction::Failed,
    )
    .unwrap();
    let queued = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler_ref).with_retry_policy(retry),
        )
        .unwrap();

    let DispatchResult::Completed(completed) = harness.scheduler.dispatch_one().unwrap() else {
        panic!("unknown handler outcome should be durably applied");
    };
    assert_eq!(
        completed.state(),
        &JobState::RecoveryRequired(JobFailureCode::Transient)
    );
    assert_eq!(completed.attempt_count(), 1);
    assert_eq!(handler.call_count(), 1);
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );

    let restarted = Scheduler::new(
        harness.store.clone(),
        harness.clock.clone(),
        Arc::new(TestAuthority::default()),
        Arc::new(BoundedJobEventRecorder::new(32)),
        harness.providers.clone(),
        SchedulerConfig::default(),
    )
    .unwrap();
    assert!(restarted.recover_startup().unwrap().is_empty());
    assert_eq!(restarted.dispatch_one().unwrap(), DispatchResult::Idle);
    assert_eq!(
        harness
            .store
            .get(queued.record.id())
            .unwrap()
            .unwrap()
            .state(),
        &JobState::RecoveryRequired(JobFailureCode::Transient)
    );
    assert_eq!(handler.call_count(), 1);
}

#[test]
fn cancellation_does_not_hide_unknown_handler_outcome() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let reference = handler_ref("cancelled-outcome-unknown");
    let started = Arc::new(Barrier::new(2));
    let handler = register(
        &harness,
        TestHandler {
            reference: reference.clone(),
            mode: HandlerMode::OutcomeUnknownAfterStop(started.clone()),
            recovery: RecoveryPolicy::Idempotent,
            supports_pause: false,
            checkpoint_valid: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        },
    );
    let retry = RetryPolicy::new(
        3,
        DurationMillis(10),
        DurationMillis(40),
        2_000,
        [JobFailureCode::Transient],
        RetryTerminalAction::Failed,
    )
    .unwrap();
    let queued = harness
        .scheduler
        .enqueue(&caller, JobRequest::new(reference).with_retry_policy(retry))
        .unwrap();
    let dispatcher = harness.scheduler.clone();
    let dispatch = thread::spawn(move || dispatcher.dispatch_one());

    started.wait();
    harness
        .scheduler
        .cancel(&caller, queued.record.id())
        .unwrap();
    let DispatchResult::Completed(completed) = dispatch.join().unwrap().unwrap() else {
        panic!("unknown handler outcome should be durably applied");
    };

    assert_eq!(
        completed.state(),
        &JobState::RecoveryRequired(JobFailureCode::Transient)
    );
    assert_eq!(handler.call_count(), 1);
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
}

#[test]
fn deadline_reached_after_effect_keeps_unknown_outcome_in_recovery() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let reference = handler_ref("deadline-outcome-unknown");
    let handler = register(
        &harness,
        TestHandler {
            reference: reference.clone(),
            mode: HandlerMode::OutcomeUnknownAfterDeadline(harness.clock.clone()),
            recovery: RecoveryPolicy::NonIdempotent,
            supports_pause: false,
            checkpoint_valid: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        },
    );
    let retry = RetryPolicy::new(
        3,
        DurationMillis(10),
        DurationMillis(40),
        2_000,
        [JobFailureCode::Transient],
        RetryTerminalAction::Failed,
    )
    .unwrap();
    let queued = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(reference)
                .with_retry_policy(retry)
                .with_constraints(JobConstraints::default().with_deadline(TimestampMillis(1))),
        )
        .unwrap();

    let DispatchResult::Completed(completed) = harness.scheduler.dispatch_one().unwrap() else {
        panic!("unknown handler outcome should be durably applied");
    };
    assert_eq!(
        completed.state(),
        &JobState::RecoveryRequired(JobFailureCode::Transient)
    );
    assert_eq!(handler.call_count(), 1);
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    assert_eq!(
        harness
            .store
            .get(queued.record.id())
            .unwrap()
            .unwrap()
            .state(),
        &JobState::RecoveryRequired(JobFailureCode::Transient)
    );
}

#[test]
fn dependency_order_failure_policy_and_cycle_detection_are_explicit() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let success_handler = handler_ref("success");
    let failed_handler = handler_ref("failed");
    let dependent_handler = handler_ref("dependent");
    register(
        &harness,
        TestHandler::scripted(success_handler.clone(), [HandlerOutcome::Succeeded]),
    );
    register(
        &harness,
        TestHandler::scripted(
            failed_handler.clone(),
            [HandlerOutcome::Failed(JobFailureCode::Permanent)],
        ),
    );
    let dependent = register(
        &harness,
        TestHandler::scripted(dependent_handler.clone(), [HandlerOutcome::Succeeded]),
    );

    let prerequisite = enqueue(&harness, &caller, &success_handler);
    let child = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(dependent_handler.clone()).with_dependencies(
                vec![DependencyRef::success(prerequisite.record.id().clone())],
                DependencyFailurePolicy::Block,
            ),
        )
        .unwrap();
    assert_eq!(child.record.state(), &JobState::WaitingOnDependency);
    assert!(matches!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Completed(_)
    ));
    assert!(matches!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Completed(_)
    ));
    assert_eq!(dependent.call_count(), 1);
    assert_eq!(
        record(&harness, &caller, child.record.id()).state(),
        &JobState::Succeeded
    );

    let failed = enqueue(&harness, &caller, &failed_handler);
    let blocked = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(dependent_handler.clone()).with_dependencies(
                vec![DependencyRef::success(failed.record.id().clone())],
                DependencyFailurePolicy::Block,
            ),
        )
        .unwrap();
    let cancelled = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(dependent_handler.clone()).with_dependencies(
                vec![DependencyRef::success(failed.record.id().clone())],
                DependencyFailurePolicy::CancelDependent,
            ),
        )
        .unwrap();
    let allowed_failure = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(dependent_handler.clone()).with_dependencies(
                vec![DependencyRef::success(failed.record.id().clone())],
                DependencyFailurePolicy::AllowFailure,
            ),
        )
        .unwrap();
    assert!(matches!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Completed(_)
    ));
    let DispatchResult::Completed(allowed_record) = harness.scheduler.dispatch_one().unwrap()
    else {
        panic!("allow-failure policy should release its dependent")
    };
    assert_eq!(allowed_record.id(), allowed_failure.record.id());
    assert_eq!(
        record(&harness, &caller, blocked.record.id()).state(),
        &JobState::WaitingOnDependency
    );
    assert_eq!(
        record(&harness, &caller, cancelled.record.id()).state(),
        &JobState::Cancelled
    );
    assert_eq!(dependent.call_count(), 2);
    assert_eq!(allowed_record.state(), &JobState::Succeeded);

    let unknown = JobId::new("job-unknown").unwrap();
    let error = harness.scheduler.enqueue(
        &caller,
        JobRequest::new(dependent_handler.clone()).with_dependencies(
            vec![DependencyRef::success(unknown)],
            DependencyFailurePolicy::Block,
        ),
    );
    assert_eq!(error.unwrap_err(), SchedulerError::UnknownDependency);

    // Normal enqueue cannot forward-reference a new job. The recovery scan
    // still validates a store restored from a corrupt cyclic snapshot.
    let first = enqueue(&harness, &caller, &dependent_handler);
    let second = enqueue(&harness, &caller, &dependent_handler);
    harness.store.seed_dependency_for_test(
        first.record.id(),
        DependencyRef::success(second.record.id().clone()),
    );
    harness.store.seed_dependency_for_test(
        second.record.id(),
        DependencyRef::success(first.record.id().clone()),
    );
    assert_eq!(
        harness.scheduler.recover_startup().unwrap_err(),
        SchedulerError::DependencyCycle
    );
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap_err(),
        SchedulerError::DependencyCycle
    );
}

#[test]
fn idempotency_deduplication_is_atomic_and_scoped() {
    let harness = harness(SchedulerConfig::default());
    let alice = caller("user.alice");
    let handler = handler_ref("dedupe");
    register(&harness, TestHandler::scripted(handler.clone(), []));
    let request =
        JobRequest::new(handler).with_idempotency_key(IdempotencyKey::new("same-request").unwrap());
    let mut workers = Vec::new();
    for _ in 0..16 {
        let scheduler = harness.scheduler.clone();
        let caller = alice.clone();
        let request = request.clone();
        workers.push(thread::spawn(move || {
            scheduler.enqueue(&caller, request).unwrap()
        }));
    }
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    let distinct = results
        .iter()
        .map(|result| result.record.id().clone())
        .collect::<HashSet<_>>();
    assert_eq!(distinct.len(), 1);
    assert_eq!(
        results.iter().filter(|result| !result.deduplicated).count(),
        1
    );
    assert_eq!(
        results.iter().filter(|result| result.deduplicated).count(),
        15
    );

    let other_owner = caller("user.bob");
    let other = harness
        .scheduler
        .enqueue(
            &other_owner,
            JobRequest::new(handler_ref("dedupe"))
                .with_idempotency_key(IdempotencyKey::new("same-request").unwrap()),
        )
        .unwrap();
    assert_ne!(other.record.id(), results[0].record.id());
}

#[test]
fn reconstructed_scheduler_allocates_a_new_job_id_from_existing_records() {
    let harness = harness(SchedulerConfig::default());
    let alice = caller("user.alice");
    let first = enqueue(&harness, &alice, &handler_ref("before-restart"));

    let restarted = Scheduler::new(
        harness.store.clone(),
        Arc::new(ManualClock::new(TimestampMillis(1))),
        Arc::new(TestAuthority::default()),
        Arc::new(BoundedJobEventRecorder::new(32)),
        Arc::new(JobProviderRegistry::new()),
        SchedulerConfig::default(),
    )
    .unwrap();
    let second = restarted
        .enqueue(&alice, JobRequest::new(handler_ref("after-restart")))
        .expect("a fresh request after restart must not collide with a stored JobId");

    assert_ne!(first.record.id(), second.record.id());
}

#[test]
fn reconstructed_scheduler_finds_free_id_at_scan_bound() {
    let harness = harness(SchedulerConfig::default());
    let alice = caller("user.alice");
    let first = enqueue(&harness, &alice, &handler_ref("scan-bound-one"));
    let second = enqueue(&harness, &alice, &handler_ref("scan-bound-two"));
    let third = enqueue(&harness, &alice, &handler_ref("scan-bound-three"));

    let restarted = Scheduler::new(
        harness.store.clone(),
        Arc::new(ManualClock::new(TimestampMillis(1))),
        Arc::new(TestAuthority::default()),
        Arc::new(BoundedJobEventRecorder::new(32)),
        Arc::new(JobProviderRegistry::new()),
        SchedulerConfig::default(),
    )
    .unwrap();
    let fourth = restarted
        .enqueue(&alice, JobRequest::new(handler_ref("scan-bound-four")))
        .unwrap();

    assert_eq!(first.record.id().as_str(), "job-0000000000000001");
    assert_eq!(second.record.id().as_str(), "job-0000000000000002");
    assert_eq!(third.record.id().as_str(), "job-0000000000000003");
    assert_eq!(fourth.record.id().as_str(), "job-0000000000000004");
    assert_eq!(harness.store.list().unwrap().len(), 4);
}

#[test]
fn reconstructed_scheduler_skips_existing_ids_across_sequence_wrap() {
    let harness = harness(SchedulerConfig::default());
    let alice = caller("user.alice");
    harness.scheduler.set_id_sequence_for_test(u64::MAX - 1);

    let before_wrap = enqueue(&harness, &alice, &handler_ref("before-wrap-one"));
    let at_max = enqueue(&harness, &alice, &handler_ref("before-wrap-two"));
    let at_zero = enqueue(&harness, &alice, &handler_ref("before-wrap-three"));
    let after_zero = enqueue(&harness, &alice, &handler_ref("before-wrap-four"));
    assert_eq!(before_wrap.record.id().as_str(), "job-fffffffffffffffe");
    assert_eq!(at_max.record.id().as_str(), "job-ffffffffffffffff");
    assert_eq!(at_zero.record.id().as_str(), "job-0000000000000000");
    assert_eq!(after_zero.record.id().as_str(), "job-0000000000000001");

    let records_before_restart = harness.store.list().unwrap();
    let restarted = Scheduler::new(
        harness.store.clone(),
        Arc::new(ManualClock::new(TimestampMillis(1))),
        Arc::new(TestAuthority::default()),
        Arc::new(BoundedJobEventRecorder::new(32)),
        Arc::new(JobProviderRegistry::new()),
        SchedulerConfig::default(),
    )
    .unwrap();
    restarted.set_id_sequence_for_test(u64::MAX);

    let next = restarted
        .enqueue(&alice, JobRequest::new(handler_ref("after-sequence-wrap")))
        .unwrap();
    assert_eq!(next.record.id().as_str(), "job-0000000000000002");
    let records_after_restart = harness.store.list().unwrap();
    assert_eq!(records_after_restart.len(), 5);
    assert!(records_before_restart
        .iter()
        .all(|before| records_after_restart.contains(before)));
}

#[test]
fn enqueue_list_read_failure_preserves_records_events_and_id_sequence() {
    let harness = harness(SchedulerConfig::default());
    let alice = caller("user.alice");
    let first = enqueue(&harness, &alice, &handler_ref("before-list-failure"));
    let records_before = harness.store.list().unwrap();
    let events_before = harness.events.snapshot();

    harness.store.fail_reads_for_test(true);
    let failed = harness
        .scheduler
        .enqueue(&alice, JobRequest::new(handler_ref("during-list-failure")));
    harness.store.fail_reads_for_test(false);

    assert!(matches!(
        failed,
        Err(SchedulerError::Store(JobStoreError::Unavailable))
    ));
    assert_eq!(harness.store.list().unwrap(), records_before);
    assert_eq!(harness.events.snapshot(), events_before);

    let next = enqueue(&harness, &alice, &handler_ref("after-list-failure"));
    assert_eq!(first.record.id().as_str(), "job-0000000000000001");
    assert_eq!(next.record.id().as_str(), "job-0000000000000002");
}

#[test]
fn deduplication_keeps_active_jobs_and_expires_completed_keys() {
    let config = SchedulerConfig::default().with_dedupe_window(DurationMillis(10));
    let harness = harness(config);
    let alice = caller("user.alice");
    let handler = handler_ref("dedupe-retention");
    register(
        &harness,
        TestHandler::scripted(handler.clone(), [HandlerOutcome::Succeeded]),
    );
    let request = JobRequest::new(handler)
        .with_idempotency_key(IdempotencyKey::new("opaque-request-id").unwrap());
    let first = harness.scheduler.enqueue(&alice, request.clone()).unwrap();
    harness.clock.advance(DurationMillis(20));
    let active_duplicate = harness.scheduler.enqueue(&alice, request.clone()).unwrap();
    assert!(active_duplicate.deduplicated);
    assert_eq!(active_duplicate.record.id(), first.record.id());
    assert!(matches!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Completed(_)
    ));
    let after_retention = harness.scheduler.enqueue(&alice, request).unwrap();
    assert!(!after_retention.deduplicated);
    assert_ne!(after_retention.record.id(), first.record.id());
}

#[test]
fn global_and_per_owner_concurrency_limits_are_atomic() {
    let harness = harness(SchedulerConfig::new(2, 1, 8).unwrap());
    let alice = caller("user.alice");
    let bob = caller("user.bob");
    let handler = handler_ref("concurrent");
    let started = Arc::new(Barrier::new(2));
    let provider = TestHandler {
        reference: handler.clone(),
        mode: HandlerMode::WaitFirst(started.clone()),
        recovery: RecoveryPolicy::Idempotent,
        supports_pause: false,
        checkpoint_valid: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    };
    let provider = register(&harness, provider);
    let alice_first = enqueue(&harness, &alice, &handler);
    let alice_second = enqueue(&harness, &alice, &handler);
    let bob_job = enqueue(&harness, &bob, &handler);

    let scheduler = harness.scheduler.clone();
    let worker = thread::spawn(move || scheduler.dispatch_one().unwrap());
    started.wait();
    let second_result = harness.scheduler.dispatch_one().unwrap();
    let DispatchResult::Completed(bob_record) = second_result else {
        panic!("another owner should use the second global slot")
    };
    assert_eq!(bob_record.id(), bob_job.record.id());
    assert_eq!(provider.call_count(), 2);

    // Alice already occupies her per-owner slot, so her second job cannot
    // claim it even though a global slot has become free.
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    harness
        .scheduler
        .cancel(&alice, alice_first.record.id())
        .unwrap();
    let first_record = worker.join().unwrap();
    let DispatchResult::Completed(first_record) = first_record else {
        panic!("blocked provider should finish after cancellation")
    };
    assert_eq!(first_record.state(), &JobState::Cancelled);
    let second_record = match harness.scheduler.dispatch_one().unwrap() {
        DispatchResult::Completed(record) => record,
        DispatchResult::Superseded { .. } => panic!("queued work cannot be superseded"),
        DispatchResult::Idle => panic!("owner slot should be free"),
    };
    assert_eq!(second_record.id(), alice_second.record.id());
    assert_eq!(provider.call_count(), 3);
}

#[test]
fn independent_workers_cannot_claim_the_same_job() {
    let harness = harness(SchedulerConfig::new(1, 1, 8).unwrap());
    let caller = caller("user.alice");
    let handler = handler_ref("single-claim");
    let provider = register(
        &harness,
        TestHandler::scripted(handler.clone(), [HandlerOutcome::Succeeded]),
    );
    enqueue(&harness, &caller, &handler);

    let second_scheduler = Scheduler::new(
        harness.store.clone(),
        harness.clock.clone(),
        harness.authority.clone(),
        harness.events.clone(),
        harness.providers.clone(),
        SchedulerConfig::new(1, 1, 8).unwrap(),
    )
    .unwrap();
    let first = harness.scheduler.clone();
    let left = thread::spawn(move || first.dispatch_one().unwrap());
    let right = thread::spawn(move || second_scheduler.dispatch_one().unwrap());
    let outcomes = [left.join().unwrap(), right.join().unwrap()];
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, DispatchResult::Completed(_)))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| **outcome == DispatchResult::Idle)
            .count(),
        1
    );
    assert_eq!(provider.call_count(), 1);
}

#[test]
fn queue_capacity_and_priority_aging_are_bounded_and_fair() {
    let queue_harness = harness(
        SchedulerConfig::new(1, 1, 1)
            .unwrap()
            .with_record_capacity(8)
            .unwrap()
            .with_priority_aging_interval(DurationMillis(1_000)),
    );
    let caller = caller("user.alice");
    let handler = handler_ref("fair");
    register(&queue_harness, TestHandler::scripted(handler.clone(), []));
    let queued = queue_harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler.clone()).with_priority(JobPriority::Low),
        )
        .unwrap();
    assert_eq!(
        queue_harness
            .scheduler
            .enqueue(&caller, JobRequest::new(handler.clone()))
            .unwrap_err(),
        SchedulerError::QueueFull
    );
    assert!(queue_harness
        .events
        .snapshot()
        .iter()
        .any(|event| event.code() == JobEventCode::QueueSaturated));

    queue_harness
        .scheduler
        .cancel(&caller, queued.record.id())
        .unwrap();
    let high = queue_harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler.clone()).with_priority(JobPriority::High),
        )
        .unwrap();
    assert_eq!(high.record.state(), &JobState::Queued);
    assert_eq!(
        queue_harness.scheduler.list_visible(&caller).unwrap().len(),
        2
    );

    let record_limited = harness(
        SchedulerConfig::new(1, 1, 1)
            .unwrap()
            .with_record_capacity(1)
            .unwrap(),
    );
    let record_owner =
        CallerRef::from_authenticated_adapter(PrincipalRef::new("user.alice").unwrap(), None);
    let handler = handler_ref("record-capacity");
    register(&record_limited, TestHandler::scripted(handler.clone(), []));
    enqueue(&record_limited, &record_owner, &handler);
    record_limited.scheduler.dispatch_one().unwrap();
    assert_eq!(
        record_limited
            .scheduler
            .enqueue(&record_owner, JobRequest::new(handler))
            .unwrap_err(),
        SchedulerError::QueueFull
    );
}

#[test]
fn starvation_aging_promotes_older_low_priority_work() {
    let harness = harness(
        SchedulerConfig::new(1, 1, 8)
            .unwrap()
            .with_priority_aging_interval(DurationMillis(1_000)),
    );
    let caller = caller("user.alice");
    let handler = handler_ref("aging");
    register(&harness, TestHandler::scripted(handler.clone(), []));
    let low = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler.clone()).with_priority(JobPriority::Low),
        )
        .unwrap();
    harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler.clone()).with_priority(JobPriority::High),
        )
        .unwrap();
    harness.clock.advance(DurationMillis(3_000));
    let DispatchResult::Completed(first) = harness.scheduler.dispatch_one().unwrap() else {
        panic!("eligible job should start")
    };
    assert_eq!(first.id(), low.record.id());
}

#[test]
fn urgent_priority_requires_an_explicit_authorization_decision() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler = handler_ref("urgent");
    register(&harness, TestHandler::scripted(handler.clone(), []));
    assert!(matches!(
        harness.scheduler.enqueue(
            &caller,
            JobRequest::new(handler.clone()).with_priority(JobPriority::Urgent)
        ),
        Err(SchedulerError::Authorization(_))
    ));
    harness.authority.allow_urgent.store(true, Ordering::SeqCst);
    assert_eq!(
        harness
            .scheduler
            .enqueue(
                &caller,
                JobRequest::new(handler).with_priority(JobPriority::Urgent)
            )
            .unwrap()
            .record
            .priority(),
        JobPriority::Urgent
    );
}

#[test]
fn permission_changes_before_retry_block_provider_execution() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler = handler_ref("permission-recheck");
    let provider = register(
        &harness,
        TestHandler::scripted(
            handler.clone(),
            [
                HandlerOutcome::Failed(JobFailureCode::Transient),
                HandlerOutcome::Succeeded,
            ],
        ),
    );
    let policy = RetryPolicy::new(
        3,
        DurationMillis(10),
        DurationMillis(20),
        2_000,
        [JobFailureCode::Transient],
        RetryTerminalAction::Failed,
    )
    .unwrap();
    let queued = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler.clone()).with_retry_policy(policy),
        )
        .unwrap();
    harness.scheduler.dispatch_one().unwrap();
    harness.authority.deny_handler.store(true, Ordering::SeqCst);
    harness.clock.advance(DurationMillis(10));
    let result = harness.scheduler.dispatch_one().unwrap();
    let DispatchResult::Completed(record) = result else {
        panic!("retry authorization must be checked before provider execution")
    };
    assert_eq!(record.id(), queued.record.id());
    assert_eq!(
        record.state(),
        &JobState::Failed(JobFailureCode::PermissionDenied)
    );
    assert_eq!(provider.call_count(), 1);
    assert!(harness.events.snapshot().iter().any(|event| {
        event.code() == JobEventCode::PermissionDenied && event.job_id() == Some(queued.record.id())
    }));
}

#[test]
fn owner_inspect_cancel_and_list_authorization_are_separate() {
    let harness = harness(SchedulerConfig::default());
    let alice = caller("user.alice");
    let bob = caller("user.bob");
    let handler = handler_ref("owner-check");
    register(&harness, TestHandler::scripted(handler.clone(), []));
    let queued = enqueue(&harness, &alice, &handler);
    assert!(matches!(
        harness.scheduler.inspect(&bob, queued.record.id()),
        Err(SchedulerError::Authorization(_))
    ));
    assert!(harness.scheduler.list_visible(&bob).unwrap().is_empty());
    assert!(matches!(
        harness.scheduler.cancel(&bob, queued.record.id()),
        Err(SchedulerError::Authorization(_))
    ));
    harness
        .authority
        .deny_action("user.alice", JobAction::Cancel);
    assert!(matches!(
        harness.scheduler.cancel(&alice, queued.record.id()),
        Err(SchedulerError::Authorization(_))
    ));
    harness
        .authority
        .allow_action("user.alice", JobAction::Cancel);
    harness.authority.deny_action("user.alice", JobAction::List);
    assert!(harness.scheduler.list_visible(&alice).unwrap().is_empty());
    assert_eq!(
        record(&harness, &alice, queued.record.id()).state(),
        &JobState::Queued
    );
    harness
        .authority
        .allow_action("user.alice", JobAction::List);
    harness
        .authority
        .deny_action("user.alice", JobAction::Enqueue);
    let denied_request = JobRequest::new(handler.clone())
        .with_idempotency_key(IdempotencyKey::new("denied-then-retried").unwrap());
    let events_before_denial = harness.events.snapshot();
    assert!(matches!(
        harness.scheduler.enqueue(&alice, denied_request.clone()),
        Err(SchedulerError::Authorization(_))
    ));
    assert_eq!(harness.scheduler.list_visible(&alice).unwrap().len(), 1);
    assert_eq!(harness.events.snapshot(), events_before_denial);

    harness
        .authority
        .allow_action("user.alice", JobAction::Enqueue);
    let accepted_after_denial = harness
        .scheduler
        .enqueue(&alice, denied_request.clone())
        .unwrap();
    assert!(!accepted_after_denial.deduplicated);
    assert_eq!(harness.scheduler.list_visible(&alice).unwrap().len(), 2);

    let exact_retry = harness.scheduler.enqueue(&alice, denied_request).unwrap();
    assert!(exact_retry.deduplicated);
    assert_eq!(exact_retry.record.id(), accepted_after_denial.record.id());
    assert_eq!(harness.scheduler.list_visible(&alice).unwrap().len(), 2);
}

#[test]
fn unavailable_priority_authorization_does_not_insert_job_or_emit_acceptance() {
    let harness = harness(SchedulerConfig::default());
    let alice = caller("user.alice");
    let handler = handler_ref("unavailable-priority-authorization");
    register(&harness, TestHandler::scripted(handler.clone(), []));
    harness
        .authority
        .unavailable_priority
        .store(true, Ordering::SeqCst);
    let events_before = harness.events.snapshot();

    let result = harness.scheduler.enqueue(&alice, JobRequest::new(handler));

    assert!(matches!(
        result,
        Err(SchedulerError::Authorization(error))
            if error.code() == AuthorizationErrorCode::Unavailable
    ));
    assert!(harness.store.list().unwrap().is_empty());
    assert_eq!(harness.events.snapshot(), events_before);
}

#[test]
fn cancellation_propagates_to_child_work_and_timeout_uses_virtual_time() {
    let cancel_harness = harness(SchedulerConfig::default());
    let alice = caller("user.alice");
    let handler = handler_ref("stop-aware");
    let started = Arc::new(Barrier::new(2));
    register(
        &cancel_harness,
        TestHandler {
            reference: handler.clone(),
            mode: HandlerMode::WaitFirst(started.clone()),
            recovery: RecoveryPolicy::NonIdempotent,
            supports_pause: false,
            checkpoint_valid: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        },
    );
    let queued = enqueue(&cancel_harness, &alice, &handler);
    let scheduler = cancel_harness.scheduler.clone();
    let worker = thread::spawn(move || scheduler.dispatch_one().unwrap());
    started.wait();
    cancel_harness
        .scheduler
        .cancel(&alice, queued.record.id())
        .unwrap();
    let DispatchResult::Completed(cancelled) = worker.join().unwrap() else {
        panic!("cooperative handler should finish cancellation")
    };
    assert_eq!(cancelled.state(), &JobState::Cancelled);

    let timeout_harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler = handler_ref("timeout-aware");
    let started = Arc::new(Barrier::new(2));
    register(
        &timeout_harness,
        TestHandler {
            reference: handler.clone(),
            mode: HandlerMode::WaitFirst(started.clone()),
            recovery: RecoveryPolicy::NonIdempotent,
            supports_pause: false,
            checkpoint_valid: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        },
    );
    let queued = timeout_harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler.clone())
                .with_constraints(JobConstraints::default().with_timeout(DurationMillis(50))),
        )
        .unwrap();
    let scheduler = timeout_harness.scheduler.clone();
    let worker = thread::spawn(move || scheduler.dispatch_one().unwrap());
    started.wait();
    timeout_harness.clock.advance(DurationMillis(50));
    assert_eq!(
        timeout_harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    let DispatchResult::Completed(timed_out) = worker.join().unwrap() else {
        panic!("cooperative timeout handler should finish")
    };
    assert_eq!(timed_out.id(), queued.record.id());
    assert_eq!(timed_out.state(), &JobState::TimedOut);
}

#[test]
fn restart_replays_only_idempotent_or_verified_checkpoint_jobs() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let idempotent_ref = handler_ref("idempotent-recovery");
    let idempotent = register(
        &harness,
        TestHandler::scripted(idempotent_ref.clone(), []).with_recovery(RecoveryPolicy::Idempotent),
    );
    let idempotent_job = enqueue(&harness, &caller, &idempotent_ref);
    harness
        .store
        .seed_state_for_test(idempotent_job.record.id(), JobState::Running);
    let recovered = harness.scheduler.recover_startup().unwrap();
    let resumed = recovered
        .iter()
        .find(|record| record.id() == idempotent_job.record.id())
        .unwrap();
    assert_eq!(resumed.state(), &JobState::Queued);
    assert_eq!(resumed.attempt_count(), 1);
    assert!(matches!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Completed(_)
    ));
    assert_eq!(idempotent.call_count(), 1);
    assert_eq!(
        record(&harness, &caller, idempotent_job.record.id()).attempt_count(),
        2
    );

    let unsafe_ref = handler_ref("non-idempotent-recovery");
    let unsafe_handler = register(
        &harness,
        TestHandler::scripted(unsafe_ref.clone(), []).with_recovery(RecoveryPolicy::NonIdempotent),
    );
    let unsafe_job = enqueue(&harness, &caller, &unsafe_ref);
    harness
        .store
        .seed_state_for_test(unsafe_job.record.id(), JobState::Running);
    let recovered = harness.scheduler.recover_startup().unwrap();
    let manual_repair = recovered
        .iter()
        .find(|record| record.id() == unsafe_job.record.id())
        .unwrap();
    assert_eq!(
        manual_repair.state(),
        &JobState::RecoveryRequired(JobFailureCode::Permanent)
    );
    assert_eq!(unsafe_handler.call_count(), 0);

    let checkpoint_ref = handler_ref("checkpoint-recovery");
    let checkpoint_handler = register(
        &harness,
        TestHandler::scripted(checkpoint_ref.clone(), [])
            .with_recovery(RecoveryPolicy::VerifiedCheckpoint)
            .with_checkpoint_valid(true),
    );
    let checkpoint_job = enqueue(&harness, &caller, &checkpoint_ref);
    harness
        .store
        .seed_state_for_test(checkpoint_job.record.id(), JobState::Running);
    harness.store.seed_checkpoint_for_test(
        checkpoint_job.record.id(),
        CheckpointRef::new("checkpoint-ok").unwrap(),
    );
    let recovered = harness.scheduler.recover_startup().unwrap();
    assert_eq!(
        recovered
            .iter()
            .find(|record| record.id() == checkpoint_job.record.id())
            .unwrap()
            .state(),
        &JobState::Queued
    );
    assert_eq!(checkpoint_handler.call_count(), 0);
}

#[test]
fn restart_rechecks_permissions_and_rejects_unverified_checkpoints() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let permission_ref = handler_ref("permission-recovery");
    register(
        &harness,
        TestHandler::scripted(permission_ref.clone(), []).with_recovery(RecoveryPolicy::Idempotent),
    );
    let queued = enqueue(&harness, &caller, &permission_ref);
    harness
        .store
        .seed_state_for_test(queued.record.id(), JobState::Running);
    harness.authority.deny_handler.store(true, Ordering::SeqCst);
    let recovered = harness.scheduler.recover_startup().unwrap();
    assert_eq!(
        recovered
            .iter()
            .find(|record| record.id() == queued.record.id())
            .unwrap()
            .state(),
        &JobState::RecoveryRequired(JobFailureCode::PermissionDenied)
    );

    let checkpoint_ref = handler_ref("invalid-checkpoint");
    let provider = register(
        &harness,
        TestHandler::scripted(checkpoint_ref.clone(), [])
            .with_recovery(RecoveryPolicy::VerifiedCheckpoint)
            .with_checkpoint_valid(false),
    );
    let checkpoint_job = enqueue(&harness, &caller, &checkpoint_ref);
    harness
        .store
        .seed_state_for_test(checkpoint_job.record.id(), JobState::Running);
    harness.store.seed_checkpoint_for_test(
        checkpoint_job.record.id(),
        CheckpointRef::new("checkpoint-bad").unwrap(),
    );
    harness
        .authority
        .deny_handler
        .store(false, Ordering::SeqCst);
    let recovered = harness.scheduler.recover_startup().unwrap();
    assert_eq!(
        recovered
            .iter()
            .find(|record| record.id() == checkpoint_job.record.id())
            .unwrap()
            .state(),
        &JobState::RecoveryRequired(JobFailureCode::InvalidCheckpoint)
    );
    assert_eq!(provider.call_count(), 0);
}

#[test]
fn restart_does_not_replay_verified_checkpoint_past_attempt_bound() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler_ref = handler_ref("max-attempt-checkpoint-recovery");
    let handler = register(
        &harness,
        TestHandler::scripted(handler_ref.clone(), [])
            .with_recovery(RecoveryPolicy::VerifiedCheckpoint)
            .with_checkpoint_valid(true),
    );
    let retry = RetryPolicy::new(
        16,
        DurationMillis(1),
        DurationMillis(1),
        1_000,
        [JobFailureCode::Transient],
        RetryTerminalAction::Failed,
    )
    .unwrap();
    let queued = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler_ref).with_retry_policy(retry),
        )
        .unwrap();
    harness
        .store
        .seed_state_for_test(queued.record.id(), JobState::Running);

    // Model the persisted row for attempt 16 when the process stops. The valid
    // checkpoint must not authorize a seventeenth scheduler attempt.
    let running = harness.store.get(queued.record.id()).unwrap().unwrap();
    let mut snapshot: serde_json::Value =
        serde_json::from_slice(&encode_job_record(&running).unwrap()).unwrap();
    snapshot["attempt_count"] = serde_json::json!(16);
    snapshot["checkpoint_ref"] = serde_json::json!("checkpoint-at-limit");
    let restored = decode_job_record(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
    assert_eq!(restored.attempt_count(), 16);
    assert_eq!(
        restored.checkpoint_ref(),
        Some(&CheckpointRef::new("checkpoint-at-limit").unwrap())
    );

    let restored_store = Arc::new(InMemoryJobStore::new());
    assert!(matches!(
        restored_store
            .insert(restored, None, TimestampMillis(2), 8, 128)
            .unwrap(),
        InsertOutcome::Inserted(_)
    ));
    let restarted = Scheduler::new(
        restored_store.clone(),
        Arc::new(ManualClock::new(TimestampMillis(2))),
        Arc::new(TestAuthority::default()),
        Arc::new(BoundedJobEventRecorder::new(32)),
        harness.providers.clone(),
        SchedulerConfig::default(),
    )
    .unwrap();

    let recovered = restarted.recover_startup().unwrap();
    let settled = recovered
        .iter()
        .find(|record| record.id() == queued.record.id())
        .unwrap();
    assert_eq!(
        settled.state(),
        &JobState::RecoveryRequired(JobFailureCode::Permanent)
    );
    assert_eq!(settled.attempt_count(), 16);
    assert_eq!(restarted.dispatch_one().unwrap(), DispatchResult::Idle);
    assert_eq!(handler.call_count(), 0);
}

#[test]
fn recovery_required_jobs_need_an_authorized_explicit_retry() {
    let harness = harness(SchedulerConfig::default());
    let alice = caller("user.alice");
    let handler = handler_ref("manual-repair");
    let provider = register(
        &harness,
        TestHandler::scripted(handler.clone(), [HandlerOutcome::Succeeded]),
    );
    let queued = enqueue(&harness, &alice, &handler);
    harness.store.seed_state_for_test(
        queued.record.id(),
        JobState::RecoveryRequired(JobFailureCode::Permanent),
    );
    harness
        .authority
        .deny_action("user.alice", JobAction::Recover);
    assert!(matches!(
        harness.scheduler.retry_recovery(&alice, queued.record.id()),
        Err(SchedulerError::Authorization(_))
    ));
    harness
        .authority
        .allow_action("user.alice", JobAction::Recover);
    assert_eq!(
        harness
            .scheduler
            .retry_recovery(&alice, queued.record.id())
            .unwrap()
            .state(),
        &JobState::Queued
    );
    assert!(matches!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Completed(_)
    ));
    assert_eq!(provider.call_count(), 1);
}

#[test]
fn authorized_recovery_can_reexecute_unknown_outcome_without_reconciliation_input() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler = handler_ref("unknown-outcome-manual-recovery");
    let provider = register(
        &harness,
        TestHandler::scripted(
            handler.clone(),
            [
                HandlerOutcome::OutcomeUnknown(JobFailureCode::Transient),
                HandlerOutcome::Succeeded,
            ],
        )
        .with_recovery(RecoveryPolicy::NonIdempotent),
    );
    let queued = enqueue(&harness, &caller, &handler);

    assert!(matches!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Completed(_)
    ));
    assert_eq!(
        record(&harness, &caller, queued.record.id()).state(),
        &JobState::RecoveryRequired(JobFailureCode::Transient)
    );
    assert_eq!(provider.call_count(), 1);

    // The current API accepts only caller and job ID; JobAction::Recover is
    // the authorization boundary, with no reconciliation receipt parameter.
    assert_eq!(
        harness
            .scheduler
            .retry_recovery(&caller, queued.record.id())
            .unwrap()
            .state(),
        &JobState::Queued
    );
    assert!(matches!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Completed(_)
    ));
    assert_eq!(provider.call_count(), 2);
    assert_eq!(
        record(&harness, &caller, queued.record.id()).state(),
        &JobState::Succeeded
    );
}

#[test]
fn pause_resume_requires_provider_support_and_has_typed_results() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let unsupported_ref = handler_ref("not-pausable");
    register(
        &harness,
        TestHandler::scripted(unsupported_ref.clone(), []).with_pause(false),
    );
    let unsupported = enqueue(&harness, &caller, &unsupported_ref);
    assert_eq!(
        harness
            .scheduler
            .pause(&caller, unsupported.record.id())
            .unwrap_err(),
        SchedulerError::UnsupportedPause
    );
    assert_eq!(
        record(&harness, &caller, unsupported.record.id()).state(),
        &JobState::Queued
    );
    harness
        .scheduler
        .cancel(&caller, unsupported.record.id())
        .unwrap();

    let supported_ref = handler_ref("pausable");
    let started = Arc::new(Barrier::new(2));
    register(
        &harness,
        TestHandler {
            reference: supported_ref.clone(),
            mode: HandlerMode::PauseFirst(started.clone()),
            recovery: RecoveryPolicy::VerifiedCheckpoint,
            supports_pause: true,
            checkpoint_valid: AtomicBool::new(true),
            calls: AtomicUsize::new(0),
        },
    );
    let job = enqueue(&harness, &caller, &supported_ref);
    let scheduler = harness.scheduler.clone();
    let worker = thread::spawn(move || scheduler.dispatch_one().unwrap());
    started.wait();
    let requested = harness.scheduler.pause(&caller, job.record.id()).unwrap();
    assert_eq!(requested.state(), &JobState::PauseRequested);
    let DispatchResult::Completed(paused) = worker.join().unwrap() else {
        panic!("pause-cooperative provider should return")
    };
    assert_eq!(paused.state(), &JobState::Paused);
    assert_eq!(paused.checkpoint_ref().unwrap().as_str(), "checkpoint-1");
    let resumed = harness.scheduler.resume(&caller, job.record.id()).unwrap();
    assert_eq!(resumed.state(), &JobState::Queued);
}

#[test]
fn missing_handler_diagnostic_can_reenter_dispatch_after_dispatch_lock_release() {
    let (harness, sink) =
        reentrant_harness(JobEventCode::HandlerMissing, ReentrantCall::DispatchIdle);
    let caller = caller("user.alice");
    let missing = handler_ref("missing-handler-reentry");
    let queued = enqueue(&harness, &caller, &missing);

    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    let reentry_completed = sink.finish_reentry();
    sink.detach_scheduler();

    assert!(
        reentry_completed,
        "diagnostic callback could not re-enter dispatch"
    );
    let failed = harness.store.get(queued.record.id()).unwrap().unwrap();
    assert_eq!(
        failed.state(),
        &JobState::Failed(JobFailureCode::MissingHandler)
    );
    let job_events = harness
        .events
        .snapshot()
        .into_iter()
        .filter(|event| event.job_id() == Some(queued.record.id()))
        .map(|event| event.code())
        .collect::<Vec<_>>();
    assert_eq!(
        job_events,
        vec![JobEventCode::EnqueueAccepted, JobEventCode::HandlerMissing]
    );
}

#[test]
fn timeout_diagnostic_can_reenter_dispatch_after_dispatch_lock_release() {
    let (harness, sink) = reentrant_harness(JobEventCode::TimedOut, ReentrantCall::DispatchIdle);
    let caller = caller("user.alice");
    let handler = handler_ref("timeout-reentry");
    let provider = register(&harness, TestHandler::scripted(handler.clone(), []));
    let queued = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler)
                .with_constraints(JobConstraints::default().with_deadline(TimestampMillis(1))),
        )
        .unwrap();
    harness.clock.set(TimestampMillis(1)).unwrap();

    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    let reentry_completed = sink.finish_reentry();
    sink.detach_scheduler();

    assert!(
        reentry_completed,
        "diagnostic callback could not re-enter dispatch"
    );
    let timed_out = harness.store.get(queued.record.id()).unwrap().unwrap();
    assert_eq!(timed_out.state(), &JobState::TimedOut);
    assert_eq!(provider.call_count(), 0);
    let job_events = harness
        .events
        .snapshot()
        .into_iter()
        .filter(|event| event.job_id() == Some(queued.record.id()))
        .map(|event| event.code())
        .collect::<Vec<_>>();
    assert_eq!(
        job_events,
        vec![JobEventCode::EnqueueAccepted, JobEventCode::TimedOut]
    );
}

#[test]
fn timeout_recovery_fences_a_claim_selected_by_the_same_dispatch() {
    let (harness, sink) =
        reentrant_harness(JobEventCode::TimedOut, ReentrantCall::RecoverOneFenced);
    let caller = caller("user.alice");
    let handler = handler_ref("recovery-fences-selected-claim");
    let provider = register(
        &harness,
        TestHandler::scripted(handler.clone(), []).with_recovery(RecoveryPolicy::NonIdempotent),
    );
    let expired = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler.clone())
                .with_constraints(JobConstraints::default().with_deadline(TimestampMillis(1))),
        )
        .unwrap();
    let selected = enqueue(&harness, &caller, &handler);
    harness.clock.set(TimestampMillis(1)).unwrap();

    let result = harness.scheduler.dispatch_one().unwrap();
    let reentry_completed = sink.finish_reentry();
    sink.detach_scheduler();

    assert!(
        reentry_completed,
        "reentrant recovery did not fence the claim"
    );
    assert!(matches!(
        result,
        DispatchResult::Superseded { record, .. } if record.id() == selected.record.id()
    ));
    assert_eq!(provider.call_count(), 0);
    assert_eq!(
        harness
            .store
            .get(expired.record.id())
            .unwrap()
            .unwrap()
            .state(),
        &JobState::TimedOut
    );
    assert_eq!(
        harness
            .store
            .get(selected.record.id())
            .unwrap()
            .unwrap()
            .state(),
        &JobState::RecoveryRequired(JobFailureCode::Permanent)
    );
    let selected_events = harness
        .events
        .snapshot()
        .into_iter()
        .filter(|event| event.job_id() == Some(selected.record.id()))
        .map(|event| event.code())
        .collect::<Vec<_>>();
    assert_eq!(
        selected_events,
        vec![
            JobEventCode::EnqueueAccepted,
            JobEventCode::RecoveryRequired
        ]
    );
}

#[test]
fn cancellation_during_started_diagnostic_fences_claim_before_provider() {
    let (harness, sink) = reentrant_harness(JobEventCode::Started, ReentrantCall::CancelCurrent);
    let caller = caller("user.alice");
    let handler = handler_ref("cancel-fences-started-claim");
    let provider = register(&harness, TestHandler::scripted(handler.clone(), []));
    let queued = enqueue(&harness, &caller, &handler);

    let result = harness.scheduler.dispatch_one().unwrap();
    let reentry_completed = sink.finish_reentry();
    sink.detach_scheduler();

    assert!(
        reentry_completed,
        "diagnostic callback could not cancel the claim"
    );
    assert!(matches!(
        result,
        DispatchResult::Completed(ref record)
            if record.id() == queued.record.id()
                && matches!(record.state(), JobState::Cancelled)
    ));
    assert_eq!(
        harness
            .store
            .get(queued.record.id())
            .unwrap()
            .unwrap()
            .state(),
        &JobState::Cancelled
    );
    assert!(!harness
        .scheduler
        .has_active_control_for_test(queued.record.id()));
    let selected_events = harness
        .events
        .snapshot()
        .into_iter()
        .filter(|event| event.job_id() == Some(queued.record.id()))
        .map(|event| event.code())
        .collect::<Vec<_>>();
    assert_eq!(
        selected_events,
        vec![
            JobEventCode::EnqueueAccepted,
            JobEventCode::Started,
            JobEventCode::CancellationRequested,
            JobEventCode::CancellationCompleted,
        ]
    );
    assert_eq!(provider.call_count(), 0);
}

#[test]
fn pause_during_started_diagnostic_settles_before_provider() {
    let (harness, sink) = reentrant_harness(JobEventCode::Started, ReentrantCall::PauseCurrent);
    let caller = caller("user.alice");
    let handler = handler_ref("pause-fences-started-claim");
    let provider = register(
        &harness,
        TestHandler::scripted(handler.clone(), []).with_pause(true),
    );
    let queued = enqueue(&harness, &caller, &handler);

    let result = harness.scheduler.dispatch_one().unwrap();
    let reentry_completed = sink.finish_reentry();
    sink.detach_scheduler();

    assert!(
        reentry_completed,
        "diagnostic callback could not pause the claim"
    );
    assert!(matches!(
        result,
        DispatchResult::Completed(ref record)
            if record.id() == queued.record.id() && matches!(record.state(), JobState::Paused)
    ));
    assert_eq!(
        harness
            .store
            .get(queued.record.id())
            .unwrap()
            .unwrap()
            .state(),
        &JobState::Paused
    );
    assert_eq!(provider.call_count(), 0);
}

#[test]
fn timeout_during_started_diagnostic_settles_before_provider() {
    let events = Arc::new(BoundedJobEventRecorder::new(32));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let diagnostics: Arc<dyn JobDiagnosticSink> = Arc::new(BlockingDiagnostics {
        events: events.clone(),
        order: Arc::new(Mutex::new(Vec::new())),
        block_on: JobEventCode::Started,
        entered: entered.clone(),
        release: release.clone(),
        blocked: AtomicBool::new(false),
    });
    let harness = harness_with_diagnostics(SchedulerConfig::default(), events, diagnostics);
    let caller = caller("user.alice");
    let handler = handler_ref("timeout-fences-started-claim");
    let provider = register(&harness, TestHandler::scripted(handler.clone(), []));
    let queued = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler)
                .with_constraints(JobConstraints::default().with_deadline(TimestampMillis(1))),
        )
        .unwrap();

    let scheduler = harness.scheduler.clone();
    let dispatch = thread::spawn(move || scheduler.dispatch_one());
    entered.wait();
    harness.clock.set(TimestampMillis(1)).unwrap();
    let second_dispatch = harness.scheduler.dispatch_one();
    release.wait();
    assert!(matches!(second_dispatch.unwrap(), DispatchResult::Idle));
    let result = dispatch.join().unwrap().unwrap();

    assert!(matches!(
        result,
        DispatchResult::Completed(ref record)
            if record.id() == queued.record.id() && matches!(record.state(), JobState::TimedOut)
    ));
    assert_eq!(
        harness
            .store
            .get(queued.record.id())
            .unwrap()
            .unwrap()
            .state(),
        &JobState::TimedOut
    );
    assert_eq!(provider.call_count(), 0);
}

#[test]
fn diagnostic_sink_panic_is_best_effort_after_claim() {
    let events = Arc::new(BoundedJobEventRecorder::new(32));
    let panic_sink = Arc::new(PanickingDiagnostics {
        events: events.clone(),
        trigger: JobEventCode::TimedOut,
        panicked: AtomicBool::new(false),
    });
    let diagnostics: Arc<dyn JobDiagnosticSink> = panic_sink.clone();
    let harness =
        harness_with_diagnostics(SchedulerConfig::new(1, 1, 3).unwrap(), events, diagnostics);
    let caller = caller("user.alice");
    let handler = handler_ref("panic-sink-guard-cleanup");
    let provider = register(
        &harness,
        TestHandler::scripted(handler.clone(), []).with_recovery(RecoveryPolicy::NonIdempotent),
    );
    let expired = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler.clone())
                .with_constraints(JobConstraints::default().with_deadline(TimestampMillis(1))),
        )
        .unwrap();
    let selected = enqueue(&harness, &caller, &handler);
    let later = enqueue(&harness, &caller, &handler);
    harness.clock.set(TimestampMillis(1)).unwrap();

    let result = harness.scheduler.dispatch_one().unwrap();
    assert!(panic_sink.panicked.load(Ordering::SeqCst));
    assert!(matches!(result, DispatchResult::Completed(_)));
    assert_eq!(
        harness
            .store
            .get(expired.record.id())
            .unwrap()
            .unwrap()
            .state(),
        &JobState::TimedOut
    );
    assert_eq!(
        harness
            .store
            .get(selected.record.id())
            .unwrap()
            .unwrap()
            .state(),
        &JobState::Succeeded
    );
    assert!(!harness
        .scheduler
        .has_active_control_for_test(selected.record.id()));

    assert!(matches!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Completed(_)
    ));
    assert_eq!(provider.call_count(), 2);
    assert!(matches!(
        harness
            .store
            .get(later.record.id())
            .unwrap()
            .unwrap()
            .state(),
        JobState::Succeeded
    ));
}

#[test]
fn recovered_diagnostic_can_reenter_startup_recovery_after_lock_release() {
    let (harness, sink) =
        reentrant_harness(JobEventCode::RecoveryRequired, ReentrantCall::RecoverEmpty);
    let caller = caller("user.alice");
    let handler = handler_ref("recovery-reentry");
    let provider = register(
        &harness,
        TestHandler::scripted(handler.clone(), []).with_recovery(RecoveryPolicy::NonIdempotent),
    );
    let queued = enqueue(&harness, &caller, &handler);
    harness
        .store
        .seed_state_for_test(queued.record.id(), JobState::Running);

    let recovered = harness.scheduler.recover_startup().unwrap();
    let reentry_completed = sink.finish_reentry();
    sink.detach_scheduler();

    assert!(
        reentry_completed,
        "diagnostic callback could not re-enter startup recovery"
    );
    assert_eq!(recovered.len(), 1);
    assert_eq!(
        recovered[0].state(),
        &JobState::RecoveryRequired(JobFailureCode::Permanent)
    );
    assert_eq!(provider.call_count(), 0);
    let job_events = harness
        .events
        .snapshot()
        .into_iter()
        .filter(|event| event.job_id() == Some(queued.record.id()))
        .map(|event| event.code())
        .collect::<Vec<_>>();
    assert_eq!(
        job_events,
        vec![
            JobEventCode::EnqueueAccepted,
            JobEventCode::RecoveryRequired
        ]
    );
}

#[test]
fn diagnostic_sink_panic_during_progress_does_not_fail_provider_outcome() {
    let events = Arc::new(BoundedJobEventRecorder::new(32));
    let panic_sink = Arc::new(PanickingDiagnostics {
        events: events.clone(),
        trigger: JobEventCode::Progress,
        panicked: AtomicBool::new(false),
    });
    let diagnostics: Arc<dyn JobDiagnosticSink> = panic_sink.clone();
    let harness = harness_with_diagnostics(SchedulerConfig::default(), events, diagnostics);
    let caller = caller("user.alice");
    let handler = handler_ref("progress-diagnostic-panic");
    let progress_results = Arc::new(Mutex::new(Vec::new()));
    let provider = register(
        &harness,
        TestHandler {
            reference: handler.clone(),
            mode: HandlerMode::ReportProgress(progress_results.clone()),
            recovery: RecoveryPolicy::NonIdempotent,
            supports_pause: false,
            checkpoint_valid: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        },
    );
    let queued = enqueue(&harness, &caller, &handler);

    assert!(matches!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Completed(ref record)
            if record.id() == queued.record.id() && matches!(record.state(), JobState::Succeeded)
    ));
    assert!(panic_sink.panicked.load(Ordering::SeqCst));
    assert_eq!(provider.call_count(), 1);
    assert_eq!(
        harness
            .store
            .get(queued.record.id())
            .unwrap()
            .unwrap()
            .state(),
        &JobState::Succeeded
    );
    assert_eq!(
        progress_results.lock().unwrap().as_slice(),
        &[Ok(()), Err(ProgressError::RateLimited)]
    );
}

#[test]
fn progress_diagnostic_waits_behind_started_in_the_shared_fifo() {
    let events = Arc::new(BoundedJobEventRecorder::new(32));
    let order = Arc::new(Mutex::new(Vec::new()));
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let diagnostics: Arc<dyn JobDiagnosticSink> = Arc::new(BlockingDiagnostics {
        events: events.clone(),
        order: order.clone(),
        block_on: JobEventCode::EnqueueAccepted,
        entered: entered.clone(),
        release: release.clone(),
        blocked: AtomicBool::new(false),
    });
    let harness = harness_with_diagnostics(SchedulerConfig::default(), events, diagnostics);
    let owner = caller("user.alice");
    let handler = handler_ref("progress-fifo-order");
    let progress_results = Arc::new(Mutex::new(Vec::new()));
    let provider = register(
        &harness,
        TestHandler {
            reference: handler.clone(),
            mode: HandlerMode::ReportProgress(progress_results.clone()),
            recovery: RecoveryPolicy::NonIdempotent,
            supports_pause: false,
            checkpoint_valid: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        },
    );
    let scheduler = harness.scheduler.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let enqueue_worker = thread::spawn(move || {
        let result = scheduler.enqueue(&owner, JobRequest::new(handler));
        sender.send(result).unwrap();
    });
    entered.wait();

    assert!(matches!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Completed(_)
    ));
    release.wait();
    let enqueued = receiver.recv().unwrap().unwrap();
    enqueue_worker.join().unwrap();

    assert_eq!(provider.call_count(), 1);
    assert_eq!(
        progress_results.lock().unwrap().as_slice(),
        &[Ok(()), Err(ProgressError::RateLimited)]
    );
    let observed = order.lock().unwrap().clone();
    let started = observed
        .iter()
        .position(|code| *code == JobEventCode::Started)
        .unwrap();
    let progress = observed
        .iter()
        .position(|code| *code == JobEventCode::Progress)
        .unwrap();
    assert!(
        started < progress,
        "progress overtook its Started event: {observed:?}"
    );
    assert_eq!(
        harness
            .events
            .snapshot()
            .into_iter()
            .filter(|event| event.job_id() == Some(enqueued.record.id()))
            .map(|event| event.code())
            .collect::<Vec<_>>(),
        vec![
            JobEventCode::EnqueueAccepted,
            JobEventCode::Started,
            JobEventCode::Progress,
            JobEventCode::StateTransition,
        ]
    );
}

#[test]
fn offline_conditions_are_injected_and_diagnostics_failures_do_not_change_jobs() {
    let offline_harness = harness(SchedulerConfig::default());
    let alice = caller("user.alice");
    let handler = handler_ref("offline-condition");
    register(&offline_harness, TestHandler::scripted(handler.clone(), []));
    let queued = offline_harness
        .scheduler
        .enqueue(
            &alice,
            JobRequest::new(handler.clone()).with_constraints(
                JobConstraints::default().with_network_policy(NetworkPolicy::RequireNetwork),
            ),
        )
        .unwrap();
    assert_eq!(
        offline_harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    assert_eq!(
        record(&offline_harness, &alice, queued.record.id()).state(),
        &JobState::Queued
    );
    offline_harness
        .scheduler
        .set_conditions(SystemConditions {
            network: NetworkConditions {
                available: true,
                unmetered: true,
            },
            system_idle: false,
            external_power: false,
        })
        .unwrap();
    assert!(matches!(
        offline_harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Completed(_)
    ));

    let store = Arc::new(InMemoryJobStore::new());
    let clock = Arc::new(ManualClock::new(TimestampMillis(0)));
    let auth = Arc::new(TestAuthority::default());
    let providers = Arc::new(JobProviderRegistry::new());
    let failed_sink_scheduler = Scheduler::new(
        store,
        clock,
        auth,
        Arc::new(FailingDiagnostics),
        providers.clone(),
        SchedulerConfig::default(),
    )
    .unwrap();
    let failure_caller = caller("user.alice");
    let handler = handler_ref("sink-failure");
    let provider = Arc::new(TestHandler::scripted(handler.clone(), []));
    providers.register(provider.clone()).unwrap();
    let job = failed_sink_scheduler
        .enqueue(&failure_caller, JobRequest::new(handler))
        .unwrap();
    let DispatchResult::Completed(completed) = failed_sink_scheduler.dispatch_one().unwrap() else {
        panic!("job should complete even when diagnostics fails")
    };
    assert_eq!(completed.id(), job.record.id());
    assert_eq!(completed.state(), &JobState::Succeeded);
    assert_eq!(provider.call_count(), 1);
}

#[test]
fn progress_is_rate_limited_bounded_and_redacted_by_construction() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler = handler_ref("progress");
    let results = Arc::new(Mutex::new(Vec::new()));
    let provider = TestHandler {
        reference: handler.clone(),
        mode: HandlerMode::ReportProgress(results.clone()),
        recovery: RecoveryPolicy::NonIdempotent,
        supports_pause: false,
        checkpoint_valid: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
    };
    register(&harness, provider);
    let job = enqueue(&harness, &caller, &handler);
    let DispatchResult::Completed(completed) = harness.scheduler.dispatch_one().unwrap() else {
        panic!("progress provider should run")
    };
    assert_eq!(completed.state(), &JobState::Succeeded);
    assert_eq!(
        results.lock().unwrap().as_slice(),
        &[Ok(()), Err(ProgressError::RateLimited)]
    );
    let saved = record(&harness, &caller, job.record.id());
    assert_eq!(saved.progress().unwrap().completed(), 2);
    let progress_event = harness
        .events
        .snapshot()
        .into_iter()
        .find(|event| event.code() == JobEventCode::Progress)
        .unwrap();
    let encoded = serde_json::to_string(&progress_event).unwrap();
    assert!(encoded.contains("phase-a"));
    assert!(!encoded.contains("user.alice"));
    assert!(!encoded.contains("secret"));

    let bounded = BoundedJobEventRecorder::new(1);
    for event in harness.events.snapshot().into_iter().take(2) {
        bounded.record(event).unwrap();
    }
    assert!(bounded.snapshot().len() <= 1);
}

#[test]
fn store_interruption_and_corruption_fail_closed_without_partial_insert() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler = handler_ref("store-failure");
    register(&harness, TestHandler::scripted(handler.clone(), []));
    harness.store.fail_next_write_for_test();
    assert_eq!(
        harness
            .scheduler
            .enqueue(&caller, JobRequest::new(handler.clone()))
            .unwrap_err(),
        SchedulerError::Store(JobStoreError::Unavailable)
    );
    assert!(harness.scheduler.list_visible(&caller).unwrap().is_empty());

    let queued = enqueue(&harness, &caller, &handler);
    harness.store.fail_reads_for_test(true);
    assert_eq!(
        harness
            .scheduler
            .inspect(&caller, queued.record.id())
            .unwrap_err(),
        SchedulerError::Store(JobStoreError::Unavailable)
    );
    harness.store.fail_reads_for_test(false);
    assert_eq!(
        record(&harness, &caller, queued.record.id()).state(),
        &JobState::Queued
    );
}

#[test]
fn deadlines_and_invalid_bounds_fail_before_handler_start() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler = handler_ref("deadline");
    let provider = register(&harness, TestHandler::scripted(handler.clone(), []));
    assert_eq!(
        harness
            .scheduler
            .enqueue(
                &caller,
                JobRequest::new(handler.clone())
                    .with_constraints(JobConstraints::default().with_timeout(DurationMillis(0)),),
            )
            .unwrap_err(),
        SchedulerError::InvalidRequest(ConstraintError::ZeroRuntimeBound)
    );
    let expired = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler.clone())
                .with_constraints(JobConstraints::default().with_deadline(TimestampMillis(1))),
        )
        .unwrap();
    harness.clock.set(TimestampMillis(1)).unwrap();
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    assert_eq!(
        record(&harness, &caller, expired.record.id()).state(),
        &JobState::TimedOut
    );
    assert_eq!(provider.call_count(), 0);

    harness.clock.set(TimestampMillis(2)).unwrap();
    assert_eq!(
        harness
            .scheduler
            .enqueue(
                &caller,
                JobRequest::new(handler)
                    .with_constraints(JobConstraints::default().with_deadline(TimestampMillis(2)),),
            )
            .unwrap_err(),
        SchedulerError::DeadlineAlreadyPassed
    );
}

#[test]
fn queued_deadline_and_handler_revocation_do_not_mutate_records_without_dispatch() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let deadline_handler = handler_ref("deadline-without-dispatch");
    let revoked_handler = handler_ref("revoked-without-dispatch");
    let deadline_provider = register(
        &harness,
        TestHandler::scripted(deadline_handler.clone(), []),
    );
    let revoked_provider = register(&harness, TestHandler::scripted(revoked_handler.clone(), []));
    let deadline_record = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(deadline_handler)
                .with_constraints(JobConstraints::default().with_deadline(TimestampMillis(1))),
        )
        .unwrap()
        .record;
    let revoked_record = harness
        .scheduler
        .enqueue(&caller, JobRequest::new(revoked_handler))
        .unwrap()
        .record;
    let events_before_observation = harness.events.snapshot();

    harness.authority.deny_handler.store(true, Ordering::SeqCst);
    harness.clock.set(TimestampMillis(1)).unwrap();

    assert_eq!(
        harness
            .scheduler
            .inspect(&caller, deadline_record.id())
            .unwrap(),
        deadline_record
    );
    assert_eq!(
        harness
            .scheduler
            .inspect(&caller, revoked_record.id())
            .unwrap(),
        revoked_record
    );
    assert_eq!(harness.scheduler.list_visible(&caller).unwrap().len(), 2);
    assert_eq!(harness.events.snapshot(), events_before_observation);
    assert_eq!(deadline_provider.call_count(), 0);
    assert_eq!(revoked_provider.call_count(), 0);
}

#[test]
fn startup_then_dispatch_times_out_expired_queued_job_once_before_authorization() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler = handler_ref("expired-before-revoked-handler");
    let provider = register(&harness, TestHandler::scripted(handler.clone(), []));
    let queued = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler)
                .with_constraints(JobConstraints::default().with_deadline(TimestampMillis(1))),
        )
        .unwrap()
        .record;
    let events_before_recovery = harness.events.snapshot();

    harness.authority.deny_handler.store(true, Ordering::SeqCst);
    harness.clock.set(TimestampMillis(1)).unwrap();
    assert!(harness.scheduler.recover_startup().unwrap().is_empty());
    assert_eq!(
        harness.store.get(queued.id()).unwrap(),
        Some(queued.clone())
    );
    assert_eq!(harness.events.snapshot(), events_before_recovery);

    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    let timed_out = harness.store.get(queued.id()).unwrap().unwrap();
    assert_eq!(timed_out.state(), &JobState::TimedOut);
    assert_eq!(provider.call_count(), 0);
    let job_events = harness
        .events
        .snapshot()
        .into_iter()
        .filter(|event| event.job_id() == Some(queued.id()))
        .collect::<Vec<_>>();
    assert_eq!(job_events.len(), 2);
    assert_eq!(job_events[0].code(), JobEventCode::EnqueueAccepted);
    assert_eq!(job_events[1].code(), JobEventCode::TimedOut);
    assert_eq!(
        job_events[1].reason(),
        Some(JobEventReason::DeadlineExceeded)
    );

    let events_after_timeout = harness.events.snapshot();
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    assert!(harness.scheduler.recover_startup().unwrap().is_empty());
    assert_eq!(harness.store.get(queued.id()).unwrap(), Some(timed_out));
    assert_eq!(harness.events.snapshot(), events_after_timeout);
}

#[test]
fn saturated_manual_clock_expires_max_deadline_once() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler = handler_ref("max-time-deadline");
    let provider = register(&harness, TestHandler::scripted(handler.clone(), []));
    let before_max = TimestampMillis(u64::MAX - 1);
    let at_max = TimestampMillis(u64::MAX);
    harness.clock.set(before_max).unwrap();
    let queued = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler)
                .with_constraints(JobConstraints::default().with_deadline(at_max)),
        )
        .unwrap()
        .record;

    assert_eq!(harness.clock.advance(DurationMillis(2)), at_max);
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    let timed_out = harness.store.get(queued.id()).unwrap().unwrap();
    assert_eq!(timed_out.state(), &JobState::TimedOut);
    assert_eq!(timed_out.finished_at(), Some(at_max));
    assert_eq!(provider.call_count(), 0);
    let job_events = harness
        .events
        .snapshot()
        .into_iter()
        .filter(|event| event.job_id() == Some(queued.id()))
        .collect::<Vec<_>>();
    assert_eq!(job_events.len(), 2);
    assert_eq!(job_events[0].code(), JobEventCode::EnqueueAccepted);
    assert_eq!(job_events[0].timestamp(), before_max);
    assert_eq!(job_events[1].code(), JobEventCode::TimedOut);
    assert_eq!(job_events[1].timestamp(), at_max);
    assert_eq!(
        job_events[1].reason(),
        Some(JobEventReason::DeadlineExceeded)
    );

    let events_after_timeout = harness.events.snapshot();
    assert_eq!(harness.clock.advance(DurationMillis(1)), at_max);
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    assert_eq!(harness.store.get(queued.id()).unwrap(), Some(timed_out));
    assert_eq!(harness.events.snapshot(), events_after_timeout);
}

#[test]
fn saturated_runtime_deadline_does_not_expire_before_max() {
    let harness = harness(SchedulerConfig::default());
    let caller = caller("user.alice");
    let handler = handler_ref("saturated-runtime-before-max");
    let provider = register(
        &harness,
        TestHandler::scripted(handler.clone(), [HandlerOutcome::Succeeded]),
    );
    let before_max = TimestampMillis(u64::MAX - 1);
    harness.clock.set(before_max).unwrap();
    let queued = harness
        .scheduler
        .enqueue(
            &caller,
            JobRequest::new(handler)
                .with_constraints(JobConstraints::default().with_timeout(DurationMillis(2))),
        )
        .unwrap()
        .record;

    let DispatchResult::Completed(completed) = harness.scheduler.dispatch_one().unwrap() else {
        panic!("job should complete before its saturated runtime deadline")
    };
    assert_eq!(completed.id(), queued.id());
    assert_eq!(completed.state(), &JobState::Succeeded);
    assert_eq!(completed.started_at(), Some(before_max));
    assert_eq!(completed.finished_at(), Some(before_max));
    assert_eq!(provider.call_count(), 1);
}

#[test]
fn missing_registered_provider_fails_without_invoking_arbitrary_code() {
    let harness = harness(SchedulerConfig::default());
    let alice = caller("user.alice");
    let queued = enqueue(&harness, &alice, &handler_ref("unregistered"));
    assert_eq!(
        harness.scheduler.dispatch_one().unwrap(),
        DispatchResult::Idle
    );
    assert_eq!(
        record(&harness, &alice, queued.record.id()).state(),
        &JobState::Failed(JobFailureCode::MissingHandler)
    );
    assert!(harness
        .events
        .snapshot()
        .iter()
        .any(|event| event.code() == JobEventCode::HandlerMissing));
}

mod fallible_clock_contracts {
    use super::*;

    #[derive(Default)]
    struct FaultClock {
        unavailable: AtomicBool,
        reads: AtomicUsize,
        fail_on_read: AtomicUsize,
    }
    impl FallibleClock for FaultClock {
        fn try_now(&self) -> Result<TimestampMillis, ClockUnavailable> {
            let read = self.reads.fetch_add(1, Ordering::SeqCst) + 1;
            let fail_on_read = self.fail_on_read.load(Ordering::SeqCst);
            if self.unavailable.load(Ordering::SeqCst)
                || (fail_on_read != 0 && read >= fail_on_read)
            {
                Err(ClockUnavailable)
            } else {
                Ok(TimestampMillis(10))
            }
        }
    }
    struct Probe {
        clock: Arc<FaultClock>,
        reference: HandlerRef,
        mode: u8,
        outcome: HandlerOutcome,
        calls: AtomicUsize,
        progress: Mutex<Vec<Result<(), ProgressError>>>,
    }
    impl JobHandler for Probe {
        fn handler_ref(&self) -> HandlerRef {
            self.reference.clone()
        }
        fn recovery_policy(&self, _record: &JobRecord) -> RecoveryPolicy {
            RecoveryPolicy::NonIdempotent
        }
        fn execute(&self, context: &mut HandlerContext) -> HandlerOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.mode > 0 {
                self.clock.unavailable.store(true, Ordering::SeqCst);
            }
            if self.mode == 2 {
                let summary =
                    ProgressSummary::new(ProgressCode::new("clock-progress").unwrap(), 1, Some(2))
                        .unwrap();
                self.progress
                    .lock()
                    .unwrap()
                    .push(context.report_progress(summary));
                self.progress.lock().unwrap().push(
                    context.publish_checkpoint(CheckpointRef::new("clock-checkpoint").unwrap()),
                );
            }
            self.outcome.clone()
        }
    }
    struct Sink {
        clock: Arc<FaultClock>,
        fail_at: Option<JobEventCode>,
        events: Arc<BoundedJobEventRecorder>,
    }
    impl JobDiagnosticSink for Sink {
        fn record(&self, event: JobDiagnosticEvent) -> Result<(), JobDiagnosticSinkError> {
            let fail = Some(event.code()) == self.fail_at;
            self.events.record(event)?;
            if fail {
                self.clock.unavailable.store(true, Ordering::SeqCst);
                Err(JobDiagnosticSinkError)
            } else {
                Ok(())
            }
        }
    }
    struct FailOnNthRecoveryWriteStore {
        inner: Arc<InMemoryJobStore>,
        compare_and_swap_countdown: AtomicUsize,
    }

    impl FailOnNthRecoveryWriteStore {
        fn fail_on_nth_compare_and_swap(&self, nth: usize) {
            self.compare_and_swap_countdown.store(nth, Ordering::SeqCst);
        }
    }

    impl JobStore for FailOnNthRecoveryWriteStore {
        fn insert(
            &self,
            record: JobRecord,
            dedupe: Option<(DedupeScope, TimestampMillis)>,
            now: TimestampMillis,
            queue_capacity: usize,
            record_capacity: usize,
        ) -> Result<InsertOutcome, JobStoreError> {
            self.inner
                .insert(record, dedupe, now, queue_capacity, record_capacity)
        }

        fn get(&self, id: &JobId) -> Result<Option<JobRecord>, JobStoreError> {
            self.inner.get(id)
        }

        fn list(&self) -> Result<Vec<JobRecord>, JobStoreError> {
            self.inner.list()
        }

        fn compare_and_swap(
            &self,
            id: &JobId,
            expected_revision: u64,
            replacement: JobRecord,
        ) -> Result<JobRecord, JobStoreError> {
            match self.compare_and_swap_countdown.fetch_update(
                Ordering::SeqCst,
                Ordering::SeqCst,
                |remaining| {
                    if remaining > 0 {
                        Some(remaining - 1)
                    } else {
                        None
                    }
                },
            ) {
                Ok(1) => Err(JobStoreError::Unavailable),
                Ok(_) | Err(_) => self
                    .inner
                    .compare_and_swap(id, expected_revision, replacement),
            }
        }

        fn claim(
            &self,
            id: &JobId,
            expected_revision: u64,
            now: TimestampMillis,
            global_limit: usize,
            per_owner_limit: usize,
        ) -> Result<Option<JobRecord>, JobStoreError> {
            self.inner
                .claim(id, expected_revision, now, global_limit, per_owner_limit)
        }
    }
    struct CompletionFaultStore {
        inner: Arc<InMemoryJobStore>,
        compare_and_swap_fault: AtomicUsize,
    }
    impl JobStore for CompletionFaultStore {
        fn insert(
            &self,
            record: JobRecord,
            dedupe: Option<(DedupeScope, TimestampMillis)>,
            now: TimestampMillis,
            queue_capacity: usize,
            record_capacity: usize,
        ) -> Result<InsertOutcome, JobStoreError> {
            self.inner
                .insert(record, dedupe, now, queue_capacity, record_capacity)
        }

        fn get(&self, id: &JobId) -> Result<Option<JobRecord>, JobStoreError> {
            self.inner.get(id)
        }

        fn list(&self) -> Result<Vec<JobRecord>, JobStoreError> {
            self.inner.list()
        }

        fn compare_and_swap(
            &self,
            id: &JobId,
            expected_revision: u64,
            replacement: JobRecord,
        ) -> Result<JobRecord, JobStoreError> {
            match self.compare_and_swap_fault.swap(0, Ordering::SeqCst) {
                1 => Err(JobStoreError::Unavailable),
                2 => {
                    self.inner
                        .compare_and_swap(id, expected_revision, replacement)?;
                    Err(JobStoreError::Unavailable)
                }
                _ => self
                    .inner
                    .compare_and_swap(id, expected_revision, replacement),
            }
        }

        fn claim(
            &self,
            id: &JobId,
            expected_revision: u64,
            now: TimestampMillis,
            global_limit: usize,
            per_owner_limit: usize,
        ) -> Result<Option<JobRecord>, JobStoreError> {
            self.inner
                .claim(id, expected_revision, now, global_limit, per_owner_limit)
        }
    }
    struct Fixture {
        s: Scheduler,
        store: Arc<InMemoryJobStore>,
        clock: Arc<FaultClock>,
        handler: Arc<Probe>,
        events: Arc<BoundedJobEventRecorder>,
    }
    fn fixture(mode: u8, fail_at: Option<JobEventCode>) -> Fixture {
        let store = Arc::new(InMemoryJobStore::new());
        let clock = Arc::new(FaultClock::default());
        let events = Arc::new(BoundedJobEventRecorder::new(64));
        let providers = Arc::new(JobProviderRegistry::new());
        let handler = Arc::new(Probe {
            clock: clock.clone(),
            reference: handler_ref("fallible-clock"),
            mode,
            outcome: HandlerOutcome::Succeeded,
            calls: AtomicUsize::new(0),
            progress: Mutex::new(Vec::new()),
        });
        providers.register(handler.clone()).unwrap();
        let sink = Arc::new(Sink {
            clock: clock.clone(),
            fail_at,
            events: events.clone(),
        });
        let s = Scheduler::new_with_fallible_clock(
            store.clone(),
            clock.clone(),
            Arc::new(TestAuthority::default()),
            sink,
            providers,
            SchedulerConfig::default(),
        )
        .unwrap();
        Fixture {
            s,
            store,
            clock,
            handler,
            events,
        }
    }
    fn queued(f: &Fixture) -> JobRecord {
        f.s.enqueue(
            &caller("user.alice"),
            JobRequest::new(f.handler.handler_ref()),
        )
        .unwrap()
        .record
    }

    struct BlockingHandler {
        reference: HandlerRef,
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
        token: Arc<Mutex<Option<CancellationToken>>>,
        supports_pause: bool,
    }
    impl JobHandler for BlockingHandler {
        fn handler_ref(&self) -> HandlerRef {
            self.reference.clone()
        }
        fn supports_pause(&self) -> bool {
            self.supports_pause
        }
        fn execute(&self, context: &mut HandlerContext) -> HandlerOutcome {
            *self.token.lock().unwrap() = Some(context.cancellation_token());
            self.entered.wait();
            self.release.wait();
            HandlerOutcome::Succeeded
        }
    }

    struct ProgressClockFailureTrigger {
        reference: HandlerRef,
        clock: Arc<FaultClock>,
        progress_reported: Arc<Barrier>,
        release: Arc<Barrier>,
        result: Arc<Mutex<Option<Result<(), ProgressError>>>>,
    }
    impl JobHandler for ProgressClockFailureTrigger {
        fn handler_ref(&self) -> HandlerRef {
            self.reference.clone()
        }
        fn execute(&self, context: &mut HandlerContext) -> HandlerOutcome {
            self.clock.unavailable.store(true, Ordering::SeqCst);
            let summary =
                ProgressSummary::new(ProgressCode::new("peer-clock-failure").unwrap(), 1, Some(2))
                    .unwrap();
            *self.result.lock().unwrap() = Some(context.report_progress(summary));
            self.progress_reported.wait();
            self.release.wait();
            HandlerOutcome::Succeeded
        }
    }

    #[derive(Clone, Copy)]
    enum ActiveClockFailurePath {
        Cancel,
        Pause,
        TimeoutRefresh,
    }

    fn verify_active_handler_receives_clock_failure(path: ActiveClockFailurePath) {
        let store = Arc::new(InMemoryJobStore::new());
        let clock = Arc::new(FaultClock::default());
        let events = Arc::new(BoundedJobEventRecorder::new(32));
        let providers = Arc::new(JobProviderRegistry::new());
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let token = Arc::new(Mutex::new(None));
        let handler = Arc::new(BlockingHandler {
            reference: handler_ref("blocked-clock-failure"),
            entered: entered.clone(),
            release: release.clone(),
            token: token.clone(),
            supports_pause: matches!(path, ActiveClockFailurePath::Pause),
        });
        providers.register(handler.clone()).unwrap();
        let scheduler = Scheduler::new_with_fallible_clock(
            store.clone(),
            clock.clone(),
            Arc::new(TestAuthority::default()),
            events,
            providers,
            SchedulerConfig::default(),
        )
        .unwrap();
        let caller = caller("user.alice");
        let record = scheduler
            .enqueue(&caller, JobRequest::new(handler.handler_ref()))
            .unwrap()
            .record;
        let dispatch_scheduler = scheduler.clone();
        let dispatch = thread::spawn(move || dispatch_scheduler.dispatch_one());
        entered.wait();

        clock.unavailable.store(true, Ordering::SeqCst);
        let trigger_result = match path {
            ActiveClockFailurePath::Cancel => scheduler.cancel(&caller, record.id()).map(|_| ()),
            ActiveClockFailurePath::Pause => scheduler.pause(&caller, record.id()).map(|_| ()),
            ActiveClockFailurePath::TimeoutRefresh => scheduler.dispatch_one().map(|_| ()),
        };
        let observed_reason = token.lock().unwrap().as_ref().unwrap().reason();
        release.wait();
        let dispatch_result = dispatch.join().unwrap();

        assert_eq!(trigger_result, Err(SchedulerError::ClockUnavailable));
        assert_eq!(observed_reason, Some(CancellationReason::ClockUnavailable));
        assert_eq!(dispatch_result, Err(SchedulerError::ClockUnavailable));
        let saved = store.get(record.id()).unwrap().unwrap();
        assert!(matches!(saved.state(), JobState::Running));
        assert_eq!(scheduler.retained_clock_outcomes().unwrap().len(), 1);
    }

    #[test]
    fn completion_clock_failure_signals_other_active_handler() {
        let store = Arc::new(InMemoryJobStore::new());
        let clock = Arc::new(FaultClock::default());
        let events = Arc::new(BoundedJobEventRecorder::new(32));
        let providers = Arc::new(JobProviderRegistry::new());
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let token = Arc::new(Mutex::new(None));
        let blocked = Arc::new(BlockingHandler {
            reference: handler_ref("blocked-during-completion-clock-failure"),
            entered: entered.clone(),
            release: release.clone(),
            token: token.clone(),
            supports_pause: false,
        });
        let trigger = Arc::new(Probe {
            clock: clock.clone(),
            reference: handler_ref("completion-clock-failure-trigger"),
            mode: 1,
            outcome: HandlerOutcome::Succeeded,
            calls: AtomicUsize::new(0),
            progress: Mutex::new(Vec::new()),
        });
        providers.register(blocked.clone()).unwrap();
        providers.register(trigger.clone()).unwrap();
        let scheduler = Scheduler::new_with_fallible_clock(
            store.clone(),
            clock.clone(),
            Arc::new(TestAuthority::default()),
            events,
            providers,
            SchedulerConfig::new(2, 2, 8).unwrap(),
        )
        .unwrap();
        let owner = caller("user.alice");
        let blocked_record = scheduler
            .enqueue(&owner, JobRequest::new(blocked.handler_ref()))
            .unwrap()
            .record;
        let trigger_record = scheduler
            .enqueue(&owner, JobRequest::new(trigger.handler_ref()))
            .unwrap()
            .record;

        let blocked_scheduler = scheduler.clone();
        let blocked_dispatch = thread::spawn(move || blocked_scheduler.dispatch_one());
        entered.wait();

        let trigger_scheduler = scheduler.clone();
        let trigger_dispatch = thread::spawn(move || trigger_scheduler.dispatch_one());
        let trigger_result = trigger_dispatch.join().unwrap();
        let observed_reason = token.lock().unwrap().as_ref().unwrap().reason();

        release.wait();
        let blocked_result = blocked_dispatch.join().unwrap();

        assert_eq!(trigger_result, Err(SchedulerError::ClockUnavailable));
        assert_eq!(observed_reason, Some(CancellationReason::ClockUnavailable));
        assert_eq!(blocked_result, Err(SchedulerError::ClockUnavailable));
        assert_eq!(trigger.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            store.get(blocked_record.id()).unwrap().unwrap().state(),
            JobState::Running
        ));
        assert!(matches!(
            store.get(trigger_record.id()).unwrap().unwrap().state(),
            JobState::Running
        ));
        assert_eq!(scheduler.retained_clock_outcomes().unwrap().len(), 2);
    }

    #[test]
    fn progress_clock_failure_signals_other_active_handler_before_completion() {
        let store = Arc::new(InMemoryJobStore::new());
        let clock = Arc::new(FaultClock::default());
        let events = Arc::new(BoundedJobEventRecorder::new(32));
        let providers = Arc::new(JobProviderRegistry::new());
        let observer_entered = Arc::new(Barrier::new(2));
        let observer_release = Arc::new(Barrier::new(2));
        let observer_token = Arc::new(Mutex::new(None));
        let observer = Arc::new(BlockingHandler {
            reference: handler_ref("blocked-during-progress-clock-failure"),
            entered: observer_entered.clone(),
            release: observer_release.clone(),
            token: observer_token.clone(),
            supports_pause: false,
        });
        let progress_reported = Arc::new(Barrier::new(2));
        let trigger_release = Arc::new(Barrier::new(2));
        let progress_result = Arc::new(Mutex::new(None));
        let trigger = Arc::new(ProgressClockFailureTrigger {
            reference: handler_ref("progress-clock-failure-trigger"),
            clock: clock.clone(),
            progress_reported: progress_reported.clone(),
            release: trigger_release.clone(),
            result: progress_result.clone(),
        });
        providers.register(observer.clone()).unwrap();
        providers.register(trigger.clone()).unwrap();
        let scheduler = Scheduler::new_with_fallible_clock(
            store.clone(),
            clock,
            Arc::new(TestAuthority::default()),
            events,
            providers,
            SchedulerConfig::new(2, 2, 8).unwrap(),
        )
        .unwrap();
        let owner = caller("user.alice");
        let observer_record = scheduler
            .enqueue(&owner, JobRequest::new(observer.handler_ref()))
            .unwrap()
            .record;
        let trigger_record = scheduler
            .enqueue(&owner, JobRequest::new(trigger.handler_ref()))
            .unwrap()
            .record;

        let observer_scheduler = scheduler.clone();
        let observer_dispatch = thread::spawn(move || observer_scheduler.dispatch_one());
        observer_entered.wait();

        let trigger_scheduler = scheduler.clone();
        let trigger_dispatch = thread::spawn(move || trigger_scheduler.dispatch_one());
        progress_reported.wait();
        let observed_reason = observer_token.lock().unwrap().as_ref().unwrap().reason();

        trigger_release.wait();
        let trigger_result = trigger_dispatch.join().unwrap();
        observer_release.wait();
        let observer_result = observer_dispatch.join().unwrap();

        assert_eq!(
            *progress_result.lock().unwrap(),
            Some(Err(ProgressError::ClockUnavailable))
        );
        assert_eq!(observed_reason, Some(CancellationReason::ClockUnavailable));
        assert_eq!(trigger_result, Err(SchedulerError::ClockUnavailable));
        assert_eq!(observer_result, Err(SchedulerError::ClockUnavailable));
        for record in [&observer_record, &trigger_record] {
            assert!(matches!(
                store.get(record.id()).unwrap().unwrap().state(),
                JobState::Running
            ));
        }
        assert_eq!(scheduler.retained_clock_outcomes().unwrap().len(), 2);
    }

    #[test]
    fn clock_failure_before_enqueue_leaves_store_and_diagnostics_unchanged() {
        let f = fixture(0, None);
        f.clock.unavailable.store(true, Ordering::SeqCst);
        assert_eq!(
            f.s.enqueue(
                &caller("user.alice"),
                JobRequest::new(f.handler.handler_ref())
            ),
            Err(SchedulerError::ClockUnavailable)
        );
        assert!(f.store.list().unwrap().is_empty());
        assert!(f.events.snapshot().is_empty());
        assert_eq!(f.handler.calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn clock_failure_before_dispatch_does_not_claim_or_start_handler() {
        let f = fixture(0, None);
        let j = queued(&f);
        let events = f.events.snapshot();
        f.clock.unavailable.store(true, Ordering::SeqCst);
        assert_eq!(f.s.dispatch_one(), Err(SchedulerError::ClockUnavailable));
        assert_eq!(f.store.get(j.id()).unwrap(), Some(j));
        assert_eq!(f.events.snapshot(), events);
        assert_eq!(f.handler.calls.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn clock_failure_between_claim_and_execute_preserves_unexecuted_attempt() {
        let f = fixture(0, Some(JobEventCode::Started));
        let j = queued(&f);
        assert_eq!(f.s.dispatch_one(), Err(SchedulerError::ClockUnavailable));
        assert_eq!(f.handler.calls.load(Ordering::SeqCst), 0);
        let retained = f.s.retained_clock_outcomes().unwrap();
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].claimed().id(), j.id());
        assert_eq!(retained[0].outcome(), None);
        assert!(!retained[0].handler_executed());
        assert!(matches!(
            f.store.get(j.id()).unwrap().unwrap().state(),
            JobState::Running
        ));
    }
    #[test]
    fn post_handler_clock_failure_retains_actual_outcome_and_fences_clones() {
        let f = fixture(1, None);
        let j = queued(&f);
        let peer = f.s.clone();
        assert_eq!(f.s.dispatch_one(), Err(SchedulerError::ClockUnavailable));
        assert_eq!(f.handler.calls.load(Ordering::SeqCst), 1);
        let saved = f.store.get(j.id()).unwrap().unwrap();
        assert!(matches!(saved.state(), JobState::Running));
        assert_eq!(saved.attempt_count(), 1);
        assert_eq!(saved.finished_at(), None);
        let retained = peer.retained_clock_outcomes().unwrap();
        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].outcome(), Some(&HandlerOutcome::Succeeded));
        assert!(retained[0].handler_executed());
        f.clock.unavailable.store(false, Ordering::SeqCst);
        assert_eq!(peer.dispatch_one(), Err(SchedulerError::ClockUnavailable));
        assert_eq!(
            peer.recover_startup(),
            Err(SchedulerError::ClockUnavailable)
        );
        assert_eq!(
            peer.cancel(&caller("user.alice"), j.id()),
            Err(SchedulerError::ClockUnavailable)
        );
        assert_eq!(f.store.get(j.id()).unwrap(), Some(saved));
        assert_eq!(f.handler.calls.load(Ordering::SeqCst), 1);
        assert_eq!(peer.retained_clock_outcomes().unwrap(), retained);
    }
    #[test]
    fn clock_failure_in_real_progress_chain_does_not_publish_progress_or_checkpoint() {
        let f = fixture(2, None);
        let j = queued(&f);
        assert_eq!(f.s.dispatch_one(), Err(SchedulerError::ClockUnavailable));
        assert_eq!(
            *f.handler.progress.lock().unwrap(),
            vec![
                Err(ProgressError::ClockUnavailable),
                Err(ProgressError::ClockUnavailable)
            ]
        );
        let saved = f.store.get(j.id()).unwrap().unwrap();
        assert_eq!(saved.progress(), None);
        assert_eq!(saved.checkpoint_ref(), None);
        assert_eq!(saved.revision(), j.revision() + 1);
        assert!(!f
            .events
            .snapshot()
            .iter()
            .any(|e| e.code() == JobEventCode::Progress));
        assert_eq!(
            f.s.retained_clock_outcomes().unwrap()[0].outcome(),
            Some(&HandlerOutcome::Succeeded)
        );
    }
    #[test]
    fn commit_success_survives_diagnostic_failure_and_clock_loss() {
        let f = fixture(0, Some(JobEventCode::StateTransition));
        let j = queued(&f);
        let completed = match f.s.dispatch_one().unwrap() {
            DispatchResult::Completed(j) => j,
            other => panic!("expected applied completion: {other:?}"),
        };
        assert!(matches!(completed.state(), JobState::Succeeded));
        assert_eq!(completed.finished_at(), Some(TimestampMillis(10)));
        assert_eq!(f.store.get(j.id()).unwrap(), Some(*completed));
        assert_eq!(f.clock.try_now(), Err(ClockUnavailable));
        assert!(f.s.retained_clock_outcomes().unwrap().is_empty());
        assert_eq!(f.s.dispatch_one(), Err(SchedulerError::ClockUnavailable));
        assert_eq!(f.handler.calls.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn clock_failure_before_cancel_preserves_record_and_cannot_be_reset() {
        let f = fixture(0, None);
        let j = queued(&f);
        f.clock.unavailable.store(true, Ordering::SeqCst);
        assert_eq!(
            f.s.cancel(&caller("user.alice"), j.id()),
            Err(SchedulerError::ClockUnavailable)
        );
        assert_eq!(f.store.get(j.id()).unwrap(), Some(j));
        f.clock.unavailable.store(false, Ordering::SeqCst);
        assert_eq!(f.s.dispatch_one(), Err(SchedulerError::ClockUnavailable));
    }

    #[test]
    fn clock_failure_is_signalled_to_active_handler_on_cancel_pause_and_timeout_paths() {
        verify_active_handler_receives_clock_failure(ActiveClockFailurePath::Cancel);
        verify_active_handler_receives_clock_failure(ActiveClockFailurePath::Pause);
        verify_active_handler_receives_clock_failure(ActiveClockFailurePath::TimeoutRefresh);
    }

    #[test]
    fn unauthorized_cancel_does_not_signal_active_control_or_sample_clock() {
        let store = Arc::new(InMemoryJobStore::new());
        let clock = Arc::new(FaultClock::default());
        let events = Arc::new(BoundedJobEventRecorder::new(32));
        let providers = Arc::new(JobProviderRegistry::new());
        let authority = Arc::new(TestAuthority::default());
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let token = Arc::new(Mutex::new(None));
        let handler = Arc::new(BlockingHandler {
            reference: handler_ref("unauthorized-clock-failure"),
            entered: entered.clone(),
            release: release.clone(),
            token: token.clone(),
            supports_pause: false,
        });
        providers.register(handler.clone()).unwrap();
        let scheduler = Scheduler::new_with_fallible_clock(
            store.clone(),
            clock.clone(),
            authority,
            events,
            providers,
            SchedulerConfig::default(),
        )
        .unwrap();
        let owner = caller("user.alice");
        let record = scheduler
            .enqueue(&owner, JobRequest::new(handler.handler_ref()))
            .unwrap()
            .record;
        let dispatch_scheduler = scheduler.clone();
        let dispatch = thread::spawn(move || dispatch_scheduler.dispatch_one());
        entered.wait();

        clock.reads.store(0, Ordering::SeqCst);
        clock.unavailable.store(true, Ordering::SeqCst);
        let denied = scheduler.cancel(&caller("user.mallory"), record.id());
        let clock_reads = clock.reads.load(Ordering::SeqCst);
        let observed_reason = token.lock().unwrap().as_ref().unwrap().reason();
        release.wait();
        let dispatch_result = dispatch.join().unwrap();

        assert!(matches!(denied, Err(SchedulerError::Authorization(_))));
        assert_eq!(clock_reads, 0);
        assert_eq!(observed_reason, None);
        assert_eq!(dispatch_result, Err(SchedulerError::ClockUnavailable));
        assert!(matches!(
            store.get(record.id()).unwrap().unwrap().state(),
            JobState::Running
        ));
    }

    #[test]
    fn startup_recovery_uses_one_clock_sample_for_all_records() {
        let f = fixture(0, None);
        let first = queued(&f);
        let second = queued(&f);
        f.store.seed_state_for_test(first.id(), JobState::Running);
        f.store.seed_state_for_test(second.id(), JobState::Running);
        f.clock.reads.store(0, Ordering::SeqCst);
        f.clock.fail_on_read.store(2, Ordering::SeqCst);

        let recovery = f.s.recover_startup();
        let first_after = f.store.get(first.id()).unwrap().unwrap();
        let second_after = f.store.get(second.id()).unwrap().unwrap();
        assert!(
            recovery.is_ok(),
            "recovery returned {recovery:?} after {} clock reads; first={:?}; second={:?}",
            f.clock.reads.load(Ordering::SeqCst),
            first_after.state(),
            second_after.state()
        );
        let recovered = recovery.unwrap();

        assert_eq!(recovered.len(), 2);
        assert_eq!(f.clock.reads.load(Ordering::SeqCst), 1);
        assert!(matches!(first_after.state(), JobState::RecoveryRequired(_)));
        assert!(matches!(
            second_after.state(),
            JobState::RecoveryRequired(_)
        ));
    }

    #[test]
    fn startup_recovery_clock_error_before_first_sample_mutates_no_record() {
        let f = fixture(0, None);
        let first = queued(&f);
        let second = queued(&f);
        f.store.seed_state_for_test(first.id(), JobState::Running);
        f.store.seed_state_for_test(second.id(), JobState::Running);
        let before_first = f.store.get(first.id()).unwrap().unwrap();
        let before_second = f.store.get(second.id()).unwrap().unwrap();
        f.clock.reads.store(0, Ordering::SeqCst);
        f.clock.fail_on_read.store(1, Ordering::SeqCst);

        assert_eq!(f.s.recover_startup(), Err(SchedulerError::ClockUnavailable));

        assert_eq!(f.clock.reads.load(Ordering::SeqCst), 1);
        assert_eq!(f.store.get(first.id()).unwrap(), Some(before_first));
        assert_eq!(f.store.get(second.id()).unwrap(), Some(before_second));
    }

    #[test]
    fn startup_recovery_list_read_failure_leaves_running_records_unchanged() {
        let f = fixture(0, None);
        let first = queued(&f);
        let second = queued(&f);
        f.store.seed_state_for_test(first.id(), JobState::Running);
        f.store.seed_state_for_test(second.id(), JobState::Running);
        let before_first = f.store.get(first.id()).unwrap().unwrap();
        let before_second = f.store.get(second.id()).unwrap().unwrap();
        f.store.fail_reads_for_test(true);

        let recovery = f.s.recover_startup();

        f.store.fail_reads_for_test(false);
        assert_eq!(
            recovery,
            Err(SchedulerError::Store(JobStoreError::Unavailable))
        );
        assert_eq!(f.store.get(first.id()).unwrap(), Some(before_first));
        assert_eq!(f.store.get(second.id()).unwrap(), Some(before_second));
        assert_eq!(f.handler.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    // Recovery commits one record at a time; a later failure is resumed from
    // the remaining Running records and does not replay an already settled job.
    fn startup_recovery_retries_after_later_record_write_failure_without_execution() {
        let inner_store = Arc::new(InMemoryJobStore::new());
        let store = Arc::new(FailOnNthRecoveryWriteStore {
            inner: inner_store.clone(),
            compare_and_swap_countdown: AtomicUsize::new(0),
        });
        let clock = Arc::new(FaultClock::default());
        let events = Arc::new(BoundedJobEventRecorder::new(32));
        let providers = Arc::new(JobProviderRegistry::new());
        let handler = Arc::new(Probe {
            clock: clock.clone(),
            reference: handler_ref("recovery-write-failure"),
            mode: 0,
            outcome: HandlerOutcome::Succeeded,
            calls: AtomicUsize::new(0),
            progress: Mutex::new(Vec::new()),
        });
        providers.register(handler.clone()).unwrap();
        let scheduler = Scheduler::new_with_fallible_clock(
            store.clone(),
            clock,
            Arc::new(TestAuthority::default()),
            events.clone(),
            providers,
            SchedulerConfig::new(2, 2, 8).unwrap(),
        )
        .unwrap();
        let owner = caller("user.alice");
        let first = scheduler
            .enqueue(&owner, JobRequest::new(handler.handler_ref()))
            .unwrap()
            .record;
        let second = scheduler
            .enqueue(&owner, JobRequest::new(handler.handler_ref()))
            .unwrap()
            .record;
        inner_store.seed_state_for_test(first.id(), JobState::Running);
        inner_store.seed_state_for_test(second.id(), JobState::Running);
        let ordered = inner_store.list().unwrap();
        let committed_id = ordered[0].id().clone();
        let pending_id = ordered[1].id().clone();
        store.fail_on_nth_compare_and_swap(2);

        assert_eq!(
            scheduler.recover_startup(),
            Err(SchedulerError::Store(JobStoreError::Unavailable))
        );
        assert!(matches!(
            inner_store.get(&committed_id).unwrap().unwrap().state(),
            JobState::RecoveryRequired(_)
        ));
        assert!(matches!(
            inner_store.get(&pending_id).unwrap().unwrap().state(),
            JobState::Running
        ));
        assert_eq!(handler.calls.load(Ordering::SeqCst), 0);

        let first_recovery_events = events
            .snapshot()
            .into_iter()
            .filter(|event| event.code() == JobEventCode::RecoveryRequired)
            .map(|event| event.job_id().cloned().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(first_recovery_events, vec![committed_id.clone()]);

        let retried = scheduler.recover_startup().unwrap();
        assert_eq!(retried.len(), 1);
        assert_eq!(retried[0].id(), &pending_id);
        for id in [&committed_id, &pending_id] {
            assert!(matches!(
                inner_store.get(id).unwrap().unwrap().state(),
                JobState::RecoveryRequired(_)
            ));
        }
        assert_eq!(handler.calls.load(Ordering::SeqCst), 0);
        let recovery_events = events
            .snapshot()
            .into_iter()
            .filter(|event| event.code() == JobEventCode::RecoveryRequired)
            .map(|event| event.job_id().cloned().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(recovery_events, vec![committed_id, pending_id]);
    }

    fn completion_fault_fixture(
        fault: usize,
        outcome: HandlerOutcome,
    ) -> (
        Scheduler,
        Arc<CompletionFaultStore>,
        Arc<InMemoryJobStore>,
        Arc<Probe>,
        Arc<JobProviderRegistry>,
        JobRecord,
    ) {
        let inner = Arc::new(InMemoryJobStore::new());
        let store = Arc::new(CompletionFaultStore {
            inner: inner.clone(),
            compare_and_swap_fault: AtomicUsize::new(fault),
        });
        let clock = Arc::new(FaultClock::default());
        let events = Arc::new(BoundedJobEventRecorder::new(32));
        let providers = Arc::new(JobProviderRegistry::new());
        let handler = Arc::new(Probe {
            clock: clock.clone(),
            reference: handler_ref("completion-commit-unknown"),
            mode: 0,
            outcome,
            calls: AtomicUsize::new(0),
            progress: Mutex::new(Vec::new()),
        });
        providers.register(handler.clone()).unwrap();
        let scheduler = Scheduler::new_with_fallible_clock(
            store.clone(),
            clock,
            Arc::new(TestAuthority::default()),
            events,
            providers.clone(),
            SchedulerConfig::default(),
        )
        .unwrap();
        let record = scheduler
            .enqueue(
                &caller("user.alice"),
                JobRequest::new(handler.handler_ref()),
            )
            .unwrap()
            .record;
        (scheduler, store, inner, handler, providers, record)
    }

    #[test]
    fn completion_commit_failure_before_save_recovers_without_reexecution() {
        let (scheduler, store, inner, handler, providers, record) =
            completion_fault_fixture(1, HandlerOutcome::Succeeded);

        assert_eq!(
            scheduler.dispatch_one(),
            Err(SchedulerError::Store(JobStoreError::Unavailable))
        );
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            inner.get(record.id()).unwrap().unwrap().state(),
            JobState::Running
        ));

        // Reuse the persisted reference store while constructing a new runtime
        // instance. This exercises recovery policy, not physical reboot I/O.
        let restarted = Scheduler::new_with_fallible_clock(
            store,
            Arc::new(FaultClock::default()),
            Arc::new(TestAuthority::default()),
            Arc::new(BoundedJobEventRecorder::new(32)),
            providers,
            SchedulerConfig::default(),
        )
        .unwrap();
        let recovered = restarted.recover_startup().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].id(), record.id());
        assert!(matches!(
            inner.get(record.id()).unwrap().unwrap().state(),
            JobState::RecoveryRequired(_)
        ));
        assert_eq!(restarted.dispatch_one().unwrap(), DispatchResult::Idle);
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn committed_completion_with_unknown_reply_is_not_reexecuted_after_restart() {
        let (scheduler, store, inner, handler, providers, record) =
            completion_fault_fixture(2, HandlerOutcome::Succeeded);

        assert_eq!(
            scheduler.dispatch_one(),
            Err(SchedulerError::Store(JobStoreError::Unavailable))
        );
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            inner.get(record.id()).unwrap().unwrap().state(),
            JobState::Succeeded
        ));

        // Reuse the committed reference-store snapshot as the restart input.
        let restarted = Scheduler::new_with_fallible_clock(
            store,
            Arc::new(FaultClock::default()),
            Arc::new(TestAuthority::default()),
            Arc::new(BoundedJobEventRecorder::new(32)),
            providers,
            SchedulerConfig::default(),
        )
        .unwrap();
        assert!(restarted.recover_startup().unwrap().is_empty());
        assert_eq!(restarted.dispatch_one().unwrap(), DispatchResult::Idle);
        assert!(matches!(
            inner.get(record.id()).unwrap().unwrap().state(),
            JobState::Succeeded
        ));
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn unknown_effect_outcome_with_completion_write_failure_recovers_without_reexecution() {
        let (scheduler, store, inner, handler, providers, record) =
            completion_fault_fixture(1, HandlerOutcome::OutcomeUnknown(JobFailureCode::Transient));

        assert_eq!(
            scheduler.dispatch_one(),
            Err(SchedulerError::Store(JobStoreError::Unavailable))
        );
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            inner.get(record.id()).unwrap().unwrap().state(),
            JobState::Running
        ));

        // External-effect handlers use NonIdempotent recovery. With the
        // completion write unavailable, startup must keep the claim fenced.
        let restarted = Scheduler::new_with_fallible_clock(
            store,
            Arc::new(FaultClock::default()),
            Arc::new(TestAuthority::default()),
            Arc::new(BoundedJobEventRecorder::new(32)),
            providers,
            SchedulerConfig::default(),
        )
        .unwrap();
        let recovered = restarted.recover_startup().unwrap();
        assert_eq!(recovered.len(), 1);
        assert!(matches!(
            inner.get(record.id()).unwrap().unwrap().state(),
            JobState::RecoveryRequired(_)
        ));
        assert_eq!(restarted.dispatch_one().unwrap(), DispatchResult::Idle);
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn committed_unknown_effect_outcome_is_not_reexecuted_after_unknown_reply() {
        let (scheduler, store, inner, handler, providers, record) =
            completion_fault_fixture(2, HandlerOutcome::OutcomeUnknown(JobFailureCode::Transient));

        assert_eq!(
            scheduler.dispatch_one(),
            Err(SchedulerError::Store(JobStoreError::Unavailable))
        );
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            inner.get(record.id()).unwrap().unwrap().state(),
            &JobState::RecoveryRequired(JobFailureCode::Transient)
        );

        let restarted = Scheduler::new_with_fallible_clock(
            store,
            Arc::new(FaultClock::default()),
            Arc::new(TestAuthority::default()),
            Arc::new(BoundedJobEventRecorder::new(32)),
            providers,
            SchedulerConfig::default(),
        )
        .unwrap();
        assert!(restarted.recover_startup().unwrap().is_empty());
        assert_eq!(restarted.dispatch_one().unwrap(), DispatchResult::Idle);
        assert_eq!(
            inner.get(record.id()).unwrap().unwrap().state(),
            &JobState::RecoveryRequired(JobFailureCode::Transient)
        );
        assert_eq!(handler.calls.load(Ordering::SeqCst), 1);
    }

    struct StopUnknownFaultStore {
        inner: Arc<InMemoryJobStore>,
        armed: AtomicBool,
        injected: AtomicBool,
    }

    impl JobStore for StopUnknownFaultStore {
        fn insert(
            &self,
            record: JobRecord,
            dedupe: Option<(DedupeScope, TimestampMillis)>,
            now: TimestampMillis,
            queue_capacity: usize,
            record_capacity: usize,
        ) -> Result<InsertOutcome, JobStoreError> {
            self.inner
                .insert(record, dedupe, now, queue_capacity, record_capacity)
        }

        fn get(&self, id: &JobId) -> Result<Option<JobRecord>, JobStoreError> {
            self.inner.get(id)
        }

        fn list(&self) -> Result<Vec<JobRecord>, JobStoreError> {
            self.inner.list()
        }

        fn compare_and_swap(
            &self,
            id: &JobId,
            expected_revision: u64,
            replacement: JobRecord,
        ) -> Result<JobRecord, JobStoreError> {
            let stopped = self
                .inner
                .get(id)?
                .is_some_and(|saved| matches!(saved.state(), JobState::CancelRequested));
            let unknown_completion = matches!(replacement.state(), JobState::RecoveryRequired(_));
            if stopped && unknown_completion && self.armed.swap(false, Ordering::SeqCst) {
                self.injected.store(true, Ordering::SeqCst);
                return Err(JobStoreError::Unavailable);
            }
            self.inner
                .compare_and_swap(id, expected_revision, replacement)
        }

        fn claim(
            &self,
            id: &JobId,
            expected_revision: u64,
            now: TimestampMillis,
            global_limit: usize,
            per_owner_limit: usize,
        ) -> Result<Option<JobRecord>, JobStoreError> {
            self.inner
                .claim(id, expected_revision, now, global_limit, per_owner_limit)
        }
    }

    fn assert_stopped_unknown_completion_recovers_as_required(timeout: bool) {
        let inner = Arc::new(InMemoryJobStore::new());
        let store = Arc::new(StopUnknownFaultStore {
            inner: inner.clone(),
            armed: AtomicBool::new(true),
            injected: AtomicBool::new(false),
        });
        let clock = Arc::new(ManualClock::new(TimestampMillis(0)));
        let events = Arc::new(BoundedJobEventRecorder::new(32));
        let providers = Arc::new(JobProviderRegistry::new());
        let started = Arc::new(Barrier::new(2));
        let handler = Arc::new(TestHandler {
            reference: handler_ref("unknown-stop-commit-failure"),
            mode: HandlerMode::OutcomeUnknownAfterStop(started.clone()),
            recovery: RecoveryPolicy::NonIdempotent,
            supports_pause: false,
            checkpoint_valid: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        });
        providers.register(handler.clone()).unwrap();
        let scheduler = Scheduler::new(
            store.clone(),
            clock.clone(),
            Arc::new(TestAuthority::default()),
            events,
            providers.clone(),
            SchedulerConfig::default(),
        )
        .unwrap();
        let caller = caller("user.alice");
        let mut request = JobRequest::new(handler.handler_ref());
        if timeout {
            request = request
                .with_constraints(JobConstraints::default().with_timeout(DurationMillis(50)));
        }
        let record = scheduler.enqueue(&caller, request).unwrap().record;
        let dispatcher = scheduler.clone();
        let worker = thread::spawn(move || dispatcher.dispatch_one());

        started.wait();
        if timeout {
            clock.advance(DurationMillis(50));
            assert_eq!(scheduler.dispatch_one().unwrap(), DispatchResult::Idle);
        } else {
            assert_eq!(
                scheduler.cancel(&caller, record.id()).unwrap().state(),
                &JobState::CancelRequested
            );
        }

        assert_eq!(
            worker.join().unwrap(),
            Err(SchedulerError::Store(JobStoreError::Unavailable))
        );
        assert!(store.injected.load(Ordering::SeqCst));
        assert_eq!(handler.call_count(), 1);
        let committed = inner.get(record.id()).unwrap().unwrap();
        assert_eq!(committed.state(), &JobState::CancelRequested);

        // Reopen from the exact last committed row, including its stop reason.
        let bytes = encode_job_record(&committed).unwrap();
        let snapshot = decode_job_record(&bytes).unwrap();
        let reopened = Arc::new(InMemoryJobStore::new());
        assert!(matches!(
            reopened
                .insert(snapshot, None, clock.now(), 8, 128)
                .unwrap(),
            InsertOutcome::Inserted(_)
        ));
        let restarted = Scheduler::new(
            reopened.clone(),
            clock,
            Arc::new(TestAuthority::default()),
            Arc::new(BoundedJobEventRecorder::new(32)),
            providers,
            SchedulerConfig::default(),
        )
        .unwrap();
        let recovered = restarted.recover_startup().unwrap();
        assert_eq!(recovered.len(), 1);
        assert!(matches!(
            reopened.get(record.id()).unwrap().unwrap().state(),
            JobState::RecoveryRequired(JobFailureCode::Permanent)
        ));
        assert_eq!(restarted.dispatch_one().unwrap(), DispatchResult::Idle);
        assert_eq!(handler.call_count(), 1);
    }

    #[test]
    fn cancel_then_unknown_completion_commit_failure_recovers_without_reexecution() {
        assert_stopped_unknown_completion_recovers_as_required(false);
    }

    #[test]
    fn timeout_then_unknown_completion_commit_failure_recovers_without_reexecution() {
        assert_stopped_unknown_completion_recovers_as_required(true);
    }
}
