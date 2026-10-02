use std::fmt;

/// Maximum bytes of a `type/subtype` identifier (RFC 6838: 127 per token).
pub const MAX_MEDIA_TYPE_BYTES: usize = 255;
const MAX_TOKEN_BYTES: usize = 127;

/// A canonical media-type identifier such as `text/plain` or `image/png`.
///
/// Only the canonical lowercase `type/subtype` form is accepted: RFC 6838
/// restricted-name characters, no parameters, no whitespace. Text
/// representations are always UTF-8, so a `charset` parameter is not needed.
/// Non-canonical spellings are rejected rather than normalized so that the
/// identifier used for authorization is exactly the identifier stored.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MediaType(String);

impl MediaType {
    /// UTF-8 plain text.
    pub const TEXT_PLAIN: &'static str = "text/plain";
    /// UTF-8 HTML fragment.
    pub const TEXT_HTML: &'static str = "text/html";
    /// UTF-8 URI list.
    pub const TEXT_URI_LIST: &'static str = "text/uri-list";
    /// PNG image.
    pub const IMAGE_PNG: &'static str = "image/png";

    /// Validates and wraps a media-type identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, MediaTypeError> {
        let value = value.into();
        validate(&value)?;
        Ok(Self(value))
    }

    /// The canonical identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The top-level type, e.g. `text`.
    pub fn top_level(&self) -> &str {
        self.0.split_once('/').map_or("", |(kind, _)| kind)
    }

    /// Whether this is a `text/*` type, whose payload must be UTF-8 text.
    pub fn is_text(&self) -> bool {
        self.top_level() == "text"
    }
}

impl fmt::Display for MediaType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn validate(value: &str) -> Result<(), MediaTypeError> {
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
    validate_token(kind)?;
    validate_token(subtype)?;
    if kind == "*" || subtype == "*" {
        return Err(MediaTypeError::WildcardNotAllowed);
    }
    Ok(())
}

fn validate_token(token: &str) -> Result<(), MediaTypeError> {
    if token.is_empty() {
        return Err(MediaTypeError::MissingSubtype);
    }
    if token.len() > MAX_TOKEN_BYTES {
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

/// Why a media-type identifier was rejected.
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

impl fmt::Display for MediaTypeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
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

impl std::error::Error for MediaTypeError {}
