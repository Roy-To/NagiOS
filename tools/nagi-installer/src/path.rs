use std::collections::BTreeMap;

use crate::InstallerError;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PackageRelativePath(String);

impl PackageRelativePath {
    pub fn parse(value: &str) -> Result<Self, InstallerError> {
        if value.is_empty()
            || value.starts_with('/')
            || value.starts_with('\\')
            || value.contains('\\')
            || value.contains(':')
            || value.len() > 4096
        {
            return Err(InstallerError::UnsafePath(format!(
                "package path is absolute, non-portable, or too long: {value:?}"
            )));
        }
        let mut segments = Vec::new();
        for segment in value.split('/') {
            if segment.is_empty()
                || segment == "."
                || segment == ".."
                || segment.len() > 255
                || segment.ends_with('.')
                || segment.ends_with(' ')
                || segment.chars().any(|character| {
                    character.is_control() || matches!(character, '<' | '>' | '"' | '|' | '?' | '*')
                })
                || is_windows_device_name(segment)
            {
                return Err(InstallerError::UnsafePath(format!(
                    "package path contains an unsafe or non-portable component: {value:?}"
                )));
            }
            segments.push(segment);
        }
        Ok(Self(segments.join("/")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn normalized_collision_key(&self) -> String {
        self.0.to_lowercase()
    }

    pub(crate) fn parent_paths(&self) -> impl Iterator<Item = String> + '_ {
        let mut prefix = String::new();
        let segment_count = self.0.split('/').count().saturating_sub(1);
        self.0.split('/').take(segment_count).map(move |segment| {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(segment);
            prefix.clone()
        })
    }
}

fn is_windows_device_name(segment: &str) -> bool {
    let stem = segment.split('.').next().unwrap_or(segment);
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || ["COM", "LPT"].iter().any(|prefix| {
        upper.strip_prefix(prefix).is_some_and(|suffix| {
            matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
                || matches!(suffix, "¹" | "²" | "³")
        })
    })
}

pub(crate) fn validate_destination_paths<'a>(
    paths: impl IntoIterator<Item = &'a PackageRelativePath>,
) -> Result<(), InstallerError> {
    let mut by_normalized = BTreeMap::<String, String>::new();
    for path in paths {
        let key = path.normalized_collision_key();
        if let Some(previous) = by_normalized.insert(key, path.as_str().to_owned()) {
            return Err(InstallerError::DuplicateDestination(format!(
                "package paths {previous:?} and {:?} conflict on case-insensitive filesystems",
                path.as_str()
            )));
        }
    }
    for path in by_normalized.values() {
        let parsed = PackageRelativePath::parse(path)?;
        for parent in parsed.parent_paths() {
            if by_normalized.contains_key(&parent.to_lowercase()) {
                return Err(InstallerError::DuplicateDestination(format!(
                    "file destination {parent:?} conflicts with descendant {path:?}"
                )));
            }
        }
    }
    Ok(())
}
