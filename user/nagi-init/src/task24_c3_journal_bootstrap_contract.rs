//! Shared naming and byte contract for the Task24 development Data journal.

use std::ffi::{OsStr, OsString};
use std::path::{Component, Path};

pub(crate) const DATA_JOURNAL_DIRECTORY: &str = "system-service-journal";
pub(crate) const DATA_JOURNAL_FILE: &str = "system-service-slot.v1";
pub(crate) const DATA_JOURNAL_LOCK_FILE: &str = "system-service-slot.v1.lock";
pub(crate) const DATA_BOOTSTRAP_MARKER_NAME: &str = "task24-c3-first-provision";
pub(crate) const DATA_BOOTSTRAP_MARKER_BYTES: &[u8] = b"NAGI-TASK24-C3-DATA-FIRST-PROVISION-V1\n";
pub(crate) const C3_FIRST_PROVISION_MARKER_MAGIC: &[u8] = b"NAGI-C3-SLOT-FIRST-PROVISION-V1\n";

/// A new ext4 development Data image contains only its normal empty
/// `lost+found` directory and the Data-owner persistence marker before the
/// explicit identity-directory provisioning step.
pub(crate) fn is_fresh_development_data_root(entries: &[OsString]) -> bool {
    entries.len() == 2
        && entries.iter().any(|name| name == OsStr::new("lost+found"))
        && entries
            .iter()
            .any(|name| name == OsStr::new("nagi-dev-data-persistence.txt"))
}

pub(crate) fn has_only_known_data_journal_entries(
    entries: &[OsString],
    c3_marker_name: &str,
) -> bool {
    entries.iter().all(|name| {
        name == OsStr::new(DATA_BOOTSTRAP_MARKER_NAME)
            || name == OsStr::new(c3_marker_name)
            || name == OsStr::new(DATA_JOURNAL_FILE)
            || name == OsStr::new(DATA_JOURNAL_LOCK_FILE)
    })
}

pub(crate) fn is_fresh_data_journal_bootstrap(entries: &[OsString], c3_marker_name: &str) -> bool {
    entries
        .iter()
        .any(|name| name == OsStr::new(DATA_BOOTSTRAP_MARKER_NAME))
        && entries.iter().all(|name| {
            name == OsStr::new(DATA_BOOTSTRAP_MARKER_NAME) || name == OsStr::new(c3_marker_name)
        })
}

/// Reject C1 identity directories that alias or sit inside the C3 journal
/// directory. Inputs must be relative paths containing only normal
/// components, matching the Data owner command-line contract.
pub(crate) fn identity_paths_avoid_data_journal(accounts: &Path, snapshots: &Path) -> bool {
    let Some(accounts) = normal_relative_components(accounts) else {
        return false;
    };
    let Some(snapshots) = normal_relative_components(snapshots) else {
        return false;
    };
    let Some(journal) = normal_relative_components(Path::new(DATA_JOURNAL_DIRECTORY)) else {
        return false;
    };
    !path_components_overlap(&accounts, &journal) && !path_components_overlap(&snapshots, &journal)
}

fn normal_relative_components(path: &Path) -> Option<Vec<&OsStr>> {
    let components = path
        .components()
        .map(|component| match component {
            Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    (!components.is_empty()).then_some(components)
}

fn path_components_overlap(left: &[&OsStr], right: &[&OsStr]) -> bool {
    fn is_prefix(prefix: &[&OsStr], full: &[&OsStr]) -> bool {
        prefix.len() <= full.len() && prefix.iter().zip(full).all(|(a, b)| a == b)
    }
    is_prefix(left, right) || is_prefix(right, left)
}

/// Build the exact service-scoped marker consumed by C3's first-provision
/// journal path. The service ID is supplied by the verified boot catalog.
pub(crate) fn c3_first_provision_marker_bytes(service_id: &str) -> Option<Vec<u8>> {
    if !canonical_service_id(service_id) {
        return None;
    }
    let mut bytes = C3_FIRST_PROVISION_MARKER_MAGIC.to_vec();
    bytes.extend_from_slice(service_id.as_bytes());
    bytes.push(b'\n');
    Some(bytes)
}

fn canonical_service_id(value: &str) -> bool {
    value.len() <= 128
        && !value.is_empty()
        && value.split('.').all(|segment| {
            let bytes = segment.as_bytes();
            if bytes.is_empty() || !bytes[0].is_ascii_lowercase() {
                return false;
            }
            let mut previous_separator = false;
            for byte in &bytes[1..] {
                if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
                    previous_separator = false;
                } else if *byte == b'-' && !previous_separator {
                    previous_separator = true;
                } else {
                    return false;
                }
            }
            !previous_separator
        })
}

#[cfg(test)]
mod tests {
    use super::{
        c3_first_provision_marker_bytes, has_only_known_data_journal_entries,
        identity_paths_avoid_data_journal, is_fresh_data_journal_bootstrap,
        is_fresh_development_data_root, C3_FIRST_PROVISION_MARKER_MAGIC,
        DATA_BOOTSTRAP_MARKER_NAME, DATA_JOURNAL_FILE, DATA_JOURNAL_LOCK_FILE,
    };
    use std::ffi::OsString;
    use std::path::Path;

    #[test]
    fn fresh_data_root_requires_exact_new_image_entries() {
        assert!(is_fresh_development_data_root(&[
            OsString::from("lost+found"),
            OsString::from("nagi-dev-data-persistence.txt"),
        ]));
        assert!(is_fresh_development_data_root(&[
            OsString::from("nagi-dev-data-persistence.txt"),
            OsString::from("lost+found"),
        ]));
        for entries in [
            vec![OsString::from("lost+found")],
            vec![OsString::from("nagi-dev-data-persistence.txt")],
            vec![
                OsString::from("lost+found"),
                OsString::from("nagi-dev-data-persistence.txt"),
                OsString::from("accounts"),
            ],
            vec![
                OsString::from("lost+found"),
                OsString::from("system-service-journal"),
            ],
            vec![OsString::from("lost+found"), OsString::from("lost+found")],
        ] {
            assert!(!is_fresh_development_data_root(&entries));
        }
    }

    #[test]
    fn c3_first_provision_marker_uses_only_canonical_catalog_ids() {
        let mut expected = C3_FIRST_PROVISION_MARKER_MAGIC.to_vec();
        expected.extend_from_slice(b"org.nagi.dev.task24-identity\n");
        assert_eq!(
            c3_first_provision_marker_bytes("org.nagi.dev.task24-identity"),
            Some(expected)
        );
        for service_id in [
            "",
            "Org.nagi.identity",
            "org..identity",
            "org.identity-",
            "org.identity\n",
        ] {
            assert_eq!(c3_first_provision_marker_bytes(service_id), None);
        }
    }

    #[test]
    fn journal_bootstrap_accepts_only_its_exact_one_time_entries() {
        let c3_marker = format!("{DATA_JOURNAL_FILE}.first-provision");
        let staged_c3_marker = format!("{c3_marker}.pending");
        assert!(is_fresh_data_journal_bootstrap(
            &[OsString::from(DATA_BOOTSTRAP_MARKER_NAME)],
            &c3_marker,
        ));
        assert!(is_fresh_data_journal_bootstrap(
            &[
                OsString::from(DATA_BOOTSTRAP_MARKER_NAME),
                OsString::from(&c3_marker),
            ],
            &c3_marker,
        ));
        for entries in [
            vec![OsString::from(&c3_marker)],
            vec![
                OsString::from(DATA_BOOTSTRAP_MARKER_NAME),
                OsString::from(&staged_c3_marker),
            ],
            vec![
                OsString::from(DATA_BOOTSTRAP_MARKER_NAME),
                OsString::from(DATA_JOURNAL_FILE),
            ],
            vec![
                OsString::from(DATA_BOOTSTRAP_MARKER_NAME),
                OsString::from("unexpected"),
            ],
        ] {
            assert!(!is_fresh_data_journal_bootstrap(&entries, &c3_marker));
        }
        assert!(has_only_known_data_journal_entries(
            &[
                OsString::from(DATA_JOURNAL_FILE),
                OsString::from(DATA_JOURNAL_LOCK_FILE),
            ],
            &c3_marker,
        ));
        assert!(!has_only_known_data_journal_entries(
            &[OsString::from("unexpected")],
            &c3_marker,
        ));
        assert!(!has_only_known_data_journal_entries(
            &[OsString::from(&staged_c3_marker)],
            &c3_marker,
        ));
    }

    #[test]
    fn c1_identity_directories_cannot_overlap_the_c3_journal_directory() {
        assert!(identity_paths_avoid_data_journal(
            Path::new("accounts"),
            Path::new("snapshots"),
        ));
        assert!(!identity_paths_avoid_data_journal(
            Path::new("system-service-journal"),
            Path::new("snapshots"),
        ));
        assert!(!identity_paths_avoid_data_journal(
            Path::new("accounts"),
            Path::new("system-service-journal/nested"),
        ));
        assert!(identity_paths_avoid_data_journal(
            Path::new("system-service-journal-sibling"),
            Path::new("snapshots"),
        ));
        assert!(!identity_paths_avoid_data_journal(
            Path::new("system-service-journal/../accounts"),
            Path::new("snapshots"),
        ));
    }
}
