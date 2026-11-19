//! C3-private Linux file journal for exact system-service slot transitions.
//!
//! Each operation is written as a durable intent before its effect callback
//! runs, then as a durable terminal row after the callback succeeds. An
//! incomplete or unreadable journal is `Unknown` and cannot run another
//! effect. The journal is append-only so opaque operation IDs can be checked
//! for exact replay after a process restart.

use nagi_system_service_recovery_contract::C1RecoveryOperationCorrelation;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::num::NonZeroU64;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const JOURNAL_MAGIC: &[u8; 8] = b"NGIC3SL1";
const JOURNAL_VERSION: u8 = 3;
const HEADER_LEN: usize = 85;
const CHECKSUM_LEN: usize = 32;
const MAX_SERVICE_ID_LEN: usize = 128;
const LOCK_EX: i32 = 2;
const LOCK_NB: i32 = 4;
const LOCK_UN: i32 = 8;
const DEFAULT_JOURNAL_LOCK_WAIT: Duration = Duration::from_secs(2);
const JOURNAL_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(2);
const O_CLOEXEC_LINUX: i32 = 0o2000000;
const O_NOFOLLOW_LINUX: i32 = 0o400000;

unsafe extern "C" {
    fn flock(fd: i32, operation: i32) -> i32;
    fn geteuid() -> u32;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum C3SlotState {
    Reserved,
    Live,
    ClearedAndAbsent,
    Quarantined,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum C3SlotJournalError {
    InvalidInput,
    Rejected,
    Replay,
    Unknown,
}

/// A candidate transition requested by C3. Its fields stay private so the
/// journal always validates the exact service, binding, generations and ID.
struct C3SlotTransition {
    service_id: String,
    catalog_generation: NonZeroU64,
    c3_launch_generation: NonZeroU64,
    scope_binding_digest: [u8; 32],
    operation_id: NonZeroU64,
    c1_operation_id: Option<NonZeroU64>,
    state: C3SlotState,
}

impl C3SlotTransition {
    fn new(
        service_id: &str,
        catalog_generation: NonZeroU64,
        c3_launch_generation: NonZeroU64,
        scope_binding_digest: [u8; 32],
        operation_id: NonZeroU64,
        state: C3SlotState,
    ) -> Result<Self, C3SlotJournalError> {
        if !canonical_service_id(service_id) {
            return Err(C3SlotJournalError::InvalidInput);
        }
        Ok(Self {
            service_id: service_id.to_owned(),
            catalog_generation,
            c3_launch_generation,
            scope_binding_digest,
            operation_id,
            c1_operation_id: None,
            state,
        })
    }
}

/// Receipt is constructible only from the exact completed row read back after
/// its durable append. The owner module implements A's shared receipt view.
pub(super) struct C3DurableSlotReceipt {
    service_id: String,
    catalog_generation: NonZeroU64,
    c3_launch_generation: NonZeroU64,
    scope_binding_digest: [u8; 32],
    durable_revision: NonZeroU64,
    operation_id: NonZeroU64,
    c1_operation_id: Option<NonZeroU64>,
    state: C3SlotState,
}

impl C3DurableSlotReceipt {
    fn from_completed_readback(record: &JournalRecord) -> Result<Self, C3SlotJournalError> {
        if record.kind != RecordKind::Committed {
            return Err(C3SlotJournalError::Unknown);
        }
        Ok(Self {
            service_id: record.row.service_id.clone(),
            catalog_generation: NonZeroU64::new(record.row.catalog_generation)
                .ok_or(C3SlotJournalError::Unknown)?,
            c3_launch_generation: NonZeroU64::new(record.row.c3_launch_generation)
                .ok_or(C3SlotJournalError::Unknown)?,
            scope_binding_digest: record.row.scope_binding_digest,
            durable_revision: NonZeroU64::new(record.revision)
                .ok_or(C3SlotJournalError::Unknown)?,
            operation_id: NonZeroU64::new(record.row.operation_id)
                .ok_or(C3SlotJournalError::Unknown)?,
            c1_operation_id: NonZeroU64::new(record.row.c1_operation_id),
            state: record.row.state,
        })
    }

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

    pub(super) fn durable_revision(&self) -> NonZeroU64 {
        self.durable_revision
    }

    pub(super) fn operation_id(&self) -> NonZeroU64 {
        self.operation_id
    }

    pub(super) fn c1_operation_id(&self) -> Option<NonZeroU64> {
        self.c1_operation_id
    }

    pub(super) fn state(&self) -> C3SlotState {
        self.state
    }
}

pub(super) struct C3SlotJournal {
    path: PathBuf,
    lock_path: PathBuf,
    parent: PathBuf,
    service_id: String,
}

#[derive(Clone, Copy)]
pub(super) struct C3SlotReservation {
    launch_generation: NonZeroU64,
    operation_id: NonZeroU64,
}

impl C3SlotReservation {
    pub(super) fn launch_generation(&self) -> NonZeroU64 {
        self.launch_generation
    }

    pub(super) fn operation_id(&self) -> NonZeroU64 {
        self.operation_id
    }
}

impl C3SlotJournal {
    pub(super) fn open(
        path: impl AsRef<Path>,
        service_id: &str,
    ) -> Result<Self, C3SlotJournalError> {
        let journal = Self::from_path(path, service_id)?;
        journal.verify_initialized()?;
        Ok(journal)
    }

    #[cfg(test)]
    pub(super) fn create(
        path: impl AsRef<Path>,
        service_id: &str,
    ) -> Result<Self, C3SlotJournalError> {
        let journal = Self::from_path(path, service_id)?;
        let lock_file =
            create_new_private_file(&journal.lock_path).map_err(|_| C3SlotJournalError::Unknown)?;
        validate_private_file(&lock_file).map_err(|_| C3SlotJournalError::Rejected)?;
        lock_file
            .sync_all()
            .map_err(|_| C3SlotJournalError::Unknown)?;
        sync_parent(&journal.parent).map_err(|_| C3SlotJournalError::Unknown)?;

        let mut file =
            create_new_private_file(&journal.path).map_err(|_| C3SlotJournalError::Unknown)?;
        validate_private_file(&file).map_err(|_| C3SlotJournalError::Rejected)?;
        file.write_all(
            &encode_record(&genesis_record(service_id)).map_err(|_| C3SlotJournalError::Unknown)?,
        )
        .map_err(|_| C3SlotJournalError::Unknown)?;
        file.sync_all().map_err(|_| C3SlotJournalError::Unknown)?;
        sync_parent(&journal.parent).map_err(|_| C3SlotJournalError::Unknown)?;
        journal.verify_initialized()?;
        Ok(journal)
    }

    fn from_path(path: impl AsRef<Path>, service_id: &str) -> Result<Self, C3SlotJournalError> {
        if !canonical_service_id(service_id) {
            return Err(C3SlotJournalError::InvalidInput);
        }
        let path = path.as_ref();
        let file_name = path
            .file_name()
            .filter(|name| !name.is_empty())
            .ok_or(C3SlotJournalError::InvalidInput)?;
        let requested_parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let parent = fs::canonicalize(requested_parent).map_err(|_| C3SlotJournalError::Unknown)?;
        validate_parent(&parent).map_err(|_| C3SlotJournalError::Rejected)?;

        let path = parent.join(file_name);
        let mut lock_name: OsString = file_name.to_os_string();
        lock_name.push(".lock");
        Ok(Self {
            path,
            lock_path: parent.join(lock_name),
            parent,
            service_id: service_id.to_owned(),
        })
    }

    fn verify_initialized(&self) -> Result<(), C3SlotJournalError> {
        let _lock = self.lock_exclusive()?;
        let scan = scan_journal(&self.path, &self.service_id, None)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        if scan.initialized {
            Ok(())
        } else {
            Err(C3SlotJournalError::Unknown)
        }
    }

    /// Durably reserve the next C3 generation before it is returned to A.
    /// A reservation left by a crashed owner is unknown and cannot be reused.
    pub(super) fn reserve_next_generation(&self) -> Result<C3SlotReservation, C3SlotJournalError> {
        let _lock = self.lock_exclusive()?;
        let scan = scan_journal(&self.path, &self.service_id, None)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        if scan.pending.is_some() || scan.reservation.is_some() {
            return Err(C3SlotJournalError::Unknown);
        }
        if scan
            .current
            .as_ref()
            .is_some_and(|current| current.row.state == C3SlotState::Quarantined)
        {
            return Err(C3SlotJournalError::Rejected);
        }

        let launch_generation = scan
            .highest_c3_launch_generation
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .ok_or(C3SlotJournalError::Unknown)?;
        let operation_id = scan
            .highest_operation_id
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .ok_or(C3SlotJournalError::Unknown)?;
        let record = JournalRecord {
            kind: RecordKind::Reservation,
            revision: scan
                .last_revision
                .checked_add(1)
                .filter(|revision| *revision != 0)
                .ok_or(C3SlotJournalError::Unknown)?,
            row: JournalRow {
                service_id: self.service_id.clone(),
                catalog_generation: 0,
                c3_launch_generation: launch_generation.get(),
                scope_binding_digest: [0; 32],
                operation_id: operation_id.get(),
                c1_operation_id: 0,
                state: C3SlotState::Reserved,
            },
        };
        let scan = self
            .append_and_readback(&record, None)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        if scan.reservation.as_ref() != Some(&record) {
            return Err(C3SlotJournalError::Unknown);
        }
        Ok(C3SlotReservation {
            launch_generation,
            operation_id,
        })
    }

    /// Consume the opaque reservation token returned in this process. A
    /// reopened process cannot reconstruct this token from the journal row.
    pub(super) fn transition_reserved_launch(
        &self,
        reservation: C3SlotReservation,
        catalog_generation: NonZeroU64,
        scope_binding_digest: [u8; 32],
        effect: impl FnOnce() -> io::Result<()>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        let _lock = self.lock_exclusive()?;
        let mut scan = scan_journal(&self.path, &self.service_id, Some(reservation.operation_id))
            .map_err(|_| C3SlotJournalError::Unknown)?;
        let transition = C3SlotTransition {
            service_id: self.service_id.clone(),
            catalog_generation,
            c3_launch_generation: reservation.launch_generation,
            scope_binding_digest,
            operation_id: reservation.operation_id,
            c1_operation_id: None,
            state: C3SlotState::Live,
        };
        self.transition_scanned(&mut scan, transition, effect)
    }

    /// Bind an otherwise unconsumed reservation to the exact A scope and
    /// durably quarantine it without creating a child. Reusing the reserved
    /// C3 operation ID consumes only that opaque reservation.
    pub(super) fn transition_reserved_quarantine(
        &self,
        reservation: C3SlotReservation,
        catalog_generation: NonZeroU64,
        scope_binding_digest: [u8; 32],
        effect: impl FnOnce() -> io::Result<()>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        let _lock = self.lock_exclusive()?;
        let mut scan = scan_journal(&self.path, &self.service_id, Some(reservation.operation_id))
            .map_err(|_| C3SlotJournalError::Unknown)?;
        let transition = C3SlotTransition::new(
            &self.service_id,
            catalog_generation,
            reservation.launch_generation,
            scope_binding_digest,
            reservation.operation_id,
            C3SlotState::Quarantined,
        )?;
        self.transition_scanned(&mut scan, transition, effect)
    }

    /// Run one effect only after its intent is durable. The returned receipt
    /// follows the completed row's sync and full-journal readback.
    #[cfg(test)]
    fn transition_once(
        &self,
        transition: C3SlotTransition,
        effect: impl FnOnce() -> io::Result<()>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        if transition.service_id != self.service_id {
            return Err(C3SlotJournalError::Rejected);
        }
        let _lock = self.lock_exclusive()?;
        let mut scan = scan_journal(&self.path, &self.service_id, Some(transition.operation_id))
            .map_err(|_| C3SlotJournalError::Unknown)?;
        self.transition_scanned(&mut scan, transition, effect)
    }

    /// Allocate a fresh operation ID under the same file lock that serializes
    /// the transition, then run and persist the exact effect.
    pub(super) fn transition_next(
        &self,
        catalog_generation: NonZeroU64,
        c3_launch_generation: NonZeroU64,
        scope_binding_digest: [u8; 32],
        state: C3SlotState,
        effect: impl FnOnce() -> io::Result<()>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        self.transition_next_with_timeout(
            catalog_generation,
            c3_launch_generation,
            scope_binding_digest,
            state,
            DEFAULT_JOURNAL_LOCK_WAIT,
            effect,
        )
    }

    pub(super) fn transition_next_with_timeout(
        &self,
        catalog_generation: NonZeroU64,
        c3_launch_generation: NonZeroU64,
        scope_binding_digest: [u8; 32],
        state: C3SlotState,
        lock_wait: Duration,
        effect: impl FnOnce() -> io::Result<()>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        let _lock = self.lock_exclusive_with_timeout(lock_wait)?;
        let mut scan = scan_journal(&self.path, &self.service_id, None)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        let operation_id = scan
            .highest_operation_id
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .ok_or(C3SlotJournalError::Unknown)?;
        let transition = C3SlotTransition::new(
            &self.service_id,
            catalog_generation,
            c3_launch_generation,
            scope_binding_digest,
            operation_id,
            state,
        )?;
        self.transition_scanned(&mut scan, transition, effect)
    }

    fn transition_scanned(
        &self,
        scan: &mut JournalScan,
        transition: C3SlotTransition,
        effect: impl FnOnce() -> io::Result<()>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        if scan.pending.is_some() {
            return Err(C3SlotJournalError::Unknown);
        }
        if scan.requested_operation_seen {
            return Err(C3SlotJournalError::Replay);
        }

        let row = JournalRow {
            service_id: transition.service_id,
            catalog_generation: transition.catalog_generation.get(),
            c3_launch_generation: transition.c3_launch_generation.get(),
            scope_binding_digest: transition.scope_binding_digest,
            operation_id: transition.operation_id.get(),
            c1_operation_id: transition.c1_operation_id.map_or(0, NonZeroU64::get),
            state: transition.state,
        };
        let reserved_transition = scan.reservation.as_ref().is_some_and(|reservation| {
            reservation.row.operation_id == row.operation_id
                && reservation.row.c3_launch_generation == row.c3_launch_generation
                && matches!(row.state, C3SlotState::Live | C3SlotState::Quarantined)
                && row.catalog_generation >= scan.highest_catalog_generation
                && scan
                    .current
                    .as_ref()
                    .is_none_or(|current| current.row.state != C3SlotState::Quarantined)
        });
        let is_valid = if reserved_transition {
            true
        } else if row.state == C3SlotState::Live {
            false
        } else {
            scan.reservation.is_none()
                && row.operation_id > scan.highest_operation_id
                && valid_transition(
                    scan.current.as_ref().map(|record| &record.row),
                    scan.highest_catalog_generation,
                    scan.highest_c3_launch_generation,
                    &row,
                )
        };
        if !is_valid {
            return Err(C3SlotJournalError::Rejected);
        }

        let intent_revision = scan
            .last_revision
            .checked_add(1)
            .filter(|revision| *revision != 0)
            .ok_or(C3SlotJournalError::Unknown)?;
        let intent = JournalRecord {
            kind: RecordKind::Intent,
            revision: intent_revision,
            row,
        };
        *scan = self
            .append_and_readback(&intent, Some(transition.operation_id))
            .map_err(|_| C3SlotJournalError::Unknown)?;
        if scan.pending.as_ref() != Some(&intent) {
            return Err(C3SlotJournalError::Unknown);
        }

        match catch_unwind(AssertUnwindSafe(effect)) {
            Ok(Ok(())) => {}
            Ok(Err(_)) | Err(_) => return Err(C3SlotJournalError::Unknown),
        }

        let completed = JournalRecord {
            kind: RecordKind::Committed,
            revision: intent_revision
                .checked_add(1)
                .filter(|revision| *revision != 0)
                .ok_or(C3SlotJournalError::Unknown)?,
            row: intent.row,
        };
        *scan = self
            .append_and_readback(&completed, Some(transition.operation_id))
            .map_err(|_| C3SlotJournalError::Unknown)?;
        if scan.pending.is_some() || scan.current.as_ref() != Some(&completed) {
            return Err(C3SlotJournalError::Unknown);
        }
        C3DurableSlotReceipt::from_completed_readback(
            scan.current.as_ref().ok_or(C3SlotJournalError::Unknown)?,
        )
    }

    pub(super) fn readback(&self) -> C3SlotJournalReadback {
        self.readback_with_timeout(DEFAULT_JOURNAL_LOCK_WAIT)
    }

    pub(super) fn readback_with_timeout(&self, lock_wait: Duration) -> C3SlotJournalReadback {
        let Ok(_lock) = self.lock_exclusive_with_timeout(lock_wait) else {
            return C3SlotJournalReadback::Unknown;
        };
        let Ok(scan) = scan_journal(&self.path, &self.service_id, None) else {
            return C3SlotJournalReadback::Unknown;
        };
        if scan.pending.is_some() || scan.reservation.is_some() {
            return C3SlotJournalReadback::Unknown;
        }
        match scan.current {
            Some(record) => C3SlotJournalReadback::Committed(C3DurableSlotRowReadback {
                service_id: record.row.service_id,
                catalog_generation: record.row.catalog_generation,
                c3_launch_generation: record.row.c3_launch_generation,
                scope_binding_digest: record.row.scope_binding_digest,
                durable_revision: record.revision,
                operation_id: record.row.operation_id,
                c1_operation_id: record.row.c1_operation_id,
                state: record.row.state,
                highest_catalog_generation: scan.highest_catalog_generation,
                highest_c3_launch_generation: scan.highest_c3_launch_generation,
                highest_operation_id: scan.highest_operation_id,
            }),
            None => C3SlotJournalReadback::Empty,
        }
    }

    /// Persist the exact C1 operation correlation before invoking its terminal
    /// callback, then return only after the correlated C3 row is committed and
    /// read back. A panic or incomplete callback leaves an intent that makes
    /// the journal unknown. The callback must not re-enter this journal or C3.
    pub(super) fn with_c1_correlated_cleared_receipt<R>(
        &self,
        catalog_generation: NonZeroU64,
        c3_launch_generation: NonZeroU64,
        scope_binding_digest: &[u8; 32],
        correlation: C1RecoveryOperationCorrelation,
        effect: impl FnOnce() -> R,
    ) -> Result<(R, C3DurableSlotReceipt), C3SlotJournalError> {
        if correlation.binding_digest() != scope_binding_digest {
            return Err(C3SlotJournalError::Rejected);
        }
        let _lock = self.lock_exclusive()?;
        let mut scan = scan_journal(&self.path, &self.service_id, None)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        if scan.pending.is_some() || scan.reservation.is_some() {
            return Err(C3SlotJournalError::Unknown);
        }
        let before = scan.current.as_ref().ok_or(C3SlotJournalError::Unknown)?;
        if before.kind != RecordKind::Committed
            || before.row.service_id != self.service_id
            || before.row.catalog_generation != catalog_generation.get()
            || before.row.c3_launch_generation != c3_launch_generation.get()
            || &before.row.scope_binding_digest != scope_binding_digest
            || before.row.state != C3SlotState::ClearedAndAbsent
        {
            return Err(C3SlotJournalError::Rejected);
        }

        let operation_id = scan
            .highest_operation_id
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .ok_or(C3SlotJournalError::Unknown)?;
        let mut transition = C3SlotTransition::new(
            &self.service_id,
            catalog_generation,
            c3_launch_generation,
            *scope_binding_digest,
            operation_id,
            C3SlotState::ClearedAndAbsent,
        )?;
        transition.c1_operation_id = Some(correlation.operation_id());
        let mut result = None;
        let receipt = self.transition_scanned(&mut scan, transition, || {
            result = Some(effect());
            Ok(())
        })?;
        if receipt.c1_operation_id() != Some(correlation.operation_id())
            || receipt.scope_binding_digest() != scope_binding_digest
            || receipt.state() != C3SlotState::ClearedAndAbsent
        {
            return Err(C3SlotJournalError::Unknown);
        }
        let result = result.ok_or(C3SlotJournalError::Unknown)?;
        Ok((result, receipt))
    }

    fn lock_exclusive(&self) -> Result<ExclusiveFileLock, C3SlotJournalError> {
        self.lock_exclusive_with_timeout(DEFAULT_JOURNAL_LOCK_WAIT)
    }

    fn lock_exclusive_with_timeout(
        &self,
        lock_wait: Duration,
    ) -> Result<ExclusiveFileLock, C3SlotJournalError> {
        let file = open_existing_private_file(&self.lock_path, false)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        validate_private_file(&file).map_err(|_| C3SlotJournalError::Rejected)?;
        ExclusiveFileLock::acquire_with_timeout(file, lock_wait)
            .map_err(|_| C3SlotJournalError::Unknown)
    }

    fn append_and_readback(
        &self,
        record: &JournalRecord,
        requested_operation_id: Option<NonZeroU64>,
    ) -> Result<JournalScan, ()> {
        let mut file = open_existing_private_file(&self.path, true).map_err(|_| ())?;
        validate_private_file(&file).map_err(|_| ())?;
        let encoded = encode_record(record)?;
        file.write_all(&encoded).map_err(|_| ())?;
        file.sync_all().map_err(|_| ())?;
        drop(file);

        let scan = scan_journal(&self.path, &self.service_id, requested_operation_id)?;
        if scan.last_record.as_ref() != Some(record) {
            return Err(());
        }
        match record.kind {
            RecordKind::Genesis => Err(()),
            RecordKind::Reservation if scan.reservation.as_ref() == Some(record) => Ok(scan),
            RecordKind::Intent if scan.pending.as_ref() == Some(record) => Ok(scan),
            RecordKind::Committed
                if scan.pending.is_none() && scan.current.as_ref() == Some(record) =>
            {
                Ok(scan)
            }
            _ => Err(()),
        }
    }
}

pub(super) enum C3SlotJournalReadback {
    Empty,
    Committed(C3DurableSlotRowReadback),
    Unknown,
}

pub(super) struct C3DurableSlotRowReadback {
    service_id: String,
    catalog_generation: u64,
    c3_launch_generation: u64,
    scope_binding_digest: [u8; 32],
    durable_revision: u64,
    operation_id: u64,
    c1_operation_id: u64,
    state: C3SlotState,
    highest_catalog_generation: u64,
    highest_c3_launch_generation: u64,
    highest_operation_id: u64,
}

impl C3DurableSlotRowReadback {
    pub(super) fn service_id(&self) -> &str {
        &self.service_id
    }

    pub(super) fn catalog_generation(&self) -> u64 {
        self.catalog_generation
    }

    pub(super) fn c3_launch_generation(&self) -> u64 {
        self.c3_launch_generation
    }

    pub(super) fn scope_binding_digest(&self) -> &[u8; 32] {
        &self.scope_binding_digest
    }

    pub(super) fn durable_revision(&self) -> u64 {
        self.durable_revision
    }

    pub(super) fn operation_id(&self) -> u64 {
        self.operation_id
    }

    pub(super) fn c1_operation_id(&self) -> Option<NonZeroU64> {
        NonZeroU64::new(self.c1_operation_id)
    }

    pub(super) fn state(&self) -> C3SlotState {
        self.state
    }

    pub(super) fn highest_catalog_generation(&self) -> u64 {
        self.highest_catalog_generation
    }

    pub(super) fn highest_c3_launch_generation(&self) -> u64 {
        self.highest_c3_launch_generation
    }

    pub(super) fn highest_operation_id(&self) -> u64 {
        self.highest_operation_id
    }
}

struct ExclusiveFileLock {
    file: File,
}

impl ExclusiveFileLock {
    fn acquire_with_timeout(file: File, timeout: Duration) -> io::Result<Self> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "lock timeout overflow"))?;
        loop {
            let result = unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
            if result == 0 {
                return Ok(Self { file });
            }

            let error = io::Error::last_os_error();
            match error.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "timed out waiting for the C3 slot journal lock",
                        ));
                    }
                    std::thread::sleep(remaining.min(JOURNAL_LOCK_RETRY_INTERVAL));
                }
                _ => return Err(error),
            }
        }
    }
}

impl Drop for ExclusiveFileLock {
    fn drop(&mut self) {
        let _ = unsafe { flock(self.file.as_raw_fd(), LOCK_UN) };
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordKind {
    Genesis,
    Intent,
    Committed,
    Reservation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct JournalRow {
    service_id: String,
    catalog_generation: u64,
    c3_launch_generation: u64,
    scope_binding_digest: [u8; 32],
    operation_id: u64,
    c1_operation_id: u64,
    state: C3SlotState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct JournalRecord {
    kind: RecordKind,
    revision: u64,
    row: JournalRow,
}

#[derive(Default)]
struct JournalScan {
    initialized: bool,
    current: Option<JournalRecord>,
    pending: Option<JournalRecord>,
    reservation: Option<JournalRecord>,
    last_record: Option<JournalRecord>,
    last_revision: u64,
    highest_catalog_generation: u64,
    highest_c3_launch_generation: u64,
    highest_operation_id: u64,
    requested_operation_seen: bool,
}

fn canonical_service_id(value: &str) -> bool {
    value.len() <= MAX_SERVICE_ID_LEN
        && !value.is_empty()
        && value.split('.').all(|segment| {
            let bytes = segment.as_bytes();
            if bytes.is_empty() || !bytes[0].is_ascii_lowercase() {
                return false;
            }
            let mut previous_separator = false;
            for byte in &bytes[1..] {
                if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
                    previous_separator = false;
                } else if *byte == b'-' && !previous_separator {
                    previous_separator = true;
                } else {
                    return false;
                }
            }
            !previous_separator
        })
}

fn valid_transition(
    current: Option<&JournalRow>,
    highest_catalog_generation: u64,
    highest_c3_launch_generation: u64,
    next: &JournalRow,
) -> bool {
    if !canonical_service_id(&next.service_id)
        || next.catalog_generation == 0
        || next.c3_launch_generation == 0
        || next.operation_id == 0
        || (next.c1_operation_id != 0 && next.state != C3SlotState::ClearedAndAbsent)
        || next.catalog_generation < highest_catalog_generation
        || next.c3_launch_generation < highest_c3_launch_generation
    {
        return false;
    }

    match next.state {
        C3SlotState::Live => {
            next.c3_launch_generation > highest_c3_launch_generation
                && current.is_none_or(|current| current.state != C3SlotState::Quarantined)
        }
        C3SlotState::Reserved => false,
        C3SlotState::ClearedAndAbsent => current.is_some_and(|current| {
            same_slot_binding(current, next)
                && if current.state == C3SlotState::ClearedAndAbsent {
                    next.c1_operation_id != 0 && current.c1_operation_id != next.c1_operation_id
                } else {
                    next.c1_operation_id == 0
                }
        }),
        C3SlotState::Quarantined => current.is_some_and(|current| {
            matches!(
                current.state,
                C3SlotState::Live | C3SlotState::ClearedAndAbsent
            ) && same_slot_binding(current, next)
        }),
    }
}

fn same_slot_binding(left: &JournalRow, right: &JournalRow) -> bool {
    left.service_id == right.service_id
        && left.catalog_generation == right.catalog_generation
        && left.c3_launch_generation == right.c3_launch_generation
        && left.scope_binding_digest == right.scope_binding_digest
}

fn genesis_record(service_id: &str) -> JournalRecord {
    JournalRecord {
        kind: RecordKind::Genesis,
        revision: 1,
        row: JournalRow {
            service_id: service_id.to_owned(),
            catalog_generation: 0,
            c3_launch_generation: 0,
            scope_binding_digest: [0; 32],
            operation_id: 0,
            c1_operation_id: 0,
            state: C3SlotState::Reserved,
        },
    }
}

fn scan_journal(
    path: &Path,
    service_id: &str,
    requested_operation_id: Option<NonZeroU64>,
) -> Result<JournalScan, ()> {
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW_LINUX | O_CLOEXEC_LINUX)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(()),
        Err(_) => return Err(()),
    };
    validate_private_file(&file).map_err(|_| ())?;
    let mut scan = JournalScan::default();
    while let Some(record) = read_record(&mut file)? {
        if record.row.service_id != service_id
            || record.revision
                != scan
                    .last_revision
                    .checked_add(1)
                    .filter(|revision| *revision != 0)
                    .ok_or(())?
        {
            return Err(());
        }
        if !scan.initialized {
            if record != genesis_record(service_id) {
                return Err(());
            }
            scan.initialized = true;
            scan.last_revision = record.revision;
            scan.last_record = Some(record);
            continue;
        }
        if record.kind == RecordKind::Genesis {
            return Err(());
        }
        if record.kind == RecordKind::Intent
            && requested_operation_id
                .is_some_and(|operation_id| operation_id.get() == record.row.operation_id)
        {
            scan.requested_operation_seen = true;
        }

        match record.kind {
            RecordKind::Genesis => return Err(()),
            RecordKind::Reservation => {
                if scan.pending.is_some()
                    || scan.reservation.is_some()
                    || record.row.state != C3SlotState::Reserved
                    || record.row.catalog_generation != 0
                    || record.row.scope_binding_digest != [0; 32]
                    || record.row.operation_id <= scan.highest_operation_id
                    || record.row.c3_launch_generation <= scan.highest_c3_launch_generation
                    || scan
                        .current
                        .as_ref()
                        .is_some_and(|current| current.row.state == C3SlotState::Quarantined)
                {
                    return Err(());
                }
                scan.highest_operation_id = record.row.operation_id;
                scan.highest_c3_launch_generation = record.row.c3_launch_generation;
                scan.reservation = Some(record.clone());
            }
            RecordKind::Intent => {
                let reserved_transition = scan.reservation.as_ref().is_some_and(|reservation| {
                    reservation.row.operation_id == record.row.operation_id
                        && reservation.row.c3_launch_generation == record.row.c3_launch_generation
                        && matches!(
                            record.row.state,
                            C3SlotState::Live | C3SlotState::Quarantined
                        )
                        && record.row.catalog_generation >= scan.highest_catalog_generation
                        && scan
                            .current
                            .as_ref()
                            .is_none_or(|current| current.row.state != C3SlotState::Quarantined)
                });
                let operation_id_is_valid = if reserved_transition {
                    true
                } else {
                    record.row.operation_id > scan.highest_operation_id
                };
                let transition_is_valid = if reserved_transition {
                    true
                } else if record.row.state == C3SlotState::Live {
                    false
                } else {
                    scan.reservation.is_none()
                        && valid_transition(
                            scan.current.as_ref().map(|current| &current.row),
                            scan.highest_catalog_generation,
                            scan.highest_c3_launch_generation,
                            &record.row,
                        )
                };
                if scan.pending.is_some() || !operation_id_is_valid || !transition_is_valid {
                    return Err(());
                }
                scan.highest_catalog_generation = scan
                    .highest_catalog_generation
                    .max(record.row.catalog_generation);
                scan.highest_c3_launch_generation = scan
                    .highest_c3_launch_generation
                    .max(record.row.c3_launch_generation);
                scan.highest_operation_id = scan.highest_operation_id.max(record.row.operation_id);
                if reserved_transition {
                    scan.reservation = None;
                }
                scan.pending = Some(record.clone());
            }
            RecordKind::Committed => {
                let pending = scan.pending.take().ok_or(())?;
                if record.revision
                    != pending
                        .revision
                        .checked_add(1)
                        .filter(|revision| *revision != 0)
                        .ok_or(())?
                    || record.row != pending.row
                {
                    return Err(());
                }
                scan.current = Some(record.clone());
            }
        }
        scan.last_revision = record.revision;
        scan.last_record = Some(record);
    }
    if !scan.initialized {
        return Err(());
    }
    Ok(scan)
}

fn read_record(reader: &mut impl Read) -> Result<Option<JournalRecord>, ()> {
    let mut header = [0u8; HEADER_LEN];
    match reader.read(&mut header[..1]) {
        Ok(0) => return Ok(None),
        Ok(1) => {}
        Ok(_) => return Err(()),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => return read_record(reader),
        Err(_) => return Err(()),
    }
    reader.read_exact(&mut header[1..]).map_err(|_| ())?;
    if &header[..JOURNAL_MAGIC.len()] != JOURNAL_MAGIC || header[8] != JOURNAL_VERSION {
        return Err(());
    }

    let kind = match header[9] {
        1 => RecordKind::Intent,
        2 => RecordKind::Committed,
        3 => RecordKind::Reservation,
        4 => RecordKind::Genesis,
        _ => return Err(()),
    };
    let state = match header[10] {
        0 => C3SlotState::Reserved,
        1 => C3SlotState::Live,
        2 => C3SlotState::ClearedAndAbsent,
        3 => C3SlotState::Quarantined,
        _ => return Err(()),
    };
    let service_id_len = u16::from_le_bytes([header[11], header[12]]) as usize;
    if service_id_len == 0 || service_id_len > MAX_SERVICE_ID_LEN {
        return Err(());
    }
    let revision = read_u64(&header[13..21])?;
    let operation_id = read_u64(&header[21..29])?;
    let c1_operation_id = read_u64(&header[29..37])?;
    let catalog_generation = read_u64(&header[37..45])?;
    let c3_launch_generation = read_u64(&header[45..53])?;
    if revision == 0 {
        return Err(());
    }
    let mut scope_binding_digest = [0u8; 32];
    scope_binding_digest.copy_from_slice(&header[53..85]);
    let mut service_id_bytes = vec![0u8; service_id_len];
    reader.read_exact(&mut service_id_bytes).map_err(|_| ())?;
    let mut supplied_checksum = [0u8; CHECKSUM_LEN];
    reader.read_exact(&mut supplied_checksum).map_err(|_| ())?;
    let mut checksum_input = Vec::with_capacity(HEADER_LEN + service_id_len);
    checksum_input.extend_from_slice(&header);
    checksum_input.extend_from_slice(&service_id_bytes);
    let expected_checksum: [u8; 32] = Sha256::digest(&checksum_input).into();
    if supplied_checksum != expected_checksum {
        return Err(());
    }
    let service_id = String::from_utf8(service_id_bytes).map_err(|_| ())?;
    if !canonical_service_id(&service_id) {
        return Err(());
    }
    match kind {
        RecordKind::Genesis
            if state == C3SlotState::Reserved
                && revision == 1
                && operation_id == 0
                && c1_operation_id == 0
                && catalog_generation == 0
                && c3_launch_generation == 0
                && scope_binding_digest == [0; 32] => {}
        RecordKind::Reservation
            if state == C3SlotState::Reserved
                && operation_id != 0
                && c1_operation_id == 0
                && c3_launch_generation != 0
                && catalog_generation == 0
                && scope_binding_digest == [0; 32] => {}
        RecordKind::Intent | RecordKind::Committed
            if state != C3SlotState::Reserved
                && operation_id != 0
                && (c1_operation_id == 0 || state == C3SlotState::ClearedAndAbsent)
                && c3_launch_generation != 0
                && catalog_generation != 0 => {}
        _ => return Err(()),
    }

    Ok(Some(JournalRecord {
        kind,
        revision,
        row: JournalRow {
            service_id,
            catalog_generation,
            c3_launch_generation,
            scope_binding_digest,
            operation_id,
            c1_operation_id,
            state,
        },
    }))
}

fn encode_record(record: &JournalRecord) -> Result<Vec<u8>, ()> {
    let service_id = record.row.service_id.as_bytes();
    if service_id.is_empty()
        || service_id.len() > MAX_SERVICE_ID_LEN
        || !canonical_service_id(&record.row.service_id)
        || record.revision == 0
    {
        return Err(());
    }
    match record.kind {
        RecordKind::Genesis
            if record.row.state == C3SlotState::Reserved
                && record.revision == 1
                && record.row.operation_id == 0
                && record.row.c1_operation_id == 0
                && record.row.catalog_generation == 0
                && record.row.c3_launch_generation == 0
                && record.row.scope_binding_digest == [0; 32] => {}
        RecordKind::Reservation
            if record.row.state == C3SlotState::Reserved
                && record.row.operation_id != 0
                && record.row.c1_operation_id == 0
                && record.row.c3_launch_generation != 0
                && record.row.catalog_generation == 0
                && record.row.scope_binding_digest == [0; 32] => {}
        RecordKind::Intent | RecordKind::Committed
            if record.row.state != C3SlotState::Reserved
                && record.row.operation_id != 0
                && (record.row.c1_operation_id == 0
                    || record.row.state == C3SlotState::ClearedAndAbsent)
                && record.row.c3_launch_generation != 0
                && record.row.catalog_generation != 0 => {}
        _ => return Err(()),
    }
    let mut bytes = Vec::with_capacity(HEADER_LEN + service_id.len() + CHECKSUM_LEN);
    bytes.extend_from_slice(JOURNAL_MAGIC);
    bytes.push(JOURNAL_VERSION);
    bytes.push(match record.kind {
        RecordKind::Intent => 1,
        RecordKind::Committed => 2,
        RecordKind::Reservation => 3,
        RecordKind::Genesis => 4,
    });
    bytes.push(match record.row.state {
        C3SlotState::Reserved => 0,
        C3SlotState::Live => 1,
        C3SlotState::ClearedAndAbsent => 2,
        C3SlotState::Quarantined => 3,
    });
    bytes.extend_from_slice(&(service_id.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&record.revision.to_le_bytes());
    bytes.extend_from_slice(&record.row.operation_id.to_le_bytes());
    bytes.extend_from_slice(&record.row.c1_operation_id.to_le_bytes());
    bytes.extend_from_slice(&record.row.catalog_generation.to_le_bytes());
    bytes.extend_from_slice(&record.row.c3_launch_generation.to_le_bytes());
    bytes.extend_from_slice(&record.row.scope_binding_digest);
    if bytes.len() != HEADER_LEN {
        return Err(());
    }
    bytes.extend_from_slice(service_id);
    let checksum: [u8; 32] = Sha256::digest(&bytes).into();
    bytes.extend_from_slice(&checksum);
    Ok(bytes)
}

fn read_u64(bytes: &[u8]) -> Result<u64, ()> {
    let array: [u8; 8] = bytes.try_into().map_err(|_| ())?;
    Ok(u64::from_le_bytes(array))
}

fn create_new_private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(O_NOFOLLOW_LINUX | O_CLOEXEC_LINUX)
        .open(path)
}

fn open_existing_private_file(path: &Path, append: bool) -> io::Result<File> {
    let mut existing = OpenOptions::new();
    existing
        .read(true)
        .write(true)
        .custom_flags(O_NOFOLLOW_LINUX | O_CLOEXEC_LINUX);
    if append {
        existing.append(true);
    }
    existing.open(path)
}

fn validate_parent(parent: &Path) -> io::Result<()> {
    let metadata = fs::metadata(parent)?;
    let effective_uid = unsafe { geteuid() };
    if !metadata.file_type().is_dir()
        || metadata.uid() != effective_uid
        || metadata.mode() & 0o022 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "C3 journal directory is not private to its owner",
        ));
    }
    Ok(())
}

fn validate_private_file(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    let effective_uid = unsafe { geteuid() };
    if !metadata.file_type().is_file()
        || metadata.uid() != effective_uid
        || metadata.mode() & 0o077 != 0
        || metadata.nlink() != 1
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "C3 journal file is not a private, singly-linked regular file",
        ));
    }
    Ok(())
}

fn sync_parent(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(test)]
mod tests {
    use super::{
        C3DurableSlotReceipt, C3SlotJournal, C3SlotJournalError, C3SlotJournalReadback,
        C3SlotState, C3SlotTransition,
    };
    use std::fs::{self, OpenOptions};
    use std::io::{Read, Seek, SeekFrom, Write};
    use std::num::NonZeroU64;
    use std::os::unix::fs::DirBuilderExt;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    const SERVICE_ID: &str = "org.nagi.c3-journal-test2";
    const DIGEST: [u8; 32] = [0x5a; 32];
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(1);
    const CHILD_JOURNAL_ENV: &str = "NAGI_C3_SLOT_JOURNAL_CHILD_PATH";
    const CHILD_EFFECT_ENV: &str = "NAGI_C3_SLOT_JOURNAL_CHILD_EFFECT";
    const CHILD_COMMIT_JOURNAL_ENV: &str = "NAGI_C3_SLOT_JOURNAL_COMMIT_PATH";
    const CHILD_COMMIT_EFFECT_ENV: &str = "NAGI_C3_SLOT_JOURNAL_COMMIT_EFFECT";
    const CHILD_LOCK_JOURNAL_ENV: &str = "NAGI_C3_SLOT_JOURNAL_LOCK_PATH";
    const CHILD_LOCK_SERVICE_ENV: &str = "NAGI_C3_SLOT_JOURNAL_LOCK_SERVICE";
    const CHILD_LOCK_READY_ENV: &str = "NAGI_C3_SLOT_JOURNAL_LOCK_READY";
    const CHILD_LOCK_RELEASE_ENV: &str = "NAGI_C3_SLOT_JOURNAL_LOCK_RELEASE";

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nagi-c3-slot-journal-{}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            builder.create(&path).unwrap();
            Self(path)
        }

        fn journal_path(&self) -> std::path::PathBuf {
            self.0.join("slot.journal")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn nonzero(value: u64) -> NonZeroU64 {
        NonZeroU64::new(value).unwrap()
    }

    fn transition(
        operation_id: u64,
        launch_generation: u64,
        state: C3SlotState,
    ) -> C3SlotTransition {
        C3SlotTransition::new(
            SERVICE_ID,
            nonzero(1),
            nonzero(launch_generation),
            DIGEST,
            nonzero(operation_id),
            state,
        )
        .unwrap()
    }

    fn reserved_live_transition(
        journal: &C3SlotJournal,
        launch_generation: u64,
    ) -> C3SlotTransition {
        let reservation = journal.reserve_next_generation().unwrap();
        assert_eq!(reservation.launch_generation(), nonzero(launch_generation));
        C3SlotTransition::new(
            SERVICE_ID,
            nonzero(1),
            reservation.launch_generation(),
            DIGEST,
            reservation.operation_id(),
            C3SlotState::Live,
        )
        .unwrap()
    }

    fn receipt_row(receipt: &C3DurableSlotReceipt) {
        assert_eq!(receipt.service_id(), SERVICE_ID);
        assert_eq!(receipt.catalog_generation(), nonzero(1));
        assert_eq!(receipt.scope_binding_digest(), &DIGEST);
    }

    #[test]
    fn c3_slot_journal_reopen_preserves_high_water_and_rejects_replay() {
        let directory = TestDirectory::new();
        let path = directory.journal_path();
        let effects = std::sync::atomic::AtomicUsize::new(0);
        let journal = C3SlotJournal::create(&path, SERVICE_ID).unwrap();
        let receipt = journal
            .transition_once(reserved_live_transition(&journal, 1), || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        receipt_row(&receipt);
        assert_eq!(receipt.durable_revision(), nonzero(4));
        assert_eq!(receipt.operation_id(), nonzero(1));
        assert_eq!(receipt.state(), C3SlotState::Live);
        drop(journal);

        let reopened = C3SlotJournal::open(&path, SERVICE_ID).unwrap();
        match reopened.readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.service_id(), SERVICE_ID);
                assert_eq!(row.catalog_generation(), 1);
                assert_eq!(row.c3_launch_generation(), 1);
                assert_eq!(row.scope_binding_digest(), &DIGEST);
                assert_eq!(row.durable_revision(), 4);
                assert_eq!(row.operation_id(), 1);
                assert_eq!(row.highest_catalog_generation(), 1);
                assert_eq!(row.highest_c3_launch_generation(), 1);
                assert_eq!(row.highest_operation_id(), 1);
                assert_eq!(row.state(), C3SlotState::Live);
            }
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                panic!("durable row did not survive journal reopen")
            }
        }

        assert!(matches!(
            reopened.transition_once(transition(1, 2, C3SlotState::Live), || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(C3SlotJournalError::Replay)
        ));
        assert!(matches!(
            reopened.transition_once(transition(2, 1, C3SlotState::Live), || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
            Err(C3SlotJournalError::Rejected)
        ));
        assert_eq!(effects.load(Ordering::SeqCst), 1);

        let next = reopened
            .transition_once(reserved_live_transition(&reopened, 2), || {
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
            .unwrap();
        assert_eq!(next.durable_revision(), nonzero(7));
        assert_eq!(next.c3_launch_generation(), nonzero(2));
        assert_eq!(next.operation_id(), nonzero(2));
        assert_eq!(effects.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn c3_slot_journal_clears_only_the_exact_live_binding() {
        let directory = TestDirectory::new();
        let journal = C3SlotJournal::create(directory.journal_path(), SERVICE_ID).unwrap();
        journal
            .transition_once(reserved_live_transition(&journal, 1), || Ok(()))
            .unwrap();
        let cleared = journal
            .transition_next(
                nonzero(1),
                nonzero(1),
                DIGEST,
                C3SlotState::ClearedAndAbsent,
                || Ok(()),
            )
            .unwrap();
        assert_eq!(cleared.state(), C3SlotState::ClearedAndAbsent);
        match journal.readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.state(), C3SlotState::ClearedAndAbsent);
                assert_eq!(row.c3_launch_generation(), 1);
                assert_eq!(row.highest_c3_launch_generation(), 1);
            }
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                panic!("cleared row did not read back")
            }
        }
    }

    #[test]
    fn c3_slot_journal_lock_wait_is_bounded() {
        let directory = TestDirectory::new();
        let journal = C3SlotJournal::create(directory.journal_path(), SERVICE_ID).unwrap();
        let held_lock = journal.lock_exclusive().unwrap();
        let started = Instant::now();

        assert!(matches!(
            journal.readback_with_timeout(Duration::from_millis(20)),
            C3SlotJournalReadback::Unknown
        ));
        assert!(started.elapsed() < Duration::from_millis(500));
        drop(held_lock);
    }

    #[test]
    fn c3_slot_journal_corrupt_tail_is_unknown_and_never_runs_effect() {
        let directory = TestDirectory::new();
        let path = directory.journal_path();
        let journal = C3SlotJournal::create(&path, SERVICE_ID).unwrap();
        journal
            .transition_once(reserved_live_transition(&journal, 1), || Ok(()))
            .unwrap();
        drop(journal);

        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        file.seek(SeekFrom::End(-1)).unwrap();
        let mut byte = [0u8; 1];
        file.read_exact(&mut byte).unwrap();
        byte[0] ^= 0x80;
        file.seek(SeekFrom::End(-1)).unwrap();
        file.write_all(&byte).unwrap();
        file.sync_all().unwrap();

        let reopened = C3SlotJournal::open(&path, SERVICE_ID).unwrap();
        assert!(matches!(
            reopened.readback(),
            C3SlotJournalReadback::Unknown
        ));
        let mut callback_ran = false;
        assert!(matches!(
            reopened.transition_once(transition(32, 6, C3SlotState::Live), || {
                callback_ran = true;
                Ok(())
            }),
            Err(C3SlotJournalError::Unknown)
        ));
        assert!(!callback_ran);
    }

    #[test]
    fn c3_slot_journal_process_restart_keeps_interrupted_effect_unknown() {
        let directory = TestDirectory::new();
        let journal_path = directory.journal_path();
        let effect_path = directory.0.join("effect.marker");
        let child = std::env::args_os().next().expect("test binary argv[0]");
        let status = Command::new(child)
            .arg("c3_slot_journal_child_crash_after_effect")
            .env(CHILD_JOURNAL_ENV, &journal_path)
            .env(CHILD_EFFECT_ENV, &effect_path)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(73));
        assert_eq!(fs::read(&effect_path).unwrap(), b"effect-once");

        let reopened = C3SlotJournal::open(&journal_path, SERVICE_ID).unwrap();
        assert!(matches!(
            reopened.readback(),
            C3SlotJournalReadback::Unknown
        ));
        let mut callback_ran = false;
        assert!(matches!(
            reopened.transition_once(transition(73, 8, C3SlotState::Live), || {
                callback_ran = true;
                Ok(())
            }),
            Err(C3SlotJournalError::Unknown)
        ));
        assert!(!callback_ran);
        assert_eq!(fs::read(&effect_path).unwrap(), b"effect-once");
    }

    #[test]
    fn c3_slot_journal_process_restart_rejects_committed_operation() {
        let directory = TestDirectory::new();
        let journal_path = directory.journal_path();
        let effect_path = directory.0.join("committed-effect.marker");
        let child = std::env::args_os().next().expect("test binary argv[0]");
        let status = Command::new(child)
            .arg("c3_slot_journal_child_commit_then_exit")
            .env(CHILD_COMMIT_JOURNAL_ENV, &journal_path)
            .env(CHILD_COMMIT_EFFECT_ENV, &effect_path)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(74));
        assert_eq!(fs::read(&effect_path).unwrap(), b"effect-once");

        let reopened = C3SlotJournal::open(&journal_path, SERVICE_ID).unwrap();
        match reopened.readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.operation_id(), 1);
                assert_eq!(row.durable_revision(), 4);
            }
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                panic!("committed row did not survive child process exit")
            }
        }
        let mut callback_ran = false;
        assert!(matches!(
            reopened.transition_once(transition(1, 2, C3SlotState::Live), || {
                callback_ran = true;
                Ok(())
            }),
            Err(C3SlotJournalError::Replay)
        ));
        assert!(!callback_ran);
        assert_eq!(fs::read(&effect_path).unwrap(), b"effect-once");
    }

    #[test]
    fn c3_slot_journal_child_crash_after_effect() {
        let (Ok(journal_path), Ok(effect_path)) = (
            std::env::var(CHILD_JOURNAL_ENV),
            std::env::var(CHILD_EFFECT_ENV),
        ) else {
            return;
        };
        let journal = C3SlotJournal::create(journal_path, SERVICE_ID).unwrap();
        let _ = journal.transition_once(reserved_live_transition(&journal, 1), || {
            let mut effect = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(effect_path)?;
            effect.write_all(b"effect-once")?;
            effect.sync_all()?;
            std::process::exit(73);
        });
        panic!("child effect unexpectedly returned from journal transition");
    }

    #[test]
    fn c3_slot_journal_child_commit_then_exit() {
        let (Ok(journal_path), Ok(effect_path)) = (
            std::env::var(CHILD_COMMIT_JOURNAL_ENV),
            std::env::var(CHILD_COMMIT_EFFECT_ENV),
        ) else {
            return;
        };
        let journal = C3SlotJournal::create(journal_path, SERVICE_ID).unwrap();
        journal
            .transition_once(reserved_live_transition(&journal, 1), || {
                let mut effect = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(effect_path)?;
                effect.write_all(b"effect-once")?;
                effect.sync_all()
            })
            .unwrap();
        std::process::exit(74);
    }

    #[test]
    fn c3_slot_journal_child_holds_lock() {
        let (Ok(journal_path), Ok(service_id), Ok(ready_path), Ok(release_path)) = (
            std::env::var(CHILD_LOCK_JOURNAL_ENV),
            std::env::var(CHILD_LOCK_SERVICE_ENV),
            std::env::var(CHILD_LOCK_READY_ENV),
            std::env::var(CHILD_LOCK_RELEASE_ENV),
        ) else {
            return;
        };
        let journal = C3SlotJournal::open(journal_path, &service_id).unwrap();
        let _lock = journal.lock_exclusive().unwrap();
        let mut ready = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(ready_path)
            .unwrap();
        ready.write_all(b"locked").unwrap();
        ready.sync_all().unwrap();
        let release_path = std::path::PathBuf::from(release_path);
        let deadline = Instant::now() + Duration::from_secs(8);
        while !release_path.exists() {
            assert!(
                Instant::now() < deadline,
                "parent did not release journal lock"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}
