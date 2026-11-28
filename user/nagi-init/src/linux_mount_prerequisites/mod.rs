//! Linux procfs/sysfs mount prerequisites for early userland startup.
//!
//! This module does not mount or manage `/dev`; the production PID 1 path in
//! `user/nagi-init` already mounts devtmpfs. A caller must keep services
//! stopped if this function returns an error.

#![cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
compile_error!("nagi-linux-mount-prereqs currently supports x86_64 Linux only");

use std::fmt;
use std::mem::MaybeUninit;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    any(linux_dev_preview_smoke, not(nagi_release_build))
))]
#[path = "data_mount_owner.rs"]
pub(super) mod data_mount_owner;
#[cfg(any(nagi_release_build, test))]
#[cfg_attr(not(nagi_release_build), allow(dead_code))]
mod release_system;
#[cfg(nagi_release_build)]
pub(super) use release_system::{
    report_failure as report_system_failure, verify_and_mount as mount_release_system,
};
#[path = "mount_flags.rs"]
mod mount_flags;
#[path = "syscalls.rs"]
mod syscalls;

#[cfg(all(linux_dev_preview_smoke, target_os = "linux", target_arch = "x86_64"))]
pub(super) fn maybe_run_development_data_mount_smoke() -> bool {
    data_mount_owner::maybe_run_smoke()
}

const MOUNT_FLAGS: std::os::raw::c_ulong =
    syscalls::MS_NOSUID | syscalls::MS_NODEV | syscalls::MS_NOEXEC;
const SYSFS_MOUNT_FLAGS: std::os::raw::c_ulong = MOUNT_FLAGS | syscalls::MS_RDONLY;
const STATX_MNT_ID_MASK: std::os::raw::c_uint = syscalls::STATX_MNT_ID;

/// A fixed early-boot mount target. Paths are not caller-controlled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountPoint {
    Proc,
    Sys,
}

impl MountPoint {
    const fn path(self) -> &'static std::ffi::CStr {
        match self {
            Self::Proc => c"/proc",
            Self::Sys => c"/sys",
        }
    }

    const fn filesystem_name(self) -> &'static std::ffi::CStr {
        match self {
            Self::Proc => c"proc",
            Self::Sys => c"sysfs",
        }
    }

    const fn expected_filesystem(self) -> FilesystemType {
        match self {
            Self::Proc => FilesystemType::Procfs,
            Self::Sys => FilesystemType::Sysfs,
        }
    }

    const fn flags(self) -> std::os::raw::c_ulong {
        match self {
            Self::Proc => MOUNT_FLAGS,
            Self::Sys => SYSFS_MOUNT_FLAGS,
        }
    }

    const fn required_statfs_flags(self) -> u64 {
        match self {
            Self::Proc => mount_flags::ST_NOSUID | mount_flags::ST_NODEV | mount_flags::ST_NOEXEC,
            Self::Sys => {
                mount_flags::ST_RDONLY
                    | mount_flags::ST_NOSUID
                    | mount_flags::ST_NODEV
                    | mount_flags::ST_NOEXEC
            }
        }
    }
}

impl fmt::Display for MountPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.path().to_str().expect("fixed mount path is UTF-8"))
    }
}

/// The observed Linux filesystem type, based on `fstatfs(2)` magic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemType {
    Procfs,
    Sysfs,
    Tmpfs,
    Other(u64),
}

impl FilesystemType {
    fn from_magic(magic: std::os::raw::c_long) -> Self {
        match magic as u64 {
            value if value == syscalls::PROC_SUPER_MAGIC => Self::Procfs,
            value if value == syscalls::SYSFS_MAGIC => Self::Sysfs,
            value if value == syscalls::TMPFS_MAGIC => Self::Tmpfs,
            value => Self::Other(value),
        }
    }
}

impl fmt::Display for FilesystemType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Procfs => f.write_str("procfs"),
            Self::Sysfs => f.write_str("sysfs"),
            Self::Tmpfs => f.write_str("tmpfs"),
            Self::Other(magic) => write!(f, "unknown(0x{magic:x})"),
        }
    }
}

/// How a required filesystem became available at its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountDisposition {
    Mounted,
    AlreadyMounted,
}

/// Successful setup for both early Linux virtual filesystems.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MountReport {
    pub procfs: MountDisposition,
    pub sysfs: MountDisposition,
}

/// The operation being performed when a mount prerequisite failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountStage {
    OpenTarget,
    InspectFilesystem,
    InspectMountId,
    Mount,
    VerifyMountedFilesystem,
}

/// A fail-closed classification for a procfs/sysfs prerequisite failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountFailure {
    /// The caller lacks permission or `CAP_SYS_ADMIN` for the requested work.
    PermissionDenied { stage: MountStage, errno: i32 },
    /// The kernel does not report mount flags through `fstatfs(2)`.
    MountFlagsUnavailable { actual: u64 },
    /// An existing or newly mounted filesystem is missing required mount flags.
    MissingRequiredMountFlags {
        required: u64,
        actual: u64,
        missing: u64,
    },
    /// A separate filesystem mount already occupies the target.
    WrongFilesystem {
        expected: FilesystemType,
        actual: FilesystemType,
    },
    /// The kernel could not provide a mount ID, so mountpoint state is
    /// ambiguous. The module will not risk overmounting an unknown target.
    MountIdUnavailable { errno: i32 },
    /// An operation failed for a reason other than permission or wrong type.
    SystemCall { stage: MountStage, errno: i32 },
}

/// The target and classified reason for a failed prerequisite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MountError {
    pub target: MountPoint,
    pub failure: MountFailure,
}

impl fmt::Display for MountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.failure {
            MountFailure::PermissionDenied { stage, errno } => write!(
                f,
                "permission denied at {} during {stage:?} (errno {errno})",
                self.target
            ),
            MountFailure::MountFlagsUnavailable { actual } => write!(
                f,
                "mount flags unavailable at {} (f_flags=0x{actual:x})",
                self.target
            ),
            MountFailure::MissingRequiredMountFlags {
                required,
                actual,
                missing,
            } => write!(
                f,
                "required mount flags missing at {}: required=0x{required:x} actual=0x{actual:x} missing=0x{missing:x}",
                self.target
            ),
            MountFailure::WrongFilesystem { expected, actual } => write!(
                f,
                "wrong filesystem at {}: expected {expected}, found {actual}",
                self.target
            ),
            MountFailure::MountIdUnavailable { errno } => write!(
                f,
                "cannot identify mountpoint state for {} (errno {errno})",
                self.target
            ),
            MountFailure::SystemCall { stage, errno } => write!(
                f,
                "{} failed at {} (errno {errno})",
                stage_name(stage),
                self.target
            ),
        }
    }
}

impl std::error::Error for MountError {}

/// Ensure `/proc` is procfs and `/sys` is sysfs before dependent services run.
///
/// Correct existing mounts are accepted only after checking their real
/// filesystem magic and required mount flags. A mount of another filesystem
/// or a mount missing required flags is rejected without attempting to cover
/// or remount it. Ordinary unmounted directories on the initramfs root
/// filesystem are valid mount targets.
///
/// Call this after the existing `/dev` setup and before starting services.
/// An error does not imply that nothing was mounted: procfs may have mounted
/// before a later sysfs failure. The caller must keep services stopped and
/// preserve its fail-closed PID 1 wait path; this function never signals
/// service readiness.
pub fn ensure_proc_and_sysfs() -> Result<MountReport, MountError> {
    let procfs = ensure_mount(MountPoint::Proc)?;
    let sysfs = ensure_mount(MountPoint::Sys)?;
    Ok(MountReport { procfs, sysfs })
}

fn ensure_mount(target: MountPoint) -> Result<MountDisposition, MountError> {
    let current = inspect_target(target)?;
    if current.is_separate_mount {
        return accept_existing_mount(target, current);
    }

    // This path intentionally uses only the fixed, least-privilege mount
    // options needed by early procfs/sysfs consumers. `/sys` is read-only.
    let result = unsafe {
        syscalls::mount(
            target.filesystem_name().as_ptr(),
            target.path().as_ptr(),
            target.filesystem_name().as_ptr(),
            target.flags(),
            std::ptr::null(),
        )
    };
    if result != 0 {
        let errno = last_errno();
        if is_permission_errno(errno) {
            return Err(MountError {
                target,
                failure: MountFailure::PermissionDenied {
                    stage: MountStage::Mount,
                    errno,
                },
            });
        }
        if errno == syscalls::EBUSY {
            // A concurrent actor may have installed a mount after the first
            // inspection. Accept only a verified, distinct mount of the
            // expected type; reject an incorrect type without overmounting.
            let after = inspect_target(target)?;
            if after.is_separate_mount {
                return accept_existing_mount(target, after);
            }
        }
        return Err(MountError {
            target,
            failure: MountFailure::SystemCall {
                stage: MountStage::Mount,
                errno,
            },
        });
    }

    let mounted = inspect_target(target)?;
    if mounted.filesystem != target.expected_filesystem() {
        return Err(wrong_filesystem(target, mounted.filesystem));
    }
    if !mounted.is_separate_mount {
        return Err(MountError {
            target,
            failure: MountFailure::SystemCall {
                stage: MountStage::VerifyMountedFilesystem,
                errno: syscalls::EINVAL,
            },
        });
    }
    verify_required_mount_flags(target, mounted.mount_flags)?;
    Ok(MountDisposition::Mounted)
}

fn accept_existing_mount(
    target: MountPoint,
    state: TargetState,
) -> Result<MountDisposition, MountError> {
    if state.filesystem != target.expected_filesystem() {
        return Err(wrong_filesystem(target, state.filesystem));
    }
    verify_required_mount_flags(target, state.mount_flags)?;
    Ok(MountDisposition::AlreadyMounted)
}

fn verify_required_mount_flags(target: MountPoint, actual: u64) -> Result<(), MountError> {
    let required = target.required_statfs_flags();
    match mount_flags::verify(required, actual) {
        Ok(()) => Ok(()),
        Err(mount_flags::VerificationError::Unsupported { actual }) => Err(MountError {
            target,
            failure: MountFailure::MountFlagsUnavailable { actual },
        }),
        Err(mount_flags::VerificationError::Missing {
            required,
            actual,
            missing,
        }) => Err(MountError {
            target,
            failure: MountFailure::MissingRequiredMountFlags {
                required,
                actual,
                missing,
            },
        }),
    }
}

#[derive(Debug, Clone, Copy)]
struct TargetState {
    filesystem: FilesystemType,
    is_separate_mount: bool,
    mount_flags: u64,
}

fn inspect_target(target: MountPoint) -> Result<TargetState, MountError> {
    let target_fd = open_directory(target, target.path())?;
    let root_fd = open_directory(target, c"/")?;

    let mut filesystem = MaybeUninit::<syscalls::StatFs>::zeroed();
    let result = unsafe { syscalls::fstatfs(target_fd.as_raw_fd(), filesystem.as_mut_ptr()) };
    if result != 0 {
        return Err(errno_error(
            target,
            MountStage::InspectFilesystem,
            last_errno(),
        ));
    }
    let filesystem = unsafe { filesystem.assume_init() };

    let target_mount_id = mount_id(target, target_fd.as_raw_fd())?;
    let root_mount_id = mount_id(target, root_fd.as_raw_fd())?;
    Ok(TargetState {
        filesystem: FilesystemType::from_magic(filesystem.f_type),
        is_separate_mount: target_mount_id != root_mount_id,
        mount_flags: filesystem.f_flags as u64,
    })
}

fn open_directory(target: MountPoint, path: &std::ffi::CStr) -> Result<OwnedFd, MountError> {
    let fd = unsafe {
        syscalls::open(
            path.as_ptr(),
            syscalls::O_PATH | syscalls::O_DIRECTORY | syscalls::O_CLOEXEC | syscalls::O_NOFOLLOW,
        )
    };
    if fd < 0 {
        return Err(errno_error(target, MountStage::OpenTarget, last_errno()));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn mount_id(target: MountPoint, fd: std::os::raw::c_int) -> Result<u64, MountError> {
    let mut stat = MaybeUninit::<syscalls::Statx>::zeroed();
    let result = unsafe {
        syscalls::statx(
            fd,
            c"".as_ptr(),
            syscalls::AT_EMPTY_PATH | syscalls::AT_STATX_SYNC_AS_STAT,
            STATX_MNT_ID_MASK,
            stat.as_mut_ptr(),
        )
    };
    if result != 0 {
        let errno = last_errno();
        return Err(MountError {
            target,
            failure: if is_permission_errno(errno) {
                MountFailure::PermissionDenied {
                    stage: MountStage::InspectMountId,
                    errno,
                }
            } else {
                MountFailure::MountIdUnavailable { errno }
            },
        });
    }
    let stat = unsafe { stat.assume_init() };
    if stat.stx_mask & STATX_MNT_ID_MASK == 0 {
        return Err(MountError {
            target,
            failure: MountFailure::MountIdUnavailable {
                errno: syscalls::ENOTSUP,
            },
        });
    }
    Ok(stat.stx_mnt_id)
}

fn wrong_filesystem(target: MountPoint, actual: FilesystemType) -> MountError {
    MountError {
        target,
        failure: MountFailure::WrongFilesystem {
            expected: target.expected_filesystem(),
            actual,
        },
    }
}

fn errno_error(target: MountPoint, stage: MountStage, errno: i32) -> MountError {
    MountError {
        target,
        failure: if is_permission_errno(errno) {
            MountFailure::PermissionDenied { stage, errno }
        } else {
            MountFailure::SystemCall { stage, errno }
        },
    }
}

fn is_permission_errno(errno: i32) -> bool {
    errno == syscalls::EPERM || errno == syscalls::EACCES
}

fn last_errno() -> i32 {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(syscalls::EIO)
}

const fn stage_name(stage: MountStage) -> &'static str {
    match stage {
        MountStage::OpenTarget => "open target",
        MountStage::InspectFilesystem => "fstatfs",
        MountStage::InspectMountId => "statx mount ID",
        MountStage::Mount => "mount",
        MountStage::VerifyMountedFilesystem => "post-mount verification",
    }
}
