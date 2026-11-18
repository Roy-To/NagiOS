//! Start the signed Identity package through the maintained C3 owner.

use super::linux_init_c3_system_service_owner::LinuxC3SystemServiceSlotOwner;
use nagi_system_principal_contract::SystemServiceEndpointOwnerProof;
use nagi_system_service_launch::{
    ArtifactPreflightReport, SystemServiceRecoveryAdmission, VerifiedBootCatalog,
    VerifiedServicePackageBinding,
};
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::time::Duration;

const RELEASE_BUDGET: Duration = Duration::from_secs(2);
const RETIREMENT_BUDGET: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Hold the exact process, authenticated endpoint, stable admission scope,
/// and installed generation until the boot coordinator hands them to C1 or
/// retires the launch. Drop order keeps the scope and package lease alive
/// through C3 cleanup.
pub(super) struct AuthenticatedBootService<'package> {
    proof: SystemServiceEndpointOwnerProof,
    owner: LinuxC3SystemServiceSlotOwner,
    admission: SystemServiceRecoveryAdmission,
    package: VerifiedServicePackageBinding<'package>,
}

impl AuthenticatedBootService<'_> {
    pub(super) fn retire(self) -> io::Result<()> {
        drop(self.proof);
        self.owner.shutdown(RETIREMENT_BUDGET, POLL_INTERVAL)
    }
}

/// Run the real C3 handoff through endpoint authentication.
pub(super) fn authenticate_boot_service<'package>(
    package_store: &File,
    catalog: &VerifiedBootCatalog,
    report: &ArtifactPreflightReport,
    package: VerifiedServicePackageBinding<'package>,
) -> io::Result<AuthenticatedBootService<'package>> {
    // procfs is already mounted by PID 1. Anchor the C3-owned journal to the
    // same retained boot directory instead of reopening a caller path.
    let mut journal = PathBuf::from(format!("/proc/self/fd/{}", package_store.as_raw_fd()));
    journal.push("system-service-slot.v1");
    let owner = LinuxC3SystemServiceSlotOwner::new_with_journal(
        catalog.descriptor().service_id().as_str(),
        journal,
    )?;
    let generation = owner.reserve_launch_generation()?;
    let admission = catalog
        .admit_installed_package(report, &package, generation)
        .map_err(|error| io::Error::other(format!("boot admission rejected: {error:?}")))?;
    let auth = (|| {
        // The verified package owns the immutable executable snapshot.
        // The admission outlives spawn, proof validation, and retirement.
        let proof = unsafe {
            owner.launch_and_authenticate_admitted(
                admission.scope(),
                package.executable_bytes(),
                &[],
                true,
                RELEASE_BUDGET,
                POLL_INTERVAL,
                RETIREMENT_BUDGET,
                POLL_INTERVAL,
                || false,
            )?
        };
        let process = owner.registered_process_identity(admission.scope())?;
        let slot = proof.service_slot_identity();
        let credentials = proof.peer_credentials();
        let (uid, gid) = admission.runtime_slot_binding().credentials();
        if proof.version()
            != nagi_system_principal_contract::SYSTEM_SERVICE_ENDPOINT_OWNER_PROOF_VERSION
            || slot.service_id() != admission.service_id().as_str()
            || slot.catalog_generation() != admission.catalog_generation()
            || proof.launch_generation() != admission.c3_launch_generation()
            || !proof.process_identity().same_instance(&process)
            || (credentials.uid, credentials.gid) != (uid, gid)
            || proof.endpoint().is_none()
            || !proof.is_live()?
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "authenticated C3 endpoint does not match the boot admission",
            ));
        }
        Ok(proof)
    })();
    match auth {
        Ok(proof) => Ok(AuthenticatedBootService {
            proof,
            owner,
            admission,
            package,
        }),
        Err(error) => {
            owner.shutdown(RETIREMENT_BUDGET, POLL_INTERVAL)?;
            Err(error)
        }
    }
}
