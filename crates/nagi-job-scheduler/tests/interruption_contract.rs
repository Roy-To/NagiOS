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

#[derive(Clone, Copy)]
enum ContextCommand {
    Publish,
    Finish,
}
struct ContextProbe {
    started: mpsc::Sender<CancellationToken>,
    commands: Mutex<VecDeque<mpsc::Receiver<ContextCommand>>>,
    results: mpsc::Sender<(Result<(), ProgressError>, Result<(), ProgressError>)>,
}
impl JobHandler for ContextProbe {
    fn handler_ref(&self) -> HandlerRef {
        reference()
    }
    fn recovery_policy(&self, _: &JobRecord) -> RecoveryPolicy {
        RecoveryPolicy::Idempotent
    }
    fn execute(&self, context: &mut HandlerContext) -> HandlerOutcome {
        let commands = self.commands.lock().unwrap().pop_front().unwrap();
        self.started
            .send(context.cancellation_token().child_token())
            .unwrap();
        while let ContextCommand::Publish = commands.recv().unwrap() {
            let summary =
                ProgressSummary::new(ProgressCode::new("fake-progress").unwrap(), 1, Some(2))
                    .unwrap();
            let progress = context.report_progress(summary);
            let checkpoint =
                context.publish_checkpoint(CheckpointRef::new("fake-checkpoint").unwrap());
            self.results.send((progress, checkpoint)).unwrap();
        }
        HandlerOutcome::Succeeded
    }
}

// Covers recovery before replacement, during replacement, and after replacement
// completion. All provider calls are in-memory, controlled by channels.
#[test]
fn stale_context_updates_do_not_mutate_records_events_or_replacement_rate_budget() {
    for replacement_phase in 0..3 {
        let clock = Arc::new(ManualClock::default());
        let events = Arc::new(BoundedJobEventRecorder::new(64));
        let registry = Arc::new(JobProviderRegistry::new());
        let (started_tx, started) = mpsc::channel();
        let (old_tx, old_commands) = mpsc::channel();
        let (new_tx, new_commands) = mpsc::channel();
        let (results_tx, results) = mpsc::channel();
        registry
            .register(Arc::new(ContextProbe {
                started: started_tx,
                commands: Mutex::new(VecDeque::from([old_commands, new_commands])),
                results: results_tx,
            }))
            .unwrap();
        let scheduler = Arc::new(
            Scheduler::new(
                Arc::new(InMemoryJobStore::new()),
                clock,
                Arc::new(FakeAuthority),
                events.clone(),
                registry,
                SchedulerConfig::default(),
            )
            .unwrap(),
        );
        let caller =
            CallerRef::from_authenticated_adapter(PrincipalRef::new("fake.owner").unwrap(), None);
        let id = scheduler
            .enqueue(&caller, JobRequest::new(reference()))
            .unwrap()
            .record
            .id()
            .clone();
        let old_scheduler = scheduler.clone();
        let old = thread::spawn(move || old_scheduler.dispatch_one());
        started.recv().unwrap();
        scheduler.recover_startup().unwrap();
        let mut current = None;
        let mut current_token = None;
        if replacement_phase > 0 {
            let new_scheduler = scheduler.clone();
            current = Some(thread::spawn(move || new_scheduler.dispatch_one()));
            current_token = Some(started.recv().unwrap());
        }
        if replacement_phase == 2 {
            new_tx.send(ContextCommand::Finish).unwrap();
            current.take().unwrap().join().unwrap().unwrap();
        }
        let before = scheduler.inspect(&caller, &id).unwrap();
        let before_events = events.snapshot();
        old_tx.send(ContextCommand::Publish).unwrap();
        let stale = results.recv().unwrap();
        let after = scheduler.inspect(&caller, &id).unwrap();
        let after_events = events.snapshot();
        let mut fresh = None;
        let mut cancel_reason = None;
        if replacement_phase == 1 {
            new_tx.send(ContextCommand::Publish).unwrap();
            fresh = Some(results.recv().unwrap());
            scheduler.cancel(&caller, &id).unwrap();
            cancel_reason = current_token.as_ref().unwrap().reason();
            new_tx.send(ContextCommand::Finish).unwrap();
            current.take().unwrap().join().unwrap().unwrap();
        }
        old_tx.send(ContextCommand::Finish).unwrap();
        let old_result = old.join().unwrap();
        // Drain workers before checking negatives, including on unfixed code.
        assert_eq!(
            stale,
            (
                Err(ProgressError::StoreUnavailable),
                Err(ProgressError::StoreUnavailable)
            )
        );
        assert_eq!(after, before);
        assert_eq!(after_events, before_events);
        assert!(old_result.is_ok());
        if replacement_phase == 1 {
            assert_eq!(fresh, Some((Ok(()), Ok(()))));
            assert_eq!(cancel_reason, Some(CancellationReason::Requested));
            let record = scheduler.inspect(&caller, &id).unwrap();
            assert!(record.progress().is_some());
            assert_eq!(
                record.checkpoint_ref(),
                Some(&CheckpointRef::new("fake-checkpoint").unwrap())
            );
        }
    }
}

// Public-API stale callback and recovery interleaving regressions.
enum CallbackCommand {
    Progress,
    Checkpoint,
    Finish,
    FinishPaused,
}
struct CallbackHandle {
    command: mpsc::Sender<CallbackCommand>,
    result: mpsc::Receiver<Result<(), ProgressError>>,
}
struct CallbackProvider {
    started: mpsc::Sender<CallbackHandle>,
}
impl JobHandler for CallbackProvider {
    fn supports_pause(&self) -> bool {
        true
    }
    fn handler_ref(&self) -> HandlerRef {
        reference()
    }
    fn recovery_policy(&self, _: &JobRecord) -> RecoveryPolicy {
        RecoveryPolicy::Idempotent
    }
    fn execute(&self, context: &mut HandlerContext) -> HandlerOutcome {
        let (command, commands) = mpsc::channel();
        let (result, results) = mpsc::channel();
        self.started
            .send(CallbackHandle {
                command,
                result: results,
            })
            .unwrap();
        loop {
            match commands.recv().unwrap() {
                CallbackCommand::Progress => result
                    .send(
                        context.report_progress(
                            ProgressSummary::new(
                                ProgressCode::new("old.progress").unwrap(),
                                1,
                                Some(2),
                            )
                            .unwrap(),
                        ),
                    )
                    .unwrap(),
                CallbackCommand::Checkpoint => result
                    .send(context.publish_checkpoint(CheckpointRef::new("old.checkpoint").unwrap()))
                    .unwrap(),
                CallbackCommand::Finish => return HandlerOutcome::Succeeded,
                CallbackCommand::FinishPaused => return HandlerOutcome::Paused(None),
            }
        }
    }
}
fn callback_harness() -> (
    Arc<Scheduler>,
    Arc<BoundedJobEventRecorder>,
    CallerRef,
    JobId,
    mpsc::Receiver<CallbackHandle>,
) {
    let events = Arc::new(BoundedJobEventRecorder::new(64));
    let providers = Arc::new(JobProviderRegistry::new());
    let (tx, rx) = mpsc::channel();
    providers
        .register(Arc::new(CallbackProvider { started: tx }))
        .unwrap();
    let scheduler = Arc::new(
        Scheduler::new(
            Arc::new(InMemoryJobStore::new()),
            Arc::new(ManualClock::default()),
            Arc::new(FakeAuthority),
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
    (scheduler, events, caller, id, rx)
}
fn stale_callback_check(command: CallbackCommand) {
    let (s, events, caller, id, started) = callback_harness();
    let old = {
        let s = s.clone();
        thread::spawn(move || s.dispatch_one())
    };
    let old_handle = started.recv().unwrap();
    s.recover_startup().unwrap();
    let new = {
        let s = s.clone();
        thread::spawn(move || s.dispatch_one())
    };
    let new_handle = started.recv().unwrap();
    let before = s.inspect(&caller, &id).unwrap();
    let events_before = events.snapshot();
    assert_eq!(before.attempt_count(), 2);
    old_handle.command.send(command).unwrap();
    let result = old_handle.result.recv().unwrap();
    let after = s.inspect(&caller, &id).unwrap();
    let events_after = events.snapshot();
    old_handle.command.send(CallbackCommand::Finish).unwrap();
    let old_result = old.join().unwrap();
    new_handle.command.send(CallbackCommand::Finish).unwrap();
    let new_result = new.join().unwrap();
    assert!(old_result.is_ok() && new_result.is_ok());
    assert_eq!(
        after,
        before,
        "stale callback changed replacement attempt; callback result={result:?}, events changed={}",
        events_before != events_after
    );
    assert_eq!(events_after, events_before);
    assert!(result.is_err(), "stale callback accepted");
}
#[test]
fn stale_progress_cannot_mutate_replacement_attempt_or_events() {
    stale_callback_check(CallbackCommand::Progress);
}
#[test]
fn stale_checkpoint_cannot_mutate_replacement_attempt() {
    stale_callback_check(CallbackCommand::Checkpoint);
}
#[test]
fn rejected_queued_callback_cannot_throttle_replacement_progress() {
    let (s, _, caller, id, started) = callback_harness();
    let old = {
        let s = s.clone();
        thread::spawn(move || s.dispatch_one())
    };
    let old_handle = started.recv().unwrap();
    s.recover_startup().unwrap();
    assert_eq!(s.inspect(&caller, &id).unwrap().state(), &JobState::Queued);
    old_handle.command.send(CallbackCommand::Progress).unwrap();
    let stale_result = old_handle.result.recv().unwrap();
    let new = {
        let s = s.clone();
        thread::spawn(move || s.dispatch_one())
    };
    let new_handle = started.recv().unwrap();
    new_handle.command.send(CallbackCommand::Progress).unwrap();
    let current_result = new_handle.result.recv().unwrap();
    old_handle.command.send(CallbackCommand::Finish).unwrap();
    old.join().unwrap().unwrap();
    new_handle.command.send(CallbackCommand::Finish).unwrap();
    new.join().unwrap().unwrap();
    assert_eq!(stale_result, Err(ProgressError::StoreUnavailable));
    assert_eq!(
        current_result,
        Ok(()),
        "rejected stale callback consumed the replacement attempt rate window"
    );
}

#[test]
fn additional_stale_outcomes_preserve_replacement_record_events_and_token() {
    for outcome in [
        HandlerOutcome::Cancelled,
        HandlerOutcome::Paused(Some(CheckpointRef::new("stale.result.checkpoint").unwrap())),
        HandlerOutcome::Failed(JobFailureCode::Permanent),
        HandlerOutcome::Failed(JobFailureCode::PermissionDenied),
    ] {
        let h = Harness::new();
        let old = h.spawn();
        h.started.recv().unwrap();
        h.scheduler.recover_startup().unwrap();
        let new = h.spawn();
        let token = h.started.recv().unwrap();
        let before = h.record();
        let events = h.events.snapshot();
        h.releases[0].send(outcome).unwrap();
        let result = old.join().unwrap();
        let after = h.record();
        let after_events = h.events.snapshot();
        h.cancel();
        let reason = token.reason();
        h.releases[1].send(HandlerOutcome::Cancelled).unwrap();
        new.join().unwrap().unwrap();
        assert_eq!(
            result,
            Ok(DispatchResult::Completed(Box::new(before.clone())))
        );
        assert_eq!(after, before);
        assert_eq!(after_events, events);
        assert_eq!(reason, Some(CancellationReason::Requested));
    }
}

#[test]
fn pause_resume_retains_checkpoint_and_current_callbacks_work() {
    let (s, events, caller, id, started) = callback_harness();
    let first = {
        let s = s.clone();
        thread::spawn(move || s.dispatch_one())
    };
    let h = started.recv().unwrap();
    h.command.send(CallbackCommand::Checkpoint).unwrap();
    assert_eq!(h.result.recv().unwrap(), Ok(()));
    let requested = s.pause(&caller, &id).unwrap();
    assert_eq!(requested.state(), &JobState::PauseRequested);
    h.command.send(CallbackCommand::FinishPaused).unwrap();
    first.join().unwrap().unwrap();
    let paused = s.inspect(&caller, &id).unwrap();
    assert_eq!(paused.state(), &JobState::Paused);
    assert_eq!(paused.checkpoint_ref().unwrap().as_str(), "old.checkpoint");
    assert_eq!(s.resume(&caller, &id).unwrap().state(), &JobState::Queued);
    let second = {
        let s = s.clone();
        thread::spawn(move || s.dispatch_one())
    };
    let h = started.recv().unwrap();
    h.command.send(CallbackCommand::Progress).unwrap();
    let progress = h.result.recv().unwrap();
    h.command.send(CallbackCommand::Finish).unwrap();
    second.join().unwrap().unwrap();
    assert_eq!(progress, Ok(()));
    let completed = s.inspect(&caller, &id).unwrap();
    assert_eq!(completed.state(), &JobState::Succeeded);
    assert_eq!(completed.attempt_count(), 2);
    assert_eq!(completed.checkpoint_ref(), paused.checkpoint_ref());
    let codes = events
        .snapshot()
        .into_iter()
        .map(|e| e.code())
        .collect::<Vec<_>>();
    assert_eq!(
        codes.iter().filter(|c| **c == JobEventCode::Paused).count(),
        1
    );
    assert_eq!(
        codes
            .iter()
            .filter(|c| **c == JobEventCode::Resumed)
            .count(),
        1
    );
}

// A linearizable store adapter gates one get after the backing store copied its
// snapshot. This models a callback preemption before validating that snapshot.
struct SnapshotGateStore {
    inner: InMemoryJobStore,
    armed: std::sync::atomic::AtomicBool,
    entered: mpsc::Sender<()>,
    release: Mutex<mpsc::Receiver<()>>,
}
impl JobStore for SnapshotGateStore {
    fn insert(
        &self,
        r: JobRecord,
        d: Option<(DedupeScope, TimestampMillis)>,
        n: TimestampMillis,
        q: usize,
        c: usize,
    ) -> Result<InsertOutcome, JobStoreError> {
        self.inner.insert(r, d, n, q, c)
    }
    fn get(&self, id: &JobId) -> Result<Option<JobRecord>, JobStoreError> {
        let snapshot = self.inner.get(id)?;
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.send(()).unwrap();
            self.release.lock().unwrap().recv().unwrap();
        }
        Ok(snapshot)
    }
    fn list(&self) -> Result<Vec<JobRecord>, JobStoreError> {
        self.inner.list()
    }
    fn compare_and_swap(
        &self,
        id: &JobId,
        rev: u64,
        r: JobRecord,
    ) -> Result<JobRecord, JobStoreError> {
        self.inner.compare_and_swap(id, rev, r)
    }
    fn claim(
        &self,
        id: &JobId,
        rev: u64,
        n: TimestampMillis,
        g: usize,
        o: usize,
    ) -> Result<Option<JobRecord>, JobStoreError> {
        self.inner.claim(id, rev, n, g, o)
    }
}
#[test]
fn progress_read_recovery_race_cannot_consume_replacement_rate_budget() {
    let (entered_tx, entered) = mpsc::channel();
    let (release, release_rx) = mpsc::channel();
    let store = Arc::new(SnapshotGateStore {
        inner: InMemoryJobStore::new(),
        armed: std::sync::atomic::AtomicBool::new(false),
        entered: entered_tx,
        release: Mutex::new(release_rx),
    });
    let events = Arc::new(BoundedJobEventRecorder::new(64));
    let providers = Arc::new(JobProviderRegistry::new());
    let (started_tx, started) = mpsc::channel();
    providers
        .register(Arc::new(CallbackProvider {
            started: started_tx,
        }))
        .unwrap();
    let s = Arc::new(
        Scheduler::new(
            store.clone(),
            Arc::new(ManualClock::default()),
            Arc::new(FakeAuthority),
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
    let id = s
        .enqueue(
            &caller,
            JobRequest::new(reference()).with_retry_policy(retry),
        )
        .unwrap()
        .record
        .id()
        .clone();
    let old = {
        let s = s.clone();
        thread::spawn(move || s.dispatch_one())
    };
    let old_h = started.recv().unwrap();
    store.armed.store(true, Ordering::SeqCst);
    old_h.command.send(CallbackCommand::Progress).unwrap();
    entered.recv().unwrap();
    // The old get has copied attempt 1. It overlaps valid recovery and claim 2.
    s.recover_startup().unwrap();
    let new = {
        let s = s.clone();
        thread::spawn(move || s.dispatch_one())
    };
    let new_h = started.recv().unwrap();
    let before = s.inspect(&caller, &id).unwrap();
    let before_events = events.snapshot();
    assert_eq!(before.attempt_count(), 2);
    release.send(()).unwrap();
    let stale = old_h.result.recv().unwrap();
    let after = s.inspect(&caller, &id).unwrap();
    let after_events = events.snapshot();
    new_h.command.send(CallbackCommand::Progress).unwrap();
    let fresh = new_h.result.recv().unwrap();
    old_h.command.send(CallbackCommand::Finish).unwrap();
    old.join().unwrap().unwrap();
    new_h.command.send(CallbackCommand::Finish).unwrap();
    new.join().unwrap().unwrap();
    assert_eq!(stale, Err(ProgressError::StoreUnavailable));
    assert_eq!(after, before);
    assert_eq!(after_events, before_events);
    assert_eq!(
        fresh,
        Ok(()),
        "old callback lost its CAS but reserved attempt 2 progress budget after recovery"
    );
}
