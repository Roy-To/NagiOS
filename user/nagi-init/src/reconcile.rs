//! Task24-side candidate for separating a one-shot exact-child cleanup request
//! from repeatable observation of the existing lifecycle's terminal result.
//!
//! This module deliberately does not define or copy C1's EndOwner types. The
//! production adapter still needs C1's owner to retain a pending reconciliation
//! across polls. `Complete` must come only from the existing ProcessLifecycle
//! terminal ledger after exact reap, descriptor close, record revoke, and lease
//! retirement; `Reaped` alone is not a cleanup receipt.

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactChildKey<ProcessInstance, LaunchRecord> {
    process_instance: ProcessInstance,
    launch_record: LaunchRecord,
}

impl<ProcessInstance, LaunchRecord> ExactChildKey<ProcessInstance, LaunchRecord> {
    /// The caller must obtain this pair from the sole spawn/lifecycle owner:
    /// clone3 pidfd identity plus the matching, owner-held LaunchRecord.
    /// Numeric PID alone is intentionally not representable here.
    pub(crate) fn from_owner(
        process_instance: ProcessInstance,
        launch_record: LaunchRecord,
    ) -> Self {
        Self {
            process_instance,
            launch_record,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestAudit {
    NotAttempted,
    Attempted,
    Queued,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReaperState<ExitStatus, CleanupReceipt> {
    /// The exact child is still live or lifecycle cleanup has not finished.
    Pending,
    /// The child has exited/reaped, but descriptor/record/lease cleanup is not
    /// yet complete. This is not sufficient to acknowledge C1 cleanup.
    Reaped(ExitStatus),
    /// The existing exact-child terminal ledger says all tracked resources
    /// were disposed for this exact process instance and LaunchRecord.
    Complete(CleanupReceipt),
    /// The sole reaper could not establish an unambiguous terminal state.
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactObservation<ProcessInstance, LaunchRecord, ExitStatus, CleanupReceipt> {
    pub key: ExactChildKey<ProcessInstance, LaunchRecord>,
    pub state: ReaperState<ExitStatus, CleanupReceipt>,
}

/// Back this trait with the existing ProcessLifecycle and its one reaper. The
/// implementation must retain the real SpawnedProcessPidfd proof and query
/// that owner; the identity stored in ExactChildKey is only its match key.
/// `request_cleanup_exact` queues one selected child for the owner's existing
/// cleanup path. `observe_exact` must query through that same lifecycle/reaper;
/// it may drive the shared owner drain, but must not create a second reaper or
/// issue another termination/revoke request. The candidate overlay adds the
/// exact request method. C1 must retain one reconciler in its pending owner and
/// call `poll` again across owner notifications; this module does not invent
/// C1's retained pending state.
pub(crate) trait ExactChildSource {
    type ProcessInstance: Clone + Eq;
    type LaunchRecord: Clone + Eq;
    type ExitStatus;
    type CleanupReceipt;
    type Error;

    fn request_cleanup_exact(
        &mut self,
        key: &ExactChildKey<Self::ProcessInstance, Self::LaunchRecord>,
    ) -> Result<(), Self::Error>;

    fn observe_exact(
        &mut self,
        key: &ExactChildKey<Self::ProcessInstance, Self::LaunchRecord>,
    ) -> ExactObservation<
        Self::ProcessInstance,
        Self::LaunchRecord,
        Self::ExitStatus,
        Self::CleanupReceipt,
    >;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompletionPhase {
    NotRequested,
    Waiting,
    Unknown,
    Delivered,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateError {
    RequestAlreadyAttempted,
    RequestNotAttempted,
    ExactChildMismatch,
    TerminalStateAlreadyReached,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PollResult<ExitStatus, CleanupReceipt> {
    Pending,
    Reaped(ExitStatus),
    Unknown,
    Confirmed {
        receipt: CleanupReceipt,
        request_audit: RequestAudit,
    },
}

/// One recipient/one exact child bridge. Request state and completion state
/// are separate: retries may observe progress but may never repeat a cleanup
/// request. Unknown remains explicit. A full `Complete` result is emitted at
/// most once, so repeated notifications cannot manufacture multiple acks.
pub(crate) struct CleanupReconciler<Source: ExactChildSource> {
    key: ExactChildKey<Source::ProcessInstance, Source::LaunchRecord>,
    source: Source,
    request_audit: RequestAudit,
    completion_phase: CompletionPhase,
}

impl<Source: ExactChildSource> CleanupReconciler<Source> {
    pub(crate) fn new(
        key: ExactChildKey<Source::ProcessInstance, Source::LaunchRecord>,
        source: Source,
    ) -> Self {
        Self {
            key,
            source,
            request_audit: RequestAudit::NotAttempted,
            completion_phase: CompletionPhase::NotRequested,
        }
    }

    pub(crate) fn request_audit(&self) -> RequestAudit {
        self.request_audit
    }

    /// Let the lifecycle owner pump its ordinary event loop and route any
    /// resulting event through its existing handler between exact observations.
    /// This does not create a second wait owner and does not mutate the key.
    pub(crate) fn with_source_mut<R>(&mut self, operation: impl FnOnce(&mut Source) -> R) -> R {
        operation(&mut self.source)
    }

    /// Invoke the potentially effectful exact-child request at most once.
    /// Mark Attempted before entering the callback so unwind/error cannot make
    /// a second call look safe. A request error is retained as Unknown.
    pub(crate) fn request_once(&mut self) -> Result<RequestAudit, CandidateError> {
        if self.request_audit != RequestAudit::NotAttempted {
            return Err(CandidateError::RequestAlreadyAttempted);
        }
        self.request_audit = RequestAudit::Attempted;
        self.completion_phase = CompletionPhase::Waiting;
        self.request_audit = if self.source.request_cleanup_exact(&self.key).is_ok() {
            RequestAudit::Queued
        } else {
            RequestAudit::Unknown
        };
        Ok(self.request_audit)
    }

    /// Re-poll only the existing exact-child owner. Pending and Reaped remain
    /// nonterminal; Unknown is terminal fail-closed; only an exact `Complete`
    /// yields one confirmation result. No cleanup callback is retried here.
    pub(crate) fn poll(
        &mut self,
    ) -> Result<PollResult<Source::ExitStatus, Source::CleanupReceipt>, CandidateError> {
        if self.request_audit == RequestAudit::NotAttempted {
            return Err(CandidateError::RequestNotAttempted);
        }
        if matches!(
            self.completion_phase,
            CompletionPhase::Unknown | CompletionPhase::Delivered
        ) {
            return Err(CandidateError::TerminalStateAlreadyReached);
        }

        let key = ExactChildKey::from_owner(
            self.key.process_instance.clone(),
            self.key.launch_record.clone(),
        );
        let observation = self.source.observe_exact(&key);
        if observation.key != key {
            self.completion_phase = CompletionPhase::Unknown;
            return Err(CandidateError::ExactChildMismatch);
        }

        match observation.state {
            ReaperState::Pending => {
                self.completion_phase = CompletionPhase::Waiting;
                Ok(PollResult::Pending)
            }
            ReaperState::Reaped(status) => {
                self.completion_phase = CompletionPhase::Waiting;
                Ok(PollResult::Reaped(status))
            }
            ReaperState::Unknown => {
                self.completion_phase = CompletionPhase::Unknown;
                Ok(PollResult::Unknown)
            }
            ReaperState::Complete(receipt) => {
                self.completion_phase = CompletionPhase::Delivered;
                Ok(PollResult::Confirmed {
                    receipt,
                    request_audit: self.request_audit,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct FakeInstance(u8);

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct FakeRecord(u8);

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct FakeReceipt(u8);

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum FakeState {
        Pending,
        Reaped(u8),
        Complete(u8),
        Unknown,
    }

    struct FakeSource {
        request_results: VecDeque<Result<(), ()>>,
        observations: VecDeque<(ExactChildKey<FakeInstance, FakeRecord>, FakeState)>,
        request_calls: usize,
        observe_calls: usize,
    }

    impl ExactChildSource for FakeSource {
        type ProcessInstance = FakeInstance;
        type LaunchRecord = FakeRecord;
        type ExitStatus = u8;
        type CleanupReceipt = FakeReceipt;
        type Error = ();

        fn request_cleanup_exact(
            &mut self,
            _key: &ExactChildKey<Self::ProcessInstance, Self::LaunchRecord>,
        ) -> Result<(), Self::Error> {
            self.request_calls += 1;
            self.request_results.pop_front().unwrap_or(Ok(()))
        }

        fn observe_exact(
            &mut self,
            key: &ExactChildKey<Self::ProcessInstance, Self::LaunchRecord>,
        ) -> ExactObservation<
            Self::ProcessInstance,
            Self::LaunchRecord,
            Self::ExitStatus,
            Self::CleanupReceipt,
        > {
            self.observe_calls += 1;
            let (observed_key, state) = self
                .observations
                .pop_front()
                .unwrap_or((key.clone(), FakeState::Unknown));
            let state = match state {
                FakeState::Pending => ReaperState::Pending,
                FakeState::Reaped(status) => ReaperState::Reaped(status),
                FakeState::Complete(receipt) => ReaperState::Complete(FakeReceipt(receipt)),
                FakeState::Unknown => ReaperState::Unknown,
            };
            ExactObservation {
                key: observed_key,
                state,
            }
        }
    }

    fn key(instance: u8, record: u8) -> ExactChildKey<FakeInstance, FakeRecord> {
        ExactChildKey::from_owner(FakeInstance(instance), FakeRecord(record))
    }

    fn source(
        states: impl IntoIterator<Item = (ExactChildKey<FakeInstance, FakeRecord>, FakeState)>,
    ) -> FakeSource {
        FakeSource {
            request_results: VecDeque::from([Ok(())]),
            observations: states.into_iter().collect(),
            request_calls: 0,
            observe_calls: 0,
        }
    }

    type Reconciler = CleanupReconciler<FakeSource>;

    fn bridge(k: ExactChildKey<FakeInstance, FakeRecord>, s: FakeSource) -> Reconciler {
        CleanupReconciler::new(k, s)
    }

    #[test]
    fn pending_and_reaped_never_confirm_until_exact_resource_completion() {
        let target = key(1, 11);
        let mut bridge = bridge(
            target.clone(),
            source([
                (target.clone(), FakeState::Pending),
                (target.clone(), FakeState::Reaped(7)),
                (target.clone(), FakeState::Complete(11)),
            ]),
        );
        assert_eq!(bridge.request_once(), Ok(RequestAudit::Queued));
        assert_eq!(bridge.poll(), Ok(PollResult::Pending));
        assert_eq!(bridge.poll(), Ok(PollResult::Reaped(7)));
        assert_eq!(
            bridge.poll(),
            Ok(PollResult::Confirmed {
                receipt: FakeReceipt(11),
                request_audit: RequestAudit::Queued,
            })
        );
        assert_eq!(bridge.source.request_calls, 1);
        assert_eq!(bridge.source.observe_calls, 3);
    }

    #[test]
    fn unknown_never_confirms_or_retries_request() {
        let target = key(2, 22);
        let mut bridge = bridge(
            target.clone(),
            source([(target.clone(), FakeState::Unknown)]),
        );
        assert_eq!(bridge.request_once(), Ok(RequestAudit::Queued));
        assert_eq!(bridge.poll(), Ok(PollResult::Unknown));
        assert_eq!(
            bridge.poll(),
            Err(CandidateError::TerminalStateAlreadyReached)
        );
        assert_eq!(
            bridge.request_once(),
            Err(CandidateError::RequestAlreadyAttempted)
        );
        assert_eq!(bridge.source.request_calls, 1);
        assert_eq!(bridge.source.observe_calls, 1);
    }

    #[test]
    fn different_child_observation_is_rejected_fail_closed() {
        let target = key(3, 33);
        let wrong = key(4, 44);
        let mut bridge = bridge(target.clone(), source([(wrong, FakeState::Complete(44))]));
        assert_eq!(bridge.request_once(), Ok(RequestAudit::Queued));
        assert_eq!(bridge.poll(), Err(CandidateError::ExactChildMismatch));
        assert_eq!(
            bridge.poll(),
            Err(CandidateError::TerminalStateAlreadyReached)
        );
        assert_eq!(bridge.source.request_calls, 1);
        assert_eq!(bridge.source.observe_calls, 1);
    }

    #[test]
    fn duplicate_completion_notification_is_rejected_without_a_second_observation() {
        let target = key(5, 55);
        let mut bridge = bridge(
            target.clone(),
            source([
                (target.clone(), FakeState::Complete(55)),
                (target.clone(), FakeState::Complete(55)),
            ]),
        );
        assert_eq!(bridge.request_once(), Ok(RequestAudit::Queued));
        assert_eq!(
            bridge.poll(),
            Ok(PollResult::Confirmed {
                receipt: FakeReceipt(55),
                request_audit: RequestAudit::Queued,
            })
        );
        assert_eq!(
            bridge.poll(),
            Err(CandidateError::TerminalStateAlreadyReached)
        );
        assert_eq!(bridge.source.request_calls, 1);
        assert_eq!(bridge.source.observe_calls, 1);
    }

    #[test]
    fn ambiguous_request_is_not_retried_and_remains_visible_after_completion() {
        let target = key(6, 66);
        let mut source = source([(target.clone(), FakeState::Complete(66))]);
        source.request_results = VecDeque::from([Err(())]);
        let mut bridge = bridge(target.clone(), source);
        assert_eq!(bridge.request_once(), Ok(RequestAudit::Unknown));
        assert_eq!(bridge.request_audit(), RequestAudit::Unknown);
        assert_eq!(
            bridge.poll(),
            Ok(PollResult::Confirmed {
                receipt: FakeReceipt(66),
                request_audit: RequestAudit::Unknown,
            })
        );
        assert_eq!(bridge.source.request_calls, 1);
    }
}
