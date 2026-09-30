use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};
use libnagi::storage::{
    BlockDevice, DirectoryEntry, FileHandle, StorageError, SyscallBlockDevice, Vfs, BLOCK_SIZE,
    MAX_FILE_SIZE,
};
use nagi_ai::{
    execute_plan, register_file_search_action, validate_plan, ActionPolicy, ActionRegistry,
    CallerIdentity, ContextAuthority, ContextRequest, ContextResolver, ExecutionStatus, NagiPlan,
    ObjectAccess, PolicyDenied,
};
use nagi_model::{AppId, AppSessionId, NodeId, ObjectId, WorkspaceId};
use nagi_model_manager::CapabilityId;
use nagi_search::{
    adapters::{FilesProducerAdapter, ProducerObject},
    AccessContext, Embedding, EmbeddingProvider, EmbeddingPurpose, EmbeddingSpaceId,
    GuestSnapshotBackend, IndexedChunk, MetadataRecord, ObjectKind, PersistentVectorIndex,
    SearchQuery, SearchService, SemanticError, SnapshotFile, SnapshotFileStore, SnapshotSlot,
    VectorIndex, VisibilityFilter, VisibilityScope, Workspace, WorkspaceSession, GUEST_FILE_BYTES,
};

const STORE_ROOT: &[u8] = b"/var/lib/nagi-search";
const SEMANTIC_STORE_ROOT: &[u8] = b"/var/lib/nagi-search-semantic";
const OBJECT_ID: ObjectId = ObjectId(0x4e41_4749_4d19_0001);
const WORKSPACE_ID: WorkspaceId = WorkspaceId(0x4e41_4749_4d19_0002);
const APP_ID: AppId = AppId(0x4e41_4749_4d19_0003);
const SESSION_ID: AppSessionId = AppSessionId(0x4e41_4749_4d19_0004);
const NODE_ID: NodeId = NodeId(0x4e41_4749_4d19_0005);
const ACCESS: AccessContext = AccessContext::for_application(APP_ID, SESSION_ID);
const FILE_ID_BASE: u64 = 0x4e41_4749_4d19_1000;
const FILE_INDEXER_ATTRIBUTE: &str = "nagi.files.indexer";
const FILE_INODE_ATTRIBUTE: &str = "nagi.files.vfs_inode";
const FILE_INDEXER_ID: &str = "m19-vfs-files-fixture";
const LIVE_FILE_SOURCE: &[u8] = b"nagi-m19-live-source.txt";
const LIVE_FILE_RENAMED: &[u8] = b"nagi-m19-live-file.txt";
const LIVE_FILE_CONTENT: &[u8] = b"A real guest VFS file indexed by Nagi Search.\n";
const MAX_M19_ROOT_ENTRIES: usize = 64;
const M24_FIXTURE_HIDDEN_OBJECT: ObjectId = ObjectId(0x4e41_4749_4d24_ffff);
const M24_FIXTURE_SPACE: EmbeddingSpaceId = EmbeddingSpaceId([0x24; 32]);

const _: [(); BLOCK_SIZE] = [(); GUEST_FILE_BYTES];

struct VfsSnapshotFiles<D: BlockDevice> {
    volume: Vfs<D>,
    namespace: SnapshotNamespace,
}

#[derive(Clone, Copy)]
enum SnapshotNamespace {
    Metadata,
    Semantic,
}

impl<D: BlockDevice> VfsSnapshotFiles<D> {
    fn new(
        mut volume: Vfs<D>,
        namespace: SnapshotNamespace,
    ) -> Result<Self, nagi_search::BackendError> {
        let root = match namespace {
            SnapshotNamespace::Metadata => STORE_ROOT,
            SnapshotNamespace::Semantic => SEMANTIC_STORE_ROOT,
        };
        volume
            .ensure_directory_path(b"/var")
            .and_then(|()| volume.ensure_directory_path(b"/var/lib"))
            .and_then(|()| volume.ensure_directory_path(root))
            .map_err(|_| nagi_search::BackendError::Io)?;
        Ok(Self { volume, namespace })
    }

    fn path(&self, slot: SnapshotSlot, file: SnapshotFile) -> Option<&'static [u8]> {
        match (self.namespace, slot, file) {
            (SnapshotNamespace::Metadata, SnapshotSlot::A, SnapshotFile::Manifest) => {
                Some(b"/var/lib/nagi-search/am")
            }
            (SnapshotNamespace::Metadata, SnapshotSlot::A, SnapshotFile::Chunk(0)) => {
                Some(b"/var/lib/nagi-search/a0")
            }
            (SnapshotNamespace::Metadata, SnapshotSlot::A, SnapshotFile::Chunk(1)) => {
                Some(b"/var/lib/nagi-search/a1")
            }
            (SnapshotNamespace::Metadata, SnapshotSlot::A, SnapshotFile::Chunk(2)) => {
                Some(b"/var/lib/nagi-search/a2")
            }
            (SnapshotNamespace::Metadata, SnapshotSlot::A, SnapshotFile::Chunk(3)) => {
                Some(b"/var/lib/nagi-search/a3")
            }
            (SnapshotNamespace::Metadata, SnapshotSlot::B, SnapshotFile::Manifest) => {
                Some(b"/var/lib/nagi-search/bm")
            }
            (SnapshotNamespace::Metadata, SnapshotSlot::B, SnapshotFile::Chunk(0)) => {
                Some(b"/var/lib/nagi-search/b0")
            }
            (SnapshotNamespace::Metadata, SnapshotSlot::B, SnapshotFile::Chunk(1)) => {
                Some(b"/var/lib/nagi-search/b1")
            }
            (SnapshotNamespace::Metadata, SnapshotSlot::B, SnapshotFile::Chunk(2)) => {
                Some(b"/var/lib/nagi-search/b2")
            }
            (SnapshotNamespace::Metadata, SnapshotSlot::B, SnapshotFile::Chunk(3)) => {
                Some(b"/var/lib/nagi-search/b3")
            }
            (SnapshotNamespace::Semantic, SnapshotSlot::A, SnapshotFile::Manifest) => {
                Some(b"/var/lib/nagi-search-semantic/am")
            }
            (SnapshotNamespace::Semantic, SnapshotSlot::A, SnapshotFile::Chunk(0)) => {
                Some(b"/var/lib/nagi-search-semantic/a0")
            }
            (SnapshotNamespace::Semantic, SnapshotSlot::A, SnapshotFile::Chunk(1)) => {
                Some(b"/var/lib/nagi-search-semantic/a1")
            }
            (SnapshotNamespace::Semantic, SnapshotSlot::A, SnapshotFile::Chunk(2)) => {
                Some(b"/var/lib/nagi-search-semantic/a2")
            }
            (SnapshotNamespace::Semantic, SnapshotSlot::A, SnapshotFile::Chunk(3)) => {
                Some(b"/var/lib/nagi-search-semantic/a3")
            }
            (SnapshotNamespace::Semantic, SnapshotSlot::B, SnapshotFile::Manifest) => {
                Some(b"/var/lib/nagi-search-semantic/bm")
            }
            (SnapshotNamespace::Semantic, SnapshotSlot::B, SnapshotFile::Chunk(0)) => {
                Some(b"/var/lib/nagi-search-semantic/b0")
            }
            (SnapshotNamespace::Semantic, SnapshotSlot::B, SnapshotFile::Chunk(1)) => {
                Some(b"/var/lib/nagi-search-semantic/b1")
            }
            (SnapshotNamespace::Semantic, SnapshotSlot::B, SnapshotFile::Chunk(2)) => {
                Some(b"/var/lib/nagi-search-semantic/b2")
            }
            (SnapshotNamespace::Semantic, SnapshotSlot::B, SnapshotFile::Chunk(3)) => {
                Some(b"/var/lib/nagi-search-semantic/b3")
            }
            (_, _, SnapshotFile::Chunk(_)) => None,
        }
    }
}

impl<D: BlockDevice> SnapshotFileStore for VfsSnapshotFiles<D> {
    fn read_file(
        &mut self,
        slot: SnapshotSlot,
        file: SnapshotFile,
        buffer: &mut [u8; GUEST_FILE_BYTES],
    ) -> Result<Option<usize>, nagi_search::BackendError> {
        let path = self.path(slot, file).ok_or(nagi_search::BackendError::Io)?;
        let handle = match self.volume.open_path(path) {
            Ok(handle) => handle,
            Err(StorageError::NotFound) => return Ok(None),
            Err(_) => return Err(nagi_search::BackendError::Io),
        };
        let length = self
            .volume
            .read(handle, buffer)
            .map_err(|_| nagi_search::BackendError::Io)?;
        Ok(Some(length))
    }

    fn write_file(
        &mut self,
        slot: SnapshotSlot,
        file: SnapshotFile,
        bytes: &[u8],
    ) -> Result<(), nagi_search::BackendError> {
        let path = self.path(slot, file).ok_or(nagi_search::BackendError::Io)?;
        let handle = match self.volume.open_path(path) {
            Ok(handle) => handle,
            Err(StorageError::NotFound) => self
                .volume
                .create_path(path)
                .map_err(|_| nagi_search::BackendError::Io)?,
            Err(_) => return Err(nagi_search::BackendError::Io),
        };
        self.volume
            .write(handle, bytes)
            .map_err(|error| match error {
                StorageError::FileTooLarge => nagi_search::BackendError::SnapshotTooLarge,
                _ => nagi_search::BackendError::Io,
            })
    }

    fn remove_file(
        &mut self,
        slot: SnapshotSlot,
        file: SnapshotFile,
    ) -> Result<(), nagi_search::BackendError> {
        let path = self.path(slot, file).ok_or(nagi_search::BackendError::Io)?;
        match self.volume.remove_path(path) {
            Ok(()) | Err(StorageError::NotFound) => Ok(()),
            Err(_) => Err(nagi_search::BackendError::Io),
        }
    }

    fn flush(&mut self) -> Result<(), nagi_search::BackendError> {
        self.volume
            .flush()
            .map_err(|_| nagi_search::BackendError::Io)
    }
}

#[derive(Clone, Copy)]
struct M19AcceptanceVisibility;

impl VisibilityFilter for M19AcceptanceVisibility {
    fn can_read_object(&self, access: AccessContext, record: &MetadataRecord) -> bool {
        access.app_id == Some(APP_ID)
            && access.app_session_id == Some(SESSION_ID)
            && record.visibility == VisibilityScope::Private
            && (record.object_id == OBJECT_ID
                || record
                    .attributes
                    .get(FILE_INDEXER_ATTRIBUTE)
                    .is_some_and(|indexer| indexer == FILE_INDEXER_ID))
    }

    fn can_read_workspace(&self, access: AccessContext, workspace: &Workspace) -> bool {
        access.app_id == Some(APP_ID)
            && access.app_session_id == Some(SESSION_ID)
            && workspace.workspace_id == WORKSPACE_ID
            && workspace.visibility == VisibilityScope::Private
    }

    fn can_read_workspace_session(
        &self,
        access: AccessContext,
        workspace: &Workspace,
        session: WorkspaceSession,
    ) -> bool {
        self.can_read_workspace(access, workspace)
            && access.app_id == Some(session.app_id)
            && access.app_session_id == Some(session.session_id)
    }
}

type M19SearchService = SearchService<
    GuestSnapshotBackend<VfsSnapshotFiles<SyscallBlockDevice>>,
    M19AcceptanceVisibility,
>;
type M24SemanticIndex =
    PersistentVectorIndex<GuestSnapshotBackend<VfsSnapshotFiles<SyscallBlockDevice>>>;

struct M19ActionPolicy {
    live_file: ObjectId,
}

impl M19ActionPolicy {
    fn caller_is_fixture(caller: CallerIdentity) -> bool {
        caller.app_id == APP_ID
            && caller.app_session_id == SESSION_ID
            && caller.node_id == NODE_ID
            && (caller.workspace_id.is_none() || caller.workspace_id == Some(WORKSPACE_ID))
    }

    fn object_is_visible(&self, caller: CallerIdentity, object_id: ObjectId) -> bool {
        Self::caller_is_fixture(caller) && (object_id == OBJECT_ID || object_id == self.live_file)
    }
}

impl ContextAuthority for M19ActionPolicy {
    fn can_read_object(&self, caller: CallerIdentity, object_id: ObjectId) -> bool {
        self.object_is_visible(caller, object_id)
    }

    fn can_read_workspace(&self, caller: CallerIdentity, workspace_id: WorkspaceId) -> bool {
        Self::caller_is_fixture(caller) && workspace_id == WORKSPACE_ID
    }
}

impl ActionPolicy for M19ActionPolicy {
    type CapabilityGrant = ();
    type ObjectHandle = ObjectId;

    fn check_capability(
        &self,
        caller: CallerIdentity,
        capability: &CapabilityId,
    ) -> Result<(), PolicyDenied> {
        if Self::caller_is_fixture(caller) && capability.as_str() == "files.search" {
            Ok(())
        } else {
            Err(PolicyDenied::Capability)
        }
    }

    fn check_object_access(
        &self,
        caller: CallerIdentity,
        object_id: ObjectId,
        access: ObjectAccess,
    ) -> Result<(), PolicyDenied> {
        if access == ObjectAccess::Read && self.object_is_visible(caller, object_id) {
            Ok(())
        } else {
            Err(PolicyDenied::Object)
        }
    }

    fn acquire_capability(
        &self,
        caller: CallerIdentity,
        capability: &CapabilityId,
    ) -> Result<Self::CapabilityGrant, PolicyDenied> {
        self.check_capability(caller, capability)?;
        Ok(())
    }

    fn resolve_object(
        &self,
        caller: CallerIdentity,
        object_id: ObjectId,
        access: ObjectAccess,
    ) -> Result<Self::ObjectHandle, PolicyDenied> {
        self.check_object_access(caller, object_id, access)?;
        Ok(object_id)
    }
}

fn run_file_search_action(service: M19SearchService, live_file: ObjectId) -> bool {
    libnagi::console_write(b"Nagi M21 trace file.search start\r\n");
    let policy = M19ActionPolicy { live_file };
    let caller = CallerIdentity {
        app_id: APP_ID,
        app_session_id: SESSION_ID,
        node_id: NODE_ID,
        workspace_id: Some(WORKSPACE_ID),
    };
    let Ok(context) = ContextResolver.resolve(
        ContextRequest {
            caller,
            selected_object: None,
            candidate_objects: alloc::vec![OBJECT_ID, live_file],
        },
        &policy,
    ) else {
        return false;
    };
    libnagi::console_write(b"Nagi M21 trace context resolved\r\n");
    if !context.contains_object(OBJECT_ID) || !context.contains_object(live_file) {
        return false;
    }

    // This capability and caller policy are private to the M19 guest fixture.
    // Production authority must come from an authenticated user-space service
    // boundary, which is not exposed to applications yet.
    let foreign_caller = CallerIdentity {
        app_id: AppId(APP_ID.0.wrapping_add(1)),
        ..caller
    };
    let Ok(capability) = CapabilityId::new("files.search") else {
        return false;
    };
    if policy.check_capability(foreign_caller, &capability).is_ok() {
        return false;
    }
    libnagi::console_write(b"Nagi M21 trace foreign caller denied\r\n");

    let Ok(plan) = NagiPlan::parse_complete(
        r#"{"plan_version":1,"intent":"find the live VFS fixture","steps":[{"action":"file.search","parameters":{"query":"nagi-m19-live-file.txt"}}]}"#,
    ) else {
        return false;
    };
    libnagi::console_write(b"Nagi M21 trace plan parsed\r\n");
    let mut registry: ActionRegistry<M19ActionPolicy> = ActionRegistry::new();
    if register_file_search_action(&mut registry, service).is_err() {
        return false;
    }
    libnagi::console_write(b"Nagi M21 trace file.search registered\r\n");
    let Ok(validated) = validate_plan(plan, &context, &registry, &policy) else {
        return false;
    };
    libnagi::console_write(b"Nagi M21 trace plan validated\r\n");
    let report = execute_plan(validated, &mut registry, &policy);
    libnagi::console_write(b"Nagi M21 trace plan executed\r\n");
    let passed = report.status == ExecutionStatus::Succeeded
        && report.completed.len() == 1
        && report.completed[0].action_id == "file.search"
        && report.completed[0].object_ids == [live_file];
    if passed {
        libnagi::console_write(b"Nagi M21 file.search Plan Validate Execute PASS\r\n");
    } else {
        libnagi::console_write(b"Nagi M21 file.search Plan Validate Execute FAIL\r\n");
    }
    passed
}

fn open_volume(block_capability: u64) -> Result<Vfs<SyscallBlockDevice>, StorageError> {
    Vfs::mount_or_format(SyscallBlockDevice::new(block_capability)).map(|(volume, _)| volume)
}

fn open_search(block_capability: u64) -> Result<M19SearchService, nagi_search::MetadataStoreError> {
    let volume = open_volume(block_capability)
        .map_err(|_| nagi_search::MetadataStoreError::Backend(nagi_search::BackendError::Io))?;
    let files = VfsSnapshotFiles::new(volume, SnapshotNamespace::Metadata)
        .map_err(nagi_search::MetadataStoreError::Backend)?;
    nagi_search::SearchService::open(GuestSnapshotBackend::new(files), M19AcceptanceVisibility)
}

fn open_semantic_index(
    block_capability: u64,
) -> Result<M24SemanticIndex, nagi_search::PersistentVectorIndexError> {
    let volume = open_volume(block_capability).map_err(|_| {
        nagi_search::PersistentVectorIndexError::Backend(nagi_search::BackendError::Io)
    })?;
    let files = VfsSnapshotFiles::new(volume, SnapshotNamespace::Semantic)
        .map_err(nagi_search::PersistentVectorIndexError::Backend)?;
    PersistentVectorIndex::open(GuestSnapshotBackend::new(files))
}

/// Deterministic test provider for guest persistence and visibility wiring.
/// It is deliberately not an embedding model or natural-language provider.
struct M24FixtureEmbeddingProvider;

impl EmbeddingProvider for M24FixtureEmbeddingProvider {
    fn embed(&self, _purpose: EmbeddingPurpose, text: &str) -> Result<Embedding, SemanticError> {
        let values = if text.contains("Servo") || text.contains("browser") {
            alloc::vec![1.0, 0.0]
        } else {
            alloc::vec![0.0, 1.0]
        };
        Embedding::try_from_values_in_space(values, M24_FIXTURE_SPACE)
    }
}

fn matches_m24_fixture(hits: &[nagi_search::SemanticHit], live_file: ObjectId) -> bool {
    hits.len() == 2
        && hits[0].record.object_id == OBJECT_ID
        && hits[0].similarity > 0.99
        && hits[1].record.object_id == live_file
        && hits[1].similarity < 0.01
}

fn run_m24_semantic_fixture(
    service: &M19SearchService,
    block_capability: u64,
    live_file: ObjectId,
) -> bool {
    libnagi::console_write(b"Nagi M24 trace semantic index start\r\n");
    let mut index = match open_semantic_index(block_capability) {
        Ok(index) => index,
        Err(_) => {
            libnagi::console_write(b"Nagi M24 trace semantic index open FAIL\r\n");
            return false;
        }
    };
    libnagi::console_write(b"Nagi M24 trace semantic index opened\r\n");
    let provider = M24FixtureEmbeddingProvider;
    let query = "The Servo article I looked at yesterday";
    let restored = match service.semantic_search(ACCESS, query, &provider, &index, 2) {
        Ok(hits) => matches_m24_fixture(&hits, live_file),
        Err(_) => {
            libnagi::console_write(b"Nagi M24 trace initial semantic query FAIL\r\n");
            false
        }
    };
    if restored {
        libnagi::console_write(b"Nagi M24 trace semantic index restored\r\n");
    }

    if service
        .index_semantic_text(
            ACCESS,
            OBJECT_ID,
            "The Servo browser article explains the rendering engine.",
            &provider,
            &mut index,
        )
        .is_err()
    {
        libnagi::console_write(b"Nagi M24 trace first passage index FAIL\r\n");
        return false;
    }
    libnagi::console_write(b"Nagi M24 trace first passage indexed\r\n");
    if service
        .index_semantic_text(
            ACCESS,
            live_file,
            "An administrative memo about office scheduling and invoices.",
            &provider,
            &mut index,
        )
        .is_err()
    {
        libnagi::console_write(b"Nagi M24 trace second passage index FAIL\r\n");
        return false;
    }
    libnagi::console_write(b"Nagi M24 trace second passage indexed\r\n");

    // Seed a high-scoring index entry without visible metadata. The real
    // PersistentVectorIndex must exclude it using SearchService's allowlist.
    let hidden_text = "Servo hidden passage";
    let hidden_chunks = match nagi_search::chunk_text(M24_FIXTURE_HIDDEN_OBJECT, hidden_text) {
        Ok(chunks) => chunks,
        Err(_) => {
            libnagi::console_write(b"Nagi M24 trace hidden chunk FAIL\r\n");
            return false;
        }
    };
    let Some(hidden_chunk) = hidden_chunks.into_iter().next() else {
        libnagi::console_write(b"Nagi M24 trace hidden chunk missing\r\n");
        return false;
    };
    let hidden_embedding = match provider.embed(EmbeddingPurpose::Passage, &hidden_chunk.text) {
        Ok(embedding) => embedding,
        Err(_) => {
            libnagi::console_write(b"Nagi M24 trace hidden embedding FAIL\r\n");
            return false;
        }
    };
    if index
        .replace_object(
            M24_FIXTURE_HIDDEN_OBJECT,
            &[IndexedChunk {
                chunk: hidden_chunk,
                embedding: hidden_embedding,
            }],
        )
        .is_err()
    {
        libnagi::console_write(b"Nagi M24 trace hidden index write FAIL\r\n");
        return false;
    }
    libnagi::console_write(b"Nagi M24 trace hidden passage indexed\r\n");

    let hits = match service.semantic_search(ACCESS, query, &provider, &index, 2) {
        Ok(hits) => hits,
        Err(_) => {
            libnagi::console_write(b"Nagi M24 trace semantic query FAIL\r\n");
            return false;
        }
    };
    let passed = matches_m24_fixture(&hits, live_file)
        && !hits
            .iter()
            .any(|hit| hit.record.object_id == M24_FIXTURE_HIDDEN_OBJECT);
    if passed {
        libnagi::console_write(b"Nagi M24 semantic index ready PASS\r\n");
        if restored {
            libnagi::console_write(b"Nagi M24 semantic index persistence PASS\r\n");
        }
    } else {
        libnagi::console_write(b"Nagi M24 trace semantic result mismatch\r\n");
    }
    passed
}

fn fixture_file(volume: &mut Vfs<SyscallBlockDevice>) -> Option<(FileHandle, &'static [u8])> {
    let source = match volume.open(LIVE_FILE_SOURCE) {
        Ok(handle) => Some(handle),
        Err(StorageError::NotFound) => None,
        Err(_) => return None,
    };
    let renamed = match volume.open(LIVE_FILE_RENAMED) {
        Ok(handle) => Some(handle),
        Err(StorageError::NotFound) => None,
        Err(_) => return None,
    };
    let (handle, name) = match (source, renamed) {
        (Some(_), Some(_)) => return None,
        (Some(handle), None) => (handle, LIVE_FILE_SOURCE),
        (None, Some(handle)) => (handle, LIVE_FILE_RENAMED),
        (None, None) => {
            let handle = volume.create(LIVE_FILE_SOURCE).ok()?;
            volume.write(handle, LIVE_FILE_CONTENT).ok()?;
            (handle, LIVE_FILE_SOURCE)
        }
    };
    let mut content = [0; MAX_FILE_SIZE];
    let length = volume.read(handle, &mut content).ok()?;
    (&content[..length] == LIVE_FILE_CONTENT).then_some((handle, name))
}

fn indexed_file_records(service: &M19SearchService) -> Option<Vec<MetadataRecord>> {
    let query = SearchQuery {
        kind: Some(ObjectKind::File),
        limit: MAX_M19_ROOT_ENTRIES,
        ..SearchQuery::default()
    };
    service
        .search(ACCESS, &query)
        .ok()
        .map(|response| response.objects.into_iter().map(|hit| hit.record).collect())
}

struct LiveFileProjection {
    inode: u32,
    name: String,
    modified_at: i64,
}

fn project_live_file(
    volume: &mut Vfs<SyscallBlockDevice>,
    expected_name: &[u8],
) -> Option<LiveFileProjection> {
    let mut entries = [DirectoryEntry::empty(); MAX_M19_ROOT_ENTRIES];
    let count = volume.list_root(&mut entries).ok()?;
    for entry in &entries[..count] {
        if entry.file_type != 1 || entry.name() != expected_name {
            continue;
        }
        let name = core::str::from_utf8(entry.name()).ok()?;
        let mut location = String::from("/");
        location.push_str(name);
        let metadata = volume.metadata_path(location.as_bytes()).ok()?;
        if metadata.inode != entry.inode {
            return None;
        }
        return Some(LiveFileProjection {
            inode: metadata.inode,
            name: String::from(name),
            modified_at: i64::from(metadata.mtime),
        });
    }
    None
}

fn index_live_file(service: &mut M19SearchService, file: LiveFileProjection) -> Option<ObjectId> {
    let existing_records = indexed_file_records(service)?;
    let inode_key = file.inode.to_string();
    let existing = existing_records.iter().find(|record| {
        record
            .attributes
            .get(FILE_INDEXER_ATTRIBUTE)
            .is_some_and(|indexer| indexer == FILE_INDEXER_ID)
            && record
                .attributes
                .get(FILE_INODE_ATTRIBUTE)
                .is_some_and(|value| value == &inode_key)
    });
    let object_id = if let Some(record) = existing {
        record.object_id
    } else {
        let next_id = existing_records
            .iter()
            .filter(|record| {
                record
                    .attributes
                    .get(FILE_INDEXER_ATTRIBUTE)
                    .is_some_and(|indexer| indexer == FILE_INDEXER_ID)
            })
            .map(|record| record.object_id.0)
            .filter(|id| *id >= FILE_ID_BASE)
            .max()
            .unwrap_or(FILE_ID_BASE - 1)
            .checked_add(1)?;
        ObjectId(next_id)
    };
    let mut location = String::from("/");
    location.push_str(&file.name);
    let mut record = FilesProducerAdapter
        .to_record(ProducerObject {
            object_id,
            title: file.name,
            location: Some(location),
            source_app: Some(APP_ID),
            source_session: Some(SESSION_ID),
            created_at: None,
            modified_at: Some(file.modified_at),
            observed_at: None,
            tags: Vec::new(),
            attributes: BTreeMap::new(),
            visibility: VisibilityScope::Private,
        })
        .ok()?;
    record.attributes.insert(
        FILE_INDEXER_ATTRIBUTE.to_string(),
        FILE_INDEXER_ID.to_string(),
    );
    record
        .attributes
        .insert(FILE_INODE_ATTRIBUTE.to_string(), inode_key);
    service.upsert_record(record).ok()?;
    Some(object_id)
}

/// Exercises the real guest VFS persistence adapter with a test-only private
/// fixture and indexes a real file entry from that VFS. The search API is not
/// registered as a production IPC service here; caller authority remains a
/// separate M19 integration requirement.
pub fn run(block_capability: u64) -> bool {
    libnagi::console_write(b"Nagi M19 trace start\r\n");
    let was_persisted = {
        let Ok(mut service) = open_search(block_capability) else {
            return false;
        };
        let previous_record = service.get_object(ACCESS, OBJECT_ID);
        let previous_workspace = service.get_workspace(ACCESS, WORKSPACE_ID);
        let was_persisted = previous_record.is_some() && previous_workspace.is_some();
        if previous_record.is_some() != previous_workspace.is_some()
            || previous_record.is_some_and(|record| record.title != "M19 persisted object")
            || previous_workspace
                .as_ref()
                .is_some_and(|workspace| workspace.objects != [OBJECT_ID])
        {
            return false;
        }
        let mut record = MetadataRecord::new(
            OBJECT_ID,
            ObjectKind::File,
            String::from("M19 persisted object"),
        );
        record.source_app = Some(APP_ID);
        record.source_session = Some(SESSION_ID);
        record.visibility = VisibilityScope::Private;
        if service.upsert_record(record).is_err() {
            return false;
        }

        let mut workspace = Workspace::new(WORKSPACE_ID, "M19 persisted workspace");
        workspace.owner_app = Some(APP_ID);
        workspace.visibility = VisibilityScope::Private;
        workspace.sessions.push(WorkspaceSession {
            app_id: APP_ID,
            session_id: SESSION_ID,
        });
        workspace.objects.push(OBJECT_ID);
        if service.upsert_workspace(workspace).is_err() {
            return false;
        }
        was_persisted
    };
    libnagi::console_write(b"Nagi M19 trace snapshot fixture persisted\r\n");

    let (file_metadata, initial_projection) = {
        let Ok(mut volume) = open_volume(block_capability) else {
            return false;
        };
        let Some((handle, name)) = fixture_file(&mut volume) else {
            return false;
        };
        if volume.flush().is_err() {
            return false;
        }
        let Ok(metadata) = volume.metadata(handle) else {
            return false;
        };
        let Some(projection) = project_live_file(&mut volume, name) else {
            return false;
        };
        (metadata, projection)
    };
    libnagi::console_write(b"Nagi M19 trace real VFS file projected\r\n");

    let object_id_before_rename = {
        let Ok(mut service) = open_search(block_capability) else {
            return false;
        };
        let Some(object_id) = index_live_file(&mut service, initial_projection) else {
            return false;
        };
        object_id
    };
    libnagi::console_write(b"Nagi M19 trace initial ObjectId indexed\r\n");

    let renamed_projection = {
        let Ok(mut volume) = open_volume(block_capability) else {
            return false;
        };
        let Some((handle, name)) = fixture_file(&mut volume) else {
            return false;
        };
        let Ok(metadata) = volume.metadata(handle) else {
            return false;
        };
        if metadata.inode != file_metadata.inode
            || (name == LIVE_FILE_SOURCE
                && volume.rename(LIVE_FILE_SOURCE, LIVE_FILE_RENAMED).is_err())
            || volume.flush().is_err()
        {
            return false;
        }
        let Some(projection) = project_live_file(&mut volume, LIVE_FILE_RENAMED) else {
            return false;
        };
        projection
    };
    libnagi::console_write(b"Nagi M19 trace VFS rename persisted\r\n");

    let object_id_after_rename = {
        let Ok(mut service) = open_search(block_capability) else {
            return false;
        };
        let Some(object_id) = index_live_file(&mut service, renamed_projection) else {
            return false;
        };
        object_id
    };
    if object_id_before_rename != object_id_after_rename {
        return false;
    }
    libnagi::console_write(b"Nagi M19 trace ObjectId stable after rename\r\n");

    let remounted_projection = {
        let Ok(mut volume) = open_volume(block_capability) else {
            return false;
        };
        let Some((handle, name)) = fixture_file(&mut volume) else {
            return false;
        };
        let Ok(metadata) = volume.metadata(handle) else {
            return false;
        };
        if name != LIVE_FILE_RENAMED || metadata.inode != file_metadata.inode {
            return false;
        }
        let Some(projection) = project_live_file(&mut volume, LIVE_FILE_RENAMED) else {
            return false;
        };
        projection
    };
    libnagi::console_write(b"Nagi M19 trace VFS remount verified\r\n");
    let Ok(mut service) = open_search(block_capability) else {
        return false;
    };
    let Some(object_id_after_restart) = index_live_file(&mut service, remounted_projection) else {
        return false;
    };
    libnagi::console_write(b"Nagi M19 trace ObjectId stable after remount\r\n");
    let query = SearchQuery {
        text: Some(String::from("persisted object")),
        workspace: Some(WORKSPACE_ID),
        ..SearchQuery::default()
    };
    let Ok(response) = service.search(ACCESS, &query) else {
        return false;
    };
    let workspace = service.get_workspace(ACCESS, WORKSPACE_ID);
    let file_query = SearchQuery {
        text: Some(String::from("nagi-m19-live-file.txt")),
        kind: Some(ObjectKind::File),
        ..SearchQuery::default()
    };
    let file_response = service.search(ACCESS, &file_query);
    let file_passed = file_response.is_ok_and(|response| {
        response.objects.len() == 1
            && response.objects[0].record.object_id == object_id_after_rename
            && response.objects[0].record.location.as_deref() == Some("/nagi-m19-live-file.txt")
            && response.objects[0]
                .record
                .attributes
                .get(FILE_INODE_ATTRIBUTE)
                == Some(&file_metadata.inode.to_string())
    });
    let semantic_passed =
        run_m24_semantic_fixture(&service, block_capability, object_id_after_rename);
    let action_passed = run_file_search_action(service, object_id_after_rename);
    let passed = action_passed
        && semantic_passed
        && file_passed
        && object_id_before_rename == object_id_after_restart
        && response.objects.len() == 1
        && response.objects[0].record.object_id == OBJECT_ID
        && response.workspace_groups.len() == 1
        && response.workspace_groups[0].object_ids == [OBJECT_ID]
        && workspace.is_some_and(|workspace| workspace.objects == [OBJECT_ID]);
    if !passed {
        libnagi::console_write(b"Nagi M19 trace final query assertion failed\r\n");
    }
    if passed {
        if was_persisted {
            libnagi::console_write(b"Nagi M19 previous-boot snapshot PASS\r\n");
        } else {
            libnagi::console_write(b"Nagi M19 initial snapshot/reopen PASS\r\n");
        }
        libnagi::console_write(b"Nagi M19 live VFS file ObjectId rename/restart PASS\r\n");
        libnagi::console_write(b"Nagi M19 guest search persistence PASS\r\n");
        libnagi::console_write(b"Nagi M19 acceptance PASS\r\n");
    }
    passed
}
