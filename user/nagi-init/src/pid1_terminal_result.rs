//! Exact process-instance + LaunchRecord terminal result handoff.
//! This is a result ledger only; all wait/reap operations remain owned by the
//! single Pid1ChildReaper coordinator.

use super::pid1_owned_launch_lease::LaunchGateDisposition;
use super::ServiceExitStatus;
use nagi_init_spawn_contract::SpawnedProcessIdentity;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExactTerminalState {
    Pending,
    Reaped(ServiceExitStatus),
    Complete(ServiceExitStatus),
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExactProcessTerminalResult<LaunchRecord> {
    pub(crate) process_instance: SpawnedProcessIdentity,
    pub(crate) record: LaunchRecord,
    /// `None` is used only by lifecycle backends without a spawn proof lease.
    /// Linux exact child results preserve whether the exec gate was closed,
    /// acknowledged, or left uncertain.
    pub(crate) gate_disposition: Option<LaunchGateDisposition>,
    pub(crate) state: ExactTerminalState,
}

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum TerminalResultLookupError {
    RecordMismatch,
}

pub(crate) struct TerminalResultLedger<LaunchRecord> {
    results: Vec<ExactProcessTerminalResult<LaunchRecord>>,
}

impl<LaunchRecord> Default for TerminalResultLedger<LaunchRecord> {
    fn default() -> Self {
        Self {
            results: Vec::new(),
        }
    }
}

impl<LaunchRecord: Copy + Eq> TerminalResultLedger<LaunchRecord> {
    /// Reserve capacity before descriptor close and record revocation effects.
    pub(crate) fn reserve_completion_slot(&mut self) -> Result<(), ()> {
        self.results.try_reserve(1).map_err(|_| ())
    }

    /// Record only after exact reap, child descriptor close, and the matching
    /// LaunchRecord revocation all completed. Caller reserves first.
    pub(crate) fn push_complete(
        &mut self,
        process_instance: SpawnedProcessIdentity,
        record: LaunchRecord,
        status: ServiceExitStatus,
        gate_disposition: Option<LaunchGateDisposition>,
    ) {
        self.results.push(ExactProcessTerminalResult {
            process_instance,
            record,
            gate_disposition,
            state: ExactTerminalState::Complete(status),
        });
    }

    /// Consume one result for the exact private instance nonce and record. A
    /// same-PID process or a different record never receives the result.
    pub(crate) fn take_complete(
        &mut self,
        process_instance: &SpawnedProcessIdentity,
        record: LaunchRecord,
    ) -> Result<Option<ExactProcessTerminalResult<LaunchRecord>>, TerminalResultLookupError> {
        let Some(index) = self
            .results
            .iter()
            .position(|result| result.process_instance == *process_instance)
        else {
            return Ok(None);
        };
        if self.results[index].record != record {
            return Err(TerminalResultLookupError::RecordMismatch);
        }
        Ok(Some(self.results.remove(index)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(exit_code: i32) -> ServiceExitStatus {
        ServiceExitStatus {
            exit_code: Some(exit_code),
            signal: None,
        }
    }

    #[test]
    fn exact_record_gets_only_its_complete_reap_result_once() {
        let identity = nagi_init_spawn_contract::test_identity(41);
        let mut ledger = TerminalResultLedger::default();
        ledger.reserve_completion_slot().unwrap();
        ledger.push_complete(identity.clone(), 7001u64, status(3), None);

        let result = ledger.take_complete(&identity, 7001).unwrap().unwrap();
        assert_eq!(result.process_instance, identity);
        assert_eq!(result.record, 7001);
        assert_eq!(result.gate_disposition, None);
        assert_eq!(result.state, ExactTerminalState::Complete(status(3)));
        assert!(ledger.take_complete(&identity, 7001).unwrap().is_none());
    }

    #[test]
    fn wrong_record_is_rejected_without_consuming_the_result() {
        let identity = nagi_init_spawn_contract::test_identity(42);
        let mut ledger = TerminalResultLedger::default();
        ledger.reserve_completion_slot().unwrap();
        ledger.push_complete(identity.clone(), 7002u64, status(4), None);

        assert_eq!(
            ledger.take_complete(&identity, 9999),
            Err(TerminalResultLookupError::RecordMismatch)
        );
        assert_eq!(
            ledger
                .take_complete(&identity, 7002)
                .unwrap()
                .unwrap()
                .state,
            ExactTerminalState::Complete(status(4))
        );
    }

    #[test]
    fn different_spawn_with_reused_pid_never_receives_another_result() {
        let original = nagi_init_spawn_contract::test_identity(43);
        let reused_pid = nagi_init_spawn_contract::test_identity(43);
        let mut ledger = TerminalResultLedger::default();
        ledger.reserve_completion_slot().unwrap();
        ledger.push_complete(original.clone(), 7003u64, status(5), None);

        assert!(ledger.take_complete(&reused_pid, 7003).unwrap().is_none());
        assert_eq!(
            ledger
                .take_complete(&original, 7003)
                .unwrap()
                .unwrap()
                .state,
            ExactTerminalState::Complete(status(5))
        );
    }

    #[test]
    fn exact_terminal_result_preserves_exec_gate_uncertainty() {
        let identity = nagi_init_spawn_contract::test_identity(44);
        let mut ledger = TerminalResultLedger::default();
        ledger.reserve_completion_slot().unwrap();
        ledger.push_complete(
            identity.clone(),
            7004u64,
            status(127),
            Some(LaunchGateDisposition::Unknown),
        );

        let result = ledger.take_complete(&identity, 7004).unwrap().unwrap();
        assert_eq!(
            result.gate_disposition,
            Some(LaunchGateDisposition::Unknown)
        );
        assert_eq!(result.state, ExactTerminalState::Complete(status(127)));
    }
}
