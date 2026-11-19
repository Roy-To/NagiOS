use std::num::NonZeroU64;
use std::sync::{Arc, Mutex, RwLock};

use crate::model::IdentitySnapshot;
use serde::{Deserialize, Serialize};

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{DurableStoreError, LinuxIdentitySnapshotStore};

#[cfg(all(test, target_os = "linux"))]
pub(crate) use linux::Fault as DurableStoreFault;

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

/// One atomic observation of the rollbackable current payload and its
/// independent, monotonically increasing store CAS revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotStoreState {
    pub current: Option<StoredSnapshot>,
    /// Highest version ever committed. This must survive current-payload loss
    /// and restoration from last-known-good data; it is not derived from either
    /// snapshot's embedded version. None is valid only when the adapter can
    /// prove no commit has ever succeeded. If the high-water was lost or its
    /// history is uncertain, adapters must return an error rather than describe
    /// the store as empty, because generation-scoped SessionIds could repeat.
    pub high_water_version: Option<SnapshotVersion>,
}

/// Opaque per-store recovery transaction identity. The eventual C1 adapter must
/// derive this only from authenticated canonical service mapping and exact
/// owner generation inputs; descriptor text or caller-supplied IDs are not a
/// valid source.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct RecoveryOperationId(u64);

impl RecoveryOperationId {
    #[cfg(test)]
    pub(crate) const fn for_test(value: u64) -> Self {
        Self(value)
    }

    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

/// One-use capability returned only after the matching durable Intent has
/// been committed. `IdentityService` retains it through every pre-effect
/// check, then consumes it only after the durable EffectStarted claim succeeds.
#[must_use]
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RecoveryOperationPermit {
    operation_id: RecoveryOperationId,
    binding: RecoveryBinding,
}

impl RecoveryOperationPermit {
    #[cfg(test)]
    pub(crate) const fn for_test(
        operation_id: RecoveryOperationId,
        binding: RecoveryBinding,
    ) -> Self {
        Self {
            operation_id,
            binding,
        }
    }

    pub(crate) const fn operation_id(&self) -> RecoveryOperationId {
        self.operation_id
    }

    pub(crate) fn binding(&self) -> &RecoveryBinding {
        &self.binding
    }
}

/// Ephemeral proof that this exact operation durably transitioned from Intent
/// to EffectStarted. It is never serialized or cloneable.
#[must_use]
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct RecoveryEffectClaim {
    operation_id: RecoveryOperationId,
    binding: RecoveryBinding,
}

impl RecoveryEffectClaim {
    pub(crate) const fn operation_id(&self) -> RecoveryOperationId {
        self.operation_id
    }

    pub(crate) fn binding(&self) -> &RecoveryBinding {
        &self.binding
    }
}

/// Non-authority correlation identity for one durably claimed C1 recovery
/// operation. It is derived from the private claim and the same C1 binding
/// that admitted the operation. The value does not authorize a C3 transition
/// or prove a C3 outcome; C3 must persist and read back this exact operation
/// ID and binding digest in its own durable transaction.
#[must_use = "propagate this correlation value to the same owner transaction"]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct C1RecoveryOperationIdentity {
    operation_id: NonZeroU64,
    binding_digest: [u8; 32],
}

impl C1RecoveryOperationIdentity {
    pub const fn operation_id(&self) -> NonZeroU64 {
        self.operation_id
    }

    pub const fn binding_digest(&self) -> &[u8; 32] {
        &self.binding_digest
    }

    pub(crate) fn from_effect_claim(
        claim: &RecoveryEffectClaim,
        expected_binding: &RecoveryBinding,
    ) -> Option<Self> {
        if claim.binding() != expected_binding {
            return None;
        }
        Some(Self {
            operation_id: NonZeroU64::new(claim.operation_id().get())?,
            binding_digest: *claim.binding().digest_bytes(),
        })
    }
}

/// Opaque digest of the authoritative service-to-C1 store binding and exact
/// owner generation. The explicit local integration adapter compares it to
/// A's canonical scope digest; the default C1 crate treats it as opaque and
/// never derives it from caller-supplied identifiers.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct RecoveryBinding([u8; 32]);

impl RecoveryBinding {
    /// Keep the journal binding constructor inside C1; production callers
    /// reach it only after the canonical A scope has been checked against
    /// C1's sealed boot principal.
    #[cfg(feature = "system-service-recovery-contract")]
    pub(crate) const fn from_canonical_scope_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    #[cfg(test)]
    pub(crate) const fn for_test(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    pub(crate) const fn digest_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Opaque result of the C1 owner's exact-generation reconciliation gate. The
/// production constructor is intentionally absent until the fixed A/C3 typed
/// admission APIs can be consumed by the private C1 SystemService adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryReconciliation {
    operation_id: RecoveryOperationId,
    binding: RecoveryBinding,
    outcome_digest: [u8; 32],
}

impl RecoveryReconciliation {
    #[cfg(test)]
    pub(crate) const fn for_test(
        operation_id: RecoveryOperationId,
        binding: RecoveryBinding,
        outcome_digest: [u8; 32],
    ) -> Self {
        Self {
            operation_id,
            binding,
            outcome_digest,
        }
    }

    /// Build only from C1's current durable journal row. This is not a public
    /// permit constructor; the caller must compare the row's operation and
    /// binding against the shared admission before reserving the store fence.
    pub(crate) fn from_journal_record(
        record: &RecoveryJournalRecord,
        outcome_digest: [u8; 32],
    ) -> Self {
        Self {
            operation_id: record.operation_id,
            binding: record.binding.clone(),
            outcome_digest,
        }
    }

    pub(crate) const fn operation_id(&self) -> RecoveryOperationId {
        self.operation_id
    }

    pub(crate) fn binding(&self) -> &RecoveryBinding {
        &self.binding
    }

    pub(crate) const fn outcome_digest(&self) -> [u8; 32] {
        self.outcome_digest
    }
}

#[derive(Debug, Default)]
struct ReconciliationFenceState {
    closed: bool,
    writer_reserved: bool,
    active_readers: usize,
}

/// Shared same-process fence for raw current-snapshot readers and terminal
/// journal publication. Reader registrations are counted independently of
/// issuer Session activity; a reservation prevents new readers while the
/// issuer is being closed and the terminal record is written.
#[derive(Debug, Default)]
pub(crate) struct ReconciliationFence {
    state: Mutex<ReconciliationFenceState>,
}

#[must_use]
pub(crate) struct CurrentSnapshotReadPermit(Arc<ReconciliationFence>);

impl ReconciliationFence {
    pub(crate) fn enter_reader(
        self: &Arc<Self>,
    ) -> Result<CurrentSnapshotReadPermit, SnapshotStoreError> {
        let mut state = self.state.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock | std::sync::TryLockError::Poisoned(_) => {
                SnapshotStoreError::Unavailable
            }
        })?;
        if state.closed || state.writer_reserved {
            return Err(SnapshotStoreError::Unavailable);
        }
        state.active_readers = state
            .active_readers
            .checked_add(1)
            .ok_or(SnapshotStoreError::Unavailable)?;
        Ok(CurrentSnapshotReadPermit(Arc::clone(self)))
    }

    pub(crate) fn is_closed_or_reserved(&self) -> Result<bool, SnapshotStoreError> {
        let state = self.state.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock | std::sync::TryLockError::Poisoned(_) => {
                SnapshotStoreError::Unavailable
            }
        })?;
        Ok(state.closed || state.writer_reserved)
    }

    fn reserve_reconciliation(
        self: &Arc<Self>,
        reconciliation: &RecoveryReconciliation,
    ) -> Result<RecoveryReconciliationPermit, RecoveryJournalError> {
        let mut state = self.state.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => RecoveryJournalError::Busy,
            std::sync::TryLockError::Poisoned(_) => RecoveryJournalError::Unavailable,
        })?;
        if state.closed {
            return Err(RecoveryJournalError::Quarantined);
        }
        if state.writer_reserved || state.active_readers != 0 {
            return Err(RecoveryJournalError::Busy);
        }
        state.writer_reserved = true;
        Ok(RecoveryReconciliationPermit {
            fence: Arc::clone(self),
            operation_id: reconciliation.operation_id,
            binding: reconciliation.binding.clone(),
            outcome_digest: reconciliation.outcome_digest,
            terminal_started: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn state_for_test(&self) -> (bool, bool, usize) {
        let state = self.state.lock().unwrap();
        (state.closed, state.writer_reserved, state.active_readers)
    }

    #[cfg(test)]
    pub(crate) fn writer_available_for_test(&self) -> bool {
        self.state
            .try_lock()
            .is_ok_and(|state| !state.closed && !state.writer_reserved && state.active_readers == 0)
    }
}

impl Drop for CurrentSnapshotReadPermit {
    fn drop(&mut self) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.active_readers = state.active_readers.saturating_sub(1);
    }
}

#[must_use]
pub(crate) struct RecoveryReconciliationPermit {
    fence: Arc<ReconciliationFence>,
    operation_id: RecoveryOperationId,
    binding: RecoveryBinding,
    outcome_digest: [u8; 32],
    terminal_started: bool,
}

impl RecoveryReconciliationPermit {
    pub(crate) fn belongs_to(&self, fence: &Arc<ReconciliationFence>) -> bool {
        Arc::ptr_eq(&self.fence, fence)
    }

    fn preserve_closed_fence(&mut self) {
        let mut state = self
            .fence
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        state.writer_reserved = true;
        self.terminal_started = true;
    }

    pub(crate) fn begin_terminal_write(
        &mut self,
        reconciliation: &RecoveryReconciliation,
    ) -> Result<(), RecoveryJournalError> {
        if self.operation_id != reconciliation.operation_id
            || self.binding != reconciliation.binding
            || self.outcome_digest != reconciliation.outcome_digest
        {
            // The service closes issuer admission before calling this method.
            // Releasing the reservation on an identity mismatch would leave
            // that issuer dead while reopening raw store readers. Preserve
            // the store fence just as for any other post-close failure.
            self.preserve_closed_fence();
            return Err(RecoveryJournalError::NotCurrentOperation);
        }
        let mut state = match self.fence.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                state.closed = true;
                state.writer_reserved = true;
                self.terminal_started = true;
                return Err(RecoveryJournalError::Unavailable);
            }
        };
        if state.closed || !state.writer_reserved || state.active_readers != 0 {
            // Issuer admission is already permanently closed at this point.
            // Preserve the same-process store fence even if an invariant was
            // violated or its state was corrupted.
            state.closed = true;
            state.writer_reserved = true;
            self.terminal_started = true;
            return Err(RecoveryJournalError::Unavailable);
        }
        state.closed = true;
        self.terminal_started = true;
        Ok(())
    }
}

impl Drop for RecoveryReconciliationPermit {
    fn drop(&mut self) {
        if self.terminal_started {
            return;
        }
        let mut state = self
            .fence
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.writer_reserved = false;
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) enum RecoveryJournalPhase {
    Intent,
    EffectStarted,
    OutcomeUnknown,
    RuntimeActive,
    Reconciled { outcome_digest: [u8; 32] },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryJournalRecord {
    pub(crate) operation_id: RecoveryOperationId,
    pub(crate) binding: RecoveryBinding,
    pub(crate) phase: RecoveryJournalPhase,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecoveryJournal {
    pub(crate) next_operation_id: u64,
    pub(crate) record: Option<RecoveryJournalRecord>,
}

impl Default for RecoveryJournal {
    fn default() -> Self {
        Self {
            next_operation_id: 1,
            record: None,
        }
    }
}

impl RecoveryJournal {
    pub(crate) fn is_quarantined(&self) -> bool {
        self.record
            .as_ref()
            .is_some_and(|record| match &record.phase {
                RecoveryJournalPhase::Intent
                | RecoveryJournalPhase::EffectStarted
                | RecoveryJournalPhase::OutcomeUnknown
                | RecoveryJournalPhase::RuntimeActive => true,
                RecoveryJournalPhase::Reconciled { .. } => false,
            })
    }

    pub(crate) fn blocks_new_operation(&self) -> bool {
        self.record
            .as_ref()
            .is_some_and(|record| !matches!(&record.phase, RecoveryJournalPhase::Reconciled { .. }))
    }

    pub(crate) fn owner_runtime_active(&self) -> bool {
        self.record
            .as_ref()
            .is_some_and(|record| matches!(&record.phase, RecoveryJournalPhase::RuntimeActive))
    }

    pub(crate) fn validate(&self) -> bool {
        self.next_operation_id != 0
            && self.record.as_ref().is_none_or(|record| {
                record.operation_id.get() != 0 && record.operation_id.get() < self.next_operation_id
            })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryJournalError {
    Unavailable,
    Busy,
    Conflict,
    Quarantined,
    AlreadyClaimed,
    NotCurrentOperation,
    OutcomeMismatch,
}

/// Internal durable quarantine interface. Only the C1 system-service adapter
/// may use it; public constructors and generic consumers cannot create or
/// reconcile a production recovery operation.
pub(crate) trait RecoveryJournalStore {
    type ReconciliationPermit;

    fn recovery_journal_record(
        &self,
    ) -> Result<Option<RecoveryJournalRecord>, RecoveryJournalError>;

    /// Persist the first durable record for a recovery operation. `Busy` is
    /// retryable only when the implementation can prove that no Intent write
    /// began and the journal is unchanged. Every other error may follow an
    /// ambiguous durable write; callers must close old issuer admission and
    /// require exact reconciliation before any retry.
    fn begin_recovery_intent(
        &mut self,
        binding: RecoveryBinding,
    ) -> Result<RecoveryOperationPermit, RecoveryJournalError>;

    /// Claim the exact current Intent before any effect callback. `Busy` must
    /// mean the store can prove that no claim mutation occurred. Any other
    /// error may follow a write whose durable outcome cannot be distinguished;
    /// the caller must close old issuer admission and quarantine the operation.
    fn claim_recovery_effect(
        &mut self,
        permit: &RecoveryOperationPermit,
    ) -> Result<RecoveryEffectClaim, RecoveryJournalError>;

    fn commit_recovery_runtime_active(
        &mut self,
        claim: &RecoveryEffectClaim,
    ) -> Result<(), RecoveryJournalError>;

    fn mark_recovery_outcome_unknown(
        &mut self,
        operation_id: RecoveryOperationId,
        binding: &RecoveryBinding,
    ) -> Result<(), RecoveryJournalError>;

    /// Reserve the exact store fence before the service permanently closes
    /// issuer admission. A reader conflict is retryable and must not mutate
    /// the journal or retain the reservation.
    fn prepare_recovery_reconciliation(
        &mut self,
        reconciliation: &RecoveryReconciliation,
    ) -> Result<Self::ReconciliationPermit, RecoveryJournalError>;

    /// Terminal persistence requires the exact prepared permit and an issuer
    /// admission guard permanently closed before the terminal write begins.
    fn reconcile_recovery(
        &mut self,
        admission: &crate::service::IssuerRecoveryAdmissionGuard,
        permit: Self::ReconciliationPermit,
        reconciliation: RecoveryReconciliation,
    ) -> Result<(), RecoveryJournalError>;
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
/// last-known-good snapshot. The Linux adapter uses an explicitly supplied,
/// private directory handle and never turns a host path into identity.
///
/// Implementations are `Send + 'static` because shutdown can close admission
/// from inside an admitted callback and defer store destruction to the thread
/// that releases the final callback guard.
pub trait IdentitySnapshotStore: Send + 'static {
    /// Protected operations need a reader that remains bound to this store.
    /// Returning None preserves source compatibility and denies live identities.
    /// A failed or unavailable reader must never fall back to cached authority.
    fn current_snapshot_reader(&self) -> Option<CurrentSnapshotReader> {
        None
    }

    /// Return a reader for a specific C1 operation that was promoted to
    /// `RuntimeActive`. This is an observation, not a grant: C1 may retain it
    /// only after validating the corresponding live runtime admission, and
    /// the reader must fail once that exact operation/binding is no longer
    /// active. Stores without this exact journal-aware capability deny by
    /// default.
    fn current_snapshot_reader_for_runtime(
        &self,
        _operation: C1RecoveryOperationIdentity,
    ) -> Option<CurrentSnapshotReader> {
        None
    }

    /// Atomically observe current bytes with the independent store CAS
    /// high-water version. If current bytes are absent or restored from an
    /// older last-known-good snapshot, `high_water_version` must still retain
    /// the newest committed CAS version. `None` is valid only before the first
    /// commit; a current payload is present only when its version equals the
    /// high-water version. Report an older current payload as absent (or fail
    /// closed) so recovery can use last-known-good data. If the high-water is
    /// lost or uncertain, return an error, never an empty state; otherwise
    /// recovery could reuse a generation-scoped SessionId.
    fn load_current_state(&self) -> Result<SnapshotStoreState, SnapshotStoreError>;

    /// Stores without an explicit recovery-journal implementation fail
    /// closed. A missing capability is not evidence that no operation is
    /// pending.
    fn recovery_quarantined(&self) -> Result<bool, SnapshotStoreError> {
        Err(SnapshotStoreError::Unavailable)
    }

    /// Whether this store contains a successfully claimed/activated recovery
    /// transaction. Public recovery remains one-shot even in the authorized
    /// active instance; normal recovered-state operations use the separate
    /// quarantine status above.
    fn recovery_operation_active(&self) -> Result<bool, SnapshotStoreError> {
        Err(SnapshotStoreError::Unavailable)
    }

    fn load_current(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError>;

    /// Load the exact (bytes, version) pair from a last-good committed
    /// generation. The version is provenance: equal versions identify the same
    /// original commit generation and history represented by those bytes. An
    /// LKG mirror may share current high-water only when it is the same
    /// generation.
    /// Implementations must not relabel restored LKG bytes with the current
    /// high-water or a newly allocated version. Restored bytes become current
    /// only through commit_atomic, which issues a distinct later CAS version;
    /// the backup keeps its source version until a later committed current
    /// snapshot naturally replaces it as LKG.
    fn load_last_known_good(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError>;

    /// Compare `expected_version` against the non-rollback high-water version,
    /// not the current or last-known-good payload version. A successful commit
    /// must issue exactly the next version after expected_version (or version 1
    /// before the first commit) and must never skip or reuse a version. Session
    /// IDs reserve that next CAS generation before committing. Durable adapters
    /// must preserve this generation source across restart and return
    /// unavailable/corrupt state if it is lost; the in-memory reference fake
    /// has no persistence guarantee.
    /// `None` is valid only before the store's first committed snapshot.
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
    high_water_version: Option<SnapshotVersion>,
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
            high_water_version: self.high_water_version,
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
            .high_water_version
            .or_else(|| self.current.as_ref().map(|snapshot| snapshot.version))
            .unwrap_or(SnapshotVersion::new(1));
        self.high_water_version = Some(version);
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

    fn load_current_state(&self) -> Result<SnapshotStoreState, SnapshotStoreError> {
        if self.fail_reads {
            return Err(SnapshotStoreError::Unavailable);
        }
        Ok(SnapshotStoreState {
            current: self.current.clone(),
            high_water_version: self.high_water_version,
        })
    }

    fn recovery_quarantined(&self) -> Result<bool, SnapshotStoreError> {
        Ok(false)
    }

    fn recovery_operation_active(&self) -> Result<bool, SnapshotStoreError> {
        Ok(false)
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

        if self.high_water_version != expected_version {
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
        self.high_water_version = Some(next_version);
        Ok(next_version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AuthenticatedInteractiveCaller, AuthenticatedSystemServicePrincipal, AuthorizationFailure,
        AuthorizationRequest, CallerContextId, Clock, GuestCleanupFailure, GuestProfileCleaner,
        IdSourceFailure, IdentityAuthorizer, IdentityIdSource, IdentityService,
        NoopIdentityEventSink, PrincipalId, RecoverySource, SessionId, TrustedCallerContext,
        UnixMillis, UserId,
    };

    #[derive(Clone, Copy)]
    struct TestClock;

    impl Clock for TestClock {
        fn now(&self) -> UnixMillis {
            UnixMillis::new(100)
        }
    }

    #[derive(Default)]
    struct TestIds {
        user: u64,
        profile: u64,
        session: u64,
    }

    impl IdentityIdSource for TestIds {
        fn next_user_id(&mut self) -> Result<UserId, IdSourceFailure> {
            self.user += 1;
            UserId::new(self.user).map_err(|_| IdSourceFailure::Unavailable)
        }

        fn next_profile_id(&mut self) -> Result<crate::ProfileId, IdSourceFailure> {
            self.profile += 1;
            crate::ProfileId::new(format!("profile_p{}", self.profile))
                .map_err(|_| IdSourceFailure::Unavailable)
        }

        fn next_session_id(&mut self) -> Result<SessionId, IdSourceFailure> {
            self.session += 1;
            SessionId::new(format!("session_s{}", self.session))
                .map_err(|_| IdSourceFailure::Unavailable)
        }
    }

    struct AllowAuthorization;

    impl IdentityAuthorizer for AllowAuthorization {
        fn authorize(
            &mut self,
            _request: &AuthorizationRequest,
        ) -> Result<(), AuthorizationFailure> {
            Ok(())
        }
    }

    struct NoopCleaner;

    impl GuestProfileCleaner for NoopCleaner {
        fn cleanup_guest_profile(
            &mut self,
            _profile_id: &crate::ProfileId,
        ) -> Result<(), GuestCleanupFailure> {
            Ok(())
        }
    }

    struct TestCaller {
        principal: PrincipalId,
        context: CallerContextId,
    }

    impl TestCaller {
        fn new() -> Self {
            Self {
                principal: PrincipalId::new("system.identity-store-test").unwrap(),
                context: CallerContextId::new("caller_identity-store-test").unwrap(),
            }
        }
    }

    impl TrustedCallerContext for TestCaller {
        fn principal_id(&self) -> &PrincipalId {
            &self.principal
        }

        fn context_id(&self) -> &CallerContextId {
            &self.context
        }
    }

    impl AuthenticatedInteractiveCaller for TestCaller {}
    impl AuthenticatedSystemServicePrincipal for TestCaller {}
    impl crate::caller_proof_seal::Interactive for TestCaller {}
    impl crate::caller_proof_seal::SystemService for TestCaller {}

    type TestService =
        IdentityService<InMemorySnapshotStore, TestIds, TestClock, AllowAuthorization, NoopCleaner>;

    #[test]
    fn mismatched_terminal_identity_keeps_raw_reader_fence_closed() {
        let operation_id = RecoveryOperationId::for_test(31);
        let binding = RecoveryBinding::for_test([0x31; 32]);
        let outcome_digest = [0x32; 32];
        let exact = RecoveryReconciliation::for_test(operation_id, binding.clone(), outcome_digest);
        let mismatches = [
            RecoveryReconciliation::for_test(
                RecoveryOperationId::for_test(32),
                binding.clone(),
                outcome_digest,
            ),
            RecoveryReconciliation::for_test(
                operation_id,
                RecoveryBinding::for_test([0x33; 32]),
                outcome_digest,
            ),
            RecoveryReconciliation::for_test(operation_id, binding, [0x34; 32]),
        ];

        for mismatch in mismatches {
            let fence = Arc::new(ReconciliationFence::default());
            let mut permit = fence.reserve_reconciliation(&exact).unwrap();
            assert_eq!(
                fence.state_for_test(),
                (false, true, 0),
                "the exact terminal reservation blocks new raw readers"
            );

            assert_eq!(
                permit.begin_terminal_write(&mismatch),
                Err(RecoveryJournalError::NotCurrentOperation)
            );
            drop(permit);

            assert_eq!(
                fence.state_for_test(),
                (true, true, 0),
                "a post-closure identity mismatch must not reopen the raw-reader fence"
            );
            assert!(matches!(
                fence.enter_reader(),
                Err(SnapshotStoreError::Unavailable)
            ));
        }

        let fence = Arc::new(ReconciliationFence::default());
        let mut permit = fence.reserve_reconciliation(&exact).unwrap();
        assert_eq!(permit.begin_terminal_write(&exact), Ok(()));
        drop(permit);
        assert_eq!(fence.state_for_test(), (true, true, 0));
        assert!(matches!(
            fence.enter_reader(),
            Err(SnapshotStoreError::Unavailable)
        ));
    }

    fn new_service(store: InMemorySnapshotStore) -> TestService {
        IdentityService::new(
            store,
            TestIds::default(),
            TestClock,
            AllowAuthorization,
            NoopCleaner,
            NoopIdentityEventSink,
        )
    }

    #[test]
    fn in_memory_store_keeps_high_water_after_current_loss_and_rejects_lkg_cas() {
        let mut store = InMemorySnapshotStore::default();
        let bytes = IdentitySnapshot::empty().to_json().unwrap();
        let first = store.commit_atomic(None, &bytes).unwrap();
        let latest = store.commit_atomic(Some(first), &bytes).unwrap();
        let lkg = store.load_last_known_good().unwrap().unwrap();
        assert_eq!(lkg.version, first);

        store.current = None;
        store.observed.write().unwrap().current = None;

        let observation = store.load_current_state().unwrap();
        assert!(observation.current.is_none());
        assert_eq!(observation.high_water_version, Some(latest));
        assert_eq!(
            store.commit_atomic(Some(lkg.version), &lkg.bytes),
            Err(SnapshotStoreError::Conflict),
            "a stale LKG revision cannot pass the independent high-water CAS"
        );
        assert_eq!(
            store.load_current_state().unwrap().high_water_version,
            Some(latest)
        );
    }

    #[test]
    fn in_memory_store_overflow_fails_without_mutating_current_or_lkg() {
        let mut store = InMemorySnapshotStore::default();
        let bytes = IdentitySnapshot::empty().to_json().unwrap();
        let maximum = SnapshotVersion::new(u64::MAX);
        let previous = SnapshotVersion::new(u64::MAX - 1);
        store.current = Some(StoredSnapshot {
            bytes: bytes.clone(),
            version: maximum,
        });
        store.last_known_good = Some(StoredSnapshot {
            bytes: bytes.clone(),
            version: previous,
        });
        store.high_water_version = Some(maximum);
        store.observed.write().unwrap().current = store.current.clone();

        assert_eq!(
            store.commit_atomic(Some(maximum), &bytes),
            Err(SnapshotStoreError::Unavailable)
        );

        let state = store.load_current_state().unwrap();
        assert_eq!(state.current.unwrap().version, maximum);
        assert_eq!(state.high_water_version, Some(maximum));
        assert_eq!(
            store.load_last_known_good().unwrap().unwrap().version,
            previous
        );
    }

    #[test]
    fn in_memory_store_lkg_recovery_persists_incomplete_profile_history() {
        let caller = TestCaller::new();
        let mut writer = new_service(InMemorySnapshotStore::default());
        assert_eq!(
            writer.recover(&caller).unwrap().source,
            RecoverySource::Empty
        );
        let guest = writer
            .create_guest_session(
                &caller,
                None,
                crate::IdempotencyKey::new("in-memory-lost-guest").unwrap(),
            )
            .unwrap();
        let lost_profile = writer
            .snapshot()
            .sessions
            .iter()
            .find(|session| session.session_id == guest)
            .unwrap()
            .profile_id
            .clone();

        let (mut store, _, _, _, _, _) = writer
            .into_parts()
            .unwrap_or_else(|_| panic!("inactive writer should yield its store parts"));
        let state = store.load_current_state().unwrap();
        let latest = state.high_water_version.unwrap();
        let lkg = store.load_last_known_good().unwrap().unwrap();
        assert_eq!(latest.get(), lkg.version.get() + 1);
        let lkg_snapshot = IdentitySnapshot::from_json(&lkg.bytes).unwrap();
        assert!(!lkg_snapshot
            .profiles
            .iter()
            .any(|profile| profile.profile_id == lost_profile));
        assert!(!lkg_snapshot
            .sessions
            .iter()
            .any(|session| session.profile_id == lost_profile));

        store.current = None;
        store.observed.write().unwrap().current = None;
        let mut recovered = new_service(store);
        assert_eq!(
            recovered.recover(&caller).unwrap().source,
            RecoverySource::LastKnownGood
        );
        assert!(!recovered.snapshot().profile_id_history_complete);
        assert_eq!(
            recovered.create_guest_session(
                &caller,
                None,
                crate::IdempotencyKey::new("blocked-in-memory-reuse").unwrap()
            ),
            Err(crate::IdentityError::ProfileHistoryIncomplete)
        );

        let (store, _, _, _, _, _) = recovered
            .into_parts()
            .unwrap_or_else(|_| panic!("inactive recovered issuer should yield its store parts"));
        let mut restarted = new_service(store);
        assert_eq!(
            restarted.recover(&caller).unwrap().source,
            RecoverySource::Current
        );
        assert!(!restarted.snapshot().profile_id_history_complete);
        assert_eq!(
            restarted.create_guest_session(
                &caller,
                None,
                crate::IdempotencyKey::new("blocked-after-restart").unwrap()
            ),
            Err(crate::IdentityError::ProfileHistoryIncomplete)
        );
    }

    #[test]
    fn in_memory_store_missing_current_and_lkg_after_commit_rejects_recovery() {
        let caller = TestCaller::new();
        let mut initialized = new_service(InMemorySnapshotStore::default());
        initialized.recover(&caller).unwrap();
        let (mut store, _, _, _, _, _) = initialized
            .into_parts()
            .unwrap_or_else(|_| panic!("inactive issuer should yield its store parts"));
        assert!(store
            .load_current_state()
            .unwrap()
            .high_water_version
            .is_some());
        assert!(store.load_last_known_good().unwrap().is_none());

        store.current = None;
        store.observed.write().unwrap().current = None;
        let mut recovered = new_service(store);
        assert_eq!(
            recovered.recover(&caller),
            Err(crate::IdentityError::CorruptState)
        );
    }

    #[test]
    fn c1_recovery_operation_identity_requires_nonzero_exact_claim_binding() {
        let binding = RecoveryBinding::for_test([0x41; 32]);
        let exact_claim = RecoveryEffectClaim {
            operation_id: RecoveryOperationId::for_test(41),
            binding: binding.clone(),
        };
        let identity = C1RecoveryOperationIdentity::from_effect_claim(&exact_claim, &binding)
            .expect("a nonzero claimed operation with the exact binding is correlatable");
        assert_eq!(identity.operation_id().get(), 41);
        assert_eq!(identity.binding_digest(), binding.digest_bytes());

        let wrong_binding = RecoveryBinding::for_test([0x42; 32]);
        assert!(
            C1RecoveryOperationIdentity::from_effect_claim(&exact_claim, &wrong_binding).is_none()
        );

        let zero_claim = RecoveryEffectClaim {
            operation_id: RecoveryOperationId::for_test(0),
            binding: binding.clone(),
        };
        assert!(C1RecoveryOperationIdentity::from_effect_claim(&zero_claim, &binding).is_none());
    }
}
