use nagi_files::{
    CancellationToken, EntryAvailability, EntryKind, FileEntry, FileName, FilesError,
    FilesErrorKind, FilesystemProvider, Location, ProviderAvailability, ResourceId, TrashEntry,
};
use nagi_storage_provider::{FileMetadata, StorageError, StorageProvider, StorageProviderError};

const FILES_ROOT: &[u8] = b"/files";

/// Read/list and folder-creation adapter for Files' existing provider contract.
/// Trash/restore, recursive copy/move, rename, and tag policy remain owned by
/// Files and are returned as unavailable until their VFS-specific semantics
/// are separately adopted. No Files accident or retention code is changed.
#[derive(Clone)]
pub struct FilesVfsProvider {
    storage: StorageProvider,
}

impl FilesVfsProvider {
    pub fn new(storage: StorageProvider) -> Self {
        Self { storage }
    }

    /// Creates the app root only when absent; existing non-directory data is
    /// rejected instead of replaced. Check/create/flush share one provider
    /// transaction so concurrent initializers cannot race and existing roots
    /// retry a failed durability boundary.
    pub fn initialize(&self) -> Result<(), FilesError> {
        self.storage
            .with_volume(|volume| {
                match volume.metadata_path(FILES_ROOT) {
                    Ok(metadata) if metadata.mode & 0xf000 == 0x4000 => {}
                    Ok(_) => return Err(StorageError::NotDirectory),
                    Err(StorageError::NotFound) => volume.mkdir_path(FILES_ROOT)?,
                    Err(error) => return Err(error),
                }
                volume.flush()
            })
            .map_err(map_storage_error)
    }

    fn vfs_path(location: &Location) -> Result<Vec<u8>, FilesError> {
        let path = if location.is_root() {
            FILES_ROOT.to_vec()
        } else {
            [FILES_ROOT, b"/", location.as_str().as_bytes()].concat()
        };
        if path.len() > 256 {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation).at(location.clone()));
        }
        Ok(path)
    }

    fn unsupported(&self, location: &Location) -> FilesError {
        FilesError::new(FilesErrorKind::ProviderUnavailable).at(location.clone())
    }

    fn entry(location: &Location, metadata: FileMetadata) -> Result<FileEntry, FilesError> {
        let name = FileName::parse(location.file_name().unwrap_or("Files"))
            .map_err(|_| FilesError::new(FilesErrorKind::CorruptMetadata).at(location.clone()))?;
        let kind = match metadata.mode & 0xf000 {
            0x4000 => EntryKind::Folder,
            0x8000 => EntryKind::File,
            _ => EntryKind::Unsupported,
        };
        let id = ResourceId((u128::from(metadata.inode) << 32) | u128::from(metadata.generation));
        Ok(FileEntry {
            id,
            name,
            location: location.clone(),
            kind,
            size_bytes: (kind == EntryKind::File).then_some(u64::from(metadata.size)),
            created_at: Some(i64::from(metadata.ctime)),
            modified_at: Some(i64::from(metadata.mtime)),
            tags: Vec::new(),
            availability: EntryAvailability::Available,
        })
    }
}

impl FilesystemProvider for FilesVfsProvider {
    fn availability(&self) -> ProviderAvailability {
        ProviderAvailability::Available
    }

    fn list(&self, location: &Location) -> Result<Vec<FileEntry>, FilesError> {
        let path = Self::vfs_path(location)?;
        let snapshot = self
            .storage
            .list_directory_snapshot(&path)
            .map_err(map_provider_error)?;
        let mut result = Vec::new();
        result
            .try_reserve_exact(snapshot.entries.len())
            .map_err(|_| FilesError::new(FilesErrorKind::ProviderFailure).at(location.clone()))?;
        for snapshot_entry in snapshot.entries {
            let directory_entry = snapshot_entry.directory_entry;
            let raw_name = std::str::from_utf8(directory_entry.name()).map_err(|_| {
                FilesError::new(FilesErrorKind::CorruptMetadata).at(location.clone())
            })?;
            let name = FileName::parse(raw_name).map_err(|_| {
                FilesError::new(FilesErrorKind::CorruptMetadata).at(location.clone())
            })?;
            let child_location = location.join(&name);
            result.push(Self::entry(&child_location, snapshot_entry.metadata)?);
        }
        Ok(result)
    }

    fn metadata(&self, location: &Location) -> Result<FileEntry, FilesError> {
        let path = Self::vfs_path(location)?;
        let metadata = self
            .storage
            .metadata_path(&path)
            .map_err(map_storage_error)?;
        Self::entry(location, metadata)
    }

    fn read_file(&self, location: &Location, max_bytes: usize) -> Result<Vec<u8>, FilesError> {
        let path = Self::vfs_path(location)?;
        let (_metadata, contents) = self
            .storage
            .read_file_snapshot(&path, max_bytes)
            .map_err(map_provider_error)?;
        Ok(contents)
    }

    fn create_folder(
        &mut self,
        parent: &Location,
        name: &FileName,
    ) -> Result<FileEntry, FilesError> {
        let location = parent.join(name);
        let path = Self::vfs_path(&location)?;
        self.storage.mkdir_path(&path).map_err(map_storage_error)?;
        self.storage.flush().map_err(map_storage_error)?;
        self.metadata(&location)
    }

    fn rename(&mut self, source: &Location, _name: &FileName) -> Result<FileEntry, FilesError> {
        Err(self.unsupported(source))
    }

    fn copy(
        &mut self,
        source: &Location,
        _destination: &Location,
        _name: &FileName,
        cancellation: &CancellationToken,
    ) -> Result<FileEntry, FilesError> {
        if cancellation.is_cancelled() {
            return Err(FilesError::new(FilesErrorKind::Cancelled).at(source.clone()));
        }
        Err(self.unsupported(source))
    }

    fn move_item(
        &mut self,
        source: &Location,
        _destination: &Location,
        _name: &FileName,
    ) -> Result<FileEntry, FilesError> {
        Err(self.unsupported(source))
    }

    fn trash(&mut self, source: &Location) -> Result<TrashEntry, FilesError> {
        Err(self.unsupported(source))
    }

    fn list_trash(&self) -> Result<Vec<TrashEntry>, FilesError> {
        Err(self.unsupported(&Location::root()))
    }

    fn restore(&mut self, _id: ResourceId) -> Result<FileEntry, FilesError> {
        Err(self.unsupported(&Location::root()))
    }

    fn permanently_delete(&mut self, _id: ResourceId) -> Result<(), FilesError> {
        Err(self.unsupported(&Location::root()))
    }
}

fn map_storage_error(error: StorageError) -> FilesError {
    let kind = match error {
        StorageError::NotFound => FilesErrorKind::NotFound,
        StorageError::AlreadyExists => FilesErrorKind::AlreadyExists,
        StorageError::NotDirectory => FilesErrorKind::NotDirectory,
        StorageError::IsDirectory => FilesErrorKind::IsDirectory,
        StorageError::InvalidName | StorageError::NameTooLong => FilesErrorKind::InvalidName,
        StorageError::FileTooLarge => FilesErrorKind::FileTooLarge,
        StorageError::Capacity | StorageError::DirectoryFull => FilesErrorKind::ProviderFailure,
        StorageError::Corrupt => FilesErrorKind::CorruptMetadata,
        StorageError::DirectoryNotEmpty => FilesErrorKind::Conflict,
        StorageError::BufferTooSmall | StorageError::Block | StorageError::InvalidHandle => {
            FilesErrorKind::ProviderFailure
        }
    };
    FilesError::new(kind)
}

fn map_provider_error(error: StorageProviderError) -> FilesError {
    match error {
        StorageProviderError::Vfs(error) => map_storage_error(error),
        StorageProviderError::CapacityTooSmall { .. }
        | StorageProviderError::MisalignedCapacity
        | StorageProviderError::NotBlank
        | StorageProviderError::ReadOnlyDescriptor
        | StorageProviderError::AppendDescriptor
        | StorageProviderError::AlreadyMounted
        | StorageProviderError::DeviceIo(_)
        | StorageProviderError::Allocation(_) => FilesError::new(FilesErrorKind::ProviderFailure),
    }
}
