//! Assemble the maintained C3 launch slot and canonical C2 endpoint owner
//! inside the same PID 1 crate. The caller supplies the installed login
//! package and the existing policy owners.

use super::linux_owner_registration_console::BootConsentUiConnection;
use super::linux_x86_64_process::LinuxProcessOs;
use super::pid1_live_peer_supplier;
use super::pid1_owned_launch_lease::{BoundLaunchLease, ExactChildLaunchLease};
use super::{BackendError, GateCommit, ProcessOs};
use super::{
    ExecutablePayload, LinuxExecutionIdentity, ProcessEndpointPolicy, ProcessLaunch,
    ProcessLifecycle,
};
use libnagi::launch::{AppManifest, GrantCheck, LaunchPlacement, LaunchRecord};
use nagi_init_package_provider::package_trust::{
    InstalledPackageBlobStore, PackageServiceGenerationLease, PackageServiceSessionOwner,
};
use nagi_init_spawn_contract::SpawnedProcessIdentity;
use nagi_service_contract::ServiceId;
use pid1_live_peer_supplier::pidfd_process_instance::{
    LinuxPidfdKernel, PidfdProcessInstanceAuthority,
};
use pid1_live_peer_supplier::{
    LaunchRecordView, PeerDenial, PendingSpawnOwnedEndpoint, Pid1EndpointOwnerRegistry,
    Pid1LivePeerDirectory, Pid1ReadinessAtomicBoundary, Pid1RunOwnerLease,
    SupervisorLaunchAuthority, Task24EndpointLaunchLease, TrustedInventory, TrustedManifest,
    TrustedServiceRequirements,
};
use std::num::NonZeroU64;
use std::time::{Duration, Instant};

#[cfg(linux_task24_initial_login_runtime)]
use super::linux_initial_login_c2::{C1InitialLoginAuthority, C2InitialLogin};
#[cfg(linux_task24_initial_login_runtime)]
use super::linux_initial_login_owner::LinuxInitialLoginOwner;
#[cfg(linux_task24_initial_login_runtime)]
use pid1_live_peer_supplier::SessionLease;

const LOGIN_PACKAGE_ID: &[u8] = b"org.nagi.dev.task24-login";
const LOGIN_PACKAGE_ENTRY: &[u8] = b"task24-login.elf";
const LOGIN_PACKAGE_PIN: &str =
    include_str!(concat!(env!("OUT_DIR"), "/linux-dev-login-package-sha256"));

/// A development package selected by the init image and verified against
/// the installed index using the signer from the authenticated boot catalog.
pub(super) struct VerifiedLoginChild<'owner> {
    executable: Box<[u8]>,
    manifest: AppManifest,
    trusted_manifest: TrustedManifest,
    owner: &'owner PackageServiceSessionOwner<'owner>,
    generation: PackageServiceGenerationLease<'owner>,
}

impl<'owner> VerifiedLoginChild<'owner> {
    pub(super) fn load(
        owner: &'owner PackageServiceSessionOwner<'owner>,
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
        let trusted_manifest = TrustedManifest::from_verified_installed_manifest(
            snapshot.app_id().0,
            snapshot.generation(),
            signed.grants().map(|grant| grant.to_vec()),
        )
        .ok_or(BackendError::IdentityRejected)?;
        let executable = Box::from(package.executable());
        ExecutablePayload::from_package_executable(&executable)
            .map_err(|_| BackendError::ProtocolViolation)?;
        let generation = owner
            .lease_for_system_service_dispatch(&snapshot)
            .map_err(|_| BackendError::IdentityRejected)?;
        Ok(Some(Self {
            executable,
            manifest,
            trusted_manifest,
            owner,
            generation,
        }))
    }

    pub(super) fn package_generation(&self) -> u64 {
        self.generation.snapshot().generation()
    }
}

impl TrustedInventory for VerifiedLoginChild<'_> {
    fn installed_manifest(&self, app_id: u64) -> Result<Option<TrustedManifest>, PeerDenial> {
        if app_id != self.generation.snapshot().app_id().0 {
            return Ok(None);
        }
        self.owner
            .lease_for_system_service_dispatch(self.generation.snapshot())
            .map_err(|_| PeerDenial::InventoryUnavailable)?;
        Ok(Some(self.trusted_manifest.clone()))
    }

    fn generation_is_current(
        &self,
        app_id: u64,
        generation: NonZeroU64,
    ) -> Result<bool, PeerDenial> {
        let current = self.generation.snapshot();
        if app_id != current.app_id().0 || generation.get() != current.generation() {
            return Ok(false);
        }
        self.owner
            .lease_for_system_service_dispatch(current)
            .map_err(|_| PeerDenial::InventoryUnavailable)?;
        Ok(true)
    }

    fn with_current_manifests(
        &self,
        requested: &[(u64, NonZeroU64)],
        inspect: &mut dyn FnMut(&[TrustedManifest]) -> Result<(), PeerDenial>,
    ) -> Result<(), PeerDenial> {
        let [(app_id, generation)] = requested else {
            return Err(PeerDenial::StaleManifest);
        };
        let current = self.generation.snapshot();
        if *app_id != current.app_id().0 || generation.get() != current.generation() {
            return Err(PeerDenial::StaleManifest);
        }
        let _lease = self
            .owner
            .lease_for_system_service_dispatch(current)
            .map_err(|_| PeerDenial::InventoryUnavailable)?;
        inspect(std::slice::from_ref(&self.trusted_manifest))
    }
}

pub(super) struct BootLoginSupervisor;

impl SupervisorLaunchAuthority for BootLoginSupervisor {
    type Record = LaunchRecord;

    fn resolve(&self, process_id: u32) -> Option<LaunchRecord> {
        crate::supervisor::resolve(process_id)
    }

    fn authorize_and_commit<T>(
        &self,
        record: LaunchRecord,
        capabilities: &[&[u8]],
        commit: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        use crate::supervisor::LaunchCommitError;
        let mut result = None;
        crate::supervisor::authorize_launch_and_commit(&record, capabilities, || {
            result = Some(commit()?);
            Ok(())
        })
        .map_err(|error| match error {
            LaunchCommitError::RegistryUnavailable => PeerDenial::Unavailable,
            LaunchCommitError::RecordMismatch | LaunchCommitError::Grant(GrantCheck::NotLive) => {
                PeerDenial::NotLive
            }
            LaunchCommitError::Grant(GrantCheck::NotDeclared) => PeerDenial::NotDeclared,
            LaunchCommitError::Grant(GrantCheck::ConsentRequired) => PeerDenial::ConsentRequired,
            LaunchCommitError::Grant(GrantCheck::Denied) => PeerDenial::ConsentDenied,
            LaunchCommitError::Grant(GrantCheck::Granted) => PeerDenial::Unavailable,
            LaunchCommitError::Commit(error) => error,
        })?;
        result.ok_or(PeerDenial::Unavailable)
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

pub(super) struct BootLoginRun {
    pub(super) directory: Pid1LivePeerDirectory<LaunchRecord>,
    pub(super) run_owner: Pid1RunOwnerLease,
    pub(super) instances: PidfdProcessInstanceAuthority<LinuxPidfdKernel>,
    pub(super) registry: Pid1EndpointOwnerRegistry,
}

impl BootLoginRun {
    pub(super) fn begin() -> Result<Self, BackendError> {
        let directory = Pid1LivePeerDirectory::new();
        let run_owner = directory
            .begin_run_owner_lease()
            .map_err(|_| BackendError::Unavailable)?;
        Ok(Self {
            directory,
            run_owner,
            instances: PidfdProcessInstanceAuthority::new(LinuxPidfdKernel),
            registry: Pid1EndpointOwnerRegistry::new(),
        })
    }
}

pub(super) struct EndpointOwners<'a, B> {
    pub(super) run: &'a BootLoginRun,
    pub(super) requirements: &'a TrustedServiceRequirements<'a>,
    pub(super) consent: &'a BootConsentUiConnection<'a>,
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

#[cfg(linux_task24_initial_login_runtime)]
pub(super) type BootLoginIssuer = C1InitialLoginAuthority<
    nagi_identity_session::LinuxIdentitySnapshotStore,
    nagi_identity_session::LinuxIdentityIdSource,
    nagi_identity_session::LinuxIdentityClock,
    nagi_identity_session::C1LinuxBootIdentityAuthorizer,
    nagi_identity_session::C1LinuxGuestProfileCleanupUnavailable,
    nagi_identity_session::LinuxIdentityEventSink,
>;

/// Start and check out the exact child while the caller retains the recovered
/// C1 issuer, signed package, and C3 process lifecycle.
#[cfg(linux_task24_initial_login_runtime)]
pub(super) fn start_verified_login<B>(
    lifecycle: &mut LoginLifecycle,
    selection: LoginChildLaunch,
    package: &VerifiedLoginChild<'_>,
    required_grants: &[&[u8]],
    pending: PendingSpawnOwnedEndpoint,
    owners: &EndpointOwners<'_, B>,
    issuer: &BootLoginIssuer,
    deadline: Instant,
) -> Result<SessionLease, PeerDenial>
where
    B: Pid1ReadinessAtomicBoundary,
{
    C2InitialLogin {
        directory: &owners.run.directory,
        start_login_child: Some(|deadline| {
            start_and_borrow(
                lifecycle,
                selection,
                package,
                required_grants,
                pending,
                owners,
                deadline,
            )
            .map_err(|error| match error {
                BackendError::Revoked => PeerDenial::NotLive,
                BackendError::ConsentDenied => PeerDenial::ConsentDenied,
                _ if Instant::now() >= deadline => PeerDenial::RequestDeadlineExpired,
                _ => PeerDenial::Unavailable,
            })
        }),
        consent: owners.consent,
        process_instances: &owners.run.instances,
        supervisor: &BootLoginSupervisor,
        readiness_boundary: owners.readiness,
        deadline,
    }
    .start_and_accept(issuer)
}

pub(super) fn start_and_borrow<'slot, B>(
    lifecycle: &'slot mut LoginLifecycle,
    selection: LoginChildLaunch,
    package: &VerifiedLoginChild<'_>,
    required_grants: &[&[u8]],
    pending: PendingSpawnOwnedEndpoint,
    owners: &EndpointOwners<'_, B>,
    deadline: Instant,
) -> Result<&'slot LoginLease, BackendError>
where
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
            &owners.run.directory,
            &owners.run.instances,
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
                owners.run.run_owner.run(),
                &owners.run.instances,
            )?);
            record = Some(*live);
            BootLoginSupervisor
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
                owners.run.run_owner.run(),
                &owners.run.instances,
                &owners.run.registry,
                &owners.run.directory,
                owners.requirements,
                package,
                owners.consent,
                &BootLoginSupervisor,
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
