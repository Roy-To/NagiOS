use std::sync::{Arc, Mutex};

use nagi_files::{
    Actor as FilesActor, CapabilitySet, FileName, FilesService, InMemoryProvider, Location,
    ResourceId,
};
use nagi_history::activity::{
    ActionKind, ActivityAccessPolicy, ActivityDraft, Actor, ActorId, ActorKind, CheckpointId,
    EventId, EventResult, RevisionId, Timestamp,
};
use nagi_history::wayback::{
    CheckpointAccessPolicy, CheckpointDraft, CheckpointObject, CheckpointOrigin, CheckpointReason,
    CheckpointScope, SnapshotBackendRef,
};
use nagi_history::{ActivityContext, AppSessionId, NodeId, SurfaceId};
use nagi_home_search::actions::ActionAvailability;
use nagi_home_search::search::{SearchContext, SearchProvider, SearchQuery, SearchRequestId};
use nagi_home_search::{Locale, TypedAction};
use nagi_model::ObjectId;
use nagi_notes::{
    ActionError, ActionPolicyError, ActionPrincipal, ActivityEvent as NotesActivityEvent,
    ActivityKind as NotesActivityKind, ActivityOrigin, ActivitySink as NotesActivitySink, Block,
    BlockKind, NotesAction, NotesActionExecutor, NotesActionKind, NotesActionPolicy,
    NotesSearchProvider, Timestamp as NotesTimestamp,
};

use crate::{
    host_preview_capabilities, insert_preview_file, FilesObjectResolver, FilesSearchAdapter,
    HostUserNotesPolicy, IntegratedHost, NotesActivityBridge, NotesSearchAdapter,
    PreviewActivityPolicy, PreviewCheckpointPolicy, PreviewObjectIdAllocator,
};

fn create_note(host: &IntegratedHost, title: &str, text: &str) -> nagi_notes::NoteDocument {
    let session = host.notes.create_note(title).expect("create note");
    session
        .append_block(Block::new(
            host.notes.new_block_id(),
            BlockKind::Paragraph(text.to_owned()),
        ))
        .expect("append searchable paragraph");
    session.flush().expect("persist note revision")
}

#[test]
fn notes_feed_search_activity_and_wayback_with_canonical_ids() {
    let host = IntegratedHost::new().unwrap();
    let note = create_note(&host, "統合ノート Integration", "横断検索 searchable body");

    let japanese = host.search("横断検索", Locale::JaJp).unwrap();
    let note_result = japanese
        .results
        .iter()
        .find(|result| result.provider_id.as_str() == "notes")
        .expect("real Notes provider result");
    assert!(!note_result.is_fixture);
    assert_eq!(
        note_result.identity,
        nagi_home_search::SearchIdentity::Object(note.id)
    );
    assert!(matches!(
        note_result.action,
        Some(TypedAction::OpenObject { object_id, app_id })
            if object_id == note.id && app_id == nagi_notes::APP_ID
    ));
    assert_eq!(
        note_result.action_availability,
        Some(ActionAvailability::HostPreviewOnly)
    );
    let body_search = host.search("searchable body", Locale::EnUs).unwrap();
    assert!(body_search
        .results
        .iter()
        .any(|result| result.provider_id.as_str() == "notes"));
    assert!(body_search
        .results
        .iter()
        .all(|result| { result.provider_id.as_str() == "notes" }));

    let reference = host
        .history
        .note_snapshot_ref(note.id, note.revision)
        .unwrap()
        .expect("saved Notes revision has a Wayback backend reference");
    let snapshot = host
        .history
        .load_note_snapshot(reference)
        .unwrap()
        .expect("backend reference resolves to persisted note revision");
    assert_eq!(snapshot.id, note.id);
    assert_eq!(snapshot.revision, note.revision);

    let event_search = host.search("項目を作成", Locale::JaJp).unwrap();
    assert!(event_search.results.iter().any(|result| {
        result.provider_id.as_str() == "activity"
            && matches!(result.action, Some(TypedAction::OpenActivityEvent { .. }))
    }));
    let checkpoint_search = host.search("checkpoint", Locale::EnUs).unwrap();
    assert!(checkpoint_search.results.iter().any(|result| {
        result.provider_id.as_str() == "wayback"
            && matches!(result.action, Some(TypedAction::OpenCheckpoint { .. }))
    }));

    let (_, checkpoints, revisions) = host.history.counts().unwrap();
    assert_eq!(checkpoints, 1);
    assert_eq!(revisions, 1);
    assert_eq!(host.notes.activity_failures(), 0);
}

#[test]
fn files_search_and_operations_use_real_service_and_share_activity() {
    let host = IntegratedHost::new().unwrap();
    let resource_id = {
        let mut files = host.files.lock().unwrap();
        insert_preview_file(
            files.provider_mut(),
            "Quarterly invoice.txt",
            b"memory sandbox content",
        )
        .unwrap()
    };
    let file_search = host.search("invoice", Locale::EnUs).unwrap();
    let result = file_search
        .results
        .iter()
        .find(|result| result.provider_id.as_str() == "files")
        .expect("real Files provider result");
    assert!(!result.is_fixture);
    let object_id = match &result.action {
        Some(TypedAction::OpenObject { object_id, app_id }) => {
            assert_eq!(*app_id, crate::FILES_APP_ID);
            *object_id
        }
        other => panic!("expected typed Files open action, got {other:?}"),
    };
    assert_eq!(
        host.files_objects.resource_id(object_id).unwrap(),
        Some(resource_id)
    );

    let before = host.history.counts().unwrap();
    let operation = {
        let mut files = host.files.lock().unwrap();
        files
            .create_folder(
                &Location::root(),
                &FileName::parse("Ledger entry").unwrap(),
                FilesActor::User,
            )
            .unwrap()
            .1
    };
    assert!(crate::operation_completed(&operation));
    assert!(matches!(
        operation.checkpoint,
        nagi_files::HookStatus::Failed(_)
    ));
    let after = host.history.counts().unwrap();
    assert_eq!(after.0, before.0 + 1);
    assert_eq!(after.1, before.1);
    let activity_search = host.search("Created", Locale::EnUs).unwrap();
    assert!(activity_search.results.iter().any(|result| {
        result.provider_id.as_str() == "activity"
            && matches!(result.action, Some(TypedAction::OpenActivityEvent { .. }))
    }));
}

#[test]
fn files_read_authorized_rename_captures_a_real_wayback_snapshot() {
    let host = IntegratedHost::new().unwrap();
    let original_location = Location::parse("Before rename.txt").unwrap();
    {
        let mut files = host.files.lock().unwrap();
        insert_preview_file(
            files.provider_mut(),
            original_location.as_str(),
            b"before bytes",
        )
        .unwrap();
    }
    let (result, operation) = {
        let mut files = host.files.lock().unwrap();
        let original = files.metadata(&original_location).unwrap();
        files
            .rename(
                &original,
                &FileName::parse("After rename.txt").unwrap(),
                FilesActor::User,
            )
            .unwrap()
    };

    assert_eq!(result.name.as_str(), "After rename.txt");
    let checkpoint_id = match operation.checkpoint {
        nagi_files::HookStatus::Created(reference) => {
            assert!(reference.starts_with("checkpoint:"));
            host.history
                .file_checkpoint_for_transaction(operation.transaction_id.0)
                .unwrap()
                .expect("Files Activity event links its pre-operation checkpoint")
        }
        other => panic!("expected an actual file checkpoint, got {other:?}"),
    };
    let snapshots = host
        .history
        .file_snapshot_for_transaction(operation.transaction_id.0)
        .unwrap()
        .expect("Wayback backend reference resolves to a real file snapshot");
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].location, "Before rename.txt");
    assert_eq!(snapshots[0].contents, b"before bytes");
    assert!(host
        .search("before bytes", Locale::EnUs)
        .unwrap()
        .results
        .is_empty());
    assert!(matches!(
        host.search("checkpoint", Locale::EnUs)
            .unwrap()
            .results
            .iter()
            .find(|result| matches!(result.action, Some(TypedAction::OpenCheckpoint { .. })))
            .map(|result| &result.action),
        Some(Some(TypedAction::OpenCheckpoint { checkpoint_id: found })) if *found == checkpoint_id
    ));
}

#[test]
fn failed_files_operation_is_recorded_as_failure_without_success_wording() {
    let host = IntegratedHost::new().unwrap();
    let source = Location::parse("source.txt").unwrap();
    {
        let mut files = host.files.lock().unwrap();
        insert_preview_file(files.provider_mut(), source.as_str(), b"source").unwrap();
        insert_preview_file(files.provider_mut(), "occupied.txt", b"destination").unwrap();
    }

    let error = {
        let mut files = host.files.lock().unwrap();
        let entry = files.metadata(&source).unwrap();
        files
            .rename(
                &entry,
                &FileName::parse("occupied.txt").unwrap(),
                FilesActor::User,
            )
            .unwrap_err()
    };
    assert_eq!(error.kind, nagi_files::FilesErrorKind::Conflict);

    let failed_en = host.search("An operation failed", Locale::EnUs).unwrap();
    assert!(failed_en.results.iter().any(|result| {
        result.provider_id.as_str() == "activity"
            && result.title == "An operation failed"
            && result.subtitle.as_deref() == Some("failed")
            && matches!(result.action, Some(TypedAction::OpenActivityEvent { .. }))
    }));
    let failed_ja = host.search("操作に失敗しました", Locale::JaJp).unwrap();
    assert!(failed_ja.results.iter().any(|result| {
        result.provider_id.as_str() == "activity"
            && result.title == "操作に失敗しました"
            && result.subtitle.as_deref() == Some("失敗")
    }));
    assert!(host
        .search("Moved an item", Locale::EnUs)
        .unwrap()
        .results
        .iter()
        .all(|result| result.provider_id.as_str() != "activity"));
}

#[test]
fn files_resource_resolver_keeps_the_full_u128_identity() {
    let resolver = FilesObjectResolver::new(Arc::new(PreviewObjectIdAllocator::new()));
    let original = ResourceId(u128::MAX - 0x1234);
    let object = resolver.resolve(original).unwrap();
    assert_ne!(object.0, original.0 as u64);
    assert_eq!(resolver.resource_id(object).unwrap(), Some(original));
}

#[test]
fn home_registry_and_primary_search_paths_are_bilingual() {
    let host = IntegratedHost::new().unwrap();
    assert_eq!(
        host.home_apps(Locale::EnUs).unwrap(),
        vec!["Notes".to_owned(), "Files".to_owned()]
    );
    assert_eq!(
        host.home_apps(Locale::JaJp).unwrap(),
        vec!["ノート".to_owned(), "ファイル".to_owned()]
    );
    let note = create_note(&host, "Integration design", "二言語の検索対象");
    assert!(host
        .search("Integration", Locale::EnUs)
        .unwrap()
        .results
        .iter()
        .any(|result| result.identity == nagi_home_search::SearchIdentity::Object(note.id)));
    assert!(host
        .search("二言語", Locale::JaJp)
        .unwrap()
        .results
        .iter()
        .any(|result| result.identity == nagi_home_search::SearchIdentity::Object(note.id)));
}

struct DenyNotes;

impl NotesActionPolicy for DenyNotes {
    fn authorize(
        &self,
        _principal: ActionPrincipal,
        _action: NotesActionKind,
        _object_id: Option<ObjectId>,
    ) -> Result<(), ActionPolicyError> {
        Err(ActionPolicyError::Denied)
    }
}

#[test]
fn notes_search_policy_filters_before_candidate_text_is_returned() {
    let host = IntegratedHost::new().unwrap();
    create_note(&host, "Restricted note", "private searchable text");
    let provider = NotesSearchAdapter::new(Arc::clone(&host.notes), Arc::new(DenyNotes));
    let query = SearchQuery::new(
        "private",
        SearchRequestId(1),
        SearchContext::default(),
        host_preview_capabilities(),
        Locale::EnUs,
    );
    assert!(provider.search(&query).unwrap().is_empty());
}

#[test]
fn host_notes_actions_and_activity_reject_agents_without_delegation_provenance() {
    let host = IntegratedHost::new().unwrap();
    let search: Arc<dyn nagi_notes::SearchProvider> =
        Arc::new(NotesSearchProvider::for_app(Arc::clone(&host.notes)));
    let executor = NotesActionExecutor::new(
        Arc::clone(&host.notes),
        search,
        Arc::new(HostUserNotesPolicy),
    );
    let result = executor.execute(
        ActionPrincipal::Agent(AppSessionId(9)),
        NotesAction::Create {
            title: "agent must be denied".to_owned(),
        },
    );
    assert!(matches!(
        result,
        Err(ActionError::Policy(ActionPolicyError::Denied))
    ));
    assert!(host.notes.list_notes().unwrap().is_empty());

    let store: Arc<dyn nagi_notes::NoteStore> = Arc::new(nagi_notes::InMemoryNoteStore::new());
    let history = crate::HostHistory::new(store);
    let bridge = NotesActivityBridge(history.clone());
    let event = NotesActivityEvent {
        object_id: ObjectId(77),
        kind: NotesActivityKind::Edited,
        origin: ActivityOrigin::Agent(AppSessionId(9)),
        revision: 0,
        workspace_id: None,
        occurred_at: NotesTimestamp(1),
    };
    let error = bridge.record(event).unwrap_err();
    assert!(error.0.contains("delegated provenance"));
    assert_eq!(history.counts().unwrap().0, 0);
}

#[test]
fn activity_and_checkpoint_visibility_policies_are_user_scoped() {
    let timestamp = Timestamp::new(10, 0).unwrap();
    let user = Actor::new(ActorId(1), ActorKind::User);
    let other_user = Actor::new(ActorId(2), ActorKind::User);
    let context = ActivityContext {
        app_id: nagi_notes::APP_ID,
        app_session_id: AppSessionId(1),
        node_id: NodeId(1),
        surface_id: Some(SurfaceId(1)),
        workspace_id: None,
    };
    let event = ActivityDraft::new(timestamp, user, context, ActionKind::ObjectCreated)
        .with_result(EventResult::Succeeded)
        .with_target(ObjectId(10))
        .unwrap()
        .into_event(EventId::new(1).unwrap())
        .unwrap();
    assert!(PreviewActivityPolicy.can_read(user, &event));
    assert!(!PreviewActivityPolicy.can_read(other_user, &event));

    let checkpoint = CheckpointDraft::new(
        timestamp,
        user,
        context,
        CheckpointScope::Document,
        CheckpointOrigin::User,
        CheckpointReason::UserRequested,
        SnapshotBackendRef(1),
    )
    .with_object(CheckpointObject::new(
        ObjectId(10),
        RevisionId::new(1).unwrap(),
    ))
    .unwrap()
    .into_record(CheckpointId::new(1).unwrap())
    .unwrap();
    assert!(PreviewCheckpointPolicy.can_read(user, &checkpoint));
    assert!(!PreviewCheckpointPolicy.can_read(other_user, &checkpoint));
}

#[test]
fn files_search_respects_the_service_capability_authorizer() {
    let mut provider = InMemoryProvider::new();
    insert_preview_file(&mut provider, "Restricted file.txt", b"private").unwrap();
    let service = Arc::new(Mutex::new(FilesService::new(
        provider,
        CapabilitySet::new(),
    )));
    let resolver = FilesObjectResolver::new(Arc::new(PreviewObjectIdAllocator::new()));
    let provider = FilesSearchAdapter::new(service, resolver, Location::root());
    let query = SearchQuery::new(
        "Restricted",
        SearchRequestId(2),
        SearchContext::default(),
        host_preview_capabilities(),
        Locale::EnUs,
    );
    assert!(provider.search(&query).unwrap().is_empty());
}

#[test]
fn agent_files_mutation_fails_closed_without_delegation_provenance() {
    let host = IntegratedHost::new().unwrap();
    let before = host.history.counts().unwrap();
    let result = host.files.lock().unwrap().create_folder(
        &Location::root(),
        &FileName::parse("Agent must not bypass Activity").unwrap(),
        FilesActor::Agent,
    );
    assert!(matches!(
        result,
        Err(error) if error.kind == nagi_files::FilesErrorKind::ActivityFailure
    ));
    assert!(host
        .files
        .lock()
        .unwrap()
        .list(&Location::root())
        .unwrap()
        .is_empty());
    assert_eq!(host.history.counts().unwrap().0, before.0);
}
