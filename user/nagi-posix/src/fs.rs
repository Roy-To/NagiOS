use libnagi::storage::{BlockDevice, FileHandle, StorageError};
use nagi_pal::FileSystem;

pub fn open<D: BlockDevice>(
    filesystem: &mut FileSystem<D>,
    name: &[u8],
) -> Result<FileHandle, StorageError> {
    filesystem.open(name)
}

pub fn read<D: BlockDevice>(
    filesystem: &mut FileSystem<D>,
    handle: FileHandle,
    destination: &mut [u8],
) -> Result<usize, StorageError> {
    filesystem.read(handle, destination)
}

pub fn write<D: BlockDevice>(
    filesystem: &mut FileSystem<D>,
    handle: FileHandle,
    bytes: &[u8],
) -> Result<(), StorageError> {
    filesystem.write(handle, bytes)
}
