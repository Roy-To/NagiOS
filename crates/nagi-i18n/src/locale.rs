use core::fmt;
use core::str::FromStr;

/// A normalized language tag containing language, optional script, region,
/// and BCP 47 variant subtags. Nagi 0.1 intentionally does not accept
/// extensions or private-use subtags.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct LocaleId {
    language: String,
    script: Option<String>,
    region: Option<String>,
    variants: Vec<String>,
    canonical: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocaleParseError {
    input: String,
    reason: &'static str,
}

impl LocaleParseError {
    fn new(input: &str, reason: &'static str) -> Self {
        Self {
            input: input.to_owned(),
            reason,
        }
    }
}

impl fmt::Display for LocaleParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid locale `{}`: {}",
            self.input, self.reason
        )
    }
}

impl std::error::Error for LocaleParseError {}

impl LocaleId {
    pub fn parse(input: &str) -> Result<Self, LocaleParseError> {
        if input.is_empty() || input.trim() != input {
            return Err(LocaleParseError::new(input, "empty or padded tag"));
        }
        let normalized = input.replace('_', "-");
        let parts: Vec<&str> = normalized.split('-').collect();
        let language = parts[0];
        if !(2..=8).contains(&language.len())
            || !language.bytes().all(|byte| byte.is_ascii_alphabetic())
        {
            return Err(LocaleParseError::new(
                input,
                "language must contain 2-8 ASCII letters",
            ));
        }

        let mut script = None;
        let mut region = None;
        let mut variants = Vec::new();
        let mut stage = 0_u8;
        for &part in parts.iter().skip(1) {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
                return Err(LocaleParseError::new(
                    input,
                    "subtags must be non-empty ASCII alphanumeric",
                ));
            }
            if part.len() == 1 {
                return Err(LocaleParseError::new(
                    input,
                    "extensions and private-use subtags are not supported in Nagi 0.1",
                ));
            }
            if part.len() == 4
                && part.bytes().all(|byte| byte.is_ascii_alphabetic())
                && script.is_none()
                && stage == 0
            {
                let mut chars = part.chars();
                let first = chars
                    .next()
                    .expect("four-character script")
                    .to_ascii_uppercase();
                let rest: String = chars.map(|ch| ch.to_ascii_lowercase()).collect();
                script = Some(format!("{first}{rest}"));
                stage = 1;
            } else if (part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic())
                || part.len() == 3 && part.bytes().all(|byte| byte.is_ascii_digit()))
                && region.is_none()
                && stage <= 1
            {
                region = Some(part.to_ascii_uppercase());
                stage = 2;
            } else if (5..=8).contains(&part.len())
                || part.len() == 4 && part.as_bytes()[0].is_ascii_digit()
            {
                let variant = part.to_ascii_lowercase();
                if variants.contains(&variant) {
                    return Err(LocaleParseError::new(input, "duplicate variant subtag"));
                }
                variants.push(variant);
                stage = 3;
            } else {
                return Err(LocaleParseError::new(
                    input,
                    "unsupported or out-of-order subtag",
                ));
            }
        }

        let language = language.to_ascii_lowercase();
        let canonical = Self::compose(&language, script.as_deref(), region.as_deref(), &variants);
        Ok(Self {
            language,
            script,
            region,
            variants,
            canonical,
        })
    }

    pub fn as_str(&self) -> &str {
        &self.canonical
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn script(&self) -> Option<&str> {
        self.script.as_deref()
    }

    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// Return progressively broader tags, preserving script before region
    /// and removing variants first. The caller decides which catalog is the
    /// product-defined fallback for a bare language.
    pub(crate) fn fallback_candidates(&self) -> Vec<Self> {
        let mut ordered = Vec::new();
        let mut append = |candidate: Self| {
            if !ordered.contains(&candidate) {
                ordered.push(candidate);
            }
        };
        let mut variants = self.variants.clone();
        append(self.clone());
        while !variants.is_empty() {
            variants.pop();
            append(Self::from_parts(
                &self.language,
                self.script.as_deref(),
                self.region.as_deref(),
                &variants,
            ));
        }
        if self.region.is_some() {
            append(Self::from_parts(
                &self.language,
                self.script.as_deref(),
                None,
                &[],
            ));
        }
        append(Self::from_parts(&self.language, None, None, &[]));
        ordered
    }

    fn from_parts(
        language: &str,
        script: Option<&str>,
        region: Option<&str>,
        variants: &[String],
    ) -> Self {
        let canonical = Self::compose(language, script, region, variants);
        Self {
            language: language.to_owned(),
            script: script.map(str::to_owned),
            region: region.map(str::to_owned),
            variants: variants.to_vec(),
            canonical,
        }
    }

    fn compose(
        language: &str,
        script: Option<&str>,
        region: Option<&str>,
        variants: &[String],
    ) -> String {
        let mut tag = language.to_owned();
        if let Some(script) = script {
            tag.push('-');
            tag.push_str(script);
        }
        if let Some(region) = region {
            tag.push('-');
            tag.push_str(region);
        }
        for variant in variants {
            tag.push('-');
            tag.push_str(variant);
        }
        tag
    }
}

impl FromStr for LocaleId {
    type Err = LocaleParseError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl fmt::Display for LocaleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Presentation language and regional formatting are deliberately separate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocaleContext {
    pub system_language: LocaleId,
    pub region: LocaleId,
}

impl LocaleContext {
    pub fn new(system_language: LocaleId, region: LocaleId) -> Self {
        Self {
            system_language,
            region,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::LocaleId;

    #[test]
    fn normalizes_language_script_region_and_underscore_separator() {
        assert_eq!(LocaleId::parse("JA_jp").unwrap().as_str(), "ja-JP");
        assert_eq!(
            LocaleId::parse("zh-hANT-tw").unwrap().as_str(),
            "zh-Hant-TW"
        );
        assert_eq!(LocaleId::parse("es-419").unwrap().as_str(), "es-419");
    }

    #[test]
    fn rejects_malformed_or_unsupported_tags() {
        for tag in ["", " ja-JP", "j", "ja--JP", "ja-JP-u-ca-japanese", "日本語"] {
            assert!(LocaleId::parse(tag).is_err(), "accepted {tag:?}");
        }
        for tag in ["en-US-Latn", "en-1901-US", "en-1901-1901"] {
            assert!(
                LocaleId::parse(tag).is_err(),
                "accepted out-of-order {tag:?}"
            );
        }
    }

    #[test]
    fn fallback_candidates_broaden_deterministically() {
        let candidates = LocaleId::parse("zh-Hant-TW").unwrap().fallback_candidates();
        let tags: Vec<&str> = candidates.iter().map(LocaleId::as_str).collect();
        assert_eq!(tags, ["zh-Hant-TW", "zh-Hant", "zh"]);
    }
}
