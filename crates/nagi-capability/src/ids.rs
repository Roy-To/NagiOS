use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

const MAX_IDENTIFIER_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentifierError {
    Empty,
    TooLong,
    InvalidNamespace,
    InvalidCharacter,
}

impl fmt::Display for IdentifierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "identifier is empty",
            Self::TooLong => "identifier exceeds the 128-byte limit",
            Self::InvalidNamespace => {
                "capability identifiers must contain lowercase dot-separated namespace segments"
            }
            Self::InvalidCharacter => "identifier contains an unsupported character",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for IdentifierError {}

macro_rules! validated_string_id {
    ($name:ident, $validator:ident) => {
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, IdentifierError> {
                let value = value.into();
                $validator(&value)?;
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
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(D::Error::custom)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

validated_string_id!(CapabilityId, validate_capability_id);
validated_string_id!(PrincipalId, validate_principal_id);
validated_string_id!(SessionId, validate_session_id);

fn validate_capability_id(value: &str) -> Result<(), IdentifierError> {
    validate_length(value)?;
    let mut segments = value.split('.');
    let Some(first) = segments.next() else {
        return Err(IdentifierError::InvalidNamespace);
    };
    if !valid_namespace_segment(first) || segments.clone().next().is_none() {
        return Err(IdentifierError::InvalidNamespace);
    }
    if segments.any(|segment| !valid_namespace_segment(segment)) {
        return Err(IdentifierError::InvalidNamespace);
    }
    Ok(())
}

fn valid_namespace_segment(segment: &str) -> bool {
    let mut chars = segment.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && chars.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn validate_principal_id(value: &str) -> Result<(), IdentifierError> {
    validate_length(value)?;
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(IdentifierError::Empty);
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(IdentifierError::InvalidCharacter);
    }
    if chars.any(|character| {
        !(character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '.' | '_' | ':' | '/' | '-'))
    }) {
        return Err(IdentifierError::InvalidCharacter);
    }
    Ok(())
}

fn validate_session_id(value: &str) -> Result<(), IdentifierError> {
    validate_length(value)?;
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(IdentifierError::Empty);
    };
    if !first.is_ascii_alphanumeric()
        || chars.any(|character| {
            !(character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-'))
        })
    {
        return Err(IdentifierError::InvalidCharacter);
    }
    Ok(())
}

fn validate_length(value: &str) -> Result<(), IdentifierError> {
    if value.is_empty() {
        return Err(IdentifierError::Empty);
    }
    if value.len() > MAX_IDENTIFIER_BYTES {
        return Err(IdentifierError::TooLong);
    }
    Ok(())
}
