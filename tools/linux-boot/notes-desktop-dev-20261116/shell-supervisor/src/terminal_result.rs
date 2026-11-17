//! Consumer view of Task24's exact terminal result contract. The boot adapter
//! maps `ProcessLifecycle::query_terminal_result(proof, record)` into this
//! small view; the Shell loop verifies the opaque process instance and record
//! again before allowing close/reopen.

use crate::existing_shell_consumer::ProcessRecord;
use nagi_init_spawn_contract::{SpawnedProcessIdentity, SpawnedProcessPidfd};

/// A non-forgeable Task24 process proof supplies the identity issued by the
/// same spawn transaction. Tests use a test-only identity fixture; the normal
/// boot adapter uses `SpawnedProcessPidfd` directly.
pub(super) trait ExactProcessProof {
    fn process_identity(&self) -> SpawnedProcessIdentity;
}

impl ExactProcessProof for SpawnedProcessPidfd {
    fn process_identity(&self) -> SpawnedProcessIdentity {
        self.identity()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExecGateDisposition {
    NotAttempted,
    Released,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ExitStatus {
    pub(super) exit_code: Option<i32>,
    pub(super) signal: Option<i32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExactTerminalState {
    Pending,
    Reaped(ExitStatus),
    Complete(ExitStatus),
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ExactTerminalResult {
    pub(super) process_instance: SpawnedProcessIdentity,
    pub(super) record: ProcessRecord,
    /// `None` is retained for backends without Task24's exact spawn proof and
    /// cannot authorize release of the Shell's active-child owner.
    pub(super) gate_disposition: Option<ExecGateDisposition>,
    pub(super) state: ExactTerminalState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TerminalDecision {
    Pending,
    ReapedCleanupPending(ExitStatus),
    Complete(ExitStatus),
    Unknown,
    ExecGateNotReleased,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TerminalIdentityError {
    ProcessInstanceMismatch,
    LaunchRecordMismatch,
}

impl ExactTerminalResult {
    /// Match both the opaque process nonce and the full Task24 LaunchRecord.
    /// PID equality is intentionally insufficient.
    pub(super) fn classify<P: ExactProcessProof>(
        &self,
        proof: &P,
        expected_record: ProcessRecord,
    ) -> Result<TerminalDecision, TerminalIdentityError> {
        if !self
            .process_instance
            .same_instance(&proof.process_identity())
        {
            return Err(TerminalIdentityError::ProcessInstanceMismatch);
        }
        if self.record != expected_record {
            return Err(TerminalIdentityError::LaunchRecordMismatch);
        }
        let decision = match self.state {
            ExactTerminalState::Pending => TerminalDecision::Pending,
            ExactTerminalState::Reaped(status) => TerminalDecision::ReapedCleanupPending(status),
            ExactTerminalState::Complete(status)
                if self.gate_disposition == Some(ExecGateDisposition::Released) =>
            {
                TerminalDecision::Complete(status)
            }
            ExactTerminalState::Complete(_) => TerminalDecision::ExecGateNotReleased,
            ExactTerminalState::Unknown => TerminalDecision::Unknown,
        };
        Ok(decision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestProof(SpawnedProcessIdentity);

    impl ExactProcessProof for TestProof {
        fn process_identity(&self) -> SpawnedProcessIdentity {
            self.0.clone()
        }
    }

    fn status(exit_code: i32) -> ExitStatus {
        ExitStatus {
            exit_code: Some(exit_code),
            signal: None,
        }
    }

    fn record(process_id: u32) -> ProcessRecord {
        #[cfg(target_os = "linux")]
        {
            // The Linux LaunchRecord is issued by the source Supervisor and
            // intentionally cannot be fabricated in this task-local crate.
            let _ = process_id;
            unreachable!("Linux exact LaunchRecord is supplied by the boot owner")
        }
        #[cfg(not(target_os = "linux"))]
        {
            use nagi_model::{AppSessionId, NodeId};
            ProcessRecord {
                process_id,
                app_id: nagi_notes::APP_ID,
                app_session_id: AppSessionId(7),
                node_id: NodeId(2),
                workspace_id: None,
            }
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn exact_result_rejects_reused_pid_and_mismatched_record() {
        let identity = nagi_init_spawn_contract::test_identity(41);
        let recycled = nagi_init_spawn_contract::test_identity(41);
        let proof = TestProof(identity.clone());
        let launch_record = record(41);
        let result = ExactTerminalResult {
            process_instance: identity,
            record: launch_record,
            gate_disposition: Some(ExecGateDisposition::Released),
            state: ExactTerminalState::Complete(status(0)),
        };
        assert_eq!(
            result.classify(&TestProof(recycled), launch_record),
            Err(TerminalIdentityError::ProcessInstanceMismatch)
        );
        assert_eq!(
            result.classify(&proof, record(42)),
            Err(TerminalIdentityError::LaunchRecordMismatch)
        );
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn only_complete_and_released_gate_authorizes_reopen() {
        let identity = nagi_init_spawn_contract::test_identity(42);
        let proof = TestProof(identity.clone());
        let launch_record = record(42);
        let classify = |state, gate_disposition| {
            ExactTerminalResult {
                process_instance: identity.clone(),
                record: launch_record,
                gate_disposition,
                state,
            }
            .classify(&proof, launch_record)
            .unwrap()
        };
        assert_eq!(
            classify(
                ExactTerminalState::Pending,
                Some(ExecGateDisposition::Released)
            ),
            TerminalDecision::Pending
        );
        assert_eq!(
            classify(
                ExactTerminalState::Reaped(status(0)),
                Some(ExecGateDisposition::Released)
            ),
            TerminalDecision::ReapedCleanupPending(status(0))
        );
        assert_eq!(
            classify(
                ExactTerminalState::Complete(status(0)),
                Some(ExecGateDisposition::Released)
            ),
            TerminalDecision::Complete(status(0))
        );
        assert_eq!(
            classify(
                ExactTerminalState::Complete(status(0)),
                Some(ExecGateDisposition::NotAttempted)
            ),
            TerminalDecision::ExecGateNotReleased
        );
        assert_eq!(
            classify(
                ExactTerminalState::Complete(status(0)),
                Some(ExecGateDisposition::Unknown)
            ),
            TerminalDecision::ExecGateNotReleased
        );
        assert_eq!(
            classify(ExactTerminalState::Unknown, None),
            TerminalDecision::Unknown
        );
    }
}
