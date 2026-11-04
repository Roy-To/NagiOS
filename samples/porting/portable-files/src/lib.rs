// SPDX-License-Identifier: MPL-2.0
// Adapted from apps/nagi-files/src/model.rs (Nagi OS contributors).
// This excerpt keeps path and name validation; it omits the OS-backed provider.

use std::collections::BTreeSet;
use std::fmt::{Display, Formatter};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathError {
    InvalidLocation,
    InvalidName,
    SandboxEscape,
    AlreadyExists,
}

impl Display for PathError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for PathError {}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Location(String);

impl Location {
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Parses a provider-relative path using `/` as the sole separator.
    /// This validation is independent of the host operating system.
    pub fn parse(path: &str) -> Result<Self, PathError> {
        if path.starts_with('/') || path.starts_with('\\') || path.contains('\\') {
            return Err(PathError::InvalidLocation);
        }
        if path.as_bytes().get(1) == Some(&b':') {
            return Err(PathError::InvalidLocation);
        }
        let mut components = Vec::new();
        for component in path.split('/') {
            if component.is_empty() {
                continue;
            }
            if component == "." || component == ".." {
                return Err(PathError::SandboxEscape);
            }
            if component.chars().any(char::is_control) {
                return Err(PathError::InvalidName);
            }
            components.push(FileName::parse(component)?.0);
        }
        Ok(Self(components.join("/")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        match self.0.rsplit_once('/') {
            Some((parent, _)) => Some(Self(parent.to_owned())),
            None => Some(Self::root()),
        }
    }

    pub fn is_within(&self, scope: &Self) -> bool {
        scope.is_root()
            || self == scope
            || self
                .0
                .strip_prefix(&scope.0)
                .is_some_and(|suffix| suffix.starts_with('/'))
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileName(String);

impl FileName {
    pub fn parse(name: &str) -> Result<Self, PathError> {
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.contains('\\')
            || name.chars().any(|character| ":*?\"<>|".contains(character))
            || name.chars().any(char::is_control)
            || name.ends_with('.')
            || name.ends_with(' ')
        {
            return Err(PathError::InvalidName);
        }
        let device_name = name
            .split('.')
            .next()
            .unwrap_or(name)
            .trim_end_matches(['.', ' '])
            .to_ascii_uppercase();
        if matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ["COM", "LPT"].iter().any(|prefix| {
                device_name.strip_prefix(prefix).is_some_and(|number| {
                    number.len() == 1
                        && number
                            .chars()
                            .next()
                            .is_some_and(|digit| ('1'..='9').contains(&digit))
                })
            })
        {
            return Err(PathError::InvalidName);
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// In-memory application core for the sample; it never opens host files.
#[derive(Default)]
pub struct PortableCatalog {
    entries: BTreeSet<Location>,
}

impl PortableCatalog {
    pub fn add(&mut self, relative_path: &str) -> Result<(), PathError> {
        let location = Location::parse(relative_path)?;
        if location.is_root() {
            return Err(PathError::InvalidLocation);
        }
        if !self.entries.insert(location) {
            return Err(PathError::AlreadyExists);
        }
        Ok(())
    }

    pub fn contains(&self, relative_path: &str) -> Result<bool, PathError> {
        Ok(self.entries.contains(&Location::parse(relative_path)?))
    }

    pub fn under(&self, scope: &str) -> Result<Vec<&Location>, PathError> {
        let scope = Location::parse(scope)?;
        Ok(self
            .entries
            .iter()
            .filter(|location| location.is_within(&scope))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::{FileName, Location, PathError, PortableCatalog};

    #[test]
    fn relative_locations_are_normalized_and_scoped_by_components() {
        let location = Location::parse("photos//2026/../cover.jpg");
        assert_eq!(location, Err(PathError::SandboxEscape));

        let location = Location::parse("photos//2026/cover.jpg").expect("portable path");
        let scope = Location::parse("photos").expect("scope");
        assert_eq!(location.as_str(), "photos/2026/cover.jpg");
        assert!(location.is_within(&scope));
        assert_eq!(location.parent().unwrap().as_str(), "photos/2026");
    }

    #[test]
    fn traversal_and_host_absolute_paths_are_rejected() {
        assert_eq!(Location::parse("../outside"), Err(PathError::SandboxEscape));
        assert_eq!(
            Location::parse("C:/outside"),
            Err(PathError::InvalidLocation)
        );
        assert_eq!(
            Location::parse("folder\\file"),
            Err(PathError::InvalidLocation)
        );
    }

    #[test]
    fn file_name_rules_are_reused_for_portable_catalog_entries() {
        assert_eq!(FileName::parse("NUL.txt"), Err(PathError::InvalidName));
        let mut catalog = PortableCatalog::default();
        catalog.add("docs/guide.md").expect("add fixture entry");
        assert_eq!(catalog.contains("docs/guide.md"), Ok(true));
        assert_eq!(catalog.add("docs/guide.md"), Err(PathError::AlreadyExists));
        assert_eq!(catalog.under("docs").unwrap().len(), 1);
    }
}
