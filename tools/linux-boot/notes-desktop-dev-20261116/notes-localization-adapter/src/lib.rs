//! A small bridge for migrating the Notes labels to `nagi_i18n` without
//! changing their existing IDs, locale values, translations, or fallback text.

use core::fmt;
use std::error::Error;

use nagi_i18n::{
    Catalog, CatalogError, CatalogSet, LocaleContext, LocaleId, LocaleParseError, Localizer,
    LookupError, MessageArgs, MessageId, MessageIdError, NoopDiagnosticSink, TextDirection,
    CATALOG_SCHEMA_VERSION,
};
use nagi_localization::Locale;

const NOTES_TITLE: &str = "desktop.notes.title";
const NOTES_CONTENT: &str = "desktop.notes.content";

/// Localization adapter for the existing Notes title and empty-state label.
///
/// Create one instance for the active legacy locale and recreate it when that
/// locale changes. Unknown IDs continue through the legacy resolver, preserving
/// its English fallback behavior.
pub struct NotesLocalization {
    legacy_locale: Locale,
    localizer: Localizer,
}

/// Errors produced while mapping the existing Notes translations into the
/// `nagi_i18n` public API.
#[derive(Debug)]
pub enum AdapterError {
    Locale(LocaleParseError),
    Catalog(CatalogError),
    BundledLocaleMissing(String),
    MessageId(MessageIdError),
    Render(LookupError),
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locale(error) => write!(formatter, "legacy locale mapping failed: {error}"),
            Self::Catalog(error) => write!(formatter, "Notes catalog construction failed: {error}"),
            Self::BundledLocaleMissing(locale) => {
                write!(formatter, "bundled catalog `{locale}` is missing")
            }
            Self::MessageId(error) => {
                write!(formatter, "invalid existing Notes message ID: {error}")
            }
            Self::Render(error) => write!(formatter, "Notes message rendering failed: {error}"),
        }
    }
}

impl Error for AdapterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Locale(error) => Some(error),
            Self::Catalog(error) => Some(error),
            Self::BundledLocaleMissing(_) => None,
            Self::MessageId(error) => Some(error),
            Self::Render(error) => Some(error),
        }
    }
}

impl From<LocaleParseError> for AdapterError {
    fn from(error: LocaleParseError) -> Self {
        Self::Locale(error)
    }
}

impl From<CatalogError> for AdapterError {
    fn from(error: CatalogError) -> Self {
        Self::Catalog(error)
    }
}

impl From<MessageIdError> for AdapterError {
    fn from(error: MessageIdError) -> Self {
        Self::MessageId(error)
    }
}

impl From<LookupError> for AdapterError {
    fn from(error: LookupError) -> Self {
        Self::Render(error)
    }
}

impl NotesLocalization {
    /// Build a new-API localizer for one of the existing `nagi_localization`
    /// locale values, using its current Notes translations as supplemental data.
    pub fn new(legacy_locale: Locale) -> Result<Self, AdapterError> {
        let locale_id = LocaleId::parse(legacy_locale.code())?;
        let base_catalogs = CatalogSet::bundled()?;
        let notes_catalog = supplemental_notes_catalog(legacy_locale, &base_catalogs)?;
        let catalogs = base_catalogs.with_additional_catalogs([notes_catalog])?;
        let localizer = Localizer::new(
            catalogs,
            LocaleContext::new(locale_id.clone(), locale_id),
            NoopDiagnosticSink,
        );

        Ok(Self {
            legacy_locale,
            localizer,
        })
    }

    /// Resolve an existing Notes ID through `nagi_i18n`; other IDs retain the
    /// legacy resolver and its safe unknown-key fallback.
    pub fn text(&self, message_id: &str) -> Result<String, AdapterError> {
        if message_id != NOTES_TITLE && message_id != NOTES_CONTENT {
            return Ok(nagi_localization::text(self.legacy_locale, message_id).to_owned());
        }

        let id = MessageId::new(message_id.to_owned())?;
        Ok(self.localizer.render(&id, &MessageArgs::new())?)
    }
}

fn supplemental_notes_catalog(
    legacy_locale: Locale,
    base_catalogs: &CatalogSet,
) -> Result<Catalog, AdapterError> {
    let locale_id = LocaleId::parse(legacy_locale.code())?;
    let base_catalog = base_catalogs
        .get(&locale_id)
        .ok_or_else(|| AdapterError::BundledLocaleMissing(locale_id.as_str().to_owned()))?;
    let direction = match base_catalog.direction() {
        TextDirection::Ltr => "ltr",
        TextDirection::Rtl => "rtl",
    };
    let font_fallback_scripts = base_catalog
        .font_fallback_scripts()
        .iter()
        .map(|script| json_string(script))
        .collect::<Vec<_>>()
        .join(", ");
    let title = json_string(nagi_localization::text(legacy_locale, NOTES_TITLE));
    let content = json_string(nagi_localization::text(legacy_locale, NOTES_CONTENT));
    let json = format!(
        "{{\"schema_version\":{CATALOG_SCHEMA_VERSION},\"locale\":{},\"script\":{},\"direction\":{},\"font_fallback_scripts\":[{}],\"messages\":{{{}:{},{}:{}}}}}",
        json_string(locale_id.as_str()),
        json_string(base_catalog.script()),
        json_string(direction),
        font_fallback_scripts,
        json_string(NOTES_TITLE),
        title,
        json_string(NOTES_CONTENT),
        content,
    );
    Ok(Catalog::parse(&json)?)
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character <= '\u{001f}' => {
                escaped.push_str(&format!("\\u{:04x}", character as u32));
            }
            character => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}
