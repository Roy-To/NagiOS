//! Maintained PID-1 owner for one A-admitted C3 system-service slot.
//!
//! This owns real clone3 children, their pidfds, endpoint proofs, replacement,
//! revocation, and reaping. The launch entry is unsafe because the current A
//! crate boundary exposes only a consistency view; the caller must supply the
//! exact private A admission and executable bytes already accepted by the
//! trusted catalog/package owner. No safe production callsite exists yet.

use super::linux_init_c3_slot_journal::{
    C3DurableSlotReceipt, C3SlotJournal, C3SlotJournalError, C3SlotJournalReadback,
    C3SlotReservation, C3SlotState,
};
use super::linux_x86_64_process::LinuxProcessOs;
use super::{
    BackendError, ExecutablePayload, GateCommit, LinuxExecutionIdentity, Prepared,
    ProcessEndpointPolicy, ProcessHandle, ProcessLaunch, ProcessLifecycle, ProcessOs,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LaunchToken(NonZeroU64);

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
    scope: RecoveryScopeSnapshot,
    scope_data_address: usize,
    launch_record: SystemServiceLaunchRecord,
    state: LaunchState,
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
    /// journal. PID-1 startup fails closed unless the prior slot is durably
    /// recorded as absent; a live or unknown row is never reconstructed as an
    /// in-memory child.
    pub(super) fn new_with_journal(
        service_id: &str,
        journal_path: impl AsRef<Path>,
    ) -> io::Result<Self> {
        let journal =
            Arc::new(C3SlotJournal::open(journal_path, service_id).map_err(c3_journal_io_error)?);
        Self::from_open_journal(service_id, journal, false)
    }

    #[cfg(test)]
    fn create_with_journal(service_id: &str, journal_path: impl AsRef<Path>) -> io::Result<Self> {
        let journal =
            Arc::new(C3SlotJournal::create(journal_path, service_id).map_err(c3_journal_io_error)?);
        Self::from_open_journal(service_id, journal, true)
    }

    fn from_open_journal(
        service_id: &str,
        journal: Arc<C3SlotJournal>,
        allow_genesis_only: bool,
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

        let mut registry = slot_owner_registry_lock()?;
        if registry.iter().any(|entry| entry.service_id == service_id) {
            return Err(permission_denied(
                "a C3 owner already holds or retains this system-service slot",
            ));
        }
        let mut state = empty_owner_state();
        state.highest_catalog_generation = NonZeroU64::new(highest_catalog_generation);
        state.highest_generation = highest_c3_launch_generation;
        state.highest_operation_id = highest_operation_id;
        state.durable_journal = Some(journal);
        let state = Arc::new(Mutex::new(state));
        registry.push(SlotOwnerRegistration {
            service_id: service_id.to_owned(),
            state: Arc::clone(&state),
            claimed: true,
            maintenance: false,
        });
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
        let state = {
            let mut registry = slot_owner_registry_lock()?;
            let Some(entry) = registry
                .iter_mut()
                .find(|entry| entry.service_id == service_id)
            else {
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
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "PID-1 still owns the system-service child after its retirement budget",
                ));
            }
            let attempt = retire_owner_state_once(&mut guard, remaining, poll_interval)
                .and_then(|()| validate_quiescent_retired_state(&guard));
            match attempt {
                Ok(()) => break Ok(()),
                Err(error) => {
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
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
        result
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
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "PID-1 still owns the system-service child after its retirement budget",
                ));
            }
            let attempt = {
                let mut state = self.lock_state_for_cleanup();
                retire_owner_state_once(&mut state, remaining, poll_interval)
                    .and_then(|()| validate_quiescent_retired_state(&state))
            };
            match attempt {
                Ok(()) => return Ok(()),
                Err(error) => {
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
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
                    io::Error::new(
                        io::ErrorKind::Other,
                        "C3 system-service launch generation is exhausted",
                    )
                })?;
            (next, None)
        };
        state.highest_generation = next.get();
        state.reserved_generation = Some(next);
        state.reserved_operation = reservation;
        if let Some(reservation) = reservation {
            state.highest_operation_id = state
                .highest_operation_id
                .max(reservation.operation_id().get());
        }
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
        required: bool,
        release_wait_budget: Duration,
        cancel_check_interval: Duration,
        retirement_wait_budget: Duration,
        reaper_poll_interval: Duration,
        mut cancelled: impl FnMut() -> bool,
    ) -> io::Result<()> {
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
        let reserved_operation = state.reserved_operation;
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
            validate_transition(&scope_snapshot, previous, state.highest_catalog_generation)?;
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
        let run_launch = || -> io::Result<()> {
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

            let spawn_result = {
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
                            &mut **launches,
                            &mut **current,
                            os,
                            prepared,
                            process_id,
                            token,
                            &self.service_id,
                            scope_snapshot.clone(),
                            scope_data_address,
                            previous_token,
                            &mut **highest_catalog_generation,
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
                        revoke_registered_launch(
                            &mut **launches,
                            &mut **current,
                            process_id,
                            *registered,
                        )
                    },
                )
            };

            if let Some(previous_token) = previous_token {
                let previous_process_id = previous_process_id.expect("previous token has a PID");
                if spawn_result.is_err() && !replacement_committed {
                    return Err(backend_error(spawn_result.expect_err("checked error")));
                }
                if let Err(error) = terminate_launch_bounded(
                    &mut state,
                    previous_process_id,
                    previous_token,
                    retirement_wait_budget,
                    reaper_poll_interval,
                ) {
                    // The previous proof was already invalidated by replacement.
                    // Retire the new slot too; the journal remains pending if
                    // exact process cleanup cannot finish.
                    if let Ok(Some((new_pid, new_token))) = revoke_current_record(&mut state) {
                        let _ = terminate_launch_bounded(
                            &mut state,
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
                if let Ok(Some((process_id, token))) = revoke_current_record(&mut state) {
                    let _ = terminate_launch_bounded(
                        &mut state,
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
                || {
                    effect_ran = true;
                    match run_launch() {
                        Ok(()) => Ok(()),
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
                        Err(io::Error::new(
                            io::ErrorKind::Other,
                            format!(
                                "C3 slot journal outcome is unknown after effect failure: {effect_error}"
                            ),
                        ))
                    } else {
                        Err(c3_journal_io_error(error))
                    }
                }
            }
        } else {
            run_launch()
        }
    }

    /// Launch the executable admitted by the exact A scope, take the child’s
    /// inherited endpoint, and authenticate that endpoint against the live
    /// child record before returning. The C3 owner retains the child and its
    /// pidfd after the proof is issued; the caller must keep the same scope
    /// object alive for later effects and reconciliation.
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
        required: bool,
        release_wait_budget: Duration,
        cancel_check_interval: Duration,
        retirement_wait_budget: Duration,
        reaper_poll_interval: Duration,
        cancelled: impl FnMut() -> bool,
    ) -> io::Result<SystemServiceEndpointOwnerProof> {
        unsafe {
            self.launch_admitted(
                scope,
                executable,
                arguments,
                required,
                release_wait_budget,
                cancel_check_interval,
                retirement_wait_budget,
                reaper_poll_interval,
                cancelled,
            )?;
        }
        let endpoint = self.take_current_service_endpoint(scope)?;
        self.issue_current_service_endpoint(scope, endpoint)
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
                if let Err(reconciliation_error) = poll_and_record_absence(&mut state) {
                    return Err(reconciliation_error);
                }
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
                if let Err(reconciliation_error) = poll_and_record_absence(&mut state) {
                    return Err(reconciliation_error);
                }
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
                    if let Err(retirement_error) = retire_owner_state_once(
                        &mut state,
                        DEFAULT_RETIREMENT_WAIT_BUDGET,
                        DEFAULT_REAPER_POLL_INTERVAL,
                    ) {
                        return Err(retirement_error);
                    }
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
        loop {
            let mut state = self.lock_state_for_cleanup();
            let had_current = state.current.is_some();
            let poll_result = poll_and_record_absence_until(&mut state, Some(deadline));
            if had_current && state.current.is_none() {
                poll_result?;
            }
            if state.lifecycle.active_len() == 0 {
                return Ok(());
            }
            drop(state);
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
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
    let Some(journal) = state.durable_journal.clone() else {
        return retire_owner_state_effect(state, wait_budget, poll_interval);
    };
    let deadline = Instant::now()
        .checked_add(wait_budget)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "retirement budget overflow"))?;

    match journal.readback_with_timeout(retirement_budget_remaining(deadline)?) {
        C3SlotJournalReadback::Empty => {
            retire_owner_state_effect(state, retirement_budget_remaining(deadline)?, poll_interval)
        }
        C3SlotJournalReadback::Committed(row) if row.state() == C3SlotState::ClearedAndAbsent => {
            retire_owner_state_effect(state, retirement_budget_remaining(deadline)?, poll_interval)
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
                retire_owner_state_effect(
                    state,
                    retirement_budget_remaining(deadline)?,
                    poll_interval,
                )?;
                return Err(c3_journal_io_error(C3SlotJournalError::Unknown));
            }
            let transition_result = journal.transition_next_with_timeout(
                catalog_generation,
                c3_launch_generation,
                scope_binding_digest,
                C3SlotState::ClearedAndAbsent,
                retirement_budget_remaining(deadline)?,
                || {
                    retire_owner_state_effect(
                        state,
                        retirement_budget_remaining(deadline)?,
                        poll_interval,
                    )
                },
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
                    let cleanup = retire_owner_state_effect(
                        state,
                        retirement_budget_remaining(deadline)?,
                        poll_interval,
                    );
                    cleanup?;
                    Err(c3_journal_io_error(C3SlotJournalError::Unknown))
                }
                Err(error) => {
                    let cleanup = retire_owner_state_effect(
                        state,
                        retirement_budget_remaining(deadline)?,
                        poll_interval,
                    );
                    cleanup?;
                    Err(c3_journal_io_error(error))
                }
            }
        }
        C3SlotJournalReadback::Unknown => {
            retire_owner_state_effect(
                state,
                retirement_budget_remaining(deadline)?,
                poll_interval,
            )?;
            Err(c3_journal_io_error(C3SlotJournalError::Unknown))
        }
    }
}

fn retirement_budget_remaining(deadline: Instant) -> io::Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
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
        terminate_launch_bounded(state, process_id, token, wait_budget, poll_interval)?;
    }

    let shutdown_result = {
        let OwnerState {
            lifecycle,
            launches,
            current,
            ..
        } = state;
        lifecycle.shutdown(|process_id, token| {
            revoke_registered_launch(launches, current, process_id, *token)
        })
    };
    prune_retired_launches(state);
    if state.current.is_none() && state.lifecycle.active_len() == 0 {
        state.launches.clear();
        return validate_quiescent_retired_state(state);
    }
    Err(shutdown_result.err().map(backend_error).unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "C3 system-service children remain owned after shutdown pass",
        )
    }))
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
) -> io::Result<()> {
    if highest_catalog_generation.is_some_and(|highest| next.catalog_generation <= highest) {
        return Err(permission_denied(
            "A system-service catalog generation does not advance the slot high-water mark",
        ));
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

fn register_prepared_launch<Os: ProcessOs>(
    launches: &mut Vec<OwnedLaunch>,
    current: &mut Option<LaunchToken>,
    os: &mut Os,
    prepared: &ProcessHandle<Prepared, Os::Raw>,
    process_id: u32,
    token: LaunchToken,
    service_id: &str,
    scope: RecoveryScopeSnapshot,
    scope_data_address: usize,
    previous_token: Option<LaunchToken>,
    highest_catalog_generation: &mut Option<NonZeroU64>,
    replacement_committed: &mut bool,
) -> Result<LaunchToken, BackendError> {
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
        scope,
        scope_data_address,
        launch_record,
        state: LaunchState::Prepared,
    });
    *highest_catalog_generation = Some(catalog_generation);
    Ok(token)
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

fn terminate_launch(state: &mut OwnerState, process_id: u32, token: LaunchToken) -> io::Result<()> {
    let OwnerState {
        lifecycle,
        launches,
        current,
        ..
    } = state;
    lifecycle
        .terminate_exact(process_id, token, |pid, record| {
            revoke_registered_launch(launches, current, pid, *record)
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
    let deadline = std::time::Instant::now()
        .checked_add(wait_budget)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "system-service retirement wait budget is too large",
            )
        })?;
    let first_error = terminate_launch(state, process_id, token).err();
    loop {
        if find_launch(&state.launches, token).is_some_and(|entry| {
            entry.process_id == process_id && entry.state == LaunchState::Retired
        }) {
            prune_retired_launches(state);
            return Ok(());
        }
        let _ = poll_state(state);
        if find_launch(&state.launches, token).is_some_and(|entry| {
            entry.process_id == process_id && entry.state == LaunchState::Retired
        }) {
            prune_retired_launches(state);
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
    lifecycle
        .poll(|process_id, token| revoke_registered_launch(launches, current, process_id, *token))
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
    let result = poll_state(state).map_err(backend_error);
    prune_retired_launches(state);
    if had_current && state.current.is_none() {
        record_current_absent_after_reap(state, deadline)?;
    }
    result
}

/// Record an externally observed exact-child exit after PID-1's reaper has
/// removed the current launch. The process exit itself is not an owner action;
/// the durable receipt follows the post-reap state check and journal readback.
fn record_current_absent_after_reap(
    state: &mut OwnerState,
    deadline: Option<Instant>,
) -> io::Result<()> {
    if state.current.is_some() {
        return Err(io::Error::other(
            "C3 current slot changed while recording an observed child exit",
        ));
    }
    let Some(journal) = state.durable_journal.clone() else {
        return Ok(());
    };
    let unknown = || c3_journal_io_error(C3SlotJournalError::Unknown);
    let readback = match deadline {
        Some(deadline) => journal
            .readback_with_timeout(retirement_budget_remaining(deadline).map_err(|_| unknown())?),
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
                Some(deadline) => journal.transition_next_with_timeout(
                    catalog_generation,
                    c3_launch_generation,
                    scope_binding_digest,
                    C3SlotState::ClearedAndAbsent,
                    retirement_budget_remaining(deadline).map_err(|_| unknown())?,
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
        entry.state != LaunchState::Retired || lifecycle.tracks_record(&entry.token)
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
    use super::{LinuxC3SystemServiceSlotOwner, SystemServiceRecoveryAdmissionScope};
    use crate::linux_init::linux_init_c3_slot_journal::{
        C3SlotJournal, C3SlotJournalReadback, C3SlotState,
    };
    use crate::linux_init::linux_process_resource_tests::namespace_root_probe_elf;
    use nagi_system_service_recovery_contract::{
        canonical_recovery_binding_digest, C1RecoveryOperationCorrelation,
        C3DurableRecoverySlotReceipt, C3RecoverySlotState, RecoveryClientError,
        RecoveryLifecycleTransition, SystemServiceRecoveryScopeInput,
    };
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    use std::num::NonZeroU64;
    use std::os::fd::AsFd;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::os::unix::net::UnixStream;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::process::{Child, Command};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    static NEXT_JOURNAL_DIR: AtomicU64 = AtomicU64::new(1);

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
            let service_id = service_id.to_owned();
            let catalog_generation = NonZeroU64::new(catalog_generation).unwrap();
            let signed_package_sha256 = [0x31; 32];
            let executable_sha256: [u8; 32] = Sha256::digest(namespace_root_probe_elf()).into();
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

    fn launch(owner: &LinuxC3SystemServiceSlotOwner, scope: &FixtureScope) {
        let image = namespace_root_probe_elf();
        launch_image(owner, scope, &image).unwrap();
    }

    fn launch_image(
        owner: &LinuxC3SystemServiceSlotOwner,
        scope: &FixtureScope,
        image: &[u8],
    ) -> std::io::Result<()> {
        unsafe {
            owner.launch_admitted(
                scope,
                image,
                &[],
                true,
                Duration::from_secs(10),
                Duration::from_millis(20),
                Duration::from_secs(2),
                Duration::from_millis(10),
                || false,
            )
        }
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
                .launch_and_authenticate_admitted(
                    scope,
                    &image,
                    &[],
                    true,
                    Duration::from_secs(10),
                    Duration::from_millis(20),
                    Duration::from_secs(2),
                    Duration::from_millis(10),
                    || false,
                )
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
        let endpoint = proof.endpoint().unwrap().try_clone_to_owned().unwrap();
        let mut stream = UnixStream::from(endpoint);
        let mut marker = [0u8; 8];
        stream.read_exact(&mut marker).unwrap();
        assert_eq!(&marker, b"ISOLATED");
        stream.write_all(b"Q").unwrap();
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
        confirm_isolated_endpoint(&proof);
        assert!(proof.is_live().unwrap());
        assert_eq!(owner.active_child_count().unwrap(), 1);

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
