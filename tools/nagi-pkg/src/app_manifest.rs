use std::fs::File;
use std::io::{Read, Take};
use std::path::Path;

use nagi_sdk::app_contract::AppStateVersion;
use nagi_sdk::app_contract::{
    is_valid_app_version, valid_locale_tag, AppEntrypoint, AppIdentity, AppManifestContract,
    AppOrigin, BackgroundServiceDeclaration, CapabilityRequest, ContractVersion, DisplayName,
    EntrypointKind, IntentDeclaration, ResourceReference, StateCompatibility,
    APP_MANIFEST_SCHEMA_VERSION, SDK_CONTRACT_VERSION,
};
use nagi_sdk::AppId;
use serde_json::{Map, Value};

pub const MAX_APP_MANIFEST_BYTES: u64 = 1_048_576;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedManifest {
    pub app_id: AppId,
    pub identifier: String,
    pub version: String,
    pub origin: AppOrigin,
    pub publisher_id: Option<String>,
    pub sdk_contract_version: ContractVersion,
}

pub fn load_manifest(path: &Path) -> Result<ValidatedManifest, String> {
    let file = File::open(path).map_err(|error| format!("cannot open manifest: {error}"))?;
    let mut limited: Take<File> = file.take(MAX_APP_MANIFEST_BYTES + 1);
    let mut bytes = Vec::new();
    limited
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read manifest: {error}"))?;
    if bytes.len() as u64 > MAX_APP_MANIFEST_BYTES {
        return Err(format!(
            "manifest exceeds the {} byte limit",
            MAX_APP_MANIFEST_BYTES
        ));
    }
    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("manifest is not valid JSON: {error}"))?;
    validate_manifest_document(&document)
}

fn validate_manifest_document(document: &Value) -> Result<ValidatedManifest, String> {
    let root = object(document, "manifest")?;
    ensure_fields(
        root,
        "manifest",
        &[
            "schemaVersion",
            "sdkContractVersion",
            "id",
            "version",
            "origin",
            "publisherId",
            "displayName",
            "supportedLocales",
            "icon",
            "entrypoint",
            "resources",
            "intents",
            "requestedCapabilities",
            "backgroundServices",
            "stateCompatibility",
            "extensions",
        ],
        &[
            "schemaVersion",
            "sdkContractVersion",
            "id",
            "version",
            "origin",
            "displayName",
            "supportedLocales",
            "entrypoint",
            "resources",
            "intents",
            "requestedCapabilities",
            "stateCompatibility",
        ],
    )?;

    let schema_version = integer(root, "schemaVersion", "manifest")?;
    if schema_version != APP_MANIFEST_SCHEMA_VERSION as u64 {
        return Err("manifest.schemaVersion is unsupported".to_owned());
    }
    let version_object = object(
        required(root, "sdkContractVersion", "manifest")?,
        "sdkContractVersion",
    )?;
    ensure_fields(
        version_object,
        "sdkContractVersion",
        &["major", "minor"],
        &["major", "minor"],
    )?;
    let sdk_contract_version = ContractVersion {
        major: bounded_u16(version_object, "major", "sdkContractVersion")?,
        minor: bounded_u16(version_object, "minor", "sdkContractVersion")?,
    };
    if !sdk_contract_version.is_compatible_with(SDK_CONTRACT_VERSION) {
        return Err(format!(
            "manifest requests incompatible SDK contract {}.{}, host provides {}.{}",
            sdk_contract_version.major,
            sdk_contract_version.minor,
            SDK_CONTRACT_VERSION.major,
            SDK_CONTRACT_VERSION.minor
        ));
    }

    let identifier = string(root, "id", "manifest")?;
    let app_version = string(root, "version", "manifest")?;
    if !is_valid_app_version(app_version) {
        return Err("manifest.version must be a semantic version".to_owned());
    }
    let origin = match string(root, "origin", "manifest")? {
        "first-party" => AppOrigin::FirstParty,
        "third-party" => AppOrigin::ThirdParty,
        _ => return Err("manifest.origin must be first-party or third-party".to_owned()),
    };
    let publisher_id = optional_string(root, "publisherId", "manifest")?;
    if origin == AppOrigin::FirstParty && publisher_id.is_none() {
        return Err("first-party manifests must declare publisherId".to_owned());
    }
    let identity = AppIdentity::new(identifier, app_version, origin, publisher_id)
        .map_err(|error| format!("manifest identity is invalid: {:?}", error.code))?;

    let display_object = object(required(root, "displayName", "manifest")?, "displayName")?;
    ensure_fields(
        display_object,
        "displayName",
        &["en-US", "ja-JP"],
        &["en-US"],
    )?;
    let display_name = DisplayName {
        en_us: string(display_object, "en-US", "displayName")?,
        ja_jp: optional_string(display_object, "ja-JP", "displayName")?,
    };

    let locales: Vec<&str> = array(root, "supportedLocales", "manifest")?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_str()
                .ok_or_else(|| format!("manifest.supportedLocales[{index}] must be a string"))
        })
        .collect::<Result<_, _>>()?;
    if locales.iter().any(|locale| !valid_locale_tag(locale)) {
        return Err("manifest.supportedLocales contains an invalid locale tag".to_owned());
    }
    let icon = optional_string(root, "icon", "manifest")?;

    let entrypoint_object = object(required(root, "entrypoint", "manifest")?, "entrypoint")?;
    ensure_fields(
        entrypoint_object,
        "entrypoint",
        &["kind", "target"],
        &["kind", "target"],
    )?;
    let entrypoint = AppEntrypoint {
        kind: match string(entrypoint_object, "kind", "entrypoint")? {
            "native" => EntrypointKind::Native,
            "portable" => EntrypointKind::Portable,
            _ => return Err("entrypoint.kind must be native or portable".to_owned()),
        },
        target: string(entrypoint_object, "target", "entrypoint")?,
    };

    let resources = array(root, "resources", "manifest")?
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let context = format!("resources[{index}]");
            let resource = object(item, &context)?;
            ensure_fields(
                resource,
                &context,
                &["id", "uri", "mediaType"],
                &["id", "uri"],
            )?;
            Ok(ResourceReference {
                id: string(resource, "id", &context)?,
                uri: string(resource, "uri", &context)?,
                media_type: optional_string(resource, "mediaType", &context)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let intents = array(root, "intents", "manifest")?
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let context = format!("intents[{index}]");
            let intent = object(item, &context)?;
            ensure_fields(
                intent,
                &context,
                &["id", "version", "payloadType", "routeId"],
                &["id", "version", "payloadType"],
            )?;
            Ok(IntentDeclaration {
                id: string(intent, "id", &context)?,
                version: bounded_u16(intent, "version", &context)?,
                payload_type: string(intent, "payloadType", &context)?,
                route_id: optional_string(intent, "routeId", &context)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let requested_capabilities = array(root, "requestedCapabilities", "manifest")?
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let context = format!("requestedCapabilities[{index}]");
            let request = object(item, &context)?;
            ensure_fields(request, &context, &["id", "purposeKey"], &["id"])?;
            Ok(CapabilityRequest {
                id: string(request, "id", &context)?,
                purpose_key: optional_string(request, "purposeKey", &context)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let background_services = optional_array(root, "backgroundServices", "manifest")?
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let context = format!("backgroundServices[{index}]");
            let service = object(item, &context)?;
            ensure_fields(
                service,
                &context,
                &["id", "entrypoint", "activation"],
                &["id", "entrypoint", "activation"],
            )?;
            Ok(BackgroundServiceDeclaration {
                id: string(service, "id", &context)?,
                entrypoint: string(service, "entrypoint", &context)?,
                activation: string(service, "activation", &context)?,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let state_object = object(
        required(root, "stateCompatibility", "manifest")?,
        "stateCompatibility",
    )?;
    ensure_fields(
        state_object,
        "stateCompatibility",
        &["currentVersion", "minimumReadableVersion"],
        &["currentVersion", "minimumReadableVersion"],
    )?;
    let state = StateCompatibility {
        current: AppStateVersion(bounded_u32(
            state_object,
            "currentVersion",
            "stateCompatibility",
        )?),
        minimum_readable: AppStateVersion(bounded_u32(
            state_object,
            "minimumReadableVersion",
            "stateCompatibility",
        )?),
    };

    if let Some(extensions) = root.get("extensions") {
        let extensions = object(extensions, "extensions")?;
        for key in extensions.keys() {
            if !is_namespaced_extension(key) {
                return Err(format!("extensions key `{key}` must be namespaced"));
            }
        }
    }

    let contract = AppManifestContract {
        schema_version: schema_version as u16,
        sdk_contract_version,
        identity,
        display_name,
        supported_locales: &locales,
        icon,
        entrypoint,
        resources: &resources,
        intents: &intents,
        requested_capabilities: &requested_capabilities,
        background_services: &background_services,
        state,
    };
    contract
        .validate()
        .map_err(|error| format!("manifest contract validation failed: {:?}", error.code))?;

    Ok(ValidatedManifest {
        app_id: identity.app_id(),
        identifier: identifier.to_owned(),
        version: app_version.to_owned(),
        origin,
        publisher_id: publisher_id.map(str::to_owned),
        sdk_contract_version,
    })
}

fn object<'a>(value: &'a Value, context: &str) -> Result<&'a Map<String, Value>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{context} must be an object"))
}

fn ensure_fields(
    object: &Map<String, Value>,
    context: &str,
    allowed: &[&str],
    required_fields: &[&str],
) -> Result<(), String> {
    for key in required_fields {
        if !object.contains_key(*key) {
            return Err(format!("{context}.{key} is required"));
        }
    }
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("{context}.{key} is not recognized"));
    }
    Ok(())
}

fn required<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<&'a Value, String> {
    object
        .get(key)
        .ok_or_else(|| format!("{context}.{key} is required"))
}

fn string<'a>(object: &'a Map<String, Value>, key: &str, context: &str) -> Result<&'a str, String> {
    required(object, key, context)?
        .as_str()
        .ok_or_else(|| format!("{context}.{key} must be a string"))
}

fn optional_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<Option<&'a str>, String> {
    object
        .get(key)
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| format!("{context}.{key} must be a string"))
        })
        .transpose()
}

fn integer(object: &Map<String, Value>, key: &str, context: &str) -> Result<u64, String> {
    required(object, key, context)?
        .as_u64()
        .ok_or_else(|| format!("{context}.{key} must be a non-negative integer"))
}

fn bounded_u16(object: &Map<String, Value>, key: &str, context: &str) -> Result<u16, String> {
    u16::try_from(integer(object, key, context)?)
        .map_err(|_| format!("{context}.{key} exceeds the u16 range"))
}

fn bounded_u32(object: &Map<String, Value>, key: &str, context: &str) -> Result<u32, String> {
    u32::try_from(integer(object, key, context)?)
        .map_err(|_| format!("{context}.{key} exceeds the u32 range"))
}

fn array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<&'a [Value], String> {
    required(object, key, context)?
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{context}.{key} must be an array"))
}

fn optional_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    context: &str,
) -> Result<&'a [Value], String> {
    match object.get(key) {
        Some(value) => value
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| format!("{context}.{key} must be an array")),
        None => Ok(&[]),
    }
}

fn is_namespaced_extension(value: &str) -> bool {
    let mut parts = value.split('.');
    parts.clone().count() >= 2
        && parts.all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::{load_manifest, validate_manifest_document, MAX_APP_MANIFEST_BYTES};
    use serde_json::Value;
    use std::fs;
    use std::path::PathBuf;

    fn fixture(name: &str) -> Value {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../sdk/rust/fixtures/manifests")
            .join(name);
        serde_json::from_slice(&fs::read(path).expect("manifest fixture"))
            .expect("valid fixture JSON")
    }

    #[test]
    fn first_party_manifest_fixtures_load_through_public_sdk_contract() {
        for name in ["files.json", "notes.json", "search.json", "activity.json"] {
            let manifest = validate_manifest_document(&fixture(name))
                .unwrap_or_else(|error| panic!("{name} should validate: {error}"));
            assert!(manifest.identifier.starts_with("com.nagi.sdk-fixture."));
            assert_eq!(manifest.version, "0.1.0");
            assert_eq!(manifest.sdk_contract_version.major, 1);
            assert_ne!(manifest.app_id.0, 0);
        }
    }

    #[test]
    fn invalid_fields_and_compatibility_versions_are_rejected() {
        let mut malformed = fixture("notes.json");
        malformed["entrypoint"]["target"] = Value::String("../outside.napp".to_owned());
        assert!(validate_manifest_document(&malformed).is_err());

        let mut incompatible = fixture("notes.json");
        incompatible["sdkContractVersion"]["major"] = Value::from(2);
        assert!(validate_manifest_document(&incompatible).is_err());

        let mut unknown_field = fixture("notes.json");
        unknown_field["unscopedVendorData"] = Value::Bool(true);
        assert!(validate_manifest_document(&unknown_field).is_err());

        let mut malformed_route = fixture("files.json");
        malformed_route["intents"][0]["routeId"] = Value::String("../hidden".to_owned());
        assert!(validate_manifest_document(&malformed_route).is_err());

        let mut malformed_resource = fixture("files.json");
        malformed_resource["resources"][0]["uri"] =
            Value::String("appres://icons/../private.svg".to_owned());
        assert!(validate_manifest_document(&malformed_resource).is_err());

        for uri in [
            "appres://icons/a b.svg",
            "appres://icons/a\u{0000}b.svg",
            "appres://icons/日本語.svg",
            "appres://icons//notes.svg",
        ] {
            let mut malformed_resource = fixture("files.json");
            malformed_resource["resources"][0]["uri"] = Value::String(uri.to_owned());
            assert!(
                validate_manifest_document(&malformed_resource).is_err(),
                "{uri:?}"
            );
        }

        for target in ["bin/a b.napp", "bin/日本語.napp", "bin//notes.napp"] {
            let mut malformed_entrypoint = fixture("files.json");
            malformed_entrypoint["entrypoint"]["target"] = Value::String(target.to_owned());
            assert!(
                validate_manifest_document(&malformed_entrypoint).is_err(),
                "{target:?}"
            );
        }

        let mut incompatible_state = fixture("notes.json");
        incompatible_state["stateCompatibility"]["minimumReadableVersion"] = Value::from(3);
        assert!(validate_manifest_document(&incompatible_state).is_err());

        let mut duplicate_intent = fixture("notes.json");
        let intent = duplicate_intent["intents"][0].clone();
        duplicate_intent["intents"]
            .as_array_mut()
            .unwrap()
            .push(intent);
        assert!(validate_manifest_document(&duplicate_intent).is_err());

        let mut unsupported_version = fixture("notes.json");
        unsupported_version["version"] = Value::String("1.0.0-01".to_owned());
        assert!(validate_manifest_document(&unsupported_version).is_err());

        let mut missing_japanese_name = fixture("notes.json");
        missing_japanese_name["displayName"]
            .as_object_mut()
            .unwrap()
            .remove("ja-JP");
        assert!(validate_manifest_document(&missing_japanese_name).is_err());

        let mut malformed_service = fixture("notes.json");
        malformed_service["backgroundServices"][0]["activation"] =
            Value::String("elevated".to_owned());
        assert!(validate_manifest_document(&malformed_service).is_err());
    }

    #[test]
    fn future_background_services_are_optional_metadata() {
        let mut document = fixture("search.json");
        document
            .as_object_mut()
            .unwrap()
            .remove("backgroundServices");
        assert!(validate_manifest_document(&document).is_ok());
    }

    #[test]
    fn size_limit_is_a_bounded_manifest_contract() {
        assert_eq!(MAX_APP_MANIFEST_BYTES, 1_048_576);
    }

    #[test]
    fn loader_rejects_malformed_json_and_oversized_input() {
        let malformed_path = temporary_path("malformed.json");
        fs::write(&malformed_path, b"{").unwrap();
        assert!(load_manifest(&malformed_path)
            .unwrap_err()
            .contains("not valid JSON"));
        fs::remove_file(&malformed_path).unwrap();

        let oversized_path = temporary_path("oversized.json");
        fs::write(
            &oversized_path,
            vec![b' '; MAX_APP_MANIFEST_BYTES as usize + 1],
        )
        .unwrap();
        assert!(load_manifest(&oversized_path)
            .unwrap_err()
            .contains("exceeds the 1048576 byte limit"));
        fs::remove_file(&oversized_path).unwrap();
    }

    fn temporary_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("nagi-app-sdk-{}-{name}", std::process::id()))
    }
}
