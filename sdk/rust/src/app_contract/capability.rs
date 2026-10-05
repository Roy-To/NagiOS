use crate::AppId;

use super::{AppError, ErrorCode};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityRequest<'a> {
    /// Stable permission/capability identifier owned by the capability system.
    pub id: &'a str,
    /// Optional localization key explaining the app's declared use.
    pub purpose_key: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityDecision {
    NotRequested,
    Resolved,
    Denied,
}

/// Adapter to the separately owned capability and permission service.
/// Implementations own policy and handle delivery; this trait grants nothing.
pub trait CapabilityResolver {
    fn resolve(
        &mut self,
        app_id: AppId,
        requested: &[CapabilityRequest<'_>],
    ) -> Result<CapabilityDecision, AppError>;
}

pub fn resolve_capabilities(
    resolver: &mut impl CapabilityResolver,
    app_id: AppId,
    requested: &[CapabilityRequest<'_>],
) -> Result<CapabilityDecision, AppError> {
    if requested
        .iter()
        .any(|request| !valid_capability_id(request.id))
    {
        return Err(AppError::new(ErrorCode::InvalidManifest));
    }
    if requested.is_empty() {
        return Ok(CapabilityDecision::NotRequested);
    }
    match resolver.resolve(app_id, requested)? {
        CapabilityDecision::Resolved => Ok(CapabilityDecision::Resolved),
        CapabilityDecision::Denied => Err(AppError::new(ErrorCode::PermissionDenied)),
        CapabilityDecision::NotRequested => Err(AppError::new(ErrorCode::InvalidManifest)),
    }
}

pub(super) fn valid_capability_id(value: &str) -> bool {
    if value.is_empty() || value.len() > 128 {
        return false;
    }
    let mut segments = value.split('.');
    let Some(first) = segments.next() else {
        return false;
    };
    valid_namespace_segment(first)
        && segments.next().is_some_and(valid_namespace_segment)
        && segments.all(valid_namespace_segment)
}

fn valid_namespace_segment(segment: &str) -> bool {
    let mut bytes = segment.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

#[cfg(test)]
mod tests {
    use super::{resolve_capabilities, CapabilityDecision, CapabilityRequest, CapabilityResolver};
    use crate::app_contract::{AppError, ErrorCode};
    use crate::AppId;

    struct Resolver(CapabilityDecision);

    impl CapabilityResolver for Resolver {
        fn resolve(
            &mut self,
            _app_id: AppId,
            _requested: &[CapabilityRequest<'_>],
        ) -> Result<CapabilityDecision, AppError> {
            Ok(self.0)
        }
    }

    #[test]
    fn capability_request_is_handed_to_external_resolver() {
        let requests = [CapabilityRequest {
            id: "storage.read",
            purpose_key: Some("files.read_purpose"),
        }];
        let result = resolve_capabilities(
            &mut Resolver(CapabilityDecision::Resolved),
            AppId(42),
            &requests,
        );
        assert_eq!(result, Ok(CapabilityDecision::Resolved));
    }

    #[test]
    fn denial_is_propagated_and_no_policy_is_implemented_here() {
        let requests = [CapabilityRequest {
            id: "storage.write",
            purpose_key: None,
        }];
        let result = resolve_capabilities(
            &mut Resolver(CapabilityDecision::Denied),
            AppId(42),
            &requests,
        );
        assert_eq!(result, Err(AppError::new(ErrorCode::PermissionDenied)));
    }

    #[test]
    fn empty_request_list_skips_resolver() {
        assert_eq!(
            resolve_capabilities(&mut Resolver(CapabilityDecision::Denied), AppId(42), &[]),
            Ok(CapabilityDecision::NotRequested)
        );
    }

    #[test]
    fn capability_requests_require_canonical_dot_separated_namespaces() {
        for id in [
            "files",
            "1files.read",
            "files.read_only",
            "files..read",
            "files.-read",
            "Files.read",
        ] {
            let requests = [CapabilityRequest {
                id,
                purpose_key: None,
            }];
            assert_eq!(
                resolve_capabilities(
                    &mut Resolver(CapabilityDecision::Resolved),
                    AppId(42),
                    &requests
                ),
                Err(AppError::new(ErrorCode::InvalidManifest)),
                "accepted non-canonical capability ID {id:?}"
            );
        }
    }
}
