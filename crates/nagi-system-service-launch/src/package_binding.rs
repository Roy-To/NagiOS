use crate::ValidatedSystemServiceDescriptor;
use nagi_init_package_provider::package_trust::{
    PackageServiceGenerationLease, PackageServiceSessionOwner, PackageServiceSessionOwnerError,
};
use nagi_model::AppId;
use nagi_package::{PackageError, PackageView};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageBindingError {
    Package(PackageError),
    Owner(PackageServiceSessionOwnerError),
    ManifestIdMismatch,
    AppIdMismatch,
    CanonicalEntryMismatch,
    ExecutableDigestMismatch,
}

/// Authenticated package values held together with the package service's
/// dispatch lease. The lease remains live for the entire A launch runtime.
pub struct VerifiedServicePackageBinding<'owner> {
    app_id: AppId,
    package_id: String,
    canonical_entry: String,
    executable_sha256: String,
    executable_bytes: Box<[u8]>,
    lease: PackageServiceGenerationLease<'owner>,
}

impl VerifiedServicePackageBinding<'_> {
    pub fn app_id(&self) -> AppId {
        self.app_id
    }

    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    pub fn canonical_entry(&self) -> &str {
        &self.canonical_entry
    }

    pub fn executable_sha256(&self) -> &str {
        &self.executable_sha256
    }

    pub fn package_generation(&self) -> u64 {
        self.lease.snapshot().generation()
    }

    pub fn signed_package_sha256(&self) -> &[u8; 32] {
        self.lease.snapshot().signed_package_sha256()
    }

    pub fn executable_sha256_bytes(&self) -> &[u8; 32] {
        self.lease.snapshot().executable_sha256()
    }

    pub fn signed_package_bytes_len(&self) -> u32 {
        self.lease.snapshot().signed_package_bytes_len()
    }

    pub fn executable_bytes_len(&self) -> u32 {
        self.lease.snapshot().executable_bytes_len()
    }

    /// Immutable copy of the exact executable bytes covered by the installed,
    /// signed package generation. Launch owners must load these bytes and must
    /// not reopen the descriptor's artifact pathname or reuse a mutable file
    /// descriptor after preflight.
    pub fn executable_bytes(&self) -> &[u8] {
        &self.executable_bytes
    }

    pub(crate) fn quarantine(&self) {
        self.lease.quarantine();
    }
}

/// Use the public package owner to authenticate the exact signed package,
/// compare both catalog-selected manifest identity and entry, compare embedded
/// executable bytes to the catalog digest, and retain the exact installed
/// package generation through launch/runtime lifetime.
pub fn bind_installed_service_package<'owner>(
    descriptor: &ValidatedSystemServiceDescriptor,
    package_bytes: &[u8],
    package_owner: &'owner PackageServiceSessionOwner<'_>,
) -> Result<VerifiedServicePackageBinding<'owner>, PackageBindingError> {
    let package = PackageView::parse(package_bytes).map_err(PackageBindingError::Package)?;
    let manifest = package.manifest();
    if manifest.id() != descriptor.package_id().as_bytes() {
        return Err(PackageBindingError::ManifestIdMismatch);
    }
    if manifest.app_id() != descriptor.package_app_id() {
        return Err(PackageBindingError::AppIdMismatch);
    }
    if manifest.entry() != descriptor.canonical_entry().as_bytes() {
        return Err(PackageBindingError::CanonicalEntryMismatch);
    }
    // Copy the exact signed-package entry before launch and hash the copy that
    // C3 will receive. A retained read FD proves inode identity, not immutable
    // contents; the owned slice closes that write-after-preflight window.
    let executable_bytes = package.executable().to_vec().into_boxed_slice();
    let executable_sha256 = sha256_hex(&executable_bytes);
    if executable_sha256 != descriptor.expected_sha256() {
        return Err(PackageBindingError::ExecutableDigestMismatch);
    }

    // capture authenticates the signature against the owner's configured key,
    // checks installed identity/generation, and delegates to the public
    // PackageSecurityProvider. System-service binding additionally requires a
    // durable digest-index readback for this exact inventory. The lease then
    // fences manifest or generation replacement until SystemServiceRuntime
    // is dropped.
    let snapshot = package_owner
        .capture_for_system_service(package_bytes, descriptor.package_id().as_bytes())
        .map_err(PackageBindingError::Owner)?;
    let lease = package_owner
        .lease_for_system_service_dispatch(&snapshot)
        .map_err(PackageBindingError::Owner)?;
    Ok(VerifiedServicePackageBinding {
        app_id: descriptor.package_app_id(),
        package_id: descriptor.package_id().to_owned(),
        canonical_entry: descriptor.canonical_entry().to_owned(),
        executable_sha256,
        executable_bytes,
        lease,
    })
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}
