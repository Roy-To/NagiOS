//! Development-only ext4 Data mount owner used by the dedicated Linux guest.
//!
//! The input is selected by both QEMU's virtio-blk serial and the ext4 UUID
//! provisioned on the host. This module never formats, repairs, or initializes
//! a device. It retains the mounted directory FD and does not authenticate a
//! Profile or issue a Verified Notes attachment.

use crate::profile_volume_notes_owner::{BootVolumeInput, DevelopmentBootVolumeIdentity};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::syscalls;

const EXT4_SUPER_MAGIC: u64 = 0xef53;
const EXT4_SUPERBLOCK_OFFSET: u64 = 1024;
const EXT4_SUPERBLOCK_BYTES: usize = 1024;
const EXT4_MAGIC_OFFSET: usize = 56;
const EXT4_UUID_OFFSET: usize = 104;
const EXPECTED_MOUNT_PATH: &str = "/run/nagi/dev-data";
const DEVICE_SYSFS_ROOT: &str = "/sys/block";
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
}

#[derive(Debug)]
struct SmokeRequest {
    mode: SmokeMode,
    device_serial: Option<String>,
    filesystem_uuid: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataMountError {
    SelectionMissing,
    SelectionMalformed,
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
    OwnerEndFailed,
}

impl DataMountError {
    const fn label(self) -> &'static str {
        match self {
            Self::SelectionMissing => "selection_missing",
            Self::SelectionMalformed => "selection_malformed",
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
            Self::OwnerEndFailed => "owner_end_failed",
        }
    }
}

struct SelectedBlockDevice {
    name: String,
    path: CString,
}

/// Owns one verified development ext4 mount and its retained root directory FD.
pub(super) struct DevelopmentDataMountOwner {
    root: Option<File>,
    identity: DevelopmentBootVolumeIdentity,
    root_mount_id: u64,
    mount_id: u64,
    mounted: bool,
}

impl DevelopmentDataMountOwner {
    fn mount(
        device_serial: Option<&str>,
        filesystem_uuid: Option<&str>,
    ) -> Result<Self, DataMountError> {
        let device_serial = device_serial.ok_or(DataMountError::SelectionMissing)?;
        let filesystem_uuid = filesystem_uuid.ok_or(DataMountError::SelectionMissing)?;
        if !valid_serial(device_serial) || !valid_uuid(filesystem_uuid) {
            return Err(DataMountError::SelectionMalformed);
        }

        let device = find_device_by_serial(device_serial)?;
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

        let mount_result = unsafe {
            syscalls::mount(
                device.path.as_ptr(),
                c"/run/nagi/dev-data".as_ptr(),
                c"ext4".as_ptr(),
                MOUNT_FLAGS,
                std::ptr::null(),
            )
        };
        if mount_result != 0 {
            return Err(DataMountError::MountRejected);
        }
        let mut rollback = MountRollback { active: true };

        fs::set_permissions(EXPECTED_MOUNT_PATH, fs::Permissions::from_mode(0o700))
            .map_err(|_| DataMountError::MountVerificationFailed)?;
        let root = open_directory(c"/run/nagi/dev-data")
            .map_err(|_| DataMountError::MountVerificationFailed)?;
        verify_ext4_mount(root.as_raw_fd())?;
        let mount_id = mount_id(root.as_raw_fd())?;
        if mount_id == root_mount_id {
            return Err(DataMountError::MountVerificationFailed);
        }

        // Recheck the exact selected device after the mount operation. The
        // development guest has no hotplug actor; this catches selection or
        // source changes without treating /dev name order as identity.
        if find_device_by_serial(device_serial)?.name != device.name
            || read_ext4_uuid(&device)? != filesystem_uuid
        {
            return Err(DataMountError::DeviceChanged);
        }

        rollback.active = false;
        Ok(Self {
            root: Some(root),
            identity: DevelopmentBootVolumeIdentity::new(
                device_serial.to_owned(),
                filesystem_uuid.to_owned(),
                mount_id,
            ),
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
        let mut content = Vec::new();
        file.read_to_end(&mut content)
            .map_err(|_| DataMountError::PersistenceMismatch)?;
        if content == MARKER_CONTENT {
            Ok(())
        } else {
            Err(DataMountError::PersistenceMismatch)
        }
    }

    fn open_marker(&self, write: bool) -> Result<File, DataMountError> {
        let root = self.root.as_ref().ok_or(DataMountError::OwnerEndFailed)?;
        let flags = if write {
            O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW
        } else {
            O_RDONLY | O_CLOEXEC | O_NOFOLLOW
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

/// Owns the retained boot input and releases it before ending the mount.
#[cfg(not(any(linux_dev_preview_smoke, nagi_release_build)))]
pub(in crate::linux_init) struct DevelopmentIdentityData {
    pub(in crate::linux_init) input: BootVolumeInput,
    pub(in crate::linux_init) accounts: PathBuf,
    pub(in crate::linux_init) snapshots: PathBuf,
    _mount: DevelopmentDataMountOwner,
}

/// Opt in to the same verified development mount while running normal PID 1.
#[cfg(not(any(linux_dev_preview_smoke, nagi_release_build)))]
pub(in crate::linux_init) fn mount_identity_data(
) -> Result<Option<DevelopmentIdentityData>, &'static str> {
    let command_line = fs::read_to_string("/proc/cmdline")
        .map_err(|_| DataMountError::SelectionMalformed.label())?;
    let selected = || {
        command_line
            .split_ascii_whitespace()
            .filter(|token| token.starts_with("nagi.dev_data."))
    };
    if !selected().any(|token| token.starts_with("nagi.dev_data.boot=")) {
        return Ok(None);
    }

    let mut enabled = false;
    let mut serial = None;
    let mut uuid = None;
    let mut accounts = None;
    let mut snapshots = None;
    for token in selected() {
        match token.split_once('=') {
            Some(("nagi.dev_data.boot", "identity")) if !enabled => enabled = true,
            Some(("nagi.dev_data.serial", value)) if serial.is_none() => serial = Some(value),
            Some(("nagi.dev_data.fs_uuid", value)) if uuid.is_none() => uuid = Some(value),
            Some(("nagi.dev_data.accounts", value)) if accounts.is_none() => accounts = Some(value),
            Some(("nagi.dev_data.snapshots", value)) if snapshots.is_none() => {
                snapshots = Some(value)
            }
            _ => return Err(DataMountError::SelectionMalformed.label()),
        }
    }
    let directories = |value: Option<&str>| {
        let path = value
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .ok_or(DataMountError::SelectionMissing.label())?;
        if !path
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
        {
            return Err(DataMountError::SelectionMalformed.label());
        }
        Ok(path)
    };
    let accounts = directories(accounts)?;
    let snapshots = directories(snapshots)?;
    let mount = DevelopmentDataMountOwner::mount(serial, uuid).map_err(DataMountError::label)?;
    let input = mount.boot_volume_input().map_err(DataMountError::label)?;
    Ok(Some(DevelopmentIdentityData {
        input,
        accounts,
        snapshots,
        _mount: mount,
    }))
}

impl Drop for DevelopmentDataMountOwner {
    fn drop(&mut self) {
        let _ = self.end();
    }
}

struct MountRollback {
    active: bool,
}

impl Drop for MountRollback {
    fn drop(&mut self) {
        if self.active {
            unsafe {
                let _ = syscalls::umount2(c"/run/nagi/dev-data".as_ptr(), UMOUNT_NOFOLLOW);
            }
        }
    }
}

fn parse_smoke_request() -> Result<Option<SmokeRequest>, DataMountError> {
    let command_line =
        fs::read_to_string("/proc/cmdline").map_err(|_| DataMountError::SelectionMalformed)?;
    let selected: Vec<&str> = command_line
        .split_ascii_whitespace()
        .filter(|token| token.starts_with("nagi.dev_data."))
        .collect();
    if selected.is_empty() {
        return Ok(None);
    }

    let mut mode = None;
    let mut device_serial = None;
    let mut filesystem_uuid = None;
    for token in selected {
        let (key, value) = token
            .split_once('=')
            .ok_or(DataMountError::SelectionMalformed)?;
        match key {
            "nagi.dev_data.smoke" if mode.is_none() => {
                mode = Some(match value {
                    "write" => SmokeMode::Write,
                    "read" => SmokeMode::Read,
                    "reject" => SmokeMode::Reject,
                    _ => return Err(DataMountError::SelectionMalformed),
                });
            }
            "nagi.dev_data.serial" if device_serial.is_none() => {
                device_serial = Some(value.to_owned());
            }
            "nagi.dev_data.fs_uuid" if filesystem_uuid.is_none() => {
                filesystem_uuid = Some(value.to_owned());
            }
            _ => return Err(DataMountError::SelectionMalformed),
        }
    }
    let mode = mode.ok_or(DataMountError::SelectionMissing)?;
    Ok(Some(SmokeRequest {
        mode,
        device_serial,
        filesystem_uuid,
    }))
}

fn find_device_by_serial(expected_serial: &str) -> Result<SelectedBlockDevice, DataMountError> {
    let entries = fs::read_dir(DEVICE_SYSFS_ROOT).map_err(|_| DataMountError::DeviceNotFound)?;
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
    let device = Path::new(DEVICE_NODE_ROOT).join(&name);
    let metadata = fs::symlink_metadata(&device).map_err(|_| DataMountError::DeviceNotFound)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_block_device() {
        return Err(DataMountError::DeviceChanged);
    }
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(O_CLOEXEC | O_NOFOLLOW)
        .open(&device)
        .map_err(|_| DataMountError::DeviceNotFound)?;
    if !opened
        .metadata()
        .map_err(|_| DataMountError::DeviceNotFound)?
        .file_type()
        .is_block_device()
    {
        return Err(DataMountError::DeviceChanged);
    }
    let sysfs_dev = fs::read_to_string(Path::new(DEVICE_SYSFS_ROOT).join(&name).join("dev"))
        .map_err(|_| DataMountError::DeviceNotFound)?;
    let (expected_major, expected_minor) = parse_major_minor(sysfs_dev.trim())?;
    let actual_dev = opened
        .metadata()
        .map_err(|_| DataMountError::DeviceNotFound)?
        .rdev();
    if linux_device_major(actual_dev) != expected_major
        || linux_device_minor(actual_dev) != expected_minor
    {
        return Err(DataMountError::DeviceChanged);
    }
    drop(opened);
    let path = CString::new(device.to_string_lossy().as_bytes())
        .map_err(|_| DataMountError::DeviceChanged)?;
    Ok(SelectedBlockDevice { name, path })
}

fn read_ext4_uuid(device: &SelectedBlockDevice) -> Result<String, DataMountError> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(O_CLOEXEC | O_NOFOLLOW)
        .open(PathBuf::from(device.path.to_string_lossy().as_ref()))
        .map_err(|_| DataMountError::DeviceNotFound)?;
    let mut superblock = [0u8; EXT4_SUPERBLOCK_BYTES];
    let mut offset = EXT4_SUPERBLOCK_OFFSET;
    let mut read = 0;
    while read < superblock.len() {
        use std::os::unix::fs::FileExt;
        let count = file
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
    Ok(format_uuid(
        &superblock[EXT4_UUID_OFFSET..EXT4_UUID_OFFSET + 16],
    ))
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
    let mut stat = MaybeUninit::<syscalls::StatFs>::zeroed();
    let result = unsafe { syscalls::fstatfs(fd, stat.as_mut_ptr()) };
    if result != 0 {
        return Err(DataMountError::MountVerificationFailed);
    }
    let stat = unsafe { stat.assume_init() };
    if stat.f_type as u64 != EXT4_SUPER_MAGIC || (stat.f_flags as u64 & 1) != 0 {
        return Err(DataMountError::MountVerificationFailed);
    }
    Ok(())
}

fn hold_pid1() -> ! {
    loop {
        std::thread::park_timeout(Duration::from_secs(60));
    }
}

/// Enter only when an explicit development-only kernel command-line mode is present.
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
        SmokeMode::Reject => {
            match DevelopmentDataMountOwner::mount(
                request.device_serial.as_deref(),
                request.filesystem_uuid.as_deref(),
            ) {
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
            let mut owner = match DevelopmentDataMountOwner::mount(
                request.device_serial.as_deref(),
                request.filesystem_uuid.as_deref(),
            ) {
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
                SmokeMode::Reject => unreachable!(),
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
                request.device_serial.as_deref().unwrap_or("<missing>"),
                request.filesystem_uuid.as_deref().unwrap_or("<missing>"),
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
    }
}
