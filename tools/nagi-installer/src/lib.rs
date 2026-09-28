//! Host-side application package installation foundation.
//!
//! This crate consumes the public Nagi App SDK manifest contract through an
//! adapter, but does not define or serialize a competing app manifest. Its
//! directory package source is deliberately independent of any archive format.

mod error;
mod installer;
mod metadata;
mod package;
mod path;
mod state;

pub use error::{ErrorKind, InstallerError, PolicyName};
pub use installer::{
    FailureInjector, FailurePoint, InstallPlan, Installer, NoFailureInjector, NoopPolicy,
    PolicyCheck, PolicyDecision, PolicyEvaluator, PolicyOperation, PolicyReferences, UninstallPlan,
};
pub use metadata::{AppSdkManifestAdapter, ManifestAdapter, MetadataReferences, PackageMetadata};
pub use package::{PackageSource, VirtualFile};
pub use path::PackageRelativePath;
pub use state::{
    CapabilityDelta, InstallOperation, InstalledPackage, InventorySnapshot, TransactionPhase,
    UninstallOutcome, INVENTORY_SCHEMA_VERSION, JOURNAL_SCHEMA_VERSION,
};
