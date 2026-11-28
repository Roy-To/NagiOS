//! Linux release PID 1 owns the System mapping after authenticating its table.

use nagi_system_service_launch::{
    BootCatalogError, VerifiedSystemBootMetadata, MAX_SIGNED_SYSTEM_METADATA_BYTES,
};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::ptr;

const SIGNED_METADATA: &str = "/nagi-system-boot-metadata.signed";
const SYSTEM_DISK: &str = "/dev/vda";
const MOUNT_PATH: &str = "/system";
const DEVICE_MAPPER_CONTROL: &str = "/dev/mapper/control";
const DEVICE_MAPPER_NAME: &[u8] = b"nagi-release-system";
const PRODUCT_KEY: &[u8; 32] = include_bytes!("../../../../release/linux-0.1/public/ed25519.raw");

const DM_DEV_CREATE: u32 = 3;
const DM_DEV_REMOVE: u32 = 4;
const DM_DEV_SUSPEND: u32 = 6;
const DM_TABLE_LOAD: u32 = 9;
const DM_READONLY_FLAG: u32 = 1;
const DM_SUSPEND_FLAG: u32 = 1 << 1;
const DM_ACTIVE_PRESENT_FLAG: u32 = 1 << 5;
const BLKROGET: libc::Ioctl = 0x125e;
const BLKGETSIZE64: libc::Ioctl = 0x8008_1272u32 as libc::Ioctl;
const MOUNT_FLAGS: libc::c_ulong =
    libc::MS_RDONLY | libc::MS_NOSUID | libc::MS_NODEV | libc::MS_NOEXEC;

// Linux x86_64 UAPI, include/uapi/linux/dm-ioctl.h.
#[repr(C)]
struct DmIoctl {
    version: [u32; 3],
    data_size: u32,
    data_start: u32,
    target_count: u32,
    open_count: i32,
    flags: u32,
    event_nr: u32,
    padding: u32,
    dev: u64,
    name: [u8; 128],
    uuid: [u8; 129],
    data: [u8; 7],
}

impl DmIoctl {
    fn new() -> Self {
        let mut name = [0; 128];
        name[..DEVICE_MAPPER_NAME.len()].copy_from_slice(DEVICE_MAPPER_NAME);
        Self {
            version: [4, 0, 0],
            data_size: size_of::<Self>() as u32,
            data_start: size_of::<Self>() as u32,
            target_count: 0,
            open_count: 0,
            flags: 0,
            event_nr: 0,
            padding: 0,
            dev: 0,
            name,
            uuid: [0; 129],
            data: [0; 7],
        }
    }
}

#[repr(C)]
struct DmTarget {
    sector_start: u64,
    length: u64,
    status: i32,
    next: u32,
    target_type: [u8; 16],
}

const _: () = assert!(size_of::<DmIoctl>() == 312);
const _: () = assert!(size_of::<DmTarget>() == 40);

#[repr(C)]
struct DmTable {
    header: DmIoctl,
    target: DmTarget,
    parameters: [u8; 256],
}

fn dm_ioctl<T>(file: &File, command: u32, request: &mut T) -> io::Result<()> {
    let ioctl = (0xc000_0000 | ((size_of::<DmIoctl>() as u32) << 16) | (0xfd << 8) | command)
        as libc::Ioctl;
    // SAFETY: the request is a writable, aligned UAPI header or a header followed
    // by the entire table buffer. The caller sets data_size to that type's size.
    if unsafe { libc::ioctl(file.as_raw_fd(), ioctl, ptr::from_mut(request)) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

struct Mapping {
    control: File,
    dev: u64,
}

impl Mapping {
    fn activate(
        metadata: &VerifiedSystemBootMetadata,
        disk_number: u64,
    ) -> Result<Self, &'static str> {
        let control = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(DEVICE_MAPPER_CONTROL)
            .map_err(|_| "device_mapper_control_unavailable")?;
        if !control
            .metadata()
            .is_ok_and(|info| info.file_type().is_char_device())
        {
            return Err("device_mapper_control_unavailable");
        }
        let parameters =
            metadata.verity_parameters(libc::major(disk_number), libc::minor(disk_number));
        let mut create = DmIoctl::new();
        dm_ioctl(&control, DM_DEV_CREATE, &mut create)
            .map_err(|_| "device_mapper_create_failed")?;
        let mapping = Self {
            control,
            dev: create.dev,
        };

        let mut table = DmTable {
            header: mapping.request(),
            target: DmTarget {
                sector_start: 0,
                length: metadata.sectors(),
                status: 0,
                next: (size_of::<DmTarget>() + parameters.len() + 1).next_multiple_of(8) as u32,
                target_type: *b"verity\0\0\0\0\0\0\0\0\0\0",
            },
            parameters: [0; 256],
        };
        table.header.data_size = size_of::<DmTable>() as u32;
        table.header.flags = DM_READONLY_FLAG;
        table.header.target_count = 1;
        table
            .parameters
            .get_mut(..parameters.len() + 1)
            .ok_or("device_mapper_parameter_length")?[..parameters.len()]
            .copy_from_slice(parameters.as_bytes());
        dm_ioctl(&mapping.control, DM_TABLE_LOAD, &mut table)
            .map_err(|_| "device_mapper_table_rejected")?;

        let mut resume = mapping.request();
        dm_ioctl(&mapping.control, DM_DEV_SUSPEND, &mut resume)
            .map_err(|_| "device_mapper_resume_failed")?;
        let required = DM_READONLY_FLAG | DM_ACTIVE_PRESENT_FLAG;
        if resume.dev != mapping.dev
            || resume.target_count != 1
            || resume.flags & (required | DM_SUSPEND_FLAG) != required
        {
            return Err("device_mapper_status_mismatch");
        }
        Ok(mapping)
    }

    fn request(&self) -> DmIoctl {
        let mut request = DmIoctl::new();
        // Lookup by the device number returned by the create ioctl.
        request.name.fill(0);
        request.dev = self.dev;
        request
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        let _ = dm_ioctl(&self.control, DM_DEV_REMOVE, &mut self.request());
    }
}

struct MountGuard;

impl Drop for MountGuard {
    fn drop(&mut self) {
        // SAFETY: the fixed path is NUL-terminated and we own this mount.
        unsafe { libc::umount2(c"/system".as_ptr(), libc::UMOUNT_NOFOLLOW) };
    }
}

pub(in crate::linux_init) struct SystemMount {
    // This order closes the root before unmounting and removing the mapping.
    _root: File,
    _mount: MountGuard,
    _mapping: Mapping,
}

fn open_read(path: impl AsRef<Path>) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

fn read_metadata(path: &Path, key: &[u8; 32]) -> Result<VerifiedSystemBootMetadata, &'static str> {
    let file = open_read(path).map_err(|error| match error.kind() {
        io::ErrorKind::NotFound => "metadata_missing",
        _ => "metadata_unavailable",
    })?;
    if !file.metadata().is_ok_and(|info| info.is_file()) {
        return Err("metadata_unavailable");
    }
    let mut bytes = Vec::new();
    file.take((MAX_SIGNED_SYSTEM_METADATA_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "metadata_unavailable")?;
    VerifiedSystemBootMetadata::verify(&bytes, key).map_err(|error| match error {
        BootCatalogError::UntrustedSigner | BootCatalogError::InvalidSignature => {
            "metadata_signature_invalid"
        }
        _ => "metadata_format_invalid",
    })
}

fn mount_id(file: &File) -> Result<u64, &'static str> {
    let stat = rustix::fs::statx(
        file,
        c"",
        rustix::fs::AtFlags::EMPTY_PATH,
        rustix::fs::StatxFlags::MNT_ID,
    )
    .map_err(|_| "system_mount_id_unavailable")?;
    if stat.stx_mask & rustix::fs::StatxFlags::MNT_ID.bits() == 0 || stat.stx_mnt_id == 0 {
        return Err("system_mount_id_unavailable");
    }
    Ok(stat.stx_mnt_id)
}

fn system_disk(metadata: &VerifiedSystemBootMetadata) -> Result<File, &'static str> {
    let disk = open_read(SYSTEM_DISK).map_err(|_| "system_disk_unavailable")?;
    if !disk
        .metadata()
        .is_ok_and(|info| info.file_type().is_block_device())
    {
        return Err("system_disk_unavailable");
    }
    let mut read_only: libc::c_int = 0;
    let mut size = 0u64;
    // SAFETY: BLKROGET and BLKGETSIZE64 write a C int and a 64-bit size, respectively.
    if unsafe { libc::ioctl(disk.as_raw_fd(), BLKROGET, &mut read_only) } != 0
        || unsafe { libc::ioctl(disk.as_raw_fd(), BLKGETSIZE64, &mut size) } != 0
        || read_only != 1
        || size != metadata.image_size_bytes()
    {
        return Err("system_disk_geometry_or_readonly_mismatch");
    }
    Ok(disk)
}

pub(in crate::linux_init) fn verify_and_mount(
    catalog_key: &[u8; 32],
) -> Result<SystemMount, &'static str> {
    if catalog_key != PRODUCT_KEY {
        return Err("system_product_anchor_mismatch");
    }
    let metadata = read_metadata(Path::new(SIGNED_METADATA), catalog_key)?;

    let root = open_read("/").map_err(|_| "system_mountpoint_unavailable")?;
    let root_id = mount_id(&root)?;
    let target = open_read(MOUNT_PATH).map_err(|_| "system_mountpoint_unavailable")?;
    if !target
        .metadata()
        .is_ok_and(|info| info.is_dir() && info.uid() == 0 && info.mode() & 0o022 == 0)
        || mount_id(&target)? != root_id
        || fs::read_dir(MOUNT_PATH)
            .map_err(|_| "system_mountpoint_unavailable")?
            .next()
            .is_some()
    {
        return Err("system_mountpoint_not_empty_root_directory");
    }

    let disk = system_disk(&metadata)?;
    let disk_number = disk
        .metadata()
        .map_err(|_| "system_disk_unavailable")?
        .rdev();
    let mapping = Mapping::activate(&metadata, disk_number)?;
    let device = open_read(format!("/dev/dm-{}", libc::minor(mapping.dev)))
        .map_err(|_| "system_mapping_unavailable")?;
    if !device
        .metadata()
        .is_ok_and(|info| info.file_type().is_block_device() && info.rdev() == mapping.dev)
    {
        return Err("system_mapping_device_mismatch");
    }

    let source = CString::new(format!("/proc/self/fd/{}", device.as_raw_fd()))
        .expect("a numeric descriptor contains no NUL");
    // SAFETY: all strings are terminated, and device retains the verified block FD.
    if unsafe {
        libc::mount(
            source.as_ptr(),
            c"/system".as_ptr(),
            c"ext4".as_ptr(),
            MOUNT_FLAGS,
            c"noload".as_ptr().cast(),
        )
    } != 0
    {
        return Err("system_verity_mount_failed");
    }
    let guard = MountGuard;
    let mounted = open_read(MOUNT_PATH).map_err(|_| "system_mount_unavailable")?;
    let fs = rustix::fs::fstatfs(&mounted).map_err(|_| "system_mount_verification_failed")?;
    let required = super::mount_flags::ST_RDONLY
        | super::mount_flags::ST_NOSUID
        | super::mount_flags::ST_NODEV
        | super::mount_flags::ST_NOEXEC;
    if mount_id(&mounted)? == root_id
        || fs.f_type != 0xef53
        || super::mount_flags::verify(required, fs.f_flags as u64).is_err()
        || !mounted
            .metadata()
            .is_ok_and(|info| info.dev() == mapping.dev)
    {
        return Err("system_mount_verification_failed");
    }
    Ok(SystemMount {
        _root: mounted,
        _mount: guard,
        _mapping: mapping,
    })
}

pub(in crate::linux_init) fn report_failure(reason: &'static str) {
    super::super::log_pid1_stderr(format_args!("nagi-init: System boot stopped ({reason})"));
    // stderr may be attached to the serial console. Also display the same
    // diagnostic through the Linux boot console when a VT is available.
    if let Ok(mut console) = OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open("/dev/tty0")
    {
        let _ = writeln!(console, "nagi-init: System boot stopped ({reason})");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_nonfile_metadata_fails_before_mapping() {
        assert_eq!(
            read_metadata(
                Path::new("/proc/self/nagi-system-boot-metadata.signed"),
                PRODUCT_KEY
            )
            .err(),
            Some("metadata_missing")
        );
        assert_eq!(
            read_metadata(Path::new("/proc/self/fd"), PRODUCT_KEY).err(),
            Some("metadata_unavailable")
        );
    }

    #[test]
    fn device_mapper_layout_matches_x86_64_uapi() {
        assert_eq!(std::mem::offset_of!(DmIoctl, dev), 40);
        assert_eq!(std::mem::offset_of!(DmIoctl, name), 48);
        assert_eq!(std::mem::offset_of!(DmTable, target), 312);
        assert_eq!(std::mem::offset_of!(DmTable, parameters), 352);
    }
}
