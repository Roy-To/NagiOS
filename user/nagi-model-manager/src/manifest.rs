use alloc::{string::String, vec::Vec};
use core::fmt;

use serde::{Deserialize, Serialize};

pub const MODEL_MANIFEST_SCHEMA_VERSION: u16 = 1;
pub const MODEL_RUNTIME_API_VERSION: &str = "nagi.ai/1";
pub const MAX_MODEL_MANIFEST_BYTES: usize = 64 * 1024;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ManifestError> {
                let value = value.into();
                if !valid_identifier(&value) {
                    return Err(ManifestError::InvalidIdentifier);
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

string_id!(ModelId);
string_id!(ProviderId);
string_id!(CapabilityId);
string_id!(ModalityId);
string_id!(RoleId);
string_id!(BackendId);
string_id!(ArtifactId);
string_id!(FormatId);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelManifest {
    pub schema_version: u16,
    pub model_id: ModelId,
    pub display_name: String,
    pub provider: ProviderMetadata,
    pub version: String,
    pub variant: String,
    pub runtime_api_version: String,
    pub execution: ExecutionScope,
    pub artifact: ArtifactDescriptor,
    pub capabilities: Vec<CapabilityId>,
    pub modalities: Vec<ModalityId>,
    pub context: ContextLimits,
    pub resources: ResourceRequirements,
    pub supported_backends: Vec<BackendId>,
    pub roles: Vec<RoleId>,
    pub compatibility: Compatibility,
    pub license: LicenseMetadata,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub source: Option<SourceMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderMetadata {
    pub provider_id: ProviderId,
    pub display_name: String,
    pub family: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionScope {
    Local,
    Remote,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactDescriptor {
    pub format: FormatId,
    pub reference: ArtifactReference,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub size_bytes: Option<u64>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub integrity: Option<IntegrityMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArtifactReference {
    ModelStore { artifact_id: ArtifactId },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrityMetadata {
    pub algorithm: String,
    pub digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextLimits {
    pub max_input_tokens: u32,
    pub max_output_tokens: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceRequirements {
    pub minimum_ram_bytes: u64,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub recommended_ram_bytes: Option<u64>,
    pub minimum_storage_bytes: u64,
    pub minimum_cpu_cores: u8,
    pub gpu_required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compatibility {
    pub architectures: Vec<String>,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub minimum_os_version: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LicenseMetadata {
    pub identifier: String,
    #[serde(deserialize_with = "deserialize_required_option")]
    pub terms_reference: Option<String>,
    pub acknowledgement_required: bool,
    pub notices: Vec<NoticeRequirement>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoticeRequirement {
    pub notice_id: String,
    pub reference: String,
    pub required: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMetadata {
    pub uri: String,
    pub revision: String,
    pub file_name: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestError {
    Empty,
    TooLarge,
    MalformedJson,
    UnsupportedSchemaVersion,
    UnsupportedRuntimeApi,
    InvalidIdentifier,
    MissingRequiredValue,
    InvalidBounds,
    InvalidIntegrity,
    InvalidSourceMetadata,
    DuplicateValue,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Self::Empty => "manifest is empty",
            Self::TooLarge => "manifest exceeds the size limit",
            Self::MalformedJson => "manifest JSON is malformed or has an unknown field",
            Self::UnsupportedSchemaVersion => "manifest schema version is unsupported",
            Self::UnsupportedRuntimeApi => "model runtime API version is unsupported",
            Self::InvalidIdentifier => "manifest contains an invalid identifier",
            Self::MissingRequiredValue => "manifest is missing a required value",
            Self::InvalidBounds => "manifest contains invalid context or resource bounds",
            Self::InvalidIntegrity => "manifest contains invalid integrity metadata",
            Self::InvalidSourceMetadata => "source metadata is incomplete",
            Self::DuplicateValue => "manifest contains a duplicate value",
        };
        formatter.write_str(description)
    }
}

fn deserialize_required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

impl ModelManifest {
    pub fn parse_json(bytes: &[u8]) -> Result<Self, ManifestError> {
        if bytes.is_empty() {
            return Err(ManifestError::Empty);
        }
        if bytes.len() > MAX_MODEL_MANIFEST_BYTES {
            return Err(ManifestError::TooLarge);
        }
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|_| ManifestError::MalformedJson)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema_version != MODEL_MANIFEST_SCHEMA_VERSION {
            return Err(ManifestError::UnsupportedSchemaVersion);
        }
        if self.runtime_api_version != MODEL_RUNTIME_API_VERSION {
            return Err(ManifestError::UnsupportedRuntimeApi);
        }
        if !valid_identifier(self.model_id.as_str())
            || !valid_identifier(self.provider.provider_id.as_str())
        {
            return Err(ManifestError::InvalidIdentifier);
        }
        if !valid_text(&self.display_name)
            || !valid_text(&self.provider.display_name)
            || !valid_text(&self.provider.family)
            || !valid_text(&self.version)
            || !valid_text(&self.variant)
            || !valid_text(&self.license.identifier)
        {
            return Err(ManifestError::MissingRequiredValue);
        }
        if self.capabilities.is_empty()
            || self.modalities.is_empty()
            || self.supported_backends.is_empty()
            || self.roles.is_empty()
        {
            return Err(ManifestError::MissingRequiredValue);
        }
        if self.context.max_input_tokens == 0
            || self.context.max_output_tokens == 0
            || self.resources.minimum_ram_bytes == 0
            || self.artifact.size_bytes == Some(0)
            || self.resources.minimum_cpu_cores == 0
            || self
                .resources
                .recommended_ram_bytes
                .is_some_and(|recommended| recommended < self.resources.minimum_ram_bytes)
        {
            return Err(ManifestError::InvalidBounds);
        }
        validate_unique(self.capabilities.iter().map(CapabilityId::as_str))?;
        validate_unique(self.modalities.iter().map(ModalityId::as_str))?;
        validate_unique(self.supported_backends.iter().map(BackendId::as_str))?;
        validate_unique(self.roles.iter().map(RoleId::as_str))?;
        validate_unique(self.compatibility.architectures.iter().map(String::as_str))?;
        validate_unique(
            self.license
                .notices
                .iter()
                .map(|notice| notice.notice_id.as_str()),
        )?;
        for identifier in self
            .capabilities
            .iter()
            .map(CapabilityId::as_str)
            .chain(self.modalities.iter().map(ModalityId::as_str))
            .chain(self.supported_backends.iter().map(BackendId::as_str))
            .chain(self.roles.iter().map(RoleId::as_str))
        {
            if !valid_identifier(identifier) {
                return Err(ManifestError::InvalidIdentifier);
            }
        }
        for architecture in &self.compatibility.architectures {
            if !valid_identifier(architecture) {
                return Err(ManifestError::InvalidIdentifier);
            }
        }
        if self
            .compatibility
            .minimum_os_version
            .as_deref()
            .is_some_and(|value| !valid_text(value))
        {
            return Err(ManifestError::MissingRequiredValue);
        }
        if self
            .license
            .terms_reference
            .as_deref()
            .is_some_and(|reference| !valid_text(reference))
        {
            return Err(ManifestError::MissingRequiredValue);
        }
        if let Some(integrity) = &self.artifact.integrity {
            if integrity.algorithm != "sha256" || !valid_sha256(&integrity.digest) {
                return Err(ManifestError::InvalidIntegrity);
            }
        }
        if self.license.acknowledgement_required
            && !self
                .license
                .terms_reference
                .as_deref()
                .is_some_and(valid_text)
        {
            return Err(ManifestError::MissingRequiredValue);
        }
        for notice in &self.license.notices {
            if !valid_identifier(&notice.notice_id) || !valid_text(&notice.reference) {
                return Err(ManifestError::MissingRequiredValue);
            }
        }
        if let Some(source) = &self.source {
            if !valid_text(&source.uri)
                || !valid_text(&source.revision)
                || !valid_file_name(&source.file_name)
                || self.artifact.integrity.is_none()
                || self.artifact.size_bytes.is_none_or(|size| size == 0)
            {
                return Err(ManifestError::InvalidSourceMetadata);
            }
        }
        if !valid_identifier(self.artifact.format.as_str()) {
            return Err(ManifestError::InvalidIdentifier);
        }
        let ArtifactReference::ModelStore { artifact_id } = &self.artifact.reference;
        if !valid_identifier(artifact_id.as_str()) {
            return Err(ManifestError::InvalidIdentifier);
        }
        Ok(())
    }

    pub fn has_capability(&self, requested: &CapabilityId) -> bool {
        self.capabilities
            .iter()
            .any(|capability| capability == requested)
    }

    pub fn has_role(&self, requested: &RoleId) -> bool {
        self.roles.iter().any(|role| role == requested)
    }
}

fn valid_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(*byte, b'.' | b'_' | b'-')
        })
}

fn valid_text(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 4096 && !value.chars().any(char::is_control)
}

fn valid_file_name(value: &str) -> bool {
    valid_text(value)
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_unique<'a>(mut values: impl Iterator<Item = &'a str>) -> Result<(), ManifestError> {
    let mut seen: Vec<&str> = Vec::new();
    for value in values.by_ref() {
        if seen.contains(&value) {
            return Err(ManifestError::DuplicateValue);
        }
        seen.push(value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = include_str!("../tests/fixtures/granite-4.2-3b.json");

    #[test]
    fn accepts_versioned_manifest_and_retains_license_metadata() {
        let manifest = ModelManifest::parse_json(VALID.as_bytes()).expect("valid manifest");
        assert_eq!(manifest.model_id.as_str(), "ibm.granite-4.2-3b");
        assert_eq!(manifest.license.identifier, "provider-terms:ibm-granite");
        assert_eq!(
            manifest.license.terms_reference.as_deref(),
            Some("provider-terms:ibm.granite")
        );
        assert_eq!(manifest.license.notices[0].notice_id, "model-notice");
    }

    #[test]
    fn rejects_unknown_fields_and_malformed_json() {
        let with_unknown = VALID.replace(
            "\"schema_version\": 1,",
            "\"schema_version\": 1,\n  \"unexpected\": true,",
        );
        assert_eq!(
            ModelManifest::parse_json(with_unknown.as_bytes()),
            Err(ManifestError::MalformedJson)
        );
        assert_eq!(
            ModelManifest::parse_json(b"{"),
            Err(ManifestError::MalformedJson)
        );
        let windows_line_endings = VALID.replace("\r\n", "\n").replace('\n', "\r\n");
        let missing_nullable_field = windows_line_endings.replace("\"source\": null", "");
        assert_eq!(
            ModelManifest::parse_json(missing_nullable_field.as_bytes()),
            Err(ManifestError::MalformedJson)
        );
    }

    #[test]
    fn rejects_unsupported_schema_and_runtime_versions() {
        let schema = VALID.replace("\"schema_version\": 1", "\"schema_version\": 2");
        assert_eq!(
            ModelManifest::parse_json(schema.as_bytes()),
            Err(ManifestError::UnsupportedSchemaVersion)
        );
        let runtime = VALID.replace(MODEL_RUNTIME_API_VERSION, "nagi.ai/99");
        assert_eq!(
            ModelManifest::parse_json(runtime.as_bytes()),
            Err(ManifestError::UnsupportedRuntimeApi)
        );
    }

    #[test]
    fn rejects_invalid_bounds_and_integrity() {
        let low_recommended = VALID.replace(
            "\"recommended_ram_bytes\": 4294967296",
            "\"recommended_ram_bytes\": 1",
        );
        assert_eq!(
            ModelManifest::parse_json(low_recommended.as_bytes()),
            Err(ManifestError::InvalidBounds)
        );
        let mut invalid_integrity = ModelManifest::parse_json(VALID.as_bytes()).unwrap();
        invalid_integrity.artifact.integrity = Some(IntegrityMetadata {
            algorithm: String::from("sha256"),
            digest: String::from("not-a-digest"),
        });
        assert_eq!(
            invalid_integrity.validate(),
            Err(ManifestError::InvalidIntegrity)
        );
    }

    #[test]
    fn terms_acknowledgement_requires_a_separate_terms_reference() {
        let mut manifest = ModelManifest::parse_json(VALID.as_bytes()).unwrap();
        manifest.license.acknowledgement_required = true;
        manifest.license.terms_reference = None;
        assert_eq!(
            manifest.validate(),
            Err(ManifestError::MissingRequiredValue)
        );
    }

    #[test]
    fn rejects_source_metadata_that_looks_like_a_filesystem_path() {
        let mut manifest = ModelManifest::parse_json(VALID.as_bytes()).unwrap();
        manifest.artifact.size_bytes = Some(1);
        manifest.artifact.integrity = Some(IntegrityMetadata {
            algorithm: String::from("sha256"),
            digest: "a".repeat(64),
        });
        manifest.source = Some(SourceMetadata {
            uri: String::from("provider-source:model"),
            revision: String::from("revision-1"),
            file_name: String::from("../model.gguf"),
        });
        assert_eq!(
            manifest.validate(),
            Err(ManifestError::InvalidSourceMetadata)
        );
    }

    #[test]
    fn accepts_unrecognized_future_capabilities_without_caller_changes() {
        let future = VALID.replace("text.generate", "system_one");
        let manifest =
            ModelManifest::parse_json(future.as_bytes()).expect("future capability is data");
        let capability = CapabilityId::new("system_one").expect("capability id");
        assert!(manifest.has_capability(&capability));
    }
}
