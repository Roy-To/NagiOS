use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

use crate::ids::{CapabilityId, PrincipalId, SessionId};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CapabilityScope {
    Unscoped,
    Filesystem {
        path: String,
    },
    NetworkOrigin {
        origin: String,
    },
    DeviceClass {
        class: String,
    },
    ModelProvider {
        provider: String,
    },
    AutomationTarget {
        target: String,
    },
    Opaque {
        namespace: CapabilityId,
        resource: String,
    },
}

impl CapabilityScope {
    pub fn validate(&self) -> Result<(), ScopeError> {
        match self {
            Self::Unscoped => Ok(()),
            Self::Filesystem { path } => {
                if valid_logical_path(path) {
                    Ok(())
                } else {
                    Err(ScopeError::InvalidFilesystemPath)
                }
            }
            Self::NetworkOrigin { origin } => {
                if valid_canonical_origin(origin) {
                    Ok(())
                } else {
                    Err(ScopeError::InvalidNetworkOrigin)
                }
            }
            Self::DeviceClass { class } => validate_scope_token(class),
            Self::ModelProvider { provider } => validate_scope_token(provider),
            Self::AutomationTarget { target } => validate_scope_token(target),
            Self::Opaque { resource, .. } => validate_scope_token(resource),
        }
    }

    /// Only filesystem scopes have hierarchical matching. Other scopes match
    /// exactly, and unscoped never means wildcard.
    pub fn covers(&self, requested: &Self) -> bool {
        match (self, requested) {
            (Self::Filesystem { path: grant }, Self::Filesystem { path: request }) => {
                grant == request
                    || grant == "/"
                    || request
                        .strip_prefix(grant)
                        .is_some_and(|suffix| suffix.starts_with('/'))
            }
            _ => self == requested,
        }
    }

    pub(crate) fn specificity(&self) -> usize {
        match self {
            Self::Filesystem { path } => path.len(),
            Self::Unscoped => 0,
            _ => usize::MAX,
        }
    }
}

fn valid_logical_path(path: &str) -> bool {
    if path.is_empty() || path.len() > 1024 || !path.starts_with('/') || path.contains('\\') {
        return false;
    }
    if path != "/" && path.ends_with('/') {
        return false;
    }
    if path.chars().any(char::is_control) {
        return false;
    }
    path == "/"
        || path
            .split('/')
            .skip(1)
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

fn valid_canonical_origin(origin: &str) -> bool {
    let Some((scheme, authority)) = origin.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "http" | "https")
        || authority.is_empty()
        || authority.len() > 512
        || authority
            .chars()
            .any(|character| matches!(character, '/' | '?' | '#' | '@'))
    {
        return false;
    }
    if authority.starts_with('[') {
        let Some(end) = authority.find(']') else {
            return false;
        };
        let address = &authority[1..end];
        let suffix = &authority[end + 1..];
        if address.parse::<std::net::Ipv6Addr>().is_err() {
            return false;
        }
        return suffix.is_empty() || suffix.strip_prefix(':').is_some_and(valid_port);
    }

    let (host, port) = match authority.split_once(':') {
        Some((host, port)) if !port.contains(':') => (host, Some(port)),
        Some(_) => return false,
        None => (authority, None),
    };
    if host.is_empty() || host.starts_with('.') || host.ends_with('.') || host.contains("..") {
        return false;
    }
    let valid_host = host.split('.').all(|label| {
        !label.is_empty()
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
            })
    });
    valid_host && port.is_none_or(valid_port)
}

fn valid_port(port: &str) -> bool {
    port.parse::<u16>()
        .is_ok_and(|number| number > 0 && port == number.to_string())
}

fn validate_scope_token(value: &str) -> Result<(), ScopeError> {
    if value.is_empty()
        || value.len() > 256
        || value.chars().any(|character| {
            character.is_control() || character.is_whitespace() || matches!(character, '*' | '?')
        })
    {
        return Err(ScopeError::InvalidScopeToken);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeError {
    InvalidFilesystemPath,
    InvalidNetworkOrigin,
    InvalidScopeToken,
}

impl fmt::Display for ScopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidFilesystemPath => "filesystem paths must be normalized absolute logical paths without dot segments",
            Self::InvalidNetworkOrigin => "network origins must be canonical lowercase HTTP(S) origins without credentials or path",
            Self::InvalidScopeToken => "scope values must be non-empty, bounded, and contain no wildcard or control characters",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ScopeError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Principal {
    pub id: PrincipalId,
    pub kind: PrincipalKind,
    #[serde(default)]
    pub publisher_id: Option<String>,
    #[serde(default)]
    pub package_id: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

impl Principal {
    pub fn validate(&self) -> Result<(), PrincipalError> {
        validate_optional_metadata(&self.publisher_id)?;
        validate_optional_metadata(&self.package_id)?;
        validate_optional_metadata(&self.display_name)?;
        Ok(())
    }
}

fn validate_optional_metadata(value: &Option<String>) -> Result<(), PrincipalError> {
    if value.as_ref().is_some_and(|value| {
        value.is_empty() || value.len() > 256 || value.chars().any(char::is_control)
    }) {
        return Err(PrincipalError::InvalidMetadata);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    FirstPartyApp,
    ThirdPartyApp,
    SystemService,
    AiMediatedAction,
    Automation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrincipalError {
    InvalidMetadata,
}

impl fmt::Display for PrincipalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("principal metadata must be bounded and contain no control characters")
    }
}

impl std::error::Error for PrincipalError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", deny_unknown_fields)]
pub enum GrantLifetime {
    OneShot,
    Session { session_id: SessionId },
    Persistent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantEffect {
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantSource {
    User,
    System,
    Policy,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct GrantId(u64);

impl<'de> Deserialize<'de> for GrantId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = u64::deserialize(deserializer)?;
        if value == 0 {
            return Err(D::Error::custom("grant ID must be positive"));
        }
        Ok(Self(value))
    }
}

impl GrantId {
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Display for GrantId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnixTimestamp(u64);

impl UnixTimestamp {
    pub const fn from_unix_seconds(seconds: u64) -> Self {
        Self(seconds)
    }

    pub const fn as_unix_seconds(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub id: GrantId,
    pub principal: PrincipalId,
    pub capability: CapabilityId,
    pub effect: GrantEffect,
    pub scope: CapabilityScope,
    pub lifetime: GrantLifetime,
    pub source: GrantSource,
    #[serde(default)]
    pub reason: Option<String>,
    pub granted_at: UnixTimestamp,
    #[serde(default)]
    pub expires_at: Option<UnixTimestamp>,
    #[serde(default)]
    pub revoked_at: Option<UnixTimestamp>,
    #[serde(default)]
    pub consumed_at: Option<UnixTimestamp>,
}

impl Grant {
    pub fn validate(&self) -> Result<(), GrantValidationError> {
        self.scope
            .validate()
            .map_err(GrantValidationError::InvalidScope)?;
        if self
            .expires_at
            .is_some_and(|expiry| expiry <= self.granted_at)
        {
            return Err(GrantValidationError::ExpiryNotAfterGrant);
        }
        if self
            .revoked_at
            .is_some_and(|revoked| revoked < self.granted_at)
        {
            return Err(GrantValidationError::RevocationBeforeGrant);
        }
        if self
            .consumed_at
            .is_some_and(|consumed| consumed < self.granted_at)
        {
            return Err(GrantValidationError::ConsumptionBeforeGrant);
        }
        if self.consumed_at.is_some()
            && (self.effect != GrantEffect::Allow || self.lifetime != GrantLifetime::OneShot)
        {
            return Err(GrantValidationError::UnexpectedConsumption);
        }
        if self.reason.as_ref().is_some_and(|reason| {
            reason.is_empty() || reason.len() > 512 || reason.chars().any(char::is_control)
        }) {
            return Err(GrantValidationError::InvalidReason);
        }
        Ok(())
    }

    pub(crate) fn is_active_at(&self, context: &crate::EvaluationContext) -> ActiveGrant {
        if self.revoked_at.is_some() {
            return ActiveGrant::Revoked;
        }
        if context.now < self.granted_at {
            return ActiveGrant::NotYetValid;
        }
        if self.expires_at.is_some_and(|expiry| expiry <= context.now) {
            return ActiveGrant::Expired;
        }
        match &self.lifetime {
            GrantLifetime::OneShot if self.consumed_at.is_some() => ActiveGrant::Consumed,
            GrantLifetime::OneShot | GrantLifetime::Persistent => ActiveGrant::Active,
            GrantLifetime::Session { session_id } => {
                if context.session_id.as_ref() == Some(session_id) {
                    ActiveGrant::Active
                } else {
                    ActiveGrant::WrongSession
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ActiveGrant {
    Active,
    Revoked,
    Expired,
    NotYetValid,
    Consumed,
    WrongSession,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GrantValidationError {
    InvalidScope(ScopeError),
    ExpiryNotAfterGrant,
    RevocationBeforeGrant,
    ConsumptionBeforeGrant,
    UnexpectedConsumption,
    InvalidReason,
}

impl fmt::Display for GrantValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScope(error) => error.fmt(formatter),
            Self::ExpiryNotAfterGrant => {
                formatter.write_str("grant expiry must follow its creation time")
            }
            Self::RevocationBeforeGrant => {
                formatter.write_str("grant revocation cannot precede its creation time")
            }
            Self::ConsumptionBeforeGrant => {
                formatter.write_str("one-shot consumption cannot precede grant creation")
            }
            Self::UnexpectedConsumption => formatter
                .write_str("only an allowed one-shot grant may have a consumption timestamp"),
            Self::InvalidReason => formatter
                .write_str("grant reason must be bounded and contain no control characters"),
        }
    }
}

impl std::error::Error for GrantValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filesystem_scope_is_segment_aware() {
        let grant = CapabilityScope::Filesystem {
            path: "/data/docs".into(),
        };
        assert!(grant.covers(&CapabilityScope::Filesystem {
            path: "/data/docs/notes.txt".into(),
        }));
        assert!(!CapabilityScope::Filesystem {
            path: "/data/docs/notes.txt".into(),
        }
        .covers(&grant));
        assert!(!grant.covers(&CapabilityScope::Filesystem {
            path: "/data/docs-old/notes.txt".into(),
        }));
    }

    #[test]
    fn filesystem_scope_rejects_traversal_and_noncanonical_paths() {
        for path in [
            "",
            "data",
            "/data/../secrets",
            "/data/./file",
            "/data//file",
            "/data/",
        ] {
            assert!(CapabilityScope::Filesystem { path: path.into() }
                .validate()
                .is_err());
        }
    }

    #[test]
    fn network_scope_requires_a_canonical_origin() {
        for origin in [
            "https://",
            "https://User@example.com",
            "https://example.com/path",
            "https://EXAMPLE.com",
            "https://:80",
            "https://example.com:",
            "https://example.com:70000",
        ] {
            assert!(CapabilityScope::NetworkOrigin {
                origin: origin.into()
            }
            .validate()
            .is_err());
        }
        assert!(CapabilityScope::NetworkOrigin {
            origin: "https://example.com:443".into()
        }
        .validate()
        .is_ok());
        assert!(CapabilityScope::NetworkOrigin {
            origin: "https://[2001:db8::1]:443".into()
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn unscoped_is_not_a_wildcard() {
        assert!(
            !CapabilityScope::Unscoped.covers(&CapabilityScope::Filesystem {
                path: "/data".into(),
            })
        );
    }
}
