use super::super::linux_init_c3_slot_journal::{
    C3DurableNormalExitReceipt, C3DurableSlotReceipt, C3SlotState,
};
#[cfg(linux_task24_initial_login_runtime)]
use super::super::{
    linux_initial_login_c2::C1InitialLoginAuthority,
    linux_initial_login_owner::LinuxInitialLoginOwner,
};
use super::AuthenticatedBootService;
use crate::profile_volume_notes_owner::BootVolumeInput;
use nagi_identity_session::{
    c1_linux_boot_identity_service_from_store, C1LinuxBootIdentityService,
    C1PreviousLaunchTerminalGate, C1RecoveryOperationIdentity, C1SystemServiceRecoveryEffectGate,
    C1SystemServiceRecoveryReceiptAdapter, C1SystemServiceRuntimeHandoff,
    C1VerifiedBootSystemServicePrincipal, Clock, GuestProfileCleaner, IdentityAuthorizer,
    IdentityError, IdentityEventSink, IdentityIdSource, IdentityService,
    LinuxIdentitySnapshotStore, RecoveryEffectGateError,
};
#[cfg(linux_task24_initial_login_runtime)]
use nagi_identity_session::{IdempotencyKey, UnixMillis};
use nagi_linux_account_verifier::LinuxAccountVerifier;
use nagi_system_service_recovery_contract::{
    C1RecoveryOperationCorrelation, C1RecoveryReceiptClient, C3RecoverySlotGate,
    SystemServiceRecoveryAdmissionScope, VerifiedRecoveryBootCatalog,
};
use std::io;
use std::os::fd::AsRawFd;
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
    #[cfg(any(
        linux_task24_initial_login_runtime,
        all(linux_boot_owner_registration, feature = "desktop-login")
    ))]
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
        let (store, mut verifier) = open_boot_identity(data, snapshot_path, account_path)
            .map_err(BootIdentityRecoveryError::Data)?;
        let (mut identity, mut event_reader) =
            c1_linux_boot_identity_service_from_store(store, &principal);
        if let Some(previous) = latest_previous_launch_normal_exit(self)? {
            // C1's terminal write may report OutcomeUnknown after a durable
            // rename. Preserve this exact C3 receipt across one same-Data
            // reopen so C1 can read back and C3 can revalidate before retry.
            match reconcile_previous_launch_with_receipt(self, &principal, &mut identity, &previous)
            {
                Ok(()) => {}
                Err(PreviousLaunchReconcileAttempt::RetryAfterReopen) => {
                    drop(identity);
                    drop(event_reader);
                    let (retry_store, retry_verifier) =
                        open_boot_identity(data, snapshot_path, account_path)
                            .map_err(BootIdentityRecoveryError::Data)?;
                    verifier = retry_verifier;
                    let (retry_identity, retry_event_reader) =
                        c1_linux_boot_identity_service_from_store(retry_store, &principal);
                    identity = retry_identity;
                    event_reader = retry_event_reader;
                    reconcile_previous_launch_with_receipt(
                        self,
                        &principal,
                        &mut identity,
                        &previous,
                    )
                    .map_err(|_| {
                        BootIdentityRecoveryError::Recovery(IdentityError::StoreUnavailable)
                    })?;
                }
                Err(PreviousLaunchReconcileAttempt::Failed) => {
                    return Err(BootIdentityRecoveryError::Recovery(
                        IdentityError::StoreUnavailable,
                    ));
                }
            }
            drop(identity);
            drop(event_reader);
            let (reopened_store, reopened_verifier) =
                open_boot_identity(data, snapshot_path, account_path)
                    .map_err(BootIdentityRecoveryError::Data)?;
            verifier = reopened_verifier;
            let (reopened_identity, reopened_event_reader) =
                c1_linux_boot_identity_service_from_store(reopened_store, &principal);
            identity = reopened_identity;
            event_reader = reopened_event_reader;
        }
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
            #[cfg(any(
                linux_task24_initial_login_runtime,
                all(linux_boot_owner_registration, feature = "desktop-login")
            ))]
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
                        nagi_identity_session::provision_c1_linux_first_owner_binding_with_runtime_admission(
                            identity,
                            principal,
                            &admission,
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
        console
            .run_registration(screen, |console| {
                #[cfg(all(linux_task24_owner_co_location, linux_task24_initial_login_runtime))]
                {
                    crate::linux_init::wait_for_task24_input(
                        |slice| console.next_key_timeout(slice),
                        || boot.poll_task24_required_identity_exit(),
                    )?
                    .ok_or_else(|| io::Error::other("required Identity exited during registration"))
                }
                #[cfg(not(all(
                    linux_task24_owner_co_location,
                    linux_task24_initial_login_runtime
                )))]
                console.next_key()
            })
            .map_err(|error| {
                use super::super::linux_owner_registration_console::RegistrationConsoleError;
                use crate::login_screen::LinuxRegistrationError;
                let stage = match error {
                    RegistrationConsoleError::Io(_) => "console",
                    RegistrationConsoleError::CredentialUnavailable => "derive",
                    RegistrationConsoleError::Save(LinuxRegistrationError::Identity(_)) => {
                        "identity"
                    }
                    RegistrationConsoleError::Save(LinuxRegistrationError::Save(_)) => {
                        "account-store"
                    }
                };
                super::super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_OWNER_REGISTRATION_DENIED stage={stage}"
                ));
                io::Error::other("owner registration failed")
            })
    }
}

#[cfg(all(linux_task24_owner_co_location, linux_task24_initial_login_runtime))]
impl AuthenticatedBootService<'_> {
    fn poll_task24_required_identity_exit(&self) -> io::Result<bool> {
        let result = match self.owner.c3.poll() {
            Ok(None) => self.proof.with_live_authority_effect(|| false),
            Ok(Some(crate::linux_init::LifecycleEvent::ServiceExited {
                service,
                status,
                required: true,
            })) => {
                crate::linux_init::log_pid1_stderr(format_args!(
                    "NAGI_DEV_TASK24_REQUIRED_IDENTITY_EXIT status={status:?} service={service}"
                ));
                Ok(true)
            }
            Ok(Some(_)) => Err(io::Error::other(
                "unexpected C3 event while monitoring Identity",
            )),
            Err(error) => Err(error),
        };
        result.inspect_err(|_| {
            crate::linux_init::log_pid1_stderr(format_args!(
                "NAGI_DEV_TASK24_REQUIRED_IDENTITY_MONITOR_UNAVAILABLE"
            ));
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
    #[cfg(feature = "system-service-readiness")]
    use crate::linux_init::pid1_live_peer_supplier::pidfd_process_instance::{
        LinuxPidfdKernel, PidfdProcessInstanceAuthority,
    };
    use crate::linux_init::{
        linux_owner_registration_console::UnlockInputOutcome,
        linux_task24_endpoint_launch::{
            start_verified_login, BootInitialLoginRoute, BootLoginRun, BootLoginSupervisor,
            EndpointOwners, LoginChildLaunch, LoginCompletion, LoginLifecycle,
            VerifiedLoginCancellation, VerifiedLoginChild,
        },
        pid1_live_peer_supplier::{PeerDenial, Pid1ReadinessAtomicBoundary, SessionLease},
        publish_lifecycle_ready,
    };
    #[cfg(feature = "system-service-readiness")]
    use crate::linux_init::{
        linux_task24_endpoint_launch::{IdentityReadyProtocol, LoginLease},
        pid1_live_peer_supplier::Pid1CompletedServiceHandshakeOwner,
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
        pub(in crate::linux_init) fn poll_task24_required_identity_exit(&self) -> io::Result<bool> {
            self.boot.poll_task24_required_identity_exit()
        }

        /// Read-only Identity status boundary for the already-attached
        /// Session. The operation must perform bounded local work and must
        /// not do transport I/O or re-enter these owners.
        fn with_current_identity_status<T>(
            &self,
            attached: SessionLease,
            read: impl FnOnce() -> Result<T, PeerDenial>,
        ) -> Result<T, PeerDenial> {
            self.with_current_identity_session(attached, || {
                self.boot
                    .proof
                    .with_live_authority_effect(read)
                    .map_err(|_| PeerDenial::StalePeer)?
            })
        }

        fn with_current_identity_session<T>(
            &self,
            attached: SessionLease,
            read: impl FnOnce() -> Result<T, PeerDenial>,
        ) -> Result<T, PeerDenial> {
            let _runtime = self
                .identity
                .enter_runtime_admission(self.boot.recovery_scope())
                .map_err(|_| PeerDenial::Unavailable)?;
            self.identity.with_ready_session(attached, read)
        }

        /// Answer the separately launched Identity process on the control
        /// endpoint retained by its exact C3 proof. I/O stays outside the
        /// C1/C3 effect gates and uses the handshake's absolute deadline.
        #[cfg(feature = "system-service-readiness")]
        pub(in crate::linux_init) fn confirm_task24_identity_readiness(
            &self,
            attached: SessionLease,
            plan: &IdentityReadyProtocol,
            deadline: Instant,
        ) -> io::Result<()> {
            let control = self
                .boot
                .proof
                .endpoint()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotConnected))?
                .try_clone_to_owned()?;
            let control = std::os::unix::net::UnixStream::from(control);
            crate::linux_init::linux_task24_identity_ready::confirm_identity_readiness(
                &control,
                deadline,
                |challenge| {
                    self.with_current_identity_status(attached, || {
                        plan.confirm_control_challenge(challenge)
                    })
                    .map_err(|_| io::Error::from(io::ErrorKind::PermissionDenied))
                },
            )
        }

        /// Complete the C2 request alongside the C3 control exchange, then
        /// publish its retained evidence inside the current C1 Session gate.
        #[cfg(feature = "system-service-readiness")]
        pub(in crate::linux_init) fn complete_and_publish_task24_ready(
            &self,
            attached: SessionLease,
            run: &BootLoginRun,
            launch_lease: &LoginLease,
            handshakes: &Pid1CompletedServiceHandshakeOwner<
                '_,
                PidfdProcessInstanceAuthority<LinuxPidfdKernel>,
            >,
            deadline: Instant,
            ready: &mut bool,
        ) -> Result<(), PeerDenial> {
            let plan = run.identity_ready.as_ref().ok_or(PeerDenial::Unavailable)?;
            let proof = &self.boot.proof;
            let instance = run.run_owner.run();
            let boundary = run.run_owner.readiness_boundary();
            let event = std::thread::scope(|scope| {
                let worker = std::thread::Builder::new()
                    .spawn_scoped(scope, || {
                        launch_lease.complete_system_service_handshake_before_ready(
                            instance,
                            proof,
                            &run.directory,
                            handshakes,
                            &BootLoginSupervisor,
                            boundary,
                            plan,
                            deadline,
                        )
                    })
                    .map_err(|_| PeerDenial::Unavailable)?;
                let control = self.confirm_task24_identity_readiness(attached, plan, deadline);
                let event = worker
                    .join()
                    .map_err(|_| PeerDenial::RequestOutcomeUnknown)??;
                control.map_err(|_| PeerDenial::HandshakeIncomplete)?;
                Ok(event)
            })?;
            crate::linux_init::log_pid1_stderr(format_args!(
                "NAGI_DEV_TASK24_IDENTITY_HANDSHAKE_COMPLETED"
            ));
            self.with_current_identity_session(attached, || {
                event.with_current(|_| {
                    publish_lifecycle_ready(ready).map_err(|_| PeerDenial::Unavailable)
                })?
            })
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
            console: &mut crate::linux_init::linux_owner_registration_console::BootOwnerConsole,
            unlock: &mut crate::login_screen::LinuxOwnerUnlock<'_>,
            input: crate::linux_init::linux_owner_registration_console::task24_input::RequestRoute,
        ) -> Result<
            (
                Runtime<'boot, 'package>,
                Result<LoginCompletion, PeerDenial>,
            ),
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
                principal: self.principal,
                identity: C1InitialLoginAuthority::new(
                    self.identity,
                    self.handoff,
                    self.boot.recovery_admission(),
                    self.verifier,
                    request_key,
                    expires_at,
                )
                .map_err(BootIdentityRecoveryError::Recovery)?,
            };
            let failures = runtime
                .identity
                .login_failures()
                .map_err(BootIdentityRecoveryError::LiveEffect)?;
            unlock.resume_throttle(failures);
            let attached = loop {
                let input_outcome = console
                    .read_unlock(unlock, || runtime.poll_task24_required_identity_exit())
                    .map_err(BootIdentityRecoveryError::LiveEffect)?;
                if runtime
                    .poll_task24_required_identity_exit()
                    .map_err(BootIdentityRecoveryError::LiveEffect)?
                {
                    return Err(BootIdentityRecoveryError::LiveEffect(io::Error::other(
                        "required Identity exited before processing unlock input",
                    )));
                }
                let submission = match input_outcome {
                    UnlockInputOutcome::Submitted(submission) => submission,
                    UnlockInputOutcome::Closed => {
                        let closed = VerifiedLoginCancellation::from_input_close(
                            lifecycle,
                            owners,
                            &runtime.identity,
                        )
                        .map(LoginCompletion::Cancelled);
                        return Ok((runtime, closed));
                    }
                };
                let deadline = Instant::now() + std::time::Duration::from_secs(300);
                let result = start_verified_login(
                    lifecycle,
                    &selection,
                    package,
                    &route,
                    owners,
                    &runtime.identity,
                    |channel| submission.submit(channel, input, deadline),
                    deadline,
                );
                match result {
                    Err(PeerDenial::AccountRejected) => {
                        let failures = runtime
                            .identity
                            .login_failures()
                            .map_err(BootIdentityRecoveryError::LiveEffect)?;
                        unlock.rejected(failures);
                        console
                            .present_unlock(unlock)
                            .map_err(BootIdentityRecoveryError::LiveEffect)?;
                    }
                    Err(PeerDenial::RequestDeadlineExpired) => {
                        if let Err(error) = runtime.identity.require_unattached() {
                            break Err(error);
                        }
                        crate::linux_init::log_pid1_stderr(format_args!(
                            "NAGI_DEV_TASK24_LOGIN_ATTEMPT_EXPIRED"
                        ));
                    }
                    result => break result,
                }
            };
            let attached = attached.map(|session| {
                if runtime.identity.reset_login_failures().is_ok() {
                    unlock.succeeded();
                    LoginCompletion::Attached(session)
                } else {
                    LoginCompletion::ThrottleResetFailed(session)
                }
            });
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
    principal: C1VerifiedBootSystemServicePrincipal,
    identity: C1InitialLoginAuthority<'boot, LinuxIdentitySnapshotStore, I, C, A, G, E>,
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
    pub(in crate::linux_init) fn reconcile_after_normal_exit(
        &self,
        package_owner: &nagi_init_package_provider::package_trust::PackageServiceSessionOwner<'_>,
    ) -> io::Result<C3DurableNormalExitReceipt> {
        use nagi_system_service_recovery_contract::{
            validate_recovery_receipts, C1RecoveryReceiptClient,
        };

        if package_owner.is_quarantined() {
            return Err(io::Error::other("boot package owner is quarantined"));
        }
        let (expected_exit, deadline) = self.request_and_reap_normal_identity_exit()?;
        self.identity.with_identity_for_terminal(|identity| {
            let scope = self.boot.recovery_scope();
            let mut client = C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                identity,
                &self.principal,
            );
            let mut gate = BootTerminalGate {
                boot: self.boot,
                expected: expected_exit,
                deadline,
                receipt: None,
                used: false,
            };
            let terminal = client
                .reconcile_terminal(scope, &mut gate)
                .map_err(|error| {
                    io::Error::other(format!("C1/C3 terminal reconciliation: {error:?}"))
                })?;
            let slot = gate
                .receipt
                .ok_or_else(|| io::Error::other("C3 terminal receipt absent"))?;
            if package_owner.is_quarantined() {
                return Err(io::Error::other("boot package owner is quarantined"));
            }
            validate_recovery_receipts(scope, &terminal, &slot).map_err(|error| {
                io::Error::other(format!("C1/C3 terminal receipt mismatch: {error:?}"))
            })?;
            client
                .consume_terminal_receipt(scope, terminal)
                .map_err(|error| io::Error::other(format!("C1 terminal readback: {error:?}")))?;
            Ok(slot)
        })
    }

    fn request_and_reap_normal_identity_exit(
        &self,
    ) -> io::Result<(C3DurableNormalExitReceipt, std::time::Instant)> {
        let scope = self.boot.recovery_scope();
        let live = &self.receipt;
        if live.state() != C3SlotState::Live
            || live.service_id() != scope.service_id()
            || live.catalog_generation() != scope.catalog_generation()
            || live.c3_launch_generation() != scope.c3_launch_generation()
            || live.scope_binding_digest() != scope.binding_digest()
            || live.c1_operation_id().is_none()
        {
            return Err(io::Error::other(
                "C3 live receipt does not match the current Identity scope",
            ));
        }

        let registered = self.boot.owner.c3.registered_process_identity(scope)?;
        let process = self.boot.proof.process_identity();
        let slot = self.boot.proof.service_slot_identity();
        if !process.same_instance(&registered)
            || process.process_id() != self.boot.proof.process_id()
            || self.boot.proof.launch_generation() != scope.c3_launch_generation()
            || slot.service_id() != scope.service_id()
            || slot.catalog_generation() != scope.catalog_generation()
            || !self.boot.proof.is_live()?
        {
            return Err(io::Error::other(
                "C3 Identity launch is not live for the exact boot scope",
            ));
        }

        let control_fd = self
            .boot
            .proof
            .endpoint()
            .ok_or_else(|| io::Error::other("C3 Identity control endpoint is unavailable"))?
            .as_raw_fd();
        let deadline = std::time::Instant::now()
            .checked_add(super::RETIREMENT_BUDGET)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "C3 deadline overflow"))?;
        self.boot.proof.with_live_authority_effect(|| {
            // The signed Task24 Identity service treats EOF on its inherited
            // C3 control endpoint as its planned, successful shutdown path.
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "C3 Identity exit deadline expired before shutdown request",
                ));
            }
            let result = unsafe { libc::shutdown(control_fd, libc::SHUT_WR) };
            if result == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        })??;
        crate::linux_init::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_IDENTITY_EXIT_REQUESTED"));

        // C3 remains the sole owner of the process reaper and journal. Its
        // bounded wait returns only after the exact current child is reaped
        // and any terminal journal transition has been read back.
        self.boot
            .owner
            .c3
            .reap_until_quiescent_until(deadline, super::POLL_INTERVAL)?;
        let terminal = self
            .boot
            .owner
            .c3
            .latest_durable_normal_exit_scope_snapshot_until(deadline)?
            .ok_or_else(|| io::Error::other("C3 normal Identity exit proof is unavailable"))?;
        let expected_scope =
            crate::c3_system_service_recovery::RecoveryScopeSnapshot::capture(scope);
        if terminal.scope_snapshot() != &expected_scope
            || terminal.service_id() != scope.service_id()
            || terminal.catalog_generation() != scope.catalog_generation()
            || terminal.c3_launch_generation() != scope.c3_launch_generation()
            || terminal.scope_binding_digest() != scope.binding_digest()
            || terminal.process_id().get() != u64::from(process.process_id())
            || Some(terminal.c1_operation_id()) != live.c1_operation_id()
        {
            return Err(io::Error::other(
                "C3 normal Identity exit proof does not match the exact live launch",
            ));
        }
        ensure_normal_exit_deadline(deadline)?;
        log_task24_c3_normal_exit_receipt(
            "NAGI_DEV_TASK24_C3_IDENTITY_NORMAL_EXIT_REAPED",
            &terminal,
        );
        Ok((terminal, deadline))
    }

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
    mut create_identity: impl FnMut(
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
    Login: LinuxInitialLoginOwner<
        C1InitialLoginAuthority<'boot, LinuxIdentitySnapshotStore, I, C, A, G, E>,
    >,
{
    let caller =
        seal_boot_service_principal(boot, catalog).map_err(BootIdentityRecoveryError::Principal)?;
    let (store, mut verifier) = open_boot_identity(data, snapshot_path, account_path)
        .map_err(BootIdentityRecoveryError::Data)?;
    let mut identity = create_identity(store);
    if let Some(previous) = latest_previous_launch_normal_exit(boot)? {
        match reconcile_previous_launch_with_receipt(boot, &caller, &mut identity, &previous) {
            Ok(()) => {}
            Err(PreviousLaunchReconcileAttempt::RetryAfterReopen) => {
                drop(identity);
                let (retry_store, retry_verifier) =
                    open_boot_identity(data, snapshot_path, account_path)
                        .map_err(BootIdentityRecoveryError::Data)?;
                verifier = retry_verifier;
                identity = create_identity(retry_store);
                reconcile_previous_launch_with_receipt(boot, &caller, &mut identity, &previous)
                    .map_err(|_| {
                        BootIdentityRecoveryError::Recovery(IdentityError::StoreUnavailable)
                    })?;
            }
            Err(PreviousLaunchReconcileAttempt::Failed) => {
                return Err(BootIdentityRecoveryError::Recovery(
                    IdentityError::StoreUnavailable,
                ));
            }
        }
        drop(identity);
        let (reopened_store, reopened_verifier) =
            open_boot_identity(data, snapshot_path, account_path)
                .map_err(BootIdentityRecoveryError::Data)?;
        verifier = reopened_verifier;
        identity = create_identity(reopened_store);
    }
    let (handoff, receipt) = recover_boot_identity(boot, &caller, &mut identity)?;
    let runtime = RecoveredBootIdentity {
        boot,
        data,
        receipt,
        principal: caller,
        identity: C1InitialLoginAuthority::new(
            identity,
            handoff,
            boot.recovery_admission(),
            verifier,
            request_key,
            expires_at,
        )
        .map_err(BootIdentityRecoveryError::Recovery)?,
    };
    let attached = login.start_and_accept(&runtime.identity);
    Ok((runtime, attached))
}

#[cfg(linux_task24_initial_login_runtime)]
struct BootTerminalGate<'boot, 'package> {
    boot: &'boot AuthenticatedBootService<'package>,
    expected: C3DurableNormalExitReceipt,
    deadline: std::time::Instant,
    receipt: Option<C3DurableNormalExitReceipt>,
    used: bool,
}

#[cfg(linux_task24_initial_login_runtime)]
impl nagi_system_service_recovery_contract::C3RecoverySlotGate for BootTerminalGate<'_, '_> {
    fn with_reconciliation_gate<R>(
        &mut self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        claim: C1RecoveryOperationCorrelation,
        reconcile: impl FnOnce() -> R,
    ) -> Result<R, nagi_system_service_recovery_contract::RecoveryClientError> {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let retained = self.boot.recovery_scope();
        if self.used
            || !std::ptr::addr_eq(scope, retained)
            || claim.binding_digest() != retained.binding_digest()
        {
            return Err(RecoveryClientError::Rejected);
        }
        self.used = true;
        // Normal child retirement has already committed an exact C3
        // normal-exit row carrying this C1 correlation. Reuse that proof
        // under the C3 owner gate instead of appending a generic
        // ClearedAndAbsent row, which would hide the normal-exit evidence
        // needed by the next cold boot.
        let mut normal_exit_gate = self
            .boot
            .owner
            .c3
            .durable_normal_exit_gate_until(&self.expected, self.deadline);
        let result = normal_exit_gate.with_reconciliation_gate(retained, claim, reconcile)?;
        self.receipt = Some(self.expected.clone());
        Ok(result)
    }
}

struct BootPreviousLaunchTerminalGate<'boot, 'receipt, 'package> {
    boot: &'boot AuthenticatedBootService<'package>,
    expected: &'receipt C3DurableNormalExitReceipt,
    used: bool,
}

impl C1PreviousLaunchTerminalGate for BootPreviousLaunchTerminalGate<'_, '_, '_> {
    type TerminalReceipt = C3DurableNormalExitReceipt;

    fn with_previous_launch_reconciliation_gate(
        &mut self,
        current_scope: &dyn SystemServiceRecoveryAdmissionScope,
        previous_scope: &dyn SystemServiceRecoveryAdmissionScope,
        claim: C1RecoveryOperationCorrelation,
        revalidate_scopes: impl FnOnce() -> Result<
            (),
            nagi_system_service_recovery_contract::RecoveryClientError,
        >,
    ) -> Result<Self::TerminalReceipt, nagi_system_service_recovery_contract::RecoveryClientError>
    {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        if self.used
            || !std::ptr::addr_eq(current_scope, self.boot.recovery_scope())
            || claim.binding_digest() != self.expected.scope_binding_digest()
        {
            return Err(RecoveryClientError::Rejected);
        }
        self.used = true;
        let mut gate = self.boot.owner.c3.durable_normal_exit_gate(self.expected);
        let revalidated =
            gate.with_reconciliation_gate(previous_scope, claim, revalidate_scopes)?;
        revalidated?;
        Ok(self.expected.clone())
    }
}

fn latest_previous_launch_normal_exit(
    boot: &AuthenticatedBootService<'_>,
) -> Result<Option<C3DurableNormalExitReceipt>, BootIdentityRecoveryError> {
    let current_scope = boot.recovery_scope();
    let current_generation = current_scope.c3_launch_generation();
    if boot.lifecycle.slot.c3_launch_generation() != current_scope.c3_launch_generation() {
        return Err(BootIdentityRecoveryError::LiveEffect(io::Error::new(
            io::ErrorKind::InvalidData,
            "C3 current scope differs from its reserved boot generation",
        )));
    }

    let previous = boot.lifecycle.previous_normal_exit.as_ref();
    match previous {
        Some(receipt)
            if receipt.service_id() == current_scope.service_id()
                && receipt.catalog_generation() == current_scope.catalog_generation()
                && receipt.c3_launch_generation().get().checked_add(1)
                    == Some(current_generation.get()) => {}
        None if current_generation.get() == 1 => {}
        _ => {
            return Err(BootIdentityRecoveryError::LiveEffect(io::Error::new(
                io::ErrorKind::InvalidData,
                "pre-reservation C3 evidence is not bound to this boot generation",
            )));
        }
    }

    Ok(previous.cloned())
}

fn ensure_normal_exit_deadline(deadline: std::time::Instant) -> io::Result<()> {
    if std::time::Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "C3 normal Identity exit deadline expired",
        ));
    }
    Ok(())
}

pub(in crate::linux_init) fn log_task24_c3_normal_exit_receipt(
    marker: &str,
    receipt: &C3DurableNormalExitReceipt,
) {
    crate::linux_init::log_pid1_stderr(format_args!(
        "{marker} service={} catalog_generation={} c3_launch_generation={} launch_instance_id={} process_id={} c1_operation_id={} terminal_revision={} terminal_operation_id={}",
        receipt.service_id(),
        receipt.catalog_generation().get(),
        receipt.c3_launch_generation().get(),
        receipt.launch_instance_id().get(),
        receipt.process_id().get(),
        receipt.c1_operation_id().get(),
        receipt.terminal_revision().get(),
        receipt.terminal_operation_id().get(),
    ));
}

enum PreviousLaunchReconcileAttempt {
    RetryAfterReopen,
    Failed,
}

fn reconcile_previous_launch_with_receipt<I, C, A, G, E>(
    boot: &AuthenticatedBootService<'_>,
    caller: &C1VerifiedBootSystemServicePrincipal,
    identity: &mut IdentityService<LinuxIdentitySnapshotStore, I, C, A, G, E>,
    previous: &C3DurableNormalExitReceipt,
) -> Result<(), PreviousLaunchReconcileAttempt>
where
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    let current_scope = boot.recovery_scope();
    let previous_scope = previous.scope_snapshot();
    let mut gate = BootPreviousLaunchTerminalGate {
        boot,
        expected: previous,
        used: false,
    };
    let mut adapter =
        C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(identity, caller);
    let terminal = match adapter.reconcile_previous_launch_terminal(
        current_scope,
        previous_scope,
        &mut gate,
    ) {
        Ok(terminal) => terminal,
        Err(nagi_system_service_recovery_contract::RecoveryClientError::OutcomeUnknown) => {
            return Err(PreviousLaunchReconcileAttempt::RetryAfterReopen);
        }
        Err(_) => return Err(PreviousLaunchReconcileAttempt::Failed),
    };
    adapter
        .consume_terminal_receipt(previous_scope, terminal)
        .map_err(|_| PreviousLaunchReconcileAttempt::RetryAfterReopen)?;
    #[cfg(linux_task24_initial_login_runtime)]
    log_task24_c3_normal_exit_receipt(
        "NAGI_DEV_TASK24_PREVIOUS_IDENTITY_TERMINAL_CONSUMED",
        previous,
    );
    Ok(())
}

// This contract fixture intentionally stays separate from BootTerminalGate.
// It tests the receipt adapter contract without claiming a production C3
// journal or rebooted Task24 login result.
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
#[path = "linux_c1_prior_launch_gate_fixture.rs"]
mod prior_launch_gate_fixture;

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
