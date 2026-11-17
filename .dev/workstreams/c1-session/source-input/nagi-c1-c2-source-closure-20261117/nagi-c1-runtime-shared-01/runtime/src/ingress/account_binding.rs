//! Explicit issuer-owned binding; no token/role/index conversion or enrollment.
//! AccountStore exposes no management generation or canonical SDK subject.
//! The borrowed gate does not attest its supplier's configuration authority.
//! `configured` owns an immutable cohort; `persistent` adds the Linux file store.
//! Management transitions must still arrange SDK revocation and cancellation.

mod configured;
#[cfg(target_os = "linux")]
pub(crate) mod persistent;

use identity::{
    Clock, GuestProfileCleaner, IdempotencyKey, IdentityAuthorizer, IdentityError,
    IdentityEventSink, IdentityIdSource, IdentityProvider, IdentityService, IdentitySnapshotStore,
    ProfileId, ProfileKind, ProviderLookupFailure, ProviderSubjectRef, SessionId,
    TrustedCallerContext, UnixMillis, UserId,
};
use libnagi::security::{AccountStore, LoginError};

struct ExplicitAccountBinding {
    // Exact credential-login name, scoped to the AccountStore borrowed below.
    // Not a new globally stable ID. The supplier must bind this store/account
    // to its canonical subject and existing SDK records truthfully.
    account_name: Vec<u8>,
    subject: ProviderSubjectRef,
    user: UserId,
    profile: ProfileId,
}

struct Credentials<'a> {
    name: &'a [u8],
    password: &'a [u8],
}

struct LoginIntent {
    expires_at: Option<UnixMillis>,
    idempotency_key: IdempotencyKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BoundLoginFailure {
    Credential(LoginError),
    MissingBinding,
    AmbiguousBinding,
    Lookup(ProviderLookupFailure),
    UnmappedSubject,
    SubjectMismatch,
    UserNotFound,
    ProfileNotFound,
    ProfileOwnershipMismatch,
    Identity(IdentityError),
}

struct AccountBindingIssuer<'accounts, 'bindings, P> {
    accounts: &'accounts mut AccountStore,
    bindings: &'bindings [ExplicitAccountBinding],
    subjects: &'bindings P,
}

impl<'accounts, 'bindings, P: IdentityProvider> AccountBindingIssuer<'accounts, 'bindings, P> {
    fn new(
        accounts: &'accounts mut AccountStore,
        bindings: &'bindings [ExplicitAccountBinding],
        subjects: &'bindings P,
    ) -> Self {
        Self {
            accounts,
            bindings,
            subjects,
        }
    }

    // Runtime must recover the real issuer before this call. It retains the
    // returned SDK SessionId for authorized end_session/recovery if subsequent
    // RuntimeIngress::supply fails. A SessionId itself is not authority.
    fn login<S, I, C, A, G, E>(
        &mut self,
        issuer: &mut IdentityService<S, I, C, A, G, E>,
        caller: &impl TrustedCallerContext,
        credentials: Credentials<'_>,
        intent: LoginIntent,
    ) -> Result<SessionId, BoundLoginFailure>
    where
        S: IdentitySnapshotStore,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        // Fresh verification against this exact borrowed store. Its returned
        // legacy Session is deliberately not projected into any SDK identifier.
        let _legacy = self
            .accounts
            .authenticate(credentials.name, credentials.password)
            .map_err(BoundLoginFailure::Credential)?;
        let mut matches = self
            .bindings
            .iter()
            .filter(|binding| binding.account_name.as_slice() == credentials.name);
        let binding = matches.next().ok_or(BoundLoginFailure::MissingBinding)?;
        if matches.next().is_some() {
            return Err(BoundLoginFailure::AmbiguousBinding);
        }
        let subject_user = self
            .subjects
            .resolve_subject(&binding.subject)
            .map_err(BoundLoginFailure::Lookup)?
            .ok_or(BoundLoginFailure::UnmappedSubject)?;
        if subject_user != binding.user {
            return Err(BoundLoginFailure::SubjectMismatch);
        }
        let snapshot = issuer.snapshot();
        if !snapshot
            .users
            .iter()
            .any(|user| user.user_id == binding.user)
        {
            return Err(BoundLoginFailure::UserNotFound);
        }
        let profile = snapshot
            .profiles
            .iter()
            .find(|profile| profile.profile_id == binding.profile)
            .ok_or(BoundLoginFailure::ProfileNotFound)?;
        if profile.owner_user_id != Some(binding.user) || profile.kind != ProfileKind::Persistent {
            return Err(BoundLoginFailure::ProfileOwnershipMismatch);
        }
        // The SDK still performs its own recovered-state, caller authorization,
        // membership, expiry, idempotency, ID and durable-commit checks.
        issuer
            .login_local(
                caller,
                binding.user,
                binding.profile.clone(),
                intent.expires_at,
                intent.idempotency_key,
            )
            .map_err(BoundLoginFailure::Identity)
    }
}

#[cfg(test)]
mod tests;
