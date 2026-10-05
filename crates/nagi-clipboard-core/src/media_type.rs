use std::fmt;

/// Canonical media-type grammar and errors, owned by `nagi-model` (ADR-0013)
/// so the SDK manifest and clipboard share one definition.
pub use nagi_model::{MediaTypeError, MAX_MEDIA_TYPE_BYTES};

/// A canonical media-type identifier such as `text/plain` or `image/png`.
///
/// Only the canonical lowercase `type/subtype` form is accepted (see
/// `nagi_model::validate_media_type`). Text representations are always UTF-8,
/// so a `charset` parameter is not needed. Non-canonical spellings are
/// rejected rather than normalized so that the identifier used for
/// authorization is exactly the identifier stored.
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
        nagi_model::validate_media_type(&value)?;
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
