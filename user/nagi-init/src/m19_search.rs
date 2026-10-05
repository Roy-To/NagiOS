use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};
use core::sync::atomic::{AtomicU64, Ordering};
use libnagi::storage::{
    BlockDevice, DirectoryEntry, FileHandle, StorageError, SyscallBlockDevice, Vfs, BLOCK_SIZE,
    MAX_SMALL_FILE_SIZE,
};
use libnagi::{
    ChannelHandleTransfer, ChannelReceiveResult, ChannelSendRequest, ProcessInfo,
    MAX_CHANNEL_QUEUE_MESSAGES, MAX_PROCESS_NAME, RIGHT_READ,
};
use nagi_ai::{
    execute_plan, register_file_search_action, validate_plan, ActionPolicy, ActionRegistry,
    CallerIdentity, ContextAuthority, ContextRequest, ContextResolver, ExecutionStatus, NagiPlan,
    ObjectAccess, PolicyDenied,
};
use nagi_history::ActivityContext;
use nagi_model::{AppId, AppSessionId, NodeId, ObjectId, WorkspaceId};
use nagi_model_manager::CapabilityId;
use nagi_search::{
    adapters::{FilesProducerAdapter, ProducerObject},
    AccessContext, Embedding, EmbeddingProvider, EmbeddingPurpose, EmbeddingSpaceId,
    GuestSnapshotBackend, IndexedChunk, MetadataRecord, ObjectKind, PersistentVectorIndex,
    SearchQuery, SearchService, SemanticError, SnapshotFile, SnapshotFileStore, SnapshotSlot,
    VectorIndex, VisibilityFilter, VisibilityScope, Workspace, WorkspaceSession, GUEST_FILE_BYTES,
};

#[cfg(feature = "m19-search-ipc")]
#[path = "m19_search_ipc.rs"]
mod search_ipc;

const STORE_ROOT: &[u8] = b"/var/lib/nagi-search";
const SEMANTIC_STORE_ROOT: &[u8] = b"/var/lib/nagi-search-semantic";
const OBJECT_ID: ObjectId = ObjectId(0x4e41_4749_4d19_0001);
const WORKSPACE_ID: WorkspaceId = WorkspaceId(0x4e41_4749_4d19_0002);
/// Declared by `manifests/org.nagi.acceptance.m19-search.manifest`.
const APP_ID: AppId = AppId::from_identifier(b"org.nagi.acceptance.m19-search");
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
const FILE_SEARCH_INTENT: &str = "find the live VFS fixture";
const FILE_SEARCH_QUERY: &str = "nagi-m19-live-file.txt";
const FILE_SEARCH_PLAN_SUMMARY: &str = "query=nagi-m19-live-file.txt";
const CHANNEL_WAIT_PENDING: u64 = u64::MAX;
const CHANNEL_WAIT_RESULT: u64 = 0x4e41_4749_0019_0001;
static CHANNEL_WAIT_WORKER_RESULT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct M19SearchActivity {
    pub occurred_at: u64,
    pub context: ActivityContext,
    pub user_intent: &'static str,
    pub plan_summary: &'static str,
    pub object_id: ObjectId,
}

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

/// Where an action policy takes capability grants from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GrantSource {
    /// The in-process acceptance caller and its fixed grant table, used only
    /// by images built without `m21-action-ipc`.
    #[cfg_attr(feature = "m21-action-ipc", allow(dead_code))]
    InProcessAcceptance,
    /// The live Supervisor launch registry: only a launched session's
    /// manifest grants count (ADR 0046).
    #[cfg(feature = "m21-action-ipc")]
    Supervisor,
}

impl GrantSource {
    /// Whether `caller` may exercise `capability`. `fixture_grants` is
    /// consulted only for the in-process acceptance caller.
    pub(crate) fn permits(
        self,
        caller: CallerIdentity,
        capability: &str,
        fixture_grants: impl FnOnce(CallerIdentity, &str) -> bool,
    ) -> bool {
        match self {
            Self::InProcessAcceptance => fixture_grants(caller, capability),
            #[cfg(feature = "m21-action-ipc")]
            Self::Supervisor => crate::action_ipc::caller_has_grant(caller, capability),
        }
    }
}

struct M19ActionPolicy {
    live_file: ObjectId,
    grants: GrantSource,
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
        if self
            .grants
            .permits(caller, capability.as_str(), |caller, capability| {
                Self::caller_is_fixture(caller) && capability == "files.search"
            })
        {
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

/// The application session the M19 acceptance grants `files.search`.
const FILE_SEARCH_APP_CALLER: CallerIdentity = CallerIdentity {
    app_id: APP_ID,
    app_session_id: SESSION_ID,
    node_id: NODE_ID,
    workspace_id: Some(WORKSPACE_ID),
};

/// ADR 0045: `file.search` requested by an isolated client. The caller is
/// resolved from the kernel-stamped sender PID and launch record. A foreign
/// application is denied before any action is registered.
#[cfg(feature = "m21-action-ipc")]
fn run_file_search_action_ipc(
    service: M19SearchService,
    live_file: ObjectId,
) -> Option<M19SearchActivity> {
    use nagi_action_ipc::{ActionResult, ActionStatus};

    use crate::action_ipc::{placement_of, serve_isolated_request};

    let foreign_placement = libnagi::launch::LaunchPlacement {
        app_session_id: AppSessionId(0x4e41_4749_4d21_00ff),
        ..placement_of(FILE_SEARCH_APP_CALLER)
    };
    let policy = M19ActionPolicy {
        live_file,
        grants: GrantSource::Supervisor,
    };
    let reported = serve_isolated_request(
        crate::supervisor::FOREIGN_APP,
        foreign_placement,
        FILE_SEARCH_INTENT,
        |caller, intent| {
            let Ok(capability) = CapabilityId::new("files.search") else {
                return ActionResult::status_only(ActionStatus::Failed);
            };
            if intent != FILE_SEARCH_INTENT {
                ActionResult::status_only(ActionStatus::InvalidRequest)
            } else if policy.check_capability(caller, &capability).is_err() {
                ActionResult::status_only(ActionStatus::Denied)
            } else {
                // The foreign launch record must never reach execution.
                ActionResult::status_only(ActionStatus::Failed)
            }
        },
    )?;
    if reported.status != ActionStatus::Denied {
        libnagi::console_write(b"Nagi M21 foreign isolated caller FAIL\r\n");
        return None;
    }
    libnagi::console_write(b"Nagi M21 foreign isolated caller denied PASS\r\n");

    let mut activity = None;
    let reported = serve_isolated_request(
        APP_ID,
        placement_of(FILE_SEARCH_APP_CALLER),
        FILE_SEARCH_INTENT,
        |caller, intent| {
            if intent != FILE_SEARCH_INTENT {
                return ActionResult::status_only(ActionStatus::InvalidRequest);
            }
            activity = run_file_search_action(service, live_file, caller, GrantSource::Supervisor);
            match activity {
                Some(activity) => ActionResult::succeeded(&[activity.object_id.0])
                    .unwrap_or(ActionResult::status_only(ActionStatus::Failed)),
                None => ActionResult::status_only(ActionStatus::Failed),
            }
        },
    )?;
    if reported.status != ActionStatus::Succeeded || reported.object_ids() != [live_file.0] {
        libnagi::console_write(b"Nagi M21 isolated caller file.search FAIL\r\n");
        return None;
    }
    libnagi::console_write(b"Nagi M21 file.search isolated caller PASS\r\n");
    activity
}

fn run_file_search_action(
    service: M19SearchService,
    live_file: ObjectId,
    caller: CallerIdentity,
    grants: GrantSource,
) -> Option<M19SearchActivity> {
    libnagi::console_write(b"Nagi M21 trace file.search start\r\n");
    let policy = M19ActionPolicy { live_file, grants };
    let Ok(context) = ContextResolver.resolve(
        ContextRequest {
            caller,
            selected_object: None,
            candidate_objects: alloc::vec![OBJECT_ID, live_file],
        },
        &policy,
    ) else {
        return None;
    };
    libnagi::console_write(b"Nagi M21 trace context resolved\r\n");
    if !context.contains_object(OBJECT_ID) || !context.contains_object(live_file) {
        return None;
    }

    // The grant table is private to the M19 guest acceptance. With
    // `m21-action-ipc`, `caller` comes from an isolated client's launch
    // record (ADR 0045); otherwise it is the in-process acceptance caller.
    let foreign_caller = CallerIdentity {
        app_id: AppId(APP_ID.0.wrapping_add(1)),
        ..caller
    };
    let Ok(capability) = CapabilityId::new("files.search") else {
        return None;
    };
    if policy.check_capability(foreign_caller, &capability).is_ok() {
        return None;
    }
    libnagi::console_write(b"Nagi M21 trace foreign caller denied\r\n");

    let plan_json = alloc::format!(
        r#"{{"plan_version":1,"intent":"{}","steps":[{{"action":"file.search","parameters":{{"query":"{}"}}}}]}}"#,
        FILE_SEARCH_INTENT,
        FILE_SEARCH_QUERY
    );
    let Ok(plan) = NagiPlan::parse_complete(&plan_json) else {
        return None;
    };
    libnagi::console_write(b"Nagi M21 trace plan parsed\r\n");
    let mut registry: ActionRegistry<M19ActionPolicy> = ActionRegistry::new();
    if register_file_search_action(&mut registry, service).is_err() {
        return None;
    }
    libnagi::console_write(b"Nagi M21 trace file.search registered\r\n");
    let Ok(validated) = validate_plan(plan, &context, &registry, &policy) else {
        return None;
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
        Some(M19SearchActivity {
            occurred_at: libnagi::time_ticks(),
            context: ActivityContext {
                app_id: caller.app_id,
                app_session_id: caller.app_session_id,
                node_id: caller.node_id,
                surface_id: None,
                workspace_id: caller.workspace_id,
            },
            user_intent: FILE_SEARCH_INTENT,
            plan_summary: FILE_SEARCH_PLAN_SUMMARY,
            object_id: live_file,
        })
    } else {
        libnagi::console_write(b"Nagi M21 file.search Plan Validate Execute FAIL\r\n");
        None
    }
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
    let mut content = [0; MAX_SMALL_FILE_SIZE];
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

/// Exercises the exposed user syscall path for the bootstrap process only.
/// This proves Channel plumbing and attenuated handle transfer; it does not
/// represent authenticated service IPC because the guest has one Process.
fn bootstrap_channel_abi_acceptance() -> bool {
    let Some(transport) = libnagi::channel_create_pair() else {
        return false;
    };
    let Some(target) = libnagi::channel_create_pair() else {
        let _ = libnagi::handle_close(transport.endpoint_a);
        let _ = libnagi::handle_close(transport.endpoint_b);
        return false;
    };

    let mut process = ProcessInfo {
        pid: 0,
        parent_pid: 0,
        state: 0,
        flags: 0,
        image_pages: 0,
        stack_pages: 0,
        name: [0; MAX_PROCESS_NAME],
    };
    let process_info_ok = libnagi::process_info(&mut process);
    let mut moved_endpoint = None;
    let mut transfer_enqueued = false;
    let checks_passed = (|| {
        if !process_info_ok || process.pid != 1 {
            return false;
        }

        let forged_identity = [0xef, 0xbe, 0xad, 0xde, 0x34, 0x12, 0x00, 0x00];
        let mut request = ChannelSendRequest::new(0x4e47, 1, 0x19_0001, 7);
        request.payload_len = forged_identity.len() as u32;
        request.payload[..forged_identity.len()].copy_from_slice(&forged_identity);
        request.transfer_count = 1;
        request.transfers[0] = ChannelHandleTransfer {
            handle: target.endpoint_a,
            rights: RIGHT_READ,
            reserved: 0,
        };
        if !libnagi::channel_send(transport.endpoint_a, &request) {
            return false;
        }
        transfer_enqueued = true;
        // A successful transfer consumes the sender's original handle.
        if libnagi::channel_send(target.endpoint_a, &ChannelSendRequest::default()) {
            return false;
        }

        let mut received = ChannelReceiveResult::default();
        if libnagi::channel_try_receive(transport.endpoint_b, &mut received) != Some(true)
            || received.sender_process_id != process.pid as u32
            || received.protocol_id != request.protocol_id
            || received.request_id != request.request_id
            || received.payload_len != forged_identity.len() as u32
            || received.payload[..forged_identity.len()] != forged_identity
            || received.transfer_count != 1
            || received.handles[0] == 0
        {
            return false;
        }
        moved_endpoint = Some(received.handles[0]);

        // The moved READ-only endpoint can receive but cannot send.
        if libnagi::channel_send(received.handles[0], &ChannelSendRequest::default()) {
            return false;
        }
        let peer_payload = b"read still works";
        let mut peer_message = ChannelSendRequest::new(0x4e47, 1, 0x19_0002, 8);
        peer_message.payload_len = peer_payload.len() as u32;
        peer_message.payload[..peer_payload.len()].copy_from_slice(peer_payload);
        if !libnagi::channel_send(target.endpoint_b, &peer_message) {
            return false;
        }
        let mut target_received = ChannelReceiveResult::default();
        if libnagi::channel_try_receive(received.handles[0], &mut target_received) != Some(true)
            || target_received.payload_len != peer_payload.len() as u32
            || &target_received.payload[..peer_payload.len()] != peer_payload
        {
            return false;
        }
        if libnagi::channel_try_receive(transport.endpoint_a, &mut received) != Some(false) {
            return false;
        }

        // The queue is bounded and an overflow leaves earlier messages intact.
        for index in 0..MAX_CHANNEL_QUEUE_MESSAGES {
            let mut queued = ChannelSendRequest::new(0x4e47, 1, index as u64, 9);
            queued.payload_len = 1;
            queued.payload[0] = index as u8;
            if !libnagi::channel_send(transport.endpoint_a, &queued) {
                return false;
            }
        }
        if libnagi::channel_send(transport.endpoint_a, &ChannelSendRequest::default()) {
            return false;
        }
        for index in 0..MAX_CHANNEL_QUEUE_MESSAGES {
            let mut queued = ChannelReceiveResult::default();
            if libnagi::channel_try_receive(transport.endpoint_b, &mut queued) != Some(true)
                || queued.payload_len != 1
                || queued.payload[0] != index as u8
            {
                return false;
            }
        }
        libnagi::channel_try_receive(transport.endpoint_b, &mut received) == Some(false)
    })();

    let mut cleanup_ok = true;
    for handle in [
        transport.endpoint_a,
        transport.endpoint_b,
        target.endpoint_b,
    ] {
        cleanup_ok &= libnagi::handle_close(handle);
    }
    if !transfer_enqueued {
        cleanup_ok &= libnagi::handle_close(target.endpoint_a);
    }
    if let Some(handle) = moved_endpoint {
        cleanup_ok &= libnagi::handle_close(handle);
    }
    let Some(reused) = libnagi::channel_create_pair() else {
        return false;
    };
    let reused_ok = reused.endpoint_a != transport.endpoint_a
        && reused.endpoint_b != transport.endpoint_b
        && libnagi::handle_close(reused.endpoint_a)
        && libnagi::handle_close(reused.endpoint_b);

    let wait_wake_ok = blocking_channel_wait_acceptance();
    if wait_wake_ok {
        libnagi::console_write(b"Nagi bootstrap Channel wait/wake PASS\r\n");
    }
    checks_passed && cleanup_ok && reused_ok && wait_wake_ok
}

extern "C" fn channel_wait_worker(endpoint: usize) {
    let mut received = ChannelReceiveResult::default();
    let completed = libnagi::channel_receive(endpoint as u64, &mut received).is_some()
        && received.request_id == 0x19_0003
        && received.payload_len == 16
        && &received.payload[..16] == b"channel wait ok!";
    CHANNEL_WAIT_WORKER_RESULT.store(
        if completed { CHANNEL_WAIT_RESULT } else { 0 },
        Ordering::Release,
    );
    libnagi::thread_exit(if completed { 0 } else { 1 });
}

fn blocking_channel_wait_acceptance() -> bool {
    let Some(endpoints) = libnagi::channel_create_pair() else {
        return false;
    };
    let stack_size = libnagi::BOOTSTRAP_USER_THREAD_STACK_DEFAULT_SIZE;
    let Some(stack) = libnagi::mmap_anonymous(stack_size, libnagi::PROT_READ | libnagi::PROT_WRITE)
    else {
        let _ = libnagi::handle_close(endpoints.endpoint_a);
        let _ = libnagi::handle_close(endpoints.endpoint_b);
        return false;
    };
    CHANNEL_WAIT_WORKER_RESULT.store(CHANNEL_WAIT_PENDING, Ordering::Release);
    let Some(thread) = libnagi::thread_create(
        channel_wait_worker as usize,
        endpoints.endpoint_b as usize,
        stack,
        stack_size,
    ) else {
        let _ = libnagi::munmap(stack, stack_size);
        let _ = libnagi::handle_close(endpoints.endpoint_a);
        let _ = libnagi::handle_close(endpoints.endpoint_b);
        return false;
    };

    // Cooperative yield schedules the worker. It must return to this main
    // thread still blocked, before the sender publishes the message.
    let yielded = libnagi::thread_yield();
    let blocked_before_send =
        CHANNEL_WAIT_WORKER_RESULT.load(Ordering::Acquire) == CHANNEL_WAIT_PENDING;
    let mut request = ChannelSendRequest::new(0x4e47, 1, 0x19_0003, 10);
    let payload = b"channel wait ok!";
    request.payload_len = payload.len() as u32;
    request.payload[..payload.len()].copy_from_slice(payload);
    let sent = libnagi::channel_send(endpoints.endpoint_a, &request);
    let joined = libnagi::thread_join(thread);
    let Some(exit_code) = joined else {
        // Keep the live stack and endpoints mapped if the worker did not
        // terminate; unmapping them here would invalidate a live thread.
        return false;
    };
    let worker_result = CHANNEL_WAIT_WORKER_RESULT.load(Ordering::Acquire);
    let stack_released = libnagi::munmap(stack, stack_size);
    let endpoints_closed =
        libnagi::handle_close(endpoints.endpoint_a) && libnagi::handle_close(endpoints.endpoint_b);
    yielded
        && blocked_before_send
        && sent
        && exit_code == 0
        && worker_result == CHANNEL_WAIT_RESULT
        && stack_released
        && endpoints_closed
}

/// Exercises the real guest VFS persistence adapter with a test-only private
/// fixture and indexes a real file entry from that VFS. The search API is not
/// registered as a production IPC service here; caller authority remains a
/// separate M19 integration requirement.
pub fn run(block_capability: u64) -> Option<M19SearchActivity> {
    libnagi::console_write(b"Nagi M19 trace start\r\n");
    if !bootstrap_channel_abi_acceptance() {
        return None;
    }
    libnagi::console_write(b"Nagi bootstrap Channel ABI PASS\r\n");
    // ADR 0051: manifest grants take effect only with the user's consent.
    #[cfg(feature = "m19-search-ipc")]
    if !crate::supervisor::record_acceptance_consents() {
        libnagi::console_write(b"Nagi Supervisor acceptance consent FAIL\r\n");
        return None;
    }
    let was_persisted = {
        let Ok(mut service) = open_search(block_capability) else {
            return None;
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
            return None;
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
            return None;
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
            return None;
        }
        was_persisted
    };
    libnagi::console_write(b"Nagi M19 trace snapshot fixture persisted\r\n");

    let (file_metadata, initial_projection) = {
        let Ok(mut volume) = open_volume(block_capability) else {
            return None;
        };
        let (handle, name) = fixture_file(&mut volume)?;
        if volume.flush().is_err() {
            return None;
        }
        let Ok(metadata) = volume.metadata(handle) else {
            return None;
        };
        let projection = project_live_file(&mut volume, name)?;
        (metadata, projection)
    };
    libnagi::console_write(b"Nagi M19 trace real VFS file projected\r\n");

    let object_id_before_rename = {
        let Ok(mut service) = open_search(block_capability) else {
            return None;
        };
        index_live_file(&mut service, initial_projection)?
    };
    libnagi::console_write(b"Nagi M19 trace initial ObjectId indexed\r\n");

    let renamed_projection = {
        let Ok(mut volume) = open_volume(block_capability) else {
            return None;
        };
        let (handle, name) = fixture_file(&mut volume)?;
        let Ok(metadata) = volume.metadata(handle) else {
            return None;
        };
        if metadata.inode != file_metadata.inode
            || (name == LIVE_FILE_SOURCE
                && volume.rename(LIVE_FILE_SOURCE, LIVE_FILE_RENAMED).is_err())
            || volume.flush().is_err()
        {
            return None;
        }
        project_live_file(&mut volume, LIVE_FILE_RENAMED)?
    };
    libnagi::console_write(b"Nagi M19 trace VFS rename persisted\r\n");

    let object_id_after_rename = {
        let Ok(mut service) = open_search(block_capability) else {
            return None;
        };
        index_live_file(&mut service, renamed_projection)?
    };
    if object_id_before_rename != object_id_after_rename {
        return None;
    }
    libnagi::console_write(b"Nagi M19 trace ObjectId stable after rename\r\n");

    let remounted_projection = {
        let Ok(mut volume) = open_volume(block_capability) else {
            return None;
        };
        let (handle, name) = fixture_file(&mut volume)?;
        let Ok(metadata) = volume.metadata(handle) else {
            return None;
        };
        if name != LIVE_FILE_RENAMED || metadata.inode != file_metadata.inode {
            return None;
        }
        project_live_file(&mut volume, LIVE_FILE_RENAMED)?
    };
    libnagi::console_write(b"Nagi M19 trace VFS remount verified\r\n");
    let Ok(mut service) = open_search(block_capability) else {
        return None;
    };
    let object_id_after_restart = index_live_file(&mut service, remounted_projection)?;
    libnagi::console_write(b"Nagi M19 trace ObjectId stable after remount\r\n");
    let query = SearchQuery {
        text: Some(String::from("persisted object")),
        workspace: Some(WORKSPACE_ID),
        ..SearchQuery::default()
    };
    let Ok(response) = service.search(ACCESS, &query) else {
        return None;
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
    #[cfg(feature = "m19-search-ipc")]
    let ipc_passed = search_ipc::run(&service, object_id_after_rename);
    #[cfg(not(feature = "m19-search-ipc"))]
    let ipc_passed = true;
    let semantic_passed =
        run_m24_semantic_fixture(&service, block_capability, object_id_after_rename);
    #[cfg(feature = "m21-action-ipc")]
    let search_activity = run_file_search_action_ipc(service, object_id_after_rename);
    #[cfg(not(feature = "m21-action-ipc"))]
    let search_activity = run_file_search_action(
        service,
        object_id_after_rename,
        FILE_SEARCH_APP_CALLER,
        GrantSource::InProcessAcceptance,
    );
    let passed = search_activity.is_some()
        && ipc_passed
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
    if passed {
        search_activity
    } else {
        None
    }
}
