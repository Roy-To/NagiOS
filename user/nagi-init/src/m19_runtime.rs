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
#[cfg(feature = "desktop-login-acceptance")]
use libnagi::storage::MAX_SMALL_FILE_SIZE;
use libnagi::storage::{DirectoryEntry, StorageError, SyscallBlockDevice, Vfs, MAX_NAME_LENGTH};
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
const ACCEPTANCE_RENAMED_FILE: &[u8] = b".nagi-m19-runtime-renamed.txt";
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
    InvalidName,
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

    /// Rename a direct child of the owner Files directory and reconcile its
    /// searchable metadata before reporting success to the caller.
    pub(super) fn rename_file(
        &mut self,
        volume: &mut UserDataVolume,
        old_name: &[u8],
        new_name: &[u8],
    ) -> Result<(), RuntimeError> {
        volume
            .rename_child(FILES_ROOT, old_name, new_name)
            .and_then(|_| volume.flush())
            .map_err(|_| RuntimeError::Storage)?;
        self.sync_files(volume)?;
        Ok(())
    }

    /// Create an empty direct child in the owner Files directory, then publish
    /// its metadata before returning the stable Search ObjectId.
    pub(super) fn create_file(
        &mut self,
        volume: &mut UserDataVolume,
        name: &[u8],
    ) -> Result<ObjectId, RuntimeError> {
        let path = owner_file_path(name)?;
        let mut entries = [DirectoryEntry::empty(); MAX_DIRECTORY_ENTRIES];
        let count = volume
            .list_directory_path(FILES_ROOT, &mut entries)
            .map_err(|_| {
                trace_acceptance_failure(
                    b"Nagi M19 lifecycle trace: Files directory listing failed\r\n",
                );
                RuntimeError::Storage
            })?;
        let regular_files = entries[..count]
            .iter()
            .filter(|entry| entry.file_type == 1)
            .count();
        if regular_files >= MAX_FILES {
            trace_acceptance_failure(b"Nagi M19 lifecycle trace: file capacity reached\r\n");
            return Err(RuntimeError::TooManyFiles);
        }

        volume.create_path(&path).map_err(|error| {
            let message: &[u8] = match error {
                StorageError::AlreadyExists => {
                    b"Nagi M19 lifecycle trace: VFS file already exists\r\n"
                }
                StorageError::Capacity => b"Nagi M19 lifecycle trace: VFS inode/block capacity\r\n",
                StorageError::DirectoryFull => {
                    #[cfg(feature = "desktop-login-acceptance")]
                    {
                        trace_acceptance_failure(
                            b"Nagi M19 lifecycle trace: existing Files entries:\r\n",
                        );
                        for entry in &entries[..count] {
                            libnagi::console_write(entry.name());
                            trace_acceptance_failure(b"\r\n");
                        }
                    }
                    b"Nagi M19 lifecycle trace: VFS directory full\r\n"
                }
                StorageError::NameTooLong => b"Nagi M19 lifecycle trace: VFS name too long\r\n",
                StorageError::InvalidName => b"Nagi M19 lifecycle trace: VFS invalid name\r\n",
                _ => b"Nagi M19 lifecycle trace: VFS create I/O or metadata error\r\n",
            };
            trace_acceptance_failure(message);
            RuntimeError::Storage
        })?;
        volume.flush().map_err(|_| {
            trace_acceptance_failure(
                b"Nagi M19 lifecycle trace: VFS flush after create failed\r\n",
            );
            RuntimeError::Storage
        })?;
        self.sync_files(volume).map_err(|error| {
            trace_acceptance_failure(
                b"Nagi M19 lifecycle trace: Search sync after create failed\r\n",
            );
            error
        })?;
        self.file_object_id(volume, &path)
    }

    /// Permanently remove a direct child and tombstone its Search record before
    /// reporting success. A restore creates a new file identity if the VFS
    /// inode was reused in the meantime.
    pub(super) fn delete_file(
        &mut self,
        volume: &mut UserDataVolume,
        name: &[u8],
    ) -> Result<ObjectId, RuntimeError> {
        let path = owner_file_path(name)?;
        let object_id = self.file_object_id(volume, &path)?;
        volume
            .remove_path(&path)
            .map_err(|_| RuntimeError::Storage)?;
        volume.flush().map_err(|_| RuntimeError::Storage)?;
        self.sync_files(volume)?;
        Ok(object_id)
    }

    fn file_object_id(
        &self,
        volume: &mut UserDataVolume,
        path: &[u8],
    ) -> Result<ObjectId, RuntimeError> {
        let metadata = volume
            .metadata_path(path)
            .map_err(|_| RuntimeError::Storage)?;
        let key = alloc::format!("{}:{}", metadata.inode, metadata.generation);
        self.service
            .producer_object_id(FILES_PRODUCER_ID, &key, FILES_APP_ID, FILES_SESSION_ID)
            .ok_or(RuntimeError::SearchUnavailable)
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

    /// Return bounded owner-visible titles for the Files panel. Search still
    /// applies the same private Files workspace and visibility policy as the
    /// ObjectId-facing API; the UI never reads file contents.
    pub(super) fn search_file_titles(&self, query: &str) -> Option<Vec<String>> {
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
                .map(|hit| hit.record.title)
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
    pub(super) fn acceptance_verify_rename(&mut self, volume: &mut UserDataVolume) -> bool {
        let Some(expected) = self.acceptance_object_id else {
            return false;
        };
        if self
            .rename_file(
                volume,
                b".nagi-m19-runtime-search.txt",
                ACCEPTANCE_RENAMED_FILE,
            )
            .is_err()
        {
            return false;
        }
        let renamed_matches = self
            .search_files("nagi-m19-runtime-renamed.txt")
            .is_some_and(|ids| ids.len() == 1 && ids[0] == expected);
        let old_name_hidden = self
            .search_files(".nagi-m19-runtime-search.txt")
            .is_some_and(|ids| ids.is_empty());

        let restored = self
            .rename_file(
                volume,
                ACCEPTANCE_RENAMED_FILE,
                b".nagi-m19-runtime-search.txt",
            )
            .is_ok()
            && self
                .acceptance_query(".nagi-m19-runtime-search.txt")
                .is_some_and(|object_id| object_id == expected);
        renamed_matches && old_name_hidden && restored
    }

    #[cfg(feature = "desktop-login-acceptance")]
    pub(super) fn acceptance_verify_file_lifecycle(&mut self, volume: &mut UserDataVolume) -> bool {
        use libnagi::console_write;

        const NAME: &[u8] = b".nagi-m19-lifecycle.txt";
        const QUERY: &str = ".nagi-m19-lifecycle.txt";
        let Ok(path) = owner_file_path(NAME) else {
            return false;
        };
        // Clean up a fixture left by an interrupted earlier acceptance run.
        if volume.metadata_path(&path).is_ok() && self.delete_file(volume, NAME).is_err() {
            console_write(b"Nagi M19 lifecycle trace: stale fixture cleanup failed\r\n");
            return false;
        }

        let result: Result<(), &'static [u8]> = (|| {
            let first_id = self
                .create_file(volume, NAME)
                .map_err(|_| &b"Nagi M19 lifecycle trace: first create failed\r\n"[..])?;
            let first_metadata = volume
                .metadata_path(&path)
                .map_err(|_| &b"Nagi M19 lifecycle trace: first metadata failed\r\n"[..])?;
            let first_visible = self
                .search_files(QUERY)
                .is_some_and(|ids| ids.len() == 1 && ids[0] == first_id);
            if !first_visible {
                return Err(&b"Nagi M19 lifecycle trace: first query mismatch\r\n"[..]);
            }
            if self.delete_file(volume, NAME) != Ok(first_id) {
                return Err(&b"Nagi M19 lifecycle trace: first delete failed\r\n"[..]);
            }
            if !self.search_files(QUERY).is_some_and(|ids| ids.is_empty()) {
                return Err(&b"Nagi M19 lifecycle trace: deleted file still searchable\r\n"[..]);
            }

            let second_id = self
                .create_file(volume, NAME)
                .map_err(|_| &b"Nagi M19 lifecycle trace: second create failed\r\n"[..])?;
            let second_metadata = volume
                .metadata_path(&path)
                .map_err(|_| &b"Nagi M19 lifecycle trace: second metadata failed\r\n"[..])?;
            let reused_inode_with_new_generation = first_metadata.inode == second_metadata.inode
                && first_metadata.generation != second_metadata.generation;
            let new_identity = first_id != second_id
                && self
                    .search_files(QUERY)
                    .is_some_and(|ids| ids.len() == 1 && ids[0] == second_id);
            if !reused_inode_with_new_generation {
                return Err(&b"Nagi M19 lifecycle trace: generation did not advance\r\n"[..]);
            }
            if !new_identity {
                return Err(&b"Nagi M19 lifecycle trace: ObjectId did not change\r\n"[..]);
            }
            if self.delete_file(volume, NAME) != Ok(second_id) {
                return Err(&b"Nagi M19 lifecycle trace: second delete failed\r\n"[..]);
            }
            let hidden_after_delete = self.search_files(QUERY).is_some_and(|ids| ids.is_empty());
            if !hidden_after_delete {
                return Err(&b"Nagi M19 lifecycle trace: second deletion still searchable\r\n"[..]);
            }
            Ok(())
        })();

        // Keep the fixture out of the owner's real Files namespace if a check
        // failed midway; the failure marker still makes acceptance fail.
        if volume.metadata_path(&path).is_ok() {
            let _ = self.delete_file(volume, NAME);
        }
        if let Err(message) = result {
            console_write(message);
            return false;
        }
        true
    }

    #[cfg(feature = "desktop-login-acceptance")]
    pub(super) const fn acceptance_record_restored(&self) -> bool {
        self.acceptance_record_restored
    }
}

fn owner_file_path(name: &[u8]) -> Result<Vec<u8>, RuntimeError> {
    if name.is_empty()
        || name.len() > MAX_NAME_LENGTH
        || name == b"."
        || name == b".."
        || name.iter().any(|byte| *byte == 0 || *byte == b'/')
        || core::str::from_utf8(name).is_err()
    {
        return Err(RuntimeError::InvalidName);
    }
    let mut path = Vec::with_capacity(FILES_ROOT.len() + 1 + name.len());
    path.extend_from_slice(FILES_ROOT);
    path.push(b'/');
    path.extend_from_slice(name);
    Ok(path)
}

fn trace_acceptance_failure(message: &[u8]) {
    #[cfg(feature = "desktop-login-acceptance")]
    libnagi::console_write(message);
    #[cfg(not(feature = "desktop-login-acceptance"))]
    let _ = message;
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
