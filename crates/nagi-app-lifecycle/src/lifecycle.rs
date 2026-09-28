use nagi_sdk::app_contract::{
    AppError, AppIdentity, LifecycleEvent, LifecycleEventKind, LifecycleMachine, LifecycleState,
};
use nagi_sdk::{AppId, AppSessionId, NodeId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleReason {
    LaunchRequested,
    CapabilitiesResolved,
    Ready,
    Activated,
    Backgrounded,
    Suspended,
    Resumed,
    TerminationRequested,
    ShutdownHookCompleted,
    Terminated,
    AbnormalTermination,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleFailureCategory {
    AbnormalTermination,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifecycleFailure {
    pub category: LifecycleFailureCategory,
    pub reason_code: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LifecycleObservation {
    pub app_id: AppId,
    pub session_id: AppSessionId,
    pub node_id: Option<NodeId>,
    pub sequence: Option<u64>,
    pub previous_state: LifecycleState,
    pub new_state: LifecycleState,
    pub reason: LifecycleReason,
    pub failure: Option<LifecycleFailure>,
}

/// Adds stable application identity to the canonical SDK lifecycle machine.
/// Transition legality and capability-resolution gates remain owned by the SDK.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedApplication<'a> {
    identity: AppIdentity<'a>,
    machine: LifecycleMachine,
}

impl<'a> ManagedApplication<'a> {
    pub fn new(identity: AppIdentity<'a>) -> Result<Self, AppError> {
        identity.validate()?;
        Ok(Self {
            identity,
            machine: LifecycleMachine::new(),
        })
    }

    pub const fn app_id(&self) -> AppId {
        self.identity.app_id()
    }

    pub const fn state(&self) -> LifecycleState {
        self.machine.state()
    }

    pub fn apply(&mut self, event: LifecycleEvent) -> Result<LifecycleObservation, AppError> {
        let reason = lifecycle_reason(event.kind);
        let failure = match event.kind {
            LifecycleEventKind::AbnormalTermination { reason_code } => Some(LifecycleFailure {
                category: LifecycleFailureCategory::AbnormalTermination,
                reason_code,
            }),
            _ => None,
        };
        let transition = self.machine.apply(event)?;
        Ok(LifecycleObservation {
            app_id: self.identity.app_id(),
            session_id: event.session_id,
            node_id: event.node_id,
            sequence: event.sequence,
            previous_state: transition.previous,
            new_state: transition.current,
            reason,
            failure,
        })
    }
}

fn lifecycle_reason(event: LifecycleEventKind) -> LifecycleReason {
    match event {
        LifecycleEventKind::LaunchRequested => LifecycleReason::LaunchRequested,
        LifecycleEventKind::CapabilitiesResolved(_) => LifecycleReason::CapabilitiesResolved,
        LifecycleEventKind::Ready => LifecycleReason::Ready,
        LifecycleEventKind::Activate => LifecycleReason::Activated,
        LifecycleEventKind::Backgrounded => LifecycleReason::Backgrounded,
        LifecycleEventKind::Suspended { .. } => LifecycleReason::Suspended,
        LifecycleEventKind::Resumed { .. } => LifecycleReason::Resumed,
        LifecycleEventKind::TerminationRequested => LifecycleReason::TerminationRequested,
        LifecycleEventKind::ShutdownHookCompleted => LifecycleReason::ShutdownHookCompleted,
        LifecycleEventKind::Terminated => LifecycleReason::Terminated,
        LifecycleEventKind::AbnormalTermination { .. } => LifecycleReason::AbnormalTermination,
    }
}
