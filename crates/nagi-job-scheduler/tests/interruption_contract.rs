//! Independent public-API orchestration tests. Every provider and authority is fake.
use nagi_job_scheduler::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

struct FakeAuthority;
impl JobAuthorization for FakeAuthority {
    fn owner_for(&self, caller: &CallerRef) -> Result<JobOwner, AuthorizationError> {
        Ok(JobOwner::from_authenticated_adapter(
            caller.principal().clone(),
            None,
        ))
    }
    fn authorize_user(
        &self,
        caller: &CallerRef,
        owner: &JobOwner,
        _: JobAction,
        _: Option<&JobId>,
    ) -> Result<(), AuthorizationError> {
        if caller.principal() == owner.principal() {
            Ok(())
        } else {
            Err(AuthorizationError::denied())
        }
    }
    fn authorize_priority(
        &self,
        _: &CallerRef,
        _: &JobOwner,
        _: &HandlerRef,
        priority: JobPriority,
    ) -> Result<(), AuthorizationError> {
        if priority == JobPriority::Normal {
            Ok(())
        } else {
            Err(AuthorizationError::denied())
        }
    }
    fn authorize_handler(
        &self,
        _: &JobOwner,
        handler: &HandlerRef,
    ) -> Result<(), AuthorizationError> {
        if handler == &reference() {
            Ok(())
        } else {
            Err(AuthorizationError::denied())
        }
    }
}
fn reference() -> HandlerRef {
    HandlerRef::new(
        OpaqueRef::new("fake").unwrap(),
        OpaqueRef::new("gated").unwrap(),
        1,
    )
    .unwrap()
}
struct FakeProvider {
    started: mpsc::Sender<CancellationToken>,
    outcomes: Mutex<VecDeque<mpsc::Receiver<HandlerOutcome>>>,
}
impl JobHandler for FakeProvider {
    fn handler_ref(&self) -> HandlerRef {
        reference()
    }
    fn recovery_policy(&self, _: &JobRecord) -> RecoveryPolicy {
        RecoveryPolicy::Idempotent
    }
    fn execute(&self, context: &mut HandlerContext) -> HandlerOutcome {
        let outcome = self
            .outcomes
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected execution");
        self.started
            .send(context.cancellation_token().child_token())
            .unwrap();
        outcome.recv().unwrap()
    }
}
struct Harness {
    scheduler: Arc<Scheduler>,
    clock: Arc<ManualClock>,
    events: Arc<BoundedJobEventRecorder>,
    caller: CallerRef,
    id: JobId,
    started: mpsc::Receiver<CancellationToken>,
    releases: Vec<mpsc::Sender<HandlerOutcome>>,
}
impl Harness {
    fn new() -> Self {
        Self::with_authority(Arc::new(FakeAuthority))
    }
    fn with_authority(authority: Arc<dyn JobAuthorization>) -> Self {
        let clock = Arc::new(ManualClock::default());
        let events = Arc::new(BoundedJobEventRecorder::new(64));
        let providers = Arc::new(JobProviderRegistry::new());
        let (started_tx, started) = mpsc::channel();
        let (first_tx, first) = mpsc::channel();
        let (second_tx, second) = mpsc::channel();
        providers
            .register(Arc::new(FakeProvider {
                started: started_tx,
                outcomes: Mutex::new(VecDeque::from([first, second])),
            }))
            .unwrap();
        let scheduler = Arc::new(
            Scheduler::new(
                Arc::new(InMemoryJobStore::new()),
                clock.clone(),
                authority,
                events.clone(),
                providers,
                SchedulerConfig::default(),
            )
            .unwrap(),
        );
        let caller =
            CallerRef::from_authenticated_adapter(PrincipalRef::new("fake.owner").unwrap(), None);
        let retry = RetryPolicy::new(
            3,
            DurationMillis(10),
            DurationMillis(20),
            2000,
            [JobFailureCode::Transient],
            RetryTerminalAction::Failed,
        )
        .unwrap();
        let id = scheduler
            .enqueue(
                &caller,
                JobRequest::new(reference()).with_retry_policy(retry),
            )
            .unwrap()
            .record
            .id()
            .clone();
        Self {
            scheduler,
            clock,
            events,
            caller,
            id,
            started,
            releases: vec![first_tx, second_tx],
        }
    }
    fn spawn(&self) -> thread::JoinHandle<Result<DispatchResult, SchedulerError>> {
        let scheduler = self.scheduler.clone();
        thread::spawn(move || scheduler.dispatch_one())
    }
    fn record(&self) -> JobRecord {
        self.scheduler.inspect(&self.caller, &self.id).unwrap()
    }
    fn cancel(&self) -> JobRecord {
        self.scheduler.cancel(&self.caller, &self.id).unwrap()
    }
}

#[test]
fn stale_results_cannot_complete_a_recovered_queued_record() {
    for outcome in [
        HandlerOutcome::Succeeded,
        HandlerOutcome::Failed(JobFailureCode::Transient),
    ] {
        let h = Harness::new();
        let old = h.spawn();
        h.started.recv().unwrap();
        h.scheduler.recover_startup().unwrap();
        let before = h.record();
        let events = h.events.snapshot();
        assert_eq!(before.state(), &JobState::Queued);
        h.releases[0].send(outcome).unwrap();
        let result = old.join().unwrap();
        assert_eq!(h.record(), before);
        assert_eq!(h.events.snapshot(), events);
        assert_eq!(result, Ok(DispatchResult::Completed(Box::new(before))));
    }
}

#[test]
fn stale_results_preserve_new_attempt_and_its_cancellation_control() {
    for outcome in [
        HandlerOutcome::Succeeded,
        HandlerOutcome::Failed(JobFailureCode::Transient),
    ] {
        let h = Harness::new();
        let old = h.spawn();
        h.started.recv().unwrap();
        h.scheduler.recover_startup().unwrap();
        let current = h.spawn();
        let token = h.started.recv().unwrap();
        let before = h.record();
        let events = h.events.snapshot();
        assert_eq!(before.attempt_count(), 2);
        h.releases[0].send(outcome).unwrap();
        let old_result = old.join().unwrap();
        let after = h.record();
        let after_events = h.events.snapshot();
        let cancelled = h.cancel();
        let reason = token.reason();
        // Release and join before assertions so a regression cannot strand a worker.
        h.releases[1].send(HandlerOutcome::Cancelled).unwrap();
        let result = current.join().unwrap();
        assert_eq!(after, before);
        assert_eq!(after_events, events);
        assert_eq!(old_result, Ok(DispatchResult::Completed(Box::new(before))));
        assert_eq!(cancelled.state(), &JobState::CancelRequested);
        assert_eq!(reason, Some(CancellationReason::Requested));
        assert!(result.is_ok());
        assert_eq!(h.record().state(), &JobState::Cancelled);
    }
}

#[test]
fn late_duplicate_result_cannot_rewrite_new_attempt_terminal_record() {
    for outcome in [
        HandlerOutcome::Succeeded,
        HandlerOutcome::Failed(JobFailureCode::Transient),
    ] {
        let h = Harness::new();
        let old = h.spawn();
        h.started.recv().unwrap();
        h.scheduler.recover_startup().unwrap();
        let current = h.spawn();
        h.started.recv().unwrap();
        h.releases[1].send(HandlerOutcome::Succeeded).unwrap();
        current.join().unwrap().unwrap();
        let before = h.record();
        let events = h.events.snapshot();
        h.releases[0].send(outcome).unwrap();
        let result = old.join().unwrap();
        assert_eq!(h.record(), before);
        assert_eq!(h.events.snapshot(), events);
        assert_eq!(result, Ok(DispatchResult::Completed(Box::new(before))));
        assert_eq!(h.cancel(), h.record());
    }
}

#[test]
fn repeated_cancel_during_backoff_prevents_retry_even_at_due_time() {
    let h = Harness::new();
    let worker = h.spawn();
    h.started.recv().unwrap();
    h.releases[0]
        .send(HandlerOutcome::Failed(JobFailureCode::Transient))
        .unwrap();
    worker.join().unwrap().unwrap();
    assert_eq!(h.record().state(), &JobState::RetryWait);
    let cancelled = h.cancel();
    let events = h.events.snapshot();
    assert_eq!(h.cancel(), cancelled);
    h.clock.advance(DurationMillis(10));
    assert_eq!(h.scheduler.dispatch_one().unwrap(), DispatchResult::Idle);
    h.clock.advance(DurationMillis(100));
    assert_eq!(h.scheduler.dispatch_one().unwrap(), DispatchResult::Idle);
    assert_eq!(h.record(), cancelled);
    assert_eq!(h.record().attempt_count(), 1);
    assert_eq!(h.events.snapshot(), events);
    assert!(h.started.try_recv().is_err());
}

#[test]
fn cooperative_cancel_preserves_success_without_claiming_rollback() {
    let h = Harness::new();
    let worker = h.spawn();
    let token = h.started.recv().unwrap();
    let requested = h.cancel();
    assert_eq!(h.cancel(), requested);
    assert_eq!(token.reason(), Some(CancellationReason::Requested));
    h.releases[0].send(HandlerOutcome::Succeeded).unwrap();
    worker.join().unwrap().unwrap();
    let completed = h.record();
    assert_eq!(completed.state(), &JobState::Succeeded);
    assert_eq!(h.cancel(), completed);
    assert_eq!(h.scheduler.dispatch_one().unwrap(), DispatchResult::Idle);
}

// Only the first handler authorization is suspended; recovery and replacement
// authorization remain independent, explicit fake decisions.
struct DeferredDenialAuthority {
    calls: AtomicUsize,
    waiting: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}
impl JobAuthorization for DeferredDenialAuthority {
    fn owner_for(&self, caller: &CallerRef) -> Result<JobOwner, AuthorizationError> {
        FakeAuthority.owner_for(caller)
    }
    fn authorize_user(
        &self,
        caller: &CallerRef,
        owner: &JobOwner,
        action: JobAction,
        id: Option<&JobId>,
    ) -> Result<(), AuthorizationError> {
        FakeAuthority.authorize_user(caller, owner, action, id)
    }
    fn authorize_priority(
        &self,
        caller: &CallerRef,
        owner: &JobOwner,
        handler: &HandlerRef,
        priority: JobPriority,
    ) -> Result<(), AuthorizationError> {
        FakeAuthority.authorize_priority(caller, owner, handler, priority)
    }
    fn authorize_handler(
        &self,
        owner: &JobOwner,
        handler: &HandlerRef,
    ) -> Result<(), AuthorizationError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            self.waiting.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
            Err(AuthorizationError::denied())
        } else {
            FakeAuthority.authorize_handler(owner, handler)
        }
    }
}

#[test]
fn stale_authorization_denial_preserves_record_events_and_new_cancel_control() {
    let (waiting_tx, waiting) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let h = Harness::with_authority(Arc::new(DeferredDenialAuthority {
        calls: AtomicUsize::new(0),
        waiting: waiting_tx,
        release: Mutex::new(release_rx),
    }));
    let old = h.spawn();
    waiting.recv().unwrap();
    h.scheduler.recover_startup().unwrap();
    let current = h.spawn();
    let token = h.started.recv().unwrap();
    let before = h.record();
    let events = h.events.snapshot();
    assert_eq!(before.attempt_count(), 2);
    assert_eq!(before.state(), &JobState::Running);
    release.send(()).unwrap();
    let old_result = old.join().unwrap();
    let after = h.record();
    let after_events = h.events.snapshot();
    let cancelled = h.cancel();
    let reason = token.reason();
    // First authorization never called the provider, so replacement uses slot 0.
    h.releases[0].send(HandlerOutcome::Cancelled).unwrap();
    let result = current.join().unwrap();
    assert_eq!(after, before);
    assert_eq!(after_events, events);
    assert_eq!(old_result, Ok(DispatchResult::Completed(Box::new(before))));
    assert_eq!(cancelled.state(), &JobState::CancelRequested);
    assert_eq!(reason, Some(CancellationReason::Requested));
    assert!(result.is_ok());
    assert_eq!(h.record().state(), &JobState::Cancelled);
    assert!(h.started.try_recv().is_err());
}
