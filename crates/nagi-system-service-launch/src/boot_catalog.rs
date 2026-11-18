use crate::{
    validate_descriptor_json, ArtifactPreflightReport, AuthenticatedSystemServiceCatalogBinding,
    DescriptorError, LaunchPortError, SystemServiceAdmissionError, SystemServiceRecoveryAdmission,
    TrustedSystemServiceCatalog, ValidatedSystemServiceDescriptor, VerifiedServicePackageBinding,
};
use ed25519_dalek::{Signature, VerifyingKey};
use nagi_identity_session::{CallerContextId, PrincipalId};
use nagi_package::TRUSTED_SIGNING_PUBLIC_KEY;
use serde::Deserialize;
use std::num::NonZeroU64;

const SIGNATURE_DOMAIN: &[u8] = b"Nagi Linux boot system service catalog v1\0";
const SIGNATURE_BYTES: usize = 64;
pub const MAX_SIGNED_BOOT_CATALOG_BYTES: usize = 32 * 1024 + SIGNATURE_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootCatalogError {
    InvalidLength,
    UntrustedSigner,
    InvalidSignature,
    InvalidRecord,
    InvalidPackageKey,
    InvalidIdentityMapping,
    InvalidDescriptor(DescriptorError),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogRecord {
    schema_version: u32,
    package_signing_public_key: [u8; 32],
    descriptor_json: String,
    principal_id: String,
    caller_context_id: String,
    mapping_generation: NonZeroU64,
}

/// One system-service record provisioned by the boot catalog and the C1
/// mapping owner. This adapter checks the complete descriptor before
/// producing the launch binding.
pub struct VerifiedBootCatalog {
    record: CatalogRecord,
    descriptor: ValidatedSystemServiceDescriptor,
    principal_id: PrincipalId,
    caller_context_id: CallerContextId,
}

impl VerifiedBootCatalog {
    /// Verify the JSON record followed by a raw Ed25519 signature. The boot
    /// caller must supply the catalog key from its verified init image.
    pub fn verify(
        file: &[u8],
        boot_catalog_public_key: &[u8; 32],
    ) -> Result<Self, BootCatalogError> {
        if file.len() <= SIGNATURE_BYTES || file.len() > MAX_SIGNED_BOOT_CATALOG_BYTES {
            return Err(BootCatalogError::InvalidLength);
        }
        let key = checked_product_key(boot_catalog_public_key)
            .ok_or(BootCatalogError::UntrustedSigner)?;
        let (payload, signature) = file.split_at(file.len() - SIGNATURE_BYTES);
        let signature =
            Signature::from_slice(signature).map_err(|_| BootCatalogError::InvalidSignature)?;
        let mut message = Vec::with_capacity(SIGNATURE_DOMAIN.len() + payload.len());
        message.extend_from_slice(SIGNATURE_DOMAIN);
        message.extend_from_slice(payload);
        key.verify_strict(&message, &signature)
            .map_err(|_| BootCatalogError::InvalidSignature)?;

        let record: CatalogRecord =
            serde_json::from_slice(payload).map_err(|_| BootCatalogError::InvalidRecord)?;
        if record.schema_version != 1 {
            return Err(BootCatalogError::InvalidRecord);
        }
        checked_product_key(&record.package_signing_public_key)
            .ok_or(BootCatalogError::InvalidPackageKey)?;
        let descriptor = validate_descriptor_json(&record.descriptor_json)
            .map_err(BootCatalogError::InvalidDescriptor)?;
        if !record.principal_id.starts_with("service:") {
            return Err(BootCatalogError::InvalidIdentityMapping);
        }
        let principal_id = PrincipalId::new(&record.principal_id)
            .map_err(|_| BootCatalogError::InvalidIdentityMapping)?;
        let caller_context_id = CallerContextId::new(&record.caller_context_id)
            .map_err(|_| BootCatalogError::InvalidIdentityMapping)?;
        Ok(Self {
            record,
            descriptor,
            principal_id,
            caller_context_id,
        })
    }

    pub fn descriptor(&self) -> &ValidatedSystemServiceDescriptor {
        &self.descriptor
    }

    pub fn descriptor_json(&self) -> &str {
        &self.record.descriptor_json
    }

    pub fn package_signing_public_key(&self) -> [u8; 32] {
        self.record.package_signing_public_key
    }

    /// Issue the scope used by the normal PID 1 process owner after artifact
    /// inspection, installed-package verification, and C3 reservation.
    pub fn admit_installed_package(
        &self,
        report: &ArtifactPreflightReport,
        package: &VerifiedServicePackageBinding<'_>,
        c3_launch_generation: NonZeroU64,
    ) -> Result<SystemServiceRecoveryAdmission, SystemServiceAdmissionError> {
        if report.artifact_sha256() != self.descriptor.expected_sha256() {
            return Err(SystemServiceAdmissionError::ArtifactDigestMismatch);
        }
        SystemServiceRecoveryAdmission::from_catalog_and_package(
            self.binding(),
            package,
            c3_launch_generation,
        )
    }

    fn binding(&self) -> AuthenticatedSystemServiceCatalogBinding {
        AuthenticatedSystemServiceCatalogBinding::from_trusted_catalog(
            &self.descriptor,
            self.principal_id.clone(),
            self.caller_context_id.clone(),
            self.record.mapping_generation,
        )
    }
}

impl TrustedSystemServiceCatalog for VerifiedBootCatalog {
    fn authenticate(
        &mut self,
        descriptor: &ValidatedSystemServiceDescriptor,
    ) -> Result<AuthenticatedSystemServiceCatalogBinding, LaunchPortError> {
        if descriptor != &self.descriptor {
            return Err(LaunchPortError::Rejected);
        }
        Ok(self.binding())
    }
}

fn checked_product_key(bytes: &[u8; 32]) -> Option<VerifyingKey> {
    if bytes == &TRUSTED_SIGNING_PUBLIC_KEY {
        return None;
    }
    let key = VerifyingKey::from_bytes(bytes).ok()?;
    (!key.is_weak()).then_some(key)
}
