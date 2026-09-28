use std::collections::BTreeSet;

use nagi_sdk::app_contract::{
    is_valid_app_identifier, is_valid_app_version, AppManifestContract, APP_MANIFEST_SCHEMA_VERSION,
};

use crate::{InstallerError, PackageRelativePath};

/// Installer-relevant projection of the canonical SDK manifest, not an
/// alternate manifest format. APP-LC-01 remains the source of manifest truth.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PackageMetadata {
    pub app_id: String,
    pub version: String,
    pub manifest_schema_version: u16,
    pub entrypoint: String,
    pub required_capabilities: Vec<String>,
    pub references: MetadataReferences,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MetadataReferences {
    pub sbom: Option<String>,
    pub license: Option<String>,
    pub provenance: Option<String>,
}

pub trait ManifestAdapter<M: ?Sized> {
    fn adapt(&self, manifest: &M) -> Result<PackageMetadata, InstallerError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AppSdkManifestAdapter;

impl ManifestAdapter<AppManifestContract<'_>> for AppSdkManifestAdapter {
    fn adapt(&self, manifest: &AppManifestContract<'_>) -> Result<PackageMetadata, InstallerError> {
        if u32::from(manifest.schema_version) != u32::from(APP_MANIFEST_SCHEMA_VERSION) {
            return Err(InstallerError::UnsupportedSchema {
                kind: "app manifest",
                version: u32::from(manifest.schema_version),
            });
        }
        manifest
            .validate()
            .map_err(|error| InstallerError::InvalidMetadata(format!("SDK manifest: {error:?}")))?;

        let required_capabilities = manifest
            .requested_capabilities
            .iter()
            .map(|capability| capability.id.to_owned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        let metadata = PackageMetadata {
            app_id: manifest.identity.identifier().to_owned(),
            version: manifest.identity.version().to_owned(),
            manifest_schema_version: manifest.schema_version,
            entrypoint: manifest.entrypoint.target.to_owned(),
            required_capabilities,
            references: MetadataReferences::default(),
        };
        metadata.validate()?;
        Ok(metadata)
    }
}

impl PackageMetadata {
    pub fn with_references(
        mut self,
        references: MetadataReferences,
    ) -> Result<Self, InstallerError> {
        self.references = references;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), InstallerError> {
        if self.manifest_schema_version != APP_MANIFEST_SCHEMA_VERSION {
            return Err(InstallerError::UnsupportedSchema {
                kind: "app manifest",
                version: u32::from(self.manifest_schema_version),
            });
        }
        if !is_valid_app_identifier(&self.app_id) {
            return Err(InstallerError::InvalidMetadata(
                "app ID is not a canonical SDK application identifier".into(),
            ));
        }
        if !is_valid_app_version(&self.version) {
            return Err(InstallerError::InvalidMetadata(
                "version is not a valid SDK semantic version".into(),
            ));
        }
        PackageRelativePath::parse(&self.entrypoint)?;
        let mut capabilities = BTreeSet::new();
        for capability in &self.required_capabilities {
            if capability.is_empty()
                || !valid_capability_id(capability)
                || !capabilities.insert(capability)
            {
                return Err(InstallerError::InvalidMetadata(
                    "required capability IDs must be unique canonical ASCII identifiers".into(),
                ));
            }
        }
        for reference in [
            self.references.sbom.as_deref(),
            self.references.license.as_deref(),
            self.references.provenance.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if reference.is_empty() || reference.contains('\0') {
                return Err(InstallerError::InvalidMetadata(
                    "metadata references must be non-empty UTF-8 values".into(),
                ));
            }
        }
        Ok(())
    }
}

fn valid_capability_id(value: &str) -> bool {
    if value.len() > 128 {
        return false;
    }
    let mut segments = value.split('.');
    let Some(first) = segments.next() else {
        return false;
    };
    let valid_segment = |segment: &str| {
        let mut bytes = segment.bytes();
        bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
            && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    valid_segment(first)
        && segments.next().is_some_and(valid_segment)
        && segments.all(valid_segment)
}
