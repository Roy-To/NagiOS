//! Host-side extension contract and observation adapter for Nagi app lifecycle.
//!
//! The base manifest and state machine come from `nagi-sdk::app_contract`.
//! This crate adds package/runtime compatibility declarations and attaches the
//! stable AppId to lifecycle observations without owning launch policy.

mod lifecycle;
mod manifest;
mod version;

pub use lifecycle::{
    LifecycleFailure, LifecycleFailureCategory, LifecycleObservation, LifecycleReason,
    ManagedApplication,
};
pub use manifest::{
    parse_app_package_manifest, AppLifecycleExtension, AppPackageManifest, ManifestValidationError,
    ManifestValidationErrorKind, ServiceRequirement, APP_LIFECYCLE_EXTENSION_ID,
    APP_LIFECYCLE_EXTENSION_SCHEMA_VERSION,
};
pub use version::AppVersion;
