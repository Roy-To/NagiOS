//! C1 receiver-side binding for the two lower-level inputs available to the
//! local Linux integration: A's password-authenticated account evidence and
//! C2 leaf's borrowed accepted-peer proof.
//!
//! The post-login receiver path issues only an account/peer binding. The
//! pre-session path can stage a login from a C2 root callback without importing
//! the root type. The stage is abortable and has no durable effect. The A-side
//! adapter must place it in C2's private `PendingInitialSessionTransaction`:
//! C2 calls C1's commit method only after its owner postcheck, and keeps abort
//! armed until the lease is attached. The public connection proof is
//! correlation only and cannot authorize publication of a caller or Session.

#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
use crate::C1SystemServiceRuntimeAdmission;
use crate::{
    AuthenticatedInteractiveCaller, CallerContextId, Clock, GuestProfileCleaner, IdempotencyKey,
    IdentityAuthorizer, IdentityError, IdentityEventSink, IdentityIdSource, IdentityService,
    IdentitySnapshotStore, InteractiveSessionContext, InteractiveSessionError, PrincipalId,
    ProfileId, ProfileKind, ResolvedIdentity, SessionId, UnixMillis, UserId,
};
use nagi_c2_accepted_connection_proof_contract::C2AcceptedConnectionProof;
use nagi_linux_account_verifier::{
    AccountAuthenticationError, AuthenticatedAccountOnConnection, LinuxAccountVerifier,
};
use nagi_peer_authenticated_connection::{
    AuthenticatedInteractivePeer, AuthenticatedInteractivePeerSelector,
    KernelAuthenticatedUnixPeer, PeerAuthenticationError, PeerConnectionId,
    TrustedInteractivePeerKey,
};
use std::num::NonZeroU64;
use std::os::unix::net::UnixStream;

/// Failure to bind authenticated account evidence to a currently known C1
/// user/profile and one exact accepted peer connection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C1AccountPeerBindingError {
    Account(AccountAuthenticationError),
    Peer(PeerAuthenticationError),
    InvalidIdentityMapping,
}

/// Failure while staging or committing the initial C1 Session for an
/// authenticated account bound to an accepted kernel peer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C1InitialInteractiveSessionError {
    Account(AccountAuthenticationError),
    Peer(PeerAuthenticationError),
    ConnectionProofMismatch,
    IdentityIssuerMismatch,
    InvalidIdentityMapping,
    InvalidTransactionState,
    Identity(IdentityError),
    Interactive(InteractiveSessionError),
}

/// C1's sealed caller for one successfully issued account Session. The
/// connection ID is a selector for the C2 root to correlate; it is not caller
/// identity, liveness, or authorization evidence.
pub struct C1AuthenticatedInteractiveCaller {
    account_id: NonZeroU64,
    principal_id: PrincipalId,
    caller_context_id: CallerContextId,
    connection_id: PeerConnectionId,
}

impl C1AuthenticatedInteractiveCaller {
    pub const fn account_id(&self) -> NonZeroU64 {
        self.account_id
    }

    /// Compare the opaque socket selector and re-read the current kernel peer.
    /// This is correlation only; the C2 spawn/endpoint owner gate is separate.
    pub fn matches_kernel_connection(
        &self,
        peer: &KernelAuthenticatedUnixPeer,
        stream: &UnixStream,
    ) -> bool {
        self.connection_id == peer.connection_id() && peer.revalidate(stream).is_ok()
    }
}

impl crate::TrustedCallerContext for C1AuthenticatedInteractiveCaller {
    fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    fn context_id(&self) -> &CallerContextId {
        &self.caller_context_id
    }
}

impl crate::service::caller_proof_seal::Interactive for C1AuthenticatedInteractiveCaller {}

impl AuthenticatedInteractiveCaller for C1AuthenticatedInteractiveCaller {}

// Forward the same sealed identity through a borrow. The reference carries no
// authority beyond the borrowed caller's lifetime and cannot be constructed
// without an already-issued C1 caller.
impl crate::TrustedCallerContext for &C1AuthenticatedInteractiveCaller {
    fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    fn context_id(&self) -> &CallerContextId {
        &self.caller_context_id
    }
}

impl crate::service::caller_proof_seal::Interactive for &C1AuthenticatedInteractiveCaller {}

impl AuthenticatedInteractiveCaller for &C1AuthenticatedInteractiveCaller {}

/// C1-owned, uncommitted initial-login state. Creating it consumes the
/// protected account evidence and binds it to the exact kernel peer, stream,
/// and C2 correlation proof, but it does not mint a caller or write a Session.
/// The C2 root owns this value in its pending transaction. Its A adapter calls
/// `commit_after_c2_owner_postcheck`, then either rolls back before attachment
/// or takes the committed caller with `finish_after_c2_attachment`.
pub struct C1PendingInitialInteractiveSession {
    account_id: NonZeroU64,
    user_id: UserId,
    profile_id: ProfileId,
    principal_id: PrincipalId,
    connection_id: PeerConnectionId,
    issuer: crate::service::IdentityIssuerBinding,
    committed: Option<C1IssuedInteractiveSession>,
    finished: bool,
}

impl C1PendingInitialInteractiveSession {
    pub const fn account_id(&self) -> NonZeroU64 {
        self.account_id
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    /// Commit when the A adapter's C2 `PendingInitialSessionTransaction`
    /// receives its owner-gated `commit` call. C2 invokes it only after its
    /// private owner postcheck and keeps rollback armed until attachment.
    /// The C2 connection proof is correlation only; this sequencing must come
    /// from C2's private pending transaction, not direct request-handler calls.
    pub fn commit_after_c2_owner_postcheck<S, I, C, A, G, E>(
        &mut self,
        identity: &mut IdentityService<S, I, C, A, G, E>,
        expires_at: Option<UnixMillis>,
        idempotency_key: IdempotencyKey,
    ) -> Result<&C1IssuedInteractiveSession, C1InitialInteractiveSessionError>
    where
        S: IdentitySnapshotStore,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        if self.finished || self.committed.is_some() {
            return Err(C1InitialInteractiveSessionError::InvalidTransactionState);
        }
        if !identity.owns_issuer_binding(&self.issuer) {
            return Err(C1InitialInteractiveSessionError::IdentityIssuerMismatch);
        }
        let caller = C1AuthenticatedInteractiveCaller {
            account_id: self.account_id,
            principal_id: self.principal_id.clone(),
            caller_context_id: identity
                .issue_fresh_caller_context_id()
                .map_err(C1InitialInteractiveSessionError::Identity)?,
            connection_id: self.connection_id,
        };
        let session_id = identity
            .login_local(
                &caller,
                self.user_id,
                self.profile_id.clone(),
                expires_at,
                idempotency_key,
            )
            .map_err(C1InitialInteractiveSessionError::Identity)?;
        self.committed = Some(C1IssuedInteractiveSession {
            caller,
            session_id,
            issuer: self.issuer.clone(),
            #[cfg(all(
                feature = "system-service-recovery-contract",
                target_os = "linux",
                target_arch = "x86_64"
            ))]
            runtime_operation_identity: None,
        });
        Ok(self.committed.as_ref().expect("committed above"))
    }

    /// Commit the first interactive Session after C2's owner postcheck while
    /// A retains the exact C1 RuntimeActive admission inside its live C3/C2
    /// callback. The token only authorizes these C1 operations; it does not
    /// replace either owner's gate or the accepted-connection proof.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    #[allow(private_bounds)]
    pub fn commit_after_c2_owner_postcheck_with_runtime_admission<S, I, C, A, G, E>(
        &mut self,
        identity: &mut IdentityService<S, I, C, A, G, E>,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
        expires_at: Option<UnixMillis>,
        idempotency_key: IdempotencyKey,
    ) -> Result<&C1IssuedInteractiveSession, C1InitialInteractiveSessionError>
    where
        S: crate::C1SystemServiceRecoveryJournal,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        if self.finished || self.committed.is_some() {
            return Err(C1InitialInteractiveSessionError::InvalidTransactionState);
        }
        if !identity.owns_issuer_binding(&self.issuer) {
            return Err(C1InitialInteractiveSessionError::IdentityIssuerMismatch);
        }
        identity
            .validate_system_service_runtime_admission(admission)
            .map_err(C1InitialInteractiveSessionError::Identity)?;
        let caller = C1AuthenticatedInteractiveCaller {
            account_id: self.account_id,
            principal_id: self.principal_id.clone(),
            caller_context_id: identity
                .issue_fresh_caller_context_id_with_runtime_admission(admission)
                .map_err(C1InitialInteractiveSessionError::Identity)?,
            connection_id: self.connection_id,
        };
        let session_id = identity
            .login_local_with_runtime_admission(
                admission,
                &caller,
                self.user_id,
                self.profile_id.clone(),
                expires_at,
                idempotency_key,
            )
            .map_err(C1InitialInteractiveSessionError::Identity)?;
        self.committed = Some(C1IssuedInteractiveSession {
            caller,
            session_id,
            issuer: self.issuer.clone(),
            runtime_operation_identity: Some(admission.operation_identity()),
        });
        Ok(self.committed.as_ref().expect("committed above"))
    }

    /// Roll back a Session committed outside RuntimeActive if C2 rejects it
    /// before final attachment. Before commit this just discards pending state.
    /// Under RuntimeActive, use
    /// `abort_before_c2_attachment_with_runtime_admission`; this ordinary
    /// lifecycle entry deliberately fails closed on the recovery journal.
    /// Errors leave the committed record available for retry/reconciliation.
    pub fn abort_before_c2_attachment<S, I, C, A, G, E>(
        &mut self,
        identity: &mut IdentityService<S, I, C, A, G, E>,
    ) -> Result<(), C1InitialInteractiveSessionError>
    where
        S: IdentitySnapshotStore,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        if self.finished {
            return Err(C1InitialInteractiveSessionError::InvalidTransactionState);
        }
        if !identity.owns_issuer_binding(&self.issuer) {
            return Err(C1InitialInteractiveSessionError::IdentityIssuerMismatch);
        }
        if let Some(issued) = self.committed.as_ref() {
            issued.abort_before_c2_attachment(identity)?;
            self.committed = None;
        }
        self.finished = true;
        Ok(())
    }

    /// Cancel an initial Session committed under the live C1 RuntimeActive
    /// admission when C2 cannot attach it. Ordinary lifecycle entry points
    /// intentionally reject a RuntimeActive journal, so this transaction must
    /// supply the exact admission that committed the Session. A failed durable
    /// cancellation leaves the transaction armed for retry/reconciliation; a
    /// successful cancellation ends only this Session and revokes its lease.
    /// The persisted User/Profile and the external account credential are not
    /// removed.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    #[allow(private_bounds)]
    pub fn abort_before_c2_attachment_with_runtime_admission<S, I, C, A, G, E>(
        &mut self,
        identity: &mut IdentityService<S, I, C, A, G, E>,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<(), C1InitialInteractiveSessionError>
    where
        S: crate::C1SystemServiceRecoveryJournal,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        if self.finished {
            return Err(C1InitialInteractiveSessionError::InvalidTransactionState);
        }
        if !identity.owns_issuer_binding(&self.issuer) {
            return Err(C1InitialInteractiveSessionError::IdentityIssuerMismatch);
        }
        identity
            .validate_system_service_runtime_admission(admission)
            .map_err(C1InitialInteractiveSessionError::Identity)?;
        if let Some(issued) = self.committed.as_ref() {
            issued.abort_before_c2_attachment_with_runtime_admission(identity, admission)?;
            self.committed = None;
        }
        self.finished = true;
        Ok(())
    }

    /// Transfer the committed caller only after C2 has attached and validated
    /// the matching SessionLease.
    pub fn finish_after_c2_attachment(
        mut self,
    ) -> Result<C1IssuedInteractiveSession, C1InitialInteractiveSessionError> {
        if self.finished {
            return Err(C1InitialInteractiveSessionError::InvalidTransactionState);
        }
        let issued = self
            .committed
            .take()
            .ok_or(C1InitialInteractiveSessionError::InvalidTransactionState)?;
        self.finished = true;
        Ok(issued)
    }
}

/// A committed C1 caller and durable Session. Its caller is the only value
/// exposed to `InteractiveSessionContext`; neither the account name nor the
/// C2 selectors are accepted as identity inputs.
pub struct C1IssuedInteractiveSession {
    caller: C1AuthenticatedInteractiveCaller,
    session_id: SessionId,
    issuer: crate::service::IdentityIssuerBinding,
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    runtime_operation_identity: Option<crate::C1RecoveryOperationIdentity>,
}

impl C1IssuedInteractiveSession {
    pub fn caller(&self) -> &C1AuthenticatedInteractiveCaller {
        &self.caller
    }

    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    /// Revoke a Session committed outside RuntimeActive if C2 rejects it
    /// before final attachment. Under RuntimeActive, use the owning pending
    /// transaction's `abort_before_c2_attachment_with_runtime_admission`.
    /// If this returns an error, keep admission closed and reconcile before
    /// retrying.
    pub fn abort_before_c2_attachment<S, I, C, A, G, E>(
        &self,
        identity: &mut IdentityService<S, I, C, A, G, E>,
    ) -> Result<(), C1InitialInteractiveSessionError>
    where
        S: IdentitySnapshotStore,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        if !identity.owns_issuer_binding(&self.issuer) {
            return Err(C1InitialInteractiveSessionError::IdentityIssuerMismatch);
        }
        identity
            .end_session(&self.caller, &self.session_id)
            .map(|_| ())
            .map_err(C1InitialInteractiveSessionError::Identity)
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    fn abort_before_c2_attachment_with_runtime_admission<S, I, C, A, G, E>(
        &self,
        identity: &mut IdentityService<S, I, C, A, G, E>,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<(), C1InitialInteractiveSessionError>
    where
        S: crate::C1SystemServiceRecoveryJournal,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        if !identity.owns_issuer_binding(&self.issuer) {
            return Err(C1InitialInteractiveSessionError::IdentityIssuerMismatch);
        }
        let operation_identity =
            self.runtime_operation_identity
                .ok_or(C1InitialInteractiveSessionError::Identity(
                    IdentityError::StoreUnavailable,
                ))?;
        identity
            .rollback_pending_initial_session_with_runtime_admission(
                &self.caller,
                &self.session_id,
                operation_identity,
                admission,
            )
            .map(|_| ())
            .map_err(C1InitialInteractiveSessionError::Identity)
    }

    /// End this exact issued Session while the matching C1 RuntimeActive
    /// admission remains held by the boot owner. This is the normal runtime
    /// logout entry; it is deliberately unavailable without the original
    /// issued-session value, its owning IdentityService, and the exact live
    /// C1 admission. Invoke it after leaving C1 effect callbacks and C2 owner
    /// gates. A concurrent admitted effect returns `SessionLeaseBusy` without
    /// committing logout, so the owner may retry with this same admission
    /// after that effect exits. Existing `SessionLeaseRevalidationHandle`s
    /// then reject protected effects against the ended durable row.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    #[allow(private_bounds)]
    pub fn end_session_with_runtime_admission<S, I, C, A, G, E>(
        &self,
        identity: &mut IdentityService<S, I, C, A, G, E>,
        admission: &crate::C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<crate::EndSessionResult, C1InitialInteractiveSessionError>
    where
        S: crate::C1SystemServiceRecoveryJournal,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        if !identity.owns_issuer_binding(&self.issuer) {
            return Err(C1InitialInteractiveSessionError::IdentityIssuerMismatch);
        }
        identity
            .end_session_with_runtime_admission(&self.caller, &self.session_id, admission)
            .map_err(C1InitialInteractiveSessionError::Identity)
    }

    /// Revoke this exact Session using the same admitted owner and caller
    /// checks as logout. Its durable reason is Revoked, not UserLogout.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    #[allow(private_bounds)]
    pub fn revoke_session_with_runtime_admission<S, I, C, A, G, E>(
        &self,
        identity: &mut IdentityService<S, I, C, A, G, E>,
        admission: &crate::C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<crate::EndSessionResult, C1InitialInteractiveSessionError>
    where
        S: crate::C1SystemServiceRecoveryJournal,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        if !identity.owns_issuer_binding(&self.issuer) {
            return Err(C1InitialInteractiveSessionError::IdentityIssuerMismatch);
        }
        identity
            .revoke_session_with_runtime_admission(&self.caller, &self.session_id, admission)
            .map_err(C1InitialInteractiveSessionError::Identity)
    }

    pub fn checked_context<'caller, S, I, C, A, G, E>(
        &'caller self,
        identity: &mut IdentityService<S, I, C, A, G, E>,
    ) -> Result<InteractiveSessionContext<'caller>, C1InitialInteractiveSessionError>
    where
        S: IdentitySnapshotStore,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        if !identity.owns_issuer_binding(&self.issuer) {
            return Err(C1InitialInteractiveSessionError::IdentityIssuerMismatch);
        }
        identity
            .interactive_session(&self.caller)
            .map_err(C1InitialInteractiveSessionError::Interactive)
    }

    /// Resolve the newly committed Session while the exact live C1 runtime
    /// admission remains held. The resulting lease reader is pinned to the
    /// same durable RuntimeActive operation, not to a generic quarantine
    /// override.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    #[allow(private_bounds)]
    pub fn checked_context_with_runtime_admission<'caller, S, I, C, A, G, E>(
        &'caller self,
        identity: &mut IdentityService<S, I, C, A, G, E>,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<InteractiveSessionContext<'caller>, C1InitialInteractiveSessionError>
    where
        S: crate::C1SystemServiceRecoveryJournal,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        if !identity.owns_issuer_binding(&self.issuer) {
            return Err(C1InitialInteractiveSessionError::IdentityIssuerMismatch);
        }
        identity
            .interactive_session_with_runtime_admission(&self.caller, admission)
            .map_err(C1InitialInteractiveSessionError::Interactive)
    }

    /// Return an owned identity observation for an external factory that
    /// retains the Session identity beyond the borrowed caller context. The
    /// value is resolved through this issued caller and the exact held C1
    /// RuntimeActive operation; it is non-cloneable and its live Session lease
    /// remains revocable. Its operation ID is exposed only as correlation.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    #[allow(private_bounds)]
    pub fn checked_identity_with_runtime_admission<S, I, C, A, G, E>(
        &self,
        identity: &mut IdentityService<S, I, C, A, G, E>,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<ResolvedIdentity, C1InitialInteractiveSessionError>
    where
        S: crate::C1SystemServiceRecoveryJournal,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        if !identity.owns_issuer_binding(&self.issuer) {
            return Err(C1InitialInteractiveSessionError::IdentityIssuerMismatch);
        }
        let resolved = identity
            .resolve_current_identity_with_runtime_admission(&self.caller, admission)
            .map_err(|error| {
                C1InitialInteractiveSessionError::Interactive(InteractiveSessionError::Identity(
                    error,
                ))
            })?;
        if resolved.session_id() != &self.session_id
            || resolved.principal_id() != &self.caller.principal_id
            || resolved.caller_context_id() != &self.caller.caller_context_id
            || resolved.runtime_operation_identity() != Some(admission.operation_identity())
        {
            return Err(C1InitialInteractiveSessionError::Identity(
                IdentityError::StoreUnavailable,
            ));
        }
        Ok(resolved)
    }
}

/// Password-authenticated A account mapping correlated with C2 leaf evidence
/// on the same live Unix socket. This is a transport/account binding only: it
/// does not prove C2 spawn/endpoint ownership, mint a caller seal, issue a
/// Session, or authorize effects. C1 must consume it together with the exact
/// C2 root admission after that root proof has a cycle-free representation.
pub struct C1AuthenticatedAccountPeerBinding<'peer, 'connection> {
    account_id: NonZeroU64,
    user_id: UserId,
    profile_id: ProfileId,
    principal_id: PrincipalId,
    owner_key: TrustedInteractivePeerKey,
    selector: AuthenticatedInteractivePeerSelector,
    peer: &'peer AuthenticatedInteractivePeer<'connection>,
    connection: &'connection UnixStream,
}

impl C1AuthenticatedAccountPeerBinding<'_, '_> {
    pub const fn account_id(&self) -> NonZeroU64 {
        self.account_id
    }

    pub const fn user_id(&self) -> UserId {
        self.user_id
    }

    pub fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    pub fn matches_peer(&self, peer: &AuthenticatedInteractivePeer<'_>) -> bool {
        std::ptr::eq(self.peer.kernel_peer(), peer.kernel_peer())
            && self.owner_key == *peer.owner_key()
            && self.selector == *peer.selector()
            && self
                .peer
                .kernel_peer()
                .require_same_connection(self.connection)
                .is_ok()
            && peer
                .kernel_peer()
                .require_same_connection(self.connection)
                .is_ok()
    }

    /// Check C2's shared correlation value against the peer already bound to
    /// this authenticated account. This compares only the kernel connection
    /// identity; it does not validate C2's root owner, endpoint liveness,
    /// callback lease, or pre/post owner checks. Callers must already be in
    /// the real C2 owner-checked callback and must keep C2's own activation
    /// transaction in force. A successful comparison is not a caller seal,
    /// Session, or effect grant.
    pub fn matches_c2_connection_proof(
        &self,
        peer: &AuthenticatedInteractivePeer<'_>,
        proof: &C2AcceptedConnectionProof<'_>,
    ) -> bool {
        self.matches_peer(peer) && proof.matches_kernel_peer(peer.kernel_peer())
    }
}

/// Consume A's non-forgeable password proof using the same selected verifier
/// and exact stream held by C2's borrowed leaf proof. It then checks that the
/// authenticated account mapping names an existing persistent C1 profile
/// owned by the mapped C1 user. The returned binding is deliberately not a
/// caller or Session authority; a C2 root admission remains mandatory.
pub fn authenticate_account_for_accepted_peer<'peer, 'connection, S, I, C, A, G, E>(
    identity: &IdentityService<S, I, C, A, G, E>,
    verifier: &LinuxAccountVerifier,
    evidence: AuthenticatedAccountOnConnection<'_, '_>,
    peer: &'peer AuthenticatedInteractivePeer<'connection>,
    connection: &'connection UnixStream,
) -> Result<C1AuthenticatedAccountPeerBinding<'peer, 'connection>, C1AccountPeerBindingError>
where
    S: IdentitySnapshotStore,
    I: crate::IdentityIdSource,
    C: crate::Clock,
    A: crate::IdentityAuthorizer,
    G: crate::GuestProfileCleaner,
    E: crate::IdentityEventSink,
{
    peer.kernel_peer()
        .require_same_connection(connection)
        .map_err(C1AccountPeerBindingError::Peer)?;

    let account = verifier
        .consume_on_connection(evidence, connection)
        .map_err(C1AccountPeerBindingError::Account)?;

    let user_id = UserId::new(account.user_id().get())
        .map_err(|_| C1AccountPeerBindingError::InvalidIdentityMapping)?;
    let profile_id = ProfileId::new(account.profile_id().to_owned())
        .map_err(|_| C1AccountPeerBindingError::InvalidIdentityMapping)?;
    let principal_id = PrincipalId::new(account.principal_id().as_str().to_owned())
        .map_err(|_| C1AccountPeerBindingError::InvalidIdentityMapping)?;

    let snapshot = identity.snapshot();
    snapshot
        .validate()
        .map_err(|_| C1AccountPeerBindingError::InvalidIdentityMapping)?;
    if !snapshot.users.iter().any(|user| user.user_id == user_id)
        || !snapshot.profiles.iter().any(|profile| {
            profile.profile_id == profile_id
                && profile.owner_user_id == Some(user_id)
                && profile.kind == ProfileKind::Persistent
        })
        || !principal_id.as_str().starts_with("user:")
    {
        return Err(C1AccountPeerBindingError::InvalidIdentityMapping);
    }

    Ok(C1AuthenticatedAccountPeerBinding {
        account_id: account.account_id(),
        user_id,
        profile_id,
        principal_id,
        owner_key: *peer.owner_key(),
        selector: peer.selector().clone(),
        peer,
        connection,
    })
}

/// Consume real A account evidence and stage the first C1 Session while the
/// C2 pre-session spawn/endpoint admission is borrowed. This boundary
/// intentionally accepts the kernel peer, not the post-login interactive
/// selector: C1 must not require an existing Session to create the first one.
/// C1 checks same-stream peer correlation, but the proof is not root-owner
/// authorization. The caller must invoke this only inside C2's private owner
/// admission. The returned state is not a caller or Session and has no
/// durable effect; it must be aborted unless C2 later invokes the transaction's
/// commit method after its private owner postcheck. The proof is correlation
/// only and does not carry that authorization.
pub fn stage_initial_interactive_session_from_c2_owner_checked_peer<S, I, C, A, G, E>(
    identity: &IdentityService<S, I, C, A, G, E>,
    verifier: &LinuxAccountVerifier,
    evidence: AuthenticatedAccountOnConnection<'_, '_>,
    peer: &KernelAuthenticatedUnixPeer,
    stream: &UnixStream,
    proof: &C2AcceptedConnectionProof<'_>,
) -> Result<C1PendingInitialInteractiveSession, C1InitialInteractiveSessionError>
where
    S: IdentitySnapshotStore,
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    peer.require_same_connection(stream)
        .map_err(C1InitialInteractiveSessionError::Peer)?;
    peer.revalidate(stream)
        .map_err(C1InitialInteractiveSessionError::Peer)?;
    if !proof.matches_kernel_peer(peer) {
        return Err(C1InitialInteractiveSessionError::ConnectionProofMismatch);
    }

    let account = verifier
        .consume_on_connection(evidence, stream)
        .map_err(C1InitialInteractiveSessionError::Account)?;
    let user_id = UserId::new(account.user_id().get())
        .map_err(|_| C1InitialInteractiveSessionError::InvalidIdentityMapping)?;
    let profile_id = ProfileId::new(account.profile_id().to_owned())
        .map_err(|_| C1InitialInteractiveSessionError::InvalidIdentityMapping)?;
    let principal_id = PrincipalId::new(account.principal_id().as_str().to_owned())
        .map_err(|_| C1InitialInteractiveSessionError::InvalidIdentityMapping)?;

    let snapshot = identity.snapshot();
    snapshot
        .validate()
        .map_err(|_| C1InitialInteractiveSessionError::InvalidIdentityMapping)?;
    if !snapshot.users.iter().any(|user| user.user_id == user_id)
        || !snapshot.profiles.iter().any(|profile| {
            profile.profile_id == profile_id
                && profile.owner_user_id == Some(user_id)
                && profile.kind == ProfileKind::Persistent
        })
        || !principal_id.as_str().starts_with("user:")
    {
        return Err(C1InitialInteractiveSessionError::InvalidIdentityMapping);
    }

    Ok(C1PendingInitialInteractiveSession {
        account_id: account.account_id(),
        user_id,
        profile_id,
        principal_id,
        connection_id: peer.connection_id(),
        issuer: identity.issuer_binding(),
        committed: None,
        finished: false,
    })
}

#[cfg(all(
    test,
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
pub(crate) fn pending_initial_interactive_session_for_test<S, I, C, A, G, E>(
    identity: &IdentityService<S, I, C, A, G, E>,
    account_id: NonZeroU64,
    user_id: UserId,
    profile_id: ProfileId,
    principal_id: PrincipalId,
    connection_id: PeerConnectionId,
) -> C1PendingInitialInteractiveSession
where
    S: IdentitySnapshotStore,
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    C1PendingInitialInteractiveSession {
        account_id,
        user_id,
        profile_id,
        principal_id,
        connection_id,
        issuer: identity.issuer_binding(),
        committed: None,
        finished: false,
    }
}
