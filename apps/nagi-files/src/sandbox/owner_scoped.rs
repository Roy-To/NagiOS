//! Linux Files provider whose root is borrowed from the Data owner for one
//! complete FilesService operation. The long-lived provider stores only the
//! owner scope reference; no root FD, directory handle or file handle escapes.

use std::cell::{Cell, RefCell, RefMut};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use nagi_profile_volume_notes_owner::{
    FilesRootEntryKind, FilesRootMetadata, FilesRootScope, FilesRootView,
};
use rustix::fs::{Mode, OFlags};

use crate::{
    CancellationToken, EntryAvailability, EntryKind, FileEntry, FileName, FilesError,
    FilesErrorKind, FilesOperationScopeToken, FilesystemProvider, Location, ProviderAvailability,
    ResourceId, TrashEntry,
};

use super::{
    decode_kind, encode_kind, hex_decode, hex_encode, is_internal_component, map_destination_error,
    map_io_error, next_trash_id, report_trash_recovery, system_time_seconds, valid_trash_name,
    StoredTrash, INDEX_FILE, INTERNAL_DIR, TAG_INDEX_FILE, TEMP_INDEX_FILE, TEMP_TAG_INDEX_FILE,
    TRASH_DIR,
};

const MAX_METADATA_INDEX_BYTES: usize = 1024 * 1024;

/// A provider bound to an owner-issued root scope. This value contains no
/// descriptor; every FilesService operation borrows a fresh owner view.
/// Callers must compose it with the C1 live-Session gate, either through the
/// C1 consumer's issued-session path or an already-authorized root scope.
pub struct OwnerScopedSandboxProvider<'scope, S: FilesRootScope> {
    root_scope: &'scope S,
    recovery_reported: Cell<bool>,
}

impl<'scope, S: FilesRootScope> OwnerScopedSandboxProvider<'scope, S> {
    pub fn new(root_scope: &'scope S) -> Self {
        Self {
            root_scope,
            recovery_reported: Cell::new(false),
        }
    }
}

impl<S: FilesRootScope> FilesystemProvider for OwnerScopedSandboxProvider<'_, S> {
    fn with_operation_scope<R>(
        &mut self,
        _token: &FilesOperationScopeToken,
        operation: impl FnOnce(&mut dyn FilesystemProvider) -> Result<R, FilesError>,
    ) -> Result<R, FilesError>
    where
        Self: Sized,
    {
        let recovery_reported = &self.recovery_reported;
        self.root_scope
            .with_files_root(|root| {
                let mut view = OwnerScopedView::new(root, recovery_reported);
                operation(&mut view)
            })
            .map_err(|_| FilesError::new(FilesErrorKind::ProviderUnavailable))?
    }

    fn availability(&self) -> ProviderAvailability {
        ProviderAvailability::Available
    }

    fn list(&self, location: &Location) -> Result<Vec<FileEntry>, FilesError> {
        Err(scope_required(location))
    }

    fn metadata(&self, location: &Location) -> Result<FileEntry, FilesError> {
        Err(scope_required(location))
    }

    fn read_file(&self, location: &Location, _max_bytes: usize) -> Result<Vec<u8>, FilesError> {
        Err(scope_required(location))
    }

    fn write_file(
        &mut self,
        location: &Location,
        _contents: &[u8],
    ) -> Result<FileEntry, FilesError> {
        Err(scope_required(location))
    }

    fn set_tags(&mut self, location: &Location, _tags: &[String]) -> Result<FileEntry, FilesError> {
        Err(scope_required(location))
    }

    fn create_folder(
        &mut self,
        parent: &Location,
        _name: &FileName,
    ) -> Result<FileEntry, FilesError> {
        Err(scope_required(parent))
    }

    fn rename(&mut self, source: &Location, _name: &FileName) -> Result<FileEntry, FilesError> {
        Err(scope_required(source))
    }

    fn copy(
        &mut self,
        source: &Location,
        _destination: &Location,
        _name: &FileName,
        _cancellation: &CancellationToken,
    ) -> Result<FileEntry, FilesError> {
        Err(scope_required(source))
    }

    fn move_item(
        &mut self,
        source: &Location,
        _destination: &Location,
        _name: &FileName,
    ) -> Result<FileEntry, FilesError> {
        Err(scope_required(source))
    }

    fn trash(&mut self, source: &Location) -> Result<TrashEntry, FilesError> {
        Err(scope_required(source))
    }

    fn list_trash(&self) -> Result<Vec<TrashEntry>, FilesError> {
        Err(FilesError::new(FilesErrorKind::ProviderUnavailable))
    }

    fn restore(&mut self, _id: ResourceId) -> Result<FileEntry, FilesError> {
        Err(FilesError::new(FilesErrorKind::ProviderUnavailable))
    }

    fn permanently_delete(&mut self, _id: ResourceId) -> Result<(), FilesError> {
        Err(FilesError::new(FilesErrorKind::ProviderUnavailable))
    }
}

fn scope_required(location: &Location) -> FilesError {
    FilesError::new(FilesErrorKind::ProviderUnavailable).at(location.clone())
}

struct OwnerScopedView<'root, 'report> {
    root: FilesRootView<'root>,
    recovery_reported: &'report Cell<bool>,
    state: RefCell<Option<OwnerScopedState>>,
}

struct OwnerScopedState {
    internal_exists: bool,
    trash_exists: bool,
    trash: Vec<StoredTrash>,
    next_trash_id: u64,
    tags: BTreeMap<ResourceId, Vec<String>>,
}

impl<'root, 'report> OwnerScopedView<'root, 'report> {
    fn new(root: FilesRootView<'root>, recovery_reported: &'report Cell<bool>) -> Self {
        Self {
            root,
            recovery_reported,
            state: RefCell::new(None),
        }
    }

    /// Load metadata only after FilesService has entered this operation's
    /// capability-authorized provider path. The state is retained only for
    /// this callback-scoped owner view.
    fn ensure_state(&self) -> Result<RefMut<'_, OwnerScopedState>, FilesError> {
        if self.state.borrow().is_none() {
            let should_report_recovery = !self.recovery_reported.get();
            let state = self.load_state(should_report_recovery)?;
            *self.state.borrow_mut() = Some(state);
            self.recovery_reported.set(true);
        }
        Ok(RefMut::map(self.state.borrow_mut(), |state| {
            state.as_mut().expect("owner-scoped state was initialized")
        }))
    }

    fn load_state(&self, report_recovery: bool) -> Result<OwnerScopedState, FilesError> {
        let (internal_exists, trash_exists) = match self.root.metadata(Path::new(INTERNAL_DIR)) {
            Ok(metadata) if metadata.kind == FilesRootEntryKind::Directory => {
                let trash_exists = match self.root.metadata(&internal_path(TRASH_DIR)) {
                    Ok(metadata) if metadata.kind == FilesRootEntryKind::Directory => true,
                    Ok(_) => return Err(FilesError::new(FilesErrorKind::SandboxEscape)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                    Err(error) => return Err(map_io_error(error)),
                };
                (true, trash_exists)
            }
            Ok(_) => return Err(FilesError::new(FilesErrorKind::SandboxEscape)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (false, false),
            Err(error) => return Err(map_io_error(error)),
        };

        let tags = if internal_exists {
            load_tags_index(&self.root)?
        } else {
            BTreeMap::new()
        };
        let trash = if internal_exists {
            load_trash_index(&self.root, trash_exists, report_recovery)?
        } else {
            Vec::new()
        };
        let next_trash_id = next_trash_id(&trash);
        Ok(OwnerScopedState {
            internal_exists,
            trash_exists,
            trash,
            next_trash_id,
            tags,
        })
    }

    fn public_path(&self, location: &Location) -> Result<PathBuf, FilesError> {
        if location
            .components()
            .next()
            .is_some_and(is_internal_component)
        {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape).at(location.clone()));
        }
        Ok(path_for(location))
    }

    fn public_metadata(&self, location: &Location) -> Result<FilesRootMetadata, FilesError> {
        self.root
            .metadata(&self.public_path(location)?)
            .map_err(|error| map_io_error(error).at(location.clone()))
    }

    fn checked_destination(
        &self,
        parent: &Location,
        name: &FileName,
    ) -> Result<(PathBuf, Location), FilesError> {
        let parent_path = self.public_path(parent)?;
        let parent_metadata = self
            .root
            .metadata(&parent_path)
            .map_err(|error| map_io_error(error).at(parent.clone()))?;
        if parent_metadata.kind != FilesRootEntryKind::Directory {
            return Err(FilesError::new(FilesErrorKind::NotDirectory).at(parent.clone()));
        }
        if is_internal_component(name.as_str()) {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape));
        }
        let target = parent.join(name);
        let target_path = path_for(&target);
        match self.root.metadata(&target_path) {
            Ok(_) => Err(FilesError::new(FilesErrorKind::Conflict).at(target)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok((target_path, target)),
            Err(error) => Err(map_io_error(error).at(target)),
        }
    }

    fn entry_from_metadata(
        &self,
        state: &OwnerScopedState,
        location: &Location,
        metadata: FilesRootMetadata,
    ) -> Result<FileEntry, FilesError> {
        let name = location
            .file_name()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        let name = FileName::parse(name)?;
        let kind = entry_kind(metadata.kind);
        let id = resource_id(metadata);
        Ok(FileEntry {
            id,
            name,
            location: location
                .parent()
                .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?,
            kind,
            size_bytes: matches!(kind, EntryKind::File | EntryKind::Symlink)
                .then_some(metadata.size_bytes),
            created_at: metadata.created_at_seconds,
            modified_at: metadata.modified_at_seconds,
            tags: state.tags.get(&id).cloned().unwrap_or_default(),
            availability: if metadata.mode & 0o222 == 0 {
                EntryAvailability::ReadOnly
            } else {
                EntryAvailability::Available
            },
        })
    }

    fn entry_from_root_entry(
        &self,
        state: &OwnerScopedState,
        parent: &Location,
        name: &str,
        metadata: FilesRootMetadata,
    ) -> Result<FileEntry, FilesError> {
        let name = FileName::parse(name)?;
        let id = resource_id(metadata);
        let kind = entry_kind(metadata.kind);
        Ok(FileEntry {
            id,
            name,
            location: parent.clone(),
            kind,
            size_bytes: matches!(kind, EntryKind::File | EntryKind::Symlink)
                .then_some(metadata.size_bytes),
            created_at: metadata.created_at_seconds,
            modified_at: metadata.modified_at_seconds,
            tags: state.tags.get(&id).cloned().unwrap_or_default(),
            availability: if metadata.mode & 0o222 == 0 {
                EntryAvailability::ReadOnly
            } else {
                EntryAvailability::Available
            },
        })
    }

    fn ensure_internal_dir(&self, state: &mut OwnerScopedState) -> Result<(), FilesError> {
        if !state.internal_exists {
            match self.root.create_directory(Path::new(INTERNAL_DIR)) {
                Ok(()) => state.internal_exists = true,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    self.verify_internal_directory()?;
                    state.internal_exists = true;
                }
                Err(error) => return Err(map_io_error(error)),
            }
            state.tags = load_tags_index(&self.root)?;
            state.trash = load_trash_index(&self.root, state.trash_exists, false)?;
            state.next_trash_id = next_trash_id(&state.trash);
        }
        self.verify_internal_directory()
    }

    fn ensure_trash_dir(&self, state: &mut OwnerScopedState) -> Result<(), FilesError> {
        self.ensure_internal_dir(state)?;
        if !state.trash_exists {
            match self.root.create_directory(&internal_path(TRASH_DIR)) {
                Ok(()) => state.trash_exists = true,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    self.verify_trash_directory()?;
                    state.trash_exists = true;
                }
                Err(error) => return Err(map_io_error(error)),
            }
            state.trash = load_trash_index(&self.root, true, false)?;
            state.next_trash_id = next_trash_id(&state.trash);
        }
        self.verify_trash_storage(state)
    }

    fn verify_internal_directory(&self) -> Result<(), FilesError> {
        let metadata = self
            .root
            .metadata(Path::new(INTERNAL_DIR))
            .map_err(map_io_error)?;
        if metadata.kind != FilesRootEntryKind::Directory {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape));
        }
        Ok(())
    }

    fn verify_trash_directory(&self) -> Result<(), FilesError> {
        self.verify_internal_directory()?;
        let metadata = self
            .root
            .metadata(&internal_path(TRASH_DIR))
            .map_err(map_io_error)?;
        if metadata.kind != FilesRootEntryKind::Directory {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape));
        }
        Ok(())
    }

    fn verify_trash_storage(&self, state: &OwnerScopedState) -> Result<(), FilesError> {
        if state.trash_exists {
            self.verify_trash_directory()
        } else {
            self.verify_internal_directory()
        }
    }

    fn persist_index(
        &self,
        state: &OwnerScopedState,
        items: &[StoredTrash],
    ) -> Result<(), FilesError> {
        self.verify_trash_storage(state)?;
        let mut contents = String::new();
        for item in items {
            let original = hex_encode(item.entry.original_location.as_str().as_bytes());
            let trash_name = hex_encode(item.trash_name.as_bytes());
            let kind = encode_kind(item.entry.kind);
            let size = item
                .entry
                .size_bytes
                .map_or_else(|| "-".to_owned(), |size| size.to_string());
            use std::fmt::Write as _;
            writeln!(
                contents,
                "{}\t{}\t{}\t{}\t{}\t{}",
                item.entry.id, item.entry.deleted_at, kind, size, original, trash_name
            )
            .map_err(|_| FilesError::new(FilesErrorKind::ProviderFailure))?;
        }
        self.root
            .write_temp_and_replace(
                &internal_path(TEMP_INDEX_FILE),
                &internal_path(INDEX_FILE),
                contents.as_bytes(),
            )
            .map_err(map_io_error)
    }

    fn persist_tags(&self, state: &OwnerScopedState) -> Result<(), FilesError> {
        self.verify_internal_directory()?;
        let mut contents = String::new();
        for (id, tags) in &state.tags {
            let encoded = tags
                .iter()
                .map(|tag| hex_encode(tag.as_bytes()))
                .collect::<Vec<_>>()
                .join(",");
            use std::fmt::Write as _;
            writeln!(contents, "{id}\t{encoded}")
                .map_err(|_| FilesError::new(FilesErrorKind::ProviderFailure))?;
        }
        self.root
            .write_temp_and_replace(
                &internal_path(TEMP_TAG_INDEX_FILE),
                &internal_path(TAG_INDEX_FILE),
                contents.as_bytes(),
            )
            .map_err(map_io_error)
    }

    fn next_trash_name(&self, state: &mut OwnerScopedState) -> Result<String, FilesError> {
        if !state.trash_exists {
            return Err(FilesError::new(FilesErrorKind::ProviderUnavailable));
        }
        loop {
            let id = state.next_trash_id;
            state.next_trash_id = state.next_trash_id.wrapping_add(1).max(1);
            let name = format!("{id}-{}", system_time_seconds(SystemTime::now()));
            match self
                .root
                .metadata(&internal_path(&format!("{TRASH_DIR}/{name}")))
            {
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(name),
                Err(error) => return Err(map_io_error(error)),
            }
        }
    }

    fn restore_to(&self, id: ResourceId, destination: &Location) -> Result<FileEntry, FilesError> {
        let target = {
            let mut state = self.ensure_state()?;
            self.verify_trash_storage(&state)?;
            let index = state
                .trash
                .iter()
                .position(|item| item.entry.id == id)
                .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
            let item = state.trash[index].clone();
            let trash_path = internal_path(&format!("{TRASH_DIR}/{}", item.trash_name));
            let trash_metadata = self.root.metadata(&trash_path).map_err(map_io_error)?;
            if !matches!(
                trash_metadata.kind,
                FilesRootEntryKind::RegularFile | FilesRootEntryKind::Directory
            ) {
                return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
            }
            if trash_metadata.kind == FilesRootEntryKind::Directory {
                self.root
                    .validate_tree_mounts(&trash_path)
                    .map_err(map_io_error)?;
            }
            let (destination_path, target) = self.checked_restore_destination(destination)?;
            self.root
                .rename_without_replacement(&trash_path, &destination_path)
                .map_err(|error| map_destination_error(error).at(target.clone()))?;
            state.trash.remove(index);
            self.persist_index(&state, &state.trash)
                .map_err(|_| FilesError::new(FilesErrorKind::PartialFailure).at(target.clone()))?;
            target
        };
        self.metadata(&target)
    }

    fn checked_restore_destination(
        &self,
        destination: &Location,
    ) -> Result<(PathBuf, Location), FilesError> {
        let name = destination
            .file_name()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        let parent = destination
            .parent()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        let name = FileName::parse(name)?;
        self.checked_destination(&parent, &name)
    }

    fn restore_destination(&self, item: &TrashEntry) -> Result<Location, FilesError> {
        let original = &item.original_location;
        let parent = original
            .parent()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        match self.root.metadata(&self.public_path(&parent)?) {
            Ok(metadata) if metadata.kind == FilesRootEntryKind::Directory => Ok(original.clone()),
            Ok(_) => Err(FilesError::new(FilesErrorKind::NotDirectory).at(parent)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = original
                    .file_name()
                    .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
                Location::parse(name)
            }
            Err(error) => Err(map_io_error(error).at(parent)),
        }
    }
}

impl FilesystemProvider for OwnerScopedView<'_, '_> {
    fn availability(&self) -> ProviderAvailability {
        ProviderAvailability::Available
    }

    fn list(&self, location: &Location) -> Result<Vec<FileEntry>, FilesError> {
        let state = self.ensure_state()?;
        let path = self.public_path(location)?;
        let mut entries = self
            .root
            .list(&path)
            .map_err(|error| map_io_error(error).at(location.clone()))?
            .into_iter()
            .filter(|entry| !(location.is_root() && is_internal_component(&entry.name)))
            .map(|entry| self.entry_from_root_entry(&state, location, &entry.name, entry.metadata))
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by(|left, right| {
            (left.kind != EntryKind::Folder)
                .cmp(&(right.kind != EntryKind::Folder))
                .then_with(|| {
                    left.name
                        .as_str()
                        .to_lowercase()
                        .cmp(&right.name.as_str().to_lowercase())
                })
        });
        Ok(entries)
    }

    fn metadata(&self, location: &Location) -> Result<FileEntry, FilesError> {
        let state = self.ensure_state()?;
        let metadata = self.public_metadata(location)?;
        self.entry_from_metadata(&state, location, metadata)
    }

    fn read_file(&self, location: &Location, max_bytes: usize) -> Result<Vec<u8>, FilesError> {
        let path = self.public_path(location)?;
        let before = self
            .root
            .metadata(&path)
            .map_err(|error| map_io_error(error).at(location.clone()))?;
        if before.kind == FilesRootEntryKind::Symlink {
            return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(location.clone()));
        }
        if before.kind != FilesRootEntryKind::RegularFile {
            return Err(FilesError::new(FilesErrorKind::IsDirectory).at(location.clone()));
        }
        if before.size_bytes > max_bytes as u64 {
            return Err(FilesError::new(FilesErrorKind::FileTooLarge).at(location.clone()));
        }
        let mut contents = Vec::new();
        self.root
            .with_open_file(&path, OFlags::RDONLY, Mode::empty(), |file| {
                let opened = file.metadata()?;
                use std::os::unix::fs::MetadataExt;
                if opened.dev() != before.device || opened.ino() != before.inode {
                    return Err(std::io::Error::other("file changed during open"));
                }
                let mut limited = (&mut *file).take((max_bytes as u64).saturating_add(1));
                limited.read_to_end(&mut contents)?;
                Ok(())
            })
            .map_err(|error| map_io_error(error).at(location.clone()))?;
        if contents.len() > max_bytes {
            return Err(FilesError::new(FilesErrorKind::FileTooLarge).at(location.clone()));
        }
        Ok(contents)
    }

    fn write_file(
        &mut self,
        location: &Location,
        contents: &[u8],
    ) -> Result<FileEntry, FilesError> {
        let path = self.public_path(location)?;
        let before = self
            .root
            .metadata(&path)
            .map_err(|error| map_io_error(error).at(location.clone()))?;
        if before.kind == FilesRootEntryKind::Symlink {
            return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(location.clone()));
        }
        if before.kind != FilesRootEntryKind::RegularFile {
            return Err(FilesError::new(FilesErrorKind::IsDirectory).at(location.clone()));
        }
        self.root
            .write_existing_file(&path, contents)
            .map_err(|_| FilesError::new(FilesErrorKind::PartialFailure).at(location.clone()))?;
        self.metadata(location)
            .map_err(|_| FilesError::new(FilesErrorKind::PartialFailure).at(location.clone()))
    }

    fn set_tags(&mut self, location: &Location, tags: &[String]) -> Result<FileEntry, FilesError> {
        let entry = self.metadata(location)?;
        {
            let mut state = self.ensure_state()?;
            self.ensure_internal_dir(&mut state)?;
            let previous = state.tags.get(&entry.id).cloned();
            if tags.is_empty() {
                state.tags.remove(&entry.id);
            } else {
                state.tags.insert(entry.id, tags.to_vec());
            }
            if let Err(error) = self.persist_tags(&state) {
                match previous {
                    Some(previous) => {
                        state.tags.insert(entry.id, previous);
                    }
                    None => {
                        state.tags.remove(&entry.id);
                    }
                }
                return Err(error.at(location.clone()));
            }
        }
        self.metadata(location)
    }

    fn create_folder(
        &mut self,
        parent: &Location,
        name: &FileName,
    ) -> Result<FileEntry, FilesError> {
        let (path, target) = self.checked_destination(parent, name)?;
        self.root
            .create_directory(&path)
            .map_err(|error| map_destination_error(error).at(target.clone()))?;
        self.metadata(&target)
    }

    fn rename(&mut self, source: &Location, name: &FileName) -> Result<FileEntry, FilesError> {
        if source.is_root() {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        let source_path = self.public_path(source)?;
        let source_metadata = self
            .root
            .metadata(&source_path)
            .map_err(|error| map_io_error(error).at(source.clone()))?;
        if source_metadata.kind == FilesRootEntryKind::Symlink {
            return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(source.clone()));
        }
        if source_metadata.kind == FilesRootEntryKind::Directory {
            self.root
                .validate_tree_mounts(&source_path)
                .map_err(|error| map_io_error(error).at(source.clone()))?;
        } else if source_metadata.kind != FilesRootEntryKind::RegularFile {
            return Err(FilesError::new(FilesErrorKind::UnsupportedEntry).at(source.clone()));
        }
        let parent = source
            .parent()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        let (destination_path, target) = self.checked_destination(&parent, name)?;
        self.root
            .rename_without_replacement(&source_path, &destination_path)
            .map_err(|error| map_destination_error(error).at(target.clone()))?;
        self.metadata(&target)
    }

    fn copy(
        &mut self,
        source: &Location,
        destination: &Location,
        name: &FileName,
        cancellation: &CancellationToken,
    ) -> Result<FileEntry, FilesError> {
        let source_path = self.public_path(source)?;
        let source_metadata = self
            .root
            .metadata(&source_path)
            .map_err(|error| map_io_error(error).at(source.clone()))?;
        if source_metadata.kind == FilesRootEntryKind::Symlink {
            return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(source.clone()));
        }
        let (destination_path, target) = self.checked_destination(destination, name)?;
        self.root
            .copy_path(&source_path, &destination_path, || {
                cancellation.is_cancelled()
            })
            .map_err(|error| {
                if error.kind() == std::io::ErrorKind::Interrupted {
                    FilesError::new(FilesErrorKind::Cancelled).at(target.clone())
                } else {
                    map_destination_error(error).at(target.clone())
                }
            })?;
        self.metadata(&target)
    }

    fn move_item(
        &mut self,
        source: &Location,
        destination: &Location,
        name: &FileName,
    ) -> Result<FileEntry, FilesError> {
        let source_path = self.public_path(source)?;
        let source_metadata = self
            .root
            .metadata(&source_path)
            .map_err(|error| map_io_error(error).at(source.clone()))?;
        if source_metadata.kind == FilesRootEntryKind::Symlink {
            return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(source.clone()));
        }
        if source_metadata.kind == FilesRootEntryKind::Directory {
            self.root
                .validate_tree_mounts(&source_path)
                .map_err(|error| map_io_error(error).at(source.clone()))?;
        } else if source_metadata.kind != FilesRootEntryKind::RegularFile {
            return Err(FilesError::new(FilesErrorKind::UnsupportedEntry).at(source.clone()));
        }
        let target = destination.join(name);
        if source.is_root() || target == *source || target.is_within(source) {
            return Err(FilesError::new(FilesErrorKind::DestinationInsideSource));
        }
        let (destination_path, target) = self.checked_destination(destination, name)?;
        self.root
            .rename_without_replacement(&source_path, &destination_path)
            .map_err(|error| map_destination_error(error).at(target.clone()))?;
        self.metadata(&target)
    }

    fn trash(&mut self, source: &Location) -> Result<TrashEntry, FilesError> {
        if source.is_root() {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        let source_path = self.public_path(source)?;
        let metadata = self
            .root
            .metadata(&source_path)
            .map_err(|error| map_io_error(error).at(source.clone()))?;
        if !matches!(
            metadata.kind,
            FilesRootEntryKind::RegularFile | FilesRootEntryKind::Directory
        ) {
            return Err(FilesError::new(FilesErrorKind::UnsupportedEntry).at(source.clone()));
        }
        if metadata.kind == FilesRootEntryKind::Directory {
            self.root
                .validate_tree_mounts(&source_path)
                .map_err(|error| map_io_error(error).at(source.clone()))?;
        }
        let entry = self.metadata(source)?;
        let deleted_at = system_time_seconds(SystemTime::now());
        let trashed = {
            let mut state = self.ensure_state()?;
            self.ensure_trash_dir(&mut state)?;
            let trash_name = self.next_trash_name(&mut state)?;
            let trash_path = internal_path(&format!("{TRASH_DIR}/{trash_name}"));
            self.root
                .rename_without_replacement(&source_path, &trash_path)
                .map_err(|error| map_destination_error(error).at(source.clone()))?;
            let trashed = TrashEntry {
                id: entry.id,
                original_location: source.clone(),
                kind: entry.kind,
                size_bytes: entry.size_bytes,
                deleted_at,
            };
            state.trash.push(StoredTrash {
                entry: trashed.clone(),
                trash_name,
            });
            self.persist_index(&state, &state.trash)
                .map_err(|_| FilesError::new(FilesErrorKind::PartialFailure).at(source.clone()))?;
            trashed
        };
        Ok(trashed)
    }

    fn list_trash(&self) -> Result<Vec<TrashEntry>, FilesError> {
        let state = self.ensure_state()?;
        self.verify_trash_storage(&state)?;
        Ok(state.trash.iter().map(|item| item.entry.clone()).collect())
    }

    fn restore(&mut self, id: ResourceId) -> Result<FileEntry, FilesError> {
        let item = {
            let state = self.ensure_state()?;
            state
                .trash
                .iter()
                .find(|item| item.entry.id == id)
                .map(|item| item.entry.clone())
                .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?
        };
        let destination = self.restore_destination(&item)?;
        self.restore_to(id, &destination)
    }

    fn restore_destination(&self, item: &TrashEntry) -> Result<Location, FilesError> {
        OwnerScopedView::restore_destination(self, item)
    }

    fn restore_to(
        &mut self,
        id: ResourceId,
        destination: &Location,
    ) -> Result<FileEntry, FilesError> {
        OwnerScopedView::restore_to(self, id, destination)
    }

    fn permanently_delete(&mut self, id: ResourceId) -> Result<(), FilesError> {
        let mut state = self.ensure_state()?;
        self.verify_trash_storage(&state)?;
        let index = state
            .trash
            .iter()
            .position(|item| item.entry.id == id)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
        let item = state.trash[index].clone();
        let trash_path = internal_path(&format!("{TRASH_DIR}/{}", item.trash_name));
        self.root
            .validate_tree_mounts(&trash_path)
            .map_err(|error| map_io_error(error).at(item.entry.original_location.clone()))?;
        self.root
            .remove_path(&trash_path)
            .map_err(|error| map_io_error(error).at(item.entry.original_location.clone()))?;
        let mut next_trash = state.trash.clone();
        next_trash.remove(index);
        let previous_tags = state.tags.remove(&id);
        self.persist_index(&state, &next_trash).map_err(|_| {
            if let Some(tags) = previous_tags.clone() {
                state.tags.insert(id, tags);
            }
            FilesError::new(FilesErrorKind::PartialFailure).at(item.entry.original_location.clone())
        })?;
        state.trash = next_trash;
        if self.persist_tags(&state).is_err() {
            if let Some(tags) = previous_tags {
                state.tags.insert(id, tags);
            }
            return Err(
                FilesError::new(FilesErrorKind::PartialFailure).at(item.entry.original_location)
            );
        }
        Ok(())
    }
}

fn path_for(location: &Location) -> PathBuf {
    if location.is_root() {
        PathBuf::from(".")
    } else {
        PathBuf::from(location.as_str())
    }
}

fn internal_path(relative: &str) -> PathBuf {
    Path::new(INTERNAL_DIR).join(relative)
}

fn entry_kind(kind: FilesRootEntryKind) -> EntryKind {
    match kind {
        FilesRootEntryKind::RegularFile => EntryKind::File,
        FilesRootEntryKind::Directory => EntryKind::Folder,
        FilesRootEntryKind::Symlink => EntryKind::Symlink,
        FilesRootEntryKind::Other => EntryKind::Unsupported,
    }
}

fn resource_id(metadata: FilesRootMetadata) -> ResourceId {
    ResourceId((u128::from(metadata.device) << 64) | u128::from(metadata.inode))
}

fn read_index_text(root: &FilesRootView<'_>, path: &Path) -> Result<Option<String>, FilesError> {
    let metadata = match root.metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(map_io_error(error)),
    };
    if metadata.kind != FilesRootEntryKind::RegularFile
        || metadata.size_bytes > MAX_METADATA_INDEX_BYTES as u64
    {
        return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
    }
    let mut bytes = Vec::new();
    root.with_open_file(path, OFlags::RDONLY, Mode::empty(), |file| {
        let mut bounded = (&mut *file).take((MAX_METADATA_INDEX_BYTES as u64).saturating_add(1));
        bounded.read_to_end(&mut bytes)
    })
    .map_err(map_io_error)?;
    if bytes.len() > MAX_METADATA_INDEX_BYTES {
        return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| FilesError::new(FilesErrorKind::CorruptMetadata))
}

fn load_tags_index(
    root: &FilesRootView<'_>,
) -> Result<BTreeMap<ResourceId, Vec<String>>, FilesError> {
    let Some(contents) = read_index_text(root, &internal_path(TAG_INDEX_FILE))? else {
        return Ok(BTreeMap::new());
    };
    let mut tags_by_id = BTreeMap::new();
    for line in contents.lines() {
        let (id, encoded_tags) = line
            .split_once('\t')
            .ok_or_else(|| FilesError::new(FilesErrorKind::CorruptMetadata))?;
        let id = u128::from_str_radix(id, 16)
            .map_err(|_| FilesError::new(FilesErrorKind::CorruptMetadata))?;
        let mut tags = Vec::new();
        if !encoded_tags.is_empty() {
            for encoded_tag in encoded_tags.split(',') {
                let value = String::from_utf8(hex_decode(encoded_tag)?)
                    .map_err(|_| FilesError::new(FilesErrorKind::CorruptMetadata))?;
                let tag = crate::Tag::parse(&value)
                    .map_err(|_| FilesError::new(FilesErrorKind::CorruptMetadata))?;
                if tags
                    .iter()
                    .any(|existing: &String| existing == tag.as_str())
                {
                    return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
                }
                tags.push(tag.as_str().to_owned());
            }
        }
        if tags_by_id.insert(ResourceId(id), tags).is_some() {
            return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
        }
    }
    Ok(tags_by_id)
}

fn load_trash_index(
    root: &FilesRootView<'_>,
    trash_exists: bool,
    report_recovery: bool,
) -> Result<Vec<StoredTrash>, FilesError> {
    let Some(contents) = read_index_text(root, &internal_path(INDEX_FILE))? else {
        return load_trash_orphans(root, trash_exists, Vec::new(), 0, report_recovery);
    };
    let mut dangling = 0;
    let mut items = Vec::new();
    let mut seen_ids = BTreeSet::new();
    let mut seen_names = BTreeSet::new();
    for line in contents.lines() {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 6 {
            return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
        }
        let id = u128::from_str_radix(fields[0], 16)
            .map_err(|_| FilesError::new(FilesErrorKind::CorruptMetadata))?;
        if !seen_ids.insert(ResourceId(id)) {
            return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
        }
        let deleted_at = fields[1]
            .parse::<i64>()
            .map_err(|_| FilesError::new(FilesErrorKind::CorruptMetadata))?;
        let kind = decode_kind(fields[2])?;
        let size_bytes = if fields[3] == "-" {
            None
        } else {
            Some(
                fields[3]
                    .parse::<u64>()
                    .map_err(|_| FilesError::new(FilesErrorKind::CorruptMetadata))?,
            )
        };
        let original = String::from_utf8(hex_decode(fields[4])?)
            .map_err(|_| FilesError::new(FilesErrorKind::CorruptMetadata))?;
        let original_location = Location::parse(&original)?;
        if original_location.is_root()
            || original_location
                .components()
                .next()
                .is_some_and(is_internal_component)
        {
            return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
        }
        let trash_name = String::from_utf8(hex_decode(fields[5])?)
            .map_err(|_| FilesError::new(FilesErrorKind::CorruptMetadata))?;
        if !valid_trash_name(&trash_name) || !seen_names.insert(trash_name.clone()) {
            return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
        }
        if !trash_exists {
            dangling += 1;
            continue;
        }
        let trash_path = internal_path(&format!("{TRASH_DIR}/{trash_name}"));
        let metadata = match root.metadata(&trash_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                dangling += 1;
                continue;
            }
            Err(error) => return Err(map_io_error(error)),
        };
        if !matches!(
            metadata.kind,
            FilesRootEntryKind::RegularFile | FilesRootEntryKind::Directory
        ) {
            return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
        }
        items.push(StoredTrash {
            entry: TrashEntry {
                id: ResourceId(id),
                original_location,
                kind,
                size_bytes,
                deleted_at,
            },
            trash_name,
        });
    }
    load_trash_orphans(root, trash_exists, items, dangling, report_recovery)
}

fn load_trash_orphans(
    root: &FilesRootView<'_>,
    trash_exists: bool,
    mut items: Vec<StoredTrash>,
    dangling: usize,
    report_recovery: bool,
) -> Result<Vec<StoredTrash>, FilesError> {
    let indexed_count = items.len();
    if !trash_exists {
        if report_recovery {
            report_trash_recovery(0, dangling);
        }
        return Ok(items);
    }
    let entries = root.list(&internal_path(TRASH_DIR)).map_err(map_io_error)?;
    let names: BTreeSet<_> = items.iter().map(|item| item.trash_name.clone()).collect();
    let mut ids: BTreeSet<_> = items.iter().map(|item| item.entry.id).collect();
    for entry in entries {
        let name = entry.name.as_str();
        if !valid_trash_name(name) {
            continue;
        }
        if names.contains(name) {
            continue;
        }
        let kind = match entry.metadata.kind {
            FilesRootEntryKind::Directory => EntryKind::Folder,
            FilesRootEntryKind::RegularFile => EntryKind::File,
            _ => continue,
        };
        let id = resource_id(entry.metadata);
        if !ids.insert(id) {
            return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
        }
        let original_location = Location::parse(name)?;
        items.push(StoredTrash {
            entry: TrashEntry {
                id,
                original_location,
                kind,
                size_bytes: (kind == EntryKind::File).then_some(entry.metadata.size_bytes),
                deleted_at: name
                    .split_once('-')
                    .and_then(|(_, timestamp)| timestamp.parse().ok())
                    .unwrap_or(0),
            },
            trash_name: name.to_owned(),
        });
    }
    if report_recovery {
        report_trash_recovery(items.len().saturating_sub(indexed_count), dangling);
    }
    Ok(items)
}
