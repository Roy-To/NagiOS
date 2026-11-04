use std::fmt;
use std::num::NonZeroU64;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use nagi_capability::PrincipalId;

const MAX_ID_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdError {
    Zero,
    Empty,
    TooLong,
    InvalidNamespace,
    InvalidCharacter,
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Zero => "user ID must be non-zero",
            Self::Empty => "identifier is empty",
            Self::TooLong => "identifier exceeds the 128-byte limit",
            Self::InvalidNamespace => "identifier has an invalid type namespace",
            Self::InvalidCharacter => "identifier contains an unsupported character",
        })
    }
}

impl std::error::Error for IdError {}

/// Validated wrapper for the existing model-level UserId. It keeps the
/// canonical numeric representation while preventing zero or unchecked IDs
/// from entering this identity boundary.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct UserId(NonZeroU64);

impl UserId {
    pub fn new(value: u64) -> Result<Self, IdError> {
        NonZeroU64::new(value).map(Self).ok_or(IdError::Zero)
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub const fn to_model(self) -> nagi_model::UserId {
        nagi_model::UserId(self.get())
    }

    pub fn from_model(value: nagi_model::UserId) -> Result<Self, IdError> {
        Self::new(value.0)
    }
}

impl Serialize for UserId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(self.get())
    }
}

impl<'de> Deserialize<'de> for UserId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(u64::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

macro_rules! opaque_string_id {
    ($name:ident, $prefix:literal, $validator:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                $validator(&value, $prefix)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

opaque_string_id!(ProfileId, "profile_", validate_prefixed_id);
opaque_string_id!(SessionId, "session_", validate_session_id);
opaque_string_id!(CallerContextId, "caller_", validate_prefixed_id);

impl SessionId {
    /// Bind a source-provided nonce to the next store CAS generation. The dot
    /// delimiter is reserved for this format and was rejected by legacy
    /// SessionId validation, so IDs from lost legacy generations cannot occupy
    /// the generated namespace.
    pub(crate) fn in_store_generation(generation: u64, nonce: &SessionId) -> Result<Self, IdError> {
        let prefix = format!("session_epoch.{generation}_");
        let available = MAX_ID_BYTES.saturating_sub(prefix.len());
        let source_suffix = nonce
            .as_str()
            .strip_prefix("session_")
            .ok_or(IdError::InvalidNamespace)?;
        let suffix: String = source_suffix
            .bytes()
            .filter(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
            })
            .take(available)
            .map(char::from)
            .collect();
        let suffix = if suffix.is_empty() {
            "id"
        } else {
            suffix.as_str()
        };
        Self::new(format!("{prefix}{suffix}"))
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(IdError::Empty);
        }
        if value.len() > MAX_ID_BYTES {
            return Err(IdError::TooLong);
        }
        if !value.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.')
        }) {
            return Err(IdError::InvalidCharacter);
        }
        Ok(Self(value))
    }
}

impl<'de> Deserialize<'de> for IdempotencyKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

/// A provider-specific subject reference is transient mapping input. It is
/// not part of LocalUser or profile metadata and must not be logged.
#[derive(Clone, Eq, PartialEq)]
pub struct ProviderSubjectRef {
    provider: String,
    subject: String,
}

impl fmt::Debug for ProviderSubjectRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProviderSubjectRef")
            .field("provider", &self.provider)
            .field("subject", &"<redacted>")
            .finish()
    }
}

impl ProviderSubjectRef {
    pub fn new(provider: impl Into<String>, subject: impl Into<String>) -> Result<Self, IdError> {
        let provider = provider.into();
        let subject = subject.into();
        validate_simple_token(&provider)?;
        if subject.is_empty() {
            return Err(IdError::Empty);
        }
        if subject.len() > MAX_ID_BYTES {
            return Err(IdError::TooLong);
        }
        if subject.chars().any(char::is_control) {
            return Err(IdError::InvalidCharacter);
        }
        Ok(Self { provider, subject })
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn subject(&self) -> &str {
        &self.subject
    }
}

fn validate_session_id(value: &str, prefix: &str) -> Result<(), IdError> {
    if value.len() > MAX_ID_BYTES {
        return Err(IdError::TooLong);
    }
    if let Some(generation_suffix) = value.strip_prefix("session_epoch.") {
        let Some((generation, nonce)) = generation_suffix.split_once('_') else {
            return Err(IdError::InvalidCharacter);
        };
        let parsed_generation = generation
            .parse::<u64>()
            .map_err(|_| IdError::InvalidCharacter)?;
        if parsed_generation.to_string() != generation
            || nonce.is_empty()
            || !nonce.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
            })
        {
            return Err(IdError::InvalidCharacter);
        }
        return Ok(());
    }
    validate_prefixed_id(value, prefix)
}

fn validate_prefixed_id(value: &str, prefix: &str) -> Result<(), IdError> {
    if value.is_empty() {
        return Err(IdError::Empty);
    }
    if value.len() > MAX_ID_BYTES {
        return Err(IdError::TooLong);
    }
    let Some(suffix) = value.strip_prefix(prefix) else {
        return Err(IdError::InvalidNamespace);
    };
    if suffix.is_empty()
        || !suffix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
    {
        return Err(IdError::InvalidCharacter);
    }
    Ok(())
}

fn validate_simple_token(value: &str) -> Result<(), IdError> {
    if value.is_empty() {
        return Err(IdError::Empty);
    }
    if value.len() > MAX_ID_BYTES {
        return Err(IdError::TooLong);
    }
    if !value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.'))
    {
        return Err(IdError::InvalidCharacter);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_generation_session_ids_use_reserved_round_trippable_namespace() {
        let source = SessionId::new("session_s1").unwrap();
        let generated = SessionId::in_store_generation(42, &source).unwrap();
        assert_eq!(generated.as_str(), "session_epoch.42_s1");
        assert_eq!(SessionId::new(generated.as_str()), Ok(generated));
        assert_eq!(
            SessionId::new("session_legacy.id"),
            Err(IdError::InvalidCharacter)
        );
    }

    #[test]
    fn store_generation_session_id_fits_a_maximum_length_source_id() {
        let source = SessionId::new(format!("session_{}", "s".repeat(120))).unwrap();
        let generated = SessionId::in_store_generation(u64::MAX, &source).unwrap();
        assert!(generated.as_str().len() <= MAX_ID_BYTES);
        assert!(generated
            .as_str()
            .starts_with("session_epoch.18446744073709551615_"));
        assert_eq!(SessionId::new(generated.as_str()), Ok(generated));
    }
}
