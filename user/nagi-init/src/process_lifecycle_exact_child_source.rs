//! Concrete Task24 `ExactChildSource` implementation co-located with the
//! existing `ProcessLifecycle` module through `#[path]` in the owner overlay.
//! The source owns a clone3-issued pidfd proof token and exact LaunchRecord;
//! all signaling, waiting, closing, revocation and lease retirement remain on
//! the borrowed ProcessLifecycle.

use super::pid1_owned_launch_lease::ExactChildLaunchLease;
use super::reconcile::{ExactChildKey, ExactChildSource, ExactObservation, ReaperState};
use super::{
    BackendError, BoundLaunchLease, ExactProcessTerminalResult, ExactTerminalState, LifecycleEvent,
    ProcessLifecycle, ProcessOs, ServiceExitStatus,
};
use nagi_init_spawn_contract::{SpawnedProcessIdentity, SpawnedProcessPidfd};

pub(crate) struct ProcessLifecycleExactChildSource<
    'owner,
    Os,
    ServiceId,
    LaunchRecord,
    LaunchLease,
    Revoke,
    Retire,
> where
    Os: ProcessOs,
    ServiceId: Clone + std::fmt::Debug + Eq,
    LaunchRecord: Copy + Eq,
    LaunchLease: ExactChildLaunchLease<LaunchRecord>,
{
    lifecycle: &'owner mut ProcessLifecycle<Os, ServiceId, LaunchRecord, LaunchLease>,
    proof: SpawnedProcessPidfd,
    record: LaunchRecord,
    revoke: Revoke,
    retire_lease: Retire,
}

impl<'owner, Os, ServiceId, LaunchRecord, LaunchLease, Revoke, Retire>
    ProcessLifecycleExactChildSource<
        'owner,
        Os,
        ServiceId,
        LaunchRecord,
        LaunchLease,
        Revoke,
        Retire,
    >
where
    Os: ProcessOs,
    ServiceId: Clone + std::fmt::Debug + Eq,
    LaunchRecord: Copy + Eq,
    LaunchLease: ExactChildLaunchLease<LaunchRecord>,
    Revoke: FnMut(u32, &LaunchRecord) -> Result<(), BackendError>,
    Retire: FnMut(
        &mut BoundLaunchLease<LaunchLease, LaunchRecord>,
        ServiceExitStatus,
    ) -> Result<(), BackendError>,
{
    /// Bind only an already-issued pidfd proof and record that are still held
    /// by this exact lifecycle and its non-Clone spawn lease. PID is used to
    /// find the slot, then the clone3 identity, lease binding, and record are
    /// checked before creating a key.
    /// The caller must select this pair from its launch-time context binding;
    /// this method does not map raw Session IDs or caller-supplied PID values.
    pub(crate) fn bind(
        lifecycle: &'owner mut ProcessLifecycle<Os, ServiceId, LaunchRecord, LaunchLease>,
        proof: SpawnedProcessPidfd,
        record: LaunchRecord,
        revoke: Revoke,
        retire_lease: Retire,
    ) -> Result<Self, BackendError> {
        let identity = proof.identity();
        let process_id = identity.process_id();
        let managed = lifecycle.children.iter().any(|entry| {
            entry.process_id == process_id
                && !entry.cleanup_requested
                && entry.record == record
                && lifecycle.os.same_process_instance(&entry.child, &identity)
                && entry.lease_cleanup == super::LeaseRetirementState::NotAttempted
                && entry.launch_lease.as_ref().is_some_and(|lease| {
                    lease.process_instance.same_instance(&identity) && lease.record == record
                })
        });
        let quarantined = lifecycle.quarantine.iter().any(|entry| {
            if entry.process_id != process_id {
                return false;
            }
            let same_instance = match &entry.child {
                super::PendingChild::Prepared(child) => {
                    lifecycle.os.same_process_instance(child, &identity)
                }
                super::PendingChild::ReleaseUncertain(child) => {
                    lifecycle.os.same_process_instance(child, &identity)
                }
                super::PendingChild::Running(child) => {
                    lifecycle.os.same_process_instance(child, &identity)
                }
            };
            let owned_record = entry
                .record
                .or_else(|| entry.launch_lease.as_ref().map(|lease| lease.record));
            let lease_matches = entry.launch_lease.as_ref().is_some_and(|lease| {
                lease.process_instance.same_instance(&identity) && lease.record == record
            });
            same_instance
                && owned_record == Some(record)
                && entry.lease_cleanup == super::LeaseRetirementState::NotAttempted
                && lease_matches
        });
        if !managed && !quarantined {
            return Err(BackendError::IdentityRejected);
        }

        Ok(Self {
            lifecycle,
            proof,
            record,
            revoke,
            retire_lease,
        })
    }

    pub(crate) fn key(&self) -> ExactChildKey<SpawnedProcessIdentity, LaunchRecord> {
        ExactChildKey::from_owner(self.proof.identity(), self.record)
    }

    /// Advance exactly one ordinary lifecycle poll. Any event is returned to
    /// the existing caller for dispatch; it is never discarded by this bridge.
    /// This may observe/reap children only through the existing ProcessOs and
    /// Pid1ChildReaper and may never repeat a failed at-most-once side effect.
    pub(crate) fn poll_owner_once(
        &mut self,
    ) -> Result<Option<LifecycleEvent<ServiceId>>, BackendError> {
        self.lifecycle
            .poll(&mut self.revoke, &mut self.retire_lease)
    }
}

impl<Os, ServiceId, Record, LaunchLease, Revoke, Retire> ExactChildSource
    for ProcessLifecycleExactChildSource<'_, Os, ServiceId, Record, LaunchLease, Revoke, Retire>
where
    Os: ProcessOs,
    ServiceId: Clone + std::fmt::Debug + Eq,
    Record: Copy + Eq,
    LaunchLease: ExactChildLaunchLease<Record>,
    Revoke: FnMut(u32, &Record) -> Result<(), BackendError>,
    Retire: FnMut(
        &mut BoundLaunchLease<LaunchLease, Record>,
        ServiceExitStatus,
    ) -> Result<(), BackendError>,
{
    type ProcessInstance = SpawnedProcessIdentity;
    type LaunchRecord = Record;
    type ExitStatus = ServiceExitStatus;
    type CleanupReceipt = ExactProcessTerminalResult<Record>;
    type Error = BackendError;

    fn request_cleanup_exact(
        &mut self,
        key: &ExactChildKey<Self::ProcessInstance, Self::LaunchRecord>,
    ) -> Result<(), Self::Error> {
        if *key != self.key() {
            return Err(BackendError::IdentityRejected);
        }
        self.lifecycle
            .request_cleanup_exact(&self.proof, self.record)
    }

    /// Observe only this proof+record through ProcessLifecycle's exact result
    /// API. It does not drive a second reaper; callers separately pump the
    /// existing lifecycle and route any returned LifecycleEvent.
    fn observe_exact(
        &mut self,
        key: &ExactChildKey<Self::ProcessInstance, Self::LaunchRecord>,
    ) -> ExactObservation<
        Self::ProcessInstance,
        Self::LaunchRecord,
        Self::ExitStatus,
        Self::CleanupReceipt,
    > {
        let exact_key = self.key();
        let state = if *key != exact_key {
            ReaperState::Unknown
        } else {
            match self
                .lifecycle
                .query_terminal_result(&self.proof, self.record)
            {
                Ok(result) => match result.state {
                    ExactTerminalState::Pending => ReaperState::Pending,
                    ExactTerminalState::Reaped(status) => ReaperState::Reaped(status),
                    ExactTerminalState::Complete(_) => ReaperState::Complete(result),
                    ExactTerminalState::Unknown => ReaperState::Unknown,
                },
                Err(_) => ReaperState::Unknown,
            }
        };
        ExactObservation {
            key: exact_key,
            state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::pid1_child_reaper::{Pid1ChildReaper, ReapKernel, ReapTicket};
    use super::super::pid1_owned_launch_lease::{
        adopt_exact_child_lease, ExactChildLaunchLease, LaunchGateDisposition,
    };
    use super::super::reconcile::{CandidateError, CleanupReconciler, PollResult, RequestAudit};
    use super::super::{
        CleanupAttempt, ManagedProcess, Prepared, ProcessHandle, Running, TerminationAuditState,
    };
    use super::*;
    use nagi_init_spawn_contract::{
        test_identity, test_pidfd_for_identity, SpawnedProcessIdentity,
    };
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;
    use std::io;
    use std::rc::Rc;

    #[derive(Default)]
    struct FakeReapKernel {
        exited: BTreeMap<u32, (i32, ServiceExitStatus)>,
        consumed: Vec<u32>,
    }

    impl ReapKernel for FakeReapKernel {
        fn observe_any_exit_nonconsuming(&mut self) -> io::Result<Option<u32>> {
            Ok(self.exited.keys().next().copied())
        }

        fn observe_cleanup_exit_nonconsuming(&mut self, process_id: u32) -> io::Result<bool> {
            Ok(self.exited.contains_key(&process_id))
        }

        fn observe_tracked_exit_nonconsuming(
            &mut self,
            process_id: u32,
            pidfd: i32,
        ) -> io::Result<bool> {
            Ok(self
                .exited
                .get(&process_id)
                .is_some_and(|(expected_pidfd, _)| *expected_pidfd == pidfd))
        }

        fn reap_exact(&mut self, process_id: u32) -> io::Result<Option<ServiceExitStatus>> {
            let result = self.exited.remove(&process_id).map(|(_, status)| status);
            if result.is_some() {
                self.consumed.push(process_id);
            }
            Ok(result)
        }
    }

    struct FakeRaw {
        identity: SpawnedProcessIdentity,
        ticket: ReapTicket,
        signal_attempted: bool,
        closed: bool,
    }

    struct FakeEndpointLease {
        identity: SpawnedProcessIdentity,
        record: u64,
        retire_calls: Rc<Cell<usize>>,
    }

    impl ExactChildLaunchLease<u64> for FakeEndpointLease {
        fn matches_spawn(&self, identity: &SpawnedProcessIdentity, record: &u64) -> bool {
            self.identity.same_instance(identity) && self.record == *record
        }
    }

    #[derive(Default)]
    struct ReaperBackedOs {
        reaper: Pid1ChildReaper,
        kernel: FakeReapKernel,
        signal_calls: usize,
    }

    impl ReaperBackedOs {
        fn register_child(&mut self, process_id: u32) -> (FakeRaw, SpawnedProcessPidfd) {
            let proof = test_pidfd_for_identity(test_identity(process_id));
            let identity = proof.identity();
            let fake_pidfd = process_id as i32 + 10_000;
            let ticket = self
                .reaper
                .reserve_ticket()
                .unwrap()
                .bind(identity.clone(), fake_pidfd);
            self.reaper.register(ticket.clone()).unwrap();
            (
                FakeRaw {
                    identity,
                    ticket,
                    signal_attempted: false,
                    closed: false,
                },
                proof,
            )
        }

        fn set_exited(&mut self, process_id: u32, status: ServiceExitStatus) {
            let fake_pidfd = process_id as i32 + 10_000;
            self.kernel.exited.insert(process_id, (fake_pidfd, status));
        }
    }

    impl ProcessOs for ReaperBackedOs {
        type Raw = FakeRaw;

        fn pidfd_handoff(
            &self,
            child: &ProcessHandle<Prepared, Self::Raw>,
        ) -> io::Result<SpawnedProcessPidfd> {
            Ok(test_pidfd_for_identity(child.raw.identity.clone()))
        }

        fn same_process_instance<Stage: super::super::ProcessStage>(
            &self,
            child: &ProcessHandle<Stage, Self::Raw>,
            identity: &SpawnedProcessIdentity,
        ) -> bool {
            child.raw.identity.same_instance(identity)
        }

        fn process_instance_identity<Stage: super::super::ProcessStage>(
            &self,
            child: &ProcessHandle<Stage, Self::Raw>,
        ) -> Option<SpawnedProcessIdentity> {
            Some(child.raw.identity.clone())
        }

        fn termination_audit_state<Stage: super::super::ProcessStage>(
            &self,
            child: &ProcessHandle<Stage, Self::Raw>,
        ) -> TerminationAuditState {
            if child.raw.signal_attempted {
                TerminationAuditState::Acknowledged
            } else {
                TerminationAuditState::NotAttempted
            }
        }

        fn prepare(
            &mut self,
            _executable: super::super::ExecutablePayload<'_>,
            _arguments: &[&str],
            _execution_identity: super::super::LinuxExecutionIdentity,
        ) -> io::Result<ProcessHandle<Prepared, Self::Raw>> {
            Err(io::Error::other("test does not spawn through prepare"))
        }

        fn commit_release_gate<LaunchRecord>(
            &mut self,
            _child: &mut ProcessHandle<Prepared, Self::Raw>,
            _registered: &super::super::RegisteredLaunch<LaunchRecord>,
            _wait_budget: std::time::Duration,
        ) -> Result<std::time::Instant, super::super::ReleaseFailureReason> {
            Err(super::super::ReleaseFailureReason::Io)
        }

        fn wait_exec_ack(
            &mut self,
            _child: &mut ProcessHandle<super::super::ReleaseUncertain, Self::Raw>,
            _started: std::time::Instant,
            _wait_budget: std::time::Duration,
            _cancel_check_interval: std::time::Duration,
            _cancelled: &mut dyn FnMut() -> bool,
        ) -> Result<(), super::super::ReleaseFailureInfo> {
            Err(super::super::ReleaseFailureInfo {
                reason: super::super::ReleaseFailureReason::Io,
                reaped: None,
            })
        }

        fn try_wait<Stage: super::super::ProcessStage>(
            &mut self,
            child: &mut ProcessHandle<Stage, Self::Raw>,
        ) -> io::Result<Option<ServiceExitStatus>> {
            self.reaper.drain(&mut self.kernel)?;
            child.raw.ticket.exit_status()
        }

        fn close_child<Stage: super::super::ProcessStage>(
            &mut self,
            child: &mut ProcessHandle<Stage, Self::Raw>,
        ) -> io::Result<()> {
            if std::mem::replace(&mut child.raw.closed, true) {
                return Err(io::Error::other("duplicate fake close"));
            }
            Ok(())
        }

        fn terminate<Stage: super::super::ProcessStage>(
            &mut self,
            child: &mut ProcessHandle<Stage, Self::Raw>,
        ) -> io::Result<()> {
            if !child.raw.signal_attempted {
                child.raw.signal_attempted = true;
                self.signal_calls += 1;
            }
            Ok(())
        }
    }

    fn status(exit_code: i32) -> ServiceExitStatus {
        ServiceExitStatus {
            exit_code: Some(exit_code),
            signal: None,
        }
    }

    fn lifecycle_with_children(
        children: &[(u32, u64, u8)],
    ) -> (
        super::super::ProcessLifecycle<ReaperBackedOs, u8, u64, FakeEndpointLease>,
        Vec<SpawnedProcessPidfd>,
    ) {
        let mut lifecycle: super::super::ProcessLifecycle<
            ReaperBackedOs,
            u8,
            u64,
            FakeEndpointLease,
        > = super::super::ProcessLifecycle::new(ReaperBackedOs::default());
        let mut proofs = Vec::new();
        for (process_id, record, service) in children.iter().copied() {
            let (raw, proof) = lifecycle.os.register_child(process_id);
            let identity = proof.identity();
            let mut launch_lease = None;
            let mut lease_cleanup = super::super::LeaseRetirementState::NotAttempted;
            adopt_exact_child_lease(
                &mut launch_lease,
                &mut lease_cleanup,
                identity.clone(),
                record,
                FakeEndpointLease {
                    identity,
                    record,
                    retire_calls: Rc::new(Cell::new(0)),
                },
            )
            .unwrap();
            lifecycle.children.push(ManagedProcess {
                process_id,
                service,
                required: false,
                child: ProcessHandle::<Prepared, _>::new(process_id, raw).transition::<Running>(),
                record,
                record_cleanup: CleanupAttempt::NotAttempted,
                launch_lease,
                lease_cleanup,
                reaped: None,
                cleanup_requested: false,
            });
            proofs.push(proof);
        }
        (lifecycle, proofs)
    }

    #[test]
    fn exact_request_rejects_another_child_and_cannot_signal_twice() {
        let (mut lifecycle, mut proofs) =
            lifecycle_with_children(&[(7001, 7101, 1), (7002, 7102, 2)]);
        let proof_a = proofs.remove(0);
        let proof_b = proofs.remove(0);
        let source = ProcessLifecycleExactChildSource::bind(
            &mut lifecycle,
            proof_a,
            7101,
            |_pid, _record| Ok(()),
            |_lease: &mut BoundLaunchLease<FakeEndpointLease, u64>, _status| Ok(()),
        )
        .unwrap();
        let key_a = source.key();
        let mut reconciler = CleanupReconciler::new(key_a.clone(), source);
        let key_b = ExactChildKey::from_owner(proof_b.identity(), 7102);

        let wrong_request = reconciler
            .with_source_mut(|source| ExactChildSource::request_cleanup_exact(source, &key_b));
        assert_eq!(wrong_request, Err(BackendError::IdentityRejected));
        let wrong_observation =
            reconciler.with_source_mut(|source| ExactChildSource::observe_exact(source, &key_b));
        assert_eq!(wrong_observation.key, key_a);
        assert_eq!(wrong_observation.state, ReaperState::Unknown);
        assert_eq!(
            reconciler.with_source_mut(|source| source.lifecycle.os.signal_calls),
            0
        );

        assert_eq!(reconciler.request_once(), Ok(RequestAudit::Queued));
        assert_eq!(
            reconciler.request_once(),
            Err(CandidateError::RequestAlreadyAttempted)
        );
        let duplicate_owner_request = reconciler
            .with_source_mut(|source| ExactChildSource::request_cleanup_exact(source, &key_a));
        assert_eq!(
            duplicate_owner_request,
            Err(BackendError::ProtocolViolation)
        );
        assert_eq!(
            reconciler.with_source_mut(|source| source.lifecycle.os.signal_calls),
            1
        );
    }

    #[test]
    fn exact_reaper_progresses_pending_reaped_complete_and_routes_other_child() {
        let (mut lifecycle, mut proofs) =
            lifecycle_with_children(&[(7201, 7211, 1), (7202, 7212, 2)]);
        let proof_a = proofs.remove(0);
        let proof_b = proofs.remove(0);
        let target_identity = proof_a.identity();
        let revoked = Rc::new(RefCell::new(Vec::new()));
        let revoked_for_callback = Rc::clone(&revoked);
        let source = ProcessLifecycleExactChildSource::bind(
            &mut lifecycle,
            proof_a,
            7211,
            move |pid, record| {
                revoked_for_callback.borrow_mut().push((pid, *record));
                Ok(())
            },
            |_lease: &mut BoundLaunchLease<FakeEndpointLease, u64>, _status| Ok(()),
        )
        .unwrap();
        let key = source.key();
        let mut reconciler = CleanupReconciler::new(key.clone(), source);
        assert_eq!(reconciler.request_once(), Ok(RequestAudit::Queued));

        assert_eq!(reconciler.poll(), Ok(PollResult::Pending));

        reconciler.with_source_mut(|source| {
            source.lifecycle.os.set_exited(7202, status(22));
        });
        assert_eq!(reconciler.poll(), Ok(PollResult::Pending));
        let other_event = reconciler
            .with_source_mut(|source| source.poll_owner_once())
            .unwrap()
            .expect("unrelated exited child event must be returned to the owner");
        match other_event {
            LifecycleEvent::ServiceExited {
                service,
                status: got,
                ..
            } => {
                assert_eq!(service, 2);
                assert_eq!(got, status(22));
            }
            _ => panic!("expected the unrelated ServiceExited event"),
        }
        assert_eq!(reconciler.poll(), Ok(PollResult::Pending));
        assert!(revoked.borrow().contains(&(7202, 7212)));

        reconciler.with_source_mut(|source| {
            source.lifecycle.os.set_exited(7201, status(11));
        });
        assert_eq!(reconciler.poll(), Ok(PollResult::Reaped(status(11))));
        let target_event = reconciler
            .with_source_mut(|source| source.poll_owner_once())
            .unwrap()
            .expect("target child completion event must be returned");
        match target_event {
            LifecycleEvent::ServiceExited {
                service,
                status: got,
                ..
            } => {
                assert_eq!(service, 1);
                assert_eq!(got, status(11));
            }
            _ => panic!("expected target ServiceExited event"),
        }

        let confirmation = reconciler.poll().unwrap();
        let receipt = match confirmation {
            PollResult::Confirmed {
                receipt,
                request_audit,
            } => {
                assert_eq!(request_audit, RequestAudit::Queued);
                receipt
            }
            other => panic!("expected exact terminal receipt, got {other:?}"),
        };
        assert!(receipt.process_instance.same_instance(&target_identity));
        assert_eq!(receipt.record, 7211);
        assert_eq!(receipt.state, ExactTerminalState::Complete(status(11)));
        assert!(revoked.borrow().contains(&(7201, 7211)));
        assert_eq!(
            reconciler.with_source_mut(|source| source.lifecycle.os.signal_calls),
            1,
            "the lifecycle poll must not resend the request's signal"
        );
        assert_eq!(
            reconciler.poll(),
            Err(CandidateError::TerminalStateAlreadyReached)
        );

        let other_receipt = reconciler.with_source_mut(|source| {
            source
                .lifecycle
                .query_terminal_result(&proof_b, 7212)
                .unwrap()
        });
        assert_eq!(other_receipt.record, 7212);
        assert_eq!(
            other_receipt.state,
            ExactTerminalState::Complete(status(22))
        );
    }

    #[test]
    fn complete_receipt_waits_for_exact_launch_lease_retirement() {
        let mut os = ReaperBackedOs::default();
        let (raw, proof) = os.register_child(7301);
        let identity = proof.identity();
        let lease_retire_calls = Rc::new(Cell::new(0));
        let lease = FakeEndpointLease {
            identity: identity.clone(),
            record: 7311,
            retire_calls: Rc::clone(&lease_retire_calls),
        };
        let mut lease_slot = None;
        let mut lease_state = super::super::LeaseRetirementState::NotAttempted;
        adopt_exact_child_lease(&mut lease_slot, &mut lease_state, identity, 7311, lease).unwrap();

        let mut lifecycle: super::super::ProcessLifecycle<
            ReaperBackedOs,
            u8,
            u64,
            FakeEndpointLease,
        > = super::super::ProcessLifecycle::new(os);
        lifecycle.children.push(ManagedProcess {
            process_id: 7301,
            service: 3,
            required: false,
            child: ProcessHandle::<Prepared, _>::new(7301, raw).transition::<Running>(),
            record: 7311,
            record_cleanup: CleanupAttempt::NotAttempted,
            launch_lease: lease_slot,
            lease_cleanup: lease_state,
            reaped: None,
            cleanup_requested: false,
        });

        let record_revocations = Rc::new(RefCell::new(Vec::new()));
        let revocations_for_callback = Rc::clone(&record_revocations);
        let retire_calls = Rc::clone(&lease_retire_calls);
        let source = ProcessLifecycleExactChildSource::bind(
            &mut lifecycle,
            proof,
            7311,
            move |pid, record| {
                revocations_for_callback.borrow_mut().push((pid, *record));
                Ok(())
            },
            move |bound: &mut BoundLaunchLease<FakeEndpointLease, u64>, _status| {
                assert_eq!(bound.record, 7311);
                assert_eq!(bound.gate_disposition, LaunchGateDisposition::NotAttempted);
                bound.lease.retire_calls.set(retire_calls.get() + 1);
                Ok(())
            },
        )
        .unwrap();
        let mut reconciler = CleanupReconciler::new(source.key(), source);
        assert_eq!(reconciler.request_once(), Ok(RequestAudit::Queued));
        assert_eq!(reconciler.poll(), Ok(PollResult::Pending));

        reconciler.with_source_mut(|source| {
            source.lifecycle.os.set_exited(7301, status(31));
        });
        assert_eq!(reconciler.poll(), Ok(PollResult::Reaped(status(31))));
        reconciler
            .with_source_mut(|source| source.poll_owner_once())
            .unwrap();
        let receipt = match reconciler.poll().unwrap() {
            PollResult::Confirmed { receipt, .. } => receipt,
            other => panic!("expected completed exact receipt, got {other:?}"),
        };
        assert_eq!(receipt.record, 7311);
        assert_eq!(receipt.state, ExactTerminalState::Complete(status(31)));
        assert_eq!(
            lease_retire_calls.get(),
            1,
            "the terminal receipt requires exact bound lease retirement"
        );
        assert_eq!(record_revocations.borrow().as_slice(), &[(7301, 7311)]);
    }
}
