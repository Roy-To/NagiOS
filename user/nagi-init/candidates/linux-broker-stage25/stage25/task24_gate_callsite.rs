//! Concrete child-module helper to insert into Task24's existing
//! `linux_init_process_owner_candidate` module. The caller invokes this from
//! its existing authorization closure, after the Supervisor LaunchRecord has
//! been registered and before `authorize_launch_and_commit(... gate.commit())`.
//! It neither opens a second gate nor creates a second child reaper.

use super::pid1_live_peer_supplier::{
    ConsentSource, LaunchRecordView, LiveSessionAuthority, PendingSpawnOwnedEndpoint,
    Pid1EndpointOwnerRegistry, Pid1LivePeerDirectory, Pid1ReadinessAtomicBoundary, Pid1RunInstance,
    PidfdProcessInstanceAuthority, SupervisorLaunchAuthority, Task24EndpointLaunchLease,
    TrustedInventory, TrustedServiceRequirements,
};
use super::{BackendError, BoundLaunchLease, GateCommit, ProcessOs};

/// Consume the Task24-issued proof and move the resulting success-or-failure
/// endpoint lease into Task24's exact child slot before returning. A failure
/// is stored in the lease, then surfaced to the authorization callback so it
/// can refuse the Supervisor gate release while Task24 retains exact cleanup.
#[allow(clippy::too_many_arguments)]
pub(crate) fn retain_task41_endpoint_before_gate<Os, R, I, S, C, A, B>(
    gate: &mut GateCommit<'_, Os, R, Task24EndpointLaunchLease<R>>,
    pending: PendingSpawnOwnedEndpoint,
    run: &Pid1RunInstance,
    process_instances: &PidfdProcessInstanceAuthority<
        super::pid1_live_peer_supplier::pidfd_process_instance::LinuxPidfdKernel,
    >,
    endpoint_owner: &Pid1EndpointOwnerRegistry,
    directory: &Pid1LivePeerDirectory<R>,
    requirements: &TrustedServiceRequirements<'_>,
    inventory: &I,
    sessions: &S,
    consent: &C,
    supervisor: &A,
    readiness: &B,
) -> Result<(), BackendError>
where
    Os: ProcessOs,
    R: LaunchRecordView,
    I: TrustedInventory,
    S: LiveSessionAuthority,
    C: ConsentSource,
    A: SupervisorLaunchAuthority<Record = R>,
    B: Pid1ReadinessAtomicBoundary,
{
    let mut install_result = None;
    gate.install_and_retain_spawn_lease(|spawn, record| {
        let lease = Task24EndpointLaunchLease::install_and_register(
            pending,
            spawn,
            *record,
            run,
            process_instances,
            endpoint_owner,
            directory,
            requirements,
            inventory,
            sessions,
            consent,
            supervisor,
            readiness,
        );
        install_result = Some(lease.install_result());
        Ok(lease)
    })?;

    install_result
        .ok_or(BackendError::ProtocolViolation)?
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
