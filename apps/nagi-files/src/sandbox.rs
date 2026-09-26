use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

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
    trash_root: PathBuf,
    index_path: PathBuf,
    temp_index_path: PathBuf,
    tag_index_path: PathBuf,
    temp_tag_index_path: PathBuf,
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
        let internal = root.join(INTERNAL_DIR);
        ensure_internal_directory(&root, &internal)?;
        let trash_root = internal.join(TRASH_DIR);
        ensure_internal_directory(&root, &trash_root)?;
        let index_path = internal.join(INDEX_FILE);
        let temp_index_path = internal.join(TEMP_INDEX_FILE);
        let tag_index_path = internal.join(TAG_INDEX_FILE);
        let temp_tag_index_path = internal.join(TEMP_TAG_INDEX_FILE);
        let trash = load_trash_index(&root, &trash_root, &index_path)?;
        let tags = load_tags_index(&root, &tag_index_path)?;
        let next_trash_id = trash
            .iter()
            .filter_map(|item| item.trash_name.split_once('-')?.0.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            .wrapping_add(1);
        Ok(Self {
            root,
            trash_root,
            index_path,
            temp_index_path,
            tag_index_path,
            temp_tag_index_path,
            trash,
            next_trash_id,
            tags,
        })
    }

    pub fn sandbox_root(&self) -> &Path {
        &self.root
    }

    fn checked_public_path(&self, location: &Location) -> Result<PathBuf, FilesError> {
        if location
            .components()
            .next()
            .is_some_and(is_internal_component)
        {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape).at(location.clone()));
        }
        let mut current = self.root.clone();
        for part in location.components() {
            current.push(part);
            let metadata = fs::symlink_metadata(&current).map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound {
                    FilesError::new(FilesErrorKind::NotFound).at(location.clone())
                } else {
                    map_io_error(error).at(location.clone())
                }
            })?;
            if metadata.file_type().is_symlink() {
                return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(location.clone()));
            }
            let canonical = fs::canonicalize(&current).map_err(map_io_error)?;
            if !canonical.starts_with(&self.root) {
                return Err(FilesError::new(FilesErrorKind::SandboxEscape).at(location.clone()));
            }
            current = canonical;
        }
        Ok(current)
    }

    fn checked_destination(
        &self,
        parent: &Location,
        name: &FileName,
    ) -> Result<(PathBuf, Location), FilesError> {
        if parent
            .components()
            .next()
            .is_some_and(is_internal_component)
            || is_internal_component(name.as_str())
        {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape));
        }
        let parent_path = self.checked_public_path(parent)?;
        let parent_metadata = fs::symlink_metadata(&parent_path).map_err(map_io_error)?;
        if !parent_metadata.is_dir() {
            return Err(FilesError::new(FilesErrorKind::NotDirectory).at(parent.clone()));
        }
        let target_location = parent.join(name);
        let target = parent_path.join(name.as_str());
        match fs::symlink_metadata(&target) {
            Ok(_) => Err(FilesError::new(FilesErrorKind::Conflict).at(target_location)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok((target, target_location))
            }
            Err(error) => Err(map_io_error(error).at(target_location)),
        }
    }

    fn entry_from_path(
        &self,
        location: &Location,
        path: &Path,
        name: &FileName,
    ) -> Result<FileEntry, FilesError> {
        let metadata =
            fs::symlink_metadata(path).map_err(|error| map_io_error(error).at(location.clone()))?;
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
        let id = resource_id(path, &metadata);
        Ok(FileEntry {
            id,
            name: name.clone(),
            location: location.clone(),
            kind,
            size_bytes: (kind == EntryKind::File || kind == EntryKind::Symlink)
                .then_some(metadata.len()),
            created_at: metadata.created().ok().map(system_time_seconds),
            modified_at: metadata.modified().ok().map(system_time_seconds),
            tags: self.tags.get(&id).cloned().unwrap_or_default(),
            availability: if metadata.permissions().readonly() {
                EntryAvailability::ReadOnly
            } else {
                EntryAvailability::Available
            },
        })
    }

    fn next_trash_name(&mut self) -> String {
        loop {
            let id = self.next_trash_id;
            self.next_trash_id = self.next_trash_id.wrapping_add(1).max(1);
            let name = format!("{id}-{}", system_time_seconds(SystemTime::now()));
            if !self.trash_root.join(&name).exists() {
                return name;
            }
        }
    }

    fn persist_index(&self, items: &[StoredTrash]) -> Result<(), FilesError> {
        self.verify_internal_storage()?;
        match fs::symlink_metadata(&self.temp_index_path) {
            Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
                // Removing a symlink removes the link itself; create_new below
                // then refuses a raced replacement rather than following it.
                fs::remove_file(&self.temp_index_path).map_err(map_io_error)?;
            }
            Ok(_) => return Err(FilesError::new(FilesErrorKind::CorruptMetadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io_error(error)),
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&self.temp_index_path)
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
        fs::rename(&self.temp_index_path, &self.index_path).map_err(map_io_error)?;
        Ok(())
    }

    fn persist_tags(&self) -> Result<(), FilesError> {
        self.verify_internal_storage()?;
        match fs::symlink_metadata(&self.temp_tag_index_path) {
            Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
                fs::remove_file(&self.temp_tag_index_path).map_err(map_io_error)?;
            }
            Ok(_) => return Err(FilesError::new(FilesErrorKind::CorruptMetadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io_error(error)),
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&self.temp_tag_index_path)
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
        fs::rename(&self.temp_tag_index_path, &self.tag_index_path).map_err(map_io_error)?;
        Ok(())
    }

    fn verify_internal_storage(&self) -> Result<(), FilesError> {
        for path in [&self.root.join(INTERNAL_DIR), &self.trash_root] {
            let metadata = fs::symlink_metadata(path).map_err(map_io_error)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed));
            }
            let canonical = fs::canonicalize(path).map_err(map_io_error)?;
            if !canonical.starts_with(&self.root) {
                return Err(FilesError::new(FilesErrorKind::SandboxEscape));
            }
        }
        Ok(())
    }
}

impl FilesystemProvider for SandboxProvider {
    fn availability(&self) -> ProviderAvailability {
        ProviderAvailability::Available
    }

    fn list(&self, location: &Location) -> Result<Vec<FileEntry>, FilesError> {
        let path = self.checked_public_path(location)?;
        if !fs::metadata(&path).map_err(map_io_error)?.is_dir() {
            return Err(FilesError::new(FilesErrorKind::NotDirectory).at(location.clone()));
        }
        let mut entries = Vec::new();
        for child in fs::read_dir(path).map_err(map_io_error)? {
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
                self.entry_from_path(location, &child.path(), &name)
                    .map_err(|e| {
                        if e.location.is_some() {
                            e
                        } else {
                            e.at(child_location)
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
        let path = self.checked_public_path(location)?;
        let name = FileName::parse(
            location
                .file_name()
                .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?,
        )?;
        let parent = location
            .parent()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        self.entry_from_path(&parent, &path, &name)
    }

    fn read_file(&self, location: &Location, max_bytes: usize) -> Result<Vec<u8>, FilesError> {
        let path = self.checked_public_path(location)?;
        let metadata = fs::symlink_metadata(&path).map_err(map_io_error)?;
        if metadata.file_type().is_symlink() {
            return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(location.clone()));
        }
        if !metadata.is_file() {
            return Err(FilesError::new(FilesErrorKind::IsDirectory).at(location.clone()));
        }
        if metadata.len() > max_bytes as u64 {
            return Err(FilesError::new(FilesErrorKind::FileTooLarge).at(location.clone()));
        }
        let file = File::open(path).map_err(map_io_error)?;
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
        let (target, target_location) = self.checked_destination(parent, name)?;
        fs::create_dir(&target).map_err(|error| map_io_error(error).at(target_location.clone()))?;
        self.entry_from_path(parent, &target, name)
    }

    fn rename(&mut self, source: &Location, name: &FileName) -> Result<FileEntry, FilesError> {
        let source_path = self.checked_public_path(source)?;
        if source.is_root() {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        let parent = source.parent().expect("non-root location has parent");
        let (target, target_location) = self.checked_destination(&parent, name)?;
        fs::rename(&source_path, &target)
            .map_err(|error| map_io_error(error).at(target_location.clone()))?;
        self.entry_from_path(&parent, &target, name)
    }

    fn copy(
        &mut self,
        source: &Location,
        destination: &Location,
        name: &FileName,
        cancellation: &CancellationToken,
    ) -> Result<FileEntry, FilesError> {
        let source_path = self.checked_public_path(source)?;
        let source_metadata = fs::symlink_metadata(&source_path).map_err(map_io_error)?;
        if source_metadata.file_type().is_symlink() {
            return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(source.clone()));
        }
        let (target, target_location) = self.checked_destination(destination, name)?;
        match copy_tree(&source_path, &target, cancellation) {
            Ok(()) => self.entry_from_path(destination, &target, name),
            Err(error) => match remove_tree(&target) {
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
        let source_path = self.checked_public_path(source)?;
        if source.is_root()
            || destination.join(name) == *source
            || destination.join(name).is_within(source)
        {
            return Err(FilesError::new(FilesErrorKind::DestinationInsideSource));
        }
        let (target, target_location) = self.checked_destination(destination, name)?;
        match fs::rename(&source_path, &target) {
            Ok(()) => self.entry_from_path(destination, &target, name),
            Err(error) if is_cross_device(&error) => {
                match copy_tree(&source_path, &target, &CancellationToken::new()) {
                    Ok(()) => {
                        if remove_tree(&source_path).is_err() {
                            // Keep the complete destination if source removal
                            // partially fails; deleting both copies could lose
                            // data. Surface the partial move for user review.
                            return Err(
                                FilesError::new(FilesErrorKind::PartialFailure).at(source.clone())
                            );
                        }
                        self.entry_from_path(destination, &target, name)
                    }
                    Err(copy_error) => {
                        if remove_tree(&target).is_err() {
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
        let source_path = self.checked_public_path(source)?;
        let metadata = fs::symlink_metadata(&source_path).map_err(map_io_error)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() && !metadata.is_dir() {
            return Err(FilesError::new(FilesErrorKind::UnsupportedEntry).at(source.clone()));
        }
        let entry = self.metadata(source)?;
        let deleted_at = system_time_seconds(SystemTime::now());
        let trash_name = self.next_trash_name();
        let destination = self.trash_root.join(&trash_name);
        let stored = StoredTrash {
            entry: TrashEntry {
                id: entry.id,
                original_location: source.clone(),
                kind: entry.kind,
                size_bytes: entry.size_bytes,
                deleted_at,
            },
            trash_name,
        };
        fs::rename(&source_path, &destination)
            .map_err(|error| map_io_error(error).at(source.clone()))?;
        let mut next = self.trash.clone();
        next.push(stored.clone());
        if let Err(error) = self.persist_index(&next) {
            if fs::rename(&destination, &source_path).is_err() {
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
        let source_path = self.trash_root.join(&item.trash_name);
        let destination = self.checked_restore_destination(&item.entry.original_location)?;
        fs::rename(&source_path, &destination)
            .map_err(|error| map_io_error(error).at(item.entry.original_location.clone()))?;
        let mut next = self.trash.clone();
        next.remove(index);
        if let Err(error) = self.persist_index(&next) {
            if fs::rename(&destination, &source_path).is_err() {
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
        let target = self.trash_root.join(&item.trash_name);
        remove_tree(&target).map_err(map_io_error)?;
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
    fn checked_restore_destination(&self, location: &Location) -> Result<PathBuf, FilesError> {
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
        let parent_path = self.checked_public_path(&parent)?;
        let name = location
            .file_name()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        let destination = parent_path.join(name);
        match fs::symlink_metadata(&destination) {
            Ok(_) => Err(FilesError::new(FilesErrorKind::Conflict).at(location.clone())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(destination),
            Err(error) => Err(map_io_error(error).at(location.clone())),
        }
    }
}

fn is_internal_component(value: &str) -> bool {
    value.eq_ignore_ascii_case(INTERNAL_DIR)
}

fn ensure_internal_directory(root: &Path, directory: &Path) -> Result<(), FilesError> {
    match fs::symlink_metadata(directory) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed))
        }
        Ok(_) => {
            let canonical = fs::canonicalize(directory).map_err(map_io_error)?;
            if canonical.starts_with(root) {
                Ok(())
            } else {
                Err(FilesError::new(FilesErrorKind::SandboxEscape))
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(directory).map_err(map_io_error)
        }
        Err(error) => Err(map_io_error(error)),
    }
}

fn load_trash_index(
    root: &Path,
    trash_root: &Path,
    index_path: &Path,
) -> Result<Vec<StoredTrash>, FilesError> {
    let index_metadata = match fs::symlink_metadata(index_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(map_io_error(error)),
    };
    if index_metadata.file_type().is_symlink() || !index_metadata.is_file() {
        return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
    }
    if !fs::canonicalize(index_path)
        .map_err(map_io_error)?
        .starts_with(root)
    {
        return Err(FilesError::new(FilesErrorKind::SandboxEscape));
    }
    let contents = fs::read_to_string(index_path).map_err(map_io_error)?;
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
        let trash_path = trash_root.join(&trash_name);
        let metadata = match fs::symlink_metadata(&trash_path) {
            Ok(metadata) => metadata,
            // A crash after confirmed permanent deletion but before the index
            // update leaves a stale manifest row; omit the missing object.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(map_io_error(error)),
        };
        if metadata.file_type().is_symlink() || !metadata.is_file() && !metadata.is_dir() {
            return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
        }
        if !trash_path
            .canonicalize()
            .map_err(map_io_error)?
            .starts_with(root)
        {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape));
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

fn load_tags_index(
    root: &Path,
    index_path: &Path,
) -> Result<BTreeMap<ResourceId, Vec<String>>, FilesError> {
    let metadata = match fs::symlink_metadata(index_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(map_io_error(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
    }
    if !fs::canonicalize(index_path)
        .map_err(map_io_error)?
        .starts_with(root)
    {
        return Err(FilesError::new(FilesErrorKind::SandboxEscape));
    }
    let contents = fs::read_to_string(index_path).map_err(map_io_error)?;
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
    source: &Path,
    destination: &Path,
    cancellation: &CancellationToken,
) -> Result<(), FilesError> {
    if cancellation.is_cancelled() {
        return Err(FilesError::new(FilesErrorKind::Cancelled));
    }
    let metadata = fs::symlink_metadata(source).map_err(map_io_error)?;
    if metadata.file_type().is_symlink() {
        return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed));
    }
    if metadata.is_file() {
        fs::copy(source, destination).map_err(map_io_error)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(FilesError::new(FilesErrorKind::UnsupportedEntry));
    }
    fs::create_dir(destination).map_err(map_io_error)?;
    for entry in fs::read_dir(source).map_err(map_io_error)? {
        if cancellation.is_cancelled() {
            return Err(FilesError::new(FilesErrorKind::Cancelled));
        }
        let entry = entry.map_err(map_io_error)?;
        let child_destination = destination.join(entry.file_name());
        copy_tree(&entry.path(), &child_destination, cancellation)?;
    }
    Ok(())
}

fn remove_tree(path: &Path) -> std::io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)
    } else if metadata.is_dir() {
        for child in fs::read_dir(path)? {
            remove_tree(&child?.path())?;
        }
        fs::remove_dir(path)
    } else {
        Err(std::io::Error::other("unsupported filesystem entry"))
    }
}

fn resource_id(_path: &Path, metadata: &fs::Metadata) -> ResourceId {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ResourceId((u128::from(metadata.dev()) << 64) | u128::from(metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let mut hash = 0xcbf2_9ce4_8422_2325_u128;
        for byte in _path.to_string_lossy().as_bytes() {
            hash ^= u128::from(*byte);
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
        ResourceId(hash)
    }
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
