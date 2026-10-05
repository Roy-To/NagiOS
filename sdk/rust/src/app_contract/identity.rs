use crate::AppId;

use super::{AppError, ErrorCode};

pub const EN_US: &str = "en-US";
pub const JA_JP: &str = "ja-JP";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppOrigin {
    FirstParty,
    ThirdParty,
}

/// Locale-neutral app identity. The identifier is canonical; a display name,
/// filesystem locator, session, and execution instance are never identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppIdentity<'a> {
    app_id: AppId,
    identifier: &'a str,
    version: &'a str,
    origin: AppOrigin,
    publisher_id: Option<&'a str>,
}

impl<'a> AppIdentity<'a> {
    pub fn new(
        identifier: &'a str,
        version: &'a str,
        origin: AppOrigin,
        publisher_id: Option<&'a str>,
    ) -> Result<Self, AppError> {
        if !is_valid_app_identifier(identifier)
            || publisher_id.is_some_and(|publisher| !is_valid_app_identifier(publisher))
            || !is_valid_app_version(version)
            || origin == AppOrigin::FirstParty && publisher_id.is_none()
        {
            return Err(AppError::new(ErrorCode::InvalidManifest));
        }
        Ok(Self {
            app_id: AppId::from_identifier(identifier.as_bytes()),
            identifier,
            version,
            origin,
            publisher_id,
        })
    }

    /// Recheck invariants at public contract boundaries. The fields are
    /// private so ordinary callers can only create identities with `new`.
    pub fn validate(&self) -> Result<(), AppError> {
        if !is_valid_app_identifier(self.identifier)
            || self.app_id != AppId::from_identifier(self.identifier.as_bytes())
            || !is_valid_app_version(self.version)
            || self
                .publisher_id
                .is_some_and(|publisher| !is_valid_app_identifier(publisher))
            || self.origin == AppOrigin::FirstParty && self.publisher_id.is_none()
        {
            return Err(AppError::new(ErrorCode::InvalidManifest));
        }
        Ok(())
    }

    pub const fn app_id(&self) -> AppId {
        self.app_id
    }

    pub const fn identifier(&self) -> &'a str {
        self.identifier
    }

    pub const fn version(&self) -> &'a str {
        self.version
    }

    pub const fn origin(&self) -> AppOrigin {
        self.origin
    }

    pub const fn publisher_id(&self) -> Option<&'a str> {
        self.publisher_id
    }
}

/// Validate the app-facing semantic version syntax used by manifest v1.
pub fn is_valid_app_version(value: &str) -> bool {
    let (without_build, build) = match value.split_once('+') {
        Some((left, right)) if !right.is_empty() && !right.contains('+') => (left, Some(right)),
        Some(_) => return false,
        None => (value, None),
    };
    let (core, pre_release) = match without_build.split_once('-') {
        Some((core, suffix)) if !suffix.is_empty() => (core, Some(suffix)),
        Some(_) => return false,
        None => (without_build, None),
    };
    let mut components = core.split('.');
    let numeric_core = components.clone().count() == 3
        && components.all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'))
        });
    let identifiers_valid = |suffix: &str, reject_numeric_leading_zero: bool| {
        suffix.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && (!reject_numeric_leading_zero
                    || !part.bytes().all(|byte| byte.is_ascii_digit())
                    || part == "0"
                    || !part.starts_with('0'))
        })
    };
    numeric_core
        && pre_release.is_none_or(|suffix| identifiers_valid(suffix, true))
        && build.is_none_or(|suffix| identifiers_valid(suffix, false))
}

/// Restrict public IDs to stable reverse-domain ASCII identifiers.
pub fn is_valid_app_identifier(value: &str) -> bool {
    let mut labels = value.split('.');
    let Some(first) = labels.next() else {
        return false;
    };
    let mut count = 1;
    if !valid_label(first) {
        return false;
    }
    for label in labels {
        if !valid_label(label) {
            return false;
        }
        count += 1;
    }
    count >= 2
}

fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisplayName<'a> {
    pub en_us: &'a str,
    pub ja_jp: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LocaleResolution<'a> {
    pub text: &'a str,
    pub resolved_locale: &'static str,
    pub used_fallback: bool,
}

impl<'a> DisplayName<'a> {
    pub fn resolve(&self, requested: &str, supported_locales: &[&str]) -> LocaleResolution<'a> {
        if requested == EN_US {
            return LocaleResolution {
                text: self.en_us,
                resolved_locale: EN_US,
                used_fallback: false,
            };
        }
        if requested == JA_JP && supported_locales.contains(&JA_JP) && self.ja_jp.is_some() {
            return LocaleResolution {
                text: self.ja_jp.expect("checked Japanese display name"),
                resolved_locale: JA_JP,
                used_fallback: false,
            };
        }
        LocaleResolution {
            text: self.en_us,
            resolved_locale: EN_US,
            used_fallback: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CorrelationId(pub [u8; 16]);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestId(pub [u8; 16]);

#[cfg(test)]
mod tests {
    use super::{
        is_valid_app_identifier, is_valid_app_version, AppIdentity, AppOrigin, DisplayName, EN_US,
        JA_JP,
    };

    #[test]
    fn stable_identity_is_independent_of_localized_display_name() {
        let first = AppIdentity::new(
            "com.nagi.notes",
            "1.2.0",
            AppOrigin::FirstParty,
            Some("com.nagi"),
        )
        .unwrap();
        let second = AppIdentity::new(
            "com.nagi.notes",
            "1.2.0",
            AppOrigin::FirstParty,
            Some("com.nagi"),
        )
        .unwrap();
        let names = DisplayName {
            en_us: "Notes",
            ja_jp: Some("メモ"),
        };
        assert_eq!(first, second);
        assert_eq!(names.resolve(EN_US, &[EN_US, JA_JP]).text, "Notes");
        assert_eq!(names.resolve(JA_JP, &[EN_US, JA_JP]).text, "メモ");
        assert_eq!(
            names.resolve("fr-FR", &[EN_US, JA_JP]).resolved_locale,
            EN_US
        );
        assert!(names.resolve("fr-FR", &[EN_US, JA_JP]).used_fallback);
    }

    #[test]
    fn identifier_validation_is_locale_and_path_independent() {
        assert!(is_valid_app_identifier("com.example.notes"));
        assert!(is_valid_app_identifier("org.nagi.app-v2"));
        assert!(!is_valid_app_identifier("../notes"));
        assert!(!is_valid_app_identifier("Com.Nagi.Notes"));
        assert!(!is_valid_app_identifier("single"));
    }

    #[test]
    fn publisher_identity_is_optional_for_third_party_apps() {
        let app =
            AppIdentity::new("com.example.tool", "0.1.0", AppOrigin::ThirdParty, None).unwrap();
        assert_eq!(app.publisher_id(), None);
        assert_eq!(app.validate(), Ok(()));
    }

    #[test]
    fn directly_forged_identity_invariants_are_rejected() {
        let wrong_app_id_and_publisher = AppIdentity {
            app_id: crate::AppId(8),
            identifier: "com.example.tool",
            version: "1.0.0",
            origin: AppOrigin::ThirdParty,
            publisher_id: Some("com.example.publisher"),
        };
        assert_eq!(
            wrong_app_id_and_publisher.validate(),
            Err(crate::app_contract::AppError::new(
                crate::app_contract::ErrorCode::InvalidManifest
            ))
        );

        let wrong_app_id = AppIdentity {
            app_id: crate::AppId(8),
            identifier: "com.example.tool",
            version: "1.0.0",
            origin: AppOrigin::ThirdParty,
            publisher_id: Some("not a valid publisher"),
        };
        assert_eq!(
            wrong_app_id.validate(),
            Err(crate::app_contract::AppError::new(
                crate::app_contract::ErrorCode::InvalidManifest
            ))
        );

        let wrong_publisher = AppIdentity {
            app_id: crate::AppId::from_identifier(b"com.example.tool"),
            identifier: "com.example.tool",
            version: "1.0.0",
            origin: AppOrigin::ThirdParty,
            publisher_id: Some("not a valid publisher"),
        };
        assert_eq!(
            wrong_publisher.validate(),
            Err(crate::app_contract::AppError::new(
                crate::app_contract::ErrorCode::InvalidManifest
            ))
        );
    }

    #[test]
    fn app_version_uses_semver_without_accepting_ambiguous_numeric_forms() {
        assert!(is_valid_app_version("1.2.3"));
        assert!(is_valid_app_version("1.2.3-beta.1+build.4"));
        assert!(!is_valid_app_version("01.2.3"));
        assert!(!is_valid_app_version("1.2.3-01"));
        assert!(!is_valid_app_version("1.2.3-"));
    }
}
