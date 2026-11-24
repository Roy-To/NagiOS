//! Candidate PID 1 launch/peer authority adapter for the Linux init workstream.
//!
//! This file uses task-39's Linux socket reader and a task-25 PackageService
//! inventory-snapshot adapter. Account, production session, consent, service
//! route, endpoint listener, and Model receipt inputs remain owned by their
//! existing suppliers. The adapter accepts only
//! opaque results from those owners, ties a kernel-authenticated process
//! instance to the exact Supervisor launch record, and asks the Supervisor to
//! check grants and commit while its launch lock is held.
//!
//! Integration requirements:
//! - `ProcessInstanceKey` is only a private table selector for the pidfd
//!   returned atomically with PID 1's child by `clone3(CLONE_PIDFD)`. The proof
//!   comes from retaining that pidfd and matching it with the live peer pidfd
//!   returned by `SO_PEERPIDFD`; the selector counter is not identity evidence.
//! - `TrustedManifest` must be created only from task-25's verified package and
//!   authoritative installed inventory result.
//! - `SessionLease` must be created only from the live production session and
//!   consent owners. It is a selector snapshot, not authorization by itself.
//!   Protected work must run inside C1's checked Session effect gate, then C2's
//!   readiness/owner gates, through the final effect commit. A point-in-time
//!   lease read cannot authorize that work.
//! - The A-root `LiveSessionAuthority` adapter receives C2's borrowed,
//!   spawn/endpoint-validated app admission, then composes it with C1's
//!   accepted-peer account binding and checked `InteractiveSessionContext` /
//!   `SessionLeaseRevalidationHandle`. C1 receives the lower leaf-issued
//!   accepted-peer proof, shared same-connection proof, and its selector,
//!   never a C2-root token. C2's
//!   copyable `SessionLease` is only a revision selector; C1 must source its
//!   generation from live owner state, never a raw ID or `SessionNotice`.
//!   Logout, expiry, and recovery must invalidate the owned C1 handle. No
//!   production `LiveSessionAuthority` adapter is wired yet, so the default
//!   peer-binding method remains fail-closed.
//! - `SupervisorLaunchAuthority::authorize_and_commit` must wrap the existing
//!   Supervisor exact-record/grant check and execute the callback under that
//!   same lock. The callback may only do bounded, nonblocking local work.
//! - `TrustedServiceRequirements` must come from the canonical route owner,
//!   never from a NIPC payload or caller-selected service identity.
//! - The accepted server-side AF_UNIX stream stays inside the PID-1 broker;
//!   the internal effect owner receives typed admission data, not that stream
//!   or its fd. This does not prevent a client from duplicating, inheriting,
//!   forking with, or passing its connected-side fd. Without a current-writer
//!   or non-delegation supplier, production endpoint authorization stays denied.
//! - Device `rdev`, socket cookie, pathname, and numeric PID do not identify
//!   one current writer or canonical Storage node. Same-`rdev` alternate nodes
//!   and duplicated/inherited descriptors are not ownership proof.
//!
//! The task-39 reader supplies typed `SO_PEERCRED`/`SO_COOKIE` results. This
//! adapter obtains the additional `SO_PEERPIDFD` handle itself. Task24's
//! source-connected v2 exposes a sealed `SpawnedProcessPidfd` from its clone3
//! child path, and this candidate accepts that exact token. The boot
//! normal-init callsite has not wired the token and listener through this
//! adapter; production endpoint, inventory, session, consent, and effect
//! suppliers are also absent, so ordinary `run()` stays unavailable.

#[cfg(feature = "task24-owner-co-location")]
#[path = "task24_owner_co_location.rs"]
mod task24_owner_co_location;

use nagi_dev_service_catalog_consumer::GenerationBoundCommitReceipt;
use nagi_init_spawn_contract::SpawnedProcessIdentity;
use nagi_service_contract::{CapabilityId, ContractVersion, OperationId, ServiceId};
use nagi_task37_receipt_supplier::service_effect::{
    DurableServiceEffectOwner, DurableServiceRequestBinding, DurableServiceRequestIdentity,
    DurableServiceRequestKey, OwnerValidatedServiceEffectRequest, ServiceEffectOwnerError,
    ServiceEffectOwnerId, UntrustedServiceEffectRequest,
};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::num::{NonZeroU32, NonZeroU64};
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, BorrowedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

#[cfg(test)]
pub(crate) use crate as supplier_root;

mod nipc_request_decoder;
pub use nipc_request_decoder::{DecodedNipcRequest, NipcRequestDecoder, NipcRequestRead};
mod initial_login_wire;
pub use initial_login_wire::AcceptedInitialLoginRequest;
pub mod initial_login {
    pub use super::initial_login_wire::{
        send_initial_login_request_until, AcceptedInitialLoginRequest, InitialLoginRequestMetadata,
        LoginWireError, MAX_LOGIN_ACCOUNT_NAME_BYTES, MAX_LOGIN_SECRET_BYTES,
    };
}
mod nipc_operation_mapper;
pub(crate) use nipc_operation_mapper::{CanonicalNipcOperationMapper, TrustedNipcOperationRoute};
mod c2_accepted_peer_owner_check;
use c2_accepted_peer_owner_check::C2SpawnEndpointAdmission;
mod c2_accepted_peer_proof;
pub use c2_accepted_peer_proof::C2AcceptedPeerProof;
mod child_service_bootstrap;
mod spawn_owned_listener_bridge;
pub use spawn_owned_listener_bridge::Task24EndpointLaunchLease;
pub(crate) use spawn_owned_listener_bridge::{
    InstalledSpawnEndpoint, PendingSpawnOwnedEndpoint, SpawnEndpointReapLease,
};
mod pid1_authenticated_endpoint_owner;
pub use pid1_authenticated_endpoint_owner::{
    Pid1EndpointOwnerRegistry, Pid1SpawnEndpointLease,
    Pid1UnavailableAuthenticatedUnixStreamProvider,
};
mod service_handshake_owner;
pub use service_handshake_owner::{
    Pid1CompletedServiceHandshakeOwner, ServiceHandshakeAttemptIds, ServiceHandshakeProtocol,
};
#[cfg(feature = "system-service-readiness")]
mod system_service_readiness;
#[cfg(feature = "system-service-readiness")]
pub use system_service_readiness::{Pid1SystemServiceReadyEvent, ReadySystemServiceInstance};

pub(crate) mod package_service_inventory_adapter;
pub(crate) mod pidfd_process_instance;
pub use pidfd_process_instance::ProcessInstanceError;

#[cfg(test)]
use nagi_task37_receipt_supplier::service_effect::UnavailableServiceEffectOwner;

use linux_peer_credentials_supplier::{
    AcceptedPeerError, AcceptedPeerListener, OwnedPeerStreamTransfer,
};
#[cfg(target_os = "linux")]
use linux_peer_credentials_supplier::{
    KernelPeerCredentials, OwnedPeerConnection, PeerConnectionIo,
};
use linux_peer_credentials_supplier::{PeerConnectionError, PeerCredentialError};
use nagi_c2_accepted_connection_proof_contract::C2AcceptedConnectionProof;
use nagi_peer_authenticated_connection::{
    AcceptedInteractivePeerHandle, AuthenticatedInteractivePeer,
    AuthenticatedInteractivePeerSelector, C2AcceptedPeerListenerOwner, TrustedInteractivePeerKey,
};
use nagi_peer_authenticated_connection::{KernelAuthenticatedUnixPeer, PeerAuthenticationError};

/// Source-backed consumer surface for task-2 readiness wiring.
///
/// The implementation lives in this crate's library root (`pid1_live_peer_supplier.rs`),
/// as selected by `Cargo.toml`'s `[lib] path`. These re-exports give consumers a stable,
/// named API module without opening constructors for run IDs, spawn proofs, routes,
/// endpoints, required-service sets, or readiness events.
pub mod pid1_ready {
    /// Child-side locator for the Task24-selected endpoint. The parsed argv
    /// value is only a path; C2 still authenticates the accepted kernel peer
    /// and rechecks its spawn-owned endpoint before any effect.
    pub use super::child_service_bootstrap::{ChildServiceBootstrap, EndpointBootstrapError};
    pub use super::initial_login::{
        send_initial_login_request_until, InitialLoginRequestMetadata, LoginWireError,
    };
    #[cfg(feature = "system-service-readiness")]
    pub use super::Task24EndpointLaunchLease;
    pub use super::{
        AcceptedInitialLoginRequest, BoundPeerSessionEffectGate, C2AcceptedPeerProof,
        C2AppSessionAdmission, CanonicalNipcOperationBinding, CompleteServiceRequestFrame,
        CompletedServiceHandshakeAuthority, ConsentSource, DecodedNipcRequest,
        EndpointCheckoutRecipient, LaunchRecordView, LiveSessionAuthority, NipcRequestDecoder,
        NipcRequestRead, PeerDenial, PeerProcessInstanceAuthority, PendingBoundPeerSession,
        PendingInitialSession, PendingServiceRequestFrame, Pid1AuthorityDomain,
        Pid1CompletedServiceHandshakeOwner, Pid1LifecycleReadyAdapter, Pid1LivePeerDirectory,
        Pid1PackageRequestReceipt, Pid1ReadinessAtomicBoundary, Pid1ReadyEvent, Pid1RunInstance,
        Pid1RunOwnerLease, Pid1SpawnProof, ProcessInstanceKey, ReadyServiceInstance,
        RegistrationId, RequiredServiceSet, RouteConsentRequirement, ServiceHandshakeAttemptIds,
        ServiceHandshakeProtocol, ServiceOwnerEffectTransaction, ServiceRequestAdmission,
        ServiceRequestFingerprint, ServiceRequestOperationState, ServiceRequestReceiptTransaction,
        SessionLease, StableServiceRequestId, SupervisorLaunchAuthority, TrustedInventory,
        TrustedManifest, TrustedRouteKey, TrustedServiceEndpoint, TrustedServiceEndpointAuthority,
        TrustedServiceRequirements,
    };
    #[cfg(feature = "system-service-readiness")]
    pub use super::{Pid1SystemServiceReadyEvent, ReadySystemServiceInstance};
    pub use nagi_c2_accepted_connection_proof_contract::C2AcceptedConnectionProof;
    pub use nagi_peer_authenticated_connection::{
        AuthenticatedInteractivePeer, AuthenticatedInteractivePeerSelector,
        KernelAuthenticatedUnixPeer, TrustedInteractivePeerKey,
    };
    pub use nagi_task37_receipt_supplier::service_effect::{
        DurableServiceEffectOwner, DurableServiceRequestBinding, DurableServiceRequestIdentity,
        DurableServiceRequestKey, OwnerValidatedServiceEffectRequest, ServiceEffectOwnerError,
        ServiceEffectOwnerId, UnavailableServiceEffectOwner, UntrustedServiceEffectRequest,
    };
}

const MAX_LIVE_PEERS: usize = 64;
const MAX_TRACKED_SERVICE_REQUESTS: usize = MAX_LIVE_PEERS * 256;
const MAX_SERVICE_REQUEST_FRAME_BYTES: usize = 78 + 128 + 1_048_576;
static NEXT_SERVICE_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_AUTHORIZED_CONNECTION_EPOCH: AtomicU64 = AtomicU64::new(1);

/// Stable protocol request identity, supplied by the trusted NIPC decoder.
/// It is an idempotency key only; it does not authenticate the peer or grant
/// permission to execute an operation.
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::StableServiceRequestId;
/// let forged = StableServiceRequestId([7; 16]);
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StableServiceRequestId([u8; 16]);

impl StableServiceRequestId {
    /// Called only by the trusted protocol decoder after reading RequestId
    /// from a validated request envelope. This task has no production decoder
    /// connection yet.
    pub(crate) const fn from_protocol_decoder(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ServiceRequestFrameProgress {
    wire_request_id: StableServiceRequestId,
    expected_bytes: usize,
    received_bytes: usize,
    authority_io_deadline: Option<Instant>,
}

impl ServiceRequestFrameProgress {
    #[cfg(any(target_os = "linux", test))]
    fn bind_authority_io_deadline(&mut self, deadline: Instant) -> Result<(), PeerDenial> {
        match self.authority_io_deadline {
            Some(bound) if bound != deadline => Err(PeerDenial::ConnectionRequestIdentityMismatch),
            Some(_) => Ok(()),
            None => {
                self.authority_io_deadline = Some(deadline);
                Ok(())
            }
        }
    }
}

/// A partial request frame bound to one authorized connection and wire ID.
/// On `WouldBlock`, keep this value and its offset and continue the same frame.
/// It cannot be replaced with a new request ID on the same connection.
#[derive(Debug)]
pub struct PendingServiceRequestFrame {
    progress: Arc<Mutex<Option<ServiceRequestFrameProgress>>>,
    connection_stale: Arc<AtomicBool>,
    wire_request_id: StableServiceRequestId,
}

impl PendingServiceRequestFrame {
    pub const fn wire_request_id(&self) -> StableServiceRequestId {
        self.wire_request_id
    }

    pub fn received_bytes(&self) -> Result<usize, PeerDenial> {
        if self.connection_stale.load(Ordering::SeqCst) {
            return Err(PeerDenial::StaleConnection);
        }
        let progress = self
            .progress
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        let progress = progress.as_ref().ok_or(PeerDenial::StaleConnection)?;
        if progress.wire_request_id != self.wire_request_id {
            return Err(PeerDenial::ConnectionRequestIdentityMismatch);
        }
        Ok(progress.received_bytes)
    }

    /// Record the result from one actual frame read. `Interrupted` and
    /// `WouldBlock` preserve the byte offset and request identity for the next
    /// call. EOF and other I/O errors stale this connection.
    pub fn record_read_result(&mut self, result: io::Result<usize>) -> Result<usize, PeerDenial> {
        if self.connection_stale.load(Ordering::SeqCst) {
            return Err(PeerDenial::StaleConnection);
        }
        match result {
            Ok(0) => {
                self.connection_stale.store(true, Ordering::SeqCst);
                Err(PeerDenial::StaleConnection)
            }
            Ok(count) => {
                let mut progress = self
                    .progress
                    .try_lock()
                    .map_err(|_| PeerDenial::Unavailable)?;
                let current = progress.as_mut().ok_or(PeerDenial::StaleConnection)?;
                if current.wire_request_id != self.wire_request_id
                    || count
                        > current
                            .expected_bytes
                            .saturating_sub(current.received_bytes)
                {
                    self.connection_stale.store(true, Ordering::SeqCst);
                    return Err(PeerDenial::InvalidRequestFrameLength);
                }
                current.received_bytes += count;
                Ok(current.received_bytes)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) =>
            {
                self.received_bytes()
            }
            Err(error) => {
                self.connection_stale.store(true, Ordering::SeqCst);
                Err(PeerDenial::PeerConnection(Arc::new(
                    PeerConnectionError::IoFailure {
                        kind: error.kind(),
                        raw_os_error: error.raw_os_error(),
                        message: error.to_string(),
                    },
                )))
            }
        }
    }

    pub fn finish(self) -> Result<CompleteServiceRequestFrame, PeerDenial> {
        if self.connection_stale.load(Ordering::SeqCst) {
            return Err(PeerDenial::StaleConnection);
        }
        let progress = self
            .progress
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        let current = progress.as_ref().ok_or(PeerDenial::StaleConnection)?;
        if current.wire_request_id != self.wire_request_id {
            return Err(PeerDenial::ConnectionRequestIdentityMismatch);
        }
        if current.received_bytes != current.expected_bytes {
            return Err(PeerDenial::IncompleteRequestFrame);
        }
        Ok(CompleteServiceRequestFrame {
            progress: Arc::clone(&self.progress),
            wire_request_id: self.wire_request_id,
            fingerprint: None,
            canonical_bytes: None,
        })
    }
}

/// Opaque proof that the connection's single request frame reached its
/// declared byte length. The trusted protocol decoder must still validate the
/// complete frame and payload before asking PID1 to admit it.
#[derive(Debug)]
pub struct CompleteServiceRequestFrame {
    progress: Arc<Mutex<Option<ServiceRequestFrameProgress>>>,
    wire_request_id: StableServiceRequestId,
    fingerprint: Option<ServiceRequestFingerprint>,
    canonical_bytes: Option<Arc<[u8]>>,
}

/// SHA-256 identity of the complete canonical NIPC request bytes. It binds a
/// stable wire RequestId to the exact route operation and payload presented to
/// the owner. It is an idempotency key, not an authentication proof.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ServiceRequestFingerprint([u8; 32]);

impl ServiceRequestFingerprint {
    fn from_validated_nipc_frame(frame: &[u8]) -> Self {
        Self(Sha256::digest(frame).into())
    }

    #[cfg(test)]
    pub(crate) fn for_test(frame: &[u8]) -> Self {
        Self(Sha256::digest(frame).into())
    }

    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

/// An operation/capability selection made by the canonical route owner after
/// it has interpreted the decoded NIPC destination, payload type, and body.
/// Its private fields prevent a caller from choosing a declared capability
/// directly from untrusted request bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalNipcOperationBinding {
    route: TrustedRouteKey,
    wire_request_id: StableServiceRequestId,
    fingerprint: ServiceRequestFingerprint,
    service_id: ServiceId,
    contract_version: ContractVersion,
    operation_id: OperationId,
    capability_id: CapabilityId,
    effect_owner: ServiceEffectOwnerId,
}

impl CanonicalNipcOperationBinding {
    /// Crate-local route-owner seam. Production admission remains unavailable
    /// until a real canonical NIPC destination/operation mapper calls this
    /// after schema validation.
    pub(crate) fn from_route_owner(
        route: TrustedRouteKey,
        request: &DecodedNipcRequest,
        service_id: ServiceId,
        contract_version: ContractVersion,
        operation_id: OperationId,
        capability_id: CapabilityId,
        effect_owner: ServiceEffectOwnerId,
    ) -> Result<Self, PeerDenial> {
        Ok(Self {
            route,
            wire_request_id: request.wire_request_id(),
            fingerprint: request.fingerprint(),
            service_id,
            contract_version,
            operation_id,
            capability_id,
            effect_owner,
        })
    }
}

impl CompleteServiceRequestFrame {
    pub const fn wire_request_id(&self) -> StableServiceRequestId {
        self.wire_request_id
    }
}

/// PID1's per-run view of the operation keyed by one stable wire request ID.
/// `Executing` left behind after an interrupted/aborted attempt is treated as
/// unknown and is never retried automatically.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ServiceRequestOperationState {
    NotInvoked = 0,
    Executing = 1,
    Executed = 2,
    OutcomeUnknown = 3,
    ReceiptRecorded = 4,
    Failed = 5,
}

impl ServiceRequestOperationState {
    fn from_raw(value: u8) -> Self {
        match value {
            0 => Self::NotInvoked,
            1 => Self::Executing,
            2 => Self::Executed,
            3 => Self::OutcomeUnknown,
            4 => Self::ReceiptRecorded,
            5 => Self::Failed,
            _ => Self::OutcomeUnknown,
        }
    }
}

/// Conservatively closes a request if an owner or effect panics after the
/// request entered `Executing`. Normal terminal transitions replace that state
/// before this guard drops; unwind never leaves a retryable or perpetually
/// in-progress admission behind.
struct RequestExecutionGuard {
    operation_state: Arc<AtomicU8>,
}

impl RequestExecutionGuard {
    fn new(operation_state: Arc<AtomicU8>) -> Self {
        Self { operation_state }
    }
}

impl Drop for RequestExecutionGuard {
    fn drop(&mut self) {
        if ServiceRequestOperationState::from_raw(self.operation_state.load(Ordering::SeqCst))
            == ServiceRequestOperationState::Executing
        {
            self.operation_state.store(
                ServiceRequestOperationState::OutcomeUnknown as u8,
                Ordering::SeqCst,
            );
        }
    }
}

/// Route-owner policy for whether every service effect on this canonical route
/// requires a live session-owned consent event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteConsentRequirement {
    NotRequired,
    Required,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ServiceRequestLedgerKey {
    run_generation: NonZeroU64,
    app_id: u64,
    wire_request_id: StableServiceRequestId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ServiceRequestAuthorityScope {
    registration: RegistrationId,
    process: ProcessInstanceKey,
    task24_identity: SpawnedProcessIdentity,
    app_session_id: u64,
    inventory_generation: NonZeroU64,
    session_generation: NonZeroU64,
    route: TrustedRouteKey,
    endpoint: TrustedServiceEndpoint,
    endpoint_epoch: NonZeroU64,
    endpoint_recipient: EndpointCheckoutRecipient,
    service_id: ServiceId,
    contract_version: ContractVersion,
    operation_id: OperationId,
    capability_id: CapabilityId,
    effect_owner: ServiceEffectOwnerId,
    consent_requirement: RouteConsentRequirement,
    fingerprint: ServiceRequestFingerprint,
}

struct ServiceRequestLedgerEntry {
    request_id: NonZeroU64,
    scope: ServiceRequestAuthorityScope,
    state: Arc<AtomicU8>,
}

#[derive(Default)]
struct ServiceRequestLedger {
    entries: HashMap<ServiceRequestLedgerKey, ServiceRequestLedgerEntry>,
}

impl ServiceRequestLedger {
    fn admit(
        &mut self,
        key: ServiceRequestLedgerKey,
        scope: ServiceRequestAuthorityScope,
        run: &Pid1RunInstance,
    ) -> Result<(NonZeroU64, Arc<AtomicU8>), PeerDenial> {
        // Admission may race run teardown after its first live check. Teardown
        // marks the run inactive before taking this same ledger lock to clear
        // it, so an old run can neither repopulate the cleared ledger nor
        // attach a request to a new run.
        if !run.is_active() || key.run_generation != run.generation {
            return Err(PeerDenial::StaleRun);
        }
        if let Some(existing) = self.entries.get(&key) {
            if existing.scope != scope {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            return Ok((existing.request_id, Arc::clone(&existing.state)));
        }
        if self.entries.len() >= MAX_TRACKED_SERVICE_REQUESTS {
            return Err(PeerDenial::RequestDedupCapacity);
        }
        let request_id = next_service_request_id()?;
        let operation_state = Arc::new(AtomicU8::new(
            ServiceRequestOperationState::NotInvoked as u8,
        ));
        self.entries.insert(
            key,
            ServiceRequestLedgerEntry {
                request_id,
                scope,
                state: Arc::clone(&operation_state),
            },
        );
        Ok((request_id, operation_state))
    }

    fn clear_run(&mut self, run_generation: NonZeroU64) {
        for (key, entry) in &self.entries {
            if key.run_generation == run_generation
                && ServiceRequestOperationState::from_raw(entry.state.load(Ordering::SeqCst))
                    == ServiceRequestOperationState::Executing
            {
                // If teardown ever observes an in-flight state, preserve the
                // conservative outcome in admission clones before unlinking
                // the RAM entry. It must not become retryable NotInvoked.
                entry.state.store(
                    ServiceRequestOperationState::OutcomeUnknown as u8,
                    Ordering::SeqCst,
                );
            }
        }
        self.entries
            .retain(|key, _| key.run_generation != run_generation);
    }
}

/// Private table selector for an init-owned kernel pidfd, not proof by itself.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProcessInstanceKey {
    pid: NonZeroU32,
    instance: NonZeroU64,
}

impl ProcessInstanceKey {
    /// Internal token allocation happens only after a kernel pidfd is stored;
    /// the token indexes that handle and is never identity evidence itself.
    const fn from_pid1_spawn(pid: u32, instance: u64) -> Option<Self> {
        match (NonZeroU32::new(pid), NonZeroU64::new(instance)) {
            (Some(pid), Some(instance)) if pid.get() > 1 => Some(Self { pid, instance }),
            _ => None,
        }
    }

    pub const fn pid(self) -> u32 {
        self.pid.get()
    }
}

#[derive(Debug)]
struct DirectoryOwnerIdentity;

/// Opaque run identity issued by one live PID 1 registry instance. It cannot
/// be reconstructed from an ID in a request and expires with that registry's
/// process lifetime.
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::Pid1RunInstance;
/// let forged = Pid1RunInstance { owner: todo!(), generation: todo!() };
/// ```
#[derive(Clone)]
pub struct Pid1RunInstance {
    owner: Arc<DirectoryOwnerIdentity>,
    generation: NonZeroU64,
    /// Per-run entropy used only to distinguish volatile authority epochs in
    /// durable request bindings. It is never supplied by a peer or derived
    /// from an Arc address, PID, or selector counter.
    authority_nonce: [u8; 32],
    active: Arc<AtomicBool>,
    authority_domain: Pid1AuthorityDomain,
}

impl std::fmt::Debug for Pid1RunInstance {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Pid1RunInstance")
            .field("generation", &self.generation)
            .field("active", &self.active.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl Pid1RunInstance {
    pub const fn generation(&self) -> NonZeroU64 {
        self.generation
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }

    pub fn is_same_run(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.authority_nonce == other.authority_nonce
            && Arc::ptr_eq(&self.owner, &other.owner)
            && self
                .authority_domain
                .is_same_domain(&other.authority_domain)
    }

    /// The production C2 readiness/owner boundary for this exact run. Pass
    /// this same value to registration, initial-session attachment, endpoint
    /// retirement, effect commits, and Ready-event mapping.
    pub fn readiness_boundary(&self) -> &Pid1AuthorityDomain {
        &self.authority_domain
    }
}

fn new_run_authority_nonce() -> Result<[u8; 32], PeerDenial> {
    // Nagi runs on Linux; fail closed if the kernel entropy source is absent.
    // This nonce is an epoch discriminator, not a process identity proof.
    let mut nonce = [0_u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut source| source.read_exact(&mut nonce))
        .map_err(|_| PeerDenial::Unavailable)?;
    Ok(nonce)
}

/// Opaque lifecycle owner lease for one PID 1 run. The source lifecycle owner
/// must retain this value from run creation through cancellation/termination
/// and call `Pid1LivePeerDirectory::end_run_owner_lease()` under the shared
/// authority gate before reporting the run ended. Ready events carry only the
/// corresponding run identity; the lease is supplied again when mapping into
/// the lifecycle observer.
pub struct Pid1RunOwnerLease {
    run: Pid1RunInstance,
}

impl Pid1RunOwnerLease {
    pub fn run(&self) -> &Pid1RunInstance {
        &self.run
    }

    pub const fn generation(&self) -> NonZeroU64 {
        self.run.generation()
    }

    pub fn is_active(&self) -> bool {
        self.run.is_active()
    }

    /// Return the one shared owner gate issued with this run. Its nonblocking
    /// lock and run invalidation are part of the same authority domain.
    pub fn readiness_boundary(&self) -> &Pid1AuthorityDomain {
        self.run.readiness_boundary()
    }
}

impl PartialEq for Pid1RunInstance {
    fn eq(&self, other: &Self) -> bool {
        self.is_same_run(other)
    }
}

impl Eq for Pid1RunInstance {}

/// A service identity produced by the registry after current live authority
/// and a completed protocol handshake have both been revalidated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReadyServiceInstance {
    registration: RegistrationId,
    process: ProcessInstanceKey,
    route: TrustedRouteKey,
    endpoint: TrustedServiceEndpoint,
}

impl ReadyServiceInstance {
    pub const fn registration(self) -> RegistrationId {
        self.registration
    }

    pub const fn process(self) -> ProcessInstanceKey {
        self.process
    }

    pub const fn route(self) -> TrustedRouteKey {
        self.route
    }

    pub const fn endpoint(self) -> TrustedServiceEndpoint {
        self.endpoint
    }
}

/// Readiness evidence that can only be emitted by `Pid1LivePeerDirectory`.
/// It binds a single run instance to every mandatory service's registration,
/// route, endpoint generation, and kernel-backed process instance.
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::Pid1ReadyEvent;
/// let forged = Pid1ReadyEvent { run: todo!(), services: vec![] };
/// ```
pub struct Pid1ReadyEvent<'a> {
    run: Pid1RunInstance,
    services: Vec<ReadyServiceInstance>,
    with_current: Box<
        dyn Fn(&mut dyn FnMut() -> Result<(), PeerDenial>) -> Result<(), PeerDenial>
            + Send
            + Sync
            + 'a,
    >,
}

/// Accepted request context. This records which live PID1 request was parsed;
/// it does not authorize a service effect. Effect rights require a separate
/// service-owner transaction using this exact context.
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::ServiceRequestAdmission;
/// let forged = ServiceRequestAdmission { run: todo!(), request_id: todo!(), wire_request_id: todo!(), fingerprint: todo!(), operation_state: todo!(), consent_event: None, consent_requirement: todo!(), registration: todo!(), process: todo!(), app_id: 1, app_session_id: 2, inventory_generation: todo!(), session_generation: todo!(), route: todo!(), endpoint: todo!(), service_id: todo!(), contract_version: todo!(), operation_id: todo!(), capability_id: todo!() };
/// ```
#[derive(Clone, Debug)]
pub struct ServiceRequestAdmission {
    run: Pid1RunInstance,
    request_id: NonZeroU64,
    wire_request_id: StableServiceRequestId,
    fingerprint: ServiceRequestFingerprint,
    operation_state: Arc<AtomicU8>,
    owner_identity: ServiceEffectOwnerId,
    canonical_request_bytes: Option<Arc<[u8]>>,
    consent_event: Option<SessionConsentGrantEvent>,
    consent_requirement: RouteConsentRequirement,
    registration: RegistrationId,
    process: ProcessInstanceKey,
    task24_identity: SpawnedProcessIdentity,
    app_id: u64,
    app_session_id: u64,
    inventory_generation: NonZeroU64,
    session_generation: NonZeroU64,
    route: TrustedRouteKey,
    endpoint: TrustedServiceEndpoint,
    endpoint_epoch: NonZeroU64,
    endpoint_recipient: EndpointCheckoutRecipient,
    service_id: ServiceId,
    contract_version: ContractVersion,
    operation_id: OperationId,
    capability_id: CapabilityId,
}

impl ServiceRequestAdmission {
    pub fn run(&self) -> &Pid1RunInstance {
        &self.run
    }

    pub const fn request_id(&self) -> NonZeroU64 {
        self.request_id
    }

    /// Stable protocol identity used to deduplicate a re-admission or replay
    /// of this request. `request_id()` remains a PID1-local owner token.
    pub const fn stable_request_id(&self) -> StableServiceRequestId {
        self.wire_request_id
    }

    pub const fn fingerprint(&self) -> ServiceRequestFingerprint {
        self.fingerprint
    }

    pub fn operation_state(&self) -> ServiceRequestOperationState {
        ServiceRequestOperationState::from_raw(self.operation_state.load(Ordering::SeqCst))
    }

    fn confirm_durable_owner_identity(
        &self,
        owner: ServiceEffectOwnerId,
    ) -> Result<(), PeerDenial> {
        if self.owner_identity == owner {
            Ok(())
        } else {
            Err(PeerDenial::RequestIdentityConflict)
        }
    }

    pub const fn registration(&self) -> RegistrationId {
        self.registration
    }

    pub const fn process(&self) -> ProcessInstanceKey {
        self.process
    }

    pub const fn app_id(&self) -> u64 {
        self.app_id
    }

    pub const fn app_session_id(&self) -> u64 {
        self.app_session_id
    }

    pub const fn inventory_generation(&self) -> NonZeroU64 {
        self.inventory_generation
    }

    pub const fn session_generation(&self) -> NonZeroU64 {
        self.session_generation
    }

    pub const fn route(&self) -> TrustedRouteKey {
        self.route
    }

    pub const fn endpoint(&self) -> TrustedServiceEndpoint {
        self.endpoint
    }

    pub fn service_id(&self) -> &ServiceId {
        &self.service_id
    }

    pub const fn contract_version(&self) -> ContractVersion {
        self.contract_version
    }

    pub fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    pub fn operation(&self) -> &[u8] {
        self.operation_id.as_str().as_bytes()
    }

    pub fn capability(&self) -> &[u8] {
        self.capability_id.as_str().as_bytes()
    }

    /// Validate this admitted request with task-37's owner boundary. The raw
    /// canonical NIPC frame is retained by the trusted decoder; synthetic
    /// admissions and fixtures without those bytes are unavailable. The owner
    /// receives raw request and route claims, not a trusted boolean or locally
    /// minted owner proof, and must return its opaque validated request only
    /// after resolving current authority itself.
    pub fn validate_with_task37_owner<O: DurableServiceEffectOwner>(
        &self,
        owner: &O,
        identity_claim: DurableServiceRequestIdentity,
    ) -> Result<ServiceOwnerRequestToken, PeerDenial> {
        self.confirm_durable_owner_identity(identity_claim.key().owner())?;
        if identity_claim != durable_request_identity_claim(self.owner_identity, self) {
            return Err(PeerDenial::RequestIdentityConflict);
        }
        let request_bytes = self
            .canonical_request_bytes
            .as_deref()
            .ok_or(PeerDenial::Unavailable)?;
        let route_claim = canonical_route_claim_bytes(self);
        let untrusted =
            UntrustedServiceEffectRequest::new(identity_claim.key(), request_bytes, &route_claim);
        let validated = owner
            .validate_request(&untrusted)
            .map_err(map_task37_owner_error)?;
        ServiceOwnerRequestToken::from_task37_owner(validated, identity_claim, self)
    }
}

impl PartialEq for ServiceRequestAdmission {
    fn eq(&self, other: &Self) -> bool {
        self.run.is_same_run(&other.run)
            && self.request_id == other.request_id
            && self.wire_request_id == other.wire_request_id
            && self.fingerprint == other.fingerprint
            && self.owner_identity == other.owner_identity
            && self.canonical_request_bytes == other.canonical_request_bytes
            && self.registration == other.registration
            && self.consent_event == other.consent_event
            && self.consent_requirement == other.consent_requirement
            && self.process == other.process
            && self.task24_identity.same_instance(&other.task24_identity)
            && self.app_id == other.app_id
            && self.app_session_id == other.app_session_id
            && self.inventory_generation == other.inventory_generation
            && self.session_generation == other.session_generation
            && self.route == other.route
            && self.endpoint == other.endpoint
            && self.endpoint_epoch == other.endpoint_epoch
            && self.endpoint_recipient == other.endpoint_recipient
            && self.service_id == other.service_id
            && self.contract_version == other.contract_version
            && self.operation_id == other.operation_id
            && self.capability_id == other.capability_id
    }
}

/// Opaque task-local view of a Task-25 package-generation receipt. This is a
/// RAM lease tied to one authorized connection epoch, PID 1 run, authority
/// state, and package owner. It is not a service effect, durable result, or
/// transferable authorization. Callers must pass it back through
/// `AuthorizedPeer::with_current_package_request_receipt` before using it.
pub struct Pid1PackageRequestReceipt {
    receipt: Option<GenerationBoundCommitReceipt>,
    admission: ServiceRequestAdmission,
    run: Pid1RunInstance,
    connection_epoch: NonZeroU64,
    connection_stale: Arc<AtomicBool>,
    authority_domain: Pid1AuthorityDomain,
    active: bool,
}

impl Pid1PackageRequestReceipt {
    fn from_recorded<R: LaunchRecordView>(
        receipt: GenerationBoundCommitReceipt,
        admission: &ServiceRequestAdmission,
        peer: &AuthorizedPeer<'_, R>,
    ) -> Self {
        Self {
            receipt: Some(receipt),
            admission: admission.clone(),
            run: peer.run.clone(),
            connection_epoch: peer.connection_epoch,
            connection_stale: Arc::clone(&peer.connection_stale),
            authority_domain: peer.authority_domain.clone(),
            active: true,
        }
    }

    fn invalidate(&mut self) {
        self.active = false;
        self.receipt = None;
    }

    pub const fn is_invalidated(&self) -> bool {
        !self.active
    }
}

impl std::fmt::Debug for Pid1PackageRequestReceipt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Pid1PackageRequestReceipt")
            .field("request_id", &self.admission.stable_request_id())
            .field("run_active", &self.run.is_active())
            .field("connection_epoch", &self.connection_epoch)
            .field("has_receipt", &self.receipt.is_some())
            .field("active", &self.active)
            .finish()
    }
}

/// A session-owner consent event identity plus its revocation revision.
/// Fields are private; the only issuer is crate-internal session-owner code.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ConsentEventRevision {
    source_owner_id: NonZeroU64,
    event_id: NonZeroU64,
    revision: NonZeroU64,
}

impl ConsentEventRevision {
    pub const fn source_owner_id(self) -> NonZeroU64 {
        self.source_owner_id
    }

    pub const fn event_id(self) -> NonZeroU64 {
        self.event_id
    }

    pub const fn revision(self) -> NonZeroU64 {
        self.revision
    }

    pub(crate) fn from_live_session_owner(
        source_owner_id: u64,
        event_id: u64,
        revision: u64,
    ) -> Option<Self> {
        Some(Self {
            source_owner_id: NonZeroU64::new(source_owner_id)?,
            event_id: NonZeroU64::new(event_id)?,
            revision: NonZeroU64::new(revision)?,
        })
    }
}

/// An opaque, per-request consent grant event from a live session owner.
/// It binds the event revision to the full service-request authority tuple.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionConsentGrantEvent {
    event: ConsentEventRevision,
    run: Pid1RunInstance,
    request_id: NonZeroU64,
    wire_request_id: StableServiceRequestId,
    fingerprint: ServiceRequestFingerprint,
    registration: RegistrationId,
    process: ProcessInstanceKey,
    app_id: u64,
    app_session_id: u64,
    inventory_generation: NonZeroU64,
    session_generation: NonZeroU64,
    route: TrustedRouteKey,
    endpoint: TrustedServiceEndpoint,
    service_id: ServiceId,
    contract_version: ContractVersion,
    operation_id: OperationId,
    capability_id: CapabilityId,
    consent_requirement: RouteConsentRequirement,
}

impl SessionConsentGrantEvent {
    pub const fn event_revision(&self) -> ConsentEventRevision {
        self.event
    }

    pub(crate) fn from_live_session_owner(
        admission: &ServiceRequestAdmission,
        event: ConsentEventRevision,
    ) -> Self {
        Self {
            event,
            run: admission.run.clone(),
            request_id: admission.request_id,
            wire_request_id: admission.wire_request_id,
            fingerprint: admission.fingerprint,
            registration: admission.registration,
            process: admission.process,
            app_id: admission.app_id,
            app_session_id: admission.app_session_id,
            inventory_generation: admission.inventory_generation,
            session_generation: admission.session_generation,
            route: admission.route,
            endpoint: admission.endpoint,
            service_id: admission.service_id.clone(),
            contract_version: admission.contract_version,
            operation_id: admission.operation_id.clone(),
            capability_id: admission.capability_id.clone(),
            consent_requirement: admission.consent_requirement,
        }
    }

    fn matches_admission(&self, admission: &ServiceRequestAdmission) -> bool {
        self.run.is_same_run(&admission.run)
            && self.request_id == admission.request_id
            && self.wire_request_id == admission.wire_request_id
            && self.fingerprint == admission.fingerprint
            && self.registration == admission.registration
            && self.process == admission.process
            && self.app_id == admission.app_id
            && self.app_session_id == admission.app_session_id
            && self.inventory_generation == admission.inventory_generation
            && self.session_generation == admission.session_generation
            && self.route == admission.route
            && self.endpoint == admission.endpoint
            && self.service_id == admission.service_id
            && self.contract_version == admission.contract_version
            && self.operation_id == admission.operation_id
            && self.capability_id == admission.capability_id
            && self.consent_requirement == admission.consent_requirement
    }
}

impl Eq for ServiceRequestAdmission {}

/// Owner validation returned by the task-41 adapter. Production construction
/// requires task-37's opaque `OwnerValidatedServiceEffectRequest`; task-41
/// never mints its durable identity or stable binding. The fixture path is
/// compiled only into test and explicit Linux-runtime-fixture builds and is
/// not part of the default library API.
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::ServiceOwnerRequestToken;
/// let forged = ServiceOwnerRequestToken {
///     validated: None,
///     identity_claim: todo!(),
///     authority_fingerprint: [0; 32],
/// };
/// ```
#[derive(Clone, Copy, Eq, PartialEq)]
pub struct ServiceOwnerRequestToken {
    validated: Option<OwnerValidatedServiceEffectRequest>,
    identity_claim: DurableServiceRequestIdentity,
    authority_fingerprint: [u8; 32],
    #[cfg(any(test, feature = "linux-runtime-fixture"))]
    fixture: bool,
}

impl std::fmt::Debug for ServiceOwnerRequestToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServiceOwnerRequestToken")
            .field("identity_claim", &self.identity_claim)
            .field("validated_by_task37", &self.validated.is_some())
            .finish_non_exhaustive()
    }
}

impl ServiceOwnerRequestToken {
    /// Internal adapter boundary. Call only with the opaque value returned by
    /// task-37's `DurableServiceEffectOwner::validate_request`; the task-41
    /// admission must still match the stable key/bindings. The durable owner
    /// must revalidate its ephemeral authority inside commit.
    #[allow(dead_code)]
    fn from_task37_owner(
        validated: OwnerValidatedServiceEffectRequest,
        identity_claim: DurableServiceRequestIdentity,
        admission: &ServiceRequestAdmission,
    ) -> Result<Self, PeerDenial> {
        let expected_key = DurableServiceRequestKey::new(
            admission.owner_identity,
            admission.app_id,
            admission.wire_request_id.as_bytes(),
        );
        if admission
            .confirm_durable_owner_identity(identity_claim.key().owner())
            .is_err()
            || identity_claim.key() != expected_key
            || validated.key() != identity_claim.key()
            || identity_claim.binding().request_fingerprint() != admission.fingerprint.as_bytes()
            || identity_claim.binding().route_fingerprint() != request_route_fingerprint(admission)
            || identity_claim.binding().authority_generation_fingerprint()
                != request_authority_fingerprint(admission)
        {
            return Err(PeerDenial::RequestIdentityConflict);
        }
        Ok(Self {
            validated: Some(validated),
            identity_claim,
            authority_fingerprint: request_authority_fingerprint(admission),
            #[cfg(any(test, feature = "linux-runtime-fixture"))]
            fixture: false,
        })
    }

    #[cfg(any(test, feature = "linux-runtime-fixture"))]
    fn fixture(owner: ServiceEffectOwnerId, admission: &ServiceRequestAdmission) -> Self {
        let identity_claim = durable_request_identity_claim(owner, admission);
        Self {
            validated: None,
            identity_claim,
            authority_fingerprint: request_authority_fingerprint(admission),
            fixture: true,
        }
    }

    fn matches_admission(
        &self,
        owner: ServiceEffectOwnerId,
        admission: &ServiceRequestAdmission,
    ) -> bool {
        if self.identity_claim.key().owner() != owner
            || self.identity_claim != durable_request_identity_claim(owner, admission)
            || self.authority_fingerprint != request_authority_fingerprint(admission)
        {
            return false;
        }
        if let Some(validated) = self.validated {
            validated.key() == self.identity_claim.key()
        } else {
            #[cfg(any(test, feature = "linux-runtime-fixture"))]
            {
                self.fixture
            }
            #[cfg(not(any(test, feature = "linux-runtime-fixture")))]
            {
                false
            }
        }
    }
}

fn update_fingerprint_u64(hasher: &mut Sha256, value: u64) {
    hasher.update(value.to_be_bytes());
}

fn request_route_fingerprint(admission: &ServiceRequestAdmission) -> [u8; 32] {
    Sha256::digest(canonical_route_claim_bytes(admission)).into()
}

fn canonical_route_claim_bytes(admission: &ServiceRequestAdmission) -> Vec<u8> {
    let mut bytes = b"nagi.pid1.request-route.v1".to_vec();
    bytes.extend_from_slice(&admission.owner_identity.as_bytes());
    append_route_u64(&mut bytes, admission.route.service_id.get());
    append_route_u64(&mut bytes, admission.route.operation_id.get());
    bytes.extend_from_slice(&admission.route.version_major.to_be_bytes());
    bytes.extend_from_slice(&admission.route.version_minor.to_be_bytes());
    append_route_u64(&mut bytes, admission.route.generation.get());
    append_route_u64(&mut bytes, admission.endpoint.endpoint_generation.get());
    append_route_u64(&mut bytes, admission.endpoint.endpoint_socket_id.get());
    append_route_field(&mut bytes, admission.service_id.as_str().as_bytes());
    bytes.extend_from_slice(&admission.contract_version.major().to_be_bytes());
    bytes.extend_from_slice(&admission.contract_version.minor().to_be_bytes());
    append_route_field(&mut bytes, admission.operation_id.as_str().as_bytes());
    append_route_field(&mut bytes, admission.capability_id.as_str().as_bytes());
    bytes
}

fn append_route_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn append_route_field(bytes: &mut Vec<u8>, value: &[u8]) {
    append_route_u64(bytes, value.len() as u64);
    bytes.extend_from_slice(value);
}

fn map_task37_owner_error(error: ServiceEffectOwnerError) -> PeerDenial {
    match error {
        ServiceEffectOwnerError::OwnerMismatch
        | ServiceEffectOwnerError::RequestIdentityConflict => PeerDenial::RequestIdentityConflict,
        ServiceEffectOwnerError::Stale => PeerDenial::StaleRegistration,
        ServiceEffectOwnerError::OutcomeUnknown => {
            PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(PeerDenial::RequestOutcomeUnknown))
        }
        ServiceEffectOwnerError::RecordMissing
        | ServiceEffectOwnerError::Unavailable
        | ServiceEffectOwnerError::Busy
        | ServiceEffectOwnerError::RetryNotAuthorized
        | ServiceEffectOwnerError::RecoveryRequired
        | ServiceEffectOwnerError::Corrupt => PeerDenial::Unavailable,
    }
}

fn request_authority_fingerprint(admission: &ServiceRequestAdmission) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"nagi.pid1.request-authority.v1");
    hasher.update(admission.run.authority_nonce);
    update_fingerprint_u64(&mut hasher, admission.run.generation.get());
    update_fingerprint_u64(&mut hasher, admission.registration.0.get());
    update_fingerprint_u64(&mut hasher, u64::from(admission.process.pid.get()));
    update_fingerprint_u64(&mut hasher, admission.process.instance.get());
    update_fingerprint_u64(&mut hasher, admission.app_session_id);
    update_fingerprint_u64(&mut hasher, admission.inventory_generation.get());
    update_fingerprint_u64(&mut hasher, admission.session_generation.get());
    hasher.update([match admission.consent_requirement {
        RouteConsentRequirement::NotRequired => 0,
        RouteConsentRequirement::Required => 1,
    }]);
    match admission
        .consent_event
        .as_ref()
        .map(|event| event.event_revision())
    {
        Some(revision) => {
            hasher.update([1]);
            update_fingerprint_u64(&mut hasher, revision.source_owner_id.get());
            update_fingerprint_u64(&mut hasher, revision.event_id.get());
            update_fingerprint_u64(&mut hasher, revision.revision.get());
        }
        None => hasher.update([0]),
    }
    hasher.finalize().into()
}

/// Construct the task-37 public identity claim used to bind owner lookup to
/// this accepted request. Task-37 documents this as untrusted data: only the
/// service owner's `validate_request` can turn the raw payload and route claim
/// into an opaque validated request.
fn durable_request_identity_claim(
    owner: ServiceEffectOwnerId,
    admission: &ServiceRequestAdmission,
) -> DurableServiceRequestIdentity {
    let key = DurableServiceRequestKey::new(
        owner,
        admission.app_id,
        admission.wire_request_id.as_bytes(),
    );
    let binding = DurableServiceRequestBinding::new(
        admission.fingerprint.as_bytes(),
        request_route_fingerprint(admission),
        request_authority_fingerprint(admission),
    );
    DurableServiceRequestIdentity::new(key, binding)
}

fn next_service_request_id() -> Result<NonZeroU64, PeerDenial> {
    let current = NEXT_SERVICE_REQUEST_ID
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
            value.checked_add(1)
        })
        .map_err(|_| PeerDenial::GenerationExhausted)?;
    NonZeroU64::new(current).ok_or(PeerDenial::GenerationExhausted)
}

fn next_authorized_connection_epoch() -> Result<NonZeroU64, PeerDenial> {
    let current = NEXT_AUTHORIZED_CONNECTION_EPOCH
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
            value.checked_add(1)
        })
        .map_err(|_| PeerDenial::GenerationExhausted)?;
    NonZeroU64::new(current).ok_or(PeerDenial::GenerationExhausted)
}

fn ensure_request_not_terminal(state: ServiceRequestOperationState) -> Result<(), PeerDenial> {
    match state {
        ServiceRequestOperationState::NotInvoked => Ok(()),
        ServiceRequestOperationState::Executed | ServiceRequestOperationState::ReceiptRecorded => {
            Err(PeerDenial::DuplicateRequestCommit)
        }
        ServiceRequestOperationState::Failed => Err(PeerDenial::ServiceEffectFailed(Arc::new(
            PeerDenial::RequestOperationFailed,
        ))),
        ServiceRequestOperationState::Executing => Err(PeerDenial::RequestInProgress),
        ServiceRequestOperationState::OutcomeUnknown => Err(
            PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(PeerDenial::RequestOutcomeUnknown)),
        ),
    }
}

fn request_state_denial(raw_state: u8) -> PeerDenial {
    match ServiceRequestOperationState::from_raw(raw_state) {
        ServiceRequestOperationState::NotInvoked => PeerDenial::Unavailable,
        ServiceRequestOperationState::Executed | ServiceRequestOperationState::ReceiptRecorded => {
            PeerDenial::DuplicateRequestCommit
        }
        ServiceRequestOperationState::Failed => {
            PeerDenial::ServiceEffectFailed(Arc::new(PeerDenial::RequestOperationFailed))
        }
        ServiceRequestOperationState::Executing => PeerDenial::RequestInProgress,
        ServiceRequestOperationState::OutcomeUnknown => {
            PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(PeerDenial::RequestOutcomeUnknown))
        }
    }
}

fn classify_service_effect_error(
    error: PeerDenial,
    operation_state: &AtomicU8,
    callback_succeeded_this_attempt: bool,
) -> PeerDenial {
    match ServiceRequestOperationState::from_raw(operation_state.load(Ordering::SeqCst)) {
        ServiceRequestOperationState::NotInvoked => match error {
            other @ PeerDenial::ServiceEffectNotInvoked(_) => other,
            PeerDenial::ServiceEffectOutcomeUnknown(cause) => {
                operation_state.store(
                    ServiceRequestOperationState::OutcomeUnknown as u8,
                    Ordering::SeqCst,
                );
                PeerDenial::ServiceEffectOutcomeUnknown(cause)
            }
            PeerDenial::DuplicateRequestCommit | PeerDenial::RequestOutcomeUnknown => {
                operation_state.store(
                    ServiceRequestOperationState::OutcomeUnknown as u8,
                    Ordering::SeqCst,
                );
                PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(PeerDenial::RequestOutcomeUnknown))
            }
            PeerDenial::RequestInProgress => {
                operation_state.store(
                    ServiceRequestOperationState::Executing as u8,
                    Ordering::SeqCst,
                );
                PeerDenial::RequestInProgress
            }
            PeerDenial::ServiceEffectFailed(cause) => {
                operation_state.store(ServiceRequestOperationState::Failed as u8, Ordering::SeqCst);
                PeerDenial::ServiceEffectFailed(cause)
            }
            cause => PeerDenial::ServiceEffectNotInvoked(Arc::new(cause)),
        },
        ServiceRequestOperationState::Executing
            if matches!(&error, PeerDenial::RequestInProgress) =>
        {
            PeerDenial::RequestInProgress
        }
        ServiceRequestOperationState::Executing | ServiceRequestOperationState::OutcomeUnknown => {
            match error {
                PeerDenial::ServiceEffectFailed(cause) => {
                    operation_state
                        .store(ServiceRequestOperationState::Failed as u8, Ordering::SeqCst);
                    PeerDenial::ServiceEffectFailed(cause)
                }
                PeerDenial::ServiceEffectOutcomeUnknown(cause) => {
                    operation_state.store(
                        ServiceRequestOperationState::OutcomeUnknown as u8,
                        Ordering::SeqCst,
                    );
                    PeerDenial::ServiceEffectOutcomeUnknown(cause)
                }
                cause => {
                    operation_state.store(
                        ServiceRequestOperationState::OutcomeUnknown as u8,
                        Ordering::SeqCst,
                    );
                    PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(cause))
                }
            }
        }
        ServiceRequestOperationState::Executed => {
            if callback_succeeded_this_attempt {
                operation_state.store(
                    ServiceRequestOperationState::OutcomeUnknown as u8,
                    Ordering::SeqCst,
                );
                match error {
                    PeerDenial::ServiceEffectOutcomeUnknown(cause) => {
                        PeerDenial::ServiceEffectOutcomeUnknown(cause)
                    }
                    cause => PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(cause)),
                }
            } else {
                PeerDenial::DuplicateRequestCommit
            }
        }
        ServiceRequestOperationState::ReceiptRecorded => PeerDenial::DuplicateRequestCommit,
        ServiceRequestOperationState::Failed => match error {
            PeerDenial::ServiceEffectOutcomeUnknown(cause) => {
                operation_state.store(
                    ServiceRequestOperationState::OutcomeUnknown as u8,
                    Ordering::SeqCst,
                );
                PeerDenial::ServiceEffectOutcomeUnknown(cause)
            }
            PeerDenial::ServiceEffectFailed(cause) => PeerDenial::ServiceEffectFailed(cause),
            _ => PeerDenial::ServiceEffectFailed(Arc::new(PeerDenial::RequestOperationFailed)),
        },
    }
}

mod service_effect_commit_seal {
    pub trait Sealed {}
}

/// Minimum service-owner transaction/CAS contract for effects. The PID1 gate
/// is held while this method runs. The owner must compare the complete
/// admission generation (run, registration/process instance, installed
/// inventory generation, app session, route consent policy and live event,
/// route, endpoint, capability, and stable wire request ID) against current
/// owner state, then deduplicate by `(app_id, stable_request_id)` rather than
/// by PID1's process-local `request_id()`. The owner compares the exact
/// `ServiceRequestFingerprint` as well as operation/route/capability. The owner
/// must atomically persist request state with the bounded, nonblocking effect,
/// and preserve `NotInvoked`, `Executing`, `Executed`, `Failed`, or
/// `OutcomeUnknown` across owner restart. If its
/// durable deduplication state is absent or unavailable, it must return
/// `Unavailable` before invoking the effect. An in-memory PID1 ledger only
/// deduplicates within this PID1 run and is not crash-surviving exactly-once
/// evidence. A cloned admission or a fresh admission carrying the same wire
/// ID must never invoke the effect twice, including under concurrent calls.
/// The package owner must validate the installed generation in that
/// transaction or deny. Revocation paths must take locks in the same order:
/// PID1 gate, then service-owner transaction. A precheck followed by
/// an unprotected effect does not implement this trait's contract. No
/// production service owner currently implements it, so production effects
/// remain unavailable. Task-25's fixed receipt operation does not implement
/// this effect contract and must never be reported as a service effect.
pub trait ServiceOwnerEffectTransaction: service_effect_commit_seal::Sealed + Send + Sync {
    /// Resolve the durable owner namespace only after confirming that this
    /// owner is the current canonical owner for the admission's exact route,
    /// endpoint, service, and operation. The namespace comes from the same
    /// authoritative result store used for commit and lookup; it must not be
    /// synthesized from request data or a process-local counter. Owners
    /// without a route-bound durable identity return `Unavailable`.
    fn durable_owner_id_for(
        &self,
        _admission: &ServiceRequestAdmission,
    ) -> Result<ServiceEffectOwnerId, PeerDenial> {
        Err(PeerDenial::Unavailable)
    }

    /// Validate the accepted task-41 request through the actual service owner
    /// and return its task-37 opaque token. Production adapters must create the
    /// token only from `DurableServiceEffectOwner::validate_request`; fixture
    /// tokens exist only under `cfg(test)`. Missing raw request/route/session
    /// inputs remain `Unavailable`.
    fn validate_request_with_identity(
        &self,
        _admission: &ServiceRequestAdmission,
        _owner: ServiceEffectOwnerId,
        _identity: &DurableServiceRequestIdentity,
    ) -> Result<ServiceOwnerRequestToken, PeerDenial> {
        Err(PeerDenial::Unavailable)
    }

    /// Recheck route registration, request ID, full authority generation,
    /// package generation, and revocation state under the same service-owner
    /// serialization boundary used by `DurableServiceEffectOwner`.
    fn admission_is_current(&self, admission: &ServiceRequestAdmission)
        -> Result<bool, PeerDenial>;

    /// Read the durable task-37 owner outcome for this exact shared identity.
    /// Production implementations must delegate to `DurableServiceEffectOwner`
    /// and preserve `Unavailable`/unknown on missing or stale owner state.
    fn resolve_request_state_with_identity(
        &self,
        _admission: &ServiceRequestAdmission,
        _identity: &ServiceOwnerRequestToken,
    ) -> Result<ServiceRequestOperationState, PeerDenial> {
        Err(PeerDenial::Unavailable)
    }

    /// Perform the bounded service-owner transaction using this same
    /// task-37 identity. Production implementations must call the durable
    /// owner transaction that commits the effect and result record together.
    fn commit_if_current_with_identity<T>(
        &self,
        _admission: &ServiceRequestAdmission,
        _identity: &ServiceOwnerRequestToken,
        _commit: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        Err(PeerDenial::ServiceEffectNotInvoked(Arc::new(
            PeerDenial::Unavailable,
        )))
    }
}

/// Task-25's fixed package-generation receipt path. It records one local
/// task receipt; it does not perform a service effect or establish a
/// production owner. Production consumers still fail closed until a real
/// owner endpoint implements the service-effect transaction above.
pub trait ServiceRequestReceiptTransaction:
    service_effect_commit_seal::Sealed + Send + Sync
{
    /// Return the canonical receipt-owner namespace that was already bound by
    /// the trusted route mapper at admission. Implementations may not choose
    /// or latch an owner on the first receipt operation.
    fn receipt_owner_id_for(
        &self,
        _admission: &ServiceRequestAdmission,
    ) -> Result<ServiceEffectOwnerId, PeerDenial> {
        Err(PeerDenial::Unavailable)
    }

    /// Query the same authoritative receipt owner after a receipt call returned
    /// an ambiguous error or was interrupted. The result must be bound to the
    /// complete admission, including the stable wire ID, exact fingerprint,
    /// route, operation, capability, and authority generations. A reused ID
    /// with different content returns `RequestIdentityConflict`. If the owner
    /// cannot prove a result, return `Unavailable`; PID 1 will not retry.
    fn record_receipt_if_current(
        &self,
        admission: &ServiceRequestAdmission,
    ) -> Result<GenerationBoundCommitReceipt, PeerDenial>;

    /// Read the owner's current result for exactly this request. Owners with no
    /// lookup table, or no durable/cross-restart guarantee for their table,
    /// remain unavailable for unresolved requests.
    fn resolve_receipt_state(
        &self,
        _admission: &ServiceRequestAdmission,
    ) -> Result<ServiceRequestOperationState, PeerDenial> {
        Err(PeerDenial::Unavailable)
    }

    /// Return the exact task25 receipt observed by the same owner-side lookup.
    /// Implementations without a request-bound result table stay unavailable;
    /// a state-only answer is not enough to reconstruct a receipt.
    fn resolve_receipt_record(
        &self,
        _admission: &ServiceRequestAdmission,
    ) -> Result<Option<GenerationBoundCommitReceipt>, PeerDenial> {
        Err(PeerDenial::Unavailable)
    }
}

impl std::fmt::Debug for Pid1ReadyEvent<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Pid1ReadyEvent")
            .field("run", &self.run)
            .field("services", &self.services)
            .finish_non_exhaustive()
    }
}

impl Pid1ReadyEvent<'_> {
    pub fn run(&self) -> &Pid1RunInstance {
        &self.run
    }

    pub fn services(&self) -> &[ReadyServiceInstance] {
        &self.services
    }

    /// Point-in-time recheck for diagnostics only. The consumer must use
    /// `with_current()` to map readiness while the owner snapshot and same-
    /// process inventory lease remain held.
    pub fn validate_current(&self) -> Result<(), PeerDenial> {
        self.with_current(|_| ())
    }

    /// Run lifecycle mapping inside the owner-serialized readiness boundary.
    /// `validate_current()` by itself is not an atomic authorization result:
    /// state can change after that method returns.
    pub fn with_current<T>(&self, map: impl FnOnce(&Self) -> T) -> Result<T, PeerDenial> {
        let mut map = Some(map);
        let mut output = None;
        let mut consume = || {
            let map = map.take().ok_or(PeerDenial::Unavailable)?;
            output = Some(map(self));
            Ok(())
        };
        (self.with_current)(&mut consume)?;
        output.ok_or(PeerDenial::Unavailable)
    }
}

/// Minimal typed bridge for a lifecycle observer. The observer must provide
/// the owner lease held by the active run; this adapter does not create a
/// Ready event or a lease. It maps only while the event, lease, complete
/// service snapshot, and current owner authorities remain inside the same
/// readiness gate.
#[derive(Clone, Copy, Debug, Default)]
pub struct Pid1LifecycleReadyAdapter;

impl Pid1LifecycleReadyAdapter {
    pub fn map_current<T>(
        &self,
        owner_lease: Option<&Pid1RunOwnerLease>,
        event: &Pid1ReadyEvent<'_>,
        map: impl FnOnce(&Pid1ReadyEvent<'_>) -> T,
    ) -> Result<T, PeerDenial> {
        let owner_lease = owner_lease.ok_or(PeerDenial::Unavailable)?;
        if !owner_lease.is_active() || !owner_lease.run.is_same_run(event.run()) {
            return Err(PeerDenial::StaleRun);
        }
        event.with_current(|current| {
            if !owner_lease.is_active() || !owner_lease.run.is_same_run(current.run()) {
                return Err(PeerDenial::StaleRun);
            }
            Ok(map(current))
        })?
    }
}

/// Canonical route identity supplied only by the trusted service-route owner.
/// Its fields are private so request payloads cannot select a route identity.
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::TrustedRouteKey;
/// let route = TrustedRouteKey::from_canonical_route(1, 2, 1, 0, 3);
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TrustedRouteKey {
    service_id: NonZeroU64,
    operation_id: NonZeroU64,
    version_major: u16,
    version_minor: u16,
    generation: NonZeroU64,
}

impl TrustedRouteKey {
    /// Crate-private hook for the canonical service-route owner only.
    pub(crate) fn from_canonical_route(
        service_id: u64,
        operation_id: u64,
        version_major: u16,
        version_minor: u16,
        generation: u64,
    ) -> Option<Self> {
        Some(Self {
            service_id: NonZeroU64::new(service_id)?,
            operation_id: NonZeroU64::new(operation_id)?,
            version_major,
            version_minor,
            generation: NonZeroU64::new(generation)?,
        })
    }
}

/// Mandatory routes from the trusted boot service plan. Fields are private so
/// a caller cannot lower the readiness requirement set from a request.
pub struct RequiredServiceSet {
    routes: Vec<TrustedRouteKey>,
}

impl RequiredServiceSet {
    /// The canonical boot service-plan owner supplies the complete mandatory
    /// set. An empty or duplicate set cannot be used to manufacture Ready.
    pub(crate) fn from_canonical_service_plan(
        routes: impl IntoIterator<Item = TrustedRouteKey>,
    ) -> Result<Self, PeerDenial> {
        let routes = routes.into_iter().collect::<Vec<_>>();
        if routes.is_empty() {
            return Err(PeerDenial::EmptyRequiredServiceSet);
        }
        let mut seen = std::collections::HashSet::with_capacity(routes.len());
        if routes.iter().any(|route| !seen.insert(*route)) {
            return Err(PeerDenial::DuplicateRequiredService);
        }
        Ok(Self { routes })
    }
}

/// Exact registered endpoint incarnation for a canonical route.
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::TrustedServiceEndpoint;
/// let endpoint = TrustedServiceEndpoint::from_service_registry(todo!(), 4, 5);
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TrustedServiceEndpoint {
    route: TrustedRouteKey,
    endpoint_generation: NonZeroU64,
    endpoint_socket_id: NonZeroU64,
}

impl TrustedServiceEndpoint {
    /// Crate-private hook for the live endpoint registry only.
    pub(crate) fn from_service_registry(
        route: TrustedRouteKey,
        endpoint_generation: u64,
        endpoint_socket_id: u64,
    ) -> Option<Self> {
        Some(Self {
            route,
            endpoint_generation: NonZeroU64::new(endpoint_generation)?,
            endpoint_socket_id: NonZeroU64::new(endpoint_socket_id)?,
        })
    }
}

#[derive(Debug)]
struct EndpointCheckoutRecipientOwner;

/// Opaque identity of the registered endpoint owner that is allowed to accept
/// the listener and mint its checkout. A matching socket cookie alone does not
/// prove that a connection came from this recipient or that no FD alias exists.
#[derive(Clone, Debug)]
pub struct EndpointCheckoutRecipient {
    endpoint: TrustedServiceEndpoint,
    owner: Arc<EndpointCheckoutRecipientOwner>,
}

impl EndpointCheckoutRecipient {
    /// Called only while the trusted service endpoint registry installs its
    /// listener owner. Replacing that owner creates a distinct recipient even
    /// if a fixture accidentally repeats the numeric endpoint fields.
    fn from_registered_owner(endpoint: TrustedServiceEndpoint) -> Self {
        Self {
            endpoint,
            owner: Arc::new(EndpointCheckoutRecipientOwner),
        }
    }

    fn is_same_recipient(&self, other: &Self) -> bool {
        self.endpoint == other.endpoint && Arc::ptr_eq(&self.owner, &other.owner)
    }
}

impl PartialEq for EndpointCheckoutRecipient {
    fn eq(&self, other: &Self) -> bool {
        self.is_same_recipient(other)
    }
}

impl Eq for EndpointCheckoutRecipient {}

/// One accepted stream, bound to a registered recipient, endpoint generation,
/// and local socket cookie observed by the trusted endpoint owner. This proves
/// stream continuity inside this adapter; it does not prove that no duplicated
/// descriptor alias exists or that an alias can be revoked. The checkout and
/// server-side stream stay inside the PID-1 broker; its internal effect owner
/// receives typed admission values. The client still controls its connected-side
/// fd and may duplicate, inherit, fork with, or pass that alias. Grant-backed
/// Storage operations must use the broker's capability/grant path rather than
/// treating a device `rdev`, pathname, or raw fd as authorization.
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::EndpointCheckout;
/// let checkout = EndpointCheckout { endpoint: todo!(), socket_cookie: todo!(), stream: todo!() };
/// ```
/// An external consumer cannot replace the leaf handle with one minted from
/// its own listener:
///
/// ```compile_fail
/// use nagi_peer_authenticated_connection::AcceptedInteractivePeerHandle;
/// use nagi_pid1_live_peer_supplier::EndpointCheckout;
/// fn substitute(mut checkout: EndpointCheckout, handle: AcceptedInteractivePeerHandle) {
///     checkout.accepted_peer_handle = Some(handle);
/// }
/// ```
/// A checkout is consumed by authorization and cannot be replayed:
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::EndpointCheckout;
/// fn reuse(checkout: EndpointCheckout) {
///     let _first_use = checkout;
///     let _second_use = checkout;
/// }
/// ```
/// A raw descriptor cannot be converted into a checkout:
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::{EndpointCheckout, TrustedServiceEndpoint};
/// use std::os::fd::RawFd;
/// fn forge(endpoint: TrustedServiceEndpoint, fd: RawFd) {
///     let _ = EndpointCheckout::from_raw_fd(endpoint, fd);
/// }
/// ```
/// A task-39 transfer is consumed only once:
///
/// ```compile_fail
/// use linux_peer_credentials_supplier::OwnedPeerStreamTransfer;
/// fn take_twice(transfer: OwnedPeerStreamTransfer) {
///     let _first = transfer.consume_with_stream(|_stream, _deadline| Ok(()));
///     let _second = transfer.consume_with_stream(|_stream, _deadline| Ok(()));
/// }
/// ```
#[derive(Debug)]
pub struct EndpointCheckout {
    endpoint: TrustedServiceEndpoint,
    recipient: EndpointCheckoutRecipient,
    endpoint_epoch: NonZeroU64,
    expires_at: Instant,
    socket_cookie: NonZeroU64,
    stream: UnixStream,
    accepted_peer_handle: Option<AcceptedInteractivePeerHandle>,
}

impl EndpointCheckout {
    /// Used only by the sealed endpoint owner after accepting from the
    /// listener bound to this recipient and reading this stream's local
    /// SO_COOKIE.
    fn from_registered_accept(
        recipient: EndpointCheckoutRecipient,
        expires_at: Instant,
        socket_cookie: NonZeroU64,
        stream: UnixStream,
    ) -> Self {
        Self {
            endpoint: recipient.endpoint,
            endpoint_epoch: recipient.endpoint.endpoint_generation,
            recipient,
            expires_at,
            socket_cookie,
            stream,
            accepted_peer_handle: None,
        }
    }
}

mod endpoint_authority_seal {
    pub trait Sealed {}
}

/// Narrow co-location adapter for C3's opaque, borrowed live-child attestation.
/// Only the C3 attestation adapter may implement this crate-private contract;
/// Task24 activation accepts no boolean or caller-provided validator.
pub(crate) mod c3_fd_attestation_seal {
    pub(crate) trait Sealed {}
}

pub(crate) trait C3FdDelegationAttestation:
    c3_fd_attestation_seal::Sealed + Send + Sync
{
    fn matches_process(&self, process: &SpawnedProcessIdentity) -> bool;
}

/// Implemented inside init only by the live endpoint owner. `accept_checkout`
/// must resolve the endpoint token to its registered listener and accept from
/// that listener itself; callers cannot supply a stream separately. The owner
/// consumes the exact move-only task-39 transfer produced by that registered
/// listener and binds it to the checkout once. The accepted stream used by the task-39
/// connection must be the same stream later used for this checkout's cookie,
/// peer credentials, and PIDFD process-instance check. A separate accept,
/// detached `KernelPeerCredentials` snapshot, arbitrary `UnixStream`/`RawFd`,
/// or caller-created proof is not a handoff. Missing or expired transfers
/// return `Unavailable` or `RequestDeadlineExpired` and never reaccept.
///
/// The cookie method must read SO_COOKIE from the accepted stream, and
/// `is_current` must revalidate the registered route and endpoint generation
/// without blocking.
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::{TrustedServiceEndpoint, TrustedServiceEndpointAuthority};
/// use std::os::unix::net::UnixStream;
/// fn inject_stream(
///     owner: &impl TrustedServiceEndpointAuthority,
///     endpoint: TrustedServiceEndpoint,
///     stream: UnixStream,
/// ) {
///     let _ = owner.accept_checkout(endpoint, stream);
/// }
/// ```
pub trait TrustedServiceEndpointAuthority: endpoint_authority_seal::Sealed + Send + Sync {
    fn is_current(&self, endpoint: TrustedServiceEndpoint) -> Result<bool, PeerDenial>;

    /// Return the opaque recipient identity of the current listener owner for
    /// this exact endpoint. A replacement listener owner must issue a different
    /// value; it cannot reuse a checkout from the prior recipient.
    fn checkout_recipient(
        &self,
        endpoint: TrustedServiceEndpoint,
    ) -> Result<EndpointCheckoutRecipient, PeerDenial>;

    /// Return the opaque key held by this exact endpoint's accepted-listener
    /// owner. The C2 live bind path compares the leaf proof key with this
    /// registry-owned key before C1 sees the proof. Implementations without a
    /// leaf-owned accepted-listener registration must fail closed.
    fn accepted_peer_owner_key(
        &self,
        _endpoint: TrustedServiceEndpoint,
    ) -> Result<TrustedInteractivePeerKey, PeerDenial> {
        Err(PeerDenial::Unavailable)
    }

    /// Return true only when the service endpoint's enforced process/protocol
    /// boundary prevents connected-FD delegation or supplies current-writer
    /// authentication. A saved SO_PEERCRED/SO_PEERPIDFD snapshot is insufficient.
    /// Implementations must answer from bounded, nonblocking owner state.
    fn fd_delegation_is_prevented(
        &self,
        endpoint: TrustedServiceEndpoint,
    ) -> Result<bool, PeerDenial>;

    fn local_socket_cookie(&self, stream: &UnixStream) -> Result<NonZeroU64, PeerDenial>;

    /// Accept one stream from the listener currently registered for this exact
    /// endpoint. The endpoint owner performs the listener lookup and accept so
    /// callers cannot pair a live token with an unrelated stream. A task-39
    /// implementation consumes the exact accepted `OwnedPeerStreamTransfer`
    /// into this checkout; it may not accept a second stream or convert raw FD
    /// or snapshot input.
    /// This adapter binds a registered recipient but cannot establish sole-FD
    /// ownership: the endpoint owner's surrounding transport boundary must prevent
    /// duplicate aliases or authenticate the current writer. Aliases that escaped
    /// before transfer cannot be revoked by the task-39 wrapper. Without that
    /// independent guarantee, production endpoint authorization remains unavailable.
    fn accept_checkout(
        &self,
        endpoint: TrustedServiceEndpoint,
    ) -> Result<EndpointCheckout, PeerDenial>;

    /// Accept from the same registered listener, honoring this absolute
    /// deadline. Implementations must use bounded/nonblocking accept and must
    /// return `RequestDeadlineExpired` when acceptance or checkout construction
    /// crosses the deadline. The registered recipient consumes the task-39
    /// transfer exactly once; route/endpoint/listener or recipient mismatch
    /// and attempted reuse are denied.
    fn accept_checkout_until(
        &self,
        endpoint: TrustedServiceEndpoint,
        deadline: Instant,
    ) -> Result<EndpointCheckout, PeerDenial>;
}

/// Listener ownership record used by a task-41 endpoint owner that accepts
/// through task-39. This value must be created only when the trusted endpoint
/// registry binds this exact listener to `endpoint`; it does not accept a
/// caller-supplied stream or descriptor.
struct Task39RegisteredEndpointListener {
    recipient: EndpointCheckoutRecipient,
    listener: Mutex<AcceptedPeerListener>,
}

#[derive(Clone, Debug)]
struct EndpointOwnerRegistrationRecord {
    run: Pid1RunInstance,
    endpoint: TrustedServiceEndpoint,
    recipient: EndpointCheckoutRecipient,
    expires_at: Instant,
}

#[derive(Default)]
struct EndpointOwnerRegistrySlot {
    current: Mutex<Option<EndpointOwnerRegistrationRecord>>,
}

/// Task41-owned candidate endpoint owner. It binds a listener handed off by
/// the trusted registry to a recipient, numeric endpoint generation, and
/// bounded lease. This type has no writer/delegation supplier: authorization
/// deliberately remains unavailable even when listener acceptance succeeds.
/// The registry installation callsite is not present in ordinary init.
struct RegisteredEndpointOwnerCandidate {
    registry_slot: Arc<EndpointOwnerRegistrySlot>,
    run: Pid1RunInstance,
    recipient: EndpointCheckoutRecipient,
    listener: Task39RegisteredEndpointListener,
}

impl endpoint_authority_seal::Sealed for RegisteredEndpointOwnerCandidate {}

impl RegisteredEndpointOwnerCandidate {
    fn from_service_registry(
        registry_slot: Arc<EndpointOwnerRegistrySlot>,
        run: &Pid1RunInstance,
        endpoint: TrustedServiceEndpoint,
        listener: UnixListener,
        expires_at: Instant,
    ) -> Result<Self, PeerDenial> {
        if !run.is_active() {
            return Err(PeerDenial::StaleRun);
        }
        if Instant::now() >= expires_at {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        let recipient = EndpointCheckoutRecipient::from_registered_owner(endpoint);
        let accepted = AcceptedPeerListener::new(listener).map_err(map_task39_accept_error)?;
        {
            let mut current = registry_slot
                .current
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            if let Some(previous) = current.as_ref() {
                let same_run = previous.run.is_same_run(run);
                if (previous.run.is_active() && !same_run)
                    || (same_run
                        && (previous.endpoint.route != endpoint.route
                            || endpoint.endpoint_generation
                                <= previous.endpoint.endpoint_generation))
                {
                    return Err(PeerDenial::StaleEndpoint);
                }
            }
            *current = Some(EndpointOwnerRegistrationRecord {
                run: run.clone(),
                endpoint,
                recipient: recipient.clone(),
                expires_at,
            });
        }
        Ok(Self {
            registry_slot,
            run: run.clone(),
            recipient: recipient.clone(),
            listener: Task39RegisteredEndpointListener::from_service_registry(recipient, accepted),
        })
    }

    fn current_state(
        &self,
        endpoint: TrustedServiceEndpoint,
    ) -> Result<EndpointOwnerRegistrationRecord, PeerDenial> {
        let state = self
            .registry_slot
            .current
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .clone()
            .ok_or(PeerDenial::StaleEndpoint)?;
        if !self.run.is_active()
            || !state.run.is_same_run(&self.run)
            || endpoint != state.endpoint
            || endpoint != self.recipient.endpoint
            || state.recipient != self.recipient
        {
            return Err(PeerDenial::StaleEndpoint);
        }
        if Instant::now() >= state.expires_at {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        Ok(state)
    }

    fn revoke_for_registry(&self) -> Result<(), PeerDenial> {
        let mut current = self
            .registry_slot
            .current
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        if current
            .as_ref()
            .is_some_and(|record| record.recipient == self.recipient)
        {
            *current = None;
        }
        Ok(())
    }

    fn read_local_socket_cookie(stream: &UnixStream) -> Result<NonZeroU64, PeerDenial> {
        #[cfg(not(target_os = "linux"))]
        {
            let _ = stream;
            Err(PeerDenial::Unavailable)
        }
        #[cfg(target_os = "linux")]
        {
            let mut cookie = 0u64;
            let mut length = std::mem::size_of::<u64>() as libc::socklen_t;
            // SAFETY: the output buffer is a live u64 and the descriptor is
            // borrowed from the exact accepted UnixStream.
            let result = unsafe {
                libc::getsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_COOKIE,
                    (&mut cookie as *mut u64).cast(),
                    &mut length,
                )
            };
            if result != 0 || length as usize != std::mem::size_of::<u64>() {
                return Err(PeerDenial::Unavailable);
            }
            NonZeroU64::new(cookie).ok_or(PeerDenial::Unavailable)
        }
    }
}

impl TrustedServiceEndpointAuthority for RegisteredEndpointOwnerCandidate {
    fn is_current(&self, endpoint: TrustedServiceEndpoint) -> Result<bool, PeerDenial> {
        match self.current_state(endpoint) {
            Ok(_) => Ok(true),
            Err(PeerDenial::StaleEndpoint | PeerDenial::RequestDeadlineExpired) => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn checkout_recipient(
        &self,
        endpoint: TrustedServiceEndpoint,
    ) -> Result<EndpointCheckoutRecipient, PeerDenial> {
        self.current_state(endpoint)?;
        Ok(self.recipient.clone())
    }

    fn fd_delegation_is_prevented(
        &self,
        endpoint: TrustedServiceEndpoint,
    ) -> Result<bool, PeerDenial> {
        self.current_state(endpoint)?;
        // No production endpoint protocol/current-writer supplier is wired.
        Ok(false)
    }

    fn local_socket_cookie(&self, stream: &UnixStream) -> Result<NonZeroU64, PeerDenial> {
        Self::read_local_socket_cookie(stream)
    }

    fn accept_checkout(
        &self,
        endpoint: TrustedServiceEndpoint,
    ) -> Result<EndpointCheckout, PeerDenial> {
        self.accept_checkout_until(endpoint, Instant::now() + std::time::Duration::from_secs(5))
    }

    fn accept_checkout_until(
        &self,
        endpoint: TrustedServiceEndpoint,
        deadline: Instant,
    ) -> Result<EndpointCheckout, PeerDenial> {
        let state = self.current_state(endpoint)?;
        let checkout_deadline = deadline.min(state.expires_at);
        if Instant::now() >= checkout_deadline {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        let current_recipient = self.checkout_recipient(endpoint)?;
        let checkout = self.listener.accept_checkout_until(
            endpoint,
            checkout_deadline,
            Self::read_local_socket_cookie,
        )?;
        if Instant::now() >= checkout_deadline {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        let after = self.current_state(endpoint)?;
        if after.endpoint.endpoint_generation != state.endpoint.endpoint_generation
            || !checkout.recipient.is_same_recipient(&current_recipient)
            || !checkout.recipient.is_same_recipient(&self.recipient)
            || checkout.endpoint_epoch != state.endpoint.endpoint_generation
            || checkout.expires_at != checkout_deadline
        {
            return Err(PeerDenial::StaleEndpoint);
        }
        Ok(checkout)
    }
}

/// Private adapter seam so host tests can exercise endpoint/socket binding
/// without constructing a transport proof. Production has exactly one
/// implementation: task-39's one-shot, deadline-bound transfer.
trait Task39OwnedStreamTransfer: Sized {
    fn deadline(&self) -> Instant;
    fn consume_with_stream(
        self,
        receive: impl FnOnce(UnixStream, Instant) -> Result<(), PeerConnectionError>,
    ) -> Result<(), PeerDenial>;
}

impl Task39OwnedStreamTransfer for OwnedPeerStreamTransfer {
    fn deadline(&self) -> Instant {
        OwnedPeerStreamTransfer::deadline(self)
    }

    fn consume_with_stream(
        self,
        receive: impl FnOnce(UnixStream, Instant) -> Result<(), PeerConnectionError>,
    ) -> Result<(), PeerDenial> {
        OwnedPeerStreamTransfer::consume_with_stream(self, receive)
            .map_err(|error| PeerDenial::PeerConnection(Arc::new(error)))
    }
}

fn bind_task39_transfer_to_endpoint_checkout<T: Task39OwnedStreamTransfer>(
    recipient: EndpointCheckoutRecipient,
    registered_recipient: &EndpointCheckoutRecipient,
    deadline: Instant,
    transfer: T,
    local_socket_cookie: impl FnOnce(&UnixStream) -> Result<NonZeroU64, PeerDenial>,
) -> Result<EndpointCheckout, PeerDenial> {
    if !recipient.is_same_recipient(registered_recipient) {
        return Err(PeerDenial::StaleEndpoint);
    }
    if transfer.deadline() != deadline || Instant::now() >= deadline {
        return Err(PeerDenial::RequestDeadlineExpired);
    }
    let mut checkout = None;
    transfer.consume_with_stream(|stream, transfer_deadline| {
        if transfer_deadline != deadline || Instant::now() >= deadline {
            drop(stream);
            checkout = Some(Err(PeerDenial::RequestDeadlineExpired));
            return Ok(());
        }
        let cookie = match local_socket_cookie(&stream) {
            Ok(cookie) => cookie,
            Err(error) => {
                drop(stream);
                checkout = Some(Err(error));
                return Ok(());
            }
        };
        if Instant::now() >= deadline {
            drop(stream);
            checkout = Some(Err(PeerDenial::RequestDeadlineExpired));
            return Ok(());
        }
        checkout = Some(Ok(EndpointCheckout::from_registered_accept(
            recipient, deadline, cookie, stream,
        )));
        Ok(())
    })?;
    checkout.unwrap_or(Err(PeerDenial::Unavailable))
}

impl Task39RegisteredEndpointListener {
    fn from_service_registry(
        recipient: EndpointCheckoutRecipient,
        listener: AcceptedPeerListener,
    ) -> Self {
        Self {
            recipient,
            listener: Mutex::new(listener),
        }
    }

    /// Accept exactly once from this registered task-39 listener, consume its
    /// AcceptedPeerConnection into the one-shot stream transfer, then move the
    /// same stream into task-41's private checkout. The caller still has to
    /// revalidate endpoint currency and all PID1 authority after acceptance.
    fn accept_checkout_until(
        &self,
        endpoint: TrustedServiceEndpoint,
        deadline: Instant,
        local_socket_cookie: impl FnOnce(&UnixStream) -> Result<NonZeroU64, PeerDenial>,
    ) -> Result<EndpointCheckout, PeerDenial> {
        if endpoint != self.recipient.endpoint {
            return Err(PeerDenial::StaleEndpoint);
        }
        if Instant::now() >= deadline {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        let mut listener = self
            .listener
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        let accepted = listener
            .accept_until(deadline)
            .map_err(map_task39_accept_error)?;
        if Instant::now() >= deadline {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        let transfer = accepted
            .into_stream_transfer_until(deadline)
            .map_err(map_task39_accept_error)?;
        bind_task39_transfer_to_endpoint_checkout(
            self.recipient.clone(),
            &self.recipient,
            deadline,
            transfer,
            local_socket_cookie,
        )
    }
}

fn map_task39_accept_error(error: AcceptedPeerError) -> PeerDenial {
    match error {
        AcceptedPeerError::Unavailable => PeerDenial::Unavailable,
        AcceptedPeerError::DeadlineExceeded => PeerDenial::RequestDeadlineExpired,
        AcceptedPeerError::Peer(source) | AcceptedPeerError::Dispatch(source) => {
            PeerDenial::PeerConnection(Arc::new(source))
        }
        AcceptedPeerError::Listener(source)
        | AcceptedPeerError::FrameIo(source)
        | AcceptedPeerError::InterruptedRetryLimit { source, .. } => {
            PeerDenial::PeerConnection(Arc::new(PeerConnectionError::IoFailure {
                kind: source.kind(),
                raw_os_error: source.raw_os_error(),
                message: source.to_string(),
            }))
        }
        AcceptedPeerError::InvalidFrame | AcceptedPeerError::NotRequest => PeerDenial::Unavailable,
    }
}

mod handshake_authority_seal {
    pub trait Sealed {}
}

/// Implemented only by the trusted protocol handshake owner. Its answer must
/// bind the completed handshake to the current run, registration, process
/// incarnation, canonical route, and endpoint generation. A service spawn or
/// successful socket read is not a completed handshake.
pub trait CompletedServiceHandshakeAuthority:
    handshake_authority_seal::Sealed + Send + Sync
{
    /// This check must be bounded and nonblocking, and must not reacquire the
    /// Supervisor registry while called from its commit callback.
    fn confirms_completed_handshake(
        &self,
        run: &Pid1RunInstance,
        registration: RegistrationId,
        process: ProcessInstanceKey,
        route: TrustedRouteKey,
        endpoint: TrustedServiceEndpoint,
    ) -> Result<bool, PeerDenial>;
}

#[derive(Clone, Debug)]
pub struct KernelPeerObservation {
    pid: i32,
    uid: u32,
    gid: u32,
    same_local_socket: bool,
    // Host orchestration fixtures deliberately have no kernel proof. The
    // production reader always supplies one and the production authorization
    // path rejects its absence.
    authenticated_peer: Option<Arc<KernelAuthenticatedUnixPeer>>,
}

mod peer_credential_source_seal {
    pub trait Sealed {}
}

/// Trusted reader boundary. Only the task-39 Linux reader is used at runtime;
/// host fixtures implement this sealed trait inside this crate for negatives.
pub trait PeerCredentialSource: peer_credential_source_seal::Sealed + Send + Sync {
    fn read(&self, stream: &UnixStream) -> Result<KernelPeerObservation, PeerDenial>;
}

pub struct Task39PeerCredentialReader;

impl peer_credential_source_seal::Sealed for Task39PeerCredentialReader {}

impl PeerCredentialSource for Task39PeerCredentialReader {
    fn read(&self, stream: &UnixStream) -> Result<KernelPeerObservation, PeerDenial> {
        read_peer_observation(stream)
    }
}

fn read_peer_observation(stream: &UnixStream) -> Result<KernelPeerObservation, PeerDenial> {
    let peer =
        KernelAuthenticatedUnixPeer::authenticate(stream).map_err(map_peer_authentication_error)?;
    peer.revalidate(stream)
        .map_err(map_peer_authentication_error)?;
    observation_from_authenticated_peer(peer)
}

fn observation_from_authenticated_peer(
    peer: KernelAuthenticatedUnixPeer,
) -> Result<KernelPeerObservation, PeerDenial> {
    let pid = peer.pid() as i32;
    if pid <= 1 {
        return Err(PeerDenial::InvalidProcess);
    }
    Ok(KernelPeerObservation {
        pid,
        uid: peer.uid(),
        gid: peer.gid(),
        same_local_socket: true,
        authenticated_peer: Some(Arc::new(peer)),
    })
}

fn map_peer_authentication_error(error: PeerAuthenticationError) -> PeerDenial {
    PeerDenial::KernelPeerAuthentication(Arc::new(error))
}

fn map_reader_error(error: PeerCredentialError) -> PeerDenial {
    PeerDenial::KernelReader(Arc::new(error))
}

/// Opaque, single-use result from the PID 1 spawn path after it has obtained a
/// process instance key and the exact Supervisor launch record. Its constructor
/// and fields are private; the task-41 overlay has no caller-facing issuer.
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::{Pid1SpawnProof, ProcessInstanceKey};
/// let process = ProcessInstanceKey { pid: todo!(), instance: todo!() };
/// let forged = Pid1SpawnProof { process, record: () };
/// ```
pub struct Pid1SpawnProof<R: Copy> {
    process: ProcessInstanceKey,
    record: R,
    task24_identity: Option<SpawnedProcessIdentity>,
}

impl<R: LaunchRecordView> Pid1SpawnProof<R> {
    const fn from_confirmed_spawn(process: ProcessInstanceKey, record: R) -> Self {
        Self {
            process,
            record,
            task24_identity: None,
        }
    }

    pub(crate) fn attach_task24_identity(
        self,
        exact_spawn: pidfd_process_instance::ExactTask24SpawnIdentity,
    ) -> Result<Self, PeerDenial> {
        self.attach_identity(exact_spawn.into_identity())
    }

    #[cfg(any(test, feature = "linux-runtime-fixture"))]
    fn attach_task24_test_identity(
        self,
        identity: SpawnedProcessIdentity,
    ) -> Result<Self, PeerDenial> {
        self.attach_identity(identity)
    }

    fn attach_identity(mut self, identity: SpawnedProcessIdentity) -> Result<Self, PeerDenial> {
        if self.task24_identity.is_some()
            || identity.process_id() != self.process.pid()
            || self.record.process_id() != self.process.pid()
        {
            return Err(PeerDenial::ForeignSpawnOwner);
        }
        self.task24_identity = Some(identity);
        Ok(self)
    }

    pub(crate) fn task24_identity(&self) -> Option<&SpawnedProcessIdentity> {
        self.task24_identity.as_ref()
    }

    pub(crate) fn exact_launch_record(&self) -> R {
        self.record
    }

    pub(crate) const fn process_key(&self) -> ProcessInstanceKey {
        self.process
    }
}

/// Minimal view implemented for the existing `libnagi::launch::LaunchRecord`.
pub trait LaunchRecordView: Copy + Eq + Send + Sync + 'static {
    fn process_id(self) -> u32;
    fn app_id(self) -> u64;
    fn app_session_id(self) -> u64;
}

/// Direct adapter for the existing Supervisor record type. The Task24
/// `SpawnedProcessPidfd` handoff and this exact record travel through the same
/// pre-exec callback; no PID-only record is synthesized here.
impl LaunchRecordView for libnagi::launch::LaunchRecord {
    fn process_id(self) -> u32 {
        self.process_id
    }

    fn app_id(self) -> u64 {
        self.app_id.0
    }

    fn app_session_id(self) -> u64 {
        self.app_session_id.0
    }
}

/// Trusted installed-manifest data copied from the existing package gate.
/// Fields and constructor are crate-private so callers cannot synthesize a
/// trusted identity or declaration from request bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedManifest {
    app_id: u64,
    inventory_generation: NonZeroU64,
    declared_capabilities: Vec<Vec<u8>>,
}

impl TrustedManifest {
    pub(crate) fn from_verified_installed_manifest(
        app_id: u64,
        inventory_generation: u64,
        declared_capabilities: impl IntoIterator<Item = Vec<u8>>,
    ) -> Option<Self> {
        let inventory_generation = NonZeroU64::new(inventory_generation)?;
        let declared_capabilities: Vec<_> = declared_capabilities.into_iter().collect();
        if declared_capabilities.iter().any(Vec::is_empty) {
            return None;
        }
        Some(Self {
            app_id,
            inventory_generation,
            declared_capabilities,
        })
    }

    fn declares(&self, capability: &[u8]) -> bool {
        self.declared_capabilities
            .iter()
            .any(|declared| declared.as_slice() == capability)
    }
}

/// A revocable production session lease bound to the exact app session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionLease {
    app_id: u64,
    app_session_id: u64,
    user_session_generation: NonZeroU64,
}

impl SessionLease {
    pub(crate) const fn from_live_session_authority(
        app_id: u64,
        app_session_id: u64,
        user_session_generation: u64,
    ) -> Option<Self> {
        let Some(user_session_generation) = NonZeroU64::new(user_session_generation) else {
            return None;
        };
        Some(Self {
            app_id,
            app_session_id,
            user_session_generation,
        })
    }

    /// Compare this exact lease tuple with a leaf selector before C1 binds the
    /// peer to its live session owner. This only compares selectors; it does
    /// not establish that the session remains live.
    pub fn matches_authenticated_peer_selector(
        self,
        selector: &AuthenticatedInteractivePeerSelector,
    ) -> bool {
        selector.matches_session(
            self.app_id,
            self.app_session_id,
            self.user_session_generation,
        )
    }

    /// Compare this opaque lease with an independently checked C1 session
    /// tuple. This is a read-only selector comparison; it neither creates a
    /// lease nor proves that the C1 session remains live.
    pub const fn matches_session(
        self,
        app_id: u64,
        app_session_id: u64,
        user_session_generation: u64,
    ) -> bool {
        self.app_id == app_id
            && self.app_session_id == app_session_id
            && self.user_session_generation.get() == user_session_generation
    }
}

/// Canonical route requirements supplied by the trusted route/mapping owner.
pub struct TrustedServiceRequirements<'a> {
    route: TrustedRouteKey,
    capabilities: &'a [&'a [u8]],
    consent_requirement: RouteConsentRequirement,
}

impl<'a> TrustedServiceRequirements<'a> {
    /// Keep this constructor in init-owned route wiring; never call it from a
    /// decoded service request or payload field.
    pub(crate) fn from_canonical_route(
        route: TrustedRouteKey,
        capabilities: &'a [&'a [u8]],
        consent_requirement: RouteConsentRequirement,
    ) -> Self {
        Self {
            route,
            capabilities,
            consent_requirement,
        }
    }
}

/// Errors are deliberately fail-closed and preserve the authority boundary.
#[derive(Clone, Debug)]
pub enum PeerDenial {
    InvalidProcess,
    Unavailable,
    /// The owner checked the credentials and declined them before issuing a Session.
    AccountRejected,
    /// The owner declined verification during its monotonic waiting period.
    AccountThrottled,
    InventoryUnavailable,
    PackageSnapshot(Arc<nagi_dev_service_catalog_consumer::SnapshotError>),
    NotInstalled,
    ManifestMismatch,
    StaleManifest,
    SessionUnavailable,
    SessionMismatch,
    SessionRevoked,
    ConsentUnavailable,
    ConsentNotRequired,
    ProcessInstanceUnavailable,
    ForeignSpawnOwner,
    NotDeclared,
    NotLive,
    ConsentRequired,
    ConsentDenied,
    ConsentPolicyMismatch,
    StalePeer,
    StaleRegistration,
    DuplicateRequestCommit,
    RequestIdentityConflict,
    RequestDedupCapacity,
    RequestInProgress,
    RequestOutcomeUnknown,
    RequestOperationFailed,
    StaleConnection,
    ConnectionRequestIdentityMismatch,
    InvalidRequestFrameLength,
    InvalidNipcRequest,
    RequestDeadlineExpired,
    RequestDecoderAlreadyStarted,
    IncompleteRequestFrame,
    UnknownNipcRoute,
    UnknownNipcOperation,
    EmptyNipcOperationRouteSet,
    DuplicateNipcOperationRoute,
    /// A receipt operation failed after entering the task-25 local receipt
    /// owner. Its outcome may be unknown; this is not a service effect.
    ServiceRequestReceiptNotRecorded(Arc<PeerDenial>),
    /// The effect callback was not entered. PID1 leaves the stable request in
    /// `NotInvoked`, so the same wire request may retry after a gate rejection.
    ServiceEffectNotInvoked(Arc<PeerDenial>),
    /// The owner guarantees the operation failed before any effect was
    /// applied. This terminal state is not retried without owner resolution.
    ServiceEffectFailed(Arc<PeerDenial>),
    /// The effect may have happened but its durable result is unavailable.
    /// Retrying the stable request is denied until the owner resolves it.
    ServiceEffectOutcomeUnknown(Arc<PeerDenial>),
    DuplicateProcess,
    RouteMismatch,
    StaleEndpoint,
    StaleSocket,
    FdDelegationNotPrevented,
    /// A task-local package receipt view was invalidated by connection, run,
    /// authority, package-generation, or owner-lifetime change.
    PackageReceiptInvalidated,
    /// Kernel process-handle acquisition/check failed; the typed errno remains
    /// attached for diagnostics and can never be promoted to authentication.
    KernelProcessInstance(Arc<ProcessInstanceError>),
    Capacity,
    GenerationExhausted,
    /// Authentication failed in the kernel reader. Keep its typed cause (and
    /// any errno/retry metadata added by task 39) available to diagnostics.
    KernelReader(Arc<PeerCredentialError>),
    /// The C2 peer-proof producer could not bind kernel credentials to a
    /// connected socket. This evidence never creates Nagi identity by itself.
    KernelPeerAuthentication(Arc<PeerAuthenticationError>),
    /// The task-39 owned stream rejected or invalidated its transport view.
    /// This is transport state, not session/grant revocation proof.
    PeerConnection(Arc<PeerConnectionError>),
    EmptyRequiredServiceSet,
    DuplicateRequiredService,
    RequiredServiceMissing,
    HandshakeIncomplete,
    UnresolvedSpawnCleanup,
    ReadyUnavailable,
    StaleRun,
    CapabilitySetMismatch,
}

pub trait TrustedInventory: Send + Sync {
    fn installed_manifest(&self, app_id: u64) -> Result<Option<TrustedManifest>, PeerDenial>;

    /// A bounded, nonblocking lease check used inside the Supervisor commit
    /// callback. It must not lock or call another authority registry.
    fn generation_is_current(
        &self,
        app_id: u64,
        generation: NonZeroU64,
    ) -> Result<bool, PeerDenial>;

    /// Validate every requested generation and hold the same owner lease for
    /// the full callback. A local PackageService adapter holds its shared
    /// mutex here; a cross-process implementation must acquire the real
    /// PackageService owner's transaction/lease and report owner loss as
    /// unavailable. A point-in-time `validate_current` followed by an
    /// unleased use does not satisfy this contract.
    fn with_current_manifests(
        &self,
        requested: &[(u64, NonZeroU64)],
        inspect: &mut dyn FnMut(&[TrustedManifest]) -> Result<(), PeerDenial>,
    ) -> Result<(), PeerDenial>;
}

/// A checked, connection-bound Session effect gate returned by the C1 adapter.
/// The adapter runs the callback through C1's checked
/// `SessionLeaseRevalidationHandle::with_live_session_effect`, after verifying
/// that this exact peer connection owns the supplied C2 lease. The callback
/// holds C1's Session lease gate until the complete C2 owner operation returns.
/// A point-in-time `check_live` or non-authoritative `try_live_snapshot` cannot
/// implement this contract.
pub trait BoundPeerSessionEffectGate: Send + Sync {
    fn with_live_session_effect(
        &self,
        lease: SessionLease,
        effect: &mut dyn FnMut() -> Result<(), PeerDenial>,
    ) -> Result<(), PeerDenial>;
}

/// Borrowed C2 admission for one accepted app connection and its trusted
/// SessionLease selector. This proves C2's spawn/endpoint owner tuple and
/// exact peer/selector match only; it is not C1 authentication or Session
/// liveness by itself. The A-root adapter composes it with C1's checked
/// account/session binding. C2 also passes the shared connection proof as a
/// separate value; C1 compares it with its own lower leaf-issued peer proof
/// and still validates its account and Session owner.
pub struct C2AppSessionAdmission<'binding, 'admission, 'connection> {
    owner: &'binding C2SpawnEndpointAdmission<'admission, 'connection>,
    lease: SessionLease,
}

impl<'binding, 'admission, 'connection> C2AppSessionAdmission<'binding, 'admission, 'connection> {
    pub fn matches_peer_and_lease(
        &self,
        peer: &AuthenticatedInteractivePeer<'_>,
        lease: SessionLease,
    ) -> bool {
        self.owner.matches_peer(peer)
            && self.lease == lease
            && lease.matches_authenticated_peer_selector(peer.selector())
    }

    /// C2-validated proof of the same leaf-authenticated connection. It is a
    /// correlation value only; the A-root adapter passes it to C1 alongside
    /// the leaf proof so C1 can independently compare the connection.
    pub fn accepted_connection_proof(&self) -> &C2AcceptedConnectionProof<'_> {
        self.owner.connection_proof()
    }

    pub const fn session_lease(&self) -> SessionLease {
        self.lease
    }
}

type PendingPeerBindingActivation = Box<dyn FnOnce() -> Result<(), PeerDenial> + Send + 'static>;
type PendingPeerBindingAbort = Box<dyn FnOnce() + Send + 'static>;

/// C1's staged binding transaction, held privately by C2 until its owner and
/// Session checks finish. The provisional gate may only be used by C2 while
/// running its final read-only authorization checks; callers receive the gate
/// only after activation and the enclosing C1 Session callback both succeed.
///
/// `activate` may publish the binding while C2's owner gate is held, but the
/// abort hook remains armed until C2 has observed successful completion of
/// the C1 Session gate. Activation runs while both the C1 Session gate and C2
/// owner gate are held, so it must not reacquire a non-reentrant lock already
/// held by either gate. On activation error, abort runs inline while both gates
/// are still held; on a later C2 owner-check failure it may run while the C2
/// owner gate is held, and on other drops it may run outside both gates. Abort
/// must not reacquire either gate, call APIs that may reacquire them, or wait
/// for work that needs them. It must synchronously roll back through a
/// transaction-local or otherwise non-reentrant operation, and be idempotent,
/// bounded, and non-panicking. Activation must publish state that this abort
/// operation can safely remove in each of those lock contexts.
/// If C1 fails or panics before returning this token, C1 must clean up its own
/// incomplete staging because C2 has not yet received an abort hook.
#[must_use = "dropping a pending C1 peer binding aborts its provisional state"]
pub struct PendingBoundPeerSession {
    provisional_gate: Arc<dyn BoundPeerSessionEffectGate>,
    activate: Option<PendingPeerBindingActivation>,
    abort: Option<PendingPeerBindingAbort>,
    activated: bool,
}

impl PendingBoundPeerSession {
    /// Build C2's pending transaction from an A-root-composed gate. The A root
    /// receives this non-forgeable C2 admission, maps the same peer through
    /// C1's accepted-account and checked-Session APIs, and keeps C1's pending
    /// gate, active effect gate, activation, and abort handles private inside
    /// its adapter. C2 accepts its own generic gate contract and has no C1
    /// crate dependency. The same borrowed peer and C2 admission are checked
    /// here so raw selectors cannot create an owner-bound transaction.
    pub fn from_app_admission(
        peer: &AuthenticatedInteractivePeer<'_>,
        app_admission: &C2AppSessionAdmission<'_, '_, '_>,
        provisional_gate: Arc<dyn BoundPeerSessionEffectGate>,
        activate: impl FnOnce() -> Result<(), PeerDenial> + Send + 'static,
        abort: impl FnOnce() + Send + 'static,
    ) -> Result<Self, PeerDenial> {
        if !app_admission.matches_peer_and_lease(peer, app_admission.session_lease()) {
            return Err(PeerDenial::SessionMismatch);
        }
        Ok(Self::new(provisional_gate, activate, abort))
    }

    /// Construct a provisional C1 binding for C2 to validate and activate.
    /// C1 must keep the gate and enrollment unavailable to other callers until
    /// `activate` runs, and `abort` must remove the staged or partially
    /// activated binding. `activate` runs while the C1 Session gate and C2
    /// owner gate are held. `abort` may run while either or both are held, so
    /// it must not reacquire them, call APIs that may reacquire them, or wait
    /// for work that needs them. Rollback must be synchronous, idempotent,
    /// bounded, and non-panicking. C2 alone invokes activation through its
    /// private owner transaction.
    fn new(
        provisional_gate: Arc<dyn BoundPeerSessionEffectGate>,
        activate: impl FnOnce() -> Result<(), PeerDenial> + Send + 'static,
        abort: impl FnOnce() + Send + 'static,
    ) -> Self {
        Self {
            provisional_gate,
            activate: Some(Box::new(activate)),
            abort: Some(Box::new(abort)),
            activated: false,
        }
    }

    fn provisional_gate(&self) -> Arc<dyn BoundPeerSessionEffectGate> {
        Arc::clone(&self.provisional_gate)
    }

    fn abort_now(&mut self) {
        if let Some(abort) = self.abort.take() {
            abort();
        }
    }

    /// Activate inside C2's shared owner gate, after all C2 checks passed.
    /// Keep abort armed until the enclosing C1 Session gate also returns Ok.
    fn activate_after_c2_owner_gate(&mut self) -> Result<(), PeerDenial> {
        if self.activated {
            return Err(PeerDenial::Unavailable);
        }
        let activate = self.activate.take().ok_or(PeerDenial::Unavailable)?;
        match activate() {
            Ok(()) => {
                self.activated = true;
                Ok(())
            }
            Err(error) => {
                self.abort_now();
                Err(error)
            }
        }
    }

    /// Finalize after the C1 Session callback returns successfully. On every
    /// earlier error, expiry, or unwind, Drop invokes the still-armed abort.
    fn commit_after_session_gate(mut self) -> Result<(), PeerDenial> {
        if !self.activated {
            return Err(PeerDenial::Unavailable);
        }
        self.abort.take();
        Ok(())
    }
}

impl Drop for PendingBoundPeerSession {
    fn drop(&mut self) {
        self.abort_now();
    }
}

/// C1's staged first-session transaction for one C2-validated accepted
/// connection. C1 keeps its owner-backed transaction object here so the
/// borrowed C1 admission need not be extended to `'static` or copied into a
/// detached authority. `commit` must first persist the real C1 Session and
/// caller, then return that Session's generation from a checked C1 context
/// (for the current C1 API, `C1IssuedInteractiveSession::checked_context()`
/// followed by `InteractiveSessionContext::revalidation_handle()`). The
/// generation is not caller authentication by itself; C2 uses it only after
/// this owner-backed commit succeeds and combines it with its own launch
/// selectors.
/// `abort` must discard staged state and, after `commit`, revoke the newly
/// committed caller and Session until `finish` is called. `abort` can run
/// while both the C1 first-session gate and C2 owner gate are held, so it must
/// use a bounded, synchronous, non-reentrant rollback path and must not panic.
/// `finish` is called only after C2 has attached and checked the real lease;
/// it must transfer or retain the issued C1 caller/Session in C1's live owner
/// so later C1 capture, revocation, and revalidation still use the real owner.
pub trait PendingInitialSessionTransaction {
    fn commit(&mut self) -> Result<NonZeroU64, PeerDenial>;
    fn abort(self: Box<Self>) -> Result<(), PeerDenial>;
    fn finish(self: Box<Self>);
}

/// C1's staged first-session transaction for one C2-validated accepted
/// connection. It contains no SessionLease until C2's owner postcheck calls
/// the C1 transaction's commit. Dropping before final attachment synchronously
/// aborts C1's staged or newly committed caller and Session state.
#[must_use = "dropping a pending initial Session aborts its staged C1 state"]
pub struct PendingInitialSession<'peer> {
    lease: Option<SessionLease>,
    transaction: Option<Box<dyn PendingInitialSessionTransaction + 'peer>>,
    rollback_failed: bool,
    _accepted_peer: std::marker::PhantomData<&'peer KernelAuthenticatedUnixPeer>,
}

impl<'peer> PendingInitialSession<'peer> {
    /// Build the C2 transaction wrapper from C1's staged initial account and
    /// Session state. No SessionLease exists yet: the C1 transaction commits
    /// a real Session and returns its checked generation only after C2's owner
    /// postcheck. C2 combines that generation with selectors from its live
    /// launch record and constructs the opaque SessionLease inside this
    /// crate. The transaction may borrow C1's admission for `'peer`; it is
    /// used synchronously and is never detached or required to be `'static`.
    /// This checks only that the shared correlation proof and borrowed kernel
    /// peer refer to the same live stream; C1 must authenticate that stream
    /// and source the eventual generation from its own committed live Session.
    pub fn from_c2_pre_session_connection(
        stream: &'peer UnixStream,
        peer: &'peer KernelAuthenticatedUnixPeer,
        connection_proof: &C2AcceptedConnectionProof<'peer>,
        transaction: impl PendingInitialSessionTransaction + 'peer,
    ) -> Result<Self, PeerDenial> {
        let pending = Self {
            lease: None,
            transaction: Some(Box::new(transaction)),
            rollback_failed: false,
            _accepted_peer: std::marker::PhantomData,
        };
        peer.revalidate(stream)
            .map_err(map_peer_authentication_error)?;
        if !connection_proof.matches_kernel_peer(peer) {
            return Err(PeerDenial::SessionMismatch);
        }
        Ok(pending)
    }

    fn abort_now(&mut self) -> Result<(), PeerDenial> {
        if let Some(transaction) = self.transaction.take() {
            if transaction.abort().is_err() {
                self.rollback_failed = true;
                return Err(PeerDenial::RequestOutcomeUnknown);
            }
        }
        if self.rollback_failed {
            Err(PeerDenial::RequestOutcomeUnknown)
        } else {
            Ok(())
        }
    }

    /// C2 calls this only after rechecking its owner while C1's first-session
    /// gate and C2 owner gate are both held. C1 returns the real Session's
    /// checked generation; this C2-owned boundary combines it with selectors
    /// captured from the trusted launch record. Abort remains armed until C2
    /// finishes validating and attaching the resulting lease.
    fn issue_session_after_c2_owner_gate(
        &mut self,
        app_id: u64,
        app_session_id: u64,
    ) -> Result<SessionLease, PeerDenial> {
        if self.lease.is_some() {
            return if self.abort_now().is_err() {
                Err(PeerDenial::RequestOutcomeUnknown)
            } else {
                Err(PeerDenial::Unavailable)
            };
        }
        let generation = match self.transaction.as_mut() {
            Some(transaction) => transaction.commit(),
            None => {
                return if self.rollback_failed {
                    Err(PeerDenial::RequestOutcomeUnknown)
                } else {
                    Err(PeerDenial::Unavailable)
                };
            }
        };
        match generation {
            Ok(generation) => {
                let Some(lease) = SessionLease::from_live_session_authority(
                    app_id,
                    app_session_id,
                    generation.get(),
                ) else {
                    return if self.abort_now().is_err() {
                        Err(PeerDenial::RequestOutcomeUnknown)
                    } else {
                        Err(PeerDenial::SessionMismatch)
                    };
                };
                self.lease = Some(lease);
                Ok(lease)
            }
            Err(error) => {
                if self.abort_now().is_err() {
                    Err(PeerDenial::RequestOutcomeUnknown)
                } else {
                    Err(error)
                }
            }
        }
    }

    /// Disarm rollback only after C2 validates and attaches the lease under
    /// both owner gates. C1's finish is infallible and consumes its retained
    /// rollback authority.
    fn finish_after_both_owner_gates(&mut self) -> Result<SessionLease, PeerDenial> {
        let lease = self.lease.ok_or(PeerDenial::Unavailable)?;
        let transaction = self.transaction.take().ok_or_else(|| {
            if self.rollback_failed {
                PeerDenial::RequestOutcomeUnknown
            } else {
                PeerDenial::Unavailable
            }
        })?;
        transaction.finish();
        Ok(lease)
    }

    fn rollback_failed(&self) -> bool {
        self.rollback_failed
    }
}

impl Drop for PendingInitialSession<'_> {
    fn drop(&mut self) {
        let _ = self.abort_now();
    }
}

/// C2's bridge for the A-root composition of the accepted peer and a checked
/// C1 Session. The A-root adapter receives both the borrowed leaf-issued peer
/// proof and C2's post-login `C2AppSessionAdmission`. It must validate the
/// token against that exact peer while C2's pre/post owner checks are in
/// force, then call C1 using only the lower `AuthenticatedInteractivePeer`
/// proof and exact Session selector. C1 must not import or accept C2-root
/// admission types. A must bind C1's checked `InteractiveSessionContext` and
/// owned `SessionLeaseRevalidationHandle` to the same accepted peer and
/// Session, then return a gate backed by C1's staged binding. Its callback
/// must be bounded and synchronous, run inside C1's live Session lease gate
/// while C2 takes its owner gate and completes the effect commit, and must
/// not re-enter C1 lifecycle/recovery APIs. It must not use a public marker,
/// caller-supplied IDs as authentication, or the proof's PID/UID/GID as a
/// Nagi principal. The supplied `SessionLease` is a trusted launch-record
/// selector; C1 must match it against its checked context.
///
/// C1's `try_live_snapshot` is maintenance-only; it cannot authorize protected
/// work. C1 now supplies peer-bound account mapping and staged/active Session
/// gates, but this C2 candidate has no A-root composition using them. The
/// default peer-binding method below therefore denies.
pub trait LiveSessionAuthority: Send + Sync {
    fn capture_live_unlocked(
        &self,
        app_id: u64,
        app_session_id: u64,
    ) -> Result<SessionLease, PeerDenial>;

    /// Must be a zero-wait read of the captured revocable lease. Contention is
    /// `Unavailable`; it must not call or wait on another owner while
    /// `authorize_and_commit` holds the Supervisor lock. A point-in-time
    /// result can reject stale state, but it cannot authorize a protected
    /// effect commit; those require `BoundPeerSessionEffectGate`.
    fn is_live_unlocked(&self, lease: SessionLease) -> Result<bool, PeerDenial>;

    /// Stage the first C1 caller and Session for a PID-1 spawn. C2 passes the
    /// exact accepted stream, its borrowed kernel peer, the trusted app and
    /// app-session selectors from its launch record, and the shared same-
    /// connection proof; it passes no C2 owner token. C1 must authenticate its
    /// own peer on this same stream, compare the proof with that peer, and
    /// stage account work using its own owner. The selectors are routing
    /// inputs, never caller identity. C2 has already decoded one bounded
    /// initial-login request from that exact accepted stream and passes the
    /// move-only, zeroizing request value; A/C1 must pass its borrowed fields
    /// directly to the real account verifier. No numeric SessionLease is
    /// created before the C1 commit.
    ///
    /// C1 must hold its first-session owner gate while it invokes `complete`
    /// with a `PendingInitialSession`. C2 rechecks its spawn/endpoint owner,
    /// then calls the pending C1 commit under C2's owner gate. C1 returns the
    /// generation from its real committed Session; C2 combines it with the
    /// trusted selectors held in its launch record and constructs the private
    /// SessionLease. C2 attaches that lease before calling C1's infallible
    /// finish. If C2 rejects or the callback fails, the pending value must
    /// synchronously abort staged state or revoke the committed caller and
    /// Session. An abort failure leaves the C2 entry closed for reconciliation.
    /// After `complete` succeeds, C1 must return success without another
    /// fallible postcheck: C2 has attached the committed lease while both
    /// owner gates were held.
    /// The default denies until a C1-backed A-root bridge is connected.
    fn with_pending_initial_session_from_c2_owner_checked_peer<'peer>(
        &'peer self,
        _app_id: u64,
        _app_session_id: u64,
        _stream: &'peer UnixStream,
        _peer: &'peer KernelAuthenticatedUnixPeer,
        _connection_proof: &C2AcceptedConnectionProof<'peer>,
        _login_request: &'peer AcceptedInitialLoginRequest<'peer>,
        _complete: &mut dyn FnMut(PendingInitialSession<'peer>) -> Result<(), PeerDenial>,
    ) -> Result<(), PeerDenial> {
        Err(PeerDenial::Unavailable)
    }

    /// Stage a kernel-verified peer from the exact registered accepted stream
    /// against the checked live Session selected by the trusted launch record.
    /// C2 invokes this method only inside the leaf's borrowed
    /// `AuthenticatedInteractivePeer` callback, with C2 owner checks before
    /// and after. The A-root adapter receives the C2 post-login admission and
    /// its shared `C2AcceptedConnectionProof`. It passes that value and the
    /// lower leaf-issued peer proof to C1. C1 must verify that the shared
    /// value matches its borrowed peer, compare the selector with its checked
    /// Session context, and retain its own revalidation handle rather than
    /// this short-lived borrow. It must
    /// resolve its canonical identity and Session owner here, and must not
    /// derive a PrincipalId or caller seal from numeric peer fields.
    ///
    /// The leaf listener-registration API rejects non-PID-1 callers but does
    /// not by itself prove that an arbitrary module within PID 1 is the
    /// canonical C2 registrar. C2's private root owner check compares the
    /// leaf-issued proof key with the exact spawn-validated endpoint owner
    /// before and after this call. The A root composes that C2 admission with
    /// C1's checked account/session mapping; it must not pass C2-root types into
    /// C1. C1 compares the lower selector's session tuple with its checked
    /// context and retains its own live revalidation handle. The selector/lease
    /// comparison is available through
    /// `SessionLease::matches_authenticated_peer_selector`.
    /// A public leaf-listener call or numeric selectors alone cannot enroll a
    /// key. Kernel PID/UID/GID and selector numbers are never caller identity.
    ///
    /// Return a `PendingBoundPeerSession` created with
    /// `from_app_admission`. A must source its provisional gate, activation,
    /// and abort handles from C1's staged binding and keep C1's types private
    /// to the A-root adapter. Staging must not publish an enrollment or an
    /// externally usable gate. C2 uses the provisional gate only inside its
    /// final owner check, activates while the shared C2 owner gate is held,
    /// and releases abort only after C1's enclosing live Session callback
    /// returns successfully and C2 rechecks owner state under the shared
    /// gate. Activation must publish atomically or be reversible; abort must
    /// remove staged or partially activated state on denial, error, panic,
    /// or expiry. Abort can execute while C1's Session gate, C2's owner gate,
    /// or both are held, so it must not reacquire those gates, call APIs that
    /// may reacquire them, or wait for work that requires them. Rollback must
    /// be a bounded, idempotent, non-panicking transaction-local operation.
    /// If staging fails or panics before this method returns, C1 must clean up
    /// that partial work.
    fn begin_authenticated_peer_binding(
        &self,
        _peer: &AuthenticatedInteractivePeer<'_>,
        _app_admission: &C2AppSessionAdmission<'_, '_, '_>,
        _connection_proof: &C2AcceptedConnectionProof<'_>,
    ) -> Result<PendingBoundPeerSession, PeerDenial> {
        Err(PeerDenial::Unavailable)
    }
}

#[cfg(test)]
mod pending_bound_peer_session_tests {
    use super::{
        run_session_effect, BoundPeerSessionEffectGate, PeerDenial, PendingBoundPeerSession,
        SessionEffectGate, SessionLease,
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    struct TestSessionEffectGate;

    impl BoundPeerSessionEffectGate for TestSessionEffectGate {
        fn with_live_session_effect(
            &self,
            _lease: SessionLease,
            effect: &mut dyn FnMut() -> Result<(), PeerDenial>,
        ) -> Result<(), PeerDenial> {
            effect()
        }
    }

    fn pending(
        activation: impl FnOnce() -> Result<(), PeerDenial> + Send + 'static,
        activations: Arc<AtomicUsize>,
        aborts: Arc<AtomicUsize>,
    ) -> PendingBoundPeerSession {
        PendingBoundPeerSession::new(
            Arc::new(TestSessionEffectGate),
            move || {
                activations.fetch_add(1, Ordering::SeqCst);
                activation()
            },
            move || {
                aborts.fetch_add(1, Ordering::SeqCst);
            },
        )
    }

    #[test]
    fn dropping_before_activation_aborts_without_publishing() {
        let activations = Arc::new(AtomicUsize::new(0));
        let aborts = Arc::new(AtomicUsize::new(0));
        let pending = pending(|| Ok(()), Arc::clone(&activations), Arc::clone(&aborts));

        drop(pending);

        assert_eq!(activations.load(Ordering::SeqCst), 0);
        assert_eq!(aborts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn activation_error_aborts_staged_binding() {
        let activations = Arc::new(AtomicUsize::new(0));
        let aborts = Arc::new(AtomicUsize::new(0));
        let mut pending = pending(
            || Err(PeerDenial::Unavailable),
            Arc::clone(&activations),
            Arc::clone(&aborts),
        );

        assert!(matches!(
            pending.activate_after_c2_owner_gate(),
            Err(PeerDenial::Unavailable)
        ));
        drop(pending);

        assert_eq!(activations.load(Ordering::SeqCst), 1);
        assert_eq!(aborts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn activation_error_rolls_back_while_both_gates_are_held() {
        let session_gate_held = Arc::new(AtomicBool::new(false));
        let c2_owner_gate_held = Arc::new(AtomicBool::new(false));
        let simulated_binding_active = Arc::new(AtomicBool::new(false));
        let abort_observed_both_gates = Arc::new(AtomicBool::new(false));
        let rollback_count = Arc::new(AtomicUsize::new(0));
        let abort_callbacks = Arc::new(AtomicUsize::new(0));

        let mut pending = PendingBoundPeerSession::new(
            Arc::new(TestSessionEffectGate),
            {
                let simulated_binding_active = Arc::clone(&simulated_binding_active);
                move || {
                    simulated_binding_active.store(true, Ordering::SeqCst);
                    Err(PeerDenial::Unavailable)
                }
            },
            {
                let session_gate_held = Arc::clone(&session_gate_held);
                let c2_owner_gate_held = Arc::clone(&c2_owner_gate_held);
                let simulated_binding_active = Arc::clone(&simulated_binding_active);
                let abort_observed_both_gates = Arc::clone(&abort_observed_both_gates);
                let rollback_count = Arc::clone(&rollback_count);
                let abort_callbacks = Arc::clone(&abort_callbacks);
                move || {
                    abort_observed_both_gates.store(
                        session_gate_held.load(Ordering::SeqCst)
                            && c2_owner_gate_held.load(Ordering::SeqCst),
                        Ordering::SeqCst,
                    );
                    if simulated_binding_active.swap(false, Ordering::SeqCst) {
                        rollback_count.fetch_add(1, Ordering::SeqCst);
                    }
                    abort_callbacks.fetch_add(1, Ordering::SeqCst);
                }
            },
        );
        let session_gate: &SessionEffectGate<'_> = &{
            let session_gate_held = Arc::clone(&session_gate_held);
            move |callback| {
                session_gate_held.store(true, Ordering::SeqCst);
                let result = callback();
                session_gate_held.store(false, Ordering::SeqCst);
                result
            }
        };

        let result = run_session_effect(session_gate, || {
            c2_owner_gate_held.store(true, Ordering::SeqCst);
            let result = pending.activate_after_c2_owner_gate();
            c2_owner_gate_held.store(false, Ordering::SeqCst);
            result
        });

        drop(pending);
        assert!(matches!(result, Err(PeerDenial::Unavailable)));
        assert!(abort_observed_both_gates.load(Ordering::SeqCst));
        assert!(!simulated_binding_active.load(Ordering::SeqCst));
        assert_eq!(rollback_count.load(Ordering::SeqCst), 1);
        assert_eq!(abort_callbacks.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn activation_panic_aborts_staged_binding() {
        let activations = Arc::new(AtomicUsize::new(0));
        let aborts = Arc::new(AtomicUsize::new(0));
        let mut pending = pending(
            || panic!("activation panic test"),
            Arc::clone(&activations),
            Arc::clone(&aborts),
        );

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = pending.activate_after_c2_owner_gate();
        }));
        drop(pending);

        assert!(result.is_err());
        assert_eq!(activations.load(Ordering::SeqCst), 1);
        assert_eq!(aborts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn abort_remains_armed_until_session_gate_returns() {
        let activations = Arc::new(AtomicUsize::new(0));
        let aborts = Arc::new(AtomicUsize::new(0));
        let mut pending = pending(|| Ok(()), Arc::clone(&activations), Arc::clone(&aborts));

        pending
            .activate_after_c2_owner_gate()
            .expect("activate under the C2 owner gate");
        drop(pending);

        assert_eq!(activations.load(Ordering::SeqCst), 1);
        assert_eq!(aborts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn session_gate_error_after_activation_aborts() {
        let activations = Arc::new(AtomicUsize::new(0));
        let aborts = Arc::new(AtomicUsize::new(0));
        let mut pending = pending(|| Ok(()), Arc::clone(&activations), Arc::clone(&aborts));
        let gate: &SessionEffectGate<'_> = &|callback| {
            callback()?;
            Err(PeerDenial::SessionRevoked)
        };

        let result = run_session_effect(gate, || pending.activate_after_c2_owner_gate());
        drop(pending);

        assert!(matches!(
            result,
            Err(PeerDenial::ServiceEffectOutcomeUnknown(cause))
                if matches!(cause.as_ref(), PeerDenial::SessionRevoked)
        ));
        assert_eq!(activations.load(Ordering::SeqCst), 1);
        assert_eq!(aborts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn session_gate_panic_after_activation_aborts() {
        let activations = Arc::new(AtomicUsize::new(0));
        let aborts = Arc::new(AtomicUsize::new(0));
        let mut pending = pending(|| Ok(()), Arc::clone(&activations), Arc::clone(&aborts));
        let gate: &SessionEffectGate<'_> = &|callback| {
            callback()?;
            panic!("session gate panic test");
        };

        let result = run_session_effect(gate, || pending.activate_after_c2_owner_gate());
        drop(pending);

        assert!(matches!(
            result,
            Err(PeerDenial::ServiceEffectOutcomeUnknown(cause))
                if matches!(cause.as_ref(), PeerDenial::RequestOutcomeUnknown)
        ));
        assert_eq!(activations.load(Ordering::SeqCst), 1);
        assert_eq!(aborts.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn successful_session_gate_completion_commits_and_returns_the_gate() {
        let activations = Arc::new(AtomicUsize::new(0));
        let aborts = Arc::new(AtomicUsize::new(0));
        let mut pending = pending(|| Ok(()), Arc::clone(&activations), Arc::clone(&aborts));

        pending
            .activate_after_c2_owner_gate()
            .expect("activate under the C2 owner gate");
        let gate = pending.provisional_gate();
        pending
            .commit_after_session_gate()
            .expect("commit after the C1 session callback succeeds");
        let mut checked = false;
        gate.with_live_session_effect(
            SessionLease::from_live_session_authority(1, 2, 3).unwrap(),
            &mut || {
                checked = true;
                Ok(())
            },
        )
        .expect("returned gate remains usable");

        assert!(checked);
        assert_eq!(activations.load(Ordering::SeqCst), 1);
        assert_eq!(aborts.load(Ordering::SeqCst), 0);
    }
}

type SessionEffectGate<'a> =
    dyn Fn(&mut dyn FnMut() -> Result<(), PeerDenial>) -> Result<(), PeerDenial> + Send + Sync + 'a;

#[cfg(target_os = "linux")]
type AuthenticatedConnectionRevalidator<'a> = dyn for<'fd> Fn(BorrowedFd<'fd>, &KernelPeerCredentials) -> Result<(), PeerDenial>
    + Send
    + Sync
    + 'a;

fn run_session_effect<T>(
    gate: &SessionEffectGate<'_>,
    effect: impl FnOnce() -> Result<T, PeerDenial>,
) -> Result<T, PeerDenial> {
    let mut effect = Some(effect);
    let mut outcome = None;
    let mut invoked = false;
    let mut callback = || {
        let effect = effect.take().ok_or(PeerDenial::Unavailable)?;
        invoked = true;
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(effect)) {
            Ok(result) => {
                outcome = Some(result);
                Ok(())
            }
            Err(payload) => std::panic::resume_unwind(payload),
        }
    };
    let gate_result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| gate(&mut callback)));
    match gate_result {
        Err(_) if invoked => Err(PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(
            PeerDenial::RequestOutcomeUnknown,
        ))),
        Err(_) => Err(PeerDenial::Unavailable),
        Ok(Err(cause)) if invoked => Err(PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(cause))),
        Ok(Err(cause)) => Err(cause),
        Ok(Ok(())) => match (invoked, outcome) {
            (true, Some(outcome)) => outcome,
            (true, None) => Err(PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(
                PeerDenial::RequestOutcomeUnknown,
            ))),
            (false, _) => Err(PeerDenial::Unavailable),
        },
    }
}

#[cfg(test)]
mod session_effect_gate_tests {
    use super::{run_session_effect, PeerDenial, SessionEffectGate};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn caught_callback_panic_is_unknown_after_partial_effect() {
        let effects = AtomicUsize::new(0);
        let gate: &SessionEffectGate<'_> = &|callback| {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback()));
            Ok(())
        };

        let result = run_session_effect(gate, || -> Result<(), PeerDenial> {
            effects.fetch_add(1, Ordering::SeqCst);
            panic!("effect failed after a partial side effect");
        });

        assert!(matches!(
            result,
            Err(PeerDenial::ServiceEffectOutcomeUnknown(cause))
                if matches!(cause.as_ref(), PeerDenial::RequestOutcomeUnknown)
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn gate_error_after_callback_is_unknown() {
        let effects = AtomicUsize::new(0);
        let gate: &SessionEffectGate<'_> = &|callback| {
            callback()?;
            Err(PeerDenial::SessionRevoked)
        };

        let result = run_session_effect(gate, || {
            effects.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });

        assert!(matches!(
            result,
            Err(PeerDenial::ServiceEffectOutcomeUnknown(cause))
                if matches!(cause.as_ref(), PeerDenial::SessionRevoked)
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn repeated_callback_invokes_effect_once() {
        let effects = AtomicUsize::new(0);
        let gate: &SessionEffectGate<'_> = &|callback| {
            callback()?;
            let _second_callback = callback();
            Ok(())
        };

        assert!(run_session_effect(gate, || {
            effects.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .is_ok());
        assert_eq!(effects.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn gate_panic_after_callback_is_unknown() {
        let effects = AtomicUsize::new(0);
        let gate: &SessionEffectGate<'_> = &|callback| {
            callback()?;
            panic!("gate failed after callback completion");
        };

        let result = run_session_effect(gate, || {
            effects.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });

        assert!(matches!(
            result,
            Err(PeerDenial::ServiceEffectOutcomeUnknown(cause))
                if matches!(cause.as_ref(), PeerDenial::RequestOutcomeUnknown)
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
    }
}

fn with_authenticated_peer_session_effect<S: LiveSessionAuthority, T>(
    sessions: &S,
    peer_session: Option<&dyn BoundPeerSessionEffectGate>,
    lease: SessionLease,
    effect: impl FnOnce() -> Result<T, PeerDenial>,
) -> Result<T, PeerDenial> {
    if let Some(peer_session) = peer_session {
        return run_session_effect(
            &|callback| peer_session.with_live_session_effect(lease, callback),
            effect,
        );
    }
    #[cfg(test)]
    {
        if sessions.is_live_unlocked(lease)? {
            effect()
        } else {
            Err(PeerDenial::SessionRevoked)
        }
    }
    #[cfg(not(test))]
    {
        let _ = (sessions, effect);
        let _ = lease;
        Err(PeerDenial::Unavailable)
    }
}

pub trait ConsentSource: Send + Sync {
    /// Reports source connectivity only. `true` is not an approval, does not
    /// identify who consented, and does not prove a grant remains current.
    /// Production authorization also needs a session-owned event revision.
    fn connected(&self) -> Result<bool, PeerDenial>;
}

mod session_consent_authority_seal {
    pub trait Sealed {}
}

/// The live session owner is the only authority that can issue and revalidate
/// consent events. Issuance must bind the request's session generation,
/// registration/process instance, route/endpoint, capability, and unique
/// request ID. Revocation or replacement must advance the event revision, and
/// `is_current_unlocked` must compare identity plus revision without blocking.
/// Its mutation path must take the same PID1 gate before changing current
/// consent so a commit can serialize with revocation. No production source
/// implements this seam yet; a connected boolean cannot substitute for it.
pub trait SessionOwnedConsentAuthority:
    session_consent_authority_seal::Sealed + Send + Sync
{
    fn issue_event(
        &self,
        admission: &ServiceRequestAdmission,
    ) -> Result<SessionConsentGrantEvent, PeerDenial>;

    fn is_current_unlocked(&self, event: &SessionConsentGrantEvent) -> Result<bool, PeerDenial>;
}

/// Explicit fail-closed provider used while the live session/consent owner is
/// absent. It never manufactures a consent event from a connected flag.
pub struct UnavailableSessionConsentAuthority;

impl session_consent_authority_seal::Sealed for UnavailableSessionConsentAuthority {}

impl SessionOwnedConsentAuthority for UnavailableSessionConsentAuthority {
    fn issue_event(
        &self,
        _admission: &ServiceRequestAdmission,
    ) -> Result<SessionConsentGrantEvent, PeerDenial> {
        Err(PeerDenial::ConsentUnavailable)
    }

    fn is_current_unlocked(&self, _event: &SessionConsentGrantEvent) -> Result<bool, PeerDenial> {
        Err(PeerDenial::ConsentUnavailable)
    }
}

/// Additional kernel process-lifetime evidence needed to distinguish a stale
/// socket whose SO_PEERCRED PID has since been reused. This must compare the
/// peer process instance to the handle captured by PID 1 at spawn; matching a
/// PID, UID/GID, SO_COOKIE, pathname, or payload value alone is insufficient.
/// The task-39 reader does not implement this proof.
mod peer_process_instance_seal {
    pub trait Sealed {}
}

pub trait PeerProcessInstanceAuthority: peer_process_instance_seal::Sealed + Send + Sync {
    /// Must be a bounded, nonblocking check over kernel process handles and
    /// must not call an authority registry while the Supervisor lock is held.
    fn confirms_same_live_process(
        &self,
        stream: &UnixStream,
        peer_pid: u32,
        peer_uid: u32,
        peer_gid: u32,
        spawned: ProcessInstanceKey,
    ) -> Result<bool, PeerDenial>;

    /// Recheck only the PID 1-owned child handle after the connection-time
    /// peer PID has already been matched to it. EOF and a saved credential
    /// snapshot are not process-liveness evidence.
    fn confirms_spawned_instance_live(
        &self,
        spawned: ProcessInstanceKey,
    ) -> Result<bool, PeerDenial>;

    /// Read the PID 1-owned cleanup ledger. Readiness is unavailable while an
    /// unregistered spawn or capture-failure cleanup remains unresolved.
    fn has_unresolved_cleanup(&self) -> Result<bool, PeerDenial>;
}

mod readiness_boundary_seal {
    pub trait Sealed {}
}

struct Pid1AuthorityDomainState {
    generation: NonZeroU64,
    authority_nonce: [u8; 32],
    active: Arc<AtomicBool>,
    owner_gate: AtomicBool,
}

/// Run-bound C2 authority domain and production nonblocking owner gate.
/// `Pid1LivePeerDirectory` issues it with each run; clones retain the same
/// lock and become stale when that run is invalidated.
#[derive(Clone)]
pub struct Pid1AuthorityDomain(Arc<Pid1AuthorityDomainState>);

impl Pid1AuthorityDomain {
    fn for_run(generation: NonZeroU64, authority_nonce: [u8; 32], active: Arc<AtomicBool>) -> Self {
        Self(Arc::new(Pid1AuthorityDomainState {
            generation,
            authority_nonce,
            active,
            owner_gate: AtomicBool::new(false),
        }))
    }

    #[cfg(any(test, feature = "linux-runtime-fixture"))]
    pub(crate) fn new() -> Self {
        Self::for_run(
            NonZeroU64::new(1).expect("nonzero fixture generation"),
            [0; 32],
            Arc::new(AtomicBool::new(true)),
        )
    }

    pub fn is_same_domain(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    fn generation(&self) -> NonZeroU64 {
        self.0.generation
    }

    fn authority_nonce(&self) -> [u8; 32] {
        self.0.authority_nonce
    }
}

struct Pid1OwnerGateRelease<'a>(&'a AtomicBool);

impl Drop for Pid1OwnerGateRelease<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl readiness_boundary_seal::Sealed for Pid1AuthorityDomain {}

/// The PID 1 run's shared, nonblocking owner gate. Every mutation that can
/// invalidate readiness must either acquire this exact domain or keep its
/// authoritative owner lease held through the complete Ready mapping callback.
/// A C2-local lock alone does not make external owner state atomic. In
/// particular, the current one-record
/// `SupervisorLaunchAuthority::authorize_and_commit` contract does not hold a
/// multi-route grant snapshot through Ready consumption, and
/// `LiveSessionAuthority::is_live_unlocked` is only a point-in-time read.
/// Existing Supervisor callbacks remain short and must not reacquire this C2
/// gate. Until all external owners meet the shared-gate/held-lease contract,
/// this implementation is only the C2 run gate, not proof that production may
/// publish Ready.
pub trait Pid1ReadinessAtomicBoundary: readiness_boundary_seal::Sealed + Send + Sync {
    /// Stable identity shared by the gate used for registration, revocation,
    /// stream revalidation, and service-effect commit.
    fn authority_domain(&self) -> &Pid1AuthorityDomain;

    fn with_owner_gate<T>(
        &self,
        run: &Pid1RunInstance,
        required: Option<&RequiredServiceSet>,
        operation: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial>;
}

impl Pid1ReadinessAtomicBoundary for Pid1AuthorityDomain {
    fn authority_domain(&self) -> &Pid1AuthorityDomain {
        self
    }

    fn with_owner_gate<T>(
        &self,
        run: &Pid1RunInstance,
        required: Option<&RequiredServiceSet>,
        operation: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        if !self.is_same_domain(&run.authority_domain)
            || self.generation() != run.generation
            || self.authority_nonce() != run.authority_nonce
            || !Arc::ptr_eq(&self.0.active, &run.active)
        {
            return Err(PeerDenial::StaleRun);
        }
        if required.is_some_and(|set| set.routes.is_empty()) {
            return Err(PeerDenial::EmptyRequiredServiceSet);
        }
        if !run.is_active() {
            return Err(PeerDenial::StaleRun);
        }
        self.0
            .owner_gate
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map_err(|_| PeerDenial::Unavailable)?;
        let _release = Pid1OwnerGateRelease(&self.0.owner_gate);
        if !run.is_active() {
            return Err(PeerDenial::StaleRun);
        }
        operation()
    }
}

fn require_same_authority_domain<B: Pid1ReadinessAtomicBoundary>(
    readiness_boundary: &B,
    expected: &Pid1AuthorityDomain,
) -> Result<(), PeerDenial> {
    if readiness_boundary
        .authority_domain()
        .is_same_domain(expected)
    {
        Ok(())
    } else {
        Err(PeerDenial::Unavailable)
    }
}

pub trait SupervisorLaunchAuthority: Send + Sync {
    type Record: LaunchRecordView;

    fn resolve(&self, process_id: u32) -> Option<Self::Record>;

    /// Check the exact live record and all required grants, then call `commit`
    /// under the Supervisor's launch registry lock. The source implementation
    /// is the existing `authorize_launch_and_commit` contract.
    fn authorize_and_commit<T>(
        &self,
        record: Self::Record,
        capabilities: &[&[u8]],
        commit: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct RegistrationId(NonZeroU64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntryState {
    Pending,
    Live,
}

#[derive(Clone, Debug)]
struct Entry<R: LaunchRecordView> {
    id: RegistrationId,
    run_generation: NonZeroU64,
    process: ProcessInstanceKey,
    task24_identity: SpawnedProcessIdentity,
    record: R,
    route: TrustedRouteKey,
    endpoint: TrustedServiceEndpoint,
    capabilities: Vec<Vec<u8>>,
    consent_requirement: RouteConsentRequirement,
    inventory_generation: NonZeroU64,
    session: Option<SessionLease>,
    system_service_owner: Option<SystemServiceOwnerBinding>,
    initial_session_issuance_pending: Arc<AtomicBool>,
    state: EntryState,
    retirement_pending: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SystemServiceOwnerBinding {
    run_generation: NonZeroU64,
    process_identity: SpawnedProcessIdentity,
    service_id: String,
    catalog_generation: NonZeroU64,
    c3_launch_generation: NonZeroU64,
    c3_control_endpoint_generation: NonZeroU64,
    c2_service_endpoint_generation: NonZeroU64,
}

struct InitialSessionIssuanceGuard {
    pending: Arc<AtomicBool>,
    finished: bool,
    session_attached: bool,
    rollback_failed: bool,
}

impl InitialSessionIssuanceGuard {
    fn acquire(pending: Arc<AtomicBool>) -> Result<Self, PeerDenial> {
        pending
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| PeerDenial::Unavailable)?;
        Ok(Self {
            pending,
            finished: false,
            session_attached: false,
            rollback_failed: false,
        })
    }

    fn mark_session_attached(&mut self) {
        self.session_attached = true;
    }

    fn mark_rollback_failed(&mut self) {
        self.rollback_failed = true;
    }

    fn finish(mut self) {
        self.pending.store(false, Ordering::SeqCst);
        self.finished = true;
    }
}

impl Drop for InitialSessionIssuanceGuard {
    fn drop(&mut self) {
        if !self.finished && !self.session_attached && !self.rollback_failed {
            self.pending.store(false, Ordering::SeqCst);
        }
    }
}

impl<R: LaunchRecordView> Entry<R> {
    fn is_live(&self) -> bool {
        self.state == EntryState::Live && !self.retirement_pending.load(Ordering::SeqCst)
    }

    fn live_session(&self) -> Result<SessionLease, PeerDenial> {
        if self.initial_session_issuance_pending.load(Ordering::SeqCst) {
            return Err(PeerDenial::SessionUnavailable);
        }
        self.session.ok_or(PeerDenial::SessionUnavailable)
    }
}

fn mark_endpoint_retirement_pending(retirement_pending: &AtomicBool) {
    retirement_pending.store(true, Ordering::SeqCst);
}

struct DirectoryState<R: LaunchRecordView> {
    next_generation: u64,
    next_run_generation: u64,
    current_run_generation: Option<NonZeroU64>,
    current_run_authority_nonce: Option<[u8; 32]>,
    current_run_active: Option<Arc<AtomicBool>>,
    current_authority_domain: Option<Pid1AuthorityDomain>,
    entries: Vec<Entry<R>>,
    package_receipt_revokers: Vec<Weak<dyn PackageRequestReceiptRevoker>>,
}

mod package_request_receipt_revoker_seal {
    pub trait Sealed {}
}

/// A package receipt table registers its owned revocation state with the live
/// directory when captured. The directory invokes it under the same owner
/// gate as registration retirement and run termination. It only cleans local
/// task-25 receipt metadata; it is not a durable effect resolver.
pub(crate) trait PackageRequestReceiptRevoker:
    package_request_receipt_revoker_seal::Sealed + Send + Sync
{
    fn revoke_registration_under_gate(
        &self,
        run: &Pid1RunInstance,
        registration: RegistrationId,
        process: ProcessInstanceKey,
    ) -> Result<(), PeerDenial>;

    fn revoke_run_under_gate(&self, run: &Pid1RunInstance) -> Result<(), PeerDenial>;
}

/// PID 1's bounded live peer index. The stream transport still owns socket
/// accept/connect and must call `authorize_peer` at each stream checkout.
pub struct Pid1LivePeerDirectory<R: LaunchRecordView> {
    owner: Arc<DirectoryOwnerIdentity>,
    state: Mutex<DirectoryState<R>>,
    request_ledger: Arc<Mutex<ServiceRequestLedger>>,
}

impl<R: LaunchRecordView> Default for Pid1LivePeerDirectory<R> {
    fn default() -> Self {
        Self::new()
    }
}

impl<R: LaunchRecordView> Pid1LivePeerDirectory<R> {
    pub fn new() -> Self {
        Self {
            owner: Arc::new(DirectoryOwnerIdentity),
            state: Mutex::new(DirectoryState {
                next_generation: 1,
                next_run_generation: 1,
                current_run_generation: None,
                current_run_authority_nonce: None,
                current_run_active: None,
                current_authority_domain: None,
                entries: Vec::with_capacity(MAX_LIVE_PEERS),
                package_receipt_revokers: Vec::with_capacity(MAX_LIVE_PEERS),
            }),
            request_ledger: Arc::new(Mutex::new(ServiceRequestLedger::default())),
        }
    }

    /// Start the one run owned by this in-memory PID 1 registry. Reinitializing
    /// a run requires a fresh directory/process lifetime; stale registrations
    /// cannot be carried into a new run token. This identity-only entry is
    /// retained for non-Ready peer operations. Readiness creation requires the
    /// explicit owner lease returned by `begin_run_owner_lease()`.
    pub fn begin_run(&self) -> Result<Pid1RunInstance, PeerDenial> {
        self.begin_run_instance()
    }

    /// Begin a lifecycle run while retaining the owner lease required to
    /// create/map typed Ready events and to invalidate them at run exit.
    pub fn begin_run_owner_lease(&self) -> Result<Pid1RunOwnerLease, PeerDenial> {
        self.begin_run_instance()
            .map(|run| Pid1RunOwnerLease { run })
    }

    fn begin_run_instance(&self) -> Result<Pid1RunInstance, PeerDenial> {
        let mut state = self.state.lock().map_err(|_| PeerDenial::Unavailable)?;
        if state.current_run_generation.is_some() || !state.entries.is_empty() {
            return Err(PeerDenial::StaleRun);
        }
        state
            .package_receipt_revokers
            .retain(|revoker| revoker.strong_count() > 0);
        if !state.package_receipt_revokers.is_empty() {
            return Err(PeerDenial::Unavailable);
        }
        if !self
            .request_ledger
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .entries
            .is_empty()
        {
            return Err(PeerDenial::Unavailable);
        }
        let authority_nonce = new_run_authority_nonce()?;
        let generation = state.next_run_generation;
        state.next_run_generation = generation
            .checked_add(1)
            .ok_or(PeerDenial::GenerationExhausted)?;
        let generation = NonZeroU64::new(generation).ok_or(PeerDenial::GenerationExhausted)?;
        let active = Arc::new(AtomicBool::new(true));
        let authority_domain =
            Pid1AuthorityDomain::for_run(generation, authority_nonce, Arc::clone(&active));
        state.current_run_generation = Some(generation);
        state.current_run_authority_nonce = Some(authority_nonce);
        state.current_run_active = Some(Arc::clone(&active));
        state.current_authority_domain = Some(authority_domain.clone());
        Ok(Pid1RunInstance {
            owner: Arc::clone(&self.owner),
            generation,
            authority_nonce,
            active,
            authority_domain,
        })
    }

    /// End a PID 1 lifecycle run under the same gate used by registrations,
    /// readiness snapshots, and effect commits. This invalidates all cloned
    /// run tokens before clearing registrations, so an old queued Ready event
    /// cannot be mapped after cancellation or required-service exit.
    pub fn end_run_owner_lease<B: Pid1ReadinessAtomicBoundary>(
        &self,
        lease: &Pid1RunOwnerLease,
        readiness_boundary: &B,
    ) -> Result<(), PeerDenial> {
        let run = lease.run.clone();
        readiness_boundary.with_owner_gate(&run, None, || {
            self.require_current_run(&run)?;
            let revokers = self.package_receipt_revokers_for_run(&run)?;
            let mut revoke_failure = None;
            for revoker in revokers {
                if let Err(error) = revoker.revoke_run_under_gate(&run) {
                    if revoke_failure.is_none() {
                        revoke_failure = Some(error);
                    }
                }
            }
            // End the run even if a local table is poisoned/unavailable. A
            // failed cleanup is reported after invalidation; old request
            // authority must never remain usable merely to preserve the cache.
            run.active.store(false, Ordering::SeqCst);
            let mut state = self.state.lock().map_err(|_| PeerDenial::Unavailable)?;
            if state.current_run_generation != Some(run.generation) {
                return Err(PeerDenial::StaleRun);
            }
            state.current_run_generation = None;
            state.current_run_authority_nonce = None;
            state.current_run_active = None;
            state.current_authority_domain = None;
            state.entries.clear();
            state.package_receipt_revokers.clear();
            // Invalidate first, then clear ledger keys under the same owner
            // gate. Any request already executing held that gate until its
            // outcome was recorded; a cloned in-flight state is not reset.
            self.request_ledger
                .lock()
                .map_err(|_| PeerDenial::Unavailable)?
                .clear_run(run.generation);
            revoke_failure.map_or(Ok(()), Err)
        })
    }

    fn package_receipt_revokers_for_run(
        &self,
        run: &Pid1RunInstance,
    ) -> Result<Vec<Arc<dyn PackageRequestReceiptRevoker>>, PeerDenial> {
        let mut state = self.state.lock().map_err(|_| PeerDenial::Unavailable)?;
        if state.current_run_generation != Some(run.generation) {
            return Err(PeerDenial::StaleRun);
        }
        state
            .package_receipt_revokers
            .retain(|revoker| revoker.strong_count() > 0);
        Ok(state
            .package_receipt_revokers
            .iter()
            .filter_map(Weak::upgrade)
            .collect())
    }

    pub(crate) fn register_package_request_receipt_revoker(
        &self,
        run: &Pid1RunInstance,
        registration: RegistrationId,
        process: ProcessInstanceKey,
        app_id: u64,
        inventory_generation: u64,
        revoker: &Arc<dyn PackageRequestReceiptRevoker>,
    ) -> Result<(), PeerDenial> {
        let mut state = self.state.lock().map_err(|_| PeerDenial::Unavailable)?;
        if !Arc::ptr_eq(&self.owner, &run.owner)
            || !run.is_active()
            || state.current_run_generation != Some(run.generation)
        {
            return Err(PeerDenial::StaleRun);
        }
        if !state.entries.iter().any(|entry| {
            entry.id == registration
                && entry.process == process
                && entry.record.app_id() == app_id
                && entry.inventory_generation.get() == inventory_generation
                && entry.is_live()
        }) {
            return Err(PeerDenial::StaleRegistration);
        }
        state
            .package_receipt_revokers
            .retain(|current| current.strong_count() > 0);
        if state
            .package_receipt_revokers
            .iter()
            .filter_map(Weak::upgrade)
            .any(|current| Arc::ptr_eq(&current, revoker))
        {
            return Ok(());
        }
        if state.package_receipt_revokers.len() >= MAX_LIVE_PEERS {
            return Err(PeerDenial::Capacity);
        }
        state.package_receipt_revokers.push(Arc::downgrade(revoker));
        Ok(())
    }

    fn require_current_run(&self, run: &Pid1RunInstance) -> Result<(), PeerDenial> {
        if !Arc::ptr_eq(&self.owner, &run.owner) || !run.is_active() {
            return Err(PeerDenial::StaleRun);
        }
        self.require_current_run_generation(run.generation)
    }

    fn require_current_run_generation(&self, generation: NonZeroU64) -> Result<(), PeerDenial> {
        let state = self.state.lock().map_err(|_| PeerDenial::Unavailable)?;
        if state.current_run_generation == Some(generation)
            && state
                .current_run_active
                .as_ref()
                .is_some_and(|active| active.load(Ordering::SeqCst))
        {
            Ok(())
        } else {
            Err(PeerDenial::StaleRun)
        }
    }

    fn run_for_generation(&self, generation: NonZeroU64) -> Result<Pid1RunInstance, PeerDenial> {
        let state = self.state.lock().map_err(|_| PeerDenial::Unavailable)?;
        if state.current_run_generation != Some(generation) {
            return Err(PeerDenial::StaleRun);
        }
        let active = state
            .current_run_active
            .as_ref()
            .filter(|active| active.load(Ordering::SeqCst))
            .cloned()
            .ok_or(PeerDenial::StaleRun)?;
        let authority_domain = state
            .current_authority_domain
            .as_ref()
            .filter(|domain| {
                domain.generation() == generation
                    && state.current_run_authority_nonce == Some(domain.authority_nonce())
                    && Arc::ptr_eq(&domain.0.active, &active)
            })
            .cloned()
            .ok_or(PeerDenial::StaleRun)?;
        Ok(Pid1RunInstance {
            owner: Arc::clone(&self.owner),
            generation,
            authority_nonce: state
                .current_run_authority_nonce
                .ok_or(PeerDenial::StaleRun)?,
            active,
            authority_domain,
        })
    }

    fn live_required_entries(
        &self,
        run: &Pid1RunInstance,
        required: &RequiredServiceSet,
    ) -> Result<Vec<Entry<R>>, PeerDenial> {
        self.require_current_run(run)?;
        if required.routes.is_empty() {
            return Err(PeerDenial::EmptyRequiredServiceSet);
        }
        let entries = self
            .state
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .entries
            .clone();
        let mut selected = Vec::with_capacity(required.routes.len());
        for route in &required.routes {
            let mut matching = entries.iter().filter(|entry| {
                entry.run_generation == run.generation
                    && entry.route == *route
                    && entry.session.is_some()
                    && entry.system_service_owner.is_none()
                    && !entry
                        .initial_session_issuance_pending
                        .load(Ordering::SeqCst)
                    && entry.is_live()
            });
            let entry = matching.next().ok_or(PeerDenial::RequiredServiceMissing)?;
            if matching.next().is_some() {
                return Err(PeerDenial::ReadyUnavailable);
            }
            selected.push(entry.clone());
        }
        Ok(selected)
    }

    /// Register only a process started by PID 1 and already present in the
    /// Supervisor. The installed inventory, session lease, consent source,
    /// and route requirements are mandatory inputs from their existing owners.
    pub fn register_pid1_spawn<I, S, C, E, A, B>(
        &self,
        run: &Pid1RunInstance,
        spawn: Pid1SpawnProof<A::Record>,
        requirements: &TrustedServiceRequirements<'_>,
        endpoint: TrustedServiceEndpoint,
        inventory: &I,
        sessions: &S,
        consent: &C,
        endpoints: &E,
        supervisor: &A,
        readiness_boundary: &B,
    ) -> Result<RegistrationId, PeerDenial>
    where
        I: TrustedInventory,
        S: LiveSessionAuthority,
        C: ConsentSource,
        E: TrustedServiceEndpointAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
    {
        let exact_identity = spawn
            .task24_identity()
            .ok_or(PeerDenial::ForeignSpawnOwner)?;
        if exact_identity.process_id() != spawn.process.pid()
            || spawn.record.process_id() != spawn.process.pid()
        {
            return Err(PeerDenial::ForeignSpawnOwner);
        }
        let run_for_gate = run.clone();
        let run_for_registration = run.clone();
        readiness_boundary.with_owner_gate(&run_for_gate, None, move || {
            self.register_pid1_spawn_under_gate(
                &run_for_registration,
                spawn,
                requirements,
                endpoint,
                inventory,
                Some(sessions),
                consent,
                endpoints,
                supervisor,
            )
        })
    }

    /// Register a PID-1 spawn and its exact endpoint before any C1 Session
    /// exists. The resulting live entry is excluded from READY and normal
    /// service authorization until `accept_and_issue_initial_session_until`
    /// attaches a lease returned by the C1-backed authority.
    pub fn register_pid1_spawn_pre_session<I, C, E, A, B>(
        &self,
        run: &Pid1RunInstance,
        spawn: Pid1SpawnProof<A::Record>,
        requirements: &TrustedServiceRequirements<'_>,
        endpoint: TrustedServiceEndpoint,
        inventory: &I,
        consent: &C,
        endpoints: &E,
        supervisor: &A,
        readiness_boundary: &B,
    ) -> Result<RegistrationId, PeerDenial>
    where
        I: TrustedInventory,
        C: ConsentSource,
        E: TrustedServiceEndpointAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
    {
        let exact_identity = spawn
            .task24_identity()
            .ok_or(PeerDenial::ForeignSpawnOwner)?;
        if exact_identity.process_id() != spawn.process.pid()
            || spawn.record.process_id() != spawn.process.pid()
        {
            return Err(PeerDenial::ForeignSpawnOwner);
        }
        let run_for_gate = run.clone();
        let run_for_registration = run.clone();
        readiness_boundary.with_owner_gate(&run_for_gate, None, move || {
            self.register_pid1_spawn_under_gate(
                &run_for_registration,
                spawn,
                requirements,
                endpoint,
                inventory,
                None,
                consent,
                endpoints,
                supervisor,
            )
        })
    }

    fn register_pid1_spawn_under_gate<I, C, E, A>(
        &self,
        run: &Pid1RunInstance,
        spawn: Pid1SpawnProof<A::Record>,
        requirements: &TrustedServiceRequirements<'_>,
        endpoint: TrustedServiceEndpoint,
        inventory: &I,
        session_authority: Option<&dyn LiveSessionAuthority>,
        consent: &C,
        endpoints: &E,
        supervisor: &A,
    ) -> Result<RegistrationId, PeerDenial>
    where
        I: TrustedInventory,
        C: ConsentSource,
        E: TrustedServiceEndpointAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
    {
        self.require_current_run(run)?;
        if endpoint.route != requirements.route {
            return Err(PeerDenial::RouteMismatch);
        }
        if !endpoints.is_current(endpoint)? {
            return Err(PeerDenial::StaleEndpoint);
        }
        if !endpoints.fd_delegation_is_prevented(endpoint)? {
            return Err(PeerDenial::FdDelegationNotPrevented);
        }
        let process_id = spawn.process.pid();
        let task24_identity = spawn
            .task24_identity()
            .cloned()
            .ok_or(PeerDenial::ForeignSpawnOwner)?;
        let record = spawn.record;
        if process_id <= 1
            || record.process_id() != process_id
            || task24_identity.process_id() != process_id
        {
            return Err(PeerDenial::InvalidProcess);
        }
        if supervisor.resolve(process_id) != Some(record) {
            return Err(PeerDenial::NotLive);
        }

        let captured_manifest = inventory
            .installed_manifest(record.app_id())?
            .ok_or(PeerDenial::NotInstalled)?;
        if captured_manifest.app_id != record.app_id() {
            return Err(PeerDenial::ManifestMismatch);
        }
        let requested = [(record.app_id(), captured_manifest.inventory_generation)];
        let mut committed_registration = None;
        inventory.with_current_manifests(&requested, &mut |manifests| {
            let manifest = manifests.first().ok_or(PeerDenial::StaleManifest)?;
            if manifests.len() != 1
                || manifest.app_id != record.app_id()
                || manifest.inventory_generation != captured_manifest.inventory_generation
            {
                return Err(PeerDenial::StaleManifest);
            }
            validate_declared(manifest, requirements.capabilities)?;

            let session = if let Some(sessions) = session_authority {
                let session =
                    sessions.capture_live_unlocked(record.app_id(), record.app_session_id())?;
                if session.app_id != record.app_id()
                    || session.app_session_id != record.app_session_id()
                {
                    return Err(PeerDenial::SessionMismatch);
                }
                if !sessions.is_live_unlocked(session)? {
                    return Err(PeerDenial::SessionRevoked);
                }
                Some(session)
            } else {
                None
            };
            if requirements.consent_requirement == RouteConsentRequirement::Required
                && !consent.connected()?
            {
                return Err(PeerDenial::ConsentUnavailable);
            }

            let registration = self.reserve(
                run.generation,
                spawn.process,
                task24_identity.clone(),
                record,
                requirements.route,
                endpoint,
                requirements.consent_requirement,
                requirements
                    .capabilities
                    .iter()
                    .map(|capability| capability.to_vec())
                    .collect(),
                manifest.inventory_generation,
                session,
            )?;
            let result = supervisor.authorize_and_commit(record, requirements.capabilities, || {
                if requirements.consent_requirement == RouteConsentRequirement::Required
                    && !consent.connected()?
                {
                    return Err(PeerDenial::ConsentUnavailable);
                }
                if let (Some(sessions), Some(session)) = (session_authority, session) {
                    if !sessions.is_live_unlocked(session)? {
                        return Err(PeerDenial::SessionRevoked);
                    }
                }
                if !endpoints.is_current(endpoint)? {
                    return Err(PeerDenial::StaleEndpoint);
                }
                self.set_live(registration, spawn.process, record, &task24_identity)
            });
            if result.is_err() {
                self.remove_pending(registration);
            }
            result?;
            committed_registration = Some(registration);
            Ok(())
        })?;
        committed_registration.ok_or(PeerDenial::Unavailable)
    }

    /// Accept the first connection for a pre-session registration, hand the
    /// same stream and kernel peer to the C1-backed issuer after C2's private
    /// spawn/endpoint checks, then attach only the returned live C1 lease.
    /// The Session-bound interactive-peer path remains unavailable until this
    /// method completes successfully.
    #[allow(clippy::too_many_arguments)]
    pub fn accept_and_issue_initial_session_until<S, C, E, P, A, B>(
        &self,
        reader: &impl PeerCredentialSource,
        checkout: EndpointCheckout,
        registration: RegistrationId,
        sessions: &S,
        consent: &C,
        endpoints: &E,
        process_instances: &P,
        supervisor: &A,
        readiness_boundary: &B,
    ) -> Result<SessionLease, PeerDenial>
    where
        S: LiveSessionAuthority,
        C: ConsentSource,
        E: TrustedServiceEndpointAuthority,
        P: PeerProcessInstanceAuthority,
        A: SupervisorLaunchAuthority<Record = R>,
        B: Pid1ReadinessAtomicBoundary,
    {
        if Instant::now() >= checkout.expires_at {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        if checkout.endpoint_epoch != checkout.endpoint.endpoint_generation {
            return Err(PeerDenial::StaleEndpoint);
        }
        let entry = self.snapshot(registration)?;
        if !entry.is_live()
            || entry.session.is_some()
            || entry.system_service_owner.is_some()
            || entry.endpoint != checkout.endpoint
        {
            return Err(PeerDenial::StaleRegistration);
        }
        let run = self.run_for_generation(entry.run_generation)?;
        if !endpoints.is_current(entry.endpoint)?
            || !checkout
                .recipient
                .is_same_recipient(&endpoints.checkout_recipient(entry.endpoint)?)
        {
            return Err(PeerDenial::StaleEndpoint);
        }
        if !endpoints.fd_delegation_is_prevented(entry.endpoint)? {
            return Err(PeerDenial::FdDelegationNotPrevented);
        }
        if endpoints.local_socket_cookie(&checkout.stream)? != checkout.socket_cookie {
            return Err(PeerDenial::StaleSocket);
        }
        let observation = reader.read(&checkout.stream)?;
        if !observation.same_local_socket
            || endpoints.local_socket_cookie(&checkout.stream)? != checkout.socket_cookie
        {
            return Err(PeerDenial::StaleSocket);
        }
        let peer = observation
            .authenticated_peer
            .as_deref()
            .ok_or(PeerDenial::Unavailable)?;
        let peer_pid = u32::try_from(observation.pid).map_err(|_| PeerDenial::InvalidProcess)?;
        if peer.pid() != peer_pid
            || peer.uid() != observation.uid
            || peer.gid() != observation.gid
            || supervisor.resolve(peer_pid) != Some(entry.record)
            || !process_instances.confirms_same_live_process(
                &checkout.stream,
                peer_pid,
                observation.uid,
                observation.gid,
                entry.process,
            )?
            || !process_instances.confirms_spawned_instance_live(entry.process)?
        {
            return Err(PeerDenial::StalePeer);
        }
        if !consent.connected()? {
            return Err(PeerDenial::ConsentUnavailable);
        }

        let mut issuance = InitialSessionIssuanceGuard::acquire(Arc::clone(
            &entry.initial_session_issuance_pending,
        ))?;
        let admission = c2_accepted_peer_owner_check::validate_current_pre_session_owner(
            self,
            &run,
            registration,
            entry.process,
            entry.record,
            entry.endpoint,
            checkout.endpoint_epoch,
            &checkout.recipient,
            &entry.task24_identity,
            checkout.expires_at,
            peer,
            &checkout.stream,
            endpoints,
            supervisor,
            process_instances,
        )?;
        let login_request = initial_login_wire::receive_from_c2_accepted_stream(
            &checkout.stream,
            peer,
            admission.connection_proof(),
            checkout.expires_at,
        )?;
        c2_accepted_peer_owner_check::validate_current_pre_session_owner(
            self,
            &run,
            registration,
            entry.process,
            entry.record,
            entry.endpoint,
            checkout.endpoint_epoch,
            &checkout.recipient,
            &entry.task24_identity,
            checkout.expires_at,
            peer,
            &checkout.stream,
            endpoints,
            supervisor,
            process_instances,
        )?;
        if endpoints.local_socket_cookie(&checkout.stream)? != checkout.socket_cookie {
            return Err(PeerDenial::StaleSocket);
        }
        let mut issued_session = None;
        let mut complete_initial_session = |mut pending: PendingInitialSession<'_>| {
            let result = (|| {
                if issued_session.is_some() {
                    return Err(PeerDenial::Unavailable);
                }
                c2_accepted_peer_owner_check::validate_current_pre_session_owner(
                    self,
                    &run,
                    registration,
                    entry.process,
                    entry.record,
                    entry.endpoint,
                    checkout.endpoint_epoch,
                    &checkout.recipient,
                    &entry.task24_identity,
                    checkout.expires_at,
                    peer,
                    &checkout.stream,
                    endpoints,
                    supervisor,
                    process_instances,
                )?;
                if endpoints.local_socket_cookie(&checkout.stream)? != checkout.socket_cookie {
                    return Err(PeerDenial::StaleSocket);
                }
                if !consent.connected()? {
                    return Err(PeerDenial::ConsentUnavailable);
                }

                let validate_current_c2_owner = || {
                    self.require_current_peer_proof_owner(
                        &run,
                        registration,
                        entry.process,
                        entry.record,
                        entry.endpoint,
                        &entry.task24_identity,
                    )?;
                    if !endpoints.is_current(entry.endpoint)?
                        || !checkout
                            .recipient
                            .is_same_recipient(&endpoints.checkout_recipient(entry.endpoint)?)
                        || !endpoints.fd_delegation_is_prevented(entry.endpoint)?
                    {
                        return Err(PeerDenial::StaleEndpoint);
                    }
                    if supervisor.resolve(peer_pid) != Some(entry.record)
                        || !process_instances.confirms_same_live_process(
                            &checkout.stream,
                            peer_pid,
                            observation.uid,
                            observation.gid,
                            entry.process,
                        )?
                        || !process_instances.confirms_spawned_instance_live(entry.process)?
                        || process_instances.has_unresolved_cleanup()?
                    {
                        return Err(PeerDenial::StalePeer);
                    }
                    if endpoints.local_socket_cookie(&checkout.stream)? != checkout.socket_cookie {
                        return Err(PeerDenial::StaleSocket);
                    }
                    if !consent.connected()? {
                        return Err(PeerDenial::ConsentUnavailable);
                    }
                    Ok(())
                };
                let run_for_attach = run.clone();
                readiness_boundary.with_owner_gate(&run_for_attach, None, || {
                    validate_current_c2_owner()?;
                    let session = pending.issue_session_after_c2_owner_gate(
                        entry.record.app_id(),
                        entry.record.app_session_id(),
                    )?;
                    if session.app_id != entry.record.app_id()
                        || session.app_session_id != entry.record.app_session_id()
                    {
                        return Err(PeerDenial::SessionMismatch);
                    }
                    validate_current_c2_owner()?;
                    let mut state = self.state.lock().map_err(|_| PeerDenial::Unavailable)?;
                    let current = state
                        .entries
                        .iter_mut()
                        .find(|current| {
                            current.id == registration && current.process == entry.process
                        })
                        .ok_or(PeerDenial::StaleRegistration)?;
                    if !current.is_live()
                        || current.run_generation != entry.run_generation
                        || current.record != entry.record
                        || current.endpoint != entry.endpoint
                        || current.session.is_some()
                        || current.system_service_owner.is_some()
                        || !Arc::ptr_eq(
                            &current.initial_session_issuance_pending,
                            &entry.initial_session_issuance_pending,
                        )
                        || !current
                            .task24_identity
                            .same_instance(&entry.task24_identity)
                    {
                        return Err(PeerDenial::StaleRegistration);
                    }
                    current.session = Some(session);
                    issuance.mark_session_attached();
                    issued_session = Some(session);
                    pending.finish_after_both_owner_gates()?;
                    Ok(())
                })?;
                Ok(())
            })();
            if result.is_err() && pending.abort_now().is_err() {
                issuance.mark_rollback_failed();
                return Err(PeerDenial::RequestOutcomeUnknown);
            }
            if pending.rollback_failed() {
                issuance.mark_rollback_failed();
            }
            result
        };
        sessions.with_pending_initial_session_from_c2_owner_checked_peer(
            entry.record.app_id(),
            entry.record.app_session_id(),
            &checkout.stream,
            peer,
            admission.connection_proof(),
            &login_request,
            &mut complete_initial_session,
        )?;
        drop(login_request);
        drop(complete_initial_session);
        let session = issued_session.ok_or(PeerDenial::Unavailable)?;
        issuance.finish();
        Ok(session)
    }

    /// Build the only task-41 readiness event and consumer entry. The eventual
    /// `InitBackend::wait_event()` must map it with
    /// `Pid1ReadyEvent::with_current()` so event conversion runs while the
    /// authority gate and inventory lease remain held. `validate_current()` is
    /// diagnostic-only and is not an atomic check-and-map operation. It requires the complete
    /// canonical mandatory-service set, live PID 1 registrations, current
    /// endpoints, inventory/session/consent authorities, kernel-backed child
    /// liveness, Supervisor grants, and a protocol-handshake owner for every
    /// route. Fixture or caller-provided strings cannot construct the event.
    pub fn ready_event<'a, I, S, C, E, P, H, A, B>(
        &'a self,
        owner_lease: &'a Pid1RunOwnerLease,
        required: &'a RequiredServiceSet,
        inventory: &'a I,
        sessions: &'a S,
        consent: &'a C,
        endpoints: &'a E,
        process_instances: &'a P,
        handshakes: &'a H,
        supervisor: &'a A,
        readiness_boundary: &'a B,
    ) -> Result<Pid1ReadyEvent<'a>, PeerDenial>
    where
        I: TrustedInventory + 'a,
        S: LiveSessionAuthority + 'a,
        C: ConsentSource + 'a,
        E: TrustedServiceEndpointAuthority + 'a,
        P: PeerProcessInstanceAuthority + 'a,
        H: CompletedServiceHandshakeAuthority + 'a,
        A: SupervisorLaunchAuthority<Record = R> + 'a,
        B: Pid1ReadinessAtomicBoundary + 'a,
    {
        let run = owner_lease.run();
        self.require_current_run(run)?;
        let selected = self.live_required_entries(run, required)?;
        let services = selected
            .iter()
            .map(|entry| ReadyServiceInstance {
                registration: entry.id,
                process: entry.process,
                route: entry.route,
                endpoint: entry.endpoint,
            })
            .collect::<Vec<_>>();
        let expected_services = services.clone();
        let run_for_validation = run.clone();
        let with_current = move |consume: &mut dyn FnMut() -> Result<(), PeerDenial>| {
            readiness_boundary.with_owner_gate(&run_for_validation, Some(required), || {
                self.require_current_run(&run_for_validation)?;
                let current_entries = self.live_required_entries(&run_for_validation, required)?;
                let current_services = current_entries
                    .iter()
                    .map(|entry| ReadyServiceInstance {
                        registration: entry.id,
                        process: entry.process,
                        route: entry.route,
                        endpoint: entry.endpoint,
                    })
                    .collect::<Vec<_>>();
                if current_services != expected_services
                    || current_services.len() != required.routes.len()
                {
                    return Err(PeerDenial::StaleRegistration);
                }
                if process_instances.has_unresolved_cleanup()? {
                    return Err(PeerDenial::UnresolvedSpawnCleanup);
                }
                let requested = current_entries
                    .iter()
                    .map(|entry| (entry.record.app_id(), entry.inventory_generation))
                    .collect::<Vec<_>>();
                inventory.with_current_manifests(&requested, &mut |manifests| {
                    if manifests.len() != current_entries.len() {
                        return Err(PeerDenial::StaleManifest);
                    }
                    for (entry, manifest) in current_entries.iter().zip(manifests) {
                        let session = entry.live_session()?;
                        if manifest.app_id != entry.record.app_id()
                            || manifest.inventory_generation != entry.inventory_generation
                        {
                            return Err(PeerDenial::StaleManifest);
                        }
                        let capabilities = entry
                            .capabilities
                            .iter()
                            .map(Vec::as_slice)
                            .collect::<Vec<_>>();
                        validate_declared(manifest, &capabilities)?;
                        if session.app_id != entry.record.app_id()
                            || session.app_session_id != entry.record.app_session_id()
                        {
                            return Err(PeerDenial::SessionMismatch);
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
                        if !process_instances.confirms_spawned_instance_live(entry.process)? {
                            return Err(PeerDenial::StalePeer);
                        }
                        if !consent.connected()? {
                            return Err(PeerDenial::ConsentUnavailable);
                        }
                        if !sessions.is_live_unlocked(session)? {
                            return Err(PeerDenial::SessionRevoked);
                        }
                        if !handshakes.confirms_completed_handshake(
                            &run_for_validation,
                            entry.id,
                            entry.process,
                            entry.route,
                            entry.endpoint,
                        )? {
                            return Err(PeerDenial::HandshakeIncomplete);
                        }

                        supervisor.authorize_and_commit(entry.record, &capabilities, || {
                            self.require_current_run_generation(entry.run_generation)?;
                            let current = self.snapshot(entry.id)?;
                            if !current.is_live()
                                || current.run_generation != entry.run_generation
                                || current.process != entry.process
                                || current.record != entry.record
                                || current.route != entry.route
                                || current.endpoint != entry.endpoint
                                || current.capabilities != entry.capabilities
                                || current.inventory_generation != entry.inventory_generation
                                || current.session != entry.session
                                || current
                                    .initial_session_issuance_pending
                                    .load(Ordering::SeqCst)
                            {
                                return Err(PeerDenial::StaleRegistration);
                            }
                            if process_instances.has_unresolved_cleanup()? {
                                return Err(PeerDenial::UnresolvedSpawnCleanup);
                            }
                            if !endpoints.is_current(entry.endpoint)? {
                                return Err(PeerDenial::StaleEndpoint);
                            }
                            if !endpoints.fd_delegation_is_prevented(entry.endpoint)? {
                                return Err(PeerDenial::FdDelegationNotPrevented);
                            }
                            if !process_instances.confirms_spawned_instance_live(entry.process)? {
                                return Err(PeerDenial::StalePeer);
                            }
                            if !consent.connected()? {
                                return Err(PeerDenial::ConsentUnavailable);
                            }
                            if !sessions.is_live_unlocked(session)? {
                                return Err(PeerDenial::SessionRevoked);
                            }
                            if !handshakes.confirms_completed_handshake(
                                &run_for_validation,
                                entry.id,
                                entry.process,
                                entry.route,
                                entry.endpoint,
                            )? {
                                return Err(PeerDenial::HandshakeIncomplete);
                            }
                            Ok(())
                        })?;
                    }
                    consume()
                })
            })
        };
        let event = Pid1ReadyEvent {
            run: run.clone(),
            services,
            with_current: Box::new(with_current),
        };
        event.validate_current()?;
        Ok(event)
    }

    /// Recheck kernel peer identity, the current installed-manifest generation,
    /// session lease, consent connection, exact Supervisor record, and every
    /// canonical route capability at every checkout.
    pub(crate) fn authorize_peer<'a, I, S, C, E, P, A, B>(
        &'a self,
        reader: &impl PeerCredentialSource,
        checkout: EndpointCheckout,
        registration: RegistrationId,
        requirements: &TrustedServiceRequirements<'_>,
        inventory: &'a I,
        sessions: &'a S,
        consent: &'a C,
        endpoints: &'a E,
        process_instances: &'a P,
        supervisor: &'a A,
        readiness_boundary: &'a B,
    ) -> Result<AuthorizedPeer<'a, R>, PeerDenial>
    where
        I: TrustedInventory + 'a,
        S: LiveSessionAuthority + 'a,
        C: ConsentSource + 'a,
        E: TrustedServiceEndpointAuthority + 'a,
        P: PeerProcessInstanceAuthority + 'a,
        A: SupervisorLaunchAuthority<Record = R> + 'a,
        B: Pid1ReadinessAtomicBoundary + 'a,
    {
        if Instant::now() >= checkout.expires_at {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        let checkout_expires_at = checkout.expires_at;
        if checkout.endpoint_epoch != checkout.endpoint.endpoint_generation {
            return Err(PeerDenial::StaleEndpoint);
        }
        let entry = self.snapshot(registration)?;
        let session = entry.live_session()?;
        let run = self.run_for_generation(entry.run_generation)?;
        let run_for_gate = run.clone();
        let run_for_observation = run.clone();
        if !endpoints.is_current(checkout.endpoint)? {
            return Err(PeerDenial::StaleEndpoint);
        }
        let current_recipient = endpoints.checkout_recipient(checkout.endpoint)?;
        if !checkout.recipient.is_same_recipient(&current_recipient) {
            return Err(PeerDenial::StaleEndpoint);
        }
        if !endpoints.fd_delegation_is_prevented(checkout.endpoint)? {
            return Err(PeerDenial::FdDelegationNotPrevented);
        }
        if endpoints.local_socket_cookie(&checkout.stream)? != checkout.socket_cookie {
            return Err(PeerDenial::StaleSocket);
        }
        let observation = reader.read(&checkout.stream)?;
        if endpoints.local_socket_cookie(&checkout.stream)? != checkout.socket_cookie {
            return Err(PeerDenial::StaleSocket);
        }
        let peer_pid = u32::try_from(observation.pid).map_err(|_| PeerDenial::InvalidProcess)?;
        if entry.record.process_id() != peer_pid
            || entry.process.pid() != peer_pid
            || supervisor.resolve(peer_pid) != Some(entry.record)
        {
            return Err(PeerDenial::NotLive);
        }
        if !process_instances.confirms_same_live_process(
            &checkout.stream,
            peer_pid,
            observation.uid,
            observation.gid,
            entry.process,
        )? || !process_instances.confirms_spawned_instance_live(entry.process)?
        {
            return Err(PeerDenial::StalePeer);
        }
        if session.app_id != entry.record.app_id()
            || session.app_session_id != entry.record.app_session_id()
        {
            return Err(PeerDenial::SessionMismatch);
        }
        let mut final_owner_selector = None;
        let final_owner_run = run.clone();
        let final_owner_process = entry.process;
        let final_owner_record = entry.record;
        let final_owner_session = session;
        let final_owner_task24_identity = entry.task24_identity.clone();
        let final_owner_endpoint = checkout.endpoint;
        let final_owner_endpoint_epoch = checkout.endpoint_epoch;
        let final_owner_recipient = checkout.recipient.clone();
        let final_owner_peer_uid = observation.uid;
        let final_owner_peer_gid = observation.gid;
        let mut pending_binding = match observation.authenticated_peer.as_deref() {
            Some(observed_peer) => {
                if observed_peer.pid() != peer_pid
                    || observed_peer.uid() != observation.uid
                    || observed_peer.gid() != observation.gid
                {
                    return Err(PeerDenial::StalePeer);
                }
                let accepted_peer_handle = checkout
                    .accepted_peer_handle
                    .as_ref()
                    .ok_or(PeerDenial::Unavailable)?;
                let mut postcheck_selector = None;
                let validate_c2_owner = |peer: &AuthenticatedInteractivePeer<'_>| {
                    if !session.matches_authenticated_peer_selector(peer.selector()) {
                        return Err(PeerDenial::SessionMismatch);
                    }
                    c2_accepted_peer_owner_check::validate_current_owner(
                        self,
                        &run,
                        registration,
                        entry.process,
                        entry.record,
                        checkout.endpoint,
                        checkout.endpoint_epoch,
                        &checkout.recipient,
                        &entry.task24_identity,
                        checkout.expires_at,
                        peer_pid,
                        observation.uid,
                        observation.gid,
                        peer,
                        &checkout.stream,
                        endpoints,
                        supervisor,
                        process_instances,
                    )
                    .map(|_| ())
                };
                let bound = accepted_peer_handle
                    .with_live_interactive_peer(
                        &checkout.stream,
                        registration.0,
                        session.app_id,
                        session.app_session_id,
                        session.user_session_generation,
                        |interactive_peer| {
                            let kernel_peer = interactive_peer.kernel_peer();
                            if kernel_peer.connection_id() != observed_peer.connection_id()
                                || kernel_peer.pid() != peer_pid
                                || kernel_peer.uid() != observation.uid
                                || kernel_peer.gid() != observation.gid
                            {
                                return Err(PeerDenial::StalePeer);
                            }
                            let owner_admission =
                                c2_accepted_peer_owner_check::validate_current_owner(
                                    self,
                                    &run,
                                    registration,
                                    entry.process,
                                    entry.record,
                                    checkout.endpoint,
                                    checkout.endpoint_epoch,
                                    &checkout.recipient,
                                    &entry.task24_identity,
                                    checkout.expires_at,
                                    peer_pid,
                                    observation.uid,
                                    observation.gid,
                                    &interactive_peer,
                                    &checkout.stream,
                                    endpoints,
                                    supervisor,
                                    process_instances,
                                )?;
                            if !owner_admission.matches_peer(&interactive_peer) {
                                return Err(PeerDenial::StalePeer);
                            }
                            let app_admission = owner_admission
                                .bind_app_session_selector(&interactive_peer, session)?;
                            if !app_admission.matches_peer_and_lease(&interactive_peer, session) {
                                return Err(PeerDenial::SessionMismatch);
                            }
                            let pending = sessions.begin_authenticated_peer_binding(
                                &interactive_peer,
                                &app_admission,
                                app_admission.accepted_connection_proof(),
                            )?;
                            validate_c2_owner(&interactive_peer)?;
                            postcheck_selector = Some(interactive_peer.selector().clone());
                            Ok(pending)
                        },
                    )
                    .map_err(map_peer_authentication_error)?;
                let pending = bound?;
                let selector = postcheck_selector.ok_or(PeerDenial::Unavailable)?;
                if !session.matches_authenticated_peer_selector(&selector) {
                    return Err(PeerDenial::SessionMismatch);
                }
                c2_accepted_peer_owner_check::validate_current_owner_selector(
                    self,
                    &run,
                    registration,
                    entry.process,
                    entry.record,
                    checkout.endpoint,
                    checkout.endpoint_epoch,
                    &checkout.recipient,
                    &entry.task24_identity,
                    checkout.expires_at,
                    peer_pid,
                    observation.uid,
                    observation.gid,
                    &selector,
                    None,
                    endpoints,
                    supervisor,
                    process_instances,
                )?;
                final_owner_selector = Some(selector);
                Some(pending)
            }
            None => {
                #[cfg(not(test))]
                return Err(PeerDenial::Unavailable);
                #[cfg(test)]
                None
            }
        };
        let peer_session = pending_binding
            .as_ref()
            .map(PendingBoundPeerSession::provisional_gate);
        let peer_session_for_owner_gate = peer_session.clone();
        let pending_binding_for_owner_gate = &mut pending_binding;
        let authorized_peer = with_authenticated_peer_session_effect(
            sessions,
            peer_session.as_deref(),
            session,
            move || {
                readiness_boundary.with_owner_gate(&run_for_gate, None, move || {
                    if requirements.route != checkout.endpoint.route {
                        return Err(PeerDenial::RouteMismatch);
                    }
                    if !endpoints.is_current(checkout.endpoint)? {
                        return Err(PeerDenial::StaleEndpoint);
                    }
                    let current_recipient = endpoints.checkout_recipient(checkout.endpoint)?;
                    if !checkout.recipient.is_same_recipient(&current_recipient) {
                        return Err(PeerDenial::StaleEndpoint);
                    }
                    if !endpoints.fd_delegation_is_prevented(checkout.endpoint)? {
                        return Err(PeerDenial::FdDelegationNotPrevented);
                    }
                    if endpoints.local_socket_cookie(&checkout.stream)? != checkout.socket_cookie {
                        return Err(PeerDenial::StaleSocket);
                    }
                    let authorized_peer = self.authorize_observation(
                        &run_for_observation,
                        observation,
                        peer_session_for_owner_gate,
                        checkout,
                        registration,
                        requirements,
                        inventory,
                        sessions,
                        consent,
                        endpoints,
                        process_instances,
                        supervisor,
                        readiness_boundary,
                    )?;
                    if let Some(pending) = pending_binding_for_owner_gate.as_mut() {
                        if Instant::now() >= checkout_expires_at {
                            return Err(PeerDenial::RequestDeadlineExpired);
                        }
                        pending.activate_after_c2_owner_gate()?;
                    }
                    Ok(authorized_peer)
                })
            },
        )?;
        if Instant::now() >= checkout_expires_at {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        match (pending_binding, peer_session) {
            (Some(pending), Some(provisional_gate)) => {
                if !Arc::ptr_eq(&pending.provisional_gate(), &provisional_gate) {
                    return Err(PeerDenial::Unavailable);
                }
                let selector = final_owner_selector.ok_or(PeerDenial::Unavailable)?;
                let final_owner_run_for_gate = final_owner_run.clone();
                readiness_boundary.with_owner_gate(&final_owner_run_for_gate, None, move || {
                    if Instant::now() >= checkout_expires_at {
                        return Err(PeerDenial::RequestDeadlineExpired);
                    }
                    if !final_owner_session.matches_authenticated_peer_selector(&selector) {
                        return Err(PeerDenial::SessionMismatch);
                    }
                    c2_accepted_peer_owner_check::validate_current_owner_selector(
                        self,
                        &final_owner_run,
                        registration,
                        final_owner_process,
                        final_owner_record,
                        final_owner_endpoint,
                        final_owner_endpoint_epoch,
                        &final_owner_recipient,
                        &final_owner_task24_identity,
                        checkout_expires_at,
                        peer_pid,
                        final_owner_peer_uid,
                        final_owner_peer_gid,
                        &selector,
                        None,
                        endpoints,
                        supervisor,
                        process_instances,
                    )?;
                    pending.commit_after_session_gate()
                })?;
            }
            (None, None) => {}
            _ => return Err(PeerDenial::Unavailable),
        }
        Ok(authorized_peer)
    }

    /// Accept from the listener registered for this live endpoint, then pass
    /// the exact owner-created checkout into peer authorization. Callers never
    /// provide a stream or a detached task39 credential snapshot. The returned
    /// peer owns the same accepted connection used for SO_COOKIE and pidfd
    /// matching; all later authority checks still use PID1/session/consent and
    /// owner transaction state. This endpoint-owner path cannot consume task-39's
    /// opaque `AcceptedPeerConnection`: that type exposes only its credential
    /// snapshot and request frame to a dispatcher, not the accepted stream or FD
    /// needed for this checkout and same-socket SO_PEERPIDFD match. Do not call
    /// this from a task-39 dispatcher for a different accept; use the registered
    /// endpoint owner as the sole acceptor for this candidate path.
    pub fn accept_and_authorize_peer_until<'a, I, S, C, E, P, A, B>(
        &'a self,
        reader: &impl PeerCredentialSource,
        endpoint: TrustedServiceEndpoint,
        registration: RegistrationId,
        requirements: &TrustedServiceRequirements<'_>,
        deadline: Instant,
        inventory: &'a I,
        sessions: &'a S,
        consent: &'a C,
        endpoints: &'a E,
        process_instances: &'a P,
        supervisor: &'a A,
        readiness_boundary: &'a B,
    ) -> Result<AuthorizedPeer<'a, R>, PeerDenial>
    where
        I: TrustedInventory + 'a,
        S: LiveSessionAuthority + 'a,
        C: ConsentSource + 'a,
        E: TrustedServiceEndpointAuthority + 'a,
        P: PeerProcessInstanceAuthority + 'a,
        A: SupervisorLaunchAuthority<Record = R> + 'a,
        B: Pid1ReadinessAtomicBoundary + 'a,
    {
        let entry = self.snapshot(registration)?;
        self.require_current_run_generation(entry.run_generation)?;
        if requirements.route != entry.route {
            return Err(PeerDenial::RouteMismatch);
        }
        if endpoint != entry.endpoint || endpoint.route != entry.route {
            return Err(PeerDenial::StaleEndpoint);
        }
        if Instant::now() >= deadline {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        if !endpoints.is_current(endpoint)? {
            return Err(PeerDenial::StaleEndpoint);
        }
        let recipient = endpoints.checkout_recipient(endpoint)?;
        if !endpoints.fd_delegation_is_prevented(endpoint)? {
            return Err(PeerDenial::FdDelegationNotPrevented);
        }
        let checkout = endpoints.accept_checkout_until(endpoint, deadline)?;
        if Instant::now() >= deadline {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        if checkout.endpoint != endpoint
            || checkout.endpoint_epoch != endpoint.endpoint_generation
            || checkout.expires_at > deadline
            || Instant::now() >= checkout.expires_at
            || !checkout.recipient.is_same_recipient(&recipient)
        {
            if Instant::now() >= checkout.expires_at {
                return Err(PeerDenial::RequestDeadlineExpired);
            }
            return Err(PeerDenial::StaleEndpoint);
        }
        let peer = self.authorize_peer(
            reader,
            checkout,
            registration,
            requirements,
            inventory,
            sessions,
            consent,
            endpoints,
            process_instances,
            supervisor,
            readiness_boundary,
        )?;
        if Instant::now() >= deadline {
            drop(peer);
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        Ok(peer)
    }

    fn authorize_observation<'a, I, S, C, E, P, A, B>(
        &'a self,
        run: &Pid1RunInstance,
        observation: KernelPeerObservation,
        peer_session: Option<Arc<dyn BoundPeerSessionEffectGate>>,
        checkout: EndpointCheckout,
        registration: RegistrationId,
        requirements: &TrustedServiceRequirements<'_>,
        inventory: &'a I,
        sessions: &'a S,
        consent: &'a C,
        endpoints: &'a E,
        process_instances: &'a P,
        supervisor: &'a A,
        readiness_boundary: &'a B,
    ) -> Result<AuthorizedPeer<'a, R>, PeerDenial>
    where
        I: TrustedInventory + 'a,
        S: LiveSessionAuthority + 'a,
        C: ConsentSource + 'a,
        E: TrustedServiceEndpointAuthority + 'a,
        P: PeerProcessInstanceAuthority + 'a,
        A: SupervisorLaunchAuthority<Record = R> + 'a,
        B: Pid1ReadinessAtomicBoundary + 'a,
    {
        let EndpointCheckout {
            endpoint,
            recipient: checkout_recipient,
            endpoint_epoch,
            expires_at,
            socket_cookie,
            stream,
            accepted_peer_handle: _,
        } = checkout;
        let peer_proof = observation.authenticated_peer.clone();
        if Instant::now() >= expires_at {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        if endpoint_epoch != endpoint.endpoint_generation {
            return Err(PeerDenial::StaleEndpoint);
        }
        let entry = self.snapshot(registration)?;
        let session = entry.live_session()?;
        self.require_current_run_generation(entry.run_generation)?;
        if requirements.route != entry.route || endpoint.route != entry.route {
            return Err(PeerDenial::RouteMismatch);
        }
        if requirements.consent_requirement != entry.consent_requirement {
            return Err(PeerDenial::ConsentPolicyMismatch);
        }
        if !capabilities_match(&entry.capabilities, requirements.capabilities) {
            return Err(PeerDenial::CapabilitySetMismatch);
        }
        if endpoint != entry.endpoint || !endpoints.is_current(entry.endpoint)? {
            return Err(PeerDenial::StaleEndpoint);
        }
        if !checkout_recipient.is_same_recipient(&endpoints.checkout_recipient(entry.endpoint)?) {
            return Err(PeerDenial::StaleEndpoint);
        }
        if endpoints.local_socket_cookie(&stream)? != socket_cookie {
            return Err(PeerDenial::StaleSocket);
        }
        if let Some(peer) = peer_proof.as_deref() {
            peer.revalidate(&stream)
                .map_err(map_peer_authentication_error)?;
        }
        if !entry.is_live() || !observation.same_local_socket {
            return Err(PeerDenial::StalePeer);
        }
        let peer_pid = u32::try_from(observation.pid).map_err(|_| PeerDenial::InvalidProcess)?;
        if entry.record.process_id() != peer_pid || entry.process.pid() != peer_pid {
            return Err(PeerDenial::NotLive);
        }
        if !process_instances.confirms_same_live_process(
            &stream,
            peer_pid,
            observation.uid,
            observation.gid,
            entry.process,
        )? {
            return Err(PeerDenial::StalePeer);
        }
        if !process_instances.confirms_spawned_instance_live(entry.process)? {
            return Err(PeerDenial::StalePeer);
        }
        if supervisor.resolve(peer_pid) != Some(entry.record) {
            return Err(PeerDenial::NotLive);
        }
        if session.app_id != entry.record.app_id()
            || session.app_session_id != entry.record.app_session_id()
        {
            return Err(PeerDenial::SessionMismatch);
        }
        let requested = [(entry.record.app_id(), entry.inventory_generation)];
        inventory.with_current_manifests(&requested, &mut |manifests| {
            let manifest = manifests.first().ok_or(PeerDenial::StaleManifest)?;
            if manifests.len() != 1
                || manifest.app_id != entry.record.app_id()
                || manifest.inventory_generation != entry.inventory_generation
            {
                return Err(PeerDenial::StaleManifest);
            }
            validate_declared(manifest, requirements.capabilities)?;
            if !consent.connected()? {
                return Err(PeerDenial::ConsentUnavailable);
            }
            supervisor.authorize_and_commit(entry.record, requirements.capabilities, || {
                if !process_instances.confirms_same_live_process(
                    &stream,
                    peer_pid,
                    observation.uid,
                    observation.gid,
                    entry.process,
                )? {
                    return Err(PeerDenial::StalePeer);
                }
                if !process_instances.confirms_spawned_instance_live(entry.process)? {
                    return Err(PeerDenial::StalePeer);
                }
                if process_instances.has_unresolved_cleanup()? {
                    return Err(PeerDenial::UnresolvedSpawnCleanup);
                }
                if !consent.connected()? {
                    return Err(PeerDenial::ConsentUnavailable);
                }
                if !endpoints.is_current(entry.endpoint)? {
                    return Err(PeerDenial::StaleEndpoint);
                }
                if !checkout_recipient
                    .is_same_recipient(&endpoints.checkout_recipient(entry.endpoint)?)
                {
                    return Err(PeerDenial::StaleEndpoint);
                }
                if !endpoints.fd_delegation_is_prevented(entry.endpoint)? {
                    return Err(PeerDenial::FdDelegationNotPrevented);
                }
                if endpoints.local_socket_cookie(&stream)? != socket_cookie {
                    return Err(PeerDenial::StaleSocket);
                }
                let current = self.snapshot(registration)?;
                if !current.is_live()
                    || current.run_generation != entry.run_generation
                    || current.process != entry.process
                    || !current
                        .task24_identity
                        .same_instance(&entry.task24_identity)
                    || current.record != entry.record
                {
                    return Err(PeerDenial::StaleRegistration);
                }
                Ok(())
            })
        })?;

        let connection_epoch = next_authorized_connection_epoch()?;
        let connection_stale = Arc::new(AtomicBool::new(false));
        let connection = AuthorizedConnection::from_accepted_stream(
            stream,
            Arc::clone(&connection_stale),
            peer_proof.as_deref(),
        )?;
        let capability_bytes = requirements
            .capabilities
            .iter()
            .map(|capability| capability.to_vec())
            .collect::<Vec<_>>();
        let revalidate_entry = entry.clone();
        let run_for_validation = run.clone();
        let revalidate_inventory = [(entry.record.app_id(), entry.inventory_generation)];
        let revalidate_endpoint_recipient = checkout_recipient.clone();
        let revalidate_endpoint_epoch = endpoint_epoch;
        let revalidate_state: Arc<dyn Fn() -> Result<(), PeerDenial> + Send + Sync + 'a> =
            Arc::new(move || {
                self.require_current_run(&run_for_validation)?;
                let capabilities = capability_bytes
                    .iter()
                    .map(Vec::as_slice)
                    .collect::<Vec<_>>();
                if supervisor.resolve(peer_pid) != Some(revalidate_entry.record) {
                    return Err(PeerDenial::NotLive);
                }
                inventory.with_current_manifests(&revalidate_inventory, &mut |manifests| {
                    let manifest = manifests.first().ok_or(PeerDenial::StaleManifest)?;
                    if manifests.len() != 1
                        || manifest.app_id != revalidate_entry.record.app_id()
                        || manifest.inventory_generation != revalidate_entry.inventory_generation
                    {
                        return Err(PeerDenial::StaleManifest);
                    }
                    validate_declared(manifest, &capabilities)?;
                    supervisor.authorize_and_commit(revalidate_entry.record, &capabilities, || {
                        let current = self.snapshot(registration)?;
                        if !current.is_live()
                            || current.run_generation != revalidate_entry.run_generation
                            || current.process != revalidate_entry.process
                            || !current
                                .task24_identity
                                .same_instance(&revalidate_entry.task24_identity)
                            || current.record != revalidate_entry.record
                            || current.route != revalidate_entry.route
                            || current.endpoint != revalidate_entry.endpoint
                            || current.inventory_generation != revalidate_entry.inventory_generation
                            || current.session != revalidate_entry.session
                            || revalidate_endpoint_epoch
                                != revalidate_entry.endpoint.endpoint_generation
                        {
                            return Err(PeerDenial::StaleRegistration);
                        }
                        if !process_instances
                            .confirms_spawned_instance_live(revalidate_entry.process)?
                        {
                            return Err(PeerDenial::StalePeer);
                        }
                        if process_instances.has_unresolved_cleanup()? {
                            return Err(PeerDenial::UnresolvedSpawnCleanup);
                        }
                        if !consent.connected()? {
                            return Err(PeerDenial::ConsentUnavailable);
                        }
                        if !endpoints.is_current(revalidate_entry.endpoint)? {
                            return Err(PeerDenial::StaleEndpoint);
                        }
                        if !revalidate_endpoint_recipient.is_same_recipient(
                            &endpoints.checkout_recipient(revalidate_entry.endpoint)?,
                        ) {
                            return Err(PeerDenial::StaleEndpoint);
                        }
                        if !endpoints.fd_delegation_is_prevented(revalidate_entry.endpoint)? {
                            return Err(PeerDenial::FdDelegationNotPrevented);
                        }
                        Ok(())
                    })
                })
            });
        let revalidate_under_gate = Arc::clone(&revalidate_state);
        let session_lease = session;
        let peer_session_for_effect_gate = peer_session.clone();
        let session_effect_gate: Arc<SessionEffectGate<'a>> =
            Arc::new(move |effect: &mut dyn FnMut() -> Result<(), PeerDenial>| {
                with_authenticated_peer_session_effect(
                    sessions,
                    peer_session_for_effect_gate.as_deref(),
                    session_lease,
                    || effect(),
                )
            });
        let run_for_io_gate = run.clone();
        let state_for_io = Arc::clone(&revalidate_state);
        let session_effect_gate_for_io = Arc::clone(&session_effect_gate);
        let revalidate = move || {
            run_session_effect(session_effect_gate_for_io.as_ref(), || {
                readiness_boundary.with_owner_gate(&run_for_io_gate, None, || state_for_io())
            })
        };
        #[cfg(target_os = "linux")]
        let revalidate_connection: Box<AuthenticatedConnectionRevalidator<'a>> = {
            let peer_for_connection = peer_proof.clone();
            let state_for_connection = Arc::clone(&revalidate_state);
            let run_for_connection = run.clone();
            let session_gate_for_connection = Arc::clone(&session_effect_gate);
            let expected_pid = observation.pid;
            let expected_uid = observation.uid;
            let expected_gid = observation.gid;
            Box::new(move |fd, credentials| {
                run_session_effect(session_gate_for_connection.as_ref(), || {
                    readiness_boundary.with_owner_gate(&run_for_connection, None, || {
                        if credentials.pid() as i32 != expected_pid
                            || credentials.uid() != expected_uid
                            || credentials.gid() != expected_gid
                        {
                            return Err(PeerDenial::StalePeer);
                        }
                        if let Some(peer) = peer_for_connection.as_deref() {
                            peer.require_same_connection_fd(fd)
                                .map_err(map_peer_authentication_error)?;
                        } else {
                            #[cfg(not(test))]
                            return Err(PeerDenial::Unavailable);
                        }
                        state_for_connection()
                    })
                })
            })
        };
        Ok(AuthorizedPeer {
            run: run.clone(),
            connection_epoch,
            registration,
            process: entry.process,
            task24_identity: entry.task24_identity.clone(),
            record: entry.record,
            route: entry.route,
            endpoint: entry.endpoint,
            endpoint_epoch,
            endpoint_recipient: checkout_recipient,
            inventory_generation: entry.inventory_generation,
            session_generation: session.user_session_generation,
            capabilities: entry.capabilities.clone(),
            consent_requirement: entry.consent_requirement,
            authority_domain: readiness_boundary.authority_domain().clone(),
            request_ledger: Arc::clone(&self.request_ledger),
            connection_stale,
            request_frame: Arc::new(Mutex::new(None)),
            request_decoder_claimed: false,
            session_effect_gate,
            #[cfg(target_os = "linux")]
            observation,
            connection,
            #[cfg(target_os = "linux")]
            revalidate_connection,
            revalidate: Box::new(revalidate),
            revalidate_under_gate: Box::new(move || revalidate_under_gate()),
        })
    }

    /// Call only after the existing Supervisor exit/revoke operation succeeds.
    /// An old registration token can never remove a replacement after PID reuse.
    pub fn retire_after_supervisor_revoke<B: Pid1ReadinessAtomicBoundary>(
        &self,
        run: &Pid1RunInstance,
        process: ProcessInstanceKey,
        registration: RegistrationId,
        readiness_boundary: &B,
    ) -> Result<bool, PeerDenial> {
        let run_for_gate = run.clone();
        let run_for_retire = run.clone();
        readiness_boundary.with_owner_gate(&run_for_gate, None, move || {
            self.require_current_run(&run_for_retire)?;
            let revokers = self.package_receipt_revokers_for_run(&run_for_retire)?;
            {
                let mut state = self.state.lock().map_err(|_| PeerDenial::Unavailable)?;
                let Some(entry) = state
                    .entries
                    .iter_mut()
                    .find(|entry| entry.id == registration && entry.process == process)
                else {
                    return Ok(false);
                };
                // Supervisor revoke has already succeeded. Deny any checkout
                // while receipt cleanup runs; on failure the caller can retry
                // retirement, but the entry cannot become live again.
                mark_endpoint_retirement_pending(&entry.retirement_pending);
            }
            for revoker in revokers {
                revoker.revoke_registration_under_gate(&run_for_retire, registration, process)?;
            }
            let mut state = self.state.lock().map_err(|_| PeerDenial::Unavailable)?;
            if let Some(index) = state
                .entries
                .iter()
                .position(|entry| entry.id == registration && entry.process == process)
            {
                state.entries.remove(index);
            }
            Ok(true)
        })
    }

    fn retirement_latch(
        &self,
        registration: RegistrationId,
        process: ProcessInstanceKey,
    ) -> Result<Arc<AtomicBool>, PeerDenial> {
        self.state
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .entries
            .iter()
            .find(|entry| entry.id == registration && entry.process == process)
            .map(|entry| Arc::clone(&entry.retirement_pending))
            .ok_or(PeerDenial::StaleRegistration)
    }

    fn require_current_peer_proof_owner(
        &self,
        run: &Pid1RunInstance,
        registration: RegistrationId,
        process: ProcessInstanceKey,
        record: R,
        endpoint: TrustedServiceEndpoint,
        task24_identity: &SpawnedProcessIdentity,
    ) -> Result<(), PeerDenial> {
        if !Arc::ptr_eq(&self.owner, &run.owner) || !run.is_active() {
            return Err(PeerDenial::StaleRun);
        }
        let state = self.state.try_lock().map_err(|_| PeerDenial::Unavailable)?;
        let current_run = state.current_run_generation == Some(run.generation)
            && state.current_run_authority_nonce == Some(run.authority_nonce)
            && state.current_run_active.as_ref().is_some_and(|active| {
                Arc::ptr_eq(active, &run.active) && active.load(Ordering::SeqCst)
            });
        if !current_run {
            return Err(PeerDenial::StaleRun);
        }
        let entry = state
            .entries
            .iter()
            .find(|entry| entry.id == registration)
            .ok_or(PeerDenial::StaleRegistration)?;
        if !entry.is_live()
            || entry.state != EntryState::Live
            || entry.run_generation != run.generation
            || entry.process != process
            || entry.record != record
            || entry.endpoint != endpoint
            || !entry.task24_identity.same_instance(task24_identity)
        {
            return Err(PeerDenial::StaleRegistration);
        }
        Ok(())
    }

    fn reserve(
        &self,
        run_generation: NonZeroU64,
        process: ProcessInstanceKey,
        task24_identity: SpawnedProcessIdentity,
        record: R,
        route: TrustedRouteKey,
        endpoint: TrustedServiceEndpoint,
        consent_requirement: RouteConsentRequirement,
        capabilities: Vec<Vec<u8>>,
        inventory_generation: NonZeroU64,
        session: Option<SessionLease>,
    ) -> Result<RegistrationId, PeerDenial> {
        let mut state = self.state.lock().map_err(|_| PeerDenial::Unavailable)?;
        if state
            .entries
            .iter()
            .any(|entry| entry.process.pid() == process.pid())
        {
            return Err(PeerDenial::DuplicateProcess);
        }
        if state.entries.len() == MAX_LIVE_PEERS {
            return Err(PeerDenial::Capacity);
        }
        let generation = state.next_generation;
        state.next_generation = generation
            .checked_add(1)
            .ok_or(PeerDenial::GenerationExhausted)?;
        let id =
            RegistrationId(NonZeroU64::new(generation).ok_or(PeerDenial::GenerationExhausted)?);
        state.entries.push(Entry {
            id,
            run_generation,
            process,
            task24_identity,
            record,
            route,
            endpoint,
            capabilities,
            consent_requirement,
            inventory_generation,
            session,
            system_service_owner: None,
            initial_session_issuance_pending: Arc::new(AtomicBool::new(false)),
            state: EntryState::Pending,
            retirement_pending: Arc::new(AtomicBool::new(false)),
        });
        Ok(id)
    }

    fn set_live(
        &self,
        registration: RegistrationId,
        process: ProcessInstanceKey,
        record: R,
        task24_identity: &SpawnedProcessIdentity,
    ) -> Result<(), PeerDenial> {
        let mut state = self.state.lock().map_err(|_| PeerDenial::Unavailable)?;
        let entry = state
            .entries
            .iter_mut()
            .find(|entry| entry.id == registration && entry.process == process)
            .ok_or(PeerDenial::StaleRegistration)?;
        if entry.record != record
            || !entry.task24_identity.same_instance(task24_identity)
            || entry.state != EntryState::Pending
            || entry.retirement_pending.load(Ordering::SeqCst)
        {
            return Err(PeerDenial::StaleRegistration);
        }
        entry.state = EntryState::Live;
        Ok(())
    }

    fn remove_pending(&self, registration: RegistrationId) {
        if let Ok(mut state) = self.state.lock() {
            if let Some(index) = state
                .entries
                .iter()
                .position(|entry| entry.id == registration && entry.state == EntryState::Pending)
            {
                state.entries.remove(index);
            }
        }
    }

    fn snapshot(&self, registration: RegistrationId) -> Result<Entry<R>, PeerDenial> {
        self.state
            .lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .entries
            .iter()
            .find(|entry| entry.id == registration)
            .cloned()
            .ok_or(PeerDenial::StaleRegistration)
    }
}

trait ReadWrite: Read + Write {
    fn recheck_before_side_effect(&mut self) -> io::Result<()>;
}

#[cfg(target_os = "linux")]
impl ReadWrite for PeerConnectionIo<'_> {
    fn recheck_before_side_effect(&mut self) -> io::Result<()> {
        PeerConnectionIo::recheck_before_side_effect(self)
    }
}

/// Read/write-only view of an authorized peer stream. It deliberately does
/// not expose `UnixStream`, raw descriptors, or `try_clone()`.
///
/// ```compile_fail
/// use nagi_pid1_live_peer_supplier::AuthorizedPeer;
/// fn retain_fd<R: nagi_pid1_live_peer_supplier::LaunchRecordView>(peer: &mut AuthorizedPeer<'_, R>) {
///     let _ = peer.with_live_io(|io| io.try_clone());
/// }
/// ```
pub struct AuthorizedStreamIo<'a> {
    inner: &'a mut dyn ReadWrite,
}

impl Read for AuthorizedStreamIo<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.inner.read(buffer)
    }
}

impl Write for AuthorizedStreamIo<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.inner.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl AuthorizedStreamIo<'_> {
    /// Recheck live authority before a logical service effect. The check is
    /// not atomic with the effect; the service commit must synchronize with
    /// the same authority owner.
    pub fn recheck_before_side_effect(&mut self) -> io::Result<()> {
        self.inner.recheck_before_side_effect()
    }
}

enum AuthorizedConnection {
    #[cfg(target_os = "linux")]
    Task39(OwnedPeerConnection),
    #[cfg(not(target_os = "linux"))]
    HostFixture(HostFixtureConnection),
}

#[cfg(not(target_os = "linux"))]
struct HostFixtureConnection {
    stream: UnixStream,
    stale: Arc<AtomicBool>,
}

#[cfg(not(target_os = "linux"))]
impl Read for HostFixtureConnection {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let result = self.stream.read(buffer);
        if matches!(result, Ok(0))
            || result.as_ref().is_err_and(|error| {
                !matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                )
            })
        {
            self.stale.store(true, Ordering::SeqCst);
        }
        result
    }
}

#[cfg(not(target_os = "linux"))]
impl Write for HostFixtureConnection {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let result = self.stream.write(buffer);
        if result.as_ref().is_err_and(|error| {
            !matches!(
                error.kind(),
                io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
            )
        }) {
            self.stale.store(true, Ordering::SeqCst);
        }
        result
    }

    fn flush(&mut self) -> io::Result<()> {
        let result = self.stream.flush();
        if result.is_err() {
            self.stale.store(true, Ordering::SeqCst);
        }
        result
    }
}

#[cfg(not(target_os = "linux"))]
impl ReadWrite for HostFixtureConnection {
    fn recheck_before_side_effect(&mut self) -> io::Result<()> {
        if self.stale.load(Ordering::SeqCst) {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "host fixture connection is stale",
            ))
        } else {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "host fixture has no live peer authority hook",
            ))
        }
    }
}

impl AuthorizedConnection {
    fn from_accepted_stream(
        stream: UnixStream,
        _stale: Arc<AtomicBool>,
        authenticated_peer: Option<&KernelAuthenticatedUnixPeer>,
    ) -> Result<Self, PeerDenial> {
        #[cfg(target_os = "linux")]
        {
            stream.set_nonblocking(true).map_err(|error| {
                PeerDenial::PeerConnection(Arc::new(PeerConnectionError::IoFailure {
                    kind: error.kind(),
                    raw_os_error: error.raw_os_error(),
                    message: error.to_string(),
                }))
            })?;
            let mut connection =
                OwnedPeerConnection::from_connected_with_current_writer_credentials(stream)
                    .map_err(|error| PeerDenial::PeerConnection(Arc::new(error)))?;
            if let Some(authenticated_peer) = authenticated_peer {
                let proof_denial = Arc::new(Mutex::new(None));
                let check_denial = Arc::clone(&proof_denial);
                let result = connection.with_connection_io_checked_with_fd(
                    move |fd, _credentials| {
                        authenticated_peer
                            .require_same_connection_fd(fd)
                            .map_err(|error| {
                                if let Ok(mut current) = check_denial.lock() {
                                    *current = Some(map_peer_authentication_error(error));
                                }
                                PeerConnectionError::AuthorityRejected
                            })
                    },
                    |_, _| (),
                );
                if let Err(error) = result {
                    return Err(proof_denial
                        .lock()
                        .ok()
                        .and_then(|mut current| current.take())
                        .unwrap_or_else(|| PeerDenial::PeerConnection(Arc::new(error))));
                }
            } else {
                #[cfg(not(test))]
                return Err(PeerDenial::Unavailable);
            }
            Ok(Self::Task39(connection))
        }
        #[cfg(not(target_os = "linux"))]
        {
            // The production reader returns Unavailable on non-Linux targets;
            // this branch only lets host-only fake tests exercise the adapter.
            let _ = authenticated_peer;
            Ok(Self::HostFixture(HostFixtureConnection {
                stream,
                stale: _stale,
            }))
        }
    }
}

/// Successful authorization retains the exact accepted connection behind
/// task-39's owned `Read`/`Write` callback. Task-39 invokes the supplied live
/// authority hook before/after the callback and on both sides of every
/// read/write/flush primitive. The hook rechecks the PID 1 registry, process
/// handle, inventory generation, session, consent, endpoint, route and
/// capability owner.
///
/// One authenticated connection is bound to one stable request ID for its
/// lifetime. This candidate has no proven frame-reset boundary, so a second
/// request ID on the same connection is rejected; a partial frame cannot
/// be reinterpreted as a new request. EOF, timeout, or any latched task-39
/// connection failure makes this peer stale and requires a new checkout.
///
/// The owner can close this descriptor on Drop, but cannot revoke an FD alias
/// created before sole-descriptor transfer. EOF/stale transport state is never
/// interpreted as session, grant, or process-liveness revocation by itself.
/// No method exposes the server-side accepted stream/fd to the effect owner.
/// A client-side `dup`, fork/inherited descriptor, or passed alias remains
/// usable, so production registration stays denied unless a separate
/// current-writer/non-delegation owner proves the boundary or authenticates each
/// request writer. Socket cookie and `rdev` do not provide that proof.
pub struct AuthorizedPeer<'a, R: LaunchRecordView> {
    run: Pid1RunInstance,
    connection_epoch: NonZeroU64,
    registration: RegistrationId,
    process: ProcessInstanceKey,
    task24_identity: SpawnedProcessIdentity,
    record: R,
    route: TrustedRouteKey,
    endpoint: TrustedServiceEndpoint,
    endpoint_epoch: NonZeroU64,
    endpoint_recipient: EndpointCheckoutRecipient,
    inventory_generation: NonZeroU64,
    session_generation: NonZeroU64,
    capabilities: Vec<Vec<u8>>,
    consent_requirement: RouteConsentRequirement,
    authority_domain: Pid1AuthorityDomain,
    request_ledger: Arc<Mutex<ServiceRequestLedger>>,
    #[cfg(target_os = "linux")]
    observation: KernelPeerObservation,
    connection: AuthorizedConnection,
    #[cfg(target_os = "linux")]
    revalidate_connection: Box<AuthenticatedConnectionRevalidator<'a>>,
    connection_stale: Arc<AtomicBool>,
    request_frame: Arc<Mutex<Option<ServiceRequestFrameProgress>>>,
    request_decoder_claimed: bool,
    session_effect_gate: Arc<SessionEffectGate<'a>>,
    revalidate: Box<dyn Fn() -> Result<(), PeerDenial> + Send + Sync + 'a>,
    revalidate_under_gate: Box<dyn Fn() -> Result<(), PeerDenial> + Send + Sync + 'a>,
}

impl<R: LaunchRecordView> std::fmt::Debug for AuthorizedPeer<'_, R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthorizedPeer")
            .field("registration", &self.registration)
            .field("process", &self.process)
            .field("route", &self.route)
            .field("endpoint", &self.endpoint)
            .field("session_generation", &self.session_generation)
            .field("stream", &"retained")
            .finish()
    }
}

impl<R: LaunchRecordView> AuthorizedPeer<'_, R> {
    pub const fn registration(&self) -> RegistrationId {
        self.registration
    }

    pub const fn process(&self) -> ProcessInstanceKey {
        self.process
    }

    pub const fn record(&self) -> R {
        self.record
    }

    pub const fn route(&self) -> TrustedRouteKey {
        self.route
    }

    pub const fn endpoint(&self) -> TrustedServiceEndpoint {
        self.endpoint
    }

    pub const fn session_generation(&self) -> NonZeroU64 {
        self.session_generation
    }

    fn ensure_connection_current(&self) -> Result<(), PeerDenial> {
        if self.connection_stale.load(Ordering::SeqCst) {
            Err(PeerDenial::StaleConnection)
        } else {
            Ok(())
        }
    }

    /// Claim the connection's sole inbound NIPC request decoder. Its absolute
    /// deadline is fixed at construction; dropping it before a complete frame,
    /// a terminal I/O result, or deadline expiry stales this connection.
    pub fn new_nipc_request_decoder(
        &mut self,
        deadline: Instant,
    ) -> Result<NipcRequestDecoder, PeerDenial> {
        self.ensure_connection_current()?;
        if deadline <= Instant::now() {
            return Err(PeerDenial::RequestDeadlineExpired);
        }
        if self.request_decoder_claimed {
            return Err(PeerDenial::RequestDecoderAlreadyStarted);
        }
        if self
            .request_frame
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?
            .is_some()
        {
            return Err(PeerDenial::ConnectionRequestIdentityMismatch);
        }
        self.request_decoder_claimed = true;
        Ok(NipcRequestDecoder::for_authorized_peer(
            Arc::clone(&self.request_frame),
            Arc::clone(&self.connection_stale),
            self.record.app_id(),
            deadline,
        ))
    }

    /// Advance the one NIPC v1 request frame using the retained authenticated
    /// connection. `Pending` preserves the same byte offset and original
    /// deadline after `Interrupted` or `WouldBlock`; yield and resume this same
    /// decoder, without re-admitting the frame or creating another decoder.
    pub fn read_nipc_request(
        &mut self,
        decoder: &mut NipcRequestDecoder,
    ) -> Result<NipcRequestRead, PeerDenial> {
        if !self.request_decoder_claimed
            || !Arc::ptr_eq(&decoder.request_frame, &self.request_frame)
            || !Arc::ptr_eq(&decoder.connection_stale, &self.connection_stale)
        {
            decoder.fail_connection();
            return Err(PeerDenial::ConnectionRequestIdentityMismatch);
        }
        match self.with_live_io(|io| decoder.read_step(io)) {
            Ok(result) => result,
            Err(denial) => {
                decoder.fail_connection();
                Err(denial)
            }
        }
    }

    /// Consume only a frame emitted by this peer's SDK-backed decoder. The
    /// capability still has to come from the trusted route/operation mapper;
    /// request payload fields never grant it.
    pub fn admit_nipc_request(
        &self,
        request: DecodedNipcRequest,
        binding: CanonicalNipcOperationBinding,
    ) -> Result<ServiceRequestAdmission, PeerDenial> {
        if request.source_app_id() != self.record.app_id() {
            return Err(PeerDenial::InvalidNipcRequest);
        }
        if binding.route != self.route
            || binding.wire_request_id != request.wire_request_id()
            || binding.fingerprint != request.fingerprint()
        {
            return Err(PeerDenial::RequestIdentityConflict);
        }
        self.admit_request_operation(
            request.into_complete_frame(),
            binding.service_id,
            binding.contract_version,
            binding.operation_id,
            binding.capability_id,
            binding.effect_owner,
        )
    }

    /// Admit a parsed request under the current live PID1 authority. The
    /// resulting context records the request identity only; it is not a
    /// transferable grant to commit a side effect.
    pub fn begin_request_frame(
        &self,
        wire_request_id: StableServiceRequestId,
        expected_bytes: usize,
    ) -> Result<PendingServiceRequestFrame, PeerDenial> {
        self.ensure_connection_current()?;
        if expected_bytes == 0 || expected_bytes > MAX_SERVICE_REQUEST_FRAME_BYTES {
            return Err(PeerDenial::InvalidRequestFrameLength);
        }
        let mut progress = self
            .request_frame
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        match progress.as_ref() {
            None => {
                *progress = Some(ServiceRequestFrameProgress {
                    wire_request_id,
                    expected_bytes,
                    received_bytes: 0,
                    authority_io_deadline: None,
                });
            }
            Some(current)
                if current.wire_request_id == wire_request_id
                    && current.expected_bytes == expected_bytes => {}
            Some(_) => {
                self.connection_stale.store(true, Ordering::SeqCst);
                return Err(PeerDenial::ConnectionRequestIdentityMismatch);
            }
        }
        Ok(PendingServiceRequestFrame {
            progress: Arc::clone(&self.request_frame),
            connection_stale: Arc::clone(&self.connection_stale),
            wire_request_id,
        })
    }

    #[cfg(test)]
    fn admit_request(
        &self,
        frame: CompleteServiceRequestFrame,
        capability: &[u8],
    ) -> Result<ServiceRequestAdmission, PeerDenial> {
        let capability_name =
            std::str::from_utf8(capability).map_err(|_| PeerDenial::NotDeclared)?;
        let capability_id =
            CapabilityId::new(capability_name.to_owned()).map_err(|_| PeerDenial::NotDeclared)?;
        self.admit_request_operation(
            frame,
            ServiceId::new("test.service").map_err(|_| PeerDenial::Unavailable)?,
            ContractVersion::new(1, 0).map_err(|_| PeerDenial::Unavailable)?,
            OperationId::new("test").map_err(|_| PeerDenial::Unavailable)?,
            capability_id,
            ServiceEffectOwnerId::new([0x41; 16]),
        )
    }

    fn admit_request_operation(
        &self,
        frame: CompleteServiceRequestFrame,
        service_id: ServiceId,
        contract_version: ContractVersion,
        operation_id: OperationId,
        capability_id: CapabilityId,
        effect_owner: ServiceEffectOwnerId,
    ) -> Result<ServiceRequestAdmission, PeerDenial> {
        self.ensure_connection_current()?;
        if !Arc::ptr_eq(&frame.progress, &self.request_frame) {
            return Err(PeerDenial::ConnectionRequestIdentityMismatch);
        }
        let wire_request_id = frame.wire_request_id;
        let fingerprint = frame.fingerprint.ok_or(PeerDenial::InvalidNipcRequest)?;
        let canonical_request_bytes = frame.canonical_bytes.clone();
        {
            let progress = self
                .request_frame
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            let current = progress
                .as_ref()
                .ok_or(PeerDenial::IncompleteRequestFrame)?;
            if current.wire_request_id != wire_request_id {
                return Err(PeerDenial::ConnectionRequestIdentityMismatch);
            }
            if current.received_bytes != current.expected_bytes {
                return Err(PeerDenial::IncompleteRequestFrame);
            }
        }
        if !self
            .capabilities
            .iter()
            .any(|allowed| allowed.as_slice() == capability_id.as_str().as_bytes())
        {
            return Err(PeerDenial::NotDeclared);
        }
        (self.revalidate)()?;
        let key = ServiceRequestLedgerKey {
            run_generation: self.run.generation,
            app_id: self.record.app_id(),
            wire_request_id,
        };
        let scope = ServiceRequestAuthorityScope {
            registration: self.registration,
            process: self.process,
            task24_identity: self.task24_identity.clone(),
            app_session_id: self.record.app_session_id(),
            inventory_generation: self.inventory_generation,
            session_generation: self.session_generation,
            route: self.route,
            endpoint: self.endpoint,
            endpoint_epoch: self.endpoint_epoch,
            endpoint_recipient: self.endpoint_recipient.clone(),
            service_id: service_id.clone(),
            contract_version,
            operation_id: operation_id.clone(),
            capability_id: capability_id.clone(),
            effect_owner,
            consent_requirement: self.consent_requirement,
            fingerprint,
        };
        let (request_id, operation_state) = {
            let mut ledger = self
                .request_ledger
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            ledger.admit(key, scope, &self.run)?
        };
        Ok(ServiceRequestAdmission {
            run: self.run.clone(),
            request_id,
            wire_request_id,
            fingerprint,
            operation_state,
            owner_identity: effect_owner,
            canonical_request_bytes,
            consent_event: None,
            consent_requirement: self.consent_requirement,
            registration: self.registration,
            process: self.process,
            task24_identity: self.task24_identity.clone(),
            app_id: self.record.app_id(),
            app_session_id: self.record.app_session_id(),
            inventory_generation: self.inventory_generation,
            session_generation: self.session_generation,
            route: self.route,
            endpoint: self.endpoint,
            endpoint_epoch: self.endpoint_epoch,
            endpoint_recipient: self.endpoint_recipient.clone(),
            service_id,
            contract_version,
            operation_id,
            capability_id,
        })
    }

    /// Issue a request context with a consent event from the live session
    /// owner. A missing or stale provenance source fails closed; the ordinary
    /// `ConsentSource::connected()` flag cannot issue this evidence.
    #[cfg(test)]
    fn admit_request_with_session_consent<C: SessionOwnedConsentAuthority>(
        &self,
        frame: CompleteServiceRequestFrame,
        capability: &[u8],
        consent_owner: &C,
    ) -> Result<ServiceRequestAdmission, PeerDenial> {
        if self.consent_requirement != RouteConsentRequirement::Required {
            return Err(PeerDenial::ConsentNotRequired);
        }
        let mut admission = self.admit_request(frame, capability)?;
        let event = consent_owner.issue_event(&admission)?;
        if !event.matches_admission(&admission) {
            return Err(PeerDenial::ConsentDenied);
        }
        admission.consent_event = Some(event);
        Ok(admission)
    }

    /// Admit a complete SDK-decoded request under the current live authority.
    /// The caller must obtain `capability` from a trusted route/operation
    /// mapper; NIPC payload fields are not an authority source.
    pub fn admit_nipc_request_with_session_consent<C: SessionOwnedConsentAuthority>(
        &self,
        request: DecodedNipcRequest,
        binding: CanonicalNipcOperationBinding,
        consent_owner: &C,
    ) -> Result<ServiceRequestAdmission, PeerDenial> {
        if request.source_app_id() != self.record.app_id() {
            return Err(PeerDenial::InvalidNipcRequest);
        }
        if self.consent_requirement != RouteConsentRequirement::Required {
            return Err(PeerDenial::ConsentNotRequired);
        }
        if binding.route != self.route
            || binding.wire_request_id != request.wire_request_id()
            || binding.fingerprint != request.fingerprint()
        {
            return Err(PeerDenial::RequestIdentityConflict);
        }
        let mut admission = self.admit_request_operation(
            request.into_complete_frame(),
            binding.service_id,
            binding.contract_version,
            binding.operation_id,
            binding.capability_id,
            binding.effect_owner,
        )?;
        let event = consent_owner.issue_event(&admission)?;
        if !event.matches_admission(&admission) {
            return Err(PeerDenial::ConsentDenied);
        }
        admission.consent_event = Some(event);
        Ok(admission)
    }

    /// Commit one service effect through the service owner's synchronized
    /// transaction. Request admission and transport success are insufficient.
    /// This method revalidates under the matching PID1 gate and holds that gate
    /// through the service-owner transaction. The owner must compare current
    /// authority and installed-inventory generations and serialize revocation
    /// with the bounded effect; a mere `revalidate()` before this call does not
    /// satisfy that contract. Kernel process exit and already-sent bytes are
    /// not cancellable by this gate.
    pub fn commit_effect<T, O: ServiceOwnerEffectTransaction, B: Pid1ReadinessAtomicBoundary>(
        &self,
        admission: &ServiceRequestAdmission,
        readiness_boundary: &B,
        owner: &O,
        effect: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        if admission.consent_requirement != RouteConsentRequirement::NotRequired {
            return Err(PeerDenial::ConsentRequired);
        }
        self.commit_effect_with_live_check(admission, readiness_boundary, owner, || Ok(()), effect)
    }

    /// Commit an effect only while the exact session-owned consent event and
    /// its revision remain current under the shared PID1 gate. Event revocation
    /// must use that same gate. The owner transaction remains responsible for
    /// synchronizing its effect with the same request and package generation.
    pub fn commit_consented_effect<
        T,
        C: SessionOwnedConsentAuthority,
        O: ServiceOwnerEffectTransaction,
        B: Pid1ReadinessAtomicBoundary,
    >(
        &self,
        admission: &ServiceRequestAdmission,
        consent_owner: &C,
        readiness_boundary: &B,
        owner: &O,
        effect: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        if admission.consent_requirement != RouteConsentRequirement::Required {
            return Err(PeerDenial::ConsentNotRequired);
        }
        let event = admission
            .consent_event
            .as_ref()
            .ok_or(PeerDenial::ConsentUnavailable)?;
        if !event.matches_admission(admission) {
            return Err(PeerDenial::ConsentDenied);
        }
        self.commit_effect_with_live_check(
            admission,
            readiness_boundary,
            owner,
            || match consent_owner.is_current_unlocked(event)? {
                true => Ok(()),
                false => Err(PeerDenial::ConsentDenied),
            },
            effect,
        )
    }

    fn commit_effect_with_live_check<
        T,
        O: ServiceOwnerEffectTransaction,
        B: Pid1ReadinessAtomicBoundary,
    >(
        &self,
        admission: &ServiceRequestAdmission,
        readiness_boundary: &B,
        owner: &O,
        live_check: impl FnOnce() -> Result<(), PeerDenial>,
        effect: impl FnOnce() -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        if !self.admission_matches_peer(admission) {
            return Err(PeerDenial::StaleRegistration);
        }
        if !readiness_boundary
            .authority_domain()
            .is_same_domain(&self.authority_domain)
        {
            return Err(PeerDenial::Unavailable);
        }
        ensure_request_not_terminal(admission.operation_state())?;
        let callback_succeeded_this_attempt = Arc::new(AtomicBool::new(false));
        if let Err(cause) = self.ensure_connection_current() {
            return Err(classify_service_effect_error(
                cause,
                &admission.operation_state,
                callback_succeeded_this_attempt.load(Ordering::SeqCst),
            ));
        }
        {
            let frame = self
                .request_frame
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            if frame
                .as_ref()
                .is_none_or(|frame| frame.wire_request_id != admission.stable_request_id())
            {
                return Err(classify_service_effect_error(
                    PeerDenial::ConnectionRequestIdentityMismatch,
                    &admission.operation_state,
                    callback_succeeded_this_attempt.load(Ordering::SeqCst),
                ));
            }
        }
        let operation_state = Arc::clone(&admission.operation_state);
        let state_for_gate = Arc::clone(&operation_state);
        let callback_succeeded_for_gate = Arc::clone(&callback_succeeded_this_attempt);
        run_session_effect(self.session_effect_gate.as_ref(), || {
            readiness_boundary.with_owner_gate(&self.run, None, move || {
                ensure_request_not_terminal(admission.operation_state())?;
                (self.revalidate_under_gate)()?;
                live_check()?;
                if !owner.admission_is_current(admission)? {
                    return Err(PeerDenial::StaleRegistration);
                }
                let owner_identity = owner.durable_owner_id_for(admission)?;
                admission.confirm_durable_owner_identity(owner_identity)?;
                let identity_claim = durable_request_identity_claim(owner_identity, admission);
                let durable_identity = owner.validate_request_with_identity(
                    admission,
                    owner_identity,
                    &identity_claim,
                )?;
                if !durable_identity.matches_admission(owner_identity, admission) {
                    return Err(PeerDenial::ServiceEffectNotInvoked(Arc::new(
                        PeerDenial::RequestIdentityConflict,
                    )));
                }
                let state_for_effect = Arc::clone(&state_for_gate);
                let callback_succeeded_for_effect = Arc::clone(&callback_succeeded_for_gate);
                let mut effect = Some(effect);
                match owner.commit_if_current_with_identity(admission, &durable_identity, || {
                    state_for_effect
                        .compare_exchange(
                            ServiceRequestOperationState::NotInvoked as u8,
                            ServiceRequestOperationState::Executing as u8,
                            Ordering::SeqCst,
                            Ordering::SeqCst,
                        )
                        .map_err(|value| request_state_denial(value))?;
                    let _execution_guard =
                        RequestExecutionGuard::new(Arc::clone(&state_for_effect));
                    let effect = effect.take().ok_or(PeerDenial::Unavailable)?;
                    match effect() {
                        Ok(value) => {
                            state_for_effect.store(
                                ServiceRequestOperationState::Executed as u8,
                                Ordering::SeqCst,
                            );
                            callback_succeeded_for_effect.store(true, Ordering::SeqCst);
                            Ok(value)
                        }
                        Err(PeerDenial::ServiceEffectFailed(cause)) => {
                            state_for_effect.store(
                                ServiceRequestOperationState::Failed as u8,
                                Ordering::SeqCst,
                            );
                            Err(PeerDenial::ServiceEffectFailed(cause))
                        }
                        Err(error) => {
                            state_for_effect.store(
                                ServiceRequestOperationState::OutcomeUnknown as u8,
                                Ordering::SeqCst,
                            );
                            Err(error)
                        }
                    }
                }) {
                    Ok(value)
                        if ServiceRequestOperationState::from_raw(
                            state_for_gate.load(Ordering::SeqCst),
                        ) == ServiceRequestOperationState::Executed =>
                    {
                        Ok(value)
                    }
                    Ok(_) => Err(PeerDenial::ServiceEffectNotInvoked(Arc::new(
                        PeerDenial::Unavailable,
                    ))),
                    Err(error) => Err(classify_service_effect_error(
                        error,
                        &state_for_gate,
                        callback_succeeded_for_gate.load(Ordering::SeqCst),
                    )),
                }
            })
        })
        .map_err(|cause| {
            classify_service_effect_error(
                cause,
                &operation_state,
                callback_succeeded_this_attempt.load(Ordering::SeqCst),
            )
        })
    }

    /// Revalidate a RAM receipt view and expose its historical value only
    /// within the same authority gate. A callback error also invalidates this
    /// view. The value is not a service effect or authorization; effectful
    /// work must use the service-owner transaction path instead.
    pub fn with_current_package_request_receipt<
        T,
        O: ServiceRequestReceiptTransaction,
        B: Pid1ReadinessAtomicBoundary,
    >(
        &self,
        receipt: &mut Pid1PackageRequestReceipt,
        readiness_boundary: &B,
        owner: &O,
        inspect: impl FnOnce(&GenerationBoundCommitReceipt) -> Result<T, PeerDenial>,
    ) -> Result<T, PeerDenial> {
        if !receipt.active {
            return Err(PeerDenial::PackageReceiptInvalidated);
        }

        let expected = match receipt.receipt {
            Some(receipt) => receipt,
            None => {
                receipt.invalidate();
                return Err(PeerDenial::PackageReceiptInvalidated);
            }
        };
        let outcome = (|| {
            if self.connection_epoch != receipt.connection_epoch
                || !Arc::ptr_eq(&self.connection_stale, &receipt.connection_stale)
            {
                return Err(PeerDenial::StaleConnection);
            }
            if !receipt.run.is_same_run(&self.run) || !receipt.run.is_active() {
                return Err(PeerDenial::StaleRun);
            }
            if !self.admission_matches_peer(&receipt.admission)
                || !readiness_boundary
                    .authority_domain()
                    .is_same_domain(&self.authority_domain)
                || !readiness_boundary
                    .authority_domain()
                    .is_same_domain(&receipt.authority_domain)
            {
                return Err(PeerDenial::Unavailable);
            }
            self.ensure_connection_current()?;
            if receipt.admission.operation_state() != ServiceRequestOperationState::ReceiptRecorded
            {
                return Err(PeerDenial::PackageReceiptInvalidated);
            }
            {
                let frame = self
                    .request_frame
                    .try_lock()
                    .map_err(|_| PeerDenial::Unavailable)?;
                if frame.as_ref().is_none_or(|frame| {
                    frame.wire_request_id != receipt.admission.stable_request_id()
                }) {
                    return Err(PeerDenial::ConnectionRequestIdentityMismatch);
                }
            }

            run_session_effect(self.session_effect_gate.as_ref(), || {
                readiness_boundary.with_owner_gate(&self.run, None, || {
                    self.ensure_connection_current()?;
                    (self.revalidate_under_gate)()?;
                    let owner_id = owner.receipt_owner_id_for(&receipt.admission)?;
                    receipt.admission.confirm_durable_owner_identity(owner_id)?;
                    if owner.resolve_receipt_state(&receipt.admission)?
                        != ServiceRequestOperationState::ReceiptRecorded
                    {
                        return Err(PeerDenial::PackageReceiptInvalidated);
                    }
                    let current = owner
                        .resolve_receipt_record(&receipt.admission)?
                        .ok_or(PeerDenial::PackageReceiptInvalidated)?;
                    if current != expected
                        || current.record_id() != receipt.admission.request_id().get()
                        || current.app_id().0 != receipt.admission.app_id()
                        || current.generation() != receipt.admission.inventory_generation().get()
                    {
                        return Err(PeerDenial::RequestIdentityConflict);
                    }
                    inspect(&current)
                })
            })
        })();

        if outcome.is_err() {
            receipt.invalidate();
        }
        outcome
    }

    /// Record task25's fixed, generation-checked local receipt for this
    /// admission. The returned RAM lease is tied to this connection epoch and
    /// run. It is invalidated when the connection, run, authority, package
    /// generation, or receipt owner stops matching. The underlying Task25
    /// receipt remains historical metadata; it does not prove an application
    /// effect or authorize a production service.
    pub fn record_package_request_receipt<
        O: ServiceRequestReceiptTransaction,
        B: Pid1ReadinessAtomicBoundary,
    >(
        &self,
        admission: &ServiceRequestAdmission,
        readiness_boundary: &B,
        owner: &O,
    ) -> Result<Pid1PackageRequestReceipt, PeerDenial> {
        if admission.consent_requirement == RouteConsentRequirement::Required {
            return Err(PeerDenial::ConsentRequired);
        }
        if !self.admission_matches_peer(admission) {
            return Err(PeerDenial::StaleRegistration);
        }
        if !readiness_boundary
            .authority_domain()
            .is_same_domain(&self.authority_domain)
        {
            return Err(PeerDenial::Unavailable);
        }
        ensure_request_not_terminal(admission.operation_state())?;
        self.ensure_connection_current()?;
        {
            let frame = self
                .request_frame
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            if frame
                .as_ref()
                .is_none_or(|frame| frame.wire_request_id != admission.stable_request_id())
            {
                return Err(PeerDenial::ConnectionRequestIdentityMismatch);
            }
        }
        let operation_state = Arc::clone(&admission.operation_state);
        run_session_effect(self.session_effect_gate.as_ref(), || {
            readiness_boundary.with_owner_gate(&self.run, None, move || {
                ensure_request_not_terminal(admission.operation_state())?;
                (self.revalidate_under_gate)()?;
                let receipt_owner = owner.receipt_owner_id_for(admission)?;
                admission.confirm_durable_owner_identity(receipt_owner)?;
                operation_state
                    .compare_exchange(
                        ServiceRequestOperationState::NotInvoked as u8,
                        ServiceRequestOperationState::Executing as u8,
                        Ordering::SeqCst,
                        Ordering::SeqCst,
                    )
                    .map_err(|value| request_state_denial(value))?;
                let _execution_guard = RequestExecutionGuard::new(Arc::clone(&operation_state));
                match owner.record_receipt_if_current(admission) {
                    Ok(receipt) => {
                        operation_state.store(
                            ServiceRequestOperationState::ReceiptRecorded as u8,
                            Ordering::SeqCst,
                        );
                        Ok(Pid1PackageRequestReceipt::from_recorded(
                            receipt, admission, self,
                        ))
                    }
                    Err(cause) => {
                        operation_state.store(
                            ServiceRequestOperationState::OutcomeUnknown as u8,
                            Ordering::SeqCst,
                        );
                        Err(cause)
                    }
                }
            })
        })
        .map_err(|cause| PeerDenial::ServiceRequestReceiptNotRecorded(Arc::new(cause)))
    }

    /// Resolve an ambiguous task-25 receipt attempt through the receipt owner's
    /// exact request lookup. The same admission and request ID remain attached
    /// to the lookup. Only an owner-confirmed `NotInvoked` result reopens the
    /// request for retry; `ReceiptRecorded` is terminal, and `OutcomeUnknown`
    /// or an unavailable/mismatched owner leaves it fail-closed.
    pub fn resolve_package_request_receipt<
        O: ServiceRequestReceiptTransaction,
        B: Pid1ReadinessAtomicBoundary,
    >(
        &self,
        admission: &ServiceRequestAdmission,
        readiness_boundary: &B,
        owner: &O,
    ) -> Result<ServiceRequestOperationState, PeerDenial> {
        if admission.consent_requirement == RouteConsentRequirement::Required {
            return Err(PeerDenial::ConsentRequired);
        }
        if !self.admission_matches_peer(admission)
            || !readiness_boundary
                .authority_domain()
                .is_same_domain(&self.authority_domain)
        {
            return Err(PeerDenial::Unavailable);
        }
        {
            let frame = self
                .request_frame
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            if frame
                .as_ref()
                .is_none_or(|frame| frame.wire_request_id != admission.stable_request_id())
            {
                return Err(PeerDenial::ConnectionRequestIdentityMismatch);
            }
        }

        run_session_effect(self.session_effect_gate.as_ref(), || {
            readiness_boundary.with_owner_gate(&self.run, None, || {
                (self.revalidate_under_gate)()?;
                let receipt_owner = owner.receipt_owner_id_for(admission)?;
                admission.confirm_durable_owner_identity(receipt_owner)?;
                let owner_state = owner.resolve_receipt_state(admission)?;
                if !matches!(
                    owner_state,
                    ServiceRequestOperationState::NotInvoked
                        | ServiceRequestOperationState::Executing
                        | ServiceRequestOperationState::ReceiptRecorded
                        | ServiceRequestOperationState::OutcomeUnknown
                ) {
                    return Err(PeerDenial::Unavailable);
                }

                let local_state = admission.operation_state();
                if matches!(
                    local_state,
                    ServiceRequestOperationState::Executed
                        | ServiceRequestOperationState::ReceiptRecorded
                        | ServiceRequestOperationState::Failed
                ) && local_state != owner_state
                {
                    admission.operation_state.store(
                        ServiceRequestOperationState::OutcomeUnknown as u8,
                        Ordering::SeqCst,
                    );
                    return Err(PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(
                        PeerDenial::RequestOutcomeUnknown,
                    )));
                }

                admission
                    .operation_state
                    .store(owner_state as u8, Ordering::SeqCst);
                Ok(owner_state)
            })
        })
    }

    /// Re-resolve the exact receipt under the same request gate and return a
    /// connection/run-scoped RAM lease for subsequent bounded use.
    /// A resolver that reports only `ReceiptRecorded` without returning the
    /// matching record is insufficient to clear an unknown outcome.
    pub fn resolve_package_request_receipt_record<
        O: ServiceRequestReceiptTransaction,
        B: Pid1ReadinessAtomicBoundary,
    >(
        &self,
        admission: &ServiceRequestAdmission,
        readiness_boundary: &B,
        owner: &O,
    ) -> Result<Pid1PackageRequestReceipt, PeerDenial> {
        if admission.consent_requirement == RouteConsentRequirement::Required {
            return Err(PeerDenial::ConsentRequired);
        }
        if !self.admission_matches_peer(admission)
            || !readiness_boundary
                .authority_domain()
                .is_same_domain(&self.authority_domain)
        {
            return Err(PeerDenial::Unavailable);
        }
        {
            let frame = self
                .request_frame
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            if frame
                .as_ref()
                .is_none_or(|frame| frame.wire_request_id != admission.stable_request_id())
            {
                return Err(PeerDenial::ConnectionRequestIdentityMismatch);
            }
        }

        run_session_effect(self.session_effect_gate.as_ref(), || {
            readiness_boundary.with_owner_gate(&self.run, None, || {
                (self.revalidate_under_gate)()?;
                let receipt_owner = owner.receipt_owner_id_for(admission)?;
                admission.confirm_durable_owner_identity(receipt_owner)?;
                let state = owner.resolve_receipt_state(admission)?;
                if !matches!(
                    state,
                    ServiceRequestOperationState::NotInvoked
                        | ServiceRequestOperationState::Executing
                        | ServiceRequestOperationState::ReceiptRecorded
                        | ServiceRequestOperationState::OutcomeUnknown
                ) {
                    return Err(PeerDenial::Unavailable);
                }
                let local_state = admission.operation_state();
                if matches!(
                    local_state,
                    ServiceRequestOperationState::Executed
                        | ServiceRequestOperationState::ReceiptRecorded
                        | ServiceRequestOperationState::Failed
                ) && local_state != state
                {
                    admission.operation_state.store(
                        ServiceRequestOperationState::OutcomeUnknown as u8,
                        Ordering::SeqCst,
                    );
                    return Err(PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(
                        PeerDenial::RequestOutcomeUnknown,
                    )));
                }
                if state != ServiceRequestOperationState::ReceiptRecorded {
                    admission
                        .operation_state
                        .store(state as u8, Ordering::SeqCst);
                    return Err(match state {
                        ServiceRequestOperationState::NotInvoked => {
                            PeerDenial::RequestOperationFailed
                        }
                        ServiceRequestOperationState::Executing
                        | ServiceRequestOperationState::OutcomeUnknown => {
                            PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(
                                PeerDenial::RequestOutcomeUnknown,
                            ))
                        }
                        _ => PeerDenial::Unavailable,
                    });
                }
                let Some(receipt) = owner.resolve_receipt_record(admission)? else {
                    admission.operation_state.store(
                        ServiceRequestOperationState::OutcomeUnknown as u8,
                        Ordering::SeqCst,
                    );
                    return Err(PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(
                        PeerDenial::RequestOutcomeUnknown,
                    )));
                };
                if receipt.record_id() != admission.request_id().get()
                    || receipt.app_id().0 != admission.app_id()
                    || receipt.generation() != admission.inventory_generation().get()
                {
                    admission.operation_state.store(
                        ServiceRequestOperationState::OutcomeUnknown as u8,
                        Ordering::SeqCst,
                    );
                    return Err(PeerDenial::RequestIdentityConflict);
                }
                admission.operation_state.store(
                    ServiceRequestOperationState::ReceiptRecorded as u8,
                    Ordering::SeqCst,
                );
                Ok(Pid1PackageRequestReceipt::from_recorded(
                    receipt, admission, self,
                ))
            })
        })
    }

    /// Ask the service owner to reconcile an interrupted operation. The
    /// in-memory PID1 ledger cannot decide whether an `Executing` operation
    /// ran before a crash or callback abort. Without a durable owner lookup,
    /// this returns `Unavailable` and leaves the operation unresolved.
    pub fn resolve_request_state<
        O: ServiceOwnerEffectTransaction,
        B: Pid1ReadinessAtomicBoundary,
    >(
        &self,
        admission: &ServiceRequestAdmission,
        readiness_boundary: &B,
        owner: &O,
    ) -> Result<ServiceRequestOperationState, PeerDenial> {
        if !self.admission_matches_peer(admission)
            || !readiness_boundary
                .authority_domain()
                .is_same_domain(&self.authority_domain)
        {
            return Err(PeerDenial::Unavailable);
        }
        run_session_effect(self.session_effect_gate.as_ref(), || {
            readiness_boundary.with_owner_gate(&self.run, None, || {
                (self.revalidate_under_gate)()?;
                let owner_identity = owner.durable_owner_id_for(admission)?;
                admission.confirm_durable_owner_identity(owner_identity)?;
                let identity_claim = durable_request_identity_claim(owner_identity, admission);
                let durable_identity = owner.validate_request_with_identity(
                    admission,
                    owner_identity,
                    &identity_claim,
                )?;
                if !durable_identity.matches_admission(owner_identity, admission) {
                    return Err(PeerDenial::RequestIdentityConflict);
                }
                let owner_state =
                    owner.resolve_request_state_with_identity(admission, &durable_identity)?;
                let local_state = admission.operation_state();
                let terminal_local_mismatch = matches!(
                    local_state,
                    ServiceRequestOperationState::Executed
                        | ServiceRequestOperationState::Failed
                        | ServiceRequestOperationState::ReceiptRecorded
                ) && local_state != owner_state;
                if terminal_local_mismatch {
                    admission.operation_state.store(
                        ServiceRequestOperationState::OutcomeUnknown as u8,
                        Ordering::SeqCst,
                    );
                    return Err(PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(
                        PeerDenial::RequestOutcomeUnknown,
                    )));
                }
                admission
                    .operation_state
                    .store(owner_state as u8, Ordering::SeqCst);
                Ok(owner_state)
            })
        })
    }

    fn admission_matches_peer(&self, admission: &ServiceRequestAdmission) -> bool {
        admission.run.is_same_run(&self.run)
            && admission.registration == self.registration
            && admission.process == self.process
            && admission
                .task24_identity
                .same_instance(&self.task24_identity)
            && admission.app_id == self.record.app_id()
            && admission.app_session_id == self.record.app_session_id()
            && admission.inventory_generation == self.inventory_generation
            && admission.session_generation == self.session_generation
            && admission.route == self.route
            && admission.endpoint == self.endpoint
            && admission.endpoint_epoch == self.endpoint_epoch
            && admission.endpoint_recipient == self.endpoint_recipient
            && admission.consent_requirement == self.consent_requirement
            && self
                .capabilities
                .iter()
                .any(|allowed| allowed.as_slice() == admission.capability())
    }

    /// Run a bounded protocol operation over the retained connection. Task-39
    /// rechecks the live authority hook around every read/write/flush and the
    /// whole callback. Call `AuthorizedStreamIo::recheck_before_side_effect()`
    /// after parsing or consuming buffered data and before each sensitive
    /// effect. These checks cannot interrupt a blocking syscall, and a partial
    /// write already accepted by the kernel cannot be rolled back. The service
    /// commit must synchronize with the same authority before accepting effects.
    pub fn with_live_io<T>(
        &mut self,
        operation: impl for<'io> FnOnce(&mut AuthorizedStreamIo<'io>) -> T,
    ) -> Result<T, PeerDenial> {
        #[cfg(target_os = "linux")]
        let observation = self.observation.clone();
        self.ensure_connection_current()?;
        let revalidate = &self.revalidate;
        #[cfg(target_os = "linux")]
        let revalidate_connection = &self.revalidate_connection;
        revalidate()?;

        match &mut self.connection {
            #[cfg(target_os = "linux")]
            AuthorizedConnection::Task39(connection) => {
                let authority_denial = Arc::new(Mutex::new(None));
                let hook_denial = Arc::clone(&authority_denial);
                let callback_denial = Arc::clone(&authority_denial);
                let mut operation = Some(operation);
                let outcome = connection.with_connection_io_checked_with_fd(
                    |fd, credentials| match revalidate_connection(fd, credentials) {
                        Ok(()) => Ok(()),
                        Err(denial) => {
                            if let Ok(mut current) = hook_denial.lock() {
                                *current = Some(denial);
                            }
                            Err(PeerConnectionError::AuthorityRejected)
                        }
                    },
                    |io, credentials| {
                        if credentials.pid() != observation.pid
                            || credentials.uid() != observation.uid
                            || credentials.gid() != observation.gid
                        {
                            if let Ok(mut current) = callback_denial.lock() {
                                *current = Some(PeerDenial::StalePeer);
                            }
                            return None;
                        }
                        let mut io = AuthorizedStreamIo { inner: io };
                        Some(operation.take().expect("task-39 calls the callback once")(
                            &mut io,
                        ))
                    },
                );
                match outcome {
                    Ok(Some(value)) => {
                        self.ensure_connection_current()?;
                        Ok(value)
                    }
                    Ok(None) => {
                        self.connection_stale.store(true, Ordering::SeqCst);
                        Err(authority_denial
                            .lock()
                            .ok()
                            .and_then(|mut current| current.take())
                            .unwrap_or(PeerDenial::Unavailable))
                    }
                    Err(error) => {
                        self.connection_stale.store(true, Ordering::SeqCst);
                        Err(authority_denial
                            .lock()
                            .ok()
                            .and_then(|mut current| current.take())
                            .unwrap_or_else(|| PeerDenial::PeerConnection(Arc::new(error))))
                    }
                }
            }
            #[cfg(not(target_os = "linux"))]
            AuthorizedConnection::HostFixture(stream) => {
                let mut io = AuthorizedStreamIo { inner: stream };
                let result = operation(&mut io);
                self.ensure_connection_current()?;
                revalidate()?;
                Ok(result)
            }
        }
    }

    /// Run one request's transport work through task-39's absolute-deadline,
    /// nonblocking API. The first hook handoff binds one absolute deadline to
    /// this request frame; a continuation must pass the identical `Instant`.
    /// Each handoff consumes its `FnOnce` callback once. After `WouldBlock`,
    /// continue the same request/admission with the same deadline and retain
    /// protocol offsets; do not re-admit or extend the deadline. Every I/O
    /// boundary takes the PID1 gate, revalidates the live registry/session/
    /// inventory state, and asks the service owner whether this exact request
    /// admission is still current. Revocation cannot recall bytes already
    /// accepted by the kernel or cancel a syscall already in progress.
    #[cfg(target_os = "linux")]
    pub fn with_request_io_until<T, O, B>(
        &mut self,
        admission: &ServiceRequestAdmission,
        readiness_boundary: &B,
        owner: &O,
        deadline: std::time::Instant,
        operation: impl for<'io> FnOnce(&mut AuthorizedStreamIo<'io>) -> T,
    ) -> Result<T, PeerDenial>
    where
        O: ServiceOwnerEffectTransaction,
        B: Pid1ReadinessAtomicBoundary,
    {
        require_same_authority_domain(readiness_boundary, &self.authority_domain)?;
        self.ensure_connection_current()?;
        if !self.admission_matches_peer(admission) {
            return Err(PeerDenial::StaleRegistration);
        }
        {
            let mut frame = self
                .request_frame
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            let Some(frame) = frame.as_mut() else {
                self.connection_stale.store(true, Ordering::SeqCst);
                return Err(PeerDenial::IncompleteRequestFrame);
            };
            if frame.wire_request_id != admission.stable_request_id() {
                self.connection_stale.store(true, Ordering::SeqCst);
                return Err(PeerDenial::ConnectionRequestIdentityMismatch);
            }
            if deadline <= Instant::now() {
                self.connection_stale.store(true, Ordering::SeqCst);
                return Err(PeerDenial::RequestDeadlineExpired);
            }
            if let Err(denial) = frame.bind_authority_io_deadline(deadline) {
                self.connection_stale.store(true, Ordering::SeqCst);
                return Err(denial);
            }
        }
        let run = self.run.clone();
        let connection_stale = Arc::clone(&self.connection_stale);
        let revalidate_under_gate = &self.revalidate_under_gate;
        let session_effect_gate = Arc::clone(&self.session_effect_gate);
        let mut operation = Some(operation);

        match &mut self.connection {
            AuthorizedConnection::Task39(connection) => {
                let observation = self.observation.clone();
                let denial = Arc::new(Mutex::new(None));
                let hook_denial = Arc::clone(&denial);
                let callback_denial = Arc::clone(&denial);
                let outcome = connection.with_nonblocking_io_until_peer_check_with_fd(
                    deadline,
                    |fd, credentials| {
                        let check = run_session_effect(session_effect_gate.as_ref(), || {
                            if Instant::now() >= deadline {
                                return Err(PeerDenial::RequestDeadlineExpired);
                            }
                            readiness_boundary.with_owner_gate(&run, None, || {
                                if credentials.pid() as i32 != observation.pid
                                    || credentials.uid() != observation.uid
                                    || credentials.gid() != observation.gid
                                {
                                    return Err(PeerDenial::StalePeer);
                                }
                                if let Some(peer) = observation.authenticated_peer.as_deref() {
                                    peer.require_same_connection_fd(fd)
                                        .map_err(map_peer_authentication_error)?;
                                } else {
                                    #[cfg(not(test))]
                                    return Err(PeerDenial::Unavailable);
                                }
                                if connection_stale.load(Ordering::SeqCst) {
                                    return Err(PeerDenial::StaleConnection);
                                }
                                revalidate_under_gate()?;
                                if owner.admission_is_current(admission)? {
                                    Ok(())
                                } else {
                                    Err(PeerDenial::StaleRegistration)
                                }
                            })
                        });
                        match check {
                            Ok(()) => Ok(()),
                            Err(cause) => {
                                if let Ok(mut current) = hook_denial.lock() {
                                    *current = Some(cause);
                                }
                                Err(PeerConnectionError::AuthorityRejected)
                            }
                        }
                    },
                    |io, credentials| {
                        if credentials.pid() != observation.pid
                            || credentials.uid() != observation.uid
                            || credentials.gid() != observation.gid
                        {
                            if let Ok(mut current) = callback_denial.lock() {
                                *current = Some(PeerDenial::StalePeer);
                            }
                            return None;
                        }
                        let mut io = AuthorizedStreamIo { inner: io };
                        Some(operation.take().expect("task-39 calls the callback once")(
                            &mut io,
                        ))
                    },
                );
                match outcome {
                    Ok(Some(value)) => {
                        // The task-39 callback can overlap decoder/EOF/error
                        // invalidation on another holder of this latch.
                        if connection_stale.load(Ordering::SeqCst) {
                            return Err(PeerDenial::StaleConnection);
                        }
                        Ok(value)
                    }
                    Ok(None) => {
                        self.connection_stale.store(true, Ordering::SeqCst);
                        Err(denial
                            .lock()
                            .ok()
                            .and_then(|mut current| current.take())
                            .unwrap_or(PeerDenial::Unavailable))
                    }
                    Err(error) => {
                        self.connection_stale.store(true, Ordering::SeqCst);
                        Err(denial
                            .lock()
                            .ok()
                            .and_then(|mut current| current.take())
                            .unwrap_or_else(|| PeerDenial::PeerConnection(Arc::new(error))))
                    }
                }
            }
        }
    }
}

fn validate_declared(manifest: &TrustedManifest, capabilities: &[&[u8]]) -> Result<(), PeerDenial> {
    if capabilities
        .iter()
        .any(|capability| capability.is_empty() || !manifest.declares(capability))
    {
        return Err(PeerDenial::NotDeclared);
    }
    Ok(())
}

fn capabilities_match(declared: &[Vec<u8>], required: &[&[u8]]) -> bool {
    declared.len() == required.len()
        && declared
            .iter()
            .zip(required)
            .all(|(left, right)| left.as_slice() == *right)
}

#[cfg(test)]
mod tests {
    use super::nipc_request_decoder::{decode_test_request, decode_test_request_for_connection};
    use super::pidfd_process_instance::{PidfdKernel, PidfdProcessInstanceAuthority};
    use super::*;
    use linux_peer_credentials_supplier::{PeerCredentialError, SocketOption};
    use nagi_package::{InstallPolicy, PackageService, PackageView, TRUSTED_SIGNING_PUBLIC_KEY};
    use nagi_service_contract::{ServiceDescriptor, ServiceOperation};
    use std::collections::{HashMap, HashSet};
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;

    fn fixture_durable_owner_id() -> ServiceEffectOwnerId {
        ServiceEffectOwnerId::new([0x41; 16])
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn current_writer_connection_is_bound_to_the_authenticated_socket_proof() {
        let (candidate, _candidate_peer) = UnixStream::pair().unwrap();
        let (other, _other_peer) = UnixStream::pair().unwrap();
        linux_peer_credentials_supplier::enable_passcred(candidate.as_raw_fd()).unwrap();
        linux_peer_credentials_supplier::enable_passcred(other.as_raw_fd()).unwrap();
        let other_proof = KernelAuthenticatedUnixPeer::authenticate(&other).unwrap();

        let rejected = AuthorizedConnection::from_accepted_stream(
            candidate,
            Arc::new(AtomicBool::new(false)),
            Some(&other_proof),
        );
        assert!(matches!(
            rejected,
            Err(PeerDenial::KernelPeerAuthentication(_))
        ));

        let (same, _same_peer) = UnixStream::pair().unwrap();
        linux_peer_credentials_supplier::enable_passcred(same.as_raw_fd()).unwrap();
        let same_proof = KernelAuthenticatedUnixPeer::authenticate(&same).unwrap();
        let accepted = AuthorizedConnection::from_accepted_stream(
            same,
            Arc::new(AtomicBool::new(false)),
            Some(&same_proof),
        );
        assert!(matches!(accepted, Ok(AuthorizedConnection::Task39(_))));
    }

    macro_rules! assert_denial {
        ($result:expr, $expected:pat) => {{
            let result = $result;
            assert!(
                matches!(&result, Err($expected)),
                "unexpected authorization outcome: {result:?}"
            );
        }};
    }

    fn wire_request_id(value: u8) -> StableServiceRequestId {
        StableServiceRequestId::from_protocol_decoder([value; 16])
    }

    fn test_nipc_route(
        route: TrustedRouteKey,
        destination_app_id: u64,
        payload_type: &str,
    ) -> TrustedNipcOperationRoute {
        let service_id = ServiceId::new("files").unwrap();
        let version = ContractVersion::new(1, 0).unwrap();
        let operation_id = OperationId::new("read").unwrap();
        let capability = CapabilityId::new("files.read").unwrap();
        let descriptor = ServiceDescriptor::new(
            service_id,
            [version],
            [ServiceOperation::new(
                operation_id.clone(),
                Some(capability),
            )],
        )
        .unwrap();
        TrustedNipcOperationRoute::from_service_contract(
            route,
            destination_app_id,
            payload_type,
            &descriptor,
            version,
            &operation_id,
            fixture_durable_owner_id(),
        )
        .unwrap()
    }

    fn test_nipc_mapper(
        route: TrustedRouteKey,
        destination_app_id: u64,
        payload_type: &str,
    ) -> CanonicalNipcOperationMapper {
        CanonicalNipcOperationMapper::from_trusted_routes([test_nipc_route(
            route,
            destination_app_id,
            payload_type,
        )])
        .unwrap()
    }

    fn decode_mapped_request<R: LaunchRecordView>(
        peer: &AuthorizedPeer<'_, R>,
        mapper: &CanonicalNipcOperationMapper,
        request_id: [u8; 16],
    ) -> (DecodedNipcRequest, CanonicalNipcOperationBinding) {
        let request = decode_test_request_for_connection(
            Arc::clone(&peer.request_frame),
            Arc::clone(&peer.connection_stale),
            peer.record.app_id(),
            Some(1),
            request_id,
            "files.read@1",
            b"mapped files request",
        )
        .unwrap();
        let binding = mapper.bind_request(&request).unwrap();
        (request, binding)
    }

    fn complete_request_frame<R: LaunchRecordView>(
        peer: &AuthorizedPeer<'_, R>,
        value: u8,
    ) -> CompleteServiceRequestFrame {
        let mut pending = peer.begin_request_frame(wire_request_id(value), 1).unwrap();
        if pending.received_bytes().unwrap() == 0 {
            assert_eq!(pending.record_read_result(Ok(1)).unwrap(), 1);
        }
        let mut frame = pending.finish().unwrap();
        frame.fingerprint = Some(ServiceRequestFingerprint::for_test(&[value]));
        frame
    }

    fn complete_request_frame_with_content<R: LaunchRecordView>(
        peer: &AuthorizedPeer<'_, R>,
        value: u8,
        content: &[u8],
    ) -> CompleteServiceRequestFrame {
        let mut frame = complete_request_frame(peer, value);
        frame.fingerprint = Some(ServiceRequestFingerprint::for_test(content));
        frame
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct Record {
        pid: u32,
        app: u64,
        session: u64,
    }

    impl LaunchRecordView for Record {
        fn process_id(self) -> u32 {
            self.pid
        }
        fn app_id(self) -> u64 {
            self.app
        }
        fn app_session_id(self) -> u64 {
            self.session
        }
    }

    struct Inventory {
        manifest: Mutex<Option<TrustedManifest>>,
        available: AtomicBool,
        app_id: u64,
        generation: AtomicU64,
    }

    impl TrustedInventory for Inventory {
        fn installed_manifest(&self, app_id: u64) -> Result<Option<TrustedManifest>, PeerDenial> {
            if !self.available.load(Ordering::SeqCst) {
                return Err(PeerDenial::Unavailable);
            }
            Ok(self
                .manifest
                .lock()
                .map_err(|_| PeerDenial::Unavailable)?
                .clone()
                .filter(|manifest| manifest.app_id == app_id))
        }

        fn generation_is_current(
            &self,
            app_id: u64,
            generation: NonZeroU64,
        ) -> Result<bool, PeerDenial> {
            if !self.available.load(Ordering::SeqCst) {
                return Err(PeerDenial::Unavailable);
            }
            Ok(app_id == self.app_id && self.generation.load(Ordering::SeqCst) == generation.get())
        }

        fn with_current_manifests(
            &self,
            requested: &[(u64, NonZeroU64)],
            inspect: &mut dyn FnMut(&[TrustedManifest]) -> Result<(), PeerDenial>,
        ) -> Result<(), PeerDenial> {
            if !self.available.load(Ordering::SeqCst) {
                return Err(PeerDenial::Unavailable);
            }
            let manifest = self.manifest.lock().map_err(|_| PeerDenial::Unavailable)?;
            let mut current = Vec::with_capacity(requested.len());
            for (app_id, generation) in requested {
                if *app_id != self.app_id
                    || self.generation.load(Ordering::SeqCst) != generation.get()
                {
                    return Err(PeerDenial::StaleManifest);
                }
                let value = manifest
                    .as_ref()
                    .filter(|manifest| {
                        manifest.app_id == *app_id && manifest.inventory_generation == *generation
                    })
                    .ok_or(PeerDenial::StaleManifest)?;
                current.push(value.clone());
            }
            inspect(&current)
        }
    }

    struct Sessions {
        lease: Mutex<Option<SessionLease>>,
        live: AtomicBool,
        app_id: u64,
        app_session_id: u64,
        user_session_generation: AtomicU64,
    }

    impl LiveSessionAuthority for Sessions {
        fn capture_live_unlocked(
            &self,
            app_id: u64,
            app_session_id: u64,
        ) -> Result<SessionLease, PeerDenial> {
            self.lease
                .lock()
                .map_err(|_| PeerDenial::Unavailable)?
                .filter(|lease| lease.app_id == app_id && lease.app_session_id == app_session_id)
                .ok_or(PeerDenial::SessionMismatch)
        }

        fn is_live_unlocked(&self, lease: SessionLease) -> Result<bool, PeerDenial> {
            Ok(self.live.load(Ordering::SeqCst)
                && lease.app_id == self.app_id
                && lease.app_session_id == self.app_session_id
                && lease.user_session_generation.get()
                    == self.user_session_generation.load(Ordering::SeqCst))
        }
    }

    /// Models only the callback-held gate contract used by C1. It is not the
    /// fixed C1 implementation and does not resolve that implementation's
    /// outstanding lifecycle/reentrancy review findings.
    struct C1SessionEffectGateFixture {
        lease: SessionLease,
        live: std::sync::RwLock<bool>,
        next_attempt: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    }

    impl C1SessionEffectGateFixture {
        fn new(lease: SessionLease) -> Self {
            Self {
                lease,
                live: std::sync::RwLock::new(true),
                next_attempt: Mutex::new(None),
            }
        }

        fn signal_next_attempt(&self, sender: std::sync::mpsc::Sender<()>) {
            *self.next_attempt.lock().unwrap() = Some(sender);
        }
    }

    impl BoundPeerSessionEffectGate for C1SessionEffectGateFixture {
        fn with_live_session_effect(
            &self,
            lease: SessionLease,
            effect: &mut dyn FnMut() -> Result<(), PeerDenial>,
        ) -> Result<(), PeerDenial> {
            if lease != self.lease {
                return Err(PeerDenial::SessionMismatch);
            }
            if let Some(sender) = self.next_attempt.lock().unwrap().take() {
                let _ = sender.send(());
            }
            let live = self.live.read().map_err(|_| PeerDenial::Unavailable)?;
            if !*live {
                return Err(PeerDenial::SessionRevoked);
            }
            effect()
        }
    }

    struct Consent {
        connected: AtomicBool,
    }
    impl ConsentSource for Consent {
        fn connected(&self) -> Result<bool, PeerDenial> {
            Ok(self.connected.load(Ordering::SeqCst))
        }
    }

    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    struct FakePidfd(u64);

    struct FakePidfdKernel {
        peer: Mutex<Option<FakePidfd>>,
        live: Mutex<HashMap<FakePidfd, bool>>,
        unavailable: AtomicBool,
        error_errno: AtomicI32,
        cleanup_errno: AtomicI32,
    }

    impl FakePidfdKernel {
        fn new() -> Self {
            Self {
                peer: Mutex::new(Some(FakePidfd(7001))),
                live: Mutex::new(HashMap::from([(FakePidfd(7001), true)])),
                unavailable: AtomicBool::new(false),
                error_errno: AtomicI32::new(0),
                cleanup_errno: AtomicI32::new(0),
            }
        }

        fn set_live(&self, handle: FakePidfd, live: bool) {
            self.live.lock().unwrap().insert(handle, live);
        }
    }

    impl PidfdKernel for FakePidfdKernel {
        type Handle = FakePidfd;

        fn terminate_and_reap_unregistered(
            &self,
            handle: &Self::Handle,
        ) -> Result<(), ProcessInstanceError> {
            let errno = self.cleanup_errno.load(Ordering::SeqCst);
            if errno != 0 {
                return Err(ProcessInstanceError::KernelRead {
                    option: pidfd_process_instance::ProcessInstanceOption::SpawnCleanup,
                    source: std::io::Error::from_raw_os_error(errno),
                });
            }
            self.set_live(*handle, false);
            Ok(())
        }

        fn peer_pidfd(&self, _stream: &UnixStream) -> Result<Self::Handle, ProcessInstanceError> {
            if self.unavailable.load(Ordering::SeqCst) {
                return Err(ProcessInstanceError::Unavailable);
            }
            let errno = self.error_errno.load(Ordering::SeqCst);
            if errno != 0 {
                return Err(ProcessInstanceError::KernelRead {
                    option: pidfd_process_instance::ProcessInstanceOption::PeerPidfd,
                    source: std::io::Error::from_raw_os_error(errno),
                });
            }
            self.peer
                .lock()
                .map_err(|_| ProcessInstanceError::Unavailable)?
                .ok_or(ProcessInstanceError::Unavailable)
        }

        fn confirms_same_live_instance(
            &self,
            spawned: &Self::Handle,
            peer: &Self::Handle,
            spawned_pid: u32,
            peer_pid: u32,
        ) -> Result<bool, ProcessInstanceError> {
            if spawned_pid != peer_pid {
                return Ok(false);
            }
            let live = self
                .live
                .lock()
                .map_err(|_| ProcessInstanceError::Unavailable)?;
            Ok(spawned == peer
                && live.get(spawned).copied() == Some(true)
                && live.get(peer).copied() == Some(true))
        }
    }

    struct FakePeerCredentialReader {
        pid: AtomicI32,
        unavailable: AtomicBool,
    }

    impl peer_credential_source_seal::Sealed for FakePeerCredentialReader {}

    impl PeerCredentialSource for FakePeerCredentialReader {
        fn read(&self, _stream: &UnixStream) -> Result<KernelPeerObservation, PeerDenial> {
            if self.unavailable.load(Ordering::SeqCst) {
                return Err(map_reader_error(PeerCredentialError::KernelRead {
                    option: SocketOption::PeerCredentials,
                    source: std::io::Error::from_raw_os_error(93),
                }));
            }
            Ok(KernelPeerObservation {
                pid: self.pid.load(Ordering::SeqCst),
                uid: 1000,
                gid: 1000,
                same_local_socket: true,
                authenticated_peer: None,
            })
        }
    }

    struct EndpointOwner {
        current: Mutex<TrustedServiceEndpoint>,
        recipient: EndpointCheckoutRecipient,
        replacement_recipient: Mutex<Option<EndpointCheckoutRecipient>>,
        current_enabled: AtomicBool,
        fd_delegation_blocked: AtomicBool,
        socket_cookie: AtomicU64,
        accepted: Mutex<std::collections::VecDeque<(TrustedServiceEndpoint, u64, UnixStream)>>,
    }

    struct FakeTask39StreamTransfer {
        stream: Option<UnixStream>,
        deadline: Instant,
    }

    impl Task39OwnedStreamTransfer for FakeTask39StreamTransfer {
        fn deadline(&self) -> Instant {
            self.deadline
        }

        fn consume_with_stream(
            mut self,
            receive: impl FnOnce(UnixStream, Instant) -> Result<(), PeerConnectionError>,
        ) -> Result<(), PeerDenial> {
            let stream = self.stream.take().ok_or(PeerDenial::StaleConnection)?;
            receive(stream, self.deadline)
                .map_err(|error| PeerDenial::PeerConnection(Arc::new(error)))
        }
    }

    impl endpoint_authority_seal::Sealed for EndpointOwner {}

    impl EndpointOwner {
        fn replace_recipient_for_test(&self) {
            *self.replacement_recipient.lock().unwrap() = Some(
                EndpointCheckoutRecipient::from_registered_owner(self.recipient.endpoint),
            );
        }

        fn current_recipient(&self) -> Result<EndpointCheckoutRecipient, PeerDenial> {
            Ok(self
                .replacement_recipient
                .lock()
                .map_err(|_| PeerDenial::Unavailable)?
                .clone()
                .unwrap_or_else(|| self.recipient.clone()))
        }
    }

    impl TrustedServiceEndpointAuthority for EndpointOwner {
        fn is_current(&self, endpoint: TrustedServiceEndpoint) -> Result<bool, PeerDenial> {
            Ok(self.current_enabled.load(Ordering::SeqCst)
                && *self.current.lock().map_err(|_| PeerDenial::Unavailable)? == endpoint)
        }

        fn checkout_recipient(
            &self,
            endpoint: TrustedServiceEndpoint,
        ) -> Result<EndpointCheckoutRecipient, PeerDenial> {
            if !self.is_current(endpoint)? || self.recipient.endpoint != endpoint {
                return Err(PeerDenial::StaleEndpoint);
            }
            Ok(self.current_recipient()?)
        }

        fn fd_delegation_is_prevented(
            &self,
            _endpoint: TrustedServiceEndpoint,
        ) -> Result<bool, PeerDenial> {
            Ok(self.fd_delegation_blocked.load(Ordering::SeqCst))
        }

        fn local_socket_cookie(&self, _stream: &UnixStream) -> Result<NonZeroU64, PeerDenial> {
            NonZeroU64::new(self.socket_cookie.load(Ordering::SeqCst))
                .ok_or(PeerDenial::StaleSocket)
        }

        fn accept_checkout(
            &self,
            endpoint: TrustedServiceEndpoint,
        ) -> Result<EndpointCheckout, PeerDenial> {
            self.accept_checkout_until(endpoint, Instant::now() + std::time::Duration::from_secs(5))
        }

        fn accept_checkout_until(
            &self,
            endpoint: TrustedServiceEndpoint,
            deadline: Instant,
        ) -> Result<EndpointCheckout, PeerDenial> {
            if Instant::now() >= deadline {
                return Err(PeerDenial::RequestDeadlineExpired);
            }
            let recipient = self.checkout_recipient(endpoint)?;
            let (accepted_endpoint, listener_socket_id, stream) = self
                .accepted
                .lock()
                .map_err(|_| PeerDenial::Unavailable)?
                .pop_front()
                .ok_or(PeerDenial::Unavailable)?;
            if accepted_endpoint != endpoint
                || listener_socket_id != endpoint.endpoint_socket_id.get()
            {
                return Err(PeerDenial::StaleEndpoint);
            }
            let cookie = self.local_socket_cookie(&stream)?;
            Ok(EndpointCheckout::from_registered_accept(
                recipient, deadline, cookie, stream,
            ))
        }
    }

    struct Supervisor {
        record: Mutex<Option<Record>>,
        grants: Mutex<Vec<(Vec<u8>, Result<(), PeerDenial>)>>,
    }

    impl SupervisorLaunchAuthority for Supervisor {
        type Record = Record;
        fn resolve(&self, process_id: u32) -> Option<Record> {
            self.record
                .lock()
                .ok()
                .and_then(|record| *record)
                .filter(|record| record.pid == process_id)
        }
        fn authorize_and_commit<T>(
            &self,
            record: Record,
            capabilities: &[&[u8]],
            commit: impl FnOnce() -> Result<T, PeerDenial>,
        ) -> Result<T, PeerDenial> {
            let current = self.record.lock().map_err(|_| PeerDenial::Unavailable)?;
            if *current != Some(record) {
                return Err(PeerDenial::NotLive);
            }
            let grants = self.grants.lock().map_err(|_| PeerDenial::Unavailable)?;
            for capability in capabilities {
                let result = grants
                    .iter()
                    .find(|(name, _)| name.as_slice() == *capability)
                    .map(|(_, result)| result.clone())
                    .unwrap_or(Err(PeerDenial::ConsentRequired));
                result.clone()?;
            }
            commit()
        }
    }

    struct Fixture {
        process: ProcessInstanceKey,
        record: Record,
        route: TrustedRouteKey,
        endpoint: TrustedServiceEndpoint,
        directory: Pid1LivePeerDirectory<Record>,
        run_owner_lease: Pid1RunOwnerLease,
        run: Pid1RunInstance,
        inventory: Inventory,
        sessions: Sessions,
        consent: Consent,
        process_instances: PidfdProcessInstanceAuthority<FakePidfdKernel>,
        peer_reader: FakePeerCredentialReader,
        endpoints: EndpointOwner,
        supervisor: Supervisor,
        readiness: FakeReadinessBoundary,
        reqs: [&'static [u8]; 1],
        consent_requirement: RouteConsentRequirement,
    }

    struct FakeCompletedHandshake {
        complete: AtomicBool,
    }

    struct FakeReadinessBoundary {
        lock: Mutex<()>,
        active: Arc<AtomicBool>,
        calls: AtomicU64,
        required_routes: AtomicU64,
        domain: Pid1AuthorityDomain,
    }

    impl FakeReadinessBoundary {
        fn new() -> Self {
            Self {
                lock: Mutex::new(()),
                active: Arc::new(AtomicBool::new(false)),
                calls: AtomicU64::new(0),
                required_routes: AtomicU64::new(0),
                domain: Pid1AuthorityDomain::new(),
            }
        }
    }

    impl readiness_boundary_seal::Sealed for FakeReadinessBoundary {}

    impl Pid1ReadinessAtomicBoundary for FakeReadinessBoundary {
        fn authority_domain(&self) -> &Pid1AuthorityDomain {
            &self.domain
        }

        fn with_owner_gate<T>(
            &self,
            _run: &Pid1RunInstance,
            required: Option<&RequiredServiceSet>,
            operation: impl FnOnce() -> Result<T, PeerDenial>,
        ) -> Result<T, PeerDenial> {
            let _guard = self.lock.try_lock().map_err(|_| PeerDenial::Unavailable)?;
            self.active.store(true, Ordering::SeqCst);
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.required_routes.store(
                required.map_or(0, |set| set.routes.len()) as u64,
                Ordering::SeqCst,
            );
            struct Reset<'a>(&'a AtomicBool);
            impl Drop for Reset<'_> {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _reset = Reset(&self.active);
            let result = operation();
            result
        }
    }

    struct FakeServiceOwner {
        state: Mutex<FakeServiceOwnerState>,
        gate_active: Arc<AtomicBool>,
        commits: AtomicU64,
        fail_after_callback_once: AtomicBool,
        report_in_progress_once: AtomicBool,
    }

    struct FakeServiceOwnerState {
        current: Option<ServiceRequestAdmission>,
        consumed_requests: HashSet<StableServiceRequestId>,
        request_keys: HashMap<DurableServiceRequestKey, ServiceOwnerRequestToken>,
    }

    impl FakeServiceOwner {
        fn new(current: ServiceRequestAdmission, gate: &FakeReadinessBoundary) -> Self {
            Self {
                state: Mutex::new(FakeServiceOwnerState {
                    current: Some(current),
                    consumed_requests: HashSet::new(),
                    request_keys: HashMap::new(),
                }),
                gate_active: Arc::clone(&gate.active),
                commits: AtomicU64::new(0),
                fail_after_callback_once: AtomicBool::new(false),
                report_in_progress_once: AtomicBool::new(false),
            }
        }

        fn fail_after_callback_once(&self) {
            self.fail_after_callback_once.store(true, Ordering::SeqCst);
        }

        fn report_in_progress_once(&self) {
            self.report_in_progress_once.store(true, Ordering::SeqCst);
        }

        fn revoke(
            &self,
            gate: &FakeReadinessBoundary,
            run: &Pid1RunInstance,
        ) -> Result<(), PeerDenial> {
            gate.with_owner_gate(run, None, || {
                self.state
                    .lock()
                    .map_err(|_| PeerDenial::Unavailable)?
                    .current = None;
                Ok(())
            })
        }
    }

    impl service_effect_commit_seal::Sealed for FakeServiceOwner {}

    impl ServiceOwnerEffectTransaction for FakeServiceOwner {
        fn durable_owner_id_for(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<ServiceEffectOwnerId, PeerDenial> {
            let state = self.state.try_lock().map_err(|_| PeerDenial::Unavailable)?;
            if state.current.as_ref() != Some(admission) {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            Ok(fixture_durable_owner_id())
        }

        fn admission_is_current(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<bool, PeerDenial> {
            if !self.gate_active.load(Ordering::SeqCst) {
                return Err(PeerDenial::Unavailable);
            }
            let state = self.state.try_lock().map_err(|_| PeerDenial::Unavailable)?;
            Ok(!state
                .consumed_requests
                .contains(&admission.stable_request_id())
                && state.current.as_ref() == Some(admission))
        }

        fn validate_request_with_identity(
            &self,
            admission: &ServiceRequestAdmission,
            owner: ServiceEffectOwnerId,
            identity: &DurableServiceRequestIdentity,
        ) -> Result<ServiceOwnerRequestToken, PeerDenial> {
            let state = self.state.try_lock().map_err(|_| PeerDenial::Unavailable)?;
            if state.current.as_ref() != Some(admission)
                || owner != fixture_durable_owner_id()
                || *identity != durable_request_identity_claim(owner, admission)
            {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            Ok(ServiceOwnerRequestToken::fixture(owner, admission))
        }

        fn commit_if_current_with_identity<T>(
            &self,
            admission: &ServiceRequestAdmission,
            identity: &ServiceOwnerRequestToken,
            commit: impl FnOnce() -> Result<T, PeerDenial>,
        ) -> Result<T, PeerDenial> {
            if !identity.matches_admission(fixture_durable_owner_id(), admission) {
                return Err(PeerDenial::ServiceEffectNotInvoked(Arc::new(
                    PeerDenial::RequestIdentityConflict,
                )));
            }
            if !self.gate_active.load(Ordering::SeqCst) {
                return Err(PeerDenial::ServiceEffectNotInvoked(Arc::new(
                    PeerDenial::Unavailable,
                )));
            }
            if self.report_in_progress_once.swap(false, Ordering::SeqCst) {
                return Err(PeerDenial::RequestInProgress);
            }
            let mut state = self.state.try_lock().map_err(|_| {
                PeerDenial::ServiceEffectNotInvoked(Arc::new(PeerDenial::Unavailable))
            })?;
            let key = identity.identity_claim.key();
            if state
                .request_keys
                .get(&key)
                .is_some_and(|stored| *stored != *identity)
            {
                return Err(PeerDenial::ServiceEffectNotInvoked(Arc::new(
                    PeerDenial::RequestIdentityConflict,
                )));
            }
            if state
                .consumed_requests
                .contains(&admission.stable_request_id())
            {
                return Err(PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(
                    PeerDenial::RequestOutcomeUnknown,
                )));
            }
            state.request_keys.insert(key, *identity);
            if state.current.as_ref() != Some(admission) {
                return Err(PeerDenial::ServiceEffectNotInvoked(Arc::new(
                    PeerDenial::StaleRegistration,
                )));
            }
            state
                .consumed_requests
                .insert(admission.stable_request_id());
            let result = match commit() {
                Ok(value) => value,
                Err(error @ PeerDenial::ServiceEffectFailed(_)) => return Err(error),
                Err(error) => return Err(PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(error))),
            };
            self.commits.fetch_add(1, Ordering::SeqCst);
            if self.fail_after_callback_once.swap(false, Ordering::SeqCst) {
                return Err(PeerDenial::DuplicateRequestCommit);
            }
            Ok(result)
        }
    }

    /// Test-only owner table whose resolver reads the state recorded by its
    /// own transaction. It does not model cross-process durability.
    struct ResolvingServiceOwner {
        admission: ServiceRequestAdmission,
        owner_identity: ServiceEffectOwnerId,
        durable_identity: ServiceOwnerRequestToken,
        gate_active: Arc<AtomicBool>,
        authoritative_state: Mutex<ServiceRequestOperationState>,
        lose_commit_reply_once: AtomicBool,
    }

    impl ResolvingServiceOwner {
        fn new(admission: ServiceRequestAdmission, gate: &FakeReadinessBoundary) -> Self {
            Self::new_with_owner_identity(admission, gate, fixture_durable_owner_id())
        }

        fn new_with_owner_identity(
            admission: ServiceRequestAdmission,
            gate: &FakeReadinessBoundary,
            owner_identity: ServiceEffectOwnerId,
        ) -> Self {
            let durable_identity = ServiceOwnerRequestToken::fixture(owner_identity, &admission);
            Self {
                admission,
                owner_identity,
                durable_identity,
                gate_active: Arc::clone(&gate.active),
                authoritative_state: Mutex::new(ServiceRequestOperationState::NotInvoked),
                lose_commit_reply_once: AtomicBool::new(false),
            }
        }

        fn lose_commit_reply_once(&self) {
            self.lose_commit_reply_once.store(true, Ordering::SeqCst);
        }

        fn set_authoritative_state(&self, state: ServiceRequestOperationState) {
            *self.authoritative_state.lock().unwrap() = state;
        }
    }

    impl service_effect_commit_seal::Sealed for ResolvingServiceOwner {}

    impl ServiceOwnerEffectTransaction for ResolvingServiceOwner {
        fn durable_owner_id_for(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<ServiceEffectOwnerId, PeerDenial> {
            if self.admission != *admission {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            Ok(self.owner_identity)
        }

        fn admission_is_current(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<bool, PeerDenial> {
            if !self.gate_active.load(Ordering::SeqCst) {
                return Err(PeerDenial::Unavailable);
            }
            let state = self
                .authoritative_state
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            Ok(self.admission == *admission && *state == ServiceRequestOperationState::NotInvoked)
        }

        fn validate_request_with_identity(
            &self,
            admission: &ServiceRequestAdmission,
            owner: ServiceEffectOwnerId,
            identity: &DurableServiceRequestIdentity,
        ) -> Result<ServiceOwnerRequestToken, PeerDenial> {
            if self.admission != *admission
                || self.owner_identity != owner
                || *identity != durable_request_identity_claim(owner, admission)
            {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            Ok(self.durable_identity)
        }

        fn commit_if_current_with_identity<T>(
            &self,
            admission: &ServiceRequestAdmission,
            identity: &ServiceOwnerRequestToken,
            commit: impl FnOnce() -> Result<T, PeerDenial>,
        ) -> Result<T, PeerDenial> {
            if !self.gate_active.load(Ordering::SeqCst) {
                return Err(PeerDenial::ServiceEffectNotInvoked(Arc::new(
                    PeerDenial::Unavailable,
                )));
            }
            if self.admission != *admission
                || identity != &self.durable_identity
                || !identity.matches_admission(self.owner_identity, admission)
            {
                return Err(PeerDenial::ServiceEffectNotInvoked(Arc::new(
                    PeerDenial::RequestIdentityConflict,
                )));
            }
            let mut state = self.authoritative_state.try_lock().map_err(|_| {
                PeerDenial::ServiceEffectNotInvoked(Arc::new(PeerDenial::Unavailable))
            })?;
            if *state != ServiceRequestOperationState::NotInvoked {
                return Err(PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(
                    PeerDenial::RequestOutcomeUnknown,
                )));
            }
            *state = ServiceRequestOperationState::Executing;
            match commit() {
                Ok(value) => {
                    *state = ServiceRequestOperationState::Executed;
                    if self.lose_commit_reply_once.swap(false, Ordering::SeqCst) {
                        Err(PeerDenial::DuplicateRequestCommit)
                    } else {
                        Ok(value)
                    }
                }
                Err(error @ PeerDenial::ServiceEffectFailed(_)) => {
                    *state = ServiceRequestOperationState::Failed;
                    Err(error)
                }
                Err(error) => {
                    *state = ServiceRequestOperationState::OutcomeUnknown;
                    Err(PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(error)))
                }
            }
        }

        fn resolve_request_state_with_identity(
            &self,
            admission: &ServiceRequestAdmission,
            identity: &ServiceOwnerRequestToken,
        ) -> Result<ServiceRequestOperationState, PeerDenial> {
            if self.admission != *admission
                || identity != &self.durable_identity
                || !identity.matches_admission(self.owner_identity, admission)
            {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            let state = self
                .authoritative_state
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?;
            Ok(*state)
        }
    }

    /// Test-only receipt owner that simulates an ambiguous record reply and
    /// then resolves it from an owner-side table bound to the full admission.
    struct ResolvingReceiptOwner {
        admission: ServiceRequestAdmission,
        owner_identity: ServiceEffectOwnerId,
        resolution_admission: Mutex<ServiceRequestAdmission>,
        gate_active: Arc<AtomicBool>,
        owner_state: Mutex<ServiceRequestOperationState>,
        record_calls: AtomicUsize,
        lookup_available: AtomicBool,
    }

    /// Test-only bridge that supplies the canonical owner ID already present
    /// on a fixture admission while delegating receipt creation and lookup to
    /// the real Task-25 local receipt candidate. This does not stand in for a
    /// production route owner or durable receipt resolver.
    struct FixtureTask25ReceiptTransaction<'authority, 'owner, 'run, Owner>
    where
        Owner: package_service_inventory_adapter::SessionPackageServiceOwner,
    {
        authority: &'authority package_service_inventory_adapter::SerializedPackageServiceAuthority<
            'owner,
            'run,
            Owner,
        >,
        admission: ServiceRequestAdmission,
        owner_id: ServiceEffectOwnerId,
    }

    impl<Owner> service_effect_commit_seal::Sealed
        for FixtureTask25ReceiptTransaction<'_, '_, '_, Owner>
    where
        Owner: package_service_inventory_adapter::SessionPackageServiceOwner,
    {
    }

    impl<Owner> ServiceRequestReceiptTransaction for FixtureTask25ReceiptTransaction<'_, '_, '_, Owner>
    where
        Owner: package_service_inventory_adapter::SessionPackageServiceOwner,
    {
        fn receipt_owner_id_for(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<ServiceEffectOwnerId, PeerDenial> {
            if *admission == self.admission {
                Ok(self.owner_id)
            } else {
                Err(PeerDenial::RequestIdentityConflict)
            }
        }

        fn record_receipt_if_current(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<GenerationBoundCommitReceipt, PeerDenial> {
            if *admission != self.admission {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            ServiceRequestReceiptTransaction::record_receipt_if_current(self.authority, admission)
        }

        fn resolve_receipt_state(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<ServiceRequestOperationState, PeerDenial> {
            if *admission != self.admission {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            ServiceRequestReceiptTransaction::resolve_receipt_state(self.authority, admission)
        }

        fn resolve_receipt_record(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<Option<GenerationBoundCommitReceipt>, PeerDenial> {
            if *admission != self.admission {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            ServiceRequestReceiptTransaction::resolve_receipt_record(self.authority, admission)
        }
    }

    impl ResolvingReceiptOwner {
        fn new(
            admission: ServiceRequestAdmission,
            gate: &FakeReadinessBoundary,
            owner_state: ServiceRequestOperationState,
        ) -> Self {
            Self::new_with_owner_identity(
                admission.clone(),
                gate,
                owner_state,
                admission.owner_identity,
            )
        }

        fn new_with_owner_identity(
            admission: ServiceRequestAdmission,
            gate: &FakeReadinessBoundary,
            owner_state: ServiceRequestOperationState,
            owner_identity: ServiceEffectOwnerId,
        ) -> Self {
            Self {
                admission: admission.clone(),
                owner_identity,
                resolution_admission: Mutex::new(admission),
                gate_active: Arc::clone(&gate.active),
                owner_state: Mutex::new(owner_state),
                record_calls: AtomicUsize::new(0),
                lookup_available: AtomicBool::new(true),
            }
        }

        fn resolve_as(&self, admission: ServiceRequestAdmission) {
            *self.resolution_admission.lock().unwrap() = admission;
        }

        fn disable_lookup(&self) {
            self.lookup_available.store(false, Ordering::SeqCst);
        }
    }

    impl service_effect_commit_seal::Sealed for ResolvingReceiptOwner {}

    impl ServiceRequestReceiptTransaction for ResolvingReceiptOwner {
        fn receipt_owner_id_for(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<ServiceEffectOwnerId, PeerDenial> {
            if self.admission != *admission {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            Ok(self.owner_identity)
        }

        fn record_receipt_if_current(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<GenerationBoundCommitReceipt, PeerDenial> {
            if self.admission != *admission {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            self.record_calls.fetch_add(1, Ordering::SeqCst);
            Err(PeerDenial::Unavailable)
        }

        fn resolve_receipt_state(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<ServiceRequestOperationState, PeerDenial> {
            if !self.gate_active.load(Ordering::SeqCst) {
                return Err(PeerDenial::Unavailable);
            }
            if !self.lookup_available.load(Ordering::SeqCst) {
                return Err(PeerDenial::Unavailable);
            }
            if *self
                .resolution_admission
                .try_lock()
                .map_err(|_| PeerDenial::Unavailable)?
                != *admission
            {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            self.owner_state
                .try_lock()
                .map(|state| *state)
                .map_err(|_| PeerDenial::Unavailable)
        }
    }

    struct FakeSessionOwnedConsent {
        current: Mutex<ConsentEventRevision>,
        expires_at: Mutex<Option<Instant>>,
    }

    impl FakeSessionOwnedConsent {
        fn new() -> Self {
            Self {
                current: Mutex::new(
                    ConsentEventRevision::from_live_session_owner(91, 7, 3)
                        .expect("nonzero test event identity and revision"),
                ),
                expires_at: Mutex::new(None),
            }
        }

        fn expire_current_event(&self, readiness: &FakeReadinessBoundary, run: &Pid1RunInstance) {
            readiness
                .with_owner_gate(run, None, || {
                    *self
                        .expires_at
                        .try_lock()
                        .map_err(|_| PeerDenial::ConsentUnavailable)? =
                        Some(Instant::now() - std::time::Duration::from_secs(1));
                    Ok(())
                })
                .unwrap();
        }

        fn advance_revision(&self, readiness: &FakeReadinessBoundary, run: &Pid1RunInstance) {
            readiness
                .with_owner_gate(run, None, || {
                    let mut current = self
                        .current
                        .try_lock()
                        .map_err(|_| PeerDenial::ConsentUnavailable)?;
                    *current = ConsentEventRevision::from_live_session_owner(
                        current.source_owner_id().get(),
                        current.event_id().get(),
                        current.revision().get() + 1,
                    )
                    .ok_or(PeerDenial::GenerationExhausted)?;
                    Ok(())
                })
                .unwrap();
        }
    }

    impl session_consent_authority_seal::Sealed for FakeSessionOwnedConsent {}

    impl SessionOwnedConsentAuthority for FakeSessionOwnedConsent {
        fn issue_event(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<SessionConsentGrantEvent, PeerDenial> {
            let event = *self
                .current
                .try_lock()
                .map_err(|_| PeerDenial::ConsentUnavailable)?;
            Ok(SessionConsentGrantEvent::from_live_session_owner(
                admission, event,
            ))
        }

        fn is_current_unlocked(
            &self,
            event: &SessionConsentGrantEvent,
        ) -> Result<bool, PeerDenial> {
            let current = *self
                .current
                .try_lock()
                .map_err(|_| PeerDenial::ConsentUnavailable)?;
            if current != event.event_revision() {
                return Ok(false);
            }
            let expires_at = *self
                .expires_at
                .try_lock()
                .map_err(|_| PeerDenial::ConsentUnavailable)?;
            Ok(expires_at.is_none_or(|deadline| Instant::now() < deadline))
        }
    }

    impl handshake_authority_seal::Sealed for FakeCompletedHandshake {}

    impl CompletedServiceHandshakeAuthority for FakeCompletedHandshake {
        fn confirms_completed_handshake(
            &self,
            _run: &Pid1RunInstance,
            _registration: RegistrationId,
            _process: ProcessInstanceKey,
            _route: TrustedRouteKey,
            _endpoint: TrustedServiceEndpoint,
        ) -> Result<bool, PeerDenial> {
            Ok(self.complete.load(Ordering::SeqCst))
        }
    }

    impl Fixture {
        fn new() -> Self {
            Self::new_with_consent_requirement(RouteConsentRequirement::NotRequired)
        }

        fn new_with_consent_requirement(consent_requirement: RouteConsentRequirement) -> Self {
            Self::new_with_route_and_consent_requirement(
                TrustedRouteKey::from_canonical_route(10, 20, 1, 0, 4).unwrap(),
                consent_requirement,
            )
        }

        fn new_with_route_and_consent_requirement(
            route: TrustedRouteKey,
            consent_requirement: RouteConsentRequirement,
        ) -> Self {
            Self::new_with_authority_generations(route, consent_requirement, 3, 88)
        }

        fn new_with_authority_generations(
            route: TrustedRouteKey,
            consent_requirement: RouteConsentRequirement,
            inventory_generation: u64,
            session_generation: u64,
        ) -> Self {
            Self::new_with_application_generations(
                route,
                consent_requirement,
                inventory_generation,
                session_generation,
                9,
            )
        }

        fn new_with_application_generations(
            route: TrustedRouteKey,
            consent_requirement: RouteConsentRequirement,
            inventory_generation: u64,
            session_generation: u64,
            app_id: u64,
        ) -> Self {
            let kernel = FakePidfdKernel::new();
            let process_instances = PidfdProcessInstanceAuthority::new(kernel);
            let process = process_instances
                .record_test_spawn_handle(41, FakePidfd(7001))
                .unwrap();
            let record = Record {
                pid: 41,
                app: app_id,
                session: 12,
            };
            let endpoint = TrustedServiceEndpoint::from_service_registry(route, 7, 8001).unwrap();
            let manifest = TrustedManifest::from_verified_installed_manifest(
                record.app,
                inventory_generation,
                [b"files.read".to_vec()],
            )
            .unwrap();
            let lease = SessionLease::from_live_session_authority(
                record.app,
                record.session,
                session_generation,
            )
            .unwrap();
            let directory = Pid1LivePeerDirectory::new();
            let run_owner_lease = directory.begin_run_owner_lease().unwrap();
            let run = run_owner_lease.run().clone();
            Self {
                process,
                record,
                route,
                endpoint,
                directory,
                run_owner_lease,
                run,
                inventory: Inventory {
                    manifest: Mutex::new(Some(manifest)),
                    available: AtomicBool::new(true),
                    app_id: record.app,
                    generation: AtomicU64::new(inventory_generation),
                },
                sessions: Sessions {
                    lease: Mutex::new(Some(lease)),
                    live: AtomicBool::new(true),
                    app_id: record.app,
                    app_session_id: record.session,
                    user_session_generation: AtomicU64::new(session_generation),
                },
                consent: Consent {
                    connected: AtomicBool::new(true),
                },
                process_instances,
                peer_reader: FakePeerCredentialReader {
                    pid: AtomicI32::new(41),
                    unavailable: AtomicBool::new(false),
                },
                endpoints: EndpointOwner {
                    current: Mutex::new(endpoint),
                    recipient: EndpointCheckoutRecipient::from_registered_owner(endpoint),
                    replacement_recipient: Mutex::new(None),
                    current_enabled: AtomicBool::new(true),
                    fd_delegation_blocked: AtomicBool::new(true),
                    socket_cookie: AtomicU64::new(9001),
                    accepted: Mutex::new(std::collections::VecDeque::new()),
                },
                supervisor: Supervisor {
                    record: Mutex::new(Some(record)),
                    grants: Mutex::new(vec![(b"files.read".to_vec(), Ok(()))]),
                },
                readiness: FakeReadinessBoundary::new(),
                reqs: [b"files.read"],
                consent_requirement,
            }
        }

        fn requirements(&self) -> TrustedServiceRequirements<'_> {
            TrustedServiceRequirements::from_canonical_route(
                self.route,
                &self.reqs,
                self.consent_requirement,
            )
        }

        fn register(&self) -> RegistrationId {
            let spawn = self.spawn_proof().unwrap();
            self.register_proof(spawn, &self.requirements(), self.endpoint)
                .unwrap()
        }

        fn spawn_proof(&self) -> Result<Pid1SpawnProof<Record>, PeerDenial> {
            self.process_instances
                .issue_spawn_proof(self.process, self.record)
                .and_then(|proof| {
                    proof.attach_task24_test_identity(nagi_init_spawn_contract::test_identity(
                        self.record.pid,
                    ))
                })
        }

        fn register_proof(
            &self,
            spawn: Pid1SpawnProof<Record>,
            requirements: &TrustedServiceRequirements<'_>,
            endpoint: TrustedServiceEndpoint,
        ) -> Result<RegistrationId, PeerDenial> {
            self.directory.register_pid1_spawn(
                &self.run,
                spawn,
                requirements,
                endpoint,
                &self.inventory,
                &self.sessions,
                &self.consent,
                &self.endpoints,
                &self.supervisor,
                &self.readiness,
            )
        }

        fn authorize(
            &self,
            peer_pid: u32,
            id: RegistrationId,
        ) -> Result<AuthorizedPeer<'_, Record>, PeerDenial> {
            self.peer_reader
                .pid
                .store(peer_pid as i32, Ordering::SeqCst);
            let (stream, _peer) = UnixStream::pair().unwrap();
            self.endpoints
                .accepted
                .lock()
                .map_err(|_| PeerDenial::Unavailable)?
                .push_back((
                    self.endpoint,
                    self.endpoint.endpoint_socket_id.get(),
                    stream,
                ));
            self.accept_authorize_until(self.endpoint, id, Instant::now() + Duration::from_secs(1))
        }

        fn accept_authorize_until(
            &self,
            endpoint: TrustedServiceEndpoint,
            id: RegistrationId,
            deadline: Instant,
        ) -> Result<AuthorizedPeer<'_, Record>, PeerDenial> {
            self.directory.accept_and_authorize_peer_until(
                &self.peer_reader,
                endpoint,
                id,
                &self.requirements(),
                deadline,
                &self.inventory,
                &self.sessions,
                &self.consent,
                &self.endpoints,
                &self.process_instances,
                &self.supervisor,
                &self.readiness,
            )
        }

        fn checkout(
            &self,
            endpoint: TrustedServiceEndpoint,
            stream: UnixStream,
        ) -> Result<EndpointCheckout, PeerDenial> {
            self.endpoints
                .accepted
                .lock()
                .map_err(|_| PeerDenial::Unavailable)?
                .push_back((endpoint, endpoint.endpoint_socket_id.get(), stream));
            self.endpoints.accept_checkout(endpoint)
        }

        fn authorize_checkout(
            &self,
            checkout: EndpointCheckout,
            id: RegistrationId,
            requirements: &TrustedServiceRequirements<'_>,
        ) -> Result<AuthorizedPeer<'_, Record>, PeerDenial> {
            self.authorize_checkout_with(checkout, id, requirements, &self.process_instances)
        }

        fn authorize_checkout_with<'a, P: PeerProcessInstanceAuthority>(
            &'a self,
            checkout: EndpointCheckout,
            id: RegistrationId,
            requirements: &TrustedServiceRequirements<'_>,
            process_instances: &'a P,
        ) -> Result<AuthorizedPeer<'a, Record>, PeerDenial> {
            self.directory.authorize_peer(
                &self.peer_reader,
                checkout,
                id,
                requirements,
                &self.inventory,
                &self.sessions,
                &self.consent,
                &self.endpoints,
                process_instances,
                &self.supervisor,
                &self.readiness,
            )
        }

        fn fake_peer_pidfd(&self, handle: FakePidfd, live: bool) {
            self.process_instances.kernel().set_live(handle, live);
            *self.process_instances.kernel().peer.lock().unwrap() = Some(handle);
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn task39_reader_unavailability_stays_unavailable_through_adapter() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        let error = read_peer_observation(&stream).unwrap_err();
        assert!(matches!(
            error,
            PeerDenial::KernelPeerAuthentication(source)
                if matches!(&*source, PeerAuthenticationError::Unavailable)
        ));
    }

    #[test]
    fn registration_rejects_candidate_pidfd_proof_without_task24_spawn_identity() {
        let fixture = Fixture::new();
        let proof = fixture
            .process_instances
            .issue_spawn_proof(fixture.process, fixture.record)
            .expect("candidate pidfd proof");
        let result = fixture.directory.register_pid1_spawn(
            &fixture.run,
            proof,
            &fixture.requirements(),
            fixture.endpoint,
            &fixture.inventory,
            &fixture.sessions,
            &fixture.consent,
            &fixture.endpoints,
            &fixture.supervisor,
            &fixture.readiness,
        );
        assert_denial!(result, PeerDenial::ForeignSpawnOwner);
        assert!(fixture.directory.state.lock().unwrap().entries.is_empty());
    }

    #[test]
    fn task24_spawn_identity_cannot_be_overwritten_and_is_retained_in_entry() {
        let overwrite_fixture = Fixture::new();
        let first = nagi_init_spawn_contract::test_identity(overwrite_fixture.record.pid);
        let same_pid_new_instance =
            nagi_init_spawn_contract::test_identity(overwrite_fixture.record.pid);
        let proof = overwrite_fixture
            .process_instances
            .issue_spawn_proof(overwrite_fixture.process, overwrite_fixture.record)
            .unwrap()
            .attach_task24_test_identity(first)
            .unwrap();
        assert!(matches!(
            proof.attach_task24_test_identity(same_pid_new_instance),
            Err(PeerDenial::ForeignSpawnOwner)
        ));

        let registration_fixture = Fixture::new();
        let identity = nagi_init_spawn_contract::test_identity(registration_fixture.record.pid);
        let proof = registration_fixture
            .process_instances
            .issue_spawn_proof(registration_fixture.process, registration_fixture.record)
            .unwrap()
            .attach_task24_test_identity(identity.clone())
            .unwrap();
        let registration = registration_fixture
            .register_proof(
                proof,
                &registration_fixture.requirements(),
                registration_fixture.endpoint,
            )
            .unwrap();
        let entry = registration_fixture
            .directory
            .snapshot(registration)
            .unwrap();
        assert!(entry.task24_identity.same_instance(&identity));
    }

    #[test]
    fn durable_authority_fingerprint_uses_fresh_run_nonce() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 61), b"files.read")
            .unwrap();

        let restarted_directory = Pid1LivePeerDirectory::<Record>::new();
        let restarted_run = restarted_directory.begin_run().unwrap();
        assert_eq!(admission.run.generation(), restarted_run.generation());
        let restarted_admission = ServiceRequestAdmission {
            run: restarted_run,
            ..admission.clone()
        };
        assert_eq!(
            request_route_fingerprint(&admission),
            request_route_fingerprint(&restarted_admission)
        );
        assert_ne!(
            request_authority_fingerprint(&admission),
            request_authority_fingerprint(&restarted_admission),
            "same counters and process selector across PID1 runs must not collide"
        );
    }

    #[test]
    fn ending_run_clears_request_ledger_without_making_inflight_request_retryable() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 62), b"files.read")
            .unwrap();
        admission.operation_state.store(
            ServiceRequestOperationState::Executing as u8,
            Ordering::SeqCst,
        );
        assert_eq!(
            fixture
                .directory
                .request_ledger
                .lock()
                .unwrap()
                .entries
                .len(),
            1
        );

        fixture
            .directory
            .end_run_owner_lease(&fixture.run_owner_lease, &fixture.readiness)
            .unwrap();
        assert!(!admission.run().is_active());
        assert!(fixture
            .directory
            .request_ledger
            .lock()
            .unwrap()
            .entries
            .is_empty());
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown,
            "an in-progress request is never reset to NotInvoked during cleanup"
        );

        let owner = ResolvingServiceOwner::new(admission.clone(), &fixture.readiness);
        let effects = AtomicUsize::new(0);
        assert!(matches!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(PeerDenial::ServiceEffectOutcomeUnknown(_))
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        assert_denial!(
            peer.admit_request(complete_request_frame(&peer, 62), b"files.read"),
            PeerDenial::StaleRun
        );
        assert!(fixture
            .directory
            .request_ledger
            .lock()
            .unwrap()
            .entries
            .is_empty());
        assert!(fixture.directory.begin_run_owner_lease().is_ok());
    }

    #[test]
    fn ready_event_requires_live_registry_and_completed_handshake() {
        let fixture = Fixture::new();
        let required = RequiredServiceSet::from_canonical_service_plan([fixture.route]).unwrap();
        let handshakes = FakeCompletedHandshake {
            complete: AtomicBool::new(false),
        };

        assert_denial!(
            fixture.directory.ready_event(
                &fixture.run_owner_lease,
                &required,
                &fixture.inventory,
                &fixture.sessions,
                &fixture.consent,
                &fixture.endpoints,
                &fixture.process_instances,
                &handshakes,
                &fixture.supervisor,
                &fixture.readiness,
            ),
            PeerDenial::RequiredServiceMissing
        );

        let registration = fixture.register();
        assert_denial!(
            fixture.directory.ready_event(
                &fixture.run_owner_lease,
                &required,
                &fixture.inventory,
                &fixture.sessions,
                &fixture.consent,
                &fixture.endpoints,
                &fixture.process_instances,
                &handshakes,
                &fixture.supervisor,
                &fixture.readiness,
            ),
            PeerDenial::HandshakeIncomplete
        );

        handshakes.complete.store(true, Ordering::SeqCst);
        let ready = fixture
            .directory
            .ready_event(
                &fixture.run_owner_lease,
                &required,
                &fixture.inventory,
                &fixture.sessions,
                &fixture.consent,
                &fixture.endpoints,
                &fixture.process_instances,
                &handshakes,
                &fixture.supervisor,
                &fixture.readiness,
            )
            .unwrap();
        assert!(ready.run().is_same_run(&fixture.run));
        assert_eq!(ready.services().len(), 1);
        assert_eq!(ready.services()[0].registration(), registration);
        assert_eq!(ready.services()[0].process(), fixture.process);
        assert_eq!(ready.services()[0].route(), fixture.route);
        assert_eq!(ready.services()[0].endpoint(), fixture.endpoint);
        ready.validate_current().unwrap();
        let mapped = ready
            .with_current(|event| {
                assert!(fixture.readiness.active.load(Ordering::SeqCst));
                assert_eq!(event.services().len(), 1);
                assert_eq!(fixture.readiness.required_routes.load(Ordering::SeqCst), 1);
                assert!(matches!(
                    fixture.readiness.lock.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ));
                event.services().len()
            })
            .unwrap();
        assert_eq!(mapped, 1);
        fixture.consent.connected.store(false, Ordering::SeqCst);
        assert!(matches!(
            ready.validate_current(),
            Err(PeerDenial::ConsentUnavailable)
        ));
    }

    #[test]
    fn ready_event_rejects_foreign_run_and_revoked_session() {
        let fixture = Fixture::new();
        let _registration = fixture.register();
        let required = RequiredServiceSet::from_canonical_service_plan([fixture.route]).unwrap();
        let handshakes = FakeCompletedHandshake {
            complete: AtomicBool::new(true),
        };
        let foreign_directory = Pid1LivePeerDirectory::<Record>::new();
        let foreign_run = foreign_directory.begin_run_owner_lease().unwrap();
        assert_denial!(
            fixture.directory.ready_event(
                &foreign_run,
                &required,
                &fixture.inventory,
                &fixture.sessions,
                &fixture.consent,
                &fixture.endpoints,
                &fixture.process_instances,
                &handshakes,
                &fixture.supervisor,
                &fixture.readiness,
            ),
            PeerDenial::StaleRun
        );

        fixture.sessions.live.store(false, Ordering::SeqCst);
        assert_denial!(
            fixture.directory.ready_event(
                &fixture.run_owner_lease,
                &required,
                &fixture.inventory,
                &fixture.sessions,
                &fixture.consent,
                &fixture.endpoints,
                &fixture.process_instances,
                &handshakes,
                &fixture.supervisor,
                &fixture.readiness,
            ),
            PeerDenial::SessionRevoked
        );
    }

    #[test]
    fn ready_event_rejects_partial_multi_route_set() {
        let fixture = Fixture::new();
        fixture.register();
        let second_route = TrustedRouteKey::from_canonical_route(11, 21, 1, 0, 4).unwrap();
        let required =
            RequiredServiceSet::from_canonical_service_plan([fixture.route, second_route]).unwrap();
        let handshakes = FakeCompletedHandshake {
            complete: AtomicBool::new(true),
        };
        assert_denial!(
            fixture.directory.ready_event(
                &fixture.run_owner_lease,
                &required,
                &fixture.inventory,
                &fixture.sessions,
                &fixture.consent,
                &fixture.endpoints,
                &fixture.process_instances,
                &handshakes,
                &fixture.supervisor,
                &fixture.readiness,
            ),
            PeerDenial::RequiredServiceMissing
        );
    }

    #[test]
    fn ready_event_rechecks_post_issue_handshake_session_inventory_and_cleanup() {
        let fixture = Fixture::new();
        fixture.register();
        let required = RequiredServiceSet::from_canonical_service_plan([fixture.route]).unwrap();
        let handshakes = FakeCompletedHandshake {
            complete: AtomicBool::new(true),
        };
        let ready = fixture
            .directory
            .ready_event(
                &fixture.run_owner_lease,
                &required,
                &fixture.inventory,
                &fixture.sessions,
                &fixture.consent,
                &fixture.endpoints,
                &fixture.process_instances,
                &handshakes,
                &fixture.supervisor,
                &fixture.readiness,
            )
            .unwrap();

        handshakes.complete.store(false, Ordering::SeqCst);
        let mut mapped = false;
        assert_denial!(
            ready.with_current(|_| mapped = true),
            PeerDenial::HandshakeIncomplete
        );
        assert!(
            !mapped,
            "readiness mapping must not run after handshake revoke"
        );
        handshakes.complete.store(true, Ordering::SeqCst);

        fixture.sessions.live.store(false, Ordering::SeqCst);
        assert_denial!(ready.validate_current(), PeerDenial::SessionRevoked);
        fixture.sessions.live.store(true, Ordering::SeqCst);

        fixture.inventory.generation.store(4, Ordering::SeqCst);
        assert_denial!(ready.validate_current(), PeerDenial::StaleManifest);
        fixture.inventory.generation.store(3, Ordering::SeqCst);

        fixture
            .process_instances
            .inject_test_cleanup_failure(fixture.process)
            .unwrap();
        assert_denial!(ready.validate_current(), PeerDenial::UnresolvedSpawnCleanup);
    }

    #[test]
    fn lifecycle_ready_adapter_requires_and_tracks_run_owner_lease() {
        let fixture = Fixture::new();
        fixture.register();
        let required = RequiredServiceSet::from_canonical_service_plan([fixture.route]).unwrap();
        let handshakes = FakeCompletedHandshake {
            complete: AtomicBool::new(true),
        };
        let ready = fixture
            .directory
            .ready_event(
                &fixture.run_owner_lease,
                &required,
                &fixture.inventory,
                &fixture.sessions,
                &fixture.consent,
                &fixture.endpoints,
                &fixture.process_instances,
                &handshakes,
                &fixture.supervisor,
                &fixture.readiness,
            )
            .unwrap();
        let adapter = Pid1LifecycleReadyAdapter;
        let mut mapped = false;

        assert_denial!(
            adapter.map_current(None, &ready, |_| mapped = true),
            PeerDenial::Unavailable
        );
        assert!(!mapped, "missing owner lease must not reach lifecycle map");
        adapter
            .map_current(Some(&fixture.run_owner_lease), &ready, |event| {
                mapped = true;
                event.services().len()
            })
            .unwrap();
        assert!(mapped);

        fixture
            .directory
            .end_run_owner_lease(&fixture.run_owner_lease, &fixture.readiness)
            .unwrap();
        assert!(!fixture.run_owner_lease.is_active());
        let mut mapped_after_exit = false;
        assert_denial!(
            adapter.map_current(Some(&fixture.run_owner_lease), &ready, |_| {
                mapped_after_exit = true;
            }),
            PeerDenial::StaleRun
        );
        assert!(!mapped_after_exit);

        let next_run_lease = fixture.directory.begin_run_owner_lease().unwrap();
        assert!(!next_run_lease.run().is_same_run(ready.run()));
        let mut mapped_old_event = false;
        assert_denial!(
            adapter.map_current(Some(&next_run_lease), &ready, |_| {
                mapped_old_event = true;
            }),
            PeerDenial::StaleRun
        );
        assert!(!mapped_old_event, "old run event cannot map into a new run");
    }

    #[test]
    fn reader_syscall_and_errno_survive_adapter_denial() {
        let denial = map_reader_error(PeerCredentialError::KernelRead {
            option: SocketOption::SocketCookie,
            source: std::io::Error::from_raw_os_error(4),
        });
        let PeerDenial::KernelReader(source) = denial else {
            panic!("reader error was flattened");
        };
        let PeerCredentialError::KernelRead { option, source: io } = &*source else {
            panic!("typed socket-option error was lost");
        };
        assert_eq!(*option, SocketOption::SocketCookie);
        assert_eq!(io.raw_os_error(), Some(4));
    }

    #[test]
    fn task39_typed_reader_error_survives_public_checkout_path() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        fixture
            .peer_reader
            .unavailable
            .store(true, Ordering::SeqCst);
        let denial = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap_err();
        let PeerDenial::KernelReader(source) = denial else {
            panic!("task-39 reader error was not retained by the public adapter");
        };
        let PeerCredentialError::KernelRead { option, source } = &*source else {
            panic!("task-39 typed error was flattened");
        };
        assert_eq!(*option, SocketOption::PeerCredentials);
        assert_eq!(source.raw_os_error(), Some(93));
    }

    #[test]
    fn missing_trusted_inventory_fails_closed() {
        let fixture = Fixture::new();
        fixture.inventory.available.store(false, Ordering::SeqCst);
        let spawn = fixture.spawn_proof().unwrap();
        assert_denial!(
            fixture.register_proof(spawn, &fixture.requirements(), fixture.endpoint),
            PeerDenial::Unavailable
        );
    }

    #[test]
    fn missing_consent_source_fails_closed() {
        let fixture = Fixture::new_with_consent_requirement(RouteConsentRequirement::Required);
        fixture.consent.connected.store(false, Ordering::SeqCst);
        let spawn = fixture.spawn_proof().unwrap();
        assert_denial!(
            fixture.directory.register_pid1_spawn_pre_session(
                &fixture.run,
                spawn,
                &fixture.requirements(),
                fixture.endpoint,
                &fixture.inventory,
                &fixture.consent,
                &fixture.endpoints,
                &fixture.supervisor,
                &fixture.readiness,
            ),
            PeerDenial::ConsentUnavailable
        );
    }

    #[test]
    fn not_required_pre_session_registration_does_not_need_connected_consent() {
        let fixture = Fixture::new();
        fixture.consent.connected.store(false, Ordering::SeqCst);
        let spawn = fixture.spawn_proof().unwrap();

        let registration = fixture
            .directory
            .register_pid1_spawn_pre_session(
                &fixture.run,
                spawn,
                &fixture.requirements(),
                fixture.endpoint,
                &fixture.inventory,
                &fixture.consent,
                &fixture.endpoints,
                &fixture.supervisor,
                &fixture.readiness,
            )
            .expect("NotRequired route has no dependency on a user-consent UI");

        let entry = fixture.directory.snapshot(registration).unwrap();
        assert!(entry.session.is_none());
        assert!(entry.system_service_owner.is_none());
    }

    #[test]
    fn undeclared_capability_is_rejected_before_registration() {
        let fixture = Fixture::new();
        let undeclared = [b"network.connect" as &[u8]];
        let requirements = TrustedServiceRequirements::from_canonical_route(
            fixture.route,
            &undeclared,
            fixture.consent_requirement,
        );
        let spawn = fixture.spawn_proof().unwrap();
        assert_denial!(
            fixture.register_proof(spawn, &requirements, fixture.endpoint),
            PeerDenial::NotDeclared
        );
    }

    #[test]
    fn consent_required_is_not_promoted_to_granted() {
        let fixture = Fixture::new();
        *fixture.supervisor.grants.lock().unwrap() = Vec::new();
        let spawn = fixture.spawn_proof().unwrap();
        assert_denial!(
            fixture.register_proof(spawn, &fixture.requirements(), fixture.endpoint),
            PeerDenial::ConsentRequired
        );
    }

    #[test]
    fn app_session_mismatch_is_rejected() {
        let fixture = Fixture::new();
        *fixture.sessions.lease.lock().unwrap() = Some(
            SessionLease::from_live_session_authority(
                fixture.record.app,
                fixture.record.session + 1,
                89,
            )
            .unwrap(),
        );
        let spawn = fixture.spawn_proof().unwrap();
        assert_denial!(
            fixture.register_proof(spawn, &fixture.requirements(), fixture.endpoint),
            PeerDenial::SessionMismatch
        );
    }

    #[test]
    fn exited_launch_is_rejected_at_checkout() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        *fixture.supervisor.record.lock().unwrap() = None;
        assert_denial!(
            fixture.authorize(fixture.process.pid(), registration),
            PeerDenial::NotLive
        );
    }

    #[test]
    fn reused_pid_requires_separate_process_instance_proof() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        // The old peer still presents the same kernel-reported PID. PID and
        // SO_COOKIE alone cannot establish that it is the newly spawned child.
        fixture.fake_peer_pidfd(FakePidfd(7002), true);
        assert_denial!(
            fixture.authorize(fixture.process.pid(), registration),
            PeerDenial::StalePeer
        );
    }

    #[test]
    fn exited_peer_pidfd_rejects_a_reused_numeric_pid() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        // This simulates a stale connection snapshot from the old process after
        // its PID was reused by the currently registered child.
        fixture.fake_peer_pidfd(FakePidfd(7000), false);
        assert_denial!(
            fixture.authorize(fixture.process.pid(), registration),
            PeerDenial::StalePeer
        );
    }

    #[test]
    fn exited_spawn_pidfd_is_rejected_even_before_registry_cleanup() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        fixture
            .process_instances
            .kernel()
            .set_live(FakePidfd(7001), false);
        assert_denial!(
            fixture.authorize(fixture.process.pid(), registration),
            PeerDenial::StalePeer
        );
    }

    #[test]
    fn pidfd_acquisition_failure_is_unavailable_and_preserves_errno() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        fixture
            .process_instances
            .kernel()
            .error_errno
            .store(93, Ordering::SeqCst);
        let denial = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap_err();
        let PeerDenial::KernelProcessInstance(source) = denial else {
            panic!("pidfd acquisition error was promoted or flattened");
        };
        let ProcessInstanceError::KernelRead { option, source } = &*source else {
            panic!("typed pidfd error was not preserved");
        };
        assert_eq!(
            *option,
            pidfd_process_instance::ProcessInstanceOption::PeerPidfd
        );
        assert_eq!(source.raw_os_error(), Some(93));
    }

    #[test]
    fn missing_peer_pidfd_is_unavailable() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        fixture
            .process_instances
            .kernel()
            .unavailable
            .store(true, Ordering::SeqCst);
        assert_denial!(
            fixture.authorize(fixture.process.pid(), registration),
            PeerDenial::ProcessInstanceUnavailable
        );
    }

    #[test]
    fn missing_spawn_pidfd_fails_at_pid1_capture() {
        let kernel = FakePidfdKernel::new();
        kernel.set_live(FakePidfd(7002), false);
        let authority = PidfdProcessInstanceAuthority::new(kernel);
        assert_denial!(
            authority.record_test_spawn_handle(42, FakePidfd(7002)),
            PeerDenial::ProcessInstanceUnavailable
        );
    }

    #[test]
    fn spawn_owner_binds_exact_child_record_and_trusted_route_once() {
        let fixture = Fixture::new();
        let trusted_route = test_nipc_route(fixture.route, fixture.record.app, "files.read@1");

        assert!(matches!(
            fixture.process_instances.issue_spawn_proof(
                fixture.process,
                Record {
                    pid: fixture.record.pid + 1,
                    ..fixture.record
                },
            ),
            Err(PeerDenial::InvalidProcess)
        ));

        let mismatched_supervisor = fixture
            .process_instances
            .issue_spawn_proof(fixture.process, fixture.record)
            .expect("opaque proof for the test-owned child");
        *fixture.supervisor.record.lock().unwrap() = None;
        assert_denial!(
            fixture.process_instances.bind_spawn_owned_endpoint(
                mismatched_supervisor,
                trusted_route.clone(),
                (),
                &fixture.supervisor,
            ),
            PeerDenial::NotLive
        );

        let stale_fixture = Fixture::new();
        let stale_route = test_nipc_route(
            stale_fixture.route,
            stale_fixture.record.app,
            "files.read@1",
        );
        let stale_proof = stale_fixture
            .process_instances
            .issue_spawn_proof(stale_fixture.process, stale_fixture.record)
            .expect("proof before simulated exit");
        stale_fixture
            .process_instances
            .kernel()
            .set_live(FakePidfd(7001), false);
        assert_denial!(
            stale_fixture.process_instances.bind_spawn_owned_endpoint(
                stale_proof,
                stale_route,
                (),
                &stale_fixture.supervisor,
            ),
            PeerDenial::StalePeer
        );

        let live_fixture = Fixture::new();
        let live_route =
            test_nipc_route(live_fixture.route, live_fixture.record.app, "files.read@1");
        let live_proof = live_fixture
            .process_instances
            .issue_spawn_proof(live_fixture.process, live_fixture.record)
            .expect("proof for live test-owned child");
        let binding = live_fixture
            .process_instances
            .bind_spawn_owned_endpoint(live_proof, live_route.clone(), (), &live_fixture.supervisor)
            .expect("test-only spawn owner binding");
        assert_eq!(binding.proof.process, live_fixture.process);
        assert_eq!(binding.proof.record, live_fixture.record);
        assert_eq!(binding.route, live_route);
        assert_eq!(binding.listener, ());

        assert!(matches!(
            live_fixture
                .process_instances
                .issue_spawn_proof(live_fixture.process, live_fixture.record),
            Err(PeerDenial::DuplicateProcess)
        ));
    }

    #[test]
    fn pending_drop_cleanup_failure_is_observable_and_retryable() {
        let kernel = FakePidfdKernel::new();
        kernel.cleanup_errno.store(5, Ordering::SeqCst);
        let authority = PidfdProcessInstanceAuthority::new(kernel);
        let pending = authority
            .pending_test_spawn_handle(41, FakePidfd(7001))
            .unwrap();
        let process = pending.process();
        drop(pending);

        let denial = authority.cleanup_failure(process).unwrap().unwrap();
        let PeerDenial::KernelProcessInstance(error) = denial else {
            panic!("cleanup failure did not remain observable");
        };
        let ProcessInstanceError::KernelRead { option, source } = &*error else {
            panic!("cleanup errno was not retained");
        };
        assert_eq!(
            *option,
            pidfd_process_instance::ProcessInstanceOption::SpawnCleanup
        );
        assert_eq!(source.raw_os_error(), Some(5));
        assert!(authority.has_unresolved_cleanup().unwrap());

        authority.kernel().cleanup_errno.store(0, Ordering::SeqCst);
        authority.retry_unregistered_abort(process).unwrap();
        assert!(authority.cleanup_failure(process).unwrap().is_none());
        assert!(!authority.has_unresolved_cleanup().unwrap());
        assert!(authority.confirms_owned_process_live(process).is_err());
    }

    #[test]
    fn precapture_drop_cleanup_failure_is_retained_by_spawn_owner() {
        let kernel = FakePidfdKernel::new();
        kernel.cleanup_errno.store(5, Ordering::SeqCst);
        let authority = PidfdProcessInstanceAuthority::new(kernel);
        let pending = authority
            .pending_test_precapture_spawn(42, Some(FakePidfd(7003)))
            .unwrap();
        drop(pending);

        let failures = authority.precapture_cleanup_failures().unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].1, 42);
        let PeerDenial::KernelProcessInstance(error) = &failures[0].2 else {
            panic!("pre-capture cleanup failure lost its typed kernel cause");
        };
        let ProcessInstanceError::KernelRead { option, source } = &**error else {
            panic!("pre-capture cleanup failure lost its syscall error");
        };
        assert_eq!(
            *option,
            pidfd_process_instance::ProcessInstanceOption::SpawnCleanup
        );
        assert_eq!(source.raw_os_error(), Some(5));
        assert!(authority.has_unresolved_cleanup().unwrap());

        authority.kernel().cleanup_errno.store(0, Ordering::SeqCst);
        authority.retry_precapture_cleanups().unwrap();
        assert!(authority.precapture_cleanup_failures().unwrap().is_empty());
        assert!(!authority.has_unresolved_cleanup().unwrap());
    }

    #[test]
    fn precapture_missing_pidfd_cleanup_failure_retains_cleanup_only_pid() {
        let authority = PidfdProcessInstanceAuthority::new(FakePidfdKernel::new());
        let pending = authority.pending_test_precapture_spawn(43, None).unwrap();
        drop(pending);

        let failures = authority.precapture_cleanup_failures().unwrap();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].1, 43);
        assert!(matches!(
            failures[0].2,
            PeerDenial::ProcessInstanceUnavailable
        ));
        assert!(authority.has_unresolved_cleanup().unwrap());
        assert!(authority.retry_precapture_cleanups().is_err());
        assert_eq!(authority.precapture_cleanup_failures().unwrap().len(), 1);
    }

    #[test]
    fn spawn_proof_cannot_be_checked_by_another_pidfd_authority() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let other_authority = PidfdProcessInstanceAuthority::new(FakePidfdKernel::new());
        let other_process = other_authority
            .record_test_spawn_handle(41, FakePidfd(7001))
            .unwrap();
        assert_ne!(fixture.process, other_process);
        let (stream, _peer) = UnixStream::pair().unwrap();
        let checkout = fixture.checkout(fixture.endpoint, stream).unwrap();
        assert_denial!(
            fixture.authorize_checkout_with(
                checkout,
                registration,
                &fixture.requirements(),
                &other_authority,
            ),
            PeerDenial::ProcessInstanceUnavailable
        );
    }

    #[test]
    fn supervisor_revoke_retires_directory_and_spawn_pidfd() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        *fixture.supervisor.record.lock().unwrap() = None;
        assert!(fixture
            .directory
            .retire_after_supervisor_revoke(
                &fixture.run,
                fixture.process,
                registration,
                &fixture.readiness,
            )
            .unwrap());
        assert!(fixture
            .process_instances
            .retire_after_revoke(fixture.process)
            .unwrap());
        assert_denial!(
            fixture.authorize(fixture.process.pid(), registration),
            PeerDenial::StaleRegistration
        );
    }

    #[test]
    fn retirement_pending_registration_is_not_authorizable_before_exact_reap() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let retirement_pending = fixture
            .directory
            .retirement_latch(registration, fixture.process)
            .expect("registered entry owns a retirement latch");

        // SpawnEndpointReapLease::drop sets this same latch before best-effort
        // endpoint revoke; the directory slot remains until Task24 exact reap.
        mark_endpoint_retirement_pending(&retirement_pending);
        assert!(!fixture.directory.snapshot(registration).unwrap().is_live());
        assert_denial!(
            fixture.authorize(fixture.process.pid(), registration),
            PeerDenial::StalePeer
        );
        assert_eq!(fixture.directory.state.lock().unwrap().entries.len(), 1);

        *fixture.supervisor.record.lock().unwrap() = None;
        assert!(fixture
            .directory
            .retire_after_supervisor_revoke(
                &fixture.run,
                fixture.process,
                registration,
                &fixture.readiness,
            )
            .unwrap());
        assert!(fixture.directory.state.lock().unwrap().entries.is_empty());
    }

    #[test]
    fn run_generation_lookup_rejects_a_new_run_active_handle_for_an_old_generation() {
        let directory = Pid1LivePeerDirectory::<Record>::new();
        let first_run = directory.begin_run().unwrap();
        let old_generation = first_run.generation();
        let replacement_generation = NonZeroU64::new(old_generation.get() + 1).unwrap();
        {
            let mut state = directory.state.lock().unwrap();
            let replacement_active = Arc::new(AtomicBool::new(true));
            let replacement_nonce = [1; 32];
            state.current_run_generation = Some(replacement_generation);
            state.current_run_authority_nonce = Some(replacement_nonce);
            state.current_run_active = Some(Arc::clone(&replacement_active));
            state.current_authority_domain = Some(Pid1AuthorityDomain::for_run(
                replacement_generation,
                replacement_nonce,
                replacement_active,
            ));
        }

        assert_denial!(
            directory.run_for_generation(old_generation),
            PeerDenial::StaleRun
        );
        let replacement = directory
            .run_for_generation(replacement_generation)
            .unwrap();
        assert_eq!(replacement.generation(), replacement_generation);
        assert!(replacement.is_active());
    }

    #[test]
    fn route_mismatch_cannot_reuse_a_spawn_registration() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let (stream, _peer) = UnixStream::pair().unwrap();
        let checkout = fixture.checkout(fixture.endpoint, stream).unwrap();
        let other_route = TrustedRouteKey::from_canonical_route(11, 20, 1, 0, 4).unwrap();
        let other_requirements = TrustedServiceRequirements::from_canonical_route(
            other_route,
            &fixture.reqs,
            RouteConsentRequirement::NotRequired,
        );
        assert_denial!(
            fixture.authorize_checkout(checkout, registration, &other_requirements),
            PeerDenial::RouteMismatch
        );
    }

    #[test]
    fn endpoint_owner_binds_accept_to_the_requested_listener_token() {
        let fixture = Fixture::new();
        let other_route = TrustedRouteKey::from_canonical_route(11, 20, 1, 0, 4).unwrap();
        let other_endpoint =
            TrustedServiceEndpoint::from_service_registry(other_route, 9, 9002).unwrap();
        let (stream, _peer) = UnixStream::pair().unwrap();
        fixture.endpoints.accepted.lock().unwrap().push_back((
            other_endpoint,
            other_endpoint.endpoint_socket_id.get(),
            stream,
        ));
        assert_denial!(
            fixture.endpoints.accept_checkout(fixture.endpoint),
            PeerDenial::StaleEndpoint
        );
    }

    #[test]
    fn endpoint_owner_rejects_stream_from_wrong_listener_identity() {
        let fixture = Fixture::new();
        let (stream, _peer) = UnixStream::pair().unwrap();
        fixture.endpoints.accepted.lock().unwrap().push_back((
            fixture.endpoint,
            fixture.endpoint.endpoint_socket_id.get() + 1,
            stream,
        ));
        assert_denial!(
            fixture.endpoints.accept_checkout(fixture.endpoint),
            PeerDenial::StaleEndpoint
        );
    }

    #[test]
    fn accepted_peer_entry_rejects_unregistered_listener_before_accept() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let unregistered =
            TrustedServiceEndpoint::from_service_registry(fixture.route, 8, 8002).unwrap();
        let (stream, _client) = UnixStream::pair().unwrap();
        fixture.endpoints.accepted.lock().unwrap().push_back((
            fixture.endpoint,
            fixture.endpoint.endpoint_socket_id.get(),
            stream,
        ));

        assert_denial!(
            fixture.accept_authorize_until(
                unregistered,
                registration,
                Instant::now() + Duration::from_secs(1),
            ),
            PeerDenial::StaleEndpoint
        );
        assert_eq!(fixture.endpoints.accepted.lock().unwrap().len(), 1);
    }

    #[test]
    fn accepted_peer_entry_rejects_expired_deadline_before_accept() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let (stream, _client) = UnixStream::pair().unwrap();
        fixture.endpoints.accepted.lock().unwrap().push_back((
            fixture.endpoint,
            fixture.endpoint.endpoint_socket_id.get(),
            stream,
        ));

        assert_denial!(
            fixture.accept_authorize_until(
                fixture.endpoint,
                registration,
                Instant::now() - Duration::from_millis(1),
            ),
            PeerDenial::RequestDeadlineExpired
        );
        assert_eq!(fixture.endpoints.accepted.lock().unwrap().len(), 1);
    }

    #[test]
    fn accepted_peer_entry_rejects_replaced_checkout_identity() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let wrong_route = TrustedRouteKey::from_canonical_route(11, 20, 1, 0, 4).unwrap();
        let replacement =
            TrustedServiceEndpoint::from_service_registry(wrong_route, 9, 9002).unwrap();
        let (stream, _client) = UnixStream::pair().unwrap();
        fixture.endpoints.accepted.lock().unwrap().push_back((
            replacement,
            replacement.endpoint_socket_id.get(),
            stream,
        ));

        assert_denial!(
            fixture.accept_authorize_until(
                fixture.endpoint,
                registration,
                Instant::now() + Duration::from_secs(1),
            ),
            PeerDenial::StaleEndpoint
        );
    }

    #[test]
    fn delegated_fd_without_enforced_endpoint_policy_is_rejected() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        fixture
            .endpoints
            .fd_delegation_blocked
            .store(false, Ordering::SeqCst);
        let (stream, _peer) = UnixStream::pair().unwrap();
        let delegated = stream.try_clone().unwrap();
        let checkout = fixture.checkout(fixture.endpoint, stream).unwrap();
        assert_denial!(
            fixture.authorize_checkout(checkout, registration, &fixture.requirements()),
            PeerDenial::FdDelegationNotPrevented
        );
        // The duplicate remains a usable FD. This candidate deliberately makes
        // the external non-delegation or per-message-writer authority mandatory.
        assert!(delegated.peer_addr().is_ok());
    }

    #[test]
    fn spawn_registration_refuses_alias_risk_before_publishing_a_live_entry() {
        let fixture = Fixture::new();
        let (stream, _peer) = UnixStream::pair().unwrap();
        let retained_alias = stream.try_clone().unwrap();
        fixture
            .endpoints
            .fd_delegation_blocked
            .store(false, Ordering::SeqCst);

        // The duplicate remains live, while the same public registration
        // boundary used before exec release refuses to publish the route.
        let spawn = fixture.spawn_proof().unwrap();
        assert_denial!(
            fixture.register_proof(spawn, &fixture.requirements(), fixture.endpoint),
            PeerDenial::FdDelegationNotPrevented
        );
        assert!(fixture.directory.state.lock().unwrap().entries.is_empty());
        assert!(retained_alias.peer_addr().is_ok());
    }

    #[test]
    fn successful_authorization_retains_the_accepted_connection_behind_io_only_api() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let checkout = fixture.checkout(fixture.endpoint, stream).unwrap();
        let mut authorized = fixture
            .authorize_checkout(checkout, registration, &fixture.requirements())
            .unwrap();
        authorized
            .with_live_io(|io| io.write_all(b"authorized"))
            .unwrap()
            .unwrap();
        let mut message = [0; 10];
        peer.read_exact(&mut message).unwrap();
        assert_eq!(&message, b"authorized");
    }

    #[test]
    fn endpoint_replacement_invalidates_old_checkout() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let mut escaped_alias = stream.try_clone().unwrap();
        let checkout = fixture.checkout(fixture.endpoint, stream).unwrap();
        let replacement =
            TrustedServiceEndpoint::from_service_registry(fixture.route, 8, 8002).unwrap();
        *fixture.endpoints.current.lock().unwrap() = replacement;
        assert_denial!(
            fixture.authorize_checkout(checkout, registration, &fixture.requirements()),
            PeerDenial::StaleEndpoint
        );
        // The escaped descriptor still transports bytes; the endpoint owner
        // cannot revoke it. Its existence does not restore the stale checkout
        // or authorize a new request through the replacement endpoint.
        escaped_alias.write_all(b"alias-outlives-checkout").unwrap();
        let mut bytes = [0; 23];
        peer.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"alias-outlives-checkout");
    }

    #[test]
    fn replacing_recipient_without_changing_endpoint_token_denies_effect_commit() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 90), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);

        // Model registry replacement that accidentally preserves the numeric
        // endpoint token. The owner incarnation must still change recipient.
        fixture.endpoints.replace_recipient_for_test();
        assert_eq!(admission.endpoint(), fixture.endpoint);
        let effects = AtomicUsize::new(0);
        assert!(matches!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(PeerDenial::ServiceEffectNotInvoked(cause))
                if matches!(cause.as_ref(), PeerDenial::StaleEndpoint)
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        assert_eq!(owner.commits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn socket_cookie_mismatch_rejects_endpoint_checkout() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let (stream, _peer) = UnixStream::pair().unwrap();
        let mut checkout = fixture.checkout(fixture.endpoint, stream).unwrap();
        checkout.socket_cookie = NonZeroU64::new(9002).unwrap();
        assert_denial!(
            fixture.authorize_checkout(checkout, registration, &fixture.requirements()),
            PeerDenial::StaleSocket
        );
    }

    #[test]
    fn endpoint_owner_rejects_a_checkout_from_an_unregistered_listener_socket() {
        let fixture = Fixture::new();
        let (stream, _peer) = UnixStream::pair().unwrap();
        fixture.endpoints.accepted.lock().unwrap().push_back((
            fixture.endpoint,
            fixture.endpoint.endpoint_socket_id.get() + 1,
            stream,
        ));

        assert_denial!(
            fixture.endpoints.accept_checkout(fixture.endpoint),
            PeerDenial::StaleEndpoint
        );
    }

    #[test]
    fn task39_transfer_binds_the_same_moved_stream_to_the_registered_recipient() {
        let fixture = Fixture::new();
        let (stream, mut accepted_peer) = UnixStream::pair().unwrap();
        // A preexisting alias remains usable. Recipient binding identifies the
        // moved socket but does not prove unique-FD ownership or alias revocation.
        let mut alias = stream.try_clone().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        let recipient = fixture
            .endpoints
            .checkout_recipient(fixture.endpoint)
            .unwrap();
        let checkout = bind_task39_transfer_to_endpoint_checkout(
            recipient.clone(),
            &fixture.endpoints.recipient,
            deadline,
            FakeTask39StreamTransfer {
                stream: Some(stream),
                deadline,
            },
            |_| Ok(NonZeroU64::new(9001).unwrap()),
        )
        .unwrap();

        assert_eq!(checkout.endpoint, fixture.endpoint);
        assert!(checkout.recipient.is_same_recipient(&recipient));
        let mut checkout_stream = checkout.stream;
        checkout_stream.write_all(b"same-accepted-stream").unwrap();
        let mut message = [0; 20];
        accepted_peer.read_exact(&mut message).unwrap();
        assert_eq!(&message, b"same-accepted-stream");
        alias.write_all(b"alias-still-live").unwrap();
        let mut alias_message = [0; 16];
        accepted_peer.read_exact(&mut alias_message).unwrap();
        assert_eq!(&alias_message, b"alias-still-live");
    }

    #[test]
    fn independent_alias_transfer_reaches_the_request_ledger_but_not_a_second_effect() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let (stream, _peer_stream) = UnixStream::pair().unwrap();
        let alias = stream.try_clone().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        let recipient = fixture
            .endpoints
            .checkout_recipient(fixture.endpoint)
            .unwrap();

        // This models Task39's independently constructible second transfer
        // from an escaped alias. It is intentionally a fixture transfer: no
        // real pidfd or current-writer authority is asserted here.
        let first_checkout = bind_task39_transfer_to_endpoint_checkout(
            recipient.clone(),
            &fixture.endpoints.recipient,
            deadline,
            FakeTask39StreamTransfer {
                stream: Some(stream),
                deadline,
            },
            |_| Ok(NonZeroU64::new(9001).unwrap()),
        )
        .unwrap();
        let alias_checkout = bind_task39_transfer_to_endpoint_checkout(
            recipient.clone(),
            &fixture.endpoints.recipient,
            deadline,
            FakeTask39StreamTransfer {
                stream: Some(alias),
                deadline,
            },
            |_| Ok(NonZeroU64::new(9001).unwrap()),
        )
        .unwrap();
        assert_eq!(first_checkout.socket_cookie, alias_checkout.socket_cookie);
        assert!(first_checkout
            .recipient
            .is_same_recipient(&alias_checkout.recipient));

        let first_peer = fixture
            .authorize_checkout(first_checkout, registration, &fixture.requirements())
            .unwrap();
        let alias_peer = fixture
            .authorize_checkout(alias_checkout, registration, &fixture.requirements())
            .unwrap();
        let first_admission = first_peer
            .admit_request(complete_request_frame(&first_peer, 91), b"files.read")
            .unwrap();
        let alias_admission = alias_peer
            .admit_request(complete_request_frame(&alias_peer, 91), b"files.read")
            .unwrap();
        assert_eq!(first_admission.request_id(), alias_admission.request_id());
        assert_eq!(
            first_admission.stable_request_id(),
            alias_admission.stable_request_id()
        );

        let owner = FakeServiceOwner::new(first_admission.clone(), &fixture.readiness);
        let effects = AtomicUsize::new(0);
        first_peer
            .commit_effect(&first_admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        assert_denial!(
            alias_peer.commit_effect(&alias_admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            PeerDenial::DuplicateRequestCommit
        );
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(owner.commits.load(Ordering::SeqCst), 1);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn registered_listener_candidate_pins_recipient_epoch_and_expiry_fail_closed() {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let socket_path = std::env::temp_dir().join(format!(
            "nagi-task41-stage17-{}-{nonce}.sock",
            std::process::id()
        ));
        let replacement_path = std::env::temp_dir().join(format!(
            "nagi-task41-stage17-{}-{nonce}-replacement.sock",
            std::process::id()
        ));
        let expired_path = std::env::temp_dir().join(format!(
            "nagi-task41-stage17-{}-{nonce}-expired.sock",
            std::process::id()
        ));
        let route = TrustedRouteKey::from_canonical_route(41, 1, 1, 0, 1).unwrap();
        let endpoint = TrustedServiceEndpoint::from_service_registry(route, 41, 4101).unwrap();
        let registry_slot = Arc::new(EndpointOwnerRegistrySlot::default());
        let authority_fixture = Fixture::new();
        let run = authority_fixture.run.clone();
        let listener = UnixListener::bind(&socket_path).unwrap();
        let owner = RegisteredEndpointOwnerCandidate::from_service_registry(
            Arc::clone(&registry_slot),
            &run,
            endpoint,
            listener,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        let client = UnixStream::connect(&socket_path).unwrap();
        let checkout_deadline = Instant::now() + Duration::from_secs(2);
        let checkout_result = owner.accept_checkout_until(endpoint, checkout_deadline);

        assert!(matches!(owner.is_current(endpoint), Ok(true)));
        assert!(matches!(
            owner.fd_delegation_is_prevented(endpoint),
            Ok(false)
        ));
        match checkout_result {
            Ok(checkout) => {
                assert_eq!(checkout.endpoint, endpoint);
                assert_eq!(checkout.endpoint_epoch, endpoint.endpoint_generation);
                assert_eq!(checkout.expires_at, checkout_deadline);
                assert!(checkout.recipient.is_same_recipient(&owner.recipient));
                drop(checkout);
            }
            Err(PeerDenial::PeerConnection(_)) | Err(PeerDenial::Unavailable) => {
                // The host's Task39 kernel credential reader is unsupported.
                // Actual AF_UNIX connect/accept was attempted and failed closed.
            }
            Err(error) => panic!("unexpected actual listener result: {error:?}"),
        }

        let replacement_endpoint =
            TrustedServiceEndpoint::from_service_registry(route, 42, 4201).unwrap();
        let replacement_listener = UnixListener::bind(&replacement_path).unwrap();
        let replacement = RegisteredEndpointOwnerCandidate::from_service_registry(
            Arc::clone(&registry_slot),
            &run,
            replacement_endpoint,
            replacement_listener,
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        assert!(matches!(owner.is_current(endpoint), Ok(false)));
        assert!(matches!(
            replacement.is_current(replacement_endpoint),
            Ok(true)
        ));
        owner.revoke_for_registry().unwrap();
        assert!(matches!(
            replacement.is_current(replacement_endpoint),
            Ok(true)
        ));
        assert_denial!(
            owner.accept_checkout_until(endpoint, Instant::now() + Duration::from_secs(1)),
            PeerDenial::StaleEndpoint
        );
        assert!(matches!(
            owner.fd_delegation_is_prevented(endpoint),
            Err(PeerDenial::StaleEndpoint)
        ));

        let expired_endpoint =
            TrustedServiceEndpoint::from_service_registry(route, 43, 4301).unwrap();
        let expired_listener = UnixListener::bind(&expired_path).unwrap();
        let expires_at = Instant::now() + Duration::from_millis(20);
        let expired_owner = RegisteredEndpointOwnerCandidate::from_service_registry(
            Arc::clone(&registry_slot),
            &run,
            expired_endpoint,
            expired_listener,
            expires_at,
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(30));
        assert!(matches!(
            expired_owner.is_current(expired_endpoint),
            Ok(false)
        ));
        assert_denial!(
            expired_owner
                .accept_checkout_until(expired_endpoint, Instant::now() + Duration::from_secs(1)),
            PeerDenial::RequestDeadlineExpired
        );
        authority_fixture
            .directory
            .end_run_owner_lease(
                &authority_fixture.run_owner_lease,
                &authority_fixture.readiness,
            )
            .unwrap();
        assert!(matches!(
            expired_owner.is_current(expired_endpoint),
            Ok(false)
        ));

        drop(client);
        drop(owner);
        drop(replacement);
        drop(expired_owner);
        let _ = std::fs::remove_file(socket_path);
        let _ = std::fs::remove_file(replacement_path);
        let _ = std::fs::remove_file(expired_path);
    }

    #[test]
    fn expired_task39_transfer_is_rejected_and_dropped_before_cookie_read() {
        let fixture = Fixture::new();
        let (stream, mut accepted_peer) = UnixStream::pair().unwrap();
        let transfer_deadline = Instant::now() + Duration::from_secs(1);
        let expected_deadline = transfer_deadline + Duration::from_millis(5);
        let recipient = fixture
            .endpoints
            .checkout_recipient(fixture.endpoint)
            .unwrap();
        let cookie_read = AtomicBool::new(false);

        assert_denial!(
            bind_task39_transfer_to_endpoint_checkout(
                recipient,
                &fixture.endpoints.recipient,
                expected_deadline,
                FakeTask39StreamTransfer {
                    stream: Some(stream),
                    deadline: transfer_deadline,
                },
                |_| {
                    cookie_read.store(true, Ordering::SeqCst);
                    Ok(NonZeroU64::new(9001).unwrap())
                },
            ),
            PeerDenial::RequestDeadlineExpired
        );
        assert!(!cookie_read.load(Ordering::SeqCst));
        let mut byte = [0; 1];
        assert_eq!(accepted_peer.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn task39_transfer_from_a_different_recipient_is_dropped_before_checkout() {
        let fixture = Fixture::new();
        let (stream, mut accepted_peer) = UnixStream::pair().unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        let replacement_recipient =
            EndpointCheckoutRecipient::from_registered_owner(fixture.endpoint);
        let cookie_read = AtomicBool::new(false);

        assert_denial!(
            bind_task39_transfer_to_endpoint_checkout(
                replacement_recipient,
                &fixture.endpoints.recipient,
                deadline,
                FakeTask39StreamTransfer {
                    stream: Some(stream),
                    deadline,
                },
                |_| {
                    cookie_read.store(true, Ordering::SeqCst);
                    Ok(NonZeroU64::new(9001).unwrap())
                },
            ),
            PeerDenial::StaleEndpoint
        );
        assert!(!cookie_read.load(Ordering::SeqCst));
        let mut byte = [0; 1];
        assert_eq!(accepted_peer.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn endpoint_owner_consumes_each_registered_listener_accept_once() {
        let fixture = Fixture::new();
        let (stream, _peer) = UnixStream::pair().unwrap();
        let checkout = fixture.checkout(fixture.endpoint, stream).unwrap();
        assert_eq!(checkout.endpoint, fixture.endpoint);

        assert_denial!(
            fixture.endpoints.accept_checkout(fixture.endpoint),
            PeerDenial::Unavailable
        );
    }

    #[test]
    fn one_spawn_proof_cannot_be_issued_twice() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        assert_ne!(registration.0.get(), 0);
        match fixture.spawn_proof() {
            Err(PeerDenial::DuplicateProcess) => {}
            _ => panic!("spawn proof was reissued for another route"),
        }
    }

    #[test]
    fn absent_process_lifetime_evidence_fails_closed() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        fixture
            .process_instances
            .kernel()
            .unavailable
            .store(true, Ordering::SeqCst);
        assert_denial!(
            fixture.authorize(fixture.process.pid(), registration),
            PeerDenial::ProcessInstanceUnavailable
        );
    }

    #[test]
    fn revoked_session_and_grant_are_rechecked_on_checkout() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        fixture.sessions.live.store(false, Ordering::SeqCst);
        assert_denial!(
            fixture.authorize(fixture.process.pid(), registration),
            PeerDenial::SessionRevoked
        );
        fixture.sessions.live.store(true, Ordering::SeqCst);
        *fixture.supervisor.grants.lock().unwrap() = Vec::new();
        assert_denial!(
            fixture.authorize(fixture.process.pid(), registration),
            PeerDenial::ConsentRequired
        );
    }

    #[test]
    fn retained_peer_rechecks_session_and_consent_before_io() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let mut peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let callback_ran = AtomicBool::new(false);

        fixture.sessions.live.store(false, Ordering::SeqCst);
        assert_denial!(
            peer.with_live_io(|_| callback_ran.store(true, Ordering::SeqCst)),
            PeerDenial::SessionRevoked
        );
        assert!(!callback_ran.load(Ordering::SeqCst));

        fixture.sessions.live.store(true, Ordering::SeqCst);
        fixture.consent.connected.store(false, Ordering::SeqCst);
        assert_denial!(
            peer.with_live_io(|_| callback_ran.store(true, Ordering::SeqCst)),
            PeerDenial::ConsentUnavailable
        );
        assert!(!callback_ran.load(Ordering::SeqCst));
    }

    #[test]
    fn effect_commit_rechecks_session_after_request_admission() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 1), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let effect_ran = AtomicBool::new(false);

        fixture.sessions.live.store(false, Ordering::SeqCst);
        let denial = peer.commit_effect(&admission, &fixture.readiness, &owner, || {
            effect_ran.store(true, Ordering::SeqCst);
            Ok(())
        });
        assert!(matches!(
            denial,
            Err(PeerDenial::ServiceEffectNotInvoked(cause))
                if matches!(cause.as_ref(), PeerDenial::SessionRevoked)
        ));
        assert!(!effect_ran.load(Ordering::SeqCst));
        assert_eq!(owner.commits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn revoke_winning_c1_gate_while_c2_commit_waits_denies_effect() {
        use std::sync::mpsc;

        let fixture = Fixture::new();
        let registration = fixture.register();
        let mut peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let lease = fixture
            .sessions
            .capture_live_unlocked(fixture.sessions.app_id, fixture.sessions.app_session_id)
            .unwrap();
        let session_gate = Arc::new(C1SessionEffectGateFixture::new(lease));
        let gate_for_peer = Arc::clone(&session_gate);
        peer.session_effect_gate =
            Arc::new(move |effect| gate_for_peer.with_live_session_effect(lease, effect));
        let admission = peer
            .admit_request(complete_request_frame(&peer, 1), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let effect_ran = AtomicBool::new(false);
        let readiness_calls = fixture.readiness.calls.load(Ordering::SeqCst);

        let (attempted, attempted_rx) = mpsc::channel();
        session_gate.signal_next_attempt(attempted);
        let mut revoke = session_gate.live.write().unwrap();
        *revoke = false;

        let outcome = std::thread::scope(|scope| {
            let commit = scope.spawn(|| {
                peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                    effect_ran.store(true, Ordering::SeqCst);
                    Ok(())
                })
            });
            attempted_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("effect did not wait at the C1 lease gate");
            assert_eq!(
                fixture.readiness.calls.load(Ordering::SeqCst),
                readiness_calls
            );
            assert_eq!(owner.commits.load(Ordering::SeqCst), 0);
            assert!(!effect_ran.load(Ordering::SeqCst));
            drop(revoke);
            commit.join().unwrap()
        });

        assert!(matches!(
            outcome,
            Err(PeerDenial::ServiceEffectNotInvoked(cause))
                if matches!(cause.as_ref(), PeerDenial::SessionRevoked)
        ));
        assert!(!effect_ran.load(Ordering::SeqCst));
        assert_eq!(owner.commits.load(Ordering::SeqCst), 0);
        assert_eq!(
            fixture.readiness.calls.load(Ordering::SeqCst),
            readiness_calls
        );
    }

    #[test]
    fn c1_revoke_waits_until_c2_owner_gate_and_effect_commit_finish() {
        use std::sync::mpsc;

        let fixture = Fixture::new();
        let registration = fixture.register();
        let mut peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let lease = fixture
            .sessions
            .capture_live_unlocked(fixture.sessions.app_id, fixture.sessions.app_session_id)
            .unwrap();
        let session_gate = Arc::new(C1SessionEffectGateFixture::new(lease));
        let gate_for_peer = Arc::clone(&session_gate);
        peer.session_effect_gate =
            Arc::new(move |effect| gate_for_peer.with_live_session_effect(lease, effect));
        let admission = peer
            .admit_request(complete_request_frame(&peer, 1), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let order = AtomicUsize::new(0);
        let effect_order = AtomicUsize::new(0);
        let revoke_order = AtomicUsize::new(0);
        let (effect_at_commit, effect_at_commit_rx) = mpsc::channel();
        let (finish_effect, finish_effect_rx) = mpsc::channel();
        let finish_effect_rx = Arc::new(Mutex::new(finish_effect_rx));
        let (revoke_started, revoke_started_rx) = mpsc::channel();
        let (revoke_finished, revoke_finished_rx) = mpsc::channel();

        let outcome = std::thread::scope(|scope| {
            let commit = scope.spawn(|| {
                peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                    assert!(fixture.readiness.active.load(Ordering::SeqCst));
                    assert!(matches!(
                        session_gate.live.try_write(),
                        Err(std::sync::TryLockError::WouldBlock)
                    ));
                    assert!(matches!(
                        owner.state.try_lock(),
                        Err(std::sync::TryLockError::WouldBlock)
                    ));
                    effect_at_commit.send(()).unwrap();
                    finish_effect_rx
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(2))
                        .map_err(|_| PeerDenial::Unavailable)?;
                    effect_order.store(order.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                    Ok(())
                })
            });
            effect_at_commit_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("effect did not reach the nested C1/C2 gates");

            let revoker = scope.spawn(|| {
                revoke_started.send(()).unwrap();
                let mut live = session_gate
                    .live
                    .write()
                    .map_err(|_| PeerDenial::Unavailable)?;
                *live = false;
                revoke_order.store(order.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
                revoke_finished.send(()).unwrap();
                Ok::<(), PeerDenial>(())
            });
            revoke_started_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("revocation did not start");
            assert!(matches!(
                revoke_finished_rx.recv_timeout(Duration::from_millis(50)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));

            finish_effect.send(()).unwrap();
            let outcome = commit.join().unwrap();
            revoke_finished_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("revocation did not finish after the effect gate released");
            revoker.join().unwrap().unwrap();
            outcome
        });

        assert!(outcome.is_ok());
        assert_eq!(owner.commits.load(Ordering::SeqCst), 1);
        assert!(effect_order.load(Ordering::SeqCst) > 0);
        assert!(revoke_order.load(Ordering::SeqCst) > effect_order.load(Ordering::SeqCst));
    }

    #[test]
    fn absent_session_consent_provenance_is_unavailable() {
        let fixture = Fixture::new_with_consent_requirement(RouteConsentRequirement::Required);
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        assert!(matches!(
            peer.admit_request_with_session_consent(
                complete_request_frame(&peer, 1),
                b"files.read",
                &UnavailableSessionConsentAuthority
            ),
            Err(PeerDenial::ConsentUnavailable)
        ));

        let admission = peer
            .admit_request(complete_request_frame(&peer, 1), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let effect_ran = AtomicBool::new(false);
        assert!(matches!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effect_ran.store(true, Ordering::SeqCst);
                Ok(())
            }),
            Err(PeerDenial::ConsentRequired)
        ));
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::NotInvoked
        );
        assert!(matches!(
            peer.commit_consented_effect(
                &admission,
                &UnavailableSessionConsentAuthority,
                &fixture.readiness,
                &owner,
                || Ok(())
            ),
            Err(PeerDenial::ConsentUnavailable)
        ));
        assert!(!effect_ran.load(Ordering::SeqCst));
    }

    #[test]
    fn session_consent_event_revision_change_denies_effect_commit() {
        let fixture = Fixture::new_with_consent_requirement(RouteConsentRequirement::Required);
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let consent_owner = FakeSessionOwnedConsent::new();
        let admission = peer
            .admit_request_with_session_consent(
                complete_request_frame(&peer, 1),
                b"files.read",
                &consent_owner,
            )
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let effect_ran = AtomicBool::new(false);
        consent_owner.advance_revision(&fixture.readiness, &fixture.run);

        let denial = peer.commit_consented_effect(
            &admission,
            &consent_owner,
            &fixture.readiness,
            &owner,
            || {
                effect_ran.store(true, Ordering::SeqCst);
                Ok(())
            },
        );
        assert!(matches!(
            denial,
            Err(PeerDenial::ServiceEffectNotInvoked(cause))
                if matches!(cause.as_ref(), PeerDenial::ConsentDenied)
        ));
        assert!(!effect_ran.load(Ordering::SeqCst));
        assert_eq!(owner.commits.load(Ordering::SeqCst), 0);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::NotInvoked
        );
        let retry = peer
            .admit_request_with_session_consent(
                complete_request_frame(&peer, 1),
                b"files.read",
                &consent_owner,
            )
            .unwrap();
        assert_eq!(retry.request_id(), admission.request_id());
        assert_eq!(
            retry.operation_state(),
            ServiceRequestOperationState::NotInvoked
        );
        let retry_owner = FakeServiceOwner::new(retry.clone(), &fixture.readiness);
        peer.commit_consented_effect(
            &retry,
            &consent_owner,
            &fixture.readiness,
            &retry_owner,
            || Ok(()),
        )
        .unwrap();
        assert_eq!(
            retry.operation_state(),
            ServiceRequestOperationState::Executed
        );
    }

    #[test]
    fn mapped_required_consent_public_path_rejects_missing_expired_and_revoked() {
        let fixture = Fixture::new_with_consent_requirement(RouteConsentRequirement::Required);
        let registration = fixture.register();
        let mapper = test_nipc_mapper(fixture.route, 1, "files.read@1");
        let effects = AtomicUsize::new(0);

        // A mapped request can be parsed without an approval event, but both
        // public commit methods reject it before the owner callback runs.
        {
            let peer = fixture
                .authorize(fixture.process.pid(), registration)
                .unwrap();
            let (request, binding) = decode_mapped_request(&peer, &mapper, [71; 16]);
            let admission = peer.admit_nipc_request(request, binding).unwrap();
            let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
            assert_denial!(
                peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                    effects.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }),
                PeerDenial::ConsentRequired
            );
            assert_denial!(
                peer.commit_consented_effect(
                    &admission,
                    &UnavailableSessionConsentAuthority,
                    &fixture.readiness,
                    &owner,
                    || {
                        effects.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                ),
                PeerDenial::ConsentUnavailable
            );
            assert_eq!(owner.commits.load(Ordering::SeqCst), 0);
        }

        // The mapped consent-aware admission surface also fails closed when
        // the session owner is absent.
        {
            let peer = fixture
                .authorize(fixture.process.pid(), registration)
                .unwrap();
            let (request, binding) = decode_mapped_request(&peer, &mapper, [72; 16]);
            assert_denial!(
                peer.admit_nipc_request_with_session_consent(
                    request,
                    binding,
                    &UnavailableSessionConsentAuthority
                ),
                PeerDenial::ConsentUnavailable
            );
        }

        // Expiry is owned by the session provider. The test provider expires
        // an issued grant under the shared gate before commit.
        {
            let peer = fixture
                .authorize(fixture.process.pid(), registration)
                .unwrap();
            let consent = FakeSessionOwnedConsent::new();
            let (request, binding) = decode_mapped_request(&peer, &mapper, [73; 16]);
            let admission = peer
                .admit_nipc_request_with_session_consent(request, binding, &consent)
                .unwrap();
            let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
            consent.expire_current_event(&fixture.readiness, &fixture.run);
            assert!(matches!(
                peer.commit_consented_effect(
                    &admission,
                    &consent,
                    &fixture.readiness,
                    &owner,
                    || {
                        effects.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                ),
                Err(PeerDenial::ServiceEffectNotInvoked(cause))
                    if matches!(cause.as_ref(), PeerDenial::ConsentDenied)
            ));
            assert_eq!(owner.commits.load(Ordering::SeqCst), 0);
            assert_eq!(
                admission.operation_state(),
                ServiceRequestOperationState::NotInvoked
            );
        }

        // Revocation advances the issuing owner's event revision and is
        // rejected by that same public commit path.
        {
            let peer = fixture
                .authorize(fixture.process.pid(), registration)
                .unwrap();
            let consent = FakeSessionOwnedConsent::new();
            let (request, binding) = decode_mapped_request(&peer, &mapper, [74; 16]);
            let admission = peer
                .admit_nipc_request_with_session_consent(request, binding, &consent)
                .unwrap();
            let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
            consent.advance_revision(&fixture.readiness, &fixture.run);
            assert!(matches!(
                peer.commit_consented_effect(
                    &admission,
                    &consent,
                    &fixture.readiness,
                    &owner,
                    || {
                        effects.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                ),
                Err(PeerDenial::ServiceEffectNotInvoked(cause))
                    if matches!(cause.as_ref(), PeerDenial::ConsentDenied)
            ));
            assert_eq!(owner.commits.load(Ordering::SeqCst), 0);
            assert_eq!(
                admission.operation_state(),
                ServiceRequestOperationState::NotInvoked
            );
        }

        // A still-current event from the consent owner admits exactly the
        // mapped operation; the effect runs once inside the owner transaction.
        {
            let peer = fixture
                .authorize(fixture.process.pid(), registration)
                .unwrap();
            let consent = FakeSessionOwnedConsent::new();
            let (request, binding) = decode_mapped_request(&peer, &mapper, [75; 16]);
            let admission = peer
                .admit_nipc_request_with_session_consent(request, binding, &consent)
                .unwrap();
            let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
            peer.commit_consented_effect(&admission, &consent, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
            assert_eq!(owner.commits.load(Ordering::SeqCst), 1);
            assert_eq!(
                admission.operation_state(),
                ServiceRequestOperationState::Executed
            );
        }

        assert_eq!(effects.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn stable_wire_re_admission_and_clones_share_one_operation_state() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 1), b"files.read")
            .unwrap();
        let clone = admission.clone();
        let replay = peer
            .admit_request(complete_request_frame(&peer, 1), b"files.read")
            .unwrap();
        let replay_on_new_connection = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let replay_on_new_connection = replay_on_new_connection
            .admit_request(
                complete_request_frame(&replay_on_new_connection, 1),
                b"files.read",
            )
            .unwrap();
        assert_eq!(admission.request_id(), clone.request_id());
        assert_eq!(admission.request_id(), replay.request_id());
        assert_eq!(admission.stable_request_id(), replay.stable_request_id());
        assert_eq!(
            admission.request_id(),
            replay_on_new_connection.request_id()
        );

        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let effects = AtomicU64::new(0);
        peer.commit_effect(&admission, &fixture.readiness, &owner, || {
            effects.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        assert_eq!(
            replay.operation_state(),
            ServiceRequestOperationState::Executed
        );

        assert_denial!(
            peer.commit_effect(&replay, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            PeerDenial::DuplicateRequestCommit
        );
        assert_denial!(
            peer.commit_effect(
                &replay_on_new_connection,
                &fixture.readiness,
                &owner,
                || {
                    effects.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            ),
            PeerDenial::DuplicateRequestCommit
        );
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(owner.commits.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::Executed
        );
    }

    #[test]
    fn same_wire_id_with_different_payload_fingerprint_is_rejected() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let first_peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = first_peer
            .admit_request(
                complete_request_frame_with_content(&first_peer, 32, b"svc.read@1:payload-a"),
                b"files.read",
            )
            .unwrap();

        let replay_peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        assert_denial!(
            replay_peer.admit_request(
                complete_request_frame_with_content(&replay_peer, 32, b"svc.write@1:payload-b",),
                b"files.read"
            ),
            PeerDenial::RequestIdentityConflict
        );
        assert_eq!(
            admission.fingerprint(),
            ServiceRequestFingerprint::for_test(b"svc.read@1:payload-a")
        );
    }

    #[test]
    fn same_wire_id_and_payload_with_different_operation_is_rejected() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let first_peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = first_peer
            .admit_request_operation(
                complete_request_frame_with_content(&first_peer, 34, b"same-canonical-frame"),
                ServiceId::new("files").unwrap(),
                ContractVersion::new(1, 0).unwrap(),
                OperationId::new("read").unwrap(),
                CapabilityId::new("files.read").unwrap(),
                fixture_durable_owner_id(),
            )
            .unwrap();

        let replay_peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        assert_denial!(
            replay_peer.admit_request_operation(
                complete_request_frame_with_content(&replay_peer, 34, b"same-canonical-frame"),
                ServiceId::new("files").unwrap(),
                ContractVersion::new(1, 0).unwrap(),
                OperationId::new("write").unwrap(),
                CapabilityId::new("files.read").unwrap(),
                fixture_durable_owner_id(),
            ),
            PeerDenial::RequestIdentityConflict
        );
        assert_eq!(admission.operation(), b"read");
    }

    #[test]
    fn canonical_nipc_mapper_uses_declared_service_operation_and_capability() {
        let route = TrustedRouteKey::from_canonical_route(10, 20, 1, 0, 4).unwrap();
        let mapper = test_nipc_mapper(route, 1, "files.read@1");
        let request = decode_test_request(
            9,
            Some(1),
            [37; 16],
            "files.read@1",
            b"opaque service payload",
        )
        .unwrap();

        let binding = mapper.bind_request(&request).unwrap();
        assert_eq!(binding.route, route);
        assert_eq!(binding.service_id.as_str(), "files");
        assert_eq!(
            binding.contract_version,
            ContractVersion::new(1, 0).unwrap()
        );
        assert_eq!(binding.operation_id.as_str(), "read");
        assert_eq!(binding.capability_id.as_str(), "files.read");
        assert_eq!(binding.fingerprint, request.fingerprint());
    }

    #[test]
    fn mapped_request_cannot_self_report_a_different_caller_than_the_peer_record() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let mapper = test_nipc_mapper(fixture.route, 1, "files.read@1");
        let request = decode_test_request_for_connection(
            Arc::clone(&peer.request_frame),
            Arc::clone(&peer.connection_stale),
            fixture.record.app + 1,
            Some(1),
            [0x39; 16],
            "files.read@1",
            b"caller-claim-is-untrusted",
        )
        .expect("well-formed fixture frame with a false caller claim");
        let binding = mapper
            .bind_request(&request)
            .expect("route mapping does not authenticate the caller field");

        assert_denial!(
            peer.admit_nipc_request(request, binding),
            PeerDenial::InvalidNipcRequest
        );
    }

    #[test]
    fn canonical_nipc_mapper_rejects_unknown_destination_and_payload_type() {
        let route = TrustedRouteKey::from_canonical_route(10, 20, 1, 0, 4).unwrap();
        let mapper = test_nipc_mapper(route, 1, "files.read@1");
        let unknown_destination =
            decode_test_request(9, Some(99), [38; 16], "files.read@1", b"payload").unwrap();
        assert_denial!(
            mapper.bind_request(&unknown_destination),
            PeerDenial::UnknownNipcRoute
        );

        let unknown_payload_type =
            decode_test_request(9, Some(1), [39; 16], "files.delete@1", b"payload").unwrap();
        assert_denial!(
            mapper.bind_request(&unknown_payload_type),
            PeerDenial::UnknownNipcOperation
        );
    }

    #[test]
    fn same_wire_request_id_cannot_be_bound_to_a_different_route() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admitted = peer
            .admit_request(
                complete_request_frame_with_content(&peer, 40, b"same request body"),
                b"files.read",
            )
            .unwrap();
        let other_route = TrustedRouteKey::from_canonical_route(11, 20, 1, 0, 4).unwrap();
        let other_route_mapper = test_nipc_mapper(other_route, 1, "files.read@1");
        let other_route_request = decode_test_request(
            fixture.record.app,
            Some(1),
            [40; 16],
            "files.read@1",
            b"same request body",
        )
        .unwrap();
        let other_binding = other_route_mapper
            .bind_request(&other_route_request)
            .unwrap();
        assert_eq!(admitted.stable_request_id(), other_binding.wire_request_id);
        assert_ne!(admitted.route(), other_binding.route);

        assert_denial!(
            peer.admit_nipc_request(other_route_request, other_binding),
            PeerDenial::RequestIdentityConflict
        );

        // Exercise the same run/app/wire-ID ledger key after the route-A
        // admission has been recorded. Changing only the canonical route is a
        // direct scope conflict, even before any effect owner is consulted.
        let conflicting_scope = ServiceRequestAuthorityScope {
            registration: admitted.registration,
            process: admitted.process,
            task24_identity: admitted.task24_identity.clone(),
            app_session_id: admitted.app_session_id,
            inventory_generation: admitted.inventory_generation,
            session_generation: admitted.session_generation,
            route: other_route,
            endpoint: admitted.endpoint,
            endpoint_epoch: admitted.endpoint_epoch,
            endpoint_recipient: admitted.endpoint_recipient.clone(),
            service_id: admitted.service_id.clone(),
            contract_version: admitted.contract_version,
            operation_id: admitted.operation_id.clone(),
            capability_id: admitted.capability_id.clone(),
            effect_owner: admitted.owner_identity,
            consent_requirement: admitted.consent_requirement,
            fingerprint: admitted.fingerprint,
        };
        let key = ServiceRequestLedgerKey {
            run_generation: admitted.run.generation,
            app_id: admitted.app_id,
            wire_request_id: admitted.stable_request_id(),
        };
        let mut ledger = peer.request_ledger.try_lock().unwrap();
        assert_denial!(
            ledger.admit(key, conflicting_scope, &admitted.run),
            PeerDenial::RequestIdentityConflict
        );
    }

    #[test]
    fn would_block_keeps_partial_frame_offset_and_rejects_new_request_id() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let mut pending = peer.begin_request_frame(wire_request_id(50), 4).unwrap();
        assert_eq!(pending.record_read_result(Ok(2)).unwrap(), 2);
        assert_eq!(
            pending
                .record_read_result(Err(io::ErrorKind::WouldBlock.into()))
                .unwrap(),
            2
        );
        assert_eq!(pending.received_bytes().unwrap(), 2);
        let resumed = peer.begin_request_frame(wire_request_id(50), 4).unwrap();
        assert_eq!(resumed.received_bytes().unwrap(), 2);
        assert_eq!(pending.record_read_result(Ok(2)).unwrap(), 4);
        pending.finish().unwrap();
        assert_denial!(
            peer.begin_request_frame(wire_request_id(51), 4),
            PeerDenial::ConnectionRequestIdentityMismatch
        );
        assert_denial!(
            peer.begin_request_frame(wire_request_id(50), 4),
            PeerDenial::StaleConnection
        );
    }

    #[test]
    fn authority_io_deadline_is_fixed_across_request_continuations() {
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let mut frame = ServiceRequestFrameProgress {
            wire_request_id: wire_request_id(55),
            expected_bytes: 4,
            received_bytes: 2,
            authority_io_deadline: None,
        };

        assert!(frame.bind_authority_io_deadline(deadline).is_ok());
        assert!(frame.bind_authority_io_deadline(deadline).is_ok());
        assert_denial!(
            frame.bind_authority_io_deadline(deadline + std::time::Duration::from_secs(1)),
            PeerDenial::ConnectionRequestIdentityMismatch
        );
        assert_eq!(frame.authority_io_deadline, Some(deadline));
    }

    #[test]
    fn deadline_after_partial_frame_stales_connection_before_admission() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let mut pending = peer.begin_request_frame(wire_request_id(54), 4).unwrap();
        assert_eq!(pending.record_read_result(Ok(2)).unwrap(), 2);
        assert_eq!(
            pending
                .record_read_result(Err(io::ErrorKind::WouldBlock.into()))
                .unwrap(),
            2
        );
        assert!(matches!(
            pending.record_read_result(Err(io::ErrorKind::TimedOut.into())),
            Err(PeerDenial::PeerConnection(_))
        ));
        assert_denial!(pending.finish(), PeerDenial::StaleConnection);
        assert_denial!(
            peer.begin_request_frame(wire_request_id(54), 4),
            PeerDenial::StaleConnection
        );
    }

    #[test]
    fn eof_after_partial_frame_stales_connection_before_admission() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let mut pending = peer.begin_request_frame(wire_request_id(52), 4).unwrap();
        assert_eq!(pending.record_read_result(Ok(2)).unwrap(), 2);
        assert_eq!(
            pending
                .record_read_result(Err(io::ErrorKind::WouldBlock.into()))
                .unwrap(),
            2
        );
        assert_denial!(
            pending.record_read_result(Ok(0)),
            PeerDenial::StaleConnection
        );
        assert_denial!(pending.finish(), PeerDenial::StaleConnection);
        assert_denial!(
            peer.begin_request_frame(wire_request_id(53), 4),
            PeerDenial::StaleConnection
        );
    }

    #[test]
    fn dropping_or_replacing_the_nipc_decoder_stales_its_connection() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let mut peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let decoder = peer.new_nipc_request_decoder(deadline).unwrap();
        assert_denial!(
            peer.new_nipc_request_decoder(deadline),
            PeerDenial::RequestDecoderAlreadyStarted
        );
        drop(decoder);
        assert_denial!(
            peer.new_nipc_request_decoder(deadline),
            PeerDenial::StaleConnection
        );
        assert_denial!(peer.with_live_io(|_| ()), PeerDenial::StaleConnection);
    }

    #[test]
    fn authorized_peer_rejects_a_decoder_created_by_another_connection() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let mut first = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let mut second = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        let mut first_decoder = first.new_nipc_request_decoder(deadline).unwrap();

        assert_denial!(
            second.read_nipc_request(&mut first_decoder),
            PeerDenial::ConnectionRequestIdentityMismatch
        );
        assert_denial!(first.with_live_io(|_| ()), PeerDenial::StaleConnection);
    }

    #[test]
    fn gate_rejection_keeps_same_wire_request_retryable() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 10), b"files.read")
            .unwrap();
        let replay = peer
            .admit_request(complete_request_frame(&peer, 10), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let effect_calls = AtomicU64::new(0);
        fixture
            .readiness
            .with_owner_gate(&fixture.run, None, || {
                assert!(matches!(
                    peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                        effect_calls.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }),
                    Err(PeerDenial::ServiceEffectNotInvoked(cause))
                        if matches!(cause.as_ref(), PeerDenial::Unavailable)
                ));
                assert_eq!(
                    admission.operation_state(),
                    ServiceRequestOperationState::NotInvoked
                );
                Ok(())
            })
            .unwrap();

        assert_eq!(replay.request_id(), admission.request_id());
        peer.commit_effect(&replay, &fixture.readiness, &owner, || {
            effect_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        assert_eq!(effect_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            replay.operation_state(),
            ServiceRequestOperationState::Executed
        );
    }

    #[test]
    fn route_policy_blocks_generic_effect_and_requires_live_consent_path() {
        let fixture = Fixture::new_with_consent_requirement(RouteConsentRequirement::Required);
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 11), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let effect_calls = AtomicU64::new(0);

        assert_denial!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effect_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            PeerDenial::ConsentRequired
        );
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::NotInvoked
        );
        assert_eq!(effect_calls.load(Ordering::SeqCst), 0);

        let consent_owner = FakeSessionOwnedConsent::new();
        let consented = peer
            .admit_request_with_session_consent(
                complete_request_frame(&peer, 11),
                b"files.read",
                &consent_owner,
            )
            .unwrap();
        assert_eq!(consented.request_id(), admission.request_id());
        let consented_owner = FakeServiceOwner::new(consented.clone(), &fixture.readiness);
        peer.commit_consented_effect(
            &consented,
            &consent_owner,
            &fixture.readiness,
            &consented_owner,
            || {
                effect_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(effect_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            consented.operation_state(),
            ServiceRequestOperationState::Executed
        );
    }

    #[test]
    fn concurrent_commits_with_same_admission_run_the_effect_once() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 1), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let effects = AtomicU64::new(0);
        let outcomes = Mutex::new(Vec::new());

        std::thread::scope(|scope| {
            for _ in 0..2 {
                let outcomes = &outcomes;
                let effects = &effects;
                let peer = &peer;
                let admission = &admission;
                let owner = &owner;
                let readiness = &fixture.readiness;
                scope.spawn(move || {
                    let result = peer.commit_effect(admission, readiness, owner, || {
                        effects.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    });
                    outcomes.lock().unwrap().push(result);
                });
            }
        });

        let outcomes = outcomes.into_inner().unwrap();
        assert_eq!(outcomes.len(), 2);
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(owner.commits.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::Executed
        );
        assert_denial!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            PeerDenial::DuplicateRequestCommit
        );
    }

    #[test]
    fn effect_error_after_owner_entry_is_unknown_and_cannot_be_replayed() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 1), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let effects = AtomicUsize::new(0);

        let denial: Result<(), PeerDenial> =
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Err(PeerDenial::Unavailable)
            });
        assert!(matches!(
            denial,
            Err(PeerDenial::ServiceEffectOutcomeUnknown(cause))
                if matches!(cause.as_ref(), PeerDenial::Unavailable)
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert!(matches!(
            peer.commit_effect(&admission.clone(), &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(PeerDenial::ServiceEffectOutcomeUnknown(cause))
                if matches!(cause.as_ref(), PeerDenial::RequestOutcomeUnknown)
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_denial!(
            peer.resolve_request_state(&admission, &fixture.readiness, &owner),
            PeerDenial::Unavailable
        );
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );
    }

    #[test]
    fn partial_effect_panic_becomes_unknown_and_cannot_be_replayed() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 91), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let effects = AtomicUsize::new(0);

        let result: Result<(), PeerDenial> =
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                panic!("partial external effect before panic");
            });
        assert!(matches!(
            result,
            Err(PeerDenial::ServiceEffectOutcomeUnknown(cause))
                if matches!(cause.as_ref(), PeerDenial::RequestOutcomeUnknown)
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );

        assert!(matches!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(PeerDenial::ServiceEffectOutcomeUnknown(_))
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );
    }

    #[test]
    fn owner_error_after_successful_callback_is_unknown_not_duplicate() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 35), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        owner.fail_after_callback_once();
        let effects = AtomicUsize::new(0);

        assert!(matches!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(PeerDenial::ServiceEffectOutcomeUnknown(cause))
                if matches!(cause.as_ref(), PeerDenial::DuplicateRequestCommit)
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );
        assert!(matches!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(PeerDenial::ServiceEffectOutcomeUnknown(cause))
                if matches!(cause.as_ref(), PeerDenial::RequestOutcomeUnknown)
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_denial!(
            peer.resolve_request_state(&admission, &fixture.readiness, &owner),
            PeerDenial::Unavailable
        );
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );
    }

    #[test]
    fn owner_authoritative_resolve_confirms_unknown_as_executed() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 41), b"files.read")
            .unwrap();
        let owner = ResolvingServiceOwner::new(admission.clone(), &fixture.readiness);
        owner.lose_commit_reply_once();
        let effects = AtomicUsize::new(0);

        assert!(matches!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(PeerDenial::ServiceEffectOutcomeUnknown(cause))
                if matches!(cause.as_ref(), PeerDenial::DuplicateRequestCommit)
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );

        // The resolver reads the state written by the owner's commit table;
        // PID 1's RAM state alone could not settle this interrupted reply.
        assert_eq!(
            peer.resolve_request_state(&admission, &fixture.readiness, &owner)
                .unwrap(),
            ServiceRequestOperationState::Executed
        );
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::Executed
        );
        assert_denial!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            PeerDenial::DuplicateRequestCommit
        );
        assert_eq!(effects.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn owner_authoritative_not_invoked_resolution_allows_same_request_retry() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 42), b"files.read")
            .unwrap();
        let stable_id = admission.stable_request_id();
        let owner = ResolvingServiceOwner::new(admission.clone(), &fixture.readiness);
        admission.operation_state.store(
            ServiceRequestOperationState::OutcomeUnknown as u8,
            Ordering::SeqCst,
        );
        owner.set_authoritative_state(ServiceRequestOperationState::NotInvoked);

        assert_eq!(
            peer.resolve_request_state(&admission, &fixture.readiness, &owner)
                .unwrap(),
            ServiceRequestOperationState::NotInvoked
        );
        assert_eq!(admission.stable_request_id(), stable_id);
        let effects = AtomicUsize::new(0);
        peer.commit_effect(&admission, &fixture.readiness, &owner, || {
            effects.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(admission.stable_request_id(), stable_id);
    }

    #[test]
    fn owner_authoritative_unknown_resolution_keeps_request_blocked() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 43), b"files.read")
            .unwrap();
        let owner = ResolvingServiceOwner::new(admission.clone(), &fixture.readiness);
        admission.operation_state.store(
            ServiceRequestOperationState::OutcomeUnknown as u8,
            Ordering::SeqCst,
        );
        owner.set_authoritative_state(ServiceRequestOperationState::OutcomeUnknown);

        assert_eq!(
            peer.resolve_request_state(&admission, &fixture.readiness, &owner)
                .unwrap(),
            ServiceRequestOperationState::OutcomeUnknown
        );
        let effects = AtomicUsize::new(0);
        assert!(matches!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(PeerDenial::ServiceEffectOutcomeUnknown(_))
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn public_owner_resolver_rejects_same_wire_id_with_changed_payload() {
        let first = Fixture::new();
        let first_registration = first.register();
        let first_peer = first
            .authorize(first.process.pid(), first_registration)
            .unwrap();
        let first_admission = first_peer
            .admit_request(
                complete_request_frame_with_content(&first_peer, 44, b"payload-a"),
                b"files.read",
            )
            .unwrap();
        let owner = ResolvingServiceOwner::new(first_admission.clone(), &first.readiness);

        let second = Fixture::new();
        let second_registration = second.register();
        let second_peer = second
            .authorize(second.process.pid(), second_registration)
            .unwrap();
        let second_admission = second_peer
            .admit_request(
                complete_request_frame_with_content(&second_peer, 44, b"payload-b"),
                b"files.read",
            )
            .unwrap();
        assert_denial!(
            second_peer.resolve_request_state(&second_admission, &second.readiness, &owner),
            PeerDenial::RequestIdentityConflict
        );
        assert_eq!(
            second_admission.operation_state(),
            ServiceRequestOperationState::NotInvoked
        );
    }

    #[test]
    fn public_owner_resolver_rejects_same_request_after_new_pid1_run() {
        let first = Fixture::new();
        let first_registration = first.register();
        let first_peer = first
            .authorize(first.process.pid(), first_registration)
            .unwrap();
        let first_admission = first_peer
            .admit_request(
                complete_request_frame_with_content(&first_peer, 48, b"same exact request"),
                b"files.read",
            )
            .unwrap();
        let owner = ResolvingServiceOwner::new(first_admission.clone(), &first.readiness);

        let restarted = Fixture::new();
        let restarted_registration = restarted.register();
        let restarted_peer = restarted
            .authorize(restarted.process.pid(), restarted_registration)
            .unwrap();
        let restarted_admission = restarted_peer
            .admit_request(
                complete_request_frame_with_content(&restarted_peer, 48, b"same exact request"),
                b"files.read",
            )
            .unwrap();
        assert_eq!(
            first_admission.stable_request_id(),
            restarted_admission.stable_request_id()
        );
        assert_eq!(
            first_admission.fingerprint(),
            restarted_admission.fingerprint()
        );
        assert_ne!(
            durable_request_identity_claim(fixture_durable_owner_id(), &first_admission),
            durable_request_identity_claim(fixture_durable_owner_id(), &restarted_admission),
            "ephemeral run authority is separate from the stable request key"
        );
        assert_denial!(
            restarted_peer.resolve_request_state(
                &restarted_admission,
                &restarted.readiness,
                &owner
            ),
            PeerDenial::RequestIdentityConflict
        );
        assert_eq!(
            restarted_admission.operation_state(),
            ServiceRequestOperationState::NotInvoked
        );
    }

    #[test]
    fn task37_validation_uses_exact_decoded_frame_and_fails_closed_without_owner() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let mapper = test_nipc_mapper(fixture.route, 1, "files.read@1");
        let (request, binding) = decode_mapped_request(&peer, &mapper, [49; 16]);
        let admission = peer.admit_nipc_request(request, binding).unwrap();
        let identity = durable_request_identity_claim(fixture_durable_owner_id(), &admission);
        let unavailable = UnavailableServiceEffectOwner;

        assert_denial!(
            admission.validate_with_task37_owner(&unavailable, identity),
            PeerDenial::Unavailable
        );

        let forged_claim = DurableServiceRequestIdentity::new(
            identity.key(),
            DurableServiceRequestBinding::new([0x99; 32], [0x88; 32], [0x77; 32]),
        );
        assert_denial!(
            admission.validate_with_task37_owner(&unavailable, forged_claim),
            PeerDenial::RequestIdentityConflict
        );
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::NotInvoked
        );
    }

    #[test]
    fn public_owner_resolver_rejects_same_wire_id_with_changed_route() {
        let first = Fixture::new();
        let first_registration = first.register();
        let first_peer = first
            .authorize(first.process.pid(), first_registration)
            .unwrap();
        let first_admission = first_peer
            .admit_request(complete_request_frame(&first_peer, 45), b"files.read")
            .unwrap();
        let owner = ResolvingServiceOwner::new(first_admission, &first.readiness);

        let other_route = TrustedRouteKey::from_canonical_route(10, 21, 1, 0, 4).unwrap();
        let second = Fixture::new_with_route_and_consent_requirement(
            other_route,
            RouteConsentRequirement::NotRequired,
        );
        let second_registration = second.register();
        let second_peer = second
            .authorize(second.process.pid(), second_registration)
            .unwrap();
        let second_admission = second_peer
            .admit_request(complete_request_frame(&second_peer, 45), b"files.read")
            .unwrap();
        assert_denial!(
            second_peer.resolve_request_state(&second_admission, &second.readiness, &owner),
            PeerDenial::RequestIdentityConflict
        );
    }

    #[test]
    fn public_owner_resolver_rejects_changed_inventory_or_session_generation() {
        let first = Fixture::new();
        let first_registration = first.register();
        let first_peer = first
            .authorize(first.process.pid(), first_registration)
            .unwrap();
        let first_admission = first_peer
            .admit_request(complete_request_frame(&first_peer, 46), b"files.read")
            .unwrap();
        let owner = ResolvingServiceOwner::new(first_admission, &first.readiness);

        let second = Fixture::new_with_authority_generations(
            TrustedRouteKey::from_canonical_route(10, 20, 1, 0, 4).unwrap(),
            RouteConsentRequirement::NotRequired,
            4,
            89,
        );
        let second_registration = second.register();
        let second_peer = second
            .authorize(second.process.pid(), second_registration)
            .unwrap();
        let second_admission = second_peer
            .admit_request(complete_request_frame(&second_peer, 46), b"files.read")
            .unwrap();
        assert_denial!(
            second_peer.resolve_request_state(&second_admission, &second.readiness, &owner),
            PeerDenial::RequestIdentityConflict
        );
    }

    #[test]
    fn canonical_owner_identity_is_bound_before_first_effect_commit() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 48), b"files.read")
            .unwrap();
        assert_eq!(admission.owner_identity, fixture_durable_owner_id());

        let replacement = ResolvingServiceOwner::new_with_owner_identity(
            admission.clone(),
            &fixture.readiness,
            ServiceEffectOwnerId::new([0x42; 16]),
        );
        let effects = AtomicUsize::new(0);
        assert!(matches!(
            peer.commit_effect(&admission, &fixture.readiness, &replacement, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(PeerDenial::ServiceEffectNotInvoked(_))
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::NotInvoked
        );
    }

    #[test]
    fn process_exit_immediately_before_commit_keeps_effect_uninvoked() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 49), b"files.read")
            .unwrap();
        let owner = ResolvingServiceOwner::new(admission.clone(), &fixture.readiness);
        let effects = AtomicUsize::new(0);

        // This is a host negative against the fixture's retained spawn handle:
        // the exact child is marked exited after peer authorization but before
        // the commit call enters the owner callback.
        fixture
            .process_instances
            .kernel()
            .set_live(FakePidfd(7001), false);
        assert!(matches!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(PeerDenial::ServiceEffectNotInvoked(_))
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 0);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::NotInvoked
        );
    }

    #[test]
    fn ambiguous_request_cannot_be_resolved_through_a_replaced_owner() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 47), b"files.read")
            .unwrap();
        let owner = ResolvingServiceOwner::new(admission.clone(), &fixture.readiness);
        owner.lose_commit_reply_once();
        let effects = AtomicUsize::new(0);
        assert!(matches!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(PeerDenial::ServiceEffectOutcomeUnknown(_))
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );

        let replacement_identity = ServiceEffectOwnerId::new([0x42; 16]);
        let replacement = ResolvingServiceOwner::new_with_owner_identity(
            admission.clone(),
            &fixture.readiness,
            replacement_identity,
        );
        assert_denial!(
            peer.resolve_request_state(&admission, &fixture.readiness, &replacement),
            PeerDenial::RequestIdentityConflict
        );
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );
        assert!(matches!(
            peer.commit_effect(&admission, &fixture.readiness, &replacement, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(PeerDenial::ServiceEffectOutcomeUnknown(_))
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn ambiguous_receipt_owner_result_reconciles_to_recorded_without_replay() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 61), b"files.read")
            .unwrap();
        let request_id = admission.request_id();
        let wire_id = admission.stable_request_id();
        let owner = ResolvingReceiptOwner::new(
            admission.clone(),
            &fixture.readiness,
            ServiceRequestOperationState::ReceiptRecorded,
        );

        assert!(matches!(
            peer.record_package_request_receipt(&admission, &fixture.readiness, &owner),
            Err(PeerDenial::ServiceRequestReceiptNotRecorded(_))
        ));
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );
        assert_eq!(admission.request_id(), request_id);
        assert_eq!(admission.stable_request_id(), wire_id);
        assert_eq!(owner.record_calls.load(Ordering::SeqCst), 1);

        assert_eq!(
            peer.resolve_package_request_receipt(&admission, &fixture.readiness, &owner)
                .unwrap(),
            ServiceRequestOperationState::ReceiptRecorded
        );
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::ReceiptRecorded
        );
        assert_denial!(
            peer.record_package_request_receipt(&admission, &fixture.readiness, &owner),
            PeerDenial::DuplicateRequestCommit
        );
        assert_eq!(owner.record_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn receipt_record_resolution_never_rolls_back_a_local_terminal_state() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 63), b"files.read")
            .unwrap();
        let owner = ResolvingReceiptOwner::new(
            admission.clone(),
            &fixture.readiness,
            ServiceRequestOperationState::NotInvoked,
        );
        admission.operation_state.store(
            ServiceRequestOperationState::ReceiptRecorded as u8,
            Ordering::SeqCst,
        );

        assert_denial!(
            peer.resolve_package_request_receipt_record(&admission, &fixture.readiness, &owner),
            PeerDenial::ServiceEffectOutcomeUnknown(_)
        );
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );
        assert_eq!(owner.record_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn receipt_owner_identity_is_bound_before_first_lookup_or_record() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 60), b"files.read")
            .unwrap();
        let replacement = ResolvingReceiptOwner::new_with_owner_identity(
            admission.clone(),
            &fixture.readiness,
            ServiceRequestOperationState::ReceiptRecorded,
            ServiceEffectOwnerId::new([0x42; 16]),
        );

        assert_denial!(
            peer.resolve_package_request_receipt(&admission, &fixture.readiness, &replacement),
            PeerDenial::RequestIdentityConflict
        );
        assert_eq!(replacement.record_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::NotInvoked
        );
    }

    #[test]
    fn owner_confirmed_unrecorded_receipt_allows_same_id_retry_only() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 62), b"files.read")
            .unwrap();
        let request_id = admission.request_id();
        let wire_id = admission.stable_request_id();
        let owner = ResolvingReceiptOwner::new(
            admission.clone(),
            &fixture.readiness,
            ServiceRequestOperationState::NotInvoked,
        );

        assert!(matches!(
            peer.record_package_request_receipt(&admission, &fixture.readiness, &owner),
            Err(PeerDenial::ServiceRequestReceiptNotRecorded(_))
        ));
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );
        assert_eq!(
            peer.resolve_package_request_receipt(&admission, &fixture.readiness, &owner)
                .unwrap(),
            ServiceRequestOperationState::NotInvoked
        );
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::NotInvoked
        );
        assert_eq!(admission.request_id(), request_id);
        assert_eq!(admission.stable_request_id(), wire_id);

        // This fake owner keeps failing, but the retry is reached only after
        // its authoritative NotInvoked result and uses the original identity.
        assert!(matches!(
            peer.record_package_request_receipt(&admission, &fixture.readiness, &owner),
            Err(PeerDenial::ServiceRequestReceiptNotRecorded(_))
        ));
        assert_eq!(owner.record_calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );
    }

    #[test]
    fn unresolved_or_unavailable_receipt_lookup_never_retries() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 63), b"files.read")
            .unwrap();
        let owner = ResolvingReceiptOwner::new(
            admission.clone(),
            &fixture.readiness,
            ServiceRequestOperationState::OutcomeUnknown,
        );

        assert!(matches!(
            peer.record_package_request_receipt(&admission, &fixture.readiness, &owner),
            Err(PeerDenial::ServiceRequestReceiptNotRecorded(_))
        ));
        assert_eq!(
            peer.resolve_package_request_receipt(&admission, &fixture.readiness, &owner)
                .unwrap(),
            ServiceRequestOperationState::OutcomeUnknown
        );
        assert_denial!(
            peer.record_package_request_receipt(&admission, &fixture.readiness, &owner),
            PeerDenial::ServiceEffectOutcomeUnknown(_)
        );
        assert_eq!(owner.record_calls.load(Ordering::SeqCst), 1);

        owner.disable_lookup();
        assert_denial!(
            peer.resolve_package_request_receipt(&admission, &fixture.readiness, &owner),
            PeerDenial::Unavailable
        );
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );
    }

    #[test]
    fn receipt_lookup_rejects_same_wire_id_with_different_payload() {
        let first_fixture = Fixture::new();
        let first_registration = first_fixture.register();
        let first_peer = first_fixture
            .authorize(first_fixture.process.pid(), first_registration)
            .unwrap();
        let first_admission = first_peer
            .admit_request(
                complete_request_frame_with_content(&first_peer, 64, b"payload-one"),
                b"files.read",
            )
            .unwrap();
        let owner = ResolvingReceiptOwner::new(
            first_admission.clone(),
            &first_fixture.readiness,
            ServiceRequestOperationState::OutcomeUnknown,
        );
        assert!(matches!(
            first_peer.record_package_request_receipt(
                &first_admission,
                &first_fixture.readiness,
                &owner
            ),
            Err(PeerDenial::ServiceRequestReceiptNotRecorded(_))
        ));

        let second_fixture = Fixture::new();
        let second_registration = second_fixture.register();
        let second_peer = second_fixture
            .authorize(second_fixture.process.pid(), second_registration)
            .unwrap();
        let second_admission = second_peer
            .admit_request(
                complete_request_frame_with_content(&second_peer, 64, b"payload-two"),
                b"files.read",
            )
            .unwrap();
        assert_eq!(
            first_admission.stable_request_id(),
            second_admission.stable_request_id()
        );
        assert_ne!(first_admission.fingerprint, second_admission.fingerprint);
        owner.resolve_as(second_admission);

        assert_denial!(
            first_peer.resolve_package_request_receipt(
                &first_admission,
                &first_fixture.readiness,
                &owner
            ),
            PeerDenial::RequestIdentityConflict
        );
        assert_eq!(
            first_admission.operation_state(),
            ServiceRequestOperationState::OutcomeUnknown
        );
        assert_eq!(owner.record_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn in_progress_stays_unresolved_without_owner_state_lookup() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 36), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        owner.report_in_progress_once();
        let effects = AtomicUsize::new(0);

        assert_denial!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            PeerDenial::RequestInProgress
        );
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::Executing
        );
        assert_denial!(
            peer.resolve_request_state(&admission, &fixture.readiness, &owner),
            PeerDenial::Unavailable
        );
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::Executing
        );
        assert_denial!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            PeerDenial::RequestInProgress
        );
        assert_eq!(effects.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn owner_can_mark_definite_failure_but_pid1_will_not_retry_it() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 33), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let effects = AtomicUsize::new(0);

        assert!(matches!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || -> Result<(), PeerDenial> {
                effects.fetch_add(1, Ordering::SeqCst);
                Err(PeerDenial::ServiceEffectFailed(Arc::new(
                    PeerDenial::NotDeclared,
                )))
            }),
            Err(PeerDenial::ServiceEffectFailed(cause))
                if matches!(cause.as_ref(), PeerDenial::NotDeclared)
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::Failed
        );
        assert_denial!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            PeerDenial::ServiceEffectFailed(_)
        );
        assert_eq!(effects.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn service_owner_transaction_rejects_a_different_session_generation() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 1), b"files.read")
            .unwrap();
        let mut other_session = admission.clone();
        other_session.app_session_id += 1;
        let owner = FakeServiceOwner::new(other_session, &fixture.readiness);
        let effect_ran = AtomicBool::new(false);

        let denial = peer.commit_effect(&admission, &fixture.readiness, &owner, || {
            effect_ran.store(true, Ordering::SeqCst);
            Ok(())
        });
        assert!(matches!(
            denial,
            Err(PeerDenial::ServiceEffectNotInvoked(cause))
                if matches!(cause.as_ref(), PeerDenial::StaleRegistration)
        ));
        assert!(!effect_ran.load(Ordering::SeqCst));
        assert_eq!(owner.commits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn boot_package_owner_lease_covers_the_authorized_peer_effect_commit() {
        let package_path = std::env::var_os("NAGI_DEV_PREVIEW_PACKAGE")
            .expect("NAGI_DEV_PREVIEW_PACKAGE must identify the signed task-25 XAPP");
        let package_bytes = std::fs::read(package_path).expect("read signed task-25 XAPP");
        let package = PackageView::parse(&package_bytes).expect("parse signed XAPP");
        let app_id = package.manifest().app_id();
        let manifest_id = package.manifest().id().to_vec();

        let mut boot_owner =
            nagi_init_package_provider::package_trust::PackageServiceSessionOwner::new(
                Some(&TRUSTED_SIGNING_PUBLIC_KEY),
                PackageService::new(),
            );
        boot_owner
            .install(
                &package_bytes,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
            )
            .expect("install into the already-owned boot session PackageService");
        let snapshot = boot_owner
            .capture(&package_bytes, &manifest_id)
            .expect("capture through the same boot owner");
        let generation = NonZeroU64::new(snapshot.generation()).unwrap();

        let fixture = Fixture::new_with_application_generations(
            TrustedRouteKey::from_canonical_route(10, 20, 1, 0, 4).unwrap(),
            RouteConsentRequirement::NotRequired,
            snapshot.generation(),
            88,
            app_id.0,
        );
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .expect("fixture endpoint owner accepts the registered listener");
        let admission = peer
            .admit_request(complete_request_frame(&peer, 114), b"files.read")
            .expect("admit the exact decoded request");
        let effect_owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let package_authority =
            package_service_inventory_adapter::SerializedPackageServiceAuthority::capture_for_registration(
                &boot_owner,
                &package_bytes,
                &manifest_id,
                &fixture.run_owner_lease,
                &fixture.directory,
                registration,
                fixture.process,
                &fixture.readiness,
            )
            .expect("borrow the exact boot DevPreview owner instance");
        let callback_count = AtomicUsize::new(0);

        peer.commit_effect_with_package_generation_lease(
            &admission,
            &fixture.readiness,
            &effect_owner,
            &package_authority,
            || {
                assert!(package_authority
                    .generation_is_current(app_id.0, generation)
                    .expect("revalidate under the same package lease"));
                callback_count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
        )
        .expect("run the service-owner callback while the same package lease is live");

        assert_eq!(callback_count.load(Ordering::SeqCst), 1);
        assert_eq!(effect_owner.commits.load(Ordering::SeqCst), 1);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::Executed
        );
        drop(package_authority);
        boot_owner
            .remove(app_id)
            .expect("package owner is usable after the commit wrapper returns");
    }

    #[test]
    fn task25_receipt_view_is_scoped_to_connection_session_revocation_and_run() {
        let package_path = std::env::var_os("NAGI_DEV_PREVIEW_PACKAGE")
            .expect("NAGI_DEV_PREVIEW_PACKAGE must identify the signed task-25 XAPP");
        let package_bytes = std::fs::read(package_path).expect("read signed task-25 XAPP");
        let package = PackageView::parse(&package_bytes).expect("parse signed XAPP");
        let app_id = package.manifest().app_id();
        let manifest_id = package.manifest().id().to_vec();

        let task25_owner = nagi_dev_service_catalog_consumer::PackageServiceSessionOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        task25_owner
            .install(
                &package_bytes,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
            )
            .expect("install through the task-25 candidate owner");
        let snapshot = task25_owner
            .capture(&package_bytes, &manifest_id)
            .expect("capture the current task-25 candidate owner generation");

        let fixture = Fixture::new_with_application_generations(
            TrustedRouteKey::from_canonical_route(10, 20, 1, 0, 4).unwrap(),
            RouteConsentRequirement::NotRequired,
            snapshot.generation(),
            88,
            app_id.0,
        );
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .expect("authorize the fixture's registered endpoint");
        let admission = peer
            .admit_request(complete_request_frame(&peer, 116), b"files.read")
            .expect("admit one decoded request");
        let package_authority =
            package_service_inventory_adapter::SerializedPackageServiceAuthority::capture_for_registration(
                &task25_owner,
                &package_bytes,
                &manifest_id,
                &fixture.run_owner_lease,
                &fixture.directory,
                registration,
                fixture.process,
                &fixture.readiness,
            )
            .expect("borrow the exact task-25 candidate PackageService owner");
        package_authority
            .bind_admission(&admission, &fixture.readiness)
            .expect("bind the request to this owner instance");
        let receipt_owner = FixtureTask25ReceiptTransaction {
            authority: &package_authority,
            admission: admission.clone(),
            owner_id: admission.owner_identity,
        };

        let mut original = peer
            .record_package_request_receipt(&admission, &fixture.readiness, &receipt_owner)
            .expect("record the task-25 local generation receipt");
        let receipt_id = peer
            .with_current_package_request_receipt(
                &mut original,
                &fixture.readiness,
                &receipt_owner,
                |receipt| Ok(receipt.record_id()),
            )
            .expect("revalidate receipt before reading it");
        assert_eq!(receipt_id, admission.request_id().get());
        assert!(!original.is_invalidated());

        let mut wrong_connection_view = peer
            .resolve_package_request_receipt_record(&admission, &fixture.readiness, &receipt_owner)
            .expect("resolve a fresh connection-scoped view");
        let replacement_peer = fixture
            .authorize(fixture.process.pid(), registration)
            .expect("accept a distinct connection to the same registered endpoint");
        assert_denial!(
            replacement_peer.with_current_package_request_receipt(
                &mut wrong_connection_view,
                &fixture.readiness,
                &receipt_owner,
                |_| Ok(()),
            ),
            PeerDenial::StaleConnection
        );
        assert!(wrong_connection_view.is_invalidated());

        let mut revoked_session_view = peer
            .resolve_package_request_receipt_record(&admission, &fixture.readiness, &receipt_owner)
            .expect("resolve another current receipt view");
        fixture.sessions.live.store(false, Ordering::SeqCst);
        assert_denial!(
            peer.with_current_package_request_receipt(
                &mut revoked_session_view,
                &fixture.readiness,
                &receipt_owner,
                |_| Ok(()),
            ),
            PeerDenial::SessionRevoked
        );
        assert!(revoked_session_view.is_invalidated());
        fixture.sessions.live.store(true, Ordering::SeqCst);

        let mut revoked_admission_view = peer
            .resolve_package_request_receipt_record(&admission, &fixture.readiness, &receipt_owner)
            .expect("resolve before request revocation");
        let mut ended_run_view = peer
            .resolve_package_request_receipt_record(&admission, &fixture.readiness, &receipt_owner)
            .expect("resolve before run shutdown");
        assert!(package_authority
            .revoke_admission(&admission, &fixture.readiness)
            .expect("revoke request under owner gate"));
        assert_denial!(
            peer.with_current_package_request_receipt(
                &mut revoked_admission_view,
                &fixture.readiness,
                &receipt_owner,
                |_| Ok(()),
            ),
            PeerDenial::StaleRegistration
        );
        assert!(revoked_admission_view.is_invalidated());

        fixture
            .directory
            .end_run_owner_lease(&fixture.run_owner_lease, &fixture.readiness)
            .expect("end this fixture run");
        assert_denial!(
            peer.with_current_package_request_receipt(
                &mut ended_run_view,
                &fixture.readiness,
                &receipt_owner,
                |_| Ok(()),
            ),
            PeerDenial::StaleRun
        );
        assert!(ended_run_view.is_invalidated());
    }

    #[test]
    fn supervisor_retirement_revokes_registered_package_receipts_and_releases_terminal_slots() {
        let package_path = std::env::var_os("NAGI_DEV_PREVIEW_PACKAGE")
            .expect("NAGI_DEV_PREVIEW_PACKAGE must identify the signed task-25 XAPP");
        let package_bytes = std::fs::read(package_path).expect("read signed task-25 XAPP");
        let package = PackageView::parse(&package_bytes).expect("parse signed XAPP");
        let app_id = package.manifest().app_id();
        let manifest_id = package.manifest().id().to_vec();
        let owner = nagi_dev_service_catalog_consumer::PackageServiceSessionOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        owner
            .install(
                &package_bytes,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
            )
            .expect("install through task-25 PackageService owner");
        let snapshot = owner
            .capture(&package_bytes, &manifest_id)
            .expect("capture installed generation");
        let fixture = Fixture::new_with_application_generations(
            TrustedRouteKey::from_canonical_route(10, 20, 1, 0, 4).unwrap(),
            RouteConsentRequirement::NotRequired,
            snapshot.generation(),
            88,
            app_id.0,
        );
        let registration = fixture.register();
        let authority = package_service_inventory_adapter::SerializedPackageServiceAuthority::capture_for_registration(
            &owner,
            &package_bytes,
            &manifest_id,
            &fixture.run_owner_lease,
            &fixture.directory,
            registration,
            fixture.process,
            &fixture.readiness,
        )
        .expect("capture and register receipt owner for exact live endpoint");

        let mut admissions = Vec::with_capacity(32);
        for raw_request in 1..=32u8 {
            let peer = fixture
                .authorize(fixture.process.pid(), registration)
                .expect("open a separate authenticated fixture connection");
            let admission = peer
                .admit_request(complete_request_frame(&peer, raw_request), b"files.read")
                .expect("admit one request on this connection");
            authority
                .bind_admission(&admission, &fixture.readiness)
                .expect("bind receipt slot to exact package generation");
            let receipt_owner = FixtureTask25ReceiptTransaction {
                authority: &authority,
                admission: admission.clone(),
                owner_id: admission.owner_identity,
            };
            let _receipt = peer
                .record_package_request_receipt(&admission, &fixture.readiness, &receipt_owner)
                .expect("record terminal local package receipt");
            admissions.push(admission);
        }
        assert_eq!(authority.test_admission_count().unwrap(), 32);
        assert_eq!(authority.test_outcome_snapshot().unwrap().len(), 32);

        // Exercise the existing owner sequence: Supervisor revoke succeeds
        // first, then the PID1 directory retires the exact registration.
        fixture
            .readiness
            .with_owner_gate(&fixture.run, None, || {
                let mut current = fixture
                    .supervisor
                    .record
                    .lock()
                    .map_err(|_| PeerDenial::Unavailable)?;
                if *current != Some(fixture.record) {
                    return Err(PeerDenial::NotLive);
                }
                *current = None;
                Ok(())
            })
            .unwrap();
        assert!(fixture
            .directory
            .retire_after_supervisor_revoke(
                &fixture.run,
                fixture.process,
                registration,
                &fixture.readiness,
            )
            .unwrap());

        assert_eq!(authority.test_admission_count().unwrap(), 0);
        assert!(authority.test_outcome_snapshot().unwrap().is_empty());
        assert!(matches!(
            authority.bind_admission(&admissions[0], &fixture.readiness),
            Err(PeerDenial::StaleRegistration)
        ));
        assert!(matches!(
            package_service_inventory_adapter::SerializedPackageServiceAuthority::capture_for_registration(
                &owner,
                &package_bytes,
                &manifest_id,
                &fixture.run_owner_lease,
                &fixture.directory,
                registration,
                fixture.process,
                &fixture.readiness,
            ),
            Err(PeerDenial::StaleRegistration)
        ));
    }

    #[test]
    fn run_end_revokes_registered_package_receipts_but_keeps_unknown_tombstones() {
        let package_path = std::env::var_os("NAGI_DEV_PREVIEW_PACKAGE")
            .expect("NAGI_DEV_PREVIEW_PACKAGE must identify the signed task-25 XAPP");
        let package_bytes = std::fs::read(package_path).expect("read signed task-25 XAPP");
        let package = PackageView::parse(&package_bytes).expect("parse signed XAPP");
        let app_id = package.manifest().app_id();
        let manifest_id = package.manifest().id().to_vec();
        let owner = nagi_dev_service_catalog_consumer::PackageServiceSessionOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        owner
            .install(
                &package_bytes,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
            )
            .expect("install through task-25 PackageService owner");
        let snapshot = owner
            .capture(&package_bytes, &manifest_id)
            .expect("capture installed generation");
        let fixture = Fixture::new_with_application_generations(
            TrustedRouteKey::from_canonical_route(10, 20, 1, 0, 4).unwrap(),
            RouteConsentRequirement::NotRequired,
            snapshot.generation(),
            88,
            app_id.0,
        );
        let registration = fixture.register();
        let authority = package_service_inventory_adapter::SerializedPackageServiceAuthority::capture_for_registration(
            &owner,
            &package_bytes,
            &manifest_id,
            &fixture.run_owner_lease,
            &fixture.directory,
            registration,
            fixture.process,
            &fixture.readiness,
        )
        .expect("capture and register receipt owner for exact live endpoint");

        let mut admissions = Vec::with_capacity(4);
        for raw_request in 33..=36u8 {
            let peer = fixture
                .authorize(fixture.process.pid(), registration)
                .expect("open a separate authenticated fixture connection");
            let admission = peer
                .admit_request(complete_request_frame(&peer, raw_request), b"files.read")
                .expect("admit one request on this connection");
            authority
                .bind_admission(&admission, &fixture.readiness)
                .expect("bind receipt slot to exact package generation");
            match raw_request {
                33 => {
                    let receipt_owner = FixtureTask25ReceiptTransaction {
                        authority: &authority,
                        admission: admission.clone(),
                        owner_id: admission.owner_identity,
                    };
                    let _receipt = peer
                        .record_package_request_receipt(
                            &admission,
                            &fixture.readiness,
                            &receipt_owner,
                        )
                        .expect("record terminal local package receipt");
                }
                34 => authority
                    .test_set_outcome_state(&admission, ServiceRequestOperationState::Executing)
                    .unwrap(),
                35 => authority
                    .test_set_outcome_state(
                        &admission,
                        ServiceRequestOperationState::OutcomeUnknown,
                    )
                    .unwrap(),
                _ => {}
            }
            admissions.push(admission);
        }
        assert_eq!(authority.test_admission_count().unwrap(), 4);
        assert_eq!(authority.test_outcome_snapshot().unwrap().len(), 3);

        fixture
            .directory
            .end_run_owner_lease(&fixture.run_owner_lease, &fixture.readiness)
            .expect("end run through the existing lifecycle owner");

        assert!(!fixture.run_owner_lease.is_active());
        assert_eq!(authority.test_admission_count().unwrap(), 0);
        let outcomes = authority.test_outcome_snapshot().unwrap();
        assert_eq!(outcomes.len(), 2);
        assert!(outcomes.iter().all(|(request_id, state, receipt)| {
            *state == ServiceRequestOperationState::OutcomeUnknown
                && !*receipt
                && admissions.iter().any(|admission| {
                    admission.request_id() == *request_id
                        && matches!(admission.stable_request_id().as_bytes()[0], 34 | 35)
                })
        }));
        assert!(matches!(
            authority.bind_admission(&admissions[3], &fixture.readiness),
            Err(PeerDenial::StaleRun)
        ));
    }

    #[test]
    fn package_generation_effect_wrapper_rejects_required_consent_without_event() {
        let package_path = std::env::var_os("NAGI_DEV_PREVIEW_PACKAGE")
            .expect("NAGI_DEV_PREVIEW_PACKAGE must identify the signed task-25 XAPP");
        let package_bytes = std::fs::read(package_path).expect("read signed task-25 XAPP");
        let package = PackageView::parse(&package_bytes).expect("parse signed XAPP");
        let app_id = package.manifest().app_id();
        let manifest_id = package.manifest().id().to_vec();
        let mut boot_owner =
            nagi_init_package_provider::package_trust::PackageServiceSessionOwner::new(
                Some(&TRUSTED_SIGNING_PUBLIC_KEY),
                PackageService::new(),
            );
        boot_owner
            .install(
                &package_bytes,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
            )
            .expect("install through the boot session owner");
        let snapshot = boot_owner
            .capture(&package_bytes, &manifest_id)
            .expect("capture through the same boot owner");
        let fixture = Fixture::new_with_application_generations(
            TrustedRouteKey::from_canonical_route(10, 20, 1, 0, 4).unwrap(),
            RouteConsentRequirement::Required,
            snapshot.generation(),
            88,
            app_id.0,
        );
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .expect("fixture endpoint owner accepts the registered listener");
        let admission = peer
            .admit_request(complete_request_frame(&peer, 115), b"files.read")
            .expect("admission does not itself grant consent");
        let effect_owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let package_authority =
            package_service_inventory_adapter::SerializedPackageServiceAuthority::capture_for_registration(
                &boot_owner,
                &package_bytes,
                &manifest_id,
                &fixture.run_owner_lease,
                &fixture.directory,
                registration,
                fixture.process,
                &fixture.readiness,
            )
            .expect("borrow the exact boot DevPreview owner instance");
        let callback_count = AtomicUsize::new(0);

        assert_denial!(
            peer.commit_effect_with_package_generation_lease(
                &admission,
                &fixture.readiness,
                &effect_owner,
                &package_authority,
                || {
                    callback_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
            ),
            PeerDenial::ConsentRequired
        );
        assert_eq!(callback_count.load(Ordering::SeqCst), 0);
        assert_eq!(effect_owner.commits.load(Ordering::SeqCst), 0);
        assert_eq!(
            admission.operation_state(),
            ServiceRequestOperationState::NotInvoked
        );
    }

    #[test]
    fn inventory_generation_change_after_request_admission_denies_effect() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 1), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let effect_ran = AtomicBool::new(false);

        fixture.inventory.generation.store(4, Ordering::SeqCst);
        let denial = peer.commit_effect(&admission, &fixture.readiness, &owner, || {
            effect_ran.store(true, Ordering::SeqCst);
            Ok(())
        });
        assert!(matches!(
            denial,
            Err(PeerDenial::ServiceEffectNotInvoked(cause))
                if matches!(cause.as_ref(), PeerDenial::StaleManifest)
        ));
        assert!(!effect_ran.load(Ordering::SeqCst));
        assert_eq!(owner.commits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn service_owner_revoke_and_effect_commit_share_the_pid1_gate() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 1), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let first_effect = AtomicBool::new(false);

        peer.commit_effect(&admission, &fixture.readiness, &owner, || {
            first_effect.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        assert!(first_effect.load(Ordering::SeqCst));
        assert_eq!(owner.commits.load(Ordering::SeqCst), 1);

        owner.revoke(&fixture.readiness, &fixture.run).unwrap();
        assert_denial!(
            peer.commit_effect(&admission, &fixture.readiness, &owner, || Ok(())),
            PeerDenial::DuplicateRequestCommit
        );

        let (fresh_stream, _fresh_remote) = UnixStream::pair().unwrap();
        let fresh_checkout = fixture.checkout(fixture.endpoint, fresh_stream).unwrap();
        let fresh_peer = fixture
            .authorize_checkout(fresh_checkout, registration, &fixture.requirements())
            .unwrap();
        let fresh_admission = fresh_peer
            .admit_request(complete_request_frame(&fresh_peer, 2), b"files.read")
            .unwrap();
        let second_effect = AtomicBool::new(false);
        let denial = fresh_peer.commit_effect(&fresh_admission, &fixture.readiness, &owner, || {
            second_effect.store(true, Ordering::SeqCst);
            Ok(())
        });
        assert!(matches!(
            denial,
            Err(PeerDenial::ServiceEffectNotInvoked(cause))
                if matches!(cause.as_ref(), PeerDenial::StaleRegistration)
        ));
        assert!(!second_effect.load(Ordering::SeqCst));
        assert_eq!(owner.commits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn request_frame_prefix_may_be_sent_but_revoked_effect_commit_is_denied() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let (stream, mut remote) = UnixStream::pair().unwrap();
        let checkout = fixture.checkout(fixture.endpoint, stream).unwrap();
        let mut peer = fixture
            .authorize_checkout(checkout, registration, &fixture.requirements())
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 1), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);

        // A prefix of the logical request frame has crossed the socket. The
        // adapter cannot retract transport bytes after a later revocation.
        assert_eq!(
            peer.with_live_io(|io| io.write(b"HEAD")).unwrap().unwrap(),
            4
        );
        let mut sent = [0; 4];
        remote.read_exact(&mut sent).unwrap();
        assert_eq!(&sent, b"HEAD");

        fixture.sessions.live.store(false, Ordering::SeqCst);
        let effect_ran = AtomicBool::new(false);
        let denial = peer.commit_effect(&admission, &fixture.readiness, &owner, || {
            effect_ran.store(true, Ordering::SeqCst);
            Ok(())
        });
        assert!(matches!(
            denial,
            Err(PeerDenial::ServiceEffectNotInvoked(cause))
                if matches!(cause.as_ref(), PeerDenial::SessionRevoked)
        ));
        assert!(!effect_ran.load(Ordering::SeqCst));
        assert_eq!(owner.commits.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn effect_commit_requires_the_same_pid1_authority_domain() {
        let fixture = Fixture::new();
        let registration = fixture.register();
        let peer = fixture
            .authorize(fixture.process.pid(), registration)
            .unwrap();
        let admission = peer
            .admit_request(complete_request_frame(&peer, 1), b"files.read")
            .unwrap();
        let owner = FakeServiceOwner::new(admission.clone(), &fixture.readiness);
        let foreign_gate = FakeReadinessBoundary::new();
        let effect_ran = AtomicBool::new(false);

        assert_denial!(
            peer.commit_effect(&admission, &foreign_gate, &owner, || {
                effect_ran.store(true, Ordering::SeqCst);
                Ok(())
            }),
            PeerDenial::Unavailable
        );
        assert!(!effect_ran.load(Ordering::SeqCst));
    }

    #[test]
    fn request_io_authority_domain_guard_rejects_a_foreign_gate() {
        let fixture = Fixture::new();
        let foreign_gate = FakeReadinessBoundary::new();
        assert!(matches!(
            require_same_authority_domain(&foreign_gate, fixture.readiness.authority_domain()),
            Err(PeerDenial::Unavailable)
        ));
        assert!(require_same_authority_domain(
            &fixture.readiness,
            fixture.readiness.authority_domain(),
        )
        .is_ok());
    }

    #[test]
    fn old_registration_cannot_retire_a_replacement() {
        let fixture = Fixture::new();
        let old_process = fixture.process;
        let old = fixture.register();
        *fixture.supervisor.record.lock().unwrap() = None;
        assert!(fixture
            .directory
            .retire_after_supervisor_revoke(&fixture.run, old_process, old, &fixture.readiness,)
            .unwrap());
        assert!(fixture
            .process_instances
            .retire_after_revoke(old_process)
            .unwrap());
        fixture
            .process_instances
            .kernel()
            .set_live(FakePidfd(7002), true);
        let new_process = fixture
            .process_instances
            .record_test_spawn_handle(41, FakePidfd(7002))
            .unwrap();
        *fixture.supervisor.record.lock().unwrap() = Some(fixture.record);
        fixture.fake_peer_pidfd(FakePidfd(7002), true);
        let proof = fixture
            .process_instances
            .issue_spawn_proof(new_process, fixture.record)
            .unwrap()
            .attach_task24_test_identity(nagi_init_spawn_contract::test_identity(
                fixture.record.pid,
            ))
            .unwrap();
        let replacement = fixture
            .register_proof(proof, &fixture.requirements(), fixture.endpoint)
            .unwrap();
        assert_ne!(old, replacement);
        assert!(!fixture
            .directory
            .retire_after_supervisor_revoke(&fixture.run, old_process, old, &fixture.readiness,)
            .unwrap());
        let (stream, _peer) = UnixStream::pair().unwrap();
        let checkout = fixture.checkout(fixture.endpoint, stream).unwrap();
        assert!(fixture
            .authorize_checkout(checkout, replacement, &fixture.requirements())
            .is_ok());
    }
}

/// Runtime-only route-binding negative. All route/endpoint and registry
/// authorities in this helper are explicit fixtures; it exercises the real
/// public adapter's early route rejection before any fixture can authorize.
#[cfg(all(target_os = "linux", feature = "linux-runtime-fixture"))]
pub(crate) mod linux_runtime_fixture_support {
    use super::pidfd_process_instance::{
        LinuxPidfdKernel, PidfdKernel, PidfdProcessInstanceAuthority,
    };
    use super::*;
    use nagi_init_spawn_contract::SpawnedProcessPidfd;
    use nagi_service_contract::{
        CapabilityId, ContractVersion, OperationId, ServiceDescriptor, ServiceId, ServiceOperation,
    };
    use nagi_task37_receipt_supplier::service_effect::ServiceEffectOwnerId;
    use std::collections::HashSet;
    use std::io::{Read, Write};
    use std::num::NonZeroU64;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::atomic::{AtomicU32, AtomicU8, AtomicUsize};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct FixtureRecord;

    impl LaunchRecordView for FixtureRecord {
        fn process_id(self) -> u32 {
            42
        }

        fn app_id(self) -> u64 {
            7
        }

        fn app_session_id(self) -> u64 {
            9
        }
    }

    struct FixtureEndpoints {
        recipient: EndpointCheckoutRecipient,
    }
    impl endpoint_authority_seal::Sealed for FixtureEndpoints {}
    impl TrustedServiceEndpointAuthority for FixtureEndpoints {
        fn is_current(&self, endpoint: TrustedServiceEndpoint) -> Result<bool, PeerDenial> {
            Ok(endpoint == self.recipient.endpoint)
        }

        fn checkout_recipient(
            &self,
            endpoint: TrustedServiceEndpoint,
        ) -> Result<EndpointCheckoutRecipient, PeerDenial> {
            if !self.is_current(endpoint)? {
                return Err(PeerDenial::StaleEndpoint);
            }
            Ok(self.recipient.clone())
        }

        fn fd_delegation_is_prevented(
            &self,
            _endpoint: TrustedServiceEndpoint,
        ) -> Result<bool, PeerDenial> {
            Ok(false)
        }

        fn local_socket_cookie(&self, _stream: &UnixStream) -> Result<NonZeroU64, PeerDenial> {
            Ok(NonZeroU64::new(1).expect("nonzero fixture cookie"))
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
            if Instant::now() >= deadline {
                return Err(PeerDenial::RequestDeadlineExpired);
            }
            if !self.is_current(endpoint)? {
                return Err(PeerDenial::StaleEndpoint);
            }
            let (stream, _peer) = UnixStream::pair().map_err(|_| PeerDenial::Unavailable)?;
            let cookie = self.local_socket_cookie(&stream)?;
            Ok(EndpointCheckout::from_registered_accept(
                self.checkout_recipient(endpoint)?,
                deadline,
                cookie,
                stream,
            ))
        }
    }

    struct FixtureReadinessBoundary(Pid1AuthorityDomain);
    impl readiness_boundary_seal::Sealed for FixtureReadinessBoundary {}
    impl Pid1ReadinessAtomicBoundary for FixtureReadinessBoundary {
        fn authority_domain(&self) -> &Pid1AuthorityDomain {
            &self.0
        }

        fn with_owner_gate<T>(
            &self,
            _run: &Pid1RunInstance,
            _required: Option<&RequiredServiceSet>,
            operation: impl FnOnce() -> Result<T, PeerDenial>,
        ) -> Result<T, PeerDenial> {
            operation()
        }
    }

    struct FixtureInventory;
    impl TrustedInventory for FixtureInventory {
        fn installed_manifest(&self, _app_id: u64) -> Result<Option<TrustedManifest>, PeerDenial> {
            Ok(None)
        }

        fn generation_is_current(
            &self,
            _app_id: u64,
            _generation: NonZeroU64,
        ) -> Result<bool, PeerDenial> {
            Ok(false)
        }

        fn with_current_manifests(
            &self,
            _requested: &[(u64, NonZeroU64)],
            _inspect: &mut dyn FnMut(&[TrustedManifest]) -> Result<(), PeerDenial>,
        ) -> Result<(), PeerDenial> {
            Err(PeerDenial::Unavailable)
        }
    }

    struct FixtureSessions;
    impl LiveSessionAuthority for FixtureSessions {
        fn capture_live_unlocked(
            &self,
            _app_id: u64,
            _app_session_id: u64,
        ) -> Result<SessionLease, PeerDenial> {
            Err(PeerDenial::SessionUnavailable)
        }

        fn is_live_unlocked(&self, _lease: SessionLease) -> Result<bool, PeerDenial> {
            Ok(false)
        }
    }

    struct FixtureConsent;
    impl ConsentSource for FixtureConsent {
        fn connected(&self) -> Result<bool, PeerDenial> {
            Ok(false)
        }
    }

    struct FixtureProcessInstances;
    impl peer_process_instance_seal::Sealed for FixtureProcessInstances {}
    impl PeerProcessInstanceAuthority for FixtureProcessInstances {
        fn confirms_same_live_process(
            &self,
            _stream: &UnixStream,
            _peer_pid: u32,
            _peer_uid: u32,
            _peer_gid: u32,
            _spawned: ProcessInstanceKey,
        ) -> Result<bool, PeerDenial> {
            Ok(false)
        }

        fn confirms_spawned_instance_live(
            &self,
            _spawned: ProcessInstanceKey,
        ) -> Result<bool, PeerDenial> {
            Ok(false)
        }

        fn has_unresolved_cleanup(&self) -> Result<bool, PeerDenial> {
            Ok(false)
        }
    }

    struct FixtureSupervisor;
    impl SupervisorLaunchAuthority for FixtureSupervisor {
        type Record = FixtureRecord;

        fn resolve(&self, _process_id: u32) -> Option<Self::Record> {
            None
        }

        fn authorize_and_commit<T>(
            &self,
            _record: Self::Record,
            _capabilities: &[&[u8]],
            _commit: impl FnOnce() -> Result<T, PeerDenial>,
        ) -> Result<T, PeerDenial> {
            Err(PeerDenial::Unavailable)
        }
    }

    pub(crate) fn route_mismatch_is_denied_by_public_adapter() -> bool {
        let source_route =
            TrustedRouteKey::from_canonical_route(1, 2, 1, 0, 3).expect("fixture route");
        let requested_route =
            TrustedRouteKey::from_canonical_route(1, 4, 1, 0, 3).expect("different fixture route");
        let endpoint = TrustedServiceEndpoint::from_service_registry(source_route, 5, 6)
            .expect("fixture endpoint");
        let endpoints = FixtureEndpoints {
            recipient: EndpointCheckoutRecipient::from_registered_owner(endpoint),
        };
        let checkout = match endpoints.accept_checkout(endpoint) {
            Ok(checkout) => checkout,
            Err(_) => return false,
        };
        let capability: &[u8] = b"fixture-only";
        let capabilities = [capability];
        let requirements = TrustedServiceRequirements::from_canonical_route(
            requested_route,
            &capabilities,
            RouteConsentRequirement::NotRequired,
        );
        let directory = Pid1LivePeerDirectory::<FixtureRecord>::new();
        let registration = RegistrationId(NonZeroU64::new(1).expect("fixture registration"));
        let _run = directory.begin_run().expect("fixture run");
        let readiness = FixtureReadinessBoundary(Pid1AuthorityDomain::new());
        let result = directory.authorize_peer(
            &Task39PeerCredentialReader,
            checkout,
            registration,
            &requirements,
            &FixtureInventory,
            &FixtureSessions,
            &FixtureConsent,
            &endpoints,
            &FixtureProcessInstances,
            &FixtureSupervisor,
            &readiness,
        );
        matches!(result, Err(PeerDenial::RouteMismatch))
    }

    // The following authorities are deliberately local fixture doubles. The
    // QEMU case uses a real clone3 pidfd child, accepted AF_UNIX stream,
    // SO_PEERCRED/SO_COOKIE, SO_PEERPIDFD, and pidfd liveness matcher, while
    // inventory/session/consent/route/Supervisor/effect owners remain fixtures.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) struct RuntimeRecord {
        pid: u32,
    }

    impl LaunchRecordView for RuntimeRecord {
        fn process_id(self) -> u32 {
            self.pid
        }

        fn app_id(self) -> u64 {
            7
        }

        fn app_session_id(self) -> u64 {
            9
        }
    }

    struct RuntimeInventory {
        current: Mutex<Option<TrustedManifest>>,
    }

    impl RuntimeInventory {
        fn new() -> Self {
            Self {
                current: Mutex::new(Some(
                    TrustedManifest::from_verified_installed_manifest(
                        7,
                        1,
                        [b"files.read".to_vec()],
                    )
                    .expect("fixture manifest"),
                )),
            }
        }
    }

    impl TrustedInventory for RuntimeInventory {
        fn installed_manifest(&self, app_id: u64) -> Result<Option<TrustedManifest>, PeerDenial> {
            let current = self.current.lock().map_err(|_| PeerDenial::Unavailable)?;
            Ok(current
                .as_ref()
                .filter(|manifest| manifest.app_id == app_id)
                .cloned())
        }

        fn generation_is_current(
            &self,
            app_id: u64,
            generation: NonZeroU64,
        ) -> Result<bool, PeerDenial> {
            let current = self.current.lock().map_err(|_| PeerDenial::Unavailable)?;
            Ok(current.as_ref().is_some_and(|manifest| {
                manifest.app_id == app_id && manifest.inventory_generation == generation
            }))
        }

        fn with_current_manifests(
            &self,
            requested: &[(u64, NonZeroU64)],
            inspect: &mut dyn FnMut(&[TrustedManifest]) -> Result<(), PeerDenial>,
        ) -> Result<(), PeerDenial> {
            let current = self.current.lock().map_err(|_| PeerDenial::Unavailable)?;
            let manifest = current.as_ref().ok_or(PeerDenial::NotInstalled)?;
            if requested != [(manifest.app_id, manifest.inventory_generation)] {
                return Err(PeerDenial::StaleManifest);
            }
            inspect(std::slice::from_ref(manifest))
        }
    }

    struct RuntimeSessions {
        lease: SessionLease,
        live: AtomicBool,
    }

    impl RuntimeSessions {
        fn new() -> Self {
            Self {
                lease: SessionLease::from_live_session_authority(7, 9, 1)
                    .expect("fixture session lease"),
                live: AtomicBool::new(true),
            }
        }
    }

    impl LiveSessionAuthority for RuntimeSessions {
        fn capture_live_unlocked(
            &self,
            app_id: u64,
            app_session_id: u64,
        ) -> Result<SessionLease, PeerDenial> {
            if self.lease.app_id != app_id || self.lease.app_session_id != app_session_id {
                return Err(PeerDenial::SessionMismatch);
            }
            if !self.live.load(Ordering::SeqCst) {
                return Err(PeerDenial::SessionRevoked);
            }
            Ok(self.lease)
        }

        fn is_live_unlocked(&self, lease: SessionLease) -> Result<bool, PeerDenial> {
            Ok(self.lease == lease && self.live.load(Ordering::SeqCst))
        }
    }

    struct RuntimeConsent(AtomicBool);

    impl ConsentSource for RuntimeConsent {
        fn connected(&self) -> Result<bool, PeerDenial> {
            Ok(self.0.load(Ordering::SeqCst))
        }
    }

    struct RuntimeEndpoints {
        listener: Option<Task39RegisteredEndpointListener>,
        spawn_owned_lease: Option<Pid1SpawnEndpointLease>,
        endpoint: TrustedServiceEndpoint,
        recipient: EndpointCheckoutRecipient,
        current: Arc<AtomicBool>,
        owner_key_override: Mutex<Option<TrustedInteractivePeerKey>>,
    }

    impl endpoint_authority_seal::Sealed for RuntimeEndpoints {}

    impl RuntimeEndpoints {
        fn is_current_endpoint(
            &self,
            endpoint: TrustedServiceEndpoint,
        ) -> Result<bool, PeerDenial> {
            if self.endpoint != endpoint || !self.current.load(Ordering::SeqCst) {
                return Ok(false);
            }
            match &self.spawn_owned_lease {
                Some(lease) => lease.is_current(endpoint),
                None => Ok(true),
            }
        }

        fn revoke_endpoint(&self) -> Result<(), PeerDenial> {
            self.current.store(false, Ordering::SeqCst);
            if let Some(lease) = &self.spawn_owned_lease {
                lease.revoke()?;
            }
            Ok(())
        }

        fn mark_endpoint_stale_for_c2_postcheck(&self) {
            // Keep the leaf listener owner alive until with_live_interactive_peer
            // returns. This lets the C2 post-callback owner check, rather than
            // the leaf's outer postcheck, be the boundary that rejects the bind.
            self.current.store(false, Ordering::SeqCst);
        }

        fn set_owner_key_override(
            &self,
            owner_key: Option<TrustedInteractivePeerKey>,
        ) -> Result<(), PeerDenial> {
            *self
                .owner_key_override
                .lock()
                .map_err(|_| PeerDenial::Unavailable)? = owner_key;
            Ok(())
        }
    }

    impl TrustedServiceEndpointAuthority for RuntimeEndpoints {
        fn is_current(&self, endpoint: TrustedServiceEndpoint) -> Result<bool, PeerDenial> {
            self.is_current_endpoint(endpoint)
        }

        fn checkout_recipient(
            &self,
            endpoint: TrustedServiceEndpoint,
        ) -> Result<EndpointCheckoutRecipient, PeerDenial> {
            if !self.is_current_endpoint(endpoint)? {
                return Err(PeerDenial::StaleEndpoint);
            }
            if let Some(lease) = &self.spawn_owned_lease {
                lease.checkout_recipient(endpoint)
            } else {
                Ok(self.recipient.clone())
            }
        }

        fn accepted_peer_owner_key(
            &self,
            endpoint: TrustedServiceEndpoint,
        ) -> Result<TrustedInteractivePeerKey, PeerDenial> {
            if !self.is_current_endpoint(endpoint)? {
                return Err(PeerDenial::StaleEndpoint);
            }
            if let Some(owner_key) = *self
                .owner_key_override
                .lock()
                .map_err(|_| PeerDenial::Unavailable)?
            {
                return Ok(owner_key);
            }
            self.spawn_owned_lease
                .as_ref()
                .ok_or(PeerDenial::Unavailable)?
                .accepted_peer_owner_key(endpoint)
        }

        fn fd_delegation_is_prevented(
            &self,
            endpoint: TrustedServiceEndpoint,
        ) -> Result<bool, PeerDenial> {
            // Fixture assertion only. The production endpoint owner and its
            // C3 current-writer/delegation policy are not connected. This
            // fixture-only override lets the regression reach the leaf-proof
            // owner-key and post-callback checks; it is not production evidence.
            Ok(self.is_current_endpoint(endpoint)?)
        }

        fn local_socket_cookie(&self, stream: &UnixStream) -> Result<NonZeroU64, PeerDenial> {
            if let Some(lease) = &self.spawn_owned_lease {
                return lease.local_socket_cookie(stream);
            }
            let mut cookie = 0u64;
            let mut length = std::mem::size_of::<u64>() as libc::socklen_t;
            let result = unsafe {
                libc::getsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_COOKIE,
                    (&mut cookie as *mut u64).cast(),
                    &mut length,
                )
            };
            if result != 0 || length as usize != std::mem::size_of::<u64>() {
                return Err(PeerDenial::Unavailable);
            }
            NonZeroU64::new(cookie).ok_or(PeerDenial::Unavailable)
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
            if !self.is_current(endpoint)? {
                return Err(PeerDenial::StaleEndpoint);
            }
            let recipient = self.checkout_recipient(endpoint)?;
            let checkout = if let Some(lease) = &self.spawn_owned_lease {
                lease.accept_checkout_until(endpoint, deadline)?
            } else {
                self.listener
                    .as_ref()
                    .ok_or(PeerDenial::Unavailable)?
                    .accept_checkout_until(endpoint, deadline, |stream| {
                        self.local_socket_cookie(stream)
                    })?
            };
            if !checkout.recipient.is_same_recipient(&recipient) || !self.is_current(endpoint)? {
                return Err(PeerDenial::StaleEndpoint);
            }
            Ok(checkout)
        }
    }

    struct RuntimeSupervisor {
        record: Mutex<Option<RuntimeRecord>>,
    }

    impl RuntimeSupervisor {
        fn new() -> Self {
            Self {
                record: Mutex::new(None),
            }
        }

        fn install(&self, record: RuntimeRecord) -> Result<(), PeerDenial> {
            let mut current = self.record.lock().map_err(|_| PeerDenial::Unavailable)?;
            if current.is_some() {
                return Err(PeerDenial::StaleRegistration);
            }
            *current = Some(record);
            Ok(())
        }

        fn revoke(&self, record: RuntimeRecord) -> Result<(), PeerDenial> {
            let mut current = self.record.lock().map_err(|_| PeerDenial::Unavailable)?;
            if *current != Some(record) {
                return Err(PeerDenial::StaleRegistration);
            }
            *current = None;
            Ok(())
        }
    }

    impl SupervisorLaunchAuthority for RuntimeSupervisor {
        type Record = RuntimeRecord;

        fn resolve(&self, process_id: u32) -> Option<Self::Record> {
            self.record
                .lock()
                .ok()
                .and_then(|record| record.filter(|record| record.pid == process_id))
        }

        fn authorize_and_commit<T>(
            &self,
            record: Self::Record,
            capabilities: &[&[u8]],
            commit: impl FnOnce() -> Result<T, PeerDenial>,
        ) -> Result<T, PeerDenial> {
            let current = self.record.lock().map_err(|_| PeerDenial::Unavailable)?;
            if *current != Some(record) {
                return Err(PeerDenial::NotLive);
            }
            if capabilities
                .iter()
                .any(|capability| *capability != b"files.read")
            {
                return Err(PeerDenial::NotDeclared);
            }
            commit()
        }
    }

    struct RuntimeReadinessBoundary {
        run: Pid1RunInstance,
        domain: Pid1AuthorityDomain,
        gate: Mutex<()>,
        active: Arc<AtomicBool>,
    }

    impl readiness_boundary_seal::Sealed for RuntimeReadinessBoundary {}

    impl Pid1ReadinessAtomicBoundary for RuntimeReadinessBoundary {
        fn authority_domain(&self) -> &Pid1AuthorityDomain {
            &self.domain
        }

        fn with_owner_gate<T>(
            &self,
            run: &Pid1RunInstance,
            _required: Option<&RequiredServiceSet>,
            operation: impl FnOnce() -> Result<T, PeerDenial>,
        ) -> Result<T, PeerDenial> {
            if !self.run.is_same_run(run) {
                return Err(PeerDenial::StaleRun);
            }
            let _guard = self.gate.lock().map_err(|_| PeerDenial::Unavailable)?;
            self.active.store(true, Ordering::SeqCst);
            struct Reset<'a>(&'a AtomicBool);
            impl Drop for Reset<'_> {
                fn drop(&mut self) {
                    self.0.store(false, Ordering::SeqCst);
                }
            }
            let _reset = Reset(&self.active);
            operation()
        }
    }

    struct RuntimeEffectOwnerState {
        current: Option<ServiceRequestAdmission>,
        consumed: HashSet<StableServiceRequestId>,
        request_keys: HashMap<DurableServiceRequestKey, ServiceOwnerRequestToken>,
    }

    pub(crate) struct RuntimeEffectOwner {
        state: Mutex<RuntimeEffectOwnerState>,
        owner_identity: ServiceEffectOwnerId,
        gate_active: Arc<AtomicBool>,
        commits: AtomicUsize,
    }

    impl RuntimeEffectOwner {
        fn new(admission: ServiceRequestAdmission, readiness: &RuntimeReadinessBoundary) -> Self {
            let owner_identity = ServiceEffectOwnerId::new([0x41; 16]);
            Self {
                state: Mutex::new(RuntimeEffectOwnerState {
                    current: Some(admission),
                    consumed: HashSet::new(),
                    request_keys: HashMap::new(),
                }),
                owner_identity,
                gate_active: Arc::clone(&readiness.active),
                commits: AtomicUsize::new(0),
            }
        }

        fn commit_count(&self) -> usize {
            self.commits.load(Ordering::SeqCst)
        }
    }

    impl service_effect_commit_seal::Sealed for RuntimeEffectOwner {}

    impl ServiceOwnerEffectTransaction for RuntimeEffectOwner {
        fn durable_owner_id_for(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<ServiceEffectOwnerId, PeerDenial> {
            let state = self.state.try_lock().map_err(|_| PeerDenial::Unavailable)?;
            if state.current.as_ref() != Some(admission) {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            Ok(self.owner_identity)
        }

        fn validate_request_with_identity(
            &self,
            admission: &ServiceRequestAdmission,
            owner: ServiceEffectOwnerId,
            identity: &DurableServiceRequestIdentity,
        ) -> Result<ServiceOwnerRequestToken, PeerDenial> {
            let state = self.state.try_lock().map_err(|_| PeerDenial::Unavailable)?;
            if state.current.as_ref() != Some(admission)
                || owner != self.owner_identity
                || *identity != durable_request_identity_claim(owner, admission)
            {
                return Err(PeerDenial::RequestIdentityConflict);
            }
            Ok(ServiceOwnerRequestToken::fixture(owner, admission))
        }

        fn admission_is_current(
            &self,
            admission: &ServiceRequestAdmission,
        ) -> Result<bool, PeerDenial> {
            if !self.gate_active.load(Ordering::SeqCst) {
                return Err(PeerDenial::Unavailable);
            }
            let state = self.state.try_lock().map_err(|_| PeerDenial::Unavailable)?;
            Ok(!state.consumed.contains(&admission.stable_request_id())
                && state.current.as_ref() == Some(admission))
        }

        fn commit_if_current_with_identity<T>(
            &self,
            admission: &ServiceRequestAdmission,
            identity: &ServiceOwnerRequestToken,
            commit: impl FnOnce() -> Result<T, PeerDenial>,
        ) -> Result<T, PeerDenial> {
            if !identity.matches_admission(self.owner_identity, admission) {
                return Err(PeerDenial::ServiceEffectNotInvoked(Arc::new(
                    PeerDenial::RequestIdentityConflict,
                )));
            }
            if !self.gate_active.load(Ordering::SeqCst) {
                return Err(PeerDenial::ServiceEffectNotInvoked(Arc::new(
                    PeerDenial::Unavailable,
                )));
            }
            let mut state = self.state.try_lock().map_err(|_| {
                PeerDenial::ServiceEffectNotInvoked(Arc::new(PeerDenial::Unavailable))
            })?;
            let key = identity.identity_claim.key();
            if state
                .request_keys
                .get(&key)
                .is_some_and(|stored| stored != identity)
            {
                return Err(PeerDenial::ServiceEffectNotInvoked(Arc::new(
                    PeerDenial::RequestIdentityConflict,
                )));
            }
            if state.consumed.contains(&admission.stable_request_id()) {
                return Err(PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(
                    PeerDenial::RequestOutcomeUnknown,
                )));
            }
            if state.current.as_ref() != Some(admission) {
                return Err(PeerDenial::ServiceEffectNotInvoked(Arc::new(
                    PeerDenial::StaleRegistration,
                )));
            }
            state.request_keys.insert(key, *identity);
            state.consumed.insert(admission.stable_request_id());
            let result = commit()
                .map_err(|error| PeerDenial::ServiceEffectOutcomeUnknown(Arc::new(error)))?;
            self.commits.fetch_add(1, Ordering::SeqCst);
            Ok(result)
        }
    }

    fn runtime_nipc_route(route: TrustedRouteKey) -> Result<TrustedNipcOperationRoute, PeerDenial> {
        let service_id =
            ServiceId::new("fixture.files").map_err(|_| PeerDenial::UnknownNipcOperation)?;
        let version = ContractVersion::new(1, 0).map_err(|_| PeerDenial::UnknownNipcOperation)?;
        let operation_id =
            OperationId::new("read").map_err(|_| PeerDenial::UnknownNipcOperation)?;
        let capability =
            CapabilityId::new("files.read").map_err(|_| PeerDenial::UnknownNipcOperation)?;
        let descriptor = ServiceDescriptor::new(
            service_id,
            [version],
            [ServiceOperation::new(
                operation_id.clone(),
                Some(capability),
            )],
        )
        .map_err(|_| PeerDenial::UnknownNipcOperation)?;
        TrustedNipcOperationRoute::from_service_contract(
            route,
            7,
            "svc.fixture@1",
            &descriptor,
            version,
            &operation_id,
            ServiceEffectOwnerId::new([0x41; 16]),
        )
    }

    pub(crate) struct RuntimeAuthorityFixture {
        directory: Pid1LivePeerDirectory<RuntimeRecord>,
        run: Pid1RunInstance,
        route: TrustedRouteKey,
        endpoint: TrustedServiceEndpoint,
        inventory: RuntimeInventory,
        sessions: RuntimeSessions,
        consent: RuntimeConsent,
        endpoints: RuntimeEndpoints,
        supervisor: RuntimeSupervisor,
        readiness: RuntimeReadinessBoundary,
    }

    impl RuntimeAuthorityFixture {
        pub(crate) fn new(listener: UnixListener) -> Result<Self, PeerDenial> {
            let route = TrustedRouteKey::from_canonical_route(41, 1, 1, 0, 1)
                .expect("fixture canonical route");
            let endpoint = TrustedServiceEndpoint::from_service_registry(route, 1, 1)
                .expect("fixture endpoint");
            let recipient = EndpointCheckoutRecipient::from_registered_owner(endpoint);
            let listener = AcceptedPeerListener::new(listener).map_err(map_task39_accept_error)?;
            let directory = Pid1LivePeerDirectory::new();
            let run = directory.begin_run().expect("fixture run");
            Ok(Self {
                directory,
                run: run.clone(),
                route,
                endpoint,
                inventory: RuntimeInventory::new(),
                sessions: RuntimeSessions::new(),
                consent: RuntimeConsent(AtomicBool::new(true)),
                endpoints: RuntimeEndpoints {
                    listener: Some(Task39RegisteredEndpointListener::from_service_registry(
                        recipient.clone(),
                        listener,
                    )),
                    spawn_owned_lease: None,
                    endpoint,
                    recipient,
                    current: Arc::new(AtomicBool::new(true)),
                    owner_key_override: Mutex::new(None),
                },
                supervisor: RuntimeSupervisor::new(),
                readiness: RuntimeReadinessBoundary {
                    run,
                    domain: Pid1AuthorityDomain::new(),
                    gate: Mutex::new(()),
                    active: Arc::new(AtomicBool::new(false)),
                },
            })
        }

        /// Build the C2 endpoint through the same opaque spawn binding and
        /// PID-1 endpoint-owner registry used by production. The fixture
        /// overrides only the production fail-closed FD-delegation predicate
        /// so the owner-key and callback revalidation gates can be exercised.
        pub(crate) fn new_spawn_owned(
            listener: UnixListener,
            spawn: SpawnedProcessPidfd,
            authority: &PidfdProcessInstanceAuthority<LinuxPidfdKernel>,
        ) -> Result<(Self, RuntimeRecord, RegistrationId, ProcessInstanceKey), PeerDenial> {
            let route_key = TrustedRouteKey::from_canonical_route(41, 1, 1, 0, 1)
                .ok_or(PeerDenial::UnknownNipcOperation)?;
            let nipc_route = runtime_nipc_route(route_key)?;
            let directory = Pid1LivePeerDirectory::new();
            let run = directory.begin_run().expect("fixture run");
            let supervisor = RuntimeSupervisor::new();
            let record = RuntimeRecord {
                pid: spawn.process_id(),
            };
            supervisor.install(record)?;

            let proof = authority.capture_task24_spawn_proof(spawn, record)?;
            let process = proof.process_key();
            let binding =
                authority.bind_spawn_owned_endpoint(proof, nipc_route, listener, &supervisor)?;
            let endpoint_owner = Pid1EndpointOwnerRegistry::new();
            let (lease, registration_proof) =
                endpoint_owner.install_from_spawn(&run, binding, authority, &supervisor)?;
            let endpoint = lease.endpoint();
            let recipient = lease.recipient();
            let fixture = Self {
                directory,
                run: run.clone(),
                route: route_key,
                endpoint,
                inventory: RuntimeInventory::new(),
                sessions: RuntimeSessions::new(),
                consent: RuntimeConsent(AtomicBool::new(true)),
                endpoints: RuntimeEndpoints {
                    listener: None,
                    spawn_owned_lease: Some(lease),
                    endpoint,
                    recipient,
                    current: Arc::new(AtomicBool::new(true)),
                    owner_key_override: Mutex::new(None),
                },
                supervisor,
                readiness: RuntimeReadinessBoundary {
                    run,
                    domain: Pid1AuthorityDomain::new(),
                    gate: Mutex::new(()),
                    active: Arc::new(AtomicBool::new(false)),
                },
            };
            let requirements = fixture.requirements();
            let registration = fixture.directory.register_pid1_spawn(
                &fixture.run,
                registration_proof,
                &requirements,
                endpoint,
                &fixture.inventory,
                &fixture.sessions,
                &fixture.consent,
                &fixture.endpoints,
                &fixture.supervisor,
                &fixture.readiness,
            )?;
            Ok((fixture, record, registration, process))
        }

        fn requirements(&self) -> TrustedServiceRequirements<'_> {
            const CAPABILITIES: [&[u8]; 1] = [b"files.read"];
            TrustedServiceRequirements::from_canonical_route(
                self.route,
                &CAPABILITIES,
                RouteConsentRequirement::NotRequired,
            )
        }

        pub(crate) fn register_spawn<K: PidfdKernel>(
            &self,
            authority: &PidfdProcessInstanceAuthority<K>,
            process: ProcessInstanceKey,
        ) -> Result<(RuntimeRecord, RegistrationId), PeerDenial> {
            let record = RuntimeRecord { pid: process.pid() };
            self.supervisor.install(record)?;
            let result = (|| {
                let proof = authority
                    .issue_spawn_proof(process, record)?
                    .attach_task24_test_identity(nagi_init_spawn_contract::test_identity(
                        record.pid,
                    ))?;
                let requirements = self.requirements();
                let registration = self.directory.register_pid1_spawn(
                    &self.run,
                    proof,
                    &requirements,
                    self.endpoint,
                    &self.inventory,
                    &self.sessions,
                    &self.consent,
                    &self.endpoints,
                    &self.supervisor,
                    &self.readiness,
                )?;
                Ok((record, registration))
            })();
            if result.is_err() {
                let _ = self.supervisor.revoke(record);
            }
            result
        }

        pub(crate) fn register_c3_test_pidfd(
            &self,
            authority: &PidfdProcessInstanceAuthority<LinuxPidfdKernel>,
            spawn: SpawnedProcessPidfd,
        ) -> Result<(RuntimeRecord, RegistrationId, ProcessInstanceKey), PeerDenial> {
            let record = RuntimeRecord {
                pid: spawn.process_id(),
            };
            self.supervisor.install(record)?;
            let result = (|| {
                let proof = authority.capture_task24_spawn_proof(spawn, record)?;
                let process = proof.process_key();
                let requirements = self.requirements();
                let registration = self.directory.register_pid1_spawn(
                    &self.run,
                    proof,
                    &requirements,
                    self.endpoint,
                    &self.inventory,
                    &self.sessions,
                    &self.consent,
                    &self.endpoints,
                    &self.supervisor,
                    &self.readiness,
                )?;
                Ok((record, registration, process))
            })();
            if result.is_err() {
                let _ = self.supervisor.revoke(record);
            }
            result
        }

        pub(crate) fn authorize_child<'a, K: PidfdKernel>(
            &'a self,
            authority: &'a PidfdProcessInstanceAuthority<K>,
            registration: RegistrationId,
        ) -> Result<AuthorizedPeer<'a, RuntimeRecord>, PeerDenial> {
            self.authorize_child_with_sessions(authority, registration, &self.sessions)
        }

        pub(crate) fn authorize_child_with_sessions<'a, K, S>(
            &'a self,
            authority: &'a PidfdProcessInstanceAuthority<K>,
            registration: RegistrationId,
            sessions: &'a S,
        ) -> Result<AuthorizedPeer<'a, RuntimeRecord>, PeerDenial>
        where
            K: PidfdKernel,
            S: LiveSessionAuthority + 'a,
        {
            let requirements = self.requirements();
            let result = self.directory.accept_and_authorize_peer_until(
                &Task39PeerCredentialReader,
                self.endpoint,
                registration,
                &requirements,
                Instant::now() + Duration::from_secs(5),
                &self.inventory,
                sessions,
                &self.consent,
                &self.endpoints,
                authority,
                &self.supervisor,
                &self.readiness,
            );
            if let Err(error) = &result {
                eprintln!("PUBLIC_ADAPTER_AUTHORIZATION=DENIED error={error:?}");
            }
            result
        }

        pub(crate) fn proof_revocation_observer(
            &self,
            record: RuntimeRecord,
            registration: RegistrationId,
            process: ProcessInstanceKey,
            revocation: ProofOwnerRevocation,
        ) -> ProofOwnerRevocationObserver<'_> {
            ProofOwnerRevocationObserver {
                runtime: self,
                record,
                registration,
                process,
                revocation,
                observed_peer_pid: AtomicU32::new(0),
                returned_peer_pid: AtomicU32::new(0),
                bind_peer_callbacks: AtomicUsize::new(0),
                rejected_after_revocation: AtomicU8::new(0),
                post_bind_authority_callbacks: AtomicUsize::new(0),
                activation_callbacks: Arc::new(AtomicUsize::new(0)),
                abort_callbacks: Arc::new(AtomicUsize::new(0)),
                gate_effect_callbacks: Arc::new(AtomicUsize::new(0)),
                simulated_binding_active: Arc::new(AtomicBool::new(false)),
                simulated_binding_rollback_count: Arc::new(AtomicUsize::new(0)),
            }
        }

        pub(crate) fn route_mismatch_is_denied_for_spawn<K: PidfdKernel>(
            &self,
            authority: &PidfdProcessInstanceAuthority<K>,
            registration: RegistrationId,
        ) -> bool {
            let mismatched_route = match TrustedRouteKey::from_canonical_route(41, 2, 1, 0, 1) {
                Some(route) => route,
                None => return false,
            };
            const CAPABILITIES: [&[u8]; 1] = [b"files.read"];
            let requirements = TrustedServiceRequirements::from_canonical_route(
                mismatched_route,
                &CAPABILITIES,
                RouteConsentRequirement::NotRequired,
            );
            matches!(
                self.directory.accept_and_authorize_peer_until(
                    &Task39PeerCredentialReader,
                    self.endpoint,
                    registration,
                    &requirements,
                    Instant::now() + Duration::from_secs(5),
                    &self.inventory,
                    &self.sessions,
                    &self.consent,
                    &self.endpoints,
                    authority,
                    &self.supervisor,
                    &self.readiness,
                ),
                Err(PeerDenial::RouteMismatch)
            )
        }

        pub(crate) fn new_effect_owner(
            &self,
            admission: ServiceRequestAdmission,
        ) -> RuntimeEffectOwner {
            RuntimeEffectOwner::new(admission, &self.readiness)
        }

        pub(crate) fn revoke_service_owner(
            &self,
            owner: &RuntimeEffectOwner,
        ) -> Result<(), PeerDenial> {
            self.readiness.with_owner_gate(&self.run, None, || {
                owner
                    .state
                    .lock()
                    .map_err(|_| PeerDenial::Unavailable)?
                    .current = None;
                Ok(())
            })
        }

        pub(crate) fn one_shot_concurrent_commit(
            &self,
            peer: &AuthorizedPeer<'_, RuntimeRecord>,
            admission: &ServiceRequestAdmission,
            owner: &RuntimeEffectOwner,
        ) -> bool {
            let effects = AtomicUsize::new(0);
            let (first, second) = std::thread::scope(|scope| {
                let first_admission = admission.clone();
                let second_admission = admission.clone();
                let peer = peer;
                let readiness = &self.readiness;
                let first_readiness = readiness;
                let second_readiness = readiness;
                let owner = owner;
                let effects = &effects;
                let first = scope.spawn(move || {
                    peer.commit_effect(&first_admission, first_readiness, owner, || {
                        effects.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                });
                let peer = peer;
                let owner = owner;
                let effects = effects;
                let second = scope.spawn(move || {
                    peer.commit_effect(&second_admission, second_readiness, owner, || {
                        effects.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                });
                (first.join(), second.join())
            });
            let outcomes = [first, second];
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Ok(Ok(()))))
                .count()
                == 1
                && outcomes
                    .iter()
                    .filter(|outcome| matches!(outcome, Ok(Err(_))))
                    .count()
                    == 1
                && effects.load(Ordering::SeqCst) == 1
                && owner.commit_count() == 1
        }

        pub(crate) fn replay_is_denied(
            &self,
            peer: &AuthorizedPeer<'_, RuntimeRecord>,
            admission: &ServiceRequestAdmission,
            owner: &RuntimeEffectOwner,
        ) -> bool {
            matches!(
                peer.commit_effect(admission, &self.readiness, owner, || Ok(())),
                Err(PeerDenial::DuplicateRequestCommit)
            )
        }

        pub(crate) fn send_partial_request(
            &self,
            peer: &mut AuthorizedPeer<'_, RuntimeRecord>,
            admission: &ServiceRequestAdmission,
            owner: &RuntimeEffectOwner,
        ) -> Result<usize, PeerDenial> {
            let deadline = Instant::now() + Duration::from_secs(5);
            let accepted_bytes = AtomicUsize::new(0);
            let post_revoke_io_denied = AtomicBool::new(false);
            let result = peer.with_request_io_until(
                admission,
                &self.readiness,
                owner,
                deadline,
                |io| -> std::io::Result<()> {
                    let payload = vec![b'P'; 4 * 1024 * 1024];
                    let mut sent = 0;
                    loop {
                        match io.write(&payload[sent..]) {
                            Ok(0) => {
                                return Err(std::io::ErrorKind::WriteZero.into());
                            }
                            Ok(count) => {
                                sent += count;
                                if sent == payload.len() {
                                    return Err(std::io::Error::other(
                                        "socket accepted entire payload without WouldBlock",
                                    ));
                                }
                            }
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                accepted_bytes.store(sent, Ordering::SeqCst);
                                break;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    if sent == 0 {
                        return Err(std::io::ErrorKind::WriteZero.into());
                    }

                    let mut ack = [0u8; 8];
                    let mut ack_offset = 0;
                    while ack_offset < ack.len() {
                        match io.read(&mut ack[ack_offset..]) {
                            Ok(0) => return Err(std::io::ErrorKind::UnexpectedEof.into()),
                            Ok(count) => ack_offset += count,
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                if Instant::now() >= deadline {
                                    return Err(std::io::ErrorKind::TimedOut.into());
                                }
                                std::thread::sleep(Duration::from_millis(2));
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    if &ack != b"CONSUMED" {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "fixture child acknowledgement malformed",
                        ));
                    }

                    self.revoke_service_owner(owner)
                        .map_err(|_| std::io::ErrorKind::PermissionDenied)?;
                    match io.write(b"X") {
                        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                            post_revoke_io_denied.store(true, Ordering::SeqCst);
                            Ok(())
                        }
                        Ok(_) => Err(std::io::Error::other(
                            "post-revocation write unexpectedly reached the socket",
                        )),
                        Err(error) => Err(error),
                    }
                },
            );

            if !post_revoke_io_denied.load(Ordering::SeqCst) {
                return Err(PeerDenial::Unavailable);
            }
            match result {
                Err(PeerDenial::StaleRegistration) => Ok(accepted_bytes.load(Ordering::SeqCst)),
                Err(error) => Err(error),
                Ok(Ok(())) | Ok(Err(_)) => Err(PeerDenial::Unavailable),
            }
        }

        pub(crate) fn partial_write_revoke_denies_commit(
            &self,
            peer: &AuthorizedPeer<'_, RuntimeRecord>,
            admission: &ServiceRequestAdmission,
            owner: &RuntimeEffectOwner,
        ) -> bool {
            match peer.commit_effect(admission, &self.readiness, owner, || Ok(())) {
                Err(PeerDenial::ServiceEffectNotInvoked(cause)) => matches!(
                    cause.as_ref(),
                    PeerDenial::StaleRegistration | PeerDenial::StaleConnection
                ),
                _ => false,
            }
        }

        pub(crate) fn stale_during_request_io_denies_success(
            &self,
            peer: &mut AuthorizedPeer<'_, RuntimeRecord>,
            admission: &ServiceRequestAdmission,
            owner: &RuntimeEffectOwner,
        ) -> bool {
            let stale_latch = Arc::clone(&peer.connection_stale);
            matches!(
                peer.with_request_io_until(
                    admission,
                    &self.readiness,
                    owner,
                    Instant::now() + Duration::from_secs(5),
                    move |io| -> std::io::Result<()> {
                        io.write_all(b"S")?;
                        stale_latch.store(true, Ordering::SeqCst);
                        Ok(())
                    },
                ),
                Err(PeerDenial::StaleConnection)
            )
        }

        pub(crate) fn wait_for_exit<K: PidfdKernel>(
            &self,
            authority: &PidfdProcessInstanceAuthority<K>,
            process: ProcessInstanceKey,
            timeout: Duration,
        ) -> Result<(), PeerDenial> {
            let deadline = Instant::now() + timeout;
            loop {
                if !authority.confirms_owned_process_live(process)? {
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    return Err(PeerDenial::Unavailable);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        pub(crate) fn retire_after_exit<K: PidfdKernel>(
            &self,
            authority: &PidfdProcessInstanceAuthority<K>,
            record: RuntimeRecord,
            process: ProcessInstanceKey,
            registration: RegistrationId,
        ) -> Result<(), PeerDenial> {
            self.readiness
                .with_owner_gate(&self.run, None, || self.supervisor.revoke(record))?;
            if !self.directory.retire_after_supervisor_revoke(
                &self.run,
                process,
                registration,
                &self.readiness,
            )? {
                return Err(PeerDenial::StaleRegistration);
            }
            if !authority.retire_after_revoke(process)? {
                return Err(PeerDenial::ProcessInstanceUnavailable);
            }
            Ok(())
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub(crate) enum ProofOwnerRevocation {
        Endpoint,
        Registration,
        AfterC2OwnerGate,
    }

    pub(crate) struct ProofOwnerRevocationObserver<'a> {
        runtime: &'a RuntimeAuthorityFixture,
        record: RuntimeRecord,
        registration: RegistrationId,
        process: ProcessInstanceKey,
        revocation: ProofOwnerRevocation,
        observed_peer_pid: AtomicU32,
        returned_peer_pid: AtomicU32,
        bind_peer_callbacks: AtomicUsize,
        rejected_after_revocation: AtomicU8,
        post_bind_authority_callbacks: AtomicUsize,
        activation_callbacks: Arc<AtomicUsize>,
        abort_callbacks: Arc<AtomicUsize>,
        gate_effect_callbacks: Arc<AtomicUsize>,
        simulated_binding_active: Arc<AtomicBool>,
        simulated_binding_rollback_count: Arc<AtomicUsize>,
    }

    impl ProofOwnerRevocationObserver<'_> {
        pub(crate) fn observed_peer_pid(&self) -> u32 {
            self.observed_peer_pid.load(Ordering::SeqCst)
        }

        pub(crate) fn owner_revocation_rejected_proof(&self) -> bool {
            self.rejected_after_revocation.load(Ordering::SeqCst) == 1
        }

        pub(crate) fn returned_peer_pid(&self) -> u32 {
            self.returned_peer_pid.load(Ordering::SeqCst)
        }

        pub(crate) fn bind_peer_callbacks(&self) -> usize {
            self.bind_peer_callbacks.load(Ordering::SeqCst)
        }

        pub(crate) fn post_bind_authority_callbacks(&self) -> usize {
            self.post_bind_authority_callbacks.load(Ordering::SeqCst)
        }

        pub(crate) fn activation_callbacks(&self) -> usize {
            self.activation_callbacks.load(Ordering::SeqCst)
        }

        pub(crate) fn abort_callbacks(&self) -> usize {
            self.abort_callbacks.load(Ordering::SeqCst)
        }

        pub(crate) fn gate_effect_callbacks(&self) -> usize {
            self.gate_effect_callbacks.load(Ordering::SeqCst)
        }

        pub(crate) fn simulated_binding_lookup_succeeds(&self) -> bool {
            self.simulated_binding_active.load(Ordering::SeqCst)
        }

        pub(crate) fn simulated_binding_rollback_count(&self) -> usize {
            self.simulated_binding_rollback_count.load(Ordering::SeqCst)
        }
    }

    struct ProofRevocationEffectGate {
        callbacks: Arc<AtomicUsize>,
        stale_endpoint_after_effect: Option<Arc<AtomicBool>>,
    }

    impl BoundPeerSessionEffectGate for ProofRevocationEffectGate {
        fn with_live_session_effect(
            &self,
            _lease: SessionLease,
            effect: &mut dyn FnMut() -> Result<(), PeerDenial>,
        ) -> Result<(), PeerDenial> {
            self.callbacks.fetch_add(1, Ordering::SeqCst);
            effect()?;
            if let Some(current) = &self.stale_endpoint_after_effect {
                current.store(false, Ordering::SeqCst);
            }
            Ok(())
        }
    }

    impl LiveSessionAuthority for ProofOwnerRevocationObserver<'_> {
        fn capture_live_unlocked(
            &self,
            _app_id: u64,
            _app_session_id: u64,
        ) -> Result<SessionLease, PeerDenial> {
            Err(PeerDenial::SessionUnavailable)
        }

        fn is_live_unlocked(&self, _lease: SessionLease) -> Result<bool, PeerDenial> {
            self.post_bind_authority_callbacks
                .fetch_add(1, Ordering::SeqCst);
            Ok(false)
        }

        fn begin_authenticated_peer_binding(
            &self,
            peer: &AuthenticatedInteractivePeer<'_>,
            app_admission: &C2AppSessionAdmission<'_, '_, '_>,
            connection_proof: &C2AcceptedConnectionProof<'_>,
        ) -> Result<PendingBoundPeerSession, PeerDenial> {
            if !app_admission.matches_peer_and_lease(peer, app_admission.session_lease()) {
                return Err(PeerDenial::StalePeer);
            }
            if !connection_proof.matches_kernel_peer(peer.kernel_peer()) {
                return Err(PeerDenial::StalePeer);
            }
            self.bind_peer_callbacks.fetch_add(1, Ordering::SeqCst);
            self.observed_peer_pid
                .store(peer.kernel_peer().pid(), Ordering::SeqCst);

            // Revoke the real C2 fixture owner while C2's proof callback is
            // active. This C1-shaped callback stages a provisional gate; C2's
            // post-validation must discard it and abort before activation.
            match self.revocation {
                ProofOwnerRevocation::Endpoint => {
                    self.runtime
                        .endpoints
                        .mark_endpoint_stale_for_c2_postcheck();
                }
                ProofOwnerRevocation::Registration => {
                    self.runtime.supervisor.revoke(self.record)?;
                    if !self.runtime.directory.retire_after_supervisor_revoke(
                        &self.runtime.run,
                        self.process,
                        self.registration,
                        &self.runtime.readiness,
                    )? {
                        return Err(PeerDenial::StaleRegistration);
                    }
                }
                ProofOwnerRevocation::AfterC2OwnerGate => {}
            }
            self.returned_peer_pid
                .store(peer.kernel_peer().pid(), Ordering::SeqCst);
            let activation_callbacks = Arc::clone(&self.activation_callbacks);
            let abort_callbacks = Arc::clone(&self.abort_callbacks);
            let simulated_binding_active_for_activation =
                Arc::clone(&self.simulated_binding_active);
            let simulated_binding_active_for_abort = Arc::clone(&self.simulated_binding_active);
            let simulated_binding_rollback_count =
                Arc::clone(&self.simulated_binding_rollback_count);
            Ok(PendingBoundPeerSession::new(
                Arc::new(ProofRevocationEffectGate {
                    callbacks: Arc::clone(&self.gate_effect_callbacks),
                    stale_endpoint_after_effect: match self.revocation {
                        ProofOwnerRevocation::AfterC2OwnerGate => {
                            Some(Arc::clone(&self.runtime.endpoints.current))
                        }
                        ProofOwnerRevocation::Endpoint | ProofOwnerRevocation::Registration => None,
                    },
                }),
                move || {
                    activation_callbacks.fetch_add(1, Ordering::SeqCst);
                    simulated_binding_active_for_activation.store(true, Ordering::SeqCst);
                    Ok(())
                },
                move || {
                    abort_callbacks.fetch_add(1, Ordering::SeqCst);
                    if simulated_binding_active_for_abort.swap(false, Ordering::SeqCst) {
                        simulated_binding_rollback_count.fetch_add(1, Ordering::SeqCst);
                    }
                },
            ))
        }
    }

    /// Exercise C2 listener-slot availability and both owner-key boundaries
    /// with a real listener installed through the spawn-owned endpoint path
    /// and real Unix connections from its live child. The public foreign
    /// listener owner supplies only a substituted key; it is never enrolled
    /// as the endpoint owner.
    pub(crate) fn run_c2_peer_proof_owner_key_boundary_regression() -> Result<(), PeerDenial> {
        if unsafe { libc::getpid() } != 1 {
            return Err(PeerDenial::InvalidProcess);
        }

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PeerDenial::Unavailable)?
            .as_nanos();
        let endpoint_path = std::path::PathBuf::from(format!(
            "/tmp/nagi-c2-owner-key-{}-{nonce}.sock",
            std::process::id()
        ));
        let foreign_path = std::path::PathBuf::from(format!(
            "/tmp/nagi-c2-owner-key-foreign-{}-{nonce}.sock",
            std::process::id()
        ));

        let mut child = std::process::Command::new("/init")
            .arg("--c2-owner-key-peer")
            .arg(&endpoint_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .map_err(|_| PeerDenial::Unavailable)?;
        let mut child_signal = child.stdin.take().ok_or(PeerDenial::Unavailable)?;

        let run_result = (|| {
            let listener =
                UnixListener::bind(&endpoint_path).map_err(|_| PeerDenial::Unavailable)?;
            let authority = PidfdProcessInstanceAuthority::new(LinuxPidfdKernel);
            let spawn = nagi_init_spawn_contract::test_pidfd_for_existing_process(child.id())
                .map_err(|_| PeerDenial::ProcessInstanceUnavailable)?;
            let (runtime, record, registration, process) =
                RuntimeAuthorityFixture::new_spawn_owned(listener, spawn, &authority)?;

            let expected_owner_key = runtime
                .endpoints
                .accepted_peer_owner_key(runtime.endpoint)?;
            let accept_endpoints = &runtime.endpoints;
            let accept_endpoint = runtime.endpoint;
            let (accept_started_tx, accept_started_rx) = std::sync::mpsc::channel();
            let concurrent_checkout = std::thread::scope(|scope| {
                let accepting = scope.spawn(move || {
                    accept_started_tx
                        .send(())
                        .map_err(|_| PeerDenial::Unavailable)?;
                    accept_endpoints.accept_checkout_until(
                        accept_endpoint,
                        Instant::now() + Duration::from_secs(10),
                    )
                });
                accept_started_rx
                    .recv_timeout(Duration::from_secs(2))
                    .map_err(|_| PeerDenial::Unavailable)?;
                // Let the nonblocking accept loop enter its wait before the
                // concurrent key lookup; the endpoint record slot must not be
                // held for that listener wait.
                std::thread::sleep(Duration::from_millis(50));
                let owner_key_during_accept =
                    runtime.endpoints.accepted_peer_owner_key(runtime.endpoint);
                child_signal
                    .write_all(&[1])
                    .map_err(|_| PeerDenial::Unavailable)?;
                child_signal.flush().map_err(|_| PeerDenial::Unavailable)?;
                let checkout = accepting.join().map_err(|_| PeerDenial::Unavailable)??;
                if owner_key_during_accept? != expected_owner_key {
                    return Err(PeerDenial::StaleEndpoint);
                }
                Ok::<_, PeerDenial>(checkout)
            })?;
            drop(concurrent_checkout);
            println!("C2_ENDPOINT_ACCEPT_CONCURRENCY=PASS key_lookup=available_during_accept_wait");

            let foreign_listener =
                UnixListener::bind(&foreign_path).map_err(|_| PeerDenial::Unavailable)?;
            let foreign_owner = C2AcceptedPeerListenerOwner::register_from_c2_owner(
                foreign_listener,
                runtime.run.generation(),
                runtime.endpoint.endpoint_generation,
                NonZeroU64::new(1).ok_or(PeerDenial::GenerationExhausted)?,
                Instant::now() + Duration::from_secs(30),
            )
            .map_err(|error| PeerDenial::KernelPeerAuthentication(Arc::new(error)))?;

            let observer = runtime.proof_revocation_observer(
                record,
                registration,
                process,
                ProofOwnerRevocation::Endpoint,
            );
            runtime
                .endpoints
                .set_owner_key_override(Some(foreign_owner.owner_key()))?;

            let mismatch =
                runtime.authorize_child_with_sessions(&authority, registration, &observer);
            if !matches!(mismatch, Err(PeerDenial::StaleEndpoint))
                || observer.bind_peer_callbacks() != 0
                || observer.observed_peer_pid() != 0
                || observer.returned_peer_pid() != 0
                || observer.activation_callbacks() != 0
                || observer.abort_callbacks() != 0
                || observer.gate_effect_callbacks() != 0
            {
                return Err(PeerDenial::StalePeer);
            }

            runtime.endpoints.set_owner_key_override(None)?;
            let post_callback =
                runtime.authorize_child_with_sessions(&authority, registration, &observer);
            if !matches!(post_callback, Err(PeerDenial::StaleEndpoint)) {
                return Err(PeerDenial::StaleEndpoint);
            }
            observer
                .rejected_after_revocation
                .store(1, Ordering::SeqCst);
            if observer.bind_peer_callbacks() != 1
                || observer.observed_peer_pid() != child.id()
                || observer.returned_peer_pid() != child.id()
                || !observer.owner_revocation_rejected_proof()
                || observer.post_bind_authority_callbacks() != 0
                || observer.activation_callbacks() != 0
                || observer.abort_callbacks() != 1
                || observer.gate_effect_callbacks() != 0
            {
                return Err(PeerDenial::StalePeer);
            }
            runtime.endpoints.revoke_endpoint()?;
            drop(foreign_owner);
            Ok(())
        })();

        drop(child_signal);
        let child_status = child.wait().map_err(|_| PeerDenial::Unavailable)?;
        let _ = std::fs::remove_file(&endpoint_path);
        let _ = std::fs::remove_file(&foreign_path);
        run_result?;
        if !child_status.success() {
            return Err(PeerDenial::ProcessInstanceUnavailable);
        }
        Ok(())
    }

    /// Exercise the real C2 accepted-listener authorization callsite on Linux
    /// with a spawned child, its C3 token/pidfd, and the C2 spawn-owned
    /// endpoint path. Endpoint and Session owners remain test fixtures.
    pub(crate) fn run_c2_peer_proof_owner_revocation_regression() -> Result<(), PeerDenial> {
        let init_pid = unsafe { libc::getpid() as u32 };
        if init_pid != 1 {
            return Err(PeerDenial::InvalidProcess);
        }

        for revocation in [
            ProofOwnerRevocation::Endpoint,
            ProofOwnerRevocation::Registration,
            ProofOwnerRevocation::AfterC2OwnerGate,
        ] {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| PeerDenial::Unavailable)?
                .as_nanos();
            let path = std::path::PathBuf::from(format!(
                "/tmp/nagi-c2-peer-proof-{}-{nonce}-{}.sock",
                init_pid,
                match revocation {
                    ProofOwnerRevocation::Endpoint => "endpoint",
                    ProofOwnerRevocation::Registration => "registration",
                    ProofOwnerRevocation::AfterC2OwnerGate => "after-c2-owner-gate",
                }
            ));

            let mut child = std::process::Command::new("/init")
                .arg("--c2-owner-key-peer")
                .arg(&path)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .spawn()
                .map_err(|_| PeerDenial::Unavailable)?;
            let mut child_signal = child.stdin.take().ok_or(PeerDenial::Unavailable)?;

            let run_result = (|| {
                let listener = UnixListener::bind(&path).map_err(|_| PeerDenial::Unavailable)?;
                listener
                    .set_nonblocking(true)
                    .map_err(|_| PeerDenial::Unavailable)?;
                let authority = PidfdProcessInstanceAuthority::new(LinuxPidfdKernel);
                let child_pid = child.id();
                let spawn = nagi_init_spawn_contract::test_pidfd_for_existing_process(child_pid)
                    .map_err(|_| PeerDenial::ProcessInstanceUnavailable)?;
                let (runtime, record, registration, process) =
                    RuntimeAuthorityFixture::new_spawn_owned(listener, spawn, &authority)?;
                let task24_identity = runtime.directory.snapshot(registration)?.task24_identity;
                let observer =
                    runtime.proof_revocation_observer(record, registration, process, revocation);
                child_signal
                    .write_all(&[1])
                    .map_err(|_| PeerDenial::Unavailable)?;
                child_signal.flush().map_err(|_| PeerDenial::Unavailable)?;
                let authorization =
                    runtime.authorize_child_with_sessions(&authority, registration, &observer);
                let owner_revoked = match revocation {
                    ProofOwnerRevocation::Endpoint => {
                        !runtime.endpoints.is_current(runtime.endpoint)?
                    }
                    ProofOwnerRevocation::Registration => {
                        runtime.supervisor.resolve(child_pid).is_none()
                            && runtime
                                .directory
                                .require_current_peer_proof_owner(
                                    &runtime.run,
                                    registration,
                                    process,
                                    record,
                                    runtime.endpoint,
                                    &task24_identity,
                                )
                                .is_err()
                    }
                    ProofOwnerRevocation::AfterC2OwnerGate => {
                        !runtime.endpoints.is_current(runtime.endpoint)?
                    }
                };
                let denied_after_revocation = authorization.is_err() && owner_revoked;
                let expected_activation =
                    usize::from(revocation == ProofOwnerRevocation::AfterC2OwnerGate);
                let expected_rollback = expected_activation;
                let expected_gate_effect = expected_activation;
                if denied_after_revocation {
                    observer
                        .rejected_after_revocation
                        .store(1, Ordering::SeqCst);
                }
                if !denied_after_revocation
                    || observer.observed_peer_pid() != child_pid
                    || !observer.owner_revocation_rejected_proof()
                    || observer.returned_peer_pid() != child_pid
                    || observer.post_bind_authority_callbacks() != 0
                    || observer.activation_callbacks() != expected_activation
                    || observer.abort_callbacks() != 1
                    || observer.simulated_binding_lookup_succeeds()
                    || observer.simulated_binding_rollback_count() != expected_rollback
                    || observer.gate_effect_callbacks() != expected_gate_effect
                {
                    eprintln!(
                        "C2_PENDING_BINDING_FIXTURE=CHECK_FAIL denied={} owner_revoked={} observed_pid={} returned_pid={} expected_activation={} activation={} abort={} simulated_lookup={} expected_rollback={} rollback={} gate_effect={}",
                        authorization.is_err(),
                        owner_revoked,
                        observer.observed_peer_pid(),
                        observer.returned_peer_pid(),
                        expected_activation,
                        observer.activation_callbacks(),
                        observer.abort_callbacks(),
                        observer.simulated_binding_lookup_succeeds(),
                        expected_rollback,
                        observer.simulated_binding_rollback_count(),
                        observer.gate_effect_callbacks(),
                    );
                    return Err(PeerDenial::StalePeer);
                }
                runtime.endpoints.revoke_endpoint()?;
                Ok::<(), PeerDenial>(())
            })();

            drop(child_signal);
            let child_status = child.wait().map_err(|_| PeerDenial::Unavailable)?;
            let _ = std::fs::remove_file(&path);
            run_result?;
            if !child_status.success() {
                return Err(PeerDenial::ProcessInstanceUnavailable);
            }
        }
        Ok(())
    }
}
