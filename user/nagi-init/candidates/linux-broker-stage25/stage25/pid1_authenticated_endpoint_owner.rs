//! Spawn-bound endpoint owner and the fail-closed SVC provider boundary.
//!
//! The owner accepts listener input only from a crate-internal spawn path that
//! already holds a non-forgeable `Pid1SpawnProof`. It assigns endpoint epoch,
//! recipient and a bounded expiry itself, and keeps the exact process-instance
//! token from that proof. The listener now enables SO_PASSCRED and task-39
//! validates each writer against the accepted peer. Production authorization
//! still denies until C3 supplies spawn-bound evidence that the child cannot
//! forge SCM_CREDENTIALS; this file does not publish a service descriptor or
//! return a stream to the SVC caller while that proof is absent.

use super::pidfd_process_instance::SpawnOwnedEndpointBinding;
use super::{
    bind_task39_transfer_to_endpoint_checkout, endpoint_authority_seal,
    C2AcceptedPeerListenerOwner, EndpointCheckout, EndpointCheckoutRecipient, LaunchRecordView,
    OwnedPeerConnection, PeerDenial, PeerProcessInstanceAuthority, Pid1RunInstance, Pid1SpawnProof,
    ProcessInstanceKey, RegisteredEndpointOwnerCandidate, SupervisorLaunchAuthority,
    TrustedRouteKey, TrustedServiceEndpoint, TrustedServiceEndpointAuthority,
};
use nagi_init_service_call::AuthenticatedUnixStreamProvider;
use nagi_init_spawn_contract::SpawnedProcessIdentity;
use nagi_service_contract::{
    ContractVersion, IpcError, IpcErrorCode, OperationId, ServiceDescriptor, ServiceDiscovery,
    ServiceId,
};
use std::collections::{HashMap, HashSet};
use std::num::NonZeroU64;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ENDPOINT_LEASE_TTL: Duration = Duration::from_secs(300);
const MAX_ENDPOINT_COOKIE_HISTORY: usize = 1024;

struct EndpointOwnerRecord {
    run: Pid1RunInstance,
    process: ProcessInstanceKey,
    spawn_identity: Option<SpawnedProcessIdentity>,
    fd_delegation_attested: AtomicBool,
    endpoint: TrustedServiceEndpoint,
    recipient: EndpointCheckoutRecipient,
    expires_at: Instant,
    revoked: AtomicBool,
    revoke_attempted: AtomicBool,
    listener_closed: AtomicBool,
    // Clone the owner while holding this slot briefly; accept_until can poll
    // without blocking key lookup or revocation on the endpoint record.
    listener: Mutex<Option<Arc<C2AcceptedPeerListenerOwner>>>,
    seen_socket_cookies: Mutex<HashSet<NonZeroU64>>,
}

#[derive(Default)]
struct RegistryState {
    next_generation: u64,
    by_route: HashMap<TrustedRouteKey, Arc<EndpointOwnerRecord>>,
}

struct RegistryInner {
    state: Mutex<RegistryState>,
}

/// PID-1-owned namespace for live endpoint registrations. It starts empty;
/// ordinary callers cannot add a listener or provide numeric process identity.
pub struct Pid1EndpointOwnerRegistry {
    inner: Arc<RegistryInner>,
}

impl Default for Pid1EndpointOwnerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Pid1EndpointOwnerRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                state: Mutex::new(RegistryState {
                    next_generation: 1,
                    by_route: HashMap::new(),
                }),
            }),
        }
    }

    /// Consume the opaque listener/child pairing issued by the pidfd-backed
    /// spawn owner. This prevents separately substituting a proof, route, and
    /// raw listener at the registry boundary. Returns the same single-use
    /// spawn proof for the existing live-peer registry to consume; it is not
    /// minted a second time here.
    pub(crate) fn install_from_spawn<R, P, A>(
        &self,
        run: &Pid1RunInstance,
        binding: SpawnOwnedEndpointBinding<R, UnixListener>,
        process_instances: &P,
        supervisor: &A,
    ) -> Result<(Pid1SpawnEndpointLease, Pid1SpawnProof<R>), PeerDenial>
    where
        R: LaunchRecordView,
        P: PeerProcessInstanceAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
    {
        let SpawnOwnedEndpointBinding {
            proof: spawn,
            route,
            listener,
        } = binding;
        if !run.is_active() || spawn.process.pid() != spawn.record.process_id() {
            return Err(PeerDenial::InvalidProcess);
        }
        let spawn_identity = spawn
            .task24_identity()
            .cloned()
            .ok_or(PeerDenial::ForeignSpawnOwner)?;
        if spawn_identity.process_id() != spawn.process.pid() {
            return Err(PeerDenial::ForeignSpawnOwner);
        }
        if supervisor.resolve(spawn.process.pid()) != Some(spawn.record) {
            return Err(PeerDenial::NotLive);
        }
        if process_instances.has_unresolved_cleanup()? {
            return Err(PeerDenial::UnresolvedSpawnCleanup);
        }
        if !process_instances.confirms_spawned_instance_live(spawn.process)? {
            return Err(PeerDenial::StalePeer);
        }
        let expires_at = Instant::now()
            .checked_add(ENDPOINT_LEASE_TTL)
            .ok_or(PeerDenial::GenerationExhausted)?;
        let lease = self.install_record(
            run.clone(),
            spawn.process,
            Some(spawn_identity),
            route.route_key(),
            expires_at,
            Some(listener),
        )?;
        Ok((lease, spawn))
    }

    fn install_record(
        &self,
        run: Pid1RunInstance,
        process: ProcessInstanceKey,
        spawn_identity: Option<SpawnedProcessIdentity>,
        route: TrustedRouteKey,
        expires_at: Instant,
        listener: Option<UnixListener>,
    ) -> Result<Pid1SpawnEndpointLease, PeerDenial> {
        if !run.is_active() || Instant::now() >= expires_at {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        let mut state = self
            .inner
            .state
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        if let Some(previous) = state.by_route.get(&route) {
            if previous.is_current() {
                return Err(PeerDenial::StaleEndpoint);
            }
            previous.revoke()?;
            state.by_route.remove(&route);
        }
        if state.by_route.len() >= super::MAX_LIVE_PEERS {
            return Err(PeerDenial::Capacity);
        }
        let generation = state.next_generation;
        state.next_generation = generation
            .checked_add(1)
            .ok_or(PeerDenial::GenerationExhausted)?;
        let endpoint = TrustedServiceEndpoint::from_service_registry(route, generation, generation)
            .ok_or(PeerDenial::GenerationExhausted)?;
        let recipient = EndpointCheckoutRecipient::from_registered_owner(endpoint);
        let listener = listener
            .map(|listener| {
                C2AcceptedPeerListenerOwner::register_from_c2_owner(
                    listener,
                    run.generation(),
                    endpoint.endpoint_generation,
                    process.instance,
                    expires_at,
                )
            })
            .transpose()
            .map_err(|error| PeerDenial::KernelPeerAuthentication(Arc::new(error)))?
            .map(Arc::new);
        let record = Arc::new(EndpointOwnerRecord {
            run,
            process,
            spawn_identity,
            fd_delegation_attested: AtomicBool::new(false),
            endpoint,
            recipient,
            expires_at,
            revoked: AtomicBool::new(false),
            revoke_attempted: AtomicBool::new(false),
            listener_closed: AtomicBool::new(false),
            listener: Mutex::new(listener),
            seen_socket_cookies: Mutex::new(HashSet::new()),
        });
        state.by_route.insert(route, Arc::clone(&record));
        Ok(Pid1SpawnEndpointLease {
            registry: Arc::clone(&self.inner),
            record,
            auto_revoke_on_drop: AtomicBool::new(true),
        })
    }

    /// Revoke every endpoint bound to this exact pidfd-backed process token.
    /// The ProcessOs/reap owner should call this after its exact Supervisor
    /// record is revoked and child cleanup is complete. A reused numeric PID
    /// cannot target these records because callers cannot construct the token.
    pub fn revoke_process_instance(
        &self,
        process: ProcessInstanceKey,
    ) -> Result<usize, PeerDenial> {
        let mut state = self
            .inner
            .state
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        let mut revoked = 0;
        let mut revoke_error = None;
        state.by_route.retain(|_, record| {
            if record.process != process {
                return true;
            }
            match record.revoke() {
                Ok(()) => {
                    revoked += 1;
                    false
                }
                Err(error) => {
                    // Keep an unresolved owner record so replacement cannot
                    // hide a listener-close failure.
                    revoke_error = Some(error);
                    true
                }
            }
        });
        if let Some(error) = revoke_error {
            return Err(error);
        }
        Ok(revoked)
    }

    #[cfg(test)]
    fn install_test_record(
        &self,
        run: Pid1RunInstance,
        process: ProcessInstanceKey,
        route: TrustedRouteKey,
        expires_at: Instant,
    ) -> Result<Pid1SpawnEndpointLease, PeerDenial> {
        self.install_record(run, process, None, route, expires_at, None)
    }
}

/// Non-cloneable lease that owns the registration lifetime. Its Drop/revoke
/// path invalidates the exact recipient and epoch; a later installation gets
/// a strictly newer generation. It also pins the process-instance key, never
/// just the numeric PID.
pub struct Pid1SpawnEndpointLease {
    registry: Arc<RegistryInner>,
    record: Arc<EndpointOwnerRecord>,
    auto_revoke_on_drop: AtomicBool,
}

impl Pid1SpawnEndpointLease {
    pub fn endpoint(&self) -> TrustedServiceEndpoint {
        self.record.endpoint
    }

    /// Opaque process-instance handle for the Task24 reap owner. It is a
    /// cleanup/association token, not independent authentication evidence.
    pub fn process_instance(&self) -> ProcessInstanceKey {
        self.record.process
    }

    pub fn recipient(&self) -> EndpointCheckoutRecipient {
        self.record.recipient.clone()
    }

    pub fn expires_at(&self) -> Instant {
        self.record.expires_at
    }

    /// Persist that C3's opaque non-delegation attestation matched this exact
    /// endpoint lease's clone3 identity. This method is crate-internal and is
    /// called only by Task24's typed attestation activation path.
    pub(crate) fn record_fd_delegation_attestation(
        &self,
        process_identity: &SpawnedProcessIdentity,
    ) -> Result<(), PeerDenial> {
        self.require_current(self.record.endpoint)?;
        let Some(spawn_identity) = self.record.spawn_identity.as_ref() else {
            return Err(PeerDenial::ForeignSpawnOwner);
        };
        if !spawn_identity.same_instance(process_identity)
            || process_identity.process_id() != self.record.process.pid()
        {
            return Err(PeerDenial::ForeignSpawnOwner);
        }
        self.record
            .fd_delegation_attested
            .store(true, Ordering::SeqCst);
        Ok(())
    }

    pub fn revoke(&self) -> Result<(), PeerDenial> {
        // A returned error is outcome-unknown. Do not let Drop issue a second
        // revocation after this attempt.
        self.auto_revoke_on_drop.store(false, Ordering::SeqCst);
        self.record.revoke()?;
        let mut state = self
            .registry
            .state
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        if state
            .by_route
            .get(&self.record.endpoint.route)
            .is_some_and(|current| Arc::ptr_eq(current, &self.record))
        {
            state.by_route.remove(&self.record.endpoint.route);
        }
        Ok(())
    }

    /// Mark the one endpoint-revocation attempt owned by Task24's exact reap
    /// callback. Disabling Drop retry before attempting the operation keeps an
    /// error or panic as OutcomeUnknown rather than silently retrying it.
    pub(crate) fn revoke_once_for_owner_retirement(&mut self) -> Result<(), PeerDenial> {
        self.auto_revoke_on_drop.store(false, Ordering::SeqCst);
        self.revoke()
    }

    /// Task24's single reap owner should consume this lease after the exact
    /// child wait and Supervisor-record revocation. This only revokes the
    /// endpoint; it does not certify that a reap occurred. Calling early is a
    /// safe denial, while dropping the lease also revokes it.
    pub(crate) fn revoke_after_exact_child_reap(mut self) -> Result<(), PeerDenial> {
        self.revoke_once_for_owner_retirement()
    }

    fn is_current_record(&self) -> Result<bool, PeerDenial> {
        if !self.record.is_current() {
            self.revoke()?;
            return Ok(false);
        }
        let state = self
            .registry
            .state
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        Ok(state
            .by_route
            .get(&self.record.endpoint.route)
            .is_some_and(|current| Arc::ptr_eq(current, &self.record)))
    }

    fn require_current(&self, endpoint: TrustedServiceEndpoint) -> Result<(), PeerDenial> {
        if endpoint != self.record.endpoint {
            return Err(PeerDenial::StaleEndpoint);
        }
        if Instant::now() >= self.record.expires_at {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        if !self.is_current_record()? {
            return Err(PeerDenial::StaleEndpoint);
        }
        Ok(())
    }

    fn claim_socket_cookie(&self, cookie: NonZeroU64) -> Result<(), PeerDenial> {
        let mut seen = self
            .record
            .seen_socket_cookies
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        if seen.contains(&cookie) {
            return Err(PeerDenial::StaleSocket);
        }
        if seen.len() >= MAX_ENDPOINT_COOKIE_HISTORY {
            return Err(PeerDenial::Capacity);
        }
        seen.insert(cookie);
        Ok(())
    }
}

impl Drop for Pid1SpawnEndpointLease {
    fn drop(&mut self) {
        if self.auto_revoke_on_drop.swap(false, Ordering::SeqCst) {
            let _ = self.revoke();
        }
    }
}

impl EndpointOwnerRecord {
    fn is_current(&self) -> bool {
        !self.revoked.load(Ordering::SeqCst)
            && self.run.is_active()
            && Instant::now() < self.expires_at
    }

    fn revoke(&self) -> Result<(), PeerDenial> {
        self.revoked.store(true, Ordering::SeqCst);
        self.fd_delegation_attested.store(false, Ordering::SeqCst);
        if self.listener_closed.load(Ordering::SeqCst) {
            return Ok(());
        }
        if self.revoke_attempted.swap(true, Ordering::SeqCst) {
            return Err(PeerDenial::Unavailable);
        }
        // Relinquish the endpoint record's listener reference before calling
        // into the leaf. Cleanup stays complete even if the leaf's best-effort
        // revoke reports contention; in-flight acceptors own temporary Arcs.
        let listener = self
            .listener
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        self.listener_closed.store(true, Ordering::SeqCst);
        let result = listener.as_ref().map_or(Ok(()), |owner| {
            owner
                .revoke()
                .map_err(|error| PeerDenial::KernelPeerAuthentication(Arc::new(error)))
        });
        drop(listener);
        result
    }
}

impl endpoint_authority_seal::Sealed for Pid1SpawnEndpointLease {}

impl TrustedServiceEndpointAuthority for Pid1SpawnEndpointLease {
    fn is_current(&self, endpoint: TrustedServiceEndpoint) -> Result<bool, PeerDenial> {
        if endpoint != self.record.endpoint {
            return Ok(false);
        }
        self.is_current_record()
    }

    fn checkout_recipient(
        &self,
        endpoint: TrustedServiceEndpoint,
    ) -> Result<EndpointCheckoutRecipient, PeerDenial> {
        self.require_current(endpoint)?;
        Ok(self.record.recipient.clone())
    }

    fn accepted_peer_owner_key(
        &self,
        endpoint: TrustedServiceEndpoint,
    ) -> Result<nagi_peer_authenticated_connection::TrustedInteractivePeerKey, PeerDenial> {
        self.require_current(endpoint)?;
        let listener = {
            self.record
                .listener
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?
                .as_ref()
                .cloned()
                .ok_or(PeerDenial::Unavailable)?
        };
        Ok(listener.owner_key())
    }

    fn fd_delegation_is_prevented(
        &self,
        endpoint: TrustedServiceEndpoint,
    ) -> Result<bool, PeerDenial> {
        self.require_current(endpoint)?;
        Ok(self.record.fd_delegation_attested.load(Ordering::SeqCst)
            && self
                .record
                .spawn_identity
                .as_ref()
                .is_some_and(|identity| identity.process_id() == self.record.process.pid()))
    }

    fn local_socket_cookie(&self, stream: &UnixStream) -> Result<NonZeroU64, PeerDenial> {
        RegisteredEndpointOwnerCandidate::read_local_socket_cookie(stream)
    }

    fn accept_checkout(
        &self,
        endpoint: TrustedServiceEndpoint,
    ) -> Result<EndpointCheckout, PeerDenial> {
        self.accept_checkout_until(endpoint, Instant::now() + Duration::from_secs(5))
    }

    fn accept_checkout_until(
        &self,
        endpoint: TrustedServiceEndpoint,
        deadline: Instant,
    ) -> Result<EndpointCheckout, PeerDenial> {
        self.require_current(endpoint)?;
        let checkout_deadline = deadline.min(self.record.expires_at);
        if Instant::now() >= checkout_deadline {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        let listener = self
            .record
            .listener
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .as_ref()
            .cloned()
            .ok_or(PeerDenial::Unavailable)?;
        let checkout = {
            let accepted = listener
                .accept_until(checkout_deadline)
                .map_err(|error| match error {
                    nagi_peer_authenticated_connection::PeerAuthenticationError::AcceptDeadlineExpired
                    | nagi_peer_authenticated_connection::PeerAuthenticationError::OwnerExpired => {
                        PeerDenial::RequestDeadlineExpired
                    }
                    other => PeerDenial::KernelPeerAuthentication(Arc::new(other)),
                })?;
            let (stream, accepted_peer_handle) = accepted.into_stream_and_handle();
            let owned = OwnedPeerConnection::from_connected_with_current_writer_credentials(stream)
                .map_err(|error| PeerDenial::PeerConnection(Arc::new(error)))?;
            let transfer = owned
                .into_stream_transfer_until(checkout_deadline)
                .map_err(|error| PeerDenial::PeerConnection(Arc::new(error)))?;
            let mut checkout = bind_task39_transfer_to_endpoint_checkout(
                self.record.recipient.clone(),
                &self.record.recipient,
                checkout_deadline,
                transfer,
                RegisteredEndpointOwnerCandidate::read_local_socket_cookie,
            )?;
            checkout.accepted_peer_handle = Some(accepted_peer_handle);
            checkout
        };
        self.require_current(endpoint)?;
        if checkout.endpoint_epoch != self.record.endpoint.endpoint_generation
            || !checkout.recipient.is_same_recipient(&self.record.recipient)
            || checkout.expires_at != checkout_deadline
        {
            return Err(PeerDenial::StaleEndpoint);
        }
        self.claim_socket_cookie(checkout.socket_cookie)?;
        Ok(checkout)
    }
}

/// Concrete implementation of the current SVC provider contract for PID 1's
/// normal unconnected state. It deliberately publishes no Notes Storage route
/// and returns Unavailable until the spawn-owned listener, live grant/session,
/// consent and service-effect transaction are joined by one runtime owner.
/// This is not a test authority and never emits a stream.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pid1UnavailableAuthenticatedUnixStreamProvider;

impl ServiceDiscovery for Pid1UnavailableAuthenticatedUnixStreamProvider {
    fn descriptors(&self) -> Vec<ServiceDescriptor> {
        Vec::new()
    }

    fn resolve(
        &self,
        _service_id: &ServiceId,
        _version: ContractVersion,
    ) -> Result<ServiceDescriptor, IpcError> {
        Err(IpcError::new(IpcErrorCode::Unavailable))
    }
}

impl AuthenticatedUnixStreamProvider for Pid1UnavailableAuthenticatedUnixStreamProvider {
    fn try_take_authenticated_single_call_stream(
        &self,
        _service_id: &ServiceId,
        _version: ContractVersion,
        _operation_id: &OperationId,
    ) -> Result<Option<UnixStream>, IpcError> {
        Err(IpcError::new(IpcErrorCode::Unavailable))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nagi_service_contract::{ContractVersion, OperationId, ServiceId};

    fn fixture_process(pid: u32, token: u64) -> ProcessInstanceKey {
        ProcessInstanceKey::from_pid1_spawn(pid, token).expect("test-only process key")
    }

    fn route() -> TrustedRouteKey {
        TrustedRouteKey::from_canonical_route(4101, 1, 1, 0, 1).expect("test-only route")
    }

    #[test]
    fn owner_issues_new_epoch_recipient_and_revokes_on_drop() {
        let run_owner = super::super::Pid1LivePeerDirectory::<TestRecord>::new();
        let run = run_owner.begin_run().expect("test run");
        let registry = Pid1EndpointOwnerRegistry::new();
        let first = registry
            .install_test_record(
                run.clone(),
                fixture_process(4102, 1),
                route(),
                Instant::now() + Duration::from_secs(30),
            )
            .expect("first test record");
        let first_endpoint = first.endpoint();
        let first_recipient = first.recipient();
        let first_record = Arc::clone(&first.record);
        let first_process = first.process_instance();
        assert_eq!(first.process_instance().pid(), 4102);
        assert!(first.expires_at() > Instant::now());
        assert!(first.is_current(first_endpoint).expect("current record"));

        drop(first);
        assert!(!first_record.is_current());

        let second = registry
            .install_test_record(
                run,
                fixture_process(4103, 2),
                route(),
                Instant::now() + Duration::from_secs(30),
            )
            .expect("replacement test record");
        assert!(second.endpoint().endpoint_generation > first_endpoint.endpoint_generation);
        assert!(!second.recipient().is_same_recipient(&first_recipient));
        assert!(first_record.revoked.load(Ordering::SeqCst));
        assert!(second
            .is_current(second.endpoint())
            .expect("new epoch current"));
        assert_ne!(second.process_instance(), first_process);
    }

    #[test]
    fn owner_rejects_expiry_process_mismatch_and_duplicate_socket_cookie() {
        let run_owner = super::super::Pid1LivePeerDirectory::<TestRecord>::new();
        let run = run_owner.begin_run().expect("test run");
        let registry = Pid1EndpointOwnerRegistry::new();
        let process = fixture_process(4112, 7);
        let lease = registry
            .install_test_record(
                run.clone(),
                process,
                route(),
                Instant::now() + Duration::from_secs(30),
            )
            .expect("test record");
        assert!(lease.confirms_process_for_test(fixture_process(4112, 7)));
        assert!(!lease.confirms_process_for_test(fixture_process(4112, 8)));
        let cookie = NonZeroU64::new(99).expect("cookie");
        assert!(lease.claim_socket_cookie(cookie).is_ok());
        assert!(matches!(
            lease.claim_socket_cookie(cookie),
            Err(PeerDenial::StaleSocket)
        ));

        assert_eq!(
            registry
                .revoke_process_instance(process)
                .expect("pidfd owner cleanup revoke"),
            1
        );
        assert!(!lease
            .is_current(lease.endpoint())
            .expect("reaped instance stale"));

        let expiring = registry
            .install_test_record(
                run,
                fixture_process(4113, 8),
                TrustedRouteKey::from_canonical_route(4102, 1, 1, 0, 1).expect("second test route"),
                Instant::now() + Duration::from_millis(5),
            )
            .expect("expiring test lease");
        assert!(!expiring.recipient().is_same_recipient(&lease.recipient()));
        std::thread::sleep(Duration::from_millis(10));
        assert!(!expiring
            .is_current(expiring.endpoint())
            .expect("expired lease stale"));
    }

    #[test]
    fn production_endpoint_owner_never_claims_current_writer_or_alias_proof() {
        let run_owner = super::super::Pid1LivePeerDirectory::<TestRecord>::new();
        let run = run_owner.begin_run().expect("test run");
        let registry = Pid1EndpointOwnerRegistry::new();
        let lease = registry
            .install_test_record(
                run,
                fixture_process(4114, 9),
                route(),
                Instant::now() + Duration::from_secs(30),
            )
            .expect("test owner record");

        // A live endpoint epoch and recipient do not prove unique FD ownership.
        assert!(matches!(
            TrustedServiceEndpointAuthority::fd_delegation_is_prevented(&lease, lease.endpoint()),
            Ok(false)
        ));
    }

    #[test]
    fn exact_reap_owner_hook_revokes_the_bound_endpoint_lease() {
        let run_owner = super::super::Pid1LivePeerDirectory::<TestRecord>::new();
        let run = run_owner.begin_run().expect("test run");
        let registry = Pid1EndpointOwnerRegistry::new();
        let lease = registry
            .install_test_record(
                run,
                fixture_process(4120, 21),
                route(),
                Instant::now() + Duration::from_secs(30),
            )
            .expect("test record");
        let record = Arc::clone(&lease.record);
        assert!(record.is_current());

        // This exercises the revocation hook only. The fixture does not prove
        // an OS child was reaped; Task24 must call it after its exact wait and
        // Supervisor-record revoke complete.
        lease
            .revoke_after_exact_child_reap()
            .expect("endpoint revoke");
        assert!(!record.is_current());
        assert!(registry.inner.state.lock().unwrap().by_route.is_empty());
    }

    #[test]
    fn listener_revoke_contention_stays_unknown_without_drop_retry() {
        let run_owner = super::super::Pid1LivePeerDirectory::<TestRecord>::new();
        let run = run_owner.begin_run().expect("test run");
        let registry = Pid1EndpointOwnerRegistry::new();
        let lease = registry
            .install_test_record(
                run,
                fixture_process(4121, 22),
                route(),
                Instant::now() + Duration::from_secs(30),
            )
            .expect("test record");

        let listener_guard = lease.record.listener.lock().expect("test listener lock");
        assert!(matches!(
            lease.record.revoke(),
            Err(PeerDenial::Unavailable)
        ));
        drop(listener_guard);
        assert!(lease.record.revoked.load(Ordering::SeqCst));
        assert!(!lease.record.listener_closed.load(Ordering::SeqCst));
        assert!(matches!(
            lease.record.revoke(),
            Err(PeerDenial::Unavailable)
        ));
        assert!(!lease.record.listener_closed.load(Ordering::SeqCst));
    }

    #[test]
    fn unconnected_provider_has_no_discovery_or_stream() {
        let provider = Pid1UnavailableAuthenticatedUnixStreamProvider;
        let service = ServiceId::new("notes.storage").expect("test service id");
        let version = ContractVersion::new(1, 0).expect("test version");
        let operation = OperationId::new("read").expect("test operation");
        assert!(provider.descriptors().is_empty());
        assert_eq!(
            provider.resolve(&service, version).unwrap_err().code(),
            IpcErrorCode::Unavailable
        );
        assert_eq!(
            provider
                .try_take_authenticated_single_call_stream(&service, version, &operation)
                .unwrap_err()
                .code(),
            IpcErrorCode::Unavailable
        );
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct TestRecord(u32);

    impl LaunchRecordView for TestRecord {
        fn process_id(self) -> u32 {
            self.0
        }

        fn app_id(self) -> u64 {
            1
        }

        fn app_session_id(self) -> u64 {
            1
        }
    }

    impl Pid1SpawnEndpointLease {
        fn confirms_process_for_test(&self, process: ProcessInstanceKey) -> bool {
            self.record.process == process
        }
    }
}
