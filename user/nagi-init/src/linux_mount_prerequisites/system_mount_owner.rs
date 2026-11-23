//! Development-only owner for the System fixture mounted by the Task24 boot.

use std::ffi::{CStr, OsStr};
use std::fs::{self, File};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;

use crate::task24_system_mount_identity as identity;

use super::super::mount_flags::{self, ST_NODEV, ST_NOEXEC, ST_NOSUID, ST_RDONLY};
use super::{
    ext4_mount_flags, find_device_by_serial, mount_id, open_directory, read_ext4_superblock,
    syscalls, verify_ext4_mount, DevelopmentDataMountOwner, MountRollback, MOUNT_FLAGS,
};

const TARGET: &CStr = c"/run/nagi/dev-system";
const REQUIRED_FLAGS: u64 = ST_RDONLY | ST_NOSUID | ST_NODEV | ST_NOEXEC;

// The opt-in candidate exposes this to its boot owner before wiring the probe.
#[allow(dead_code)]
pub(in crate::linux_init) struct DevelopmentSystemMountOwner {
    // The directory FD must close before the guard unmounts the filesystem.
    root: File,
    mount_id: u64,
    _lifetime: MountRollback,
}

#[allow(dead_code)]
impl DevelopmentSystemMountOwner {
    /// Require the retained Data owner first. Its mount setup already made
    /// propagation private. The CPIO supplies the unmounted System target.
    pub(super) fn mount(data: &DevelopmentDataMountOwner) -> Result<Self, &'static str> {
        let data_root = data.root.as_ref().ok_or("data_owner_ended")?;
        let device = find_device_by_serial(identity::SERIAL).map_err(|error| error.label())?;
        let superblock = read_ext4_superblock(&device).map_err(|error| error.label())?;
        if !identity::clean_fixed_system(&superblock) {
            return Err("system_identity_or_clean_state_mismatch");
        }
        let metadata = |fd: &File| fd.metadata().map_err(|_| "system_metadata_unavailable");
        if metadata(data_root)?.dev() == device.device_number {
            return Err("system_and_data_share_device");
        }

        let root = open_directory(c"/").map_err(|_| "system_root_unavailable")?;
        let target = open_directory(TARGET).map_err(|_| "system_target_unavailable")?;
        let root_mount_id = mount_id(root.as_raw_fd()).map_err(|error| error.label())?;
        let target_info = metadata(&target)?;
        if mount_id(target.as_raw_fd()).map_err(|error| error.label())? != root_mount_id
            || target_info.uid() != 0
            || target_info.mode() & 0o022 != 0
            || fs::read_dir(OsStr::from_bytes(TARGET.to_bytes()))
                .map_err(|_| "system_target_unavailable")?
                .next()
                .is_some()
        {
            return Err("system_target_not_empty_root_directory");
        }

        // ext4 can replay its journal on an ordinary read-only mount.
        device
            .mount_ext4(TARGET, MOUNT_FLAGS | syscalls::MS_RDONLY, Some(c"noload"))
            .map_err(|error| error.label())?;
        let guard = MountRollback {
            target: TARGET,
            active: true,
        };
        let mounted = open_directory(TARGET).map_err(|_| "system_mount_unavailable")?;
        let mounted_id = mount_id(mounted.as_raw_fd()).map_err(|error| error.label())?;
        let flags = ext4_mount_flags(mounted.as_raw_fd()).map_err(|error| error.label())?;
        mount_flags::verify(REQUIRED_FLAGS, flags).map_err(|_| "system_mount_flags_mismatch")?;
        if mounted_id == 0
            || mounted_id == root_mount_id
            || mounted_id == data.mount_id
            || metadata(&mounted)?.dev() != device.device_number
            || find_device_by_serial(identity::SERIAL)
                .map_err(|error| error.label())?
                .device_number
                != device.device_number
        {
            return Err("system_mounted_device_mismatch");
        }
        Ok(Self {
            root: mounted,
            mount_id: mounted_id,
            _lifetime: guard,
        })
    }

    /// Borrow System first and Data second for the existing Mount probe. Both
    /// descriptors remain owned by their original mount owners.
    pub(super) fn observation_roots<'a>(
        &'a self,
        data: &'a DevelopmentDataMountOwner,
    ) -> Result<(BorrowedFd<'a>, BorrowedFd<'a>), &'static str> {
        let data_root = data.root.as_ref().ok_or("data_owner_ended")?;
        verify_ext4_mount(data_root.as_raw_fd()).map_err(|error| error.label())?;
        if self.mount_id == data.mount_id || data.mount_id == 0 {
            return Err("system_and_data_mount_ids_not_distinct");
        }
        Ok((self.root.as_fd(), data_root.as_fd()))
    }
}
