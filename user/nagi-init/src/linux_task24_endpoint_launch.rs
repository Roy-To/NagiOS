//! Assemble the maintained C3 launch slot and canonical C2 endpoint owner
//! inside the same PID 1 crate. The caller supplies the installed login
//! package and the existing policy owners.

use super::linux_x86_64_process::LinuxProcessOs;
use super::pid1_live_peer_supplier;
use super::pid1_owned_launch_lease::{BoundLaunchLease, ExactChildLaunchLease};
use super::{BackendError, GateCommit, ProcessOs};
use super::{
    ExecutablePayload, LinuxExecutionIdentity, ProcessEndpointPolicy, ProcessLaunch,
    ProcessLifecycle,
};
use libnagi::launch::{AppManifest, LaunchPlacement, LaunchRecord};
use nagi_init_package_provider::package_trust::{
    InstalledPackageBlobStore, PackageServiceGenerationLease, PackageServiceSessionOwner,
};
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
use std::time::{Duration, Instant};

const LOGIN_PACKAGE_ID: &[u8] = b"org.nagi.dev.task24-login";
const LOGIN_PACKAGE_ENTRY: &[u8] = b"task24-login.elf";
const LOGIN_PACKAGE_PIN: &str =
    include_str!(concat!(env!("OUT_DIR"), "/linux-dev-login-package-sha256"));

/// A development package selected by the init image and verified against
/// the installed index using the signer from the authenticated boot catalog.
pub(super) struct VerifiedLoginChild<'owner> {
    executable: Box<[u8]>,
    manifest: AppManifest,
    generation: PackageServiceGenerationLease<'owner>,
}

impl<'owner> VerifiedLoginChild<'owner> {
    pub(super) fn load(
        owner: &'owner PackageServiceSessionOwner<'_>,
        blobs: &dyn InstalledPackageBlobStore,
    ) -> Result<Option<Self>, BackendError> {
        if LOGIN_PACKAGE_PIN.is_empty() {
            return Ok(None);
        }
        let bytes = owner
            .read_installed_system_service_package(LOGIN_PACKAGE_ID, blobs)
            .map_err(|_| BackendError::Unavailable)?;
        let snapshot = owner
            .capture_for_system_service(&bytes, LOGIN_PACKAGE_ID)
            .map_err(|_| BackendError::IdentityRejected)?;
        use sha2::{Digest, Sha256};
        if format!("{:x}", Sha256::digest(&bytes)) != LOGIN_PACKAGE_PIN {
            return Err(BackendError::IdentityRejected);
        }
        let package =
            nagi_package::PackageView::parse(&bytes).map_err(|_| BackendError::IdentityRejected)?;
        let signed = package.manifest();
        if signed.entry() != LOGIN_PACKAGE_ENTRY {
            return Err(BackendError::IdentityRejected);
        }
        let manifest = AppManifest::from_declaration(signed.id(), signed.grants())
            .map_err(|_| BackendError::IdentityRejected)?;
        let executable = Box::from(package.executable());
        ExecutablePayload::from_package_executable(&executable)
            .map_err(|_| BackendError::ProtocolViolation)?;
        let generation = owner
            .lease_for_system_service_dispatch(&snapshot)
            .map_err(|_| BackendError::IdentityRejected)?;
        Ok(Some(Self {
            executable,
            manifest,
            generation,
        }))
    }

    pub(super) fn package_generation(&self) -> u64 {
        self.generation.snapshot().generation()
    }
}

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

pub(super) struct LoginChildLaunch {
    pub(super) service: ServiceId,
    pub(super) credentials: LinuxExecutionIdentity,
    pub(super) placement: LaunchPlacement,
}

pub(super) fn start_and_borrow<'slot, I, C, A, B>(
    lifecycle: &'slot mut LoginLifecycle,
    selection: LoginChildLaunch,
    package: &VerifiedLoginChild<'_>,
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
    let release_wait_budget = deadline.saturating_duration_since(Instant::now());
    if release_wait_budget.is_zero() {
        return Err(BackendError::Unavailable);
    }
    let app_id = package.generation.snapshot().app_id();
    if package.manifest.app_id() != app_id
        || required_grants
            .iter()
            .any(|capability| !package.manifest.grants(capability))
    {
        return Err(BackendError::IdentityRejected);
    }
    crate::supervisor::register_or_match_manifest(package.manifest)
        .map_err(|_| BackendError::IdentityRejected)?;
    crate::supervisor::check_launch(app_id, selection.placement)
        .map_err(|_| BackendError::IdentityRejected)?;
    let argv = pending
        .child_arguments(&[])
        .map_err(|_| BackendError::ProtocolViolation)?;
    let arguments: Vec<&str> = argv.iter().map(String::as_str).collect();
    let launch = ProcessLaunch {
        service: selection.service,
        executable: ExecutablePayload::from_package_executable(&package.executable)
            .map_err(|_| BackendError::ProtocolViolation)?,
        arguments: &arguments,
        execution_identity: selection.credentials,
        endpoint_policy: ProcessEndpointPolicy::InheritedPeerChannel,
        required: false,
        release_wait_budget,
        cancel_check_interval: Duration::from_millis(10),
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
            crate::supervisor::record_launch(pid, app_id, selection.placement)
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
