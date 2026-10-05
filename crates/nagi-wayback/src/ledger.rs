use std::collections::BTreeMap;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::*;
use crate::store::{InMemoryLedgerStore, LedgerStore, StoreError, StoredLedger};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerError {
    Validation(ValidationError),
    UnsupportedVersion { kind: &'static str, version: u32 },
    InvalidTransition(&'static str),
    DuplicateId(&'static str),
    NotFound(&'static str),
    IntegrityMismatch { sequence: u64 },
    CheckpointMismatch,
    InvalidQuery(&'static str),
    Store(StoreError),
    Serialization(String),
}

impl std::fmt::Display for LedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(error) => write!(f, "invalid ledger data: {error}"),
            Self::UnsupportedVersion { kind, version } => {
                write!(f, "unsupported {kind} schema version {version}")
            }
            Self::InvalidTransition(reason) => write!(f, "invalid ledger transition: {reason}"),
            Self::DuplicateId(kind) => write!(f, "duplicate {kind} id"),
            Self::NotFound(kind) => write!(f, "{kind} was not found"),
            Self::IntegrityMismatch { sequence } => {
                write!(f, "ledger integrity mismatch at sequence {sequence}")
            }
            Self::CheckpointMismatch => f.write_str("ledger checkpoint does not match records"),
            Self::InvalidQuery(reason) => write!(f, "invalid activity query: {reason}"),
            Self::Store(error) => write!(f, "ledger storage error: {error}"),
            Self::Serialization(error) => write!(f, "ledger serialization error: {error}"),
        }
    }
}

impl std::error::Error for LedgerError {}

impl From<ValidationError> for LedgerError {
    fn from(value: ValidationError) -> Self {
        Self::Validation(value)
    }
}

impl From<StoreError> for LedgerError {
    fn from(value: StoreError) -> Self {
        Self::Store(value)
    }
}

#[derive(Serialize)]
struct RecordHashInput<'a> {
    schema_version: u32,
    sequence: u64,
    previous_hash: &'a Option<String>,
    payload: &'a LedgerPayload,
}

fn hash_payload_projection(payload: &LedgerPayload) -> LedgerPayload {
    let mut projected = payload.clone();
    if let LedgerPayload::ActivityEntry(entry) = &mut projected {
        entry.integrity = EntryIntegrity::default();
    }
    projected
}

fn calculate_record_hash(
    sequence: u64,
    previous_hash: &Option<String>,
    payload: &LedgerPayload,
) -> Result<String, LedgerError> {
    let projected = hash_payload_projection(payload);
    let input = RecordHashInput {
        schema_version: SCHEMA_VERSION,
        sequence,
        previous_hash,
        payload: &projected,
    };
    let bytes = serde_json::to_vec(&input)
        .map_err(|error| LedgerError::Serialization(error.to_string()))?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeRange {
    pub from_ms: Option<i64>,
    pub through_ms: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActivityQuery {
    pub time: Option<TimeRange>,
    pub actor_kind: Option<ActorKind>,
    pub principal: Option<PrincipalReference>,
    pub app_id: Option<String>,
    pub target: Option<TargetReference>,
    pub action_type: Option<ActionType>,
    pub transaction_id: Option<TransactionId>,
    pub correlation_id: Option<CorrelationId>,
    pub causation_id: Option<EntryId>,
    pub reversibility: Option<ReversibilityClass>,
    pub restore_point_id: Option<RestorePointId>,
    pub newest_first: bool,
}

#[derive(Debug, Clone)]
struct TransactionProjection {
    value: Transaction,
    finished_sequence: Option<u64>,
}

type TransactionProjectionResult = Result<
    (
        BTreeMap<TransactionId, TransactionProjection>,
        BTreeMap<EntryId, ActivityEntry>,
        BTreeMap<RestorePointId, RestorePointDraft>,
    ),
    LedgerError,
>;

fn verify_integrity(snapshot: &StoredLedger) -> Result<(), LedgerError> {
    let mut expected_previous: Option<String> = None;
    for (index, record) in snapshot.records.iter().enumerate() {
        let expected_sequence = index as u64 + 1;
        if record.schema_version != SCHEMA_VERSION {
            return Err(LedgerError::UnsupportedVersion {
                kind: "ledger record",
                version: record.schema_version,
            });
        }
        if record.sequence != expected_sequence || record.previous_hash != expected_previous {
            return Err(LedgerError::IntegrityMismatch {
                sequence: expected_sequence,
            });
        }
        if !is_sha256_hex(&record.record_hash) {
            return Err(LedgerError::IntegrityMismatch {
                sequence: expected_sequence,
            });
        }
        if let LedgerPayload::ActivityEntry(entry) = &record.payload {
            let expected_entry_integrity = EntryIntegrity {
                sequence: record.sequence,
                previous_entry_hash: record.previous_hash.clone(),
                entry_hash: record.record_hash.clone(),
            };
            if entry.integrity != expected_entry_integrity {
                return Err(LedgerError::IntegrityMismatch {
                    sequence: expected_sequence,
                });
            }
        }
        let calculated =
            calculate_record_hash(record.sequence, &record.previous_hash, &record.payload)?;
        if calculated != record.record_hash {
            return Err(LedgerError::IntegrityMismatch {
                sequence: expected_sequence,
            });
        }
        expected_previous = Some(record.record_hash.clone());
    }
    if snapshot.checkpoint.last_sequence != snapshot.records.len() as u64
        || snapshot.checkpoint.head_hash != expected_previous
    {
        return Err(LedgerError::CheckpointMismatch);
    }
    Ok(())
}

fn project_transactions(records: &[LedgerRecord]) -> TransactionProjectionResult {
    let mut transactions = BTreeMap::<TransactionId, TransactionProjection>::new();
    let mut entries = BTreeMap::<EntryId, ActivityEntry>::new();
    let mut restore_points = BTreeMap::<RestorePointId, RestorePointDraft>::new();

    for record in records {
        match &record.payload {
            LedgerPayload::TransactionStarted(draft) => {
                draft.validate()?;
                if transactions.contains_key(&draft.id) {
                    return Err(LedgerError::DuplicateId("transaction"));
                }
                let transaction = Transaction {
                    schema_version: SCHEMA_VERSION,
                    id: draft.id.clone(),
                    started_at_ms: draft.started_at_ms,
                    actor: draft.actor.clone(),
                    summary: draft.summary.clone(),
                    atomicity: draft.atomicity,
                    entry_ids: Vec::new(),
                    state: TransactionState::Open,
                };
                transactions.insert(
                    draft.id.clone(),
                    TransactionProjection {
                        value: transaction,
                        finished_sequence: None,
                    },
                );
            }
            LedgerPayload::ActivityEntry(entry) => {
                let draft = entry.draft();
                draft.validate()?;
                if entries.contains_key(&entry.id) {
                    return Err(LedgerError::DuplicateId("entry"));
                }
                for parent in [
                    draft.provenance.parent_entry_id.as_ref(),
                    draft.provenance.cause_entry_id.as_ref(),
                ]
                .into_iter()
                .flatten()
                {
                    if !entries.contains_key(parent) {
                        return Err(LedgerError::InvalidTransition(
                            "parent/cause must refer to an earlier activity entry",
                        ));
                    }
                }
                for restore_point_id in &draft.restore_point_refs {
                    if !restore_points.contains_key(restore_point_id) {
                        return Err(LedgerError::InvalidTransition(
                            "entry references a restore point that has not been recorded",
                        ));
                    }
                }
                if let Some(transaction_id) = &draft.transaction_id {
                    let projection = transactions.get_mut(transaction_id).ok_or(
                        LedgerError::InvalidTransition(
                            "entry transaction must be started before its actions",
                        ),
                    )?;
                    if projection.value.state != TransactionState::Open {
                        return Err(LedgerError::InvalidTransition(
                            "entries can only be appended to an open transaction",
                        ));
                    }
                    if draft.timestamp_ms < projection.value.started_at_ms {
                        return Err(LedgerError::InvalidTransition(
                            "entry time precedes transaction start",
                        ));
                    }
                    projection.value.entry_ids.push(entry.id.clone());
                }
                entries.insert(entry.id.clone(), entry.as_ref().clone());
            }
            LedgerPayload::TransactionFinished(finished) => {
                if finished.schema_version != SCHEMA_VERSION {
                    return Err(LedgerError::UnsupportedVersion {
                        kind: "transaction",
                        version: finished.schema_version,
                    });
                }
                finished.id.validate()?;
                let projection =
                    transactions
                        .get_mut(&finished.id)
                        .ok_or(LedgerError::InvalidTransition(
                            "transaction must be started before it finishes",
                        ))?;
                if projection.value.state != TransactionState::Open {
                    return Err(LedgerError::InvalidTransition(
                        "transaction can only finish once",
                    ));
                }
                if finished.finished_at_ms < projection.value.started_at_ms {
                    return Err(LedgerError::InvalidTransition(
                        "transaction finish time precedes its start",
                    ));
                }
                match finished.outcome {
                    TransactionEndKind::Committed => {
                        if finished.failure.is_some() {
                            return Err(LedgerError::InvalidTransition(
                                "committed transaction cannot contain a failure reason",
                            ));
                        }
                        projection.value.state = TransactionState::Committed {
                            at_ms: finished.finished_at_ms,
                        };
                    }
                    TransactionEndKind::Aborted => {
                        if !projection.value.entry_ids.is_empty() {
                            return Err(LedgerError::InvalidTransition(
                                "transaction with recorded actions must be partial, not aborted",
                            ));
                        }
                        if let Some(reason) = &finished.failure {
                            reason.validate()?;
                        }
                        projection.value.state = TransactionState::Aborted {
                            at_ms: finished.finished_at_ms,
                            reason: finished.failure.clone(),
                        };
                    }
                    TransactionEndKind::PartiallyApplied => {
                        let failure =
                            finished
                                .failure
                                .as_ref()
                                .ok_or(LedgerError::InvalidTransition(
                                    "partial transaction requires an explicit failure summary",
                                ))?;
                        if projection.value.entry_ids.is_empty() {
                            return Err(LedgerError::InvalidTransition(
                                "partial transaction must identify at least one recorded action",
                            ));
                        }
                        failure.validate()?;
                        projection.value.state = TransactionState::PartiallyApplied {
                            at_ms: finished.finished_at_ms,
                            failure: failure.clone(),
                        };
                    }
                }
                projection.finished_sequence = Some(record.sequence);
            }
            LedgerPayload::RestorePointCreated(point) => {
                point.validate()?;
                if restore_points.contains_key(&point.id) {
                    return Err(LedgerError::DuplicateId("restore point"));
                }
                if point.ledger_boundary.sequence >= record.sequence {
                    return Err(LedgerError::InvalidTransition(
                        "restore point boundary must precede its creation record",
                    ));
                }
                let expected_hash = if point.ledger_boundary.sequence == 0 {
                    None
                } else {
                    records
                        .get(point.ledger_boundary.sequence as usize - 1)
                        .map(|boundary| boundary.record_hash.clone())
                };
                if point.ledger_boundary.record_hash != expected_hash {
                    return Err(LedgerError::InvalidTransition(
                        "restore point boundary hash does not match ledger history",
                    ));
                }
                if let Some(transaction_id) = &point.source_transaction {
                    if !transactions.contains_key(transaction_id) {
                        return Err(LedgerError::InvalidTransition(
                            "restore point transaction reference is unknown",
                        ));
                    }
                }
                restore_points.insert(point.id.clone(), point.clone());
            }
        }
    }

    Ok((transactions, entries, restore_points))
}

fn make_entry(draft: ActivityEntryDraft, integrity: EntryIntegrity) -> ActivityEntry {
    ActivityEntry {
        schema_version: draft.schema_version,
        id: draft.id,
        timestamp_ms: draft.timestamp_ms,
        actor: draft.actor,
        app_id: draft.app_id,
        action_type: draft.action_type,
        target: draft.target,
        transaction_id: draft.transaction_id,
        provenance: draft.provenance,
        summary: draft.summary,
        metadata: draft.metadata,
        reversibility: draft.reversibility,
        snapshot_refs: draft.snapshot_refs,
        restore_point_refs: draft.restore_point_refs,
        payload: draft.payload,
        integrity,
    }
}

pub struct ActivityLedger<S: LedgerStore> {
    store: S,
}

impl<S: LedgerStore> ActivityLedger<S> {
    pub fn new(store: S) -> Result<Self, LedgerError> {
        Self::open(store)
    }

    pub fn open(store: S) -> Result<Self, LedgerError> {
        let snapshot = store.load()?;
        verify_integrity(&snapshot)?;
        project_transactions(&snapshot.records)?;
        Ok(Self { store })
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn current_boundary(&self) -> Result<LedgerBoundary, LedgerError> {
        let snapshot = self.verified_snapshot()?;
        Ok(LedgerBoundary {
            sequence: snapshot.checkpoint.last_sequence,
            record_hash: snapshot.checkpoint.head_hash,
        })
    }

    pub fn append_entry(
        &mut self,
        draft: ActivityEntryDraft,
    ) -> Result<ActivityEntry, LedgerError> {
        draft.validate()?;
        let sequence = self.next_sequence()?;
        let previous_hash = self.current_head()?;
        let entry = make_entry(
            draft,
            EntryIntegrity {
                sequence,
                previous_entry_hash: previous_hash.clone(),
                entry_hash: String::new(),
            },
        );
        let record = self.append_payload(LedgerPayload::ActivityEntry(Box::new(entry.clone())))?;
        match record.payload {
            LedgerPayload::ActivityEntry(entry) => Ok(*entry),
            _ => unreachable!("append_payload preserves payload variant"),
        }
    }

    pub fn start_transaction(
        &mut self,
        draft: TransactionDraft,
    ) -> Result<TransactionId, LedgerError> {
        draft.validate()?;
        let id = draft.id.clone();
        self.append_payload(LedgerPayload::TransactionStarted(draft))?;
        Ok(id)
    }

    pub fn commit_transaction(
        &mut self,
        id: &TransactionId,
        at_ms: i64,
    ) -> Result<(), LedgerError> {
        self.finish_transaction(id, at_ms, TransactionEndKind::Committed, None)
    }

    pub fn abort_transaction(
        &mut self,
        id: &TransactionId,
        at_ms: i64,
        reason: Option<HumanSummary>,
    ) -> Result<(), LedgerError> {
        self.finish_transaction(id, at_ms, TransactionEndKind::Aborted, reason)
    }

    pub fn mark_partial_failure(
        &mut self,
        id: &TransactionId,
        at_ms: i64,
        failure: HumanSummary,
    ) -> Result<(), LedgerError> {
        self.finish_transaction(
            id,
            at_ms,
            TransactionEndKind::PartiallyApplied,
            Some(failure),
        )
    }

    fn finish_transaction(
        &mut self,
        id: &TransactionId,
        at_ms: i64,
        outcome: TransactionEndKind,
        failure: Option<HumanSummary>,
    ) -> Result<(), LedgerError> {
        let transaction = self
            .transaction(id)?
            .ok_or(LedgerError::NotFound("transaction"))?;
        if transaction.state != TransactionState::Open {
            return Err(LedgerError::InvalidTransition(
                "transaction can only finish once",
            ));
        }
        let finished = TransactionFinished {
            schema_version: SCHEMA_VERSION,
            id: id.clone(),
            finished_at_ms: at_ms,
            outcome,
            failure,
        };
        self.append_payload(LedgerPayload::TransactionFinished(finished))?;
        Ok(())
    }

    pub fn record_restore_point(
        &mut self,
        draft: RestorePointDraft,
    ) -> Result<RestorePointId, LedgerError> {
        draft.validate()?;
        let id = draft.id.clone();
        self.append_payload(LedgerPayload::RestorePointCreated(draft))?;
        Ok(id)
    }

    pub fn transaction(&self, id: &TransactionId) -> Result<Option<Transaction>, LedgerError> {
        let snapshot = self.verified_snapshot()?;
        let (transactions, _, _) = project_transactions(&snapshot.records)?;
        Ok(transactions.get(id).map(|value| value.value.clone()))
    }

    pub fn transactions(&self) -> Result<Vec<Transaction>, LedgerError> {
        let snapshot = self.verified_snapshot()?;
        let (transactions, _, _) = project_transactions(&snapshot.records)?;
        Ok(transactions
            .into_values()
            .map(|value| value.value)
            .collect())
    }

    pub fn transaction_entries(
        &self,
        id: &TransactionId,
    ) -> Result<Vec<ActivityEntry>, LedgerError> {
        let snapshot = self.verified_snapshot()?;
        let (transactions, entries, _) = project_transactions(&snapshot.records)?;
        let Some(transaction) = transactions.get(id) else {
            return Ok(Vec::new());
        };
        transaction
            .value
            .entry_ids
            .iter()
            .map(|entry_id| {
                entries
                    .get(entry_id)
                    .cloned()
                    .ok_or(LedgerError::IntegrityMismatch {
                        sequence: transaction
                            .value
                            .entry_ids
                            .iter()
                            .position(|candidate| candidate == entry_id)
                            .unwrap_or_default() as u64,
                    })
            })
            .collect()
    }

    pub fn restore_point(
        &self,
        id: &RestorePointId,
    ) -> Result<Option<RestorePointDraft>, LedgerError> {
        let snapshot = self.verified_snapshot()?;
        let (_, _, points) = project_transactions(&snapshot.records)?;
        Ok(points.get(id).cloned())
    }

    pub fn query(&self, query: &ActivityQuery) -> Result<Vec<ActivityEntry>, LedgerError> {
        if let Some(time) = &query.time {
            if time.from_ms.is_some_and(|value| value < 0)
                || time.through_ms.is_some_and(|value| value < 0)
                || matches!((time.from_ms, time.through_ms), (Some(from), Some(to)) if from > to)
            {
                return Err(LedgerError::InvalidQuery("time range is invalid"));
            }
        }
        if let Some(principal) = &query.principal {
            principal.validate()?;
        }
        if let Some(app_id) = &query.app_id {
            if app_id.is_empty() || app_id.len() > 160 || app_id.chars().any(char::is_control) {
                return Err(LedgerError::InvalidQuery("app id is invalid"));
            }
        }
        if let Some(target) = &query.target {
            target.validate()?;
        }
        if let Some(action_type) = &query.action_type {
            action_type.validate()?;
        }
        if let Some(transaction_id) = &query.transaction_id {
            transaction_id.validate()?;
        }
        if let Some(correlation_id) = &query.correlation_id {
            correlation_id.validate()?;
        }
        if let Some(causation_id) = &query.causation_id {
            causation_id.validate()?;
        }
        if let Some(restore_point_id) = &query.restore_point_id {
            restore_point_id.validate()?;
        }
        let snapshot = self.verified_snapshot()?;
        let mut entries = snapshot
            .records
            .iter()
            .filter_map(|record| match &record.payload {
                LedgerPayload::ActivityEntry(entry) => Some(entry.as_ref().clone()),
                _ => None,
            })
            .filter(|entry| matches_query(entry, query))
            .collect::<Vec<_>>();
        if query.newest_first {
            entries.reverse();
        }
        Ok(entries)
    }

    pub fn committed_transactions_newest_first(&self) -> Result<Vec<Transaction>, LedgerError> {
        let snapshot = self.verified_snapshot()?;
        let (transactions, _, _) = project_transactions(&snapshot.records)?;
        let mut ordered = transactions
            .into_values()
            .filter_map(|projection| {
                if matches!(projection.value.state, TransactionState::Committed { .. }) {
                    Some((projection.finished_sequence?, projection.value))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        ordered.sort_by_key(|(sequence, _)| std::cmp::Reverse(*sequence));
        Ok(ordered
            .into_iter()
            .map(|(_, transaction)| transaction)
            .collect())
    }

    pub fn serialize(&self) -> Result<Vec<u8>, LedgerError> {
        let snapshot = self.verified_snapshot()?;
        let document = LedgerDocument {
            schema_version: SCHEMA_VERSION,
            records: snapshot.records,
            checkpoint: snapshot.checkpoint,
        };
        serde_json::to_vec_pretty(&document)
            .map_err(|error| LedgerError::Serialization(error.to_string()))
    }

    pub fn document(&self) -> Result<LedgerDocument, LedgerError> {
        let snapshot = self.verified_snapshot()?;
        Ok(LedgerDocument {
            schema_version: SCHEMA_VERSION,
            records: snapshot.records,
            checkpoint: snapshot.checkpoint,
        })
    }

    fn append_payload(&mut self, mut payload: LedgerPayload) -> Result<LedgerRecord, LedgerError> {
        let snapshot = self.verified_snapshot()?;
        let sequence = snapshot.records.len() as u64 + 1;
        let previous_hash = snapshot.checkpoint.head_hash.clone();
        if let LedgerPayload::ActivityEntry(entry) = &mut payload {
            entry.integrity = EntryIntegrity {
                sequence,
                previous_entry_hash: previous_hash.clone(),
                entry_hash: String::new(),
            };
        }
        let mut projected = snapshot.records.clone();
        projected.push(LedgerRecord {
            schema_version: SCHEMA_VERSION,
            sequence,
            previous_hash: previous_hash.clone(),
            payload: payload.clone(),
            record_hash: String::new(),
        });
        project_transactions(&projected)?;
        let hash = calculate_record_hash(sequence, &previous_hash, &payload)?;
        if let LedgerPayload::ActivityEntry(entry) = &mut payload {
            entry.integrity.entry_hash = hash.clone();
        }
        let record = LedgerRecord {
            schema_version: SCHEMA_VERSION,
            sequence,
            previous_hash,
            payload,
            record_hash: hash.clone(),
        };
        self.store.append(
            record.clone(),
            LedgerCheckpoint {
                last_sequence: sequence,
                head_hash: Some(hash),
            },
        )?;
        Ok(record)
    }

    fn verified_snapshot(&self) -> Result<StoredLedger, LedgerError> {
        let snapshot = self.store.load()?;
        verify_integrity(&snapshot)?;
        project_transactions(&snapshot.records)?;
        Ok(snapshot)
    }

    fn next_sequence(&self) -> Result<u64, LedgerError> {
        Ok(self.verified_snapshot()?.records.len() as u64 + 1)
    }

    fn current_head(&self) -> Result<Option<String>, LedgerError> {
        Ok(self.verified_snapshot()?.checkpoint.head_hash)
    }
}

impl ActivityLedger<InMemoryLedgerStore> {
    pub fn from_json(bytes: &[u8]) -> Result<Self, LedgerError> {
        let document: LedgerDocument = serde_json::from_slice(bytes)
            .map_err(|error| LedgerError::Serialization(error.to_string()))?;
        if document.schema_version != SCHEMA_VERSION {
            return Err(LedgerError::UnsupportedVersion {
                kind: "ledger document",
                version: document.schema_version,
            });
        }
        Self::open(InMemoryLedgerStore::from_stored(StoredLedger {
            records: document.records,
            checkpoint: document.checkpoint,
        }))
    }
}

fn matches_query(entry: &ActivityEntry, query: &ActivityQuery) -> bool {
    if query.time.as_ref().is_some_and(|range| {
        range.from_ms.is_some_and(|from| entry.timestamp_ms < from)
            || range
                .through_ms
                .is_some_and(|through| entry.timestamp_ms > through)
    }) {
        return false;
    }
    if query
        .actor_kind
        .is_some_and(|kind| entry.actor.kind != kind)
    {
        return false;
    }
    if query.principal.as_ref().is_some_and(|principal| {
        entry.actor.principal.as_ref() != Some(principal)
            && entry.actor.delegated_for.as_ref() != Some(principal)
    }) {
        return false;
    }
    if query.app_id.as_ref().is_some_and(|app_id| {
        entry.app_id.as_ref() != Some(app_id)
            && entry.actor.app_id.as_ref() != Some(app_id)
            && entry.target.app_id.as_ref() != Some(app_id)
    }) {
        return false;
    }
    if query
        .target
        .as_ref()
        .is_some_and(|target| &entry.target != target)
    {
        return false;
    }
    if query
        .action_type
        .as_ref()
        .is_some_and(|action| &entry.action_type != action)
    {
        return false;
    }
    if query
        .transaction_id
        .as_ref()
        .is_some_and(|transaction| entry.transaction_id.as_ref() != Some(transaction))
    {
        return false;
    }
    if query
        .correlation_id
        .as_ref()
        .is_some_and(|correlation| entry.provenance.correlation_id.as_ref() != Some(correlation))
    {
        return false;
    }
    if query.causation_id.as_ref().is_some_and(|cause| {
        entry.provenance.parent_entry_id.as_ref() != Some(cause)
            && entry.provenance.cause_entry_id.as_ref() != Some(cause)
    }) {
        return false;
    }
    if query
        .reversibility
        .is_some_and(|class| entry.reversibility.class() != class)
    {
        return false;
    }
    if query
        .restore_point_id
        .as_ref()
        .is_some_and(|point| !entry.restore_point_refs.contains(point))
    {
        return false;
    }
    true
}

impl<S: LedgerStore> ActivityLedger<S> {
    pub(crate) fn entries_after(&self, sequence: u64) -> Result<Vec<ActivityEntry>, LedgerError> {
        let snapshot = self.verified_snapshot()?;
        Ok(snapshot
            .records
            .iter()
            .filter(|record| record.sequence > sequence)
            .filter_map(|record| match &record.payload {
                LedgerPayload::ActivityEntry(entry) => Some(entry.as_ref().clone()),
                _ => None,
            })
            .collect())
    }
}
