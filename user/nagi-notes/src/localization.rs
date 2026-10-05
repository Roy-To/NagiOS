use std::collections::BTreeMap;

const EN_US: &str = include_str!("../locales/en-US.properties");
const JA_JP: &str = include_str!("../locales/ja-JP.properties");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Locale {
    EnUs,
    JaJp,
}

impl Locale {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "en-US" | "en-us" | "en" => Some(Self::EnUs),
            "ja-JP" | "ja-jp" | "ja" => Some(Self::JaJp),
            _ => None,
        }
    }

    pub const fn tag(self) -> &'static str {
        match self {
            Self::EnUs => "en-US",
            Self::JaJp => "ja-JP",
        }
    }
}

pub struct Localizer {
    locale: Locale,
    en_us: BTreeMap<&'static str, &'static str>,
    ja_jp: BTreeMap<&'static str, &'static str>,
}

impl Localizer {
    pub fn new(locale: Locale) -> Self {
        Self {
            locale,
            en_us: parse_catalog(EN_US),
            ja_jp: parse_catalog(JA_JP),
        }
    }

    pub fn locale(&self) -> Locale {
        self.locale
    }

    pub fn set_locale(&mut self, locale: Locale) {
        self.locale = locale;
    }

    /// Resolve selected locale first, then en-US. Unknown keys resolve to a
    /// localized generic message rather than leaking an internal key to UI.
    pub fn text(&self, key: &str) -> &str {
        let selected = match self.locale {
            Locale::EnUs => self.en_us.get(key),
            Locale::JaJp => self.ja_jp.get(key),
        };
        selected
            .copied()
            .or_else(|| self.en_us.get(key).copied())
            .unwrap_or(match self.locale {
                Locale::EnUs => "Text unavailable",
                Locale::JaJp => "表示できません",
            })
    }

    pub fn required_keys() -> &'static [&'static str] {
        &[
            "app.title",
            "app.host_preview",
            "prompt.command",
            "prompt.document",
            "empty.notes",
            "empty.note",
            "state.dirty",
            "state.saved",
            "state.saving",
            "state.save_failed",
            "action.help",
            "action.quick_title",
            "message.created",
            "message.opened",
            "message.saved",
            "message.deleted",
            "message.restored",
            "message.closed",
            "message.language",
            "message.host_preview",
            "message.search_empty",
            "message.search_results",
            "error.not_found",
            "error.no_open_note",
            "error.invalid_id",
            "error.unsaved_close",
            "error.invalid_command",
            "error.preview_storage",
            "error.title_required",
        ]
    }

    pub fn catalog_complete(locale: Locale) -> bool {
        let source = match locale {
            Locale::EnUs => EN_US,
            Locale::JaJp => JA_JP,
        };
        let catalog = parse_catalog(source);
        Self::required_keys()
            .iter()
            .all(|key| catalog.contains_key(key))
    }
}

fn parse_catalog(input: &'static str) -> BTreeMap<&'static str, &'static str> {
    input
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            line.split_once('=')
                .map(|(key, value)| (key.trim(), value.trim()))
        })
        .collect()
}
