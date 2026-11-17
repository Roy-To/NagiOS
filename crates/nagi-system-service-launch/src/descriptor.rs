use nagi_capability::PrincipalKind;
use nagi_model::AppId;
use nagi_service_contract::ServiceId;
use serde::{Deserialize, Serialize};
use std::num::NonZeroU64;

pub const SYSTEM_SERVICE_TARGET: &str = "x86_64-unknown-linux-musl";
const MAX_DESCRIPTOR_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorError {
    TooLarge,
    InvalidJson,
    UnsupportedSchemaVersion,
    InvalidServiceId,
    InvalidPackageId,
    InvalidCanonicalEntry,
    WrongRole,
    InvalidArtifactPath,
    InvalidDigest,
    UnsupportedTarget,
    ZeroGeneration,
    InvalidGenerationTransition,
    UnsupportedEndpoint,
    PeerCredentialsMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GenerationTransition {
    Initial,
    Replace { previous_generation: NonZeroU64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedSystemServiceDescriptor {
    service_id: ServiceId,
    package_id: String,
    canonical_entry: String,
    artifact_path: String,
    expected_sha256: String,
    uid: u32,
    gid: u32,
    generation: NonZeroU64,
    transition: GenerationTransition,
}

/// Path-free C3 input. The artifact pathname is intentionally excluded so a
/// launch owner can only consume the immutable executable bytes from the
/// authenticated package binding.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemServiceLaunchMetadata {
    service_id: ServiceId,
    package_id: String,
    package_app_id: AppId,
    canonical_entry: String,
    expected_sha256: String,
    uid: u32,
    gid: u32,
    generation: NonZeroU64,
}

impl SystemServiceLaunchMetadata {
    pub fn service_id(&self) -> &ServiceId {
        &self.service_id
    }

    pub fn role(&self) -> PrincipalKind {
        PrincipalKind::SystemService
    }

    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    pub fn package_app_id(&self) -> AppId {
        self.package_app_id
    }

    pub fn canonical_entry(&self) -> &str {
        &self.canonical_entry
    }

    pub fn expected_sha256(&self) -> &str {
        &self.expected_sha256
    }

    pub fn credentials(&self) -> (u32, u32) {
        (self.uid, self.gid)
    }

    pub fn generation(&self) -> NonZeroU64 {
        self.generation
    }
}

impl ValidatedSystemServiceDescriptor {
    pub fn service_id(&self) -> &ServiceId {
        &self.service_id
    }

    pub fn role(&self) -> PrincipalKind {
        PrincipalKind::SystemService
    }

    pub fn package_id(&self) -> &str {
        &self.package_id
    }

    pub fn package_app_id(&self) -> AppId {
        AppId::from_identifier(self.package_id.as_bytes())
    }

    pub fn canonical_entry(&self) -> &str {
        &self.canonical_entry
    }

    pub fn artifact_path(&self) -> &str {
        &self.artifact_path
    }

    pub fn expected_sha256(&self) -> &str {
        &self.expected_sha256
    }

    pub fn credentials(&self) -> (u32, u32) {
        (self.uid, self.gid)
    }

    pub fn generation(&self) -> NonZeroU64 {
        self.generation
    }

    pub fn transition(&self) -> GenerationTransition {
        self.transition
    }

    pub fn launch_metadata(&self) -> SystemServiceLaunchMetadata {
        SystemServiceLaunchMetadata {
            service_id: self.service_id.clone(),
            package_id: self.package_id.clone(),
            package_app_id: self.package_app_id(),
            canonical_entry: self.canonical_entry.clone(),
            expected_sha256: self.expected_sha256.clone(),
            uid: self.uid,
            gid: self.gid,
            generation: self.generation,
        }
    }

    pub fn artifact_preflight_request(&self) -> ArtifactPreflightRequest {
        ArtifactPreflightRequest {
            schema_version: 1,
            artifact_path: self.artifact_path.clone(),
            expected_sha256: self.expected_sha256.clone(),
            target: SYSTEM_SERVICE_TARGET.to_owned(),
        }
    }
}

/// Exact schema-v1 input accepted by `tools/service-artifact-preflight`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPreflightRequest {
    schema_version: u32,
    artifact_path: String,
    expected_sha256: String,
    target: String,
}

impl ArtifactPreflightRequest {
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn artifact_path(&self) -> &str {
        &self.artifact_path
    }

    pub fn expected_sha256(&self) -> &str {
        &self.expected_sha256
    }

    pub fn target(&self) -> &str {
        &self.target
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDescriptor {
    schema_version: u32,
    service_id: String,
    role: PrincipalKind,
    package: RawPackage,
    executable: RawExecutable,
    credentials: RawCredentials,
    generation: u64,
    lifecycle: RawLifecycle,
    endpoint: RawEndpoint,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackage {
    id: String,
    canonical_entry: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExecutable {
    artifact_path: String,
    expected_sha256: String,
    target: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCredentials {
    uid: u32,
    gid: u32,
}

#[derive(Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum RawLifecycle {
    Initial,
    Replace { previous_generation: u64 },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEndpoint {
    transport: String,
    owner: String,
    handoff: String,
    generation_policy: String,
    expected_peer_uid: u32,
    expected_peer_gid: u32,
}

/// Parse and structurally validate a catalog-shaped descriptor. This function
/// does not authenticate the catalog or authorize process launch.
pub fn validate_descriptor_json(
    input: &str,
) -> Result<ValidatedSystemServiceDescriptor, DescriptorError> {
    if input.len() > MAX_DESCRIPTOR_BYTES {
        return Err(DescriptorError::TooLarge);
    }
    let raw: RawDescriptor =
        serde_json::from_str(input).map_err(|_| DescriptorError::InvalidJson)?;
    if raw.schema_version != 1 {
        return Err(DescriptorError::UnsupportedSchemaVersion);
    }
    if raw.role != PrincipalKind::SystemService {
        return Err(DescriptorError::WrongRole);
    }
    let service_id =
        ServiceId::new(raw.service_id).map_err(|_| DescriptorError::InvalidServiceId)?;
    if raw.package.id.is_empty()
        || !raw.package.id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
    {
        return Err(DescriptorError::InvalidPackageId);
    }
    if raw.package.canonical_entry.is_empty()
        || raw.package.canonical_entry.contains("..")
        || raw.package.canonical_entry.contains('/')
        || raw.package.canonical_entry.contains('\\')
        || raw.package.canonical_entry.contains('\0')
        || raw.package.canonical_entry.contains('\r')
        || raw.package.canonical_entry.contains('\n')
    {
        return Err(DescriptorError::InvalidCanonicalEntry);
    }
    if raw.executable.artifact_path.is_empty()
        || raw.executable.artifact_path.contains('\0')
        || raw
            .executable
            .artifact_path
            .split('/')
            .any(|component| component == "..")
    {
        return Err(DescriptorError::InvalidArtifactPath);
    }
    if !is_lower_hex_sha256(&raw.executable.expected_sha256) {
        return Err(DescriptorError::InvalidDigest);
    }
    if raw.executable.target != SYSTEM_SERVICE_TARGET {
        return Err(DescriptorError::UnsupportedTarget);
    }
    let generation = NonZeroU64::new(raw.generation).ok_or(DescriptorError::ZeroGeneration)?;
    let transition = match raw.lifecycle {
        RawLifecycle::Initial => GenerationTransition::Initial,
        RawLifecycle::Replace {
            previous_generation,
        } => {
            let previous_generation = NonZeroU64::new(previous_generation)
                .ok_or(DescriptorError::InvalidGenerationTransition)?;
            if previous_generation >= generation {
                return Err(DescriptorError::InvalidGenerationTransition);
            }
            GenerationTransition::Replace {
                previous_generation,
            }
        }
    };
    if raw.endpoint.transport != "unix_stream"
        || raw.endpoint.owner != "c3_pidfd_supervisor_generation"
        || raw.endpoint.handoff != "connected_owned_fd"
        || raw.endpoint.generation_policy != "unique_per_connection"
    {
        return Err(DescriptorError::UnsupportedEndpoint);
    }
    if raw.endpoint.expected_peer_uid != raw.credentials.uid
        || raw.endpoint.expected_peer_gid != raw.credentials.gid
    {
        return Err(DescriptorError::PeerCredentialsMismatch);
    }
    Ok(ValidatedSystemServiceDescriptor {
        service_id,
        package_id: raw.package.id,
        canonical_entry: raw.package.canonical_entry,
        artifact_path: raw.executable.artifact_path,
        expected_sha256: raw.executable.expected_sha256,
        uid: raw.credentials.uid,
        gid: raw.credentials.gid,
        generation,
        transition,
    })
}

fn is_lower_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
