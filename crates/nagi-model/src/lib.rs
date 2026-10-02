#![no_std]

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UserId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppId(pub u64);

impl AppId {
    /// Derive the stable logical identity used by packages and SDK clients.
    /// The identifier is a model-level naming rule, not an authority grant.
    pub const fn from_identifier(identifier: &[u8]) -> Self {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        let mut index = 0;
        while index < identifier.len() {
            hash ^= identifier[index] as u64;
            hash = hash.wrapping_mul(0x1000_0000_01b3);
            index += 1;
        }
        Self(hash)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppSessionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExecutionInstanceId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SurfaceId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObjectId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PresentationClass {
    Compact,
    Medium,
    Expanded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentationContext {
    pub logical_width: u16,
    pub logical_height: u16,
    pub dpi: u16,
    pub touch: bool,
    pub keyboard: bool,
    pub pointer: bool,
    pub class: PresentationClass,
}

impl PresentationContext {
    pub const fn compact(width: u16, height: u16) -> Self {
        Self {
            logical_width: width,
            logical_height: height,
            dpi: 96,
            touch: false,
            keyboard: true,
            pointer: true,
            class: PresentationClass::Compact,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PresentationSurface {
    pub surface_id: SurfaceId,
    pub node_id: NodeId,
    pub context: PresentationContext,
}

/// Maximum bytes of a canonical media-type identifier.
pub const MAX_MEDIA_TYPE_BYTES: usize = 255;
/// Maximum bytes of one media-type token (RFC 6838 restricted-name limit).
pub const MAX_MEDIA_TYPE_TOKEN_BYTES: usize = 127;

/// Why a media-type identifier is not canonical.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaTypeError {
    /// Empty identifier.
    Empty,
    /// Identifier or one of its tokens is too long.
    TooLong,
    /// Missing `/subtype` or empty token.
    MissingSubtype,
    /// Parameters such as `;charset=` are not part of the identifier.
    ParametersNotAllowed,
    /// Wildcards are query syntax, not representation identifiers.
    WildcardNotAllowed,
    /// Uppercase, whitespace, or a character outside RFC 6838 restricted names.
    InvalidCharacter,
}

impl core::fmt::Display for MediaTypeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let message = match self {
            Self::Empty => "media type is empty",
            Self::TooLong => "media type exceeds its length bound",
            Self::MissingSubtype => "media type must be type/subtype",
            Self::ParametersNotAllowed => "media type parameters are not allowed",
            Self::WildcardNotAllowed => "media type wildcards are not allowed",
            Self::InvalidCharacter => "media type contains a non-canonical character",
        };
        formatter.write_str(message)
    }
}

impl core::error::Error for MediaTypeError {}

/// Validates the canonical Nagi media-type identifier grammar (ADR-0013).
///
/// Canonical means lowercase `type/subtype`, each token 1-127 bytes of RFC
/// 6838 restricted-name characters starting with a letter or digit, with no
/// parameters, whitespace, or wildcards. Non-canonical spellings are rejected,
/// not normalized, so the identifier that is validated is exactly the one
/// stored and compared. This is a naming rule, not an authority grant.
pub fn validate_media_type(value: &str) -> Result<(), MediaTypeError> {
    if value.is_empty() {
        return Err(MediaTypeError::Empty);
    }
    if value.len() > MAX_MEDIA_TYPE_BYTES {
        return Err(MediaTypeError::TooLong);
    }
    if value.contains(';') {
        return Err(MediaTypeError::ParametersNotAllowed);
    }
    let Some((kind, subtype)) = value.split_once('/') else {
        return Err(MediaTypeError::MissingSubtype);
    };
    validate_media_type_token(kind)?;
    validate_media_type_token(subtype)?;
    if kind == "*" || subtype == "*" {
        return Err(MediaTypeError::WildcardNotAllowed);
    }
    Ok(())
}

fn validate_media_type_token(token: &str) -> Result<(), MediaTypeError> {
    if token.is_empty() {
        return Err(MediaTypeError::MissingSubtype);
    }
    if token.len() > MAX_MEDIA_TYPE_TOKEN_BYTES {
        return Err(MediaTypeError::TooLong);
    }
    if token == "*" {
        return Ok(());
    }
    let mut bytes = token.bytes();
    let first = bytes.next().unwrap_or(0);
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(MediaTypeError::InvalidCharacter);
    }
    if bytes.any(|byte| {
        !(byte.is_ascii_lowercase()
            || byte.is_ascii_digit()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'-' | b'^' | b'_' | b'.' | b'+'
            ))
    }) {
        return Err(MediaTypeError::InvalidCharacter);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        validate_media_type, AppId, AppSessionId, MediaTypeError, PresentationClass,
        PresentationContext,
    };

    #[test]
    fn canonical_media_types_follow_one_grammar() {
        for valid in [
            "text/plain",
            "image/svg+xml",
            "application/vnd.nagi.note-block",
            "application/x.nagi.object-ref",
            "application/a!#$&^_.+-z",
        ] {
            assert_eq!(validate_media_type(valid), Ok(()), "{valid}");
        }
        for (invalid, error) in [
            ("", MediaTypeError::Empty),
            ("text", MediaTypeError::MissingSubtype),
            ("Text/Plain", MediaTypeError::InvalidCharacter),
            ("image/SVG+xml", MediaTypeError::InvalidCharacter),
            (".text/plain", MediaTypeError::InvalidCharacter),
            (
                "text/plain; charset=utf-8",
                MediaTypeError::ParametersNotAllowed,
            ),
            ("text/*", MediaTypeError::WildcardNotAllowed),
        ] {
            assert_eq!(validate_media_type(invalid), Err(error), "{invalid}");
        }
    }

    #[test]
    fn presentation_is_separate_from_logical_application_identity() {
        let context = PresentationContext::compact(320, 200);
        assert_eq!(context.class, PresentationClass::Compact);
        assert_eq!(AppId(7), AppId(7));
        assert_eq!(AppSessionId(9), AppSessionId(9));
    }

    #[test]
    fn identifier_identity_is_stable() {
        assert_eq!(
            AppId::from_identifier(b"com.example.hello-nagi"),
            AppId::from_identifier(b"com.example.hello-nagi")
        );
        assert_ne!(
            AppId::from_identifier(b"com.example.hello-nagi"),
            AppId::from_identifier(b"com.example.other")
        );
    }
}
