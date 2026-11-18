//! Assemble the maintained C3 launch slot and canonical C2 endpoint owner
//! inside the same PID 1 crate. The caller supplies the installed login
//! package and the existing policy owners.

use super::linux_c3_boot_launch::AuthenticatedBootService;
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
use nagi_service_contract::{
    CapabilityId, ContractVersion, OperationId, ServiceDescriptor, ServiceId, ServiceOperation,
};
use nagi_task37_receipt_supplier::service_effect::ServiceEffectOwnerId;
use pid1_live_peer_supplier::pidfd_process_instance::{
    LinuxPidfdKernel, PidfdProcessInstanceAuthority,
};
use pid1_live_peer_supplier::{
    LaunchRecordView, PeerDenial, PendingSpawnOwnedEndpoint, Pid1EndpointOwnerRegistry,
    Pid1LivePeerDirectory, Pid1RunInstance, Pid1RunOwnerLease, RouteConsentRequirement,
    SupervisorLaunchAuthority, Task24EndpointLaunchLease, TrustedInventory, TrustedManifest,
    TrustedNipcOperationRoute, TrustedRouteKey, TrustedServiceRequirements,
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
const LOGIN_OPERATION: &str = "initial-login";
const LOGIN_PAYLOAD: &str = "nagi.auth.initial-login@1";
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
        if signed.entry() != LOGIN_PACKAGE_ENTRY || signed.grants().next().is_some() {
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

/// Pre-Session route from the recovered system-service binding and this
/// exact C2 run. Only the first-checkout path can take its broker route.
pub(super) struct BootInitialLoginRoute {
    run: Pid1RunInstance,
    boot_binding: [u8; 32],
    pub(super) destination: nagi_sdk::AppId,
    route: TrustedNipcOperationRoute,
}

impl BootInitialLoginRoute {
    pub(super) fn from_recovered_boot(
        boot: &AuthenticatedBootService<'_>,
        run: &BootLoginRun,
    ) -> Result<Self, PeerDenial> {
        use sha2::{Digest, Sha256};

        let instance = run.run_owner.run();
        if !instance.is_active() {
            return Err(PeerDenial::StaleRun);
        }
        let scope = boot.recovery_scope();
        let service =
            ServiceId::new(scope.service_id()).map_err(|_| PeerDenial::UnknownNipcRoute)?;
        let operation =
            OperationId::new(LOGIN_OPERATION).map_err(|_| PeerDenial::UnknownNipcOperation)?;
        let version = ContractVersion::new(1, 0).map_err(|_| PeerDenial::UnknownNipcOperation)?;
        let service_selector = nagi_model::AppId::from_identifier(service.as_str().as_bytes());
        let operation_selector = nagi_model::AppId::from_identifier(LOGIN_PAYLOAD.as_bytes());
        let key = TrustedRouteKey::from_canonical_route(
            service_selector.0,
            operation_selector.0,
            version.major(),
            version.minor(),
            instance.generation().get(),
        )
        .ok_or(PeerDenial::UnknownNipcRoute)?;

        // The generic C2 route descriptor requires a capability label. This
        // operation enters only the pre-Session checkout, is excluded from
        // the general mapper, and the pinned child declares no grants.
        let label = CapabilityId::new("auth.initial-login").map_err(|_| PeerDenial::NotDeclared)?;
        let descriptor = ServiceDescriptor::new(
            service,
            [version],
            [ServiceOperation::new(operation.clone(), Some(label))],
        )
        .map_err(|_| PeerDenial::UnknownNipcOperation)?;
        let mut binding = Sha256::new();
        binding.update(b"Nagi initial login route owner v1\0");
        binding.update(scope.binding_digest());
        binding.update(instance.generation().get().to_be_bytes());
        let digest = binding.finalize();
        let mut owner = [0; 16];
        owner.copy_from_slice(&digest[..16]);
        let route = TrustedNipcOperationRoute::from_service_contract(
            key,
            scope.package_app_id(),
            LOGIN_PAYLOAD,
            &descriptor,
            version,
            &operation,
            ServiceEffectOwnerId::new(owner),
        )?;
        Ok(Self {
            run: instance.clone(),
            boot_binding: *scope.binding_digest(),
            destination: nagi_sdk::AppId(scope.package_app_id()),
            route,
        })
    }

    pub(super) fn matches(&self, boot: &AuthenticatedBootService<'_>, run: &BootLoginRun) -> bool {
        self.run.is_active()
            && self.run.is_same_run(run.run_owner.run())
            && &self.boot_binding == boot.recovery_scope().binding_digest()
    }
}

pub(super) struct EndpointOwners<'a> {
    pub(super) run: &'a BootLoginRun,
    pub(super) consent: &'a BootConsentUiConnection,
}

impl EndpointOwners<'_> {
    fn retire(
        &self,
        lease: &mut BoundLaunchLease<LoginLease, LaunchRecord>,
    ) -> Result<(), BackendError> {
        gate::retire_task41_endpoint_after_exact_reap(
            lease,
            &self.run.directory,
            &self.run.instances,
            self.run.run_owner.readiness_boundary(),
        )
    }
}

fn revoke_login_record(pid: u32, launch: &LaunchRecord) -> Result<(), BackendError> {
    if crate::supervisor::record_exit_if_matches(pid, launch) {
        Ok(())
    } else {
        Err(BackendError::Revoked)
    }
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

#[cfg(all(linux_task24_initial_login_runtime, linux_boot_owner_registration))]
pub(super) fn run_owner_login(
    mut runtime: super::linux_c3_boot_launch::linux_c1_boot_recovery::SelectedBootIdentityRuntime<
        '_,
        '_,
    >,
    package: &VerifiedLoginChild<'_>,
    route: BootInitialLoginRoute,
    run: &BootLoginRun,
    console: &mut super::linux_owner_registration_console::BootOwnerConsole,
) -> ! {
    use super::linux_owner_registration_console::task24_input::RequestRoute;
    use std::io::Read;

    let mut execute = || -> Result<(), BackendError> {
        if runtime
            .owner_registration_required()
            .map_err(|_| BackendError::Unavailable)?
        {
            runtime
                .run_owner_registration(console)
                .map_err(|_| BackendError::Unavailable)?;
            super::log_pid1_stderr(format_args!("NAGI_LINUX_OWNER_REGISTRATION_SAVED"));
        }
        let account = runtime
            .owner_login_name()
            .map_err(|_| BackendError::Unavailable)?;
        let mut unlock =
            crate::login_screen::LinuxOwnerUnlock::new(&account, nagi_localization::Locale::EnUs);
        console
            .present_unlock(&unlock)
            .map_err(|_| BackendError::Unavailable)?;
        let consent = console
            .consent_connection()
            .map_err(|_| BackendError::Unavailable)?;
        let mut request = [0; 16];
        std::fs::File::open("/dev/urandom")
            .and_then(|mut random| random.read_exact(&mut request))
            .map_err(|_| BackendError::Unavailable)?;
        let request_key = nagi_identity_session::IdempotencyKey::new(
            request
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        )
        .map_err(|_| BackendError::ProtocolViolation)?;
        let input = RequestRoute {
            destination: route.destination,
            request,
        };
        let deadline = Instant::now() + Duration::from_secs(300);
        let placement = LaunchPlacement {
            app_session_id: nagi_model::AppSessionId(1),
            node_id: nagi_model::NodeId(1),
            workspace_id: None,
        };
        let selection = LoginChildLaunch {
            service: ServiceId::new("dev.task24-login")
                .map_err(|_| BackendError::ProtocolViolation)?,
            credentials: LinuxExecutionIdentity::DEV_SERVICE,
            placement,
        };
        let owners = EndpointOwners {
            run,
            consent: &consent,
        };
        let mut lifecycle = new_lifecycle();
        let (runtime, attached) = runtime
            .start_task24_login(
                &mut lifecycle,
                selection,
                package,
                route,
                &owners,
                request_key,
                None,
                |channel| console.submit_unlock(&mut unlock, channel, input, deadline),
                deadline,
            )
            .map_err(|_| BackendError::Unavailable)?;
        match attached {
            Ok(session) => {
                super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_SESSION_ATTACHED"));
                let notes_owner = runtime.notes_owner().map_err(|_| {
                    crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError::Unavailable
                });
                let _notes_bundle = match notes_owner
                    .as_ref()
                    .map_err(|error| *error)
                    .and_then(|owner| runtime.request_notes_attachment(session.lease, owner))
                {
                    Ok(bundle) => {
                        super::log_pid1_stderr(format_args!(
                            "NAGI_NOTES_PROFILE_ATTACHMENT_CHECKED"
                        ));
                        Some(bundle)
                    }
                    Err(error) => {
                        super::log_pid1_stderr(format_args!(
                            "NAGI_NOTES_PROFILE_VOLUME_UNAVAILABLE error={error} fallback=none"
                        ));
                        None
                    }
                };
                super::wait_in_pid1(super::wait_for_pid1_os_event);
            }
            Err(error) => {
                super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_TASK24_LOGIN_UNAVAILABLE ({error:?})"
                ));
                // A bounded retirement failure can leave the exact child
                // and broker lease in this lifecycle's quarantine.
                super::wait_in_pid1(super::wait_for_pid1_os_event);
            }
        }
    };
    if execute().is_err() {
        super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_LOGIN_STOPPED"));
    }
    super::wait_in_pid1(super::wait_for_pid1_os_event)
}

#[cfg(linux_task24_initial_login_runtime)]
pub(super) struct AttachedLogin {
    lease: SessionLease,
    _control: std::os::unix::net::UnixStream,
}

/// Start and check out the exact child while the caller retains the recovered
/// C1 issuer, signed package, and C3 process lifecycle.
#[cfg(linux_task24_initial_login_runtime)]
pub(super) fn start_verified_login(
    lifecycle: &mut LoginLifecycle,
    selection: LoginChildLaunch,
    package: &VerifiedLoginChild<'_>,
    route: BootInitialLoginRoute,
    owners: &EndpointOwners<'_>,
    issuer: &BootLoginIssuer,
    submit: impl FnOnce(&mut std::os::unix::net::UnixStream) -> std::io::Result<()>,
    deadline: Instant,
) -> Result<AttachedLogin, PeerDenial> {
    let mut input_channel = None;
    let result = C2InitialLogin {
        directory: &owners.run.directory,
        start_login_child: Some(|deadline| {
            start_and_borrow(
                lifecycle,
                selection,
                package,
                route,
                owners,
                |channel| {
                    let stream = input_channel.insert(channel);
                    submit(stream).map_err(|_| BackendError::Unavailable)
                },
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
        readiness_boundary: owners.run.run_owner.readiness_boundary(),
        deadline,
    }
    .start_and_accept(issuer);
    match result {
        Ok(session) => Ok(AttachedLogin {
            lease: session,
            _control: input_channel.ok_or(PeerDenial::RequestOutcomeUnknown)?,
        }),
        Err(error) => {
            lifecycle
                .shutdown_with_launch_lease(revoke_login_record, |lease, _status| {
                    owners.retire(lease)
                })
                .map_err(|_| PeerDenial::RequestOutcomeUnknown)?;
            Err(error)
        }
    }
}

#[cfg(linux_task24_initial_login_runtime)]
fn start_and_borrow<'slot>(
    lifecycle: &'slot mut LoginLifecycle,
    selection: LoginChildLaunch,
    package: &VerifiedLoginChild<'_>,
    route: BootInitialLoginRoute,
    owners: &EndpointOwners<'_>,
    receive_control: impl FnOnce(std::os::unix::net::UnixStream) -> Result<(), BackendError>,
    deadline: Instant,
) -> Result<&'slot LoginLease, BackendError> {
    let release_wait_budget = deadline.saturating_duration_since(Instant::now());
    if release_wait_budget.is_zero() {
        return Err(BackendError::Unavailable);
    }
    let app_id = package.generation.snapshot().app_id();
    let readiness = owners.run.run_owner.readiness_boundary();
    if package.manifest.app_id() != app_id {
        super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_LAUNCH_PACKAGE_ID_MISMATCH"));
        return Err(BackendError::IdentityRejected);
    }
    crate::supervisor::register_or_match_manifest(package.manifest).map_err(|error| {
        super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_LAUNCH_MANIFEST ({error:?})"));
        BackendError::IdentityRejected
    })?;
    crate::supervisor::check_launch(app_id, selection.placement).map_err(|error| {
        super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_LAUNCH_CHECK ({error:?})"));
        BackendError::IdentityRejected
    })?;
    // The route owner supplies the canonical login route. This listener is
    // checked out only for first Session issuance. Application capabilities
    // and decisions are handled after C1 attaches the issued Session.
    let requirements = TrustedServiceRequirements::from_canonical_route(
        route.route.route_key(),
        &[],
        RouteConsentRequirement::NotRequired,
    );
    prepare_task24_endpoint_directory()?;
    let pending = PendingSpawnOwnedEndpoint::bind_for_spawn(
        owners.run.run_owner.run(),
        route.route,
        selection.credentials.uid,
        selection.credentials.gid,
    )
    .map_err(|_| BackendError::Unavailable)?;
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
    let retire =
        |saved: &mut BoundLaunchLease<LoginLease, LaunchRecord>, _status| owners.retire(saved);
    let pid = lifecycle
        .spawn_authorized_with_prepared_and_launch_lease(
            launch,
            || Instant::now() >= deadline,
            |pid, _, _| {
                crate::supervisor::record_launch(pid, app_id, selection.placement).map_err(
                    |error| {
                        super::log_pid1_stderr(format_args!(
                            "NAGI_DEV_TASK24_LAUNCH_RECORD ({error:?})"
                        ));
                        BackendError::IdentityRejected
                    },
                )
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
                    .authorize_and_commit(*live, &[], || {
                        gate.commit().map_err(|_| PeerDenial::Unavailable)
                    })
                    .map_err(|_| BackendError::Unavailable)
            },
            revoke_login_record,
            retire,
        )
        .map_err(|error| {
            super::log_pid1_stderr(format_args!(
                "NAGI_DEV_TASK24_CHILD_START_FAILED ({error:?})"
            ));
            error
        })?;
    super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_CHILD_EXEC_ACK"));
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
                &requirements,
                package,
                owners.consent,
                &BootLoginSupervisor,
                readiness,
            )
        })
    };
    if let Err(error) = activation {
        super::log_pid1_stderr(format_args!(
            "NAGI_DEV_TASK24_CHILD_ACTIVATE_FAILED ({error:?})"
        ));
        lifecycle.terminate_exact_with_launch_lease(pid, record, revoke_login_record, retire)?;
        return Err(error);
    }
    lifecycle.borrow_current_launch_lease(&proof, record)?;
    super::log_pid1_stderr(format_args!(
        "NAGI_DEV_TASK24_CHILD_ACK_ATTESTED_AND_BORROWED"
    ));
    let owner_channel = lifecycle
        .take_live_endpoint(pid, record)
        .map_err(|_| BackendError::Unavailable)?;
    use std::io::Read;
    let mut channel: std::os::unix::net::UnixStream = owner_channel.endpoint.into();
    let remaining = deadline.saturating_duration_since(Instant::now());
    channel
        .set_read_timeout(Some(remaining.min(Duration::from_secs(2))))
        .map_err(|_| BackendError::Unavailable)?;
    let mut ready = [0u8; 1];
    channel
        .read_exact(&mut ready)
        .map_err(|_| BackendError::Unavailable)?;
    if ready != [super::linux_owner_registration_console::task24_input::CONNECTED] {
        return Err(BackendError::ProtocolViolation);
    }
    super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_CHILD_BROKER_CONNECTED"));
    receive_control(channel)?;
    lifecycle.borrow_current_launch_lease(&proof, record)
}

#[cfg(linux_task24_initial_login_runtime)]
fn prepare_task24_endpoint_directory() -> Result<(), BackendError> {
    use std::io::ErrorKind;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};

    // Validate the parent namespace without changing the Data owner's
    // private /run/nagi. C3 exposes only its listener inside the child.
    for parent in ["/run", "/run/nagi"] {
        let metadata = std::fs::symlink_metadata(parent).map_err(|_| BackendError::Unavailable)?;
        if !metadata.is_dir()
            || metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
            || metadata.mode() & 0o100 == 0
        {
            return Err(BackendError::IdentityRejected);
        }
    }
    match std::fs::DirBuilder::new()
        .mode(0o755)
        .create("/run/nagi/ipc")
    {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
        Err(_) => Err(BackendError::Unavailable),
    }
    // C2 independently validates the IPC directory before binding.
}
