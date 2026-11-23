//! Linux file-descriptor block device and Notes adapter for the boot checkout's VFS.
//!
//! This development adapter is deliberately caller-rooted: it opens the device
//! path selected by the initramfs caller, never creates a file or chooses a
//! product storage authority. Desktop and Notes share the same locked VFS.

use super::{
    decode_stored, encode_stored_checked, same_candidate, summary, validate_candidate, NoteStore,
    NoteSummary, StoreError, MAX_STORED_NOTE_BYTES,
};
use crate::domain::NoteDocument;
use crate::identity::{HostObjectIdSource, ObjectIdSource};
use libnagi::storage::{
    BlockDevice, DirectoryEntry, FileHandle, ReadOnlyBlockDevice, StorageError, Vfs, BLOCK_SIZE,
    MAX_DIRECTORY_ENTRIES, MAX_FILE_SIZE,
};
use nagi_model::ObjectId;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

const SECTOR_SIZE: usize = 512;
const MIN_VFS_BYTES: u64 = 8 * 1024 * 1024;
const NOTES_DIRECTORY: &[u8] = b"/notes";
const DIRECTORY_RECORD_MIN_BYTES: usize = 8;
// The VFS stores one directory block, and its listing API validates each
// record as at least eight bytes. This format-derived ceiling is distinct
// from MAX_DIRECTORY_ENTRIES, which is only a small caller buffer size.
const MAX_DIRECTORY_LIST_ENTRIES: usize = BLOCK_SIZE / DIRECTORY_RECORD_MIN_BYTES;

pub struct LinuxBlockDevice {
    file: File,
    capacity_bytes: u64,
}

impl LinuxBlockDevice {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| StoreError::Io(format!("open block device: {error}")))?;
        let capacity_bytes = capacity_for_file(&file)
            .map_err(|error| StoreError::Io(format!("query block device capacity: {error}")))?;
        Ok(Self {
            file,
            capacity_bytes,
        })
    }

    fn capacity_bytes(&self) -> u64 {
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
        self.file.sync_all().map_err(|_| StorageError::Block)
    }
}

/// Cloneable access to one VFS instance. Every operation takes the same mutex,
/// so the Desktop's user-data handlers and Notes' private stored codec cannot
/// mount independent stale views over the same block device.
#[derive(Clone)]
pub struct SharedVfs {
    inner: Arc<Mutex<Vfs<LinuxBlockDevice>>>,
}

impl SharedVfs {
    pub fn mount_existing(path: &Path) -> Result<Self, StoreError> {
        let mut device = LinuxBlockDevice::open(path)?;
        validate_device_capacity(&device)?;
        Vfs::<LinuxBlockDevice>::check_existing(&mut device).map_err(storage_error)?;
        let volume = Vfs::mount_existing(device).map_err(storage_error)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(volume)),
        })
    }

    pub fn mount_or_format(path: &Path) -> Result<(Self, bool), StoreError> {
        let mut device = LinuxBlockDevice::open(path)?;
        validate_device_capacity(&device)?;
        let blank = device
            .is_blank()
            .map_err(|error| StoreError::Io(format!("inspect development disk: {error}")))?;
        if blank {
            let (volume, formatted) = Vfs::mount_or_format(device).map_err(storage_error)?;
            let volume = if formatted {
                volume
            } else {
                let mut device = volume.into_device();
                Vfs::<LinuxBlockDevice>::check_existing(&mut device).map_err(storage_error)?;
                Vfs::mount_existing(device).map_err(storage_error)?
            };
            return Ok((
                Self {
                    inner: Arc::new(Mutex::new(volume)),
                },
                formatted,
            ));
        }

        Vfs::<LinuxBlockDevice>::check_existing(&mut device).map_err(storage_error)?;
        let volume = Vfs::mount_existing(device).map_err(storage_error)?;
        Ok((
            Self {
                inner: Arc::new(Mutex::new(volume)),
            },
            false,
        ))
    }

    fn lock(&self) -> Result<MutexGuard<'_, Vfs<LinuxBlockDevice>>, StorageError> {
        self.inner.lock().map_err(|_| StorageError::Block)
    }

    pub(crate) fn with_volume<T>(
        &self,
        operation: impl FnOnce(&mut Vfs<LinuxBlockDevice>) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let mut volume = self.lock()?;
        operation(&mut volume)
    }

    fn with_note_volume<T>(
        &self,
        operation: impl FnOnce(&mut Vfs<LinuxBlockDevice>) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let mut volume = self
            .inner
            .lock()
            .map_err(|_| StoreError::Io("shared VFS lock poisoned".to_owned()))?;
        operation(&mut volume)
    }

    pub fn create_path(&mut self, path: &[u8]) -> Result<FileHandle, StorageError> {
        self.with_volume(|volume| volume.create_path(path))
    }

    pub fn open_path(&mut self, path: &[u8]) -> Result<FileHandle, StorageError> {
        self.with_volume(|volume| volume.open_path(path))
    }

    pub fn read(
        &mut self,
        handle: FileHandle,
        destination: &mut [u8],
    ) -> Result<usize, StorageError> {
        self.with_volume(|volume| volume.read(handle, destination))
    }

    pub fn write(&mut self, handle: FileHandle, contents: &[u8]) -> Result<(), StorageError> {
        self.with_volume(|volume| volume.write(handle, contents))
    }

    pub fn flush(&mut self) -> Result<(), StorageError> {
        self.with_volume(Vfs::flush)
    }
}

/// Notes `NoteStore` persisted as immutable revision files inside the same VFS
/// object passed to `Desktop::handle_event`. The adapter uses Notes' stored
/// Markdown codec from this crate, keeping ID/revision/body integrity checks.
pub struct VfsNoteStore {
    volume: SharedVfs,
    ids: HostObjectIdSource,
    gate: Mutex<()>,
}

impl VfsNoteStore {
    pub fn new(volume: SharedVfs) -> Self {
        Self {
            volume,
            ids: HostObjectIdSource::new(),
            gate: Mutex::new(()),
        }
    }

    fn note_directory(id: ObjectId) -> Vec<u8> {
        format!("/notes/{:016x}", id.0).into_bytes()
    }

    fn revision_path(id: ObjectId, revision: u64) -> Vec<u8> {
        format!("/notes/{:016x}/rev-{revision:020}.md", id.0).into_bytes()
    }

    fn list_directory_entries(
        volume: &mut Vfs<LinuxBlockDevice>,
        directory: &[u8],
    ) -> Result<Vec<DirectoryEntry>, StoreError> {
        let mut capacity = MAX_DIRECTORY_ENTRIES.min(MAX_DIRECTORY_LIST_ENTRIES);
        let mut entries = Vec::new();

        loop {
            entries.try_reserve_exact(capacity).map_err(|error| {
                StoreError::Io(format!("allocate bounded VFS directory buffer: {error}"))
            })?;
            entries.resize(capacity, DirectoryEntry::empty());

            match volume.list_directory_path(directory, &mut entries) {
                Ok(count) => {
                    entries.truncate(count);
                    return Ok(entries);
                }
                Err(StorageError::BufferTooSmall) if capacity < MAX_DIRECTORY_LIST_ENTRIES => {
                    entries.clear();
                    let next_capacity = capacity.saturating_mul(2).min(MAX_DIRECTORY_LIST_ENTRIES);
                    if next_capacity <= capacity {
                        return Err(storage_error(StorageError::BufferTooSmall));
                    }
                    capacity = next_capacity;
                }
                Err(error) => return Err(storage_error(error)),
            }
        }
    }

    fn versions(
        volume: &mut Vfs<LinuxBlockDevice>,
        id: ObjectId,
    ) -> Result<Vec<(u64, Vec<u8>)>, StoreError> {
        let directory = Self::note_directory(id);
        match volume.metadata_path(&directory) {
            Ok(metadata) if metadata.mode & 0xf000 == 0x4000 => {}
            Ok(_) => {
                return Err(StoreError::SandboxViolation(
                    "VFS note identity is not a directory".to_owned(),
                ));
            }
            Err(StorageError::NotFound) => return Ok(Vec::new()),
            Err(error) => return Err(storage_error(error)),
        }

        let entries = Self::list_directory_entries(volume, &directory)?;
        let mut versions = Vec::new();
        for entry in &entries {
            let name = std::str::from_utf8(entry.name())
                .map_err(|error| StoreError::Corrupt(error.to_string()))?;
            let Some(revision) = name
                .strip_prefix("rev-")
                .and_then(|value| value.strip_suffix(".md"))
            else {
                return Err(StoreError::Corrupt(
                    "unrecognized entry in VFS note directory".to_owned(),
                ));
            };
            if revision.len() != 20 || !revision.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(StoreError::Corrupt(
                    "invalid VFS note revision name".to_owned(),
                ));
            }
            let revision = revision
                .parse::<u64>()
                .map_err(|error| StoreError::Corrupt(error.to_string()))?;
            versions.push((
                revision,
                [directory.as_slice(), b"/", name.as_bytes()].concat(),
            ));
        }
        versions.sort_by_key(|(revision, _)| *revision);
        if versions.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(StoreError::Corrupt(
                "duplicate VFS note revision".to_owned(),
            ));
        }
        Ok(versions)
    }

    fn read_file(
        volume: &mut Vfs<LinuxBlockDevice>,
        id: ObjectId,
        revision: u64,
        path: &[u8],
        ids: &dyn ObjectIdSource,
    ) -> Result<Option<NoteDocument>, StoreError> {
        let metadata = match volume.metadata_path(path) {
            Ok(metadata) => metadata,
            Err(StorageError::NotFound) => return Ok(None),
            Err(error) => return Err(storage_error(error)),
        };
        if metadata.mode & 0xf000 != 0x8000 {
            return Err(StoreError::SandboxViolation(
                "VFS note revision is not a regular file".to_owned(),
            ));
        }
        let length = metadata.size as usize;
        if length > MAX_FILE_SIZE || length > MAX_STORED_NOTE_BYTES {
            return Err(StoreError::Corrupt(
                "stored note exceeds VFS capacity".to_owned(),
            ));
        }
        let handle = volume.open_path(path).map_err(storage_error)?;
        let mut bytes = vec![0; length];
        let read = volume.read(handle, &mut bytes).map_err(storage_error)?;
        if read != length {
            return Err(StoreError::Corrupt(
                "VFS note length changed during read".to_owned(),
            ));
        }
        let input = std::str::from_utf8(&bytes)
            .map_err(|error| StoreError::Corrupt(format!("note is not valid UTF-8: {error}")))?;
        let note = decode_stored(input, ids)?;
        if note.id != id || note.revision != revision {
            return Err(StoreError::Corrupt(
                "VFS note identity does not match its revision path".to_owned(),
            ));
        }
        validate_candidate(&note, revision)
            .map_err(|error| StoreError::Corrupt(error.to_string()))?;
        Ok(Some(note))
    }

    fn latest(
        volume: &mut Vfs<LinuxBlockDevice>,
        id: ObjectId,
        ids: &dyn ObjectIdSource,
    ) -> Result<Option<NoteDocument>, StoreError> {
        let versions = Self::versions(volume, id)?;
        let Some((revision, path)) = versions.last() else {
            return Ok(None);
        };
        Self::read_file(volume, id, *revision, path, ids)
    }

    fn ensure_directory(volume: &mut Vfs<LinuxBlockDevice>, path: &[u8]) -> Result<(), StoreError> {
        match volume.ensure_directory_path(path) {
            Ok(()) => Ok(()),
            Err(error) => Err(storage_error(error)),
        }
    }

    fn install_revision(
        volume: &mut Vfs<LinuxBlockDevice>,
        note: &NoteDocument,
        revision: u64,
    ) -> Result<(), StoreError> {
        let mut saved = note.clone();
        saved.revision = revision;
        let contents = encode_stored_checked(&saved).map_err(StoreError::InvalidDocument)?;
        if contents.len() > MAX_FILE_SIZE || contents.len() > MAX_STORED_NOTE_BYTES {
            return Err(StoreError::InvalidDocument(
                "stored note exceeds the VFS file capacity",
            ));
        }
        Self::ensure_directory(volume, NOTES_DIRECTORY)?;
        Self::ensure_directory(volume, &Self::note_directory(note.id))?;
        let path = Self::revision_path(note.id, revision);
        match volume.metadata_path(&path) {
            Ok(_) => {
                return Err(StoreError::RevisionConflict {
                    expected: revision.saturating_sub(1),
                    actual: revision,
                });
            }
            Err(StorageError::NotFound) => {}
            Err(error) => return Err(storage_error(error)),
        }
        let handle = volume.create_path(&path).map_err(storage_error)?;
        let result = volume
            .write(handle, contents.as_bytes())
            .and_then(|()| volume.flush());
        if let Err(error) = result {
            let rollback = volume.remove_path(&path).and_then(|()| volume.flush());
            return match rollback {
                Ok(()) => Err(storage_error(error)),
                Err(rollback_error) => Err(StoreError::Io(format!(
                    "VFS note write failed ({error:?}); rollback failed ({rollback_error:?})"
                ))),
            };
        }
        Ok(())
    }
}

impl NoteStore for VfsNoteStore {
    fn list(&self, include_deleted: bool) -> Result<Vec<NoteSummary>, StoreError> {
        self.volume.with_note_volume(|volume| {
            let metadata = match volume.metadata_path(NOTES_DIRECTORY) {
                Ok(metadata) if metadata.mode & 0xf000 == 0x4000 => metadata,
                Ok(_) => {
                    return Err(StoreError::SandboxViolation(
                        "VFS Notes root is not a directory".to_owned(),
                    ));
                }
                Err(StorageError::NotFound) => return Ok(Vec::new()),
                Err(error) => return Err(storage_error(error)),
            };
            let _ = metadata;
            let entries = Self::list_directory_entries(volume, NOTES_DIRECTORY)?;
            let mut summaries = Vec::new();
            for entry in &entries {
                let name = std::str::from_utf8(entry.name())
                    .map_err(|error| StoreError::Corrupt(error.to_string()))?;
                if name.len() != 16 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(StoreError::Corrupt(
                        "invalid VFS note identity directory".to_owned(),
                    ));
                }
                let id = u64::from_str_radix(name, 16)
                    .map_err(|error| StoreError::Corrupt(error.to_string()))?;
                let note = Self::latest(volume, ObjectId(id), &self.ids)?;
                if let Some(note) = note {
                    if include_deleted || !note.deleted {
                        summaries.push(summary(&note));
                    }
                }
            }
            summaries.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| left.id.0.cmp(&right.id.0))
            });
            Ok(summaries)
        })
    }

    fn load(&self, id: ObjectId) -> Result<Option<NoteDocument>, StoreError> {
        self.volume
            .with_note_volume(|volume| Self::latest(volume, id, &self.ids))
    }

    fn load_revision(
        &self,
        id: ObjectId,
        revision: u64,
    ) -> Result<Option<NoteDocument>, StoreError> {
        self.volume.with_note_volume(|volume| {
            let versions = Self::versions(volume, id)?;
            let Some((_, path)) = versions.iter().find(|(number, _)| *number == revision) else {
                return Ok(None);
            };
            Self::read_file(volume, id, revision, path, &self.ids)
        })
    }

    fn commit(
        &self,
        note: &NoteDocument,
        expected_revision: u64,
    ) -> Result<NoteDocument, StoreError> {
        validate_candidate(note, expected_revision)?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Io("VFS Notes lock poisoned".to_owned()))?;
        self.volume.with_note_volume(|volume| {
            let current = Self::latest(volume, note.id, &self.ids)?;
            let actual = current
                .as_ref()
                .map(|document| document.revision)
                .unwrap_or(0);
            if actual != expected_revision {
                if actual == expected_revision.saturating_add(1)
                    && current
                        .as_ref()
                        .is_some_and(|saved| same_candidate(saved, note))
                {
                    volume.flush().map_err(storage_error)?;
                    return Ok(current.expect("checked above"));
                }
                return Err(StoreError::RevisionConflict {
                    expected: expected_revision,
                    actual,
                });
            }
            let revision = expected_revision
                .checked_add(1)
                .ok_or(StoreError::RevisionOverflow)?;
            Self::install_revision(volume, note, revision)?;
            let mut saved = note.clone();
            saved.revision = revision;
            Ok(saved)
        })
    }
}

fn storage_error(error: StorageError) -> StoreError {
    StoreError::Io(format!("Nagi VFS operation failed: {error:?}"))
}

fn validate_device_capacity(device: &LinuxBlockDevice) -> Result<(), StoreError> {
    if device.capacity_bytes() < MIN_VFS_BYTES {
        return Err(StoreError::Io(format!(
            "Nagi VFS development disk is smaller than {} bytes",
            MIN_VFS_BYTES
        )));
    }
    if !device.capacity_bytes().is_multiple_of(SECTOR_SIZE as u64) {
        return Err(StoreError::Io(
            "Nagi VFS development disk has a non-sector-aligned capacity".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct FailOneDirectoryBufferAllocation;

    static DIRECTORY_BUFFER_ALLOCATION_OBSERVED: AtomicBool = AtomicBool::new(false);
    static DIRECTORY_BUFFER_LAYOUT_SIZE: AtomicUsize = AtomicUsize::new(0);
    static DIRECTORY_BUFFER_LAYOUT_ALIGN: AtomicUsize = AtomicUsize::new(0);

    thread_local! {
        static FAIL_NEXT_DIRECTORY_BUFFER: Cell<bool> = const { Cell::new(false) };
    }

    unsafe impl GlobalAlloc for FailOneDirectoryBufferAllocation {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let target_layout = layout.size()
                == DIRECTORY_BUFFER_LAYOUT_SIZE.load(Ordering::SeqCst)
                && layout.align() == DIRECTORY_BUFFER_LAYOUT_ALIGN.load(Ordering::SeqCst);
            let fail_this_allocation = target_layout
                && FAIL_NEXT_DIRECTORY_BUFFER
                    .try_with(|armed| armed.replace(false))
                    .unwrap_or(false);
            if fail_this_allocation {
                DIRECTORY_BUFFER_ALLOCATION_OBSERVED.store(true, Ordering::SeqCst);
                return std::ptr::null_mut();
            }
            // SAFETY: forwards the exact layout to the system allocator.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            // SAFETY: this allocator pairs every successful allocation with System.
            unsafe { System.dealloc(pointer, layout) }
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            // SAFETY: forwards the original allocation and requested size to System.
            unsafe { System.realloc(pointer, layout, new_size) }
        }
    }

    #[global_allocator]
    static TEST_ALLOCATOR: FailOneDirectoryBufferAllocation = FailOneDirectoryBufferAllocation;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "nagi-notes-{name}-{}-{}.img",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn create_blank_disk(path: &Path) {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        file.set_len(MIN_VFS_BYTES).unwrap();
        file.sync_all().unwrap();
    }

    #[test]
    fn directory_buffer_reservation_failure_returns_error_without_partial_load() {
        let path = temp_path("directory-allocation-failure");
        create_blank_disk(&path);
        let (shared, formatted) = SharedVfs::mount_or_format(&path).unwrap();
        assert!(formatted);
        let store = VfsNoteStore::new(shared.clone());
        let id = ObjectId(0x4e41_4749_0000_0101);
        let initial = NoteDocument::new(id, "allocation failure", crate::domain::Timestamp(1));
        let saved = store.commit(&initial, 0).unwrap();

        let layout = Layout::array::<DirectoryEntry>(MAX_DIRECTORY_ENTRIES).unwrap();
        DIRECTORY_BUFFER_LAYOUT_SIZE.store(layout.size(), Ordering::SeqCst);
        DIRECTORY_BUFFER_LAYOUT_ALIGN.store(layout.align(), Ordering::SeqCst);
        DIRECTORY_BUFFER_ALLOCATION_OBSERVED.store(false, Ordering::SeqCst);
        FAIL_NEXT_DIRECTORY_BUFFER.with(|armed| armed.set(true));

        let failed_load = store.load(id);
        FAIL_NEXT_DIRECTORY_BUFFER.with(|armed| armed.set(false));
        assert!(
            DIRECTORY_BUFFER_ALLOCATION_OBSERVED.load(Ordering::SeqCst),
            "the test allocator did not intercept the directory buffer reservation"
        );
        assert!(matches!(
            &failed_load,
            Err(StoreError::Io(message))
                if message.contains("allocate bounded VFS directory buffer")
        ));
        assert_eq!(store.load(id).unwrap(), Some(saved));

        drop(store);
        drop(shared);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn bounded_listing_propagates_vfs_errors_and_near_full_revision_directory() {
        let path = temp_path("directory-upper-bound");
        create_blank_disk(&path);
        let (shared, formatted) = SharedVfs::mount_or_format(&path).unwrap();
        assert!(formatted);
        let store = VfsNoteStore::new(shared.clone());
        let id = ObjectId(0x4e41_4749_0000_0102);
        let initial = NoteDocument::new(id, "target revision 1", crate::domain::Timestamp(1));
        let mut saved = store.commit(&initial, 0).unwrap();

        let missing = shared
            .with_note_volume(|volume| {
                VfsNoteStore::list_directory_entries(volume, b"/notes/missing")
            })
            .unwrap_err();
        assert!(matches!(
            &missing,
            StoreError::Io(message) if message.contains("NotFound")
        ));

        for revision in 2..=27 {
            let mut candidate = saved.clone();
            candidate.title = format!("target revision {revision}");
            candidate.updated_at = crate::domain::Timestamp(revision);
            saved = store.commit(&candidate, saved.revision).unwrap();
        }
        assert_eq!(saved.revision, 27);
        assert_eq!(store.load(id).unwrap(), Some(saved.clone()));
        assert_eq!(store.load_revision(id, 1).unwrap().unwrap().revision, 1);
        assert_eq!(
            MAX_DIRECTORY_LIST_ENTRIES,
            BLOCK_SIZE / DIRECTORY_RECORD_MIN_BYTES
        );

        let mut over_limit = saved.clone();
        over_limit.title = "target revision 28".to_owned();
        over_limit.updated_at = crate::domain::Timestamp(28);
        let error = store.commit(&over_limit, saved.revision).unwrap_err();
        assert!(matches!(
            &error,
            StoreError::Io(message) if message.contains("DirectoryFull")
        ));
        assert_eq!(store.load(id).unwrap(), Some(saved.clone()));
        assert_eq!(store.load_revision(id, 28).unwrap(), None);

        drop(store);
        drop(shared);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn vfs_notes_revisions_survive_file_backed_remount_and_reject_stale_writes() {
        let path = temp_path("vfs-revisions");
        create_blank_disk(&path);
        let (shared, formatted) = SharedVfs::mount_or_format(&path).unwrap();
        assert!(formatted);
        let store = VfsNoteStore::new(shared.clone());
        let id = ObjectId(0x4e41_4749_0000_0001);
        let initial = NoteDocument::new(id, "VFS Notes", crate::domain::Timestamp(1));
        let first = store.commit(&initial, 0).unwrap();
        assert_eq!(first.revision, 1);
        let mut second_candidate = first.clone();
        second_candidate.blocks.push(crate::domain::Block::new(
            ObjectId(0x424c_4f43_4b00_0001),
            crate::domain::BlockKind::Paragraph("persisted".to_owned()),
        ));
        let second = store.commit(&second_candidate, 1).unwrap();
        assert_eq!(second.revision, 2);
        assert_eq!(store.commit(&second_candidate, 1).unwrap(), second.clone());
        let mut conflicting_stale = second_candidate.clone();
        conflicting_stale.title = "conflicting stale title".to_owned();
        assert!(matches!(
            store.commit(&conflicting_stale, 1),
            Err(StoreError::RevisionConflict {
                expected: 1,
                actual: 2
            })
        ));

        drop(store);
        drop(shared);
        let before_reopen = std::fs::read(&path).unwrap();
        let (remounted, formatted) = SharedVfs::mount_or_format(&path).unwrap();
        assert!(
            !formatted,
            "a valid nonblank VFS must be mounted, not reformatted"
        );
        let reopened = VfsNoteStore::new(remounted);
        assert_eq!(reopened.load(id).unwrap(), Some(second.clone()));
        assert_eq!(reopened.load_revision(id, 1).unwrap(), Some(first));
        drop(reopened);
        assert_eq!(std::fs::read(&path).unwrap(), before_reopen);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn block_device_enforces_capacity_and_rejects_short_reads() {
        let boundary_path = temp_path("block-boundary");
        let original = [0xa5; SECTOR_SIZE];
        let mut backing = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&boundary_path)
            .unwrap();
        backing.write_all(&original).unwrap();
        backing.sync_all().unwrap();
        drop(backing);

        let mut device = LinuxBlockDevice {
            file: OpenOptions::new()
                .read(true)
                .write(true)
                .open(&boundary_path)
                .unwrap(),
            capacity_bytes: SECTOR_SIZE as u64,
        };
        let mut last_sector = [0; SECTOR_SIZE];
        device.read_sector(0, &mut last_sector).unwrap();
        assert_eq!(last_sector, original);
        assert_eq!(
            device.read_sector(1, &mut last_sector),
            Err(StorageError::Block)
        );
        assert_eq!(
            device.write_sector(1, &[0x5a; SECTOR_SIZE]),
            Err(StorageError::Block)
        );
        assert_eq!(
            device.checked_sector_offset(u64::MAX),
            Err(StorageError::Block)
        );
        assert_eq!(std::fs::read(&boundary_path).unwrap(), original);
        drop(device);
        std::fs::remove_file(boundary_path).unwrap();

        let short_path = temp_path("block-short-read");
        let backing = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&short_path)
            .unwrap();
        backing.set_len(SECTOR_SIZE as u64).unwrap();
        backing.sync_all().unwrap();
        drop(backing);

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

    #[test]
    fn mismatched_stored_note_identity_is_rejected_by_revision_path() {
        let path = temp_path("codec-mismatch");
        create_blank_disk(&path);
        let (shared, formatted) = SharedVfs::mount_or_format(&path).unwrap();
        assert!(formatted);

        let expected_id = ObjectId(0x4e41_4749_0000_0041);
        let wrong_id = ObjectId(0x4e41_4749_0000_0042);
        let mut wrong_note =
            NoteDocument::new(wrong_id, "Wrong identity", crate::domain::Timestamp(1));
        wrong_note.revision = 1;
        let encoded = encode_stored_checked(&wrong_note).unwrap();
        shared
            .with_volume(|volume| {
                volume.ensure_directory_path(NOTES_DIRECTORY)?;
                volume.ensure_directory_path(&VfsNoteStore::note_directory(expected_id))?;
                let revision_path = VfsNoteStore::revision_path(expected_id, 1);
                let handle = volume.create_path(&revision_path)?;
                volume.write(handle, encoded.as_bytes())?;
                volume.flush()
            })
            .unwrap();

        let store = VfsNoteStore::new(shared.clone());
        assert!(matches!(
            store.load(expected_id),
            Err(StoreError::Corrupt(_))
        ));
        drop(store);
        drop(shared);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn undersized_development_disk_is_rejected_without_formatting() {
        let path = temp_path("short-vfs");
        let original = vec![0x5au8; 2048];
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(&original).unwrap();
        file.sync_all().unwrap();
        drop(file);

        assert!(SharedVfs::mount_or_format(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn nonblank_invalid_development_disk_is_rejected_without_formatting() {
        let path = temp_path("invalid-vfs");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.set_len(MIN_VFS_BYTES).unwrap();
        file.write_all(&[0x5a; SECTOR_SIZE]).unwrap();
        file.sync_all().unwrap();
        drop(file);

        let before = std::fs::read(&path).unwrap();
        assert!(SharedVfs::mount_or_format(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_file(path).unwrap();
    }
}
