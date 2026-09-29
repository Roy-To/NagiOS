use alloc::string::String;
use libnagi::storage::{BlockDevice, StorageError, SyscallBlockDevice, Vfs, BLOCK_SIZE};
use nagi_model::{AppId, AppSessionId, ObjectId, WorkspaceId};
use nagi_search::{
    AccessContext, GuestSnapshotBackend, MetadataRecord, ObjectKind, SearchQuery, SnapshotFile,
    SnapshotFileStore, SnapshotSlot, VisibilityFilter, VisibilityScope, Workspace,
    WorkspaceSession, GUEST_FILE_BYTES,
};

const STORE_ROOT: &[u8] = b"/var/lib/nagi-search";
const OBJECT_ID: ObjectId = ObjectId(0x4e41_4749_4d19_0001);
const WORKSPACE_ID: WorkspaceId = WorkspaceId(0x4e41_4749_4d19_0002);
const APP_ID: AppId = AppId(0x4e41_4749_4d19_0003);
const SESSION_ID: AppSessionId = AppSessionId(0x4e41_4749_4d19_0004);
const ACCESS: AccessContext = AccessContext::for_application(APP_ID, SESSION_ID);

const _: [(); BLOCK_SIZE] = [(); GUEST_FILE_BYTES];

struct VfsSnapshotFiles<D: BlockDevice> {
    volume: Vfs<D>,
}

impl<D: BlockDevice> VfsSnapshotFiles<D> {
    fn new(mut volume: Vfs<D>) -> Result<Self, nagi_search::BackendError> {
        volume
            .ensure_directory_path(b"/var")
            .and_then(|()| volume.ensure_directory_path(b"/var/lib"))
            .and_then(|()| volume.ensure_directory_path(STORE_ROOT))
            .map_err(|_| nagi_search::BackendError::Io)?;
        Ok(Self { volume })
    }

    fn path(slot: SnapshotSlot, file: SnapshotFile) -> Option<&'static [u8]> {
        match (slot, file) {
            (SnapshotSlot::A, SnapshotFile::Manifest) => Some(b"/var/lib/nagi-search/am"),
            (SnapshotSlot::A, SnapshotFile::Chunk(0)) => Some(b"/var/lib/nagi-search/a0"),
            (SnapshotSlot::A, SnapshotFile::Chunk(1)) => Some(b"/var/lib/nagi-search/a1"),
            (SnapshotSlot::A, SnapshotFile::Chunk(2)) => Some(b"/var/lib/nagi-search/a2"),
            (SnapshotSlot::A, SnapshotFile::Chunk(3)) => Some(b"/var/lib/nagi-search/a3"),
            (SnapshotSlot::B, SnapshotFile::Manifest) => Some(b"/var/lib/nagi-search/bm"),
            (SnapshotSlot::B, SnapshotFile::Chunk(0)) => Some(b"/var/lib/nagi-search/b0"),
            (SnapshotSlot::B, SnapshotFile::Chunk(1)) => Some(b"/var/lib/nagi-search/b1"),
            (SnapshotSlot::B, SnapshotFile::Chunk(2)) => Some(b"/var/lib/nagi-search/b2"),
            (SnapshotSlot::B, SnapshotFile::Chunk(3)) => Some(b"/var/lib/nagi-search/b3"),
            (_, SnapshotFile::Chunk(_)) => None,
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
        let path = Self::path(slot, file).ok_or(nagi_search::BackendError::Io)?;
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
        let path = Self::path(slot, file).ok_or(nagi_search::BackendError::Io)?;
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
        let path = Self::path(slot, file).ok_or(nagi_search::BackendError::Io)?;
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
            && record.object_id == OBJECT_ID
            && record.visibility == VisibilityScope::Private
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

fn open_search(
    block_capability: u64,
) -> Result<
    nagi_search::SearchService<
        GuestSnapshotBackend<VfsSnapshotFiles<SyscallBlockDevice>>,
        M19AcceptanceVisibility,
    >,
    nagi_search::MetadataStoreError,
> {
    let (volume, _) = Vfs::mount_or_format(SyscallBlockDevice::new(block_capability))
        .map_err(|_| nagi_search::MetadataStoreError::Backend(nagi_search::BackendError::Io))?;
    let files = VfsSnapshotFiles::new(volume).map_err(nagi_search::MetadataStoreError::Backend)?;
    nagi_search::SearchService::open(GuestSnapshotBackend::new(files), M19AcceptanceVisibility)
}

/// Exercises the real guest VFS persistence adapter with a test-only private
/// fixture. The search API is not registered as a production IPC service here;
/// caller authority remains a separate M19 integration requirement.
pub fn run(block_capability: u64) -> bool {
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
    drop(service);

    let Ok(service) = open_search(block_capability) else {
        return false;
    };
    let query = SearchQuery {
        text: Some(String::from("persisted object")),
        workspace: Some(WORKSPACE_ID),
        ..SearchQuery::default()
    };
    let Ok(response) = service.search(ACCESS, &query) else {
        return false;
    };
    let workspace = service.get_workspace(ACCESS, WORKSPACE_ID);
    let passed = response.objects.len() == 1
        && response.objects[0].record.object_id == OBJECT_ID
        && response.workspace_groups.len() == 1
        && response.workspace_groups[0].object_ids == [OBJECT_ID]
        && workspace.is_some_and(|workspace| workspace.objects == [OBJECT_ID]);
    if passed {
        if was_persisted {
            libnagi::console_write(b"Nagi M19 previous-boot snapshot PASS\r\n");
        } else {
            libnagi::console_write(b"Nagi M19 initial snapshot/reopen PASS\r\n");
        }
        libnagi::console_write(b"Nagi M19 guest search persistence PASS\r\n");
        libnagi::console_write(b"Nagi M19 acceptance PASS\r\n");
    }
    passed
}
