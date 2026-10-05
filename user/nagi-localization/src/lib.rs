#![no_std]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    EnUs,
    JaJp,
}

impl Locale {
    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "en-US" => Some(Self::EnUs),
            "ja-JP" => Some(Self::JaJp),
            _ => None,
        }
    }

    pub const fn code(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::JaJp => "ja-JP",
        }
    }
}

const EN_US: &[u8] = include_bytes!("../locales/en-US.lang");
const JA_JP: &[u8] = include_bytes!("../locales/ja-JP.lang");
const MISSING_TEXT: &str = "Text unavailable.";

/// Resolve a stable English key in the selected locale, falling back to en-US.
/// An unknown key returns safe UI text and is never exposed to the user.
pub fn text(locale: Locale, key: &str) -> &'static str {
    let selected = match locale {
        Locale::EnUs => EN_US,
        Locale::JaJp => JA_JP,
    };
    lookup_with_fallback(selected, EN_US, key).unwrap_or(MISSING_TEXT)
}

fn lookup_with_fallback<'a>(selected: &'a [u8], english: &'a [u8], key: &str) -> Option<&'a str> {
    lookup_resource(selected, key).or_else(|| lookup_resource(english, key))
}

fn lookup_resource<'a>(resource: &'a [u8], key: &str) -> Option<&'a str> {
    if key.is_empty() {
        return None;
    }
    for raw_line in resource.split(|byte| *byte == b'\n') {
        let line = if raw_line.last() == Some(&b'\r') {
            &raw_line[..raw_line.len() - 1]
        } else {
            raw_line
        };
        let Some(separator) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        if &line[..separator] == key.as_bytes() {
            return core::str::from_utf8(&line[separator + 1..]).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{lookup_resource, lookup_with_fallback, text, Locale, EN_US, JA_JP};

    #[test]
    fn parses_only_supported_canonical_locale_codes() {
        assert_eq!(Locale::parse("en-US"), Some(Locale::EnUs));
        assert_eq!(Locale::parse("ja-JP"), Some(Locale::JaJp));
        for invalid in ["en", "EN-us", "ja", "ja-JP-x", "fr-FR", ""] {
            assert_eq!(
                Locale::parse(invalid),
                None,
                "unexpectedly parsed {invalid}"
            );
        }
    }

    #[test]
    fn selected_japanese_resource_returns_utf8_text() {
        assert_eq!(text(Locale::JaJp, "desktop.settings.button"), "設定");
        assert_eq!(
            text(Locale::JaJp, "desktop.settings.option.ja-JP"),
            "日本語"
        );
    }

    #[test]
    fn selected_locale_state_is_translated_in_both_first_party_languages() {
        assert_eq!(
            text(Locale::EnUs, "desktop.settings.option.selected"),
            "Selected"
        );
        assert_eq!(
            text(Locale::JaJp, "desktop.settings.option.selected"),
            "選択中"
        );
    }

    #[test]
    fn lookup_resource_accepts_windows_crlf_line_endings() {
        static CRLF_RESOURCE: &[u8] = b"common.ok=OK\r\n";
        assert_eq!(lookup_resource(CRLF_RESOURCE, "common.ok"), Some("OK"));
    }

    #[test]
    fn english_fallback_is_used_when_a_selected_entry_is_missing() {
        static PARTIAL_JAPANESE: &[u8] = b"common.ok=OK\n";
        static ENGLISH_FIXTURE: &[u8] = b"common.cancel=Cancel\n";
        assert_eq!(
            lookup_with_fallback(PARTIAL_JAPANESE, ENGLISH_FIXTURE, "common.cancel"),
            Some("Cancel")
        );
    }

    #[test]
    fn missing_keys_return_safe_text_without_exposing_the_key() {
        let key = "private.internal.key";
        let value = text(Locale::JaJp, key);
        assert_eq!(value, "Text unavailable.");
        assert!(!value.contains(key));
    }

    #[test]
    fn first_party_catalogs_have_unique_matching_keys() {
        let mut english_lines = EN_US
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty());
        while let Some(line) = english_lines.next() {
            let separator = line
                .iter()
                .position(|byte| *byte == b'=')
                .expect("English key/value");
            let key = core::str::from_utf8(&line[..separator]).expect("ASCII stable key");
            assert!(
                lookup_resource(EN_US, key).is_some(),
                "English catalog has an invalid UTF-8 value for {key}"
            );
            assert!(
                lookup_resource(JA_JP, key).is_some(),
                "Japanese catalog misses {key}"
            );
            assert_eq!(
                english_lines
                    .clone()
                    .filter(|candidate| candidate.split(|byte| *byte == b'=').next()
                        == Some(&line[..separator]))
                    .count(),
                0,
                "duplicate English catalog key {key}"
            );
        }
        let mut japanese_lines = JA_JP
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty());
        while let Some(line) = japanese_lines.next() {
            let separator = line
                .iter()
                .position(|byte| *byte == b'=')
                .expect("Japanese key/value");
            let key = core::str::from_utf8(&line[..separator]).expect("ASCII stable key");
            assert!(
                lookup_resource(EN_US, key).is_some(),
                "English catalog misses {key}"
            );
            assert_eq!(
                japanese_lines
                    .clone()
                    .filter(|candidate| candidate.split(|byte| *byte == b'=').next()
                        == Some(&line[..separator]))
                    .count(),
                0,
                "duplicate Japanese catalog key {key}"
            );
        }
    }
}
