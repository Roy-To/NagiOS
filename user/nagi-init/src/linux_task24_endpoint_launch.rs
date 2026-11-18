//! Assemble the maintained C3 launch slot and canonical C2 endpoint owner
//! inside the same PID 1 crate. The caller supplies the installed login
//! package and the existing policy owners.

use super::linux_x86_64_process::LinuxProcessOs;
use super::pid1_live_peer_supplier;
use super::pid1_owned_launch_lease::{BoundLaunchLease, ExactChildLaunchLease};
use super::{BackendError, GateCommit, ProcessOs};
use super::{ProcessEndpointPolicy, ProcessLaunch, ProcessLifecycle};
use libnagi::launch::{LaunchPlacement, LaunchRecord};
use nagi_init_spawn_contract::SpawnedProcessIdentity;
use nagi_service_contract::ServiceId;
use pid1_live_peer_supplier::pidfd_process_instance::{
    LinuxPidfdKernel, PidfdProcessInstanceAuthority,
};
use pid1_live_peer_supplier::{
    ConsentSource, LaunchRecordView, PeerDenial, PendingSpawnOwnedEndpoint,
    Pid1EndpointOwnerRegistry, Pid1LivePeerDirectory, Pid1ReadinessAtomicBoundary, Pid1RunInstance,
    SupervisorLaunchAuthority, Task24EndpointLaunchLease, TrustedInventory,
    TrustedServiceRequirements,
};
use std::time::Instant;

#[path = "../candidates/linux-broker-stage25/stage25/task24_gate_callsite.rs"]
mod gate;

impl<R: LaunchRecordView> ExactChildLaunchLease<R> for Task24EndpointLaunchLease<R> {
    fn matches_spawn(&self, process: &SpawnedProcessIdentity, record: &R) -> bool {
        Task24EndpointLaunchLease::matches_spawn(self, process, record)
    }
}

pub(super) type LoginLease = Task24EndpointLaunchLease<LaunchRecord>;
pub(super) type LoginLifecycle =
    ProcessLifecycle<LinuxProcessOs, ServiceId, LaunchRecord, LoginLease>;

pub(super) struct EndpointOwners<'a, I, C, A, B> {
    pub(super) run: &'a Pid1RunInstance,
    pub(super) instances: &'a PidfdProcessInstanceAuthority<LinuxPidfdKernel>,
    pub(super) registry: &'a Pid1EndpointOwnerRegistry,
    pub(super) directory: &'a Pid1LivePeerDirectory<LaunchRecord>,
    pub(super) requirements: &'a TrustedServiceRequirements<'a>,
    pub(super) inventory: &'a I,
    pub(super) consent: &'a C,
    pub(super) supervisor: &'a A,
    pub(super) readiness: &'a B,
}

pub(super) fn new_lifecycle() -> LoginLifecycle {
    LoginLifecycle::new_with_launch_lease(LinuxProcessOs)
}

pub(super) fn start_and_borrow<'slot, I, C, A, B>(
    lifecycle: &'slot mut LoginLifecycle,
    launch: ProcessLaunch<'_, ServiceId>,
    placement: LaunchPlacement,
    app_id: nagi_model::AppId,
    required_grants: &[&[u8]],
    pending: PendingSpawnOwnedEndpoint,
    owners: &EndpointOwners<'_, I, C, A, B>,
    deadline: Instant,
) -> Result<&'slot LoginLease, BackendError>
where
    I: TrustedInventory,
    C: ConsentSource,
    A: SupervisorLaunchAuthority<Record = LaunchRecord>,
    B: Pid1ReadinessAtomicBoundary,
{
    if !matches!(
        launch.endpoint_policy,
        ProcessEndpointPolicy::InheritedPeerChannel
    ) || Instant::now() >= deadline
    {
        return Err(BackendError::Unavailable);
    }
    let argv = pending
        .child_arguments(launch.arguments)
        .map_err(|_| BackendError::ProtocolViolation)?;
    let arguments: Vec<&str> = argv.iter().map(String::as_str).collect();
    let launch = ProcessLaunch {
        arguments: &arguments,
        ..launch
    };

    let mut boot_proof = None;
    let mut record = None;
    let revoke = |pid, launch: &LaunchRecord| {
        if crate::supervisor::record_exit_if_matches(pid, launch) {
            Ok(())
        } else {
            Err(BackendError::Revoked)
        }
    };
    let retire = |saved: &mut BoundLaunchLease<LoginLease, LaunchRecord>, _status| {
        gate::retire_task41_endpoint_after_exact_reap(
            saved,
            owners.directory,
            owners.instances,
            owners.readiness,
        )
    };
    let pid = lifecycle.spawn_authorized_with_prepared_and_launch_lease(
        launch,
        || Instant::now() >= deadline,
        |pid, _, _| {
            crate::supervisor::record_launch(pid, app_id, placement)
                .map_err(|_| BackendError::IdentityRejected)
        },
        |_, live, gate| {
            boot_proof = Some(gate::retain_task41_endpoint_before_gate(
                gate,
                pending,
                owners.run,
                owners.instances,
            )?);
            record = Some(*live);
            owners
                .supervisor
                .authorize_and_commit(*live, required_grants, || {
                    gate.commit().map_err(|_| PeerDenial::Unavailable)
                })
                .map_err(|_| BackendError::Unavailable)
        },
        revoke,
        retire,
    )?;
    let proof = boot_proof.ok_or(BackendError::ProtocolViolation)?;
    let record = record.ok_or(BackendError::ProtocolViolation)?;
    let activation = if Instant::now() >= deadline {
        Err(BackendError::Unavailable)
    } else {
        lifecycle.activate_current_launch_lease(&proof, record, |lease, attestation| {
            gate::activate_task41_endpoint_after_exec_ack(
                lease,
                &proof,
                record,
                attestation,
                owners.run,
                owners.instances,
                owners.registry,
                owners.directory,
                owners.requirements,
                owners.inventory,
                owners.consent,
                owners.supervisor,
                owners.readiness,
            )
        })
    };
    if let Err(error) = activation {
        lifecycle.terminate_exact_with_launch_lease(pid, record, revoke, retire)?;
        return Err(error);
    }
    lifecycle.borrow_current_launch_lease(&proof, record)
}
