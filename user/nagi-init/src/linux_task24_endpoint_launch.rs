//! Assemble the maintained C3 launch slot and canonical C2 endpoint owner
//! inside the same PID 1 crate. The caller supplies the installed login
//! package and the existing policy owners.

use super::linux_boot_package_inputs::BootCatalogInputs;
use super::linux_c3_boot_launch::{AuthenticatedBootService, TASK24_IDENTITY_PACKAGE_ID};
use super::linux_owner_registration_console::BootConsentUiConnection;
use super::linux_task24_identity_ready::{
    readiness_challenge, BROKER_APP, CHALLENGE_BYTES, REQUEST_TYPE, RESPONSE_TYPE,
};
use super::linux_x86_64_process::LinuxProcessOs;
use super::pid1_live_peer_supplier;
use super::pid1_owned_launch_lease::{BoundLaunchLease, ExactChildLaunchLease};
use super::task24_login_cancellation::{
    classify_login_denial_after_cleanup, wait_for_login_cleanup_until, LoginCleanupPoll,
    LoginCleanupWait, VerifiedLoginCancellation,
};
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
use nagi_sdk::app_contract::OrderingMetadata;
use nagi_sdk::AppId;
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
    ServiceHandshakeAttemptIds, ServiceHandshakeProtocol, SupervisorLaunchAuthority,
    Task24EndpointLaunchLease, TrustedInventory, TrustedManifest, TrustedNipcOperationRoute,
    TrustedRouteKey, TrustedServiceRequirements,
};
use std::num::NonZeroU64;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[cfg(linux_task24_initial_login_runtime)]
use super::linux_initial_login_c2::{C1InitialLoginAuthority, C2InitialLogin};
#[cfg(linux_task24_initial_login_runtime)]
use super::linux_initial_login_owner::LinuxInitialLoginOwner;
#[cfg(all(
    linux_task24_initial_login_runtime,
    linux_boot_owner_registration,
    feature = "system-service-readiness"
))]
use pid1_live_peer_supplier::Pid1CompletedServiceHandshakeOwner;
#[cfg(linux_task24_initial_login_runtime)]
use pid1_live_peer_supplier::SessionLease;

#[cfg(all(linux_task24_notes_consumer, linux_task24_initial_login_runtime))]
#[path = "linux_task24_notes_console.rs"]
mod notes_console;

#[cfg(all(
    linux_task24_notes_consumer,
    linux_task24_initial_login_runtime,
    linux_task24_desktop_shell,
    not(nagi_release_build)
))]
#[path = "linux_task24_desktop_shell.rs"]
mod desktop_shell;

#[cfg(all(
    linux_task24_initial_login_runtime,
    feature = "system-service-readiness"
))]
#[path = "linux_task24_identity_c2_adapter.rs"]
pub(super) mod identity_listener;

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
    pub(super) identity_ready: Option<IdentityReadyProtocol>,
}

impl BootLoginRun {
    pub(super) fn begin(catalogs: &BootCatalogInputs) -> Result<Self, BackendError> {
        let directory = Pid1LivePeerDirectory::new();
        let run_owner = directory
            .begin_run_owner_lease()
            .map_err(|_| BackendError::Unavailable)?;
        let identity_ready = IdentityReadyProtocol::from_verified_catalog(catalogs, &run_owner);
        if catalogs.ready_plan.is_some() && identity_ready.is_none() {
            super::log_pid1_stderr(format_args!(
                "nagi-init: development READY plan does not select the boot Identity operation"
            ));
        }
        Ok(Self {
            directory,
            run_owner,
            instances: PidfdProcessInstanceAuthority::new(LinuxPidfdKernel),
            registry: Pid1EndpointOwnerRegistry::new(),
            identity_ready,
        })
    }

    /// Bind the pending service listener to the run that owns its signed plan.
    #[cfg(linux_task24_initial_login_runtime)]
    pub(super) fn prepare_identity_listener(
        &self,
    ) -> Result<PendingSpawnOwnedEndpoint, BackendError> {
        self.identity_ready
            .as_ref()
            .ok_or(BackendError::Unavailable)?
            .prepare_listener(&self.run_owner)
    }
}

/// The single read-only Identity operation selected by the signed Task24 plan.
pub(super) struct IdentityReadyProtocol {
    route: TrustedRouteKey,
    service_id: String,
    catalog_generation: NonZeroU64,
    destination: AppId,
    credentials: (u32, u32),
    attempt: Mutex<Option<IdentityHandshakeAttempt>>,
}

struct IdentityHandshakeAttempt {
    challenge: [u8; CHALLENGE_BYTES],
    confirmed_on_control: bool,
}

impl IdentityReadyProtocol {
    fn from_verified_catalog(
        catalogs: &BootCatalogInputs,
        run: &Pid1RunOwnerLease,
    ) -> Option<Self> {
        let plan = catalogs.ready_plan.as_ref()?;
        let [mandatory] = plan.mandatory_routes() else {
            return None;
        };
        let descriptor = catalogs.catalog.descriptor();
        let service_id = descriptor.service_id().as_str();
        let service_selector = AppId::from_identifier(service_id.as_bytes());
        let operation = AppId::from_identifier(REQUEST_TYPE.as_bytes());
        if descriptor.package_id() != TASK24_IDENTITY_PACKAGE_ID
            || descriptor.generation() != plan.generation()
            || mandatory.service_id.get() != service_selector.0
            || mandatory.operation_id.get() != operation.0
            || (mandatory.version_major, mandatory.version_minor) != (1, 0)
        {
            return None;
        }
        Some(Self {
            route: TrustedRouteKey::from_canonical_route(
                service_selector.0,
                operation.0,
                1,
                0,
                run.generation().get(),
            )?,
            service_id: service_id.to_owned(),
            catalog_generation: plan.generation(),
            destination: AppId::from_identifier(TASK24_IDENTITY_PACKAGE_ID.as_bytes()),
            credentials: descriptor.credentials(),
            attempt: Mutex::new(None),
        })
    }

    pub(super) fn confirm_control_challenge(
        &self,
        challenge: &[u8; CHALLENGE_BYTES],
    ) -> Result<(), PeerDenial> {
        let mut attempt = self
            .attempt
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        let attempt = attempt.as_mut().ok_or(PeerDenial::HandshakeIncomplete)?;
        if attempt.confirmed_on_control || &attempt.challenge != challenge {
            return Err(PeerDenial::HandshakeIncomplete);
        }
        attempt.confirmed_on_control = true;
        Ok(())
    }

    /// Prepare the owner-derived service locator and retain its listener
    /// for C3's pre-exec gate. This does not publish a live registration.
    #[cfg(linux_task24_initial_login_runtime)]
    fn prepare_listener(
        &self,
        run: &Pid1RunOwnerLease,
    ) -> Result<PendingSpawnOwnedEndpoint, BackendError> {
        use sha2::{Digest, Sha256};

        let service = ServiceId::new(self.service_id.as_str())
            .map_err(|_| BackendError::ProtocolViolation)?;
        let version = ContractVersion::new(1, 0).map_err(|_| BackendError::ProtocolViolation)?;
        let operation =
            OperationId::new("readiness").map_err(|_| BackendError::ProtocolViolation)?;
        let label = CapabilityId::new("auth.identity-readiness")
            .map_err(|_| BackendError::ProtocolViolation)?;
        let descriptor = ServiceDescriptor::new(
            service,
            [version],
            [ServiceOperation::new(operation.clone(), Some(label))],
        )
        .map_err(|_| BackendError::ProtocolViolation)?;
        let mut binding = Sha256::new();
        binding.update(b"Nagi Task24 Identity readiness owner v1\0");
        binding.update(self.service_id.as_bytes());
        binding.update(self.catalog_generation.get().to_be_bytes());
        binding.update(run.generation().get().to_be_bytes());
        let mut owner = [0; 16];
        owner.copy_from_slice(&binding.finalize()[..16]);
        let route = TrustedNipcOperationRoute::from_service_contract(
            self.route,
            self.destination.0,
            REQUEST_TYPE,
            &descriptor,
            version,
            &operation,
            ServiceEffectOwnerId::new(owner),
        )
        .map_err(|_| BackendError::ProtocolViolation)?;
        prepare_task24_endpoint_directory()?;
        PendingSpawnOwnedEndpoint::bind_for_spawn(
            run.run(),
            route,
            self.credentials.0,
            self.credentials.1,
        )
        .map_err(|_| BackendError::Unavailable)
    }
}

impl ServiceHandshakeProtocol for IdentityReadyProtocol {
    fn route(&self) -> TrustedRouteKey {
        self.route
    }

    fn system_service_slot(&self) -> Option<(&str, NonZeroU64)> {
        Some((&self.service_id, self.catalog_generation))
    }

    fn source_app_id(&self) -> AppId {
        BROKER_APP
    }

    fn destination_app_id(&self) -> AppId {
        self.destination
    }

    fn request_payload_type(&self) -> &str {
        REQUEST_TYPE
    }

    fn response_payload_type(&self) -> &str {
        RESPONSE_TYPE
    }

    fn ordering(&self) -> OrderingMetadata {
        OrderingMetadata {
            timestamp_micros: None,
            sequence: None,
        }
    }

    fn build_request_payload(
        &self,
        ids: ServiceHandshakeAttemptIds,
    ) -> Result<Vec<u8>, PeerDenial> {
        let mut attempt = self
            .attempt
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        if attempt.is_some() {
            return Err(PeerDenial::HandshakeIncomplete);
        }
        *attempt = Some(IdentityHandshakeAttempt {
            challenge: readiness_challenge(ids.request_id(), ids.correlation_id()),
            confirmed_on_control: false,
        });
        Ok(Vec::new())
    }

    fn validate_response_payload(&self, payload: &[u8]) -> Result<(), PeerDenial> {
        let attempt = self
            .attempt
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        if !attempt.as_ref().is_some_and(|attempt| {
            attempt.confirmed_on_control && attempt.challenge.as_slice() == payload
        }) {
            return Err(PeerDenial::HandshakeIncomplete);
        }
        Ok(())
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
pub(super) type BootLoginIssuer<'boot> = C1InitialLoginAuthority<
    'boot,
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
    #[cfg(feature = "system-service-readiness")] identity_listener: Option<
        &identity_listener::PendingIdentityListener,
    >,
    console: &mut super::linux_owner_registration_console::BootOwnerConsole,
) -> ! {
    use super::linux_owner_registration_console::task24_input::RequestRoute;
    use std::io::Read;

    #[cfg(feature = "system-service-readiness")]
    let mut lifecycle_ready = false;
    #[cfg(all(linux_task24_notes_consumer, not(all(linux_task24_desktop_shell, not(nagi_release_build)))))]
    let mut retained_notes_draft = None;
    #[cfg(all(linux_task24_notes_consumer, linux_task24_desktop_shell, not(nagi_release_build)))]
    let mut retained_notes_draft = Vec::new();
    let mut execute = || -> Result<(), BackendError> {
        #[cfg(all(linux_dev_ws10, not(nagi_release_build), linux_task24_notes_consumer))]
        let ws10 = ws10_mode().map_err(|_| BackendError::ProtocolViolation)?;
        #[cfg(all(linux_dev_ws10, not(nagi_release_build), linux_task24_notes_consumer))]
        if ws10 == Some(Ws10Mode::StopIdentity) && run.identity_ready.is_none() {
            return Err(BackendError::Unavailable);
        }
        #[cfg(all(linux_dev_ws10, not(nagi_release_build), linux_task24_notes_consumer))]
        if ws10 == Some(Ws10Mode::CheckPreviousLogout) {
            runtime
                .ws10_check_previous_logout()
                .map_err(|_| BackendError::Unavailable)?;
        }
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
        #[cfg(feature = "system-service-readiness")]
        let handshakes =
            Pid1CompletedServiceHandshakeOwner::new(run.run_owner.run(), &run.instances);
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
                console,
                &mut unlock,
                input,
            )
            .map_err(|_| BackendError::Unavailable)?;
        let reconcile_after_identity_exit = || match runtime.poll_task24_required_identity_exit() {
            Ok(false) => runtime.reconcile_after_normal_exit(package.owner),
            Ok(true) => Err(std::io::Error::other(
                "required Identity exited before reconciliation",
            )),
            Err(error) => Err(error),
        };
        let reconcile_normal_exit = || {
            match reconcile_after_identity_exit() {
                Ok(receipt) => {
                    super::linux_c3_boot_launch::linux_c1_boot_recovery::log_task24_c3_normal_exit_receipt(
                        "NAGI_DEV_TASK24_IDENTITY_TERMINAL_CONSUMED",
                        &receipt,
                    )
                }
                Err(error) => {
                    super::log_pid1_stderr(format_args!(
                        "NAGI_DEV_TASK24_IDENTITY_TERMINAL_UNAVAILABLE ({error})"
                    ));
                    super::wait_in_pid1(super::wait_for_pid1_os_event);
                }
            }
        };
        match attached {
            Ok(LoginCompletion::Attached(session)) => {
                super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_SESSION_ATTACHED"));
                let notes_allowed = if run.identity_ready.is_some() {
                    #[cfg(feature = "system-service-readiness")]
                    let readiness = identity_listener
                        .ok_or(PeerDenial::RequiredServiceMissing)
                        .and_then(|identity| {
                            let deadline = Instant::now() + Duration::from_secs(5);
                            identity.with_ready_owner(|lease, spawn_proof, supervisor_record| {
                                runtime.complete_and_publish_task24_ready(
                                    session.lease,
                                    run,
                                    lease,
                                    spawn_proof,
                                    supervisor_record,
                                    &handshakes,
                                    deadline,
                                    || {
                                        super::publish_lifecycle_ready(&mut lifecycle_ready)
                                            .map_err(|_| PeerDenial::Unavailable)
                                    },
                                )
                            })
                        });
                    #[cfg(not(feature = "system-service-readiness"))]
                    let readiness: Result<(), PeerDenial> = Err(PeerDenial::Unavailable);
                    if let Err(error) = &readiness {
                        super::log_pid1_stderr(format_args!(
                            "NAGI_DEV_TASK24_IDENTITY_READY_UNAVAILABLE ({error:?})"
                        ));
                    }
                    readiness.is_ok()
                } else {
                    true
                };
                #[cfg(linux_task24_notes_consumer)]
                let mut notes_exit = None;
                if notes_allowed {
                    let mut notes_owner = runtime.notes_owner().map_err(|_| {
                        crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError::Unavailable
                    });
                    match notes_owner
                        .as_mut()
                        .map_err(|error| *error)
                        .and_then(|owner| runtime.request_notes_attachment(session.lease, owner))
                    {
                        Ok(inputs) => {
                            super::log_pid1_stderr(format_args!(
                                "NAGI_NOTES_PROFILE_ATTACHMENT_CHECKED"
                            ));
                            #[cfg(linux_task24_notes_consumer)]
                            {
                                let issued = std::sync::Arc::clone(&inputs.2);
                                #[cfg(all(linux_dev_ws10, not(nagi_release_build)))]
                                let mut revoked = false;
                                #[allow(unused_mut)]
                                let mut run_notes = |console: &mut super::linux_owner_registration_console::BootOwnerConsole, inputs| notes_console::run(
                                    console,
                                    inputs,
                                    || runtime.poll_task24_required_identity_exit(),
                                    || runtime.check_notes_session(session.lease),
                                    #[cfg(all(linux_dev_ws10, not(nagi_release_build)))]
                                    || match ws10 {
                                        Some(Ws10Mode::StopIdentity) => {
                                            runtime.ws10_stop_identity()?;
                                            super::log_pid1_stderr(format_args!(
                                                "NAGI_DEV_WS10_IDENTITY_REAPED signal=SIGKILL"
                                            ));
                                            Ok(Some(
                                                super::NotesConsoleExit::RequiredIdentityExited,
                                            ))
                                        }
                                        Some(Ws10Mode::RevokeSession) if !revoked => {
                                            let reason =
                                                runtime.ws10_revoke_notes_session(&issued)?;
                                            revoked = true;
                                            super::log_pid1_stderr(format_args!(
                                                "NAGI_DEV_WS10_SESSION_REVOKED reason={reason:?}"
                                            ));
                                            Ok(None)
                                        }
                                        _ => Ok(None),
                                    },
                                );
                                #[cfg(not(all(linux_task24_desktop_shell, not(nagi_release_build))))]
                                let result = run_notes(console, inputs);

                                #[cfg(all(linux_task24_desktop_shell, not(nagi_release_build)))]
                                let result = {
                                    use desktop_shell::ShellExit;
                                    use super::NotesConsoleExit;
                                    let mut first = Some(inputs);
                                    desktop_shell::run(
                                        console,
                                        || runtime.poll_task24_required_identity_exit(),
                                        || runtime.check_notes_session(session.lease),
                                        |console| {
                                            let inputs = match first.take() {
                                                Some(inputs) => inputs,
                                                None => {
                                                    let owner = notes_owner.as_mut().map_err(|error| std::io::Error::other(*error))?;
                                                    let inputs = runtime.request_notes_attachment(session.lease, owner).map_err(|error| {
                                                        super::log_pid1_stderr(format_args!(
                                                            "NAGI_DEV_DESKTOP_NOTES_ATTACHMENT_REISSUE_FAILED ({error})"
                                                        ));
                                                        std::io::Error::other(error)
                                                    })?;
                                                    super::log_pid1_stderr(format_args!(
                                                        "NAGI_DEV_DESKTOP_NOTES_ATTACHMENT_REISSUED"
                                                    ));
                                                    inputs
                                                }
                                            };
                                            run_notes(console, inputs)
                                        },
                                        &mut retained_notes_draft,
                                    )
                                    .map(|exit| {
                                        let notes_exit = match exit {
                                            ShellExit::Logout => NotesConsoleExit::ClosedByUser,
                                            ShellExit::RequiredIdentityExited => NotesConsoleExit::RequiredIdentityExited,
                                            ShellExit::StateUnknown => NotesConsoleExit::StateUnknown,
                                        };
                                        (notes_exit, None::<notes_files_codec_candidate::NoteDocument>)
                                    })
                                };
                                let (exit, draft) = match result {
                                    Ok(exit) => exit,
                                    Err(error) => {
                                        super::log_pid1_stderr(format_args!(
                                            "NAGI_DEV_NOTES_CONSOLE_UNAVAILABLE ({error})"
                                        ));
                                        (super::NotesConsoleExit::StateUnknown, None)
                                    }
                                };
                                #[cfg(not(all(linux_task24_desktop_shell, not(nagi_release_build))))]
                                {
                                    retained_notes_draft = draft;
                                }
                                #[cfg(all(linux_task24_desktop_shell, not(nagi_release_build)))]
                                debug_assert!(draft.is_none(), "the shell retains its own drafts");
                                if matches!(
                                    exit,
                                    super::NotesConsoleExit::ClosedByUser
                                        | super::NotesConsoleExit::ClosedWithoutSave
                                ) {
                                    match runtime.end_notes_session(&issued) {
                                        Ok(reason) => super::log_pid1_stderr(format_args!(
                                            "NAGI_DEV_NOTES_SESSION_ENDED reason={reason:?}"
                                        )),
                                        Err(_) => {
                                            super::log_pid1_stderr(format_args!(
                                                "NAGI_DEV_NOTES_SESSION_END_UNCONFIRMED"
                                            ));
                                            // Keep the login control FD alive. EOF must only follow
                                            // confirmed C1 termination on this close path.
                                            super::wait_in_pid1(super::wait_for_pid1_os_event);
                                        }
                                    }
                                }
                                notes_exit = Some(exit);
                            }
                            #[cfg(not(linux_task24_notes_consumer))]
                            let _ = inputs;
                        }
                        Err(error) => {
                            super::log_pid1_stderr(format_args!(
                                "NAGI_NOTES_PROFILE_VOLUME_UNAVAILABLE error={error} fallback=none"
                            ));
                        }
                    };
                }
                let normal_exit = close_attached_login(session, &mut lifecycle, &owners);
                // Abnormal or unknown Notes exit leaves durable state for recovery.
                #[cfg(linux_task24_notes_consumer)]
                let normal_exit = super::NotesConsoleExit::allows_normal_login_reconciliation(
                    notes_exit,
                    normal_exit,
                );
                if normal_exit {
                    reconcile_normal_exit();
                }
            }
            Ok(LoginCompletion::ThrottleResetFailed(session)) => {
                super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_TASK24_LOGIN_THROTTLE_PERSISTENCE_FAILED stage=reset"
                ));
                if close_attached_login(session, &mut lifecycle, &owners) {
                    reconcile_normal_exit();
                }
            }
            Ok(LoginCompletion::Cancelled(_)) => {
                super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_LOGIN_CANCELLED_NORMALLY"));
                reconcile_normal_exit();
            }
            Err(error) => {
                super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_TASK24_LOGIN_UNAVAILABLE ({error:?})"
                ));
                // A bounded retirement failure can leave the exact child
                // and broker lease in this lifecycle's quarantine.
                wait_for_login_cleanup(&mut lifecycle, &owners);
            }
        }
        Ok(())
    };
    if execute().is_err() {
        super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_LOGIN_STOPPED"));
    }
    super::wait_in_pid1(move || {
        #[cfg(linux_task24_notes_consumer)]
        let _retained_draft = &retained_notes_draft;
        super::wait_for_pid1_os_event();
    })
}

#[cfg(linux_task24_initial_login_runtime)]
pub(super) struct AttachedLogin {
    lease: SessionLease,
    control: std::os::unix::net::UnixStream,
}

#[cfg(linux_task24_initial_login_runtime)]
pub(super) enum LoginCompletion {
    Attached(AttachedLogin),
    ThrottleResetFailed(AttachedLogin),
    Cancelled(VerifiedLoginCancellation),
}

#[cfg(linux_task24_initial_login_runtime)]
fn wait_for_rejected_login_cleanup(
    lifecycle: &mut LoginLifecycle,
    owners: &EndpointOwners<'_>,
    deadline: Instant,
) -> LoginCleanupWait {
    wait_for_login_cleanup_until(
        deadline,
        || {
            if lifecycle.active_len() != 0 {
                // Keep using this lifecycle's one reaper and the exact C2
                // record/endpoint retirement callbacks. Errors remain pending
                // only while the same child stays owned here.
                let _ = lifecycle.poll_with_launch_lease(revoke_login_record, |lease, _status| {
                    owners.retire(lease)
                });
            }
            if lifecycle.active_len() != 0 {
                return LoginCleanupPoll::Pending;
            }
            match owners.run.instances.has_unresolved_cleanup() {
                Ok(false) => LoginCleanupPoll::Complete,
                Ok(true) | Err(_) => LoginCleanupPoll::Unknown,
            }
        },
        |remaining| {
            // Re-poll the same owned child at a short bounded cadence so this
            // wait cannot overshoot the login attempt's absolute deadline.
            std::thread::park_timeout(remaining.min(std::time::Duration::from_millis(10)));
        },
    )
}

#[cfg(linux_task24_initial_login_runtime)]
fn close_attached_login(
    session: AttachedLogin,
    lifecycle: &mut LoginLifecycle,
    owners: &EndpointOwners<'_>,
) -> bool {
    // Keep the control socket until the exact signed child is reaped and retired.
    let eof_sent = match session.control.shutdown(std::net::Shutdown::Write) {
        Ok(()) => true,
        Err(error) => {
            super::log_pid1_stderr(format_args!(
                "NAGI_DEV_TASK24_CONTROL_EOF_UNAVAILABLE ({})",
                error.kind()
            ));
            false
        }
    };
    wait_for_login_cleanup(lifecycle, owners) && eof_sent
}

#[cfg(linux_task24_initial_login_runtime)]
fn wait_for_login_cleanup(lifecycle: &mut LoginLifecycle, owners: &EndpointOwners<'_>) -> bool {
    let mut cleanup_warned = false;
    let mut normal_exit = false;
    while lifecycle.active_len() != 0 {
        super::wait_for_pid1_os_event();
        match lifecycle
            .poll_with_launch_lease(revoke_login_record, |lease, _status| owners.retire(lease))
        {
            Ok(Some(super::LifecycleEvent::ServiceExited { status, .. })) => {
                normal_exit = status.exit_code == Some(0) && status.signal.is_none();
                super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_TASK24_CHILD_REAPED_AND_RETIRED status={status:?}"
                ));
            }
            Err(error) if !cleanup_warned => {
                cleanup_warned = true;
                super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_CLEANUP_PENDING ({error:?})"));
            }
            _ => {}
        }
    }
    normal_exit
}

#[cfg(linux_task24_initial_login_runtime)]
#[derive(Debug)]
enum LoginStartError {
    DeadlineExpired,
    Backend(BackendError),
}

#[cfg(linux_task24_initial_login_runtime)]
impl LoginStartError {
    fn from_spawn(error: BackendError, deadline_cancelled_spawn: bool) -> Self {
        if error == BackendError::Cancelled && deadline_cancelled_spawn {
            Self::DeadlineExpired
        } else {
            Self::Backend(error)
        }
    }

    fn into_denial(self) -> PeerDenial {
        match self {
            Self::DeadlineExpired => PeerDenial::RequestDeadlineExpired,
            Self::Backend(BackendError::Revoked) => PeerDenial::NotLive,
            Self::Backend(BackendError::ConsentDenied) => PeerDenial::ConsentDenied,
            Self::Backend(_) => PeerDenial::Unavailable,
        }
    }
}

#[cfg(linux_task24_initial_login_runtime)]
impl From<BackendError> for LoginStartError {
    fn from(error: BackendError) -> Self {
        Self::Backend(error)
    }
}

/// Start and check out the exact child while the caller retains the recovered
/// C1 issuer, signed package, and C3 process lifecycle.
#[cfg(linux_task24_initial_login_runtime)]
pub(super) fn start_verified_login(
    lifecycle: &mut LoginLifecycle,
    selection: &LoginChildLaunch,
    package: &VerifiedLoginChild<'_>,
    route: &BootInitialLoginRoute,
    owners: &EndpointOwners<'_>,
    issuer: &BootLoginIssuer<'_>,
    submit: impl FnOnce(
        &mut std::os::unix::net::UnixStream,
    )
        -> Result<(), super::linux_owner_registration_console::UnlockSubmissionError>,
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
                    use super::linux_owner_registration_console::UnlockSubmissionError;
                    submit(stream).map_err(|error| match error {
                        UnlockSubmissionError::DeadlineExpired => LoginStartError::DeadlineExpired,
                        UnlockSubmissionError::Unavailable => BackendError::Unavailable.into(),
                    })
                },
                deadline,
            )
            .map_err(LoginStartError::into_denial)
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
            control: input_channel.ok_or(PeerDenial::RequestOutcomeUnknown)?,
        }),
        Err(error) => {
            // Preserve the admission failure even if bounded cleanup has
            // an independently unresolved outcome.
            if !matches!(error, PeerDenial::AccountRejected) {
                super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_TASK24_INITIAL_ADMISSION_FAILED ({error:?})"
                ));
            }
            let cleanup = lifecycle
                .shutdown_with_launch_lease(revoke_login_record, |lease, _| owners.retire(lease));
            let mut cleanup_confirmed = cleanup.is_ok()
                && !owners
                    .run
                    .instances
                    .has_unresolved_cleanup()
                    .unwrap_or(true);
            let may_retry_after_cleanup = matches!(
                &error,
                PeerDenial::AccountRejected | PeerDenial::RequestDeadlineExpired
            );
            if !cleanup_confirmed && may_retry_after_cleanup {
                cleanup_confirmed = matches!(
                    wait_for_rejected_login_cleanup(lifecycle, owners, deadline),
                    LoginCleanupWait::Complete
                );
                if !cleanup_confirmed {
                    super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_CLEANUP_PENDING"));
                }
            }
            let error = login_attempt_denial_after_cleanup(error, cleanup_confirmed);
            Err(error)
        }
    }
}

#[cfg(linux_task24_initial_login_runtime)]
fn login_attempt_denial_after_cleanup(error: PeerDenial, cleanup_confirmed: bool) -> PeerDenial {
    classify_login_denial_after_cleanup(
        error,
        cleanup_confirmed,
        |error| {
            matches!(
                error,
                PeerDenial::AccountRejected | PeerDenial::RequestDeadlineExpired
            )
        },
        PeerDenial::UnresolvedSpawnCleanup,
        PeerDenial::RequestOutcomeUnknown,
    )
}

#[cfg(all(test, linux_task24_initial_login_runtime))]
mod login_attempt_cleanup_tests {
    use super::*;

    #[test]
    fn only_an_explicit_attempt_deadline_is_retryable() {
        use PeerDenial::*;
        assert!(matches!(
            LoginStartError::from_spawn(BackendError::Cancelled, true).into_denial(),
            RequestDeadlineExpired
        ));
        assert!(matches!(
            LoginStartError::from_spawn(BackendError::Cancelled, false).into_denial(),
            Unavailable
        ));
        for error in [
            BackendError::IdentityRejected,
            BackendError::TransportFailure,
            BackendError::ProtocolViolation,
            BackendError::Unavailable,
        ] {
            assert!(matches!(
                LoginStartError::from_spawn(error, true).into_denial(),
                Unavailable
            ));
        }
        assert!(matches!(
            LoginStartError::from_spawn(BackendError::Revoked, true).into_denial(),
            NotLive
        ));
        assert!(matches!(
            LoginStartError::from_spawn(BackendError::ConsentDenied, true).into_denial(),
            ConsentDenied
        ));
    }

    #[test]
    fn expired_attempt_returns_to_input_only_after_exact_cleanup() {
        use PeerDenial::*;
        assert!(matches!(
            login_attempt_denial_after_cleanup(RequestDeadlineExpired, true),
            RequestDeadlineExpired
        ));
        assert!(matches!(
            login_attempt_denial_after_cleanup(RequestDeadlineExpired, false),
            UnresolvedSpawnCleanup
        ));
        assert!(matches!(
            login_attempt_denial_after_cleanup(RequestOutcomeUnknown, true),
            RequestOutcomeUnknown
        ));
    }
}

#[cfg(linux_task24_initial_login_runtime)]
fn start_and_borrow<'slot>(
    lifecycle: &'slot mut LoginLifecycle,
    selection: &LoginChildLaunch,
    package: &VerifiedLoginChild<'_>,
    route: &BootInitialLoginRoute,
    owners: &EndpointOwners<'_>,
    receive_control: impl FnOnce(std::os::unix::net::UnixStream) -> Result<(), LoginStartError>,
    deadline: Instant,
) -> Result<&'slot LoginLease, LoginStartError> {
    let release_wait_budget = deadline.saturating_duration_since(Instant::now());
    if release_wait_budget.is_zero() {
        return Err(LoginStartError::DeadlineExpired);
    }
    let app_id = package.generation.snapshot().app_id();
    let readiness = owners.run.run_owner.readiness_boundary();
    if package.manifest.app_id() != app_id {
        super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_LAUNCH_PACKAGE_ID_MISMATCH"));
        return Err(BackendError::IdentityRejected.into());
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
        route.route.clone(),
        selection.credentials.uid,
        selection.credentials.gid,
    )
    .map_err(|_| BackendError::Unavailable)?;
    let argv = pending
        .child_arguments(&[])
        .map_err(|_| BackendError::ProtocolViolation)?;
    let arguments: Vec<&str> = argv.iter().map(String::as_str).collect();
    let launch = ProcessLaunch {
        service: selection.service.clone(),
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
    let mut deadline_cancelled_spawn = false;
    let pid = lifecycle
        .spawn_authorized_with_prepared_and_launch_lease(
            launch,
            || {
                deadline_cancelled_spawn = Instant::now() >= deadline;
                deadline_cancelled_spawn
            },
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
            LoginStartError::from_spawn(error, deadline_cancelled_spawn)
        })?;
    super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_CHILD_EXEC_ACK"));
    let proof = boot_proof.ok_or(BackendError::ProtocolViolation)?;
    let record = record.ok_or(BackendError::ProtocolViolation)?;
    let activation = if Instant::now() >= deadline {
        Err(LoginStartError::DeadlineExpired)
    } else {
        lifecycle
            .activate_current_launch_lease(&proof, record, |lease, attestation| {
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
            .map_err(LoginStartError::from)
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
    if remaining.is_zero() {
        return Err(LoginStartError::DeadlineExpired);
    }
    let ack_limit = Duration::from_secs(2);
    channel
        .set_read_timeout(Some(remaining.min(ack_limit)))
        .map_err(|_| BackendError::Unavailable)?;
    let mut ready = [0u8; 1];
    channel.read_exact(&mut ready).map_err(|error| {
        use std::io::ErrorKind;
        let window_expired = error.kind() == ErrorKind::TimedOut
            || (error.kind() == ErrorKind::WouldBlock && Instant::now() >= deadline);
        if remaining <= ack_limit && window_expired {
            LoginStartError::DeadlineExpired
        } else {
            BackendError::Unavailable.into()
        }
    })?;
    if ready != [super::linux_owner_registration_console::task24_input::CONNECTED] {
        return Err(BackendError::ProtocolViolation.into());
    }
    super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_CHILD_BROKER_CONNECTED"));
    receive_control(channel)?;
    lifecycle
        .borrow_current_launch_lease(&proof, record)
        .map_err(LoginStartError::from)
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

#[cfg(all(linux_dev_ws10, not(nagi_release_build), linux_task24_notes_consumer))]
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ws10Mode {
    StopIdentity,
    RevokeSession,
    CheckPreviousLogout,
}

#[cfg(all(linux_dev_ws10, not(nagi_release_build), linux_task24_notes_consumer))]
fn ws10_mode() -> std::io::Result<Option<Ws10Mode>> {
    let command_line = std::fs::read_to_string("/proc/cmdline")?;
    let mut selected = None;
    for value in command_line
        .split_ascii_whitespace()
        .filter_map(|argument| argument.strip_prefix("nagi.dev_ws10="))
    {
        let mode = match value {
            "stop-identity" => Ws10Mode::StopIdentity,
            "revoke-session" => Ws10Mode::RevokeSession,
            "check-previous-logout" => Ws10Mode::CheckPreviousLogout,
            _ => return Err(std::io::Error::other("unknown WS-10 development mode")),
        };
        if selected.replace(mode).is_some() {
            return Err(std::io::Error::other("duplicate WS-10 development mode"));
        }
    }
    Ok(selected)
}
