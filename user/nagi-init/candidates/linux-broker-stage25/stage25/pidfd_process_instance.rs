//! Kernel-backed process-instance matching for the task-41 live-peer adapter.
//!
//! The production path accepts Task24's `SpawnedProcessPidfd` handoff, whose
//! only issuer is the parent branch of the shared clone3(CLONE_PIDFD) contract.
//! The adapter retains only its duplicated liveness descriptor and leaves wait,
//! signal, and reaping to Task24. There is no `pidfd_open(pid)` fallback. The
//! checkout side obtains `SO_PEERPIDFD` from the exact accepted stream.

use super::{
    LaunchRecordView, PeerDenial, PeerProcessInstanceAuthority, Pid1SpawnProof, ProcessInstanceKey,
    SupervisorLaunchAuthority, TrustedNipcOperationRoute,
};
use nagi_init_spawn_contract::{SpawnedProcessIdentity, SpawnedProcessPidfd};
use std::collections::{HashMap, HashSet};
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

static NEXT_PROCESS_INSTANCE_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Opaque pairing issued by the pidfd-backed spawn owner after the exact
/// Supervisor record and live child instance have both been checked. The
/// endpoint owner consumes this value; callers cannot replace its listener,
/// route, record, or process-instance token independently.
///
/// Task24 now provides the sealed spawn token this adapter consumes. This
/// remains a candidate boundary until the boot owner invokes it with that
/// token and the listener prepared for the same child before the exec gate.
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::pidfd_process_instance::SpawnOwnedEndpointBinding;
/// let forged = SpawnOwnedEndpointBinding { proof: todo!(), route: todo!(), listener: todo!() };
/// ```
pub(crate) struct SpawnOwnedEndpointBinding<R: LaunchRecordView, L = UnixListener> {
    pub(super) proof: Pid1SpawnProof<R>,
    pub(super) route: TrustedNipcOperationRoute,
    pub(super) listener: L,
}

/// Sealed evidence copied from the one non-Clone Task24 spawn token. The
/// constructor is private to this module so root registry callers cannot
/// attach an arbitrary same-PID identity to a pidfd selector.
pub(crate) struct ExactTask24SpawnIdentity {
    identity: SpawnedProcessIdentity,
}

impl ExactTask24SpawnIdentity {
    fn from_spawn(spawn: &SpawnedProcessPidfd) -> Self {
        Self {
            identity: spawn.identity(),
        }
    }

    pub(super) fn into_identity(self) -> SpawnedProcessIdentity {
        self.identity
    }
}

impl<R: LaunchRecordView, L> std::fmt::Debug for SpawnOwnedEndpointBinding<R, L> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SpawnOwnedEndpointBinding")
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessInstanceOption {
    PeerPidfd,
    SpawnPidfdLiveness,
    PeerPidfdLiveness,
    SpawnCleanup,
}

#[derive(Debug)]
pub enum ProcessInstanceError {
    Unavailable,
    KernelRead {
        option: ProcessInstanceOption,
        source: io::Error,
    },
    MalformedResponse,
}

impl std::fmt::Display for ProcessInstanceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("kernel process-instance proof unavailable"),
            Self::KernelRead { option, source } => {
                write!(
                    formatter,
                    "kernel process-instance operation {option:?} failed: {source}"
                )
            }
            Self::MalformedResponse => {
                formatter.write_str("malformed kernel process-instance response")
            }
        }
    }
}

impl std::error::Error for ProcessInstanceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::KernelRead { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub(crate) trait PidfdKernel: Send + Sync {
    type Handle: Clone + Send + Sync + 'static;

    fn peer_pidfd(&self, stream: &UnixStream) -> Result<Self::Handle, ProcessInstanceError>;

    fn terminate_and_reap_unregistered(
        &self,
        _handle: &Self::Handle,
    ) -> Result<(), ProcessInstanceError> {
        Err(ProcessInstanceError::Unavailable)
    }

    /// Cleanup for a direct child whose atomic pidfd result was absent. This
    /// path is cleanup-only; the numeric PID is never process proof.
    fn terminate_and_reap_untracked(&self, _pid: u32) -> Result<(), ProcessInstanceError> {
        Err(ProcessInstanceError::Unavailable)
    }

    fn confirms_same_live_instance(
        &self,
        spawned: &Self::Handle,
        peer: &Self::Handle,
        spawned_pid: u32,
        peer_pid: u32,
    ) -> Result<bool, ProcessInstanceError>;
}

/// Process-instance authority with an adapter-owned table of Task24 duplicate
/// pidfds. Task24 retains the original child pidfd and its single reap owner.
pub(crate) struct PidfdProcessInstanceAuthority<K: PidfdKernel> {
    kernel: K,
    spawned: Mutex<HashMap<ProcessInstanceKey, K::Handle>>,
    issued_proofs: Mutex<HashSet<ProcessInstanceKey>>,
    task24_spawned: Mutex<HashMap<u32, Task24SpawnIdentityRecord>>,
    cleanup_failures: Mutex<HashMap<ProcessInstanceKey, PeerDenial>>,
    pre_capture_cleanup: Mutex<HashMap<u32, PreCaptureCleanupEntry<K::Handle>>>,
}

struct Task24SpawnIdentityRecord {
    process: ProcessInstanceKey,
    identity: SpawnedProcessIdentity,
}

#[derive(Clone)]
struct PreCaptureCleanupEntry<H> {
    process_id: u32,
    pidfd: Option<H>,
    failure: PeerDenial,
}

impl<K: PidfdKernel> PidfdProcessInstanceAuthority<K> {
    pub(crate) fn new(kernel: K) -> Self {
        Self {
            kernel,
            spawned: Mutex::new(HashMap::new()),
            issued_proofs: Mutex::new(HashSet::new()),
            task24_spawned: Mutex::new(HashMap::new()),
            cleanup_failures: Mutex::new(HashMap::new()),
            pre_capture_cleanup: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    pub(super) fn kernel(&self) -> &K {
        &self.kernel
    }

    /// The generic insertion point is test-only. Production Linux callers must
    /// use `capture_clone3_child`, which accepts the opaque pair emitted by the
    /// atomic clone3 helper rather than caller-supplied PID/FD values.
    #[cfg(test)]
    pub(super) fn record_test_spawn_handle(
        &self,
        pid: u32,
        pidfd: K::Handle,
    ) -> Result<ProcessInstanceKey, PeerDenial> {
        self.record_spawn_handle(pid, pidfd)
    }

    #[cfg(test)]
    pub(super) fn pending_test_spawn_handle(
        &self,
        pid: u32,
        pidfd: K::Handle,
    ) -> Result<PendingPid1Spawn<'_, K>, PeerDenial> {
        let process = self.record_spawn_handle(pid, pidfd)?;
        Ok(PendingPid1Spawn {
            authority: self,
            process,
            armed: true,
        })
    }

    #[cfg(test)]
    pub(super) fn pending_test_precapture_spawn(
        &self,
        pid: u32,
        pidfd: Option<K::Handle>,
    ) -> Result<PreCaptureSpawnGuard<'_, K>, PeerDenial> {
        self.new_precapture_guard(pid, pidfd)
    }

    #[cfg(test)]
    pub(super) fn inject_test_cleanup_failure(
        &self,
        process: ProcessInstanceKey,
    ) -> Result<(), PeerDenial> {
        self.cleanup_failures
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .insert(process, PeerDenial::Unavailable);
        Ok(())
    }

    fn new_precapture_guard(
        &self,
        pid: u32,
        pidfd: Option<K::Handle>,
    ) -> Result<PreCaptureSpawnGuard<'_, K>, PeerDenial> {
        if pid == 0 {
            return Err(PeerDenial::InvalidProcess);
        }
        Ok(PreCaptureSpawnGuard {
            authority: self,
            cleanup_id: pid,
            process_id: pid,
            pidfd,
            armed: true,
        })
    }

    fn retain_precapture_cleanup(
        &self,
        cleanup_id: u32,
        process_id: u32,
        pidfd: Option<K::Handle>,
        failure: PeerDenial,
    ) {
        let mut pending = self
            .pre_capture_cleanup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pending.insert(
            cleanup_id,
            PreCaptureCleanupEntry {
                process_id,
                pidfd,
                failure,
            },
        );
    }

    /// Failures from a parent clone3 result dropped before PID 1 captured it.
    /// The authority retains the pidfd (or cleanup-only child PID) for retry.
    pub(crate) fn precapture_cleanup_failures(
        &self,
    ) -> Result<Vec<(u32, u32, PeerDenial)>, PeerDenial> {
        Ok(self
            .pre_capture_cleanup
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .iter()
            .map(|(id, entry)| (*id, entry.process_id, entry.failure.clone()))
            .collect())
    }

    fn precapture_cleanup_failure(
        &self,
        cleanup_id: u32,
    ) -> Result<Option<PeerDenial>, PeerDenial> {
        Ok(self
            .pre_capture_cleanup
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .get(&cleanup_id)
            .map(|entry| entry.failure.clone()))
    }

    /// Retry failures retained before spawn capture. PID 1 must keep launch
    /// readiness unavailable while any retained cleanup entry remains.
    pub(crate) fn retry_precapture_cleanups(&self) -> Result<(), PeerDenial> {
        let snapshot = self
            .pre_capture_cleanup
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .iter()
            .map(|(id, entry)| (*id, entry.clone()))
            .collect::<Vec<_>>();
        let mut first_failure = None;
        for (cleanup_id, entry) in snapshot {
            match self.cleanup_precapture_child(entry.process_id, entry.pidfd.as_ref()) {
                Ok(()) => {
                    self.pre_capture_cleanup
                        .lock()
                        .map_err(|_| PeerDenial::Unavailable)?
                        .remove(&cleanup_id);
                }
                Err(failure) => {
                    if let Some(current) = self
                        .pre_capture_cleanup
                        .lock()
                        .map_err(|_| PeerDenial::Unavailable)?
                        .get_mut(&cleanup_id)
                    {
                        current.failure = failure.clone();
                    }
                    first_failure.get_or_insert(failure);
                }
            }
        }
        first_failure.map_or(Ok(()), Err)
    }

    fn cleanup_precapture_child(
        &self,
        process_id: u32,
        pidfd: Option<&K::Handle>,
    ) -> Result<(), PeerDenial> {
        match pidfd {
            Some(pidfd) => self
                .kernel
                .terminate_and_reap_unregistered(pidfd)
                .map_err(map_process_instance_error),
            None => self
                .kernel
                .terminate_and_reap_untracked(process_id)
                .map_err(map_process_instance_error),
        }
    }

    fn record_spawn_handle(
        &self,
        pid: u32,
        pidfd: K::Handle,
    ) -> Result<ProcessInstanceKey, PeerDenial> {
        if pid <= 1 {
            return Err(PeerDenial::InvalidProcess);
        }
        if !self
            .kernel
            .confirms_same_live_instance(&pidfd, &pidfd, pid, pid)
            .map_err(map_process_instance_error)?
        {
            return Err(PeerDenial::ProcessInstanceUnavailable);
        }
        let instance = NEXT_PROCESS_INSTANCE_TOKEN
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                current.checked_add(1)
            })
            .map_err(|_| PeerDenial::GenerationExhausted)?;
        let process = ProcessInstanceKey::from_pid1_spawn(pid, instance)
            .ok_or(PeerDenial::ProcessInstanceUnavailable)?;
        let mut spawned = self.spawned.lock().map_err(|_| PeerDenial::Unavailable)?;
        if spawned.contains_key(&process) {
            return Err(PeerDenial::DuplicateProcess);
        }
        spawned.insert(process, pidfd);
        Ok(process)
    }

    /// Confirm that a process key still names the same kernel-backed child.
    /// This is useful to the owning lifecycle code; it does not grant a Nagi
    /// identity or authorize a service route.
    pub(crate) fn confirms_owned_process_live(
        &self,
        process: ProcessInstanceKey,
    ) -> Result<bool, PeerDenial> {
        let pidfd = self
            .spawned
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .get(&process)
            .cloned()
            .ok_or(PeerDenial::ProcessInstanceUnavailable)?;
        self.kernel
            .confirms_same_live_instance(&pidfd, &pidfd, process.pid(), process.pid())
            .map_err(map_process_instance_error)
    }

    /// Abort a child that has not yet committed its exact Supervisor record.
    /// The pidfd, not the reusable numeric PID, is used to signal and reap it.
    fn abort_unregistered_spawn(&self, process: ProcessInstanceKey) -> Result<(), PeerDenial> {
        let result = self.abort_unregistered_spawn_inner(process);
        let mut failures = self
            .cleanup_failures
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        match result {
            Ok(()) => {
                failures.remove(&process);
                Ok(())
            }
            Err(error) => {
                failures.insert(process, error.clone());
                Err(error)
            }
        }
    }

    fn abort_unregistered_spawn_inner(
        &self,
        process: ProcessInstanceKey,
    ) -> Result<(), PeerDenial> {
        let pidfd = self
            .spawned
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .get(&process)
            .cloned()
            .ok_or(PeerDenial::ProcessInstanceUnavailable)?;
        self.kernel
            .terminate_and_reap_unregistered(&pidfd)
            .map_err(map_process_instance_error)?;

        self.spawned
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .remove(&process);
        self.issued_proofs
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .remove(&process);
        Ok(())
    }

    /// Observable failure left by an uncommitted-spawn cleanup attempt.
    /// PID 1 must keep this authority alive and retry cleanup before resuming
    /// launch service; the entry retains the pidfd while cleanup is unresolved.
    pub(crate) fn cleanup_failure(
        &self,
        process: ProcessInstanceKey,
    ) -> Result<Option<PeerDenial>, PeerDenial> {
        Ok(self
            .cleanup_failures
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .get(&process)
            .cloned())
    }

    /// Retry cleanup after an explicit abort or best-effort Drop reported a
    /// failure. This is a cleanup operation only and never produces proof.
    pub(crate) fn retry_unregistered_abort(
        &self,
        process: ProcessInstanceKey,
    ) -> Result<(), PeerDenial> {
        self.abort_unregistered_spawn(process)
    }

    /// Report unresolved capture-failure cleanup so the PID 1 owner can keep
    /// launch readiness unavailable until every owned child is reaped.
    pub(crate) fn has_unresolved_cleanup(&self) -> Result<bool, PeerDenial> {
        Ok(!self
            .cleanup_failures
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .is_empty()
            || !self
                .pre_capture_cleanup
                .lock()
                .map_err(|_| PeerDenial::Unavailable)?
                .is_empty())
    }

    /// Bind the one-use proof to a child handle already observed by PID 1 and
    /// the exact Supervisor record returned for that child.
    pub(crate) fn issue_spawn_proof<R: LaunchRecordView>(
        &self,
        process: ProcessInstanceKey,
        record: R,
    ) -> Result<Pid1SpawnProof<R>, PeerDenial> {
        if record.process_id() != process.pid() {
            return Err(PeerDenial::InvalidProcess);
        }
        let pidfd = self
            .spawned
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .get(&process)
            .cloned()
            .ok_or(PeerDenial::ProcessInstanceUnavailable)?;
        if !self
            .kernel
            .confirms_same_live_instance(&pidfd, &pidfd, process.pid(), process.pid())
            .map_err(map_process_instance_error)?
        {
            return Err(PeerDenial::ProcessInstanceUnavailable);
        }
        let mut issued = self
            .issued_proofs
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        if !issued.insert(process) {
            return Err(PeerDenial::DuplicateProcess);
        }
        Ok(Pid1SpawnProof::from_confirmed_spawn(process, record))
    }

    /// Pair one listener with the exact child instance owned by this PID 1
    /// spawn table. This private issuer checks the live pidfd and Supervisor
    /// record before creating the opaque binding. The call site must pass the
    /// listener returned by that same spawn transaction; a raw listener by
    /// itself is never accepted by `Pid1EndpointOwnerRegistry`.
    pub(crate) fn bind_spawn_owned_endpoint<R, L, A>(
        &self,
        spawn: Pid1SpawnProof<R>,
        route: TrustedNipcOperationRoute,
        listener: L,
        supervisor: &A,
    ) -> Result<SpawnOwnedEndpointBinding<R, L>, PeerDenial>
    where
        R: LaunchRecordView,
        A: SupervisorLaunchAuthority<Record = R>,
    {
        let process = spawn.process;
        let record = spawn.record;
        if process.pid() <= 1 || record.process_id() != process.pid() {
            return Err(PeerDenial::InvalidProcess);
        }
        if supervisor.resolve(process.pid()) != Some(record) {
            return Err(PeerDenial::NotLive);
        }
        if self.has_unresolved_cleanup()? {
            return Err(PeerDenial::UnresolvedSpawnCleanup);
        }
        if !self.confirms_spawned_instance_live(process)? {
            return Err(PeerDenial::StalePeer);
        }
        Ok(SpawnOwnedEndpointBinding {
            proof: spawn,
            route,
            listener,
        })
    }

    /// Call only after the exact Supervisor record has been revoked. Dropping
    /// the pidfd then makes old registrations unable to authenticate a reused
    /// numeric PID.
    pub(crate) fn retire_after_revoke(
        &self,
        process: ProcessInstanceKey,
    ) -> Result<bool, PeerDenial> {
        // Keep lock ordering consistent with Task24 proof capture:
        // task24_spawned -> spawned -> issued_proofs.
        let mut task24_spawned = self
            .task24_spawned
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        let removed = self
            .spawned
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .remove(&process)
            .is_some();
        self.issued_proofs
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .remove(&process);
        if task24_spawned
            .get(&process.pid())
            .is_some_and(|entry| entry.process == process)
        {
            task24_spawned.remove(&process.pid());
        }
        Ok(removed)
    }

    /// Retire the Task24 duplicate pidfd only after its exact reaper has
    /// observed/reaped that child and the existing Supervisor record has been
    /// revoked. The Task24 identity nonce prevents a reused PID from retiring
    /// another child's entry. This closes only our liveness duplicate; Task24
    /// retains sole wait/kill/close ownership of its original child handle.
    pub(crate) fn retire_task24_spawn_after_exact_reap(
        &self,
        identity: &SpawnedProcessIdentity,
    ) -> Result<bool, PeerDenial> {
        let process = {
            let entries = self
                .task24_spawned
                .lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            match entries.get(&identity.process_id()) {
                Some(entry) if entry.identity.same_instance(identity) => entry.process,
                Some(_) => return Err(PeerDenial::StalePeer),
                None => return Ok(false),
            }
        };
        self.retire_after_revoke(process)
    }
}

impl<K: PidfdKernel> super::peer_process_instance_seal::Sealed
    for PidfdProcessInstanceAuthority<K>
{
}

impl<K: PidfdKernel> PeerProcessInstanceAuthority for PidfdProcessInstanceAuthority<K> {
    fn confirms_same_live_process(
        &self,
        stream: &UnixStream,
        peer_pid: u32,
        _peer_uid: u32,
        _peer_gid: u32,
        spawned: ProcessInstanceKey,
    ) -> Result<bool, PeerDenial> {
        if peer_pid != spawned.pid() {
            return Ok(false);
        }
        let spawned_pidfd = self
            .spawned
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .get(&spawned)
            .cloned()
            .ok_or(PeerDenial::ProcessInstanceUnavailable)?;
        let peer_pidfd = self
            .kernel
            .peer_pidfd(stream)
            .map_err(map_process_instance_error)?;
        self.kernel
            .confirms_same_live_instance(&spawned_pidfd, &peer_pidfd, spawned.pid(), peer_pid)
            .map_err(map_process_instance_error)
    }

    fn confirms_spawned_instance_live(
        &self,
        spawned: ProcessInstanceKey,
    ) -> Result<bool, PeerDenial> {
        let spawned_pidfd = self
            .spawned
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .get(&spawned)
            .cloned()
            .ok_or(PeerDenial::ProcessInstanceUnavailable)?;
        self.kernel
            .confirms_same_live_instance(
                &spawned_pidfd,
                &spawned_pidfd,
                spawned.pid(),
                spawned.pid(),
            )
            .map_err(map_process_instance_error)
    }

    fn has_unresolved_cleanup(&self) -> Result<bool, PeerDenial> {
        PidfdProcessInstanceAuthority::has_unresolved_cleanup(self)
    }
}

/// Transaction guard for a clone3 child whose exact Supervisor record has not
/// yet been committed. Drop performs bounded best-effort kill/reap and records
/// failures in the authority. Call `commit_after_supervisor_registration` only
/// after the record exists; ordinary process cleanup then owns revocation.
pub(crate) struct PendingPid1Spawn<'a, K: PidfdKernel> {
    authority: &'a PidfdProcessInstanceAuthority<K>,
    process: ProcessInstanceKey,
    armed: bool,
}

impl<K: PidfdKernel> PendingPid1Spawn<'_, K> {
    pub(crate) fn process(&self) -> ProcessInstanceKey {
        self.process
    }

    pub(crate) fn commit_after_supervisor_registration(mut self) -> ProcessInstanceKey {
        self.armed = false;
        self.process
    }

    pub(crate) fn abort(mut self) -> Result<(), PeerDenial> {
        self.armed = false;
        self.authority.abort_unregistered_spawn(self.process)
    }
}

impl<K: PidfdKernel> Drop for PendingPid1Spawn<'_, K> {
    fn drop(&mut self) {
        if self.armed {
            let _ = self.authority.abort_unregistered_spawn(self.process);
        }
    }
}

/// Owns a clone3 parent result until the PID 1 authority has captured it.
/// Dropping before capture performs cleanup and transfers any failure plus its
/// pidfd back into the same authority for diagnosis and retry.
pub(crate) struct PreCaptureSpawnGuard<'a, K: PidfdKernel> {
    authority: &'a PidfdProcessInstanceAuthority<K>,
    cleanup_id: u32,
    process_id: u32,
    pidfd: Option<K::Handle>,
    armed: bool,
}

impl<K: PidfdKernel> PreCaptureSpawnGuard<'_, K> {
    fn process_id(&self) -> u32 {
        self.process_id
    }

    fn pidfd(&self) -> Option<&K::Handle> {
        self.pidfd.as_ref()
    }

    fn disarm_after_transfer(&mut self) {
        self.armed = false;
    }
}

impl<K: PidfdKernel> Drop for PreCaptureSpawnGuard<'_, K> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let Err(failure) = self
            .authority
            .cleanup_precapture_child(self.process_id, self.pidfd.as_ref())
        {
            self.authority.retain_precapture_cleanup(
                self.cleanup_id,
                self.process_id,
                self.pidfd.clone(),
                failure,
            );
        }
    }
}

fn map_process_instance_error(error: ProcessInstanceError) -> PeerDenial {
    match error {
        ProcessInstanceError::Unavailable => PeerDenial::ProcessInstanceUnavailable,
        error => PeerDenial::KernelProcessInstance(std::sync::Arc::new(error)),
    }
}

#[cfg(target_os = "linux")]
pub(crate) struct LinuxPidfdKernel;

/// A duplicate pidfd received from Task24 is liveness evidence only. It is
/// deliberately a separate variant so this adapter cannot signal/reap the
/// child through Task24's token. `Task41FixtureSpawn` remains limited to the
/// isolated runtime fixture path.
#[cfg(target_os = "linux")]
pub(crate) enum LinuxPidfdHandle {
    #[cfg(feature = "linux-runtime-fixture")]
    Task41FixtureSpawn(std::sync::Arc<std::os::fd::OwnedFd>),
    Task24Spawn(SpawnedProcessPidfd),
    Peer(std::os::fd::OwnedFd),
}

#[cfg(target_os = "linux")]
impl LinuxPidfdHandle {
    fn as_raw_fd(&self) -> io::Result<std::os::fd::RawFd> {
        use std::os::fd::AsRawFd;

        match self {
            #[cfg(feature = "linux-runtime-fixture")]
            Self::Task41FixtureSpawn(pidfd) => Ok(pidfd.as_raw_fd()),
            Self::Task24Spawn(pidfd) => pidfd.as_raw_fd(),
            Self::Peer(pidfd) => Ok(pidfd.as_raw_fd()),
        }
    }
}

/// Parent/child result of an atomic clone3 child and optional pidfd output.
#[must_use = "the parent result must be captured by PID 1 before releasing its exec gate"]
#[cfg(all(target_os = "linux", feature = "linux-runtime-fixture"))]
pub(crate) enum Clone3PidfdOutcome<'a> {
    Child,
    Parent(Clone3PidfdChild<'a>),
}

/// Opaque PID/pidfd pair produced only by the owning authority's clone3 call.
#[cfg(all(target_os = "linux", feature = "linux-runtime-fixture"))]
pub(crate) struct Clone3PidfdChild<'a> {
    pending: PreCaptureSpawnGuard<'a, LinuxPidfdKernel>,
}

#[cfg(target_os = "linux")]
impl PidfdProcessInstanceAuthority<LinuxPidfdKernel> {
    /// Accept only Task24's opaque pidfd token, which can only originate in
    /// `clone3_pid1_service_child()` and is handed off from the exact prepared
    /// ProcessHandle before gate release. This method retains one liveness
    /// duplicate and issues the Task41 binding proof; it does not take over
    /// Task24 wait, signal, or reaping ownership.
    pub(crate) fn capture_task24_spawn_proof<R: LaunchRecordView>(
        &self,
        spawn: SpawnedProcessPidfd,
        record: R,
    ) -> Result<Pid1SpawnProof<R>, PeerDenial> {
        let process_id = spawn.process_id();
        if process_id <= 1 || record.process_id() != process_id {
            return Err(PeerDenial::InvalidProcess);
        }
        let identity = spawn.identity();
        let exact_identity = ExactTask24SpawnIdentity::from_spawn(&spawn);
        let mut imported = self
            .task24_spawned
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        if let Some(current) = imported.get(&process_id) {
            return Err(if current.identity.same_instance(&identity) {
                PeerDenial::DuplicateProcess
            } else {
                // Do not replace an unreaped prior generation at the same PID.
                PeerDenial::ProcessInstanceUnavailable
            });
        }

        let handle = std::sync::Arc::new(LinuxPidfdHandle::Task24Spawn(spawn));
        let process = self.record_spawn_handle(process_id, handle)?;
        let proof = match self.issue_spawn_proof(process, record) {
            Ok(proof) => proof,
            Err(error) => {
                // The Task24 owner still owns the real child and will perform
                // rollback/reap. Remove only this adapter's duplicate handle.
                self.spawned
                    .lock()
                    .map_err(|_| PeerDenial::Unavailable)?
                    .remove(&process);
                self.issued_proofs
                    .lock()
                    .map_err(|_| PeerDenial::Unavailable)?
                    .remove(&process);
                return Err(error);
            }
        };
        let proof = match proof.attach_task24_identity(exact_identity) {
            Ok(proof) => proof,
            Err(error) => {
                // Preserve Task24's sole cleanup/reap ownership. Remove only
                // this adapter's duplicate state after a failed exact binding.
                self.spawned
                    .lock()
                    .map_err(|_| PeerDenial::Unavailable)?
                    .remove(&process);
                self.issued_proofs
                    .lock()
                    .map_err(|_| PeerDenial::Unavailable)?
                    .remove(&process);
                return Err(error);
            }
        };
        imported.insert(process_id, Task24SpawnIdentityRecord { process, identity });
        Ok(proof)
    }

    /// Spawn through `clone3(CLONE_PIDFD)` so the parent receives a pidfd for
    /// the exact child. Any pre-capture guard is tied to this owner and records
    /// a failed Drop cleanup back into its retry table.
    #[cfg(feature = "linux-runtime-fixture")]
    pub(crate) unsafe fn clone3_with_pidfd(
        &self,
        flags: libc::c_ulonglong,
    ) -> io::Result<Clone3PidfdOutcome<'_>> {
        // SAFETY: the caller must handle the child branch immediately and
        // preserve the existing child setup discipline.
        unsafe { clone3_with_pidfd_for_owner(self, flags) }
    }

    /// Retain the exact child handle before PID 1 opens its exec gate.
    #[cfg(feature = "linux-runtime-fixture")]
    pub(crate) fn capture_clone3_child(
        &self,
        child: Clone3PidfdChild<'_>,
    ) -> Result<PendingPid1Spawn<'_, LinuxPidfdKernel>, PeerDenial> {
        let mut pending = child.pending;
        if !std::ptr::eq(pending.authority, self) {
            return Err(PeerDenial::ForeignSpawnOwner);
        }
        let pid = pending.process_id();
        let Some(pidfd) = pending.pidfd().cloned() else {
            let cleanup_id = pending.cleanup_id;
            drop(pending);
            return Err(self
                .precapture_cleanup_failure(cleanup_id)?
                .unwrap_or(PeerDenial::ProcessInstanceUnavailable));
        };
        let process = match self.record_spawn_handle(pid, pidfd.clone()) {
            Ok(process) => process,
            Err(original_error) => {
                let cleanup_id = pending.cleanup_id;
                drop(pending);
                return Err(self
                    .precapture_cleanup_failure(cleanup_id)?
                    .unwrap_or(original_error));
            }
        };
        pending.disarm_after_transfer();
        Ok(PendingPid1Spawn {
            authority: self,
            process,
            armed: true,
        })
    }
}

#[cfg(all(target_os = "linux", feature = "linux-runtime-fixture"))]
unsafe fn clone3_with_pidfd_for_owner(
    owner: &PidfdProcessInstanceAuthority<LinuxPidfdKernel>,
    flags: libc::c_ulonglong,
) -> io::Result<Clone3PidfdOutcome<'_>> {
    use std::mem::{size_of, zeroed};
    use std::os::fd::FromRawFd;

    let mut pidfd: libc::c_int = -1;
    // SAFETY: zero is a valid initialization for the UAPI struct; the fields
    // below are then set to the child flags, pidfd output pointer, and SIGCHLD.
    let mut arguments: libc::clone_args = unsafe { zeroed() };
    arguments.flags = flags | libc::CLONE_PIDFD as libc::c_ulonglong;
    arguments.pidfd = (&mut pidfd as *mut libc::c_int) as libc::c_ulonglong;
    arguments.exit_signal = libc::SIGCHLD as libc::c_ulonglong;

    // SAFETY: clone3 reads the ABI-sized argument struct. This function is
    // unsafe because the caller must handle the child branch immediately and
    // preserve the existing child setup discipline.
    let result = unsafe {
        libc::syscall(
            libc::SYS_clone3,
            (&arguments as *const libc::clone_args).cast::<libc::c_void>(),
            size_of::<libc::clone_args>(),
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    if result == 0 {
        return Ok(Clone3PidfdOutcome::Child);
    }
    // CLONE_PIDFD guarantees a pidfd on supported kernels. If the kernel ever
    // violates that contract, preserve the positive child PID in a cleanup-only
    // guard. It is never admitted as process-instance proof; capture immediately
    // fails closed and attempts to reap this still-owned direct child.
    let pidfd = if pidfd >= 0 {
        // SAFETY: CLONE_PIDFD caused the kernel to install this newly returned
        // descriptor in the parent for the child PID from this same syscall.
        Some(std::sync::Arc::new(LinuxPidfdHandle::Task41FixtureSpawn(
            std::sync::Arc::new(unsafe { std::os::fd::OwnedFd::from_raw_fd(pidfd) }),
        )))
    } else {
        None
    };
    Ok(Clone3PidfdOutcome::Parent(Clone3PidfdChild {
        pending: owner
            .new_precapture_guard(result as u32, pidfd)
            .map_err(|error| io::Error::other(format!("PID 1 spawn capture failed: {error:?}")))?,
    }))
}

#[cfg(target_os = "linux")]
impl PidfdKernel for LinuxPidfdKernel {
    type Handle = std::sync::Arc<LinuxPidfdHandle>;

    fn peer_pidfd(&self, stream: &UnixStream) -> Result<Self::Handle, ProcessInstanceError> {
        use std::mem::size_of;
        use std::os::fd::{AsRawFd, FromRawFd};

        loop {
            let mut raw_fd: libc::c_int = -1;
            let mut length = size_of::<libc::c_int>() as libc::socklen_t;
            // SAFETY: `raw_fd` and `length` are writable ABI-sized buffers;
            // the socket descriptor is borrowed from the exact checked stream.
            let result = unsafe {
                libc::getsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERPIDFD,
                    (&mut raw_fd as *mut libc::c_int).cast(),
                    &mut length,
                )
            };
            if result == 0 {
                if raw_fd < 0 {
                    return Err(ProcessInstanceError::MalformedResponse);
                }
                // SAFETY: successful SO_PEERPIDFD installs one new descriptor
                // in this process. Wrap it immediately so all error paths close it.
                let pidfd = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw_fd) };
                if length as usize != size_of::<libc::c_int>() {
                    return Err(ProcessInstanceError::MalformedResponse);
                }
                return Ok(std::sync::Arc::new(LinuxPidfdHandle::Peer(pidfd)));
            }
            let source = io::Error::last_os_error();
            if source.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(ProcessInstanceError::KernelRead {
                option: ProcessInstanceOption::PeerPidfd,
                source,
            });
        }
    }

    fn terminate_and_reap_unregistered(
        &self,
        handle: &Self::Handle,
    ) -> Result<(), ProcessInstanceError> {
        #[cfg(feature = "linux-runtime-fixture")]
        if matches!(handle.as_ref(), LinuxPidfdHandle::Task41FixtureSpawn(_)) {
            return terminate_and_reap_pidfd(handle).map_err(|source| {
                ProcessInstanceError::KernelRead {
                    option: ProcessInstanceOption::SpawnPidfdLiveness,
                    source,
                }
            });
        }
        let _ = handle;
        Err(ProcessInstanceError::Unavailable)
    }

    fn terminate_and_reap_untracked(&self, pid: u32) -> Result<(), ProcessInstanceError> {
        terminate_and_reap_untracked_child(pid).map_err(|source| ProcessInstanceError::KernelRead {
            option: ProcessInstanceOption::SpawnCleanup,
            source,
        })
    }

    fn confirms_same_live_instance(
        &self,
        spawned: &Self::Handle,
        peer: &Self::Handle,
        spawned_pid: u32,
        peer_pid: u32,
    ) -> Result<bool, ProcessInstanceError> {
        if spawned_pid != peer_pid {
            return Ok(false);
        }
        if !pidfd_is_live(spawned, ProcessInstanceOption::SpawnPidfdLiveness)?
            || !pidfd_is_live(peer, ProcessInstanceOption::PeerPidfdLiveness)?
        {
            return Ok(false);
        }
        // The child PID from clone3 and the SO_PEERCRED PID are both visible in
        // PID 1's active namespace. The spawn pidfd is returned atomically with
        // its PID; SO_PEERPIDFD refers to the socket's saved kernel peer PID.
        // Two live process handles with the same PID in this namespace denote
        // the same instance. A stale socket snapshot for a reused PID has an
        // exited peer pidfd and is rejected above.
        Ok(true)
    }
}

#[cfg(target_os = "linux")]
fn pidfd_is_live(
    pidfd: &std::sync::Arc<LinuxPidfdHandle>,
    option: ProcessInstanceOption,
) -> Result<bool, ProcessInstanceError> {
    let fd = pidfd
        .as_raw_fd()
        .map_err(|source| ProcessInstanceError::KernelRead { option, source })?;
    loop {
        let mut poll_fd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `poll_fd` points to one initialized pollfd and uses a zero
        // timeout, so this liveness probe is nonblocking.
        let result = unsafe { libc::poll(&mut poll_fd, 1, 0) };
        if result < 0 {
            let source = io::Error::last_os_error();
            if source.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(ProcessInstanceError::KernelRead { option, source });
        }
        if result == 0 {
            return Ok(true);
        }
        if poll_fd.revents & libc::POLLNVAL != 0 {
            return Err(ProcessInstanceError::KernelRead {
                option,
                source: io::Error::from_raw_os_error(libc::EBADF),
            });
        }
        if poll_fd.revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0 {
            return Ok(false);
        }
        return Err(ProcessInstanceError::MalformedResponse);
    }
}

#[cfg(all(target_os = "linux", feature = "linux-runtime-fixture"))]
fn terminate_and_reap_pidfd(pidfd: &std::sync::Arc<LinuxPidfdHandle>) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    let raw_fd = match pidfd.as_ref() {
        LinuxPidfdHandle::Task41FixtureSpawn(fd) => fd.as_raw_fd(),
        LinuxPidfdHandle::Task24Spawn(_) | LinuxPidfdHandle::Peer(_) => {
            unreachable!("only the feature-gated fixture spawn handle is reaped here")
        }
    };

    loop {
        // SAFETY: the signal is directed by the owned pidfd and has no siginfo.
        let result = unsafe {
            libc::syscall(
                libc::SYS_pidfd_send_signal,
                raw_fd,
                libc::SIGKILL,
                std::ptr::null::<libc::siginfo_t>(),
                0,
            )
        };
        if result == 0 {
            break;
        }
        let source = io::Error::last_os_error();
        match source.raw_os_error() {
            Some(libc::EINTR) => continue,
            // The child can exit between capture failure and cleanup.
            Some(libc::ESRCH) => break,
            _ => return Err(source),
        }
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        // SAFETY: siginfo is initialized and waitid targets the owned child
        // through P_PIDFD. WNOHANG bounds each individual wait operation.
        let mut status: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PIDFD,
                raw_fd as libc::id_t,
                &mut status,
                libc::WEXITED | libc::WNOHANG,
            )
        };
        if result == 0 {
            match pidfd_is_live(pidfd, ProcessInstanceOption::SpawnCleanup) {
                Ok(false) => return Ok(()),
                Ok(true) => {}
                Err(ProcessInstanceError::KernelRead { source, .. }) => return Err(source),
                Err(ProcessInstanceError::Unavailable) => {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "pidfd liveness unavailable during cleanup",
                    ));
                }
                Err(ProcessInstanceError::MalformedResponse) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "malformed pidfd poll response during cleanup",
                    ));
                }
            }
        } else {
            let source = io::Error::last_os_error();
            match source.raw_os_error() {
                Some(libc::EINTR) => continue,
                Some(libc::ECHILD) => {
                    if matches!(
                        pidfd_is_live(pidfd, ProcessInstanceOption::SpawnCleanup),
                        Ok(false)
                    ) {
                        return Ok(());
                    }
                    return Err(source);
                }
                _ => return Err(source),
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "pidfd child did not exit within bounded cleanup deadline",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Cleanup-only fallback for a just-created direct child when clone3 reports a
/// positive PID but violates CLONE_PIDFD output. The child is still our
/// unreaped child, so its numeric PID cannot have been reused. This PID is never
/// put into a proof or authentication table.
#[cfg(target_os = "linux")]
fn terminate_and_reap_untracked_child(pid: u32) -> io::Result<()> {
    let pid = libc::pid_t::try_from(pid)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "child PID overflow"))?;
    loop {
        let result = unsafe { libc::kill(pid, libc::SIGKILL) };
        if result == 0 {
            break;
        }
        let source = io::Error::last_os_error();
        match source.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(libc::ESRCH) => break,
            _ => return Err(source),
        }
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        let mut status = 0;
        let result = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if result == pid {
            return Ok(());
        }
        if result < 0 {
            let source = io::Error::last_os_error();
            if source.raw_os_error() != Some(libc::EINTR) {
                return Err(source);
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "untracked clone3 child did not reap within bounded cleanup deadline",
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
