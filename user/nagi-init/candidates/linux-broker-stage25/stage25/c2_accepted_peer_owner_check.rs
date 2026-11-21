use super::{
    map_peer_authentication_error, EndpointCheckoutRecipient, LaunchRecordView, PeerDenial,
    PeerProcessInstanceAuthority, Pid1LivePeerDirectory, Pid1RunInstance, ProcessInstanceKey,
    RegistrationId, SupervisorLaunchAuthority, TrustedServiceEndpoint,
    TrustedServiceEndpointAuthority,
};
use nagi_c2_accepted_connection_proof_contract::C2AcceptedConnectionProof;
use nagi_init_spawn_contract::SpawnedProcessIdentity;
use nagi_peer_authenticated_connection::{
    AuthenticatedInteractivePeer, AuthenticatedInteractivePeerSelector,
    KernelAuthenticatedUnixPeer, PeerConnectionId, TrustedInteractivePeerKey,
};
use std::os::unix::net::UnixStream;
use std::time::Instant;

/// Borrowed C2-root proof that a leaf-issued accepted peer matches its live,
/// spawn-validated endpoint owner. Only this module's validation routine can
/// mint it. Its contents remain private to C2; A receives the later app
/// admission and passes the lower leaf proof with the shared connection
/// proof to A for C1's independent comparison.
pub(super) struct C2SpawnEndpointAdmission<'admission, 'connection> {
    peer: &'admission AuthenticatedInteractivePeer<'connection>,
    connection_proof: C2AcceptedConnectionProof<'admission>,
    stream: &'admission UnixStream,
    run: &'admission Pid1RunInstance,
    registration: RegistrationId,
    process: ProcessInstanceKey,
    endpoint: TrustedServiceEndpoint,
    endpoint_epoch: std::num::NonZeroU64,
    recipient: &'admission EndpointCheckoutRecipient,
    task24_identity: &'admission SpawnedProcessIdentity,
    expires_at: Instant,
    connection_id: PeerConnectionId,
    _not_send_or_sync: std::marker::PhantomData<std::rc::Rc<()>>,
}

/// Borrowed C2-root admission for the first session handoff. This path uses
/// only the kernel-authenticated accepted stream and the exact C2
/// spawn/endpoint owner; it does not need a SessionLease or a leaf selector.
/// Only this module's pre-session validator can construct it.
pub(super) struct C2PreSessionSpawnEndpointAdmission<'admission> {
    peer: &'admission KernelAuthenticatedUnixPeer,
    connection_proof: C2AcceptedConnectionProof<'admission>,
    stream: &'admission UnixStream,
    run: &'admission Pid1RunInstance,
    process: ProcessInstanceKey,
    endpoint: TrustedServiceEndpoint,
    endpoint_epoch: std::num::NonZeroU64,
    recipient: &'admission EndpointCheckoutRecipient,
    task24_identity: &'admission SpawnedProcessIdentity,
    expires_at: Instant,
}

impl<'admission> C2PreSessionSpawnEndpointAdmission<'admission> {
    fn from_validated_owner(
        peer: &'admission KernelAuthenticatedUnixPeer,
        stream: &'admission UnixStream,
        run: &'admission Pid1RunInstance,
        process: ProcessInstanceKey,
        endpoint: TrustedServiceEndpoint,
        endpoint_epoch: std::num::NonZeroU64,
        recipient: &'admission EndpointCheckoutRecipient,
        task24_identity: &'admission SpawnedProcessIdentity,
        expires_at: Instant,
    ) -> Self {
        Self {
            peer,
            connection_proof: C2AcceptedConnectionProof::after_c2_owner_check(peer),
            stream,
            run,
            process,
            endpoint,
            endpoint_epoch,
            recipient,
            task24_identity,
            expires_at,
        }
    }

    fn matches_peer(&self, peer: &KernelAuthenticatedUnixPeer) -> bool {
        std::ptr::eq(self.peer, peer)
            && self.connection_proof.matches_kernel_peer(peer)
            && peer.pid() == self.process.pid()
            && self.task24_identity.process_id() == self.process.pid()
            && self.endpoint.endpoint_generation == self.endpoint_epoch
            && self.recipient.endpoint == self.endpoint
            && self.run.is_active()
            && Instant::now() < self.expires_at
            && peer.revalidate(self.stream).is_ok()
    }

    /// Share only the same-connection correlation value with C1 while this
    /// validated C2 admission remains borrowed. C1 performs its own peer and
    /// account/session checks on the exact stream.
    pub(super) fn connection_proof(&self) -> &C2AcceptedConnectionProof<'admission> {
        &self.connection_proof
    }
}

impl<'admission, 'connection> C2SpawnEndpointAdmission<'admission, 'connection> {
    fn from_validated_owner(
        peer: &'admission AuthenticatedInteractivePeer<'connection>,
        stream: &'admission UnixStream,
        run: &'admission Pid1RunInstance,
        registration: RegistrationId,
        process: ProcessInstanceKey,
        endpoint: TrustedServiceEndpoint,
        endpoint_epoch: std::num::NonZeroU64,
        recipient: &'admission EndpointCheckoutRecipient,
        task24_identity: &'admission SpawnedProcessIdentity,
        expires_at: Instant,
    ) -> Self {
        Self {
            peer,
            connection_proof: C2AcceptedConnectionProof::after_c2_owner_check(peer.kernel_peer()),
            stream,
            run,
            registration,
            process,
            endpoint,
            endpoint_epoch,
            recipient,
            task24_identity,
            expires_at,
            connection_id: peer.kernel_peer().connection_id(),
            _not_send_or_sync: std::marker::PhantomData,
        }
    }

    /// Confirm the borrowed leaf proof is the same accepted connection and
    /// C2 owner tuple this module validated.
    pub(super) fn matches_peer(&self, peer: &AuthenticatedInteractivePeer<'_>) -> bool {
        let selector = peer.selector();
        std::ptr::eq(self.peer.kernel_peer(), peer.kernel_peer())
            && selector == self.peer.selector()
            && peer.owner_key() == self.peer.owner_key()
            && peer.kernel_peer().connection_id() == self.connection_id
            && selector.matches_c2_binding(
                *peer.owner_key(),
                self.registration.0,
                self.run.generation(),
                self.endpoint_epoch,
                self.process.instance,
            )
            && self.endpoint.endpoint_generation == self.endpoint_epoch
            && self.recipient.endpoint == self.endpoint
            && self.run.is_active()
            && Instant::now() < self.expires_at
            && self
                .peer
                .kernel_peer()
                .require_same_connection(self.stream)
                .is_ok()
            && self.task24_identity.process_id() == self.process.pid()
    }

    pub(super) fn connection_proof(&self) -> &C2AcceptedConnectionProof<'admission> {
        &self.connection_proof
    }

    /// Bind the trusted launch's Session selector after C2 has validated the
    /// exact spawn/endpoint owner. The A root must still establish C1's live
    /// Session owner.
    pub(super) fn bind_app_session_selector<'binding>(
        &'binding self,
        peer: &AuthenticatedInteractivePeer<'_>,
        lease: super::SessionLease,
    ) -> Result<super::C2AppSessionAdmission<'binding, 'admission, 'connection>, PeerDenial> {
        if !self.matches_peer(peer) || !lease.matches_authenticated_peer_selector(peer.selector()) {
            return Err(PeerDenial::SessionMismatch);
        }
        Ok(super::C2AppSessionAdmission { owner: self, lease })
    }
}

/// Recheck the leaf peer and C2-owned identity/liveness facts surrounding one
/// staged C1 binding. This check runs before C1 begins and after C1 returns its
/// pending transaction; it does not activate C1 enrollment or construct a
/// caller identity.
pub(super) fn validate_current_owner<'admission, 'connection, R, E, P, A>(
    directory: &Pid1LivePeerDirectory<R>,
    run: &'admission Pid1RunInstance,
    registration: RegistrationId,
    process: ProcessInstanceKey,
    record: R,
    endpoint: TrustedServiceEndpoint,
    endpoint_epoch: std::num::NonZeroU64,
    recipient: &'admission EndpointCheckoutRecipient,
    task24_identity: &'admission SpawnedProcessIdentity,
    expires_at: Instant,
    peer_pid: u32,
    peer_uid: u32,
    peer_gid: u32,
    authenticated_peer: &'admission AuthenticatedInteractivePeer<'connection>,
    stream: &'admission UnixStream,
    endpoints: &E,
    supervisor: &A,
    process_instances: &P,
) -> Result<C2SpawnEndpointAdmission<'admission, 'connection>, PeerDenial>
where
    R: LaunchRecordView,
    E: TrustedServiceEndpointAuthority,
    P: PeerProcessInstanceAuthority,
    A: SupervisorLaunchAuthority<Record = R>,
{
    let kernel_peer = authenticated_peer.kernel_peer();
    if Instant::now() >= expires_at {
        return Err(PeerDenial::RequestDeadlineExpired);
    }
    kernel_peer
        .revalidate(stream)
        .map_err(map_peer_authentication_error)?;
    if kernel_peer.pid() != peer_pid
        || kernel_peer.uid() != peer_uid
        || kernel_peer.gid() != peer_gid
    {
        return Err(PeerDenial::StalePeer);
    }
    validate_current_owner_selector(
        directory,
        run,
        registration,
        process,
        record,
        endpoint,
        endpoint_epoch,
        recipient,
        task24_identity,
        expires_at,
        peer_pid,
        peer_uid,
        peer_gid,
        authenticated_peer.selector(),
        Some(authenticated_peer.owner_key()),
        endpoints,
        supervisor,
        process_instances,
    )?;
    Ok(C2SpawnEndpointAdmission::from_validated_owner(
        authenticated_peer,
        stream,
        run,
        registration,
        process,
        endpoint,
        endpoint_epoch,
        recipient,
        task24_identity,
        expires_at,
    ))
}

/// Validate the C2-owned accepted stream and spawn/endpoint tuple before C1
/// has issued the first Session. This path deliberately does not require a
/// leaf interactive selector; the endpoint checkout, kernel peer, Supervisor
/// record, and live spawned-process identity bind the accepted connection.
#[allow(clippy::too_many_arguments)]
pub(super) fn validate_current_pre_session_owner<'admission, R, E, P, A>(
    directory: &Pid1LivePeerDirectory<R>,
    run: &'admission Pid1RunInstance,
    registration: RegistrationId,
    process: ProcessInstanceKey,
    record: R,
    endpoint: TrustedServiceEndpoint,
    endpoint_epoch: std::num::NonZeroU64,
    recipient: &'admission EndpointCheckoutRecipient,
    task24_identity: &'admission SpawnedProcessIdentity,
    expires_at: Instant,
    peer: &'admission KernelAuthenticatedUnixPeer,
    stream: &'admission UnixStream,
    endpoints: &E,
    supervisor: &A,
    process_instances: &P,
) -> Result<C2PreSessionSpawnEndpointAdmission<'admission>, PeerDenial>
where
    R: LaunchRecordView,
    E: TrustedServiceEndpointAuthority,
    P: PeerProcessInstanceAuthority,
    A: SupervisorLaunchAuthority<Record = R>,
{
    if Instant::now() >= expires_at {
        return Err(PeerDenial::RequestDeadlineExpired);
    }
    if endpoint.endpoint_generation != endpoint_epoch {
        return Err(PeerDenial::StaleEndpoint);
    }
    peer.revalidate(stream)
        .map_err(map_peer_authentication_error)?;
    directory.require_current_peer_proof_owner(
        run,
        registration,
        process,
        record,
        endpoint,
        task24_identity,
    )?;
    if !endpoints.is_current(endpoint)?
        || !endpoints
            .checkout_recipient(endpoint)?
            .is_same_recipient(recipient)
    {
        return Err(PeerDenial::StaleEndpoint);
    }
    if !endpoints.fd_delegation_is_prevented(endpoint)? {
        return Err(PeerDenial::FdDelegationNotPrevented);
    }
    let peer_pid = peer.pid();
    if record.process_id() != peer_pid
        || process.pid() != peer_pid
        || task24_identity.process_id() != peer_pid
        || supervisor.resolve(peer_pid) != Some(record)
    {
        return Err(PeerDenial::StalePeer);
    }
    if !process_instances.confirms_same_live_process(
        stream,
        peer_pid,
        peer.uid(),
        peer.gid(),
        process,
    )? || !process_instances.confirms_spawned_instance_live(process)?
    {
        return Err(PeerDenial::StalePeer);
    }
    Ok(C2PreSessionSpawnEndpointAdmission::from_validated_owner(
        peer,
        stream,
        run,
        process,
        endpoint,
        endpoint_epoch,
        recipient,
        task24_identity,
        expires_at,
    ))
}

/// Revalidate C2's exact selector and owner state after the leaf callback has
/// completed its own kernel-peer and listener-owner postcheck. Keeping this
/// check separate lets C2 retain the selector value and defer activation until
/// both owners have finished their postchecks.
#[allow(clippy::too_many_arguments)]
pub(super) fn validate_current_owner_selector<R, E, P, A>(
    directory: &Pid1LivePeerDirectory<R>,
    run: &Pid1RunInstance,
    registration: RegistrationId,
    process: ProcessInstanceKey,
    record: R,
    endpoint: TrustedServiceEndpoint,
    endpoint_epoch: std::num::NonZeroU64,
    recipient: &EndpointCheckoutRecipient,
    task24_identity: &SpawnedProcessIdentity,
    expires_at: Instant,
    peer_pid: u32,
    peer_uid: u32,
    peer_gid: u32,
    selector: &AuthenticatedInteractivePeerSelector,
    proof_owner_key: Option<&TrustedInteractivePeerKey>,
    endpoints: &E,
    supervisor: &A,
    process_instances: &P,
) -> Result<(), PeerDenial>
where
    R: LaunchRecordView,
    E: TrustedServiceEndpointAuthority,
    P: PeerProcessInstanceAuthority,
    A: SupervisorLaunchAuthority<Record = R>,
{
    if Instant::now() >= expires_at {
        return Err(PeerDenial::RequestDeadlineExpired);
    }
    if endpoint.endpoint_generation != endpoint_epoch {
        return Err(PeerDenial::StaleEndpoint);
    }
    directory.require_current_peer_proof_owner(
        run,
        registration,
        process,
        record,
        endpoint,
        task24_identity,
    )?;
    if !endpoints.is_current(endpoint)?
        || !endpoints
            .checkout_recipient(endpoint)?
            .is_same_recipient(recipient)
    {
        return Err(PeerDenial::StaleEndpoint);
    }
    let expected_owner_key = endpoints.accepted_peer_owner_key(endpoint)?;
    if !selector.matches_c2_binding(
        expected_owner_key,
        registration.0,
        run.generation(),
        endpoint.endpoint_generation,
        process.instance,
    ) || proof_owner_key.is_some_and(|owner_key| owner_key != &expected_owner_key)
    {
        return Err(PeerDenial::StaleEndpoint);
    }
    if !endpoints.fd_delegation_is_prevented(endpoint)? {
        return Err(PeerDenial::FdDelegationNotPrevented);
    }
    if record.process_id() != peer_pid
        || process.pid() != peer_pid
        || task24_identity.process_id() != peer_pid
        || supervisor.resolve(peer_pid) != Some(record)
    {
        return Err(PeerDenial::StalePeer);
    }
    if !process_instances.confirms_spawned_instance_live(process)? {
        return Err(PeerDenial::StalePeer);
    }
    Ok(())
}
