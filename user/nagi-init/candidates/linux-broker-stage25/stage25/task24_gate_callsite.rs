//! Concrete child-module helper to insert into Task24's existing
//! `linux_init_process_owner_candidate` module. The caller invokes this from
//! its existing authorization closure, after the Supervisor LaunchRecord has
//! been registered and before `authorize_launch_and_commit(... gate.commit())`.
//! It neither opens a second gate nor creates a second child reaper.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use super::super::FdDelegationRestrictionAttestation;
use super::pid1_live_peer_supplier::pidfd_process_instance::PidfdProcessInstanceAuthority;
use super::pid1_live_peer_supplier::{
    ConsentSource, LaunchRecordView, LiveSessionAuthority, PeerDenial,
    PeerProcessInstanceAuthority, PendingSpawnOwnedEndpoint, Pid1EndpointOwnerRegistry,
    Pid1LivePeerDirectory, Pid1ReadinessAtomicBoundary, Pid1RunInstance, SessionLease,
    SupervisorLaunchAuthority, Task24EndpointLaunchLease, TrustedInventory,
    TrustedServiceRequirements,
};
use super::{BackendError, BoundLaunchLease, GateCommit, ProcessOs};
use std::time::Instant;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl super::pid1_live_peer_supplier::c3_fd_attestation_seal::Sealed
    for FdDelegationRestrictionAttestation
{
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl super::pid1_live_peer_supplier::C3FdDelegationAttestation
    for FdDelegationRestrictionAttestation
{
    fn matches_process(&self, process: &nagi_init_spawn_contract::SpawnedProcessIdentity) -> bool {
        FdDelegationRestrictionAttestation::matches_process(self, process)
    }
}

/// Retain only the pending listener and exact-child binding before the release
/// gate. No C2 endpoint-owner or live-peer registration is installed here.
/// Return a same-instance pidfd duplicate for C3's later activation and slot
/// borrow; the caller should drop it after the first checkout completes.
pub(crate) fn retain_task41_endpoint_before_gate<Os, R>(
    gate: &mut GateCommit<'_, Os, R, Task24EndpointLaunchLease<R>>,
    pending: PendingSpawnOwnedEndpoint,
    run: &Pid1RunInstance,
    process_instances: &PidfdProcessInstanceAuthority<
        super::pid1_live_peer_supplier::pidfd_process_instance::LinuxPidfdKernel,
    >,
) -> Result<nagi_init_spawn_contract::SpawnedProcessPidfd, BackendError>
where
    Os: ProcessOs,
    R: LaunchRecordView,
{
    let mut pending_result = None;
    let mut activation_proof = None;
    gate.install_and_retain_spawn_lease(|spawn, record| {
        // C3's later activation and slot-borrow methods require an opaque
        // pidfd token. Preserve one duplicate for those calls while moving the
        // original into C2's pending exact-child lease.
        let retained_activation_proof = spawn
            .duplicate_for_authority()
            .map_err(|_| BackendError::Unavailable)?;
        let lease = Task24EndpointLaunchLease::retain_pending_before_exec_gate(
            pending,
            spawn,
            *record,
            run,
            process_instances,
        );
        pending_result = Some(lease.pending_result());
        activation_proof = Some(retained_activation_proof);
        Ok(lease)
    })?;

    pending_result
        .ok_or(BackendError::ProtocolViolation)?
        .map_err(|_| BackendError::Unavailable)?;
    activation_proof.ok_or(BackendError::ProtocolViolation)
}

/// C3 calls this from `activate_current_launch_lease` after exec ack. The
/// concrete attestation borrow is adapted only for this activation call; C2
/// checks it against the retained child identity before installing the
/// endpoint owner or registering a live peer.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn activate_task41_endpoint_after_exec_ack<R, I, C, A, B>(
    lease: &mut Task24EndpointLaunchLease<R>,
    proof: &nagi_init_spawn_contract::SpawnedProcessPidfd,
    record: R,
    attestation: &FdDelegationRestrictionAttestation,
    run: &Pid1RunInstance,
    process_instances: &PidfdProcessInstanceAuthority<
        super::pid1_live_peer_supplier::pidfd_process_instance::LinuxPidfdKernel,
    >,
    endpoint_owner: &Pid1EndpointOwnerRegistry,
    directory: &Pid1LivePeerDirectory<R>,
    requirements: &TrustedServiceRequirements<'_>,
    inventory: &I,
    consent: &C,
    supervisor: &A,
    readiness: &B,
) -> Result<(), BackendError>
where
    R: LaunchRecordView,
    I: TrustedInventory,
    C: ConsentSource,
    A: SupervisorLaunchAuthority<Record = R>,
    B: Pid1ReadinessAtomicBoundary,
{
    lease
        .activate_after_fd_delegation_attestation(
            proof,
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
        .map_err(|_| BackendError::Unavailable)
}

/// Exact-reap callback body for Task24's existing `retire_exact_once`. The
/// Task24 caller must invoke this only after its wait result and Supervisor
/// revoke both succeeded; the closure itself does not manufacture either fact.
pub(crate) fn retire_task41_endpoint_after_exact_reap<R, B>(
    bound: &mut BoundLaunchLease<Task24EndpointLaunchLease<R>, R>,
    directory: &Pid1LivePeerDirectory<R>,
    process_instances: &PidfdProcessInstanceAuthority<
        super::pid1_live_peer_supplier::pidfd_process_instance::LinuxPidfdKernel,
    >,
    readiness: &B,
) -> Result<(), BackendError>
where
    R: LaunchRecordView,
    B: Pid1ReadinessAtomicBoundary,
{
    bound
        .lease
        .retire_after_exact_child_reap(
            &bound.process_instance,
            bound.record,
            directory,
            process_instances,
            readiness,
        )
        .map(|_| ())
        .map_err(|_| BackendError::Unavailable)
}

/// A calls this once after checking the C1 handoff. The callback starts the
/// service and borrows the lease from this PID-1 run's exact Task24 slot. The
/// shared absolute deadline covers startup and checkout; the borrow remains
/// live until the first authenticated checkout completes.
#[allow(clippy::too_many_arguments)]
pub(crate) fn accept_task41_initial_session_after_spawn<'run, R, S, C, P, A, B>(
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
) -> Result<SessionLease, BackendError>
where
    R: LaunchRecordView + 'run,
    S: LiveSessionAuthority,
    C: ConsentSource,
    P: PeerProcessInstanceAuthority,
    A: SupervisorLaunchAuthority<Record = R>,
    B: Pid1ReadinessAtomicBoundary,
{
    Task24EndpointLaunchLease::accept_after_startup_and_borrow_slot(
        start_exact_child_and_borrow_slot,
        run,
        directory,
        sessions,
        consent,
        process_instances,
        supervisor,
        readiness,
        deadline,
    )
    .map_err(|_| BackendError::Unavailable)
}
