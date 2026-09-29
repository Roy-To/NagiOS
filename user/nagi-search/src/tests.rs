use alloc::{
    collections::{BTreeMap, BTreeSet},
    format,
    string::String,
    vec,
    vec::Vec,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
};

use nagi_model::{AppId, AppSessionId, ObjectId, WorkspaceId};

use crate::{
    adapters::{
        FilesProducerAdapter, PageProducerAdapter, ProducerObject, WorkspaceProducerAdapter,
    },
    codec,
    host::HostFileBackend,
    store::StoreState,
    AccessContext, AttributeMatch, BackendError, DenyAllVisibility, MetadataRecord,
    MetadataStoreError, ObjectKind, Relation, RelationDirection, RelationKind, RelationProvenance,
    SearchError, SearchMatch, SearchQuery, SearchService, SearchSort, SearchTimeRange,
    SnapshotBackend, VisibilityFilter, VisibilityScope, Workspace, WorkspaceSession,
    CURRENT_STORE_VERSION,
};

static TEST_PATH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Default)]
struct MemoryBackend(Arc<Mutex<MemoryState>>);

#[derive(Default)]
struct MemoryState {
    snapshot: Option<Vec<u8>>,
    fail_writes: bool,
}

impl MemoryBackend {
    fn with_snapshot(snapshot: Vec<u8>) -> Self {
        Self(Arc::new(Mutex::new(MemoryState {
            snapshot: Some(snapshot),
            fail_writes: false,
        })))
    }

    fn fail_writes(&self) {
        self.0.lock().expect("memory state").fail_writes = true;
    }
}

impl SnapshotBackend for MemoryBackend {
    fn load_snapshot(&mut self) -> Result<Option<Vec<u8>>, BackendError> {
        Ok(self.0.lock().expect("memory state").snapshot.clone())
    }

    fn write_snapshot(&mut self, snapshot: &[u8]) -> Result<(), BackendError> {
        let mut state = self.0.lock().expect("memory state");
        if state.fail_writes {
            return Err(BackendError::Io);
        }
        state.snapshot = Some(snapshot.to_vec());
        Ok(())
    }
}

#[derive(Clone, Default)]
struct FixtureVisibility {
    denied_objects: BTreeSet<ObjectId>,
    denied_workspaces: BTreeSet<WorkspaceId>,
}

impl VisibilityFilter for FixtureVisibility {
    fn can_read_object(&self, access: AccessContext, record: &MetadataRecord) -> bool {
        if self.denied_objects.contains(&record.object_id) {
            return false;
        }
        match record.visibility {
            VisibilityScope::Public => true,
            VisibilityScope::SourceApplication => record.source_app == access.app_id,
            VisibilityScope::Private => false,
        }
    }

    fn can_read_workspace(&self, access: AccessContext, workspace: &Workspace) -> bool {
        if self.denied_workspaces.contains(&workspace.workspace_id) {
            return false;
        }
        match workspace.visibility {
            VisibilityScope::Public => true,
            VisibilityScope::SourceApplication => workspace.owner_app == access.app_id,
            VisibilityScope::Private => false,
        }
    }

    fn can_read_workspace_session(
        &self,
        access: AccessContext,
        _workspace: &Workspace,
        session: WorkspaceSession,
    ) -> bool {
        access.app_id == Some(session.app_id)
    }
}

fn public_record(id: u64, title: &str) -> MetadataRecord {
    let mut record = MetadataRecord::new(ObjectId(id), ObjectKind::File, title);
    record.visibility = VisibilityScope::Public;
    record
}

fn public_workspace(id: u64, title: &str) -> Workspace {
    let mut workspace = Workspace::new(WorkspaceId(id), title);
    workspace.visibility = VisibilityScope::Public;
    workspace
}

fn service() -> SearchService<MemoryBackend, FixtureVisibility> {
    SearchService::open(MemoryBackend::default(), FixtureVisibility::default()).expect("open")
}

fn access() -> AccessContext {
    AccessContext::for_application(AppId(7), AppSessionId(70))
}

fn result_ids(response: &crate::SearchResponse) -> Vec<ObjectId> {
    response
        .objects
        .iter()
        .map(|hit| hit.record.object_id)
        .collect()
}

fn host_test_path() -> PathBuf {
    let sequence = TEST_PATH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "nagi-search-test-{}-{sequence}.snapshot",
        std::process::id()
    ))
}

#[test]
fn existing_stable_object_id_survives_rename_and_file_store_restart() {
    assert_eq!(CURRENT_STORE_VERSION, 1);
    let path = host_test_path();
    let id = ObjectId(0x1234);
    {
        let mut service =
            SearchService::open(HostFileBackend::new(&path), FixtureVisibility::default())
                .expect("new host store");
        let mut record = public_record(id.0, "Budget draft.pdf");
        record.location = Some("/files/budget-draft.pdf".into());
        record.modified_at = Some(10);
        service.upsert_record(record).expect("initial metadata");
    }
    {
        let mut service =
            SearchService::open(HostFileBackend::new(&path), FixtureVisibility::default())
                .expect("restart host store");
        let mut renamed = public_record(id.0, "Budget final.pdf");
        renamed.location = Some("/archive/budget-final.pdf".into());
        renamed.modified_at = Some(20);
        service.upsert_record(renamed).expect("rename metadata");
    }
    let service = SearchService::open(HostFileBackend::new(&path), FixtureVisibility::default())
        .expect("restart after rename");
    let result = service
        .search(
            access(),
            &SearchQuery {
                text: Some("final".into()),
                ..SearchQuery::default()
            },
        )
        .expect("search renamed item");
    assert_eq!(result_ids(&result), [id]);
    assert_eq!(
        result.objects[0].record.location.as_deref(),
        Some("/archive/budget-final.pdf")
    );
    let old_name = service
        .search(
            access(),
            &SearchQuery {
                text: Some("draft".into()),
                ..SearchQuery::default()
            },
        )
        .expect("search old title");
    assert!(old_name.objects.is_empty());
    std::fs::remove_file(&path).expect("remove test snapshot");
}

#[test]
fn host_restart_persists_relations_workspace_membership_and_metadata_fields() {
    let path = host_test_path();
    {
        let mut service =
            SearchService::open(HostFileBackend::new(&path), FixtureVisibility::default())
                .expect("new host store");
        let mut origin = public_record(70, "persisted origin");
        origin.tags = vec!["durable".into()];
        origin.attributes.insert("format".into(), "text".into());
        service.upsert_record(origin).unwrap();
        let mut target = public_record(71, "persisted target");
        target.created_at = Some(700);
        service.upsert_record(target).unwrap();
        service
            .add_relation(Relation {
                source: ObjectId(70),
                kind: RelationKind::References,
                target: ObjectId(71),
                provenance: RelationProvenance::User,
            })
            .unwrap();
        let mut workspace = public_workspace(700, "Persisted Workspace");
        workspace.tags = vec!["checkpoint".into()];
        workspace.attributes.insert("state".into(), "saved".into());
        workspace.sessions = vec![WorkspaceSession {
            app_id: AppId(7),
            session_id: AppSessionId(70),
        }];
        workspace.objects = vec![ObjectId(70), ObjectId(71)];
        service.upsert_workspace(workspace).unwrap();
    }

    let service = SearchService::open(HostFileBackend::new(&path), FixtureVisibility::default())
        .expect("reopen host store");
    let related = service
        .search(
            access(),
            &SearchQuery {
                related_to: Some(ObjectId(70)),
                ..SearchQuery::default()
            },
        )
        .unwrap();
    assert_eq!(result_ids(&related), [ObjectId(71)]);
    let workspace = service.get_workspace(access(), WorkspaceId(700)).unwrap();
    assert_eq!(workspace.objects, [ObjectId(70), ObjectId(71)]);
    assert_eq!(workspace.tags, ["checkpoint"]);
    assert_eq!(
        workspace.attributes.get("state").map(String::as_str),
        Some("saved")
    );
    let tagged = service
        .search(
            access(),
            &SearchQuery {
                tags_any: vec!["durable".into()],
                attributes_all: vec![AttributeMatch {
                    key: "format".into(),
                    value: "text".into(),
                }],
                ..SearchQuery::default()
            },
        )
        .unwrap();
    assert_eq!(result_ids(&tagged), [ObjectId(70)]);
    std::fs::remove_file(&path).expect("remove test snapshot");
}

#[test]
fn snapshot_version_checksum_truncation_and_unknown_version_fail_closed() {
    let mut state = StoreState::default();
    state
        .records
        .insert(ObjectId(1), public_record(1, "checked"));
    let valid = codec::encode(&state).expect("encode version one");
    assert!(codec::decode(&valid).is_ok());

    let truncated = &valid[..valid.len() - 1];
    assert_eq!(
        codec::decode(truncated),
        Err(MetadataStoreError::CorruptSnapshot)
    );
    assert_eq!(
        SearchService::open(
            MemoryBackend::with_snapshot(truncated.to_vec()),
            FixtureVisibility::default()
        )
        .err(),
        Some(MetadataStoreError::CorruptSnapshot)
    );

    let mut checksum_mismatch = valid.clone();
    *checksum_mismatch.last_mut().expect("payload byte") ^= 0x80;
    assert_eq!(
        codec::decode(&checksum_mismatch),
        Err(MetadataStoreError::CorruptSnapshot)
    );
    assert_eq!(
        SearchService::open(
            MemoryBackend::with_snapshot(checksum_mismatch.clone()),
            FixtureVisibility::default()
        )
        .err(),
        Some(MetadataStoreError::CorruptSnapshot)
    );

    let mut unsupported = valid;
    unsupported[8..10].copy_from_slice(&2_u16.to_le_bytes());
    assert_eq!(
        codec::decode(&unsupported),
        Err(MetadataStoreError::UnsupportedVersion(2))
    );
    assert_eq!(
        SearchService::open(
            MemoryBackend::with_snapshot(unsupported),
            FixtureVisibility::default()
        )
        .err(),
        Some(MetadataStoreError::UnsupportedVersion(2))
    );
    assert_eq!(
        codec::decode(b"not a metadata snapshot"),
        Err(MetadataStoreError::CorruptSnapshot)
    );
}

#[test]
fn failed_snapshot_write_does_not_publish_in_memory_mutation() {
    let backend = MemoryBackend::default();
    backend.fail_writes();
    let mut service = SearchService::open(backend, FixtureVisibility::default()).expect("open");
    assert_eq!(
        service.upsert_record(public_record(5, "not committed")),
        Err(MetadataStoreError::Backend(BackendError::Io))
    );
    assert_eq!(service.get_object(access(), ObjectId(5)), None);
}

#[test]
fn relations_are_unique_queryable_removable_and_tombstone_cleaned() {
    let mut service = service();
    for id in 1..=3 {
        service
            .upsert_record(public_record(id, &format!("object {id}")))
            .unwrap();
    }
    let forward = Relation {
        source: ObjectId(1),
        kind: RelationKind::References,
        target: ObjectId(2),
        provenance: RelationProvenance::Application,
    };
    assert_eq!(service.add_relation(forward), Ok(true));
    assert_eq!(service.add_relation(forward), Ok(false));
    let cycle = Relation {
        source: ObjectId(2),
        kind: RelationKind::RelatedTo,
        target: ObjectId(3),
        provenance: RelationProvenance::User,
    };
    service.add_relation(cycle).unwrap();
    service
        .add_relation(Relation {
            source: ObjectId(3),
            kind: RelationKind::Contains,
            target: ObjectId(1),
            provenance: RelationProvenance::User,
        })
        .unwrap();

    let outgoing = service
        .related_objects(access(), ObjectId(1), RelationDirection::Outgoing, 1, 10)
        .unwrap();
    assert_eq!(outgoing, [ObjectId(2)]);
    let transitive = service
        .related_objects(access(), ObjectId(1), RelationDirection::Either, 3, 10)
        .unwrap();
    assert_eq!(transitive, [ObjectId(2), ObjectId(3)]);
    assert_eq!(
        service.related_objects(access(), ObjectId(1), RelationDirection::Either, 0, 10),
        Err(SearchError::InvalidTraversalBound)
    );

    assert_eq!(service.remove_relation(forward), Ok(true));
    assert_eq!(service.remove_relation(forward), Ok(false));
    service.add_relation(forward).unwrap();
    assert_eq!(service.remove_record(ObjectId(2), 77), Ok(true));
    assert_eq!(service.remove_record(ObjectId(2), 78), Ok(false));
    assert_eq!(service.get_object(access(), ObjectId(2)), None);
    assert_eq!(
        service.related_objects(access(), ObjectId(1), RelationDirection::Either, 4, 10),
        Ok(vec![ObjectId(3)])
    );
    service
        .upsert_record(public_record(2, "revived same stable object"))
        .expect("same ID revives");
    assert_eq!(
        service
            .get_object(access(), ObjectId(2))
            .map(|record| record.object_id),
        Some(ObjectId(2))
    );
    assert_eq!(
        service.related_objects(access(), ObjectId(2), RelationDirection::Either, 1, 10),
        Ok(Vec::new()),
        "reviving metadata does not silently restore deleted relations"
    );
}

#[test]
fn workspace_membership_is_logical_multi_workspace_and_session_scoped() {
    let visibility = FixtureVisibility {
        denied_objects: [ObjectId(11)].into_iter().collect(),
        ..FixtureVisibility::default()
    };
    // Re-open a copy of the committed backend snapshot in a policy-filtered
    // service by applying the same write sequence to the same shared backend.
    let shared_backend = MemoryBackend::default();
    let mut privileged =
        SearchService::open(shared_backend.clone(), FixtureVisibility::default()).unwrap();
    privileged
        .upsert_record(public_record(10, "shared file"))
        .unwrap();
    privileged
        .upsert_record(public_record(11, "denied file"))
        .unwrap();
    let mut first = public_workspace(5, "Work");
    first.objects = vec![ObjectId(10), ObjectId(11)];
    first.sessions = vec![
        WorkspaceSession {
            app_id: AppId(7),
            session_id: AppSessionId(70),
        },
        WorkspaceSession {
            app_id: AppId(8),
            session_id: AppSessionId(80),
        },
    ];
    privileged.upsert_workspace(first).unwrap();
    let mut second = public_workspace(4, "Research");
    second.objects = vec![ObjectId(10)];
    privileged.upsert_workspace(second).unwrap();
    let filtered = SearchService::open(shared_backend, visibility).unwrap();
    let workspace = filtered
        .get_workspace(access(), WorkspaceId(5))
        .expect("visible workspace");
    assert_eq!(workspace.objects, [ObjectId(10)]);
    assert_eq!(
        workspace.sessions,
        [WorkspaceSession {
            app_id: AppId(7),
            session_id: AppSessionId(70)
        }]
    );
    let grouped = filtered.search(access(), &SearchQuery::default()).unwrap();
    assert_eq!(grouped.workspace_groups.len(), 2);
    assert_eq!(grouped.workspace_groups[0].workspace_id, WorkspaceId(4));
    assert_eq!(grouped.workspace_groups[0].object_ids, [ObjectId(10)]);
    assert_eq!(grouped.workspace_groups[1].workspace_id, WorkspaceId(5));
    assert_eq!(grouped.workspace_groups[1].object_ids, [ObjectId(10)]);

    let scoped = SearchQuery {
        workspace: Some(WorkspaceId(5)),
        text: Some("file".into()),
        ..SearchQuery::default()
    };
    let scoped = filtered.search(access(), &scoped).unwrap();
    assert_eq!(result_ids(&scoped), [ObjectId(10)]);
    assert_eq!(scoped.workspace_groups.len(), 1);
    assert_eq!(scoped.workspace_groups[0].workspace_id, WorkspaceId(5));
    assert_eq!(
        scoped.workspaces.len(),
        0,
        "Workspace title does not contain file"
    );
}

#[test]
fn workspace_membership_mutations_are_idempotent_and_delete_cleans_tombstones() {
    let mut service = service();
    service.upsert_record(public_record(1, "member")).unwrap();
    service
        .upsert_record(public_record(2, "second member"))
        .unwrap();
    service
        .upsert_workspace(public_workspace(4, "Group"))
        .unwrap();
    assert_eq!(
        service.add_workspace_object(WorkspaceId(4), ObjectId(1)),
        Ok(true)
    );
    assert_eq!(
        service.add_workspace_object(WorkspaceId(4), ObjectId(1)),
        Ok(false)
    );
    assert_eq!(
        service.add_workspace_object(WorkspaceId(4), ObjectId(2)),
        Ok(true)
    );
    assert_eq!(
        service.remove_workspace_object(WorkspaceId(4), ObjectId(1)),
        Ok(true)
    );
    assert_eq!(
        service.remove_workspace_object(WorkspaceId(4), ObjectId(1)),
        Ok(false)
    );
    service
        .add_workspace_object(WorkspaceId(4), ObjectId(1))
        .unwrap();
    service.remove_record(ObjectId(1), 100).unwrap();
    assert_eq!(
        service
            .get_workspace(access(), WorkspaceId(4))
            .unwrap()
            .objects,
        [ObjectId(2)]
    );
    assert_eq!(service.remove_workspace(WorkspaceId(4)), Ok(true));
    assert_eq!(service.remove_workspace(WorkspaceId(4)), Ok(false));
    assert_eq!(
        service
            .get_object(access(), ObjectId(2))
            .map(|record| record.object_id),
        Some(ObjectId(2))
    );
    let remaining = service.search(access(), &SearchQuery::default()).unwrap();
    assert_eq!(result_ids(&remaining), [ObjectId(2)]);
    assert!(remaining.workspace_groups.is_empty());
}

#[test]
fn exact_prefix_substring_and_utf8_title_search_have_stable_order_and_rationale() {
    let mut service = service();
    let mut records = vec![
        public_record(20, "Budget"),
        public_record(3, "Budget final"),
        public_record(2, "Annual Budget"),
        public_record(10, "Budget"),
        public_record(30, "議事録 2026"),
    ];
    for record in &mut records {
        record.modified_at = Some(50);
    }
    for record in records {
        service.upsert_record(record).unwrap();
    }
    let exact = service
        .search(
            access(),
            &SearchQuery {
                text: Some("budget".into()),
                ..SearchQuery::default()
            },
        )
        .unwrap();
    assert_eq!(
        result_ids(&exact),
        [ObjectId(10), ObjectId(20), ObjectId(3), ObjectId(2)]
    );
    assert!(exact.objects[0]
        .rationale
        .contains(&SearchMatch::TitleExact));
    assert!(exact.objects[2]
        .rationale
        .contains(&SearchMatch::TitlePrefix));
    assert!(exact.objects[3]
        .rationale
        .contains(&SearchMatch::TitleSubstring));

    let japanese = service
        .search(
            access(),
            &SearchQuery {
                text: Some("議事録".into()),
                ..SearchQuery::default()
            },
        )
        .unwrap();
    assert_eq!(result_ids(&japanese), [ObjectId(30)]);
    assert!(japanese.objects[0]
        .rationale
        .contains(&SearchMatch::TitlePrefix));
    let substring = service
        .search(
            access(),
            &SearchQuery {
                text: Some("事録 20".into()),
                ..SearchQuery::default()
            },
        )
        .unwrap();
    assert_eq!(result_ids(&substring), [ObjectId(30)]);
    assert!(substring.objects[0]
        .rationale
        .contains(&SearchMatch::TitleSubstring));
}

#[test]
fn metadata_filters_are_conjunctive_and_report_the_reason() {
    let mut service = service();
    let mut invoice = public_record(1, "Invoice April.pdf");
    invoice.source_app = Some(AppId(7));
    invoice.tags = vec!["finance".into(), "approved".into()];
    invoice
        .attributes
        .insert("department".into(), "research".into());
    service.upsert_record(invoice).unwrap();
    let mut other_kind = public_record(2, "Different title");
    other_kind.kind = ObjectKind::ApplicationData;
    other_kind.source_app = Some(AppId(7));
    service.upsert_record(other_kind).unwrap();
    let mut other_source = public_record(3, "Invoice old");
    other_source.source_app = Some(AppId(8));
    service.upsert_record(other_source).unwrap();

    let query = SearchQuery {
        text: Some("invoice".into()),
        kind: Some(ObjectKind::File),
        source_app: Some(AppId(7)),
        tags_any: vec!["finance".into()],
        attributes_all: vec![AttributeMatch {
            key: "Department".into(),
            value: "Research".into(),
        }],
        ..SearchQuery::default()
    };
    let result = service.search(access(), &query).unwrap();
    assert_eq!(result_ids(&result), [ObjectId(1)]);
    assert!(result.objects[0].rationale.contains(&SearchMatch::Kind));
    assert!(result.objects[0]
        .rationale
        .contains(&SearchMatch::SourceApplication));
    assert!(result.objects[0]
        .rationale
        .contains(&SearchMatch::Tag("finance".into())));
    assert!(result.objects[0]
        .rationale
        .contains(&SearchMatch::Attribute("Department".into())));

    let nonmatching = SearchQuery {
        text: Some("not present".into()),
        kind: Some(ObjectKind::File),
        source_app: Some(AppId(7)),
        ..SearchQuery::default()
    };
    assert!(service
        .search(access(), &nonmatching)
        .unwrap()
        .objects
        .is_empty());
}

#[test]
fn time_ranges_are_inclusive_and_missing_times_do_not_match() {
    let mut service = service();
    let mut boundary = public_record(1, "created boundary");
    boundary.created_at = Some(100);
    boundary.modified_at = Some(300);
    boundary.observed_at = Some(500);
    service.upsert_record(boundary).unwrap();
    let mut recent = public_record(2, "recent");
    recent.created_at = Some(101);
    recent.modified_at = Some(400);
    recent.observed_at = Some(600);
    service.upsert_record(recent).unwrap();
    service
        .upsert_record(public_record(3, "timestamp missing"))
        .unwrap();

    let created = SearchQuery {
        created: Some(SearchTimeRange::new(Some(100), Some(100))),
        ..SearchQuery::default()
    };
    assert_eq!(
        result_ids(&service.search(access(), &created).unwrap()),
        [ObjectId(1)]
    );
    let observed = SearchQuery {
        observed: Some(SearchTimeRange::new(Some(500), Some(600))),
        ..SearchQuery::default()
    };
    assert_eq!(
        result_ids(&service.search(access(), &observed).unwrap()),
        [ObjectId(2), ObjectId(1)]
    );
    let recent = SearchQuery {
        sort: SearchSort::ModifiedNewest,
        ..SearchQuery::default()
    };
    assert_eq!(
        result_ids(&service.search(access(), &recent).unwrap()),
        [ObjectId(2), ObjectId(1), ObjectId(3)]
    );
    let invalid = SearchQuery {
        modified: Some(SearchTimeRange::new(Some(5), Some(4))),
        ..SearchQuery::default()
    };
    assert_eq!(
        service.search(access(), &invalid),
        Err(SearchError::InvalidModel(
            crate::ModelError::InvalidTimeRange
        ))
    );
}

#[test]
fn workspace_title_search_grouping_and_modified_order_are_deterministic() {
    let mut service = service();
    service
        .upsert_record(public_record(6, "Workspace file"))
        .unwrap();
    let mut old = public_workspace(9, "Project Atlas");
    old.owner_app = Some(AppId(7));
    old.modified_at = Some(20);
    old.objects = vec![ObjectId(6)];
    service.upsert_workspace(old).unwrap();
    let mut latest = public_workspace(3, "Project Atlas Notes");
    latest.owner_app = Some(AppId(8));
    latest.modified_at = Some(30);
    latest.objects = vec![ObjectId(6)];
    service.upsert_workspace(latest).unwrap();

    let by_title = service
        .search(
            access(),
            &SearchQuery {
                text: Some("Project Atlas".into()),
                ..SearchQuery::default()
            },
        )
        .unwrap();
    assert_eq!(
        by_title
            .workspaces
            .iter()
            .map(|hit| hit.workspace_id)
            .collect::<Vec<_>>(),
        [WorkspaceId(9), WorkspaceId(3)]
    );
    let by_owner = service
        .search(
            access(),
            &SearchQuery {
                text: Some("Project".into()),
                source_app: Some(AppId(7)),
                ..SearchQuery::default()
            },
        )
        .unwrap();
    assert_eq!(
        by_owner
            .workspaces
            .iter()
            .map(|hit| hit.workspace_id)
            .collect::<Vec<_>>(),
        [WorkspaceId(9)]
    );
    assert!(by_owner.workspaces[0]
        .rationale
        .contains(&SearchMatch::SourceApplication));
    let recent = service
        .search(
            access(),
            &SearchQuery {
                text: Some("Project".into()),
                sort: SearchSort::ModifiedNewest,
                ..SearchQuery::default()
            },
        )
        .unwrap();
    assert_eq!(
        recent
            .workspaces
            .iter()
            .map(|hit| hit.workspace_id)
            .collect::<Vec<_>>(),
        [WorkspaceId(3), WorkspaceId(9)]
    );
    let groups = service
        .search(
            access(),
            &SearchQuery {
                text: Some("Workspace file".into()),
                ..SearchQuery::default()
            },
        )
        .unwrap();
    assert_eq!(
        groups
            .workspace_groups
            .iter()
            .map(|group| group.workspace_id)
            .collect::<Vec<_>>(),
        [WorkspaceId(3), WorkspaceId(9)]
    );
    assert_eq!(groups.workspace_groups[0].object_ids, [ObjectId(6)]);
}

#[test]
fn denied_metadata_ids_counts_groups_rationales_and_relations_are_hidden() {
    let backend = MemoryBackend::default();
    let mut writer = SearchService::open(backend.clone(), FixtureVisibility::default()).unwrap();
    writer
        .upsert_record(public_record(1, "visible file"))
        .unwrap();
    let mut hidden = public_record(2, "secret plan");
    hidden.tags = vec!["secret-tag".into()];
    hidden
        .attributes
        .insert("classification".into(), "private words".into());
    writer.upsert_record(hidden).unwrap();
    let mut workspace = public_workspace(7, "shared workspace");
    workspace.objects = vec![ObjectId(1), ObjectId(2)];
    workspace.sessions = vec![
        WorkspaceSession {
            app_id: AppId(7),
            session_id: AppSessionId(70),
        },
        WorkspaceSession {
            app_id: AppId(8),
            session_id: AppSessionId(80),
        },
    ];
    writer.upsert_workspace(workspace).unwrap();
    let mut hidden_workspace = public_workspace(8, "secret workspace");
    hidden_workspace.objects = vec![ObjectId(2)];
    writer.upsert_workspace(hidden_workspace).unwrap();
    writer
        .add_relation(Relation {
            source: ObjectId(1),
            kind: RelationKind::References,
            target: ObjectId(2),
            provenance: RelationProvenance::AiSuggestion,
        })
        .unwrap();

    let visibility = FixtureVisibility {
        denied_objects: [ObjectId(2)].into_iter().collect(),
        denied_workspaces: [WorkspaceId(8)].into_iter().collect(),
    };
    let service = SearchService::open(backend, visibility).unwrap();
    assert_eq!(
        service.get_object(access(), ObjectId(2)),
        service.get_object(access(), ObjectId(999))
    );
    let hidden_text = service
        .search(
            access(),
            &SearchQuery {
                text: Some("secret plan".into()),
                ..SearchQuery::default()
            },
        )
        .unwrap();
    assert!(hidden_text.objects.is_empty());
    assert!(hidden_text.workspaces.is_empty());
    let hidden_tag = service
        .search(
            access(),
            &SearchQuery {
                tags_any: vec!["secret-tag".into()],
                ..SearchQuery::default()
            },
        )
        .unwrap();
    assert!(hidden_tag.objects.is_empty());
    let all_visible = service.search(access(), &SearchQuery::default()).unwrap();
    assert_eq!(result_ids(&all_visible), [ObjectId(1)]);
    assert_eq!(all_visible.workspace_groups.len(), 1);
    assert_eq!(all_visible.workspace_groups[0].workspace_id, WorkspaceId(7));
    assert_eq!(all_visible.workspace_groups[0].object_ids, [ObjectId(1)]);
    assert!(!all_visible
        .workspaces
        .iter()
        .any(|hit| hit.workspace_id == WorkspaceId(8)));
    let returned_workspace = service.get_workspace(access(), WorkspaceId(7)).unwrap();
    assert_eq!(returned_workspace.objects, [ObjectId(1)]);
    assert_eq!(
        returned_workspace.sessions,
        [WorkspaceSession {
            app_id: AppId(7),
            session_id: AppSessionId(70)
        }]
    );

    let by_hidden_workspace = SearchQuery {
        workspace: Some(WorkspaceId(8)),
        ..SearchQuery::default()
    };
    assert_eq!(
        service.search(access(), &by_hidden_workspace).unwrap(),
        Default::default()
    );
    assert_eq!(
        service
            .related_objects(access(), ObjectId(1), RelationDirection::Either, 2, 10)
            .unwrap(),
        []
    );
    let by_hidden_relation = SearchQuery {
        related_to: Some(ObjectId(2)),
        ..SearchQuery::default()
    };
    assert_eq!(
        service.search(access(), &by_hidden_relation).unwrap(),
        Default::default()
    );
}

#[test]
fn ai_relation_provenance_survives_storage_and_match_rationale() {
    let mut service = service();
    service.upsert_record(public_record(1, "origin")).unwrap();
    service
        .upsert_record(public_record(2, "suggested result"))
        .unwrap();
    service
        .add_relation(Relation {
            source: ObjectId(1),
            kind: RelationKind::RelatedTo,
            target: ObjectId(2),
            provenance: RelationProvenance::AiSuggestion,
        })
        .unwrap();
    let result = service
        .search(
            access(),
            &SearchQuery {
                related_to: Some(ObjectId(1)),
                ..SearchQuery::default()
            },
        )
        .unwrap();
    assert_eq!(result_ids(&result), [ObjectId(2)]);
    assert!(result.objects[0]
        .rationale
        .contains(&SearchMatch::Relation {
            kind: RelationKind::RelatedTo,
            provenance: RelationProvenance::AiSuggestion,
        }));
}

#[test]
fn deny_all_filter_is_safe_and_query_bounds_are_validated() {
    let mut service = SearchService::open(MemoryBackend::default(), DenyAllVisibility).unwrap();
    service
        .upsert_record(public_record(1, "public but gated"))
        .unwrap();
    service
        .upsert_workspace(public_workspace(1, "workspace gated"))
        .unwrap();
    assert_eq!(
        service
            .search(AccessContext::anonymous(), &SearchQuery::default())
            .unwrap(),
        Default::default()
    );
    assert_eq!(service.get_object(access(), ObjectId(1)), None);
    assert_eq!(
        service.search(
            access(),
            &SearchQuery {
                limit: crate::MAX_SEARCH_RESULTS + 1,
                ..SearchQuery::default()
            }
        ),
        Err(SearchError::InvalidModel(
            crate::ModelError::SearchLimitExceeded
        ))
    );
}

#[test]
fn files_pages_and_workspace_producer_fixtures_index_and_search() {
    let files = FilesProducerAdapter
        .to_record(ProducerObject {
            object_id: ObjectId(100),
            title: "Quarterly report.pdf".into(),
            location: Some("vfs://documents/report.pdf".into()),
            source_app: Some(AppId(7)),
            source_session: Some(AppSessionId(70)),
            created_at: Some(10),
            modified_at: Some(20),
            observed_at: Some(30),
            tags: vec!["finance".into()],
            attributes: BTreeMap::new(),
            visibility: VisibilityScope::Public,
        })
        .unwrap();
    let page = PageProducerAdapter
        .to_record(ProducerObject {
            object_id: ObjectId(101),
            title: "Nagi Browser Project".into(),
            location: Some("page://history/77".into()),
            source_app: Some(AppId(9)),
            source_session: Some(AppSessionId(90)),
            created_at: Some(5),
            modified_at: Some(40),
            observed_at: Some(41),
            tags: vec!["browser".into()],
            attributes: BTreeMap::new(),
            visibility: VisibilityScope::Public,
        })
        .unwrap();
    let mut workspace = WorkspaceProducerAdapter
        .create(
            WorkspaceId(77),
            "Nagi Project",
            Some(AppId(7)),
            VisibilityScope::Public,
        )
        .unwrap();
    workspace.sessions.push(WorkspaceSession {
        app_id: AppId(7),
        session_id: AppSessionId(70),
    });
    workspace.objects = vec![files.object_id, page.object_id];

    let mut service = service();
    service.upsert_record(files).unwrap();
    service.upsert_record(page).unwrap();
    service.upsert_workspace(workspace).unwrap();
    assert_eq!(
        result_ids(
            &service
                .search(
                    access(),
                    &SearchQuery {
                        text: Some("report".into()),
                        ..SearchQuery::default()
                    }
                )
                .unwrap()
        ),
        [ObjectId(100)]
    );
    assert_eq!(
        result_ids(
            &service
                .search(
                    access(),
                    &SearchQuery {
                        kind: Some(ObjectKind::Page),
                        tags_any: vec!["browser".into()],
                        ..SearchQuery::default()
                    }
                )
                .unwrap()
        ),
        [ObjectId(101)]
    );
    let workspace_search = service
        .search(
            access(),
            &SearchQuery {
                text: Some("Nagi Project".into()),
                ..SearchQuery::default()
            },
        )
        .unwrap();
    assert_eq!(
        workspace_search
            .workspaces
            .iter()
            .map(|hit| hit.workspace_id)
            .collect::<Vec<_>>(),
        [WorkspaceId(77)]
    );
    let all_results = service.search(access(), &SearchQuery::default()).unwrap();
    assert_eq!(
        all_results.workspace_groups[0].object_ids,
        [ObjectId(101), ObjectId(100)]
    );
}

#[test]
fn m19_acceptance_indexes_filters_restarts_and_researches_stable_objects() {
    let path = host_test_path();
    let file_id = ObjectId(0x1901);
    let page_id = ObjectId(0x1902);
    let hidden_id = ObjectId(0x1903);
    let workspace_id = WorkspaceId(0x1910);

    {
        let mut service =
            SearchService::open(HostFileBackend::new(&path), FixtureVisibility::default())
                .expect("create acceptance store");

        let mut file = FilesProducerAdapter
            .to_record(ProducerObject {
                object_id: file_id,
                title: "Quarterly budget draft.pdf".into(),
                location: Some("vfs://documents/budget-draft.pdf".into()),
                source_app: Some(AppId(7)),
                source_session: Some(AppSessionId(70)),
                created_at: Some(100),
                modified_at: Some(150),
                observed_at: Some(160),
                tags: vec!["finance".into()],
                attributes: BTreeMap::new(),
                visibility: VisibilityScope::Public,
            })
            .expect("map file producer record");
        file.attributes.insert("format".into(), "pdf".into());
        service.upsert_record(file).expect("index file");

        let page = PageProducerAdapter
            .to_record(ProducerObject {
                object_id: page_id,
                title: "Quarterly budget review".into(),
                location: Some("page://history/review".into()),
                source_app: Some(AppId(9)),
                source_session: Some(AppSessionId(90)),
                created_at: Some(90),
                modified_at: Some(170),
                observed_at: Some(175),
                tags: vec!["review".into()],
                attributes: BTreeMap::new(),
                visibility: VisibilityScope::Public,
            })
            .expect("map page producer record");
        service.upsert_record(page).expect("index page");

        service
            .upsert_record(public_record(hidden_id.0, "Quarterly budget restricted"))
            .expect("index restricted record");

        let mut workspace = WorkspaceProducerAdapter
            .create(
                workspace_id,
                "Quarterly budget workspace",
                Some(AppId(7)),
                VisibilityScope::Public,
            )
            .expect("create workspace");
        workspace.objects = vec![file_id, page_id, hidden_id];
        workspace.sessions.push(WorkspaceSession {
            app_id: AppId(7),
            session_id: AppSessionId(70),
        });
        service
            .upsert_workspace(workspace)
            .expect("persist workspace");

        let initial = service
            .search(
                access(),
                &SearchQuery {
                    text: Some("budget".into()),
                    ..SearchQuery::default()
                },
            )
            .expect("search indexed data");
        assert_eq!(result_ids(&initial), [page_id, file_id, hidden_id]);
    }

    let filtered = FixtureVisibility {
        denied_objects: [hidden_id].into_iter().collect(),
        ..FixtureVisibility::default()
    };
    {
        let service = SearchService::open(HostFileBackend::new(&path), filtered.clone())
            .expect("restart with caller visibility policy");
        let query = service
            .search(
                access(),
                &SearchQuery {
                    text: Some("budget".into()),
                    modified: Some(SearchTimeRange::new(Some(140), Some(180))),
                    ..SearchQuery::default()
                },
            )
            .expect("search after restart");
        assert_eq!(result_ids(&query), [page_id, file_id]);
        assert!(query
            .objects
            .iter()
            .all(|hit| !hit.record.title.contains("restricted")));

        let metadata = service
            .search(
                access(),
                &SearchQuery {
                    tags_any: vec!["finance".into()],
                    attributes_all: vec![AttributeMatch {
                        key: "format".into(),
                        value: "pdf".into(),
                    }],
                    ..SearchQuery::default()
                },
            )
            .expect("search metadata after restart");
        assert_eq!(result_ids(&metadata), [file_id]);

        let grouped = service
            .search(access(), &SearchQuery::default())
            .expect("group visible objects");
        assert_eq!(grouped.workspace_groups.len(), 1);
        assert_eq!(grouped.workspace_groups[0].workspace_id, workspace_id);
        assert_eq!(grouped.workspace_groups[0].object_ids, [page_id, file_id]);
        assert!(service.get_object(access(), hidden_id).is_none());
    }

    {
        let mut service = SearchService::open(HostFileBackend::new(&path), filtered.clone())
            .expect("second restart");
        let mut renamed = public_record(file_id.0, "Quarterly budget final.pdf");
        renamed.location = Some("vfs://archive/budget-final.pdf".into());
        renamed.modified_at = Some(200);
        renamed.tags = vec!["finance".into()];
        renamed.attributes.insert("format".into(), "pdf".into());
        service
            .upsert_record(renamed)
            .expect("update same stable ID");
    }

    let service = SearchService::open(HostFileBackend::new(&path), filtered)
        .expect("restart after stable-ID update");
    let final_result = service
        .search(
            access(),
            &SearchQuery {
                text: Some("final".into()),
                modified: Some(SearchTimeRange::new(Some(200), Some(200))),
                ..SearchQuery::default()
            },
        )
        .expect("re-search updated metadata");
    assert_eq!(result_ids(&final_result), [file_id]);
    assert_eq!(final_result.objects[0].record.object_id, file_id);
    assert_eq!(
        final_result.objects[0].record.location.as_deref(),
        Some("vfs://archive/budget-final.pdf")
    );
    std::fs::remove_file(&path).expect("remove acceptance snapshot");
}
