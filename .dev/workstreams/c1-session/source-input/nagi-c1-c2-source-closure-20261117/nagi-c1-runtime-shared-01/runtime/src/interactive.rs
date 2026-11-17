//! C1's in-process interactive Session context, issued by the real SDK resolver.
//! This is separate from a system service's OS credential/endpoint evidence.
//! It has no raw-ID constructor, serialization, service-principal conversion or
//! operation grant. C3 supplies the trusted caller; operation policy still runs.

use consumer::session_binding::{AdmissionFailure, SessionBinding};
use identity::{
    CurrentIdentityResolver, IdentityAccessContext, ResolvedIdentity, TrustedCallerContext,
};
use svc::CancellationToken;

/// Owns a revocable SDK observation and borrows the runtime caller.
/// This is not an operation lease; operations must still reauthorize.
/// User/Profile/Session metadata must never replace this context at admission.
/// Dropping it releases the observation; it does not log out the SDK session.
pub struct InteractiveSessionContext<'caller> {
    binding: SessionBinding<'caller>,
}

impl<'caller> InteractiveSessionContext<'caller> {
    pub(crate) fn resolve(
        resolver: &mut impl CurrentIdentityResolver,
        caller: &'caller dyn TrustedCallerContext,
        cancellation: CancellationToken,
    ) -> Result<Self, AdmissionFailure> {
        SessionBinding::resolve(resolver, caller, cancellation).map(|binding| Self { binding })
    }

    /// Rechecks caller pair, cancellation, expiry and issuer/session revocation.
    /// This is an admission check, not an atomic dispatch or consent decision.
    pub fn check(&self) -> Result<(), AdmissionFailure> {
        self.binding.check()
    }

    /// Checked SDK metadata. The IDs remain observations, not bearer authority.
    pub fn identity(&self) -> Result<&ResolvedIdentity, AdmissionFailure> {
        self.binding.identity()
    }

    /// Use only with the real SDK storage adapter and ProfileRootAuthorizer,
    /// which recheck the live caller/session and authorize each operation.
    pub fn access_context(&self) -> Result<IdentityAccessContext<'_>, AdmissionFailure> {
        self.binding.access_context()
    }
}
