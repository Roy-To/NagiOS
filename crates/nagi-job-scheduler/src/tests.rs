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
        if priority == JobPriority::Urgent && !self.allow_urgent.load(Ordering::SeqCst) {
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
                    None => HandlerOutcome::Succeeded,
                }
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

struct Harness {
    scheduler: Scheduler,
    store: Arc<InMemoryJobStore>,
    clock: Arc<ManualClock>,
    authority: Arc<TestAuthority>,
    providers: Arc<JobProviderRegistry>,
    events: Arc<BoundedJobEventRecorder>,
}

fn harness(config: SchedulerConfig) -> Harness {
    let store = Arc::new(InMemoryJobStore::new());
    let clock = Arc::new(ManualClock::new(TimestampMillis(0)));
    let authority = Arc::new(TestAuthority::default());
    let providers = Arc::new(JobProviderRegistry::new());
    let events = Arc::new(BoundedJobEventRecorder::new(32));
    let scheduler = Scheduler::new(
        store.clone(),
        clock.clone(),
        authority.clone(),
        events.clone(),
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
    assert!(matches!(
        harness
            .scheduler
            .enqueue(&alice, JobRequest::new(handler.clone())),
        Err(SchedulerError::Authorization(_))
    ));
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
