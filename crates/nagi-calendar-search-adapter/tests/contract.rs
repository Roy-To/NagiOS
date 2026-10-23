use nagi_calendar_core::{model::*, time::Instant, AppId, ObjectId};
use nagi_calendar_search_adapter::{CalendarSearchAdapter, Error, EventObjects};
use nagi_search_core::{
    nagi_search::*, DocumentRevision, IngestOutcome, MemorySnapshotBackend, ProviderAuthority,
    ProviderScope, RemoveOutcome, ScopedQuery, SearchCore, SearchCoreError,
};
use std::{cell::Cell, rc::Rc};
const PROVIDER: AppId = AppId(41);
struct Objects;
impl EventObjects for Objects {
    fn object_id(&self, id: &EventId) -> Option<ObjectId> {
        (id.as_str() == "event_1").then_some(ObjectId(17))
    }
}
#[derive(Clone)]
struct Authority(Rc<Cell<bool>>);
impl ProviderAuthority for Authority {
    fn may_publish(&self, p: AppId) -> bool {
        p == PROVIDER && self.0.get()
    }
}
#[derive(Clone)]
struct Policy(Rc<Cell<bool>>);
impl VisibilityFilter for Policy {
    fn can_read_object(&self, _: AccessContext, _: &MetadataRecord) -> bool {
        self.0.get()
    }
    fn can_read_workspace(&self, _: AccessContext, _: &Workspace) -> bool {
        false
    }
}
type Core = SearchCore<MemorySnapshotBackend, MemorySnapshotBackend, Policy, Authority>;
fn fixture() -> (
    CalendarSearchAdapter<Objects>,
    Core,
    Policy,
    Authority,
    MemorySnapshotBackend,
) {
    let policy = Policy(Rc::new(Cell::new(true)));
    let authority = Authority(Rc::new(Cell::new(true)));
    let metadata = MemorySnapshotBackend::new();
    let mut core = SearchCore::open(
        metadata.clone(),
        MemorySnapshotBackend::new(),
        policy.clone(),
        authority.clone(),
    )
    .unwrap();
    core.register_provider(PROVIDER).unwrap();
    (
        CalendarSearchAdapter::new(PROVIDER, Objects),
        core,
        policy,
        authority,
        metadata,
    )
}
fn event() -> Event {
    let mut draft = EventDraft::new(
        CalendarId::new("cal_1").unwrap(),
        "Planning",
        Schedule::Instant(Interval::new(Instant(0), Instant(10)).unwrap()),
    );
    draft.location = "Meeting room".into();
    draft.participants = vec!["Ada".into()];
    draft.description = "private notes never projected".into();
    Event {
        id: EventId::new("event_1").unwrap(),
        revision: EventRevision::new(1).unwrap(),
        created_at: Instant(0),
        modified_at: Instant(0),
        draft,
    }
}
fn get(core: &Core) -> Option<nagi_search_core::ProviderHit> {
    core.get(
        AccessContext::anonymous(),
        &ProviderScope::only([PROVIDER]),
        ObjectId(17),
    )
    .unwrap()
}
#[test]
fn canonical_types_and_projection() {
    let (adapter, mut core, _, _, _) = fixture();
    let mut e = event();
    // Compiles only when Calendar and Search use the same canonical IDs.
    e.draft
        .resources
        .push(nagi_search_core::nagi_model::ObjectId(22));
    assert_eq!(adapter.publish(&mut core, &e), Ok(IngestOutcome::Applied));
    let hit = get(&core).unwrap();
    assert_eq!(hit.hit.record.kind, ObjectKind::ApplicationData);
    assert_eq!(hit.hit.record.title, "Planning");
    assert_eq!(hit.hit.record.location.as_deref(), Some("Meeting room"));
    assert_eq!(hit.hit.record.tags, ["Ada"]);
    assert_eq!(hit.hit.record.source_app, Some(PROVIDER));
    assert_eq!(hit.hit.record.visibility, VisibilityScope::Private);
    assert!(hit.hit.record.attributes.is_empty());
    assert!(!format!("{hit:?}").contains("private notes"));
    let query = SearchQuery {
        tags_any: vec!["Ada".into()],
        ..Default::default()
    };
    assert_eq!(
        core.search(
            AccessContext::anonymous(),
            &ScopedQuery::only(query, [PROVIDER])
        )
        .unwrap()
        .objects
        .len(),
        1
    );
}
#[test]
fn update_delete_stale_conflict_and_replay() {
    let (adapter, mut core, _, _, _) = fixture();
    let mut e = event();
    adapter.publish(&mut core, &e).unwrap();
    assert_eq!(adapter.publish(&mut core, &e), Ok(IngestOutcome::Unchanged));
    e.draft.title = "changed".into();
    assert_eq!(
        adapter.publish(&mut core, &e),
        Err(Error::Search(SearchCoreError::RevisionConflict))
    );
    e.revision = EventRevision::new(2).unwrap();
    adapter.publish(&mut core, &e).unwrap();
    assert_eq!(get(&core).unwrap().hit.record.title, "changed");
    assert_eq!(
        adapter.publish(&mut core, &event()),
        Err(Error::Search(SearchCoreError::StaleRevision))
    );
    let rev = EventRevision::new(3).unwrap();
    assert_eq!(
        adapter.remove(&mut core, &e.id, rev, 1),
        Ok(RemoveOutcome::Removed)
    );
    assert_eq!(
        adapter.remove(&mut core, &e.id, rev, 1),
        Ok(RemoveOutcome::AlreadyRemoved)
    );
    assert!(get(&core).is_none());
    assert_eq!(
        adapter.publish(&mut core, &e),
        Err(Error::Search(SearchCoreError::StaleRevision))
    );
    e.revision = rev;
    assert_eq!(
        adapter.publish(&mut core, &e),
        Err(Error::Search(SearchCoreError::RevisionConflict))
    );
}
#[test]
fn rejected_events_do_not_reach_index() {
    let (adapter, mut core, _, _, metadata) = fixture();
    for mutate in [0, 1, 2, 3, 4] {
        let mut e = event();
        match mutate {
            0 => e.draft.title = "x".repeat(1025),
            1 => e.draft.participants[0] = "x".repeat(257),
            2 => e.draft.participants = vec!["Ada".into(); 129],
            3 => e.draft.location = "x".repeat(4097),
            _ => e.draft.title = " ".into(),
        }
        let before = metadata.writes();
        assert!(adapter.publish(&mut core, &e).is_err());
        assert_eq!(metadata.writes(), before);
        assert!(get(&core).is_none());
    }
    let mut e = event();
    e.id = EventId::new("event_unmapped").unwrap();
    assert_eq!(adapter.publish(&mut core, &e), Err(Error::UnmappedEvent));
    assert_eq!(
        adapter.remove(&mut core, &e.id, e.revision, 0),
        Err(Error::UnmappedEvent)
    );
}
#[test]
fn spoof_and_revoked_publish_are_rejected() {
    let (adapter, mut core, _, authority, metadata) = fixture();
    let mut e = event();
    e.draft.owner = Some(OwnerMetadata {
        app: AppId(99),
        user: None,
    });
    assert_eq!(adapter.publish(&mut core, &e), Err(Error::SourceMismatch));
    assert_eq!(metadata.writes(), 0);
    e.draft.owner = None;
    adapter.publish(&mut core, &e).unwrap();
    authority.0.set(false);
    e.revision = EventRevision::new(2).unwrap();
    let before = metadata.writes();
    assert_eq!(
        adapter.publish(&mut core, &e),
        Err(Error::Search(SearchCoreError::ProviderNotAuthorized))
    );
    assert_eq!(
        adapter.remove(&mut core, &e.id, e.revision, 0),
        Err(Error::Search(SearchCoreError::ProviderNotAuthorized))
    );
    assert_eq!(metadata.writes(), before);
    let spoof = CalendarSearchAdapter::new(AppId(99), Objects);
    assert_eq!(
        spoof.publish(&mut core, &e),
        Err(Error::Search(SearchCoreError::ProviderNotAuthorized))
    );
}
#[test]
fn denied_read_leaks_no_result_count_or_metadata() {
    let (adapter, mut core, policy, _, _) = fixture();
    adapter.publish(&mut core, &event()).unwrap();
    policy.0.set(false);
    assert!(get(&core).is_none());
    for query in [
        SearchQuery::default(),
        SearchQuery {
            text: Some("Planning".into()),
            limit: 1,
            ..Default::default()
        },
        SearchQuery {
            tags_any: vec!["Ada".into()],
            ..Default::default()
        },
    ] {
        let response = core
            .search(
                AccessContext::anonymous(),
                &ScopedQuery::only(query, [PROVIDER]),
            )
            .unwrap();
        assert!(response.objects.is_empty());
        assert!(response.workspaces.is_empty());
        assert!(response.workspace_groups.is_empty());
        assert!(!format!("{response:?}").contains("Planning"));
    }
}
#[test]
fn revision_contract_is_identical() {
    assert_eq!(
        DocumentRevision::new(event().revision.get()).unwrap().get(),
        1
    );
}

#[test]
fn exact_utf8_bounds_are_preserved_without_truncation() {
    let (adapter, mut core, _, _, _) = fixture();
    let mut e = event();
    e.draft.title = "é".repeat(512);
    e.draft.location = "é".repeat(2048);
    e.draft.participants = vec!["é".repeat(128); 128];
    adapter.publish(&mut core, &e).unwrap();
    let record = get(&core).unwrap().hit.record;
    assert_eq!(record.title, e.draft.title);
    assert_eq!(record.location.as_ref(), Some(&e.draft.location));
    assert_eq!(record.tags, e.draft.participants);
    e.draft.title.push('é');
    e.revision = EventRevision::new(2).unwrap();
    assert!(adapter.publish(&mut core, &e).is_err());
    assert_eq!(get(&core).unwrap().hit.record, record);
}

#[test]
fn reopened_index_retains_mapping_and_tombstone_revision() {
    let metadata = MemorySnapshotBackend::new();
    let ledger = MemorySnapshotBackend::new();
    let policy = Policy(Rc::new(Cell::new(true)));
    let authority = Authority(Rc::new(Cell::new(true)));
    let adapter = CalendarSearchAdapter::new(PROVIDER, Objects);
    let mut core = SearchCore::open(
        metadata.clone(),
        ledger.clone(),
        policy.clone(),
        authority.clone(),
    )
    .unwrap();
    core.register_provider(PROVIDER).unwrap();
    let e = event();
    adapter.publish(&mut core, &e).unwrap();
    adapter
        .remove(&mut core, &e.id, EventRevision::new(2).unwrap(), 0)
        .unwrap();
    drop(core);
    let mut reopened = SearchCore::open(metadata, ledger, policy, authority).unwrap();
    assert!(get(&reopened).is_none());
    assert_eq!(
        adapter.publish(&mut reopened, &e),
        Err(Error::Search(SearchCoreError::StaleRevision))
    );
}
