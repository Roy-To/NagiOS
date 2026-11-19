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
    ConsentSource, LaunchRecordView, LiveSessionAuthority, PeerDenial,
    PeerProcessInstanceAuthority, Pid1LivePeerDirectory, Pid1ReadinessAtomicBoundary,
    Pid1RunInstance, Pid1SpawnEndpointLease, Pid1SpawnProof, RegistrationId, SessionLease,
    SupervisorLaunchAuthority, TrustedInventory, TrustedNipcOperationRoute,
    TrustedServiceEndpointAuthority, TrustedServiceRequirements,
};
use nagi_init_spawn_contract::{SpawnedProcessIdentity, SpawnedProcessPidfd};
#[cfg(feature = "system-service-readiness")]
use nagi_system_principal_contract::SystemServiceEndpointOwnerProof;
use std::fs;
#[cfg(target_os = "linux")]
use std::io;
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
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
use std::time::Instant;

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
        // This binds an unlistening socket, enables SO_PASSCRED, installs the
        // child-only pathname permissions, and calls listen last. No accepted
        // connection can therefore predate the current-writer setting.
        let (listener, path_cleanup) =
            bind_spawn_listener_with_passcred(&socket_path, child_uid, child_gid)?;
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

    /// Install the retained listener only after C3 has acknowledged exec and
    /// supplied its borrowed attestation for the exact child.
    fn install_after_exec_ack<R, K, A>(
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

    /// Register the exact spawned child before its first Session exists. The
    /// supplied C3 evidence is checked against the proof retained with this
    /// listener before the endpoint can report FD non-delegation as prevented.
    fn register_pre_session_after_attestation<I, C, A, B>(
        self,
        attested_process: &SpawnedProcessIdentity,
        directory: &Pid1LivePeerDirectory<R>,
        requirements: &TrustedServiceRequirements<'_>,
        inventory: &I,
        consent: &C,
        supervisor: &A,
        readiness: &B,
    ) -> Result<SpawnEndpointReapLease<R>, EndpointRegistrationFailure<R>>
    where
        I: TrustedInventory,
        C: ConsentSource,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
    {
        let Some(expected_identity) = self.proof.task24_identity() else {
            return Err(self.registration_failure(PeerDenial::ForeignSpawnOwner));
        };
        if !expected_identity.same_instance(attested_process)
            || expected_identity.process_id() != self.proof.process.pid()
        {
            return Err(self.registration_failure(PeerDenial::ForeignSpawnOwner));
        }
        if let Err(error) = self
            .lifetime
            .lease()
            .record_fd_delegation_attestation(attested_process)
        {
            return Err(self.registration_failure(error));
        }

        self.register_after_attestation_with(directory, |directory, run, proof, endpoint| {
            directory.register_pid1_spawn_pre_session(
                run,
                proof,
                requirements,
                endpoint.endpoint(),
                inventory,
                consent,
                endpoint,
                supervisor,
                readiness,
            )
        })
    }

    fn registration_failure(self, reason: PeerDenial) -> EndpointRegistrationFailure<R> {
        let Self {
            run,
            proof,
            lifetime,
            bootstrap: _,
        } = self;
        EndpointRegistrationFailure::new(
            reason,
            &run,
            lifetime.lease().process_instance(),
            proof.exact_launch_record(),
            None,
            None,
            lifetime,
        )
    }

    fn register_after_attestation_with(
        self,
        directory: &Pid1LivePeerDirectory<R>,
        register: impl FnOnce(
            &Pid1LivePeerDirectory<R>,
            &Pid1RunInstance,
            Pid1SpawnProof<R>,
            &Pid1SpawnEndpointLease,
        ) -> Result<RegistrationId, PeerDenial>,
    ) -> Result<SpawnEndpointReapLease<R>, EndpointRegistrationFailure<R>> {
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
        let registration = match register(directory, &run, proof, lifetime.lease()) {
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
/// Opaque C2 endpoint lease stored beside Task24's exact child lease.
/// External owners may request the first Session from a registered lease, but
/// cannot create, replace, or retarget its spawn, registration, listener, or
/// checkout. Before C3 supplies its live-child FD non-delegation attestation,
/// this value retains only the pending endpoint and exact spawn binding.
pub struct Task24EndpointLaunchLease<R: LaunchRecordView> {
    run: Pid1RunInstance,
    spawn_identity: SpawnedProcessIdentity,
    record: R,
    endpoint: Task41EndpointInstallOutcome<R>,
    retirement_state: ReapLeaseRetirementState,
}

enum Task41EndpointInstallOutcome<R: LaunchRecordView> {
    Pending(PendingTask24Endpoint<R>),
    Registered(SpawnEndpointReapLease<R>),
    RegistrationFailed(EndpointRegistrationFailure<R>),
    Rejected(PeerDenial),
}

/// Listener and one-use C2 spawn proof retained by the Task24 slot before the
/// release gate. No endpoint-owner or live-peer registration exists yet.
struct PendingTask24Endpoint<R: LaunchRecordView> {
    run: Pid1RunInstance,
    pending: Option<PendingSpawnOwnedEndpoint>,
    proof: Option<Pid1SpawnProof<R>>,
}

impl<R: LaunchRecordView> PendingTask24Endpoint<R> {
    fn retire_after_exact_child_reap<K: PidfdKernel>(
        &mut self,
        process_instance: &SpawnedProcessIdentity,
        record: R,
        process_instances: &PidfdProcessInstanceAuthority<K>,
    ) -> Result<bool, PeerDenial> {
        let proof = self.proof.as_ref().ok_or(PeerDenial::StaleRegistration)?;
        let exact_identity = proof
            .task24_identity()
            .ok_or(PeerDenial::ForeignSpawnOwner)?;
        if !exact_identity.same_instance(process_instance)
            || exact_identity.process_id() != proof.process.pid()
            || proof.exact_launch_record() != record
            || record.process_id() != process_instance.process_id()
        {
            return Err(PeerDenial::ForeignSpawnOwner);
        }

        // The caller reaches this only after Task24's exact wait and record
        // revoke. Close/unlink the still-pending listener before removing C2's
        // duplicate pidfd; neither operation is attempted on a mismatch.
        drop(self.pending.take());
        drop(self.proof.take());
        match process_instances.retire_task24_spawn_after_exact_reap(process_instance) {
            Ok(true) => Ok(true),
            Ok(false) => Err(PeerDenial::ProcessInstanceUnavailable),
            Err(error) => Err(error),
        }
    }
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

fn c3_identity_ack_matches_task24_spawn<R: LaunchRecordView>(
    stored_identity: &SpawnedProcessIdentity,
    stored_record: R,
    attested_identity: &SpawnedProcessIdentity,
    attested_record: R,
    c3_attestation_matches_stored_identity: bool,
) -> bool {
    task24_spawn_binding_matches(
        stored_identity,
        stored_record,
        attested_identity,
        &attested_record,
    ) && c3_attestation_matches_stored_identity
}

impl<R: LaunchRecordView> Task24EndpointLaunchLease<R> {
    /// Consume the Task24-issued pidfd token and retain the listener for the
    /// same child before the release gate. This phase deliberately does not
    /// publish a live peer registration: C3 must first provide its opaque
    /// attestation for this exact running child. Every preparation failure is
    /// retained in the lease so Task24 can adopt it before the gate decision.
    #[cfg(target_os = "linux")]
    pub(crate) fn retain_pending_before_exec_gate(
        pending: PendingSpawnOwnedEndpoint,
        spawn: SpawnedProcessPidfd,
        record: R,
        run: &Pid1RunInstance,
        process_instances: &PidfdProcessInstanceAuthority<
            super::pidfd_process_instance::LinuxPidfdKernel,
        >,
    ) -> Self {
        let spawn_identity = spawn.identity();
        let endpoint = match process_instances.capture_task24_spawn_proof(spawn, record) {
            Err(error) => Task41EndpointInstallOutcome::Rejected(error),
            Ok(proof) => Task41EndpointInstallOutcome::Pending(PendingTask24Endpoint {
                run: run.clone(),
                pending: Some(pending),
                proof: Some(proof),
            }),
        };
        Self {
            run: run.clone(),
            spawn_identity,
            record,
            endpoint,
            retirement_state: ReapLeaseRetirementState::NotAttempted,
        }
    }

    /// Confirm that gate-time preparation produced and retained the pending
    /// listener/exact-child binding. This does not claim FD non-delegation or
    /// registered status; callers must not publish service readiness from it.
    pub(crate) fn pending_result(&self) -> Result<(), PeerDenial> {
        match &self.endpoint {
            Task41EndpointInstallOutcome::Pending(pending)
                if pending.run.is_active() && pending.run.is_same_run(&self.run) =>
            {
                Ok(())
            }
            Task41EndpointInstallOutcome::Pending(_) => Err(PeerDenial::StaleRun),
            Task41EndpointInstallOutcome::Registered(_) => {
                Err(PeerDenial::FdDelegationNotPrevented)
            }
            Task41EndpointInstallOutcome::RegistrationFailed(failure) => {
                Err(failure.reason.clone())
            }
            Task41EndpointInstallOutcome::Rejected(error) => Err(error.clone()),
        }
    }

    /// Compare this opaque lease to the active PID-1 run selected by the
    /// caller. Exact child-slot ownership remains the existing Task24 owner's
    /// responsibility.
    pub(crate) fn matches_run(&self, run: &Pid1RunInstance) -> bool {
        self.run.is_active() && self.run.is_same_run(run)
    }

    /// Activate the retained endpoint only while C3 lends its opaque
    /// non-delegation attestation for the exact live child. The sealed adapter
    /// is implemented only for C3's `FdDelegationRestrictionAttestation` in
    /// the co-located Task24 callsite.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn activate_after_fd_delegation_attestation<I, C, A, B, T>(
        &mut self,
        proof: &SpawnedProcessPidfd,
        record: R,
        attestation: &T,
        run: &Pid1RunInstance,
        process_instances: &PidfdProcessInstanceAuthority<
            super::pidfd_process_instance::LinuxPidfdKernel,
        >,
        endpoint_owner: &Pid1EndpointOwnerRegistry,
        directory: &Pid1LivePeerDirectory<R>,
        requirements: &TrustedServiceRequirements<'_>,
        inventory: &I,
        consent: &C,
        supervisor: &A,
        readiness: &B,
    ) -> Result<(), PeerDenial>
    where
        I: TrustedInventory,
        C: ConsentSource,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
        T: super::C3FdDelegationAttestation,
    {
        let proof_identity = proof.identity();
        self.activate_after_c3_identity_exec_ack(
            &proof_identity,
            record,
            attestation,
            run,
            process_instances,
            endpoint_owner,
            directory,
            requirements,
            inventory,
            consent,
            supervisor,
            readiness,
        )
    }

    /// Start (or confirm startup of) the exact child through Task24's owner,
    /// then keep its borrowed slot lease alive until the first checkout ends.
    /// The callback is called once and receives the same absolute deadline
    /// used for acceptance. It does not supply or manufacture FD evidence.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn accept_after_startup_and_borrow_slot<'run, S, C, P, A, B>(
        start_exact_child_and_borrow_slot: impl FnOnce(
            Instant,
        ) -> Result<
            &'run Task24EndpointLaunchLease<R>,
            PeerDenial,
        >,
        run: &Pid1RunInstance,
        directory: &Pid1LivePeerDirectory<R>,
        sessions: &S,
        consent: &C,
        process_instances: &P,
        supervisor: &A,
        readiness: &B,
        deadline: Instant,
    ) -> Result<SessionLease, PeerDenial>
    where
        R: 'run,
        S: LiveSessionAuthority,
        C: ConsentSource,
        P: PeerProcessInstanceAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
    {
        let lease = start_exact_child_and_borrow_slot(deadline)?;
        if !lease.matches_run(run) {
            return Err(PeerDenial::StaleRun);
        }
        lease.accept_initial_session_until(
            directory,
            sessions,
            consent,
            process_instances,
            supervisor,
            readiness,
            deadline,
        )
    }

    /// Accept the first authenticated connection after Task24 releases the
    /// exact child. This routes through the listener and pre-session
    /// registration retained in this lease; callers cannot supply a stream.
    #[allow(clippy::too_many_arguments)]
    pub fn accept_initial_session_until<S, C, P, A, B>(
        &self,
        directory: &Pid1LivePeerDirectory<R>,
        sessions: &S,
        consent: &C,
        process_instances: &P,
        supervisor: &A,
        readiness: &B,
        deadline: Instant,
    ) -> Result<SessionLease, PeerDenial>
    where
        S: LiveSessionAuthority,
        C: ConsentSource,
        P: PeerProcessInstanceAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
    {
        match &self.endpoint {
            Task41EndpointInstallOutcome::Registered(endpoint) => endpoint
                .accept_initial_session_until(
                    directory,
                    sessions,
                    consent,
                    process_instances,
                    supervisor,
                    readiness,
                    deadline,
                ),
            Task41EndpointInstallOutcome::Pending(_) => Err(PeerDenial::FdDelegationNotPrevented),
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

    /// Activate a pending Identity endpoint from C3's independent launch
    /// callback. C3 lends only the shared non-forgeable child identity here;
    /// the caller must pass the same launch record retained before exec. The
    /// sealed attestation adapter verifies C3's exec acknowledgement against
    /// this lease's child. This keeps the C2 implementation independent of
    /// C3's private PID-1 binding and attestation types.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn activate_after_c3_identity_exec_ack<I, C, A, B, T>(
        &mut self,
        process_identity: &SpawnedProcessIdentity,
        record: R,
        attestation: &T,
        run: &Pid1RunInstance,
        process_instances: &PidfdProcessInstanceAuthority<
            super::pidfd_process_instance::LinuxPidfdKernel,
        >,
        endpoint_owner: &Pid1EndpointOwnerRegistry,
        directory: &Pid1LivePeerDirectory<R>,
        requirements: &TrustedServiceRequirements<'_>,
        inventory: &I,
        consent: &C,
        supervisor: &A,
        readiness: &B,
    ) -> Result<(), PeerDenial>
    where
        I: TrustedInventory,
        C: ConsentSource,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
        T: super::C3FdDelegationAttestation,
    {
        if !c3_identity_ack_matches_task24_spawn(
            &self.spawn_identity,
            self.record,
            process_identity,
            record,
            attestation.matches_process(&self.spawn_identity),
        ) {
            return Err(PeerDenial::ForeignSpawnOwner);
        }
        if !self.matches_run(run) {
            return Err(PeerDenial::StaleRun);
        }

        let pending = std::mem::replace(
            &mut self.endpoint,
            Task41EndpointInstallOutcome::Rejected(PeerDenial::Unavailable),
        );
        let mut pending = match pending {
            Task41EndpointInstallOutcome::Pending(pending) => pending,
            other => {
                self.endpoint = other;
                return Err(PeerDenial::StaleRegistration);
            }
        };
        if !pending.run.is_same_run(run) || !pending.run.is_active() {
            self.endpoint = Task41EndpointInstallOutcome::Pending(pending);
            return Err(PeerDenial::StaleRun);
        }
        let (listener, spawn_proof) = match (pending.pending.take(), pending.proof.take()) {
            (Some(listener), Some(spawn_proof)) => (listener, spawn_proof),
            (listener, spawn_proof) => {
                pending.pending = listener;
                pending.proof = spawn_proof;
                self.endpoint = Task41EndpointInstallOutcome::Pending(pending);
                return Err(PeerDenial::StaleRegistration);
            }
        };

        let installed = match listener.install_after_exec_ack(
            run,
            spawn_proof,
            process_instances,
            endpoint_owner,
            supervisor,
        ) {
            Ok(installed) => installed,
            Err(error) => {
                self.endpoint = Task41EndpointInstallOutcome::Rejected(error.clone());
                return Err(error);
            }
        };
        match installed.register_pre_session_after_attestation(
            &self.spawn_identity,
            directory,
            requirements,
            inventory,
            consent,
            supervisor,
            readiness,
        ) {
            Ok(registered) => {
                self.endpoint = Task41EndpointInstallOutcome::Registered(registered);
                Ok(())
            }
            Err(failure) => {
                let reason = failure.reason.clone();
                self.endpoint = Task41EndpointInstallOutcome::RegistrationFailed(failure);
                Err(reason)
            }
        }
    }

    /// Lend the exact registered Task24 endpoint owner to C1's READY consumer.
    /// Call this from C1's checked Session callback with the event created for
    /// this same run. `event.with_current` holds the existing C3, C2 run-owner,
    /// and Supervisor gates while the callback runs; it also revalidates the
    /// current C2 registration and system-service handshake. The callback is
    /// synchronous and receives no raw descriptor or replacement proof.
    #[cfg(feature = "system-service-readiness")]
    pub(crate) fn with_current_system_service_ready_owner(
        &self,
        proof: &SpawnedProcessPidfd,
        record: R,
        event: &super::Pid1SystemServiceReadyEvent<'_>,
        consume: impl FnOnce(RegistrationId, &Pid1SpawnEndpointLease) -> Result<(), PeerDenial>,
    ) -> Result<(), PeerDenial> {
        let identity = proof.identity();
        event.with_current(move |current| {
            let service = current.service();
            if !self.matches_spawn(&identity, &record) {
                return Err(PeerDenial::ForeignSpawnOwner);
            }
            if !self.matches_run(current.run()) || !self.run.is_same_run(current.run()) {
                return Err(PeerDenial::StaleRun);
            }
            if self.retirement_state != ReapLeaseRetirementState::NotAttempted {
                return Err(PeerDenial::StaleRegistration);
            }

            let registered = match &self.endpoint {
                Task41EndpointInstallOutcome::Registered(registered) => registered,
                Task41EndpointInstallOutcome::Pending(_) => {
                    return Err(PeerDenial::FdDelegationNotPrevented);
                }
                Task41EndpointInstallOutcome::RegistrationFailed(failure) => {
                    return Err(failure.reason.clone());
                }
                Task41EndpointInstallOutcome::Rejected(error) => return Err(error.clone()),
            };
            if !registered.matches_spawn(&identity, &record) {
                return Err(PeerDenial::ForeignSpawnOwner);
            }
            if !registered.run.is_same_run(current.run())
                || registered.retirement_state != ReapLeaseRetirementState::NotAttempted
                || registered
                    .retirement_pending
                    .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Err(PeerDenial::StaleRegistration);
            }
            let endpoint = registered.endpoint();
            if registered.lease.process_instance() != registered.process
                || registered.registration != service.registration()
                || registered.process != service.process()
                || endpoint != service.endpoint()
                || endpoint.route != service.route()
            {
                return Err(PeerDenial::StaleRegistration);
            }
            if !registered.lease.is_current(endpoint)? {
                return Err(PeerDenial::StaleEndpoint);
            }

            consume(registered.registration, &registered.lease)
        })?
    }

    /// Bind this exact registered Task24 child to C3's SystemService proof and
    /// signed handshake plan, execute C2's bounded handshake, then return the
    /// READY evidence for that same registration. The endpoint authority is
    /// borrowed internally from this lease's registered endpoint; callers
    /// cannot supply a different authority or retarget the registration. The
    /// returned event remains borrowed from this lease and must be consumed
    /// inside C1's checked Session gate.
    #[cfg(feature = "system-service-readiness")]
    #[allow(clippy::too_many_arguments)]
    pub fn complete_system_service_handshake_before_ready<'a, P, A, B, H>(
        &'a self,
        run: &'a Pid1RunInstance,
        proof: &'a SystemServiceEndpointOwnerProof,
        directory: &'a Pid1LivePeerDirectory<R>,
        handshakes: &'a super::Pid1CompletedServiceHandshakeOwner<'a, P>,
        supervisor: &'a A,
        readiness: &'a B,
        plan: &'a H,
        deadline: Instant,
    ) -> Result<super::Pid1SystemServiceReadyEvent<'a>, PeerDenial>
    where
        P: PeerProcessInstanceAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
        H: super::ServiceHandshakeProtocol + ?Sized,
    {
        if Instant::now() >= deadline {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        let identity = proof.process_identity();
        if !self.matches_spawn(&identity, &self.record) {
            return Err(PeerDenial::ForeignSpawnOwner);
        }
        if !self.matches_run(run) || !self.run.is_same_run(run) {
            return Err(PeerDenial::StaleRun);
        }
        if self.retirement_state != ReapLeaseRetirementState::NotAttempted {
            return Err(PeerDenial::StaleRegistration);
        }

        let registered = match &self.endpoint {
            Task41EndpointInstallOutcome::Registered(registered) => registered,
            Task41EndpointInstallOutcome::Pending(_) => {
                return Err(PeerDenial::FdDelegationNotPrevented);
            }
            Task41EndpointInstallOutcome::RegistrationFailed(failure) => {
                return Err(failure.reason.clone());
            }
            Task41EndpointInstallOutcome::Rejected(error) => return Err(error.clone()),
        };
        if !registered.matches_spawn(&identity, &self.record) {
            return Err(PeerDenial::ForeignSpawnOwner);
        }
        if !registered.run.is_same_run(run)
            || registered.retirement_state != ReapLeaseRetirementState::NotAttempted
            || registered
                .retirement_pending
                .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err(PeerDenial::StaleRegistration);
        }
        let endpoint = registered.endpoint();
        if registered.lease.process_instance() != registered.process
            || endpoint.route != plan.route()
        {
            return Err(PeerDenial::StaleRegistration);
        }
        if !registered.lease.is_current(endpoint)? {
            return Err(PeerDenial::StaleEndpoint);
        }

        let endpoints = &registered.lease;
        directory.bind_system_service_registration(
            run,
            registered.registration,
            proof,
            endpoints,
            supervisor,
            readiness,
            plan,
        )?;

        handshakes.complete_system_service_handshake(
            directory,
            run,
            readiness,
            registered.registration,
            proof,
            endpoints,
            supervisor,
            plan,
            deadline,
        )?;

        let ready = handshakes.system_service_ready_event(
            directory, run, proof, endpoints, supervisor, readiness, plan,
        )?;
        let service = ready.service();
        if service.registration() != registered.registration
            || service.process() != registered.process
            || service.endpoint() != endpoint
            || service.route() != endpoint.route
        {
            return Err(PeerDenial::StaleRegistration);
        }
        Ok(ready)
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
            Task41EndpointInstallOutcome::Pending(endpoint) => {
                endpoint.retire_after_exact_child_reap(process_instance, record, process_instances)
            }
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
            .field(
                "endpoint_pending",
                &matches!(&self.endpoint, Task41EndpointInstallOutcome::Pending(_)),
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

    /// Accept and issue the first Session from this registration's own
    /// spawn-owned listener. The directory revalidates the authenticated peer,
    /// process instance, endpoint and C1 authority against this exact checkout.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn accept_initial_session_until<S, C, P, A, B>(
        &self,
        directory: &Pid1LivePeerDirectory<R>,
        sessions: &S,
        consent: &C,
        process_instances: &P,
        supervisor: &A,
        readiness: &B,
        deadline: Instant,
    ) -> Result<SessionLease, PeerDenial>
    where
        S: LiveSessionAuthority,
        C: ConsentSource,
        P: PeerProcessInstanceAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
    {
        let endpoint = self.endpoint();
        let checkout = self.lease.accept_checkout_until(endpoint, deadline)?;
        directory.accept_and_issue_initial_session_until(
            &super::Task39PeerCredentialReader,
            checkout,
            self.registration,
            sessions,
            consent,
            &self.lease,
            process_instances,
            supervisor,
            readiness,
        )
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

    #[test]
    fn c3_identity_ack_requires_exact_spawn_record_and_attestation() {
        let original = nagi_init_spawn_contract::test_identity(47);
        let reused_pid = nagi_init_spawn_contract::test_identity(47);
        let record = TestRecord(47, 11);

        assert!(c3_identity_ack_matches_task24_spawn(
            &original, record, &original, record, true,
        ));
        assert!(!c3_identity_ack_matches_task24_spawn(
            &original,
            record,
            &reused_pid,
            record,
            true,
        ));
        assert!(!c3_identity_ack_matches_task24_spawn(
            &original,
            record,
            &original,
            TestRecord(47, 12),
            true,
        ));
        assert!(!c3_identity_ack_matches_task24_spawn(
            &original, record, &original, record, false,
        ));
    }

    #[test]
    fn task24_identity_lease_can_transfer_to_c3_launch_owner() {
        fn assert_send<T: Send>() {}
        assert_send::<Task24EndpointLaunchLease<TestRecord>>();
    }

    #[cfg(feature = "task24-owner-co-location")]
    #[test]
    fn actual_task24_owner_trait_matches_task41_spawn_nonce_and_record() {
        use super::super::task24_owner_co_location::ExactChildLaunchLease;
        let original = nagi_init_spawn_contract::test_identity(81);
        let reused = nagi_init_spawn_contract::test_identity(81);
        let record = TestRecord(81, 17);
        let run = Pid1LivePeerDirectory::<TestRecord>::new()
            .begin_run()
            .expect("test run");
        let lease = Task24EndpointLaunchLease {
            run,
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

#[cfg(target_os = "linux")]
fn bind_spawn_listener_with_passcred(
    path: &Path,
    child_uid: u32,
    child_gid: u32,
) -> io::Result<(UnixListener, SocketPathCleanup)> {
    let raw_fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if raw_fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw_fd` was just returned as a new owned descriptor.
    let fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

    let path_bytes = path.as_os_str().as_bytes();
    let path_c = std::ffi::CString::new(path_bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "socket path contains NUL"))?;
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if path_c.as_bytes_with_nul().len() > address.sun_path.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Unix socket path exceeds sun_path capacity",
        ));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (target, source) in address
        .sun_path
        .iter_mut()
        .zip(path_c.as_bytes_with_nul().iter().copied())
    {
        *target = source as libc::c_char;
    }
    let address_length = std::mem::offset_of!(libc::sockaddr_un, sun_path)
        .checked_add(path_c.as_bytes_with_nul().len())
        .and_then(|length| libc::socklen_t::try_from(length).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "socket path too long"))?;
    // SAFETY: `address` contains a NUL-terminated path within `sun_path`, and
    // address_length covers the family and exact path bytes.
    let bind_result = unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            address_length,
        )
    };
    if bind_result != 0 {
        return Err(io::Error::last_os_error());
    }
    let cleanup = SocketPathCleanup::new(path.to_path_buf());

    // Configure the bound socket before listen makes it reachable. Binding
    // first also avoids asking the kernel to implicitly bind this AF_UNIX
    // socket while applying SO_PASSCRED.
    linux_peer_credentials_supplier::enable_passcred(fd.as_raw_fd())?;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    // SAFETY: `path_c` names the socket just bound in the verified private
    // directory; peers cannot replace that final component.
    let chown_result = unsafe { libc::chown(path_c.as_ptr(), child_uid, child_gid) };
    if chown_result != 0 {
        return Err(io::Error::last_os_error());
    }

    // `listen` is intentionally last: clients cannot create accepted sockets
    // before the kernel has inherited SO_PASSCRED from this listener.
    let listen_result = unsafe { libc::listen(fd.as_raw_fd(), libc::SOMAXCONN) };
    if listen_result != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: ownership of the successfully-bound/listening descriptor moves
    // into UnixListener exactly once.
    let listener = unsafe { UnixListener::from_raw_fd(fd.into_raw_fd()) };
    Ok((listener, cleanup))
}
