//! Owner-scoped M19 Search runtime for the ordinary desktop session.
//!
//! This path indexes only `/home/owner/files` on the User Data volume. It
//! keeps account and system metadata outside the search producer boundary.

use alloc::{
    collections::BTreeSet,
    string::{String, ToString},
    vec::Vec,
};

use crate::m19_storage::{self, VfsSnapshotFiles};
use libnagi::storage::{DirectoryEntry, SyscallBlockDevice, Vfs};
#[cfg(feature = "desktop-login-acceptance")]
use libnagi::storage::{StorageError, MAX_SMALL_FILE_SIZE};
use nagi_model::{AppId, AppSessionId, ObjectId, WorkspaceId};
use nagi_search::{
    adapters::{
        FilesProducerAdapter, ProducerObject, WorkspaceProducerAdapter, PRODUCER_ID_ATTRIBUTE,
        PRODUCER_KEY_ATTRIBUTE,
    },
    AccessContext, GuestSnapshotBackend, MetadataRecord, ObjectKind, SearchService,
    VisibilityFilter, VisibilityScope, Workspace, WorkspaceSession,
};

type UserDataVolume = Vfs<SyscallBlockDevice>;
type SearchBackend = GuestSnapshotBackend<VfsSnapshotFiles<SyscallBlockDevice>>;
type OwnerFilesSearchService = SearchService<SearchBackend, OwnerFilesVisibility>;

const FILES_ROOT: &[u8] = b"/home/owner/files";
const FILES_APP_ID: AppId = AppId::from_identifier(b"org.nagi.files");
const FILES_SESSION_ID: AppSessionId = AppSessionId(0x4e41_4749_4649_4c45);
const FILES_WORKSPACE_ID: WorkspaceId = WorkspaceId(0x4e41_4749_4649_0001);
const FILES_PRODUCER_ID: &str = "org.nagi.files.user-files";
const FILES_ACCESS: AccessContext = AccessContext::for_application(FILES_APP_ID, FILES_SESSION_ID);
const MAX_FILES: usize = 8;
const MAX_DIRECTORY_ENTRIES: usize = 64;
const INODE_ATTRIBUTE: &str = "nagi.files.vfs_inode";
const GENERATION_ATTRIBUTE: &str = "nagi.files.vfs_generation";
#[cfg(feature = "desktop-login-acceptance")]
const ACCEPTANCE_FILE: &[u8] = b"/home/owner/files/.nagi-m19-runtime-search.txt";
#[cfg(feature = "desktop-login-acceptance")]
const ACCEPTANCE_CONTENT: &[u8] = b"Nagi M19 authenticated desktop Search fixture";
#[cfg(feature = "desktop-login-acceptance")]
const NON_UTF8_NAME_FIXTURE: &[u8] = b"/home/owner/files/\xff";

#[derive(Clone, Copy)]
struct OwnerFilesVisibility;

impl VisibilityFilter for OwnerFilesVisibility {
    fn can_read_object(&self, access: AccessContext, record: &MetadataRecord) -> bool {
        access == FILES_ACCESS
            && record.kind == ObjectKind::File
            && record.visibility == VisibilityScope::Private
            && record.source_app == Some(FILES_APP_ID)
            && record.source_session == Some(FILES_SESSION_ID)
            && record
                .attributes
                .get(PRODUCER_ID_ATTRIBUTE)
                .is_some_and(|producer| producer == FILES_PRODUCER_ID)
    }

    fn can_read_workspace(&self, access: AccessContext, workspace: &Workspace) -> bool {
        access == FILES_ACCESS
            && workspace.workspace_id == FILES_WORKSPACE_ID
            && workspace.owner_app == Some(FILES_APP_ID)
            && workspace.visibility == VisibilityScope::Private
    }

    fn can_read_workspace_session(
        &self,
        access: AccessContext,
        workspace: &Workspace,
        session: WorkspaceSession,
    ) -> bool {
        self.can_read_workspace(access, workspace)
            && session.app_id == FILES_APP_ID
            && session.session_id == FILES_SESSION_ID
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RuntimeError {
    Storage,
    SearchUnavailable,
    TooManyFiles,
    InvalidMetadata,
}

/// Persistent Search service active during the signed-in owner desktop.
pub(super) struct Runtime {
    service: OwnerFilesSearchService,
    #[cfg(feature = "desktop-login-acceptance")]
    acceptance_record_restored: bool,
    #[cfg(feature = "desktop-login-acceptance")]
    acceptance_object_id: Option<ObjectId>,
}

impl Runtime {
    pub(super) fn open(
        block_capability: u64,
        volume: &mut UserDataVolume,
    ) -> Result<Self, RuntimeError> {
        volume
            .ensure_directory_path(b"/home")
            .and_then(|()| volume.ensure_directory_path(b"/home/owner"))
            .and_then(|()| volume.ensure_directory_path(FILES_ROOT))
            .map_err(|_| RuntimeError::Storage)?;
        #[cfg(feature = "desktop-login-acceptance")]
        {
            ensure_acceptance_file(volume)?;
            ensure_non_utf8_name_fixture(volume)?;
        }
        let mut runtime = Self {
            service: m19_storage::open_search_with_visibility(
                block_capability,
                OwnerFilesVisibility,
            )
            .map_err(|_| RuntimeError::SearchUnavailable)?,
            #[cfg(feature = "desktop-login-acceptance")]
            acceptance_record_restored: false,
            #[cfg(feature = "desktop-login-acceptance")]
            acceptance_object_id: None,
        };
        #[cfg(feature = "desktop-login-acceptance")]
        {
            let metadata = volume
                .metadata_path(ACCEPTANCE_FILE)
                .map_err(|_| RuntimeError::Storage)?;
            let key = alloc::format!("{}:{}", metadata.inode, metadata.generation);
            runtime.acceptance_record_restored = runtime
                .service
                .producer_object_id(FILES_PRODUCER_ID, &key, FILES_APP_ID, FILES_SESSION_ID)
                .is_some();
            runtime.sync_files(volume)?;
            runtime.acceptance_object_id = runtime.service.producer_object_id(
                FILES_PRODUCER_ID,
                &key,
                FILES_APP_ID,
                FILES_SESSION_ID,
            );
        }
        #[cfg(not(feature = "desktop-login-acceptance"))]
        runtime.sync_files(volume)?;
        Ok(runtime)
    }

    /// Reconcile producer metadata after a Files operation. Only names and VFS
    /// identity/timestamps are indexed; file contents are never read.
    pub(super) fn sync_files(
        &mut self,
        volume: &mut UserDataVolume,
    ) -> Result<(usize, usize), RuntimeError> {
        let mut entries = [DirectoryEntry::empty(); MAX_DIRECTORY_ENTRIES];
        let count = volume
            .list_directory_path(FILES_ROOT, &mut entries)
            .map_err(|_| RuntimeError::Storage)?;
        let mut regular_files = 0;
        for entry in &entries[..count] {
            if entry.file_type == 1 {
                regular_files += 1;
            }
        }
        if regular_files > MAX_FILES {
            return Err(RuntimeError::TooManyFiles);
        }

        let mut present_keys = BTreeSet::new();
        let mut present_ids = Vec::new();
        let mut restored_records = 0;
        let mut workspace = WorkspaceProducerAdapter
            .create(
                FILES_WORKSPACE_ID,
                "Files",
                Some(FILES_APP_ID),
                VisibilityScope::Private,
            )
            .map_err(|_| RuntimeError::InvalidMetadata)?;
        workspace.sessions.push(WorkspaceSession {
            app_id: FILES_APP_ID,
            session_id: FILES_SESSION_ID,
        });

        for entry in &entries[..count] {
            if entry.file_type != 1 {
                continue;
            }
            // Search metadata is UTF-8. Keep an arbitrary POSIX filename from
            // taking down the whole owner Search runtime; it has no searchable
            // text representation in the current metadata contract.
            let Ok(name) = core::str::from_utf8(entry.name()) else {
                continue;
            };
            let mut path = String::from("/home/owner/files/");
            path.push_str(name);
            let metadata = volume
                .metadata_path(path.as_bytes())
                .map_err(|_| RuntimeError::Storage)?;
            if metadata.inode != entry.inode {
                return Err(RuntimeError::InvalidMetadata);
            }
            let key = alloc::format!("{}:{}", metadata.inode, metadata.generation);
            present_keys.insert(key.clone());
            let restored_id = self.service.producer_object_id(
                FILES_PRODUCER_ID,
                &key,
                FILES_APP_ID,
                FILES_SESSION_ID,
            );
            let object_id = match restored_id {
                Some(object_id) => {
                    restored_records += 1;
                    object_id
                }
                None => self
                    .service
                    .next_object_id()
                    .ok_or(RuntimeError::SearchUnavailable)?,
            };

            let mut attributes = alloc::collections::BTreeMap::new();
            attributes.insert(PRODUCER_ID_ATTRIBUTE.into(), FILES_PRODUCER_ID.into());
            attributes.insert(PRODUCER_KEY_ATTRIBUTE.into(), key);
            attributes.insert(INODE_ATTRIBUTE.into(), metadata.inode.to_string());
            attributes.insert(GENERATION_ATTRIBUTE.into(), metadata.generation.to_string());
            let record = FilesProducerAdapter
                .to_record(ProducerObject {
                    object_id,
                    title: String::from(name),
                    location: Some(path),
                    source_app: Some(FILES_APP_ID),
                    source_session: Some(FILES_SESSION_ID),
                    created_at: None,
                    modified_at: Some(i64::from(metadata.mtime)),
                    observed_at: None,
                    tags: Vec::new(),
                    attributes,
                    visibility: VisibilityScope::Private,
                })
                .map_err(|_| RuntimeError::InvalidMetadata)?;
            if self.service.get_object(FILES_ACCESS, object_id).as_ref() != Some(&record) {
                self.service
                    .upsert_record(record)
                    .map_err(|_| RuntimeError::SearchUnavailable)?;
            }
            present_ids.push(object_id);
        }

        workspace.objects = present_ids.clone();
        if self
            .service
            .get_workspace(FILES_ACCESS, FILES_WORKSPACE_ID)
            .as_ref()
            != Some(&workspace)
        {
            self.service
                .upsert_workspace(workspace)
                .map_err(|_| RuntimeError::SearchUnavailable)?;
        }

        let old_records =
            self.service
                .producer_records(FILES_PRODUCER_ID, FILES_APP_ID, FILES_SESSION_ID);
        for record in old_records {
            let Some(key) = record.attributes.get(PRODUCER_KEY_ATTRIBUTE) else {
                return Err(RuntimeError::InvalidMetadata);
            };
            if !present_keys.contains(key) {
                self.service
                    .remove_workspace_object(FILES_WORKSPACE_ID, record.object_id)
                    .map_err(|_| RuntimeError::SearchUnavailable)?;
                self.service
                    .remove_record(record.object_id, record.modified_at.unwrap_or_default())
                    .map_err(|_| RuntimeError::SearchUnavailable)?;
            }
        }
        Ok((present_ids.len(), restored_records))
    }

    pub(super) fn search_files(&self, query: &str) -> Option<Vec<ObjectId>> {
        let response = self
            .service
            .search(
                FILES_ACCESS,
                &nagi_search::SearchQuery {
                    text: Some(String::from(query)),
                    kind: Some(ObjectKind::File),
                    workspace: Some(FILES_WORKSPACE_ID),
                    ..nagi_search::SearchQuery::default()
                },
            )
            .ok()?;
        Some(
            response
                .objects
                .into_iter()
                .map(|hit| hit.record.object_id)
                .collect(),
        )
    }

    #[cfg(feature = "desktop-login-acceptance")]
    pub(super) fn acceptance_query(&self, query: &str) -> Option<ObjectId> {
        let expected = self.acceptance_object_id?;
        self.search_files(query)?
            .into_iter()
            .find(|object_id| *object_id == expected)
    }

    #[cfg(feature = "desktop-login-acceptance")]
    pub(super) const fn acceptance_record_restored(&self) -> bool {
        self.acceptance_record_restored
    }
}

#[cfg(feature = "desktop-login-acceptance")]
fn ensure_acceptance_file(volume: &mut UserDataVolume) -> Result<(), RuntimeError> {
    match volume.open_path(ACCEPTANCE_FILE) {
        Ok(handle) => {
            let mut contents = [0; MAX_SMALL_FILE_SIZE];
            let length = volume
                .read(handle, &mut contents)
                .map_err(|_| RuntimeError::Storage)?;
            if contents[..length] == *ACCEPTANCE_CONTENT {
                Ok(())
            } else {
                Err(RuntimeError::InvalidMetadata)
            }
        }
        Err(StorageError::NotFound) => {
            let handle = volume
                .create_path(ACCEPTANCE_FILE)
                .map_err(|_| RuntimeError::Storage)?;
            volume
                .write(handle, ACCEPTANCE_CONTENT)
                .and_then(|()| volume.flush())
                .map_err(|_| RuntimeError::Storage)
        }
        Err(_) => Err(RuntimeError::Storage),
    }
}

#[cfg(feature = "desktop-login-acceptance")]
fn ensure_non_utf8_name_fixture(volume: &mut UserDataVolume) -> Result<(), RuntimeError> {
    match volume.open_path(NON_UTF8_NAME_FIXTURE) {
        Ok(_) => Ok(()),
        Err(StorageError::NotFound) => volume
            .create_path(NON_UTF8_NAME_FIXTURE)
            .and_then(|_| volume.flush())
            .map_err(|_| RuntimeError::Storage),
        Err(_) => Err(RuntimeError::Storage),
    }
}
