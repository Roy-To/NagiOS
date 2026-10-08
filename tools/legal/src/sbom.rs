use crate::license::{is_spdx_expression, SPDX_LICENSE_LIST_VERSION};
use crate::model::{ComponentRecord, Inventory};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

const SPDX_VERSION: &str = "SPDX-2.3";

pub fn build_spdx_document(inventory: &Inventory, created: &str) -> Result<Value, Vec<String>> {
    let mut errors = Vec::new();
    if !valid_timestamp(created) {
        errors.push("creationInfo.created must be an RFC 3339 UTC timestamp".to_owned());
    }
    let mut components = inventory.components.clone();
    components.sort_by(|left, right| {
        (&left.name, &left.version, &left.component_id).cmp(&(
            &right.name,
            &right.version,
            &right.component_id,
        ))
    });

    let mut package_ids = std::collections::BTreeMap::new();
    for component in &components {
        package_ids.insert(
            component.component_id.clone(),
            spdx_package_id(&component.component_id),
        );
    }
    let mut packages = vec![project_package(inventory)];
    packages.extend(components.iter().map(component_package));

    let mut described = vec![json!("SPDXRef-NagiOS")];
    for component in &components {
        if let Some(id) = package_ids.get(&component.component_id) {
            described.push(json!(id));
        }
    }
    described.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
    described.dedup();

    let mut relationships = vec![json!({
        "spdxElementId": "SPDXRef-DOCUMENT",
        "relationshipType": "DESCRIBES",
        "relatedSpdxElement": "SPDXRef-NagiOS"
    })];
    for component in &components {
        let Some(parent_id) = package_ids.get(&component.component_id) else {
            continue;
        };
        for child in &component.dependencies {
            if let Some(child_id) = package_ids.get(child) {
                relationships.push(json!({
                    "spdxElementId": parent_id,
                    "relationshipType": "DEPENDS_ON",
                    "relatedSpdxElement": child_id
                }));
            }
        }
    }
    relationships.sort_by(|left, right| {
        (
            left.get("spdxElementId").and_then(Value::as_str),
            left.get("relationshipType").and_then(Value::as_str),
            left.get("relatedSpdxElement").and_then(Value::as_str),
        )
            .cmp(&(
                right.get("spdxElementId").and_then(Value::as_str),
                right.get("relationshipType").and_then(Value::as_str),
                right.get("relatedSpdxElement").and_then(Value::as_str),
            ))
    });
    relationships.dedup();

    let mut document = json!({
        "spdxVersion": SPDX_VERSION,
        "dataLicense": "CC0-1.0",
        "SPDXID": "SPDXRef-DOCUMENT",
        "name": format!("{} {} source dependency inventory", inventory.project.name, inventory.project.version),
        "creationInfo": {
            "creators": ["Tool: nagi-legal-0.1.0"],
            "licenseListVersion": SPDX_LICENSE_LIST_VERSION,
            "created": created
        },
        "documentDescribes": described,
        "packages": packages,
        "relationships": relationships
    });
    let mut namespace_material = document.clone();
    if let Some(creation_info) = namespace_material
        .get_mut("creationInfo")
        .and_then(Value::as_object_mut)
    {
        creation_info.remove("created");
    }
    let canonical_material = serde_json::to_vec(&namespace_material)
        .map_err(|_| vec!["cannot serialize SPDX namespace material".to_owned()])?;
    let namespace_digest = sha256_hex(&canonical_material);
    document["documentNamespace"] = json!(format!(
        "https://spdx.org/spdxdocs/nagi-os-{namespace_digest}"
    ));
    if let Err(mut schema_errors) = validate_spdx_document(&document) {
        errors.append(&mut schema_errors);
    }
    if errors.is_empty() {
        Ok(document)
    } else {
        errors.sort();
        errors.dedup();
        Err(errors)
    }
}

pub fn build_spdx_now(inventory: &Inventory) -> Result<Value, Vec<String>> {
    let epoch = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|time| time.as_secs())
        })
        .ok_or_else(|| vec!["system clock is before the Unix epoch".to_owned()])?;
    build_spdx_document(inventory, &format_epoch(epoch))
}

pub fn validate_spdx_document(document: &Value) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    for field in [
        "spdxVersion",
        "dataLicense",
        "SPDXID",
        "name",
        "documentNamespace",
    ] {
        if document.get(field).and_then(Value::as_str).is_none() {
            errors.push(format!("SPDX document is missing string field {field}"));
        }
    }
    if document.get("spdxVersion").and_then(Value::as_str) != Some(SPDX_VERSION) {
        errors.push("SPDX document spdxVersion must be SPDX-2.3".to_owned());
    }
    if document.get("dataLicense").and_then(Value::as_str) != Some("CC0-1.0") {
        errors.push("SPDX document dataLicense must be CC0-1.0".to_owned());
    }
    if document.get("SPDXID").and_then(Value::as_str) != Some("SPDXRef-DOCUMENT") {
        errors.push("SPDX document SPDXID must be SPDXRef-DOCUMENT".to_owned());
    }
    let namespace = document
        .get("documentNamespace")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !namespace.starts_with("https://spdx.org/spdxdocs/") {
        errors
            .push("SPDX document namespace must use the SPDX document namespace prefix".to_owned());
    }
    let creation = document.get("creationInfo");
    if creation.is_none() {
        errors.push("SPDX document is missing creationInfo".to_owned());
    } else {
        let creation = creation.unwrap_or(&Value::Null);
        if creation
            .get("created")
            .and_then(Value::as_str)
            .is_none_or(|value| !valid_timestamp(value))
        {
            errors.push("creationInfo.created is missing or invalid".to_owned());
        }
        if creation
            .get("creators")
            .and_then(Value::as_array)
            .is_none_or(|creators| {
                creators.is_empty() || creators.iter().any(|value| value.as_str().is_none())
            })
        {
            errors.push("creationInfo.creators must be a non-empty string array".to_owned());
        }
        if creation.get("licenseListVersion").and_then(Value::as_str)
            != Some(SPDX_LICENSE_LIST_VERSION)
        {
            errors.push(
                "creationInfo.licenseListVersion must match the bundled SPDX identifier list"
                    .to_owned(),
            );
        }
    }
    let packages = document.get("packages").and_then(Value::as_array);
    if packages.is_none() {
        errors.push("SPDX document packages must be an array".to_owned());
    }
    let mut package_ids = BTreeSet::new();
    if let Some(packages) = packages {
        for (index, package) in packages.iter().enumerate() {
            for field in [
                "name",
                "SPDXID",
                "downloadLocation",
                "licenseConcluded",
                "licenseDeclared",
                "copyrightText",
            ] {
                if package.get(field).and_then(Value::as_str).is_none() {
                    errors.push(format!(
                        "SPDX package {index} is missing string field {field}"
                    ));
                }
            }
            if package
                .get("filesAnalyzed")
                .and_then(Value::as_bool)
                .is_none()
            {
                errors.push(format!(
                    "SPDX package {index} is missing boolean filesAnalyzed"
                ));
            }
            if let Some(id) = package.get("SPDXID").and_then(Value::as_str) {
                if !valid_spdx_identifier(id) {
                    errors.push(format!("SPDX package {index} has an invalid SPDXID"));
                }
                if !package_ids.insert(id.to_owned()) {
                    errors.push(format!("duplicate SPDX package ID {id}"));
                }
            }
            if package
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(str::is_empty)
            {
                errors.push(format!("SPDX package {index} has an empty name"));
            }
            for field in ["licenseDeclared", "licenseConcluded"] {
                if let Some(value) = package.get(field).and_then(Value::as_str) {
                    if value != "NOASSERTION" && !is_spdx_expression(value) {
                        errors.push(format!("SPDX package {index} has invalid {field}"));
                    }
                }
            }
        }
    }
    let document_id = document
        .get("SPDXID")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let described = document.get("documentDescribes").and_then(Value::as_array);
    if described.is_none_or(|values| values.is_empty()) {
        errors.push("documentDescribes must be a non-empty array".to_owned());
    } else if let Some(described) = described {
        for value in described {
            let Some(id) = value.as_str() else {
                errors.push("documentDescribes entries must be SPDX identifiers".to_owned());
                continue;
            };
            if id != "SPDXRef-NagiOS" && !package_ids.contains(id) {
                errors.push(format!("documentDescribes refers to absent package {id}"));
            }
        }
    }
    let relationships = document.get("relationships").and_then(Value::as_array);
    if relationships.is_none() {
        errors.push("SPDX document relationships must be an array".to_owned());
    } else if let Some(relationships) = relationships {
        for relationship in relationships {
            let from = relationship
                .get("spdxElementId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let to = relationship
                .get("relatedSpdxElement")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let relationship_type = relationship
                .get("relationshipType")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if !matches!(relationship_type, "DESCRIBES" | "DEPENDS_ON") {
                errors.push("SPDX relationship has an unsupported relationshipType".to_owned());
            }
            if (from != document_id && !package_ids.contains(from))
                || (to != document_id && !package_ids.contains(to))
            {
                errors.push("SPDX relationship refers to an absent element".to_owned());
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        errors.sort();
        errors.dedup();
        Err(errors)
    }
}

fn valid_spdx_identifier(value: &str) -> bool {
    value.strip_prefix("SPDXRef-").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    })
}

fn project_package(inventory: &Inventory) -> Value {
    let mut package = json!({
        "name": inventory.project.name,
        "SPDXID": "SPDXRef-NagiOS",
        "versionInfo": inventory.project.version,
        "downloadLocation": "NOASSERTION",
        "filesAnalyzed": false,
        "licenseConcluded": "NOASSERTION",
        "licenseDeclared": inventory.project.declared_license_expression.as_deref().unwrap_or("NOASSERTION"),
        "copyrightText": "NOASSERTION"
    });
    if let Some(revision) = &inventory.repository_revision {
        package["sourceInfo"] = json!(format!("Repository revision: {revision}"));
    }
    package
}

fn component_package(component: &ComponentRecord) -> Value {
    let license = if component.license_status == "declared"
        && component
            .declared_license_expression
            .as_deref()
            .is_some_and(is_spdx_expression)
    {
        component
            .declared_license_expression
            .as_deref()
            .unwrap_or("NOASSERTION")
    } else {
        "NOASSERTION"
    };
    let mut package = json!({
        "name": component.name,
        "SPDXID": spdx_package_id(&component.component_id),
        "downloadLocation": "NOASSERTION",
        "filesAnalyzed": false,
        "licenseConcluded": "NOASSERTION",
        "licenseDeclared": license,
        "copyrightText": "NOASSERTION"
    });
    if let Some(version) = &component.version {
        package["versionInfo"] = json!(version);
    } else if let Some(revision) = &component.revision {
        package["versionInfo"] = json!(revision);
    }
    if let Some(checksum) = component
        .checksum
        .as_deref()
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        package["checksums"] = json!([{"algorithm": "SHA256", "checksumValue": checksum}]);
    }
    if let Some(purl) = &component.purl {
        package["externalRefs"] = json!([{
            "referenceCategory": "PACKAGE-MANAGER",
            "referenceType": "purl",
            "referenceLocator": purl
        }]);
    }
    let mut source_info = Vec::new();
    if let Some(origin) = &component.origin {
        source_info.push(format!("Origin: {origin}"));
    }
    if let Some(revision) = &component.revision {
        source_info.push(format!("Revision: {revision}"));
    }
    if let Some(path) = &component.local_path {
        source_info.push(format!("Repository path: {path}"));
    }
    if !component.evidence.is_empty() {
        let sources = component
            .evidence
            .iter()
            .map(|evidence| evidence.source.as_str())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        source_info.push(format!("Evidence: {}", sources.join(", ")));
    }
    if !source_info.is_empty() {
        package["sourceInfo"] = json!(source_info.join("; "));
    }
    package
}

fn spdx_package_id(identity: &str) -> String {
    let digest = sha256_hex(identity.as_bytes());
    format!("SPDXRef-Package-{}", &digest[..20])
}

fn sha256_hex(value: &[u8]) -> String {
    let digest = Sha256::digest(value);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn valid_timestamp(value: &str) -> bool {
    let bytes = value.as_bytes();
    if !(bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z'
        && bytes.iter().enumerate().all(|(index, byte)| {
            matches!(index, 4 | 7 | 10 | 13 | 16 | 19) || byte.is_ascii_digit()
        }))
    {
        return false;
    }
    let Ok(year) = value[0..4].parse::<u32>() else {
        return false;
    };
    let Ok(month) = value[5..7].parse::<u32>() else {
        return false;
    };
    let Ok(day) = value[8..10].parse::<u32>() else {
        return false;
    };
    let Ok(hour) = value[11..13].parse::<u32>() else {
        return false;
    };
    let Ok(minute) = value[14..16].parse::<u32>() else {
        return false;
    };
    let Ok(second) = value[17..19].parse::<u32>() else {
        return false;
    };
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap_year => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days_in_month).contains(&day) && year > 0 && hour < 24 && minute < 60 && second <= 60
}

fn format_epoch(epoch: u64) -> String {
    let seconds = (epoch % 60) as i64;
    let minutes_total = (epoch / 60) as i64;
    let minutes = minutes_total % 60;
    let hours_total = minutes_total / 60;
    let hours = hours_total % 24;
    let days = hours_total / 24;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::{format_epoch, valid_timestamp, validate_spdx_document};

    #[test]
    fn epoch_formatter_produces_utc_calendar_time() {
        assert_eq!(format_epoch(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_epoch(1_754_006_400), "2025-08-01T00:00:00Z");
    }

    #[test]
    fn timestamp_validator_checks_calendar_and_clock_ranges() {
        assert!(valid_timestamp("2024-02-29T23:59:59Z"));
        assert!(!valid_timestamp("2023-02-29T23:59:59Z"));
        assert!(!valid_timestamp("2026-99-99T99:99:99Z"));
        assert!(!valid_timestamp("2026-01-01T24:00:00Z"));
    }

    #[test]
    fn profile_validator_rejects_missing_required_fields() {
        let invalid = serde_json::json!({"spdxVersion": "SPDX-2.3"});
        assert!(validate_spdx_document(&invalid).is_err());
    }
}
