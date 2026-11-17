//! Same-crate bridge from a spawn-owned UnixListener to the existing Task41
//! endpoint registry and live-peer registration path.
//!
//! This module must be included beside `pidfd_process_instance` and the
//! endpoint owner inside the PID-1 crate. Its crate-private API intentionally
//! cannot be used through a separate public Cargo dependency. The Task24
//! SpawnedProcessPidfd handoff now binds the opaque child identity and exact
//! Supervisor record into this candidate; the normal-init callsite and the
//! crate-private ExactChildLaunchLease implementation still require
//! co-location in the boot owner before production installation is available.

use super::child_service_bootstrap::ChildServiceBootstrap;
use super::pid1_authenticated_endpoint_owner::Pid1EndpointOwnerRegistry;
use super::pidfd_process_instance::{PidfdKernel, PidfdProcessInstanceAuthority};
use super::{
    ConsentSource, LaunchRecordView, LiveSessionAuthority, PeerDenial, Pid1LivePeerDirectory,
    Pid1ReadinessAtomicBoundary, Pid1RunInstance, Pid1SpawnEndpointLease, Pid1SpawnProof,
    RegistrationId, SupervisorLaunchAuthority, TrustedInventory, TrustedNipcOperationRoute,
    TrustedServiceRequirements,
};
use nagi_init_spawn_contract::{SpawnedProcessIdentity, SpawnedProcessPidfd};
use std::fs;
#[cfg(target_os = "linux")]
use std::io;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStrExt;
#[cfg(target_os = "linux")]
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
#[cfg(target_os = "linux")]
use std::path::Path;
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_os = "linux")]
const PRIVATE_ENDPOINT_DIRECTORY: &str = "/run/nagi/ipc";
#[cfg(target_os = "linux")]
static NEXT_SOCKET_SLOT: AtomicU64 = AtomicU64::new(1);

/// A listener/path/route transaction prepared by trusted PID-1 spawn wiring.
/// The listener is non-cloneable, and its argv locator is derived from that
/// listener's bound address. The locator does not authorize the child.
pub(crate) struct PendingSpawnOwnedEndpoint {
    listener: UnixListener,
    socket_path: SocketPathCleanup,
    route: TrustedNipcOperationRoute,
    bootstrap: ChildServiceBootstrap,
}

impl PendingSpawnOwnedEndpoint {
    /// Create one PID-1-owned AF_UNIX listener for the exact upcoming child.
    /// The private directory must already exist and be root-owned/non-writable
    /// by other users. A bind error is returned once; there is no retry,
    /// alternate path, or unlink-before-bind fallback.
    #[cfg(target_os = "linux")]
    pub(crate) fn bind_for_spawn(
        run: &Pid1RunInstance,
        route: TrustedNipcOperationRoute,
        child_uid: u32,
        child_gid: u32,
    ) -> io::Result<Self> {
        ensure_private_endpoint_directory(Path::new(PRIVATE_ENDPOINT_DIRECTORY))?;
        if !run.is_active() || child_uid == 0 || child_gid == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "inactive run or privileged child identity",
            ));
        }
        let slot = NEXT_SOCKET_SLOT
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| io::Error::other("endpoint socket sequence exhausted"))?;
        let socket_path = PathBuf::from(format!(
            "{PRIVATE_ENDPOINT_DIRECTORY}/r{:016x}-e{slot:016x}.sock",
            run.generation.get()
        ));

        // Exactly one bind attempt. Stale or colliding names stay a hard
        // failure; the code does not try another port/path or unlink anything.
        let listener = UnixListener::bind(&socket_path)?;
        let path_cleanup = SocketPathCleanup::new(socket_path.clone());
        let configure = (|| {
            fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
            let path =
                std::ffi::CString::new(socket_path.as_os_str().as_bytes()).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "socket path contains NUL")
                })?;
            // SAFETY: `path` is NUL terminated and points at the socket we
            // just bound under a directory verified as non-writable to peers.
            let result = unsafe { libc::chown(path.as_ptr(), child_uid, child_gid) };
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        })();
        if let Err(error) = configure {
            drop(listener);
            drop(path_cleanup);
            return Err(error);
        }
        let bootstrap = match ChildServiceBootstrap::from_listener(&listener) {
            Ok(bootstrap) => bootstrap,
            Err(_) => {
                drop(listener);
                drop(path_cleanup);
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "bound endpoint did not produce a valid child locator",
                ));
            }
        };
        Ok(Self {
            listener,
            socket_path: path_cleanup,
            route,
            bootstrap,
        })
    }

    /// Append this argument to the existing `ProcessLaunch.arguments` before
    /// `ProcessOs::prepare`. It conveys only the listening address.
    pub(crate) fn child_argument(&self) -> &str {
        self.bootstrap.argument()
    }

    /// Build the exact argv slice consumed by the existing `ProcessOs::prepare`
    /// path. Reject a caller-supplied endpoint flag so there is one owner-made
    /// locator and the child parser cannot be steered to a second socket.
    pub(crate) fn child_arguments(
        &self,
        application_arguments: &[&str],
    ) -> Result<Vec<String>, super::child_service_bootstrap::EndpointBootstrapError> {
        append_child_endpoint_argument(&self.bootstrap, application_arguments)
    }

    /// Consume the one-use process proof and this exact listener before the
    /// exec gate is released. If registration fails, owned endpoint state drops
    /// and revokes; callers must not release the child.
    pub(crate) fn install_before_exec_gate<R, K, A>(
        self,
        run: &Pid1RunInstance,
        spawn: Pid1SpawnProof<R>,
        process_instances: &PidfdProcessInstanceAuthority<K>,
        endpoint_owner: &Pid1EndpointOwnerRegistry,
        supervisor: &A,
    ) -> Result<InstalledSpawnEndpoint<R>, PeerDenial>
    where
        R: LaunchRecordView,
        K: PidfdKernel,
        A: SupervisorLaunchAuthority<Record = R>,
    {
        let Self {
            listener,
            socket_path,
            route,
            bootstrap,
        } = self;
        let exact_identity = spawn
            .task24_identity()
            .cloned()
            .ok_or(PeerDenial::ForeignSpawnOwner)?;
        if exact_identity.process_id() != spawn.process.pid()
            || spawn.exact_launch_record().process_id() != spawn.process.pid()
        {
            return Err(PeerDenial::ForeignSpawnOwner);
        }
        let binding =
            process_instances.bind_spawn_owned_endpoint(spawn, route, listener, supervisor)?;
        let (lease, same_spawn_proof) =
            endpoint_owner.install_from_spawn(run, binding, process_instances, supervisor)?;
        Ok(InstalledSpawnEndpoint {
            run: run.clone(),
            proof: same_spawn_proof,
            lifetime: EndpointLifetime::new(lease, socket_path),
            bootstrap,
        })
    }
}

/// Endpoint installation has succeeded, but live service registration still
/// requires the actual inventory/session/consent/atomic-gate suppliers.
pub(crate) struct InstalledSpawnEndpoint<R: LaunchRecordView> {
    run: Pid1RunInstance,
    proof: Pid1SpawnProof<R>,
    lifetime: EndpointLifetime,
    bootstrap: ChildServiceBootstrap,
}

impl<R: LaunchRecordView> InstalledSpawnEndpoint<R> {
    pub(crate) fn child_argument(&self) -> &str {
        self.bootstrap.argument()
    }

    pub(crate) fn child_arguments(
        &self,
        application_arguments: &[&str],
    ) -> Result<Vec<String>, super::child_service_bootstrap::EndpointBootstrapError> {
        append_child_endpoint_argument(&self.bootstrap, application_arguments)
    }

    /// Register through the existing peer owner. The current endpoint owner
    /// reports FD delegation as unprevented, so registration rejects before
    /// publishing a live entry.
    pub(crate) fn register_before_exec_gate<I, S, C, A, B>(
        self,
        directory: &Pid1LivePeerDirectory<R>,
        requirements: &TrustedServiceRequirements<'_>,
        inventory: &I,
        sessions: &S,
        consent: &C,
        supervisor: &A,
        readiness: &B,
    ) -> Result<SpawnEndpointReapLease<R>, EndpointRegistrationFailure<R>>
    where
        I: TrustedInventory,
        S: LiveSessionAuthority,
        C: ConsentSource,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
    {
        let Self {
            run,
            proof,
            lifetime,
            bootstrap: _,
        } = self;
        let record = proof.exact_launch_record();
        let process = lifetime.lease().process_instance();
        let Some(spawn_identity) = proof.task24_identity().cloned() else {
            return Err(EndpointRegistrationFailure::new(
                PeerDenial::ForeignSpawnOwner,
                &run,
                process,
                record,
                None,
                None,
                lifetime,
            ));
        };
        if spawn_identity.process_id() != record.process_id()
            || spawn_identity.process_id() != process.pid()
        {
            return Err(EndpointRegistrationFailure::new(
                PeerDenial::ForeignSpawnOwner,
                &run,
                process,
                record,
                None,
                None,
                lifetime,
            ));
        }
        let registration = match directory.register_pid1_spawn(
            &run,
            proof,
            requirements,
            lifetime.lease().endpoint(),
            inventory,
            sessions,
            consent,
            lifetime.lease(),
            supervisor,
            readiness,
        ) {
            Ok(registration) => registration,
            Err(error) => {
                return Err(EndpointRegistrationFailure::new(
                    error, &run, process, record, None, None, lifetime,
                ));
            }
        };
        let retirement_pending = match directory.retirement_latch(registration, process) {
            Ok(retirement_pending) => retirement_pending,
            Err(error) => {
                return Err(EndpointRegistrationFailure::new(
                    error,
                    &run,
                    process,
                    record,
                    Some(registration),
                    None,
                    lifetime,
                ));
            }
        };
        let (lease, socket_path) = lifetime.into_parts();
        Ok(SpawnEndpointReapLease {
            run,
            process,
            spawn_identity,
            record,
            registration,
            retirement_pending,
            lease,
            socket_path,
            retirement_state: ReapLeaseRetirementState::NotAttempted,
        })
    }
}

/// Error result that retains the installed endpoint owner until Task24's
/// exact reap. A registration failure after endpoint installation must not
/// drop the only cleanup owner before the child lifecycle resolves.
pub(crate) struct EndpointRegistrationFailure<R: LaunchRecordView> {
    reason: PeerDenial,
    run: Pid1RunInstance,
    process: super::ProcessInstanceKey,
    record: R,
    registration: Option<RegistrationId>,
    retirement_pending: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    lifetime: EndpointLifetime,
}

impl<R: LaunchRecordView> EndpointRegistrationFailure<R> {
    fn new(
        reason: PeerDenial,
        run: &Pid1RunInstance,
        process: super::ProcessInstanceKey,
        record: R,
        registration: Option<RegistrationId>,
        retirement_pending: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        lifetime: EndpointLifetime,
    ) -> Self {
        Self {
            reason,
            run: run.clone(),
            process,
            record,
            registration,
            retirement_pending,
            lifetime,
        }
    }

    fn retire_after_exact_child_reap<B: Pid1ReadinessAtomicBoundary>(
        &mut self,
        process_instance: &SpawnedProcessIdentity,
        record: R,
        directory: &Pid1LivePeerDirectory<R>,
        readiness: &B,
    ) -> Result<(), PeerDenial> {
        if process_instance.process_id() != self.process.pid()
            || self.record != record
            || self.record.process_id() != process_instance.process_id()
        {
            return Err(PeerDenial::ForeignSpawnOwner);
        }
        if let Some(registration) = self.registration {
            let retirement_pending = match self.retirement_pending.as_ref() {
                Some(pending) => std::sync::Arc::clone(pending),
                None => directory.retirement_latch(registration, self.process)?,
            };
            super::mark_endpoint_retirement_pending(&retirement_pending);
            match directory.retire_after_supervisor_revoke(
                &self.run,
                self.process,
                registration,
                readiness,
            )? {
                true => {}
                false => return Err(PeerDenial::StaleRegistration),
            }
        }
        self.lifetime.revoke_after_exact_child_reap()
    }
}

/// The value returned by Task24's pre-gate installer even when endpoint
/// installation or registration fails. Task24 must own this exact, non-Clone
/// value before the authorization callback can return. The failed result is
/// retained so the callback can refuse `gate.commit()` while the sole reaper
/// still has the Task41 child-instance state needed for exact retirement.
///
/// This is a same-crate co-location type. Its constructors and fields are
/// private; no caller can create an accepted child identity from a PID.
pub(crate) struct Task24EndpointLaunchLease<R: LaunchRecordView> {
    spawn_identity: SpawnedProcessIdentity,
    record: R,
    endpoint: Task41EndpointInstallOutcome<R>,
    retirement_state: ReapLeaseRetirementState,
}

enum Task41EndpointInstallOutcome<R: LaunchRecordView> {
    Registered(SpawnEndpointReapLease<R>),
    RegistrationFailed(EndpointRegistrationFailure<R>),
    Rejected(PeerDenial),
}

fn task24_spawn_binding_matches<R: LaunchRecordView>(
    stored_identity: &SpawnedProcessIdentity,
    stored_record: R,
    process_instance: &SpawnedProcessIdentity,
    record: &R,
) -> bool {
    stored_identity.same_instance(process_instance)
        && stored_identity.process_id() == stored_record.process_id()
        && stored_record == *record
        && record.process_id() == process_instance.process_id()
}

impl<R: LaunchRecordView> Task24EndpointLaunchLease<R> {
    /// Consume the Task24-issued pidfd token and the listener prepared for the
    /// same child. Every failure is returned inside the lease rather than
    /// escaping the Task24 installer before `adopt_exact_child_lease` runs.
    #[allow(clippy::too_many_arguments)]
    #[cfg(target_os = "linux")]
    pub(crate) fn install_and_register<I, S, C, A, B>(
        pending: PendingSpawnOwnedEndpoint,
        spawn: SpawnedProcessPidfd,
        record: R,
        run: &Pid1RunInstance,
        process_instances: &PidfdProcessInstanceAuthority<
            super::pidfd_process_instance::LinuxPidfdKernel,
        >,
        endpoint_owner: &Pid1EndpointOwnerRegistry,
        directory: &Pid1LivePeerDirectory<R>,
        requirements: &TrustedServiceRequirements<'_>,
        inventory: &I,
        sessions: &S,
        consent: &C,
        supervisor: &A,
        readiness: &B,
    ) -> Self
    where
        I: TrustedInventory,
        S: LiveSessionAuthority,
        C: ConsentSource,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
    {
        let spawn_identity = spawn.identity();
        let endpoint = match process_instances.capture_task24_spawn_proof(spawn, record) {
            Err(error) => Task41EndpointInstallOutcome::Rejected(error),
            Ok(proof) => match pending.install_before_exec_gate(
                run,
                proof,
                process_instances,
                endpoint_owner,
                supervisor,
            ) {
                Err(error) => Task41EndpointInstallOutcome::Rejected(error),
                Ok(installed) => match installed.register_before_exec_gate(
                    directory,
                    requirements,
                    inventory,
                    sessions,
                    consent,
                    supervisor,
                    readiness,
                ) {
                    Ok(endpoint) => Task41EndpointInstallOutcome::Registered(endpoint),
                    Err(failure) => Task41EndpointInstallOutcome::RegistrationFailed(failure),
                },
            },
        };
        Self {
            spawn_identity,
            record,
            endpoint,
            retirement_state: ReapLeaseRetirementState::NotAttempted,
        }
    }

    /// Required immediately after Task24 has adopted this lease and before
    /// its caller invokes the Supervisor commit gate. `Err` must be returned
    /// from the authorization callback; it is never converted to success.
    pub(crate) fn install_result(&self) -> Result<(), PeerDenial> {
        match &self.endpoint {
            Task41EndpointInstallOutcome::Registered(_) => Ok(()),
            Task41EndpointInstallOutcome::RegistrationFailed(failure) => {
                Err(failure.reason.clone())
            }
            Task41EndpointInstallOutcome::Rejected(error) => Err(error.clone()),
        }
    }

    pub(crate) fn matches_spawn(
        &self,
        process_instance: &SpawnedProcessIdentity,
        record: &R,
    ) -> bool {
        task24_spawn_binding_matches(&self.spawn_identity, self.record, process_instance, record)
    }

    /// Task24 calls this only after exact child wait and Supervisor revoke.
    /// Successful endpoint registration retires through its full Task41
    /// registration/endpoint path. If registration never completed, remove
    /// only this exact Task24-associated Task41 pidfd duplicate; absence is
    /// Unknown, not success, because a failed capture may have stopped midway.
    pub(crate) fn retire_after_exact_child_reap<K, B>(
        &mut self,
        process_instance: &SpawnedProcessIdentity,
        record: R,
        directory: &Pid1LivePeerDirectory<R>,
        process_instances: &PidfdProcessInstanceAuthority<K>,
        readiness: &B,
    ) -> Result<bool, PeerDenial>
    where
        K: PidfdKernel,
        B: Pid1ReadinessAtomicBoundary,
    {
        if self.retirement_state != ReapLeaseRetirementState::NotAttempted {
            return Err(PeerDenial::StaleRegistration);
        }
        self.retirement_state = ReapLeaseRetirementState::Attempted;
        if !self.matches_spawn(process_instance, &record) {
            self.retirement_state = ReapLeaseRetirementState::Unknown;
            return Err(PeerDenial::ForeignSpawnOwner);
        }

        let retired = match &mut self.endpoint {
            Task41EndpointInstallOutcome::Registered(endpoint) => endpoint
                .retire_after_exact_child_reap(
                    process_instance,
                    record,
                    directory,
                    process_instances,
                    readiness,
                ),
            Task41EndpointInstallOutcome::RegistrationFailed(failure) => failure
                .retire_after_exact_child_reap(process_instance, record, directory, readiness)
                .and_then(|()| {
                    match process_instances.retire_task24_spawn_after_exact_reap(process_instance) {
                        Ok(true) => Ok(true),
                        Ok(false) => Err(PeerDenial::ProcessInstanceUnavailable),
                        Err(error) => Err(error),
                    }
                }),
            Task41EndpointInstallOutcome::Rejected(_) => {
                match process_instances.retire_task24_spawn_after_exact_reap(process_instance) {
                    Ok(true) => Ok(true),
                    Ok(false) => Err(PeerDenial::ProcessInstanceUnavailable),
                    Err(error) => Err(error),
                }
            }
        };
        match retired {
            Ok(removed) => {
                self.retirement_state = ReapLeaseRetirementState::Complete;
                Ok(removed)
            }
            Err(error) => {
                self.retirement_state = ReapLeaseRetirementState::Unknown;
                Err(error)
            }
        }
    }
}

impl<R: LaunchRecordView> std::fmt::Debug for Task24EndpointLaunchLease<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Task24EndpointLaunchLease")
            .field("process_id", &self.spawn_identity.process_id())
            .field(
                "endpoint_registered",
                &matches!(&self.endpoint, Task41EndpointInstallOutcome::Registered(_)),
            )
            .field("retirement_state", &self.retirement_state)
            .finish_non_exhaustive()
    }
}

fn append_child_endpoint_argument(
    bootstrap: &ChildServiceBootstrap,
    application_arguments: &[&str],
) -> Result<Vec<String>, super::child_service_bootstrap::EndpointBootstrapError> {
    if application_arguments
        .iter()
        .any(|argument| argument.starts_with("--nagi-service-endpoint"))
    {
        return Err(super::child_service_bootstrap::EndpointBootstrapError::DuplicateEndpoint);
    }
    let mut arguments: Vec<String> = application_arguments
        .iter()
        .map(|argument| (*argument).to_owned())
        .collect();
    arguments.push(bootstrap.argument().to_owned());
    Ok(arguments)
}

/// Non-cloneable endpoint/registration lease to store in the exact child
/// lifecycle record before `GateCommit::commit`. Dropping it revokes the
/// endpoint lease. The exact Task24 reaper must remove the peer registration
/// after successful wait and Supervisor-record revoke, then drop this value.
pub(crate) struct SpawnEndpointReapLease<R: LaunchRecordView> {
    run: Pid1RunInstance,
    process: super::ProcessInstanceKey,
    spawn_identity: nagi_init_spawn_contract::SpawnedProcessIdentity,
    record: R,
    registration: RegistrationId,
    retirement_pending: std::sync::Arc<std::sync::atomic::AtomicBool>,
    lease: Pid1SpawnEndpointLease,
    socket_path: SocketPathCleanup,
    retirement_state: ReapLeaseRetirementState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReapLeaseRetirementState {
    NotAttempted,
    Attempted,
    Complete,
    Unknown,
}

fn exact_spawn_binding_matches<R: LaunchRecordView>(
    stored_identity: &nagi_init_spawn_contract::SpawnedProcessIdentity,
    stored_record: R,
    process_pid: u32,
    supplied_identity: &nagi_init_spawn_contract::SpawnedProcessIdentity,
    supplied_record: &R,
) -> bool {
    stored_identity.same_instance(supplied_identity)
        && stored_identity.process_id() == process_pid
        && stored_record == *supplied_record
        && stored_record.process_id() == process_pid
}

fn begin_exact_retirement(state: &mut ReapLeaseRetirementState) -> bool {
    if *state != ReapLeaseRetirementState::NotAttempted {
        return false;
    }
    *state = ReapLeaseRetirementState::Attempted;
    true
}

impl<R: LaunchRecordView> SpawnEndpointReapLease<R> {
    pub(crate) fn endpoint(&self) -> super::TrustedServiceEndpoint {
        self.lease.endpoint()
    }

    /// ExactChildLaunchLease's predicate, exposed as an inherent method so the
    /// boot crate can implement its crate-private trait when this module is
    /// co-located there. PID equality is necessary but never sufficient.
    pub(crate) fn matches_spawn(
        &self,
        process_instance: &nagi_init_spawn_contract::SpawnedProcessIdentity,
        record: &R,
    ) -> bool {
        exact_spawn_binding_matches(
            &self.spawn_identity,
            self.record,
            self.process.pid(),
            process_instance,
            record,
        )
    }

    /// Call only from Task24's exact-child reap path, after its wait and
    /// existing Supervisor revoke succeeded. This candidate cannot itself
    /// certify a Linux wait result or implement Task24's crate-private trait
    /// across this separate candidate-crate boundary.
    pub(crate) fn retire_after_exact_child_reap<K, B>(
        &mut self,
        process_instance: &nagi_init_spawn_contract::SpawnedProcessIdentity,
        record: R,
        directory: &Pid1LivePeerDirectory<R>,
        process_instances: &PidfdProcessInstanceAuthority<K>,
        readiness: &B,
    ) -> Result<bool, PeerDenial>
    where
        K: PidfdKernel,
        B: Pid1ReadinessAtomicBoundary,
    {
        if !begin_exact_retirement(&mut self.retirement_state) {
            return Err(PeerDenial::StaleRegistration);
        }
        if !self.matches_spawn(process_instance, &record) {
            self.retirement_state = ReapLeaseRetirementState::Unknown;
            super::mark_endpoint_retirement_pending(&self.retirement_pending);
            return Err(PeerDenial::ForeignSpawnOwner);
        }
        super::mark_endpoint_retirement_pending(&self.retirement_pending);
        let removed = match directory.retire_after_supervisor_revoke(
            &self.run,
            self.process,
            self.registration,
            readiness,
        ) {
            Ok(true) => true,
            Ok(false) => {
                self.retirement_state = ReapLeaseRetirementState::Unknown;
                return Err(PeerDenial::StaleRegistration);
            }
            Err(error) => {
                self.retirement_state = ReapLeaseRetirementState::Unknown;
                return Err(error);
            }
        };
        if let Err(error) = self.lease.revoke_once_for_owner_retirement() {
            self.retirement_state = ReapLeaseRetirementState::Unknown;
            return Err(error);
        }
        match process_instances.retire_task24_spawn_after_exact_reap(process_instance) {
            Ok(true) => {
                self.retirement_state = ReapLeaseRetirementState::Complete;
                Ok(removed)
            }
            Ok(false) => {
                self.retirement_state = ReapLeaseRetirementState::Unknown;
                Err(PeerDenial::ProcessInstanceUnavailable)
            }
            Err(error) => {
                self.retirement_state = ReapLeaseRetirementState::Unknown;
                Err(error)
            }
        }
    }
}

impl<R: LaunchRecordView> Drop for SpawnEndpointReapLease<R> {
    fn drop(&mut self) {
        match self.retirement_state {
            ReapLeaseRetirementState::Complete => {}
            ReapLeaseRetirementState::NotAttempted => {
                // Abandonment invalidates authorization immediately, but only
                // Task24 exact reap may remove the directory slot.
                super::mark_endpoint_retirement_pending(&self.retirement_pending);
                let _ = self.lease.revoke_once_for_owner_retirement();
            }
            ReapLeaseRetirementState::Attempted | ReapLeaseRetirementState::Unknown => {
                // Preserve Task24's one-shot Unknown result. Do not repeat an
                // endpoint/reaper action after a possibly partial outcome.
                super::mark_endpoint_retirement_pending(&self.retirement_pending);
            }
        }
    }
}

struct EndpointLifetime {
    lease: Option<Pid1SpawnEndpointLease>,
    socket_path: Option<SocketPathCleanup>,
}

impl EndpointLifetime {
    fn new(lease: Pid1SpawnEndpointLease, socket_path: SocketPathCleanup) -> Self {
        Self {
            lease: Some(lease),
            socket_path: Some(socket_path),
        }
    }

    fn lease(&self) -> &Pid1SpawnEndpointLease {
        self.lease
            .as_ref()
            .expect("endpoint lifetime owns its lease")
    }

    fn into_parts(mut self) -> (Pid1SpawnEndpointLease, SocketPathCleanup) {
        let lease = self.lease.take().expect("endpoint lifetime owns its lease");
        let socket_path = self
            .socket_path
            .take()
            .expect("endpoint lifetime owns its socket path");
        (lease, socket_path)
    }

    fn revoke_after_exact_child_reap(&mut self) -> Result<(), PeerDenial> {
        let lease = self.lease.as_mut().ok_or(PeerDenial::StaleEndpoint)?;
        lease.revoke_once_for_owner_retirement()?;
        // Drop the already-revoked owner before its exact pathname guard.
        self.lease.take();
        self.socket_path.take();
        Ok(())
    }
}

impl Drop for EndpointLifetime {
    fn drop(&mut self) {
        // Keep teardown ordered: close/revoke the endpoint first, then unlink
        // its exact owned pathname. Never unlink a path before acquiring its
        // listener and cleanup owner.
        self.lease.take();
        self.socket_path.take();
    }
}

struct SocketPathCleanup {
    path: PathBuf,
}

impl SocketPathCleanup {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for SocketPathCleanup {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestRecord(u32, u64);

    impl LaunchRecordView for TestRecord {
        fn process_id(self) -> u32 {
            self.0
        }

        fn app_id(self) -> u64 {
            1
        }

        fn app_session_id(self) -> u64 {
            1
        }
    }

    #[test]
    fn exact_child_binding_rejects_pid_reuse_and_record_replacement() {
        let stored_identity = nagi_init_spawn_contract::test_identity(41);
        let reused_pid_identity = nagi_init_spawn_contract::test_identity(41);
        let stored_record = TestRecord(41, 7);

        assert!(exact_spawn_binding_matches(
            &stored_identity,
            stored_record,
            41,
            &stored_identity,
            &stored_record,
        ));
        assert!(!exact_spawn_binding_matches(
            &stored_identity,
            stored_record,
            41,
            &reused_pid_identity,
            &stored_record,
        ));
        assert!(!exact_spawn_binding_matches(
            &stored_identity,
            stored_record,
            41,
            &stored_identity,
            &TestRecord(41, 8),
        ));
        assert!(!exact_spawn_binding_matches(
            &stored_identity,
            stored_record,
            42,
            &stored_identity,
            &stored_record,
        ));
    }

    #[test]
    fn task24_launch_lease_binding_rejects_reused_pid_and_wrong_record() {
        let original = nagi_init_spawn_contract::test_identity(43);
        let reused_pid = nagi_init_spawn_contract::test_identity(43);
        let record = TestRecord(43, 9);

        assert!(task24_spawn_binding_matches(
            &original, record, &original, &record,
        ));
        assert!(!task24_spawn_binding_matches(
            &original,
            record,
            &reused_pid,
            &record,
        ));
        assert!(!task24_spawn_binding_matches(
            &original,
            record,
            &original,
            &TestRecord(43, 10),
        ));
    }

    #[cfg(feature = "task24-owner-co-location")]
    #[test]
    fn actual_task24_owner_trait_matches_task41_spawn_nonce_and_record() {
        use super::super::task24_owner_co_location::ExactChildLaunchLease;
        let original = nagi_init_spawn_contract::test_identity(81);
        let reused = nagi_init_spawn_contract::test_identity(81);
        let record = TestRecord(81, 17);
        let lease = Task24EndpointLaunchLease {
            spawn_identity: original.clone(),
            record,
            endpoint: Task41EndpointInstallOutcome::Rejected(PeerDenial::Unavailable),
            retirement_state: ReapLeaseRetirementState::NotAttempted,
        };
        assert!(<Task24EndpointLaunchLease<TestRecord> as ExactChildLaunchLease<
            TestRecord,
        >>::matches_spawn(&lease, &original, &record));
        assert!(!<Task24EndpointLaunchLease<TestRecord> as ExactChildLaunchLease<
            TestRecord,
        >>::matches_spawn(&lease, &reused, &record));
        assert!(!<Task24EndpointLaunchLease<TestRecord> as ExactChildLaunchLease<
            TestRecord,
        >>::matches_spawn(&lease, &original, &TestRecord(81, 18)));
    }

    #[test]
    fn exact_child_retirement_state_is_one_shot_after_unknown_result() {
        let mut state = ReapLeaseRetirementState::NotAttempted;
        assert!(begin_exact_retirement(&mut state));
        assert_eq!(state, ReapLeaseRetirementState::Attempted);
        assert!(!begin_exact_retirement(&mut state));
        state = ReapLeaseRetirementState::Unknown;
        assert!(!begin_exact_retirement(&mut state));
        state = ReapLeaseRetirementState::Complete;
        assert!(!begin_exact_retirement(&mut state));
    }

    #[test]
    fn only_owner_appended_locator_enters_process_argv() {
        let bootstrap = ChildServiceBootstrap::from_args(&[OsString::from(
            "--nagi-service-endpoint-v1=/run/nagi/ipc/run.sock",
        )])
        .expect("well-formed argument")
        .expect("endpoint present");
        let argv = append_child_endpoint_argument(&bootstrap, &["--mode=service", "--safe"])
            .expect("append owner locator");
        assert_eq!(
            argv,
            [
                "--mode=service",
                "--safe",
                "--nagi-service-endpoint-v1=/run/nagi/ipc/run.sock"
            ]
        );
        assert_eq!(
            append_child_endpoint_argument(
                &bootstrap,
                &["--nagi-service-endpoint-v1=/run/nagi/ipc/attacker.sock"]
            ),
            Err(super::super::child_service_bootstrap::EndpointBootstrapError::DuplicateEndpoint)
        );
        assert_eq!(
            append_child_endpoint_argument(
                &bootstrap,
                &["--nagi-service-endpoint-v2=/run/nagi/ipc/future.sock"]
            ),
            Err(super::super::child_service_bootstrap::EndpointBootstrapError::DuplicateEndpoint)
        );
    }
}

#[cfg(target_os = "linux")]
fn ensure_private_endpoint_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_dir()
        || metadata.uid() != 0
        || metadata.mode() & 0o022 != 0
        // The unprivileged child must be able to traverse to its socket. The
        // directory may be world-readable/searchable, but not writable.
        || metadata.mode() & 0o001 == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "PID 1 endpoint directory must be a real root-owned non-writable directory",
        ));
    }
    Ok(())
}
