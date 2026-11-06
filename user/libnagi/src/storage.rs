use core::{
    ptr,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};
use sha2::{Digest, Sha256};

pub const SECTOR_SIZE: usize = 512;
pub const BLOCK_SIZE: usize = 1024;
/// Direct block pointers in an ext2 inode (`i_block[0..12]`).
const DIRECT_BLOCKS: usize = 12;
/// Block pointers held by the single indirect block (`i_block[12]`).
const INDIRECT_POINTERS: usize = BLOCK_SIZE / 4;
/// Largest regular file: twelve direct blocks plus one single-indirect block.
pub const MAX_FILE_SIZE: usize = (DIRECT_BLOCKS + INDIRECT_POINTERS) * BLOCK_SIZE;
/// Bound for callers that keep whole small records (settings, ledgers,
/// fixtures) in one stack buffer, and for whole-file `mmap` mappings.
pub const MAX_SMALL_FILE_SIZE: usize = BLOCK_SIZE;
pub const MAX_NAME_LENGTH: usize = 32;
pub const MAX_PATH_LENGTH: usize = 256;
pub const MAX_DIRECTORY_ENTRIES: usize = 8;

const EXT2_MAGIC: u16 = 0xef53;
const EXT2_BLOCK_COUNT: u32 = 8192;
const EXT2_INODE_COUNT: u32 = 64;
const EXT2_FIRST_DATA_BLOCK: u32 = 1;
const EXT2_BLOCKS_PER_GROUP: u32 = EXT2_BLOCK_COUNT;
const EXT2_INODES_PER_GROUP: u32 = EXT2_INODE_COUNT;
const EXT2_INODE_SIZE: u16 = 128;
const ROOT_INODE: u32 = 2;
const FIRST_FILE_INODE: u32 = 3;
const BLOCK_BITMAP: u32 = 3;
const INODE_BITMAP: u32 = 4;
const INODE_TABLE: u32 = 5;
const ROOT_DIRECTORY_BLOCK: u32 = 13;
const FIRST_FILE_BLOCK: u32 = 14;
const SUPERBLOCK_BLOCK: u32 = 1;
/// Blocks 0..15 are reserved metadata and are never file data.
const RESERVED_BLOCKS: u32 = 15;
const GROUP_DESCRIPTOR_BLOCK: u32 = 2;
// Already reserved by the Nagi format, never allocated as file contents.
const UNDO_BLOCK: u32 = 14;
const LEGACY_UNDO_MAGIC: &[u8; 8] = b"NAGIVTX1";
const UNDO_MAGIC: &[u8; 8] = b"NAGIVTX2";
const UNDO_FORMAT_V1: u8 = 1;
const UNDO_FORMAT_V2: u8 = 2;
const UNDO_CREATE: u8 = 1;
const UNDO_RENAME: u8 = 2;
const UNDO_DELETE: u8 = 3;

// This gate and the active-instance ID are process-local. Block authority
// must remain exclusive to one userspace storage service across processes.
// Nested API calls on the active Vfs are allowed; BlockDevice callbacks must
// not reenter Vfs.
static VOLUME_ACCESS: AtomicBool = AtomicBool::new(false);
static NEXT_ACCESS_ID: AtomicU64 = AtomicU64::new(1);
static ACTIVE_ACCESS_ID: AtomicU64 = AtomicU64::new(0);

fn allocate_access_id() -> Result<u64, StorageError> {
    NEXT_ACCESS_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| StorageError::Capacity)
}

struct VolumeAccess;
impl VolumeAccess {
    fn acquire() -> Self {
        while VOLUME_ACCESS
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        Self
    }
}
impl Drop for VolumeAccess {
    fn drop(&mut self) {
        VOLUME_ACCESS.store(false, Ordering::Release);
    }
}

struct ResetAccess(u64);
impl Drop for ResetAccess {
    fn drop(&mut self) {
        // A guard may clear only the active ID it installed. This remains
        // correct even if the gate's drop order changes.
        let _ = ACTIVE_ACCESS_ID.compare_exchange(self.0, 0, Ordering::Release, Ordering::Relaxed);
    }
}

struct Undo {
    kind: u8,
    directory_inode: u32,
    directory_block: u32,
    inode: u32,
    data_block: u32,
    previous_inode: [u8; EXT2_INODE_SIZE as usize],
    previous_indirect_checksum: [u8; 32],
    directory: [u8; BLOCK_SIZE],
}

#[derive(Clone, Copy)]
enum UndoAction {
    Create { inode: u32, data_block: u32 },
    Rename,
    Delete { inode: u32 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageError {
    /// A block-device operation failed. A started mutation whose rollback
    /// cannot be confirmed returns Indeterminate instead.
    Block,
    /// A mutation may have taken effect, but durable rollback could not be
    /// confirmed. Inspect the volume or run explicit Recovery before retrying.
    Indeterminate,
    Corrupt,
    NameTooLong,
    InvalidName,
    NotFound,
    AlreadyExists,
    NotDirectory,
    IsDirectory,
    DirectoryNotEmpty,
    DirectoryFull,
    InvalidHandle,
    FileTooLarge,
    BufferTooSmall,
    Capacity,
    /// A valid undo transaction exists and requires an explicit Recovery action.
    RecoveryRequired,
}

pub trait ReadOnlyBlockDevice {
    fn read_sector(
        &mut self,
        sector: u64,
        destination: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), StorageError>;
}

pub trait BlockDevice: ReadOnlyBlockDevice {
    fn write_sector(&mut self, sector: u64, source: &[u8; SECTOR_SIZE])
        -> Result<(), StorageError>;

    fn flush(&mut self) -> Result<(), StorageError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyscallBlockDevice {
    capability: u64,
}

impl SyscallBlockDevice {
    pub const fn new(capability: u64) -> Self {
        Self { capability }
    }
}

impl ReadOnlyBlockDevice for SyscallBlockDevice {
    fn read_sector(
        &mut self,
        sector: u64,
        destination: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        if block_read(self.capability, sector, destination) {
            Ok(())
        } else {
            Err(StorageError::Block)
        }
    }
}

impl BlockDevice for SyscallBlockDevice {
    fn write_sector(
        &mut self,
        sector: u64,
        source: &[u8; SECTOR_SIZE],
    ) -> Result<(), StorageError> {
        if block_write(self.capability, sector, source) {
            Ok(())
        } else {
            Err(StorageError::Block)
        }
    }

    fn flush(&mut self) -> Result<(), StorageError> {
        if block_flush(self.capability) {
            Ok(())
        } else {
            Err(StorageError::Block)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileHandle {
    inode: u32,
    /// The inode's generation when the handle was issued; a handle to a
    /// deleted and reused inode no longer validates.
    generation: u32,
}

impl FileHandle {
    pub const fn invalid() -> Self {
        Self {
            inode: 0,
            generation: 0,
        }
    }

    pub const fn inode(self) -> u32 {
        self.inode
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub inode: u32,
    name: [u8; MAX_NAME_LENGTH],
    pub name_len: u8,
    pub file_type: u8,
}

impl DirectoryEntry {
    pub const fn empty() -> Self {
        Self {
            inode: 0,
            name: [0; MAX_NAME_LENGTH],
            name_len: 0,
            file_type: 0,
        }
    }

    pub fn name(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.name.as_ptr(), self.name_len as usize) }
    }
}

#[derive(Clone, Copy)]
struct InodeInfo {
    mode: u16,
    uid: u16,
    gid: u16,
    size: u32,
    atime: u32,
    ctime: u32,
    mtime: u32,
    blocks: u32,
    /// `i_block[0]`; directories use only this block.
    direct_block: u32,
    /// `i_block[1..12]` for regular files.
    extra_blocks: [u32; DIRECT_BLOCKS - 1],
    /// `i_block[12]`, the single-indirect block, or 0.
    indirect_block: u32,
    /// ext2 `i_generation`; 0 on volumes written before generations.
    generation: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileMetadata {
    pub inode: u32,
    /// Distinguishes successive files that reuse the same inode number.
    pub generation: u32,
    pub mode: u16,
    pub uid: u16,
    pub gid: u16,
    pub size: u32,
    pub atime: u32,
    pub ctime: u32,
    pub mtime: u32,
    pub blocks: u32,
}

/// Counts from a bounded, read-only check of Nagi's ext2-like VFS layout.
/// The directory count includes root. The entry count excludes dot entries
/// and tombstones. Allocated data blocks exclude reserved blocks and root's
/// block. This checks the current Nagi format, not general ext2.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VfsIntegrityReport {
    pub regular_files: u32,
    pub directories: u32,
    pub allocated_data_blocks: u32,
    pub directory_entries: u32,
}

#[derive(Clone, Copy)]
pub struct FileMapping {
    handle: FileHandle,
    bytes: [u8; MAX_SMALL_FILE_SIZE],
    length: usize,
}

impl FileMapping {
    pub fn bytes(&self) -> &[u8] {
        unsafe { core::slice::from_raw_parts(self.bytes.as_ptr(), self.length) }
    }

    pub fn bytes_mut(&mut self) -> &mut [u8] {
        unsafe { core::slice::from_raw_parts_mut(self.bytes.as_mut_ptr(), self.length) }
    }

    pub fn set_length(&mut self, length: usize) -> Result<(), StorageError> {
        if length > MAX_SMALL_FILE_SIZE {
            return Err(StorageError::FileTooLarge);
        }
        self.length = length;
        Ok(())
    }

    pub const fn handle(&self) -> FileHandle {
        self.handle
    }

    pub const fn munmap(self) -> FileHandle {
        self.handle
    }
}

pub struct Vfs<D> {
    device: D,
    access_id: u64,
    recovery_required: bool,
}

impl<D: BlockDevice> Vfs<D> {
    /// Mount an existing Nagi VFS without formatting, initializing, or repairing it.
    /// A valid pending undo transaction returns StorageError::RecoveryRequired; use the
    /// explicit Recovery path to restore it. Callers that need an integrity
    /// guarantee must run check_existing first.
    pub fn mount_existing(device: D) -> Result<Self, StorageError> {
        let _access = VolumeAccess::acquire();
        let mut volume = Self {
            device,
            access_id: allocate_access_id()?,
            recovery_required: false,
        };
        let mut superblock = [0; BLOCK_SIZE];
        volume.read_block(SUPERBLOCK_BLOCK, &mut superblock)?;
        if read_u16(&superblock, 56) != EXT2_MAGIC {
            return Err(StorageError::Corrupt);
        }
        validate_superblock(&superblock)?;
        if load_undo(&mut volume.device)?.is_some() {
            return Err(StorageError::RecoveryRequired);
        }
        Ok(volume)
    }

    /// Mount an existing volume or format a blank device, without repairing
    /// pending undo. A valid pending transaction returns StorageError::RecoveryRequired.
    pub fn mount_or_format(device: D) -> Result<(Self, bool), StorageError> {
        let _access = VolumeAccess::acquire();
        let mut volume = Self {
            device,
            access_id: allocate_access_id()?,
            recovery_required: false,
        };
        let mut superblock = [0; BLOCK_SIZE];
        volume.read_block(SUPERBLOCK_BLOCK, &mut superblock)?;
        if read_u16(&superblock, 56) != EXT2_MAGIC {
            let mut header = [0; SECTOR_SIZE];
            volume.device.read_sector(0, &mut header)?;
            if header.iter().any(|byte| *byte != 0) {
                return Err(StorageError::Corrupt);
            }
            volume.format()?;
            return Ok((volume, true));
        }
        validate_superblock(&superblock)?;
        if load_undo(&mut volume.device)?.is_some() {
            return Err(StorageError::RecoveryRequired);
        }
        Ok((volume, false))
    }

    /// Explicitly restore a valid pending undo transaction without formatting.
    /// This is intended for the Recovery console; ordinary mounts and VFS
    /// operations only inspect the journal and return StorageError::RecoveryRequired.
    pub fn recover_existing(device: D) -> Result<Self, StorageError> {
        let _access = VolumeAccess::acquire();
        let mut volume = Self {
            device,
            access_id: allocate_access_id()?,
            recovery_required: false,
        };
        let mut superblock = [0; BLOCK_SIZE];
        volume.read_block(SUPERBLOCK_BLOCK, &mut superblock)?;
        if read_u16(&superblock, 56) != EXT2_MAGIC {
            return Err(StorageError::Corrupt);
        }
        validate_superblock(&superblock)?;
        volume.recover_undo()?;
        Ok(volume)
    }

    pub fn into_device(self) -> D {
        self.device
    }

    /// Resolve a bounded path below this volume's process root and ensure its
    /// final component names a directory. This is used when mounting the
    /// conventional guest `/tmp` directory; an existing non-directory is an
    /// error rather than being treated as a successful setup.
    pub fn ensure_directory_path(&mut self, path: &[u8]) -> Result<(), StorageError> {
        self.with_access(|volume| match volume.resolve_path(path) {
            Ok(inode) => {
                volume.directory_inode(inode)?;
                Ok(())
            }
            Err(StorageError::NotFound) => volume.mkdir_path(path),
            Err(error) => Err(error),
        })
    }

    pub fn create_path(&mut self, path: &[u8]) -> Result<FileHandle, StorageError> {
        self.with_access(|volume| {
            let (parent, name) = volume.resolve_parent_path(path, false)?;
            volume.create_in_directory(parent, name)
        })
    }

    /// Create a new single-block file with its initial bytes inside the create
    /// undo transaction. Existing paths are never replaced. Reported I/O errors
    /// follow the same rollback/explicit-Recovery contract as ordinary create.
    pub fn create_path_with_initial_contents(
        &mut self,
        path: &[u8],
        contents: &[u8],
    ) -> Result<FileHandle, StorageError> {
        self.with_access(|volume| {
            if contents.len() > BLOCK_SIZE {
                return Err(StorageError::FileTooLarge);
            }
            let (parent, name) = volume.resolve_parent_path(path, false)?;
            volume.create_in_directory_with_initial_contents(parent, name, contents)
        })
    }

    pub fn mkdir_path(&mut self, path: &[u8]) -> Result<(), StorageError> {
        self.with_access(|volume| {
            let (parent, name) = volume.resolve_parent_path(path, true)?;
            volume.mkdir_in_directory(parent, name)
        })
    }

    pub fn open_path(&mut self, path: &[u8]) -> Result<FileHandle, StorageError> {
        self.with_access(|volume| {
            let inode_number = volume.resolve_path(path)?;
            let inode = volume.read_inode(inode_number)?;
            if inode.mode & 0xf000 == 0x4000 {
                return Err(StorageError::IsDirectory);
            }
            if inode.mode & 0xf000 != 0x8000 {
                return Err(StorageError::InvalidHandle);
            }
            volume.handle_for(inode_number)
        })
    }

    pub fn metadata_path(&mut self, path: &[u8]) -> Result<FileMetadata, StorageError> {
        self.with_access(|volume| {
            let inode_number = volume.resolve_path(path)?;
            let inode = volume.read_inode(inode_number)?;
            if inode.mode & 0xf000 != 0x8000 && inode.mode & 0xf000 != 0x4000 {
                return Err(StorageError::InvalidHandle);
            }
            Ok(metadata_from_inode(inode_number, inode))
        })
    }

    /// Remove a regular file by path. Directory removal has a separate POSIX
    /// operation so `unlink` cannot silently remove a directory.
    pub fn remove_path(&mut self, path: &[u8]) -> Result<(), StorageError> {
        self.with_access(|volume| volume.remove_path_with_kind(path, Some(false), false))
    }

    pub fn rmdir_path(&mut self, path: &[u8]) -> Result<(), StorageError> {
        self.with_access(|volume| volume.remove_path_with_kind(path, Some(true), true))
    }

    pub fn list_directory_path(
        &mut self,
        path: &[u8],
        entries: &mut [DirectoryEntry],
    ) -> Result<usize, StorageError> {
        self.with_access(|volume| {
            let inode = volume.resolve_path(path)?;
            volume.list_directory_inode(inode, entries)
        })
    }

    pub fn create(&mut self, name: &[u8]) -> Result<FileHandle, StorageError> {
        self.with_access(|volume| {
            validate_name(name)?;
            volume.create_in_directory(ROOT_INODE, name)
        })
    }

    fn create_in_directory(
        &mut self,
        parent_inode: u32,
        name: &[u8],
    ) -> Result<FileHandle, StorageError> {
        self.create_in_directory_with_initial_contents(parent_inode, name, &[])
    }

    fn create_in_directory_with_initial_contents(
        &mut self,
        parent_inode: u32,
        name: &[u8],
        contents: &[u8],
    ) -> Result<FileHandle, StorageError> {
        if contents.len() > BLOCK_SIZE {
            return Err(StorageError::FileTooLarge);
        }
        validate_name(name)?;
        self.directory_inode(parent_inode)?;
        if self.find_inode_in_directory(parent_inode, name)?.is_some() {
            return Err(StorageError::AlreadyExists);
        }
        let inode = self.find_free_bit(INODE_BITMAP, FIRST_FILE_INODE - 1, EXT2_INODE_COUNT)? + 1;
        let data_block = self.find_free_bit(BLOCK_BITMAP, RESERVED_BLOCKS, EXT2_BLOCK_COUNT)?;
        let generation = self.next_generation(inode)?;
        self.transaction(
            parent_inode,
            UndoAction::Create { inode, data_block },
            |volume| {
                volume.set_allocated(INODE_BITMAP, inode - 1)?;
                volume.set_allocated(BLOCK_BITMAP, data_block)?;
                // Initialize only this newly allocated block before publishing
                // the inode or directory entry. Empty create still zeroes it.
                let mut initial = [0; BLOCK_SIZE];
                initial[..contents.len()].copy_from_slice(contents);
                volume.write_block(data_block, &initial)?;
                let now = current_timestamp();
                volume.write_inode(
                    inode,
                    InodeInfo {
                        mode: 0x8000,
                        uid: 0,
                        gid: 0,
                        size: contents.len() as u32,
                        atime: now,
                        ctime: now,
                        mtime: now,
                        blocks: if contents.is_empty() { 0 } else { 2 },
                        direct_block: data_block,
                        extra_blocks: [0; DIRECT_BLOCKS - 1],
                        indirect_block: 0,
                        generation,
                    },
                )?;
                volume.add_directory_entry_in_directory(parent_inode, name, inode, 1)?;
                volume.adjust_free_counts(-1, -1)?;
                Ok(FileHandle { inode, generation })
            },
        )
    }

    pub fn mkdir(&mut self, name: &[u8]) -> Result<(), StorageError> {
        self.with_access(|volume| {
            validate_name(name)?;
            volume.mkdir_in_directory(ROOT_INODE, name)
        })
    }

    fn mkdir_in_directory(&mut self, parent_inode: u32, name: &[u8]) -> Result<(), StorageError> {
        validate_name(name)?;
        self.directory_inode(parent_inode)?;
        if self.find_inode_in_directory(parent_inode, name)?.is_some() {
            return Err(StorageError::AlreadyExists);
        }
        let inode = self.find_free_bit(INODE_BITMAP, FIRST_FILE_INODE - 1, EXT2_INODE_COUNT)? + 1;
        // Block 14 is the persistent undo backup. Keep mkdir on the same
        // allocatable range as create so it cannot overwrite its own journal.
        let data_block = self.find_free_bit(BLOCK_BITMAP, RESERVED_BLOCKS, EXT2_BLOCK_COUNT)?;
        let generation = self.next_generation(inode)?;
        self.transaction(
            parent_inode,
            UndoAction::Create { inode, data_block },
            |volume| {
                volume.set_allocated(INODE_BITMAP, inode - 1)?;
                volume.set_allocated(BLOCK_BITMAP, data_block)?;
                let now = current_timestamp();
                volume.write_inode(
                    inode,
                    InodeInfo {
                        mode: 0x4000,
                        uid: 0,
                        gid: 0,
                        size: BLOCK_SIZE as u32,
                        atime: now,
                        ctime: now,
                        mtime: now,
                        blocks: 2,
                        direct_block: data_block,
                        extra_blocks: [0; DIRECT_BLOCKS - 1],
                        indirect_block: 0,
                        generation,
                    },
                )?;
                let mut directory = [0; BLOCK_SIZE];
                let dot = [b'.'];
                let dot_dot = [b'.', b'.'];
                write_directory_record(&mut directory, 0, inode, 12, 1, 2, &dot);
                write_directory_record(&mut directory, 12, parent_inode, 12, 2, 2, &dot_dot);
                write_directory_record(&mut directory, 24, 0, BLOCK_SIZE as u16 - 24, 0, 0, b"");
                volume.write_block(data_block, &directory)?;
                volume.add_directory_entry_in_directory(parent_inode, name, inode, 2)?;
                volume.adjust_free_counts(-1, -1)?;
                Ok(())
            },
        )
    }

    pub fn remove(&mut self, name: &[u8]) -> Result<(), StorageError> {
        self.with_access(|volume| {
            validate_name(name)?;
            let (directory_offset, inode_number) = volume
                .find_directory_entry(name)?
                .ok_or(StorageError::NotFound)?;
            let inode = volume.read_inode(inode_number)?;
            if inode.mode & 0xf000 != 0x8000 && inode.mode & 0xf000 != 0x4000 {
                return Err(StorageError::InvalidHandle);
            }
            if inode.mode & 0xf000 == 0x4000 && !volume.directory_is_empty(inode.direct_block)? {
                return Err(StorageError::DirectoryFull);
            }
            let mut directory = [0; BLOCK_SIZE];
            volume.read_block(ROOT_DIRECTORY_BLOCK, &mut directory)?;
            write_u32(&mut directory, directory_offset, 0);
            volume.transaction(
                ROOT_INODE,
                UndoAction::Delete {
                    inode: inode_number,
                },
                |volume| {
                    volume.write_block(ROOT_DIRECTORY_BLOCK, &directory)?;
                    volume.clear_bit(INODE_BITMAP, inode_number - 1)?;
                    let released = volume.release_inode_blocks(&inode)?;
                    volume.clear_inode(inode_number)?;
                    volume.adjust_free_counts(released as i32, 1)
                },
            )
        })
    }

    pub fn open(&mut self, name: &[u8]) -> Result<FileHandle, StorageError> {
        self.with_access(|volume| {
            validate_name(name)?;
            let inode = volume.find_inode(name)?.ok_or(StorageError::NotFound)?;
            volume.handle_for(inode)
        })
    }

    pub fn rename(&mut self, old_name: &[u8], new_name: &[u8]) -> Result<FileHandle, StorageError> {
        self.with_access(|volume| volume.rename_child_inode(ROOT_INODE, old_name, new_name))
    }

    /// Rename an entry in the directory at `directory_path` without changing
    /// its inode or generation. Both names must be direct children of that
    /// directory; moving entries between directories is a separate operation.
    /// The returned value identifies either a regular file or directory; file
    /// I/O continues to reject directory identities.
    pub fn rename_child(
        &mut self,
        directory_path: &[u8],
        old_name: &[u8],
        new_name: &[u8],
    ) -> Result<FileHandle, StorageError> {
        self.with_access(|volume| {
            let directory_inode = volume.resolve_path(directory_path)?;
            volume.rename_child_inode(directory_inode, old_name, new_name)
        })
    }

    fn rename_child_inode(
        &mut self,
        directory_inode: u32,
        old_name: &[u8],
        new_name: &[u8],
    ) -> Result<FileHandle, StorageError> {
        validate_name(old_name)?;
        validate_name(new_name)?;
        if old_name == b"." || old_name == b".." || new_name == b"." || new_name == b".." {
            return Err(StorageError::InvalidName);
        }
        // Resolve and validate the result before starting mutation. Directory
        // rename must not commit and then fail regular-file handle validation.
        let source_inode = self
            .find_inode_in_directory(directory_inode, old_name)?
            .ok_or(StorageError::NotFound)?;
        let source = self.read_inode(source_inode)?;
        let identity = match source.mode & 0xf000 {
            0x8000 => self.handle_for(source_inode)?,
            0x4000 => {
                self.directory_inode(source_inode)?;
                FileHandle {
                    inode: source_inode,
                    generation: effective_generation(source.generation),
                }
            }
            _ => return Err(StorageError::InvalidHandle),
        };
        if old_name == new_name {
            return Ok(identity);
        }
        if self
            .find_inode_in_directory(directory_inode, new_name)?
            .is_some()
        {
            return Err(StorageError::AlreadyExists);
        }
        let directory = self.directory_inode(directory_inode)?;
        let mut entries = [0; BLOCK_SIZE];
        self.read_block(directory.direct_block, &mut entries)?;

        // Directory records can use different amounts of slack depending on
        // when each entry was created. Repack the single-block directory so a
        // longer replacement name can use free space elsewhere in that block.
        let mut offset = 0;
        let mut live_entries = 0;
        let mut required_bytes = 0;
        let mut found_old_name = false;
        while offset < BLOCK_SIZE {
            let inode = read_u32(&entries, offset);
            let record_length = usize::from(read_u16(&entries, offset + 4));
            let name_length = usize::from(read_u8(&entries, offset + 6));
            if record_length < 8
                || record_length % 4 != 0
                || offset + record_length > BLOCK_SIZE
                || name_length > record_length - 8
            {
                return Err(StorageError::Corrupt);
            }
            if inode != 0 {
                let is_target = names_equal(&entries, offset + 8, name_length, old_name);
                if is_target && found_old_name {
                    return Err(StorageError::Corrupt);
                }
                found_old_name |= is_target;
                let output_name_length = if is_target {
                    new_name.len()
                } else {
                    name_length
                };
                required_bytes += align4(8 + output_name_length);
                live_entries += 1;
            }
            offset += record_length;
        }
        if !found_old_name {
            return Err(StorageError::NotFound);
        }
        if required_bytes > BLOCK_SIZE {
            return Err(StorageError::DirectoryFull);
        }

        let mut compacted = [0; BLOCK_SIZE];
        let mut source_offset = 0;
        let mut destination_offset = 0;
        let mut remaining_entries = live_entries;
        // Keep a reusable free record at the end when it can hold even the
        // shortest valid filename. Otherwise the final live record owns the
        // small tail so the directory remains well-formed.
        let trailing_bytes = BLOCK_SIZE - required_bytes;
        let preserve_free_record = trailing_bytes >= align4(8 + 1);
        while source_offset < BLOCK_SIZE {
            let inode = read_u32(&entries, source_offset);
            let record_length = usize::from(read_u16(&entries, source_offset + 4));
            let name_length = usize::from(read_u8(&entries, source_offset + 6));
            if inode != 0 {
                let source_name = &entries[source_offset + 8..source_offset + 8 + name_length];
                let is_target = names_equal(&entries, source_offset + 8, name_length, old_name);
                let output_name = if is_target { new_name } else { source_name };
                let minimum_length = align4(8 + output_name.len());
                let output_record_length = if remaining_entries == 1 && !preserve_free_record {
                    BLOCK_SIZE - destination_offset
                } else {
                    minimum_length
                };
                if output_record_length < minimum_length {
                    return Err(StorageError::DirectoryFull);
                }
                write_directory_record(
                    &mut compacted,
                    destination_offset,
                    inode,
                    output_record_length as u16,
                    output_name.len() as u8,
                    read_u8(&entries, source_offset + 7),
                    output_name,
                );
                destination_offset += output_record_length;
                remaining_entries -= 1;
            }
            source_offset += record_length;
        }
        if preserve_free_record {
            let free_length = BLOCK_SIZE - destination_offset;
            write_directory_record(
                &mut compacted,
                destination_offset,
                0,
                free_length as u16,
                0,
                0,
                b"",
            );
        }
        self.transaction(directory_inode, UndoAction::Rename, |volume| {
            volume.write_changed_block(directory.direct_block, &compacted)
        })?;
        Ok(identity)
    }

    /// Replace a root entry in one directory-block update. The destination
    /// keeps its name while the source inode becomes visible there; the old
    /// destination inode is released only after the directory transaction is
    /// durable. Package updates use this to avoid a remove-then-rename gap.
    pub fn replace(
        &mut self,
        source_name: &[u8],
        destination_name: &[u8],
    ) -> Result<FileHandle, StorageError> {
        self.with_access(|volume| {
            validate_name(source_name)?;
            validate_name(destination_name)?;
            if source_name == destination_name {
                return volume.open(source_name);
            }
            let (source_offset, source_inode_number) = volume
                .find_directory_entry(source_name)?
                .ok_or(StorageError::NotFound)?;
            let (destination_offset, destination_inode_number) = volume
                .find_directory_entry(destination_name)?
                .ok_or(StorageError::NotFound)?;
            let source_inode = volume.read_inode(source_inode_number)?;
            let destination_inode = volume.read_inode(destination_inode_number)?;
            if source_inode.mode & 0xf000 != 0x8000 || destination_inode.mode & 0xf000 != 0x8000 {
                return Err(StorageError::InvalidHandle);
            }

            let mut directory = [0; BLOCK_SIZE];
            volume.read_block(ROOT_DIRECTORY_BLOCK, &mut directory)?;
            write_u32(&mut directory, destination_offset, source_inode_number);
            write_u32(&mut directory, source_offset, 0);
            volume.write_block(ROOT_DIRECTORY_BLOCK, &directory)?;
            volume.clear_bit(INODE_BITMAP, destination_inode_number - 1)?;
            let released = volume.release_inode_blocks(&destination_inode)?;
            volume.clear_inode(destination_inode_number)?;
            volume.adjust_free_counts(released as i32, 1)?;
            volume.handle_for(source_inode_number)
        })
    }

    pub fn list_root(&mut self, entries: &mut [DirectoryEntry]) -> Result<usize, StorageError> {
        self.with_access(|volume| volume.list_directory_inode(ROOT_INODE, entries))
    }

    fn list_directory_inode(
        &mut self,
        inode_number: u32,
        entries: &mut [DirectoryEntry],
    ) -> Result<usize, StorageError> {
        let inode = self.directory_inode(inode_number)?;
        let mut directory = [0; BLOCK_SIZE];
        self.read_block(inode.direct_block, &mut directory)?;
        let mut offset = 0;
        let mut count = 0;
        while offset < BLOCK_SIZE {
            let inode = read_u32(&directory, offset);
            let record_length = usize::from(read_u16(&directory, offset + 4));
            let name_length = usize::from(read_u8(&directory, offset + 6));
            if record_length < 8
                || record_length % 4 != 0
                || offset + record_length > BLOCK_SIZE
                || name_length > record_length - 8
            {
                return Err(StorageError::Corrupt);
            }
            if inode != 0 && !is_dot_entry(&directory, offset, name_length) {
                let Some(entry) = entries.get_mut(count) else {
                    return Err(StorageError::BufferTooSmall);
                };
                entry.inode = inode;
                entry.name_len = name_length as u8;
                entry.file_type = read_u8(&directory, offset + 7);
                copy_bytes_from_offset(&mut entry.name, &directory, offset + 8, name_length);
                count += 1;
            }
            offset += record_length;
        }
        Ok(count)
    }

    /// Replace the whole file contents with `data`.
    pub fn write(&mut self, handle: FileHandle, data: &[u8]) -> Result<(), StorageError> {
        self.with_access(|volume| {
            if data.len() > MAX_FILE_SIZE {
                return Err(StorageError::FileTooLarge);
            }
            let mut inode = volume.validate_handle(handle)?;
            let current = data_blocks_for_size(inode.size as usize);
            let needed = data_blocks_for_size(data.len());
            volume.resize_file_blocks(&mut inode, current, needed)?;
            volume.store_file_range(&inode, 0, data)?;
            volume.clear_past_end(&inode, data.len())?;
            inode.size = data.len() as u32;
            let now = current_timestamp();
            inode.ctime = now;
            inode.mtime = now;
            volume.write_inode(handle.inode, inode)
        })
    }

    /// Write `data` at `offset`, growing the file as needed. A gap between
    /// the old end and `offset` reads as zeros.
    pub fn write_at(
        &mut self,
        handle: FileHandle,
        offset: usize,
        data: &[u8],
    ) -> Result<(), StorageError> {
        self.with_access(|volume| {
            let end = offset
                .checked_add(data.len())
                .filter(|end| *end <= MAX_FILE_SIZE)
                .ok_or(StorageError::FileTooLarge)?;
            let mut inode = volume.validate_handle(handle)?;
            let size = inode.size as usize;
            let new_size = size.max(end);
            let current = data_blocks_for_size(size);
            let needed = data_blocks_for_size(new_size);
            if needed > current || inode.blocks == 0 {
                volume.resize_file_blocks(&mut inode, current, needed)?;
            }
            if offset > size {
                volume.zero_file_range(&inode, size, offset)?;
            }
            volume.store_file_range(&inode, offset, data)?;
            inode.size = new_size as u32;
            let now = current_timestamp();
            inode.ctime = now;
            inode.mtime = now;
            volume.write_inode(handle.inode, inode)
        })
    }

    pub fn truncate(&mut self, handle: FileHandle, length: usize) -> Result<(), StorageError> {
        self.with_access(|volume| {
            if length > MAX_FILE_SIZE {
                return Err(StorageError::FileTooLarge);
            }
            let mut inode = volume.validate_handle(handle)?;
            let size = inode.size as usize;
            let current = data_blocks_for_size(size);
            let needed = data_blocks_for_size(length);
            if length < size {
                volume.clear_past_end(&inode, length)?;
            } else if length > size {
                // Zero only the already-owned tail before allocating more
                // blocks; resize_file_blocks zeros every newly allocated block.
                let existing_capacity = current
                    .checked_mul(BLOCK_SIZE)
                    .ok_or(StorageError::Corrupt)?;
                let existing_end = length.min(existing_capacity);
                if existing_end > size {
                    volume.zero_file_range(&inode, size, existing_end)?;
                }
            }
            volume.resize_file_blocks(&mut inode, current, needed)?;
            inode.size = length as u32;
            let now = current_timestamp();
            inode.ctime = now;
            inode.mtime = now;
            volume.write_inode(handle.inode, inode)
        })
    }

    pub fn metadata(&mut self, handle: FileHandle) -> Result<FileMetadata, StorageError> {
        self.with_access(|volume| {
            let inode = volume.validate_handle(handle)?;
            Ok(metadata_from_inode(handle.inode, inode))
        })
    }

    pub fn set_mode(&mut self, handle: FileHandle, mode: u16) -> Result<(), StorageError> {
        self.with_access(|volume| {
            let mut inode = volume.validate_handle(handle)?;
            inode.mode = (inode.mode & 0xf000) | (mode & 0x0fff);
            inode.ctime = current_timestamp();
            volume.write_inode(handle.inode, inode)
        })
    }

    pub fn set_owner(
        &mut self,
        handle: FileHandle,
        uid: Option<u16>,
        gid: Option<u16>,
    ) -> Result<(), StorageError> {
        self.with_access(|volume| {
            let mut inode = volume.validate_handle(handle)?;
            if let Some(uid) = uid {
                inode.uid = uid;
            }
            if let Some(gid) = gid {
                inode.gid = gid;
            }
            inode.ctime = current_timestamp();
            volume.write_inode(handle.inode, inode)
        })
    }

    pub fn set_times(
        &mut self,
        handle: FileHandle,
        atime: u32,
        mtime: u32,
    ) -> Result<(), StorageError> {
        self.with_access(|volume| {
            let mut inode = volume.validate_handle(handle)?;
            inode.atime = atime;
            inode.mtime = mtime;
            inode.ctime = current_timestamp();
            volume.write_inode(handle.inode, inode)
        })
    }

    pub fn flush(&mut self) -> Result<(), StorageError> {
        self.with_access(|volume| volume.device.flush())
    }

    pub fn read(
        &mut self,
        handle: FileHandle,
        destination: &mut [u8],
    ) -> Result<usize, StorageError> {
        self.with_access(|volume| {
            let mut inode = volume.validate_handle(handle)?;
            let length = usize::try_from(inode.size).map_err(|_| StorageError::Corrupt)?;
            if length > MAX_FILE_SIZE {
                return Err(StorageError::Corrupt);
            }
            if destination.len() < length {
                return Err(StorageError::BufferTooSmall);
            }
            volume.copy_file_range(&inode, 0, &mut destination[..length])?;
            inode.atime = current_timestamp();
            volume.write_inode(handle.inode, inode)?;
            Ok(length)
        })
    }

    /// Read up to `destination.len()` bytes starting at `offset`. Returns
    /// the count read, 0 at or past the end of the file.
    pub fn read_at(
        &mut self,
        handle: FileHandle,
        offset: usize,
        destination: &mut [u8],
    ) -> Result<usize, StorageError> {
        self.with_access(|volume| {
            let inode = volume.validate_handle(handle)?;
            volume.copy_file_range(&inode, offset, destination)
        })
    }

    pub fn mmap(&mut self, handle: FileHandle) -> Result<FileMapping, StorageError> {
        self.with_access(|volume| {
            let mut mapping = FileMapping {
                handle,
                bytes: [0; MAX_SMALL_FILE_SIZE],
                length: 0,
            };
            let length = volume.read(handle, &mut mapping.bytes)?;
            mapping.length = length;
            Ok(mapping)
        })
    }

    pub fn flush_mapping(&mut self, mapping: &FileMapping) -> Result<(), StorageError> {
        self.with_access(|volume| volume.write(mapping.handle, mapping.bytes()))
    }

    fn format(&mut self) -> Result<(), StorageError> {
        self.device.write_sector(0, &[0; SECTOR_SIZE])?;
        self.device.flush()?;
        let mut superblock = [0; BLOCK_SIZE];
        write_u32(&mut superblock, 0, EXT2_INODE_COUNT);
        write_u32(&mut superblock, 4, EXT2_BLOCK_COUNT);
        write_u32(&mut superblock, 12, EXT2_BLOCK_COUNT - 15);
        write_u32(&mut superblock, 16, EXT2_INODE_COUNT - 2);
        write_u32(&mut superblock, 20, EXT2_FIRST_DATA_BLOCK);
        write_u32(&mut superblock, 24, 0);
        write_u32(&mut superblock, 32, EXT2_BLOCKS_PER_GROUP);
        write_u32(&mut superblock, 40, EXT2_INODES_PER_GROUP);
        write_u16(&mut superblock, 56, EXT2_MAGIC);
        write_u32(&mut superblock, 76, 1);
        write_u32(&mut superblock, 84, 11);
        write_u16(&mut superblock, 88, EXT2_INODE_SIZE);
        self.write_block(SUPERBLOCK_BLOCK, &superblock)?;

        let mut group = [0; BLOCK_SIZE];
        write_u32(&mut group, 0, BLOCK_BITMAP);
        write_u32(&mut group, 4, INODE_BITMAP);
        write_u32(&mut group, 8, INODE_TABLE);
        write_u16(&mut group, 12, EXT2_BLOCK_COUNT as u16 - 15);
        write_u16(&mut group, 14, EXT2_INODE_COUNT as u16 - 2);
        write_u16(&mut group, 16, 1);
        self.write_block(GROUP_DESCRIPTOR_BLOCK, &group)?;

        let mut block_bitmap = [0; BLOCK_SIZE];
        let mut block = 0;
        while block < 15 {
            set_bit(&mut block_bitmap, block);
            block += 1;
        }
        self.write_block(BLOCK_BITMAP, &block_bitmap)?;

        let mut inode_bitmap = [0; BLOCK_SIZE];
        set_bit(&mut inode_bitmap, 0);
        set_bit(&mut inode_bitmap, 1);
        self.write_block(INODE_BITMAP, &inode_bitmap)?;

        self.write_inode(
            ROOT_INODE,
            InodeInfo {
                mode: 0x4000,
                uid: 0,
                gid: 0,
                size: BLOCK_SIZE as u32,
                atime: 0,
                ctime: 0,
                mtime: 0,
                blocks: 2,
                direct_block: ROOT_DIRECTORY_BLOCK,
                extra_blocks: [0; DIRECT_BLOCKS - 1],
                indirect_block: 0,
                generation: 0,
            },
        )?;

        let mut directory = [0; BLOCK_SIZE];
        let dot = [b'.'];
        let dot_dot = [b'.', b'.'];
        write_directory_record(&mut directory, 0, ROOT_INODE, 12, 1, 2, &dot);
        write_directory_record(&mut directory, 12, ROOT_INODE, 12, 2, 2, &dot_dot);
        write_directory_record(&mut directory, 24, 0, BLOCK_SIZE as u16 - 24, 0, 0, b"");
        self.write_block(ROOT_DIRECTORY_BLOCK, &directory)
    }

    /// Run a complete logical operation under this process local VFS access gate.
    /// Nested calls through the same Vfs remain reentrant; this does not coordinate
    /// other processes or callers that bypass the VFS and use the block device.
    pub fn with_access<T>(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        if self.recovery_required {
            return Err(StorageError::RecoveryRequired);
        }
        if ACTIVE_ACCESS_ID.load(Ordering::Acquire) == self.access_id {
            return operation(self);
        }
        let _gate = VolumeAccess::acquire();
        ACTIVE_ACCESS_ID.store(self.access_id, Ordering::Release);
        let _reset = ResetAccess(self.access_id);
        if load_undo(&mut self.device)?.is_some() {
            self.recovery_required = true;
            return Err(StorageError::RecoveryRequired);
        }
        operation(self)
    }

    fn find_free_bit(
        &mut self,
        bitmap_block: u32,
        start: u32,
        end: u32,
    ) -> Result<u32, StorageError> {
        let mut bitmap = [0; BLOCK_SIZE];
        self.read_block(bitmap_block, &mut bitmap)?;
        (start..end)
            .find(|bit| !is_bit_set(&bitmap, *bit))
            .ok_or(StorageError::Capacity)
    }

    fn set_allocated(&mut self, block: u32, bit: u32) -> Result<(), StorageError> {
        let mut bitmap = [0; BLOCK_SIZE];
        self.read_block(block, &mut bitmap)?;
        if is_bit_set(&bitmap, bit) {
            return Err(StorageError::Corrupt);
        }
        set_bit(&mut bitmap, bit);
        self.write_changed_block(block, &bitmap)
    }

    fn write_changed_block(
        &mut self,
        block: u32,
        bytes: &[u8; BLOCK_SIZE],
    ) -> Result<(), StorageError> {
        let mut previous = [0; BLOCK_SIZE];
        self.read_block(block, &mut previous)?;
        for index in 0..2 {
            let offset = index * SECTOR_SIZE;
            if previous[offset..offset + SECTOR_SIZE] != bytes[offset..offset + SECTOR_SIZE] {
                let sector: &[u8; SECTOR_SIZE] = bytes[offset..offset + SECTOR_SIZE]
                    .try_into()
                    .map_err(|_| StorageError::Corrupt)?;
                self.device
                    .write_sector(u64::from(block) * 2 + index as u64, sector)?;
            }
        }
        Ok(())
    }

    fn transaction<T>(
        &mut self,
        directory_inode: u32,
        action: UndoAction,
        operation: impl FnOnce(&mut Self) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let mut rollback_undo = None;
        let mut operation_started = false;
        let result = (|| {
            // A preceding header-clear flush may have failed after reads began
            // seeing zero. Make that zero durable before reusing the backup.
            self.device.flush()?;
            check_vfs_integrity(&mut self.device)?;
            let directory_block = self.directory_inode(directory_inode)?.direct_block;
            let mut undo = Undo {
                kind: UNDO_RENAME,
                directory_inode,
                directory_block,
                inode: 0,
                data_block: 0,
                previous_inode: [0; EXT2_INODE_SIZE as usize],
                previous_indirect_checksum: [0; 32],
                directory: [0; BLOCK_SIZE],
            };
            self.read_block(directory_block, &mut undo.directory)?;
            match action {
                UndoAction::Create { inode, data_block } => {
                    undo.kind = UNDO_CREATE;
                    undo.inode = inode;
                    undo.data_block = data_block;
                    let table_block = INODE_TABLE + (inode - 1) / 8;
                    let offset = ((inode - 1) % 8) as usize * EXT2_INODE_SIZE as usize;
                    let mut table = [0; BLOCK_SIZE];
                    self.read_block(table_block, &mut table)?;
                    undo.previous_inode
                        .copy_from_slice(&table[offset..offset + EXT2_INODE_SIZE as usize]);
                }
                UndoAction::Rename => {}
                UndoAction::Delete { inode } => {
                    undo.kind = UNDO_DELETE;
                    undo.inode = inode;
                    let table_block = INODE_TABLE + (inode - 1) / 8;
                    let offset = ((inode - 1) % 8) as usize * EXT2_INODE_SIZE as usize;
                    let mut table = [0; BLOCK_SIZE];
                    self.read_block(table_block, &mut table)?;
                    undo.previous_inode
                        .copy_from_slice(&table[offset..offset + EXT2_INODE_SIZE as usize]);
                    let deleted = self.read_inode(inode)?;
                    if deleted.indirect_block != 0 {
                        let mut indirect = [0; BLOCK_SIZE];
                        self.read_block(deleted.indirect_block, &mut indirect)?;
                        undo.previous_indirect_checksum
                            .copy_from_slice(&Sha256::digest(indirect));
                    }
                }
            }
            validate_undo(&mut self.device, &undo, UNDO_FORMAT_V2)?;
            self.write_changed_block(UNDO_BLOCK, &undo.directory)?;
            self.device.flush()?;
            let header = undo.header();
            // Retain the validated undo in memory before attempting to publish
            // its header. It also lets this failed transaction roll back if a
            // header-clear write became visible but its flush failed.
            rollback_undo = Some((undo, header));
            self.device.write_sector(0, &header)?;
            self.device.flush()?;
            operation_started = true;
            let value = operation(self)?;
            self.device.flush()?;
            self.device.write_sector(0, &[0; SECTOR_SIZE])?;
            self.device.flush()?;
            Ok(value)
        })();
        match result {
            Err(error) => {
                if let Some((undo, header)) = rollback_undo {
                    // A prior clear write can be visible even when its flush
                    // failed. Re-publish and flush the validated undo marker
                    // before restoring any data, so a power cut during undo
                    // can always resume through explicit Recovery.
                    let marker_durable = self
                        .device
                        .write_sector(0, &header)
                        .and_then(|()| self.device.flush())
                        .is_ok();
                    let rollback_complete = marker_durable
                        && validate_undo(&mut self.device, &undo, UNDO_FORMAT_V2).is_ok()
                        && self.apply_undo(&undo).is_ok();
                    if !rollback_complete {
                        self.recovery_required = true;
                        return Err(if operation_started {
                            StorageError::Indeterminate
                        } else {
                            error
                        });
                    }
                }
                Err(error)
            }
            Ok(value) => Ok(value),
        }
    }

    fn recover_undo(&mut self) -> Result<(), StorageError> {
        let Some(undo) = load_undo(&mut self.device)? else {
            return Ok(());
        };
        self.apply_undo(&undo)
    }

    fn apply_undo(&mut self, undo: &Undo) -> Result<(), StorageError> {
        if undo.kind == UNDO_DELETE {
            self.restore_deleted_inode(undo)?;
        } else {
            self.write_changed_block(undo.directory_block, &undo.directory)?;
            if undo.kind == UNDO_CREATE {
                let table_block = INODE_TABLE + (undo.inode - 1) / 8;
                let offset = ((undo.inode - 1) % 8) as usize * EXT2_INODE_SIZE as usize;
                let mut table = [0; BLOCK_SIZE];
                self.read_block(table_block, &mut table)?;
                table[offset..offset + EXT2_INODE_SIZE as usize]
                    .copy_from_slice(&undo.previous_inode);
                self.write_changed_block(table_block, &table)?;
                for (block, bit) in [
                    (INODE_BITMAP, undo.inode - 1),
                    (BLOCK_BITMAP, undo.data_block),
                ] {
                    let mut bitmap = [0; BLOCK_SIZE];
                    self.read_block(block, &mut bitmap)?;
                    clear_bit_value(&mut bitmap, bit);
                    self.write_changed_block(block, &bitmap)?;
                }
                let mut bitmap = [0; BLOCK_SIZE];
                self.read_block(BLOCK_BITMAP, &mut bitmap)?;
                let free_blocks = (0..EXT2_BLOCK_COUNT)
                    .filter(|bit| !is_bit_set(&bitmap, *bit))
                    .count() as u32;
                self.read_block(INODE_BITMAP, &mut bitmap)?;
                let free_inodes = (0..EXT2_INODE_COUNT)
                    .filter(|bit| !is_bit_set(&bitmap, *bit))
                    .count() as u32;
                let mut metadata = [0; BLOCK_SIZE];
                self.read_block(SUPERBLOCK_BLOCK, &mut metadata)?;
                write_u32(&mut metadata, 12, free_blocks);
                write_u32(&mut metadata, 16, free_inodes);
                self.write_changed_block(SUPERBLOCK_BLOCK, &metadata)?;
                self.read_block(GROUP_DESCRIPTOR_BLOCK, &mut metadata)?;
                write_u16(&mut metadata, 12, free_blocks as u16);
                write_u16(&mut metadata, 14, free_inodes as u16);
                self.write_changed_block(GROUP_DESCRIPTOR_BLOCK, &metadata)?;
            }
        }
        self.device.flush()?;
        self.device.write_sector(0, &[0; SECTOR_SIZE])?;
        self.device.flush()
    }

    fn restore_deleted_inode(&mut self, undo: &Undo) -> Result<(), StorageError> {
        let (blocks, block_count) =
            deleted_inode_block_list(&mut self.device, &undo.previous_inode)?;
        let inode_table_block = INODE_TABLE + (undo.inode - 1) / 8;
        let inode_offset = ((undo.inode - 1) % 8) as usize * EXT2_INODE_SIZE as usize;

        self.write_changed_block(undo.directory_block, &undo.directory)?;

        let mut inode_table = [0; BLOCK_SIZE];
        self.read_block(inode_table_block, &mut inode_table)?;
        inode_table[inode_offset..inode_offset + EXT2_INODE_SIZE as usize]
            .copy_from_slice(&undo.previous_inode);
        self.write_changed_block(inode_table_block, &inode_table)?;

        let mut block_bitmap = [0; BLOCK_SIZE];
        self.read_block(BLOCK_BITMAP, &mut block_bitmap)?;
        for block in blocks[..block_count].iter().copied() {
            set_bit(&mut block_bitmap, block);
        }
        self.write_changed_block(BLOCK_BITMAP, &block_bitmap)?;

        let mut inode_bitmap = [0; BLOCK_SIZE];
        self.read_block(INODE_BITMAP, &mut inode_bitmap)?;
        set_bit(&mut inode_bitmap, undo.inode - 1);
        self.write_changed_block(INODE_BITMAP, &inode_bitmap)?;

        let free_blocks = (0..EXT2_BLOCK_COUNT)
            .filter(|bit| !is_bit_set(&block_bitmap, *bit))
            .count() as u32;
        let free_inodes = (0..EXT2_INODE_COUNT)
            .filter(|bit| !is_bit_set(&inode_bitmap, *bit))
            .count() as u32;
        let mut metadata = [0; BLOCK_SIZE];
        self.read_block(SUPERBLOCK_BLOCK, &mut metadata)?;
        write_u32(&mut metadata, 12, free_blocks);
        write_u32(&mut metadata, 16, free_inodes);
        self.write_changed_block(SUPERBLOCK_BLOCK, &metadata)?;
        self.read_block(GROUP_DESCRIPTOR_BLOCK, &mut metadata)?;
        write_u16(&mut metadata, 12, free_blocks as u16);
        write_u16(&mut metadata, 14, free_inodes as u16);
        self.write_changed_block(GROUP_DESCRIPTOR_BLOCK, &metadata)
    }

    fn find_inode(&mut self, name: &[u8]) -> Result<Option<u32>, StorageError> {
        self.find_inode_in_directory(ROOT_INODE, name)
    }

    fn find_inode_in_directory(
        &mut self,
        directory_inode: u32,
        name: &[u8],
    ) -> Result<Option<u32>, StorageError> {
        let directory_inode = self.directory_inode(directory_inode)?;
        let mut directory = [0; BLOCK_SIZE];
        self.read_block(directory_inode.direct_block, &mut directory)?;
        let mut offset = 0;
        while offset < BLOCK_SIZE {
            let inode = read_u32(&directory, offset);
            let record_length = usize::from(read_u16(&directory, offset + 4));
            let name_length = usize::from(read_u8(&directory, offset + 6));
            if record_length < 8
                || record_length % 4 != 0
                || offset + record_length > BLOCK_SIZE
                || name_length > record_length - 8
            {
                return Err(StorageError::Corrupt);
            }
            if inode != 0 && names_equal(&directory, offset + 8, name_length, name) {
                return Ok(Some(inode));
            }
            offset += record_length;
        }
        Ok(None)
    }

    fn add_directory_entry_in_directory(
        &mut self,
        directory_inode: u32,
        name: &[u8],
        inode: u32,
        file_type: u8,
    ) -> Result<(), StorageError> {
        let directory_inode = self.directory_inode(directory_inode)?;
        let mut directory = [0; BLOCK_SIZE];
        self.read_block(directory_inode.direct_block, &mut directory)?;
        let required = align4(8 + name.len());
        let mut offset = 0;
        while offset < BLOCK_SIZE {
            let current_inode = read_u32(&directory, offset);
            let record_length = usize::from(read_u16(&directory, offset + 4));
            let name_length = usize::from(read_u8(&directory, offset + 6));
            if record_length < 8
                || record_length % 4 != 0
                || offset + record_length > BLOCK_SIZE
                || name_length > record_length - 8
            {
                return Err(StorageError::Corrupt);
            }
            if current_inode == 0 && record_length >= required {
                let remainder = record_length - required;
                if remainder >= 8 {
                    write_u16(&mut directory, offset + 4, required as u16);
                    write_u32(&mut directory, offset + required, 0);
                    write_u16(&mut directory, offset + required + 4, remainder as u16);
                    write_u8(&mut directory, offset + required + 6, 0);
                    write_u8(&mut directory, offset + required + 7, 0);
                }
                write_u32(&mut directory, offset, inode);
                write_u8(&mut directory, offset + 6, name.len() as u8);
                write_u8(&mut directory, offset + 7, file_type);
                copy_bytes_to_offset(&mut directory, offset + 8, name);
                return self.write_changed_block(directory_inode.direct_block, &directory);
            }
            offset += record_length;
        }
        Err(StorageError::DirectoryFull)
    }

    fn find_directory_entry(&mut self, name: &[u8]) -> Result<Option<(usize, u32)>, StorageError> {
        self.find_directory_entry_in_block(ROOT_DIRECTORY_BLOCK, name)
    }

    fn find_directory_entry_in_block(
        &mut self,
        block: u32,
        name: &[u8],
    ) -> Result<Option<(usize, u32)>, StorageError> {
        let mut directory = [0; BLOCK_SIZE];
        self.read_block(block, &mut directory)?;
        let mut offset = 0;
        while offset < BLOCK_SIZE {
            let inode = read_u32(&directory, offset);
            let record_length = usize::from(read_u16(&directory, offset + 4));
            let name_length = usize::from(read_u8(&directory, offset + 6));
            if record_length < 8
                || record_length % 4 != 0
                || offset + record_length > BLOCK_SIZE
                || name_length > record_length - 8
            {
                return Err(StorageError::Corrupt);
            }
            if inode != 0
                && !is_dot_entry(&directory, offset, name_length)
                && names_equal(&directory, offset + 8, name_length, name)
            {
                return Ok(Some((offset, inode)));
            }
            offset += record_length;
        }
        Ok(None)
    }

    fn resolve_path(&mut self, path: &[u8]) -> Result<u32, StorageError> {
        if path.is_empty() || path.len() > MAX_PATH_LENGTH || path.contains(&0) {
            return Err(if path.len() > MAX_PATH_LENGTH {
                StorageError::NameTooLong
            } else {
                StorageError::InvalidName
            });
        }

        let mut current = ROOT_INODE;
        for component in path.split(|byte| *byte == b'/') {
            if component.is_empty() {
                continue;
            }
            self.directory_inode(current)?;
            if component == b"." {
                continue;
            }
            if component == b".." {
                current = self
                    .find_inode_in_directory(current, component)?
                    .ok_or(StorageError::Corrupt)?;
                continue;
            }
            validate_name(component)?;
            current = self
                .find_inode_in_directory(current, component)?
                .ok_or(StorageError::NotFound)?;
        }
        if path.last() == Some(&b'/') {
            self.directory_inode(current)?;
        }
        Ok(current)
    }

    fn resolve_parent_path<'a>(
        &mut self,
        path: &'a [u8],
        allow_trailing_slash: bool,
    ) -> Result<(u32, &'a [u8]), StorageError> {
        if path.is_empty() || path.len() > MAX_PATH_LENGTH || path.contains(&0) {
            return Err(if path.len() > MAX_PATH_LENGTH {
                StorageError::NameTooLong
            } else {
                StorageError::InvalidName
            });
        }

        let mut end = path.len();
        while end > 0 && path[end - 1] == b'/' {
            end -= 1;
        }
        if end == 0 {
            return Err(StorageError::InvalidName);
        }
        if !allow_trailing_slash && end != path.len() {
            return Err(StorageError::NotDirectory);
        }

        let trimmed = &path[..end];
        let separator = trimmed.iter().rposition(|byte| *byte == b'/');
        let name_start = separator.map_or(0, |index| index + 1);
        let name = &trimmed[name_start..];
        validate_name(name)?;
        if name == b"." || name == b".." {
            return Err(StorageError::InvalidName);
        }
        let parent_path = separator.map_or(&[][..], |index| &trimmed[..index]);
        let parent = if parent_path.is_empty() {
            ROOT_INODE
        } else {
            self.resolve_path(parent_path)?
        };
        self.directory_inode(parent)?;
        Ok((parent, name))
    }

    fn remove_path_with_kind(
        &mut self,
        path: &[u8],
        expected_directory: Option<bool>,
        allow_trailing_slash: bool,
    ) -> Result<(), StorageError> {
        let (parent, name) = self.resolve_parent_path(path, allow_trailing_slash)?;
        let parent_inode = self.directory_inode(parent)?;
        let (directory_offset, inode_number) = self
            .find_directory_entry_in_block(parent_inode.direct_block, name)?
            .ok_or(StorageError::NotFound)?;
        if inode_number == ROOT_INODE {
            return Err(StorageError::InvalidName);
        }

        let inode = self.read_inode(inode_number)?;
        let is_directory = match inode.mode & 0xf000 {
            0x4000 => true,
            0x8000 => false,
            _ => return Err(StorageError::InvalidHandle),
        };
        if let Some(expected_directory) = expected_directory {
            if expected_directory && !is_directory {
                return Err(StorageError::NotDirectory);
            }
            if !expected_directory && is_directory {
                return Err(StorageError::IsDirectory);
            }
        }
        if is_directory && !self.directory_is_empty(inode.direct_block)? {
            return Err(StorageError::DirectoryNotEmpty);
        }

        let mut directory = [0; BLOCK_SIZE];
        self.read_block(parent_inode.direct_block, &mut directory)?;
        write_u32(&mut directory, directory_offset, 0);
        self.transaction(
            parent,
            UndoAction::Delete {
                inode: inode_number,
            },
            |volume| {
                volume.write_block(parent_inode.direct_block, &directory)?;
                volume.clear_bit(INODE_BITMAP, inode_number - 1)?;
                let released = volume.release_inode_blocks(&inode)?;
                volume.clear_inode(inode_number)?;
                volume.adjust_free_counts(released as i32, 1)
            },
        )
    }

    fn directory_inode(&mut self, inode_number: u32) -> Result<InodeInfo, StorageError> {
        let inode = self.read_inode(inode_number)?;
        if inode.mode & 0xf000 != 0x4000 {
            return Err(StorageError::NotDirectory);
        }
        if inode.direct_block == 0 {
            return Err(StorageError::Corrupt);
        }
        Ok(inode)
    }

    fn directory_is_empty(&mut self, block: u32) -> Result<bool, StorageError> {
        let mut directory = [0; BLOCK_SIZE];
        self.read_block(block, &mut directory)?;
        let mut offset = 0;
        while offset < BLOCK_SIZE {
            let inode = read_u32(&directory, offset);
            let record_length = usize::from(read_u16(&directory, offset + 4));
            let name_length = usize::from(read_u8(&directory, offset + 6));
            if record_length < 8
                || record_length % 4 != 0
                || offset + record_length > BLOCK_SIZE
                || name_length > record_length - 8
            {
                return Err(StorageError::Corrupt);
            }
            if inode != 0 && !is_dot_entry(&directory, offset, name_length) {
                return Ok(false);
            }
            offset += record_length;
        }
        Ok(true)
    }

    fn allocate_bit(
        &mut self,
        bitmap_block: u32,
        start: u32,
        end: u32,
    ) -> Result<u32, StorageError> {
        let mut bitmap = [0; BLOCK_SIZE];
        self.read_block(bitmap_block, &mut bitmap)?;
        let mut bit = start;
        while bit < end {
            if !is_bit_set(&bitmap, bit) {
                set_bit(&mut bitmap, bit);
                self.write_block(bitmap_block, &bitmap)?;
                return Ok(if bitmap_block == INODE_BITMAP {
                    bit + 1
                } else {
                    bit
                });
            }
            bit += 1;
        }
        Err(StorageError::Capacity)
    }

    fn clear_bit(&mut self, bitmap_block: u32, bit: u32) -> Result<(), StorageError> {
        let mut bitmap = [0; BLOCK_SIZE];
        self.read_block(bitmap_block, &mut bitmap)?;
        clear_bit_value(&mut bitmap, bit);
        self.write_block(bitmap_block, &bitmap)
    }

    fn adjust_free_counts(&mut self, blocks: i32, inodes: i32) -> Result<(), StorageError> {
        let mut superblock = [0; BLOCK_SIZE];
        self.read_block(SUPERBLOCK_BLOCK, &mut superblock)?;
        let free_blocks = read_u32(&superblock, 12) as i32 + blocks;
        let free_inodes = read_u32(&superblock, 16) as i32 + inodes;
        if free_blocks < 0 || free_inodes < 0 {
            return Err(StorageError::Corrupt);
        }
        write_u32(&mut superblock, 12, free_blocks as u32);
        write_u32(&mut superblock, 16, free_inodes as u32);
        self.write_block(SUPERBLOCK_BLOCK, &superblock)?;

        let mut group = [0; BLOCK_SIZE];
        self.read_block(GROUP_DESCRIPTOR_BLOCK, &mut group)?;
        let group_blocks = read_u16(&group, 12) as i32 + blocks;
        let group_inodes = read_u16(&group, 14) as i32 + inodes;
        if group_blocks < 0 || group_inodes < 0 {
            return Err(StorageError::Corrupt);
        }
        write_u16(&mut group, 12, group_blocks as u16);
        write_u16(&mut group, 14, group_inodes as u16);
        self.write_block(GROUP_DESCRIPTOR_BLOCK, &group)
    }

    /// Block holding data block `index` of a regular file, or 0 when that
    /// block is not allocated. Pointers outside the data area are corrupt.
    fn file_block(&mut self, inode: &InodeInfo, index: usize) -> Result<u32, StorageError> {
        let pointer = match index {
            0 => inode.direct_block,
            1..DIRECT_BLOCKS => inode.extra_blocks[index - 1],
            _ => {
                let slot = index - DIRECT_BLOCKS;
                if slot >= INDIRECT_POINTERS {
                    return Err(StorageError::FileTooLarge);
                }
                if inode.indirect_block == 0 {
                    return Ok(0);
                }
                check_data_block(inode.indirect_block)?;
                let mut table = [0; BLOCK_SIZE];
                self.read_block(inode.indirect_block, &mut table)?;
                read_u32(&table, slot * 4)
            }
        };
        if pointer != 0 {
            check_data_block(pointer)?;
        }
        Ok(pointer)
    }

    /// Grow or shrink a regular file from `current` to `needed` data blocks
    /// (both at least 1). New blocks are zeroed; released blocks are freed.
    /// On an allocation failure every block taken by this call is released
    /// and `inode` is left unchanged.
    fn resize_file_blocks(
        &mut self,
        inode: &mut InodeInfo,
        current: usize,
        needed: usize,
    ) -> Result<(), StorageError> {
        if needed == 0 || current == 0 || needed > DIRECT_BLOCKS + INDIRECT_POINTERS {
            return Err(StorageError::FileTooLarge);
        }
        let original = *inode;
        let mut table = [0; BLOCK_SIZE];
        if inode.indirect_block != 0 {
            check_data_block(inode.indirect_block)?;
            self.read_block(inode.indirect_block, &mut table)?;
        }
        let zero = [0; BLOCK_SIZE];
        let mut allocated_indirect = false;
        if needed > current {
            let mut taken = [0u32; DIRECT_BLOCKS + INDIRECT_POINTERS];
            let mut taken_count = 0;
            let result = (|| -> Result<(), StorageError> {
                if needed > DIRECT_BLOCKS && inode.indirect_block == 0 {
                    let block =
                        self.allocate_bit(BLOCK_BITMAP, FIRST_FILE_BLOCK, EXT2_BLOCK_COUNT)?;
                    inode.indirect_block = block;
                    allocated_indirect = true;
                    table = [0; BLOCK_SIZE];
                }
                for index in current..needed {
                    let block =
                        self.allocate_bit(BLOCK_BITMAP, FIRST_FILE_BLOCK, EXT2_BLOCK_COUNT)?;
                    taken[taken_count] = block;
                    taken_count += 1;
                    self.write_block(block, &zero)?;
                    set_file_pointer(inode, &mut table, index, block);
                }
                if needed > DIRECT_BLOCKS {
                    self.write_block(inode.indirect_block, &table)?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                for block in &taken[..taken_count] {
                    self.clear_bit(BLOCK_BITMAP, *block)?;
                }
                if allocated_indirect {
                    self.clear_bit(BLOCK_BITMAP, inode.indirect_block)?;
                }
                *inode = original;
                return Err(error);
            }
            let added = (needed - current) + usize::from(allocated_indirect);
            self.adjust_free_counts(-(added as i32), 0)?;
        } else if needed < current {
            let mut released = 0usize;
            for index in needed..current {
                let block = self.file_block(inode, index)?;
                if block != 0 {
                    self.clear_bit(BLOCK_BITMAP, block)?;
                    released += 1;
                }
                set_file_pointer(inode, &mut table, index, 0);
            }
            if needed <= DIRECT_BLOCKS && inode.indirect_block != 0 {
                self.clear_bit(BLOCK_BITMAP, inode.indirect_block)?;
                inode.indirect_block = 0;
                released += 1;
            } else if inode.indirect_block != 0 {
                self.write_block(inode.indirect_block, &table)?;
            }
            self.adjust_free_counts(released as i32, 0)?;
        }
        inode.blocks = 2 * (needed + usize::from(inode.indirect_block != 0)) as u32;
        Ok(())
    }

    /// Release the blocks of an inode being deleted: every data block of a
    /// regular file, or a directory's single block.
    fn release_inode_blocks(&mut self, inode: &InodeInfo) -> Result<u32, StorageError> {
        if inode.mode & 0xf000 == 0x8000 {
            self.free_file_blocks(inode)
        } else {
            self.clear_bit(BLOCK_BITMAP, inode.direct_block)?;
            Ok(1)
        }
    }

    /// Release every data block (and the indirect block) of a regular file.
    /// Returns the number of blocks freed.
    fn free_file_blocks(&mut self, inode: &InodeInfo) -> Result<u32, StorageError> {
        let size = usize::try_from(inode.size).map_err(|_| StorageError::Corrupt)?;
        let count = data_blocks_for_size(size).min(DIRECT_BLOCKS + INDIRECT_POINTERS);
        let mut released = 0u32;
        for index in 0..count {
            let block = self.file_block(inode, index)?;
            if block != 0 {
                self.clear_bit(BLOCK_BITMAP, block)?;
                released += 1;
            }
        }
        if inode.indirect_block != 0 {
            self.clear_bit(BLOCK_BITMAP, inode.indirect_block)?;
            released += 1;
        }
        Ok(released)
    }

    /// Copy file bytes starting at `offset`; returns the count copied.
    fn copy_file_range(
        &mut self,
        inode: &InodeInfo,
        offset: usize,
        destination: &mut [u8],
    ) -> Result<usize, StorageError> {
        let size = usize::try_from(inode.size).map_err(|_| StorageError::Corrupt)?;
        if size > MAX_FILE_SIZE {
            return Err(StorageError::Corrupt);
        }
        if offset >= size {
            return Ok(0);
        }
        let count = destination.len().min(size - offset);
        let mut copied = 0;
        let mut block = [0; BLOCK_SIZE];
        while copied < count {
            let position = offset + copied;
            let index = position / BLOCK_SIZE;
            let within = position % BLOCK_SIZE;
            let chunk = (BLOCK_SIZE - within).min(count - copied);
            let pointer = self.file_block(inode, index)?;
            if pointer == 0 {
                return Err(StorageError::Corrupt);
            }
            self.read_block(pointer, &mut block)?;
            destination[copied..copied + chunk].copy_from_slice(&block[within..within + chunk]);
            copied += chunk;
        }
        Ok(count)
    }

    /// Write `data` at `offset` into already-allocated blocks.
    fn store_file_range(
        &mut self,
        inode: &InodeInfo,
        offset: usize,
        data: &[u8],
    ) -> Result<(), StorageError> {
        let mut written = 0;
        let mut block = [0; BLOCK_SIZE];
        while written < data.len() {
            let position = offset + written;
            let index = position / BLOCK_SIZE;
            let within = position % BLOCK_SIZE;
            let chunk = (BLOCK_SIZE - within).min(data.len() - written);
            let pointer = self.file_block(inode, index)?;
            if pointer == 0 {
                return Err(StorageError::Corrupt);
            }
            if chunk == BLOCK_SIZE {
                block.copy_from_slice(&data[written..written + chunk]);
            } else {
                self.read_block(pointer, &mut block)?;
                block[within..within + chunk].copy_from_slice(&data[written..written + chunk]);
            }
            self.write_block(pointer, &block)?;
            written += chunk;
        }
        Ok(())
    }

    /// Zero a file range without changing its length. This is needed for
    /// gaps in blocks that were already allocated by older on-disk versions.
    fn zero_file_range(
        &mut self,
        inode: &InodeInfo,
        start: usize,
        end: usize,
    ) -> Result<(), StorageError> {
        let zeros = [0; BLOCK_SIZE];
        let mut cursor = start;
        while cursor < end {
            let chunk = (BLOCK_SIZE - cursor % BLOCK_SIZE).min(end - cursor);
            self.store_file_range(inode, cursor, &zeros[..chunk])?;
            cursor += chunk;
        }
        Ok(())
    }

    /// Zero the bytes at and after `end` in the block that contains the new
    /// end of the file, keeping the invariant that bytes past the end of a
    /// file read as zero. Blocks wholly past `end` are released by resizing.
    fn clear_past_end(&mut self, inode: &InodeInfo, end: usize) -> Result<(), StorageError> {
        let within = end % BLOCK_SIZE;
        if end != 0 && within == 0 {
            return Ok(());
        }
        let pointer = self.file_block(inode, end / BLOCK_SIZE)?;
        if pointer == 0 {
            return Ok(());
        }
        let mut block = [0; BLOCK_SIZE];
        self.read_block(pointer, &mut block)?;
        block[within..].fill(0);
        self.write_block(pointer, &block)
    }

    fn clear_inode(&mut self, inode: u32) -> Result<(), StorageError> {
        // Keep the generation so the next file in this slot gets a new one.
        let generation = self.read_inode(inode)?.generation;
        self.write_inode(
            inode,
            InodeInfo {
                mode: 0,
                uid: 0,
                gid: 0,
                size: 0,
                atime: 0,
                ctime: 0,
                mtime: 0,
                blocks: 0,
                direct_block: 0,
                extra_blocks: [0; DIRECT_BLOCKS - 1],
                indirect_block: 0,
                generation,
            },
        )
    }

    /// Generation for a file about to occupy inode slot `inode`: always
    /// greater than any generation previously issued for that slot.
    fn next_generation(&mut self, inode: u32) -> Result<u32, StorageError> {
        let previous = self.read_inode(inode)?.generation;
        Ok(effective_generation(previous).wrapping_add(1).max(2))
    }

    /// Issue a handle for an existing regular-file inode.
    fn handle_for(&mut self, inode_number: u32) -> Result<FileHandle, StorageError> {
        let inode = self.read_inode(inode_number)?;
        if inode.mode & 0xf000 != 0x8000 || inode.direct_block == 0 {
            return Err(StorageError::InvalidHandle);
        }
        Ok(FileHandle {
            inode: inode_number,
            generation: effective_generation(inode.generation),
        })
    }

    fn validate_handle(&mut self, handle: FileHandle) -> Result<InodeInfo, StorageError> {
        if handle.generation == 0
            || handle.inode < FIRST_FILE_INODE
            || handle.inode > EXT2_INODE_COUNT
        {
            return Err(StorageError::InvalidHandle);
        }
        let inode = self.read_inode(handle.inode)?;
        if inode.mode & 0xf000 != 0x8000
            || inode.direct_block == 0
            || effective_generation(inode.generation) != handle.generation
        {
            return Err(StorageError::InvalidHandle);
        }
        Ok(inode)
    }

    fn read_inode(&mut self, inode: u32) -> Result<InodeInfo, StorageError> {
        if inode == 0 || inode > EXT2_INODE_COUNT {
            return Err(StorageError::InvalidHandle);
        }
        let block = INODE_TABLE + (inode - 1) / 8;
        let offset = ((inode - 1) % 8) as usize * EXT2_INODE_SIZE as usize;
        let mut bytes = [0; BLOCK_SIZE];
        self.read_block(block, &mut bytes)?;
        Ok(InodeInfo {
            mode: read_u16(&bytes, offset),
            uid: read_u16(&bytes, offset + 2),
            size: read_u32(&bytes, offset + 4),
            atime: read_u32(&bytes, offset + 8),
            ctime: read_u32(&bytes, offset + 12),
            mtime: read_u32(&bytes, offset + 16),
            gid: read_u16(&bytes, offset + 24),
            blocks: read_u32(&bytes, offset + 28),
            direct_block: read_u32(&bytes, offset + 40),
            extra_blocks: read_extra_blocks(&bytes, offset),
            indirect_block: read_u32(&bytes, offset + 40 + DIRECT_BLOCKS * 4),
            generation: read_u32(&bytes, offset + 100),
        })
    }

    fn write_inode(&mut self, inode: u32, info: InodeInfo) -> Result<(), StorageError> {
        if inode == 0 || inode > EXT2_INODE_COUNT {
            return Err(StorageError::InvalidHandle);
        }
        let block = INODE_TABLE + (inode - 1) / 8;
        let offset = ((inode - 1) % 8) as usize * EXT2_INODE_SIZE as usize;
        let mut bytes = [0; BLOCK_SIZE];
        self.read_block(block, &mut bytes)?;
        write_u16(&mut bytes, offset, info.mode);
        write_u16(&mut bytes, offset + 2, info.uid);
        write_u32(&mut bytes, offset + 4, info.size);
        write_u32(&mut bytes, offset + 8, info.atime);
        write_u32(&mut bytes, offset + 12, info.ctime);
        write_u32(&mut bytes, offset + 16, info.mtime);
        write_u16(&mut bytes, offset + 24, info.gid);
        write_u32(&mut bytes, offset + 28, info.blocks);
        write_u32(&mut bytes, offset + 40, info.direct_block);
        for (index, pointer) in info.extra_blocks.iter().enumerate() {
            write_u32(&mut bytes, offset + 44 + index * 4, *pointer);
        }
        write_u32(
            &mut bytes,
            offset + 40 + DIRECT_BLOCKS * 4,
            info.indirect_block,
        );
        write_u32(&mut bytes, offset + 100, info.generation);
        self.write_block(block, &bytes)
    }

    fn read_block(
        &mut self,
        block: u32,
        destination: &mut [u8; BLOCK_SIZE],
    ) -> Result<(), StorageError> {
        let sector = u64::from(block).checked_mul(2).ok_or(StorageError::Block)?;
        let (first, second) = destination.split_at_mut(SECTOR_SIZE);
        let first: &mut [u8; SECTOR_SIZE] = first.try_into().map_err(|_| StorageError::Block)?;
        let second: &mut [u8; SECTOR_SIZE] = second.try_into().map_err(|_| StorageError::Block)?;
        self.device.read_sector(sector, first)?;
        self.device.read_sector(sector + 1, second)
    }

    fn write_block(&mut self, block: u32, source: &[u8; BLOCK_SIZE]) -> Result<(), StorageError> {
        let sector = u64::from(block).checked_mul(2).ok_or(StorageError::Block)?;
        let (first, second) = source.split_at(SECTOR_SIZE);
        let first: &[u8; SECTOR_SIZE] = first.try_into().map_err(|_| StorageError::Block)?;
        let second: &[u8; SECTOR_SIZE] = second.try_into().map_err(|_| StorageError::Block)?;
        self.device.write_sector(sector, first)?;
        self.device.write_sector(sector + 1, second)
    }
}

impl<D: ReadOnlyBlockDevice> Vfs<D> {
    /// Inspect an existing volume without formatting, writing, or flushing it.
    /// Invalid or unsupported on-disk state returns `StorageError::Corrupt`.
    pub fn check_existing(device: &mut D) -> Result<VfsIntegrityReport, StorageError> {
        let _access = VolumeAccess::acquire();
        if load_undo(device)?.is_some() {
            return Err(StorageError::RecoveryRequired);
        }
        check_vfs_integrity(device)
    }
}

impl Undo {
    fn header(&self) -> [u8; SECTOR_SIZE] {
        let mut header = [0; SECTOR_SIZE];
        header[..8].copy_from_slice(UNDO_MAGIC);
        header[8] = self.kind;
        write_u32(&mut header, 12, self.directory_inode);
        write_u32(&mut header, 16, self.directory_block);
        write_u32(&mut header, 20, self.inode);
        write_u32(&mut header, 24, self.data_block);
        header[32..64].copy_from_slice(&Sha256::digest(self.directory));
        header[64..192].copy_from_slice(&self.previous_inode);
        header[192..224].copy_from_slice(&self.previous_indirect_checksum);
        let digest = Sha256::digest(&header[..480]);
        header[480..].copy_from_slice(&digest);
        header
    }
}

fn load_undo<D: ReadOnlyBlockDevice>(device: &mut D) -> Result<Option<Undo>, StorageError> {
    let mut header = [0; SECTOR_SIZE];
    device.read_sector(0, &mut header)?;
    if header.iter().all(|byte| *byte == 0) {
        return Ok(None);
    }
    // Zero is the only supported legacy header. Unknown boot/reserved bytes
    // are never overwritten, interpreted as authority, or auto-formatted.
    let format_version = if &header[..8] == UNDO_MAGIC {
        UNDO_FORMAT_V2
    } else if &header[..8] == LEGACY_UNDO_MAGIC {
        UNDO_FORMAT_V1
    } else {
        return Err(StorageError::Corrupt);
    };
    if Sha256::digest(&header[..480])[..] != header[480..] {
        return Err(StorageError::Corrupt);
    }
    let mut undo = Undo {
        kind: header[8],
        directory_inode: read_u32(&header, 12),
        directory_block: read_u32(&header, 16),
        inode: read_u32(&header, 20),
        data_block: read_u32(&header, 24),
        previous_inode: [0; EXT2_INODE_SIZE as usize],
        previous_indirect_checksum: [0; 32],
        directory: [0; BLOCK_SIZE],
    };
    undo.previous_inode.copy_from_slice(&header[64..192]);
    undo.previous_indirect_checksum
        .copy_from_slice(&header[192..224]);
    read_device_block(device, UNDO_BLOCK, &mut undo.directory)?;
    if Sha256::digest(undo.directory)[..] != header[32..64] {
        return Err(StorageError::Corrupt);
    }
    validate_undo(device, &undo, format_version)?;
    Ok(Some(undo))
}

const MAX_DELETE_BLOCKS: usize = DIRECT_BLOCKS + INDIRECT_POINTERS + 1;

fn deleted_inode_info(raw: &[u8; EXT2_INODE_SIZE as usize]) -> InodeInfo {
    let mut extra_blocks = [0; DIRECT_BLOCKS - 1];
    for (index, block) in extra_blocks.iter_mut().enumerate() {
        *block = read_u32(raw, 44 + index * 4);
    }
    InodeInfo {
        mode: read_u16(raw, 0),
        uid: read_u16(raw, 2),
        gid: read_u16(raw, 24),
        size: read_u32(raw, 4),
        atime: read_u32(raw, 8),
        ctime: read_u32(raw, 12),
        mtime: read_u32(raw, 16),
        blocks: read_u32(raw, 28),
        direct_block: read_u32(raw, 40),
        extra_blocks,
        indirect_block: read_u32(raw, 40 + DIRECT_BLOCKS * 4),
        generation: read_u32(raw, 100),
    }
}

fn deleted_inode_block_list<D: ReadOnlyBlockDevice>(
    device: &mut D,
    raw: &[u8; EXT2_INODE_SIZE as usize],
) -> Result<([u32; MAX_DELETE_BLOCKS], usize), StorageError> {
    let inode = deleted_inode_info(raw);
    let mut blocks = [0; MAX_DELETE_BLOCKS];
    let mut count = 0;
    let mut add = |block: u32| -> Result<(), StorageError> {
        if block == 0 {
            return Err(StorageError::Corrupt);
        }
        check_data_block(block)?;
        if blocks[..count].contains(&block) {
            return Err(StorageError::Corrupt);
        }
        blocks[count] = block;
        count += 1;
        Ok(())
    };

    match inode.mode & 0xf000 {
        0x8000 => {
            let size = usize::try_from(inode.size).map_err(|_| StorageError::Corrupt)?;
            if size > MAX_FILE_SIZE || inode.direct_block == 0 {
                return Err(StorageError::Corrupt);
            }
            let data_count = data_blocks_for_size(size).min(DIRECT_BLOCKS + INDIRECT_POINTERS);
            let needs_indirect = data_count > DIRECT_BLOCKS;
            if needs_indirect != (inode.indirect_block != 0) {
                return Err(StorageError::Corrupt);
            }
            for index in data_count..DIRECT_BLOCKS {
                let pointer = if index == 0 {
                    inode.direct_block
                } else {
                    inode.extra_blocks[index - 1]
                };
                if pointer != 0 {
                    return Err(StorageError::Corrupt);
                }
            }

            let mut indirect = [0; BLOCK_SIZE];
            if inode.indirect_block != 0 {
                check_data_block(inode.indirect_block)?;
                read_device_block(device, inode.indirect_block, &mut indirect)?;
            }
            for index in 0..data_count {
                let pointer = match index {
                    0 => inode.direct_block,
                    1..DIRECT_BLOCKS => inode.extra_blocks[index - 1],
                    _ => read_u32(&indirect, (index - DIRECT_BLOCKS) * 4),
                };
                add(pointer)?;
            }
            if inode.indirect_block != 0 {
                add(inode.indirect_block)?;
            }
            let indirect_block_count = if inode.indirect_block == 0 { 0 } else { 1 };
            let expected_sectors = 2 * (data_count + indirect_block_count) as u32;
            if inode.blocks != expected_sectors && !(inode.blocks == 0 && size == 0) {
                return Err(StorageError::Corrupt);
            }
        }
        0x4000 => {
            if inode.size != BLOCK_SIZE as u32
                || inode.blocks != 2
                || inode.direct_block == 0
                || inode.indirect_block != 0
                || inode.extra_blocks.iter().any(|block| *block != 0)
            {
                return Err(StorageError::Corrupt);
            }
            add(inode.direct_block)?;
        }
        _ => return Err(StorageError::Corrupt),
    }
    Ok((blocks, count))
}

fn read_inode_bytes_from_device<D: ReadOnlyBlockDevice>(
    device: &mut D,
    inode: u32,
) -> Result<[u8; EXT2_INODE_SIZE as usize], StorageError> {
    if inode == 0 || inode > EXT2_INODE_COUNT {
        return Err(StorageError::Corrupt);
    }
    let block = INODE_TABLE + (inode - 1) / 8;
    let offset = ((inode - 1) % 8) as usize * EXT2_INODE_SIZE as usize;
    let mut table = [0; BLOCK_SIZE];
    read_device_block(device, block, &mut table)?;
    let mut raw = [0; EXT2_INODE_SIZE as usize];
    raw.copy_from_slice(&table[offset..offset + EXT2_INODE_SIZE as usize]);
    Ok(raw)
}

fn delete_entry_offset(
    directory: &[u8; BLOCK_SIZE],
    deleted_inode: u32,
) -> Result<usize, StorageError> {
    let mut offset = 0;
    let mut found = None;
    while offset < BLOCK_SIZE {
        let inode = read_u32(directory, offset);
        let length = usize::from(read_u16(directory, offset + 4));
        if inode == deleted_inode {
            if found.is_some() {
                return Err(StorageError::Corrupt);
            }
            found = Some(offset);
        }
        offset += length;
    }
    found.ok_or(StorageError::Corrupt)
}

// clear_inode may reach stable storage before the inode-bitmap update on a
// cached device. Under the atomic-sector assumption, that sector is either the
// prior inode or exactly the cleared record with its generation retained.
fn cleared_inode_matches(
    current: &[u8; EXT2_INODE_SIZE as usize],
    previous: &[u8; EXT2_INODE_SIZE as usize],
) -> bool {
    let mut cleared = *previous;
    write_u16(&mut cleared, 0, 0);
    write_u16(&mut cleared, 2, 0);
    write_u32(&mut cleared, 4, 0);
    write_u32(&mut cleared, 8, 0);
    write_u32(&mut cleared, 12, 0);
    write_u32(&mut cleared, 16, 0);
    write_u16(&mut cleared, 24, 0);
    write_u32(&mut cleared, 28, 0);
    write_u32(&mut cleared, 40, 0);
    for index in 0..(DIRECT_BLOCKS - 1) {
        write_u32(&mut cleared, 44 + index * 4, 0);
    }
    write_u32(&mut cleared, 40 + DIRECT_BLOCKS * 4, 0);
    cleared == *current
}

fn validate_undo<D: ReadOnlyBlockDevice>(
    device: &mut D,
    undo: &Undo,
    format_version: u8,
) -> Result<(), StorageError> {
    let kind_supported = match format_version {
        UNDO_FORMAT_V1 => matches!(undo.kind, UNDO_CREATE | UNDO_RENAME),
        UNDO_FORMAT_V2 => matches!(undo.kind, UNDO_CREATE | UNDO_RENAME | UNDO_DELETE),
        _ => false,
    };
    if !kind_supported
        || !(ROOT_INODE..=EXT2_INODE_COUNT).contains(&undo.directory_inode)
        || (undo.directory_inode == ROOT_INODE && undo.directory_block != ROOT_DIRECTORY_BLOCK)
        || (undo.directory_inode != ROOT_INODE
            && !(RESERVED_BLOCKS..EXT2_BLOCK_COUNT).contains(&undo.directory_block))
    {
        return Err(StorageError::Corrupt);
    }
    if undo.kind == UNDO_CREATE {
        if !(FIRST_FILE_INODE..=EXT2_INODE_COUNT).contains(&undo.inode)
            || undo.inode == undo.directory_inode
            || !(RESERVED_BLOCKS..EXT2_BLOCK_COUNT).contains(&undo.data_block)
            || undo.data_block == undo.directory_block
            || undo.previous_indirect_checksum != [0; 32]
        {
            return Err(StorageError::Corrupt);
        }
    } else if undo.kind == UNDO_RENAME {
        if undo.inode != 0
            || undo.data_block != 0
            || undo.previous_inode.iter().any(|byte| *byte != 0)
            || undo.previous_indirect_checksum != [0; 32]
        {
            return Err(StorageError::Corrupt);
        }
    } else if undo.kind == UNDO_DELETE
        && (!(FIRST_FILE_INODE..=EXT2_INODE_COUNT).contains(&undo.inode)
            || undo.inode == undo.directory_inode
            || undo.data_block != 0)
    {
        return Err(StorageError::Corrupt);
    }

    let mut superblock = [0; BLOCK_SIZE];
    read_device_block(device, SUPERBLOCK_BLOCK, &mut superblock)?;
    if read_u16(&superblock, 56) != EXT2_MAGIC {
        return Err(StorageError::Corrupt);
    }
    validate_superblock(&superblock)?;
    let mut group = [0; BLOCK_SIZE];
    read_device_block(device, GROUP_DESCRIPTOR_BLOCK, &mut group)?;
    if read_u32(&group, 0) != BLOCK_BITMAP
        || read_u32(&group, 4) != INODE_BITMAP
        || read_u32(&group, 8) != INODE_TABLE
    {
        return Err(StorageError::Corrupt);
    }
    let mut block_bitmap = [0; BLOCK_SIZE];
    let mut inode_bitmap = [0; BLOCK_SIZE];
    read_device_block(device, BLOCK_BITMAP, &mut block_bitmap)?;
    read_device_block(device, INODE_BITMAP, &mut inode_bitmap)?;
    if (0..RESERVED_BLOCKS).any(|bit| !is_bit_set(&block_bitmap, bit))
        || !is_bit_set(&block_bitmap, undo.directory_block)
        || !is_bit_set(&inode_bitmap, ROOT_INODE - 1)
        || !is_bit_set(&inode_bitmap, undo.directory_inode - 1)
    {
        return Err(StorageError::Corrupt);
    }
    let root = read_inode_from_device(device, ROOT_INODE)?;
    if root.mode & 0xf000 != 0x4000
        || root.size != BLOCK_SIZE as u32
        || root.blocks != 2
        || root.direct_block != ROOT_DIRECTORY_BLOCK
        || root.indirect_block != 0
        || root.extra_blocks.iter().any(|pointer| *pointer != 0)
    {
        return Err(StorageError::Corrupt);
    }
    let parent = read_inode_from_device(device, undo.directory_inode)?;
    if parent.mode & 0xf000 != 0x4000
        || parent.direct_block != undo.directory_block
        || parent.size != BLOCK_SIZE as u32
        || parent.blocks != 2
        || parent.indirect_block != 0
        || parent.extra_blocks.iter().any(|pointer| *pointer != 0)
    {
        return Err(StorageError::Corrupt);
    }
    validate_undo_directory_block_owner(
        device,
        &inode_bitmap,
        undo.directory_inode,
        undo.directory_block,
    )?;

    let forbidden_inode = if undo.kind == UNDO_DELETE {
        0
    } else {
        undo.inode
    };
    validate_undo_directory(&undo.directory, undo.directory_inode, forbidden_inode)?;

    if undo.kind == UNDO_CREATE {
        // Neither a forged/corrupt journal nor an overlapping writer may free
        // storage owned by another allocated inode or directory entry.
        for inode in ROOT_INODE..=EXT2_INODE_COUNT {
            if inode == undo.inode || !is_bit_set(&inode_bitmap, inode - 1) {
                continue;
            }
            let info = read_inode_from_device(device, inode)?;
            if info.direct_block == undo.data_block
                || info.extra_blocks.contains(&undo.data_block)
                || info.indirect_block == undo.data_block
            {
                return Err(StorageError::Corrupt);
            }
            if info.mode & 0xf000 == 0x8000 && info.indirect_block != 0 {
                check_data_block(info.indirect_block)?;
                let mut table = [0; BLOCK_SIZE];
                read_device_block(device, info.indirect_block, &mut table)?;
                if (0..INDIRECT_POINTERS)
                    .any(|index| read_u32(&table, index * 4) == undo.data_block)
                {
                    return Err(StorageError::Corrupt);
                }
            }
            if info.mode & 0xf000 == 0x4000 && inode != undo.directory_inode {
                if inode == ROOT_INODE {
                    if info.size != BLOCK_SIZE as u32
                        || info.blocks != 2
                        || info.direct_block != ROOT_DIRECTORY_BLOCK
                        || info.indirect_block != 0
                        || info.extra_blocks.iter().any(|pointer| *pointer != 0)
                    {
                        return Err(StorageError::Corrupt);
                    }
                } else {
                    check_data_block(info.direct_block)?;
                }
                let mut directory = [0; BLOCK_SIZE];
                read_device_block(device, info.direct_block, &mut directory)?;
                validate_undo_directory(&directory, inode, undo.inode)?;
            }
        }
    } else if undo.kind == UNDO_DELETE {
        let (deleted_blocks, deleted_block_count) =
            deleted_inode_block_list(device, &undo.previous_inode)?;
        let deleted_info = deleted_inode_info(&undo.previous_inode);
        if deleted_info.indirect_block == 0 {
            if undo.previous_indirect_checksum != [0; 32] {
                return Err(StorageError::Corrupt);
            }
        } else {
            let mut indirect = [0; BLOCK_SIZE];
            read_device_block(device, deleted_info.indirect_block, &mut indirect)?;
            let indirect_checksum = Sha256::digest(indirect);
            if indirect_checksum[..] != undo.previous_indirect_checksum[..] {
                return Err(StorageError::Corrupt);
            }
        }

        let entry_offset = delete_entry_offset(&undo.directory, undo.inode)?;
        let mut removed_directory = undo.directory;
        write_u32(&mut removed_directory, entry_offset, 0);
        let mut current_directory = [0; BLOCK_SIZE];
        read_device_block(device, undo.directory_block, &mut current_directory)?;
        for start in (0..BLOCK_SIZE).step_by(SECTOR_SIZE) {
            let end = start + SECTOR_SIZE;
            let current = &current_directory[start..end];
            let original = &undo.directory[start..end];
            let removed = &removed_directory[start..end];
            if current != original && current != removed {
                return Err(StorageError::Corrupt);
            }
        }

        if is_bit_set(&inode_bitmap, undo.inode - 1) {
            let current_inode = read_inode_bytes_from_device(device, undo.inode)?;
            if current_inode != undo.previous_inode
                && !cleared_inode_matches(&current_inode, &undo.previous_inode)
            {
                return Err(StorageError::Corrupt);
            }
        }

        // A pending delete may restore only blocks that no other live inode
        // has claimed. The marker blocks normal allocation on this process.
        for inode in ROOT_INODE..=EXT2_INODE_COUNT {
            if inode == undo.inode || !is_bit_set(&inode_bitmap, inode - 1) {
                continue;
            }
            let info = read_inode_from_device(device, inode)?;
            let references_deleted_block = deleted_blocks[..deleted_block_count]
                .contains(&info.direct_block)
                || info.extra_blocks.iter().any(|block| {
                    *block != 0 && deleted_blocks[..deleted_block_count].contains(block)
                })
                || (info.indirect_block != 0
                    && deleted_blocks[..deleted_block_count].contains(&info.indirect_block));
            if references_deleted_block {
                return Err(StorageError::Corrupt);
            }
            if info.mode & 0xf000 == 0x8000 && info.indirect_block != 0 {
                check_data_block(info.indirect_block)?;
                let mut table = [0; BLOCK_SIZE];
                read_device_block(device, info.indirect_block, &mut table)?;
                if (0..INDIRECT_POINTERS).any(|index| {
                    deleted_blocks[..deleted_block_count].contains(&read_u32(&table, index * 4))
                }) {
                    return Err(StorageError::Corrupt);
                }
            }
            if info.mode & 0xf000 == 0x4000 && inode != undo.directory_inode {
                if inode == ROOT_INODE {
                    if info.size != BLOCK_SIZE as u32
                        || info.blocks != 2
                        || info.direct_block != ROOT_DIRECTORY_BLOCK
                        || info.indirect_block != 0
                        || info.extra_blocks.iter().any(|pointer| *pointer != 0)
                    {
                        return Err(StorageError::Corrupt);
                    }
                } else {
                    check_data_block(info.direct_block)?;
                }
                let mut directory = [0; BLOCK_SIZE];
                read_device_block(device, info.direct_block, &mut directory)?;
                validate_undo_directory(&directory, inode, undo.inode)?;
            }
        }
    }
    Ok(())
}

fn validate_undo_directory_block_owner<D: ReadOnlyBlockDevice>(
    device: &mut D,
    inode_bitmap: &[u8; BLOCK_SIZE],
    parent_inode: u32,
    parent_block: u32,
) -> Result<(), StorageError> {
    for inode in ROOT_INODE..=EXT2_INODE_COUNT {
        if inode == parent_inode || !is_bit_set(inode_bitmap, inode - 1) {
            continue;
        }
        let info = read_inode_from_device(device, inode)?;
        if info.direct_block == parent_block
            || info.extra_blocks.contains(&parent_block)
            || info.indirect_block == parent_block
        {
            return Err(StorageError::Corrupt);
        }
        if info.indirect_block != 0 {
            check_data_block(info.indirect_block)?;
            let mut table = [0; BLOCK_SIZE];
            read_device_block(device, info.indirect_block, &mut table)?;
            if (0..INDIRECT_POINTERS).any(|index| read_u32(&table, index * 4) == parent_block) {
                return Err(StorageError::Corrupt);
            }
        }
    }
    Ok(())
}

fn validate_undo_directory(
    directory: &[u8; BLOCK_SIZE],
    inode: u32,
    forbidden_inode: u32,
) -> Result<(), StorageError> {
    let mut offset = 0;
    let mut dot = 0;
    let mut dot_dot = 0;
    while offset < BLOCK_SIZE {
        let target = read_u32(directory, offset);
        let length = usize::from(read_u16(directory, offset + 4));
        let name_length = usize::from(directory[offset + 6]);
        if length < 8
            || length % 4 != 0
            || offset + length > BLOCK_SIZE
            || name_length > length - 8
            || name_length > MAX_NAME_LENGTH
        {
            return Err(StorageError::Corrupt);
        }
        if target != 0 {
            if target > EXT2_INODE_COUNT || target == forbidden_inode {
                return Err(StorageError::Corrupt);
            }
            let name = &directory[offset + 8..offset + 8 + name_length];
            if name == b"." {
                if target != inode || directory[offset + 7] != 2 {
                    return Err(StorageError::Corrupt);
                }
                dot += 1;
            } else if name == b".." {
                if target < ROOT_INODE || directory[offset + 7] != 2 {
                    return Err(StorageError::Corrupt);
                }
                dot_dot += 1;
            } else {
                validate_name(name)?;
            }
        }
        offset += length;
    }
    if dot != 1 || dot_dot != 1 {
        return Err(StorageError::Corrupt);
    }
    Ok(())
}

fn metadata_from_inode(inode: u32, info: InodeInfo) -> FileMetadata {
    FileMetadata {
        inode,
        generation: effective_generation(info.generation),
        mode: info.mode,
        uid: info.uid,
        gid: info.gid,
        size: info.size,
        atime: info.atime,
        ctime: info.ctime,
        mtime: info.mtime,
        blocks: info.blocks,
    }
}

fn block_read(capability: u64, sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> bool {
    crate::block_read(capability, sector, buffer)
}

fn block_write(capability: u64, sector: u64, buffer: &[u8; SECTOR_SIZE]) -> bool {
    crate::block_write(capability, sector, buffer)
}

fn block_flush(capability: u64) -> bool {
    crate::block_flush(capability)
}

fn current_timestamp() -> u32 {
    #[cfg(target_os = "nagi")]
    {
        return crate::time_realtime_ns()
            .map(|nanoseconds| u32::try_from(nanoseconds / 1_000_000_000).unwrap_or(u32::MAX))
            .unwrap_or(0);
    }
    #[cfg(not(target_os = "nagi"))]
    {
        0
    }
}

fn validate_superblock(superblock: &[u8; BLOCK_SIZE]) -> Result<(), StorageError> {
    if read_u32(superblock, 0) != EXT2_INODE_COUNT
        || read_u32(superblock, 4) != EXT2_BLOCK_COUNT
        || read_u32(superblock, 20) != EXT2_FIRST_DATA_BLOCK
        || read_u32(superblock, 24) != 0
        || read_u32(superblock, 32) != EXT2_BLOCKS_PER_GROUP
        || read_u32(superblock, 40) != EXT2_INODES_PER_GROUP
        || read_u16(superblock, 88) != EXT2_INODE_SIZE
    {
        return Err(StorageError::Corrupt);
    }
    Ok(())
}

fn check_vfs_integrity<D: ReadOnlyBlockDevice>(
    device: &mut D,
) -> Result<VfsIntegrityReport, StorageError> {
    let mut superblock = [0; BLOCK_SIZE];
    read_device_block(device, SUPERBLOCK_BLOCK, &mut superblock)?;
    if read_u16(&superblock, 56) != EXT2_MAGIC {
        return Err(StorageError::Corrupt);
    }
    validate_superblock(&superblock)?;

    let mut group = [0; BLOCK_SIZE];
    read_device_block(device, GROUP_DESCRIPTOR_BLOCK, &mut group)?;
    if read_u32(&group, 0) != BLOCK_BITMAP
        || read_u32(&group, 4) != INODE_BITMAP
        || read_u32(&group, 8) != INODE_TABLE
    {
        return Err(StorageError::Corrupt);
    }

    let mut block_bitmap = [0; BLOCK_SIZE];
    let mut inode_bitmap = [0; BLOCK_SIZE];
    read_device_block(device, BLOCK_BITMAP, &mut block_bitmap)?;
    read_device_block(device, INODE_BITMAP, &mut inode_bitmap)?;

    let mut used_blocks = 0u32;
    let mut block = 0;
    while block < EXT2_BLOCK_COUNT {
        if !is_bit_set(&block_bitmap, block) {
            if block < 15 {
                return Err(StorageError::Corrupt);
            }
        } else {
            used_blocks += 1;
        }
        block += 1;
    }

    let mut used_inodes = 0u32;
    let mut inode_index = 0;
    while inode_index < EXT2_INODE_COUNT {
        if is_bit_set(&inode_bitmap, inode_index) {
            used_inodes += 1;
        }
        inode_index += 1;
    }
    while inode_index < (BLOCK_SIZE * 8) as u32 {
        if is_bit_set(&inode_bitmap, inode_index) {
            return Err(StorageError::Corrupt);
        }
        inode_index += 1;
    }
    if !is_bit_set(&inode_bitmap, 0) || !is_bit_set(&inode_bitmap, ROOT_INODE - 1) {
        return Err(StorageError::Corrupt);
    }

    let free_blocks = EXT2_BLOCK_COUNT - used_blocks;
    let free_inodes = EXT2_INODE_COUNT - used_inodes;
    if read_u32(&superblock, 12) != free_blocks
        || read_u32(&superblock, 16) != free_inodes
        || u32::from(read_u16(&group, 12)) != free_blocks
        || u32::from(read_u16(&group, 14)) != free_inodes
    {
        return Err(StorageError::Corrupt);
    }

    let mut inode_info = [None; EXT2_INODE_COUNT as usize];
    let mut inode_modes = [0u16; EXT2_INODE_COUNT as usize];
    let mut referenced_blocks = [0u8; BLOCK_SIZE];
    let mut regular_files = 0u32;
    let mut directories = 0u32;

    let mut inode_number = ROOT_INODE;
    while inode_number <= EXT2_INODE_COUNT {
        if is_bit_set(&inode_bitmap, inode_number - 1) {
            let info = read_inode_from_device(device, inode_number)?;
            let kind = info.mode & 0xf000;
            if kind != 0x4000 && kind != 0x8000 {
                return Err(StorageError::Corrupt);
            }
            if info.direct_block >= EXT2_BLOCK_COUNT {
                return Err(StorageError::Corrupt);
            }

            if inode_number == ROOT_INODE {
                if kind != 0x4000
                    || info.size != BLOCK_SIZE as u32
                    || info.blocks != 2
                    || info.direct_block != ROOT_DIRECTORY_BLOCK
                {
                    return Err(StorageError::Corrupt);
                }
            } else {
                if info.size > MAX_FILE_SIZE as u32 {
                    return Err(StorageError::Corrupt);
                }
                // Each data block must be allocated and owned by one inode.
                let mut claim = |block: u32| -> Result<(), StorageError> {
                    if !(RESERVED_BLOCKS..EXT2_BLOCK_COUNT).contains(&block)
                        || !is_bit_set(&block_bitmap, block)
                        || is_bit_set(&referenced_blocks, block)
                    {
                        return Err(StorageError::Corrupt);
                    }
                    set_bit(&mut referenced_blocks, block);
                    Ok(())
                };
                claim(info.direct_block)?;
                if kind == 0x4000 {
                    if info.size != BLOCK_SIZE as u32
                        || info.blocks != 2
                        || info.indirect_block != 0
                        || info.extra_blocks.iter().any(|pointer| *pointer != 0)
                    {
                        return Err(StorageError::Corrupt);
                    }
                } else {
                    let count = data_blocks_for_size(info.size as usize);
                    let uses_indirect = count > DIRECT_BLOCKS;
                    let expected_sectors = 2 * (count + usize::from(uses_indirect)) as u32;
                    if info.blocks != expected_sectors && !(info.blocks == 0 && info.size == 0) {
                        return Err(StorageError::Corrupt);
                    }
                    for (index, pointer) in info.extra_blocks.iter().enumerate() {
                        if index + 1 < count {
                            claim(*pointer)?;
                        } else if *pointer != 0 {
                            return Err(StorageError::Corrupt);
                        }
                    }
                    if uses_indirect {
                        claim(info.indirect_block)?;
                        let mut table = [0; BLOCK_SIZE];
                        read_device_block(device, info.indirect_block, &mut table)?;
                        for slot in 0..INDIRECT_POINTERS {
                            let pointer = read_u32(&table, slot * 4);
                            if slot < count - DIRECT_BLOCKS {
                                claim(pointer)?;
                            } else if pointer != 0 {
                                return Err(StorageError::Corrupt);
                            }
                        }
                    } else if info.indirect_block != 0 {
                        return Err(StorageError::Corrupt);
                    }
                }
                if kind == 0x4000 {
                    directories += 1;
                } else {
                    regular_files += 1;
                }
            }

            inode_modes[(inode_number - 1) as usize] = info.mode;
            inode_info[(inode_number - 1) as usize] = Some(info);
        }
        inode_number += 1;
    }

    let root_info = inode_info[(ROOT_INODE - 1) as usize].ok_or(StorageError::Corrupt)?;
    let mut root = [0; BLOCK_SIZE];
    read_device_block(device, root_info.direct_block, &mut root)?;

    let mut parent_by_inode = [0u32; EXT2_INODE_COUNT as usize];
    let mut parent_directory = [0u32; EXT2_INODE_COUNT as usize];
    let mut dot_dot_parent = [0u32; EXT2_INODE_COUNT as usize];
    let mut inbound_entries = [0u8; EXT2_INODE_COUNT as usize];
    let mut directory_entries = 0u32;
    let mut current_inode = ROOT_INODE;
    while current_inode <= EXT2_INODE_COUNT {
        let Some(info) = inode_info[(current_inode - 1) as usize] else {
            current_inode += 1;
            continue;
        };
        if inode_modes[(current_inode - 1) as usize] & 0xf000 != 0x4000 {
            current_inode += 1;
            continue;
        }

        let directory = if current_inode == ROOT_INODE {
            root
        } else {
            let mut bytes = [0; BLOCK_SIZE];
            read_device_block(device, info.direct_block, &mut bytes)?;
            bytes
        };
        let mut offset = 0usize;
        let mut dot_count = 0u8;
        let mut dot_dot_count = 0u8;
        let mut dot_dot_inode = 0u32;
        while offset < BLOCK_SIZE {
            let target = read_u32(&directory, offset);
            let record_length = usize::from(read_u16(&directory, offset + 4));
            let name_length = usize::from(read_u8(&directory, offset + 6));
            if record_length < 8
                || record_length % 4 != 0
                || offset + record_length > BLOCK_SIZE
                || name_length > record_length - 8
                || name_length > MAX_NAME_LENGTH
            {
                return Err(StorageError::Corrupt);
            }

            if target != 0 {
                if name_length == 0 {
                    return Err(StorageError::Corrupt);
                }
                let name = &directory[offset + 8..offset + 8 + name_length];
                if name.iter().any(|byte| *byte == 0 || *byte == b'/') {
                    return Err(StorageError::Corrupt);
                }
                if name != b"." && name != b".." {
                    let mut previous_offset = 0usize;
                    while previous_offset < offset {
                        let previous_target = read_u32(&directory, previous_offset);
                        let previous_length =
                            usize::from(read_u16(&directory, previous_offset + 4));
                        let previous_name_length =
                            usize::from(read_u8(&directory, previous_offset + 6));
                        if previous_target != 0
                            && previous_name_length == name_length
                            && !is_dot_entry(&directory, previous_offset, previous_name_length)
                        {
                            let previous_name = &directory
                                [previous_offset + 8..previous_offset + 8 + previous_name_length];
                            if previous_name == name {
                                return Err(StorageError::Corrupt);
                            }
                        }
                        previous_offset += previous_length;
                    }
                }
                let target_index =
                    usize::try_from(target - 1).map_err(|_| StorageError::Corrupt)?;
                if target == 0
                    || target > EXT2_INODE_COUNT
                    || !is_bit_set(&inode_bitmap, target - 1)
                {
                    return Err(StorageError::Corrupt);
                }
                let target_kind = inode_modes[target_index] & 0xf000;
                let entry_kind = read_u8(&directory, offset + 7);
                if (target_kind == 0x8000 && entry_kind != 1)
                    || (target_kind == 0x4000 && entry_kind != 2)
                    || (target_kind != 0x8000 && target_kind != 0x4000)
                {
                    return Err(StorageError::Corrupt);
                }

                if name == b"." {
                    dot_count += 1;
                    if target != current_inode || target_kind != 0x4000 {
                        return Err(StorageError::Corrupt);
                    }
                } else if name == b".." {
                    dot_dot_count += 1;
                    dot_dot_inode = target;
                    if target_kind != 0x4000 {
                        return Err(StorageError::Corrupt);
                    }
                } else {
                    if target == ROOT_INODE {
                        return Err(StorageError::Corrupt);
                    }
                    let target_slot = target_index;
                    if parent_by_inode[target_slot] != 0 {
                        return Err(StorageError::Corrupt);
                    }
                    parent_by_inode[target_slot] = current_inode;
                    parent_directory[target_slot] = current_inode;
                    inbound_entries[target_slot] = inbound_entries[target_slot]
                        .checked_add(1)
                        .ok_or(StorageError::Corrupt)?;
                    directory_entries += 1;
                }
            }
            offset += record_length;
        }
        if dot_count != 1 || dot_dot_count != 1 {
            return Err(StorageError::Corrupt);
        }
        dot_dot_parent[(current_inode - 1) as usize] = dot_dot_inode;
        current_inode += 1;
    }

    let mut inode_number = ROOT_INODE;
    while inode_number <= EXT2_INODE_COUNT {
        let slot = (inode_number - 1) as usize;
        if inode_info[slot].is_some() {
            if inode_number == ROOT_INODE {
                if dot_dot_parent[slot] != ROOT_INODE || inbound_entries[slot] != 0 {
                    return Err(StorageError::Corrupt);
                }
            } else {
                let parent = parent_directory[slot];
                if parent == 0 || inbound_entries[slot] != 1 {
                    return Err(StorageError::Corrupt);
                }
                if inode_modes[slot] & 0xf000 == 0x4000 && dot_dot_parent[slot] != parent {
                    return Err(StorageError::Corrupt);
                }
                let mut ancestor = parent;
                let mut depth = 0;
                while ancestor != ROOT_INODE && depth < EXT2_INODE_COUNT {
                    if ancestor == 0 || ancestor > EXT2_INODE_COUNT {
                        return Err(StorageError::Corrupt);
                    }
                    let ancestor_slot = (ancestor - 1) as usize;
                    if inode_modes[ancestor_slot] & 0xf000 != 0x4000 {
                        return Err(StorageError::Corrupt);
                    }
                    ancestor = parent_directory[ancestor_slot];
                    depth += 1;
                }
                if ancestor != ROOT_INODE {
                    return Err(StorageError::Corrupt);
                }
            }
        }
        inode_number += 1;
    }

    let mut allocated_data_blocks = 0u32;
    let mut block = 15;
    while block < EXT2_BLOCK_COUNT {
        let allocated = is_bit_set(&block_bitmap, block);
        let referenced = is_bit_set(&referenced_blocks, block);
        if allocated != referenced {
            return Err(StorageError::Corrupt);
        }
        if allocated {
            allocated_data_blocks += 1;
        }
        block += 1;
    }

    Ok(VfsIntegrityReport {
        regular_files,
        directories: directories + 1,
        allocated_data_blocks,
        directory_entries,
    })
}

/// Volumes written before generations store 0; handles issued for them
/// used generation 1, so 0 reads as 1.
fn effective_generation(raw: u32) -> u32 {
    raw.max(1)
}

fn check_data_block(block: u32) -> Result<(), StorageError> {
    if (RESERVED_BLOCKS..EXT2_BLOCK_COUNT).contains(&block) {
        Ok(())
    } else {
        Err(StorageError::Corrupt)
    }
}

fn set_file_pointer(inode: &mut InodeInfo, table: &mut [u8; BLOCK_SIZE], index: usize, block: u32) {
    match index {
        0 => inode.direct_block = block,
        1..DIRECT_BLOCKS => inode.extra_blocks[index - 1] = block,
        _ => write_u32(table, (index - DIRECT_BLOCKS) * 4, block),
    }
}

fn read_extra_blocks(bytes: &[u8; BLOCK_SIZE], offset: usize) -> [u32; DIRECT_BLOCKS - 1] {
    let mut pointers = [0; DIRECT_BLOCKS - 1];
    for (index, pointer) in pointers.iter_mut().enumerate() {
        *pointer = read_u32(bytes, offset + 44 + index * 4);
    }
    pointers
}

/// Data blocks a regular file of `size` bytes occupies. The first block is
/// allocated at creation, so an empty file still owns one block.
fn data_blocks_for_size(size: usize) -> usize {
    size.div_ceil(BLOCK_SIZE).max(1)
}

fn read_inode_from_device<D: ReadOnlyBlockDevice>(
    device: &mut D,
    inode: u32,
) -> Result<InodeInfo, StorageError> {
    if inode == 0 || inode > EXT2_INODE_COUNT {
        return Err(StorageError::Corrupt);
    }
    let block = INODE_TABLE + (inode - 1) / 8;
    let offset = ((inode - 1) % 8) as usize * EXT2_INODE_SIZE as usize;
    let mut bytes = [0; BLOCK_SIZE];
    read_device_block(device, block, &mut bytes)?;
    Ok(InodeInfo {
        mode: read_u16(&bytes, offset),
        uid: read_u16(&bytes, offset + 2),
        size: read_u32(&bytes, offset + 4),
        atime: read_u32(&bytes, offset + 8),
        ctime: read_u32(&bytes, offset + 12),
        mtime: read_u32(&bytes, offset + 16),
        gid: read_u16(&bytes, offset + 24),
        blocks: read_u32(&bytes, offset + 28),
        direct_block: read_u32(&bytes, offset + 40),
        extra_blocks: read_extra_blocks(&bytes, offset),
        indirect_block: read_u32(&bytes, offset + 40 + DIRECT_BLOCKS * 4),
        generation: read_u32(&bytes, offset + 100),
    })
}

fn read_device_block<D: ReadOnlyBlockDevice>(
    device: &mut D,
    block: u32,
    destination: &mut [u8; BLOCK_SIZE],
) -> Result<(), StorageError> {
    let sector = u64::from(block).checked_mul(2).ok_or(StorageError::Block)?;
    let (first, second) = destination.split_at_mut(SECTOR_SIZE);
    let first: &mut [u8; SECTOR_SIZE] = first.try_into().map_err(|_| StorageError::Block)?;
    let second: &mut [u8; SECTOR_SIZE] = second.try_into().map_err(|_| StorageError::Block)?;
    device.read_sector(sector, first)?;
    device.read_sector(sector + 1, second)
}

fn validate_name(name: &[u8]) -> Result<(), StorageError> {
    if name.is_empty() {
        return Err(StorageError::InvalidName);
    }
    if name.len() > MAX_NAME_LENGTH {
        return Err(StorageError::NameTooLong);
    }
    if name.iter().any(|byte| *byte == b'/' || *byte == 0) {
        return Err(StorageError::InvalidName);
    }
    Ok(())
}

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn is_dot_entry(directory: &[u8; BLOCK_SIZE], offset: usize, name_length: usize) -> bool {
    (name_length == 1 && read_u8(directory, offset + 8) == b'.')
        || (name_length == 2
            && read_u8(directory, offset + 8) == b'.'
            && read_u8(directory, offset + 9) == b'.')
}

fn names_equal(directory: &[u8; BLOCK_SIZE], offset: usize, length: usize, name: &[u8]) -> bool {
    if length != name.len() {
        return false;
    }
    let mut index = 0;
    while index < length {
        if read_u8(directory, offset + index) != read_u8(name, index) {
            return false;
        }
        index += 1;
    }
    true
}

fn write_directory_record(
    directory: &mut [u8; BLOCK_SIZE],
    offset: usize,
    inode: u32,
    record_length: u16,
    name_length: u8,
    file_type: u8,
    name: &[u8],
) {
    write_u32(directory, offset, inode);
    write_u16(directory, offset + 4, record_length);
    write_u8(directory, offset + 6, name_length);
    write_u8(directory, offset + 7, file_type);
    copy_bytes_to_offset(directory, offset + 8, name);
}

fn copy_bytes_to_offset(destination: &mut [u8; BLOCK_SIZE], offset: usize, source: &[u8]) {
    let mut index = 0;
    while index < source.len() {
        unsafe {
            ptr::write_volatile(
                destination.as_mut_ptr().add(offset + index),
                ptr::read_volatile(source.as_ptr().add(index)),
            );
        }
        index += 1;
    }
}

fn copy_bytes_from_offset(
    destination: &mut [u8; MAX_NAME_LENGTH],
    source: &[u8; BLOCK_SIZE],
    offset: usize,
    length: usize,
) {
    let mut index = 0;
    while index < length {
        unsafe {
            ptr::write_volatile(
                destination.as_mut_ptr().add(index),
                ptr::read_volatile(source.as_ptr().add(offset + index)),
            );
        }
        index += 1;
    }
}

fn read_u8(bytes: &[u8], offset: usize) -> u8 {
    unsafe { ptr::read(bytes.as_ptr().add(offset)) }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le(unsafe { ptr::read_unaligned(bytes.as_ptr().add(offset).cast()) })
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le(unsafe { ptr::read_unaligned(bytes.as_ptr().add(offset).cast()) })
}

fn write_u8(bytes: &mut [u8], offset: usize, value: u8) {
    unsafe { ptr::write(bytes.as_mut_ptr().add(offset), value) };
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    unsafe { ptr::write_unaligned(bytes.as_mut_ptr().add(offset).cast(), value.to_le()) };
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    unsafe { ptr::write_unaligned(bytes.as_mut_ptr().add(offset).cast(), value.to_le()) };
}

fn is_bit_set(bitmap: &[u8; BLOCK_SIZE], bit: u32) -> bool {
    let byte = usize::try_from(bit / 8).unwrap_or(usize::MAX);
    byte < BLOCK_SIZE && read_u8(bitmap, byte) & (1 << (bit % 8)) != 0
}

fn set_bit(bitmap: &mut [u8; BLOCK_SIZE], bit: u32) {
    let byte = (bit / 8) as usize;
    let value = read_u8(bitmap, byte) | (1 << (bit % 8));
    write_u8(bitmap, byte, value);
}

fn clear_bit_value(bitmap: &mut [u8; BLOCK_SIZE], bit: u32) {
    let byte = (bit / 8) as usize;
    let value = read_u8(bitmap, byte) & !(1 << (bit % 8));
    write_u8(bitmap, byte, value);
}

#[cfg(test)]
mod tests {
    extern crate std;

    use core::sync::atomic::Ordering;
    use sha2::{Digest, Sha256};
    use std::vec;
    use std::vec::Vec;

    use super::{
        allocate_access_id, cleared_inode_matches, is_bit_set, read_device_block,
        read_inode_bytes_from_device, read_u16, read_u32, BlockDevice, DirectoryEntry, FileHandle,
        FileMapping, FileMetadata, ResetAccess, StorageError, Undo, Vfs, VolumeAccess,
        ACTIVE_ACCESS_ID, BLOCK_BITMAP, BLOCK_SIZE, DIRECT_BLOCKS, EXT2_BLOCK_COUNT,
        EXT2_INODE_COUNT, EXT2_INODE_SIZE, FIRST_FILE_INODE, GROUP_DESCRIPTOR_BLOCK, INODE_BITMAP,
        INODE_TABLE, LEGACY_UNDO_MAGIC, MAX_FILE_SIZE, MAX_NAME_LENGTH, RESERVED_BLOCKS,
        ROOT_DIRECTORY_BLOCK, ROOT_INODE, SECTOR_SIZE, SUPERBLOCK_BLOCK, UNDO_BLOCK, UNDO_RENAME,
    };

    /// A device backing the whole 8 MiB volume, for multi-block files.
    struct FullVolumeDevice {
        sectors: Vec<[u8; SECTOR_SIZE]>,
    }

    impl FullVolumeDevice {
        fn new() -> Self {
            Self {
                sectors: vec![[0; SECTOR_SIZE]; EXT2_BLOCK_COUNT as usize * 2],
            }
        }
    }

    impl super::ReadOnlyBlockDevice for FullVolumeDevice {
        fn read_sector(
            &mut self,
            sector: u64,
            destination: &mut [u8; SECTOR_SIZE],
        ) -> Result<(), StorageError> {
            let source = self
                .sectors
                .get(sector as usize)
                .ok_or(StorageError::Block)?;
            destination.copy_from_slice(source);
            Ok(())
        }
    }

    impl BlockDevice for FullVolumeDevice {
        fn write_sector(
            &mut self,
            sector: u64,
            source: &[u8; SECTOR_SIZE],
        ) -> Result<(), StorageError> {
            self.sectors
                .get_mut(sector as usize)
                .ok_or(StorageError::Block)?
                .copy_from_slice(source);
            Ok(())
        }

        fn flush(&mut self) -> Result<(), StorageError> {
            Ok(())
        }
    }

    fn pattern(length: usize, seed: u8) -> Vec<u8> {
        (0..length)
            .map(|index| (index as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    fn check(volume: Vfs<FullVolumeDevice>) -> (super::VfsIntegrityReport, Vfs<FullVolumeDevice>) {
        let mut device = volume.into_device();
        let report = Vfs::<FullVolumeDevice>::check_existing(&mut device).expect("integrity");
        (report, Vfs::mount_existing(device).expect("remount"))
    }

    #[test]
    fn multi_block_files_round_trip_through_direct_and_indirect_blocks() {
        let (mut volume, _) = Vfs::mount_or_format(FullVolumeDevice::new()).expect("format");
        let direct = volume.create(b"direct.bin").expect("create");
        let direct_bytes = pattern(DIRECT_BLOCKS * BLOCK_SIZE - 7, 1);
        volume.write(direct, &direct_bytes).expect("write direct");
        let large = volume.create(b"large.bin").expect("create");
        let large_bytes = pattern(MAX_FILE_SIZE, 2);
        volume.write(large, &large_bytes).expect("write indirect");

        let mut read_back = vec![0; MAX_FILE_SIZE];
        assert_eq!(volume.read(direct, &mut read_back), Ok(direct_bytes.len()));
        assert_eq!(&read_back[..direct_bytes.len()], &direct_bytes[..]);
        assert_eq!(volume.read(large, &mut read_back), Ok(MAX_FILE_SIZE));
        assert_eq!(read_back, large_bytes);

        // Ranged reads cross block boundaries.
        let mut window = [0; 3000];
        let offset = DIRECT_BLOCKS * BLOCK_SIZE - 1000;
        assert_eq!(volume.read_at(large, offset, &mut window), Ok(3000));
        assert_eq!(&window[..], &large_bytes[offset..offset + 3000]);
        assert_eq!(volume.read_at(large, MAX_FILE_SIZE, &mut window), Ok(0));

        let metadata = volume.metadata(large).expect("metadata");
        let data_blocks = MAX_FILE_SIZE / BLOCK_SIZE;
        assert_eq!(metadata.blocks as usize, 2 * (data_blocks + 1));

        let (report, _) = check(volume);
        assert_eq!(report.regular_files, 2);
        // 12 direct + 268 data + 1 indirect.
        assert_eq!(
            report.allocated_data_blocks as usize,
            DIRECT_BLOCKS + data_blocks + 1
        );
    }

    #[test]
    fn shrinking_and_removing_files_release_every_block() {
        let (mut volume, _) = Vfs::mount_or_format(FullVolumeDevice::new()).expect("format");
        let (baseline, mut volume) = check({
            let small = volume.create(b"small.txt").expect("create");
            volume.write(small, b"x").expect("write");
            volume
        });
        let handle = volume.create(b"big.bin").expect("create");
        volume
            .write(handle, &pattern(100 * BLOCK_SIZE, 3))
            .expect("write big");
        volume
            .write(handle, &pattern(2 * BLOCK_SIZE + 5, 4))
            .expect("shrink by rewrite");
        let (report, mut volume) = check(volume);
        assert_eq!(
            report.allocated_data_blocks,
            baseline.allocated_data_blocks + 3
        );
        volume.truncate(handle, 10).expect("truncate");
        let (report, mut volume) = check(volume);
        assert_eq!(
            report.allocated_data_blocks,
            baseline.allocated_data_blocks + 1
        );
        volume
            .write(handle, &pattern(50 * BLOCK_SIZE, 5))
            .expect("grow past direct blocks");
        volume.remove(b"big.bin").expect("remove");
        let (report, _) = check(volume);
        assert_eq!(report.allocated_data_blocks, baseline.allocated_data_blocks);
        assert_eq!(report.regular_files, baseline.regular_files);
    }

    #[test]
    fn ranged_writes_grow_files_and_gaps_read_as_zero() {
        let (mut volume, _) = Vfs::mount_or_format(FullVolumeDevice::new()).expect("format");
        let handle = volume.create(b"sparse.bin").expect("create");
        volume.write(handle, b"head").expect("write");
        volume
            .write_at(handle, 3 * BLOCK_SIZE + 10, b"tail")
            .expect("write past end");
        let size = 3 * BLOCK_SIZE + 14;
        let mut contents = vec![0xaa; size];
        assert_eq!(volume.read(handle, &mut contents), Ok(size));
        assert_eq!(&contents[..4], b"head");
        assert!(contents[4..3 * BLOCK_SIZE + 10]
            .iter()
            .all(|byte| *byte == 0));
        assert_eq!(&contents[size - 4..], b"tail");

        // Shrink then grow: bytes past the old end must not reappear.
        volume.truncate(handle, 2).expect("shrink");
        volume.truncate(handle, BLOCK_SIZE + 1).expect("grow");
        let mut contents = vec![0xaa; BLOCK_SIZE + 1];
        assert_eq!(volume.read(handle, &mut contents), Ok(BLOCK_SIZE + 1));
        assert_eq!(&contents[..2], b"he");
        assert!(contents[2..].iter().all(|byte| *byte == 0));

        assert_eq!(
            volume.write_at(handle, MAX_FILE_SIZE, b"x"),
            Err(StorageError::FileTooLarge)
        );
        let (report, _) = check(volume);
        assert_eq!(report.regular_files, 1);
    }

    #[test]
    fn single_block_files_keep_the_original_on_disk_layout() {
        let (mut volume, _) = Vfs::mount_or_format(FullVolumeDevice::new()).expect("format");
        let handle = volume.create(b"note.txt").expect("create");
        volume.write(handle, b"small").expect("write");
        let inode = volume.read_inode(handle.inode).expect("inode");
        assert_eq!(inode.blocks, 2);
        assert!(inode.extra_blocks.iter().all(|pointer| *pointer == 0));
        assert_eq!(inode.indirect_block, 0);
    }

    #[test]
    fn reused_inodes_get_new_generations_and_stale_handles_fail() {
        let (mut volume, _) = Vfs::mount_or_format(FullVolumeDevice::new()).expect("format");
        let first = volume.create(b"doc.txt").expect("create");
        volume.write(first, b"first").expect("write");
        volume.remove(b"doc.txt").expect("remove");
        let second = volume.create(b"doc.txt").expect("recreate");
        assert_eq!(second.inode(), first.inode(), "the slot is reused");
        assert!(second.generation() > first.generation());
        assert_eq!(
            volume.read(first, &mut [0; 8]),
            Err(StorageError::InvalidHandle)
        );
        assert_eq!(volume.open(b"doc.txt"), Ok(second));
        let metadata = volume.metadata(second).expect("metadata");
        assert_eq!(metadata.generation, second.generation());

        // Rename keeps the same file, so the handle stays valid.
        assert_eq!(volume.rename(b"doc.txt", b"new.txt"), Ok(second));
        volume
            .write(second, b"still mine")
            .expect("write after rename");
    }

    #[test]
    fn inodes_written_before_generations_keep_their_handles() {
        let (mut volume, _) = Vfs::mount_or_format(FullVolumeDevice::new()).expect("format");
        let handle = volume.create(b"legacy.txt").expect("create");
        volume.write(handle, b"old").expect("write");
        let mut inode = volume.read_inode(handle.inode()).expect("inode");
        inode.generation = 0;
        volume
            .write_inode(handle.inode(), inode)
            .expect("legacy inode");
        let legacy = volume.open(b"legacy.txt").expect("open legacy");
        assert_eq!(legacy.generation(), 1);
        let mut contents = [0; 8];
        assert_eq!(volume.read(legacy, &mut contents), Ok(3));
        volume.remove(b"legacy.txt").expect("remove");
        let reused = volume.create(b"legacy.txt").expect("recreate");
        assert!(reused.generation() > 1);
    }

    #[test]
    fn failed_growth_rolls_back_allocations() {
        // The small test device ends at block 32, so growth past it fails
        // part-way through and must release what it took.
        let (mut volume, _) = Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        let handle = volume.create(b"grow.bin").expect("create");
        volume.write(handle, b"keep").expect("write");
        assert_eq!(
            volume.write(handle, &[7; 40 * BLOCK_SIZE]),
            Err(StorageError::Block)
        );
        let mut contents = [0; 8];
        assert_eq!(volume.read(handle, &mut contents), Ok(4));
        assert_eq!(&contents[..4], b"keep");
        let mut device = volume.into_device();
        let report = Vfs::<MemoryBlockDevice>::check_existing(&mut device).expect("integrity");
        assert_eq!(report.allocated_data_blocks, 1);
    }

    struct MemoryBlockDevice {
        sectors: [[u8; SECTOR_SIZE]; 64],
        writes: u32,
        flushes: u32,
    }

    impl MemoryBlockDevice {
        fn new() -> Self {
            Self {
                sectors: [[0; SECTOR_SIZE]; 64],
                writes: 0,
                flushes: 0,
            }
        }
    }

    #[test]
    fn mount_existing_accepts_a_valid_volume_without_writes_or_flushes() {
        let (mut volume, formatted) =
            Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format test volume");
        assert!(formatted);
        let handle = volume.create(b"recovery").expect("create test file");
        volume.write(handle, b"keep me").expect("write test file");
        let device = volume.into_device();
        let writes_before = device.writes;
        let flushes_before = device.flushes;
        let sectors_before = device.sectors;

        let mut mounted = Vfs::mount_existing(device).expect("mount existing volume");
        assert_eq!(mounted.open(b"recovery"), Ok(handle));
        let device = mounted.into_device();
        assert_eq!(device.writes, writes_before);
        assert_eq!(device.flushes, flushes_before);
        assert_eq!(device.sectors, sectors_before);
    }

    #[test]
    fn mount_existing_rejects_an_unformatted_device_instead_of_formatting_it() {
        let device = MemoryBlockDevice::new();
        assert_eq!(
            Vfs::mount_existing(device).err(),
            Some(StorageError::Corrupt)
        );
    }

    impl super::ReadOnlyBlockDevice for MemoryBlockDevice {
        fn read_sector(
            &mut self,
            sector: u64,
            destination: &mut [u8; SECTOR_SIZE],
        ) -> Result<(), StorageError> {
            let index = usize::try_from(sector).map_err(|_| StorageError::Block)?;
            let Some(source) = self.sectors.get(index) else {
                return Err(StorageError::Block);
            };
            destination.copy_from_slice(source);
            Ok(())
        }
    }

    impl BlockDevice for MemoryBlockDevice {
        fn write_sector(
            &mut self,
            sector: u64,
            source: &[u8; SECTOR_SIZE],
        ) -> Result<(), StorageError> {
            let index = usize::try_from(sector).map_err(|_| StorageError::Block)?;
            let Some(destination) = self.sectors.get_mut(index) else {
                return Err(StorageError::Block);
            };
            destination.copy_from_slice(source);
            self.writes += 1;
            Ok(())
        }

        fn flush(&mut self) -> Result<(), StorageError> {
            self.flushes += 1;
            Ok(())
        }
    }

    #[test]
    fn read_only_integrity_check_accepts_nested_vfs_and_never_writes() {
        let (mut volume, formatted) =
            Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        assert!(formatted);
        let root_file = volume.create_path(b"/readme.txt").expect("root file");
        volume
            .write(root_file, b"read-only check")
            .expect("write root file");
        volume.mkdir_path(b"/docs").expect("create directory");
        let nested_file = volume.create_path(b"/docs/page.txt").expect("nested file");
        volume
            .write(nested_file, b"nested data")
            .expect("write nested file");
        let mut device = volume.into_device();
        let writes_before = device.writes;
        let flushes_before = device.flushes;
        let image_before = device.sectors;

        let report =
            Vfs::<MemoryBlockDevice>::check_existing(&mut device).expect("valid volume check");

        assert_eq!(report.regular_files, 2);
        assert_eq!(report.directories, 2);
        assert_eq!(report.allocated_data_blocks, 3);
        assert_eq!(report.directory_entries, 3);
        assert_eq!(device.writes, writes_before);
        assert_eq!(device.flushes, flushes_before);
        assert_eq!(device.sectors, image_before);
    }

    #[test]
    fn read_only_integrity_check_rejects_invalid_volume_without_formatting() {
        let (volume, formatted) = Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        assert!(formatted);
        let mut device = volume.into_device();
        device.sectors[2][56] = 0;
        let writes_before = device.writes;
        let flushes_before = device.flushes;
        let image_before = device.sectors;

        assert_eq!(
            Vfs::<MemoryBlockDevice>::check_existing(&mut device),
            Err(StorageError::Corrupt)
        );
        assert_eq!(device.writes, writes_before);
        assert_eq!(device.flushes, flushes_before);
        assert_eq!(device.sectors, image_before);
    }

    #[test]
    fn read_only_integrity_check_detects_accounting_corruption_without_writes() {
        let (volume, formatted) = Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        assert!(formatted);
        let mut device = volume.into_device();
        device.sectors[2][12] ^= 1;
        let writes_before = device.writes;
        let flushes_before = device.flushes;
        let image_before = device.sectors;

        assert_eq!(
            Vfs::<MemoryBlockDevice>::check_existing(&mut device),
            Err(StorageError::Corrupt)
        );
        assert_eq!(device.writes, writes_before);
        assert_eq!(device.flushes, flushes_before);
        assert_eq!(device.sectors, image_before);
    }

    #[test]
    fn read_only_integrity_check_detects_directory_record_corruption_without_writes() {
        let (volume, formatted) = Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        assert!(formatted);
        let mut device = volume.into_device();
        device.sectors[ROOT_DIRECTORY_BLOCK as usize * 2][4] = 0;
        device.sectors[ROOT_DIRECTORY_BLOCK as usize * 2][5] = 0;
        let writes_before = device.writes;
        let flushes_before = device.flushes;
        let image_before = device.sectors;

        assert_eq!(
            Vfs::<MemoryBlockDevice>::check_existing(&mut device),
            Err(StorageError::Corrupt)
        );
        assert_eq!(device.writes, writes_before);
        assert_eq!(device.flushes, flushes_before);
        assert_eq!(device.sectors, image_before);
    }

    #[test]
    fn read_only_integrity_check_rejects_out_of_range_inode_bits_without_writes() {
        let (volume, formatted) = Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        assert!(formatted);
        let mut device = volume.into_device();
        device.sectors[8][8] = 1;
        let writes_before = device.writes;
        let flushes_before = device.flushes;
        let image_before = device.sectors;

        assert_eq!(
            Vfs::<MemoryBlockDevice>::check_existing(&mut device),
            Err(StorageError::Corrupt)
        );
        assert_eq!(device.writes, writes_before);
        assert_eq!(device.flushes, flushes_before);
        assert_eq!(device.sectors, image_before);
    }

    #[test]
    fn read_only_integrity_check_rejects_orphan_parent_chain_without_writes() {
        let (mut volume, formatted) =
            Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        assert!(formatted);
        volume
            .create_path(b"/reused")
            .expect("allocate first inode");
        volume
            .mkdir_path(b"/orphan")
            .expect("allocate higher directory inode");
        volume.remove_path(b"/reused").expect("free first inode");
        volume
            .create_path(b"/orphan/child")
            .expect("reuse lower inode under orphan directory");
        let mut device = volume.into_device();

        let root_start = ROOT_DIRECTORY_BLOCK as usize * 2;
        let root = &mut device.sectors[root_start];
        let mut offset = 0usize;
        let mut orphan_offset = None;
        while offset < SECTOR_SIZE {
            let record_length =
                usize::from(u16::from_le_bytes([root[offset + 4], root[offset + 5]]));
            let name_length = usize::from(root[offset + 6]);
            if name_length == b"orphan".len()
                && &root[offset + 8..offset + 8 + name_length] == b"orphan"
            {
                orphan_offset = Some(offset);
                break;
            }
            if record_length < 8 {
                break;
            }
            offset += record_length;
        }
        let orphan_offset = orphan_offset.expect("root entry for orphan directory");
        root[orphan_offset..orphan_offset + 4].fill(0);

        let writes_before = device.writes;
        let flushes_before = device.flushes;
        let image_before = device.sectors;
        assert_eq!(
            Vfs::<MemoryBlockDevice>::check_existing(&mut device),
            Err(StorageError::Corrupt)
        );
        assert_eq!(device.writes, writes_before);
        assert_eq!(device.flushes, flushes_before);
        assert_eq!(device.sectors, image_before);
    }

    #[test]
    fn formats_mounts_and_round_trips_a_root_file() {
        let (mut volume, formatted) =
            Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        assert!(formatted);
        let handle = volume.create(b"nagi.txt").expect("create");
        let payload = b"nagi persistent";
        volume.write(handle, payload).expect("write");
        let mut entries = [DirectoryEntry::empty(); 8];
        assert_eq!(volume.list_root(&mut entries).expect("list"), 1);
        assert_eq!(entries[0].name(), b"nagi.txt");
        let mut read_back = [0; 32];
        let length = volume.read(handle, &mut read_back).expect("read");
        assert_eq!(&read_back[..length], payload);

        let device = volume.into_device();
        let (mut mounted, formatted) = Vfs::mount_or_format(device).expect("mount");
        assert!(!formatted);
        let handle = mounted.open(b"nagi.txt").expect("open");
        let mut remounted = [0; 32];
        let length = mounted
            .read(handle, &mut remounted)
            .expect("remounted read");
        assert_eq!(&remounted[..length], payload);
    }

    #[test]
    fn truncate_and_inode_metadata_round_trip_through_ext2_fields() {
        let (mut volume, _) = Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        let handle = volume.create(b"metadata").expect("create");
        volume.write(handle, b"abcdef").expect("write");
        volume.truncate(handle, 3).expect("shrink");
        volume.truncate(handle, 6).expect("extend");
        volume.set_mode(handle, 0o100640).expect("mode");
        volume
            .set_owner(handle, Some(123), Some(456))
            .expect("owner");

        let mut contents = [0; 8];
        let length = volume.read(handle, &mut contents).expect("read");
        assert_eq!(length, 6);
        assert_eq!(&contents[..length], b"abc\0\0\0");
        volume.set_times(handle, 101, 202).expect("times");
        let metadata = volume.metadata(handle).expect("metadata");
        assert_eq!(metadata.mode, 0o100640);
        assert_eq!(metadata.uid, 123);
        assert_eq!(metadata.gid, 456);
        assert_eq!(metadata.size, 6);
        assert_eq!(metadata.atime, 101);
        assert_eq!(metadata.mtime, 202);
        assert_eq!(metadata.blocks, 2);

        let device = volume.into_device();
        let (mut remounted, formatted) = Vfs::mount_or_format(device).expect("remount");
        assert!(!formatted);
        let handle = remounted.open(b"metadata").expect("open after remount");
        let persisted = remounted.metadata(handle).expect("persisted metadata");
        assert_eq!(persisted.uid, 123);
        assert_eq!(persisted.gid, 456);
        assert_eq!(persisted.atime, 101);
        assert_eq!(persisted.mtime, 202);
    }

    #[test]
    fn rejects_duplicate_names_invalid_handles_and_oversized_files() {
        let (mut volume, _) = Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        let handle = volume.create(b"nagi.txt").expect("create");
        assert_eq!(volume.create(b"nagi.txt"), Err(StorageError::AlreadyExists));
        assert_eq!(
            volume.read(FileHandle::invalid(), &mut [0; 4]),
            Err(StorageError::InvalidHandle)
        );
        assert_eq!(
            volume.write(handle, &[0; MAX_FILE_SIZE + 1]),
            Err(StorageError::FileTooLarge)
        );
        assert_eq!(volume.create(&[b'x'; 33]), Err(StorageError::NameTooLong));
    }

    #[test]
    fn supports_bounded_root_directories_and_removal() {
        let device = MemoryBlockDevice::new();
        let (mut volume, formatted) = Vfs::mount_or_format(device).expect("format");
        assert!(formatted);
        volume.mkdir(b"docs").expect("mkdir");
        let mut entries = [DirectoryEntry::empty(); 8];
        let count = volume.list_root(&mut entries).expect("list");
        assert_eq!(count, 1);
        assert_eq!(entries[0].name(), b"docs");
        assert_eq!(entries[0].file_type, 2);
        assert_eq!(volume.remove(b"docs"), Ok(()));
        assert_eq!(volume.list_root(&mut entries).expect("empty list"), 0);

        let source = volume.create(b"source").expect("source");
        volume.write(source, b"copy me").expect("write source");
        let source_handle = volume.open(b"source").expect("reopen source");
        let mut bytes = [0; 16];
        let length = volume.read(source_handle, &mut bytes).expect("read source");
        assert_eq!(&bytes[..length], b"copy me");
        assert_eq!(volume.remove(b"source"), Ok(()));
        assert_eq!(volume.open(b"source"), Err(StorageError::NotFound));
    }

    #[test]
    fn resolves_persistent_nested_paths_without_escaping_the_volume_root() {
        let (mut volume, _) = Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        volume.ensure_directory_path(b"/tmp").expect("tmp");
        volume
            .ensure_directory_path(b"/tmp")
            .expect("existing tmp directory");
        volume.mkdir_path(b"/tmp/.tmp1234").expect("temp dir");
        volume
            .mkdir_path(b"/tmp/.tmp1234/clientstorage")
            .expect("client storage");
        volume
            .mkdir_path(b"/tmp/.tmp1234/clientstorage/default_v1")
            .expect("default storage");
        let state = volume
            .create_path(b"/tmp/.tmp1234/clientstorage/default_v1/state.sqlite")
            .expect("nested file");
        volume.write(state, b"guest storage").expect("write");
        assert_eq!(
            volume.ensure_directory_path(b"/tmp/.tmp1234/clientstorage/default_v1/state.sqlite"),
            Err(StorageError::NotDirectory)
        );

        let normalized =
            b"/../../tmp//.tmp1234/clientstorage/./default_v1/../default_v1/state.sqlite";
        let reopened = volume.open_path(normalized).expect("bounded resolution");
        let mut contents = [0; 32];
        let length = volume.read(reopened, &mut contents).expect("read");
        assert_eq!(&contents[..length], b"guest storage");
        assert_eq!(
            volume.open_path(b"/tmp/.tmp1234/clientstorage/default_v1/state.sqlite/child"),
            Err(StorageError::NotDirectory)
        );
        assert_eq!(
            volume.open_path(b"/tmp/.tmp1234/clientstorage/default_v1/state.sqlite/"),
            Err(StorageError::NotDirectory)
        );
        assert_eq!(
            volume.remove_path(b"/tmp/.tmp1234/clientstorage/default_v1"),
            Err(StorageError::IsDirectory)
        );
        assert_eq!(
            volume.rmdir_path(b"/tmp/.tmp1234/clientstorage/default_v1"),
            Err(StorageError::DirectoryNotEmpty)
        );

        let mut entries = [DirectoryEntry::empty(); 8];
        let count = volume
            .list_directory_path(b"/tmp/.tmp1234/clientstorage/default_v1", &mut entries)
            .expect("nested listing");
        assert_eq!(count, 1);
        assert_eq!(entries[0].name(), b"state.sqlite");

        let device = volume.into_device();
        let (mut remounted, formatted) = Vfs::mount_or_format(device).expect("remount");
        assert!(!formatted);
        let reopened = remounted.open_path(normalized).expect("persisted path");
        let mut persisted = [0; 32];
        let length = remounted
            .read(reopened, &mut persisted)
            .expect("persisted read");
        assert_eq!(&persisted[..length], b"guest storage");

        remounted
            .remove_path(b"/tmp/.tmp1234/clientstorage/default_v1/state.sqlite")
            .expect("unlink nested file");
        remounted
            .rmdir_path(b"/tmp/.tmp1234/clientstorage/default_v1")
            .expect("remove empty directory");
        assert_eq!(
            remounted.open_path(b"/tmp/.tmp1234/clientstorage/default_v1"),
            Err(StorageError::NotFound)
        );
    }

    #[test]
    fn renames_a_file_without_changing_its_handle_or_contents() {
        let (mut volume, _) = Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        let handle = volume.create(b"before").expect("create");
        volume.write(handle, b"rename me").expect("write");
        let renamed = volume.rename(b"before", b"after").expect("rename");
        assert_eq!(renamed, handle);
        assert_eq!(volume.open(b"before"), Err(StorageError::NotFound));
        let reopened = volume.open(b"after").expect("reopen");
        let mut bytes = [0; 16];
        let length = volume.read(reopened, &mut bytes).expect("read");
        assert_eq!(&bytes[..length], b"rename me");

        // Renaming must leave a free directory record for a later create.
        volume
            .create(b"created-after-rename")
            .expect("create after rename");
    }

    #[test]
    fn renames_a_file_in_a_nested_directory_without_changing_its_identity() {
        let (mut volume, _) = Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        volume.mkdir_path(b"/files").expect("directory");
        let old_path = b"/files/before";
        let new_path = b"/files/a-much-longer-name-after";
        let handle = volume.create_path(old_path).expect("create");
        volume.write(handle, b"rename me").expect("write");
        let old_metadata = volume.metadata_path(old_path).expect("old metadata");

        let renamed = volume
            .rename_child(b"/files", b"before", b"a-much-longer-name-after")
            .expect("rename child");
        assert_eq!(renamed, handle);
        assert_eq!(volume.open_path(old_path), Err(StorageError::NotFound));
        let new_metadata = volume.metadata_path(new_path).expect("new metadata");
        assert_eq!(new_metadata.inode, old_metadata.inode);
        assert_eq!(new_metadata.generation, old_metadata.generation);
        let reopened = volume.open_path(new_path).expect("reopen");
        let mut bytes = [0; 16];
        let length = volume.read(reopened, &mut bytes).expect("read");
        assert_eq!(&bytes[..length], b"rename me");

        volume
            .create_path(b"/files/created-after-rename")
            .expect("create after nested rename");
    }

    #[test]
    fn rename_reserved_dot_entries_is_rejected_without_any_write() {
        for parent in ["/", "/parent"] {
            let disk = FaultDisk::new(false);
            let (mut volume, _) = Vfs::mount_or_format(disk.clone()).unwrap();
            volume.mkdir_path(b"/parent").unwrap();
            let root_file = volume.create(b"before").unwrap();
            volume.write(root_file, b"root retained").unwrap();
            let nested = volume.create_path(b"/parent/before").unwrap();
            volume.write(nested, b"nested retained").unwrap();
            volume.flush().unwrap();
            for (old, new) in [
                (b".".as_slice(), b"after".as_slice()),
                (b"..", b"after"),
                (b"before", b"."),
                (b"before", b".."),
                (b".", b"."),
                (b"..", b".."),
            ] {
                let before = disk.0.lock().unwrap().live.clone();
                disk.inject(None, None, false);
                assert_eq!(
                    volume.rename_child(parent.as_bytes(), old, new),
                    Err(StorageError::InvalidName)
                );
                if parent == "/" {
                    assert_eq!(volume.rename(old, new), Err(StorageError::InvalidName));
                }
                let state = disk.0.lock().unwrap();
                assert_eq!((state.writes, state.flushes), (0, 0));
                assert_eq!(state.live, before);
            }
            drop(volume);
            Vfs::<FaultDisk>::check_existing(&mut disk.clone()).unwrap();
        }
    }

    #[test]
    fn directory_rename_returns_identity_and_preserves_children_without_file_io() {
        let disk = FaultDisk::new(false);
        let (mut volume, _) = Vfs::mount_or_format(disk.clone()).unwrap();
        volume.mkdir_path(b"/parent").unwrap();
        volume.mkdir_path(b"/parent/before").unwrap();
        let child = volume.create_path(b"/parent/before/child").unwrap();
        volume.write(child, b"directory child").unwrap();
        let before = volume.metadata_path(b"/parent/before").unwrap();
        let identity = volume
            .rename_child(b"/parent", b"before", b"after")
            .unwrap();
        assert_eq!(identity.inode(), before.inode);
        assert_eq!(volume.metadata_path(b"/parent/after").unwrap(), before);
        assert_eq!(
            volume.metadata_path(b"/parent/before"),
            Err(StorageError::NotFound)
        );
        assert_eq!(
            volume.rename_child(b"/parent", b"after", b"after"),
            Ok(identity)
        );
        assert_eq!(volume.open_path(b"/parent/after/child"), Ok(child));
        let mut bytes = [0; 32];
        assert_eq!(volume.read(child, &mut bytes), Ok(15));
        assert_eq!(&bytes[..15], b"directory child");
        assert_eq!(
            volume.write(identity, b"not a regular file"),
            Err(StorageError::InvalidHandle)
        );
        assert_eq!(
            volume.read(identity, &mut bytes),
            Err(StorageError::InvalidHandle)
        );
        assert_eq!(
            volume.open_path(b"/parent/after"),
            Err(StorageError::IsDirectory)
        );
        drop(volume);
        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).unwrap();
    }

    #[test]
    fn directory_rename_faults_preserve_identity_children_and_retry() {
        for cached in [false, true] {
            for parent in ["/", "/parent"] {
                let baseline = failure_baseline(cached, false);
                let mut volume = Vfs::mount_existing(baseline.clone()).unwrap();
                if parent != "/" {
                    volume.mkdir_path(parent.as_bytes()).unwrap();
                }
                let old = std::format!("{}/before", parent.trim_end_matches('/'));
                let new = std::format!("{}/after", parent.trim_end_matches('/'));
                let old_child = std::format!("{old}/child");
                let new_child = std::format!("{new}/child");
                volume.mkdir_path(old.as_bytes()).unwrap();
                let child = volume.create_path(old_child.as_bytes()).unwrap();
                volume.write(child, b"retained child").unwrap();
                let before = volume.metadata_path(old.as_bytes()).unwrap();
                volume.flush().unwrap();
                drop(volume);
                let probe = baseline.fork();
                let mut volume = Vfs::mount_existing(probe.clone()).unwrap();
                probe.inject(None, None, false);
                volume
                    .rename_child(parent.as_bytes(), b"before", b"after")
                    .unwrap();
                let (writes, flushes) = {
                    let state = probe.0.lock().unwrap();
                    (state.writes, state.flushes)
                };
                drop(volume);
                for flush_fault in [false, true] {
                    let limit = if flush_fault { flushes } else { writes };
                    for after in 0..limit {
                        let disk = baseline.fork();
                        let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
                        disk.inject(
                            (!flush_fault).then_some(after),
                            flush_fault.then_some(after),
                            false,
                        );
                        assert!(volume
                            .rename_child(parent.as_bytes(), b"before", b"after")
                            .is_err());
                        drop(volume);
                        disk.reconnect(true);
                        let mut volume = Vfs::recover_existing(disk.clone()).unwrap();
                        assert_eq!(volume.metadata_path(old.as_bytes()).unwrap(), before);
                        assert_eq!(
                            volume.metadata_path(new.as_bytes()),
                            Err(StorageError::NotFound)
                        );
                        assert_eq!(volume.open_path(old_child.as_bytes()), Ok(child));
                        let result = volume
                            .rename_child(parent.as_bytes(), b"before", b"after")
                            .unwrap();
                        assert_eq!(result.inode(), before.inode);
                        assert_eq!(volume.metadata_path(new.as_bytes()).unwrap(), before);
                        assert_eq!(volume.open_path(new_child.as_bytes()), Ok(child));
                        let mut bytes = [0; 32];
                        assert_eq!(volume.read(child, &mut bytes), Ok(14));
                        assert_eq!(&bytes[..14], b"retained child");
                        let kept = volume.open(b"kept.txt").unwrap();
                        let length = volume.read(kept, &mut bytes).unwrap();
                        assert_eq!(&bytes[..length], b"retained neighbor");
                        drop(volume);
                        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).unwrap();
                    }
                }
            }
        }
    }

    #[test]
    fn replaces_a_file_without_a_remove_then_rename_gap() {
        let (mut volume, _) = Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        let old = volume.create(b"old-package").expect("old package");
        volume.write(old, b"old bytes").expect("old contents");
        let replacement = volume.create(b"next-package").expect("replacement");
        volume
            .write(replacement, b"new bytes")
            .expect("replacement contents");

        let destination = volume
            .replace(b"next-package", b"old-package")
            .expect("atomic replacement");
        assert_eq!(volume.open(b"next-package"), Err(StorageError::NotFound));
        let mut bytes = [0; 16];
        let length = volume
            .read(destination, &mut bytes)
            .expect("read replacement");
        assert_eq!(&bytes[..length], b"new bytes");
        assert_eq!(
            volume.read(old, &mut bytes),
            Err(StorageError::InvalidHandle)
        );
    }

    #[test]
    fn maps_flushes_and_unmaps_file_contents() {
        let (mut volume, _) = Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        let handle = volume.create(b"mapped").expect("create");
        volume.write(handle, b"before").expect("write");
        let mut mapping: FileMapping = volume.mmap(handle).expect("mmap");
        assert_eq!(mapping.bytes(), b"before");
        mapping.bytes_mut()[..5].copy_from_slice(b"after");
        mapping.set_length(5).expect("length");
        volume.flush_mapping(&mapping).expect("flush");
        let mut read_back = [0; 8];
        let length = volume.read(handle, &mut read_back).expect("read");
        assert_eq!(&read_back[..length], b"after");
        assert_eq!(mapping.handle(), handle);
    }

    #[test]
    fn rejects_malformed_superblock() {
        let mut device = MemoryBlockDevice::new();
        let mut sector = [0; SECTOR_SIZE];
        sector[56] = 0x53;
        sector[57] = 0xef;
        device.write_sector(2, &sector).expect("seed");
        assert!(matches!(
            Vfs::mount_or_format(device),
            Err(StorageError::Corrupt)
        ));
    }
    #[derive(Clone)]
    struct FaultDisk(std::sync::Arc<std::sync::Mutex<FaultState>>);
    // `live` includes volatile cache state. A write-through sector write or
    // a cached-device flush advances `durable`; power cuts occur after those
    // explicit durability boundaries.
    struct FaultState {
        live: Vec<u8>,
        durable: Vec<u8>,
        cached: bool,
        fail_write: Option<usize>,
        fail_flush: Option<usize>,
        fail_sector_writes: Vec<(u64, usize)>,
        cut_after_durable_event: Option<usize>,
        persistent: bool,
        offline: bool,
        writes: usize,
        flushes: usize,
        durable_events: usize,
        write_sectors: Vec<u64>,
    }
    impl FaultDisk {
        fn new(cached: bool) -> Self {
            Self(std::sync::Arc::new(std::sync::Mutex::new(FaultState {
                live: vec![0; super::EXT2_BLOCK_COUNT as usize * BLOCK_SIZE],
                durable: vec![0; super::EXT2_BLOCK_COUNT as usize * BLOCK_SIZE],
                cached,
                fail_write: None,
                fail_flush: None,
                fail_sector_writes: Vec::new(),
                cut_after_durable_event: None,
                persistent: false,
                offline: false,
                writes: 0,
                flushes: 0,
                durable_events: 0,
                write_sectors: Vec::new(),
            })))
        }
        fn fork(&self) -> Self {
            let state = self.0.lock().unwrap();
            let disk = Self::new(state.cached);
            {
                let mut copy = disk.0.lock().unwrap();
                copy.live.clone_from(&state.live);
                copy.durable.clone_from(&state.durable);
            }
            disk
        }
        fn inject(&self, write: Option<usize>, flush: Option<usize>, persistent: bool) {
            self.inject_with_cut(write, flush, persistent, None);
        }
        fn inject_with_cut(
            &self,
            write: Option<usize>,
            flush: Option<usize>,
            persistent: bool,
            cut_after_durable_event: Option<usize>,
        ) {
            let mut state = self.0.lock().unwrap();
            state.fail_write = write;
            state.fail_flush = flush;
            state.fail_sector_writes.clear();
            state.cut_after_durable_event = cut_after_durable_event;
            state.persistent = persistent;
            state.offline = false;
            state.writes = 0;
            state.flushes = 0;
            state.durable_events = 0;
            state.write_sectors.clear();
        }
        // Fail selected attempts to specific sectors. Each tuple contains a
        // sector number and the number of earlier writes to that sector to
        // allow. Multiple entries can target successive rollback attempts.
        fn inject_sector_failures(&self, failures: &[(u64, usize)]) {
            let mut state = self.0.lock().unwrap();
            state.fail_write = None;
            state.fail_flush = None;
            state.fail_sector_writes = failures.to_vec();
            state.cut_after_durable_event = None;
            state.persistent = false;
            state.offline = false;
            state.writes = 0;
            state.flushes = 0;
            state.durable_events = 0;
            state.write_sectors.clear();
        }
        fn reconnect(&self, power_cut: bool) {
            let mut state = self.0.lock().unwrap();
            if power_cut {
                let durable = state.durable.clone();
                state.live = durable;
            }
            state.offline = false;
            state.fail_write = None;
            state.fail_flush = None;
            state.fail_sector_writes.clear();
        }
        // Construct one selected-sector writeback order that a whole-cache
        // flush cannot produce by itself. This is a synthetic atomic-sector
        // persistence fixture, not a physical-device power-cut simulation.
        fn persist_durable_sector_from(&self, source: &Self, sector: u64) {
            let offset = sector as usize * SECTOR_SIZE;
            let mut bytes = [0; SECTOR_SIZE];
            {
                let state = source.0.lock().unwrap();
                bytes.copy_from_slice(&state.durable[offset..offset + SECTOR_SIZE]);
            }
            let mut state = self.0.lock().unwrap();
            state.durable[offset..offset + SECTOR_SIZE].copy_from_slice(&bytes);
            state.live[offset..offset + SECTOR_SIZE].copy_from_slice(&bytes);
        }
    }
    fn fault(counter: &mut Option<usize>) -> bool {
        match counter {
            Some(0) => {
                *counter = None;
                true
            }
            Some(value) => {
                *value -= 1;
                false
            }
            None => false,
        }
    }
    fn fault_sector_write(failures: &mut Vec<(u64, usize)>, sector: u64) -> bool {
        let mut index = 0;
        while index < failures.len() {
            if failures[index].0 == sector {
                if failures[index].1 == 0 {
                    failures.swap_remove(index);
                    return true;
                }
                failures[index].1 -= 1;
            }
            index += 1;
        }
        false
    }
    impl super::ReadOnlyBlockDevice for FaultDisk {
        fn read_sector(
            &mut self,
            sector: u64,
            output: &mut [u8; SECTOR_SIZE],
        ) -> Result<(), StorageError> {
            let state = self.0.lock().unwrap();
            let offset = sector as usize * SECTOR_SIZE;
            output.copy_from_slice(
                state
                    .live
                    .get(offset..offset + SECTOR_SIZE)
                    .ok_or(StorageError::Block)?,
            );
            Ok(())
        }
    }
    impl BlockDevice for FaultDisk {
        fn write_sector(
            &mut self,
            sector: u64,
            bytes: &[u8; SECTOR_SIZE],
        ) -> Result<(), StorageError> {
            let mut state = self.0.lock().unwrap();
            if state.offline {
                return Err(StorageError::Block);
            }
            if fault(&mut state.fail_write)
                || fault_sector_write(&mut state.fail_sector_writes, sector)
            {
                state.offline = state.persistent;
                return Err(StorageError::Block);
            }
            let offset = sector as usize * SECTOR_SIZE;
            let cached = state.cached;
            state
                .live
                .get_mut(offset..offset + SECTOR_SIZE)
                .ok_or(StorageError::Block)?
                .copy_from_slice(bytes);
            if !cached {
                state.durable[offset..offset + SECTOR_SIZE].copy_from_slice(bytes);
            }
            state.writes += 1;
            state.write_sectors.push(sector);
            if !cached {
                state.durable_events += 1;
                if fault(&mut state.cut_after_durable_event) {
                    state.offline = true;
                    return Err(StorageError::Block);
                }
            }
            Ok(())
        }
        fn flush(&mut self) -> Result<(), StorageError> {
            let mut state = self.0.lock().unwrap();
            if state.offline {
                return Err(StorageError::Block);
            }
            if fault(&mut state.fail_flush) {
                state.offline = state.persistent;
                return Err(StorageError::Block);
            }
            let cached = state.cached;
            if cached {
                let live = state.live.clone();
                state.durable = live;
            }
            state.flushes += 1;
            if cached {
                state.durable_events += 1;
                if fault(&mut state.cut_after_durable_event) {
                    state.offline = true;
                    return Err(StorageError::Block);
                }
            }
            Ok(())
        }
    }
    fn crowded_name(index: usize) -> Vec<u8> {
        std::format!("{index:02}{}", "x".repeat(30)).into_bytes()
    }
    fn failure_baseline(cached: bool, crowded: bool) -> FaultDisk {
        let disk = FaultDisk::new(cached);
        let (mut volume, _) = Vfs::mount_or_format(disk.clone()).unwrap();
        let kept = volume.create(b"kept.txt").unwrap();
        volume.write(kept, b"retained neighbor").unwrap();
        if crowded {
            for index in 0..14 {
                let name = crowded_name(index);
                let handle = volume.create(&name).unwrap();
                volume.write(handle, &name).unwrap();
            }
        }
        volume.flush().unwrap();
        disk
    }
    fn failure_operation(
        volume: &mut Vfs<FaultDisk>,
        rename: bool,
    ) -> Result<FileHandle, StorageError> {
        if rename {
            volume.rename(&crowded_name(0), b"renamed")
        } else {
            volume.create(b"created")
        }
    }
    fn assert_recovered(disk: FaultDisk, rename: bool, success: bool, context: &str) {
        let before = {
            let state = disk.0.lock().unwrap();
            (state.live.clone(), state.writes, state.flushes)
        };
        let mut volume = match Vfs::mount_existing(disk.clone()) {
            Ok(volume) => volume,
            Err(StorageError::RecoveryRequired) => {
                assert_eq!(
                    Vfs::mount_or_format(disk.clone()).err(),
                    Some(StorageError::RecoveryRequired)
                );
                assert_eq!(
                    Vfs::<FaultDisk>::check_existing(&mut disk.clone()),
                    Err(StorageError::RecoveryRequired)
                );
                let state = disk.0.lock().unwrap();
                assert_eq!((state.live.clone(), state.writes, state.flushes), before);
                drop(state);
                Vfs::recover_existing(disk.clone()).expect("explicit recovery")
            }
            Err(error) => panic!("mount failed before recovery: {error:?}"),
        };
        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).expect("recovered integrity");
        let mut output = [0; 64];
        let kept = volume.open(b"kept.txt").unwrap();
        let count = volume.read_at(kept, 0, &mut output).unwrap();
        assert_eq!(&output[..count], b"retained neighbor");
        let mut entries = [DirectoryEntry::empty(); 64];
        let count = volume.list_root(&mut entries).unwrap();
        for (index, entry) in entries[..count].iter().enumerate() {
            assert!(
                entries[..index]
                    .iter()
                    .all(|other| other.inode != entry.inode),
                "inode alias"
            );
        }
        if rename {
            let old = volume.open(&crowded_name(0));
            let new = volume.open(b"renamed");
            assert_eq!(old.is_ok(), !success, "old rename name state: {context}");
            assert_eq!(new.is_ok(), success, "new rename name state: {context}");
            for index in 1..14 {
                let name = crowded_name(index);
                let handle = volume.open(&name).unwrap();
                let count = volume.read_at(handle, 0, &mut output).unwrap();
                assert_eq!(&output[..count], &name);
            }
            let handle = old.or(new).unwrap();
            let count = volume.read_at(handle, 0, &mut output).unwrap();
            assert_eq!(&output[..count], crowded_name(0));
            assert_eq!(count, 32);
        } else {
            assert_eq!(
                volume.open(b"created").is_ok(),
                success,
                "create state: {context}"
            );
            let next = volume.create(b"next").unwrap();
            if let Ok(created) = volume.open(b"created") {
                assert_ne!(created.inode(), next.inode());
            }
        }
        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).unwrap();
    }

    const SPLIT_ENTRY_NAME: &[u8] = b"cross-sector-dir-entry";
    const SPLIT_ENTRY_PATH: &[u8] = b"/cross-sector-dir-entry";

    #[derive(Clone, Copy, Debug)]
    enum CreateKind {
        RegularFile,
        Directory,
    }

    fn create_entry(
        volume: &mut Vfs<FaultDisk>,
        kind: CreateKind,
        name: &[u8],
    ) -> Result<(), StorageError> {
        match kind {
            CreateKind::RegularFile => volume.create(name).map(|_| ()),
            CreateKind::Directory => volume.mkdir(name),
        }
    }

    fn split_entry_baseline(cached: bool) -> FaultDisk {
        let disk = FaultDisk::new(cached);
        let (mut volume, _) = Vfs::mount_or_format(disk.clone()).expect("synthetic format");
        let kept = volume.create(b"kept.txt").expect("kept file");
        volume
            .write(kept, b"retained neighbor")
            .expect("kept contents");
        for index in 0..10 {
            volume
                .create(&crowded_name(index))
                .expect("crowded root entry");
        }
        // The kept file and ten 32-byte names leave the free record at 440.
        // Two legal 20-byte names each consume a 28-byte record, placing it
        // at byte 496 so the next entry crosses the 512-byte sector boundary.
        for name in [b"aaaaaaaaaaaaaaaaaaaa", b"bbbbbbbbbbbbbbbbbbbb"] {
            assert_eq!(name.len(), 20);
            assert!(name.len() <= MAX_NAME_LENGTH);
            volume.create(name).expect("legal padding root entry");
        }
        volume.flush().expect("flush synthetic baseline");

        let mut device = disk.clone();
        let mut directory = [0; BLOCK_SIZE];
        read_device_block(&mut device, ROOT_DIRECTORY_BLOCK, &mut directory)
            .expect("read synthetic root");
        assert_eq!(
            read_u32(&directory, 496),
            0,
            "free record starts at byte 496"
        );
        assert_eq!(read_u16(&directory, 500), 528, "rec_len is at byte 500");
        disk
    }

    fn successful_create_trace(baseline: &FaultDisk, kind: CreateKind) -> (Vec<u64>, usize) {
        let disk = baseline.fork();
        let mut volume = Vfs::mount_existing(disk.clone()).expect("mount probe baseline");
        disk.inject(None, None, false);
        create_entry(&mut volume, kind, SPLIT_ENTRY_NAME)
            .expect("successful synthetic create probe");
        let state = disk.0.lock().unwrap();
        (state.write_sectors.clone(), state.flushes)
    }

    fn assert_kept_neighbor(volume: &mut Vfs<FaultDisk>, context: &str) {
        let kept = volume
            .open_path(b"/kept.txt")
            .unwrap_or_else(|error| panic!("kept neighbor open: {error:?}: {context}"));
        let mut output = [0; 32];
        let count = volume
            .read_at(kept, 0, &mut output)
            .unwrap_or_else(|error| panic!("kept neighbor read: {error:?}: {context}"));
        assert_eq!(&output[..count], b"retained neighbor", "{context}");
    }

    fn assert_create_rolled_back(
        disk: &FaultDisk,
        volume: &mut Vfs<FaultDisk>,
        kind: CreateKind,
        context: &str,
    ) {
        Vfs::<FaultDisk>::check_existing(&mut disk.clone())
            .unwrap_or_else(|error| panic!("integrity after rollback: {error:?}: {context}"));
        assert_eq!(
            volume.open_path(SPLIT_ENTRY_PATH),
            Err(StorageError::NotFound),
            "failed create must not leave a parent reference: {context}"
        );
        assert_kept_neighbor(volume, context);
        create_entry(volume, kind, SPLIT_ENTRY_NAME)
            .unwrap_or_else(|error| panic!("create retry: {error:?}: {context}"));
        Vfs::<FaultDisk>::check_existing(&mut disk.clone())
            .unwrap_or_else(|error| panic!("integrity after retry: {error:?}: {context}"));
        match kind {
            CreateKind::RegularFile => assert!(
                volume.open_path(SPLIT_ENTRY_PATH).is_ok(),
                "regular file must exist after retry: {context}"
            ),
            CreateKind::Directory => assert_eq!(
                volume.open_path(SPLIT_ENTRY_PATH),
                Err(StorageError::IsDirectory),
                "directory must exist after retry: {context}"
            ),
        }
        assert_kept_neighbor(volume, context);
    }

    #[test]
    fn create_and_mkdir_undo_restore_every_single_write_and_flush_failure() {
        for cached in [false, true] {
            for kind in [CreateKind::RegularFile, CreateKind::Directory] {
                let baseline = split_entry_baseline(cached);
                let (write_sectors, flushes) = successful_create_trace(&baseline, kind);
                let root_first_sector = u64::from(ROOT_DIRECTORY_BLOCK) * 2;
                let first_parent_write = write_sectors
                    .iter()
                    .position(|sector| *sector == root_first_sector)
                    .expect("parent first sector write");
                let second_parent_write = write_sectors
                    .iter()
                    .position(|sector| *sector == root_first_sector + 1)
                    .expect("parent second sector write");
                assert!(
                    first_parent_write < second_parent_write,
                    "cached={cached} {kind:?}"
                );

                for fail_after in 0..write_sectors.len() {
                    let disk = baseline.fork();
                    let mut volume = Vfs::mount_existing(disk.clone()).expect("mount write case");
                    disk.inject(Some(fail_after), None, false);
                    let context = std::format!("cached={cached} kind={kind:?} write={fail_after}");
                    assert_eq!(
                        create_entry(&mut volume, kind, SPLIT_ENTRY_NAME),
                        Err(StorageError::Block),
                        "one-shot write fault: {context}"
                    );
                    assert_create_rolled_back(&disk, &mut volume, kind, &context);
                }

                for fail_after in 0..flushes {
                    let disk = baseline.fork();
                    let mut volume = Vfs::mount_existing(disk.clone()).expect("mount flush case");
                    disk.inject(None, Some(fail_after), false);
                    let context = std::format!("cached={cached} kind={kind:?} flush={fail_after}");
                    assert_eq!(
                        create_entry(&mut volume, kind, SPLIT_ENTRY_NAME),
                        Err(StorageError::Block),
                        "one-shot flush fault: {context}"
                    );
                    assert_create_rolled_back(&disk, &mut volume, kind, &context);
                }
            }
        }
    }

    #[test]
    fn create_and_mkdir_persistent_parent_fault_requires_explicit_recovery() {
        for cached in [false, true] {
            for kind in [CreateKind::RegularFile, CreateKind::Directory] {
                let baseline = split_entry_baseline(cached);
                let (write_sectors, _) = successful_create_trace(&baseline, kind);
                let parent_second_sector = u64::from(ROOT_DIRECTORY_BLOCK) * 2 + 1;
                let fail_after = write_sectors
                    .iter()
                    .position(|sector| *sector == parent_second_sector)
                    .expect("parent second-sector write");
                let disk = baseline.fork();
                let mut volume = Vfs::mount_existing(disk.clone()).expect("mount baseline");
                disk.inject(Some(fail_after), None, true);
                assert_eq!(
                    create_entry(&mut volume, kind, SPLIT_ENTRY_NAME),
                    Err(StorageError::Indeterminate),
                    "cached={cached} kind={kind:?}"
                );
                assert_eq!(
                    create_entry(&mut volume, kind, b"blocked-while-recovery-pending"),
                    Err(StorageError::RecoveryRequired),
                    "same Vfs must fail closed: cached={cached} kind={kind:?}"
                );

                disk.reconnect(true);
                let before_inspection = {
                    let state = disk.0.lock().unwrap();
                    (state.live.clone(), state.writes, state.flushes)
                };
                assert_eq!(
                    Vfs::mount_existing(disk.clone()).err(),
                    Some(StorageError::RecoveryRequired),
                    "ordinary remount must not repair: cached={cached} kind={kind:?}"
                );
                assert_eq!(
                    Vfs::<FaultDisk>::check_existing(&mut disk.clone()),
                    Err(StorageError::RecoveryRequired),
                    "read-only check must report pending undo: cached={cached} kind={kind:?}"
                );
                {
                    let state = disk.0.lock().unwrap();
                    assert_eq!(
                        (state.live.clone(), state.writes, state.flushes),
                        before_inspection,
                        "inspection must not mutate pending state: cached={cached} kind={kind:?}"
                    );
                }

                let mut recovered = Vfs::recover_existing(disk.clone()).unwrap_or_else(|error| {
                    panic!("explicit recovery: {error:?}: cached={cached} kind={kind:?}")
                });
                Vfs::<FaultDisk>::check_existing(&mut disk.clone())
                    .expect("integrity after explicit recovery");
                assert_eq!(
                    recovered.open_path(SPLIT_ENTRY_PATH),
                    Err(StorageError::NotFound)
                );
                assert_kept_neighbor(&mut recovered, "persistent parent-sector failure");
                create_entry(&mut recovered, kind, SPLIT_ENTRY_NAME)
                    .expect("retry after explicit recovery");
                Vfs::<FaultDisk>::check_existing(&mut disk.clone())
                    .expect("integrity after recovered retry");
            }
        }
    }

    #[test]
    fn create_and_mkdir_parent_rollback_failure_keeps_allocations_until_recovery_retry() {
        for kind in [CreateKind::RegularFile, CreateKind::Directory] {
            for fail_second_parent_restore_sector in [false, true] {
                let baseline = split_entry_baseline(false);
                let disk = baseline.fork();
                let mut volume = Vfs::mount_existing(disk.clone()).expect("mount baseline");
                let inode_bit = volume
                    .find_free_bit(INODE_BITMAP, FIRST_FILE_INODE - 1, EXT2_INODE_COUNT)
                    .expect("free inode");
                let inode = inode_bit + 1;
                let data_block = volume
                    .find_free_bit(BLOCK_BITMAP, RESERVED_BLOCKS, EXT2_BLOCK_COUNT)
                    .expect("free data block");

                let root_first_sector = u64::from(ROOT_DIRECTORY_BLOCK) * 2;
                let root_second_sector = root_first_sector + 1;
                if fail_second_parent_restore_sector {
                    // Let the operation write the complete parent block, then fail the
                    // free-count update and the second-sector undo write.
                    disk.inject_sector_failures(&[
                        (root_second_sector, 1),
                        (u64::from(GROUP_DESCRIPTOR_BLOCK) * 2, 0),
                    ]);
                } else {
                    // Fail the second parent-sector write, then fail rollback
                    // before it can restore the first sector's entry header.
                    disk.inject_sector_failures(&[(root_second_sector, 0), (root_first_sector, 1)]);
                }
                assert_eq!(
                    create_entry(&mut volume, kind, SPLIT_ENTRY_NAME),
                    Err(StorageError::Indeterminate),
                    "kind={kind:?} fail_second_parent_restore_sector={fail_second_parent_restore_sector}"
                );
                assert_eq!(
                    create_entry(&mut volume, kind, b"blocked-while-rollback-pending"),
                    Err(StorageError::RecoveryRequired)
                );
                assert_eq!(
                    Vfs::<FaultDisk>::check_existing(&mut disk.clone()),
                    Err(StorageError::RecoveryRequired)
                );
                assert_eq!(
                    Vfs::mount_existing(disk.clone()).err(),
                    Some(StorageError::RecoveryRequired)
                );

                let mut device = disk.clone();
                let mut inode_bitmap = [0; BLOCK_SIZE];
                let mut block_bitmap = [0; BLOCK_SIZE];
                let mut parent = [0; BLOCK_SIZE];
                read_device_block(&mut device, INODE_BITMAP, &mut inode_bitmap)
                    .expect("read pending inode bitmap");
                read_device_block(&mut device, BLOCK_BITMAP, &mut block_bitmap)
                    .expect("read pending block bitmap");
                read_device_block(&mut device, ROOT_DIRECTORY_BLOCK, &mut parent)
                    .expect("read pending parent");
                assert!(
                    is_bit_set(&inode_bitmap, inode - 1),
                    "child inode stays allocated"
                );
                assert!(
                    is_bit_set(&block_bitmap, data_block),
                    "child data block stays allocated"
                );
                assert_eq!(
                    read_u32(&parent, 496),
                    if fail_second_parent_restore_sector {
                        0
                    } else {
                        inode
                    },
                    "parent ref state while undo is pending"
                );
                assert_eq!(
                    read_u16(&parent, 500),
                    if fail_second_parent_restore_sector {
                        528
                    } else {
                        32
                    },
                    "parent rec_len state while undo is pending"
                );

                // Fail one explicit recovery attempt as well. The marker and
                // allocations must survive so recovery can retry safely.
                disk.inject(Some(0), None, true);
                assert_eq!(
                    Vfs::recover_existing(disk.clone()).err(),
                    Some(StorageError::Block)
                );
                disk.reconnect(false);
                let mut recovered = Vfs::recover_existing(disk.clone()).expect("recovery retry");
                Vfs::<FaultDisk>::check_existing(&mut disk.clone())
                    .expect("integrity after recovery retry");
                assert_eq!(
                    recovered.open_path(SPLIT_ENTRY_PATH),
                    Err(StorageError::NotFound)
                );

                let mut device = disk.clone();
                read_device_block(&mut device, INODE_BITMAP, &mut inode_bitmap)
                    .expect("read recovered inode bitmap");
                read_device_block(&mut device, BLOCK_BITMAP, &mut block_bitmap)
                    .expect("read recovered block bitmap");
                assert!(
                    !is_bit_set(&inode_bitmap, inode - 1),
                    "recovery releases child inode"
                );
                assert!(
                    !is_bit_set(&block_bitmap, data_block),
                    "recovery releases child block"
                );
                assert_kept_neighbor(&mut recovered, "rollback parent retry");
                create_entry(&mut recovered, kind, SPLIT_ENTRY_NAME)
                    .expect("create after recovery retry");
                Vfs::<FaultDisk>::check_existing(&mut disk.clone())
                    .expect("integrity after post-recovery create");
            }
        }
    }

    #[test]
    fn create_and_mkdir_reject_inconsistent_zero_free_counts_without_writes() {
        for kind in [CreateKind::RegularFile, CreateKind::Directory] {
            let disk = FaultDisk::new(false);
            let (mut volume, _) = Vfs::mount_or_format(disk.clone()).expect("synthetic format");
            {
                let mut state = disk.0.lock().unwrap();
                let superblock = SUPERBLOCK_BLOCK as usize * BLOCK_SIZE;
                state.live[superblock + 12..superblock + 20].fill(0);
                state.durable[superblock + 12..superblock + 20].fill(0);
                let group = GROUP_DESCRIPTOR_BLOCK as usize * BLOCK_SIZE;
                state.live[group + 12..group + 16].fill(0);
                state.durable[group + 12..group + 16].fill(0);
            }
            let before = disk.0.lock().unwrap().live.clone();
            disk.inject(None, None, false);
            assert_eq!(
                create_entry(&mut volume, kind, b"zero-free-count"),
                Err(StorageError::Corrupt),
                "kind={kind:?}"
            );
            let state = disk.0.lock().unwrap();
            assert_eq!(state.writes, 0, "kind={kind:?}");
            assert_eq!(state.live, before, "kind={kind:?}");
            assert_eq!(state.durable, before, "kind={kind:?}");
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum DeleteMode {
        PathFile,
        RootFile,
        Directory,
    }

    fn delete_baseline(cached: bool, mode: DeleteMode) -> (FaultDisk, std::vec::Vec<u8>) {
        let disk = FaultDisk::new(cached);
        let (mut volume, _) = Vfs::mount_or_format(disk.clone()).unwrap();
        let kept = volume.create(b"kept.txt").unwrap();
        volume.write(kept, b"retained neighbor").unwrap();
        let contents = match mode {
            DeleteMode::Directory => {
                volume.mkdir_path(b"/empty").unwrap();
                std::vec::Vec::new()
            }
            DeleteMode::PathFile | DeleteMode::RootFile => {
                let file = volume.create(b"removed").unwrap();
                let length = (DIRECT_BLOCKS + 2) * BLOCK_SIZE + 37;
                let contents = (0..length)
                    .map(|index| (index as u8).wrapping_mul(37))
                    .collect::<std::vec::Vec<_>>();
                volume.write(file, &contents).unwrap();
                contents
            }
        };
        volume.flush().unwrap();
        (disk, contents)
    }

    fn delete_operation(volume: &mut Vfs<FaultDisk>, mode: DeleteMode) -> Result<(), StorageError> {
        match mode {
            DeleteMode::PathFile => volume.remove_path(b"/removed"),
            DeleteMode::RootFile => volume.remove(b"removed"),
            DeleteMode::Directory => volume.rmdir_path(b"/empty"),
        }
    }

    fn delete_identity(disk: &FaultDisk) -> (FileHandle, FileMetadata) {
        let mut volume = Vfs::mount_existing(disk.clone()).expect("baseline mount");
        let handle = volume.open_path(b"/removed").expect("baseline file");
        let metadata = volume.metadata(handle).expect("baseline metadata");
        (handle, metadata)
    }

    fn delete_metadata_differs(baseline: &FaultDisk, candidate: &FaultDisk, inode: u32) -> bool {
        let inode_block = INODE_TABLE + (inode - 1) / 8;
        let inode_offset = ((inode - 1) % 8) * EXT2_INODE_SIZE as u32;
        let inode_sector = inode_block * 2 + inode_offset / SECTOR_SIZE as u32;
        let sectors = [
            ROOT_DIRECTORY_BLOCK * 2,
            BLOCK_BITMAP * 2,
            INODE_BITMAP * 2,
            inode_sector,
            SUPERBLOCK_BLOCK * 2,
            GROUP_DESCRIPTOR_BLOCK * 2,
        ];
        let baseline_state = baseline.0.lock().unwrap();
        let candidate_state = candidate.0.lock().unwrap();
        sectors.into_iter().any(|sector| {
            let offset = sector as usize * SECTOR_SIZE;
            baseline_state.durable[offset..offset + SECTOR_SIZE]
                != candidate_state.durable[offset..offset + SECTOR_SIZE]
        })
    }

    fn assert_delete_identity_restored(
        disk: FaultDisk,
        expected_handle: FileHandle,
        expected_metadata: FileMetadata,
        expected_contents: &[u8],
        context: &str,
    ) {
        let mut volume = match Vfs::mount_existing(disk.clone()) {
            Ok(volume) => volume,
            Err(StorageError::RecoveryRequired) => {
                Vfs::recover_existing(disk.clone()).expect("retry explicit recovery")
            }
            Err(error) => panic!("mount failed after recovery fault: {error:?}: {context}"),
        };
        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).expect("recovered integrity");
        let handle = volume.open_path(b"/removed").expect("restored file");
        assert_eq!(handle, expected_handle, "handle identity: {context}");
        let metadata = volume.metadata(handle).expect("restored metadata");
        assert_eq!(metadata.inode, expected_metadata.inode, "inode: {context}");
        assert_eq!(
            metadata.generation, expected_metadata.generation,
            "generation: {context}"
        );

        let mut bytes = std::vec![0; expected_contents.len()];
        let count = volume
            .read_at(handle, 0, &mut bytes)
            .expect("restored read");
        assert_eq!(count, expected_contents.len(), "file length: {context}");
        assert_eq!(
            &bytes[..count],
            expected_contents,
            "file contents: {context}"
        );

        let kept = volume.open_path(b"/kept.txt").expect("kept neighbor");
        let mut kept_bytes = [0; 32];
        let count = volume
            .read_at(kept, 0, &mut kept_bytes)
            .expect("kept neighbor read");
        assert_eq!(
            &kept_bytes[..count],
            b"retained neighbor",
            "neighbor: {context}"
        );
    }

    fn retry_delete_from_each_recovery_fault(
        pending: &FaultDisk,
        expected_handle: FileHandle,
        expected_metadata: FileMetadata,
        expected_contents: &[u8],
        context: &str,
    ) {
        let probe = pending.fork();
        probe.inject(None, None, false);
        Vfs::recover_existing(probe.clone()).expect("recovery probe");
        let (writes, flushes) = {
            let state = probe.0.lock().unwrap();
            (state.writes, state.flushes)
        };
        assert!(writes > 0, "recovery must write: {context}");
        assert!(flushes > 0, "recovery must flush: {context}");

        for after in 0..writes {
            let attempt = pending.fork();
            attempt.inject(Some(after), None, true);
            assert!(
                Vfs::recover_existing(attempt.clone()).is_err(),
                "recovery write fault {after} should fail: {context}"
            );
            attempt.reconnect(true);
            assert_delete_identity_restored(
                attempt,
                expected_handle,
                expected_metadata,
                expected_contents,
                &std::format!("{context}; recovery write fault {after}"),
            );
        }
        for after in 0..flushes {
            let attempt = pending.fork();
            attempt.inject(None, Some(after), true);
            assert!(
                Vfs::recover_existing(attempt.clone()).is_err(),
                "recovery flush fault {after} should fail: {context}"
            );
            attempt.reconnect(true);
            assert_delete_identity_restored(
                attempt,
                expected_handle,
                expected_metadata,
                expected_contents,
                &std::format!("{context}; recovery flush fault {after}"),
            );
        }
    }

    fn assert_delete_recovered(
        disk: FaultDisk,
        mode: DeleteMode,
        removed: bool,
        expected_data: &[u8],
        context: &str,
    ) {
        let before = {
            let state = disk.0.lock().unwrap();
            (state.live.clone(), state.writes, state.flushes)
        };
        let mut volume = match Vfs::mount_existing(disk.clone()) {
            Ok(volume) => volume,
            Err(StorageError::RecoveryRequired) => {
                assert_eq!(
                    Vfs::mount_or_format(disk.clone()).err(),
                    Some(StorageError::RecoveryRequired),
                    "ordinary format mount must not repair: {context}"
                );
                assert_eq!(
                    Vfs::<FaultDisk>::check_existing(&mut disk.clone()),
                    Err(StorageError::RecoveryRequired),
                    "read-only check must report the pending delete: {context}"
                );
                let state = disk.0.lock().unwrap();
                assert_eq!(
                    (state.live.clone(), state.writes, state.flushes),
                    before,
                    "mount/check changed a pending delete: {context}"
                );
                drop(state);
                Vfs::recover_existing(disk.clone()).expect("explicit delete recovery")
            }
            Err(error) => panic!("mount failed before delete recovery: {error:?}: {context}"),
        };
        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).expect("delete recovery integrity");

        let kept = volume.open_path(b"/kept.txt").expect("kept neighbor");
        let mut kept_bytes = [0; 32];
        let kept_count = volume.read_at(kept, 0, &mut kept_bytes).unwrap();
        assert_eq!(&kept_bytes[..kept_count], b"retained neighbor", "{context}");

        match mode {
            DeleteMode::Directory => {
                assert_eq!(
                    volume.metadata_path(b"/empty").is_ok(),
                    !removed,
                    "{context}"
                );
            }
            DeleteMode::PathFile | DeleteMode::RootFile => {
                let handle = volume.open_path(b"/removed");
                assert_eq!(handle.is_ok(), !removed, "delete outcome: {context}");
                if let Ok(handle) = handle {
                    let mut bytes = std::vec![0; expected_data.len()];
                    let count = volume.read_at(handle, 0, &mut bytes).unwrap();
                    assert_eq!(count, expected_data.len(), "file length: {context}");
                    assert_eq!(&bytes[..count], expected_data, "file data: {context}");
                }
            }
        }
        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).expect("final delete integrity");
    }

    fn empty_file_with_zero_block_count(cached: bool) -> FaultDisk {
        let disk = FaultDisk::new(cached);
        let (mut volume, _) = Vfs::mount_or_format(disk.clone()).unwrap();
        let empty = volume.create(b"empty").unwrap();
        let mut inode = volume.read_inode(empty.inode()).unwrap();
        assert_eq!(inode.size, 0);
        inode.blocks = 0;
        volume.write_inode(empty.inode(), inode).unwrap();
        let kept = volume.create(b"kept.txt").unwrap();
        volume.write(kept, b"retained neighbor").unwrap();
        volume.flush().unwrap();
        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).unwrap();
        disk
    }

    #[test]
    fn zero_block_count_empty_files_unlink_through_both_entrypoints() {
        for cached in [false, true] {
            for mode in [DeleteMode::PathFile, DeleteMode::RootFile] {
                let disk = empty_file_with_zero_block_count(cached);
                let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
                let metadata = volume.metadata_path(b"/empty").unwrap();
                assert_eq!(metadata.size, 0);
                assert_eq!(metadata.blocks, 0);

                let result = match mode {
                    DeleteMode::PathFile => volume.remove_path(b"/empty"),
                    DeleteMode::RootFile => volume.remove(b"empty"),
                    DeleteMode::Directory => unreachable!(),
                };
                assert_eq!(result, Ok(()), "cached={cached} mode={mode:?}");
                assert_eq!(volume.open_path(b"/empty"), Err(StorageError::NotFound));
                let kept = volume.open_path(b"/kept.txt").unwrap();
                let mut bytes = [0; 32];
                let count = volume.read_at(kept, 0, &mut bytes).unwrap();
                assert_eq!(&bytes[..count], b"retained neighbor");
                Vfs::<FaultDisk>::check_existing(&mut disk.clone()).unwrap();
            }
        }
    }

    #[test]
    fn all_remove_entrypoints_recover_every_write_and_flush_failure() {
        for cached in [false, true] {
            for mode in [
                DeleteMode::PathFile,
                DeleteMode::RootFile,
                DeleteMode::Directory,
            ] {
                let (baseline, expected_data) = delete_baseline(cached, mode);
                let probe = baseline.fork();
                let mut volume = Vfs::mount_existing(probe.clone()).unwrap();
                delete_operation(&mut volume, mode).unwrap();
                let (writes, flushes) = {
                    let state = probe.0.lock().unwrap();
                    (state.writes, state.flushes)
                };
                for persistent in [false, true] {
                    for flush_fault in [false, true] {
                        let limit = if flush_fault { flushes } else { writes };
                        for after in 0..=limit {
                            let disk = baseline.fork();
                            let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
                            disk.inject(
                                (!flush_fault).then_some(after),
                                flush_fault.then_some(after),
                                persistent,
                            );
                            let result = delete_operation(&mut volume, mode);
                            disk.reconnect(cached && persistent);
                            let commit_visible_after_error =
                                !cached && persistent && flush_fault && after + 1 == flushes;
                            let removed = result.is_ok() || commit_visible_after_error;
                            let context = std::format!(
                                "cached={cached} mode={mode:?} persistent={persistent} flush_fault={flush_fault} after={after}"
                            );
                            assert_delete_recovered(disk, mode, removed, &expected_data, &context);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn root_remove_uses_delete_undo() {
        let (baseline, expected_data) = delete_baseline(false, DeleteMode::RootFile);
        let disk = baseline.fork();
        let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
        disk.inject(Some(4), None, true);
        assert_eq!(
            delete_operation(&mut volume, DeleteMode::RootFile),
            Err(StorageError::Indeterminate)
        );
        disk.reconnect(false);
        assert_delete_recovered(
            disk,
            DeleteMode::RootFile,
            false,
            &expected_data,
            "root remove after persistent directory-write failure",
        );
    }

    #[test]
    fn pending_delete_recovery_retries_from_progressed_power_cut_states() {
        for cached in [false, true] {
            let (baseline, expected_contents) = delete_baseline(cached, DeleteMode::PathFile);
            let (expected_handle, expected_metadata) = delete_identity(&baseline);

            let probe = baseline.fork();
            let mut probe_volume = Vfs::mount_existing(probe.clone()).unwrap();
            probe.inject(None, None, false);
            delete_operation(&mut probe_volume, DeleteMode::PathFile).unwrap();
            let events = probe.0.lock().unwrap().durable_events;
            assert!(events > 0, "cached={cached}");

            let mut progressed_states = 0;
            for after in 0..events {
                let disk = baseline.fork();
                let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
                disk.inject_with_cut(None, None, false, Some(after));
                assert!(
                    delete_operation(&mut volume, DeleteMode::PathFile).is_err(),
                    "power cut after event {after} must interrupt deletion: cached={cached}"
                );
                disk.reconnect(true);

                let progressed = delete_metadata_differs(&baseline, &disk, expected_handle.inode());
                match Vfs::mount_existing(disk.clone()) {
                    Err(StorageError::RecoveryRequired) if progressed => {
                        progressed_states += 1;
                        assert_eq!(
                            Vfs::<FaultDisk>::check_existing(&mut disk.clone()),
                            Err(StorageError::RecoveryRequired),
                            "pending cut remains read-only: cached={cached} event={after}"
                        );
                        retry_delete_from_each_recovery_fault(
                            &disk,
                            expected_handle,
                            expected_metadata,
                            &expected_contents,
                            &std::format!("cached={cached}; delete cut after event {after}"),
                        );
                    }
                    Err(StorageError::RecoveryRequired) => {
                        // The journal reached stable storage before the delete
                        // mutation did. The existing marker-only case is covered
                        // by the general recovery tests.
                    }
                    Ok(mut volume) if progressed => {
                        Vfs::<FaultDisk>::check_existing(&mut disk.clone())
                            .expect("committed delete integrity");
                        assert_eq!(
                            volume.open_path(b"/removed"),
                            Err(StorageError::NotFound),
                            "delete completed before journal clear became visible: cached={cached} event={after}"
                        );
                    }
                    Ok(mut volume) => {
                        assert_eq!(
                            volume.open_path(b"/removed"),
                            Ok(expected_handle),
                            "delete had not started: cached={cached} event={after}"
                        );
                    }
                    Err(error) => panic!(
                        "unexpected mount after delete cut: {error:?}; cached={cached} event={after}"
                    ),
                }
            }

            assert!(
                progressed_states >= if cached { 1 } else { 4 },
                "expected several progressed pending states, got {progressed_states}: cached={cached}"
            );
        }
    }

    #[test]
    fn cached_delete_recovery_accepts_inode_sector_persisted_before_bitmap() {
        let (baseline, expected_contents) = delete_baseline(true, DeleteMode::PathFile);
        let (expected_handle, expected_metadata) = delete_identity(&baseline);

        let completed = baseline.fork();
        let mut completed_volume = Vfs::mount_existing(completed.clone()).unwrap();
        delete_operation(&mut completed_volume, DeleteMode::PathFile).unwrap();

        let probe = baseline.fork();
        let mut probe_volume = Vfs::mount_existing(probe.clone()).unwrap();
        probe.inject(None, None, false);
        delete_operation(&mut probe_volume, DeleteMode::PathFile).unwrap();
        let events = probe.0.lock().unwrap().durable_events;

        let mut marker_only = None;
        for after in 0..events {
            let candidate = baseline.fork();
            let mut volume = Vfs::mount_existing(candidate.clone()).unwrap();
            candidate.inject_with_cut(None, None, false, Some(after));
            assert!(delete_operation(&mut volume, DeleteMode::PathFile).is_err());
            candidate.reconnect(true);
            if Vfs::mount_existing(candidate.clone()).err() == Some(StorageError::RecoveryRequired)
                && !delete_metadata_differs(&baseline, &candidate, expected_handle.inode())
            {
                marker_only = Some(candidate);
                break;
            }
        }
        let pending = marker_only.expect("cached cut after journal publication");
        let inode_block = INODE_TABLE + (expected_handle.inode() - 1) / 8;
        let inode_offset = ((expected_handle.inode() - 1) % 8) * EXT2_INODE_SIZE as u32;
        let inode_sector = inode_block * 2 + inode_offset / SECTOR_SIZE as u32;
        pending.persist_durable_sector_from(&completed, u64::from(inode_sector));

        let mut inode_bitmap = [0; BLOCK_SIZE];
        read_device_block(&mut pending.clone(), INODE_BITMAP, &mut inode_bitmap).unwrap();
        assert!(
            is_bit_set(&inode_bitmap, expected_handle.inode() - 1),
            "the synthetic reordered state leaves the inode bitmap allocated"
        );
        let current_inode =
            read_inode_bytes_from_device(&mut pending.clone(), expected_handle.inode()).unwrap();
        let previous_inode = expected_metadata_inode(&baseline, expected_handle.inode());
        assert!(cleared_inode_matches(&current_inode, &previous_inode));

        let before = {
            let state = pending.0.lock().unwrap();
            (state.live.clone(), state.writes, state.flushes)
        };
        assert_eq!(
            Vfs::mount_existing(pending.clone()).err(),
            Some(StorageError::RecoveryRequired)
        );
        assert_eq!(
            Vfs::<FaultDisk>::check_existing(&mut pending.clone()),
            Err(StorageError::RecoveryRequired)
        );

        let corrupt = pending.fork();
        let inode_record_offset = inode_block as usize * BLOCK_SIZE
            + ((expected_handle.inode() - 1) % 8) as usize * EXT2_INODE_SIZE as usize;
        {
            let mut state = corrupt.0.lock().unwrap();
            state.live[inode_record_offset + 2] = 1;
            state.durable[inode_record_offset + 2] = 1;
        }
        assert_eq!(
            Vfs::mount_existing(corrupt.clone()).err(),
            Some(StorageError::Corrupt),
            "unrecognized changes to an allocated inode must remain fail-closed"
        );

        {
            let state = pending.0.lock().unwrap();
            assert_eq!(
                (state.live.clone(), state.writes, state.flushes),
                before,
                "ordinary inspection must not repair reordered pending state"
            );
        }

        assert_delete_identity_restored(
            pending,
            expected_handle,
            expected_metadata,
            &expected_contents,
            "cached inode-sector writeback before inode bitmap",
        );
    }

    fn expected_metadata_inode(disk: &FaultDisk, inode: u32) -> [u8; EXT2_INODE_SIZE as usize] {
        read_inode_bytes_from_device(&mut disk.clone(), inode).expect("baseline inode")
    }

    fn v1_create_undo_fixture(
        nested: bool,
        with_owner_file: bool,
    ) -> (FaultDisk, Undo, u32, Option<u32>) {
        let disk = FaultDisk::new(false);
        let (mut volume, _) = Vfs::mount_or_format(disk.clone()).unwrap();
        if nested {
            volume.mkdir_path(b"/nested").unwrap();
        }
        let owner_inode = if with_owner_file {
            Some(volume.create(b"owner").unwrap().inode())
        } else {
            None
        };
        let directory_inode = if nested {
            volume.resolve_path(b"/nested").unwrap()
        } else {
            super::ROOT_INODE
        };
        let parent = volume.read_inode(directory_inode).unwrap();
        let mut directory = [0; BLOCK_SIZE];
        read_device_block(&mut disk.clone(), parent.direct_block, &mut directory).unwrap();

        let mut inode_bitmap = [0; BLOCK_SIZE];
        read_device_block(&mut disk.clone(), super::INODE_BITMAP, &mut inode_bitmap).unwrap();
        let inode = (super::FIRST_FILE_INODE..=super::EXT2_INODE_COUNT)
            .find(|candidate| !is_bit_set(&inode_bitmap, candidate - 1))
            .expect("free inode for an uncommitted create");
        let mut block_bitmap = [0; BLOCK_SIZE];
        read_device_block(&mut disk.clone(), super::BLOCK_BITMAP, &mut block_bitmap).unwrap();
        let data_block = (super::RESERVED_BLOCKS..super::EXT2_BLOCK_COUNT)
            .find(|candidate| !is_bit_set(&block_bitmap, *candidate))
            .expect("free block for an uncommitted create");
        drop(volume);

        (
            disk,
            Undo {
                kind: super::UNDO_CREATE,
                directory_inode,
                directory_block: parent.direct_block,
                inode,
                data_block,
                previous_inode: [0; EXT2_INODE_SIZE as usize],
                previous_indirect_checksum: [0; 32],
                directory,
            },
            directory_inode,
            owner_inode,
        )
    }

    fn install_v1_undo(disk: &FaultDisk, undo: &Undo) {
        let mut header = undo.header();
        header[..8].copy_from_slice(LEGACY_UNDO_MAGIC);
        let digest = Sha256::digest(&header[..480]);
        header[480..].copy_from_slice(&digest);
        let backup_start = super::UNDO_BLOCK as usize * BLOCK_SIZE;
        let mut state = disk.0.lock().unwrap();
        state.live[backup_start..backup_start + BLOCK_SIZE].copy_from_slice(&undo.directory);
        state.durable[backup_start..backup_start + BLOCK_SIZE].copy_from_slice(&undo.directory);
        state.live[..SECTOR_SIZE].copy_from_slice(&header);
        state.durable[..SECTOR_SIZE].copy_from_slice(&header);
    }

    fn set_inode_u32(disk: &FaultDisk, inode: u32, field_offset: usize, value: u32) {
        let block = super::INODE_TABLE + (inode - 1) / 8;
        let offset = block as usize * BLOCK_SIZE
            + ((inode - 1) % 8) as usize * EXT2_INODE_SIZE as usize
            + field_offset;
        let bytes = value.to_le_bytes();
        let mut state = disk.0.lock().unwrap();
        state.live[offset..offset + 4].copy_from_slice(&bytes);
        state.durable[offset..offset + 4].copy_from_slice(&bytes);
    }

    fn assert_corrupt_undo_is_read_only(disk: FaultDisk) {
        let before = {
            let state = disk.0.lock().unwrap();
            (
                state.live.clone(),
                state.durable.clone(),
                state.writes,
                state.flushes,
            )
        };
        assert_eq!(
            Vfs::mount_existing(disk.clone()).err(),
            Some(StorageError::Corrupt)
        );
        assert_eq!(
            Vfs::mount_or_format(disk.clone()).err(),
            Some(StorageError::Corrupt)
        );
        assert_eq!(
            Vfs::<FaultDisk>::check_existing(&mut disk.clone()),
            Err(StorageError::Corrupt)
        );
        assert_eq!(
            Vfs::recover_existing(disk.clone()).err(),
            Some(StorageError::Corrupt)
        );
        let state = disk.0.lock().unwrap();
        assert_eq!(
            (
                state.live.clone(),
                state.durable.clone(),
                state.writes,
                state.flushes,
            ),
            before,
            "invalid undo must leave the full volume and journal untouched"
        );
    }

    #[test]
    fn undo_rejects_nonroot_parent_alias_to_root_directory_block() {
        let (disk, mut undo, parent_inode, _) = v1_create_undo_fixture(true, false);
        let mut forged_directory = [0; BLOCK_SIZE];
        read_device_block(
            &mut disk.clone(),
            ROOT_DIRECTORY_BLOCK,
            &mut forged_directory,
        )
        .unwrap();
        forged_directory[..4].copy_from_slice(&parent_inode.to_le_bytes());
        set_inode_u32(&disk, parent_inode, 40, ROOT_DIRECTORY_BLOCK);
        undo.directory_block = ROOT_DIRECTORY_BLOCK;
        undo.directory = forged_directory;
        install_v1_undo(&disk, &undo);

        assert_corrupt_undo_is_read_only(disk);
    }

    #[test]
    fn undo_rejects_reserved_nonroot_parent_block() {
        let (disk, mut undo, parent_inode, _) = v1_create_undo_fixture(true, false);
        let reserved_nonroot_block = ROOT_DIRECTORY_BLOCK + 1;
        set_inode_u32(&disk, parent_inode, 40, reserved_nonroot_block);
        undo.directory_block = reserved_nonroot_block;
        install_v1_undo(&disk, &undo);

        assert_corrupt_undo_is_read_only(disk);
    }

    #[test]
    fn undo_rejects_noncanonical_root_layout_before_recovery() {
        let (disk, mut undo, _, _) = v1_create_undo_fixture(true, false);
        undo.kind = super::UNDO_RENAME;
        undo.inode = 0;
        undo.data_block = 0;
        set_inode_u32(&disk, super::ROOT_INODE, 28, 4);
        install_v1_undo(&disk, &undo);

        assert_corrupt_undo_is_read_only(disk);
    }

    #[test]
    fn undo_rejects_parent_block_owned_by_another_inode() {
        let (disk, undo, _, owner_inode) = v1_create_undo_fixture(true, true);
        set_inode_u32(
            &disk,
            owner_inode.expect("fixture owner file"),
            40,
            undo.directory_block,
        );
        install_v1_undo(&disk, &undo);

        assert_corrupt_undo_is_read_only(disk);
    }

    #[test]
    fn legacy_v1_nested_create_undo_remains_recoverable() {
        let (disk, undo, parent_inode, _) = v1_create_undo_fixture(true, false);
        install_v1_undo(&disk, &undo);
        assert_ne!(parent_inode, super::ROOT_INODE);
        assert_eq!(
            Vfs::<FaultDisk>::check_existing(&mut disk.clone()),
            Err(StorageError::RecoveryRequired)
        );

        let mut recovered = Vfs::recover_existing(disk.clone()).expect("legacy nested recovery");
        assert_eq!(
            recovered.open_path(b"/nested/not-created"),
            Err(StorageError::NotFound)
        );
        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).expect("recovered volume integrity");
    }

    fn legacy_empty_reused_block_fixture(cached: bool) -> (FaultDisk, u32) {
        let disk = FaultDisk::new(cached);
        let (mut volume, _) = Vfs::mount_or_format(disk.clone()).unwrap();
        let neighbor = volume.create_path(b"/neighbor").unwrap();
        volume.write(neighbor, b"neighbor payload").unwrap();

        let old = volume.create_path(b"/old-payload").unwrap();
        volume.write(old, &[0xa5; BLOCK_SIZE]).unwrap();
        let old_block = volume.read_inode(old.inode()).unwrap().direct_block;
        volume.remove_path(b"/old-payload").unwrap();

        // This empty inode has the on-disk form written by the old create
        // path: size and i_blocks are zero, but it owns a direct data block.
        let empty = volume.create_path(b"/legacy-empty").unwrap();
        let inode = volume.read_inode(empty.inode()).unwrap();
        assert_eq!(inode.size, 0);
        assert_eq!(inode.blocks, 0);
        assert_eq!(inode.direct_block, old_block);
        let neighbor_inode = volume.read_inode(neighbor.inode()).unwrap();
        assert_ne!(neighbor_inode.direct_block, old_block);

        // Model an older image where the reused empty-file block still
        // contains the deleted file's bytes. Keep both live and durable state
        // aligned so the fixture remains valid across a simulated restart.
        let offset = old_block as usize * BLOCK_SIZE;
        {
            let mut state = disk.0.lock().unwrap();
            state.live[offset..offset + BLOCK_SIZE].fill(0xa5);
            state.durable[offset..offset + BLOCK_SIZE].fill(0xa5);
        }
        drop(volume);
        Vfs::<FaultDisk>::check_existing(&mut disk.clone())
            .expect("legacy-compatible empty inode remains valid");
        (disk, old_block)
    }

    #[test]
    fn initialized_create_preserves_payload_or_recovers_absent_at_every_fault() {
        for cached in [false, true] {
            let baseline = failure_baseline(cached, false);
            let payload = b"NAGITR01 initial journal payload";
            let path = b"/initialized";
            let probe = baseline.fork();
            let mut volume = Vfs::mount_existing(probe.clone()).unwrap();
            probe.inject(None, None, false);
            let handle = volume
                .create_path_with_initial_contents(path, payload)
                .unwrap();
            let (writes, flushes) = {
                let state = probe.0.lock().unwrap();
                (state.writes, state.flushes)
            };
            assert!(writes > 0 && flushes > 0);
            assert_eq!(volume.metadata(handle).unwrap().size, payload.len() as u32);
            let mut bytes = [0; 64];
            assert_eq!(volume.read(handle, &mut bytes), Ok(payload.len()));
            assert_eq!(&bytes[..payload.len()], payload);
            assert_eq!(
                volume.create_path_with_initial_contents(path, b"replacement"),
                Err(StorageError::AlreadyExists)
            );
            probe.inject(None, None, false);
            assert_eq!(
                volume.create_path_with_initial_contents(b"/oversized", &[0; BLOCK_SIZE + 1]),
                Err(StorageError::FileTooLarge)
            );
            {
                let state = probe.0.lock().unwrap();
                assert_eq!((state.writes, state.flushes), (0, 0));
            }
            drop(volume);
            for flush_fault in [false, true] {
                let limit = if flush_fault { flushes } else { writes };
                for after in 0..limit {
                    let disk = baseline.fork();
                    let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
                    disk.inject(
                        (!flush_fault).then_some(after),
                        flush_fault.then_some(after),
                        false,
                    );
                    assert!(volume
                        .create_path_with_initial_contents(path, payload)
                        .is_err());
                    drop(volume);
                    disk.reconnect(true);
                    let mut volume = Vfs::recover_existing(disk.clone()).unwrap();
                    assert_eq!(volume.open_path(path), Err(StorageError::NotFound));
                    let kept = volume.open(b"kept.txt").unwrap();
                    let mut bytes = [0; 64];
                    let length = volume.read(kept, &mut bytes).unwrap();
                    assert_eq!(&bytes[..length], b"retained neighbor");
                    let handle = volume
                        .create_path_with_initial_contents(path, payload)
                        .unwrap();
                    assert_eq!(volume.read(handle, &mut bytes), Ok(payload.len()));
                    assert_eq!(&bytes[..payload.len()], payload);
                    drop(volume);
                    Vfs::<FaultDisk>::check_existing(&mut disk.clone()).unwrap();
                }
            }
        }
    }

    #[test]
    fn legacy_empty_reused_block_first_ranged_write_zeroes_gap() {
        for cached in [false, true] {
            let (disk, old_block) = legacy_empty_reused_block_fixture(cached);
            let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
            let empty = volume.open_path(b"/legacy-empty").unwrap();
            let inode = volume.read_inode(empty.inode()).unwrap();
            assert_eq!(inode.direct_block, old_block);
            assert_eq!(inode.blocks, 0);

            volume.write_at(empty, 59, b"range").unwrap();
            volume.flush().unwrap();

            let mut bytes = [0xff; 64];
            assert_eq!(volume.read(empty, &mut bytes), Ok(64));
            assert_eq!(
                &bytes[..59],
                &[0; 59],
                "cached={cached}: an old empty inode must not expose bytes from a reused block"
            );
            assert_eq!(&bytes[59..], b"range");

            let neighbor = volume.open_path(b"/neighbor").unwrap();
            let mut neighbor_bytes = [0; 32];
            let expected = b"neighbor payload";
            assert_eq!(
                volume.read(neighbor, &mut neighbor_bytes),
                Ok(expected.len())
            );
            assert_eq!(&neighbor_bytes[..expected.len()], expected);
            drop(volume);
            Vfs::<FaultDisk>::check_existing(&mut disk.clone())
                .expect("integrity after legacy empty-file write");
        }
    }

    #[test]
    fn legacy_empty_reused_block_truncate_extension_zeroes_tail_and_boundary() {
        for cached in [false, true] {
            let (disk, old_block) = legacy_empty_reused_block_fixture(cached);
            let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
            let empty = volume.open_path(b"/legacy-empty").unwrap();
            assert_eq!(
                volume.read_inode(empty.inode()).unwrap().direct_block,
                old_block
            );

            volume.write_at(empty, 59, b"range").unwrap();
            volume.flush().unwrap();
            assert_eq!(volume.truncate(empty, 128), Ok(()));
            volume.flush().unwrap();

            let mut same_block = [0xff; 128];
            assert_eq!(volume.read(empty, &mut same_block), Ok(128));
            assert_eq!(&same_block[..59], &[0; 59]);
            assert_eq!(&same_block[59..64], b"range");
            assert_eq!(
                &same_block[64..128],
                &[0; 64],
                "cached={cached}: truncate must not expose the old block tail"
            );

            let extended = BLOCK_SIZE + 64;
            volume.truncate(empty, extended).unwrap();
            volume.flush().unwrap();
            let mut bytes = [0xff; BLOCK_SIZE + 64];
            assert_eq!(volume.read(empty, &mut bytes), Ok(extended));
            assert_eq!(&bytes[..59], &[0; 59]);
            assert_eq!(&bytes[59..64], b"range");
            assert_eq!(&bytes[64..], &[0; BLOCK_SIZE + 64 - 64]);
            let neighbor = volume.open_path(b"/neighbor").unwrap();
            let mut neighbor_bytes = [0; 32];
            let expected = b"neighbor payload";
            assert_eq!(
                volume.read(neighbor, &mut neighbor_bytes),
                Ok(expected.len())
            );
            assert_eq!(&neighbor_bytes[..expected.len()], expected);
            drop(volume);
            Vfs::<FaultDisk>::check_existing(&mut disk.clone())
                .expect("integrity after legacy truncate extension");
        }
    }

    #[test]
    fn legacy_empty_reused_block_truncate_failures_recover_and_retry() {
        for cached in [false, true] {
            let (baseline, old_block) = legacy_empty_reused_block_fixture(cached);
            let mut setup = Vfs::mount_existing(baseline.clone()).unwrap();
            let empty = setup.open_path(b"/legacy-empty").unwrap();
            setup.write_at(empty, 59, b"range").unwrap();
            setup.flush().unwrap();
            drop(setup);

            let extended = BLOCK_SIZE + 64;
            let probe = baseline.fork();
            let mut probe_volume = Vfs::mount_existing(probe.clone()).unwrap();
            let probe_handle = probe_volume.open_path(b"/legacy-empty").unwrap();
            probe.inject(None, None, false);
            probe_volume
                .truncate(probe_handle, extended)
                .expect("probe boundary extension");
            let (first_sector_indices, second_sector_indices) = {
                let state = probe.0.lock().unwrap();
                let first_sector = u64::from(old_block) * 2;
                let first = state
                    .write_sectors
                    .iter()
                    .enumerate()
                    .filter_map(|(index, sector)| (*sector == first_sector).then_some(index))
                    .collect::<Vec<_>>();
                let second = state
                    .write_sectors
                    .iter()
                    .enumerate()
                    .filter_map(|(index, sector)| (*sector == first_sector + 1).then_some(index))
                    .collect::<Vec<_>>();
                (first, second)
            };
            assert_eq!(
                first_sector_indices.len(),
                1,
                "truncate clears the existing tail once before allocating"
            );
            assert_eq!(
                second_sector_indices.len(),
                1,
                "truncate clears both sectors of the existing block tail"
            );
            drop(probe_volume);

            for (sector_name, write_index) in [
                ("first sector", first_sector_indices[0]),
                ("second sector", second_sector_indices[0]),
            ] {
                let disk = baseline.fork();
                let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
                let empty = volume.open_path(b"/legacy-empty").unwrap();
                disk.inject(Some(write_index), None, false);
                assert_eq!(
                    volume.truncate(empty, extended),
                    Err(StorageError::Block),
                    "cached={cached} fault={sector_name}"
                );
                assert!(
                    disk.0.lock().unwrap().fail_write.is_none(),
                    "the selected {sector_name} fault must be reached"
                );
                drop(volume);

                // Gap clearing precedes allocation and inode-size publication.
                disk.reconnect(true);
                Vfs::<FaultDisk>::check_existing(&mut disk.clone())
                    .expect("integrity after interrupted truncate expansion");
                let recovery_counts = {
                    let state = disk.0.lock().unwrap();
                    (state.writes, state.flushes)
                };
                let mut recovered =
                    Vfs::recover_existing(disk.clone()).expect("explicit Recovery remount");
                assert_eq!(
                    {
                        let state = disk.0.lock().unwrap();
                        (state.writes, state.flushes)
                    },
                    recovery_counts,
                    "no undo marker exists; Recovery must not mutate this volume"
                );

                let empty = recovered.open_path(b"/legacy-empty").unwrap();
                assert_eq!(recovered.metadata(empty).unwrap().size, 64);
                let mut visible = [0xff; 64];
                assert_eq!(recovered.read(empty, &mut visible), Ok(64));
                assert_eq!(&visible[..59], &[0; 59]);
                assert_eq!(&visible[59..], b"range");
                let neighbor = recovered.open_path(b"/neighbor").unwrap();
                let mut neighbor_bytes = [0; 32];
                let expected = b"neighbor payload";
                assert_eq!(
                    recovered.read(neighbor, &mut neighbor_bytes),
                    Ok(expected.len())
                );
                assert_eq!(&neighbor_bytes[..expected.len()], expected);

                recovered
                    .truncate(empty, extended)
                    .expect("retry boundary extension after Recovery");
                recovered.flush().unwrap();
                let mut bytes = [0xff; BLOCK_SIZE + 64];
                assert_eq!(recovered.read(empty, &mut bytes), Ok(extended));
                assert_eq!(&bytes[..59], &[0; 59]);
                assert_eq!(&bytes[59..64], b"range");
                assert_eq!(&bytes[64..], &[0; BLOCK_SIZE + 64 - 64]);
                neighbor_bytes.fill(0);
                assert_eq!(
                    recovered.read(neighbor, &mut neighbor_bytes),
                    Ok(expected.len())
                );
                assert_eq!(&neighbor_bytes[..expected.len()], expected);
                drop(recovered);
                Vfs::<FaultDisk>::check_existing(&mut disk.clone())
                    .expect("integrity after Recovery and truncate retry");
            }
        }
    }

    #[test]
    fn legacy_empty_reused_block_gap_failures_recover_and_retry() {
        for cached in [false, true] {
            let (baseline, old_block) = legacy_empty_reused_block_fixture(cached);
            let probe = baseline.fork();
            let mut probe_volume = Vfs::mount_existing(probe.clone()).unwrap();
            let probe_handle = probe_volume.open_path(b"/legacy-empty").unwrap();
            probe.inject(None, None, false);
            probe_volume
                .write_at(probe_handle, 59, b"range")
                .expect("probe successful old-volume write");
            let (first_sector_indices, second_sector_indices) = {
                let state = probe.0.lock().unwrap();
                let first_sector = u64::from(old_block) * 2;
                let first = state
                    .write_sectors
                    .iter()
                    .enumerate()
                    .filter_map(|(index, sector)| (*sector == first_sector).then_some(index))
                    .collect::<Vec<_>>();
                let second = state
                    .write_sectors
                    .iter()
                    .enumerate()
                    .filter_map(|(index, sector)| (*sector == first_sector + 1).then_some(index))
                    .collect::<Vec<_>>();
                (first, second)
            };
            assert_eq!(
                first_sector_indices.len(),
                2,
                "the first sector is written once for gap zeroing and once for payload"
            );
            assert_eq!(
                second_sector_indices.len(),
                2,
                "the second sector is written once in each block update"
            );
            drop(probe_volume);

            for (phase, write_index) in [
                ("gap zero first sector", first_sector_indices[0]),
                ("gap zero second sector", second_sector_indices[0]),
                ("payload first sector", first_sector_indices[1]),
                ("payload second sector", second_sector_indices[1]),
            ] {
                let disk = baseline.fork();
                let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
                let empty = volume.open_path(b"/legacy-empty").unwrap();
                disk.inject(Some(write_index), None, false);
                assert_eq!(
                    volume.write_at(empty, 59, b"range"),
                    Err(StorageError::Block),
                    "cached={cached} phase={phase}"
                );
                assert!(
                    disk.0.lock().unwrap().fail_write.is_none(),
                    "the selected {phase} failure must be reached"
                );
                drop(volume);

                // This failure occurs outside an undo transaction. Recovery
                // must reopen the old empty inode without changing its size;
                // retry then zeros the gap before publishing the new length.
                disk.reconnect(true);
                Vfs::<FaultDisk>::check_existing(&mut disk.clone())
                    .expect("old empty volume remains structurally valid after failure");
                let recovery_counts = {
                    let state = disk.0.lock().unwrap();
                    (state.writes, state.flushes)
                };
                let mut recovered =
                    Vfs::recover_existing(disk.clone()).expect("explicit Recovery remount");
                assert_eq!(
                    {
                        let state = disk.0.lock().unwrap();
                        (state.writes, state.flushes)
                    },
                    recovery_counts,
                    "no pending undo exists; explicit Recovery must not mutate the volume"
                );

                let empty = recovered.open_path(b"/legacy-empty").unwrap();
                assert_eq!(recovered.metadata(empty).unwrap().size, 0);
                let mut bytes = [0xff; 64];
                assert_eq!(recovered.read(empty, &mut bytes), Ok(0));
                let neighbor = recovered.open_path(b"/neighbor").unwrap();
                let mut neighbor_bytes = [0; 32];
                let expected_neighbor = b"neighbor payload";
                assert_eq!(
                    recovered.read(neighbor, &mut neighbor_bytes),
                    Ok(expected_neighbor.len())
                );
                assert_eq!(
                    &neighbor_bytes[..expected_neighbor.len()],
                    expected_neighbor
                );

                recovered
                    .write_at(empty, 59, b"range")
                    .expect("retry after explicit Recovery");
                recovered.flush().unwrap();
                bytes.fill(0xff);
                assert_eq!(recovered.read(empty, &mut bytes), Ok(64));
                assert_eq!(&bytes[..59], &[0; 59]);
                assert_eq!(&bytes[59..], b"range");
                neighbor_bytes.fill(0);
                assert_eq!(
                    recovered.read(neighbor, &mut neighbor_bytes),
                    Ok(expected_neighbor.len())
                );
                assert_eq!(
                    &neighbor_bytes[..expected_neighbor.len()],
                    expected_neighbor
                );
                drop(recovered);
                Vfs::<FaultDisk>::check_existing(&mut disk.clone())
                    .expect("integrity after Recovery and retry");
            }
        }
    }

    fn m24_fixture(nested: bool) -> (FaultDisk, Vfs<FaultDisk>, std::string::String, u32) {
        let disk = FaultDisk::new(false);
        let (mut volume, _) = Vfs::mount_or_format(disk.clone()).unwrap();
        let prefix = if nested {
            volume.mkdir_path(b"/parent").unwrap();
            "/parent"
        } else {
            ""
        };
        let keep_path = std::format!("{prefix}/keep.bin");
        let keep = volume.create_path(keep_path.as_bytes()).unwrap();
        volume.write(keep, b"live neighbor remains").unwrap();
        volume.flush().unwrap();
        let parent_inode = if nested {
            volume.resolve_path(b"/parent").unwrap()
        } else {
            super::ROOT_INODE
        };
        let parent_block = volume.read_inode(parent_inode).unwrap().direct_block;
        (disk, volume, prefix.into(), parent_block)
    }

    #[test]
    fn m24_initial_ranged_write_accounting_survives_integrity() {
        let (disk, mut volume, _, _) = m24_fixture(false);
        let fresh = volume.create_path(b"/fresh.bin").unwrap();
        volume.write_at(fresh, 59, b"range").unwrap();
        volume.flush().unwrap();

        assert_eq!(volume.metadata(fresh).unwrap().blocks, 2);
        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).expect("integrity after ranged write");
    }

    #[test]
    fn m24_reused_block_gap_does_not_disclose_deleted_bytes() {
        let (disk, mut volume, prefix, _) = m24_fixture(false);
        let deleted_path = std::format!("{prefix}/deleted.bin");
        let deleted = volume.create_path(deleted_path.as_bytes()).unwrap();
        volume.write(deleted, &[0xa5; BLOCK_SIZE]).unwrap();
        let deleted_block = volume.read_inode(deleted.inode()).unwrap().direct_block;
        volume.remove_path(deleted_path.as_bytes()).unwrap();
        volume.flush().unwrap();

        let fresh_path = std::format!("{prefix}/fresh.bin");
        let fresh = volume.create_path(fresh_path.as_bytes()).unwrap();
        assert_eq!(
            volume.read_inode(fresh.inode()).unwrap().direct_block,
            deleted_block,
            "the regression must exercise reuse of the stale-data block"
        );
        volume.write_at(fresh, 59, b"range").unwrap();
        volume.flush().unwrap();

        let mut bytes = [0xff; 64];
        assert_eq!(volume.read(fresh, &mut bytes), Ok(64));
        assert_eq!(&bytes[..59], &[0; 59]);
        assert_eq!(&bytes[59..], b"range");
        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).expect("integrity after block reuse");
    }

    #[test]
    fn m24_nested_zero_write_sector_failures_rollback_and_retry() {
        let (baseline, mut volume, prefix, parent_block) = m24_fixture(true);
        let deleted_path = std::format!("{prefix}/deleted.bin");
        let deleted = volume.create_path(deleted_path.as_bytes()).unwrap();
        volume.write(deleted, &[0xa5; BLOCK_SIZE]).unwrap();
        let reused_block = volume.read_inode(deleted.inode()).unwrap().direct_block;
        volume.remove_path(deleted_path.as_bytes()).unwrap();
        volume.flush().unwrap();
        drop(volume);

        let fresh_path = std::format!("{prefix}/fresh.bin");
        let probe = baseline.fork();
        let mut probe_volume = Vfs::mount_existing(probe.clone()).unwrap();
        probe.inject(None, None, false);
        let created = probe_volume.create_path(fresh_path.as_bytes()).unwrap();
        assert_eq!(
            probe_volume
                .read_inode(created.inode())
                .unwrap()
                .direct_block,
            reused_block
        );
        let zero_write_offsets = {
            let state = probe.0.lock().unwrap();
            [0u64, 1].map(|half| {
                state
                    .write_sectors
                    .iter()
                    .position(|sector| *sector == u64::from(reused_block) * 2 + half)
                    .expect("create must write both zeroed data sectors")
            })
        };

        for offset in zero_write_offsets {
            let disk = baseline.fork();
            let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
            let mut parent_before = [0; BLOCK_SIZE];
            read_device_block(&mut disk.clone(), parent_block, &mut parent_before).unwrap();

            disk.inject(Some(offset), None, false);
            assert_eq!(
                volume.create_path(fresh_path.as_bytes()),
                Err(StorageError::Block),
                "a one-shot zero-sector failure must roll back the create"
            );
            assert!(
                disk.0.lock().unwrap().fail_write.is_none(),
                "the selected zero-sector fault must actually be reached"
            );
            assert_eq!(
                volume.open_path(fresh_path.as_bytes()),
                Err(StorageError::NotFound)
            );
            let mut parent_after = [0; BLOCK_SIZE];
            read_device_block(&mut disk.clone(), parent_block, &mut parent_after).unwrap();
            assert_eq!(parent_after, parent_before);
            Vfs::<FaultDisk>::check_existing(&mut disk.clone()).expect("rollback integrity");

            let retry = volume.create_path(fresh_path.as_bytes()).unwrap();
            volume.write_at(retry, 59, b"retry").unwrap();
            volume.flush().unwrap();
            let mut bytes = [0xff; 64];
            assert_eq!(volume.read(retry, &mut bytes), Ok(64));
            assert_eq!(&bytes[..59], &[0; 59]);
            assert_eq!(&bytes[59..], b"retry");
            Vfs::<FaultDisk>::check_existing(&mut disk.clone()).expect("retry integrity");
        }
    }

    #[test]
    fn m24_nested_parent_second_sector_failure_requires_explicit_recovery() {
        for nested in [false, true] {
            let (baseline, mut prep, prefix, parent_block) = m24_fixture(nested);
            for index in 0..11 {
                let filler = std::format!("{prefix}/{index:032}");
                prep.create_path(filler.as_bytes()).unwrap();
            }
            prep.flush().unwrap();
            drop(prep);

            let new_path = std::format!("{prefix}/{}", "t".repeat(32));
            let probe = baseline.fork();
            let mut probe_volume = Vfs::mount_existing(probe.clone()).unwrap();
            probe.inject(None, None, false);
            probe_volume.create_path(new_path.as_bytes()).unwrap();
            let second_sector_attempt = {
                let state = probe.0.lock().unwrap();
                let first_sector = state
                    .write_sectors
                    .iter()
                    .position(|sector| *sector == u64::from(parent_block) * 2)
                    .expect("boundary create must write the first parent sector");
                let second_sector = state
                    .write_sectors
                    .iter()
                    .position(|sector| *sector == u64::from(parent_block) * 2 + 1)
                    .expect("boundary create must write the second parent sector");
                assert!(first_sector < second_sector);
                second_sector
            };

            let disk = baseline.fork();
            let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
            disk.inject(Some(second_sector_attempt), None, true);
            assert_eq!(
                volume.create_path(new_path.as_bytes()),
                Err(StorageError::Indeterminate),
                "second-sector failure plus failed immediate rollback must retain intent"
            );
            assert!(disk.0.lock().unwrap().offline);
            disk.reconnect(false);

            let before_read_only_checks = {
                let state = disk.0.lock().unwrap();
                (
                    state.live.clone(),
                    state.durable.clone(),
                    state.writes,
                    state.flushes,
                )
            };
            assert_eq!(
                Vfs::mount_existing(disk.clone()).err(),
                Some(StorageError::RecoveryRequired)
            );
            assert_eq!(
                Vfs::mount_or_format(disk.clone()).err(),
                Some(StorageError::RecoveryRequired)
            );
            assert_eq!(
                Vfs::<FaultDisk>::check_existing(&mut disk.clone()),
                Err(StorageError::RecoveryRequired)
            );
            assert_eq!(
                volume.open_path(new_path.as_bytes()),
                Err(StorageError::RecoveryRequired)
            );
            {
                let state = disk.0.lock().unwrap();
                assert_eq!(
                    (
                        state.live.clone(),
                        state.durable.clone(),
                        state.writes,
                        state.flushes,
                    ),
                    before_read_only_checks,
                    "ordinary inspection must leave pending recovery untouched"
                );
            }

            let mut recovered = Vfs::recover_existing(disk.clone()).expect("explicit recovery");
            assert_eq!(
                recovered.open_path(new_path.as_bytes()),
                Err(StorageError::NotFound)
            );
            Vfs::<FaultDisk>::check_existing(&mut disk.clone()).expect("recovery integrity");
            let retry = recovered.create_path(new_path.as_bytes()).unwrap();
            recovered.write_at(retry, 59, b"retry").unwrap();
            let mut bytes = [0xff; 64];
            assert_eq!(recovered.read(retry, &mut bytes), Ok(64));
            assert_eq!(&bytes[..59], &[0; 59]);
            assert_eq!(&bytes[59..], b"retry");
            let keep_path = std::format!("{prefix}/keep.bin");
            let keep = recovered.open_path(keep_path.as_bytes()).unwrap();
            let mut neighbor = [0; 64];
            assert_eq!(recovered.read(keep, &mut neighbor), Ok(21));
            assert_eq!(&neighbor[..21], b"live neighbor remains");
            Vfs::<FaultDisk>::check_existing(&mut disk.clone()).expect("retry integrity");
        }
    }

    #[test]
    fn legacy_v1_rename_journal_fixture_remains_recoverable() {
        let disk = FaultDisk::new(false);
        let (mut volume, _) = Vfs::mount_or_format(disk.clone()).unwrap();
        let handle = volume.create(b"before").unwrap();
        volume.write(handle, b"legacy journal").unwrap();
        volume.flush().unwrap();

        let mut previous_directory = [0; BLOCK_SIZE];
        read_device_block(
            &mut disk.clone(),
            ROOT_DIRECTORY_BLOCK,
            &mut previous_directory,
        )
        .unwrap();
        volume.rename(b"before", b"after").unwrap();
        volume.flush().unwrap();

        let undo = Undo {
            kind: UNDO_RENAME,
            directory_inode: ROOT_INODE,
            directory_block: ROOT_DIRECTORY_BLOCK,
            inode: 0,
            data_block: 0,
            previous_inode: [0; EXT2_INODE_SIZE as usize],
            previous_indirect_checksum: [0; 32],
            directory: previous_directory,
        };
        let mut header = undo.header();
        header[..8].copy_from_slice(LEGACY_UNDO_MAGIC);
        let digest = Sha256::digest(&header[..480]);
        header[480..].copy_from_slice(&digest);
        {
            let mut state = disk.0.lock().unwrap();
            let backup_start = UNDO_BLOCK as usize * BLOCK_SIZE;
            state.live[backup_start..backup_start + BLOCK_SIZE].copy_from_slice(&undo.directory);
            state.durable[backup_start..backup_start + BLOCK_SIZE].copy_from_slice(&undo.directory);
            state.live[..SECTOR_SIZE].copy_from_slice(&header);
            state.durable[..SECTOR_SIZE].copy_from_slice(&header);
        }

        assert_eq!(
            Vfs::<FaultDisk>::check_existing(&mut disk.clone()),
            Err(StorageError::RecoveryRequired)
        );
        let mut recovered = Vfs::recover_existing(disk.clone()).expect("legacy explicit recovery");
        assert_eq!(recovered.open_path(b"/before"), Ok(handle));
        assert_eq!(recovered.open_path(b"/after"), Err(StorageError::NotFound));
        let metadata = recovered.metadata(handle).expect("legacy file metadata");
        assert_eq!(metadata.inode, handle.inode());
        assert_eq!(metadata.generation, handle.generation());
        let mut bytes = [0; 32];
        let count = recovered.read(handle, &mut bytes).unwrap();
        assert_eq!(&bytes[..count], b"legacy journal");
        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).unwrap();
    }

    #[test]
    fn pending_delete_recovery_is_idempotent_at_every_recovery_write() {
        let (baseline, expected_data) = delete_baseline(false, DeleteMode::PathFile);
        let pending = baseline.fork();
        let mut volume = Vfs::mount_existing(pending.clone()).unwrap();
        pending.inject(Some(4), None, true);
        assert_eq!(
            delete_operation(&mut volume, DeleteMode::PathFile),
            Err(StorageError::Indeterminate)
        );
        pending.reconnect(false);
        assert_eq!(
            Vfs::<FaultDisk>::check_existing(&mut pending.clone()),
            Err(StorageError::RecoveryRequired)
        );

        let probe = pending.fork();
        probe.inject(None, None, false);
        Vfs::recover_existing(probe.clone()).expect("delete recovery probe");
        let writes = probe.0.lock().unwrap().writes;
        for after in 0..writes {
            let disk = pending.fork();
            disk.inject(Some(after), None, true);
            assert!(
                Vfs::recover_existing(disk.clone()).is_err(),
                "recovery write {after} unexpectedly passed"
            );
            disk.reconnect(false);
            let context = std::format!("recovery write failure after {after}");
            assert_delete_recovered(disk, DeleteMode::PathFile, false, &expected_data, &context);
        }
    }

    #[test]
    fn create_and_crowded_rename_recover_every_write_and_flush_failure() {
        for cached in [false, true] {
            for rename in [false, true] {
                let baseline = failure_baseline(cached, rename);
                let probe = baseline.fork();
                let mut volume = Vfs::mount_existing(probe.clone()).unwrap();
                failure_operation(&mut volume, rename).unwrap();
                let (writes, flushes) = {
                    let state = probe.0.lock().unwrap();
                    (state.writes, state.flushes)
                };
                for persistent in [false, true] {
                    for flush_fault in [false, true] {
                        for after in 0..=if flush_fault { flushes } else { writes } {
                            let disk = baseline.fork();
                            let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
                            disk.inject(
                                (!flush_fault).then_some(after),
                                flush_fault.then_some(after),
                                persistent,
                            );
                            let result = failure_operation(&mut volume, rename);
                            if disk.0.lock().unwrap().offline {
                                assert!(volume.create(b"blocked").is_err());
                            }
                            disk.reconnect(cached && persistent);
                            // A write-through device may have made the operation
                            // durable before its final journal-clear flush failed.
                            // If persistent I/O loss also prevents rollback, the
                            // complete new state is an indeterminate commit outcome.
                            let commit_visible_after_error =
                                !cached && persistent && flush_fault && after + 1 == flushes;
                            let expected_new = result.is_ok() || commit_visible_after_error;
                            let context = std::format!("cached={cached} rename={rename} persistent={persistent} flush_fault={flush_fault} after={after}");
                            assert_recovered(disk, rename, expected_new, &context);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn undo_header_is_durable_before_each_power_cut_during_rollback() {
        for cached in [false, true] {
            for rename in [false, true] {
                let baseline = failure_baseline(cached, rename);
                let success_probe = baseline.fork();
                let mut success_volume = Vfs::mount_existing(success_probe.clone()).unwrap();
                failure_operation(&mut success_volume, rename).unwrap();
                let (success_events, success_flushes, success_write_count) = {
                    let state = success_probe.0.lock().unwrap();
                    (
                        state.durable_events,
                        state.flushes,
                        state.write_sectors.len(),
                    )
                };
                assert!(success_events > 0);
                assert!(success_flushes > 0);

                let rollback_probe = baseline.fork();
                let mut rollback_volume = Vfs::mount_existing(rollback_probe.clone()).unwrap();
                rollback_probe.inject(None, Some(success_flushes - 1), false);
                assert_eq!(
                    failure_operation(&mut rollback_volume, rename),
                    Err(StorageError::Block),
                    "transient final clear-flush failure should complete rollback"
                );
                {
                    let state = rollback_probe.0.lock().unwrap();
                    assert_eq!(
                        state.write_sectors.get(success_write_count),
                        Some(&0),
                        "rollback must re-publish sector-zero header before data writes: cached={cached} rename={rename}"
                    );
                }
                let total_events = rollback_probe.0.lock().unwrap().durable_events;
                assert_recovered(
                    rollback_probe.clone(),
                    rename,
                    false,
                    "successful rollback after transient final flush failure",
                );
                let before_rollback = if cached {
                    success_events - 1
                } else {
                    success_events
                };
                assert!(total_events > before_rollback);
                for offset in 0..(total_events - before_rollback) {
                    let disk = baseline.fork();
                    let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
                    disk.inject_with_cut(
                        None,
                        Some(success_flushes - 1),
                        false,
                        Some(before_rollback + offset),
                    );
                    assert_eq!(
                        failure_operation(&mut volume, rename),
                        Err(StorageError::Indeterminate),
                        "rollback interrupted after durable event {offset}: cached={cached} rename={rename}"
                    );
                    {
                        let state = disk.0.lock().unwrap();
                        assert!(state.offline);
                        assert_eq!(
                            state.write_sectors.get(success_write_count),
                            Some(&0),
                            "sector-zero header must precede every undo write: cached={cached} rename={rename} event={offset}"
                        );
                    }
                    disk.reconnect(true);
                    let context = std::format!(
                        "rollback cached={cached} rename={rename} after durable event {offset}"
                    );
                    assert_recovered(disk, rename, false, &context);
                }
            }
        }
    }

    #[test]
    fn pending_create_recovery_is_idempotent_at_every_persistent_recovery_write() {
        let baseline = failure_baseline(false, false);
        let pending = baseline.fork();
        let mut volume = Vfs::mount_existing(pending.clone()).unwrap();
        // Stop after mutation has begun while keeping a durable undo header.
        pending.inject(Some(6), None, true);
        assert!(volume.create(b"created").is_err());
        pending.reconnect(false);
        assert_eq!(
            Vfs::<FaultDisk>::check_existing(&mut pending.clone()),
            Err(StorageError::RecoveryRequired)
        );
        let probe = pending.fork();
        probe.inject(None, None, false);
        Vfs::recover_existing(probe.clone()).unwrap();
        let writes = probe.0.lock().unwrap().writes;
        for after in 0..writes {
            let disk = pending.fork();
            disk.inject(Some(after), None, true);
            assert!(Vfs::mount_existing(disk.clone()).is_err());
            disk.reconnect(false);
            assert_recovered(disk, false, false, "pending-create recovery write fault");
        }
    }

    #[test]
    fn unsupported_or_corrupt_undo_never_formats_or_writes() {
        let baseline = failure_baseline(false, false);
        for mode in 0..4 {
            let disk = baseline.fork();
            {
                let mut state = disk.0.lock().unwrap();
                match mode {
                    0 => state.live[0] = 0xff,
                    _ => {
                        drop(state);
                        let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
                        disk.inject(Some(6), None, true);
                        assert!(volume.create(b"created").is_err());
                        disk.reconnect(false);
                        state = disk.0.lock().unwrap();
                        match mode {
                            1 => state.live[480] ^= 1,
                            2 => state.live[super::UNDO_BLOCK as usize * BLOCK_SIZE] ^= 1,
                            _ => {
                                state.live[20..24].copy_from_slice(&1u32.to_le_bytes());
                                let digest = sha2::Sha256::digest(&state.live[..480]);
                                state.live[480..512].copy_from_slice(&digest);
                            }
                        }
                    }
                }
                let before = state.live.clone();
                let writes = state.writes;
                let flushes = state.flushes;
                drop(state);
                assert_eq!(
                    Vfs::mount_or_format(disk.clone()).err(),
                    Some(StorageError::Corrupt)
                );
                assert_eq!(
                    Vfs::<FaultDisk>::check_existing(&mut disk.clone()),
                    Err(StorageError::Corrupt)
                );
                let state = disk.0.lock().unwrap();
                assert_eq!(state.live, before);
                assert_eq!(state.writes, writes);
                assert_eq!(state.flushes, flushes);
            }
        }
    }

    #[test]
    fn read_only_pending_check_and_mounts_do_not_write_or_flush() {
        let disk = failure_baseline(false, false);
        let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
        disk.inject(Some(6), None, true);
        assert!(volume.create(b"created").is_err());
        disk.reconnect(false);
        let before = {
            let state = disk.0.lock().unwrap();
            (state.live.clone(), state.writes, state.flushes)
        };
        assert_eq!(
            Vfs::<FaultDisk>::check_existing(&mut disk.clone()),
            Err(StorageError::RecoveryRequired)
        );
        assert_eq!(
            Vfs::mount_existing(disk.clone()).err(),
            Some(StorageError::RecoveryRequired)
        );
        assert_eq!(
            Vfs::mount_or_format(disk.clone()).err(),
            Some(StorageError::RecoveryRequired)
        );
        assert_eq!(
            volume.open(b"kept.txt"),
            Err(StorageError::RecoveryRequired)
        );
        {
            let state = disk.0.lock().unwrap();
            assert_eq!((state.live.clone(), state.writes, state.flushes), before);
        }
        let mut recovered = Vfs::recover_existing(disk.clone()).expect("explicit recovery");
        assert!(recovered.open(b"kept.txt").is_ok());
        assert_eq!(recovered.open(b"created"), Err(StorageError::NotFound));
    }

    #[test]
    fn cached_header_clear_failure_rolls_back_before_next_transaction() {
        let disk = failure_baseline(true, false);
        let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
        disk.inject(None, Some(4), false); // Fail the first header-clear flush.
        assert!(volume.create(b"first").is_err());
        assert_eq!(volume.open(b"first"), Err(StorageError::NotFound));
        {
            let state = disk.0.lock().unwrap();
            assert!(state.live[..SECTOR_SIZE].iter().all(|byte| *byte == 0));
            assert!(state.durable[..SECTOR_SIZE].iter().all(|byte| *byte == 0));
        }
        disk.reconnect(true);
        let mut volume = Vfs::mount_existing(disk.clone()).expect("clean remount");
        let second = volume.create(b"second").expect("next transaction");
        assert_eq!(volume.open(b"first"), Err(StorageError::NotFound));
        assert_eq!(volume.open(b"second"), Ok(second));
        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).unwrap();
    }

    #[test]
    fn recovery_flush_failures_keep_idempotent_durable_recovery() {
        let disk = failure_baseline(true, false);
        let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
        disk.inject(Some(6), None, true);
        assert!(volume.create(b"created").is_err());
        disk.reconnect(true);
        let probe = disk.fork();
        probe.inject(None, None, false);
        Vfs::recover_existing(probe.clone()).unwrap();
        let flushes = probe.0.lock().unwrap().flushes;
        for after in 0..flushes {
            let attempt = disk.fork();
            attempt.inject(None, Some(after), true);
            assert!(Vfs::mount_existing(attempt.clone()).is_err());
            attempt.reconnect(true);
            assert_recovered(attempt, false, false, "pending recovery flush fault");
        }
    }

    #[test]
    fn invalid_superblock_with_unknown_or_pending_header_is_never_formatted() {
        for pending in [false, true] {
            let disk = failure_baseline(false, false);
            if pending {
                let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
                disk.inject(Some(6), None, true);
                assert!(volume.create(b"created").is_err());
                disk.reconnect(false);
            }
            {
                let mut state = disk.0.lock().unwrap();
                if !pending {
                    state.live[0] = 0xff;
                }
                state.live[BLOCK_SIZE + 56] = 0;
            }
            let (before, writes, flushes) = {
                let state = disk.0.lock().unwrap();
                (state.live.clone(), state.writes, state.flushes)
            };
            assert_eq!(
                Vfs::mount_or_format(disk.clone()).err(),
                Some(StorageError::Corrupt)
            );
            let state = disk.0.lock().unwrap();
            assert_eq!(state.live, before);
            assert_eq!(state.writes, writes);
            assert_eq!(state.flushes, flushes);
        }
    }

    #[test]
    fn checksummed_undo_cannot_free_an_existing_files_data() {
        let disk = failure_baseline(false, false);
        let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
        let kept_inode = volume.open(b"kept.txt").unwrap().inode();
        let kept_data = volume.read_inode(kept_inode).unwrap().direct_block;
        disk.inject(Some(6), None, true);
        assert!(volume.create(b"created").is_err());
        disk.reconnect(false);
        {
            let mut state = disk.0.lock().unwrap();
            state.live[24..28].copy_from_slice(&kept_data.to_le_bytes());
            let digest = sha2::Sha256::digest(&state.live[..480]);
            state.live[480..512].copy_from_slice(&digest);
        }
        let (before, writes, flushes) = {
            let state = disk.0.lock().unwrap();
            (state.live.clone(), state.writes, state.flushes)
        };
        assert_eq!(
            Vfs::mount_existing(disk.clone()).err(),
            Some(StorageError::Corrupt)
        );
        let state = disk.0.lock().unwrap();
        assert_eq!(state.live, before);
        assert_eq!(state.writes, writes);
        assert_eq!(state.flushes, flushes);
    }

    #[test]
    fn reset_access_does_not_clear_another_active_instance_id() {
        let _gate = VolumeAccess::acquire();
        let stale_id = allocate_access_id().unwrap();
        let active_id = allocate_access_id().unwrap();
        ACTIVE_ACCESS_ID.store(active_id, Ordering::Release);
        let _active_reset = ResetAccess(active_id);

        drop(ResetAccess(stale_id));

        assert_eq!(ACTIVE_ACCESS_ID.load(Ordering::Acquire), active_id);
    }

    #[test]
    fn separately_mounted_instances_serialize_and_unwind_releases_access() {
        use std::sync::mpsc;
        let disk = failure_baseline(false, false);
        let mut first = Vfs::mount_existing(disk.clone()).unwrap();
        let mut second = Vfs::mount_existing(disk.clone()).unwrap();
        let (held_send, held_receive) = mpsc::channel();
        let (release_send, release_receive) = mpsc::channel();
        let (started_send, started_receive) = mpsc::channel();
        let (done_send, done_receive) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            first
                .with_access(|volume| {
                    held_send.send(()).unwrap();
                    release_receive.recv().unwrap();
                    volume.create(b"first")
                })
                .unwrap()
        });
        held_receive.recv().unwrap();
        let waiter = std::thread::spawn(move || {
            started_send.send(()).unwrap();
            let result = second.create(b"second");
            done_send.send(result).unwrap();
        });
        started_receive.recv().unwrap();
        assert!(done_receive
            .recv_timeout(std::time::Duration::from_millis(20))
            .is_err());
        release_send.send(()).unwrap();
        let first = worker.join().unwrap();
        let second = done_receive
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("second Vfs instance did not acquire the released access gate")
            .unwrap();
        waiter.join().unwrap();
        assert_ne!(first.inode(), second.inode());

        let mut volume = Vfs::mount_existing(disk.clone()).unwrap();
        let volume_access_id = volume.access_id;
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _: Result<(), StorageError> =
                volume.with_access(|_| panic!("injected operation unwind"));
        }));
        assert!(panic.is_err());
        assert_ne!(
            ACTIVE_ACCESS_ID.load(Ordering::Acquire),
            volume_access_id,
            "unwind left this Vfs instance's access ID active"
        );

        let (recovered_send, recovered_receive) = mpsc::channel();
        let recovered = std::thread::spawn(move || {
            recovered_send.send(volume.create(b"after-unwind")).unwrap();
        });
        recovered_receive
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("another Vfs operation remained blocked after unwind")
            .unwrap();
        recovered.join().unwrap();
        Vfs::<FaultDisk>::check_existing(&mut disk.clone()).unwrap();
    }
}
