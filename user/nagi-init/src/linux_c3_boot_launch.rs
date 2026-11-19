//! Start the signed Identity package through the maintained C3 owner.

use super::{
    linux_boot_package_inputs::LinuxBootArtifactPreflight,
    linux_init_c3_slot_journal::C3DurableSlotReceipt,
    linux_init_c3_system_service_owner::LinuxC3SystemServiceSlotOwner,
    linux_initial_login_owner::LinuxInitialLoginOwner,
};
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

pub(super) const TASK24_IDENTITY_PACKAGE_ID: &str = "org.nagi.dev.task24-identity";

#[cfg(feature = "system-service-readiness")]
pub(super) struct IdentityListenerHandoff<'a> {
    pub(super) arguments: &'a [&'a str],
    pub(super) pending: &'a mut dyn C3PendingIdentityListener,
}

const RELEASE_BUDGET: Duration = Duration::from_secs(2);
const RETIREMENT_BUDGET: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[cfg(all(linux_boot_identity_runtime, not(nagi_release_build)))]
#[path = "linux_c1_boot_recovery.rs"]
pub(super) mod linux_c1_boot_recovery;

/// A's normal-boot adapter to the maintained C3 process and journal owner.
pub(super) struct LinuxBootServiceOwner {
    c3: LinuxC3SystemServiceSlotOwner,
    metadata: SystemServiceLaunchMetadata,
    reservation_attempted: bool,
}

pub(super) struct LinuxBootServiceLifecycle {
    slot: SystemServiceRuntimeSlotBinding,
    process: Option<SpawnedProcessIdentity>,
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
    pub(super) fn new(package_store: &File, catalog: &VerifiedBootCatalog) -> io::Result<Self> {
        // PID 1 has mounted procfs. The journal belongs to the retained store.
        let mut journal = PathBuf::from(format!("/proc/self/fd/{}", package_store.as_raw_fd()));
        journal.push("system-service-slot.v1");
        let metadata = catalog.descriptor().launch_metadata();
        let c3 = LinuxC3SystemServiceSlotOwner::new_with_journal(
            metadata.service_id().as_str(),
            journal,
        )?;
        Ok(Self {
            c3,
            metadata,
            reservation_attempted: false,
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
        Ok(LinuxBootServiceLifecycle {
            slot: SystemServiceRuntimeSlotBinding::for_c3_reservation(
                metadata, package, generation,
            ),
            process: None,
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
    #[cfg(feature = "system-service-readiness")] identity_listener: Option<
        IdentityListenerHandoff<'_>,
    >,
) -> io::Result<AuthenticatedBootService<'package>> {
    let mut owner = LinuxBootServiceOwner::new(package_store, catalog)?;
    super::log_pid1_stderr(format_args!("nagi-init: C3 boot slot journal opened"));
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
        }),
        Err(error) => {
            finish_without_c1(&mut owner, &admission, lifecycle, None)?;
            Err(io::Error::other(format!(
                "C3 boot authentication failed: {error:?}"
            )))
        }
    }
}
