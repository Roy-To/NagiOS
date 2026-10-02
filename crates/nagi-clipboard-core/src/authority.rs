use nagi_model::{AppId, AppSessionId, ExecutionInstanceId};

use crate::media_type::MediaType;

/// Caller identity established by the trusted transport boundary.
///
/// Only the future service-IPC / Capability adapter, which authenticates the
/// channel peer, may construct this. It is passed to every operation as a
/// separate argument and is never derived from clipboard content, metadata,
/// or [`crate::ClaimedOrigin`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallerContext {
    app: AppId,
    app_session: AppSessionId,
    execution_instance: ExecutionInstanceId,
}

impl CallerContext {
    /// Records the authenticated caller. Must only be called by the trusted
    /// boundary after authenticating the transport peer.
    pub const fn from_trusted_boundary(
        app: AppId,
        app_session: AppSessionId,
        execution_instance: ExecutionInstanceId,
    ) -> Self {
        Self {
            app,
            app_session,
            execution_instance,
        }
    }

    /// Authenticated application.
    pub const fn app(&self) -> AppId {
        self.app
    }

    /// Authenticated application session.
    pub const fn app_session(&self) -> AppSessionId {
        self.app_session
    }

    /// Authenticated execution instance.
    pub const fn execution_instance(&self) -> ExecutionInstanceId {
        self.execution_instance
    }
}

/// The writer as authenticated by the trusted boundary at write time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedOrigin {
    /// Authenticated writing application.
    pub app: AppId,
    /// Authenticated writing application session.
    pub app_session: AppSessionId,
    /// Authenticated writing execution instance.
    pub execution_instance: ExecutionInstanceId,
}

impl From<&CallerContext> for VerifiedOrigin {
    fn from(caller: &CallerContext) -> Self {
        Self {
            app: caller.app,
            app_session: caller.app_session,
            execution_instance: caller.execution_instance,
        }
    }
}

/// Operations that pass through the authorization seam.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardOperation<'a> {
    /// Replace clipboard content.
    Write,
    /// Clear clipboard content.
    Clear,
    /// Read the current generation only.
    ReadGeneration,
    /// List available formats and untrusted metadata.
    ReadFormats,
    /// Read one representation's payload. Also consulted per representation
    /// while listing, so restricted formats are neither readable nor listed.
    ReadRepresentation {
        /// Requested media type.
        media_type: &'a MediaType,
    },
}

impl ClipboardOperation<'_> {
    /// Permission identifier the future Capability adapter should check.
    ///
    /// `clipboard.read` is the canonical permission named by the 0.2
    /// specification. `clipboard.write` is a CLIP-01 *proposal* recorded in
    /// the registration proposal; it is not registered by this crate.
    pub const fn proposed_permission(&self) -> &'static str {
        match self {
            Self::Write | Self::Clear => "clipboard.write",
            Self::ReadGeneration | Self::ReadFormats | Self::ReadRepresentation { .. } => {
                "clipboard.read"
            }
        }
    }
}

/// Everything the authorizer may see. It intentionally has no field for
/// claimed origin or metadata, so untrusted content cannot influence a
/// decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorizationRequest<'a> {
    /// Authenticated caller.
    pub caller: &'a CallerContext,
    /// Requested operation.
    pub operation: ClipboardOperation<'a>,
}

/// Authorizer verdict.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationDecision {
    /// The operation may proceed.
    Allow,
    /// The operation is denied.
    Deny,
    /// The policy provider is unavailable; treated as denial.
    Unavailable,
}

/// Injected authorization boundary for future Capability integration.
///
/// This crate ships no permissive default. Any decision other than `Allow`
/// fails closed with no state change.
pub trait ClipboardAuthorizer {
    /// Decide one request.
    fn authorize(&mut self, request: &AuthorizationRequest<'_>) -> AuthorizationDecision;
}

impl<F> ClipboardAuthorizer for F
where
    F: FnMut(&AuthorizationRequest<'_>) -> AuthorizationDecision,
{
    fn authorize(&mut self, request: &AuthorizationRequest<'_>) -> AuthorizationDecision {
        self(request)
    }
}

/// Authorizer that denies everything; the safe default for wiring.
#[derive(Clone, Copy, Debug, Default)]
pub struct DenyAllAuthorizer;

impl ClipboardAuthorizer for DenyAllAuthorizer {
    fn authorize(&mut self, _request: &AuthorizationRequest<'_>) -> AuthorizationDecision {
        AuthorizationDecision::Deny
    }
}
