use std::any::Any;
use std::cell::Cell;
#[cfg(test)]
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;
use std::num::NonZeroU64;
use std::ops::{Deref, DerefMut};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock, RwLockReadGuard, Weak};

use crate::model::{
    Clock, GuestCleanupStatus, IdentitySnapshot, LocalUser, LocalUserMetadata, ProfileKind,
    ProfileMetadata, ProfileRecord, SessionEndReason, SessionKind, SessionLifecycle, SessionRecord,
    SnapshotError, TryClockError, UnixMillis, IDENTITY_SCHEMA_VERSION,
};
use crate::storage::{ProfileStorageBackend, StorageRootEventSink, UserStorageRootAdapter};
#[cfg(any(test, feature = "system-service-recovery-contract"))]
use crate::store::{
    C1RecoveryOperationIdentity, RecoveryJournalError, RecoveryJournalPhase, RecoveryJournalStore,
    RecoveryOperationId, RecoveryOperationPermit, RecoveryReconciliation,
};
use crate::store::{
    CurrentSnapshotReader, IdentitySnapshotStore, RecoveryBinding, SnapshotStoreError,
    SnapshotVersion,
};
use crate::{CallerContextId, IdempotencyKey, ProfileId, ProviderSubjectRef, SessionId, UserId};
use crate::{CapabilityPrincipalAdapter, PrincipalId, PrincipalMappingFailure};

thread_local! {
    /// C1 lease scopes are intentionally non-reentrant. Per-Session reads and
    /// lifecycle writes use try-lock admission so cross-thread contention
    /// fails closed instead of waiting on an RwLock policy.
    static C1_LEASE_GATE_HELD: Cell<bool> = const { Cell::new(false) };
}

thread_local! {
    /// Identifies the exact staged binding whose read-only owner-admission
    /// callback is currently running under the C1 Session lease. Activation
    /// must occur in this scope; a prior point-in-time liveness check is not
    /// sufficient.
    static C1_PENDING_SESSION_ADMISSION: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
thread_local! {
    /// Deterministically holds a live Session effect after C1 activity
    /// admission but before it attempts the Session read gate.
    static LEASE_READ_CHANNELS: RefCell<Option<(std::sync::mpsc::SyncSender<()>, std::sync::mpsc::Receiver<()>)>> = const { RefCell::new(None) };
}

#[cfg(test)]
thread_local! {
    /// Test-only evidence that recovery acquired earlier lease write guards
    /// before a later lease returned `SessionLeaseBusy`.
    static RECOVERY_WRITE_GUARDS_ACQUIRED: Cell<usize> = const { Cell::new(0) };
}

#[cfg(test)]
fn wait_for_lease_read_barrier_for_test() {
    let channels = LEASE_READ_CHANNELS.with(|slot| slot.borrow_mut().take());
    if let Some((entered, release)) = channels {
        entered
            .send(())
            .expect("the test must observe activity before read-gate acquisition");
        release
            .recv_timeout(std::time::Duration::from_secs(3))
            .expect("the test must release the admitted reader within the watchdog bound");
    }
}

struct C1LeaseGateScope(PhantomData<Rc<()>>);

impl C1LeaseGateScope {
    fn enter() -> Result<Self, ResolutionError> {
        C1_LEASE_GATE_HELD.with(|held| {
            if held.replace(true) {
                Err(ResolutionError::Unavailable)
            } else {
                Ok(Self(PhantomData))
            }
        })
    }

    fn try_enter() -> Result<Self, TryLiveError> {
        C1_LEASE_GATE_HELD.with(|held| {
            if held.replace(true) {
                Err(TryLiveError::StateUnavailable)
            } else {
                Ok(Self(PhantomData))
            }
        })
    }

    fn held_on_current_thread() -> bool {
        C1_LEASE_GATE_HELD.with(Cell::get)
    }
}

impl Drop for C1LeaseGateScope {
    fn drop(&mut self) {
        C1_LEASE_GATE_HELD.with(|held| held.set(false));
    }
}

struct C1SessionAdmissionScope(usize);

impl C1SessionAdmissionScope {
    fn enter(binding: usize) -> Result<Self, C1SessionAdmissionError> {
        let entered = C1_PENDING_SESSION_ADMISSION.with(|current| {
            if current.get() != 0 {
                return false;
            }
            current.set(binding);
            true
        });
        entered
            .then_some(Self(binding))
            .ok_or(C1SessionAdmissionError::AdmissionAlreadyStarted)
    }

    fn held_for(binding: usize) -> bool {
        C1_PENDING_SESSION_ADMISSION.with(|current| current.get() == binding)
    }
}

impl Drop for C1SessionAdmissionScope {
    fn drop(&mut self) {
        C1_PENDING_SESSION_ADMISSION.with(|current| {
            if current.get() == self.0 {
                current.set(0);
            }
        });
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityAction {
    CreateLocalUser,
    CreateProfile,
    LoginLocalUser,
    CreateLocalSession,
    CreateGuestSession,
    ResolveCurrentIdentity,
    AccessProfileRoot,
    EndSession,
    Recover,
    ValidateRecoveredState,
}

/// Base identity identifiers from an integration boundary. App payloads must
/// never supply these values, and this trait alone is not authentication
/// evidence: Session APIs require [`AuthenticatedInteractiveCaller`] or
/// [`AuthenticatedSystemServicePrincipal`] as appropriate.
pub trait TrustedCallerContext {
    fn principal_id(&self) -> &crate::PrincipalId;
    fn context_id(&self) -> &CallerContextId;
}

/// Private proof traits seal the authentication boundaries. Only code in this
/// crate can mint these proofs; external callers must wait for the real C2/C3
/// producer adapters instead of asserting authentication themselves.
pub(crate) mod caller_proof_seal {
    pub trait Interactive {}
    pub trait SystemService {}
    pub trait Provisioning {}
}

/// A caller context emitted by the trusted peer-authentication adapter after
/// authenticating the OS peer for this connection. The trait is sealed so a
/// consumer cannot mark its own payload-supplied identifiers as authenticated.
///
/// ```compile_fail,E0277
/// use nagi_identity_session::{AuthenticatedInteractiveCaller, CallerContextId, PrincipalId, TrustedCallerContext};
/// struct PayloadCaller;
/// impl TrustedCallerContext for PayloadCaller {
///     fn principal_id(&self) -> &PrincipalId { unimplemented!() }
///     fn context_id(&self) -> &CallerContextId { unimplemented!() }
/// }
/// fn require_authenticated<T: AuthenticatedInteractiveCaller>() {}
/// require_authenticated::<PayloadCaller>();
/// ```
///
/// ```compile_fail,E0277
/// use nagi_identity_session::{AuthenticatedInteractiveCaller, AuthenticatedSystemServicePrincipal, CallerContextId, PrincipalId, TrustedCallerContext};
/// struct InteractivePeer;
/// impl TrustedCallerContext for InteractivePeer {
///     fn principal_id(&self) -> &PrincipalId { unimplemented!() }
///     fn context_id(&self) -> &CallerContextId { unimplemented!() }
/// }
/// impl AuthenticatedInteractiveCaller for InteractivePeer {}
/// fn require_service<T: AuthenticatedSystemServicePrincipal>() {}
/// require_service::<InteractivePeer>();
/// ```
///
/// ```compile_fail
/// use nagi_identity_session::{AuthenticatedInteractiveCaller, CallerContextId, PrincipalId, TrustedCallerContext};
/// struct PayloadCaller;
/// impl TrustedCallerContext for PayloadCaller {
///     fn principal_id(&self) -> &PrincipalId { unimplemented!() }
///     fn context_id(&self) -> &CallerContextId { unimplemented!() }
/// }
/// impl AuthenticatedInteractiveCaller for PayloadCaller {}
/// ```
///
/// ```compile_fail
/// use nagi_identity_session::{AuthenticatedInteractiveCaller, AuthenticatedProvisioningCaller};
/// fn interactive_cannot_provision<T: AuthenticatedInteractiveCaller>() {
///     fn create_profile<T: AuthenticatedProvisioningCaller>() {}
///     create_profile::<T>();
/// }
/// ```
///
/// ```compile_fail
/// use nagi_identity_session::{AuthenticatedSystemServicePrincipal, CallerContextId, PrincipalId, TrustedCallerContext};
/// struct ClaimedSystem;
/// impl TrustedCallerContext for ClaimedSystem {
///     fn principal_id(&self) -> &PrincipalId { unimplemented!() }
///     fn context_id(&self) -> &CallerContextId { unimplemented!() }
/// }
/// impl AuthenticatedSystemServicePrincipal for ClaimedSystem {}
/// ```
///
/// ```compile_fail
/// use nagi_identity_session::{AuthenticatedInteractiveCaller, AuthenticatedSystemServicePrincipal};
/// fn interactive_cannot_recover<T: AuthenticatedInteractiveCaller>() {
///     fn recover<T: AuthenticatedSystemServicePrincipal>() {}
///     recover::<T>();
/// }
/// ```
#[allow(private_bounds)]
pub trait AuthenticatedInteractiveCaller:
    TrustedCallerContext + caller_proof_seal::Interactive
{
}

/// The distinct caller context for an OS service principal. Its implementation
/// must bind OS credentials to ownership of the service endpoint. It is not
/// derived from, or interchangeable with, an interactive Session. This proof
/// is separately sealed from interactive and provisioning caller proofs.
/// Numeric process credentials alone cannot create this authority:
///
/// ```compile_fail,E0277
/// use nagi_identity_session::AuthenticatedSystemServicePrincipal;
/// struct OsPeerCredentials { pid: u32, uid: u32, gid: u32 }
/// fn require_system_service<T: AuthenticatedSystemServicePrincipal>() {}
/// require_system_service::<OsPeerCredentials>();
/// ```
#[allow(private_bounds)]
pub trait AuthenticatedSystemServicePrincipal:
    TrustedCallerContext + caller_proof_seal::SystemService
{
}

/// A trusted provisioning operation context. Account and profile creation
/// needs an authenticated provisioning path; an interactive Session context
/// alone is not sufficient. The account mapping owner supplies the real
/// producer adapter.
#[allow(private_bounds)]
pub trait AuthenticatedProvisioningCaller:
    TrustedCallerContext + caller_proof_seal::Provisioning
{
}

/// Caller contexts may be unique only inside a principal namespace. The
/// service therefore binds every in-memory current-session key to both values.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CallerKey {
    principal_id: PrincipalId,
    context_id: CallerContextId,
}

impl CallerKey {
    fn from_trusted(caller: &(impl TrustedCallerContext + ?Sized)) -> Self {
        Self {
            principal_id: caller.principal_id().clone(),
            context_id: caller.context_id().clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationRequest {
    pub action: IdentityAction,
    pub caller_principal: crate::PrincipalId,
    pub caller_context: CallerContextId,
    pub user_id: Option<UserId>,
    pub profile_id: Option<ProfileId>,
    pub session_id: Option<SessionId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationFailure {
    Denied,
    Unavailable,
}

/// The identity crate asks the owning host for a decision. It does not own a
/// capability policy or treat persisted identity data as authority.
pub trait IdentityAuthorizer {
    fn authorize(&mut self, request: &AuthorizationRequest) -> Result<(), AuthorizationFailure>;

    /// Authorize the post-persist resolution of a Session committed while the
    /// exact C1 RuntimeActive operation is admitted. The default preserves an
    /// implementor's ordinary policy; production boot policy may impose the
    /// stronger operation and boot-principal binding required for this path.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    fn authorize_with_runtime_admission(
        &mut self,
        request: &AuthorizationRequest,
        _admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<(), AuthorizationFailure> {
        self.authorize(request)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdSourceFailure {
    Unavailable,
}

/// Production implementations must draw from an approved unpredictable
/// source and check collisions against the durable snapshot. Session IDs from
/// this source are nonce inputs: IdentityService binds each one to the next
/// store CAS generation before persisting it, so a reset source cannot reuse a
/// SessionId from a lost snapshot generation.
pub trait IdentityIdSource {
    fn next_user_id(&mut self) -> Result<UserId, IdSourceFailure>;
    fn next_profile_id(&mut self) -> Result<ProfileId, IdSourceFailure>;
    /// Supply a nonce for a service-issued SessionId. The service adds the
    /// next store CAS generation and does not return this value unchanged; its
    /// suffix may be compacted to fit the validated ID length. Treat issued
    /// SessionIds as opaque.
    fn next_session_id(&mut self) -> Result<SessionId, IdSourceFailure>;

    /// Supply a fresh C1 caller-context identifier for a connection-bound
    /// caller issuer. This identifier is only a selector: it does not
    /// authenticate a peer or mint an `AuthenticatedInteractiveCaller`.
    /// Sources that have not adopted C1-owned context issuance fail closed.
    fn next_caller_context_id(&mut self) -> Result<CallerContextId, IdSourceFailure> {
        Err(IdSourceFailure::Unavailable)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuestCleanupFailure {
    Unavailable,
}

/// Cleanup must be idempotent and scoped to exactly the supplied ephemeral
/// ProfileId. It must not accept a host path or traverse a neighboring profile.
pub trait GuestProfileCleaner {
    fn cleanup_guest_profile(&mut self, profile_id: &ProfileId) -> Result<(), GuestCleanupFailure>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityEventKind {
    LocalUserCreated,
    ProfileCreated,
    LocalSessionCreated,
    GuestSessionCreated,
    CurrentIdentityResolved,
    CurrentIdentityResolutionDenied,
    PrincipalMappingAllowed,
    SessionEnded,
    PersistenceCommitted,
    PersistenceCommitFailed,
    RecoveryUsedLastKnownGood,
    CorruptStateRejected,
    GuestCleanupFailed,
    PrincipalMappingDenied,
    StorageRootResolutionFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityEventReason {
    NoSession,
    Ended,
    Expired,
    Corrupt,
    Unavailable,
    Conflict,
    Denied,
    CleanupPending,
    Recovery,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityEvent {
    pub kind: IdentityEventKind,
    pub user_id: Option<UserId>,
    pub profile_id: Option<ProfileId>,
    pub session_id: Option<SessionId>,
    pub reason: Option<IdentityEventReason>,
}

pub trait IdentityEventSink {
    fn emit(&mut self, event: &IdentityEvent) -> bool;
}

#[derive(Default)]
pub struct NoopIdentityEventSink;

impl IdentityEventSink for NoopIdentityEventSink {
    fn emit(&mut self, _event: &IdentityEvent) -> bool {
        true
    }
}

/// Failure returned by the external exact-generation owner gate. This trait
/// only orders the callback; it is not authentication evidence. Production
/// implementations must delegate to the A/C3 one-shot gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(feature = "system-service-recovery-contract"), allow(dead_code))]
pub enum RecoveryEffectGateError {
    Unavailable,
}

/// A's bridge for one exact C3-gated C1 recovery effect.
///
/// Implementations must use the same private A admission that owns `scope`,
/// hold its real C3 process/endpoint gate while invoking `effect` exactly
/// once, and forward `operation` unchanged to the owner-side correlation
/// boundary. This value is correlation from C1's durable effect claim, not an
/// authorization token. A public scope or an implementation of this trait
/// cannot create a C1 caller or operation identity.
#[cfg(feature = "system-service-recovery-contract")]
pub trait C1SystemServiceRecoveryEffectGate {
    fn apply_recovery_effect<R>(
        &mut self,
        scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        operation: C1RecoveryOperationIdentity,
        effect: impl FnOnce() -> Result<R, IdentityError>,
    ) -> Result<Result<R, IdentityError>, RecoveryEffectGateError>;
}

/// A's bridge for terminal reconciliation of an operation recorded by a
/// previous C3 launch.
///
/// Production implementations must hold the C3 owner gate, obtain the older
/// scope from its durable journal row, and return a durable, read-back
/// `C3DurableRecoverySlotReceipt` for `claim`. The C1 callback receives that
/// owner-supplied previous scope and validates it against C1's exact durable
/// operation before C3 may commit terminal state. Callers must not synthesize
/// the old scope from the current boot or from C1's digest-only journal row.
/// The callback must not commit the C1 journal. C1 publishes its pending row
/// as reconciled only after validating the typed receipt's prior
/// service/catalog/launch, C1 operation ID, binding digest, and
/// `ClearedAndAbsent` state. That shared receipt attests to durable C3 slot
/// state; it does not expose the previous `SystemServiceLaunchRecord`,
/// process identity, or normal exit status. If a caller requires those facts
/// and cannot obtain them after a cold reboot, it must return `OutcomeUnknown`;
/// C1 never infers them from an absent owner object. An unknown result leaves
/// the pending row untouched, including when C3 reports uncertainty after the
/// callback. This trait provides no C1 authority by itself.
#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
pub trait C1PreviousLaunchTerminalGate {
    type TerminalReceipt: nagi_system_service_recovery_contract::C3DurableRecoverySlotReceipt;

    fn with_previous_launch_reconciliation_gate<R>(
        &mut self,
        current_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
        validate_previous_scope: impl FnOnce(
            &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        ) -> Result<
            R,
            nagi_system_service_recovery_contract::RecoveryClientError,
        >,
    ) -> Result<
        (R, Self::TerminalReceipt),
        nagi_system_service_recovery_contract::RecoveryClientError,
    >;
}

/// C3 owner gate for durable evidence that a prior boot was cleared.
#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
pub trait C1PreviousBootTerminalGate {
    type TerminalReceipt: nagi_system_service_recovery_contract::C3DurablePriorBootClearedReceipt;

    fn with_previous_boot_reconciliation_gate<R>(
        &mut self,
        current_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
        validate_previous_scope: impl FnOnce(
            &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        ) -> Result<
            R,
            nagi_system_service_recovery_contract::RecoveryClientError,
        >,
    ) -> Result<
        (R, Self::TerminalReceipt),
        nagi_system_service_recovery_contract::RecoveryClientError,
    >;
}

#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
type PreviousScopeValidator<'a> = dyn FnMut(
        &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
    ) -> Result<
        OwnedRecoveryScopeSnapshot,
        nagi_system_service_recovery_contract::RecoveryClientError,
    > + 'a;

#[cfg(feature = "system-service-recovery-contract")]
trait C1TerminalReconciliationGate {
    fn with_reconciliation_gate<R>(
        &mut self,
        current_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        operation_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
        reconcile: impl FnOnce() -> R,
    ) -> Result<R, nagi_system_service_recovery_contract::RecoveryClientError>;
}

#[cfg(feature = "system-service-recovery-contract")]
struct CurrentLaunchTerminalGate<'gate, Gate>(&'gate mut Gate);

#[cfg(feature = "system-service-recovery-contract")]
impl<Gate: nagi_system_service_recovery_contract::C3RecoverySlotGate> C1TerminalReconciliationGate
    for CurrentLaunchTerminalGate<'_, Gate>
{
    fn with_reconciliation_gate<R>(
        &mut self,
        current_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        operation_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
        reconcile: impl FnOnce() -> R,
    ) -> Result<R, nagi_system_service_recovery_contract::RecoveryClientError> {
        use nagi_system_service_recovery_contract::RecoveryClientError;
        if !std::ptr::eq(current_scope, operation_scope) {
            return Err(RecoveryClientError::Rejected);
        }
        self.0
            .with_reconciliation_gate(operation_scope, claim, reconcile)
    }
}

#[cfg(feature = "system-service-recovery-contract")]
mod c1_recovery_journal_seal {
    pub trait Sealed {}
}

/// A C1-owned store capability for the production recovery journal API.
///
/// This marker is sealed to the maintained Linux durable store. Test stores
/// and arbitrary external `IdentitySnapshotStore` implementations cannot
/// claim durable recovery support by implementing a public trait.
#[cfg(feature = "system-service-recovery-contract")]
#[allow(private_bounds)]
pub trait C1SystemServiceRecoveryJournal:
    IdentitySnapshotStore + RecoveryJournalStore + c1_recovery_journal_seal::Sealed
{
}

#[cfg(all(feature = "system-service-recovery-contract", target_os = "linux"))]
impl c1_recovery_journal_seal::Sealed for crate::store::LinuxIdentitySnapshotStore {}

#[cfg(all(feature = "system-service-recovery-contract", target_os = "linux"))]
impl C1SystemServiceRecoveryJournal for crate::store::LinuxIdentitySnapshotStore {}

/// Leaf adapter seam for running one C1 recovery effect under the current
/// exact-generation owner gate without making C1 depend on A or C3 crates.
#[cfg(any(test, feature = "system-service-recovery-contract"))]
pub(crate) trait SystemServiceRecoveryEffectGate {
    /// The exact C1 binding represented by this owner admission. Production
    /// adapters must derive it from the same sealed A/C3 admission used by
    /// `apply_recovery_effect`; callers cannot pair a separate binding with a
    /// different gate.
    fn recovery_binding(&self) -> &RecoveryBinding;

    /// Correlate the owner-side durable transition with C1's already claimed
    /// operation. This identity is not authorization and does not attest to
    /// the owner's durable outcome.
    fn apply_recovery_effect<R>(
        &mut self,
        operation: C1RecoveryOperationIdentity,
        effect: impl FnOnce() -> Result<R, IdentityError>,
    ) -> Result<Result<R, IdentityError>, RecoveryEffectGateError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    NotRecovered,
    AuthorizationDenied,
    AuthorizationUnavailable,
    /// A C1 lifecycle operation was called from inside a live effect callback.
    LeaseEffectReentry,
    /// A lifecycle operation could not acquire the Session lease immediately;
    /// no durable transition was committed and the caller may retry.
    SessionLeaseBusy,
    IdSourceUnavailable,
    IdCollision,
    /// ProfileId history cannot be proven complete, so new users, profiles,
    /// and guest profiles are blocked. This crate has no authoritative repair
    /// API; only an external authoritative history repair can restore creation.
    ProfileHistoryIncomplete,
    InvalidMetadata,
    UserNotFound,
    ProfileNotFound,
    ProfileOwnershipMismatch,
    ProfileAlreadyActive,
    CallerAlreadyHasSession,
    IdempotencyConflict,
    ExpiryNotInFuture,
    SessionNotFound,
    SessionNotCurrent,
    StoreUnavailable,
    StoreConflict,
    /// A lifecycle cleanup helper was given an adapter owned by another
    /// issuer. No durable transition or foreign-root removal was performed.
    StorageOwnerMismatch,
    CorruptState,
    UnsupportedSchemaVersion,
    RevisionExhausted,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotRecovered => "identity state has not completed recovery",
            Self::AuthorizationDenied => "identity operation was denied",
            Self::AuthorizationUnavailable => "identity authorization is unavailable",
            Self::LeaseEffectReentry => {
                "identity lifecycle operation re-entered a live Session effect"
            }
            Self::SessionLeaseBusy => {
                "identity Session lease is busy; retry after the active use finishes"
            }
            Self::IdSourceUnavailable => "identity ID source is unavailable",
            Self::IdCollision => "identity ID collides with an existing record",
            Self::ProfileHistoryIncomplete => {
                "profile ID history is incomplete; new identity creation requires authoritative repair"
            }
            Self::InvalidMetadata => "identity metadata is invalid",
            Self::UserNotFound => "local user was not found",
            Self::ProfileNotFound => "profile was not found",
            Self::ProfileOwnershipMismatch => "profile does not belong to the requested user",
            Self::ProfileAlreadyActive => "profile already has an active identity session",
            Self::CallerAlreadyHasSession => "caller already has a current identity session",
            Self::IdempotencyConflict => "idempotency key conflicts with an earlier request",
            Self::ExpiryNotInFuture => "session expiry must be later than its creation time",
            Self::SessionNotFound => "session was not found",
            Self::SessionNotCurrent => "session is not current for this caller",
            Self::StoreUnavailable => "identity snapshot store is unavailable",
            Self::StoreConflict => "identity snapshot changed since it was read",
            Self::StorageOwnerMismatch => {
                "profile storage adapter is bound to a different identity issuer"
            }
            Self::CorruptState => "identity state is corrupt and requires repair",
            Self::UnsupportedSchemaVersion => "identity state schema version is unsupported",
            Self::RevisionExhausted => "identity snapshot revision is exhausted",
        })
    }
}

impl std::error::Error for IdentityError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionError {
    NoSession,
    Ended,
    Expired,
    CorruptState,
    Unavailable,
    Denied,
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoSession => "no current identity session exists for this caller",
            Self::Ended => "the caller's current identity session has ended",
            Self::Expired => "the caller's current identity session has expired",
            Self::CorruptState => "identity state is corrupt and was rejected",
            Self::Unavailable => "identity state or its dependencies are unavailable",
            Self::Denied => "current identity resolution was denied",
        })
    }
}

impl std::error::Error for ResolutionError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoverySource {
    Current,
    LastKnownGood,
    Empty,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryReport {
    pub source: RecoverySource,
    pub interrupted_sessions_ended: usize,
    pub guest_cleanups_pending: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndSessionResult {
    Ended,
    AlreadyEnded,
    GuestCleanupPending,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SessionIntent {
    kind: SessionKind,
    user_id: Option<UserId>,
    profile_id: ProfileId,
    expires_at: Option<UnixMillis>,
}

#[derive(Clone, Debug)]
struct IdempotentSession {
    session_id: SessionId,
    intent: SessionIntent,
}

/// A resolved identity is an in-process, revocable observation. It is not a
/// durable login token and it carries no capability grant by itself.
pub struct ResolvedIdentity {
    session_id: SessionId,
    kind: SessionKind,
    user_id: Option<UserId>,
    profile_id: ProfileId,
    caller: CallerKey,
    expires_at: Option<UnixMillis>,
    clock_now: Arc<dyn Fn() -> UnixMillis + Send + Sync>,
    clock_try_now: Arc<dyn Fn() -> Result<UnixMillis, TryClockError> + Send + Sync>,
    live: Arc<SessionLeaseState>,
    issuer_alive: Arc<AtomicBool>,
    issuer_admission: Arc<IssuerAdmission>,
    issued_session: SessionRecord,
    store_current: Option<CurrentSnapshotReader>,
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    runtime_operation_identity: Option<C1RecoveryOperationIdentity>,
}

impl fmt::Debug for ResolvedIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedIdentity")
            .field("session_id", &self.session_id)
            .field("kind", &self.kind)
            .field("user_id", &self.user_id)
            .field("profile_id", &self.profile_id)
            .field("caller", &self.caller)
            .field("expires_at", &self.expires_at)
            .field("live", &self.is_live())
            .finish()
    }
}

impl ResolvedIdentity {
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub const fn kind(&self) -> SessionKind {
        self.kind
    }

    pub const fn user_id(&self) -> Option<UserId> {
        self.user_id
    }

    pub fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.caller.principal_id
    }

    pub fn caller_context_id(&self) -> &CallerContextId {
        &self.caller.context_id
    }

    /// Exact C1 recovery operation whose admitted snapshot reader backs this
    /// resolved identity. This is correlation only; the identity's live
    /// Session lease remains the authority boundary for protected effects.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    pub const fn runtime_operation_identity(&self) -> Option<C1RecoveryOperationIdentity> {
        self.runtime_operation_identity
    }

    /// Lossy convenience check. `false` means the Session was not confirmed
    /// live; it can reflect a transient unavailable gate/store as well as a
    /// durable end or expiry. Use [`Self::check_live`] when the distinction
    /// matters, and do not treat `false` alone as proof of revocation.
    pub fn is_live(&self) -> bool {
        self.acquire_live_lease().is_ok()
    }

    /// Revalidate this service-issued observation against the live lease,
    /// expiry clock, and current committed snapshot. A saved Session record
    /// alone is never enough to make this observation live again.
    pub fn check_live(&self) -> Result<(), ResolutionError> {
        self.checked_live_lease().map(drop)
    }

    /// Read the process-local lease without waiting on the lease lock or clock
    /// lock. This is a point-in-time observation for connection maintenance;
    /// it does not read the durable snapshot and is not authority to perform
    /// protected work. Call `check_live` or the owning operation gate before
    /// every protected effect.
    pub fn try_live_snapshot(&self) -> Result<LiveSnapshot, TryLiveError> {
        let _gate_scope = C1LeaseGateScope::try_enter()?;
        let Some(_issuer_activity) = self.issuer_admission.try_enter()? else {
            return Ok(LiveSnapshot {
                generation: self.live.generation,
                status: LiveStatus::Revoked,
            });
        };
        if !self.issuer_alive.load(Ordering::Acquire) {
            return Ok(LiveSnapshot {
                generation: self.live.generation,
                status: LiveStatus::Revoked,
            });
        }
        let lease = self
            .live
            .live
            .try_read()
            .map_err(|_| TryLiveError::StateUnavailable)?;
        let observed_revision = self.live.revision.load(Ordering::Acquire);
        if !*lease
            || observed_revision != self.live.generation.get()
            || !self.issuer_alive.load(Ordering::Acquire)
        {
            return Ok(LiveSnapshot {
                generation: self.live.generation,
                status: LiveStatus::Revoked,
            });
        }
        let expired = if let Some(expires_at) = self.expires_at {
            let now = (self.clock_try_now)().map_err(TryLiveError::Clock)?;
            now >= expires_at
        } else {
            false
        };
        let revision_after = self.live.revision.load(Ordering::Acquire);
        if revision_after != observed_revision
            || !*lease
            || !self.issuer_alive.load(Ordering::Acquire)
        {
            return Ok(LiveSnapshot {
                generation: self.live.generation,
                status: LiveStatus::Revoked,
            });
        }
        let status = if expired {
            LiveStatus::Expired
        } else {
            LiveStatus::Live
        };
        Ok(LiveSnapshot {
            generation: self.live.generation,
            status,
        })
    }

    fn duplicate_observation(&self) -> Self {
        Self {
            session_id: self.session_id.clone(),
            kind: self.kind,
            user_id: self.user_id,
            profile_id: self.profile_id.clone(),
            caller: self.caller.clone(),
            expires_at: self.expires_at,
            clock_now: Arc::clone(&self.clock_now),
            clock_try_now: Arc::clone(&self.clock_try_now),
            live: Arc::clone(&self.live),
            issuer_alive: Arc::clone(&self.issuer_alive),
            issuer_admission: Arc::clone(&self.issuer_admission),
            issued_session: self.issued_session.clone(),
            store_current: self.store_current.clone(),
            #[cfg(all(
                feature = "system-service-recovery-contract",
                target_os = "linux",
                target_arch = "x86_64"
            ))]
            runtime_operation_identity: self.runtime_operation_identity,
        }
    }

    pub(crate) fn acquire_live_lease(&self) -> Result<LiveSessionLeaseGuard<'_>, ResolutionError> {
        self.checked_live_lease()
    }

    pub(crate) fn issuer_identity(&self) -> std::sync::Weak<AtomicBool> {
        Arc::downgrade(&self.issuer_alive)
    }

    pub(crate) fn bind_storage_adapter(
        &self,
        adapter_identity: &Arc<()>,
    ) -> Result<(), ResolutionError> {
        let mut state = self
            .issuer_admission
            .state
            .try_lock()
            .map_err(|_| ResolutionError::Unavailable)?;
        if state.closed || !self.issuer_alive.load(Ordering::Acquire) {
            return Err(ResolutionError::Unavailable);
        }
        match state.storage_adapter.as_ref() {
            Some(owner) if !Weak::ptr_eq(owner, &Arc::downgrade(adapter_identity)) => {
                if owner.upgrade().is_some() {
                    Err(ResolutionError::Unavailable)
                } else {
                    state.storage_adapter = Some(Arc::downgrade(adapter_identity));
                    Ok(())
                }
            }
            Some(_) => Ok(()),
            None => {
                state.storage_adapter = Some(Arc::downgrade(adapter_identity));
                Ok(())
            }
        }
    }

    pub(crate) fn accepts_storage_adapter(
        &self,
        adapter_identity: &Arc<()>,
    ) -> Result<bool, ResolutionError> {
        let state = self
            .issuer_admission
            .state
            .try_lock()
            .map_err(|_| ResolutionError::Unavailable)?;
        Ok(state.storage_adapter.as_ref().is_none_or(|owner| {
            Weak::ptr_eq(owner, &Arc::downgrade(adapter_identity)) || owner.upgrade().is_none()
        }))
    }

    fn checked_live_lease(&self) -> Result<LiveSessionLeaseGuard<'_>, ResolutionError> {
        let gate_scope = C1LeaseGateScope::enter()?;
        let issuer_activity = self.issuer_admission.enter()?;
        if !self.issuer_alive.load(Ordering::Acquire) {
            return Err(ResolutionError::Unavailable);
        }
        #[cfg(test)]
        wait_for_lease_read_barrier_for_test();
        let guard = self
            .live
            .live
            .try_read()
            .map_err(|_| ResolutionError::Unavailable)?;
        if !*guard || self.live.revision.load(Ordering::Acquire) != self.live.generation.get() {
            return Err(ResolutionError::Ended);
        }
        if self
            .expires_at
            .is_some_and(|expires| (self.clock_now)() >= expires)
        {
            return Err(ResolutionError::Expired);
        }
        // Only the current committed snapshot can confirm a lease. In
        // particular, recovery by a different instance must revoke this one.
        let reader = self
            .store_current
            .as_ref()
            .ok_or(ResolutionError::Unavailable)?;
        let stored = reader()
            .map_err(|_| ResolutionError::Unavailable)?
            .ok_or(ResolutionError::Unavailable)?;
        let snapshot = IdentitySnapshot::from_json(&stored.bytes)
            .map_err(|_| ResolutionError::CorruptState)?;
        if !snapshot.sessions.iter().any(|session| {
            session == &self.issued_session && matches!(session.lifecycle, SessionLifecycle::Active)
        }) {
            return Err(ResolutionError::Ended);
        }
        // A store read can take time; expiry must also be checked afterward.
        if self
            .expires_at
            .is_some_and(|expires| (self.clock_now)() >= expires)
        {
            return Err(ResolutionError::Expired);
        }
        if !self.issuer_alive.load(Ordering::Acquire) {
            return Err(ResolutionError::Unavailable);
        }
        Ok(LiveSessionLeaseGuard {
            _lease: guard,
            _issuer_activity: issuer_activity,
            _gate_scope: gate_scope,
        })
    }
}

/// One admitted C1 use. Fields are deliberately ordered so the per-session
/// read lease is released before issuer shutdown observes the active count
/// reach zero, and the thread-local non-reentry marker is released last.
pub(crate) struct LiveSessionLeaseGuard<'a> {
    _lease: RwLockReadGuard<'a, bool>,
    _issuer_activity: IssuerActivityGuard,
    _gate_scope: C1LeaseGateScope,
}

/// Owned C1 revalidation handle for trusted cross-thread connection owners.
/// It can be created only from a checked `InteractiveSessionContext`, carries
/// the authenticated caller and service-issued Session association, and has
/// no constructor from identifiers or durable records. `try_live_snapshot` is
/// a nonblocking maintenance observation; protected effects still require
/// `check_live` before admission. The handle is not serializable or a grant.
///
/// ```
/// use nagi_identity_session::SessionLeaseRevalidationHandle;
/// fn require_send_sync<T: Send + Sync>() {}
/// require_send_sync::<SessionLeaseRevalidationHandle>();
/// ```
pub struct SessionLeaseRevalidationHandle {
    identity: ResolvedIdentity,
}

impl SessionLeaseRevalidationHandle {
    pub fn session_id(&self) -> &SessionId {
        self.identity.session_id()
    }

    pub fn principal_id(&self) -> &PrincipalId {
        self.identity.principal_id()
    }

    pub fn caller_context_id(&self) -> &CallerContextId {
        self.identity.caller_context_id()
    }

    pub const fn kind(&self) -> SessionKind {
        self.identity.kind()
    }

    pub const fn user_id(&self) -> Option<UserId> {
        self.identity.user_id()
    }

    pub fn profile_id(&self) -> &ProfileId {
        self.identity.profile_id()
    }

    pub fn generation(&self) -> NonZeroU64 {
        self.identity.live.generation
    }

    /// Full revalidation against live lease, expiry, issuer, and the current
    /// committed snapshot. Admission to the per-Session read gate is
    /// zero-wait: if the read cannot be acquired immediately, this returns
    /// `Unavailable` instead of waiting. The subsequent durable snapshot read
    /// may still perform store I/O, so this operation is not an overall
    /// latency bound.
    pub fn check_live(&self) -> Result<(), ResolutionError> {
        self.identity.check_live()
    }

    /// Run one bounded synchronous effect while holding the same Session
    /// lease gate that C1 takes before persisting logout, expiry, recovery, or
    /// another revocation. A point-in-time `check_live` followed by an
    /// unleased effect permits a revoke/commit race; callers that perform
    /// protected effects must put the owner gate and its effect commit inside
    /// this callback.
    ///
    /// The callback must remain on its current thread. It must not spawn or
    /// join a child that calls a C1 lease/lifecycle API, call any C1 lifecycle
    /// API itself, acquire an external lock whose holder may be waiting for a
    /// C1 gate, or call profile storage recursively. Same-thread lease
    /// re-entry fails closed; same-thread lifecycle re-entry returns
    /// [`IdentityError::LeaseEffectReentry`]. `check_live` attempts
    /// per-Session read admission without waiting; if the gate is unavailable,
    /// it fails closed. A contending lifecycle write returns
    /// [`IdentityError::SessionLeaseBusy`] before commit. Callback-created
    /// threads must still not use C1 APIs: authorization and owner/store work
    /// can precede lease admission, and a producer-held mutex can block before
    /// C1 reaches the gate.
    ///
    /// Do not capture or otherwise reach the issuing `IdentityService`, call
    /// `into_parts`, or drop that issuer from this callback. Dropping an issuer
    /// closes admission immediately and defers its store drop until all
    /// already-admitted callbacks return. Such a callback may finish, but no
    /// new callback or later revalidation can use that issuer. `into_parts`
    /// returns the still-live issuer when callbacks are active; retain it and
    /// retry after joining effect workers. These are fail-closed safeguards,
    /// not a shutdown coordination API. A panic propagates and releases the
    /// C1 lease, but does not imply the Session or an external effect was
    /// rolled back. If an external effect may have committed before panic, its
    /// outcome is indeterminate; reconcile or use idempotency before retrying.
    ///
    /// C1 checks expiry at callback admission, including durable revalidation
    /// immediately before entry. An admitted callback may finish after the
    /// expiry timestamp. If policy requires the effect commit itself to occur
    /// before expiry, the producer must recheck its trusted deadline inside
    /// its owner gate immediately before commit. C1 does not promise
    /// commit-time expiry. Acquire C1's lease before any C2 owner gate, and do
    /// not hold a C2 gate while waiting for C1 logout, recovery, or shutdown.
    pub fn with_live_session_effect<R>(
        &self,
        effect: impl FnOnce() -> R,
    ) -> Result<R, ResolutionError> {
        let _lease = self.identity.checked_live_lease()?;
        Ok(effect())
    }

    /// Check C1's exact Session generation against the opaque C2 peer
    /// selector, then hold this C1 lease across the supplied synchronous
    /// callback. `app_id` and `app_session_id` must come from C2's existing
    /// non-cloneable SessionLease; C2 must still validate the complete lease
    /// and endpoint owner before and after invoking its callback.
    ///
    /// This is only the C1 lease half of peer binding. It does not enroll or
    /// authenticate `owner_key`, create a caller proof, map a peer to an
    /// account, or validate C2 registration/run/endpoint/process selectors.
    /// Use an already checked C1 handle and keep C2's owner gate plus effect
    /// commit inside this callback. Until the exact owner key has been
    /// pre-enrolled from a trusted C2 registration path, this method alone
    /// must not be used to accept a peer.
    #[cfg(feature = "c2-accepted-peer")]
    pub fn with_live_c2_peer_session_effect<R>(
        &self,
        selector: &nagi_peer_authenticated_connection::AuthenticatedInteractivePeerSelector,
        app_id: u64,
        app_session_id: u64,
        effect: impl FnOnce() -> R,
    ) -> Result<R, ResolutionError> {
        if !selector.matches_session(app_id, app_session_id, self.generation()) {
            return Err(ResolutionError::Unavailable);
        }
        self.with_live_session_effect(effect)
    }

    /// Zero-wait lease/expiry observation for connection maintenance. It does
    /// not read current durable state and cannot authorize a protected effect.
    pub fn try_live_snapshot(&self) -> Result<LiveSnapshot, TryLiveError> {
        self.identity.try_live_snapshot()
    }
}

const C1_SESSION_EFFECT_PENDING: u8 = 0;
const C1_SESSION_EFFECT_ACTIVE: u8 = 1;
const C1_SESSION_EFFECT_REVOKED: u8 = 2;

struct C1SessionEffectBindingState {
    phase: AtomicU8,
    preflight_started: AtomicBool,
    lease: SessionLeaseRevalidationHandle,
}

impl C1SessionEffectBindingState {
    fn revoke_if_pending(&self) {
        let _ = self.phase.compare_exchange(
            C1_SESSION_EFFECT_PENDING,
            C1_SESSION_EFFECT_REVOKED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

/// A C1 Session-only staged effect gate created from a checked interactive
/// context. It is not a peer proof, owner enrollment, caller seal, or effect
/// authorization; a C2 owner adapter must validate its own exact connection
/// and serialize effect, activation, and abort under the C2 owner gate.
#[must_use = "abort or transfer this staged C1 Session gate into the owner transaction"]
pub struct C1PendingSessionEffectBinding {
    state: Arc<C1SessionEffectBindingState>,
    armed: bool,
}

/// A one-shot, read-only owner-admission gate for the pending phase. It holds
/// C1's exact Session lease while the outer owner performs its final
/// authorization checks and activates the binding. It never admits a
/// protected effect; callers must use `C1SessionEffectBindingGate` after
/// activation.
#[derive(Clone)]
pub struct C1SessionAdmissionGate {
    state: Arc<C1SessionEffectBindingState>,
}

/// Why a pending owner-admission callback could not be entered. `NotPending`
/// means the outer adapter may try the distinct active-effect gate; every
/// other result is a denial and must not be retried as an effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C1SessionAdmissionError {
    NotPending,
    Revoked,
    AdmissionAlreadyStarted,
    SessionMismatch,
    Session(ResolutionError),
}

/// Revalidation gate for one pending C1 Session binding. It denies while
/// pending and permanently after abort, and it still enters C1's checked lease
/// gate before every protected callback.
#[derive(Clone)]
pub struct C1SessionEffectBindingGate {
    state: Arc<C1SessionEffectBindingState>,
}

/// Nonblocking activation handle for a staged C1 Session effect gate. A root
/// adapter may call it only after its own exact peer and owner checks pass.
pub struct C1SessionEffectBindingActivation {
    state: Arc<C1SessionEffectBindingState>,
}

/// Idempotent, nonblocking revocation handle for a staged C1 Session effect
/// gate. It touches only an atomic state word and never re-enters C1 lifecycle
/// or C2 owner APIs.
pub struct C1SessionEffectBindingAbort {
    state: Arc<C1SessionEffectBindingState>,
}

impl C1PendingSessionEffectBinding {
    /// Provide parts for an outer transaction to hold until it commits or
    /// aborts. If the caller drops this object before transfer, its Drop
    /// revokes the still-pending gate.
    pub fn into_parts(
        mut self,
    ) -> (
        C1SessionEffectBindingGate,
        C1SessionEffectBindingActivation,
        C1SessionEffectBindingAbort,
    ) {
        self.armed = false;
        let state = Arc::clone(&self.state);
        (
            C1SessionEffectBindingGate {
                state: Arc::clone(&state),
            },
            C1SessionEffectBindingActivation {
                state: Arc::clone(&state),
            },
            C1SessionEffectBindingAbort { state },
        )
    }

    /// Observe the provisional gate. It remains unavailable until the
    /// activation handle is called.
    pub fn gate(&self) -> C1SessionEffectBindingGate {
        C1SessionEffectBindingGate {
            state: Arc::clone(&self.state),
        }
    }

    /// Obtain the separate pending-only gate for the outer owner's final
    /// read-only checks. The returned gate cannot admit a protected effect.
    pub fn admission_gate(&self) -> C1SessionAdmissionGate {
        C1SessionAdmissionGate {
            state: Arc::clone(&self.state),
        }
    }
}

impl Drop for C1PendingSessionEffectBinding {
    fn drop(&mut self) {
        if self.armed {
            self.state
                .phase
                .store(C1_SESSION_EFFECT_REVOKED, Ordering::Release);
        }
    }
}

impl C1SessionEffectBindingActivation {
    /// Activate exactly once, from inside this binding's pending-only
    /// admission callback while C1 still holds the exact Session lease. The
    /// caller must finish the outer owner's positive checks first; C1 does not
    /// validate peer ownership here.
    pub fn activate(&self) -> Result<(), ResolutionError> {
        if !self.state.preflight_started.load(Ordering::Acquire)
            || !C1LeaseGateScope::held_on_current_thread()
            || !C1SessionAdmissionScope::held_for(Arc::as_ptr(&self.state) as usize)
        {
            return Err(ResolutionError::Unavailable);
        }
        self.state
            .phase
            .compare_exchange(
                C1_SESSION_EFFECT_PENDING,
                C1_SESSION_EFFECT_ACTIVE,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map(|_| ())
            .map_err(|_| ResolutionError::Unavailable)
    }
}

impl Drop for C1SessionEffectBindingActivation {
    fn drop(&mut self) {
        self.state.revoke_if_pending();
    }
}

impl C1SessionEffectBindingAbort {
    /// Revoke immediately without waiting or acquiring either owner's gate.
    /// The outer owner must serialize this with callbacks admitted under its
    /// own connection gate; a callback already admitted by C1 may finish.
    pub fn abort(&self) {
        self.state
            .phase
            .store(C1_SESSION_EFFECT_REVOKED, Ordering::Release);
    }
}

impl Drop for C1SessionEffectBindingAbort {
    fn drop(&mut self) {
        self.state.revoke_if_pending();
    }
}

impl C1SessionEffectBindingGate {
    fn phase_is_active(&self) -> bool {
        self.state.phase.load(Ordering::Acquire) == C1_SESSION_EFFECT_ACTIVE
    }

    /// Admit a synchronous callback only while both the staged binding is
    /// active and its exact C1 Session lease remains live. The callback must
    /// not re-enter C1 lifecycle APIs or escape its current thread.
    pub fn with_live_session_effect<R>(
        &self,
        effect: impl FnOnce() -> R,
    ) -> Result<R, ResolutionError> {
        if !self.phase_is_active() {
            return Err(ResolutionError::Unavailable);
        }
        let mut effect = Some(effect);
        self.state
            .lease
            .with_live_session_effect(|| {
                if !self.phase_is_active() {
                    return Err(ResolutionError::Unavailable);
                }
                Ok(effect.take().map(|run| run()))
            })??
            .ok_or(ResolutionError::Unavailable)
    }

    /// C2 adapter variant that also matches the C2 Session selector to the
    /// C1-issued generation. This still proves only Session liveness; C2 root
    /// must validate the endpoint and trusted owner on both sides of the
    /// callback.
    #[cfg(feature = "c2-accepted-peer")]
    pub fn with_live_c2_peer_session_effect<R>(
        &self,
        selector: &nagi_peer_authenticated_connection::AuthenticatedInteractivePeerSelector,
        app_id: u64,
        app_session_id: u64,
        effect: impl FnOnce() -> R,
    ) -> Result<R, ResolutionError> {
        if !self.phase_is_active() {
            return Err(ResolutionError::Unavailable);
        }
        let mut effect = Some(effect);
        let admitted = self.state.lease.with_live_c2_peer_session_effect(
            selector,
            app_id,
            app_session_id,
            || {
                if !self.phase_is_active() {
                    return None;
                }
                effect.take().map(|run| run())
            },
        )?;
        admitted.ok_or(ResolutionError::Unavailable)
    }
}

impl C1SessionAdmissionGate {
    /// Run one synchronous, read-only outer-owner admission callback while the
    /// exact C1 Session lease is held. This is separate from the protected
    /// effect API: it is available only before activation and may be consumed
    /// once. The callback may activate this binding after completing the
    /// outer owner's final checks. It must not perform the protected effect,
    /// escape its thread, or re-enter C1 lifecycle APIs.
    pub fn with_live_session_admission<R>(
        &self,
        admission: impl FnOnce() -> R,
    ) -> Result<R, C1SessionAdmissionError> {
        match self.state.phase.load(Ordering::Acquire) {
            C1_SESSION_EFFECT_PENDING => {}
            C1_SESSION_EFFECT_ACTIVE => return Err(C1SessionAdmissionError::NotPending),
            C1_SESSION_EFFECT_REVOKED => return Err(C1SessionAdmissionError::Revoked),
            _ => return Err(C1SessionAdmissionError::Revoked),
        }

        let binding = Arc::as_ptr(&self.state) as usize;
        let mut admission = Some(admission);
        let admitted = self
            .state
            .lease
            .with_live_session_effect(|| {
                match self.state.phase.load(Ordering::Acquire) {
                    C1_SESSION_EFFECT_PENDING => {}
                    C1_SESSION_EFFECT_ACTIVE => return Err(C1SessionAdmissionError::NotPending),
                    C1_SESSION_EFFECT_REVOKED => return Err(C1SessionAdmissionError::Revoked),
                    _ => return Err(C1SessionAdmissionError::Revoked),
                }
                if self
                    .state
                    .preflight_started
                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    return Err(C1SessionAdmissionError::AdmissionAlreadyStarted);
                }
                let _scope = C1SessionAdmissionScope::enter(binding)?;
                let Some(admission) = admission.take() else {
                    return Err(C1SessionAdmissionError::AdmissionAlreadyStarted);
                };
                Ok(admission())
            })
            .map_err(C1SessionAdmissionError::Session)?;
        let result = admitted?;
        if self.state.phase.load(Ordering::Acquire) == C1_SESSION_EFFECT_REVOKED {
            return Err(C1SessionAdmissionError::Revoked);
        }
        Ok(result)
    }

    /// C2 adapter variant that binds the read-only admission callback to the
    /// selector's exact C1 Session generation as well as its app/session pair.
    /// This comparison is not peer authentication; C2 root must still hold
    /// and validate its exact accepted-connection owner around the callback.
    #[cfg(feature = "c2-accepted-peer")]
    pub fn with_live_c2_peer_session_admission<R>(
        &self,
        selector: &nagi_peer_authenticated_connection::AuthenticatedInteractivePeerSelector,
        app_id: u64,
        app_session_id: u64,
        admission: impl FnOnce() -> R,
    ) -> Result<R, C1SessionAdmissionError> {
        if !selector.matches_session(app_id, app_session_id, self.state.lease.generation()) {
            return Err(C1SessionAdmissionError::SessionMismatch);
        }
        self.with_live_session_admission(admission)
    }
}

/// Current process-local Session lease state. The generation is never
/// persisted or reconstructed from a Session record; it changes for each
/// issued lease. A host restart invalidates all contexts from the old process.
struct SessionLeaseState {
    generation: NonZeroU64,
    revision: AtomicU64,
    live: RwLock<bool>,
}

impl SessionLeaseState {
    fn new(generation: NonZeroU64) -> Self {
        Self {
            generation,
            revision: AtomicU64::new(generation.get()),
            live: RwLock::new(true),
        }
    }

    fn revoke(&self, live: &mut bool) {
        *live = false;
        self.revision.store(0, Ordering::Release);
    }
}

static NEXT_SESSION_LEASE_GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_session_lease_generation() -> Result<NonZeroU64, IdentityError> {
    NEXT_SESSION_LEASE_GENERATION
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current.checked_add(1)
        })
        .ok()
        .and_then(NonZeroU64::new)
        .ok_or(IdentityError::StoreUnavailable)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LiveStatus {
    Live,
    Revoked,
    Expired,
}

/// Non-authoritative observation tied to one process-local Session lease.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LiveSnapshot {
    generation: NonZeroU64,
    status: LiveStatus,
}

impl LiveSnapshot {
    pub const fn generation(self) -> NonZeroU64 {
        self.generation
    }

    pub const fn status(self) -> LiveStatus {
        self.status
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TryLiveError {
    Clock(TryClockError),
    StateUnavailable,
}

/// Invalidates observations on drop while allowing active leases to retain
/// only the admission state and defer destruction of the snapshot store.
struct IssuerGuard {
    alive: Arc<AtomicBool>,
    admission: Arc<IssuerAdmission>,
}

/// Opaque identity for one live C1 issuer instance. Staged login state keeps
/// this token so a later commit cannot be redirected to another IdentityService
/// that happens to contain the same User/Profile IDs.
#[cfg(all(feature = "linux-interactive-account", target_os = "linux"))]
#[derive(Clone)]
pub(crate) struct IdentityIssuerBinding(Arc<IssuerAdmission>);

/// A user supplied identity component whose destructor may panic. During an
/// existing unwind, contain a second destructor panic and forget its payload
/// just as the deferred store path does. This does not suppress the first
/// destructor panic during ordinary drop. `panic=abort` remains outside this
/// containment contract.
struct PanicContainedDrop<T>(Option<T>);

impl<T> PanicContainedDrop<T> {
    fn new(value: T) -> Self {
        Self(Some(value))
    }

    fn into_inner(mut self) -> T {
        self.0
            .take()
            .expect("component is present until extracted exactly once")
    }
}

impl<T> Deref for PanicContainedDrop<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.0
            .as_ref()
            .expect("component remains present while the issuer is usable")
    }
}

impl<T> DerefMut for PanicContainedDrop<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
            .as_mut()
            .expect("component remains present while the issuer is usable")
    }
}

impl<T> Drop for PanicContainedDrop<T> {
    fn drop(&mut self) {
        if let Some(component) = self.0.take() {
            drop_issuer_owned_value(component);
        }
    }
}

#[derive(Default)]
struct IssuerAdmissionState {
    closed: bool,
    recovering: bool,
    active: usize,
    deferred_stores: Vec<Box<dyn Any + Send>>,
    storage_adapter: Option<Weak<()>>,
}

/// Serializes Session-use admission with issuer shutdown. The store is kept in
/// `deferred_stores` until recovery and all admitted Session uses finish.
struct IssuerAdmission {
    state: Mutex<IssuerAdmissionState>,
    changed: Condvar,
}

/// A store destructor is user-provided trait code. If it panics while another
/// callback panic is unwinding, catch the cleanup panic so the process does
/// not abort from a double panic. Forget the caught panic payload: its own
/// destructor is user-provided too and could panic during the original unwind.
/// The issuer has already been closed; a partially completed destructor
/// cannot restore authority. This protection requires panic=unwind; with
/// panic=abort, Rust terminates at the first panic and catch_unwind cannot
/// intercept it.
fn drop_issuer_owned_value<T>(value: T) {
    if std::thread::panicking() {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(value))) {
            std::mem::forget(payload);
        }
    } else {
        drop(value);
    }
}

impl IssuerAdmission {
    fn new() -> Self {
        Self {
            state: Mutex::new(IssuerAdmissionState::default()),
            changed: Condvar::new(),
        }
    }

    fn enter(self: &Arc<Self>) -> Result<IssuerActivityGuard, ResolutionError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| ResolutionError::Unavailable)?;
        if state.closed || state.recovering {
            return Err(ResolutionError::Unavailable);
        }
        state.active = state
            .active
            .checked_add(1)
            .ok_or(ResolutionError::Unavailable)?;
        Ok(IssuerActivityGuard(Arc::clone(self)))
    }

    fn try_enter(self: &Arc<Self>) -> Result<Option<IssuerActivityGuard>, TryLiveError> {
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(TryLiveError::StateUnavailable);
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(TryLiveError::StateUnavailable);
            }
        };
        if state.closed {
            return Ok(None);
        }
        if state.recovering {
            return Err(TryLiveError::StateUnavailable);
        }
        state.active = state
            .active
            .checked_add(1)
            .ok_or(TryLiveError::StateUnavailable)?;
        Ok(Some(IssuerActivityGuard(Arc::clone(self))))
    }

    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn begin_recovery(self: &Arc<Self>) -> Result<IssuerRecoveryAdmissionGuard, IdentityError> {
        #[cfg(test)]
        RECOVERY_WRITE_GUARDS_ACQUIRED.with(|count| count.set(0));
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(IdentityError::SessionLeaseBusy)
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(IdentityError::StoreUnavailable)
            }
        };
        if state.closed {
            return Err(IdentityError::StoreUnavailable);
        }
        if state.recovering {
            return Err(IdentityError::SessionLeaseBusy);
        }
        // Activity admission and recovery reservation share this mutex. An
        // effect that incremented `active` but has not taken its Session read
        // gate yet must therefore block recovery before durable intent. Once
        // this reservation succeeds, no new activity can enter.
        if state.active != 0 {
            return Err(IdentityError::SessionLeaseBusy);
        }
        state.recovering = true;
        Ok(IssuerRecoveryAdmissionGuard {
            admission: Arc::clone(self),
            preserve_closed: false,
        })
    }

    /// Reserve the issuer for an exact terminal reconciliation. A closed
    /// issuer may perform this store-only terminal operation, but remains
    /// closed for the entire reservation and afterward. This lets a
    /// post-claim failure be reconciled without reopening any old identity.
    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn begin_terminal_reconciliation(
        self: &Arc<Self>,
    ) -> Result<IssuerRecoveryAdmissionGuard, IdentityError> {
        #[cfg(test)]
        RECOVERY_WRITE_GUARDS_ACQUIRED.with(|count| count.set(0));
        let mut state = match self.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(IdentityError::SessionLeaseBusy)
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(IdentityError::StoreUnavailable)
            }
        };
        if state.recovering || state.active != 0 {
            return Err(IdentityError::SessionLeaseBusy);
        }
        let preserve_closed = state.closed;
        state.recovering = true;
        Ok(IssuerRecoveryAdmissionGuard {
            admission: Arc::clone(self),
            preserve_closed,
        })
    }

    fn close(&self, alive: &AtomicBool) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        alive.store(false, Ordering::Release);
        self.changed.notify_all();
        let deferred_stores = Self::take_deferred_stores_if_idle(&mut state);
        drop(state);
        drop_issuer_owned_value(deferred_stores);
    }

    fn close_if_idle(&self, alive: &AtomicBool) -> bool {
        let Ok(mut state) = self.state.try_lock() else {
            return false;
        };
        if state.active != 0 || state.recovering {
            return false;
        }
        state.closed = true;
        alive.store(false, Ordering::Release);
        self.changed.notify_all();
        true
    }

    fn defer_store<S: Any + Send>(&self, store: S) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.active != 0 || state.recovering {
            state.deferred_stores.push(Box::new(store));
            return;
        }
        drop(state);
        drop_issuer_owned_value(store);
    }

    fn take_deferred_stores_if_idle(state: &mut IssuerAdmissionState) -> Vec<Box<dyn Any + Send>> {
        if state.closed && state.active == 0 && !state.recovering {
            std::mem::take(&mut state.deferred_stores)
        } else {
            Vec::new()
        }
    }

    #[cfg(test)]
    fn wait_until_closed_active(&self, expected: usize) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut state = self.state.lock().unwrap();
        while !(state.closed && state.active == expected) {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(
                !remaining.is_zero(),
                "issuer shutdown did not reach expected gate state"
            );
            let (next, timeout) = self.changed.wait_timeout(state, remaining).unwrap();
            state = next;
            assert!(!timeout.timed_out() || (state.closed && state.active == expected));
        }
    }
}

struct IssuerActivityGuard(Arc<IssuerAdmission>);

impl Drop for IssuerActivityGuard {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active = state.active.saturating_sub(1);
        self.0.changed.notify_all();
        let deferred_stores = IssuerAdmission::take_deferred_stores_if_idle(&mut state);
        drop(state);
        drop_issuer_owned_value(deferred_stores);
    }
}

#[cfg(any(test, feature = "system-service-recovery-contract"))]
pub(crate) struct IssuerRecoveryAdmissionGuard {
    admission: Arc<IssuerAdmission>,
    preserve_closed: bool,
}

#[cfg(any(test, feature = "system-service-recovery-contract"))]
impl IssuerRecoveryAdmissionGuard {
    pub(crate) fn is_permanently_closed(&self) -> bool {
        self.preserve_closed
    }

    fn close_permanently(&mut self, alive: &AtomicBool) -> Result<(), IdentityError> {
        // Once a durable terminal transition is claimed, even an unexpected
        // admission-state invariant failure must not let Drop reopen the old
        // issuer. Recover poisoned state only to publish the fail-closed bit.
        self.preserve_closed = true;
        let mut state = self
            .admission
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // Terminal reconciliation may start from an already-closed issuer.
        // It can finish the durable terminal write, but must never reopen it.
        let was_ready = state.recovering && state.active == 0;
        state.closed = true;
        state.recovering = false;
        alive.store(false, Ordering::Release);
        self.admission.changed.notify_all();
        let deferred_stores = IssuerAdmission::take_deferred_stores_if_idle(&mut state);
        drop(state);
        drop_issuer_owned_value(deferred_stores);
        if was_ready {
            Ok(())
        } else {
            Err(IdentityError::StoreUnavailable)
        }
    }
}

#[cfg(any(test, feature = "system-service-recovery-contract"))]
impl Drop for IssuerRecoveryAdmissionGuard {
    fn drop(&mut self) {
        let mut state = self
            .admission
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.recovering = false;
        self.admission.changed.notify_all();
        let deferred_stores = IssuerAdmission::take_deferred_stores_if_idle(&mut state);
        drop(state);
        drop_issuer_owned_value(deferred_stores);
    }
}

impl IssuerGuard {
    fn new() -> Self {
        Self {
            alive: Arc::new(AtomicBool::new(true)),
            admission: Arc::new(IssuerAdmission::new()),
        }
    }

    fn accepts_storage_adapter(
        &self,
        adapter_identity: &Arc<()>,
    ) -> Result<bool, IssuerStorageAdapterCheckError> {
        let state = match self.admission.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(IssuerStorageAdapterCheckError::Busy)
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(IssuerStorageAdapterCheckError::Unavailable)
            }
        };
        Ok(state.storage_adapter.as_ref().is_none_or(|owner| {
            Weak::ptr_eq(owner, &Arc::downgrade(adapter_identity)) || owner.upgrade().is_none()
        }))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssuerStorageAdapterCheckError {
    Busy,
    Unavailable,
}

impl IssuerStorageAdapterCheckError {
    fn into_identity_error(self) -> IdentityError {
        match self {
            Self::Busy => IdentityError::SessionLeaseBusy,
            Self::Unavailable => IdentityError::StoreUnavailable,
        }
    }
}

impl Drop for IssuerGuard {
    fn drop(&mut self) {
        self.admission.close(&self.alive);
    }
}

/// Store ownership follows active issuer work after shutdown. The issuer
/// closes admission first (its field precedes this one), then this guard drops
/// the store immediately or transfers it to the final active/recovery guard.
struct DeferredSnapshotStore<S: IdentitySnapshotStore> {
    store: Option<S>,
    alive: Arc<AtomicBool>,
    admission: Arc<IssuerAdmission>,
}

impl<S: IdentitySnapshotStore> DeferredSnapshotStore<S> {
    fn new(store: S, issuer: &IssuerGuard) -> Self {
        Self {
            store: Some(store),
            alive: Arc::clone(&issuer.alive),
            admission: Arc::clone(&issuer.admission),
        }
    }

    /// Transfer the store wrapper to a fresh issuer only after recovery has
    /// permanently closed and drained the previous issuer. The old Session
    /// handles keep their old closed admission and cannot regain authority.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    fn rebind_from_closed_issuer(
        &mut self,
        previous: &IssuerGuard,
        next: &IssuerGuard,
    ) -> Result<(), IdentityError> {
        if !Arc::ptr_eq(&self.alive, &previous.alive)
            || !Arc::ptr_eq(&self.admission, &previous.admission)
        {
            return Err(IdentityError::StoreUnavailable);
        }
        let state = match previous.admission.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(_))
            | Err(std::sync::TryLockError::WouldBlock) => {
                return Err(IdentityError::StoreUnavailable)
            }
        };
        if !state.closed
            || state.recovering
            || state.active != 0
            || previous.alive.load(Ordering::Acquire)
        {
            return Err(IdentityError::StoreUnavailable);
        }
        drop(state);

        self.alive = Arc::clone(&next.alive);
        self.admission = Arc::clone(&next.admission);
        Ok(())
    }

    fn into_inner(mut self) -> S {
        self.store
            .take()
            .expect("snapshot store is present until extracted exactly once")
    }
}

impl<S: IdentitySnapshotStore> Deref for DeferredSnapshotStore<S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        self.store
            .as_ref()
            .expect("snapshot store remains present while the issuer is usable")
    }
}

impl<S: IdentitySnapshotStore> DerefMut for DeferredSnapshotStore<S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.store
            .as_mut()
            .expect("snapshot store remains present while the issuer is usable")
    }
}

impl<S: IdentitySnapshotStore> Drop for DeferredSnapshotStore<S> {
    fn drop(&mut self) {
        let Some(store) = self.store.take() else {
            return;
        };
        self.admission.close(&self.alive);
        self.admission.defer_store(store);
    }
}

/// Failure to admit an operation through an interactive caller/session pair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InteractiveSessionError {
    CallerMismatch,
    Identity(ResolutionError),
    Live(TryLiveError),
}

/// A borrowed caller paired with a service-issued identity observation.
///
/// This context is local to the trusted caller and must be revalidated at each
/// use. It is not a token, serializer, or shared-thread authority. The C1
/// service creates it only after resolving the caller's current Session.
///
/// ```compile_fail,E0277
/// use nagi_identity_session::InteractiveSessionContext;
/// fn require_send_sync<T: Send + Sync>() {}
/// require_send_sync::<InteractiveSessionContext<'static>>();
/// ```
///
/// ```compile_fail,E0277
/// use nagi_identity_session::InteractiveSessionContext;
/// fn require_serialize<T: serde::Serialize>() {}
/// require_serialize::<InteractiveSessionContext<'static>>();
/// ```
pub struct InteractiveSessionContext<'caller> {
    identity: ResolvedIdentity,
    caller: &'caller dyn AuthenticatedInteractiveCaller,
    principal: PrincipalId,
    caller_context: CallerContextId,
}

impl<'caller> InteractiveSessionContext<'caller> {
    fn attach(
        identity: ResolvedIdentity,
        caller: &'caller dyn AuthenticatedInteractiveCaller,
    ) -> Result<Self, InteractiveSessionError> {
        if identity.principal_id() != caller.principal_id()
            || identity.caller_context_id() != caller.context_id()
        {
            return Err(InteractiveSessionError::CallerMismatch);
        }
        identity
            .check_live()
            .map_err(InteractiveSessionError::Identity)?;
        Ok(Self {
            principal: identity.principal_id().clone(),
            caller_context: identity.caller_context_id().clone(),
            identity,
            caller,
        })
    }

    /// Re-check the exact trusted caller and the real current Session state.
    pub fn check(&self) -> Result<(), InteractiveSessionError> {
        if self.caller.principal_id() != &self.principal
            || self.caller.context_id() != &self.caller_context
        {
            return Err(InteractiveSessionError::CallerMismatch);
        }
        self.identity
            .check_live()
            .map_err(InteractiveSessionError::Identity)
    }

    /// Nonblocking status observation for connection maintenance. It is not
    /// sufficient to admit an operation; use `check` before protected effects.
    pub fn try_live_snapshot(&self) -> Result<LiveSnapshot, InteractiveSessionError> {
        if self.caller.principal_id() != &self.principal
            || self.caller.context_id() != &self.caller_context
        {
            return Err(InteractiveSessionError::CallerMismatch);
        }
        self.identity
            .try_live_snapshot()
            .map_err(InteractiveSessionError::Live)
    }

    /// Return the captured C1 identity only while the paired context is live.
    pub fn identity(&self) -> Result<&ResolvedIdentity, InteractiveSessionError> {
        self.check()?;
        Ok(&self.identity)
    }

    /// Create an owned revalidation handle for a trusted connection owner.
    /// The exact caller association and current durable Session are checked
    /// before the handle is detached from this borrowed context. A consumer
    /// must bind the handle to the authenticated connection that supplied this
    /// context; identifiers alone cannot reconstruct one.
    pub fn revalidation_handle(
        &self,
    ) -> Result<SessionLeaseRevalidationHandle, InteractiveSessionError> {
        self.check()?;
        Ok(SessionLeaseRevalidationHandle {
            identity: self.identity.duplicate_observation(),
        })
    }

    /// Stage a C1 Session-only effect gate for an outer owner transaction.
    /// The resulting gate is unavailable until the outer owner calls its
    /// activation handle and remains revoked after abort. This does not
    /// authenticate a peer, enroll an owner key, issue a caller seal, or
    /// validate C2 endpoint state.
    pub fn stage_session_effect_binding(
        &self,
    ) -> Result<C1PendingSessionEffectBinding, InteractiveSessionError> {
        let lease = self.revalidation_handle()?;
        Ok(C1PendingSessionEffectBinding {
            state: Arc::new(C1SessionEffectBindingState {
                phase: AtomicU8::new(C1_SESSION_EFFECT_PENDING),
                preflight_started: AtomicBool::new(false),
                lease,
            }),
            armed: true,
        })
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.principal
    }

    pub fn caller_context_id(&self) -> &CallerContextId {
        &self.caller_context
    }

    /// End-owner adapters use this instance binding before invoking a
    /// lifecycle method. Matching SessionId strings across independent stores
    /// cannot authorize an end through the wrong service.
    pub fn is_issued_by<S: IdentitySnapshotStore, I, C, A, G, E>(
        &self,
        service: &IdentityService<S, I, C, A, G, E>,
    ) -> bool {
        Arc::ptr_eq(&self.identity.issuer_alive, &service.issuer.alive)
    }
}

/// The service-issued identity paired with the trusted caller presented for a
/// storage operation. Storage adapters re-check the pair on every operation.
pub struct IdentityAccessContext<'a> {
    pub(crate) identity: &'a ResolvedIdentity,
    pub(crate) caller: &'a dyn AuthenticatedInteractiveCaller,
}

impl<'a> IdentityAccessContext<'a> {
    pub fn new(
        identity: &'a ResolvedIdentity,
        caller: &'a impl AuthenticatedInteractiveCaller,
    ) -> Self {
        Self { identity, caller }
    }
}

pub trait CurrentIdentityResolver {
    fn resolve_current_identity(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
    ) -> Result<ResolvedIdentity, ResolutionError>;
}

/// Provider-neutral lookup seam. Local identity remains the stable UserId;
/// federation implementations may map an external reference to it later.
pub trait IdentityProvider {
    fn resolve_subject(
        &self,
        subject: &ProviderSubjectRef,
    ) -> Result<Option<UserId>, ProviderLookupFailure>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderLookupFailure {
    Unavailable,
    Unmapped,
}

/// Opaque C1 evidence that the exact operation, binding, and owner outcome
/// reached durable Reconciled state. It has no public constructor, is not an
/// authority grant, and cannot reopen the issuer fenced for terminal writing.
#[must_use]
#[derive(Debug, Eq, PartialEq)]
pub struct C1SystemServiceRecoveryTerminalReceipt {
    operation_id: crate::store::RecoveryOperationId,
    binding: RecoveryBinding,
    outcome_digest: [u8; 32],
}

impl C1SystemServiceRecoveryTerminalReceipt {
    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn from_reconciliation(reconciliation: &RecoveryReconciliation) -> Self {
        Self {
            operation_id: reconciliation.operation_id(),
            binding: reconciliation.binding().clone(),
            outcome_digest: reconciliation.outcome_digest(),
        }
    }

    pub const fn operation_id(&self) -> u64 {
        self.operation_id.get()
    }

    /// Opaque C1 recovery binding digest; this is consistency metadata and
    /// carries no authority by itself.
    pub const fn binding_digest(&self) -> &[u8; 32] {
        self.binding.digest_bytes()
    }

    pub const fn outcome_digest(&self) -> [u8; 32] {
        self.outcome_digest
    }
}

/// C1's owned caller identity copied from the shared recovery contract only
/// while its live binding is borrowed inside C3's exact endpoint gate. This
/// records the canonical service and generation tuple for later scope
/// comparison; it does not keep the C3 gate open or prove current liveness.
/// The integrating A owner must compare the binding to its exact admission
/// before issuing this caller, then use its live C3 gate for every recovery
/// operation.
#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
#[derive(Debug)]
pub struct C1VerifiedBootSystemServicePrincipal {
    service_id: String,
    principal_id: PrincipalId,
    caller_context_id: CallerContextId,
    mapping_generation: NonZeroU64,
    catalog_generation: NonZeroU64,
    c3_launch_generation: NonZeroU64,
}

#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C1SystemServicePrincipalIssueError {
    InvalidServiceId,
    InvalidPrincipalId,
    InvalidCallerContextId,
}

#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
impl C1VerifiedBootSystemServicePrincipal {
    /// Copy the authenticated mapping while the shared contract's
    /// higher-ranked callback holds the exact C3 process/endpoint gate. The
    /// binding itself cannot escape that callback; only the owned identifiers
    /// and generations are retained here.
    pub fn from_live_binding(
        binding: &nagi_system_service_recovery_contract::LiveBootSystemServiceBinding<'_>,
    ) -> Result<Self, C1SystemServicePrincipalIssueError> {
        let service_id = binding.service_id();
        if service_id.is_empty() {
            return Err(C1SystemServicePrincipalIssueError::InvalidServiceId);
        }

        let principal_id = PrincipalId::new(binding.principal_id().to_owned())
            .map_err(|_| C1SystemServicePrincipalIssueError::InvalidPrincipalId)?;
        if !principal_id.as_str().starts_with("service:") {
            return Err(C1SystemServicePrincipalIssueError::InvalidPrincipalId);
        }

        let caller_context_id = CallerContextId::new(binding.caller_context_id().to_owned())
            .map_err(|_| C1SystemServicePrincipalIssueError::InvalidCallerContextId)?;

        Ok(Self {
            service_id: service_id.to_owned(),
            principal_id,
            caller_context_id,
            mapping_generation: binding.mapping_generation(),
            catalog_generation: binding.catalog_generation(),
            c3_launch_generation: binding.c3_launch_generation(),
        })
    }

    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    pub fn mapping_generation(&self) -> NonZeroU64 {
        self.mapping_generation
    }

    pub fn catalog_generation(&self) -> NonZeroU64 {
        self.catalog_generation
    }

    pub fn c3_launch_generation(&self) -> NonZeroU64 {
        self.c3_launch_generation
    }
}

#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
impl TrustedCallerContext for C1VerifiedBootSystemServicePrincipal {
    fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    fn context_id(&self) -> &CallerContextId {
        &self.caller_context_id
    }
}

#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
impl caller_proof_seal::SystemService for C1VerifiedBootSystemServicePrincipal {}

#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
impl AuthenticatedSystemServicePrincipal for C1VerifiedBootSystemServicePrincipal {}

/// C1's private bridge from one already-authenticated SystemService caller and
/// the canonical durable identity journal to A's cycle-free receipt client.
/// Production construction requires C1's caller copied from the shared live
/// boot binding. The crate-private constructor remains available only to C1's
/// own tests; public scope metadata cannot create a caller proof or operation
/// permit.
#[cfg(feature = "system-service-recovery-contract")]
pub struct C1SystemServiceRecoveryReceiptAdapter<
    'issuer,
    'caller,
    S: IdentitySnapshotStore,
    I,
    C,
    A,
    G,
    E = NoopIdentityEventSink,
> {
    identity: &'issuer mut IdentityService<S, I, C, A, G, E>,
    caller: &'caller dyn AuthenticatedSystemServicePrincipal,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    boot_binding: Option<&'caller C1VerifiedBootSystemServicePrincipal>,
}

/// C1's half of a successful system-service runtime handoff. This non-cloneable
/// token is created only after C1 reads back the exact `RuntimeActive` journal
/// row and replaces the permanently closed recovery issuer with a fresh issuer
/// for the recovered service. It is not C3 liveness or authorization evidence;
/// the integrating A runtime must retain its exact C3 proof, lifecycle, and
/// package owner alongside this token.
#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
#[must_use = "retain this C1 handoff with the matching live C3 runtime"]
pub struct C1SystemServiceRuntimeHandoff {
    operation: C1RecoveryOperationIdentity,
    report: RecoveryReport,
    issuer: Arc<IssuerAdmission>,
}

/// C1's short-lived admission for one live effect owned by the recovered
/// system-service runtime. It retains the exact scope and durable C1 operation
/// identity together with an active issuer guard. A must hold it across the
/// matching C3 live-effect gate; it is not C3 proof or a substitute for the
/// owner's durable `Live` receipt.
#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
#[must_use = "hold the C1 runtime admission across the matching C3 live-effect gate"]
pub struct C1SystemServiceRuntimeAdmission<'runtime> {
    handoff: &'runtime C1SystemServiceRuntimeHandoff,
    scope: &'runtime dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
    _issuer_activity: IssuerActivityGuard,
}

#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
impl C1SystemServiceRuntimeAdmission<'_> {
    /// The exact A-owned scope captured by this C1 admission. Pass this same
    /// reference to the C3 live-effect gate.
    pub fn scope(
        &self,
    ) -> &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope {
        self.scope
    }

    /// Correlation for the C3 durable live receipt. This value grants no
    /// authority; C3 must still validate the retained proof and exact slot.
    pub const fn operation_identity(&self) -> C1RecoveryOperationIdentity {
        self.handoff.operation_identity()
    }

    pub(crate) fn matches_boot_principal_binding(
        &self,
        service_id: &str,
        principal_id: &PrincipalId,
        caller_context_id: &CallerContextId,
        mapping_generation: NonZeroU64,
        catalog_generation: NonZeroU64,
        c3_launch_generation: NonZeroU64,
    ) -> bool {
        let scope = OwnedRecoveryScopeSnapshot::capture(self.scope);
        scope.service_id == service_id
            && scope.principal_id == principal_id.as_str()
            && scope.caller_context_id == caller_context_id.as_str()
            && scope.mapping_generation == mapping_generation
            && scope.role == "system_service"
            && scope.catalog_generation == catalog_generation
            && scope.c3_launch_generation == c3_launch_generation
            && scope.canonical_binding_digest() == scope.binding_digest
            && self.operation_identity().binding_digest() == &scope.binding_digest
    }
}

#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
impl C1SystemServiceRuntimeHandoff {
    /// The durable C1 operation that reached `RuntimeActive`. This value is
    /// correlation only and grants no C3 or Session authority.
    pub const fn operation_identity(&self) -> C1RecoveryOperationIdentity {
        self.operation
    }

    pub fn recovery_report(&self) -> &RecoveryReport {
        &self.report
    }
}

#[cfg(feature = "system-service-recovery-contract")]
impl<'issuer, 'caller, S: IdentitySnapshotStore, I, C, A, G, E>
    C1SystemServiceRecoveryReceiptAdapter<'issuer, 'caller, S, I, C, A, G, E>
{
    #[cfg(test)]
    pub(crate) fn new(
        identity: &'issuer mut IdentityService<S, I, C, A, G, E>,
        caller: &'caller dyn AuthenticatedSystemServicePrincipal,
    ) -> Self {
        Self {
            identity,
            caller,
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            boot_binding: None,
        }
    }

    /// Construct A's C1 receipt client from the C1-private principal emitted
    /// by [`C1VerifiedBootSystemServicePrincipal::from_live_binding`]. Every
    /// current-launch operation validates its scope against that principal.
    /// Prior-launch rows use the separate
    /// `reconcile_previous_launch_terminal` entry point and still require A's
    /// actual serialized C3 owner gate.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub fn new_for_verified_boot_principal(
        identity: &'issuer mut IdentityService<S, I, C, A, G, E>,
        caller: &'caller C1VerifiedBootSystemServicePrincipal,
    ) -> Self {
        Self {
            identity,
            caller,
            boot_binding: Some(caller),
        }
    }

    /// Reconcile an exact C1 operation from a prior C3 launch during a new
    /// boot. `current_scope` must match this adapter's current verified boot
    /// principal exactly. The C3 owner gate supplies the previous scope from
    /// its durable row inside the gate callback; callers cannot synthesize it
    /// from the current boot or C1's digest-only journal.
    ///
    /// C1 supplies the exact durable operation correlation to `gate`. The
    /// dedicated C3 owner gate must return a durable terminal slot receipt
    /// whose C1 operation ID, complete prior binding digest, service, catalog
    /// and launch generations, and cleared/absent state all match the
    /// owner-supplied previous scope. Scope metadata alone never clears the
    /// journal. The shared C3 receipt does not carry the old process identity
    /// or normal exit status; if the caller's terminal policy requires those
    /// facts and C3 cannot prove them after a cold reboot, the gate must return
    /// `OutcomeUnknown`. In that case C1 leaves the journal and quarantine in
    /// place. This entry point is terminal reconciliation only; new recovery
    /// intents continue to require the exact current scope.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub fn reconcile_previous_launch_terminal<Gate>(
        &mut self,
        current_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        gate: &mut Gate,
    ) -> Result<
        C1SystemServiceRecoveryTerminalReceipt,
        nagi_system_service_recovery_contract::RecoveryClientError,
    >
    where
        S: C1SystemServiceRecoveryJournal,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
        Gate: C1PreviousLaunchTerminalGate,
    {
        self.reconcile_previous_launch_terminal_inner(current_scope, false, |claim, validate| {
            let (scope, receipt) =
                gate.with_previous_launch_reconciliation_gate(current_scope, claim, |scope| {
                    validate(scope)
                })?;
            if previous_launch_terminal_receipt_matches(&receipt, &scope, claim) {
                Ok(())
            } else {
                Err(nagi_system_service_recovery_contract::RecoveryClientError::Rejected)
            }
        })
    }

    /// Close the exact C1 operation that was active in a different boot after
    /// C3 durably records `PriorBootCleared` for its old launch. This path
    /// accepts only a prior `RuntimeActive` journal row and an owner receipt
    /// whose previous/current boot IDs are both valid and different. It does
    /// not resume the old Session; the caller must consume the exact terminal
    /// receipt, drop this issuer, reopen the same store, and only then create
    /// the next runtime intent.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub fn reconcile_previous_boot_terminal<Gate>(
        &mut self,
        current_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        gate: &mut Gate,
    ) -> Result<
        C1SystemServiceRecoveryTerminalReceipt,
        nagi_system_service_recovery_contract::RecoveryClientError,
    >
    where
        S: C1SystemServiceRecoveryJournal,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
        Gate: C1PreviousBootTerminalGate,
    {
        self.reconcile_previous_launch_terminal_inner(current_scope, true, |claim, validate| {
            let (scope, receipt) =
                gate.with_previous_boot_reconciliation_gate(current_scope, claim, |scope| {
                    validate(scope)
                })?;
            if previous_boot_terminal_receipt_matches(&receipt, &scope, claim) {
                Ok(())
            } else {
                Err(nagi_system_service_recovery_contract::RecoveryClientError::Rejected)
            }
        })
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn reconcile_previous_launch_terminal_inner(
        &mut self,
        current_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        prior_boot: bool,
        confirm: impl FnOnce(
            nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
            &mut PreviousScopeValidator<'_>,
        )
            -> Result<(), nagi_system_service_recovery_contract::RecoveryClientError>,
    ) -> Result<
        C1SystemServiceRecoveryTerminalReceipt,
        nagi_system_service_recovery_contract::RecoveryClientError,
    >
    where
        S: C1SystemServiceRecoveryJournal,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let caller = self.boot_binding.ok_or(RecoveryClientError::Rejected)?;
        let current_snapshot = OwnedRecoveryScopeSnapshot::capture(current_scope);
        if !current_snapshot.matches_boot_principal(caller)
            || current_snapshot.canonical_binding_digest() != current_snapshot.binding_digest
        {
            return Err(RecoveryClientError::Rejected);
        }

        let caller_principal = self.caller.principal_id().clone();
        let caller_context = self.caller.context_id().clone();
        current_snapshot
            .validate_caller(&caller_principal, &caller_context)
            .map_err(|_| RecoveryClientError::Rejected)?;
        self.identity
            .authorize_recovery_with_captured_caller(caller_principal, caller_context)
            .map_err(recovery_client_error)?;

        self.reconcile_previous_launch_after_scope_preflight(
            current_scope,
            current_snapshot,
            caller,
            prior_boot,
            confirm,
        )
    }

    /// Consume a previous-launch terminal receipt by exact durable C1
    /// readback. The receipt was created only after the C3 owner gate matched
    /// the old scope to C1's operation row; no caller-supplied old scope is
    /// needed to consume the C1 receipt. This verifies the existing terminal
    /// row and does not start a recovery intent.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub fn consume_previous_launch_terminal_receipt(
        &mut self,
        receipt: C1SystemServiceRecoveryTerminalReceipt,
    ) -> Result<(), nagi_system_service_recovery_contract::RecoveryClientError>
    where
        S: C1SystemServiceRecoveryJournal,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        if self.boot_binding.is_none() {
            return Err(RecoveryClientError::Rejected);
        }
        self.identity
            .consume_reconciliation_receipt(receipt)
            .map_err(recovery_client_error)
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn reconcile_previous_launch_after_scope_preflight(
        &mut self,
        current_boot_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        current_boot_snapshot: OwnedRecoveryScopeSnapshot,
        caller: &C1VerifiedBootSystemServicePrincipal,
        prior_boot: bool,
        confirm: impl FnOnce(
            nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
            &mut PreviousScopeValidator<'_>,
        )
            -> Result<(), nagi_system_service_recovery_contract::RecoveryClientError>,
    ) -> Result<
        C1SystemServiceRecoveryTerminalReceipt,
        nagi_system_service_recovery_contract::RecoveryClientError,
    >
    where
        S: RecoveryJournalStore,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
    {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let record = self
            .identity
            .store
            .recovery_journal_record()
            .map_err(map_recovery_journal_error)
            .map_err(recovery_client_error)?
            .ok_or(RecoveryClientError::Rejected)?;
        let outcome_digest = match prior_boot {
            false => match &record.phase {
                RecoveryJournalPhase::Reconciled { outcome_digest } => {
                    if *outcome_digest
                        == system_service_prior_boot_outcome_digest(
                            record.operation_id,
                            &record.binding,
                        )
                    {
                        return Err(RecoveryClientError::Rejected);
                    }
                    *outcome_digest
                }
                RecoveryJournalPhase::Intent
                | RecoveryJournalPhase::EffectStarted
                | RecoveryJournalPhase::OutcomeUnknown
                | RecoveryJournalPhase::RuntimeActive => {
                    system_service_terminal_outcome_digest(record.operation_id, &record.binding)
                }
            },
            true => {
                let reboot_digest =
                    system_service_prior_boot_outcome_digest(record.operation_id, &record.binding);
                match &record.phase {
                    RecoveryJournalPhase::RuntimeActive => reboot_digest,
                    RecoveryJournalPhase::Reconciled { outcome_digest }
                        if *outcome_digest == reboot_digest =>
                    {
                        reboot_digest
                    }
                    _ => return Err(RecoveryClientError::Rejected),
                }
            }
        };
        let reconciliation = RecoveryReconciliation::from_journal_record(&record, outcome_digest);
        let claim = nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation::new(
            NonZeroU64::new(reconciliation.operation_id().get())
                .ok_or(RecoveryClientError::Rejected)?,
            *reconciliation.binding().digest_bytes(),
        );
        let (mut admission, store_permit) = self
            .identity
            .prepare_recovery_reconciliation(reconciliation.clone())
            .map_err(recovery_client_error)?;

        let caller_principal = self.caller.principal_id().clone();
        let caller_context = self.caller.context_id().clone();
        let mut callback_error = None;
        let mut callback_validated = false;
        let gate_result = confirm(claim, &mut |operation_scope| {
            let operation_snapshot = OwnedRecoveryScopeSnapshot::capture(operation_scope);
            let scope_valid = current_boot_snapshot.matches_boot_principal(caller)
                && operation_snapshot.matches_previous_launch_principal(caller)
                && operation_snapshot.canonical_binding_digest()
                    == operation_snapshot.binding_digest
                && operation_snapshot
                    .validate_caller(&caller_principal, &caller_context)
                    .is_ok()
                && operation_snapshot
                    .validate_operation_identity(record.operation_id, &record.binding)
                    .is_ok()
                && previous_launch_scope_snapshots_match(
                    current_boot_scope,
                    operation_scope,
                    &current_boot_snapshot,
                    &operation_snapshot,
                );
            if scope_valid {
                callback_validated = true;
                Ok(operation_snapshot)
            } else {
                callback_error = Some(RecoveryClientError::Rejected);
                Err(RecoveryClientError::Rejected)
            }
        });

        // The C3 receipt below is the durable re-read of the owner-supplied
        // previous scope. Re-read the current boot view independently after
        // the gate; a previous scope never comes from the current view.
        let current_scope_matches =
            OwnedRecoveryScopeSnapshot::capture(current_boot_scope) == current_boot_snapshot;
        if !current_scope_matches {
            return if matches!(&gate_result, Err(RecoveryClientError::OutcomeUnknown))
                || admission.is_permanently_closed()
            {
                Err(RecoveryClientError::OutcomeUnknown)
            } else {
                Err(RecoveryClientError::Rejected)
            };
        }
        match gate_result {
            Ok(()) => {}
            Err(_) if admission.is_permanently_closed() => {
                return Err(RecoveryClientError::OutcomeUnknown);
            }
            Err(error) => return Err(error),
        }
        if !callback_validated || callback_error.is_some() {
            return Err(RecoveryClientError::Rejected);
        }

        // The pending C1 journal row remains quarantining until C3 has
        // returned its exact durable terminal confirmation. If C3 returns
        // OutcomeUnknown after invoking the callback, no C1 terminal write
        // has happened and a newly opened store will still see the old row.
        match self.identity.commit_prepared_recovery_reconciliation(
            &mut admission,
            store_permit,
            reconciliation,
        ) {
            Ok(receipt) => Ok(receipt),
            // Terminal persistence starts only after this issuer is
            // permanently closed. Any error from that point may follow a
            // durable rename, so callers must reopen and exact-read the row
            // instead of treating it as an ordinary failure or retrying on
            // this issuer.
            Err(_error) if admission.is_permanently_closed() => {
                Err(RecoveryClientError::OutcomeUnknown)
            }
            Err(error) => Err(recovery_client_error(error)),
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    fn reconcile_terminal_after_scope_preflight<Gate>(
        &mut self,
        operation_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        operation_snapshot: OwnedRecoveryScopeSnapshot,
        current_boot_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        current_boot_snapshot: OwnedRecoveryScopeSnapshot,
        gate: &mut Gate,
    ) -> Result<
        C1SystemServiceRecoveryTerminalReceipt,
        nagi_system_service_recovery_contract::RecoveryClientError,
    >
    where
        S: RecoveryJournalStore,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
        Gate: C1TerminalReconciliationGate,
    {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let record = self
            .identity
            .store
            .recovery_journal_record()
            .map_err(map_recovery_journal_error)
            .map_err(recovery_client_error)?
            .ok_or(RecoveryClientError::Rejected)?;
        operation_snapshot
            .validate_operation_identity(record.operation_id, &record.binding)
            .map_err(|_| RecoveryClientError::Rejected)?;

        let outcome_digest = match &record.phase {
            RecoveryJournalPhase::Reconciled { outcome_digest } => *outcome_digest,
            RecoveryJournalPhase::Intent
            | RecoveryJournalPhase::EffectStarted
            | RecoveryJournalPhase::OutcomeUnknown
            | RecoveryJournalPhase::RuntimeActive => {
                system_service_terminal_outcome_digest(record.operation_id, &record.binding)
            }
        };
        let reconciliation = RecoveryReconciliation::from_journal_record(&record, outcome_digest);
        let claim = nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation::new(
            NonZeroU64::new(reconciliation.operation_id().get())
                .ok_or(RecoveryClientError::Rejected)?,
            *reconciliation.binding().digest_bytes(),
        );
        let (mut admission, store_permit) = self
            .identity
            .prepare_recovery_reconciliation(reconciliation.clone())
            .map_err(recovery_client_error)?;

        let same_scope = std::ptr::eq(operation_scope, current_boot_scope);
        let gated_result =
            gate.with_reconciliation_gate(current_boot_scope, operation_scope, claim, || {
                // A previous-launch caller must keep both the current boot scope
                // and the old operation scope stable while C3 holds its owner
                // gate. In the current-launch compatibility path they are the
                // same object, so snapshot it only once at each boundary.
                let operation_changed =
                    OwnedRecoveryScopeSnapshot::capture(operation_scope) != operation_snapshot;
                let current_boot_changed = !same_scope
                    && OwnedRecoveryScopeSnapshot::capture(current_boot_scope)
                        != current_boot_snapshot;
                if operation_changed || current_boot_changed {
                    return Err(RecoveryClientError::Rejected);
                }
                self.identity
                    .commit_prepared_recovery_reconciliation(
                        &mut admission,
                        store_permit,
                        reconciliation,
                    )
                    .map_err(recovery_client_error)
            });

        let gated_result = match gated_result {
            Ok(result) => result,
            Err(error) => Err(error),
        };
        let operation_changed =
            OwnedRecoveryScopeSnapshot::capture(operation_scope) != operation_snapshot;
        let current_boot_changed = !same_scope
            && OwnedRecoveryScopeSnapshot::capture(current_boot_scope) != current_boot_snapshot;
        if operation_changed || current_boot_changed {
            return if matches!(&gated_result, Err(RecoveryClientError::OutcomeUnknown))
                || admission.is_permanently_closed()
            {
                Err(RecoveryClientError::OutcomeUnknown)
            } else {
                Err(RecoveryClientError::Rejected)
            };
        }

        match gated_result {
            Ok(receipt) => Ok(receipt),
            Err(_error) if admission.is_permanently_closed() => {
                Err(RecoveryClientError::OutcomeUnknown)
            }
            Err(error) => Err(error),
        }
    }

    /// Start one normal C1 recovery operation for the exact verified boot
    /// service admission. The concrete sealed principal is captured by this
    /// adapter; arbitrary system-service callers and public scope metadata
    /// cannot construct this entry point.
    ///
    /// C1 validates the full scope snapshot and its canonical digest against
    /// that principal before authorization or journal access. Once the exact
    /// intent is claimed, the private operation identity is passed together
    /// with the original A-owned scope to `gate`. The A implementation must
    /// compare the scope object to its private admission and run the callback
    /// synchronously inside the real C3 process/endpoint gate. A gate failure
    /// after the durable claim remains quarantined by C1's journal path.
    ///
    /// This claim-only compatibility entrypoint intentionally has no success
    /// result that can be retained as a running service. Use
    /// `recover_with_new_intent_into_runtime` when A will retain the matching
    /// C3 runtime owner.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub fn recover_with_new_intent<Gate>(
        &mut self,
        scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        gate: &mut Gate,
    ) -> Result<RecoveryReport, IdentityError>
    where
        S: C1SystemServiceRecoveryJournal,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
        Gate: C1SystemServiceRecoveryEffectGate,
    {
        let caller = self.boot_binding.ok_or(IdentityError::StoreUnavailable)?;
        let scope_snapshot = OwnedRecoveryScopeSnapshot::capture(scope);
        if scope_snapshot.role != "system_service"
            || !scope_snapshot.matches_boot_principal(caller)
            || scope_snapshot.canonical_binding_digest() != scope_snapshot.binding_digest
        {
            return Err(IdentityError::StoreUnavailable);
        }

        let binding = RecoveryBinding::from_canonical_scope_digest(scope_snapshot.binding_digest);
        let mut gate = VerifiedBootRecoveryEffectGate {
            scope,
            scope_snapshot,
            binding,
            gate,
            effect_invoked: Cell::new(false),
        };
        self.identity
            .recover_with_new_intent(self.caller, &mut gate)
    }

    /// Run recovery and return C1's validated runtime handoff. This succeeds
    /// only after the exact effect callback ran, C1 committed and read back
    /// `RuntimeActive`, and the identity store was rebound to a new issuer.
    /// A must retain this token with its `SystemServiceRuntime`, which owns the
    /// corresponding C3 proof, lifecycle, and package lease. This method does
    /// not publish service READY or independently prove C3 liveness.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub fn recover_with_new_intent_into_runtime<Gate>(
        &mut self,
        scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        gate: &mut Gate,
    ) -> Result<C1SystemServiceRuntimeHandoff, IdentityError>
    where
        S: C1SystemServiceRecoveryJournal,
        I: IdentityIdSource,
        C: Clock,
        A: IdentityAuthorizer,
        G: GuestProfileCleaner,
        E: IdentityEventSink,
        Gate: C1SystemServiceRecoveryEffectGate,
    {
        let caller = self.boot_binding.ok_or(IdentityError::StoreUnavailable)?;
        let scope_snapshot = OwnedRecoveryScopeSnapshot::capture(scope);
        if scope_snapshot.role != "system_service"
            || !scope_snapshot.matches_boot_principal(caller)
            || scope_snapshot.canonical_binding_digest() != scope_snapshot.binding_digest
        {
            return Err(IdentityError::StoreUnavailable);
        }

        let binding = RecoveryBinding::from_canonical_scope_digest(scope_snapshot.binding_digest);
        let mut gate = VerifiedBootRecoveryEffectGate {
            scope,
            scope_snapshot,
            binding,
            gate,
            effect_invoked: Cell::new(false),
        };
        let handoff = self
            .identity
            .recover_with_new_intent_into_runtime(self.caller, &mut gate)?;
        if !gate.effect_invoked.get() {
            return Err(IdentityError::StoreUnavailable);
        }
        self.identity
            .validate_system_service_runtime_handoff(&handoff)?;
        Ok(handoff)
    }
}

/// Owned view of one A admission observation. Every public getter is invoked
/// exactly once; all caller, binding, and operation checks use this same
/// snapshot, so a stateful trait implementation cannot change the compared
/// values between validation steps. This is consistency checking only: the
/// snapshot does not authenticate the scope or create C1 authority.
#[cfg(feature = "system-service-recovery-contract")]
#[derive(Clone, Debug, Eq, PartialEq)]
struct OwnedRecoveryScopeSnapshot {
    service_id: String,
    principal_id: String,
    caller_context_id: String,
    mapping_generation: NonZeroU64,
    role: String,
    catalog_generation: NonZeroU64,
    transition: nagi_system_service_recovery_contract::RecoveryLifecycleTransition,
    package_id: String,
    package_app_id: u64,
    canonical_entry: String,
    signed_package_sha256: [u8; 32],
    executable_sha256: [u8; 32],
    package_generation: u64,
    c3_launch_generation: NonZeroU64,
    uid: u32,
    gid: u32,
    binding_digest: [u8; 32],
}

#[cfg(feature = "system-service-recovery-contract")]
#[derive(Clone, Copy)]
struct RecoveryBootScopeIdentity<'a> {
    service_id: &'a str,
    principal_id: &'a str,
    caller_context_id: &'a str,
    mapping_generation: NonZeroU64,
    catalog_generation: NonZeroU64,
    c3_launch_generation: NonZeroU64,
}

#[cfg(feature = "system-service-recovery-contract")]
impl RecoveryBootScopeIdentity<'_> {
    fn same_service_mapping(&self, other: &Self) -> bool {
        self.service_id == other.service_id
            && self.principal_id == other.principal_id
            && self.caller_context_id == other.caller_context_id
            && self.mapping_generation == other.mapping_generation
            && self.catalog_generation == other.catalog_generation
    }

    fn is_exact_launch_of(&self, current: &Self) -> bool {
        self.same_service_mapping(current)
            && self.c3_launch_generation == current.c3_launch_generation
    }

    fn is_prior_launch_of(&self, current: &Self) -> bool {
        self.same_service_mapping(current)
            && self.c3_launch_generation < current.c3_launch_generation
    }
}

#[cfg(feature = "system-service-recovery-contract")]
impl<'a> From<&'a OwnedRecoveryScopeSnapshot> for RecoveryBootScopeIdentity<'a> {
    fn from(scope: &'a OwnedRecoveryScopeSnapshot) -> Self {
        Self {
            service_id: &scope.service_id,
            principal_id: &scope.principal_id,
            caller_context_id: &scope.caller_context_id,
            mapping_generation: scope.mapping_generation,
            catalog_generation: scope.catalog_generation,
            c3_launch_generation: scope.c3_launch_generation,
        }
    }
}

#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
impl<'a> From<&'a C1VerifiedBootSystemServicePrincipal> for RecoveryBootScopeIdentity<'a> {
    fn from(principal: &'a C1VerifiedBootSystemServicePrincipal) -> Self {
        Self {
            service_id: &principal.service_id,
            principal_id: principal.principal_id.as_str(),
            caller_context_id: principal.caller_context_id.as_str(),
            mapping_generation: principal.mapping_generation,
            catalog_generation: principal.catalog_generation,
            c3_launch_generation: principal.c3_launch_generation,
        }
    }
}

#[cfg(feature = "system-service-recovery-contract")]
impl OwnedRecoveryScopeSnapshot {
    fn capture(
        scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
    ) -> Self {
        let service_id = scope.service_id().to_owned();
        let principal_id = scope.principal_id().to_owned();
        let caller_context_id = scope.caller_context_id().to_owned();
        let mapping_generation = scope.mapping_generation();
        let role = scope.role().to_owned();
        let catalog_generation = scope.catalog_generation();
        let transition = scope.transition();
        let package_id = scope.package_id().to_owned();
        let package_app_id = scope.package_app_id();
        let canonical_entry = scope.canonical_entry().to_owned();
        let signed_package_sha256 = *scope.signed_package_sha256();
        let executable_sha256 = *scope.executable_sha256();
        let package_generation = scope.package_generation();
        let c3_launch_generation = scope.c3_launch_generation();
        let (uid, gid) = scope.credentials();
        let binding_digest = *scope.binding_digest();
        Self {
            service_id,
            principal_id,
            caller_context_id,
            mapping_generation,
            role,
            catalog_generation,
            transition,
            package_id,
            package_app_id,
            canonical_entry,
            signed_package_sha256,
            executable_sha256,
            package_generation,
            c3_launch_generation,
            uid,
            gid,
            binding_digest,
        }
    }

    fn canonical_binding_digest(&self) -> [u8; 32] {
        nagi_system_service_recovery_contract::canonical_recovery_binding_digest(
            &nagi_system_service_recovery_contract::SystemServiceRecoveryScopeInput {
                service_id: &self.service_id,
                principal_id: &self.principal_id,
                caller_context_id: &self.caller_context_id,
                mapping_generation: self.mapping_generation,
                role: &self.role,
                catalog_generation: self.catalog_generation,
                transition: self.transition,
                package_id: &self.package_id,
                package_app_id: self.package_app_id,
                canonical_entry: &self.canonical_entry,
                signed_package_sha256: &self.signed_package_sha256,
                executable_sha256: &self.executable_sha256,
                package_generation: self.package_generation,
                c3_launch_generation: self.c3_launch_generation,
                uid: self.uid,
                gid: self.gid,
            },
        )
    }

    fn validate_operation_identity(
        &self,
        operation_id: crate::store::RecoveryOperationId,
        binding: &RecoveryBinding,
    ) -> Result<(), IdentityError> {
        if operation_id.get() == 0
            || self.canonical_binding_digest() != self.binding_digest
            || binding.digest_bytes() != &self.binding_digest
        {
            return Err(IdentityError::StoreUnavailable);
        }
        Ok(())
    }

    fn validate_caller(
        &self,
        principal_id: &PrincipalId,
        caller_context_id: &CallerContextId,
    ) -> Result<(), IdentityError> {
        if self.principal_id != principal_id.as_str()
            || self.caller_context_id != caller_context_id.as_str()
        {
            return Err(IdentityError::StoreUnavailable);
        }
        Ok(())
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn matches_boot_principal(&self, caller: &C1VerifiedBootSystemServicePrincipal) -> bool {
        self.role == "system_service"
            && RecoveryBootScopeIdentity::from(self)
                .is_exact_launch_of(&RecoveryBootScopeIdentity::from(caller))
    }

    /// Match an older scope only for the dedicated terminal-reconciliation
    /// path. New recovery intents continue to require `matches_boot_principal`
    /// and an exact current launch generation.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn matches_previous_launch_principal(
        &self,
        caller: &C1VerifiedBootSystemServicePrincipal,
    ) -> bool {
        self.role == "system_service"
            && RecoveryBootScopeIdentity::from(self)
                .is_prior_launch_of(&RecoveryBootScopeIdentity::from(caller))
    }
}

#[cfg(feature = "system-service-recovery-contract")]
fn previous_launch_scope_snapshots_match(
    current_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
    previous_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
    current_snapshot: &OwnedRecoveryScopeSnapshot,
    previous_snapshot: &OwnedRecoveryScopeSnapshot,
) -> bool {
    let previous_matches =
        OwnedRecoveryScopeSnapshot::capture(previous_scope) == *previous_snapshot;
    let current_matches = OwnedRecoveryScopeSnapshot::capture(current_scope) == *current_snapshot;
    previous_matches && current_matches
}

#[cfg(all(
    feature = "system-service-recovery-contract",
    any(test, all(target_os = "linux", target_arch = "x86_64"))
))]
fn previous_launch_terminal_receipt_matches(
    receipt: &impl nagi_system_service_recovery_contract::C3DurableRecoverySlotReceipt,
    previous_scope: &OwnedRecoveryScopeSnapshot,
    claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
) -> bool {
    receipt.state() == nagi_system_service_recovery_contract::C3RecoverySlotState::ClearedAndAbsent
        && previous_terminal_receipt_correlation_matches(receipt, previous_scope, claim)
}

#[cfg(all(
    feature = "system-service-recovery-contract",
    any(test, all(target_os = "linux", target_arch = "x86_64"))
))]
fn previous_terminal_receipt_correlation_matches(
    receipt: &impl nagi_system_service_recovery_contract::C3DurableRecoverySlotReceipt,
    previous_scope: &OwnedRecoveryScopeSnapshot,
    claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
) -> bool {
    receipt.c1_operation_id().map(NonZeroU64::get) == Some(claim.operation_id().get())
        && receipt.service_id() == previous_scope.service_id
        && receipt.catalog_generation() == previous_scope.catalog_generation.get()
        && receipt.c3_launch_generation() == previous_scope.c3_launch_generation.get()
        && receipt.scope_binding_digest() == &previous_scope.binding_digest
        && receipt.scope_binding_digest() == claim.binding_digest()
}

#[cfg(all(
    feature = "system-service-recovery-contract",
    any(test, all(target_os = "linux", target_arch = "x86_64"))
))]
fn previous_boot_terminal_receipt_matches(
    receipt: &impl nagi_system_service_recovery_contract::C3DurablePriorBootClearedReceipt,
    previous_scope: &OwnedRecoveryScopeSnapshot,
    claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
) -> bool {
    receipt.state() == nagi_system_service_recovery_contract::C3RecoverySlotState::PriorBootCleared
        && previous_terminal_receipt_correlation_matches(receipt, previous_scope, claim)
        && OwnedRecoveryScopeSnapshot::capture(receipt.previous_scope()) == *previous_scope
        && receipt.previous_boot_id() != &[0; 16]
        && receipt.current_boot_id() != &[0; 16]
        && receipt.previous_boot_id() != receipt.current_boot_id()
}

#[cfg(all(test, feature = "system-service-recovery-contract"))]
mod recovery_boot_scope_identity_tests {
    use super::{
        previous_boot_terminal_receipt_matches, previous_launch_scope_snapshots_match,
        previous_launch_terminal_receipt_matches, OwnedRecoveryScopeSnapshot,
        RecoveryBootScopeIdentity,
    };
    use nagi_system_service_recovery_contract::{
        RecoveryLifecycleTransition, SystemServiceRecoveryAdmissionScope,
        SystemServiceRecoveryScopeInput,
    };
    use std::cell::Cell;
    use std::num::NonZeroU64;

    fn identity(launch: u64) -> RecoveryBootScopeIdentity<'static> {
        RecoveryBootScopeIdentity {
            service_id: "service:fixture.identity",
            principal_id: "service:fixture.identity",
            caller_context_id: "caller_fixture_identity",
            mapping_generation: NonZeroU64::new(3).unwrap(),
            catalog_generation: NonZeroU64::new(9).unwrap(),
            c3_launch_generation: NonZeroU64::new(launch).unwrap(),
        }
    }

    #[test]
    fn previous_launch_match_requires_same_service_and_strictly_older_generation() {
        let current = identity(18);
        assert!(identity(17).is_prior_launch_of(&current));
        assert!(identity(18).is_exact_launch_of(&current));
        assert!(!identity(18).is_prior_launch_of(&current));
        assert!(!identity(19).is_prior_launch_of(&current));

        let mut different_mapping = identity(17);
        different_mapping.mapping_generation = NonZeroU64::new(4).unwrap();
        assert!(!different_mapping.is_prior_launch_of(&current));

        let mut different_catalog = identity(17);
        different_catalog.catalog_generation = NonZeroU64::new(10).unwrap();
        assert!(!different_catalog.is_prior_launch_of(&current));

        let mut different_service = identity(17);
        different_service.service_id = "service:other";
        assert!(!different_service.is_prior_launch_of(&current));
    }

    #[derive(Clone)]
    struct ScopeFixture {
        generation: NonZeroU64,
        binding_digest: [u8; 32],
    }

    impl ScopeFixture {
        fn new(generation: u64) -> Self {
            let generation = NonZeroU64::new(generation).unwrap();
            let binding_digest =
                nagi_system_service_recovery_contract::canonical_recovery_binding_digest(
                    &SystemServiceRecoveryScopeInput {
                        service_id: "service:fixture.identity",
                        principal_id: "service:fixture.identity",
                        caller_context_id: "caller_fixture_identity",
                        mapping_generation: NonZeroU64::new(3).unwrap(),
                        role: "system_service",
                        catalog_generation: NonZeroU64::new(9).unwrap(),
                        transition: RecoveryLifecycleTransition::Initial,
                        package_id: "org.nagi.fixture",
                        package_app_id: 0x1122,
                        canonical_entry: "fixture.elf",
                        signed_package_sha256: &[0x11; 32],
                        executable_sha256: &[0x22; 32],
                        package_generation: 17,
                        c3_launch_generation: generation,
                        uid: 1001,
                        gid: 1002,
                    },
                );
            Self {
                generation,
                binding_digest,
            }
        }
    }

    impl SystemServiceRecoveryAdmissionScope for ScopeFixture {
        fn service_id(&self) -> &str {
            "service:fixture.identity"
        }

        fn principal_id(&self) -> &str {
            "service:fixture.identity"
        }

        fn caller_context_id(&self) -> &str {
            "caller_fixture_identity"
        }

        fn mapping_generation(&self) -> NonZeroU64 {
            NonZeroU64::new(3).unwrap()
        }

        fn role(&self) -> &str {
            "system_service"
        }

        fn catalog_generation(&self) -> NonZeroU64 {
            NonZeroU64::new(9).unwrap()
        }

        fn transition(&self) -> RecoveryLifecycleTransition {
            RecoveryLifecycleTransition::Initial
        }

        fn package_id(&self) -> &str {
            "org.nagi.fixture"
        }

        fn package_app_id(&self) -> u64 {
            0x1122
        }

        fn canonical_entry(&self) -> &str {
            "fixture.elf"
        }

        fn signed_package_sha256(&self) -> &[u8; 32] {
            &[0x11; 32]
        }

        fn executable_sha256(&self) -> &[u8; 32] {
            &[0x22; 32]
        }

        fn package_generation(&self) -> u64 {
            17
        }

        fn c3_launch_generation(&self) -> NonZeroU64 {
            self.generation
        }

        fn credentials(&self) -> (u32, u32) {
            (1001, 1002)
        }

        fn binding_digest(&self) -> &[u8; 32] {
            &self.binding_digest
        }
    }

    #[derive(Clone)]
    struct TerminalReceiptFixture {
        previous_scope: ScopeFixture,
        c1_operation_id: Option<NonZeroU64>,
        service_id: String,
        catalog_generation: u64,
        c3_launch_generation: u64,
        scope_binding_digest: [u8; 32],
        state: nagi_system_service_recovery_contract::C3RecoverySlotState,
        boot_ids: ([u8; 16], [u8; 16]),
    }

    impl nagi_system_service_recovery_contract::C3DurableRecoverySlotReceipt
        for TerminalReceiptFixture
    {
        fn c1_operation_id(&self) -> Option<NonZeroU64> {
            self.c1_operation_id
        }

        fn service_id(&self) -> &str {
            &self.service_id
        }

        fn catalog_generation(&self) -> u64 {
            self.catalog_generation
        }

        fn c3_launch_generation(&self) -> u64 {
            self.c3_launch_generation
        }

        fn scope_binding_digest(&self) -> &[u8; 32] {
            &self.scope_binding_digest
        }

        fn state(&self) -> nagi_system_service_recovery_contract::C3RecoverySlotState {
            self.state
        }
    }

    impl nagi_system_service_recovery_contract::C3DurablePriorBootClearedReceipt
        for TerminalReceiptFixture
    {
        fn previous_scope(
            &self,
        ) -> &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope
        {
            &self.previous_scope
        }
        fn previous_boot_id(&self) -> &[u8; 16] {
            &self.boot_ids.0
        }
        fn current_boot_id(&self) -> &[u8; 16] {
            &self.boot_ids.1
        }
        fn previous_c3_launch_instance_id(&self) -> NonZeroU64 {
            NonZeroU64::MIN
        }
    }

    #[test]
    fn same_catalog_successor_recomputes_scope_digest_and_keeps_old_terminal_claim() {
        use nagi_system_service_recovery_contract::{
            C1RecoveryOperationCorrelation, C3RecoverySlotState,
        };

        let previous = ScopeFixture::new(17);
        let current = ScopeFixture::new(18);
        let previous_snapshot = OwnedRecoveryScopeSnapshot::capture(&previous);
        let current_snapshot = OwnedRecoveryScopeSnapshot::capture(&current);

        assert_eq!(previous.package_id(), current.package_id());
        assert_eq!(
            previous.signed_package_sha256(),
            current.signed_package_sha256()
        );
        assert_eq!(previous.executable_sha256(), current.executable_sha256());
        assert!(RecoveryBootScopeIdentity::from(&previous_snapshot)
            .is_prior_launch_of(&RecoveryBootScopeIdentity::from(&current_snapshot)));
        assert!(previous_launch_scope_snapshots_match(
            &current,
            &previous,
            &current_snapshot,
            &previous_snapshot,
        ));
        assert_eq!(
            nagi_system_service_recovery_contract::canonical_recovery_binding_digest_from_scope(
                &previous,
            ),
            previous.binding_digest
        );
        assert_eq!(
            nagi_system_service_recovery_contract::canonical_recovery_binding_digest_from_scope(
                &current,
            ),
            current.binding_digest
        );
        assert_ne!(
            previous.binding_digest, current.binding_digest,
            "the next C3 generation changes the full scope digest even with identical package and executable bindings"
        );

        let previous_claim = C1RecoveryOperationCorrelation::new(
            NonZeroU64::new(41).unwrap(),
            previous.binding_digest,
        );
        let current_claim = C1RecoveryOperationCorrelation::new(
            NonZeroU64::new(42).unwrap(),
            current.binding_digest,
        );
        let previous_receipt = TerminalReceiptFixture {
            c1_operation_id: Some(NonZeroU64::new(41).unwrap()),
            service_id: previous.service_id().to_owned(),
            catalog_generation: previous.catalog_generation().get(),
            c3_launch_generation: previous.c3_launch_generation().get(),
            scope_binding_digest: previous.binding_digest,
            state: C3RecoverySlotState::ClearedAndAbsent,
            previous_scope: previous.clone(),
            boot_ids: ([0; 16], [0; 16]),
        };

        assert!(previous_launch_terminal_receipt_matches(
            &previous_receipt,
            &previous_snapshot,
            previous_claim,
        ));
        assert!(
            !previous_launch_terminal_receipt_matches(
                &previous_receipt,
                &current_snapshot,
                current_claim,
            ),
            "a terminal row for the old C1 operation and C3 generation cannot confirm the next C1 operation"
        );
    }

    #[test]
    fn previous_launch_terminal_receipt_requires_exact_operation_and_scope() {
        use nagi_system_service_recovery_contract::{
            C1RecoveryOperationCorrelation, C3RecoverySlotState,
        };

        let previous = ScopeFixture::new(17);
        let snapshot = OwnedRecoveryScopeSnapshot::capture(&previous);
        let claim = C1RecoveryOperationCorrelation::new(
            NonZeroU64::new(41).unwrap(),
            previous.binding_digest,
        );
        let exact = TerminalReceiptFixture {
            c1_operation_id: Some(NonZeroU64::new(41).unwrap()),
            service_id: "service:fixture.identity".to_owned(),
            catalog_generation: 9,
            c3_launch_generation: 17,
            scope_binding_digest: previous.binding_digest,
            state: C3RecoverySlotState::ClearedAndAbsent,
            previous_scope: previous.clone(),
            boot_ids: ([0; 16], [0; 16]),
        };

        assert!(previous_launch_terminal_receipt_matches(
            &exact, &snapshot, claim,
        ));
        let wrong_binding_claim =
            C1RecoveryOperationCorrelation::new(NonZeroU64::new(41).unwrap(), [0x77; 32]);
        assert!(
            !previous_launch_terminal_receipt_matches(&exact, &snapshot, wrong_binding_claim,),
            "the receipt must match the binding digest in C1's exact claim"
        );

        let mut wrong_operation = exact.clone();
        wrong_operation.c1_operation_id = Some(NonZeroU64::new(42).unwrap());
        let mut missing_operation = exact.clone();
        missing_operation.c1_operation_id = None;
        let mut wrong_service = exact.clone();
        wrong_service.service_id = "service:other".to_owned();
        let mut wrong_catalog = exact.clone();
        wrong_catalog.catalog_generation += 1;
        let mut wrong_launch = exact.clone();
        wrong_launch.c3_launch_generation += 1;
        let mut wrong_digest = exact.clone();
        wrong_digest.scope_binding_digest[0] ^= 0x80;
        let mut live = exact.clone();
        live.state = C3RecoverySlotState::Live;
        let mut quarantined = exact.clone();
        quarantined.state = C3RecoverySlotState::Quarantined;
        let mut unknown = exact;
        unknown.state = C3RecoverySlotState::Unknown;

        for mismatch in [
            wrong_operation,
            missing_operation,
            wrong_service,
            wrong_catalog,
            wrong_launch,
            wrong_digest,
            live,
            quarantined,
            unknown,
        ] {
            assert!(
                !previous_launch_terminal_receipt_matches(&mismatch, &snapshot, claim),
                "C1 must reject any C3 receipt that does not exactly confirm this prior operation and scope"
            );
        }
    }

    #[test]
    fn prior_boot_terminal_receipt_requires_exact_claim_and_distinct_boot_ids() {
        use nagi_system_service_recovery_contract::{
            C1RecoveryOperationCorrelation, C3RecoverySlotState,
        };

        let previous = ScopeFixture::new(17);
        let snapshot = OwnedRecoveryScopeSnapshot::capture(&previous);
        let claim = C1RecoveryOperationCorrelation::new(
            NonZeroU64::new(41).unwrap(),
            previous.binding_digest,
        );
        let exact = TerminalReceiptFixture {
            c1_operation_id: Some(NonZeroU64::new(41).unwrap()),
            service_id: "service:fixture.identity".to_owned(),
            catalog_generation: 9,
            c3_launch_generation: 17,
            scope_binding_digest: previous.binding_digest,
            state: C3RecoverySlotState::PriorBootCleared,
            previous_scope: previous.clone(),
            boot_ids: ([0x11; 16], [0x22; 16]),
        };

        assert!(previous_boot_terminal_receipt_matches(
            &exact, &snapshot, claim,
        ));

        let mut wrong_operation = exact.clone();
        wrong_operation.c1_operation_id = Some(NonZeroU64::new(42).unwrap());
        let mut same_boot = exact.clone();
        same_boot.boot_ids.1 = same_boot.boot_ids.0;
        let mut zero_old_boot = exact.clone();
        zero_old_boot.boot_ids.0 = [0; 16];
        let mut normal_exit = exact.clone();
        normal_exit.state = C3RecoverySlotState::ClearedAndAbsent;
        let mut wrong_scope = exact.clone();
        wrong_scope.previous_scope = ScopeFixture::new(16);
        let mut live = exact;
        live.state = C3RecoverySlotState::Live;

        for mismatch in [
            wrong_operation,
            same_boot,
            zero_old_boot,
            normal_exit,
            wrong_scope,
            live,
        ] {
            assert!(
                !previous_boot_terminal_receipt_matches(&mismatch, &snapshot, claim),
                "C1 must reject a non-exact claim or any receipt that does not prove a distinct prior boot"
            );
        }
    }

    struct AliasedCurrentThenPreviousScope {
        current: ScopeFixture,
        previous: ScopeFixture,
        current_view: Cell<bool>,
    }

    impl AliasedCurrentThenPreviousScope {
        fn selected_view(&self) -> &ScopeFixture {
            if self.current_view.get() {
                &self.current
            } else {
                &self.previous
            }
        }
    }

    impl SystemServiceRecoveryAdmissionScope for AliasedCurrentThenPreviousScope {
        fn service_id(&self) -> &str {
            self.selected_view().service_id()
        }

        fn principal_id(&self) -> &str {
            self.selected_view().principal_id()
        }

        fn caller_context_id(&self) -> &str {
            self.selected_view().caller_context_id()
        }

        fn mapping_generation(&self) -> NonZeroU64 {
            self.selected_view().mapping_generation()
        }

        fn role(&self) -> &str {
            self.selected_view().role()
        }

        fn catalog_generation(&self) -> NonZeroU64 {
            self.selected_view().catalog_generation()
        }

        fn transition(&self) -> RecoveryLifecycleTransition {
            self.selected_view().transition()
        }

        fn package_id(&self) -> &str {
            self.selected_view().package_id()
        }

        fn package_app_id(&self) -> u64 {
            self.selected_view().package_app_id()
        }

        fn canonical_entry(&self) -> &str {
            self.selected_view().canonical_entry()
        }

        fn signed_package_sha256(&self) -> &[u8; 32] {
            self.selected_view().signed_package_sha256()
        }

        fn executable_sha256(&self) -> &[u8; 32] {
            self.selected_view().executable_sha256()
        }

        fn package_generation(&self) -> u64 {
            self.selected_view().package_generation()
        }

        fn c3_launch_generation(&self) -> NonZeroU64 {
            self.selected_view().c3_launch_generation()
        }

        fn credentials(&self) -> (u32, u32) {
            self.selected_view().credentials()
        }

        fn binding_digest(&self) -> &[u8; 32] {
            let was_current = self.current_view.replace(false);
            if was_current {
                self.current.binding_digest()
            } else {
                self.previous.binding_digest()
            }
        }
    }

    #[test]
    fn prior_launch_revalidates_both_scopes_when_the_views_alias() {
        let scope = AliasedCurrentThenPreviousScope {
            current: ScopeFixture::new(18),
            previous: ScopeFixture::new(17),
            current_view: Cell::new(true),
        };
        let current_snapshot = OwnedRecoveryScopeSnapshot::capture(&scope);
        let previous_snapshot = OwnedRecoveryScopeSnapshot::capture(&scope);
        assert_eq!(current_snapshot.c3_launch_generation.get(), 18);
        assert_eq!(previous_snapshot.c3_launch_generation.get(), 17);

        // The old pointer-identity shortcut checked only the previous view,
        // which is stable here. The current view has drifted to generation 17
        // and must independently fail revalidation.
        assert!(!previous_launch_scope_snapshots_match(
            &scope,
            &scope,
            &current_snapshot,
            &previous_snapshot,
        ));
    }
}

/// Binds A's public gate port to the one owned scope snapshot that C1 checked
/// against its sealed boot principal. The durable C1 binding is built from
/// this same snapshot; it is never read from a separately supplied gate.
#[cfg(feature = "system-service-recovery-contract")]
struct VerifiedBootRecoveryEffectGate<'scope, 'gate, Gate> {
    scope: &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
    scope_snapshot: OwnedRecoveryScopeSnapshot,
    binding: RecoveryBinding,
    gate: &'gate mut Gate,
    effect_invoked: Cell<bool>,
}

#[cfg(feature = "system-service-recovery-contract")]
impl<Gate: C1SystemServiceRecoveryEffectGate> SystemServiceRecoveryEffectGate
    for VerifiedBootRecoveryEffectGate<'_, '_, Gate>
{
    fn recovery_binding(&self) -> &RecoveryBinding {
        &self.binding
    }

    fn apply_recovery_effect<R>(
        &mut self,
        operation: C1RecoveryOperationIdentity,
        effect: impl FnOnce() -> Result<R, IdentityError>,
    ) -> Result<Result<R, IdentityError>, RecoveryEffectGateError> {
        if self.effect_invoked.get()
            || operation.binding_digest() != self.binding.digest_bytes()
            || OwnedRecoveryScopeSnapshot::capture(self.scope) != self.scope_snapshot
        {
            return Err(RecoveryEffectGateError::Unavailable);
        }
        let result = self.gate.apply_recovery_effect(self.scope, operation, || {
            self.effect_invoked.set(true);
            effect()
        });
        if !self.effect_invoked.get()
            || OwnedRecoveryScopeSnapshot::capture(self.scope) != self.scope_snapshot
        {
            return Err(RecoveryEffectGateError::Unavailable);
        }
        result
    }
}

#[cfg(any(test, feature = "system-service-recovery-contract"))]
#[derive(Clone)]
struct OwnedTrustedCallerContext {
    principal_id: PrincipalId,
    caller_context_id: CallerContextId,
}

#[cfg(any(test, feature = "system-service-recovery-contract"))]
impl TrustedCallerContext for OwnedTrustedCallerContext {
    fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    fn context_id(&self) -> &CallerContextId {
        &self.caller_context_id
    }
}

/// Derive a stable C1 receipt digest from the exact operation and canonical
/// binding. It identifies the C1 terminal transition; C3's separately retained
/// durable slot receipt remains the evidence for the owner-side outcome.
#[cfg(feature = "system-service-recovery-contract")]
fn system_service_terminal_outcome_digest(
    operation_id: crate::store::RecoveryOperationId,
    binding: &RecoveryBinding,
) -> [u8; 32] {
    use sha2::Digest;

    let mut digest = sha2::Sha256::new();
    digest.update(b"NAGI-C1-SYSTEM-SERVICE-RECOVERY-TERMINAL\0\x01");
    digest.update(operation_id.get().to_be_bytes());
    digest.update(binding.digest_bytes());
    digest.finalize().into()
}

#[cfg(feature = "system-service-recovery-contract")]
fn system_service_prior_boot_outcome_digest(
    operation_id: crate::store::RecoveryOperationId,
    binding: &RecoveryBinding,
) -> [u8; 32] {
    use sha2::Digest;

    let mut digest = sha2::Sha256::new();
    digest.update(b"NAGI-C1-SYSTEM-SERVICE-RECOVERY-PRIOR-BOOT-TERMINAL\0\x01");
    digest.update(operation_id.get().to_be_bytes());
    digest.update(binding.digest_bytes());
    digest.finalize().into()
}

#[cfg(feature = "system-service-recovery-contract")]
impl nagi_system_service_recovery_contract::C1RecoveryTerminalReceipt
    for C1SystemServiceRecoveryTerminalReceipt
{
    fn operation_id(&self) -> u64 {
        self.operation_id()
    }

    fn binding_digest(&self) -> &[u8; 32] {
        self.binding_digest()
    }

    fn outcome_digest(&self) -> [u8; 32] {
        self.outcome_digest()
    }
}

#[cfg(any(test, feature = "system-service-recovery-contract"))]
struct RecoveryOperationSuccess {
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    operation: C1RecoveryOperationIdentity,
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    operation_id: RecoveryOperationId,
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    binding: RecoveryBinding,
    report: RecoveryReport,
}

pub struct IdentityService<S: IdentitySnapshotStore, I, C, A, G, E = NoopIdentityEventSink> {
    // Keep first: its Drop closes admission before the snapshot store decides
    // whether active work requires deferred destruction.
    issuer: IssuerGuard,
    store: DeferredSnapshotStore<S>,
    ids: PanicContainedDrop<I>,
    clock: PanicContainedDrop<C>,
    authorizer: PanicContainedDrop<A>,
    guest_cleaner: PanicContainedDrop<G>,
    events: PanicContainedDrop<E>,
    snapshot: IdentitySnapshot,
    store_version: Option<SnapshotVersion>,
    recovered: bool,
    active_by_caller: HashMap<CallerKey, SessionId>,
    last_by_caller: HashMap<CallerKey, SessionId>,
    session_owners: HashMap<SessionId, CallerKey>,
    leases: HashMap<SessionId, Arc<SessionLeaseState>>,
    idempotency: HashMap<(CallerKey, IdempotencyKey), IdempotentSession>,
}

impl<S, I, C, A, G, E> IdentityService<S, I, C, A, G, E>
where
    S: IdentitySnapshotStore,
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    pub fn new(store: S, ids: I, clock: C, authorizer: A, guest_cleaner: G, events: E) -> Self {
        let issuer = IssuerGuard::new();
        let store = DeferredSnapshotStore::new(store, &issuer);
        Self {
            issuer,
            store,
            ids: PanicContainedDrop::new(ids),
            clock: PanicContainedDrop::new(clock),
            authorizer: PanicContainedDrop::new(authorizer),
            guest_cleaner: PanicContainedDrop::new(guest_cleaner),
            events: PanicContainedDrop::new(events),
            snapshot: IdentitySnapshot::empty(),
            store_version: None,
            recovered: false,
            active_by_caller: HashMap::new(),
            last_by_caller: HashMap::new(),
            session_owners: HashMap::new(),
            leases: HashMap::new(),
            idempotency: HashMap::new(),
        }
    }

    pub fn snapshot(&self) -> &IdentitySnapshot {
        &self.snapshot
    }

    #[cfg(all(feature = "linux-interactive-account", target_os = "linux"))]
    pub(crate) fn issuer_binding(&self) -> IdentityIssuerBinding {
        IdentityIssuerBinding(Arc::clone(&self.issuer.admission))
    }

    #[cfg(all(feature = "linux-interactive-account", target_os = "linux"))]
    pub(crate) fn owns_issuer_binding(&self, binding: &IdentityIssuerBinding) -> bool {
        Arc::ptr_eq(&self.issuer.admission, &binding.0)
    }

    /// Allocate a fresh process-local caller-context selector for an
    /// authenticated C1 caller. The ID source is C1-owned; the value is not
    /// authentication evidence. Do not reuse a context already issued during
    /// this issuer lifetime, including contexts whose Session has ended.
    #[cfg(all(feature = "linux-interactive-account", target_os = "linux"))]
    pub(crate) fn issue_fresh_caller_context_id(
        &mut self,
    ) -> Result<CallerContextId, IdentityError> {
        self.ensure_recovered()?;
        self.issue_fresh_caller_context_id_after_admission()
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    pub(crate) fn issue_fresh_caller_context_id_with_runtime_admission(
        &mut self,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<CallerContextId, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        self.validate_system_service_runtime_admission(admission)?;
        self.issue_fresh_caller_context_id_after_admission()
    }

    #[cfg(any(
        all(feature = "linux-interactive-account", target_os = "linux"),
        all(
            feature = "system-service-recovery-contract",
            target_os = "linux",
            target_arch = "x86_64"
        )
    ))]
    fn issue_fresh_caller_context_id_after_admission(
        &mut self,
    ) -> Result<CallerContextId, IdentityError> {
        let context_id = self
            .ids
            .next_caller_context_id()
            .map_err(|_| IdentityError::IdSourceUnavailable)?;
        if self
            .last_by_caller
            .keys()
            .any(|key| key.context_id == context_id)
        {
            return Err(IdentityError::IdCollision);
        }
        Ok(context_id)
    }

    /// Resolve and pin the current interactive identity to the supplied
    /// trusted caller. The returned context owns only a revocable observation;
    /// every operation must call `check` again before using it.
    pub fn interactive_session<'caller>(
        &mut self,
        caller: &'caller impl AuthenticatedInteractiveCaller,
    ) -> Result<InteractiveSessionContext<'caller>, InteractiveSessionError> {
        let identity = CurrentIdentityResolver::resolve_current_identity(self, caller)
            .map_err(InteractiveSessionError::Identity)?;
        InteractiveSessionContext::attach(identity, caller)
    }

    /// Resolve the just-issued Session while the exact C1 system-service
    /// runtime admission remains held by A's C3/C2 owner callback. Unlike the
    /// ordinary resolver, this path obtains a reader that accepts only the
    /// same durable operation in `RuntimeActive`; stale, unknown, or terminal
    /// journal rows still fail closed.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    #[allow(private_bounds)]
    pub fn interactive_session_with_runtime_admission<'caller>(
        &mut self,
        caller: &'caller impl AuthenticatedInteractiveCaller,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<InteractiveSessionContext<'caller>, InteractiveSessionError>
    where
        S: C1SystemServiceRecoveryJournal,
    {
        let identity = self
            .resolve_current_identity_with_runtime_admission(caller, admission)
            .map_err(InteractiveSessionError::Identity)?;
        InteractiveSessionContext::attach(identity, caller)
    }

    /// Return an owned, non-cloneable identity observation backed by the
    /// exact RuntimeActive operation reader. This is the admitted handoff for
    /// consumers that own the identity beyond the borrowed interactive
    /// context; it carries correlation, not authority apart from its live
    /// Session lease.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    #[allow(private_bounds)]
    pub(crate) fn resolve_current_identity_with_runtime_admission(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<ResolvedIdentity, ResolutionError>
    where
        S: C1SystemServiceRecoveryJournal,
    {
        self.validate_system_service_runtime_admission(admission)
            .map_err(map_resolution_identity_error)?;
        let reader = self
            .store
            .current_snapshot_reader_for_runtime(admission.operation_identity())
            .ok_or(ResolutionError::Unavailable)?;
        let mut identity =
            resolve_current_identity_after_admission(self, caller, Some(reader), false)?;
        identity.runtime_operation_identity = Some(admission.operation_identity());
        Ok(identity)
    }

    /// Resolve the current identity and reclaim its root if expiry or a
    /// previously committed logout is confirmed. A transient or concurrent
    /// lease failure returns `Unavailable` and leaves roots intact for an
    /// explicit retry. The adapter must be unbound or the exact adapter
    /// previously registered by this issuer.
    pub fn resolve_current_identity_with_storage<B, SE>(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
        storage: &mut UserStorageRootAdapter<B, SE>,
    ) -> Result<ResolvedIdentity, ResolutionError>
    where
        B: ProfileStorageBackend,
        SE: StorageRootEventSink,
    {
        if !matches!(
            self.issuer
                .accepts_storage_adapter(storage.adapter_identity()),
            Ok(true)
        ) {
            return Err(ResolutionError::Unavailable);
        }
        let result = CurrentIdentityResolver::resolve_current_identity(self, caller);
        if matches!(
            result,
            Err(ResolutionError::Ended | ResolutionError::Expired)
        ) {
            let caller_key = CallerKey::from_trusted(caller);
            if let Some(session_id) = self.last_by_caller.get(&caller_key) {
                storage.revoke_session_for_issuer(&self.issuer.alive, session_id);
            }
        }
        result
    }

    /// Close the issuer and extract its components only when no Session or
    /// recovery callback is active. A busy call returns the still-live issuer
    /// unchanged so its owner can retry after joining effect workers. Dropping
    /// that returned issuer closes admission immediately and defers the store
    /// drop until already-admitted work exits. If this issuer opened profile
    /// roots, use `into_parts_with_storage` so its adapter-owned roots are also
    /// released before extraction.
    // An unsuccessful close returns the owning service unchanged and without allocating.
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    pub fn into_parts(self) -> Result<(S, I, C, A, G, E), Self> {
        if !self.issuer.admission.close_if_idle(&self.issuer.alive) {
            return Err(self);
        }
        Ok(self.into_parts_after_close())
    }

    /// Close this issuer and synchronously release its profile roots from the
    /// exact adapter registered by the issuer. If an effect or recovery is
    /// active or the supplied adapter belongs to another issuer, this returns
    /// the issuer unchanged and does not touch the adapter. Retry after that
    /// work exits or pass the registered adapter. Other issuer roots are never
    /// removed.
    // An unsuccessful close returns the owning service unchanged and without allocating.
    #[allow(clippy::result_large_err, clippy::type_complexity)]
    pub fn into_parts_with_storage<B, SE>(
        self,
        storage: &mut UserStorageRootAdapter<B, SE>,
    ) -> Result<(S, I, C, A, G, E), Self>
    where
        B: ProfileStorageBackend,
        SE: StorageRootEventSink,
    {
        if !matches!(
            self.issuer
                .accepts_storage_adapter(storage.adapter_identity()),
            Ok(true)
        ) {
            return Err(self);
        }
        if !self.issuer.admission.close_if_idle(&self.issuer.alive) {
            return Err(self);
        }
        storage.revoke_issuer(&self.issuer.alive);
        Ok(self.into_parts_after_close())
    }

    fn into_parts_after_close(self) -> (S, I, C, A, G, E) {
        let Self {
            issuer,
            store,
            ids,
            clock,
            authorizer,
            guest_cleaner,
            events,
            ..
        } = self;
        let store = store.into_inner();
        drop(issuer);
        (
            store,
            ids.into_inner(),
            clock.into_inner(),
            authorizer.into_inner(),
            guest_cleaner.into_inner(),
            events.into_inner(),
        )
    }

    fn authorization_request(
        caller: &(impl TrustedCallerContext + ?Sized),
        action: IdentityAction,
        user_id: Option<UserId>,
        profile_id: Option<ProfileId>,
        session_id: Option<SessionId>,
    ) -> AuthorizationRequest {
        AuthorizationRequest {
            action,
            caller_principal: caller.principal_id().clone(),
            caller_context: caller.context_id().clone(),
            user_id,
            profile_id,
            session_id,
        }
    }

    fn authorize(
        &mut self,
        caller: &(impl TrustedCallerContext + ?Sized),
        action: IdentityAction,
        user_id: Option<UserId>,
        profile_id: Option<ProfileId>,
        session_id: Option<SessionId>,
    ) -> Result<(), IdentityError> {
        if C1LeaseGateScope::held_on_current_thread() {
            return Err(IdentityError::LeaseEffectReentry);
        }
        let request = Self::authorization_request(caller, action, user_id, profile_id, session_id);
        self.authorize_request(&request)
    }

    fn authorize_request(&mut self, request: &AuthorizationRequest) -> Result<(), IdentityError> {
        if C1LeaseGateScope::held_on_current_thread() {
            return Err(IdentityError::LeaseEffectReentry);
        }
        self.authorizer
            .authorize(request)
            .map_err(|error| match error {
                AuthorizationFailure::Denied => IdentityError::AuthorizationDenied,
                AuthorizationFailure::Unavailable => IdentityError::AuthorizationUnavailable,
            })
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    fn authorize_committed_session_with_runtime_admission(
        &mut self,
        caller: &dyn AuthenticatedInteractiveCaller,
        request: &AuthorizationRequest,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<(), IdentityError>
    where
        S: RecoveryJournalStore,
    {
        if C1LeaseGateScope::held_on_current_thread() {
            return Err(IdentityError::LeaseEffectReentry);
        }
        self.validate_system_service_runtime_admission(admission)?;

        let (Some(user_id), Some(profile_id), Some(session_id)) = (
            request.user_id,
            request.profile_id.as_ref(),
            request.session_id.as_ref(),
        ) else {
            return Err(IdentityError::AuthorizationDenied);
        };
        let caller_key = CallerKey::from_trusted(caller);
        // Keep principal naming policy in the owning authorizer: generic C1
        // callers may map to a local user without using the Linux `user:<id>`
        // convention, which the production boot authorizer checks itself.
        if request.action != IdentityAction::ResolveCurrentIdentity
            || request.caller_principal != *caller.principal_id()
            || request.caller_context != *caller.context_id()
            || self.session_owners.get(session_id) != Some(&caller_key)
        {
            return Err(IdentityError::AuthorizationDenied);
        }

        let session_is_the_committed_identity = self.snapshot.sessions.iter().any(|session| {
            session.session_id == *session_id
                && session.kind == SessionKind::LocalUser
                && session.user_id == Some(user_id)
                && session.profile_id == *profile_id
                && matches!(session.lifecycle, SessionLifecycle::Active)
        });
        if !session_is_the_committed_identity {
            return Err(IdentityError::AuthorizationDenied);
        }

        self.authorizer
            .authorize_with_runtime_admission(request, admission)
            .map_err(|error| match error {
                AuthorizationFailure::Denied => IdentityError::AuthorizationDenied,
                AuthorizationFailure::Unavailable => IdentityError::AuthorizationUnavailable,
            })
    }

    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn authorize_recovery(
        &mut self,
        caller: &dyn AuthenticatedSystemServicePrincipal,
    ) -> Result<(), IdentityError> {
        let principal_id = caller.principal_id().clone();
        let caller_context_id = caller.context_id().clone();
        self.authorize_recovery_with_captured_caller(principal_id, caller_context_id)
    }

    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn authorize_recovery_with_captured_caller(
        &mut self,
        principal_id: PrincipalId,
        caller_context_id: CallerContextId,
    ) -> Result<(), IdentityError> {
        let caller = OwnedTrustedCallerContext {
            principal_id,
            caller_context_id,
        };
        self.authorize(&caller, IdentityAction::Recover, None, None, None)?;
        self.authorize(
            &caller,
            IdentityAction::ValidateRecoveredState,
            None,
            None,
            None,
        )
    }

    fn emit(
        &mut self,
        kind: IdentityEventKind,
        user_id: Option<UserId>,
        profile_id: Option<ProfileId>,
        session_id: Option<SessionId>,
        reason: Option<IdentityEventReason>,
    ) {
        self.events.emit(&IdentityEvent {
            kind,
            user_id,
            profile_id,
            session_id,
            reason,
        });
    }

    fn ensure_recovered(&self) -> Result<(), IdentityError> {
        if self.store.recovery_quarantined().map_err(map_store_error)? {
            return Err(IdentityError::StoreUnavailable);
        }
        if self.recovered {
            Ok(())
        } else {
            Err(IdentityError::NotRecovered)
        }
    }

    fn save_candidate(&mut self, candidate: IdentitySnapshot) -> Result<(), IdentityError> {
        self.save_candidate_at_version(candidate, self.store_version)
    }

    fn save_candidate_at_version(
        &mut self,
        candidate: IdentitySnapshot,
        expected_version: Option<SnapshotVersion>,
    ) -> Result<(), IdentityError> {
        let bytes = candidate.to_json().map_err(map_snapshot_error)?;
        let next_store_version = match self.store.commit_atomic(expected_version, &bytes) {
            Ok(version) => version,
            Err(error) => {
                self.emit(
                    IdentityEventKind::PersistenceCommitFailed,
                    None,
                    None,
                    None,
                    Some(if error == SnapshotStoreError::Conflict {
                        IdentityEventReason::Conflict
                    } else {
                        IdentityEventReason::Unavailable
                    }),
                );
                return Err(map_store_error(error));
            }
        };
        self.snapshot = candidate;
        self.store_version = Some(next_store_version);
        self.emit(
            IdentityEventKind::PersistenceCommitted,
            None,
            None,
            None,
            None,
        );
        Ok(())
    }

    fn next_revision(&self) -> Result<u64, IdentityError> {
        self.snapshot
            .revision
            .checked_add(1)
            .ok_or(IdentityError::RevisionExhausted)
    }

    fn profile_id_is_reserved(&self, profile_id: &ProfileId) -> bool {
        self.snapshot
            .profiles
            .iter()
            .any(|profile| &profile.profile_id == profile_id)
            || self
                .snapshot
                .sessions
                .iter()
                .any(|session| &session.profile_id == profile_id)
    }

    fn ensure_profile_id_history_complete(&self) -> Result<(), IdentityError> {
        if self.snapshot.profile_id_history_complete {
            Ok(())
        } else {
            Err(IdentityError::ProfileHistoryIncomplete)
        }
    }

    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn read_valid_snapshot(&self, bytes: &[u8]) -> Result<IdentitySnapshot, IdentityError> {
        IdentitySnapshot::from_json(bytes).map_err(map_snapshot_error)
    }

    /// Recovery never resumes a saved session. It selects a valid current or
    /// last-known-good snapshot, ends all active sessions, and cleans only the
    /// exact guest profile IDs recorded in that snapshot.
    ///
    /// Selecting any LKG snapshot persists an incomplete ProfileId-history
    /// marker, even when its version equals the high-water: fallback selection
    /// is not proof that every allocation is represented. Current JSON without
    /// the history marker is also treated as incomplete. New users, profiles,
    /// and guest profiles then return ProfileHistoryIncomplete until an
    /// authoritative repair supplies complete history. This crate currently
    /// has no repair API; existing profiles can still be used for login.
    ///
    /// Snapshot revision is not a global generation: it can roll back when an
    /// LKG snapshot is selected. Session IDs therefore use the store CAS
    /// high-water generation, not snapshot revision. Recovery may initialize
    /// Empty only when the store proves no commit has ever succeeded; durable
    /// adapters that lose or cannot verify high-water must return unavailable
    /// or corrupt state. The in-memory test store makes no persistence claim;
    /// the Linux adapter persists the independent high-water value and refuses
    /// a missing committed state.
    /// Recover durable identity state. If any currently issued Session lease
    /// is held, recovery returns `SessionLeaseBusy` before committing or
    /// revoking any lease; retry after admitted effects have returned.
    pub fn recover(
        &mut self,
        caller: &impl AuthenticatedSystemServicePrincipal,
    ) -> Result<RecoveryReport, IdentityError> {
        #[cfg(not(test))]
        {
            // Production recovery may run only through the C1-private
            // operation/journal path. Until its authenticated A/C3 adapter is
            // connected, this legacy entrypoint denies every production call
            // before reserving issuer admission or touching the store.
            let _ = caller;
            Err(IdentityError::StoreUnavailable)
        }
        #[cfg(test)]
        {
            self.authorize_recovery(caller)?;
            let _recovery_admission = self.issuer.admission.begin_recovery()?;
            if self.store.recovery_quarantined().map_err(map_store_error)?
                || self
                    .store
                    .recovery_operation_active()
                    .map_err(map_store_error)?
            {
                return Err(IdentityError::StoreUnavailable);
            }
            self.preflight_recovery_session_gates()?;
            self.recover_inner_authorized()
        }
    }

    /// Persist an intent and run its one recovery effect inside the supplied
    /// exact-generation owner admission. Recovery policy is checked first;
    /// the gate supplies the same opaque C1 binding used by the journal claim.
    /// C1 admission reserves only when no callbacks are active, and the
    /// Session write-gate preflight succeeds before durable intent is written.
    /// Only a store `Busy` result contractually guaranteed to occur before
    /// any durable intent mutation remains retryable. Any other intent-stage
    /// error or panic closes this issuer because the write outcome may be
    /// uncertain. If a later journal read/claim reports Busy, this convenience
    /// API cannot return its one-shot permit, so it closes the issuer and
    /// leaves an exact OutcomeUnknown row for reconciliation instead of
    /// promising retry.
    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    pub(crate) fn recover_with_new_intent(
        &mut self,
        caller: &dyn AuthenticatedSystemServicePrincipal,
        gate: &mut impl SystemServiceRecoveryEffectGate,
    ) -> Result<RecoveryReport, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        self.recover_with_new_intent_run(caller, gate, false)
            .map(|success| success.report)
    }

    /// Complete the C1 half of a verified boot runtime handoff. The effect
    /// callback must run under A's exact live C3 gate. After it returns, C1
    /// reads back the exact durable `RuntimeActive` row, confirms the recovered
    /// snapshot, and moves the store to a fresh issuer while leaving every old
    /// Session issuer permanently closed. The returned token is intended to
    /// be retained by A beside its live C3 `SystemServiceRuntime`.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    pub(crate) fn recover_with_new_intent_into_runtime(
        &mut self,
        caller: &dyn AuthenticatedSystemServicePrincipal,
        gate: &mut impl SystemServiceRecoveryEffectGate,
    ) -> Result<C1SystemServiceRuntimeHandoff, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        let success = self.recover_with_new_intent_run(caller, gate, true)?;
        self.promote_committed_recovery_runtime(success)
    }

    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn recover_with_new_intent_run(
        &mut self,
        caller: &dyn AuthenticatedSystemServicePrincipal,
        gate: &mut impl SystemServiceRecoveryEffectGate,
        runtime_handoff: bool,
    ) -> Result<RecoveryOperationSuccess, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        self.authorize_recovery(caller)?;
        let mut recovery_admission = self.issuer.admission.begin_recovery()?;
        self.preflight_recovery_session_gates()?;
        let binding = gate.recovery_binding().clone();
        let permit = match catch_unwind(AssertUnwindSafe(|| {
            self.store.begin_recovery_intent(binding.clone())
        })) {
            Ok(Ok(permit)) => permit,
            Ok(Err(RecoveryJournalError::Busy)) => {
                // The store contract permits retry only when Busy proves no
                // intent write began and the durable journal is unchanged.
                return Err(IdentityError::SessionLeaseBusy);
            }
            Ok(Err(error)) => {
                self.close_after_uncertain_recovery_intent(&mut recovery_admission);
                return Err(map_recovery_journal_error(error));
            }
            Err(payload) => {
                self.close_after_uncertain_recovery_intent(&mut recovery_admission);
                // A custom store may panic after publishing the Intent. Keep
                // the old issuer closed and avoid a second panic from the
                // arbitrary payload's destructor.
                std::mem::forget(payload);
                return Err(IdentityError::StoreUnavailable);
            }
        };
        let mut permit = Some(permit);
        let result = self.run_recovery_operation(
            &mut recovery_admission,
            &mut permit,
            gate,
            &binding,
            runtime_handoff,
        );
        if matches!(&result, Err(IdentityError::SessionLeaseBusy)) {
            // This convenience API cannot return its one-shot permit to the
            // caller. Once Intent is durable, reporting retryable Busy would
            // strand that operation after the local permit is dropped. Close
            // this issuer and persist OutcomeUnknown so only exact
            // reconciliation can continue.
            self.close_after_uncertain_recovery_claim(&mut recovery_admission, &mut permit);
            return Err(IdentityError::StoreUnavailable);
        }

        // `promote_committed_recovery_runtime` replaces the old issuer only
        // after this guard has released the old recovery reservation. On any
        // failure the durable claim path has already left the old issuer
        // closed and quarantined.
        drop(recovery_admission);
        result
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    fn promote_committed_recovery_runtime(
        &mut self,
        success: RecoveryOperationSuccess,
    ) -> Result<C1SystemServiceRuntimeHandoff, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        let exact_runtime_active = self
            .store
            .recovery_journal_record()
            .ok()
            .flatten()
            .is_some_and(|record| {
                record.operation_id == success.operation_id
                    && record.binding == success.binding
                    && matches!(record.phase, RecoveryJournalPhase::RuntimeActive)
            });
        if !exact_runtime_active || !self.recovered {
            self.recovered = false;
            self.mark_recovery_outcome_unknown_safely(success.operation_id, &success.binding);
            return Err(IdentityError::StoreUnavailable);
        }

        let current_state = match self.store.load_current_state() {
            Ok(state) => state,
            Err(_) => {
                self.recovered = false;
                self.mark_recovery_outcome_unknown_safely(success.operation_id, &success.binding);
                return Err(IdentityError::StoreUnavailable);
            }
        };
        let snapshot_matches = if current_state.high_water_version != self.store_version {
            false
        } else {
            match current_state.current {
                Some(stored) if current_state.high_water_version == Some(stored.version) => self
                    .read_valid_snapshot(&stored.bytes)
                    .is_ok_and(|snapshot| snapshot == self.snapshot),
                Some(_) => false,
                None => {
                    current_state.high_water_version.is_none()
                        && self.snapshot == IdentitySnapshot::empty()
                }
            }
        };
        if !snapshot_matches
            || !self.active_by_caller.is_empty()
            || !self.session_owners.is_empty()
            || !self.leases.is_empty()
            || !self.idempotency.is_empty()
        {
            self.recovered = false;
            self.mark_recovery_outcome_unknown_safely(success.operation_id, &success.binding);
            return Err(IdentityError::StoreUnavailable);
        }

        let next_issuer = IssuerGuard::new();
        if self
            .store
            .rebind_from_closed_issuer(&self.issuer, &next_issuer)
            .is_err()
        {
            self.recovered = false;
            self.mark_recovery_outcome_unknown_safely(success.operation_id, &success.binding);
            return Err(IdentityError::StoreUnavailable);
        }
        let previous_issuer = std::mem::replace(&mut self.issuer, next_issuer);
        drop(previous_issuer);

        Ok(C1SystemServiceRuntimeHandoff {
            operation: success.operation,
            report: success.report,
            issuer: Arc::clone(&self.issuer.admission),
        })
    }

    /// Validate C1's retained half of a runtime handoff before A publishes its
    /// service as ready. A must separately retain and validate the exact C3
    /// proof, lifecycle, and package admission in its `SystemServiceRuntime`.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    #[allow(private_bounds)]
    pub fn validate_system_service_runtime_handoff(
        &self,
        handoff: &C1SystemServiceRuntimeHandoff,
    ) -> Result<(), IdentityError>
    where
        S: RecoveryJournalStore,
    {
        if !self.recovered
            || !self.issuer.alive.load(Ordering::Acquire)
            || !Arc::ptr_eq(&self.issuer.admission, &handoff.issuer)
        {
            return Err(IdentityError::StoreUnavailable);
        }
        let admission = self
            .issuer
            .admission
            .state
            .try_lock()
            .map_err(|_| IdentityError::StoreUnavailable)?;
        if admission.closed || admission.recovering {
            return Err(IdentityError::StoreUnavailable);
        }
        drop(admission);

        let record = self
            .store
            .recovery_journal_record()
            .map_err(|_| IdentityError::StoreUnavailable)?
            .ok_or(IdentityError::StoreUnavailable)?;
        if record.operation_id.get() != handoff.operation.operation_id().get()
            || record.binding.digest_bytes() != handoff.operation.binding_digest()
            || !matches!(record.phase, RecoveryJournalPhase::RuntimeActive)
        {
            return Err(IdentityError::StoreUnavailable);
        }
        Ok(())
    }

    /// Validate C1's durable runtime handoff and enter its live issuer for one
    /// synchronous effect. A must keep the returned guard alive while calling
    /// C3's `with_durable_live_effect_gate`, using `admission.scope()` and
    /// `admission.operation_identity()` unchanged. The C1 activity count
    /// prevents a new recovery/close from passing admission until that C3
    /// callback returns. A new C1 recovery reservation therefore fails before
    /// it can write another intent. C3 still owns process/endpoint liveness
    /// and its own durable `Live` receipt.
    ///
    /// The scope is captured once and must reproduce the exact canonical
    /// binding stored in the C1 handoff. This method makes no decision from a
    /// point-in-time `is_live` result and does not accept public identifiers
    /// as a caller proof.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    #[allow(private_bounds)]
    pub fn enter_system_service_runtime_admission<'runtime>(
        &mut self,
        handoff: &'runtime C1SystemServiceRuntimeHandoff,
        scope: &'runtime dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
    ) -> Result<C1SystemServiceRuntimeAdmission<'runtime>, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        let scope_snapshot = OwnedRecoveryScopeSnapshot::capture(scope);
        if scope_snapshot.role != "system_service"
            || scope_snapshot.canonical_binding_digest() != scope_snapshot.binding_digest
            || handoff.operation.binding_digest() != &scope_snapshot.binding_digest
        {
            return Err(IdentityError::StoreUnavailable);
        }

        self.validate_system_service_runtime_handoff(handoff)?;
        let Some(issuer_activity) = handoff
            .issuer
            .try_enter()
            .map_err(|_| IdentityError::StoreUnavailable)?
        else {
            return Err(IdentityError::StoreUnavailable);
        };

        // Recheck after admission so a concurrent issuer close or durable
        // terminal transition cannot race the C1-to-C3 handoff.
        self.validate_system_service_runtime_handoff(handoff)?;
        Ok(C1SystemServiceRuntimeAdmission {
            handoff,
            scope,
            _issuer_activity: issuer_activity,
        })
    }

    /// Revalidate a previously entered runtime admission for one C1 operation.
    /// The non-cloneable activity guard must belong to this exact issuer, and
    /// the same borrowed A scope must still hash to the operation recorded as
    /// `RuntimeActive`. This does not validate or replace A/C3's live gate;
    /// callers must keep this admission inside that owner callback.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    pub(crate) fn validate_system_service_runtime_admission(
        &self,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<(), IdentityError>
    where
        S: RecoveryJournalStore,
    {
        if !Arc::ptr_eq(&self.issuer.admission, &admission.handoff.issuer)
            || !Arc::ptr_eq(&self.issuer.admission, &admission._issuer_activity.0)
        {
            return Err(IdentityError::StoreUnavailable);
        }

        let scope_snapshot = OwnedRecoveryScopeSnapshot::capture(admission.scope);
        if scope_snapshot.role != "system_service"
            || scope_snapshot.canonical_binding_digest() != scope_snapshot.binding_digest
            || scope_snapshot.binding_digest != *admission.operation_identity().binding_digest()
        {
            return Err(IdentityError::StoreUnavailable);
        }

        self.validate_system_service_runtime_handoff(admission.handoff)
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        feature = "linux-interactive-account",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    pub(crate) fn authorize_first_owner_provisioning_with_runtime_admission(
        &mut self,
        caller: &impl AuthenticatedProvisioningCaller,
        principal: &C1VerifiedBootSystemServicePrincipal,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<(), IdentityError>
    where
        S: RecoveryJournalStore,
    {
        self.validate_system_service_runtime_admission(admission)?;
        let scope_snapshot = OwnedRecoveryScopeSnapshot::capture(admission.scope);
        if !scope_snapshot.matches_boot_principal(principal) {
            return Err(IdentityError::StoreUnavailable);
        }
        self.authorize(caller, IdentityAction::CreateLocalUser, None, None, None)
    }

    /// Run the exact C1 recovery path and release roots from the issuer's
    /// registered adapter once recovery has made the current issuer unusable.
    /// Pre-intent authorization, admission, session-gate, and store contention
    /// failures keep roots so the owner can retry. Once an intent is durable,
    /// any later error that cannot return its one-shot permit closes the issuer
    /// and leaves the operation for exact reconciliation before roots are
    /// released.
    #[cfg(test)]
    pub(crate) fn recover_with_new_intent_and_storage<B, SE>(
        &mut self,
        caller: &impl AuthenticatedSystemServicePrincipal,
        gate: &mut impl SystemServiceRecoveryEffectGate,
        storage: &mut UserStorageRootAdapter<B, SE>,
    ) -> Result<RecoveryReport, IdentityError>
    where
        S: RecoveryJournalStore,
        B: ProfileStorageBackend,
        SE: StorageRootEventSink,
    {
        match self
            .issuer
            .accepts_storage_adapter(storage.adapter_identity())
        {
            Ok(true) => {}
            Ok(false) => return Err(IdentityError::StorageOwnerMismatch),
            Err(error) => return Err(error.into_identity_error()),
        }
        let was_recovered = self.recovered;
        let result = self.recover_with_new_intent(caller, gate);
        if was_recovered && !self.recovered {
            storage.revoke_issuer(&self.issuer.alive);
        }
        result
    }

    /// Continue one exact durable intent while its one-shot permit remains in
    /// memory. The permit and owner gate must carry the same opaque binding;
    /// mismatches and already-claimed operations are rejected before the gate
    /// callback. C1 does not reconstruct permits after restart; an unresolved
    /// persisted intent then requires exact owner reconciliation.
    #[cfg(test)]
    pub(crate) fn recover_with_operation(
        &mut self,
        caller: &impl AuthenticatedSystemServicePrincipal,
        permit: &mut Option<RecoveryOperationPermit>,
        gate: &mut impl SystemServiceRecoveryEffectGate,
    ) -> Result<RecoveryReport, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        let Some(current_permit) = permit.as_ref() else {
            return Err(IdentityError::StoreUnavailable);
        };
        let expected_binding = gate.recovery_binding().clone();
        if current_permit.binding() != &expected_binding {
            return Err(IdentityError::StoreUnavailable);
        }
        self.authorize_recovery(caller)?;
        self.recover_with_operation_after_authorization(permit, gate, &expected_binding)
    }

    #[cfg(test)]
    fn recover_with_operation_after_authorization(
        &mut self,
        permit: &mut Option<RecoveryOperationPermit>,
        gate: &mut impl SystemServiceRecoveryEffectGate,
        expected_binding: &RecoveryBinding,
    ) -> Result<RecoveryReport, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        let mut recovery_admission = self.issuer.admission.begin_recovery()?;
        self.preflight_recovery_session_gates()?;
        self.run_recovery_operation(
            &mut recovery_admission,
            permit,
            gate,
            expected_binding,
            false,
        )
        .map(|success| success.report)
    }

    /// Validate the same canonical A scope against C1's existing private
    /// permit before delegating to the ordinary C1 recovery gate path. This is
    /// an integration preflight only; a public scope cannot mint the permit or
    /// authenticate `caller`.
    #[cfg(all(test, feature = "system-service-recovery-contract"))]
    pub(crate) fn recover_with_scoped_operation(
        &mut self,
        caller: &impl AuthenticatedSystemServicePrincipal,
        scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        permit: &mut Option<RecoveryOperationPermit>,
        gate: &mut impl SystemServiceRecoveryEffectGate,
    ) -> Result<RecoveryReport, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        let Some(current_permit) = permit.as_ref() else {
            return Err(IdentityError::StoreUnavailable);
        };
        let expected_binding = gate.recovery_binding().clone();
        let scope = OwnedRecoveryScopeSnapshot::capture(scope);
        let principal_id = caller.principal_id().clone();
        let caller_context_id = caller.context_id().clone();
        scope.validate_caller(&principal_id, &caller_context_id)?;
        scope
            .validate_operation_identity(current_permit.operation_id(), current_permit.binding())?;
        if current_permit.binding() != &expected_binding {
            return Err(IdentityError::StoreUnavailable);
        }
        self.authorize_recovery_with_captured_caller(principal_id, caller_context_id)?;
        self.recover_with_operation_after_authorization(permit, gate, &expected_binding)
    }

    /// Reconcile one exact durable operation after reserving the store fence
    /// and draining this issuer. The returned opaque receipt attests to the
    /// exact durable terminal record; it never reopens this issuer.
    #[cfg(test)]
    pub(crate) fn reconcile_recovery(
        &mut self,
        caller: &impl AuthenticatedSystemServicePrincipal,
        reconciliation: RecoveryReconciliation,
    ) -> Result<C1SystemServiceRecoveryTerminalReceipt, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        self.authorize_recovery(caller)?;
        self.reconcile_recovery_after_authorization(reconciliation)
    }

    #[cfg(test)]
    fn reconcile_recovery_after_authorization(
        &mut self,
        reconciliation: RecoveryReconciliation,
    ) -> Result<C1SystemServiceRecoveryTerminalReceipt, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        let (mut admission, store_permit) =
            self.prepare_recovery_reconciliation(reconciliation.clone())?;
        self.commit_prepared_recovery_reconciliation(&mut admission, store_permit, reconciliation)
    }

    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn prepare_recovery_reconciliation(
        &mut self,
        reconciliation: RecoveryReconciliation,
    ) -> Result<(IssuerRecoveryAdmissionGuard, S::ReconciliationPermit), IdentityError>
    where
        S: RecoveryJournalStore,
    {
        let admission = self.issuer.admission.begin_terminal_reconciliation()?;
        self.preflight_recovery_session_gates()?;
        let record = self
            .store
            .recovery_journal_record()
            .map_err(map_recovery_journal_error)?
            .ok_or(IdentityError::StoreUnavailable)?;
        if record.operation_id != reconciliation.operation_id()
            || &record.binding != reconciliation.binding()
        {
            return Err(IdentityError::StoreUnavailable);
        }
        match record.phase {
            RecoveryJournalPhase::Reconciled { outcome_digest }
                if outcome_digest == reconciliation.outcome_digest() =>
            {
                // Readback is still a terminal operation for this newly
                // opened issuer. Continue through store reservation and
                // permanent admission closure before reissuing the receipt.
            }
            RecoveryJournalPhase::Reconciled { .. } => return Err(IdentityError::StoreUnavailable),
            RecoveryJournalPhase::Intent
            | RecoveryJournalPhase::EffectStarted
            | RecoveryJournalPhase::OutcomeUnknown
            | RecoveryJournalPhase::RuntimeActive => {}
        }
        // Reserve the store-side terminal writer before irreversibly closing
        // issuer admission. Raw public snapshot readers do not increment the
        // issuer activity count; their contention must remain retryable.
        let store_permit = self
            .store
            .prepare_recovery_reconciliation(&reconciliation)
            .map_err(map_recovery_journal_error)?;
        Ok((admission, store_permit))
    }

    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn commit_prepared_recovery_reconciliation(
        &mut self,
        admission: &mut IssuerRecoveryAdmissionGuard,
        store_permit: S::ReconciliationPermit,
        reconciliation: RecoveryReconciliation,
    ) -> Result<C1SystemServiceRecoveryTerminalReceipt, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        admission.close_permanently(&self.issuer.alive)?;
        self.store
            .reconcile_recovery(admission, store_permit, reconciliation.clone())
            .map_err(map_recovery_journal_error)?;
        Ok(C1SystemServiceRecoveryTerminalReceipt::from_reconciliation(
            &reconciliation,
        ))
    }

    /// Verify scope, private permit, and durable reconciliation identity before
    /// the existing path can reserve the store fence or close issuer admission.
    /// The permit is supplied by C1's caller; this method never reconstructs it
    /// from the public scope metadata.
    #[cfg(all(test, feature = "system-service-recovery-contract"))]
    pub(crate) fn reconcile_recovery_with_scope(
        &mut self,
        caller: &impl AuthenticatedSystemServicePrincipal,
        scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        permit: &RecoveryOperationPermit,
        reconciliation: RecoveryReconciliation,
    ) -> Result<C1SystemServiceRecoveryTerminalReceipt, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        let scope = OwnedRecoveryScopeSnapshot::capture(scope);
        let principal_id = caller.principal_id().clone();
        let caller_context_id = caller.context_id().clone();
        scope.validate_caller(&principal_id, &caller_context_id)?;
        scope.validate_operation_identity(permit.operation_id(), permit.binding())?;
        scope
            .validate_operation_identity(reconciliation.operation_id(), reconciliation.binding())?;
        if permit.operation_id() != reconciliation.operation_id()
            || permit.binding() != reconciliation.binding()
        {
            return Err(IdentityError::StoreUnavailable);
        }
        self.authorize_recovery_with_captured_caller(principal_id, caller_context_id)?;
        self.reconcile_recovery_after_authorization(reconciliation)
    }

    /// Consume C1's exact terminal receipt by checking the durable journal
    /// again. This verifies evidence only; it never resumes a Session issuer
    /// or clears an A/C3 owner quarantine.
    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    pub(crate) fn consume_reconciliation_receipt(
        &self,
        receipt: C1SystemServiceRecoveryTerminalReceipt,
    ) -> Result<(), IdentityError>
    where
        S: RecoveryJournalStore,
    {
        let record = self
            .store
            .recovery_journal_record()
            .map_err(map_recovery_journal_error)?
            .ok_or(IdentityError::StoreUnavailable)?;
        if record.operation_id.get() != receipt.operation_id()
            || record.binding.digest_bytes() != receipt.binding_digest()
            || !matches!(
                record.phase,
                RecoveryJournalPhase::Reconciled { outcome_digest }
                    if outcome_digest == receipt.outcome_digest()
            )
        {
            return Err(IdentityError::StoreUnavailable);
        }
        Ok(())
    }

    /// Check that the opaque C1 receipt belongs to the same canonical A scope,
    /// then re-read the durable row through the existing exact operation,
    /// binding, and outcome check. Scope metadata alone never clears a fence.
    #[cfg(all(test, feature = "system-service-recovery-contract"))]
    pub(crate) fn consume_reconciliation_receipt_with_scope(
        &self,
        scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        receipt: C1SystemServiceRecoveryTerminalReceipt,
    ) -> Result<(), IdentityError>
    where
        S: RecoveryJournalStore,
    {
        let scope = OwnedRecoveryScopeSnapshot::capture(scope);
        scope.validate_operation_identity(receipt.operation_id, &receipt.binding)?;
        self.consume_reconciliation_receipt(receipt)
    }

    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn run_recovery_operation(
        &mut self,
        recovery_admission: &mut IssuerRecoveryAdmissionGuard,
        permit: &mut Option<RecoveryOperationPermit>,
        gate: &mut impl SystemServiceRecoveryEffectGate,
        expected_binding: &RecoveryBinding,
        runtime_handoff: bool,
    ) -> Result<RecoveryOperationSuccess, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        let Some(current_permit) = permit.as_ref() else {
            return Err(IdentityError::StoreUnavailable);
        };
        let binding_matches = current_permit.binding() == expected_binding;
        if !binding_matches {
            // An Intent may already be durable. A changed gate binding after
            // that point must close this issuer and quarantine the exact
            // operation rather than return with the old Session still live.
            self.close_after_uncertain_recovery_claim(recovery_admission, permit);
            return Err(IdentityError::StoreUnavailable);
        }
        let operation_id = current_permit.operation_id();
        let record = match self.store.recovery_journal_record() {
            Ok(record) => record,
            Err(RecoveryJournalError::Busy) => return Err(IdentityError::SessionLeaseBusy),
            Err(_error) => {
                self.close_after_uncertain_recovery_claim(recovery_admission, permit);
                return Err(IdentityError::StoreUnavailable);
            }
        };
        match record {
            Some(record)
                if record.operation_id == operation_id
                    && &record.binding == expected_binding
                    && matches!(record.phase, RecoveryJournalPhase::Intent) => {}
            Some(record)
                if matches!(
                    record.phase,
                    RecoveryJournalPhase::EffectStarted
                        | RecoveryJournalPhase::OutcomeUnknown
                        | RecoveryJournalPhase::RuntimeActive
                ) =>
            {
                self.close_after_uncertain_recovery_claim(recovery_admission, permit);
                return Err(IdentityError::StoreUnavailable);
            }
            _ => return Err(IdentityError::StoreUnavailable),
        }

        let claim_attempt = catch_unwind(AssertUnwindSafe(|| {
            self.store
                .claim_recovery_effect(permit.as_ref().expect("permit checked before claim"))
        }));
        let claim = match claim_attempt {
            Ok(Ok(claim)) => claim,
            // Busy is the only claim error that the store contract guarantees
            // is reported before mutation, so the same permit remains safe to
            // retry. Every other error may follow an atomic rename whose
            // durability is unknown; permanently close C1 and consume it.
            Ok(Err(RecoveryJournalError::Busy)) => return Err(IdentityError::SessionLeaseBusy),
            Ok(Err(_error)) => {
                self.close_after_uncertain_recovery_claim(recovery_admission, permit);
                return Err(IdentityError::StoreUnavailable);
            }
            Err(payload) => {
                self.close_after_uncertain_recovery_claim(recovery_admission, permit);
                // A panic payload has arbitrary user code in Drop. Do not let
                // it panic while returning through the caller's root-cleanup
                // path after issuer admission has been closed.
                std::mem::forget(payload);
                return Err(IdentityError::StoreUnavailable);
            }
        };
        let operation_identity =
            match C1RecoveryOperationIdentity::from_effect_claim(&claim, expected_binding) {
                Some(identity) => identity,
                None => {
                    self.close_after_uncertain_recovery_claim(recovery_admission, permit);
                    return Err(IdentityError::StoreUnavailable);
                }
            };
        // Once the durable claim succeeds, the journal itself prevents replay
        // and the in-memory permit is consumed. Permanently close old C1
        // identities before calling any fallible owner gate; the store's
        // current-snapshot reader is not itself an issuer fence.
        drop(permit.take().ok_or(IdentityError::StoreUnavailable)?);
        self.recovered = false;
        if recovery_admission
            .close_permanently(&self.issuer.alive)
            .is_err()
        {
            self.mark_recovery_outcome_unknown_safely(claim.operation_id(), claim.binding());
            return Err(IdentityError::StoreUnavailable);
        }
        let effect_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            gate.apply_recovery_effect(operation_identity, || {
                let recovery = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.recover_inner_authorized()
                }));
                match recovery {
                    Ok(Ok(report)) => {
                        if self.store.commit_recovery_runtime_active(&claim).is_ok() {
                            Ok(report)
                        } else {
                            self.recovered = false;
                            self.mark_recovery_outcome_unknown_safely(
                                claim.operation_id(),
                                claim.binding(),
                            );
                            Err(IdentityError::StoreUnavailable)
                        }
                    }
                    Ok(Err(_error)) => {
                        self.recovered = false;
                        self.mark_recovery_outcome_unknown_safely(
                            claim.operation_id(),
                            claim.binding(),
                        );
                        // After the durable EffectStarted claim, even a
                        // SessionLeaseBusy from a lower gate is an uncertain
                        // effect outcome. Never expose it as retryable
                        // pre-effect Busy.
                        Err(IdentityError::StoreUnavailable)
                    }
                    Err(payload) => {
                        self.recovered = false;
                        self.mark_recovery_outcome_unknown_safely(
                            claim.operation_id(),
                            claim.binding(),
                        );
                        // Panic payloads are arbitrary user values. Forget
                        // the caught payload so a panicking payload Drop
                        // cannot turn this contained failure into a second
                        // panic while the storage owner is cleaning up roots.
                        std::mem::forget(payload);
                        Err(IdentityError::StoreUnavailable)
                    }
                }
            })
        }));

        match effect_result {
            Ok(Ok(Ok(report))) => {
                if !runtime_handoff {
                    // The legacy one-shot entrypoint cannot transfer a C1
                    // issuer to A's retained C3 runtime. Keep it fail-closed;
                    // only the dedicated runtime handoff path may replace the
                    // closed issuer after exact durable readback.
                    self.recovered = false;
                    return Err(IdentityError::StoreUnavailable);
                }
                Ok(RecoveryOperationSuccess {
                    #[cfg(all(
                        feature = "system-service-recovery-contract",
                        target_os = "linux",
                        target_arch = "x86_64"
                    ))]
                    operation: operation_identity,
                    #[cfg(all(
                        feature = "system-service-recovery-contract",
                        target_os = "linux",
                        target_arch = "x86_64"
                    ))]
                    operation_id: claim.operation_id(),
                    #[cfg(all(
                        feature = "system-service-recovery-contract",
                        target_os = "linux",
                        target_arch = "x86_64"
                    ))]
                    binding: claim.binding().clone(),
                    report,
                })
            }
            Ok(Ok(Err(_))) => {
                // The supplied callback already converted its post-claim
                // failure to OutcomeUnknown (or retained the blocking
                // EffectStarted row if that best-effort write failed).
                Err(IdentityError::StoreUnavailable)
            }
            Ok(Err(_)) => {
                self.recovered = false;
                self.mark_recovery_outcome_unknown_safely(claim.operation_id(), claim.binding());
                Err(IdentityError::StoreUnavailable)
            }
            Err(payload) => {
                self.recovered = false;
                self.mark_recovery_outcome_unknown_safely(claim.operation_id(), claim.binding());
                std::mem::forget(payload);
                Err(IdentityError::StoreUnavailable)
            }
        }
    }

    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn mark_recovery_outcome_unknown_safely(
        &mut self,
        operation_id: RecoveryOperationId,
        binding: &RecoveryBinding,
    ) where
        S: RecoveryJournalStore,
    {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
            self.store
                .mark_recovery_outcome_unknown(operation_id, binding)
        })) {
            // The durable EffectStarted/Intent record remains quarantining if
            // this best-effort refinement fails. Do not let an arbitrary
            // store panic payload skip issuer/root cleanup.
            std::mem::forget(payload);
        }
    }

    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn close_after_uncertain_recovery_claim(
        &mut self,
        recovery_admission: &mut IssuerRecoveryAdmissionGuard,
        permit: &mut Option<RecoveryOperationPermit>,
    ) where
        S: RecoveryJournalStore,
    {
        let pending = permit.take();
        self.recovered = false;
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
            recovery_admission.close_permanently(&self.issuer.alive)
        })) {
            // Closure is published before deferred destructors run. Their
            // panic must not skip root cleanup in the caller.
            std::mem::forget(payload);
        }
        if let Some(pending) = pending {
            let operation_id = pending.operation_id();
            let binding = pending.binding().clone();
            drop(pending);
            self.mark_recovery_outcome_unknown_safely(operation_id, &binding);
        }
    }

    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn close_after_uncertain_recovery_intent(
        &mut self,
        recovery_admission: &mut IssuerRecoveryAdmissionGuard,
    ) {
        self.recovered = false;
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
            recovery_admission.close_permanently(&self.issuer.alive)
        })) {
            // Closing publishes the permanent fence before deferred store
            // destruction. Do not let an arbitrary destructor panic prevent
            // the paired storage adapter from revoking its roots.
            std::mem::forget(payload);
        }
    }

    /// Verify that all extant Session writer gates can be acquired before an
    /// intent is staged. C1 admission is reserved with `active == 0`, so no
    /// new callback can race between this preflight and the durable claim.
    /// Partial acquisitions are dropped automatically on contention.
    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn preflight_recovery_session_gates(&self) -> Result<(), IdentityError> {
        #[cfg(test)]
        RECOVERY_WRITE_GUARDS_ACQUIRED.with(|count| count.set(0));
        let leases = self.leases.values().cloned().collect::<Vec<_>>();
        let mut guards = Vec::with_capacity(leases.len());
        for lease in &leases {
            let guard = match lease.live.try_write() {
                Ok(guard) => guard,
                Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    return Err(IdentityError::SessionLeaseBusy);
                }
            };
            guards.push(guard);
            #[cfg(test)]
            RECOVERY_WRITE_GUARDS_ACQUIRED.with(|count| count.set(count.get() + 1));
        }
        drop(guards);
        Ok(())
    }

    #[cfg(any(test, feature = "system-service-recovery-contract"))]
    fn recover_inner_authorized(&mut self) -> Result<RecoveryReport, IdentityError> {
        let store_state = self.store.load_current_state().map_err(map_store_error)?;
        let current_version = store_state.high_water_version;
        if store_state
            .current
            .as_ref()
            .is_some_and(|snapshot| current_version != Some(snapshot.version))
        {
            return Err(IdentityError::StoreUnavailable);
        }
        let current = store_state.current.map(|snapshot| snapshot.bytes);
        let last_known_good = self.store.load_last_known_good().map_err(map_store_error)?;
        if last_known_good.as_ref().is_some_and(|snapshot| {
            current_version.is_none_or(|high_water| snapshot.version.get() > high_water.get())
        }) {
            return Err(IdentityError::StoreUnavailable);
        }
        // Keep the cached snapshot and its CAS version paired on every failure.
        let (mut candidate, source) = match current {
            Some(bytes) => match self.read_valid_snapshot(&bytes) {
                Ok(snapshot) => (snapshot, RecoverySource::Current),
                Err(_) => match last_known_good {
                    Some(snapshot) => match self.read_valid_snapshot(&snapshot.bytes) {
                        Ok(snapshot) => (snapshot, RecoverySource::LastKnownGood),
                        Err(_) => {
                            self.emit(
                                IdentityEventKind::CorruptStateRejected,
                                None,
                                None,
                                None,
                                Some(IdentityEventReason::Corrupt),
                            );
                            return Err(IdentityError::CorruptState);
                        }
                    },
                    None => {
                        self.emit(
                            IdentityEventKind::CorruptStateRejected,
                            None,
                            None,
                            None,
                            Some(IdentityEventReason::Corrupt),
                        );
                        return Err(IdentityError::CorruptState);
                    }
                },
            },
            None => match last_known_good {
                Some(snapshot) => (
                    self.read_valid_snapshot(&snapshot.bytes)?,
                    RecoverySource::LastKnownGood,
                ),
                None if current_version.is_none() => {
                    (IdentitySnapshot::empty(), RecoverySource::Empty)
                }
                None => {
                    self.emit(
                        IdentityEventKind::CorruptStateRejected,
                        None,
                        None,
                        None,
                        Some(IdentityEventReason::Corrupt),
                    );
                    return Err(IdentityError::CorruptState);
                }
            },
        };

        // Any LKG fallback is weaker evidence than a valid current payload,
        // including an equal-version backup. Persist the loss marker so the
        // fail-closed allocation rule survives subsequent restarts.
        if source == RecoverySource::LastKnownGood {
            candidate.profile_id_history_complete = false;
        }

        let now = self.clock.now();
        let mut ended = 0;
        for session in &mut candidate.sessions {
            if matches!(session.lifecycle, SessionLifecycle::Active) {
                session.lifecycle = SessionLifecycle::Ended {
                    at: now,
                    reason: SessionEndReason::SystemRestart,
                };
                if session.kind == SessionKind::Guest {
                    session.guest_cleanup = GuestCleanupStatus::Pending;
                }
                ended += 1;
            }
        }
        let rewrite_snapshot = source != RecoverySource::Current || ended != 0;
        if rewrite_snapshot {
            candidate.revision = candidate
                .revision
                .checked_add(1)
                .ok_or(IdentityError::RevisionExhausted)?;
        }
        let guest_ids = candidate
            .sessions
            .iter()
            .filter(|session| {
                session.kind == SessionKind::Guest
                    && session.guest_cleanup == GuestCleanupStatus::Pending
            })
            .map(|session| session.profile_id.clone())
            .collect::<Vec<_>>();

        let lease_arcs = self.leases.values().cloned().collect::<Vec<_>>();
        let mut lease_guards = Vec::with_capacity(lease_arcs.len());
        for lease in &lease_arcs {
            let guard = match lease.live.try_write() {
                Ok(guard) => guard,
                Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    return Err(IdentityError::SessionLeaseBusy);
                }
            };
            lease_guards.push(guard);
        }
        if rewrite_snapshot {
            self.save_candidate_at_version(candidate, current_version)?;
        } else {
            self.snapshot = candidate;
            self.store_version = current_version;
        }
        for (lease, guard) in lease_arcs.iter().zip(&mut lease_guards) {
            lease.revoke(guard);
        }
        drop(lease_guards);
        for (caller_key, session_id) in self.active_by_caller.drain() {
            self.last_by_caller.insert(caller_key, session_id);
        }
        self.leases.clear();
        self.session_owners.clear();
        self.idempotency.clear();
        self.recovered = false;
        self.recovered = true;

        if source == RecoverySource::LastKnownGood {
            self.emit(
                IdentityEventKind::RecoveryUsedLastKnownGood,
                None,
                None,
                None,
                Some(IdentityEventReason::Recovery),
            );
        }

        let mut pending = 0;
        for profile_id in guest_ids {
            if self
                .guest_cleaner
                .cleanup_guest_profile(&profile_id)
                .is_ok()
            {
                let mut next = self.snapshot.clone();
                if let Some(session) = next.sessions.iter_mut().find(|s| {
                    s.profile_id == profile_id
                        && s.kind == SessionKind::Guest
                        && s.guest_cleanup == GuestCleanupStatus::Pending
                }) {
                    session.guest_cleanup = GuestCleanupStatus::Complete;
                    next.profiles.retain(|p| p.profile_id != profile_id);
                    next.revision = self.next_revision()?;
                    if self.save_candidate(next).is_err() {
                        pending += 1;
                    }
                }
            } else {
                pending += 1;
                self.emit(
                    IdentityEventKind::GuestCleanupFailed,
                    None,
                    Some(profile_id),
                    None,
                    Some(IdentityEventReason::CleanupPending),
                );
            }
        }

        Ok(RecoveryReport {
            source,
            interrupted_sessions_ended: ended,
            guest_cleanups_pending: pending,
        })
    }

    /// Check the exact verified boot principal's first-owner permission before
    /// inspecting a possibly resumable persisted mapping.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        feature = "linux-interactive-account",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    pub(crate) fn authorize_first_owner_provisioning(
        &mut self,
        caller: &impl AuthenticatedProvisioningCaller,
    ) -> Result<(), IdentityError> {
        self.ensure_recovered()?;
        self.authorize(caller, IdentityAction::CreateLocalUser, None, None, None)
    }

    /// Create a local user and its first profile. Returns
    /// ProfileHistoryIncomplete when restored state cannot prove complete
    /// ProfileId history; this crate has no repair API for that condition.
    pub fn create_local_user(
        &mut self,
        caller: &impl AuthenticatedProvisioningCaller,
        user_metadata: LocalUserMetadata,
        profile_metadata: ProfileMetadata,
    ) -> Result<(UserId, ProfileId), IdentityError> {
        self.ensure_recovered()?;
        self.create_local_user_after_admission(caller, user_metadata, profile_metadata)
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    pub(crate) fn create_local_user_with_runtime_admission(
        &mut self,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
        caller: &impl AuthenticatedProvisioningCaller,
        user_metadata: LocalUserMetadata,
        profile_metadata: ProfileMetadata,
    ) -> Result<(UserId, ProfileId), IdentityError>
    where
        S: RecoveryJournalStore,
    {
        self.validate_system_service_runtime_admission(admission)?;
        self.create_local_user_after_admission(caller, user_metadata, profile_metadata)
    }

    fn create_local_user_after_admission(
        &mut self,
        caller: &impl AuthenticatedProvisioningCaller,
        user_metadata: LocalUserMetadata,
        profile_metadata: ProfileMetadata,
    ) -> Result<(UserId, ProfileId), IdentityError> {
        user_metadata
            .validate()
            .map_err(|_| IdentityError::InvalidMetadata)?;
        profile_metadata
            .validate()
            .map_err(|_| IdentityError::InvalidMetadata)?;
        self.authorize(caller, IdentityAction::CreateLocalUser, None, None, None)?;
        self.ensure_profile_id_history_complete()?;

        let user_id = self
            .ids
            .next_user_id()
            .map_err(|_| IdentityError::IdSourceUnavailable)?;
        let profile_id = self
            .ids
            .next_profile_id()
            .map_err(|_| IdentityError::IdSourceUnavailable)?;
        if self
            .snapshot
            .users
            .iter()
            .any(|user| user.user_id == user_id)
            || self.profile_id_is_reserved(&profile_id)
        {
            return Err(IdentityError::IdCollision);
        }

        let mut candidate = self.snapshot.clone();
        candidate.revision = self.next_revision()?;
        candidate.users.push(LocalUser {
            schema_version: IDENTITY_SCHEMA_VERSION,
            user_id,
            created_at: self.clock.now(),
            metadata: user_metadata,
        });
        candidate.profiles.push(ProfileRecord {
            schema_version: IDENTITY_SCHEMA_VERSION,
            profile_id: profile_id.clone(),
            owner_user_id: Some(user_id),
            kind: ProfileKind::Persistent,
            created_at: self.clock.now(),
            metadata: profile_metadata,
        });
        self.save_candidate(candidate)?;
        self.emit(
            IdentityEventKind::LocalUserCreated,
            Some(user_id),
            Some(profile_id.clone()),
            None,
            None,
        );
        Ok((user_id, profile_id))
    }

    /// A user may own multiple profiles. Each session binds to exactly one;
    /// this API does not assume a profile-selection UI. Returns
    /// ProfileHistoryIncomplete when restored state cannot prove complete
    /// ProfileId history; this crate has no repair API for that condition.
    pub fn create_profile(
        &mut self,
        caller: &impl AuthenticatedProvisioningCaller,
        user_id: UserId,
        metadata: ProfileMetadata,
    ) -> Result<ProfileId, IdentityError> {
        self.ensure_recovered()?;
        metadata
            .validate()
            .map_err(|_| IdentityError::InvalidMetadata)?;
        self.authorize(
            caller,
            IdentityAction::CreateProfile,
            Some(user_id),
            None,
            None,
        )?;
        if !self
            .snapshot
            .users
            .iter()
            .any(|user| user.user_id == user_id)
        {
            return Err(IdentityError::UserNotFound);
        }
        self.ensure_profile_id_history_complete()?;
        let profile_id = self
            .ids
            .next_profile_id()
            .map_err(|_| IdentityError::IdSourceUnavailable)?;
        if self.profile_id_is_reserved(&profile_id) {
            return Err(IdentityError::IdCollision);
        }
        let mut candidate = self.snapshot.clone();
        candidate.revision = self.next_revision()?;
        candidate.profiles.push(ProfileRecord {
            schema_version: IDENTITY_SCHEMA_VERSION,
            profile_id: profile_id.clone(),
            owner_user_id: Some(user_id),
            kind: ProfileKind::Persistent,
            created_at: self.clock.now(),
            metadata,
        });
        self.save_candidate(candidate)?;
        self.emit(
            IdentityEventKind::ProfileCreated,
            Some(user_id),
            Some(profile_id.clone()),
            None,
            None,
        );
        Ok(profile_id)
    }

    /// Issue a local Session only after the trusted integration has completed
    /// its selected account authentication and supplied the authenticated
    /// interactive peer context. C1 chooses neither the credential method nor
    /// the expiry duration; the runtime adapter is responsible for proving the
    /// configured account record and binding it to the persisted User/Profile.
    pub fn login_local(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
        user_id: UserId,
        profile_id: ProfileId,
        expires_at: Option<UnixMillis>,
        idempotency_key: IdempotencyKey,
    ) -> Result<SessionId, IdentityError> {
        self.ensure_recovered()?;
        self.login_local_after_admission(caller, user_id, profile_id, expires_at, idempotency_key)
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    pub(crate) fn login_local_with_runtime_admission(
        &mut self,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
        caller: &impl AuthenticatedInteractiveCaller,
        user_id: UserId,
        profile_id: ProfileId,
        expires_at: Option<UnixMillis>,
        idempotency_key: IdempotencyKey,
    ) -> Result<SessionId, IdentityError>
    where
        S: RecoveryJournalStore,
    {
        self.validate_system_service_runtime_admission(admission)?;
        self.login_local_after_admission_with(
            caller,
            user_id,
            profile_id,
            expires_at,
            idempotency_key,
            |service, committed_caller, request| {
                service.authorize_committed_session_with_runtime_admission(
                    committed_caller,
                    request,
                    admission,
                )
            },
        )
    }

    fn login_local_after_admission(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
        user_id: UserId,
        profile_id: ProfileId,
        expires_at: Option<UnixMillis>,
        idempotency_key: IdempotencyKey,
    ) -> Result<SessionId, IdentityError> {
        self.login_local_after_admission_with(
            caller,
            user_id,
            profile_id,
            expires_at,
            idempotency_key,
            |service, _, request| service.authorize_request(request),
        )
    }

    fn login_local_after_admission_with<F>(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
        user_id: UserId,
        profile_id: ProfileId,
        expires_at: Option<UnixMillis>,
        idempotency_key: IdempotencyKey,
        authorize_committed_session: F,
    ) -> Result<SessionId, IdentityError>
    where
        F: FnOnce(
            &mut Self,
            &dyn AuthenticatedInteractiveCaller,
            &AuthorizationRequest,
        ) -> Result<(), IdentityError>,
    {
        self.authorize(
            caller,
            IdentityAction::LoginLocalUser,
            Some(user_id),
            Some(profile_id.clone()),
            None,
        )?;
        self.create_session(
            caller,
            SessionIntent {
                kind: SessionKind::LocalUser,
                user_id: Some(user_id),
                profile_id,
                expires_at,
            },
            idempotency_key,
            IdentityAction::CreateLocalSession,
            authorize_committed_session,
        )
    }

    /// Create a guest profile and session. Returns ProfileHistoryIncomplete
    /// when restored state cannot prove complete ProfileId history; this crate
    /// has no repair API for that condition.
    pub fn create_guest_session(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
        expires_at: Option<UnixMillis>,
        idempotency_key: IdempotencyKey,
    ) -> Result<SessionId, IdentityError> {
        self.ensure_recovered()?;
        if expires_at.is_some_and(|expires| expires <= self.clock.now()) {
            return Err(IdentityError::ExpiryNotInFuture);
        }
        self.authorize(caller, IdentityAction::CreateGuestSession, None, None, None)?;
        let caller_key = CallerKey::from_trusted(caller);
        let idempotency_scope = (caller_key.clone(), idempotency_key.clone());
        if let Some(existing) = self.idempotency.get(&idempotency_scope) {
            let same_guest_request = existing.intent.kind == SessionKind::Guest
                && existing.intent.user_id.is_none()
                && existing.intent.expires_at == expires_at;
            if same_guest_request {
                return Ok(existing.session_id.clone());
            }
            return Err(IdentityError::IdempotencyConflict);
        }
        self.ensure_profile_id_history_complete()?;
        let profile_id = self
            .ids
            .next_profile_id()
            .map_err(|_| IdentityError::IdSourceUnavailable)?;
        if self.profile_id_is_reserved(&profile_id) {
            return Err(IdentityError::IdCollision);
        }
        let intent = SessionIntent {
            kind: SessionKind::Guest,
            user_id: None,
            profile_id: profile_id.clone(),
            expires_at,
        };
        self.create_session(
            caller,
            intent,
            idempotency_key,
            IdentityAction::CreateGuestSession,
            |service, _, request| service.authorize_request(request),
        )
    }

    fn create_session<F>(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
        intent: SessionIntent,
        idempotency_key: IdempotencyKey,
        action: IdentityAction,
        authorize_committed_session: F,
    ) -> Result<SessionId, IdentityError>
    where
        F: FnOnce(
            &mut Self,
            &dyn AuthenticatedInteractiveCaller,
            &AuthorizationRequest,
        ) -> Result<(), IdentityError>,
    {
        let now = self.clock.now();
        if intent.expires_at.is_some_and(|expires| expires <= now) {
            return Err(IdentityError::ExpiryNotInFuture);
        }

        let caller_key = CallerKey::from_trusted(caller);
        let idempotency_scope = (caller_key.clone(), idempotency_key.clone());
        if let Some(existing) = self.idempotency.get(&idempotency_scope) {
            if existing.intent != intent {
                return Err(IdentityError::IdempotencyConflict);
            }
            return Ok(existing.session_id.clone());
        }
        if let Some(current) = self.active_by_caller.get(&caller_key).cloned() {
            if let Some(existing) = self
                .snapshot
                .sessions
                .iter()
                .find(|session| session.session_id == current)
                .cloned()
            {
                if matches!(existing.lifecycle, SessionLifecycle::Active)
                    && existing.expires_at.is_none_or(|expires| now < expires)
                {
                    return Err(IdentityError::CallerAlreadyHasSession);
                }
                if matches!(existing.lifecycle, SessionLifecycle::Active) {
                    self.authorize(
                        caller,
                        IdentityAction::EndSession,
                        existing.user_id,
                        Some(existing.profile_id.clone()),
                        Some(existing.session_id.clone()),
                    )?;
                    self.end_record(&caller_key, &existing.session_id, SessionEndReason::Expired)?;
                } else {
                    self.active_by_caller.remove(&caller_key);
                }
            } else {
                return Err(IdentityError::CorruptState);
            }
        }

        match intent.kind {
            SessionKind::LocalUser => {
                let user_id = intent.user_id.ok_or(IdentityError::UserNotFound)?;
                if !self
                    .snapshot
                    .users
                    .iter()
                    .any(|user| user.user_id == user_id)
                {
                    return Err(IdentityError::UserNotFound);
                }
                let profile = self
                    .snapshot
                    .profiles
                    .iter()
                    .find(|profile| profile.profile_id == intent.profile_id)
                    .ok_or(IdentityError::ProfileNotFound)?;
                if profile.owner_user_id != Some(user_id) || profile.kind != ProfileKind::Persistent
                {
                    return Err(IdentityError::ProfileOwnershipMismatch);
                }
            }
            SessionKind::Guest => {
                if intent.user_id.is_some() {
                    return Err(IdentityError::ProfileOwnershipMismatch);
                }
            }
        }
        self.authorize(
            caller,
            action,
            intent.user_id,
            Some(intent.profile_id.clone()),
            None,
        )?;

        if self.snapshot.sessions.iter().any(|session| {
            session.profile_id == intent.profile_id
                && matches!(session.lifecycle, SessionLifecycle::Active)
        }) {
            return Err(IdentityError::ProfileAlreadyActive);
        }

        let next_store_generation = self
            .store_version
            .and_then(|version| version.get().checked_add(1))
            .ok_or(IdentityError::StoreUnavailable)?;
        let session_nonce = self
            .ids
            .next_session_id()
            .map_err(|_| IdentityError::IdSourceUnavailable)?;
        let session_id = SessionId::in_store_generation(next_store_generation, &session_nonce)
            .map_err(|_| IdentityError::IdSourceUnavailable)?;
        if self
            .snapshot
            .sessions
            .iter()
            .any(|session| session.session_id == session_id)
        {
            return Err(IdentityError::IdCollision);
        }
        // Consume a fresh process-local generation before persistence. Failed
        // commits may skip a generation, but it is never reused in-process.
        let lease_generation = next_session_lease_generation()?;

        let mut candidate = self.snapshot.clone();
        candidate.revision = self.next_revision()?;
        if intent.kind == SessionKind::Guest {
            candidate.profiles.push(ProfileRecord {
                schema_version: IDENTITY_SCHEMA_VERSION,
                profile_id: intent.profile_id.clone(),
                owner_user_id: None,
                kind: ProfileKind::GuestEphemeral,
                created_at: now,
                metadata: ProfileMetadata::new("Guest", None)
                    .map_err(|_| IdentityError::InvalidMetadata)?,
            });
        }
        candidate.sessions.push(SessionRecord {
            schema_version: IDENTITY_SCHEMA_VERSION,
            session_id: session_id.clone(),
            kind: intent.kind,
            user_id: intent.user_id,
            profile_id: intent.profile_id.clone(),
            created_at: now,
            expires_at: intent.expires_at,
            lifecycle: SessionLifecycle::Active,
            guest_cleanup: if intent.kind == SessionKind::Guest {
                GuestCleanupStatus::Pending
            } else {
                GuestCleanupStatus::NotApplicable
            },
        });
        self.save_candidate(candidate)?;
        self.session_owners
            .insert(session_id.clone(), caller_key.clone());

        // The saved session is not authority. Revalidate after creation before
        // publishing it as current to this trusted caller context.
        let resolve_request = Self::authorization_request(
            caller,
            IdentityAction::ResolveCurrentIdentity,
            intent.user_id,
            Some(intent.profile_id.clone()),
            Some(session_id.clone()),
        );
        if let Err(error) = authorize_committed_session(self, caller, &resolve_request) {
            let reason = SessionEndReason::Recovery;
            self.end_record(&caller_key, &session_id, reason)?;
            return Err(error);
        }

        self.active_by_caller
            .insert(caller_key.clone(), session_id.clone());
        self.last_by_caller
            .insert(caller_key.clone(), session_id.clone());
        self.leases.insert(
            session_id.clone(),
            Arc::new(SessionLeaseState::new(lease_generation)),
        );
        self.idempotency.insert(
            idempotency_scope,
            IdempotentSession {
                session_id: session_id.clone(),
                intent: intent.clone(),
            },
        );
        self.emit(
            if intent.kind == SessionKind::Guest {
                IdentityEventKind::GuestSessionCreated
            } else {
                IdentityEventKind::LocalSessionCreated
            },
            intent.user_id,
            Some(intent.profile_id),
            Some(session_id.clone()),
            None,
        );
        Ok(session_id)
    }

    /// End the current durable Session. If an admitted effect holds its lease,
    /// this returns `SessionLeaseBusy` without committing; retry after the
    /// effect has returned. When the Session opened profile roots, use
    /// `end_session_with_storage` to release those adapter-owned roots after
    /// the durable transition succeeds.
    pub fn end_session(
        &mut self,
        caller: &(impl AuthenticatedInteractiveCaller + ?Sized),
        session_id: &SessionId,
    ) -> Result<EndSessionResult, IdentityError> {
        self.ensure_recovered()?;
        self.end_session_after_admission(caller, session_id, SessionEndReason::UserLogout)
    }

    /// End one issued interactive Session while its exact system-service
    /// runtime operation is still active. A `RuntimeActive` journal row is
    /// intentionally quarantined from ordinary entry points, so this path
    /// requires the same non-cloneable C1 admission used by the live owner
    /// callback. The admission does not replace the interactive caller's
    /// normal `EndSession` authorization or the exact caller/Session ownership
    /// checks below. Call this outside any C1 session-effect callback and
    /// outside a C2 owner gate; a held effect returns `SessionLeaseBusy` before
    /// the durable end is committed.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    pub(crate) fn end_session_with_runtime_admission(
        &mut self,
        caller: &(impl AuthenticatedInteractiveCaller + ?Sized),
        session_id: &SessionId,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<EndSessionResult, IdentityError>
    where
        S: C1SystemServiceRecoveryJournal,
    {
        self.validate_system_service_runtime_admission(admission)?;
        self.end_session_after_admission(caller, session_id, SessionEndReason::UserLogout)
    }

    /// Roll back the exact first-login Session owned by C1's still-pending
    /// C2 attachment transaction. Unlike ordinary `end_session`, this path is
    /// valid while the caller holds the same RuntimeActive admission that
    /// committed the Session. It does not acquire or re-enter either owner's
    /// gate; the pending C1 transaction supplies the sealed caller and exact
    /// SessionId, and normal EndSession authorization plus the Session lease's
    /// nonblocking write admission still apply.
    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    pub(crate) fn rollback_pending_initial_session_with_runtime_admission(
        &mut self,
        caller: &(impl AuthenticatedInteractiveCaller + ?Sized),
        session_id: &SessionId,
        operation_identity: C1RecoveryOperationIdentity,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<EndSessionResult, IdentityError>
    where
        S: C1SystemServiceRecoveryJournal,
    {
        self.validate_system_service_runtime_admission(admission)?;
        if admission.operation_identity() != operation_identity {
            return Err(IdentityError::StoreUnavailable);
        }
        self.end_session_after_admission(caller, session_id, SessionEndReason::UserLogout)
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        feature = "linux-interactive-account",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    pub(crate) fn revoke_session_with_runtime_admission(
        &mut self,
        caller: &(impl AuthenticatedInteractiveCaller + ?Sized),
        session_id: &SessionId,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<EndSessionResult, IdentityError>
    where
        S: C1SystemServiceRecoveryJournal,
    {
        self.validate_system_service_runtime_admission(admission)?;
        self.end_session_after_admission(caller, session_id, SessionEndReason::Revoked)
    }

    fn end_session_after_admission(
        &mut self,
        caller: &(impl AuthenticatedInteractiveCaller + ?Sized),
        session_id: &SessionId,
        reason: SessionEndReason,
    ) -> Result<EndSessionResult, IdentityError> {
        let session = self
            .snapshot
            .sessions
            .iter()
            .find(|session| &session.session_id == session_id)
            .ok_or(IdentityError::SessionNotFound)?;
        self.authorize(
            caller,
            IdentityAction::EndSession,
            session.user_id,
            Some(session.profile_id.clone()),
            Some(session_id.clone()),
        )?;
        let caller_key = CallerKey::from_trusted(caller);
        if self.active_by_caller.get(&caller_key) != Some(session_id) {
            if self.session_owners.get(session_id) != Some(&caller_key) {
                return Err(IdentityError::SessionNotCurrent);
            }
            let active = self
                .snapshot
                .sessions
                .iter()
                .find(|session| &session.session_id == session_id)
                .is_some_and(|session| matches!(session.lifecycle, SessionLifecycle::Active));
            if active {
                // A post-commit rollback can fail before this session is ever
                // published in active_by_caller or receives a live lease.
                // The owner may retry durable cancellation, but the lease-less
                // record is never accepted as current authority.
                return self.end_record(&caller_key, session_id, reason);
            }
            if let Some(profile_id) = self
                .snapshot
                .sessions
                .iter()
                .find(|session| {
                    &session.session_id == session_id
                        && session.kind == SessionKind::Guest
                        && session.guest_cleanup == GuestCleanupStatus::Pending
                })
                .map(|session| session.profile_id.clone())
            {
                return self.finish_guest_cleanup(session_id, &profile_id);
            }
            return Ok(EndSessionResult::AlreadyEnded);
        }
        self.end_record(&caller_key, session_id, reason)
    }

    /// End one durable Session and then synchronously release only its root
    /// handles. The owner check happens before the durable operation; C1 lease
    /// contention returns `SessionLeaseBusy` without committing or cleaning,
    /// so the caller may retry after the active effect exits.
    pub fn end_session_with_storage<B, SE>(
        &mut self,
        caller: &(impl AuthenticatedInteractiveCaller + ?Sized),
        session_id: &SessionId,
        storage: &mut UserStorageRootAdapter<B, SE>,
    ) -> Result<EndSessionResult, IdentityError>
    where
        B: ProfileStorageBackend,
        SE: StorageRootEventSink,
    {
        match self
            .issuer
            .accepts_storage_adapter(storage.adapter_identity())
        {
            Ok(true) => {}
            Ok(false) => return Err(IdentityError::StorageOwnerMismatch),
            Err(error) => return Err(error.into_identity_error()),
        }
        let result = self.end_session(caller, session_id)?;
        storage.revoke_session_for_issuer(&self.issuer.alive, session_id);
        Ok(result)
    }

    /// Resolve the canonical capability principal for the current identity.
    /// A mapping result is not an authorization decision or a grant.
    pub fn capability_principal_for_current(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
        adapter: &impl CapabilityPrincipalAdapter,
    ) -> Result<PrincipalId, PrincipalMappingFailure> {
        let identity = match self.resolve_current_identity(caller) {
            Ok(identity) => identity,
            Err(error) => {
                let mapping_error = match error {
                    ResolutionError::NoSession => PrincipalMappingFailure::MissingIdentity,
                    ResolutionError::Ended | ResolutionError::Expired => {
                        PrincipalMappingFailure::StaleSession
                    }
                    ResolutionError::CorruptState => PrincipalMappingFailure::CorruptState,
                    ResolutionError::Unavailable => PrincipalMappingFailure::Unavailable,
                    ResolutionError::Denied => PrincipalMappingFailure::Denied,
                };
                self.emit(
                    IdentityEventKind::PrincipalMappingDenied,
                    None,
                    None,
                    None,
                    Some(match mapping_error {
                        PrincipalMappingFailure::MissingIdentity => IdentityEventReason::NoSession,
                        PrincipalMappingFailure::StaleSession => IdentityEventReason::Ended,
                        PrincipalMappingFailure::CorruptState => IdentityEventReason::Corrupt,
                        PrincipalMappingFailure::Denied => IdentityEventReason::Denied,
                        PrincipalMappingFailure::Unavailable => IdentityEventReason::Unavailable,
                    }),
                );
                return Err(mapping_error);
            }
        };
        match crate::resolve_capability_principal(adapter, &identity) {
            Ok(principal) => {
                self.emit(
                    IdentityEventKind::PrincipalMappingAllowed,
                    identity.user_id(),
                    Some(identity.profile_id().clone()),
                    Some(identity.session_id().clone()),
                    None,
                );
                Ok(principal)
            }
            Err(error) => {
                let reason = match error {
                    PrincipalMappingFailure::MissingIdentity => IdentityEventReason::NoSession,
                    PrincipalMappingFailure::StaleSession => IdentityEventReason::Ended,
                    PrincipalMappingFailure::CorruptState => IdentityEventReason::Corrupt,
                    PrincipalMappingFailure::Denied => IdentityEventReason::Denied,
                    PrincipalMappingFailure::Unavailable => IdentityEventReason::Unavailable,
                };
                self.emit(
                    IdentityEventKind::PrincipalMappingDenied,
                    identity.user_id(),
                    Some(identity.profile_id().clone()),
                    Some(identity.session_id().clone()),
                    Some(reason),
                );
                Err(error)
            }
        }
    }

    fn end_record(
        &mut self,
        caller_key: &CallerKey,
        session_id: &SessionId,
        reason: SessionEndReason,
    ) -> Result<EndSessionResult, IdentityError> {
        let mut candidate = self.snapshot.clone();
        let session = candidate
            .sessions
            .iter_mut()
            .find(|session| &session.session_id == session_id)
            .ok_or(IdentityError::SessionNotFound)?;
        if matches!(session.lifecycle, SessionLifecycle::Ended { .. }) {
            return Ok(EndSessionResult::AlreadyEnded);
        }
        session.lifecycle = SessionLifecycle::Ended {
            at: self.clock.now(),
            reason,
        };
        if session.kind == SessionKind::Guest {
            session.guest_cleanup = GuestCleanupStatus::Pending;
        }
        let user_id = session.user_id;
        let profile_id = session.profile_id.clone();
        let is_guest = session.kind == SessionKind::Guest;
        candidate.revision = self.next_revision()?;
        let lease = self.leases.get(session_id).cloned();
        let mut lease_guard = match lease.as_ref() {
            Some(lease) => match lease.live.try_write() {
                Ok(guard) => Some(guard),
                Err(std::sync::TryLockError::Poisoned(error)) => Some(error.into_inner()),
                Err(std::sync::TryLockError::WouldBlock) => {
                    return Err(IdentityError::SessionLeaseBusy);
                }
            },
            None => None,
        };
        self.save_candidate(candidate)?;

        if let (Some(lease), Some(guard)) = (lease.as_ref(), lease_guard.as_mut()) {
            lease.revoke(guard);
        }
        drop(lease_guard);
        let another_session_is_current = self
            .active_by_caller
            .get(caller_key)
            .is_some_and(|current_session| current_session != session_id);
        if !another_session_is_current {
            self.active_by_caller.remove(caller_key);
            self.last_by_caller
                .insert(caller_key.clone(), session_id.clone());
        }
        self.emit(
            IdentityEventKind::SessionEnded,
            user_id,
            Some(profile_id.clone()),
            Some(session_id.clone()),
            Some(if reason == SessionEndReason::Expired {
                IdentityEventReason::Expired
            } else {
                IdentityEventReason::Ended
            }),
        );

        if is_guest {
            return self.finish_guest_cleanup(session_id, &profile_id);
        }
        Ok(EndSessionResult::Ended)
    }

    fn finish_guest_cleanup(
        &mut self,
        session_id: &SessionId,
        profile_id: &ProfileId,
    ) -> Result<EndSessionResult, IdentityError> {
        if self
            .guest_cleaner
            .cleanup_guest_profile(profile_id)
            .is_err()
        {
            self.emit(
                IdentityEventKind::GuestCleanupFailed,
                None,
                Some(profile_id.clone()),
                Some(session_id.clone()),
                Some(IdentityEventReason::CleanupPending),
            );
            return Ok(EndSessionResult::GuestCleanupPending);
        }
        let mut cleaned = self.snapshot.clone();
        let Some(session) = cleaned
            .sessions
            .iter_mut()
            .find(|session| &session.session_id == session_id)
        else {
            return Err(IdentityError::SessionNotFound);
        };
        if session.kind != SessionKind::Guest
            || !matches!(session.lifecycle, SessionLifecycle::Ended { .. })
            || session.profile_id != *profile_id
        {
            return Err(IdentityError::ProfileOwnershipMismatch);
        }
        session.guest_cleanup = GuestCleanupStatus::Complete;
        cleaned
            .profiles
            .retain(|profile| profile.profile_id != *profile_id);
        cleaned.revision = self.next_revision()?;
        if self.save_candidate(cleaned).is_err() {
            return Ok(EndSessionResult::GuestCleanupPending);
        }
        Ok(EndSessionResult::Ended)
    }

    fn expire_current(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
        session_id: &SessionId,
    ) -> Result<(), ResolutionError> {
        self.authorize(
            caller,
            IdentityAction::EndSession,
            None,
            None,
            Some(session_id.clone()),
        )
        .map_err(map_resolution_identity_error)?;
        let caller_key = CallerKey::from_trusted(caller);
        self.end_record(&caller_key, session_id, SessionEndReason::Expired)
            .map(|_| ())
            .map_err(map_resolution_identity_error)
    }

    fn require_resolved(&self) -> Result<(), ResolutionError> {
        if self.recovered {
            Ok(())
        } else {
            Err(ResolutionError::Unavailable)
        }
    }
}

#[cfg(feature = "system-service-recovery-contract")]
impl<S, I, C, A, G, E> nagi_system_service_recovery_contract::C1RecoveryReceiptClient
    for C1SystemServiceRecoveryReceiptAdapter<'_, '_, S, I, C, A, G, E>
where
    S: IdentitySnapshotStore + RecoveryJournalStore,
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    type Receipt = C1SystemServiceRecoveryTerminalReceipt;

    fn reconcile_terminal<Gate: nagi_system_service_recovery_contract::C3RecoverySlotGate>(
        &mut self,
        scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        gate: &mut Gate,
    ) -> Result<Self::Receipt, nagi_system_service_recovery_contract::RecoveryClientError> {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let scope_snapshot = OwnedRecoveryScopeSnapshot::capture(scope);
        if scope_snapshot.role != "system_service" {
            return Err(RecoveryClientError::Rejected);
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        if self
            .boot_binding
            .is_some_and(|caller| !scope_snapshot.matches_boot_principal(caller))
        {
            return Err(RecoveryClientError::Rejected);
        }
        let caller_principal = self.caller.principal_id().clone();
        let caller_context = self.caller.context_id().clone();
        scope_snapshot
            .validate_caller(&caller_principal, &caller_context)
            .map_err(|_| RecoveryClientError::Rejected)?;
        if scope_snapshot.canonical_binding_digest() != scope_snapshot.binding_digest {
            return Err(RecoveryClientError::Rejected);
        }

        self.identity
            .authorize_recovery_with_captured_caller(caller_principal, caller_context)
            .map_err(recovery_client_error)?;
        let mut gate = CurrentLaunchTerminalGate(gate);
        self.reconcile_terminal_after_scope_preflight(
            scope,
            scope_snapshot.clone(),
            scope,
            scope_snapshot,
            &mut gate,
        )
    }

    fn consume_terminal_receipt(
        &mut self,
        scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        receipt: Self::Receipt,
    ) -> Result<(), nagi_system_service_recovery_contract::RecoveryClientError> {
        let scope_snapshot = OwnedRecoveryScopeSnapshot::capture(scope);
        scope_snapshot
            .validate_operation_identity(receipt.operation_id, &receipt.binding)
            .map_err(|_| nagi_system_service_recovery_contract::RecoveryClientError::Rejected)?;
        self.identity
            .consume_reconciliation_receipt(receipt)
            .map_err(recovery_client_error)
    }
}

#[cfg(feature = "system-service-recovery-contract")]
fn recovery_client_error(
    error: IdentityError,
) -> nagi_system_service_recovery_contract::RecoveryClientError {
    use nagi_system_service_recovery_contract::RecoveryClientError;
    match error {
        IdentityError::AuthorizationDenied => RecoveryClientError::Rejected,
        IdentityError::AuthorizationUnavailable
        | IdentityError::SessionLeaseBusy
        | IdentityError::StoreConflict => RecoveryClientError::Unavailable,
        IdentityError::NotRecovered
        | IdentityError::LeaseEffectReentry
        | IdentityError::StorageOwnerMismatch
        | IdentityError::IdSourceUnavailable
        | IdentityError::IdCollision
        | IdentityError::ProfileHistoryIncomplete
        | IdentityError::InvalidMetadata
        | IdentityError::UserNotFound
        | IdentityError::ProfileNotFound
        | IdentityError::ProfileOwnershipMismatch
        | IdentityError::ProfileAlreadyActive
        | IdentityError::CallerAlreadyHasSession
        | IdentityError::IdempotencyConflict
        | IdentityError::ExpiryNotInFuture
        | IdentityError::SessionNotFound
        | IdentityError::SessionNotCurrent
        | IdentityError::StoreUnavailable
        | IdentityError::CorruptState
        | IdentityError::UnsupportedSchemaVersion
        | IdentityError::RevisionExhausted => RecoveryClientError::Failed,
    }
}

impl<S, I, C, A, G, E> CurrentIdentityResolver for IdentityService<S, I, C, A, G, E>
where
    S: IdentitySnapshotStore,
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    fn resolve_current_identity(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
    ) -> Result<ResolvedIdentity, ResolutionError> {
        self.require_resolved()?;
        let reader = self.store.current_snapshot_reader();
        resolve_current_identity_after_admission(self, caller, reader, true)
    }
}

fn resolve_current_identity_after_admission<S, I, C, A, G, E>(
    service: &mut IdentityService<S, I, C, A, G, E>,
    caller: &impl AuthenticatedInteractiveCaller,
    store_current: Option<CurrentSnapshotReader>,
    expire_current_if_needed: bool,
) -> Result<ResolvedIdentity, ResolutionError>
where
    S: IdentitySnapshotStore,
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    if let Err(error) = service.authorize(
        caller,
        IdentityAction::ResolveCurrentIdentity,
        None,
        None,
        None,
    ) {
        service.emit(
            IdentityEventKind::CurrentIdentityResolutionDenied,
            None,
            None,
            None,
            Some(if error == IdentityError::AuthorizationDenied {
                IdentityEventReason::Denied
            } else {
                IdentityEventReason::Unavailable
            }),
        );
        return Err(map_resolution_identity_error(error));
    }

    let caller_key = CallerKey::from_trusted(caller);
    let session_id = match service.active_by_caller.get(&caller_key).cloned() {
        Some(session_id) => session_id,
        None => {
            let ended = service.last_by_caller.get(&caller_key).and_then(|id| {
                service
                    .snapshot
                    .sessions
                    .iter()
                    .find(|s| &s.session_id == id)
            });
            let (error, user_id, profile_id, session_id, reason) = match ended {
                Some(session) => match session.lifecycle {
                    SessionLifecycle::Ended {
                        reason: SessionEndReason::Expired,
                        ..
                    } => (
                        ResolutionError::Expired,
                        session.user_id,
                        Some(session.profile_id.clone()),
                        Some(session.session_id.clone()),
                        IdentityEventReason::Expired,
                    ),
                    SessionLifecycle::Ended { .. } => (
                        ResolutionError::Ended,
                        session.user_id,
                        Some(session.profile_id.clone()),
                        Some(session.session_id.clone()),
                        IdentityEventReason::Ended,
                    ),
                    SessionLifecycle::Active => (
                        ResolutionError::CorruptState,
                        session.user_id,
                        Some(session.profile_id.clone()),
                        Some(session.session_id.clone()),
                        IdentityEventReason::Corrupt,
                    ),
                },
                None => (
                    ResolutionError::NoSession,
                    None,
                    None,
                    None,
                    IdentityEventReason::NoSession,
                ),
            };
            service.emit(
                IdentityEventKind::CurrentIdentityResolved,
                user_id,
                profile_id,
                session_id,
                Some(reason),
            );
            return Err(error);
        }
    };
    let Some(session) = service
        .snapshot
        .sessions
        .iter()
        .find(|session| session.session_id == session_id)
        .cloned()
    else {
        return Err(ResolutionError::CorruptState);
    };
    match session.lifecycle {
        SessionLifecycle::Ended {
            reason: SessionEndReason::Expired,
            ..
        } => return Err(ResolutionError::Expired),
        SessionLifecycle::Ended { .. } => return Err(ResolutionError::Ended),
        SessionLifecycle::Active => {}
    }
    if session
        .expires_at
        .is_some_and(|expires| service.clock.now() >= expires)
    {
        if expire_current_if_needed {
            service.expire_current(caller, &session_id)?;
        }
        service.emit(
            IdentityEventKind::CurrentIdentityResolved,
            session.user_id,
            Some(session.profile_id),
            Some(session_id),
            Some(IdentityEventReason::Expired),
        );
        return Err(ResolutionError::Expired);
    }

    let live = service
        .leases
        .get(&session_id)
        .cloned()
        .ok_or(ResolutionError::Unavailable)?;
    let identity = ResolvedIdentity {
        session_id: session_id.clone(),
        kind: session.kind,
        user_id: session.user_id,
        profile_id: session.profile_id.clone(),
        caller: caller_key,
        expires_at: session.expires_at,
        clock_now: {
            let clock = service.clock.clone();
            Arc::new(move || clock.now())
        },
        clock_try_now: {
            let clock = service.clock.clone();
            Arc::new(move || clock.try_now())
        },
        live,
        issuer_alive: Arc::clone(&service.issuer.alive),
        issuer_admission: Arc::clone(&service.issuer.admission),
        issued_session: session.clone(),
        store_current,
        #[cfg(all(
            feature = "system-service-recovery-contract",
            target_os = "linux",
            target_arch = "x86_64"
        ))]
        runtime_operation_identity: None,
    };
    drop(identity.checked_live_lease()?);
    service.emit(
        IdentityEventKind::CurrentIdentityResolved,
        session.user_id,
        Some(session.profile_id),
        Some(session_id),
        None,
    );
    Ok(identity)
}

fn map_store_error(error: SnapshotStoreError) -> IdentityError {
    match error {
        SnapshotStoreError::Unavailable => IdentityError::StoreUnavailable,
        SnapshotStoreError::Conflict => IdentityError::StoreConflict,
    }
}

#[cfg(any(test, feature = "system-service-recovery-contract"))]
fn map_recovery_journal_error(error: RecoveryJournalError) -> IdentityError {
    match error {
        RecoveryJournalError::Busy => IdentityError::SessionLeaseBusy,
        RecoveryJournalError::Conflict => IdentityError::StoreConflict,
        RecoveryJournalError::Unavailable
        | RecoveryJournalError::Quarantined
        | RecoveryJournalError::AlreadyClaimed
        | RecoveryJournalError::NotCurrentOperation
        | RecoveryJournalError::OutcomeMismatch => IdentityError::StoreUnavailable,
    }
}

fn map_snapshot_error(error: SnapshotError) -> IdentityError {
    match error {
        SnapshotError::UnsupportedVersion(_) => IdentityError::UnsupportedSchemaVersion,
        _ => IdentityError::CorruptState,
    }
}

fn map_resolution_identity_error(error: IdentityError) -> ResolutionError {
    match error {
        IdentityError::AuthorizationDenied => ResolutionError::Denied,
        IdentityError::CorruptState | IdentityError::UnsupportedSchemaVersion => {
            ResolutionError::CorruptState
        }
        IdentityError::NotRecovered
        | IdentityError::StoreUnavailable
        | IdentityError::StoreConflict
        | IdentityError::StorageOwnerMismatch
        | IdentityError::AuthorizationUnavailable
        | IdentityError::LeaseEffectReentry
        | IdentityError::SessionLeaseBusy
        | IdentityError::IdSourceUnavailable
        | IdentityError::IdCollision
        | IdentityError::InvalidMetadata
        | IdentityError::UserNotFound
        | IdentityError::ProfileNotFound
        | IdentityError::ProfileOwnershipMismatch
        | IdentityError::ProfileAlreadyActive
        | IdentityError::CallerAlreadyHasSession
        | IdentityError::IdempotencyConflict
        | IdentityError::ExpiryNotInFuture
        | IdentityError::SessionNotFound
        | IdentityError::SessionNotCurrent
        | IdentityError::ProfileHistoryIncomplete
        | IdentityError::RevisionExhausted => ResolutionError::Unavailable,
    }
}

#[cfg(all(test, target_os = "linux"))]
mod durable_store_failure_tests {
    use super::*;
    use crate::contract::{
        DurableTestDirectory, TestAuthorizer, TestCaller, TestCleaner, TestClock, TestIds,
    };
    use crate::storage::{
        ProfileRootAuthorizer, ProfileStorageBackend, StorageRootFailure, UserStorageRootAdapter,
    };
    use crate::store::{
        DurableStoreFault, InMemorySnapshotStore, LinuxIdentitySnapshotStore, RecoveryBinding,
        RecoveryEffectClaim, RecoveryJournalError, RecoveryJournalPhase, RecoveryJournalStore,
        RecoveryOperationId, RecoveryOperationPermit, RecoveryReconciliation,
        RecoveryReconciliationPermit, SnapshotStoreState, StoredSnapshot,
    };
    #[cfg(feature = "system-service-recovery-contract")]
    use nagi_system_service_recovery_contract::C1RecoveryReceiptClient as _;
    use std::collections::HashMap;
    use std::process::{Command, ExitStatus};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::{Duration, Instant};

    const TEST_CHANNEL_WATCHDOG: Duration = Duration::from_secs(60);

    #[cfg(all(feature = "system-service-recovery-contract", target_arch = "x86_64"))]
    fn verified_test_principal(c3_launch_generation: u64) -> C1VerifiedBootSystemServicePrincipal {
        C1VerifiedBootSystemServicePrincipal {
            service_id: "service:fixture.identity".to_owned(),
            principal_id: PrincipalId::new("service:fixture.identity").unwrap(),
            caller_context_id: CallerContextId::new("caller_fixture_identity").unwrap(),
            mapping_generation: NonZeroU64::new(3).unwrap(),
            catalog_generation: NonZeroU64::new(9).unwrap(),
            c3_launch_generation: NonZeroU64::new(c3_launch_generation).unwrap(),
        }
    }

    type MemoryTestService = IdentityService<
        InMemorySnapshotStore,
        TestIds,
        TestClock,
        TestAuthorizer,
        TestCleaner,
        NoopIdentityEventSink,
    >;

    #[cfg(feature = "system-service-recovery-contract")]
    type DurableTestService = IdentityService<
        LinuxIdentitySnapshotStore,
        TestIds,
        TestClock,
        TestAuthorizer,
        TestCleaner,
        NoopIdentityEventSink,
    >;

    #[cfg(feature = "system-service-recovery-contract")]
    #[derive(Clone)]
    struct TestRecoveryScope {
        service_id: String,
        principal_id: String,
        caller_context_id: String,
        mapping_generation: std::num::NonZeroU64,
        role: String,
        catalog_generation: std::num::NonZeroU64,
        c3_launch_generation: std::num::NonZeroU64,
        binding_digest: [u8; 32],
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl TestRecoveryScope {
        fn new(c3_launch_generation: u64, caller: &impl TrustedCallerContext) -> Self {
            let mut scope = Self {
                service_id: "service:fixture.identity".to_owned(),
                principal_id: caller.principal_id().as_str().to_owned(),
                caller_context_id: caller.context_id().as_str().to_owned(),
                mapping_generation: std::num::NonZeroU64::new(3).unwrap(),
                role: "system_service".to_owned(),
                catalog_generation: std::num::NonZeroU64::new(9).unwrap(),
                c3_launch_generation: std::num::NonZeroU64::new(c3_launch_generation).unwrap(),
                binding_digest: [0; 32],
            };
            scope.rehash();
            scope
        }

        fn rehash(&mut self) {
            self.binding_digest =
                nagi_system_service_recovery_contract::canonical_recovery_binding_digest_from_scope(
                    self,
                );
        }

        fn change_generation_without_rehash(&mut self, generation: u64) {
            self.c3_launch_generation = std::num::NonZeroU64::new(generation).unwrap();
        }

        fn change_generation_and_rehash(&mut self, generation: u64) {
            self.c3_launch_generation = std::num::NonZeroU64::new(generation).unwrap();
            self.rehash();
        }

        fn change_service_id_and_rehash(&mut self, service_id: &str) {
            self.service_id = service_id.to_owned();
            self.rehash();
        }

        fn change_principal_id_and_rehash(&mut self, principal_id: &str) {
            self.principal_id = principal_id.to_owned();
            self.rehash();
        }

        fn change_caller_context_id_and_rehash(&mut self, caller_context_id: &str) {
            self.caller_context_id = caller_context_id.to_owned();
            self.rehash();
        }

        fn change_mapping_generation_and_rehash(&mut self, generation: u64) {
            self.mapping_generation = std::num::NonZeroU64::new(generation).unwrap();
            self.rehash();
        }

        fn change_role_and_rehash(&mut self, role: &str) {
            self.role = role.to_owned();
            self.rehash();
        }

        fn change_catalog_generation_and_rehash(&mut self, generation: u64) {
            self.catalog_generation = std::num::NonZeroU64::new(generation).unwrap();
            self.rehash();
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope
        for TestRecoveryScope
    {
        fn service_id(&self) -> &str {
            &self.service_id
        }

        fn principal_id(&self) -> &str {
            &self.principal_id
        }

        fn caller_context_id(&self) -> &str {
            &self.caller_context_id
        }

        fn mapping_generation(&self) -> std::num::NonZeroU64 {
            self.mapping_generation
        }

        fn role(&self) -> &str {
            &self.role
        }

        fn catalog_generation(&self) -> std::num::NonZeroU64 {
            self.catalog_generation
        }

        fn transition(&self) -> nagi_system_service_recovery_contract::RecoveryLifecycleTransition {
            nagi_system_service_recovery_contract::RecoveryLifecycleTransition::Initial
        }

        fn package_id(&self) -> &str {
            "org.nagi.fixture"
        }

        fn package_app_id(&self) -> u64 {
            0x1122
        }

        fn canonical_entry(&self) -> &str {
            "fixture.elf"
        }

        fn signed_package_sha256(&self) -> &[u8; 32] {
            &[0x11; 32]
        }

        fn executable_sha256(&self) -> &[u8; 32] {
            &[0x22; 32]
        }

        fn package_generation(&self) -> u64 {
            17
        }

        fn c3_launch_generation(&self) -> std::num::NonZeroU64 {
            self.c3_launch_generation
        }

        fn credentials(&self) -> (u32, u32) {
            (1001, 1002)
        }

        fn binding_digest(&self) -> &[u8; 32] {
            &self.binding_digest
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    struct ImmediateRuntimeRecoveryGate;

    #[cfg(feature = "system-service-recovery-contract")]
    impl C1SystemServiceRecoveryEffectGate for ImmediateRuntimeRecoveryGate {
        fn apply_recovery_effect<R>(
            &mut self,
            scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            operation: crate::store::C1RecoveryOperationIdentity,
            effect: impl FnOnce() -> Result<R, IdentityError>,
        ) -> Result<Result<R, IdentityError>, RecoveryEffectGateError> {
            if operation.binding_digest() != scope.binding_digest() {
                return Err(RecoveryEffectGateError::Unavailable);
            }
            Ok(effect())
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    struct StatefulContextRecoveryScope {
        base: TestRecoveryScope,
        first_context: String,
        context_calls: std::cell::Cell<usize>,
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl StatefulContextRecoveryScope {
        fn mismatching_second_context(generation: u64, caller: &impl TrustedCallerContext) -> Self {
            let mut base = TestRecoveryScope::new(generation, caller);
            let first_context = base.caller_context_id.clone();
            base.caller_context_id = format!("{first_context}-changed-after-read");
            base.binding_digest =
                nagi_system_service_recovery_contract::canonical_recovery_binding_digest_from_scope(
                    &base,
                );
            Self {
                base,
                first_context,
                context_calls: std::cell::Cell::new(0),
            }
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope
        for StatefulContextRecoveryScope
    {
        fn service_id(&self) -> &str {
            self.base.service_id()
        }

        fn principal_id(&self) -> &str {
            self.base.principal_id()
        }

        fn caller_context_id(&self) -> &str {
            if self.context_calls.replace(self.context_calls.get() + 1) == 0 {
                &self.first_context
            } else {
                &self.base.caller_context_id
            }
        }

        fn mapping_generation(&self) -> std::num::NonZeroU64 {
            self.base.mapping_generation()
        }

        fn role(&self) -> &str {
            self.base.role()
        }

        fn catalog_generation(&self) -> std::num::NonZeroU64 {
            self.base.catalog_generation()
        }

        fn transition(&self) -> nagi_system_service_recovery_contract::RecoveryLifecycleTransition {
            self.base.transition()
        }

        fn package_id(&self) -> &str {
            self.base.package_id()
        }

        fn package_app_id(&self) -> u64 {
            self.base.package_app_id()
        }

        fn canonical_entry(&self) -> &str {
            self.base.canonical_entry()
        }

        fn signed_package_sha256(&self) -> &[u8; 32] {
            self.base.signed_package_sha256()
        }

        fn executable_sha256(&self) -> &[u8; 32] {
            self.base.executable_sha256()
        }

        fn package_generation(&self) -> u64 {
            self.base.package_generation()
        }

        fn c3_launch_generation(&self) -> std::num::NonZeroU64 {
            self.base.c3_launch_generation()
        }

        fn credentials(&self) -> (u32, u32) {
            self.base.credentials()
        }

        fn binding_digest(&self) -> &[u8; 32] {
            self.base.binding_digest()
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    struct CanonicalSecondViewRecoveryScope {
        first: TestRecoveryScope,
        second_generation: std::num::NonZeroU64,
        second_binding_digest: [u8; 32],
        second_view: std::cell::Cell<bool>,
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl CanonicalSecondViewRecoveryScope {
        fn new(first_generation: u64, caller: &impl TrustedCallerContext) -> Self {
            let first = TestRecoveryScope::new(first_generation, caller);
            let second = TestRecoveryScope::new(first_generation + 1, caller);
            Self {
                first,
                second_generation: second.c3_launch_generation,
                second_binding_digest: second.binding_digest,
                second_view: std::cell::Cell::new(false),
            }
        }

        fn select_second_view(&self) {
            self.second_view.set(true);
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope
        for CanonicalSecondViewRecoveryScope
    {
        fn service_id(&self) -> &str {
            self.first.service_id()
        }

        fn principal_id(&self) -> &str {
            self.first.principal_id()
        }

        fn caller_context_id(&self) -> &str {
            self.first.caller_context_id()
        }

        fn mapping_generation(&self) -> std::num::NonZeroU64 {
            self.first.mapping_generation()
        }

        fn role(&self) -> &str {
            self.first.role()
        }

        fn catalog_generation(&self) -> std::num::NonZeroU64 {
            self.first.catalog_generation()
        }

        fn transition(&self) -> nagi_system_service_recovery_contract::RecoveryLifecycleTransition {
            self.first.transition()
        }

        fn package_id(&self) -> &str {
            self.first.package_id()
        }

        fn package_app_id(&self) -> u64 {
            self.first.package_app_id()
        }

        fn canonical_entry(&self) -> &str {
            self.first.canonical_entry()
        }

        fn signed_package_sha256(&self) -> &[u8; 32] {
            self.first.signed_package_sha256()
        }

        fn executable_sha256(&self) -> &[u8; 32] {
            self.first.executable_sha256()
        }

        fn package_generation(&self) -> u64 {
            self.first.package_generation()
        }

        fn c3_launch_generation(&self) -> std::num::NonZeroU64 {
            if self.second_view.get() {
                self.second_generation
            } else {
                self.first.c3_launch_generation
            }
        }

        fn credentials(&self) -> (u32, u32) {
            self.first.credentials()
        }

        fn binding_digest(&self) -> &[u8; 32] {
            if self.second_view.get() {
                &self.second_binding_digest
            } else {
                &self.first.binding_digest
            }
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    struct CanonicalSecondViewC3Gate<'scope> {
        expected_scope:
            &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        owner_scope: &'scope CanonicalSecondViewRecoveryScope,
        invocations: usize,
        callbacks: usize,
        observed_binding_digest: Option<[u8; 32]>,
        observed_view_is_canonical: bool,
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl<'scope> CanonicalSecondViewC3Gate<'scope> {
        fn new(owner_scope: &'scope CanonicalSecondViewRecoveryScope) -> Self {
            Self {
                expected_scope: owner_scope,
                owner_scope,
                invocations: 0,
                callbacks: 0,
                observed_binding_digest: None,
                observed_view_is_canonical: false,
            }
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl nagi_system_service_recovery_contract::C3RecoverySlotGate for CanonicalSecondViewC3Gate<'_> {
        fn with_reconciliation_gate<R>(
            &mut self,
            scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            _claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
            reconcile: impl FnOnce() -> R,
        ) -> Result<R, nagi_system_service_recovery_contract::RecoveryClientError> {
            use nagi_system_service_recovery_contract::RecoveryClientError;
            if !std::ptr::eq(scope, self.expected_scope) {
                return Err(RecoveryClientError::Rejected);
            }
            self.invocations += 1;
            self.owner_scope.select_second_view();
            let digest =
                nagi_system_service_recovery_contract::canonical_recovery_binding_digest_from_scope(
                    scope,
                );
            self.observed_view_is_canonical = digest == *scope.binding_digest();
            self.observed_binding_digest = Some(digest);
            let result = reconcile();
            self.callbacks += 1;
            Ok(result)
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    struct PostTerminalScopeChangeC3Gate<'scope> {
        expected_scope:
            &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        owner_scope: &'scope CanonicalSecondViewRecoveryScope,
        invocations: usize,
        callbacks: usize,
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl<'scope> PostTerminalScopeChangeC3Gate<'scope> {
        fn new(owner_scope: &'scope CanonicalSecondViewRecoveryScope) -> Self {
            Self {
                expected_scope: owner_scope,
                owner_scope,
                invocations: 0,
                callbacks: 0,
            }
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl nagi_system_service_recovery_contract::C3RecoverySlotGate
        for PostTerminalScopeChangeC3Gate<'_>
    {
        fn with_reconciliation_gate<R>(
            &mut self,
            scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            _claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
            reconcile: impl FnOnce() -> R,
        ) -> Result<R, nagi_system_service_recovery_contract::RecoveryClientError> {
            use nagi_system_service_recovery_contract::RecoveryClientError;
            if !std::ptr::eq(scope, self.expected_scope) {
                return Err(RecoveryClientError::Rejected);
            }
            self.invocations += 1;
            let result = reconcile();
            self.callbacks += 1;
            // Model a persistent change after C1 commits but before the owner
            // gate returns. C1 must withhold its receipt and retain its fence.
            self.owner_scope.select_second_view();
            Ok(result)
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    struct UnknownAfterScopeChangeC3Gate<'scope> {
        expected_scope:
            &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        owner_scope: &'scope CanonicalSecondViewRecoveryScope,
        invocations: usize,
        callbacks: usize,
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl<'scope> UnknownAfterScopeChangeC3Gate<'scope> {
        fn new(owner_scope: &'scope CanonicalSecondViewRecoveryScope) -> Self {
            Self {
                expected_scope: owner_scope,
                owner_scope,
                invocations: 0,
                callbacks: 0,
            }
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl nagi_system_service_recovery_contract::C3RecoverySlotGate
        for UnknownAfterScopeChangeC3Gate<'_>
    {
        fn with_reconciliation_gate<R>(
            &mut self,
            scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            _claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
            _reconcile: impl FnOnce() -> R,
        ) -> Result<R, nagi_system_service_recovery_contract::RecoveryClientError> {
            use nagi_system_service_recovery_contract::RecoveryClientError;
            if !std::ptr::eq(scope, self.expected_scope) {
                return Err(RecoveryClientError::Rejected);
            }
            self.invocations += 1;
            self.owner_scope.select_second_view();
            Err(RecoveryClientError::OutcomeUnknown)
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    struct CountingC3ReconciliationGate<'scope> {
        expected_scope:
            &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        invocations: usize,
        callbacks: usize,
        observed_claim:
            Option<nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation>,
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl<'scope> CountingC3ReconciliationGate<'scope> {
        fn new(
            scope: &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        ) -> Self {
            Self {
                expected_scope: scope,
                invocations: 0,
                callbacks: 0,
                observed_claim: None,
            }
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    struct ConfirmedPriorLaunchC3GateFixture<'scope> {
        current_scope:
            &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        previous_scope:
            &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        invocations: usize,
        callbacks: usize,
        observed_claim:
            Option<nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation>,
        boot_ids: ([u8; 16], [u8; 16]),
    }

    #[cfg(feature = "system-service-recovery-contract")]
    struct SyntheticC3PriorLaunchTerminalReceipt<'scope> {
        previous_scope:
            &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        c1_operation_id: Option<NonZeroU64>,
        service_id: String,
        catalog_generation: u64,
        c3_launch_generation: u64,
        scope_binding_digest: [u8; 32],
        state: nagi_system_service_recovery_contract::C3RecoverySlotState,
        boot_ids: ([u8; 16], [u8; 16]),
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl nagi_system_service_recovery_contract::C3DurableRecoverySlotReceipt
        for SyntheticC3PriorLaunchTerminalReceipt<'_>
    {
        fn c1_operation_id(&self) -> Option<NonZeroU64> {
            self.c1_operation_id
        }

        fn service_id(&self) -> &str {
            &self.service_id
        }

        fn catalog_generation(&self) -> u64 {
            self.catalog_generation
        }

        fn c3_launch_generation(&self) -> u64 {
            self.c3_launch_generation
        }

        fn scope_binding_digest(&self) -> &[u8; 32] {
            &self.scope_binding_digest
        }

        fn state(&self) -> nagi_system_service_recovery_contract::C3RecoverySlotState {
            self.state
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl nagi_system_service_recovery_contract::C3DurablePriorBootClearedReceipt
        for SyntheticC3PriorLaunchTerminalReceipt<'_>
    {
        fn previous_scope(
            &self,
        ) -> &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope
        {
            self.previous_scope
        }
        fn previous_boot_id(&self) -> &[u8; 16] {
            &self.boot_ids.0
        }
        fn current_boot_id(&self) -> &[u8; 16] {
            &self.boot_ids.1
        }
        fn previous_c3_launch_instance_id(&self) -> NonZeroU64 {
            NonZeroU64::MIN
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl<'scope> ConfirmedPriorLaunchC3GateFixture<'scope> {
        fn new(
            current_scope: &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            previous_scope: &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        ) -> Self {
            Self {
                current_scope,
                previous_scope,
                invocations: 0,
                callbacks: 0,
                observed_claim: None,
                boot_ids: ([0; 16], [0; 16]),
            }
        }

        fn prior_boot_clear(
            current_scope: &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            previous_scope: &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            previous_boot_id: [u8; 16],
            current_boot_id: [u8; 16],
        ) -> Self {
            let mut gate = Self::new(current_scope, previous_scope);
            gate.boot_ids = (previous_boot_id, current_boot_id);
            gate
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl<'scope> C1PreviousLaunchTerminalGate for ConfirmedPriorLaunchC3GateFixture<'scope> {
        type TerminalReceipt = SyntheticC3PriorLaunchTerminalReceipt<'scope>;

        fn with_previous_launch_reconciliation_gate<R>(
            &mut self,
            current_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
            validate_previous_scope: impl FnOnce(
                &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            ) -> Result<
                R,
                nagi_system_service_recovery_contract::RecoveryClientError,
            >,
        ) -> Result<
            (R, Self::TerminalReceipt),
            nagi_system_service_recovery_contract::RecoveryClientError,
        > {
            use nagi_system_service_recovery_contract::RecoveryClientError;
            if !std::ptr::eq(current_scope, self.current_scope) {
                return Err(RecoveryClientError::Rejected);
            }
            let current = OwnedRecoveryScopeSnapshot::capture(current_scope);
            let previous = OwnedRecoveryScopeSnapshot::capture(self.previous_scope);
            if current.c3_launch_generation <= previous.c3_launch_generation {
                return Err(RecoveryClientError::Rejected);
            }

            // This is an explicitly synthetic owner proof. Production C3 must
            // verify and read back its durable terminal evidence before
            // invoking the callback and returning the typed slot receipt.
            self.invocations += 1;
            self.observed_claim = Some(claim);
            let validated_scope = validate_previous_scope(self.previous_scope)?;
            self.callbacks += 1;
            let receipt = SyntheticC3PriorLaunchTerminalReceipt {
                c1_operation_id: Some(claim.operation_id()),
                service_id: self.previous_scope.service_id().to_owned(),
                catalog_generation: self.previous_scope.catalog_generation().get(),
                c3_launch_generation: self.previous_scope.c3_launch_generation().get(),
                scope_binding_digest: *self.previous_scope.binding_digest(),
                state: nagi_system_service_recovery_contract::C3RecoverySlotState::ClearedAndAbsent,
                previous_scope: self.previous_scope,
                boot_ids: self.boot_ids,
            };
            Ok((validated_scope, receipt))
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl<'scope> C1PreviousBootTerminalGate for ConfirmedPriorLaunchC3GateFixture<'scope> {
        type TerminalReceipt = SyntheticC3PriorLaunchTerminalReceipt<'scope>;

        fn with_previous_boot_reconciliation_gate<R>(
            &mut self,
            current_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
            validate_previous_scope: impl FnOnce(
                &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            ) -> Result<
                R,
                nagi_system_service_recovery_contract::RecoveryClientError,
            >,
        ) -> Result<
            (R, Self::TerminalReceipt),
            nagi_system_service_recovery_contract::RecoveryClientError,
        > {
            let (validated, mut receipt) = self.with_previous_launch_reconciliation_gate(
                current_scope,
                claim,
                validate_previous_scope,
            )?;
            receipt.state =
                nagi_system_service_recovery_contract::C3RecoverySlotState::PriorBootCleared;
            Ok((validated, receipt))
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    struct UnknownPriorLaunchC3GateFixture<'scope> {
        current_scope:
            &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        previous_scope:
            &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        invocations: usize,
        callbacks: usize,
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl<'scope> UnknownPriorLaunchC3GateFixture<'scope> {
        fn new(
            current_scope: &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            previous_scope: &'scope dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
        ) -> Self {
            Self {
                current_scope,
                previous_scope,
                invocations: 0,
                callbacks: 0,
            }
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl<'scope> C1PreviousBootTerminalGate for UnknownPriorLaunchC3GateFixture<'scope> {
        type TerminalReceipt = SyntheticC3PriorLaunchTerminalReceipt<'scope>;

        fn with_previous_boot_reconciliation_gate<R>(
            &mut self,
            current_scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            _claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
            validate_previous_scope: impl FnOnce(
                &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            ) -> Result<
                R,
                nagi_system_service_recovery_contract::RecoveryClientError,
            >,
        ) -> Result<
            (R, Self::TerminalReceipt),
            nagi_system_service_recovery_contract::RecoveryClientError,
        > {
            use nagi_system_service_recovery_contract::RecoveryClientError;
            if !std::ptr::eq(current_scope, self.current_scope) {
                return Err(RecoveryClientError::Rejected);
            }
            self.invocations += 1;
            // Exercise the post-callback uncertain C3 receipt/readback order.
            // This callback may only validate scopes; C1 storage publication
            // happens after a typed confirmation is returned.
            let _validated_scope = validate_previous_scope(self.previous_scope)?;
            self.callbacks += 1;
            Err(RecoveryClientError::OutcomeUnknown)
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    impl nagi_system_service_recovery_contract::C3RecoverySlotGate
        for CountingC3ReconciliationGate<'_>
    {
        fn with_reconciliation_gate<R>(
            &mut self,
            scope: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope,
            claim: nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation,
            reconcile: impl FnOnce() -> R,
        ) -> Result<R, nagi_system_service_recovery_contract::RecoveryClientError> {
            use nagi_system_service_recovery_contract::RecoveryClientError;
            if !std::ptr::eq(scope, self.expected_scope) {
                return Err(RecoveryClientError::Rejected);
            }
            self.invocations += 1;
            self.observed_claim = Some(claim);
            let result = reconcile();
            self.callbacks += 1;
            Ok(result)
        }
    }

    struct PanicPayloadThatPanicsWhenDropped;

    impl Drop for PanicPayloadThatPanicsWhenDropped {
        fn drop(&mut self) {
            panic!("panic payload destructor panic");
        }
    }

    struct DropTrackedStore {
        inner: InMemorySnapshotStore,
        drop_count: Arc<AtomicU64>,
        panic_on_drop: bool,
        panic_payload_drop: bool,
    }

    impl Drop for DropTrackedStore {
        fn drop(&mut self) {
            self.drop_count.fetch_add(1, Ordering::SeqCst);
            if self.panic_on_drop {
                if self.panic_payload_drop {
                    std::panic::panic_any(PanicPayloadThatPanicsWhenDropped);
                }
                panic!("test snapshot store destructor panic");
            }
        }
    }

    impl IdentitySnapshotStore for DropTrackedStore {
        fn current_snapshot_reader(&self) -> Option<CurrentSnapshotReader> {
            self.inner.current_snapshot_reader()
        }

        fn load_current_state(&self) -> Result<SnapshotStoreState, SnapshotStoreError> {
            self.inner.load_current_state()
        }

        fn recovery_quarantined(&self) -> Result<bool, SnapshotStoreError> {
            self.inner.recovery_quarantined()
        }

        fn recovery_operation_active(&self) -> Result<bool, SnapshotStoreError> {
            self.inner.recovery_operation_active()
        }

        fn load_current(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
            self.inner.load_current()
        }

        fn load_last_known_good(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
            self.inner.load_last_known_good()
        }

        fn commit_atomic(
            &mut self,
            expected_version: Option<SnapshotVersion>,
            complete_snapshot: &[u8],
        ) -> Result<SnapshotVersion, SnapshotStoreError> {
            self.inner
                .commit_atomic(expected_version, complete_snapshot)
        }
    }

    type DropTrackedTestService = IdentityService<
        DropTrackedStore,
        TestIds,
        TestClock,
        TestAuthorizer,
        TestCleaner,
        NoopIdentityEventSink,
    >;

    struct NonStorePanicComponent {
        drops: Arc<AtomicU64>,
        panic_on_drop: bool,
    }

    impl Clone for NonStorePanicComponent {
        fn clone(&self) -> Self {
            Self {
                drops: Arc::clone(&self.drops),
                panic_on_drop: false,
            }
        }
    }

    impl Drop for NonStorePanicComponent {
        fn drop(&mut self) {
            if self.panic_on_drop {
                self.drops.fetch_add(1, Ordering::SeqCst);
                panic!("injected non-store identity component destructor panic");
            }
        }
    }

    impl Clock for NonStorePanicComponent {
        fn now(&self) -> UnixMillis {
            UnixMillis::new(100)
        }
    }

    impl IdentityAuthorizer for NonStorePanicComponent {
        fn authorize(
            &mut self,
            _request: &AuthorizationRequest,
        ) -> Result<(), AuthorizationFailure> {
            Ok(())
        }
    }

    impl GuestProfileCleaner for NonStorePanicComponent {
        fn cleanup_guest_profile(
            &mut self,
            _profile_id: &ProfileId,
        ) -> Result<(), GuestCleanupFailure> {
            Ok(())
        }
    }

    impl IdentityEventSink for NonStorePanicComponent {
        fn emit(&mut self, _event: &IdentityEvent) -> bool {
            true
        }
    }

    type NonStorePanicTestService = IdentityService<
        DropTrackedStore,
        TestIds,
        NonStorePanicComponent,
        NonStorePanicComponent,
        NonStorePanicComponent,
        NonStorePanicComponent,
    >;

    struct DropIssuerOnUnwind(Arc<Mutex<Option<NonStorePanicTestService>>>);

    impl Drop for DropIssuerOnUnwind {
        fn drop(&mut self) {
            let issuer = self.0.lock().unwrap().take();
            drop(issuer);
        }
    }

    fn drop_tracked_session(
        context: &str,
        drop_count: Arc<AtomicU64>,
        panic_on_drop: bool,
    ) -> (
        DropTrackedTestService,
        TestCaller,
        SessionId,
        Arc<SessionLeaseRevalidationHandle>,
    ) {
        let caller = TestCaller::new(context);
        let store = DropTrackedStore {
            inner: InMemorySnapshotStore::default(),
            drop_count,
            panic_on_drop,
            panic_payload_drop: false,
        };
        let mut issuer = IdentityService::new(
            store,
            TestIds::default(),
            TestClock::at(100),
            TestAuthorizer::default(),
            TestCleaner::default(),
            NoopIdentityEventSink,
        );
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Gate probe").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        let session_id = issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                Some(UnixMillis::new(1_000)),
                IdempotencyKey::new(format!("drop-probe-{context}")).unwrap(),
            )
            .unwrap();
        let context = issuer.interactive_session(&caller).unwrap();
        let handle = Arc::new(context.revalidation_handle().unwrap());
        drop(context);
        (issuer, caller, session_id, handle)
    }

    fn drop_tracked_session_with_panicking_panic_payload(
        context: &str,
        drop_count: Arc<AtomicU64>,
    ) -> (
        DropTrackedTestService,
        TestCaller,
        SessionId,
        Arc<SessionLeaseRevalidationHandle>,
    ) {
        let (mut issuer, caller, session_id, handle) =
            drop_tracked_session(context, drop_count, true);
        issuer
            .store
            .store
            .as_mut()
            .expect("the tracked store exists before its issuer closes")
            .panic_payload_drop = true;
        (issuer, caller, session_id, handle)
    }

    struct CountingRecoveryGate {
        binding: RecoveryBinding,
        block_session_writer: Option<Arc<SessionLeaseState>>,
        operation_identities: Vec<C1RecoveryOperationIdentity>,
        invocations: usize,
        effects: usize,
        successful_callbacks: usize,
        fail_before_effect: bool,
        panic_before_effect: bool,
        panic_payload_before_effect: bool,
        panic_after_effect: bool,
    }

    impl CountingRecoveryGate {
        fn for_binding(binding: RecoveryBinding) -> Self {
            Self {
                binding,
                block_session_writer: None,
                operation_identities: Vec::new(),
                invocations: 0,
                effects: 0,
                successful_callbacks: 0,
                fail_before_effect: false,
                panic_before_effect: false,
                panic_payload_before_effect: false,
                panic_after_effect: false,
            }
        }
    }

    impl SystemServiceRecoveryEffectGate for CountingRecoveryGate {
        fn recovery_binding(&self) -> &RecoveryBinding {
            &self.binding
        }

        fn apply_recovery_effect<R>(
            &mut self,
            operation: C1RecoveryOperationIdentity,
            effect: impl FnOnce() -> Result<R, IdentityError>,
        ) -> Result<Result<R, IdentityError>, RecoveryEffectGateError> {
            self.operation_identities.push(operation);
            self.invocations += 1;
            if self.fail_before_effect {
                return Err(RecoveryEffectGateError::Unavailable);
            }
            if self.panic_before_effect {
                panic!("injected owner-gate panic before recovery callback");
            }
            if self.panic_payload_before_effect {
                std::panic::panic_any(PanicPayloadThatPanicsWhenDropped);
            }
            self.effects += 1;
            let blocked_session = self.block_session_writer.clone();
            let _blocked_session_writer = blocked_session
                .as_ref()
                .map(|session| session.live.write().unwrap());
            let result = effect();
            if result.is_ok() {
                self.successful_callbacks += 1;
            }
            if self.panic_after_effect {
                panic!("injected owner-gate panic after recovery callback");
            }
            Ok(result)
        }
    }

    /// Owns the raw-reader release channel and worker so assertion unwind
    /// cannot strand a paused thread. It disconnects the entry receiver before
    /// waking/joining the worker, so an entry rendezvous cannot wait on a
    /// receiver whose owner is already joining it.
    struct PausedRawSnapshotReader {
        entered_rx: Option<mpsc::Receiver<()>>,
        start_tx: Option<mpsc::Sender<()>>,
        release_tx: Option<mpsc::Sender<()>>,
        result_rx: mpsc::Receiver<Result<Option<StoredSnapshot>, SnapshotStoreError>>,
        worker: Option<thread::JoinHandle<()>>,
    }

    impl PausedRawSnapshotReader {
        fn wait_until_entered(&self, timeout: Duration) -> Result<(), mpsc::RecvTimeoutError> {
            self.entered_rx
                .as_ref()
                .expect("entry receiver is owned until the reader worker joins")
                .recv_timeout(timeout)
        }

        fn release_and_join(&mut self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
            if let Some(start_tx) = self.start_tx.take() {
                start_tx
                    .send(())
                    .expect("raw reader start gate remains connected");
            }
            self.release_tx
                .take()
                .expect("raw reader release channel is used once")
                .send(())
                .expect("raw reader release channel remains connected");
            let result = self
                .result_rx
                .recv_timeout(TEST_CHANNEL_WATCHDOG)
                .expect("raw reader result must arrive before the test watchdog");
            self.worker
                .take()
                .expect("raw reader worker is joined only once")
                .join()
                .expect("raw reader worker must not panic");
            self.entered_rx.take();
            result
        }
    }

    impl Drop for PausedRawSnapshotReader {
        fn drop(&mut self) {
            // Disconnect first. If the worker is blocked in a zero-capacity
            // entry send, it wakes with SendError before this thread joins it.
            self.entered_rx.take();
            if let Some(start_tx) = self.start_tx.take() {
                let _ = start_tx.send(());
            }
            if let Some(release_tx) = self.release_tx.take() {
                let _ = release_tx.send(());
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    struct DenyRecoveryAuthorizer;

    impl IdentityAuthorizer for DenyRecoveryAuthorizer {
        fn authorize(
            &mut self,
            request: &AuthorizationRequest,
        ) -> Result<(), AuthorizationFailure> {
            if request.action == IdentityAction::Recover {
                Err(AuthorizationFailure::Denied)
            } else {
                Ok(())
            }
        }
    }

    struct NoopProfileRootAuthorizer;

    impl ProfileRootAuthorizer for NoopProfileRootAuthorizer {
        fn authorize_profile_root(
            &mut self,
            _caller: &dyn crate::TrustedCallerContext,
            _identity: &ResolvedIdentity,
            _requested_profile: &ProfileId,
        ) -> Result<(), StorageRootFailure> {
            Ok(())
        }
    }

    struct CallbackProfileRootAuthorizer<F>(F);

    impl<F> ProfileRootAuthorizer for CallbackProfileRootAuthorizer<F>
    where
        F: FnMut() -> Result<(), StorageRootFailure>,
    {
        fn authorize_profile_root(
            &mut self,
            _caller: &dyn crate::TrustedCallerContext,
            _identity: &ResolvedIdentity,
            _requested_profile: &ProfileId,
        ) -> Result<(), StorageRootFailure> {
            (self.0)()
        }
    }

    struct NoopProfileStorageBackend;

    impl ProfileStorageBackend for NoopProfileStorageBackend {
        type Root = ();

        fn open_profile_root(
            &mut self,
            _user_id: Option<UserId>,
            _profile_id: &ProfileId,
        ) -> Result<Self::Root, StorageRootFailure> {
            Ok(())
        }

        fn read_object(
            &mut self,
            _root: &Self::Root,
            _object_id: nagi_model::ObjectId,
        ) -> Result<Vec<u8>, StorageRootFailure> {
            Ok(Vec::new())
        }

        fn write_object(
            &mut self,
            _root: &Self::Root,
            _object_id: nagi_model::ObjectId,
            _data: &[u8],
        ) -> Result<(), StorageRootFailure> {
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct ProfileContextProbeBackend {
        calls: Arc<Mutex<Vec<&'static str>>>,
    }

    impl ProfileStorageBackend for ProfileContextProbeBackend {
        type Root = ();

        fn open_profile_root(
            &mut self,
            _user_id: Option<UserId>,
            _profile_id: &ProfileId,
        ) -> Result<Self::Root, StorageRootFailure> {
            self.calls.lock().unwrap().push("open");
            Ok(())
        }

        fn read_object(
            &mut self,
            _root: &Self::Root,
            _object_id: nagi_model::ObjectId,
        ) -> Result<Vec<u8>, StorageRootFailure> {
            self.calls.lock().unwrap().push("read");
            Ok(Vec::new())
        }

        fn write_object(
            &mut self,
            _root: &Self::Root,
            _object_id: nagi_model::ObjectId,
            _data: &[u8],
        ) -> Result<(), StorageRootFailure> {
            self.calls.lock().unwrap().push("write");
            Ok(())
        }
    }

    #[derive(Clone, Default)]
    struct DropProbeBackend {
        opened: Arc<AtomicU64>,
        dropped: Arc<AtomicU64>,
    }

    struct DropProbeRoot(Arc<AtomicU64>);

    impl Drop for DropProbeRoot {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl ProfileStorageBackend for DropProbeBackend {
        type Root = DropProbeRoot;

        fn open_profile_root(
            &mut self,
            _user_id: Option<UserId>,
            _profile_id: &ProfileId,
        ) -> Result<Self::Root, StorageRootFailure> {
            self.opened.fetch_add(1, Ordering::SeqCst);
            Ok(DropProbeRoot(Arc::clone(&self.dropped)))
        }

        fn read_object(
            &mut self,
            _root: &Self::Root,
            _object_id: nagi_model::ObjectId,
        ) -> Result<Vec<u8>, StorageRootFailure> {
            Ok(Vec::new())
        }

        fn write_object(
            &mut self,
            _root: &Self::Root,
            _object_id: nagi_model::ObjectId,
            _data: &[u8],
        ) -> Result<(), StorageRootFailure> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct RecoveryFenceDropProbeBackend {
        admission: Arc<IssuerAdmission>,
        issuer_alive: Arc<AtomicBool>,
        opened: Arc<AtomicU64>,
        dropped: Arc<AtomicU64>,
        closed_at_drop: Arc<AtomicBool>,
    }

    struct RecoveryFenceDropProbeRoot {
        admission: Arc<IssuerAdmission>,
        issuer_alive: Arc<AtomicBool>,
        dropped: Arc<AtomicU64>,
        closed_at_drop: Arc<AtomicBool>,
    }

    impl Drop for RecoveryFenceDropProbeRoot {
        fn drop(&mut self) {
            let state = self
                .admission
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.closed_at_drop.store(
                state.closed && !self.issuer_alive.load(Ordering::Acquire),
                Ordering::SeqCst,
            );
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl ProfileStorageBackend for RecoveryFenceDropProbeBackend {
        type Root = RecoveryFenceDropProbeRoot;

        fn open_profile_root(
            &mut self,
            _user_id: Option<UserId>,
            _profile_id: &ProfileId,
        ) -> Result<Self::Root, StorageRootFailure> {
            self.opened.fetch_add(1, Ordering::SeqCst);
            Ok(RecoveryFenceDropProbeRoot {
                admission: Arc::clone(&self.admission),
                issuer_alive: Arc::clone(&self.issuer_alive),
                dropped: Arc::clone(&self.dropped),
                closed_at_drop: Arc::clone(&self.closed_at_drop),
            })
        }

        fn read_object(
            &mut self,
            _root: &Self::Root,
            _object_id: nagi_model::ObjectId,
        ) -> Result<Vec<u8>, StorageRootFailure> {
            Ok(Vec::new())
        }

        fn write_object(
            &mut self,
            _root: &Self::Root,
            _object_id: nagi_model::ObjectId,
            _data: &[u8],
        ) -> Result<(), StorageRootFailure> {
            Ok(())
        }
    }

    #[derive(Clone)]
    struct CleanupTestClock(Arc<AtomicU64>);

    impl CleanupTestClock {
        fn at(now: u64) -> Self {
            Self(Arc::new(AtomicU64::new(now)))
        }

        fn set(&self, now: u64) {
            self.0.store(now, Ordering::SeqCst);
        }
    }

    impl Clock for CleanupTestClock {
        fn now(&self) -> UnixMillis {
            UnixMillis::new(self.0.load(Ordering::SeqCst))
        }

        fn try_now(&self) -> Result<UnixMillis, TryClockError> {
            Ok(self.now())
        }
    }

    #[derive(Debug)]
    enum ProbeOutcome {
        BeforeReadyExit,
        AfterReadyExit(ExitStatus),
        BeforeReadyTimeout,
        AfterReadyTimeout,
    }

    static NEXT_PROBE_ID: AtomicU64 = AtomicU64::new(1);
    const PROBE_READY_ENV: &str = "NAGI_C1_GATE_PROBE_READY_FILE";

    fn mark_probe_ready() {
        if let Some(path) = std::env::var_os(PROBE_READY_ENV) {
            std::fs::write(path, b"ready").unwrap();
        }
    }

    /// Run a deliberately ignored probe in a separate process. A stuck test
    /// child is killed and reaped so a regression cannot wedge the test runner.
    fn run_ignored_probe(name: &str, timeout: Duration) -> ProbeOutcome {
        let probe_id = NEXT_PROBE_ID.fetch_add(1, Ordering::Relaxed);
        let ready_file = std::env::temp_dir().join(format!(
            "nagi-c1-gate-probe-{}-{probe_id}",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&ready_file);
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--nocapture", "--test-threads=1", name])
            .env(PROBE_READY_ENV, &ready_file)
            .spawn()
            .unwrap();

        let mut ready = false;
        let mut deadline = Instant::now() + timeout;
        loop {
            if !ready && ready_file.exists() {
                ready = true;
                deadline = Instant::now() + timeout;
            }
            if let Some(status) = child.try_wait().unwrap() {
                let _ = std::fs::remove_file(&ready_file);
                return if ready {
                    ProbeOutcome::AfterReadyExit(status)
                } else {
                    ProbeOutcome::BeforeReadyExit
                };
            }
            if Instant::now() >= deadline {
                let outcome = if ready {
                    ProbeOutcome::AfterReadyTimeout
                } else {
                    ProbeOutcome::BeforeReadyTimeout
                };
                let killed = child.kill().is_ok();
                let status = child.wait().unwrap();
                let _ = std::fs::remove_file(&ready_file);
                if killed {
                    return outcome;
                }
                return ProbeOutcome::AfterReadyExit(status);
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn memory_service(clock: TestClock) -> MemoryTestService {
        IdentityService::new(
            InMemorySnapshotStore::default(),
            TestIds::default(),
            clock,
            TestAuthorizer::default(),
            TestCleaner::default(),
            NoopIdentityEventSink,
        )
    }

    fn memory_session(
        context: &str,
    ) -> (
        MemoryTestService,
        TestCaller,
        SessionId,
        Arc<SessionLeaseRevalidationHandle>,
    ) {
        let caller = TestCaller::new(context);
        let mut issuer = memory_service(TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Gate probe").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        let session_id = issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                Some(UnixMillis::new(1_000)),
                IdempotencyKey::new(format!("gate-probe-{context}")).unwrap(),
            )
            .unwrap();
        let context = issuer.interactive_session(&caller).unwrap();
        let handle = Arc::new(context.revalidation_handle().unwrap());
        drop(context);
        (issuer, caller, session_id, handle)
    }

    #[derive(Clone)]
    struct TestRecoveryCaller {
        principal: PrincipalId,
        context: CallerContextId,
    }

    impl TestRecoveryCaller {
        fn new() -> Self {
            Self {
                principal: PrincipalId::new("system.identity-recovery-test").unwrap(),
                context: CallerContextId::new("caller_identity_recovery_test").unwrap(),
            }
        }
    }

    impl TrustedCallerContext for TestRecoveryCaller {
        fn principal_id(&self) -> &PrincipalId {
            &self.principal
        }

        fn context_id(&self) -> &CallerContextId {
            &self.context
        }
    }

    impl AuthenticatedSystemServicePrincipal for TestRecoveryCaller {}
    impl super::caller_proof_seal::SystemService for TestRecoveryCaller {}

    #[derive(Debug)]
    struct RecoveryBusyObservation {
        result: Result<RecoveryReport, IdentityError>,
        successful_guard_acquisitions: usize,
        store_state: SnapshotStoreState,
        last_known_good: Option<StoredSnapshot>,
        snapshot: IdentitySnapshot,
        store_version: Option<SnapshotVersion>,
        recovered: bool,
        active_by_caller: HashMap<CallerKey, SessionId>,
        last_by_caller: HashMap<CallerKey, SessionId>,
        session_owners: HashMap<SessionId, CallerKey>,
        idempotency: HashMap<(CallerKey, IdempotencyKey), (SessionId, SessionIntent)>,
        lease_ids: Vec<SessionId>,
        lease_states: HashMap<SessionId, (NonZeroU64, u64, Option<bool>)>,
        released_preceding_guards: Vec<SessionId>,
        held_lease_still_busy: bool,
        old_handle_checks: Vec<Result<(), ResolutionError>>,
    }

    #[derive(Debug)]
    struct RecoverySuccessObservation {
        result: Result<RecoveryReport, IdentityError>,
        successful_guard_acquisitions: usize,
        store_state: SnapshotStoreState,
        last_known_good: Option<StoredSnapshot>,
        snapshot: IdentitySnapshot,
        store_version: Option<SnapshotVersion>,
        recovered: bool,
        active_by_caller_len: usize,
        last_by_caller_len: usize,
        session_owners_len: usize,
        lease_count: usize,
        old_handle_checks: Vec<Result<(), ResolutionError>>,
        old_handle_snapshots: Vec<Result<LiveSnapshot, TryLiveError>>,
    }

    fn multi_session_recovery_contention_probe_child() {
        if std::env::var_os(PROBE_READY_ENV).is_none() {
            return;
        }

        let recovery_caller = TestRecoveryCaller::new();
        let mut issuer = memory_service(TestClock::at(100));
        issuer.recover(&recovery_caller).unwrap();

        let mut handles = Vec::new();
        let mut session_ids = Vec::new();
        for index in 0..3 {
            let caller = TestCaller::new(&format!("multi-recovery-{index}"));
            let (user_id, profile_id) = issuer
                .create_local_user(
                    &caller,
                    LocalUserMetadata::new(format!("Recovery owner {index}")).unwrap(),
                    ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
                )
                .unwrap();
            let session_id = issuer
                .login_local(
                    &caller,
                    user_id,
                    profile_id,
                    None,
                    IdempotencyKey::new(format!("multi-recovery-{index}")).unwrap(),
                )
                .unwrap();
            let context = issuer.interactive_session(&caller).unwrap();
            handles.push(Arc::new(context.revalidation_handle().unwrap()));
            session_ids.push(session_id);
        }
        assert_eq!(handles.len(), 3);
        for handle in &handles {
            assert_eq!(handle.check_live(), Ok(()));
        }

        // Recover iterates this exact HashMap's values in the same table order
        // as keys. Holding the final lease forces it to acquire all preceding
        // write guards before it reaches the contended lease.
        let recovery_order = issuer.leases.keys().cloned().collect::<Vec<_>>();
        assert_eq!(recovery_order.len(), session_ids.len());
        let held_session_id = recovery_order.last().unwrap().clone();
        let held_handle = handles
            .iter()
            .find(|handle| handle.session_id() == &held_session_id)
            .unwrap()
            .clone();
        let preceding_session_ids = recovery_order[..recovery_order.len() - 1].to_vec();

        let before_snapshot = issuer.snapshot.clone();
        let before_store_state = issuer.store.load_current_state().unwrap();
        let before_last_known_good = issuer.store.load_last_known_good().unwrap();
        let before_store_version = issuer.store_version;
        let before_recovered = issuer.recovered;
        let before_active_by_caller = issuer.active_by_caller.clone();
        let before_last_by_caller = issuer.last_by_caller.clone();
        let before_session_owners = issuer.session_owners.clone();
        let before_idempotency = issuer
            .idempotency
            .iter()
            .map(|(key, session)| {
                (
                    key.clone(),
                    (session.session_id.clone(), session.intent.clone()),
                )
            })
            .collect::<HashMap<_, _>>();
        let before_lease_states = issuer
            .leases
            .iter()
            .map(|(session_id, lease)| {
                (
                    session_id.clone(),
                    (
                        lease.generation,
                        lease.revision.load(Ordering::Acquire),
                        lease.live.try_read().ok().map(|live| *live),
                    ),
                )
            })
            .collect::<HashMap<_, _>>();
        let before_lease_ids = recovery_order.clone();

        let service = Arc::new(Mutex::new(issuer));
        let worker_service = Arc::clone(&service);
        let worker_handles = handles.clone();
        let worker_recovery_caller = recovery_caller.clone();
        let worker_preceding_session_ids = preceding_session_ids.clone();
        let worker_held_session_id = held_session_id.clone();
        let (worker_ready_tx, worker_ready_rx) = mpsc::sync_channel(0);
        let (attempt_tx, attempt_rx) = mpsc::sync_channel(0);
        let (busy_tx, busy_rx) = mpsc::sync_channel(0);
        let (retry_tx, retry_rx) = mpsc::sync_channel(0);
        let (success_tx, success_rx) = mpsc::sync_channel(0);

        let worker = thread::spawn(move || {
            worker_ready_tx.send(()).unwrap();
            attempt_rx
                .recv_timeout(Duration::from_secs(3))
                .expect("parent must hold a Session writer gate before recovery starts");

            let busy = {
                let mut issuer = worker_service.lock().unwrap();
                let result = issuer.recover(&worker_recovery_caller);
                let successful_guard_acquisitions = RECOVERY_WRITE_GUARDS_ACQUIRED.with(Cell::get);
                let released_preceding_guards = worker_preceding_session_ids
                    .iter()
                    .filter_map(|session_id| {
                        let lease = issuer.leases.get(session_id)?;
                        match lease.live.try_write() {
                            Ok(guard) => {
                                drop(guard);
                                Some(session_id.clone())
                            }
                            Err(_) => None,
                        }
                    })
                    .collect();
                let held_lease_still_busy = matches!(
                    issuer
                        .leases
                        .get(&worker_held_session_id)
                        .unwrap()
                        .live
                        .try_write(),
                    Err(std::sync::TryLockError::WouldBlock)
                );
                let old_handle_checks = worker_handles
                    .iter()
                    .map(|handle| handle.check_live())
                    .collect();

                RecoveryBusyObservation {
                    result,
                    successful_guard_acquisitions,
                    store_state: issuer.store.load_current_state().unwrap(),
                    last_known_good: issuer.store.load_last_known_good().unwrap(),
                    snapshot: issuer.snapshot.clone(),
                    store_version: issuer.store_version,
                    recovered: issuer.recovered,
                    active_by_caller: issuer.active_by_caller.clone(),
                    last_by_caller: issuer.last_by_caller.clone(),
                    session_owners: issuer.session_owners.clone(),
                    idempotency: issuer
                        .idempotency
                        .iter()
                        .map(|(key, session)| {
                            (
                                key.clone(),
                                (session.session_id.clone(), session.intent.clone()),
                            )
                        })
                        .collect(),
                    lease_ids: issuer.leases.keys().cloned().collect(),
                    lease_states: issuer
                        .leases
                        .iter()
                        .map(|(session_id, lease)| {
                            (
                                session_id.clone(),
                                (
                                    lease.generation,
                                    lease.revision.load(Ordering::Acquire),
                                    lease.live.try_read().ok().map(|live| *live),
                                ),
                            )
                        })
                        .collect(),
                    released_preceding_guards,
                    held_lease_still_busy,
                    old_handle_checks,
                }
            };
            busy_tx.send(busy).unwrap();
            retry_rx
                .recv_timeout(Duration::from_secs(4))
                .expect("recovery retry must wait until the admitted effect returns");

            let success = {
                let mut issuer = worker_service.lock().unwrap();
                let result = issuer.recover(&worker_recovery_caller);
                let successful_guard_acquisitions = RECOVERY_WRITE_GUARDS_ACQUIRED.with(Cell::get);
                let old_handle_checks = worker_handles
                    .iter()
                    .map(|handle| handle.check_live())
                    .collect();
                let old_handle_snapshots = worker_handles
                    .iter()
                    .map(|handle| handle.try_live_snapshot())
                    .collect();
                RecoverySuccessObservation {
                    result,
                    successful_guard_acquisitions,
                    store_state: issuer.store.load_current_state().unwrap(),
                    last_known_good: issuer.store.load_last_known_good().unwrap(),
                    snapshot: issuer.snapshot.clone(),
                    store_version: issuer.store_version,
                    recovered: issuer.recovered,
                    active_by_caller_len: issuer.active_by_caller.len(),
                    last_by_caller_len: issuer.last_by_caller.len(),
                    session_owners_len: issuer.session_owners.len(),
                    lease_count: issuer.leases.len(),
                    old_handle_checks,
                    old_handle_snapshots,
                }
            };
            success_tx.send(success).unwrap();
        });

        worker_ready_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("recovery worker must be ready before gate contention");
        let held_writer_gate = held_handle.identity.live.live.write().unwrap();
        mark_probe_ready();
        attempt_tx.send(()).unwrap();
        let busy = busy_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("multi-Session recovery must return Busy without waiting for a writer gate");

        assert_eq!(busy.result, Err(IdentityError::SessionLeaseBusy));
        assert_eq!(
            busy.successful_guard_acquisitions,
            preceding_session_ids.len()
        );
        assert_eq!(busy.released_preceding_guards, preceding_session_ids);
        assert!(busy.held_lease_still_busy);
        assert_eq!(busy.store_state, before_store_state);
        assert_eq!(busy.last_known_good, before_last_known_good);
        assert_eq!(busy.snapshot, before_snapshot);
        assert_eq!(busy.store_version, before_store_version);
        assert_eq!(busy.recovered, before_recovered);
        assert_eq!(busy.active_by_caller, before_active_by_caller);
        assert_eq!(busy.last_by_caller, before_last_by_caller);
        assert_eq!(busy.session_owners, before_session_owners);
        assert_eq!(busy.idempotency, before_idempotency);
        assert_eq!(busy.lease_ids, before_lease_ids);
        assert_eq!(busy.lease_ids, recovery_order);
        let mut expected_lease_states = before_lease_states.clone();
        expected_lease_states.get_mut(&held_session_id).unwrap().2 = None;
        assert_eq!(busy.lease_states, expected_lease_states);
        assert!(busy.lease_states.iter().all(|(session_id, (_, _, live))| {
            if session_id == &held_session_id {
                live.is_none()
            } else {
                *live == Some(true)
            }
        }));
        let expected_handle_checks = handles
            .iter()
            .map(|handle| {
                if handle.session_id() == &held_session_id {
                    Err(ResolutionError::Unavailable)
                } else {
                    Ok(())
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(busy.old_handle_checks, expected_handle_checks);

        drop(held_writer_gate);

        for handle in &handles {
            assert_eq!(
                handle.check_live(),
                Ok(()),
                "Busy must not partially revoke any issued Session"
            );
        }

        retry_tx.send(()).unwrap();
        let success = success_rx
            .recv_timeout(Duration::from_secs(4))
            .expect("explicit recovery retry must complete after effect release");
        worker.join().unwrap();

        let report = success.result.unwrap();
        assert_eq!(report.source, RecoverySource::Current);
        assert_eq!(report.interrupted_sessions_ended, session_ids.len());
        assert_eq!(report.guest_cleanups_pending, 0);
        assert_eq!(success.successful_guard_acquisitions, session_ids.len());
        assert!(success.recovered);
        assert_eq!(success.active_by_caller_len, 0);
        assert_eq!(success.last_by_caller_len, session_ids.len());
        assert_eq!(success.session_owners_len, 0);
        assert_eq!(success.lease_count, 0);
        assert_eq!(success.snapshot.revision, before_snapshot.revision + 1);
        assert_eq!(
            success.store_state.high_water_version.unwrap().get(),
            before_store_state.high_water_version.unwrap().get() + 1
        );
        assert_eq!(
            success.store_state.current.as_ref().unwrap().version,
            success.store_state.high_water_version.unwrap()
        );
        assert_eq!(
            success.store_version,
            success.store_state.high_water_version
        );
        assert_eq!(success.last_known_good, before_store_state.current);
        assert!(success.snapshot.sessions.iter().all(|session| matches!(
            &session.lifecycle,
            SessionLifecycle::Ended {
                reason: SessionEndReason::SystemRestart,
                ..
            }
        )));
        for session_id in &session_ids {
            assert!(success
                .snapshot
                .sessions
                .iter()
                .any(|session| &session.session_id == session_id));
        }
        assert_eq!(
            success.old_handle_checks,
            vec![Err(ResolutionError::Ended); handles.len()],
            "explicit recovery ends the old authority; it does not revive it"
        );
        assert!(success.old_handle_snapshots.iter().all(|snapshot| matches!(
            snapshot,
            Ok(snapshot) if snapshot.status() == LiveStatus::Revoked
        )));
    }

    #[test]
    #[ignore = "spawned only by the bounded parent watchdog"]
    fn multi_session_recovery_contention_probe() {
        multi_session_recovery_contention_probe_child();
    }

    #[test]
    fn multi_session_recovery_busy_is_bounded_atomic_and_retryable() {
        match run_ignored_probe(
            "multi_session_recovery_contention_probe",
            Duration::from_secs(8),
        ) {
            ProbeOutcome::AfterReadyExit(status) if status.success() => {}
            outcome => panic!("multi-Session recovery contention probe failed: {outcome:?}"),
        }
    }

    #[test]
    fn poisoned_session_gate_fails_closed() {
        let (issuer, _caller, _session_id, handle) = memory_session("writer-poison");
        let live = Arc::clone(&handle.identity.live);
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _writer = live.live.write().unwrap();
            panic!("poison the session writer gate");
        }));
        assert!(poisoned.is_err());
        assert_eq!(handle.check_live(), Err(ResolutionError::Unavailable));
        assert!(!handle.identity.is_live());
        assert_eq!(
            handle.try_live_snapshot(),
            Err(TryLiveError::StateUnavailable)
        );
        let mut effect_invoked = false;
        assert_eq!(
            handle.with_live_session_effect(|| effect_invoked = true),
            Err(ResolutionError::Unavailable)
        );
        assert!(!effect_invoked);
        drop(issuer);
    }

    #[test]
    fn poisoned_admission_mutex_denies_and_shutdown_closes_it() {
        let (issuer, _caller, _session_id, handle) = memory_session("admission-poison");
        let admission = Arc::clone(&handle.identity.issuer_admission);
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _state = admission.state.lock().unwrap();
            panic!("poison issuer admission mutex");
        }));
        assert!(poisoned.is_err());
        assert_eq!(handle.check_live(), Err(ResolutionError::Unavailable));
        assert_eq!(
            handle.try_live_snapshot(),
            Err(TryLiveError::StateUnavailable)
        );
        drop(issuer);
        let shutdown_state = match admission.state.lock() {
            Ok(_) => panic!("shutdown unexpectedly cleared admission poison"),
            Err(poisoned) => poisoned.into_inner(),
        };
        assert!(
            shutdown_state.closed,
            "shutdown must close poisoned admission"
        );
        assert!(!handle.identity.issuer_alive.load(Ordering::Acquire));
        drop(shutdown_state);
        assert_eq!(
            handle.try_live_snapshot(),
            Err(TryLiveError::StateUnavailable),
            "shutdown closes admission even when its mutex was poisoned"
        );
    }

    #[test]
    fn interactive_context_binds_effect_and_profile_root_to_same_caller_and_profile() {
        let (mut issuer, caller, session_id, _old_handle) =
            memory_session("interactive-profile-context");
        let context = issuer.interactive_session(&caller).unwrap();
        let identity = context.identity().unwrap();
        let effect_handle = context.revalidation_handle().unwrap();
        let access = IdentityAccessContext::new(identity, &caller);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut storage = UserStorageRootAdapter::new(ProfileContextProbeBackend {
            calls: Arc::clone(&calls),
        });
        let mut root_authorizer = NoopProfileRootAuthorizer;
        let profile_id = identity.profile_id().clone();
        let root = storage
            .resolve_root(&access, &profile_id, &mut root_authorizer)
            .unwrap();

        assert_eq!(effect_handle.session_id(), identity.session_id());
        assert_eq!(effect_handle.principal_id(), caller.principal_id());
        assert_eq!(effect_handle.caller_context_id(), caller.context_id());
        assert_eq!(effect_handle.profile_id(), &profile_id);
        let mut effect_called = false;
        assert_eq!(
            effect_handle.with_live_session_effect(|| effect_called = true),
            Ok(())
        );
        assert!(effect_called);
        assert_eq!(
            storage.read_object(
                &access,
                &root,
                &profile_id,
                nagi_model::ObjectId(1),
                &mut root_authorizer,
            ),
            Ok(Vec::new())
        );
        assert_eq!(*calls.lock().unwrap(), vec!["open", "read"]);

        let other_caller = TestCaller::new("interactive-profile-other-caller");
        let other_access = IdentityAccessContext::new(identity, &other_caller);
        assert_eq!(
            storage.read_object(
                &other_access,
                &root,
                &profile_id,
                nagi_model::ObjectId(1),
                &mut root_authorizer,
            ),
            Err(StorageRootFailure::CallerMismatch)
        );

        let other_profile = ProfileId::new("profile_unbound").unwrap();
        assert_eq!(
            storage.read_object(
                &access,
                &root,
                &other_profile,
                nagi_model::ObjectId(1),
                &mut root_authorizer,
            ),
            Err(StorageRootFailure::CrossProfile)
        );
        assert_eq!(
            *calls.lock().unwrap(),
            vec!["open", "read"],
            "mismatched caller/profile must be denied before backend access"
        );

        assert_eq!(
            issuer.end_session_with_storage(&caller, &session_id, &mut storage),
            Ok(EndSessionResult::Ended)
        );
        let mut stale_effect_called = false;
        assert_eq!(
            effect_handle.with_live_session_effect(|| stale_effect_called = true),
            Err(ResolutionError::Ended)
        );
        assert!(!stale_effect_called);
        assert_eq!(
            storage.read_object(
                &access,
                &root,
                &profile_id,
                nagi_model::ObjectId(1),
                &mut root_authorizer,
            ),
            Err(StorageRootFailure::StaleSession)
        );
        assert_eq!(*calls.lock().unwrap(), vec!["open", "read"]);
    }

    #[test]
    fn end_session_with_storage_drops_only_the_exact_session_root() {
        let (mut issuer, caller, session_id, _handle) = memory_session("profile-root-end-cleanup");
        let identity =
            CurrentIdentityResolver::resolve_current_identity(&mut issuer, &caller).unwrap();
        let access = IdentityAccessContext::new(&identity, &caller);
        let drops = Arc::new(AtomicU64::new(0));
        let mut storage = UserStorageRootAdapter::new(DropProbeBackend {
            opened: Arc::new(AtomicU64::new(0)),
            dropped: Arc::clone(&drops),
        });
        let mut authorizer = NoopProfileRootAuthorizer;
        let root = storage
            .resolve_root(&access, identity.profile_id(), &mut authorizer)
            .unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 0);

        assert_eq!(
            issuer.end_session_with_storage(&caller, &session_id, &mut storage),
            Ok(EndSessionResult::Ended)
        );
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            storage.read_object(
                &access,
                &root,
                identity.profile_id(),
                nagi_model::ObjectId(1),
                &mut authorizer,
            ),
            Err(StorageRootFailure::StaleSession)
        );
    }

    #[test]
    fn confirmed_expiry_reclaims_its_profile_root() {
        let clock = CleanupTestClock::at(100);
        let caller = TestCaller::new("profile-root-expiry-cleanup");
        let mut issuer = IdentityService::new(
            InMemorySnapshotStore::default(),
            TestIds::default(),
            clock.clone(),
            TestAuthorizer::default(),
            TestCleaner::default(),
            NoopIdentityEventSink,
        );
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Cleanup probe").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        let session_id = issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                Some(UnixMillis::new(200)),
                IdempotencyKey::new("profile-root-expiry-cleanup").unwrap(),
            )
            .unwrap();
        let identity =
            CurrentIdentityResolver::resolve_current_identity(&mut issuer, &caller).unwrap();
        let access = IdentityAccessContext::new(&identity, &caller);
        let drops = Arc::new(AtomicU64::new(0));
        let mut storage = UserStorageRootAdapter::new(DropProbeBackend {
            opened: Arc::new(AtomicU64::new(0)),
            dropped: Arc::clone(&drops),
        });
        let mut authorizer = NoopProfileRootAuthorizer;
        storage
            .resolve_root(&access, identity.profile_id(), &mut authorizer)
            .unwrap();
        clock.set(200);

        assert!(matches!(
            issuer.resolve_current_identity_with_storage(&caller, &mut storage),
            Err(ResolutionError::Expired)
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            issuer.end_session_with_storage(&caller, &session_id, &mut storage),
            Ok(EndSessionResult::AlreadyEnded)
        );
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn recovery_attempt_that_quarantines_issuer_reclaims_its_profile_roots() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("profile-root-recovery-cleanup");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Recovery cleanup").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("profile-root-recovery-cleanup").unwrap(),
            )
            .unwrap();
        let identity =
            CurrentIdentityResolver::resolve_current_identity(&mut issuer, &caller).unwrap();
        let access = IdentityAccessContext::new(&identity, &caller);
        let drops = Arc::new(AtomicU64::new(0));
        let mut storage = UserStorageRootAdapter::new(DropProbeBackend {
            opened: Arc::new(AtomicU64::new(0)),
            dropped: Arc::clone(&drops),
        });
        let mut authorizer = NoopProfileRootAuthorizer;
        storage
            .resolve_root(&access, identity.profile_id(), &mut authorizer)
            .unwrap();

        let binding = RecoveryBinding::for_test([0x6b; 32]);
        let mut gate = CountingRecoveryGate::for_binding(binding);
        let issuer_admission = Arc::clone(&issuer.issuer.admission);
        let admission_guard = issuer_admission.state.lock().unwrap();
        assert_eq!(
            issuer.recover_with_new_intent_and_storage(&caller, &mut gate, &mut storage),
            Err(IdentityError::SessionLeaseBusy),
            "transient adapter-owner mutex contention must remain retryable Busy"
        );
        assert_eq!(gate.invocations, 0);
        assert!(issuer.store.recovery_journal_record().unwrap().is_none());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(admission_guard);

        let writer = identity.live.live.write().unwrap();
        assert_eq!(
            issuer.recover_with_new_intent_and_storage(&caller, &mut gate, &mut storage),
            Err(IdentityError::SessionLeaseBusy),
            "recovery preflight contention must preserve roots and remain retryable"
        );
        assert_eq!(gate.invocations, 0);
        assert!(issuer.store.recovery_journal_record().unwrap().is_none());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(writer);

        assert_eq!(
            issuer.recover_with_new_intent_and_storage(&caller, &mut gate, &mut storage),
            Err(IdentityError::StoreUnavailable),
            "runtime-active recovery remains quarantined until the real owner gate is connected"
        );
        assert_eq!(gate.successful_callbacks, 1);
        assert!(!issuer.recovered);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            identity.check_live(),
            Err(ResolutionError::Unavailable),
            "post-claim recovery permanently closes the old issuer before root cleanup"
        );
        let admission = issuer.issuer.admission.state.lock().unwrap();
        assert!(admission.closed);
        assert!(!admission.recovering);
        assert!(!issuer.issuer.alive.load(Ordering::Acquire));
    }

    #[test]
    fn adapter_identity_prevents_same_issuer_handle_reuse_before_authorizer() {
        let (mut issuer, caller, _session_id, _handle) =
            memory_session("profile-root-adapter-identity");
        let identity =
            CurrentIdentityResolver::resolve_current_identity(&mut issuer, &caller).unwrap();
        let access = IdentityAccessContext::new(&identity, &caller);
        let first_calls = Arc::new(Mutex::new(Vec::new()));
        let second_calls = Arc::new(Mutex::new(Vec::new()));
        let mut first = UserStorageRootAdapter::new(ProfileContextProbeBackend {
            calls: Arc::clone(&first_calls),
        });
        let mut second = UserStorageRootAdapter::new(ProfileContextProbeBackend {
            calls: Arc::clone(&second_calls),
        });
        let mut authorizer = NoopProfileRootAuthorizer;
        let root = first
            .resolve_root(&access, identity.profile_id(), &mut authorizer)
            .unwrap();
        let mut foreign_authorizer_called = false;
        let mut foreign_authorizer = CallbackProfileRootAuthorizer(|| {
            foreign_authorizer_called = true;
            Ok(())
        });

        assert_eq!(
            second.read_object(
                &access,
                &root,
                identity.profile_id(),
                nagi_model::ObjectId(1),
                &mut foreign_authorizer,
            ),
            Err(StorageRootFailure::InvalidHandle)
        );
        assert!(!foreign_authorizer_called);
        assert_eq!(*first_calls.lock().unwrap(), vec!["open"]);
        assert!(second_calls.lock().unwrap().is_empty());
        let mut second_authorizer_called = false;
        let mut second_authorizer = CallbackProfileRootAuthorizer(|| {
            second_authorizer_called = true;
            Ok(())
        });
        assert_eq!(
            second
                .resolve_root(&access, identity.profile_id(), &mut second_authorizer,)
                .unwrap_err(),
            StorageRootFailure::InvalidHandle,
            "one live C1 issuer cannot move its storage ownership to another adapter"
        );
        assert!(!second_authorizer_called);
        assert!(second_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn storage_cleanup_preserves_roots_for_another_issuer_with_same_session_id() {
        let (mut issuer, caller, session_id, _handle) =
            memory_session("profile-root-same-id-foreign-issuer");
        let identity =
            CurrentIdentityResolver::resolve_current_identity(&mut issuer, &caller).unwrap();
        let access = IdentityAccessContext::new(&identity, &caller);
        let (mut foreign_issuer, foreign_caller, foreign_session_id, _foreign_handle) =
            memory_session("profile-root-same-id-foreign-issuer");
        assert_eq!(foreign_session_id, session_id);
        assert_eq!(foreign_caller.principal_id(), caller.principal_id());
        assert_eq!(foreign_caller.context_id(), caller.context_id());
        let foreign_identity =
            CurrentIdentityResolver::resolve_current_identity(&mut foreign_issuer, &foreign_caller)
                .unwrap();
        assert_eq!(foreign_identity.profile_id(), identity.profile_id());
        let foreign_access = IdentityAccessContext::new(&foreign_identity, &foreign_caller);

        let opened = Arc::new(AtomicU64::new(0));
        let drops = Arc::new(AtomicU64::new(0));
        let backend = DropProbeBackend {
            opened: Arc::clone(&opened),
            dropped: Arc::clone(&drops),
        };
        let mut shared_adapter = UserStorageRootAdapter::new(backend.clone());
        let mut authorizer = NoopProfileRootAuthorizer;
        let root = shared_adapter
            .resolve_root(&access, identity.profile_id(), &mut authorizer)
            .unwrap();
        assert_eq!(opened.load(Ordering::SeqCst), 1);
        let foreign_root = shared_adapter
            .resolve_root(
                &foreign_access,
                foreign_identity.profile_id(),
                &mut authorizer,
            )
            .expect("the same adapter may retain multiple issuer-owned roots");
        assert_eq!(opened.load(Ordering::SeqCst), 2);
        let mut foreign_authorizer_called = false;
        let mut foreign_authorizer = CallbackProfileRootAuthorizer(|| {
            foreign_authorizer_called = true;
            Ok(())
        });
        assert_eq!(
            shared_adapter.read_object(
                &foreign_access,
                &root,
                foreign_identity.profile_id(),
                nagi_model::ObjectId(1),
                &mut foreign_authorizer,
            ),
            Err(StorageRootFailure::InvalidHandle)
        );
        assert!(
            !foreign_authorizer_called,
            "foreign issuer handles are rejected before invoking policy callbacks"
        );
        assert_eq!(
            shared_adapter.revoke_session(&foreign_identity),
            1,
            "cleanup selects the exact foreign issuer token despite colliding Session IDs"
        );
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(identity.check_live(), Ok(()));
        assert_eq!(
            foreign_issuer.end_session_with_storage(
                &foreign_caller,
                &foreign_session_id,
                &mut shared_adapter,
            ),
            Ok(EndSessionResult::Ended)
        );
        assert_eq!(foreign_identity.check_live(), Err(ResolutionError::Ended));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            shared_adapter.read_object(
                &access,
                &root,
                identity.profile_id(),
                nagi_model::ObjectId(1),
                &mut authorizer,
            ),
            Ok(Vec::new()),
            "ending the colliding foreign Session must preserve this issuer's root"
        );
        assert_eq!(
            shared_adapter.read_object(
                &foreign_access,
                &foreign_root,
                foreign_identity.profile_id(),
                nagi_model::ObjectId(1),
                &mut authorizer,
            ),
            Err(StorageRootFailure::StaleSession)
        );

        assert_eq!(
            issuer.end_session_with_storage(&caller, &session_id, &mut shared_adapter),
            Ok(EndSessionResult::Ended)
        );
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(identity.check_live(), Err(ResolutionError::Ended));
        assert_eq!(
            shared_adapter.read_object(
                &access,
                &root,
                identity.profile_id(),
                nagi_model::ObjectId(1),
                &mut authorizer,
            ),
            Err(StorageRootFailure::StaleSession)
        );
    }

    #[test]
    fn issuer_shutdown_with_storage_drops_roots_and_closes_old_handles() {
        let (mut issuer, caller, _session_id, old_handle) =
            memory_session("profile-root-shutdown-cleanup");
        let identity =
            CurrentIdentityResolver::resolve_current_identity(&mut issuer, &caller).unwrap();
        let access = IdentityAccessContext::new(&identity, &caller);
        let drops = Arc::new(AtomicU64::new(0));
        let mut storage = UserStorageRootAdapter::new(DropProbeBackend {
            opened: Arc::new(AtomicU64::new(0)),
            dropped: Arc::clone(&drops),
        });
        let mut authorizer = NoopProfileRootAuthorizer;
        storage
            .resolve_root(&access, identity.profile_id(), &mut authorizer)
            .unwrap();

        let parts = match issuer.into_parts_with_storage(&mut storage) {
            Ok(parts) => parts,
            Err(_) => panic!("idle issuer must close and clean its roots"),
        };
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(old_handle.check_live(), Err(ResolutionError::Unavailable));
        drop(parts);
    }

    fn profile_storage_cleanup_busy_probe_child() {
        if std::env::var_os(PROBE_READY_ENV).is_none() {
            return;
        }

        let (mut service, caller, session_id, handle) = memory_session("profile-root-cleanup-busy");
        let identity =
            CurrentIdentityResolver::resolve_current_identity(&mut service, &caller).unwrap();
        let access = IdentityAccessContext::new(&identity, &caller);
        let service = Arc::new(Mutex::new(service));
        let drops = Arc::new(AtomicU64::new(0));
        let backend = DropProbeBackend {
            opened: Arc::new(AtomicU64::new(0)),
            dropped: Arc::clone(&drops),
        };
        let mut storage = UserStorageRootAdapter::new(backend);
        let mut authorizer = NoopProfileRootAuthorizer;
        let _root = storage
            .resolve_root(&access, identity.profile_id(), &mut authorizer)
            .unwrap();
        mark_probe_ready();

        assert_eq!(
            handle.with_live_session_effect(|| {
                {
                    let mut owner = service.lock().unwrap();
                    let placeholder = memory_service(TestClock::at(100));
                    let active_issuer = std::mem::replace(&mut *owner, placeholder);
                    let active_issuer = match active_issuer.into_parts_with_storage(&mut storage) {
                        Err(active_issuer) => active_issuer,
                        Ok(_) => panic!("shutdown must remain retryable during a live effect"),
                    };
                    *owner = active_issuer;
                }
                assert_eq!(
                    drops.load(Ordering::SeqCst),
                    0,
                    "busy issuer shutdown must preserve roots for an explicit retry"
                );
                let service = Arc::clone(&service);
                let caller = caller.clone();
                let session_id = session_id.clone();
                std::thread::scope(|scope| {
                    let storage = &mut storage;
                    let cleanup = scope.spawn(move || {
                        service.lock().unwrap().end_session_with_storage(
                            &caller,
                            &session_id,
                            storage,
                        )
                    });
                    assert_eq!(
                        cleanup.join().unwrap(),
                        Err(IdentityError::SessionLeaseBusy),
                        "a live effect must make cleanup retryable before root removal"
                    );
                });
                assert_eq!(drops.load(Ordering::SeqCst), 0);
            }),
            Ok(())
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert_eq!(
            service
                .lock()
                .unwrap()
                .end_session_with_storage(&caller, &session_id, &mut storage,),
            Ok(EndSessionResult::Ended)
        );
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    #[ignore = "spawned only by the bounded parent watchdog"]
    fn profile_storage_cleanup_busy_probe() {
        profile_storage_cleanup_busy_probe_child();
    }

    #[test]
    fn profile_storage_cleanup_is_retryable_while_effect_lease_is_active() {
        match run_ignored_probe("profile_storage_cleanup_busy_probe", Duration::from_secs(8)) {
            ProbeOutcome::AfterReadyExit(status) if status.success() => {}
            outcome => panic!("profile-root cleanup busy/retry probe failed: {outcome:?}"),
        }
    }

    #[test]
    fn adapter_owner_mutex_contention_is_retryable_before_session_end() {
        let (mut issuer, caller, session_id, _handle) =
            memory_session("profile-root-owner-check-contention");
        let identity =
            CurrentIdentityResolver::resolve_current_identity(&mut issuer, &caller).unwrap();
        let access = IdentityAccessContext::new(&identity, &caller);
        let drops = Arc::new(AtomicU64::new(0));
        let mut storage = UserStorageRootAdapter::new(DropProbeBackend {
            opened: Arc::new(AtomicU64::new(0)),
            dropped: Arc::clone(&drops),
        });
        let mut authorizer = NoopProfileRootAuthorizer;
        storage
            .resolve_root(&access, identity.profile_id(), &mut authorizer)
            .unwrap();

        let issuer_admission = Arc::clone(&issuer.issuer.admission);
        let admission_guard = issuer_admission.state.lock().unwrap();
        assert_eq!(
            issuer.end_session_with_storage(&caller, &session_id, &mut storage),
            Err(IdentityError::SessionLeaseBusy),
            "try-lock contention must not be misreported as an adapter owner mismatch"
        );
        assert!(matches!(
            issuer.snapshot().sessions[0].lifecycle,
            SessionLifecycle::Active
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(admission_guard);

        assert_eq!(
            issuer.end_session_with_storage(&caller, &session_id, &mut storage),
            Ok(EndSessionResult::Ended)
        );
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        assert_eq!(identity.check_live(), Err(ResolutionError::Ended));
    }

    #[test]
    fn profile_root_effect_runs_root_io_inside_one_c1_session_lease() {
        let (mut issuer, caller, session_id, _old_handle) =
            memory_session("profile-root-single-lease");
        let context = issuer.interactive_session(&caller).unwrap();
        let identity = context.identity().unwrap();
        let effect_handle = context.revalidation_handle().unwrap();
        let access = IdentityAccessContext::new(identity, &caller);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut storage = UserStorageRootAdapter::new(ProfileContextProbeBackend {
            calls: Arc::clone(&calls),
        });
        let mut root_authorizer = NoopProfileRootAuthorizer;
        let profile_id = identity.profile_id().clone();
        let root = storage
            .resolve_root(&access, &profile_id, &mut root_authorizer)
            .unwrap();

        let effect_result = storage.with_live_root_effect(
            &access,
            &root,
            &profile_id,
            &mut root_authorizer,
            |root_io| {
                let nested_effect = effect_handle.with_live_session_effect(|| ());
                assert_eq!(nested_effect, Err(ResolutionError::Unavailable));
                root_io.write_object(nagi_model::ObjectId(7), b"note")?;
                assert_eq!(
                    root_io.read_object(nagi_model::ObjectId(7))?,
                    Vec::<u8>::new()
                );
                Ok(())
            },
        );
        assert_eq!(effect_result, Ok(()));
        assert_eq!(*calls.lock().unwrap(), vec!["open", "write", "read"]);

        let other_caller = TestCaller::new("profile-root-single-lease-other-caller");
        let other_access = IdentityAccessContext::new(identity, &other_caller);
        let mut denied_callback_called = false;
        assert_eq!(
            storage.with_live_root_effect(
                &other_access,
                &root,
                &profile_id,
                &mut root_authorizer,
                |_| {
                    denied_callback_called = true;
                    Ok(())
                },
            ),
            Err(StorageRootFailure::CallerMismatch)
        );
        assert!(!denied_callback_called);

        let other_profile = ProfileId::new("profile_other_single_lease").unwrap();
        let mut wrong_profile_callback_called = false;
        assert_eq!(
            storage.with_live_root_effect(
                &access,
                &root,
                &other_profile,
                &mut root_authorizer,
                |_| {
                    wrong_profile_callback_called = true;
                    Ok(())
                }
            ),
            Err(StorageRootFailure::CrossProfile)
        );
        assert!(!wrong_profile_callback_called);
        assert_eq!(*calls.lock().unwrap(), vec!["open", "write", "read"]);

        assert_eq!(
            issuer.end_session_with_storage(&caller, &session_id, &mut storage),
            Ok(EndSessionResult::Ended)
        );
        let mut stale_callback_called = false;
        assert_eq!(
            storage.with_live_root_effect(
                &access,
                &root,
                &profile_id,
                &mut root_authorizer,
                |_| {
                    stale_callback_called = true;
                    Ok(())
                },
            ),
            Err(StorageRootFailure::StaleSession)
        );
        assert!(!stale_callback_called);
        assert_eq!(*calls.lock().unwrap(), vec!["open", "write", "read"]);
    }

    #[test]
    fn profile_root_effect_revalidates_after_authorizer_revokes_session() {
        let (mut issuer, caller, session_id, _old_handle) =
            memory_session("profile-root-authorizer-revokes");
        let context = issuer.interactive_session(&caller).unwrap();
        let identity = context.identity().unwrap();
        let access = IdentityAccessContext::new(identity, &caller);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut storage = UserStorageRootAdapter::new(ProfileContextProbeBackend {
            calls: Arc::clone(&calls),
        });
        let profile_id = identity.profile_id().clone();
        let mut root_authorizer = NoopProfileRootAuthorizer;
        let root = storage
            .resolve_root(&access, &profile_id, &mut root_authorizer)
            .unwrap();

        let mut authorizer = CallbackProfileRootAuthorizer(|| {
            assert_eq!(
                issuer.end_session(&caller, &session_id),
                Ok(EndSessionResult::Ended)
            );
            Ok(())
        });
        let mut effect_called = false;
        assert_eq!(
            storage.with_live_root_effect(&access, &root, &profile_id, &mut authorizer, |_| {
                effect_called = true;
                Ok(())
            },),
            Err(StorageRootFailure::StaleSession)
        );
        assert!(!effect_called);
        assert_eq!(*calls.lock().unwrap(), vec!["open"]);
        assert_eq!(storage.revoke_session(identity), 1);
    }

    #[test]
    fn profile_root_effect_revalidates_after_authorizer_recovers_issuer() {
        let (mut issuer, caller, _session_id, _old_handle) =
            memory_session("profile-root-authorizer-recovers");
        let context = issuer.interactive_session(&caller).unwrap();
        let identity = context.identity().unwrap();
        let access = IdentityAccessContext::new(identity, &caller);
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut storage = UserStorageRootAdapter::new(ProfileContextProbeBackend {
            calls: Arc::clone(&calls),
        });
        let profile_id = identity.profile_id().clone();
        let mut root_authorizer = NoopProfileRootAuthorizer;
        let root = storage
            .resolve_root(&access, &profile_id, &mut root_authorizer)
            .unwrap();

        let recovery_caller = TestRecoveryCaller::new();
        let mut authorizer = CallbackProfileRootAuthorizer(|| {
            issuer
                .recover(&recovery_caller)
                .map(|_| ())
                .map_err(|_| StorageRootFailure::Unavailable)
        });
        let mut effect_called = false;
        assert_eq!(
            storage.with_live_root_effect(&access, &root, &profile_id, &mut authorizer, |_| {
                effect_called = true;
                Ok(())
            },),
            Err(StorageRootFailure::StaleSession)
        );
        assert!(!effect_called);
        assert_eq!(*calls.lock().unwrap(), vec!["open"]);
        assert_eq!(storage.revoke_session(identity), 1);
    }

    #[test]
    fn profile_root_effect_rejects_foreign_issuer_with_identical_session_fields() {
        let (mut issuer, caller, session_id, _old_handle) =
            memory_session("profile-root-foreign-issuer");
        let context = issuer.interactive_session(&caller).unwrap();
        let identity = context.identity().unwrap();
        let access = IdentityAccessContext::new(identity, &caller);
        let profile_id = identity.profile_id().clone();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut storage = UserStorageRootAdapter::new(ProfileContextProbeBackend {
            calls: Arc::clone(&calls),
        });
        let mut root_authorizer = NoopProfileRootAuthorizer;
        let root = storage
            .resolve_root(&access, &profile_id, &mut root_authorizer)
            .unwrap();

        let (mut foreign_issuer, foreign_caller, foreign_session_id, _foreign_handle) =
            memory_session("profile-root-foreign-issuer");
        assert_eq!(foreign_session_id, session_id);
        assert_eq!(foreign_caller.principal_id(), caller.principal_id());
        assert_eq!(foreign_caller.context_id(), caller.context_id());
        let foreign_context = foreign_issuer.interactive_session(&foreign_caller).unwrap();
        let foreign_identity = foreign_context.identity().unwrap();
        assert_eq!(foreign_identity.profile_id(), &profile_id);
        assert_eq!(foreign_identity.user_id(), identity.user_id());
        let foreign_access = IdentityAccessContext::new(foreign_identity, &foreign_caller);

        let mut effect_called = false;
        assert_eq!(
            storage.with_live_root_effect(
                &foreign_access,
                &root,
                &profile_id,
                &mut root_authorizer,
                |_| {
                    effect_called = true;
                    Ok(())
                },
            ),
            Err(StorageRootFailure::InvalidHandle)
        );
        assert!(!effect_called);
        assert_eq!(*calls.lock().unwrap(), vec!["open"]);
    }

    fn writer_held_live_state_probe_child() {
        if std::env::var_os(PROBE_READY_ENV).is_none() {
            return;
        }
        let (mut issuer, caller, session_id, handle) = memory_session("writer-held-status");
        let identity =
            CurrentIdentityResolver::resolve_current_identity(&mut issuer, &caller).unwrap();
        let access = IdentityAccessContext::new(&identity, &caller);
        let mut storage = UserStorageRootAdapter::new(NoopProfileStorageBackend);
        let mut root_authorizer = NoopProfileRootAuthorizer;
        let writer = handle.identity.live.live.write().unwrap();
        mark_probe_ready();
        assert_eq!(handle.check_live(), Err(ResolutionError::Unavailable));
        assert!(!handle.identity.is_live());
        assert_eq!(
            handle.try_live_snapshot(),
            Err(TryLiveError::StateUnavailable)
        );
        assert_eq!(
            crate::resolve_capability_principal(&crate::DenyCapabilityPrincipalAdapter, &identity,),
            Err(PrincipalMappingFailure::Unavailable),
            "a gate conflict must not be reported as a stale capability Session"
        );
        assert!(matches!(
            storage.resolve_root(&access, identity.profile_id(), &mut root_authorizer),
            Err(StorageRootFailure::Unavailable)
        ));

        drop(writer);
        assert_eq!(handle.check_live(), Ok(()));
        assert!(handle.identity.is_live());
        assert!(storage
            .resolve_root(&access, identity.profile_id(), &mut root_authorizer)
            .is_ok());

        let committed_version = issuer.store_version.unwrap();
        issuer.store_version = Some(crate::store::SnapshotVersion::new(
            committed_version.get().checked_add(1).unwrap(),
        ));
        assert_eq!(
            issuer.end_session(&caller, &session_id),
            Err(IdentityError::StoreConflict),
            "failed store commit must release the acquired write gate without revoking"
        );
        assert_eq!(handle.check_live(), Ok(()));
        assert!(handle.identity.is_live());

        issuer.store_version = Some(committed_version);
        assert_eq!(
            issuer.end_session(&caller, &session_id),
            Ok(EndSessionResult::Ended)
        );
        assert_eq!(handle.check_live(), Err(ResolutionError::Ended));
        assert_eq!(
            crate::resolve_capability_principal(&crate::DenyCapabilityPrincipalAdapter, &identity,),
            Err(PrincipalMappingFailure::StaleSession)
        );
        assert!(matches!(
            storage.resolve_root(&access, identity.profile_id(), &mut root_authorizer),
            Err(StorageRootFailure::StaleSession)
        ));
    }

    #[test]
    #[ignore = "spawned only by the bounded parent watchdog"]
    fn writer_held_live_state_probe() {
        writer_held_live_state_probe_child();
    }

    #[test]
    fn writer_held_unavailable_is_typed_and_live_recovers_after_failed_commit() {
        match run_ignored_probe("writer_held_live_state_probe", Duration::from_secs(6)) {
            ProbeOutcome::AfterReadyExit(status) if status.success() => {}
            outcome => {
                panic!("writer-held lease status probe failed: {outcome:?}")
            }
        }
    }

    fn lifecycle_reentry_probe_child() {
        if std::env::var_os(PROBE_READY_ENV).is_none() {
            return;
        }
        let (issuer, caller, session_id, handle) = memory_session("lifecycle-reentry");
        let issuer = Arc::new(Mutex::new(issuer));
        let result = handle.with_live_session_effect(|| {
            mark_probe_ready();
            let check_live = handle.check_live();
            let is_live = handle.identity.is_live();
            let live_snapshot = handle.try_live_snapshot();
            let mut nested_effect_invoked = false;
            let nested_effect = handle.with_live_session_effect(|| nested_effect_invoked = true);
            let lifecycle = issuer.lock().unwrap().end_session(&caller, &session_id);
            (
                check_live,
                is_live,
                live_snapshot,
                nested_effect,
                nested_effect_invoked,
                lifecycle,
            )
        });
        assert_eq!(
            result,
            Ok((
                Err(ResolutionError::Unavailable),
                false,
                Err(TryLiveError::StateUnavailable),
                Err(ResolutionError::Unavailable),
                false,
                Err(IdentityError::LeaseEffectReentry),
            )),
            "same-thread lease and lifecycle reentry must fail before self-wait"
        );
        assert_eq!(handle.check_live(), Ok(()));
    }

    #[test]
    #[ignore = "spawned only by the bounded parent watchdog"]
    fn lifecycle_reentry_probe() {
        lifecycle_reentry_probe_child();
    }

    #[test]
    fn lifecycle_reentry_fails_fast_in_watchdog_child() {
        match run_ignored_probe("lifecycle_reentry_probe", Duration::from_secs(4)) {
            ProbeOutcome::AfterReadyExit(status) if status.success() => {}
            outcome => panic!("same-thread lifecycle reentry was not rejected: {outcome:?}"),
        }
    }

    fn child_thread_end_session_probe_child() {
        if std::env::var_os(PROBE_READY_ENV).is_none() {
            return;
        }
        let (issuer, caller, session_id, handle) = memory_session("child-thread-end-session");
        let owner = Arc::new(Mutex::new(issuer));
        let effect_owner = Arc::clone(&owner);
        let effect_caller = caller.clone();
        let effect_session = session_id.clone();
        let effect_result = handle.with_live_session_effect(|| {
            mark_probe_ready();
            let child_owner = Arc::clone(&effect_owner);
            let child_caller = effect_caller.clone();
            let recovery_caller = effect_caller.clone();
            let child_session = effect_session.clone();
            thread::spawn(move || {
                let mut owner = child_owner.lock().unwrap();
                let end = owner.end_session(&child_caller, &child_session);
                let recover = owner.recover(&recovery_caller);
                (end, recover)
            })
            .join()
            .unwrap()
        });

        assert_eq!(
            effect_result,
            Ok((
                Err(IdentityError::SessionLeaseBusy),
                Err(IdentityError::SessionLeaseBusy),
            )),
            "child-thread end/recovery must fail fast instead of waiting on its parent lease"
        );
        assert_eq!(handle.check_live(), Ok(()));
        assert_eq!(
            owner.lock().unwrap().end_session(&caller, &session_id),
            Ok(EndSessionResult::Ended),
            "a retry after the admitted effect returns must commit the logout"
        );
        assert_eq!(handle.check_live(), Err(ResolutionError::Ended));
    }

    #[test]
    #[ignore = "spawned only by the bounded parent watchdog"]
    fn child_thread_end_session_probe() {
        child_thread_end_session_probe_child();
    }

    #[test]
    fn child_thread_end_session_fails_fast_in_watchdog_child() {
        match run_ignored_probe("child_thread_end_session_probe", Duration::from_secs(5)) {
            ProbeOutcome::AfterReadyExit(status) if status.success() => {}
            outcome => panic!("child-thread end_session was not rejected promptly: {outcome:?}"),
        }
    }

    fn issuer_self_drop_probe_child() {
        if std::env::var_os(PROBE_READY_ENV).is_none() {
            return;
        }
        let store_drops = Arc::new(AtomicU64::new(0));
        let (issuer, _caller, _session_id, handle) =
            drop_tracked_session("issuer-self-drop", Arc::clone(&store_drops), false);
        let alive = Arc::clone(&issuer.issuer.alive);
        let admission = Arc::clone(&issuer.issuer.admission);
        let owner = Arc::new(Mutex::new(Some(issuer)));
        let worker_handle = Arc::clone(&handle);
        let (worker_ready_tx, worker_ready_rx) = mpsc::sync_channel(0);
        let (worker_release_tx, worker_release_rx) = mpsc::sync_channel(0);
        let worker = thread::spawn(move || {
            worker_handle.with_live_session_effect(|| {
                worker_ready_tx.send(()).unwrap();
                worker_release_rx
                    .recv_timeout(Duration::from_secs(3))
                    .expect("second admitted effect must leave within the watchdog bound");
            })
        });
        worker_ready_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("second effect must be admitted before self-drop starts");
        let effect_result = handle.with_live_session_effect(|| {
            mark_probe_ready();
            let issuer = owner.lock().unwrap().take().unwrap();
            drop(issuer);
            assert!(!alive.load(Ordering::Acquire));
            let state = admission.state.lock().unwrap();
            assert_eq!(
                (state.closed, state.recovering, state.active),
                (true, false, 2)
            );
            assert_eq!(store_drops.load(Ordering::SeqCst), 0);
        });
        assert_eq!(effect_result, Ok(()));
        assert_eq!(store_drops.load(Ordering::SeqCst), 0);
        worker_release_tx
            .send(())
            .expect("second admitted effect remains connected until released");
        assert_eq!(worker.join().unwrap(), Ok(()));
        assert_eq!(store_drops.load(Ordering::SeqCst), 1);
        assert_eq!(
            handle.check_live(),
            Err(ResolutionError::Unavailable),
            "dropping the issuer must not let its existing handle become live again"
        );
        let later_effects = Arc::new(AtomicU64::new(0));
        let later_effects_in_callback = Arc::clone(&later_effects);
        assert_eq!(
            handle.with_live_session_effect(|| {
                later_effects_in_callback.fetch_add(1, Ordering::SeqCst);
            }),
            Err(ResolutionError::Unavailable)
        );
        assert_eq!(later_effects.load(Ordering::SeqCst), 0);
    }

    #[test]
    #[ignore = "spawned only by the bounded parent watchdog"]
    fn issuer_self_drop_probe() {
        issuer_self_drop_probe_child();
    }

    #[test]
    fn issuer_self_drop_closes_and_defers_store_in_watchdog_child() {
        match run_ignored_probe("issuer_self_drop_probe", Duration::from_secs(5)) {
            ProbeOutcome::AfterReadyExit(status) if status.success() => {}
            outcome => panic!("issuer self-drop did not finish safely: {outcome:?}"),
        }
    }

    fn issuer_into_parts_probe_child() {
        if std::env::var_os(PROBE_READY_ENV).is_none() {
            return;
        }
        let store_drops = Arc::new(AtomicU64::new(0));
        let (issuer, _caller, _session_id, handle) =
            drop_tracked_session("issuer-into-parts", Arc::clone(&store_drops), false);
        let alive = Arc::clone(&issuer.issuer.alive);
        let admission = Arc::clone(&issuer.issuer.admission);
        let owner = Arc::new(Mutex::new(Some(issuer)));
        let effect_result = handle.with_live_session_effect(|| {
            mark_probe_ready();
            let issuer = owner.lock().unwrap().take().unwrap();
            let issuer = match issuer.into_parts() {
                Ok(_) => panic!("same-thread into_parts extracted during an active effect"),
                Err(issuer) => issuer,
            };
            assert!(alive.load(Ordering::Acquire));
            assert_eq!(
                {
                    let state = admission.state.lock().unwrap();
                    (state.closed, state.recovering, state.active)
                },
                (false, false, 1),
                "same-thread into_parts must reject the active callback without closing it"
            );
            *owner.lock().unwrap() = Some(issuer);

            let child_owner = Arc::clone(&owner);
            let child_handle = Arc::clone(&handle);
            let child_alive = Arc::clone(&alive);
            let child_admission = Arc::clone(&admission);
            let child_store_drops = Arc::clone(&store_drops);
            let later_effects = Arc::new(AtomicU64::new(0));
            let child_later_effects = Arc::clone(&later_effects);
            thread::spawn(move || {
                let issuer = child_owner.lock().unwrap().take().unwrap();
                let issuer = match issuer.into_parts() {
                    Ok(_) => panic!("into_parts extracted a store while an effect was active"),
                    Err(issuer) => issuer,
                };
                assert!(child_alive.load(Ordering::Acquire));
                assert_eq!(
                    {
                        let state = child_admission.state.lock().unwrap();
                        (state.closed, state.recovering, state.active)
                    },
                    (false, false, 1),
                    "busy into_parts must return the unmodified issuer for retry"
                );
                drop(issuer);
                assert!(!child_alive.load(Ordering::Acquire));
                assert_eq!(
                    child_handle.check_live(),
                    Err(ResolutionError::Unavailable),
                    "dropping the refused issuer must revoke the child-thread view"
                );
                assert_eq!(
                    child_handle.with_live_session_effect(|| {
                        child_later_effects.fetch_add(1, Ordering::SeqCst);
                    }),
                    Err(ResolutionError::Unavailable)
                );
                assert_eq!(child_later_effects.load(Ordering::SeqCst), 0);
                assert_eq!(child_store_drops.load(Ordering::SeqCst), 0);
            })
            .join()
            .unwrap();
            assert_eq!(store_drops.load(Ordering::SeqCst), 0);
        });
        assert_eq!(effect_result, Ok(()));
        assert_eq!(store_drops.load(Ordering::SeqCst), 1);
        assert_eq!(handle.check_live(), Err(ResolutionError::Unavailable));
    }

    #[cfg(panic = "unwind")]
    fn issuer_callback_panic_with_panicking_store_probe_child() {
        if std::env::var_os(PROBE_READY_ENV).is_none() {
            return;
        }
        let store_drops = Arc::new(AtomicU64::new(0));
        let (issuer, _caller, _session_id, handle) =
            drop_tracked_session_with_panicking_panic_payload(
                "issuer-double-panic",
                Arc::clone(&store_drops),
            );
        let alive = Arc::clone(&issuer.issuer.alive);
        let owner = Arc::new(Mutex::new(Some(issuer)));
        mark_probe_ready();

        let result = catch_unwind(AssertUnwindSafe(|| {
            handle.with_live_session_effect(|| {
                drop(owner.lock().unwrap().take().unwrap());
                panic!("primary callback panic");
            })
        }));
        let primary_panic = result.expect_err("the injected callback panic must propagate");
        assert_eq!(
            *primary_panic.downcast_ref::<&str>().unwrap(),
            "primary callback panic",
            "the cleanup panic must not replace the original callback panic"
        );
        assert!(!alive.load(Ordering::Acquire));
        assert_eq!(store_drops.load(Ordering::SeqCst), 1);
        assert!(
            !C1LeaseGateScope::held_on_current_thread(),
            "the thread-local reentry marker must clear during unwind"
        );
        assert_eq!(handle.check_live(), Err(ResolutionError::Unavailable));
    }

    #[cfg(panic = "unwind")]
    #[test]
    #[ignore = "spawned only by the bounded parent watchdog"]
    fn issuer_callback_panic_with_panicking_store_probe() {
        issuer_callback_panic_with_panicking_store_probe_child();
    }

    #[cfg(panic = "unwind")]
    #[test]
    fn deferred_store_cleanup_panic_during_callback_unwind_does_not_abort() {
        match run_ignored_probe(
            "issuer_callback_panic_with_panicking_store_probe",
            Duration::from_secs(5),
        ) {
            ProbeOutcome::AfterReadyExit(status) if status.success() => {}
            outcome => panic!("double-panic cleanup was not bounded and caught: {outcome:?}"),
        }
    }

    #[cfg(panic = "unwind")]
    fn non_store_component_drop_panic_probe_child() {
        if std::env::var_os(PROBE_READY_ENV).is_none() {
            return;
        }

        let component_drops = Arc::new(AtomicU64::new(0));
        let store_drops = Arc::new(AtomicU64::new(0));
        let component = || NonStorePanicComponent {
            drops: Arc::clone(&component_drops),
            panic_on_drop: true,
        };
        let caller = TestCaller::new("non-store-panic-during-effect-unwind");
        let store = DropTrackedStore {
            inner: InMemorySnapshotStore::default(),
            drop_count: Arc::clone(&store_drops),
            panic_on_drop: false,
            panic_payload_drop: false,
        };
        let mut issuer = IdentityService::new(
            store,
            TestIds::default(),
            component(),
            component(),
            component(),
            component(),
        );
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Panic cleanup").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                Some(UnixMillis::new(1_000)),
                IdempotencyKey::new("non-store-panic-cleanup").unwrap(),
            )
            .unwrap();
        let context = issuer.interactive_session(&caller).unwrap();
        let handle = Arc::new(context.revalidation_handle().unwrap());
        drop(context);

        let alive = Arc::clone(&issuer.issuer.alive);
        let owner = Arc::new(Mutex::new(Some(issuer)));
        mark_probe_ready();
        let effect_result = catch_unwind(AssertUnwindSafe(|| {
            handle.with_live_session_effect(|| {
                let _drop_issuer_during_unwind = DropIssuerOnUnwind(Arc::clone(&owner));
                panic!("primary effect callback panic");
            })
        }));
        let panic = effect_result.expect_err("the primary callback panic must propagate");
        assert_eq!(
            *panic.downcast_ref::<&str>().unwrap(),
            "primary effect callback panic",
            "component cleanup panics must not replace the callback panic"
        );
        assert!(!alive.load(Ordering::Acquire));
        assert_eq!(component_drops.load(Ordering::SeqCst), 4);
        assert_eq!(store_drops.load(Ordering::SeqCst), 1);
        assert_eq!(handle.check_live(), Err(ResolutionError::Unavailable));
    }

    #[cfg(panic = "unwind")]
    #[test]
    #[ignore = "spawned only by the bounded parent watchdog"]
    fn non_store_component_drop_panic_probe() {
        non_store_component_drop_panic_probe_child();
    }

    #[cfg(panic = "unwind")]
    #[test]
    fn non_store_component_drop_panics_are_contained_in_watchdog_child() {
        match run_ignored_probe(
            "non_store_component_drop_panic_probe",
            Duration::from_secs(8),
        ) {
            ProbeOutcome::AfterReadyExit(status) if status.success() => {}
            outcome => panic!("non-store component double-panic cleanup failed: {outcome:?}"),
        }
    }

    #[test]
    #[ignore = "spawned only by the bounded parent watchdog"]
    fn issuer_into_parts_probe() {
        issuer_into_parts_probe_child();
    }

    #[test]
    fn issuer_into_parts_refuses_child_drop_and_defers_store_in_watchdog_child() {
        match run_ignored_probe("issuer_into_parts_probe", Duration::from_secs(5)) {
            ProbeOutcome::AfterReadyExit(status) if status.success() => {}
            outcome => panic!("busy issuer into_parts did not finish safely: {outcome:?}"),
        }
    }

    #[test]
    fn into_parts_succeeds_when_idle_and_invalidates_old_handle() {
        let store_drops = Arc::new(AtomicU64::new(0));
        let (issuer, _caller, _session_id, handle) =
            drop_tracked_session("issuer-into-parts-idle", Arc::clone(&store_drops), false);
        let (store, _ids, _clock, _authorizer, _cleaner, _events) = match issuer.into_parts() {
            Ok(parts) => parts,
            Err(_) => panic!("idle issuer should allow component extraction"),
        };
        assert_eq!(handle.check_live(), Err(ResolutionError::Unavailable));
        assert_eq!(store_drops.load(Ordering::SeqCst), 0);
        drop(store);
        assert_eq!(store_drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn into_parts_busy_issuer_can_retry_after_callback_and_transfer_store_once() {
        let store_drops = Arc::new(AtomicU64::new(0));
        let (issuer, _caller, _session_id, handle) =
            drop_tracked_session("issuer-into-parts-retry", Arc::clone(&store_drops), false);
        let alive = Arc::clone(&issuer.issuer.alive);
        let owner = Arc::new(Mutex::new(Some(issuer)));

        assert_eq!(
            handle.with_live_session_effect(|| {
                let issuer = owner.lock().unwrap().take().unwrap();
                let issuer = match issuer.into_parts() {
                    Ok(_) => panic!("into_parts extracted while its effect was active"),
                    Err(issuer) => issuer,
                };
                assert!(alive.load(Ordering::Acquire));
                assert_eq!(issuer.issuer.admission.state.lock().unwrap().active, 1);
                *owner.lock().unwrap() = Some(issuer);
            }),
            Ok(())
        );
        assert_eq!(store_drops.load(Ordering::SeqCst), 0);

        let issuer = owner.lock().unwrap().take().unwrap();
        let (store, _ids, _clock, _authorizer, _cleaner, _events) = match issuer.into_parts() {
            Ok(parts) => parts,
            Err(_) => panic!("the same issuer should extract after activity drains"),
        };
        assert!(!alive.load(Ordering::Acquire));
        assert_eq!(handle.check_live(), Err(ResolutionError::Unavailable));
        assert_eq!(store_drops.load(Ordering::SeqCst), 0);
        drop(store);
        assert_eq!(store_drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn deferred_store_remains_alive_until_recovery_guard_releases() {
        let store_drops = Arc::new(AtomicU64::new(0));
        let issuer = IssuerGuard::new();
        let alive = Arc::clone(&issuer.alive);
        let admission = Arc::clone(&issuer.admission);
        let store = DeferredSnapshotStore::new(
            DropTrackedStore {
                inner: InMemorySnapshotStore::default(),
                drop_count: Arc::clone(&store_drops),
                panic_on_drop: false,
                panic_payload_drop: false,
            },
            &issuer,
        );
        let recovery = admission.begin_recovery().unwrap();

        drop(issuer);
        drop(store);
        assert!(!alive.load(Ordering::Acquire));
        assert_eq!(store_drops.load(Ordering::SeqCst), 0);
        assert_eq!(
            {
                let state = admission.state.lock().unwrap();
                (state.closed, state.recovering, state.active)
            },
            (true, true, 0)
        );

        drop(recovery);
        assert_eq!(store_drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn poisoned_admission_refuses_parts_and_drop_still_closes_issuer() {
        let store_drops = Arc::new(AtomicU64::new(0));
        let (issuer, _caller, _session_id, handle) =
            drop_tracked_session("issuer-poisoned-drop", Arc::clone(&store_drops), false);
        let alive = Arc::clone(&issuer.issuer.alive);
        let admission = Arc::clone(&issuer.issuer.admission);
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _state = admission.state.lock().unwrap();
            panic!("poison issuer admission for fail-closed shutdown test");
        }));
        assert!(poisoned.is_err());

        let issuer = issuer
            .into_parts()
            .err()
            .expect("poisoned admission must refuse extraction");
        assert!(alive.load(Ordering::Acquire));
        assert_eq!(handle.check_live(), Err(ResolutionError::Unavailable));
        drop(issuer);
        assert!(!alive.load(Ordering::Acquire));
        assert_eq!(store_drops.load(Ordering::SeqCst), 1);
        assert_eq!(handle.check_live(), Err(ResolutionError::Unavailable));
    }

    #[cfg(panic = "unwind")]
    #[test]
    fn deferred_store_destructor_panic_keeps_issuer_closed() {
        match run_ignored_probe("issuer_deferred_store_panic_probe", Duration::from_secs(5)) {
            ProbeOutcome::AfterReadyExit(status) if status.success() => {}
            outcome => panic!("store destructor panic was not bounded and caught: {outcome:?}"),
        }
    }

    #[cfg(panic = "unwind")]
    fn deferred_store_destructor_panic_probe_child() {
        if std::env::var_os(PROBE_READY_ENV).is_none() {
            return;
        }
        let store_drops = Arc::new(AtomicU64::new(0));
        let (issuer, _caller, _session_id, handle) =
            drop_tracked_session("issuer-drop-panic", Arc::clone(&store_drops), true);
        let alive = Arc::clone(&issuer.issuer.alive);
        let owner = Arc::new(Mutex::new(Some(issuer)));
        mark_probe_ready();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            handle.with_live_session_effect(|| {
                drop(owner.lock().unwrap().take().unwrap());
            })
        }));

        assert!(
            result.is_err(),
            "the injected store destructor should panic"
        );
        assert!(!alive.load(Ordering::Acquire));
        assert_eq!(store_drops.load(Ordering::SeqCst), 1);
        assert!(
            !C1LeaseGateScope::held_on_current_thread(),
            "unwind must release the same-thread reentry marker"
        );
        assert_eq!(handle.check_live(), Err(ResolutionError::Unavailable));
    }

    #[cfg(panic = "unwind")]
    #[test]
    #[ignore = "spawned only by the bounded parent watchdog"]
    fn issuer_deferred_store_panic_probe() {
        deferred_store_destructor_panic_probe_child();
    }

    fn service(
        directory: &DurableTestDirectory,
        clock: TestClock,
    ) -> IdentityService<
        LinuxIdentitySnapshotStore,
        TestIds,
        TestClock,
        TestAuthorizer,
        TestCleaner,
        NoopIdentityEventSink,
    > {
        // A parallel test can fork just before the prior store is dropped.
        // Its close-on-exec descriptor may retain the flock until exec.
        // Lock-lifetime assertions below use the direct, non-retrying open.
        let deadline = Instant::now() + Duration::from_secs(2);
        let store = loop {
            match LinuxIdentitySnapshotStore::open(directory.open()) {
                Err(crate::store::DurableStoreError::Locked) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(1));
                }
                result => break result.unwrap(),
            }
        };
        IdentityService::new(
            store,
            TestIds::default(),
            clock,
            TestAuthorizer::default(),
            TestCleaner::default(),
            NoopIdentityEventSink,
        )
    }

    #[cfg(all(feature = "system-service-recovery-contract", target_arch = "x86_64"))]
    #[test]
    fn runtime_active_admission_persists_first_user_and_session() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("runtime-active-first-session");
        let mut identity = service(&directory, TestClock::at(100));
        let scope = TestRecoveryScope::new(17, &caller);
        let binding = RecoveryBinding::from_canonical_scope_digest(scope.binding_digest);
        let mut gate = CountingRecoveryGate::for_binding(binding);

        // The production C1 promotion path commits and reads back RuntimeActive.
        // This fixture gate only exercises C1's side of the boundary; it is not
        // an A/C3 owner proof or a product READY result.
        let handoff = identity
            .recover_with_new_intent_into_runtime(&caller, &mut gate)
            .unwrap();
        assert_eq!(gate.invocations, 1);
        assert_eq!(gate.effects, 1);
        assert_eq!(
            identity
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::RuntimeActive
        );
        assert!(identity.store.recovery_quarantined().unwrap());

        let admission = identity
            .enter_system_service_runtime_admission(&handoff, &scope)
            .unwrap();
        let (user_id, profile_id) = identity
            .create_local_user_with_runtime_admission(
                &admission,
                &caller,
                LocalUserMetadata::new("Runtime owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        let session_id = identity
            .login_local_with_runtime_admission(
                &admission,
                &caller,
                user_id,
                profile_id.clone(),
                None,
                IdempotencyKey::new("runtime-active-first-login").unwrap(),
            )
            .unwrap();
        {
            let context = identity
                .interactive_session_with_runtime_admission(&caller, &admission)
                .unwrap();
            let lease = context.revalidation_handle().unwrap();
            assert_eq!(lease.session_id(), &session_id);
            assert_eq!(lease.user_id(), Some(user_id));
            assert_eq!(lease.profile_id(), &profile_id);
            lease.check_live().unwrap();
        }
        assert!(identity.store.recovery_quarantined().unwrap());
        drop(admission);
        drop(identity);

        let reopened = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        assert!(reopened.recovery_operation_active().unwrap());
        let persisted = reopened.load_current().unwrap().unwrap();
        let snapshot = IdentitySnapshot::from_json(&persisted.bytes).unwrap();
        assert_eq!(snapshot.users.len(), 1);
        assert_eq!(snapshot.profiles.len(), 1);
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.users[0].user_id, user_id);
        assert_eq!(snapshot.profiles[0].profile_id, profile_id);
        assert_eq!(snapshot.profiles[0].owner_user_id, Some(user_id));
        assert_eq!(snapshot.sessions[0].user_id, Some(user_id));
        assert_eq!(snapshot.sessions[0].profile_id, profile_id);
        assert!(matches!(
            snapshot.sessions[0].lifecycle,
            SessionLifecycle::Active
        ));
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        feature = "linux-interactive-account",
        target_arch = "x86_64"
    ))]
    #[test]
    fn runtime_admission_persists_first_owner_and_initial_session() {
        runtime_admission_session_fixture(false);
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        feature = "linux-interactive-account",
        target_arch = "x86_64"
    ))]
    #[test]
    fn runtime_revocation_stays_revoked_after_logout_retry_and_reopen() {
        runtime_admission_session_fixture(true);
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        feature = "linux-interactive-account",
        target_arch = "x86_64"
    ))]
    fn runtime_admission_session_fixture(revoke: bool) {
        use std::os::unix::net::UnixStream;

        let directory = DurableTestDirectory::new();
        // This synthetic principal exists only inside the crate's test build;
        // the test exercises C1's mapping storage, not the C3 signer/owner path.
        let principal = C1VerifiedBootSystemServicePrincipal {
            service_id: "service:fixture.identity".to_owned(),
            principal_id: PrincipalId::new("service:fixture.identity").unwrap(),
            caller_context_id: CallerContextId::new("caller_fixture_identity").unwrap(),
            mapping_generation: std::num::NonZeroU64::new(3).unwrap(),
            catalog_generation: std::num::NonZeroU64::new(9).unwrap(),
            c3_launch_generation: std::num::NonZeroU64::new(17).unwrap(),
        };

        let store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let (mut identity, _events) =
            crate::c1_linux_boot_identity_service_from_store(store, &principal);
        let scope = TestRecoveryScope::new(17, &principal);
        let handoff = {
            let mut receipt_adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut identity,
                    &principal,
                );
            receipt_adapter
                .recover_with_new_intent_into_runtime(&scope, &mut ImmediateRuntimeRecoveryGate)
                .unwrap()
        };
        assert_eq!(
            identity
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::RuntimeActive,
            "the regression must use the production C1 runtime-promotion path"
        );
        assert!(identity.store.recovery_quarantined().unwrap());
        let admission = identity
            .enter_system_service_runtime_admission(&handoff, &scope)
            .unwrap();

        let binding = crate::provision_c1_linux_first_owner_binding_with_runtime_admission(
            &mut identity,
            &principal,
            &admission,
            b"fixture-owner",
        )
        .unwrap();
        let user_id = UserId::new(binding.user_id.get()).unwrap();
        let profile_id = ProfileId::new(binding.profile_id.clone()).unwrap();
        assert_eq!(
            binding.principal_id.as_str(),
            format!("user:{}", user_id.get())
        );
        assert!(binding.account_id.get() > 0);

        let (stream, _peer_stream) = UnixStream::pair().unwrap();
        let peer =
            nagi_peer_authenticated_connection::KernelAuthenticatedUnixPeer::authenticate(&stream)
                .unwrap();
        let mut pending =
            crate::linux_interactive_account::pending_initial_interactive_session_for_test(
                &identity,
                binding.account_id,
                user_id,
                profile_id.clone(),
                binding.principal_id.clone(),
                peer.connection_id(),
            );
        let idempotency_key = IdempotencyKey::new("runtime-initial-login").unwrap();
        let session_id = {
            let issued = pending
                .commit_after_c2_owner_postcheck_with_runtime_admission(
                    &mut identity,
                    &admission,
                    None,
                    idempotency_key,
                )
                .unwrap();
            assert_eq!(issued.caller().principal_id(), &binding.principal_id);
            let context = issued
                .checked_context_with_runtime_admission(&mut identity, &admission)
                .unwrap();
            let lease = context.revalidation_handle().unwrap();
            assert_eq!(lease.session_id(), issued.session_id());
            assert_eq!(lease.principal_id(), issued.caller().principal_id());
            assert_eq!(lease.caller_context_id(), issued.caller().context_id());
            lease.check_live().unwrap();
            let owned_identity = issued
                .checked_identity_with_runtime_admission(&mut identity, &admission)
                .unwrap();
            assert_eq!(owned_identity.session_id(), issued.session_id());
            assert_eq!(
                owned_identity.runtime_operation_identity(),
                Some(handoff.operation_identity())
            );
            owned_identity.check_live().unwrap();
            issued.session_id().clone()
        };
        let attached = pending.finish_after_c2_attachment().unwrap();
        assert_eq!(attached.session_id(), &session_id);
        assert!(identity.store.recovery_quarantined().unwrap());
        assert!(identity.snapshot().sessions.iter().any(|session| {
            session.session_id == session_id
                && matches!(session.lifecycle, SessionLifecycle::Active)
        }));

        if revoke {
            let lease = attached
                .checked_context_with_runtime_admission(&mut identity, &admission)
                .unwrap()
                .revalidation_handle()
                .unwrap();
            assert_eq!(
                attached.revoke_session_with_runtime_admission(&mut identity, &admission),
                Ok(EndSessionResult::Ended)
            );
            assert!(lease.check_live().is_err());
            assert_eq!(
                attached.end_session_with_runtime_admission(&mut identity, &admission),
                Ok(EndSessionResult::AlreadyEnded)
            );
        }
        drop(admission);
        drop(identity);
        let reopened_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        assert!(reopened_store.recovery_operation_active().unwrap());
        let persisted = reopened_store.load_current().unwrap().unwrap();
        let snapshot = IdentitySnapshot::from_json(&persisted.bytes).unwrap();
        assert_eq!(snapshot.users.len(), 1);
        assert_eq!(snapshot.profiles.len(), 1);
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.users[0].user_id, user_id);
        assert_eq!(snapshot.users[0].metadata.display_name, "fixture-owner");
        assert_eq!(snapshot.profiles[0].profile_id, profile_id);
        assert_eq!(snapshot.profiles[0].owner_user_id, Some(user_id));
        assert_eq!(snapshot.profiles[0].metadata.display_name, "fixture-owner");
        assert_eq!(snapshot.sessions[0].session_id, session_id);
        if revoke {
            assert!(matches!(
                snapshot.sessions[0].lifecycle,
                SessionLifecycle::Ended {
                    reason: SessionEndReason::Revoked,
                    ..
                }
            ));
        } else {
            assert!(matches!(
                snapshot.sessions[0].lifecycle,
                SessionLifecycle::Active
            ));
        }
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        feature = "linux-interactive-account",
        target_arch = "x86_64"
    ))]
    #[test]
    fn runtime_active_attach_failure_rolls_back_only_initial_session_before_reopen() {
        use std::os::unix::net::UnixStream;

        let directory = DurableTestDirectory::new();
        let principal = C1VerifiedBootSystemServicePrincipal {
            service_id: "service:fixture.identity".to_owned(),
            principal_id: PrincipalId::new("service:fixture.identity").unwrap(),
            caller_context_id: CallerContextId::new("caller_fixture_identity").unwrap(),
            mapping_generation: std::num::NonZeroU64::new(3).unwrap(),
            catalog_generation: std::num::NonZeroU64::new(9).unwrap(),
            c3_launch_generation: std::num::NonZeroU64::new(17).unwrap(),
        };
        let store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let (mut identity, _events) =
            crate::c1_linux_boot_identity_service_from_store(store, &principal);
        let scope = TestRecoveryScope::new(17, &principal);
        let handoff = {
            let mut receipt_adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut identity,
                    &principal,
                );
            receipt_adapter
                .recover_with_new_intent_into_runtime(&scope, &mut ImmediateRuntimeRecoveryGate)
                .unwrap()
        };
        assert_eq!(
            identity
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::RuntimeActive
        );
        let admission = identity
            .enter_system_service_runtime_admission(&handoff, &scope)
            .unwrap();
        let binding = crate::provision_c1_linux_first_owner_binding_with_runtime_admission(
            &mut identity,
            &principal,
            &admission,
            b"fixture-owner",
        )
        .unwrap();
        let user_id = UserId::new(binding.user_id.get()).unwrap();
        let profile_id = ProfileId::new(binding.profile_id.clone()).unwrap();
        let (stream, _peer_stream) = UnixStream::pair().unwrap();
        let peer =
            nagi_peer_authenticated_connection::KernelAuthenticatedUnixPeer::authenticate(&stream)
                .unwrap();
        let mut pending =
            crate::linux_interactive_account::pending_initial_interactive_session_for_test(
                &identity,
                binding.account_id,
                user_id,
                profile_id.clone(),
                binding.principal_id.clone(),
                peer.connection_id(),
            );
        let (session_id, stale_handle) = {
            let issued = pending
                .commit_after_c2_owner_postcheck_with_runtime_admission(
                    &mut identity,
                    &admission,
                    None,
                    IdempotencyKey::new("runtime-attach-failure").unwrap(),
                )
                .unwrap();
            let context = issued
                .checked_context_with_runtime_admission(&mut identity, &admission)
                .unwrap();
            let lease = context.revalidation_handle().unwrap();
            lease.check_live().unwrap();
            (issued.session_id().clone(), lease)
        };

        // The legacy abort reaches ordinary end_session(), which rejects the
        // still-active recovery journal and leaves the C1 transaction retryable.
        assert!(matches!(
            pending.abort_before_c2_attachment(&mut identity),
            Err(
                crate::linux_interactive_account::C1InitialInteractiveSessionError::Identity(
                    IdentityError::StoreUnavailable
                )
            )
        ));
        assert!(identity.snapshot().sessions.iter().any(|session| {
            session.session_id == session_id
                && matches!(session.lifecycle, SessionLifecycle::Active)
        }));

        pending
            .abort_before_c2_attachment_with_runtime_admission(&mut identity, &admission)
            .unwrap();
        let mut stale_effect_called = false;
        assert!(stale_handle
            .with_live_session_effect(|| stale_effect_called = true)
            .is_err());
        assert!(!stale_effect_called);
        assert!(pending.finish_after_c2_attachment().is_err());
        assert_eq!(identity.snapshot().users.len(), 1);
        assert_eq!(identity.snapshot().profiles.len(), 1);
        assert_eq!(identity.snapshot().profiles[0].profile_id, profile_id);
        assert_eq!(identity.snapshot().profiles[0].owner_user_id, Some(user_id));
        assert_eq!(identity.snapshot().sessions.len(), 1);
        assert!(matches!(
            identity.snapshot().sessions[0].lifecycle,
            SessionLifecycle::Ended {
                reason: SessionEndReason::UserLogout,
                ..
            }
        ));

        drop(admission);
        drop(identity);
        let reopened_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        assert!(reopened_store.recovery_operation_active().unwrap());
        let persisted = reopened_store.load_current().unwrap().unwrap();
        let snapshot = IdentitySnapshot::from_json(&persisted.bytes).unwrap();
        assert_eq!(snapshot.users.len(), 1);
        assert_eq!(snapshot.profiles.len(), 1);
        assert_eq!(snapshot.sessions.len(), 1);
        assert_eq!(snapshot.users[0].user_id, user_id);
        assert_eq!(snapshot.profiles[0].profile_id, profile_id);
        assert_eq!(snapshot.profiles[0].owner_user_id, Some(user_id));
        assert_eq!(snapshot.sessions[0].session_id, session_id);
        assert!(matches!(
            snapshot.sessions[0].lifecycle,
            SessionLifecycle::Ended {
                reason: SessionEndReason::UserLogout,
                ..
            }
        ));
        assert!(!snapshot
            .sessions
            .iter()
            .any(|session| matches!(session.lifecycle, SessionLifecycle::Active)));
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        feature = "linux-interactive-account",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    #[test]
    fn normal_runtime_terminal_receipt_allows_new_intent_after_same_store_reopen() {
        use nagi_system_service_recovery_contract::C1RecoveryReceiptClient;

        // Exercise C1's real RuntimeActive promotion, durable receipt, and
        // same-store reopen path. The C3 gate remains an explicit C1 test
        // fixture; this is not evidence of a production C3 owner or READY.
        let directory = DurableTestDirectory::new();
        let make_principal = |c3_launch_generation| C1VerifiedBootSystemServicePrincipal {
            service_id: "service:fixture.identity".to_owned(),
            principal_id: PrincipalId::new("service:fixture.identity").unwrap(),
            caller_context_id: CallerContextId::new("caller_fixture_identity").unwrap(),
            mapping_generation: std::num::NonZeroU64::new(3).unwrap(),
            catalog_generation: std::num::NonZeroU64::new(9).unwrap(),
            c3_launch_generation: std::num::NonZeroU64::new(c3_launch_generation).unwrap(),
        };

        let old_principal = make_principal(17);
        let old_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let (mut old_identity, _old_events) =
            crate::c1_linux_boot_identity_service_from_store(old_store, &old_principal);
        let old_scope = TestRecoveryScope::new(17, &old_principal);
        let old_handoff = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut old_identity,
                    &old_principal,
                );
            adapter
                .recover_with_new_intent_into_runtime(&old_scope, &mut ImmediateRuntimeRecoveryGate)
                .unwrap()
        };

        let old_operation = old_handoff.operation_identity();
        let old_binding = RecoveryBinding::from_canonical_scope_digest(old_scope.binding_digest);
        let old_record = old_identity
            .store
            .recovery_journal_record()
            .unwrap()
            .expect("runtime promotion must retain its durable operation");
        assert_eq!(
            old_record.operation_id.get(),
            old_operation.operation_id().get()
        );
        assert_eq!(old_record.binding, old_binding);
        assert_eq!(
            old_record.phase,
            RecoveryJournalPhase::RuntimeActive,
            "capture the previous boot's durable phase before terminal receipt handling"
        );
        assert!(old_identity.store.recovery_quarantined().unwrap());

        let old_scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &old_scope;
        let mut c3_gate = CountingC3ReconciliationGate::new(old_scope_ref);
        let terminal_receipt = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut old_identity,
                    &old_principal,
                );
            adapter
                .reconcile_terminal(old_scope_ref, &mut c3_gate)
                .expect("normal C3 retirement must permit exact C1 terminalization")
        };
        assert_eq!(c3_gate.invocations, 1);
        assert_eq!(c3_gate.callbacks, 1);
        let observed_claim = c3_gate
            .observed_claim
            .expect("C3 must receive the old durable C1 operation correlation");
        assert_eq!(
            observed_claim.operation_id().get(),
            old_operation.operation_id().get()
        );
        assert_eq!(observed_claim.binding_digest(), &old_scope.binding_digest);

        let terminal_record = old_identity
            .store
            .recovery_journal_record()
            .unwrap()
            .expect("C1 terminal row must remain durable");
        assert_eq!(
            terminal_record.operation_id.get(),
            old_operation.operation_id().get()
        );
        assert_eq!(terminal_record.binding, old_binding);
        assert_eq!(
            terminal_record.phase,
            RecoveryJournalPhase::Reconciled {
                outcome_digest: terminal_receipt.outcome_digest()
            }
        );
        {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut old_identity,
                    &old_principal,
                );
            adapter
                .consume_terminal_receipt(old_scope_ref, terminal_receipt)
                .expect("C1 must consume only the exact durable terminal receipt");
        }
        drop(old_handoff);
        drop(old_identity);

        // Reopen the same test Data directory and inspect the durable row
        // before starting recovery, so the new operation cannot hide or
        // overwrite the previous RuntimeActive phase in this regression.
        let reopened_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let reopened_record = reopened_store
            .recovery_journal_record()
            .unwrap()
            .expect("the same Data directory must retain the terminal row");
        assert_eq!(reopened_record, terminal_record);
        assert!(!reopened_store.recovery_quarantined().unwrap());

        let new_principal = make_principal(18);
        let (mut next_identity, _next_events) =
            crate::c1_linux_boot_identity_service_from_store(reopened_store, &new_principal);
        let new_scope = TestRecoveryScope::new(18, &new_principal);
        let next_handoff = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut next_identity,
                    &new_principal,
                );
            adapter
                .recover_with_new_intent_into_runtime(&new_scope, &mut ImmediateRuntimeRecoveryGate)
                .expect("the exact terminal row must allow a new normal boot operation")
        };
        let next_record = next_identity
            .store
            .recovery_journal_record()
            .unwrap()
            .expect("the new runtime operation must be durable");
        assert!(next_record.operation_id.get() > old_operation.operation_id().get());
        assert_eq!(
            next_handoff.operation_identity().operation_id().get(),
            next_record.operation_id.get()
        );
        assert_eq!(
            next_record.binding,
            RecoveryBinding::from_canonical_scope_digest(new_scope.binding_digest)
        );
        assert_eq!(next_record.phase, RecoveryJournalPhase::RuntimeActive);
    }

    #[cfg(feature = "c2-accepted-peer")]
    #[test]
    fn c2_owner_pid1_boundary_and_c1_exact_session_effect() {
        use std::num::NonZeroU64;
        use std::os::unix::net::{UnixListener, UnixStream};
        use std::path::PathBuf;

        static NEXT_SOCKET: AtomicU64 = AtomicU64::new(1);

        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("c1-c2-exact-session-generation");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("C2 exact session binding").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        let session_id = issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("c2-exact-session-binding").unwrap(),
            )
            .unwrap();
        let context = issuer.interactive_session(&caller).unwrap();
        let lease = context.revalidation_handle().unwrap();
        let generation = lease.generation();
        let pending_binding = context.stage_session_effect_binding().unwrap();
        let admission_gate = pending_binding.admission_gate();
        let (binding_gate, activate_binding, abort_binding) = pending_binding.into_parts();
        drop(context);

        // This leaf registration is only a proof fixture. C1 deliberately
        // does not enroll its owner key or produce a caller seal here. The
        // dedicated QEMU runner executes this test ELF as a child of PID 1,
        // so preserve and exercise the expected non-PID1 rejection instead
        // of treating the fixture as a trusted C2 owner.
        let socket_path: PathBuf = std::env::temp_dir().join(format!(
            "nagi-c1-peer-session-{}-{}.sock",
            std::process::id(),
            NEXT_SOCKET.fetch_add(1, Ordering::SeqCst),
        ));
        let listener = UnixListener::bind(&socket_path).unwrap();
        let owner_result =
            nagi_peer_authenticated_connection::C2AcceptedPeerListenerOwner::register_from_c2_owner(
                listener,
                NonZeroU64::new(101).unwrap(),
                NonZeroU64::new(102).unwrap(),
                NonZeroU64::new(103).unwrap(),
                Instant::now() + Duration::from_secs(30),
            );
        let owner = match owner_result {
            Ok(owner) => owner,
            Err(nagi_peer_authenticated_connection::PeerAuthenticationError::OwnerUnavailable)
                if std::process::id() != 1 =>
            {
                // The checked-in QEMU harness owns PID 1 and launches the
                // Rust test ELF as a child. This branch is the expected
                // non-PID1 denial; the exact-selector positive path below is
                // reachable only in a test environment that runs this test
                // as the canonical PID1 owner.
                std::fs::remove_file(&socket_path).unwrap();
                return;
            }
            Err(error) => panic!("unexpected C2 listener registration result: {error:?}"),
        };
        let client = UnixStream::connect(&socket_path).unwrap();
        let accepted = owner
            .accept_until(Instant::now() + Duration::from_secs(5))
            .unwrap();
        let (stream, peer_handle) = accepted.into_stream_and_handle();
        let app_id = 0x101;
        let app_session_id = 0x202;
        let registration_id = NonZeroU64::new(303).unwrap();
        let mut saved_selector = None;
        let selector_probe = peer_handle.with_live_interactive_peer(
            &stream,
            registration_id,
            app_id,
            app_session_id,
            generation,
            |peer| -> Result<(), ()> {
                saved_selector = Some(peer.selector().clone());
                Ok(())
            },
        );
        assert!(matches!(selector_probe, Ok(Ok(()))));
        let selector = saved_selector.as_ref().unwrap();

        // C2's final read-only admission runs through the distinct pending
        // gate while C1 holds the exact Session lease. Activation occurs only
        // at the end of that admission callback; protected work uses the
        // separate ACTIVE effect gate below.
        let mut admission_checks_ran = false;
        assert_eq!(
            admission_gate.with_live_c2_peer_session_admission(
                selector,
                app_id,
                app_session_id,
                || {
                    assert!(selector.matches_session(app_id, app_session_id, generation));
                    admission_checks_ran = true;
                    activate_binding.activate().unwrap();
                },
            ),
            Ok(())
        );
        assert!(admission_checks_ran);

        // Protected work takes the C1 Session gate first and invokes C2's
        // live accepted-connection callback only after the binding is ACTIVE.
        let mut exact_effect_ran = false;
        let exact_result =
            binding_gate.with_live_c2_peer_session_effect(selector, app_id, app_session_id, || {
                peer_handle.with_live_interactive_peer(
                    &stream,
                    registration_id,
                    app_id,
                    app_session_id,
                    generation,
                    |peer| -> Result<(), ()> {
                        assert!(peer.selector().matches_session(
                            app_id,
                            app_session_id,
                            generation
                        ));
                        exact_effect_ran = true;
                        Ok(())
                    },
                )
            });
        assert!(matches!(exact_result, Ok(Ok(Ok(())))));
        assert!(exact_effect_ran);

        let mut wrong_app_effect_ran = false;
        assert_eq!(
            binding_gate.with_live_c2_peer_session_effect(
                selector,
                app_id + 1,
                app_session_id,
                || wrong_app_effect_ran = true,
            ),
            Err(ResolutionError::Unavailable)
        );
        assert!(!wrong_app_effect_ran);

        let mut wrong_session_effect_ran = false;
        assert_eq!(
            binding_gate.with_live_c2_peer_session_effect(
                selector,
                app_id,
                app_session_id + 1,
                || wrong_session_effect_ran = true,
            ),
            Err(ResolutionError::Unavailable)
        );
        assert!(!wrong_session_effect_ran);

        let other_generation = NonZeroU64::new(generation.get().checked_add(1).unwrap()).unwrap();
        let mut wrong_generation_selector = None;
        let mismatch_result = peer_handle.with_live_interactive_peer(
            &stream,
            registration_id,
            app_id,
            app_session_id,
            other_generation,
            |peer| -> Result<(), ()> {
                wrong_generation_selector = Some(peer.selector().clone());
                Ok(())
            },
        );
        assert!(matches!(mismatch_result, Ok(Ok(()))));
        let mut wrong_generation_effect_ran = false;
        assert_eq!(
            binding_gate.with_live_c2_peer_session_effect(
                wrong_generation_selector.as_ref().unwrap(),
                app_id,
                app_session_id,
                || wrong_generation_effect_ran = true,
            ),
            Err(ResolutionError::Unavailable)
        );
        assert!(!wrong_generation_effect_ran);

        issuer.end_session(&caller, &session_id).unwrap();
        let mut stale_effect_ran = false;
        assert_eq!(
            binding_gate.with_live_c2_peer_session_effect(selector, app_id, app_session_id, || {
                stale_effect_ran = true
            },),
            Err(ResolutionError::Ended)
        );
        assert!(!stale_effect_ran);
        abort_binding.abort();

        owner.revoke().unwrap();
        drop(client);
        std::fs::remove_file(socket_path).unwrap();
    }

    /// The C1 account/peer receiver is intentionally exercised only from the
    /// nested PID-1 guest harness: the C2 leaf refuses listener registration
    /// from an ordinary test child. This validates the lower account and
    /// transport binding only; it does not establish the C2 root owner proof
    /// or issue a C1 caller/Session.
    #[cfg(feature = "linux-interactive-account")]
    #[test]
    #[ignore = "run only by the dedicated nested PID-1 Linux guest"]
    fn c1_account_peer_binding_consumes_same_verifier_stream_and_profile() {
        use libnagi::credential::{AccountRecord, Credential, MIN_ITERATIONS, SALT_BYTES};
        use std::fs::{self, File, OpenOptions};
        use std::io::Write;
        use std::num::NonZeroU64;
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        use std::os::unix::net::{UnixListener, UnixStream};
        use std::path::PathBuf;

        static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("account-peer-binding");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("accepted account").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".to_owned())).unwrap(),
            )
            .unwrap();

        let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let account_directory = std::env::temp_dir().join(format!(
            "nagi-c1-account-peer-{}-{sequence}",
            std::process::id()
        ));
        let mut directory_builder = fs::DirBuilder::new();
        directory_builder
            .mode(0o700)
            .create(&account_directory)
            .unwrap();
        let account_name = b"owner";
        let password = b"accepted-account-password";
        let credential = Credential::derive(password, [0x81; SALT_BYTES], MIN_ITERATIONS).unwrap();
        let account_record = AccountRecord::new(account_name, credential).unwrap();
        let persisted = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "account_id": 771,
            "user_id": user_id.get(),
            "profile_id": profile_id.as_str(),
            "principal_id": "user:accepted-account",
            "account_record": account_record.encode().to_vec(),
        }))
        .unwrap();
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(account_directory.join("owner-account-identity.v1"))
            .unwrap()
            .write_all(&persisted)
            .unwrap();
        let verifier = nagi_linux_account_verifier::LinuxAccountVerifier::from_protected_directory(
            File::open(&account_directory).unwrap(),
        )
        .unwrap();

        let socket_path: PathBuf = account_directory.join("accepted-peer.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        let owner = nagi_peer_authenticated_connection::C2AcceptedPeerListenerOwner::register_from_c2_owner(
            listener,
            NonZeroU64::new(101).unwrap(),
            NonZeroU64::new(102).unwrap(),
            NonZeroU64::new(103).unwrap(),
            Instant::now() + Duration::from_secs(30),
        )
        .unwrap();
        let client = UnixStream::connect(&socket_path).unwrap();
        let accepted = owner
            .accept_until(Instant::now() + Duration::from_secs(5))
            .unwrap();
        let (stream, peer_handle) = accepted.into_stream_and_handle();
        let registration_id = NonZeroU64::new(303).unwrap();
        let app_id = 0x101;
        let app_session_id = 0x202;
        let generation = NonZeroU64::new(404).unwrap();

        let result = peer_handle.with_live_interactive_peer(
            &stream,
            registration_id,
            app_id,
            app_session_id,
            generation,
            |peer| -> Result<(), ()> {
                let evidence = verifier
                    .authenticate(&stream, account_name, password)
                    .unwrap();
                let binding = crate::authenticate_account_for_accepted_peer(
                    &issuer, &verifier, evidence, &peer, &stream,
                )
                .unwrap();
                assert_eq!(binding.account_id().get(), 771);
                assert_eq!(binding.user_id(), user_id);
                assert_eq!(binding.profile_id(), &profile_id);
                assert_eq!(binding.principal_id().as_str(), "user:accepted-account");
                assert!(binding.matches_peer(&peer));
                Ok(())
            },
        );
        assert!(matches!(result, Ok(Ok(()))));

        owner.revoke().unwrap();
        drop(peer_handle);
        drop(stream);
        drop(client);
        drop(owner);
        fs::remove_dir_all(account_directory).unwrap();
    }

    /// Single-writer test wrapper whose reader tracks every successful current
    /// snapshot commit. Recovery-journal writes do not change that snapshot;
    /// the reader intentionally does not inspect the journal so this test
    /// isolates C1 admission's own post-claim fence.
    struct JournalIndependentSnapshotReaderStore {
        inner: LinuxIdentitySnapshotStore,
        current: Arc<RwLock<Option<StoredSnapshot>>>,
        begin_intent_fault: Option<DurableStoreFault>,
        begin_intent_busy_once: bool,
        claim_fault: Option<DurableStoreFault>,
        panic_after_claim: bool,
        panic_on_mark_unknown: bool,
        post_intent_busy: AtomicU64,
    }

    impl IdentitySnapshotStore for JournalIndependentSnapshotReaderStore {
        fn current_snapshot_reader(&self) -> Option<CurrentSnapshotReader> {
            let current = Arc::clone(&self.current);
            Some(Arc::new(move || {
                current
                    .read()
                    .map(|snapshot| snapshot.clone())
                    .map_err(|_| SnapshotStoreError::Unavailable)
            }))
        }

        fn load_current_state(&self) -> Result<SnapshotStoreState, SnapshotStoreError> {
            self.inner.load_current_state()
        }

        fn recovery_quarantined(&self) -> Result<bool, SnapshotStoreError> {
            self.inner.recovery_quarantined()
        }

        fn recovery_operation_active(&self) -> Result<bool, SnapshotStoreError> {
            self.inner.recovery_operation_active()
        }

        fn load_current(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
            self.inner.load_current()
        }

        fn load_last_known_good(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
            self.inner.load_last_known_good()
        }

        fn commit_atomic(
            &mut self,
            expected_version: Option<SnapshotVersion>,
            complete_snapshot: &[u8],
        ) -> Result<SnapshotVersion, SnapshotStoreError> {
            let version = self
                .inner
                .commit_atomic(expected_version, complete_snapshot)?;
            let current = self.inner.load_current()?;
            *self
                .current
                .write()
                .map_err(|_| SnapshotStoreError::Unavailable)? = current;
            Ok(version)
        }
    }

    impl RecoveryJournalStore for JournalIndependentSnapshotReaderStore {
        type ReconciliationPermit = RecoveryReconciliationPermit;

        fn recovery_journal_record(
            &self,
        ) -> Result<Option<crate::store::RecoveryJournalRecord>, RecoveryJournalError> {
            if self
                .post_intent_busy
                .compare_exchange(1, 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Err(RecoveryJournalError::Busy);
            }
            self.inner.recovery_journal_record()
        }

        fn begin_recovery_intent(
            &mut self,
            binding: RecoveryBinding,
        ) -> Result<RecoveryOperationPermit, RecoveryJournalError> {
            if std::mem::take(&mut self.begin_intent_busy_once) {
                return Err(RecoveryJournalError::Busy);
            }
            if let Some(fault) = self.begin_intent_fault.take() {
                self.inner.set_fault_for_test(fault);
            }
            let permit = self.inner.begin_recovery_intent(binding)?;
            if let Some(fault) = self.claim_fault.take() {
                self.inner.set_fault_for_test(fault);
            }
            Ok(permit)
        }

        fn claim_recovery_effect(
            &mut self,
            permit: &RecoveryOperationPermit,
        ) -> Result<RecoveryEffectClaim, RecoveryJournalError> {
            if self
                .post_intent_busy
                .compare_exchange(2, 0, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Err(RecoveryJournalError::Busy);
            }
            let claim = self.inner.claim_recovery_effect(permit)?;
            if self.panic_after_claim {
                self.panic_after_claim = false;
                panic!("injected panic after durable recovery claim");
            }
            Ok(claim)
        }

        fn commit_recovery_runtime_active(
            &mut self,
            claim: &RecoveryEffectClaim,
        ) -> Result<(), RecoveryJournalError> {
            self.inner.commit_recovery_runtime_active(claim)
        }

        fn mark_recovery_outcome_unknown(
            &mut self,
            operation_id: RecoveryOperationId,
            binding: &RecoveryBinding,
        ) -> Result<(), RecoveryJournalError> {
            if self.panic_on_mark_unknown {
                self.panic_on_mark_unknown = false;
                std::panic::panic_any(PanicPayloadThatPanicsWhenDropped);
            }
            self.inner
                .mark_recovery_outcome_unknown(operation_id, binding)
        }

        fn prepare_recovery_reconciliation(
            &mut self,
            reconciliation: &RecoveryReconciliation,
        ) -> Result<Self::ReconciliationPermit, RecoveryJournalError> {
            self.inner.prepare_recovery_reconciliation(reconciliation)
        }

        fn reconcile_recovery(
            &mut self,
            admission: &IssuerRecoveryAdmissionGuard,
            permit: Self::ReconciliationPermit,
            reconciliation: RecoveryReconciliation,
        ) -> Result<(), RecoveryJournalError> {
            self.inner
                .reconcile_recovery(admission, permit, reconciliation)
        }
    }

    #[test]
    fn denied_recovery_cannot_create_or_claim_durable_intent() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("recovery-denied-before-intent");
        let mut issuer = IdentityService::new(
            LinuxIdentitySnapshotStore::open(directory.open()).unwrap(),
            TestIds::default(),
            TestClock::at(100),
            DenyRecoveryAuthorizer,
            TestCleaner::default(),
            NoopIdentityEventSink,
        );
        let binding = RecoveryBinding::for_test([0x80; 32]);
        let before = issuer.store.load_current_state().unwrap();
        let mut gate = CountingRecoveryGate::for_binding(binding.clone());

        assert_eq!(
            issuer.recover_with_new_intent(&caller, &mut gate),
            Err(IdentityError::AuthorizationDenied)
        );
        assert_eq!(gate.invocations, 0);
        assert_eq!(gate.effects, 0);
        assert!(issuer.store.recovery_journal_record().unwrap().is_none());
        assert_eq!(issuer.store.load_current_state().unwrap(), before);

        let permit = issuer.store.begin_recovery_intent(binding.clone()).unwrap();
        let mut permit_slot = Some(permit);
        let before_claim = issuer.store.recovery_journal_record().unwrap();
        let before_claim_state = issuer.store.load_current_state().unwrap();
        let before_claim_lkg = issuer.store.load_last_known_good().unwrap();
        let mut resume_gate = CountingRecoveryGate::for_binding(binding);
        assert_eq!(
            issuer.recover_with_operation(&caller, &mut permit_slot, &mut resume_gate),
            Err(IdentityError::AuthorizationDenied)
        );
        assert_eq!(resume_gate.invocations, 0);
        assert_eq!(resume_gate.effects, 0);
        assert_eq!(
            issuer.store.recovery_journal_record().unwrap(),
            before_claim,
            "denial must leave an existing exact intent unclaimed"
        );
        assert_eq!(
            issuer.store.load_current_state().unwrap(),
            before_claim_state
        );
        assert_eq!(
            issuer.store.load_last_known_good().unwrap(),
            before_claim_lkg
        );
        assert!(issuer.store.recovery_quarantined().unwrap());
        assert!(
            permit_slot.is_some(),
            "pre-effect denial must preserve the permit for retry"
        );
    }

    #[cfg(feature = "system-service-recovery-contract")]
    #[test]
    fn scoped_operation_rejects_changed_scope_before_store_or_gate() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("scoped-recovery-operation-preflight");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let scope = TestRecoveryScope::new(91, &caller);
        let binding = RecoveryBinding::for_test(scope.binding_digest);
        let permit = issuer.store.begin_recovery_intent(binding.clone()).unwrap();
        let mut permit_slot = Some(permit);
        let before_journal = issuer.store.recovery_journal_record().unwrap();
        let before_store_state = issuer.store.load_current_state().unwrap();
        let before_lkg = issuer.store.load_last_known_good().unwrap();
        let before_snapshot = issuer.snapshot.clone();

        let assert_unchanged = |issuer: &DurableTestService| {
            assert_eq!(
                issuer.store.recovery_journal_record().unwrap(),
                before_journal
            );
            assert_eq!(
                issuer.store.load_current_state().unwrap(),
                before_store_state
            );
            assert_eq!(issuer.store.load_last_known_good().unwrap(), before_lkg);
            assert_eq!(issuer.snapshot, before_snapshot);
            assert_eq!(
                issuer.store.reconciliation_fence_state_for_test(),
                (false, false, 0)
            );
            assert_eq!(
                {
                    let state = issuer.issuer.admission.state.lock().unwrap();
                    (state.closed, state.recovering, state.active)
                },
                (false, false, 0)
            );
            assert!(issuer.issuer.alive.load(Ordering::Acquire));
        };

        let mut stale_generation = scope.clone();
        stale_generation.change_generation_without_rehash(92);
        let mut stale_gate = CountingRecoveryGate::for_binding(binding.clone());
        assert_eq!(
            issuer.recover_with_scoped_operation(
                &caller,
                &stale_generation,
                &mut permit_slot,
                &mut stale_gate,
            ),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(stale_gate.invocations, 0);
        assert_eq!(stale_gate.effects, 0);
        assert!(permit_slot.is_some());
        assert_unchanged(&issuer);

        let mut changed_generation = scope.clone();
        changed_generation.change_generation_and_rehash(92);
        let mut changed_scope_gate = CountingRecoveryGate::for_binding(binding.clone());
        assert_eq!(
            issuer.recover_with_scoped_operation(
                &caller,
                &changed_generation,
                &mut permit_slot,
                &mut changed_scope_gate,
            ),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(changed_scope_gate.invocations, 0);
        assert_eq!(changed_scope_gate.effects, 0);
        assert!(permit_slot.is_some());
        assert_unchanged(&issuer);

        let mut wrong_gate =
            CountingRecoveryGate::for_binding(RecoveryBinding::for_test([0x93; 32]));
        assert_eq!(
            issuer.recover_with_scoped_operation(
                &caller,
                &scope,
                &mut permit_slot,
                &mut wrong_gate,
            ),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(wrong_gate.invocations, 0);
        assert_eq!(wrong_gate.effects, 0);
        assert!(permit_slot.is_some());
        assert_unchanged(&issuer);

        let other_caller = TestCaller::new("scoped-recovery-different-caller");
        let mut wrong_caller_gate = CountingRecoveryGate::for_binding(binding);
        assert_eq!(
            issuer.recover_with_scoped_operation(
                &other_caller,
                &scope,
                &mut permit_slot,
                &mut wrong_caller_gate,
            ),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(wrong_caller_gate.invocations, 0);
        assert_eq!(wrong_caller_gate.effects, 0);
        assert!(permit_slot.is_some());
        assert_unchanged(&issuer);
    }

    #[cfg(feature = "system-service-recovery-contract")]
    #[test]
    fn receipt_adapter_rejects_caller_and_service_mapping_mismatches_before_gate() {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("service-mapping-intake");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let scope = TestRecoveryScope::new(141, &caller);
        let binding = RecoveryBinding::for_test(scope.binding_digest);
        drop(issuer.store.begin_recovery_intent(binding).unwrap());

        let before_journal = issuer.store.recovery_journal_record().unwrap();
        let before_current = issuer.store.load_current_state().unwrap();
        let before_last_known_good = issuer.store.load_last_known_good().unwrap();
        let before_snapshot = issuer.snapshot.clone();

        let mut wrong_principal = scope.clone();
        wrong_principal.change_principal_id_and_rehash("service:other-principal");
        let mut wrong_context = scope.clone();
        wrong_context.change_caller_context_id_and_rehash("caller_other_context");
        let mut wrong_service_slot = scope.clone();
        wrong_service_slot.change_service_id_and_rehash("service:other-slot");
        let mut wrong_mapping_generation = scope.clone();
        wrong_mapping_generation.change_mapping_generation_and_rehash(4);
        let mut wrong_role = scope.clone();
        wrong_role.change_role_and_rehash("third_party_app");
        let mut wrong_catalog_generation = scope.clone();
        wrong_catalog_generation.change_catalog_generation_and_rehash(10);
        let mut wrong_launch_generation = scope.clone();
        wrong_launch_generation.change_generation_and_rehash(142);

        let candidates: [(&str, &TestRecoveryScope); 7] = [
            ("principal", &wrong_principal),
            ("caller context", &wrong_context),
            ("service slot", &wrong_service_slot),
            ("mapping generation", &wrong_mapping_generation),
            ("principal role", &wrong_role),
            ("catalog generation", &wrong_catalog_generation),
            ("C3 launch generation", &wrong_launch_generation),
        ];

        for (field, candidate) in candidates {
            let scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
                candidate;
            let mut gate = CountingC3ReconciliationGate::new(scope_ref);
            let result = {
                let mut adapter = C1SystemServiceRecoveryReceiptAdapter::new(&mut issuer, &caller);
                adapter.reconcile_terminal(scope_ref, &mut gate)
            };

            assert_eq!(result, Err(RecoveryClientError::Rejected), "{field}");
            assert_eq!(gate.invocations, 0, "{field} must not enter the C3 gate");
            assert_eq!(gate.callbacks, 0, "{field} must not run an effect callback");
            assert_eq!(
                issuer.store.recovery_journal_record().unwrap(),
                before_journal,
                "{field} mismatch must preserve the durable C1 operation"
            );
            assert_eq!(issuer.store.load_current_state().unwrap(), before_current);
            assert_eq!(
                issuer.store.load_last_known_good().unwrap(),
                before_last_known_good
            );
            assert_eq!(issuer.snapshot, before_snapshot);
            assert_eq!(
                issuer.store.reconciliation_fence_state_for_test(),
                (false, false, 0)
            );
            assert_eq!(
                {
                    let state = issuer.issuer.admission.state.lock().unwrap();
                    (state.closed, state.recovering, state.active)
                },
                (false, false, 0)
            );
            assert!(issuer.issuer.alive.load(Ordering::Acquire));
        }
    }

    #[cfg(feature = "system-service-recovery-contract")]
    #[test]
    fn receipt_adapter_snapshots_stateful_scope_once_before_gate_or_store_mutation() {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("stateful-recovery-scope");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let scope = StatefulContextRecoveryScope::mismatching_second_context(121, &caller);
        let binding = RecoveryBinding::for_test(scope.base.binding_digest);
        drop(issuer.store.begin_recovery_intent(binding).unwrap());

        let before_journal = issuer.store.recovery_journal_record().unwrap();
        let before_state = issuer.store.load_current_state().unwrap();
        let before_last_known_good = issuer.store.load_last_known_good().unwrap();
        let before_snapshot = issuer.snapshot.clone();
        let scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &scope;
        let mut gate = CountingC3ReconciliationGate::new(scope_ref);

        let result = {
            let mut adapter = C1SystemServiceRecoveryReceiptAdapter::new(&mut issuer, &caller);
            adapter.reconcile_terminal(scope_ref, &mut gate)
        };

        assert_eq!(result, Err(RecoveryClientError::Rejected));
        assert_eq!(scope.context_calls.get(), 1);
        assert_eq!(gate.invocations, 0);
        assert_eq!(gate.callbacks, 0);
        assert_eq!(
            issuer.store.recovery_journal_record().unwrap(),
            before_journal
        );
        assert_eq!(issuer.store.load_current_state().unwrap(), before_state);
        assert_eq!(
            issuer.store.load_last_known_good().unwrap(),
            before_last_known_good
        );
        assert_eq!(issuer.snapshot, before_snapshot);
        assert_eq!(
            issuer.store.reconciliation_fence_state_for_test(),
            (false, false, 0)
        );
        assert_eq!(
            {
                let state = issuer.issuer.admission.state.lock().unwrap();
                (state.closed, state.recovering, state.active)
            },
            (false, false, 0)
        );
        assert!(issuer.issuer.alive.load(Ordering::Acquire));
    }

    #[cfg(feature = "system-service-recovery-contract")]
    #[test]
    fn receipt_adapter_rejects_canonical_scope_change_at_c3_callback_before_terminal_write() {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("canonical-scope-change-at-gate");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let scope = CanonicalSecondViewRecoveryScope::new(130, &caller);
        let first_binding = RecoveryBinding::for_test(scope.first.binding_digest);
        drop(issuer.store.begin_recovery_intent(first_binding).unwrap());

        let before_journal = issuer.store.recovery_journal_record().unwrap();
        let before_current = issuer.store.load_current_state().unwrap();
        let before_last_known_good = issuer.store.load_last_known_good().unwrap();
        let before_snapshot = issuer.snapshot.clone();
        let scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &scope;
        let mut gate = CanonicalSecondViewC3Gate::new(&scope);

        let result = {
            let mut adapter = C1SystemServiceRecoveryReceiptAdapter::new(&mut issuer, &caller);
            adapter.reconcile_terminal(scope_ref, &mut gate)
        };

        assert_eq!(result, Err(RecoveryClientError::Rejected));
        assert_eq!(gate.invocations, 1);
        assert_eq!(gate.callbacks, 1);
        assert!(gate.observed_view_is_canonical);
        assert_eq!(
            gate.observed_binding_digest,
            Some(scope.second_binding_digest)
        );
        assert_ne!(scope.first.binding_digest, scope.second_binding_digest);
        assert_eq!(
            issuer.store.recovery_journal_record().unwrap(),
            before_journal,
            "C1 must not terminalize the earlier view after C3 sees another canonical view"
        );
        assert_eq!(issuer.store.load_current_state().unwrap(), before_current);
        assert_eq!(
            issuer.store.load_last_known_good().unwrap(),
            before_last_known_good
        );
        assert_eq!(issuer.snapshot, before_snapshot);
        assert_eq!(
            issuer.store.reconciliation_fence_state_for_test(),
            (false, false, 0),
            "rejecting the changed view must release the reserved C1 reader fence"
        );
        assert_eq!(
            {
                let state = issuer.issuer.admission.state.lock().unwrap();
                (state.closed, state.recovering, state.active)
            },
            (false, false, 0),
            "the C1 issuer must remain available for a valid retry"
        );
        assert!(issuer.issuer.alive.load(Ordering::Acquire));
    }

    #[cfg(feature = "system-service-recovery-contract")]
    #[test]
    fn receipt_adapter_preserves_gate_unknown_when_scope_changes_after_gate() {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("unknown-after-scope-change");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let scope = CanonicalSecondViewRecoveryScope::new(130, &caller);
        let binding = RecoveryBinding::for_test(scope.first.binding_digest);
        drop(issuer.store.begin_recovery_intent(binding).unwrap());

        let before_journal = issuer.store.recovery_journal_record().unwrap();
        let before_current = issuer.store.load_current_state().unwrap();
        let before_last_known_good = issuer.store.load_last_known_good().unwrap();
        let before_snapshot = issuer.snapshot.clone();
        let scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &scope;
        let mut gate = UnknownAfterScopeChangeC3Gate::new(&scope);

        let result = {
            let mut adapter = C1SystemServiceRecoveryReceiptAdapter::new(&mut issuer, &caller);
            adapter.reconcile_terminal(scope_ref, &mut gate)
        };

        assert_eq!(result, Err(RecoveryClientError::OutcomeUnknown));
        assert_eq!(gate.invocations, 1);
        assert_eq!(gate.callbacks, 0);
        assert_ne!(scope.first.binding_digest, scope.second_binding_digest);
        assert_eq!(
            issuer.store.recovery_journal_record().unwrap(),
            before_journal
        );
        assert_eq!(issuer.store.load_current_state().unwrap(), before_current);
        assert_eq!(
            issuer.store.load_last_known_good().unwrap(),
            before_last_known_good
        );
        assert_eq!(issuer.snapshot, before_snapshot);
        assert_eq!(
            issuer.store.reconciliation_fence_state_for_test(),
            (false, false, 0),
            "the gate error must release an unused C1 store reservation"
        );
        assert_eq!(
            {
                let state = issuer.issuer.admission.state.lock().unwrap();
                (state.closed, state.recovering, state.active)
            },
            (false, false, 0),
            "the owner-reported unknown must not be recast as a C1 terminal commit"
        );
        assert!(issuer.issuer.alive.load(Ordering::Acquire));
    }

    #[cfg(feature = "system-service-recovery-contract")]
    #[test]
    fn receipt_adapter_withholds_receipt_and_keeps_c1_fence_on_post_terminal_scope_change() {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("post-terminal-scope-change");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let scope = CanonicalSecondViewRecoveryScope::new(131, &caller);
        let binding = RecoveryBinding::for_test(scope.first.binding_digest);
        let permit = issuer.store.begin_recovery_intent(binding.clone()).unwrap();
        let operation_id = permit.operation_id();
        drop(permit);

        let scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &scope;
        let mut gate = PostTerminalScopeChangeC3Gate::new(&scope);
        let result = {
            let mut adapter = C1SystemServiceRecoveryReceiptAdapter::new(&mut issuer, &caller);
            adapter.reconcile_terminal(scope_ref, &mut gate)
        };

        assert_eq!(result, Err(RecoveryClientError::OutcomeUnknown));
        assert_eq!(gate.invocations, 1);
        assert_eq!(gate.callbacks, 1);
        assert!(scope.second_view.get());

        let terminal_record = issuer.store.recovery_journal_record().unwrap().unwrap();
        assert_eq!(terminal_record.operation_id, operation_id);
        assert_eq!(terminal_record.binding, binding);
        assert_eq!(
            terminal_record.phase,
            RecoveryJournalPhase::Reconciled {
                outcome_digest: system_service_terminal_outcome_digest(
                    operation_id,
                    &terminal_record.binding,
                )
            },
            "the callback committed before the scope changed, so C1 must not claim rejection"
        );
        assert!(!issuer.issuer.alive.load(Ordering::Acquire));
        assert_eq!(
            {
                let state = issuer.issuer.admission.state.lock().unwrap();
                (state.closed, state.active)
            },
            (true, 0),
            "a persistent post-commit mismatch must not reopen C1 admission"
        );
        assert_eq!(
            issuer.store.reconciliation_fence_state_for_test(),
            (true, true, 0),
            "a withheld receipt after terminal publication must retain the C1 store fence"
        );
        let changed_view_digest =
            nagi_system_service_recovery_contract::canonical_recovery_binding_digest_from_scope(
                scope_ref,
            );
        assert_eq!(changed_view_digest, scope.second_binding_digest);
        assert_ne!(changed_view_digest, scope.first.binding_digest);
    }

    #[cfg(feature = "system-service-recovery-contract")]
    #[test]
    fn receipt_adapter_reconciles_exact_journal_and_rejects_scope_and_replay_mismatch() {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("receipt-adapter-exact-consume");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let scope = TestRecoveryScope::new(122, &caller);
        let binding = RecoveryBinding::for_test(scope.binding_digest);
        let permit = issuer.store.begin_recovery_intent(binding.clone()).unwrap();
        let operation_id = permit.operation_id();
        drop(permit);
        let scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &scope;
        let mut gate = CountingC3ReconciliationGate::new(scope_ref);

        let receipt = {
            let mut adapter = C1SystemServiceRecoveryReceiptAdapter::new(&mut issuer, &caller);
            adapter
                .reconcile_terminal(scope_ref, &mut gate)
                .expect("the exact C1 row should terminalize inside the bound gate")
        };
        assert_eq!(gate.invocations, 1);
        assert_eq!(gate.callbacks, 1);
        let observed_claim = gate
            .observed_claim
            .expect("C3 receives C1's exact journal operation correlation");
        assert_eq!(observed_claim.operation_id().get(), operation_id.get());
        assert_eq!(observed_claim.binding_digest(), &scope.binding_digest);
        assert_eq!(receipt.operation_id(), operation_id.get());
        assert_eq!(receipt.binding_digest(), &scope.binding_digest);
        assert_eq!(
            receipt.outcome_digest(),
            system_service_terminal_outcome_digest(operation_id, &binding)
        );
        assert!(!issuer.issuer.alive.load(Ordering::Acquire));
        let terminal_record = issuer.store.recovery_journal_record().unwrap().unwrap();
        assert_eq!(terminal_record.operation_id, operation_id);
        assert_eq!(terminal_record.binding, binding);
        assert_eq!(
            terminal_record.phase,
            RecoveryJournalPhase::Reconciled {
                outcome_digest: receipt.outcome_digest()
            }
        );
        let terminal_outcome = match &terminal_record.phase {
            RecoveryJournalPhase::Reconciled { outcome_digest } => *outcome_digest,
            _ => unreachable!("the adapter returned only after terminal commit"),
        };

        let mut changed_scope = scope.clone();
        changed_scope.change_generation_and_rehash(123);
        let changed_scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &changed_scope;
        let wrong_scope_result = {
            let mut adapter = C1SystemServiceRecoveryReceiptAdapter::new(&mut issuer, &caller);
            adapter.consume_terminal_receipt(changed_scope_ref, receipt)
        };
        assert_eq!(wrong_scope_result, Err(RecoveryClientError::Rejected));
        assert_eq!(
            issuer.store.recovery_journal_record().unwrap(),
            Some(terminal_record.clone())
        );

        let exact_receipt = C1SystemServiceRecoveryTerminalReceipt {
            operation_id,
            binding: terminal_record.binding.clone(),
            outcome_digest: terminal_outcome,
        };
        {
            let mut adapter = C1SystemServiceRecoveryReceiptAdapter::new(&mut issuer, &caller);
            adapter
                .consume_terminal_receipt(scope_ref, exact_receipt)
                .expect("the exact durable row should validate the receipt");
        }

        drop(issuer);
        let mut reopened = service(&directory, TestClock::at(100));
        let reopened_alive = Arc::clone(&reopened.issuer.alive);
        let mut reopened_gate = CountingC3ReconciliationGate::new(scope_ref);
        let reopened_receipt = {
            let mut adapter = C1SystemServiceRecoveryReceiptAdapter::new(&mut reopened, &caller);
            adapter
                .reconcile_terminal(scope_ref, &mut reopened_gate)
                .expect("an exact terminal row may be reissued only after fencing this issuer")
        };
        assert_eq!(reopened_gate.invocations, 1);
        assert_eq!(reopened_gate.callbacks, 1);
        assert_eq!(reopened_receipt.operation_id(), operation_id.get());
        assert_eq!(reopened_receipt.binding_digest(), &scope.binding_digest);
        assert_eq!(reopened_receipt.outcome_digest(), terminal_outcome);
        assert!(!reopened_alive.load(Ordering::Acquire));
        {
            let mut adapter = C1SystemServiceRecoveryReceiptAdapter::new(&mut reopened, &caller);
            adapter
                .consume_terminal_receipt(scope_ref, reopened_receipt)
                .expect("the reopened exact durable row should validate its reissued receipt");
        }
        drop(reopened);

        let mut next_service = service(&directory, TestClock::at(100));
        let next_binding = RecoveryBinding::for_test([0x7c; 32]);
        drop(
            next_service
                .store
                .begin_recovery_intent(next_binding)
                .unwrap(),
        );
        let current_record = next_service.store.recovery_journal_record().unwrap();
        let stale_receipt = C1SystemServiceRecoveryTerminalReceipt {
            operation_id,
            binding: terminal_record.binding.clone(),
            outcome_digest: terminal_outcome,
        };
        let replay_result = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new(&mut next_service, &caller);
            adapter.consume_terminal_receipt(scope_ref, stale_receipt)
        };
        assert_eq!(replay_result, Err(RecoveryClientError::Failed));
        assert_eq!(
            next_service.store.recovery_journal_record().unwrap(),
            current_record
        );
        assert!(next_service.issuer.alive.load(Ordering::Acquire));
        drop(next_service);
    }

    #[cfg(feature = "system-service-recovery-contract")]
    #[test]
    fn receipt_adapter_store_write_failure_returns_unknown_and_keeps_c1_fence_closed() {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("receipt-adapter-terminal-write-failure");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let scope = TestRecoveryScope::new(124, &caller);
        let binding = RecoveryBinding::for_test(scope.binding_digest);
        drop(issuer.store.begin_recovery_intent(binding).unwrap());
        issuer
            .store
            .set_fault_for_test(DurableStoreFault::AfterRename);
        let before_snapshot = issuer.snapshot.clone();
        let scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &scope;
        let mut gate = CountingC3ReconciliationGate::new(scope_ref);

        let result = {
            let mut adapter = C1SystemServiceRecoveryReceiptAdapter::new(&mut issuer, &caller);
            adapter.reconcile_terminal(scope_ref, &mut gate)
        };

        assert_eq!(result, Err(RecoveryClientError::OutcomeUnknown));
        assert_eq!(gate.invocations, 1);
        assert_eq!(gate.callbacks, 1);
        assert_eq!(issuer.snapshot, before_snapshot);
        assert!(!issuer.issuer.alive.load(Ordering::Acquire));
        assert_eq!(
            {
                let state = issuer.issuer.admission.state.lock().unwrap();
                (state.closed, state.recovering, state.active)
            },
            (true, false, 0)
        );
        assert_eq!(
            issuer.store.reconciliation_fence_state_for_test(),
            (true, true, 0),
            "a terminal write error after issuer closure must not reopen raw readers"
        );
    }

    #[cfg(feature = "system-service-recovery-contract")]
    #[test]
    fn scoped_reconciliation_checks_permit_and_exact_terminal_readback() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("scoped-recovery-reconciliation-preflight");
        let clock = TestClock::at(100);
        let mut issuer = service(&directory, clock.clone());
        issuer.recover(&caller).unwrap();
        let scope = TestRecoveryScope::new(101, &caller);
        let binding = RecoveryBinding::for_test(scope.binding_digest);
        let permit = issuer.store.begin_recovery_intent(binding.clone()).unwrap();
        issuer
            .store
            .mark_recovery_outcome_unknown(permit.operation_id(), &binding)
            .unwrap();
        let operation_id = permit.operation_id();
        let outcome = [0x94; 32];
        let exact = RecoveryReconciliation::for_test(operation_id, binding.clone(), outcome);
        let before_journal = issuer.store.recovery_journal_record().unwrap();
        let before_store_state = issuer.store.load_current_state().unwrap();
        let before_lkg = issuer.store.load_last_known_good().unwrap();
        let before_snapshot = issuer.snapshot.clone();

        let assert_open_unchanged = |issuer: &DurableTestService| {
            assert_eq!(
                issuer.store.recovery_journal_record().unwrap(),
                before_journal
            );
            assert_eq!(
                issuer.store.load_current_state().unwrap(),
                before_store_state
            );
            assert_eq!(issuer.store.load_last_known_good().unwrap(), before_lkg);
            assert_eq!(issuer.snapshot, before_snapshot);
            assert_eq!(
                issuer.store.reconciliation_fence_state_for_test(),
                (false, false, 0)
            );
            assert_eq!(
                {
                    let state = issuer.issuer.admission.state.lock().unwrap();
                    (state.closed, state.recovering, state.active)
                },
                (false, false, 0)
            );
            assert!(issuer.issuer.alive.load(Ordering::Acquire));
        };

        let wrong_operation = RecoveryReconciliation::for_test(
            RecoveryOperationId::for_test(operation_id.get() + 1),
            binding.clone(),
            outcome,
        );
        assert_eq!(
            issuer.reconcile_recovery_with_scope(&caller, &scope, &permit, wrong_operation),
            Err(IdentityError::StoreUnavailable)
        );
        assert_open_unchanged(&issuer);

        let mut stale_generation = scope.clone();
        stale_generation.change_generation_without_rehash(102);
        assert_eq!(
            issuer.reconcile_recovery_with_scope(
                &caller,
                &stale_generation,
                &permit,
                exact.clone(),
            ),
            Err(IdentityError::StoreUnavailable)
        );
        assert_open_unchanged(&issuer);

        let mut changed_generation = scope.clone();
        changed_generation.change_generation_and_rehash(102);
        assert_eq!(
            issuer.reconcile_recovery_with_scope(
                &caller,
                &changed_generation,
                &permit,
                exact.clone(),
            ),
            Err(IdentityError::StoreUnavailable)
        );
        assert_open_unchanged(&issuer);

        let other_caller = TestCaller::new("scoped-reconciliation-different-caller");
        assert_eq!(
            issuer.reconcile_recovery_with_scope(&other_caller, &scope, &permit, exact.clone()),
            Err(IdentityError::StoreUnavailable)
        );
        assert_open_unchanged(&issuer);

        let receipt = issuer
            .reconcile_recovery_with_scope(&caller, &scope, &permit, exact.clone())
            .unwrap();
        assert_eq!(receipt.operation_id(), operation_id.get());
        assert_eq!(receipt.binding_digest(), &scope.binding_digest);
        assert_eq!(receipt.outcome_digest(), outcome);
        issuer
            .consume_reconciliation_receipt_with_scope(&scope, receipt)
            .unwrap();
        assert!(!issuer.issuer.alive.load(Ordering::Acquire));
        assert_eq!(
            issuer
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::Reconciled {
                outcome_digest: outcome
            }
        );
        drop(issuer);

        let mut reopened = service(&directory, clock);
        let before_terminal = reopened.store.recovery_journal_record().unwrap();
        let before_terminal_state = reopened.store.load_current_state().unwrap();
        let before_terminal_lkg = reopened.store.load_last_known_good().unwrap();
        let wrong_outcome =
            RecoveryReconciliation::for_test(operation_id, binding.clone(), [0x95; 32]);
        assert_eq!(
            reopened.reconcile_recovery_with_scope(&caller, &scope, &permit, wrong_outcome.clone()),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(
            reopened.store.recovery_journal_record().unwrap(),
            before_terminal
        );
        assert_eq!(
            reopened.store.load_current_state().unwrap(),
            before_terminal_state
        );
        assert_eq!(
            reopened.store.load_last_known_good().unwrap(),
            before_terminal_lkg
        );
        assert_eq!(
            reopened.store.reconciliation_fence_state_for_test(),
            (false, false, 0)
        );
        assert!(reopened.issuer.alive.load(Ordering::Acquire));

        let wrong_receipt =
            C1SystemServiceRecoveryTerminalReceipt::from_reconciliation(&wrong_outcome);
        assert_eq!(
            reopened.consume_reconciliation_receipt_with_scope(&scope, wrong_receipt),
            Err(IdentityError::StoreUnavailable),
            "consume must re-read the exact durable outcome"
        );
        assert_eq!(
            reopened.store.recovery_journal_record().unwrap(),
            before_terminal
        );
        assert_eq!(
            reopened.store.load_current_state().unwrap(),
            before_terminal_state
        );
        assert_eq!(
            reopened.store.load_last_known_good().unwrap(),
            before_terminal_lkg
        );
        assert_eq!(
            reopened.store.reconciliation_fence_state_for_test(),
            (false, false, 0)
        );
        assert!(reopened.issuer.alive.load(Ordering::Acquire));

        let mismatched_scope = TestRecoveryScope::new(102, &caller);
        let mismatched_receipt =
            C1SystemServiceRecoveryTerminalReceipt::from_reconciliation(&exact);
        assert_eq!(
            reopened
                .consume_reconciliation_receipt_with_scope(&mismatched_scope, mismatched_receipt,),
            Err(IdentityError::StoreUnavailable),
            "a different generation must be rejected before receipt readback"
        );
        assert_eq!(
            reopened.store.recovery_journal_record().unwrap(),
            before_terminal
        );
        assert_eq!(
            reopened.store.load_current_state().unwrap(),
            before_terminal_state
        );
        assert_eq!(
            reopened.store.load_last_known_good().unwrap(),
            before_terminal_lkg
        );
        assert_eq!(
            reopened.store.reconciliation_fence_state_for_test(),
            (false, false, 0)
        );
        assert!(reopened.issuer.alive.load(Ordering::Acquire));

        let receipt = reopened
            .reconcile_recovery_with_scope(&caller, &scope, &permit, exact)
            .unwrap();
        reopened
            .consume_reconciliation_receipt_with_scope(&scope, receipt)
            .unwrap();
        assert!(!reopened.issuer.alive.load(Ordering::Acquire));
        assert_eq!(
            reopened.store.reconciliation_fence_state_for_test(),
            (true, true, 0)
        );
    }

    #[test]
    fn recovery_intent_write_failure_never_enters_the_effect_gate() {
        for (fault, expected_quarantine) in [
            (DurableStoreFault::PartialWrite, false),
            (DurableStoreFault::AfterRename, true),
        ] {
            let directory = DurableTestDirectory::new();
            let caller = TestCaller::new("recovery-intent-effect-gate-failure");
            let mut issuer = service(&directory, TestClock::at(100));
            issuer.recover(&caller).unwrap();
            let before_state = issuer.store.load_current_state().unwrap();
            let before_lkg = issuer.store.load_last_known_good().unwrap();
            issuer.store.set_fault_for_test(fault);

            let mut gate = CountingRecoveryGate::for_binding(RecoveryBinding::for_test([0x81; 32]));
            assert_eq!(
                issuer.recover_with_new_intent(&caller, &mut gate),
                Err(IdentityError::StoreUnavailable)
            );
            assert_eq!(gate.invocations, 0);
            assert_eq!(gate.effects, 0);
            assert_eq!(
                issuer.store.recovery_quarantined(),
                Err(SnapshotStoreError::Unavailable),
                "the failed writer poisons the open instance"
            );
            drop(issuer);

            let reopened = service(&directory, TestClock::at(100));
            assert_eq!(
                reopened.store.recovery_quarantined(),
                Ok(expected_quarantine)
            );
            assert_eq!(reopened.store.load_current_state().unwrap(), before_state);
            assert_eq!(reopened.store.load_last_known_good().unwrap(), before_lkg);
            let record = reopened.store.recovery_journal_record().unwrap();
            if expected_quarantine {
                assert_eq!(
                    record.unwrap().phase,
                    RecoveryJournalPhase::Intent,
                    "an uncertain rename preserves the exact intent without running the effect"
                );
            } else {
                assert!(record.is_none());
            }
        }
    }

    #[test]
    fn ambiguous_intent_write_closes_old_issuer_before_paired_root_cleanup() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("recovery-ambiguous-intent-closes-issuer");
        let current = Arc::new(RwLock::new(None));
        let store = JournalIndependentSnapshotReaderStore {
            inner: LinuxIdentitySnapshotStore::open(directory.open()).unwrap(),
            current,
            begin_intent_fault: None,
            begin_intent_busy_once: true,
            claim_fault: None,
            panic_after_claim: false,
            panic_on_mark_unknown: false,
            post_intent_busy: AtomicU64::new(0),
        };
        let mut issuer = IdentityService::new(
            store,
            TestIds::default(),
            TestClock::at(100),
            TestAuthorizer::default(),
            TestCleaner::default(),
            NoopIdentityEventSink,
        );
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Ambiguous intent owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("ambiguous-intent-root").unwrap(),
            )
            .unwrap();
        let context = issuer.interactive_session(&caller).unwrap();
        let identity = context.identity().unwrap();
        let old_handle = context.revalidation_handle().unwrap();
        let access = IdentityAccessContext::new(identity, &caller);
        let profile_id = identity.profile_id().clone();
        let admission = Arc::clone(&issuer.issuer.admission);
        let issuer_alive = Arc::clone(&issuer.issuer.alive);
        let opened = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let closed_at_drop = Arc::new(AtomicBool::new(false));
        let mut storage = UserStorageRootAdapter::new(RecoveryFenceDropProbeBackend {
            admission: Arc::clone(&admission),
            issuer_alive: Arc::clone(&issuer_alive),
            opened: Arc::clone(&opened),
            dropped: Arc::clone(&dropped),
            closed_at_drop: Arc::clone(&closed_at_drop),
        });
        let mut root_authorizer = NoopProfileRootAuthorizer;
        storage
            .resolve_root(&access, &profile_id, &mut root_authorizer)
            .unwrap();
        drop(context);

        let reader = issuer.store.current_snapshot_reader().unwrap();
        let active_snapshot = reader().unwrap().expect("the reader retains Active state");
        let before = issuer.store.inner.load_current().unwrap().unwrap();
        assert_eq!(active_snapshot, before);
        assert_eq!(opened.load(Ordering::SeqCst), 1);
        assert_eq!(dropped.load(Ordering::SeqCst), 0);

        let mut gate = CountingRecoveryGate::for_binding(RecoveryBinding::for_test([0x82; 32]));
        issuer.store.begin_intent_fault = Some(DurableStoreFault::AfterRename);
        assert_eq!(
            issuer.recover_with_new_intent_and_storage(&caller, &mut gate, &mut storage),
            Err(IdentityError::SessionLeaseBusy),
            "the store's no-write Busy result remains retryable"
        );
        assert!(issuer.recovered);
        assert!(issuer_alive.load(Ordering::Acquire));
        assert!(!admission.state.lock().unwrap().closed);
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        assert!(issuer
            .store
            .inner
            .recovery_journal_record()
            .unwrap()
            .is_none());
        assert_eq!(gate.invocations, 0);

        let mut pre_failure_callback_ran = false;
        assert_eq!(
            old_handle.with_live_session_effect(|| pre_failure_callback_ran = true),
            Ok(())
        );
        assert!(pre_failure_callback_ran);

        assert_eq!(
            issuer.recover_with_new_intent_and_storage(&caller, &mut gate, &mut storage),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(gate.invocations, 0);
        assert_eq!(gate.effects, 0);
        assert_eq!(
            reader().unwrap(),
            Some(active_snapshot),
            "the journal-independent reader still exposes the pre-intent Active snapshot"
        );
        assert!(!issuer.recovered);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(closed_at_drop.load(Ordering::SeqCst));
        assert!(admission.state.lock().unwrap().closed);
        assert!(!issuer_alive.load(Ordering::Acquire));
        let mut old_effect_called = false;
        assert_eq!(
            old_handle.with_live_session_effect(|| old_effect_called = true),
            Err(ResolutionError::Unavailable)
        );
        assert!(!old_effect_called);

        drop(issuer);
        let reopened = service(&directory, TestClock::at(100));
        assert!(reopened.store.recovery_quarantined().unwrap());
        assert_eq!(
            reopened
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::Intent,
            "AfterRename must preserve the exact Intent for later reconciliation"
        );
    }

    #[test]
    fn live_session_contention_leaves_journal_unchanged_then_explicit_retry_runs_once() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("recovery-admission-contention");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Recovery owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("recovery-admission-contention").unwrap(),
            )
            .unwrap();
        let handle = Arc::new(
            issuer
                .interactive_session(&caller)
                .unwrap()
                .revalidation_handle()
                .unwrap(),
        );
        assert_eq!(handle.check_live(), Ok(()));
        let before_snapshot = issuer.snapshot.clone();
        let before_store_state = issuer.store.load_current_state().unwrap();
        let before_lkg = issuer.store.load_last_known_good().unwrap();
        let service = Arc::new(Mutex::new(issuer));
        let worker_service = Arc::clone(&service);
        let worker_caller = caller.clone();
        let worker_binding = RecoveryBinding::for_test([0x82; 32]);
        let worker_gate_binding = worker_binding.clone();
        let (attempt_tx, attempt_rx) = mpsc::sync_channel(0);

        let worker = thread::spawn(move || {
            let mut gate = CountingRecoveryGate::for_binding(worker_gate_binding);
            let result = worker_service
                .lock()
                .unwrap()
                .recover_with_new_intent(&worker_caller, &mut gate);
            let observation = {
                let issuer = worker_service.lock().unwrap();
                (
                    issuer.store.recovery_journal_record().unwrap(),
                    issuer.store.load_current_state().unwrap(),
                    issuer.store.load_last_known_good().unwrap(),
                    issuer.snapshot.clone(),
                )
            };
            attempt_tx.send((result, gate, observation)).unwrap();
        });

        let busy = handle.with_live_session_effect(|| {
            attempt_rx
                .recv_timeout(Duration::from_secs(3))
                .expect("recovery must return Busy without waiting for the live effect")
        });
        let (result, gate, (journal, store_state, lkg, snapshot)) = busy.unwrap();
        worker.join().unwrap();
        assert_eq!(result, Err(IdentityError::SessionLeaseBusy));
        assert_eq!(gate.invocations, 0);
        assert_eq!(gate.effects, 0);
        assert!(journal.is_none(), "Busy must not leave a staged intent");
        assert_eq!(store_state, before_store_state);
        assert_eq!(lkg, before_lkg);
        assert_eq!(snapshot, before_snapshot);
        assert_eq!(handle.check_live(), Ok(()));

        let mut issuer = Arc::try_unwrap(service).ok().unwrap().into_inner().unwrap();
        let mut retry_gate = CountingRecoveryGate::for_binding(worker_binding);
        assert_eq!(
            issuer.recover_with_new_intent(&caller, &mut retry_gate),
            Err(IdentityError::StoreUnavailable),
            "the effect commits, then the missing runtime-owner admission fails closed"
        );
        assert_eq!(retry_gate.invocations, 1);
        assert_eq!(retry_gate.effects, 1);
        assert_eq!(
            issuer
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::RuntimeActive
        );
        assert!(issuer.store.recovery_quarantined().unwrap());
        assert_eq!(
            issuer
                .store
                .load_current_state()
                .unwrap()
                .high_water_version,
            Some(SnapshotVersion::new(
                before_store_state.high_water_version.unwrap().get() + 1
            ))
        );
        assert!(handle.check_live().is_err());
    }

    #[test]
    fn active_before_session_read_gate_is_busy_before_intent_and_retries_once() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("recovery-active-before-read-gate");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Read gate race owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("recovery-active-before-read-gate").unwrap(),
            )
            .unwrap();
        let handle = Arc::new(
            issuer
                .interactive_session(&caller)
                .unwrap()
                .revalidation_handle()
                .unwrap(),
        );
        let before_snapshot = issuer.snapshot.clone();
        let before_store_state = issuer.store.load_current_state().unwrap();
        let before_lkg = issuer.store.load_last_known_good().unwrap();
        let binding = RecoveryBinding::for_test([0x88; 32]);
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);
        let worker_handle = Arc::clone(&handle);
        let (effect_tx, effect_rx) = mpsc::sync_channel(0);
        let worker = thread::spawn(move || {
            LEASE_READ_CHANNELS.with(|slot| {
                *slot.borrow_mut() = Some((entered_tx, release_rx));
            });
            let effect = worker_handle.with_live_session_effect(|| ());
            effect_tx.send(effect).unwrap();
        });

        entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("reader admission must reach the pre-read pause within the bound");
        assert_eq!(
            issuer.issuer.admission.state.lock().unwrap().active,
            1,
            "the reader is admitted but has not taken its Session read gate"
        );
        let mut gate = CountingRecoveryGate::for_binding(binding);
        assert_eq!(
            issuer.recover_with_new_intent(&caller, &mut gate),
            Err(IdentityError::SessionLeaseBusy)
        );
        assert_eq!(gate.invocations, 0);
        assert_eq!(gate.effects, 0);
        assert_eq!(
            RECOVERY_WRITE_GUARDS_ACQUIRED.with(Cell::get),
            0,
            "the early active check returns before any Session writer preflight"
        );
        assert!(issuer.store.recovery_journal_record().unwrap().is_none());
        assert_eq!(issuer.snapshot, before_snapshot);
        assert_eq!(
            issuer.store.load_current_state().unwrap(),
            before_store_state
        );
        assert_eq!(issuer.store.load_last_known_good().unwrap(), before_lkg);

        release_tx
            .send(())
            .expect("release the paused reader after the Busy result");
        assert_eq!(
            effect_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("the admitted reader resumes after the Busy decision"),
            Ok(())
        );
        worker.join().unwrap();
        assert_eq!(handle.check_live(), Ok(()));

        let mut retry_gate =
            CountingRecoveryGate::for_binding(RecoveryBinding::for_test([0x88; 32]));
        assert_eq!(
            issuer.recover_with_new_intent(&caller, &mut retry_gate),
            Err(IdentityError::StoreUnavailable),
            "the explicit retry runs once and remains quarantined without continuing owner admission"
        );
        assert_eq!(retry_gate.invocations, 1);
        assert_eq!(retry_gate.effects, 1);
        assert_eq!(retry_gate.successful_callbacks, 1);
        let after_retry = issuer.store.load_current_state().unwrap();
        assert_ne!(after_retry, before_store_state);
        assert_eq!(
            after_retry.high_water_version.unwrap().get(),
            before_store_state.high_water_version.unwrap().get() + 1
        );
        assert!(issuer.snapshot.sessions.iter().any(|session| {
            session.session_id == *handle.session_id()
                && matches!(
                    &session.lifecycle,
                    SessionLifecycle::Ended {
                        reason: SessionEndReason::SystemRestart,
                        ..
                    }
                )
        }));
        assert_eq!(
            issuer
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::RuntimeActive
        );
        assert!(issuer.store.recovery_quarantined().unwrap());
    }

    #[test]
    fn pre_effect_busy_preserves_exact_operation_permit_for_retry() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("recovery-permit-pre-effect-busy-retry");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Permit retry owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("recovery-permit-pre-effect-busy-retry").unwrap(),
            )
            .unwrap();
        let handle = Arc::new(
            issuer
                .interactive_session(&caller)
                .unwrap()
                .revalidation_handle()
                .unwrap(),
        );

        // Admit the live callback before the pending intent is written. Once
        // the intent is durable, a newly admitted callback must fail closed at
        // the current-snapshot reader; this already-admitted callback may
        // finish while recovery observes its active issuer count.
        let (effect_entered_tx, effect_entered_rx) = mpsc::sync_channel(0);
        let (effect_release_tx, effect_release_rx) = mpsc::sync_channel(0);
        let callback_effects = Arc::new(AtomicU64::new(0));
        let callback_effects_in_thread = Arc::clone(&callback_effects);
        let handle_in_thread = Arc::clone(&handle);
        let effect_thread = thread::spawn(move || {
            handle_in_thread.with_live_session_effect(|| {
                callback_effects_in_thread.fetch_add(1, Ordering::SeqCst);
                effect_entered_tx.send(()).unwrap();
                effect_release_rx
                    .recv_timeout(Duration::from_secs(3))
                    .expect("admitted effect must leave within the watchdog bound");
            })
        });
        effect_entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the live effect must be admitted before staging recovery");
        assert_eq!(issuer.issuer.admission.state.lock().unwrap().active, 1);

        let binding = RecoveryBinding::for_test([0x8a; 32]);
        let permit = issuer.store.begin_recovery_intent(binding.clone()).unwrap();
        let before_journal = issuer.store.recovery_journal_record().unwrap();
        let before_store_state = issuer.store.load_current_state().unwrap();
        let before_lkg = issuer.store.load_last_known_good().unwrap();
        let before_snapshot = issuer.snapshot.clone();
        assert_eq!(
            handle.check_live(),
            Err(ResolutionError::Unavailable),
            "a new live check must deny while the durable intent is unresolved"
        );
        let mut late_effect_called = false;
        assert_eq!(
            handle.with_live_session_effect(|| late_effect_called = true),
            Err(ResolutionError::Unavailable),
            "a new effect cannot enter after the intent is durable"
        );
        assert!(!late_effect_called);
        assert_eq!(issuer.issuer.admission.state.lock().unwrap().active, 1);

        let mut permit_slot = Some(permit);
        let mut busy_gate = CountingRecoveryGate::for_binding(binding.clone());
        let result = issuer.recover_with_operation(&caller, &mut permit_slot, &mut busy_gate);
        let observation = (
            issuer.store.recovery_journal_record().unwrap(),
            issuer.store.load_current_state().unwrap(),
            issuer.store.load_last_known_good().unwrap(),
            issuer.snapshot.clone(),
        );
        assert_eq!(result, Err(IdentityError::SessionLeaseBusy));
        assert_eq!(busy_gate.invocations, 0);
        assert_eq!(busy_gate.effects, 0);
        assert!(
            permit_slot.is_some(),
            "pre-effect Busy must preserve the exact permit"
        );
        assert_eq!(observation.0, before_journal);
        assert_eq!(observation.1, before_store_state);
        assert_eq!(observation.2, before_lkg);
        assert_eq!(observation.3, before_snapshot);

        effect_release_tx.send(()).unwrap();
        assert_eq!(effect_thread.join().unwrap(), Ok(()));
        assert_eq!(callback_effects.load(Ordering::SeqCst), 1);
        assert_eq!(issuer.issuer.admission.state.lock().unwrap().active, 0);

        let mut retry_gate = CountingRecoveryGate::for_binding(binding);
        assert_eq!(
            issuer.recover_with_operation(&caller, &mut permit_slot, &mut retry_gate),
            Err(IdentityError::StoreUnavailable),
            "the explicit retry claims once and remains quarantined without owner runtime admission"
        );
        assert!(
            permit_slot.is_none(),
            "the durable effect claim consumes the permit"
        );
        assert_eq!(retry_gate.invocations, 1);
        assert_eq!(retry_gate.effects, 1);
        assert_eq!(
            issuer
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::RuntimeActive
        );
        assert!(issuer.store.recovery_quarantined().unwrap());
    }

    #[test]
    fn operation_id_only_mismatch_and_replay_never_reenter_effect_gate() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("recovery-operation-id-only-mismatch");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let binding = RecoveryBinding::for_test([0x83; 32]);
        let permit = issuer.store.begin_recovery_intent(binding.clone()).unwrap();
        let operation_id = permit.operation_id();
        let wrong_permit = RecoveryOperationPermit::for_test(
            RecoveryOperationId::for_test(operation_id.get() + 1),
            binding.clone(),
        );
        let before_journal = issuer.store.recovery_journal_record().unwrap();
        let before_state = issuer.store.load_current_state().unwrap();
        let before_lkg = issuer.store.load_last_known_good().unwrap();
        let mut gate = CountingRecoveryGate::for_binding(binding.clone());

        let wrong_binding = RecoveryBinding::for_test([0x84; 32]);
        let mut wrong_binding_gate = CountingRecoveryGate::for_binding(wrong_binding);
        let wrong_binding_permit = RecoveryOperationPermit::for_test(operation_id, binding.clone());
        let mut wrong_binding_slot = Some(wrong_binding_permit);
        assert_eq!(
            issuer.recover_with_operation(
                &caller,
                &mut wrong_binding_slot,
                &mut wrong_binding_gate
            ),
            Err(IdentityError::StoreUnavailable),
            "the gate and journal permit must carry the same opaque binding"
        );
        assert_eq!(wrong_binding_gate.invocations, 0);
        assert_eq!(wrong_binding_gate.effects, 0);
        assert_eq!(
            issuer.store.recovery_journal_record().unwrap(),
            before_journal
        );
        assert_eq!(issuer.store.load_current_state().unwrap(), before_state);
        assert_eq!(issuer.store.load_last_known_good().unwrap(), before_lkg);
        assert!(wrong_binding_slot.is_some());

        let mut wrong_permit_slot = Some(wrong_permit);
        assert_eq!(
            issuer.recover_with_operation(&caller, &mut wrong_permit_slot, &mut gate),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(gate.invocations, 0);
        assert_eq!(gate.effects, 0);
        assert_eq!(
            issuer.store.recovery_journal_record().unwrap(),
            before_journal
        );
        assert_eq!(issuer.store.load_current_state().unwrap(), before_state);
        assert_eq!(issuer.store.load_last_known_good().unwrap(), before_lkg);
        assert!(wrong_permit_slot.is_some());

        let mut permit_slot = Some(permit);
        assert_eq!(
            issuer.recover_with_operation(&caller, &mut permit_slot, &mut gate),
            Err(IdentityError::StoreUnavailable),
            "a one-shot effect may commit, but no runtime authority is inferred"
        );
        assert_eq!(gate.invocations, 1);
        assert_eq!(gate.effects, 1);
        assert!(
            permit_slot.is_none(),
            "a successful durable claim consumes the permit"
        );
        let active_journal = issuer.store.recovery_journal_record().unwrap();
        let active_state = issuer.store.load_current_state().unwrap();
        let active_lkg = issuer.store.load_last_known_good().unwrap();
        let outcome = [0x84; 32];
        assert_eq!(
            issuer.reconcile_recovery(
                &caller,
                RecoveryReconciliation::for_test(
                    RecoveryOperationId::for_test(operation_id.get() + 1),
                    binding.clone(),
                    outcome,
                ),
            ),
            Err(IdentityError::StoreUnavailable),
            "operation ID is independently bound when all other terminal fields match"
        );
        assert_eq!(
            issuer.store.recovery_journal_record().unwrap(),
            active_journal
        );
        assert_eq!(issuer.store.load_current_state().unwrap(), active_state);
        assert_eq!(issuer.store.load_last_known_good().unwrap(), active_lkg);
        assert_eq!(gate.effects, 1);
        {
            let admission = issuer.issuer.admission.state.lock().unwrap();
            assert!(admission.closed);
            assert!(!admission.recovering);
        }
        assert!(!issuer.issuer.alive.load(Ordering::Acquire));

        let receipt = issuer
            .reconcile_recovery(
                &caller,
                RecoveryReconciliation::for_test(operation_id, binding.clone(), outcome),
            )
            .unwrap();
        assert_eq!(receipt.operation_id(), operation_id.get());
        assert_eq!(receipt.binding_digest(), binding.digest_bytes());
        assert_eq!(receipt.outcome_digest(), outcome);
        issuer.consume_reconciliation_receipt(receipt).unwrap();
        assert!(
            !issuer.issuer.alive.load(Ordering::Acquire),
            "consuming terminal evidence does not reopen issuer admission"
        );
        let terminal_journal = issuer.store.recovery_journal_record().unwrap();
        let terminal_state = issuer.store.load_current_state().unwrap();
        let terminal_lkg = issuer.store.load_last_known_good().unwrap();
        let replay = RecoveryOperationPermit::for_test(operation_id, binding);
        let mut replay_slot = Some(replay);
        assert_eq!(
            issuer.recover_with_operation(&caller, &mut replay_slot, &mut gate),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(gate.invocations, 1);
        assert_eq!(gate.effects, 1);
        assert_eq!(
            issuer.store.recovery_journal_record().unwrap(),
            terminal_journal
        );
        assert_eq!(issuer.store.load_current_state().unwrap(), terminal_state);
        assert_eq!(issuer.store.load_last_known_good().unwrap(), terminal_lkg);
        assert!(
            replay_slot.is_some(),
            "replay rejection is pre-effect and leaves the token inspectable"
        );
    }

    #[test]
    fn runtime_active_journal_remains_quarantined_after_service_drop() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("runtime-active-recovery-quarantine");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let binding = RecoveryBinding::for_test([0x86; 32]);
        let mut gate = CountingRecoveryGate::for_binding(binding);
        assert_eq!(
            issuer.recover_with_new_intent(&caller, &mut gate),
            Err(IdentityError::StoreUnavailable),
            "a one-shot gate cannot become a continuing runtime admission"
        );
        assert_eq!(gate.effects, 1);
        assert_eq!(
            issuer
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::RuntimeActive
        );
        assert!(issuer.store.recovery_quarantined().unwrap());
        drop(issuer);

        let mut restarted = service(&directory, TestClock::at(100));
        assert!(restarted.store.recovery_quarantined().unwrap());
        assert_eq!(
            restarted
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::RuntimeActive
        );
        let mut replay_gate =
            CountingRecoveryGate::for_binding(RecoveryBinding::for_test([0x87; 32]));
        assert_eq!(
            restarted.recover_with_new_intent(&caller, &mut replay_gate),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(replay_gate.invocations, 0);
        assert_eq!(replay_gate.effects, 0);
        assert_eq!(
            restarted.recover(&caller),
            Err(IdentityError::StoreUnavailable)
        );
    }

    #[test]
    fn panic_after_recovery_effect_keeps_restart_quarantine_and_blocks_replay() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("recovery-owner-panic");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Panic recovery owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        let session_id = issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("panic-after-recovery-effect").unwrap(),
            )
            .unwrap();
        let before = issuer.store.load_current_state().unwrap();
        let binding = RecoveryBinding::for_test([0x85; 32]);
        let mut gate = CountingRecoveryGate {
            binding: binding.clone(),
            block_session_writer: None,
            operation_identities: Vec::new(),
            panic_after_effect: true,
            panic_before_effect: false,
            panic_payload_before_effect: false,
            invocations: 0,
            effects: 0,
            successful_callbacks: 0,
            fail_before_effect: false,
        };

        assert_eq!(
            issuer.recover_with_new_intent(&caller, &mut gate),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(gate.invocations, 1);
        assert_eq!(gate.effects, 1);
        assert_eq!(gate.successful_callbacks, 1);
        let record = issuer.store.recovery_journal_record().unwrap().unwrap();
        assert_eq!(gate.operation_identities.len(), 1);
        let correlation = gate.operation_identities[0];
        assert_eq!(correlation.operation_id().get(), record.operation_id.get());
        assert_eq!(correlation.binding_digest(), record.binding.digest_bytes());
        assert_eq!(record.phase, RecoveryJournalPhase::OutcomeUnknown);
        assert!(issuer.store.recovery_quarantined().unwrap());
        let after_effect = issuer.store.load_current_state().unwrap();
        assert_ne!(
            after_effect, before,
            "a live Session forces recovery to commit the durable terminal snapshot"
        );
        assert_eq!(
            after_effect.high_water_version.unwrap().get(),
            before.high_water_version.unwrap().get() + 1
        );
        assert!(issuer.snapshot.sessions.iter().any(|session| {
            session.session_id == session_id
                && matches!(
                    &session.lifecycle,
                    SessionLifecycle::Ended {
                        reason: SessionEndReason::SystemRestart,
                        ..
                    }
                )
        }));
        drop(issuer);

        let mut restarted = service(&directory, TestClock::at(100));
        let before_replay_journal = restarted.store.recovery_journal_record().unwrap();
        let before_replay_state = restarted.store.load_current_state().unwrap();
        let before_replay_lkg = restarted.store.load_last_known_good().unwrap();
        let mut replay_gate = CountingRecoveryGate::for_binding(binding.clone());
        let replay = RecoveryOperationPermit::for_test(record.operation_id, binding.clone());
        let mut replay_slot = Some(replay);
        assert_eq!(
            restarted.recover_with_operation(&caller, &mut replay_slot, &mut replay_gate),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(replay_gate.invocations, 0);
        assert_eq!(replay_gate.effects, 0);
        assert_eq!(
            restarted.store.recovery_journal_record().unwrap(),
            before_replay_journal
        );
        assert_eq!(
            restarted.store.load_current_state().unwrap(),
            before_replay_state
        );
        assert_eq!(
            restarted.store.load_last_known_good().unwrap(),
            before_replay_lkg
        );
        assert!(restarted.store.recovery_quarantined().unwrap());
        assert!(restarted.issuer.admission.state.lock().unwrap().closed);
        assert!(!restarted.issuer.alive.load(Ordering::Acquire));
        assert!(
            replay_slot.is_none(),
            "a permit for a durably claimed unknown operation is consumed"
        );
    }

    #[test]
    fn unknown_recovery_before_snapshot_revoke_fences_existing_live_handle() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("recovery-unknown-before-revoke");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Unknown recovery owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("unknown-before-revoke").unwrap(),
            )
            .unwrap();
        let old_handle = issuer
            .interactive_session(&caller)
            .unwrap()
            .revalidation_handle()
            .unwrap();
        assert_eq!(old_handle.check_live(), Ok(()));

        let before_revision_injection = issuer.store.load_current_state().unwrap();
        let mut exhausted = issuer.snapshot.clone();
        exhausted.revision = u64::MAX;
        let exhausted_version = issuer
            .store
            .commit_atomic(
                before_revision_injection.high_water_version,
                &exhausted.to_json().unwrap(),
            )
            .unwrap();
        let before_recovery = issuer.store.load_current_state().unwrap();
        assert_eq!(before_recovery.high_water_version, Some(exhausted_version));

        let binding = RecoveryBinding::for_test([0x89; 32]);
        let mut gate = CountingRecoveryGate::for_binding(binding);
        assert_eq!(
            issuer.recover_with_new_intent(&caller, &mut gate),
            Err(IdentityError::StoreUnavailable),
            "post-claim revision exhaustion becomes an unknown effect outcome"
        );
        assert_eq!(gate.invocations, 1);
        assert_eq!(gate.effects, 1);
        assert_eq!(gate.successful_callbacks, 0);
        assert_eq!(
            issuer
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::OutcomeUnknown
        );
        assert!(issuer.store.recovery_quarantined().unwrap());
        assert_eq!(issuer.store.load_current_state().unwrap(), before_recovery);
        assert_eq!(
            old_handle.check_live(),
            Err(ResolutionError::Unavailable),
            "Unknown quarantine must block an old Active snapshot before lease revoke"
        );
        let mut effect_called = false;
        assert_eq!(
            old_handle.with_live_session_effect(|| effect_called = true),
            Err(ResolutionError::Unavailable)
        );
        assert!(!effect_called);
    }

    fn assert_post_claim_gate_rejection_closes_old_issuer(
        panic_before_effect: bool,
        panic_payload_before_effect: bool,
        panic_unknown_mark: bool,
    ) {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new(if panic_before_effect {
            "recovery-post-claim-gate-panic-closes-issuer"
        } else if panic_payload_before_effect {
            "recovery-post-claim-gate-payload-closes-issuer"
        } else {
            "recovery-post-claim-gate-error-closes-issuer"
        });
        let current = Arc::new(RwLock::new(None));
        let store = JournalIndependentSnapshotReaderStore {
            inner: LinuxIdentitySnapshotStore::open(directory.open()).unwrap(),
            current,
            begin_intent_fault: None,
            begin_intent_busy_once: false,
            claim_fault: None,
            panic_after_claim: false,
            panic_on_mark_unknown: panic_unknown_mark,
            post_intent_busy: AtomicU64::new(0),
        };
        let mut issuer = IdentityService::new(
            store,
            TestIds::default(),
            TestClock::at(100),
            TestAuthorizer::default(),
            TestCleaner::default(),
            NoopIdentityEventSink,
        );
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Recovery gate fence owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("recovery-post-claim-gate-fence").unwrap(),
            )
            .unwrap();
        let context = issuer.interactive_session(&caller).unwrap();
        let identity = context.identity().unwrap();
        let old_handle = context.revalidation_handle().unwrap();
        let access = IdentityAccessContext::new(identity, &caller);
        let profile_id = identity.profile_id().clone();
        let admission = Arc::clone(&issuer.issuer.admission);
        let issuer_alive = Arc::clone(&issuer.issuer.alive);
        let opened = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let closed_at_drop = Arc::new(AtomicBool::new(false));
        let mut storage = UserStorageRootAdapter::new(RecoveryFenceDropProbeBackend {
            admission: Arc::clone(&admission),
            issuer_alive: Arc::clone(&issuer_alive),
            opened: Arc::clone(&opened),
            dropped: Arc::clone(&dropped),
            closed_at_drop: Arc::clone(&closed_at_drop),
        });
        let mut root_authorizer = NoopProfileRootAuthorizer;
        storage
            .resolve_root(&access, &profile_id, &mut root_authorizer)
            .unwrap();
        drop(context);
        assert_eq!(opened.load(Ordering::SeqCst), 1);
        assert_eq!(dropped.load(Ordering::SeqCst), 0);

        let mut gate = CountingRecoveryGate::for_binding(RecoveryBinding::for_test([0xa7; 32]));
        gate.fail_before_effect = !(panic_before_effect || panic_payload_before_effect);
        gate.panic_before_effect = panic_before_effect;
        gate.panic_payload_before_effect = panic_payload_before_effect;
        assert_eq!(
            issuer.recover_with_new_intent_and_storage(&caller, &mut gate, &mut storage),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(gate.invocations, 1);
        assert_eq!(gate.effects, 0, "the supplied recovery closure did not run");
        let record = issuer.store.recovery_journal_record().unwrap().unwrap();
        assert_eq!(
            record.phase,
            if panic_unknown_mark {
                RecoveryJournalPhase::EffectStarted
            } else {
                RecoveryJournalPhase::OutcomeUnknown
            }
        );
        assert!(issuer.store.recovery_quarantined().unwrap());
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        let mut old_effect_called = false;
        let old_effect_result = old_handle.with_live_session_effect(|| old_effect_called = true);
        let issuer_closed = admission.state.lock().unwrap().closed;
        let issuer_alive = issuer_alive.load(Ordering::Acquire);
        let root_dropped_after_close = closed_at_drop.load(Ordering::SeqCst);
        assert!(
            old_effect_result == Err(ResolutionError::Unavailable)
                && !old_effect_called
                && issuer_closed
                && !issuer_alive
                && root_dropped_after_close,
            "post-claim rejection must deny the retained identity before root cleanup: \
             effect_result={old_effect_result:?}, effect_called={old_effect_called}, \
             issuer_closed={issuer_closed}, issuer_alive={issuer_alive}, \
             root_dropped_after_close={root_dropped_after_close}"
        );
    }

    #[test]
    fn post_claim_gate_error_closes_old_issuer_before_root_cleanup() {
        assert_post_claim_gate_rejection_closes_old_issuer(false, false, false);
    }

    #[test]
    fn post_claim_gate_panic_closes_old_issuer_before_root_cleanup() {
        assert_post_claim_gate_rejection_closes_old_issuer(true, false, false);
    }

    #[test]
    fn panicking_gate_payload_and_unknown_marker_cannot_skip_root_cleanup() {
        assert_post_claim_gate_rejection_closes_old_issuer(false, true, true);
    }

    fn assert_post_intent_busy_is_unknown_and_closes_old_issuer(busy_point: u64) {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new(match busy_point {
            1 => "post-intent-journal-read-busy-is-unknown",
            2 => "post-intent-claim-busy-is-unknown",
            _ => unreachable!("test only accepts the two injected Busy points"),
        });
        let current = Arc::new(RwLock::new(None));
        let store = JournalIndependentSnapshotReaderStore {
            inner: LinuxIdentitySnapshotStore::open(directory.open()).unwrap(),
            current,
            begin_intent_fault: None,
            begin_intent_busy_once: false,
            claim_fault: None,
            panic_after_claim: false,
            panic_on_mark_unknown: false,
            post_intent_busy: AtomicU64::new(0),
        };
        let mut issuer = IdentityService::new(
            store,
            TestIds::default(),
            TestClock::at(100),
            TestAuthorizer::default(),
            TestCleaner::default(),
            NoopIdentityEventSink,
        );
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Post intent Busy owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("post-intent-busy-session").unwrap(),
            )
            .unwrap();
        let context = issuer.interactive_session(&caller).unwrap();
        let old_handle = context.revalidation_handle().unwrap();
        drop(context);
        let before_snapshot = issuer.store.load_current_state().unwrap();
        issuer
            .store
            .post_intent_busy
            .store(busy_point, Ordering::SeqCst);

        let mut gate = CountingRecoveryGate::for_binding(RecoveryBinding::for_test([0xb9; 32]));
        assert_eq!(
            issuer.recover_with_new_intent(&caller, &mut gate),
            Err(IdentityError::StoreUnavailable),
            "the convenience API cannot return its local one-shot permit"
        );
        assert_eq!(gate.invocations, 0, "Busy happened before the effect gate");
        assert_eq!(gate.effects, 0);
        assert_eq!(
            issuer
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::OutcomeUnknown,
            "the durable Intent remains explicitly reconcilable"
        );
        assert!(issuer.store.recovery_quarantined().unwrap());
        assert_eq!(issuer.store.load_current_state().unwrap(), before_snapshot);
        let mut stale_effect_called = false;
        assert_eq!(
            old_handle.with_live_session_effect(|| stale_effect_called = true),
            Err(ResolutionError::Unavailable)
        );
        assert!(!stale_effect_called);
        assert!(issuer.issuer.admission.state.lock().unwrap().closed);
        assert!(!issuer.issuer.alive.load(Ordering::Acquire));
    }

    #[test]
    fn post_intent_record_read_busy_requires_exact_reconciliation() {
        assert_post_intent_busy_is_unknown_and_closes_old_issuer(1);
    }

    #[test]
    fn post_intent_claim_busy_requires_exact_reconciliation() {
        assert_post_intent_busy_is_unknown_and_closes_old_issuer(2);
    }

    fn assert_claim_failure_closes_old_issuer(
        caller_context: &str,
        claim_fault: Option<DurableStoreFault>,
        panic_after_claim: bool,
        expected_phase: RecoveryJournalPhase,
    ) {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new(caller_context);
        let current = Arc::new(RwLock::new(None));
        let store = JournalIndependentSnapshotReaderStore {
            inner: LinuxIdentitySnapshotStore::open(directory.open()).unwrap(),
            current,
            begin_intent_fault: None,
            begin_intent_busy_once: false,
            claim_fault,
            panic_after_claim,
            panic_on_mark_unknown: false,
            post_intent_busy: AtomicU64::new(0),
        };
        let mut issuer = IdentityService::new(
            store,
            TestIds::default(),
            TestClock::at(100),
            TestAuthorizer::default(),
            TestCleaner::default(),
            NoopIdentityEventSink,
        );
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Ambiguous claim recovery").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("ambiguous-claim-after-rename").unwrap(),
            )
            .unwrap();

        let context = issuer.interactive_session(&caller).unwrap();
        let identity = context.identity().unwrap();
        let old_handle = context.revalidation_handle().unwrap();
        let access = IdentityAccessContext::new(identity, &caller);
        let profile_id = identity.profile_id().clone();
        let admission = Arc::clone(&issuer.issuer.admission);
        let issuer_alive = Arc::clone(&issuer.issuer.alive);
        let opened = Arc::new(AtomicU64::new(0));
        let dropped = Arc::new(AtomicU64::new(0));
        let closed_at_drop = Arc::new(AtomicBool::new(false));
        let mut storage = UserStorageRootAdapter::new(RecoveryFenceDropProbeBackend {
            admission: Arc::clone(&admission),
            issuer_alive: Arc::clone(&issuer_alive),
            opened: Arc::clone(&opened),
            dropped: Arc::clone(&dropped),
            closed_at_drop: Arc::clone(&closed_at_drop),
        });
        let mut root_authorizer = NoopProfileRootAuthorizer;
        storage
            .resolve_root(&access, &profile_id, &mut root_authorizer)
            .unwrap();
        drop(context);

        let mut gate = CountingRecoveryGate::for_binding(RecoveryBinding::for_test([0xa8; 32]));
        assert_eq!(
            issuer.recover_with_new_intent_and_storage(&caller, &mut gate, &mut storage),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(gate.invocations, 0);
        assert_eq!(gate.effects, 0);
        assert_eq!(opened.load(Ordering::SeqCst), 1);
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
        assert!(closed_at_drop.load(Ordering::SeqCst));
        assert!(issuer.issuer.admission.state.lock().unwrap().closed);
        assert!(!issuer.issuer.alive.load(Ordering::Acquire));
        let mut old_effect_called = false;
        assert_eq!(
            old_handle.with_live_session_effect(|| old_effect_called = true),
            Err(ResolutionError::Unavailable)
        );
        assert!(!old_effect_called);

        drop(issuer);
        let reopened = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let record = reopened.recovery_journal_record().unwrap().unwrap();
        assert_eq!(record.phase, expected_phase);
        assert!(reopened.recovery_quarantined().unwrap());
    }

    #[test]
    fn ambiguous_claim_after_rename_closes_old_issuer_before_root_cleanup() {
        assert_claim_failure_closes_old_issuer(
            "recovery-claim-rename-failure-closes-issuer",
            Some(DurableStoreFault::AfterRename),
            false,
            RecoveryJournalPhase::EffectStarted,
        );
    }

    #[test]
    fn panic_after_durable_claim_closes_old_issuer_before_root_cleanup() {
        assert_claim_failure_closes_old_issuer(
            "recovery-claim-panic-closes-issuer",
            None,
            true,
            RecoveryJournalPhase::OutcomeUnknown,
        );
    }

    #[test]
    fn post_claim_session_busy_is_unknown_and_never_retryable_busy() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("recovery-post-claim-session-busy");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Post-claim owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        let session_id = issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("post-claim-session-busy").unwrap(),
            )
            .unwrap();
        let old_handle = issuer
            .interactive_session(&caller)
            .unwrap()
            .revalidation_handle()
            .unwrap();
        let held_session = Arc::clone(issuer.leases.get(&session_id).unwrap());
        let before = issuer.store.load_current_state().unwrap();
        let binding = RecoveryBinding::for_test([0x8a; 32]);
        let mut gate = CountingRecoveryGate::for_binding(binding);
        gate.block_session_writer = Some(held_session);

        assert_eq!(
            issuer.recover_with_new_intent(&caller, &mut gate),
            Err(IdentityError::StoreUnavailable),
            "a Busy after durable claim is an unknown outcome, not safe retry advice"
        );
        assert_eq!(gate.invocations, 1);
        assert_eq!(gate.effects, 1);
        assert_eq!(gate.successful_callbacks, 0);
        assert_eq!(
            issuer
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::OutcomeUnknown
        );
        assert!(issuer.store.recovery_quarantined().unwrap());
        assert_eq!(issuer.store.load_current_state().unwrap(), before);
        assert_eq!(
            old_handle.check_live(),
            Err(ResolutionError::Unavailable),
            "the unresolved journal fences the old handle while the Session is still Active"
        );
    }

    #[test]
    fn poisoned_durable_commit_denies_old_authority_and_recovers_after_restart() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("durable-poisoned-commit");
        let clock = TestClock::at(100);
        let mut issuer = service(&directory, clock.clone());
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Durable owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        let session_id = issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("durable-poisoned-commit").unwrap(),
            )
            .unwrap();
        let context = issuer.interactive_session(&caller).unwrap();
        let old_handle = context.revalidation_handle().unwrap();
        assert_eq!(
            old_handle.try_live_snapshot().unwrap().status(),
            LiveStatus::Live
        );

        issuer
            .store
            .set_fault_for_test(DurableStoreFault::AfterHighWater);
        assert_eq!(
            issuer.create_profile(
                &caller,
                user_id,
                ProfileMetadata::new("Work", Some("en-US".into())).unwrap(),
            ),
            Err(IdentityError::StoreUnavailable)
        );
        assert!(old_handle.check_live().is_err());
        assert_eq!(
            issuer.create_profile(
                &caller,
                user_id,
                ProfileMetadata::new("Work", Some("en-US".into())).unwrap(),
            ),
            Err(IdentityError::StoreUnavailable),
            "a same-instance write retry remains poisoned"
        );
        assert_eq!(
            issuer.recover(&caller),
            Err(IdentityError::StoreUnavailable),
            "the poisoned issuer cannot repair or retry from cached authority"
        );
        assert!(old_handle.check_live().is_err());
        drop(issuer);

        let mut restarted = service(&directory, clock.clone());
        let report = restarted.recover(&caller).unwrap();
        assert_eq!(report.source, RecoverySource::LastKnownGood);
        assert_eq!(report.interrupted_sessions_ended, 1);
        assert!(restarted.resolve_current_identity(&caller).is_err());
        assert!(matches!(
            restarted
                .snapshot()
                .sessions
                .iter()
                .find(|session| session.session_id == session_id)
                .unwrap()
                .lifecycle,
            SessionLifecycle::Ended { .. }
        ));
        assert!(old_handle.check_live().is_err());
    }

    #[test]
    fn initial_recovery_intent_is_durable_before_first_snapshot_effect() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("initial-recovery-intent");
        let clock = TestClock::at(100);
        let mut issuer = service(&directory, clock.clone());
        let binding = RecoveryBinding::for_test([0x61; 32]);
        let operation_permit = issuer.store.begin_recovery_intent(binding.clone()).unwrap();
        let operation_id = operation_permit.operation_id();

        assert_eq!(
            issuer.recover(&caller),
            Err(IdentityError::StoreUnavailable),
            "a staged intent must block an initial recovery effect"
        );
        assert_eq!(
            issuer.store.load_current_state().unwrap(),
            SnapshotStoreState {
                current: None,
                high_water_version: None,
            },
            "the denied attempt must not initialize the durable snapshot"
        );
        drop(issuer);

        let mut restarted = service(&directory, clock.clone());
        assert_eq!(
            restarted.recover(&caller),
            Err(IdentityError::StoreUnavailable),
            "restart must preserve the unresolved initial intent"
        );
        assert_eq!(
            restarted
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::Intent
        );

        let _receipt = restarted
            .reconcile_recovery(
                &caller,
                RecoveryReconciliation::for_test(operation_id, binding, [0x62; 32]),
            )
            .unwrap();
        assert!(restarted.store.recovery_quarantined().unwrap());
        assert_eq!(
            restarted.recover(&caller),
            Err(IdentityError::StoreUnavailable),
            "same-instance reconciliation cannot resume the issuer that held old handles"
        );
        drop(restarted);

        let mut resumed = service(&directory, clock);
        assert!(!resumed.store.recovery_quarantined().unwrap());
        let report = resumed.recover(&caller).unwrap();
        assert_eq!(report.source, RecoverySource::Empty);
        assert_eq!(report.interrupted_sessions_ended, 0);
        assert!(resumed
            .store
            .load_current_state()
            .unwrap()
            .current
            .is_some());
    }

    #[test]
    fn exact_reconciliation_keeps_same_service_and_old_handles_fenced_until_restart() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("reconcile-live-service-fence");
        let clock = TestClock::at(100);
        let mut issuer = service(&directory, clock.clone());
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Reconciled owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        let session_id = issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("reconcile-live-service-fence").unwrap(),
            )
            .unwrap();
        let old_handle = issuer
            .interactive_session(&caller)
            .unwrap()
            .revalidation_handle()
            .unwrap();
        assert_eq!(old_handle.check_live(), Ok(()));

        let before_state = issuer.store.load_current_state().unwrap();
        let before_lkg = issuer.store.load_last_known_good().unwrap();
        let binding = RecoveryBinding::for_test([0x33; 32]);
        let permit = issuer.store.begin_recovery_intent(binding.clone()).unwrap();
        issuer
            .store
            .mark_recovery_outcome_unknown(permit.operation_id(), &binding)
            .unwrap();
        let first_unknown = issuer.store.recovery_journal_record().unwrap();
        let first_unknown_state = issuer.store.load_current_state().unwrap();
        issuer
            .store
            .mark_recovery_outcome_unknown(permit.operation_id(), &binding)
            .unwrap();
        assert_eq!(
            issuer.store.recovery_journal_record().unwrap(),
            first_unknown,
            "repeating quarantine for the exact operation is idempotent"
        );
        assert_eq!(
            issuer.store.load_current_state().unwrap(),
            first_unknown_state
        );
        assert_eq!(
            issuer.store.mark_recovery_outcome_unknown(
                RecoveryOperationId::for_test(permit.operation_id().get() + 1),
                &binding,
            ),
            Err(RecoveryJournalError::NotCurrentOperation),
            "a different operation cannot reuse the exact quarantine record"
        );
        assert_eq!(
            issuer.store.recovery_journal_record().unwrap(),
            first_unknown
        );
        assert_eq!(
            old_handle.check_live(),
            Err(ResolutionError::Unavailable),
            "an unknown outcome blocks the pre-recovery Session"
        );

        let receipt = issuer
            .reconcile_recovery(
                &caller,
                RecoveryReconciliation::for_test(permit.operation_id(), binding, [0x43; 32]),
            )
            .unwrap();
        issuer.consume_reconciliation_receipt(receipt).unwrap();
        assert!(
            !issuer.issuer.alive.load(Ordering::Acquire),
            "consuming the exact receipt leaves the old issuer closed"
        );
        assert_eq!(
            issuer
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::Reconciled {
                outcome_digest: [0x43; 32]
            }
        );
        assert!(
            issuer.store.recovery_quarantined().unwrap(),
            "reconciliation is terminal on disk but this issuer remains fenced"
        );
        assert_eq!(
            old_handle.check_live(),
            Err(ResolutionError::Unavailable),
            "terminal bookkeeping cannot reactivate a live old handle"
        );
        let mut effect_called = false;
        assert_eq!(
            old_handle.with_live_session_effect(|| effect_called = true),
            Err(ResolutionError::Unavailable)
        );
        assert!(!effect_called);
        assert_eq!(issuer.store.load_current_state().unwrap(), before_state);
        assert_eq!(issuer.store.load_last_known_good().unwrap(), before_lkg);
        assert_eq!(
            issuer.create_local_user(
                &caller,
                LocalUserMetadata::new("Blocked after reconcile").unwrap(),
                ProfileMetadata::new("Work", Some("en-US".into())).unwrap(),
            ),
            Err(IdentityError::StoreUnavailable)
        );
        drop(issuer);

        let mut restarted = service(&directory, clock.clone());
        assert!(!restarted.store.recovery_quarantined().unwrap());
        let report = restarted.recover(&caller).unwrap();
        assert_eq!(report.interrupted_sessions_ended, 1);
        assert!(restarted.snapshot.sessions.iter().any(|session| {
            session.session_id == session_id
                && matches!(
                    &session.lifecycle,
                    SessionLifecycle::Ended {
                        reason: SessionEndReason::SystemRestart,
                        ..
                    }
                )
        }));
        assert_eq!(old_handle.check_live(), Err(ResolutionError::Unavailable));
    }

    #[test]
    fn reopened_store_reissues_only_exact_terminal_receipt_after_fencing_current_issuer() {
        match run_ignored_probe(
            "reopened_store_reissues_only_exact_terminal_receipt_probe",
            Duration::from_secs(90),
        ) {
            ProbeOutcome::AfterReadyExit(status) if status.success() => {}
            outcome => panic!("reopened exact receipt probe did not complete: {outcome:?}"),
        }
    }

    #[test]
    #[ignore = "spawned only by the bounded parent watchdog"]
    fn reopened_store_reissues_only_exact_terminal_receipt_probe() {
        reopened_store_reissues_only_exact_terminal_receipt_child();
    }

    fn reopened_store_reissues_only_exact_terminal_receipt_child() {
        if std::env::var_os(PROBE_READY_ENV).is_none() {
            return;
        }
        mark_probe_ready();
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("reconcile-terminal-receipt-readback");
        let clock = TestClock::at(100);
        let mut original = service(&directory, clock.clone());
        original.recover(&caller).unwrap();
        let old_issuer_alive = Arc::clone(&original.issuer.alive);

        let binding = RecoveryBinding::for_test([0x89; 32]);
        let permit = original
            .store
            .begin_recovery_intent(binding.clone())
            .unwrap();
        let reconciliation =
            RecoveryReconciliation::for_test(permit.operation_id(), binding.clone(), [0x8a; 32]);
        original
            .store
            .mark_recovery_outcome_unknown(permit.operation_id(), &binding)
            .unwrap();
        let lost_response = original
            .reconcile_recovery(&caller, reconciliation.clone())
            .unwrap();
        assert_eq!(lost_response.operation_id(), permit.operation_id().get());
        assert!(!old_issuer_alive.load(Ordering::Acquire));
        drop(lost_response); // Simulate a crash after durable terminal commit but before delivery.
        let terminal_record = original.store.recovery_journal_record().unwrap().unwrap();
        assert_eq!(
            terminal_record.phase,
            RecoveryJournalPhase::Reconciled {
                outcome_digest: [0x8a; 32]
            }
        );
        drop(original);

        let mut reopened = service(&directory, clock);
        assert_eq!(
            reopened.store.recovery_journal_record().unwrap(),
            Some(terminal_record.clone())
        );
        let before_mismatch_state = reopened.store.load_current_state().unwrap();
        let before_mismatch_lkg = reopened.store.load_last_known_good().unwrap();
        let reopened_issuer_alive = Arc::clone(&reopened.issuer.alive);
        assert!(reopened_issuer_alive.load(Ordering::Acquire));
        let before_reopened_admission = {
            let state = reopened.issuer.admission.state.lock().unwrap();
            (state.closed, state.recovering, state.active)
        };
        assert_eq!(before_reopened_admission, (false, false, 0));

        let wrong_operation = RecoveryReconciliation::for_test(
            RecoveryOperationId::for_test(permit.operation_id().get() + 1),
            binding.clone(),
            [0x8a; 32],
        );
        assert_eq!(
            reopened.reconcile_recovery(&caller, wrong_operation),
            Err(IdentityError::StoreUnavailable),
            "a different operation cannot read back this terminal receipt"
        );
        let wrong_binding = RecoveryReconciliation::for_test(
            permit.operation_id(),
            RecoveryBinding::for_test([0x8b; 32]),
            [0x8a; 32],
        );
        assert_eq!(
            reopened.reconcile_recovery(&caller, wrong_binding),
            Err(IdentityError::StoreUnavailable),
            "a different binding cannot read back this terminal receipt"
        );
        let wrong_outcome =
            RecoveryReconciliation::for_test(permit.operation_id(), binding.clone(), [0x8c; 32]);
        assert_eq!(
            reopened.reconcile_recovery(&caller, wrong_outcome),
            Err(IdentityError::StoreUnavailable),
            "a different outcome cannot read back this terminal receipt"
        );
        assert_eq!(
            reopened.store.recovery_journal_record().unwrap(),
            Some(terminal_record.clone())
        );
        assert_eq!(
            reopened.store.load_current_state().unwrap(),
            before_mismatch_state
        );
        assert_eq!(
            reopened.store.load_last_known_good().unwrap(),
            before_mismatch_lkg
        );

        // Model a newly opened service that has recovered and issued a fresh
        // Session before it reads back the prior operation's terminal receipt.
        // Exact terminal readback must drain this issuer and fence its store.
        reopened.recover(&caller).unwrap();
        let (reopened_user, reopened_profile) = reopened
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Reopened receipt owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        reopened
            .login_local(
                &caller,
                reopened_user,
                reopened_profile,
                None,
                IdempotencyKey::new("reopened-terminal-receipt-live-session").unwrap(),
            )
            .unwrap();
        let reopened_handle = reopened
            .interactive_session(&caller)
            .unwrap()
            .revalidation_handle()
            .unwrap();
        assert_eq!(reopened_handle.check_live(), Ok(()));
        let before_receipt_state = reopened.store.load_current_state().unwrap();
        let before_receipt_lkg = reopened.store.load_last_known_good().unwrap();
        let before_receipt_snapshot = reopened.snapshot.clone();

        let (reader_entered_tx, reader_entered_rx) = mpsc::sync_channel(0);
        let (reader_release_tx, reader_release_rx) = mpsc::channel();
        reopened
            .store
            .pause_reader_after_fence_check_for_test(reader_entered_tx, reader_release_rx);
        let raw_reader = reopened.store.current_snapshot_reader().unwrap();
        let (reader_result_tx, reader_result_rx) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            reader_result_tx.send(raw_reader()).unwrap();
        });
        let mut reader_worker = PausedRawSnapshotReader {
            entered_rx: Some(reader_entered_rx),
            start_tx: None,
            release_tx: Some(reader_release_tx),
            result_rx: reader_result_rx,
            worker: Some(reader),
        };
        reader_worker
            .wait_until_entered(TEST_CHANNEL_WATCHDOG)
            .expect("reopened raw reader must register before exact terminal retry");

        assert_eq!(reopened.issuer.admission.state.lock().unwrap().active, 0);
        assert!(reopened_issuer_alive.load(Ordering::Acquire));
        assert_eq!(
            reopened.store.reconciliation_fence_state_for_test(),
            (false, false, 1)
        );
        assert_eq!(
            reopened.reconcile_recovery(&caller, reconciliation.clone()),
            Err(IdentityError::SessionLeaseBusy),
            "raw-reader contention must precede irreversible issuer closure"
        );
        assert_eq!(
            reopened.store.reconciliation_fence_state_for_test(),
            (false, false, 1),
            "a busy exact terminal readback must leave no writer reservation"
        );
        assert_eq!(
            {
                let state = reopened.issuer.admission.state.lock().unwrap();
                (state.closed, state.recovering, state.active)
            },
            (false, false, 0),
            "busy exact terminal readback must restore admission for retry"
        );
        assert!(reopened_issuer_alive.load(Ordering::Acquire));

        assert!(reader_worker
            .release_and_join()
            .expect("terminal readback must not fence a raw reader before retry")
            .is_some());
        assert_eq!(
            reopened.store.reconciliation_fence_state_for_test(),
            (false, false, 0)
        );
        assert!(reopened_issuer_alive.load(Ordering::Acquire));
        assert_eq!(reopened_handle.check_live(), Ok(()));
        assert_eq!(
            reopened.store.recovery_journal_record().unwrap(),
            Some(terminal_record.clone())
        );
        assert_eq!(
            reopened.store.load_current_state().unwrap(),
            before_receipt_state
        );
        assert_eq!(
            reopened.store.load_last_known_good().unwrap(),
            before_receipt_lkg
        );
        assert_eq!(reopened.snapshot, before_receipt_snapshot);

        let receipt = reopened
            .reconcile_recovery(&caller, reconciliation)
            .expect("exact durable Reconciled readback fences the reopened issuer and store");
        assert_eq!(receipt.operation_id(), permit.operation_id().get());
        assert_eq!(receipt.binding_digest(), binding.digest_bytes());
        assert_eq!(receipt.outcome_digest(), [0x8a; 32]);
        #[cfg(feature = "system-service-recovery-contract")]
        {
            let shared_receipt:
                &dyn nagi_system_service_recovery_contract::C1RecoveryTerminalReceipt = &receipt;
            assert_eq!(shared_receipt.operation_id(), receipt.operation_id());
            assert_eq!(shared_receipt.binding_digest(), receipt.binding_digest());
            assert_eq!(shared_receipt.outcome_digest(), receipt.outcome_digest());
        }
        reopened.consume_reconciliation_receipt(receipt).unwrap();

        assert!(!old_issuer_alive.load(Ordering::Acquire));
        assert!(!reopened_issuer_alive.load(Ordering::Acquire));
        let after_reopened_admission = {
            let state = reopened.issuer.admission.state.lock().unwrap();
            (state.closed, state.recovering, state.active)
        };
        assert_eq!(after_reopened_admission, (true, false, 0));
        assert_ne!(after_reopened_admission, before_reopened_admission);
        assert_eq!(
            reopened_handle.check_live(),
            Err(ResolutionError::Unavailable)
        );
        let reopened_effect_calls = Arc::new(AtomicU64::new(0));
        let reopened_effect_calls_in_callback = Arc::clone(&reopened_effect_calls);
        assert_eq!(
            reopened_handle.with_live_session_effect(|| {
                reopened_effect_calls_in_callback.fetch_add(1, Ordering::SeqCst);
            }),
            Err(ResolutionError::Unavailable)
        );
        assert_eq!(reopened_effect_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            reopened.store.reconciliation_fence_state_for_test(),
            (true, true, 0)
        );
        assert_eq!(
            reopened.store.current_snapshot_reader().unwrap()(),
            Err(SnapshotStoreError::Unavailable),
            "receipt readback leaves the reopened store fenced against raw reads"
        );
        assert_eq!(
            reopened.store.recovery_journal_record().unwrap(),
            Some(terminal_record)
        );
        assert_eq!(
            reopened.store.load_current_state().unwrap(),
            before_receipt_state
        );
        assert_eq!(
            reopened.store.load_last_known_good().unwrap(),
            before_receipt_lkg
        );
        assert_eq!(reopened.snapshot, before_receipt_snapshot);
        // This C1 consumer only re-reads its journal; no A/C3 release hook is
        // available or invoked by receipt readback/consumption.
    }

    fn paused_raw_snapshot_reader_entry_timeout_probe_child() {
        if std::env::var_os(PROBE_READY_ENV).is_none() {
            return;
        }

        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (start_tx, start_rx) = mpsc::channel();
        let (release_tx, _release_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let entry_send_failed = Arc::new(AtomicBool::new(false));
        let entry_send_failed_in_worker = Arc::clone(&entry_send_failed);
        let worker = thread::spawn(move || {
            start_rx
                .recv()
                .expect("drop cleanup must release the test worker");
            let disconnected = entered_tx.send(()).is_err();
            entry_send_failed_in_worker.store(disconnected, Ordering::Release);
            result_tx
                .send(if disconnected {
                    Err(SnapshotStoreError::Unavailable)
                } else {
                    Ok(None)
                })
                .unwrap();
        });
        let reader_worker = PausedRawSnapshotReader {
            entered_rx: Some(entered_rx),
            start_tx: Some(start_tx),
            release_tx: Some(release_tx),
            result_rx,
            worker: Some(worker),
        };
        mark_probe_ready();
        assert!(matches!(
            reader_worker.wait_until_entered(Duration::from_millis(10)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(reader_worker);
        assert!(
            entry_send_failed.load(Ordering::Acquire),
            "entry receiver must disconnect before cleanup joins the worker"
        );
    }

    #[test]
    #[ignore = "spawned only by the bounded parent watchdog"]
    fn paused_raw_snapshot_reader_entry_timeout_probe() {
        paused_raw_snapshot_reader_entry_timeout_probe_child();
    }

    #[test]
    fn paused_raw_snapshot_reader_entry_timeout_disconnects_and_joins_worker() {
        match run_ignored_probe(
            "paused_raw_snapshot_reader_entry_timeout_probe",
            Duration::from_secs(3),
        ) {
            ProbeOutcome::AfterReadyExit(status) if status.success() => {}
            outcome => panic!("paused raw-reader cleanup did not complete safely: {outcome:?}"),
        }
    }

    #[test]
    fn reconciliation_after_rename_failure_keeps_same_issuer_and_reader_fenced() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("reconcile-after-rename-failure");
        let clock = TestClock::at(100);
        let mut issuer = service(&directory, clock.clone());
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Rename failure owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("reconcile-after-rename-failure").unwrap(),
            )
            .unwrap();
        let old_handle = issuer
            .interactive_session(&caller)
            .unwrap()
            .revalidation_handle()
            .unwrap();
        assert_eq!(old_handle.check_live(), Ok(()));
        let reader = issuer.store.current_snapshot_reader().unwrap();

        let binding = RecoveryBinding::for_test([0x71; 32]);
        let permit = issuer.store.begin_recovery_intent(binding.clone()).unwrap();
        issuer
            .store
            .mark_recovery_outcome_unknown(permit.operation_id(), &binding)
            .unwrap();
        issuer
            .store
            .set_fault_for_test(DurableStoreFault::AfterRename);
        assert_eq!(
            issuer.reconcile_recovery(
                &caller,
                RecoveryReconciliation::for_test(permit.operation_id(), binding, [0x72; 32],),
            ),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(
            issuer.store.current_snapshot_reader().unwrap()(),
            Err(SnapshotStoreError::Unavailable),
            "the store reader stays fenced after terminal rename and a pre-sync failure"
        );
        assert!(
            issuer.issuer.admission.state.lock().unwrap().closed,
            "issuer admission remains closed after uncertain terminal persistence"
        );
        assert!(!issuer.issuer.alive.load(Ordering::Acquire));
        assert_eq!(reader(), Err(SnapshotStoreError::Unavailable));
        assert_eq!(old_handle.check_live(), Err(ResolutionError::Unavailable));
        let effect_calls = Arc::new(AtomicU64::new(0));
        let effect_calls_in_callback = Arc::clone(&effect_calls);
        assert_eq!(
            old_handle.with_live_session_effect(|| {
                effect_calls_in_callback.fetch_add(1, Ordering::SeqCst);
            }),
            Err(ResolutionError::Unavailable)
        );
        assert_eq!(effect_calls.load(Ordering::SeqCst), 0);

        drop(issuer);
        let mut restarted = service(&directory, clock);
        assert!(matches!(
            restarted
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::Reconciled { .. }
        ));
        let report = restarted.recover(&caller).unwrap();
        assert_eq!(report.interrupted_sessions_ended, 1);
        assert_eq!(old_handle.check_live(), Err(ResolutionError::Unavailable));
    }

    #[test]
    fn reconciliation_cannot_publish_while_old_reader_is_paused_after_fence_check() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("reconcile-reader-publication-race");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Reader race owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("reconcile-reader-publication-race").unwrap(),
            )
            .unwrap();
        let old_handle = Arc::new(
            issuer
                .interactive_session(&caller)
                .unwrap()
                .revalidation_handle()
                .unwrap(),
        );
        let binding = RecoveryBinding::for_test([0x73; 32]);
        let permit = issuer.store.begin_recovery_intent(binding.clone()).unwrap();
        issuer
            .store
            .mark_recovery_outcome_unknown(permit.operation_id(), &binding)
            .unwrap();

        let (reader_entered_tx, reader_entered_rx) = mpsc::sync_channel(0);
        let (reader_release_tx, reader_release_rx) = mpsc::sync_channel(0);
        issuer
            .store
            .pause_reader_after_fence_check_for_test(reader_entered_tx, reader_release_rx);
        let effect_calls = Arc::new(AtomicU64::new(0));
        let effect_calls_in_callback = Arc::clone(&effect_calls);
        let handle_in_thread = Arc::clone(&old_handle);
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let reader_thread = thread::spawn(move || {
            let result = handle_in_thread.with_live_session_effect(|| {
                effect_calls_in_callback.fetch_add(1, Ordering::SeqCst);
            });
            result_tx.send(result).unwrap();
        });
        reader_entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("reader must pause after observing the open fence");

        assert!(
            !issuer.store.reconciliation_writer_available_for_test(),
            "the paused reader registration prevents terminal publication"
        );
        assert_eq!(
            issuer.reconcile_recovery(
                &caller,
                RecoveryReconciliation::for_test(
                    permit.operation_id(),
                    binding.clone(),
                    [0x74; 32],
                ),
            ),
            Err(IdentityError::SessionLeaseBusy),
            "issuer admission rejects reconciliation while an old callback is active"
        );
        assert_eq!(
            issuer
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::OutcomeUnknown
        );
        assert_eq!(effect_calls.load(Ordering::SeqCst), 0);

        reader_release_tx.send(()).unwrap();
        assert_eq!(
            result_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("reader callback must finish within the watchdog bound"),
            Err(ResolutionError::Unavailable)
        );
        reader_thread.join().unwrap();
        assert_eq!(effect_calls.load(Ordering::SeqCst), 0);
        assert_eq!(old_handle.check_live(), Err(ResolutionError::Unavailable));

        let _receipt = issuer
            .reconcile_recovery(
                &caller,
                RecoveryReconciliation::for_test(permit.operation_id(), binding, [0x74; 32]),
            )
            .unwrap();
        assert_eq!(old_handle.check_live(), Err(ResolutionError::Unavailable));
        assert_eq!(effect_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn raw_store_reader_contention_is_retryable_before_issuer_closure() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("reconcile-raw-store-reader-retry");
        let mut issuer = service(&directory, TestClock::at(100));
        issuer.recover(&caller).unwrap();
        let binding = RecoveryBinding::for_test([0x75; 32]);
        let permit = issuer.store.begin_recovery_intent(binding.clone()).unwrap();
        issuer
            .store
            .mark_recovery_outcome_unknown(permit.operation_id(), &binding)
            .unwrap();
        let reconciliation =
            RecoveryReconciliation::for_test(permit.operation_id(), binding, [0x76; 32]);
        let before_journal = issuer.store.recovery_journal_record().unwrap();
        let before_store_state = issuer.store.load_current_state().unwrap();
        let before_lkg = issuer.store.load_last_known_good().unwrap();
        let before_snapshot = issuer.snapshot.clone();

        let (reader_entered_tx, reader_entered_rx) = mpsc::sync_channel(0);
        let (reader_release_tx, reader_release_rx) = mpsc::sync_channel(0);
        issuer
            .store
            .pause_reader_after_fence_check_for_test(reader_entered_tx, reader_release_rx);
        let raw_reader = issuer.store.current_snapshot_reader().unwrap();
        let (reader_result_tx, reader_result_rx) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            reader_result_tx.send(raw_reader()).unwrap();
        });
        reader_entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("raw current-store reader must hold its fence registration");

        assert_eq!(
            issuer.issuer.admission.state.lock().unwrap().active,
            0,
            "a raw observation is not an admitted Session callback"
        );
        assert!(issuer.issuer.alive.load(Ordering::Acquire));
        assert_eq!(
            issuer.store.reconciliation_fence_state_for_test(),
            (false, false, 1)
        );
        assert!(!issuer.store.reconciliation_writer_available_for_test());
        assert_eq!(
            issuer.reconcile_recovery(&caller, reconciliation.clone()),
            Err(IdentityError::SessionLeaseBusy),
            "reader contention before terminal writing must remain retryable"
        );
        assert_eq!(issuer.issuer.admission.state.lock().unwrap().active, 0);
        assert!(issuer.issuer.alive.load(Ordering::Acquire));
        assert_eq!(
            issuer.store.reconciliation_fence_state_for_test(),
            (false, false, 1),
            "busy prepare must not close or retain a writer reservation"
        );
        assert_eq!(
            issuer.store.recovery_journal_record().unwrap(),
            before_journal
        );
        assert_eq!(
            issuer.store.load_current_state().unwrap(),
            before_store_state
        );
        assert_eq!(issuer.store.load_last_known_good().unwrap(), before_lkg);
        assert_eq!(issuer.snapshot, before_snapshot);

        reader_release_tx.send(()).unwrap();
        assert_eq!(
            reader_result_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("raw observation must leave within the watchdog bound"),
            Err(SnapshotStoreError::Unavailable),
            "a raw reader released after observing an unresolved journal must still deny authority"
        );
        reader.join().unwrap();
        assert_eq!(
            issuer.store.reconciliation_fence_state_for_test(),
            (false, false, 0)
        );
        assert_eq!(
            issuer.store.recovery_journal_record().unwrap(),
            before_journal,
            "a denied raw read does not mutate the exact pending operation"
        );
        assert_eq!(
            issuer.store.load_current_state().unwrap(),
            before_store_state
        );
        assert_eq!(issuer.store.load_last_known_good().unwrap(), before_lkg);
        assert_eq!(issuer.snapshot, before_snapshot);

        let receipt = issuer
            .reconcile_recovery(&caller, reconciliation)
            .expect("same exact operation retries after raw reader release");
        assert_eq!(receipt.operation_id(), permit.operation_id().get());
        assert_eq!(receipt.outcome_digest(), [0x76; 32]);
        issuer.consume_reconciliation_receipt(receipt).unwrap();
        assert!(
            !issuer.issuer.alive.load(Ordering::Acquire),
            "terminal receipt consumption leaves the original issuer closed"
        );
        assert_eq!(
            issuer
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::Reconciled {
                outcome_digest: [0x76; 32]
            }
        );
        assert_eq!(
            issuer.store.reconciliation_fence_state_for_test(),
            (true, true, 0),
            "successful terminal publication permanently closes this service's store fence"
        );
        assert!(!issuer.issuer.alive.load(Ordering::Acquire));
    }

    #[test]
    fn recovery_quarantine_survives_restart_and_requires_exact_reconciliation() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("durable-recovery-quarantine");
        let clock = TestClock::at(100);
        let mut issuer = service(&directory, clock.clone());
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Quarantined owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("durable-recovery-quarantine").unwrap(),
            )
            .unwrap();
        let context = issuer.interactive_session(&caller).unwrap();
        let old_handle = context.revalidation_handle().unwrap();
        let binding = RecoveryBinding::for_test([0x31; 32]);
        let operation_permit = issuer.store.begin_recovery_intent(binding.clone()).unwrap();
        let operation_id = operation_permit.operation_id();

        assert_eq!(
            issuer.recover(&caller),
            Err(IdentityError::StoreUnavailable),
            "a fresh recovery intent must fence effect retry"
        );
        assert_eq!(
            issuer.create_local_user(
                &caller,
                LocalUserMetadata::new("Blocked while quarantined").unwrap(),
                ProfileMetadata::new("Work", Some("en-US".into())).unwrap(),
            ),
            Err(IdentityError::StoreUnavailable)
        );
        assert_eq!(
            old_handle.check_live(),
            Err(ResolutionError::Unavailable),
            "already issued handles must fail closed while the durable intent is pending"
        );

        issuer
            .store
            .mark_recovery_outcome_unknown(operation_id, &binding)
            .unwrap();
        assert_eq!(
            issuer
                .store
                .recovery_journal_record()
                .unwrap()
                .unwrap()
                .phase,
            RecoveryJournalPhase::OutcomeUnknown
        );
        drop(issuer);

        let mut restarted = service(&directory, clock.clone());
        assert_eq!(
            restarted.recover(&caller),
            Err(IdentityError::StoreUnavailable),
            "a process restart must not reissue an unresolved effect permit"
        );
        let changed_generation = RecoveryBinding::for_test([0x32; 32]);
        assert_eq!(
            restarted
                .store
                .begin_recovery_intent(changed_generation.clone()),
            Err(RecoveryJournalError::Quarantined),
            "a different generation cannot implicitly clear an older intent"
        );
        assert_eq!(
            restarted.reconcile_recovery(
                &caller,
                RecoveryReconciliation::for_test(
                    operation_id,
                    changed_generation.clone(),
                    [0x41; 32],
                ),
            ),
            Err(IdentityError::StoreUnavailable)
        );
        assert!(restarted.store.recovery_quarantined().unwrap());

        let _receipt = restarted
            .reconcile_recovery(
                &caller,
                RecoveryReconciliation::for_test(operation_id, binding.clone(), [0x41; 32]),
            )
            .unwrap();
        assert!(
            restarted.store.recovery_quarantined().unwrap(),
            "only the exact operation is terminal, but the live issuer remains fenced"
        );
        assert_eq!(
            restarted.reconcile_recovery(
                &caller,
                RecoveryReconciliation::for_test(operation_id, binding, [0x42; 32],),
            ),
            Err(IdentityError::StoreUnavailable)
        );

        assert_eq!(
            restarted.recover(&caller),
            Err(IdentityError::StoreUnavailable),
            "the exact terminal record does not reactivate the issuer that owns old handles"
        );
        drop(restarted);

        let mut resumed = service(&directory, clock);
        assert!(!resumed.store.recovery_quarantined().unwrap());
        let report = resumed.recover(&caller).unwrap();
        assert_eq!(report.interrupted_sessions_ended, 1);
        assert!(old_handle.check_live().is_err());
        let next_operation_id = resumed
            .store
            .begin_recovery_intent(changed_generation)
            .unwrap();
        assert_eq!(
            next_operation_id.operation_id().get(),
            operation_id.get() + 1
        );
    }

    #[test]
    fn live_session_effect_releases_gate_before_logout_and_stale_handle_denies_effect() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("durable-effect-gate");
        let clock = TestClock::at(100);
        let mut issuer = IdentityService::new(
            LinuxIdentitySnapshotStore::open(directory.open()).unwrap(),
            TestIds::default(),
            clock,
            TestAuthorizer::default(),
            TestCleaner::default(),
            NoopIdentityEventSink,
        );
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Effect owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        let session_id = issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("durable-effect-gate").unwrap(),
            )
            .unwrap();
        let context = issuer.interactive_session(&caller).unwrap();
        let handle = context.revalidation_handle().unwrap();
        drop(context);
        assert_eq!(handle.with_live_session_effect(|| ()), Ok(()));
        assert_eq!(
            issuer.end_session(&caller, &session_id),
            Ok(EndSessionResult::Ended)
        );
        assert!(handle.check_live().is_err());

        let mut stale_effect_invoked = false;
        assert!(handle
            .with_live_session_effect(|| stale_effect_invoked = true)
            .is_err());
        assert!(!stale_effect_invoked);
    }

    #[test]
    fn issuer_shutdown_drains_multiple_live_effects_before_durable_recovery() {
        let directory = DurableTestDirectory::new();
        let callers = [
            TestCaller::new("service-drop-effect-owner-a"),
            TestCaller::new("service-drop-effect-owner-b"),
        ];
        let clock = TestClock::at(100);
        let mut issuer = service(&directory, clock.clone());
        issuer.recover(&callers[0]).unwrap();
        let mut handles = Vec::new();
        let mut session_ids = Vec::new();
        for (index, caller) in callers.iter().enumerate() {
            let (user_id, profile_id) = issuer
                .create_local_user(
                    caller,
                    LocalUserMetadata::new(format!("Drop owner {index}")).unwrap(),
                    ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
                )
                .unwrap();
            let session_id = issuer
                .login_local(
                    caller,
                    user_id,
                    profile_id,
                    None,
                    IdempotencyKey::new(format!("service-drop-effect-race-{index}")).unwrap(),
                )
                .unwrap();
            let context = issuer.interactive_session(caller).unwrap();
            handles.push(Arc::new(context.revalidation_handle().unwrap()));
            session_ids.push(session_id);
        }

        let recovery_finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let after_recovery = Arc::new([
            std::sync::atomic::AtomicBool::new(false),
            std::sync::atomic::AtomicBool::new(false),
        ]);
        let (effect_entered, effect_entered_rx) = mpsc::channel();
        let mut release_effects = Vec::new();
        let mut effects = Vec::new();
        for (index, handle) in handles.iter().cloned().enumerate() {
            let (release_effect, release_effect_rx) = mpsc::channel();
            release_effects.push(release_effect);
            let effect_entered = effect_entered.clone();
            let recovery_flag = Arc::clone(&recovery_finished);
            let effect_after_recovery = Arc::clone(&after_recovery);
            effects.push(Some(thread::spawn(move || {
                handle.with_live_session_effect(|| {
                    effect_entered.send(index).unwrap();
                    release_effect_rx
                        .recv_timeout(Duration::from_secs(2))
                        .unwrap();
                    effect_after_recovery[index]
                        .store(recovery_flag.load(Ordering::Acquire), Ordering::Release);
                })
            })));
        }
        let entered = [
            effect_entered_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
            effect_entered_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
        ];
        assert_eq!(entered.len(), 2);

        let admission = Arc::clone(&issuer.issuer.admission);
        let (dropped, dropped_rx) = mpsc::channel();
        let dropper = thread::spawn(move || {
            drop(issuer);
            dropped.send(()).unwrap();
        });

        admission.wait_until_closed_active(2);
        assert_eq!(
            handles[0].check_live(),
            Err(ResolutionError::Unavailable),
            "issuer shutdown closes new admission before draining existing effects"
        );
        assert_eq!(
            LinuxIdentitySnapshotStore::open(directory.open()).err(),
            Some(crate::store::DurableStoreError::Locked),
            "the durable lock remains held until all admitted callbacks finish"
        );

        release_effects[entered[0]].send(()).unwrap();
        assert_eq!(effects[entered[0]].take().unwrap().join().unwrap(), Ok(()));
        admission.wait_until_closed_active(1);
        assert!(matches!(
            LinuxIdentitySnapshotStore::open(directory.open()),
            Err(crate::store::DurableStoreError::Locked)
        ));

        release_effects[entered[1]].send(()).unwrap();
        assert_eq!(effects[entered[1]].take().unwrap().join().unwrap(), Ok(()));
        dropped_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("issuer shutdown did not finish after all callbacks drained");
        dropper.join().unwrap();
        assert!(after_recovery
            .iter()
            .all(|value| !value.load(Ordering::Acquire)));

        let mut restarted = service(&directory, clock);
        let report = restarted.recover(&callers[0]).unwrap();
        assert_eq!(report.interrupted_sessions_ended, 2);
        recovery_finished.store(true, Ordering::Release);
        for (handle, session_id) in handles.iter().zip(session_ids) {
            assert_eq!(handle.session_id(), &session_id);
            assert_eq!(handle.check_live(), Err(ResolutionError::Unavailable));
        }
    }

    #[test]
    fn callback_panic_releases_gate_without_poisoning_reusable_session() {
        let directory = DurableTestDirectory::new();
        let caller = TestCaller::new("callback-panic-gate");
        let clock = TestClock::at(100);
        let mut issuer = service(&directory, clock);
        issuer.recover(&caller).unwrap();
        let (user_id, profile_id) = issuer
            .create_local_user(
                &caller,
                LocalUserMetadata::new("Panic owner").unwrap(),
                ProfileMetadata::new("Home", Some("en-US".into())).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &caller,
                user_id,
                profile_id,
                None,
                IdempotencyKey::new("callback-panic-gate").unwrap(),
            )
            .unwrap();
        let context = issuer.interactive_session(&caller).unwrap();
        let handle = Arc::new(context.revalidation_handle().unwrap());

        let admission_state = Arc::clone(&handle.identity.issuer_admission);
        let admission_guard = admission_state.state.lock().unwrap();
        let (snapshot_result, snapshot_result_rx) = mpsc::channel();
        let snapshot_handle = Arc::clone(&handle);
        let snapshot_probe = thread::spawn(move || {
            snapshot_result
                .send(snapshot_handle.try_live_snapshot())
                .unwrap();
        });
        let nonblocking_snapshot = snapshot_result_rx.recv_timeout(Duration::from_secs(2));
        drop(admission_guard);
        assert_eq!(
            nonblocking_snapshot.unwrap(),
            Err(TryLiveError::StateUnavailable),
            "zero-wait snapshots also fail fast when issuer admission is contended"
        );
        snapshot_probe.join().unwrap();

        let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = handle.with_live_session_effect(|| panic!("effect outcome is indeterminate"));
        }));
        assert!(panic_result.is_err());
        assert_eq!(handle.check_live(), Ok(()));

        let mut effect_reused = false;
        assert_eq!(
            handle.with_live_session_effect(|| effect_reused = true),
            Ok(())
        );
        assert!(effect_reused);
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        feature = "linux-interactive-account",
        target_arch = "x86_64"
    ))]
    #[test]
    fn previous_launch_terminal_receipt_recovers_pre_session_login_rejection() {
        let directory = DurableTestDirectory::new();
        let old_principal = verified_test_principal(17);
        let previous_scope = TestRecoveryScope::new(17, &old_principal);
        let old_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let (mut old_identity, _old_events) =
            crate::c1_linux_boot_identity_service_from_store(old_store, &old_principal);
        let old_handoff = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut old_identity,
                    &old_principal,
                );
            adapter
                .recover_with_new_intent_into_runtime(
                    &previous_scope,
                    &mut ImmediateRuntimeRecoveryGate,
                )
                .unwrap()
        };

        // Model a definitive password rejection before C1 stages a Session.
        // The first-owner mapping is durable, and no Session is present.
        let (user_id, profile_id) = {
            let admission = old_identity
                .enter_system_service_runtime_admission(&old_handoff, &previous_scope)
                .unwrap();
            let binding = crate::provision_c1_linux_first_owner_binding_with_runtime_admission(
                &mut old_identity,
                &old_principal,
                &admission,
                b"fixture-owner",
            )
            .unwrap();
            (
                UserId::new(binding.user_id.get()).unwrap(),
                ProfileId::new(binding.profile_id).unwrap(),
            )
        };
        let old_record = old_identity
            .store
            .recovery_journal_record()
            .unwrap()
            .unwrap();
        assert_eq!(old_record.phase, RecoveryJournalPhase::RuntimeActive);
        assert!(old_identity.snapshot().sessions.is_empty());
        drop(old_identity);

        let current_principal = verified_test_principal(18);
        let current_scope = TestRecoveryScope::new(18, &current_principal);
        let current_scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &current_scope;
        let previous_scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &previous_scope;
        let current_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let (mut current_identity, _current_events) =
            crate::c1_linux_boot_identity_service_from_store(current_store, &current_principal);
        let mut c3_gate =
            ConfirmedPriorLaunchC3GateFixture::new(current_scope_ref, previous_scope_ref);
        let receipt = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut current_identity,
                    &current_principal,
                );
            adapter
                .reconcile_previous_launch_terminal(current_scope_ref, &mut c3_gate)
                .unwrap()
        };
        assert_eq!(c3_gate.invocations, 1);
        assert_eq!(c3_gate.callbacks, 1);
        let expected_claim =
            nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation::new(
                NonZeroU64::new(old_record.operation_id.get()).unwrap(),
                *old_record.binding.digest_bytes(),
            );
        assert_eq!(c3_gate.observed_claim, Some(expected_claim));
        let reconciled = current_identity
            .store
            .recovery_journal_record()
            .unwrap()
            .unwrap();
        assert_eq!(reconciled.operation_id, old_record.operation_id);
        assert_eq!(reconciled.binding, old_record.binding);
        assert!(matches!(
            reconciled.phase,
            RecoveryJournalPhase::Reconciled { .. }
        ));
        assert!(
            current_identity.store.recovery_quarantined().unwrap(),
            "the confirmed row remains quarantined while this terminally fenced issuer is alive"
        );
        {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut current_identity,
                    &current_principal,
                );
            adapter
                .consume_previous_launch_terminal_receipt(receipt)
                .unwrap();
        }
        drop(current_identity);

        // A fresh C1 service can now recover with the exact current scope.
        let reopened_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        assert!(
            !reopened_store.recovery_quarantined().unwrap(),
            "only the reopened store observes the confirmed terminal row as unquarantined"
        );
        let (mut reopened_identity, _reopened_events) =
            crate::c1_linux_boot_identity_service_from_store(reopened_store, &current_principal);
        let current_handoff = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut reopened_identity,
                    &current_principal,
                );
            adapter
                .recover_with_new_intent_into_runtime(
                    &current_scope,
                    &mut ImmediateRuntimeRecoveryGate,
                )
                .unwrap()
        };
        assert_eq!(
            current_handoff.recovery_report().interrupted_sessions_ended,
            0
        );
        assert_eq!(reopened_identity.snapshot().users.len(), 1);
        assert_eq!(reopened_identity.snapshot().users[0].user_id, user_id);
        assert_eq!(reopened_identity.snapshot().profiles.len(), 1);
        assert_eq!(
            reopened_identity.snapshot().profiles[0].profile_id,
            profile_id
        );
        assert_eq!(
            reopened_identity.snapshot().profiles[0].owner_user_id,
            Some(user_id)
        );
        assert!(reopened_identity.snapshot().sessions.is_empty());

        let admission = reopened_identity
            .enter_system_service_runtime_admission(&current_handoff, &current_scope)
            .unwrap();
        let resumed = crate::provision_c1_linux_first_owner_binding_with_runtime_admission(
            &mut reopened_identity,
            &current_principal,
            &admission,
            b"fixture-owner",
        )
        .unwrap();
        assert_eq!(resumed.user_id.get(), user_id.get());
        assert_eq!(resumed.profile_id, profile_id.as_str());
        assert!(reopened_identity.snapshot().sessions.is_empty());
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    #[test]
    fn previous_launch_terminal_store_write_failure_returns_unknown_and_exact_reopen_readback_recovers(
    ) {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let directory = DurableTestDirectory::new();
        let old_principal = verified_test_principal(47);
        let previous_scope = TestRecoveryScope::new(47, &old_principal);
        let old_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let (mut old_identity, _old_events) =
            crate::c1_linux_boot_identity_service_from_store(old_store, &old_principal);
        let old_handoff = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut old_identity,
                    &old_principal,
                );
            adapter
                .recover_with_new_intent_into_runtime(
                    &previous_scope,
                    &mut ImmediateRuntimeRecoveryGate,
                )
                .unwrap()
        };
        let old_record = old_identity
            .store
            .recovery_journal_record()
            .unwrap()
            .unwrap();
        assert_eq!(old_record.phase, RecoveryJournalPhase::RuntimeActive);
        assert!(old_identity.snapshot().sessions.is_empty());
        drop(old_handoff);
        drop(old_identity);

        let current_principal = verified_test_principal(48);
        let current_scope = TestRecoveryScope::new(48, &current_principal);
        let current_scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &current_scope;
        let previous_scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &previous_scope;
        let current_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let (mut current_identity, _current_events) =
            crate::c1_linux_boot_identity_service_from_store(current_store, &current_principal);
        current_identity
            .store
            .set_fault_for_test(DurableStoreFault::AfterRename);
        let mut first_c3_gate =
            ConfirmedPriorLaunchC3GateFixture::new(current_scope_ref, previous_scope_ref);
        let first_result = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut current_identity,
                    &current_principal,
                );
            adapter.reconcile_previous_launch_terminal(current_scope_ref, &mut first_c3_gate)
        };
        assert_eq!(first_result, Err(RecoveryClientError::OutcomeUnknown));
        assert_eq!(first_c3_gate.invocations, 1);
        assert_eq!(first_c3_gate.callbacks, 1);
        assert_eq!(
            first_c3_gate.observed_claim,
            Some(
                nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation::new(
                    NonZeroU64::new(old_record.operation_id.get()).unwrap(),
                    *old_record.binding.digest_bytes(),
                )
            )
        );
        assert!(!current_identity.issuer.alive.load(Ordering::Acquire));
        assert_eq!(
            current_identity.store.reconciliation_fence_state_for_test(),
            (true, true, 0),
            "an uncertain final C1 write must keep the same issuer and store fence closed"
        );
        drop(current_identity);

        // The fault happens after rename, so a new store can observe the exact
        // terminal row. Do not treat the poisoned same-process read as proof.
        let reopened_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let expected_outcome =
            system_service_terminal_outcome_digest(old_record.operation_id, &old_record.binding);
        let readback = reopened_store.recovery_journal_record().unwrap().unwrap();
        assert_eq!(readback.operation_id, old_record.operation_id);
        assert_eq!(readback.binding, old_record.binding);
        assert_eq!(
            readback.phase,
            RecoveryJournalPhase::Reconciled {
                outcome_digest: expected_outcome
            }
        );
        assert!(
            !reopened_store.recovery_quarantined().unwrap(),
            "a fresh store may clear quarantine only after exact terminal readback"
        );

        // Re-run the C3 durable readback after reopening to reissue a C1
        // receipt. This is the normal exact reconciliation path, not a guess
        // based on the previous process's in-memory result.
        let (mut reopened_identity, _reopened_events) =
            crate::c1_linux_boot_identity_service_from_store(reopened_store, &current_principal);
        let mut retry_c3_gate =
            ConfirmedPriorLaunchC3GateFixture::new(current_scope_ref, previous_scope_ref);
        let receipt = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut reopened_identity,
                    &current_principal,
                );
            adapter
                .reconcile_previous_launch_terminal(current_scope_ref, &mut retry_c3_gate)
                .unwrap()
        };
        assert_eq!(retry_c3_gate.invocations, 1);
        assert_eq!(retry_c3_gate.callbacks, 1);
        {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut reopened_identity,
                    &current_principal,
                );
            adapter
                .consume_previous_launch_terminal_receipt(receipt)
                .unwrap();
        }
        assert!(reopened_identity.store.recovery_quarantined().unwrap());
        drop(reopened_identity);

        let final_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        assert!(
            !final_store.recovery_quarantined().unwrap(),
            "a successful exact receipt becomes unquarantined after reopen and before intent"
        );
        let (mut final_identity, _final_events) =
            crate::c1_linux_boot_identity_service_from_store(final_store, &current_principal);
        let handoff = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut final_identity,
                    &current_principal,
                );
            adapter
                .recover_with_new_intent_into_runtime(
                    &current_scope,
                    &mut ImmediateRuntimeRecoveryGate,
                )
                .unwrap()
        };
        let next_record = final_identity
            .store
            .recovery_journal_record()
            .unwrap()
            .unwrap();
        assert!(next_record.operation_id.get() > old_record.operation_id.get());
        assert_eq!(next_record.phase, RecoveryJournalPhase::RuntimeActive);
        assert_eq!(
            handoff.operation_identity().operation_id().get(),
            next_record.operation_id.get()
        );
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        feature = "linux-interactive-account",
        target_arch = "x86_64"
    ))]
    #[test]
    fn prior_boot_clear_ends_initial_session_and_recovers_after_power_loss() {
        interrupted_initial_session_ends_on_boot(true);
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        feature = "linux-interactive-account",
        target_arch = "x86_64"
    ))]
    #[test]
    fn previous_launch_terminal_receipt_ends_unattached_initial_session_on_boot() {
        interrupted_initial_session_ends_on_boot(false);
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        feature = "linux-interactive-account",
        target_arch = "x86_64"
    ))]
    fn interrupted_initial_session_ends_on_boot(prior_boot: bool) {
        use std::os::unix::net::UnixStream;

        let directory = DurableTestDirectory::new();
        let old_principal = verified_test_principal(27);
        let previous_scope = TestRecoveryScope::new(27, &old_principal);
        let old_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let (mut old_identity, _old_events) =
            crate::c1_linux_boot_identity_service_from_store(old_store, &old_principal);
        let old_handoff = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut old_identity,
                    &old_principal,
                );
            adapter
                .recover_with_new_intent_into_runtime(
                    &previous_scope,
                    &mut ImmediateRuntimeRecoveryGate,
                )
                .unwrap()
        };
        let admission = old_identity
            .enter_system_service_runtime_admission(&old_handoff, &previous_scope)
            .unwrap();
        let owner = crate::provision_c1_linux_first_owner_binding_with_runtime_admission(
            &mut old_identity,
            &old_principal,
            &admission,
            b"fixture-owner",
        )
        .unwrap();
        let user_id = UserId::new(owner.user_id.get()).unwrap();
        let profile_id = ProfileId::new(owner.profile_id).unwrap();
        let (stream, _peer_stream) = UnixStream::pair().unwrap();
        let peer =
            nagi_peer_authenticated_connection::KernelAuthenticatedUnixPeer::authenticate(&stream)
                .unwrap();
        let mut pending =
            crate::linux_interactive_account::pending_initial_interactive_session_for_test(
                &old_identity,
                owner.account_id,
                user_id,
                profile_id.clone(),
                owner.principal_id,
                peer.connection_id(),
            );
        let session_id = pending
            .commit_after_c2_owner_postcheck_with_runtime_admission(
                &mut old_identity,
                &admission,
                None,
                IdempotencyKey::new("initial-attach-interrupted").unwrap(),
            )
            .unwrap()
            .session_id()
            .clone();
        assert!(old_identity.snapshot().sessions.iter().any(|session| {
            session.session_id == session_id
                && matches!(session.lifecycle, SessionLifecycle::Active)
        }));
        let old_record = old_identity
            .store
            .recovery_journal_record()
            .unwrap()
            .unwrap();
        assert_eq!(old_record.phase, RecoveryJournalPhase::RuntimeActive);
        drop(pending);
        drop(admission);
        drop(old_identity);

        let current_principal = verified_test_principal(28);
        let current_scope = TestRecoveryScope::new(28, &current_principal);
        let current_scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &current_scope;
        let previous_scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &previous_scope;
        let current_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let (mut current_identity, _current_events) =
            crate::c1_linux_boot_identity_service_from_store(current_store, &current_principal);
        let mut c3_gate = ConfirmedPriorLaunchC3GateFixture::prior_boot_clear(
            current_scope_ref,
            previous_scope_ref,
            [0x11; 16],
            [0x22; 16],
        );
        let receipt = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut current_identity,
                    &current_principal,
                );
            if prior_boot {
                adapter.reconcile_previous_boot_terminal(current_scope_ref, &mut c3_gate)
            } else {
                adapter.reconcile_previous_launch_terminal(current_scope_ref, &mut c3_gate)
            }
            .unwrap()
        };
        assert_eq!(c3_gate.callbacks, 1);
        assert_eq!(
            c3_gate.observed_claim.unwrap().operation_id(),
            NonZeroU64::new(old_record.operation_id.get()).unwrap()
        );
        assert!(
            current_identity.store.recovery_quarantined().unwrap(),
            "the current issuer stays quarantined after C3 confirmation until it is dropped"
        );
        {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut current_identity,
                    &current_principal,
                );
            adapter
                .consume_previous_launch_terminal_receipt(receipt)
                .unwrap();
        }
        drop(current_identity);

        let reopened_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let reopened_record = reopened_store.recovery_journal_record().unwrap().unwrap();
        assert_eq!(reopened_record.operation_id, old_record.operation_id);
        assert_eq!(reopened_record.binding, old_record.binding);
        assert_eq!(
            reopened_record.phase,
            RecoveryJournalPhase::Reconciled {
                outcome_digest: if prior_boot {
                    system_service_prior_boot_outcome_digest(
                        old_record.operation_id,
                        &old_record.binding,
                    )
                } else {
                    system_service_terminal_outcome_digest(
                        old_record.operation_id,
                        &old_record.binding,
                    )
                }
            },
            "the durable outcome digest distinguishes the terminal evidence"
        );
        assert!(
            !reopened_store.recovery_quarantined().unwrap(),
            "the exact terminal row is unquarantined only in the reopened store"
        );
        let (mut reopened_identity, _reopened_events) =
            crate::c1_linux_boot_identity_service_from_store(reopened_store, &current_principal);
        let current_handoff = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut reopened_identity,
                    &current_principal,
                );
            adapter
                .recover_with_new_intent_into_runtime(
                    &current_scope,
                    &mut ImmediateRuntimeRecoveryGate,
                )
                .unwrap()
        };
        assert_eq!(
            current_handoff.recovery_report().interrupted_sessions_ended,
            1
        );
        let next_record = reopened_identity
            .store
            .recovery_journal_record()
            .unwrap()
            .unwrap();
        assert_eq!(next_record.phase, RecoveryJournalPhase::RuntimeActive);
        assert_ne!(next_record.operation_id, old_record.operation_id);
        assert_eq!(reopened_identity.snapshot().users.len(), 1);
        assert_eq!(reopened_identity.snapshot().users[0].user_id, user_id);
        assert_eq!(reopened_identity.snapshot().profiles.len(), 1);
        assert_eq!(
            reopened_identity.snapshot().profiles[0].profile_id,
            profile_id
        );
        assert_eq!(reopened_identity.snapshot().sessions.len(), 1);
        assert_eq!(
            reopened_identity.snapshot().sessions[0].session_id,
            session_id
        );
        assert!(matches!(
            reopened_identity.snapshot().sessions[0].lifecycle,
            SessionLifecycle::Ended {
                reason: SessionEndReason::SystemRestart,
                ..
            }
        ));
        assert!(!reopened_identity
            .snapshot()
            .sessions
            .iter()
            .any(|session| matches!(session.lifecycle, SessionLifecycle::Active)));
    }

    #[cfg(all(feature = "system-service-recovery-contract", target_arch = "x86_64"))]
    #[test]
    fn unknown_previous_boot_terminal_result_preserves_c1_quarantine() {
        use nagi_system_service_recovery_contract::RecoveryClientError;

        let directory = DurableTestDirectory::new();
        let old_principal = verified_test_principal(37);
        let previous_scope = TestRecoveryScope::new(37, &old_principal);
        let old_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let (mut old_identity, _old_events) =
            crate::c1_linux_boot_identity_service_from_store(old_store, &old_principal);
        {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut old_identity,
                    &old_principal,
                );
            let _old_handoff = adapter
                .recover_with_new_intent_into_runtime(
                    &previous_scope,
                    &mut ImmediateRuntimeRecoveryGate,
                )
                .unwrap();
        }
        let old_record = old_identity
            .store
            .recovery_journal_record()
            .unwrap()
            .unwrap();
        drop(old_identity);

        let current_principal = verified_test_principal(38);
        let current_scope = TestRecoveryScope::new(38, &current_principal);
        let current_scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &current_scope;
        let previous_scope_ref: &dyn nagi_system_service_recovery_contract::SystemServiceRecoveryAdmissionScope =
            &previous_scope;
        let current_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let (mut current_identity, _current_events) =
            crate::c1_linux_boot_identity_service_from_store(current_store, &current_principal);
        let mut c3_gate =
            UnknownPriorLaunchC3GateFixture::new(current_scope_ref, previous_scope_ref);
        let result = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut current_identity,
                    &current_principal,
                );
            adapter.reconcile_previous_boot_terminal(current_scope_ref, &mut c3_gate)
        };
        assert_eq!(result, Err(RecoveryClientError::OutcomeUnknown));
        assert_eq!(c3_gate.invocations, 1);
        assert_eq!(c3_gate.callbacks, 1);
        assert_eq!(
            current_identity.store.recovery_journal_record().unwrap(),
            Some(old_record.clone())
        );
        assert!(current_identity.store.recovery_quarantined().unwrap());

        let blocked_new_intent = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut current_identity,
                    &current_principal,
                );
            adapter.recover_with_new_intent_into_runtime(
                &current_scope,
                &mut ImmediateRuntimeRecoveryGate,
            )
        };
        assert!(matches!(
            blocked_new_intent,
            Err(IdentityError::StoreUnavailable)
        ));
        assert_eq!(
            current_identity.store.recovery_journal_record().unwrap(),
            Some(old_record.clone())
        );
        assert!(current_identity.store.recovery_quarantined().unwrap());
        drop(current_identity);

        // The post-callback C3 OutcomeUnknown must survive a full C1 service
        // and store reopen, not only checks against the original instance.
        let reopened_store = LinuxIdentitySnapshotStore::open(directory.open()).unwrap();
        let (mut reopened_identity, _reopened_events) =
            crate::c1_linux_boot_identity_service_from_store(reopened_store, &current_principal);
        assert_eq!(
            reopened_identity.store.recovery_journal_record().unwrap(),
            Some(old_record.clone())
        );
        assert!(reopened_identity.store.recovery_quarantined().unwrap());

        let blocked_after_reopen = {
            let mut adapter =
                C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(
                    &mut reopened_identity,
                    &current_principal,
                );
            adapter.recover_with_new_intent_into_runtime(
                &current_scope,
                &mut ImmediateRuntimeRecoveryGate,
            )
        };
        assert!(matches!(
            blocked_after_reopen,
            Err(IdentityError::StoreUnavailable)
        ));
        assert_eq!(
            reopened_identity.store.recovery_journal_record().unwrap(),
            Some(old_record)
        );
        assert!(reopened_identity.store.recovery_quarantined().unwrap());
    }
}
