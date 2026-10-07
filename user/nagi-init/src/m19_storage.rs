//! Crash-recoverable M19 snapshots stored in the guest User Data VFS.

use libnagi::storage::{BlockDevice, StorageError, SyscallBlockDevice, Vfs, BLOCK_SIZE};
use nagi_search::{
    GuestSnapshotBackend, SearchService, SnapshotFile, SnapshotFileStore, SnapshotSlot,
    VisibilityFilter, GUEST_FILE_BYTES,
};

const STORE_ROOT: &[u8] = b"/var/lib/nagi-search";
#[cfg(feature = "m19-search")]
const SEMANTIC_STORE_ROOT: &[u8] = b"/var/lib/nagi-search-semantic";

const _: [(); BLOCK_SIZE] = [(); GUEST_FILE_BYTES];

pub(super) struct VfsSnapshotFiles<D: BlockDevice> {
    volume: Vfs<D>,
    namespace: SnapshotNamespace,
}

#[derive(Clone, Copy)]
pub(super) enum SnapshotNamespace {
    Metadata,
    #[cfg(feature = "m19-search")]
    Semantic,
}

impl<D: BlockDevice> VfsSnapshotFiles<D> {
    pub(super) fn new(
        mut volume: Vfs<D>,
        namespace: SnapshotNamespace,
    ) -> Result<Self, nagi_search::BackendError> {
        let root = match namespace {
            SnapshotNamespace::Metadata => STORE_ROOT,
            #[cfg(feature = "m19-search")]
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
            #[cfg(feature = "m19-search")]
            (SnapshotNamespace::Semantic, SnapshotSlot::A, SnapshotFile::Manifest) => {
                Some(b"/var/lib/nagi-search-semantic/am")
            }
            #[cfg(feature = "m19-search")]
            (SnapshotNamespace::Semantic, SnapshotSlot::A, SnapshotFile::Chunk(0)) => {
                Some(b"/var/lib/nagi-search-semantic/a0")
            }
            #[cfg(feature = "m19-search")]
            (SnapshotNamespace::Semantic, SnapshotSlot::A, SnapshotFile::Chunk(1)) => {
                Some(b"/var/lib/nagi-search-semantic/a1")
            }
            #[cfg(feature = "m19-search")]
            (SnapshotNamespace::Semantic, SnapshotSlot::A, SnapshotFile::Chunk(2)) => {
                Some(b"/var/lib/nagi-search-semantic/a2")
            }
            #[cfg(feature = "m19-search")]
            (SnapshotNamespace::Semantic, SnapshotSlot::A, SnapshotFile::Chunk(3)) => {
                Some(b"/var/lib/nagi-search-semantic/a3")
            }
            #[cfg(feature = "m19-search")]
            (SnapshotNamespace::Semantic, SnapshotSlot::B, SnapshotFile::Manifest) => {
                Some(b"/var/lib/nagi-search-semantic/bm")
            }
            #[cfg(feature = "m19-search")]
            (SnapshotNamespace::Semantic, SnapshotSlot::B, SnapshotFile::Chunk(0)) => {
                Some(b"/var/lib/nagi-search-semantic/b0")
            }
            #[cfg(feature = "m19-search")]
            (SnapshotNamespace::Semantic, SnapshotSlot::B, SnapshotFile::Chunk(1)) => {
                Some(b"/var/lib/nagi-search-semantic/b1")
            }
            #[cfg(feature = "m19-search")]
            (SnapshotNamespace::Semantic, SnapshotSlot::B, SnapshotFile::Chunk(2)) => {
                Some(b"/var/lib/nagi-search-semantic/b2")
            }
            #[cfg(feature = "m19-search")]
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

pub(super) fn open_search_with_visibility<V: VisibilityFilter>(
    block_capability: u64,
    visibility: V,
) -> Result<
    SearchService<GuestSnapshotBackend<VfsSnapshotFiles<SyscallBlockDevice>>, V>,
    nagi_search::MetadataStoreError,
> {
    let volume = Vfs::mount_or_format(SyscallBlockDevice::new(block_capability))
        .map(|(volume, _)| volume)
        .map_err(|_| nagi_search::MetadataStoreError::Backend(nagi_search::BackendError::Io))?;
    let files = VfsSnapshotFiles::new(volume, SnapshotNamespace::Metadata)
        .map_err(nagi_search::MetadataStoreError::Backend)?;
    SearchService::open(GuestSnapshotBackend::new(files), visibility)
}
