//! Shared, non-forgeable identity contract for the PID-1 Linux spawn owner.
//!
//! The only constructor for `SpawnedProcessPidfd` is the parent branch of
//! `clone3_pid1_service_child()`. There is deliberately no `from_raw_fd`, PID
//! constructor, or numeric-PID fallback. Both the ProcessOs owner and an
//! identity-only registry bridge can depend on this small crate.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::sync::Arc;

/// Opaque identity for one exact child created by this crate's clone3 call.
/// Clones preserve identity through a private Arc nonce; a PID alone cannot
/// create or compare equal to this value.
#[derive(Clone)]
pub struct SpawnedProcessIdentity {
    process_id: u32,
    nonce: Arc<()>,
}

impl SpawnedProcessIdentity {
    pub fn process_id(&self) -> u32 {
        self.process_id
    }

    pub fn same_instance(&self, other: &Self) -> bool {
        self.process_id == other.process_id && Arc::ptr_eq(&self.nonce, &other.nonce)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub fn test_identity(process_id: u32) -> SpawnedProcessIdentity {
    SpawnedProcessIdentity {
        process_id,
        nonce: Arc::new(()),
    }
}

impl PartialEq for SpawnedProcessIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.same_instance(other)
    }
}

impl Eq for SpawnedProcessIdentity {}

impl std::fmt::Debug for SpawnedProcessIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SpawnedProcessIdentity")
            .field("process_id", &self.process_id)
            .finish_non_exhaustive()
    }
}

/// The live pidfd and opaque instance identity created atomically by clone3.
/// This type is not `Clone`; a recipient gets a fallible duplicated pidfd.
pub struct SpawnedProcessPidfd {
    identity: SpawnedProcessIdentity,
    pidfd: Option<OwnedFd>,
}

impl SpawnedProcessPidfd {
    pub fn process_id(&self) -> u32 {
        self.identity.process_id
    }

    pub fn identity(&self) -> SpawnedProcessIdentity {
        self.identity.clone()
    }

    pub fn same_instance(&self, other: &Self) -> bool {
        self.identity.same_instance(&other.identity)
    }

    pub fn as_raw_fd(&self) -> io::Result<RawFd> {
        self.pidfd
            .as_ref()
            .map(AsRawFd::as_raw_fd)
            .ok_or_else(|| io::Error::other("spawn pidfd has already been closed"))
    }

    /// Duplicate the exact live pidfd for an identity authority. The duplicate
    /// shares only the opaque identity nonce; it does not transfer wait/reap
    /// ownership from the ProcessOs lifecycle.
    pub fn duplicate_for_authority(&self) -> io::Result<Self> {
        let source = self.as_raw_fd()?;
        let duplicated = duplicate_cloexec(source)?;
        // SAFETY: duplicate_cloexec returns a fresh owned descriptor.
        let pidfd = unsafe { OwnedFd::from_raw_fd(duplicated) };
        Ok(Self {
            identity: self.identity.clone(),
            pidfd: Some(pidfd),
        })
    }

    /// Close this token's descriptor once while keeping its opaque identity
    /// available for terminal-result correlation. A failed close remains
    /// ambiguous and is never retried.
    pub fn close_now(&mut self) -> io::Result<()> {
        let Some(fd) = self.pidfd.take() else {
            return Ok(());
        };
        let raw = fd.into_raw_fd();
        close_once(raw)
    }
}

/// Result from the only pidfd-token issuer. `Child` has no token. A missing
/// pidfd after a positive clone result is surfaced separately so the caller can
/// close the exec gate and fail closed; it must not invent identity from PID.
pub enum Clone3Pid1Child {
    Child,
    Parent(SpawnedProcessPidfd),
    ParentWithoutPidfd { process_id: u32 },
}

/// Create a service namespace child with an atomically returned pidfd.
///
/// # Safety
/// The caller must run only its child-side, raw-syscall setup path after the
/// `Child` result. It must not use inherited process-wide locks or perform
/// unrelated Rust work before exec/exit. Unsupported targets return an error;
/// this function never falls back to `clone`, `fork`, or `pidfd_open`.
pub unsafe fn clone3_pid1_service_child() -> io::Result<Clone3Pid1Child> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        clone3_x86_64()
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "atomic clone3/CLONE_PIDFD is supported only on Linux x86_64",
        ))
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[repr(C)]
#[derive(Default)]
struct CloneArgs {
    flags: u64,
    pidfd: u64,
    child_tid: u64,
    parent_tid: u64,
    exit_signal: u64,
    stack: u64,
    stack_size: u64,
    tls: u64,
    set_tid: u64,
    set_tid_size: u64,
    cgroup: u64,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const _: [(); 88] = [(); std::mem::size_of::<CloneArgs>()];

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn clone3_x86_64() -> io::Result<Clone3Pid1Child> {
    use core::arch::asm;

    const NR_CLONE3: usize = 435;
    const CLONE_NEWPID: u64 = 0x2000_0000;
    const CLONE_PIDFD: u64 = 0x0000_1000;
    const SIGCHLD: u64 = 17;

    // Allocate the nonce before clone3. The parent path can then construct the
    // token without allocation after the child exists. The child intentionally
    // forgets its inherited Arc before entering the raw setup path.
    let nonce = Arc::new(());
    let mut pidfd_raw = -1i32;
    let args = CloneArgs {
        flags: CLONE_NEWPID | CLONE_PIDFD,
        pidfd: &mut pidfd_raw as *mut i32 as u64,
        exit_signal: SIGCHLD,
        ..CloneArgs::default()
    };

    let result: isize;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") NR_CLONE3 as isize => result,
            in("rdi") &args as *const CloneArgs as usize,
            in("rsi") std::mem::size_of::<CloneArgs>(),
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }

    if result < 0 {
        return Err(io::Error::from_raw_os_error((-result) as i32));
    }
    if result == 0 {
        std::mem::forget(nonce);
        return Ok(Clone3Pid1Child::Child);
    }

    let process_id = result as u32;
    if pidfd_raw < 0 {
        return Ok(Clone3Pid1Child::ParentWithoutPidfd { process_id });
    }
    // SAFETY: clone3 with CLONE_PIDFD wrote a fresh pidfd into pidfd_raw.
    let pidfd = unsafe { OwnedFd::from_raw_fd(pidfd_raw) };
    Ok(Clone3Pid1Child::Parent(SpawnedProcessPidfd {
        identity: SpawnedProcessIdentity { process_id, nonce },
        pidfd: Some(pidfd),
    }))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn duplicate_cloexec(fd: RawFd) -> io::Result<RawFd> {
    use core::arch::asm;
    const NR_FCNTL: usize = 72;
    const F_DUPFD_CLOEXEC: usize = 1030;
    let result: isize;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") NR_FCNTL as isize => result,
            in("rdi") fd as usize,
            in("rsi") F_DUPFD_CLOEXEC,
            in("rdx") 3usize,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    if result < 0 {
        Err(io::Error::from_raw_os_error((-result) as i32))
    } else {
        Ok(result as RawFd)
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn duplicate_cloexec(_fd: RawFd) -> io::Result<RawFd> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "pidfd duplication is supported only on Linux x86_64",
    ))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn close_once(fd: RawFd) -> io::Result<()> {
    use core::arch::asm;
    const NR_CLOSE: usize = 3;
    let result: isize;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") NR_CLOSE as isize => result,
            in("rdi") fd as usize,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    if result < 0 {
        Err(io::Error::from_raw_os_error((-result) as i32))
    } else {
        Ok(())
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn close_once(_fd: RawFd) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "pidfd close is supported only on Linux x86_64",
    ))
}

#[cfg(test)]
mod tests {
    use super::test_identity;

    #[test]
    fn cloned_identity_keeps_the_same_private_nonce() {
        let original = test_identity(51);
        let handoff = original.clone();
        assert!(original.same_instance(&handoff));
        assert_eq!(original, handoff);
    }

    #[test]
    fn pid_reuse_does_not_make_a_different_spawn_equal() {
        let first = test_identity(52);
        let second = test_identity(52);
        assert!(!first.same_instance(&second));
        assert_ne!(first, second);
    }
}


/// Test-only identity token for owner-state regression tests. The descriptor
/// is intentionally absent; this must never stand in for Linux pidfd runtime.
#[cfg(feature = "test-support")]
pub fn test_pidfd_for_identity(identity: SpawnedProcessIdentity) -> SpawnedProcessPidfd {
    SpawnedProcessPidfd { identity, pidfd: None }
}
