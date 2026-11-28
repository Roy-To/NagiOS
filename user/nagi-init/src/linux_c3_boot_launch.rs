//! Start the signed Identity package through the maintained C3 owner.

#[cfg(linux_boot_identity_runtime)]
use super::linux_init_c3_slot_journal::{C3DurableNormalExitReceipt, PriorBootClear};
use super::linux_task24_c3_data_journal::Task24C3DataJournalDirectory;
use super::{
    linux_boot_package_inputs::LinuxBootArtifactPreflight,
    linux_init_c3_slot_journal::C3DurableSlotReceipt,
    linux_init_c3_system_service_owner::LinuxC3SystemServiceSlotOwner,
    linux_initial_login_owner::LinuxInitialLoginOwner,
};
#[cfg(nagi_release_build)]
use crate::profile_volume_notes_owner::BootVolumeInput;
use crate::profile_volume_notes_owner::SelectedDataRoot;
use nagi_init_package_provider::package_trust::{
    InstalledPackageBlobStore, PackageServiceSessionOwner,
};
use nagi_init_spawn_contract::SpawnedProcessIdentity;
use nagi_system_principal_contract::SystemServiceEndpointOwnerProof;
use nagi_system_service_launch::{
    launch_identity_service_from_store, ArtifactPreflightReport, C1SystemServiceRecoveryClient,
    GenerationTransition, LaunchError, LaunchPortError, LifecycleFinish, ReservationError,
    SystemServiceLaunchLifecycle, SystemServiceLaunchMetadata, SystemServiceLaunchOwner,
    SystemServiceRecoveryAdmission, SystemServiceRuntime, SystemServiceRuntimeSlotBinding,
    VerifiedBootCatalog, VerifiedServicePackageBinding,
};
use nagi_system_service_recovery_contract::{
    C1RecoveryOperationCorrelation, LiveBootSystemServiceBinding,
    SystemServiceRecoveryAdmissionScope, VerifiedRecoveryBootCatalog,
};
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::Duration;

#[cfg(feature = "system-service-readiness")]
use super::linux_init_c3_system_service_owner::C3PendingIdentityListener;

#[cfg(nagi_release_build)]
pub(super) const TASK24_IDENTITY_PACKAGE_ID: &str = "org.nagi.identity";
#[cfg(not(nagi_release_build))]
pub(super) const TASK24_IDENTITY_PACKAGE_ID: &str = "org.nagi.dev.task24-identity";

#[cfg(feature = "system-service-readiness")]
pub(super) struct IdentityListenerHandoff<'a> {
    pub(super) arguments: &'a [&'a str],
    pub(super) pending: &'a mut dyn C3PendingIdentityListener,
}

const RELEASE_BUDGET: Duration = Duration::from_secs(2);
const RETIREMENT_BUDGET: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

fn package_store_journal_path(package_store: &File) -> PathBuf {
    let mut journal = PathBuf::from(format!("/proc/self/fd/{}", package_store.as_raw_fd()));
    journal.push("system-service-slot.v1");
    journal
}

#[cfg(linux_boot_identity_runtime)]
#[path = "linux_c1_boot_recovery.rs"]
pub(super) mod linux_c1_boot_recovery;

/// A's normal-boot adapter to the maintained C3 process and journal owner.
pub(super) struct LinuxBootServiceOwner {
    c3: LinuxC3SystemServiceSlotOwner,
    metadata: SystemServiceLaunchMetadata,
    reservation_attempted: bool,
    #[cfg(not(linux_dev_preview_smoke))]
    _task24_data_journal: Option<Task24C3DataJournalDirectory>,
    // Capture the prior boot's proof while the journal still describes that
    // boot. The current reservation may make the latest-row API unavailable.
    #[cfg(linux_boot_identity_runtime)]
    validate_previous_identity_terminal: bool,
    #[cfg(linux_boot_identity_runtime)]
    previous_identity_terminal: Option<PreviousIdentityTerminal>,
}

pub(super) struct LinuxBootServiceLifecycle {
    slot: SystemServiceRuntimeSlotBinding,
    process: Option<SpawnedProcessIdentity>,
    #[cfg(linux_boot_identity_runtime)]
    previous_identity_terminal: Option<PreviousIdentityTerminal>,
}

#[cfg(linux_boot_identity_runtime)]
#[derive(Clone)]
enum PreviousIdentityTerminal {
    NormalExit(C3DurableNormalExitReceipt),
    PriorBootCleared(PriorBootClear),
}

#[cfg(linux_boot_identity_runtime)]
impl PreviousIdentityTerminal {
    fn service_and_catalog(&self) -> (&str, Option<std::num::NonZeroU64>) {
        match self {
            Self::NormalExit(receipt) => (receipt.service_id(), Some(receipt.catalog_generation())),
            Self::PriorBootCleared(clear) => (
                clear.service_id(),
                std::num::NonZeroU64::new(clear.catalog_generation()),
            ),
        }
    }

    fn generation(&self) -> u64 {
        match self {
            Self::NormalExit(receipt) => receipt.c3_launch_generation().get(),
            Self::PriorBootCleared(clear) => clear.cleared_generation(),
        }
    }
}

type RecoveredLaunch<'owner, C3, I, Attached, AttachError> = Result<
    (
        SystemServiceRuntime<'owner, C3, I>,
        Result<Attached, AttachError>,
    ),
    LaunchError,
>;

impl SystemServiceLaunchLifecycle for LinuxBootServiceLifecycle {
    fn runtime_slot_binding(&self) -> &SystemServiceRuntimeSlotBinding {
        &self.slot
    }

    fn process_identity(&self) -> Option<SpawnedProcessIdentity> {
        self.process.clone()
    }
}

impl LinuxBootServiceOwner {
    fn new(
        package_store: &File,
        catalog: &VerifiedBootCatalog,
        task24_data_root: Option<&SelectedDataRoot>,
        verified_identity_receiver: bool,
    ) -> io::Result<Self> {
        let metadata = catalog.descriptor().launch_metadata();
        let selected_data_root =
            identity_c3_data_root(catalog.descriptor().package_id(), task24_data_root)?;
        let task24_data_journal = selected_data_root
            .map(|root| Task24C3DataJournalDirectory::open(root, metadata.service_id().as_str()))
            .transpose()?;
        let journal = task24_data_journal
            .as_ref()
            .map(|directory| directory.journal_path().to_path_buf())
            .unwrap_or_else(|| package_store_journal_path(package_store));
        let c3 = LinuxC3SystemServiceSlotOwner::new_with_journal(
            metadata.service_id().as_str(),
            journal,
        )?;
        #[cfg(linux_boot_identity_runtime)]
        let validate_previous_identity_terminal = should_validate_previous_identity_terminal(
            cfg!(nagi_release_build),
            verified_identity_receiver,
        );
        #[cfg(linux_boot_identity_runtime)]
        let previous_identity_terminal = if validate_previous_identity_terminal {
            let previous = match c3.latest_durable_prior_boot_clear()? {
                Some(clear) => Some(PreviousIdentityTerminal::PriorBootCleared(clear)),
                None => c3
                    .latest_durable_normal_exit_scope_snapshot()?
                    .map(PreviousIdentityTerminal::NormalExit),
            };
            validate_previous_launch_catalog(
                previous
                    .as_ref()
                    .map(PreviousIdentityTerminal::service_and_catalog),
                metadata.service_id().as_str(),
                metadata.generation(),
            )?;
            previous
        } else {
            None
        };
        Ok(Self {
            c3,
            metadata,
            reservation_attempted: false,
            #[cfg(not(linux_dev_preview_smoke))]
            _task24_data_journal: task24_data_journal,
            #[cfg(linux_boot_identity_runtime)]
            validate_previous_identity_terminal,
            #[cfg(linux_boot_identity_runtime)]
            previous_identity_terminal,
        })
    }

    /// Enter the shared runtime path once the configured C1 owner can
    /// recover. The result retains the exact C3 proof, package and lifecycle.
    pub(super) fn launch_recovered<'owner, I, L>(
        &'owner mut self,
        catalog: &mut VerifiedBootCatalog,
        blobs: &dyn InstalledPackageBlobStore,
        package_owner: &'owner PackageServiceSessionOwner<'_>,
        identity: &'owner mut I,
        login: &mut L,
    ) -> RecoveredLaunch<'owner, Self, I, L::Attached, L::Error>
    where
        I: C1SystemServiceRecoveryClient,
        L: LinuxInitialLoginOwner<I::Runtime>,
    {
        let descriptor = catalog.descriptor_json().to_owned();
        let mut preflight =
            LinuxBootArtifactPreflight::new().map_err(LaunchError::ArtifactPreflight)?;
        let runtime = launch_identity_service_from_store(
            &descriptor,
            blobs,
            package_owner,
            catalog,
            &mut preflight,
            self,
            identity,
        )?;
        let attached = login.start_and_accept(runtime.identity_runtime());
        Ok((runtime, attached))
    }
}

fn identity_c3_data_root<'a, Root>(
    package_id: &str,
    selected_data_root: Option<&'a Root>,
) -> io::Result<Option<&'a Root>> {
    validate_identity_c3_data_root(package_id, selected_data_root.is_some())?;
    if package_id == TASK24_IDENTITY_PACKAGE_ID {
        Ok(selected_data_root)
    } else {
        Ok(None)
    }
}

#[cfg(linux_boot_identity_runtime)]
fn should_validate_previous_identity_terminal(
    release_build: bool,
    verified_identity_receiver: bool,
) -> bool {
    // Developer Preview retains its existing C1/C3 validation behavior.
    // In release, only the Mount-verified Identity receiver runs the extra
    // C1 previous-terminal checks; generic release services leave them to C3.
    !release_build || verified_identity_receiver
}

fn validate_identity_c3_data_root(
    package_id: &str,
    has_selected_data_root: bool,
) -> io::Result<()> {
    if package_id == TASK24_IDENTITY_PACKAGE_ID {
        if !has_selected_data_root {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Task24 C3 journal requires its selected Data root",
            ))
        } else {
            Ok(())
        }
    } else if has_selected_data_root {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Task24 selected Data root cannot be reused by another service",
        ))
    } else {
        Ok(())
    }
}

fn validate_generic_boot_service_receiver(package_id: &str, release_build: bool) -> io::Result<()> {
    if release_build && package_id == TASK24_IDENTITY_PACKAGE_ID {
        Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "release Identity must use its Mount-verified boot receiver",
        ))
    } else {
        Ok(())
    }
}

impl LinuxBootServiceOwner {
    fn launch_boot_payload(
        &mut self,
        admission: &SystemServiceRecoveryAdmission,
        package: &VerifiedServicePackageBinding<'_>,
        lifecycle: &mut LinuxBootServiceLifecycle,
        #[cfg(feature = "system-service-readiness")] identity_listener: Option<
            IdentityListenerHandoff<'_>,
        >,
    ) -> Result<SystemServiceEndpointOwnerProof, LaunchPortError> {
        if admission.runtime_slot_binding() != &lifecycle.slot || lifecycle.process.is_some() {
            return Err(LaunchPortError::Rejected);
        }
        // SAFETY: the immutable executable is bound to this admission.
        // The Identity adapter is the C2 owner of the matching pending listener.
        let proof = unsafe {
            #[cfg(feature = "system-service-readiness")]
            let result = if let Some(listener) = identity_listener {
                self.c3
                    .launch_and_authenticate_admitted_with_identity_listener(
                        admission.scope(),
                        package.executable_bytes(),
                        listener.arguments,
                        true,
                        RELEASE_BUDGET,
                        POLL_INTERVAL,
                        RETIREMENT_BUDGET,
                        POLL_INTERVAL,
                        listener.pending,
                        || false,
                    )
            } else {
                self.launch_control_endpoint(admission, package)
            };
            #[cfg(not(feature = "system-service-readiness"))]
            let result = self.launch_control_endpoint(admission, package);
            result
        }
        .map_err(|error| {
            super::log_pid1_stderr(format_args!(
                "nagi-init: C3 service launch failed ({error})"
            ));
            LaunchPortError::Failed
        })?;
        let process = self
            .c3
            .registered_process_identity(admission.scope())
            .map_err(|_| LaunchPortError::Unavailable)?;
        let slot = proof.service_slot_identity();
        let credentials = proof.peer_credentials();
        if proof.version()
            != nagi_system_principal_contract::SYSTEM_SERVICE_ENDPOINT_OWNER_PROOF_VERSION
            || slot.service_id() != admission.service_id().as_str()
            || slot.catalog_generation() != admission.catalog_generation()
            || proof.launch_generation() != admission.c3_launch_generation()
            || !proof.process_identity().same_instance(&process)
            || (credentials.uid, credentials.gid) != lifecycle.slot.credentials()
            || proof.endpoint().is_none()
            || !proof.is_live().map_err(|_| LaunchPortError::Unavailable)?
        {
            return Err(LaunchPortError::Rejected);
        }
        lifecycle.process = Some(process);
        Ok(proof)
    }

    unsafe fn launch_control_endpoint(
        &self,
        admission: &SystemServiceRecoveryAdmission,
        package: &VerifiedServicePackageBinding<'_>,
    ) -> io::Result<SystemServiceEndpointOwnerProof> {
        // SAFETY: launch_boot_payload checks the matching admission and package.
        unsafe {
            self.c3.launch_and_authenticate_admitted(
                admission.scope(),
                package.executable_bytes(),
                &[],
                true,
                RELEASE_BUDGET,
                POLL_INTERVAL,
                RETIREMENT_BUDGET,
                POLL_INTERVAL,
                || false,
            )
        }
    }
}

impl SystemServiceLaunchOwner for LinuxBootServiceOwner {
    type Lifecycle = LinuxBootServiceLifecycle;
    type RecoveryReceipt = C3DurableSlotReceipt;

    fn reserve_launch(
        &mut self,
        metadata: &SystemServiceLaunchMetadata,
        package: &VerifiedServicePackageBinding<'_>,
    ) -> Result<Self::Lifecycle, ReservationError> {
        // This boot owner opens a durably absent slot and makes one launch.
        if metadata != &self.metadata
            || metadata.transition() != GenerationTransition::Initial
            || self.reservation_attempted
        {
            return Err(ReservationError::Rejected);
        }
        self.reservation_attempted = true;
        let generation = self.c3.reserve_launch_generation().map_err(|error| {
            if error.kind() == io::ErrorKind::PermissionDenied {
                ReservationError::Rejected
            } else {
                ReservationError::Unavailable
            }
        })?;
        #[cfg(linux_boot_identity_runtime)]
        let previous_identity_terminal = {
            let previous = self.previous_identity_terminal.take();
            validate_previous_launch_generation_for_policy(
                previous.as_ref(),
                generation,
                self.validate_previous_identity_terminal,
            )
            .map_err(|_| ReservationError::Unavailable)?;
            previous
        };
        Ok(LinuxBootServiceLifecycle {
            slot: SystemServiceRuntimeSlotBinding::for_c3_reservation(
                metadata, package, generation,
            ),
            process: None,
            #[cfg(linux_boot_identity_runtime)]
            previous_identity_terminal,
        })
    }

    fn launch_and_authenticate(
        &mut self,
        admission: &SystemServiceRecoveryAdmission,
        package: &VerifiedServicePackageBinding<'_>,
        lifecycle: &mut Self::Lifecycle,
    ) -> Result<SystemServiceEndpointOwnerProof, LaunchPortError> {
        self.launch_boot_payload(
            admission,
            package,
            lifecycle,
            #[cfg(feature = "system-service-readiness")]
            None,
        )
    }

    fn finish_exact(
        &mut self,
        admission: &SystemServiceRecoveryAdmission,
        lifecycle: Self::Lifecycle,
        proof: Option<&SystemServiceEndpointOwnerProof>,
    ) -> LifecycleFinish<Self::Lifecycle> {
        if admission.runtime_slot_binding() == &lifecycle.slot
            && self
                .c3
                .finish_admitted(admission.scope(), proof, RETIREMENT_BUDGET, POLL_INTERVAL)
                .is_ok()
        {
            LifecycleFinish::StoppedAndRevoked
        } else {
            LifecycleFinish::Unresolved(lifecycle)
        }
    }

    fn persist_recovery_quarantine(
        &mut self,
        admission: &SystemServiceRecoveryAdmission,
    ) -> Result<(), LaunchPortError> {
        self.c3
            .persist_recovery_quarantine_admitted(admission.scope())
            .map_err(|_| LaunchPortError::OutcomeUnknown)
    }

    fn with_reconciliation_gate<R>(
        &mut self,
        admission: &SystemServiceRecoveryAdmission,
        claim: C1RecoveryOperationCorrelation,
        reconcile: impl FnOnce() -> R,
    ) -> Result<(R, Self::RecoveryReceipt), LaunchPortError> {
        self.c3
            .with_durable_reconciliation_gate(admission.scope(), claim, reconcile)
            .map_err(|_| LaunchPortError::OutcomeUnknown)
    }
}

#[cfg(linux_boot_identity_runtime)]
fn validate_previous_launch_catalog(
    previous: Option<(&str, Option<std::num::NonZeroU64>)>,
    expected_service_id: &str,
    expected_catalog_generation: std::num::NonZeroU64,
) -> io::Result<()> {
    if let Some((service_id, catalog_generation)) = previous {
        if service_id != expected_service_id
            || catalog_generation.is_some_and(|catalog| catalog != expected_catalog_generation)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "C3 previous terminal differs from the boot catalog",
            ));
        }
    }
    Ok(())
}

#[cfg(linux_boot_identity_runtime)]
fn validate_previous_launch_generation(
    previous: Option<&PreviousIdentityTerminal>,
    current_generation: std::num::NonZeroU64,
) -> io::Result<()> {
    validate_previous_launch_generation_for_policy(previous, current_generation, true)
}

#[cfg(linux_boot_identity_runtime)]
fn validate_previous_launch_generation_for_policy(
    previous: Option<&PreviousIdentityTerminal>,
    current_generation: std::num::NonZeroU64,
    validate_previous_terminal: bool,
) -> io::Result<()> {
    // The C3 API returns None only for a fully validated genesis-only
    // journal, whose first launch must use generation one.
    let previous_generation = previous.map(PreviousIdentityTerminal::generation);
    if validate_previous_generation_if_required(
        validate_previous_terminal,
        previous_generation,
        current_generation.get(),
    ) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "C3 previous terminal is not bound to this launch generation",
        ))
    }
}

#[cfg(linux_boot_identity_runtime)]
fn validate_previous_generation_if_required(
    validate_previous_terminal: bool,
    previous_generation: Option<u64>,
    current_generation: u64,
) -> bool {
    !validate_previous_terminal
        || validate_previous_generation_number(previous_generation, current_generation)
}

#[cfg(linux_boot_identity_runtime)]
fn validate_previous_generation_number(previous_generation: Option<u64>, current: u64) -> bool {
    let expected_current = previous_generation
        .map(|generation| generation.checked_add(1))
        .unwrap_or(Some(1));
    expected_current == Some(current)
}

#[cfg(all(
    test,
    target_os = "linux",
    target_arch = "x86_64",
    linux_boot_identity_runtime
))]
mod previous_launch_generation_tests {
    use super::{
        identity_c3_data_root, should_validate_previous_identity_terminal,
        validate_generic_boot_service_receiver, validate_identity_c3_data_root,
        validate_previous_generation_if_required, validate_previous_generation_number,
        validate_previous_launch_catalog, TASK24_IDENTITY_PACKAGE_ID,
    };
    use std::num::NonZeroU64;

    #[test]
    fn genesis_and_normal_exit_bind_to_the_immediate_next_generation() {
        assert!(validate_previous_generation_number(None, 1));
        assert!(!validate_previous_generation_number(None, 2));
        assert!(validate_previous_generation_number(Some(1), 2));
        assert!(!validate_previous_generation_number(Some(1), 3));
        assert!(!validate_previous_generation_number(Some(u64::MAX), 1));
    }

    #[test]
    fn prior_terminal_checks_are_limited_to_dev_and_verified_release_identity() {
        let development_generic = should_validate_previous_identity_terminal(false, false);
        let development_identity = should_validate_previous_identity_terminal(false, true);
        let generic_release = should_validate_previous_identity_terminal(true, false);
        let verified_release_identity = should_validate_previous_identity_terminal(true, true);

        assert!(development_generic);
        assert!(development_identity);
        assert!(!generic_release);
        assert!(verified_release_identity);

        // C3 owns the ClearedAndAbsent generation transition for generic
        // release services. The Identity-only previous-terminal gate must
        // leave that already validated next generation alone.
        assert!(validate_previous_generation_if_required(
            generic_release,
            Some(7),
            8,
        ));
        assert!(validate_previous_generation_if_required(
            development_generic,
            Some(7),
            8,
        ));
        assert!(validate_previous_generation_if_required(
            verified_release_identity,
            Some(7),
            8,
        ));
        assert!(!validate_previous_generation_if_required(
            verified_release_identity,
            Some(7),
            9,
        ));
    }

    #[test]
    fn catalog_mismatch_is_rejected_before_reservation() {
        let catalog_generation = NonZeroU64::new(9).unwrap();
        assert!(validate_previous_launch_catalog(
            None,
            "service:fixture.identity",
            catalog_generation,
        )
        .is_ok());
        // A boot-tagged reservation may precede the first admitted catalog.
        assert!(validate_previous_launch_catalog(
            Some(("service:fixture.identity", None)),
            "service:fixture.identity",
            catalog_generation,
        )
        .is_ok());
        assert!(validate_previous_launch_catalog(
            Some(("service:fixture.identity", Some(catalog_generation))),
            "service:fixture.identity",
            catalog_generation,
        )
        .is_ok());
        assert!(validate_previous_launch_catalog(
            Some(("service:fixture.other", Some(catalog_generation))),
            "service:fixture.identity",
            catalog_generation,
        )
        .is_err());
        assert!(validate_previous_launch_catalog(
            Some(("service:fixture.identity", NonZeroU64::new(8))),
            "service:fixture.identity",
            catalog_generation,
        )
        .is_err());
    }

    #[test]
    fn identity_c3_journal_cannot_fall_back_without_selected_data() {
        assert_eq!(
            validate_identity_c3_data_root(TASK24_IDENTITY_PACKAGE_ID, false)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotFound
        );
        assert!(validate_identity_c3_data_root(TASK24_IDENTITY_PACKAGE_ID, true).is_ok());
        assert!(validate_identity_c3_data_root("org.nagi.other-service", false).is_ok());
        assert_eq!(
            validate_identity_c3_data_root("org.nagi.other-service", true)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn identity_c3_journal_uses_the_exact_selected_root() {
        let selected_root = 42u8;
        let c3_root = identity_c3_data_root(TASK24_IDENTITY_PACKAGE_ID, Some(&selected_root))
            .expect("accept exact root")
            .expect("Identity must retain the selected root");
        assert!(std::ptr::eq(c3_root, &selected_root));
        assert!(identity_c3_data_root("org.nagi.other-service", Some(&selected_root)).is_err());
    }

    #[test]
    fn release_identity_cannot_use_the_generic_receiver() {
        assert_eq!(
            validate_generic_boot_service_receiver(TASK24_IDENTITY_PACKAGE_ID, true)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotConnected
        );
        assert!(validate_generic_boot_service_receiver("org.nagi.other-service", true).is_ok());
        assert!(validate_generic_boot_service_receiver(TASK24_IDENTITY_PACKAGE_ID, false).is_ok());
    }
}

/// Hold the exact process, authenticated endpoint, stable admission scope,
/// and installed generation until the boot coordinator hands them to C1 or
/// retires the launch. Drop order keeps the scope and package lease alive
/// through C3 cleanup.
pub(super) struct AuthenticatedBootService<'package> {
    proof: SystemServiceEndpointOwnerProof,
    owner: LinuxBootServiceOwner,
    lifecycle: LinuxBootServiceLifecycle,
    admission: SystemServiceRecoveryAdmission,
    package: VerifiedServicePackageBinding<'package>,
    #[cfg(all(nagi_release_build, linux_task24_initial_login_runtime))]
    verified_identity_data: Option<BootVolumeInput>,
}

impl AuthenticatedBootService<'_> {
    #[cfg(linux_task24_initial_login_runtime)]
    pub(super) fn recovery_admission(&self) -> &SystemServiceRecoveryAdmission {
        &self.admission
    }

    pub(super) fn recovery_scope(&self) -> &dyn SystemServiceRecoveryAdmissionScope {
        self.admission.scope()
    }

    /// Offer the authenticated mapping to the future C1 seal receiver.
    /// The receiver must leave the gate before running C1's recovery effect.
    pub(super) fn with_verified_system_mapping<R>(
        &self,
        catalog: &VerifiedRecoveryBootCatalog,
        receive: impl for<'mapping> FnOnce(LiveBootSystemServiceBinding<'mapping>) -> R,
    ) -> io::Result<R> {
        let process = self
            .owner
            .c3
            .registered_process_identity(self.admission.scope())?;
        if !self.proof.process_identity().same_instance(&process) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "C3 launch no longer matches the signed boot identity",
            ));
        }
        catalog
            .with_live_binding(&self.proof, |binding| {
                if binding.service_id() != self.admission.service_id().as_str()
                    || binding.principal_id() != self.admission.principal_id().as_str()
                    || binding.caller_context_id() != self.admission.caller_context_id().as_str()
                    || binding.mapping_generation() != self.admission.mapping_generation()
                    || binding.catalog_generation() != self.admission.catalog_generation()
                    || binding.c3_launch_generation() != self.admission.c3_launch_generation()
                {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "signed C1 mapping differs from the exact C3 admission",
                    ));
                }
                Ok(receive(binding))
            })
            .map_err(|error| io::Error::other(format!("live boot mapping rejected: {error:?}")))?
    }

    pub(super) fn retire(mut self) -> io::Result<()> {
        finish_without_c1(
            &mut self.owner,
            &self.admission,
            self.lifecycle,
            Some(&self.proof),
        )
    }
}

fn finish_without_c1(
    owner: &mut LinuxBootServiceOwner,
    admission: &SystemServiceRecoveryAdmission,
    lifecycle: LinuxBootServiceLifecycle,
    proof: Option<&SystemServiceEndpointOwnerProof>,
) -> io::Result<()> {
    match owner.finish_exact(admission, lifecycle, proof) {
        LifecycleFinish::StoppedAndRevoked => Ok(()),
        LifecycleFinish::Quarantined => Err(io::Error::other("exact C3 launch quarantined")),
        LifecycleFinish::Unresolved(_) => {
            owner
                .persist_recovery_quarantine(admission)
                .map_err(|error| {
                    io::Error::other(format!("C3 quarantine unresolved: {error:?}"))
                })?;
            Err(io::Error::other("exact C3 launch quarantined"))
        }
    }
}

/// Run the real C3 handoff through endpoint authentication.
pub(super) fn authenticate_boot_service<'package>(
    package_store: &File,
    catalog: &VerifiedBootCatalog,
    report: &ArtifactPreflightReport,
    package: VerifiedServicePackageBinding<'package>,
    task24_data_root: Option<&SelectedDataRoot>,
    #[cfg(feature = "system-service-readiness")] identity_listener: Option<
        IdentityListenerHandoff<'_>,
    >,
) -> io::Result<AuthenticatedBootService<'package>> {
    validate_generic_boot_service_receiver(
        catalog.descriptor().package_id(),
        cfg!(nagi_release_build),
    )?;
    authenticate_boot_service_inner(
        package_store,
        catalog,
        report,
        package,
        task24_data_root,
        false,
        #[cfg(feature = "system-service-readiness")]
        identity_listener,
    )
}

fn authenticate_boot_service_inner<'package>(
    package_store: &File,
    catalog: &VerifiedBootCatalog,
    report: &ArtifactPreflightReport,
    package: VerifiedServicePackageBinding<'package>,
    task24_data_root: Option<&SelectedDataRoot>,
    verified_identity_receiver: bool,
    #[cfg(feature = "system-service-readiness")] identity_listener: Option<
        IdentityListenerHandoff<'_>,
    >,
) -> io::Result<AuthenticatedBootService<'package>> {
    let mut owner = LinuxBootServiceOwner::new(
        package_store,
        catalog,
        task24_data_root,
        verified_identity_receiver,
    )?;
    super::log_pid1_stderr(format_args!(
        "nagi-init: C3 boot slot journal opened backing={}",
        if catalog.descriptor().package_id() == TASK24_IDENTITY_PACKAGE_ID
            && task24_data_root.is_some()
        {
            "selected-data"
        } else {
            "package-store"
        }
    ));
    #[cfg(feature = "system-service-readiness")]
    let has_identity_listener = identity_listener.is_some();
    #[cfg(not(feature = "system-service-readiness"))]
    let has_identity_listener = false;
    if (catalog.descriptor().package_id() == TASK24_IDENTITY_PACKAGE_ID) != has_identity_listener {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "Task24 Identity requires its C2 pending listener adapter",
        ));
    }

    let mut lifecycle = owner
        .reserve_launch(&catalog.descriptor().launch_metadata(), &package)
        .map_err(|error| io::Error::other(format!("C3 reservation failed: {error:?}")))?;
    let admission = catalog
        .admit_installed_package(report, &package, lifecycle.slot.c3_launch_generation())
        .map_err(|error| io::Error::other(format!("boot admission rejected: {error:?}")))?;
    match owner.launch_boot_payload(
        &admission,
        &package,
        &mut lifecycle,
        #[cfg(feature = "system-service-readiness")]
        identity_listener,
    ) {
        Ok(proof) => Ok(AuthenticatedBootService {
            proof,
            owner,
            lifecycle,
            admission,
            package,
            #[cfg(all(nagi_release_build, linux_task24_initial_login_runtime))]
            verified_identity_data: None,
        }),
        Err(error) => {
            finish_without_c1(&mut owner, &admission, lifecycle, None)?;
            Err(io::Error::other(format!(
                "C3 boot authentication failed: {error:?}"
            )))
        }
    }
}

/// Release receiver for the exact Mount-issued Data root and C3 listener.
/// Retaining the input here binds C1 recovery to the same verified selection.
#[cfg(all(
    nagi_release_build,
    linux_task24_initial_login_runtime,
    feature = "system-service-readiness"
))]
pub(super) fn authenticate_verified_identity_boot_service<'package>(
    package_store: &File,
    catalog: &VerifiedBootCatalog,
    report: &ArtifactPreflightReport,
    package: VerifiedServicePackageBinding<'package>,
    data: &BootVolumeInput,
    identity_listener: IdentityListenerHandoff<'_>,
) -> io::Result<AuthenticatedBootService<'package>> {
    if catalog.descriptor().package_id() != TASK24_IDENTITY_PACKAGE_ID {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "verified Identity receiver requires the signed Identity package",
        ));
    }
    // Both paths are owned by the same opaque release selection as the root
    // FD. Validate their presence here before C3 can launch or admit READY.
    data.verified_identity_paths()?;
    let retained_data = data.try_clone()?;
    let selected_data_root = data.selected_data_root()?;
    let mut authenticated = authenticate_boot_service_inner(
        package_store,
        catalog,
        report,
        package,
        Some(&selected_data_root),
        true,
        Some(identity_listener),
    )?;
    authenticated.verified_identity_data = Some(retained_data);
    Ok(authenticated)
}

#[cfg(test)]
mod identity_package_profile_tests {
    use super::TASK24_IDENTITY_PACKAGE_ID;

    #[test]
    fn c3_identity_package_binding_is_profile_scoped() {
        if cfg!(nagi_release_build) {
            assert_eq!(TASK24_IDENTITY_PACKAGE_ID, "org.nagi.identity");
            assert_ne!(TASK24_IDENTITY_PACKAGE_ID, "org.nagi.dev.task24-identity");
        } else {
            assert_eq!(TASK24_IDENTITY_PACKAGE_ID, "org.nagi.dev.task24-identity");
            assert_ne!(TASK24_IDENTITY_PACKAGE_ID, "org.nagi.identity");
        }
    }
}
