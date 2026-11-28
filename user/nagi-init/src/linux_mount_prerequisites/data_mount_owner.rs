//! Development-only ext4 Data mount owner used by the dedicated Linux guest.
//!
//! The input comes from a signed development selection file in initramfs; the
//! configured UUID must match the ext4 UUID read from the selected block device.
//! This module never formats, repairs, or initializes
//! a device. The explicit development provisioning boot may create private
//! identity and Task24 journal directories on a fresh development disk.
//! It retains the mounted directory FD and does not authenticate a Profile
//! or issue a Verified Notes attachment.

use crate::profile_volume_notes_owner::{BootVolumeInput, DevelopmentBootVolumeIdentity};
use crate::task24_c3_journal_bootstrap_contract::identity_paths_avoid_data_journal;
#[cfg(linux_dev_preview_smoke)]
use crate::task24_c3_journal_bootstrap_contract::{
    is_fresh_development_data_root, DATA_BOOTSTRAP_MARKER_BYTES, DATA_BOOTSTRAP_MARKER_NAME,
    DATA_JOURNAL_DIRECTORY,
};
use nagi_data_boot_selection::{DataBootSelection, SelectionError, SelectionMode};
use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::mem::MaybeUninit;
#[cfg(all(linux_dev_system_mount, not(nagi_release_build)))]
use std::os::fd::BorrowedFd;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::{FileExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::syscalls;

#[cfg(all(
    linux_dev_system_mount,
    not(any(linux_dev_preview_smoke, nagi_release_build))
))]
#[path = "system_mount_owner.rs"]
pub(in crate::linux_init) mod system_mount_owner;

#[cfg(all(linux_dev_system_mount, not(nagi_release_build)))]
#[path = "system_data_observation.rs"]
pub(super) mod system_data_observation;

#[cfg(all(
    linux_dev_system_mount,
    linux_dev_system_verity,
    target_os = "linux",
    target_arch = "x86_64",
    not(any(linux_dev_preview_smoke, nagi_release_build))
))]
#[path = "system_verity.rs"]
pub(in crate::linux_init) mod system_verity;

const EXT4_SUPER_MAGIC: u64 = 0xef53;
const EXT4_SUPERBLOCK_OFFSET: u64 = 1024;
const EXT4_SUPERBLOCK_BYTES: usize = 1024;
const EXT4_MAGIC_OFFSET: usize = 56;
const EXT4_UUID_OFFSET: usize = 104;
const EXPECTED_MOUNT_PATH: &str = "/run/nagi/dev-data";
const SELECTION_CONFIG_PATH: &str = "/nagi-dev-data-selection.signed";
const DEVICE_SYSFS_ROOT: &str = "/sys/class/block";
const SYSTEM_DEVICE_SYSFS_ROOT: &str = "/sys/block";
const DEVICE_NODE_ROOT: &str = "/dev";
const MARKER_NAME: &std::ffi::CStr = c"nagi-dev-data-persistence.txt";
const MARKER_CONTENT: &[u8] = b"NAGI_DEV_DATA_PERSISTED_V1\n";
const O_RDONLY: std::os::raw::c_int = 0;
const O_WRONLY: std::os::raw::c_int = 1;
const O_CREAT: std::os::raw::c_int = 0o100;
const O_EXCL: std::os::raw::c_int = 0o200;
const O_CLOEXEC: std::os::raw::c_int = 0o2000000;
const O_NOFOLLOW: std::os::raw::c_int = 0o400000;
const O_DIRECTORY: std::os::raw::c_int = 0o200000;
const MS_REC: std::os::raw::c_ulong = 1 << 14;
const MS_PRIVATE: std::os::raw::c_ulong = 1 << 18;
const MOUNT_FLAGS: std::os::raw::c_ulong =
    syscalls::MS_NOSUID | syscalls::MS_NODEV | syscalls::MS_NOEXEC;
const UMOUNT_NOFOLLOW: std::os::raw::c_int = 8;
const STATX_MNT_ID_MASK: std::os::raw::c_uint = syscalls::STATX_MNT_ID;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SmokeMode {
    Write,
    Read,
    Reject,
    #[cfg(linux_dev_preview_smoke)]
    ProvisionIdentityDirectories,
}

#[derive(Debug)]
struct SmokeRequest {
    mode: SmokeMode,
    filesystem_uuid: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataMountError {
    SelectionMalformed,
    SelectionSignatureInvalid,
    DeviceNotFound,
    DeviceAmbiguous,
    DeviceChanged,
    NotExt4,
    FilesystemUuidMismatch,
    MountpointUnavailable,
    AlreadyMounted,
    MountRejected,
    MountVerificationFailed,
    PersistenceMismatch,
    IdentityPathConflict,
    OwnerEndFailed,
}

impl DataMountError {
    const fn label(self) -> &'static str {
        match self {
            Self::SelectionMalformed => "selection_malformed",
            Self::SelectionSignatureInvalid => "selection_signature_invalid",
            Self::DeviceNotFound => "device_not_found",
            Self::DeviceAmbiguous => "device_ambiguous",
            Self::DeviceChanged => "device_changed",
            Self::NotExt4 => "not_ext4",
            Self::FilesystemUuidMismatch => "filesystem_uuid_mismatch",
            Self::MountpointUnavailable => "mountpoint_unavailable",
            Self::AlreadyMounted => "mountpoint_already_mounted",
            Self::MountRejected => "mount_rejected",
            Self::MountVerificationFailed => "mount_verification_failed",
            Self::PersistenceMismatch => "persistence_mismatch",
            Self::IdentityPathConflict => "identity_path_conflict",
            Self::OwnerEndFailed => "owner_end_failed",
        }
    }
}

struct SelectedBlockDevice {
    file: File,
    path: CString,
    device_number: u64,
    serial: String,
}

impl SelectedBlockDevice {
    fn mount_ext4(
        &self,
        target: &CStr,
        flags: std::os::raw::c_ulong,
        options: Option<&CStr>,
    ) -> Result<(), DataMountError> {
        let options = options.map_or(std::ptr::null(), |value| value.as_ptr().cast());
        if unsafe {
            syscalls::mount(
                self.path.as_ptr(),
                target.as_ptr(),
                c"ext4".as_ptr(),
                flags,
                options,
            )
        } != 0
        {
            return Err(DataMountError::MountRejected);
        }
        Ok(())
    }
}

/// Owns one verified development ext4 mount and its retained root directory FD.
pub(super) struct DevelopmentDataMountOwner {
    root: Option<File>,
    identity: DevelopmentBootVolumeIdentity,
    device_serial: String,
    root_mount_id: u64,
    mount_id: u64,
    mounted: bool,
}

impl DevelopmentDataMountOwner {
    fn mount(filesystem_uuid: &str) -> Result<Self, DataMountError> {
        if !valid_uuid(filesystem_uuid) {
            return Err(DataMountError::SelectionMalformed);
        }

        let device = find_device_by_uuid(filesystem_uuid)?;
        let observed_uuid = read_ext4_uuid(&device)?;
        if observed_uuid != filesystem_uuid {
            return Err(DataMountError::FilesystemUuidMismatch);
        }

        ensure_private_mount_target()?;
        let root_before =
            open_directory(c"/").map_err(|_| DataMountError::MountpointUnavailable)?;
        let target_before = open_directory(c"/run/nagi/dev-data")
            .map_err(|_| DataMountError::MountpointUnavailable)?;
        let root_mount_id = mount_id(root_before.as_raw_fd())?;
        if mount_id(target_before.as_raw_fd())? != root_mount_id {
            return Err(DataMountError::AlreadyMounted);
        }
        if fs::read_dir(EXPECTED_MOUNT_PATH)
            .map_err(|_| DataMountError::MountpointUnavailable)?
            .next()
            .is_some()
        {
            return Err(DataMountError::MountpointUnavailable);
        }

        let propagation_result = unsafe {
            syscalls::mount(
                std::ptr::null(),
                c"/".as_ptr(),
                std::ptr::null(),
                MS_REC | MS_PRIVATE,
                std::ptr::null(),
            )
        };
        if propagation_result != 0 {
            return Err(DataMountError::MountRejected);
        }

        device.mount_ext4(c"/run/nagi/dev-data", MOUNT_FLAGS, None)?;
        let mut rollback = MountRollback {
            target: c"/run/nagi/dev-data",
            active: true,
        };
        let root = open_directory(c"/run/nagi/dev-data")
            .map_err(|_| DataMountError::MountVerificationFailed)?;
        verify_ext4_mount(root.as_raw_fd())?;
        let mount_id = mount_id(root.as_raw_fd())?;
        if mount_id == root_mount_id {
            return Err(DataMountError::MountVerificationFailed);
        }

        // Verify the mounted root's actual device and recheck the signed UUID
        // before changing permissions through the retained FD.
        secure_mounted_data_root(&root, device.device_number, || {
            if find_device_by_uuid(filesystem_uuid)?.device_number != device.device_number
                || read_ext4_uuid(&device)? != filesystem_uuid
            {
                return Err(DataMountError::DeviceChanged);
            }
            Ok(())
        })?;

        rollback.active = false;
        Ok(Self {
            root: Some(root),
            identity: DevelopmentBootVolumeIdentity::new(
                device.serial.clone(),
                filesystem_uuid.to_owned(),
                mount_id,
            ),
            device_serial: device.serial,
            root_mount_id,
            mount_id,
            mounted: true,
        })
    }

    fn boot_volume_input(&self) -> Result<BootVolumeInput, DataMountError> {
        let root = self
            .root
            .as_ref()
            .ok_or(DataMountError::OwnerEndFailed)?
            .try_clone()
            .map_err(|_| DataMountError::OwnerEndFailed)?;
        Ok(BootVolumeInput::from_development_data_mount(
            root,
            self.identity.clone(),
        ))
    }

    fn write_marker(&self) -> Result<(), DataMountError> {
        let mut file = self.open_marker(true)?;
        file.write_all(MARKER_CONTENT)
            .map_err(|_| DataMountError::PersistenceMismatch)?;
        file.sync_all()
            .map_err(|_| DataMountError::PersistenceMismatch)?;
        self.root
            .as_ref()
            .ok_or(DataMountError::OwnerEndFailed)?
            .sync_all()
            .map_err(|_| DataMountError::PersistenceMismatch)
    }

    fn verify_marker(&self) -> Result<(), DataMountError> {
        let mut file = self.open_marker(false)?;
        let metadata = file
            .metadata()
            .map_err(|_| DataMountError::PersistenceMismatch)?;
        if !metadata.file_type().is_file()
            || metadata.uid() != 0
            || metadata.mode() & 0o777 != 0o600
            || metadata.nlink() != 1
        {
            return Err(DataMountError::PersistenceMismatch);
        }
        let mut content = Vec::new();
        file.read_to_end(&mut content)
            .map_err(|_| DataMountError::PersistenceMismatch)?;
        if content == MARKER_CONTENT {
            Ok(())
        } else {
            Err(DataMountError::PersistenceMismatch)
        }
    }

    #[cfg(linux_dev_preview_smoke)]
    fn provision_identity_directories(&self) -> io::Result<()> {
        self.verify_marker()
            .map_err(|error| io::Error::other(error.label()))?;
        self.require_fresh_identity_root()?;
        let input = self
            .boot_volume_input()
            .map_err(|error| io::Error::other(error.label()))?;
        let selected = input.selected_data_root()?;
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| io::Error::other("Data owner ended"))?;
        for name in ["accounts", "snapshots", DATA_JOURNAL_DIRECTORY] {
            rustix::fs::mkdirat(root, name, rustix::fs::Mode::RWXU)?;
            // Require private ownership for each directory and open them
            // below the retained root without crossing mounts or symlinks.
            selected
                .retain_identity_snapshot_directory(Path::new(name))?
                .sync_all()?;
        }
        input.identity_inputs_for_boot(Path::new("accounts"), Path::new("snapshots"))?;
        let journal_directory =
            selected.retain_identity_snapshot_directory(Path::new(DATA_JOURNAL_DIRECTORY))?;
        let marker_name = CString::new(DATA_BOOTSTRAP_MARKER_NAME)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid marker name"))?;
        let raw_marker = unsafe {
            syscalls::openat(
                journal_directory.as_raw_fd(),
                marker_name.as_ptr(),
                O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW,
                0o600 as std::os::raw::c_int,
            )
        };
        if raw_marker < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut marker = unsafe { File::from_raw_fd(raw_marker) };
        let marker_metadata = marker.metadata()?;
        if !marker_metadata.file_type().is_file()
            || marker_metadata.uid() != 0
            || marker_metadata.mode() & 0o777 != 0o600
            || marker_metadata.nlink() != 1
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Task24 C3 bootstrap marker is not a private regular file",
            ));
        }
        marker.write_all(DATA_BOOTSTRAP_MARKER_BYTES)?;
        marker.sync_all()?;
        journal_directory.sync_all()?;
        root.sync_all()
    }

    #[cfg(linux_dev_preview_smoke)]
    fn require_fresh_identity_root(&self) -> io::Result<()> {
        let root = self
            .root
            .as_ref()
            .ok_or_else(|| io::Error::other("Data owner ended"))?;
        let entries = read_directory_entries(root)?;
        if !is_fresh_development_data_root(&entries) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "identity provisioning requires an unregistered fresh Data root",
            ));
        }
        let input = self
            .boot_volume_input()
            .map_err(|error| io::Error::other(error.label()))?;
        let selected = input.selected_data_root()?;
        let lost_found = selected.retain_identity_snapshot_directory(Path::new("lost+found"))?;
        let metadata = lost_found.metadata()?;
        if metadata.uid() != 0 || metadata.mode() & 0o777 != 0o700 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "fresh Data lost+found directory does not have its expected owner and mode",
            ));
        }
        if !read_directory_entries(&lost_found)?.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "fresh Data lost+found directory is not empty",
            ));
        }
        Ok(())
    }

    fn open_marker(&self, write: bool) -> Result<File, DataMountError> {
        let root = self.root.as_ref().ok_or(DataMountError::OwnerEndFailed)?;
        let flags = if write {
            O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW
        } else {
            O_RDONLY | O_CLOEXEC | O_NOFOLLOW | libc::O_NONBLOCK
        };
        let raw_fd = unsafe {
            syscalls::openat(
                root.as_raw_fd(),
                MARKER_NAME.as_ptr(),
                flags,
                0o600 as std::os::raw::c_int,
            )
        };
        if raw_fd < 0 {
            return Err(DataMountError::PersistenceMismatch);
        }
        Ok(unsafe { File::from_raw_fd(raw_fd) })
    }

    fn end(&mut self) -> Result<(), DataMountError> {
        if !self.mounted {
            return Ok(());
        }
        // The retained root FD prevents a normal unmount from completing.
        // The caller first ends the typed Notes input, so release this final
        // owner-held reference before detaching the mount.
        drop(self.root.take());
        let result = unsafe { syscalls::umount2(c"/run/nagi/dev-data".as_ptr(), UMOUNT_NOFOLLOW) };
        if result != 0 {
            return Err(DataMountError::OwnerEndFailed);
        }
        self.mounted = false;
        Ok(())
    }

    fn verify_ended(&self) -> Result<(), DataMountError> {
        let target = open_directory(c"/run/nagi/dev-data")
            .map_err(|_| DataMountError::MountVerificationFailed)?;
        if mount_id(target.as_raw_fd())? == self.root_mount_id {
            Ok(())
        } else {
            Err(DataMountError::MountVerificationFailed)
        }
    }

    #[allow(dead_code)]
    fn mount_id(&self) -> u64 {
        self.mount_id
    }
}

#[cfg(linux_dev_preview_smoke)]
fn read_directory_entries(directory: &File) -> io::Result<Vec<std::ffi::OsString>> {
    let path = format!("/proc/self/fd/{}", directory.as_raw_fd());
    fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect()
}

/// Owns the retained boot input and releases it before ending the mount.
#[cfg(not(any(linux_dev_preview_smoke, nagi_release_build)))]
pub(in crate::linux_init) struct DevelopmentIdentityData {
    pub(in crate::linux_init) input: BootVolumeInput,
    pub(in crate::linux_init) accounts: PathBuf,
    pub(in crate::linux_init) snapshots: PathBuf,
    #[cfg(all(linux_dev_system_mount, not(nagi_release_build)))]
    system_mount: Option<system_mount_owner::DevelopmentSystemMountOwner>,
    _mount: DevelopmentDataMountOwner,
}

/// Opt in to the same verified development mount while running normal PID 1.
#[cfg(not(any(linux_dev_preview_smoke, nagi_release_build)))]
pub(in crate::linux_init) fn mount_identity_data(
) -> Result<Option<DevelopmentIdentityData>, &'static str> {
    let Some(bytes) = read_signed_selection_config().map_err(DataMountError::label)? else {
        return Ok(None);
    };
    let selection = DataBootSelection::verify(&bytes)
        .map_err(selection_error)
        .map_err(DataMountError::label)?;
    if selection.mode != SelectionMode::Identity {
        return Err(DataMountError::SelectionMalformed.label());
    }
    let accounts = PathBuf::from(selection.accounts);
    let snapshots = PathBuf::from(selection.snapshots);
    if !identity_paths_avoid_data_journal(&accounts, &snapshots) {
        return Err(DataMountError::IdentityPathConflict.label());
    }
    let mount = DevelopmentDataMountOwner::mount(selection.filesystem_uuid)
        .map_err(DataMountError::label)?;
    let input = mount.boot_volume_input().map_err(DataMountError::label)?;
    Ok(Some(DevelopmentIdentityData {
        input,
        accounts,
        snapshots,
        #[cfg(all(linux_dev_system_mount, not(nagi_release_build)))]
        system_mount: None,
        _mount: mount,
    }))
}

fn read_signed_selection_config() -> Result<Option<Vec<u8>>, DataMountError> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(O_CLOEXEC | O_NOFOLLOW)
        .open(SELECTION_CONFIG_PATH)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(DataMountError::SelectionMalformed),
    };
    let metadata = file
        .metadata()
        .map_err(|_| DataMountError::SelectionMalformed)?;
    if !metadata.is_file()
        || metadata.len() <= nagi_data_boot_selection::SIGNATURE_BYTES as u64
        || metadata.len() > nagi_data_boot_selection::MAX_CONFIG_BYTES as u64
    {
        return Err(DataMountError::SelectionMalformed);
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((nagi_data_boot_selection::MAX_CONFIG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| DataMountError::SelectionMalformed)?;
    if bytes.len() > nagi_data_boot_selection::MAX_CONFIG_BYTES {
        return Err(DataMountError::SelectionMalformed);
    }
    Ok(Some(bytes))
}

fn selection_error(error: SelectionError) -> DataMountError {
    match error {
        SelectionError::InvalidLength | SelectionError::Malformed => {
            DataMountError::SelectionMalformed
        }
        SelectionError::Signature => DataMountError::SelectionSignatureInvalid,
        SelectionError::FilesystemUuidMismatch => DataMountError::FilesystemUuidMismatch,
    }
}

#[cfg(all(linux_dev_system_mount, not(nagi_release_build)))]
pub(in crate::linux_init) fn requested_system_data_probe(
) -> Result<Option<crate::task24_system_data_probe_config::ProbeRequest>, &'static str> {
    system_data_observation::requested_probe()
}

/// Observe roots borrowed from the live System and Data mount owners.
///
/// The caller keeps both owners alive for the duration of this call. This
/// adapter does not mount, unmount, close, or retain either descriptor.
#[cfg(all(linux_dev_system_mount, not(nagi_release_build)))]
pub(in crate::linux_init) fn observe_borrowed_owner_roots(
    system_root: BorrowedFd<'_>,
    data_root: BorrowedFd<'_>,
    system_fstype: &str,
    data_fstype: &str,
    token: &str,
) -> Result<String, String> {
    system_data_observation::observe_existing_roots(
        system_root,
        data_root,
        system_fstype,
        data_fstype,
        token,
    )
}

#[cfg(all(
    linux_dev_system_mount,
    not(any(linux_dev_preview_smoke, nagi_release_build))
))]
impl DevelopmentIdentityData {
    pub(in crate::linux_init) fn observe_system_data(
        &mut self,
        token: &str,
    ) -> Result<String, String> {
        if self.system_mount.is_none() {
            self.system_mount = Some(
                system_mount_owner::DevelopmentSystemMountOwner::mount(&self._mount)
                    .map_err(str::to_owned)?,
            );
        }
        let system = self
            .system_mount
            .as_ref()
            .ok_or_else(|| "system_mount_owner_unavailable".to_owned())?;
        let (system_root, data_root) = system
            .observation_roots(&self._mount)
            .map_err(str::to_owned)?;
        observe_borrowed_owner_roots(system_root, data_root, "ext4", "ext4", token)
    }
}

impl Drop for DevelopmentDataMountOwner {
    fn drop(&mut self) {
        let _ = self.end();
    }
}

struct MountRollback {
    target: &'static CStr,
    active: bool,
}

impl Drop for MountRollback {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                let _ = syscalls::umount2(self.target.as_ptr(), UMOUNT_NOFOLLOW);
            }
        }
    }
}

fn parse_smoke_request() -> Result<Option<SmokeRequest>, DataMountError> {
    let Some(bytes) = read_signed_selection_config()? else {
        return Ok(None);
    };
    let selection = DataBootSelection::verify(&bytes).map_err(selection_error)?;
    let mode = match selection.mode {
        SelectionMode::Identity => return Ok(None),
        SelectionMode::SmokeWrite => SmokeMode::Write,
        SelectionMode::SmokeRead => SmokeMode::Read,
        SelectionMode::SmokeReject => SmokeMode::Reject,
        SelectionMode::SmokeProvisionIdentityDirectories => {
            #[cfg(linux_dev_preview_smoke)]
            {
                SmokeMode::ProvisionIdentityDirectories
            }
            #[cfg(not(linux_dev_preview_smoke))]
            {
                return Err(DataMountError::SelectionMalformed);
            }
        }
    };
    Ok(Some(SmokeRequest {
        mode,
        filesystem_uuid: selection.filesystem_uuid.to_owned(),
    }))
}

fn find_device_by_uuid(expected_uuid: &str) -> Result<SelectedBlockDevice, DataMountError> {
    let sysfs_root = Path::new(DEVICE_SYSFS_ROOT);
    let names = block_device_names(sysfs_root)?;
    let mut matches = Vec::new();
    let mut seen_device_numbers = Vec::new();
    let mut found_ext4 = false;
    for name in names {
        let sysfs_path = sysfs_root.join(&name);
        let device_path = Path::new(DEVICE_NODE_ROOT).join(&name);
        match fs::symlink_metadata(&device_path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return Err(DataMountError::DeviceNotFound),
            Ok(metadata)
                if metadata.file_type().is_symlink() || !metadata.file_type().is_block_device() =>
            {
                return Err(DataMountError::DeviceChanged)
            }
            Ok(_) => {}
        }
        let device = open_block_device(name, &sysfs_path)?;
        if !remember_device_number(&mut seen_device_numbers, device.device_number) {
            continue;
        }
        match read_ext4_uuid(&device) {
            Ok(observed_uuid) => {
                found_ext4 = true;
                if observed_uuid == expected_uuid {
                    matches.push(device);
                }
            }
            Err(DataMountError::NotExt4) => {}
            Err(error) => return Err(error),
        }
    }
    if matches.is_empty() {
        return Err(if found_ext4 {
            DataMountError::FilesystemUuidMismatch
        } else {
            DataMountError::DeviceNotFound
        });
    }
    if matches.len() != 1 {
        return Err(DataMountError::DeviceAmbiguous);
    }
    matches.pop().ok_or(DataMountError::DeviceNotFound)
}

// The System fixture has its own fixed serial identity. Data selection never
// uses this path; it is retained for the separate read-only System owner.
fn find_device_by_serial(expected_serial: &str) -> Result<SelectedBlockDevice, DataMountError> {
    let sysfs_root = Path::new(SYSTEM_DEVICE_SYSFS_ROOT);
    let entries = fs::read_dir(sysfs_root).map_err(|_| DataMountError::DeviceNotFound)?;
    let mut matches = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| DataMountError::DeviceNotFound)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| DataMountError::DeviceNotFound)?;
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'!' | b'_' | b'-'))
        {
            continue;
        }
        let serial_path = entry.path().join("serial");
        let serial = match fs::read_to_string(serial_path) {
            Ok(serial) => serial.trim().to_owned(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => return Err(DataMountError::DeviceNotFound),
        };
        if serial == expected_serial {
            matches.push(name);
        }
    }
    if matches.is_empty() {
        return Err(DataMountError::DeviceNotFound);
    }
    if matches.len() != 1 {
        return Err(DataMountError::DeviceAmbiguous);
    }
    let name = matches.pop().ok_or(DataMountError::DeviceNotFound)?;
    let device = open_block_device(name.clone(), &sysfs_root.join(&name))?;
    if device.serial != expected_serial {
        return Err(DataMountError::DeviceChanged);
    }
    Ok(device)
}

fn block_device_names(sysfs_root: &Path) -> Result<Vec<String>, DataMountError> {
    let entries = fs::read_dir(sysfs_root).map_err(|_| DataMountError::DeviceNotFound)?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|_| DataMountError::DeviceNotFound)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| DataMountError::DeviceNotFound)?;
        if valid_block_device_name(&name) {
            names.push(name);
        }
    }
    Ok(names)
}

fn valid_block_device_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'!' | b'_' | b'-'))
}

fn remember_device_number(seen: &mut Vec<u64>, device_number: u64) -> bool {
    if seen.contains(&device_number) {
        return false;
    }
    seen.push(device_number);
    true
}

fn open_block_device(
    name: String,
    sysfs_path: &Path,
) -> Result<SelectedBlockDevice, DataMountError> {
    let device_path = Path::new(DEVICE_NODE_ROOT).join(&name);
    let metadata =
        fs::symlink_metadata(&device_path).map_err(|_| DataMountError::DeviceNotFound)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_block_device() {
        return Err(DataMountError::DeviceChanged);
    }
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(O_CLOEXEC | O_NOFOLLOW)
        .open(&device_path)
        .map_err(|_| DataMountError::DeviceNotFound)?;
    let metadata = opened
        .metadata()
        .map_err(|_| DataMountError::DeviceNotFound)?;
    if !metadata.file_type().is_block_device() {
        return Err(DataMountError::DeviceChanged);
    }
    let sysfs_dev =
        fs::read_to_string(sysfs_path.join("dev")).map_err(|_| DataMountError::DeviceNotFound)?;
    let (expected_major, expected_minor) = parse_major_minor(sysfs_dev.trim())?;
    let actual_dev = metadata.rdev();
    if linux_device_major(actual_dev) != expected_major
        || linux_device_minor(actual_dev) != expected_minor
    {
        return Err(DataMountError::DeviceChanged);
    }
    let serial = read_device_serial(sysfs_path, &name);
    let path = fd_source_path(opened.as_raw_fd())?;
    Ok(SelectedBlockDevice {
        file: opened,
        path,
        device_number: actual_dev,
        serial,
    })
}

fn read_device_serial(sysfs_path: &Path, name: &str) -> String {
    for serial_path in [sysfs_path.join("serial"), sysfs_path.join("device/serial")] {
        if let Some(serial) = fs::read_to_string(serial_path)
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| valid_serial(value))
        {
            return serial;
        }
    }
    name.to_owned()
}

fn fd_source_path(fd: RawFd) -> Result<CString, DataMountError> {
    CString::new(format!("/proc/self/fd/{fd}")).map_err(|_| DataMountError::DeviceChanged)
}

fn read_ext4_uuid(device: &SelectedBlockDevice) -> Result<String, DataMountError> {
    let superblock = read_ext4_superblock(device)?;
    Ok(format_uuid(
        &superblock[EXT4_UUID_OFFSET..EXT4_UUID_OFFSET + 16],
    ))
}

fn read_ext4_superblock(
    device: &SelectedBlockDevice,
) -> Result<[u8; EXT4_SUPERBLOCK_BYTES], DataMountError> {
    let metadata = device
        .file
        .metadata()
        .map_err(|_| DataMountError::DeviceNotFound)?;
    if !metadata.file_type().is_block_device() || metadata.rdev() != device.device_number {
        return Err(DataMountError::DeviceChanged);
    }
    let mut superblock = [0u8; EXT4_SUPERBLOCK_BYTES];
    let mut offset = EXT4_SUPERBLOCK_OFFSET;
    let mut read = 0;
    while read < superblock.len() {
        let count = device
            .file
            .read_at(&mut superblock[read..], offset)
            .map_err(|_| DataMountError::NotExt4)?;
        if count == 0 {
            return Err(DataMountError::NotExt4);
        }
        read += count;
        offset += count as u64;
    }
    if superblock[EXT4_MAGIC_OFFSET..EXT4_MAGIC_OFFSET + 2] != [0x53, 0xef] {
        return Err(DataMountError::NotExt4);
    }
    Ok(superblock)
}

fn format_uuid(bytes: &[u8]) -> String {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase(),
        })
}

fn valid_serial(value: &str) -> bool {
    (1..=20).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn parse_major_minor(value: &str) -> Result<(u32, u32), DataMountError> {
    let (major, minor) = value.split_once(':').ok_or(DataMountError::DeviceChanged)?;
    Ok((
        major.parse().map_err(|_| DataMountError::DeviceChanged)?,
        minor.parse().map_err(|_| DataMountError::DeviceChanged)?,
    ))
}

fn linux_device_major(device: u64) -> u32 {
    (((device >> 8) & 0x0fff) | ((device >> 32) & 0xffff_f000)) as u32
}

fn linux_device_minor(device: u64) -> u32 {
    ((device & 0x00ff) | ((device >> 12) & 0xffff_ff00)) as u32
}

fn ensure_private_mount_target() -> Result<(), DataMountError> {
    ensure_directory(Path::new("/run"), false)?;
    ensure_directory(Path::new("/run/nagi"), true)?;
    ensure_directory(Path::new(EXPECTED_MOUNT_PATH), true)?;
    Ok(())
}

fn ensure_directory(path: &Path, private: bool) -> Result<(), DataMountError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => return Err(DataMountError::MountpointUnavailable),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|_| DataMountError::MountpointUnavailable)?;
        }
        Err(_) => return Err(DataMountError::MountpointUnavailable),
    }
    let mode = if private { 0o700 } else { 0o755 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|_| DataMountError::MountpointUnavailable)
}

fn open_directory(path: &std::ffi::CStr) -> Result<File, DataMountError> {
    let raw_fd = unsafe {
        syscalls::open(
            path.as_ptr(),
            O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW,
        )
    };
    if raw_fd < 0 {
        return Err(DataMountError::MountpointUnavailable);
    }
    Ok(unsafe { File::from_raw_fd(raw_fd) })
}

fn mount_id(fd: std::os::raw::c_int) -> Result<u64, DataMountError> {
    let mut stat = MaybeUninit::<syscalls::Statx>::zeroed();
    let result = unsafe {
        syscalls::statx(
            fd,
            c"".as_ptr(),
            syscalls::AT_EMPTY_PATH,
            STATX_MNT_ID_MASK,
            stat.as_mut_ptr(),
        )
    };
    if result != 0 {
        return Err(DataMountError::MountVerificationFailed);
    }
    let stat = unsafe { stat.assume_init() };
    if stat.stx_mask & STATX_MNT_ID_MASK == 0 {
        return Err(DataMountError::MountVerificationFailed);
    }
    Ok(stat.stx_mnt_id)
}

fn verify_ext4_mount(fd: std::os::raw::c_int) -> Result<(), DataMountError> {
    if ext4_mount_flags(fd)? & 1 != 0 {
        return Err(DataMountError::MountVerificationFailed);
    }
    Ok(())
}

fn secure_mounted_data_root(
    root: &File,
    selected_device: u64,
    verify_selection: impl FnOnce() -> Result<(), DataMountError>,
) -> Result<(), DataMountError> {
    if root
        .metadata()
        .map_err(|_| DataMountError::MountVerificationFailed)?
        .dev()
        != selected_device
    {
        return Err(DataMountError::DeviceChanged);
    }
    verify_selection()?;
    root.set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|_| DataMountError::MountVerificationFailed)
}

fn ext4_mount_flags(fd: std::os::raw::c_int) -> Result<u64, DataMountError> {
    let mut stat = MaybeUninit::<syscalls::StatFs>::zeroed();
    let result = unsafe { syscalls::fstatfs(fd, stat.as_mut_ptr()) };
    if result != 0 {
        return Err(DataMountError::MountVerificationFailed);
    }
    let stat = unsafe { stat.assume_init() };
    if stat.f_type as u64 != EXT4_SUPER_MAGIC {
        return Err(DataMountError::MountVerificationFailed);
    }
    Ok(stat.f_flags as u64)
}

fn hold_pid1() -> ! {
    loop {
        std::thread::park_timeout(Duration::from_secs(60));
    }
}

/// Enter only when a signed development-only Data selection file is present.
pub(super) fn maybe_run_smoke() -> bool {
    let request = match parse_smoke_request() {
        Ok(None) => return false,
        Ok(Some(request)) => request,
        Err(error) => {
            super::super::log_pid1_stderr(format_args!(
                "NAGI_DEV_DATA_OWNER_UNAVAILABLE reason={}",
                error.label()
            ));
            hold_pid1();
        }
    };
    run_smoke(request)
}

fn run_smoke(request: SmokeRequest) -> ! {
    match request.mode {
        #[cfg(linux_dev_preview_smoke)]
        SmokeMode::ProvisionIdentityDirectories => {
            let result = (|| -> io::Result<()> {
                let mut owner = DevelopmentDataMountOwner::mount(&request.filesystem_uuid)
                    .map_err(|error| io::Error::other(error.label()))?;
                owner.provision_identity_directories()?;
                owner
                    .end()
                    .map_err(|error| io::Error::other(error.label()))?;
                owner
                    .verify_ended()
                    .map_err(|error| io::Error::other(error.label()))
            })();
            match result {
                Ok(()) => super::super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_IDENTITY_DIRECTORIES_PROVISIONED accounts=accounts snapshots=snapshots c3_journal=system-service-journal first_provision=true owner_ended=true"
                )),
                Err(error) => super::super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_IDENTITY_DIRECTORIES_PROVISION_FAILED ({error})"
                )),
            }
            hold_pid1();
        }
        SmokeMode::Reject => {
            match DevelopmentDataMountOwner::mount(&request.filesystem_uuid) {
                Ok(mut owner) => {
                    let _ = owner.end();
                    super::super::log_pid1_stderr(format_args!(
                        "NAGI_DEV_DATA_REJECT_FAIL reason=unexpected_mount"
                    ));
                }
                Err(error) => {
                    super::super::log_pid1_stderr(format_args!(
                        "NAGI_DEV_DATA_REJECT_PASS reason={}",
                        error.label()
                    ));
                }
            }
            hold_pid1();
        }
        SmokeMode::Write | SmokeMode::Read => {
            let mut owner = match DevelopmentDataMountOwner::mount(&request.filesystem_uuid) {
                Ok(owner) => owner,
                Err(error) => {
                    super::super::log_pid1_stderr(format_args!(
                        "NAGI_DEV_DATA_OWNER_UNAVAILABLE reason={}",
                        error.label()
                    ));
                    hold_pid1();
                }
            };
            let input = match owner.boot_volume_input() {
                Ok(input) => input,
                Err(error) => {
                    super::super::log_pid1_stderr(format_args!(
                        "NAGI_DEV_DATA_OWNER_FAIL phase=boot_input reason={}",
                        error.label()
                    ));
                    hold_pid1();
                }
            };
            let mut profile_owner =
                crate::profile_volume_notes_owner::pending_with_boot_volume(input);
            let notes_are_pending = profile_owner.status()
                == nagi_profile_volume_notes_owner::ProfileVolumeNotesOwnerStatus::Pending
                && profile_owner.has_boot_volume_input()
                && matches!(
                    crate::linux_init::normal_startup_profile_notes_bundle(&profile_owner),
                    Err(nagi_profile_volume_notes_owner::ProfileVolumeNotesOwnerError::Unavailable)
                );
            let persistence = match request.mode {
                SmokeMode::Write => owner.write_marker(),
                SmokeMode::Read => owner.verify_marker(),
                _ => unreachable!(),
            };
            if persistence.is_err() || !notes_are_pending {
                super::super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_DATA_OWNER_FAIL phase={} persist={} notes_pending={}",
                    smoke_mode_name(request.mode),
                    persistence.is_ok(),
                    notes_are_pending
                ));
                hold_pid1();
            }
            if let Err(error) = profile_owner.end() {
                super::super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_DATA_OWNER_FAIL phase=end owner=profile_notes reason={error}"
                ));
                hold_pid1();
            }
            if let Err(error) = owner.end() {
                super::super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_DATA_OWNER_FAIL phase=end owner=data_mount reason={:?}",
                    error
                ));
                hold_pid1();
            }
            if let Err(error) = owner.verify_ended() {
                super::super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_DATA_OWNER_FAIL phase=end owner=mount_verification reason={:?}",
                    error
                ));
                hold_pid1();
            }
            super::super::log_pid1_stderr(format_args!(
                "NAGI_DEV_DATA_OWNER_PASS phase={} serial={} uuid={} mount_id={} retained_fd=true same_volume_notes_input=true notes_pending=true owner_ended=true",
                smoke_mode_name(request.mode),
                owner.device_serial,
                request.filesystem_uuid,
                owner.mount_id
            ));
            hold_pid1();
        }
    }
}

const fn smoke_mode_name(mode: SmokeMode) -> &'static str {
    match mode {
        SmokeMode::Write => "write",
        SmokeMode::Read => "read",
        SmokeMode::Reject => "reject",
        #[cfg(linux_dev_preview_smoke)]
        SmokeMode::ProvisionIdentityDirectories => "provision-identity-directories",
    }
}

#[cfg(test)]
mod selected_mount_tests {
    use super::{
        block_device_names, fd_source_path, remember_device_number, secure_mounted_data_root,
        DataMountError,
    };
    use std::fs::{self, File, OpenOptions};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestFile(PathBuf);

    impl TestFile {
        fn create() -> (Self, File) {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "nagi-selected-data-root-{}-{nonce}",
                std::process::id()
            ));
            let file = OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .mode(0o600)
                .open(&path)
                .expect("create private test root");
            (Self(path), file)
        }
    }

    impl Drop for TestFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn create() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("test clock")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "nagi-data-block-sysfs-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("create private test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn mismatched_mount_does_not_change_permissions() {
        let (_path, root) = TestFile::create();
        let selected_device = root.metadata().expect("test file metadata").dev();

        assert_eq!(
            secure_mounted_data_root(&root, selected_device.wrapping_add(1), || Ok(())),
            Err(DataMountError::DeviceChanged)
        );
        assert_eq!(root.metadata().unwrap().mode() & 0o777, 0o600);

        assert_eq!(
            secure_mounted_data_root(&root, selected_device, || {
                Err(DataMountError::DeviceChanged)
            }),
            Err(DataMountError::DeviceChanged)
        );
        assert_eq!(root.metadata().unwrap().mode() & 0o777, 0o600);

        assert_eq!(
            secure_mounted_data_root(&root, selected_device, || Ok(())),
            Ok(())
        );
        assert_eq!(root.metadata().unwrap().mode() & 0o777, 0o700);
    }

    #[test]
    fn data_sysfs_enumeration_includes_partition_names() {
        let root = TestDirectory::create();
        for name in ["vda", "vda1", "nvme0n1", "nvme0n1p1"] {
            fs::create_dir(root.0.join(name)).expect("create sysfs block entry");
        }

        let names = block_device_names(&root.0).expect("enumerate sysfs block entries");

        assert!(names.iter().any(|name| name == "vda1"));
        assert!(names.iter().any(|name| name == "nvme0n1p1"));
        assert!(names.iter().any(|name| name == "vda"));
    }

    #[test]
    fn sysfs_aliases_are_deduplicated_by_device_number() {
        let mut seen = Vec::new();

        assert!(remember_device_number(&mut seen, 0x0801));
        assert!(!remember_device_number(&mut seen, 0x0801));
        assert!(remember_device_number(&mut seen, 0x0802));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn mount_source_path_stays_pinned_after_device_node_replacement() {
        let root = TestDirectory::create();
        let device_path = root.0.join("selected-device");
        let replacement_path = root.0.join("replacement-device");
        fs::write(&device_path, b"validated-device").expect("create selected device fixture");
        fs::write(&replacement_path, b"replacement-device")
            .expect("create replacement device fixture");
        let opened = File::open(&device_path).expect("open selected device fixture");
        let source_path = fd_source_path(opened.as_raw_fd()).expect("derive fd-backed source path");

        fs::rename(&replacement_path, &device_path).expect("replace device path");
        let mut through_fd = File::open(source_path.to_str().expect("fd path is UTF-8"))
            .expect("open pinned fd path");
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut through_fd, &mut bytes).expect("read pinned device");

        assert_eq!(bytes, b"validated-device");
        assert_ne!(
            through_fd.metadata().expect("pinned metadata").ino(),
            fs::metadata(&device_path)
                .expect("replacement metadata")
                .ino()
        );
    }
}
