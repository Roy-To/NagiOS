//! Private Linux supplier candidate. Persisted IDs/configuration are not grants.
//! The caller supplies an authorized exclusive directory and truthful account
//! generation. No credential verifier material is serialized here.

mod linux_inputs;
mod store;

use super::configured::{ConfiguredAccountInputs, SubjectMapping};
use super::{BoundLoginFailure, Credentials, ExplicitAccountBinding, LoginIntent};
use crate::ingress::RuntimeIngress;
use crate::InteractiveSessionContext;
use consumer::session_binding::AdmissionFailure;
use identity::{
    CurrentIdentityResolver, GuestProfileCleaner, IdentityAuthorizer, IdentityError,
    IdentityEventSink, IdentityService, IdentitySnapshot, ProfileId, ProviderSubjectRef, SessionId,
    TrustedCallerContext, UserId,
};
use libnagi::security::AccountStore;
use linux_inputs::{IssuerClock, LinuxIds};
use serde::{Deserialize, Serialize};
use std::fs::File;
use svc::{CancellationToken, ServiceClient};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SharedIssueFailure {
    Resolution(identity::ResolutionError),
    Admission(shared::SharedCheckFailure),
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BindingRow {
    account_name: Vec<u8>,
    provider: String,
    subject: String,
    user: UserId,
    profile: ProfileId,
}

impl<A: IdentityAuthorizer, G: GuestProfileCleaner, E: IdentityEventSink> CurrentIdentityResolver
    for PersistentRuntime<A, G, E>
{
    fn resolve_current_identity(
        &mut self,
        caller: &impl TrustedCallerContext,
    ) -> Result<identity::ResolvedIdentity, identity::ResolutionError> {
        self.check_clock()
            .map_err(|_| identity::ResolutionError::Unavailable)?;
        let result = self
            .service
            .as_mut()
            .ok_or(identity::ResolutionError::Unavailable)?
            .resolve_current_identity(caller);
        self.check_clock()
            .map_err(|_| identity::ResolutionError::Unavailable)?;
        result
    }
}

pub struct InitialState {
    generation: u64,
    bindings: Vec<BindingRow>,
    snapshot: IdentitySnapshot,
}

// The credential owner labels and transfers the exact store for this config.
// The private legacy AccountStore has no export, canonical ID or generation
// accessor, so this label cannot attest the original store's truth by itself.
pub struct SuppliedAccounts {
    generation: u64,
    accounts: AccountStore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistentFailure {
    Unavailable,
    Corrupt,
    GenerationMismatch,
    ClockUnavailable,
    FiniteExpiryRequired,
    Stopped,
    Login(BoundLoginFailure),
    Identity(IdentityError),
    Resolution(identity::ResolutionError),
}

type Service<A, G, E> = IdentityService<store::FileSnapshotStore, LinuxIds, IssuerClock, A, G, E>;

// Only a borrowed sized view for the existing SDK's &impl caller parameters.
// No captured ID, payload or OS UID is converted into authority here.
struct BorrowedCaller<'caller>(&'caller dyn TrustedCallerContext);
impl TrustedCallerContext for BorrowedCaller<'_> {
    fn principal_id(&self) -> &identity::PrincipalId {
        self.0.principal_id()
    }
    fn context_id(&self) -> &identity::CallerContextId {
        self.0.context_id()
    }
}

pub struct PersistentRuntime<A, G, E> {
    service: Option<Service<A, G, E>>,
    inputs: ConfiguredAccountInputs,
    clock: IssuerClock,
    cancellation: CancellationToken,
    #[cfg(test)]
    fault: std::sync::Arc<std::sync::Mutex<Option<store::Fault>>>,
}

impl BindingRow {
    /// Explicit configuration supplied by the account authority. These values
    /// are not authentication evidence and never provision a User or Profile.
    pub fn new(
        account_name: Vec<u8>,
        subject: ProviderSubjectRef,
        user: UserId,
        profile: ProfileId,
    ) -> Result<Self, PersistentFailure> {
        if account_name.is_empty() || account_name.len() > 16 {
            return Err(PersistentFailure::Corrupt);
        }
        Ok(Self {
            account_name,
            provider: subject.provider().to_owned(),
            subject: subject.subject().to_owned(),
            user,
            profile,
        })
    }
}

impl InitialState {
    /// Used only for explicitly authorized initialization into an empty store.
    pub fn new(generation: u64, bindings: Vec<BindingRow>, snapshot: IdentitySnapshot) -> Self {
        Self {
            generation,
            bindings,
            snapshot,
        }
    }
}

impl SuppliedAccounts {
    /// The owner attests this exact transferred verifier store's generation.
    /// A number alone does not authenticate or validate that attestation.
    pub fn new(generation: u64, accounts: AccountStore) -> Result<Self, PersistentFailure> {
        if generation == 0 {
            return Err(PersistentFailure::GenerationMismatch);
        }
        Ok(Self {
            generation,
            accounts,
        })
    }
}

pub struct AuthenticatedIngress<'runtime> {
    inner: RuntimeIngress<'runtime>,
}

impl AuthenticatedIngress<'_> {
    /// Point-in-time and per-poll admission, not an atomic policy/dispatch lease.
    pub fn call<'call>(
        &'call self,
        request: svc::RequestEnvelope,
    ) -> Result<
        impl std::future::Future<Output = Result<svc::ResponseEnvelope, AdmissionFailure>> + 'call,
        AdmissionFailure,
    > {
        self.inner.call(request)
    }

    /// Cooperative cancellation of this issuer runtime's shared call token.
    pub fn revoke(&self) {
        self.inner.revoke();
    }
}

impl<A: IdentityAuthorizer, G: GuestProfileCleaner, E: IdentityEventSink>
    PersistentRuntime<A, G, E>
{
    // initial=None always requires an existing valid file; it never provisions
    // missing accounts/User/Profile or resets a corrupt/unknown generation.
    pub fn start(
        directory: File,
        supplied: SuppliedAccounts,
        initial: Option<InitialState>,
        authorizer: A,
        cleaner: G,
        events: E,
        caller: &dyn TrustedCallerContext,
    ) -> Result<Self, PersistentFailure> {
        let store = store::FileSnapshotStore::open(directory, supplied.generation, initial)?;
        let clock = store.clock.clone();
        let bindings = store
            .bindings()
            .iter()
            .map(|row| {
                Ok(ExplicitAccountBinding {
                    account_name: row.account_name.clone(),
                    subject: ProviderSubjectRef::new(row.provider.clone(), row.subject.clone())
                        .map_err(|_| PersistentFailure::Corrupt)?,
                    user: row.user,
                    profile: row.profile.clone(),
                })
            })
            .collect::<Result<Vec<_>, PersistentFailure>>()?;
        let subjects = bindings
            .iter()
            .map(|binding| SubjectMapping {
                subject: binding.subject.clone(),
                user: binding.user,
            })
            .collect();
        let inputs = ConfiguredAccountInputs::from_supplied(supplied.accounts, bindings, subjects)
            .map_err(|_| PersistentFailure::Corrupt)?;
        #[cfg(test)]
        let fault = store.fault.clone();
        let mut service =
            IdentityService::new(store, LinuxIds, clock.clone(), authorizer, cleaner, events);
        // Actual SDK recovery ends every saved Active session (SystemRestart),
        // even in the same Linux boot. No old caller mapping/lease is restored.
        service
            .recover(&BorrowedCaller(caller))
            .map_err(PersistentFailure::Identity)?;
        clock
            .checked_now()
            .map_err(|_| PersistentFailure::ClockUnavailable)?;
        Ok(Self {
            service: Some(service),
            inputs,
            clock,
            cancellation: CancellationToken::new(),
            #[cfg(test)]
            fault,
        })
    }

    fn login(
        &mut self,
        caller: &dyn TrustedCallerContext,
        credentials: Credentials<'_>,
        intent: LoginIntent,
    ) -> Result<SessionId, PersistentFailure> {
        self.check_clock()?;
        // A failed Clock must not leave an unbounded lease alive. This private
        // consumer accepts only a finite expiry; SDK policy remains unchanged.
        if intent.expires_at.is_none() {
            return Err(PersistentFailure::FiniteExpiryRequired);
        }
        let result = self.inputs.issuer().login(
            self.service.as_mut().ok_or(PersistentFailure::Stopped)?,
            &BorrowedCaller(caller),
            credentials,
            intent,
        );
        if matches!(
            result,
            Err(BoundLoginFailure::Identity(
                IdentityError::StoreUnavailable | IdentityError::StoreConflict
            ))
        ) {
            self.stop();
            return result.map_err(PersistentFailure::Login);
        }
        self.check_clock()?;
        result.map_err(PersistentFailure::Login)
    }

    /// Performs fresh verification in the transferred real AccountStore before
    /// binding lookup and the SDK's independent authorization/durable issuance.
    pub fn authenticate_and_login(
        &mut self,
        caller: &dyn TrustedCallerContext,
        account_name: &[u8],
        password: &[u8],
        expires_at: identity::UnixMillis,
        idempotency_key: identity::IdempotencyKey,
    ) -> Result<SessionId, PersistentFailure> {
        self.login(
            caller,
            Credentials {
                name: account_name,
                password,
            },
            LoginIntent {
                expires_at: Some(expires_at),
                idempotency_key,
            },
        )
    }

    /// The runtime must already associate this trusted caller with the supplied
    /// client/transport; SVC has no API to attest that relationship here.
    pub fn bind_service<'runtime>(
        &mut self,
        caller: &'runtime dyn TrustedCallerContext,
        client: &'runtime ServiceClient,
    ) -> Result<AuthenticatedIngress<'runtime>, AdmissionFailure> {
        self.ingress(caller, client)
            .map(|inner| AuthenticatedIngress { inner })
    }

    /// C1 interactive context for an already authenticated, runtime-owned
    /// caller. This never interprets a payload/OS UID as a trusted caller and
    /// never constructs a system service principal from a human Session.
    pub fn interactive_session<'caller>(
        &mut self,
        caller: &'caller dyn TrustedCallerContext,
    ) -> Result<InteractiveSessionContext<'caller>, AdmissionFailure> {
        let cancellation = self.cancellation.clone();
        InteractiveSessionContext::resolve(self, caller, cancellation)
    }

    /// Resolve through the real issuer, retaining its actual caller and token.
    /// Issuance authorizes/resolves and may do normal owner work; call it BEFORE
    /// entering C2's commit locks. Only the returned observation.check() is the
    /// non-waiting admission read. No app number or metadata can create one.
    pub fn shared_interactive_session<'caller>(
        &mut self,
        caller: &'caller (dyn TrustedCallerContext + Sync),
    ) -> Result<shared::SharedObservation<'caller>, SharedIssueFailure> {
        let identity = self
            .resolve_current_identity(&BorrowedCaller(caller))
            .map_err(SharedIssueFailure::Resolution)?;
        shared::SharedObservation::from_resolved(identity, caller, self.cancellation.clone())
            .map_err(SharedIssueFailure::Admission)
    }

    /// Same producer for 'static shared storage: own the runtime's original
    /// caller Arc, not a copy of its labels. The issuer remains runtime-owned.
    pub fn shared_interactive_session_owned(
        &mut self,
        caller: std::sync::Arc<dyn TrustedCallerContext + Send + Sync>,
    ) -> Result<shared::SharedObservation<'static>, SharedIssueFailure> {
        let identity = self
            .resolve_current_identity(&BorrowedCaller(caller.as_ref()))
            .map_err(SharedIssueFailure::Resolution)?;
        shared::SharedObservation::from_owned(identity, caller, self.cancellation.clone())
            .map_err(SharedIssueFailure::Admission)
    }

    /// Existing SDK Recover authorization and semantics. Success never resumes
    /// a persisted Session; all previously issued interactive contexts stay
    /// revoked even after fresh authentication into the same User/Profile.
    pub fn recover(
        &mut self,
        caller: &dyn TrustedCallerContext,
    ) -> Result<identity::RecoveryReport, PersistentFailure> {
        self.check_clock()?;
        let result = self
            .service
            .as_mut()
            .ok_or(PersistentFailure::Stopped)?
            .recover(&BorrowedCaller(caller));
        if matches!(
            result,
            Err(IdentityError::StoreUnavailable | IdentityError::StoreConflict)
        ) {
            self.stop();
        }
        let report = result.map_err(PersistentFailure::Identity)?;
        self.check_clock()?;
        Ok(report)
    }

    /// Fail-closed interactive lock: resolve the actual current Session, then
    /// use existing EndSession authorization/revocation. No caller-supplied ID
    /// selects a different Session. User/Profile persist; resuming interactive
    /// admission requires fresh authentication and a new SDK Session.
    /// UI lock presentation and system-service OS credentials remain separate;
    /// no new on-disk Locked lifecycle or permission policy is introduced.
    pub fn lock_interactive_session(
        &mut self,
        caller: &dyn TrustedCallerContext,
    ) -> Result<(), PersistentFailure> {
        let identity = self
            .resolve_current_identity(&BorrowedCaller(caller))
            .map_err(PersistentFailure::Resolution)?;
        let session = identity.session_id().clone();
        self.end_session(caller, &session)
    }

    fn ingress<'runtime>(
        &mut self,
        caller: &'runtime dyn TrustedCallerContext,
        client: &'runtime ServiceClient,
    ) -> Result<RuntimeIngress<'runtime>, AdmissionFailure> {
        self.check_clock()
            .map_err(|_| AdmissionFailure::StaleIdentity)?;
        RuntimeIngress::supply(
            self.service
                .as_mut()
                .ok_or(AdmissionFailure::StaleIdentity)?,
            caller,
            client,
            self.cancellation.clone(),
        )
    }

    pub fn end_session(
        &mut self,
        caller: &dyn TrustedCallerContext,
        session: &SessionId,
    ) -> Result<(), PersistentFailure> {
        self.check_clock()?;
        let result = self
            .service
            .as_mut()
            .ok_or(PersistentFailure::Stopped)?
            .end_session(&BorrowedCaller(caller), session);
        if matches!(
            result,
            Err(IdentityError::StoreUnavailable | IdentityError::StoreConflict)
        ) {
            self.stop();
        }
        result.map_err(PersistentFailure::Identity)?;
        self.check_clock()
    }

    fn check_clock(&mut self) -> Result<(), PersistentFailure> {
        if self.service.is_none() || self.cancellation.is_cancelled() {
            self.stop();
            return Err(PersistentFailure::Stopped);
        }
        if self.clock.checked_now().is_err() {
            self.stop();
            return Err(PersistentFailure::ClockUnavailable);
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        self.cancellation.cancel();
        drop(self.service.take());
    }
}

impl<A, G, E> Drop for PersistentRuntime<A, G, E> {
    fn drop(&mut self) {
        self.cancellation.cancel();
        // Real SDK Drop revokes its evidence without waiting on callback leases.
        drop(self.service.take());
    }
}

#[cfg(test)]
mod tests;
