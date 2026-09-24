use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use nagi_model::{AppId, ObjectId, TransactionId, UserId};
use nagi_model::{AppSessionId, NodeId, WorkspaceId};

use crate::{
    decode_event, encode_event, ActionDraft, ActionOutcome, ActionType, ActivityAccessPolicy,
    ActivityEvent, ActivityEventKind, ActivityLedger, ActivityQuery, ActorFilter, ActorIdentity,
    ActorKind, ActorRef, ContentHash, EventContext, InMemoryActivityStore, MetadataValue,
    ObjectVersion, Reversibility, ReversibilityKind, RevertExecutionResult, RevertExecutor,
    RevertOutcome, SequentialIdSource, SnapshotDraft, SnapshotId, SnapshotScope, StateReference,
    StorageObjectId, StorageReference, TargetId, TargetKind, TargetRef, Timestamp,
    TransactionStatus,
};

type Ledger = ActivityLedger<InMemoryActivityStore, SequentialIdSource>;

fn ledger() -> Ledger {
    ActivityLedger::new(InMemoryActivityStore::new(), SequentialIdSource::new(7))
}

fn time(seconds: i64) -> Timestamp {
    Timestamp::from_unix_seconds(seconds)
}

fn user() -> ActorRef {
    ActorRef::user(UserId(3))
}

fn ai(delegated_for: Option<UserId>) -> ActorRef {
    ActorRef::ai(
        0xA1,
        Some(String::from("granite-test")),
        Some(AppId(77)),
        delegated_for,
    )
}

fn context(actor: ActorRef, seconds: i64) -> EventContext {
    EventContext::new(time(seconds), actor)
}

fn target(kind: TargetKind, id: u64) -> TargetRef {
    TargetRef::object(kind, ObjectId(id), None)
}

fn state(version: u128) -> StateReference {
    StateReference {
        content_hash: Some(ContentHash([version as u8; 32])),
        object_version: Some(ObjectVersion(version)),
        snapshot: None,
        delta: None,
        metadata: BTreeMap::new(),
    }
}

fn draft(action_type: ActionType, object: u64, reversible: ReversibilityKind) -> ActionDraft {
    ActionDraft {
        action_type,
        target: target(TargetKind::File, object),
        summary_key: String::from("activity.file.changed"),
        metadata: BTreeMap::new(),
        before: Some(state(10 + object as u128)),
        after: Some(state(20 + object as u128)),
        reversibility: Reversibility::new(reversible),
        related_snapshot: None,
        authorization: None,
        provenance: Default::default(),
    }
}

struct TestAccess;

impl ActivityAccessPolicy for TestAccess {
    type Error = &'static str;

    fn authorize_query(
        &self,
        _requester: &ActorRef,
        _query: &ActivityQuery,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn can_read_event(&self, _requester: &ActorRef, _event: &ActivityEvent) -> bool {
        true
    }
}

struct TransactionScopedAccess;

impl ActivityAccessPolicy for TransactionScopedAccess {
    type Error = &'static str;

    fn authorize_query(
        &self,
        _requester: &ActorRef,
        query: &ActivityQuery,
    ) -> Result<(), Self::Error> {
        query
            .transaction_id
            .map(|_| ())
            .ok_or("activity queries require a transaction scope")
    }

    fn can_read_event(&self, _requester: &ActorRef, event: &ActivityEvent) -> bool {
        event.context.transaction_id == Some(TransactionId(404))
    }
}

fn query_events(ledger: &Ledger, query: &ActivityQuery) -> Vec<ActivityEvent> {
    ledger.query(&user(), query, &TestAccess).unwrap()
}

#[test]
fn action_creation_has_stable_identity_actor_target_and_serialization() {
    let mut ledger = ledger();
    let action_id = ledger
        .record_action(
            context(ai(Some(UserId(3))), 10),
            draft(ActionType::FileMove, 21, ReversibilityKind::FullyReversible),
        )
        .unwrap();

    let action = ledger.get_action(action_id).unwrap();
    assert_eq!(action.action_id, action_id);
    assert_eq!(action.context.actor.kind, ActorKind::Ai);
    assert_eq!(action.context.actor.delegated_for, Some(UserId(3)));
    assert_eq!(action.target.id, TargetId::Object(ObjectId(21)));
    assert_eq!(action.action_type, ActionType::FileMove);

    let event = ledger.get_event(action.event_id).unwrap();
    let encoded = encode_event(&event).unwrap();
    assert_eq!(decode_event(&encoded).unwrap(), event);
}

#[test]
fn transaction_collects_multiple_actions_and_completes() {
    let mut ledger = ledger();
    let transaction_id = TransactionId(45);
    ledger
        .begin_transaction(
            transaction_id,
            context(ai(None), 20),
            Some(String::from("activity.organize")),
        )
        .unwrap();

    for object in [1, 2, 3] {
        let mut action_context = context(ai(None), 21 + object as i64);
        action_context.transaction_id = Some(transaction_id);
        ledger
            .record_action(
                action_context,
                draft(
                    ActionType::FileMove,
                    object,
                    ReversibilityKind::FullyReversible,
                ),
            )
            .unwrap();
    }
    ledger
        .complete_transaction(transaction_id, context(ai(None), 30))
        .unwrap();

    let transaction = ledger.transaction(transaction_id).unwrap();
    assert_eq!(transaction.status, TransactionStatus::Completed);
    assert_eq!(transaction.action_ids.len(), 3);
    assert_eq!(
        transaction.reversibility,
        ReversibilityKind::FullyReversible
    );
    assert_eq!(
        ledger
            .transaction_events(transaction_id, &user(), &TestAccess)
            .unwrap()
            .len(),
        5
    );
}

#[test]
fn failed_and_partially_completed_transactions_keep_their_outcomes() {
    let mut ledger = ledger();
    let failed_id = TransactionId(1);
    ledger
        .begin_transaction(failed_id, context(ai(None), 1), None)
        .unwrap();
    ledger
        .fail_transaction(
            failed_id,
            context(ai(None), 2),
            Some(String::from("io.failed")),
        )
        .unwrap();

    let partial_id = TransactionId(2);
    ledger
        .begin_transaction(partial_id, context(ai(None), 3), None)
        .unwrap();
    ledger
        .partially_complete_transaction(
            partial_id,
            context(ai(None), 4),
            Some(String::from("activity.partial")),
        )
        .unwrap();

    assert_eq!(
        ledger.transaction(failed_id).unwrap().status,
        TransactionStatus::Failed
    );
    assert_eq!(
        ledger.transaction(partial_id).unwrap().status,
        TransactionStatus::PartiallyCompleted
    );
}

#[test]
fn parent_child_correlation_and_causation_are_preserved() {
    let mut ledger = ledger();
    let request_id = ledger
        .record_action(
            context(user(), 1),
            ActionDraft::simple(
                ActionType::UserRequest,
                target(TargetKind::Directory, 90),
                "activity.request.organize",
                ReversibilityKind::Irreversible,
            ),
        )
        .unwrap();
    let request = ledger.get_action(request_id).unwrap();
    let mut child_context = context(ai(Some(UserId(3))), 2);
    child_context.parent_action_id = Some(request_id);
    child_context.causation_id = Some(request.event_id);
    child_context.correlation_id = Some(crate::CorrelationId(800));

    let child_id = ledger
        .record_action(
            child_context,
            draft(ActionType::FileMove, 1, ReversibilityKind::FullyReversible),
        )
        .unwrap();
    let child = ledger.get_action(child_id).unwrap();
    assert_eq!(child.context.parent_action_id, Some(request_id));
    assert_eq!(child.context.causation_id, Some(request.event_id));
    assert_eq!(
        child.context.correlation_id,
        Some(crate::CorrelationId(800))
    );
}

#[test]
fn ai_direct_and_delegated_actors_remain_distinct() {
    let mut ledger = ledger();
    let direct = ledger
        .record_action(
            context(ai(None), 1),
            draft(ActionType::FileMove, 1, ReversibilityKind::FullyReversible),
        )
        .unwrap();
    let delegated = ledger
        .record_action(
            context(ai(Some(UserId(3))), 2),
            draft(ActionType::FileMove, 2, ReversibilityKind::FullyReversible),
        )
        .unwrap();

    assert_eq!(
        ledger.get_action(direct).unwrap().context.actor.kind,
        ActorKind::Ai
    );
    assert_eq!(
        ledger
            .get_action(direct)
            .unwrap()
            .context
            .actor
            .delegated_for,
        None
    );
    assert_eq!(
        ledger
            .get_action(delegated)
            .unwrap()
            .context
            .actor
            .delegated_for,
        Some(UserId(3))
    );
}

struct ApplyRevert;

impl RevertExecutor for ApplyRevert {
    fn apply(
        &mut self,
        _request_id: crate::EventId,
        _action: &crate::ActionRecord,
        _snapshot: Option<&crate::SnapshotReference>,
    ) -> RevertExecutionResult {
        RevertExecutionResult::Applied
    }
}

#[test]
fn revert_is_appended_and_already_reverted_is_rejected_without_mutating_history() {
    let mut ledger = ledger();
    let action = ledger
        .record_action(
            context(ai(None), 1),
            draft(ActionType::FileMove, 4, ReversibilityKind::FullyReversible),
        )
        .unwrap();
    let mut executor = ApplyRevert;

    assert_eq!(
        ledger
            .request_revert(action, context(user(), 2), &mut executor)
            .unwrap(),
        RevertOutcome::Applied
    );
    assert_eq!(
        ledger
            .request_revert(action, context(user(), 3), &mut executor)
            .unwrap(),
        RevertOutcome::AlreadyReverted
    );
    let events = query_events(&ledger, &ActivityQuery::default());
    assert_eq!(events.len(), 5);
    assert!(events.iter().any(|event| matches!(
        event.kind,
        ActivityEventKind::ActionRevertRecorded {
            action_id,
            outcome: RevertOutcome::Applied,
            ..
        } if action_id == action
    )));
}

struct PartialRevert;

impl RevertExecutor for PartialRevert {
    fn apply(
        &mut self,
        _request_id: crate::EventId,
        _action: &crate::ActionRecord,
        _snapshot: Option<&crate::SnapshotReference>,
    ) -> RevertExecutionResult {
        RevertExecutionResult::PartiallyApplied
    }
}

#[test]
fn partial_revert_is_recorded_as_partial() {
    let mut ledger = ledger();
    let action = ledger
        .record_action(
            context(ai(None), 1),
            draft(
                ActionType::FileMove,
                5,
                ReversibilityKind::PartiallyReversible,
            ),
        )
        .unwrap();
    assert_eq!(
        ledger
            .request_revert(action, context(user(), 2), &mut PartialRevert)
            .unwrap(),
        RevertOutcome::PartiallyApplied
    );
}

fn snapshot_draft() -> SnapshotDraft {
    SnapshotDraft {
        scope: SnapshotScope::Target(target(TargetKind::File, 6)),
        parent_snapshot: None,
        reason_key: Some(String::from("activity.before-change")),
        related_transaction: None,
        storage: StorageReference {
            backend_id: String::from("nagi.snapshot.test"),
            object_id: StorageObjectId(600),
            revision: None,
        },
        retention: crate::RetentionClass::UserPinned,
        pinned: true,
    }
}

#[test]
fn snapshot_required_revert_waits_for_reference_then_uses_it() {
    let mut ledger = ledger();
    let missing_snapshot = SnapshotId(999);
    let mut action_draft = draft(
        ActionType::FileEdit,
        6,
        ReversibilityKind::ReversibleWithSnapshot,
    );
    action_draft.related_snapshot = Some(missing_snapshot);
    let action = ledger
        .record_action(context(ai(None), 1), action_draft)
        .unwrap();
    let mut executor = ApplyRevert;
    assert_eq!(
        ledger
            .request_revert(action, context(user(), 2), &mut executor)
            .unwrap(),
        RevertOutcome::RejectedMissingSnapshot
    );

    let snapshot_id = ledger
        .create_snapshot_reference(snapshot_draft(), context(ai(None), 3))
        .unwrap();
    let mut linked_draft = draft(
        ActionType::FileEdit,
        7,
        ReversibilityKind::ReversibleWithSnapshot,
    );
    linked_draft.related_snapshot = Some(snapshot_id);
    let linked_action = ledger
        .record_action(context(ai(None), 4), linked_draft)
        .unwrap();
    assert_eq!(
        ledger
            .request_revert(linked_action, context(user(), 5), &mut executor)
            .unwrap(),
        RevertOutcome::Applied
    );
    assert!(ledger.resolve_snapshot(snapshot_id).is_some());
}

#[test]
fn irreversible_revert_is_rejected_and_recorded_without_calling_executor() {
    let mut ledger = ledger();
    let action = ledger
        .record_action(
            context(ai(None), 1),
            draft(ActionType::FileDelete, 12, ReversibilityKind::Irreversible),
        )
        .unwrap();
    let mut executor = ApplyRevert;
    assert_eq!(
        ledger
            .request_revert(action, context(user(), 2), &mut executor)
            .unwrap(),
        RevertOutcome::RejectedIrreversible
    );
    let events = query_events(&ledger, &ActivityQuery::default());
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        ActivityEventKind::ActionRevertRecorded {
            outcome: RevertOutcome::RejectedIrreversible,
            ..
        }
    )));
}

#[test]
fn action_failure_is_an_additional_event() {
    let mut ledger = ledger();
    let action = ledger
        .record_action(
            context(ai(None), 1),
            draft(ActionType::FileMove, 8, ReversibilityKind::FullyReversible),
        )
        .unwrap();
    ledger
        .record_action_outcome(
            action,
            context(ai(None), 2),
            ActionOutcome::Failed,
            Some(String::from("storage.denied")),
        )
        .unwrap();
    assert_eq!(query_events(&ledger, &ActivityQuery::default()).len(), 2);
}

#[test]
fn query_filters_actor_app_time_type_target_transaction_and_reversibility() {
    let mut ledger = ledger();
    let transaction_id = TransactionId(88);
    ledger
        .begin_transaction(transaction_id, context(ai(None), 10), None)
        .unwrap();
    let mut reversible_context = context(ai(Some(UserId(3))), 11);
    reversible_context.transaction_id = Some(transaction_id);
    let reversible = ledger
        .record_action(
            reversible_context,
            draft(ActionType::FileMove, 11, ReversibilityKind::FullyReversible),
        )
        .unwrap();
    let mut irreversible_context = context(ActorRef::application(AppId(77)), 12);
    irreversible_context.transaction_id = Some(transaction_id);
    let irreversible = ledger
        .record_action(
            irreversible_context,
            draft(ActionType::FileDelete, 12, ReversibilityKind::Irreversible),
        )
        .unwrap();
    ledger
        .complete_transaction(transaction_id, context(ai(None), 13))
        .unwrap();

    let query = ActivityQuery {
        actor: Some(ActorFilter {
            kind: Some(ActorKind::Ai),
            identity: None,
            delegated_for: Some(UserId(3)),
        }),
        application_id: Some(AppId(77)),
        from: Some(time(11)),
        through: Some(time(11)),
        action_type: Some(ActionType::FileMove),
        target: Some(TargetId::Object(ObjectId(11))),
        transaction_id: Some(transaction_id),
        reversible_only: true,
        ..ActivityQuery::default()
    };
    let events = query_events(&ledger, &query);
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].kind,
        ActivityEventKind::ActionCreated(ref data) if data.action_id == reversible
    ));
    assert!(!events.iter().any(|event| matches!(
        event.kind,
        ActivityEventKind::ActionCreated(ref data) if data.action_id == irreversible
    )));

    let by_app = ActivityQuery {
        application_id: Some(AppId(77)),
        ..ActivityQuery::default()
    };
    assert!(query_events(&ledger, &by_app).iter().any(|event| matches!(
        event.kind,
        ActivityEventKind::ActionCreated(ref data) if data.action_id == irreversible
    )));
}

#[test]
fn events_are_enumerated_chronologically() {
    let mut ledger = ledger();
    ledger
        .record_action(
            context(user(), 9),
            draft(ActionType::FileMove, 1, ReversibilityKind::FullyReversible),
        )
        .unwrap();
    ledger
        .record_action(
            context(user(), 2),
            draft(ActionType::FileMove, 2, ReversibilityKind::FullyReversible),
        )
        .unwrap();
    let events = query_events(&ledger, &ActivityQuery::default());
    assert_eq!(events[0].context.timestamp, time(2));
    assert_eq!(events[1].context.timestamp, time(9));
}

#[derive(Default)]
struct FakeReverter {
    current_versions: BTreeMap<TargetId, ObjectVersion>,
}

impl RevertExecutor for FakeReverter {
    fn apply(
        &mut self,
        _request_id: crate::EventId,
        action: &crate::ActionRecord,
        _snapshot: Option<&crate::SnapshotReference>,
    ) -> RevertExecutionResult {
        let Some(before) = action
            .before
            .as_ref()
            .and_then(|state| state.object_version)
        else {
            return RevertExecutionResult::Failed;
        };
        self.current_versions.insert(action.target.id, before);
        RevertExecutionResult::Applied
    }
}

#[test]
fn in_memory_documents_organization_fixture_records_and_reverts_without_filesystem() {
    let mut ledger = ledger();
    let user_request = ledger
        .record_action(
            context(user(), 100),
            ActionDraft::simple(
                ActionType::UserRequest,
                target(TargetKind::Directory, 500),
                "activity.request.organize",
                ReversibilityKind::Irreversible,
            ),
        )
        .unwrap();
    let request_record = ledger.get_action(user_request).unwrap();

    let mut ai_context = context(ai(Some(UserId(3))), 101);
    ai_context.parent_action_id = Some(user_request);
    ai_context.causation_id = Some(request_record.event_id);
    ai_context.correlation_id = Some(crate::CorrelationId(1000));
    let high_level = ledger
        .record_action(
            ai_context,
            ActionDraft::simple(
                ActionType::AssistantPlan,
                target(TargetKind::Directory, 500),
                "activity.ai.organize-folder",
                ReversibilityKind::FullyReversible,
            ),
        )
        .unwrap();
    let high_level_event = ledger.get_action(high_level).unwrap().event_id;

    let transaction_id = TransactionId(5000);
    let mut start = context(ai(Some(UserId(3))), 102);
    start.parent_action_id = Some(high_level);
    start.causation_id = Some(high_level_event);
    start.correlation_id = Some(crate::CorrelationId(1000));
    ledger
        .begin_transaction(
            transaction_id,
            start,
            Some(String::from("activity.ai.organize-folder")),
        )
        .unwrap();

    let mut reverter = FakeReverter::default();
    let mut actions = Vec::new();
    for (index, action_type) in [
        ActionType::FileMove,
        ActionType::FileMove,
        ActionType::FileRename,
    ]
    .into_iter()
    .enumerate()
    {
        let object_id = 501 + index as u64;
        let mut action_context = context(ai(Some(UserId(3))), 103 + index as i64);
        action_context.transaction_id = Some(transaction_id);
        action_context.parent_action_id = Some(high_level);
        action_context.correlation_id = Some(crate::CorrelationId(1000));
        action_context.causation_id = Some(high_level_event);
        let mut action = draft(action_type, object_id, ReversibilityKind::FullyReversible);
        action.target.parent = Some(TargetId::Object(ObjectId(500)));
        let action_id = ledger.record_action(action_context, action).unwrap();
        let record = ledger.get_action(action_id).unwrap();
        reverter.current_versions.insert(
            record.target.id,
            record.after.as_ref().unwrap().object_version.unwrap(),
        );
        actions.push(action_id);
    }
    ledger
        .complete_transaction(transaction_id, context(ai(Some(UserId(3))), 110))
        .unwrap();

    let query = ActivityQuery {
        transaction_id: Some(transaction_id),
        ..ActivityQuery::default()
    };
    assert_eq!(query_events(&ledger, &query).len(), 5);
    assert_eq!(
        ledger.transaction(transaction_id).unwrap().status,
        TransactionStatus::Completed
    );

    for action_id in actions {
        assert_eq!(
            ledger
                .request_revert(action_id, context(user(), 111), &mut reverter)
                .unwrap(),
            RevertOutcome::Applied
        );
    }
    ledger
        .mark_reverted(transaction_id, context(user(), 112))
        .unwrap();

    let transaction = ledger.transaction(transaction_id).unwrap();
    assert_eq!(transaction.status, TransactionStatus::Reverted);
    assert!(transaction.action_ids.iter().all(|action_id| {
        let action = ledger.get_action(*action_id).unwrap();
        reverter.current_versions.get(&action.target.id)
            == action.before.as_ref().unwrap().object_version.as_ref()
    }));
    let revert_events = ledger
        .query(
            &user(),
            &ActivityQuery {
                transaction_id: Some(transaction_id),
                ..ActivityQuery::default()
            },
            &TestAccess,
        )
        .unwrap();
    assert_eq!(
        revert_events
            .iter()
            .filter(|event| matches!(event.kind, ActivityEventKind::ActionRevertRecorded { .. }))
            .count(),
        3
    );
}

#[test]
fn query_by_actor_identity_uses_logical_application_ids() {
    let mut ledger = ledger();
    ledger
        .record_action(
            context(ActorRef::application(AppId(42)), 1),
            draft(ActionType::FileMove, 1, ReversibilityKind::FullyReversible),
        )
        .unwrap();
    let query = ActivityQuery {
        actor: Some(ActorFilter {
            kind: Some(ActorKind::Application),
            identity: Some(ActorIdentity::Application(AppId(42))),
            delegated_for: None,
        }),
        ..ActivityQuery::default()
    };
    assert_eq!(query_events(&ledger, &query).len(), 1);
}

#[test]
fn query_can_scope_to_logical_node_session_and_workspace() {
    let mut ledger = ledger();
    let mut origin = context(ai(None), 1);
    origin.origin.node_id = Some(NodeId(4));
    origin.origin.app_session_id = Some(AppSessionId(8));
    origin.origin.workspace_id = Some(WorkspaceId(12));
    ledger
        .record_action(
            origin,
            draft(ActionType::FileMove, 1, ReversibilityKind::FullyReversible),
        )
        .unwrap();

    let query = ActivityQuery {
        node_id: Some(NodeId(4)),
        app_session_id: Some(AppSessionId(8)),
        workspace_id: Some(WorkspaceId(12)),
        ..ActivityQuery::default()
    };
    assert_eq!(query_events(&ledger, &query).len(), 1);
}

#[test]
fn read_policy_blocks_broad_access_and_filters_individual_events() {
    let mut ledger = ledger();
    let tx_id = TransactionId(404);
    let start = ledger
        .begin_transaction(tx_id, context(ai(None), 1), None)
        .unwrap();
    let mut action_context = context(ai(None), 2);
    action_context.transaction_id = Some(tx_id);
    let action_id = ledger
        .record_action(
            action_context,
            draft(ActionType::FileMove, 1, ReversibilityKind::FullyReversible),
        )
        .unwrap();
    ledger
        .complete_transaction(tx_id, context(ai(None), 3))
        .unwrap();

    let broad = ledger.query(&user(), &ActivityQuery::default(), &TransactionScopedAccess);
    assert!(matches!(broad, Err(crate::AuthorizedQueryError::Denied(_))));
    let events = ledger
        .transaction_events(tx_id, &user(), &TransactionScopedAccess)
        .unwrap();
    assert_eq!(events.len(), 3);
    assert!(ledger
        .read_event(start, &user(), &TransactionScopedAccess)
        .unwrap()
        .is_some());
    let standalone = ledger
        .record_action(
            context(user(), 4),
            ActionDraft::simple(
                ActionType::UserRequest,
                target(TargetKind::Directory, 1),
                "activity.request",
                ReversibilityKind::Irreversible,
            ),
        )
        .unwrap();
    let standalone_event = ledger.get_action(standalone).unwrap().event_id;
    assert!(ledger
        .read_event(standalone_event, &user(), &TransactionScopedAccess)
        .unwrap()
        .is_none());
    assert_eq!(ledger.transaction(tx_id).unwrap().action_ids, [action_id]);
}

#[test]
fn snapshot_is_linked_to_action_and_transaction_without_embedding_contents() {
    let mut ledger = ledger();
    let transaction_id = TransactionId(700);
    ledger
        .begin_transaction(transaction_id, context(ai(None), 1), None)
        .unwrap();
    let mut snapshot = snapshot_draft();
    snapshot.scope = SnapshotScope::Transaction(transaction_id);
    snapshot.related_transaction = Some(transaction_id);
    let mut snapshot_context = context(ai(None), 2);
    snapshot_context.transaction_id = Some(transaction_id);
    let snapshot_id = ledger
        .create_snapshot_reference(snapshot, snapshot_context)
        .unwrap();

    let mut action_context = context(ai(None), 3);
    action_context.transaction_id = Some(transaction_id);
    let mut action = draft(
        ActionType::FileEdit,
        6,
        ReversibilityKind::ReversibleWithSnapshot,
    );
    action.related_snapshot = Some(snapshot_id);
    action.before.as_mut().unwrap().snapshot = Some(snapshot_id);
    let action_id = ledger.record_action(action_context, action).unwrap();
    let action_record = ledger.get_action(action_id).unwrap();

    assert_eq!(action_record.related_snapshot, Some(snapshot_id));
    assert_eq!(action_record.before.unwrap().snapshot, Some(snapshot_id));
    assert_eq!(
        ledger
            .resolve_snapshot(snapshot_id)
            .unwrap()
            .related_transaction,
        Some(transaction_id)
    );
    assert_eq!(
        ledger.transaction(transaction_id).unwrap().snapshot_ids,
        [snapshot_id]
    );
}

#[test]
fn redaction_marker_hides_event_from_query_and_id_read() {
    let mut ledger = ledger();
    let action_id = ledger
        .record_action(
            context(user(), 1),
            draft(ActionType::FileMove, 14, ReversibilityKind::FullyReversible),
        )
        .unwrap();
    let event_id = ledger.get_action(action_id).unwrap().event_id;
    ledger
        .redact_event(
            event_id,
            context(user(), 2),
            String::from("privacy.request"),
        )
        .unwrap();

    assert!(ledger
        .read_event(event_id, &user(), &TestAccess)
        .unwrap()
        .is_none());
    assert!(!query_events(&ledger, &ActivityQuery::default())
        .iter()
        .any(|event| event.event_id == event_id));
}

#[test]
fn before_after_store_references_and_hashes_not_content() {
    let reference = StateReference {
        content_hash: Some(ContentHash([0xAA; 32])),
        object_version: None,
        snapshot: None,
        delta: None,
        metadata: BTreeMap::from([(String::from("size_bytes"), MetadataValue::Integer(23))]),
    };
    assert_eq!(reference.content_hash.unwrap().0.len(), 32);
    assert_eq!(reference.metadata.len(), 1);
}
