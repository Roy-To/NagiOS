use super::broker::{
    AcceptedInitialLoginRequest, PeerDenial, PendingInitialSession,
    PendingInitialSessionTransaction, SessionLease,
};
use crate::linux_login_throttle::{LoginAttemptError, LoginFailureCount};
use nagi_c2_accepted_connection_proof_contract::C2AcceptedConnectionProof;
use nagi_identity_session::{
    stage_initial_interactive_session_from_c2_owner_checked_peer, C1InitialInteractiveSessionError,
    C1IssuedInteractiveSession, C1PendingInitialInteractiveSession, C1SystemServiceRecoveryJournal,
    C1SystemServiceRuntimeAdmission, Clock, GuestProfileCleaner, IdempotencyKey,
    IdentityAuthorizer, IdentityEventSink, IdentityIdSource, IdentityService,
    IdentitySnapshotStore, LiveStatus, SessionLeaseRevalidationHandle, TrustedCallerContext,
    UnixMillis,
};
use nagi_linux_account_verifier::{AccountAuthenticationError, LinuxAccountVerifier};
use nagi_peer_authenticated_connection::KernelAuthenticatedUnixPeer;
use std::num::NonZeroU64;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, MutexGuard};
use std::time::Instant;

pub enum InitialSessionOutcome {
    Unattached,
    Attached {
        issued: Arc<C1IssuedInteractiveSession>,
        revalidation: Arc<SessionLeaseRevalidationHandle>,
        selectors: (u64, u64),
    },
    RollbackRequired(C1PendingInitialInteractiveSession),
    OutcomeUnknown,
}

pub(super) struct InitialLoginState<S: IdentitySnapshotStore, I, C, A, G, E> {
    identity: IdentityService<S, I, C, A, G, E>,
    outcome: InitialSessionOutcome,
    lease: Option<SessionLease>,
    pub(super) login_failures: LoginFailureCount,
}

impl<S: IdentitySnapshotStore, I, C, A, G, E> InitialLoginState<S, I, C, A, G, E> {
    pub(super) fn identity(&self) -> &IdentityService<S, I, C, A, G, E> {
        &self.identity
    }

    pub(super) fn identity_mut(&mut self) -> &mut IdentityService<S, I, C, A, G, E> {
        &mut self.identity
    }

    pub(super) fn new(identity: IdentityService<S, I, C, A, G, E>, failures: u32) -> Self {
        Self {
            identity,
            outcome: InitialSessionOutcome::Unattached,
            lease: None,
            login_failures: LoginFailureCount::restored(failures),
        }
    }

    pub(super) fn require_unattached(&self) -> Result<(), PeerDenial> {
        match self.outcome {
            InitialSessionOutcome::Unattached if self.lease.is_none() => Ok(()),
            InitialSessionOutcome::RollbackRequired(_) | InitialSessionOutcome::OutcomeUnknown => {
                Err(PeerDenial::RequestOutcomeUnknown)
            }
            _ => Err(PeerDenial::SessionMismatch),
        }
    }

    /// Retain the opaque lease returned by this owner's one-shot C2 accept.
    /// The C2 callback has already attached the corresponding C1 Session.
    pub(super) fn retain_accepted_lease(&mut self, lease: SessionLease) -> Result<(), PeerDenial> {
        let (revalidation, app_id, app_session_id) = match &self.outcome {
            InitialSessionOutcome::Attached {
                revalidation,
                selectors: (app_id, app_session_id),
                ..
            } => (revalidation, app_id, app_session_id),
            InitialSessionOutcome::Unattached => return Err(PeerDenial::Unavailable),
            InitialSessionOutcome::RollbackRequired(_) | InitialSessionOutcome::OutcomeUnknown => {
                return Err(PeerDenial::RequestOutcomeUnknown);
            }
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

    pub(super) fn retained_revalidation(
        &self,
        lease: SessionLease,
    ) -> Result<Arc<SessionLeaseRevalidationHandle>, PeerDenial> {
        let InitialSessionOutcome::Attached { revalidation, .. } = &self.outcome else {
            return Err(PeerDenial::SessionUnavailable);
        };
        if self.lease != Some(lease) {
            return Err(PeerDenial::SessionMismatch);
        }
        Ok(Arc::clone(revalidation))
    }
}

impl<S, I, C, A, G, E> InitialLoginState<S, I, C, A, G, E>
where
    S: C1SystemServiceRecoveryJournal,
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    pub(super) fn request_notes_attachment(
        &mut self,
        lease: SessionLease,
        notes: &mut crate::profile_volume_notes_owner::AppStorageOwners,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<
        (
            crate::profile_volume_notes_owner::AuthorizedNotesAttachment,
            nagi_identity_session::ResolvedIdentity,
            Arc<C1IssuedInteractiveSession>,
        ),
        crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError,
    > {
        self.with_attached_profile_session(lease, admission, |issued, identity, _| {
            let attachment = notes.issue_for_attached_session(issued.caller(), &identity)?;
            Ok((attachment, identity, Arc::clone(issued)))
        })
    }

    pub(super) fn request_profile_snapshot(
        &mut self,
        lease: SessionLease,
        owners: &mut crate::profile_volume_notes_owner::AppStorageOwners,
        kind: crate::profile_volume_notes_owner::ProfileSnapshotKind,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<
        crate::profile_volume_notes_owner::AuthorizedProfileSnapshot,
        crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError,
    > {
        self.with_attached_profile_session(lease, admission, |issued, identity, revalidation| {
            owners.issue_snapshot_for_attached_session(
                issued.caller(),
                &identity,
                revalidation,
                kind,
            )
        })
    }

    pub(super) fn request_files_root(
        &mut self,
        lease: SessionLease,
        owners: &mut crate::profile_volume_notes_owner::AppStorageOwners,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<
        crate::profile_volume_notes_owner::AuthorizedFilesRoot,
        crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError,
    > {
        self.with_attached_profile_session(lease, admission, |issued, identity, revalidation| {
            owners.issue_files_root_for_attached_session(issued.caller(), &identity, revalidation)
        })
    }

    fn with_attached_profile_session<R>(
        &mut self,
        lease: SessionLease,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
        effect: impl FnOnce(
            &Arc<C1IssuedInteractiveSession>,
            nagi_identity_session::ResolvedIdentity,
            &Arc<SessionLeaseRevalidationHandle>,
        ) -> Result<
            R,
            crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError,
        >,
    ) -> Result<R, crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError> {
        use crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError::Unavailable;

        let revalidation = self.retained_revalidation(lease).map_err(|_| Unavailable)?;
        let InitialSessionOutcome::Attached { issued, .. } = &self.outcome else {
            return Err(Unavailable);
        };
        let identity = issued
            .checked_identity_with_runtime_admission(&mut self.identity, admission)
            .map_err(|_| Unavailable)?;
        if identity.session_id() != issued.session_id() {
            return Err(Unavailable);
        }
        revalidation
            .with_live_session_effect(|| effect(issued, identity, &revalidation))
            .map_err(|_| Unavailable)?
    }
}

struct InitialSessionTransaction<'owner, S: IdentitySnapshotStore, I, C, A, G, E> {
    owner: MutexGuard<'owner, InitialLoginState<S, I, C, A, G, E>>,
    pending: C1PendingInitialInteractiveSession,
    admission: C1SystemServiceRuntimeAdmission<'owner>,
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
    S: C1SystemServiceRecoveryJournal,
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
            .commit_after_c2_owner_postcheck_with_runtime_admission(
                &mut self.owner.identity,
                &self.admission,
                self.expires_at,
                request_key,
            )
            .map_err(|error| deny_initial_session("commit", error))?;
        super::super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_C1_SESSION_COMMITTED"));
        let caller = issued.caller();
        if !caller.matches_kernel_connection(self.peer, self.stream) {
            return Err(PeerDenial::StalePeer);
        }
        let context = issued
            .checked_context_with_runtime_admission(&mut self.owner.identity, &self.admission)
            .map_err(|error| deny_initial_session("checked-context", error))?;
        let lease = context.revalidation_handle().map_err(|error| {
            deny_initial_session(
                "revalidation",
                C1InitialInteractiveSessionError::Interactive(error),
            )
        })?;
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
        if let Err(error) = self
            .pending
            .abort_before_c2_attachment_with_runtime_admission(
                &mut self.owner.identity,
                &self.admission,
            )
        {
            deny_initial_session("rollback", error);
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
                issued: Arc::new(issued),
                revalidation: Arc::new(revalidation),
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
    mut owner: MutexGuard<'peer, InitialLoginState<S, I, C, A, G, E>>,
    admission: C1SystemServiceRuntimeAdmission<'peer>,
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
    S: C1SystemServiceRecoveryJournal + 'peer,
    I: IdentityIdSource + 'peer,
    C: Clock + 'peer,
    A: IdentityAuthorizer + 'peer,
    G: GuestProfileCleaner + 'peer,
    E: IdentityEventSink + 'peer,
{
    owner.require_unattached()?;
    peer.require_same_connection(stream)
        .map_err(|_| PeerDenial::StalePeer)?;
    if !proof.matches_kernel_peer(peer) {
        return Err(PeerDenial::StalePeer);
    }
    let evidence = owner
        .login_failures
        .attempt(
            Instant::now,
            || {
                verifier
                    .authenticate(
                        stream,
                        request.account_name().as_bytes(),
                        request.secret().as_bytes(),
                    )
                    .inspect_err(|error| {
                        super::super::log_pid1_stderr(format_args!(
                            "NAGI_DEV_TASK24_ACCOUNT_AUTHENTICATION_DENIED ({error:?})"
                        ));
                    })
            },
            |error| matches!(error, AccountAuthenticationError::Rejected),
            |count| verifier.store_login_failures(count),
        )
        .map_err(|error| match error {
            LoginAttemptError::Throttled => PeerDenial::AccountThrottled,
            LoginAttemptError::Verification(error) => account_denial(error),
            LoginAttemptError::PersistenceUnavailable => {
                super::super::log_pid1_stderr(format_args!(
                    "NAGI_DEV_TASK24_LOGIN_THROTTLE_PERSISTENCE_FAILED"
                ));
                PeerDenial::Unavailable
            }
        })?;
    super::super::log_pid1_stderr(format_args!("NAGI_DEV_TASK24_ACCOUNT_AUTHENTICATED"));
    let pending = stage_initial_interactive_session_from_c2_owner_checked_peer(
        &owner.identity,
        verifier,
        evidence,
        peer,
        stream,
        proof,
    )
    .map_err(|error| deny_initial_session("stage", error))?;
    let transaction = InitialSessionTransaction {
        owner,
        pending,
        admission,
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

fn account_denial(error: AccountAuthenticationError) -> PeerDenial {
    match error {
        AccountAuthenticationError::Rejected => PeerDenial::AccountRejected,
        AccountAuthenticationError::Unavailable
        | AccountAuthenticationError::InvalidRecord
        | AccountAuthenticationError::WrongVerifierOrConnection => PeerDenial::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task24_verifier_distinguishes_confirmed_rejection_from_unavailability() {
        use AccountAuthenticationError::*;
        assert!(matches!(
            account_denial(Rejected),
            PeerDenial::AccountRejected
        ));
        for error in [Unavailable, InvalidRecord, WrongVerifierOrConnection] {
            assert!(matches!(account_denial(error), PeerDenial::Unavailable));
        }
    }
}

fn deny_initial_session(stage: &str, error: C1InitialInteractiveSessionError) -> PeerDenial {
    use C1InitialInteractiveSessionError::*;
    let reason = match error {
        Account(_) => "account-binding".to_owned(),
        Peer(_) => "peer".to_owned(),
        ConnectionProofMismatch => "connection-proof-mismatch".to_owned(),
        IdentityIssuerMismatch => "issuer-mismatch".to_owned(),
        InvalidIdentityMapping => "invalid-identity-mapping".to_owned(),
        InvalidTransactionState => "invalid-transaction-state".to_owned(),
        Identity(ref error) => format!("{error:?}"),
        Interactive(ref error) => format!("{error:?}"),
    };
    super::super::log_pid1_stderr(format_args!(
        "NAGI_DEV_TASK24_C1_INITIAL_SESSION_DENIED stage={stage} reason={reason}"
    ));
    PeerDenial::Unavailable
}
