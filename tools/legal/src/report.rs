use crate::diff::missing_license_evidence;
use crate::model::{Inventory, InventoryDiff};
use std::collections::BTreeMap;

pub fn render_scan_report(inventory: &Inventory) -> String {
    let mut licenses = BTreeMap::<String, usize>::new();
    for component in &inventory.components {
        let label = component
            .declared_license_expression
            .clone()
            .unwrap_or_else(|| "UNKNOWN".to_owned());
        *licenses.entry(label).or_default() += 1;
    }
    let unknown = inventory
        .components
        .iter()
        .filter(|component| component.license_status != "declared")
        .count();
    let manual_review = inventory
        .components
        .iter()
        .filter(|component| component.review_status == "manual_review_required")
        .count()
        + inventory
            .model_assets
            .iter()
            .filter(|model| model.review_status == "manual_review_required")
            .count();
    let missing_license = inventory
        .components
        .iter()
        .filter(|component| missing_license_evidence(component))
        .count();
    let missing_notice = inventory
        .components
        .iter()
        .filter(|component| component.notice_files.is_empty())
        .count();
    let errors = inventory
        .findings
        .iter()
        .filter(|finding| finding.severity == "error")
        .count();

    let mut output = format!(
        "Nagi OS dependency and license scan\nProject: {} {}\nRepository revision: {}\nComponents: {}\nLicense expressions unknown, malformed, or conflicting: {}\nComponents without license expression/file evidence: {}\nComponents without located NOTICE reference: {}\nComponents requiring manual review: {}\nPlanned model records (not SBOM components): {}\nStructural check errors: {}\n\nLicenses seen:\n",
        inventory.project.name,
        inventory.project.version,
        inventory.repository_revision.as_deref().unwrap_or("unknown"),
        inventory.components.len(),
        unknown,
        missing_license,
        missing_notice,
        manual_review,
        inventory.model_assets.len(),
        errors
    );
    for (license, count) in licenses {
        output.push_str(&format!("  {license}: {count}\n"));
    }
    let mut finding_groups = BTreeMap::<(String, String, String), Vec<String>>::new();
    for finding in &inventory.findings {
        let key = (
            finding.severity.to_ascii_uppercase(),
            finding.code.clone(),
            finding.message.clone(),
        );
        let affected = finding
            .component_id
            .as_deref()
            .map(markdown_safe)
            .unwrap_or_else(|| "(project)".to_owned());
        finding_groups.entry(key).or_default().push(affected);
    }
    if !finding_groups.is_empty() {
        output.push_str("\nFindings:\n");
        for ((severity, code, message), mut affected) in finding_groups {
            affected.sort();
            affected.dedup();
            output.push_str(&format!(
                "  {severity} {code} ({} occurrence(s)): {}\n",
                affected.len(),
                markdown_safe(&message)
            ));
            if !affected.is_empty() {
                let shown = affected
                    .iter()
                    .take(3)
                    .map(|id| format!("`{id}`"))
                    .collect::<Vec<_>>();
                let remaining = affected.len().saturating_sub(shown.len());
                output.push_str(&format!(
                    "    affected: {}{}\n",
                    shown.join(", "),
                    if remaining > 0 {
                        format!("; +{remaining} more in JSON inventory")
                    } else {
                        String::new()
                    }
                ));
            }
        }
    }
    output
}

pub fn render_notice_candidate(inventory: &Inventory) -> String {
    let mut output = String::from(
        "# Nagi OS NOTICE candidate\n\nGenerated evidence report. This is not an authoritative legal determination. Do not redistribute it as a completed notice without human review. The tool records references and does not copy or rewrite third-party license text.\n\n",
    );
    output.push_str("## Generated project metadata\n\n");
    output.push_str(&format!(
        "- Project: {} {}\n- Repository revision: {}\n- Nagi OS license expression: {}\n\n",
        inventory.project.name,
        inventory.project.version,
        inventory
            .repository_revision
            .as_deref()
            .unwrap_or("unknown"),
        inventory
            .project
            .declared_license_expression
            .as_deref()
            .unwrap_or("unknown / not selected")
    ));

    output.push_str("## Detected source text and reference locations\n\n");
    for component in &inventory.components {
        output.push_str(&format!(
            "### {}{}\n\n",
            markdown_safe(&component.name),
            component
                .version
                .as_deref()
                .or(component.revision.as_deref())
                .map(|version| format!(" {version}"))
                .unwrap_or_default()
        ));
        output.push_str(&format!(
            "- Package identity: `{}`\n- Declared SPDX expression: {}\n- Raw license metadata when not an SPDX expression: {}\n- License file reference declared by package metadata: {}\n- License files located in the tracked repository: {}\n- NOTICE/copyright references: {}\n- Evidence sources: {}\n- Manual review required: {}\n\n",
            markdown_safe(&component.component_id),
            component
                .declared_license_expression
                .as_deref()
                .map(markdown_safe)
                .unwrap_or_else(|| "unknown".to_owned()),
                component
                .raw_license_metadata
                .as_deref()
                .filter(|raw| component.declared_license_expression.as_deref() != Some(*raw))
                .map(safe_metadata_value)
                .unwrap_or_else(|| "none".to_owned()),
            component
                .license_file_declared
                .as_deref()
                .map(markdown_safe)
                .unwrap_or_else(|| "none".to_owned()),
            render_references(&component.detected_license_files),
            render_references(&component.notice_files),
            render_references(
                &component
                    .evidence
                    .iter()
                    .map(|evidence| evidence.source.clone())
                    .collect::<Vec<_>>()
            ),
            if component.review_status == "manual_review_required" {
                "yes"
            } else {
                "review status not assessed as legal compliance"
            }
        ));
    }

    output.push_str("## Model terms and notice references\n\n");
    if inventory.model_assets.is_empty() {
        output.push_str("No model-license catalog records were found.\n\n");
    } else {
        for model in &inventory.model_assets {
            let notice_refs = model
                .notices
                .iter()
                .map(|notice| {
                    format!(
                        "{}: {} ({})",
                        safe_metadata_value(&notice.notice_id),
                        safe_metadata_value(&notice.reference),
                        if notice.required {
                            "required"
                        } else {
                            "optional"
                        }
                    )
                })
                .collect::<Vec<_>>();
            output.push_str(&format!(
                "- **{}** (`{}`): inclusion `{}`, artifact present `{}`; provider terms identifier `{}`; terms reference `{}`; acknowledgement required `{}`; notice references {}; SPDX expression `{}`; unclassified license metadata `{}`; manual review `{}`. Evidence: `{}`.\n",
                safe_metadata_value(&model.display_name),
                safe_metadata_value(&model.model_id),
                safe_metadata_value(&model.inclusion_status),
                model.artifact_present,
                model.opaque_license_identifier.as_deref().map(safe_metadata_value).unwrap_or_else(|| "unknown".to_owned()),
                model.terms_reference.as_deref().map(safe_metadata_value).unwrap_or_else(|| "unknown".to_owned()),
                model.acknowledgement_required.map(|value| value.to_string()).unwrap_or_else(|| "unknown".to_owned()),
                if notice_refs.is_empty() { "none".to_owned() } else { notice_refs.join(", ") },
                model.declared_license_expression.as_deref().map(markdown_safe).unwrap_or_else(|| "unknown".to_owned()),
                model.raw_license_metadata.as_deref().map(safe_metadata_value).unwrap_or_else(|| "none".to_owned()),
                if model.review_status == "manual_review_required" { "yes" } else { "not resolved" },
                safe_metadata_value(&model.evidence_source)
            ));
        }
        output.push('\n');
    }
    output.push_str("## Review notes\n\n");
    output.push_str("Unknown or conflicting license information remains unknown. A located filename or provider reference does not establish legal permission, attribution sufficiency, or binary redistribution readiness. Confirm upstream terms and ship required texts before redistribution.\n");
    output
}

pub fn render_diff(diff: &InventoryDiff) -> String {
    let mut output = format!(
        "Inventory comparison\nAdded: {}\nRemoved: {}\nVersion/revision changes: {}\nLicense metadata changes: {}\nNew missing-license conditions: {}\n",
        diff.added.len(),
        diff.removed.len(),
        diff.version_changes.len(),
        diff.license_changes.len(),
        diff.new_missing_license_conditions.len()
    );
    for change in &diff.version_changes {
        output.push_str(&format!(
            "  VERSION {}: {} -> {}\n",
            change.name, change.from, change.to
        ));
    }
    for change in &diff.license_changes {
        output.push_str(&format!(
            "  LICENSE {}: {} -> {}\n",
            change.name,
            change.from.as_deref().unwrap_or("unknown"),
            change.to.as_deref().unwrap_or("unknown")
        ));
    }
    for component in &diff.added {
        output.push_str(&format!(
            "  ADDED {} {}\n",
            component.name,
            component.version.as_deref().unwrap_or("unknown")
        ));
    }
    for component in &diff.removed {
        output.push_str(&format!(
            "  REMOVED {} {}\n",
            component.name,
            component.version.as_deref().unwrap_or("unknown")
        ));
    }
    for component in &diff.new_missing_license_conditions {
        output.push_str(&format!("  MISSING LICENSE EVIDENCE {}\n", component.name));
    }
    output
}

fn render_references(values: &[String]) -> String {
    if values.is_empty() {
        "none".to_owned()
    } else {
        values
            .iter()
            .map(|value| format!("`{}`", markdown_safe(value)))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn markdown_safe(value: &str) -> String {
    value
        .replace('`', "'")
        .replace(['\n', '\r'], " ")
        .replace('|', "\\|")
}

fn safe_metadata_value(value: &str) -> String {
    let value = value.trim();
    let windows_absolute = value.as_bytes().get(1) == Some(&b':')
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic)
        && value
            .as_bytes()
            .get(2)
            .is_some_and(|byte| matches!(byte, b'/' | b'\\'));
    if std::path::Path::new(value).is_absolute() || windows_absolute || value.starts_with("file:") {
        return "[unsafe reference redacted]".to_owned();
    }
    if let Some((scheme, rest)) = value.split_once("://") {
        if !matches!(scheme, "http" | "https") {
            return "[unsafe reference redacted]".to_owned();
        }
        let authority_end = rest.find('/').unwrap_or(rest.len());
        let authority = &rest[..authority_end];
        let host = authority.rsplit('@').next().unwrap_or_default();
        if host.is_empty()
            || host.chars().any(|ch| {
                !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | ':' | '[' | ']'))
            })
        {
            return "[unsafe reference redacted]".to_owned();
        }
        let path = rest[authority_end..]
            .split(['?', '#'])
            .next()
            .unwrap_or_default();
        return markdown_safe(&format!("{scheme}://{host}{path}"));
    }
    markdown_safe(value)
}
