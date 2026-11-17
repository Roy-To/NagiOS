//! Consumer wiring for an actual service-issued observation and borrowed caller.
//! This does not authenticate credentials, grant consent, or construct principals.

use identity::{
    CallerContextId, CurrentIdentityResolver, IdentityAccessContext, PrincipalId, ResolutionError,
    ResolvedIdentity, TrustedCallerContext,
};
use svc::{CancellationToken, RequestEnvelope, ServiceClient, ServiceFuture};

struct BorrowedCaller<'a>(&'a dyn TrustedCallerContext);

impl TrustedCallerContext for BorrowedCaller<'_> {
    fn principal_id(&self) -> &PrincipalId {
        self.0.principal_id()
    }

    fn context_id(&self) -> &CallerContextId {
        self.0.context_id()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdmissionFailure {
    Cancelled,
    CallerMismatch,
    StaleIdentity,
    Resolution(ResolutionError),
}

/// Owns the observation, not the issuing service. The runtime retains that
/// service and supplies a trusted caller that outlives this binding.
///
/// The shared cancellation token is supplied by the runtime. Cancellation is
/// explicit and cooperative; dropping the binding does not cancel other work
/// or end the Identity session. Issuer loss is checked on each new admission.
///
/// A caller supplied by value cannot be destroyed while the binding is used:
/// ```compile_fail,E0505
/// use identity::{ResolvedIdentity, TrustedCallerContext};
/// use identity_svc_consumer_fixture::session_binding::SessionBinding;
/// use svc::CancellationToken;
/// fn invalid(identity: ResolvedIdentity, caller: impl TrustedCallerContext) {
///     let binding = SessionBinding::from_resolved(identity, &caller, CancellationToken::new()).unwrap();
///     drop(caller);
///     binding.check().unwrap();
/// }
/// ```
pub struct SessionBinding<'caller> {
    identity: ResolvedIdentity,
    caller: BorrowedCaller<'caller>,
    cancellation: CancellationToken,
}

impl<'caller> SessionBinding<'caller> {
    /// Accepts an opaque SDK observation, never raw principal/session strings.
    /// The observation is not proof of credential verification or consent.
    pub fn from_resolved(
        identity: ResolvedIdentity,
        caller: &'caller dyn TrustedCallerContext,
        cancellation: CancellationToken,
    ) -> Result<Self, AdmissionFailure> {
        let binding = Self {
            identity,
            caller: BorrowedCaller(caller),
            cancellation,
        };
        binding.check()?;
        Ok(binding)
    }

    /// Uses the real resolver's authorization and preserves its failure type.
    /// Recovery/login and credential validation remain the producer's work.
    pub fn resolve(
        resolver: &mut impl CurrentIdentityResolver,
        caller: &'caller dyn TrustedCallerContext,
        cancellation: CancellationToken,
    ) -> Result<Self, AdmissionFailure> {
        if cancellation.is_cancelled() {
            return Err(AdmissionFailure::Cancelled);
        }
        let identity = resolver
            .resolve_current_identity(&BorrowedCaller(caller))
            .map_err(AdmissionFailure::Resolution)?;
        Self::from_resolved(identity, caller, cancellation)
    }

    fn matches_caller(&self) -> bool {
        self.identity.principal_id() == self.caller.principal_id()
            && self.identity.caller_context_id() == self.caller.context_id()
    }

    /// Point-in-time check, not an operation lease or atomic dispatch barrier.
    pub fn check(&self) -> Result<(), AdmissionFailure> {
        if self.cancellation.is_cancelled() {
            return Err(AdmissionFailure::Cancelled);
        }
        if !self.matches_caller() {
            return Err(AdmissionFailure::CallerMismatch);
        }
        // is_live may invoke the producer's clock; recheck caller afterwards.
        let live = self.identity.is_live();
        if !self.matches_caller() {
            return Err(AdmissionFailure::CallerMismatch);
        }
        if self.cancellation.is_cancelled() {
            return Err(AdmissionFailure::Cancelled);
        }
        if !live {
            return Err(AdmissionFailure::StaleIdentity);
        }
        Ok(())
    }

    /// Checked access to the real observation and its existing typed metadata.
    /// Returning it does not turn a User/Profile/Session ID into authority.
    pub fn identity(&self) -> Result<&ResolvedIdentity, AdmissionFailure> {
        self.check()?;
        Ok(&self.identity)
    }

    /// Storage operations must still use UserStorageRootAdapter and the real
    /// ProfileRootAuthorizer. This context carries no consent grant.
    pub fn access_context(&self) -> Result<IdentityAccessContext<'_>, AdmissionFailure> {
        self.check()?;
        Ok(IdentityAccessContext::new(&self.identity, &self.caller))
    }

    /// The runtime must supply an already correctly authenticated transport;
    /// SVC has no public accessor to verify its principal against this caller.
    /// Its policy still authorizes the service operation. No caller is added to
    /// the payload. The returned future's lifetime also borrows this binding,
    /// but it does not continuously revalidate Identity or hold a private lease.
    /// ```compile_fail,E0505
    /// use identity::{ResolvedIdentity, TrustedCallerContext};
    /// use identity_svc_consumer_fixture::session_binding::SessionBinding;
    /// use svc::{CancellationToken, RequestEnvelope, ServiceClient};
    /// fn invalid(identity: ResolvedIdentity, caller: &dyn TrustedCallerContext,
    ///            client: &ServiceClient, request: RequestEnvelope) {
    ///     let binding = SessionBinding::from_resolved(identity, caller, CancellationToken::new()).unwrap();
    ///     let pending = binding.call(client, request).unwrap();
    ///     drop(binding);
    ///     drop(pending);
    /// }
    /// ```
    pub fn call<'operation>(
        &'operation self,
        client: &'operation ServiceClient,
        request: RequestEnvelope,
    ) -> Result<ServiceFuture<'operation>, AdmissionFailure> {
        self.check()?;
        Ok(client.call(request, self.cancellation.clone()))
    }

    /// Requests cooperative cancellation, without claiming rollback or logout.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}
