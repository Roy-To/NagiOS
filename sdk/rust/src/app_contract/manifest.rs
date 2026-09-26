use super::{
    AppError, AppIdentity, AppStateVersion, CapabilityRequest, ContractVersion, DisplayName,
    ErrorCode, IntentDeclaration, APP_MANIFEST_SCHEMA_VERSION, EN_US, JA_JP, SDK_CONTRACT_VERSION,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntrypointKind {
    Native,
    Portable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppEntrypoint<'a> {
    pub kind: EntrypointKind,
    /// Package-relative artifact locator. This is never application identity.
    pub target: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceReference<'a> {
    pub id: &'a str,
    pub uri: &'a str,
    pub media_type: Option<&'a str>,
}

/// Future service declaration metadata. It describes a capability for a later
/// host to interpret; it does not start a background process in this SDK.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackgroundServiceDeclaration<'a> {
    pub id: &'a str,
    pub entrypoint: &'a str,
    pub activation: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StateCompatibility {
    pub current: AppStateVersion,
    pub minimum_readable: AppStateVersion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppManifestContract<'a> {
    pub schema_version: u16,
    pub sdk_contract_version: ContractVersion,
    pub identity: AppIdentity<'a>,
    pub display_name: DisplayName<'a>,
    pub supported_locales: &'a [&'a str],
    pub icon: Option<&'a str>,
    pub entrypoint: AppEntrypoint<'a>,
    pub resources: &'a [ResourceReference<'a>],
    pub intents: &'a [IntentDeclaration<'a>],
    pub requested_capabilities: &'a [CapabilityRequest<'a>],
    pub background_services: &'a [BackgroundServiceDeclaration<'a>],
    pub state: StateCompatibility,
}

impl AppManifestContract<'_> {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.schema_version != APP_MANIFEST_SCHEMA_VERSION
            || self.identity.validate().is_err()
            || self.supported_locales.is_empty()
            || !self.supported_locales.contains(&EN_US)
            || self.display_name.en_us.trim().is_empty()
            || self
                .display_name
                .ja_jp
                .is_some_and(|name| name.trim().is_empty())
            || self.display_name.ja_jp.is_some() && !self.supported_locales.contains(&JA_JP)
            || self.icon.is_some_and(|uri| !valid_resource_uri(uri))
            || !valid_package_relative_target(self.entrypoint.target)
            || self.state.current.0 == 0
            || self.state.minimum_readable.0 == 0
            || self.state.minimum_readable > self.state.current
        {
            return Err(AppError::new(ErrorCode::InvalidManifest));
        }
        if !self
            .sdk_contract_version
            .is_compatible_with(SDK_CONTRACT_VERSION)
        {
            return Err(AppError::new(ErrorCode::IncompatibleContractVersion));
        }
        for (index, locale) in self.supported_locales.iter().enumerate() {
            if !valid_locale_tag(locale)
                || self.supported_locales[..index].contains(locale)
                || *locale == JA_JP && self.display_name.ja_jp.is_none()
            {
                return Err(AppError::new(ErrorCode::InvalidManifest));
            }
        }
        for (index, intent) in self.intents.iter().enumerate() {
            if self.intents[..index]
                .iter()
                .any(|previous| previous.id == intent.id)
                || intent.version == 0
                || !valid_symbol(intent.id)
                || !valid_payload_type(intent.payload_type)
                || intent.route_id.is_some_and(|route| !valid_symbol(route))
            {
                return Err(AppError::new(ErrorCode::InvalidManifest));
            }
        }
        for (index, request) in self.requested_capabilities.iter().enumerate() {
            if self.requested_capabilities[..index]
                .iter()
                .any(|previous| previous.id == request.id)
                || !valid_symbol(request.id)
                || request.purpose_key.is_some_and(|key| !valid_symbol(key))
            {
                return Err(AppError::new(ErrorCode::InvalidManifest));
            }
        }
        for (index, resource) in self.resources.iter().enumerate() {
            if self.resources[..index]
                .iter()
                .any(|previous| previous.id == resource.id)
                || !valid_symbol(resource.id)
                || !valid_resource_uri(resource.uri)
                || resource
                    .media_type
                    .is_some_and(|media_type| !valid_media_type(media_type))
            {
                return Err(AppError::new(ErrorCode::InvalidManifest));
            }
        }
        for (index, service) in self.background_services.iter().enumerate() {
            if !valid_symbol(service.id)
                || self.background_services[..index]
                    .iter()
                    .any(|previous| previous.id == service.id)
                || !valid_package_relative_target(service.entrypoint)
                || !matches!(
                    service.activation,
                    "deferred" | "on-demand" | "system-event"
                )
            {
                return Err(AppError::new(ErrorCode::InvalidManifest));
            }
        }
        Ok(())
    }
}

pub fn valid_locale_tag(value: &str) -> bool {
    let mut parts = value.split('-');
    let Some(language) = parts.next() else {
        return false;
    };
    if !(2..=3).contains(&language.len()) || !language.bytes().all(|b| b.is_ascii_lowercase()) {
        return false;
    }
    parts.all(|part| {
        (2..=8).contains(&part.len()) && part.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

fn valid_package_relative_target(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn valid_resource_uri(value: &str) -> bool {
    let Some(path) = value.strip_prefix("appres://") else {
        return false;
    };
    !path.is_empty()
        && !path.starts_with('/')
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'/'))
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

fn valid_symbol(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
}

fn valid_media_type(value: &str) -> bool {
    let Some((kind, subtype)) = value.split_once('/') else {
        return false;
    };
    let valid_token = |token: &str| {
        !token.is_empty()
            && token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-'))
    };
    valid_token(kind) && valid_token(subtype)
}

fn valid_payload_type(value: &str) -> bool {
    let Some((name, version)) = value.split_once('@') else {
        return false;
    };
    valid_symbol(name) && !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::{
        valid_locale_tag, AppEntrypoint, AppManifestContract, BackgroundServiceDeclaration,
        EntrypointKind, StateCompatibility,
    };
    use crate::app_contract::{
        AppIdentity, AppOrigin, AppStateVersion, CapabilityRequest, ContractVersion, DisplayName,
        ErrorCode, IntentDeclaration, EN_US, JA_JP, SDK_CONTRACT_VERSION,
    };

    fn manifest<'a>(locales: &'a [&'a str]) -> AppManifestContract<'a> {
        AppManifestContract {
            schema_version: 1,
            sdk_contract_version: SDK_CONTRACT_VERSION,
            identity: AppIdentity::new(
                "com.nagi.sdk-fixture.notes",
                "1.0.0",
                AppOrigin::FirstParty,
                Some("com.nagi"),
            )
            .unwrap(),
            display_name: DisplayName {
                en_us: "Notes Fixture",
                ja_jp: locales.contains(&JA_JP).then_some("メモfixture"),
            },
            supported_locales: locales,
            icon: Some("appres://icons/notes.svg"),
            entrypoint: AppEntrypoint {
                kind: EntrypointKind::Native,
                target: "bin/notes.napp",
            },
            resources: &[],
            intents: &[IntentDeclaration {
                id: "notes.open-note",
                version: 1,
                payload_type: "nagi.note-reference@1",
                route_id: Some("open-note"),
            }],
            requested_capabilities: &[CapabilityRequest {
                id: "storage.read",
                purpose_key: Some("notes.read_purpose"),
            }],
            background_services: &[BackgroundServiceDeclaration {
                id: "notes.indexer",
                entrypoint: "bin/indexer.napp",
                activation: "deferred",
            }],
            state: StateCompatibility {
                current: AppStateVersion(2),
                minimum_readable: AppStateVersion(1),
            },
        }
    }

    #[test]
    fn manifest_contract_validates_locales_capabilities_and_future_services() {
        assert_eq!(manifest(&[EN_US, JA_JP]).validate(), Ok(()));
        assert!(valid_locale_tag("fr-CA"));
        assert!(!valid_locale_tag("EN-us"));
    }

    #[test]
    fn manifest_contract_rejects_unsafe_entrypoints_and_bad_state_versions() {
        let mut invalid = manifest(&[EN_US]);
        invalid.entrypoint.target = "../outside";
        assert_eq!(
            invalid.validate().unwrap_err().code,
            ErrorCode::InvalidManifest
        );
        let mut invalid = manifest(&[EN_US]);
        invalid.state.minimum_readable = AppStateVersion(3);
        assert_eq!(
            invalid.validate().unwrap_err().code,
            ErrorCode::InvalidManifest
        );
    }

    #[test]
    fn manifest_contract_rejects_incompatible_sdk_major_version() {
        let mut invalid = manifest(&[EN_US]);
        invalid.sdk_contract_version = ContractVersion { major: 2, minor: 0 };
        assert_eq!(
            invalid.validate().unwrap_err().code,
            ErrorCode::IncompatibleContractVersion
        );
    }

    #[test]
    fn resource_and_entrypoint_targets_reject_whitespace_controls_and_unicode() {
        let invalid_uris = [
            "appres://icons/a b.svg",
            "appres://icons/a\u{0000}b.svg",
            "appres://icons/日本語.svg",
            "appres://icons//notes.svg",
        ];
        for uri in invalid_uris {
            let mut invalid = manifest(&[EN_US]);
            let resources = [super::ResourceReference {
                id: "notes.icon",
                uri,
                media_type: Some("image/svg+xml"),
            }];
            invalid.resources = &resources;
            assert_eq!(
                invalid.validate().unwrap_err().code,
                ErrorCode::InvalidManifest,
                "accepted URI {uri:?}"
            );
        }

        for target in ["bin/a b.napp", "bin/a\u{0000}b.napp", "bin/日本語.napp"] {
            let mut invalid = manifest(&[EN_US]);
            invalid.entrypoint.target = target;
            assert_eq!(
                invalid.validate().unwrap_err().code,
                ErrorCode::InvalidManifest,
                "accepted target {target:?}"
            );
        }
    }

    #[test]
    fn semantic_validation_rejects_duplicate_declaration_ids() {
        let mut invalid = manifest(&[EN_US]);
        let intents = [
            IntentDeclaration {
                id: "notes.open-note",
                version: 1,
                payload_type: "nagi.note-reference@1",
                route_id: Some("open-note"),
            },
            IntentDeclaration {
                id: "notes.open-note",
                version: 2,
                payload_type: "nagi.note-reference@2",
                route_id: Some("open-note-v2"),
            },
        ];
        invalid.intents = &intents;
        assert_eq!(
            invalid.validate().unwrap_err().code,
            ErrorCode::InvalidManifest
        );

        let resources = [
            super::ResourceReference {
                id: "notes.icon",
                uri: "appres://icons/notes.svg",
                media_type: Some("image/svg+xml"),
            },
            super::ResourceReference {
                id: "notes.icon",
                uri: "appres://icons/notes-alt.svg",
                media_type: Some("image/svg+xml"),
            },
        ];
        invalid.intents = &[];
        invalid.resources = &resources;
        assert_eq!(
            invalid.validate().unwrap_err().code,
            ErrorCode::InvalidManifest
        );
    }
}
