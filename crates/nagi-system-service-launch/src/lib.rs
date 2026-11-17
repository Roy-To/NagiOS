//! A-owned receiver and sequencing contract for Linux Identity system services.
//!
//! Parsing validates structure only. Production use also requires an
//! authenticated catalog, a preflight adapter that retains the checked file
//! descriptor, C3's real Linux process owner, and C1's private service seal.
//! No implementation in this crate manufactures those authorities or emits
//! production `READY`.

mod descriptor;
mod launch;
mod package_binding;
mod preflight;

pub use descriptor::{
    validate_descriptor_json, ArtifactPreflightRequest, DescriptorError, GenerationTransition,
    SystemServiceLaunchMetadata, ValidatedSystemServiceDescriptor, SYSTEM_SERVICE_TARGET,
};
pub use launch::{
    launch_identity_service, reconcile_identity_service, ArtifactPreflight,
    AuthenticatedSystemServiceCatalogBinding, C1RecoveryFailure, C1SystemServiceRecoveryClient,
    LaunchError, LaunchPortError, LifecycleFinish, ProofValidationError, RecoveryGateError,
    ReservationError, SystemServiceAdmissionError, SystemServiceLaunchLifecycle,
    SystemServiceLaunchOwner, SystemServiceReconciliationGate, SystemServiceRecoveryAdmission,
    SystemServiceRecoveryGate, SystemServiceRuntime, SystemServiceRuntimeSlotBinding,
    SystemServiceRuntimeSlotKey, TrustedSystemServiceCatalog,
};
pub use package_binding::{
    bind_installed_service_package, PackageBindingError, VerifiedServicePackageBinding,
};
pub use preflight::{
    validate_artifact_preflight_report, ArtifactPreflightReport, PreflightReportError,
};
