use crate::{ValidatedSystemServiceDescriptor, SYSTEM_SERVICE_TARGET};
use serde::Deserialize;

const REQUIRED_NOT_PROVEN: [&str; 7] = [
    "signature",
    "canonical_package_identity",
    "service_role",
    "user_authentication",
    "policy_authorization",
    "installation",
    "runtime_ready",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreflightReportError {
    InvalidJson,
    WrongStatus,
    UnsupportedSchemaVersion,
    TargetMismatch,
    DigestMismatch,
    InvalidElfSummary,
    CheckFailed,
    ClaimScopeMismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactPreflightReport {
    artifact_sha256: String,
    elf_type: String,
    program_header_count: usize,
}

impl ArtifactPreflightReport {
    pub fn artifact_sha256(&self) -> &str {
        &self.artifact_sha256
    }

    pub fn elf_type(&self) -> &str {
        &self.elf_type
    }

    pub fn program_header_count(&self) -> usize {
        self.program_header_count
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReport {
    status: String,
    schema_version: u32,
    target: String,
    artifact_sha256: String,
    elf_type: String,
    program_header_count: usize,
    checks: RawChecks,
    claim_scope: String,
    not_proven: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawChecks {
    regular_file: bool,
    no_symlink_path_components: bool,
    sha256_matches_expected: bool,
    elf64_little_endian: bool,
    x86_64: bool,
    executable_elf: bool,
    no_pt_interp: bool,
}

/// Check that a helper report matches the descriptor and states its limited
/// claim scope. A valid report is still not a trust or authorization proof.
pub fn validate_artifact_preflight_report(
    input: &str,
    descriptor: &ValidatedSystemServiceDescriptor,
) -> Result<ArtifactPreflightReport, PreflightReportError> {
    let raw: RawReport =
        serde_json::from_str(input).map_err(|_| PreflightReportError::InvalidJson)?;
    if raw.status != "artifact_preflight_passed" {
        return Err(PreflightReportError::WrongStatus);
    }
    if raw.schema_version != 1 {
        return Err(PreflightReportError::UnsupportedSchemaVersion);
    }
    if raw.target != SYSTEM_SERVICE_TARGET {
        return Err(PreflightReportError::TargetMismatch);
    }
    if raw.artifact_sha256 != descriptor.expected_sha256() {
        return Err(PreflightReportError::DigestMismatch);
    }
    if !matches!(raw.elf_type.as_str(), "ET_EXEC" | "ET_DYN") || raw.program_header_count == 0 {
        return Err(PreflightReportError::InvalidElfSummary);
    }
    if !(raw.checks.regular_file
        && raw.checks.no_symlink_path_components
        && raw.checks.sha256_matches_expected
        && raw.checks.elf64_little_endian
        && raw.checks.x86_64
        && raw.checks.executable_elf
        && raw.checks.no_pt_interp)
    {
        return Err(PreflightReportError::CheckFailed);
    }
    if raw.claim_scope != "artifact_preflight_only"
        || REQUIRED_NOT_PROVEN
            .iter()
            .any(|claim| !raw.not_proven.iter().any(|item| item == claim))
    {
        return Err(PreflightReportError::ClaimScopeMismatch);
    }
    Ok(ArtifactPreflightReport {
        artifact_sha256: raw.artifact_sha256,
        elf_type: raw.elf_type,
        program_header_count: raw.program_header_count,
    })
}
