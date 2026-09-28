use crate::model::{ComponentRecord, Inventory, InventoryDiff, LicenseChange, VersionChange};
use std::collections::{BTreeMap, BTreeSet};

pub fn compare_inventories(before: &Inventory, after: &Inventory) -> InventoryDiff {
    let before_by_id = before
        .components
        .iter()
        .map(|component| (component.component_id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    let after_by_id = after
        .components
        .iter()
        .map(|component| (component.component_id.as_str(), component))
        .collect::<BTreeMap<_, _>>();

    let mut added = after
        .components
        .iter()
        .filter(|component| !before_by_id.contains_key(component.component_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let mut removed = before
        .components
        .iter()
        .filter(|component| !after_by_id.contains_key(component.component_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();

    let mut old_by_stable_identity = BTreeMap::<String, Vec<&ComponentRecord>>::new();
    let mut new_by_stable_identity = BTreeMap::<String, Vec<&ComponentRecord>>::new();
    for component in &before.components {
        old_by_stable_identity
            .entry(stable_identity(component))
            .or_default()
            .push(component);
    }
    for component in &after.components {
        new_by_stable_identity
            .entry(stable_identity(component))
            .or_default()
            .push(component);
    }

    let mut version_changes = Vec::new();
    let mut license_changes = Vec::new();
    let mut consumed_added = BTreeSet::new();
    let mut consumed_removed = BTreeSet::new();
    for (identity, old_values) in &old_by_stable_identity {
        let Some(new_values) = new_by_stable_identity.get(identity) else {
            continue;
        };
        if old_values.len() == 1 && new_values.len() == 1 {
            let old = old_values[0];
            let new = new_values[0];
            if old.version != new.version {
                version_changes.push(VersionChange {
                    name: new.name.clone(),
                    from: old.version.clone().unwrap_or_else(|| "unknown".to_owned()),
                    to: new.version.clone().unwrap_or_else(|| "unknown".to_owned()),
                    source: new.origin.clone().or_else(|| old.origin.clone()),
                });
                consumed_added.insert(new.component_id.clone());
                consumed_removed.insert(old.component_id.clone());
            }
            if old.declared_license_expression != new.declared_license_expression
                || old.license_status != new.license_status
            {
                license_changes.push(LicenseChange {
                    component_id: new.component_id.clone(),
                    name: new.name.clone(),
                    from: old.declared_license_expression.clone(),
                    to: new.declared_license_expression.clone(),
                });
            }
        }
    }
    added.retain(|component| !consumed_added.contains(&component.component_id));
    removed.retain(|component| !consumed_removed.contains(&component.component_id));

    let before_missing = before
        .components
        .iter()
        .filter(|component| missing_license_evidence(component))
        .map(|component| component.component_id.as_str())
        .collect::<BTreeSet<_>>();
    let new_missing_license_conditions = after
        .components
        .iter()
        .filter(|component| {
            missing_license_evidence(component)
                && !before_missing.contains(component.component_id.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();

    added.sort_by(component_order);
    removed.sort_by(component_order);
    version_changes.sort_by(|left, right| {
        (&left.name, &left.from, &left.to).cmp(&(&right.name, &right.from, &right.to))
    });
    license_changes.sort_by(|left, right| {
        (&left.name, &left.component_id).cmp(&(&right.name, &right.component_id))
    });

    InventoryDiff {
        added,
        removed,
        version_changes,
        license_changes,
        new_missing_license_conditions,
    }
}

pub fn missing_license_evidence(component: &ComponentRecord) -> bool {
    component.declared_license_expression.is_none()
        && component.license_file_declared.is_none()
        && component.detected_license_files.is_empty()
}

fn stable_identity(component: &ComponentRecord) -> String {
    if let Some(purl) = &component.purl {
        let prefix = purl.split('@').next().unwrap_or(purl);
        return format!("{}|{}", component.package_type, prefix);
    }
    format!(
        "{}|{}|{}|{}",
        component.package_type,
        component.name,
        component.origin.as_deref().unwrap_or_default(),
        component.local_path.as_deref().unwrap_or_default()
    )
}

fn component_order(left: &ComponentRecord, right: &ComponentRecord) -> std::cmp::Ordering {
    (&left.name, &left.version, &left.component_id).cmp(&(
        &right.name,
        &right.version,
        &right.component_id,
    ))
}
