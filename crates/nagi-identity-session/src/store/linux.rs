use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(test)]
use std::sync::{mpsc, Mutex};

use serde::{Deserialize, Serialize};

use crate::model::IdentitySnapshot;
use crate::store::{
    C1RecoveryOperationIdentity, CurrentSnapshotReader, IdentitySnapshotStore, ReconciliationFence,
    RecoveryJournal, RecoveryJournalPhase, SnapshotStoreError, SnapshotStoreState, SnapshotVersion,
    StoredSnapshot,
};
#[cfg(any(test, feature = "system-service-recovery-contract"))]
use crate::store::{
    RecoveryBinding, RecoveryEffectClaim, RecoveryJournalError, RecoveryJournalRecord,
    RecoveryJournalStore, RecoveryOperationId, RecoveryOperationPermit, RecoveryReconciliation,
    RecoveryReconciliationPermit,
};

#[cfg(test)]
type ReaderPauseAfterFenceCheck = Arc<Mutex<Option<(mpsc::SyncSender<()>, mpsc::Receiver<()>)>>>;

const SCHEMA: u32 = 1;
const MAX_SNAPSHOT_BYTES: usize = 1024 * 1024;
const MAX_STORE_BYTES: u64 = 12 * 1024 * 1024;
const MAX_HIGH_WATER_BYTES: u64 = 1024;
const NOFOLLOW: i32 = 0x20000; // Linux O_NOFOLLOW.
const LOCK_FILE: &str = "issuer.lock";
const STATE_FILE: &str = "issuer.json";
const HIGH_WATER_FILE: &str = "issuer.highwater";
const FRESH_MARKER: &[u8; 8] = b"NAGI-ID0";
const COMMITTED_MARKER: &[u8; 8] = b"NAGI-ID1";
const HIGH_WATER_SCHEMA: u32 = 1;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedPayload {
    version: u64,
    bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: u32,
    high_water_version: Option<u64>,
    current: Option<SavedPayload>,
    last_known_good: Option<SavedPayload>,
    #[serde(default)]
    recovery_journal: RecoveryJournal,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HighWaterRecord {
    schema: u32,
    version: u64,
}

impl Envelope {
    fn empty() -> Self {
        Self {
            schema: SCHEMA,
            high_water_version: None,
            current: None,
            last_known_good: None,
            recovery_journal: RecoveryJournal::default(),
        }
    }

    fn validate(&self) -> Result<(), DurableStoreError> {
        if self.schema != SCHEMA || self.high_water_version == Some(0) {
            return Err(DurableStoreError::Corrupt);
        }
        if !self.recovery_journal.validate() {
            return Err(DurableStoreError::Corrupt);
        }

        match self.high_water_version {
            None if self.current.is_none() && self.last_known_good.is_none() => {}
            None => return Err(DurableStoreError::Corrupt),
            Some(high_water) => {
                if self.current.as_ref().is_some_and(|saved| {
                    saved.version == 0
                        || saved.version > high_water
                        || saved.bytes.len() > MAX_SNAPSHOT_BYTES
                }) || self.last_known_good.as_ref().is_some_and(|saved| {
                    saved.version == 0
                        || saved.version >= high_water
                        || saved.bytes.len() > MAX_SNAPSHOT_BYTES
                }) {
                    return Err(DurableStoreError::Corrupt);
                }
            }
        }
        if self
            .current
            .as_ref()
            .is_some_and(|saved| saved.bytes.len() > MAX_SNAPSHOT_BYTES)
            || self
                .last_known_good
                .as_ref()
                .is_some_and(|saved| saved.bytes.len() > MAX_SNAPSHOT_BYTES)
        {
            return Err(DurableStoreError::Corrupt);
        }
        Ok(())
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, DurableStoreError> {
        let envelope: Self =
            serde_json::from_slice(bytes).map_err(|_| DurableStoreError::Corrupt)?;
        envelope.validate()?;
        Ok(envelope)
    }

    fn serialized(&self) -> Result<Vec<u8>, SnapshotStoreError> {
        serde_json::to_vec(self).map_err(|_| SnapshotStoreError::Unavailable)
    }

    fn current_at_high_water(&self) -> Option<StoredSnapshot> {
        let high_water = self.high_water_version?;
        let current = self.current.as_ref()?;
        (current.version == high_water).then(|| StoredSnapshot {
            bytes: current.bytes.clone(),
            version: SnapshotVersion::new(current.version),
        })
    }

    fn last_known_good(&self) -> Option<StoredSnapshot> {
        self.last_known_good.as_ref().map(|saved| StoredSnapshot {
            bytes: saved.bytes.clone(),
            version: SnapshotVersion::new(saved.version),
        })
    }
}

/// An operational error opening the Linux durable identity store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DurableStoreError {
    Unavailable,
    Corrupt,
    Locked,
}

/// Linux file-backed implementation of the C1 snapshot contract.
///
/// The integrating service supplies an already authorized directory handle.
/// The adapter requires a private directory, serializes writers with an
/// exclusive OS file lock, atomically replaces the complete envelope, and
/// advances the independent `issuer.highwater` revision before replacing that
/// envelope. Saved payloads are observations only; `IdentityService` recovery
/// still ends every saved active Session before it can issue a new one.
pub struct LinuxIdentitySnapshotStore {
    directory: File,
    lock: File,
    owner_uid: u32,
    marker_committed: bool,
    state: Envelope,
    disk: Option<Vec<u8>>,
    poisoned: bool,
    /// Counts raw snapshot readers and reserves/closes terminal publication.
    /// Store-only readers are independent of issuer Session activity, so a
    /// contended pre-terminal reservation can return retryable Busy.
    reconciliation_fence: Arc<ReconciliationFence>,
    #[cfg(test)]
    fault: Option<Fault>,
    #[cfg(test)]
    reader_pause_after_fence_check: ReaderPauseAfterFenceCheck,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Fault {
    AfterHighWater,
    PartialWrite,
    BeforeRename,
    AfterRename,
    ProcessExitDuringHighWaterWrite,
    ProcessExitAfterHighWater,
    ProcessExitDuringEnvelopeWrite,
}

impl LinuxIdentitySnapshotStore {
    /// Open an existing private store or a truly empty directory.
    ///
    /// The first successful commit permanently changes `issuer.lock` to a
    /// committed marker before replacing `issuer.json`. A missing/corrupt
    /// state after that point therefore fails closed instead of looking like a
    /// never-used store with a reusable Session generation.
    pub fn open(directory: File) -> Result<Self, DurableStoreError> {
        let metadata = directory
            .metadata()
            .map_err(|_| DurableStoreError::Unavailable)?;
        if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(DurableStoreError::Unavailable);
        }
        let owner_uid = metadata.uid();
        let root = root_for(&directory);
        let (mut lock, created) = open_lock(&root)?;
        lock.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => DurableStoreError::Locked,
            std::fs::TryLockError::Error(_) => DurableStoreError::Unavailable,
        })?;
        validate_regular_private_file(&lock, owner_uid)?;
        remove_stale_pending_files(&root, &directory, owner_uid)?;

        if created {
            lock.write_all(FRESH_MARKER)
                .and_then(|()| lock.sync_all())
                .map_err(|_| DurableStoreError::Unavailable)?;
            directory
                .sync_all()
                .map_err(|_| DurableStoreError::Unavailable)?;
        }
        let committed = read_marker(&mut lock)?;
        let durable_high_water = read_high_water(&root, owner_uid)?;
        let disk = read_state_bytes(&root, owner_uid)?;
        if created && disk.is_some() {
            return Err(DurableStoreError::Corrupt);
        }

        let mut state = match disk.as_deref() {
            Some(bytes) if committed => Envelope::from_bytes(bytes)?,
            Some(bytes) => {
                let state = Envelope::from_bytes(bytes)?;
                if state.high_water_version.is_none()
                    && state.current.is_none()
                    && state.last_known_good.is_none()
                    && state.recovery_journal.record.is_some()
                {
                    ensure_only_lock_and_state_file(&root)?;
                    state
                } else {
                    return Err(DurableStoreError::Corrupt);
                }
            }
            None if committed => return Err(DurableStoreError::Corrupt),
            None => {
                ensure_only_lock_file(&root)?;
                Envelope::empty()
            }
        };
        match (committed, durable_high_water) {
            (false, None)
                if state.high_water_version.is_none()
                    && state.current.is_none()
                    && state.last_known_good.is_none()
                    && state.recovery_journal.record.is_some() => {}
            (false, None) if state.high_water_version.is_none() => {}
            (true, Some(high_water)) => {
                let saved_high_water = match state.high_water_version {
                    Some(saved_high_water) => saved_high_water,
                    None if state.current.is_none()
                        && state.last_known_good.is_none()
                        && state.recovery_journal.record.is_some() =>
                    {
                        high_water
                    }
                    None => return Err(DurableStoreError::Corrupt),
                };
                if saved_high_water > high_water {
                    return Err(DurableStoreError::Corrupt);
                }
                let current = state.current.take();
                if let Some(current) = current {
                    if current.version > high_water {
                        return Err(DurableStoreError::Corrupt);
                    }
                    if current.version == high_water {
                        state.current = Some(current);
                    } else if current.version == saved_high_water
                        && IdentitySnapshot::from_json(&current.bytes).is_ok()
                        && state
                            .last_known_good
                            .as_ref()
                            .is_none_or(|last_good| last_good.version < current.version)
                    {
                        // A separately persisted high-water can be ahead of a
                        // valid but rolled-back/unfinished envelope. Keep that
                        // exact older snapshot only as recovery input.
                        state.last_known_good = Some(current);
                    }
                }
                state.high_water_version = Some(high_water);
                state.validate()?;
            }
            _ => return Err(DurableStoreError::Corrupt),
        }
        Ok(Self {
            directory,
            lock,
            owner_uid,
            marker_committed: committed,
            state,
            disk,
            poisoned: false,
            reconciliation_fence: Arc::new(ReconciliationFence::default()),
            #[cfg(test)]
            fault: None,
            #[cfg(test)]
            reader_pause_after_fence_check: Arc::new(Mutex::new(None)),
        })
    }

    fn root(&self) -> PathBuf {
        root_for(&self.directory)
    }

    fn ensure_lock_identity(&self) -> Result<(), SnapshotStoreError> {
        let held = self
            .lock
            .metadata()
            .map_err(|_| SnapshotStoreError::Unavailable)?;
        ensure_lock_identity_at(
            &self.root(),
            self.owner_uid,
            held.dev(),
            held.ino(),
            Some(self.marker_committed),
        )
        .map(|_| ())
    }

    fn ensure_disk(&self) -> Result<(), SnapshotStoreError> {
        if self.poisoned {
            return Err(SnapshotStoreError::Unavailable);
        }
        self.ensure_lock_identity()?;
        if read_state_bytes(&self.root(), self.owner_uid)
            .map_err(|_| SnapshotStoreError::Unavailable)?
            != self.disk
        {
            return Err(SnapshotStoreError::Conflict);
        }
        if read_high_water(&self.root(), self.owner_uid)
            .map_err(|_| SnapshotStoreError::Unavailable)?
            != self.state.high_water_version
        {
            return Err(SnapshotStoreError::Conflict);
        }
        Ok(())
    }

    fn mark_committed(&mut self) -> Result<(), SnapshotStoreError> {
        if self.state.high_water_version.is_some() {
            return Ok(());
        }
        self.lock
            .seek(SeekFrom::Start(0))
            .and_then(|_| self.lock.write_all(COMMITTED_MARKER))
            .and_then(|()| self.lock.sync_all())
            .map_err(|_| SnapshotStoreError::Unavailable)?;
        self.marker_committed = true;
        Ok(())
    }

    fn persist_high_water(&mut self, version: u64) -> Result<(), SnapshotStoreError> {
        if version == 0 {
            return Err(SnapshotStoreError::Unavailable);
        }
        let record = HighWaterRecord {
            schema: HIGH_WATER_SCHEMA,
            version,
        };
        let bytes = serde_json::to_vec(&record).map_err(|_| SnapshotStoreError::Unavailable)?;
        let (path, mut pending) = create_pending(&self.root(), version)?;
        #[cfg(test)]
        let fault = self.fault;
        let write_result = (|| -> std::io::Result<()> {
            #[cfg(test)]
            if fault == Some(Fault::ProcessExitDuringHighWaterWrite) {
                let midpoint = bytes.len() / 2;
                pending.write_all(&bytes[..midpoint])?;
                pending.sync_all()?;
                std::process::exit(86);
            }
            pending.write_all(&bytes)?;
            pending.sync_all()?;
            fs::rename(&path, self.root().join(HIGH_WATER_FILE))?;
            self.directory.sync_all()?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(path);
            return Err(SnapshotStoreError::Unavailable);
        }
        self.state.high_water_version = Some(version);
        Ok(())
    }

    fn replace(&mut self, next: Envelope) -> Result<(), SnapshotStoreError> {
        self.ensure_disk()?;
        next.validate()
            .map_err(|_| SnapshotStoreError::Unavailable)?;
        let bytes = next.serialized()?;
        if bytes.len() as u64 > MAX_STORE_BYTES {
            return Err(SnapshotStoreError::Unavailable);
        }
        let (path, mut pending) =
            create_pending(&self.root(), next.high_water_version.unwrap_or(0))?;
        #[cfg(test)]
        let fault = self.fault.take();
        let write_result = (|| -> std::io::Result<()> {
            #[cfg(test)]
            if fault == Some(Fault::PartialWrite) {
                let midpoint = bytes.len() / 2;
                pending.write_all(&bytes[..midpoint])?;
                return Err(std::io::Error::other("injected interrupted store write"));
            }
            #[cfg(test)]
            if fault == Some(Fault::ProcessExitDuringEnvelopeWrite) {
                let midpoint = bytes.len() / 2;
                pending.write_all(&bytes[..midpoint])?;
                pending.sync_all()?;
                std::process::exit(86);
            }
            pending.write_all(&bytes)?;
            pending.sync_all()?;
            #[cfg(test)]
            if fault == Some(Fault::BeforeRename) {
                return Err(std::io::Error::other("injected pre-rename failure"));
            }
            fs::rename(&path, self.root().join(STATE_FILE))?;
            #[cfg(test)]
            if fault == Some(Fault::AfterRename) {
                return Err(std::io::Error::other("injected post-rename sync failure"));
            }
            self.directory.sync_all()?;
            Ok(())
        })();
        if write_result.is_err() {
            self.poisoned = true;
            let _ = fs::remove_file(path);
            return Err(SnapshotStoreError::Unavailable);
        }

        self.disk = Some(bytes);
        self.state = next;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn set_fault_for_test(&mut self, fault: Fault) {
        self.fault = Some(fault);
    }

    #[cfg(test)]
    pub(crate) fn pause_reader_after_fence_check_for_test(
        &mut self,
        entered: mpsc::SyncSender<()>,
        release: mpsc::Receiver<()>,
    ) {
        *self.reader_pause_after_fence_check.lock().unwrap() = Some((entered, release));
    }

    #[cfg(test)]
    pub(crate) fn reconciliation_writer_available_for_test(&self) -> bool {
        self.reconciliation_fence.writer_available_for_test()
    }

    #[cfg(test)]
    pub(crate) fn reconciliation_fence_state_for_test(&self) -> (bool, bool, usize) {
        self.reconciliation_fence.state_for_test()
    }

    fn current_snapshot_reader_with_runtime_operation(
        &self,
        runtime_operation: Option<C1RecoveryOperationIdentity>,
    ) -> Option<CurrentSnapshotReader> {
        let directory = self.directory.try_clone().ok()?;
        let owner_uid = self.owner_uid;
        let held_lock = self.lock.metadata().ok()?;
        let lock_dev = held_lock.dev();
        let lock_ino = held_lock.ino();
        let reconciliation_fence = Arc::clone(&self.reconciliation_fence);
        #[cfg(test)]
        let reader_pause_after_fence_check = Arc::clone(&self.reader_pause_after_fence_check);
        Some(Arc::new(move || {
            let _reader_permit = reconciliation_fence.enter_reader()?;
            #[cfg(test)]
            if let Some((entered, release)) = reader_pause_after_fence_check
                .lock()
                .map_err(|_| SnapshotStoreError::Unavailable)?
                .take()
            {
                entered
                    .send(())
                    .map_err(|_| SnapshotStoreError::Unavailable)?;
                release
                    .recv()
                    .map_err(|_| SnapshotStoreError::Unavailable)?;
            }
            let root = root_for(&directory);
            let marker_committed =
                ensure_lock_identity_at(&root, owner_uid, lock_dev, lock_ino, None)?;
            current_at_durable_high_water(&root, owner_uid, marker_committed, runtime_operation)
        }))
    }
}

impl IdentitySnapshotStore for LinuxIdentitySnapshotStore {
    fn current_snapshot_reader(&self) -> Option<CurrentSnapshotReader> {
        self.current_snapshot_reader_with_runtime_operation(None)
    }

    fn current_snapshot_reader_for_runtime(
        &self,
        operation: C1RecoveryOperationIdentity,
    ) -> Option<CurrentSnapshotReader> {
        self.current_snapshot_reader_with_runtime_operation(Some(operation))
    }

    fn load_current_state(&self) -> Result<SnapshotStoreState, SnapshotStoreError> {
        self.ensure_disk()?;
        Ok(SnapshotStoreState {
            current: self.state.current_at_high_water(),
            high_water_version: self.state.high_water_version.map(SnapshotVersion::new),
        })
    }

    fn recovery_quarantined(&self) -> Result<bool, SnapshotStoreError> {
        self.ensure_disk()?;
        let fence_quarantined = self.reconciliation_fence.is_closed_or_reserved()?;
        Ok(self.state.recovery_journal.is_quarantined() || fence_quarantined)
    }

    fn recovery_operation_active(&self) -> Result<bool, SnapshotStoreError> {
        self.ensure_disk()?;
        Ok(self.state.recovery_journal.owner_runtime_active())
    }

    fn load_current(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
        self.ensure_disk()?;
        Ok(self.state.current_at_high_water())
    }

    fn load_last_known_good(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
        self.ensure_disk()?;
        Ok(self.state.last_known_good())
    }

    fn commit_atomic(
        &mut self,
        expected_version: Option<SnapshotVersion>,
        complete_snapshot: &[u8],
    ) -> Result<SnapshotVersion, SnapshotStoreError> {
        self.ensure_disk()?;
        if self.state.high_water_version.map(SnapshotVersion::new) != expected_version {
            return Err(SnapshotStoreError::Conflict);
        }
        if complete_snapshot.len() > MAX_SNAPSHOT_BYTES {
            return Err(SnapshotStoreError::Unavailable);
        }
        IdentitySnapshot::from_json(complete_snapshot)
            .map_err(|_| SnapshotStoreError::Unavailable)?;

        let version = match expected_version {
            Some(current) => current
                .get()
                .checked_add(1)
                .ok_or(SnapshotStoreError::Unavailable)?,
            None => 1,
        };
        let next_version = SnapshotVersion::new(version);
        let last_known_good = match (self.state.current.as_ref(), expected_version) {
            (Some(current), Some(expected))
                if current.version == expected.get()
                    && IdentitySnapshot::from_json(&current.bytes).is_ok() =>
            {
                Some(current.clone())
            }
            _ => self.state.last_known_good.clone(),
        };
        let next = Envelope {
            schema: SCHEMA,
            high_water_version: Some(version),
            current: Some(SavedPayload {
                version,
                bytes: complete_snapshot.to_vec(),
            }),
            last_known_good,
            recovery_journal: self.state.recovery_journal.clone(),
        };
        next.validate()
            .map_err(|_| SnapshotStoreError::Unavailable)?;

        // Persist an independent high-water before replacing the envelope.
        // If the snapshot write is interrupted, reopening cannot allocate the
        // same CAS generation or treat an older envelope as current.
        if expected_version.is_none() {
            if let Err(error) = self.mark_committed() {
                self.poisoned = true;
                return Err(error);
            }
        }
        if let Err(error) = self.persist_high_water(version) {
            self.poisoned = true;
            return Err(error);
        }
        #[cfg(test)]
        if self.fault == Some(Fault::ProcessExitAfterHighWater) {
            std::process::exit(86);
        }
        #[cfg(test)]
        if self.fault == Some(Fault::AfterHighWater) {
            self.fault = None;
            self.poisoned = true;
            return Err(SnapshotStoreError::Unavailable);
        }
        if let Err(error) = self.replace(next) {
            self.poisoned = true;
            return Err(error);
        }
        Ok(next_version)
    }
}

#[cfg(any(test, feature = "system-service-recovery-contract"))]
impl RecoveryJournalStore for LinuxIdentitySnapshotStore {
    type ReconciliationPermit = RecoveryReconciliationPermit;

    fn recovery_journal_record(
        &self,
    ) -> Result<Option<RecoveryJournalRecord>, RecoveryJournalError> {
        self.ensure_disk().map_err(map_journal_store_error)?;
        Ok(self.state.recovery_journal.record.clone())
    }

    fn begin_recovery_intent(
        &mut self,
        binding: RecoveryBinding,
    ) -> Result<RecoveryOperationPermit, RecoveryJournalError> {
        self.ensure_disk().map_err(map_journal_store_error)?;
        if self
            .reconciliation_fence
            .is_closed_or_reserved()
            .map_err(map_journal_store_error)?
        {
            return Err(RecoveryJournalError::Quarantined);
        }
        if self.state.recovery_journal.blocks_new_operation() {
            return Err(RecoveryJournalError::Quarantined);
        }
        let operation_id = RecoveryOperationId(self.state.recovery_journal.next_operation_id);
        let next_operation_id = operation_id
            .get()
            .checked_add(1)
            .ok_or(RecoveryJournalError::Unavailable)?;
        let mut next = self.state.clone();
        next.recovery_journal.next_operation_id = next_operation_id;
        next.recovery_journal.record = Some(RecoveryJournalRecord {
            operation_id,
            binding: binding.clone(),
            phase: RecoveryJournalPhase::Intent,
        });
        self.replace(next).map_err(map_journal_store_error)?;
        Ok(RecoveryOperationPermit {
            operation_id,
            binding,
        })
    }

    fn claim_recovery_effect(
        &mut self,
        permit: &RecoveryOperationPermit,
    ) -> Result<RecoveryEffectClaim, RecoveryJournalError> {
        self.ensure_disk().map_err(map_journal_store_error)?;
        let record = self
            .state
            .recovery_journal
            .record
            .as_ref()
            .ok_or(RecoveryJournalError::NotCurrentOperation)?;
        if record.operation_id != permit.operation_id || record.binding != permit.binding {
            return Err(RecoveryJournalError::NotCurrentOperation);
        }
        match &record.phase {
            RecoveryJournalPhase::Intent => {}
            RecoveryJournalPhase::EffectStarted | RecoveryJournalPhase::OutcomeUnknown => {
                return Err(RecoveryJournalError::AlreadyClaimed)
            }
            RecoveryJournalPhase::RuntimeActive | RecoveryJournalPhase::Reconciled { .. } => {
                return Err(RecoveryJournalError::NotCurrentOperation)
            }
        }
        let mut next = self.state.clone();
        next.recovery_journal.record.as_mut().unwrap().phase = RecoveryJournalPhase::EffectStarted;
        self.replace(next).map_err(map_journal_store_error)?;
        Ok(RecoveryEffectClaim {
            operation_id: permit.operation_id,
            binding: permit.binding.clone(),
        })
    }

    fn commit_recovery_runtime_active(
        &mut self,
        claim: &RecoveryEffectClaim,
    ) -> Result<(), RecoveryJournalError> {
        self.ensure_disk().map_err(map_journal_store_error)?;
        let record = self
            .state
            .recovery_journal
            .record
            .as_ref()
            .ok_or(RecoveryJournalError::NotCurrentOperation)?;
        if record.operation_id != claim.operation_id || record.binding != claim.binding {
            return Err(RecoveryJournalError::NotCurrentOperation);
        }
        if !matches!(&record.phase, RecoveryJournalPhase::EffectStarted) {
            return Err(RecoveryJournalError::AlreadyClaimed);
        }
        let mut next = self.state.clone();
        next.recovery_journal.record.as_mut().unwrap().phase = RecoveryJournalPhase::RuntimeActive;
        self.replace(next).map_err(map_journal_store_error)?;
        Ok(())
    }

    fn mark_recovery_outcome_unknown(
        &mut self,
        operation_id: RecoveryOperationId,
        binding: &RecoveryBinding,
    ) -> Result<(), RecoveryJournalError> {
        self.ensure_disk().map_err(map_journal_store_error)?;
        let record = self
            .state
            .recovery_journal
            .record
            .as_ref()
            .ok_or(RecoveryJournalError::NotCurrentOperation)?;
        if record.operation_id != operation_id || &record.binding != binding {
            return Err(RecoveryJournalError::NotCurrentOperation);
        }
        match &record.phase {
            RecoveryJournalPhase::Intent
            | RecoveryJournalPhase::EffectStarted
            | RecoveryJournalPhase::RuntimeActive => {}
            RecoveryJournalPhase::OutcomeUnknown => return Ok(()),
            RecoveryJournalPhase::Reconciled { .. } => {
                return Err(RecoveryJournalError::NotCurrentOperation)
            }
        }
        let mut next = self.state.clone();
        next.recovery_journal.record.as_mut().unwrap().phase = RecoveryJournalPhase::OutcomeUnknown;
        self.replace(next).map_err(map_journal_store_error)
    }

    fn reconcile_recovery(
        &mut self,
        admission: &crate::service::IssuerRecoveryAdmissionGuard,
        mut permit: Self::ReconciliationPermit,
        reconciliation: RecoveryReconciliation,
    ) -> Result<(), RecoveryJournalError> {
        if !admission.is_permanently_closed() {
            return Err(RecoveryJournalError::Unavailable);
        }
        if !permit.belongs_to(&self.reconciliation_fence) {
            return Err(RecoveryJournalError::Unavailable);
        }
        // The exact operation/binding/digest was validated when this reserved
        // writer permit was prepared. Close the store fence immediately after
        // issuer closure so every later failure stays fail-closed.
        permit.begin_terminal_write(&reconciliation)?;
        let operation_id = reconciliation.operation_id();
        let binding = reconciliation.binding().clone();
        let outcome_digest = reconciliation.outcome_digest();
        self.ensure_disk().map_err(map_journal_store_error)?;
        let record = self
            .state
            .recovery_journal
            .record
            .as_ref()
            .ok_or(RecoveryJournalError::NotCurrentOperation)?;
        if record.operation_id != operation_id || record.binding != binding {
            return Err(RecoveryJournalError::NotCurrentOperation);
        }
        match &record.phase {
            RecoveryJournalPhase::Intent
            | RecoveryJournalPhase::EffectStarted
            | RecoveryJournalPhase::OutcomeUnknown
            | RecoveryJournalPhase::RuntimeActive => {}
            RecoveryJournalPhase::Reconciled {
                outcome_digest: existing,
            } if *existing == outcome_digest => return Ok(()),
            RecoveryJournalPhase::Reconciled { .. } => {
                return Err(RecoveryJournalError::OutcomeMismatch)
            }
        }
        let mut next = self.state.clone();
        next.recovery_journal.record.as_mut().unwrap().phase =
            RecoveryJournalPhase::Reconciled { outcome_digest };
        // The store reservation predates permanent issuer closure. The fence
        // is already published; retain it on any uncertain terminal write.
        self.replace(next).map_err(map_journal_store_error)?;
        Ok(())
    }

    fn prepare_recovery_reconciliation(
        &mut self,
        reconciliation: &RecoveryReconciliation,
    ) -> Result<Self::ReconciliationPermit, RecoveryJournalError> {
        self.ensure_disk().map_err(map_journal_store_error)?;
        let record = self
            .state
            .recovery_journal
            .record
            .as_ref()
            .ok_or(RecoveryJournalError::NotCurrentOperation)?;
        if record.operation_id != reconciliation.operation_id()
            || &record.binding != reconciliation.binding()
        {
            return Err(RecoveryJournalError::NotCurrentOperation);
        }
        match &record.phase {
            RecoveryJournalPhase::Intent
            | RecoveryJournalPhase::EffectStarted
            | RecoveryJournalPhase::OutcomeUnknown
            | RecoveryJournalPhase::RuntimeActive => {}
            RecoveryJournalPhase::Reconciled {
                outcome_digest: existing,
            } if *existing == reconciliation.outcome_digest() => {
                // A reopened store has a fresh process-local fence. Reserve
                // it again so the service can close its current issuer and
                // fence this store before reissuing the exact terminal receipt.
            }
            RecoveryJournalPhase::Reconciled { .. } => {
                return Err(RecoveryJournalError::OutcomeMismatch)
            }
        }
        self.reconciliation_fence
            .reserve_reconciliation(reconciliation)
    }
}

#[cfg(any(test, feature = "system-service-recovery-contract"))]
fn map_journal_store_error(error: SnapshotStoreError) -> RecoveryJournalError {
    match error {
        SnapshotStoreError::Unavailable => RecoveryJournalError::Unavailable,
        SnapshotStoreError::Conflict => RecoveryJournalError::Conflict,
    }
}

fn root_for(directory: &File) -> PathBuf {
    PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()))
}

fn open_lock(root: &std::path::Path) -> Result<(File, bool), DurableStoreError> {
    match OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(NOFOLLOW)
        .open(root.join(LOCK_FILE))
    {
        Ok(file) => Ok((file, true)),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(NOFOLLOW)
            .open(root.join(LOCK_FILE))
            .map(|file| (file, false))
            .map_err(|_| DurableStoreError::Unavailable),
        Err(_) => Err(DurableStoreError::Unavailable),
    }
}

fn validate_regular_private_file(file: &File, owner_uid: u32) -> Result<(), DurableStoreError> {
    let metadata = file
        .metadata()
        .map_err(|_| DurableStoreError::Unavailable)?;
    if !metadata.is_file()
        || metadata.uid() != owner_uid
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(DurableStoreError::Corrupt);
    }
    Ok(())
}

fn ensure_lock_identity_at(
    root: &std::path::Path,
    owner_uid: u32,
    expected_dev: u64,
    expected_ino: u64,
    expected_committed: Option<bool>,
) -> Result<bool, SnapshotStoreError> {
    let mut current = OpenOptions::new()
        .read(true)
        .custom_flags(NOFOLLOW)
        .open(root.join(LOCK_FILE))
        .map_err(|_| SnapshotStoreError::Unavailable)?;
    validate_regular_private_file(&current, owner_uid)
        .map_err(|_| SnapshotStoreError::Unavailable)?;
    let path = current
        .metadata()
        .map_err(|_| SnapshotStoreError::Unavailable)?;
    let committed = read_marker(&mut current).map_err(|_| SnapshotStoreError::Unavailable)?;
    if path.dev() != expected_dev
        || path.ino() != expected_ino
        || expected_committed.is_some_and(|expected| expected != committed)
    {
        return Err(SnapshotStoreError::Unavailable);
    }
    Ok(committed)
}

fn read_marker(lock: &mut File) -> Result<bool, DurableStoreError> {
    lock.seek(SeekFrom::Start(0))
        .map_err(|_| DurableStoreError::Unavailable)?;
    let mut bytes = Vec::new();
    lock.take(9)
        .read_to_end(&mut bytes)
        .map_err(|_| DurableStoreError::Unavailable)?;
    if bytes.as_slice() == FRESH_MARKER {
        Ok(false)
    } else if bytes.as_slice() == COMMITTED_MARKER {
        Ok(true)
    } else {
        Err(DurableStoreError::Corrupt)
    }
}

fn read_state_bytes(
    root: &std::path::Path,
    owner_uid: u32,
) -> Result<Option<Vec<u8>>, DurableStoreError> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(NOFOLLOW)
        .open(root.join(STATE_FILE))
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(DurableStoreError::Unavailable),
    };
    validate_regular_private_file(&file, owner_uid)?;
    let metadata = file
        .metadata()
        .map_err(|_| DurableStoreError::Unavailable)?;
    if metadata.len() > MAX_STORE_BYTES {
        return Err(DurableStoreError::Corrupt);
    }
    let mut bytes = Vec::new();
    file.take(MAX_STORE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DurableStoreError::Unavailable)?;
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err(DurableStoreError::Corrupt);
    }
    Ok(Some(bytes))
}

fn read_envelope(
    root: &std::path::Path,
    owner_uid: u32,
) -> Result<Option<Envelope>, SnapshotStoreError> {
    read_state_bytes(root, owner_uid)
        .map_err(|_| SnapshotStoreError::Unavailable)?
        .map(|bytes| Envelope::from_bytes(&bytes).map_err(|_| SnapshotStoreError::Unavailable))
        .transpose()
}

fn current_at_durable_high_water(
    root: &std::path::Path,
    owner_uid: u32,
    committed: bool,
    runtime_operation: Option<C1RecoveryOperationIdentity>,
) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
    let high_water =
        read_high_water(root, owner_uid).map_err(|_| SnapshotStoreError::Unavailable)?;
    let envelope = read_envelope(root, owner_uid)?;
    if let Some(operation) = runtime_operation {
        let exact_runtime_active = envelope.as_ref().is_some_and(|envelope| {
            envelope
                .recovery_journal
                .record
                .as_ref()
                .is_some_and(|record| {
                    record.operation_id.get() == operation.operation_id().get()
                        && record.binding.digest_bytes() == operation.binding_digest()
                        && matches!(record.phase, RecoveryJournalPhase::RuntimeActive)
                })
        });
        if !exact_runtime_active {
            return Err(SnapshotStoreError::Unavailable);
        }
    } else if envelope
        .as_ref()
        .is_some_and(|envelope| envelope.recovery_journal.is_quarantined())
    {
        return Err(SnapshotStoreError::Unavailable);
    }
    match (committed, high_water, envelope) {
        (false, None, None) => Ok(None),
        (true, Some(high_water), Some(envelope)) => {
            let saved_high_water = envelope
                .high_water_version
                .ok_or(SnapshotStoreError::Unavailable)?;
            if saved_high_water > high_water {
                return Err(SnapshotStoreError::Unavailable);
            }
            Ok(envelope
                .current
                .filter(|current| current.version == high_water)
                .map(|current| StoredSnapshot {
                    bytes: current.bytes,
                    version: SnapshotVersion::new(current.version),
                }))
        }
        _ => Err(SnapshotStoreError::Unavailable),
    }
}

fn read_high_water(
    root: &std::path::Path,
    owner_uid: u32,
) -> Result<Option<u64>, DurableStoreError> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(NOFOLLOW)
        .open(root.join(HIGH_WATER_FILE))
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(DurableStoreError::Unavailable),
    };
    validate_regular_private_file(&file, owner_uid)?;
    let metadata = file
        .metadata()
        .map_err(|_| DurableStoreError::Unavailable)?;
    if metadata.len() > MAX_HIGH_WATER_BYTES {
        return Err(DurableStoreError::Corrupt);
    }
    let mut bytes = Vec::new();
    file.take(MAX_HIGH_WATER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| DurableStoreError::Unavailable)?;
    if bytes.len() as u64 > MAX_HIGH_WATER_BYTES {
        return Err(DurableStoreError::Corrupt);
    }
    let record: HighWaterRecord =
        serde_json::from_slice(&bytes).map_err(|_| DurableStoreError::Corrupt)?;
    if record.schema != HIGH_WATER_SCHEMA || record.version == 0 {
        return Err(DurableStoreError::Corrupt);
    }
    Ok(Some(record.version))
}

fn ensure_only_lock_file(root: &std::path::Path) -> Result<(), DurableStoreError> {
    let entries = fs::read_dir(root).map_err(|_| DurableStoreError::Unavailable)?;
    for entry in entries {
        let entry = entry.map_err(|_| DurableStoreError::Unavailable)?;
        if entry.file_name() != LOCK_FILE {
            return Err(DurableStoreError::Corrupt);
        }
    }
    Ok(())
}

fn ensure_only_lock_and_state_file(root: &std::path::Path) -> Result<(), DurableStoreError> {
    let entries = fs::read_dir(root).map_err(|_| DurableStoreError::Unavailable)?;
    for entry in entries {
        let entry = entry.map_err(|_| DurableStoreError::Unavailable)?;
        if !matches!(entry.file_name().to_str(), Some(LOCK_FILE | STATE_FILE)) {
            return Err(DurableStoreError::Corrupt);
        }
    }
    Ok(())
}

fn remove_stale_pending_files(
    root: &std::path::Path,
    directory: &File,
    owner_uid: u32,
) -> Result<(), DurableStoreError> {
    let entries = fs::read_dir(root).map_err(|_| DurableStoreError::Unavailable)?;
    let mut removed_any = false;
    for entry in entries {
        let entry = entry.map_err(|_| DurableStoreError::Unavailable)?;
        let name = entry.file_name();
        let Some(rest) = name
            .to_str()
            .and_then(|name| name.strip_prefix(".pending-"))
        else {
            continue;
        };
        let mut fields = rest.split('-');
        let valid_name = fields
            .next()
            .and_then(|field| field.parse::<u32>().ok())
            .is_some_and(|pid| pid != 0)
            && fields
                .next()
                .and_then(|field| field.parse::<u64>().ok())
                .is_some()
            && fields
                .next()
                .and_then(|field| field.parse::<u64>().ok())
                .is_some()
            && fields.next().is_none();
        if !valid_name {
            continue;
        }

        let path = entry.path();
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(NOFOLLOW)
            .open(&path)
            .map_err(|_| DurableStoreError::Unavailable)?;
        validate_regular_private_file(&file, owner_uid)?;
        let metadata = file
            .metadata()
            .map_err(|_| DurableStoreError::Unavailable)?;
        if metadata.permissions().mode() & 0o7777 != 0o600 || metadata.nlink() != 1 {
            return Err(DurableStoreError::Corrupt);
        }
        drop(file);
        fs::remove_file(path).map_err(|_| DurableStoreError::Unavailable)?;
        removed_any = true;
    }
    if removed_any {
        directory
            .sync_all()
            .map_err(|_| DurableStoreError::Unavailable)?;
    }
    Ok(())
}

fn create_pending(
    root: &std::path::Path,
    version: u64,
) -> Result<(PathBuf, File), SnapshotStoreError> {
    for _ in 0..1024 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!(
            ".pending-{}-{version}-{sequence}",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(NOFOLLOW)
            .open(&path)
        {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(SnapshotStoreError::Unavailable),
        }
    }
    Err(SnapshotStoreError::Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::IdentitySnapshot;
    use crate::store::IdentitySnapshotStore;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::os::unix::process::ExitStatusExt;
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "nagi-c1-durable-store-{}-{sequence}",
                std::process::id()
            ));
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700).create(&path).unwrap();
            Self(path)
        }

        fn new_under_worktree_target() -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let parent = std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf();
            let path = parent.join(format!(
                "nagi-c1-process-store-{}-{sequence}",
                std::process::id()
            ));
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700).create(&path).unwrap();
            Self(path)
        }

        fn open_directory(&self) -> File {
            File::open(&self.0).unwrap()
        }

        fn state_path(&self) -> PathBuf {
            self.0.join(STATE_FILE)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn snapshot(revision: u64) -> Vec<u8> {
        let mut snapshot = IdentitySnapshot::empty();
        snapshot.revision = revision;
        snapshot.to_json().unwrap()
    }

    fn write_tampered_envelope(path: &std::path::Path, mutate: impl FnOnce(&mut Envelope)) {
        let bytes = fs::read(path).unwrap();
        let mut envelope = Envelope::from_bytes(&bytes).unwrap();
        mutate(&mut envelope);
        fs::write(path, envelope.serialized().unwrap()).unwrap();
    }

    fn exit_child_at_store_boundary(directory: &TestDirectory, boundary: &str) {
        const CHILD_TEST: &str =
            "store::linux::tests::child_process_exits_at_selected_store_boundary";
        const DIRECTORY_ENV: &str = "NAGI_C1_DURABLE_CHILD_DIRECTORY";
        const BOUNDARY_ENV: &str = "NAGI_C1_DURABLE_CHILD_BOUNDARY";

        let output = Command::new(std::env::current_exe().unwrap())
            .arg("--exact")
            .arg(CHILD_TEST)
            .arg("--nocapture")
            .env(DIRECTORY_ENV, &directory.0)
            .env(BOUNDARY_ENV, boundary)
            .current_dir(&directory.0)
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(86),
            "child did not exit at the store boundary"
        );
        assert_eq!(output.status.signal(), None);
    }

    fn assert_one_pending_file(directory: &TestDirectory) {
        let pending = fs::read_dir(&directory.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".pending-"))
            .count();
        assert_eq!(pending, 1);
    }

    fn assert_no_pending_files(directory: &TestDirectory) {
        let pending = fs::read_dir(&directory.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".pending-"))
            .count();
        assert_eq!(pending, 0);
    }

    fn committed_directory() -> TestDirectory {
        let directory = TestDirectory::new_under_worktree_target();
        let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        store.commit_atomic(None, &snapshot(0)).unwrap();
        drop(store);
        directory
    }

    #[test]
    fn recovery_intent_save_failures_poison_or_preserve_pending_state() {
        for (fault, intent_may_have_committed) in
            [(Fault::PartialWrite, false), (Fault::AfterRename, true)]
        {
            let directory = committed_directory();
            let binding = RecoveryBinding::for_test([0x51; 32]);
            let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
            store.set_fault_for_test(fault);

            let stage_result = store.begin_recovery_intent(binding.clone());
            assert_eq!(stage_result, Err(RecoveryJournalError::Unavailable));
            assert_eq!(
                store.recovery_quarantined(),
                Err(SnapshotStoreError::Unavailable),
                "an uncertain write poisons the current store instance"
            );
            drop(store);

            let mut reopened =
                LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
            assert_eq!(
                reopened.recovery_quarantined(),
                Ok(intent_may_have_committed),
                "restart either sees no admitted intent or preserves its quarantine"
            );
            if intent_may_have_committed {
                assert_eq!(
                    reopened.recovery_journal_record().unwrap().unwrap().phase,
                    RecoveryJournalPhase::Intent
                );
                assert_eq!(
                    reopened.begin_recovery_intent(binding.clone()),
                    Err(RecoveryJournalError::Quarantined)
                );
            } else {
                let _next_permit = reopened.begin_recovery_intent(binding.clone()).unwrap();
            }
        }
    }

    fn pending_test_path(directory: &TestDirectory) -> PathBuf {
        directory
            .0
            .join(format!(".pending-{}-1-1", std::process::id()))
    }

    #[test]
    fn stale_pending_cleanup_ignores_malformed_names() {
        let directory = committed_directory();
        let malformed = directory.0.join(".pending-not-a-store-temp");
        fs::write(&malformed, b"keep").unwrap();

        assert!(LinuxIdentitySnapshotStore::open(directory.open_directory()).is_ok());
        assert_eq!(fs::read(malformed).unwrap(), b"keep");
    }

    #[test]
    fn stale_pending_cleanup_rejects_symlinks_and_hardlinks_without_deleting_them() {
        let directory = committed_directory();
        let target = directory.0.join("sentinel");
        let pending = pending_test_path(&directory);
        fs::write(&target, b"preserve target").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink("sentinel", &pending).unwrap();

        assert_eq!(
            LinuxIdentitySnapshotStore::open(directory.open_directory()).err(),
            Some(DurableStoreError::Unavailable)
        );
        assert!(fs::symlink_metadata(&pending)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(&target).unwrap(), b"preserve target");
        fs::remove_file(&pending).unwrap();

        fs::hard_link(&target, &pending).unwrap();
        assert_eq!(
            LinuxIdentitySnapshotStore::open(directory.open_directory()).err(),
            Some(DurableStoreError::Corrupt)
        );
        assert!(pending.is_file());
        assert!(target.is_file());
        assert_eq!(
            fs::metadata(&target).unwrap().nlink(),
            2,
            "the rejected hardlink must not be unlinked"
        );
        assert_eq!(fs::read(&target).unwrap(), b"preserve target");
    }

    #[test]
    fn stale_pending_cleanup_rejects_owner_and_mode_mismatches_without_deleting() {
        for mode in [0o400, 0o644] {
            let directory = committed_directory();
            let pending = pending_test_path(&directory);
            fs::write(&pending, b"keep").unwrap();
            fs::set_permissions(&pending, fs::Permissions::from_mode(mode)).unwrap();

            assert_eq!(
                LinuxIdentitySnapshotStore::open(directory.open_directory()).err(),
                Some(DurableStoreError::Corrupt)
            );
            assert_eq!(fs::read(&pending).unwrap(), b"keep");
        }

        let directory = committed_directory();
        let pending = pending_test_path(&directory);
        fs::write(&pending, b"keep").unwrap();
        fs::set_permissions(&pending, fs::Permissions::from_mode(0o600)).unwrap();
        let directory_handle = directory.open_directory();
        let owner_uid = directory_handle.metadata().unwrap().uid();
        let mismatched_owner_uid = if owner_uid == u32::MAX {
            owner_uid - 1
        } else {
            owner_uid + 1
        };

        assert_eq!(
            remove_stale_pending_files(&directory.0, &directory_handle, mismatched_owner_uid),
            Err(DurableStoreError::Corrupt)
        );
        assert_eq!(fs::read(&pending).unwrap(), b"keep");
    }

    #[test]
    fn child_process_exits_at_selected_store_boundary() {
        const DIRECTORY_ENV: &str = "NAGI_C1_DURABLE_CHILD_DIRECTORY";
        const BOUNDARY_ENV: &str = "NAGI_C1_DURABLE_CHILD_BOUNDARY";

        let (Ok(directory), Ok(boundary)) =
            (std::env::var(DIRECTORY_ENV), std::env::var(BOUNDARY_ENV))
        else {
            return;
        };
        let mut store = LinuxIdentitySnapshotStore::open(File::open(directory).unwrap()).unwrap();
        let fault = match boundary.as_str() {
            "during-high-water-write" => Fault::ProcessExitDuringHighWaterWrite,
            "after-high-water" => Fault::ProcessExitAfterHighWater,
            "during-envelope-write" => Fault::ProcessExitDuringEnvelopeWrite,
            other => panic!("unknown child boundary: {other}"),
        };
        let expected_version = store.load_current_state().unwrap().high_water_version;
        store.set_fault_for_test(fault);
        let next_revision = expected_version.map_or(0, |version| version.get() + 1);
        let _ = store.commit_atomic(expected_version, &snapshot(next_revision));
        panic!("configured process exit boundary was not reached");
    }

    #[test]
    fn process_exit_during_first_high_water_write_fails_closed() {
        let directory = TestDirectory::new_under_worktree_target();
        exit_child_at_store_boundary(&directory, "during-high-water-write");
        assert!(!directory.state_path().is_file());
        assert_one_pending_file(&directory);
        assert_eq!(
            LinuxIdentitySnapshotStore::open(directory.open_directory()).err(),
            Some(DurableStoreError::Corrupt)
        );
        assert_no_pending_files(&directory);
    }

    #[test]
    fn process_exit_during_high_water_update_keeps_the_previous_commit() {
        let directory = TestDirectory::new_under_worktree_target();
        let first = snapshot(0);
        let second = snapshot(1);
        let third = snapshot(2);
        let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        store.commit_atomic(None, &first).unwrap();
        store
            .commit_atomic(Some(SnapshotVersion::new(1)), &second)
            .unwrap();
        drop(store);

        exit_child_at_store_boundary(&directory, "during-high-water-write");
        assert_one_pending_file(&directory);
        let mut reopened = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        assert_no_pending_files(&directory);
        assert_eq!(
            reopened.load_current_state().unwrap(),
            SnapshotStoreState {
                current: Some(StoredSnapshot {
                    bytes: second,
                    version: SnapshotVersion::new(2),
                }),
                high_water_version: Some(SnapshotVersion::new(2)),
            }
        );
        assert_eq!(
            reopened.commit_atomic(Some(SnapshotVersion::new(2)), &third),
            Ok(SnapshotVersion::new(3))
        );
    }

    #[test]
    fn process_exit_after_reservation_or_during_envelope_write_skips_generation() {
        for boundary in ["after-high-water", "during-envelope-write"] {
            let directory = TestDirectory::new_under_worktree_target();
            let first = snapshot(0);
            let second = snapshot(1);
            let fourth = snapshot(3);
            let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
            store.commit_atomic(None, &first).unwrap();
            store
                .commit_atomic(Some(SnapshotVersion::new(1)), &second)
                .unwrap();
            drop(store);

            exit_child_at_store_boundary(&directory, boundary);
            if boundary == "during-envelope-write" {
                assert_one_pending_file(&directory);
            }
            let mut reopened =
                LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
            assert_no_pending_files(&directory);
            assert_eq!(
                reopened.load_current_state().unwrap(),
                SnapshotStoreState {
                    current: None,
                    high_water_version: Some(SnapshotVersion::new(3)),
                }
            );
            let last_good = reopened.load_last_known_good().unwrap().unwrap();
            assert_eq!(last_good.bytes, second);
            assert_eq!(last_good.version, SnapshotVersion::new(2));
            assert_eq!(
                reopened.commit_atomic(Some(SnapshotVersion::new(3)), &fourth),
                Ok(SnapshotVersion::new(4))
            );
        }
    }

    #[test]
    fn first_commit_restarts_with_a_persistent_high_water_and_exact_lkg() {
        let directory = TestDirectory::new();
        let first = snapshot(0);
        let second = snapshot(1);
        let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        assert_eq!(store.load_current_state().unwrap().high_water_version, None);
        assert_eq!(
            store.commit_atomic(None, &first),
            Ok(SnapshotVersion::new(1))
        );
        assert_eq!(
            store.commit_atomic(None, &second),
            Err(SnapshotStoreError::Conflict)
        );
        drop(store);

        let mut reopened = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        assert_eq!(
            reopened.load_current_state().unwrap(),
            SnapshotStoreState {
                current: Some(StoredSnapshot {
                    bytes: first.clone(),
                    version: SnapshotVersion::new(1),
                }),
                high_water_version: Some(SnapshotVersion::new(1)),
            }
        );
        assert_eq!(
            reopened.commit_atomic(Some(SnapshotVersion::new(1)), &second),
            Ok(SnapshotVersion::new(2))
        );
        let last_good = reopened.load_last_known_good().unwrap().unwrap();
        assert_eq!(last_good.version, SnapshotVersion::new(1));
        assert_eq!(last_good.bytes, first);
    }

    #[test]
    fn corrupt_current_keeps_the_exact_lkg_but_never_relables_it_as_current() {
        let directory = TestDirectory::new();
        let first = snapshot(0);
        let second = snapshot(1);
        let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        store.commit_atomic(None, &first).unwrap();
        store
            .commit_atomic(Some(SnapshotVersion::new(1)), &second)
            .unwrap();
        store
            .replace(Envelope {
                schema: SCHEMA,
                high_water_version: Some(2),
                current: Some(SavedPayload {
                    version: 2,
                    bytes: b"broken current payload".to_vec(),
                }),
                last_known_good: Some(SavedPayload {
                    version: 1,
                    bytes: first.clone(),
                }),
                recovery_journal: RecoveryJournal::default(),
            })
            .unwrap();

        let current = store.load_current_state().unwrap();
        assert_eq!(current.high_water_version, Some(SnapshotVersion::new(2)));
        assert_eq!(
            current.current,
            Some(StoredSnapshot {
                bytes: b"broken current payload".to_vec(),
                version: SnapshotVersion::new(2),
            })
        );
        let last_good = store.load_last_known_good().unwrap().unwrap();
        assert_eq!(last_good.bytes, first);
        assert_eq!(last_good.version, SnapshotVersion::new(1));
    }

    #[test]
    fn missing_state_after_a_commit_is_not_reinterpreted_as_an_empty_store() {
        let directory = TestDirectory::new();
        let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        store.commit_atomic(None, &snapshot(0)).unwrap();
        drop(store);
        fs::remove_file(directory.state_path()).unwrap();
        assert_eq!(
            LinuxIdentitySnapshotStore::open(directory.open_directory()).err(),
            Some(DurableStoreError::Corrupt)
        );
    }

    #[test]
    fn writer_lock_rejects_a_second_live_issuer() {
        let directory = TestDirectory::new();
        let store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        assert_eq!(
            LinuxIdentitySnapshotStore::open(directory.open_directory()).err(),
            Some(DurableStoreError::Locked)
        );
        drop(store);
        assert!(LinuxIdentitySnapshotStore::open(directory.open_directory()).is_ok());
    }

    #[test]
    fn current_reader_fails_closed_if_the_named_writer_lock_is_replaced_or_removed() {
        let directory = TestDirectory::new();
        let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        store.commit_atomic(None, &snapshot(0)).unwrap();
        let reader = store.current_snapshot_reader().unwrap();
        assert!(reader().unwrap().is_some());
        fs::remove_file(directory.0.join(LOCK_FILE)).unwrap();
        assert_eq!(reader(), Err(SnapshotStoreError::Unavailable));
    }

    #[test]
    fn current_reader_fails_closed_if_the_commit_marker_is_rolled_back() {
        let directory = TestDirectory::new();
        let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        store.commit_atomic(None, &snapshot(0)).unwrap();
        let reader = store.current_snapshot_reader().unwrap();
        assert!(reader().unwrap().is_some());
        fs::write(directory.0.join(LOCK_FILE), FRESH_MARKER).unwrap();
        assert_eq!(reader(), Err(SnapshotStoreError::Unavailable));
    }

    #[test]
    fn current_reader_tracks_the_first_commit_from_a_fresh_store() {
        let directory = TestDirectory::new();
        let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        let reader = store.current_snapshot_reader().unwrap();
        assert_eq!(reader(), Ok(None));
        store.commit_atomic(None, &snapshot(0)).unwrap();
        assert!(reader().unwrap().is_some());
    }

    #[test]
    fn restoring_an_older_valid_envelope_never_restores_its_authority_or_generation() {
        let directory = TestDirectory::new();
        let first = snapshot(0);
        let second = snapshot(1);
        let third = snapshot(2);
        let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        store.commit_atomic(None, &first).unwrap();
        let older_envelope = fs::read(directory.state_path()).unwrap();
        store
            .commit_atomic(Some(SnapshotVersion::new(1)), &second)
            .unwrap();
        drop(store);

        fs::write(directory.state_path(), older_envelope).unwrap();
        let mut reopened = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        let current = reopened.load_current_state().unwrap();
        assert_eq!(current.high_water_version, Some(SnapshotVersion::new(2)));
        assert_eq!(current.current, None);
        let last_good = reopened.load_last_known_good().unwrap().unwrap();
        assert_eq!(last_good.version, SnapshotVersion::new(1));
        assert_eq!(last_good.bytes, first);
        assert_eq!(
            reopened.commit_atomic(Some(SnapshotVersion::new(2)), &third),
            Ok(SnapshotVersion::new(3))
        );
    }

    #[test]
    fn a_missing_independent_high_water_is_corruption_after_commit() {
        let directory = TestDirectory::new();
        let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        store.commit_atomic(None, &snapshot(0)).unwrap();
        drop(store);
        fs::remove_file(directory.0.join(HIGH_WATER_FILE)).unwrap();
        assert_eq!(
            LinuxIdentitySnapshotStore::open(directory.open_directory()).err(),
            Some(DurableStoreError::Corrupt)
        );
    }

    #[test]
    fn an_interrupted_envelope_write_keeps_the_reserved_generation_after_restart() {
        let directory = TestDirectory::new();
        let first = snapshot(0);
        let second = snapshot(1);
        let third = snapshot(2);
        let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        store.commit_atomic(None, &first).unwrap();
        store.set_fault_for_test(Fault::AfterHighWater);
        assert_eq!(
            store.commit_atomic(Some(SnapshotVersion::new(1)), &second),
            Err(SnapshotStoreError::Unavailable)
        );
        assert_eq!(
            store.commit_atomic(Some(SnapshotVersion::new(2)), &third),
            Err(SnapshotStoreError::Unavailable)
        );
        drop(store);

        let mut reopened = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        let current = reopened.load_current_state().unwrap();
        assert_eq!(current.high_water_version, Some(SnapshotVersion::new(2)));
        assert_eq!(current.current, None);
        let last_good = reopened.load_last_known_good().unwrap().unwrap();
        assert_eq!(last_good.version, SnapshotVersion::new(1));
        assert_eq!(last_good.bytes, first);
        assert_eq!(
            reopened.commit_atomic(Some(SnapshotVersion::new(2)), &third),
            Ok(SnapshotVersion::new(3))
        );
    }

    #[test]
    fn external_state_change_is_a_cas_conflict_and_does_not_update_the_cache() {
        let directory = TestDirectory::new();
        let first = snapshot(0);
        let second = snapshot(1);
        let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
        store.commit_atomic(None, &first).unwrap();
        write_tampered_envelope(&directory.state_path(), |state| {
            state.current.as_mut().unwrap().bytes = second.clone();
        });
        assert_eq!(
            store.commit_atomic(Some(SnapshotVersion::new(1)), &second),
            Err(SnapshotStoreError::Conflict)
        );
    }

    #[test]
    fn unknown_commit_outcomes_poison_the_instance_and_reopen_never_reuses_a_generation() {
        for (fault, durable_generation) in [
            (Fault::PartialWrite, None),
            (Fault::BeforeRename, None),
            (Fault::AfterRename, Some(SnapshotVersion::new(1))),
        ] {
            let directory = TestDirectory::new();
            let mut store = LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
            store.set_fault_for_test(fault);
            assert_eq!(
                store.commit_atomic(None, &snapshot(0)),
                Err(SnapshotStoreError::Unavailable)
            );
            assert_eq!(
                store.commit_atomic(None, &snapshot(1)),
                Err(SnapshotStoreError::Unavailable)
            );
            drop(store);
            match durable_generation {
                None => assert_eq!(
                    LinuxIdentitySnapshotStore::open(directory.open_directory()).err(),
                    Some(DurableStoreError::Corrupt)
                ),
                Some(generation) => {
                    let reopened =
                        LinuxIdentitySnapshotStore::open(directory.open_directory()).unwrap();
                    assert_eq!(
                        reopened.load_current_state().unwrap().high_water_version,
                        Some(generation)
                    );
                }
            }
        }
    }

    #[test]
    fn public_or_symlinked_store_files_are_rejected() {
        let directory = TestDirectory::new();
        fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            LinuxIdentitySnapshotStore::open(directory.open_directory()).err(),
            Some(DurableStoreError::Unavailable)
        );

        let symlink_directory = TestDirectory::new();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/dev/null", symlink_directory.0.join(LOCK_FILE)).unwrap();
        assert_eq!(
            LinuxIdentitySnapshotStore::open(symlink_directory.open_directory()).err(),
            Some(DurableStoreError::Unavailable)
        );
    }
}
