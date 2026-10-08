use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Inventory {
    pub schema_version: u32,
    pub project: ProjectRecord,
    pub repository_revision: Option<String>,
    pub components: Vec<ComponentRecord>,
    pub model_assets: Vec<ModelLicenseRecord>,
    pub findings: Vec<Finding>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProjectRecord {
    pub name: String,
    pub version: String,
    pub declared_license_expression: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ComponentRecord {
    pub component_id: String,
    pub name: String,
    pub package_type: String,
    pub version: Option<String>,
    pub revision: Option<String>,
    pub origin: Option<String>,
    pub local_path: Option<String>,
    pub purl: Option<String>,
    pub checksum: Option<String>,
    pub declared_license_expression: Option<String>,
    pub raw_license_metadata: Option<String>,
    pub license_status: String,
    pub license_file_declared: Option<String>,
    pub detected_license_files: Vec<String>,
    pub notice_files: Vec<String>,
    pub directness: String,
    pub scopes: Vec<String>,
    pub dependencies: Vec<String>,
    pub vendored: bool,
    pub review_status: String,
    pub review_recorded: bool,
    pub evidence: Vec<EvidenceRecord>,
    pub conflicts: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EvidenceRecord {
    pub kind: String,
    pub source: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub code: String,
    pub severity: String,
    pub component_id: Option<String>,
    pub message: String,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelLicenseRecord {
    pub model_id: String,
    pub display_name: String,
    pub provider_id: String,
    pub version: String,
    pub variant: String,
    pub inclusion_status: String,
    pub artifact_present: bool,
    pub opaque_license_identifier: Option<String>,
    pub terms_reference: Option<String>,
    pub acknowledgement_required: Option<bool>,
    pub notices: Vec<ModelNoticeRecord>,
    pub declared_license_expression: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_license_metadata: Option<String>,
    pub review_status: String,
    pub evidence_source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ModelNoticeRecord {
    pub notice_id: String,
    pub reference: String,
    pub required: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InventoryDiff {
    pub added: Vec<ComponentRecord>,
    pub removed: Vec<ComponentRecord>,
    pub version_changes: Vec<VersionChange>,
    pub license_changes: Vec<LicenseChange>,
    pub new_missing_license_conditions: Vec<ComponentRecord>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VersionChange {
    pub name: String,
    pub from: String,
    pub to: String,
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct LicenseChange {
    pub component_id: String,
    pub name: String,
    pub from: Option<String>,
    pub to: Option<String>,
}
