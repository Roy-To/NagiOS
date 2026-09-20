use core::ptr;

pub const SECTOR_SIZE: usize = 512;
pub const BLOCK_SIZE: usize = 1024;
pub const MAX_FILE_SIZE: usize = BLOCK_SIZE;
pub const MAX_NAME_LENGTH: usize = 32;
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
const GROUP_DESCRIPTOR_BLOCK: u32 = 2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageError {
    Block,
    Corrupt,
    NameTooLong,
    InvalidName,
    NotFound,
    AlreadyExists,
    DirectoryFull,
    InvalidHandle,
    FileTooLarge,
    BufferTooSmall,
    Capacity,
}

pub trait BlockDevice {
    fn read_sector(
        &mut self,
        sector: u64,
        destination: &mut [u8; SECTOR_SIZE],
    ) -> Result<(), StorageError>;

    fn write_sector(&mut self, sector: u64, source: &[u8; SECTOR_SIZE])
        -> Result<(), StorageError>;
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

impl BlockDevice for SyscallBlockDevice {
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileHandle {
    inode: u32,
    generation: u16,
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
    size: u32,
    blocks: u32,
    direct_block: u32,
}

#[derive(Clone, Copy)]
pub struct FileMapping {
    handle: FileHandle,
    bytes: [u8; MAX_FILE_SIZE],
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
        if length > MAX_FILE_SIZE {
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

pub struct Vfs<D: BlockDevice> {
    device: D,
}

impl<D: BlockDevice> Vfs<D> {
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

    pub fn create(&mut self, name: &[u8]) -> Result<FileHandle, StorageError> {
        validate_name(name)?;
        if self.find_inode(name)?.is_some() {
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
        self.write_inode(
            inode,
            InodeInfo {
                mode: 0x8000,
                size: 0,
                blocks: 0,
                direct_block: data_block,
            },
        )?;
        if let Err(error) = self.add_directory_entry(name, inode, 1) {
            self.clear_bit(INODE_BITMAP, inode - 1)?;
            self.clear_bit(BLOCK_BITMAP, data_block)?;
            return Err(error);
        }
        self.adjust_free_counts(-1, -1)?;
        Ok(FileHandle {
            inode,
            generation: 1,
        })
    }

    pub fn mkdir(&mut self, name: &[u8]) -> Result<(), StorageError> {
        validate_name(name)?;
        if self.find_inode(name)?.is_some() {
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
        self.write_inode(
            inode,
            InodeInfo {
                mode: 0x4000,
                size: BLOCK_SIZE as u32,
                blocks: 2,
                direct_block: data_block,
            },
        )?;
        let mut directory = [0; BLOCK_SIZE];
        let dot = [b'.'];
        let dot_dot = [b'.', b'.'];
        write_directory_record(&mut directory, 0, inode, 12, 1, 2, &dot);
        write_directory_record(&mut directory, 12, ROOT_INODE, 12, 2, 2, &dot_dot);
        write_directory_record(&mut directory, 24, 0, BLOCK_SIZE as u16 - 24, 0, 0, b"");
        if let Err(error) = self.write_block(data_block, &directory) {
            self.clear_bit(INODE_BITMAP, inode - 1)?;
            self.clear_bit(BLOCK_BITMAP, data_block)?;
            return Err(error);
        }
        if let Err(error) = self.add_directory_entry(name, inode, 2) {
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
        self.clear_bit(BLOCK_BITMAP, inode.direct_block)?;
        self.clear_inode(inode_number)?;
        self.adjust_free_counts(1, 1)
    }

    pub fn open(&mut self, name: &[u8]) -> Result<FileHandle, StorageError> {
        validate_name(name)?;
        let inode = self.find_inode(name)?.ok_or(StorageError::NotFound)?;
        self.validate_handle(FileHandle {
            inode,
            generation: 1,
        })?;
        Ok(FileHandle {
            inode,
            generation: 1,
        })
    }

    pub fn rename(&mut self, old_name: &[u8], new_name: &[u8]) -> Result<FileHandle, StorageError> {
        validate_name(old_name)?;
        validate_name(new_name)?;
        if old_name == new_name {
            return self.open(old_name);
        }
        if self.find_inode(new_name)?.is_some() {
            return Err(StorageError::AlreadyExists);
        }
        let (offset, inode) = self
            .find_directory_entry(old_name)?
            .ok_or(StorageError::NotFound)?;
        let mut directory = [0; BLOCK_SIZE];
        self.read_block(ROOT_DIRECTORY_BLOCK, &mut directory)?;
        let record_length = usize::from(read_u16(&directory, offset + 4));
        if align4(8 + new_name.len()) > record_length {
            return Err(StorageError::DirectoryFull);
        }
        write_u8(&mut directory, offset + 6, new_name.len() as u8);
        copy_bytes_to_offset(&mut directory, offset + 8, new_name);
        self.write_block(ROOT_DIRECTORY_BLOCK, &directory)?;
        Ok(FileHandle {
            inode,
            generation: 1,
        })
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
        self.clear_bit(BLOCK_BITMAP, destination_inode.direct_block)?;
        self.clear_inode(destination_inode_number)?;
        self.adjust_free_counts(1, 1)?;
        Ok(FileHandle {
            inode: source_inode_number,
            generation: 1,
        })
    }

    pub fn list_root(&mut self, entries: &mut [DirectoryEntry]) -> Result<usize, StorageError> {
        let mut directory = [0; BLOCK_SIZE];
        self.read_block(ROOT_DIRECTORY_BLOCK, &mut directory)?;
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

    pub fn write(&mut self, handle: FileHandle, data: &[u8]) -> Result<(), StorageError> {
        if data.len() > MAX_FILE_SIZE {
            return Err(StorageError::FileTooLarge);
        }
        let mut inode = self.validate_handle(handle)?;
        let mut block = [0; BLOCK_SIZE];
        copy_bytes(&mut block, data);
        self.write_block(inode.direct_block, &block)?;
        inode.size = data.len() as u32;
        inode.blocks = 2;
        self.write_inode(handle.inode, inode)
    }

    pub fn read(
        &mut self,
        handle: FileHandle,
        destination: &mut [u8],
    ) -> Result<usize, StorageError> {
        let inode = self.validate_handle(handle)?;
        let length = usize::try_from(inode.size).map_err(|_| StorageError::Corrupt)?;
        if length > MAX_FILE_SIZE {
            return Err(StorageError::Corrupt);
        }
        if destination.len() < length {
            return Err(StorageError::BufferTooSmall);
        }
        let mut block = [0; BLOCK_SIZE];
        self.read_block(inode.direct_block, &mut block)?;
        copy_bytes(destination, block_as_slice(&block, length));
        Ok(length)
    }

    pub fn mmap(&mut self, handle: FileHandle) -> Result<FileMapping, StorageError> {
        let mut mapping = FileMapping {
            handle,
            bytes: [0; MAX_FILE_SIZE],
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
                size: BLOCK_SIZE as u32,
                blocks: 2,
                direct_block: ROOT_DIRECTORY_BLOCK,
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
        let mut directory = [0; BLOCK_SIZE];
        self.read_block(ROOT_DIRECTORY_BLOCK, &mut directory)?;
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

    fn add_directory_entry(
        &mut self,
        name: &[u8],
        inode: u32,
        file_type: u8,
    ) -> Result<(), StorageError> {
        let mut directory = [0; BLOCK_SIZE];
        self.read_block(ROOT_DIRECTORY_BLOCK, &mut directory)?;
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
                return self.write_block(ROOT_DIRECTORY_BLOCK, &directory);
            }
            offset += record_length;
        }
        Err(StorageError::DirectoryFull)
    }

    fn find_directory_entry(&mut self, name: &[u8]) -> Result<Option<(usize, u32)>, StorageError> {
        let mut directory = [0; BLOCK_SIZE];
        self.read_block(ROOT_DIRECTORY_BLOCK, &mut directory)?;
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

    fn clear_inode(&mut self, inode: u32) -> Result<(), StorageError> {
        self.write_inode(
            inode,
            InodeInfo {
                mode: 0,
                size: 0,
                blocks: 0,
                direct_block: 0,
            },
        )
    }

    fn validate_handle(&mut self, handle: FileHandle) -> Result<InodeInfo, StorageError> {
        if handle.generation != 1
            || handle.inode < FIRST_FILE_INODE
            || handle.inode > EXT2_INODE_COUNT
        {
            return Err(StorageError::InvalidHandle);
        }
        let inode = self.read_inode(handle.inode)?;
        if inode.mode & 0xf000 != 0x8000 || inode.direct_block == 0 {
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
            size: read_u32(&bytes, offset + 4),
            blocks: read_u32(&bytes, offset + 28),
            direct_block: read_u32(&bytes, offset + 40),
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
        write_u32(&mut bytes, offset + 4, info.size);
        write_u32(&mut bytes, offset + 28, info.blocks);
        write_u32(&mut bytes, offset + 40, info.direct_block);
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

fn block_read(capability: u64, sector: u64, buffer: &mut [u8; SECTOR_SIZE]) -> bool {
    crate::block_read(capability, sector, buffer)
}

fn block_write(capability: u64, sector: u64, buffer: &[u8; SECTOR_SIZE]) -> bool {
    crate::block_write(capability, sector, buffer)
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

fn block_as_slice(block: &[u8; BLOCK_SIZE], length: usize) -> &[u8] {
    unsafe { core::slice::from_raw_parts(block.as_ptr(), length) }
}

fn copy_bytes(destination: &mut [u8], source: &[u8]) {
    let mut index = 0;
    while index < source.len() {
        unsafe {
            ptr::write_volatile(
                destination.as_mut_ptr().add(index),
                ptr::read_volatile(source.as_ptr().add(index)),
            );
        }
        index += 1;
    }
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
    use super::{
        BlockDevice, DirectoryEntry, FileHandle, FileMapping, StorageError, Vfs, BLOCK_SIZE,
        SECTOR_SIZE,
    };

    struct MemoryBlockDevice {
        sectors: [[u8; SECTOR_SIZE]; 64],
    }

    impl MemoryBlockDevice {
        fn new() -> Self {
            Self {
                sectors: [[0; SECTOR_SIZE]; 64],
            }
        }
    }

    impl BlockDevice for MemoryBlockDevice {
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
            Ok(())
        }
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
    fn rejects_duplicate_names_invalid_handles_and_oversized_files() {
        let (mut volume, _) = Vfs::mount_or_format(MemoryBlockDevice::new()).expect("format");
        let handle = volume.create(b"nagi.txt").expect("create");
        assert_eq!(volume.create(b"nagi.txt"), Err(StorageError::AlreadyExists));
        assert_eq!(
            volume.read(FileHandle::invalid(), &mut [0; 4]),
            Err(StorageError::InvalidHandle)
        );
        assert_eq!(
            volume.write(handle, &[0; BLOCK_SIZE + 1]),
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
