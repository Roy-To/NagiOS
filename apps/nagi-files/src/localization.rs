#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Locale {
    EnUs,
    JaJp,
}

impl Locale {
    pub fn parse(tag: &str) -> Self {
        match tag.to_ascii_lowercase().as_str() {
            "ja-jp" | "ja" => Self::JaJp,
            _ => Self::EnUs,
        }
    }
}

pub fn text(locale: Locale, key: &str) -> Option<&'static str> {
    let selected = match locale {
        Locale::EnUs => EN_US,
        Locale::JaJp => JA_JP,
    };
    lookup(selected, key).or_else(|| lookup(EN_US, key))
}

pub fn keys(resource: &str) -> Vec<&str> {
    resource
        .lines()
        .filter_map(|line| line.split_once('=').map(|(key, _)| key.trim()))
        .filter(|key| !key.is_empty() && !key.starts_with('#'))
        .collect()
}

pub const EN_US: &str = include_str!("../locales/en-US.properties");
pub const JA_JP: &str = include_str!("../locales/ja-JP.properties");

fn lookup(resource: &'static str, wanted: &str) -> Option<&'static str> {
    resource.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == wanted).then_some(value.trim())
    })
}
