use serde::{Deserialize, Serialize};
use std::fmt;

use crate::model::{HandlerRef, JobId, JobPriority};

/// An identity asserted by a trusted IPC/session adapter.
///
/// Constructing this value is not itself proof of authority. Runtime callers
/// must only create it from an authenticated transport context.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct CallerRef {
    principal: PrincipalRef,
    profile: Option<OwnerProfileRef>,
}

impl CallerRef {
    pub fn from_authenticated_adapter(
        principal: PrincipalRef,
        profile: Option<OwnerProfileRef>,
    ) -> Self {
        Self { principal, profile }
    }

    pub fn principal(&self) -> &PrincipalRef {
        &self.principal
    }

    pub fn profile(&self) -> Option<&OwnerProfileRef> {
        self.profile.as_ref()
    }
}

/// Stable owner metadata. It is not an authorization token.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JobOwner {
    principal: PrincipalRef,
    profile: Option<OwnerProfileRef>,
}

impl JobOwner {
    pub fn principal(&self) -> &PrincipalRef {
        &self.principal
    }

    pub fn profile(&self) -> Option<&OwnerProfileRef> {
        self.profile.as_ref()
    }

    pub fn from_authenticated_adapter(
        principal: PrincipalRef,
        profile: Option<OwnerProfileRef>,
    ) -> Self {
        Self { principal, profile }
    }
}

/// Opaque principal identity mapped by the integration adapter to the
/// capability-permissions contract.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PrincipalRef(String);

impl PrincipalRef {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthorizationError> {
        let value = value.into();
        validate_reference(&value, "principal")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Optional user/profile namespace. It never grants access by itself.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OwnerProfileRef(String);

impl OwnerProfileRef {
    pub fn new(value: impl Into<String>) -> Result<Self, AuthorizationError> {
        let value = value.into();
        validate_reference(&value, "profile")?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum JobAction {
    Enqueue,
    List,
    Inspect,
    Cancel,
    Pause,
    Resume,
    Recover,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorizationError {
    code: AuthorizationErrorCode,
}

impl AuthorizationError {
    pub const fn denied() -> Self {
        Self {
            code: AuthorizationErrorCode::Denied,
        }
    }

    pub const fn unavailable() -> Self {
        Self {
            code: AuthorizationErrorCode::Unavailable,
        }
    }

    const fn invalid_reference() -> Self {
        Self {
            code: AuthorizationErrorCode::InvalidReference,
        }
    }

    pub const fn code(&self) -> AuthorizationErrorCode {
        self.code
    }
}

impl fmt::Display for AuthorizationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.code {
            AuthorizationErrorCode::Denied => "job authorization denied",
            AuthorizationErrorCode::Unavailable => "job authorization adapter unavailable",
            AuthorizationErrorCode::InvalidReference => {
                "principal or profile reference is malformed"
            }
        })
    }
}

impl std::error::Error for AuthorizationError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationErrorCode {
    Denied,
    Unavailable,
    InvalidReference,
}

/// Adapter boundary for fresh, operation-specific authorization.
///
/// Implementations must resolve `CallerRef` from authenticated transport
/// state, authorize every user operation, and re-check handler authority on
/// every start/retry/recovery. Persisted owners are metadata only. The test
/// crate supplies fakes; this crate intentionally has no permissive default.
pub trait JobAuthorization: Send + Sync {
    fn owner_for(&self, caller: &CallerRef) -> Result<JobOwner, AuthorizationError>;

    fn authorize_user(
        &self,
        caller: &CallerRef,
        owner: &JobOwner,
        action: JobAction,
        job_id: Option<&JobId>,
    ) -> Result<(), AuthorizationError>;

    fn authorize_priority(
        &self,
        caller: &CallerRef,
        owner: &JobOwner,
        handler: &HandlerRef,
        priority: JobPriority,
    ) -> Result<(), AuthorizationError>;

    fn authorize_handler(
        &self,
        owner: &JobOwner,
        handler: &HandlerRef,
    ) -> Result<(), AuthorizationError>;
}

fn validate_reference(value: &str, _label: &'static str) -> Result<(), AuthorizationError> {
    if value.is_empty()
        || value.len() > 128
        || !value.as_bytes()[0].is_ascii_alphanumeric()
        || value.as_bytes().iter().any(|byte| {
            !(byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b':' | b'-'))
        })
    {
        return Err(AuthorizationError::invalid_reference());
    }
    Ok(())
}
