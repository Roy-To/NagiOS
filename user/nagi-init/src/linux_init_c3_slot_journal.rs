//! C3-private Linux file journal for exact system-service slot transitions.
//!
//! Each operation is written as a durable intent before its effect callback
//! runs, then as a durable terminal row after the callback succeeds. An
//! incomplete or unreadable journal is `Unknown` and cannot run another
//! effect. The journal is append-only so opaque operation IDs can be checked
//! for exact replay after a process restart.

use crate::c3_system_service_recovery::RecoveryScopeSnapshot;
use nagi_system_service_recovery_contract::{
    C1RecoveryOperationCorrelation, RecoveryLifecycleTransition,
};
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
const JOURNAL_VERSION: u8 = 5;
const LEGACY_JOURNAL_VERSION_V3: u8 = 3;
const LEGACY_JOURNAL_VERSION_V4: u8 = 4;
const FIRST_PROVISION_MARKER_MAGIC: &[u8] = b"NAGI-C3-SLOT-FIRST-PROVISION-V1\n";
const HEADER_LEN: usize = 85;
const V5_LAUNCH_EVIDENCE_LEN: usize = 17;
const V5_EXTENSION_HEADER_LEN: usize = V5_LAUNCH_EVIDENCE_LEN + 4;
const MAX_SCOPE_FIELD_LEN: usize = 4096;
const MAX_SCOPE_SNAPSHOT_LEN: usize = 16 * 1024;
const CHECKSUM_LEN: usize = 32;
const MAX_SERVICE_ID_LEN: usize = 128;
const LOCK_EX: i32 = 2;
const LOCK_NB: i32 = 4;
const LOCK_UN: i32 = 8;
const DEFAULT_JOURNAL_LOCK_WAIT: Duration = Duration::from_secs(2);
const JOURNAL_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(2);
const O_CLOEXEC_LINUX: i32 = 0o2000000;
const O_DIRECTORY_LINUX: i32 = 0o200000;
const O_NONBLOCK_LINUX: i32 = 0o4000;
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
pub(super) enum LaunchEvidenceState {
    Running,
    NormalExit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DurableLaunchEvidence {
    launch_operation_id: u64,
    process_id: u32,
    state: LaunchEvidenceState,
}

impl DurableLaunchEvidence {
    pub(super) fn launch_operation_id(&self) -> NonZeroU64 {
        NonZeroU64::new(self.launch_operation_id).expect("validated durable launch operation ID")
    }

    pub(super) fn process_id(&self) -> u32 {
        self.process_id
    }

    pub(super) fn is_normal_exit(&self) -> bool {
        self.state == LaunchEvidenceState::NormalExit
    }

    fn is_valid_for(&self, row: &JournalRow) -> bool {
        self.launch_operation_id != 0
            && self.process_id != 0
            && match self.state {
                LaunchEvidenceState::Running => {
                    row.state == C3SlotState::Live && self.launch_operation_id <= row.operation_id
                }
                LaunchEvidenceState::NormalExit => {
                    row.state == C3SlotState::ClearedAndAbsent
                        && row.c1_operation_id != 0
                        && self.launch_operation_id < row.operation_id
                }
            }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum C3SlotJournalError {
    InvalidInput,
    /// C3 confirmed that this journal operation rejected input before any
    /// filesystem write began.
    RejectedBeforeWrite,
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
    launch_evidence: Option<DurableLaunchEvidence>,
    scope_snapshot: Option<RecoveryScopeSnapshot>,
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
            launch_evidence: None,
            scope_snapshot: None,
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

/// Read-only evidence for one historical, committed exact normal child exit.
/// This proves a journaled C3 observation, not that the old process is live.
#[derive(Clone, Eq, PartialEq)]
pub(super) struct C3DurableNormalExitReceipt {
    service_id: String,
    catalog_generation: NonZeroU64,
    c3_launch_generation: NonZeroU64,
    scope_binding_digest: [u8; 32],
    scope_snapshot: RecoveryScopeSnapshot,
    launch_operation_id: NonZeroU64,
    process_id: NonZeroU64,
    c1_operation_id: NonZeroU64,
    terminal_revision: NonZeroU64,
    terminal_operation_id: NonZeroU64,
}

impl C3DurableNormalExitReceipt {
    fn from_committed_record(record: &JournalRecord) -> Result<Self, C3SlotJournalError> {
        let evidence = record
            .row
            .launch_evidence
            .filter(|evidence| evidence.state == LaunchEvidenceState::NormalExit)
            .ok_or(C3SlotJournalError::Unknown)?;
        if record.format_version != JOURNAL_VERSION
            || record.kind != RecordKind::Committed
            || record.row.state != C3SlotState::ClearedAndAbsent
            || !evidence.is_valid_for(&record.row)
        {
            return Err(C3SlotJournalError::Unknown);
        }
        Ok(Self {
            service_id: record.row.service_id.clone(),
            catalog_generation: NonZeroU64::new(record.row.catalog_generation)
                .ok_or(C3SlotJournalError::Unknown)?,
            c3_launch_generation: NonZeroU64::new(record.row.c3_launch_generation)
                .ok_or(C3SlotJournalError::Unknown)?,
            scope_binding_digest: record.row.scope_binding_digest,
            scope_snapshot: record
                .row
                .scope_snapshot
                .clone()
                .ok_or(C3SlotJournalError::Unknown)?,
            launch_operation_id: NonZeroU64::new(evidence.launch_operation_id)
                .ok_or(C3SlotJournalError::Unknown)?,
            process_id: NonZeroU64::new(u64::from(evidence.process_id))
                .ok_or(C3SlotJournalError::Unknown)?,
            c1_operation_id: NonZeroU64::new(record.row.c1_operation_id)
                .ok_or(C3SlotJournalError::Unknown)?,
            terminal_revision: NonZeroU64::new(record.revision)
                .ok_or(C3SlotJournalError::Unknown)?,
            terminal_operation_id: NonZeroU64::new(record.row.operation_id)
                .ok_or(C3SlotJournalError::Unknown)?,
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

    pub(super) fn scope_snapshot(&self) -> &RecoveryScopeSnapshot {
        &self.scope_snapshot
    }

    pub(super) fn launch_operation_id(&self) -> NonZeroU64 {
        self.launch_operation_id
    }

    /// Stable C3-issued launch-instance key. Unlike SpawnedProcessIdentity's
    /// in-memory nonce, this operation ID survives a cold journal reopen.
    pub(super) fn launch_instance_id(&self) -> NonZeroU64 {
        self.launch_operation_id
    }

    pub(super) fn process_id(&self) -> NonZeroU64 {
        self.process_id
    }

    pub(super) fn c1_operation_id(&self) -> NonZeroU64 {
        self.c1_operation_id
    }

    pub(super) fn terminal_revision(&self) -> NonZeroU64 {
        self.terminal_revision
    }

    pub(super) fn terminal_operation_id(&self) -> NonZeroU64 {
        self.terminal_operation_id
    }
}

pub(super) struct C3SlotJournal {
    path: PathBuf,
    lock_path: PathBuf,
    first_provision_marker_path: PathBuf,
    // Keep the selected directory object alive and address children through
    // this descriptor, so replacing its display pathname cannot redirect C3
    // journal operations to another Data directory.
    _parent_directory: File,
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

    #[cfg(any(test, not(nagi_release_build)))]
    pub(super) fn create(
        path: impl AsRef<Path>,
        service_id: &str,
    ) -> Result<Self, C3SlotJournalError> {
        let journal = Self::from_path(path, service_id)?;
        let lock_file =
            create_new_private_file(&journal.lock_path).map_err(|_| C3SlotJournalError::Unknown)?;
        let result = (|| {
            validate_private_file_after_write(&lock_file)?;
            lock_file
                .sync_all()
                .map_err(|_| C3SlotJournalError::Unknown)?;
            journal
                ._parent_directory
                .sync_all()
                .map_err(|_| C3SlotJournalError::Unknown)?;

            let mut file =
                create_new_private_file(&journal.path).map_err(|_| C3SlotJournalError::Unknown)?;
            validate_private_file_after_write(&file)?;
            file.write_all(
                &encode_record(&genesis_record(service_id))
                    .map_err(|_| C3SlotJournalError::Unknown)?,
            )
            .map_err(|_| C3SlotJournalError::Unknown)?;
            file.sync_all().map_err(|_| C3SlotJournalError::Unknown)?;
            journal
                ._parent_directory
                .sync_all()
                .map_err(|_| C3SlotJournalError::Unknown)?;
            journal.verify_initialized()?;
            Ok(journal)
        })();
        normalize_after_first_write(result)
    }

    /// Create a genesis-only journal exactly once for an explicitly marked
    /// development image. Existing journal or lock entries are never opened,
    /// replaced, or repaired through this path.
    #[cfg(not(nagi_release_build))]
    pub(super) fn create_from_first_provision_marker(
        path: impl AsRef<Path>,
        service_id: &str,
    ) -> Result<Option<Self>, C3SlotJournalError> {
        let journal = Self::from_path(path, service_id)?;
        let mut marker = match open_existing_private_marker(&journal.first_provision_marker_path) {
            Ok(marker) => marker,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(C3SlotJournalError::Unknown),
        };
        validate_private_file_before_write(&marker)?;
        let mut marker_bytes = Vec::new();
        Read::take(
            &mut marker,
            (MAX_SERVICE_ID_LEN + FIRST_PROVISION_MARKER_MAGIC.len() + 2) as u64,
        )
        .read_to_end(&mut marker_bytes)
        .map_err(|_| C3SlotJournalError::Unknown)?;
        if marker_bytes != first_provision_marker_bytes(service_id)? {
            return Err(C3SlotJournalError::RejectedBeforeWrite);
        }

        let journal_exists = path_exists(&journal.path)?;
        let lock_exists = path_exists(&journal.lock_path)?;
        let initialized = match (journal_exists, lock_exists) {
            (false, false) => Self::create(&journal.path, service_id)?,
            (true, true) => journal,
            // A single file or any unclassified filesystem result is partial
            // state. Preserve the marker and both paths for explicit recovery.
            (false, true) | (true, false) => return Err(C3SlotJournalError::Unknown),
        };
        // Once a journal pair exists, every failure in marker validation,
        // sync, or consumption is post-write. Do not let a late pre-write
        // classification release the PID-1 slot reservation.
        normalize_after_first_write(
            initialized
                .consume_first_provision_marker_if_genesis(&marker)
                .map(|()| Some(initialized)),
        )
    }

    fn consume_first_provision_marker_if_genesis(
        &self,
        marker: &File,
    ) -> Result<(), C3SlotJournalError> {
        let _lock = self.lock_exclusive()?;
        let mut journal_file = open_existing_private_file(&self.path, false)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        validate_private_file_before_write(&journal_file)?;
        let scan = scan_journal_file(&mut journal_file, &self.service_id, None)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        if !scan_is_exact_genesis(&scan, &self.service_id) {
            return Err(C3SlotJournalError::Unknown);
        }

        validate_private_file(marker).map_err(|_| C3SlotJournalError::Rejected)?;
        let marker_descriptor_metadata =
            marker.metadata().map_err(|_| C3SlotJournalError::Unknown)?;
        let marker_path_metadata = fs::symlink_metadata(&self.first_provision_marker_path)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        if !marker_path_metadata.file_type().is_file()
            || marker_path_metadata.dev() != marker_descriptor_metadata.dev()
            || marker_path_metadata.ino() != marker_descriptor_metadata.ino()
        {
            return Err(C3SlotJournalError::Unknown);
        }
        journal_file
            .sync_all()
            .map_err(|_| C3SlotJournalError::Unknown)?;
        marker.sync_all().map_err(|_| C3SlotJournalError::Unknown)?;
        self._parent_directory
            .sync_all()
            .map_err(|_| C3SlotJournalError::Unknown)?;
        fs::remove_file(&self.first_provision_marker_path)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        self._parent_directory
            .sync_all()
            .map_err(|_| C3SlotJournalError::Unknown)?;
        Ok(())
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
        let parent_directory = OpenOptions::new()
            .read(true)
            .custom_flags(O_DIRECTORY_LINUX | O_CLOEXEC_LINUX)
            .open(requested_parent)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        validate_parent_before_write(&parent_directory)?;
        let parent = PathBuf::from(format!("/proc/self/fd/{}", parent_directory.as_raw_fd()));

        let path = parent.join(file_name);
        let mut lock_name: OsString = file_name.to_os_string();
        lock_name.push(".lock");
        let mut first_provision_marker_name = file_name.to_os_string();
        first_provision_marker_name.push(".first-provision");
        Ok(Self {
            path,
            lock_path: parent.join(lock_name),
            first_provision_marker_path: parent.join(first_provision_marker_name),
            _parent_directory: parent_directory,
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

    pub(super) fn same_selected_location(&self, other: &Self) -> io::Result<bool> {
        if self.service_id != other.service_id || self.path.file_name() != other.path.file_name() {
            return Ok(false);
        }
        let self_parent = self._parent_directory.metadata()?;
        let other_parent = other._parent_directory.metadata()?;
        Ok(self_parent.dev() == other_parent.dev() && self_parent.ino() == other_parent.ino())
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
            format_version: JOURNAL_VERSION,
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
                launch_evidence: None,
                scope_snapshot: None,
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
        scope_snapshot: RecoveryScopeSnapshot,
        effect: impl FnOnce(&mut dyn FnMut(u32) -> io::Result<()>) -> io::Result<u32>,
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
            launch_evidence: None,
            scope_snapshot: None,
        };
        self.transition_scanned_with_launch_identity(&mut scan, transition, scope_snapshot, effect)
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
        self.transition_next_locked(
            catalog_generation,
            c3_launch_generation,
            scope_binding_digest,
            state,
            None,
            effect,
        )
    }

    pub(super) fn transition_next_until(
        &self,
        catalog_generation: NonZeroU64,
        c3_launch_generation: NonZeroU64,
        scope_binding_digest: [u8; 32],
        state: C3SlotState,
        deadline: Instant,
        effect: impl FnOnce() -> io::Result<()>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        let _lock = self.lock_exclusive_until(deadline)?;
        ensure_cleanup_deadline_not_expired(Some(deadline))?;
        self.transition_next_locked(
            catalog_generation,
            c3_launch_generation,
            scope_binding_digest,
            state,
            Some(deadline),
            effect,
        )
    }

    fn transition_next_locked(
        &self,
        catalog_generation: NonZeroU64,
        c3_launch_generation: NonZeroU64,
        scope_binding_digest: [u8; 32],
        state: C3SlotState,
        cleanup_deadline: Option<Instant>,
        effect: impl FnOnce() -> io::Result<()>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        ensure_cleanup_deadline_not_expired(cleanup_deadline)?;
        let scan_result = scan_journal(&self.path, &self.service_id, None);
        ensure_cleanup_deadline_not_expired(cleanup_deadline)?;
        let mut scan = scan_result.map_err(|_| C3SlotJournalError::Unknown)?;
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
        self.transition_scanned_with_deadline(
            &mut scan,
            transition,
            cleanup_deadline,
            &Instant::now,
            effect,
        )
    }

    /// Commit normal-exit evidence only for the exact previously committed
    /// live launch key and C1 operation correlation. The caller's effect runs
    /// after the intent has been synced and must recheck its post-reap state.
    pub(super) fn record_normal_exit_after_reap(
        &self,
        launch: C3ReapedLaunchKey,
        correlation: C1RecoveryOperationCorrelation,
        cleanup_deadline: Option<Instant>,
        effect: impl FnOnce() -> io::Result<()>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        self.record_normal_exit_after_reap_with_clock(
            launch,
            correlation,
            cleanup_deadline,
            &Instant::now,
            effect,
        )
    }

    fn record_normal_exit_after_reap_with_clock(
        &self,
        launch: C3ReapedLaunchKey,
        correlation: C1RecoveryOperationCorrelation,
        cleanup_deadline: Option<Instant>,
        now: &dyn Fn() -> Instant,
        effect: impl FnOnce() -> io::Result<()>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
        let C3ReapedLaunchKey {
            catalog_generation,
            c3_launch_generation,
            scope_binding_digest,
            launch_operation_id,
            process_id,
        } = launch;
        let valid_input = process_id != 0 && correlation.binding_digest() == &scope_binding_digest;
        if !valid_input {
            ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
            return Err(C3SlotJournalError::Rejected);
        }
        let lock_result = match cleanup_deadline {
            Some(deadline) => self.lock_exclusive_until(deadline),
            None => self.lock_exclusive(),
        };
        let _lock = match lock_result {
            Ok(lock) => lock,
            Err(error) => {
                ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
                return Err(error);
            }
        };
        ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
        let scan_result = scan_journal(&self.path, &self.service_id, None);
        ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
        let mut scan = scan_result.map_err(|_| C3SlotJournalError::Unknown)?;
        if scan.pending.is_some() || scan.reservation.is_some() {
            return Err(C3SlotJournalError::Unknown);
        }
        let current = scan.current.as_ref().ok_or(C3SlotJournalError::Unknown)?;
        let expected_evidence = DurableLaunchEvidence {
            launch_operation_id: launch_operation_id.get(),
            process_id,
            state: LaunchEvidenceState::Running,
        };
        let matching_live_launch = current.kind == RecordKind::Committed
            && current.row.state == C3SlotState::Live
            && current.row.service_id == self.service_id
            && current.row.catalog_generation == catalog_generation.get()
            && current.row.c3_launch_generation == c3_launch_generation.get()
            && current.row.scope_binding_digest == scope_binding_digest
            && current.row.c1_operation_id == correlation.operation_id().get()
            && current.row.launch_evidence == Some(expected_evidence);
        ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
        if !matching_live_launch {
            ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
            return Err(C3SlotJournalError::Rejected);
        }
        let operation_id = scan
            .highest_operation_id
            .checked_add(1)
            .and_then(NonZeroU64::new)
            .ok_or(C3SlotJournalError::Unknown)?;
        let mut transition = match C3SlotTransition::new(
            &self.service_id,
            catalog_generation,
            c3_launch_generation,
            scope_binding_digest,
            operation_id,
            C3SlotState::ClearedAndAbsent,
        ) {
            Ok(transition) => transition,
            Err(error) => {
                ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
                return Err(error);
            }
        };
        transition.c1_operation_id = Some(correlation.operation_id());
        transition.launch_evidence = Some(DurableLaunchEvidence {
            state: LaunchEvidenceState::NormalExit,
            ..expected_evidence
        });
        transition.scope_snapshot = current.row.scope_snapshot.clone();
        let receipt_result = self.transition_scanned_with_deadline(
            &mut scan,
            transition,
            cleanup_deadline,
            now,
            effect,
        );
        let receipt = match receipt_result {
            Ok(receipt) => receipt,
            Err(error) => {
                ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
                return Err(error);
            }
        };
        ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
        if receipt.state() != C3SlotState::ClearedAndAbsent
            || receipt.c1_operation_id() != Some(correlation.operation_id())
            || receipt.scope_binding_digest() != &scope_binding_digest
        {
            return Err(C3SlotJournalError::Unknown);
        }
        ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
        Ok(receipt)
    }

    /// Return the latest fully committed normal-exit scope snapshot, or None
    /// only for a validated genesis-only journal with no prior slot row.
    pub(super) fn latest_normal_exit_scope_snapshot(
        &self,
    ) -> Result<Option<C3DurableNormalExitReceipt>, C3SlotJournalError> {
        self.latest_normal_exit_scope_snapshot_with_deadline(None)
    }

    pub(super) fn latest_normal_exit_scope_snapshot_until(
        &self,
        deadline: Instant,
    ) -> Result<Option<C3DurableNormalExitReceipt>, C3SlotJournalError> {
        self.latest_normal_exit_scope_snapshot_with_deadline(Some(deadline))
    }

    fn latest_normal_exit_scope_snapshot_with_deadline(
        &self,
        deadline: Option<Instant>,
    ) -> Result<Option<C3DurableNormalExitReceipt>, C3SlotJournalError> {
        let _lock = match deadline {
            Some(deadline) => self.lock_exclusive_until(deadline)?,
            None => self.lock_exclusive()?,
        };
        ensure_cleanup_deadline_not_expired(deadline)?;
        let scan = scan_journal(&self.path, &self.service_id, None)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        ensure_cleanup_deadline_not_expired(deadline)?;
        if scan.pending.is_some() || scan.reservation.is_some() {
            return Err(C3SlotJournalError::Unknown);
        }
        let receipt = match scan.current.as_ref() {
            None if scan.last_revision == 1
                && scan.highest_operation_id == 0
                && scan.highest_c3_launch_generation == 0 =>
            {
                None
            }
            Some(record)
                if record.kind == RecordKind::Committed
                    && record.row.state == C3SlotState::ClearedAndAbsent
                    && record.row.launch_evidence.is_some_and(|evidence| {
                        evidence.state == LaunchEvidenceState::NormalExit
                    }) =>
            {
                Some(C3DurableNormalExitReceipt::from_committed_record(record)?)
            }
            Some(_) | None => return Err(C3SlotJournalError::Unknown),
        };
        ensure_cleanup_deadline_not_expired(deadline)?;
        Ok(receipt)
    }

    /// Re-read and select one exact old terminal receipt from the complete
    /// journal. No current-boot process record participates in this lookup.
    pub(super) fn read_normal_exit_receipt(
        &self,
        scope: &RecoveryScopeSnapshot,
        launch_operation_id: NonZeroU64,
        process_id: u32,
        correlation: C1RecoveryOperationCorrelation,
    ) -> Result<C3DurableNormalExitReceipt, C3SlotJournalError> {
        self.read_normal_exit_receipt_with_deadline(
            scope,
            launch_operation_id,
            process_id,
            correlation,
            None,
        )
    }

    pub(super) fn read_normal_exit_receipt_until(
        &self,
        scope: &RecoveryScopeSnapshot,
        launch_operation_id: NonZeroU64,
        process_id: u32,
        correlation: C1RecoveryOperationCorrelation,
        deadline: Instant,
    ) -> Result<C3DurableNormalExitReceipt, C3SlotJournalError> {
        self.read_normal_exit_receipt_with_deadline(
            scope,
            launch_operation_id,
            process_id,
            correlation,
            Some(deadline),
        )
    }

    fn read_normal_exit_receipt_with_deadline(
        &self,
        scope: &RecoveryScopeSnapshot,
        launch_operation_id: NonZeroU64,
        process_id: u32,
        correlation: C1RecoveryOperationCorrelation,
        deadline: Option<Instant>,
    ) -> Result<C3DurableNormalExitReceipt, C3SlotJournalError> {
        if process_id == 0
            || correlation.binding_digest() != &scope.binding_digest
            || scope.canonical_digest() != scope.binding_digest
        {
            return Err(C3SlotJournalError::Rejected);
        }
        let _lock = match deadline {
            Some(deadline) => self.lock_exclusive_until(deadline)?,
            None => self.lock_exclusive()?,
        };
        ensure_cleanup_deadline_not_expired(deadline)?;
        let scan = scan_journal(&self.path, &self.service_id, None)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        ensure_cleanup_deadline_not_expired(deadline)?;
        if scan.pending.is_some() || scan.reservation.is_some() {
            return Err(C3SlotJournalError::Unknown);
        }
        let mut matches = scan.normal_exit_records.iter().filter(|record| {
            let row = &record.row;
            row.service_id == self.service_id
                && row.catalog_generation == scope.catalog_generation.get()
                && row.c3_launch_generation == scope.c3_launch_generation.get()
                && row.scope_binding_digest == scope.binding_digest
                && row.scope_snapshot.as_ref() == Some(scope)
                && row.c1_operation_id == correlation.operation_id().get()
                && row.launch_evidence.is_some_and(|evidence| {
                    evidence.launch_operation_id == launch_operation_id.get()
                        && evidence.process_id == process_id
                        && evidence.state == LaunchEvidenceState::NormalExit
                })
        });
        let record = matches.next().ok_or(C3SlotJournalError::Unknown)?;
        if matches.next().is_some() {
            return Err(C3SlotJournalError::Unknown);
        }
        let receipt = C3DurableNormalExitReceipt::from_committed_record(record)?;
        ensure_cleanup_deadline_not_expired(deadline)?;
        Ok(receipt)
    }

    fn transition_scanned(
        &self,
        scan: &mut JournalScan,
        transition: C3SlotTransition,
        effect: impl FnOnce() -> io::Result<()>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        self.transition_scanned_with_deadline(scan, transition, None, &Instant::now, effect)
    }

    fn transition_scanned_with_deadline(
        &self,
        scan: &mut JournalScan,
        transition: C3SlotTransition,
        cleanup_deadline: Option<Instant>,
        now: &dyn Fn() -> Instant,
        effect: impl FnOnce() -> io::Result<()>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        self.transition_scanned_with_identity_result(
            scan,
            transition,
            None,
            cleanup_deadline,
            now,
            |_| effect().map(|()| None),
        )
    }

    fn transition_scanned_with_launch_identity(
        &self,
        scan: &mut JournalScan,
        transition: C3SlotTransition,
        scope_snapshot: RecoveryScopeSnapshot,
        effect: impl FnOnce(&mut dyn FnMut(u32) -> io::Result<()>) -> io::Result<u32>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        self.transition_scanned_with_identity_result(
            scan,
            transition,
            Some(scope_snapshot),
            None,
            &Instant::now,
            |stage_launch_identity| effect(stage_launch_identity).map(Some),
        )
    }

    fn transition_scanned_with_identity_result(
        &self,
        scan: &mut JournalScan,
        transition: C3SlotTransition,
        launch_scope_snapshot: Option<RecoveryScopeSnapshot>,
        cleanup_deadline: Option<Instant>,
        now: &dyn Fn() -> Instant,
        effect: impl FnOnce(&mut dyn FnMut(u32) -> io::Result<()>) -> io::Result<Option<u32>>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
        if scan.pending.is_some() {
            return Err(C3SlotJournalError::Unknown);
        }
        if scan.requested_operation_seen {
            return Err(C3SlotJournalError::Replay);
        }
        let prior_committed_row = scan.current.clone();

        // Preserve the identity metadata only when updating the same Live
        // generation (for example, binding its exact C1 correlation). A new
        // launch consumes a reservation at a higher generation and must not
        // inherit the previous generation's terminal receipt.
        let preserve_live_identity = transition.state == C3SlotState::Live
            && scan.current.as_ref().is_some_and(|current| {
                current.row.state == C3SlotState::Live
                    && current.row.service_id == transition.service_id
                    && current.row.catalog_generation == transition.catalog_generation.get()
                    && current.row.c3_launch_generation == transition.c3_launch_generation.get()
                    && current.row.scope_binding_digest == transition.scope_binding_digest
            });
        let row = JournalRow {
            service_id: transition.service_id,
            catalog_generation: transition.catalog_generation.get(),
            c3_launch_generation: transition.c3_launch_generation.get(),
            scope_binding_digest: transition.scope_binding_digest,
            operation_id: transition.operation_id.get(),
            c1_operation_id: transition.c1_operation_id.map_or(0, NonZeroU64::get),
            state: transition.state,
            launch_evidence: transition.launch_evidence.or_else(|| {
                if preserve_live_identity {
                    scan.current
                        .as_ref()
                        .and_then(|current| current.row.launch_evidence)
                } else {
                    None
                }
            }),
            scope_snapshot: transition.scope_snapshot.or_else(|| {
                if preserve_live_identity {
                    scan.current
                        .as_ref()
                        .and_then(|current| current.row.scope_snapshot.clone())
                } else {
                    None
                }
            }),
        };
        let reserved_transition = scan.reservation.as_ref().is_some_and(|reservation| {
            reservation.row.operation_id == row.operation_id
                && reservation.row.c3_launch_generation == row.c3_launch_generation
                && matches!(row.state, C3SlotState::Live | C3SlotState::Quarantined)
                && row.c1_operation_id == 0
                && row.launch_evidence.is_none()
                && row.scope_snapshot.is_none()
                && row.catalog_generation >= scan.highest_catalog_generation
                && scan
                    .current
                    .as_ref()
                    .is_none_or(|current| current.row.state != C3SlotState::Quarantined)
        });
        let is_valid = if reserved_transition {
            true
        } else if row.state == C3SlotState::Live && row.c1_operation_id == 0 {
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
        if !is_valid
            || row.launch_evidence.is_some() != row.scope_snapshot.is_some()
            || row
                .scope_snapshot
                .as_ref()
                .is_some_and(|scope| !scope_snapshot_matches_row(scope, &row))
        {
            ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
            return Err(C3SlotJournalError::Rejected);
        }

        let intent_revision = scan
            .last_revision
            .checked_add(1)
            .filter(|revision| *revision != 0)
            .ok_or(C3SlotJournalError::Unknown)?;
        let intent = JournalRecord {
            format_version: JOURNAL_VERSION,
            kind: RecordKind::Intent,
            revision: intent_revision,
            row,
        };
        ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
        *scan = self
            .append_and_readback_with_deadline(
                &intent,
                Some(NonZeroU64::new(intent.row.operation_id).ok_or(C3SlotJournalError::Unknown)?),
                cleanup_deadline,
                now,
            )
            .map_err(|_| C3SlotJournalError::Unknown)?;
        ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
        if scan.pending.as_ref() != Some(&intent) {
            return Err(C3SlotJournalError::Unknown);
        }

        let mut staged_process_id = None;
        let mut stage_launch_identity = |process_id: u32| -> io::Result<()> {
            if staged_process_id.is_some()
                || process_id == 0
                || !reserved_transition
                || launch_scope_snapshot.is_none()
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "C3 launch identity cannot be staged for this transition",
                ));
            }
            let pending = scan
                .pending
                .as_ref()
                .filter(|pending| pending.row == intent.row)
                .ok_or_else(|| io::Error::other("C3 launch intent is no longer pending"))?;
            let mut bound_row = pending.row.clone();
            let scope_snapshot = launch_scope_snapshot
                .as_ref()
                .ok_or_else(|| io::Error::other("C3 launch scope snapshot is missing"))?;
            bound_row.launch_evidence = Some(DurableLaunchEvidence {
                launch_operation_id: bound_row.operation_id,
                process_id,
                state: LaunchEvidenceState::Running,
            });
            bound_row.scope_snapshot = Some(scope_snapshot.clone());
            if !scope_snapshot_matches_row(scope_snapshot, &bound_row) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "C3 launch scope snapshot does not match its reserved row",
                ));
            }
            let staged = JournalRecord {
                format_version: JOURNAL_VERSION,
                kind: RecordKind::Intent,
                revision: pending
                    .revision
                    .checked_add(1)
                    .filter(|revision| *revision != 0)
                    .ok_or_else(|| io::Error::other("C3 journal revision overflow"))?,
                row: bound_row,
            };
            *scan = self
                .append_and_readback_with_deadline(
                    &staged,
                    NonZeroU64::new(staged.row.operation_id),
                    cleanup_deadline,
                    now,
                )
                .map_err(|_| io::Error::other("C3 staged launch identity readback failed"))?;
            ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now).map_err(|_| {
                io::Error::other("C3 cleanup deadline expired after intent readback")
            })?;
            if scan.pending.as_ref() != Some(&staged)
                || !scan.pending_reserved_launch
                || scan.current.as_ref() != prior_committed_row.as_ref()
            {
                return Err(io::Error::other(
                    "C3 staged launch identity is not authoritative",
                ));
            }
            staged_process_id = Some(process_id);
            Ok(())
        };
        let process_id = match catch_unwind(AssertUnwindSafe(|| effect(&mut stage_launch_identity)))
        {
            Ok(Ok(process_id)) => process_id,
            Ok(Err(_)) | Err(_) => return Err(C3SlotJournalError::Unknown),
        };
        ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;

        let completed_row = scan
            .pending
            .as_ref()
            .ok_or(C3SlotJournalError::Unknown)?
            .row
            .clone();
        if let Some(process_id) = process_id {
            if staged_process_id != Some(process_id)
                || completed_row
                    .launch_evidence
                    .is_none_or(|evidence| evidence.process_id != process_id)
            {
                return Err(C3SlotJournalError::Unknown);
            }
        } else if staged_process_id.is_some() || launch_scope_snapshot.is_some() {
            return Err(C3SlotJournalError::Unknown);
        }
        let completed = JournalRecord {
            format_version: JOURNAL_VERSION,
            kind: RecordKind::Committed,
            revision: scan
                .last_revision
                .checked_add(1)
                .filter(|revision| *revision != 0)
                .ok_or(C3SlotJournalError::Unknown)?,
            row: completed_row,
        };
        ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
        *scan = self
            .append_and_readback_with_deadline(
                &completed,
                NonZeroU64::new(completed.row.operation_id),
                cleanup_deadline,
                now,
            )
            .map_err(|_| C3SlotJournalError::Unknown)?;
        ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now)?;
        if scan.pending.is_some() || scan.current.as_ref() != Some(&completed) {
            return Err(C3SlotJournalError::Unknown);
        }
        completed_readback_receipt_before_deadline(
            scan.current.as_ref().ok_or(C3SlotJournalError::Unknown)?,
            cleanup_deadline,
            now,
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
        self.readback_from_scan(scan)
    }

    pub(super) fn readback_until(&self, deadline: Instant) -> C3SlotJournalReadback {
        let Ok(_lock) = self.lock_exclusive_until(deadline) else {
            return C3SlotJournalReadback::Unknown;
        };
        if Instant::now() >= deadline {
            return C3SlotJournalReadback::Unknown;
        }
        let Ok(scan) = scan_journal(&self.path, &self.service_id, None) else {
            return C3SlotJournalReadback::Unknown;
        };
        if Instant::now() >= deadline {
            return C3SlotJournalReadback::Unknown;
        }
        self.readback_from_scan(scan)
    }

    fn readback_from_scan(&self, scan: JournalScan) -> C3SlotJournalReadback {
        if scan.pending.is_some() || scan.reservation.is_some() {
            return C3SlotJournalReadback::Unknown;
        }
        match scan.current {
            Some(record) => C3SlotJournalReadback::Committed(Box::new(C3DurableSlotRowReadback {
                service_id: record.row.service_id,
                catalog_generation: record.row.catalog_generation,
                c3_launch_generation: record.row.c3_launch_generation,
                scope_binding_digest: record.row.scope_binding_digest,
                durable_revision: record.revision,
                operation_id: record.row.operation_id,
                c1_operation_id: record.row.c1_operation_id,
                state: record.row.state,
                launch_evidence: record.row.launch_evidence,
                scope_snapshot: record.row.scope_snapshot,
                highest_catalog_generation: scan.highest_catalog_generation,
                highest_c3_launch_generation: scan.highest_c3_launch_generation,
                highest_operation_id: scan.highest_operation_id,
            })),
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

    /// Persist the exact C1 operation correlation before its first live-slot
    /// effect, then return only after the same Live binding and correlation
    /// have been committed and read back. The C3 operation sequence remains
    /// independent from C1's operation ID.
    pub(super) fn with_c1_correlated_live_receipt<R>(
        &self,
        catalog_generation: NonZeroU64,
        c3_launch_generation: NonZeroU64,
        scope_binding_digest: &[u8; 32],
        correlation: C1RecoveryOperationCorrelation,
        effect: impl FnOnce() -> io::Result<R>,
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
            || before.row.state != C3SlotState::Live
        {
            return Err(C3SlotJournalError::Rejected);
        }
        if before.row.c1_operation_id == correlation.operation_id().get() {
            return Err(C3SlotJournalError::Replay);
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
            C3SlotState::Live,
        )?;
        transition.c1_operation_id = Some(correlation.operation_id());
        let mut result = None;
        let receipt = self.transition_scanned(&mut scan, transition, || {
            result = Some(effect()?);
            Ok(())
        })?;
        if receipt.c1_operation_id() != Some(correlation.operation_id())
            || receipt.scope_binding_digest() != scope_binding_digest
            || receipt.state() != C3SlotState::Live
        {
            return Err(C3SlotJournalError::Unknown);
        }
        let result = result.ok_or(C3SlotJournalError::Unknown)?;
        Ok((result, receipt))
    }

    /// Continue a C1 runtime operation whose initial recovery already committed
    /// the exact correlated Live row. The bootstrap entry remains one-shot;
    /// this separate entry requires the current row to carry the same C1 ID,
    /// slot generations, and scope digest before it writes a fresh C3-local
    /// intent for the continuation callback.
    pub(super) fn with_c1_correlated_live_continuation_receipt<R>(
        &self,
        catalog_generation: NonZeroU64,
        c3_launch_generation: NonZeroU64,
        scope_binding_digest: &[u8; 32],
        correlation: C1RecoveryOperationCorrelation,
        effect: impl FnOnce() -> io::Result<R>,
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
            || before.row.state != C3SlotState::Live
            || before.row.c1_operation_id != correlation.operation_id().get()
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
            C3SlotState::Live,
        )?;
        transition.c1_operation_id = Some(correlation.operation_id());
        let mut result = None;
        let receipt = self.transition_scanned(&mut scan, transition, || {
            result = Some(effect()?);
            Ok(())
        })?;
        if receipt.c1_operation_id() != Some(correlation.operation_id())
            || receipt.scope_binding_digest() != scope_binding_digest
            || receipt.state() != C3SlotState::Live
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

    fn lock_exclusive_until(
        &self,
        deadline: Instant,
    ) -> Result<ExclusiveFileLock, C3SlotJournalError> {
        ensure_cleanup_deadline_not_expired(Some(deadline))?;
        let file = open_existing_private_file(&self.lock_path, false)
            .map_err(|_| C3SlotJournalError::Unknown)?;
        let validation = validate_private_file(&file);
        ensure_cleanup_deadline_not_expired(Some(deadline))?;
        validation.map_err(|_| C3SlotJournalError::Rejected)?;
        ExclusiveFileLock::acquire_until(file, deadline).map_err(|_| C3SlotJournalError::Unknown)
    }

    fn append_and_readback(
        &self,
        record: &JournalRecord,
        requested_operation_id: Option<NonZeroU64>,
    ) -> Result<JournalScan, ()> {
        self.append_and_readback_with_deadline(record, requested_operation_id, None, &Instant::now)
    }

    fn append_and_readback_with_deadline(
        &self,
        record: &JournalRecord,
        requested_operation_id: Option<NonZeroU64>,
        cleanup_deadline: Option<Instant>,
        now: &dyn Fn() -> Instant,
    ) -> Result<JournalScan, ()> {
        if ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now).is_err() {
            return Err(());
        }
        let mut file = open_existing_private_file(&self.path, true).map_err(|_| ())?;
        validate_private_file(&file).map_err(|_| ())?;
        if ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now).is_err() {
            return Err(());
        }
        let encoded = encode_record(record)?;
        if ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now).is_err() {
            return Err(());
        }
        file.write_all(&encoded).map_err(|_| ())?;
        file.sync_all().map_err(|_| ())?;
        if ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now).is_err() {
            return Err(());
        }
        drop(file);

        let scan = scan_journal(&self.path, &self.service_id, requested_operation_id)?;
        if ensure_cleanup_deadline_not_expired_at(cleanup_deadline, now).is_err() {
            return Err(());
        }
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

fn first_provision_marker_bytes(service_id: &str) -> Result<Vec<u8>, C3SlotJournalError> {
    if !canonical_service_id(service_id) {
        return Err(C3SlotJournalError::InvalidInput);
    }
    let mut bytes = FIRST_PROVISION_MARKER_MAGIC.to_vec();
    bytes.extend_from_slice(service_id.as_bytes());
    bytes.push(b'\n');
    Ok(bytes)
}

fn path_exists(path: &Path) -> Result<bool, C3SlotJournalError> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Ok(_) => Ok(true),
        Err(_) => Err(C3SlotJournalError::Unknown),
    }
}

fn scan_is_exact_genesis(scan: &JournalScan, service_id: &str) -> bool {
    scan.initialized
        && scan.current.is_none()
        && scan.pending.is_none()
        && scan.reservation.is_none()
        && scan.last_revision == 1
        && scan.highest_catalog_generation == 0
        && scan.highest_c3_launch_generation == 0
        && scan.highest_operation_id == 0
        && scan.last_record.as_ref() == Some(&genesis_record(service_id))
}

pub(super) struct C3ReapedLaunchKey {
    pub(super) catalog_generation: NonZeroU64,
    pub(super) c3_launch_generation: NonZeroU64,
    pub(super) scope_binding_digest: [u8; 32],
    pub(super) launch_operation_id: NonZeroU64,
    pub(super) process_id: u32,
}

pub(super) enum C3SlotJournalReadback {
    Empty,
    Committed(Box<C3DurableSlotRowReadback>),
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
    launch_evidence: Option<DurableLaunchEvidence>,
    scope_snapshot: Option<RecoveryScopeSnapshot>,
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

    pub(super) fn launch_evidence(&self) -> Option<DurableLaunchEvidence> {
        self.launch_evidence
    }

    pub(super) fn scope_snapshot(&self) -> Option<&RecoveryScopeSnapshot> {
        self.scope_snapshot.as_ref()
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

fn ensure_cleanup_deadline_not_expired(
    deadline: Option<Instant>,
) -> Result<(), C3SlotJournalError> {
    ensure_cleanup_deadline_not_expired_at(deadline, &Instant::now)
}

fn ensure_cleanup_deadline_not_expired_at(
    deadline: Option<Instant>,
    now: &dyn Fn() -> Instant,
) -> Result<(), C3SlotJournalError> {
    if deadline.is_some_and(|deadline| now() >= deadline) {
        Err(C3SlotJournalError::Unknown)
    } else {
        Ok(())
    }
}

fn completed_readback_receipt_before_deadline(
    record: &JournalRecord,
    cleanup_deadline: Option<Instant>,
    now: &dyn Fn() -> Instant,
) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
    let receipt = C3DurableSlotReceipt::from_completed_readback(record)?;
    if let Some(deadline) = cleanup_deadline {
        // Take a new sample after receipt materialization; do not reuse the
        // pre-readback or pre-construction sample for the final success check.
        let completed_at = now();
        if completed_at >= deadline {
            return Err(C3SlotJournalError::Unknown);
        }
    }
    Ok(receipt)
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

    fn acquire_until(file: File, deadline: Instant) -> io::Result<Self> {
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "timed out waiting for the C3 slot journal lock",
                ));
            }
            let result = unsafe { flock(file.as_raw_fd(), LOCK_EX | LOCK_NB) };
            if result == 0 {
                let lock = Self { file };
                if Instant::now() >= deadline {
                    drop(lock);
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "timed out waiting for the C3 slot journal lock",
                    ));
                }
                return Ok(lock);
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
    launch_evidence: Option<DurableLaunchEvidence>,
    scope_snapshot: Option<RecoveryScopeSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct JournalRecord {
    format_version: u8,
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
    pending_reserved_launch: bool,
    normal_exit_records: Vec<JournalRecord>,
}

/// A reserved Live intent may be durably enriched exactly once after C3 has
/// created its official record for the gated child and before execution is
/// released. The C3 operation ID remains the launch-instance key throughout.
fn reserved_launch_identity_update_matches(pending: &JournalRow, staged: &JournalRow) -> bool {
    let (Some(evidence), Some(scope)) = (&staged.launch_evidence, &staged.scope_snapshot) else {
        return false;
    };
    pending.state == C3SlotState::Live
        && pending.c1_operation_id == 0
        && pending.launch_evidence.is_none()
        && pending.scope_snapshot.is_none()
        && staged.state == C3SlotState::Live
        && staged.c1_operation_id == 0
        && pending.service_id == staged.service_id
        && pending.catalog_generation == staged.catalog_generation
        && pending.c3_launch_generation == staged.c3_launch_generation
        && pending.scope_binding_digest == staged.scope_binding_digest
        && pending.operation_id == staged.operation_id
        && evidence.state == LaunchEvidenceState::Running
        && evidence.launch_operation_id == staged.operation_id
        && evidence.process_id != 0
        && evidence.is_valid_for(staged)
        && scope_snapshot_matches_row(scope, staged)
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
        || (next.c1_operation_id != 0
            && !matches!(
                next.state,
                C3SlotState::Live | C3SlotState::ClearedAndAbsent
            ))
        || next.catalog_generation < highest_catalog_generation
        || next.c3_launch_generation < highest_c3_launch_generation
        || next
            .launch_evidence
            .is_some_and(|evidence| !evidence.is_valid_for(next))
        || next.launch_evidence.is_some() != next.scope_snapshot.is_some()
        || next
            .scope_snapshot
            .as_ref()
            .is_some_and(|scope| !scope_snapshot_matches_row(scope, next))
    {
        return false;
    }

    match next.state {
        C3SlotState::Live => {
            if next.c1_operation_id != 0 {
                current.is_some_and(|current| {
                    current.state == C3SlotState::Live
                        && same_slot_binding(current, next)
                        && current.launch_evidence == next.launch_evidence
                        && current.scope_snapshot == next.scope_snapshot
                })
            } else {
                next.c3_launch_generation > highest_c3_launch_generation
                    && current.is_none_or(|current| current.state != C3SlotState::Quarantined)
            }
        }
        C3SlotState::Reserved => false,
        C3SlotState::ClearedAndAbsent => current.is_some_and(|current| {
            same_slot_binding(current, next)
                && if current.state == C3SlotState::ClearedAndAbsent {
                    next.c1_operation_id != 0
                        && current.c1_operation_id != next.c1_operation_id
                        && next.launch_evidence.is_none()
                } else if next
                    .launch_evidence
                    .is_some_and(|evidence| evidence.state == LaunchEvidenceState::NormalExit)
                {
                    current.state == C3SlotState::Live
                        && current.c1_operation_id != 0
                        && current.c1_operation_id == next.c1_operation_id
                        && current.scope_snapshot == next.scope_snapshot
                        && current.launch_evidence.is_some_and(|current_evidence| {
                            current_evidence.state == LaunchEvidenceState::Running
                                && next.launch_evidence.is_some_and(|next_evidence| {
                                    next_evidence.launch_operation_id
                                        == current_evidence.launch_operation_id
                                        && next_evidence.process_id == current_evidence.process_id
                                })
                        })
                } else {
                    current.state == C3SlotState::Live
                        && next.c1_operation_id == 0
                        && next.launch_evidence.is_none()
                }
        }),
        C3SlotState::Quarantined => current.is_some_and(|current| {
            matches!(
                current.state,
                C3SlotState::Live | C3SlotState::ClearedAndAbsent
            ) && same_slot_binding(current, next)
                && next.launch_evidence.is_none()
        }),
    }
}

fn same_slot_binding(left: &JournalRow, right: &JournalRow) -> bool {
    left.service_id == right.service_id
        && left.catalog_generation == right.catalog_generation
        && left.c3_launch_generation == right.c3_launch_generation
        && left.scope_binding_digest == right.scope_binding_digest
}

fn scope_snapshot_matches_row(scope: &RecoveryScopeSnapshot, row: &JournalRow) -> bool {
    scope.service_id == row.service_id
        && scope.catalog_generation.get() == row.catalog_generation
        && scope.c3_launch_generation.get() == row.c3_launch_generation
        && scope.binding_digest == row.scope_binding_digest
        && scope.canonical_digest() == row.scope_binding_digest
}

fn genesis_record(service_id: &str) -> JournalRecord {
    JournalRecord {
        format_version: JOURNAL_VERSION,
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
            launch_evidence: None,
            scope_snapshot: None,
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
        .custom_flags(O_NOFOLLOW_LINUX | O_CLOEXEC_LINUX | O_NONBLOCK_LINUX)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(()),
        Err(_) => return Err(()),
    };
    scan_journal_file(&mut file, service_id, requested_operation_id)
}

fn scan_journal_file(
    file: &mut File,
    service_id: &str,
    requested_operation_id: Option<NonZeroU64>,
) -> Result<JournalScan, ()> {
    validate_private_file(file).map_err(|_| ())?;
    let mut scan = JournalScan::default();
    while let Some(record) = read_record(file)? {
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
            let genesis = genesis_record(service_id);
            if record.kind != genesis.kind
                || record.revision != genesis.revision
                || record.row != genesis.row
            {
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
                let reserved_launch_identity_update = scan.pending_reserved_launch
                    && scan.pending.as_ref().is_some_and(|pending| {
                        reserved_launch_identity_update_matches(&pending.row, &record.row)
                    });
                let reserved_transition = scan.reservation.as_ref().is_some_and(|reservation| {
                    reservation.row.operation_id == record.row.operation_id
                        && reservation.row.c3_launch_generation == record.row.c3_launch_generation
                        && matches!(
                            record.row.state,
                            C3SlotState::Live | C3SlotState::Quarantined
                        )
                        && record.row.c1_operation_id == 0
                        && record.row.launch_evidence.is_none()
                        && record.row.scope_snapshot.is_none()
                        && record.row.catalog_generation >= scan.highest_catalog_generation
                        && scan
                            .current
                            .as_ref()
                            .is_none_or(|current| current.row.state != C3SlotState::Quarantined)
                });
                let operation_id_is_valid =
                    if reserved_transition || reserved_launch_identity_update {
                        true
                    } else {
                        record.row.operation_id > scan.highest_operation_id
                    };
                let transition_is_valid = if reserved_transition || reserved_launch_identity_update
                {
                    true
                } else if record.row.state == C3SlotState::Live && record.row.c1_operation_id == 0 {
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
                if (scan.pending.is_some() && !reserved_launch_identity_update)
                    || !operation_id_is_valid
                    || !transition_is_valid
                {
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
                if !reserved_launch_identity_update {
                    scan.pending_reserved_launch = reserved_transition
                        && record.format_version == JOURNAL_VERSION
                        && record.row.state == C3SlotState::Live
                        && record.row.launch_evidence.is_none()
                        && record.row.scope_snapshot.is_none();
                }
                scan.pending = Some(record.clone());
            }
            RecordKind::Committed => {
                let pending = scan.pending.take().ok_or(())?;
                if record.format_version != pending.format_version
                    || record.revision
                        != pending
                            .revision
                            .checked_add(1)
                            .filter(|revision| *revision != 0)
                            .ok_or(())?
                    || !committed_row_matches_intent(&pending.row, &record.row)
                    || (scan.pending_reserved_launch
                        && record
                            .row
                            .launch_evidence
                            .is_none_or(|evidence| evidence.state != LaunchEvidenceState::Running))
                {
                    return Err(());
                }
                scan.pending_reserved_launch = false;
                if record
                    .row
                    .launch_evidence
                    .is_some_and(|evidence| evidence.state == LaunchEvidenceState::NormalExit)
                {
                    scan.normal_exit_records.push(record.clone());
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

fn committed_row_matches_intent(intent: &JournalRow, committed: &JournalRow) -> bool {
    intent == committed
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
    let version = header[8];
    if &header[..JOURNAL_MAGIC.len()] != JOURNAL_MAGIC
        || !matches!(
            version,
            LEGACY_JOURNAL_VERSION_V3 | LEGACY_JOURNAL_VERSION_V4 | JOURNAL_VERSION
        )
    {
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
    let mut extension_bytes = Vec::new();
    let mut launch_evidence = None;
    let mut scope_snapshot_bytes = None;
    if version == JOURNAL_VERSION {
        let mut launch_header = [0u8; V5_EXTENSION_HEADER_LEN];
        reader.read_exact(&mut launch_header).map_err(|_| ())?;
        extension_bytes.extend_from_slice(&launch_header);
        let launch_state = launch_header[0];
        let launch_operation_id = read_u64(&launch_header[1..9])?;
        let process_id = read_u64(&launch_header[9..17])?;
        let scope_len = u32::from_le_bytes(
            launch_header[V5_LAUNCH_EVIDENCE_LEN..V5_EXTENSION_HEADER_LEN]
                .try_into()
                .map_err(|_| ())?,
        ) as usize;
        if scope_len > MAX_SCOPE_SNAPSHOT_LEN {
            return Err(());
        }
        match launch_state {
            0 if launch_operation_id == 0 && process_id == 0 && scope_len == 0 => {}
            1 if launch_operation_id != 0
                && process_id != 0
                && process_id <= u64::from(u32::MAX)
                && scope_len != 0 =>
            {
                launch_evidence = Some(DurableLaunchEvidence {
                    launch_operation_id,
                    process_id: u32::try_from(process_id).map_err(|_| ())?,
                    state: LaunchEvidenceState::Running,
                });
            }
            2 if launch_operation_id != 0
                && process_id != 0
                && process_id <= u64::from(u32::MAX)
                && scope_len != 0 =>
            {
                launch_evidence = Some(DurableLaunchEvidence {
                    launch_operation_id,
                    process_id: u32::try_from(process_id).map_err(|_| ())?,
                    state: LaunchEvidenceState::NormalExit,
                });
            }
            _ => return Err(()),
        }
        if scope_len != 0 {
            let mut bytes = vec![0u8; scope_len];
            reader.read_exact(&mut bytes).map_err(|_| ())?;
            extension_bytes.extend_from_slice(&bytes);
            scope_snapshot_bytes = Some(bytes);
        }
    }
    let mut supplied_checksum = [0u8; CHECKSUM_LEN];
    reader.read_exact(&mut supplied_checksum).map_err(|_| ())?;
    let mut checksum_input =
        Vec::with_capacity(HEADER_LEN + service_id_len + extension_bytes.len());
    checksum_input.extend_from_slice(&header);
    checksum_input.extend_from_slice(&service_id_bytes);
    checksum_input.extend_from_slice(&extension_bytes);
    let expected_checksum: [u8; 32] = Sha256::digest(&checksum_input).into();
    if supplied_checksum != expected_checksum {
        return Err(());
    }
    let service_id = String::from_utf8(service_id_bytes).map_err(|_| ())?;
    if !canonical_service_id(&service_id) {
        return Err(());
    }
    let scope_snapshot = scope_snapshot_bytes
        .as_deref()
        .map(|bytes| {
            decode_scope_snapshot(
                bytes,
                &service_id,
                catalog_generation,
                c3_launch_generation,
                &scope_binding_digest,
            )
        })
        .transpose()?;
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
                && (c1_operation_id == 0
                    || state == C3SlotState::ClearedAndAbsent
                    || (version >= LEGACY_JOURNAL_VERSION_V4 && state == C3SlotState::Live))
                && c3_launch_generation != 0
                && catalog_generation != 0 => {}
        _ => return Err(()),
    }

    let row = JournalRow {
        service_id,
        catalog_generation,
        c3_launch_generation,
        scope_binding_digest,
        operation_id,
        c1_operation_id,
        state,
        launch_evidence,
        scope_snapshot,
    };
    if row.launch_evidence.is_some() != row.scope_snapshot.is_some()
        || row
            .launch_evidence
            .is_some_and(|evidence| !evidence.is_valid_for(&row))
        || row
            .scope_snapshot
            .as_ref()
            .is_some_and(|scope| !scope_snapshot_matches_row(scope, &row))
        || (version != JOURNAL_VERSION
            && (row.launch_evidence.is_some() || row.scope_snapshot.is_some()))
    {
        return Err(());
    }

    Ok(Some(JournalRecord {
        format_version: version,
        kind,
        revision,
        row,
    }))
}

fn encode_record(record: &JournalRecord) -> Result<Vec<u8>, ()> {
    let service_id = record.row.service_id.as_bytes();
    if record.format_version != JOURNAL_VERSION
        || service_id.is_empty()
        || service_id.len() > MAX_SERVICE_ID_LEN
        || !canonical_service_id(&record.row.service_id)
        || record.revision == 0
        || record.row.launch_evidence.is_some() != record.row.scope_snapshot.is_some()
        || record
            .row
            .launch_evidence
            .is_some_and(|evidence| !evidence.is_valid_for(&record.row))
        || record
            .row
            .scope_snapshot
            .as_ref()
            .is_some_and(|scope| !scope_snapshot_matches_row(scope, &record.row))
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
                    || matches!(
                        record.row.state,
                        C3SlotState::Live | C3SlotState::ClearedAndAbsent
                    ))
                && record.row.c3_launch_generation != 0
                && record.row.catalog_generation != 0 => {}
        _ => return Err(()),
    }
    let scope_bytes = match record.row.scope_snapshot.as_ref() {
        Some(scope) => encode_scope_snapshot(scope, &record.row)?,
        None => Vec::new(),
    };
    if scope_bytes.len() > MAX_SCOPE_SNAPSHOT_LEN {
        return Err(());
    }
    let mut bytes = Vec::with_capacity(
        HEADER_LEN + service_id.len() + V5_EXTENSION_HEADER_LEN + scope_bytes.len() + CHECKSUM_LEN,
    );
    bytes.extend_from_slice(JOURNAL_MAGIC);
    bytes.push(record.format_version);
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
    match record.row.launch_evidence {
        None => bytes.push(0),
        Some(evidence) => {
            bytes.push(match evidence.state {
                LaunchEvidenceState::Running => 1,
                LaunchEvidenceState::NormalExit => 2,
            });
            bytes.extend_from_slice(&evidence.launch_operation_id.to_le_bytes());
            bytes.extend_from_slice(&u64::from(evidence.process_id).to_le_bytes());
        }
    }
    if record.row.launch_evidence.is_none() {
        bytes.extend_from_slice(&[0; 16]);
    }
    bytes.extend_from_slice(&(scope_bytes.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&scope_bytes);
    let checksum: [u8; 32] = Sha256::digest(&bytes).into();
    bytes.extend_from_slice(&checksum);
    Ok(bytes)
}

fn read_u64(bytes: &[u8]) -> Result<u64, ()> {
    let array: [u8; 8] = bytes.try_into().map_err(|_| ())?;
    Ok(u64::from_le_bytes(array))
}

fn push_scope_text(output: &mut Vec<u8>, value: &str) -> Result<(), ()> {
    let bytes = value.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_SCOPE_FIELD_LEN || bytes.len() > u16::MAX as usize {
        return Err(());
    }
    output.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    output.extend_from_slice(bytes);
    Ok(())
}

fn encode_scope_snapshot(scope: &RecoveryScopeSnapshot, row: &JournalRow) -> Result<Vec<u8>, ()> {
    if !scope_snapshot_matches_row(scope, row) {
        return Err(());
    }
    let mut bytes = Vec::new();
    push_scope_text(&mut bytes, &scope.principal_id)?;
    push_scope_text(&mut bytes, &scope.caller_context_id)?;
    bytes.extend_from_slice(&scope.mapping_generation.get().to_le_bytes());
    push_scope_text(&mut bytes, &scope.role)?;
    match scope.transition {
        RecoveryLifecycleTransition::Initial => {
            bytes.push(0);
            bytes.extend_from_slice(&0u64.to_le_bytes());
        }
        RecoveryLifecycleTransition::Replace {
            previous_generation,
        } => {
            bytes.push(1);
            bytes.extend_from_slice(&previous_generation.get().to_le_bytes());
        }
    }
    push_scope_text(&mut bytes, &scope.package_id)?;
    bytes.extend_from_slice(&scope.package_app_id.to_le_bytes());
    push_scope_text(&mut bytes, &scope.canonical_entry)?;
    bytes.extend_from_slice(&scope.signed_package_sha256);
    bytes.extend_from_slice(&scope.executable_sha256);
    bytes.extend_from_slice(&scope.package_generation.to_le_bytes());
    bytes.extend_from_slice(&scope.uid.to_le_bytes());
    bytes.extend_from_slice(&scope.gid.to_le_bytes());
    if bytes.len() > MAX_SCOPE_SNAPSHOT_LEN {
        return Err(());
    }
    Ok(bytes)
}

fn decode_scope_snapshot(
    bytes: &[u8],
    service_id: &str,
    catalog_generation: u64,
    c3_launch_generation: u64,
    scope_binding_digest: &[u8; 32],
) -> Result<RecoveryScopeSnapshot, ()> {
    let mut cursor = std::io::Cursor::new(bytes);
    let principal_id = read_scope_text(&mut cursor)?;
    let caller_context_id = read_scope_text(&mut cursor)?;
    let mapping_generation = NonZeroU64::new(read_scope_u64(&mut cursor)?).ok_or(())?;
    let role = read_scope_text(&mut cursor)?;
    let mut transition_tag = [0u8; 1];
    cursor.read_exact(&mut transition_tag).map_err(|_| ())?;
    let transition_generation = read_scope_u64(&mut cursor)?;
    let transition = match transition_tag[0] {
        0 if transition_generation == 0 => RecoveryLifecycleTransition::Initial,
        1 => RecoveryLifecycleTransition::Replace {
            previous_generation: NonZeroU64::new(transition_generation).ok_or(())?,
        },
        _ => return Err(()),
    };
    let package_id = read_scope_text(&mut cursor)?;
    let package_app_id = read_scope_u64(&mut cursor)?;
    let canonical_entry = read_scope_text(&mut cursor)?;
    let mut signed_package_sha256 = [0u8; 32];
    cursor
        .read_exact(&mut signed_package_sha256)
        .map_err(|_| ())?;
    let mut executable_sha256 = [0u8; 32];
    cursor.read_exact(&mut executable_sha256).map_err(|_| ())?;
    let package_generation = read_scope_u64(&mut cursor)?;
    let uid = read_scope_u32(&mut cursor)?;
    let gid = read_scope_u32(&mut cursor)?;
    if cursor.position() != bytes.len() as u64 {
        return Err(());
    }
    let scope = RecoveryScopeSnapshot {
        service_id: service_id.to_owned(),
        principal_id,
        caller_context_id,
        mapping_generation,
        role,
        catalog_generation: NonZeroU64::new(catalog_generation).ok_or(())?,
        transition,
        package_id,
        package_app_id,
        canonical_entry,
        signed_package_sha256,
        executable_sha256,
        package_generation,
        c3_launch_generation: NonZeroU64::new(c3_launch_generation).ok_or(())?,
        uid,
        gid,
        binding_digest: *scope_binding_digest,
    };
    if scope.canonical_digest() != *scope_binding_digest {
        return Err(());
    }
    Ok(scope)
}

fn read_scope_text(reader: &mut impl Read) -> Result<String, ()> {
    let mut length = [0u8; 2];
    reader.read_exact(&mut length).map_err(|_| ())?;
    let length = u16::from_le_bytes(length) as usize;
    if length == 0 || length > MAX_SCOPE_FIELD_LEN {
        return Err(());
    }
    let mut bytes = vec![0u8; length];
    reader.read_exact(&mut bytes).map_err(|_| ())?;
    String::from_utf8(bytes).map_err(|_| ())
}

fn read_scope_u64(reader: &mut impl Read) -> Result<u64, ()> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes).map_err(|_| ())?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_scope_u32(reader: &mut impl Read) -> Result<u32, ()> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes).map_err(|_| ())?;
    Ok(u32::from_le_bytes(bytes))
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
        .custom_flags(O_NOFOLLOW_LINUX | O_CLOEXEC_LINUX | O_NONBLOCK_LINUX);
    if append {
        existing.append(true);
    }
    existing.open(path)
}

fn open_existing_private_marker(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(O_NOFOLLOW_LINUX | O_CLOEXEC_LINUX | O_NONBLOCK_LINUX)
        .open(path)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MetadataAttributeCheck {
    Matches,
    Mismatch,
    Unavailable,
}

#[derive(Clone, Copy)]
enum MetadataWriteBoundary {
    BeforeFirstWrite,
    AfterWriteStarted,
}

fn classify_metadata_attributes<T>(
    metadata: io::Result<T>,
    attributes_match: impl FnOnce(&T) -> bool,
) -> MetadataAttributeCheck {
    match metadata {
        Ok(metadata) if attributes_match(&metadata) => MetadataAttributeCheck::Matches,
        Ok(_) => MetadataAttributeCheck::Mismatch,
        Err(_) => MetadataAttributeCheck::Unavailable,
    }
}

fn metadata_attribute_result(
    check: MetadataAttributeCheck,
    boundary: MetadataWriteBoundary,
) -> Result<(), C3SlotJournalError> {
    match check {
        MetadataAttributeCheck::Matches => Ok(()),
        MetadataAttributeCheck::Mismatch => Err(match boundary {
            MetadataWriteBoundary::BeforeFirstWrite => C3SlotJournalError::RejectedBeforeWrite,
            MetadataWriteBoundary::AfterWriteStarted => C3SlotJournalError::Rejected,
        }),
        MetadataAttributeCheck::Unavailable => Err(C3SlotJournalError::Unknown),
    }
}

fn normalize_after_first_write<T>(
    result: Result<T, C3SlotJournalError>,
) -> Result<T, C3SlotJournalError> {
    result.map_err(|error| match error {
        C3SlotJournalError::RejectedBeforeWrite => C3SlotJournalError::Rejected,
        error => error,
    })
}

fn validate_parent_before_write(parent: &File) -> Result<(), C3SlotJournalError> {
    let effective_uid = unsafe { geteuid() };
    let check = classify_metadata_attributes(parent.metadata(), |metadata| {
        metadata.file_type().is_dir()
            && metadata.uid() == effective_uid
            && metadata.mode() & 0o022 == 0
    });
    metadata_attribute_result(check, MetadataWriteBoundary::BeforeFirstWrite)
}

fn validate_private_file(file: &File) -> io::Result<()> {
    let metadata = file.metadata()?;
    let effective_uid = unsafe { geteuid() };
    if !private_file_attributes_match(&metadata, effective_uid) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "C3 journal file is not a private, singly-linked regular file",
        ));
    }
    Ok(())
}

fn private_file_attributes_match(metadata: &std::fs::Metadata, effective_uid: u32) -> bool {
    metadata.file_type().is_file()
        && metadata.uid() == effective_uid
        && metadata.mode() & 0o077 == 0
        && metadata.nlink() == 1
}

fn validate_private_file_before_write(file: &File) -> Result<(), C3SlotJournalError> {
    let effective_uid = unsafe { geteuid() };
    let check = classify_metadata_attributes(file.metadata(), |metadata| {
        private_file_attributes_match(metadata, effective_uid)
    });
    metadata_attribute_result(check, MetadataWriteBoundary::BeforeFirstWrite)
}

fn validate_private_file_after_write(file: &File) -> Result<(), C3SlotJournalError> {
    let effective_uid = unsafe { geteuid() };
    let check = classify_metadata_attributes(file.metadata(), |metadata| {
        private_file_attributes_match(metadata, effective_uid)
    });
    metadata_attribute_result(check, MetadataWriteBoundary::AfterWriteStarted)
}

#[cfg(test)]
mod tests {
    use super::{
        C3DurableSlotReceipt, C3ReapedLaunchKey, C3SlotJournal, C3SlotJournalError,
        C3SlotJournalReadback, C3SlotState, C3SlotTransition, DurableLaunchEvidence,
        LaunchEvidenceState, RecoveryScopeSnapshot,
    };
    use nagi_system_service_recovery_contract::{
        C1RecoveryOperationCorrelation, RecoveryLifecycleTransition,
    };
    use sha2::Digest;
    use std::fs::{self, OpenOptions};
    use std::io::{self, Read, Seek, SeekFrom, Write};
    use std::num::NonZeroU64;
    use std::os::fd::AsRawFd;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, FileTypeExt};
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
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
    const CHILD_TERMINAL_JOURNAL_ENV: &str = "NAGI_C3_SLOT_JOURNAL_TERMINAL_PATH";
    const CHILD_TERMINAL_MARKER_ENV: &str = "NAGI_C3_SLOT_JOURNAL_TERMINAL_MARKER";
    const F_GETFL: i32 = 3;

    unsafe extern "C" {
        fn fcntl(fd: i32, command: i32, ...) -> i32;
        fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
    }

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

    fn scope_snapshot(launch_generation: u64) -> RecoveryScopeSnapshot {
        let mut scope = RecoveryScopeSnapshot {
            service_id: SERVICE_ID.to_owned(),
            principal_id: "service:fixture".to_owned(),
            caller_context_id: "caller_fixture".to_owned(),
            mapping_generation: nonzero(2),
            role: "system_service".to_owned(),
            catalog_generation: nonzero(1),
            transition: RecoveryLifecycleTransition::Initial,
            package_id: "org.nagi.fixture".to_owned(),
            package_app_id: 7,
            canonical_entry: "bin/fixture-service".to_owned(),
            signed_package_sha256: [0x11; 32],
            executable_sha256: [0x22; 32],
            package_generation: 3,
            c3_launch_generation: nonzero(launch_generation),
            uid: 1000,
            gid: 1000,
            binding_digest: [0; 32],
        };
        scope.binding_digest = scope.canonical_digest();
        scope
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

    fn reserved_live_launch(
        journal: &C3SlotJournal,
        launch_generation: u64,
        effect: impl FnOnce() -> io::Result<()>,
    ) -> Result<C3DurableSlotReceipt, C3SlotJournalError> {
        let scope = scope_snapshot(launch_generation);
        let reservation = journal.reserve_next_generation().unwrap();
        assert_eq!(reservation.launch_generation(), nonzero(launch_generation));
        journal.transition_reserved_launch(
            reservation,
            scope.catalog_generation,
            scope.binding_digest,
            scope,
            |stage| {
                stage(4107)?;
                effect()?;
                Ok(4107)
            },
        )
    }

    fn receipt_row(receipt: &C3DurableSlotReceipt, expected_digest: &[u8; 32]) {
        assert_eq!(receipt.service_id(), SERVICE_ID);
        assert_eq!(receipt.catalog_generation(), nonzero(1));
        assert_eq!(receipt.scope_binding_digest(), expected_digest);
    }

    #[test]
    fn c3_slot_journal_keeps_proc_fd_parent_identity_after_path_replacement() {
        let directory = TestDirectory::new();
        let selected_parent = directory.0.join("selected-journal");
        fs::create_dir(&selected_parent).unwrap();
        let selected_journal_path = selected_parent.join("slot.journal");
        drop(C3SlotJournal::create(&selected_journal_path, SERVICE_ID).unwrap());

        let selected_parent_fd = std::fs::File::open(&selected_parent).unwrap();
        let selected_fd_path =
            std::path::PathBuf::from(format!("/proc/self/fd/{}", selected_parent_fd.as_raw_fd()));
        let anchored =
            C3SlotJournal::open(selected_fd_path.join("slot.journal"), SERVICE_ID).unwrap();
        drop(selected_parent_fd);

        let moved_parent = directory.0.join("selected-journal-moved");
        fs::rename(&selected_parent, &moved_parent).unwrap();
        fs::create_dir(&selected_parent).unwrap();
        drop(C3SlotJournal::create(&selected_journal_path, SERVICE_ID).unwrap());

        assert_eq!(
            anchored
                .reserve_next_generation()
                .unwrap()
                .launch_generation(),
            nonzero(1),
            "an open journal stays bound to the selected directory object"
        );
        assert!(matches!(
            C3SlotJournal::open(moved_parent.join("slot.journal"), SERVICE_ID)
                .unwrap()
                .reserve_next_generation(),
            Err(C3SlotJournalError::Unknown)
        ));
        assert_eq!(
            C3SlotJournal::open(&selected_journal_path, SERVICE_ID)
                .unwrap()
                .reserve_next_generation()
                .unwrap()
                .launch_generation(),
            nonzero(1),
            "the replacement at the old path is an independent journal"
        );
    }

    #[test]
    fn c3_first_provision_marker_opens_nonblocking_and_checks_fifo_before_read() {
        let directory = TestDirectory::new();
        let marker_path = directory.0.join("slot.journal.first-provision");
        let c_path = std::ffi::CString::new(marker_path.as_os_str().as_bytes()).unwrap();
        let created = unsafe { mkfifo(c_path.as_ptr(), 0o600) };
        assert_eq!(created, 0, "mkfifo failed: {}", io::Error::last_os_error());

        let marker = super::open_existing_private_marker(&marker_path).unwrap();
        let flags = unsafe { fcntl(marker.as_raw_fd(), F_GETFL) };
        assert!(flags >= 0, "F_GETFL failed: {}", io::Error::last_os_error());
        assert_ne!(flags & 0o4000, 0, "marker descriptor lacks O_NONBLOCK");
        assert!(marker.metadata().unwrap().file_type().is_fifo());
        assert_eq!(
            super::validate_private_file_before_write(&marker),
            Err(C3SlotJournalError::RejectedBeforeWrite)
        );
    }

    #[test]
    fn c3_metadata_read_failure_remains_unknown_at_both_write_boundaries() {
        let unavailable = super::classify_metadata_attributes::<bool>(
            Err(io::Error::other("synthetic metadata read failure")),
            |matches| *matches,
        );
        assert_eq!(unavailable, super::MetadataAttributeCheck::Unavailable);
        assert_eq!(
            super::metadata_attribute_result(
                unavailable,
                super::MetadataWriteBoundary::BeforeFirstWrite
            ),
            Err(C3SlotJournalError::Unknown)
        );
        assert_eq!(
            super::metadata_attribute_result(
                unavailable,
                super::MetadataWriteBoundary::AfterWriteStarted
            ),
            Err(C3SlotJournalError::Unknown)
        );
        assert_eq!(
            super::normalize_after_first_write::<()>(Err(C3SlotJournalError::RejectedBeforeWrite)),
            Err(C3SlotJournalError::Rejected)
        );
    }

    #[test]
    fn c3_first_provision_marker_consumption_is_post_write() {
        assert_eq!(
            super::normalize_after_first_write::<()>(Err(C3SlotJournalError::RejectedBeforeWrite)),
            Err(C3SlotJournalError::Rejected),
            "marker consumption cannot release a reservation after the journal pair exists"
        );
        assert_eq!(
            super::normalize_after_first_write::<()>(Err(C3SlotJournalError::Unknown)),
            Err(C3SlotJournalError::Unknown)
        );
        assert_eq!(super::normalize_after_first_write(Ok(())), Ok(()));
    }

    #[test]
    fn c3_slot_journal_reopen_preserves_high_water_and_rejects_replay() {
        let directory = TestDirectory::new();
        let path = directory.journal_path();
        let effects = std::sync::atomic::AtomicUsize::new(0);
        let journal = C3SlotJournal::create(&path, SERVICE_ID).unwrap();
        let expected_scope = scope_snapshot(1);
        let receipt = reserved_live_launch(&journal, 1, || {
            effects.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        receipt_row(&receipt, &expected_scope.binding_digest);
        assert_eq!(receipt.durable_revision(), nonzero(5));
        assert_eq!(receipt.operation_id(), nonzero(1));
        assert_eq!(receipt.state(), C3SlotState::Live);
        drop(journal);

        let reopened = C3SlotJournal::open(&path, SERVICE_ID).unwrap();
        match reopened.readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.service_id(), SERVICE_ID);
                assert_eq!(row.catalog_generation(), 1);
                assert_eq!(row.c3_launch_generation(), 1);
                assert_eq!(row.scope_binding_digest(), &expected_scope.binding_digest);
                assert_eq!(row.durable_revision(), 5);
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

        let next = reserved_live_launch(&reopened, 2, || {
            effects.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        assert_eq!(next.durable_revision(), nonzero(9));
        assert_eq!(next.c3_launch_generation(), nonzero(2));
        assert_eq!(next.operation_id(), nonzero(2));
        assert_eq!(effects.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn c3_slot_journal_clears_only_the_exact_live_binding() {
        let directory = TestDirectory::new();
        let journal = C3SlotJournal::create(directory.journal_path(), SERVICE_ID).unwrap();
        let scope = scope_snapshot(1);
        reserved_live_launch(&journal, 1, || Ok(())).unwrap();
        let cleared = journal
            .transition_next(
                scope.catalog_generation,
                scope.c3_launch_generation,
                scope.binding_digest,
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
    fn c3_slot_journal_genesis_only_proves_no_prior_launch_row() {
        let directory = TestDirectory::new();
        let journal = C3SlotJournal::create(directory.journal_path(), SERVICE_ID).unwrap();

        assert!(journal
            .latest_normal_exit_scope_snapshot()
            .unwrap()
            .is_none());
    }

    #[test]
    fn c3_slot_journal_cold_reopen_reads_exact_normal_exit_scope_and_receipt() {
        let directory = TestDirectory::new();
        let path = directory.journal_path();
        let journal = C3SlotJournal::create(&path, SERVICE_ID).unwrap();
        let scope = scope_snapshot(1);
        let reservation = journal.reserve_next_generation().unwrap();
        let live = journal
            .transition_reserved_launch(
                reservation,
                scope.catalog_generation,
                scope.binding_digest,
                scope.clone(),
                |stage| {
                    stage(4107)?;
                    Ok(4107)
                },
            )
            .unwrap();
        let c1_operation_id = nonzero(77);
        let correlation =
            C1RecoveryOperationCorrelation::new(c1_operation_id, scope.binding_digest);
        let cleanup_deadline = Instant::now() + Duration::from_secs(10);
        journal
            .with_c1_correlated_live_receipt(
                scope.catalog_generation,
                scope.c3_launch_generation,
                &scope.binding_digest,
                correlation,
                || Ok(()),
            )
            .unwrap();
        let written_receipt = journal
            .record_normal_exit_after_reap(
                C3ReapedLaunchKey {
                    catalog_generation: scope.catalog_generation,
                    c3_launch_generation: scope.c3_launch_generation,
                    scope_binding_digest: scope.binding_digest,
                    launch_operation_id: live.operation_id(),
                    process_id: 4107,
                },
                correlation,
                Some(cleanup_deadline),
                || Ok(()),
            )
            .unwrap();
        assert!(Instant::now() < cleanup_deadline);
        assert_eq!(written_receipt.state(), C3SlotState::ClearedAndAbsent);
        drop(journal);

        // A reopened journal has no OwnedLaunch or process handle. The old
        // selectors and terminal evidence must all come from durable rows.
        let reopened = C3SlotJournal::open(&path, SERVICE_ID).unwrap();
        let receipt = reopened
            .read_normal_exit_receipt(&scope, live.operation_id(), 4107, correlation)
            .unwrap();
        assert_eq!(receipt.service_id(), SERVICE_ID);
        assert_eq!(receipt.catalog_generation(), scope.catalog_generation);
        assert_eq!(receipt.c3_launch_generation(), scope.c3_launch_generation);
        assert_eq!(receipt.scope_binding_digest(), &scope.binding_digest);
        assert_eq!(receipt.scope_snapshot(), &scope);
        assert_eq!(receipt.launch_operation_id(), live.operation_id());
        assert_eq!(receipt.launch_instance_id(), live.operation_id());
        assert_eq!(receipt.process_id(), nonzero(4107));
        assert_eq!(receipt.c1_operation_id(), c1_operation_id);
        assert_eq!(receipt.terminal_revision(), nonzero(9));
        assert!(receipt.terminal_operation_id() > live.operation_id());
        assert_eq!(
            receipt.terminal_operation_id(),
            written_receipt.operation_id()
        );
        assert!(reopened.latest_normal_exit_scope_snapshot().unwrap() == Some(receipt));

        assert!(matches!(
            reopened.read_normal_exit_receipt(&scope, nonzero(999), 4107, correlation),
            Err(C3SlotJournalError::Unknown)
        ));
        assert!(matches!(
            reopened.read_normal_exit_receipt(&scope, live.operation_id(), 4108, correlation),
            Err(C3SlotJournalError::Unknown)
        ));
        assert!(matches!(
            reopened.read_normal_exit_receipt(
                &scope,
                live.operation_id(),
                4107,
                C1RecoveryOperationCorrelation::new(nonzero(78), scope.binding_digest),
            ),
            Err(C3SlotJournalError::Unknown)
        ));
        let mut changed_scope = scope.clone();
        changed_scope.package_app_id += 1;
        changed_scope.binding_digest = changed_scope.canonical_digest();
        let changed_correlation =
            C1RecoveryOperationCorrelation::new(c1_operation_id, changed_scope.binding_digest);
        assert!(matches!(
            reopened.read_normal_exit_receipt(
                &changed_scope,
                live.operation_id(),
                4107,
                changed_correlation,
            ),
            Err(C3SlotJournalError::Unknown)
        ));
        assert!(matches!(
            reopened.read_normal_exit_receipt(
                &scope_snapshot(2),
                live.operation_id(),
                4107,
                correlation,
            ),
            Err(C3SlotJournalError::Rejected | C3SlotJournalError::Unknown)
        ));
    }

    #[test]
    fn c3_completed_normal_exit_receipt_samples_deadline_after_materialization() {
        let directory = TestDirectory::new();
        let journal = C3SlotJournal::create(directory.journal_path(), SERVICE_ID).unwrap();
        let scope = scope_snapshot(1);
        let reservation = journal.reserve_next_generation().unwrap();
        let live = journal
            .transition_reserved_launch(
                reservation,
                scope.catalog_generation,
                scope.binding_digest,
                scope.clone(),
                |stage| {
                    stage(4107)?;
                    Ok(4107)
                },
            )
            .unwrap();
        let correlation = C1RecoveryOperationCorrelation::new(nonzero(92), scope.binding_digest);
        journal
            .with_c1_correlated_live_receipt(
                scope.catalog_generation,
                scope.c3_launch_generation,
                &scope.binding_digest,
                correlation,
                || Ok(()),
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(60);
        let before_deadline = deadline - Duration::from_nanos(1);
        let normal_exit_now = || before_deadline;
        let terminal_receipt = journal
            .record_normal_exit_after_reap_with_clock(
                C3ReapedLaunchKey {
                    catalog_generation: scope.catalog_generation,
                    c3_launch_generation: scope.c3_launch_generation,
                    scope_binding_digest: scope.binding_digest,
                    launch_operation_id: live.operation_id(),
                    process_id: 4107,
                },
                correlation,
                Some(deadline),
                &normal_exit_now,
                || Ok(()),
            )
            .unwrap();
        assert_eq!(terminal_receipt.state(), C3SlotState::ClearedAndAbsent);

        let scan = super::scan_journal(&journal.path, SERVICE_ID, None).unwrap();
        let completed_record = scan.current.as_ref().unwrap();
        let success_samples = AtomicUsize::new(0);
        let before_sample = || {
            success_samples.fetch_add(1, Ordering::SeqCst);
            before_deadline
        };
        let completed_receipt = super::completed_readback_receipt_before_deadline(
            completed_record,
            Some(deadline),
            &before_sample,
        )
        .unwrap();
        assert_eq!(success_samples.load(Ordering::SeqCst), 1);
        assert_eq!(
            completed_receipt.operation_id(),
            terminal_receipt.operation_id()
        );

        let boundary_samples = AtomicUsize::new(0);
        let at_deadline_sample = || {
            boundary_samples.fetch_add(1, Ordering::SeqCst);
            deadline
        };
        assert!(matches!(
            super::completed_readback_receipt_before_deadline(
                completed_record,
                Some(deadline),
                &at_deadline_sample,
            ),
            Err(C3SlotJournalError::Unknown)
        ));
        assert_eq!(boundary_samples.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn c3_slot_journal_new_generation_does_not_inherit_prior_exit_identity() {
        let directory = TestDirectory::new();
        let journal = C3SlotJournal::create(directory.journal_path(), SERVICE_ID).unwrap();
        let first_scope = scope_snapshot(1);
        let first_reservation = journal.reserve_next_generation().unwrap();
        let first_live = journal
            .transition_reserved_launch(
                first_reservation,
                first_scope.catalog_generation,
                first_scope.binding_digest,
                first_scope.clone(),
                |stage| {
                    stage(4107)?;
                    Ok(4107)
                },
            )
            .unwrap();
        let first_correlation =
            C1RecoveryOperationCorrelation::new(nonzero(79), first_scope.binding_digest);
        journal
            .with_c1_correlated_live_receipt(
                first_scope.catalog_generation,
                first_scope.c3_launch_generation,
                &first_scope.binding_digest,
                first_correlation,
                || Ok(()),
            )
            .unwrap();
        journal
            .record_normal_exit_after_reap(
                C3ReapedLaunchKey {
                    catalog_generation: first_scope.catalog_generation,
                    c3_launch_generation: first_scope.c3_launch_generation,
                    scope_binding_digest: first_scope.binding_digest,
                    launch_operation_id: first_live.operation_id(),
                    process_id: 4107,
                },
                first_correlation,
                None,
                || Ok(()),
            )
            .unwrap();
        let first_terminal_receipt = journal
            .read_normal_exit_receipt(
                &first_scope,
                first_live.operation_id(),
                4107,
                first_correlation,
            )
            .unwrap();

        let second_scope = scope_snapshot(2);
        let second_reservation = journal.reserve_next_generation().unwrap();
        let second_live = journal
            .transition_reserved_launch(
                second_reservation,
                second_scope.catalog_generation,
                second_scope.binding_digest,
                second_scope.clone(),
                |stage| {
                    stage(4108)?;
                    Ok(4108)
                },
            )
            .unwrap();
        assert!(second_live.operation_id() > first_live.operation_id());
        assert!(
            journal
                .read_normal_exit_receipt(
                    &first_scope,
                    first_live.operation_id(),
                    4107,
                    first_correlation,
                )
                .unwrap()
                == first_terminal_receipt,
            "the completed previous-generation receipt stays exact across the next reservation, staged intent, and commit",
        );
        match journal.readback() {
            C3SlotJournalReadback::Committed(row) => {
                assert_eq!(row.state(), C3SlotState::Live);
                assert_eq!(
                    row.c3_launch_generation(),
                    second_scope.c3_launch_generation.get()
                );
                assert_eq!(row.scope_binding_digest(), &second_scope.binding_digest);
                assert_eq!(row.scope_snapshot(), Some(&second_scope));
                let evidence = row
                    .launch_evidence()
                    .expect("new live generation has its own durable process identity");
                assert!(!evidence.is_normal_exit());
                assert_eq!(evidence.launch_operation_id(), second_live.operation_id());
                assert_eq!(evidence.process_id(), 4108);
            }
            C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                panic!("second-generation live row did not read back")
            }
        }
    }

    #[test]
    fn c3_slot_journal_terminal_intent_crash_stays_unknown_and_blocks_reuse() {
        let directory = TestDirectory::new();
        let journal_path = directory.journal_path();
        let marker_path = directory.0.join("terminal-intent.synced");
        let child = std::env::args_os().next().expect("test binary argv[0]");
        let status = Command::new(child)
            .arg("c3_slot_journal_child_crash_after_terminal_intent")
            .env(CHILD_TERMINAL_JOURNAL_ENV, &journal_path)
            .env(CHILD_TERMINAL_MARKER_ENV, &marker_path)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(75));
        assert_eq!(fs::read(&marker_path).unwrap(), b"terminal-intent-synced");

        let reopened = C3SlotJournal::open(&journal_path, SERVICE_ID).unwrap();
        assert!(matches!(
            reopened.readback(),
            C3SlotJournalReadback::Unknown
        ));
        assert!(matches!(
            reopened.latest_normal_exit_scope_snapshot(),
            Err(C3SlotJournalError::Unknown)
        ));
        assert!(matches!(
            reopened.reserve_next_generation(),
            Err(C3SlotJournalError::Unknown)
        ));
    }

    #[test]
    fn c3_slot_journal_reads_legacy_v3_and_v4_rows_without_proof() {
        for version in [3, 4] {
            for has_legacy_terminal in [false, true] {
                let directory = TestDirectory::new();
                let path = directory.journal_path();
                let journal = C3SlotJournal::create(&path, SERVICE_ID).unwrap();
                let records = legacy_history_records(version, has_legacy_terminal);
                drop(journal);
                let mut file = OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(&path)
                    .unwrap();
                for record in records {
                    file.write_all(&record).unwrap();
                }
                file.sync_all().unwrap();
                drop(file);

                let reopened = C3SlotJournal::open(&path, SERVICE_ID).unwrap();
                let scan = super::scan_journal(&path, SERVICE_ID, None).unwrap();
                assert_eq!(
                    scan.current.as_ref().map(|record| record.format_version),
                    Some(if version == 3 { 3 } else { 4 })
                );
                assert!(scan.normal_exit_records.is_empty());
                let expected_state = if has_legacy_terminal {
                    C3SlotState::ClearedAndAbsent
                } else {
                    C3SlotState::Live
                };
                match reopened.readback() {
                    C3SlotJournalReadback::Committed(row) => {
                        assert_eq!(row.state(), expected_state);
                        assert_eq!(row.launch_evidence(), None);
                        assert_eq!(row.scope_snapshot(), None);
                    }
                    C3SlotJournalReadback::Empty | C3SlotJournalReadback::Unknown => {
                        panic!("legacy v{version} {expected_state:?} row did not remain readable")
                    }
                }
                assert!(matches!(
                    reopened.latest_normal_exit_scope_snapshot(),
                    Err(C3SlotJournalError::Unknown)
                ));
            }
        }
    }

    #[test]
    fn c3_slot_journal_rejects_v5_launch_evidence_without_matching_staged_intent() {
        let directory = TestDirectory::new();
        let path = directory.journal_path();
        let journal = C3SlotJournal::create(&path, SERVICE_ID).unwrap();
        let scope = scope_snapshot(1);
        let live_identity_row = super::JournalRow {
            service_id: SERVICE_ID.to_owned(),
            catalog_generation: scope.catalog_generation.get(),
            c3_launch_generation: scope.c3_launch_generation.get(),
            scope_binding_digest: scope.binding_digest,
            operation_id: 1,
            c1_operation_id: 0,
            state: C3SlotState::Live,
            launch_evidence: None,
            scope_snapshot: None,
        };
        let mut evidence_row = live_identity_row.clone();
        evidence_row.launch_evidence = Some(DurableLaunchEvidence {
            launch_operation_id: 1,
            process_id: 4107,
            state: LaunchEvidenceState::Running,
        });
        evidence_row.scope_snapshot = Some(scope);
        let records = [
            super::JournalRecord {
                format_version: super::JOURNAL_VERSION,
                kind: super::RecordKind::Reservation,
                revision: 2,
                row: super::JournalRow {
                    service_id: SERVICE_ID.to_owned(),
                    catalog_generation: 0,
                    c3_launch_generation: 1,
                    scope_binding_digest: [0; 32],
                    operation_id: 1,
                    c1_operation_id: 0,
                    state: C3SlotState::Reserved,
                    launch_evidence: None,
                    scope_snapshot: None,
                },
            },
            super::JournalRecord {
                format_version: super::JOURNAL_VERSION,
                kind: super::RecordKind::Intent,
                revision: 3,
                row: live_identity_row,
            },
            super::JournalRecord {
                format_version: super::JOURNAL_VERSION,
                kind: super::RecordKind::Committed,
                revision: 4,
                row: evidence_row,
            },
        ];
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        for record in records {
            file.write_all(&super::encode_record(&record).unwrap())
                .unwrap();
        }
        file.sync_all().unwrap();

        assert!(matches!(journal.readback(), C3SlotJournalReadback::Unknown));
    }

    #[test]
    fn c3_slot_journal_rejects_v5_reserved_live_without_running_evidence() {
        let directory = TestDirectory::new();
        let path = directory.journal_path();
        let journal = C3SlotJournal::create(&path, SERVICE_ID).unwrap();
        let live_row = super::JournalRow {
            service_id: SERVICE_ID.to_owned(),
            catalog_generation: 1,
            c3_launch_generation: 1,
            scope_binding_digest: DIGEST,
            operation_id: 1,
            c1_operation_id: 0,
            state: C3SlotState::Live,
            launch_evidence: None,
            scope_snapshot: None,
        };
        let records = [
            super::JournalRecord {
                format_version: super::JOURNAL_VERSION,
                kind: super::RecordKind::Reservation,
                revision: 2,
                row: super::JournalRow {
                    service_id: SERVICE_ID.to_owned(),
                    catalog_generation: 0,
                    c3_launch_generation: 1,
                    scope_binding_digest: [0; 32],
                    operation_id: 1,
                    c1_operation_id: 0,
                    state: C3SlotState::Reserved,
                    launch_evidence: None,
                    scope_snapshot: None,
                },
            },
            super::JournalRecord {
                format_version: super::JOURNAL_VERSION,
                kind: super::RecordKind::Intent,
                revision: 3,
                row: live_row.clone(),
            },
            super::JournalRecord {
                format_version: super::JOURNAL_VERSION,
                kind: super::RecordKind::Committed,
                revision: 4,
                row: live_row,
            },
        ];
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        for record in records {
            file.write_all(&super::encode_record(&record).unwrap())
                .unwrap();
        }
        file.sync_all().unwrap();

        assert!(matches!(journal.readback(), C3SlotJournalReadback::Unknown));
    }

    fn legacy_history_records(version: u8, has_terminal: bool) -> Vec<Vec<u8>> {
        // Both legacy versions accept the ordinary uncorrelated reserved-Live
        // transition (C1 operation ID zero). Keep v3 bytes on that exact path.
        let row = |catalog_generation, operation_id, state| super::JournalRow {
            service_id: SERVICE_ID.to_owned(),
            catalog_generation,
            c3_launch_generation: 1,
            scope_binding_digest: if catalog_generation == 0 {
                [0; 32]
            } else {
                DIGEST
            },
            operation_id,
            c1_operation_id: 0,
            state,
            launch_evidence: None,
            scope_snapshot: None,
        };
        let mut records = vec![
            super::genesis_record(SERVICE_ID),
            super::JournalRecord {
                format_version: super::JOURNAL_VERSION,
                kind: super::RecordKind::Reservation,
                revision: 2,
                row: row(0, 1, C3SlotState::Reserved),
            },
            super::JournalRecord {
                format_version: super::JOURNAL_VERSION,
                kind: super::RecordKind::Intent,
                revision: 3,
                row: row(1, 1, C3SlotState::Live),
            },
            super::JournalRecord {
                format_version: super::JOURNAL_VERSION,
                kind: super::RecordKind::Committed,
                revision: 4,
                row: row(1, 1, C3SlotState::Live),
            },
        ];
        if has_terminal {
            records.extend([
                super::JournalRecord {
                    format_version: super::JOURNAL_VERSION,
                    kind: super::RecordKind::Intent,
                    revision: 5,
                    row: row(1, 2, C3SlotState::ClearedAndAbsent),
                },
                super::JournalRecord {
                    format_version: super::JOURNAL_VERSION,
                    kind: super::RecordKind::Committed,
                    revision: 6,
                    row: row(1, 2, C3SlotState::ClearedAndAbsent),
                },
            ]);
        }
        records
            .iter()
            .map(|record| {
                let encoded = super::encode_record(record).unwrap();
                let service_id_len = u16::from_le_bytes([encoded[11], encoded[12]]) as usize;
                let mut legacy = encoded[..super::HEADER_LEN + service_id_len].to_vec();
                legacy[8] = version;
                let checksum: [u8; 32] = super::Sha256::digest(&legacy).into();
                legacy.extend_from_slice(&checksum);
                legacy
            })
            .collect()
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
    fn c3_journal_absolute_deadline_covers_readback_and_uncorrelated_transition() {
        let directory = TestDirectory::new();
        let journal = C3SlotJournal::create(directory.journal_path(), SERVICE_ID).unwrap();
        reserved_live_launch(&journal, 1, || Ok(())).unwrap();
        let expired_deadline = Instant::now() - Duration::from_millis(1);
        assert!(matches!(
            journal.readback_until(expired_deadline),
            C3SlotJournalReadback::Unknown
        ));

        let effect_ran = AtomicBool::new(false);
        assert!(matches!(
            journal.transition_next_until(
                nonzero(1),
                nonzero(1),
                DIGEST,
                C3SlotState::ClearedAndAbsent,
                expired_deadline,
                || {
                    effect_ran.store(true, Ordering::SeqCst);
                    Ok(())
                },
            ),
            Err(C3SlotJournalError::Unknown)
        ));
        assert!(!effect_ran.load(Ordering::SeqCst));
        assert!(matches!(
            journal.readback(),
            C3SlotJournalReadback::Committed(row) if row.state() == C3SlotState::Live
        ));
    }

    #[test]
    fn c3_normal_exit_transition_honors_absolute_journal_lock_deadline() {
        let directory = TestDirectory::new();
        let journal = C3SlotJournal::create(directory.journal_path(), SERVICE_ID).unwrap();
        let scope = scope_snapshot(1);
        let reservation = journal.reserve_next_generation().unwrap();
        let live = journal
            .transition_reserved_launch(
                reservation,
                scope.catalog_generation,
                scope.binding_digest,
                scope.clone(),
                |stage| {
                    stage(4107)?;
                    Ok(4107)
                },
            )
            .unwrap();
        let correlation = C1RecoveryOperationCorrelation::new(nonzero(89), scope.binding_digest);
        journal
            .with_c1_correlated_live_receipt(
                scope.catalog_generation,
                scope.c3_launch_generation,
                &scope.binding_digest,
                correlation,
                || Ok(()),
            )
            .unwrap();
        let held_lock = journal.lock_exclusive().unwrap();
        let effect_ran = AtomicBool::new(false);
        let cleanup_deadline = Instant::now() + Duration::from_millis(600);
        std::thread::sleep(Duration::from_millis(400));
        let started = Instant::now();

        assert!(matches!(
            journal.record_normal_exit_after_reap(
                C3ReapedLaunchKey {
                    catalog_generation: scope.catalog_generation,
                    c3_launch_generation: scope.c3_launch_generation,
                    scope_binding_digest: scope.binding_digest,
                    launch_operation_id: live.operation_id(),
                    process_id: 4107,
                },
                correlation,
                Some(cleanup_deadline),
                || {
                    effect_ran.store(true, Ordering::SeqCst);
                    Ok(())
                },
            ),
            Err(C3SlotJournalError::Unknown)
        ));
        assert!(!effect_ran.load(Ordering::SeqCst));
        assert!(started.elapsed() < Duration::from_millis(400));
        drop(held_lock);
        assert!(matches!(
            journal.readback(),
            C3SlotJournalReadback::Committed(row) if row.state() == C3SlotState::Live
        ));
    }

    #[test]
    fn c3_normal_exit_expired_deadline_precedes_evidence_rejection() {
        let directory = TestDirectory::new();
        let journal = C3SlotJournal::create(directory.journal_path(), SERVICE_ID).unwrap();
        let scope = scope_snapshot(1);
        let reservation = journal.reserve_next_generation().unwrap();
        let live = journal
            .transition_reserved_launch(
                reservation,
                scope.catalog_generation,
                scope.binding_digest,
                scope.clone(),
                |stage| {
                    stage(4107)?;
                    Ok(4107)
                },
            )
            .unwrap();
        let correlation = C1RecoveryOperationCorrelation::new(nonzero(90), scope.binding_digest);
        journal
            .with_c1_correlated_live_receipt(
                scope.catalog_generation,
                scope.c3_launch_generation,
                &scope.binding_digest,
                correlation,
                || Ok(()),
            )
            .unwrap();
        let effect_ran = AtomicBool::new(false);
        let expired_deadline = Instant::now() - Duration::from_millis(1);

        assert!(matches!(
            journal.record_normal_exit_after_reap(
                C3ReapedLaunchKey {
                    catalog_generation: scope.catalog_generation,
                    c3_launch_generation: scope.c3_launch_generation,
                    scope_binding_digest: scope.binding_digest,
                    launch_operation_id: live.operation_id(),
                    process_id: 4108,
                },
                correlation,
                Some(expired_deadline),
                || {
                    effect_ran.store(true, Ordering::SeqCst);
                    Ok(())
                },
            ),
            Err(C3SlotJournalError::Unknown)
        ));
        assert!(!effect_ran.load(Ordering::SeqCst));
        assert!(matches!(
            journal.readback(),
            C3SlotJournalReadback::Committed(row) if row.state() == C3SlotState::Live
        ));
    }

    #[test]
    fn c3_normal_exit_deadline_expiry_before_terminal_append_keeps_intent_pending() {
        let directory = TestDirectory::new();
        let journal = C3SlotJournal::create(directory.journal_path(), SERVICE_ID).unwrap();
        let scope = scope_snapshot(1);
        let reservation = journal.reserve_next_generation().unwrap();
        let live = journal
            .transition_reserved_launch(
                reservation,
                scope.catalog_generation,
                scope.binding_digest,
                scope.clone(),
                |stage| {
                    stage(4107)?;
                    Ok(4107)
                },
            )
            .unwrap();
        let correlation = C1RecoveryOperationCorrelation::new(nonzero(91), scope.binding_digest);
        journal
            .with_c1_correlated_live_receipt(
                scope.catalog_generation,
                scope.c3_launch_generation,
                &scope.binding_digest,
                correlation,
                || Ok(()),
            )
            .unwrap();

        // The fake clock advances only after the normal-exit effect runs. This
        // deterministically checks the boundary before the terminal append.
        let cleanup_deadline = Instant::now() + Duration::from_secs(60);
        let before_deadline = cleanup_deadline - Duration::from_secs(1);
        let after_deadline = cleanup_deadline + Duration::from_nanos(1);
        let effect_ran = AtomicBool::new(false);
        let now = || {
            if effect_ran.load(Ordering::SeqCst) {
                after_deadline
            } else {
                before_deadline
            }
        };

        assert!(matches!(
            journal.record_normal_exit_after_reap_with_clock(
                C3ReapedLaunchKey {
                    catalog_generation: scope.catalog_generation,
                    c3_launch_generation: scope.c3_launch_generation,
                    scope_binding_digest: scope.binding_digest,
                    launch_operation_id: live.operation_id(),
                    process_id: 4107,
                },
                correlation,
                Some(cleanup_deadline),
                &now,
                || {
                    effect_ran.store(true, Ordering::SeqCst);
                    Ok(())
                },
            ),
            Err(C3SlotJournalError::Unknown)
        ));
        assert!(effect_ran.load(Ordering::SeqCst));

        let scan = super::scan_journal(&journal.path, &journal.service_id, None).unwrap();
        assert!(matches!(
            scan.current.as_ref(),
            Some(record) if record.row.state == C3SlotState::Live
        ));
        assert!(matches!(
            scan.pending.as_ref(),
            Some(record)
                if record.row.state == C3SlotState::ClearedAndAbsent
                    && record.row.launch_evidence.is_some_and(|evidence|
                        evidence.state == LaunchEvidenceState::NormalExit)
        ));
        assert!(matches!(journal.readback(), C3SlotJournalReadback::Unknown));
    }

    #[test]
    fn c3_journal_deadline_is_checked_after_encode_before_append() {
        let directory = TestDirectory::new();
        let journal = C3SlotJournal::create(directory.journal_path(), SERVICE_ID).unwrap();
        let initial_len = fs::metadata(&journal.path).unwrap().len();
        let cleanup_deadline = Instant::now() + Duration::from_secs(60);
        let before_deadline = cleanup_deadline - Duration::from_secs(1);
        let after_deadline = cleanup_deadline + Duration::from_nanos(1);
        let clock_reads = AtomicUsize::new(0);
        let now = || {
            if clock_reads.fetch_add(1, Ordering::SeqCst) < 2 {
                before_deadline
            } else {
                after_deadline
            }
        };

        assert!(journal
            .append_and_readback_with_deadline(
                &super::genesis_record(SERVICE_ID),
                None,
                Some(cleanup_deadline),
                &now,
            )
            .is_err());
        assert_eq!(clock_reads.load(Ordering::SeqCst), 3);
        assert_eq!(fs::metadata(&journal.path).unwrap().len(), initial_len);
    }

    #[test]
    fn c3_slot_journal_corrupt_tail_is_unknown_and_never_runs_effect() {
        let directory = TestDirectory::new();
        let path = directory.journal_path();
        let journal = C3SlotJournal::create(&path, SERVICE_ID).unwrap();
        reserved_live_launch(&journal, 1, || Ok(())).unwrap();
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
        drop(file);

        assert!(matches!(journal.readback(), C3SlotJournalReadback::Unknown));
        let mut callback_ran = false;
        assert!(matches!(
            journal.transition_once(transition(32, 6, C3SlotState::Live), || {
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
                assert_eq!(row.durable_revision(), 5);
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
        let _ = reserved_live_launch(&journal, 1, || {
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
        reserved_live_launch(&journal, 1, || {
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
    fn c3_slot_journal_child_crash_after_terminal_intent() {
        let (Ok(journal_path), Ok(marker_path)) = (
            std::env::var(CHILD_TERMINAL_JOURNAL_ENV),
            std::env::var(CHILD_TERMINAL_MARKER_ENV),
        ) else {
            return;
        };
        let journal = C3SlotJournal::create(journal_path, SERVICE_ID).unwrap();
        let scope = scope_snapshot(1);
        let reservation = journal.reserve_next_generation().unwrap();
        let live = journal
            .transition_reserved_launch(
                reservation,
                scope.catalog_generation,
                scope.binding_digest,
                scope.clone(),
                |stage| {
                    stage(5107)?;
                    Ok(5107)
                },
            )
            .unwrap();
        let correlation = C1RecoveryOperationCorrelation::new(nonzero(88), scope.binding_digest);
        journal
            .with_c1_correlated_live_receipt(
                scope.catalog_generation,
                scope.c3_launch_generation,
                &scope.binding_digest,
                correlation,
                || Ok(()),
            )
            .unwrap();
        let _ = journal.record_normal_exit_after_reap(
            C3ReapedLaunchKey {
                catalog_generation: scope.catalog_generation,
                c3_launch_generation: scope.c3_launch_generation,
                scope_binding_digest: scope.binding_digest,
                launch_operation_id: live.operation_id(),
                process_id: 5107,
            },
            correlation,
            None,
            || {
                let mut marker = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(marker_path)?;
                marker.write_all(b"terminal-intent-synced")?;
                marker.sync_all()?;
                std::process::exit(75);
            },
        );
        panic!("child unexpectedly returned after terminal intent");
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
