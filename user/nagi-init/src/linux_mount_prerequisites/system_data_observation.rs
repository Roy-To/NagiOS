//! Observe two already-mounted development filesystems through borrowed root FDs.
//!
//! This probe never mounts, unmounts, closes, or takes ownership of either root.

use std::ffi::CString;
use std::fs;
use std::io::{Read, Write};
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd};
use std::os::raw::{c_char, c_int};

use crate::task24_system_data_probe_config::{parse_probe_request, ProbeRequest};

use super::super::{mount_flags, syscalls};

const O_RDONLY: c_int = 0;
const O_WRONLY: c_int = 1;
const O_CREAT: c_int = 0o100;
const O_EXCL: c_int = 0o200;
const O_CLOEXEC: c_int = 0o2000000;
const O_NOFOLLOW: c_int = 0o400000;
const MODE_0666: c_int = 0o666;
const EROFS: i32 = 30;
const SYSTEM_FIXTURE_MARKER: &[u8] = b"Nagi Linux development fixture: system\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct MountSnapshot {
    filesystem_magic: i64,
    flags: u64,
    mount_id: u64,
}

/// Read one opt-in probe token from PID 1's command line.
pub(super) fn requested_probe() -> Result<Option<ProbeRequest>, &'static str> {
    let command_line =
        fs::read_to_string("/proc/cmdline").map_err(|_| "system_data_probe_cmdline_unavailable")?;
    parse_probe_request(command_line.split_ascii_whitespace())
}

/// Verify existing System and Data roots and return one serial result record.
///
/// The borrowed descriptors remain owned by their mount owners. The only
/// filesystem mutation is a uniquely named Data probe file. If System accepts
/// the dedicated write attempt, this function removes that file before failing.
pub(super) fn observe_existing_roots(
    system_root: BorrowedFd<'_>,
    data_root: BorrowedFd<'_>,
    system_fstype: &str,
    data_fstype: &str,
    token: &str,
) -> Result<String, String> {
    if token.len() != 32 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("probe token must be 32 hexadecimal characters".into());
    }
    let system_magic = expected_filesystem_magic(system_fstype, true)?;
    let data_magic = expected_filesystem_magic(data_fstype, false)?;

    let system = mount_snapshot(system_root)?;
    let data = mount_snapshot(data_root)?;
    verify_disk_mount(system, system_magic, true, "system")?;
    verify_disk_mount(data, data_magic, false, "data")?;
    if system.mount_id == 0 || data.mount_id == 0 {
        return Err("System or Data mount ID is zero".into());
    }
    if system.mount_id == data.mount_id {
        return Err("System and Data resolved to the same mount ID".into());
    }

    verify_system_content_readable(system_root)?;
    let system_write_errno = verify_system_write_is_erofs(system_root, token)?;
    verify_data_write_sync_readback(data_root, token)?;

    Ok(format!(
        "SYSTEM_DATA_MOUNT_GUEST_PASS system_fs={system_fstype} system_magic=0x{:x} system_flags=0x{:x} system_readonly=true system_content_readable=true data_fs={data_fstype} data_magic=0x{:x} data_flags=0x{:x} data_readonly=false data_write_sync_readback=true distinct_mounts=true system_write_errno={system_write_errno} system_mount_id={} data_mount_id={}",
        system.filesystem_magic as u64,
        system.flags,
        data.filesystem_magic as u64,
        data.flags,
        system.mount_id,
        data.mount_id,
    ))
}

fn mount_snapshot(root: BorrowedFd<'_>) -> Result<MountSnapshot, String> {
    let mut filesystem = MaybeUninit::<syscalls::StatFs>::zeroed();
    if unsafe { syscalls::fstatfs(root.as_raw_fd(), filesystem.as_mut_ptr()) } != 0 {
        return Err(format!(
            "fstatfs for borrowed mount root failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let filesystem = unsafe { filesystem.assume_init() };

    let mut stat = MaybeUninit::<syscalls::Statx>::zeroed();
    if unsafe {
        syscalls::statx(
            root.as_raw_fd(),
            c"".as_ptr(),
            syscalls::AT_EMPTY_PATH | syscalls::AT_STATX_SYNC_AS_STAT,
            syscalls::STATX_MNT_ID,
            stat.as_mut_ptr(),
        )
    } != 0
    {
        return Err(format!(
            "statx for borrowed mount root failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let stat = unsafe { stat.assume_init() };
    if stat.stx_mask & syscalls::STATX_MNT_ID == 0 {
        return Err("statx did not report a mount ID for borrowed root".into());
    }

    Ok(MountSnapshot {
        filesystem_magic: filesystem.f_type,
        flags: filesystem.f_flags as u64,
        mount_id: stat.stx_mnt_id,
    })
}

fn expected_filesystem_magic(filesystem: &str, system: bool) -> Result<u64, String> {
    let magic = match filesystem {
        "ext4" => 0x0000_ef53,
        "erofs" if system => 0xe0f5_e1e2,
        "btrfs" if !system => 0x9123_683e,
        _ if system => return Err("System filesystem must be ext4 or erofs".into()),
        _ => return Err("Data filesystem must be ext4 or btrfs".into()),
    };
    Ok(magic)
}

fn verify_disk_mount(
    snapshot: MountSnapshot,
    expected_magic: u64,
    readonly: bool,
    label: &str,
) -> Result<(), String> {
    if snapshot.filesystem_magic as u64 != expected_magic {
        return Err(format!(
            "{label} filesystem magic mismatch: expected=0x{expected_magic:x} actual=0x{:x}",
            snapshot.filesystem_magic as u64
        ));
    }
    if snapshot.flags & mount_flags::ST_VALID == 0 {
        return Err(format!("{label} filesystem flags are not marked valid"));
    }
    let actual_readonly = snapshot.flags & mount_flags::ST_RDONLY != 0;
    if actual_readonly != readonly {
        return Err(format!(
            "{label} read-only state mismatch: expected={readonly} actual={actual_readonly} flags=0x{:x}",
            snapshot.flags
        ));
    }
    Ok(())
}

fn verify_system_content_readable(system_root: BorrowedFd<'_>) -> Result<(), String> {
    let name = CString::new("fixture.txt").expect("fixed System marker path has no NUL bytes");
    let mut marker_file = openat_file(system_root, &name, O_RDONLY | O_CLOEXEC | O_NOFOLLOW)?;
    let metadata = marker_file
        .metadata()
        .map_err(|error| format!("cannot stat System image marker: {error}"))?;
    if !metadata.is_file() || metadata.len() != SYSTEM_FIXTURE_MARKER.len() as u64 {
        return Err("System image marker has the wrong type or length".into());
    }
    let mut observed = Vec::with_capacity(SYSTEM_FIXTURE_MARKER.len());
    marker_file
        .read_to_end(&mut observed)
        .map_err(|error| format!("cannot read System image marker: {error}"))?;
    if observed != SYSTEM_FIXTURE_MARKER {
        return Err("System image marker content differs".into());
    }
    Ok(())
}

fn verify_system_write_is_erofs(system_root: BorrowedFd<'_>, token: &str) -> Result<i32, String> {
    let name = CString::new(format!(".nagi-readonly-probe-{token}"))
        .expect("hex token contains no NUL bytes");
    let raw_fd = unsafe {
        syscalls::openat(
            system_root.as_raw_fd(),
            name.as_ptr(),
            O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW,
            MODE_0666,
        )
    };
    if raw_fd >= 0 {
        drop(unsafe { std::fs::File::from_raw_fd(raw_fd) });
        if unsafe { unlinkat(system_root.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            return Err(format!(
                "System accepted the write probe and cleanup failed: {}",
                std::io::Error::last_os_error()
            ));
        }
        return Err("System image accepted a write despite the read-only mount".into());
    }

    let error = std::io::Error::last_os_error();
    match error.raw_os_error() {
        Some(EROFS) => Ok(EROFS),
        errno => Err(format!(
            "System write probe did not fail with EROFS: errno={errno:?} error={error}"
        )),
    }
}

fn verify_data_write_sync_readback(data_root: BorrowedFd<'_>, token: &str) -> Result<(), String> {
    let name =
        CString::new(format!(".nagi-rw-probe-{token}")).expect("hex token contains no NUL bytes");
    let expected = format!("NAGI-DATA-RW-{token}\n");
    let raw_fd = unsafe {
        syscalls::openat(
            data_root.as_raw_fd(),
            name.as_ptr(),
            O_WRONLY | O_CREAT | O_EXCL | O_CLOEXEC | O_NOFOLLOW,
            MODE_0666,
        )
    };
    if raw_fd < 0 {
        return Err(format!(
            "cannot create Data image probe: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut output = unsafe { std::fs::File::from_raw_fd(raw_fd) };
    output
        .write_all(expected.as_bytes())
        .map_err(|error| format!("cannot write Data image probe: {error}"))?;
    output
        .sync_all()
        .map_err(|error| format!("cannot sync Data image probe: {error}"))?;
    drop(output);

    if unsafe { fsync(data_root.as_raw_fd()) } != 0 {
        return Err(format!(
            "cannot sync Data image directory: {}",
            std::io::Error::last_os_error()
        ));
    }

    let read_fd = openat_file(data_root, &name, O_RDONLY | O_CLOEXEC | O_NOFOLLOW)?;
    let mut observed = Vec::new();
    read_fd
        .take(4096)
        .read_to_end(&mut observed)
        .map_err(|error| format!("cannot read back Data image probe: {error}"))?;
    if observed != expected.as_bytes() {
        return Err("Data image probe differs after write, sync, and readback".into());
    }
    Ok(())
}

fn openat_file(
    root: BorrowedFd<'_>,
    name: &CString,
    flags: c_int,
) -> Result<std::fs::File, String> {
    let raw_fd = unsafe { syscalls::openat(root.as_raw_fd(), name.as_ptr(), flags) };
    if raw_fd < 0 {
        return Err(format!(
            "cannot open image probe file for readback: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(unsafe { std::fs::File::from_raw_fd(raw_fd) })
}

unsafe extern "C" {
    fn fsync(fd: c_int) -> c_int;
    fn unlinkat(dirfd: c_int, path: *const c_char, flags: c_int) -> c_int;
}
