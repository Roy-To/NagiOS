use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use crate::authorization::{JobOwner, OwnerProfileRef, PrincipalRef};
use crate::model::{
    ConstraintError, HandlerRef, IdempotencyKey, JobId, JobRecord, JobState, OpaqueRef,
    TimestampMillis,
};

#[derive(Clone, Debug, Eq, Hash, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DedupeScope {
    principal: PrincipalRef,
    profile: Option<OwnerProfileRef>,
    provider: OpaqueRef,
    handler: OpaqueRef,
    handler_version: u16,
    key: IdempotencyKey,
}

impl DedupeScope {
    pub fn new(owner: &JobOwner, handler: &HandlerRef, key: IdempotencyKey) -> Self {
        Self {
            principal: owner.principal().clone(),
            profile: owner.profile().cloned(),
            provider: handler.provider().clone(),
            handler: handler.name().clone(),
            handler_version: handler.version(),
            key,
        }
    }

    pub fn principal(&self) -> &PrincipalRef {
        &self.principal
    }

    pub fn profile(&self) -> Option<&OwnerProfileRef> {
        self.profile.as_ref()
    }

    pub fn provider(&self) -> &OpaqueRef {
        &self.provider
    }

    pub fn handler(&self) -> &OpaqueRef {
        &self.handler
    }

    pub const fn handler_version(&self) -> u16 {
        self.handler_version
    }

    pub fn key(&self) -> &IdempotencyKey {
        &self.key
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InsertOutcome {
    Inserted(JobRecord),
    Existing(JobRecord),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobStoreError {
    Unavailable,
    Corrupt,
    Conflict,
    DuplicateId,
    Capacity,
    Missing,
    InvalidRecord(ConstraintError),
}

/// Atomic metadata store boundary. Implementations must make insert+dedupe,
/// revision updates, and claims transactional. A durable backend is not
/// included in this host-only workstream foundation.
pub trait JobStore: Send + Sync {
    fn insert(
        &self,
        record: JobRecord,
        dedupe: Option<(DedupeScope, TimestampMillis)>,
        now: TimestampMillis,
        queue_capacity: usize,
        record_capacity: usize,
    ) -> Result<InsertOutcome, JobStoreError>;

    fn get(&self, id: &JobId) -> Result<Option<JobRecord>, JobStoreError>;

    fn list(&self) -> Result<Vec<JobRecord>, JobStoreError>;

    fn compare_and_swap(
        &self,
        id: &JobId,
        expected_revision: u64,
        replacement: JobRecord,
    ) -> Result<JobRecord, JobStoreError>;

    /// Atomically verifies revision, queue state, and both concurrency bounds
    /// before marking one job running. This is the multi-worker store contract.
    fn claim(
        &self,
        id: &JobId,
        expected_revision: u64,
        now: TimestampMillis,
        global_limit: usize,
        per_owner_limit: usize,
    ) -> Result<Option<JobRecord>, JobStoreError>;
}

#[derive(Clone, Default)]
pub struct InMemoryJobStore {
    inner: Arc<Mutex<StoreInner>>,
}

#[derive(Default)]
struct StoreInner {
    records: BTreeMap<JobId, JobRecord>,
    dedupe: HashMap<DedupeScope, DedupeEntry>,
    next_sequence: u64,
    fail_next_write: bool,
    fail_reads: bool,
}

#[derive(Clone)]
struct DedupeEntry {
    job_id: JobId,
    retained_until: TimestampMillis,
}

impl InMemoryJobStore {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn fail_next_write_for_test(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.fail_next_write = true;
        }
    }

    #[cfg(test)]
    pub(crate) fn fail_reads_for_test(&self, fail: bool) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.fail_reads = fail;
        }
    }

    #[cfg(test)]
    pub(crate) fn seed_state_for_test(&self, id: &JobId, state: JobState) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(record) = inner.records.get_mut(id) {
                record.seed_state_for_test(state);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn seed_dependency_for_test(
        &self,
        id: &JobId,
        dependency: crate::model::DependencyRef,
    ) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(record) = inner.records.get_mut(id) {
                record.seed_dependency_for_test(dependency);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn seed_checkpoint_for_test(
        &self,
        id: &JobId,
        checkpoint: crate::model::CheckpointRef,
    ) {
        if let Ok(mut inner) = self.inner.lock() {
            if let Some(record) = inner.records.get_mut(id) {
                record.seed_checkpoint_for_test(checkpoint);
            }
        }
    }

    fn fail_write(inner: &mut StoreInner) -> Result<(), JobStoreError> {
        if inner.fail_next_write {
            inner.fail_next_write = false;
            Err(JobStoreError::Unavailable)
        } else {
            Ok(())
        }
    }
}

impl JobStore for InMemoryJobStore {
    fn insert(
        &self,
        mut record: JobRecord,
        dedupe: Option<(DedupeScope, TimestampMillis)>,
        now: TimestampMillis,
        queue_capacity: usize,
        record_capacity: usize,
    ) -> Result<InsertOutcome, JobStoreError> {
        record.validate().map_err(JobStoreError::InvalidRecord)?;
        let mut inner = self.inner.lock().map_err(|_| JobStoreError::Unavailable)?;
        Self::fail_write(&mut inner)?;

        if let Some((scope, retained_until)) = dedupe.as_ref() {
            if let Some(entry) = inner.dedupe.get(scope).cloned() {
                let existing = inner
                    .records
                    .get(&entry.job_id)
                    .ok_or(JobStoreError::Corrupt)?;
                if !existing.state().is_terminal() || entry.retained_until >= now {
                    return Ok(InsertOutcome::Existing(existing.clone()));
                }
                inner.dedupe.remove(scope);
            }
            if *retained_until < now {
                return Err(JobStoreError::InvalidRecord(
                    ConstraintError::InvalidIdentifier,
                ));
            }
        }

        if inner.records.contains_key(record.id()) {
            return Err(JobStoreError::DuplicateId);
        }
        if inner.records.len() >= record_capacity {
            return Err(JobStoreError::Capacity);
        }
        let active_count = inner
            .records
            .values()
            .filter(|saved| saved.is_active())
            .count();
        if active_count >= queue_capacity {
            return Err(JobStoreError::Capacity);
        }

        inner.next_sequence = inner.next_sequence.saturating_add(1);
        record.set_enqueue_sequence(inner.next_sequence);
        record.set_revision(1);
        record.validate().map_err(JobStoreError::InvalidRecord)?;
        let id = record.id().clone();
        inner.records.insert(id.clone(), record.clone());
        if let Some((scope, retained_until)) = dedupe {
            inner.dedupe.insert(
                scope,
                DedupeEntry {
                    job_id: id,
                    retained_until,
                },
            );
        }
        Ok(InsertOutcome::Inserted(record))
    }

    fn get(&self, id: &JobId) -> Result<Option<JobRecord>, JobStoreError> {
        let inner = self.inner.lock().map_err(|_| JobStoreError::Unavailable)?;
        if inner.fail_reads {
            return Err(JobStoreError::Unavailable);
        }
        inner
            .records
            .get(id)
            .map(|record| {
                record.validate().map_err(|_| JobStoreError::Corrupt)?;
                Ok(record.clone())
            })
            .transpose()
    }

    fn list(&self) -> Result<Vec<JobRecord>, JobStoreError> {
        let inner = self.inner.lock().map_err(|_| JobStoreError::Unavailable)?;
        if inner.fail_reads {
            return Err(JobStoreError::Unavailable);
        }
        inner
            .records
            .values()
            .map(|record| {
                record.validate().map_err(|_| JobStoreError::Corrupt)?;
                Ok(record.clone())
            })
            .collect()
    }

    fn compare_and_swap(
        &self,
        id: &JobId,
        expected_revision: u64,
        mut replacement: JobRecord,
    ) -> Result<JobRecord, JobStoreError> {
        replacement
            .validate()
            .map_err(JobStoreError::InvalidRecord)?;
        let mut inner = self.inner.lock().map_err(|_| JobStoreError::Unavailable)?;
        Self::fail_write(&mut inner)?;
        let current = inner.records.get(id).ok_or(JobStoreError::Missing)?;
        if current.revision() != expected_revision || !current.copy_identity_matches(&replacement) {
            return Err(JobStoreError::Conflict);
        }
        replacement.set_revision(expected_revision.saturating_add(1));
        replacement
            .validate()
            .map_err(JobStoreError::InvalidRecord)?;
        inner.records.insert(id.clone(), replacement.clone());
        Ok(replacement)
    }

    fn claim(
        &self,
        id: &JobId,
        expected_revision: u64,
        now: TimestampMillis,
        global_limit: usize,
        per_owner_limit: usize,
    ) -> Result<Option<JobRecord>, JobStoreError> {
        let mut inner = self.inner.lock().map_err(|_| JobStoreError::Unavailable)?;
        Self::fail_write(&mut inner)?;
        let Some(current) = inner.records.get(id) else {
            return Err(JobStoreError::Missing);
        };
        if current.revision() != expected_revision {
            return Err(JobStoreError::Conflict);
        }
        if !matches!(current.state(), JobState::Queued) {
            return Ok(None);
        }
        let owner = current.owner().clone();
        let running = inner
            .records
            .values()
            .filter(|saved| saved.state().occupies_slot());
        let mut global_count = 0;
        let mut owner_count = 0;
        for saved in running {
            global_count += 1;
            if saved.owner() == &owner {
                owner_count += 1;
            }
        }
        if global_count >= global_limit || owner_count >= per_owner_limit {
            return Ok(None);
        }

        let record = inner.records.get_mut(id).expect("record checked above");
        record
            .start_attempt(now)
            .map_err(JobStoreError::InvalidRecord)?;
        record.set_revision(expected_revision.saturating_add(1));
        record.validate().map_err(JobStoreError::InvalidRecord)?;
        Ok(Some(record.clone()))
    }
}
