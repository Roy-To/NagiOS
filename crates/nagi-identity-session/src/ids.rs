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
    ($name:ident, $prefix:literal) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdError> {
                let value = value.into();
                validate_prefixed_id(&value, $prefix)?;
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

opaque_string_id!(ProfileId, "profile_");
opaque_string_id!(SessionId, "session_");
opaque_string_id!(CallerContextId, "caller_");

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
