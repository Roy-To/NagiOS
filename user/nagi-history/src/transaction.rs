//! Semantic Undo is planned and executed separately from the Activity ledger.

use crate::activity::RevisionId;
use crate::activity::{
    ActionKind, ActivityDraft, ActivityError, ActivityEvent, ActivitySink, Actor, CausalParent,
    EventId, EventResult, FailureCode, Provenance, Reversibility, TransactionId, UndoDescriptor,
};
use crate::{ActivityContext, ObjectId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UndoPlanId(pub u64);

#[derive(Debug, Eq, PartialEq)]
pub struct UndoPlan {
    id: UndoPlanId,
    source_event: EventId,
    transaction: Option<TransactionId>,
    correlation: Option<crate::activity::CorrelationId>,
    context: ActivityContext,
    object: ObjectId,
    descriptor: UndoDescriptor,
    conditional: bool,
}

impl UndoPlan {
    pub const fn id(&self) -> UndoPlanId {
        self.id
    }

    pub const fn source_event(&self) -> EventId {
        self.source_event
    }

    pub const fn transaction(&self) -> Option<TransactionId> {
        self.transaction
    }

    pub const fn object(&self) -> ObjectId {
        self.object
    }

    pub const fn descriptor(&self) -> UndoDescriptor {
        self.descriptor
    }

    pub const fn is_conditional(&self) -> bool {
        self.conditional
    }

    pub fn confirm(self, preconditions_checked: bool) -> Result<ConfirmedUndoPlan, UndoError> {
        if self.conditional && !preconditions_checked {
            return Err(UndoError::PreconditionsNotChecked);
        }
        Ok(ConfirmedUndoPlan { plan: self })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct ConfirmedUndoPlan {
    plan: UndoPlan,
}

impl ConfirmedUndoPlan {
    pub const fn plan(&self) -> &UndoPlan {
        &self.plan
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UndoError {
    NotReversible,
    SourceActionDidNotSucceed,
    RequiresSingleObject,
    PreconditionsNotChecked,
    Capacity,
    Activity(ActivityError),
}

pub fn plan_undo(event: &ActivityEvent, id: UndoPlanId) -> Result<UndoPlan, UndoError> {
    let (descriptor, conditional) = match event.reversibility() {
        Reversibility::Reversible(descriptor) => (descriptor, false),
        Reversibility::ConditionallyReversible(descriptor) => (descriptor, true),
        Reversibility::Irreversible | Reversibility::Unknown => {
            return Err(UndoError::NotReversible)
        }
    };
    if event.result() != EventResult::Succeeded {
        return Err(UndoError::SourceActionDidNotSucceed);
    }
    let mut targets = event.targets();
    let object = targets.next().ok_or(UndoError::RequiresSingleObject)?;
    if targets.next().is_some() {
        return Err(UndoError::RequiresSingleObject);
    }
    Ok(UndoPlan {
        id,
        source_event: event.id(),
        transaction: event.transaction_id(),
        correlation: event.correlation_id(),
        context: event.context(),
        object,
        descriptor,
        conditional,
    })
}

pub trait UndoPolicy {
    fn authorize(&self, actor: Actor, plan: &UndoPlan) -> Result<(), FailureCode>;
}

pub trait UndoBackend {
    fn current_revision(&self, object: ObjectId) -> Option<RevisionId>;

    /// Must compare the expected revision again at mutation time to protect
    /// against a state change between precondition lookup and execution.
    fn apply_inverse(
        &mut self,
        object: ObjectId,
        descriptor: UndoDescriptor,
    ) -> Result<(), FailureCode>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UndoOutcome {
    event_id: EventId,
    result: EventResult,
}

impl UndoOutcome {
    pub const fn event_id(self) -> EventId {
        self.event_id
    }

    pub const fn result(self) -> EventResult {
        self.result
    }
}

/// Execute only a separately confirmed plan. Policy and stale-state checks run
/// before mutation; every completed or denied attempt appends a result event.
pub fn execute_undo(
    confirmation: ConfirmedUndoPlan,
    actor: Actor,
    provenance: Provenance,
    occurred_at: crate::activity::Timestamp,
    policy: &dyn UndoPolicy,
    backend: &mut dyn UndoBackend,
    activity: &mut dyn ActivitySink,
) -> Result<UndoOutcome, UndoError> {
    let plan = confirmation.plan;
    if !activity.can_append() {
        return Err(UndoError::Capacity);
    }
    ActivityDraft::new(occurred_at, actor, plan.context, ActionKind::UndoApplied)
        .with_provenance(provenance)
        .with_target(plan.object)
        .map_err(UndoError::Activity)?
        .with_result(EventResult::Pending)
        .validate()
        .map_err(UndoError::Activity)?;
    let result = match policy.authorize(actor, &plan) {
        Err(failure) => EventResult::Failed(failure),
        Ok(()) => match backend.current_revision(plan.object) {
            None => EventResult::Failed(FailureCode::MissingObject),
            Some(current) if current != plan.descriptor.expected_current() => {
                EventResult::Failed(FailureCode::StaleState)
            }
            Some(_) => match backend.apply_inverse(plan.object, plan.descriptor) {
                Ok(()) => EventResult::Succeeded,
                Err(failure) => EventResult::Failed(failure),
            },
        },
    };
    let action = if result == EventResult::Succeeded {
        ActionKind::UndoApplied
    } else {
        ActionKind::OperationFailed
    };
    let mut event = ActivityDraft::new(occurred_at, actor, plan.context, action)
        .with_provenance(provenance)
        .with_target(plan.object)
        .map_err(UndoError::Activity)?
        .with_parent(CausalParent(plan.source_event))
        .with_result(result);
    if let Some(transaction) = plan.transaction {
        event = event.with_transaction(transaction);
    }
    if let Some(correlation) = plan.correlation {
        event = event.with_correlation(correlation);
    }
    let event_id = activity.append(event).map_err(UndoError::Activity)?;
    Ok(UndoOutcome { event_id, result })
}

#[cfg(any(test, feature = "sandbox"))]
pub mod sandbox {
    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct SandboxRevision {
        object: ObjectId,
        revision: RevisionId,
    }

    /// Small, deterministic backend for exercising Undo semantics off-target.
    pub struct InMemoryUndoSandbox {
        revisions: [Option<SandboxRevision>; 16],
        fail_next: Option<FailureCode>,
    }

    impl InMemoryUndoSandbox {
        pub const fn new() -> Self {
            Self {
                revisions: [None; 16],
                fail_next: None,
            }
        }

        pub fn insert(
            &mut self,
            object: ObjectId,
            revision: RevisionId,
        ) -> Result<(), FailureCode> {
            if let Some(slot) = self
                .revisions
                .iter_mut()
                .find(|slot| slot.is_some_and(|entry| entry.object == object))
            {
                *slot = Some(SandboxRevision { object, revision });
                return Ok(());
            }
            let slot = self
                .revisions
                .iter_mut()
                .find(|slot| slot.is_none())
                .ok_or(FailureCode::Capacity)?;
            *slot = Some(SandboxRevision { object, revision });
            Ok(())
        }

        pub fn fail_next(&mut self, failure: FailureCode) {
            self.fail_next = Some(failure);
        }
    }

    impl Default for InMemoryUndoSandbox {
        fn default() -> Self {
            Self::new()
        }
    }

    impl UndoBackend for InMemoryUndoSandbox {
        fn current_revision(&self, object: ObjectId) -> Option<RevisionId> {
            self.revisions
                .iter()
                .filter_map(|entry| *entry)
                .find(|entry| entry.object == object)
                .map(|entry| entry.revision)
        }

        fn apply_inverse(
            &mut self,
            object: ObjectId,
            descriptor: UndoDescriptor,
        ) -> Result<(), FailureCode> {
            if let Some(failure) = self.fail_next.take() {
                return Err(failure);
            }
            let entry = self
                .revisions
                .iter_mut()
                .filter_map(|entry| entry.as_mut())
                .find(|entry| entry.object == object)
                .ok_or(FailureCode::MissingObject)?;
            if entry.revision != descriptor.expected_current() {
                return Err(FailureCode::StaleState);
            }
            entry.revision = descriptor.target_revision();
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::activity::{
            ActionKind, ActivityLedger, ActivityQuery, ActorId, ActorKind, CorrelationId,
            EventResult, Reversibility, Timestamp,
        };
        use crate::{ActivityContext, AppId, AppSessionId, NodeId, WorkspaceId};

        const CONTEXT: ActivityContext = ActivityContext {
            app_id: AppId(1),
            app_session_id: AppSessionId(2),
            node_id: NodeId(3),
            surface_id: None,
            workspace_id: Some(WorkspaceId(4)),
        };
        const USER: Actor = Actor::new(ActorId(1), ActorKind::User);
        const DIRECT: Provenance = Provenance::Direct {
            originating_intent: None,
        };

        struct Allow;
        impl UndoPolicy for Allow {
            fn authorize(&self, _actor: Actor, _plan: &UndoPlan) -> Result<(), FailureCode> {
                Ok(())
            }
        }

        fn time(seconds: i64) -> Timestamp {
            Timestamp::new(seconds, 0).unwrap()
        }

        fn reversible_event(activity: &mut ActivityLedger, current: u64) -> EventId {
            activity
                .append(
                    ActivityDraft::new(time(1), USER, CONTEXT, ActionKind::ObjectChanged)
                        .with_result(EventResult::Succeeded)
                        .with_target(ObjectId(7))
                        .unwrap()
                        .with_transaction(TransactionId(9))
                        .with_correlation(CorrelationId(10))
                        .with_reversibility(Reversibility::Reversible(UndoDescriptor::new(
                            crate::activity::InverseKind::RestoreObjectRevision,
                            RevisionId(current),
                            RevisionId(current - 1),
                        ))),
                )
                .unwrap()
        }

        #[test]
        fn undo_success_applies_inverse_and_records_causal_transaction_activity() {
            let mut activity = ActivityLedger::new();
            let source = reversible_event(&mut activity, 2);
            let plan = plan_undo(activity.get(source).unwrap(), UndoPlanId(1)).unwrap();
            let mut sandbox = InMemoryUndoSandbox::new();
            sandbox.insert(ObjectId(7), RevisionId(2)).unwrap();
            let result = execute_undo(
                plan.confirm(true).unwrap(),
                USER,
                DIRECT,
                time(2),
                &Allow,
                &mut sandbox,
                &mut activity,
            )
            .unwrap();
            assert_eq!(result.result(), EventResult::Succeeded);
            assert_eq!(sandbox.current_revision(ObjectId(7)), Some(RevisionId(1)));
            let event = activity.get(result.event_id()).unwrap();
            assert_eq!(event.action(), ActionKind::UndoApplied);
            assert_eq!(event.causal_parent(), Some(CausalParent(source)));
            assert_eq!(event.transaction_id(), Some(TransactionId(9)));
            assert_eq!(event.correlation_id(), Some(CorrelationId(10)));
        }

        #[test]
        fn undo_stale_state_fails_without_mutation_and_is_recorded_as_failure() {
            let mut activity = ActivityLedger::new();
            let source = reversible_event(&mut activity, 2);
            let plan = plan_undo(activity.get(source).unwrap(), UndoPlanId(2)).unwrap();
            let mut sandbox = InMemoryUndoSandbox::new();
            sandbox.insert(ObjectId(7), RevisionId(3)).unwrap();
            let result = execute_undo(
                plan.confirm(true).unwrap(),
                USER,
                DIRECT,
                time(3),
                &Allow,
                &mut sandbox,
                &mut activity,
            )
            .unwrap();
            assert_eq!(
                result.result(),
                EventResult::Failed(FailureCode::StaleState)
            );
            assert_eq!(sandbox.current_revision(ObjectId(7)), Some(RevisionId(3)));
            assert_eq!(
                activity.get(result.event_id()).unwrap().action(),
                ActionKind::OperationFailed
            );
        }

        #[test]
        fn irreversible_unknown_failed_and_conditional_actions_do_not_offer_unsafe_undo() {
            let mut activity = ActivityLedger::new();
            let irreversible = activity
                .append(
                    ActivityDraft::new(time(1), USER, CONTEXT, ActionKind::ObjectDeleted)
                        .with_reversibility(Reversibility::Irreversible),
                )
                .unwrap();
            assert_eq!(
                plan_undo(activity.get(irreversible).unwrap(), UndoPlanId(3)),
                Err(UndoError::NotReversible)
            );
            let failed = activity
                .append(
                    ActivityDraft::new(time(2), USER, CONTEXT, ActionKind::OperationFailed)
                        .with_result(EventResult::Failed(FailureCode::BackendFailure))
                        .with_reversibility(Reversibility::Reversible(UndoDescriptor::new(
                            crate::activity::InverseKind::RestoreObjectRevision,
                            RevisionId(2),
                            RevisionId(1),
                        ))),
                )
                .unwrap();
            assert_eq!(
                plan_undo(activity.get(failed).unwrap(), UndoPlanId(4)),
                Err(UndoError::SourceActionDidNotSucceed)
            );
            let conditional = activity
                .append(
                    ActivityDraft::new(time(3), USER, CONTEXT, ActionKind::ObjectChanged)
                        .with_result(EventResult::Succeeded)
                        .with_target(ObjectId(7))
                        .unwrap()
                        .with_reversibility(Reversibility::ConditionallyReversible(
                            UndoDescriptor::new(
                                crate::activity::InverseKind::RestoreObjectRevision,
                                RevisionId(2),
                                RevisionId(1),
                            ),
                        )),
                )
                .unwrap();
            let plan = plan_undo(activity.get(conditional).unwrap(), UndoPlanId(5)).unwrap();
            assert!(plan.is_conditional());
            assert_eq!(plan.confirm(false), Err(UndoError::PreconditionsNotChecked));
            let plan = plan_undo(activity.get(conditional).unwrap(), UndoPlanId(6)).unwrap();
            assert!(plan.confirm(true).is_ok());
        }

        #[test]
        fn undo_query_can_find_original_and_inverse_in_one_transaction() {
            let mut activity = ActivityLedger::new();
            let source = reversible_event(&mut activity, 2);
            let plan = plan_undo(activity.get(source).unwrap(), UndoPlanId(6)).unwrap();
            let mut sandbox = InMemoryUndoSandbox::new();
            sandbox.insert(ObjectId(7), RevisionId(2)).unwrap();
            execute_undo(
                plan.confirm(true).unwrap(),
                USER,
                DIRECT,
                time(2),
                &Allow,
                &mut sandbox,
                &mut activity,
            )
            .unwrap();
            assert_eq!(
                activity
                    .query(ActivityQuery::default().transaction(TransactionId(9)))
                    .count(),
                2
            );
        }
    }
}
