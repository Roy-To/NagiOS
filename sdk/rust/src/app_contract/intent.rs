use crate::AppId;

use super::{AppError, CorrelationId, ErrorCode};

pub const MAX_INTENT_PAYLOAD_BYTES: usize = 1_048_576;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntentDeclaration<'a> {
    pub id: &'a str,
    pub version: u16,
    pub payload_type: &'a str,
    /// Route identifier from a `nagi://<app-id>/<route>` deep link.
    pub route_id: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeepLink<'a> {
    pub app_identifier: &'a str,
    pub route_id: &'a str,
}

impl<'a> DeepLink<'a> {
    /// Parse the payload-free route form `nagi://<stable-app-id>/<route-id>`.
    /// Object IDs and user data belong in the typed intent payload, not URI text.
    pub fn parse(uri: &'a str) -> Result<Self, AppError> {
        let remainder = uri
            .strip_prefix("nagi://")
            .ok_or(AppError::new(ErrorCode::InvalidRoute))?;
        if remainder.contains(['?', '#', '%', '@', ':']) {
            return Err(AppError::new(ErrorCode::InvalidRoute));
        }
        let (app_identifier, route_id) = remainder
            .split_once('/')
            .ok_or(AppError::new(ErrorCode::InvalidRoute))?;
        if !super::is_valid_app_identifier(app_identifier) || !valid_route_id(route_id) {
            return Err(AppError::new(ErrorCode::InvalidRoute));
        }
        Ok(Self {
            app_identifier,
            route_id,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Intent<'a> {
    pub id: &'a str,
    pub version: u16,
    pub target_app: Option<AppId>,
    pub source_app: AppId,
    pub correlation_id: CorrelationId,
    pub payload_type: &'a str,
    pub payload: &'a [u8],
    pub deep_link: Option<DeepLink<'a>>,
}

pub fn validate_intent(
    intent: &Intent<'_>,
    target_app_identifier: &str,
    declarations: &[IntentDeclaration<'_>],
) -> Result<(), AppError> {
    if !super::is_valid_app_identifier(target_app_identifier)
        || !valid_symbol(intent.id)
        || !valid_payload_type(intent.payload_type)
        || intent.payload.len() > MAX_INTENT_PAYLOAD_BYTES
    {
        return Err(AppError::new(ErrorCode::InvalidManifest));
    }
    let target_app_id = AppId::from_identifier(target_app_identifier.as_bytes());
    if intent
        .target_app
        .is_some_and(|target| target != target_app_id)
    {
        return Err(AppError::new(ErrorCode::UnsupportedIntent));
    }
    let declaration = declarations
        .iter()
        .find(|declaration| declaration.id == intent.id)
        .ok_or(AppError::new(ErrorCode::UnsupportedIntent))?;
    if declaration.version == 0
        || !valid_symbol(declaration.id)
        || !valid_payload_type(declaration.payload_type)
        || declaration
            .route_id
            .is_some_and(|route| !valid_route_id(route))
    {
        return Err(AppError::new(ErrorCode::InvalidManifest));
    }
    if declaration.version != intent.version || declaration.payload_type != intent.payload_type {
        return Err(AppError::new(ErrorCode::UnsupportedIntent));
    }
    match (intent.deep_link, declaration.route_id) {
        (Some(link), Some(route_id))
            if link.app_identifier == target_app_identifier && link.route_id == route_id => {}
        (None, None) => {}
        (None, Some(_)) | (Some(_), None) => {
            return Err(AppError::new(ErrorCode::UnsupportedIntent));
        }
        _ => return Err(AppError::new(ErrorCode::InvalidRoute)),
    }
    Ok(())
}

fn valid_route_id(value: &str) -> bool {
    valid_symbol(value) && !value.contains("..")
}

fn valid_symbol(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

fn valid_payload_type(value: &str) -> bool {
    let Some((name, version)) = value.split_once('@') else {
        return false;
    };
    valid_symbol(name) && !version.is_empty() && version.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::{validate_intent, DeepLink, Intent, IntentDeclaration};
    use crate::app_contract::{AppError, CorrelationId, ErrorCode};
    use crate::AppId;

    const APP: &str = "com.nagi.sdk-fixture.files";
    const OPEN: IntentDeclaration<'_> = IntentDeclaration {
        id: "files.open-resource",
        version: 1,
        payload_type: "nagi.resource-reference@1",
        route_id: Some("open-resource"),
    };

    fn intent() -> Intent<'static> {
        Intent {
            id: OPEN.id,
            version: OPEN.version,
            target_app: Some(AppId::from_identifier(APP.as_bytes())),
            source_app: AppId(9),
            correlation_id: CorrelationId([5; 16]),
            payload_type: OPEN.payload_type,
            payload: b"resource:opaque-id",
            deep_link: Some(
                DeepLink::parse("nagi://com.nagi.sdk-fixture.files/open-resource").unwrap(),
            ),
        }
    }

    #[test]
    fn declared_intent_and_payload_free_deep_link_validate() {
        assert_eq!(validate_intent(&intent(), APP, &[OPEN]), Ok(()));
    }

    #[test]
    fn declared_intent_without_a_route_does_not_require_a_deep_link() {
        let declaration = IntentDeclaration {
            route_id: None,
            ..OPEN
        };
        let mut request = intent();
        request.deep_link = None;
        assert_eq!(validate_intent(&request, APP, &[declaration]), Ok(()));
    }

    #[test]
    fn unsupported_intent_and_version_are_rejected() {
        let mut unsupported = intent();
        unsupported.id = "notes.open-note";
        assert_eq!(
            validate_intent(&unsupported, APP, &[OPEN]),
            Err(AppError::new(ErrorCode::UnsupportedIntent))
        );
        let mut bad_version = intent();
        bad_version.version = 2;
        assert_eq!(
            validate_intent(&bad_version, APP, &[OPEN]),
            Err(AppError::new(ErrorCode::UnsupportedIntent))
        );
    }

    #[test]
    fn route_rejects_external_schemes_payloads_and_path_traversal() {
        for uri in [
            "https://example.com/open",
            "nagi://com.nagi.files/../open",
            "nagi://com.nagi.files/open?object=secret",
            "nagi://com.nagi.files/open%2fsecret",
        ] {
            assert_eq!(
                DeepLink::parse(uri),
                Err(AppError::new(ErrorCode::InvalidRoute))
            );
        }
    }

    #[test]
    fn mismatched_route_or_target_is_rejected() {
        let mut wrong_route = intent();
        wrong_route.deep_link =
            Some(DeepLink::parse("nagi://com.nagi.sdk-fixture.files/show-hidden").unwrap());
        assert_eq!(
            validate_intent(&wrong_route, APP, &[OPEN]),
            Err(AppError::new(ErrorCode::InvalidRoute))
        );
        let mut wrong_target = intent();
        wrong_target.target_app = Some(AppId(88));
        assert_eq!(
            validate_intent(&wrong_target, APP, &[OPEN]),
            Err(AppError::new(ErrorCode::UnsupportedIntent))
        );
    }

    #[test]
    fn invalid_route_declaration_is_rejected_before_dispatch() {
        let invalid = IntentDeclaration {
            route_id: Some("../escape"),
            ..OPEN
        };
        assert_eq!(
            validate_intent(&intent(), APP, &[invalid]),
            Err(AppError::new(ErrorCode::InvalidManifest))
        );
    }
}
