//! Nagi's portable user-space localization foundation.
//!
//! Internal APIs and message IDs stay in English. Display language is selected
//! independently from regional formatting; all bundled data is UTF-8 and all
//! lookup/fallback behavior is deterministic.

mod catalog;
mod formatting;
mod locale;
mod localizer;
mod pseudo;
mod template;

pub use catalog::{
    validate_bundled_catalogs, Catalog, CatalogError, CatalogSet, MessageId, MessageIdError,
    TextDirection, ValidationIssue, CATALOG_SCHEMA_VERSION, ENGLISH_REFERENCE_LOCALE,
    JAPANESE_LOCALE, MISSING_MESSAGE_ID,
};
pub use formatting::{
    Collator, Currency, Date, DateTime, FormatError, Formatter, NumberOptions, Time, Unit,
};
pub use locale::{LocaleContext, LocaleId, LocaleParseError};
pub use localizer::{
    ArgumentError, DiagnosticKind, DiagnosticSink, LocalizationDiagnostic, Localizer, LookupError,
    MessageArgs, NoopDiagnosticSink,
};
pub use pseudo::{pseudo_localize, PseudoLocaleError};

#[cfg(test)]
mod tests {
    use super::{validate_bundled_catalogs, CatalogSet, LocaleId};

    #[test]
    fn bundled_english_and_japanese_catalogs_pass_first_class_parity() {
        validate_bundled_catalogs().unwrap();
        assert_eq!(CatalogSet::bundled().unwrap().iter().count(), 2);
    }

    #[test]
    fn future_locale_tags_can_preserve_script_and_region_identity() {
        let traditional_chinese = LocaleId::parse("zh-Hant-TW").unwrap();
        assert_eq!(traditional_chinese.language(), "zh");
        assert_eq!(traditional_chinese.script(), Some("Hant"));
        assert_eq!(traditional_chinese.region(), Some("TW"));
    }
}
