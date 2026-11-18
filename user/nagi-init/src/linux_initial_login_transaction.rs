use nagi_c2_accepted_connection_proof_contract::C2AcceptedConnectionProof;
use nagi_identity_session::{
    stage_initial_interactive_session_from_c2_owner_checked_peer, C1IssuedInteractiveSession,
    C1PendingInitialInteractiveSession, Clock, GuestProfileCleaner, IdempotencyKey,
    IdentityAuthorizer, IdentityEventSink, IdentityIdSource, IdentityService,
    IdentitySnapshotStore, LiveStatus, SessionLeaseRevalidationHandle, TrustedCallerContext,
    UnixMillis,
};
use nagi_linux_account_verifier::LinuxAccountVerifier;
use nagi_peer_authenticated_connection::KernelAuthenticatedUnixPeer;
use nagi_pid1_live_peer_supplier::{
    AcceptedInitialLoginRequest, PeerDenial, PendingInitialSession,
    PendingInitialSessionTransaction, SessionLease,
};
use std::num::NonZeroU64;
use std::os::unix::net::UnixStream;
use std::sync::MutexGuard;

pub enum InitialSessionOutcome {
    Unattached,
    Attached {
        issued: C1IssuedInteractiveSession,
        revalidation: SessionLeaseRevalidationHandle,
        selectors: (u64, u64),
    },
    RollbackRequired(C1PendingInitialInteractiveSession),
    OutcomeUnknown,
}

pub(super) struct InitialLoginState<S: IdentitySnapshotStore, I, C, A, G, E> {
    identity: IdentityService<S, I, C, A, G, E>,
    outcome: InitialSessionOutcome,
    lease: Option<SessionLease>,
}

impl<S: IdentitySnapshotStore, I, C, A, G, E> InitialLoginState<S, I, C, A, G, E> {
    pub(super) fn identity(&self) -> &IdentityService<S, I, C, A, G, E> {
        &self.identity
    }

    pub(super) fn new(identity: IdentityService<S, I, C, A, G, E>) -> Self {
        Self {
            identity,
            outcome: InitialSessionOutcome::Unattached,
            lease: None,
        }
    }

    /// Retain the opaque lease returned by this owner's one-shot C2 accept.
    /// The C2 callback has already attached the corresponding C1 Session.
    pub(super) fn retain_accepted_lease(&mut self, lease: SessionLease) -> Result<(), PeerDenial> {
        let InitialSessionOutcome::Attached {
            revalidation,
            selectors: (app_id, app_session_id),
            ..
        } = &self.outcome
        else {
            return Err(PeerDenial::Unavailable);
        };
        if self.lease.is_some() {
            return Err(PeerDenial::Unavailable);
        }
        if !lease.matches_session(*app_id, *app_session_id, revalidation.generation().get()) {
            return Err(PeerDenial::SessionMismatch);
        }
        self.lease = Some(lease);
        Ok(())
    }

    pub(super) fn capture(&self, selectors: (u64, u64)) -> Result<SessionLease, PeerDenial> {
        let InitialSessionOutcome::Attached {
            selectors: attached,
            ..
        } = &self.outcome
        else {
            return Err(PeerDenial::Unavailable);
        };
        let lease = self.lease.ok_or(PeerDenial::Unavailable)?;
        if selectors != *attached || !self.is_live(lease)? {
            return Err(PeerDenial::SessionMismatch);
        }
        Ok(lease)
    }

    pub(super) fn is_live(&self, lease: SessionLease) -> Result<bool, PeerDenial> {
        let InitialSessionOutcome::Attached { revalidation, .. } = &self.outcome else {
            return Ok(false);
        };
        if self.lease != Some(lease) {
            return Ok(false);
        }
        // This is the zero-wait observation required by C2's liveness API.
        // Protected effects still require the separate C1 effect gate.
        revalidation
            .try_live_snapshot()
            .map(|snapshot| snapshot.status() == LiveStatus::Live)
            .map_err(|_| PeerDenial::Unavailable)
    }
}

struct InitialSessionTransaction<'owner, S: IdentitySnapshotStore, I, C, A, G, E> {
    owner: MutexGuard<'owner, InitialLoginState<S, I, C, A, G, E>>,
    pending: C1PendingInitialInteractiveSession,
    stream: &'owner UnixStream,
    peer: &'owner KernelAuthenticatedUnixPeer,
    selectors: (u64, u64),
    expires_at: Option<UnixMillis>,
    request_key: Option<IdempotencyKey>,
    revalidation: Option<SessionLeaseRevalidationHandle>,
}

impl<S, I, C, A, G, E> PendingInitialSessionTransaction
    for InitialSessionTransaction<'_, S, I, C, A, G, E>
where
    S: IdentitySnapshotStore,
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    fn commit(&mut self) -> Result<NonZeroU64, PeerDenial> {
        let request_key = self.request_key.take().ok_or(PeerDenial::Unavailable)?;
        let issued = self
            .pending
            .commit_after_c2_owner_postcheck(&mut self.owner.identity, self.expires_at, request_key)
            .map_err(|_| PeerDenial::Unavailable)?;
        let caller = issued.caller();
        if !caller.matches_kernel_connection(self.peer, self.stream) {
            return Err(PeerDenial::StalePeer);
        }
        let context = issued
            .checked_context(&mut self.owner.identity)
            .map_err(|_| PeerDenial::Unavailable)?;
        let lease = context
            .revalidation_handle()
            .map_err(|_| PeerDenial::Unavailable)?;
        if lease.session_id() != issued.session_id()
            || lease.principal_id() != caller.principal_id()
            || lease.caller_context_id() != caller.context_id()
        {
            return Err(PeerDenial::SessionMismatch);
        }
        let generation = lease.generation();
        self.revalidation = Some(lease);
        Ok(generation)
    }

    fn abort(mut self: Box<Self>) -> Result<(), PeerDenial> {
        if self
            .pending
            .abort_before_c2_attachment(&mut self.owner.identity)
            .is_err()
        {
            self.owner.outcome = InitialSessionOutcome::RollbackRequired(self.pending);
            return Err(PeerDenial::RequestOutcomeUnknown);
        }
        Ok(())
    }

    fn finish(mut self: Box<Self>) {
        let Some(revalidation) = self.revalidation.take() else {
            self.owner.outcome = InitialSessionOutcome::RollbackRequired(self.pending);
            return;
        };
        self.owner.outcome = match self.pending.finish_after_c2_attachment() {
            Ok(issued) => InitialSessionOutcome::Attached {
                issued,
                revalidation,
                selectors: self.selectors,
            },
            Err(_) => InitialSessionOutcome::OutcomeUnknown,
        };
    }
}

/// Give C2's synchronous completion path the recovered C1 owner guard
/// through commit, rollback, or attachment. Only C2's accepted request can
/// supply credentials to this transaction.
pub(super) fn complete_initial_session<'peer, S, I, C, A, G, E>(
    owner: MutexGuard<'peer, InitialLoginState<S, I, C, A, G, E>>,
    verifier: &LinuxAccountVerifier,
    request: &'peer AcceptedInitialLoginRequest<'peer>,
    stream: &'peer UnixStream,
    peer: &'peer KernelAuthenticatedUnixPeer,
    proof: &C2AcceptedConnectionProof<'peer>,
    selectors: (u64, u64),
    expires_at: Option<UnixMillis>,
    request_key: IdempotencyKey,
    complete: &mut dyn FnMut(PendingInitialSession<'peer>) -> Result<(), PeerDenial>,
) -> Result<(), PeerDenial>
where
    S: IdentitySnapshotStore + 'peer,
    I: IdentityIdSource + 'peer,
    C: Clock + 'peer,
    A: IdentityAuthorizer + 'peer,
    G: GuestProfileCleaner + 'peer,
    E: IdentityEventSink + 'peer,
{
    if !matches!(owner.outcome, InitialSessionOutcome::Unattached) {
        return Err(PeerDenial::Unavailable);
    }
    peer.require_same_connection(stream)
        .map_err(|_| PeerDenial::StalePeer)?;
    if !proof.matches_kernel_peer(peer) {
        return Err(PeerDenial::StalePeer);
    }
    let evidence = verifier
        .authenticate(
            stream,
            request.account_name().as_bytes(),
            request.secret().as_bytes(),
        )
        .map_err(|_| PeerDenial::Unavailable)?;
    let pending = stage_initial_interactive_session_from_c2_owner_checked_peer(
        &owner.identity,
        verifier,
        evidence,
        peer,
        stream,
        proof,
    )
    .map_err(|_| PeerDenial::Unavailable)?;
    let transaction = InitialSessionTransaction {
        owner,
        pending,
        stream,
        peer,
        selectors,
        expires_at,
        request_key: Some(request_key),
        revalidation: None,
    };
    complete(PendingInitialSession::from_c2_pre_session_connection(
        stream,
        peer,
        proof,
        transaction,
    )?)
}
