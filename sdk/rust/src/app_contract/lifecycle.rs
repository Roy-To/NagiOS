use crate::{AppSessionId, NodeId};

use super::{AppError, CapabilityDecision, ErrorCode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleState {
    Registered,
    Launching,
    Ready,
    Foreground,
    Background,
    Suspended,
    Terminating,
    Terminated,
    Crashed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResumeTarget {
    Foreground,
    Background,
}

/// Events are observations/requests; they are not stored as lifecycle state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleEventKind {
    LaunchRequested,
    CapabilitiesResolved(CapabilityDecision),
    Ready,
    Activate,
    Backgrounded,
    Suspended {
        checkpoint: Option<u64>,
    },
    Resumed {
        target: ResumeTarget,
        checkpoint: Option<u64>,
    },
    TerminationRequested,
    ShutdownHookCompleted,
    Terminated,
    AbnormalTermination {
        reason_code: u16,
    },
}

/// Carries logical session and optional execution-node context without
/// requiring a single session/process/node mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifecycleEvent {
    pub kind: LifecycleEventKind,
    pub session_id: AppSessionId,
    pub node_id: Option<NodeId>,
    pub sequence: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifecycleTransition {
    pub previous: LifecycleState,
    pub current: LifecycleState,
    pub event: LifecycleEvent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifecycleMachine {
    state: LifecycleState,
    capability_decision: Option<CapabilityDecision>,
    shutdown_hook_completed: bool,
    session_id: Option<AppSessionId>,
    last_sequence: Option<u64>,
}

impl Default for LifecycleMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl LifecycleMachine {
    pub const fn new() -> Self {
        Self {
            state: LifecycleState::Registered,
            capability_decision: None,
            shutdown_hook_completed: false,
            session_id: None,
            last_sequence: None,
        }
    }

    pub const fn state(&self) -> LifecycleState {
        self.state
    }

    pub const fn capability_decision(&self) -> Option<CapabilityDecision> {
        self.capability_decision
    }

    pub fn apply(&mut self, event: LifecycleEvent) -> Result<LifecycleTransition, AppError> {
        if self
            .session_id
            .is_some_and(|session_id| session_id != event.session_id)
            || self
                .last_sequence
                .is_some_and(|last| event.sequence.is_none_or(|sequence| sequence <= last))
        {
            return Err(AppError::new(ErrorCode::InvalidLifecycleTransition));
        }
        let previous = self.state;
        let (next, decision) = match (self.state, event.kind) {
            (LifecycleState::Registered, LifecycleEventKind::LaunchRequested) => {
                (LifecycleState::Launching, None)
            }
            (LifecycleState::Launching, LifecycleEventKind::CapabilitiesResolved(resolution))
                if self.capability_decision.is_none() =>
            {
                (LifecycleState::Launching, Some(resolution))
            }
            (LifecycleState::Launching, LifecycleEventKind::Ready) => {
                match self.capability_decision {
                    Some(CapabilityDecision::Resolved | CapabilityDecision::NotRequested) => {
                        (LifecycleState::Ready, self.capability_decision)
                    }
                    Some(CapabilityDecision::Denied) => {
                        return Err(AppError::new(ErrorCode::PermissionDenied));
                    }
                    None => return Err(AppError::new(ErrorCode::InvalidLifecycleTransition)),
                }
            }
            (LifecycleState::Ready, LifecycleEventKind::Activate)
            | (LifecycleState::Background, LifecycleEventKind::Activate) => {
                (LifecycleState::Foreground, self.capability_decision)
            }
            (LifecycleState::Foreground, LifecycleEventKind::Backgrounded) => {
                (LifecycleState::Background, self.capability_decision)
            }
            (
                LifecycleState::Foreground | LifecycleState::Background,
                LifecycleEventKind::Suspended { .. },
            ) => (LifecycleState::Suspended, self.capability_decision),
            (LifecycleState::Suspended, LifecycleEventKind::Resumed { target, .. }) => (
                match target {
                    ResumeTarget::Foreground => LifecycleState::Foreground,
                    ResumeTarget::Background => LifecycleState::Background,
                },
                self.capability_decision,
            ),
            (
                LifecycleState::Registered
                | LifecycleState::Launching
                | LifecycleState::Ready
                | LifecycleState::Foreground
                | LifecycleState::Background
                | LifecycleState::Suspended,
                LifecycleEventKind::TerminationRequested,
            ) => (LifecycleState::Terminating, self.capability_decision),
            (LifecycleState::Terminating, LifecycleEventKind::ShutdownHookCompleted)
                if !self.shutdown_hook_completed =>
            {
                (LifecycleState::Terminating, self.capability_decision)
            }
            (LifecycleState::Terminating, LifecycleEventKind::Terminated)
                if self.shutdown_hook_completed =>
            {
                (LifecycleState::Terminated, self.capability_decision)
            }
            (
                LifecycleState::Launching
                | LifecycleState::Ready
                | LifecycleState::Foreground
                | LifecycleState::Background
                | LifecycleState::Suspended
                | LifecycleState::Terminating,
                LifecycleEventKind::AbnormalTermination { .. },
            ) => (LifecycleState::Crashed, self.capability_decision),
            _ => return Err(AppError::new(ErrorCode::InvalidLifecycleTransition)),
        };

        self.state = next;
        self.capability_decision = decision;
        if self.session_id.is_none() {
            self.session_id = Some(event.session_id);
        }
        if event.sequence.is_some() {
            self.last_sequence = event.sequence;
        }
        if matches!(event.kind, LifecycleEventKind::TerminationRequested) {
            self.shutdown_hook_completed = false;
        } else if matches!(event.kind, LifecycleEventKind::ShutdownHookCompleted) {
            self.shutdown_hook_completed = true;
        }
        Ok(LifecycleTransition {
            previous,
            current: next,
            event,
        })
    }
}

/// App-provided graceful shutdown callback. The host invokes it after the
/// lifecycle machine accepts `TerminationRequested` and before `Terminated`.
pub trait GracefulShutdownHook {
    fn on_termination_requested(&mut self, event: LifecycleEvent) -> Result<(), AppError>;
}

/// Deliver a validated termination request to an app-owned shutdown hook.
pub fn invoke_graceful_shutdown(
    hook: &mut impl GracefulShutdownHook,
    event: LifecycleEvent,
) -> Result<(), AppError> {
    if event.kind != LifecycleEventKind::TerminationRequested {
        return Err(AppError::new(ErrorCode::InvalidLifecycleTransition));
    }
    hook.on_termination_requested(event)
}

#[cfg(test)]
mod tests {
    use super::{
        LifecycleEvent, LifecycleEventKind, LifecycleMachine, LifecycleState, ResumeTarget,
    };
    use crate::app_contract::{AppError, CapabilityDecision, ErrorCode};
    use crate::{AppSessionId, NodeId};

    fn event(kind: LifecycleEventKind) -> LifecycleEvent {
        LifecycleEvent {
            kind,
            session_id: AppSessionId(7),
            node_id: Some(NodeId(2)),
            sequence: None,
        }
    }

    #[test]
    fn valid_launch_foreground_suspend_resume_shutdown_flow() {
        let mut machine = LifecycleMachine::new();
        assert_eq!(machine.state(), LifecycleState::Registered);
        machine
            .apply(event(LifecycleEventKind::LaunchRequested))
            .unwrap();
        assert_eq!(machine.state(), LifecycleState::Launching);
        machine
            .apply(event(LifecycleEventKind::CapabilitiesResolved(
                CapabilityDecision::Resolved,
            )))
            .unwrap();
        machine.apply(event(LifecycleEventKind::Ready)).unwrap();
        machine.apply(event(LifecycleEventKind::Activate)).unwrap();
        machine
            .apply(event(LifecycleEventKind::Suspended {
                checkpoint: Some(4),
            }))
            .unwrap();
        machine
            .apply(event(LifecycleEventKind::Resumed {
                target: ResumeTarget::Foreground,
                checkpoint: Some(4),
            }))
            .unwrap();
        machine
            .apply(event(LifecycleEventKind::TerminationRequested))
            .unwrap();
        machine
            .apply(event(LifecycleEventKind::ShutdownHookCompleted))
            .unwrap();
        let transition = machine
            .apply(event(LifecycleEventKind::Terminated))
            .unwrap();
        assert_eq!(transition.previous, LifecycleState::Terminating);
        assert_eq!(machine.state(), LifecycleState::Terminated);
    }

    #[test]
    fn ready_requires_capability_resolution() {
        let mut machine = LifecycleMachine::new();
        machine
            .apply(event(LifecycleEventKind::LaunchRequested))
            .unwrap();
        assert_eq!(
            machine.apply(event(LifecycleEventKind::Ready)),
            Err(AppError::new(ErrorCode::InvalidLifecycleTransition))
        );
    }

    #[test]
    fn permission_denial_is_not_misreported_as_ready() {
        let mut machine = LifecycleMachine::new();
        machine
            .apply(event(LifecycleEventKind::LaunchRequested))
            .unwrap();
        machine
            .apply(event(LifecycleEventKind::CapabilitiesResolved(
                CapabilityDecision::Denied,
            )))
            .unwrap();
        assert_eq!(
            machine.apply(event(LifecycleEventKind::Ready)),
            Err(AppError::new(ErrorCode::PermissionDenied))
        );
        assert_eq!(machine.state(), LifecycleState::Launching);
    }

    #[test]
    fn abnormal_termination_is_distinct_and_terminal() {
        let mut machine = LifecycleMachine::new();
        machine
            .apply(event(LifecycleEventKind::LaunchRequested))
            .unwrap();
        machine
            .apply(event(LifecycleEventKind::AbnormalTermination {
                reason_code: 7,
            }))
            .unwrap();
        assert_eq!(machine.state(), LifecycleState::Crashed);
        assert_eq!(
            machine.apply(event(LifecycleEventKind::Ready)),
            Err(AppError::new(ErrorCode::InvalidLifecycleTransition))
        );
    }

    #[test]
    fn duplicate_or_out_of_order_event_is_rejected_without_mutation() {
        let mut machine = LifecycleMachine::new();
        assert_eq!(
            machine.apply(event(LifecycleEventKind::Activate)),
            Err(AppError::new(ErrorCode::InvalidLifecycleTransition))
        );
        assert_eq!(machine.state(), LifecycleState::Registered);
    }

    #[test]
    fn graceful_shutdown_hook_receives_context_and_propagates_app_failure() {
        struct Hook(Option<AppSessionId>);

        impl super::GracefulShutdownHook for Hook {
            fn on_termination_requested(&mut self, event: LifecycleEvent) -> Result<(), AppError> {
                self.0 = Some(event.session_id);
                Err(AppError::new(ErrorCode::StateUnavailable))
            }
        }

        let request = event(LifecycleEventKind::TerminationRequested);
        let mut hook = Hook(None);
        assert_eq!(
            super::invoke_graceful_shutdown(&mut hook, request),
            Err(AppError::new(ErrorCode::StateUnavailable))
        );
        assert_eq!(hook.0, Some(AppSessionId(7)));
        assert_eq!(
            super::invoke_graceful_shutdown(&mut hook, event(LifecycleEventKind::Ready)),
            Err(AppError::new(ErrorCode::InvalidLifecycleTransition))
        );
    }

    #[test]
    fn terminated_requires_shutdown_hook_completion() {
        let mut machine = LifecycleMachine::new();
        machine
            .apply(event(LifecycleEventKind::TerminationRequested))
            .unwrap();
        assert_eq!(
            machine.apply(event(LifecycleEventKind::Terminated)),
            Err(AppError::new(ErrorCode::InvalidLifecycleTransition))
        );
        assert_eq!(machine.state(), LifecycleState::Terminating);
        machine
            .apply(event(LifecycleEventKind::ShutdownHookCompleted))
            .unwrap();
        assert_eq!(
            machine
                .apply(event(LifecycleEventKind::Terminated))
                .unwrap()
                .current,
            LifecycleState::Terminated
        );
    }

    #[test]
    fn machine_binds_session_and_rejects_duplicate_or_reversed_sequences() {
        let mut machine = LifecycleMachine::new();
        let launch = LifecycleEvent {
            sequence: Some(5),
            ..event(LifecycleEventKind::LaunchRequested)
        };
        machine.apply(launch).unwrap();

        let other_session = LifecycleEvent {
            session_id: AppSessionId(8),
            sequence: Some(6),
            ..event(LifecycleEventKind::CapabilitiesResolved(
                CapabilityDecision::Resolved,
            ))
        };
        assert_eq!(
            machine.apply(other_session),
            Err(AppError::new(ErrorCode::InvalidLifecycleTransition))
        );
        let reversed = LifecycleEvent {
            sequence: Some(4),
            ..event(LifecycleEventKind::CapabilitiesResolved(
                CapabilityDecision::Resolved,
            ))
        };
        assert_eq!(
            machine.apply(reversed),
            Err(AppError::new(ErrorCode::InvalidLifecycleTransition))
        );
        let next = LifecycleEvent {
            sequence: Some(6),
            ..event(LifecycleEventKind::CapabilitiesResolved(
                CapabilityDecision::Resolved,
            ))
        };
        machine.apply(next).unwrap();
        assert_eq!(machine.state(), LifecycleState::Launching);
    }
}
