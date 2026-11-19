//! C2-owned completed service-handshake owner.
//!
//! The canonical boot plan must select a non-side-effecting operation for
//! each mandatory service and provide its typed request/response contract.
//! This module does not choose that operation or manufacture a required
//! service set. It accepts only such a plan contract, sends a real NIPC v1
//! request over the exact C2-accepted stream, validates the correlated
//! response, and retains the accepted connection plus its owner generations.

#[cfg(feature = "system-service-readiness")]
use super::Pid1SystemServiceReadyEvent;
use super::{
    map_peer_authentication_error, CompletedServiceHandshakeAuthority, Entry, LaunchRecordView,
    PeerCredentialSource, PeerDenial, PeerProcessInstanceAuthority, Pid1LivePeerDirectory,
    Pid1ReadinessAtomicBoundary, Pid1RunInstance, ProcessInstanceKey, RegistrationId,
    SupervisorLaunchAuthority, Task39PeerCredentialReader, TrustedRouteKey, TrustedServiceEndpoint,
    TrustedServiceEndpointAuthority,
};
use nagi_peer_authenticated_connection::{KernelAuthenticatedUnixPeer, PeerConnectionId};
use nagi_sdk::app_contract::{
    CorrelationId, IpcEnvelope, MessageKind, OrderingMetadata, RequestId, IPC_PROTOCOL_VERSION,
    MAX_IPC_PAYLOAD_BYTES,
};
use nagi_sdk::AppId;
use nagi_service_contract::CancellationToken;
use std::io::Read;
use std::os::unix::net::UnixStream;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[cfg(feature = "system-service-readiness")]
use nagi_system_principal_contract::SystemServiceEndpointOwnerProof;

const MAX_HANDSHAKE_ATTEMPTS: usize = super::MAX_LIVE_PEERS;
const MAX_NIPC_PAYLOAD_TYPE_BYTES: usize = 128;

/// Fresh wire identities allocated by C2 for one canonical handshake call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceHandshakeAttemptIds {
    request_id: RequestId,
    correlation_id: CorrelationId,
}

impl ServiceHandshakeAttemptIds {
    pub const fn request_id(self) -> RequestId {
        self.request_id
    }

    pub const fn correlation_id(self) -> CorrelationId {
        self.correlation_id
    }
}

/// Trusted service-plan contract for one handshake operation.
///
/// The implementation must be created from the canonical service descriptor
/// and signed boot plan. `route` must be the route registered for the exact
/// service/version/operation; the selected operation must be a bounded,
/// non-side-effecting readiness handshake. Payload builders and validators
/// are deterministic bounded CPU work and must not perform I/O or wait on
/// other owners. NIPC carries protocol version and AppId routing; the SVC
/// service/version/operation identity remains bound by `route`.
pub trait ServiceHandshakeProtocol: Send + Sync {
    fn route(&self) -> TrustedRouteKey;

    /// Return the C3 service slot selected by the verified boot plan. The C2
    /// owner cross-checks this identity against C3's live endpoint proof.
    fn system_service_slot(&self) -> Option<(&str, std::num::NonZeroU64)> {
        None
    }

    fn source_app_id(&self) -> AppId;

    fn destination_app_id(&self) -> AppId;

    fn request_payload_type(&self) -> &str;

    fn response_payload_type(&self) -> &str;

    fn ordering(&self) -> OrderingMetadata;

    fn build_request_payload(&self, ids: ServiceHandshakeAttemptIds)
        -> Result<Vec<u8>, PeerDenial>;

    fn validate_response_payload(&self, payload: &[u8]) -> Result<(), PeerDenial>;
}

#[derive(Clone, Copy)]
enum HandshakePrincipal<'a> {
    Interactive(std::marker::PhantomData<&'a ()>),
    #[cfg(feature = "system-service-readiness")]
    SystemService(&'a SystemServiceEndpointOwnerProof),
}

/// The production owner records an attempt before wire I/O and grants
/// completion only after the accepted connection returns a validated,
/// correlated NIPC Response. An incomplete/unknown attempt cannot be retried
/// for the same registration/process/route/endpoint tuple.
pub struct Pid1CompletedServiceHandshakeOwner<'a, P: PeerProcessInstanceAuthority> {
    run: Pid1RunInstance,
    process_instances: &'a P,
    attempts: Mutex<Vec<HandshakeAttemptRecord>>,
}

impl<'a, P: PeerProcessInstanceAuthority> Pid1CompletedServiceHandshakeOwner<'a, P> {
    pub fn new(run: &Pid1RunInstance, process_instances: &'a P) -> Self {
        Self {
            run: run.clone(),
            process_instances,
            attempts: Mutex::new(Vec::with_capacity(MAX_HANDSHAKE_ATTEMPTS)),
        }
    }

    /// Execute the plan-selected handshake against the current registered
    /// process. Process, route, and endpoint are resolved from the live C2
    /// entry; callers provide only its registration and canonical protocol
    /// contract. Success retains the same accepted stream used for the NIPC
    /// request and response so READY can revalidate it later.
    #[allow(clippy::too_many_arguments)]
    pub fn complete_handshake<R, E, A, B, H>(
        &self,
        directory: &Pid1LivePeerDirectory<R>,
        run: &Pid1RunInstance,
        readiness_boundary: &B,
        registration: RegistrationId,
        endpoints: &E,
        supervisor: &A,
        protocol: &H,
        deadline: Instant,
    ) -> Result<(), PeerDenial>
    where
        R: LaunchRecordView,
        E: TrustedServiceEndpointAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
        H: ServiceHandshakeProtocol + ?Sized,
    {
        self.complete_handshake_for_principal(
            directory,
            run,
            readiness_boundary,
            registration,
            endpoints,
            supervisor,
            protocol,
            HandshakePrincipal::Interactive(std::marker::PhantomData),
            deadline,
        )
    }

    /// Use C2's registered service listener while keeping the C3 proof on its
    /// separate control endpoint. The same accepted service stream is retained
    /// after the correlated NIPC response validates.
    #[cfg(feature = "system-service-readiness")]
    #[allow(clippy::too_many_arguments)]
    pub fn complete_system_service_handshake<R, E, A, B, H>(
        &self,
        directory: &Pid1LivePeerDirectory<R>,
        run: &Pid1RunInstance,
        readiness_boundary: &B,
        registration: RegistrationId,
        proof: &SystemServiceEndpointOwnerProof,
        endpoints: &E,
        supervisor: &A,
        protocol: &H,
        deadline: Instant,
    ) -> Result<(), PeerDenial>
    where
        R: LaunchRecordView,
        E: TrustedServiceEndpointAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
        H: ServiceHandshakeProtocol + ?Sized,
    {
        self.complete_handshake_for_principal(
            directory,
            run,
            readiness_boundary,
            registration,
            endpoints,
            supervisor,
            protocol,
            HandshakePrincipal::SystemService(proof),
            deadline,
        )
    }

    /// Build the C2 READY evidence with the same endpoint authority used by
    /// the preceding SystemService handshake. The caller may hold this event
    /// before READY, but must consume it inside C1's checked Session gate.
    #[cfg(feature = "system-service-readiness")]
    pub(crate) fn system_service_ready_event<'b, R, E, A, B, H>(
        &'b self,
        directory: &'b Pid1LivePeerDirectory<R>,
        run: &'b Pid1RunInstance,
        proof: &'b SystemServiceEndpointOwnerProof,
        endpoints: &'b E,
        supervisor: &'b A,
        readiness_boundary: &'b B,
        plan: &'b H,
    ) -> Result<Pid1SystemServiceReadyEvent<'b>, PeerDenial>
    where
        R: LaunchRecordView,
        E: TrustedServiceEndpointAuthority + 'b,
        A: SupervisorLaunchAuthority<Record = R> + 'b,
        B: Pid1ReadinessAtomicBoundary + 'b,
        H: ServiceHandshakeProtocol + ?Sized + 'b,
    {
        directory.system_service_ready_event(
            run,
            proof,
            endpoints,
            self.process_instances,
            self,
            supervisor,
            readiness_boundary,
            plan,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn complete_handshake_for_principal<R, E, A, B, H>(
        &self,
        directory: &Pid1LivePeerDirectory<R>,
        run: &Pid1RunInstance,
        readiness_boundary: &B,
        registration: RegistrationId,
        endpoints: &E,
        supervisor: &A,
        protocol: &H,
        principal: HandshakePrincipal<'_>,
        deadline: Instant,
    ) -> Result<(), PeerDenial>
    where
        R: LaunchRecordView,
        E: TrustedServiceEndpointAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
        H: ServiceHandshakeProtocol + ?Sized,
    {
        require_same_run(&self.run, run)?;
        if Instant::now() >= deadline {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        let route = protocol.route();
        #[cfg(feature = "system-service-readiness")]
        if let HandshakePrincipal::SystemService(proof) = principal {
            let Some((service_id, catalog_generation)) = protocol.system_service_slot() else {
                return Err(PeerDenial::InvalidNipcRequest);
            };
            let slot = proof.service_slot_identity();
            if slot.service_id() != service_id || slot.catalog_generation() != catalog_generation {
                return Err(PeerDenial::StaleRegistration);
            }
        }
        let entry =
            self.current_entry(directory, run, registration, route, supervisor, principal)?;
        let endpoint = entry.endpoint;
        self.validate_external_owners(run, &entry, endpoints, supervisor, deadline)?;

        if protocol.source_app_id().0 == 0
            || protocol.destination_app_id().0 == 0
            || protocol.destination_app_id().0 != entry.record.app_id()
            || protocol.request_payload_type().is_empty()
            || protocol.request_payload_type().len() > MAX_NIPC_PAYLOAD_TYPE_BYTES
            || protocol.response_payload_type().is_empty()
            || protocol.response_payload_type().len() > MAX_NIPC_PAYLOAD_TYPE_BYTES
        {
            return Err(PeerDenial::InvalidNipcRequest);
        }

        let mut checkout = endpoints.accept_checkout_until(endpoint, deadline)?;
        if checkout.endpoint != endpoint
            || checkout.endpoint_epoch != endpoint.endpoint_generation
            || checkout.expires_at > deadline
            || !checkout
                .recipient
                .is_same_recipient(&endpoints.checkout_recipient(endpoint)?)
        {
            return Err(PeerDenial::StaleEndpoint);
        }
        let deadline = checkout.expires_at;
        if Instant::now() >= deadline {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        let observation = Task39PeerCredentialReader.read(&checkout.stream)?;
        let peer = observation
            .authenticated_peer
            .as_ref()
            .cloned()
            .ok_or(PeerDenial::StalePeer)?;
        self.validate_peer(
            run,
            &entry,
            &checkout.stream,
            peer.as_ref(),
            endpoints,
            supervisor,
            principal,
            false,
            deadline,
        )?;

        let ids = fresh_attempt_ids()?;
        let payload = protocol.build_request_payload(ids)?;
        if payload.len() > MAX_IPC_PAYLOAD_BYTES {
            return Err(PeerDenial::InvalidNipcRequest);
        }
        let request = IpcEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            source: protocol.source_app_id(),
            destination: Some(protocol.destination_app_id()),
            kind: MessageKind::Request,
            correlation_id: ids.correlation_id,
            request_id: Some(ids.request_id),
            ordering: protocol.ordering(),
            payload_type: protocol.request_payload_type(),
            payload: &payload,
        };
        let key = HandshakeAttemptKey {
            run: run.clone(),
            registration,
            process: entry.process,
            route,
            endpoint,
        };
        self.begin_attempt(key.clone(), ids)?;
        let mut attempt = HandshakeAttemptGuard {
            owner: self,
            key: key.clone(),
            armed: true,
        };

        if Instant::now() >= deadline {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        let timeout = deadline.saturating_duration_since(Instant::now());
        let mut reply_storage = Vec::new();
        let reply = match nagi_init_service_call::call_unix_stream(
            &mut checkout.stream,
            &request,
            &mut reply_storage,
            &CancellationToken::new(),
            timeout,
        ) {
            Ok(nagi_nipc_v1_stream_candidate::CallReply::Response(response)) => response,
            Ok(nagi_nipc_v1_stream_candidate::CallReply::Error { .. }) => {
                attempt.mark_failed()?;
                return Err(PeerDenial::HandshakeIncomplete);
            }
            Err(_) => return Err(PeerDenial::RequestOutcomeUnknown),
        };
        if !response_matches(&request, &reply, protocol.response_payload_type()) {
            attempt.mark_failed()?;
            return Err(PeerDenial::HandshakeIncomplete);
        }
        if protocol.validate_response_payload(reply.payload).is_err() {
            attempt.mark_failed()?;
            return Err(PeerDenial::HandshakeIncomplete);
        }
        if Instant::now() >= deadline {
            return Err(PeerDenial::RequestOutcomeUnknown);
        }
        self.validate_peer(
            run,
            &entry,
            &checkout.stream,
            peer.as_ref(),
            endpoints,
            supervisor,
            principal,
            false,
            deadline,
        )?;
        let response_peer = peer.as_ref();
        let stream = checkout.stream;
        let completion = CompletedHandshakeConnection {
            peer: Arc::clone(&peer),
            connection_id: response_peer.connection_id(),
            request_id: ids.request_id,
            correlation_id: ids.correlation_id,
            stream: Mutex::new(stream),
        };

        if !run
            .readiness_boundary()
            .is_same_domain(readiness_boundary.authority_domain())
        {
            return Err(PeerDenial::StaleRun);
        }
        let finalize = || {
            readiness_boundary.with_owner_gate(run, None, || {
                let current = self.current_entry_without_proof(
                    directory,
                    run,
                    registration,
                    route,
                    supervisor,
                )?;
                self.require_principal_binding(&current, principal, false)?;
                if current.process != entry.process || current.endpoint != endpoint {
                    return Err(PeerDenial::StaleRegistration);
                }
                let stream = completion
                    .stream
                    .try_lock()
                    .map_err(|_| PeerDenial::Unavailable)?;
                self.validate_peer(
                    run,
                    &current,
                    &stream,
                    completion.peer.as_ref(),
                    endpoints,
                    supervisor,
                    principal,
                    true,
                    deadline,
                )?;
                drop(stream);
                attempt.complete(completion)
            })
        };
        match principal {
            HandshakePrincipal::Interactive(_) => finalize(),
            #[cfg(feature = "system-service-readiness")]
            HandshakePrincipal::SystemService(proof) => proof
                .with_live_authority_effect(finalize)
                .map_err(|_| PeerDenial::StalePeer)?,
        }
    }

    fn current_entry<R, A>(
        &self,
        directory: &Pid1LivePeerDirectory<R>,
        run: &Pid1RunInstance,
        registration: RegistrationId,
        route: TrustedRouteKey,
        supervisor: &A,
        principal: HandshakePrincipal<'_>,
    ) -> Result<Entry<R>, PeerDenial>
    where
        R: LaunchRecordView,
        A: SupervisorLaunchAuthority<Record = R>,
    {
        require_same_run(&self.run, run)?;
        directory.require_current_run(run)?;
        let entry =
            self.current_entry_without_proof(directory, run, registration, route, supervisor)?;
        self.require_principal_binding(&entry, principal, true)?;
        Ok(entry)
    }

    fn current_entry_without_proof<R, A>(
        &self,
        directory: &Pid1LivePeerDirectory<R>,
        run: &Pid1RunInstance,
        registration: RegistrationId,
        route: TrustedRouteKey,
        supervisor: &A,
    ) -> Result<Entry<R>, PeerDenial>
    where
        R: LaunchRecordView,
        A: SupervisorLaunchAuthority<Record = R>,
    {
        require_same_run(&self.run, run)?;
        directory.require_current_run(run)?;
        let entry = directory.snapshot(registration)?;
        if entry.run_generation != run.generation()
            || entry.route != route
            || !entry.is_live()
            || entry
                .initial_session_issuance_pending
                .load(super::Ordering::SeqCst)
            || entry.record.process_id() != entry.process.pid()
            || supervisor.resolve(entry.record.process_id()) != Some(entry.record)
        {
            return Err(PeerDenial::StaleRegistration);
        }
        Ok(entry)
    }

    fn require_principal_binding<R: LaunchRecordView>(
        &self,
        entry: &Entry<R>,
        principal: HandshakePrincipal<'_>,
        check_system_proof_live: bool,
    ) -> Result<(), PeerDenial> {
        match principal {
            HandshakePrincipal::Interactive(_) => {
                if entry.session.is_none() || entry.system_service_owner.is_some() {
                    return Err(PeerDenial::StaleRegistration);
                }
            }
            #[cfg(feature = "system-service-readiness")]
            HandshakePrincipal::SystemService(proof) => {
                let Some(binding) = entry.system_service_owner.as_ref() else {
                    return Err(PeerDenial::StaleRegistration);
                };
                let identity = proof.process_identity();
                let slot = proof.service_slot_identity();
                if entry.session.is_some()
                    || binding.run_generation != entry.run_generation
                    || !binding
                        .process_identity
                        .same_instance(&entry.task24_identity)
                    || !binding.process_identity.same_instance(&identity)
                    || binding.service_id != slot.service_id()
                    || binding.catalog_generation != slot.catalog_generation()
                    || binding.c3_launch_generation != proof.launch_generation()
                    || binding.c3_control_endpoint_generation != proof.endpoint_generation()
                    || binding.c2_service_endpoint_generation != entry.endpoint.endpoint_generation
                {
                    return Err(PeerDenial::StaleRegistration);
                }
                if check_system_proof_live && !proof.is_live().map_err(|_| PeerDenial::StalePeer)? {
                    return Err(PeerDenial::StalePeer);
                }
            }
        }
        Ok(())
    }

    fn validate_external_owners<R, E, A>(
        &self,
        run: &Pid1RunInstance,
        entry: &Entry<R>,
        endpoints: &E,
        supervisor: &A,
        deadline: Instant,
    ) -> Result<(), PeerDenial>
    where
        R: LaunchRecordView,
        E: TrustedServiceEndpointAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
    {
        if Instant::now() >= deadline || !run.is_active() {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        if !endpoints.is_current(entry.endpoint)? {
            return Err(PeerDenial::StaleEndpoint);
        }
        if !endpoints.fd_delegation_is_prevented(entry.endpoint)? {
            return Err(PeerDenial::FdDelegationNotPrevented);
        }
        if supervisor.resolve(entry.record.process_id()) != Some(entry.record) {
            return Err(PeerDenial::NotLive);
        }
        if self.process_instances.has_unresolved_cleanup()? {
            return Err(PeerDenial::UnresolvedSpawnCleanup);
        }
        if !self
            .process_instances
            .confirms_spawned_instance_live(entry.process)?
        {
            return Err(PeerDenial::StalePeer);
        }
        Ok(())
    }

    fn validate_peer<R, E, A>(
        &self,
        run: &Pid1RunInstance,
        entry: &Entry<R>,
        stream: &UnixStream,
        peer: &KernelAuthenticatedUnixPeer,
        endpoints: &E,
        supervisor: &A,
        principal: HandshakePrincipal<'_>,
        system_proof_gate_held: bool,
        deadline: Instant,
    ) -> Result<(), PeerDenial>
    where
        R: LaunchRecordView,
        E: TrustedServiceEndpointAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
    {
        if Instant::now() >= deadline {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        peer.revalidate(stream)
            .map_err(map_peer_authentication_error)?;
        if peer.pid() != entry.process.pid()
            || entry.record.process_id() != peer.pid()
            || !self.process_instances.confirms_same_live_process(
                stream,
                peer.pid(),
                peer.uid(),
                peer.gid(),
                entry.process,
            )?
        {
            return Err(PeerDenial::StalePeer);
        }
        self.validate_external_owners(run, entry, endpoints, supervisor, deadline)?;
        self.require_principal_binding(entry, principal, !system_proof_gate_held)
    }

    fn begin_attempt(
        &self,
        key: HandshakeAttemptKey,
        ids: ServiceHandshakeAttemptIds,
    ) -> Result<(), PeerDenial> {
        let mut attempts = self
            .attempts
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        if attempts.iter().any(|attempt| attempt.key.same_target(&key)) {
            return Err(PeerDenial::RequestOutcomeUnknown);
        }
        if attempts.len() >= MAX_HANDSHAKE_ATTEMPTS {
            return Err(PeerDenial::Capacity);
        }
        attempts.try_reserve(1).map_err(|_| PeerDenial::Capacity)?;
        attempts.push(HandshakeAttemptRecord {
            key,
            request_id: ids.request_id,
            correlation_id: ids.correlation_id,
            state: HandshakeAttemptState::InFlight,
            completion: None,
        });
        Ok(())
    }

    fn mark_unknown(&self, key: &HandshakeAttemptKey) {
        if let Ok(mut attempts) = self.attempts.try_lock() {
            if let Some(attempt) = attempts
                .iter_mut()
                .find(|attempt| attempt.key.same_target(key))
            {
                if attempt.state == HandshakeAttemptState::InFlight {
                    attempt.state = HandshakeAttemptState::OutcomeUnknown;
                }
            }
        }
    }

    fn mark_failed(&self, key: &HandshakeAttemptKey) -> Result<(), PeerDenial> {
        let mut attempts = self
            .attempts
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        let attempt = attempts
            .iter_mut()
            .find(|attempt| attempt.key.same_target(key))
            .ok_or(PeerDenial::StaleRegistration)?;
        if attempt.state != HandshakeAttemptState::InFlight {
            return Err(PeerDenial::RequestOutcomeUnknown);
        }
        attempt.state = HandshakeAttemptState::Failed;
        Ok(())
    }

    fn mark_complete(
        &self,
        key: &HandshakeAttemptKey,
        completion: CompletedHandshakeConnection,
    ) -> Result<(), PeerDenial> {
        let mut attempts = self
            .attempts
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        let attempt = attempts
            .iter_mut()
            .find(|attempt| attempt.key.same_target(key))
            .ok_or(PeerDenial::StaleRegistration)?;
        if attempt.state != HandshakeAttemptState::InFlight {
            return Err(PeerDenial::RequestOutcomeUnknown);
        }
        attempt.state = HandshakeAttemptState::Complete;
        attempt.completion = Some(completion);
        Ok(())
    }
}

impl<P: PeerProcessInstanceAuthority> super::handshake_authority_seal::Sealed
    for Pid1CompletedServiceHandshakeOwner<'_, P>
{
}

impl<P: PeerProcessInstanceAuthority> CompletedServiceHandshakeAuthority
    for Pid1CompletedServiceHandshakeOwner<'_, P>
{
    fn confirms_completed_handshake(
        &self,
        run: &Pid1RunInstance,
        registration: RegistrationId,
        process: ProcessInstanceKey,
        route: TrustedRouteKey,
        endpoint: TrustedServiceEndpoint,
    ) -> Result<bool, PeerDenial> {
        if !self.run.is_same_run(run) || !run.is_active() {
            return Ok(false);
        }
        let attempts = self
            .attempts
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        let Some(attempt) = attempts.iter().find(|attempt| {
            attempt.state == HandshakeAttemptState::Complete
                && attempt
                    .key
                    .same_query(run, registration, process, route, endpoint)
        }) else {
            return Ok(false);
        };
        let Some(completion) = attempt.completion.as_ref() else {
            return Err(PeerDenial::HandshakeIncomplete);
        };
        if attempt.request_id != completion.request_id
            || attempt.correlation_id != completion.correlation_id
        {
            return Err(PeerDenial::HandshakeIncomplete);
        }
        if completion.peer.connection_id() != completion.connection_id {
            return Ok(false);
        }
        let stream = completion
            .stream
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        completion
            .peer
            .revalidate(&stream)
            .map_err(map_peer_authentication_error)?;
        if !self.process_instances.confirms_same_live_process(
            &stream,
            completion.peer.pid(),
            completion.peer.uid(),
            completion.peer.gid(),
            process,
        )? || !self
            .process_instances
            .confirms_spawned_instance_live(process)?
        {
            return Ok(false);
        }
        Ok(true)
    }
}

#[derive(Clone)]
struct HandshakeAttemptKey {
    run: Pid1RunInstance,
    registration: RegistrationId,
    process: ProcessInstanceKey,
    route: TrustedRouteKey,
    endpoint: TrustedServiceEndpoint,
}

impl HandshakeAttemptKey {
    fn same_target(&self, other: &Self) -> bool {
        self.run.is_same_run(&other.run)
            && self.registration == other.registration
            && self.process == other.process
            && self.route == other.route
            && self.endpoint == other.endpoint
    }

    fn same_query(
        &self,
        run: &Pid1RunInstance,
        registration: RegistrationId,
        process: ProcessInstanceKey,
        route: TrustedRouteKey,
        endpoint: TrustedServiceEndpoint,
    ) -> bool {
        self.run.is_same_run(run)
            && self.registration == registration
            && self.process == process
            && self.route == route
            && self.endpoint == endpoint
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum HandshakeAttemptState {
    InFlight,
    OutcomeUnknown,
    Failed,
    Complete,
}

struct HandshakeAttemptRecord {
    key: HandshakeAttemptKey,
    request_id: RequestId,
    correlation_id: CorrelationId,
    state: HandshakeAttemptState,
    completion: Option<CompletedHandshakeConnection>,
}

struct CompletedHandshakeConnection {
    peer: Arc<KernelAuthenticatedUnixPeer>,
    connection_id: PeerConnectionId,
    request_id: RequestId,
    correlation_id: CorrelationId,
    stream: Mutex<UnixStream>,
}

struct HandshakeAttemptGuard<'owner, 'process, P: PeerProcessInstanceAuthority> {
    owner: &'owner Pid1CompletedServiceHandshakeOwner<'process, P>,
    key: HandshakeAttemptKey,
    armed: bool,
}

impl<P: PeerProcessInstanceAuthority> HandshakeAttemptGuard<'_, '_, P> {
    fn mark_failed(&mut self) -> Result<(), PeerDenial> {
        self.owner.mark_failed(&self.key)?;
        self.armed = false;
        Ok(())
    }

    fn complete(&mut self, completion: CompletedHandshakeConnection) -> Result<(), PeerDenial> {
        self.owner.mark_complete(&self.key, completion)?;
        self.armed = false;
        Ok(())
    }
}

impl<P: PeerProcessInstanceAuthority> Drop for HandshakeAttemptGuard<'_, '_, P> {
    fn drop(&mut self) {
        if self.armed {
            self.owner.mark_unknown(&self.key);
        }
    }
}

fn require_same_run(
    expected: &Pid1RunInstance,
    current: &Pid1RunInstance,
) -> Result<(), PeerDenial> {
    if expected.is_same_run(current) && current.is_active() {
        Ok(())
    } else {
        Err(PeerDenial::StaleRun)
    }
}

fn fresh_attempt_ids() -> Result<ServiceHandshakeAttemptIds, PeerDenial> {
    let mut bytes = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut bytes))
        .map_err(|_| PeerDenial::Unavailable)?;
    let mut request = [0_u8; 16];
    request.copy_from_slice(&bytes[..16]);
    let mut correlation = [0_u8; 16];
    correlation.copy_from_slice(&bytes[16..]);
    if request.iter().all(|byte| *byte == 0) || correlation.iter().all(|byte| *byte == 0) {
        return Err(PeerDenial::Unavailable);
    }
    Ok(ServiceHandshakeAttemptIds {
        request_id: RequestId(request),
        correlation_id: CorrelationId(correlation),
    })
}

fn response_matches(
    request: &IpcEnvelope<'_>,
    response: &IpcEnvelope<'_>,
    expected_payload_type: &str,
) -> bool {
    response.protocol_version == IPC_PROTOCOL_VERSION
        && response.kind == MessageKind::Response
        && response.source == request.destination.unwrap_or(AppId(0))
        && response.destination == Some(request.source)
        && response.correlation_id == request.correlation_id
        && response.request_id == request.request_id
        && response.payload_type == expected_payload_type
}

#[cfg(test)]
mod tests {
    use super::{
        response_matches, AppId, CorrelationId, IpcEnvelope, MessageKind, OrderingMetadata,
    };
    use nagi_sdk::app_contract::{RequestId, IPC_PROTOCOL_VERSION};

    fn request() -> IpcEnvelope<'static> {
        IpcEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            source: AppId(1),
            destination: Some(AppId(2)),
            kind: MessageKind::Request,
            correlation_id: CorrelationId([3; 16]),
            request_id: Some(RequestId([4; 16])),
            ordering: OrderingMetadata {
                timestamp_micros: None,
                sequence: None,
            },
            payload_type: "nagi.handshake@1",
            payload: b"request",
        }
    }

    fn response(request: &IpcEnvelope<'_>) -> IpcEnvelope<'static> {
        IpcEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            source: request.destination.expect("request destination"),
            destination: Some(request.source),
            kind: MessageKind::Response,
            correlation_id: request.correlation_id,
            request_id: request.request_id,
            ordering: request.ordering,
            payload_type: "nagi.handshake.reply@1",
            payload: b"response",
        }
    }

    #[test]
    fn completed_handshake_requires_exact_correlated_response() {
        let request = request();
        let response = response(&request);
        assert!(response_matches(
            &request,
            &response,
            "nagi.handshake.reply@1"
        ));

        assert!(!response_matches(
            &request,
            &IpcEnvelope {
                correlation_id: CorrelationId([5; 16]),
                ..response
            },
            "nagi.handshake.reply@1"
        ));
        assert!(!response_matches(
            &request,
            &IpcEnvelope {
                request_id: Some(RequestId([6; 16])),
                ..response
            },
            "nagi.handshake.reply@1"
        ));
        assert!(!response_matches(
            &request,
            &IpcEnvelope {
                source: AppId(7),
                ..response
            },
            "nagi.handshake.reply@1"
        ));
        assert!(!response_matches(
            &request,
            &IpcEnvelope {
                destination: Some(AppId(8)),
                ..response
            },
            "nagi.handshake.reply@1"
        ));
        assert!(!response_matches(
            &request,
            &IpcEnvelope {
                protocol_version: IPC_PROTOCOL_VERSION + 1,
                ..response
            },
            "nagi.handshake.reply@1"
        ));
        assert!(!response_matches(
            &request,
            &IpcEnvelope {
                kind: MessageKind::Request,
                ..response
            },
            "nagi.handshake.reply@1"
        ));
        assert!(!response_matches(
            &request,
            &IpcEnvelope {
                payload_type: "nagi.other.reply@1",
                ..response
            },
            "nagi.handshake.reply@1"
        ));
    }
}
