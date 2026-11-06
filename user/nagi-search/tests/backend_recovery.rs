//! Host fault model around the real two-slot backend; no guest or real media.
use nagi_model::{AppId, AppSessionId, ObjectId, WorkspaceId};
use nagi_search::{
    BackendError, DenyAllVisibility, Embedding, EmbeddingSpaceId, GuestSnapshotBackend,
    IndexedChunk, MetadataRecord, MetadataStoreError, ObjectKind, PersistentVectorIndex,
    PersistentVectorIndexError, Relation, RelationKind, RelationProvenance, SearchService,
    SnapshotBackend, SnapshotFile, SnapshotFileStore, SnapshotSlot, TextChunk, VectorIndex,
    VectorIndexError, Workspace, WorkspaceSession, GUEST_FILE_BYTES,
};
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FileBytes {
    bytes: [u8; GUEST_FILE_BYTES],
    length: usize,
}
type Image = [[Option<FileBytes>; 5]; 2];
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fault {
    None,
    ManifestBefore,
    ManifestAfter,
    FlushBefore(usize),
    FlushAfter(usize),
}
struct Disk {
    visible: Image,
    durable: Image,
    fault: Fault,
    deny_reads: bool,
    invalid_length: bool,
    deny_flush: bool,
    writes: usize,
    removes: usize,
    flushes: usize,
}
#[derive(Clone)]
struct Files(Arc<Mutex<Disk>>);
impl Files {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(Disk {
            visible: [[None; 5]; 2],
            durable: [[None; 5]; 2],
            fault: Fault::None,
            deny_reads: false,
            invalid_length: false,
            deny_flush: false,
            writes: 0,
            removes: 0,
            flushes: 0,
        })))
    }
    fn arm(&self, fault: Fault) {
        let mut d = self.0.lock().unwrap();
        d.fault = fault;
        d.writes = 0;
        d.removes = 0;
        d.flushes = 0;
    }
    fn image(&self) -> (Image, Image) {
        let d = self.0.lock().unwrap();
        (d.visible, d.durable)
    }
    fn effects(&self) -> (usize, usize, usize) {
        let d = self.0.lock().unwrap();
        (d.writes, d.removes, d.flushes)
    }
}
fn key(slot: SnapshotSlot, file: SnapshotFile) -> (usize, usize) {
    (
        usize::from(slot == SnapshotSlot::B),
        match file {
            SnapshotFile::Manifest => 0,
            SnapshotFile::Chunk(i) => usize::from(i) + 1,
        },
    )
}
impl SnapshotFileStore for Files {
    fn read_file(
        &mut self,
        slot: SnapshotSlot,
        file: SnapshotFile,
        buffer: &mut [u8; GUEST_FILE_BYTES],
    ) -> Result<Option<usize>, BackendError> {
        let d = self.0.lock().unwrap();
        if d.deny_reads {
            return Err(BackendError::Io);
        }
        if d.invalid_length {
            return Ok(Some(GUEST_FILE_BYTES + 1));
        }
        let (s, f) = key(slot, file);
        let Some(value) = &d.visible[s][f] else {
            return Ok(None);
        };
        buffer[..value.length].copy_from_slice(&value.bytes[..value.length]);
        Ok(Some(value.length))
    }
    fn write_file(
        &mut self,
        slot: SnapshotSlot,
        file: SnapshotFile,
        bytes: &[u8],
    ) -> Result<(), BackendError> {
        let mut d = self.0.lock().unwrap();
        d.writes += 1;
        let manifest = file == SnapshotFile::Manifest;
        if manifest && d.fault == Fault::ManifestBefore {
            d.fault = Fault::None;
            return Err(BackendError::Io);
        }
        let (s, f) = key(slot, file);
        let mut value = FileBytes {
            bytes: [0; GUEST_FILE_BYTES],
            length: bytes.len(),
        };
        value.bytes[..bytes.len()].copy_from_slice(bytes);
        d.visible[s][f] = Some(value);
        if manifest && d.fault == Fault::ManifestAfter {
            d.fault = Fault::None;
            return Err(BackendError::Io);
        }
        Ok(())
    }
    fn remove_file(&mut self, slot: SnapshotSlot, file: SnapshotFile) -> Result<(), BackendError> {
        let mut d = self.0.lock().unwrap();
        d.removes += 1;
        let (s, f) = key(slot, file);
        d.visible[s][f] = None;
        Ok(())
    }
    fn flush(&mut self) -> Result<(), BackendError> {
        let mut d = self.0.lock().unwrap();
        d.flushes += 1;
        if d.deny_flush {
            return Err(BackendError::Io);
        }
        if d.fault == Fault::FlushBefore(d.flushes) {
            d.fault = Fault::None;
            return Err(BackendError::Io);
        }
        d.durable = d.visible;
        if d.fault == Fault::FlushAfter(d.flushes) {
            d.fault = Fault::None;
            return Err(BackendError::Io);
        }
        Ok(())
    }
}
fn record(id: u64) -> MetadataRecord {
    MetadataRecord::new(ObjectId(id), ObjectKind::File, "record")
}

#[test]
fn manifest_flush_failure_blocks_the_next_metadata_mutation() {
    let files = Files::new();
    let mut service =
        SearchService::open(GuestSnapshotBackend::new(files.clone()), DenyAllVisibility).unwrap();
    service.upsert_record(record(1)).unwrap();
    files.arm(Fault::FlushBefore(2));
    assert!(service.upsert_record(record(2)).is_err());
    let saved = files.image();
    let effects = files.effects();
    assert!(
        service.upsert_record(record(3)).is_err(),
        "stale-memory mutation was accepted after an uncertain manifest flush"
    );
    assert_eq!(files.effects(), effects, "blocked mutation reached storage");
    assert_eq!(
        files.image(),
        saved,
        "blocked mutation changed stored bytes"
    );
}

#[test]
fn metadata_recovery_requires_fence_and_complete_reload() {
    for fault in [
        Fault::ManifestBefore,
        Fault::ManifestAfter,
        Fault::FlushBefore(2),
        Fault::FlushAfter(2),
    ] {
        let files = Files::new();
        let mut service =
            SearchService::open(GuestSnapshotBackend::new(files.clone()), DenyAllVisibility)
                .unwrap();
        service.upsert_record(record(1)).unwrap();
        files.arm(fault);
        let blocked = MetadataStoreError::Backend(BackendError::RecoveryRequired);
        assert_eq!(service.upsert_record(record(2)), Err(blocked));
        assert!(service.requires_reload());
        assert_eq!(
            service.next_object_id(),
            Some(ObjectId(2)),
            "last confirmed view remains installed"
        );
        let saved = files.image();
        let effects = files.effects();
        assert_eq!(service.upsert_record(record(3)), Err(blocked));
        assert_eq!(service.remove_record(ObjectId(1), 1), Err(blocked));
        assert_eq!(service.remove_record(ObjectId(99), 1), Err(blocked));
        assert_eq!(service.remove_workspace(WorkspaceId(99)), Err(blocked));
        assert_eq!(
            service.upsert_workspace(Workspace::new(WorkspaceId(99), "workspace")),
            Err(blocked)
        );
        assert_eq!(
            service.add_workspace_object(WorkspaceId(99), ObjectId(1)),
            Err(blocked)
        );
        assert_eq!(
            service.remove_workspace_object(WorkspaceId(99), ObjectId(1)),
            Err(blocked)
        );
        assert_eq!(
            service.link_session(
                WorkspaceId(99),
                WorkspaceSession {
                    app_id: AppId(1),
                    session_id: AppSessionId(1)
                }
            ),
            Err(blocked)
        );
        let relation = Relation {
            source: ObjectId(1),
            target: ObjectId(2),
            kind: RelationKind::RelatedTo,
            provenance: RelationProvenance::User,
        };
        assert_eq!(service.add_relation(relation), Err(blocked));
        assert_eq!(service.remove_relation(relation), Err(blocked));
        assert_eq!(files.image(), saved);
        assert_eq!(files.effects(), effects);
        files.0.lock().unwrap().deny_flush = true;
        assert_eq!(service.reload(), Err(blocked));
        assert!(service.requires_reload());
        assert_eq!(files.image(), saved);
        files.0.lock().unwrap().deny_flush = false;
        files.0.lock().unwrap().deny_reads = true;
        assert_eq!(
            service.reload(),
            Err(MetadataStoreError::Backend(BackendError::Io))
        );
        assert!(service.requires_reload());
        let saved = files.image();
        let effects = files.effects();
        assert_eq!(service.upsert_record(record(3)), Err(blocked));
        assert_eq!(files.image(), saved);
        assert_eq!(files.effects(), effects);
        files.0.lock().unwrap().deny_reads = false;
        service.reload().unwrap();
        assert!(!service.requires_reload());
        assert_eq!(
            service.next_object_id(),
            Some(ObjectId(if fault == Fault::ManifestBefore { 2 } else { 3 }))
        );
        service.upsert_record(record(3)).unwrap();
        let reopened =
            SearchService::open(GuestSnapshotBackend::new(files), DenyAllVisibility).unwrap();
        assert_eq!(reopened.next_object_id(), Some(ObjectId(4)));
    }
}

fn chunk(id: u64) -> Vec<IndexedChunk> {
    vec![IndexedChunk {
        chunk: TextChunk {
            object_id: ObjectId(id),
            ordinal: 0,
            start_byte: 0,
            end_byte: 4,
            text: "text".into(),
        },
        embedding: Embedding::try_from_values_in_space(vec![1., 0.], EmbeddingSpaceId([1; 32]))
            .unwrap(),
    }]
}
#[test]
fn semantic_store_and_direct_backend_block_uncertain_commits() {
    for fault in [
        Fault::ManifestBefore,
        Fault::ManifestAfter,
        Fault::FlushBefore(2),
        Fault::FlushAfter(2),
    ] {
        let files = Files::new();
        let mut index =
            PersistentVectorIndex::open(GuestSnapshotBackend::new(files.clone())).unwrap();
        index.replace_object(ObjectId(1), &chunk(1)).unwrap();
        files.arm(fault);
        assert_eq!(
            index.replace_object(ObjectId(2), &chunk(2)),
            Err(VectorIndexError::RecoveryRequired)
        );
        assert!(index.requires_reload());
        let saved = files.image();
        let effects = files.effects();
        assert_eq!(
            index.replace_object(ObjectId(3), &chunk(3)),
            Err(VectorIndexError::RecoveryRequired)
        );
        assert_eq!(
            index.remove_object(ObjectId(1)),
            Err(VectorIndexError::RecoveryRequired)
        );
        assert_eq!(
            index.remove_object(ObjectId(99)),
            Err(VectorIndexError::RecoveryRequired)
        );
        assert_eq!(files.image(), saved);
        assert_eq!(files.effects(), effects);
        files.0.lock().unwrap().deny_flush = true;
        assert_eq!(
            index.reload(),
            Err(PersistentVectorIndexError::Backend(
                BackendError::RecoveryRequired
            ))
        );
        assert!(index.requires_reload());
        files.0.lock().unwrap().deny_flush = false;
        index.reload().unwrap();
        assert!(!index.requires_reload());
        let found = index
            .search(&chunk(1)[0].embedding, &[ObjectId(1), ObjectId(2)], 2)
            .unwrap();
        assert_eq!(
            found.len(),
            if fault == Fault::ManifestBefore { 1 } else { 2 }
        );
        index.replace_object(ObjectId(3), &chunk(3)).unwrap();

        // The backend's own guard also protects direct users without a store.
        let mut backend = GuestSnapshotBackend::new(files.clone());
        files.arm(fault);
        assert_eq!(
            backend.write_snapshot(b"unknown"),
            Err(BackendError::RecoveryRequired)
        );
        let saved = files.image();
        let effects = files.effects();
        assert_eq!(
            backend.write_snapshot(b"blocked"),
            Err(BackendError::RecoveryRequired)
        );
        assert_eq!(files.image(), saved);
        assert_eq!(files.effects(), effects);
        backend.load_snapshot().unwrap();
        backend.write_snapshot(b"recovered").unwrap();
        assert_eq!(
            backend.load_snapshot().unwrap(),
            Some(b"recovered".to_vec())
        );
    }
}

#[test]
fn pre_manifest_flush_failure_is_retryable_without_publication() {
    let files = Files::new();
    let mut service =
        SearchService::open(GuestSnapshotBackend::new(files.clone()), DenyAllVisibility).unwrap();
    service.upsert_record(record(1)).unwrap();
    files.arm(Fault::FlushBefore(1));
    assert_eq!(
        service.upsert_record(record(2)),
        Err(MetadataStoreError::Backend(BackendError::Io))
    );
    assert!(!service.requires_reload());
    let reopened =
        SearchService::open(GuestSnapshotBackend::new(files.clone()), DenyAllVisibility).unwrap();
    assert_eq!(reopened.next_object_id(), Some(ObjectId(2)));
    service.upsert_record(record(3)).unwrap();
    assert_eq!(service.next_object_id(), Some(ObjectId(4)));
}

// Real allocator null denial is scoped to the calling thread. Subprocesses
// make an accidental infallible allocation a visible failing child, not UB.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};
thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static REMAINING: Cell<isize> = const { Cell::new(-1) };
    static PERSISTENT: Cell<bool> = const { Cell::new(false) };
    static ATTEMPTS: Cell<usize> = const { Cell::new(0) };
    static FAILURES: Cell<usize> = const { Cell::new(0) };
    static BALANCE: Cell<isize> = const { Cell::new(0) };
}
fn deny() -> bool {
    if !ARMED.try_with(Cell::get).unwrap_or(false) {
        return false;
    }
    ATTEMPTS.with(|v| v.set(v.get() + 1));
    REMAINING.with(|v| {
        if v.get() == 0 {
            FAILURES.with(|f| f.set(f.get() + 1));
            if !PERSISTENT.with(Cell::get) {
                v.set(-1);
            }
            true
        } else {
            if v.get() > 0 {
                v.set(v.get() - 1);
            }
            false
        }
    })
}
fn account(delta: isize) {
    if ARMED.try_with(Cell::get).unwrap_or(false) {
        BALANCE.with(|v| v.set(v.get() + delta));
    }
}
struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if deny() {
            return std::ptr::null_mut();
        }
        let p = unsafe { System.alloc(l) };
        if !p.is_null() {
            account(l.size() as isize);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if deny() {
            return std::ptr::null_mut();
        }
        let p = unsafe { System.alloc_zeroed(l) };
        if !p.is_null() {
            account(l.size() as isize);
        }
        p
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if deny() {
            return std::ptr::null_mut();
        }
        let next = unsafe { System.realloc(p, l, n) };
        if !next.is_null() {
            account(n as isize - l.size() as isize);
        }
        next
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        account(-(l.size() as isize));
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
fn arm_alloc(nth: isize, persistent: bool) {
    REMAINING.with(|v| v.set(nth));
    PERSISTENT.with(|v| v.set(persistent));
    ATTEMPTS.with(|v| v.set(0));
    FAILURES.with(|v| v.set(0));
    BALANCE.with(|v| v.set(0));
    ARMED.with(|v| v.set(true));
}
fn disarm_alloc() -> (usize, usize, isize) {
    ARMED.with(|v| v.set(false));
    (
        ATTEMPTS.with(Cell::get),
        FAILURES.with(Cell::get),
        BALANCE.with(Cell::get),
    )
}
fn prepared(kind: &str) -> Files {
    let files = Files::new();
    let mut backend = GuestSnapshotBackend::new(files.clone());
    if kind != "missing" {
        let bytes = [0x53; 4096];
        backend
            .write_snapshot(if kind == "empty" { &[] } else { &bytes })
            .unwrap();
        if !matches!(kind, "empty" | "one") {
            backend.write_snapshot(&bytes[..2500]).unwrap();
        }
    }
    {
        let mut d = files.0.lock().unwrap();
        match kind {
            "corrupt-manifest" => d.visible[1][0].as_mut().unwrap().bytes[20] ^= 1,
            "corrupt-payload" => d.visible[1][1].as_mut().unwrap().bytes[0] ^= 1,
            "missing-chunk" => d.visible[1][1] = None,
            "both-corrupt" => {
                for slot in &mut d.visible {
                    slot[0].as_mut().unwrap().bytes[0] = 0;
                }
            }
            "io" => d.deny_reads = true,
            "invalid-length" => d.invalid_length = true,
            _ => {}
        }
        d.durable = d.visible;
    }
    files.arm(Fault::None);
    files
}

#[test]
#[ignore]
fn backend_load_child() {
    let kind = std::env::var("BACKEND_CASE").unwrap();
    let nth: isize = std::env::var("BACKEND_NTH").unwrap().parse().unwrap();
    let persistent = std::env::var("BACKEND_PERSISTENT").unwrap() == "1";
    if kind.starts_with("host-") {
        host_load_child(&kind, nth, persistent);
        return;
    }
    let files = prepared(&kind);
    let mut backend = GuestSnapshotBackend::new(files.clone());
    let expected = backend.load_snapshot();
    let independent = match kind.as_str() {
        "missing" => Ok(None),
        "empty" => Ok(Some(Vec::new())),
        "one" | "corrupt-manifest" | "corrupt-payload" | "missing-chunk" => {
            Ok(Some(vec![0x53; 4096]))
        }
        "two" => Ok(Some(vec![0x53; 2500])),
        _ => Err(BackendError::Io),
    };
    assert_eq!(expected, independent);
    let saved = files.image();
    arm_alloc(nth, persistent);
    let result = backend.load_snapshot();
    let (attempts, failures, balance) = disarm_alloc();
    if failures > 0 {
        assert_eq!(format!("{result:?}"), "Err(Capacity)");
        assert_eq!(balance, 0, "partial load leaked an allocation");
    } else {
        assert_eq!(result, expected);
    }
    assert_eq!(files.image(), saved);
    assert_eq!(files.effects(), (0, 0, 0));
    assert_eq!(backend.load_snapshot(), expected);
    assert_eq!(files.image(), saved);
    assert_eq!(files.effects(), (0, 0, 0));
    println!("LOAD attempts={attempts} failures={failures}");
}
fn run_load(kind: &str, nth: usize, persistent: bool) -> String {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "backend_load_child", "--ignored", "--nocapture"])
        .env("BACKEND_CASE", kind)
        .env("BACKEND_NTH", nth.to_string())
        .env("BACKEND_PERSISTENT", if persistent { "1" } else { "0" })
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{kind} nth={nth} persistent={persistent}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
#[test]
fn all_backend_load_allocations_are_fallible_read_only_and_retryable() {
    for kind in [
        "missing",
        "empty",
        "one",
        "two",
        "corrupt-manifest",
        "corrupt-payload",
        "missing-chunk",
        "both-corrupt",
        "io",
        "invalid-length",
        "host-valid",
        "host-empty",
        "host-missing",
        "host-relative",
        "host-unicode",
        "host-long",
    ] {
        let normal = run_load(kind, 1_000_000, false);
        let count: usize = normal
            .split("LOAD attempts=")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        for persistent in [false, true] {
            for nth in 0..count.max(1) {
                run_load(kind, nth, persistent);
            }
        }
        println!("{kind}: {count} load allocations checked in both denial modes");
    }
}
#[test]
fn recovery_load_oom_keeps_both_guards_latched() {
    let files = Files::new();
    let mut service =
        SearchService::open(GuestSnapshotBackend::new(files.clone()), DenyAllVisibility).unwrap();
    service.upsert_record(record(1)).unwrap();
    files.arm(Fault::FlushBefore(2));
    assert!(service.upsert_record(record(2)).is_err());
    arm_alloc(0, true);
    let result = service.reload();
    let (_, failures, balance) = disarm_alloc();
    assert_eq!(
        result,
        Err(MetadataStoreError::Backend(BackendError::Capacity))
    );
    assert_eq!(failures, 1);
    assert_eq!(balance, 0);
    assert!(service.requires_reload());
    let saved = files.image();
    let effects = files.effects();
    assert!(service.upsert_record(record(3)).is_err());
    assert_eq!(files.image(), saved);
    assert_eq!(files.effects(), effects);
    service.reload().unwrap();
    service.upsert_record(record(3)).unwrap();

    let mut backend = GuestSnapshotBackend::new(files.clone());
    files.arm(Fault::ManifestAfter);
    assert_eq!(
        backend.write_snapshot(b"uncertain"),
        Err(BackendError::RecoveryRequired)
    );
    arm_alloc(0, true);
    let result = backend.load_snapshot();
    let (_, failures, balance) = disarm_alloc();
    assert_eq!(result, Err(BackendError::Capacity));
    assert_eq!(failures, 1);
    assert_eq!(balance, 0);
    let saved = files.image();
    let effects = files.effects();
    assert_eq!(
        backend.write_snapshot(b"blocked"),
        Err(BackendError::RecoveryRequired)
    );
    assert_eq!(files.image(), saved);
    assert_eq!(files.effects(), effects);
    assert_eq!(
        backend.load_snapshot().unwrap(),
        Some(b"uncertain".to_vec())
    );
    backend.write_snapshot(b"recovered").unwrap();
}

// HostFileBackend is a host reference, not the Nagi VFS. These temporary
// files exercise its real read algorithm, not guest/media durability.
fn host_load_child(kind: &str, nth: isize, persistent: bool) {
    let directory = std::env::temp_dir().join(format!("nagi-backend-load-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let mut storage = directory.clone();
    if kind == "host-long" {
        for index in 0..8 {
            storage.push(format!("segment-{index:02}-{}", "a".repeat(32)));
        }
        std::fs::create_dir_all(&storage).unwrap();
    }
    let path = storage.join(if kind == "host-unicode" {
        "snapshot-日本語-🦀"
    } else {
        "snapshot"
    });
    let bytes = if kind == "host-empty" {
        Vec::new()
    } else {
        vec![0x27; 20_000]
    };
    let present = kind != "host-missing";
    if present {
        std::fs::write(&path, &bytes).unwrap();
    }
    let modified = present.then(|| std::fs::metadata(&path).unwrap().modified().unwrap());
    let previous_directory = if kind == "host-relative" {
        let previous = std::env::current_dir().unwrap();
        std::env::set_current_dir(&directory).unwrap();
        Some(previous)
    } else {
        None
    };
    let backend_path = if kind == "host-relative" {
        std::path::PathBuf::from("snapshot")
    } else {
        path.clone()
    };
    let mut backend = nagi_search::host::HostFileBackend::new(backend_path);
    arm_alloc(nth, persistent);
    let result = backend.load_snapshot();
    let (attempts, failures, balance) = disarm_alloc();
    if failures > 0 {
        assert_eq!(result, Err(BackendError::Capacity));
        assert_eq!(balance, 0);
    } else {
        assert_eq!(result, Ok(present.then(|| bytes.clone())));
    }
    assert_eq!(
        backend.load_snapshot().unwrap(),
        present.then(|| bytes.clone())
    );
    if present {
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified.unwrap()
        );
    } else {
        assert!(!path.exists());
    }
    if let Some(previous) = previous_directory {
        std::env::set_current_dir(previous).unwrap();
    }
    std::fs::remove_dir_all(&directory).unwrap();
    println!("LOAD attempts={attempts} failures={failures}");
}

#[test]
fn failed_decode_does_not_unlock_either_store() {
    let files = Files::new();
    let mut service =
        SearchService::open(GuestSnapshotBackend::new(files.clone()), DenyAllVisibility).unwrap();
    service.upsert_record(record(1)).unwrap();
    // Inject a valid storage envelope containing an invalid metadata payload.
    let mut fixture = GuestSnapshotBackend::new(files.clone());
    let original = fixture.load_snapshot().unwrap().unwrap();
    fixture.write_snapshot(b"invalid payload").unwrap();
    assert_eq!(service.reload(), Err(MetadataStoreError::CorruptSnapshot));
    assert!(service.requires_reload());
    assert_eq!(service.next_object_id(), Some(ObjectId(2)));
    let saved = files.image();
    let effects = files.effects();
    assert!(service.upsert_record(record(2)).is_err());
    assert_eq!(files.image(), saved);
    assert_eq!(files.effects(), effects);
    fixture.write_snapshot(&original).unwrap();
    service.reload().unwrap();
    service.upsert_record(record(2)).unwrap();

    let files = Files::new();
    let mut index = PersistentVectorIndex::open(GuestSnapshotBackend::new(files.clone())).unwrap();
    index.replace_object(ObjectId(1), &chunk(1)).unwrap();
    let mut fixture = GuestSnapshotBackend::new(files.clone());
    let original = fixture.load_snapshot().unwrap().unwrap();
    fixture.write_snapshot(b"invalid payload").unwrap();
    assert_eq!(
        index.reload(),
        Err(PersistentVectorIndexError::CorruptSnapshot)
    );
    assert!(index.requires_reload());
    let saved = files.image();
    let effects = files.effects();
    assert!(index.replace_object(ObjectId(2), &chunk(2)).is_err());
    assert_eq!(files.image(), saved);
    assert_eq!(files.effects(), effects);
    fixture.write_snapshot(&original).unwrap();
    index.reload().unwrap();
    index.replace_object(ObjectId(2), &chunk(2)).unwrap();
}

#[test]
fn host_size_bound_and_io_errors_remain_distinct() {
    let directory = std::env::temp_dir().join(format!("nagi-backend-bound-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("oversized");
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(16 * 1024 * 1024 + 1).unwrap();
    drop(file);
    let mut backend = nagi_search::host::HostFileBackend::new(&path);
    assert_eq!(backend.load_snapshot(), Err(BackendError::SnapshotTooLarge));
    assert_eq!(
        std::fs::metadata(&path).unwrap().len(),
        16 * 1024 * 1024 + 1
    );
    let mut backend = nagi_search::host::HostFileBackend::new(&directory);
    assert_eq!(backend.load_snapshot(), Err(BackendError::Io));
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn write_preflight_oom_never_reaches_file_mutations() {
    let probe = prepared("two");
    let mut backend = GuestSnapshotBackend::new(probe);
    arm_alloc(1_000_000, false);
    let normal = backend.load_snapshot();
    let (count, failures, _) = disarm_alloc();
    assert!(normal.is_ok());
    assert_eq!(failures, 0);
    assert!(count > 0);
    for persistent in [false, true] {
        for nth in 0..count {
            let files = prepared("two");
            let saved = files.image();
            let mut backend = GuestSnapshotBackend::new(files.clone());
            arm_alloc(nth as isize, persistent);
            let result = backend.write_snapshot(b"candidate");
            let (_, failures, balance) = disarm_alloc();
            assert_eq!(result, Err(BackendError::Capacity));
            assert_eq!(failures, 1);
            assert_eq!(balance, 0);
            assert_eq!(files.image(), saved);
            assert_eq!(files.effects(), (0, 0, 0));
            backend.write_snapshot(b"retry").unwrap();
            assert_eq!(backend.load_snapshot().unwrap(), Some(b"retry".to_vec()));
        }
    }
}

// Compile the exact production path preparation on every host, with a fake
// GetFullPathNameW boundary. Native Windows file opening is still exercised by
// the existing real HostFileBackend probes, without narrowing their OOM window.
#[path = "../src/host/windows_path.rs"]
mod windows_path;

#[test]
fn windows_path_preparation_never_allocates_and_preserves_path_semantics() {
    fn check(input: &[u16], normalized: Option<&[u16]>, expected: &[u16]) {
        let mut terminated = input.to_vec();
        terminated.push(0);
        let mut wanted = expected.to_vec();
        wanted.push(0);
        for persistent in [false, true] {
            let mut path = [0; 512];
            let mut absolute = [0; 512];
            let mut calls = 0;
            let mut valid_input = true;
            arm_alloc(0, persistent);
            let result = windows_path::prepare_path(
                input.iter().copied(),
                &mut path,
                &mut absolute,
                |input, output| {
                    calls += 1;
                    valid_input &= input == terminated;
                    if let Some(normalized) = normalized {
                        output[..normalized.len()].copy_from_slice(normalized);
                        output[normalized.len()] = 0;
                        Ok(normalized.len())
                    } else {
                        Err(std::io::Error::from_raw_os_error(123))
                    }
                },
            );
            let (attempts, failures, balance) = disarm_alloc();
            assert_eq!((attempts, failures, balance), (0, 0, 0));
            assert!(valid_input);
            assert_eq!(calls, usize::from(normalized.is_some()));
            assert_eq!(&path[..result.unwrap()], wanted.as_slice());
        }
    }
    let wide = |s: &str| s.encode_utf16().collect::<Vec<_>>();
    for spelling in [
        r"C:\snapshot.",
        r"C:/snapshot ",
        r"\\server\share\snapshot",
        r"//server/share/snapshot",
        r"\\?\C:\snapshot",
        r"\??\C:\snapshot",
        r"C:",
        "",
        r"C:\日本語-🦀",
    ] {
        let path = wide(spelling);
        check(&path, None, &path);
    }
    for (input, normalized, expected) in [
        (
            r"sub\..\snapshot",
            r"C:\cwd\snapshot",
            r"\\?\C:\cwd\snapshot",
        ),
        (
            r"C:snapshot",
            r"C:\drive\snapshot",
            r"\\?\C:\drive\snapshot",
        ),
        (r"\snapshot", r"C:\snapshot", r"\\?\C:\snapshot"),
        (r"device", r"\\.\NUL", r"\\?\NUL"),
        (r"verbatim", r"\\?\C:\snapshot", r"\\?\C:\snapshot"),
    ] {
        check(&wide(input), Some(&wide(normalized)), &wide(expected));
    }
    let long_drive = format!(r"C:\{}\snapshot", "a".repeat(300));
    check(
        &wide(&long_drive),
        Some(&wide(&long_drive)),
        &wide(&format!(r"\\?\{long_drive}")),
    );
    let long_unc = format!(r"\\server\share\{}\snapshot", "a".repeat(300));
    check(
        &wide(&long_unc),
        Some(&wide(&long_unc)),
        &wide(&format!(r"\\?\UNC\{}", &long_unc[2..])),
    );
    // OsStr's unpaired UTF-16 surrogates must not be converted through String.
    let surrogate = [b'C' as u16, b':' as u16, b'\\' as u16, 0xd800];
    check(&surrogate, None, &surrogate);
}

#[test]
fn windows_path_bounds_and_errors_need_no_allocator() {
    for (input, capacity, normalized, error) in [
        (vec![b'a' as u16, 0, b'b' as u16], 16, None, 123),
        (vec![1; 4], 4, None, 206),
        (vec![], 0, None, 206),
        (vec![b'x' as u16], 8, Some(16), 206),
        (vec![b'x' as u16], 8, Some(7), 206),
        (vec![b'x' as u16], 8, None, 5),
    ] {
        let mut path = [0; 16];
        let mut absolute = [0; 16];
        arm_alloc(0, true);
        let result = windows_path::prepare_path(
            input.iter().copied(),
            &mut path[..capacity],
            &mut absolute,
            |_, output| {
                if let Some(length) = normalized {
                    output[..3].copy_from_slice(&[b'C' as u16, b':' as u16, b'\\' as u16]);
                    Ok(length)
                } else {
                    Err(std::io::Error::from_raw_os_error(5))
                }
            },
        );
        let (attempts, failures, balance) = disarm_alloc();
        assert_eq!((attempts, failures, balance), (0, 0, 0));
        assert_eq!(result.unwrap_err().raw_os_error(), Some(error));
    }
    let mut input = vec![b'a' as u16; windows_path::MAX_PATH_UNITS - 1];
    input[..4].copy_from_slice(&[92, 92, 63, 92]);
    let mut path = [0; windows_path::MAX_PATH_UNITS];
    let mut absolute = [0; windows_path::MAX_PATH_UNITS];
    arm_alloc(0, true);
    let result =
        windows_path::prepare_path(input.iter().copied(), &mut path, &mut absolute, |_, _| {
            Err(std::io::Error::from_raw_os_error(5))
        });
    let meter = disarm_alloc();
    assert_eq!(meter, (0, 0, 0));
    assert_eq!(result.unwrap(), windows_path::MAX_PATH_UNITS);
    input.push(b'a' as u16);
    arm_alloc(0, true);
    let result =
        windows_path::prepare_path(input.iter().copied(), &mut path, &mut absolute, |_, _| {
            Err(std::io::Error::from_raw_os_error(5))
        });
    let meter = disarm_alloc();
    assert_eq!(meter, (0, 0, 0));
    assert_eq!(result.unwrap_err().raw_os_error(), Some(206));
}

#[test]
fn host_nul_path_is_rejected_without_allocating_or_opening() {
    let path = std::path::PathBuf::from("bad\0snapshot");
    let mut backend = nagi_search::host::HostFileBackend::new(path);
    for persistent in [false, true] {
        arm_alloc(0, persistent);
        let result = backend.load_snapshot();
        let meter = disarm_alloc();
        assert_eq!(meter, (0, 0, 0));
        assert_eq!(result, Err(BackendError::Io));
    }
}
