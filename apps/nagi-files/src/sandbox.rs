use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use cap_std::fs::{Dir, MetadataExt, OpenOptions};

use crate::{
    CancellationToken, EntryAvailability, EntryKind, FileEntry, FileName, FilesError,
    FilesErrorKind, FilesystemProvider, Location, ProviderAvailability, ResourceId, TrashEntry,
};

const INTERNAL_DIR: &str = ".nagi-files";
const TRASH_DIR: &str = "trash";
const INDEX_FILE: &str = "trash-index-v1";
const TEMP_INDEX_FILE: &str = "trash-index-v1.tmp";
const TAG_INDEX_FILE: &str = "tags-v1";
const TEMP_TAG_INDEX_FILE: &str = "tags-v1.tmp";

#[derive(Clone)]
struct StoredTrash {
    entry: TrashEntry,
    trash_name: String,
}

/// A host-only provider rooted at one explicit directory. It rejects
/// symlinks in every path it opens and never follows a symlink during copy.
/// This preview backend is not a production Nagi capability implementation.
pub struct SandboxProvider {
    root: PathBuf,
    root_dir: Dir,
    internal_dir: Dir,
    trash_dir: Dir,
    trash: Vec<StoredTrash>,
    next_trash_id: u64,
    tags: BTreeMap<ResourceId, Vec<String>>,
}

impl SandboxProvider {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, FilesError> {
        let requested_root = root.as_ref();
        let requested_metadata = fs::symlink_metadata(requested_root).map_err(map_io_error)?;
        if requested_metadata.file_type().is_symlink() || !requested_metadata.is_dir() {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        let root = fs::canonicalize(requested_root).map_err(map_io_error)?;
        let root_metadata = fs::symlink_metadata(&root).map_err(map_io_error)?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        let root_dir =
            Dir::open_ambient_dir(&root, cap_std::ambient_authority()).map_err(map_io_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as StdMetadataExt;

            let opened_root_metadata = root_dir.dir_metadata().map_err(map_io_error)?;
            if opened_root_metadata.dev() != root_metadata.dev()
                || opened_root_metadata.ino() != root_metadata.ino()
            {
                return Err(FilesError::new(FilesErrorKind::Conflict));
            }
        }
        ensure_internal_directory(&root_dir, INTERNAL_DIR)?;
        let internal_dir = root_dir.open_dir(INTERNAL_DIR).map_err(map_io_error)?;
        ensure_internal_directory(&internal_dir, TRASH_DIR)?;
        let trash_dir = internal_dir.open_dir(TRASH_DIR).map_err(map_io_error)?;
        let trash = load_trash_index(&internal_dir, &trash_dir)?;
        let tags = load_tags_index(&internal_dir)?;
        let next_trash_id = trash
            .iter()
            .filter_map(|item| item.trash_name.split_once('-')?.0.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            .wrapping_add(1);
        Ok(Self {
            root,
            root_dir,
            internal_dir,
            trash_dir,
            trash,
            next_trash_id,
            tags,
        })
    }

    pub fn sandbox_root(&self) -> &Path {
        &self.root
    }

    fn public_directory(&self, location: &Location) -> Result<Dir, FilesError> {
        if location
            .components()
            .next()
            .is_some_and(is_internal_component)
        {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape).at(location.clone()));
        }
        let mut current = self.root_dir.open_dir(".").map_err(map_io_error)?;
        for part in location.components() {
            let metadata = current.symlink_metadata(part).map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    FilesError::new(FilesErrorKind::NotFound).at(location.clone())
                } else {
                    map_io_error(error).at(location.clone())
                }
            })?;
            if metadata.file_type().is_symlink() {
                return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(location.clone()));
            }
            if !metadata.is_dir() {
                return Err(FilesError::new(FilesErrorKind::NotDirectory).at(location.clone()));
            }
            current = current
                .open_dir(part)
                .map_err(|error| map_io_error(error).at(location.clone()))?;
        }
        Ok(current)
    }

    fn authorization_location(&self, location: &Location) -> Result<Location, FilesError> {
        let requested: Vec<_> = location.components().collect();
        if requested
            .first()
            .is_some_and(|part| is_internal_component(part))
        {
            return Ok(location.clone());
        }

        let mut canonical = Vec::with_capacity(requested.len());
        let mut current = self.root_dir.try_clone().map_err(map_io_error)?;
        for (index, part) in requested.iter().enumerate() {
            let (name, metadata, used_alias) = match resolve_component_name(&current, part)? {
                Some(resolved) => resolved,
                None => {
                    canonical.extend(requested[index..].iter().map(|part| (*part).to_owned()));
                    break;
                }
            };
            canonical.push(name.clone());

            if index + 1 == requested.len()
                || metadata.file_type().is_symlink()
                || !metadata.is_dir()
            {
                if index + 1 < requested.len() {
                    canonical.extend(requested[index + 1..].iter().map(|part| (*part).to_owned()));
                }
                break;
            }

            let next = current.open_dir(&name).map_err(|_| {
                FilesError::new(FilesErrorKind::ProviderFailure).at(location.clone())
            })?;
            let opened_metadata = next.dir_metadata().map_err(|_| {
                FilesError::new(FilesErrorKind::ProviderFailure).at(location.clone())
            })?;
            let metadata_id = stable_resource_id(&metadata);
            let opened_id = stable_resource_id(&opened_metadata);
            if (used_alias && (metadata_id.is_none() || opened_id.is_none()))
                || metadata_id
                    .zip(opened_id)
                    .is_some_and(|(left, right)| left != right)
            {
                return Err(FilesError::new(FilesErrorKind::ProviderFailure).at(location.clone()));
            }
            current = next;
        }

        Location::parse(&canonical.join("/"))
    }

    fn public_parent(&self, location: &Location) -> Result<(Dir, String), FilesError> {
        let name = location
            .file_name()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        if is_internal_component(name) {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape).at(location.clone()));
        }
        let parent = location
            .parent()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        Ok((self.public_directory(&parent)?, name.to_owned()))
    }

    fn checked_destination(
        &self,
        parent: &Location,
        name: &FileName,
    ) -> Result<(Dir, Location), FilesError> {
        if parent
            .components()
            .next()
            .is_some_and(is_internal_component)
            || is_internal_component(name.as_str())
        {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape));
        }
        let parent_dir = self.public_directory(parent)?;
        let target_location = parent.join(name);
        match parent_dir.symlink_metadata(name.as_str()) {
            Ok(_) => Err(FilesError::new(FilesErrorKind::Conflict).at(target_location)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok((parent_dir, target_location))
            }
            Err(error) => Err(map_io_error(error).at(target_location)),
        }
    }

    fn entry_from_directory(
        &self,
        parent_location: &Location,
        parent: &Dir,
        name: &FileName,
    ) -> Result<FileEntry, FilesError> {
        let metadata = parent
            .symlink_metadata(name.as_str())
            .map_err(|error| map_io_error(error).at(parent_location.join(name)))?;
        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            EntryKind::Symlink
        } else if file_type.is_dir() {
            EntryKind::Folder
        } else if file_type.is_file() {
            EntryKind::File
        } else {
            EntryKind::Unsupported
        };
        let absolute_path = self.root.join(parent_location.as_str()).join(name.as_str());
        let id = resource_id(&absolute_path, &metadata);
        Ok(FileEntry {
            id,
            name: name.clone(),
            location: parent_location.clone(),
            kind,
            size_bytes: (kind == EntryKind::File || kind == EntryKind::Symlink)
                .then_some(metadata.len()),
            created_at: metadata
                .created()
                .ok()
                .map(|time| system_time_seconds(time.into_std())),
            modified_at: metadata
                .modified()
                .ok()
                .map(|time| system_time_seconds(time.into_std())),
            tags: self.tags.get(&id).cloned().unwrap_or_default(),
            availability: if metadata.permissions().readonly() {
                EntryAvailability::ReadOnly
            } else {
                EntryAvailability::Available
            },
        })
    }

    fn next_trash_name(&mut self) -> Result<String, FilesError> {
        loop {
            let id = self.next_trash_id;
            self.next_trash_id = self.next_trash_id.wrapping_add(1).max(1);
            let name = format!("{id}-{}", system_time_seconds(SystemTime::now()));
            match self.trash_dir.symlink_metadata(&name) {
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(name),
                Err(error) => return Err(map_io_error(error)),
            }
        }
    }

    fn persist_index(&self, items: &[StoredTrash]) -> Result<(), FilesError> {
        self.verify_internal_storage()?;
        match self.internal_dir.symlink_metadata(TEMP_INDEX_FILE) {
            Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
                // Removing a symlink removes the link itself; create_new below
                // then refuses a raced replacement rather than following it.
                self.internal_dir
                    .remove_file(TEMP_INDEX_FILE)
                    .map_err(map_io_error)?;
            }
            Ok(_) => return Err(FilesError::new(FilesErrorKind::CorruptMetadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io_error(error)),
        }
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        let mut file = self
            .internal_dir
            .open_with(TEMP_INDEX_FILE, &options)
            .map_err(map_io_error)?;
        for item in items {
            let original = hex_encode(item.entry.original_location.as_str().as_bytes());
            let trash_name = hex_encode(item.trash_name.as_bytes());
            let kind = encode_kind(item.entry.kind);
            let size = item
                .entry
                .size_bytes
                .map_or_else(|| "-".to_owned(), |n| n.to_string());
            writeln!(
                file,
                "{}\t{}\t{}\t{}\t{}\t{}",
                item.entry.id, item.entry.deleted_at, kind, size, original, trash_name
            )
            .map_err(map_io_error)?;
        }
        file.sync_all().map_err(map_io_error)?;
        self.internal_dir
            .rename(TEMP_INDEX_FILE, &self.internal_dir, INDEX_FILE)
            .map_err(map_io_error)?;
        Ok(())
    }

    fn persist_tags(&self) -> Result<(), FilesError> {
        self.verify_internal_storage()?;
        match self.internal_dir.symlink_metadata(TEMP_TAG_INDEX_FILE) {
            Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
                self.internal_dir
                    .remove_file(TEMP_TAG_INDEX_FILE)
                    .map_err(map_io_error)?;
            }
            Ok(_) => return Err(FilesError::new(FilesErrorKind::CorruptMetadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io_error(error)),
        }
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        let mut file = self
            .internal_dir
            .open_with(TEMP_TAG_INDEX_FILE, &options)
            .map_err(map_io_error)?;
        for (id, tags) in &self.tags {
            let encoded = tags
                .iter()
                .map(|tag| hex_encode(tag.as_bytes()))
                .collect::<Vec<_>>()
                .join(",");
            writeln!(file, "{id}\t{encoded}").map_err(map_io_error)?;
        }
        file.sync_all().map_err(map_io_error)?;
        self.internal_dir
            .rename(TEMP_TAG_INDEX_FILE, &self.internal_dir, TAG_INDEX_FILE)
            .map_err(map_io_error)?;
        Ok(())
    }

    fn verify_internal_storage(&self) -> Result<(), FilesError> {
        let internal_path = self.root.join(INTERNAL_DIR);
        let internal_metadata = self
            .root_dir
            .symlink_metadata(INTERNAL_DIR)
            .map_err(map_io_error)?;
        let internal_handle_metadata = self.internal_dir.dir_metadata().map_err(map_io_error)?;
        if internal_metadata.file_type().is_symlink()
            || !internal_metadata.is_dir()
            || resource_id(&internal_path, &internal_metadata)
                != resource_id(&internal_path, &internal_handle_metadata)
        {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape));
        }

        let trash_path = internal_path.join(TRASH_DIR);
        let trash_metadata = self
            .internal_dir
            .symlink_metadata(TRASH_DIR)
            .map_err(map_io_error)?;
        let trash_handle_metadata = self.trash_dir.dir_metadata().map_err(map_io_error)?;
        if trash_metadata.file_type().is_symlink()
            || !trash_metadata.is_dir()
            || resource_id(&trash_path, &trash_metadata)
                != resource_id(&trash_path, &trash_handle_metadata)
        {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape));
        }
        Ok(())
    }
}

impl FilesystemProvider for SandboxProvider {
    fn availability(&self) -> ProviderAvailability {
        ProviderAvailability::Available
    }

    fn authorization_location(&self, location: &Location) -> Result<Location, FilesError> {
        SandboxProvider::authorization_location(self, location)
    }

    fn list(&self, location: &Location) -> Result<Vec<FileEntry>, FilesError> {
        let directory = self.public_directory(location)?;
        let mut entries = Vec::new();
        for child in directory.entries().map_err(map_io_error)? {
            let child = child.map_err(map_io_error)?;
            let file_name = child
                .file_name()
                .into_string()
                .map_err(|_| FilesError::new(FilesErrorKind::InvalidName))?;
            if location.is_root() && is_internal_component(&file_name) {
                continue;
            }
            let name = FileName::parse(&file_name)?;
            let child_location = location.join(&name);
            entries.push(
                self.entry_from_directory(location, &directory, &name)
                    .map_err(|error| {
                        if error.location.is_some() {
                            error
                        } else {
                            error.at(child_location)
                        }
                    })?,
            );
        }
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
        let (parent, name) = self.public_parent(location)?;
        let name = FileName::parse(&name)?;
        let parent_location = location
            .parent()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        self.entry_from_directory(&parent_location, &parent, &name)
    }

    fn read_file(&self, location: &Location, max_bytes: usize) -> Result<Vec<u8>, FilesError> {
        let (parent, name) = self.public_parent(location)?;
        let metadata = parent.symlink_metadata(&name).map_err(map_io_error)?;
        if metadata.file_type().is_symlink() {
            return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(location.clone()));
        }
        if !metadata.is_file() {
            return Err(FilesError::new(FilesErrorKind::IsDirectory).at(location.clone()));
        }
        if metadata.len() > max_bytes as u64 {
            return Err(FilesError::new(FilesErrorKind::FileTooLarge).at(location.clone()));
        }
        let file = parent.open(&name).map_err(map_io_error)?;
        let opened = file.metadata().map_err(map_io_error)?;
        let absolute_path = self.root.join(location.as_str());
        if !opened.is_file()
            || resource_id(&absolute_path, &opened) != resource_id(&absolute_path, &metadata)
        {
            return Err(FilesError::new(FilesErrorKind::Conflict).at(location.clone()));
        }
        let mut contents = Vec::new();
        file.take(max_bytes.saturating_add(1) as u64)
            .read_to_end(&mut contents)
            .map_err(map_io_error)?;
        if contents.len() > max_bytes {
            return Err(FilesError::new(FilesErrorKind::FileTooLarge).at(location.clone()));
        }
        Ok(contents)
    }

    fn set_tags(&mut self, location: &Location, tags: &[String]) -> Result<FileEntry, FilesError> {
        let entry = self.metadata(location)?;
        let previous = self.tags.get(&entry.id).cloned();
        if tags.is_empty() {
            self.tags.remove(&entry.id);
        } else {
            self.tags.insert(entry.id, tags.to_vec());
        }
        if let Err(error) = self.persist_tags() {
            match previous {
                Some(previous) => {
                    self.tags.insert(entry.id, previous);
                }
                None => {
                    self.tags.remove(&entry.id);
                }
            }
            return Err(error.at(location.clone()));
        }
        self.metadata(location)
    }

    fn create_folder(
        &mut self,
        parent: &Location,
        name: &FileName,
    ) -> Result<FileEntry, FilesError> {
        let (parent_dir, target_location) = self.checked_destination(parent, name)?;
        parent_dir
            .create_dir(name.as_str())
            .map_err(|error| map_io_error(error).at(target_location))?;
        self.entry_from_directory(parent, &parent_dir, name)
    }

    fn rename(&mut self, source: &Location, name: &FileName) -> Result<FileEntry, FilesError> {
        if source.is_root() {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        let (source_parent, source_name) = self.public_parent(source)?;
        let source_metadata = source_parent
            .symlink_metadata(&source_name)
            .map_err(map_io_error)?;
        if source_metadata.file_type().is_symlink() {
            return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(source.clone()));
        }
        let parent = source.parent().expect("non-root location has parent");
        let (target_parent, target_location) = self.checked_destination(&parent, name)?;
        source_parent
            .rename(&source_name, &target_parent, name.as_str())
            .map_err(|error| map_io_error(error).at(target_location))?;
        self.entry_from_directory(&parent, &target_parent, name)
    }

    fn copy(
        &mut self,
        source: &Location,
        destination: &Location,
        name: &FileName,
        cancellation: &CancellationToken,
    ) -> Result<FileEntry, FilesError> {
        let (source_parent, source_name) = self.public_parent(source)?;
        let source_metadata = source_parent
            .symlink_metadata(&source_name)
            .map_err(map_io_error)?;
        if source_metadata.file_type().is_symlink() {
            return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(source.clone()));
        }
        let (target_parent, target_location) = self.checked_destination(destination, name)?;
        match copy_tree(
            &source_parent,
            &source_name,
            &target_parent,
            name.as_str(),
            cancellation,
        ) {
            Ok(()) => self.entry_from_directory(destination, &target_parent, name),
            Err(error) => match remove_tree(&target_parent, name.as_str()) {
                Ok(()) => Err(error.at(target_location)),
                Err(_) => Err(FilesError::new(FilesErrorKind::PartialFailure).at(target_location)),
            },
        }
    }

    fn move_item(
        &mut self,
        source: &Location,
        destination: &Location,
        name: &FileName,
    ) -> Result<FileEntry, FilesError> {
        let (source_parent, source_name) = self.public_parent(source)?;
        let source_metadata = source_parent
            .symlink_metadata(&source_name)
            .map_err(map_io_error)?;
        if source_metadata.file_type().is_symlink() {
            return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(source.clone()));
        }
        if source.is_root()
            || destination.join(name) == *source
            || destination.join(name).is_within(source)
        {
            return Err(FilesError::new(FilesErrorKind::DestinationInsideSource));
        }
        let (target_parent, target_location) = self.checked_destination(destination, name)?;
        match source_parent.rename(&source_name, &target_parent, name.as_str()) {
            Ok(()) => self.entry_from_directory(destination, &target_parent, name),
            Err(error) if is_cross_device(&error) => {
                match copy_tree(
                    &source_parent,
                    &source_name,
                    &target_parent,
                    name.as_str(),
                    &CancellationToken::new(),
                ) {
                    Ok(()) => {
                        if remove_tree(&source_parent, &source_name).is_err() {
                            // Keep the complete destination if source removal
                            // partially fails; deleting both copies could lose
                            // data. Surface the partial move for user review.
                            return Err(
                                FilesError::new(FilesErrorKind::PartialFailure).at(source.clone())
                            );
                        }
                        self.entry_from_directory(destination, &target_parent, name)
                    }
                    Err(copy_error) => {
                        if remove_tree(&target_parent, name.as_str()).is_err() {
                            Err(FilesError::new(FilesErrorKind::PartialFailure).at(target_location))
                        } else {
                            Err(copy_error.at(target_location))
                        }
                    }
                }
            }
            Err(error) => Err(map_io_error(error).at(target_location)),
        }
    }

    fn trash(&mut self, source: &Location) -> Result<TrashEntry, FilesError> {
        if source.is_root() {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        self.verify_internal_storage()?;
        let (source_parent, source_name) = self.public_parent(source)?;
        let metadata = source_parent
            .symlink_metadata(&source_name)
            .map_err(map_io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() && !metadata.is_dir() {
            return Err(FilesError::new(FilesErrorKind::UnsupportedEntry).at(source.clone()));
        }
        let entry = self.metadata(source)?;
        let deleted_at = system_time_seconds(SystemTime::now());
        let trash_name = self.next_trash_name()?;
        let stored = StoredTrash {
            entry: TrashEntry {
                id: entry.id,
                original_location: source.clone(),
                kind: entry.kind,
                size_bytes: entry.size_bytes,
                deleted_at,
            },
            trash_name: trash_name.clone(),
        };
        source_parent
            .rename(&source_name, &self.trash_dir, &trash_name)
            .map_err(|error| map_io_error(error).at(source.clone()))?;
        let mut next = self.trash.clone();
        next.push(stored.clone());
        if let Err(error) = self.persist_index(&next) {
            if self
                .trash_dir
                .rename(&trash_name, &source_parent, &source_name)
                .is_err()
            {
                return Err(FilesError::new(FilesErrorKind::PartialFailure).at(source.clone()));
            }
            return Err(error.at(source.clone()));
        }
        self.trash = next;
        Ok(stored.entry.clone())
    }

    fn list_trash(&self) -> Result<Vec<TrashEntry>, FilesError> {
        self.verify_internal_storage()?;
        Ok(self.trash.iter().map(|item| item.entry.clone()).collect())
    }

    fn restore(&mut self, id: ResourceId) -> Result<FileEntry, FilesError> {
        self.verify_internal_storage()?;
        let index = self
            .trash
            .iter()
            .position(|item| item.entry.id == id)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
        let item = self.trash[index].clone();
        let (destination_parent, destination_name) =
            self.checked_restore_destination(&item.entry.original_location)?;
        self.trash_dir
            .rename(&item.trash_name, &destination_parent, &destination_name)
            .map_err(|error| map_io_error(error).at(item.entry.original_location.clone()))?;
        let mut next = self.trash.clone();
        next.remove(index);
        if let Err(error) = self.persist_index(&next) {
            if destination_parent
                .rename(&destination_name, &self.trash_dir, &item.trash_name)
                .is_err()
            {
                return Err(FilesError::new(FilesErrorKind::PartialFailure)
                    .at(item.entry.original_location));
            }
            return Err(error);
        }
        self.trash = next;
        self.metadata(&item.entry.original_location)
    }

    fn permanently_delete(&mut self, id: ResourceId) -> Result<(), FilesError> {
        self.verify_internal_storage()?;
        let index = self
            .trash
            .iter()
            .position(|item| item.entry.id == id)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
        let item = self.trash[index].clone();
        remove_tree(&self.trash_dir, &item.trash_name).map_err(map_io_error)?;
        let mut next = self.trash.clone();
        next.remove(index);
        self.trash = next;
        self.tags.remove(&id);
        if self.persist_index(&self.trash).is_err() || self.persist_tags().is_err() {
            return Err(
                FilesError::new(FilesErrorKind::PartialFailure).at(item.entry.original_location)
            );
        }
        Ok(())
    }
}

impl SandboxProvider {
    fn checked_restore_destination(
        &self,
        location: &Location,
    ) -> Result<(Dir, String), FilesError> {
        if location.is_root()
            || location
                .components()
                .next()
                .is_some_and(is_internal_component)
        {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation).at(location.clone()));
        }
        let parent = location
            .parent()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        let parent_dir = self.public_directory(&parent)?;
        let name = location
            .file_name()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        match parent_dir.symlink_metadata(name) {
            Ok(_) => Err(FilesError::new(FilesErrorKind::Conflict).at(location.clone())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok((parent_dir, name.to_owned()))
            }
            Err(error) => Err(map_io_error(error).at(location.clone())),
        }
    }
}

fn is_internal_component(value: &str) -> bool {
    value.eq_ignore_ascii_case(INTERNAL_DIR)
}

fn resolve_component_name(
    directory: &Dir,
    requested: &str,
) -> Result<Option<(String, cap_std::fs::Metadata, bool)>, FilesError> {
    let mut names = Vec::<OsString>::new();
    for entry in directory
        .entries()
        .map_err(|_| FilesError::new(FilesErrorKind::ProviderFailure))?
    {
        let entry = entry.map_err(|_| FilesError::new(FilesErrorKind::ProviderFailure))?;
        let name = entry.file_name();
        if name == OsStr::new(requested) {
            let metadata = directory
                .symlink_metadata(&name)
                .map_err(|_| FilesError::new(FilesErrorKind::ProviderFailure))?;
            let name = name
                .into_string()
                .map_err(|_| FilesError::new(FilesErrorKind::ProviderFailure))?;
            return Ok(Some((name, metadata, false)));
        }
        names.push(name);
    }

    let requested_metadata = match directory.symlink_metadata(requested) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(FilesError::new(FilesErrorKind::ProviderFailure)),
    };
    let requested_id = stable_resource_id(&requested_metadata)
        .ok_or_else(|| FilesError::new(FilesErrorKind::ProviderFailure))?;

    let mut matching_name = None;
    for name in names {
        let metadata = directory
            .symlink_metadata(&name)
            .map_err(|_| FilesError::new(FilesErrorKind::ProviderFailure))?;
        if stable_resource_id(&metadata) == Some(requested_id) {
            if matching_name.is_some() {
                // Multiple hard links have the same identity, so metadata
                // alone cannot safely reveal which spelling the host resolved.
                return Err(FilesError::new(FilesErrorKind::ProviderFailure));
            }
            matching_name = Some((name, metadata));
        }
    }

    matching_name
        .map(|(name, metadata)| {
            let name = name
                .into_string()
                .map_err(|_| FilesError::new(FilesErrorKind::ProviderFailure))?;
            Ok((name, metadata, true))
        })
        .transpose()
}

fn ensure_internal_directory(parent: &Dir, name: &str) -> Result<(), FilesError> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            parent.create_dir(name).map_err(map_io_error)
        }
        Err(error) => Err(map_io_error(error)),
    }
}

fn load_trash_index(internal: &Dir, trash: &Dir) -> Result<Vec<StoredTrash>, FilesError> {
    let index_metadata = match internal.symlink_metadata(INDEX_FILE) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(map_io_error(error)),
    };
    if index_metadata.file_type().is_symlink() || !index_metadata.is_file() {
        return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
    }
    let contents = internal.read_to_string(INDEX_FILE).map_err(map_io_error)?;
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
        let metadata = match trash.symlink_metadata(&trash_name) {
            Ok(metadata) => metadata,
            // A crash after confirmed permanent deletion but before the index
            // update leaves a stale manifest row; omit the missing object.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(map_io_error(error)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() && !metadata.is_dir() {
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
    Ok(items)
}

fn load_tags_index(internal: &Dir) -> Result<BTreeMap<ResourceId, Vec<String>>, FilesError> {
    let metadata = match internal.symlink_metadata(TAG_INDEX_FILE) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(map_io_error(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
    }
    let contents = internal
        .read_to_string(TAG_INDEX_FILE)
        .map_err(map_io_error)?;
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

fn copy_tree(
    source_parent: &Dir,
    source_name: &str,
    destination_parent: &Dir,
    destination_name: &str,
    cancellation: &CancellationToken,
) -> Result<(), FilesError> {
    if cancellation.is_cancelled() {
        return Err(FilesError::new(FilesErrorKind::Cancelled));
    }
    let metadata = source_parent
        .symlink_metadata(source_name)
        .map_err(map_io_error)?;
    if metadata.file_type().is_symlink() {
        return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed));
    }
    if metadata.is_file() {
        let mut source_file = source_parent.open(source_name).map_err(map_io_error)?;
        let opened_metadata = source_file.metadata().map_err(map_io_error)?;
        if !opened_metadata.is_file()
            || resource_id(Path::new(source_name), &metadata)
                != resource_id(Path::new(source_name), &opened_metadata)
        {
            return Err(FilesError::new(FilesErrorKind::Conflict));
        }
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        let mut destination_file = destination_parent
            .open_with(destination_name, &options)
            .map_err(map_io_error)?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            if cancellation.is_cancelled() {
                return Err(FilesError::new(FilesErrorKind::Cancelled));
            }
            let read = source_file.read(&mut buffer).map_err(map_io_error)?;
            if read == 0 {
                break;
            }
            destination_file
                .write_all(&buffer[..read])
                .map_err(map_io_error)?;
        }
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(FilesError::new(FilesErrorKind::UnsupportedEntry));
    }
    destination_parent
        .create_dir(destination_name)
        .map_err(map_io_error)?;
    let source_directory = source_parent.open_dir(source_name).map_err(map_io_error)?;
    let opened_metadata = source_directory.dir_metadata().map_err(map_io_error)?;
    if resource_id(Path::new(source_name), &metadata)
        != resource_id(Path::new(source_name), &opened_metadata)
    {
        return Err(FilesError::new(FilesErrorKind::Conflict));
    }
    let destination_directory = destination_parent
        .open_dir(destination_name)
        .map_err(map_io_error)?;
    for entry in source_directory.entries().map_err(map_io_error)? {
        if cancellation.is_cancelled() {
            return Err(FilesError::new(FilesErrorKind::Cancelled));
        }
        let entry = entry.map_err(map_io_error)?;
        let child_name = entry
            .file_name()
            .into_string()
            .map_err(|_| FilesError::new(FilesErrorKind::InvalidName))?;
        FileName::parse(&child_name)?;
        copy_tree(
            &source_directory,
            &child_name,
            &destination_directory,
            &child_name,
            cancellation,
        )?;
    }
    Ok(())
}

fn remove_tree(parent: &Dir, name: &str) -> std::io::Result<()> {
    let metadata = match parent.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        parent.remove_file(name)
    } else if metadata.is_dir() {
        parent.remove_dir_all(name)
    } else {
        Err(std::io::Error::other("unsupported filesystem entry"))
    }
}

fn stable_resource_id(metadata: &cap_std::fs::Metadata) -> Option<ResourceId> {
    #[cfg(unix)]
    {
        Some(ResourceId(
            (u128::from(metadata.dev()) << 64) | u128::from(metadata.ino()),
        ))
    }
    #[cfg(windows)]
    {
        metadata
            .volume_serial_number()
            .zip(metadata.file_index())
            .map(|(volume_serial_number, file_index)| {
                windows_resource_id(volume_serial_number, file_index)
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

fn resource_id(path: &Path, metadata: &cap_std::fs::Metadata) -> ResourceId {
    if let Some(id) = stable_resource_id(metadata) {
        return id;
    }
    #[cfg(not(unix))]
    {
        path_fallback_resource_id(path)
    }
    #[cfg(unix)]
    {
        let _ = path;
        unreachable!("Unix metadata always has a stable resource identity")
    }
}

#[cfg(any(windows, test))]
fn windows_resource_id(volume_serial_number: u32, file_index: u64) -> ResourceId {
    ResourceId((u128::from(volume_serial_number) << 64) | u128::from(file_index))
}

#[cfg(not(unix))]
fn path_fallback_resource_id(path: &Path) -> ResourceId {
    let mut hash = 0xcbf2_9ce4_8422_2325_u128;
    for byte in path.to_string_lossy().as_bytes() {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    ResourceId(hash)
}

fn map_io_error(error: std::io::Error) -> FilesError {
    let kind = match error.kind() {
        std::io::ErrorKind::NotFound => FilesErrorKind::NotFound,
        std::io::ErrorKind::AlreadyExists => FilesErrorKind::AlreadyExists,
        std::io::ErrorKind::PermissionDenied => FilesErrorKind::PermissionDenied,
        std::io::ErrorKind::NotADirectory => FilesErrorKind::NotDirectory,
        _ => FilesErrorKind::ProviderFailure,
    };
    FilesError::new(kind)
}

fn system_time_seconds(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() as i64)
}

fn encode_kind(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::File => "f",
        EntryKind::Folder => "d",
        EntryKind::Symlink => "l",
        EntryKind::Unsupported => "u",
    }
}

fn decode_kind(value: &str) -> Result<EntryKind, FilesError> {
    match value {
        "f" => Ok(EntryKind::File),
        "d" => Ok(EntryKind::Folder),
        "l" => Ok(EntryKind::Symlink),
        "u" => Ok(EntryKind::Unsupported),
        _ => Err(FilesError::new(FilesErrorKind::CorruptMetadata)),
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(char::from(HEX[(byte >> 4) as usize]));
        encoded.push(char::from(HEX[(byte & 0xf) as usize]));
    }
    encoded
}

fn hex_decode(value: &str) -> Result<Vec<u8>, FilesError> {
    if !value.len().is_multiple_of(2) {
        return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let hi = hex_nibble(chunk[0])?;
            let lo = hex_nibble(chunk[1])?;
            Ok((hi << 4) | lo)
        })
        .collect()
}

fn hex_nibble(value: u8) -> Result<u8, FilesError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        _ => Err(FilesError::new(FilesErrorKind::CorruptMetadata)),
    }
}

fn valid_trash_name(value: &str) -> bool {
    if value.len() > 48 {
        return false;
    }
    let mut parts = value.split('-');
    let Some(sequence) = parts.next() else {
        return false;
    };
    let Some(timestamp) = parts.next() else {
        return false;
    };
    sequence.bytes().all(|byte| byte.is_ascii_digit())
        && timestamp.bytes().all(|byte| byte.is_ascii_digit())
        && !sequence.is_empty()
        && !timestamp.is_empty()
        && parts.next().is_none()
}

fn is_cross_device(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(18)
    }
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(17)
    }
    #[cfg(not(any(unix, windows)))]
    {
        false
    }
}

#[cfg(test)]
mod windows_identity_tests {
    use super::windows_resource_id;

    #[test]
    fn windows_resource_id_uses_volume_and_file_identity() {
        let first_location = windows_resource_id(0x1234_abcd, 0x0102_0304_0506_0708);
        let moved_location = windows_resource_id(0x1234_abcd, 0x0102_0304_0506_0708);

        assert_eq!(first_location, moved_location);
        assert_ne!(
            first_location,
            windows_resource_id(0x1234_abce, 0x0102_0304_0506_0708)
        );
        assert_ne!(
            first_location,
            windows_resource_id(0x1234_abcd, 0x0102_0304_0506_0709)
        );
    }
}
