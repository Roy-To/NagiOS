use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(not(target_os = "linux"))]
use cap_std::fs::OpenOptions;
use cap_std::fs::{Dir, File as CapFile, MetadataExt};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as StdMetadataExt;

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

fn next_trash_id(items: &[StoredTrash]) -> u64 {
    items
        .iter()
        .filter_map(|item| item.trash_name.split_once('-')?.0.parse::<u64>().ok())
        .max()
        .unwrap_or(0)
        .wrapping_add(1)
}

/// A cap-std FilesystemProvider with a host-preview path constructor and a
/// directory-FD entry point for a Data-issued profile root. It rejects
/// symlinks in traversed paths and refuses mount crossings. It does not supply
/// C1 authentication, Files grants, or persistent logical ResourceIds.
pub struct SandboxProvider {
    // Only the explicit-path preview constructor retains a display path. An
    // authorized-root provider is anchored solely by its owned directory FD.
    root_path: Option<PathBuf>,
    root_dir: Dir,
    internal_dir: Option<Dir>,
    trash_dir: Option<Dir>,
    root_device: Option<u64>,
    root_mount_id: Option<u64>,
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
        Self::from_root_dir(root_dir, Some(root))
    }

    /// Adopts a directory FD already issued for one authenticated User/Profile
    /// Files root. The caller must supply exactly that private root, opened by
    /// the Data owner without following a root symlink and with no group/other
    /// permission bits. This provider does not create a C1 session or Files
    /// grants; callers must still route each effect through the C1 live-session
    /// gate and a per-operation `CapabilityAuthorizer`. The FD is made
    /// close-on-exec before it is retained. The provider never resolves or
    /// records a host path for it.
    ///
    /// This cap-std adapter uses inode-based `ResourceId`s on Unix. Those IDs
    /// are useful for operations within the opened filesystem but are not
    /// stable logical IDs across reboot or inode reuse.
    #[cfg(unix)]
    pub fn from_authorized_root(root: File) -> Result<Self, FilesError> {
        ensure_close_on_exec(&root)?;
        let metadata = root.metadata().map_err(map_io_error)?;
        if !metadata.is_dir() {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        if metadata.mode() & 0o077 != 0 {
            return Err(FilesError::new(FilesErrorKind::PermissionDenied));
        }
        Self::from_root_dir(Dir::from_std_file(root), None)
    }

    pub fn sandbox_root(&self) -> Option<&Path> {
        self.root_path.as_deref()
    }

    fn resource_id(&self, relative_path: &Path, metadata: &cap_std::fs::Metadata) -> ResourceId {
        let resource_path = self.root_path.as_ref().map_or_else(
            || relative_path.to_path_buf(),
            |root| root.join(relative_path),
        );
        resource_id(&resource_path, metadata)
    }

    fn ensure_path_mount(
        &self,
        parent: &Dir,
        name: &str,
        metadata: &cap_std::fs::Metadata,
    ) -> Result<(), FilesError> {
        ensure_mount_path(self.root_device, self.root_mount_id, parent, name, metadata)
    }

    fn ensure_dir_mount(&self, directory: &Dir) -> Result<(), FilesError> {
        ensure_mount_handle(self.root_device, self.root_mount_id, directory)
    }

    fn ensure_file_mount(&self, file: &cap_std::fs::File) -> Result<(), FilesError> {
        let metadata = file.metadata().map_err(map_io_error)?;
        ensure_mount_metadata(self.root_device, &metadata)?;
        #[cfg(target_os = "linux")]
        if Some(linux_mount_id_for_handle(file)?) != self.root_mount_id {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape));
        }
        Ok(())
    }

    fn validate_tree_mounts(&self, parent: &Dir, name: &str) -> Result<(), FilesError> {
        let metadata = parent.symlink_metadata(name).map_err(map_io_error)?;
        self.ensure_path_mount(parent, name, &metadata)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Ok(());
        }
        let directory = open_directory_relative(parent, name)?;
        self.ensure_dir_mount(&directory)?;
        for entry in directory.entries().map_err(map_io_error)? {
            let entry = entry.map_err(map_io_error)?;
            let child_name = entry
                .file_name()
                .into_string()
                .map_err(|_| FilesError::new(FilesErrorKind::InvalidName))?;
            self.validate_tree_mounts(&directory, &child_name)?;
        }
        Ok(())
    }

    fn remove_tree(&self, parent: &Dir, name: &str) -> Result<(), FilesError> {
        let metadata = match parent.symlink_metadata(name) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(map_io_error(error)),
        };
        self.ensure_path_mount(parent, name, &metadata)?;
        if metadata.file_type().is_symlink() || metadata.is_file() {
            parent.remove_file(name).map_err(map_io_error)
        } else if metadata.is_dir() {
            let directory = open_directory_relative(parent, name)?;
            self.ensure_dir_mount(&directory)?;
            for entry in directory.entries().map_err(map_io_error)? {
                let entry = entry.map_err(map_io_error)?;
                let child_name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| FilesError::new(FilesErrorKind::InvalidName))?;
                self.remove_tree(&directory, &child_name)?;
            }
            directory.remove_open_dir().map_err(map_io_error)
        } else {
            Err(FilesError::new(FilesErrorKind::UnsupportedEntry))
        }
    }

    fn copy_tree(
        &self,
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
        self.ensure_path_mount(source_parent, source_name, &metadata)?;
        if metadata.file_type().is_symlink() {
            return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed));
        }
        if metadata.is_file() {
            let mut source_file = open_read_file(source_parent, source_name)?;
            let opened_metadata = source_file.metadata().map_err(map_io_error)?;
            self.ensure_file_mount(&source_file)?;
            if !opened_metadata.is_file()
                || self.resource_id(Path::new(source_name), &metadata)
                    != self.resource_id(Path::new(source_name), &opened_metadata)
            {
                return Err(FilesError::new(FilesErrorKind::Conflict));
            }
            let mut destination_file = create_new_write_file(destination_parent, destination_name)?;
            self.ensure_file_mount(&destination_file)?;
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
        let source_directory = open_directory_relative(source_parent, source_name)?;
        self.ensure_dir_mount(&source_directory)?;
        let opened_metadata = source_directory.dir_metadata().map_err(map_io_error)?;
        if self.resource_id(Path::new(source_name), &metadata)
            != self.resource_id(Path::new(source_name), &opened_metadata)
        {
            return Err(FilesError::new(FilesErrorKind::Conflict));
        }
        let destination_directory = open_directory_relative(destination_parent, destination_name)?;
        self.ensure_dir_mount(&destination_directory)?;
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
            self.copy_tree(
                &source_directory,
                &child_name,
                &destination_directory,
                &child_name,
                cancellation,
            )?;
        }
        Ok(())
    }

    fn from_root_dir(root_dir: Dir, root_path: Option<PathBuf>) -> Result<Self, FilesError> {
        let root_metadata = root_dir.dir_metadata().map_err(map_io_error)?;
        if !root_metadata.is_dir() {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        #[cfg(unix)]
        let root_device = Some(root_metadata.dev());
        #[cfg(not(unix))]
        let root_device = None;
        #[cfg(target_os = "linux")]
        let root_mount_id = Some(linux_mount_id_for_handle(&root_dir)?);
        #[cfg(not(target_os = "linux"))]
        let root_mount_id = None;

        // Opening a provider is read-only. Existing internal state is loaded
        // when present, while the metadata directories are created only by
        // an already-authorized mutation that needs them.
        let internal_dir =
            open_optional_directory_relative(&root_dir, INTERNAL_DIR, root_device, root_mount_id)?;
        let trash_dir = internal_dir
            .as_ref()
            .map(|internal| {
                open_optional_directory_relative(internal, TRASH_DIR, root_device, root_mount_id)
            })
            .transpose()?
            .flatten();
        let (trash, tags) = match internal_dir.as_ref() {
            Some(internal) => (
                load_trash_index(internal, trash_dir.as_ref(), root_device, root_mount_id)?,
                load_tags_index(internal)?,
            ),
            None => (Vec::new(), BTreeMap::new()),
        };
        let next_trash_id = next_trash_id(&trash);
        Ok(Self {
            root_path,
            root_dir,
            internal_dir,
            trash_dir,
            root_device,
            root_mount_id,
            trash,
            next_trash_id,
            tags,
        })
    }

    fn public_directory(&self, location: &Location) -> Result<Dir, FilesError> {
        if location
            .components()
            .next()
            .is_some_and(is_internal_component)
        {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape).at(location.clone()));
        }

        #[cfg(target_os = "linux")]
        {
            let path = if location.is_root() {
                "."
            } else {
                location.as_str()
            };
            let directory = open_directory_relative(&self.root_dir, path)
                .map_err(|error| error.at(location.clone()))?;
            self.ensure_dir_mount(&directory)
                .map_err(|error| error.at(location.clone()))?;
            return Ok(directory);
        }

        #[cfg(not(target_os = "linux"))]
        {
            let mut current = open_directory_relative(&self.root_dir, ".")
                .map_err(|error| error.at(location.clone()))?;
            self.ensure_dir_mount(&current)?;
            for part in location.components() {
                let metadata = current.symlink_metadata(part).map_err(|error| {
                    if error.kind() == std::io::ErrorKind::NotFound {
                        FilesError::new(FilesErrorKind::NotFound).at(location.clone())
                    } else {
                        map_io_error(error).at(location.clone())
                    }
                })?;
                if metadata.file_type().is_symlink() {
                    return Err(
                        FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(location.clone())
                    );
                }
                if !metadata.is_dir() {
                    return Err(FilesError::new(FilesErrorKind::NotDirectory).at(location.clone()));
                }
                self.ensure_path_mount(&current, part, &metadata)?;
                current = open_directory_relative(&current, part)
                    .map_err(|error| error.at(location.clone()))?;
                self.ensure_dir_mount(&current)?;
            }
            Ok(current)
        }
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
        self.ensure_path_mount(parent, name.as_str(), &metadata)?;
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
        let relative_path = Path::new(parent_location.as_str()).join(name.as_str());
        let id = self.resource_id(&relative_path, &metadata);
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
        let trash_dir = self
            .trash_dir
            .as_ref()
            .ok_or_else(|| FilesError::new(FilesErrorKind::ProviderUnavailable))?;
        loop {
            let id = self.next_trash_id;
            self.next_trash_id = self.next_trash_id.wrapping_add(1).max(1);
            let name = format!("{id}-{}", system_time_seconds(SystemTime::now()));
            match trash_dir.symlink_metadata(&name) {
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(name),
                Err(error) => return Err(map_io_error(error)),
            }
        }
    }

    fn ensure_internal_dir(&mut self) -> Result<(), FilesError> {
        if self.internal_dir.is_none() {
            ensure_internal_directory(&self.root_dir, INTERNAL_DIR)?;
            let internal_dir = open_optional_directory_relative(
                &self.root_dir,
                INTERNAL_DIR,
                self.root_device,
                self.root_mount_id,
            )?
            .ok_or_else(|| FilesError::new(FilesErrorKind::ProviderUnavailable))?;
            let trash_dir = open_optional_directory_relative(
                &internal_dir,
                TRASH_DIR,
                self.root_device,
                self.root_mount_id,
            )?;
            let tags = load_tags_index(&internal_dir)?;
            let trash = load_trash_index(
                &internal_dir, trash_dir.as_ref(), self.root_device, self.root_mount_id,
            )?;
            self.next_trash_id = next_trash_id(&trash);
            self.tags = tags;
            self.trash = trash;
            self.trash_dir = trash_dir;
            self.internal_dir = Some(internal_dir);
        }
        self.verify_internal_directory()
    }

    fn ensure_trash_dir(&mut self) -> Result<(), FilesError> {
        self.ensure_internal_dir()?;
        if self.trash_dir.is_none() {
            let internal_dir = self
                .internal_dir
                .as_ref()
                .ok_or_else(|| FilesError::new(FilesErrorKind::ProviderUnavailable))?;
            ensure_internal_directory(internal_dir, TRASH_DIR)?;
            let trash_dir = open_optional_directory_relative(
                internal_dir,
                TRASH_DIR,
                self.root_device,
                self.root_mount_id,
            )?
            .ok_or_else(|| FilesError::new(FilesErrorKind::ProviderUnavailable))?;
            let trash = load_trash_index(
                internal_dir, Some(&trash_dir), self.root_device, self.root_mount_id,
            )?;
            self.next_trash_id = next_trash_id(&trash);
            self.trash = trash;
            self.trash_dir = Some(trash_dir);
        }
        self.verify_internal_storage()
    }

    fn persist_index(&self, items: &[StoredTrash]) -> Result<(), FilesError> {
        self.verify_internal_storage()?;
        let internal_dir = self
            .internal_dir
            .as_ref()
            .ok_or_else(|| FilesError::new(FilesErrorKind::ProviderUnavailable))?;
        match internal_dir.symlink_metadata(TEMP_INDEX_FILE) {
            Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
                // Removing a symlink removes the link itself; create_new below
                // then refuses a raced replacement rather than following it.
                internal_dir
                    .remove_file(TEMP_INDEX_FILE)
                    .map_err(map_io_error)?;
            }
            Ok(_) => return Err(FilesError::new(FilesErrorKind::CorruptMetadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io_error(error)),
        }
        let mut file = create_new_write_file(internal_dir, TEMP_INDEX_FILE)?;
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
        internal_dir
            .rename(TEMP_INDEX_FILE, internal_dir, INDEX_FILE)
            .map_err(map_io_error)?;
        sync_directory_change(internal_dir, &Location::root())
    }

    fn persist_tags(&self) -> Result<(), FilesError> {
        self.verify_internal_directory()?;
        let internal_dir = self
            .internal_dir
            .as_ref()
            .ok_or_else(|| FilesError::new(FilesErrorKind::ProviderUnavailable))?;
        match internal_dir.symlink_metadata(TEMP_TAG_INDEX_FILE) {
            Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
                internal_dir
                    .remove_file(TEMP_TAG_INDEX_FILE)
                    .map_err(map_io_error)?;
            }
            Ok(_) => return Err(FilesError::new(FilesErrorKind::CorruptMetadata)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(map_io_error(error)),
        }
        let mut file = create_new_write_file(internal_dir, TEMP_TAG_INDEX_FILE)?;
        for (id, tags) in &self.tags {
            let encoded = tags
                .iter()
                .map(|tag| hex_encode(tag.as_bytes()))
                .collect::<Vec<_>>()
                .join(",");
            writeln!(file, "{id}\t{encoded}").map_err(map_io_error)?;
        }
        file.sync_all().map_err(map_io_error)?;
        internal_dir
            .rename(TEMP_TAG_INDEX_FILE, internal_dir, TAG_INDEX_FILE)
            .map_err(map_io_error)?;
        Ok(())
    }

    fn verify_internal_directory(&self) -> Result<(), FilesError> {
        let internal_dir = self
            .internal_dir
            .as_ref()
            .ok_or_else(|| FilesError::new(FilesErrorKind::ProviderUnavailable))?;
        let internal_metadata = self
            .root_dir
            .symlink_metadata(INTERNAL_DIR)
            .map_err(map_io_error)?;
        self.ensure_path_mount(&self.root_dir, INTERNAL_DIR, &internal_metadata)?;
        let internal_handle_metadata = internal_dir.dir_metadata().map_err(map_io_error)?;
        self.ensure_dir_mount(internal_dir)?;
        if internal_metadata.file_type().is_symlink()
            || !internal_metadata.is_dir()
            || self.resource_id(Path::new(INTERNAL_DIR), &internal_metadata)
                != self.resource_id(Path::new(INTERNAL_DIR), &internal_handle_metadata)
        {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape));
        }

        Ok(())
    }

    fn verify_internal_storage(&self) -> Result<(), FilesError> {
        self.verify_internal_directory()?;
        let internal_dir = self
            .internal_dir
            .as_ref()
            .ok_or_else(|| FilesError::new(FilesErrorKind::ProviderUnavailable))?;
        let trash_dir = self
            .trash_dir
            .as_ref()
            .ok_or_else(|| FilesError::new(FilesErrorKind::ProviderUnavailable))?;
        let trash_metadata = internal_dir
            .symlink_metadata(TRASH_DIR)
            .map_err(map_io_error)?;
        self.ensure_path_mount(internal_dir, TRASH_DIR, &trash_metadata)?;
        let trash_handle_metadata = trash_dir.dir_metadata().map_err(map_io_error)?;
        self.ensure_dir_mount(trash_dir)?;
        if trash_metadata.file_type().is_symlink()
            || !trash_metadata.is_dir()
            || self.resource_id(
                Path::new(INTERNAL_DIR).join(TRASH_DIR).as_path(),
                &trash_metadata,
            ) != self.resource_id(
                Path::new(INTERNAL_DIR).join(TRASH_DIR).as_path(),
                &trash_handle_metadata,
            )
        {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape));
        }
        Ok(())
    }

    fn verify_trash_storage_if_present(&self) -> Result<(), FilesError> {
        match (&self.internal_dir, &self.trash_dir) {
            (None, None) => Ok(()),
            (Some(_), None) => self.verify_internal_directory(),
            (Some(_), Some(_)) => self.verify_internal_storage(),
            (None, Some(_)) => Err(FilesError::new(FilesErrorKind::SandboxEscape)),
        }
    }
}

impl FilesystemProvider for SandboxProvider {
    fn availability(&self) -> ProviderAvailability {
        ProviderAvailability::Available
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
        self.ensure_path_mount(&parent, &name, &metadata)?;
        if !metadata.is_file() {
            return Err(FilesError::new(FilesErrorKind::IsDirectory).at(location.clone()));
        }
        if metadata.len() > max_bytes as u64 {
            return Err(FilesError::new(FilesErrorKind::FileTooLarge).at(location.clone()));
        }
        let file = open_read_file(&parent, &name).map_err(|error| error.at(location.clone()))?;
        let opened = file.metadata().map_err(map_io_error)?;
        self.ensure_file_mount(&file)?;
        if !opened.is_file()
            || self.resource_id(Path::new(location.as_str()), &opened)
                != self.resource_id(Path::new(location.as_str()), &metadata)
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

    fn write_file(
        &mut self,
        location: &Location,
        contents: &[u8],
    ) -> Result<FileEntry, FilesError> {
        let (parent, name) = self.public_parent(location)?;
        let metadata = parent.symlink_metadata(&name).map_err(map_io_error)?;
        if metadata.file_type().is_symlink() {
            return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed).at(location.clone()));
        }
        self.ensure_path_mount(&parent, &name, &metadata)?;
        if !metadata.is_file() {
            return Err(FilesError::new(FilesErrorKind::IsDirectory).at(location.clone()));
        }
        let mut file =
            open_write_file(&parent, &name).map_err(|error| error.at(location.clone()))?;
        let opened = file
            .metadata()
            .map_err(|error| map_io_error(error).at(location.clone()))?;
        self.ensure_file_mount(&file)?;
        if !opened.is_file()
            || self.resource_id(Path::new(location.as_str()), &opened)
                != self.resource_id(Path::new(location.as_str()), &metadata)
        {
            return Err(FilesError::new(FilesErrorKind::Conflict).at(location.clone()));
        }
        let partial_write = || FilesError::new(FilesErrorKind::PartialFailure).at(location.clone());
        file.set_len(0).map_err(|_| partial_write())?;
        file.write_all(contents).map_err(|_| partial_write())?;
        file.sync_all().map_err(|_| partial_write())?;
        self.metadata(location).map_err(|_| partial_write())
    }

    fn set_tags(&mut self, location: &Location, tags: &[String]) -> Result<FileEntry, FilesError> {
        let entry = self.metadata(location)?;
        self.ensure_internal_dir()?;
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
            .map_err(|error| map_destination_error(error).at(target_location.clone()))?;
        sync_directory_change(&parent_dir, &target_location)?;
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
        self.ensure_path_mount(&source_parent, &source_name, &source_metadata)?;
        if source_metadata.is_dir() {
            self.validate_tree_mounts(&source_parent, &source_name)?;
        }
        let parent = source.parent().expect("non-root location has parent");
        let (target_parent, target_location) = self.checked_destination(&parent, name)?;
        rename_without_replace(&source_parent, &source_name, &target_parent, name.as_str())
            .map_err(|error| map_destination_error(error).at(target_location.clone()))?;
        sync_directory_change(&target_parent, &target_location)?;
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
        self.ensure_path_mount(&source_parent, &source_name, &source_metadata)?;
        let (target_parent, target_location) = self.checked_destination(destination, name)?;
        match self.copy_tree(
            &source_parent,
            &source_name,
            &target_parent,
            name.as_str(),
            cancellation,
        ) {
            Ok(()) => self.entry_from_directory(destination, &target_parent, name),
            Err(error) => match self.remove_tree(&target_parent, name.as_str()) {
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
        self.ensure_path_mount(&source_parent, &source_name, &source_metadata)?;
        if source_metadata.is_dir() {
            self.validate_tree_mounts(&source_parent, &source_name)?;
        }
        if source.is_root()
            || destination.join(name) == *source
            || destination.join(name).is_within(source)
        {
            return Err(FilesError::new(FilesErrorKind::DestinationInsideSource));
        }
        let (target_parent, target_location) = self.checked_destination(destination, name)?;
        match rename_without_replace(&source_parent, &source_name, &target_parent, name.as_str()) {
            Ok(()) => {
                sync_directory_change(&target_parent, &target_location)?;
                sync_directory_change(&source_parent, source)?;
                self.entry_from_directory(destination, &target_parent, name)
            }
            #[cfg(not(target_os = "linux"))]
            Err(error) if is_cross_device(&error) => {
                match self.copy_tree(
                    &source_parent,
                    &source_name,
                    &target_parent,
                    name.as_str(),
                    &CancellationToken::new(),
                ) {
                    Ok(()) => {
                        if self.remove_tree(&source_parent, &source_name).is_err() {
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
                        if self.remove_tree(&target_parent, name.as_str()).is_err() {
                            Err(FilesError::new(FilesErrorKind::PartialFailure).at(target_location))
                        } else {
                            Err(copy_error.at(target_location))
                        }
                    }
                }
            }
            Err(error) => Err(map_destination_error(error).at(target_location)),
        }
    }

    fn trash(&mut self, source: &Location) -> Result<TrashEntry, FilesError> {
        if source.is_root() {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        let (source_parent, source_name) = self.public_parent(source)?;
        let metadata = source_parent
            .symlink_metadata(&source_name)
            .map_err(map_io_error)?;
        self.ensure_path_mount(&source_parent, &source_name, &metadata)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() && !metadata.is_dir() {
            return Err(FilesError::new(FilesErrorKind::UnsupportedEntry).at(source.clone()));
        }
        if metadata.is_dir() {
            self.validate_tree_mounts(&source_parent, &source_name)?;
        }
        let entry = self.metadata(source)?;
        let deleted_at = system_time_seconds(SystemTime::now());
        self.ensure_trash_dir()?;
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
        let trash_dir = self
            .trash_dir
            .as_ref()
            .ok_or_else(|| FilesError::new(FilesErrorKind::ProviderUnavailable))?;
        rename_without_replace(&source_parent, &source_name, trash_dir, &trash_name)
            .map_err(|error| map_destination_error(error).at(source.clone()))?;
        let entry = stored.entry.clone();
        self.trash.push(stored);
        sync_directory_change(trash_dir, source)?;
        sync_directory_change(&source_parent, source)?;
        self.persist_index(&self.trash)
            .map_err(|_| FilesError::new(FilesErrorKind::PartialFailure).at(source.clone()))?;
        Ok(entry)
    }

    fn list_trash(&self) -> Result<Vec<TrashEntry>, FilesError> {
        self.verify_trash_storage_if_present()?;
        Ok(self.trash.iter().map(|item| item.entry.clone()).collect())
    }

    fn restore(&mut self, id: ResourceId) -> Result<FileEntry, FilesError> {
        let item = self.list_trash()?.into_iter().find(|item| item.id == id)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
        let destination = self.restore_destination(&item)?;
        self.restore_to(id, &destination)
    }

    fn restore_destination(&self, item: &TrashEntry) -> Result<Location, FilesError> {
        let original = &item.original_location;
        let parent = original.parent()
            .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
        match self.public_directory(&parent) {
            Ok(_) => Ok(original.clone()),
            Err(error) if error.kind == FilesErrorKind::NotFound => {
                let name = original.file_name()
                    .ok_or_else(|| FilesError::new(FilesErrorKind::InvalidLocation))?;
                Location::parse(name)
            }
            Err(error) => Err(error),
        }
    }

    fn restore_to(&mut self, id: ResourceId, destination: &Location) -> Result<FileEntry, FilesError> {
        self.verify_trash_storage_if_present()?;
        let index = self
            .trash
            .iter()
            .position(|item| item.entry.id == id)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
        let item = self.trash[index].clone();
        let trash_dir = self
            .trash_dir
            .as_ref()
            .ok_or_else(|| FilesError::new(FilesErrorKind::ProviderUnavailable))?;
        let trash_metadata = trash_dir
            .symlink_metadata(&item.trash_name)
            .map_err(map_io_error)?;
        self.ensure_path_mount(trash_dir, &item.trash_name, &trash_metadata)?;
        if trash_metadata.is_dir() {
            self.validate_tree_mounts(trash_dir, &item.trash_name)?;
        }
        let (destination_parent, destination_name) = self.checked_restore_destination(destination)?;
        rename_without_replace(trash_dir, &item.trash_name, &destination_parent, &destination_name)
            .map_err(|error| map_destination_error(error).at(destination.clone()))?;
        self.trash.remove(index);
        sync_directory_change(&destination_parent, destination)?;
        sync_directory_change(trash_dir, destination)?;
        self.persist_index(&self.trash).map_err(|_| {
            FilesError::new(FilesErrorKind::PartialFailure).at(destination.clone())
        })?;
        self.metadata(destination)
    }

    fn permanently_delete(&mut self, id: ResourceId) -> Result<(), FilesError> {
        self.verify_trash_storage_if_present()?;
        let index = self
            .trash
            .iter()
            .position(|item| item.entry.id == id)
            .ok_or_else(|| FilesError::new(FilesErrorKind::NotFound))?;
        let item = self.trash[index].clone();
        let trash_dir = self
            .trash_dir
            .as_ref()
            .ok_or_else(|| FilesError::new(FilesErrorKind::ProviderUnavailable))?;
        self.validate_tree_mounts(trash_dir, &item.trash_name)?;
        self.remove_tree(trash_dir, &item.trash_name)?;
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
        if location.is_root() || location.file_name().is_some_and(is_internal_component)
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

fn rename_without_replace(
    source_parent: &Dir,
    source_name: &str,
    destination_parent: &Dir,
    destination_name: &str,
) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        rustix::fs::renameat_with(
            source_parent,
            source_name,
            destination_parent,
            destination_name,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(std::io::Error::from)
    }
    #[cfg(not(target_os = "linux"))]
    {
        source_parent.rename(source_name, destination_parent, destination_name)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod linux_rename_tests {
    use super::*;
    use crate::tests::TempSandbox;

    #[test]
    fn rename_refuses_a_destination_created_after_the_absence_check() {
        let temp = TempSandbox::new();
        fs::write(temp.path().join("source"), b"source").unwrap();
        let parent = Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        assert_eq!(
            parent.symlink_metadata("target").unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        fs::write(temp.path().join("target"), b"created after check").unwrap();

        let error = rename_without_replace(&parent, "source", &parent, "target").unwrap_err();
        assert_eq!(map_destination_error(error).kind, FilesErrorKind::Conflict);
        assert_eq!(fs::read(temp.path().join("source")).unwrap(), b"source");
        assert_eq!(
            fs::read(temp.path().join("target")).unwrap(),
            b"created after check"
        );
    }

    #[test]
    fn failed_parent_fsync_reports_partial_failure_at_the_changed_location() {
        let temp = TempSandbox::new();
        let parent = Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        parent.create_dir("new-folder").unwrap();
        let location = Location::parse("new-folder").unwrap();
        let error = sync_directory_change_with(&parent, &location, |_| {
            Err(std::io::Error::other("injected fsync failure"))
        })
        .unwrap_err();
        assert_eq!(error.kind, FilesErrorKind::PartialFailure);
        assert_eq!(error.location, Some(location));
        assert!(temp.path().join("new-folder").is_dir());
    }
}

fn sync_directory_change(directory: &Dir, location: &Location) -> Result<(), FilesError> {
    #[cfg(target_os = "linux")]
    {
        sync_directory_change_with(directory, location, File::sync_all)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (directory, location);
        Ok(())
    }
}

#[cfg(target_os = "linux")]
fn sync_directory_change_with(
    directory: &Dir,
    location: &Location,
    sync: impl FnOnce(&File) -> std::io::Result<()>,
) -> Result<(), FilesError> {
    open_directory_relative(directory, ".")
        .and_then(|opened| sync(&opened.into_std_file()).map_err(map_io_error))
        .map_err(|_| FilesError::new(FilesErrorKind::PartialFailure).at(location.clone()))
}

fn is_internal_component(value: &str) -> bool {
    value.eq_ignore_ascii_case(INTERNAL_DIR)
}

#[cfg(unix)]
pub(crate) fn ensure_close_on_exec(root: &File) -> Result<(), FilesError> {
    let flags = rustix::io::fcntl_getfd(root).map_err(map_rustix_error)?;
    if !flags.contains(rustix::io::FdFlags::CLOEXEC) {
        rustix::io::fcntl_setfd(root, flags | rustix::io::FdFlags::CLOEXEC)
            .map_err(map_rustix_error)?;
    }
    Ok(())
}

fn ensure_mount_metadata(
    root_device: Option<u64>,
    metadata: &cap_std::fs::Metadata,
) -> Result<(), FilesError> {
    #[cfg(unix)]
    if root_device.is_some_and(|device| metadata.dev() != device) {
        return Err(FilesError::new(FilesErrorKind::SandboxEscape));
    }
    #[cfg(not(unix))]
    let _ = (root_device, metadata);
    Ok(())
}

fn ensure_mount_path(
    root_device: Option<u64>,
    root_mount_id: Option<u64>,
    parent: &Dir,
    name: &str,
    metadata: &cap_std::fs::Metadata,
) -> Result<(), FilesError> {
    ensure_mount_metadata(root_device, metadata)?;
    #[cfg(target_os = "linux")]
    if Some(linux_mount_id_at(parent, name)?) != root_mount_id {
        return Err(FilesError::new(FilesErrorKind::SandboxEscape));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (root_mount_id, parent, name);
    Ok(())
}

fn ensure_mount_handle(
    root_device: Option<u64>,
    root_mount_id: Option<u64>,
    directory: &Dir,
) -> Result<(), FilesError> {
    let metadata = directory.dir_metadata().map_err(map_io_error)?;
    ensure_mount_metadata(root_device, &metadata)?;
    #[cfg(target_os = "linux")]
    if Some(linux_mount_id_for_handle(directory)?) != root_mount_id {
        return Err(FilesError::new(FilesErrorKind::SandboxEscape));
    }
    #[cfg(not(target_os = "linux"))]
    let _ = root_mount_id;
    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_mount_id_for_handle<Fd: rustix::fd::AsFd>(fd: Fd) -> Result<u64, FilesError> {
    let stat = rustix::fs::statx(
        fd,
        "",
        rustix::fs::AtFlags::EMPTY_PATH,
        rustix::fs::StatxFlags::MNT_ID,
    )
    .map_err(map_rustix_error)?;
    if stat.stx_mask & rustix::fs::StatxFlags::MNT_ID.bits() == 0 {
        return Err(FilesError::new(FilesErrorKind::ProviderUnavailable));
    }
    Ok(stat.stx_mnt_id)
}

#[cfg(target_os = "linux")]
fn linux_mount_id_at(parent: &Dir, name: &str) -> Result<u64, FilesError> {
    let stat = rustix::fs::statx(
        parent,
        name,
        rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        rustix::fs::StatxFlags::MNT_ID,
    )
    .map_err(map_rustix_error)?;
    if stat.stx_mask & rustix::fs::StatxFlags::MNT_ID.bits() == 0 {
        return Err(FilesError::new(FilesErrorKind::ProviderUnavailable));
    }
    Ok(stat.stx_mnt_id)
}

#[cfg(target_os = "linux")]
const LINUX_FILES_RESOLVE: rustix::fs::ResolveFlags = rustix::fs::ResolveFlags::BENEATH
    .union(rustix::fs::ResolveFlags::NO_MAGICLINKS)
    .union(rustix::fs::ResolveFlags::NO_SYMLINKS)
    .union(rustix::fs::ResolveFlags::NO_XDEV);

// Fail closed on Linux when openat2 is unavailable; cap-std's fallback does
// not request the same no-symlink/no-mount resolution constraints.
#[cfg(target_os = "linux")]
fn open_linux_relative(
    parent: &Dir,
    path: &str,
    flags: rustix::fs::OFlags,
    mode: rustix::fs::Mode,
) -> Result<File, FilesError> {
    let file = rustix::fs::openat2(
        parent,
        path,
        flags | rustix::fs::OFlags::CLOEXEC,
        mode,
        LINUX_FILES_RESOLVE,
    )
    .map_err(map_linux_openat2_error)?;
    Ok(File::from(file))
}

#[cfg(target_os = "linux")]
fn map_linux_openat2_error(error: rustix::io::Errno) -> FilesError {
    let kind = match error {
        rustix::io::Errno::LOOP => FilesErrorKind::SymlinkNotAllowed,
        rustix::io::Errno::XDEV => FilesErrorKind::SandboxEscape,
        rustix::io::Errno::NOENT => FilesErrorKind::NotFound,
        rustix::io::Errno::NOTDIR => FilesErrorKind::NotDirectory,
        rustix::io::Errno::NOSYS | rustix::io::Errno::INVAL => FilesErrorKind::ProviderUnavailable,
        other => return map_rustix_error(other),
    };
    FilesError::new(kind)
}

fn open_directory_relative(parent: &Dir, path: &str) -> Result<Dir, FilesError> {
    #[cfg(target_os = "linux")]
    {
        let file = open_linux_relative(
            parent,
            path,
            rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY,
            rustix::fs::Mode::empty(),
        )?;
        Ok(Dir::from_std_file(file))
    }
    #[cfg(not(target_os = "linux"))]
    {
        parent.open_dir(path).map_err(map_io_error)
    }
}

fn open_read_file(parent: &Dir, path: &str) -> Result<CapFile, FilesError> {
    #[cfg(target_os = "linux")]
    {
        let file = open_linux_relative(
            parent,
            path,
            rustix::fs::OFlags::RDONLY,
            rustix::fs::Mode::empty(),
        )?;
        Ok(CapFile::from_std(file))
    }
    #[cfg(not(target_os = "linux"))]
    {
        parent.open(path).map_err(map_io_error)
    }
}

fn open_write_file(parent: &Dir, path: &str) -> Result<CapFile, FilesError> {
    #[cfg(target_os = "linux")]
    {
        let file = open_linux_relative(
            parent,
            path,
            rustix::fs::OFlags::WRONLY,
            rustix::fs::Mode::empty(),
        )?;
        Ok(CapFile::from_std(file))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut options = OpenOptions::new();
        options.write(true);
        parent.open_with(path, &options).map_err(map_io_error)
    }
}

fn create_new_write_file(parent: &Dir, path: &str) -> Result<CapFile, FilesError> {
    #[cfg(target_os = "linux")]
    {
        let file = open_linux_relative(
            parent,
            path,
            rustix::fs::OFlags::WRONLY | rustix::fs::OFlags::CREATE | rustix::fs::OFlags::EXCL,
            rustix::fs::Mode::from_raw_mode(0o666),
        )?;
        Ok(CapFile::from_std(file))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        parent.open_with(path, &options).map_err(map_io_error)
    }
}

#[cfg(unix)]
fn map_rustix_error(error: rustix::io::Errno) -> FilesError {
    map_io_error(std::io::Error::from(error))
}

fn open_optional_directory_relative(
    parent: &Dir,
    name: &str,
    root_device: Option<u64>,
    root_mount_id: Option<u64>,
) -> Result<Option<Dir>, FilesError> {
    let metadata = match parent.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(map_io_error(error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed));
    }
    ensure_mount_path(root_device, root_mount_id, parent, name, &metadata)?;
    let directory = open_directory_relative(parent, name)?;
    ensure_mount_handle(root_device, root_mount_id, &directory)?;
    #[cfg(unix)]
    {
        let opened_metadata = directory.dir_metadata().map_err(map_io_error)?;
        if metadata.dev() != opened_metadata.dev() || metadata.ino() != opened_metadata.ino() {
            return Err(FilesError::new(FilesErrorKind::SandboxEscape));
        }
    }
    Ok(Some(directory))
}

fn ensure_internal_directory(parent: &Dir, name: &str) -> Result<(), FilesError> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(FilesError::new(FilesErrorKind::SymlinkNotAllowed))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            parent.create_dir(name).map_err(map_io_error)?;
            sync_directory_change(parent, &Location::root())
        }
        Err(error) => Err(map_io_error(error)),
    }
}

fn load_trash_index(
    internal: &Dir,
    trash: Option<&Dir>,
    root_device: Option<u64>,
    root_mount_id: Option<u64>,
) -> Result<Vec<StoredTrash>, FilesError> {
    let mut dangling = 0;
    let mut items = read_trash_index(internal, trash, &mut dangling)?;
    let live_count = items.len();
    let Some(trash) = trash else {
        report_trash_recovery(0, dangling);
        return Ok(items);
    };
    let names: BTreeSet<_> = items.iter().map(|item| item.trash_name.clone()).collect();
    let mut ids: BTreeSet<_> = items.iter().map(|item| item.entry.id).collect();
    for entry in trash.entries().map_err(map_io_error)? {
        let entry = entry.map_err(map_io_error)?;
        let name = entry.file_name();
        let Some(name) = name.to_str().filter(|name| valid_trash_name(name)) else { continue };
        if names.contains(name) {
            continue;
        }
        let metadata = trash.symlink_metadata(name).map_err(map_io_error)?;
        let kind = if metadata.is_dir() {
            EntryKind::Folder
        } else if metadata.is_file() {
            EntryKind::File
        } else {
            continue;
        };
        ensure_mount_path(root_device, root_mount_id, trash, name, &metadata)?;
        let id = resource_id(Path::new(name), &metadata);
        if !ids.insert(id) {
            return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
        }
        items.push(StoredTrash {
            entry: TrashEntry {
                id,
                original_location: Location::parse(name)?,
                kind,
                size_bytes: metadata.is_file().then_some(metadata.len()),
                deleted_at: name.split_once('-').and_then(|(_, time)| time.parse().ok()).unwrap_or(0),
            },
            trash_name: name.to_owned(),
        });
    }
    report_trash_recovery(items.len() - live_count, dangling);
    Ok(items)
}

fn report_trash_recovery(recovered: usize, dangling: usize) {
    #[cfg(target_os = "linux")]
    {
        if recovered != 0 {
            let _ = writeln!(std::io::stderr().lock(), "NAGI_FILES_TRASH_ORPHAN_RECOVERED count={recovered}");
        }
        if dangling != 0 {
            let _ = writeln!(std::io::stderr().lock(), "NAGI_FILES_TRASH_INDEX_DANGLING count={dangling}");
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (recovered, dangling);
}

fn read_trash_index(
    internal: &Dir,
    trash: Option<&Dir>,
    dangling: &mut usize,
) -> Result<Vec<StoredTrash>, FilesError> {
    let index_metadata = match internal.symlink_metadata(INDEX_FILE) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(map_io_error(error)),
    };
    if index_metadata.file_type().is_symlink() || !index_metadata.is_file() {
        return Err(FilesError::new(FilesErrorKind::CorruptMetadata));
    }
    let mut index_file = open_read_file(internal, INDEX_FILE)?;
    let mut contents = String::new();
    index_file
        .read_to_string(&mut contents)
        .map_err(map_io_error)?;
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
        let Some(trash) = trash else {
            // A missing directory cannot contain a live trashed object. As in
            // the prior eager-empty-directory behavior, stale rows stay out
            // of the in-memory view and the manifest is not rewritten here.
            *dangling += 1;
            continue;
        };
        let metadata = match trash.symlink_metadata(&trash_name) {
            Ok(metadata) => metadata,
            // A crash after confirmed permanent deletion but before the index
            // update leaves a stale manifest row; omit the missing object.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                *dangling += 1;
                continue;
            }
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
    let mut tag_file = open_read_file(internal, TAG_INDEX_FILE)?;
    let mut contents = String::new();
    tag_file
        .read_to_string(&mut contents)
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

fn resource_id(_path: &Path, metadata: &cap_std::fs::Metadata) -> ResourceId {
    #[cfg(unix)]
    {
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

fn map_destination_error(error: std::io::Error) -> FilesError {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        return FilesError::new(FilesErrorKind::Conflict);
    }
    #[cfg(target_os = "linux")]
    if error.raw_os_error() == Some(rustix::io::Errno::XDEV.raw_os_error()) {
        return FilesError::new(FilesErrorKind::SandboxEscape);
    }
    map_io_error(error)
}

#[cfg(not(target_os = "linux"))]
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
