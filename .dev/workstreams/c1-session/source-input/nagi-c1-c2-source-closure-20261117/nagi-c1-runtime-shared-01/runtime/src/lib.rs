//! Init-facing consumer of existing session and profile-root contracts.
//! Reuses the legacy credential checker; no consent policy or READY claim.
#![forbid(unsafe_code)]

mod interactive;
pub use interactive::InteractiveSessionContext;

#[cfg_attr(not(test), allow(dead_code))]
mod credential_entry;

#[cfg_attr(not(test), allow(dead_code))]
mod ingress;

// Linux candidate API only; the existing SDK/OS interfaces are unchanged.
#[cfg(target_os = "linux")]
pub use ingress::account_binding::persistent::{
    AuthenticatedIngress, BindingRow as ConfiguredBinding, InitialState, PersistentFailure,
    PersistentRuntime, SharedIssueFailure, SuppliedAccounts,
};
#[cfg(target_os = "linux")]
pub use ingress::account_binding::BoundLoginFailure;
pub use shared::{SharedCheckFailure, SharedObservation};

// Candidate-internal only: no public SDK/OS-fd interface is adopted here.
#[cfg(target_os = "linux")]
#[cfg_attr(not(test), allow(dead_code))]
mod linux_directory;

#[cfg(test)]
#[path = "../../consumer/src/fixture.rs"]
#[allow(dead_code)]
mod session_fixture;

use consumer::session_binding::{AdmissionFailure, SessionBinding};
use identity::{
    CurrentIdentityResolver, ProfileRootAuthorizer, ProfileStorageBackend, StorageRootFailure,
    StorageRootHandle, TrustedCallerContext, UserStorageRootAdapter,
};
use svc::CancellationToken;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SupplyFailure {
    Session(AdmissionFailure),
    Profile(StorageRootFailure),
}

/// Keeps the checked binding and its actual opaque profile-root handle together.
/// The caller and issuing IdentityService remain runtime-owned. Owning the
/// storage adapter scopes opened roots to this supply, including failure cleanup.
pub struct SessionProfile<'caller, B: ProfileStorageBackend> {
    session: SessionBinding<'caller>,
    root: StorageRootHandle,
    roots: UserStorageRootAdapter<B>,
}

impl<B: ProfileStorageBackend> SessionProfile<'_, B> {
    pub fn session(&self) -> &SessionBinding<'_> {
        &self.session
    }

    /// Still requires a fresh session admission/context and the existing
    /// storage authorizer for use. The handle is not a durable consent receipt.
    pub fn parts(
        &mut self,
    ) -> (
        &SessionBinding<'_>,
        &StorageRootHandle,
        &mut UserStorageRootAdapter<B>,
    ) {
        (&self.session, &self.root, &mut self.roots)
    }
}

/// Consume an already established current session through the real SDK resolver.
/// The producer must first authorize recover and validate credentials before
/// login_local; this function never accepts raw User/Profile/Principal strings.
///
/// `authorizer` is the SDK's existing ProfileRootAuthorizer. Its real policy
/// source must be supplied by the consent/authorization owner. A live Identity
/// does not substitute for that decision. No approval or grant is invented here.
///
/// These are point-in-time checks, not atomic cancellation or init readiness.
pub fn supply_current<'caller, B: ProfileStorageBackend>(
    resolver: &mut impl CurrentIdentityResolver,
    caller: &'caller dyn TrustedCallerContext,
    cancellation: CancellationToken,
    mut roots: UserStorageRootAdapter<B>,
    authorizer: &mut impl ProfileRootAuthorizer,
) -> Result<SessionProfile<'caller, B>, SupplyFailure> {
    let session =
        SessionBinding::resolve(resolver, caller, cancellation).map_err(SupplyFailure::Session)?;
    let profile = session
        .identity()
        .map_err(SupplyFailure::Session)?
        .profile_id()
        .clone();
    let access = session.access_context().map_err(SupplyFailure::Session)?;
    let root = roots
        .resolve_root(&access, &profile, authorizer)
        .map_err(SupplyFailure::Profile)?;
    // A provider callback can invalidate the issuer or caller while opening.
    // Do not deliver that stale session to the init consumer as a success.
    session.check().map_err(SupplyFailure::Session)?;
    Ok(SessionProfile {
        session,
        root,
        roots,
    })
}
