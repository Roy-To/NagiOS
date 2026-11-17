//! Opaque OS proof for a system service endpoint owned by one exact C3 launch.
//!
//! This crate is deliberately independent of C1. C1 may seal
//! [`SystemServiceEndpointOwnerProof`] as its authenticated system-service
//! caller type, while C3 remains responsible for creating the launch binding
//! only after its trusted Supervisor has accepted the exact system-service
//! record.

use nagi_init_spawn_contract::{SpawnedProcessIdentity, SpawnedProcessPidfd};
use std::io;
use std::num::NonZeroU64;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::os::fd::{AsRawFd, RawFd};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// Version of the C3-to-C1 system-service endpoint-owner proof contract.
pub const SYSTEM_SERVICE_ENDPOINT_OWNER_PROOF_VERSION: u32 = 2;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
static NEXT_ENDPOINT_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Role recorded by C3 for the launch currently occupying a Supervisor slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupervisorLaunchRole {
    SystemService,
    InteractiveApplication,
}

struct CurrentSupervisorLaunch {
    process: SpawnedProcessIdentity,
    generation: NonZeroU64,
    role: SupervisorLaunchRole,
    active: bool,
}

/// The process and exact Supervisor launch generation C3 accepted for a
/// system-service launch. The shared cell is the serialized C3 record gate:
/// replacement, revocation, and authority callbacks must all use it.
#[derive(Clone)]
pub struct SystemServiceLaunchRecord {
    process: SpawnedProcessIdentity,
    generation: NonZeroU64,
    role: SupervisorLaunchRole,
    current: Arc<Mutex<CurrentSupervisorLaunch>>,
}

impl SystemServiceLaunchRecord {
    /// Bind a newly allocated generation to the exact record that C3 already
    /// validated in its trusted Supervisor.
    ///
    /// # Safety
    /// The caller must have verified that `generation` identifies the current,
    /// live Supervisor record for `process` with the supplied `role`. The
    /// returned handle must be stored in the owning Supervisor slot, and every
    /// later replacement/revocation of that slot must use this shared gate.
    /// This function preserves C3's decision; it does not establish package
    /// trust or authorization itself.
    pub unsafe fn after_supervisor_record_validation(
        process: &SpawnedProcessPidfd,
        generation: NonZeroU64,
        role: SupervisorLaunchRole,
    ) -> Self {
        let process_identity = process.identity();
        Self {
            process: process_identity.clone(),
            generation,
            role,
            current: Arc::new(Mutex::new(CurrentSupervisorLaunch {
                process: process_identity,
                generation,
                role,
                active: true,
            })),
        }
    }

    /// Replace this exact Supervisor slot after C3 has validated the new
    /// record while holding its owning registry lock. Existing proofs keep the
    /// shared gate and fail their generation/process comparison after this
    /// method commits.
    ///
    /// # Safety
    /// The caller must hold the canonical C3 Supervisor lock. The closure must
    /// replace that exact registry slot and return true only after the registry
    /// accepted the new record. If it returns false, it must leave the registry
    /// unchanged. The registry mutation runs while this shared generation gate
    /// is held, before the new gate state is published.
    pub unsafe fn replace_after_supervisor_record_validation(
        &self,
        process: &SpawnedProcessPidfd,
        generation: NonZeroU64,
        role: SupervisorLaunchRole,
        replace_registry_record: impl FnOnce() -> bool,
    ) -> io::Result<Self> {
        let process_identity = process.identity();
        let mut current = self.lock_current()?;
        if !current_matches(&current, &self.process, self.generation, self.role) {
            return Err(permission_denied(
                "Supervisor launch slot was already replaced",
            ));
        }
        if !replace_registry_record() {
            return Err(io::Error::other(
                "Supervisor rejected the replacement launch record",
            ));
        }
        *current = CurrentSupervisorLaunch {
            process: process_identity.clone(),
            generation,
            role,
            active: true,
        };
        drop(current);
        Ok(Self {
            process: process_identity,
            generation,
            role,
            current: Arc::clone(&self.current),
        })
    }

    pub fn process_identity(&self) -> SpawnedProcessIdentity {
        self.process.clone()
    }

    pub fn generation(&self) -> NonZeroU64 {
        self.generation
    }

    pub fn is_live(&self) -> bool {
        self.lock_current()
            .map(|current| {
                current.active
                    && current.role == SupervisorLaunchRole::SystemService
                    && current_matches(&current, &self.process, self.generation, self.role)
            })
            .unwrap_or(false)
    }

    /// Revoke this exact Supervisor slot and every proof derived from it.
    ///
    /// # Safety
    /// The caller must hold the canonical C3 Supervisor lock. The closure must
    /// revoke the matching registry record and return true only if that exact
    /// record was removed. If it returns false, it must leave the registry
    /// unchanged. Registry mutation runs while the shared generation gate is
    /// held, so admitted effects finish before both records are retired.
    pub unsafe fn revoke_after_supervisor_record_validation(
        &self,
        revoke_registry_record: impl FnOnce() -> bool,
    ) -> bool {
        let Ok(mut current) = self.lock_current() else {
            return false;
        };
        if !current.active || !current_matches(&current, &self.process, self.generation, self.role)
        {
            return false;
        }
        if !revoke_registry_record() {
            return false;
        }
        current.active = false;
        true
    }

    fn lock_current(&self) -> io::Result<MutexGuard<'_, CurrentSupervisorLaunch>> {
        self.current
            .lock()
            .map_err(|_| io::Error::other("C3 Supervisor launch gate is poisoned"))
    }
}

fn current_matches(
    current: &CurrentSupervisorLaunch,
    process: &SpawnedProcessIdentity,
    generation: NonZeroU64,
    role: SupervisorLaunchRole,
) -> bool {
    current.process.same_instance(process)
        && current.generation == generation
        && current.role == role
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinuxPeerCredentials {
    pub process_id: u32,
    pub uid: u32,
    pub gid: u32,
}

/// Non-Clone proof joining the exact spawned process, its C3 launch record,
/// the acquired endpoint generation, and kernel-reported Unix peer identity.
///
/// C1 should borrow this object for the lifetime of a recovery call and use
/// [`Self::with_live_authority_effect`] around every authority effect. The
/// endpoint descriptor stays owned here so dropping or revoking this proof
/// closes only this authenticated connection.
pub struct SystemServiceEndpointOwnerProof {
    process: SpawnedProcessPidfd,
    process_identity: SpawnedProcessIdentity,
    launch_generation: NonZeroU64,
    endpoint: Option<OwnedFd>,
    peer: LinuxPeerCredentials,
    endpoint_generation: NonZeroU64,
    launch: SystemServiceLaunchRecord,
    revoked: bool,
}

impl SystemServiceEndpointOwnerProof {
    /// Authenticate the peer on an already acquired connected Unix stream.
    /// The caller must create/receive this endpoint as part of the exact C3
    /// launch and pass the expected credentials chosen by that launch policy.
    pub fn authenticate(
        process: &SpawnedProcessPidfd,
        launch: &SystemServiceLaunchRecord,
        endpoint: OwnedFd,
        expected_uid: u32,
        expected_gid: u32,
    ) -> io::Result<Self> {
        let current = launch.lock_current()?;
        if !current.active
            || !current_matches(&current, &launch.process, launch.generation, launch.role)
            || launch.role != SupervisorLaunchRole::SystemService
            || !launch.process.same_instance(&process.identity())
        {
            return Err(permission_denied(
                "launch record is stale, has the wrong role, or belongs to another process",
            ));
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let peer = linux_peer_credentials(endpoint.as_raw_fd())?;
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (endpoint, expected_uid, expected_gid);
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "system-service endpoint authentication requires Linux x86_64 SO_PEERCRED",
            ));
        }

        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            if peer.process_id != process.process_id()
                || peer.uid != expected_uid
                || peer.gid != expected_gid
            {
                return Err(permission_denied(
                    "kernel peer credentials do not match the exact C3 process launch",
                ));
            }
            let endpoint_events = poll_zero(endpoint.as_raw_fd())?;
            if endpoint_events & (POLLERR | POLLHUP | POLLNVAL | POLLRDHUP) != 0 {
                return Err(permission_denied(
                    "system-service endpoint is already closed or unusable",
                ));
            }
            let endpoint_generation = next_endpoint_generation()?;
            let process_identity = process.identity();
            let process = process.duplicate_for_authority()?;
            let process_fd = process.as_raw_fd()?;
            if poll_zero(process_fd)? & (POLLIN | POLLERR | POLLHUP | POLLNVAL) != 0 {
                return Err(permission_denied(
                    "system-service process exited before endpoint proof issuance",
                ));
            }
            Ok(Self {
                process,
                process_identity,
                launch_generation: launch.generation,
                endpoint: Some(endpoint),
                peer,
                endpoint_generation,
                launch: SystemServiceLaunchRecord {
                    process: launch.process.clone(),
                    generation: launch.generation,
                    role: launch.role,
                    current: Arc::clone(&launch.current),
                },
                revoked: false,
            })
        }
    }

    pub fn version(&self) -> u32 {
        SYSTEM_SERVICE_ENDPOINT_OWNER_PROOF_VERSION
    }

    pub fn process_identity(&self) -> SpawnedProcessIdentity {
        self.process_identity.clone()
    }

    pub fn process_id(&self) -> u32 {
        self.process_identity.process_id()
    }

    pub fn launch_generation(&self) -> NonZeroU64 {
        self.launch_generation
    }

    pub fn endpoint_generation(&self) -> NonZeroU64 {
        self.endpoint_generation
    }

    pub fn peer_credentials(&self) -> LinuxPeerCredentials {
        self.peer
    }

    pub fn endpoint(&self) -> Option<BorrowedFd<'_>> {
        self.endpoint.as_ref().map(AsFd::as_fd)
    }

    /// Check C3 launch revocation plus the actual pidfd and peer endpoint.
    /// Readable data is allowed; process exit, endpoint hangup/error, or a
    /// revoked launch returns `false`.
    pub fn is_live(&self) -> io::Result<bool> {
        if self.revoked {
            return Ok(false);
        }
        let current = match self.launch.lock_current() {
            Ok(current) => current,
            Err(_) => return Ok(false),
        };
        if !current.active
            || current.role != SupervisorLaunchRole::SystemService
            || !current_matches(
                &current,
                &self.process_identity,
                self.launch_generation,
                SupervisorLaunchRole::SystemService,
            )
        {
            return Ok(false);
        }
        let Some(endpoint) = self.endpoint.as_ref() else {
            return Ok(false);
        };
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let pidfd = match self.process.as_raw_fd() {
                Ok(fd) => fd,
                Err(_) => return Ok(false),
            };
            let pidfd_events = poll_zero(pidfd)?;
            if pidfd_events & (POLLIN | POLLERR | POLLHUP | POLLNVAL) != 0 {
                return Ok(false);
            }
            let endpoint_events = poll_zero(endpoint.as_raw_fd())?;
            if endpoint_events & (POLLERR | POLLHUP | POLLNVAL | POLLRDHUP) != 0 {
                return Ok(false);
            }
            Ok(true)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = endpoint;
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "system-service endpoint liveness requires Linux x86_64 pidfd polling",
            ))
        }
    }

    /// Hold the exact C3 Supervisor generation gate while checking the kernel
    /// process/endpoint and applying one authority effect. C3 record replacement
    /// and revocation block on this same gate and therefore occur wholly before
    /// admission or after the callback returns. `is_live()` is only a snapshot
    /// and is not a substitute for this method.
    ///
    /// Lock order for a caller that also holds C1's Session gate is: C1 Session
    /// gate, then this C3 Supervisor gate, then the effect. The callback must
    /// not re-enter C3 replacement/revocation or the same C1 lifecycle gate.
    pub fn with_live_authority_effect<R>(&self, effect: impl FnOnce() -> R) -> io::Result<R> {
        if self.revoked {
            return Err(permission_denied("system-service proof was revoked"));
        }
        let current = self.launch.lock_current()?;
        if !current.active
            || current.role != SupervisorLaunchRole::SystemService
            || !current_matches(
                &current,
                &self.process_identity,
                self.launch_generation,
                SupervisorLaunchRole::SystemService,
            )
        {
            return Err(permission_denied(
                "Supervisor record no longer matches the authenticated launch",
            ));
        }
        let endpoint = self
            .endpoint
            .as_ref()
            .ok_or_else(|| permission_denied("system-service endpoint was closed"))?;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let pidfd = self
                .process
                .as_raw_fd()
                .map_err(|_| permission_denied("system-service pidfd was closed"))?;
            let pidfd_events = poll_zero(pidfd)?;
            if pidfd_events & (POLLIN | POLLERR | POLLHUP | POLLNVAL) != 0 {
                return Err(permission_denied("system-service process has exited"));
            }
            let endpoint_events = poll_zero(endpoint.as_raw_fd())?;
            if endpoint_events & (POLLERR | POLLHUP | POLLNVAL | POLLRDHUP) != 0 {
                return Err(permission_denied("system-service endpoint is closed"));
            }
            Ok(effect())
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = endpoint;
            let _ = effect;
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "system-service authority effects require Linux x86_64",
            ));
        }
    }

    /// Revoke this proof and close its owned endpoint and pidfd. The C3
    /// Supervisor launch owner alone invalidates the shared launch generation.
    pub fn revoke(&mut self) -> bool {
        if self.revoked {
            return false;
        }
        self.revoked = true;
        self.endpoint.take();
        self.process.close_now().ok();
        true
    }
}

fn permission_denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn next_endpoint_generation() -> io::Result<NonZeroU64> {
    NEXT_ENDPOINT_GENERATION
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            current.checked_add(1)
        })
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or_else(|| io::Error::other("system-service endpoint generation exhausted"))
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const POLLIN: i16 = 0x001;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const POLLERR: i16 = 0x008;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const POLLHUP: i16 = 0x010;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const POLLNVAL: i16 = 0x020;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const POLLRDHUP: i16 = 0x2000;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[repr(C)]
struct PollFd {
    fd: i32,
    events: i16,
    revents: i16,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[repr(C)]
#[derive(Default)]
struct Ucred {
    pid: i32,
    uid: u32,
    gid: u32,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const AF_UNIX: i32 = 1;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SOCK_STREAM: i32 = 1;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SOL_SOCKET: i32 = 1;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SO_TYPE: i32 = 3;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SO_PEERCRED: i32 = 17;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const SO_DOMAIN: i32 = 39;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn linux_peer_credentials(fd: RawFd) -> io::Result<LinuxPeerCredentials> {
    let domain = get_socket_integer(fd, SO_DOMAIN)?;
    let socket_type = get_socket_integer(fd, SO_TYPE)?;
    if domain != AF_UNIX || socket_type != SOCK_STREAM {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "system-service endpoint must be a connected AF_UNIX SOCK_STREAM socket",
        ));
    }

    let mut peer = Ucred::default();
    let mut length = std::mem::size_of::<Ucred>() as u32;
    syscall_result(unsafe {
        syscall5(
            55, // getsockopt
            fd as usize,
            SOL_SOCKET as usize,
            SO_PEERCRED as usize,
            &mut peer as *mut Ucred as usize,
            &mut length as *mut u32 as usize,
        )
    })?;
    if length as usize != std::mem::size_of::<Ucred>() || peer.pid <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Linux returned malformed SO_PEERCRED data",
        ));
    }
    Ok(LinuxPeerCredentials {
        process_id: peer.pid as u32,
        uid: peer.uid,
        gid: peer.gid,
    })
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn get_socket_integer(fd: RawFd, option: i32) -> io::Result<i32> {
    let mut value = 0i32;
    let mut length = std::mem::size_of::<i32>() as u32;
    syscall_result(unsafe {
        syscall5(
            55,
            fd as usize,
            SOL_SOCKET as usize,
            option as usize,
            &mut value as *mut i32 as usize,
            &mut length as *mut u32 as usize,
        )
    })?;
    if length as usize != std::mem::size_of::<i32>() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Linux returned malformed socket metadata",
        ));
    }
    Ok(value)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn poll_zero(fd: RawFd) -> io::Result<i16> {
    let mut pollfd = PollFd {
        fd,
        events: POLLIN | POLLRDHUP,
        revents: 0,
    };
    syscall_result(unsafe { syscall3(7, &mut pollfd as *mut PollFd as usize, 1, 0) })?;
    if pollfd.revents & POLLNVAL != 0 {
        return Err(io::Error::from_raw_os_error(9));
    }
    Ok(pollfd.revents)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn syscall_result(result: isize) -> io::Result<usize> {
    if result < 0 {
        Err(io::Error::from_raw_os_error((-result) as i32))
    } else {
        Ok(result as usize)
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
unsafe fn syscall3(number: usize, a1: usize, a2: usize, a3: usize) -> isize {
    use core::arch::asm;
    let result: isize;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") number as isize => result,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    result
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
unsafe fn syscall5(number: usize, a1: usize, a2: usize, a3: usize, a4: usize, a5: usize) -> isize {
    use core::arch::asm;
    let result: isize;
    unsafe {
        asm!(
            "syscall",
            inlateout("rax") number as isize => result,
            in("rdi") a1,
            in("rsi") a2,
            in("rdx") a3,
            in("r10") a4,
            in("r8") a5,
            lateout("rcx") _,
            lateout("r11") _,
            options(nostack)
        );
    }
    result
}
