use super::linux_inputs::{entropy_hex, IssuerClock};
use super::{BindingRow, InitialState, PersistentFailure};
use identity::{
    IdentitySnapshot, IdentitySnapshotStore, ProfileKind, ProviderSubjectRef, SnapshotStoreError,
    SnapshotVersion, StoredSnapshot,
};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;

const MAX_BYTES: u64 = 1024 * 1024;
const NOFOLLOW: i32 = 0x20000; // Linux O_NOFOLLOW, never applied to external paths.

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u64,
    snapshot: IdentitySnapshot,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: u32,
    generation: u64,
    issuer_epoch: String,
    floor_millis: u64,
    bindings: Vec<BindingRow>,
    current: Saved,
    previous: Option<Saved>,
}

impl Envelope {
    fn validate(&self) -> Result<(), PersistentFailure> {
        if self.schema != 1
            || self.generation == 0
            || self.current.version == 0
            || self.issuer_epoch.len() != 32
            || !self.issuer_epoch.bytes().all(|b| b.is_ascii_hexdigit())
            || self.bindings.is_empty()
            || self.bindings.len() > 4
        {
            return Err(PersistentFailure::Corrupt);
        }
        self.current
            .snapshot
            .validate()
            .map_err(|_| PersistentFailure::Corrupt)?;
        if let Some(previous) = &self.previous {
            if previous.version >= self.current.version || previous.version == 0 {
                return Err(PersistentFailure::Corrupt);
            }
            previous
                .snapshot
                .validate()
                .map_err(|_| PersistentFailure::Corrupt)?;
        }
        for (index, row) in self.bindings.iter().enumerate() {
            if row.account_name.is_empty()
                || row.account_name.len() > 16
                || ProviderSubjectRef::new(row.provider.clone(), row.subject.clone()).is_err()
                || self.bindings[..index].iter().any(|prior| {
                    prior.account_name == row.account_name
                        || (prior.provider == row.provider && prior.subject == row.subject)
                })
                || !self
                    .current
                    .snapshot
                    .users
                    .iter()
                    .any(|user| user.user_id == row.user)
                || !self.current.snapshot.profiles.iter().any(|profile| {
                    profile.profile_id == row.profile
                        && profile.owner_user_id == Some(row.user)
                        && profile.kind == ProfileKind::Persistent
                })
            {
                return Err(PersistentFailure::Corrupt);
            }
        }
        Ok(())
    }
}

pub(super) struct FileSnapshotStore {
    // An externally authorized directory, kept alive for every /proc/self/fd
    // relative operation. Requires Linux procfs; no arbitrary path fallback.
    directory: File,
    _exclusive: File,
    state: Envelope,
    disk: Option<Vec<u8>>,
    poisoned: bool,
    pub(super) clock: IssuerClock,
    #[cfg(test)]
    pub(super) fault: std::sync::Arc<std::sync::Mutex<Option<Fault>>>,
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(super) enum Fault {
    PartialWrite,
    BeforeRename,
    AfterRename,
}

impl FileSnapshotStore {
    pub(super) fn open(
        directory: File,
        generation: u64,
        initial: Option<InitialState>,
    ) -> Result<Self, PersistentFailure> {
        let metadata = directory
            .metadata()
            .map_err(|_| PersistentFailure::Unavailable)?;
        if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 || generation == 0 {
            return Err(PersistentFailure::Unavailable);
        }
        let root = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd()));
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(NOFOLLOW)
            .open(root.join("issuer.lock"))
            .map_err(|_| PersistentFailure::Unavailable)?;
        lock.try_lock()
            .map_err(|_| PersistentFailure::Unavailable)?;
        if !lock
            .metadata()
            .map_err(|_| PersistentFailure::Unavailable)?
            .is_file()
        {
            return Err(PersistentFailure::Unavailable);
        }
        let disk = read_state(&root)?;
        let mut state: Envelope = match (&disk, initial) {
            (Some(bytes), None) => {
                serde_json::from_slice(bytes).map_err(|_| PersistentFailure::Corrupt)?
            }
            (None, Some(initial)) if initial.generation == generation => Envelope {
                schema: 1,
                generation,
                issuer_epoch: entropy_hex().map_err(|_| PersistentFailure::Unavailable)?,
                floor_millis: 0,
                bindings: initial.bindings,
                current: Saved {
                    version: 1,
                    snapshot: initial.snapshot,
                },
                previous: None,
            },
            (None, Some(_)) => return Err(PersistentFailure::GenerationMismatch),
            _ => return Err(PersistentFailure::Unavailable),
        };
        if state.generation != generation {
            return Err(PersistentFailure::GenerationMismatch);
        }
        state.validate()?;
        let clock = IssuerClock::open(state.floor_millis)
            .map_err(|_| PersistentFailure::ClockUnavailable)?;
        // A fresh process epoch is durably installed before service recovery.
        // This is not an identity, grant, or Linux boot-ID mapping.
        state.issuer_epoch = entropy_hex().map_err(|_| PersistentFailure::Unavailable)?;
        state.floor_millis = clock
            .checked_now()
            .map_err(|_| PersistentFailure::ClockUnavailable)?
            .get();
        let mut store = Self {
            directory,
            _exclusive: lock,
            state: state.clone(),
            disk,
            poisoned: false,
            clock,
            #[cfg(test)]
            fault: Default::default(),
        };
        store
            .replace(state)
            .map_err(|_| PersistentFailure::Unavailable)?;
        Ok(store)
    }

    pub(super) fn bindings(&self) -> &[BindingRow] {
        &self.state.bindings
    }

    fn root(&self) -> PathBuf {
        PathBuf::from(format!("/proc/self/fd/{}", self.directory.as_raw_fd()))
    }

    fn ensure_disk(&self) -> Result<(), SnapshotStoreError> {
        if self.poisoned {
            return Err(SnapshotStoreError::Unavailable);
        }
        if read_state(&self.root()).map_err(|_| SnapshotStoreError::Unavailable)? != self.disk {
            return Err(SnapshotStoreError::Conflict);
        }
        Ok(())
    }

    fn replace(&mut self, state: Envelope) -> Result<(), SnapshotStoreError> {
        self.ensure_disk()?;
        state
            .validate()
            .map_err(|_| SnapshotStoreError::Unavailable)?;
        let bytes = serde_json::to_vec(&state).map_err(|_| SnapshotStoreError::Unavailable)?;
        if bytes.len() as u64 > MAX_BYTES {
            return Err(SnapshotStoreError::Unavailable);
        }
        let path = self.root().join(format!(
            ".pending-{}",
            entropy_hex().map_err(|_| SnapshotStoreError::Unavailable)?
        ));
        // Once I/O starts, any error may have unknown durability. Latch failure
        // and require reopening+SDK recovery, never retry a commit as success.
        let result = (|| -> std::io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(NOFOLLOW)
                .open(&path)?;
            #[cfg(test)]
            if matches!(
                self.fault
                    .lock()
                    .unwrap()
                    .take_if(|f| matches!(f, Fault::PartialWrite)),
                Some(Fault::PartialWrite)
            ) {
                file.write_all(&bytes[..bytes.len() / 2])?;
                return Err(std::io::Error::other("injected interrupted write"));
            }
            file.write_all(&bytes)?;
            file.sync_all()?;
            #[cfg(test)]
            if matches!(
                self.fault
                    .lock()
                    .unwrap()
                    .take_if(|f| matches!(f, Fault::BeforeRename)),
                Some(Fault::BeforeRename)
            ) {
                return Err(std::io::Error::other("injected before rename"));
            }
            std::fs::rename(&path, self.root().join("issuer.json"))?;
            #[cfg(test)]
            if matches!(
                self.fault
                    .lock()
                    .unwrap()
                    .take_if(|f| matches!(f, Fault::AfterRename)),
                Some(Fault::AfterRename)
            ) {
                return Err(std::io::Error::other("injected unknown directory sync"));
            }
            self.directory.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            self.poisoned = true;
            // Only our own unique uncommitted temporary file is removed.
            let _ = std::fs::remove_file(path);
            return Err(SnapshotStoreError::Unavailable);
        }
        self.disk = Some(bytes);
        self.state = state;
        Ok(())
    }
}

fn read_state(root: &std::path::Path) -> Result<Option<Vec<u8>>, PersistentFailure> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(NOFOLLOW)
        .open(root.join("issuer.json"))
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(PersistentFailure::Unavailable),
    };
    let metadata = file
        .metadata()
        .map_err(|_| PersistentFailure::Unavailable)?;
    if !metadata.is_file()
        || metadata.len() > MAX_BYTES
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(PersistentFailure::Corrupt);
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PersistentFailure::Unavailable)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(PersistentFailure::Corrupt);
    }
    Ok(Some(bytes))
}

impl IdentitySnapshotStore for FileSnapshotStore {
    fn load_current(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
        self.ensure_disk()?;
        Ok(Some(StoredSnapshot {
            version: SnapshotVersion::new(self.state.current.version),
            bytes: self
                .state
                .current
                .snapshot
                .to_json()
                .map_err(|_| SnapshotStoreError::Unavailable)?,
        }))
    }
    fn load_last_known_good(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
        self.ensure_disk()?;
        self.state
            .previous
            .as_ref()
            .map(|saved| {
                Ok(StoredSnapshot {
                    version: SnapshotVersion::new(saved.version),
                    bytes: saved
                        .snapshot
                        .to_json()
                        .map_err(|_| SnapshotStoreError::Unavailable)?,
                })
            })
            .transpose()
    }
    fn commit_atomic(
        &mut self,
        expected: Option<SnapshotVersion>,
        bytes: &[u8],
    ) -> Result<SnapshotVersion, SnapshotStoreError> {
        self.ensure_disk()?;
        if expected != Some(SnapshotVersion::new(self.state.current.version)) {
            return Err(SnapshotStoreError::Conflict);
        }
        let snapshot =
            IdentitySnapshot::from_json(bytes).map_err(|_| SnapshotStoreError::Unavailable)?;
        let mut next = self.state.clone();
        next.previous = Some(next.current.clone());
        let version = next
            .current
            .version
            .checked_add(1)
            .ok_or(SnapshotStoreError::Unavailable)?;
        next.current = Saved { version, snapshot };
        next.floor_millis = self
            .clock
            .checked_now()
            .map_err(|_| SnapshotStoreError::Unavailable)?
            .get();
        self.replace(next)?;
        Ok(SnapshotVersion::new(version))
    }
}
