//! Linux runtime providers for the verified C1 boot identity service.
//!
//! The factory consumes an already-selected private Data directory handle and
//! a C1 system-service principal copied from A's live C3 binding. It opens the
//! durable C1 journal and supplies Linux-kernel entropy/time, a narrow boot
//! identity policy, a bounded in-memory diagnostic sink, and an explicitly
//! unavailable guest-profile cleaner. It never creates or selects Data paths.

#[cfg(feature = "linux-interactive-account")]
use crate::{AuthenticatedProvisioningCaller, IdentityError, LocalUserMetadata, ProfileMetadata};
#[cfg(feature = "linux-interactive-account")]
use nagi_linux_account_verifier::OwnerAccountBinding;
use std::collections::VecDeque;
use std::fs::File;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
use crate::C1SystemServiceRuntimeAdmission;
use crate::{
    AuthorizationFailure, AuthorizationRequest, C1VerifiedBootSystemServicePrincipal,
    CallerContextId, Clock, GuestCleanupFailure, GuestProfileCleaner, IdentityAction,
    IdentityAuthorizer, IdentityEvent, IdentityEventKind, IdentityEventReason, IdentityEventSink,
    IdentityIdSource, IdentityService, LinuxIdentitySnapshotStore, PrincipalId, ProfileId,
    SessionId, TrustedCallerContext, TryClockError, UnixMillis, UserId,
};

/// Concrete C1 service type for recovery from a verified Linux boot service.
pub type C1LinuxBootIdentityService = IdentityService<
    LinuxIdentitySnapshotStore,
    LinuxIdentityIdSource,
    LinuxIdentityClock,
    C1LinuxBootIdentityAuthorizer,
    C1LinuxGuestProfileCleanupUnavailable,
    LinuxIdentityEventSink,
>;

/// The same concrete Linux providers with the profile-volume owner's actual
/// guest deletion implementation substituted for the fail-closed default.
pub type C1LinuxBootIdentityServiceWithGuestCleaner<G> = IdentityService<
    LinuxIdentitySnapshotStore,
    LinuxIdentityIdSource,
    LinuxIdentityClock,
    C1LinuxBootIdentityAuthorizer,
    G,
    LinuxIdentityEventSink,
>;

#[cfg(feature = "linux-interactive-account")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum C1LinuxFirstOwnerProvisionError {
    InvalidAccountName,
    AlreadyProvisioned,
    Identity(IdentityError),
    IdUnavailable,
}

/// Persist the C1 User/Profile mapping for the account name submitted by A's
/// first-run registration UI, then return the exact binding A stores beside
/// the credential. The account credential never enters C1.
///
/// If C1 mapping persistence succeeds but A's account publication fails
/// before creating its record, another submission with the same account name
/// resumes that one-user/one-profile mapping. A's account verifier remains the
/// owner of atomic no-overwrite publication and its unknown-outcome contract.
#[cfg(feature = "linux-interactive-account")]
pub fn provision_c1_linux_first_owner_binding<G: GuestProfileCleaner>(
    identity: &mut C1LinuxBootIdentityServiceWithGuestCleaner<G>,
    principal: &C1VerifiedBootSystemServicePrincipal,
    account_name: &[u8],
) -> Result<OwnerAccountBinding, C1LinuxFirstOwnerProvisionError> {
    let account_name = std::str::from_utf8(account_name)
        .map_err(|_| C1LinuxFirstOwnerProvisionError::InvalidAccountName)?;
    let user_metadata = LocalUserMetadata::new(account_name)
        .map_err(|_| C1LinuxFirstOwnerProvisionError::InvalidAccountName)?;
    let profile_metadata = ProfileMetadata::new(account_name, None)
        .map_err(|_| C1LinuxFirstOwnerProvisionError::InvalidAccountName)?;

    let caller = BootProvisioningCaller::from_verified(principal);
    identity
        .authorize_first_owner_provisioning(&caller)
        .map_err(C1LinuxFirstOwnerProvisionError::Identity)?;

    let (user_id, profile_id) = match resume_pending_first_owner_mapping(identity, account_name)? {
        Some(mapping) => mapping,
        None => identity
            .create_local_user(&caller, user_metadata, profile_metadata)
            .map_err(C1LinuxFirstOwnerProvisionError::Identity)?,
    };
    let principal_id = PrincipalId::new(format!("user:{}", user_id.get()))
        .map_err(|_| C1LinuxFirstOwnerProvisionError::InvalidAccountName)?;
    let account_id = random_nonzero_u64().ok_or(C1LinuxFirstOwnerProvisionError::IdUnavailable)?;

    Ok(OwnerAccountBinding {
        account_id,
        user_id: NonZeroU64::new(user_id.get())
            .ok_or(C1LinuxFirstOwnerProvisionError::IdUnavailable)?,
        profile_id: profile_id.as_str().to_owned(),
        principal_id,
    })
}

/// Persist the first C1 User/Profile mapping while the caller holds the exact
/// C1 RuntimeActive admission inside A's C3 continuation gate. The admission
/// is checked against the sealed boot principal and its durable operation on
/// each C1 mutation; generic recovered-state operations remain quarantined.
#[cfg(all(
    feature = "system-service-recovery-contract",
    feature = "linux-interactive-account",
    target_os = "linux",
    target_arch = "x86_64"
))]
pub fn provision_c1_linux_first_owner_binding_with_runtime_admission<G: GuestProfileCleaner>(
    identity: &mut C1LinuxBootIdentityServiceWithGuestCleaner<G>,
    principal: &C1VerifiedBootSystemServicePrincipal,
    admission: &C1SystemServiceRuntimeAdmission<'_>,
    account_name: &[u8],
) -> Result<OwnerAccountBinding, C1LinuxFirstOwnerProvisionError> {
    let account_name = std::str::from_utf8(account_name)
        .map_err(|_| C1LinuxFirstOwnerProvisionError::InvalidAccountName)?;
    let user_metadata = LocalUserMetadata::new(account_name)
        .map_err(|_| C1LinuxFirstOwnerProvisionError::InvalidAccountName)?;
    let profile_metadata = ProfileMetadata::new(account_name, None)
        .map_err(|_| C1LinuxFirstOwnerProvisionError::InvalidAccountName)?;

    let caller = BootProvisioningCaller::from_verified(principal);
    identity
        .authorize_first_owner_provisioning_with_runtime_admission(&caller, principal, admission)
        .map_err(C1LinuxFirstOwnerProvisionError::Identity)?;

    let (user_id, profile_id) = match resume_pending_first_owner_mapping(identity, account_name)? {
        Some(mapping) => mapping,
        None => identity
            .create_local_user_with_runtime_admission(
                admission,
                &caller,
                user_metadata,
                profile_metadata,
            )
            .map_err(C1LinuxFirstOwnerProvisionError::Identity)?,
    };
    let principal_id = PrincipalId::new(format!("user:{}", user_id.get()))
        .map_err(|_| C1LinuxFirstOwnerProvisionError::InvalidAccountName)?;
    let account_id = random_nonzero_u64().ok_or(C1LinuxFirstOwnerProvisionError::IdUnavailable)?;

    Ok(OwnerAccountBinding {
        account_id,
        user_id: NonZeroU64::new(user_id.get())
            .ok_or(C1LinuxFirstOwnerProvisionError::IdUnavailable)?,
        profile_id: profile_id.as_str().to_owned(),
        principal_id,
    })
}

#[cfg(feature = "linux-interactive-account")]
fn resume_pending_first_owner_mapping<G: GuestProfileCleaner>(
    identity: &C1LinuxBootIdentityServiceWithGuestCleaner<G>,
    account_name: &str,
) -> Result<Option<(UserId, ProfileId)>, C1LinuxFirstOwnerProvisionError> {
    let snapshot = identity.snapshot();
    snapshot
        .validate()
        .map_err(|_| C1LinuxFirstOwnerProvisionError::AlreadyProvisioned)?;
    if snapshot.users.is_empty()
        && snapshot.profiles.is_empty()
        && snapshot.sessions.is_empty()
        && snapshot.profile_id_history_complete
    {
        return Ok(None);
    }

    if snapshot.users.len() != 1
        || snapshot.profiles.len() != 1
        || !snapshot.sessions.is_empty()
        || !snapshot.profile_id_history_complete
    {
        return Err(C1LinuxFirstOwnerProvisionError::AlreadyProvisioned);
    }
    let user = &snapshot.users[0];
    let profile = &snapshot.profiles[0];
    if user.metadata.display_name != account_name
        || profile.metadata.display_name != account_name
        || profile.owner_user_id != Some(user.user_id)
        || profile.kind != crate::ProfileKind::Persistent
    {
        return Err(C1LinuxFirstOwnerProvisionError::AlreadyProvisioned);
    }
    Ok(Some((user.user_id, profile.profile_id.clone())))
}

#[cfg(feature = "linux-interactive-account")]
fn random_nonzero_u64() -> Option<NonZeroU64> {
    for _ in 0..8 {
        let bytes = random_bytes::<8>().ok()?;
        if let Some(value) = NonZeroU64::new(u64::from_le_bytes(bytes)) {
            return Some(value);
        }
    }
    None
}

#[cfg(feature = "linux-interactive-account")]
struct BootProvisioningCaller {
    principal: PrincipalId,
    context: CallerContextId,
}

#[cfg(feature = "linux-interactive-account")]
impl BootProvisioningCaller {
    fn from_verified(principal: &C1VerifiedBootSystemServicePrincipal) -> Self {
        Self {
            principal: principal.principal_id().clone(),
            context: principal.context_id().clone(),
        }
    }
}

#[cfg(feature = "linux-interactive-account")]
impl TrustedCallerContext for BootProvisioningCaller {
    fn principal_id(&self) -> &PrincipalId {
        &self.principal
    }

    fn context_id(&self) -> &CallerContextId {
        &self.context
    }
}

#[cfg(feature = "linux-interactive-account")]
impl crate::service::caller_proof_seal::Provisioning for BootProvisioningCaller {}

#[cfg(feature = "linux-interactive-account")]
impl AuthenticatedProvisioningCaller for BootProvisioningCaller {}

/// Open C1's durable store from an owner-selected, already-existing directory
/// and build the concrete runtime provider set. The caller remains responsible
/// for selecting and provisioning the Data volume and for retaining the
/// account verifier from that same volume.
pub fn open_c1_linux_boot_identity_service(
    directory: File,
    principal: &C1VerifiedBootSystemServicePrincipal,
) -> Result<(C1LinuxBootIdentityService, LinuxIdentityEventReader), crate::DurableStoreError> {
    let store = LinuxIdentitySnapshotStore::open(directory)?;
    Ok(c1_linux_boot_identity_service_from_store(store, principal))
}

/// Build the same runtime providers around a C1 store that the boot owner has
/// already opened. This is the direct bridge for A's existing
/// `identity_inputs()` path, which returns the store directory beside the
/// account verifier.
pub fn c1_linux_boot_identity_service_from_store(
    store: LinuxIdentitySnapshotStore,
    principal: &C1VerifiedBootSystemServicePrincipal,
) -> (C1LinuxBootIdentityService, LinuxIdentityEventReader) {
    c1_linux_boot_identity_service_with_guest_cleaner(
        store,
        principal,
        C1LinuxGuestProfileCleanupUnavailable,
    )
}

/// Build the Linux identity service with the selected Data/profile owner's
/// guest deletion implementation. This keeps the deletion policy with the
/// owner of guest profile storage while reusing C1's entropy, clock,
/// authorizer, and bounded event sink.
pub fn c1_linux_boot_identity_service_with_guest_cleaner<G: GuestProfileCleaner>(
    store: LinuxIdentitySnapshotStore,
    principal: &C1VerifiedBootSystemServicePrincipal,
    guest_cleaner: G,
) -> (
    C1LinuxBootIdentityServiceWithGuestCleaner<G>,
    LinuxIdentityEventReader,
) {
    let (events, reader) = LinuxIdentityEventSink::channel();
    let authorizer = C1LinuxBootIdentityAuthorizer::for_verified_boot_principal(principal);
    let identity = IdentityService::new(
        store,
        LinuxIdentityIdSource,
        LinuxIdentityClock::default(),
        authorizer,
        guest_cleaner,
        events,
    );
    (identity, reader)
}

/// Draw all generated identity IDs from the running Linux kernel's
/// `getrandom(2)` source. Collisions remain checked against the durable C1
/// snapshot by `IdentityService`; no boot-fixed or host-derived secret is
/// used.
#[derive(Clone, Copy, Debug, Default)]
pub struct LinuxIdentityIdSource;

impl IdentityIdSource for LinuxIdentityIdSource {
    fn next_user_id(&mut self) -> Result<UserId, crate::IdSourceFailure> {
        for _ in 0..8 {
            let bytes = random_bytes::<8>()?;
            if let Ok(id) = UserId::new(u64::from_le_bytes(bytes)) {
                return Ok(id);
            }
        }
        Err(crate::IdSourceFailure::Unavailable)
    }

    fn next_profile_id(&mut self) -> Result<ProfileId, crate::IdSourceFailure> {
        ProfileId::new(format!("profile_{}", random_hex::<16>()?))
            .map_err(|_| crate::IdSourceFailure::Unavailable)
    }

    fn next_session_id(&mut self) -> Result<SessionId, crate::IdSourceFailure> {
        SessionId::new(format!("session_{}", random_hex::<16>()?))
            .map_err(|_| crate::IdSourceFailure::Unavailable)
    }

    fn next_caller_context_id(&mut self) -> Result<CallerContextId, crate::IdSourceFailure> {
        CallerContextId::new(format!("caller_{}", random_hex::<16>()?))
            .map_err(|_| crate::IdSourceFailure::Unavailable)
    }
}

fn random_bytes<const N: usize>() -> Result<[u8; N], crate::IdSourceFailure> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| crate::IdSourceFailure::Unavailable)?;
    Ok(bytes)
}

fn random_hex<const N: usize>() -> Result<String, crate::IdSourceFailure> {
    let bytes = random_bytes::<N>()?;
    let mut encoded = String::with_capacity(N * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").map_err(|_| crate::IdSourceFailure::Unavailable)?;
    }
    Ok(encoded)
}

/// Wall-clock milliseconds from the running Linux kernel. Values are clamped
/// monotonically for this provider instance so a clock rollback cannot make an
/// already-expired Session live again. If the kernel clock cannot be read,
/// `try_now` fails and `now` returns the fail-closed maximum value.
#[derive(Clone, Debug, Default)]
pub struct LinuxIdentityClock {
    last: Arc<AtomicU64>,
    initialized: Arc<AtomicBool>,
}

impl LinuxIdentityClock {
    fn sample_millis() -> Result<u64, TryClockError> {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| TryClockError::Unavailable)?;
        Ok(duration.as_millis().min(u64::MAX as u128) as u64)
    }

    fn monotonic_max(&self, sampled: u64) -> u64 {
        let mut previous = self.last.load(Ordering::Acquire);
        loop {
            let value = previous.max(sampled);
            match self.last.compare_exchange_weak(
                previous,
                value,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    self.initialized.store(true, Ordering::Release);
                    return value;
                }
                Err(actual) => previous = actual,
            }
        }
    }
}

impl Clock for LinuxIdentityClock {
    fn now(&self) -> UnixMillis {
        let now = Self::sample_millis().map(|sampled| self.monotonic_max(sampled));
        UnixMillis::new(now.unwrap_or_else(|_| {
            if self.initialized.load(Ordering::Acquire) {
                self.last.load(Ordering::Acquire)
            } else {
                u64::MAX
            }
        }))
    }

    fn try_now(&self) -> Result<UnixMillis, TryClockError> {
        Self::sample_millis().map(|sampled| UnixMillis::new(self.monotonic_max(sampled)))
    }
}

/// Narrow policy for the boot identity service. Recovery is permitted only to
/// the exact principal copied from a verified live C3 binding. Interactive
/// session actions require a C1-sealed `user:` caller. First-owner
/// provisioning is accepted only for this exact verified boot principal;
/// guest creation remains denied and profile-root policy stays with its
/// owning storage adapter.
pub struct C1LinuxBootIdentityAuthorizer {
    service_id: String,
    service_principal: PrincipalId,
    service_context: CallerContextId,
    mapping_generation: NonZeroU64,
    catalog_generation: NonZeroU64,
    c3_launch_generation: NonZeroU64,
}

impl C1LinuxBootIdentityAuthorizer {
    pub fn for_verified_boot_principal(principal: &C1VerifiedBootSystemServicePrincipal) -> Self {
        Self {
            service_id: principal.service_id().to_owned(),
            service_principal: principal.principal_id().clone(),
            service_context: principal.context_id().clone(),
            mapping_generation: principal.mapping_generation(),
            catalog_generation: principal.catalog_generation(),
            c3_launch_generation: principal.c3_launch_generation(),
        }
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    fn is_exact_runtime_session_resolution(
        &self,
        request: &AuthorizationRequest,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> bool {
        let Some(user_id) = request.user_id else {
            return false;
        };
        request.action == IdentityAction::ResolveCurrentIdentity
            && request.profile_id.is_some()
            && request.session_id.is_some()
            && request.caller_principal.as_str() == format!("user:{}", user_id.get())
            && admission.matches_boot_principal_binding(
                &self.service_id,
                &self.service_principal,
                &self.service_context,
                self.mapping_generation,
                self.catalog_generation,
                self.c3_launch_generation,
            )
    }

    fn is_exact_boot_request(&self, request: &AuthorizationRequest) -> bool {
        let action_allowed = match request.action {
            IdentityAction::Recover | IdentityAction::ValidateRecoveredState => true,
            #[cfg(feature = "linux-interactive-account")]
            IdentityAction::CreateLocalUser => true,
            _ => false,
        };
        request.caller_principal == self.service_principal
            && request.caller_context == self.service_context
            && action_allowed
            && request.user_id.is_none()
            && request.profile_id.is_none()
            && request.session_id.is_none()
    }

    fn is_c1_interactive_request(request: &AuthorizationRequest) -> bool {
        request.caller_principal.as_str().starts_with("user:")
            && match request.action {
                IdentityAction::LoginLocalUser | IdentityAction::CreateLocalSession => {
                    request.user_id.is_some()
                        && request.profile_id.is_some()
                        && request.session_id.is_none()
                }
                IdentityAction::ResolveCurrentIdentity => {
                    request.user_id.is_none()
                        && request.profile_id.is_none()
                        && request.session_id.is_none()
                }
                IdentityAction::EndSession => {
                    request.user_id.is_some()
                        && request.profile_id.is_some()
                        && request.session_id.is_some()
                }
                _ => false,
            }
    }
}

impl IdentityAuthorizer for C1LinuxBootIdentityAuthorizer {
    fn authorize(&mut self, request: &AuthorizationRequest) -> Result<(), AuthorizationFailure> {
        if self.is_exact_boot_request(request) || Self::is_c1_interactive_request(request) {
            Ok(())
        } else {
            Err(AuthorizationFailure::Denied)
        }
    }

    #[cfg(all(
        feature = "system-service-recovery-contract",
        target_os = "linux",
        target_arch = "x86_64"
    ))]
    fn authorize_with_runtime_admission(
        &mut self,
        request: &AuthorizationRequest,
        admission: &C1SystemServiceRuntimeAdmission<'_>,
    ) -> Result<(), AuthorizationFailure> {
        if self.is_exact_runtime_session_resolution(request, admission) {
            Ok(())
        } else {
            Err(AuthorizationFailure::Denied)
        }
    }
}

/// Fail-closed guest cleanup provider for the persistent-account boot profile.
/// Guest profile storage is owned by the selected Data/profile-volume backend;
/// until that owner supplies its exact deletion operation, C1 leaves cleanup
/// pending instead of reporting a successful no-op.
#[derive(Clone, Copy, Debug, Default)]
pub struct C1LinuxGuestProfileCleanupUnavailable;

impl GuestProfileCleaner for C1LinuxGuestProfileCleanupUnavailable {
    fn cleanup_guest_profile(
        &mut self,
        _profile_id: &ProfileId,
    ) -> Result<(), GuestCleanupFailure> {
        Err(GuestCleanupFailure::Unavailable)
    }
}

const EVENT_CAPACITY: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LinuxIdentityEventRecord {
    pub kind: IdentityEventKind,
    pub reason: Option<IdentityEventReason>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LinuxIdentityEventBatch {
    pub records: Vec<LinuxIdentityEventRecord>,
    pub dropped_total: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LinuxIdentityEventReadError {
    Busy,
    Poisoned,
}

#[derive(Default)]
struct EventQueue {
    records: VecDeque<LinuxIdentityEventRecord>,
    dropped_total: u64,
}

#[derive(Clone, Default)]
pub struct LinuxIdentityEventSink {
    queue: Arc<Mutex<EventQueue>>,
}

#[derive(Clone)]
pub struct LinuxIdentityEventReader {
    queue: Arc<Mutex<EventQueue>>,
}

impl LinuxIdentityEventSink {
    pub fn channel() -> (Self, LinuxIdentityEventReader) {
        let queue = Arc::new(Mutex::new(EventQueue {
            records: VecDeque::with_capacity(EVENT_CAPACITY),
            dropped_total: 0,
        }));
        (
            Self {
                queue: Arc::clone(&queue),
            },
            LinuxIdentityEventReader { queue },
        )
    }
}

impl LinuxIdentityEventReader {
    /// Drain diagnostics without blocking an identity effect. IDs and account
    /// data are intentionally omitted from the bounded event record.
    pub fn try_drain(&self) -> Result<LinuxIdentityEventBatch, LinuxIdentityEventReadError> {
        let mut queue = match self.queue.try_lock() {
            Ok(queue) => queue,
            Err(TryLockError::WouldBlock) => return Err(LinuxIdentityEventReadError::Busy),
            Err(TryLockError::Poisoned(_)) => return Err(LinuxIdentityEventReadError::Poisoned),
        };
        Ok(LinuxIdentityEventBatch {
            records: queue.records.drain(..).collect(),
            dropped_total: queue.dropped_total,
        })
    }
}

impl IdentityEventSink for LinuxIdentityEventSink {
    fn emit(&mut self, event: &IdentityEvent) -> bool {
        let mut queue = match self.queue.try_lock() {
            Ok(queue) => queue,
            Err(TryLockError::WouldBlock | TryLockError::Poisoned(_)) => return false,
        };
        if queue.records.len() == EVENT_CAPACITY {
            queue.records.pop_front();
            queue.dropped_total = queue.dropped_total.saturating_add(1);
        }
        queue.records.push_back(LinuxIdentityEventRecord {
            kind: event.kind,
            reason: event.reason,
        });
        true
    }
}
