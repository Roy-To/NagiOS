use super::super::linux_init_c3_slot_journal::{C3DurableSlotReceipt, C3SlotState};
#[cfg(linux_task24_initial_login_runtime)]
use super::super::{
    linux_initial_login_c2::C1InitialLoginAuthority,
    linux_initial_login_owner::LinuxInitialLoginOwner,
};
use super::AuthenticatedBootService;
use crate::profile_volume_notes_owner::BootVolumeInput;
use nagi_identity_session::{
    c1_linux_boot_identity_service_from_store, C1LinuxBootIdentityService,
    C1RecoveryOperationIdentity, C1SystemServiceRecoveryEffectGate,
    C1SystemServiceRecoveryReceiptAdapter, C1SystemServiceRuntimeHandoff,
    C1VerifiedBootSystemServicePrincipal, Clock, GuestProfileCleaner, IdentityAuthorizer,
    IdentityError, IdentityEventSink, IdentityIdSource, IdentityService,
    LinuxIdentitySnapshotStore, RecoveryEffectGateError,
};
#[cfg(linux_task24_initial_login_runtime)]
use nagi_identity_session::{IdempotencyKey, UnixMillis};
use nagi_linux_account_verifier::LinuxAccountVerifier;
use nagi_system_service_recovery_contract::{
    C1RecoveryOperationCorrelation, SystemServiceRecoveryAdmissionScope,
    VerifiedRecoveryBootCatalog,
};
use std::io;
use std::path::Path;

/// Seal the caller while the exact C3 slot is live. The owned principal only
/// identifies the caller for C1; the recovery effect still needs its own gate.
pub(super) fn seal_boot_service_principal(
    boot: &AuthenticatedBootService<'_>,
    catalog: &VerifiedRecoveryBootCatalog,
) -> io::Result<C1VerifiedBootSystemServicePrincipal> {
    boot.with_verified_system_mapping(catalog, |binding| {
        C1VerifiedBootSystemServicePrincipal::from_live_binding(&binding)
    })?
    .map_err(|error| io::Error::new(io::ErrorKind::PermissionDenied, format!("{error:?}")))
}

#[derive(Debug)]
pub(in crate::linux_init) enum BootIdentityRecoveryError {
    Principal(io::Error),
    Data(io::Error),
    Recovery(IdentityError),
    LiveEffect(io::Error),
}

pub(in crate::linux_init) struct SelectedBootIdentityRuntime<'boot, 'package> {
    boot: &'boot AuthenticatedBootService<'package>,
    #[cfg(all(
        linux_task24_owner_co_location,
        linux_task24_initial_login_runtime,
        feature = "desktop-login"
    ))]
    data: &'boot BootVolumeInput,
    // Retain the selected account verifier beside the recovered snapshot store
    // while the login owner is unavailable.
    verifier: LinuxAccountVerifier,
    #[cfg(all(linux_boot_owner_registration, feature = "desktop-login"))]
    principal: C1VerifiedBootSystemServicePrincipal,
    identity: C1LinuxBootIdentityService,
    handoff: C1SystemServiceRuntimeHandoff,
    receipt: C3DurableSlotReceipt,
}

impl<'package> AuthenticatedBootService<'package> {
    pub(in crate::linux_init) fn recover_selected_identity_data<'boot>(
        &'boot self,
        catalog: &VerifiedRecoveryBootCatalog,
        data: &'boot BootVolumeInput,
        snapshot_path: &Path,
        account_path: &Path,
    ) -> Result<SelectedBootIdentityRuntime<'boot, 'package>, BootIdentityRecoveryError> {
        let principal = seal_boot_service_principal(self, catalog)
            .map_err(BootIdentityRecoveryError::Principal)?;
        let (store, verifier) = open_boot_identity(data, snapshot_path, account_path)
            .map_err(BootIdentityRecoveryError::Data)?;
        let (mut identity, _event_reader) =
            c1_linux_boot_identity_service_from_store(store, &principal);
        let (handoff, receipt) = recover_boot_identity(self, &principal, &mut identity)?;
        Ok(SelectedBootIdentityRuntime {
            boot: self,
            #[cfg(all(
                linux_task24_owner_co_location,
                linux_task24_initial_login_runtime,
                feature = "desktop-login"
            ))]
            data,
            verifier,
            #[cfg(all(linux_boot_owner_registration, feature = "desktop-login"))]
            principal,
            identity,
            handoff,
            receipt,
        })
    }
}

impl SelectedBootIdentityRuntime<'_, '_> {
    pub(in crate::linux_init) fn verify_continuing_admission(
        &mut self,
    ) -> Result<(), BootIdentityRecoveryError> {
        let admission = self
            .identity
            .enter_system_service_runtime_admission(&self.handoff, self.boot.recovery_scope())
            .map_err(BootIdentityRecoveryError::Recovery)?;
        let scope = admission.scope();
        let claim = correlated_c3_claim(self.boot, scope, admission.operation_identity())
            .map_err(BootIdentityRecoveryError::LiveEffect)?;
        let ((), receipt) = self
            .boot
            .owner
            .c3
            .with_durable_live_continuation_gate(scope, &self.boot.proof, claim, || {
                self.identity
                    .validate_system_service_runtime_handoff(&self.handoff)
                    .map_err(|error| io::Error::other(format!("{error:?}")))
            })
            .map_err(BootIdentityRecoveryError::LiveEffect)?;
        require_correlated_live_receipt(scope, claim, &receipt)
            .map_err(BootIdentityRecoveryError::LiveEffect)?;
        self.receipt = receipt;
        drop(admission);
        Ok(())
    }

    #[cfg(all(linux_boot_owner_registration, feature = "desktop-login"))]
    pub(in crate::linux_init) fn run_owner_registration(
        &mut self,
        console: &mut super::super::linux_owner_registration_console::BootOwnerConsole,
    ) -> io::Result<()> {
        let Self {
            boot,
            verifier,
            principal,
            identity,
            handoff,
            receipt,
            ..
        } = self;
        let screen = crate::login_screen::selected_data_owner_registration(
            verifier,
            nagi_localization::Locale::EnUs,
            |account_name| {
                let admission = identity
                    .enter_system_service_runtime_admission(handoff, boot.recovery_scope())
                    .inspect_err(|_| {
                        super::super::log_pid1_stderr(format_args!(
                            "NAGI_DEV_OWNER_REGISTRATION_DENIED stage=c1-admission"
                        ));
                    })
                    .map_err(|_| io::Error::other("owner registration admission denied"))?;
                let scope = admission.scope();
                let claim = correlated_c3_claim(boot, scope, admission.operation_identity())
                    .inspect_err(|_| {
                        super::super::log_pid1_stderr(format_args!(
                            "NAGI_DEV_OWNER_REGISTRATION_DENIED stage=c1-c3-correlation"
                        ));
                    })?;
                let (binding, current_receipt) = boot
                    .owner
                    .c3
                    .with_durable_live_continuation_gate(scope, &boot.proof, claim, || {
                        nagi_identity_session::provision_c1_linux_first_owner_binding(
                            identity,
                            principal,
                            account_name,
                        )
                        .inspect_err(|error| {
                            // The C1 provisioning error is an enum with no account data.
                            super::super::log_pid1_stderr(format_args!(
                                "NAGI_DEV_OWNER_REGISTRATION_DENIED stage=c1-provision error={error:?}"
                            ));
                        })
                        .map_err(|_| io::Error::other("C1 owner provisioning denied"))
                    })
                    .inspect_err(|error| {
                        super::super::log_pid1_stderr(format_args!(
                            "NAGI_DEV_OWNER_REGISTRATION_DENIED stage=c3-continuation kind={}",
                            error.kind()
                        ));
                    })?;
                require_correlated_live_receipt(scope, claim, &current_receipt).inspect_err(
                    |_| {
                        super::super::log_pid1_stderr(format_args!(
                            "NAGI_DEV_OWNER_REGISTRATION_DENIED stage=c3-receipt"
                        ));
                    },
                )?;
                *receipt = current_receipt;
                Ok::<_, io::Error>(binding)
            },
        )
        .map_err(|error| io::Error::other(format!("{error:?}")))?;
        console.run_registration(screen).map_err(|error| {
            use super::super::linux_owner_registration_console::RegistrationConsoleError;
            use crate::login_screen::LinuxRegistrationError;
            let stage = match error {
                RegistrationConsoleError::Io(_) => "console",
                RegistrationConsoleError::CredentialUnavailable => "derive",
                RegistrationConsoleError::Save(LinuxRegistrationError::Identity(_)) => "identity",
                RegistrationConsoleError::Save(LinuxRegistrationError::Save(_)) => "account-store",
            };
            super::super::log_pid1_stderr(format_args!(
                "NAGI_DEV_OWNER_REGISTRATION_DENIED stage={stage}"
            ));
            io::Error::other("owner registration failed")
        })
    }
}

#[cfg(all(
    linux_task24_owner_co_location,
    linux_task24_initial_login_runtime,
    feature = "desktop-login"
))]
mod task24 {
    use super::*;
    use crate::linux_init::{
        linux_task24_endpoint_launch::{
            start_verified_login, AttachedLogin, BootInitialLoginRoute, BootLoginRun,
            BootLoginSupervisor, EndpointOwners, LoginChildLaunch, LoginLifecycle,
            VerifiedLoginChild,
        },
        pid1_live_peer_supplier::{
            CompletedServiceHandshakeAuthority, PeerDenial, Pid1LifecycleReadyAdapter,
            Pid1ReadinessAtomicBoundary, RequiredServiceSet, SessionLease, TrustedInventory,
            TrustedServiceEndpointAuthority,
        },
        publish_lifecycle_ready,
    };
    use std::time::Instant;

    type Runtime<'boot, 'package> = RecoveredBootIdentity<
        'boot,
        'package,
        nagi_identity_session::LinuxIdentityIdSource,
        nagi_identity_session::LinuxIdentityClock,
        nagi_identity_session::C1LinuxBootIdentityAuthorizer,
        nagi_identity_session::C1LinuxGuestProfileCleanupUnavailable,
        nagi_identity_session::LinuxIdentityEventSink,
    >;

    impl Runtime<'_, '_> {
        /// Publish the normal init transition inside the C2 owner gate and
        /// the exact attached C1 Session effect. The boot plan and handshake
        /// authorities are supplied by their owners.
        pub(in crate::linux_init) fn publish_task24_ready(
            &self,
            attached: SessionLease,
            owners: &EndpointOwners<'_>,
            required: &RequiredServiceSet,
            inventory: &impl TrustedInventory,
            endpoints: &impl TrustedServiceEndpointAuthority,
            handshakes: &impl CompletedServiceHandshakeAuthority,
            ready: &mut bool,
        ) -> Result<(), PeerDenial> {
            let run = owners.run;
            let event = run.directory.ready_event(
                &run.run_owner,
                required,
                inventory,
                &self.identity,
                owners.consent,
                endpoints,
                &run.instances,
                handshakes,
                &BootLoginSupervisor,
                run.run_owner.readiness_boundary(),
            )?;
            Pid1LifecycleReadyAdapter.map_current(Some(&run.run_owner), &event, |_| {
                self.identity.with_ready_session(attached, || {
                    publish_lifecycle_ready(ready).map_err(|_| PeerDenial::Unavailable)
                })
            })?
        }
    }

    impl<'boot, 'package> SelectedBootIdentityRuntime<'boot, 'package> {
        pub(in crate::linux_init) fn prepare_task24_route(
            &self,
            run: &BootLoginRun,
        ) -> Result<BootInitialLoginRoute, PeerDenial> {
            self.identity
                .validate_system_service_runtime_handoff(&self.handoff)
                .map_err(|_| PeerDenial::Unavailable)?;
            run.run_owner
                .readiness_boundary()
                .with_owner_gate(run.run_owner.run(), None, || {
                    BootInitialLoginRoute::from_recovered_boot(self.boot, run)
                })
        }

        pub(in crate::linux_init) fn owner_login_name(&self) -> io::Result<Vec<u8>> {
            self.verifier
                .owner_login_name()
                .map_err(|_| io::Error::other("owner account unavailable"))
        }

        pub(in crate::linux_init) fn owner_registration_required(&self) -> io::Result<bool> {
            use nagi_linux_account_verifier::OwnerAccountSaveError;
            match self.verifier.require_unprovisioned_private_directory() {
                Ok(()) => Ok(true),
                Err(OwnerAccountSaveError::AlreadyExists) => Ok(false),
                Err(error) => Err(io::Error::other(format!("{error:?}"))),
            }
        }

        /// Reuse the recovered store and its exact C3 handoff for the first
        /// checkout. The process lifecycle remains owned by the caller.
        pub(in crate::linux_init) fn start_task24_login(
            self,
            lifecycle: &mut LoginLifecycle,
            selection: LoginChildLaunch,
            package: &VerifiedLoginChild<'_>,
            route: BootInitialLoginRoute,
            owners: &EndpointOwners<'_>,
            request_key: IdempotencyKey,
            expires_at: Option<UnixMillis>,
            submit: impl FnOnce(&mut std::os::unix::net::UnixStream) -> io::Result<()>,
            deadline: Instant,
        ) -> Result<
            (Runtime<'boot, 'package>, Result<AttachedLogin, PeerDenial>),
            BootIdentityRecoveryError,
        > {
            if !route.matches(self.boot, owners.run) {
                return Err(BootIdentityRecoveryError::LiveEffect(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "initial login route belongs to a different boot run",
                )));
            }
            let runtime = RecoveredBootIdentity {
                boot: self.boot,
                data: self.data,
                receipt: self.receipt,
                identity: C1InitialLoginAuthority::new(
                    self.identity,
                    self.handoff,
                    self.verifier,
                    request_key,
                    expires_at,
                )
                .map_err(BootIdentityRecoveryError::Recovery)?,
            };
            let attached = start_verified_login(
                lifecycle,
                selection,
                package,
                route,
                owners,
                &runtime.identity,
                submit,
                deadline,
            );
            Ok((runtime, attached))
        }
    }
}

/// Keep the authenticated C3 launch borrowed for as long as PID 1 retains
/// the recovered C1 runtime and the corresponding live C3 receipt.
#[cfg(linux_task24_initial_login_runtime)]
pub(in crate::linux_init) struct RecoveredBootIdentity<'boot, 'package, I, C, A, G, E> {
    boot: &'boot AuthenticatedBootService<'package>,
    data: &'boot BootVolumeInput,
    receipt: C3DurableSlotReceipt,
    identity: C1InitialLoginAuthority<LinuxIdentitySnapshotStore, I, C, A, G, E>,
}

#[cfg(linux_task24_initial_login_runtime)]
impl<I, C, A, G, E> RecoveredBootIdentity<'_, '_, I, C, A, G, E>
where
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    #[cfg(all(linux_task24_owner_co_location, feature = "desktop-login"))]
    pub(in crate::linux_init) fn notes_owner(
        &self,
    ) -> io::Result<crate::profile_volume_notes_owner::ProfileVolumeNotesOwner> {
        self.data
            .try_clone()
            .map(crate::profile_volume_notes_owner::pending_with_boot_volume)
    }

    #[cfg(all(linux_task24_owner_co_location, feature = "desktop-login"))]
    pub(in crate::linux_init) fn request_notes_attachment(
        &self,
        lease: super::super::pid1_live_peer_supplier::SessionLease,
        owner: &mut crate::profile_volume_notes_owner::ProfileVolumeNotesOwner,
    ) -> Result<
        (
            crate::profile_volume_notes_owner::AuthorizedNotesAttachment,
            nagi_identity_session::ResolvedIdentity,
            std::sync::Arc<nagi_identity_session::C1IssuedInteractiveSession>,
        ),
        crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError,
    > {
        self.identity.request_notes_attachment(lease, owner)
    }

    /// Enter C1 issuer activity before C3's exact live gate. The issuer
    /// mutex has been released before the effect, while the C1 guard stays
    /// held through the C3 callback and durable receipt validation.
    pub(super) fn with_live_effect<R>(
        &mut self,
        effect: impl FnOnce() -> io::Result<R>,
    ) -> Result<R, BootIdentityRecoveryError> {
        let admission = self
            .identity
            .enter_runtime_admission(self.boot.recovery_scope())
            .map_err(BootIdentityRecoveryError::Recovery)?;
        let scope = admission.scope();
        let claim = correlated_c3_claim(self.boot, scope, admission.operation_identity())
            .map_err(BootIdentityRecoveryError::LiveEffect)?;
        let (result, receipt) = self
            .boot
            .owner
            .c3
            .with_durable_live_continuation_gate(scope, &self.boot.proof, claim, effect)
            .map_err(BootIdentityRecoveryError::LiveEffect)?;
        require_correlated_live_receipt(scope, claim, &receipt)
            .map_err(BootIdentityRecoveryError::LiveEffect)?;
        self.receipt = receipt;
        drop(admission);
        Ok(result)
    }
}

/// Recover the issuer and enter the first-login path while PID 1 still owns
/// the authenticated boot service. The Task24 launcher is supplied by the
/// existing owner and is consumed by the first-login adapter.
#[cfg(linux_task24_initial_login_runtime)]
pub(super) fn recover_and_start_initial_login<'boot, 'package, I, C, A, G, E, Login>(
    boot: &'boot AuthenticatedBootService<'package>,
    catalog: &VerifiedRecoveryBootCatalog,
    data: &'boot BootVolumeInput,
    snapshot_path: &Path,
    account_path: &Path,
    create_identity: impl FnOnce(
        LinuxIdentitySnapshotStore,
    ) -> IdentityService<LinuxIdentitySnapshotStore, I, C, A, G, E>,
    request_key: IdempotencyKey,
    expires_at: Option<UnixMillis>,
    login: &mut Login,
) -> Result<
    (
        RecoveredBootIdentity<'boot, 'package, I, C, A, G, E>,
        Result<Login::Attached, Login::Error>,
    ),
    BootIdentityRecoveryError,
>
where
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
    Login:
        LinuxInitialLoginOwner<C1InitialLoginAuthority<LinuxIdentitySnapshotStore, I, C, A, G, E>>,
{
    let caller =
        seal_boot_service_principal(boot, catalog).map_err(BootIdentityRecoveryError::Principal)?;
    let (store, verifier) = open_boot_identity(data, snapshot_path, account_path)
        .map_err(BootIdentityRecoveryError::Data)?;
    let mut identity = create_identity(store);
    let (handoff, receipt) = recover_boot_identity(boot, &caller, &mut identity)?;
    let runtime = RecoveredBootIdentity {
        boot,
        data,
        receipt,
        identity: C1InitialLoginAuthority::new(
            identity,
            handoff,
            verifier,
            request_key,
            expires_at,
        )
        .map_err(BootIdentityRecoveryError::Recovery)?,
    };
    let attached = login.start_and_accept(&runtime.identity);
    Ok((runtime, attached))
}

struct BootRecoveryGate<'boot, 'package> {
    boot: &'boot AuthenticatedBootService<'package>,
    receipt: Option<C3DurableSlotReceipt>,
    used: bool,
}

impl C1SystemServiceRecoveryEffectGate for BootRecoveryGate<'_, '_> {
    fn apply_recovery_effect<R>(
        &mut self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        operation: C1RecoveryOperationIdentity,
        effect: impl FnOnce() -> Result<R, IdentityError>,
    ) -> Result<Result<R, IdentityError>, RecoveryEffectGateError> {
        let admission = self.boot.recovery_scope();
        if self.used
            || !std::ptr::eq(scope, admission)
            || operation.binding_digest() != admission.binding_digest()
        {
            return Err(RecoveryEffectGateError::Unavailable);
        }
        self.used = true;
        let claim = correlated_c3_claim(self.boot, scope, operation)
            .map_err(|_| RecoveryEffectGateError::Unavailable)?;
        let (result, receipt) = self
            .boot
            .owner
            .c3
            .with_durable_live_effect_gate(scope, &self.boot.proof, claim, effect)
            .map_err(|_| RecoveryEffectGateError::Unavailable)?;
        require_correlated_live_receipt(scope, claim, &receipt)
            .map_err(|_| RecoveryEffectGateError::Unavailable)?;
        self.receipt = Some(receipt);
        Ok(result)
    }
}

fn correlated_c3_claim(
    boot: &AuthenticatedBootService<'_>,
    scope: &dyn SystemServiceRecoveryAdmissionScope,
    operation: C1RecoveryOperationIdentity,
) -> io::Result<C1RecoveryOperationCorrelation> {
    if !std::ptr::eq(scope, boot.recovery_scope())
        || operation.binding_digest() != scope.binding_digest()
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "C1 operation does not match the boot admission",
        ));
    }
    Ok(C1RecoveryOperationCorrelation::new(
        operation.operation_id(),
        *operation.binding_digest(),
    ))
}

fn require_correlated_live_receipt(
    scope: &dyn SystemServiceRecoveryAdmissionScope,
    claim: C1RecoveryOperationCorrelation,
    receipt: &C3DurableSlotReceipt,
) -> io::Result<()> {
    if receipt.state() != C3SlotState::Live
        || receipt.c1_operation_id() != Some(claim.operation_id())
        || receipt.service_id() != scope.service_id()
        || receipt.catalog_generation() != scope.catalog_generation()
        || receipt.c3_launch_generation() != scope.c3_launch_generation()
        || receipt.scope_binding_digest() != claim.binding_digest()
    {
        return Err(io::Error::other(
            "C3 live receipt does not match the C1 operation",
        ));
    }
    Ok(())
}

/// Start recovery after principal issuance releases its gate and retain both
/// owners' evidence for the same authenticated boot launch.
fn recover_boot_identity<I, C, A, G, E>(
    boot: &AuthenticatedBootService<'_>,
    caller: &C1VerifiedBootSystemServicePrincipal,
    identity: &mut IdentityService<LinuxIdentitySnapshotStore, I, C, A, G, E>,
) -> Result<(C1SystemServiceRuntimeHandoff, C3DurableSlotReceipt), BootIdentityRecoveryError>
where
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    let scope = boot.recovery_scope();
    let mut gate = BootRecoveryGate {
        boot,
        receipt: None,
        used: false,
    };
    let handoff =
        C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(identity, caller)
            .recover_with_new_intent_into_runtime(scope, &mut gate)
            .map_err(BootIdentityRecoveryError::Recovery)?;
    let receipt = gate
        .receipt
        .filter(|receipt| {
            let operation = handoff.operation_identity();
            receipt.c1_operation_id() == Some(operation.operation_id())
                && receipt.scope_binding_digest() == operation.binding_digest()
        })
        .ok_or(BootIdentityRecoveryError::Recovery(
            IdentityError::StoreUnavailable,
        ))?;
    Ok((handoff, receipt))
}

/// Open the actual C1 snapshot store and retain the verifier on the same
/// owner-selected Data volume. Both relative paths come from the boot owner.
pub(super) fn open_boot_identity(
    data: &BootVolumeInput,
    snapshot_path: &Path,
    account_path: &Path,
) -> io::Result<(LinuxIdentitySnapshotStore, LinuxAccountVerifier)> {
    let (directory, verifier) = data.identity_inputs(account_path, snapshot_path)?;
    let store = LinuxIdentitySnapshotStore::open(directory).map_err(|error| {
        io::Error::other(format!("identity snapshot directory rejected: {error:?}"))
    })?;
    Ok((store, verifier))
}
