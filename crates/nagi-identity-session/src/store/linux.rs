use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::model::IdentitySnapshot;
use crate::store::{
    CurrentSnapshotReader, IdentitySnapshotStore, SnapshotStoreError, SnapshotStoreState,
    SnapshotVersion, StoredSnapshot,
};

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
        }
    }

    fn validate(&self) -> Result<(), DurableStoreError> {
        if self.schema != SCHEMA || self.high_water_version == Some(0) {
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
    #[cfg(test)]
    fault: Option<Fault>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Fault {
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
            Some(_) => return Err(DurableStoreError::Corrupt),
            None if committed => return Err(DurableStoreError::Corrupt),
            None => {
                ensure_only_lock_file(&root)?;
                Envelope::empty()
            }
        };
        match (committed, durable_high_water) {
            (false, None) if state.high_water_version.is_none() => {}
            (true, Some(high_water)) => {
                let saved_high_water =
                    state.high_water_version.ok_or(DurableStoreError::Corrupt)?;
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
            #[cfg(test)]
            fault: None,
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
    fn set_fault_for_test(&mut self, fault: Fault) {
        self.fault = Some(fault);
    }
}

impl IdentitySnapshotStore for LinuxIdentitySnapshotStore {
    fn current_snapshot_reader(&self) -> Option<CurrentSnapshotReader> {
        let directory = self.directory.try_clone().ok()?;
        let owner_uid = self.owner_uid;
        let held_lock = self.lock.metadata().ok()?;
        let lock_dev = held_lock.dev();
        let lock_ino = held_lock.ino();
        Some(Arc::new(move || {
            let root = root_for(&directory);
            let marker_committed =
                ensure_lock_identity_at(&root, owner_uid, lock_dev, lock_ino, None)?;
            current_at_durable_high_water(&root, owner_uid, marker_committed)
        }))
    }

    fn load_current_state(&self) -> Result<SnapshotStoreState, SnapshotStoreError> {
        self.ensure_disk()?;
        Ok(SnapshotStoreState {
            current: self.state.current_at_high_water(),
            high_water_version: self.state.high_water_version.map(SnapshotVersion::new),
        })
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
) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
    let high_water =
        read_high_water(root, owner_uid).map_err(|_| SnapshotStoreError::Unavailable)?;
    let envelope = read_envelope(root, owner_uid)?;
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
    let mut entries = fs::read_dir(root).map_err(|_| DurableStoreError::Unavailable)?;
    while let Some(entry) = entries.next() {
        let entry = entry.map_err(|_| DurableStoreError::Unavailable)?;
        if entry.file_name() != LOCK_FILE {
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
        if file
            .metadata()
            .map_err(|_| DurableStoreError::Unavailable)?
            .nlink()
            != 1
        {
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
