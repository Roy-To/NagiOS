use std::collections::BTreeMap;

use crate::model::{LedgerCheckpoint, LedgerRecord, SnapshotId, SnapshotManifest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    SequenceMismatch,
    CheckpointMismatch,
    DuplicateSnapshot,
    MissingParentSnapshot,
    InvalidSnapshot(&'static str),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SequenceMismatch => f.write_str("ledger store sequence mismatch"),
            Self::CheckpointMismatch => f.write_str("ledger store checkpoint mismatch"),
            Self::DuplicateSnapshot => f.write_str("duplicate snapshot id"),
            Self::MissingParentSnapshot => f.write_str("parent snapshot is not present"),
            Self::InvalidSnapshot(reason) => write!(f, "invalid snapshot: {reason}"),
        }
    }
}

impl std::error::Error for StoreError {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoredLedger {
    pub records: Vec<LedgerRecord>,
    pub checkpoint: LedgerCheckpoint,
}

/// A backend must durably append one record and its external checkpoint atomically.
/// The checkpoint should be stored independently enough to detect a truncated tail.
pub trait LedgerStore {
    fn load(&self) -> Result<StoredLedger, StoreError>;
    fn append(
        &mut self,
        record: LedgerRecord,
        checkpoint: LedgerCheckpoint,
    ) -> Result<(), StoreError>;
}

/// Deterministic host fixture. This stores only ledger metadata and references.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InMemoryLedgerStore {
    stored: StoredLedger,
}

impl InMemoryLedgerStore {
    pub fn from_stored(stored: StoredLedger) -> Self {
        Self { stored }
    }
}

impl LedgerStore for InMemoryLedgerStore {
    fn load(&self) -> Result<StoredLedger, StoreError> {
        Ok(self.stored.clone())
    }

    fn append(
        &mut self,
        record: LedgerRecord,
        checkpoint: LedgerCheckpoint,
    ) -> Result<(), StoreError> {
        let expected_sequence = self.stored.records.len() as u64 + 1;
        let prior_hash = self.stored.checkpoint.head_hash.clone();
        if record.sequence != expected_sequence || record.previous_hash != prior_hash {
            return Err(StoreError::SequenceMismatch);
        }
        if checkpoint.last_sequence != record.sequence
            || checkpoint.head_hash.as_deref() != Some(record.record_hash.as_str())
        {
            return Err(StoreError::CheckpointMismatch);
        }
        self.stored.records.push(record);
        self.stored.checkpoint = checkpoint;
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InMemorySnapshotStore {
    manifests: BTreeMap<SnapshotId, SnapshotManifest>,
}

impl InMemorySnapshotStore {
    pub fn insert(&mut self, manifest: SnapshotManifest) -> Result<(), StoreError> {
        manifest
            .validate()
            .map_err(|error| StoreError::InvalidSnapshot(error.0))?;
        if let Some(parent) = &manifest.parent_snapshot {
            if !self.manifests.contains_key(parent) {
                return Err(StoreError::MissingParentSnapshot);
            }
        }
        if self.manifests.contains_key(&manifest.id) {
            return Err(StoreError::DuplicateSnapshot);
        }
        self.manifests.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.manifests.len()
    }

    pub fn is_empty(&self) -> bool {
        self.manifests.is_empty()
    }
}

pub trait SnapshotStore {
    fn get_manifest(&self, id: &SnapshotId) -> Result<Option<SnapshotManifest>, StoreError>;
}

impl SnapshotStore for InMemorySnapshotStore {
    fn get_manifest(&self, id: &SnapshotId) -> Result<Option<SnapshotManifest>, StoreError> {
        Ok(self.manifests.get(id).cloned())
    }
}
