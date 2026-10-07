use core::ptr;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageError {
    Block,
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
}

impl<D: BlockDevice> Vfs<D> {
    /// Mount an existing Nagi VFS without formatting or initializing it.
    /// This only validates the fixed superblock geometry; callers that need
    /// an integrity guarantee must run `check_existing` first.
    pub fn mount_existing(device: D) -> Result<Self, StorageError> {
        let mut volume = Self { device };
        let mut superblock = [0; BLOCK_SIZE];
        volume.read_block(SUPERBLOCK_BLOCK, &mut superblock)?;
        if read_u16(&superblock, 56) != EXT2_MAGIC {
            return Err(StorageError::Corrupt);
        }
        validate_superblock(&superblock)?;
        Ok(volume)
    }

    pub fn mount_or_format(device: D) -> Result<(Self, bool), StorageError> {
        let mut volume = Self { device };
        let mut superblock = [0; BLOCK_SIZE];
        volume.read_block(SUPERBLOCK_BLOCK, &mut superblock)?;
        if read_u16(&superblock, 56) != EXT2_MAGIC {
            volume.format()?;
            return Ok((volume, true));
        }
        validate_superblock(&superblock)?;
        Ok((volume, false))
    }

    pub fn into_device(self) -> D {
        self.device
    }

    /// Resolve a bounded path below this volume's process root and ensure its
    /// final component names a directory. This is used when mounting the
    /// conventional guest `/tmp` directory; an existing non-directory is an
    /// error rather than being treated as a successful setup.
    pub fn ensure_directory_path(&mut self, path: &[u8]) -> Result<(), StorageError> {
        match self.resolve_path(path) {
            Ok(inode) => {
                self.directory_inode(inode)?;
                Ok(())
            }
            Err(StorageError::NotFound) => self.mkdir_path(path),
            Err(error) => Err(error),
        }
    }

    pub fn create_path(&mut self, path: &[u8]) -> Result<FileHandle, StorageError> {
        let (parent, name) = self.resolve_parent_path(path, false)?;
        self.create_in_directory(parent, name)
    }

    pub fn mkdir_path(&mut self, path: &[u8]) -> Result<(), StorageError> {
        let (parent, name) = self.resolve_parent_path(path, true)?;
        self.mkdir_in_directory(parent, name)
    }

    pub fn open_path(&mut self, path: &[u8]) -> Result<FileHandle, StorageError> {
        let inode_number = self.resolve_path(path)?;
        let inode = self.read_inode(inode_number)?;
        if inode.mode & 0xf000 == 0x4000 {
            return Err(StorageError::IsDirectory);
        }
        if inode.mode & 0xf000 != 0x8000 {
            return Err(StorageError::InvalidHandle);
        }
        self.handle_for(inode_number)
    }

    pub fn metadata_path(&mut self, path: &[u8]) -> Result<FileMetadata, StorageError> {
        let inode_number = self.resolve_path(path)?;
        let inode = self.read_inode(inode_number)?;
        if inode.mode & 0xf000 != 0x8000 && inode.mode & 0xf000 != 0x4000 {
            return Err(StorageError::InvalidHandle);
        }
        Ok(metadata_from_inode(inode_number, inode))
    }

    /// Remove a regular file by path. Directory removal has a separate POSIX
    /// operation so `unlink` cannot silently remove a directory.
    pub fn remove_path(&mut self, path: &[u8]) -> Result<(), StorageError> {
        self.remove_path_with_kind(path, Some(false), false)
    }

    pub fn rmdir_path(&mut self, path: &[u8]) -> Result<(), StorageError> {
        self.remove_path_with_kind(path, Some(true), true)
    }

    pub fn list_directory_path(
        &mut self,
        path: &[u8],
        entries: &mut [DirectoryEntry],
    ) -> Result<usize, StorageError> {
        let inode = self.resolve_path(path)?;
        self.list_directory_inode(inode, entries)
    }

    pub fn create(&mut self, name: &[u8]) -> Result<FileHandle, StorageError> {
        validate_name(name)?;
        self.create_in_directory(ROOT_INODE, name)
    }

    fn create_in_directory(
        &mut self,
        parent_inode: u32,
        name: &[u8],
    ) -> Result<FileHandle, StorageError> {
        validate_name(name)?;
        self.directory_inode(parent_inode)?;
        if self.find_inode_in_directory(parent_inode, name)?.is_some() {
            return Err(StorageError::AlreadyExists);
        }
        let inode = self.allocate_bit(INODE_BITMAP, FIRST_FILE_INODE - 1, EXT2_INODE_COUNT)?;
        let generation = match self.next_generation(inode) {
            Ok(generation) => generation,
            Err(error) => {
                self.clear_bit(INODE_BITMAP, inode - 1)?;
                return Err(error);
            }
        };
        let data_block = match self.allocate_bit(BLOCK_BITMAP, FIRST_FILE_BLOCK, EXT2_BLOCK_COUNT) {
            Ok(block) => block,
            Err(error) => {
                self.clear_bit(INODE_BITMAP, inode - 1)?;
                return Err(error);
            }
        };
        let now = current_timestamp();
        self.write_inode(
            inode,
            InodeInfo {
                mode: 0x8000,
                uid: 0,
                gid: 0,
                size: 0,
                atime: now,
                ctime: now,
                mtime: now,
                blocks: 0,
                direct_block: data_block,
                extra_blocks: [0; DIRECT_BLOCKS - 1],
                indirect_block: 0,
                generation,
            },
        )?;
        if let Err(error) = self.add_directory_entry_in_directory(parent_inode, name, inode, 1) {
            self.clear_bit(INODE_BITMAP, inode - 1)?;
            self.clear_bit(BLOCK_BITMAP, data_block)?;
            return Err(error);
        }
        self.adjust_free_counts(-1, -1)?;
        Ok(FileHandle { inode, generation })
    }

    pub fn mkdir(&mut self, name: &[u8]) -> Result<(), StorageError> {
        validate_name(name)?;
        self.mkdir_in_directory(ROOT_INODE, name)
    }

    fn mkdir_in_directory(&mut self, parent_inode: u32, name: &[u8]) -> Result<(), StorageError> {
        validate_name(name)?;
        self.directory_inode(parent_inode)?;
        if self.find_inode_in_directory(parent_inode, name)?.is_some() {
            return Err(StorageError::AlreadyExists);
        }
        let inode = self.allocate_bit(INODE_BITMAP, FIRST_FILE_INODE - 1, EXT2_INODE_COUNT)?;
        let data_block = match self.allocate_bit(BLOCK_BITMAP, FIRST_FILE_BLOCK, EXT2_BLOCK_COUNT) {
            Ok(block) => block,
            Err(error) => {
                self.clear_bit(INODE_BITMAP, inode - 1)?;
                return Err(error);
            }
        };
        let generation = match self.next_generation(inode) {
            Ok(generation) => generation,
            Err(error) => {
                self.clear_bit(INODE_BITMAP, inode - 1)?;
                self.clear_bit(BLOCK_BITMAP, data_block)?;
                return Err(error);
            }
        };
        let now = current_timestamp();
        self.write_inode(
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
        if let Err(error) = self.write_block(data_block, &directory) {
            self.clear_bit(INODE_BITMAP, inode - 1)?;
            self.clear_bit(BLOCK_BITMAP, data_block)?;
            return Err(error);
        }
        if let Err(error) = self.add_directory_entry_in_directory(parent_inode, name, inode, 2) {
            self.clear_bit(INODE_BITMAP, inode - 1)?;
            self.clear_bit(BLOCK_BITMAP, data_block)?;
            return Err(error);
        }
        self.adjust_free_counts(-1, -1)
    }

    pub fn remove(&mut self, name: &[u8]) -> Result<(), StorageError> {
        validate_name(name)?;
        let (directory_offset, inode_number) = self
            .find_directory_entry(name)?
            .ok_or(StorageError::NotFound)?;
        let inode = self.read_inode(inode_number)?;
        if inode.mode & 0xf000 != 0x8000 && inode.mode & 0xf000 != 0x4000 {
            return Err(StorageError::InvalidHandle);
        }
        if inode.mode & 0xf000 == 0x4000 && !self.directory_is_empty(inode.direct_block)? {
            return Err(StorageError::DirectoryFull);
        }
        let mut directory = [0; BLOCK_SIZE];
        self.read_block(ROOT_DIRECTORY_BLOCK, &mut directory)?;
        write_u32(&mut directory, directory_offset, 0);
        self.write_block(ROOT_DIRECTORY_BLOCK, &directory)?;
        self.clear_bit(INODE_BITMAP, inode_number - 1)?;
        let released = self.release_inode_blocks(&inode)?;
        self.clear_inode(inode_number)?;
        self.adjust_free_counts(released as i32, 1)
    }

    pub fn open(&mut self, name: &[u8]) -> Result<FileHandle, StorageError> {
        validate_name(name)?;
        let inode = self.find_inode(name)?.ok_or(StorageError::NotFound)?;
        self.handle_for(inode)
    }

    pub fn rename(&mut self, old_name: &[u8], new_name: &[u8]) -> Result<FileHandle, StorageError> {
        self.rename_child_inode(ROOT_INODE, old_name, new_name)
    }

    /// Rename an entry in the directory at `directory_path` without changing
    /// its inode or generation. Both names must be direct children of that
    /// directory; moving entries between directories is a separate operation.
    pub fn rename_child(
        &mut self,
        directory_path: &[u8],
        old_name: &[u8],
        new_name: &[u8],
    ) -> Result<FileHandle, StorageError> {
        let directory_inode = self.resolve_path(directory_path)?;
        self.rename_child_inode(directory_inode, old_name, new_name)
    }

    fn rename_child_inode(
        &mut self,
        directory_inode: u32,
        old_name: &[u8],
        new_name: &[u8],
    ) -> Result<FileHandle, StorageError> {
        validate_name(old_name)?;
        validate_name(new_name)?;
        if old_name == new_name {
            let inode = self
                .find_inode_in_directory(directory_inode, old_name)?
                .ok_or(StorageError::NotFound)?;
            return self.handle_for(inode);
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
        self.write_block(directory.direct_block, &compacted)?;
        let inode = self
            .find_inode_in_directory(directory_inode, new_name)?
            .ok_or(StorageError::Corrupt)?;
        self.handle_for(inode)
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
        validate_name(source_name)?;
        validate_name(destination_name)?;
        if source_name == destination_name {
            return self.open(source_name);
        }
        let (source_offset, source_inode_number) = self
            .find_directory_entry(source_name)?
            .ok_or(StorageError::NotFound)?;
        let (destination_offset, destination_inode_number) = self
            .find_directory_entry(destination_name)?
            .ok_or(StorageError::NotFound)?;
        let source_inode = self.read_inode(source_inode_number)?;
        let destination_inode = self.read_inode(destination_inode_number)?;
        if source_inode.mode & 0xf000 != 0x8000 || destination_inode.mode & 0xf000 != 0x8000 {
            return Err(StorageError::InvalidHandle);
        }

        let mut directory = [0; BLOCK_SIZE];
        self.read_block(ROOT_DIRECTORY_BLOCK, &mut directory)?;
        write_u32(&mut directory, destination_offset, source_inode_number);
        write_u32(&mut directory, source_offset, 0);
        self.write_block(ROOT_DIRECTORY_BLOCK, &directory)?;
        self.clear_bit(INODE_BITMAP, destination_inode_number - 1)?;
        let released = self.release_inode_blocks(&destination_inode)?;
        self.clear_inode(destination_inode_number)?;
        self.adjust_free_counts(released as i32, 1)?;
        self.handle_for(source_inode_number)
    }

    pub fn list_root(&mut self, entries: &mut [DirectoryEntry]) -> Result<usize, StorageError> {
        self.list_directory_inode(ROOT_INODE, entries)
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
        if data.len() > MAX_FILE_SIZE {
            return Err(StorageError::FileTooLarge);
        }
        let mut inode = self.validate_handle(handle)?;
        let current = data_blocks_for_size(inode.size as usize);
        let needed = data_blocks_for_size(data.len());
        self.resize_file_blocks(&mut inode, current, needed)?;
        self.store_file_range(&inode, 0, data)?;
        self.clear_past_end(&inode, data.len())?;
        inode.size = data.len() as u32;
        let now = current_timestamp();
        inode.ctime = now;
        inode.mtime = now;
        self.write_inode(handle.inode, inode)
    }

    /// Write `data` at `offset`, growing the file as needed. A gap between
    /// the old end and `offset` reads as zeros.
    pub fn write_at(
        &mut self,
        handle: FileHandle,
        offset: usize,
        data: &[u8],
    ) -> Result<(), StorageError> {
        let end = offset
            .checked_add(data.len())
            .filter(|end| *end <= MAX_FILE_SIZE)
            .ok_or(StorageError::FileTooLarge)?;
        let mut inode = self.validate_handle(handle)?;
        let size = inode.size as usize;
        let new_size = size.max(end);
        let current = data_blocks_for_size(size);
        let needed = data_blocks_for_size(new_size);
        if needed > current {
            self.resize_file_blocks(&mut inode, current, needed)?;
        }
        self.store_file_range(&inode, offset, data)?;
        inode.size = new_size as u32;
        let now = current_timestamp();
        inode.ctime = now;
        inode.mtime = now;
        self.write_inode(handle.inode, inode)
    }

    pub fn truncate(&mut self, handle: FileHandle, length: usize) -> Result<(), StorageError> {
        if length > MAX_FILE_SIZE {
            return Err(StorageError::FileTooLarge);
        }
        let mut inode = self.validate_handle(handle)?;
        let size = inode.size as usize;
        if length < size {
            self.clear_past_end(&inode, length)?;
        }
        let current = data_blocks_for_size(size);
        let needed = data_blocks_for_size(length);
        self.resize_file_blocks(&mut inode, current, needed)?;
        inode.size = length as u32;
        let now = current_timestamp();
        inode.ctime = now;
        inode.mtime = now;
        self.write_inode(handle.inode, inode)
    }

    pub fn metadata(&mut self, handle: FileHandle) -> Result<FileMetadata, StorageError> {
        let inode = self.validate_handle(handle)?;
        Ok(metadata_from_inode(handle.inode, inode))
    }

    pub fn set_mode(&mut self, handle: FileHandle, mode: u16) -> Result<(), StorageError> {
        let mut inode = self.validate_handle(handle)?;
        inode.mode = (inode.mode & 0xf000) | (mode & 0x0fff);
        inode.ctime = current_timestamp();
        self.write_inode(handle.inode, inode)
    }

    pub fn set_owner(
        &mut self,
        handle: FileHandle,
        uid: Option<u16>,
        gid: Option<u16>,
    ) -> Result<(), StorageError> {
        let mut inode = self.validate_handle(handle)?;
        if let Some(uid) = uid {
            inode.uid = uid;
        }
        if let Some(gid) = gid {
            inode.gid = gid;
        }
        inode.ctime = current_timestamp();
        self.write_inode(handle.inode, inode)
    }

    pub fn set_times(
        &mut self,
        handle: FileHandle,
        atime: u32,
        mtime: u32,
    ) -> Result<(), StorageError> {
        let mut inode = self.validate_handle(handle)?;
        inode.atime = atime;
        inode.mtime = mtime;
        inode.ctime = current_timestamp();
        self.write_inode(handle.inode, inode)
    }

    pub fn flush(&mut self) -> Result<(), StorageError> {
        self.device.flush()
    }

    pub fn read(
        &mut self,
        handle: FileHandle,
        destination: &mut [u8],
    ) -> Result<usize, StorageError> {
        let mut inode = self.validate_handle(handle)?;
        let length = usize::try_from(inode.size).map_err(|_| StorageError::Corrupt)?;
        if length > MAX_FILE_SIZE {
            return Err(StorageError::Corrupt);
        }
        if destination.len() < length {
            return Err(StorageError::BufferTooSmall);
        }
        self.copy_file_range(&inode, 0, &mut destination[..length])?;
        inode.atime = current_timestamp();
        self.write_inode(handle.inode, inode)?;
        Ok(length)
    }

    /// Read up to `destination.len()` bytes starting at `offset`. Returns
    /// the count read, 0 at or past the end of the file.
    pub fn read_at(
        &mut self,
        handle: FileHandle,
        offset: usize,
        destination: &mut [u8],
    ) -> Result<usize, StorageError> {
        let inode = self.validate_handle(handle)?;
        self.copy_file_range(&inode, offset, destination)
    }

    pub fn mmap(&mut self, handle: FileHandle) -> Result<FileMapping, StorageError> {
        let mut mapping = FileMapping {
            handle,
            bytes: [0; MAX_SMALL_FILE_SIZE],
            length: 0,
        };
        let length = self.read(handle, &mut mapping.bytes)?;
        mapping.length = length;
        Ok(mapping)
    }

    pub fn flush_mapping(&mut self, mapping: &FileMapping) -> Result<(), StorageError> {
        self.write(mapping.handle, mapping.bytes())
    }

    fn format(&mut self) -> Result<(), StorageError> {
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
                return self.write_block(directory_inode.direct_block, &directory);
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
        self.write_block(parent_inode.direct_block, &directory)?;
        self.clear_bit(INODE_BITMAP, inode_number - 1)?;
        let released = self.release_inode_blocks(&inode)?;
        self.clear_inode(inode_number)?;
        self.adjust_free_counts(released as i32, 1)
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
        check_vfs_integrity(device)
    }
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

    use std::vec;
    use std::vec::Vec;

    use super::{
        BlockDevice, DirectoryEntry, FileHandle, FileMapping, StorageError, Vfs, BLOCK_SIZE,
        DIRECT_BLOCKS, EXT2_BLOCK_COUNT, MAX_FILE_SIZE, ROOT_DIRECTORY_BLOCK, SECTOR_SIZE,
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
            .create_path(b"/files/created-after-rename")
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
}
