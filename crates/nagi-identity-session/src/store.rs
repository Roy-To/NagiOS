use std::sync::{Arc, RwLock};

use crate::model::IdentitySnapshot;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SnapshotVersion(u64);

impl SnapshotVersion {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredSnapshot {
    pub bytes: Vec<u8>,
    pub version: SnapshotVersion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotStoreError {
    Unavailable,
    Conflict,
}

/// A fresh read of the same committed current store, including mutations by
/// other service instances. It must not use a stale cache or last-known-good
/// snapshot. Readers are observations, not grants or transactions.
pub type CurrentSnapshotReader =
    Arc<dyn Fn() -> Result<Option<StoredSnapshot>, SnapshotStoreError> + Send + Sync>;

/// Implementations must atomically replace the current snapshot and preserve a
/// last-known-good snapshot. No host filesystem implementation is provided.
pub trait IdentitySnapshotStore {
    /// Protected operations need a reader that remains bound to this store.
    /// Returning None preserves source compatibility and denies live identities.
    /// A failed or unavailable reader must never fall back to cached authority.
    fn current_snapshot_reader(&self) -> Option<CurrentSnapshotReader> {
        None
    }

    fn load_current(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError>;
    fn load_last_known_good(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError>;
    fn commit_atomic(
        &mut self,
        expected_version: Option<SnapshotVersion>,
        complete_snapshot: &[u8],
    ) -> Result<SnapshotVersion, SnapshotStoreError>;
}

/// Deterministic store fake for orchestration tests. It has no guest or host
/// file persistence semantics.
#[derive(Debug, Default)]
pub struct InMemorySnapshotStore {
    current: Option<StoredSnapshot>,
    last_known_good: Option<StoredSnapshot>,
    fail_next_commit: bool,
    fail_reads: bool,
    observed: Arc<RwLock<MemoryObservation>>,
}

#[derive(Debug, Default)]
struct MemoryObservation {
    current: Option<StoredSnapshot>,
    unavailable: bool,
}

// This fake has always cloned independent snapshots, not shared stores.
impl Clone for InMemorySnapshotStore {
    fn clone(&self) -> Self {
        Self {
            current: self.current.clone(),
            last_known_good: self.last_known_good.clone(),
            fail_next_commit: self.fail_next_commit,
            fail_reads: self.fail_reads,
            observed: Arc::new(RwLock::new(MemoryObservation {
                current: self.current.clone(),
                unavailable: self.fail_reads,
            })),
        }
    }
}

impl InMemorySnapshotStore {
    pub fn fail_next_commit(&mut self) {
        self.fail_next_commit = true;
    }

    pub fn fail_reads(&mut self) {
        self.fail_reads = true;
        if let Ok(mut observation) = self.observed.write() {
            observation.unavailable = true;
        }
    }

    pub fn corrupt_current_for_test(&mut self, bytes: Vec<u8>) {
        let version = self
            .current
            .as_ref()
            .map(|snapshot| snapshot.version)
            .unwrap_or(SnapshotVersion::new(1));
        self.current = Some(StoredSnapshot { bytes, version });
        if let Ok(mut observation) = self.observed.write() {
            observation.current = self.current.clone();
        }
    }

    pub fn current_bytes(&self) -> Option<&[u8]> {
        self.current
            .as_ref()
            .map(|snapshot| snapshot.bytes.as_slice())
    }
}

impl IdentitySnapshotStore for InMemorySnapshotStore {
    fn current_snapshot_reader(&self) -> Option<CurrentSnapshotReader> {
        let observed = self.observed.clone();
        Some(Arc::new(move || {
            let observation = observed
                .read()
                .map_err(|_| SnapshotStoreError::Unavailable)?;
            if observation.unavailable {
                return Err(SnapshotStoreError::Unavailable);
            }
            Ok(observation.current.clone())
        }))
    }

    fn load_current(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
        if self.fail_reads {
            return Err(SnapshotStoreError::Unavailable);
        }
        Ok(self.current.clone())
    }

    fn load_last_known_good(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
        if self.fail_reads {
            return Err(SnapshotStoreError::Unavailable);
        }
        Ok(self.last_known_good.clone())
    }

    fn commit_atomic(
        &mut self,
        expected_version: Option<SnapshotVersion>,
        complete_snapshot: &[u8],
    ) -> Result<SnapshotVersion, SnapshotStoreError> {
        if self.fail_next_commit {
            self.fail_next_commit = false;
            return Err(SnapshotStoreError::Unavailable);
        }
        IdentitySnapshot::from_json(complete_snapshot)
            .map_err(|_| SnapshotStoreError::Unavailable)?;

        if self.current.as_ref().map(|snapshot| snapshot.version) != expected_version {
            return Err(SnapshotStoreError::Conflict);
        }

        let next_version = match expected_version {
            Some(version) => version
                .get()
                .checked_add(1)
                .ok_or(SnapshotStoreError::Unavailable)?,
            None => 1,
        };
        let next_version = SnapshotVersion::new(next_version);

        // Acquire observation access before changing either saved snapshot.
        // Poisoned observation state must never publish authority.
        let mut observation = self
            .observed
            .write()
            .map_err(|_| SnapshotStoreError::Unavailable)?;
        if let Some(current) = self.current.as_ref() {
            if IdentitySnapshot::from_json(&current.bytes).is_ok() {
                self.last_known_good = Some(current.clone());
            }
        }
        self.current = Some(StoredSnapshot {
            bytes: complete_snapshot.to_vec(),
            version: next_version,
        });
        observation.current = self.current.clone();
        Ok(next_version)
    }
}
