use super::linux_initial_login_owner::LinuxInitialLoginOwner;
#[cfg(linux_task24_owner_co_location)]
use super::pid1_live_peer_supplier as broker;
use broker::{
    AcceptedInitialLoginRequest, ConsentSource, LaunchRecordView, LiveSessionAuthority, PeerDenial,
    PeerProcessInstanceAuthority, PendingInitialSession, Pid1LivePeerDirectory,
    Pid1ReadinessAtomicBoundary, SessionLease, SupervisorLaunchAuthority,
    Task24EndpointLaunchLease,
};
use nagi_c2_accepted_connection_proof_contract::C2AcceptedConnectionProof;
use nagi_identity_session::{
    C1SystemServiceRecoveryJournal, C1SystemServiceRuntimeAdmission, C1SystemServiceRuntimeHandoff,
    Clock, GuestProfileCleaner, IdempotencyKey, IdentityAuthorizer, IdentityError,
    IdentityEventSink, IdentityIdSource, IdentityService, IdentitySnapshotStore, UnixMillis,
};
use nagi_linux_account_verifier::LinuxAccountVerifier;
use nagi_peer_authenticated_connection::KernelAuthenticatedUnixPeer;
#[cfg(not(linux_task24_owner_co_location))]
use nagi_pid1_live_peer_supplier as broker;
use nagi_system_service_launch::SystemServiceRecoveryAdmission;
use nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope;
use std::os::unix::net::UnixStream;
use std::sync::Mutex;
use std::time::Instant;

#[path = "linux_initial_login_transaction.rs"]
mod transaction;

/// Own the recovered issuer and its verifier for the first attached C2 lease.
pub(super) struct C1InitialLoginAuthority<'boot, S: IdentitySnapshotStore, I, C, A, G, E> {
    owner: Mutex<transaction::InitialLoginState<S, I, C, A, G, E>>,
    handoff: C1SystemServiceRuntimeHandoff,
    boot_admission: &'boot SystemServiceRecoveryAdmission,
    verifier: LinuxAccountVerifier,
    request_key: IdempotencyKey,
    expires_at: Option<UnixMillis>,
}

impl<'boot, S, I, C, A, G, E> C1InitialLoginAuthority<'boot, S, I, C, A, G, E>
where
    S: C1SystemServiceRecoveryJournal,
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    pub(super) fn new(
        identity: IdentityService<S, I, C, A, G, E>,
        handoff: C1SystemServiceRuntimeHandoff,
        boot_admission: &'boot SystemServiceRecoveryAdmission,
        verifier: LinuxAccountVerifier,
        request_key: IdempotencyKey,
        expires_at: Option<UnixMillis>,
    ) -> Result<Self, IdentityError> {
        identity.validate_system_service_runtime_handoff(&handoff)?;
        let failures = verifier.load_login_failures();
        Ok(Self {
            owner: Mutex::new(transaction::InitialLoginState::new(identity, failures)),
            handoff,
            boot_admission,
            verifier,
            request_key,
            expires_at,
        })
    }

    pub(super) fn login_failures(&self) -> std::io::Result<u32> {
        self.owner
            .try_lock()
            .map(|owner| owner.login_failures.current())
            .map_err(|_| std::io::Error::other("login throttle owner unavailable"))
    }

    pub(super) fn reset_login_failures(&self) -> std::io::Result<()> {
        self.owner
            .try_lock()
            .map_err(|_| std::io::Error::other("login throttle owner unavailable"))?
            .login_failures
            .persist(0, |count| self.verifier.store_login_failures(count))
    }

    pub(super) fn require_unattached(&self) -> Result<(), PeerDenial> {
        self.owner
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .require_unattached()
    }

    fn validate_runtime(&self) -> Result<(), PeerDenial> {
        self.owner
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .identity()
            .validate_system_service_runtime_handoff(&self.handoff)
            .map_err(|_| PeerDenial::Unavailable)
    }

    pub(super) fn request_notes_attachment(
        &self,
        lease: SessionLease,
        notes: &mut crate::profile_volume_notes_owner::AppStorageOwners,
    ) -> Result<
        (
            crate::profile_volume_notes_owner::AuthorizedNotesAttachment,
            nagi_identity_session::ResolvedIdentity,
            std::sync::Arc<nagi_identity_session::C1IssuedInteractiveSession>,
        ),
        crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError,
    > {
        use crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError::Unavailable;

        let mut owner = self.owner.try_lock().map_err(|_| Unavailable)?;
        let admission = owner
            .identity_mut()
            .enter_system_service_runtime_admission(&self.handoff, self.boot_admission.scope())
            .map_err(|_| Unavailable)?;
        owner.request_notes_attachment(lease, notes, &admission)
    }

    pub(super) fn request_settings_snapshot(
        &self,
        lease: SessionLease,
        owners: &mut crate::profile_volume_notes_owner::AppStorageOwners,
    ) -> Result<
        crate::profile_volume_notes_owner::AuthorizedProfileSnapshot,
        crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError,
    > {
        use crate::profile_volume_notes_owner::ProfileVolumeNotesOwnerError::Unavailable;

        let mut owner = self.owner.try_lock().map_err(|_| Unavailable)?;
        let admission = owner
            .identity_mut()
            .enter_system_service_runtime_admission(&self.handoff, self.boot_admission.scope())
            .map_err(|_| Unavailable)?;
        owner.request_settings_snapshot(lease, owners, &admission)
    }

    pub(super) fn enter_runtime_admission<'runtime>(
        &'runtime self,
        scope: &'runtime dyn SystemServiceRecoveryAdmissionScope,
    ) -> Result<C1SystemServiceRuntimeAdmission<'runtime>, IdentityError> {
        let mut owner = self
            .owner
            .try_lock()
            .map_err(|_| IdentityError::StoreUnavailable)?;
        owner
            .identity_mut()
            .enter_system_service_runtime_admission(&self.handoff, scope)
    }

    pub(super) fn with_identity_for_terminal<T>(
        &self,
        reconcile: impl FnOnce(&mut IdentityService<S, I, C, A, G, E>) -> std::io::Result<T>,
    ) -> std::io::Result<T> {
        let mut owner = self.owner.try_lock().map_err(|_| {
            std::io::Error::other("C1 runtime unavailable during terminal reconciliation")
        })?;
        reconcile(owner.identity_mut())
    }

    #[cfg(linux_task24_notes_consumer)]
    pub(super) fn check_notes_session(
        &self,
        lease: SessionLease,
    ) -> Result<(), nagi_identity_session::ResolutionError> {
        use nagi_identity_session::ResolutionError;
        let revalidation = self
            .owner
            .try_lock()
            .map_err(|_| ResolutionError::Unavailable)?
            .retained_revalidation(lease)
            .map_err(|_| ResolutionError::Unavailable)?;
        revalidation.check_live()
    }

    /// Hold C1's exact attached Session across the final Ready consumer.
    /// Release the issuer mutex before entering the live guard so the C2
    /// callback can still perform its independent zero-wait observations.
    pub(super) fn with_ready_session<T>(
        &self,
        lease: SessionLease,
        consume: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        let revalidation = self
            .owner
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .retained_revalidation(lease)?;
        revalidation
            .with_live_session_effect(consume)
            .map_err(|_| PeerDenial::SessionRevoked)?
    }
}

impl<S, I, C, A, G, E> LiveSessionAuthority for C1InitialLoginAuthority<'_, S, I, C, A, G, E>
where
    S: C1SystemServiceRecoveryJournal + Send,
    I: IdentityIdSource + Send,
    C: Clock,
    A: IdentityAuthorizer + Send,
    G: GuestProfileCleaner + Send,
    E: IdentityEventSink + Send,
{
    fn capture_live_unlocked(
        &self,
        app_id: u64,
        app_session_id: u64,
    ) -> Result<SessionLease, PeerDenial> {
        self.owner
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .capture((app_id, app_session_id))
    }

    fn is_live_unlocked(&self, lease: SessionLease) -> Result<bool, PeerDenial> {
        self.owner
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .is_live(lease)
    }

    fn with_pending_initial_session_from_c2_owner_checked_peer<'peer>(
        &'peer self,
        app_id: u64,
        app_session_id: u64,
        stream: &'peer UnixStream,
        peer: &'peer KernelAuthenticatedUnixPeer,
        connection_proof: &C2AcceptedConnectionProof<'peer>,
        request: &'peer AcceptedInitialLoginRequest<'peer>,
        complete: &mut dyn FnMut(PendingInitialSession<'peer>) -> Result<(), PeerDenial>,
    ) -> Result<(), PeerDenial> {
        let mut owner = self.owner.try_lock().map_err(|_| PeerDenial::Unavailable)?;
        let admission = owner
            .identity_mut()
            .enter_system_service_runtime_admission(&self.handoff, self.boot_admission.scope())
            .map_err(|_| PeerDenial::Unavailable)?;
        transaction::complete_initial_session(
            owner,
            admission,
            &self.verifier,
            request,
            stream,
            peer,
            connection_proof,
            (app_id, app_session_id),
            self.expires_at,
            self.request_key.clone(),
            complete,
        )
    }
}

/// Starts login via the configured Task24 owner after C1 recovers. Task24
/// retains the child's lease and remains its exact cleanup owner.
pub(super) struct C2InitialLogin<'run, R: LaunchRecordView, C, P, A, B, Start> {
    pub(super) directory: &'run Pid1LivePeerDirectory<R>,
    pub(super) start_login_child: Option<Start>,
    pub(super) consent: &'run C,
    pub(super) process_instances: &'run P,
    pub(super) supervisor: &'run A,
    pub(super) readiness_boundary: &'run B,
    pub(super) deadline: Instant,
}

impl<'run, 'boot, R, S, I, C, IA, G, E, Consent, P, Supervisor, B, Start>
    LinuxInitialLoginOwner<C1InitialLoginAuthority<'boot, S, I, C, IA, G, E>>
    for C2InitialLogin<'run, R, Consent, P, Supervisor, B, Start>
where
    R: LaunchRecordView + 'run,
    S: C1SystemServiceRecoveryJournal + Send,
    I: IdentityIdSource + Send,
    C: Clock,
    IA: IdentityAuthorizer + Send,
    G: GuestProfileCleaner + Send,
    E: IdentityEventSink + Send,
    Consent: ConsentSource,
    P: PeerProcessInstanceAuthority,
    Supervisor: SupervisorLaunchAuthority<Record = R>,
    B: Pid1ReadinessAtomicBoundary,
    Start: FnOnce(Instant) -> Result<&'run Task24EndpointLaunchLease<R>, PeerDenial>,
{
    type Attached = SessionLease;
    type Error = PeerDenial;

    fn start_and_accept(
        &mut self,
        sessions: &C1InitialLoginAuthority<'boot, S, I, C, IA, G, E>,
    ) -> Result<SessionLease, PeerDenial> {
        let start = self
            .start_login_child
            .take()
            .ok_or(PeerDenial::Unavailable)?;
        if Instant::now() >= self.deadline {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        sessions.validate_runtime()?;
        let launch = start(self.deadline)?;
        let lease = launch.accept_initial_session_until(
            self.directory,
            sessions,
            self.consent,
            self.process_instances,
            self.supervisor,
            self.readiness_boundary,
            self.deadline,
        )?;
        // The C2 admission gates have released. A concurrent zero-wait
        // reader must not strand an already-attached C1 Session.
        sessions
            .owner
            .lock()
            .map_err(|_| PeerDenial::RequestOutcomeUnknown)?
            .retain_accepted_lease(lease)?;
        Ok(lease)
    }
}
