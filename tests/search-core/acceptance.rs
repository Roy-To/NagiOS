//! SEARCH-CORE-01 host Foundation acceptance tests.
//!
//! Every test is deterministic: no clock, randomness, threads, or host
//! filesystem. Persistence uses the in-memory reference backend.

use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

use nagi_search_core::{
    nagi_model::{AppId, AppSessionId, ObjectId},
    nagi_search::{
        AccessContext, BackendError, MetadataRecord, MetadataStoreError, ModelError, ObjectKind,
        SearchError, SearchMatch, SearchQuery, SearchSort, SearchTimeRange, VisibilityFilter,
        VisibilityScope, Workspace, MAX_SEARCH_RESULTS,
    },
    DocumentRevision, IngestOutcome, LedgerError, MemorySnapshotBackend, ProviderAuthority,
    ProviderScope, RemoveOutcome, ScopedQuery, ScopedSearchResponse, SearchCore, SearchCoreError,
    CURRENT_LEDGER_VERSION, LEDGER_MAGIC, MAX_LEDGER_SNAPSHOT_BYTES, MAX_PROVIDERS,
    SEARCH_CORE_CONTRACT_VERSION,
};

const FILES: AppId = AppId(0x1001);
const NOTES: AppId = AppId(0x2002);
const ALBERT: AppId = AppId(0x3003);
const CALLER: AppId = AppId(0x9009);

/// Test-only caller policy: explicit deny list, Public readable, and
/// SourceApplication readable only by that application. Private is denied.
#[derive(Clone, Default)]
struct Policy {
    denied: Rc<RefCell<BTreeSet<ObjectId>>>,
}

impl Policy {
    fn deny(&self, id: u64) {
        self.denied.borrow_mut().insert(ObjectId(id));
    }
}

impl VisibilityFilter for Policy {
    fn can_read_object(&self, access: AccessContext, record: &MetadataRecord) -> bool {
        if self.denied.borrow().contains(&record.object_id) {
            return false;
        }
        match record.visibility {
            VisibilityScope::Public => true,
            VisibilityScope::SourceApplication => {
                access.app_id.is_some() && access.app_id == record.source_app
            }
            VisibilityScope::Private => false,
        }
    }

    fn can_read_workspace(&self, _access: AccessContext, _workspace: &Workspace) -> bool {
        true
    }
}

/// Test-only provider authority with revocation.
#[derive(Clone, Default)]
struct Authority {
    allowed: Rc<RefCell<BTreeSet<AppId>>>,
}

impl Authority {
    fn allowing(providers: &[AppId]) -> Self {
        let authority = Self::default();
        authority
            .allowed
            .borrow_mut()
            .extend(providers.iter().copied());
        authority
    }

    fn revoke(&self, provider: AppId) {
        self.allowed.borrow_mut().remove(&provider);
    }
}

impl ProviderAuthority for Authority {
    fn may_publish(&self, provider: AppId) -> bool {
        self.allowed.borrow().contains(&provider)
    }
}

type Core = SearchCore<MemorySnapshotBackend, MemorySnapshotBackend, Policy, Authority>;

struct Fixture {
    core: Core,
    metadata: MemorySnapshotBackend,
    ledger: MemorySnapshotBackend,
    policy: Policy,
    authority: Authority,
}

impl Fixture {
    fn new() -> Self {
        let metadata = MemorySnapshotBackend::new();
        let ledger = MemorySnapshotBackend::new();
        let policy = Policy::default();
        let authority = Authority::allowing(&[FILES, NOTES, ALBERT]);
        let mut core = SearchCore::open(
            metadata.clone(),
            ledger.clone(),
            policy.clone(),
            authority.clone(),
        )
        .expect("open empty core");
        core.register_provider(FILES).unwrap();
        core.register_provider(NOTES).unwrap();
        Self {
            core,
            metadata,
            ledger,
            policy,
            authority,
        }
    }

    fn reopen(&self) -> Result<Core, SearchCoreError> {
        SearchCore::open(
            self.metadata.clone(),
            self.ledger.clone(),
            self.policy.clone(),
            self.authority.clone(),
        )
    }
}

fn rev(value: u64) -> DocumentRevision {
    DocumentRevision::new(value).unwrap()
}

fn record(id: u64, title: &str) -> MetadataRecord {
    let mut record = MetadataRecord::new(ObjectId(id), ObjectKind::File, title);
    record.visibility = VisibilityScope::Public;
    record.modified_at = Some(1_000);
    record
}

fn caller() -> AccessContext {
    AccessContext::for_application(CALLER, AppSessionId(1))
}

fn text(value: &str) -> SearchQuery {
    SearchQuery {
        text: Some(value.into()),
        ..SearchQuery::default()
    }
}

fn ids(response: &ScopedSearchResponse) -> Vec<u64> {
    response
        .objects
        .iter()
        .map(|hit| hit.hit.record.object_id.0)
        .collect()
}

fn search(core: &Core, query: SearchQuery) -> ScopedSearchResponse {
    core.search(caller(), &ScopedQuery::all(query)).unwrap()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn reseal(bytes: &mut [u8]) {
    let len = bytes.len() - 8;
    let checksum = fnv1a(&bytes[..len]);
    bytes[len..].copy_from_slice(&checksum.to_le_bytes());
}

#[test]
fn contract_version_and_revision_zero_are_explicit() {
    assert_eq!(SEARCH_CORE_CONTRACT_VERSION, 1);
    assert_eq!(CURRENT_LEDGER_VERSION, 1);
    assert_eq!(DocumentRevision::new(0), None);
    assert_eq!(rev(7).get(), 7);
}

#[test]
fn insert_then_query_returns_hit_with_provenance() {
    let mut fx = Fixture::new();
    assert_eq!(
        fx.core
            .upsert(FILES, rev(1), record(10, "Quarterly report.pdf"))
            .unwrap(),
        IngestOutcome::Applied
    );
    let response = search(&fx.core, text("quarterly"));
    assert_eq!(ids(&response), vec![10]);
    let hit = &response.objects[0];
    assert_eq!(hit.provider, FILES);
    assert_eq!(hit.revision, rev(1));
    assert_eq!(hit.hit.record.source_app, Some(FILES));
    assert!(hit.hit.rationale.contains(&SearchMatch::TitlePrefix));
}

#[test]
fn identical_replay_is_idempotent_and_does_not_rewrite() {
    let mut fx = Fixture::new();
    fx.core.upsert(FILES, rev(3), record(10, "Plan")).unwrap();
    let metadata_writes = fx.metadata.writes();
    let ledger_writes = fx.ledger.writes();
    assert_eq!(
        fx.core.upsert(FILES, rev(3), record(10, "Plan")).unwrap(),
        IngestOutcome::Unchanged
    );
    assert_eq!(fx.metadata.writes(), metadata_writes);
    assert_eq!(fx.ledger.writes(), ledger_writes);
    assert_eq!(ids(&search(&fx.core, text("plan"))), vec![10]);
}

#[test]
fn newer_revision_replaces_content_incrementally() {
    let mut fx = Fixture::new();
    fx.core.upsert(FILES, rev(1), record(10, "Draft")).unwrap();
    fx.core.upsert(FILES, rev(2), record(10, "Final")).unwrap();
    assert!(ids(&search(&fx.core, text("draft"))).is_empty());
    let response = search(&fx.core, text("final"));
    assert_eq!(ids(&response), vec![10]);
    assert_eq!(response.objects[0].revision, rev(2));
    assert_eq!(fx.core.revision(FILES, ObjectId(10)), Some(rev(2)));
}

#[test]
fn remove_hides_object_from_search_and_lookup() {
    let mut fx = Fixture::new();
    fx.core.upsert(FILES, rev(1), record(10, "Budget")).unwrap();
    assert_eq!(
        fx.core.remove(FILES, ObjectId(10), rev(2), 5_000).unwrap(),
        RemoveOutcome::Removed
    );
    assert!(ids(&search(&fx.core, text("budget"))).is_empty());
    assert!(ids(&search(&fx.core, SearchQuery::default())).is_empty());
    assert_eq!(
        fx.core
            .get(caller(), &ProviderScope::All, ObjectId(10))
            .unwrap(),
        None
    );
    assert_eq!(
        fx.core.remove(FILES, ObjectId(10), rev(2), 5_000).unwrap(),
        RemoveOutcome::AlreadyRemoved
    );
}

#[test]
fn stale_duplicate_and_conflicting_revisions_are_rejected() {
    let mut fx = Fixture::new();
    fx.core
        .upsert(FILES, rev(5), record(10, "Current"))
        .unwrap();
    assert_eq!(
        fx.core.upsert(FILES, rev(4), record(10, "Older")),
        Err(SearchCoreError::StaleRevision)
    );
    assert_eq!(
        fx.core.upsert(FILES, rev(5), record(10, "Different")),
        Err(SearchCoreError::RevisionConflict)
    );
    assert_eq!(
        fx.core.remove(FILES, ObjectId(10), rev(5), 1),
        Err(SearchCoreError::RevisionConflict)
    );
    assert_eq!(
        fx.core.remove(FILES, ObjectId(10), rev(4), 1),
        Err(SearchCoreError::StaleRevision)
    );
    assert_eq!(ids(&search(&fx.core, text("current"))), vec![10]);
    assert!(ids(&search(&fx.core, text("older"))).is_empty());
    assert!(ids(&search(&fx.core, text("different"))).is_empty());
}

#[test]
fn delayed_older_upsert_cannot_resurrect_removed_object() {
    let mut fx = Fixture::new();
    fx.core
        .upsert(FILES, rev(1), record(10, "Secret plan"))
        .unwrap();
    fx.core.remove(FILES, ObjectId(10), rev(2), 10).unwrap();
    assert_eq!(
        fx.core.upsert(FILES, rev(1), record(10, "Secret plan")),
        Err(SearchCoreError::StaleRevision)
    );
    assert_eq!(
        fx.core.upsert(FILES, rev(2), record(10, "Secret plan")),
        Err(SearchCoreError::RevisionConflict)
    );
    assert!(ids(&search(&fx.core, text("secret"))).is_empty());
    // A genuinely newer revision revives the identity.
    fx.core
        .upsert(FILES, rev(3), record(10, "Plan v3"))
        .unwrap();
    assert_eq!(ids(&search(&fx.core, text("plan"))), vec![10]);
}

#[test]
fn multiple_providers_are_searched_and_scoped() {
    let mut fx = Fixture::new();
    fx.core
        .upsert(FILES, rev(1), record(10, "Trip itinerary"))
        .unwrap();
    fx.core
        .upsert(NOTES, rev(1), record(20, "Trip notes"))
        .unwrap();
    assert_eq!(ids(&search(&fx.core, text("trip"))), vec![10, 20]);

    let notes_only = fx
        .core
        .search(caller(), &ScopedQuery::only(text("trip"), [NOTES]))
        .unwrap();
    assert_eq!(ids(&notes_only), vec![20]);
    assert!(notes_only.objects.iter().all(|hit| hit.provider == NOTES));

    // An unregistered provider in scope contributes nothing and is not an error.
    let albert_only = fx
        .core
        .search(caller(), &ScopedQuery::only(text("trip"), [ALBERT]))
        .unwrap();
    assert!(albert_only.objects.is_empty());

    // Canonical source filter composes with the scope.
    let conflicting = fx
        .core
        .search(
            caller(),
            &ScopedQuery::only(
                SearchQuery {
                    source_app: Some(FILES),
                    ..text("trip")
                },
                [NOTES],
            ),
        )
        .unwrap();
    assert!(conflicting.objects.is_empty());
}

#[test]
fn scope_is_applied_before_the_result_limit() {
    let mut fx = Fixture::new();
    for id in 1..=5 {
        fx.core
            .upsert(FILES, rev(1), record(id, &format!("alpha {id}")))
            .unwrap();
    }
    fx.core
        .upsert(NOTES, rev(1), record(100, "alpha notes"))
        .unwrap();
    let response = fx
        .core
        .search(
            caller(),
            &ScopedQuery::only(
                SearchQuery {
                    limit: 1,
                    sort: SearchSort::ObjectIdAscending,
                    ..text("alpha")
                },
                [NOTES],
            ),
        )
        .unwrap();
    assert_eq!(ids(&response), vec![100]);
}

#[test]
fn provider_cannot_touch_another_providers_object() {
    let mut fx = Fixture::new();
    fx.core
        .upsert(FILES, rev(1), record(10, "Owned by files"))
        .unwrap();
    assert_eq!(
        fx.core.upsert(NOTES, rev(9), record(10, "Hijack")),
        Err(SearchCoreError::ObjectOwnedByOtherProvider)
    );
    assert_eq!(
        fx.core.remove(NOTES, ObjectId(10), rev(9), 1),
        Err(SearchCoreError::UnknownObject)
    );
    assert_eq!(
        fx.core.remove(NOTES, ObjectId(999), rev(1), 1),
        Err(SearchCoreError::UnknownObject)
    );
    assert_eq!(fx.core.revision(NOTES, ObjectId(10)), None);
    assert_eq!(fx.core.revision(FILES, ObjectId(10)), Some(rev(1)));
    assert!(ids(&search(&fx.core, text("hijack"))).is_empty());
    assert_eq!(ids(&search(&fx.core, text("owned"))), vec![10]);
}

#[test]
fn payload_source_claim_is_not_identity() {
    let mut fx = Fixture::new();
    let mut spoof = record(10, "Spoofed");
    spoof.source_app = Some(FILES);
    assert_eq!(
        fx.core.upsert(NOTES, rev(1), spoof),
        Err(SearchCoreError::SourceMismatch)
    );
    assert_eq!(fx.core.revision(FILES, ObjectId(10)), None);
    assert_eq!(fx.core.revision(NOTES, ObjectId(10)), None);

    // An absent claim is bound to the trusted provider identity.
    fx.core
        .upsert(NOTES, rev(1), record(11, "Unclaimed"))
        .unwrap();
    let response = search(&fx.core, text("unclaimed"));
    assert_eq!(response.objects[0].hit.record.source_app, Some(NOTES));
    assert_eq!(response.objects[0].provider, NOTES);
}

#[test]
fn provider_authority_fails_closed_and_revocation_applies_immediately() {
    let mut fx = Fixture::new();
    let outsider = AppId(0x6666);
    assert_eq!(
        fx.core.register_provider(outsider),
        Err(SearchCoreError::ProviderNotAuthorized)
    );
    assert_eq!(
        fx.core.upsert(outsider, rev(1), record(1, "x")),
        Err(SearchCoreError::ProviderNotAuthorized)
    );
    // Authorized but never registered.
    assert_eq!(
        fx.core.upsert(ALBERT, rev(1), record(1, "x")),
        Err(SearchCoreError::UnknownProvider)
    );
    assert_eq!(
        fx.core.register_provider(FILES),
        Err(SearchCoreError::ProviderAlreadyRegistered)
    );

    fx.core
        .upsert(FILES, rev(1), record(10, "Before revoke"))
        .unwrap();
    fx.authority.revoke(FILES);
    assert_eq!(
        fx.core.upsert(FILES, rev(2), record(10, "After revoke")),
        Err(SearchCoreError::ProviderNotAuthorized)
    );
    assert_eq!(
        fx.core.remove(FILES, ObjectId(10), rev(2), 1),
        Err(SearchCoreError::ProviderNotAuthorized)
    );
    assert_eq!(fx.core.revision(FILES, ObjectId(10)), Some(rev(1)));
}

#[test]
fn deny_all_default_authority_rejects_registration() {
    let mut core = SearchCore::open(
        MemorySnapshotBackend::new(),
        MemorySnapshotBackend::new(),
        Policy::default(),
        nagi_search_core::DenyAllProviders,
    )
    .unwrap();
    assert_eq!(
        core.register_provider(FILES),
        Err(SearchCoreError::ProviderNotAuthorized)
    );
    assert_eq!(core.registered_providers().count(), 0);
}

#[test]
fn unregistering_a_provider_propagates_removal() {
    let mut fx = Fixture::new();
    fx.core
        .upsert(FILES, rev(1), record(10, "shared term a"))
        .unwrap();
    fx.core
        .upsert(FILES, rev(1), record(11, "shared term b"))
        .unwrap();
    fx.core
        .upsert(NOTES, rev(1), record(20, "shared term c"))
        .unwrap();
    fx.core.remove(FILES, ObjectId(11), rev(2), 1).unwrap();

    assert_eq!(fx.core.unregister_provider(FILES, 2_000).unwrap(), 1);
    assert_eq!(ids(&search(&fx.core, text("shared"))), vec![20]);
    assert_eq!(
        fx.core.registered_providers().collect::<Vec<_>>(),
        vec![NOTES]
    );
    assert_eq!(
        fx.core.upsert(FILES, rev(2), record(10, "again")),
        Err(SearchCoreError::UnknownProvider)
    );
    assert_eq!(
        fx.core.unregister_provider(FILES, 1),
        Err(SearchCoreError::UnknownProvider)
    );

    // Re-registration keeps tombstones, so stale replays stay rejected.
    fx.core.register_provider(FILES).unwrap();
    assert_eq!(
        fx.core.upsert(FILES, rev(1), record(10, "shared term a")),
        Err(SearchCoreError::RevisionConflict)
    );
    fx.core
        .upsert(FILES, rev(2), record(10, "shared term a2"))
        .unwrap();
    assert_eq!(ids(&search(&fx.core, text("shared"))), vec![10, 20]);
}

#[test]
fn result_count_is_bounded() {
    let mut fx = Fixture::new();
    for id in 1..=5 {
        fx.core
            .upsert(FILES, rev(1), record(id, &format!("item {id}")))
            .unwrap();
    }
    let limited = search(
        &fx.core,
        SearchQuery {
            limit: 2,
            sort: SearchSort::ObjectIdAscending,
            ..text("item")
        },
    );
    assert_eq!(ids(&limited), vec![1, 2]);
    let zero = search(
        &fx.core,
        SearchQuery {
            limit: 0,
            ..text("item")
        },
    );
    assert!(zero.objects.is_empty());
    assert_eq!(
        fx.core.search(
            caller(),
            &ScopedQuery::all(SearchQuery {
                limit: MAX_SEARCH_RESULTS + 1,
                ..text("item")
            })
        ),
        Err(SearchCoreError::Search(SearchError::InvalidModel(
            ModelError::SearchLimitExceeded
        )))
    );
}

#[test]
fn ordering_and_ties_are_deterministic() {
    let mut fx = Fixture::new();
    // Equal relevance (title prefix) and modified time: ObjectId breaks ties.
    fx.core
        .upsert(NOTES, rev(1), record(30, "Report b"))
        .unwrap();
    fx.core
        .upsert(FILES, rev(1), record(12, "Report a"))
        .unwrap();
    let mut newer = record(40, "Report c");
    newer.modified_at = Some(9_000);
    fx.core.upsert(FILES, rev(1), newer).unwrap();
    fx.core.upsert(FILES, rev(1), record(50, "report")).unwrap();

    let first = search(&fx.core, text("report"));
    // Exact title first, then newer modified, then ascending ObjectId.
    assert_eq!(ids(&first), vec![50, 40, 12, 30]);
    for _ in 0..10 {
        assert_eq!(search(&fx.core, text("report")), first);
    }
}

#[test]
fn empty_index_and_empty_query_behavior() {
    let mut fx = Fixture::new();
    assert!(search(&fx.core, text("anything")).objects.is_empty());
    assert!(search(&fx.core, SearchQuery::default()).objects.is_empty());

    fx.core.upsert(FILES, rev(1), record(1, "one")).unwrap();
    fx.core.upsert(NOTES, rev(1), record(2, "two")).unwrap();
    // No text: a filter-only query lists every visible in-scope object.
    assert_eq!(ids(&search(&fx.core, SearchQuery::default())).len(), 2);
    // Blank text is rejected rather than silently matching everything.
    assert_eq!(
        fx.core.search(caller(), &ScopedQuery::all(text("   "))),
        Err(SearchCoreError::Search(SearchError::InvalidModel(
            ModelError::EmptyTextQuery
        )))
    );
}

#[test]
fn malformed_and_oversized_records_are_rejected_without_side_effects() {
    let mut fx = Fixture::new();
    let cases = [
        (record(1, "   "), ModelError::EmptyTitle),
        (record(1, &"t".repeat(1025)), ModelError::TitleTooLong),
        (
            {
                let mut r = record(1, "tags");
                r.tags = (0..129).map(|i| format!("tag{i}")).collect();
                r
            },
            ModelError::TooManyTags,
        ),
        (
            {
                let mut r = record(1, "attr");
                r.attributes.insert("k".into(), "v".repeat(4097));
                r
            },
            ModelError::AttributeTooLong,
        ),
    ];
    for (bad, error) in cases {
        assert_eq!(
            fx.core.upsert(FILES, rev(1), bad),
            Err(SearchCoreError::InvalidRecord(error))
        );
    }
    let mut tombstoned = record(1, "tomb");
    tombstoned.tombstoned_at = Some(1);
    assert_eq!(
        fx.core.upsert(FILES, rev(1), tombstoned),
        Err(SearchCoreError::TombstonedInput)
    );
    assert_eq!(fx.core.revision(FILES, ObjectId(1)), None);
    assert!(search(&fx.core, SearchQuery::default()).objects.is_empty());
}

#[test]
fn invalid_provider_scopes_are_rejected() {
    let fx = Fixture::new();
    assert_eq!(
        fx.core
            .search(caller(), &ScopedQuery::only(text("a"), std::iter::empty())),
        Err(SearchCoreError::EmptyProviderScope)
    );
    let too_many = (0..=MAX_PROVIDERS as u64).map(AppId);
    assert_eq!(
        fx.core
            .search(caller(), &ScopedQuery::only(text("a"), too_many)),
        Err(SearchCoreError::ProviderScopeTooLarge)
    );
}

fn populate_with_denied(core: &mut Core, include_denied: bool) {
    core.upsert(FILES, rev(1), record(10, "Project alpha"))
        .unwrap();
    core.upsert(NOTES, rev(1), record(20, "Project beta"))
        .unwrap();
    if include_denied {
        // Matches the query through title, tag and attribute.
        let mut secret = record(15, "Project alpha secret");
        secret.tags = vec!["project".into()];
        secret.attributes.insert("project".into(), "alpha".into());
        secret.modified_at = Some(99_999);
        core.upsert(FILES, rev(1), secret).unwrap();
        // Private visibility: denied by policy without a deny-list entry.
        let mut private = record(16, "Project private");
        private.visibility = VisibilityScope::Private;
        core.upsert(NOTES, rev(1), private).unwrap();
        // Readable only by its own source app, not by CALLER.
        let mut app_only = record(17, "Project app-only");
        app_only.visibility = VisibilityScope::SourceApplication;
        core.upsert(NOTES, rev(1), app_only).unwrap();
    }
}

#[test]
fn unauthorized_documents_do_not_leak_through_results_counts_or_rationale() {
    let mut with_denied = Fixture::new();
    with_denied.policy.deny(15);
    populate_with_denied(&mut with_denied.core, true);
    let mut without = Fixture::new();
    populate_with_denied(&mut without.core, false);

    let queries = [
        text("project"),
        text("alpha"),
        text("secret"),
        SearchQuery::default(),
        SearchQuery {
            tags_any: vec!["project".into()],
            ..SearchQuery::default()
        },
        SearchQuery {
            limit: 1,
            sort: SearchSort::ModifiedNewest,
            ..SearchQuery::default()
        },
    ];
    for query in queries {
        for scope in [
            ProviderScope::All,
            ProviderScope::only([FILES]),
            ProviderScope::only([NOTES]),
        ] {
            let scoped = ScopedQuery {
                query: query.clone(),
                providers: scope,
            };
            let observed = with_denied.core.search(caller(), &scoped).unwrap();
            let baseline = without.core.search(caller(), &scoped).unwrap();
            assert_eq!(
                observed, baseline,
                "denied content changed output for {scoped:?}"
            );
        }
    }
    for hidden in [15, 16, 17] {
        assert_eq!(
            with_denied
                .core
                .get(caller(), &ProviderScope::All, ObjectId(hidden))
                .unwrap(),
            None
        );
    }
    // The owning application itself may read its SourceApplication record.
    let own = with_denied
        .core
        .get(
            AccessContext::for_application(NOTES, AppSessionId(2)),
            &ProviderScope::All,
            ObjectId(17),
        )
        .unwrap()
        .expect("source application can read its own record");
    assert_eq!(own.provider, NOTES);
}

#[test]
fn out_of_scope_providers_do_not_leak() {
    let mut fx = Fixture::new();
    fx.core
        .upsert(FILES, rev(1), record(10, "Shared title"))
        .unwrap();
    fx.core
        .upsert(NOTES, rev(1), record(20, "Shared title"))
        .unwrap();
    assert_eq!(
        fx.core
            .get(caller(), &ProviderScope::only([NOTES]), ObjectId(10))
            .unwrap(),
        None
    );
    let hit = fx
        .core
        .get(caller(), &ProviderScope::only([FILES]), ObjectId(10))
        .unwrap()
        .unwrap();
    assert_eq!((hit.provider, hit.revision), (FILES, rev(1)));
    let scoped = fx
        .core
        .search(caller(), &ScopedQuery::only(text("shared"), [NOTES]))
        .unwrap();
    assert_eq!(ids(&scoped), vec![20]);
    assert!(scoped.workspaces.is_empty());
}

#[test]
fn ledger_survives_reopen_and_keeps_rejecting_stale_updates() {
    let mut fx = Fixture::new();
    fx.core
        .upsert(FILES, rev(4), record(10, "Durable"))
        .unwrap();
    fx.core.upsert(NOTES, rev(2), record(20, "Gone")).unwrap();
    fx.core.remove(NOTES, ObjectId(20), rev(3), 7).unwrap();
    let before = search(&fx.core, SearchQuery::default());

    let mut reopened = fx.reopen().unwrap();
    assert_eq!(search(&reopened, SearchQuery::default()), before);
    assert_eq!(
        reopened.registered_providers().collect::<Vec<_>>(),
        vec![FILES, NOTES]
    );
    assert_eq!(reopened.revision(FILES, ObjectId(10)), Some(rev(4)));
    assert_eq!(
        reopened.upsert(FILES, rev(3), record(10, "Stale")),
        Err(SearchCoreError::StaleRevision)
    );
    assert_eq!(
        reopened.upsert(NOTES, rev(2), record(20, "Gone")),
        Err(SearchCoreError::StaleRevision)
    );
    assert_eq!(
        reopened
            .upsert(FILES, rev(4), record(10, "Durable"))
            .unwrap(),
        IngestOutcome::Unchanged
    );
}

#[test]
fn unsupported_ledger_version_is_rejected_not_reinterpreted() {
    let mut fx = Fixture::new();
    fx.core.upsert(FILES, rev(1), record(10, "x")).unwrap();
    let mut bytes = fx.ledger.snapshot().unwrap();
    assert_eq!(&bytes[..4], &LEDGER_MAGIC);
    bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
    reseal(&mut bytes);
    fx.ledger.set_snapshot(Some(bytes));
    assert_eq!(
        fx.reopen().err(),
        Some(SearchCoreError::Ledger(LedgerError::UnsupportedVersion(2)))
    );
}

#[test]
fn corrupt_ledger_snapshots_are_rejected() {
    let mut fx = Fixture::new();
    fx.core.upsert(FILES, rev(1), record(10, "x")).unwrap();
    fx.core.remove(FILES, ObjectId(10), rev(2), 1).unwrap();
    let valid = fx.ledger.snapshot().unwrap();
    let flags_offset = valid.len() - 8 - 8 - 1;
    let revision_offset = flags_offset - 8;

    let mut flipped = valid.clone();
    flipped[valid.len() - 9] ^= 0x01;
    let mut trailing = valid.clone();
    trailing.push(0);
    let mut bad_magic = valid.clone();
    bad_magic[0] = b'X';
    reseal(&mut bad_magic);
    let mut bad_flags = valid.clone();
    bad_flags[flags_offset] |= 0x80;
    reseal(&mut bad_flags);
    let mut zero_revision = valid.clone();
    zero_revision[revision_offset..flags_offset].copy_from_slice(&0u64.to_le_bytes());
    reseal(&mut zero_revision);
    let mut bad_reserved = valid.clone();
    bad_reserved[6] = 1;
    reseal(&mut bad_reserved);
    let mut resealed_trailing = valid.clone();
    let body_end = valid.len() - 8;
    resealed_trailing.insert(body_end, 0);
    reseal(&mut resealed_trailing);

    let cases: Vec<(Vec<u8>, LedgerError)> = vec![
        (Vec::new(), LedgerError::Corrupt),
        (valid[..5].to_vec(), LedgerError::Corrupt),
        (valid[..valid.len() - 1].to_vec(), LedgerError::Corrupt),
        (flipped, LedgerError::Corrupt),
        (trailing, LedgerError::Corrupt),
        (bad_magic, LedgerError::Corrupt),
        (bad_flags, LedgerError::Corrupt),
        (zero_revision, LedgerError::Corrupt),
        (bad_reserved, LedgerError::Corrupt),
        (resealed_trailing, LedgerError::Corrupt),
        (
            vec![0u8; MAX_LEDGER_SNAPSHOT_BYTES + 1],
            LedgerError::TooLarge,
        ),
    ];
    for (bytes, error) in cases {
        fx.ledger.set_snapshot(Some(bytes));
        assert_eq!(fx.reopen().err(), Some(SearchCoreError::Ledger(error)));
    }
    fx.ledger.set_snapshot(Some(valid));
    assert!(fx.reopen().is_ok());
}

#[test]
fn ledger_persist_failure_is_reported_truthfully_and_recoverable() {
    let mut fx = Fixture::new();
    fx.ledger.set_fail_writes(true);
    assert_eq!(
        fx.core.upsert(FILES, rev(1), record(10, "Committed")),
        Err(SearchCoreError::LedgerPersist(BackendError::Io))
    );
    // The canonical index did commit; the error does not pretend otherwise.
    assert!(fx.core.ledger_dirty());
    assert_eq!(ids(&search(&fx.core, text("committed"))), vec![10]);
    // Retrying while storage is still failing keeps reporting the failure.
    assert_eq!(
        fx.core.upsert(FILES, rev(1), record(10, "Committed")),
        Err(SearchCoreError::LedgerPersist(BackendError::Io))
    );

    fx.ledger.set_fail_writes(false);
    assert_eq!(
        fx.core
            .upsert(FILES, rev(1), record(10, "Committed"))
            .unwrap(),
        IngestOutcome::Unchanged
    );
    assert!(!fx.core.ledger_dirty());
    let reopened = fx.reopen().unwrap();
    assert_eq!(reopened.revision(FILES, ObjectId(10)), Some(rev(1)));
}

#[test]
fn index_failure_leaves_ledger_unchanged() {
    let mut fx = Fixture::new();
    fx.metadata.set_fail_writes(true);
    assert_eq!(
        fx.core.upsert(FILES, rev(1), record(10, "Lost")),
        Err(SearchCoreError::Index(MetadataStoreError::Backend(
            BackendError::Io
        )))
    );
    assert_eq!(fx.core.revision(FILES, ObjectId(10)), None);
    assert!(search(&fx.core, text("lost")).objects.is_empty());
    fx.metadata.set_fail_writes(false);
    assert_eq!(
        fx.core.upsert(FILES, rev(1), record(10, "Lost")).unwrap(),
        IngestOutcome::Applied
    );
}

#[test]
fn ledger_load_failure_is_reported_on_open() {
    let fx = Fixture::new();
    fx.ledger.set_fail_loads(true);
    assert_eq!(
        fx.reopen().err(),
        Some(SearchCoreError::LedgerLoad(BackendError::Io))
    );
}

fn scripted_run() -> (Vec<u8>, Vec<ScopedSearchResponse>) {
    let mut fx = Fixture::new();
    for id in 0..40u64 {
        let provider = if id % 2 == 0 { FILES } else { NOTES };
        let mut r = record(1_000 - id, &format!("doc {} {}", id % 7, id));
        r.modified_at = Some((id % 5) as i64);
        r.tags = vec![format!("t{}", id % 3)];
        fx.core.upsert(provider, rev(1), r).unwrap();
    }
    for id in (0..40u64).step_by(6) {
        let provider = if id % 2 == 0 { FILES } else { NOTES };
        fx.core
            .remove(provider, ObjectId(1_000 - id), rev(2), 50)
            .unwrap();
    }
    let queries = [
        text("doc 3"),
        SearchQuery {
            tags_any: vec!["t1".into()],
            sort: SearchSort::ModifiedNewest,
            ..SearchQuery::default()
        },
        SearchQuery {
            limit: 5,
            sort: SearchSort::TitleAscending,
            ..SearchQuery::default()
        },
    ];
    let mut responses = Vec::new();
    for query in queries {
        for providers in [ProviderScope::All, ProviderScope::only([NOTES])] {
            let scoped = ScopedQuery {
                query: query.clone(),
                providers,
            };
            responses.push(fx.core.search(caller(), &scoped).unwrap());
        }
    }
    (fx.ledger.snapshot().unwrap(), responses)
}

#[test]
fn independent_runs_are_bit_for_bit_deterministic() {
    let first = scripted_run();
    for _ in 0..3 {
        assert_eq!(scripted_run(), first);
    }
    assert!(first.1.iter().any(|response| !response.objects.is_empty()));
}

#[test]
fn temporal_and_metadata_filters_work_through_the_provider_boundary() {
    let mut fx = Fixture::new();
    for (id, provider, modified, kind) in [
        (1, FILES, 100, ObjectKind::File),
        (2, FILES, 200, ObjectKind::File),
        (3, NOTES, 200, ObjectKind::Note),
        (4, NOTES, 300, ObjectKind::Note),
    ] {
        let mut r = record(id, &format!("entry {id}"));
        r.modified_at = Some(modified);
        r.kind = kind;
        fx.core.upsert(provider, rev(1), r).unwrap();
    }
    let window = SearchQuery {
        modified: Some(SearchTimeRange::new(Some(150), Some(250))),
        sort: SearchSort::ObjectIdAscending,
        ..SearchQuery::default()
    };
    assert_eq!(ids(&search(&fx.core, window.clone())), vec![2, 3]);
    let notes_window = fx
        .core
        .search(caller(), &ScopedQuery::only(window.clone(), [NOTES]))
        .unwrap();
    assert_eq!(ids(&notes_window), vec![3]);
    assert!(notes_window.objects[0]
        .hit
        .rationale
        .contains(&SearchMatch::ModifiedTime));
    let notes_kind = SearchQuery {
        kind: Some(ObjectKind::Note),
        sort: SearchSort::ModifiedNewest,
        ..SearchQuery::default()
    };
    assert_eq!(ids(&search(&fx.core, notes_kind)), vec![4, 3]);
    assert_eq!(
        fx.core.search(
            caller(),
            &ScopedQuery::all(SearchQuery {
                modified: Some(SearchTimeRange::new(Some(10), Some(1))),
                ..SearchQuery::default()
            })
        ),
        Err(SearchCoreError::Search(SearchError::InvalidModel(
            ModelError::InvalidTimeRange
        )))
    );
}
