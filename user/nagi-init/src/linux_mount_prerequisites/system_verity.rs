//! Explicit development-only verifier for the fixed System ext4 fixture.
//!
//! The kernel creates the read-only dm-verity mapping before PID 1 from the
//! exact table recorded in `/proc/cmdline`. This module checks that mapping,
//! reads the fixed ext4 superblock through `/dev/dm-0`, then mounts that same
//! verified device read-only before the PID 1 lifecycle can start.

use super::{
    DataMountError, MOUNT_FLAGS, SelectedBlockDevice, ext4_mount_flags, linux_device_major,
    linux_device_minor, mount_id, open_directory, parse_major_minor,
};
use crate::task24_system_mount_identity as system_identity;
use std::ffi::{CStr, CString};
use std::fs::{self, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileExt, FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::Path;

const ROOT_HASH: &str = env!("NAGI_DEV_SYSTEM_VERITY_ROOT_HASH");
const DATA_BLOCKS: &str = env!("NAGI_DEV_SYSTEM_VERITY_DATA_BLOCKS");
const SALT: &str = env!("NAGI_DEV_SYSTEM_VERITY_SALT");
const SYSTEM_DM_NAME: &str = "nagi-system-verity";
const SYSTEM_DM_UUID: &str = "4e616769-5373-7973-7465-6d2d76303100";
const DEVICE_PATH: &str = "/dev/dm-0";
const SYSFS_DEVICE: &str = "/sys/block/dm-0";
const TARGET_PATH: &CStr = c"/run/nagi/dev-system";
const SUPERBLOCK_OFFSET: u64 = 1024;
const SUPERBLOCK_BYTES: usize = 1024;
const EXT4_MAGIC_OFFSET: usize = 56;
const FIXTURE_MARKER: &[u8] = b"Nagi Linux development fixture: system\n";
const O_CLOEXEC: i32 = 0o2000000;
const O_NOFOLLOW: i32 = 0o400000;
const EIO: i32 = 5;
const EINVAL: i32 = 22;
const ENODEV: i32 = 19;

pub(in crate::linux_init) enum ProbeOutcome {
    Accepted { root_digest: String, mount_id: u64 },
    Rejected { stage: &'static str, errno: i32 },
}

#[derive(Clone, Copy)]
struct VerityFailure {
    stage: &'static str,
    errno: i32,
}

impl VerityFailure {
    const fn new(stage: &'static str, errno: i32) -> Self {
        Self { stage, errno }
    }

    const fn from_data_mount(stage: &'static str, _error: DataMountError) -> Self {
        Self::new(stage, EIO)
    }
}

pub(in crate::linux_init) fn run_if_requested() -> ProbeOutcome {
    match verify_system() {
        Ok(mount_id) => ProbeOutcome::Accepted {
            root_digest: ROOT_HASH.to_owned(),
            mount_id,
        },
        Err(failure) => ProbeOutcome::Rejected {
            stage: failure.stage,
            errno: failure.errno,
        },
    }
}

fn verify_system() -> Result<u64, VerityFailure> {
    verify_boot_table()?;
    let device = verified_dm_device()?;
    let superblock = read_critical_superblock(&device)?;
    if !system_identity::clean_fixed_system(&superblock) {
        return Err(VerityFailure::new("system_identity", EINVAL));
    }
    let root_mount = open_directory(c"/")
        .map_err(|error| VerityFailure::from_data_mount("system_mountpoint", error))?;
    let target = open_directory(TARGET_PATH)
        .map_err(|error| VerityFailure::from_data_mount("system_mountpoint", error))?;
    let root_mount_id = mount_id(root_mount.as_raw_fd())
        .map_err(|error| VerityFailure::from_data_mount("system_mountpoint", error))?;
    let target_metadata = target
        .metadata()
        .map_err(|_| VerityFailure::new("system_mountpoint", EIO))?;
    if mount_id(target.as_raw_fd())
        .map_err(|error| VerityFailure::from_data_mount("system_mountpoint", error))?
        != root_mount_id
        || target_metadata.uid() != 0
        || target_metadata.mode() & 0o022 != 0
        || fs::read_dir(TARGET_PATH.to_str().expect("fixed target is UTF-8"))
            .map_err(|_| VerityFailure::new("system_mountpoint", EIO))?
            .next()
            .is_some()
    {
        return Err(VerityFailure::new("system_mountpoint", EINVAL));
    }
    if device
        .mount_ext4(
            TARGET_PATH,
            MOUNT_FLAGS | super::super::syscalls::MS_RDONLY,
            Some(c"noload"),
        )
        .is_err()
    {
        return Err(VerityFailure::new("system_mount", last_errno()));
    }
    let root = open_directory(TARGET_PATH)
        .map_err(|error| VerityFailure::from_data_mount("system_mount_verify", error))?;
    let mounted_id = mount_id(root.as_raw_fd())
        .map_err(|error| VerityFailure::from_data_mount("system_mount_id", error))?;
    let flags = ext4_mount_flags(root.as_raw_fd())
        .map_err(|error| VerityFailure::from_data_mount("system_mount_flags", error))?;
    if mounted_id == 0
        || mounted_id == root_mount_id
        || flags & super::super::mount_flags::ST_RDONLY == 0
        || flags
            & (super::super::mount_flags::ST_NOSUID
                | super::super::mount_flags::ST_NODEV
                | super::super::mount_flags::ST_NOEXEC)
            != (super::super::mount_flags::ST_NOSUID
                | super::super::mount_flags::ST_NODEV
                | super::super::mount_flags::ST_NOEXEC)
        || root
            .metadata()
            .map_err(|_| VerityFailure::new("system_device", EIO))?
            .dev()
            != device.device_number
    {
        return Err(VerityFailure::new("system_mount_flags", EINVAL));
    }
    let marker_path = Path::new("/run/nagi/dev-system/fixture.txt");
    let mut marker = OpenOptions::new()
        .read(true)
        .custom_flags(O_CLOEXEC | O_NOFOLLOW)
        .open(marker_path)
        .map_err(|error| {
            VerityFailure::new("system_fixture", error.raw_os_error().unwrap_or(EIO))
        })?;
    let metadata = marker.metadata().map_err(|error| {
        VerityFailure::new("system_fixture", error.raw_os_error().unwrap_or(EIO))
    })?;
    if !metadata.is_file() || metadata.len() != FIXTURE_MARKER.len() as u64 {
        return Err(VerityFailure::new("system_fixture", EINVAL));
    }
    let mut observed = Vec::with_capacity(FIXTURE_MARKER.len());
    marker.read_to_end(&mut observed).map_err(|error| {
        VerityFailure::new("system_fixture", error.raw_os_error().unwrap_or(EIO))
    })?;
    if observed != FIXTURE_MARKER {
        return Err(VerityFailure::new("system_fixture", EINVAL));
    }
    Ok(mounted_id)
}

fn verify_boot_table() -> Result<(), VerityFailure> {
    let command_line = fs::read_to_string("/proc/cmdline")
        .map_err(|error| VerityFailure::new("cmdline", error.raw_os_error().unwrap_or(EIO)))?;
    let words =
        parse_kernel_cmdline(&command_line).map_err(|_| VerityFailure::new("cmdline", EINVAL))?;
    let selectors: Vec<&str> = words
        .iter()
        .filter(|word| word.starts_with("nagi.dev_system_verity"))
        .map(String::as_str)
        .collect();
    if selectors.as_slice() != ["nagi.dev_system_verity=probe"] {
        return Err(VerityFailure::new("selector", EINVAL));
    }
    let tables: Vec<&str> = words
        .iter()
        .filter_map(|word| word.strip_prefix("dm-mod.create="))
        .collect();
    let waitfor: Vec<&str> = words
        .iter()
        .filter_map(|word| word.strip_prefix("dm-mod.waitfor="))
        .collect();
    let expected_table = expected_dm_table();
    if tables.as_slice() != [expected_table.as_str()] || waitfor.as_slice() != ["/dev/vda"] {
        return Err(VerityFailure::new("dm_table", EINVAL));
    }
    Ok(())
}

fn expected_dm_table() -> String {
    let data_blocks = DATA_BLOCKS.parse::<u64>().expect("validated by build.rs");
    let sectors = data_blocks * 8;
    format!(
        "{SYSTEM_DM_NAME},{SYSTEM_DM_UUID},,ro,0 {sectors} verity 1 /dev/vda /dev/vda 4096 4096 {data_blocks} {data_blocks} sha256 {ROOT_HASH} {SALT}"
    )
}

fn parse_kernel_cmdline(raw: &str) -> Result<Vec<String>, ()> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    for character in raw.chars() {
        match character {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' => return Err(()),
            ch if ch.is_whitespace() && !quoted => {
                if started {
                    words.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            ch => {
                current.push(ch);
                started = true;
            }
        }
    }
    if quoted {
        return Err(());
    }
    if started {
        words.push(current);
    }
    Ok(words)
}

fn verified_dm_device() -> Result<SelectedBlockDevice, VerityFailure> {
    let name = fs::read_to_string(format!("{SYSFS_DEVICE}/dm/name"))
        .map_err(|error| VerityFailure::new("dm_device", error.raw_os_error().unwrap_or(ENODEV)))?;
    let uuid = fs::read_to_string(format!("{SYSFS_DEVICE}/dm/uuid"))
        .map_err(|error| VerityFailure::new("dm_device", error.raw_os_error().unwrap_or(ENODEV)))?;
    let readonly = fs::read_to_string(format!("{SYSFS_DEVICE}/ro"))
        .map_err(|error| VerityFailure::new("dm_device", error.raw_os_error().unwrap_or(ENODEV)))?;
    if name.trim() != SYSTEM_DM_NAME || uuid.trim() != SYSTEM_DM_UUID || readonly.trim() != "1" {
        return Err(VerityFailure::new("dm_identity", EINVAL));
    }
    let device_path = Path::new(DEVICE_PATH);
    let node = fs::symlink_metadata(device_path)
        .map_err(|error| VerityFailure::new("dm_device", error.raw_os_error().unwrap_or(ENODEV)))?;
    if node.file_type().is_symlink() || !node.file_type().is_block_device() {
        return Err(VerityFailure::new("dm_device", EINVAL));
    }
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(O_CLOEXEC | O_NOFOLLOW)
        .open(device_path)
        .map_err(|error| VerityFailure::new("dm_device", error.raw_os_error().unwrap_or(ENODEV)))?;
    let metadata = opened
        .metadata()
        .map_err(|error| VerityFailure::new("dm_device", error.raw_os_error().unwrap_or(ENODEV)))?;
    if !metadata.file_type().is_block_device() {
        return Err(VerityFailure::new("dm_device", EINVAL));
    }
    let sysfs_device = fs::read_to_string(format!("{SYSFS_DEVICE}/dev"))
        .map_err(|error| VerityFailure::new("dm_device", error.raw_os_error().unwrap_or(ENODEV)))?;
    let (major, minor) = parse_major_minor(sysfs_device.trim())
        .map_err(|error| VerityFailure::from_data_mount("dm_device", error))?;
    let rdev = metadata.rdev();
    if linux_device_major(rdev) != major || linux_device_minor(rdev) != minor {
        return Err(VerityFailure::new("dm_device", EINVAL));
    }
    let path = CString::new(DEVICE_PATH).expect("fixed device path has no NUL");
    Ok(SelectedBlockDevice {
        name: "dm-0".to_owned(),
        path,
        device_number: rdev,
    })
}

fn read_critical_superblock(
    device: &SelectedBlockDevice,
) -> Result<[u8; SUPERBLOCK_BYTES], VerityFailure> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(O_CLOEXEC | O_NOFOLLOW)
        .open(device.path.to_string_lossy().as_ref())
        .map_err(|error| {
            VerityFailure::new("critical_block_open", error.raw_os_error().unwrap_or(EIO))
        })?;
    let metadata = file.metadata().map_err(|error| {
        VerityFailure::new("critical_block_open", error.raw_os_error().unwrap_or(EIO))
    })?;
    if !metadata.file_type().is_block_device() || metadata.rdev() != device.device_number {
        return Err(VerityFailure::new("critical_block_open", EINVAL));
    }
    let mut superblock = [0u8; SUPERBLOCK_BYTES];
    let mut read = 0;
    while read < superblock.len() {
        let count = file
            .read_at(&mut superblock[read..], SUPERBLOCK_OFFSET + read as u64)
            .map_err(|error| {
                VerityFailure::new("critical_block_read", error.raw_os_error().unwrap_or(EIO))
            })?;
        if count == 0 {
            return Err(VerityFailure::new("critical_block_read", EIO));
        }
        read += count;
    }
    if superblock[EXT4_MAGIC_OFFSET..EXT4_MAGIC_OFFSET + 2] != [0x53, 0xef] {
        return Err(VerityFailure::new("system_identity", EINVAL));
    }
    Ok(superblock)
}

fn last_errno() -> i32 {
    io::Error::last_os_error().raw_os_error().unwrap_or(EIO)
}
