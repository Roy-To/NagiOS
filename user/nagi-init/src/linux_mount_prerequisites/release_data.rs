//! Release PID 1 owns one Data ext4 mount selected by the opaque Data verifier.
//!
//! This module never parses selection bytes, trusts development preview keys,
//! formats a device, or changes the mounted Data filesystem. A caller must
//! keep services stopped if selection, candidate matching, mounting, or
//! post-mount verification fails.

use nagi_data_boot_selection::{SelectionMode, VerifiedDataBootSelection};
use std::ffi::{CStr, CString};
use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, RawFd};
use std::os::unix::fs::{FileExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path};

use super::{mount_flags, mount_validation};

const EXT4_SUPER_MAGIC: u64 = 0xef53;
const EXT4_SUPERBLOCK_OFFSET: u64 = 1024;
const EXT4_SUPERBLOCK_BYTES: usize = 1024;
const EXT4_MAGIC_OFFSET: usize = 56;
const EXT4_UUID_OFFSET: usize = 104;
const MOUNT_PATH: &str = "/run/nagi/data";
const DEVICE_SYSFS_ROOT: &str = "/sys/class/block";
const DEVICE_NODE_ROOT: &str = "/dev";
const DATA_MOUNT_FLAGS: libc::c_ulong = libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC;
const DATA_REQUIRED_STATFS_FLAGS: u64 =
    mount_flags::ST_NOSUID | mount_flags::ST_NODEV | mount_flags::ST_NOEXEC;
const O_CLOEXEC: libc::c_int = 0o2000000;
const O_NOFOLLOW: libc::c_int = 0o400000;
const O_DIRECTORY: libc::c_int = 0o200000;

/// Opaque Data root proof emitted only by this verified Mount owner.
pub(crate) struct VerifiedDataRoot {
    directory: File,
    identity: VerifiedDataRootIdentity,
}

struct VerifiedDataRootIdentity {
    filesystem_uuid: String,
    device_rdev: u64,
    device_major: u32,
    device_minor: u32,
    mount_id: u64,
    accounts_path: String,
    snapshots_path: String,
}

impl VerifiedDataRoot {
    fn from_verified_mount(
        directory: File,
        selection: &VerifiedDataBootSelection<'_>,
        device_rdev: u64,
        device_major: u32,
        device_minor: u32,
        mount_id: u64,
    ) -> io::Result<Self> {
        if selection.mode() != SelectionMode::Identity
            || !valid_uuid(selection.filesystem_uuid())
            || device_rdev == 0
            || mount_id == 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "release Data mount identity is incomplete",
            ));
        }
        let accounts_path = checked_selection_path(selection.accounts_path())?;
        let snapshots_path = checked_selection_path(selection.snapshots_path())?;
        let metadata = directory.metadata()?;
        if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "verified Data root is not an owner-controlled directory",
            ));
        }
        Ok(Self {
            directory,
            identity: VerifiedDataRootIdentity {
                filesystem_uuid: selection.filesystem_uuid().to_owned(),
                device_rdev,
                device_major,
                device_minor,
                mount_id,
                accounts_path,
                snapshots_path,
            },
        })
    }

    #[cfg(test)]
    pub(crate) fn test_fixture(directory: File) -> io::Result<Self> {
        let filesystem_uuid = "12345678-1234-4234-8234-123456789abc".to_owned();
        let device_rdev = 0x800;
        let device_major = 8;
        let device_minor = 0;
        let mount_id = 42;
        if !valid_uuid(&filesystem_uuid) || device_rdev == 0 || mount_id == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "release Data mount identity is incomplete",
            ));
        }
        let accounts_path = checked_selection_path("accounts")?;
        let snapshots_path = checked_selection_path("identity/snapshots")?;
        let metadata = directory.metadata()?;
        if !metadata.is_dir() || metadata.mode() & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "verified Data root is not an owner-controlled directory",
            ));
        }
        Ok(Self {
            directory,
            identity: VerifiedDataRootIdentity {
                filesystem_uuid,
                device_rdev,
                device_major,
                device_minor,
                mount_id,
                accounts_path,
                snapshots_path,
            },
        })
    }

    pub(crate) fn as_fd(&self) -> BorrowedFd<'_> {
        self.directory.as_fd()
    }

    pub(crate) fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            directory: self.directory.try_clone()?,
            identity: VerifiedDataRootIdentity {
                filesystem_uuid: self.identity.filesystem_uuid.clone(),
                device_rdev: self.identity.device_rdev,
                device_major: self.identity.device_major,
                device_minor: self.identity.device_minor,
                mount_id: self.identity.mount_id,
                accounts_path: self.identity.accounts_path.clone(),
                snapshots_path: self.identity.snapshots_path.clone(),
            },
        })
    }

    pub(crate) fn filesystem_uuid(&self) -> &str {
        &self.identity.filesystem_uuid
    }

    pub(crate) fn device_rdev(&self) -> u64 {
        self.identity.device_rdev
    }

    pub(crate) fn device_major_minor(&self) -> (u32, u32) {
        (self.identity.device_major, self.identity.device_minor)
    }

    pub(crate) fn mount_id(&self) -> u64 {
        self.identity.mount_id
    }

    pub(crate) fn accounts_path(&self) -> &Path {
        Path::new(&self.identity.accounts_path)
    }

    pub(crate) fn snapshots_path(&self) -> &Path {
        Path::new(&self.identity.snapshots_path)
    }

    pub(crate) fn into_parts(self) -> (File, String, u64, u32, u32, u64, String, String) {
        (
            self.directory,
            self.identity.filesystem_uuid,
            self.identity.device_rdev,
            self.identity.device_major,
            self.identity.device_minor,
            self.identity.mount_id,
            self.identity.accounts_path,
            self.identity.snapshots_path,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataMountFailure {
    SelectionPolicy,
    DeviceNotFound,
    DeviceAmbiguous,
    DeviceChanged,
    DeviceUnreadable,
    NotExt4,
    FilesystemUuidMismatch,
    MountpointUnavailable,
    AlreadyMounted,
    MountRejected,
    MountVerificationFailed,
}

impl DataMountFailure {
    const fn label(self) -> &'static str {
        match self {
            Self::SelectionPolicy => "selection_policy_rejected",
            Self::DeviceNotFound => "device_not_found",
            Self::DeviceAmbiguous => "device_ambiguous",
            Self::DeviceChanged => "device_changed",
            Self::DeviceUnreadable => "device_unreadable",
            Self::NotExt4 => "not_ext4",
            Self::FilesystemUuidMismatch => "filesystem_uuid_mismatch",
            Self::MountpointUnavailable => "mountpoint_unavailable",
            Self::AlreadyMounted => "mountpoint_already_mounted",
            Self::MountRejected => "mount_rejected",
            Self::MountVerificationFailed => "mount_verification_failed",
        }
    }
}

struct SelectedBlockDevice {
    file: File,
    device_number: u64,
    major: u32,
    minor: u32,
}

/// Retains the mount root until all Data-root consumers have dropped their
/// descriptors. Field order closes the owner's root before unmounting.
#[must_use = "retain this owner until all Data root FDs are closed and end() succeeds"]
pub(in crate::linux_init) struct ReleaseDataMountOwner {
    root: Option<File>,
    _device: File,
    parent_mount_id: u64,
    mount_id: u64,
    device_rdev: u64,
    device_major: u32,
    device_minor: u32,
    mount: MountGuard,
    ended: bool,
}

impl ReleaseDataMountOwner {
    fn mount(selection: &VerifiedDataBootSelection<'_>) -> Result<Self, DataMountFailure> {
        if selection.mode() != SelectionMode::Identity || !valid_uuid(selection.filesystem_uuid()) {
            return Err(DataMountFailure::SelectionPolicy);
        }

        let device = find_device_by_uuid(selection.filesystem_uuid())?;
        if read_ext4_uuid(&device)? != selection.filesystem_uuid() {
            return Err(DataMountFailure::FilesystemUuidMismatch);
        }

        ensure_mount_target()?;
        let parent_before = open_directory(c"/run")?;
        let target_before = open_directory(c"/run/nagi/data")?;
        let parent_mount_id = mount_id(&parent_before)?;
        if mount_id(&target_before)? != parent_mount_id {
            return Err(DataMountFailure::AlreadyMounted);
        }
        if fs::read_dir(MOUNT_PATH)
            .map_err(|_| DataMountFailure::MountpointUnavailable)?
            .next()
            .is_some()
        {
            return Err(DataMountFailure::MountpointUnavailable);
        }

        let source = fd_source_path(device.file.as_raw_fd())?;
        // No MS_RDONLY bit is set; the mounted filesystem is read/write.
        // SAFETY: all strings are NUL-terminated and the block FD remains open.
        if unsafe {
            libc::mount(
                source.as_ptr(),
                c"/run/nagi/data".as_ptr(),
                c"ext4".as_ptr(),
                DATA_MOUNT_FLAGS,
                c"rw".as_ptr().cast(),
            )
        } != 0
        {
            return Err(DataMountFailure::MountRejected);
        }
        let mount = MountGuard { active: true };

        let root = open_directory(c"/run/nagi/data")?;
        let mount_id = mount_id(&root)?;
        let statfs =
            rustix::fs::fstatfs(&root).map_err(|_| DataMountFailure::MountVerificationFailed)?;
        let mounted_metadata = root
            .metadata()
            .map_err(|_| DataMountFailure::MountVerificationFailed)?;
        let observation = mount_validation::MountIdentityObservation {
            parent_mount_id,
            data_mount_id: mount_id,
            selected_rdev: device.device_number,
            mounted_dev: mounted_metadata.dev(),
            filesystem_magic: statfs.f_type as u64,
        };
        mount_validation::verify_mount_identity(observation)
            .map_err(|_| DataMountFailure::MountVerificationFailed)?;
        mount_flags::verify_rw(DATA_REQUIRED_STATFS_FLAGS, statfs.f_flags as u64)
            .map_err(|_| DataMountFailure::MountVerificationFailed)?;
        if !mounted_metadata.is_dir()
            || mounted_metadata.uid() != 0
            || mounted_metadata.mode() & 0o022 != 0
            || find_device_by_uuid(selection.filesystem_uuid())?.device_number
                != device.device_number
            || read_ext4_uuid(&device)? != selection.filesystem_uuid()
        {
            return Err(DataMountFailure::MountVerificationFailed);
        }

        Ok(Self {
            root: Some(root),
            _device: device.file,
            parent_mount_id,
            mount_id,
            device_rdev: device.device_number,
            device_major: device.major,
            device_minor: device.minor,
            mount,
            ended: false,
        })
    }

    fn issue_verified_root(
        &self,
        selection: &VerifiedDataBootSelection<'_>,
    ) -> Result<VerifiedDataRoot, DataMountFailure> {
        let root = self
            .root
            .as_ref()
            .ok_or(DataMountFailure::MountVerificationFailed)?
            .try_clone()
            .map_err(|_| DataMountFailure::MountVerificationFailed)?;
        VerifiedDataRoot::from_verified_mount(
            root,
            selection,
            self.device_rdev,
            self.device_major,
            self.device_minor,
            self.mount_id,
        )
        .map_err(|_| DataMountFailure::MountVerificationFailed)
    }

    pub(in crate::linux_init) fn end(&mut self) -> Result<(), &'static str> {
        self.end_inner().map_err(DataMountFailure::label)
    }

    fn end_inner(&mut self) -> Result<(), DataMountFailure> {
        if self.ended {
            return Ok(());
        }
        // A child/root FD still held by A, C1, or AppStorageOwners makes
        // umount fail. Never use lazy or forced unmount to bypass that owner.
        drop(self.root.take());
        self.mount.unmount()?;
        self.verify_ended()?;
        self.ended = true;
        Ok(())
    }

    fn verify_ended(&self) -> Result<(), DataMountFailure> {
        let target = open_directory(c"/run/nagi/data")?;
        if mount_id(&target)? == self.parent_mount_id {
            Ok(())
        } else {
            Err(DataMountFailure::MountVerificationFailed)
        }
    }
}

impl Drop for ReleaseDataMountOwner {
    fn drop(&mut self) {
        if let Err(reason) = self.end() {
            super::super::log_pid1_stderr(format_args!(
                "nagi-init: Data mount cleanup failed ({reason})"
            ));
        }
    }
}

struct MountGuard {
    active: bool,
}

impl MountGuard {
    fn unmount(&mut self) -> Result<(), DataMountFailure> {
        if !self.active {
            return Ok(());
        }
        // SAFETY: the fixed target is NUL-terminated and this owner mounted it.
        if unsafe { libc::umount2(c"/run/nagi/data".as_ptr(), libc::UMOUNT_NOFOLLOW) } != 0 {
            return Err(DataMountFailure::MountVerificationFailed);
        }
        self.active = false;
        Ok(())
    }
}

impl Drop for MountGuard {
    fn drop(&mut self) {
        let _ = self.unmount();
    }
}

pub(in crate::linux_init) fn mount_verified_selection(
    selection: &VerifiedDataBootSelection<'_>,
) -> Result<(ReleaseDataMountOwner, VerifiedDataRoot), &'static str> {
    let owner = ReleaseDataMountOwner::mount(selection).map_err(DataMountFailure::label)?;
    let root = owner
        .issue_verified_root(selection)
        .map_err(DataMountFailure::label)?;
    Ok((owner, root))
}

fn find_device_by_uuid(expected_uuid: &str) -> Result<SelectedBlockDevice, DataMountFailure> {
    if !valid_uuid(expected_uuid) {
        return Err(DataMountFailure::SelectionPolicy);
    }
    let sysfs_root = Path::new(DEVICE_SYSFS_ROOT);
    let entries = fs::read_dir(sysfs_root).map_err(|_| DataMountFailure::DeviceNotFound)?;
    let mut matches = Vec::new();
    let mut seen_device_numbers = Vec::new();
    let mut saw_ext4_candidate = false;
    for entry in entries {
        let entry = entry.map_err(|_| DataMountFailure::DeviceNotFound)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| DataMountFailure::DeviceChanged)?;
        if !valid_block_device_name(&name) {
            continue;
        }
        let sysfs_path = sysfs_root.join(&name);
        let device_path = Path::new(DEVICE_NODE_ROOT).join(&name);
        match fs::symlink_metadata(&device_path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(DataMountFailure::DeviceNotFound)
            }
            Err(_) => return Err(DataMountFailure::DeviceNotFound),
            Ok(metadata)
                if metadata.file_type().is_symlink() || !metadata.file_type().is_block_device() =>
            {
                return Err(DataMountFailure::DeviceChanged)
            }
            Ok(_) => {}
        }
        let device = open_block_device(&name, &sysfs_path)?;
        if seen_device_numbers.contains(&device.device_number) {
            continue;
        }
        seen_device_numbers.push(device.device_number);
        match read_ext4_uuid(&device) {
            Ok(uuid) => {
                saw_ext4_candidate = true;
                if uuid == expected_uuid {
                    matches.push(device);
                }
            }
            Err(DataMountFailure::NotExt4) => {}
            Err(error) => return Err(error),
        }
    }
    mount_validation::require_single_uuid_match(matches.len(), saw_ext4_candidate).map_err(
        |error| match error {
            mount_validation::CandidateCountFailure::DeviceNotFound => {
                DataMountFailure::DeviceNotFound
            }
            mount_validation::CandidateCountFailure::FilesystemUuidMismatch => {
                DataMountFailure::FilesystemUuidMismatch
            }
            mount_validation::CandidateCountFailure::DeviceAmbiguous => {
                DataMountFailure::DeviceAmbiguous
            }
        },
    )?;
    matches.pop().ok_or(DataMountFailure::DeviceNotFound)
}

fn open_block_device(
    name: &str,
    sysfs_path: &Path,
) -> Result<SelectedBlockDevice, DataMountFailure> {
    let device_path = Path::new(DEVICE_NODE_ROOT).join(name);
    let before =
        fs::symlink_metadata(&device_path).map_err(|_| DataMountFailure::DeviceNotFound)?;
    if before.file_type().is_symlink() || !before.file_type().is_block_device() {
        return Err(DataMountFailure::DeviceChanged);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(O_CLOEXEC | O_NOFOLLOW)
        .open(&device_path)
        .map_err(|_| DataMountFailure::DeviceNotFound)?;
    let metadata = file
        .metadata()
        .map_err(|_| DataMountFailure::DeviceNotFound)?;
    if !metadata.file_type().is_block_device() || metadata.rdev() == 0 {
        return Err(DataMountFailure::DeviceChanged);
    }
    let dev =
        fs::read_to_string(sysfs_path.join("dev")).map_err(|_| DataMountFailure::DeviceNotFound)?;
    let (major, minor) = parse_major_minor(dev.trim())?;
    if linux_device_major(metadata.rdev()) != major || linux_device_minor(metadata.rdev()) != minor
    {
        return Err(DataMountFailure::DeviceChanged);
    }
    Ok(SelectedBlockDevice {
        file,
        device_number: metadata.rdev(),
        major,
        minor,
    })
}

fn read_ext4_uuid(device: &SelectedBlockDevice) -> Result<String, DataMountFailure> {
    let metadata = device
        .file
        .metadata()
        .map_err(|_| DataMountFailure::DeviceNotFound)?;
    if !metadata.file_type().is_block_device() || metadata.rdev() != device.device_number {
        return Err(DataMountFailure::DeviceChanged);
    }
    let mut superblock = [0u8; EXT4_SUPERBLOCK_BYTES];
    let mut offset = EXT4_SUPERBLOCK_OFFSET;
    let mut read = 0;
    while read < superblock.len() {
        let count = device
            .file
            .read_at(&mut superblock[read..], offset)
            .map_err(|_| DataMountFailure::DeviceUnreadable)?;
        if count == 0 {
            return Err(DataMountFailure::DeviceUnreadable);
        }
        read += count;
        offset += count as u64;
    }
    if superblock[EXT4_MAGIC_OFFSET..EXT4_MAGIC_OFFSET + 2] != [0x53, 0xef] {
        return Err(DataMountFailure::NotExt4);
    }
    Ok(format_uuid(
        &superblock[EXT4_UUID_OFFSET..EXT4_UUID_OFFSET + 16],
    ))
}

fn ensure_mount_target() -> Result<(), DataMountFailure> {
    validate_directory(Path::new("/run"), false)?;
    ensure_private_directory(Path::new("/run/nagi"))?;
    ensure_private_directory(Path::new(MOUNT_PATH))
}

fn validate_directory(path: &Path, private: bool) -> Result<(), DataMountFailure> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| DataMountFailure::MountpointUnavailable)?;
    let mode = metadata.mode() & 0o777;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_dir()
        || metadata.uid() != 0
        || (private && mode != 0o700)
        || (!private && mode & 0o022 != 0)
    {
        return Err(DataMountFailure::MountpointUnavailable);
    }
    Ok(())
}

fn ensure_private_directory(path: &Path) -> Result<(), DataMountFailure> {
    match fs::symlink_metadata(path) {
        Ok(_) => validate_directory(path, true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|_| DataMountFailure::MountpointUnavailable)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .map_err(|_| DataMountFailure::MountpointUnavailable)?;
            validate_directory(path, true)
        }
        Err(_) => Err(DataMountFailure::MountpointUnavailable),
    }
}

fn open_directory(path: &CStr) -> Result<File, DataMountFailure> {
    // SAFETY: path is NUL-terminated and flags only request a read-only
    // directory FD with close-on-exec and a final-component no-follow check.
    let fd = unsafe {
        libc::open(
            path.as_ptr(),
            libc::O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(DataMountFailure::MountpointUnavailable);
    }
    // SAFETY: successful open returns a new uniquely owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

fn mount_id(file: &File) -> Result<u64, DataMountFailure> {
    let stat = rustix::fs::statx(
        file,
        c"",
        rustix::fs::AtFlags::EMPTY_PATH,
        rustix::fs::StatxFlags::MNT_ID,
    )
    .map_err(|_| DataMountFailure::MountVerificationFailed)?;
    if stat.stx_mask & rustix::fs::StatxFlags::MNT_ID.bits() == 0 || stat.stx_mnt_id == 0 {
        return Err(DataMountFailure::MountVerificationFailed);
    }
    Ok(stat.stx_mnt_id)
}

fn fd_source_path(fd: RawFd) -> Result<CString, DataMountFailure> {
    CString::new(format!("/proc/self/fd/{fd}")).map_err(|_| DataMountFailure::DeviceChanged)
}

fn parse_major_minor(value: &str) -> Result<(u32, u32), DataMountFailure> {
    let (major, minor) = value
        .split_once(':')
        .ok_or(DataMountFailure::DeviceChanged)?;
    Ok((
        major.parse().map_err(|_| DataMountFailure::DeviceChanged)?,
        minor.parse().map_err(|_| DataMountFailure::DeviceChanged)?,
    ))
}

fn linux_device_major(device: u64) -> u32 {
    (((device >> 8) & 0x0fff) | ((device >> 32) & 0xffff_f000)) as u32
}

fn linux_device_minor(device: u64) -> u32 {
    ((device & 0x00ff) | ((device >> 12) & 0xffff_ff00)) as u32
}

fn valid_block_device_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'!' | b'_' | b'-'))
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte),
        })
}

fn checked_selection_path(value: &str) -> io::Result<String> {
    let path = Path::new(value);
    if value.is_empty()
        || value.contains('\0')
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "release Data selection path is not a canonical relative path",
        ));
    }
    Ok(value.to_owned())
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

pub(in crate::linux_init) fn report_failure(reason: &'static str) {
    super::super::log_pid1_stderr(format_args!("nagi-init: Data mount stopped ({reason})"));
}
