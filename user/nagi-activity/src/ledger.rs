use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

use nagi_model::{AppId, AppSessionId, NodeId, TransactionId, UserId, WorkspaceId};

use crate::model::{
    valid_key, ActionData, ActionDraft, ActionId, ActionOutcome, ActionRecord, ActionType,
    ActivityEvent, ActivityEventKind, ActorKind, ActorRef, EventContext, EventId,
    ReversibilityKind, RevertExecutionResult, RevertOutcome, SnapshotDraft, SnapshotId,
    SnapshotReference, TargetId, Timestamp, TransactionStatus, ACTIVITY_EVENT_SCHEMA_VERSION,
    MAX_SUMMARY_KEY_BYTES,
};
use crate::store::{ActivityStore, StoreError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdSourceError {
    Exhausted,
}

/// The ID source must provide a unique namespace across durable ledger
/// instances. The sequential implementation is intended for tests and local
/// fixtures; production services should bind it to Nagi's durable ID source.
pub trait ActivityIdSource {
    fn next_id(&mut self) -> Result<u128, IdSourceError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SequentialIdSource {
    namespace: u64,
    next_sequence: Option<u64>,
}

impl SequentialIdSource {
    pub const fn new(namespace: u64) -> Self {
        Self {
            namespace,
            next_sequence: Some(1),
        }
    }
}

impl ActivityIdSource for SequentialIdSource {
    fn next_id(&mut self) -> Result<u128, IdSourceError> {
        let sequence = self.next_sequence.ok_or(IdSourceError::Exhausted)?;
        self.next_sequence = sequence.checked_add(1);
        Ok(((self.namespace as u128) << 64) | sequence as u128)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActorFilter {
    pub kind: Option<ActorKind>,
    pub identity: Option<crate::ActorIdentity>,
    pub delegated_for: Option<UserId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ActivityQuery {
    /// Inclusive beginning of the requested range.
    pub from: Option<Timestamp>,
    /// Inclusive end of the requested range.
    pub through: Option<Timestamp>,
    pub actor: Option<ActorFilter>,
    pub application_id: Option<AppId>,
    pub app_session_id: Option<AppSessionId>,
    pub node_id: Option<NodeId>,
    pub workspace_id: Option<WorkspaceId>,
    pub action_type: Option<ActionType>,
    pub target: Option<TargetId>,
    pub transaction_id: Option<TransactionId>,
    pub reversible_only: bool,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionRecord {
    pub transaction_id: TransactionId,
    pub actor: ActorRef,
    pub started_at: Timestamp,
    pub finished_at: Option<Timestamp>,
    pub status: TransactionStatus,
    pub action_ids: Vec<ActionId>,
    pub snapshot_ids: Vec<SnapshotId>,
    pub reversibility: ReversibilityKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerError {
    Store(StoreError),
    IdSource(IdSourceError),
    InvalidContext,
    InvalidSummaryKey,
    InvalidMetadata,
    InvalidTimeRange,
    DuplicateActionId,
    DuplicateTransactionId,
    DuplicateSnapshotId,
    MissingAction,
    MissingParentAction,
    MissingTransaction,
    TransactionNotInProgress,
    InvalidTransactionStatus,
    TransactionNotFullyReverted,
    MissingSnapshotParent,
    RevertRequestNotFound,
    RevertRequestAlreadyResolved,
    UnsupportedSchemaVersion,
}

impl From<StoreError> for LedgerError {
    fn from(value: StoreError) -> Self {
        match value {
            StoreError::UnsupportedSchemaVersion => Self::UnsupportedSchemaVersion,
            other => Self::Store(other),
        }
    }
}

impl From<IdSourceError> for LedgerError {
    fn from(value: IdSourceError) -> Self {
        Self::IdSource(value)
    }
}

pub trait RevertExecutor {
    /// Apply the inverse for `action`. Implementations should be idempotent
    /// for a given request event ID so a service can recover after interruption.
    fn apply(
        &mut self,
        request_id: EventId,
        action: &ActionRecord,
        snapshot: Option<&SnapshotReference>,
    ) -> RevertExecutionResult;
}

pub trait ActivityAccessPolicy {
    type Error;

    fn authorize_query(
        &self,
        requester: &ActorRef,
        query: &ActivityQuery,
    ) -> Result<(), Self::Error>;

    fn can_read_event(&self, requester: &ActorRef, event: &ActivityEvent) -> bool;
}

#[derive(Debug, Eq, PartialEq)]
pub enum AuthorizedQueryError<E> {
    InvalidQuery(LedgerError),
    Denied(E),
}

/// Ledger orchestration over an append/read store and an injected stable ID
/// source. The ledger does not mutate files or materialize snapshots.
pub struct ActivityLedger<S, I> {
    store: S,
    ids: I,
}

impl<S: ActivityStore, I: ActivityIdSource> ActivityLedger<S, I> {
    pub const fn new(store: S, ids: I) -> Self {
        Self { store, ids }
    }

    pub fn begin_transaction(
        &mut self,
        transaction_id: TransactionId,
        mut context: EventContext,
        reason_key: Option<String>,
    ) -> Result<EventId, LedgerError> {
        validate_context(&context)?;
        validate_optional_key(reason_key.as_deref())?;
        if self.has_transaction_id(transaction_id) {
            return Err(LedgerError::DuplicateTransactionId);
        }
        if let Some(parent) = context.parent_action_id {
            if self.get_action(parent).is_none() {
                return Err(LedgerError::MissingParentAction);
            }
        }
        context.transaction_id = Some(transaction_id);
        self.append_with_context(
            context,
            ActivityEventKind::TransactionStarted {
                transaction_id,
                reason_key,
            },
        )
    }

    pub fn record_action(
        &mut self,
        context: EventContext,
        draft: ActionDraft,
    ) -> Result<ActionId, LedgerError> {
        validate_context(&context)?;
        if !draft.is_valid() {
            return Err(if valid_key(&draft.summary_key, MAX_SUMMARY_KEY_BYTES) {
                LedgerError::InvalidMetadata
            } else {
                LedgerError::InvalidSummaryKey
            });
        }
        if let Some(transaction_id) = context.transaction_id {
            let transaction = self
                .transaction(transaction_id)
                .ok_or(LedgerError::MissingTransaction)?;
            if transaction.status != TransactionStatus::InProgress {
                return Err(LedgerError::TransactionNotInProgress);
            }
        }
        if let Some(parent) = context.parent_action_id {
            if self.get_action(parent).is_none() {
                return Err(LedgerError::MissingParentAction);
            }
        }

        let action_id = ActionId(self.ids.next_id()?);
        if self.has_action_id(action_id) {
            return Err(LedgerError::DuplicateActionId);
        }
        let data = ActionData {
            action_id,
            action_type: draft.action_type,
            target: draft.target,
            summary_key: draft.summary_key,
            metadata: draft.metadata,
            before: draft.before,
            after: draft.after,
            reversibility: draft.reversibility,
            related_snapshot: draft.related_snapshot,
            authorization: draft.authorization,
            provenance: draft.provenance,
        };
        self.append_with_context(context, ActivityEventKind::ActionCreated(Box::new(data)))?;
        Ok(action_id)
    }

    pub fn record_action_outcome(
        &mut self,
        action_id: ActionId,
        mut context: EventContext,
        outcome: ActionOutcome,
        result_key: Option<String>,
    ) -> Result<EventId, LedgerError> {
        validate_context(&context)?;
        validate_optional_key(result_key.as_deref())?;
        let action = self
            .get_action(action_id)
            .ok_or(LedgerError::MissingAction)?;
        if context.transaction_id.is_none() {
            context.transaction_id = action.context.transaction_id;
        }
        if context.parent_action_id.is_none() {
            context.parent_action_id = Some(action_id);
        }
        self.append_with_context(
            context,
            ActivityEventKind::ActionOutcomeRecorded {
                action_id,
                outcome,
                result_key,
            },
        )
    }

    pub fn complete_transaction(
        &mut self,
        transaction_id: TransactionId,
        context: EventContext,
    ) -> Result<EventId, LedgerError> {
        self.change_transaction_status(transaction_id, context, TransactionStatus::Completed, None)
    }

    pub fn fail_transaction(
        &mut self,
        transaction_id: TransactionId,
        context: EventContext,
        result_key: Option<String>,
    ) -> Result<EventId, LedgerError> {
        self.change_transaction_status(
            transaction_id,
            context,
            TransactionStatus::Failed,
            result_key,
        )
    }

    pub fn partially_complete_transaction(
        &mut self,
        transaction_id: TransactionId,
        context: EventContext,
        result_key: Option<String>,
    ) -> Result<EventId, LedgerError> {
        self.change_transaction_status(
            transaction_id,
            context,
            TransactionStatus::PartiallyCompleted,
            result_key,
        )
    }

    fn change_transaction_status(
        &mut self,
        transaction_id: TransactionId,
        mut context: EventContext,
        status: TransactionStatus,
        result_key: Option<String>,
    ) -> Result<EventId, LedgerError> {
        validate_context(&context)?;
        validate_optional_key(result_key.as_deref())?;
        let transaction = self
            .transaction(transaction_id)
            .ok_or(LedgerError::MissingTransaction)?;
        if transaction.status != TransactionStatus::InProgress {
            return Err(LedgerError::TransactionNotInProgress);
        }
        if !matches!(
            status,
            TransactionStatus::Completed
                | TransactionStatus::Failed
                | TransactionStatus::PartiallyCompleted
        ) {
            return Err(LedgerError::InvalidTransactionStatus);
        }
        context.transaction_id = Some(transaction_id);
        self.append_with_context(
            context,
            ActivityEventKind::TransactionStatusChanged {
                transaction_id,
                status,
                result_key,
            },
        )
    }

    pub fn mark_reverted(
        &mut self,
        transaction_id: TransactionId,
        mut context: EventContext,
    ) -> Result<EventId, LedgerError> {
        validate_context(&context)?;
        let transaction = self
            .transaction(transaction_id)
            .ok_or(LedgerError::MissingTransaction)?;
        if !matches!(
            transaction.status,
            TransactionStatus::Completed | TransactionStatus::PartiallyCompleted
        ) || transaction.action_ids.is_empty()
            || !transaction
                .action_ids
                .iter()
                .all(|action_id| self.is_action_reverted(*action_id))
        {
            return Err(LedgerError::TransactionNotFullyReverted);
        }
        context.transaction_id = Some(transaction_id);
        self.append_with_context(
            context,
            ActivityEventKind::TransactionStatusChanged {
                transaction_id,
                status: TransactionStatus::Reverted,
                result_key: None,
            },
        )
    }

    pub fn create_snapshot_reference(
        &mut self,
        draft: SnapshotDraft,
        mut context: EventContext,
    ) -> Result<SnapshotId, LedgerError> {
        validate_context(&context)?;
        validate_optional_key(draft.reason_key.as_deref())?;
        if !valid_key(&draft.storage.backend_id, MAX_SUMMARY_KEY_BYTES) {
            return Err(LedgerError::InvalidSummaryKey);
        }
        if let Some(parent) = draft.parent_snapshot {
            if self.resolve_snapshot(parent).is_none() {
                return Err(LedgerError::MissingSnapshotParent);
            }
        }
        let scope_transaction = match &draft.scope {
            crate::SnapshotScope::Transaction(id) => Some(*id),
            _ => None,
        };
        let related_transaction = draft.related_transaction.or(scope_transaction);
        if let Some(transaction_id) = related_transaction {
            if self.transaction(transaction_id).is_none() {
                return Err(LedgerError::MissingTransaction);
            }
            if context
                .transaction_id
                .is_some_and(|context_id| context_id != transaction_id)
            {
                return Err(LedgerError::InvalidContext);
            }
            context.transaction_id = Some(transaction_id);
        }

        let id = SnapshotId(self.ids.next_id()?);
        if self.has_snapshot_id(id) {
            return Err(LedgerError::DuplicateSnapshotId);
        }
        let reference = SnapshotReference {
            id,
            created_at: context.timestamp,
            scope: draft.scope,
            parent_snapshot: draft.parent_snapshot,
            reason_key: draft.reason_key,
            related_transaction,
            storage: draft.storage,
            retention: draft.retention,
            pinned: draft.pinned,
        };
        self.append_with_context(
            context,
            ActivityEventKind::SnapshotReferenceCreated(Box::new(reference)),
        )?;
        Ok(id)
    }

    pub fn redact_event(
        &mut self,
        target_event_id: EventId,
        context: EventContext,
        reason_key: String,
    ) -> Result<EventId, LedgerError> {
        validate_context(&context)?;
        if !valid_key(&reason_key, MAX_SUMMARY_KEY_BYTES) {
            return Err(LedgerError::InvalidSummaryKey);
        }
        self.append_with_context(
            context,
            ActivityEventKind::RedactionRecorded {
                target_event_id,
                reason_key,
            },
        )
    }

    /// Appends a request before executing a revert. If execution is interrupted,
    /// the request event remains and may be resumed using `complete_revert_request`.
    pub fn request_revert<E: RevertExecutor>(
        &mut self,
        action_id: ActionId,
        mut context: EventContext,
        executor: &mut E,
    ) -> Result<RevertOutcome, LedgerError> {
        validate_context(&context)?;
        let action = self
            .get_action(action_id)
            .ok_or(LedgerError::MissingAction)?;
        context.transaction_id = action.context.transaction_id;
        context.parent_action_id = Some(action_id);
        if context.causation_id.is_none() {
            context.causation_id = Some(action.event_id);
        }
        if context.correlation_id.is_none() {
            context.correlation_id = action.context.correlation_id;
        }
        let request_id = self.next_event_id()?;
        self.append_event(ActivityEvent::new(
            request_id,
            context.clone(),
            ActivityEventKind::ActionRevertRequested { action_id },
        ))?;
        self.complete_revert_request(request_id, context, executor)
    }

    /// Completes a recorded revert request. Reusing the same request event ID
    /// gives a storage-backed executor a stable idempotency key after a crash.
    pub fn complete_revert_request<E: RevertExecutor>(
        &mut self,
        request_id: EventId,
        mut context: EventContext,
        executor: &mut E,
    ) -> Result<RevertOutcome, LedgerError> {
        validate_context(&context)?;
        let request = self
            .raw_event(request_id)
            .ok_or(LedgerError::RevertRequestNotFound)?;
        let ActivityEventKind::ActionRevertRequested { action_id } = request.kind else {
            return Err(LedgerError::RevertRequestNotFound);
        };
        if self.revert_result(request_id).is_some() {
            return Err(LedgerError::RevertRequestAlreadyResolved);
        }
        let action = self
            .get_action(action_id)
            .ok_or(LedgerError::MissingAction)?;
        context.transaction_id = action.context.transaction_id;
        context.parent_action_id = Some(action_id);
        context.causation_id = Some(request_id);
        if context.correlation_id.is_none() {
            context.correlation_id = request.context.correlation_id;
        }

        let outcome = if self.is_action_reverted(action_id) {
            RevertOutcome::AlreadyReverted
        } else if self.has_pending_revert(action_id, request_id) {
            RevertOutcome::AlreadyInProgress
        } else {
            match action.reversibility.kind {
                ReversibilityKind::FullyReversible | ReversibilityKind::PartiallyReversible => {
                    let snapshot = action
                        .related_snapshot
                        .and_then(|snapshot_id| self.resolve_snapshot(snapshot_id));
                    match executor.apply(request_id, &action, snapshot.as_ref()) {
                        RevertExecutionResult::Applied => RevertOutcome::Applied,
                        RevertExecutionResult::PartiallyApplied => RevertOutcome::PartiallyApplied,
                        RevertExecutionResult::Failed => RevertOutcome::Failed,
                    }
                }
                ReversibilityKind::ReversibleWithSnapshot => {
                    let Some(snapshot) = action
                        .related_snapshot
                        .and_then(|snapshot_id| self.resolve_snapshot(snapshot_id))
                    else {
                        self.append_revert_result(
                            action_id,
                            RevertOutcome::RejectedMissingSnapshot,
                            Some(String::from("wayback.snapshot.required")),
                            context,
                        )?;
                        return Ok(RevertOutcome::RejectedMissingSnapshot);
                    };
                    match executor.apply(request_id, &action, Some(&snapshot)) {
                        RevertExecutionResult::Applied => RevertOutcome::Applied,
                        RevertExecutionResult::PartiallyApplied => RevertOutcome::PartiallyApplied,
                        RevertExecutionResult::Failed => RevertOutcome::Failed,
                    }
                }
                ReversibilityKind::Irreversible => RevertOutcome::RejectedIrreversible,
                ReversibilityKind::Unknown => RevertOutcome::RejectedUnknown,
            }
        };
        let reason_key = match outcome {
            RevertOutcome::RejectedIrreversible => Some(String::from("wayback.irreversible")),
            RevertOutcome::RejectedUnknown => Some(String::from("wayback.reversibility.unknown")),
            RevertOutcome::AlreadyReverted => Some(String::from("wayback.already-reverted")),
            RevertOutcome::AlreadyInProgress => Some(String::from("wayback.revert.pending")),
            _ => None,
        };
        self.append_revert_result(action_id, outcome, reason_key, context)?;
        Ok(outcome)
    }

    fn append_revert_result(
        &mut self,
        action_id: ActionId,
        outcome: RevertOutcome,
        reason_key: Option<String>,
        context: EventContext,
    ) -> Result<EventId, LedgerError> {
        self.append_with_context(
            context,
            ActivityEventKind::ActionRevertRecorded {
                action_id,
                outcome,
                reason_key,
            },
        )
    }

    fn query_unchecked(&self, query: &ActivityQuery) -> Result<Vec<ActivityEvent>, LedgerError> {
        validate_query(query)?;
        let raw_events = self.store.all_events();
        let redacted = redacted_event_ids(&raw_events);
        let mut matching = raw_events
            .iter()
            .filter(|event| !redacted.contains(&event.event_id))
            .filter(|event| query_matches_context(query, event))
            .filter(|event| {
                if query.action_type.is_none() && query.target.is_none() && !query.reversible_only {
                    return true;
                }
                let Some(action_id) = action_id_for_event(&event.kind) else {
                    return false;
                };
                let Some(action) = action_from_events(&raw_events, action_id) else {
                    return false;
                };
                if query
                    .action_type
                    .as_ref()
                    .is_some_and(|action_type| &action.action_type != action_type)
                {
                    return false;
                }
                if query
                    .target
                    .is_some_and(|target| action.target.id != target)
                {
                    return false;
                }
                !query.reversible_only || is_reversible_for_query(self, &action)
            })
            .cloned()
            .collect::<Vec<_>>();
        matching.sort_by(|left, right| {
            left.context
                .timestamp
                .cmp(&right.context.timestamp)
                .then_with(|| left.event_id.cmp(&right.event_id))
        });
        if let Some(limit) = query.limit {
            matching.truncate(limit);
        }
        Ok(matching)
    }

    pub fn query<P: ActivityAccessPolicy>(
        &self,
        requester: &ActorRef,
        query: &ActivityQuery,
        policy: &P,
    ) -> Result<Vec<ActivityEvent>, AuthorizedQueryError<P::Error>> {
        policy
            .authorize_query(requester, query)
            .map_err(AuthorizedQueryError::Denied)?;
        self.query_unchecked(query)
            .map(|events| {
                events
                    .into_iter()
                    .filter(|event| policy.can_read_event(requester, event))
                    .collect()
            })
            .map_err(AuthorizedQueryError::InvalidQuery)
    }

    /// Read one event through the same per-record policy gate used by queries.
    pub fn read_event<P: ActivityAccessPolicy>(
        &self,
        event_id: EventId,
        requester: &ActorRef,
        policy: &P,
    ) -> Result<Option<ActivityEvent>, AuthorizedQueryError<P::Error>> {
        let Some(event) = self.get_event(event_id) else {
            return Ok(None);
        };
        if policy.can_read_event(requester, &event) {
            Ok(Some(event))
        } else {
            Ok(None)
        }
    }

    /// Enumerate a transaction using both query authorization and per-event
    /// filtering. Results are returned in timestamp order.
    pub fn transaction_events<P: ActivityAccessPolicy>(
        &self,
        transaction_id: TransactionId,
        requester: &ActorRef,
        policy: &P,
    ) -> Result<Vec<ActivityEvent>, AuthorizedQueryError<P::Error>> {
        self.query(
            requester,
            &ActivityQuery {
                transaction_id: Some(transaction_id),
                ..ActivityQuery::default()
            },
            policy,
        )
    }

    pub(crate) fn get_event(&self, event_id: EventId) -> Option<ActivityEvent> {
        if self.is_event_redacted(event_id) {
            None
        } else {
            self.store.read_event(event_id)
        }
    }

    pub(crate) fn get_action(&self, action_id: ActionId) -> Option<ActionRecord> {
        let events = self.store.all_events();
        action_from_events(&events, action_id)
            .filter(|action| !self.is_event_redacted(action.event_id))
    }

    pub(crate) fn resolve_snapshot(&self, snapshot_id: SnapshotId) -> Option<SnapshotReference> {
        let events = self.store.all_events();
        events.iter().find_map(|event| {
            if self.is_event_redacted(event.event_id) {
                return None;
            }
            match &event.kind {
                ActivityEventKind::SnapshotReferenceCreated(reference)
                    if reference.id == snapshot_id =>
                {
                    Some((**reference).clone())
                }
                _ => None,
            }
        })
    }

    pub(crate) fn transaction(&self, transaction_id: TransactionId) -> Option<TransactionRecord> {
        let events = self.store.all_events();
        let started = events.iter().find(|event| {
            !self.is_event_redacted(event.event_id)
                && matches!(
                    &event.kind,
                    ActivityEventKind::TransactionStarted { transaction_id: id, .. }
                        if *id == transaction_id
                )
        })?;
        let mut status = TransactionStatus::InProgress;
        let mut finished_at = None;
        for event in &events {
            if self.is_event_redacted(event.event_id) {
                continue;
            }
            if let ActivityEventKind::TransactionStatusChanged {
                transaction_id: id,
                status: next_status,
                ..
            } = &event.kind
            {
                if *id == transaction_id {
                    status = *next_status;
                    finished_at = Some(event.context.timestamp);
                }
            }
        }
        let actions = events
            .iter()
            .filter_map(|event| match &event.kind {
                ActivityEventKind::ActionCreated(data)
                    if event.context.transaction_id == Some(transaction_id)
                        && !self.is_event_redacted(event.event_id) =>
                {
                    Some(action_record(event, data))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let action_ids = actions.iter().map(|action| action.action_id).collect();
        let snapshot_ids = events
            .iter()
            .filter_map(|event| {
                if self.is_event_redacted(event.event_id) {
                    return None;
                }
                match &event.kind {
                    ActivityEventKind::SnapshotReferenceCreated(reference)
                        if reference.related_transaction == Some(transaction_id) =>
                    {
                        Some(reference.id)
                    }
                    _ => None,
                }
            })
            .collect();
        let reversibility = aggregate_reversibility(&actions);
        Some(TransactionRecord {
            transaction_id,
            actor: started.context.actor.clone(),
            started_at: started.context.timestamp,
            finished_at,
            status,
            action_ids,
            snapshot_ids,
            reversibility,
        })
    }

    fn is_action_reverted(&self, action_id: ActionId) -> bool {
        self.store.all_events().iter().any(|event| {
            matches!(
                &event.kind,
                ActivityEventKind::ActionRevertRecorded {
                    action_id: recorded,
                    outcome: RevertOutcome::Applied,
                    ..
                } if *recorded == action_id
            )
        })
    }

    fn has_action_id(&self, action_id: ActionId) -> bool {
        self.store.all_events().iter().any(|event| {
            matches!(
                &event.kind,
                ActivityEventKind::ActionCreated(data) if data.action_id == action_id
            )
        })
    }

    fn has_transaction_id(&self, transaction_id: TransactionId) -> bool {
        self.store.all_events().iter().any(|event| {
            matches!(
                &event.kind,
                ActivityEventKind::TransactionStarted {
                    transaction_id: id,
                    ..
                } if *id == transaction_id
            )
        })
    }

    fn has_snapshot_id(&self, snapshot_id: SnapshotId) -> bool {
        self.store.all_events().iter().any(|event| {
            matches!(
                &event.kind,
                ActivityEventKind::SnapshotReferenceCreated(reference)
                    if reference.id == snapshot_id
            )
        })
    }

    fn has_pending_revert(&self, action_id: ActionId, excluding: EventId) -> bool {
        let events = self.store.all_events();
        events.iter().any(|request| {
            if request.event_id == excluding
                || !matches!(
                    &request.kind,
                    ActivityEventKind::ActionRevertRequested { action_id: requested }
                        if *requested == action_id
                )
            {
                return false;
            }
            !events.iter().any(|result| {
                result.context.causation_id == Some(request.event_id)
                    && matches!(&result.kind, ActivityEventKind::ActionRevertRecorded { .. })
            })
        })
    }

    fn revert_result(&self, request_id: EventId) -> Option<RevertOutcome> {
        self.store.all_events().iter().find_map(|event| {
            if event.context.causation_id != Some(request_id) {
                return None;
            }
            match &event.kind {
                ActivityEventKind::ActionRevertRecorded { outcome, .. } => Some(*outcome),
                _ => None,
            }
        })
    }

    fn is_event_redacted(&self, event_id: EventId) -> bool {
        self.store.all_events().iter().any(|event| {
            matches!(
                &event.kind,
                ActivityEventKind::RedactionRecorded {
                    target_event_id,
                    ..
                } if *target_event_id == event_id
            )
        })
    }

    fn raw_event(&self, event_id: EventId) -> Option<ActivityEvent> {
        self.store.read_event(event_id)
    }

    fn append_event(&mut self, event: ActivityEvent) -> Result<EventId, LedgerError> {
        if event.schema_version != ACTIVITY_EVENT_SCHEMA_VERSION {
            return Err(LedgerError::UnsupportedSchemaVersion);
        }
        validate_context(&event.context)?;
        let event_id = event.event_id;
        self.store.append(event)?;
        Ok(event_id)
    }

    fn append_with_context(
        &mut self,
        context: EventContext,
        kind: ActivityEventKind,
    ) -> Result<EventId, LedgerError> {
        let event_id = self.next_event_id()?;
        self.append_event(ActivityEvent::new(event_id, context, kind))
    }

    fn next_event_id(&mut self) -> Result<EventId, LedgerError> {
        Ok(EventId(self.ids.next_id()?))
    }
}

fn validate_context(context: &EventContext) -> Result<(), LedgerError> {
    if context.is_valid() {
        Ok(())
    } else {
        Err(LedgerError::InvalidContext)
    }
}

fn validate_optional_key(value: Option<&str>) -> Result<(), LedgerError> {
    if value.is_some_and(|key| !valid_key(key, MAX_SUMMARY_KEY_BYTES)) {
        Err(LedgerError::InvalidSummaryKey)
    } else {
        Ok(())
    }
}

fn validate_query(query: &ActivityQuery) -> Result<(), LedgerError> {
    if query
        .from
        .zip(query.through)
        .is_some_and(|(from, through)| from > through)
        || query.from.is_some_and(|timestamp| !timestamp.is_valid())
        || query.through.is_some_and(|timestamp| !timestamp.is_valid())
    {
        Err(LedgerError::InvalidTimeRange)
    } else {
        Ok(())
    }
}

fn query_matches_context(query: &ActivityQuery, event: &ActivityEvent) -> bool {
    let context = &event.context;
    if query.from.is_some_and(|from| context.timestamp < from)
        || query
            .through
            .is_some_and(|through| context.timestamp > through)
        || query
            .transaction_id
            .is_some_and(|id| event_transaction_id(event) != Some(id))
        || query
            .application_id
            .is_some_and(|id| context.actor.application_id != Some(id))
        || query
            .app_session_id
            .is_some_and(|id| context.origin.app_session_id != Some(id))
        || query
            .node_id
            .is_some_and(|id| context.origin.node_id != Some(id))
        || query
            .workspace_id
            .is_some_and(|id| context.origin.workspace_id != Some(id))
    {
        return false;
    }
    if let Some(filter) = &query.actor {
        if filter.kind.is_some_and(|kind| context.actor.kind != kind)
            || filter
                .identity
                .is_some_and(|identity| context.actor.identity != Some(identity))
            || filter
                .delegated_for
                .is_some_and(|user| context.actor.delegated_for != Some(user))
        {
            return false;
        }
    }
    true
}

fn event_transaction_id(event: &ActivityEvent) -> Option<TransactionId> {
    event.context.transaction_id.or(match &event.kind {
        ActivityEventKind::TransactionStarted { transaction_id, .. }
        | ActivityEventKind::TransactionStatusChanged { transaction_id, .. } => {
            Some(*transaction_id)
        }
        _ => None,
    })
}

fn action_id_for_event(kind: &ActivityEventKind) -> Option<ActionId> {
    match kind {
        ActivityEventKind::ActionCreated(data) => Some(data.action_id),
        ActivityEventKind::ActionOutcomeRecorded { action_id, .. }
        | ActivityEventKind::ActionRevertRequested { action_id }
        | ActivityEventKind::ActionRevertRecorded { action_id, .. } => Some(*action_id),
        _ => None,
    }
}

fn action_from_events(events: &[ActivityEvent], action_id: ActionId) -> Option<ActionRecord> {
    events.iter().find_map(|event| match &event.kind {
        ActivityEventKind::ActionCreated(data) if data.action_id == action_id => {
            Some(action_record(event, data))
        }
        _ => None,
    })
}

fn action_record(event: &ActivityEvent, data: &ActionData) -> ActionRecord {
    ActionRecord {
        event_id: event.event_id,
        context: event.context.clone(),
        action_id: data.action_id,
        action_type: data.action_type.clone(),
        target: data.target.clone(),
        summary_key: data.summary_key.clone(),
        metadata: data.metadata.clone(),
        before: data.before.clone(),
        after: data.after.clone(),
        reversibility: data.reversibility.clone(),
        related_snapshot: data.related_snapshot,
        authorization: data.authorization.clone(),
        provenance: data.provenance.clone(),
    }
}

fn redacted_event_ids(events: &[ActivityEvent]) -> BTreeSet<EventId> {
    events
        .iter()
        .filter_map(|event| match &event.kind {
            ActivityEventKind::RedactionRecorded {
                target_event_id, ..
            } => Some(*target_event_id),
            _ => None,
        })
        .collect()
}

fn is_reversible_for_query<S: ActivityStore, I: ActivityIdSource>(
    ledger: &ActivityLedger<S, I>,
    action: &ActionRecord,
) -> bool {
    match action.reversibility.kind {
        ReversibilityKind::FullyReversible | ReversibilityKind::PartiallyReversible => true,
        ReversibilityKind::ReversibleWithSnapshot => action
            .related_snapshot
            .is_some_and(|snapshot| ledger.resolve_snapshot(snapshot).is_some()),
        ReversibilityKind::Irreversible | ReversibilityKind::Unknown => false,
    }
}

fn aggregate_reversibility(actions: &[ActionRecord]) -> ReversibilityKind {
    if actions.is_empty() {
        return ReversibilityKind::Unknown;
    }
    let mut has_fully = false;
    let mut has_snapshot = false;
    let mut has_partial = false;
    let mut has_irreversible = false;
    let mut has_unknown = false;
    for action in actions {
        match action.reversibility.kind {
            ReversibilityKind::FullyReversible => has_fully = true,
            ReversibilityKind::ReversibleWithSnapshot => has_snapshot = true,
            ReversibilityKind::PartiallyReversible => has_partial = true,
            ReversibilityKind::Irreversible => has_irreversible = true,
            ReversibilityKind::Unknown => has_unknown = true,
        }
    }
    if has_fully && !has_snapshot && !has_partial && !has_irreversible && !has_unknown {
        ReversibilityKind::FullyReversible
    } else if has_snapshot && !has_partial && !has_irreversible && !has_unknown {
        ReversibilityKind::ReversibleWithSnapshot
    } else if has_irreversible && !has_fully && !has_snapshot && !has_partial && !has_unknown {
        ReversibilityKind::Irreversible
    } else if has_unknown && !has_fully && !has_snapshot && !has_partial && !has_irreversible {
        ReversibilityKind::Unknown
    } else {
        ReversibilityKind::PartiallyReversible
    }
}

impl From<crate::CodecError> for LedgerError {
    fn from(value: crate::CodecError) -> Self {
        match value {
            crate::CodecError::UnsupportedSchemaVersion => Self::UnsupportedSchemaVersion,
            _ => Self::InvalidContext,
        }
    }
}
