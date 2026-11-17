//! Shared, caller-granted access to one Nagi VFS volume.
//!
//! Authentication, peer credentials, profile selection, and Storage grants
//! belong to the caller (the Supervisor/task41 boundary). This crate accepts
//! an already-authorized owned file descriptor and never resolves a product
//! path or chooses a profile. Notes, Files, and Settings adapters can share a
//! clone of one `StorageProvider`; all operations serialize on the same VFS
//! lock and `flush` is the durable acknowledgement boundary exposed by Nagi's
//! current block-device contract.

use libnagi::storage::{BlockDevice, ReadOnlyBlockDevice, Vfs};
use std::fmt;
use std::fs::File;
#[cfg(any(test, feature = "dev-fixtures"))]
use std::fs::OpenOptions;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::fs::FileExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
#[cfg(any(test, feature = "dev-fixtures"))]
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::{collections::HashSet, hash::Hash};

#[cfg(any(test, feature = "test-hooks"))]
thread_local! {
    static FLUSH_FAILURE_COUNTDOWN: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub fn fail_flush_after_for_test(successful_flushes_before_failure: usize) {
    FLUSH_FAILURE_COUNTDOWN.with(|countdown| {
        countdown.set(Some(successful_flushes_before_failure));
    });
}

#[cfg(any(test, feature = "test-hooks"))]
fn injected_flush_failure() -> bool {
    FLUSH_FAILURE_COUNTDOWN.with(|countdown| match countdown.get() {
        Some(0) => {
            countdown.set(None);
            true
        }
        Some(remaining) => {
            countdown.set(Some(remaining - 1));
            false
        }
        None => false,
    })
}

pub use libnagi::storage::{
    DirectoryEntry, FileHandle, FileMetadata, StorageError, BLOCK_SIZE, MAX_DIRECTORY_ENTRIES,
    MAX_FILE_SIZE, MAX_PATH_LENGTH, SECTOR_SIZE,
};

pub const MIN_VFS_BYTES: u64 = 8 * 1024 * 1024;
const DIRECTORY_RECORD_MIN_BYTES: usize = 8;
const MAX_DIRECTORY_LIST_ENTRIES: usize = BLOCK_SIZE / DIRECTORY_RECORD_MIN_BYTES;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectorySnapshotEntry {
    pub directory_entry: DirectoryEntry,
    pub metadata: FileMetadata,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectorySnapshot {
    pub directory_metadata: FileMetadata,
    pub entries: Vec<DirectorySnapshotEntry>,
}

#[derive(Debug)]
pub enum StorageProviderError {
    DeviceIo(String),
    Vfs(StorageError),
    Allocation(String),
    CapacityTooSmall { actual: u64, minimum: u64 },
    MisalignedCapacity,
    NotBlank,
    ReadOnlyDescriptor,
    AppendDescriptor,
    AlreadyMounted,
}

impl fmt::Display for StorageProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceIo(message) => write!(f, "storage device I/O failed: {message}"),
            Self::Vfs(error) => write!(f, "Nagi VFS operation failed: {error:?}"),
            Self::Allocation(message) => write!(f, "storage allocation failed: {message}"),
            Self::CapacityTooSmall { actual, minimum } => {
                write!(f, "storage capacity {actual} is below minimum {minimum}")
            }
            Self::MisalignedCapacity => f.write_str("storage capacity is not sector-aligned"),
            Self::NotBlank => f.write_str("refusing to format a nonblank storage device"),
            Self::ReadOnlyDescriptor => f.write_str(
                "storage descriptor must be read-write because VFS reads update inode atime",
            ),
            Self::AppendDescriptor => {
                f.write_str("storage descriptor must not use O_APPEND with offset-based writes")
            }
            Self::AlreadyMounted => {
                f.write_str("this storage volume already has a mounted provider owner")
            }
        }
    }
}

impl std::error::Error for StorageProviderError {}

impl From<StorageError> for StorageProviderError {
    fn from(error: StorageError) -> Self {
        Self::Vfs(error)
    }
}

/// File-backed implementation of Nagi's public block-device contract.
/// `from_authorized_fd` is the task41 handoff: its caller must already have
/// checked SO_PEERCRED and obtained the profile-scoped Storage grant.
pub struct LinuxBlockDevice {
    file: File,
    capacity_bytes: u64,
}

impl LinuxBlockDevice {
    /// Opens a caller-selected path for development tools and isolated tests.
    /// Production service callers should pass an authorized `OwnedFd` instead.
    #[cfg(any(test, feature = "dev-fixtures"))]
    pub fn open(path: &Path) -> Result<Self, StorageProviderError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| StorageProviderError::DeviceIo(format!("open device: {error}")))?;
        Self::from_file_inner(file)
    }

    #[cfg(any(test, feature = "dev-fixtures"))]
    pub fn from_file(file: File) -> Result<Self, StorageProviderError> {
        Self::from_file_inner(file)
    }

    fn from_file_inner(file: File) -> Result<Self, StorageProviderError> {
        ensure_read_write(&file)?;
        let capacity_bytes = capacity_for_file(&file)
            .map_err(|error| StorageProviderError::DeviceIo(format!("query capacity: {error}")))?;
        Ok(Self {
            file,
            capacity_bytes,
        })
    }

    pub fn capacity_bytes(&self) -> u64 {
        self.capacity_bytes
    }

    fn is_blank(&self) -> io::Result<bool> {
        let mut buffer = [0u8; 64 * 1024];
        let mut offset = 0u64;
        while offset < self.capacity_bytes {
            let remaining = usize::try_from(self.capacity_bytes - offset).unwrap_or(usize::MAX);
            let length = buffer.len().min(remaining);
            let read = self.file.read_at(&mut buffer[..length], offset)?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "block device returned a short read while checking for a blank volume",
                ));
            }
            if buffer[..read].iter().any(|byte| *byte != 0) {
                return Ok(false);
            }
            offset = offset
                .checked_add(read as u64)
                .ok_or_else(|| io::Error::other("block device offset overflow"))?;
        }
        Ok(true)
    }

    fn checked_sector_offset(&self, sector: u64) -> Result<u64, StorageError> {
        let offset = sector
            .checked_mul(SECTOR_SIZE as u64)
            .ok_or(StorageError::Block)?;
        let end = offset
            .checked_add(SECTOR_SIZE as u64)
            .ok_or(StorageError::Block)?;
        if end > self.capacity_bytes {
            return Err(StorageError::Block);
        }
        Ok(offset)
    }
}

fn ensure_read_write(file: &File) -> Result<(), StorageProviderError> {
    use std::os::raw::c_int;

    const F_GETFL: c_int = 3;
    const O_ACCMODE: c_int = 3;
    const O_RDWR: c_int = 2;
    #[cfg(target_os = "linux")]
    const O_APPEND: c_int = 0x400;
    extern "C" {
        fn fcntl(fd: c_int, command: c_int, ...) -> c_int;
    }

    // SAFETY: F_GETFL reads the descriptor flags and takes no third argument.
    let flags = unsafe { fcntl(file.as_raw_fd(), F_GETFL) };
    if flags < 0 {
        return Err(StorageProviderError::DeviceIo(format!(
            "inspect storage descriptor access mode: {}",
            io::Error::last_os_error()
        )));
    }
    if flags & O_ACCMODE != O_RDWR {
        return Err(StorageProviderError::ReadOnlyDescriptor);
    }
    #[cfg(target_os = "linux")]
    if flags & O_APPEND != 0 {
        return Err(StorageProviderError::AppendDescriptor);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum VolumeIdentity {
    BlockDevice { device: u64 },
    File { device: u64, inode: u64 },
}

fn volume_identity(file: &File) -> Result<VolumeIdentity, StorageProviderError> {
    let metadata = file
        .metadata()
        .map_err(|error| StorageProviderError::DeviceIo(format!("identify volume: {error}")))?;
    if metadata.file_type().is_block_device() {
        Ok(VolumeIdentity::BlockDevice {
            device: metadata.rdev(),
        })
    } else {
        Ok(VolumeIdentity::File {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
}

fn active_mounts() -> &'static Mutex<HashSet<VolumeIdentity>> {
    static ACTIVE_MOUNTS: OnceLock<Mutex<HashSet<VolumeIdentity>>> = OnceLock::new();
    ACTIVE_MOUNTS.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Process-local duplicate-mount prevention plus a Linux advisory lock for
/// other provider processes opening the same file/device-node inode. The
/// in-process key uses block-device rdev, but flock does not unify distinct
/// device-node inodes with the same rdev. The service boundary must therefore
/// keep one broker owner per rdev across processes.
struct MountLease {
    identity: VolumeIdentity,
}

impl MountLease {
    fn acquire(file: &File) -> Result<Self, StorageProviderError> {
        let identity = volume_identity(file)?;
        let mut active = active_mounts().lock().map_err(|_| {
            StorageProviderError::DeviceIo("mount registry lock was poisoned".to_owned())
        })?;
        if !active.insert(identity) {
            return Err(StorageProviderError::AlreadyMounted);
        }

        if let Err(error) = lock_volume_file(file) {
            active.remove(&identity);
            return Err(if error.kind() == io::ErrorKind::WouldBlock {
                StorageProviderError::AlreadyMounted
            } else {
                StorageProviderError::DeviceIo(format!("lock storage volume: {error}"))
            });
        }
        drop(active);
        Ok(Self { identity })
    }
}

impl Drop for MountLease {
    fn drop(&mut self) {
        if let Ok(mut active) = active_mounts().lock() {
            active.remove(&self.identity);
        }
    }
}

#[cfg(target_os = "linux")]
fn lock_volume_file(file: &File) -> io::Result<()> {
    use std::os::raw::c_int;

    const LOCK_EX: c_int = 2;
    const LOCK_NB: c_int = 4;

    extern "C" {
        fn flock(fd: c_int, operation: c_int) -> c_int;
    }

    // SAFETY: flock only uses the descriptor and scalar operation flags.
    if unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn lock_volume_file(_file: &File) -> io::Result<()> {
    // Non-Linux builds are developer-only. The production Linux target uses
    // flock in addition to the in-process identity registry.
    Ok(())
}

#[cfg(target_os = "linux")]
fn capacity_for_file(file: &File) -> io::Result<u64> {
    use std::os::fd::AsRawFd;
    use std::os::raw::{c_int, c_ulong};
    use std::os::unix::fs::FileTypeExt;

    let metadata = file.metadata()?;
    if !metadata.file_type().is_block_device() {
        return Ok(metadata.len());
    }

    // Linux BLKGETSIZE64: _IOR(0x12, 114, size_of::<u64>()).
    const BLKGETSIZE64: c_ulong = 0x8008_1272;
    extern "C" {
        fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    }

    let mut capacity = 0u64;
    // SAFETY: BLKGETSIZE64 writes one u64 to the pointer provided by the caller.
    let status = unsafe { ioctl(file.as_raw_fd(), BLKGETSIZE64, &mut capacity) };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(capacity)
}

#[cfg(not(target_os = "linux"))]
fn capacity_for_file(file: &File) -> io::Result<u64> {
    file.metadata().map(|metadata| metadata.len())
}

impl ReadOnlyBlockDevice for LinuxBlockDevice {
    fn read_sector(
        &mut self,
        sector: u64,
        destination: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        let base = self.checked_sector_offset(sector)?;
        let mut offset = 0;
        while offset < destination.len() {
            let file_offset = base.checked_add(offset as u64).ok_or(StorageError::Block)?;
            let read = self
                .file
                .read_at(&mut destination[offset..], file_offset)
                .map_err(|_| StorageError::Block)?;
            if read == 0 {
                return Err(StorageError::Block);
            }
            offset += read;
        }
        Ok(())
    }
}

impl BlockDevice for LinuxBlockDevice {
    fn write_sector(
        &mut self,
        sector: u64,
        source: &[u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        let base = self.checked_sector_offset(sector)?;
        let mut offset = 0;
        while offset < source.len() {
            let file_offset = base.checked_add(offset as u64).ok_or(StorageError::Block)?;
            let written = self
                .file
                .write_at(&source[offset..], file_offset)
                .map_err(|_| StorageError::Block)?;
            if written == 0 {
                return Err(StorageError::Block);
            }
            offset += written;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<(), StorageError> {
        #[cfg(any(test, feature = "test-hooks"))]
        if injected_flush_failure() {
            return Err(StorageError::Block);
        }
        self.file.sync_all().map_err(|_| StorageError::Block)
    }
}

/// Internal shared state. Field order is intentional: the VFS closes its
/// device descriptor before `MountLease` releases the process-local key.
struct MountedVfs {
    volume: Mutex<Vfs<LinuxBlockDevice>>,
    _mount_lease: MountLease,
}

/// Cloneable consumer capability for one mounted profile volume.
#[derive(Clone)]
pub struct StorageProvider {
    inner: Arc<MountedVfs>,
}

/// Compatibility name for existing Notes and Desktop consumers. Mounting is
/// only available through `StorageProviderOwner`; consumers get clones from
/// that owner, so Files, Settings, and Notes use one VFS state and lock.
pub type SharedVfs = StorageProvider;

/// The unique in-process mount owner. Keep this object at the volume service
/// boundary and distribute `consumer_handle()` clones to authorized clients.
/// On Linux, an advisory `flock` rejects a second provider using an independent
/// open file description for the same opened file/device-node inode. It does
/// not detect a duplicated/inherited open file description or distinct block
/// device-node inodes with the same rdev. The
/// caller must transfer one descriptor to this owner and must not pass a dup or
/// inherited descriptor to another process; `flock` treats shared descriptors
/// as one lock owner. Consumers receive this owner's handle or use its broker.
pub struct StorageProviderOwner {
    provider: StorageProvider,
}

impl StorageProviderOwner {
    /// Mounts an already-authorized descriptor and never formats it.
    ///
    /// The caller transfers this descriptor exclusively to the single volume
    /// owner. Do not duplicate or inherit it into another process for mounting.
    pub fn mount_authorized_fd(fd: OwnedFd) -> Result<Self, StorageProviderError> {
        Self::mount_existing_file_inner(File::from(fd))
    }

    #[cfg(any(test, feature = "dev-fixtures"))]
    pub fn mount_existing_file(file: File) -> Result<Self, StorageProviderError> {
        Self::mount_existing_file_inner(file)
    }

    fn mount_existing_file_inner(file: File) -> Result<Self, StorageProviderError> {
        let device = LinuxBlockDevice::from_file_inner(file)?;
        validate_device_capacity(&device)?;
        let lease = MountLease::acquire(&device.file)?;
        Self::mount_existing_device(device, lease)
    }

    /// Explicit provisioning entry point. It only formats a zero-filled disk;
    /// callers with an existing profile volume must use `mount_existing_file`.
    #[cfg(any(test, feature = "dev-fixtures"))]
    pub fn format_blank_file(file: File) -> Result<Self, StorageProviderError> {
        Self::format_blank_file_inner(file)
    }

    /// Formats an already-authorized descriptor only when its volume is blank.
    pub fn format_blank_authorized_fd(fd: OwnedFd) -> Result<Self, StorageProviderError> {
        Self::format_blank_file_inner(File::from(fd))
    }

    fn format_blank_file_inner(file: File) -> Result<Self, StorageProviderError> {
        let device = LinuxBlockDevice::from_file_inner(file)?;
        validate_device_capacity(&device)?;
        let lease = MountLease::acquire(&device.file)?;
        let blank = match device.is_blank() {
            Ok(blank) => blank,
            Err(error) => {
                drop(device);
                drop(lease);
                return Err(StorageProviderError::DeviceIo(format!(
                    "inspect blank volume: {error}"
                )));
            }
        };
        if !blank {
            drop(device);
            drop(lease);
            return Err(StorageProviderError::NotBlank);
        }
        Self::format_device(device, lease)
    }

    /// Caller-selected path constructor retained for developer fixtures only.
    #[cfg(any(test, feature = "dev-fixtures"))]
    pub fn mount_existing(path: &Path) -> Result<Self, StorageProviderError> {
        let device = LinuxBlockDevice::open(path)?;
        Self::mount_existing_device_from_unleased(device)
    }

    /// Caller-selected path constructor retained for developer fixtures only.
    #[cfg(any(test, feature = "dev-fixtures"))]
    pub fn mount_or_format(path: &Path) -> Result<(Self, bool), StorageProviderError> {
        let device = LinuxBlockDevice::open(path)?;
        validate_device_capacity(&device)?;
        let lease = MountLease::acquire(&device.file)?;
        let blank = match device.is_blank() {
            Ok(blank) => blank,
            Err(error) => {
                drop(device);
                drop(lease);
                return Err(StorageProviderError::DeviceIo(format!(
                    "inspect blank volume: {error}"
                )));
            }
        };
        if blank {
            Ok((Self::format_device(device, lease)?, true))
        } else {
            Ok((Self::mount_existing_device(device, lease)?, false))
        }
    }

    /// Returns a clone of the already-mounted provider capability. No second
    /// mount or independent VFS cache is created.
    pub fn consumer_handle(&self) -> StorageProvider {
        self.provider.clone()
    }

    #[cfg(any(test, feature = "dev-fixtures"))]
    fn mount_existing_device_from_unleased(
        device: LinuxBlockDevice,
    ) -> Result<Self, StorageProviderError> {
        validate_device_capacity(&device)?;
        let lease = MountLease::acquire(&device.file)?;
        Self::mount_existing_device(device, lease)
    }

    fn mount_existing_device(
        mut device: LinuxBlockDevice,
        lease: MountLease,
    ) -> Result<Self, StorageProviderError> {
        validate_device_capacity(&device)?;
        if let Err(error) = Vfs::<LinuxBlockDevice>::check_existing(&mut device) {
            drop(device);
            drop(lease);
            return Err(error.into());
        }
        let volume = match Vfs::mount_existing(device) {
            Ok(volume) => volume,
            Err(error) => {
                drop(lease);
                return Err(error.into());
            }
        };
        Ok(StorageProvider::from_volume(volume, lease))
    }

    fn format_device(
        device: LinuxBlockDevice,
        lease: MountLease,
    ) -> Result<Self, StorageProviderError> {
        let (mut volume, formatted) = Vfs::mount_or_format(device)?;
        if !formatted {
            drop(volume);
            drop(lease);
            return Err(StorageProviderError::NotBlank);
        }
        if let Err(error) = volume.flush() {
            drop(volume);
            drop(lease);
            return Err(error.into());
        }
        Ok(StorageProvider::from_volume(volume, lease))
    }
}

impl StorageProvider {
    fn from_volume(volume: Vfs<LinuxBlockDevice>, lease: MountLease) -> StorageProviderOwner {
        StorageProviderOwner {
            provider: Self {
                inner: Arc::new(MountedVfs {
                    volume: Mutex::new(volume),
                    _mount_lease: lease,
                }),
            },
        }
    }

    /// Runs one serialized adapter transaction. Adapter error types implement
    /// `From<StorageError>` so lock poisoning is propagated as an I/O failure.
    pub fn with_volume<T, E>(
        &self,
        operation: impl FnOnce(&mut Vfs<LinuxBlockDevice>) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<StorageError>,
    {
        let mut volume = self
            .inner
            .volume
            .lock()
            .map_err(|_| StorageError::Block.into())?;
        operation(&mut volume)
    }

    pub fn create_path(&self, path: &[u8]) -> Result<FileHandle, StorageError> {
        self.with_volume(|volume| volume.create_path(path))
    }

    pub fn mkdir_path(&self, path: &[u8]) -> Result<(), StorageError> {
        self.with_volume(|volume| volume.mkdir_path(path))
    }

    pub fn open_path(&self, path: &[u8]) -> Result<FileHandle, StorageError> {
        self.with_volume(|volume| volume.open_path(path))
    }

    pub fn metadata_path(&self, path: &[u8]) -> Result<FileMetadata, StorageError> {
        self.with_volume(|volume| volume.metadata_path(path))
    }

    /// Reads a regular file's metadata and bytes while holding the provider's
    /// VFS lock once. This gives a logical read a single serialization point
    /// against writes and prevents length/data mismatches between calls.
    pub fn read_file_snapshot(
        &self,
        path: &[u8],
        max_bytes: usize,
    ) -> Result<(FileMetadata, Vec<u8>), StorageProviderError> {
        self.with_volume(|volume| {
            let metadata = volume.metadata_path(path)?;
            if metadata.mode & 0xf000 != 0x8000 {
                return Err(StorageError::IsDirectory.into());
            }
            let length = usize::try_from(metadata.size).map_err(|_| StorageError::FileTooLarge)?;
            if length > max_bytes {
                return Err(StorageError::FileTooLarge.into());
            }
            let handle = volume.open_path(path)?;
            let mut bytes = Vec::new();
            bytes
                .try_reserve_exact(length)
                .map_err(|_| StorageProviderError::Allocation("file snapshot".to_owned()))?;
            bytes.resize(length, 0);
            let read = volume.read(handle, &mut bytes)?;
            if read != length {
                return Err(StorageError::Block.into());
            }
            Ok((metadata, bytes))
        })
    }

    /// Returns the directory's entries and every child's metadata under one
    /// lock, so Files cannot combine a directory listing from one revision
    /// with metadata read after a concurrent rename/create/remove sequence.
    pub fn list_directory_snapshot(
        &self,
        path: &[u8],
    ) -> Result<DirectorySnapshot, StorageProviderError> {
        self.with_volume(|volume| {
            let directory_metadata = volume.metadata_path(path)?;
            if directory_metadata.mode & 0xf000 != 0x4000 {
                return Err(StorageError::NotDirectory.into());
            }
            let directory_entries = list_directory_entries_in_volume(volume, path)?;
            let mut entries = Vec::new();
            entries
                .try_reserve_exact(directory_entries.len())
                .map_err(|error| StorageProviderError::Allocation(error.to_string()))?;
            for directory_entry in directory_entries {
                let name = directory_entry.name();
                let separator_length = usize::from(!path.ends_with(b"/"));
                let path_length = path
                    .len()
                    .checked_add(separator_length)
                    .and_then(|length| length.checked_add(name.len()))
                    .filter(|length| *length <= MAX_PATH_LENGTH)
                    .ok_or(StorageError::NameTooLong)?;
                let mut child_path = Vec::new();
                child_path
                    .try_reserve_exact(path_length)
                    .map_err(|error| StorageProviderError::Allocation(error.to_string()))?;
                child_path.extend_from_slice(path);
                if separator_length != 0 {
                    child_path.push(b'/');
                }
                child_path.extend_from_slice(name);
                let metadata = volume.metadata_path(&child_path)?;
                entries.push(DirectorySnapshotEntry {
                    directory_entry,
                    metadata,
                });
            }
            Ok(DirectorySnapshot {
                directory_metadata,
                entries,
            })
        })
    }

    pub fn remove_path(&self, path: &[u8]) -> Result<(), StorageError> {
        self.with_volume(|volume| volume.remove_path(path))
    }

    pub fn rmdir_path(&self, path: &[u8]) -> Result<(), StorageError> {
        self.with_volume(|volume| volume.rmdir_path(path))
    }

    pub fn list_directory_entries(
        &self,
        path: &[u8],
    ) -> Result<Vec<DirectoryEntry>, StorageProviderError> {
        self.with_volume(|volume| list_directory_entries_in_volume(volume, path))
    }

    pub fn read(&self, handle: FileHandle, destination: &mut [u8]) -> Result<usize, StorageError> {
        self.with_volume(|volume| volume.read(handle, destination))
    }

    pub fn write(&self, handle: FileHandle, contents: &[u8]) -> Result<(), StorageError> {
        self.with_volume(|volume| volume.write(handle, contents))
    }

    pub fn truncate(&self, handle: FileHandle, length: usize) -> Result<(), StorageError> {
        self.with_volume(|volume| volume.truncate(handle, length))
    }

    pub fn flush(&self) -> Result<(), StorageError> {
        self.with_volume(Vfs::flush)
    }
}

fn validate_device_capacity(device: &LinuxBlockDevice) -> Result<(), StorageProviderError> {
    if device.capacity_bytes() < MIN_VFS_BYTES {
        return Err(StorageProviderError::CapacityTooSmall {
            actual: device.capacity_bytes(),
            minimum: MIN_VFS_BYTES,
        });
    }
    if device.capacity_bytes() % SECTOR_SIZE as u64 != 0 {
        return Err(StorageProviderError::MisalignedCapacity);
    }
    Ok(())
}

fn list_directory_entries_in_volume(
    volume: &mut Vfs<LinuxBlockDevice>,
    path: &[u8],
) -> Result<Vec<DirectoryEntry>, StorageProviderError> {
    let mut capacity = MAX_DIRECTORY_ENTRIES.min(MAX_DIRECTORY_LIST_ENTRIES);
    loop {
        let mut entries = Vec::new();
        entries
            .try_reserve_exact(capacity)
            .map_err(|error| StorageProviderError::Allocation(error.to_string()))?;
        entries.resize(capacity, DirectoryEntry::empty());
        match volume.list_directory_path(path, &mut entries) {
            Ok(count) => {
                entries.truncate(count);
                return Ok(entries);
            }
            Err(StorageError::BufferTooSmall) if capacity < MAX_DIRECTORY_LIST_ENTRIES => {
                let next = capacity.saturating_mul(2).min(MAX_DIRECTORY_LIST_ENTRIES);
                if next <= capacity {
                    return Err(StorageError::BufferTooSmall.into());
                }
                capacity = next;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::Command;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "nagi-storage-provider-{name}-{}-{}.img",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    fn subprocess_mount_probe_rejects_an_independent_second_owner() {
        let Ok(path) = std::env::var("NAGI_PROVIDER_MOUNT_PROBE_PATH") else {
            return;
        };
        assert!(matches!(
            StorageProviderOwner::mount_existing(Path::new(&path)),
            Err(StorageProviderError::AlreadyMounted)
        ));
        println!("INDEPENDENT_OPEN_SECOND_PROCESS_MOUNT=REJECTED");
    }

    #[test]
    fn block_device_enforces_capacity_and_short_read_bounds() {
        let path = temp_path("bounds");
        let original = [0xa5; SECTOR_SIZE];
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(&original).unwrap();
        file.sync_all().unwrap();
        drop(file);

        let mut device = LinuxBlockDevice::open(&path).unwrap();
        assert_eq!(device.capacity_bytes(), SECTOR_SIZE as u64);
        assert_eq!(device.read_sector(0, &mut [0; SECTOR_SIZE]), Ok(()));
        assert_eq!(
            device.read_sector(1, &mut [0; SECTOR_SIZE]),
            Err(StorageError::Block)
        );
        assert_eq!(
            device.write_sector(1, &[0x5a; SECTOR_SIZE]),
            Err(StorageError::Block)
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        drop(device);
        std::fs::remove_file(path).unwrap();

        let short_path = temp_path("short-read");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&short_path)
            .unwrap();
        file.set_len(SECTOR_SIZE as u64).unwrap();
        file.sync_all().unwrap();
        drop(file);
        let device = LinuxBlockDevice {
            file: OpenOptions::new()
                .read(true)
                .write(true)
                .open(&short_path)
                .unwrap(),
            capacity_bytes: (2 * SECTOR_SIZE) as u64,
        };
        assert_eq!(
            device.is_blank().unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        let mut device = device;
        assert_eq!(
            device.read_sector(1, &mut [0; SECTOR_SIZE]),
            Err(StorageError::Block)
        );
        drop(device);
        std::fs::remove_file(short_path).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn append_descriptor_is_rejected_before_offset_writes() {
        let path = temp_path("append-descriptor");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .append(true)
            .create_new(true)
            .open(&path)
            .unwrap();

        assert!(matches!(
            LinuxBlockDevice::from_file(file),
            Err(StorageProviderError::AppendDescriptor)
        ));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn format_returns_error_when_its_durability_flush_fails() {
        let path = temp_path("format-flush-failure");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.set_len(MIN_VFS_BYTES).unwrap();
        file.sync_all().unwrap();

        FLUSH_FAILURE_COUNTDOWN.with(|countdown| countdown.set(Some(0)));
        assert!(matches!(
            StorageProviderOwner::format_blank_file(file),
            Err(StorageProviderError::Vfs(StorageError::Block))
        ));

        let (owner, formatted) = StorageProviderOwner::mount_or_format(&path).unwrap();
        assert!(
            !formatted,
            "format wrote a valid filesystem before flush failed"
        );
        drop(owner);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn owner_shares_one_mount_and_rejects_duplicate_read_write_mounts() {
        let path = temp_path("single-owner");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.set_len(MIN_VFS_BYTES).unwrap();
        file.sync_all().unwrap();
        let owner = StorageProviderOwner::format_blank_file(file).unwrap();
        let provider = owner.consumer_handle();
        let second_consumer = provider.clone();

        let duplicate_fd = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(
            lock_volume_file(&duplicate_fd).unwrap_err().kind(),
            io::ErrorKind::WouldBlock,
            "the kernel lock rejects a separate open description of this volume"
        );
        assert!(matches!(
            StorageProviderOwner::mount_existing_file(duplicate_fd),
            Err(StorageProviderError::AlreadyMounted)
        ));

        #[cfg(target_os = "linux")]
        {
            let executable = std::env::var_os("NAGI_PROVIDER_MOUNT_PROBE_EXECUTABLE")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| std::env::current_exe().unwrap());
            let child = Command::new(executable)
                .arg("--exact")
                .arg("tests::subprocess_mount_probe_rejects_an_independent_second_owner")
                .arg("--nocapture")
                .env("NAGI_PROVIDER_MOUNT_PROBE_PATH", &path)
                .status()
                .unwrap_or_else(|error| {
                    panic!("start child process with independent volume open: {error}")
                });
            assert!(child.success(), "independent second mount was not rejected");
        }

        let handle = provider.create_path(b"/shared").unwrap();
        provider.write(handle, b"through-clone").unwrap();
        provider.flush().unwrap();
        assert_eq!(
            second_consumer
                .read_file_snapshot(b"/shared", 32)
                .unwrap()
                .1,
            b"through-clone"
        );

        drop(owner);
        let duplicate_fd = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert!(matches!(
            StorageProviderOwner::mount_existing_file(duplicate_fd),
            Err(StorageProviderError::AlreadyMounted)
        ));
        drop(provider);
        drop(second_consumer);
        let remounted = StorageProviderOwner::mount_existing(&path).unwrap();
        drop(remounted);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_only_descriptor_is_rejected_before_vfs_atime_can_write() {
        let path = temp_path("readonly");
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.set_len(MIN_VFS_BYTES).unwrap();
        file.sync_all().unwrap();
        let owner = StorageProviderOwner::format_blank_file(file).unwrap();
        let provider = owner.consumer_handle();
        let handle = provider.create_path(b"/file").unwrap();
        provider.write(handle, b"stable").unwrap();
        provider.flush().unwrap();
        drop(provider);
        drop(owner);

        let before = std::fs::read(&path).unwrap();
        let readonly = OpenOptions::new().read(true).open(&path).unwrap();
        assert!(matches!(
            StorageProviderOwner::mount_existing_file(readonly),
            Err(StorageProviderError::ReadOnlyDescriptor)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), before);

        let nonblank = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        assert!(matches!(
            StorageProviderOwner::format_blank_file(nonblank),
            Err(StorageProviderError::NotBlank)
        ));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn each_owner_is_bound_to_its_supplied_volume() {
        let first = temp_path("profile-a");
        let second = temp_path("profile-b");
        let first_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&first)
            .unwrap();
        first_file.set_len(MIN_VFS_BYTES).unwrap();
        let owner_a = StorageProviderOwner::format_blank_file(first_file).unwrap();
        let provider_a = owner_a.consumer_handle();
        let second_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&second)
            .unwrap();
        second_file.set_len(MIN_VFS_BYTES).unwrap();
        let owner_b = StorageProviderOwner::format_blank_file(second_file).unwrap();
        let provider_b = owner_b.consumer_handle();
        provider_a.mkdir_path(b"/notes").unwrap();
        let note = provider_a.create_path(b"/notes/only-a").unwrap();
        provider_a.write(note, b"profile-a").unwrap();
        provider_a.flush().unwrap();
        assert_eq!(
            provider_b.metadata_path(b"/notes/only-a"),
            Err(StorageError::NotFound)
        );
        drop(provider_a);
        drop(provider_b);
        drop(owner_a);
        drop(owner_b);
        std::fs::remove_file(first).unwrap();
        std::fs::remove_file(second).unwrap();
    }
}
