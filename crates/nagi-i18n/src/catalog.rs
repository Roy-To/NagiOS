use core::fmt;
use std::collections::{BTreeMap, BTreeSet};

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::locale::LocaleId;
use crate::template::{parse_template, TemplatePart};

pub const CATALOG_SCHEMA_VERSION: u32 = 1;
pub const ENGLISH_REFERENCE_LOCALE: &str = "en-US";
pub const JAPANESE_LOCALE: &str = "ja-JP";
pub const MISSING_MESSAGE_ID: &str = "system.message_unavailable";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TextDirection {
    Ltr,
    Rtl,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct MessageId(String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MessageIdError(String);

impl MessageId {
    pub fn new(value: impl Into<String>) -> Result<Self, MessageIdError> {
        let value = value.into();
        if !valid_message_id(&value) {
            return Err(MessageIdError(value));
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for MessageIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid stable message ID `{}`", self.0)
    }
}

impl std::error::Error for MessageIdError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogError(String);

impl CatalogError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for CatalogError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationIssue {
    pub code: &'static str,
    pub locale: Option<String>,
    pub message_id: Option<String>,
    pub detail: String,
}

impl fmt::Display for ValidationIssue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.code)?;
        if let Some(locale) = &self.locale {
            write!(formatter, " [{locale}]")?;
        }
        if let Some(message_id) = &self.message_id {
            write!(formatter, " {message_id}")?;
        }
        write!(formatter, ": {}", self.detail)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Catalog {
    schema_version: u32,
    locale: LocaleId,
    script: String,
    direction: TextDirection,
    font_fallback_scripts: Vec<String>,
    messages: BTreeMap<MessageId, String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogWire {
    schema_version: u32,
    locale: String,
    script: String,
    direction: TextDirection,
    font_fallback_scripts: Vec<String>,
    messages: UniqueMessages,
}

#[derive(Default)]
struct UniqueMessages(BTreeMap<String, String>);

impl<'de> Deserialize<'de> for UniqueMessages {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UniqueMessagesVisitor;

        impl<'de> Visitor<'de> for UniqueMessagesVisitor {
            type Value = UniqueMessages;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object with unique message IDs")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut messages = BTreeMap::new();
                while let Some((key, value)) = access.next_entry::<String, String>()? {
                    if messages.insert(key.clone(), value).is_some() {
                        return Err(serde::de::Error::custom(format!(
                            "duplicate message ID `{key}`"
                        )));
                    }
                }
                Ok(UniqueMessages(messages))
            }
        }

        deserializer.deserialize_map(UniqueMessagesVisitor)
    }
}

impl Catalog {
    pub fn parse(json: &str) -> Result<Self, CatalogError> {
        let wire: CatalogWire = serde_json::from_str(json)
            .map_err(|error| CatalogError::new(format!("catalog JSON: {error}")))?;
        let locale =
            LocaleId::parse(&wire.locale).map_err(|error| CatalogError::new(error.to_string()))?;
        if locale.as_str() != wire.locale {
            return Err(CatalogError::new(format!(
                "catalog locale `{}` is not canonical; use `{}`",
                wire.locale,
                locale.as_str()
            )));
        }

        let mut messages = BTreeMap::new();
        for (key, value) in wire.messages.0 {
            let id = MessageId::new(key).map_err(|error| CatalogError::new(error.to_string()))?;
            if value.trim().is_empty() {
                return Err(CatalogError::new(format!(
                    "message `{id}` must not be empty"
                )));
            }
            parse_template(&value).map_err(|error| {
                CatalogError::new(format!(
                    "message `{id}` has malformed placeholders: {error}"
                ))
            })?;
            messages.insert(id, value);
        }

        let catalog = Self {
            schema_version: wire.schema_version,
            locale,
            script: wire.script,
            direction: wire.direction,
            font_fallback_scripts: wire.font_fallback_scripts,
            messages,
        };
        let issues = catalog.validate_metadata();
        if !issues.is_empty() {
            return Err(CatalogError::new(
                issues
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("\n"),
            ));
        }
        Ok(catalog)
    }

    pub fn locale(&self) -> &LocaleId {
        &self.locale
    }

    pub fn script(&self) -> &str {
        &self.script
    }

    pub fn direction(&self) -> TextDirection {
        self.direction
    }

    pub fn font_fallback_scripts(&self) -> &[String] {
        &self.font_fallback_scripts
    }

    pub fn message(&self, id: &MessageId) -> Option<&str> {
        self.messages.get(id).map(String::as_str)
    }

    pub fn message_count(&self) -> usize {
        self.messages.len()
    }

    /// Verify that every message ID in this package resource uses the
    /// namespace declared by its package manifest.
    pub fn validate_namespace(&self, namespace: &str) -> Vec<ValidationIssue> {
        if !valid_message_id(&format!("{namespace}.validation_probe")) {
            return vec![issue(
                "invalid_translation_namespace",
                Some(self.locale.as_str()),
                None,
                format!("`{namespace}` is not a stable message namespace"),
            )];
        }
        let prefix = format!("{namespace}.");
        self.messages
            .keys()
            .filter(|message_id| !message_id.as_str().starts_with(&prefix))
            .map(|message_id| {
                issue(
                    "message_outside_namespace",
                    Some(self.locale.as_str()),
                    Some(message_id.as_str()),
                    format!("message ID must begin with `{prefix}`"),
                )
            })
            .collect()
    }

    pub(crate) fn messages(&self) -> &BTreeMap<MessageId, String> {
        &self.messages
    }

    pub(crate) fn pseudo_from(reference: &Catalog) -> Result<Self, CatalogError> {
        let locale =
            LocaleId::parse("en-XA").map_err(|error| CatalogError::new(error.to_string()))?;
        let mut messages = BTreeMap::new();
        for (id, message) in &reference.messages {
            messages.insert(
                id.clone(),
                crate::pseudo::pseudo_localize(message)
                    .map_err(|error| CatalogError::new(error.to_string()))?,
            );
        }
        Ok(Self {
            schema_version: CATALOG_SCHEMA_VERSION,
            locale,
            script: reference.script.clone(),
            direction: reference.direction,
            font_fallback_scripts: reference.font_fallback_scripts.clone(),
            messages,
        })
    }

    fn validate_metadata(&self) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        if self.schema_version != CATALOG_SCHEMA_VERSION {
            issues.push(issue(
                "unsupported_schema_version",
                Some(self.locale.as_str()),
                None,
                format!(
                    "catalog schema {} is unsupported; expected {}",
                    self.schema_version, CATALOG_SCHEMA_VERSION
                ),
            ));
        }
        if !valid_script_code(&self.script) {
            issues.push(issue(
                "invalid_script_metadata",
                Some(self.locale.as_str()),
                None,
                format!("`{}` is not a four-letter script code", self.script),
            ));
        }
        if let Some(locale_script) = self.locale.script() {
            if locale_script != self.script {
                issues.push(issue(
                    "script_locale_mismatch",
                    Some(self.locale.as_str()),
                    None,
                    format!(
                        "locale script `{locale_script}` disagrees with catalog script `{}`",
                        self.script
                    ),
                ));
            }
        }
        let expected_official_script = match self.locale.as_str() {
            ENGLISH_REFERENCE_LOCALE => Some("Latn"),
            JAPANESE_LOCALE => Some("Jpan"),
            _ => None,
        };
        if let Some(expected) = expected_official_script {
            if self.script != expected || self.direction != TextDirection::Ltr {
                issues.push(issue(
                    "invalid_official_locale_metadata",
                    Some(self.locale.as_str()),
                    None,
                    format!("official locale requires script `{expected}` and `ltr` direction"),
                ));
            }
        }
        if self.font_fallback_scripts.is_empty() {
            issues.push(issue(
                "missing_font_fallback_metadata",
                Some(self.locale.as_str()),
                None,
                "at least one fallback script is required",
            ));
        }
        let mut fallback_scripts = BTreeSet::new();
        for script in &self.font_fallback_scripts {
            if !valid_script_code(script) {
                issues.push(issue(
                    "invalid_font_fallback_script",
                    Some(self.locale.as_str()),
                    None,
                    format!("`{script}` is not a four-letter script code"),
                ));
            } else if !fallback_scripts.insert(script) {
                issues.push(issue(
                    "duplicate_font_fallback_script",
                    Some(self.locale.as_str()),
                    None,
                    format!("`{script}` appears more than once"),
                ));
            }
        }
        if self.font_fallback_scripts.first() != Some(&self.script) {
            issues.push(issue(
                "catalog_script_not_in_fallbacks",
                Some(self.locale.as_str()),
                None,
                format!(
                    "catalog script `{}` must be the first font fallback script",
                    self.script
                ),
            ));
        }
        issues
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogSet {
    catalogs: BTreeMap<LocaleId, Catalog>,
}

impl CatalogSet {
    pub fn new(catalogs: impl IntoIterator<Item = Catalog>) -> Result<Self, CatalogError> {
        let mut by_locale = BTreeMap::new();
        for catalog in catalogs {
            let locale = catalog.locale.clone();
            if by_locale.insert(locale.clone(), catalog).is_some() {
                return Err(CatalogError::new(format!(
                    "duplicate catalog locale `{locale}`"
                )));
            }
        }
        if !by_locale
            .contains_key(&LocaleId::parse(ENGLISH_REFERENCE_LOCALE).expect("valid locale"))
        {
            return Err(CatalogError::new(format!(
                "required reference catalog `{ENGLISH_REFERENCE_LOCALE}` is missing"
            )));
        }
        Ok(Self {
            catalogs: by_locale,
        })
    }

    pub fn bundled() -> Result<Self, CatalogError> {
        let english = Catalog::parse(include_str!("../locales/en-US.json"))?;
        let japanese = Catalog::parse(include_str!("../locales/ja-JP.json"))?;
        Self::new([english, japanese])
    }

    /// Return a copy of this set with application or feature catalogs merged
    /// by locale. Message IDs must be globally unique after namespace prefixing.
    pub fn with_additional_catalogs(
        &self,
        additions: impl IntoIterator<Item = Catalog>,
    ) -> Result<Self, CatalogError> {
        let mut merged = self.clone();
        for addition in additions {
            let locale = addition.locale.clone();
            let Some(existing) = merged.catalogs.get_mut(&locale) else {
                merged.catalogs.insert(locale, addition);
                continue;
            };
            if existing.schema_version != addition.schema_version
                || existing.script != addition.script
                || existing.direction != addition.direction
            {
                return Err(CatalogError::new(format!(
                    "catalog metadata conflicts while merging locale `{}`",
                    addition.locale
                )));
            }
            for (id, message) in addition.messages {
                if existing.messages.contains_key(&id) {
                    return Err(CatalogError::new(format!(
                        "duplicate message ID `{id}` while merging locale `{}`; use package namespaces",
                        addition.locale
                    )));
                }
                existing.messages.insert(id, message);
            }
            for script in addition.font_fallback_scripts {
                if !existing.font_fallback_scripts.contains(&script) {
                    existing.font_fallback_scripts.push(script);
                }
            }
        }
        Ok(merged)
    }

    pub fn with_pseudo_locale(&self) -> Result<Self, CatalogError> {
        let english_locale = LocaleId::parse(ENGLISH_REFERENCE_LOCALE).expect("valid locale");
        let english = self
            .catalogs
            .get(&english_locale)
            .expect("CatalogSet always has en-US");
        let mut catalogs: Vec<Catalog> = self.catalogs.values().cloned().collect();
        catalogs.push(Catalog::pseudo_from(english)?);
        Self::new(catalogs)
    }

    pub fn get(&self, locale: &LocaleId) -> Option<&Catalog> {
        self.catalogs.get(locale)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Catalog> {
        self.catalogs.values()
    }

    pub fn validate_parity(&self) -> Vec<ValidationIssue> {
        let english_locale = LocaleId::parse(ENGLISH_REFERENCE_LOCALE).expect("valid locale");
        let Some(reference) = self.catalogs.get(&english_locale) else {
            return vec![issue(
                "missing_reference_locale",
                Some(ENGLISH_REFERENCE_LOCALE),
                None,
                "English reference catalog is required",
            )];
        };
        let mut issues = Vec::new();
        for catalog in self
            .catalogs
            .values()
            .filter(|catalog| catalog.locale != english_locale)
        {
            let locale = catalog.locale.as_str();
            for (id, reference_text) in reference.messages() {
                match catalog.messages().get(id) {
                    None => issues.push(issue(
                        "missing_translation",
                        Some(locale),
                        Some(id.as_str()),
                        "message is present in en-US but missing from this catalog",
                    )),
                    Some(translated_text) => {
                        let reference_arguments = template_arguments(reference_text);
                        let translated_arguments = template_arguments(translated_text);
                        if reference_arguments != translated_arguments {
                            issues.push(issue(
                                "placeholder_mismatch",
                                Some(locale),
                                Some(id.as_str()),
                                format!(
                                    "reference placeholders {reference_arguments:?} differ from localized placeholders {translated_arguments:?}"
                                ),
                            ));
                        }
                    }
                }
            }
            for id in catalog.messages().keys() {
                if !reference.messages().contains_key(id) {
                    issues.push(issue(
                        "unreferenced_translation",
                        Some(locale),
                        Some(id.as_str()),
                        "message is absent from the en-US reference catalog",
                    ));
                }
            }
        }
        issues
    }

    pub fn validate_first_class_locales(&self) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        for required in [ENGLISH_REFERENCE_LOCALE, JAPANESE_LOCALE] {
            let locale = LocaleId::parse(required).expect("valid official locale");
            if !self.catalogs.contains_key(&locale) {
                issues.push(issue(
                    "missing_first_class_locale",
                    Some(required),
                    None,
                    "official Nagi 0.1 locale catalog is required",
                ));
            }
        }
        issues
    }
}

pub fn validate_bundled_catalogs() -> Result<(), Vec<ValidationIssue>> {
    let catalogs = CatalogSet::bundled()
        .map_err(|error| vec![issue("invalid_catalog", None, None, error.to_string())])?;
    let mut issues = catalogs.validate_first_class_locales();
    issues.extend(catalogs.validate_parity());
    if issues.is_empty() {
        Ok(())
    } else {
        Err(issues)
    }
}

fn valid_message_id(value: &str) -> bool {
    let segments: Vec<&str> = value.split('.').collect();
    segments.len() >= 2
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && segment.as_bytes()[0].is_ascii_lowercase()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
}

fn valid_script_code(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 4
        && bytes[0].is_ascii_uppercase()
        && bytes[1..].iter().all(u8::is_ascii_lowercase)
}

fn template_arguments(value: &str) -> BTreeSet<String> {
    parse_template(value)
        .map(|parts| {
            parts
                .into_iter()
                .filter_map(|part| match part {
                    TemplatePart::Argument(name) => Some(name),
                    TemplatePart::Literal(_) => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn issue(
    code: &'static str,
    locale: Option<&str>,
    message_id: Option<&str>,
    detail: impl Into<String>,
) -> ValidationIssue {
    ValidationIssue {
        code,
        locale: locale.map(str::to_owned),
        message_id: message_id.map(str::to_owned),
        detail: detail.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Catalog, CatalogSet, MessageId};

    const VALID: &str = r#"{
      "schema_version": 1,
      "locale": "en-US",
      "script": "Latn",
      "direction": "ltr",
      "font_fallback_scripts": ["Latn"],
      "messages": {"common.save": "Save {name}"}
    }"#;

    #[test]
    fn parses_english_and_japanese_resources_as_real_catalogs() {
        let catalogs = CatalogSet::bundled().unwrap();
        let english = catalogs
            .get(&super::LocaleId::parse("en-US").unwrap())
            .unwrap();
        let japanese = catalogs
            .get(&super::LocaleId::parse("ja-JP").unwrap())
            .unwrap();
        let save = MessageId::new("common.save").unwrap();
        assert_eq!(english.message(&save), Some("Save"));
        assert_eq!(japanese.message(&save), Some("保存"));
        assert_eq!(japanese.script(), "Jpan");
        assert_eq!(japanese.font_fallback_scripts(), ["Jpan", "Hani", "Latn"]);
    }

    #[test]
    fn rejects_duplicate_ids_instead_of_silently_overwriting() {
        let duplicate = VALID.replace(
            "\"common.save\": \"Save {name}\"",
            "\"common.save\": \"Save {name}\", \"common.save\": \"Overwrite\"",
        );
        let error = Catalog::parse(&duplicate).unwrap_err().to_string();
        assert!(error.contains("duplicate message ID"));
    }

    #[test]
    fn validates_declared_package_namespaces() {
        let catalog = Catalog::parse(
            r#"{"schema_version":1,"locale":"en-US","script":"Latn","direction":"ltr","font_fallback_scripts":["Latn"],"messages":{"app.files.rename":"Rename"}}"#,
        )
        .unwrap();
        assert!(catalog.validate_namespace("app.files").is_empty());
        assert_eq!(
            catalog.validate_namespace("app.notes")[0].code,
            "message_outside_namespace"
        );
        assert_eq!(
            catalog.validate_namespace("App Files")[0].code,
            "invalid_translation_namespace"
        );
    }

    #[test]
    fn rejects_invalid_locale_schema_keys_and_placeholders() {
        assert!(Catalog::parse(&VALID.replace("en-US", "en_us")).is_err());
        assert!(
            Catalog::parse(&VALID.replace("\"schema_version\": 1", "\"schema_version\": 2"))
                .is_err()
        );
        assert!(Catalog::parse(&VALID.replace("common.save", "Common Save")).is_err());
        assert!(Catalog::parse(&VALID.replace("Save {name}", "Save {name")).is_err());
        assert!(
            Catalog::parse(&VALID.replace("\"script\": \"Latn\"", "\"script\": \"bad\"")).is_err()
        );
        assert!(Catalog::parse(
            &VALID.replace("\"direction\": \"ltr\"", "\"direction\": \"sideways\"")
        )
        .is_err());
        assert!(Catalog::parse(&VALID.replace(
            "\"font_fallback_scripts\": [\"Latn\"]",
            "\"font_fallback_scripts\": []"
        ))
        .is_err());
        let wrong_japanese_script = VALID.replace("en-US", "ja-JP");
        assert!(Catalog::parse(&wrong_japanese_script).is_err());
    }

    #[test]
    fn detects_missing_extra_and_mismatched_translations() {
        let english = Catalog::parse(VALID).unwrap();
        let japanese_json = VALID
            .replace("en-US", "ja-JP")
            .replace("Latn", "Jpan")
            .replace("Save {name}", "{file}を保存")
            .replace(
                "\"common.save\": \"{file}を保存\"",
                "\"new.key\": \"追加\", \"common.save\": \"{file}を保存\"",
            );
        let japanese = Catalog::parse(&japanese_json).unwrap();
        let catalogs = CatalogSet::new([english, japanese]).unwrap();
        let issues = catalogs.validate_parity();
        assert!(issues
            .iter()
            .any(|issue| issue.code == "placeholder_mismatch"));
        assert!(issues
            .iter()
            .any(|issue| issue.code == "unreferenced_translation"));
        assert!(!issues
            .iter()
            .any(|issue| issue.code == "missing_translation"));

        let missing = japanese_json.replace(", \"common.save\": \"{file}を保存\"", "");
        let japanese = Catalog::parse(&missing).unwrap();
        let catalogs = CatalogSet::new([Catalog::parse(VALID).unwrap(), japanese]).unwrap();
        assert!(catalogs
            .validate_parity()
            .iter()
            .any(|issue| issue.code == "missing_translation"));
    }

    #[test]
    fn rejects_duplicate_catalog_locales() {
        let error = CatalogSet::new([
            Catalog::parse(VALID).unwrap(),
            Catalog::parse(VALID).unwrap(),
        ])
        .unwrap_err();
        assert!(error.to_string().contains("duplicate catalog locale"));
    }
}
