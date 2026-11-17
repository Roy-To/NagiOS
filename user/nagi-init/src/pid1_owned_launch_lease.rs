//! Exact managed-process ownership for the non-Clone endpoint/reap lease.
//! No syscall or reaper lives here; the ProcessLifecycle remains sole waiter.

use super::{BackendError, ServiceExitStatus};
use nagi_init_spawn_contract::SpawnedProcessIdentity;

pub(crate) struct BoundLaunchLease<LaunchLease, LaunchRecord> {
    pub(crate) process_instance: SpawnedProcessIdentity,
    pub(crate) record: LaunchRecord,
    /// Whether the exact child passed the exec gate. A failed write remains
    /// Unknown; all dispositions still require exact reap and Supervisor
    /// record revocation before lease retirement.
    pub(crate) gate_disposition: LaunchGateDisposition,
    pub(crate) lease: LaunchLease,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LaunchGateDisposition {
    NotAttempted,
    /// One release byte was acknowledged; this does not imply exec succeeded.
    Released,
    /// The attempted write failed, so whether the child crossed the gate is unknown.
    Unknown,
}

/// Implemented by Task41's existing endpoint/reap lease. There is deliberately
/// no implementation for `()` or a numeric PID.
pub(crate) trait ExactChildLaunchLease<LaunchRecord> {
    fn matches_spawn(
        &self,
        process_instance: &SpawnedProcessIdentity,
        record: &LaunchRecord,
    ) -> bool;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum LeaseRetirementState {
    NotAttempted,
    Attempted,
    Complete,
    Unknown,
}

/// Move a newly returned bridge lease into its exact lifecycle slot before
/// the authorization callback can return. A mismatch remains owner-held and
/// permanently Unknown so no later child can retire it by PID reuse.
pub(crate) fn adopt_exact_child_lease<LaunchLease, LaunchRecord: Copy + Eq>(
    owner_slot: &mut Option<BoundLaunchLease<LaunchLease, LaunchRecord>>,
    retirement_state: &mut LeaseRetirementState,
    process_instance: SpawnedProcessIdentity,
    record: LaunchRecord,
    lease: LaunchLease,
) -> Result<(), BackendError>
where
    LaunchLease: ExactChildLaunchLease<LaunchRecord>,
{
    if owner_slot.is_some() || *retirement_state != LeaseRetirementState::NotAttempted {
        return Err(BackendError::ProtocolViolation);
    }
    *owner_slot = Some(BoundLaunchLease {
        process_instance,
        record,
        gate_disposition: LaunchGateDisposition::NotAttempted,
        lease,
    });
    let bound = owner_slot
        .as_ref()
        .expect("lease was installed in the owner slot");
    let matches = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        bound
            .lease
            .matches_spawn(&bound.process_instance, &bound.record)
    }));
    match matches {
        Ok(true) => Ok(()),
        Ok(false) => {
            *retirement_state = LeaseRetirementState::Unknown;
            Err(BackendError::IdentityRejected)
        }
        Err(_) => {
            *retirement_state = LeaseRetirementState::Unknown;
            Err(BackendError::TransportFailure)
        }
    }
}

/// Retire once only after the caller supplies the actual exact-reap status and
/// after it has already revoked the matching Supervisor record. A mismatch,
/// error, or panic never becomes success and never triggers another attempt.
pub(crate) fn retire_exact_once<LaunchLease, LaunchRecord: Copy + Eq>(
    lease: &mut Option<BoundLaunchLease<LaunchLease, LaunchRecord>>,
    state: &mut LeaseRetirementState,
    expected_instance: &SpawnedProcessIdentity,
    expected_record: LaunchRecord,
    exit_status: ServiceExitStatus,
    retire: &mut impl FnMut(
        &mut BoundLaunchLease<LaunchLease, LaunchRecord>,
        ServiceExitStatus,
    ) -> Result<(), BackendError>,
) -> bool {
    match *state {
        LeaseRetirementState::Complete => return lease.is_none(),
        LeaseRetirementState::Attempted | LeaseRetirementState::Unknown => return false,
        LeaseRetirementState::NotAttempted => {}
    }
    let Some(bound) = lease.as_ref() else {
        *state = LeaseRetirementState::Complete;
        return true;
    };
    if !bound.process_instance.same_instance(expected_instance) || bound.record != expected_record {
        *state = LeaseRetirementState::Unknown;
        return false;
    }

    *state = LeaseRetirementState::Attempted;
    let owned = lease
        .as_mut()
        .expect("validated launch lease remains owned");
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| retire(owned, exit_status))) {
        Ok(Ok(())) => {
            *state = LeaseRetirementState::Complete;
            // The lease implementation must make Drop inert after explicit
            // retirement. Error/panic leaves the value in the owner slot.
            drop(lease.take());
            true
        }
        Ok(Err(_)) | Err(_) => {
            *state = LeaseRetirementState::Unknown;
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct FakeLease {
        instance: SpawnedProcessIdentity,
        record: u64,
    }

    impl ExactChildLaunchLease<u64> for FakeLease {
        fn matches_spawn(&self, process_instance: &SpawnedProcessIdentity, record: &u64) -> bool {
            self.instance.same_instance(process_instance) && self.record == *record
        }
    }

    struct PanicDuringBinding;

    impl ExactChildLaunchLease<u64> for PanicDuringBinding {
        fn matches_spawn(&self, _: &SpawnedProcessIdentity, _: &u64) -> bool {
            panic!("binding check outcome unknown")
        }
    }

    fn status(code: i32) -> ServiceExitStatus {
        ServiceExitStatus {
            exit_code: Some(code),
            signal: None,
        }
    }

    #[test]
    fn exact_reap_and_record_retire_nonclone_lease_once() {
        let instance = nagi_init_spawn_contract::test_identity(71);
        let mut lease = Some(BoundLaunchLease {
            process_instance: instance.clone(),
            record: 3001,
            gate_disposition: LaunchGateDisposition::Released,
            lease: FakeLease {
                instance: instance.clone(),
                record: 3001,
            },
        });
        let calls = Cell::new(0);
        let mut state = LeaseRetirementState::NotAttempted;
        let mut retire = |owned: &mut BoundLaunchLease<FakeLease, u64>, got_status| {
            assert_eq!(owned.record, 3001);
            assert_eq!(got_status, status(6));
            calls.set(calls.get() + 1);
            Ok(())
        };

        assert!(retire_exact_once(
            &mut lease,
            &mut state,
            &instance,
            3001,
            status(6),
            &mut retire
        ));
        assert!(lease.is_none());
        assert_eq!(state, LeaseRetirementState::Complete);
        assert!(retire_exact_once(
            &mut lease,
            &mut state,
            &instance,
            3001,
            status(6),
            &mut retire
        ));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn wrong_record_or_reused_pid_cannot_retire_another_lease() {
        let original = nagi_init_spawn_contract::test_identity(72);
        let reused = nagi_init_spawn_contract::test_identity(72);
        let mut lease = Some(BoundLaunchLease {
            process_instance: original.clone(),
            record: 3002,
            gate_disposition: LaunchGateDisposition::NotAttempted,
            lease: FakeLease {
                instance: original.clone(),
                record: 3002,
            },
        });
        let calls = Cell::new(0);
        let mut state = LeaseRetirementState::NotAttempted;
        let mut retire = |_: &mut BoundLaunchLease<FakeLease, u64>, _| {
            calls.set(calls.get() + 1);
            Ok(())
        };

        assert!(!retire_exact_once(
            &mut lease,
            &mut state,
            &reused,
            3002,
            status(0),
            &mut retire
        ));
        assert_eq!(state, LeaseRetirementState::Unknown);
        assert!(lease.is_some());
        assert_eq!(calls.get(), 0);
    }

    #[test]
    fn ambiguous_retirement_is_never_retried_or_reported_complete() {
        let instance = nagi_init_spawn_contract::test_identity(73);
        let mut lease = Some(BoundLaunchLease {
            process_instance: instance.clone(),
            record: 3003,
            gate_disposition: LaunchGateDisposition::Unknown,
            lease: FakeLease {
                instance: instance.clone(),
                record: 3003,
            },
        });
        let calls = Cell::new(0);
        let mut state = LeaseRetirementState::NotAttempted;
        let mut retire = |_: &mut BoundLaunchLease<FakeLease, u64>, _| {
            calls.set(calls.get() + 1);
            Err(BackendError::Unavailable)
        };

        assert!(!retire_exact_once(
            &mut lease,
            &mut state,
            &instance,
            3003,
            status(1),
            &mut retire
        ));
        assert_eq!(state, LeaseRetirementState::Unknown);
        assert!(!retire_exact_once(
            &mut lease,
            &mut state,
            &instance,
            3003,
            status(1),
            &mut retire
        ));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn retirement_panic_is_attempted_once_and_fails_closed() {
        let instance = nagi_init_spawn_contract::test_identity(74);
        let mut lease = Some(BoundLaunchLease {
            process_instance: instance.clone(),
            record: 3004,
            gate_disposition: LaunchGateDisposition::Released,
            lease: FakeLease {
                instance: instance.clone(),
                record: 3004,
            },
        });
        let calls = Cell::new(0);
        let mut state = LeaseRetirementState::NotAttempted;
        let mut retire =
            |_: &mut BoundLaunchLease<FakeLease, u64>, _| -> Result<(), BackendError> {
                calls.set(calls.get() + 1);
                panic!("retire outcome unknown")
            };

        assert!(!retire_exact_once(
            &mut lease,
            &mut state,
            &instance,
            3004,
            status(9),
            &mut retire
        ));
        assert_eq!(state, LeaseRetirementState::Unknown);
        assert!(!retire_exact_once(
            &mut lease,
            &mut state,
            &instance,
            3004,
            status(9),
            &mut retire
        ));
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn closed_exec_gate_abort_still_retires_after_exact_reap_and_record_revoke() {
        let instance = nagi_init_spawn_contract::test_identity(75);
        let mut lease = Some(BoundLaunchLease {
            process_instance: instance.clone(),
            record: 3005,
            gate_disposition: LaunchGateDisposition::NotAttempted,
            lease: FakeLease {
                instance: instance.clone(),
                record: 3005,
            },
        });
        let mut state = LeaseRetirementState::NotAttempted;
        let mut observed = None;
        let mut retire = |owned: &mut BoundLaunchLease<FakeLease, u64>, _| {
            observed = Some(owned.gate_disposition);
            Ok(())
        };

        assert!(retire_exact_once(
            &mut lease,
            &mut state,
            &instance,
            3005,
            status(127),
            &mut retire
        ));
        drop(retire);
        assert_eq!(observed, Some(LaunchGateDisposition::NotAttempted));
        assert_eq!(state, LeaseRetirementState::Complete);
    }

    #[test]
    fn uncertain_exec_gate_remains_distinct_when_exact_reap_retires_lease() {
        let instance = nagi_init_spawn_contract::test_identity(76);
        let mut lease = Some(BoundLaunchLease {
            process_instance: instance.clone(),
            record: 3006,
            gate_disposition: LaunchGateDisposition::Unknown,
            lease: FakeLease {
                instance: instance.clone(),
                record: 3006,
            },
        });
        let mut state = LeaseRetirementState::NotAttempted;
        let mut observed = None;
        let mut retire = |owned: &mut BoundLaunchLease<FakeLease, u64>, _| {
            observed = Some(owned.gate_disposition);
            Ok(())
        };

        assert!(retire_exact_once(
            &mut lease,
            &mut state,
            &instance,
            3006,
            status(1),
            &mut retire
        ));
        drop(retire);
        assert_eq!(observed, Some(LaunchGateDisposition::Unknown));
        assert_eq!(state, LeaseRetirementState::Complete);
    }

    #[test]
    fn mismatched_lease_is_retained_unknown_and_never_retired_by_reused_pid() {
        let expected = nagi_init_spawn_contract::test_identity(77);
        let lease_identity = nagi_init_spawn_contract::test_identity(77);
        let mut slot = None;
        let mut retirement_state = LeaseRetirementState::NotAttempted;
        let lease = FakeLease {
            instance: lease_identity,
            record: 3007,
        };

        assert_eq!(
            adopt_exact_child_lease(
                &mut slot,
                &mut retirement_state,
                expected.clone(),
                3007,
                lease,
            ),
            Err(BackendError::IdentityRejected)
        );
        assert!(slot.is_some());
        assert_eq!(retirement_state, LeaseRetirementState::Unknown);

        let calls = Cell::new(0);
        let mut retire = |_: &mut BoundLaunchLease<FakeLease, u64>, _| {
            calls.set(calls.get() + 1);
            Ok(())
        };
        assert!(!retire_exact_once(
            &mut slot,
            &mut retirement_state,
            &expected,
            3007,
            status(0),
            &mut retire,
        ));
        assert_eq!(calls.get(), 0);
        assert!(slot.is_some());
    }

    #[test]
    fn exact_lease_is_owner_held_before_authorization_callback_returns() {
        let instance = nagi_init_spawn_contract::test_identity(78);
        let mut slot = None;
        let mut retirement_state = LeaseRetirementState::NotAttempted;
        let lease = FakeLease {
            instance: instance.clone(),
            record: 3008,
        };

        assert_eq!(
            adopt_exact_child_lease(
                &mut slot,
                &mut retirement_state,
                instance.clone(),
                3008,
                lease,
            ),
            Ok(())
        );
        let owned = slot.as_ref().expect("exact owner slot has the lease");
        assert!(owned.process_instance.same_instance(&instance));
        assert_eq!(owned.record, 3008);
        assert_eq!(owned.gate_disposition, LaunchGateDisposition::NotAttempted);
        assert_eq!(retirement_state, LeaseRetirementState::NotAttempted);
    }

    #[test]
    fn binding_panic_leaves_lease_owner_held_and_unknown() {
        let instance = nagi_init_spawn_contract::test_identity(79);
        let mut slot = None;
        let mut retirement_state = LeaseRetirementState::NotAttempted;

        assert_eq!(
            adopt_exact_child_lease(
                &mut slot,
                &mut retirement_state,
                instance,
                3009,
                PanicDuringBinding,
            ),
            Err(BackendError::TransportFailure)
        );
        assert!(slot.is_some());
        assert_eq!(retirement_state, LeaseRetirementState::Unknown);
    }
}
