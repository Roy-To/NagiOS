//! Minimal x86_64 Linux ABI declarations used by the mount prerequisite.
#![allow(dead_code)] // The same ABI module is compiled into the library and guest driver.

use std::os::raw::{c_char, c_int, c_long, c_uint, c_ulong, c_void};

pub const MS_RDONLY: c_ulong = 1;
pub const MS_NOSUID: c_ulong = 2;
pub const MS_NODEV: c_ulong = 4;
pub const MS_NOEXEC: c_ulong = 8;

pub const O_CLOEXEC: c_int = 0o2000000;
pub const O_DIRECTORY: c_int = 0o200000;
pub const O_NOFOLLOW: c_int = 0o400000;
pub const O_PATH: c_int = 0o10000000;

pub const AT_EMPTY_PATH: c_int = 0x1000;
pub const AT_STATX_SYNC_AS_STAT: c_int = 0;
pub const STATX_MNT_ID: c_uint = 0x1000;
pub const SYS_STATX: c_long = 332;

pub const EACCES: c_int = 13;
pub const EBUSY: c_int = 16;
pub const EINVAL: c_int = 22;
pub const EIO: c_int = 5;
pub const ENOTSUP: c_int = 95;
pub const EPERM: c_int = 1;

pub const PROC_SUPER_MAGIC: u64 = 0x0000_9fa0;
pub const SYSFS_MAGIC: u64 = 0x6265_6572;
pub const TMPFS_MAGIC: u64 = 0x0102_1994;

#[repr(C)]
pub struct StatFs {
    pub f_type: c_long,
    pub f_bsize: c_long,
    pub f_blocks: c_ulong,
    pub f_bfree: c_ulong,
    pub f_bavail: c_ulong,
    pub f_files: c_ulong,
    pub f_ffree: c_ulong,
    pub f_fsid: [c_int; 2],
    pub f_namelen: c_long,
    pub f_frsize: c_long,
    pub f_flags: c_long,
    pub f_spare: [c_long; 4],
}

#[repr(C)]
pub struct StatxTimestamp {
    pub tv_sec: i64,
    pub tv_nsec: u32,
    pub reserved: i32,
}

#[repr(C)]
pub struct Statx {
    pub stx_mask: u32,
    pub stx_blksize: u32,
    pub stx_attributes: u64,
    pub stx_nlink: u32,
    pub stx_uid: u32,
    pub stx_gid: u32,
    pub stx_mode: u16,
    pub spare0: u16,
    pub stx_ino: u64,
    pub stx_size: u64,
    pub stx_blocks: u64,
    pub stx_attributes_mask: u64,
    pub stx_atime: StatxTimestamp,
    pub stx_btime: StatxTimestamp,
    pub stx_ctime: StatxTimestamp,
    pub stx_mtime: StatxTimestamp,
    pub stx_rdev_major: u32,
    pub stx_rdev_minor: u32,
    pub stx_dev_major: u32,
    pub stx_dev_minor: u32,
    pub stx_mnt_id: u64,
    pub stx_dio_mem_align: u32,
    pub stx_dio_offset_align: u32,
    pub stx_subvol: u64,
    pub stx_atomic_write_unit_min: u32,
    pub stx_atomic_write_unit_max: u32,
    pub stx_atomic_write_segments_max: u32,
    pub spare1: u32,
    pub spare2: [u64; 9],
}

const _: [(); 120] = [(); std::mem::size_of::<StatFs>()];
const _: [(); 256] = [(); std::mem::size_of::<Statx>()];
const _: [(); 16] = [(); std::mem::offset_of!(StatFs, f_blocks)];
const _: [(); 144] = [(); std::mem::offset_of!(Statx, stx_mnt_id)];

extern "C" {
    pub fn mount(
        source: *const c_char,
        target: *const c_char,
        filesystemtype: *const c_char,
        mountflags: c_ulong,
        data: *const c_void,
    ) -> c_int;
    pub fn open(path: *const c_char, flags: c_int, ...) -> c_int;
    pub fn openat(dirfd: c_int, path: *const c_char, flags: c_int, ...) -> c_int;
    pub fn fstatfs(fd: c_int, buf: *mut StatFs) -> c_int;
    pub fn umount2(target: *const c_char, flags: c_int) -> c_int;
    pub fn syscall(number: c_long, ...) -> c_long;
    pub fn geteuid() -> u32;
    pub fn getegid() -> u32;
    pub fn setuid(uid: u32) -> c_int;
    pub fn setgid(gid: u32) -> c_int;
}

/// Call Linux `statx(2)` directly because musl's static libc does not export a
/// `statx` wrapper on every supported toolchain, while the kernel syscall is
/// available on the supported Linux target.
pub unsafe fn statx(
    dirfd: c_int,
    path: *const c_char,
    flags: c_int,
    mask: c_uint,
    stat: *mut Statx,
) -> c_int {
    syscall(SYS_STATX, dirfd, path, flags, mask, stat) as c_int
}
