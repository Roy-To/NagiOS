use libnagi::storage::{BlockDevice, FileHandle, StorageError};
use nagi_pal::FileSystem;

/// Matches the temporary directory names created by the pinned `tempfile`
/// Nagi backend (`.tmp` followed by its six-character random suffix).
pub(crate) fn is_servo_tempdir_name(name: &[u8]) -> bool {
    name.len() == 10
        && name.starts_with(b".tmp")
        && name[4..].iter().all(|byte| byte.is_ascii_alphanumeric())
}

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

#[cfg(test)]
mod tests {
    use super::is_servo_tempdir_name;

    #[test]
    fn identifies_only_tempfile_generated_directory_names() {
        assert!(is_servo_tempdir_name(b".tmpB6sLMi"));
        assert!(!is_servo_tempdir_name(b".tmp"));
        assert!(!is_servo_tempdir_name(b".tmpabcde"));
        assert!(!is_servo_tempdir_name(b".tmpabcde/"));
        assert!(!is_servo_tempdir_name(b".tmpabcde!"));
        assert!(!is_servo_tempdir_name(b"browser-data"));
    }
}
