use core::cmp::Ordering;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::str::FromStr;

/// Validated semantic version used for app and minimum Nagi runtime versions.
///
/// Parsing and precedence follow the standard SemVer 2.0 rules provided by the
/// `semver` crate. Build metadata is retained for display but ignored in
/// equality, hashing, and ordering because it does not affect SemVer precedence.
#[derive(Clone, Debug)]
pub struct AppVersion(semver::Version);

impl AppVersion {
    pub fn parse(value: &str) -> Result<Self, semver::Error> {
        value.parse()
    }
}

impl FromStr for AppVersion {
    type Err = semver::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        semver::Version::parse(value).map(Self)
    }
}

impl fmt::Display for AppVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl PartialEq for AppVersion {
    fn eq(&self, other: &Self) -> bool {
        self.0.cmp_precedence(&other.0) == Ordering::Equal
    }
}

impl Eq for AppVersion {}

impl PartialOrd for AppVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AppVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp_precedence(&other.0)
    }
}

impl Hash for AppVersion {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.major.hash(state);
        self.0.minor.hash(state);
        self.0.patch.hash(state);
        self.0.pre.hash(state);
    }
}
