//! Maintained PID-1 owner for one A-admitted C3 system-service slot.
//!
//! This owns real clone3 children, their pidfds, endpoint proofs, replacement,
//! revocation, and reaping. The launch entry is unsafe because the current A
//! crate boundary exposes only a consistency view; the caller must supply the
//! exact private A admission and executable bytes already accepted by the
//! trusted catalog/package owner. No safe production callsite exists yet.

use super::linux_init_c3_slot_journal::{
    C3DurableNormalExitReceipt, C3DurableSlotReceipt, C3ReapedLaunchKey, C3SlotJournal,
    C3SlotJournalError, C3SlotJournalReadback, C3SlotReservation, C3SlotState,
};
use super::linux_x86_64_process::LinuxProcessOs;
use super::{
    BackendError, ExecutablePayload, FdDelegationRestrictionAttestation, GateCommit,
    LinuxExecutionIdentity, Prepared, ProcessEndpointPolicy, ProcessHandle, ProcessLaunch,
    ProcessLifecycle, ProcessOs,
};
use crate::c3_system_service_recovery::{C3ProofRecoveryGate, RecoveryScopeSnapshot};
use nagi_system_principal_contract::{SystemServiceEndpointOwnerProof, SystemServiceLaunchRecord};
use nagi_system_service_recovery_contract::{
    C1RecoveryOperationCorrelation, C3DurableRecoverySlotReceipt, C3RecoverySlotGate,
    C3RecoverySlotState, RecoveryClientError, RecoveryLifecycleTransition,
    SystemServiceRecoveryAdmissionScope,
};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::io;
use std::num::NonZeroU64;
use std::os::fd::OwnedFd;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub(super) struct C3LaunchPolicy {
    pub(super) required: bool,
    pub(super) release_wait_budget: Duration,
    pub(super) cancel_check_interval: Duration,
    pub(super) retirement_wait_budget: Duration,
    pub(super) reaper_poll_interval: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LaunchToken(NonZeroU64);

/// Non-forgeable binding for the C3-owned standalone Identity spawn. The
/// caller cannot construct this value; C2 adapters receive the exact C3
/// launch-record handle by borrow and must bind their own endpoint-registration
/// generation before transferring the resulting lease to C3.
#[derive(Clone)]
pub(super) struct C3IdentityLaunchBinding {
    service_id: String,
    catalog_generation: NonZeroU64,
    c3_launch_generation: NonZeroU64,
    scope_binding_digest: [u8; 32],
    process_identity: nagi_init_spawn_contract::SpawnedProcessIdentity,
    launch_record: SystemServiceLaunchRecord,
}

impl C3IdentityLaunchBinding {
    pub(super) fn service_id(&self) -> &str {
        &self.service_id
    }

    pub(super) fn catalog_generation(&self) -> NonZeroU64 {
        self.catalog_generation
    }

    pub(super) fn c3_launch_generation(&self) -> NonZeroU64 {
        self.c3_launch_generation
    }

    pub(super) fn scope_binding_digest(&self) -> &[u8; 32] {
        &self.scope_binding_digest
    }

    pub(super) fn process_identity(&self) -> &nagi_init_spawn_contract::SpawnedProcessIdentity {
        &self.process_identity
    }

    /// The exact C3-owned Supervisor record behind this binding. C2 may clone
    /// the handle into its lease; it must not re-enter the record gate from a
    /// C3 callback, and `is_live` remains a snapshot rather than authority.
    pub(super) fn launch_record(&self) -> &SystemServiceLaunchRecord {
        &self.launch_record
    }

    fn matches_launch_record(&self, record: &SystemServiceLaunchRecord) -> bool {
        let process_identity = record.process_identity();
        let Some(slot) = record.system_service_slot_identity() else {
            return false;
        };
        record.generation() == self.c3_launch_generation
            && process_identity.same_instance(&self.process_identity)
            && slot.service_id() == self.service_id
            && slot.catalog_generation() == self.catalog_generation
    }

    fn matches(&self, other: &Self) -> bool {
        self.service_id == other.service_id
            && self.catalog_generation == other.catalog_generation
            && self.c3_launch_generation == other.c3_launch_generation
            && self.scope_binding_digest == other.scope_binding_digest
            && self.process_identity.same_instance(&other.process_identity)
            && self.matches_launch_record(&self.launch_record)
            && other.matches_launch_record(&other.launch_record)
    }
}

/// C2 supplies its current pending Identity listener through this mutable
/// borrow. Binding runs after C3 has registered the exact prepared child and
/// before its exec gate can commit. On error, C2 must retain responsibility
/// for any pending state it did not transfer. C3 holds the owner lock while
/// calling this method, so it must be bounded, nonblocking, and must not
/// re-enter the C3 owner.
pub(super) trait C3PendingIdentityListener {
    fn bind_pending_before_exec(
        &mut self,
        child: nagi_init_spawn_contract::SpawnedProcessPidfd,
        binding: &C3IdentityLaunchBinding,
    ) -> io::Result<Box<dyn C3IdentityListenerLaunchLease>>;
}

/// Non-clone C2 state transferred to C3 after the pending listener is bound
/// to the exact prepared spawn. Implementations validate their own current
/// registration and endpoint generation on every phase. Binding and retirement
/// run under the C3 owner-state lock. Activation runs after releasing that lock
/// while the exact C3 launch-record gate is held. All callbacks must be bounded
/// and must not re-enter the C3 owner or its launch-record gate.
pub(super) trait C3IdentityListenerLaunchLease: Send {
    fn matches_binding(&self, binding: &C3IdentityLaunchBinding) -> bool;

    fn activate_after_exec_ack(
        &mut self,
        binding: &C3IdentityLaunchBinding,
        attestation: &FdDelegationRestrictionAttestation,
    ) -> io::Result<()>;

    /// Called only after ProcessLifecycle observed the exact child reap and
    /// C3 revoked its matching launch record. Success must make Drop inert;
    /// an error or panic is an unknown outcome and is never retried.
    fn retire_after_exact_reap(&mut self, binding: &C3IdentityLaunchBinding) -> io::Result<()>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IdentityListenerActivation {
    Pending,
    Active,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum IdentityListenerRetirement {
    NotAttempted,
    Complete,
    Unknown,
}

struct BoundIdentityListener {
    binding: C3IdentityLaunchBinding,
    lease: Arc<Mutex<Box<dyn C3IdentityListenerLaunchLease>>>,
    activation: IdentityListenerActivation,
    retirement: IdentityListenerRetirement,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaunchState {
    Prepared,
    Running,
    Superseded,
    Revoked,
    Retired,
}

struct OwnedLaunch {
    token: LaunchToken,
    process_id: u32,
    /// Stable journal operation ID assigned while the prepared child is still
    /// gated. It is distinct from the in-memory SpawnedProcessIdentity nonce.
    launch_instance_id: Option<NonZeroU64>,
    scope: RecoveryScopeSnapshot,
    scope_data_address: usize,
    launch_record: SystemServiceLaunchRecord,
    state: LaunchState,
    fd_delegation_attestation: Option<Arc<FdDelegationRestrictionAttestation>>,
    identity_listener: Option<BoundIdentityListener>,
}

/// Borrowed C3 evidence for the exact current service child. The constructor
/// remains private to this owner, and the reference is exposed only while the
/// owner holds its nonblocking current-slot check.
pub(super) struct C3FdDelegationPreventionAttestation<'a> {
    service_id: &'a str,
    catalog_generation: NonZeroU64,
    launch_generation: NonZeroU64,
    scope_binding_digest: &'a [u8; 32],
    process_identity: &'a nagi_init_spawn_contract::SpawnedProcessIdentity,
    _kernel_attestation: &'a FdDelegationRestrictionAttestation,
}

impl C3FdDelegationPreventionAttestation<'_> {
    pub(super) fn service_id(&self) -> &str {
        self.service_id
    }

    pub(super) fn catalog_generation(&self) -> NonZeroU64 {
        self.catalog_generation
    }

    pub(super) fn launch_generation(&self) -> NonZeroU64 {
        self.launch_generation
    }

    pub(super) fn scope_binding_digest(&self) -> &[u8; 32] {
        self.scope_binding_digest
    }

    pub(super) fn process_identity(&self) -> &nagi_init_spawn_contract::SpawnedProcessIdentity {
        self.process_identity
    }

    /// Borrow the underlying C3-only nondelegation attestation for C2's
    /// endpoint-registration adapter. This exposes no endpoint descriptor or
    /// principal authority; C2 must still match it against its retained spawn
    /// identity and run while this callback is active.
    pub(super) fn c2_fd_delegation_restriction(&self) -> &FdDelegationRestrictionAttestation {
        self._kernel_attestation
    }
}

struct OwnerState {
    lifecycle: ProcessLifecycle<LinuxProcessOs, String, LaunchToken>,
    launches: Vec<OwnedLaunch>,
    current: Option<LaunchToken>,
    highest_generation: u64,
    highest_catalog_generation: Option<NonZeroU64>,
    reserved_generation: Option<NonZeroU64>,
    reserved_operation: Option<C3SlotReservation>,
    highest_operation_id: u64,
    durable_journal: Option<Arc<C3SlotJournal>>,
    #[cfg(test)]
    fail_retirement_after_revoke_once: bool,
}

struct SlotOwnerRegistration {
    service_id: String,
    state: Arc<Mutex<OwnerState>>,
    claimed: bool,
    maintenance: bool,
}

static SLOT_OWNER_REGISTRY: Mutex<Vec<SlotOwnerRegistration>> = Mutex::new(Vec::new());

const DEFAULT_RETIREMENT_WAIT_BUDGET: Duration = Duration::from_secs(2);
const DEFAULT_REAPER_POLL_INTERVAL: Duration = Duration::from_millis(10);

impl C3DurableRecoverySlotReceipt for C3DurableSlotReceipt {
    fn c1_operation_id(&self) -> Option<NonZeroU64> {
        C3DurableSlotReceipt::c1_operation_id(self)
    }

    fn service_id(&self) -> &str {
        C3DurableSlotReceipt::service_id(self)
    }

    fn catalog_generation(&self) -> u64 {
        C3DurableSlotReceipt::catalog_generation(self).get()
    }

    fn c3_launch_generation(&self) -> u64 {
        C3DurableSlotReceipt::c3_launch_generation(self).get()
    }

    fn scope_binding_digest(&self) -> &[u8; 32] {
        C3DurableSlotReceipt::scope_binding_digest(self)
    }

    fn state(&self) -> C3RecoverySlotState {
        match C3DurableSlotReceipt::state(self) {
            C3SlotState::ClearedAndAbsent => C3RecoverySlotState::ClearedAndAbsent,
            C3SlotState::Live => C3RecoverySlotState::Live,
            C3SlotState::Quarantined => C3RecoverySlotState::Quarantined,
            C3SlotState::Reserved => C3RecoverySlotState::Unknown,
        }
    }
}

// A normal-exit receipt is built only from a committed ClearedAndAbsent row
// with exact launch/PID/C1 evidence. The previous-launch gate re-reads this
// same receipt under the C3 owner lock before returning it to C1.
impl C3DurableRecoverySlotReceipt for C3DurableNormalExitReceipt {
    fn c1_operation_id(&self) -> Option<NonZeroU64> {
        Some(C3DurableNormalExitReceipt::c1_operation_id(self))
    }

    fn service_id(&self) -> &str {
        C3DurableNormalExitReceipt::service_id(self)
    }

    fn catalog_generation(&self) -> u64 {
        C3DurableNormalExitReceipt::catalog_generation(self).get()
    }

    fn c3_launch_generation(&self) -> u64 {
        C3DurableNormalExitReceipt::c3_launch_generation(self).get()
    }

    fn scope_binding_digest(&self) -> &[u8; 32] {
        C3DurableNormalExitReceipt::scope_binding_digest(self)
    }

    fn state(&self) -> C3RecoverySlotState {
        C3RecoverySlotState::ClearedAndAbsent
    }
}

/// PID-1's exact owner for one canonical system-service slot.
///
/// A single owner instance represents one service slot. All operations that
/// can publish, replace, revoke, or consume its current endpoint serialize on
/// `state`; proof effects serialize on the shared `SystemServiceLaunchRecord`
/// generation gate.
pub(super) struct LinuxC3SystemServiceSlotOwner {
    service_id: String,
    state: Arc<Mutex<OwnerState>>,
}

/// One-shot C3 gate for reconciling the exact old scope selected by a
/// committed normal-exit journal receipt.
pub(super) struct C3DurableNormalExitGate<'owner, 'receipt> {
    owner: &'owner LinuxC3SystemServiceSlotOwner,
    expected: &'receipt C3DurableNormalExitReceipt,
    deadline: Option<Instant>,
    used: bool,
}

impl LinuxC3SystemServiceSlotOwner {
    pub(super) fn new(service_id: &str) -> io::Result<Self> {
        if service_id.is_empty() || service_id.as_bytes().contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "system-service slot ID must be nonempty and contain no NUL",
            ));
        }
        let service_id = service_id.to_owned();
        let existing_state = {
            let mut registry = slot_owner_registry_lock()?;
            if let Some(entry) = registry
                .iter_mut()
                .find(|entry| entry.service_id == service_id)
            {
                if entry.claimed || entry.maintenance {
                    return Err(permission_denied(
                        "a C3 owner already holds this system-service slot",
                    ));
                }
                entry.maintenance = true;
                Some(Arc::clone(&entry.state))
            } else {
                let state = Arc::new(Mutex::new(empty_owner_state()));
                registry.push(SlotOwnerRegistration {
                    service_id: service_id.clone(),
                    state: Arc::clone(&state),
                    claimed: true,
                    maintenance: false,
                });
                return Ok(Self { service_id, state });
            }
        };

        let state = existing_state.expect("existing registry entry has state");
        let cleanup = match state.lock() {
            Ok(mut state) => retire_owner_state_once(
                &mut state,
                DEFAULT_RETIREMENT_WAIT_BUDGET,
                DEFAULT_REAPER_POLL_INTERVAL,
            ),
            Err(poisoned) => {
                let mut state_guard = poisoned.into_inner();
                let cleanup = retire_owner_state_once(
                    &mut state_guard,
                    DEFAULT_RETIREMENT_WAIT_BUDGET,
                    DEFAULT_REAPER_POLL_INTERVAL,
                )
                .and_then(|()| validate_quiescent_retired_state(&state_guard));
                if cleanup.is_ok() {
                    // Clear poison only after exact children and launch records
                    // are gone; high-water generations remain in this state.
                    state.clear_poison();
                }
                cleanup
            }
        };
        let mut registry = slot_owner_registry_lock()?;
        let entry = registry
            .iter_mut()
            .find(|entry| entry.service_id == service_id)
            .ok_or_else(|| io::Error::other("C3 slot registry entry disappeared"))?;
        if !Arc::ptr_eq(&entry.state, &state) {
            return Err(io::Error::other("C3 slot registry identity changed"));
        }
        entry.maintenance = false;
        cleanup?;
        if entry.claimed {
            return Err(permission_denied(
                "a C3 owner already holds this system-service slot",
            ));
        }
        entry.claimed = true;
        Ok(Self { service_id, state })
    }

    /// Open the maintained owner against its private file-backed C3 slot
    /// journal. Normal startup requires a committed absent row. A development
    /// build may create genesis only when an exact first-provision marker is
    /// present and both journal paths are absent. A live or unknown row is
    /// never reconstructed as an in-memory child.
    pub(super) fn new_with_journal(
        service_id: &str,
        journal_path: impl AsRef<Path>,
    ) -> io::Result<Self> {
        let journal_path = journal_path.as_ref();
        let (state, fresh_registration) = reserve_slot_owner_registration(service_id)?;
        let opened_journal = (|| {
            if fresh_registration {
                #[cfg(not(nagi_release_build))]
                if let Some(journal) =
                    C3SlotJournal::create_from_first_provision_marker(journal_path, service_id)?
                {
                    return Ok((Arc::new(journal), true));
                }
            }
            // A registered or previously provisioned slot is open-only. The
            // first-provision marker is considered only after this owner has
            // reserved a fresh service slot in the PID-1 registry.
            C3SlotJournal::open(journal_path, service_id).map(|journal| (Arc::new(journal), false))
        })();
        let (journal, allow_genesis_only) = match opened_journal {
            Ok(opened) => opened,
            Err(error) => {
                let release_fresh_reservation = fresh_registration
                    && matches!(
                        error,
                        C3SlotJournalError::InvalidInput | C3SlotJournalError::RejectedBeforeWrite
                    );
                let error = c3_journal_io_error(error);
                abandon_slot_owner_registration(
                    service_id,
                    &state,
                    fresh_registration,
                    release_fresh_reservation,
                );
                return Err(error);
            }
        };
        Self::finish_open_journal_registration(
            service_id,
            journal,
            allow_genesis_only,
            state,
            fresh_registration,
        )
    }

    #[cfg(test)]
    fn create_with_journal(service_id: &str, journal_path: impl AsRef<Path>) -> io::Result<Self> {
        let (state, fresh_registration) = reserve_slot_owner_registration(service_id)?;
        if !fresh_registration {
            let error = permission_denied("a test journal requires a fresh C3 slot");
            abandon_slot_owner_registration(service_id, &state, fresh_registration, false);
            return Err(error);
        }
        let journal = match C3SlotJournal::create(journal_path, service_id) {
            Ok(journal) => Arc::new(journal),
            Err(error) => {
                let release_fresh_reservation = matches!(
                    error,
                    C3SlotJournalError::InvalidInput | C3SlotJournalError::RejectedBeforeWrite
                );
                let error = c3_journal_io_error(error);
                abandon_slot_owner_registration(
                    service_id,
                    &state,
                    fresh_registration,
                    release_fresh_reservation,
                );
                return Err(error);
            }
        };
        Self::finish_open_journal_registration(service_id, journal, true, state, fresh_registration)
    }

    fn finish_open_journal_registration(
        service_id: &str,
        journal: Arc<C3SlotJournal>,
        allow_genesis_only: bool,
        state: Arc<Mutex<OwnerState>>,
        fresh_registration: bool,
    ) -> io::Result<Self> {
        let result = Self::from_open_journal(
            service_id,
            journal,
            allow_genesis_only,
            Arc::clone(&state),
            fresh_registration,
        );
        if result.is_err() {
            abandon_slot_owner_registration(service_id, &state, fresh_registration, false);
        }
        result
    }

    fn from_open_journal(
        service_id: &str,
        journal: Arc<C3SlotJournal>,
        allow_genesis_only: bool,
        state: Arc<Mutex<OwnerState>>,
        fresh_registration: bool,
    ) -> io::Result<Self> {
        let (highest_catalog_generation, highest_c3_launch_generation, highest_operation_id) =
            match journal.readback() {
                C3SlotJournalReadback::Empty if allow_genesis_only => (0, 0, 0),
                C3SlotJournalReadback::Empty => {
                    return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
                }
                C3SlotJournalReadback::Committed(row)
                    if row.service_id() == service_id
                        && row.state() == C3SlotState::ClearedAndAbsent =>
                {
                    (
                        row.highest_catalog_generation(),
                        row.highest_c3_launch_generation(),
                        row.highest_operation_id(),
                    )
                }
                C3SlotJournalReadback::Committed(_) | C3SlotJournalReadback::Unknown => {
                    return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
                }
            };

        let mut state_guard = state.lock().map_err(|_| {
            io::Error::other("reserved C3 owner state is poisoned; journal open is unknown")
        })?;
        let state_matches_registration = if fresh_registration {
            state_guard.durable_journal.is_none()
                && state_guard.highest_generation == 0
                && state_guard.highest_catalog_generation.is_none()
                && state_guard.highest_operation_id == 0
                && state_guard.reserved_generation.is_none()
                && state_guard.reserved_operation.is_none()
                && validate_quiescent_retired_state(&state_guard).is_ok()
        } else {
            state_guard
                .durable_journal
                .as_ref()
                .is_none_or(|registered| {
                    registered.same_selected_location(&journal).unwrap_or(false)
                })
                && state_guard.highest_generation == highest_c3_launch_generation
                && state_guard
                    .highest_catalog_generation
                    .map_or(0, NonZeroU64::get)
                    == highest_catalog_generation
                && state_guard.highest_operation_id == highest_operation_id
                && state_guard.reserved_operation.is_none()
                && validate_quiescent_retired_state(&state_guard).is_ok()
        };
        if !state_matches_registration {
            return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
        }

        let mut registry = slot_owner_registry_lock()?;
        let Some(entry) = registry
            .iter_mut()
            .find(|entry| entry.service_id == service_id && Arc::ptr_eq(&entry.state, &state))
        else {
            return Err(io::Error::other(
                "reserved C3 registry identity disappeared during journal open",
            ));
        };
        if entry.claimed || !entry.maintenance {
            return Err(permission_denied(
                "reserved C3 slot changed while opening its journal",
            ));
        }
        if fresh_registration {
            state_guard.highest_catalog_generation = NonZeroU64::new(highest_catalog_generation);
            state_guard.highest_generation = highest_c3_launch_generation;
            state_guard.highest_operation_id = highest_operation_id;
        }
        state_guard.durable_journal = Some(journal);
        entry.claimed = true;
        entry.maintenance = false;
        drop(registry);
        drop(state_guard);
        Ok(Self {
            service_id: service_id.to_owned(),
            state,
        })
    }

    /// Retry the exact PID-1 cleanup retained after an owner was dropped.
    /// The service ID stays reserved until its child and launch records retire.
    pub(super) fn reap_unowned_slot(
        service_id: &str,
        wait_budget: Duration,
        poll_interval: Duration,
    ) -> io::Result<()> {
        validate_reaper_wait(wait_budget, poll_interval)?;
        let deadline = std::time::Instant::now()
            .checked_add(wait_budget)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "system-service retirement wait budget is too large",
                )
            })?;
        Self::reap_unowned_slot_until(service_id, deadline, poll_interval, &Instant::now)
    }

    fn reap_unowned_slot_until(
        service_id: &str,
        deadline: Instant,
        poll_interval: Duration,
        now: &impl Fn() -> Instant,
    ) -> io::Result<()> {
        let state = {
            let mut registry = slot_owner_registry_lock()?;
            let Some(entry) = registry
                .iter_mut()
                .find(|entry| entry.service_id == service_id)
            else {
                drop(registry);
                retirement_budget_remaining_at(deadline, now())?;
                return Ok(());
            };
            if entry.claimed || entry.maintenance {
                return Err(permission_denied(
                    "the system-service slot is held by an active C3 owner",
                ));
            }
            entry.maintenance = true;
            Arc::clone(&entry.state)
        };

        let result = loop {
            let mut guard = lock_owner_state_for_cleanup(&state);
            let remaining = deadline.saturating_duration_since(now());
            if remaining.is_zero() {
                break Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "PID-1 still owns the system-service child after its retirement budget",
                ));
            }
            let attempt = retire_owner_state_until(&mut guard, deadline, poll_interval)
                .and_then(|()| validate_quiescent_retired_state(&guard));
            match attempt {
                Ok(()) => break Ok(()),
                Err(error) => {
                    let remaining = deadline.saturating_duration_since(now());
                    if remaining.is_zero() {
                        break Err(error);
                    }
                    std::thread::sleep(poll_interval.min(remaining));
                }
            }
        };

        let mut registry = slot_owner_registry_lock()?;
        if let Some(entry) = registry
            .iter_mut()
            .find(|entry| entry.service_id == service_id && Arc::ptr_eq(&entry.state, &state))
        {
            entry.maintenance = false;
        }
        drop(registry);
        match result {
            Ok(()) => retirement_budget_remaining_at(deadline, now()).map(|_| ()),
            Err(error) => Err(error),
        }
    }

    /// Retire every tracked child before this PID-1 owner can be reused.
    pub(super) fn shutdown(
        &self,
        wait_budget: Duration,
        poll_interval: Duration,
    ) -> io::Result<()> {
        validate_reaper_wait(wait_budget, poll_interval)?;
        let deadline = std::time::Instant::now()
            .checked_add(wait_budget)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "system-service retirement wait budget is too large",
                )
            })?;
        self.shutdown_until(deadline, poll_interval, &Instant::now)
    }

    fn shutdown_until(
        &self,
        deadline: Instant,
        poll_interval: Duration,
        now: &impl Fn() -> Instant,
    ) -> io::Result<()> {
        loop {
            let remaining = deadline.saturating_duration_since(now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "PID-1 still owns the system-service child after its retirement budget",
                ));
            }
            let attempt = {
                let mut state = self.lock_state_for_cleanup();
                retire_owner_state_until(&mut state, deadline, poll_interval)
                    .and_then(|()| validate_quiescent_retired_state(&state))
            };
            match attempt {
                Ok(()) => {
                    retirement_budget_remaining_at(deadline, now())?;
                    return Ok(());
                }
                Err(error) => {
                    let remaining = deadline.saturating_duration_since(now());
                    if remaining.is_zero() {
                        return Err(error);
                    }
                    std::thread::sleep(poll_interval.min(remaining));
                }
            }
        }
    }

    /// Allocate the next C3-owned process generation before A creates the
    /// immutable admission view that carries it.
    pub(super) fn reserve_launch_generation(&self) -> io::Result<NonZeroU64> {
        let mut state = self.lock_state()?;
        if state.reserved_generation.is_some() {
            return Err(permission_denied(
                "a system-service launch generation is already reserved",
            ));
        }
        let (next, reservation) = if let Some(journal) = state.durable_journal.as_ref() {
            let reservation = journal
                .reserve_next_generation()
                .map_err(c3_journal_io_error)?;
            (reservation.launch_generation(), Some(reservation))
        } else {
            let next = state
                .highest_generation
                .checked_add(1)
                .and_then(NonZeroU64::new)
                .ok_or_else(|| {
                    io::Error::other("C3 system-service launch generation is exhausted")
                })?;
            (next, None)
        };
        state.highest_generation = next.get();
        state.reserved_generation = Some(next);
        if let Some(reservation) = reservation.as_ref() {
            state.highest_operation_id = state
                .highest_operation_id
                .max(reservation.operation_id().get());
        }
        state.reserved_operation = reservation;
        Ok(next)
    }

    /// Spawn and register the exact child described by one A-issued
    /// admission. The caller supplies every release/cancellation deadline;
    /// this owner invents no product timeout.
    ///
    /// # Safety
    /// `scope` must be the exact immutable private A admission produced from
    /// the trusted catalog, installed signed package, and this owner's slot
    /// reservation. `executable` must be the exact executable slice covered by
    /// that admission. The admission reference must remain the same object
    /// through endpoint issuance and any later C1/C3 reconciliation.
    pub(super) unsafe fn launch_admitted(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        executable: &[u8],
        arguments: &[&str],
        policy: C3LaunchPolicy,
        cancelled: impl FnMut() -> bool,
    ) -> io::Result<()> {
        unsafe { self.launch_admitted_inner(scope, executable, arguments, policy, None, cancelled) }
    }

    /// C3-owned standalone Identity path. C2 lends its current pending
    /// listener-registration owner by mutable borrow; C3 binds it to the
    /// prepared clone3 identity before the exec gate and retains the verified
    /// FD attestation after exec acknowledgement. The outer authenticated
    /// entry point then activates the listener outside the owner-state lock
    /// under the same C3 launch-record gate, and retirement remains after
    /// exact reap and C3 launch-record revocation.
    ///
    /// # Safety
    /// The `scope`, executable, and lifetime requirements are identical to
    /// `launch_admitted`. The listener adapter must represent the current C2
    /// registration for this exact service launch and obey the phase contract
    /// on `C3PendingIdentityListener` and `C3IdentityListenerLaunchLease`.
    pub(super) unsafe fn launch_admitted_with_identity_listener(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        executable: &[u8],
        arguments: &[&str],
        policy: C3LaunchPolicy,
        pending_listener: &mut dyn C3PendingIdentityListener,
        cancelled: impl FnMut() -> bool,
    ) -> io::Result<()> {
        unsafe {
            self.launch_admitted_inner(
                scope,
                executable,
                arguments,
                policy,
                Some(pending_listener),
                cancelled,
            )
        }
    }

    unsafe fn launch_admitted_inner(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        executable: &[u8],
        arguments: &[&str],
        policy: C3LaunchPolicy,
        mut identity_listener_registration: Option<&mut dyn C3PendingIdentityListener>,
        mut cancelled: impl FnMut() -> bool,
    ) -> io::Result<()> {
        let C3LaunchPolicy {
            required,
            release_wait_budget,
            cancel_check_interval,
            retirement_wait_budget,
            reaper_poll_interval,
        } = policy;
        if release_wait_budget.is_zero()
            || cancel_check_interval.is_zero()
            || retirement_wait_budget.is_zero()
            || reaper_poll_interval.is_zero()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "launch, retirement, and reaper intervals must be nonzero",
            ));
        }
        let scope_snapshot = capture_scope(scope)?;
        validate_scope_for_slot(&scope_snapshot, &self.service_id)?;
        let scope_data_address = scope_data_address(scope);
        let executable_sha256: [u8; 32] = Sha256::digest(executable).into();
        if executable_sha256 != scope_snapshot.executable_sha256 {
            return Err(permission_denied(
                "executable bytes do not match the A-admitted SHA-256 digest",
            ));
        }
        let executable = ExecutablePayload::from_package_executable(executable).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "admitted system-service executable is not a static x86_64 ELF",
            )
        })?;

        let mut state = self.lock_state()?;
        let reserved = state.reserved_generation.ok_or_else(|| {
            permission_denied("system-service launch has no C3 generation reservation")
        })?;
        let reserved_operation = state.reserved_operation.clone();
        let launch_instance_id = reserved_operation
            .as_ref()
            .map(C3SlotReservation::operation_id);
        if state.durable_journal.is_some() && reserved_operation.is_none() {
            return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
        }
        if reserved != scope_snapshot.c3_launch_generation {
            return Err(permission_denied(
                "A admission does not carry this C3 slot's exact reserved generation",
            ));
        }
        let (previous_token, previous_process_id) = {
            let previous = current_launch(&state)?;
            validate_transition(
                &scope_snapshot,
                previous,
                state.highest_catalog_generation,
                state.current.is_none()
                    && state.launches.is_empty()
                    && state.lifecycle.active_len() == 0,
                reserved_operation.as_ref(),
            )?;
            previous
                .map(|entry| (Some(entry.token), Some(entry.process_id)))
                .unwrap_or((None, None))
        };
        let token = LaunchToken(scope_snapshot.c3_launch_generation);
        let launch = ProcessLaunch {
            service: self.service_id.clone(),
            executable,
            arguments,
            execution_identity: LinuxExecutionIdentity {
                uid: scope_snapshot.uid,
                gid: scope_snapshot.gid,
            },
            endpoint_policy: ProcessEndpointPolicy::InheritedPeerChannel,
            required,
            release_wait_budget,
            cancel_check_interval,
        };

        let journal = state.durable_journal.clone();
        let run_launch = |state: &mut OwnerState,
                          stage_launch_identity: &mut dyn FnMut(u32) -> io::Result<()>|
         -> io::Result<()> {
            state.reserved_generation = None;
            state.reserved_operation = None;

            let mut replacement_committed = false;
            let mut cancellation_panicked = false;
            let mut cancellation = || match catch_unwind(AssertUnwindSafe(&mut cancelled)) {
                Ok(cancelled) => cancelled,
                Err(_) => {
                    cancellation_panicked = true;
                    true
                }
            };

            let mut spawn_result = {
                let OwnerState {
                    lifecycle,
                    launches,
                    current,
                    highest_catalog_generation,
                    ..
                } = &mut *state;
                let launches = RefCell::new(launches);
                let current = RefCell::new(current);
                let highest_catalog_generation = RefCell::new(highest_catalog_generation);
                lifecycle.spawn_authorized_with_prepared(
                    launch,
                    &mut cancellation,
                    |process_id, os, prepared| {
                        validate_same_scope(
                            scope,
                            &scope_snapshot,
                            scope_data_address,
                            &self.service_id,
                        )
                        .map_err(|_| BackendError::Revoked)?;
                        let mut launches = launches.borrow_mut();
                        let mut current = current.borrow_mut();
                        let mut highest_catalog_generation =
                            highest_catalog_generation.borrow_mut();
                        register_prepared_launch(
                            &mut launches,
                            &mut current,
                            os,
                            prepared,
                            PreparedLaunchRegistration {
                                process_id,
                                token,
                                service_id: &self.service_id,
                                scope: scope_snapshot.clone(),
                                launch_instance_id,
                                scope_data_address,
                                previous_token,
                            },
                            &mut highest_catalog_generation,
                            &mut replacement_committed,
                        )
                    },
                    |process_id, registered, gate| {
                        let mut launches = launches.borrow_mut();
                        let current = current.borrow_mut();
                        if *registered != token
                            || !is_current_launch(&launches, **current, process_id, token)
                        {
                            return Err(BackendError::Revoked);
                        }
                        validate_same_scope(
                            scope,
                            &scope_snapshot,
                            scope_data_address,
                            &self.service_id,
                        )
                        .map_err(|_| BackendError::Revoked)?;
                        if let Some(registration) = identity_listener_registration.as_mut() {
                            let prepared_entry = find_launch(&launches, token)
                                .filter(|entry| entry.process_id == process_id)
                                .ok_or(BackendError::ProtocolViolation)?;
                            if prepared_entry.identity_listener.is_some() {
                                return Err(BackendError::ProtocolViolation);
                            }
                            let binding = identity_launch_binding(prepared_entry);
                            let child = gate.duplicate_prepared_spawn_pidfd()?;
                            if !child.identity().same_instance(binding.process_identity()) {
                                return Err(BackendError::IdentityRejected);
                            }
                            let lease = match catch_unwind(AssertUnwindSafe(|| {
                                registration.bind_pending_before_exec(child, &binding)
                            })) {
                                Ok(Ok(lease)) => lease,
                                Ok(Err(_)) => return Err(BackendError::Unavailable),
                                Err(_) => return Err(BackendError::TransportFailure),
                            };
                            let matches_binding =
                                catch_unwind(AssertUnwindSafe(|| lease.matches_binding(&binding)))
                                    .unwrap_or(false);
                            let entry = find_launch_mut(&mut launches, token, process_id)
                                .ok_or(BackendError::ProtocolViolation)?;
                            entry.identity_listener = Some(BoundIdentityListener {
                                binding,
                                lease: Arc::new(Mutex::new(lease)),
                                activation: IdentityListenerActivation::Pending,
                                retirement: IdentityListenerRetirement::NotAttempted,
                            });
                            if !matches_binding {
                                return Err(BackendError::IdentityRejected);
                            }
                        }
                        if let Some(launch_instance_id) = launch_instance_id {
                            let entry = find_launch(&launches, token)
                                .filter(|entry| entry.process_id == process_id)
                                .ok_or(BackendError::ProtocolViolation)?;
                            if entry.launch_instance_id != Some(launch_instance_id)
                                || entry.state != LaunchState::Prepared
                                || !entry.launch_record.is_live()
                                || entry.launch_record.process_identity().process_id() != process_id
                            {
                                return Err(BackendError::ProtocolViolation);
                            }
                            stage_launch_identity(process_id)
                                .map_err(|_| BackendError::TransportFailure)?;
                        }
                        gate.commit()?;
                        if let Some(entry) = find_launch_mut(&mut launches, token, process_id) {
                            entry.state = LaunchState::Running;
                        } else {
                            return Err(BackendError::ProtocolViolation);
                        }
                        Ok(())
                    },
                    |process_id, registered| {
                        let mut launches = launches.borrow_mut();
                        let mut current = current.borrow_mut();
                        revoke_and_retire_identity_listener(
                            &mut launches,
                            &mut current,
                            process_id,
                            *registered,
                        )
                    },
                )
            };

            if let Some(process_id) = spawn_result.as_ref().ok().copied() {
                let attestation = state
                    .lifecycle
                    .take_live_fd_delegation_restriction_attestation(process_id, token);
                let retained = match attestation {
                    Ok(attestation) => find_launch_mut(&mut state.launches, token, process_id)
                        .is_some_and(|entry| {
                            let process_identity = entry.launch_record.process_identity();
                            if !attestation.matches_process(&process_identity) {
                                return false;
                            }
                            entry.fd_delegation_attestation = Some(Arc::new(attestation));
                            true
                        }),
                    Err(_) => false,
                };
                if !retained {
                    if let Ok(Some((failed_pid, failed_token))) = revoke_current_record(state) {
                        let _ = terminate_launch_bounded(
                            state,
                            failed_pid,
                            failed_token,
                            retirement_wait_budget,
                            reaper_poll_interval,
                        );
                    }
                    spawn_result = Err(BackendError::ProtocolViolation);
                }
            }

            if let Some(previous_token) = previous_token {
                let previous_process_id = previous_process_id.expect("previous token has a PID");
                if spawn_result.is_err() && !replacement_committed {
                    return Err(backend_error(spawn_result.expect_err("checked error")));
                }
                if let Err(error) = terminate_launch_bounded(
                    state,
                    previous_process_id,
                    previous_token,
                    retirement_wait_budget,
                    reaper_poll_interval,
                ) {
                    // The previous proof was already invalidated by replacement.
                    // Retire the new slot too; the journal remains pending if
                    // exact process cleanup cannot finish.
                    if let Ok(Some((new_pid, new_token))) = revoke_current_record(state) {
                        let _ = terminate_launch_bounded(
                            state,
                            new_pid,
                            new_token,
                            retirement_wait_budget,
                            reaper_poll_interval,
                        );
                    }
                    return Err(error);
                }
            }

            if cancellation_panicked {
                if let Ok(Some((process_id, token))) = revoke_current_record(state) {
                    let _ = terminate_launch_bounded(
                        state,
                        process_id,
                        token,
                        retirement_wait_budget,
                        reaper_poll_interval,
                    );
                }
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "system-service launch cancellation callback panicked",
                ));
            }

            match spawn_result {
                Ok(_) => Ok(()),
                Err(error) => Err(io::Error::new(
                    backend_error(error).kind(),
                    format!("C3 system-service lifecycle failed: {error:?}"),
                )),
            }
        };

        if let Some(journal) = journal {
            let reservation = reserved_operation
                .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
            let mut effect_ran = false;
            let mut effect_error = None;
            let result = journal.transition_reserved_launch(
                reservation,
                scope_snapshot.catalog_generation,
                scope_snapshot.binding_digest,
                scope_snapshot.clone(),
                |stage_launch_identity| {
                    effect_ran = true;
                    match run_launch(&mut state, stage_launch_identity) {
                        Ok(()) => match registered_launch_process_id(
                            &state,
                            &scope_snapshot,
                            &self.service_id,
                        ) {
                            Ok(process_id) => Ok(process_id),
                            Err(error) => {
                                effect_error = Some(format!("{:?}: {error}", error.kind()));
                                Err(error)
                            }
                        },
                        Err(error) => {
                            effect_error = Some(format!("{:?}: {error}", error.kind()));
                            Err(error)
                        }
                    }
                },
            );
            match result {
                Ok(receipt)
                    if exact_durable_receipt(
                        &receipt,
                        &self.service_id,
                        scope_snapshot.catalog_generation,
                        scope_snapshot.c3_launch_generation,
                        &scope_snapshot.binding_digest,
                        C3SlotState::Live,
                    ) =>
                {
                    state.highest_operation_id =
                        state.highest_operation_id.max(receipt.operation_id().get());
                    Ok(())
                }
                Ok(_) => {
                    let cleanup = retire_owner_state_effect(
                        &mut state,
                        retirement_wait_budget,
                        reaper_poll_interval,
                    );
                    cleanup?;
                    Err(c3_journal_io_error(C3SlotJournalError::Unknown))
                }
                Err(error) => {
                    if effect_ran {
                        retire_owner_state_effect(
                            &mut state,
                            retirement_wait_budget,
                            reaper_poll_interval,
                        )?;
                    }
                    if let Some(effect_error) = effect_error {
                        Err(io::Error::other(format!(
                            "C3 slot journal outcome is unknown after effect failure: {effect_error}"
                        )))
                    } else {
                        Err(c3_journal_io_error(error))
                    }
                }
            }
        } else {
            run_launch(&mut state, &mut |_| Ok(()))
        }
    }

    /// Launch the executable admitted by the exact A scope, take the child’s
    /// inherited C3 control endpoint, and authenticate that endpoint against
    /// the live child record before returning. This does not bind a C2 service
    /// listener. Standalone Identity callers that supply one must use
    /// `launch_and_authenticate_admitted_with_identity_listener`. The C3 owner
    /// retains the child and its pidfd after the proof is issued; the caller
    /// must keep the same scope object alive for later effects and
    /// reconciliation.
    ///
    /// # Safety
    /// `scope` must be the exact immutable scope view from A's private
    /// `SystemServiceRecoveryAdmission`, and `executable` must be the exact
    /// executable bytes from the verified package bound into that scope. The
    /// caller must keep that same admission object alive through all later C1
    /// and C3 reconciliation.
    pub(super) unsafe fn launch_and_authenticate_admitted(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        executable: &[u8],
        arguments: &[&str],
        policy: C3LaunchPolicy,
        cancelled: impl FnMut() -> bool,
    ) -> io::Result<SystemServiceEndpointOwnerProof> {
        unsafe {
            self.launch_admitted(scope, executable, arguments, policy, cancelled)?;
        }
        let endpoint = self.take_current_service_endpoint(scope)?;
        self.issue_current_service_endpoint(scope, endpoint)
    }

    /// Launch and authenticate the inherited C3 control endpoint while also
    /// binding C2's borrowed current listener registration through the exact
    /// pre-exec, attested activation, and post-reap retirement phases.
    /// Standalone Identity readiness callers must use this entry point when
    /// they have a C2 service listener; the control endpoint remains separate.
    ///
    /// # Safety
    /// The scope, executable, and admission-lifetime requirements are the same
    /// as `launch_and_authenticate_admitted`. `pending_listener` must borrow
    /// the current C2 pending registration for this exact Identity launch.
    pub(super) unsafe fn launch_and_authenticate_admitted_with_identity_listener(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        executable: &[u8],
        arguments: &[&str],
        policy: C3LaunchPolicy,
        pending_listener: &mut dyn C3PendingIdentityListener,
        cancelled: impl FnMut() -> bool,
    ) -> io::Result<SystemServiceEndpointOwnerProof> {
        unsafe {
            self.launch_admitted_with_identity_listener(
                scope,
                executable,
                arguments,
                policy,
                pending_listener,
                cancelled,
            )?;
        }
        let endpoint = self.take_current_service_endpoint(scope)?;
        let proof = self.issue_current_service_endpoint(scope, endpoint)?;
        self.activate_current_identity_listener_after_exec_ack(
            scope,
            &proof,
            policy.retirement_wait_budget,
            policy.reaper_poll_interval,
        )?;
        Ok(proof)
    }

    /// Consume C2's retained launch lease after C3 has authenticated its own
    /// exact control endpoint. The owner-state lock protects only snapshot and
    /// phase transitions; the callback runs under the shared C3 launch-record
    /// gate so replacement/reap cannot revoke the admitted child mid-effect.
    fn activate_current_identity_listener_after_exec_ack(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        proof: &SystemServiceEndpointOwnerProof,
        retirement_wait_budget: Duration,
        reaper_poll_interval: Duration,
    ) -> io::Result<()> {
        let scope_snapshot = capture_scope(scope)?;
        validate_scope_for_slot(&scope_snapshot, &self.service_id)?;
        let scope_address = scope_data_address(scope);
        let (token, process_id, binding, lease, attestation) = {
            let mut state = self.lock_state()?;
            let token = state
                .current
                .ok_or_else(|| permission_denied("system-service slot has no current launch"))?;
            let entry = find_launch(&state.launches, token).ok_or_else(|| {
                permission_denied("current system-service launch record is missing")
            })?;
            validate_entry_scope(
                entry,
                scope,
                &scope_snapshot,
                scope_address,
                &self.service_id,
            )?;
            let proof_identity = proof.process_identity();
            let proof_slot = proof.service_slot_identity();
            if entry.state != LaunchState::Running
                || !entry.launch_record.is_live()
                || !entry
                    .launch_record
                    .process_identity()
                    .same_instance(&proof_identity)
                || proof.launch_generation() != token.0
                || proof_slot.service_id() != entry.scope.service_id
                || proof_slot.catalog_generation() != entry.scope.catalog_generation
            {
                return Err(permission_denied(
                    "C3 endpoint proof does not match the current listener launch",
                ));
            }
            let binding = entry
                .identity_listener
                .as_ref()
                .map(|bound| bound.binding.clone())
                .ok_or_else(|| permission_denied("current launch has no C2 listener lease"))?;
            if !identity_binding_matches_launch(&binding, entry) {
                return Err(permission_denied(
                    "C2 listener binding does not match the current C3 launch",
                ));
            }
            let attestation = entry
                .fd_delegation_attestation
                .as_ref()
                .map(Arc::clone)
                .ok_or_else(|| permission_denied("C3 launch has no FD restriction attestation"))?;
            if !attestation.matches_process(binding.process_identity()) {
                return Err(permission_denied(
                    "C3 FD restriction attestation belongs to another process",
                ));
            }
            let process_id = entry.process_id;
            let lease = Arc::clone(
                &entry
                    .identity_listener
                    .as_ref()
                    .expect("listener binding was checked above")
                    .lease,
            );
            let bound = find_launch_mut(&mut state.launches, token, process_id)
                .and_then(|entry| entry.identity_listener.as_mut())
                .ok_or_else(|| permission_denied("current C2 listener lease is missing"))?;
            if bound.activation != IdentityListenerActivation::Pending
                || bound.retirement != IdentityListenerRetirement::NotAttempted
            {
                return Err(permission_denied(
                    "C2 listener activation is not in its pending phase",
                ));
            }
            // A callback error or panic has an unknown publication result and
            // is never retried by another launch/reconcile pass.
            bound.activation = IdentityListenerActivation::Unknown;
            (token, process_id, binding, lease, attestation)
        };

        let activation = catch_unwind(AssertUnwindSafe(|| {
            proof.with_live_authority_effect(|| {
                let mut lease = lease
                    .lock()
                    .map_err(|_| io::Error::other("C3 Identity listener lease is poisoned"))?;
                if !lease.matches_binding(&binding) {
                    return Err(permission_denied(
                        "C2 listener lease no longer matches its C3 launch binding",
                    ));
                }
                lease.activate_after_exec_ack(&binding, attestation.as_ref())
            })
        }));
        let activation_error = match activation {
            Ok(Ok(Ok(()))) => None,
            Ok(Ok(Err(error))) | Ok(Err(error)) => Some(error),
            Err(_) => Some(io::Error::other("C2 listener activation callback panicked")),
        };
        if let Some(error) = activation_error {
            self.abort_identity_listener_activation(
                token,
                process_id,
                retirement_wait_budget,
                reaper_poll_interval,
            );
            return Err(error);
        }
        match proof.is_live() {
            Ok(true) => {}
            Ok(false) => {
                self.abort_identity_listener_activation(
                    token,
                    process_id,
                    retirement_wait_budget,
                    reaper_poll_interval,
                );
                return Err(permission_denied(
                    "C3 listener launch ceased to be live during activation",
                ));
            }
            Err(error) => {
                self.abort_identity_listener_activation(
                    token,
                    process_id,
                    retirement_wait_budget,
                    reaper_poll_interval,
                );
                return Err(error);
            }
        }

        let recorded = (|| {
            let mut state = self.lock_state()?;
            if state.current != Some(token) {
                return Err(permission_denied(
                    "C3 listener launch was retired during activation",
                ));
            }
            let entry = find_launch_mut(&mut state.launches, token, process_id)
                .filter(|entry| entry.process_id == process_id)
                .ok_or_else(|| {
                    permission_denied("C3 listener launch was retired during activation")
                })?;
            if entry.state != LaunchState::Running
                || !entry.launch_record.is_live()
                || !identity_binding_matches_launch(&binding, entry)
            {
                return Err(permission_denied(
                    "C3 launch changed before listener activation could be recorded",
                ));
            }
            let bound = entry.identity_listener.as_mut().ok_or_else(|| {
                permission_denied("C3 listener lease was retired during activation")
            })?;
            if bound.activation != IdentityListenerActivation::Unknown
                || bound.retirement != IdentityListenerRetirement::NotAttempted
            {
                return Err(permission_denied(
                    "C3 listener activation state changed during callback",
                ));
            }
            bound.activation = IdentityListenerActivation::Active;
            Ok(())
        })();
        if let Err(error) = recorded {
            self.abort_identity_listener_activation(
                token,
                process_id,
                retirement_wait_budget,
                reaper_poll_interval,
            );
            return Err(error);
        }
        Ok(())
    }

    fn abort_identity_listener_activation(
        &self,
        token: LaunchToken,
        process_id: u32,
        retirement_wait_budget: Duration,
        reaper_poll_interval: Duration,
    ) {
        let mut state = self.lock_state_for_cleanup();
        if state.current != Some(token)
            || find_launch(&state.launches, token)
                .is_none_or(|entry| entry.process_id != process_id)
        {
            return;
        }
        if let Ok(Some((failed_process_id, failed_token))) = revoke_current_record(&mut state) {
            let _ = terminate_launch_bounded(
                &mut state,
                failed_process_id,
                failed_token,
                retirement_wait_budget,
                reaper_poll_interval,
            );
        }
    }

    /// Read the process identity from the registered launch for A's
    /// independent endpoint-proof comparison.
    pub(super) fn registered_process_identity(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
    ) -> io::Result<nagi_init_spawn_contract::SpawnedProcessIdentity> {
        let scope_snapshot = capture_scope(scope)?;
        let scope_address = scope_data_address(scope);
        let state = self.lock_state()?;
        let entry = current_launch(&state)?
            .ok_or_else(|| permission_denied("system-service slot has no current launch"))?;
        validate_entry_scope(
            entry,
            scope,
            &scope_snapshot,
            scope_address,
            &self.service_id,
        )?;
        Ok(entry.launch_record.process_identity())
    }

    /// Borrow the real C3 endpoint nondelegation attestation only for the
    /// current, unreaped clone3 process and exact A admission. The owner uses
    /// `try_lock`, checks the live pidfd without waiting, and keeps the slot
    /// locked while invoking a bounded adapter. A C2 adapter may register its
    /// own listener against this exact spawn; it must not perform blocking I/O
    /// or re-enter this owner. This check does not expose or replace the
    /// separate PID 1 control endpoint, and it does not perform A/C1's
    /// after-login Session revalidation.
    pub(super) fn with_current_fd_delegation_prevention<R>(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        expected_process_identity: &nagi_init_spawn_contract::SpawnedProcessIdentity,
        consume: impl FnOnce(&C3FdDelegationPreventionAttestation<'_>) -> R,
    ) -> io::Result<R> {
        let scope_snapshot = capture_scope(scope)?;
        validate_scope_for_slot(&scope_snapshot, &self.service_id)?;
        let scope_address = scope_data_address(scope);
        let state = match self.state.try_lock() {
            Ok(state) => state,
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "C3 system-service slot is busy",
                ));
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                return Err(io::Error::other("C3 system-service slot owner is poisoned"));
            }
        };
        let token = state
            .current
            .ok_or_else(|| permission_denied("system-service slot has no current launch"))?;
        let entry = find_launch(&state.launches, token)
            .ok_or_else(|| permission_denied("current system-service launch record is missing"))?;
        validate_entry_scope(
            entry,
            scope,
            &scope_snapshot,
            scope_address,
            &self.service_id,
        )?;
        if entry.state != LaunchState::Running
            || token.0 != scope_snapshot.c3_launch_generation
            || state.lifecycle.live_record(entry.process_id) != Some(&token)
            || !state.lifecycle.is_live_record(entry.process_id, token)?
        {
            return Err(permission_denied(
                "C3 endpoint delegation evidence is not for the live current child",
            ));
        }
        let restriction = entry.fd_delegation_attestation.as_ref().ok_or_else(|| {
            permission_denied("C3 child has no verified FD delegation restriction")
        })?;
        let process_identity = restriction.process_identity();
        if !restriction.matches_process(expected_process_identity)
            || !entry
                .launch_record
                .process_identity()
                .same_instance(expected_process_identity)
        {
            return Err(permission_denied(
                "FD delegation evidence belongs to a different clone3 process",
            ));
        }
        let attestation = C3FdDelegationPreventionAttestation {
            service_id: &entry.scope.service_id,
            catalog_generation: entry.scope.catalog_generation,
            launch_generation: entry.scope.c3_launch_generation,
            scope_binding_digest: &entry.scope.binding_digest,
            process_identity,
            _kernel_attestation: restriction.as_ref(),
        };
        Ok(consume(&attestation))
    }

    /// Retire only the exact admission supplied by A and verify its durable
    /// slot is cleared before reporting success to the launch owner.
    pub(super) fn finish_admitted(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        proof: Option<&SystemServiceEndpointOwnerProof>,
        wait_budget: Duration,
        poll_interval: Duration,
    ) -> io::Result<()> {
        validate_reaper_wait(wait_budget, poll_interval)?;
        let deadline = Instant::now().checked_add(wait_budget).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "retirement budget overflow")
        })?;
        let scope_snapshot = capture_scope(scope)?;
        validate_scope_for_slot(&scope_snapshot, &self.service_id)?;
        let scope_address = scope_data_address(scope);
        let mut state = self.lock_state()?;
        if state.reserved_generation.is_some() || state.reserved_operation.is_some() {
            return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
        }

        let matching_entry = state.launches.iter().find(|entry| {
            entry.token.0 == scope_snapshot.c3_launch_generation
                && entry.scope == scope_snapshot
                && entry.scope_data_address == scope_address
        });
        if let Some(current) = state.current {
            if matching_entry.is_none_or(|entry| entry.token != current) {
                return Err(permission_denied(
                    "C3 current launch is not the exact admission being finished",
                ));
            }
            let entry = find_launch(&state.launches, current)
                .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
            validate_entry_scope(
                entry,
                scope,
                &scope_snapshot,
                scope_address,
                &self.service_id,
            )?;
        }
        if let Some(proof) = proof {
            validate_proof_for_scope(proof, &scope_snapshot, &self.service_id, matching_entry)?;
        }

        let journal = state
            .durable_journal
            .clone()
            .ok_or_else(|| io::Error::other("exact C3 finish requires the durable slot journal"))?;
        let before = match journal.readback_until(deadline) {
            C3SlotJournalReadback::Committed(row) => row,
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
            }
        };
        if !slot_row_matches_scope(&before, &self.service_id, &scope_snapshot) {
            return Err(permission_denied(
                "C3 durable row does not match the exact A admission being finished",
            ));
        }
        match before.state() {
            C3SlotState::ClearedAndAbsent => {
                validate_quiescent_retired_state(&state)?;
                retirement_budget_remaining(deadline)?;
                return Ok(());
            }
            C3SlotState::Live | C3SlotState::Quarantined => {
                if matching_entry.is_none() {
                    return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
                }
                if state.launches.iter().any(|entry| {
                    entry.state != LaunchState::Retired
                        && (entry.scope != scope_snapshot
                            || entry.scope_data_address != scope_address)
                }) {
                    return Err(permission_denied(
                        "C3 owner retains a child outside the exact admission being finished",
                    ));
                }
            }
            C3SlotState::Reserved => {
                return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
            }
        }

        retire_owner_state_until(&mut state, deadline, poll_interval)?;
        validate_quiescent_retired_state(&state)?;
        let after = match journal.readback_until(deadline) {
            C3SlotJournalReadback::Committed(row) => row,
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
            }
        };
        if !slot_row_matches_scope(&after, &self.service_id, &scope_snapshot)
            || after.state() != C3SlotState::ClearedAndAbsent
        {
            return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
        }
        state.highest_operation_id = state.highest_operation_id.max(after.highest_operation_id());
        retirement_budget_remaining(deadline)?;
        Ok(())
    }

    /// Persist an exact C3 quarantine for an unresolved A admission. Repeating
    /// the call is safe. An outstanding reservation is bound to this exact
    /// admission before it is recorded as quarantined.
    pub(super) fn persist_recovery_quarantine_admitted(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
    ) -> io::Result<()> {
        let scope_snapshot = capture_scope(scope)?;
        validate_scope_for_slot(&scope_snapshot, &self.service_id)?;
        let scope_address = scope_data_address(scope);
        let mut state = self.lock_state()?;
        let journal = state
            .durable_journal
            .clone()
            .ok_or_else(|| io::Error::other("C3 quarantine requires the durable slot journal"))?;

        if let Some(reserved_generation) = state.reserved_generation {
            if reserved_generation != scope_snapshot.c3_launch_generation {
                return Err(permission_denied(
                    "C3 reservation does not match the exact A admission being quarantined",
                ));
            }
            if state.current.is_some() || state.lifecycle.active_len() != 0 {
                return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
            }
            let reservation = state
                .reserved_operation
                .clone()
                .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
            let receipt = journal
                .transition_reserved_quarantine(
                    reservation,
                    scope_snapshot.catalog_generation,
                    scope_snapshot.binding_digest,
                    || {
                        if state.current.is_none() && state.lifecycle.active_len() == 0 {
                            Ok(())
                        } else {
                            Err(io::Error::other(
                                "C3 owner changed while persisting reserved quarantine",
                            ))
                        }
                    },
                )
                .map_err(c3_journal_io_error)?;
            if !exact_durable_receipt(
                &receipt,
                &self.service_id,
                scope_snapshot.catalog_generation,
                scope_snapshot.c3_launch_generation,
                &scope_snapshot.binding_digest,
                C3SlotState::Quarantined,
            ) {
                return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
            }
            state.reserved_generation = None;
            state.reserved_operation = None;
            state.highest_catalog_generation = Some(
                state
                    .highest_catalog_generation
                    .map_or(scope_snapshot.catalog_generation, |current| {
                        current.max(scope_snapshot.catalog_generation)
                    }),
            );
            state.highest_generation = state
                .highest_generation
                .max(scope_snapshot.c3_launch_generation.get());
            state.highest_operation_id =
                state.highest_operation_id.max(receipt.operation_id().get());
            return Ok(());
        }
        if state.reserved_operation.is_some() {
            return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
        }

        let row = match journal.readback() {
            C3SlotJournalReadback::Committed(row) => row,
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
            }
        };
        if row.service_id() != self.service_id
            || row.catalog_generation() != scope_snapshot.catalog_generation.get()
            || row.c3_launch_generation() != scope_snapshot.c3_launch_generation.get()
            || row.scope_binding_digest() != &scope_snapshot.binding_digest
        {
            return Err(permission_denied(
                "C3 durable row does not match the exact A admission being quarantined",
            ));
        }
        match row.state() {
            C3SlotState::ClearedAndAbsent => {
                validate_quiescent_retired_state(&state)?;
                let receipt = journal
                    .transition_next(
                        scope_snapshot.catalog_generation,
                        scope_snapshot.c3_launch_generation,
                        scope_snapshot.binding_digest,
                        C3SlotState::Quarantined,
                        || validate_quiescent_retired_state(&state),
                    )
                    .map_err(c3_journal_io_error)?;
                if !exact_durable_receipt(
                    &receipt,
                    &self.service_id,
                    scope_snapshot.catalog_generation,
                    scope_snapshot.c3_launch_generation,
                    &scope_snapshot.binding_digest,
                    C3SlotState::Quarantined,
                ) {
                    return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
                }
                state.highest_catalog_generation = Some(
                    state
                        .highest_catalog_generation
                        .map_or(scope_snapshot.catalog_generation, |current| {
                            current.max(scope_snapshot.catalog_generation)
                        }),
                );
                state.highest_generation = state
                    .highest_generation
                    .max(scope_snapshot.c3_launch_generation.get());
                state.highest_operation_id =
                    state.highest_operation_id.max(receipt.operation_id().get());
                Ok(())
            }
            C3SlotState::Quarantined => Ok(()),
            C3SlotState::Live => {
                let matching_token = state
                    .launches
                    .iter()
                    .find(|entry| {
                        entry.token.0 == scope_snapshot.c3_launch_generation
                            && entry.scope == scope_snapshot
                            && entry.scope_data_address == scope_address
                    })
                    .map(|entry| entry.token)
                    .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
                if state
                    .current
                    .is_some_and(|current| current != matching_token)
                {
                    return Err(permission_denied(
                        "C3 current launch is not the exact admission being quarantined",
                    ));
                }
                if state.launches.iter().any(|entry| {
                    entry.state != LaunchState::Retired
                        && (entry.scope != scope_snapshot
                            || entry.scope_data_address != scope_address)
                }) {
                    return Err(permission_denied(
                        "C3 owner retains a child outside the exact admission being quarantined",
                    ));
                }
                let receipt = journal
                    .transition_next(
                        scope_snapshot.catalog_generation,
                        scope_snapshot.c3_launch_generation,
                        scope_snapshot.binding_digest,
                        C3SlotState::Quarantined,
                        || {
                            let still_exact =
                                find_launch(&state.launches, matching_token).is_some_and(|entry| {
                                    entry.scope == scope_snapshot
                                        && entry.scope_data_address == scope_address
                                }) && state
                                    .current
                                    .is_none_or(|current| current == matching_token);
                            if still_exact {
                                Ok(())
                            } else {
                                Err(io::Error::other(
                                    "C3 owner changed while persisting exact quarantine",
                                ))
                            }
                        },
                    )
                    .map_err(c3_journal_io_error)?;
                if !exact_durable_receipt(
                    &receipt,
                    &self.service_id,
                    scope_snapshot.catalog_generation,
                    scope_snapshot.c3_launch_generation,
                    &scope_snapshot.binding_digest,
                    C3SlotState::Quarantined,
                ) {
                    return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
                }
                state.highest_operation_id =
                    state.highest_operation_id.max(receipt.operation_id().get());
                Ok(())
            }
            C3SlotState::Reserved => Err(c3_journal_io_error(C3SlotJournalError::Unknown)),
        }
    }

    /// Transfer the single endpoint descriptor from the exact live child.
    /// This is an internal handoff to `issue_current_service_endpoint`; it is
    /// not an IPC descriptor-delegation API.
    pub(super) fn take_current_service_endpoint(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
    ) -> io::Result<OwnedFd> {
        let scope_snapshot = capture_scope(scope)?;
        let scope_address = scope_data_address(scope);
        let mut state = self.lock_state()?;
        let token = state
            .current
            .ok_or_else(|| permission_denied("system-service slot has no current launch"))?;
        let entry = find_launch(&state.launches, token)
            .ok_or_else(|| permission_denied("current system-service launch record is missing"))?;
        validate_entry_scope(
            entry,
            scope,
            &scope_snapshot,
            scope_address,
            &self.service_id,
        )?;
        let process_id = entry.process_id;
        let resources = match state.lifecycle.take_live_endpoint(process_id, token) {
            Ok(resources) => resources,
            Err(error) => {
                poll_and_record_absence(&mut state)?;
                return Err(error);
            }
        };
        drop(resources.pidfd);
        Ok(resources.endpoint)
    }

    /// Issue C3's non-Clone proof for the current service slot and an endpoint
    /// taken from its exact live child. Full A scope recapture and pointer
    /// identity checks run before pidfd lookup or proof issuance.
    pub(super) fn issue_current_service_endpoint(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        endpoint: OwnedFd,
    ) -> io::Result<SystemServiceEndpointOwnerProof> {
        let scope_snapshot = capture_scope(scope)?;
        let scope_address = scope_data_address(scope);
        let mut state = self.lock_state()?;
        let token = state
            .current
            .ok_or_else(|| permission_denied("system-service slot has no current launch"))?;
        let entry = find_launch(&state.launches, token)
            .ok_or_else(|| permission_denied("current system-service launch record is missing"))?;
        validate_entry_scope(
            entry,
            scope,
            &scope_snapshot,
            scope_address,
            &self.service_id,
        )?;
        let process_id = entry.process_id;
        let uid = entry.scope.uid;
        let gid = entry.scope.gid;
        let launch_record = entry.launch_record.clone();
        let pidfd = match state.lifecycle.duplicate_live_pidfd(process_id, token) {
            Ok(pidfd) => pidfd,
            Err(error) => {
                poll_and_record_absence(&mut state)?;
                return Err(error);
            }
        };
        match SystemServiceEndpointOwnerProof::authenticate(
            &pidfd,
            &launch_record,
            endpoint,
            uid,
            gid,
        ) {
            Ok(proof) => Ok(proof),
            Err(error) => {
                if state.durable_journal.is_some() {
                    retire_owner_state_once(
                        &mut state,
                        DEFAULT_RETIREMENT_WAIT_BUDGET,
                        DEFAULT_REAPER_POLL_INTERVAL,
                    )?;
                    return Err(error);
                }
                if let Ok(Some((process_id, token))) = revoke_current_record(&mut state) {
                    let _ = terminate_launch(&mut state, process_id, token);
                }
                Err(error)
            }
        }
    }

    /// Serialize a C1 reconciliation callback against the exact current slot.
    /// The C1 caller must pass the same A admission reference used by launch
    /// and endpoint issuance. The callback's panic remains OutcomeUnknown in
    /// `C3ProofRecoveryGate`; this method performs no follow-up effect.
    pub(super) fn with_current_reconciliation_gate<R>(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        proof: &SystemServiceEndpointOwnerProof,
        claim: C1RecoveryOperationCorrelation,
        reconcile: impl FnOnce() -> R,
    ) -> Result<R, RecoveryClientError> {
        let scope_snapshot = capture_scope(scope).map_err(|_| RecoveryClientError::Rejected)?;
        let scope_address = scope_data_address(scope);
        {
            let state = self
                .state
                .lock()
                .map_err(|_| RecoveryClientError::Unavailable)?;
            let Some(token) = state.current else {
                return Err(RecoveryClientError::Rejected);
            };
            let Some(entry) = find_launch(&state.launches, token) else {
                return Err(RecoveryClientError::Rejected);
            };
            validate_entry_scope(
                entry,
                scope,
                &scope_snapshot,
                scope_address,
                &self.service_id,
            )
            .map_err(|_| RecoveryClientError::Rejected)?;
            let peer = proof.peer_credentials();
            if entry.process_id != proof.process_id()
                || token.0 != proof.launch_generation()
                || entry.scope.catalog_generation
                    != proof.service_slot_identity().catalog_generation()
                || entry.scope.service_id != proof.service_slot_identity().service_id()
                || (entry.scope.uid, entry.scope.gid) != (peer.uid, peer.gid)
                || proof.process_identity().process_id() != peer.process_id
                || entry.state != LaunchState::Running
            {
                return Err(RecoveryClientError::Rejected);
            }
        }

        let mut gate = C3ProofRecoveryGate::new(scope, proof);
        gate.with_reconciliation_gate(scope, claim, reconcile)
    }

    /// Apply one C1 recovery effect under the exact live C3 proof and persist
    /// the unchanged C1 operation correlation before invoking the callback.
    /// The C3 row remains Live only after the callback returns and that exact
    /// correlated row has been committed and read back. Any incomplete journal
    /// transaction or rejected proof leaves recovery fail-closed.
    ///
    /// The callback must not re-enter this owner or its journal. The C1
    /// operation identity is correlation only and never grants authority.
    pub(super) fn with_durable_live_effect_gate<R>(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        proof: &SystemServiceEndpointOwnerProof,
        claim: C1RecoveryOperationCorrelation,
        effect: impl FnOnce() -> R,
    ) -> io::Result<(R, C3DurableSlotReceipt)> {
        let scope_snapshot = capture_scope(scope)?;
        validate_scope_for_slot(&scope_snapshot, &self.service_id)?;
        if claim.binding_digest() != &scope_snapshot.binding_digest {
            return Err(permission_denied(
                "C1 operation correlation does not match the exact A recovery admission",
            ));
        }
        let scope_address = scope_data_address(scope);
        let mut state = self.lock_state()?;
        let token = state
            .current
            .ok_or_else(|| permission_denied("system-service slot has no current launch"))?;
        let entry = find_launch(&state.launches, token)
            .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
        validate_entry_scope(
            entry,
            scope,
            &scope_snapshot,
            scope_address,
            &self.service_id,
        )?;
        let peer = proof.peer_credentials();
        if entry.state != LaunchState::Running
            || token.0 != scope_snapshot.c3_launch_generation
            || entry.process_id != proof.process_id()
            || token.0 != proof.launch_generation()
            || entry.scope.catalog_generation != proof.service_slot_identity().catalog_generation()
            || entry.scope.service_id != proof.service_slot_identity().service_id()
            || (entry.scope.uid, entry.scope.gid) != (peer.uid, peer.gid)
            || proof.process_identity().process_id() != peer.process_id
            || !entry
                .launch_record
                .process_identity()
                .same_instance(&proof.process_identity())
            || state.lifecycle.live_record(entry.process_id) != Some(&token)
            || !state.lifecycle.is_live_record(entry.process_id, token)?
        {
            return Err(permission_denied(
                "C3 live-effect proof is not for the exact current child",
            ));
        }

        let journal = state.durable_journal.clone().ok_or_else(|| {
            io::Error::other("C3 live-effect gate requires the durable slot journal")
        })?;
        let row = match journal.readback() {
            C3SlotJournalReadback::Committed(row) => row,
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
            }
        };
        if row.service_id() != self.service_id
            || row.catalog_generation() != scope_snapshot.catalog_generation.get()
            || row.c3_launch_generation() != scope_snapshot.c3_launch_generation.get()
            || row.scope_binding_digest() != &scope_snapshot.binding_digest
            || row.state() != C3SlotState::Live
        {
            return Err(permission_denied(
                "C3 durable live row does not match the exact A recovery admission",
            ));
        }

        let (result, receipt) = journal
            .with_c1_correlated_live_receipt(
                scope_snapshot.catalog_generation,
                scope_snapshot.c3_launch_generation,
                &scope_snapshot.binding_digest,
                claim,
                || proof.with_live_authority_effect(effect),
            )
            .map_err(c3_journal_io_error)?;
        if receipt.c1_operation_id() != Some(claim.operation_id())
            || !exact_durable_receipt(
                &receipt,
                &self.service_id,
                scope_snapshot.catalog_generation,
                scope_snapshot.c3_launch_generation,
                &scope_snapshot.binding_digest,
                C3SlotState::Live,
            )
        {
            return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
        }
        state.highest_operation_id = state.highest_operation_id.max(receipt.operation_id().get());
        Ok((result, receipt))
    }

    /// Gate a continuing effect for the same C1 runtime operation after its
    /// initial recovery has made the exact correlated Live row durable. This
    /// entry cannot replay the bootstrap transition: it requires the current
    /// journal row to match that same C1 operation ID and exact live slot.
    pub(super) fn with_durable_live_continuation_gate<R>(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        proof: &SystemServiceEndpointOwnerProof,
        claim: C1RecoveryOperationCorrelation,
        effect: impl FnOnce() -> io::Result<R>,
    ) -> io::Result<(R, C3DurableSlotReceipt)> {
        let scope_snapshot = capture_scope(scope)?;
        validate_scope_for_slot(&scope_snapshot, &self.service_id)?;
        if claim.binding_digest() != &scope_snapshot.binding_digest {
            return Err(permission_denied(
                "C1 continuation correlation does not match the exact A recovery admission",
            ));
        }
        let scope_address = scope_data_address(scope);
        let mut state = self.lock_state()?;
        let token = state
            .current
            .ok_or_else(|| permission_denied("system-service slot has no current launch"))?;
        let entry = find_launch(&state.launches, token)
            .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
        validate_entry_scope(
            entry,
            scope,
            &scope_snapshot,
            scope_address,
            &self.service_id,
        )?;
        let peer = proof.peer_credentials();
        if entry.state != LaunchState::Running
            || token.0 != scope_snapshot.c3_launch_generation
            || entry.process_id != proof.process_id()
            || token.0 != proof.launch_generation()
            || entry.scope.catalog_generation != proof.service_slot_identity().catalog_generation()
            || entry.scope.service_id != proof.service_slot_identity().service_id()
            || (entry.scope.uid, entry.scope.gid) != (peer.uid, peer.gid)
            || proof.process_identity().process_id() != peer.process_id
            || !entry
                .launch_record
                .process_identity()
                .same_instance(&proof.process_identity())
            || state.lifecycle.live_record(entry.process_id) != Some(&token)
            || !state.lifecycle.is_live_record(entry.process_id, token)?
        {
            return Err(permission_denied(
                "C3 continuation proof is not for the exact current child",
            ));
        }

        let journal = state.durable_journal.clone().ok_or_else(|| {
            io::Error::other("C3 live-continuation gate requires the durable slot journal")
        })?;
        let row = match journal.readback() {
            C3SlotJournalReadback::Committed(row) => row,
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
            }
        };
        if row.service_id() != self.service_id
            || row.catalog_generation() != scope_snapshot.catalog_generation.get()
            || row.c3_launch_generation() != scope_snapshot.c3_launch_generation.get()
            || row.scope_binding_digest() != &scope_snapshot.binding_digest
            || row.state() != C3SlotState::Live
            || row.c1_operation_id() != Some(claim.operation_id())
        {
            return Err(permission_denied(
                "C3 Live row does not match the continuing C1 operation and exact admission",
            ));
        }

        let (result, receipt) = journal
            .with_c1_correlated_live_continuation_receipt(
                scope_snapshot.catalog_generation,
                scope_snapshot.c3_launch_generation,
                &scope_snapshot.binding_digest,
                claim,
                || proof.with_live_authority_effect(effect),
            )
            .map_err(c3_journal_io_error)?;
        let result = result?;
        if receipt.c1_operation_id() != Some(claim.operation_id())
            || !exact_durable_receipt(
                &receipt,
                &self.service_id,
                scope_snapshot.catalog_generation,
                scope_snapshot.c3_launch_generation,
                &scope_snapshot.binding_digest,
                C3SlotState::Live,
            )
        {
            return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
        }
        state.highest_operation_id = state.highest_operation_id.max(receipt.operation_id().get());
        Ok((result, receipt))
    }

    /// Reconcile the exact C3 slot before C1 terminal recovery. A tracked
    /// current or quarantined child is revoked and reaped first; then the C1
    /// operation identity is written into a durable C3 intent before its
    /// callback runs under both the C3 slot mutex and journal lock. The C3
    /// transaction commits after the callback and returns a receipt only from
    /// the exact correlated `ClearedAndAbsent` row read back from disk.
    ///
    /// The callback must not re-enter this C3 owner or journal. No C1 operation
    /// identity is correlation data only; C3 keeps its local journal sequence
    /// separate and grants no authority from the supplied C1 operation ID.
    pub(super) fn with_durable_reconciliation_gate<R>(
        &self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        claim: C1RecoveryOperationCorrelation,
        reconcile: impl FnOnce() -> R,
    ) -> io::Result<(R, C3DurableSlotReceipt)> {
        let scope_snapshot = capture_scope(scope)?;
        validate_scope_for_slot(&scope_snapshot, &self.service_id)?;
        let scope_address = scope_data_address(scope);
        let mut state = self.lock_state()?;
        let journal = state.durable_journal.clone().ok_or_else(|| {
            io::Error::other("C3 reconciliation requires the durable slot journal")
        })?;

        let row = match journal.readback() {
            C3SlotJournalReadback::Committed(row) => row,
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
            }
        };
        if row.service_id() != self.service_id
            || row.catalog_generation() != scope_snapshot.catalog_generation.get()
            || row.c3_launch_generation() != scope_snapshot.c3_launch_generation.get()
            || row.scope_binding_digest() != &scope_snapshot.binding_digest
        {
            return Err(permission_denied(
                "C3 durable row does not match the exact A recovery admission",
            ));
        }

        match row.state() {
            C3SlotState::ClearedAndAbsent => {
                validate_quiescent_retired_state(&state)?;
            }
            C3SlotState::Live | C3SlotState::Quarantined => {
                let matching_token = state
                    .launches
                    .iter()
                    .find(|entry| {
                        entry.token.0 == scope_snapshot.c3_launch_generation
                            && entry.scope == scope_snapshot
                            && entry.scope_data_address == scope_address
                    })
                    .map(|entry| entry.token)
                    .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
                if let Some(current) = state.current {
                    if current != matching_token {
                        return Err(permission_denied(
                            "C3 current launch is not the exact admission being reconciled",
                        ));
                    }
                    let entry = find_launch(&state.launches, current)
                        .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
                    validate_entry_scope(
                        entry,
                        scope,
                        &scope_snapshot,
                        scope_address,
                        &self.service_id,
                    )?;
                }
                retire_owner_state_once(
                    &mut state,
                    DEFAULT_RETIREMENT_WAIT_BUDGET,
                    DEFAULT_REAPER_POLL_INTERVAL,
                )?;
                validate_quiescent_retired_state(&state)?;
            }
            C3SlotState::Reserved => {
                return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
            }
        }

        if claim.binding_digest() != &scope_snapshot.binding_digest {
            return Err(permission_denied(
                "C1 operation correlation does not match the exact A recovery admission",
            ));
        }
        let (result, receipt) = journal
            .with_c1_correlated_cleared_receipt(
                scope_snapshot.catalog_generation,
                scope_snapshot.c3_launch_generation,
                &scope_snapshot.binding_digest,
                claim,
                reconcile,
            )
            .map_err(c3_journal_io_error)?;
        if !exact_durable_receipt(
            &receipt,
            &self.service_id,
            scope_snapshot.catalog_generation,
            scope_snapshot.c3_launch_generation,
            &scope_snapshot.binding_digest,
            C3SlotState::ClearedAndAbsent,
        ) {
            return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
        }
        state.highest_operation_id = state.highest_operation_id.max(receipt.operation_id().get());
        Ok((result, receipt))
    }

    /// Read the latest full previous-scope snapshot attached to a durable C3
    /// normal-exit terminal record. `Ok(None)` is reserved for a validated
    /// genesis-only journal; any existing row without exact proof remains
    /// `OutcomeUnknown` at the C1 adapter boundary.
    pub(super) fn latest_durable_normal_exit_scope_snapshot(
        &self,
    ) -> io::Result<Option<C3DurableNormalExitReceipt>> {
        self.latest_durable_normal_exit_scope_snapshot_with_deadline(None)
    }

    pub(super) fn latest_durable_normal_exit_scope_snapshot_until(
        &self,
        deadline: Instant,
    ) -> io::Result<Option<C3DurableNormalExitReceipt>> {
        self.latest_durable_normal_exit_scope_snapshot_with_deadline(Some(deadline))
    }

    fn latest_durable_normal_exit_scope_snapshot_with_deadline(
        &self,
        deadline: Option<Instant>,
    ) -> io::Result<Option<C3DurableNormalExitReceipt>> {
        let state = self.lock_state()?;
        ensure_normal_exit_deadline(deadline)?;
        let journal = state
            .durable_journal
            .as_ref()
            .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
        let receipt = match deadline {
            Some(deadline) => journal.latest_normal_exit_scope_snapshot_until(deadline),
            None => journal.latest_normal_exit_scope_snapshot(),
        }
        .map_err(c3_journal_io_error)?;
        let Some(receipt) = receipt else {
            ensure_normal_exit_deadline(deadline)?;
            return Ok(None);
        };
        let scope = receipt.scope_snapshot();
        if receipt.service_id() != self.service_id
            || scope.service_id != self.service_id
            || scope.canonical_digest() != *receipt.scope_binding_digest()
            || scope.binding_digest != *receipt.scope_binding_digest()
            || scope.catalog_generation != receipt.catalog_generation()
            || scope.c3_launch_generation != receipt.c3_launch_generation()
        {
            return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
        }
        ensure_normal_exit_deadline(deadline)?;
        Ok(Some(receipt))
    }

    pub(super) fn durable_normal_exit_gate<'owner, 'receipt>(
        &'owner self,
        expected: &'receipt C3DurableNormalExitReceipt,
    ) -> C3DurableNormalExitGate<'owner, 'receipt> {
        self.durable_normal_exit_gate_with_deadline(expected, None)
    }

    pub(super) fn durable_normal_exit_gate_until<'owner, 'receipt>(
        &'owner self,
        expected: &'receipt C3DurableNormalExitReceipt,
        deadline: Instant,
    ) -> C3DurableNormalExitGate<'owner, 'receipt> {
        self.durable_normal_exit_gate_with_deadline(expected, Some(deadline))
    }

    fn durable_normal_exit_gate_with_deadline<'owner, 'receipt>(
        &'owner self,
        expected: &'receipt C3DurableNormalExitReceipt,
        deadline: Option<Instant>,
    ) -> C3DurableNormalExitGate<'owner, 'receipt> {
        C3DurableNormalExitGate {
            owner: self,
            expected,
            deadline,
            used: false,
        }
    }

    fn with_durable_normal_exit_gate<R>(
        &self,
        expected: &C3DurableNormalExitReceipt,
        claim: C1RecoveryOperationCorrelation,
        deadline: Option<Instant>,
        reconcile: impl FnOnce() -> R,
    ) -> io::Result<R> {
        let scope = expected.scope_snapshot();
        if expected.service_id() != self.service_id
            || scope.service_id != self.service_id
            || scope.canonical_digest() != *expected.scope_binding_digest()
            || scope.binding_digest != *expected.scope_binding_digest()
            || scope.catalog_generation != expected.catalog_generation()
            || scope.c3_launch_generation != expected.c3_launch_generation()
            || claim.operation_id() != expected.c1_operation_id()
            || claim.binding_digest() != expected.scope_binding_digest()
        {
            return Err(permission_denied(
                "C1 correlation does not select the exact C3 historical normal-exit row",
            ));
        }
        ensure_normal_exit_deadline(deadline)?;
        let process_id = u32::try_from(expected.process_id().get())
            .map_err(|_| c3_journal_io_error(C3SlotJournalError::Unknown))?;
        let state = self.lock_state()?;
        ensure_normal_exit_deadline(deadline)?;
        let journal = state
            .durable_journal
            .as_ref()
            .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
        let readback = match deadline {
            Some(deadline) => journal.read_normal_exit_receipt_until(
                scope,
                expected.launch_operation_id(),
                process_id,
                claim,
                deadline,
            ),
            None => journal.read_normal_exit_receipt(
                scope,
                expected.launch_operation_id(),
                process_id,
                claim,
            ),
        }
        .map_err(c3_journal_io_error)?;
        if &readback != expected {
            return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
        }
        // This is the final deadline check after exact historical row and
        // claim validation, immediately before C1's reconciliation callback.
        ensure_normal_exit_deadline(deadline)?;
        // The callback is inside the owner gate. It must not re-enter C3.
        Ok(reconcile())
    }

    /// Revoke the exact current launch before terminating its pidfd-owned
    /// process. A failed kill/reap remains in the lifecycle's quarantine.
    pub(super) fn revoke_current(
        &self,
        retirement_wait_budget: Duration,
        reaper_poll_interval: Duration,
    ) -> io::Result<()> {
        validate_reaper_wait(retirement_wait_budget, reaper_poll_interval)?;
        let mut state = self.lock_state_for_cleanup();
        if state.durable_journal.is_some() {
            retire_owner_state_once(&mut state, retirement_wait_budget, reaper_poll_interval)?;
            return validate_quiescent_retired_state(&state);
        }
        match revoke_current_record(&mut state)? {
            Some((process_id, token)) => {
                let result = terminate_launch_bounded(
                    &mut state,
                    process_id,
                    token,
                    retirement_wait_budget,
                    reaper_poll_interval,
                );
                prune_retired_launches(&mut state);
                result
            }
            None => {
                drop(state);
                self.reap_until_quiescent(retirement_wait_budget, reaper_poll_interval)
            }
        }
    }

    /// Drive the shared PID-1 child reaper and retire a naturally exited
    /// service only after the exact process status has been observed.
    pub(super) fn poll(&self) -> io::Result<Option<super::LifecycleEvent<String>>> {
        let mut state = self.lock_state()?;
        poll_and_record_absence(&mut state)
    }

    /// Wait only up to the caller's budget for quarantined children to be
    /// consumed by PID-1's shared exact reaper.
    pub(super) fn reap_until_quiescent(
        &self,
        wait_budget: Duration,
        poll_interval: Duration,
    ) -> io::Result<()> {
        validate_reaper_wait(wait_budget, poll_interval)?;
        let deadline = std::time::Instant::now()
            .checked_add(wait_budget)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "system-service reaper wait budget is too large",
                )
            })?;
        self.reap_until_quiescent_until(deadline, poll_interval)
    }

    pub(super) fn reap_until_quiescent_until(
        &self,
        deadline: Instant,
        poll_interval: Duration,
    ) -> io::Result<()> {
        if poll_interval.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "system-service reaper poll interval must be nonzero",
            ));
        }
        self.reap_until_quiescent_with_clock(deadline, poll_interval, &Instant::now)
    }

    fn reap_until_quiescent_with_clock(
        &self,
        deadline: Instant,
        poll_interval: Duration,
        now: &impl Fn() -> Instant,
    ) -> io::Result<()> {
        loop {
            let mut state = self.lock_state_for_cleanup();
            let had_current = state.current.is_some();
            let poll_result = poll_and_record_absence_until(&mut state, Some(deadline));
            if had_current && state.current.is_none() {
                poll_result?;
            }
            if state.lifecycle.active_len() == 0 {
                retirement_budget_remaining_at(deadline, now())?;
                return Ok(());
            }
            drop(state);
            let remaining = deadline.saturating_duration_since(now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "PID-1 still owns quarantined system-service children",
                ));
            }
            std::thread::sleep(poll_interval.min(remaining));
        }
    }

    #[cfg(test)]
    fn active_child_count(&self) -> io::Result<usize> {
        Ok(self.lock_state()?.lifecycle.active_len())
    }

    fn lock_state(&self) -> io::Result<std::sync::MutexGuard<'_, OwnerState>> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("C3 system-service slot owner is poisoned"))
    }

    /// Cleanup may borrow poisoned state exclusively, but it never clears the
    /// poison bit or returns the state to ordinary launch/authority methods.
    fn lock_state_for_cleanup(&self) -> std::sync::MutexGuard<'_, OwnerState> {
        lock_owner_state_for_cleanup(&self.state)
    }
}

fn ensure_normal_exit_deadline(deadline: Option<Instant>) -> io::Result<()> {
    if let Some(deadline) = deadline {
        retirement_budget_remaining_at(deadline, Instant::now())?;
    }
    Ok(())
}

impl Drop for LinuxC3SystemServiceSlotOwner {
    fn drop(&mut self) {
        let state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut state = state;
            let _ = retire_owner_state_once(
                &mut state,
                DEFAULT_RETIREMENT_WAIT_BUDGET,
                DEFAULT_REAPER_POLL_INTERVAL,
            );
        }));

        if let Ok(mut registry) = SLOT_OWNER_REGISTRY.lock() {
            if let Some(entry) = registry.iter_mut().find(|entry| {
                entry.service_id == self.service_id && Arc::ptr_eq(&entry.state, &self.state)
            }) {
                entry.claimed = false;
                entry.maintenance = false;
            }
        }
    }
}

fn slot_owner_registry_lock() -> io::Result<MutexGuard<'static, Vec<SlotOwnerRegistration>>> {
    SLOT_OWNER_REGISTRY
        .lock()
        .map_err(|_| io::Error::other("C3 system-service owner registry is poisoned"))
}

fn reserve_slot_owner_registration(service_id: &str) -> io::Result<(Arc<Mutex<OwnerState>>, bool)> {
    if service_id.is_empty() || service_id.as_bytes().contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "system-service slot ID must be nonempty and contain no NUL",
        ));
    }
    let mut registry = slot_owner_registry_lock()?;
    if let Some(entry) = registry
        .iter_mut()
        .find(|entry| entry.service_id == service_id)
    {
        if entry.claimed || entry.maintenance {
            return Err(permission_denied(
                "a C3 owner already holds or maintains this system-service slot",
            ));
        }
        entry.maintenance = true;
        return Ok((Arc::clone(&entry.state), false));
    }

    let state = Arc::new(Mutex::new(empty_owner_state()));
    registry.push(SlotOwnerRegistration {
        service_id: service_id.to_owned(),
        state: Arc::clone(&state),
        claimed: false,
        maintenance: true,
    });
    Ok((state, true))
}

fn abandon_slot_owner_registration(
    service_id: &str,
    state: &Arc<Mutex<OwnerState>>,
    fresh_registration: bool,
    release_fresh_registration: bool,
) {
    let Ok(mut registry) = SLOT_OWNER_REGISTRY.lock() else {
        return;
    };
    let Some(index) = registry
        .iter()
        .position(|entry| entry.service_id == service_id && Arc::ptr_eq(&entry.state, state))
    else {
        return;
    };
    if fresh_registration {
        if release_fresh_registration {
            registry.remove(index);
        }
        // Unknown or post-write outcomes keep the fresh slot reserved and
        // quarantined; only a proven pre-write rejection releases it.
    } else {
        registry[index].maintenance = false;
    }
}

fn lock_owner_state_for_cleanup(state: &Mutex<OwnerState>) -> MutexGuard<'_, OwnerState> {
    match state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn empty_owner_state() -> OwnerState {
    OwnerState {
        lifecycle: ProcessLifecycle::new(LinuxProcessOs),
        launches: Vec::new(),
        current: None,
        highest_generation: 0,
        highest_catalog_generation: None,
        reserved_generation: None,
        reserved_operation: None,
        highest_operation_id: 0,
        durable_journal: None,
        #[cfg(test)]
        fail_retirement_after_revoke_once: false,
    }
}

fn retire_owner_state_once(
    state: &mut OwnerState,
    wait_budget: Duration,
    poll_interval: Duration,
) -> io::Result<()> {
    validate_reaper_wait(wait_budget, poll_interval)?;
    let deadline = Instant::now()
        .checked_add(wait_budget)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "retirement budget overflow"))?;
    retire_owner_state_until(state, deadline, poll_interval)
}

fn retire_owner_state_until(
    state: &mut OwnerState,
    deadline: Instant,
    poll_interval: Duration,
) -> io::Result<()> {
    let Some(journal) = state.durable_journal.clone() else {
        return retire_owner_state_effect_until(state, deadline, poll_interval);
    };

    match journal.readback_until(deadline) {
        C3SlotJournalReadback::Empty => {
            retire_owner_state_effect_until(state, deadline, poll_interval)
        }
        C3SlotJournalReadback::Committed(row) if row.state() == C3SlotState::ClearedAndAbsent => {
            retire_owner_state_effect_until(state, deadline, poll_interval)
        }
        C3SlotJournalReadback::Committed(row) => {
            let catalog_generation = NonZeroU64::new(row.catalog_generation())
                .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
            let c3_launch_generation = NonZeroU64::new(row.c3_launch_generation())
                .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
            let service_id = row.service_id().to_owned();
            let scope_binding_digest = *row.scope_binding_digest();
            let exact_current_matches = state.current.is_none_or(|token| {
                find_launch(&state.launches, token).is_some_and(|entry| {
                    entry.scope.service_id == service_id
                        && entry.scope.catalog_generation == catalog_generation
                        && entry.scope.c3_launch_generation == c3_launch_generation
                        && entry.scope.binding_digest == scope_binding_digest
                })
            });
            if !exact_current_matches {
                retire_owner_state_effect_until(state, deadline, poll_interval)?;
                return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
            }
            let transition_result = journal.transition_next_until(
                catalog_generation,
                c3_launch_generation,
                scope_binding_digest,
                C3SlotState::ClearedAndAbsent,
                deadline,
                || retire_owner_state_effect_until(state, deadline, poll_interval),
            );
            match transition_result {
                Ok(receipt)
                    if exact_durable_receipt(
                        &receipt,
                        &service_id,
                        catalog_generation,
                        c3_launch_generation,
                        &scope_binding_digest,
                        C3SlotState::ClearedAndAbsent,
                    ) =>
                {
                    state.highest_operation_id =
                        state.highest_operation_id.max(receipt.operation_id().get());
                    Ok(())
                }
                Ok(_) => {
                    let cleanup = retire_owner_state_effect_until(state, deadline, poll_interval);
                    cleanup?;
                    Err(c3_journal_io_error(C3SlotJournalError::Unknown))
                }
                Err(error) => {
                    let cleanup = retire_owner_state_effect_until(state, deadline, poll_interval);
                    cleanup?;
                    Err(c3_journal_io_error(error))
                }
            }
        }
        C3SlotJournalReadback::Unknown => {
            retire_owner_state_effect_until(state, deadline, poll_interval)?;
            Err(c3_journal_io_error(C3SlotJournalError::Unknown))
        }
    }
}

fn retirement_budget_remaining(deadline: Instant) -> io::Result<Duration> {
    retirement_budget_remaining_at(deadline, Instant::now())
}

fn retirement_budget_remaining_at(deadline: Instant, now: Instant) -> io::Result<Duration> {
    let remaining = deadline.saturating_duration_since(now);
    if remaining.is_zero() {
        Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "C3 system-service retirement budget expired",
        ))
    } else {
        Ok(remaining)
    }
}

fn retire_owner_state_effect(
    state: &mut OwnerState,
    wait_budget: Duration,
    poll_interval: Duration,
) -> io::Result<()> {
    validate_reaper_wait(wait_budget, poll_interval)?;
    let deadline = Instant::now()
        .checked_add(wait_budget)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "retirement budget overflow"))?;
    retire_owner_state_effect_until(state, deadline, poll_interval)
}

fn retire_owner_state_effect_until(
    state: &mut OwnerState,
    deadline: Instant,
    poll_interval: Duration,
) -> io::Result<()> {
    validate_reaper_wait(retirement_budget_remaining(deadline)?, poll_interval)?;
    state.reserved_generation = None;
    state.reserved_operation = None;
    if state.current.is_some() {
        let Some((process_id, token)) = revoke_current_record(state)? else {
            return Err(io::Error::other(
                "C3 current system-service slot disappeared during shutdown",
            ));
        };
        #[cfg(test)]
        if std::mem::take(&mut state.fail_retirement_after_revoke_once) {
            return Err(io::Error::other(
                "injected one-shot retirement interruption after proof revocation",
            ));
        }
        terminate_launch_bounded_until(state, process_id, token, deadline, poll_interval)?;
    }

    let shutdown_result = {
        let OwnerState {
            lifecycle,
            launches,
            current,
            ..
        } = state;
        lifecycle.shutdown(|process_id, token| {
            revoke_and_retire_identity_listener(launches, current, process_id, *token)
        })
    };
    prune_retired_launches(state);
    if state.current.is_none() && state.lifecycle.active_len() == 0 && state.launches.is_empty() {
        retirement_budget_remaining(deadline)?;
        return validate_quiescent_retired_state(state);
    }
    if deadline.saturating_duration_since(Instant::now()).is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "C3 system-service retirement budget expired",
        ));
    }
    Err(shutdown_result.err().map(backend_error).unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "C3 system-service children remain owned after shutdown pass",
        )
    }))
}

fn slot_row_matches_scope(
    row: &super::linux_init_c3_slot_journal::C3DurableSlotRowReadback,
    service_id: &str,
    scope: &RecoveryScopeSnapshot,
) -> bool {
    row.service_id() == service_id
        && row.catalog_generation() == scope.catalog_generation.get()
        && row.c3_launch_generation() == scope.c3_launch_generation.get()
        && row.scope_binding_digest() == &scope.binding_digest
}

fn validate_proof_for_scope(
    proof: &SystemServiceEndpointOwnerProof,
    scope: &RecoveryScopeSnapshot,
    service_id: &str,
    entry: Option<&OwnedLaunch>,
) -> io::Result<()> {
    let slot = proof.service_slot_identity();
    let peer = proof.peer_credentials();
    if proof.version()
        != nagi_system_principal_contract::SYSTEM_SERVICE_ENDPOINT_OWNER_PROOF_VERSION
        || slot.service_id() != service_id
        || slot.catalog_generation() != scope.catalog_generation
        || proof.launch_generation() != scope.c3_launch_generation
        || (peer.uid, peer.gid) != (scope.uid, scope.gid)
        || proof.process_identity().process_id() != peer.process_id
    {
        return Err(permission_denied(
            "C3 endpoint proof does not match the exact A recovery admission",
        ));
    }
    if entry.is_some_and(|entry| {
        entry.process_id != proof.process_id()
            || !entry
                .launch_record
                .process_identity()
                .same_instance(&proof.process_identity())
    }) {
        return Err(permission_denied(
            "C3 endpoint proof does not match the exact registered child",
        ));
    }
    Ok(())
}

fn exact_durable_receipt(
    receipt: &C3DurableSlotReceipt,
    service_id: &str,
    catalog_generation: NonZeroU64,
    c3_launch_generation: NonZeroU64,
    scope_binding_digest: &[u8; 32],
    state: C3SlotState,
) -> bool {
    receipt.service_id() == service_id
        && receipt.catalog_generation() == catalog_generation
        && receipt.c3_launch_generation() == c3_launch_generation
        && receipt.scope_binding_digest() == scope_binding_digest
        && receipt.operation_id().get() != 0
        && receipt.durable_revision().get() != 0
        && receipt.state() == state
}

fn c3_journal_io_error(error: C3SlotJournalError) -> io::Error {
    let (kind, message) = match error {
        C3SlotJournalError::InvalidInput => (
            io::ErrorKind::InvalidInput,
            "C3 slot journal rejected invalid input",
        ),
        C3SlotJournalError::RejectedBeforeWrite => (
            io::ErrorKind::PermissionDenied,
            "C3 slot journal rejected input before writing",
        ),
        C3SlotJournalError::Rejected => (
            io::ErrorKind::PermissionDenied,
            "C3 slot journal rejected the transition",
        ),
        C3SlotJournalError::Replay => (
            io::ErrorKind::AlreadyExists,
            "C3 slot journal rejected an operation replay",
        ),
        C3SlotJournalError::Unknown => (
            io::ErrorKind::Other,
            "C3 slot journal outcome is unknown; slot reuse is disabled",
        ),
    };
    io::Error::new(kind, message)
}

fn validate_quiescent_retired_state(state: &OwnerState) -> io::Result<()> {
    if state.current.is_none()
        && state.reserved_generation.is_none()
        && state.launches.is_empty()
        && state.lifecycle.active_len() == 0
    {
        Ok(())
    } else {
        Err(io::Error::other(
            "C3 owner state is not quiescent after exact-child retirement",
        ))
    }
}

fn capture_scope(
    scope: &dyn SystemServiceRecoveryAdmissionScope,
) -> io::Result<RecoveryScopeSnapshot> {
    let snapshot = catch_unwind(AssertUnwindSafe(|| RecoveryScopeSnapshot::capture(scope)))
        .map_err(|_| permission_denied("A system-service admission panicked while being read"))?;
    if snapshot.canonical_digest() != snapshot.binding_digest {
        return Err(permission_denied(
            "A system-service admission has a noncanonical binding digest",
        ));
    }
    Ok(snapshot)
}

fn scope_data_address(scope: &dyn SystemServiceRecoveryAdmissionScope) -> usize {
    (scope as *const dyn SystemServiceRecoveryAdmissionScope as *const ()) as usize
}

fn validate_scope_for_slot(scope: &RecoveryScopeSnapshot, service_id: &str) -> io::Result<()> {
    if scope.service_id != service_id
        || scope.role != "system_service"
        || scope.uid == 0
        || scope.gid == 0
        || scope.principal_id.is_empty()
        || scope.caller_context_id.is_empty()
        || scope.package_id.is_empty()
        || scope.canonical_entry.is_empty()
    {
        return Err(permission_denied(
            "A system-service admission does not describe this non-root canonical slot",
        ));
    }
    Ok(())
}

fn validate_transition(
    next: &RecoveryScopeSnapshot,
    previous: Option<&OwnedLaunch>,
    highest_catalog_generation: Option<NonZeroU64>,
    slot_is_quiescent: bool,
    reservation: Option<&C3SlotReservation>,
) -> io::Result<()> {
    if let Some(highest) = highest_catalog_generation {
        let exact_normal_restart = slot_is_quiescent
            && previous.is_none()
            && reservation.is_some_and(|reservation| {
                reservation.permits_same_catalog_normal_restart(next, highest.get())
            });
        if next.catalog_generation < highest
            || (next.catalog_generation == highest && !exact_normal_restart)
        {
            return Err(permission_denied(
                "A system-service catalog generation rolls back or lacks an exact normal-restart receipt",
            ));
        }
    }
    match (previous, next.transition) {
        (None, RecoveryLifecycleTransition::Initial) => Ok(()),
        (
            Some(previous),
            RecoveryLifecycleTransition::Replace {
                previous_generation,
            },
        ) if previous.scope.catalog_generation == previous_generation
            && next.catalog_generation > previous.scope.catalog_generation
            && next.c3_launch_generation > previous.scope.c3_launch_generation =>
        {
            Ok(())
        }
        _ => Err(permission_denied(
            "A system-service admission transition does not advance the exact current slot",
        )),
    }
}

fn validate_same_scope(
    scope: &dyn SystemServiceRecoveryAdmissionScope,
    pinned: &RecoveryScopeSnapshot,
    pinned_data_address: usize,
    service_id: &str,
) -> io::Result<()> {
    if scope_data_address(scope) != pinned_data_address {
        return Err(permission_denied(
            "C3 operation received a different A admission object",
        ));
    }
    let current = capture_scope(scope)?;
    if current != *pinned {
        return Err(permission_denied(
            "A admission changed after C3 captured its complete scope pin",
        ));
    }
    validate_scope_for_slot(&current, service_id)
}

fn validate_entry_scope(
    entry: &OwnedLaunch,
    scope: &dyn SystemServiceRecoveryAdmissionScope,
    current: &RecoveryScopeSnapshot,
    current_data_address: usize,
    service_id: &str,
) -> io::Result<()> {
    if entry.scope_data_address != current_data_address {
        return Err(permission_denied(
            "C3 operation received a different A admission object",
        ));
    }
    if entry.scope != *current {
        return Err(permission_denied(
            "A admission changed after C3 captured its complete scope pin",
        ));
    }
    validate_scope_for_slot(current, service_id)?;
    validate_same_scope(scope, &entry.scope, entry.scope_data_address, service_id)
}

fn current_launch(state: &OwnerState) -> io::Result<Option<&OwnedLaunch>> {
    match state.current {
        Some(token) => find_launch(&state.launches, token)
            .map(Some)
            .ok_or_else(|| io::Error::other("C3 current slot points to a missing launch")),
        None => Ok(None),
    }
}

struct PreparedLaunchRegistration<'a> {
    process_id: u32,
    token: LaunchToken,
    service_id: &'a str,
    scope: RecoveryScopeSnapshot,
    launch_instance_id: Option<NonZeroU64>,
    scope_data_address: usize,
    previous_token: Option<LaunchToken>,
}

fn register_prepared_launch<Os: ProcessOs>(
    launches: &mut Vec<OwnedLaunch>,
    current: &mut Option<LaunchToken>,
    os: &mut Os,
    prepared: &ProcessHandle<Prepared, Os::Raw>,
    registration: PreparedLaunchRegistration<'_>,
    highest_catalog_generation: &mut Option<NonZeroU64>,
    replacement_committed: &mut bool,
) -> Result<LaunchToken, BackendError> {
    let PreparedLaunchRegistration {
        process_id,
        token,
        service_id,
        scope,
        launch_instance_id,
        scope_data_address,
        previous_token,
    } = registration;
    if token.0 != scope.c3_launch_generation
        || scope.service_id != service_id
        || launches.iter().any(|entry| entry.token == token)
    {
        return Err(BackendError::ProtocolViolation);
    }
    let pidfd = os
        .duplicate_pidfd_for_prepared(prepared)
        .map_err(|_| BackendError::TransportFailure)?;
    if pidfd.process_id() != process_id {
        return Err(BackendError::ProtocolViolation);
    }

    let launch_record = if let Some(previous_token) = previous_token {
        if *current != Some(previous_token) {
            return Err(BackendError::Revoked);
        }
        let old = find_launch(launches, previous_token).ok_or(BackendError::ProtocolViolation)?;
        let old_pid = old.process_id;
        let old_catalog_generation = old.scope.catalog_generation;
        let old_record = old.launch_record.clone();
        let record = unsafe {
            old_record.replace_after_supervisor_system_service_slot_validation(
                &pidfd,
                service_id,
                old_catalog_generation,
                scope.catalog_generation,
                scope.c3_launch_generation,
                || {
                    let Some(old_entry) = find_launch_mut(launches, previous_token, old_pid) else {
                        return false;
                    };
                    if *current != Some(previous_token) {
                        return false;
                    }
                    old_entry.state = LaunchState::Superseded;
                    *current = Some(token);
                    *replacement_committed = true;
                    true
                },
            )
        }
        .map_err(|_| BackendError::Revoked)?;
        record
    } else {
        if current.is_some() {
            return Err(BackendError::ProtocolViolation);
        }
        let record = unsafe {
            SystemServiceLaunchRecord::after_supervisor_system_service_slot_validation(
                &pidfd,
                service_id,
                scope.catalog_generation,
                scope.c3_launch_generation,
            )
        };
        *current = Some(token);
        record
    };

    let catalog_generation = scope.catalog_generation;
    launches.push(OwnedLaunch {
        token,
        process_id,
        launch_instance_id,
        scope,
        scope_data_address,
        launch_record,
        state: LaunchState::Prepared,
        fd_delegation_attestation: None,
        identity_listener: None,
    });
    *highest_catalog_generation = Some(catalog_generation);
    Ok(token)
}

fn registered_launch_process_id(
    state: &OwnerState,
    scope: &RecoveryScopeSnapshot,
    service_id: &str,
) -> io::Result<u32> {
    let token = state
        .current
        .ok_or_else(|| io::Error::other("C3 launch completed without a current owner token"))?;
    let entry = find_launch(&state.launches, token)
        .ok_or_else(|| io::Error::other("C3 current launch record is missing"))?;
    let record = &entry.launch_record;
    let slot = record
        .system_service_slot_identity()
        .ok_or_else(|| io::Error::other("C3 launch record has no system-service slot identity"))?;
    if entry.state != LaunchState::Running
        || entry.scope != *scope
        || entry.token.0 != scope.c3_launch_generation
        || entry.process_id == 0
        || record.generation() != entry.token.0
        || record.process_identity().process_id() != entry.process_id
        || slot.service_id() != service_id
        || slot.catalog_generation() != scope.catalog_generation
        || !record.is_live()
    {
        return Err(io::Error::other(
            "C3 official launch record does not match its durable scope and child identity",
        ));
    }
    Ok(entry.process_id)
}

fn identity_launch_binding(entry: &OwnedLaunch) -> C3IdentityLaunchBinding {
    C3IdentityLaunchBinding {
        service_id: entry.scope.service_id.clone(),
        catalog_generation: entry.scope.catalog_generation,
        c3_launch_generation: entry.token.0,
        scope_binding_digest: entry.scope.binding_digest,
        process_identity: entry.launch_record.process_identity().clone(),
        launch_record: entry.launch_record.clone(),
    }
}

fn identity_binding_matches_launch(binding: &C3IdentityLaunchBinding, entry: &OwnedLaunch) -> bool {
    binding.matches(&identity_launch_binding(entry))
}

fn authorize_prepared_launch<Os: ProcessOs>(
    launches: &mut [OwnedLaunch],
    current: Option<LaunchToken>,
    process_id: u32,
    token: LaunchToken,
    gate: &mut GateCommit<'_, Os, LaunchToken>,
) -> Result<(), BackendError> {
    if !is_current_launch(launches, current, process_id, token) {
        return Err(BackendError::Revoked);
    }
    gate.commit()?;
    let entry =
        find_launch_mut(launches, token, process_id).ok_or(BackendError::ProtocolViolation)?;
    entry.state = LaunchState::Running;
    Ok(())
}

fn is_current_launch(
    launches: &[OwnedLaunch],
    current: Option<LaunchToken>,
    process_id: u32,
    token: LaunchToken,
) -> bool {
    current == Some(token)
        && find_launch(launches, token).is_some_and(|entry| {
            entry.process_id == process_id && entry.state == LaunchState::Prepared
        })
}

fn revoke_registered_launch(
    launches: &mut [OwnedLaunch],
    current: &mut Option<LaunchToken>,
    process_id: u32,
    token: LaunchToken,
) -> Result<(), BackendError> {
    let index = launches
        .iter()
        .position(|entry| entry.token == token && entry.process_id == process_id)
        .ok_or(BackendError::ProtocolViolation)?;
    match launches[index].state {
        LaunchState::Superseded | LaunchState::Revoked => {
            launches[index].state = LaunchState::Retired;
            Ok(())
        }
        LaunchState::Prepared | LaunchState::Running => {
            if *current != Some(token) {
                return Err(BackendError::Revoked);
            }
            let record = launches[index].launch_record.clone();
            let revoked = unsafe {
                record.revoke_after_supervisor_record_validation(|| {
                    if *current != Some(token) {
                        return false;
                    }
                    *current = None;
                    true
                })
            };
            if revoked || !launches[index].launch_record.is_live() {
                if *current == Some(token) {
                    *current = None;
                }
                launches[index].state = LaunchState::Retired;
                Ok(())
            } else {
                Err(BackendError::Revoked)
            }
        }
        LaunchState::Retired => Ok(()),
    }
}

/// ProcessLifecycle invokes this callback only after its sole reaper has an
/// exact wait result. The C3 Supervisor record is revoked first; only then
/// may C2 retire the listener bound to that same C3 launch binding.
fn revoke_and_retire_identity_listener(
    launches: &mut [OwnedLaunch],
    current: &mut Option<LaunchToken>,
    process_id: u32,
    token: LaunchToken,
) -> Result<(), BackendError> {
    revoke_registered_launch(launches, current, process_id, token)?;
    retire_identity_listener_after_exact_reap(launches, process_id, token)
}

fn retire_identity_listener_after_exact_reap(
    launches: &mut [OwnedLaunch],
    process_id: u32,
    token: LaunchToken,
) -> Result<(), BackendError> {
    let entry = find_launch_mut(launches, token, process_id).ok_or(BackendError::Revoked)?;
    let Some(bound_binding) = entry.identity_listener.as_ref().map(|bound| &bound.binding) else {
        return Ok(());
    };
    let binding_matches = entry.state == LaunchState::Retired
        && identity_binding_matches_launch(bound_binding, entry);
    let Some(bound) = entry.identity_listener.as_mut() else {
        return Err(BackendError::ProtocolViolation);
    };
    if bound.retirement != IdentityListenerRetirement::NotAttempted || !binding_matches {
        bound.retirement = IdentityListenerRetirement::Unknown;
        return Err(BackendError::IdentityRejected);
    }

    // Mark first: a failing callback has an unknown result and is never
    // retried by a later reap/reconcile pass.
    bound.retirement = IdentityListenerRetirement::Unknown;
    let retirement = catch_unwind(AssertUnwindSafe(|| {
        bound
            .lease
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retire_after_exact_reap(&bound.binding)
    }));
    if matches!(retirement, Ok(Ok(()))) {
        bound.retirement = IdentityListenerRetirement::Complete;
        entry.identity_listener = None;
        Ok(())
    } else {
        Err(BackendError::TransportFailure)
    }
}

fn terminate_launch(state: &mut OwnerState, process_id: u32, token: LaunchToken) -> io::Result<()> {
    let OwnerState {
        lifecycle,
        launches,
        current,
        ..
    } = state;
    lifecycle
        .terminate_exact(process_id, token, |pid, record| {
            revoke_and_retire_identity_listener(launches, current, pid, *record)
        })
        .map_err(backend_error)
}

fn terminate_launch_bounded(
    state: &mut OwnerState,
    process_id: u32,
    token: LaunchToken,
    wait_budget: Duration,
    poll_interval: Duration,
) -> io::Result<()> {
    validate_reaper_wait(wait_budget, poll_interval)?;
    let deadline = Instant::now().checked_add(wait_budget).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "system-service retirement wait budget is too large",
        )
    })?;
    terminate_launch_bounded_until(state, process_id, token, deadline, poll_interval)
}

fn terminate_launch_bounded_until(
    state: &mut OwnerState,
    process_id: u32,
    token: LaunchToken,
    deadline: Instant,
    poll_interval: Duration,
) -> io::Result<()> {
    validate_reaper_wait(retirement_budget_remaining(deadline)?, poll_interval)?;
    let first_error = terminate_launch(state, process_id, token).err();
    loop {
        retirement_budget_remaining(deadline)?;
        prune_retired_launches(state);
        if find_launch(&state.launches, token).is_none() {
            return Ok(());
        }
        let _ = poll_state(state);
        prune_retired_launches(state);
        if find_launch(&state.launches, token).is_none() {
            retirement_budget_remaining(deadline)?;
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err(first_error.unwrap_or_else(|| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    "PID-1 still owns the system-service child after its retirement budget",
                )
            }));
        }
        std::thread::sleep(poll_interval.min(remaining));
    }
}

fn revoke_current_record(state: &mut OwnerState) -> io::Result<Option<(u32, LaunchToken)>> {
    let Some(token) = state.current else {
        return Ok(None);
    };
    let entry = find_launch(&state.launches, token)
        .ok_or_else(|| io::Error::other("C3 current slot points to a missing launch"))?;
    let process_id = entry.process_id;
    let index = state
        .launches
        .iter()
        .position(|entry| entry.token == token)
        .ok_or_else(|| io::Error::other("C3 current launch record disappeared"))?;
    let record = state.launches[index].launch_record.clone();
    let revoked = unsafe {
        record.revoke_after_supervisor_record_validation(|| {
            if state.current != Some(token) {
                return false;
            }
            state.current = None;
            true
        })
    };
    if !revoked && record.is_live() {
        return Err(permission_denied(
            "C3 could not revoke the exact current system-service slot while its proof remained live",
        ));
    }
    if revoked && state.current == Some(token) {
        return Err(io::Error::other(
            "C3 launch record was revoked without clearing the exact current slot",
        ));
    }
    if !revoked {
        if state.current != Some(token) {
            return Err(io::Error::other(
                "C3 current slot changed before fail-closed launch retirement",
            ));
        }
        state.current = None;
    }
    state.launches[index].state = LaunchState::Revoked;
    Ok(Some((process_id, token)))
}

impl C3RecoverySlotGate for C3DurableNormalExitGate<'_, '_> {
    fn with_reconciliation_gate<R>(
        &mut self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        claim: C1RecoveryOperationCorrelation,
        reconcile: impl FnOnce() -> R,
    ) -> Result<R, RecoveryClientError> {
        if self.used {
            return Err(RecoveryClientError::Rejected);
        }
        self.used = true;
        let scope_snapshot = capture_scope(scope).map_err(|_| RecoveryClientError::Rejected)?;
        if &scope_snapshot != self.expected.scope_snapshot() {
            return Err(RecoveryClientError::Rejected);
        }
        self.owner
            .with_durable_normal_exit_gate(self.expected, claim, self.deadline, reconcile)
            .map_err(|error| match error.kind() {
                io::ErrorKind::PermissionDenied | io::ErrorKind::InvalidInput => {
                    RecoveryClientError::Rejected
                }
                _ => RecoveryClientError::OutcomeUnknown,
            })
    }
}

fn validate_reaper_wait(wait_budget: Duration, poll_interval: Duration) -> io::Result<()> {
    if wait_budget.is_zero() || poll_interval.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "system-service retirement wait and reaper poll intervals must be nonzero",
        ));
    }
    Ok(())
}

fn poll_state(
    state: &mut OwnerState,
) -> Result<Option<super::LifecycleEvent<String>>, BackendError> {
    let OwnerState {
        lifecycle,
        launches,
        current,
        ..
    } = state;
    lifecycle.poll(|process_id, token| {
        revoke_and_retire_identity_listener(launches, current, process_id, *token)
    })
}

fn poll_and_record_absence(
    state: &mut OwnerState,
) -> io::Result<Option<super::LifecycleEvent<String>>> {
    poll_and_record_absence_until(state, None)
}

fn poll_and_record_absence_until(
    state: &mut OwnerState,
    deadline: Option<Instant>,
) -> io::Result<Option<super::LifecycleEvent<String>>> {
    let had_current = state.current.is_some();
    let observed_launch = capture_current_launch_identity(state).ok().flatten();
    let result = poll_state(state).map_err(backend_error);
    prune_retired_launches(state);
    if had_current && state.current.is_none() {
        let normal_exit = matches!(
            &result,
            Ok(Some(super::LifecycleEvent::ServiceExited { service, status, .. }))
                if observed_launch.as_ref().is_some_and(|launch| {
                    service == &launch.scope.service_id
                        && status.exit_code == Some(0)
                        && status.signal.is_none()
                })
        );
        record_current_absent_after_reap(state, deadline, observed_launch.as_ref(), normal_exit)?;
    }
    result
}

#[derive(Clone)]
struct ReapedLaunchObservation {
    scope: RecoveryScopeSnapshot,
    launch_instance_id: Option<NonZeroU64>,
    process_id: u32,
}

fn capture_current_launch_identity(
    state: &OwnerState,
) -> io::Result<Option<ReapedLaunchObservation>> {
    let Some(token) = state.current else {
        return Ok(None);
    };
    let entry = find_launch(&state.launches, token)
        .ok_or_else(|| io::Error::other("C3 current launch has no owner record"))?;
    let record = &entry.launch_record;
    let slot = record
        .system_service_slot_identity()
        .ok_or_else(|| io::Error::other("C3 launch record has no system-service slot"))?;
    if entry.state != LaunchState::Running
        || (state.durable_journal.is_some() && entry.launch_instance_id.is_none())
        || entry.process_id == 0
        || entry.token.0 != entry.scope.c3_launch_generation
        || record.generation() != entry.token.0
        || record.process_identity().process_id() != entry.process_id
        || slot.service_id() != entry.scope.service_id
        || slot.catalog_generation() != entry.scope.catalog_generation
        || !entry.launch_record.is_live()
        || entry.scope.canonical_digest() != entry.scope.binding_digest
    {
        return Err(io::Error::other(
            "C3 current official launch record does not match the process owner",
        ));
    }
    Ok(Some(ReapedLaunchObservation {
        scope: entry.scope.clone(),
        launch_instance_id: entry.launch_instance_id,
        process_id: entry.process_id,
    }))
}

/// Record an externally observed exact-child exit after PID-1's reaper has
/// removed the current launch. The process exit itself is not an owner action;
/// the durable receipt follows the post-reap state check and journal readback.
fn record_current_absent_after_reap(
    state: &mut OwnerState,
    deadline: Option<Instant>,
    observed_launch: Option<&ReapedLaunchObservation>,
    normal_exit: bool,
) -> io::Result<()> {
    if state.current.is_some() || !state.launches.is_empty() || state.lifecycle.active_len() != 0 {
        return Err(io::Error::other(
            "C3 child or listener lease remains while recording an observed slot exit",
        ));
    }
    let Some(journal) = state.durable_journal.clone() else {
        return Ok(());
    };
    let unknown = || c3_journal_io_error(C3SlotJournalError::Unknown);
    let readback = match deadline {
        Some(deadline) => journal.readback_until(deadline),
        None => journal.readback(),
    };
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(unknown());
    }
    match readback {
        C3SlotJournalReadback::Empty => Ok(()),
        C3SlotJournalReadback::Unknown => Err(unknown()),
        C3SlotJournalReadback::Committed(row) if row.state() == C3SlotState::ClearedAndAbsent => {
            Ok(())
        }
        C3SlotJournalReadback::Committed(row) => {
            let service_id = row.service_id().to_owned();
            let catalog_generation = NonZeroU64::new(row.catalog_generation())
                .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
            let c3_launch_generation = NonZeroU64::new(row.c3_launch_generation())
                .ok_or_else(|| c3_journal_io_error(C3SlotJournalError::Unknown))?;
            let scope_binding_digest = *row.scope_binding_digest();
            let launch_evidence = row.launch_evidence();
            if let Some(evidence) = launch_evidence {
                let Some(observed_launch) = observed_launch else {
                    return Err(unknown());
                };
                if evidence.process_id() != observed_launch.process_id
                    || Some(evidence.launch_operation_id()) != observed_launch.launch_instance_id
                    || row.scope_snapshot() != Some(&observed_launch.scope)
                    || observed_launch.scope.service_id != service_id
                    || observed_launch.scope.catalog_generation != catalog_generation
                    || observed_launch.scope.c3_launch_generation != c3_launch_generation
                    || observed_launch.scope.binding_digest != scope_binding_digest
                {
                    return Err(unknown());
                }
                if normal_exit {
                    if !evidence.is_normal_exit() && row.c1_operation_id().is_some() {
                        let correlation = C1RecoveryOperationCorrelation::new(
                            row.c1_operation_id().expect("checked C1 operation ID"),
                            scope_binding_digest,
                        );
                        let transition = journal.record_normal_exit_after_reap(
                            C3ReapedLaunchKey {
                                catalog_generation,
                                c3_launch_generation,
                                scope_binding_digest,
                                launch_operation_id: observed_launch
                                    .launch_instance_id
                                    .ok_or_else(unknown)?,
                                process_id: observed_launch.process_id,
                            },
                            correlation,
                            deadline,
                            || {
                                if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                                    Err(io::Error::new(
                                        io::ErrorKind::TimedOut,
                                        "natural-exit journal intent exceeded reaper budget",
                                    ))
                                } else if state.current.is_none()
                                    && state.launches.is_empty()
                                    && state.lifecycle.active_len() == 0
                                {
                                    Ok(())
                                } else {
                                    Err(io::Error::other(
                                        "C3 owner changed while recording exact normal exit",
                                    ))
                                }
                            },
                        );
                        let receipt = transition.map_err(c3_journal_io_error)?;
                        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                            return Err(unknown());
                        }
                        if !exact_durable_receipt(
                            &receipt,
                            &service_id,
                            catalog_generation,
                            c3_launch_generation,
                            &scope_binding_digest,
                            C3SlotState::ClearedAndAbsent,
                        ) || receipt.c1_operation_id() != Some(correlation.operation_id())
                        {
                            return Err(unknown());
                        }
                        state.highest_operation_id =
                            state.highest_operation_id.max(receipt.operation_id().get());
                        return Ok(());
                    }
                    if evidence.is_normal_exit() {
                        return Ok(());
                    }
                }
            }
            let effect = || {
                if let Some(deadline) = deadline {
                    retirement_budget_remaining(deadline).map_err(|_| {
                        io::Error::new(
                            io::ErrorKind::TimedOut,
                            "natural-exit journal transition exceeded reaper budget",
                        )
                    })?;
                }
                if state.current.is_none() {
                    Ok(())
                } else {
                    Err(io::Error::other(
                        "C3 slot gained a current child during exit reconciliation",
                    ))
                }
            };
            let transition = match deadline {
                Some(deadline) => journal.transition_next_until(
                    catalog_generation,
                    c3_launch_generation,
                    scope_binding_digest,
                    C3SlotState::ClearedAndAbsent,
                    deadline,
                    effect,
                ),
                None => journal.transition_next(
                    catalog_generation,
                    c3_launch_generation,
                    scope_binding_digest,
                    C3SlotState::ClearedAndAbsent,
                    effect,
                ),
            };
            let receipt = transition.map_err(c3_journal_io_error)?;
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Err(unknown());
            }
            if !exact_durable_receipt(
                &receipt,
                &service_id,
                catalog_generation,
                c3_launch_generation,
                &scope_binding_digest,
                C3SlotState::ClearedAndAbsent,
            ) {
                return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
            }
            state.highest_operation_id =
                state.highest_operation_id.max(receipt.operation_id().get());
            Ok(())
        }
    }
}

fn prune_retired_launches(state: &mut OwnerState) {
    let OwnerState {
        lifecycle,
        launches,
        ..
    } = state;
    launches.retain(|entry| {
        entry.state != LaunchState::Retired
            || lifecycle.tracks_record(&entry.token)
            || entry.identity_listener.is_some()
    });
}

fn find_launch(launches: &[OwnedLaunch], token: LaunchToken) -> Option<&OwnedLaunch> {
    launches.iter().find(|entry| entry.token == token)
}

fn find_launch_mut(
    launches: &mut [OwnedLaunch],
    token: LaunchToken,
    process_id: u32,
) -> Option<&mut OwnedLaunch> {
    launches
        .iter_mut()
        .find(|entry| entry.token == token && entry.process_id == process_id)
}

fn backend_error(error: BackendError) -> io::Error {
    match error {
        BackendError::Cancelled => io::Error::new(io::ErrorKind::Interrupted, "launch cancelled"),
        BackendError::Revoked | BackendError::IdentityRejected | BackendError::ConsentDenied => {
            permission_denied("C3 system-service launch was rejected")
        }
        BackendError::Unavailable => io::Error::new(
            io::ErrorKind::NotFound,
            "C3 system-service launch is unavailable",
        ),
        BackendError::ProtocolViolation => io::Error::new(
            io::ErrorKind::InvalidData,
            "C3 system-service lifecycle protocol violation",
        ),
        BackendError::TransportFailure | BackendError::ServiceFailure => {
            io::Error::other("C3 system-service lifecycle operation failed")
        }
    }
}

fn permission_denied(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::{
        C3IdentityLaunchBinding, C3IdentityListenerLaunchLease, C3LaunchPolicy,
        C3PendingIdentityListener, LinuxC3SystemServiceSlotOwner,
        SystemServiceRecoveryAdmissionScope,
    };
    use crate::linux_init::linux_init_c3_slot_journal::{
        C3SlotJournal, C3SlotJournalReadback, C3SlotState,
    };
    use crate::linux_init::linux_process_resource_tests::namespace_root_probe_elf;
    use crate::linux_init::{LifecycleEvent, ServiceExitStatus};
    use nagi_system_principal_contract::{
        SystemServiceEndpointOwnerProof, SystemServiceLaunchRecord,
    };
    use nagi_system_service_recovery_contract::{
        canonical_recovery_binding_digest, C1RecoveryOperationCorrelation,
        C3DurableRecoverySlotReceipt, C3RecoverySlotGate, C3RecoverySlotState, RecoveryClientError,
        RecoveryLifecycleTransition, SystemServiceRecoveryScopeInput,
    };
    use sha2::{Digest, Sha256};
    use std::ffi::CString;
    use std::io::{Read, Write};
    use std::num::NonZeroU64;
    use std::os::fd::AsFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{
        DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt,
    };
    use std::os::unix::net::UnixStream;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    static NEXT_JOURNAL_DIR: AtomicU64 = AtomicU64::new(1);

    unsafe extern "C" {
        fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> std::ffi::c_int;
    }

    struct JournalDirectory(std::path::PathBuf);

    impl JournalDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nagi-c3-owner-journal-{}-{}",
                std::process::id(),
                NEXT_JOURNAL_DIR.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = std::fs::DirBuilder::new();
            builder.mode(0o700);
            builder.create(&path).unwrap();
            Self(path)
        }

        fn journal_path(&self) -> std::path::PathBuf {
            self.0.join("slot.journal")
        }
    }

    impl Drop for JournalDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct JournalLockHolder {
        child: Child,
        release_path: std::path::PathBuf,
    }

    impl JournalLockHolder {
        fn start(
            directory: &JournalDirectory,
            service_id: &str,
            journal_path: &std::path::Path,
        ) -> Self {
            const JOURNAL_ENV: &str = "NAGI_C3_SLOT_JOURNAL_LOCK_PATH";
            const SERVICE_ENV: &str = "NAGI_C3_SLOT_JOURNAL_LOCK_SERVICE";
            const READY_ENV: &str = "NAGI_C3_SLOT_JOURNAL_LOCK_READY";
            const RELEASE_ENV: &str = "NAGI_C3_SLOT_JOURNAL_LOCK_RELEASE";

            let ready_path = directory.0.join("journal-lock-ready");
            let release_path = directory.0.join("journal-lock-release");
            let test_executable = std::env::args_os().next().expect("test binary argv[0]");
            let mut child = Command::new(test_executable)
                .arg("linux_init::linux_init_c3_slot_journal::tests::c3_slot_journal_child_holds_lock")
                .arg("--exact")
                .env(JOURNAL_ENV, journal_path)
                .env(SERVICE_ENV, service_id)
                .env(READY_ENV, &ready_path)
                .env(RELEASE_ENV, &release_path)
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            loop {
                if ready_path.exists() {
                    return Self {
                        child,
                        release_path,
                    };
                }
                if let Some(status) = child.try_wait().unwrap() {
                    panic!("journal lock helper exited before acquiring lock: {status}");
                }
                assert!(
                    Instant::now() < deadline,
                    "journal lock helper did not become ready"
                );
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }

    impl Drop for JournalLockHolder {
        fn drop(&mut self) {
            let _ = std::fs::write(&self.release_path, b"release");
            let _ = self.child.wait();
        }
    }

    fn observe_owner_restart(
        directory: &JournalDirectory,
        service_id: &str,
        journal_path: &std::path::Path,
        result_name: &str,
    ) -> String {
        const SERVICE_ENV: &str = "NAGI_C3_OWNER_REOPEN_SERVICE";
        const JOURNAL_ENV: &str = "NAGI_C3_OWNER_REOPEN_JOURNAL";
        const RESULT_ENV: &str = "NAGI_C3_OWNER_REOPEN_RESULT";
        let result_path = directory.0.join(result_name);
        let test_executable = std::env::args_os().next().expect("test binary argv[0]");
        let status = Command::new(test_executable)
            .arg("linux_init::linux_init_c3_system_service_owner::tests::c3_slot_journal_child_reopen_observation")
            .arg("--exact")
            .env(SERVICE_ENV, service_id)
            .env(JOURNAL_ENV, journal_path)
            .env(RESULT_ENV, &result_path)
            .status()
            .unwrap();
        assert!(status.success(), "owner-reopen child failed: {status}");
        std::fs::read_to_string(result_path).unwrap()
    }

    struct FixtureScope {
        service_id: String,
        catalog_generation: NonZeroU64,
        transition: RecoveryLifecycleTransition,
        c3_launch_generation: NonZeroU64,
        uid: u32,
        gid: u32,
        executable_sha256: [u8; 32],
        binding: [u8; 32],
    }

    impl FixtureScope {
        fn new(
            service_id: &str,
            catalog_generation: u64,
            transition: RecoveryLifecycleTransition,
            c3_launch_generation: NonZeroU64,
        ) -> Self {
            let image = namespace_root_probe_elf();
            Self::new_with_image(
                service_id,
                catalog_generation,
                transition,
                c3_launch_generation,
                &image,
            )
        }

        fn new_with_image(
            service_id: &str,
            catalog_generation: u64,
            transition: RecoveryLifecycleTransition,
            c3_launch_generation: NonZeroU64,
            image: &[u8],
        ) -> Self {
            let service_id = service_id.to_owned();
            let catalog_generation = NonZeroU64::new(catalog_generation).unwrap();
            let signed_package_sha256 = [0x31; 32];
            let executable_sha256: [u8; 32] = Sha256::digest(image).into();
            let binding = canonical_recovery_binding_digest(&SystemServiceRecoveryScopeInput {
                service_id: &service_id,
                principal_id: "service:fixture.identity",
                caller_context_id: "caller_fixture_identity",
                mapping_generation: NonZeroU64::new(1).unwrap(),
                role: "system_service",
                catalog_generation,
                transition,
                package_id: "org.nagi.fixture",
                package_app_id: 0x1122,
                canonical_entry: "bin/service",
                signed_package_sha256: &signed_package_sha256,
                executable_sha256: &executable_sha256,
                package_generation: 1,
                c3_launch_generation,
                uid: 65_534,
                gid: 65_534,
            });
            Self {
                service_id,
                catalog_generation,
                transition,
                c3_launch_generation,
                uid: 65_534,
                gid: 65_534,
                executable_sha256,
                binding,
            }
        }
    }

    impl SystemServiceRecoveryAdmissionScope for FixtureScope {
        fn service_id(&self) -> &str {
            &self.service_id
        }
        fn principal_id(&self) -> &str {
            "service:fixture.identity"
        }
        fn caller_context_id(&self) -> &str {
            "caller_fixture_identity"
        }
        fn mapping_generation(&self) -> NonZeroU64 {
            NonZeroU64::new(1).unwrap()
        }
        fn role(&self) -> &str {
            "system_service"
        }
        fn catalog_generation(&self) -> NonZeroU64 {
            self.catalog_generation
        }
        fn transition(&self) -> RecoveryLifecycleTransition {
            self.transition
        }
        fn package_id(&self) -> &str {
            "org.nagi.fixture"
        }
        fn package_app_id(&self) -> u64 {
            0x1122
        }
        fn canonical_entry(&self) -> &str {
            "bin/service"
        }
        fn signed_package_sha256(&self) -> &[u8; 32] {
            &[0x31; 32]
        }
        fn executable_sha256(&self) -> &[u8; 32] {
            &self.executable_sha256
        }
        fn package_generation(&self) -> u64 {
            1
        }
        fn c3_launch_generation(&self) -> NonZeroU64 {
            self.c3_launch_generation
        }
        fn credentials(&self) -> (u32, u32) {
            (self.uid, self.gid)
        }
        fn binding_digest(&self) -> &[u8; 32] {
            &self.binding
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum IdentityListenerPhase {
        BoundBeforeExec,
        ActivatedAfterExecAck,
        RetiredAfterReap,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct IdentityListenerEvent {
        phase: IdentityListenerPhase,
        process_id: u32,
        exact_spawn: bool,
        pidfd_live: bool,
        attested: bool,
        launch_record_revoked: bool,
    }

    struct RecordingPendingIdentityListener {
        events: Arc<Mutex<Vec<IdentityListenerEvent>>>,
        launch_record_probe: Option<Arc<Mutex<Option<SystemServiceLaunchRecord>>>>,
        owner_state_probe: Option<std::sync::Weak<Mutex<super::OwnerState>>>,
        owner_lock_available: Option<Arc<std::sync::atomic::AtomicBool>>,
        pending: bool,
    }

    impl RecordingPendingIdentityListener {
        fn new(events: Arc<Mutex<Vec<IdentityListenerEvent>>>) -> Self {
            Self {
                events,
                launch_record_probe: None,
                owner_state_probe: None,
                owner_lock_available: None,
                pending: true,
            }
        }

        fn with_launch_record_probe(
            events: Arc<Mutex<Vec<IdentityListenerEvent>>>,
            launch_record_probe: Arc<Mutex<Option<SystemServiceLaunchRecord>>>,
            owner_state_probe: std::sync::Weak<Mutex<super::OwnerState>>,
            owner_lock_available: Arc<std::sync::atomic::AtomicBool>,
        ) -> Self {
            Self {
                events,
                launch_record_probe: Some(launch_record_probe),
                owner_state_probe: Some(owner_state_probe),
                owner_lock_available: Some(owner_lock_available),
                pending: true,
            }
        }
    }

    impl C3PendingIdentityListener for RecordingPendingIdentityListener {
        fn bind_pending_before_exec(
            &mut self,
            child: nagi_init_spawn_contract::SpawnedProcessPidfd,
            binding: &C3IdentityLaunchBinding,
        ) -> std::io::Result<Box<dyn C3IdentityListenerLaunchLease>> {
            if !self.pending {
                return Err(std::io::Error::other(
                    "listener registration was already transferred",
                ));
            }
            let process_id = child.process_id();
            let exact_spawn = child.identity().same_instance(binding.process_identity());
            let pidfd_live = child.as_raw_fd().is_ok();
            if !exact_spawn
                || !pidfd_live
                || !binding.matches_launch_record(binding.launch_record())
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "listener did not receive the exact prepared child and C3 launch record",
                ));
            }
            let launch_record = binding.launch_record().clone();
            if let Some(probe) = &self.launch_record_probe {
                let mut probe = probe.lock().unwrap();
                if probe.is_some() {
                    return Err(std::io::Error::other(
                        "listener already captured a C3 launch record",
                    ));
                }
                *probe = Some(launch_record.clone());
            }
            self.events.lock().unwrap().push(IdentityListenerEvent {
                phase: IdentityListenerPhase::BoundBeforeExec,
                process_id,
                exact_spawn,
                pidfd_live,
                attested: false,
                launch_record_revoked: false,
            });
            self.pending = false;
            Ok(Box::new(RecordingIdentityListenerLease {
                service_id: binding.service_id().to_owned(),
                catalog_generation: binding.catalog_generation(),
                c3_launch_generation: binding.c3_launch_generation(),
                scope_binding_digest: *binding.scope_binding_digest(),
                launch_record,
                child,
                events: Arc::clone(&self.events),
                launch_record_probe: self.launch_record_probe.as_ref().map(Arc::clone),
                owner_state_probe: self.owner_state_probe.clone(),
                owner_lock_available: self.owner_lock_available.as_ref().map(Arc::clone),
            }))
        }
    }

    struct RecordingIdentityListenerLease {
        service_id: String,
        catalog_generation: NonZeroU64,
        c3_launch_generation: NonZeroU64,
        scope_binding_digest: [u8; 32],
        launch_record: SystemServiceLaunchRecord,
        child: nagi_init_spawn_contract::SpawnedProcessPidfd,
        events: Arc<Mutex<Vec<IdentityListenerEvent>>>,
        launch_record_probe: Option<Arc<Mutex<Option<SystemServiceLaunchRecord>>>>,
        owner_state_probe: Option<std::sync::Weak<Mutex<super::OwnerState>>>,
        owner_lock_available: Option<Arc<std::sync::atomic::AtomicBool>>,
    }

    impl C3IdentityListenerLaunchLease for RecordingIdentityListenerLease {
        fn matches_binding(&self, binding: &C3IdentityLaunchBinding) -> bool {
            self.service_id == binding.service_id()
                && self.catalog_generation == binding.catalog_generation()
                && self.c3_launch_generation == binding.c3_launch_generation()
                && self.scope_binding_digest == *binding.scope_binding_digest()
                && self
                    .child
                    .identity()
                    .same_instance(binding.process_identity())
                && binding.matches_launch_record(&self.launch_record)
                && binding.matches_launch_record(binding.launch_record())
        }

        fn activate_after_exec_ack(
            &mut self,
            binding: &C3IdentityLaunchBinding,
            attestation: &super::super::FdDelegationRestrictionAttestation,
        ) -> std::io::Result<()> {
            if let (Some(owner_state), Some(owner_lock_available)) =
                (&self.owner_state_probe, &self.owner_lock_available)
            {
                let owner_lock_released =
                    owner_state
                        .upgrade()
                        .is_some_and(|state| match state.try_lock() {
                            Ok(guard) => {
                                drop(guard);
                                true
                            }
                            Err(_) => false,
                        });
                owner_lock_available.store(owner_lock_released, Ordering::SeqCst);
                if !owner_lock_released {
                    return Err(std::io::Error::other(
                        "C3 owner-state lock remained held during listener activation",
                    ));
                }
            }
            let exact_spawn = self.matches_binding(binding);
            let attested = attestation.matches_process(binding.process_identity());
            let pidfd_live = self.child.as_raw_fd().is_ok();
            if !exact_spawn || !attested || !pidfd_live {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "listener activation did not match the attested child",
                ));
            }
            self.events.lock().unwrap().push(IdentityListenerEvent {
                phase: IdentityListenerPhase::ActivatedAfterExecAck,
                process_id: self.child.process_id(),
                exact_spawn,
                pidfd_live,
                attested,
                launch_record_revoked: false,
            });
            Ok(())
        }

        fn retire_after_exact_reap(
            &mut self,
            binding: &C3IdentityLaunchBinding,
        ) -> std::io::Result<()> {
            let exact_spawn = self.matches_binding(binding);
            let pidfd_live = self.child.as_raw_fd().is_ok();
            let probe_record_revoked = self
                .launch_record_probe
                .as_ref()
                .map(|probe| {
                    probe
                        .lock()
                        .unwrap()
                        .as_ref()
                        .is_some_and(|record| !record.is_live())
                })
                .unwrap_or(true);
            let launch_record_revoked = !self.launch_record.is_live() && probe_record_revoked;
            if !exact_spawn
                || !pidfd_live
                || (self.launch_record_probe.is_some() && !launch_record_revoked)
            {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "listener retirement preceded exact launch-record revocation or did not match its reaped child",
                ));
            }
            self.events.lock().unwrap().push(IdentityListenerEvent {
                phase: IdentityListenerPhase::RetiredAfterReap,
                process_id: self.child.process_id(),
                exact_spawn,
                pidfd_live,
                attested: false,
                launch_record_revoked,
            });
            Ok(())
        }
    }

    struct OwnerShutdownGuard<'a>(&'a LinuxC3SystemServiceSlotOwner);

    impl Drop for OwnerShutdownGuard<'_> {
        fn drop(&mut self) {
            let _ = self
                .0
                .shutdown(Duration::from_secs(2), Duration::from_millis(10));
        }
    }

    const TEST_LAUNCH_POLICY: C3LaunchPolicy = C3LaunchPolicy {
        required: true,
        release_wait_budget: Duration::from_secs(10),
        cancel_check_interval: Duration::from_millis(20),
        retirement_wait_budget: Duration::from_secs(2),
        reaper_poll_interval: Duration::from_millis(10),
    };

    fn launch(owner: &LinuxC3SystemServiceSlotOwner, scope: &FixtureScope) {
        let image = namespace_root_probe_elf();
        launch_image(owner, scope, &image).unwrap();
    }

    fn launch_image(
        owner: &LinuxC3SystemServiceSlotOwner,
        scope: &FixtureScope,
        image: &[u8],
    ) -> std::io::Result<()> {
        unsafe { owner.launch_admitted(scope, image, &[], TEST_LAUNCH_POLICY, || false) }
    }

    fn retained_owner_state(service_id: &str) -> Arc<Mutex<super::OwnerState>> {
        let registry = super::SLOT_OWNER_REGISTRY.lock().unwrap();
        let registration = registry
            .iter()
            .find(|entry| entry.service_id == service_id)
            .expect("dropped owner retains its exact slot state");
        Arc::clone(&registration.state)
    }

    fn retained_active_child_count(state: &Mutex<super::OwnerState>) -> usize {
        let state = match state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.lifecycle.active_len()
    }

    fn poison_owner_state_after_current_revoke(owner: &LinuxC3SystemServiceSlotOwner) {
        let poison = catch_unwind(AssertUnwindSafe(|| {
            let mut state = owner.state.lock().unwrap();
            state.fail_retirement_after_revoke_once = true;
            panic!("fixture poisons C3 OwnerState while retaining a child");
        }));
        assert!(poison.is_err());
    }

    fn issue(
        owner: &LinuxC3SystemServiceSlotOwner,
        scope: &FixtureScope,
    ) -> nagi_system_principal_contract::SystemServiceEndpointOwnerProof {
        let endpoint = owner.take_current_service_endpoint(scope).unwrap();
        owner
            .issue_current_service_endpoint(scope, endpoint)
            .unwrap()
    }

    fn launch_and_authenticate(
        owner: &LinuxC3SystemServiceSlotOwner,
        scope: &FixtureScope,
    ) -> nagi_system_principal_contract::SystemServiceEndpointOwnerProof {
        let image = namespace_root_probe_elf();
        unsafe {
            owner
                .launch_and_authenticate_admitted(scope, &image, &[], TEST_LAUNCH_POLICY, || false)
                .unwrap()
        }
    }

    fn confirm_isolated_endpoint(
        proof: &nagi_system_principal_contract::SystemServiceEndpointOwnerProof,
    ) {
        let endpoint = proof.endpoint().unwrap().try_clone_to_owned().unwrap();
        let mut stream = UnixStream::from(endpoint);
        let mut marker = [0u8; 8];
        stream.read_exact(&mut marker).unwrap();
        assert_eq!(&marker, b"ISOLATED");
    }

    fn allow_probe_to_exit(
        proof: &nagi_system_principal_contract::SystemServiceEndpointOwnerProof,
    ) {
        allow_probe_to_exit_with_marker(proof, b"ISOLATED");
    }

    fn allow_probe_to_exit_with_marker(
        proof: &nagi_system_principal_contract::SystemServiceEndpointOwnerProof,
        expected_marker: &[u8; 8],
    ) {
        let endpoint = proof.endpoint().unwrap().try_clone_to_owned().unwrap();
        let mut stream = UnixStream::from(endpoint);
        let mut marker = [0u8; 8];
        stream.read_exact(&mut marker).unwrap();
        assert_eq!(&marker, expected_marker);
        stream.write_all(b"Q").unwrap();
    }

    fn allow_normal_probe_to_exit(
        proof: &SystemServiceEndpointOwnerProof,
        expected_image_marker: &[u8; 8],
    ) {
        let endpoint = proof.endpoint().unwrap().try_clone_to_owned().unwrap();
        let mut stream = UnixStream::from(endpoint);
        let mut launch_marker = [0u8; 8];
        stream.read_exact(&mut launch_marker).unwrap();
        assert_eq!(&launch_marker, expected_image_marker);
        stream.write_all(b"Q").unwrap();

        let mut normal_exit_marker = [0u8; 8];
        stream.read_exact(&mut normal_exit_marker).unwrap();
        assert_eq!(
            &normal_exit_marker, expected_image_marker,
            "normal exit marker must match the executable used for this launch"
        );
    }

    fn namespace_root_probe_elf_with_generation_marker(marker: [u8; 8]) -> Vec<u8> {
        const CODE_START: usize = 120;
        const PATH_OFFSET: usize = 320;
        const ISOLATED_MARKER_OFFSET: usize = 352;
        const NORMAL_EXIT_MARKER_OFFSET: usize = 368;
        const NORMAL_EXIT_SEQUENCE: [u8; 9] = [
            0xb8, 0x3c, 0x00, 0x00, 0x00, // mov eax, SYS_exit
            0x31, 0xff, // xor edi, edi
            0x0f, 0x05, // syscall
        ];
        const VISIBLE_EXIT_SEQUENCE: [u8; 12] = [
            0xb8, 0x3c, 0x00, 0x00, 0x00, // mov eax, SYS_exit
            0xbf, 0x01, 0x00, 0x00, 0x00, // mov edi, 1
            0x0f, 0x05, // syscall
        ];

        let mut image = namespace_root_probe_elf();
        let code = &mut image[CODE_START..PATH_OFFSET];
        let normal_exit_offset = code
            .windows(NORMAL_EXIT_SEQUENCE.len())
            .position(|candidate| candidate == NORMAL_EXIT_SEQUENCE.as_slice())
            .expect("isolated namespace probe has a normal exit sequence");
        let handler_offset = code
            .windows(VISIBLE_EXIT_SEQUENCE.len())
            .rposition(|candidate| candidate == VISIBLE_EXIT_SEQUENCE.as_slice())
            .map(|offset| offset + VISIBLE_EXIT_SEQUENCE.len())
            .expect("visible namespace path has a terminal exit sequence");

        let mut exit_handler = Vec::new();
        exit_handler.extend_from_slice(&[0xb8, 0x01, 0x00, 0x00, 0x00]); // mov eax, SYS_write
        exit_handler.extend_from_slice(&[0xbf, 0x03, 0x00, 0x00, 0x00]); // mov edi, fd 3
        exit_handler.push(0xbe); // mov esi, absolute marker address
        exit_handler
            .extend_from_slice(&((0x400000 + NORMAL_EXIT_MARKER_OFFSET) as u32).to_le_bytes());
        exit_handler.extend_from_slice(&[0xba, 0x08, 0x00, 0x00, 0x00, 0x0f, 0x05]); // write 8
        exit_handler.extend_from_slice(&[0xb8, 0x3c, 0x00, 0x00, 0x00]); // mov eax, SYS_exit
        exit_handler.extend_from_slice(&[0x31, 0xff, 0x0f, 0x05]); // exit 0
        assert!(handler_offset + exit_handler.len() < code.len());

        let jump_displacement =
            i32::try_from(handler_offset as isize - (normal_exit_offset + 5) as isize)
                .expect("fixture marker handler fits a relative jump");
        code[normal_exit_offset] = 0xe9; // jmp rel32 to the normal-exit marker handler
        code[normal_exit_offset + 1..normal_exit_offset + 5]
            .copy_from_slice(&jump_displacement.to_le_bytes());
        code[handler_offset..handler_offset + exit_handler.len()].copy_from_slice(&exit_handler);

        image[ISOLATED_MARKER_OFFSET..ISOLATED_MARKER_OFFSET + marker.len()]
            .copy_from_slice(&marker);
        image[NORMAL_EXIT_MARKER_OFFSET..NORMAL_EXIT_MARKER_OFFSET + marker.len()]
            .copy_from_slice(&marker);
        image
    }

    fn finish_normal_identity_listener_lifecycle(
        owner: &LinuxC3SystemServiceSlotOwner,
        service_id: &str,
        proof: &SystemServiceEndpointOwnerProof,
        launch_token: super::LaunchToken,
        listener_state: (
            &Mutex<Vec<IdentityListenerEvent>>,
            &Mutex<super::OwnerState>,
        ),
        launch_record: &SystemServiceLaunchRecord,
        expected_marker: &[u8; 8],
    ) {
        let (events, retained_state) = listener_state;
        let process_id = proof.process_identity().process_id();
        allow_normal_probe_to_exit(proof, expected_marker);

        let reap_deadline = Instant::now() + Duration::from_secs(5);
        let mut service_exit_count = 0;
        loop {
            if let Some(event) = owner.poll().unwrap() {
                match event {
                    LifecycleEvent::ServiceExited {
                        service,
                        status,
                        required,
                    } => {
                        service_exit_count += 1;
                        assert_eq!(service_exit_count, 1, "normal exit is notified once");
                        assert_eq!(service, service_id);
                        assert_eq!(
                            status,
                            ServiceExitStatus {
                                exit_code: Some(0),
                                signal: None,
                            }
                        );
                        assert!(required);
                    }
                    other => panic!("expected normal service exit, got {other:?}"),
                }
            }
            let retired_count = events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event.phase == IdentityListenerPhase::RetiredAfterReap)
                .count();
            if service_exit_count == 1
                && retired_count == 1
                && owner.active_child_count().unwrap() == 0
            {
                break;
            }
            assert!(
                Instant::now() < reap_deadline,
                "normal exact reap did not retire the Identity listener"
            );
            std::thread::sleep(Duration::from_millis(10));
        }

        assert!(
            owner.poll().unwrap().is_none(),
            "re-poll after normal termination must not emit a duplicate exit"
        );
        assert!(!proof.is_live().unwrap());
        assert!(!launch_record.is_live());

        let events_after_reap = events.lock().unwrap().clone();
        assert_eq!(
            events_after_reap
                .iter()
                .map(|event| event.phase)
                .collect::<Vec<_>>(),
            vec![
                IdentityListenerPhase::BoundBeforeExec,
                IdentityListenerPhase::ActivatedAfterExecAck,
                IdentityListenerPhase::RetiredAfterReap,
            ]
        );
        assert!(events_after_reap.iter().all(|event| {
            event.process_id == process_id && event.exact_spawn && event.pidfd_live
        }));
        assert!(events_after_reap[1].attested);
        assert!(
            !events_after_reap[0].launch_record_revoked
                && !events_after_reap[1].launch_record_revoked
                && events_after_reap[2].launch_record_revoked,
            "the launch record is revoked before its listener lease retires"
        );

        let state = retained_state.lock().unwrap();
        assert!(
            state.current.is_none(),
            "normal reap clears the current slot"
        );
        assert_eq!(state.lifecycle.active_len(), 0);
        assert!(
            super::find_launch(&state.launches, launch_token).is_none(),
            "normal exact reap retires and prunes the launch"
        );
        assert!(
            state.launches.is_empty(),
            "the slot is empty before the next Identity generation starts"
        );
    }

    fn namespace_root_sigill_probe_elf() -> Vec<u8> {
        const EXIT_SEQUENCE: [u8; 9] = [
            0xb8, 0x3c, 0x00, 0x00, 0x00, // mov eax, SYS_exit
            0x31, 0xff, // xor edi, edi
            0x0f, 0x05, // syscall
        ];

        let mut image = namespace_root_probe_elf();
        let code = &mut image[120..320];
        let exit_offset = code
            .windows(EXIT_SEQUENCE.len())
            .position(|candidate| candidate == EXIT_SEQUENCE.as_slice())
            .expect("isolated namespace probe ends with its controlled exit");
        // Keep the real endpoint handshake, then make the released child
        // terminate through Linux SIGILL for the C3 owner regression.
        code[exit_offset..exit_offset + 2].copy_from_slice(&[0x0f, 0x0b]);
        image
    }

    #[test]
    fn exact_slot_owner_issues_proof_replaces_revokes_and_reaps_real_clone3_children() {
        let service_id = "system.fixture.owner-replacement";
        let owner = LinuxC3SystemServiceSlotOwner::new(service_id).unwrap();
        let generation1 = owner.reserve_launch_generation().unwrap();
        let scope1 = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation1,
        );
        launch(&owner, &scope1);
        let first = issue(&owner, &scope1);
        assert_eq!(first.launch_generation(), generation1);
        assert!(first.is_live().unwrap());
        confirm_isolated_endpoint(&first);
        let first_process_identity = first.process_identity();
        let exact_live_attestation = owner
            .with_current_fd_delegation_prevention(
                &scope1,
                &first_process_identity,
                |attestation| {
                    attestation.service_id() == service_id
                        && attestation.catalog_generation().get() == 1
                        && attestation.launch_generation() == generation1
                        && attestation.scope_binding_digest() == &scope1.binding
                        && attestation
                            .process_identity()
                            .same_instance(&first_process_identity)
                        && attestation
                            .c2_fd_delegation_restriction()
                            .matches_process(&first_process_identity)
                },
            )
            .unwrap();
        assert!(exact_live_attestation);

        let generation2 = owner.reserve_launch_generation().unwrap();
        let scope2 = FixtureScope::new(
            service_id,
            2,
            RecoveryLifecycleTransition::Replace {
                previous_generation: NonZeroU64::new(1).unwrap(),
            },
            generation2,
        );
        launch(&owner, &scope2);
        assert!(!first.is_live().unwrap());
        let effects = AtomicUsize::new(0);
        let stale_effect =
            first.with_live_authority_effect(|| effects.fetch_add(1, Ordering::SeqCst));
        assert_eq!(
            stale_effect.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert_eq!(effects.load(Ordering::SeqCst), 0);

        let second = issue(&owner, &scope2);
        assert_eq!(second.launch_generation(), generation2);
        assert!(second.is_live().unwrap());
        confirm_isolated_endpoint(&second);
        owner
            .revoke_current(Duration::from_secs(2), Duration::from_millis(10))
            .unwrap();
        assert_eq!(owner.active_child_count().unwrap(), 0);
        assert!(!second.is_live().unwrap());
    }

    #[test]
    fn identity_listener_handoff_binds_exact_spawn_retains_and_retires_after_normal_exit() {
        let service_id = "system.fixture.identity-listener-normal-exit";
        let owner = LinuxC3SystemServiceSlotOwner::new(service_id).unwrap();
        let _shutdown = OwnerShutdownGuard(&owner);
        let generation = owner.reserve_launch_generation().unwrap();
        let scope = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let launch_record_probe = Arc::new(Mutex::new(None));
        let retained_state = retained_owner_state(service_id);
        let owner_lock_available = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut pending_listener = RecordingPendingIdentityListener::with_launch_record_probe(
            Arc::clone(&events),
            Arc::clone(&launch_record_probe),
            Arc::downgrade(&retained_state),
            Arc::clone(&owner_lock_available),
        );
        let image = namespace_root_probe_elf();
        let proof = unsafe {
            owner.launch_and_authenticate_admitted_with_identity_listener(
                &scope,
                &image,
                &[],
                TEST_LAUNCH_POLICY,
                &mut pending_listener,
                || false,
            )
        }
        .unwrap();
        let process_id = proof.process_identity().process_id();
        assert!(proof.is_live().unwrap());
        assert!(!pending_listener.pending);
        assert!(
            owner_lock_available.load(Ordering::SeqCst),
            "C2 activation callback must run after releasing the C3 owner-state lock"
        );

        let bound_and_active = events.lock().unwrap().clone();
        assert_eq!(
            bound_and_active
                .iter()
                .map(|event| event.phase)
                .collect::<Vec<_>>(),
            vec![
                IdentityListenerPhase::BoundBeforeExec,
                IdentityListenerPhase::ActivatedAfterExecAck,
            ]
        );
        assert!(bound_and_active.iter().all(|event| {
            event.process_id == process_id && event.exact_spawn && event.pidfd_live
        }));
        assert!(bound_and_active[1].attested);

        let launch_token = {
            let state = retained_state.lock().unwrap();
            let token = state.current.expect("normal launch remains current");
            let entry = super::find_launch(&state.launches, token)
                .expect("normal launch remains in the C3 owner");
            let bound = entry
                .identity_listener
                .as_ref()
                .expect("C3 retains the bound Identity listener lease");
            assert_eq!(bound.activation, super::IdentityListenerActivation::Active);
            assert_eq!(
                bound.retirement,
                super::IdentityListenerRetirement::NotAttempted
            );
            assert!(entry.launch_record.is_live());
            token
        };

        // A may be servicing ordinary Notes input while this required child
        // remains live. Its bounded wait must keep driving C3 poll without
        // manufacturing an exit event or retiring the active listener lease.
        for _ in 0..3 {
            assert!(
                owner.poll().unwrap().is_none(),
                "a live Identity child must not notify exit during input wait"
            );
            assert!(proof.is_live().unwrap());
            let state = retained_state.lock().unwrap();
            assert_eq!(state.current, Some(launch_token));
            assert_eq!(state.lifecycle.active_len(), 1);
            let entry = super::find_launch(&state.launches, launch_token)
                .expect("input wait retains the current C3 launch");
            let bound = entry
                .identity_listener
                .as_ref()
                .expect("input wait retains the C2 listener lease");
            assert_eq!(bound.activation, super::IdentityListenerActivation::Active);
            assert_eq!(
                bound.retirement,
                super::IdentityListenerRetirement::NotAttempted
            );
            drop(state);
            std::thread::sleep(Duration::from_millis(10));
        }

        // The real C3 child fixture exits normally only after this control
        // byte. Its ISOLATED marker is fixture coordination, not Identity READY.
        allow_probe_to_exit(&proof);
        let reap_deadline = Instant::now() + Duration::from_secs(5);
        let mut service_exit_seen = false;
        loop {
            if let Some(event) = owner.poll().unwrap() {
                match event {
                    LifecycleEvent::ServiceExited {
                        service,
                        status,
                        required,
                    } => {
                        assert!(!service_exit_seen, "normal exit must be reported once");
                        assert_eq!(service, service_id);
                        assert_eq!(
                            status,
                            ServiceExitStatus {
                                exit_code: Some(0),
                                signal: None,
                            }
                        );
                        assert!(required);
                        service_exit_seen = true;
                    }
                    other => panic!("expected normal service exit, got {other:?}"),
                }
            }
            let retired = events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event.phase == IdentityListenerPhase::RetiredAfterReap);
            if retired && owner.active_child_count().unwrap() == 0 {
                break;
            }
            assert!(
                Instant::now() < reap_deadline,
                "normal child reap did not retire the listener"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            service_exit_seen,
            "normal child exit event was not observed"
        );
        assert!(
            owner.poll().unwrap().is_none(),
            "re-poll after normal termination must not emit a duplicate exit"
        );
        assert!(!proof.is_live().unwrap());
        drop(proof);

        let events_after_reap = events.lock().unwrap().clone();
        assert_eq!(
            events_after_reap
                .iter()
                .map(|event| event.phase)
                .collect::<Vec<_>>(),
            vec![
                IdentityListenerPhase::BoundBeforeExec,
                IdentityListenerPhase::ActivatedAfterExecAck,
                IdentityListenerPhase::RetiredAfterReap,
            ]
        );
        assert!(events_after_reap.iter().all(|event| {
            event.process_id == process_id && event.exact_spawn && event.pidfd_live
        }));
        assert!(
            !events_after_reap[0].launch_record_revoked
                && !events_after_reap[1].launch_record_revoked
                && events_after_reap[2].launch_record_revoked,
            "the Supervisor record must be revoked before the listener lease retires"
        );

        let state = retained_state.lock().unwrap();
        assert!(
            state.current.is_none(),
            "normal reap revokes the current launch"
        );
        assert_eq!(state.lifecycle.active_len(), 0);
        assert!(
            super::find_launch(&state.launches, launch_token).is_none(),
            "exact reap retires the C3-held listener lease and prunes its launch"
        );
    }

    #[test]
    fn identity_listener_handoff_restarts_same_owner_after_normal_exit() {
        let service_id = "system.fixture.identity-listener-normal-restart";
        let journal_directory = JournalDirectory::new();
        let owner = LinuxC3SystemServiceSlotOwner::create_with_journal(
            service_id,
            journal_directory.journal_path(),
        )
        .unwrap();
        let _shutdown = OwnerShutdownGuard(&owner);
        assert!(owner
            .latest_durable_normal_exit_scope_snapshot()
            .unwrap()
            .is_none());
        let retained_state = retained_owner_state(service_id);

        let first_generation = owner.reserve_launch_generation().unwrap();
        let first_generation_marker = first_generation.get().to_be_bytes();
        let first_image = namespace_root_probe_elf_with_generation_marker(first_generation_marker);
        let first_scope = FixtureScope::new_with_image(
            service_id,
            2,
            RecoveryLifecycleTransition::Initial,
            first_generation,
            &first_image,
        );
        let first_events = Arc::new(Mutex::new(Vec::new()));
        let first_launch_record_probe = Arc::new(Mutex::new(None));
        let first_owner_lock_available = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut first_listener = RecordingPendingIdentityListener::with_launch_record_probe(
            Arc::clone(&first_events),
            Arc::clone(&first_launch_record_probe),
            Arc::downgrade(&retained_state),
            Arc::clone(&first_owner_lock_available),
        );
        let first_proof = unsafe {
            owner.launch_and_authenticate_admitted_with_identity_listener(
                &first_scope,
                &first_image,
                &[],
                TEST_LAUNCH_POLICY,
                &mut first_listener,
                || false,
            )
        }
        .unwrap();
        let first_c1_operation_id = NonZeroU64::new(1601).unwrap();
        let first_c1_claim =
            C1RecoveryOperationCorrelation::new(first_c1_operation_id, first_scope.binding);
        owner
            .with_durable_live_effect_gate(&first_scope, &first_proof, first_c1_claim, || ())
            .unwrap();
        let first_identity = first_proof.process_identity();
        assert!(first_proof.is_live().unwrap());
        assert!(!first_listener.pending);
        assert!(first_owner_lock_available.load(Ordering::SeqCst));
        assert_eq!(first_proof.launch_generation(), first_generation);

        let first_launch_token = {
            let state = retained_state.lock().unwrap();
            let token = state.current.expect("first launch is current");
            let entry = super::find_launch(&state.launches, token)
                .expect("first generation has a C3 launch entry");
            let binding = entry
                .identity_listener
                .as_ref()
                .expect("first generation retains its C2 listener lease");
            assert_eq!(
                binding.activation,
                super::IdentityListenerActivation::Active
            );
            assert_eq!(
                binding.retirement,
                super::IdentityListenerRetirement::NotAttempted
            );
            assert_eq!(binding.binding.service_id, service_id);
            assert_eq!(
                binding.binding.catalog_generation,
                first_scope.catalog_generation
            );
            assert_eq!(binding.binding.c3_launch_generation, first_generation);
            assert_eq!(binding.binding.scope_binding_digest, first_scope.binding);
            assert!(binding
                .binding
                .process_identity
                .same_instance(&first_identity));
            assert_eq!(entry.launch_record.process_identity(), first_identity);
            assert_eq!(entry.launch_record.generation(), first_generation);
            assert!(entry.launch_record.is_live());
            token
        };
        let first_launch_instance_id = {
            let state = retained_state.lock().unwrap();
            super::find_launch(&state.launches, first_launch_token)
                .and_then(|entry| entry.launch_instance_id)
                .expect("the prepared official record receives a durable C3 launch-instance ID")
        };
        let first_launch_record = first_launch_record_probe
            .lock()
            .unwrap()
            .clone()
            .expect("C2 captured the official first C3 launch record from its binding");
        assert!(first_launch_record.is_live());
        assert_eq!(first_launch_record.generation(), first_generation);
        assert!(first_launch_record
            .process_identity()
            .same_instance(&first_identity));
        let first_record_slot = first_launch_record
            .system_service_slot_identity()
            .expect("first C3 record remains a system-service slot record");
        assert_eq!(first_record_slot.service_id(), service_id);
        assert_eq!(
            first_record_slot.catalog_generation(),
            first_scope.catalog_generation
        );

        finish_normal_identity_listener_lifecycle(
            &owner,
            service_id,
            &first_proof,
            first_launch_token,
            (&first_events, &retained_state),
            &first_launch_record,
            &first_generation_marker,
        );
        drop(first_proof);

        let normal_exit_receipt = owner
            .latest_durable_normal_exit_scope_snapshot()
            .expect("C3 journal readback succeeds")
            .expect("C3 journal reads back the exact old normal-exit scope");
        let old_scope = normal_exit_receipt.scope_snapshot();
        assert_eq!(old_scope.service_id(), service_id);
        assert_eq!(old_scope.principal_id(), first_scope.principal_id());
        assert_eq!(
            old_scope.caller_context_id(),
            first_scope.caller_context_id()
        );
        assert_eq!(
            old_scope.mapping_generation(),
            first_scope.mapping_generation()
        );
        assert_eq!(old_scope.role(), first_scope.role());
        assert_eq!(
            old_scope.catalog_generation(),
            first_scope.catalog_generation()
        );
        assert_eq!(old_scope.transition(), first_scope.transition());
        assert_eq!(old_scope.package_id(), first_scope.package_id());
        assert_eq!(old_scope.package_app_id(), first_scope.package_app_id());
        assert_eq!(old_scope.canonical_entry(), first_scope.canonical_entry());
        assert_eq!(
            old_scope.signed_package_sha256(),
            first_scope.signed_package_sha256()
        );
        assert_eq!(
            old_scope.executable_sha256(),
            first_scope.executable_sha256()
        );
        assert_eq!(
            old_scope.package_generation(),
            first_scope.package_generation()
        );
        assert_eq!(
            old_scope.c3_launch_generation(),
            first_scope.c3_launch_generation()
        );
        assert_eq!(old_scope.credentials(), first_scope.credentials());
        assert_eq!(old_scope.binding_digest(), first_scope.binding_digest());
        assert_eq!(
            normal_exit_receipt.process_id().get(),
            u64::from(first_identity.process_id())
        );
        assert_eq!(
            normal_exit_receipt.launch_instance_id(),
            first_launch_instance_id,
            "cold-readable terminal evidence carries the ID assigned before exec release"
        );
        assert_eq!(normal_exit_receipt.c1_operation_id(), first_c1_operation_id);
        let expired_callback_count = AtomicUsize::new(0);
        let mut expired_terminal_gate =
            owner.durable_normal_exit_gate_until(&normal_exit_receipt, Instant::now());
        assert!(matches!(
            expired_terminal_gate.with_reconciliation_gate(old_scope, first_c1_claim, || {
                expired_callback_count.fetch_add(1, Ordering::SeqCst);
                0
            }),
            Err(RecoveryClientError::OutcomeUnknown)
        ));
        assert_eq!(expired_callback_count.load(Ordering::SeqCst), 0);

        let mut terminal_gate = owner.durable_normal_exit_gate_until(
            &normal_exit_receipt,
            Instant::now() + Duration::from_secs(5),
        );
        assert_eq!(
            terminal_gate
                .with_reconciliation_gate(old_scope, first_c1_claim, || 71)
                .unwrap(),
            71
        );

        assert!(!first_launch_record.is_live());
        assert_eq!(
            first_events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event.phase == IdentityListenerPhase::RetiredAfterReap)
                .count(),
            1,
            "the first listener lease retires exactly once before reuse"
        );

        let second_generation = owner.reserve_launch_generation().unwrap();
        assert_eq!(second_generation.get(), first_generation.get() + 1);

        let changed_image = namespace_root_probe_elf_with_generation_marker(*b"changed!");
        let changed_scope = FixtureScope::new_with_image(
            service_id,
            first_scope.catalog_generation.get(),
            RecoveryLifecycleTransition::Initial,
            second_generation,
            &changed_image,
        );
        let error = launch_image(&owner, &changed_scope, &changed_image)
            .expect_err("a changed executable must not restart under the same catalog");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);

        let lower_catalog_scope = FixtureScope::new_with_image(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            second_generation,
            &first_image,
        );
        let error = launch_image(&owner, &lower_catalog_scope, &first_image)
            .expect_err("an exact normal-exit receipt must not authorize catalog rollback");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(owner.active_child_count().unwrap(), 0);

        let second_image = &first_image;
        let second_scope = FixtureScope::new_with_image(
            service_id,
            first_scope.catalog_generation.get(),
            RecoveryLifecycleTransition::Initial,
            second_generation,
            second_image,
        );
        assert_ne!(second_scope.binding, first_scope.binding);
        let second_events = Arc::new(Mutex::new(Vec::new()));
        let second_launch_record_probe = Arc::new(Mutex::new(None));
        let second_owner_lock_available = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut second_listener = RecordingPendingIdentityListener::with_launch_record_probe(
            Arc::clone(&second_events),
            Arc::clone(&second_launch_record_probe),
            Arc::downgrade(&retained_state),
            Arc::clone(&second_owner_lock_available),
        );
        let second_proof = unsafe {
            owner.launch_and_authenticate_admitted_with_identity_listener(
                &second_scope,
                second_image,
                &[],
                TEST_LAUNCH_POLICY,
                &mut second_listener,
                || false,
            )
        }
        .unwrap();
        let second_identity = second_proof.process_identity();
        assert!(second_proof.is_live().unwrap());
        assert!(!second_listener.pending);
        assert!(second_owner_lock_available.load(Ordering::SeqCst));
        assert_eq!(second_proof.launch_generation(), second_generation);
        assert!(
            !first_identity.same_instance(&second_identity),
            "the restarted generation has an independent process identity"
        );

        let second_launch_token = {
            let state = retained_state.lock().unwrap();
            let token = state.current.expect("second launch is current");
            assert_ne!(token, first_launch_token);
            assert_eq!(state.lifecycle.active_len(), 1);
            assert_eq!(state.launches.len(), 1);
            let entry = super::find_launch(&state.launches, token)
                .expect("second generation has a C3 launch entry");
            let binding = entry
                .identity_listener
                .as_ref()
                .expect("second generation retains its new C2 listener lease");
            assert_eq!(
                binding.activation,
                super::IdentityListenerActivation::Active
            );
            assert_eq!(
                binding.retirement,
                super::IdentityListenerRetirement::NotAttempted
            );
            assert_eq!(binding.binding.service_id, service_id);
            assert_eq!(
                binding.binding.catalog_generation,
                second_scope.catalog_generation
            );
            assert_eq!(binding.binding.c3_launch_generation, second_generation);
            assert_eq!(binding.binding.scope_binding_digest, second_scope.binding);
            assert!(binding
                .binding
                .process_identity
                .same_instance(&second_identity));
            assert_eq!(entry.launch_record.process_identity(), second_identity);
            assert_eq!(entry.launch_record.generation(), second_generation);
            let slot = entry
                .launch_record
                .system_service_slot_identity()
                .expect("second launch remains bound to the same service slot");
            assert_eq!(slot.service_id(), service_id);
            assert_eq!(slot.catalog_generation(), second_scope.catalog_generation);
            assert!(entry.launch_record.is_live());
            token
        };
        let second_launch_record = second_launch_record_probe
            .lock()
            .unwrap()
            .clone()
            .expect("C2 captured the official second C3 launch record from its binding");
        assert!(second_launch_record.is_live());
        assert_eq!(second_launch_record.generation(), second_generation);
        assert!(second_launch_record
            .process_identity()
            .same_instance(&second_identity));
        let second_record_slot = second_launch_record
            .system_service_slot_identity()
            .expect("second C3 record remains a system-service slot record");
        assert_eq!(second_record_slot.service_id(), service_id);
        assert_eq!(
            second_record_slot.catalog_generation(),
            second_scope.catalog_generation
        );

        let second_events_before_exit = second_events.lock().unwrap().clone();
        assert_eq!(
            second_events_before_exit
                .iter()
                .map(|event| event.phase)
                .collect::<Vec<_>>(),
            vec![
                IdentityListenerPhase::BoundBeforeExec,
                IdentityListenerPhase::ActivatedAfterExecAck,
            ]
        );
        assert!(second_events_before_exit.iter().all(|event| {
            event.process_id == second_identity.process_id()
                && event.exact_spawn
                && event.pidfd_live
        }));
        assert!(second_events_before_exit[1].attested);

        for _ in 0..3 {
            assert!(
                owner.poll().unwrap().is_none(),
                "a live second-generation Identity child must not notify exit"
            );
            assert!(second_proof.is_live().unwrap());
            assert!(second_launch_record.is_live());
            let state = retained_state.lock().unwrap();
            assert_eq!(state.current, Some(second_launch_token));
            assert_eq!(state.lifecycle.active_len(), 1);
            let entry = super::find_launch(&state.launches, second_launch_token)
                .expect("the current second-generation launch remains retained");
            let binding = entry
                .identity_listener
                .as_ref()
                .expect("the second-generation listener lease remains retained");
            assert_eq!(
                binding.activation,
                super::IdentityListenerActivation::Active
            );
            assert_eq!(
                binding.retirement,
                super::IdentityListenerRetirement::NotAttempted
            );
            drop(state);
            std::thread::sleep(Duration::from_millis(10));
        }

        finish_normal_identity_listener_lifecycle(
            &owner,
            service_id,
            &second_proof,
            second_launch_token,
            (&second_events, &retained_state),
            &second_launch_record,
            &first_generation_marker,
        );
        drop(second_proof);

        assert!(!first_launch_record.is_live());
        assert!(!second_launch_record.is_live());
        assert_eq!(
            second_events
                .lock()
                .unwrap()
                .iter()
                .filter(|event| event.phase == IdentityListenerPhase::RetiredAfterReap)
                .count(),
            1,
            "the second listener lease retires exactly once"
        );
    }

    #[test]
    fn identity_listener_handoff_retires_once_after_sigill_and_rejects_old_proof_reuse() {
        let service_id = "system.fixture.identity-listener-sigill-exit";
        let journal_directory = JournalDirectory::new();
        let journal_path = journal_directory.journal_path();
        let owner =
            LinuxC3SystemServiceSlotOwner::create_with_journal(service_id, &journal_path).unwrap();
        let _shutdown = OwnerShutdownGuard(&owner);
        let journal = owner_journal(&owner);
        let generation = owner.reserve_launch_generation().unwrap();
        let image = namespace_root_sigill_probe_elf();
        let scope = FixtureScope::new_with_image(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
            &image,
        );
        let events = Arc::new(Mutex::new(Vec::new()));
        let retained_state = retained_owner_state(service_id);
        let mut pending_listener = RecordingPendingIdentityListener::new(Arc::clone(&events));
        let proof = unsafe {
            owner.launch_and_authenticate_admitted_with_identity_listener(
                &scope,
                &image,
                &[],
                TEST_LAUNCH_POLICY,
                &mut pending_listener,
                || false,
            )
        }
        .unwrap();
        let process_identity = proof.process_identity();
        let process_id = process_identity.process_id();
        assert!(proof.is_live().unwrap());
        assert!(!pending_listener.pending);

        let launch_token = {
            let state = retained_state.lock().unwrap();
            let token = state
                .current
                .expect("SIGILL child remains the current launch");
            let entry = super::find_launch(&state.launches, token)
                .expect("SIGILL child has its exact C3 launch record");
            let bound = entry
                .identity_listener
                .as_ref()
                .expect("C3 retains the listener lease until exact reap");
            assert_eq!(bound.activation, super::IdentityListenerActivation::Active);
            assert_eq!(
                bound.retirement,
                super::IdentityListenerRetirement::NotAttempted
            );
            token
        };

        allow_probe_to_exit(&proof);
        let reap_deadline = Instant::now() + Duration::from_secs(5);
        let event = loop {
            if let Some(event) = owner.poll().unwrap() {
                break event;
            }
            assert!(
                Instant::now() < reap_deadline,
                "SIGILL child did not produce an exact C3 lifecycle exit"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        match event {
            LifecycleEvent::ServiceExited {
                service,
                status,
                required,
            } => {
                assert_eq!(service, service_id);
                assert_eq!(
                    status,
                    ServiceExitStatus {
                        exit_code: None,
                        signal: Some(4)
                    }
                );
                assert!(required);
            }
            other => panic!("expected the exact SIGILL child exit, got {other:?}"),
        }

        assert!(!proof.is_live().unwrap());
        let events_after_reap = events.lock().unwrap().clone();
        assert_eq!(
            events_after_reap
                .iter()
                .map(|event| event.phase)
                .collect::<Vec<_>>(),
            vec![
                IdentityListenerPhase::BoundBeforeExec,
                IdentityListenerPhase::ActivatedAfterExecAck,
                IdentityListenerPhase::RetiredAfterReap,
            ]
        );
        assert_eq!(
            events_after_reap
                .iter()
                .filter(|event| event.phase == IdentityListenerPhase::RetiredAfterReap)
                .count(),
            1
        );
        assert!(events_after_reap.iter().all(|event| {
            event.process_id == process_id && event.exact_spawn && event.pidfd_live
        }));
        {
            let state = retained_state.lock().unwrap();
            assert!(state.current.is_none());
            assert_eq!(state.lifecycle.active_len(), 0);
            assert!(super::find_launch(&state.launches, launch_token).is_none());
        }

        let cleared_receipt = match journal.readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.service_id(), service_id);
                assert_eq!(row.catalog_generation(), 1);
                assert_eq!(row.c3_launch_generation(), generation.get());
                assert_eq!(row.scope_binding_digest(), &scope.binding);
                assert_eq!(row.state(), C3SlotState::ClearedAndAbsent);
                (row.durable_revision(), row.operation_id())
            }
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                panic!("exact SIGILL reap did not read back its C3 cleared receipt")
            }
        };

        assert!(owner.poll().unwrap().is_none());
        assert_eq!(*events.lock().unwrap(), events_after_reap);
        match journal.readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.state(), C3SlotState::ClearedAndAbsent);
                assert_eq!(
                    (row.durable_revision(), row.operation_id()),
                    cleared_receipt,
                    "a repeated poll must not append a duplicate terminal receipt"
                );
            }
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                panic!("SIGILL terminal receipt disappeared after a second poll")
            }
        }

        let next_generation = owner.reserve_launch_generation().unwrap();
        assert!(next_generation > generation);
        let next_scope = FixtureScope::new(
            service_id,
            2,
            RecoveryLifecycleTransition::Initial,
            next_generation,
        );
        launch(&owner, &next_scope);
        let next_proof = issue(&owner, &next_scope);
        assert!(next_proof.is_live().unwrap());
        assert!(!process_identity.same_instance(&next_proof.process_identity()));

        let old_effects = AtomicUsize::new(0);
        assert_eq!(
            proof
                .with_live_authority_effect(|| old_effects.fetch_add(1, Ordering::SeqCst))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert_eq!(old_effects.load(Ordering::SeqCst), 0);
        let new_effects = AtomicUsize::new(0);
        assert_eq!(
            next_proof
                .with_live_authority_effect(|| new_effects.fetch_add(1, Ordering::SeqCst))
                .unwrap(),
            0
        );
        assert_eq!(new_effects.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn c3_slot_journal_owner_commits_launch_and_exact_retirement() {
        let service_id = "system.fixture.durable-owner";
        let directory = JournalDirectory::new();
        let journal_path = directory.journal_path();
        let owner =
            LinuxC3SystemServiceSlotOwner::create_with_journal(service_id, &journal_path).unwrap();
        let generation = owner.reserve_launch_generation().unwrap();
        let scope = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
        );
        let image = namespace_root_probe_elf();
        launch_image(&owner, &scope, &image).unwrap();

        let journal = owner_journal(&owner);
        match journal.readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.service_id(), service_id);
                assert_eq!(row.catalog_generation(), 1);
                assert_eq!(row.c3_launch_generation(), generation.get());
                assert_eq!(row.scope_binding_digest(), &scope.binding);
                assert_eq!(row.operation_id(), 1);
                assert_eq!(row.state(), C3SlotState::Live);
            }
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                panic!("C3 owner did not commit its exact launch row")
            }
        }

        owner
            .revoke_current(Duration::from_secs(2), Duration::from_millis(10))
            .unwrap();
        assert_eq!(owner.active_child_count().unwrap(), 0);
        match journal.readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.service_id(), service_id);
                assert_eq!(row.catalog_generation(), 1);
                assert_eq!(row.c3_launch_generation(), generation.get());
                assert_eq!(row.scope_binding_digest(), &scope.binding);
                assert_eq!(row.operation_id(), 2);
                assert_eq!(row.state(), C3SlotState::ClearedAndAbsent);
                assert_eq!(row.highest_c3_launch_generation(), generation.get());
                assert_eq!(row.highest_operation_id(), 2);
            }
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                panic!("C3 owner did not commit its exact retirement row")
            }
        }
        owner
            .shutdown(Duration::from_secs(2), Duration::from_millis(10))
            .unwrap();
    }

    #[cfg(not(nagi_release_build))]
    #[test]
    fn c3_slot_journal_dev_first_provision_creates_private_pair_and_first_launch() {
        let service_id = "system.fixture.dev-first-provision";
        let directory = JournalDirectory::new();
        let journal_path = directory.journal_path();
        let marker_path = directory.0.join("slot.journal.first-provision");
        let mut marker = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&marker_path)
            .unwrap();
        marker
            .write_all(b"NAGI-C3-SLOT-FIRST-PROVISION-V1\nsystem.fixture.dev-first-provision\n")
            .unwrap();
        marker.sync_all().unwrap();
        drop(marker);

        let owner =
            LinuxC3SystemServiceSlotOwner::new_with_journal(service_id, &journal_path).unwrap();
        assert!(
            !marker_path.exists(),
            "first-provision marker was not consumed"
        );
        let lock_path = directory.0.join("slot.journal.lock");
        for path in [&journal_path, &lock_path] {
            let metadata = std::fs::metadata(path).unwrap();
            assert_eq!(metadata.mode() & 0o777, 0o600);
            assert_eq!(metadata.nlink(), 1);
        }
        assert!(matches!(
            owner_journal(&owner).readback(),
            C3SlotJournalReadback::Empty
        ));

        let generation = owner.reserve_launch_generation().unwrap();
        assert_eq!(generation, NonZeroU64::new(1).unwrap());
        let scope = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
        );
        launch(&owner, &scope);
        match owner_journal(&owner).readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.service_id(), service_id);
                assert_eq!(row.catalog_generation(), 1);
                assert_eq!(row.c3_launch_generation(), 1);
                assert_eq!(row.scope_binding_digest(), &scope.binding);
                assert_eq!(row.state(), C3SlotState::Live);
            }
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                panic!("first launch did not commit a durable Live row")
            }
        }
        owner
            .revoke_current(Duration::from_secs(2), Duration::from_millis(10))
            .unwrap();
        owner
            .shutdown(Duration::from_secs(2), Duration::from_millis(10))
            .unwrap();
    }

    #[cfg(not(nagi_release_build))]
    #[test]
    fn c3_slot_journal_dev_first_provision_resumes_exact_genesis_after_interruption() {
        let service_id = "system.fixture.dev-first-provision-resume-genesis";
        let directory = JournalDirectory::new();
        let journal_path = directory.journal_path();
        let marker_path = directory.0.join("slot.journal.first-provision");
        let mut marker = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&marker_path)
            .unwrap();
        marker
            .write_all(
                b"NAGI-C3-SLOT-FIRST-PROVISION-V1\nsystem.fixture.dev-first-provision-resume-genesis\n",
            )
            .unwrap();
        marker.sync_all().unwrap();
        drop(marker);

        // Model a stop after the complete journal and lock are durable but
        // before the C3 owner consumes the one-time marker.
        let interrupted = C3SlotJournal::create(&journal_path, service_id).unwrap();
        assert!(marker_path.exists());
        assert!(matches!(
            interrupted.readback(),
            C3SlotJournalReadback::Empty
        ));
        drop(interrupted);

        let owner = LinuxC3SystemServiceSlotOwner::new_with_journal(service_id, &journal_path)
            .expect("the exact validated genesis pair completes first provisioning");
        assert!(!marker_path.exists(), "resumed marker was not consumed");
        assert!(matches!(
            owner_journal(&owner).readback(),
            C3SlotJournalReadback::Empty
        ));
        owner
            .shutdown(Duration::from_secs(2), Duration::from_millis(10))
            .unwrap();
        drop(owner);

        // Without the one-time marker, genesis alone is not accepted as a
        // completed slot. Keep that fail-closed boundary explicit: an
        // interruption around marker removal may require separate recovery.
        let journal_before = std::fs::read(&journal_path).unwrap();
        let lock_path = directory.0.join("slot.journal.lock");
        let lock_before = std::fs::read(&lock_path).unwrap();
        assert!(
            LinuxC3SystemServiceSlotOwner::new_with_journal(service_id, &journal_path).is_err()
        );
        assert_eq!(std::fs::read(&journal_path).unwrap(), journal_before);
        assert_eq!(std::fs::read(&lock_path).unwrap(), lock_before);
        assert!(!marker_path.exists());
    }

    #[cfg(not(nagi_release_build))]
    #[test]
    fn c3_slot_journal_dev_first_provision_keeps_non_genesis_state_unknown() {
        let service_id = "system.fixture.dev-first-provision-non-genesis";
        let directory = JournalDirectory::new();
        let journal_path = directory.journal_path();
        let lock_path = directory.0.join("slot.journal.lock");
        let marker_path = directory.0.join("slot.journal.first-provision");
        let marker_bytes =
            b"NAGI-C3-SLOT-FIRST-PROVISION-V1\nsystem.fixture.dev-first-provision-non-genesis\n"
                .to_vec();
        let mut marker = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&marker_path)
            .unwrap();
        marker.write_all(&marker_bytes).unwrap();
        marker.sync_all().unwrap();
        drop(marker);

        let journal = C3SlotJournal::create(&journal_path, service_id).unwrap();
        journal.reserve_next_generation().unwrap();
        assert!(matches!(journal.readback(), C3SlotJournalReadback::Unknown));
        drop(journal);
        let journal_before = std::fs::read(&journal_path).unwrap();
        let lock_before = std::fs::read(&lock_path).unwrap();

        assert!(
            LinuxC3SystemServiceSlotOwner::new_with_journal(service_id, &journal_path).is_err()
        );
        assert_eq!(std::fs::read(&marker_path).unwrap(), marker_bytes);
        assert_eq!(std::fs::read(&journal_path).unwrap(), journal_before);
        assert_eq!(std::fs::read(&lock_path).unwrap(), lock_before);
    }

    #[cfg(not(nagi_release_build))]
    #[test]
    fn c3_slot_journal_dev_first_provision_rejects_replay_and_partial_inputs() {
        let cases = ["mismatch", "journal-only", "lock-only", "journal-and-lock"];
        for (index, case) in cases.into_iter().enumerate() {
            let service_id = format!("system.fixture.dev-first-provision-{index}");
            let directory = JournalDirectory::new();
            let journal_path = directory.journal_path();
            let lock_path = directory.0.join("slot.journal.lock");
            let marker_path = directory.0.join("slot.journal.first-provision");
            if case == "journal-only" || case == "journal-and-lock" {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&journal_path)
                    .unwrap()
                    .write_all(b"existing-journal")
                    .unwrap();
            }
            if case == "lock-only" || case == "journal-and-lock" {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&lock_path)
                    .unwrap()
                    .write_all(b"existing-lock")
                    .unwrap();
            }
            let marker_service = if case == "mismatch" {
                "system.fixture.other"
            } else {
                service_id.as_str()
            };
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&marker_path)
                .unwrap()
                .write_all(
                    format!("NAGI-C3-SLOT-FIRST-PROVISION-V1\n{marker_service}\n").as_bytes(),
                )
                .unwrap();

            assert!(
                LinuxC3SystemServiceSlotOwner::new_with_journal(&service_id, &journal_path)
                    .is_err(),
                "accepted unsafe first-provision input case {case}"
            );
            assert!(
                marker_path.exists(),
                "rejected marker was consumed for {case}"
            );
            assert_eq!(
                std::fs::read(&journal_path).ok(),
                (case == "journal-only" || case == "journal-and-lock")
                    .then_some(b"existing-journal".to_vec()),
                "journal changed for {case}"
            );
            assert_eq!(
                std::fs::read(&lock_path).ok(),
                (case == "lock-only" || case == "journal-and-lock")
                    .then_some(b"existing-lock".to_vec()),
                "lock changed for {case}"
            );
        }
    }

    #[cfg(not(nagi_release_build))]
    #[test]
    fn c3_registered_slot_never_processes_first_provision_marker() {
        let service_id = "system.fixture.registered-slot-open-only";
        let directory = JournalDirectory::new();
        let journal_path = directory.journal_path();
        let marker_path = directory.0.join("slot.journal.first-provision");
        let marker_bytes =
            b"NAGI-C3-SLOT-FIRST-PROVISION-V1\nsystem.fixture.registered-slot-open-only\n";
        std::fs::write(&marker_path, marker_bytes).unwrap();
        std::fs::set_permissions(&marker_path, std::fs::Permissions::from_mode(0o600)).unwrap();

        // An active slot is rejected before the marker path can be opened.
        let owner = LinuxC3SystemServiceSlotOwner::new(service_id).unwrap();
        let active_error =
            LinuxC3SystemServiceSlotOwner::new_with_journal(service_id, &journal_path)
                .err()
                .expect("an active slot must reject a second owner");
        assert_eq!(active_error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(std::fs::read(&marker_path).unwrap(), marker_bytes);
        assert!(!journal_path.exists());
        assert!(!directory.0.join("slot.journal.lock").exists());
        drop(owner);

        // A previously registered but currently unclaimed slot is open-only:
        // missing journal state remains Unknown instead of consuming marker.
        let reopened_error =
            LinuxC3SystemServiceSlotOwner::new_with_journal(service_id, &journal_path)
                .err()
                .expect("an existing slot must remain open-only");
        assert_eq!(reopened_error.kind(), std::io::ErrorKind::Other);
        assert_eq!(std::fs::read(&marker_path).unwrap(), marker_bytes);
        assert!(!journal_path.exists());
        assert!(!directory.0.join("slot.journal.lock").exists());
    }

    #[cfg(not(nagi_release_build))]
    #[test]
    fn c3_first_provision_fifo_marker_rejects_without_blocking_or_writing() {
        const SERVICE_ENV: &str = "NAGI_C3_FIRST_PROVISION_FIFO_SERVICE";
        const JOURNAL_ENV: &str = "NAGI_C3_FIRST_PROVISION_FIFO_JOURNAL";
        const MARKER_ENV: &str = "NAGI_C3_FIRST_PROVISION_FIFO_MARKER";
        const RESULT_ENV: &str = "NAGI_C3_FIRST_PROVISION_FIFO_RESULT";

        if let (Ok(service_id), Ok(journal_path), Ok(marker_path), Ok(result_path)) = (
            std::env::var(SERVICE_ENV),
            std::env::var(JOURNAL_ENV),
            std::env::var(MARKER_ENV),
            std::env::var(RESULT_ENV),
        ) {
            assert!(std::fs::symlink_metadata(marker_path)
                .unwrap()
                .file_type()
                .is_fifo());
            let result =
                match LinuxC3SystemServiceSlotOwner::new_with_journal(&service_id, &journal_path) {
                    Ok(_) => "accepted".to_owned(),
                    Err(error) => format!("rejected:{:?}", error.kind()),
                };
            std::fs::write(result_path, result).unwrap();
            return;
        }

        let service_id = "system.fixture.first-provision-fifo";
        let directory = JournalDirectory::new();
        let journal_path = directory.journal_path();
        let marker_path = directory.0.join("slot.journal.first-provision");
        let result_path = directory.0.join("child.result");
        let marker_c_path = CString::new(marker_path.as_os_str().as_bytes()).unwrap();
        let fifo_created = unsafe { mkfifo(marker_c_path.as_ptr(), 0o600) };
        assert_eq!(
            fifo_created,
            0,
            "mkfifo failed: {}",
            std::io::Error::last_os_error()
        );

        let test_executable = std::env::args_os().next().expect("test binary argv[0]");
        let mut child = Command::new(test_executable)
            .arg(
                "linux_init::linux_init_c3_system_service_owner::tests::c3_first_provision_fifo_marker_rejects_without_blocking_or_writing",
            )
            .arg("--exact")
            .env(SERVICE_ENV, service_id)
            .env(JOURNAL_ENV, &journal_path)
            .env(MARKER_ENV, &marker_path)
            .env(RESULT_ENV, &result_path)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let reap_deadline = Instant::now() + Duration::from_secs(1);
                loop {
                    if let Some(status) = child.try_wait().unwrap() {
                        panic!("FIFO marker open did not return promptly; helper was terminated ({status})");
                    }
                    assert!(
                        Instant::now() < reap_deadline,
                        "FIFO marker helper was not reaped after termination"
                    );
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        assert!(status.success(), "FIFO marker helper failed: {status}");
        assert_eq!(
            std::fs::read_to_string(result_path).unwrap(),
            "rejected:PermissionDenied"
        );
        assert!(std::fs::symlink_metadata(&marker_path)
            .unwrap()
            .file_type()
            .is_fifo());
        assert!(!journal_path.exists());
        assert!(!directory.0.join("slot.journal.lock").exists());
    }

    #[test]
    fn c3_slot_journal_reconciliation_returns_exact_cleared_receipt() {
        let service_id = "system.fixture.reconciliation-receipt";
        let directory = JournalDirectory::new();
        let journal_path = directory.journal_path();
        let owner =
            LinuxC3SystemServiceSlotOwner::create_with_journal(service_id, &journal_path).unwrap();
        let generation = owner.reserve_launch_generation().unwrap();
        let scope = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
        );
        let proof = launch_and_authenticate(&owner, &scope);
        assert!(proof.is_live().unwrap());
        assert_eq!(owner.active_child_count().unwrap(), 1);

        let live_effect_id = NonZeroU64::new(72).unwrap();
        let live_effect_claim = C1RecoveryOperationCorrelation::new(live_effect_id, scope.binding);
        let live_effect_count = AtomicUsize::new(0);
        let (live_effect_result, live_effect_receipt) = owner
            .with_durable_live_effect_gate(&scope, &proof, live_effect_claim, || {
                live_effect_count.fetch_add(1, Ordering::SeqCst);
                71
            })
            .unwrap();
        assert_eq!(live_effect_result, 71);
        assert_eq!(live_effect_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            C3DurableRecoverySlotReceipt::c1_operation_id(&live_effect_receipt),
            Some(live_effect_id)
        );
        assert_ne!(live_effect_receipt.operation_id(), live_effect_id);
        assert_eq!(
            C3DurableRecoverySlotReceipt::state(&live_effect_receipt),
            C3RecoverySlotState::Live
        );
        match owner_journal(&owner).readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.state(), C3SlotState::Live);
                assert_eq!(row.service_id(), service_id);
                assert_eq!(row.catalog_generation(), 1);
                assert_eq!(row.c3_launch_generation(), generation.get());
                assert_eq!(row.scope_binding_digest(), &scope.binding);
                assert_eq!(row.c1_operation_id(), Some(live_effect_id));
            }
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                panic!("C3 live-effect correlation was not committed and read back")
            }
        }

        let continuation_count = AtomicUsize::new(0);
        let (continuation_result, continuation_receipt) = owner
            .with_durable_live_continuation_gate(&scope, &proof, live_effect_claim, || {
                continuation_count.fetch_add(1, Ordering::SeqCst);
                Ok(72)
            })
            .unwrap();
        assert_eq!(continuation_result, 72);
        assert_eq!(continuation_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            C3DurableRecoverySlotReceipt::c1_operation_id(&continuation_receipt),
            Some(live_effect_id)
        );
        assert_ne!(
            continuation_receipt.operation_id(),
            live_effect_receipt.operation_id()
        );
        assert_eq!(
            C3DurableRecoverySlotReceipt::state(&continuation_receipt),
            C3RecoverySlotState::Live
        );
        match owner_journal(&owner).readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.state(), C3SlotState::Live);
                assert_eq!(row.c1_operation_id(), Some(live_effect_id));
                assert_eq!(
                    row.operation_id(),
                    continuation_receipt.operation_id().get()
                );
                assert_eq!(row.scope_binding_digest(), &scope.binding);
            }
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                panic!("C3 continuation did not commit and read back its exact Live row")
            }
        }

        confirm_isolated_endpoint(&proof);
        owner
            .finish_admitted(
                &scope,
                Some(&proof),
                Duration::from_secs(2),
                Duration::from_millis(10),
            )
            .unwrap();
        assert!(!proof.is_live().unwrap());
        assert_eq!(owner.active_child_count().unwrap(), 0);

        let c1_operation_id = NonZeroU64::new(73).unwrap();
        let correlation = C1RecoveryOperationCorrelation::new(c1_operation_id, scope.binding);
        let callback_count = AtomicUsize::new(0);
        let (result, receipt) = owner
            .with_durable_reconciliation_gate(&scope, correlation, || {
                callback_count.fetch_add(1, Ordering::SeqCst);
                74
            })
            .unwrap();
        assert_eq!(result, 74);
        assert_eq!(callback_count.load(Ordering::SeqCst), 1);
        assert_eq!(
            C3DurableRecoverySlotReceipt::service_id(&receipt),
            service_id
        );
        assert_eq!(
            C3DurableRecoverySlotReceipt::catalog_generation(&receipt),
            1
        );
        assert_eq!(
            C3DurableRecoverySlotReceipt::c1_operation_id(&receipt),
            Some(c1_operation_id)
        );
        assert_ne!(receipt.operation_id(), c1_operation_id);
        assert_eq!(
            C3DurableRecoverySlotReceipt::c3_launch_generation(&receipt),
            generation.get()
        );
        assert_eq!(
            C3DurableRecoverySlotReceipt::scope_binding_digest(&receipt),
            &scope.binding
        );
        assert_eq!(
            C3DurableRecoverySlotReceipt::state(&receipt),
            C3RecoverySlotState::ClearedAndAbsent
        );
        assert!(!proof.is_live().unwrap());
        assert_eq!(owner.active_child_count().unwrap(), 0);
        match owner_journal(&owner).readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.state(), C3SlotState::ClearedAndAbsent);
                assert_eq!(row.service_id(), service_id);
                assert_eq!(row.c3_launch_generation(), generation.get());
                assert_eq!(row.scope_binding_digest(), &scope.binding);
                assert_eq!(row.c1_operation_id(), Some(c1_operation_id));
            }
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                panic!("C3 reconciliation receipt lost its durable cleared row")
            }
        }
        drop(proof);
        owner
            .shutdown(Duration::from_secs(2), Duration::from_millis(10))
            .unwrap();
        eprintln!("C3_DURABLE_RECONCILIATION_RECEIPT_PASS: exact cleared slot returned");
    }

    #[test]
    fn c3_retirement_deadline_boundary_is_strict_at_the_same_clock_time() {
        let deadline = Instant::now() + Duration::from_secs(1);
        let immediately_before = deadline - Duration::from_nanos(1);

        assert_eq!(
            super::retirement_budget_remaining_at(deadline, immediately_before).unwrap(),
            Duration::from_nanos(1)
        );
        assert_eq!(
            super::retirement_budget_remaining_at(deadline, deadline)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn c3_public_reapers_reject_success_at_the_final_deadline_boundary() {
        let shutdown_owner =
            LinuxC3SystemServiceSlotOwner::new("system.fixture.shutdown-final-deadline-boundary")
                .unwrap();
        let shutdown_deadline = Instant::now() + Duration::from_secs(60);
        let shutdown_clock_reads = AtomicUsize::new(0);
        let shutdown_now = || {
            if shutdown_clock_reads.fetch_add(1, Ordering::SeqCst) == 0 {
                shutdown_deadline - Duration::from_nanos(1)
            } else {
                shutdown_deadline
            }
        };
        assert_eq!(
            shutdown_owner
                .shutdown_until(shutdown_deadline, Duration::from_millis(1), &shutdown_now,)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::TimedOut
        );
        assert_eq!(shutdown_clock_reads.load(Ordering::SeqCst), 2);

        let reap_owner =
            LinuxC3SystemServiceSlotOwner::new("system.fixture.quiescent-final-deadline-boundary")
                .unwrap();
        let reap_deadline = Instant::now() + Duration::from_secs(60);
        assert_eq!(
            reap_owner
                .reap_until_quiescent_with_clock(reap_deadline, Duration::from_millis(1), &|| {
                    reap_deadline
                },)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::TimedOut
        );

        let unowned_service = "system.fixture.unowned-final-deadline-boundary";
        drop(LinuxC3SystemServiceSlotOwner::new(unowned_service).unwrap());
        let unowned_deadline = Instant::now() + Duration::from_secs(60);
        let unowned_clock_reads = AtomicUsize::new(0);
        let unowned_now = || {
            if unowned_clock_reads.fetch_add(1, Ordering::SeqCst) == 0 {
                unowned_deadline - Duration::from_nanos(1)
            } else {
                unowned_deadline
            }
        };
        assert_eq!(
            LinuxC3SystemServiceSlotOwner::reap_unowned_slot_until(
                unowned_service,
                unowned_deadline,
                Duration::from_millis(1),
                &unowned_now,
            )
            .unwrap_err()
            .kind(),
            std::io::ErrorKind::TimedOut
        );
        assert_eq!(unowned_clock_reads.load(Ordering::SeqCst), 2);

        let missing_service = "system.fixture.unregistered-no-op-before-deadline";
        let missing_deadline = Instant::now() + Duration::from_secs(60);
        let before_deadline = missing_deadline - Duration::from_nanos(1);
        LinuxC3SystemServiceSlotOwner::reap_unowned_slot_until(
            missing_service,
            missing_deadline,
            Duration::from_millis(1),
            &|| before_deadline,
        )
        .unwrap();

        let expired_missing_service = "system.fixture.unregistered-no-op-at-deadline";
        let expired_missing_deadline = Instant::now() + Duration::from_secs(60);
        assert_eq!(
            LinuxC3SystemServiceSlotOwner::reap_unowned_slot_until(
                expired_missing_service,
                expired_missing_deadline,
                Duration::from_millis(1),
                &|| expired_missing_deadline,
            )
            .unwrap_err()
            .kind(),
            std::io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn c3_slot_journal_natural_exit_honors_reaper_deadline() {
        let service_id = "system.fixture.natural-exit-lock-budget";
        let directory = JournalDirectory::new();
        let journal_path = directory.journal_path();
        let owner =
            LinuxC3SystemServiceSlotOwner::create_with_journal(service_id, &journal_path).unwrap();
        let generation = owner.reserve_launch_generation().unwrap();
        let scope = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
        );
        launch(&owner, &scope);
        let proof = issue(&owner, &scope);
        let lock_holder = JournalLockHolder::start(&directory, service_id, &journal_path);

        allow_probe_to_exit(&proof);
        let exit_deadline = Instant::now() + Duration::from_secs(2);
        while proof.is_live().unwrap() {
            assert!(
                Instant::now() < exit_deadline,
                "service probe did not exit naturally"
            );
            std::thread::sleep(Duration::from_millis(1));
        }

        let started = Instant::now();
        let result =
            owner.reap_until_quiescent(Duration::from_millis(20), Duration::from_millis(2));
        let elapsed = started.elapsed();
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Other);
        assert!(elapsed < Duration::from_millis(500));
        assert!(!proof.is_live().unwrap());
        let state = owner.state.lock().unwrap();
        assert_eq!(state.current, None);
        assert_eq!(state.lifecycle.active_len(), 0);
        drop(state);
        eprintln!("C3_P2_NATURAL_EXIT_REAPER_BUDGET_BOUNDED: elapsed={elapsed:?}");

        drop(lock_holder);
        match owner_journal(&owner).readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.service_id(), service_id);
                assert_eq!(row.c3_launch_generation(), generation.get());
                assert_eq!(row.scope_binding_digest(), &scope.binding);
                assert_eq!(row.state(), C3SlotState::Live);
            }
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                panic!("natural exit changed the durable slot while its lock was held")
            }
        }
        owner
            .revoke_current(Duration::from_secs(2), Duration::from_millis(10))
            .unwrap();
    }

    #[test]
    fn c3_slot_journal_reopen_rejects_missing_empty_and_fresh_replacement() {
        let service_id = "system.fixture.journal-path-reset";
        let directory = JournalDirectory::new();
        let journal_path = directory.journal_path();
        let owner =
            LinuxC3SystemServiceSlotOwner::create_with_journal(service_id, &journal_path).unwrap();
        let generation1 = owner.reserve_launch_generation().unwrap();
        assert_eq!(generation1, NonZeroU64::new(1).unwrap());
        let scope = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation1,
        );
        launch(&owner, &scope);
        owner
            .revoke_current(Duration::from_secs(2), Duration::from_millis(10))
            .unwrap();
        drop(owner);

        let backup_path = directory.0.join("slot.journal.backup");
        std::fs::rename(&journal_path, &backup_path).unwrap();
        let missing_path_result =
            observe_owner_restart(&directory, service_id, &journal_path, "missing.result");
        assert_eq!(missing_path_result, "rejected:Other");
        if journal_path.exists() {
            std::fs::remove_file(&journal_path).unwrap();
        }
        std::fs::rename(&backup_path, &journal_path).unwrap();

        let lock_path = directory.0.join("slot.journal.lock");
        let lock_backup_path = directory.0.join("slot.journal.lock.backup");
        std::fs::rename(&lock_path, &lock_backup_path).unwrap();
        let missing_lock_result =
            observe_owner_restart(&directory, service_id, &journal_path, "missing-lock.result");
        assert_eq!(missing_lock_result, "rejected:Other");
        assert!(!lock_path.exists(), "reopen recreated a missing lock file");
        std::fs::rename(&lock_backup_path, &lock_path).unwrap();

        let intact_path_result =
            observe_owner_restart(&directory, service_id, &journal_path, "intact.result");
        assert_eq!(intact_path_result, "accepted:2");

        let other_directory = JournalDirectory::new();
        let other_path = other_directory.journal_path();
        let other_lock_path = other_directory.0.join("slot.journal.lock");
        for path in [&other_path, &other_lock_path] {
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .unwrap();
        }
        let empty_path_result =
            observe_owner_restart(&directory, service_id, &other_path, "empty.result");
        assert_eq!(empty_path_result, "rejected:Other");

        let fresh_directory = JournalDirectory::new();
        let fresh_path = fresh_directory.journal_path();
        let fresh_journal = C3SlotJournal::create(&fresh_path, service_id).unwrap();
        drop(fresh_journal);
        let genesis_only_result =
            observe_owner_restart(&directory, service_id, &fresh_path, "genesis-only.result");
        assert_eq!(genesis_only_result, "rejected:Other");
        eprintln!(
            "C3_P2_JOURNAL_REOPEN_FAILS_CLOSED: missing={missing_path_result} missing_lock={missing_lock_result} empty={empty_path_result} genesis_only={genesis_only_result} intact={intact_path_result}"
        );
    }

    #[test]
    fn c3_slot_journal_child_reopen_observation() {
        const SERVICE_ENV: &str = "NAGI_C3_OWNER_REOPEN_SERVICE";
        const JOURNAL_ENV: &str = "NAGI_C3_OWNER_REOPEN_JOURNAL";
        const RESULT_ENV: &str = "NAGI_C3_OWNER_REOPEN_RESULT";
        let (Ok(service_id), Ok(journal_path), Ok(result_path)) = (
            std::env::var(SERVICE_ENV),
            std::env::var(JOURNAL_ENV),
            std::env::var(RESULT_ENV),
        ) else {
            return;
        };
        let result =
            match LinuxC3SystemServiceSlotOwner::new_with_journal(&service_id, &journal_path) {
                Ok(owner) => match owner.reserve_launch_generation() {
                    Ok(generation) => format!("accepted:{}", generation.get()),
                    Err(error) => format!("reserved-error:{:?}", error.kind()),
                },
                Err(error) => format!("rejected:{:?}", error.kind()),
            };
        std::fs::write(result_path, result).unwrap();
    }

    fn owner_journal(owner: &LinuxC3SystemServiceSlotOwner) -> Arc<super::C3SlotJournal> {
        let state = owner.state.lock().unwrap();
        Arc::clone(state.durable_journal.as_ref().unwrap())
    }

    #[test]
    fn wrong_endpoint_is_rejected_and_revokes_the_exact_current_child() {
        let service_id = "system.fixture.wrong-endpoint";
        let owner = LinuxC3SystemServiceSlotOwner::new(service_id).unwrap();
        let generation = owner.reserve_launch_generation().unwrap();
        let scope = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
        );
        launch(&owner, &scope);
        let (wrong_endpoint, _peer) = UnixStream::pair().unwrap();
        let error = match owner.issue_current_service_endpoint(
            &scope,
            wrong_endpoint.as_fd().try_clone_to_owned().unwrap(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("wrong endpoint received a C3 proof"),
        };
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        owner
            .reap_until_quiescent(Duration::from_secs(2), Duration::from_millis(10))
            .unwrap();
        assert_eq!(owner.active_child_count().unwrap(), 0);
    }

    #[test]
    fn callback_panic_stays_unknown_then_revokes_and_reaps_the_exact_child() {
        let service_id = "system.fixture.panic-cleanup";
        let owner = LinuxC3SystemServiceSlotOwner::new(service_id).unwrap();
        let generation = owner.reserve_launch_generation().unwrap();
        let scope = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
        );
        launch(&owner, &scope);
        let proof = issue(&owner, &scope);

        let correlation =
            C1RecoveryOperationCorrelation::new(NonZeroU64::new(1).unwrap(), scope.binding);
        assert_eq!(
            owner.with_current_reconciliation_gate(&scope, &proof, correlation, || {
                panic!("fixture recovery callback panic")
            }),
            Err(RecoveryClientError::OutcomeUnknown)
        );
        owner
            .revoke_current(Duration::from_secs(2), Duration::from_millis(10))
            .unwrap();
        assert_eq!(owner.active_child_count().unwrap(), 0);
        assert!(!proof.is_live().unwrap());
        assert!(proof.with_live_authority_effect(|| ()).is_err());
    }

    #[test]
    fn dropped_owner_keeps_unique_slot_until_exact_child_reaps() {
        let service_id = "system.fixture.owner-drop";
        let owner = LinuxC3SystemServiceSlotOwner::new(service_id).unwrap();
        assert!(LinuxC3SystemServiceSlotOwner::new(service_id).is_err());

        let generation = owner.reserve_launch_generation().unwrap();
        let scope = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
        );
        launch(&owner, &scope);
        let proof = issue(&owner, &scope);
        drop(owner);

        assert!(!proof.is_live().unwrap());
        assert!(proof.with_live_authority_effect(|| ()).is_err());
        LinuxC3SystemServiceSlotOwner::reap_unowned_slot(
            service_id,
            Duration::from_secs(2),
            Duration::from_millis(10),
        )
        .unwrap();
        let reopened = LinuxC3SystemServiceSlotOwner::new(service_id).unwrap();
        assert_eq!(reopened.active_child_count().unwrap(), 0);
        let generation = reopened.reserve_launch_generation().unwrap();
        assert_eq!(generation.get(), 2);
        let catalog_rollback = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
        );
        let error = launch_image(&reopened, &catalog_rollback, &namespace_root_probe_elf())
            .expect_err("owner recreation must retain the catalog-generation high-water mark");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn owner_state_poison_reaps_exact_child_and_reacquires_with_fresh_generation() {
        let service_id = "system.fixture.owner-state-poison";
        let owner = LinuxC3SystemServiceSlotOwner::new(service_id).unwrap();
        let generation = owner.reserve_launch_generation().unwrap();
        let scope = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
        );
        launch(&owner, &scope);
        let proof = issue(&owner, &scope);

        poison_owner_state_after_current_revoke(&owner);
        drop(owner);

        assert!(!proof.is_live().unwrap());
        assert!(proof.with_live_authority_effect(|| ()).is_err());
        let retained_state = retained_owner_state(service_id);
        assert_eq!(retained_active_child_count(&retained_state), 1);

        LinuxC3SystemServiceSlotOwner::reap_unowned_slot(
            service_id,
            Duration::from_secs(2),
            Duration::from_millis(10),
        )
        .expect("poisoned state must still permit exact pidfd cleanup");
        assert_eq!(retained_active_child_count(&retained_state), 0);
        let reopened = LinuxC3SystemServiceSlotOwner::new(service_id)
            .expect("only a fully retired and validated poisoned state can be reused");
        let next_generation = reopened.reserve_launch_generation().unwrap();
        assert_eq!(next_generation.get(), 2);
        let catalog_rollback = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            next_generation,
        );
        let error = launch_image(&reopened, &catalog_rollback, &namespace_root_probe_elf())
            .expect_err("poison recovery must preserve the catalog-generation high-water mark");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(reopened.active_child_count().unwrap(), 0);
        let next_scope = FixtureScope::new(
            service_id,
            2,
            RecoveryLifecycleTransition::Initial,
            next_generation,
        );
        launch(&reopened, &next_scope);
        let next_proof = issue(&reopened, &next_scope);
        assert!(!proof.is_live().unwrap());
        assert!(proof.with_live_authority_effect(|| ()).is_err());
        assert!(next_proof.is_live().unwrap());
        reopened
            .shutdown(Duration::from_secs(2), Duration::from_millis(10))
            .unwrap();
        assert!(!next_proof.is_live().unwrap());
    }

    #[test]
    fn owner_state_poison_shutdown_retries_exact_cleanup_without_clearing_poison() {
        let service_id = "system.fixture.owner-state-poison-shutdown";
        let owner = LinuxC3SystemServiceSlotOwner::new(service_id).unwrap();
        let generation = owner.reserve_launch_generation().unwrap();
        let scope = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
        );
        launch(&owner, &scope);
        let proof = issue(&owner, &scope);
        poison_owner_state_after_current_revoke(&owner);

        owner
            .shutdown(Duration::from_secs(2), Duration::from_millis(10))
            .expect("shutdown must retry cleanup through a poisoned owner mutex");
        assert!(!proof.is_live().unwrap());
        assert!(proof.with_live_authority_effect(|| ()).is_err());
        assert!(owner.reserve_launch_generation().is_err());
    }

    #[test]
    fn catalog_generation_high_water_survives_revoke_and_natural_exit() {
        for (service_id, natural_exit) in [
            ("system.fixture.catalog-after-revoke", false),
            ("system.fixture.catalog-after-exit", true),
        ] {
            let owner = LinuxC3SystemServiceSlotOwner::new(service_id).unwrap();
            let generation = owner.reserve_launch_generation().unwrap();
            let scope = FixtureScope::new(
                service_id,
                7,
                RecoveryLifecycleTransition::Initial,
                generation,
            );
            launch(&owner, &scope);
            let proof = issue(&owner, &scope);
            if natural_exit {
                allow_probe_to_exit(&proof);
                let deadline = Instant::now() + Duration::from_secs(2);
                loop {
                    if owner.poll().unwrap().is_some() {
                        break;
                    }
                    assert!(Instant::now() < deadline, "probe child did not exit");
                    std::thread::sleep(Duration::from_millis(10));
                }
            } else {
                owner
                    .revoke_current(Duration::from_secs(2), Duration::from_millis(10))
                    .unwrap();
            }
            assert!(!proof.is_live().unwrap());

            let next_generation = owner.reserve_launch_generation().unwrap();
            let rolled_back = FixtureScope::new(
                service_id,
                7,
                RecoveryLifecycleTransition::Initial,
                next_generation,
            );
            let error = launch_image(&owner, &rolled_back, &namespace_root_probe_elf())
                .expect_err("catalog generation reuse after current=None must be rejected");
            assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
            assert_eq!(owner.active_child_count().unwrap(), 0);
        }
    }

    #[test]
    fn explicit_shutdown_retires_current_child_before_return() {
        let service_id = "system.fixture.explicit-shutdown";
        let owner = LinuxC3SystemServiceSlotOwner::new(service_id).unwrap();
        let generation = owner.reserve_launch_generation().unwrap();
        let scope = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
        );
        launch(&owner, &scope);
        let proof = issue(&owner, &scope);

        owner
            .shutdown(Duration::from_secs(2), Duration::from_millis(10))
            .unwrap();
        assert_eq!(owner.active_child_count().unwrap(), 0);
        assert!(!proof.is_live().unwrap());
    }

    #[test]
    fn executable_bytes_must_match_the_admitted_digest_before_spawn() {
        let service_id = "system.fixture.executable-digest";
        let owner = LinuxC3SystemServiceSlotOwner::new(service_id).unwrap();
        let generation = owner.reserve_launch_generation().unwrap();
        let scope = FixtureScope::new(
            service_id,
            1,
            RecoveryLifecycleTransition::Initial,
            generation,
        );
        let mut wrong_image = namespace_root_probe_elf();
        wrong_image[224] ^= 1;

        let error = launch_image(&owner, &scope, &wrong_image)
            .expect_err("different executable bytes must not enter C3 launch");
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(owner.active_child_count().unwrap(), 0);
    }
}
