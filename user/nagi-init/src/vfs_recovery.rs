//! Recovery callers distinguish inspection from an explicit writable repair.
//! The caller must hold exclusive authority over a quiescent volume throughout
//! check/mount/recheck; these steps do not establish cross-process exclusion.

use libnagi::storage::{ReadOnlyBlockDevice, StorageError, Vfs, VfsIntegrityReport};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Inspection {
    Clean(VfsIntegrityReport),
    RecoveryRequired,
}

/// Diagnosis has only the read-only device interface and cannot write or flush.
pub(crate) fn inspect<D: ReadOnlyBlockDevice>(device: &mut D) -> Result<Inspection, StorageError> {
    match Vfs::<D>::check_existing(device) {
        Ok(report) => Ok(Inspection::Clean(report)),
        Err(StorageError::RecoveryRequired) => Ok(Inspection::RecoveryRequired),
        Err(error) => Err(error),
    }
}

/// Only the explicit Recovery-console operation may call this writable path.
/// Reject unknown/corrupt/unformatted input before mounting; never format.
/// Publish a mounted volume only after the recovered disk passes inspection.
#[cfg(any(feature = "m27-recovery", test))]
pub(crate) fn recover_existing<D: libnagi::storage::BlockDevice>(
    mut device: D,
) -> Result<(Vfs<D>, VfsIntegrityReport), StorageError> {
    let _ = inspect(&mut device)?;
    let volume = Vfs::mount_existing(device)?;
    let mut device = volume.into_device();
    let report = Vfs::<D>::check_existing(&mut device)?;
    let volume = Vfs::mount_existing(device)?;
    Ok((volume, report))
}
