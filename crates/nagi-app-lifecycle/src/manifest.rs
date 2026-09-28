use std::collections::{BTreeMap, BTreeSet};

use nagi_sdk::app_contract::{AppManifestContract, ContractVersion};
use serde_json::{Map, Value};

use crate::AppVersion;

pub const APP_LIFECYCLE_EXTENSION_ID: &str = "org.nagi.app-lifecycle";
pub const APP_LIFECYCLE_EXTENSION_SCHEMA_VERSION: u64 = 1;

/// Stable categories for callers. `field` and `index` identify the invalid
/// location without requiring callers to inspect diagnostic prose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestValidationErrorKind {
    InvalidDocument,
    InvalidBaseManifest,
    MissingField,
    UnknownField,
    InvalidType,
    UnsupportedVersion,
    InvalidValue,
    InvalidVersion,
    DuplicateDeclaration,
    ContradictoryDeclaration,
}

impl ManifestValidationErrorKind {
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::InvalidDocument => "APP_LC_INVALID_DOCUMENT",
            Self::InvalidBaseManifest => "APP_LC_INVALID_BASE_MANIFEST",
            Self::MissingField => "APP_LC_MISSING_FIELD",
            Self::UnknownField => "APP_LC_UNKNOWN_FIELD",
            Self::InvalidType => "APP_LC_INVALID_TYPE",
            Self::UnsupportedVersion => "APP_LC_UNSUPPORTED_VERSION",
            Self::InvalidValue => "APP_LC_INVALID_VALUE",
            Self::InvalidVersion => "APP_LC_INVALID_VERSION",
            Self::DuplicateDeclaration => "APP_LC_DUPLICATE_DECLARATION",
            Self::ContradictoryDeclaration => "APP_LC_CONTRADICTORY_DECLARATION",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestValidationError {
    pub kind: ManifestValidationErrorKind,
    pub field: String,
    pub index: Option<usize>,
}

impl ManifestValidationError {
    fn new(kind: ManifestValidationErrorKind, field: impl Into<String>) -> Self {
        Self {
            kind,
            field: field.into(),
            index: None,
        }
    }

    fn at_index(mut self, index: usize) -> Self {
        self.index = Some(index);
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceRequirement {
    pub id: String,
    pub contract_version: ContractVersion,
}

impl ServiceRequirement {
    /// A host service is compatible when it provides the same major version
    /// and at least the declared minor version.
    pub const fn is_compatible_with(&self, host: ContractVersion) -> bool {
        self.contract_version.is_compatible_with(host)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppLifecycleExtension {
    pub schema_version: u64,
    pub minimum_nagi_version: Option<AppVersion>,
    /// Capability requests in the base manifest are required. These are
    /// optional requests and never represent granted authority.
    pub optional_capabilities: Vec<String>,
    pub required_services: Vec<ServiceRequirement>,
    /// Namespaced metadata is preserved but does not affect runtime behavior.
    pub metadata: BTreeMap<String, Value>,
}

impl AppLifecycleExtension {
    pub fn supports_runtime(&self, runtime_version: &AppVersion) -> bool {
        self.minimum_nagi_version
            .as_ref()
            .is_none_or(|minimum| runtime_version >= minimum)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppPackageManifest<'a> {
    /// The canonical, versioned base contract owned by `nagi-sdk`.
    pub manifest: AppManifestContract<'a>,
    /// Parsed application version, suitable for deterministic ordering.
    pub version: AppVersion,
    /// Additional lifecycle/package declarations in the namespaced extension.
    pub lifecycle: AppLifecycleExtension,
}

/// Validate a typed App SDK manifest together with this workstream's
/// namespaced extension. `manifest` must be the typed base parsed from the
/// same JSON document; the extension is read from that document here.
pub fn parse_app_package_manifest<'a>(
    document: &Value,
    manifest: AppManifestContract<'a>,
) -> Result<AppPackageManifest<'a>, ManifestValidationError> {
    manifest.validate().map_err(|_| {
        ManifestValidationError::new(ManifestValidationErrorKind::InvalidBaseManifest, "manifest")
    })?;
    let version = AppVersion::parse(manifest.identity.version()).map_err(|_| {
        ManifestValidationError::new(ManifestValidationErrorKind::InvalidVersion, "version")
    })?;

    let root = document.as_object().ok_or_else(|| {
        ManifestValidationError::new(ManifestValidationErrorKind::InvalidDocument, "manifest")
    })?;
    let extensions = match root.get("extensions") {
        Some(value) => value.as_object().ok_or_else(|| {
            ManifestValidationError::new(ManifestValidationErrorKind::InvalidType, "extensions")
        })?,
        None => {
            return Ok(AppPackageManifest {
                lifecycle: AppLifecycleExtension::default(),
                manifest,
                version,
            });
        }
    };

    let Some(extension) = extensions.get(APP_LIFECYCLE_EXTENSION_ID) else {
        return Ok(AppPackageManifest {
            lifecycle: AppLifecycleExtension::default(),
            manifest,
            version,
        });
    };
    let extension = extension.as_object().ok_or_else(|| {
        ManifestValidationError::new(
            ManifestValidationErrorKind::InvalidType,
            format!("extensions.{APP_LIFECYCLE_EXTENSION_ID}"),
        )
    })?;
    validate_extension_fields(extension)?;

    let schema_version = unsigned_integer(extension, "schemaVersion")?.ok_or_else(|| {
        ManifestValidationError::new(ManifestValidationErrorKind::MissingField, "schemaVersion")
    })?;
    if schema_version != APP_LIFECYCLE_EXTENSION_SCHEMA_VERSION {
        return Err(ManifestValidationError::new(
            ManifestValidationErrorKind::UnsupportedVersion,
            "schemaVersion",
        ));
    }

    let minimum_nagi_version = match extension.get("minimumNagiVersion") {
        None => None,
        Some(value) => {
            let value = value.as_str().ok_or_else(|| {
                ManifestValidationError::new(
                    ManifestValidationErrorKind::InvalidType,
                    "minimumNagiVersion",
                )
            })?;
            Some(AppVersion::parse(value).map_err(|_| {
                ManifestValidationError::new(
                    ManifestValidationErrorKind::InvalidVersion,
                    "minimumNagiVersion",
                )
            })?)
        }
    };

    let optional_capabilities = parse_optional_capabilities(extension, &manifest)?;
    let required_services = parse_required_services(extension)?;
    let metadata = parse_metadata(extension)?;

    Ok(AppPackageManifest {
        manifest,
        version,
        lifecycle: AppLifecycleExtension {
            schema_version,
            minimum_nagi_version,
            optional_capabilities,
            required_services,
            metadata,
        },
    })
}

impl Default for AppLifecycleExtension {
    fn default() -> Self {
        Self {
            schema_version: APP_LIFECYCLE_EXTENSION_SCHEMA_VERSION,
            minimum_nagi_version: None,
            optional_capabilities: Vec::new(),
            required_services: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }
}

fn validate_extension_fields(
    extension: &Map<String, Value>,
) -> Result<(), ManifestValidationError> {
    const ALLOWED: &[&str] = &[
        "schemaVersion",
        "minimumNagiVersion",
        "optionalCapabilities",
        "requiredServices",
        "metadata",
    ];
    if !extension.contains_key("schemaVersion") {
        return Err(ManifestValidationError::new(
            ManifestValidationErrorKind::MissingField,
            "schemaVersion",
        ));
    }
    if let Some(field) = extension
        .keys()
        .find(|field| !ALLOWED.contains(&field.as_str()))
    {
        return Err(ManifestValidationError::new(
            ManifestValidationErrorKind::UnknownField,
            format!("extensions.{APP_LIFECYCLE_EXTENSION_ID}.{field}"),
        ));
    }
    Ok(())
}

fn parse_optional_capabilities(
    extension: &Map<String, Value>,
    manifest: &AppManifestContract<'_>,
) -> Result<Vec<String>, ManifestValidationError> {
    let Some(value) = extension.get("optionalCapabilities") else {
        return Ok(Vec::new());
    };
    let entries = value.as_array().ok_or_else(|| {
        ManifestValidationError::new(
            ManifestValidationErrorKind::InvalidType,
            "optionalCapabilities",
        )
    })?;
    let required: BTreeSet<&str> = manifest
        .requested_capabilities
        .iter()
        .map(|request| request.id)
        .collect();
    let mut seen = BTreeSet::new();
    let mut capabilities = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let id = entry.as_str().ok_or_else(|| {
            ManifestValidationError::new(
                ManifestValidationErrorKind::InvalidType,
                "optionalCapabilities",
            )
            .at_index(index)
        })?;
        if !valid_capability_id(id) {
            return Err(ManifestValidationError::new(
                ManifestValidationErrorKind::InvalidValue,
                "optionalCapabilities",
            )
            .at_index(index));
        }
        if required.contains(id) {
            return Err(ManifestValidationError::new(
                ManifestValidationErrorKind::ContradictoryDeclaration,
                "optionalCapabilities",
            )
            .at_index(index));
        }
        if !seen.insert(id) {
            return Err(ManifestValidationError::new(
                ManifestValidationErrorKind::DuplicateDeclaration,
                "optionalCapabilities",
            )
            .at_index(index));
        }
        capabilities.push(id.to_owned());
    }
    Ok(capabilities)
}

fn parse_required_services(
    extension: &Map<String, Value>,
) -> Result<Vec<ServiceRequirement>, ManifestValidationError> {
    let Some(value) = extension.get("requiredServices") else {
        return Ok(Vec::new());
    };
    let entries = value.as_array().ok_or_else(|| {
        ManifestValidationError::new(ManifestValidationErrorKind::InvalidType, "requiredServices")
    })?;
    let mut seen = BTreeSet::new();
    let mut services = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let item = entry.as_object().ok_or_else(|| {
            ManifestValidationError::new(
                ManifestValidationErrorKind::InvalidType,
                "requiredServices",
            )
            .at_index(index)
        })?;
        validate_service_fields(item, index)?;
        let id = item["id"].as_str().ok_or_else(|| {
            ManifestValidationError::new(
                ManifestValidationErrorKind::InvalidType,
                "requiredServices.id",
            )
            .at_index(index)
        })?;
        if !valid_service_id(id) {
            return Err(ManifestValidationError::new(
                ManifestValidationErrorKind::InvalidValue,
                "requiredServices.id",
            )
            .at_index(index));
        }
        if !seen.insert(id) {
            return Err(ManifestValidationError::new(
                ManifestValidationErrorKind::DuplicateDeclaration,
                "requiredServices.id",
            )
            .at_index(index));
        }
        let version = item["contractVersion"].as_object().ok_or_else(|| {
            ManifestValidationError::new(
                ManifestValidationErrorKind::InvalidType,
                "requiredServices.contractVersion",
            )
            .at_index(index)
        })?;
        validate_contract_version_fields(version, index)?;
        let major = version["major"]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok());
        let minor = version["minor"]
            .as_u64()
            .and_then(|value| u16::try_from(value).ok());
        let (Some(major), Some(minor)) = (major, minor) else {
            return Err(ManifestValidationError::new(
                ManifestValidationErrorKind::InvalidValue,
                "requiredServices.contractVersion",
            )
            .at_index(index));
        };
        services.push(ServiceRequirement {
            id: id.to_owned(),
            contract_version: ContractVersion { major, minor },
        });
    }
    Ok(services)
}

fn validate_service_fields(
    item: &Map<String, Value>,
    index: usize,
) -> Result<(), ManifestValidationError> {
    if !item.contains_key("id") || !item.contains_key("contractVersion") {
        return Err(ManifestValidationError::new(
            ManifestValidationErrorKind::MissingField,
            "requiredServices",
        )
        .at_index(index));
    }
    if let Some(field) = item
        .keys()
        .find(|field| !["id", "contractVersion"].contains(&field.as_str()))
    {
        return Err(ManifestValidationError::new(
            ManifestValidationErrorKind::UnknownField,
            format!("requiredServices.{field}"),
        )
        .at_index(index));
    }
    Ok(())
}

fn validate_contract_version_fields(
    version: &Map<String, Value>,
    index: usize,
) -> Result<(), ManifestValidationError> {
    if !version.contains_key("major") || !version.contains_key("minor") {
        return Err(ManifestValidationError::new(
            ManifestValidationErrorKind::MissingField,
            "requiredServices.contractVersion",
        )
        .at_index(index));
    }
    if let Some(field) = version
        .keys()
        .find(|field| !["major", "minor"].contains(&field.as_str()))
    {
        return Err(ManifestValidationError::new(
            ManifestValidationErrorKind::UnknownField,
            format!("requiredServices.contractVersion.{field}"),
        )
        .at_index(index));
    }
    Ok(())
}

fn parse_metadata(
    extension: &Map<String, Value>,
) -> Result<BTreeMap<String, Value>, ManifestValidationError> {
    let Some(value) = extension.get("metadata") else {
        return Ok(BTreeMap::new());
    };
    let metadata = value.as_object().ok_or_else(|| {
        ManifestValidationError::new(ManifestValidationErrorKind::InvalidType, "metadata")
    })?;
    for key in metadata.keys() {
        if !valid_namespaced_id(key) {
            return Err(ManifestValidationError::new(
                ManifestValidationErrorKind::InvalidValue,
                format!("metadata.{key}"),
            ));
        }
    }
    Ok(metadata
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

fn unsigned_integer(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<u64>, ManifestValidationError> {
    object.get(field).map_or(Ok(None), |value| {
        value.as_u64().map(Some).ok_or_else(|| {
            ManifestValidationError::new(ManifestValidationErrorKind::InvalidType, field)
        })
    })
}

fn valid_capability_id(value: &str) -> bool {
    // The SDK currently exposes capability declarations but keeps its ID
    // validator private. Mirror that syntax only for the optional extension;
    // converge on a shared public validator when the SDK owner exposes one.
    valid_namespaced_id_with_leading(value, |byte| byte.is_ascii_lowercase())
}

fn valid_service_id(value: &str) -> bool {
    valid_namespaced_id_with_leading(value, |byte| byte.is_ascii_lowercase())
}

fn valid_namespaced_id(value: &str) -> bool {
    valid_namespaced_id_with_leading(value, |byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit()
    })
}

fn valid_namespaced_id_with_leading(value: &str, valid_first_byte: impl Fn(u8) -> bool) -> bool {
    if value.is_empty() || value.len() > 128 {
        return false;
    }
    let mut segments = value.split('.');
    let Some(first) = segments.next() else {
        return false;
    };
    valid_segment(first, &valid_first_byte)
        && segments
            .next()
            .is_some_and(|segment| valid_segment(segment, &valid_first_byte))
        && segments.all(|segment| valid_segment(segment, &valid_first_byte))
}

fn valid_segment(segment: &str, valid_first_byte: &impl Fn(u8) -> bool) -> bool {
    let mut bytes = segment.bytes();
    bytes.next().is_some_and(valid_first_byte)
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
