//! Host-only fault injection for the production read path; not guest OOM.
use crate::*;
use alloc::{collections::BTreeMap, format, string::String, vec, vec::Vec};
use core::cell::Cell;
use nagi_model::{AppId, AppSessionId, ObjectId, WorkspaceId};
use std::alloc::{GlobalAlloc, Layout, System};

#[derive(Clone, Copy)]
struct Meter {
    active: bool,
    fail: usize,
    persistent: bool,
    calls: usize,
    denials: usize,
}
const OFF: Meter = Meter {
    active: false,
    fail: 0,
    persistent: false,
    calls: 0,
    denials: 0,
};
std::thread_local! { static METER: Cell<Meter> = const { Cell::new(OFF) }; static LIVE: Cell<isize> = const { Cell::new(0) }; }
fn deny() -> bool {
    METER
        .try_with(|cell| {
            let mut m = cell.get();
            if !m.active {
                return false;
            }
            m.calls += 1;
            let deny = m.fail != 0 && (m.calls == m.fail || (m.persistent && m.calls >= m.fail));
            if deny {
                m.denials += 1
            }
            cell.set(m);
            deny
        })
        .unwrap_or(false)
}
fn adjust(bytes: isize) {
    let _ = LIVE.try_with(|live| live.set(live.get() + bytes));
}
fn live() -> isize {
    LIVE.with(Cell::get)
}
struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if deny() {
            return core::ptr::null_mut();
        }
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            adjust(l.size() as isize)
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if deny() {
            return core::ptr::null_mut();
        }
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            adjust(l.size() as isize)
        }
        p
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if deny() {
            return core::ptr::null_mut();
        }
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() {
            adjust(n as isize - l.size() as isize)
        }
        q
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        adjust(-(l.size() as isize));
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
fn measured<T>(ordinal: usize, persistent: bool, f: impl FnOnce() -> T) -> (T, Meter, isize) {
    let baseline = live();
    METER.with(|m| {
        m.set(Meter {
            active: true,
            fail: ordinal,
            persistent,
            ..OFF
        })
    });
    let result = f();
    let meter = METER.with(|m| m.replace(OFF));
    (result, meter, baseline)
}
#[derive(Default)]
struct Memory {
    bytes: Option<Vec<u8>>,
    writes: usize,
}
impl SnapshotBackend for Memory {
    fn load_snapshot(&mut self) -> Result<Option<Vec<u8>>, BackendError> {
        Ok(self.bytes.clone())
    }
    fn write_snapshot(&mut self, b: &[u8]) -> Result<(), BackendError> {
        self.writes += 1;
        self.bytes = Some(b.to_vec());
        Ok(())
    }
}
#[derive(Clone, Copy)]
struct Policy;
const ACCESS: AccessContext = AccessContext::for_application(AppId(7), AppSessionId(8));
impl VisibilityFilter for Policy {
    fn can_read_object(&self, a: AccessContext, r: &MetadataRecord) -> bool {
        a == ACCESS && r.object_id != ObjectId(4)
    }
    fn can_read_workspace(&self, a: AccessContext, w: &Workspace) -> bool {
        a == ACCESS && w.workspace_id != WorkspaceId(20)
    }
    fn can_read_workspace_session(
        &self,
        _: AccessContext,
        _: &Workspace,
        _: WorkspaceSession,
    ) -> bool {
        false
    }
}
fn fixture() -> SearchService<Memory, Policy> {
    let mut service = SearchService::open(Memory::default(), Policy).unwrap();
    for id in 1..=6 {
        let mut r = MetadataRecord::new(
            ObjectId(id),
            if id == 6 {
                ObjectKind::Page
            } else {
                ObjectKind::File
            },
            format!("資料 {id}"),
        );
        r.source_app = Some(AppId(if id == 5 { 99 } else { 7 }));
        r.source_session = Some(AppSessionId(8));
        r.modified_at = Some(id as i64);
        r.attributes = BTreeMap::new();
        for i in 0..126 {
            r.attributes.insert(format!("key{i:03}"), "value".into());
        }
        r.attributes
            .insert(adapters::PRODUCER_ID_ATTRIBUTE.into(), "producer".into());
        r.attributes.insert(
            adapters::PRODUCER_KEY_ATTRIBUTE.into(),
            if id == 2 {
                "shared".into()
            } else {
                format!("k{id}")
            },
        );
        service.upsert_record(r).unwrap();
    }
    // A second record with the same key establishes first-by-ID behavior.
    let mut r = MetadataRecord::new(ObjectId(7), ObjectKind::File, "duplicate key");
    r.source_app = Some(AppId(7));
    r.source_session = Some(AppSessionId(8));
    r.attributes
        .insert(adapters::PRODUCER_ID_ATTRIBUTE.into(), "producer".into());
    r.attributes
        .insert(adapters::PRODUCER_KEY_ATTRIBUTE.into(), "shared".into());
    service.upsert_record(r).unwrap();
    for id in [10, 20] {
        let mut w = Workspace::new(WorkspaceId(id), "Files");
        w.objects = vec![
            ObjectId(1),
            ObjectId(2),
            ObjectId(3),
            ObjectId(4),
            ObjectId(6),
        ];
        w.attributes
            .insert("extra".into(), "workspace metadata".into());
        service.upsert_workspace(w).unwrap();
    }
    service.remove_record(ObjectId(3), 9).unwrap();
    service
}
fn original_titles(
    service: &SearchService<Memory, Policy>,
    a: AccessContext,
    w: WorkspaceId,
    ids: &[u64],
) -> Option<Vec<String>> {
    let workspace = service.get_workspace(a, w)?;
    let mut titles = Vec::with_capacity(ids.len());
    for id in ids {
        let id = ObjectId(*id);
        if !workspace.objects.contains(&id) {
            return None;
        }
        let r = service.get_object(a, id)?;
        if r.kind != ObjectKind::File {
            return None;
        }
        titles.push(r.title);
    }
    Some(titles)
}
#[test]
fn borrowed_producer_and_scalar_identity_preserve_filters_order_and_tombstones() {
    let service = fixture();
    let expected: Vec<_> = service
        .producer_records("producer", AppId(7), AppSessionId(8))
        .into_iter()
        .map(|r| r.object_id)
        .collect();
    let (result, meter, baseline) = measured(1, true, || {
        let mut index = 0;
        for r in service.producer_records_ref("producer", AppId(7), AppSessionId(8)) {
            assert_eq!(r.object_id, expected[index]);
            index += 1;
        }
        assert_eq!(index, expected.len());
        assert_eq!(
            service.producer_object_id("producer", "shared", AppId(7), AppSessionId(8)),
            Some(ObjectId(2))
        );
        assert_eq!(
            service.producer_object_id("producer", "k3", AppId(7), AppSessionId(8)),
            Some(ObjectId(3))
        );
        assert_eq!(
            service.producer_object_id("producer", "missing", AppId(7), AppSessionId(8)),
            None
        );
        assert_eq!(
            service.producer_object_id("wrong", "shared", AppId(7), AppSessionId(8)),
            None
        );
        assert_eq!(
            service.producer_object_id("producer", "shared", AppId(7), AppSessionId(0)),
            None
        );
        assert_eq!(
            service.producer_object_id("producer", "shared", AppId(0), AppSessionId(8)),
            None
        );
        assert_eq!(
            service
                .producer_records_ref("wrong", AppId(7), AppSessionId(8))
                .count(),
            0
        );
        index
    });
    assert!(result > 0);
    assert_eq!(meter.calls, 0);
    assert_eq!(meter.denials, 0);
    assert_eq!(live(), baseline);
}
#[test]
fn borrowed_authorized_object_matches_legacy_without_allocations() {
    let service = fixture();
    for a in [ACCESS, AccessContext::anonymous()] {
        for id in 1..=8 {
            let original = service.get_object(a, ObjectId(id));
            let (borrowed, meter, baseline) =
                measured(1, true, || service.get_object_ref(a, ObjectId(id)));
            assert_eq!(borrowed, original.as_ref());
            assert_eq!(meter.calls, 0);
            assert_eq!(meter.denials, 0);
            assert_eq!(live(), baseline);
        }
    }
}
#[test]
fn actual_files_title_algorithm_matches_legacy_for_permissions_membership_order_and_duplicates() {
    let service = fixture();
    let ids = [
        vec![],
        vec![1],
        vec![2, 1, 2],
        vec![1, 2],
        vec![3],
        vec![4],
        vec![5],
        vec![6],
        vec![99],
        vec![1, 4],
        vec![1, 6],
    ];
    for a in [ACCESS, AccessContext::anonymous()] {
        for w in [WorkspaceId(10), WorkspaceId(20), WorkspaceId(99)] {
            for ids in &ids {
                assert_eq!(
                    service.try_workspace_file_titles(a, w, ids).unwrap(),
                    original_titles(&service, a, w, ids)
                );
            }
        }
    }
}
#[test]
fn invalid_title_requests_allocate_nothing_and_never_return_partial_data() {
    let service = fixture();
    for (a, w, ids) in [
        (ACCESS, WorkspaceId(10), vec![1, 4]),
        (ACCESS, WorkspaceId(10), vec![3]),
        (ACCESS, WorkspaceId(10), vec![6]),
        (ACCESS, WorkspaceId(10), vec![99]),
        (ACCESS, WorkspaceId(20), vec![1]),
        (AccessContext::anonymous(), WorkspaceId(10), vec![1]),
    ] {
        let (result, meter, baseline) =
            measured(1, true, || service.try_workspace_file_titles(a, w, &ids));
        assert_eq!(result, Ok(None));
        assert_eq!(meter.calls, 0);
        assert_eq!(meter.denials, 0);
        assert_eq!(live(), baseline);
    }
    let (result, meter, baseline) = measured(1, true, || {
        service.try_workspace_file_titles(ACCESS, WorkspaceId(10), &[])
    });
    assert_eq!(result, Ok(Some(vec![])));
    assert_eq!(meter.calls, 0);
    assert_eq!(live(), baseline);
}
#[test]
fn every_actual_title_output_allocation_returns_error_cleans_up_and_retries() {
    let service = fixture();
    let ids = [2, 1, 2];
    let expected = original_titles(&service, ACCESS, WorkspaceId(10), &ids);
    let (result, meter, baseline) = measured(0, false, || {
        service.try_workspace_file_titles(ACCESS, WorkspaceId(10), &ids)
    });
    assert_eq!(result.as_ref().unwrap(), &expected);
    assert_eq!(
        meter.calls, 4,
        "only output Vec and three title strings may allocate"
    );
    drop(result);
    assert_eq!(live(), baseline);
    for persistent in [false, true] {
        for ordinal in 1..=meter.calls {
            let (result, failed, baseline) = measured(ordinal, persistent, || {
                service.try_workspace_file_titles(ACCESS, WorkspaceId(10), &ids)
            });
            assert_eq!(result, Err(MetadataStoreError::Backend(BackendError::Io)));
            assert!(failed.denials > 0);
            drop(result);
            assert_eq!(live(), baseline);
            assert_eq!(
                service
                    .try_workspace_file_titles(ACCESS, WorkspaceId(10), &ids)
                    .unwrap(),
                expected
            );
        }
    }
}

#[test]
fn scalar_stale_scan_matches_legacy_including_tombstones_and_order() {
    let service = fixture();
    let present = alloc::collections::BTreeSet::from([String::from("k1"), String::from("shared")]);
    let expected: Vec<_> = service
        .producer_records("producer", AppId(7), AppSessionId(8))
        .into_iter()
        .filter(|r| !present.contains(r.attributes.get(adapters::PRODUCER_KEY_ATTRIBUTE).unwrap()))
        .map(|r| (r.object_id, r.modified_at))
        .collect();
    let actual = service
        .try_stale_producer_records("producer", AppId(7), AppSessionId(8), &present)
        .unwrap();
    assert_eq!(actual, expected);
    assert!(
        actual.iter().any(|r| r.0 == ObjectId(3)),
        "trusted scan retains tombstones"
    );
    let (result, meter, baseline) = measured(1, true, || {
        service.try_stale_producer_records("wrong", AppId(7), AppSessionId(8), &present)
    });
    assert_eq!(result, Ok(vec![]));
    assert_eq!(meter.calls, 0);
    assert_eq!(live(), baseline);
}
#[test]
fn every_stale_stage_allocation_returns_error_and_drops_partial_scalars() {
    let service = fixture();
    let present = alloc::collections::BTreeSet::new();
    let (result, meter, baseline) = measured(0, false, || {
        service.try_stale_producer_records("producer", AppId(7), AppSessionId(8), &present)
    });
    assert_eq!(result.as_ref().unwrap().len(), 6);
    assert!(meter.calls > 0);
    let calls = meter.calls;
    drop(result);
    assert_eq!(live(), baseline);
    for persistent in [false, true] {
        for ordinal in 1..=calls {
            let (result, meter, baseline) = measured(ordinal, persistent, || {
                service.try_stale_producer_records("producer", AppId(7), AppSessionId(8), &present)
            });
            assert_eq!(result, Err(ProducerScanError::Allocation));
            assert!(meter.denials > 0);
            drop(result);
            assert_eq!(live(), baseline);
            assert_eq!(
                service
                    .try_stale_producer_records("producer", AppId(7), AppSessionId(8), &present)
                    .unwrap()
                    .len(),
                6
            );
        }
    }
    std::println!("production stale stage: {calls} allocation positions x 2 denial modes");
}
#[test]
fn missing_producer_key_is_explicit_and_discards_prepared_deletions() {
    let mut service = fixture();
    let mut record = MetadataRecord::new(ObjectId(8), ObjectKind::File, "missing producer key");
    record.source_app = Some(AppId(7));
    record.source_session = Some(AppSessionId(8));
    record
        .attributes
        .insert(adapters::PRODUCER_ID_ATTRIBUTE.into(), "producer".into());
    service.upsert_record(record).unwrap();
    let present = alloc::collections::BTreeSet::new();
    let (result, meter, baseline) = measured(0, false, || {
        service.try_stale_producer_records("producer", AppId(7), AppSessionId(8), &present)
    });
    assert_eq!(result, Err(ProducerScanError::MissingProducerKey));
    assert!(meter.calls > 0);
    assert_eq!(live(), baseline);
    assert_eq!(
        service
            .producer_records_ref("producer", AppId(7), AppSessionId(8))
            .count(),
        7
    );
    assert!(service.get_object_ref(ACCESS, ObjectId(8)).is_some());
}

#[test]
fn semantic_top_k_denial_returns_capacity_without_leaks_and_retries() {
    let mut index = PersistentVectorIndex::open(Memory::default()).unwrap();
    let space = EmbeddingSpaceId([0x24; 32]);
    for id in 1..=128 {
        let embedding =
            Embedding::try_from_values_in_space(vec![id as f32, 129.0 - id as f32], space).unwrap();
        index
            .replace_object(
                ObjectId(id),
                &[IndexedChunk {
                    chunk: TextChunk {
                        object_id: ObjectId(id),
                        ordinal: 0,
                        start_byte: 0,
                        end_byte: 1,
                        text: "x".into(),
                    },
                    embedding,
                }],
            )
            .unwrap();
    }
    let query = Embedding::try_from_values_in_space(vec![1.0, 0.0], space).unwrap();
    let permitted: Vec<_> = (1..=128)
        .rev()
        .map(ObjectId)
        .chain([ObjectId(128), ObjectId(1)])
        .collect();
    for limit in [1, 2, MAX_SEMANTIC_RESULTS] {
        let expected = index.search(&query, &permitted, limit).unwrap();
        let (result, meter, baseline) =
            measured(0, false, || index.search(&query, &permitted, limit));
        assert_eq!(result.as_ref().unwrap(), &expected);
        assert_eq!(meter.calls, 2, "allowlist scratch and bounded output only");
        assert_eq!(result.as_ref().unwrap().capacity(), limit);
        drop(result);
        assert_eq!(live(), baseline);
        for persistent in [false, true] {
            for ordinal in 1..=2 {
                let (result, meter, baseline) = measured(ordinal, persistent, || {
                    index.search(&query, &permitted, limit)
                });
                assert_eq!(result, Err(VectorIndexError::Capacity));
                assert_eq!(meter.calls, ordinal);
                assert_eq!(meter.denials, 1);
                drop(result);
                assert_eq!(live(), baseline);
                assert_eq!(index.search(&query, &permitted, limit).unwrap(), expected);
            }
        }
        // Refuse all requests after scratch/output reservation: heap operations
        // and the final in-place sort must not allocate.
        let (result, meter, baseline) =
            measured(3, true, || index.search(&query, &permitted, limit));
        assert_eq!(result.as_ref().unwrap(), &expected);
        assert_eq!(meter.calls, 2);
        assert_eq!(meter.denials, 0);
        drop(result);
        assert_eq!(live(), baseline);
    }
    let large = vec![ObjectId(1); crate::store::MAX_OBJECT_RECORDS];
    let expected = index.search(&query, &large, MAX_SEMANTIC_RESULTS).unwrap();
    assert_eq!(expected.len(), 1);
    assert_eq!(
        expected.capacity(),
        1,
        "deduplication bounds the output reservation"
    );
    for persistent in [false, true] {
        for ordinal in 1..=2 {
            let (result, meter, baseline) = measured(ordinal, persistent, || {
                index.search(&query, &large, MAX_SEMANTIC_RESULTS)
            });
            assert_eq!(result, Err(VectorIndexError::Capacity));
            assert_eq!(meter.calls, ordinal);
            assert_eq!(meter.denials, 1);
            drop(result);
            assert_eq!(live(), baseline);
            assert_eq!(
                index.search(&query, &large, MAX_SEMANTIC_RESULTS).unwrap(),
                expected
            );
        }
    }

    let (result, meter, baseline) = measured(1, true, || index.search(&query, &[], 1));
    assert_eq!(result, Ok(vec![]));
    assert_eq!(meter.calls, 0);
    drop(result);
    assert_eq!(live(), baseline);
    let (result, meter, baseline) = measured(2, true, || index.search(&query, &[ObjectId(999)], 1));
    assert_eq!(result, Ok(vec![]));
    assert_eq!(
        meter.calls, 1,
        "no output allocation for disjoint allowlist"
    );
    drop(result);
    assert_eq!(live(), baseline);
}

#[test]
fn semantic_search_validation_and_empty_index_need_no_allocations() {
    let index = PersistentVectorIndex::open(Memory::default()).unwrap();
    let query =
        Embedding::try_from_values_in_space(vec![1.0, 0.0], EmbeddingSpaceId([0x24; 32])).unwrap();
    let too_many = vec![ObjectId(1); crate::store::MAX_OBJECT_RECORDS + 1];
    for (permitted, limit, expected) in [
        (&[][..], 1, Ok(vec![])),
        (&[ObjectId(1)][..], 1, Ok(vec![])),
        (&too_many[..], 1, Err(VectorIndexError::Capacity)),
        (&too_many[..], 0, Err(VectorIndexError::InvalidLimit)),
        (
            &[][..],
            MAX_SEMANTIC_RESULTS + 1,
            Err(VectorIndexError::InvalidLimit),
        ),
    ] {
        let (result, meter, baseline) =
            measured(1, true, || index.search(&query, permitted, limit));
        assert_eq!(result, expected);
        assert_eq!(meter.calls, 0);
        drop(result);
        assert_eq!(live(), baseline);
    }
}

#[path = "semantic_staging_tests.rs"]
mod semantic_staging_tests;
