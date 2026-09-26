//! Checkpoint metadata, restore planning, and backend boundaries.
//!
//! A checkpoint stores references to versioned objects and a backend handle. The
//! snapshot bytes live behind `SnapshotBackend`; this module never treats an
//! Activity event or a checkpoint record as snapshot data.

use crate::activity::RevisionId;
use crate::activity::{
    ActionKind, ActivityDraft, ActivityError, ActivitySink, Actor, CausalParent, CheckpointId,
    CorrelationId, EventId, EventResult, FailureCode, Provenance, Timestamp, TransactionId,
};
use crate::{ActivityContext, ObjectId, WorkspaceId};

pub const MAX_CHECKPOINTS: usize = 64;
pub const MAX_CHECKPOINT_OBJECTS: usize = 8;
pub const MAX_RESTORE_ITEMS: usize = MAX_CHECKPOINT_OBJECTS;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotBackendRef(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointObject {
    object_id: ObjectId,
    revision_id: RevisionId,
}

impl CheckpointObject {
    pub const fn new(object_id: ObjectId, revision_id: RevisionId) -> Self {
        Self {
            object_id,
            revision_id,
        }
    }

    pub const fn object_id(self) -> ObjectId {
        self.object_id
    }

    pub const fn revision_id(self) -> RevisionId {
        self.revision_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointScope {
    Document,
    Workspace,
    Transaction,
    System,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointOrigin {
    User,
    Agent,
    Application,
    Automation,
    System,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointReason {
    UserRequested,
    BeforeAgentChange,
    TransactionBoundary,
    AutomaticHistory,
    SystemMaintenance,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointValidity {
    Available,
    Expired,
    Unavailable,
    Corrupt,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointDraft {
    created_at: Timestamp,
    actor: Actor,
    context: ActivityContext,
    provenance: Provenance,
    scope: CheckpointScope,
    origin: CheckpointOrigin,
    reason: CheckpointReason,
    backend_ref: SnapshotBackendRef,
    objects: [Option<CheckpointObject>; MAX_CHECKPOINT_OBJECTS],
    object_count: u8,
    transaction_id: Option<TransactionId>,
    pinned: bool,
}

impl CheckpointDraft {
    pub const fn new(
        created_at: Timestamp,
        actor: Actor,
        context: ActivityContext,
        scope: CheckpointScope,
        origin: CheckpointOrigin,
        reason: CheckpointReason,
        backend_ref: SnapshotBackendRef,
    ) -> Self {
        Self {
            created_at,
            actor,
            context,
            provenance: Provenance::Direct {
                originating_intent: None,
            },
            scope,
            origin,
            reason,
            backend_ref,
            objects: [None; MAX_CHECKPOINT_OBJECTS],
            object_count: 0,
            transaction_id: None,
            pinned: false,
        }
    }

    pub fn with_object(mut self, object: CheckpointObject) -> Result<Self, CheckpointError> {
        if self.objects[..self.object_count as usize]
            .iter()
            .any(|existing| existing.is_some_and(|item| item.object_id == object.object_id))
        {
            return Ok(self);
        }
        if self.object_count as usize == MAX_CHECKPOINT_OBJECTS {
            return Err(CheckpointError::TooManyObjects);
        }
        self.objects[self.object_count as usize] = Some(object);
        self.object_count += 1;
        Ok(self)
    }

    pub const fn created_at(self) -> Timestamp {
        self.created_at
    }

    pub const fn actor(self) -> Actor {
        self.actor
    }

    pub const fn context(self) -> ActivityContext {
        self.context
    }

    pub const fn provenance(self) -> Provenance {
        self.provenance
    }

    pub const fn scope(self) -> CheckpointScope {
        self.scope
    }

    pub const fn origin(self) -> CheckpointOrigin {
        self.origin
    }

    pub const fn reason(self) -> CheckpointReason {
        self.reason
    }

    pub const fn backend_ref(self) -> SnapshotBackendRef {
        self.backend_ref
    }

    pub const fn transaction_id(self) -> Option<TransactionId> {
        self.transaction_id
    }

    pub const fn is_pinned(self) -> bool {
        self.pinned
    }

    pub const fn object_count(self) -> usize {
        self.object_count as usize
    }

    pub fn objects(&self) -> impl Iterator<Item = CheckpointObject> + '_ {
        self.objects[..self.object_count as usize]
            .iter()
            .filter_map(|object| *object)
    }

    pub fn creation_activity(self, id: CheckpointId) -> Result<ActivityDraft, CheckpointError> {
        let draft = self.validate()?;
        CheckpointId::new(id.0).map_err(|_| CheckpointError::InvalidId)?;
        let mut event = ActivityDraft::new(
            draft.created_at,
            draft.actor,
            draft.context,
            ActionKind::CheckpointCreated,
        )
        .with_result(EventResult::Succeeded)
        .with_provenance(draft.provenance)
        .with_checkpoint_after(id);
        if let Some(transaction_id) = draft.transaction_id {
            event = event.with_transaction(transaction_id);
        }
        event = event
            .with_metadata(
                crate::activity::MetadataKey::ItemCount,
                crate::activity::MetadataValue::Count(draft.object_count as u32),
                crate::activity::PrivacyClass::PublicMetadata,
            )
            .map_err(CheckpointError::Activity)?;
        Ok(event)
    }

    /// Validate and materialize a durable checkpoint record with its stored ID.
    pub fn into_record(self, id: CheckpointId) -> Result<CheckpointRecord, CheckpointError> {
        CheckpointId::new(id.0).map_err(|_| CheckpointError::InvalidId)?;
        let draft = self.validate()?;
        Ok(CheckpointRecord {
            id,
            created_at: draft.created_at,
            actor: draft.actor,
            context: draft.context,
            provenance: draft.provenance,
            scope: draft.scope,
            origin: draft.origin,
            reason: draft.reason,
            backend_ref: draft.backend_ref,
            objects: draft.objects,
            object_count: draft.object_count,
            transaction_id: draft.transaction_id,
            validity: CheckpointValidity::Available,
            pinned: draft.pinned,
        })
    }

    pub const fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = provenance;
        self
    }

    pub const fn with_transaction(mut self, transaction_id: TransactionId) -> Self {
        self.transaction_id = Some(transaction_id);
        self
    }

    pub const fn pinned(mut self) -> Self {
        self.pinned = true;
        self
    }

    /// Set the persisted pin state while rebuilding a draft from adapter data.
    /// Durable providers must still route user changes through
    /// `CheckpointWriteStore::set_checkpoint_pin` and its mutation policy.
    pub const fn with_pinned_state(mut self, pinned: bool) -> Self {
        self.pinned = pinned;
        self
    }

    fn validate(self) -> Result<Self, CheckpointError> {
        if self.backend_ref.0 == 0 {
            return Err(CheckpointError::InvalidBackendReference);
        }
        if self.object_count == 0 {
            return Err(CheckpointError::EmptyCheckpoint);
        }
        if self.scope == CheckpointScope::Document && self.object_count != 1 {
            return Err(CheckpointError::InvalidScopeObjects);
        }
        if self.scope == CheckpointScope::Workspace && self.context.workspace_id.is_none() {
            return Err(CheckpointError::MissingWorkspace);
        }
        if self.scope == CheckpointScope::Transaction && self.transaction_id.is_none() {
            return Err(CheckpointError::MissingTransaction);
        }
        if self.actor.kind() == crate::activity::ActorKind::Agent
            && !matches!(self.provenance, Provenance::Delegated { .. })
        {
            return Err(CheckpointError::MissingDelegationProvenance);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointRecord {
    id: CheckpointId,
    created_at: Timestamp,
    actor: Actor,
    context: ActivityContext,
    provenance: Provenance,
    scope: CheckpointScope,
    origin: CheckpointOrigin,
    reason: CheckpointReason,
    backend_ref: SnapshotBackendRef,
    objects: [Option<CheckpointObject>; MAX_CHECKPOINT_OBJECTS],
    object_count: u8,
    transaction_id: Option<TransactionId>,
    validity: CheckpointValidity,
    pinned: bool,
}

impl CheckpointRecord {
    pub const fn id(&self) -> CheckpointId {
        self.id
    }

    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub const fn actor(&self) -> Actor {
        self.actor
    }

    pub const fn context(&self) -> ActivityContext {
        self.context
    }

    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub const fn scope(&self) -> CheckpointScope {
        self.scope
    }

    pub const fn origin(&self) -> CheckpointOrigin {
        self.origin
    }

    pub const fn reason(&self) -> CheckpointReason {
        self.reason
    }

    pub const fn backend_ref(&self) -> SnapshotBackendRef {
        self.backend_ref
    }

    pub const fn transaction_id(&self) -> Option<TransactionId> {
        self.transaction_id
    }

    pub const fn validity(&self) -> CheckpointValidity {
        self.validity
    }

    pub const fn is_pinned(&self) -> bool {
        self.pinned
    }

    pub fn objects(&self) -> impl Iterator<Item = CheckpointObject> + '_ {
        self.objects[..self.object_count as usize]
            .iter()
            .filter_map(|object| *object)
    }

    pub fn object(&self, object_id: ObjectId) -> Option<CheckpointObject> {
        self.objects().find(|object| object.object_id == object_id)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointError {
    Capacity,
    Activity(ActivityError),
    InvalidId,
    InvalidBackendReference,
    EmptyCheckpoint,
    InvalidScopeObjects,
    MissingWorkspace,
    MissingTransaction,
    MissingDelegationProvenance,
    TooManyObjects,
    NotFound,
    PermissionDenied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointPinChange {
    Changed(EventId),
    Unchanged,
}

pub trait CheckpointMutationPolicy {
    fn can_change_pin(&self, actor: Actor, checkpoint: &CheckpointRecord, pinned: bool) -> bool;
}

pub struct CheckpointStore {
    records: [Option<CheckpointRecord>; MAX_CHECKPOINTS],
    length: usize,
    next_id: u64,
}

impl CheckpointStore {
    pub const fn new() -> Self {
        Self {
            records: [None; MAX_CHECKPOINTS],
            length: 0,
            next_id: 1,
        }
    }

    pub const fn len(&self) -> usize {
        self.length
    }

    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn can_create(&self) -> bool {
        self.next_id != u64::MAX
            && (self.length < MAX_CHECKPOINTS || self.oldest_unpinned_index().is_some())
    }

    pub(crate) fn get(&self, id: CheckpointId) -> Option<&CheckpointRecord> {
        self.records[..MAX_CHECKPOINTS]
            .iter()
            .filter_map(|record| record.as_ref())
            .find(|record| record.id == id)
    }

    pub fn get_visible(
        &self,
        id: CheckpointId,
        viewer: Actor,
        policy: &dyn CheckpointAccessPolicy,
    ) -> Option<&CheckpointRecord> {
        self.get(id)
            .filter(|record| policy.can_read(viewer, record))
    }

    /// Record checkpoint metadata and its Activity entry. Full stores evict the
    /// oldest unpinned checkpoint only; pinned records are never auto-pruned.
    pub fn create(
        &mut self,
        draft: CheckpointDraft,
        activity: &mut dyn ActivitySink,
    ) -> Result<(CheckpointId, EventId), CheckpointError> {
        let draft = draft.validate()?;
        if !self.can_create() || !activity.can_append() {
            return Err(CheckpointError::Capacity);
        }
        let id = CheckpointId::new(self.next_id).map_err(|_| CheckpointError::InvalidId)?;
        let destination = self
            .records
            .iter()
            .position(Option::is_none)
            .or_else(|| self.oldest_unpinned_index())
            .ok_or(CheckpointError::Capacity)?;
        let record = draft.into_record(id)?;
        let event = draft.creation_activity(id)?;
        let event_id = activity.append(event).map_err(CheckpointError::Activity)?;
        if self.records[destination].is_none() {
            self.length += 1;
        }
        self.records[destination] = Some(record);
        self.next_id += 1;
        Ok((id, event_id))
    }

    /// Pin state is user-visible Activity and is changed only after policy and
    /// ledger capacity checks succeed. Repeating the current state is a no-op.
    pub fn set_pinned(
        &mut self,
        id: CheckpointId,
        pinned: bool,
        occurred_at: Timestamp,
        actor: Actor,
        provenance: Provenance,
        policy: &dyn CheckpointMutationPolicy,
        activity: &mut dyn ActivitySink,
    ) -> Result<CheckpointPinChange, CheckpointError> {
        let index = self.records[..MAX_CHECKPOINTS]
            .iter()
            .position(|record| record.is_some_and(|record| record.id == id))
            .ok_or(CheckpointError::NotFound)?;
        let record = self.records[index].ok_or(CheckpointError::NotFound)?;
        if !policy.can_change_pin(actor, &record, pinned) {
            return Err(CheckpointError::PermissionDenied);
        }
        if record.pinned == pinned {
            return Ok(CheckpointPinChange::Unchanged);
        }
        if !activity.can_append() {
            return Err(CheckpointError::Capacity);
        }
        let action = if pinned {
            ActionKind::CheckpointPinned
        } else {
            ActionKind::CheckpointUnpinned
        };
        let mut event = ActivityDraft::new(occurred_at, actor, record.context, action)
            .with_result(EventResult::Succeeded)
            .with_provenance(provenance)
            .with_checkpoint_after(id)
            .with_metadata(
                crate::activity::MetadataKey::ItemCount,
                crate::activity::MetadataValue::Count(record.object_count as u32),
                crate::activity::PrivacyClass::PublicMetadata,
            )
            .map_err(CheckpointError::Activity)?;
        if let Some(transaction_id) = record.transaction_id {
            event = event.with_transaction(transaction_id);
        }
        self.records[index]
            .as_mut()
            .ok_or(CheckpointError::NotFound)?
            .pinned = pinned;
        let event_id = match activity.append(event) {
            Ok(event_id) => event_id,
            Err(error) => {
                if let Some(record) = self.records[index].as_mut() {
                    record.pinned = !pinned;
                }
                return Err(CheckpointError::Activity(error));
            }
        };
        Ok(CheckpointPinChange::Changed(event_id))
    }

    #[cfg(test)]
    pub(crate) fn query(&self, filter: CheckpointQuery) -> CheckpointQueryIter<'_> {
        CheckpointQueryIter {
            store: self,
            filter,
            viewer: None,
            policy: None,
            cursor: None,
        }
    }

    pub fn query_visible<'a>(
        &'a self,
        filter: CheckpointQuery,
        viewer: Actor,
        policy: &'a dyn CheckpointAccessPolicy,
    ) -> CheckpointQueryIter<'a> {
        CheckpointQueryIter {
            store: self,
            filter,
            viewer: Some(viewer),
            policy: Some(policy),
            cursor: None,
        }
    }

    fn oldest_unpinned_index(&self) -> Option<usize> {
        self.records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| record.as_ref().map(|record| (index, record)))
            .filter(|(_, record)| !record.pinned)
            .min_by_key(|(_, record)| (record.created_at, record.id.0))
            .map(|(index, _)| index)
    }
}

impl Default for CheckpointStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CheckpointVersionRequest {
    opened_by: Actor,
    checkpoint_id: CheckpointId,
    object_id: ObjectId,
    revision_id: RevisionId,
    backend_ref: SnapshotBackendRef,
    context: ActivityContext,
}

impl CheckpointVersionRequest {
    pub const fn opened_by(self) -> Actor {
        self.opened_by
    }

    pub const fn checkpoint_id(self) -> CheckpointId {
        self.checkpoint_id
    }

    pub const fn object_id(self) -> ObjectId {
        self.object_id
    }

    pub const fn revision_id(self) -> RevisionId {
        self.revision_id
    }

    pub const fn backend_ref(self) -> SnapshotBackendRef {
        self.backend_ref
    }

    pub const fn context(self) -> ActivityContext {
        self.context
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointVersionOpenError {
    CheckpointUnavailable,
    ObjectNotInCheckpoint,
    Denied(FailureCode),
}

pub trait CheckpointVersionOpenPolicy {
    fn authorize_open(
        &self,
        actor: Actor,
        checkpoint: &CheckpointRecord,
        object: ObjectId,
    ) -> Result<(), FailureCode>;
}

/// The resource provider opens a version by opaque ObjectId/revision reference
/// and performs its own capability validation. No host path is accepted here.
pub trait CheckpointVersionOpener {
    fn open_version(&mut self, request: CheckpointVersionRequest) -> Result<(), FailureCode>;
}

pub fn open_checkpoint_version<S: CheckpointReadStore>(
    store: &S,
    checkpoint_id: CheckpointId,
    object_id: ObjectId,
    viewer: Actor,
    visibility: &dyn CheckpointAccessPolicy,
    policy: &dyn CheckpointVersionOpenPolicy,
    opener: &mut dyn CheckpointVersionOpener,
) -> Result<(), CheckpointVersionOpenError> {
    let checkpoint = store
        .get_checkpoint_visible(checkpoint_id, viewer, visibility)
        .filter(|record| record.validity == CheckpointValidity::Available)
        .ok_or(CheckpointVersionOpenError::CheckpointUnavailable)?;
    let object = checkpoint
        .object(object_id)
        .ok_or(CheckpointVersionOpenError::ObjectNotInCheckpoint)?;
    policy
        .authorize_open(viewer, checkpoint, object_id)
        .map_err(CheckpointVersionOpenError::Denied)?;
    opener
        .open_version(CheckpointVersionRequest {
            opened_by: viewer,
            checkpoint_id,
            object_id,
            revision_id: object.revision_id,
            backend_ref: checkpoint.backend_ref,
            context: checkpoint.context,
        })
        .map_err(CheckpointVersionOpenError::Denied)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionDraft {
    object_id: ObjectId,
    parent: Option<RevisionId>,
    created_at: Timestamp,
    actor: Actor,
    provenance: Provenance,
    context: ActivityContext,
    backend_ref: SnapshotBackendRef,
    transaction_id: Option<TransactionId>,
}

impl RevisionDraft {
    pub const fn new(
        object_id: ObjectId,
        parent: Option<RevisionId>,
        created_at: Timestamp,
        actor: Actor,
        context: ActivityContext,
        backend_ref: SnapshotBackendRef,
    ) -> Self {
        Self {
            object_id,
            parent,
            created_at,
            actor,
            provenance: Provenance::Direct {
                originating_intent: None,
            },
            context,
            backend_ref,
            transaction_id: None,
        }
    }

    pub const fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = provenance;
        self
    }

    pub const fn with_transaction(mut self, transaction: TransactionId) -> Self {
        self.transaction_id = Some(transaction);
        self
    }

    /// Validate and materialize a durable revision record with its stored ID.
    pub fn into_record(self, id: RevisionId) -> Result<RevisionRecord, RevisionError> {
        RevisionId::new(id.0).map_err(|_| RevisionError::InvalidId)?;
        let draft = self.validate()?;
        Ok(RevisionRecord {
            id,
            object_id: draft.object_id,
            parent: draft.parent,
            created_at: draft.created_at,
            actor: draft.actor,
            provenance: draft.provenance,
            context: draft.context,
            backend_ref: draft.backend_ref,
            transaction_id: draft.transaction_id,
        })
    }

    fn validate(self) -> Result<Self, RevisionError> {
        if self.backend_ref.0 == 0 {
            return Err(RevisionError::InvalidBackendReference);
        }
        if self.actor.kind() == crate::activity::ActorKind::Agent
            && !matches!(self.provenance, Provenance::Delegated { .. })
        {
            return Err(RevisionError::MissingDelegationProvenance);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RevisionRecord {
    id: RevisionId,
    object_id: ObjectId,
    parent: Option<RevisionId>,
    created_at: Timestamp,
    actor: Actor,
    provenance: Provenance,
    context: ActivityContext,
    backend_ref: SnapshotBackendRef,
    transaction_id: Option<TransactionId>,
}

impl RevisionRecord {
    pub const fn id(&self) -> RevisionId {
        self.id
    }

    pub const fn object_id(&self) -> ObjectId {
        self.object_id
    }

    pub const fn parent(&self) -> Option<RevisionId> {
        self.parent
    }

    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub const fn actor(&self) -> Actor {
        self.actor
    }

    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub const fn context(&self) -> ActivityContext {
        self.context
    }

    pub const fn backend_ref(&self) -> SnapshotBackendRef {
        self.backend_ref
    }

    pub const fn transaction_id(&self) -> Option<TransactionId> {
        self.transaction_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevisionError {
    Capacity,
    InvalidId,
    InvalidBackendReference,
    MissingDelegationProvenance,
    MissingParent,
    ParentObjectMismatch,
}

pub const MAX_REVISIONS: usize = 128;

/// Bounded semantic revision index. Version bytes remain in a snapshot/version
/// backend and are referenced by `SnapshotBackendRef`.
pub struct RevisionStore {
    records: [Option<RevisionRecord>; MAX_REVISIONS],
    length: usize,
    next_id: u64,
}

impl RevisionStore {
    pub const fn new() -> Self {
        Self {
            records: [None; MAX_REVISIONS],
            length: 0,
            next_id: 1,
        }
    }

    pub const fn len(&self) -> usize {
        self.length
    }

    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub fn append(&mut self, draft: RevisionDraft) -> Result<RevisionId, RevisionError> {
        if self.length == MAX_REVISIONS || self.next_id == u64::MAX {
            return Err(RevisionError::Capacity);
        }
        let draft = draft.validate()?;
        if let Some(parent_id) = draft.parent {
            let Some(parent) = self.get(parent_id) else {
                return Err(RevisionError::MissingParent);
            };
            if parent.object_id != draft.object_id {
                return Err(RevisionError::ParentObjectMismatch);
            }
        }
        let id = RevisionId::new(self.next_id).map_err(|_| RevisionError::InvalidId)?;
        let record = draft.into_record(id)?;
        self.records[self.length] = Some(record);
        self.length += 1;
        self.next_id += 1;
        Ok(id)
    }

    pub(crate) fn get(&self, id: RevisionId) -> Option<&RevisionRecord> {
        self.records[..self.length]
            .iter()
            .filter_map(|record| record.as_ref())
            .find(|record| record.id == id)
    }

    pub fn get_visible(
        &self,
        id: RevisionId,
        viewer: Actor,
        policy: &dyn RevisionAccessPolicy,
    ) -> Option<&RevisionRecord> {
        self.get(id)
            .filter(|record| policy.can_read(viewer, record))
    }

    #[cfg(test)]
    pub(crate) fn query(&self, filter: RevisionQuery) -> RevisionQueryIter<'_> {
        RevisionQueryIter {
            store: self,
            filter,
            viewer: None,
            policy: None,
            cursor: None,
        }
    }

    pub fn query_visible<'a>(
        &'a self,
        filter: RevisionQuery,
        viewer: Actor,
        policy: &'a dyn RevisionAccessPolicy,
    ) -> RevisionQueryIter<'a> {
        RevisionQueryIter {
            store: self,
            filter,
            viewer: Some(viewer),
            policy: Some(policy),
            cursor: None,
        }
    }
}

impl Default for RevisionStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RevisionQuery {
    object: Option<ObjectId>,
    actor: Option<Actor>,
    workspace: Option<WorkspaceId>,
    transaction: Option<TransactionId>,
    time_range: Option<crate::activity::ActivityTimeRange>,
}

impl RevisionQuery {
    pub const fn object(mut self, object: ObjectId) -> Self {
        self.object = Some(object);
        self
    }

    pub const fn actor(mut self, actor: Actor) -> Self {
        self.actor = Some(actor);
        self
    }

    pub const fn workspace(mut self, workspace: WorkspaceId) -> Self {
        self.workspace = Some(workspace);
        self
    }

    pub const fn transaction(mut self, transaction: TransactionId) -> Self {
        self.transaction = Some(transaction);
        self
    }

    pub const fn time_range(mut self, range: crate::activity::ActivityTimeRange) -> Self {
        self.time_range = Some(range);
        self
    }

    pub fn matches(self, record: &RevisionRecord) -> bool {
        !self.object.is_some_and(|object| record.object_id != object)
            && !self.actor.is_some_and(|actor| record.actor != actor)
            && !self
                .workspace
                .is_some_and(|workspace| record.context.workspace_id != Some(workspace))
            && !self
                .transaction
                .is_some_and(|transaction| record.transaction_id != Some(transaction))
            && !self.time_range.is_some_and(|range| {
                record.created_at < range.start_inclusive()
                    || record.created_at >= range.end_exclusive()
            })
    }
}

pub struct RevisionQueryIter<'a> {
    store: &'a RevisionStore,
    filter: RevisionQuery,
    viewer: Option<Actor>,
    policy: Option<&'a dyn RevisionAccessPolicy>,
    cursor: Option<(Timestamp, RevisionId)>,
}

pub trait RevisionAccessPolicy {
    fn can_read(&self, viewer: Actor, revision: &RevisionRecord) -> bool;
}

impl<'a> Iterator for RevisionQueryIter<'a> {
    type Item = &'a RevisionRecord;

    fn next(&mut self) -> Option<Self::Item> {
        let mut next: Option<&RevisionRecord> = None;
        for record in self.store.records[..self.store.length]
            .iter()
            .filter_map(|record| record.as_ref())
        {
            let key = (record.created_at, record.id);
            if self.cursor.is_some_and(|cursor| key <= cursor)
                || !self.filter.matches(record)
                || self
                    .policy
                    .zip(self.viewer)
                    .is_some_and(|(policy, viewer)| !policy.can_read(viewer, record))
            {
                continue;
            }
            if next.is_none_or(|current| key < (current.created_at, current.id)) {
                next = Some(record);
            }
        }
        if let Some(record) = next {
            self.cursor = Some((record.created_at, record.id));
        }
        next
    }
}

pub trait RevisionReadStore {
    type QueryIter<'a>: Iterator<Item = &'a RevisionRecord>
    where
        Self: 'a;

    /// The viewer and policy must be supplied by a trusted service boundary;
    /// they are not an OS capability check on their own.
    fn get_revision_visible(
        &self,
        id: RevisionId,
        viewer: Actor,
        policy: &dyn RevisionAccessPolicy,
    ) -> Option<&RevisionRecord>;

    fn query_visible<'a>(
        &'a self,
        filter: RevisionQuery,
        viewer: Actor,
        policy: &'a dyn RevisionAccessPolicy,
    ) -> Self::QueryIter<'a>;
}

/// Mutation contract for durable semantic revision indexes. Snapshot bytes
/// remain owned by the referenced version backend.
pub trait RevisionWriteStore: RevisionReadStore {
    fn can_append_revision(&self) -> bool;

    fn append_revision(&mut self, draft: RevisionDraft) -> Result<RevisionId, RevisionError>;
}

impl RevisionReadStore for RevisionStore {
    type QueryIter<'a>
        = RevisionQueryIter<'a>
    where
        Self: 'a;

    fn get_revision_visible(
        &self,
        id: RevisionId,
        viewer: Actor,
        policy: &dyn RevisionAccessPolicy,
    ) -> Option<&RevisionRecord> {
        RevisionStore::get_visible(self, id, viewer, policy)
    }

    fn query_visible<'a>(
        &'a self,
        filter: RevisionQuery,
        viewer: Actor,
        policy: &'a dyn RevisionAccessPolicy,
    ) -> Self::QueryIter<'a> {
        RevisionStore::query_visible(self, filter, viewer, policy)
    }
}

impl RevisionWriteStore for RevisionStore {
    fn can_append_revision(&self) -> bool {
        self.length < MAX_REVISIONS && self.next_id != u64::MAX
    }

    fn append_revision(&mut self, draft: RevisionDraft) -> Result<RevisionId, RevisionError> {
        RevisionStore::append(self, draft)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffSummary {
    Unchanged,
    Changed {
        current_bytes: u32,
        target_bytes: u32,
    },
    MissingCurrent {
        target_bytes: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffError {
    Unsupported,
    MissingRevision,
    PermissionDenied,
    BackendFailure,
}

pub trait DiffProvider {
    fn compare(
        &self,
        backend_ref: SnapshotBackendRef,
        object: ObjectId,
        current_revision: Option<RevisionId>,
        target_revision: RevisionId,
    ) -> Result<DiffSummary, DiffError>;
}

pub trait DiffAccessPolicy {
    fn can_compare(&self, viewer: Actor, object: ObjectId) -> bool;
}

pub fn compare_with_policy(
    provider: &dyn DiffProvider,
    policy: &dyn DiffAccessPolicy,
    viewer: Actor,
    backend_ref: SnapshotBackendRef,
    object: ObjectId,
    current_revision: Option<RevisionId>,
    target_revision: RevisionId,
) -> Result<DiffSummary, DiffError> {
    if !policy.can_compare(viewer, object) {
        return Err(DiffError::PermissionDenied);
    }
    provider.compare(backend_ref, object, current_revision, target_revision)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CheckpointQuery {
    time_range: Option<crate::activity::ActivityTimeRange>,
    workspace: Option<WorkspaceId>,
    object: Option<ObjectId>,
    transaction: Option<TransactionId>,
    pinned_only: bool,
}

impl CheckpointQuery {
    pub const fn time_range(mut self, range: crate::activity::ActivityTimeRange) -> Self {
        self.time_range = Some(range);
        self
    }

    pub const fn workspace(mut self, workspace: WorkspaceId) -> Self {
        self.workspace = Some(workspace);
        self
    }

    pub const fn object(mut self, object: ObjectId) -> Self {
        self.object = Some(object);
        self
    }

    pub const fn transaction(mut self, transaction: TransactionId) -> Self {
        self.transaction = Some(transaction);
        self
    }

    pub const fn pinned_only(mut self) -> Self {
        self.pinned_only = true;
        self
    }

    pub fn matches(self, record: &CheckpointRecord) -> bool {
        if self.time_range.is_some_and(|range| {
            record.created_at < range.start_inclusive()
                || record.created_at >= range.end_exclusive()
        }) {
            return false;
        }
        self.workspace
            .is_none_or(|workspace| record.context.workspace_id == Some(workspace))
            && self
                .object
                .is_none_or(|object| record.object(object).is_some())
            && self
                .transaction
                .is_none_or(|transaction| record.transaction_id == Some(transaction))
            && (!self.pinned_only || record.pinned)
            && record.validity == CheckpointValidity::Available
    }
}

pub struct CheckpointQueryIter<'a> {
    store: &'a CheckpointStore,
    filter: CheckpointQuery,
    viewer: Option<Actor>,
    policy: Option<&'a dyn CheckpointAccessPolicy>,
    cursor: Option<(Timestamp, CheckpointId)>,
}

pub trait CheckpointAccessPolicy {
    fn can_read(&self, viewer: Actor, checkpoint: &CheckpointRecord) -> bool;
}

impl<'a> Iterator for CheckpointQueryIter<'a> {
    type Item = &'a CheckpointRecord;

    fn next(&mut self) -> Option<Self::Item> {
        let mut next: Option<&CheckpointRecord> = None;
        for record in self
            .store
            .records
            .iter()
            .filter_map(|record| record.as_ref())
        {
            let key = (record.created_at, record.id);
            if self.cursor.is_some_and(|cursor| key <= cursor)
                || !self.filter.matches(record)
                || self
                    .policy
                    .zip(self.viewer)
                    .is_some_and(|(policy, viewer)| !policy.can_read(viewer, record))
            {
                continue;
            }
            if next.is_none_or(|current| key < (current.created_at, current.id)) {
                next = Some(record);
            }
        }
        if let Some(record) = next {
            self.cursor = Some((record.created_at, record.id));
        }
        next
    }
}

/// Read contract for target or durable checkpoint stores. Checkpoint records
/// still contain only version references; snapshot bytes remain in a backend.
/// Providers must validate that the referenced snapshot is currently available
/// before returning a record, so callers cannot turn an unavailable checkpoint
/// into an eligible restore by changing a copied record. Viewer identity and
/// access policy come from a trusted service boundary.
pub trait CheckpointReadStore {
    type QueryIter<'a>: Iterator<Item = &'a CheckpointRecord>
    where
        Self: 'a;

    fn get_checkpoint_visible(
        &self,
        id: CheckpointId,
        viewer: Actor,
        policy: &dyn CheckpointAccessPolicy,
    ) -> Option<&CheckpointRecord>;

    fn query_visible<'a>(
        &'a self,
        filter: CheckpointQuery,
        viewer: Actor,
        policy: &'a dyn CheckpointAccessPolicy,
    ) -> Self::QueryIter<'a>;
}

/// Mutation contract kept separate from the record reader so providers can
/// persist creates and authorized pin state without changing the UI. Backend
/// validity is maintained inside the trusted provider, not by app callers.
pub trait CheckpointWriteStore: CheckpointReadStore {
    fn can_create_checkpoint(&self) -> bool;

    fn create_checkpoint(
        &mut self,
        draft: CheckpointDraft,
        activity: &mut dyn ActivitySink,
    ) -> Result<(CheckpointId, EventId), CheckpointError>;

    fn set_checkpoint_pin(
        &mut self,
        id: CheckpointId,
        pinned: bool,
        occurred_at: Timestamp,
        actor: Actor,
        provenance: Provenance,
        policy: &dyn CheckpointMutationPolicy,
        activity: &mut dyn ActivitySink,
    ) -> Result<CheckpointPinChange, CheckpointError>;
}

impl CheckpointReadStore for CheckpointStore {
    type QueryIter<'a>
        = CheckpointQueryIter<'a>
    where
        Self: 'a;

    fn get_checkpoint_visible(
        &self,
        id: CheckpointId,
        viewer: Actor,
        policy: &dyn CheckpointAccessPolicy,
    ) -> Option<&CheckpointRecord> {
        CheckpointStore::get_visible(self, id, viewer, policy)
    }

    fn query_visible<'a>(
        &'a self,
        filter: CheckpointQuery,
        viewer: Actor,
        policy: &'a dyn CheckpointAccessPolicy,
    ) -> Self::QueryIter<'a> {
        CheckpointStore::query_visible(self, filter, viewer, policy)
    }
}

impl CheckpointWriteStore for CheckpointStore {
    fn can_create_checkpoint(&self) -> bool {
        CheckpointStore::can_create(self)
    }

    fn create_checkpoint(
        &mut self,
        draft: CheckpointDraft,
        activity: &mut dyn ActivitySink,
    ) -> Result<(CheckpointId, EventId), CheckpointError> {
        CheckpointStore::create(self, draft, activity)
    }

    fn set_checkpoint_pin(
        &mut self,
        id: CheckpointId,
        pinned: bool,
        occurred_at: Timestamp,
        actor: Actor,
        provenance: Provenance,
        policy: &dyn CheckpointMutationPolicy,
        activity: &mut dyn ActivitySink,
    ) -> Result<CheckpointPinChange, CheckpointError> {
        CheckpointStore::set_pinned(
            self,
            id,
            pinned,
            occurred_at,
            actor,
            provenance,
            policy,
            activity,
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CurrentObjectRevision {
    object_id: ObjectId,
    revision_id: RevisionId,
}

impl CurrentObjectRevision {
    pub const fn new(object_id: ObjectId, revision_id: RevisionId) -> Self {
        Self {
            object_id,
            revision_id,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreMode {
    InPlace,
    AsCopy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestorePlanId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoreItem {
    object_id: ObjectId,
    current_revision: Option<RevisionId>,
    target_revision: RevisionId,
}

impl RestoreItem {
    pub const fn object_id(self) -> ObjectId {
        self.object_id
    }

    pub const fn current_revision(self) -> Option<RevisionId> {
        self.current_revision
    }

    pub const fn target_revision(self) -> RevisionId {
        self.target_revision
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct RestorePlan {
    id: RestorePlanId,
    checkpoint_id: CheckpointId,
    backend_ref: SnapshotBackendRef,
    checkpoint_scope: CheckpointScope,
    checkpoint_transaction_id: Option<TransactionId>,
    mode: RestoreMode,
    items: [Option<RestoreItem>; MAX_RESTORE_ITEMS],
    item_count: u8,
    actor: Actor,
    provenance: Provenance,
    context: ActivityContext,
    created_at: Timestamp,
    correlation: CorrelationId,
    plan_event_id: EventId,
}

impl RestorePlan {
    pub const fn id(&self) -> RestorePlanId {
        self.id
    }

    pub const fn checkpoint_id(&self) -> CheckpointId {
        self.checkpoint_id
    }

    pub const fn backend_ref(&self) -> SnapshotBackendRef {
        self.backend_ref
    }

    pub const fn checkpoint_scope(&self) -> CheckpointScope {
        self.checkpoint_scope
    }

    pub const fn mode(&self) -> RestoreMode {
        self.mode
    }

    pub const fn actor(&self) -> Actor {
        self.actor
    }

    pub const fn context(&self) -> ActivityContext {
        self.context
    }

    pub const fn created_at(&self) -> Timestamp {
        self.created_at
    }

    pub const fn correlation(&self) -> CorrelationId {
        self.correlation
    }

    pub const fn plan_event_id(&self) -> EventId {
        self.plan_event_id
    }

    pub const fn item_count(&self) -> usize {
        self.item_count as usize
    }

    pub fn items(&self) -> impl Iterator<Item = RestoreItem> + '_ {
        self.items[..self.item_count as usize]
            .iter()
            .filter_map(|item| *item)
    }

    /// A UI must explicitly acknowledge that external effects are not reversed
    /// before it can produce the confirmation token consumed by the executor.
    pub fn confirm(
        self,
        confirmed_by: Actor,
        acknowledged_external_effects: bool,
    ) -> Result<ConfirmedRestorePlan, RestoreError> {
        if confirmed_by.kind() != crate::activity::ActorKind::User {
            return Err(RestoreError::HumanConfirmationRequired);
        }
        if !acknowledged_external_effects {
            return Err(RestoreError::ExternalEffectsNotAcknowledged);
        }
        Ok(ConfirmedRestorePlan {
            plan: self,
            confirmed_by,
        })
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct ConfirmedRestorePlan {
    plan: RestorePlan,
    confirmed_by: Actor,
}

impl ConfirmedRestorePlan {
    pub const fn plan(&self) -> &RestorePlan {
        &self.plan
    }

    pub const fn confirmed_by(&self) -> Actor {
        self.confirmed_by
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreError {
    Activity(ActivityError),
    CheckpointUnavailable,
    EmptySelection,
    TooManyItems,
    ObjectNotInCheckpoint,
    DuplicatePlanObject,
    Capacity,
    HumanConfirmationRequired,
    ExternalEffectsNotAcknowledged,
}

/// Build and record a restore preview. No object state changes in this step.
pub fn prepare_restore_plan(
    id: RestorePlanId,
    checkpoint: &CheckpointRecord,
    actor: Actor,
    provenance: Provenance,
    context: ActivityContext,
    created_at: Timestamp,
    mode: RestoreMode,
    selected_objects: &[ObjectId],
    current: &[CurrentObjectRevision],
    correlation: CorrelationId,
    activity: &mut dyn ActivitySink,
) -> Result<RestorePlan, RestoreError> {
    if checkpoint.validity != CheckpointValidity::Available || checkpoint.backend_ref.0 == 0 {
        return Err(RestoreError::CheckpointUnavailable);
    }
    if selected_objects.is_empty() {
        return Err(RestoreError::EmptySelection);
    }
    if selected_objects.len() > MAX_RESTORE_ITEMS {
        return Err(RestoreError::TooManyItems);
    }
    if mode == RestoreMode::AsCopy && selected_objects.len() > crate::activity::MAX_ACTIVITY_OBJECTS
    {
        return Err(RestoreError::TooManyItems);
    }
    if !activity.can_append() {
        return Err(RestoreError::Capacity);
    }
    let mut items = [None; MAX_RESTORE_ITEMS];
    for (index, object_id) in selected_objects.iter().copied().enumerate() {
        if selected_objects[..index].contains(&object_id) {
            return Err(RestoreError::DuplicatePlanObject);
        }
        let checkpoint_object = checkpoint
            .object(object_id)
            .ok_or(RestoreError::ObjectNotInCheckpoint)?;
        let current_revision = current
            .iter()
            .find(|current| current.object_id == object_id)
            .map(|current| current.revision_id);
        items[index] = Some(RestoreItem {
            object_id,
            current_revision,
            target_revision: checkpoint_object.revision_id,
        });
    }
    let mut event = ActivityDraft::new(created_at, actor, context, ActionKind::RestorePlanned)
        .with_result(EventResult::Succeeded)
        .with_provenance(provenance)
        .with_checkpoint_before(checkpoint.id)
        .with_correlation(correlation)
        .with_metadata(
            crate::activity::MetadataKey::ItemCount,
            crate::activity::MetadataValue::Count(selected_objects.len() as u32),
            crate::activity::PrivacyClass::PublicMetadata,
        )
        .map_err(RestoreError::Activity)?;
    event = event
        .with_metadata(
            crate::activity::MetadataKey::RestoreMode,
            crate::activity::MetadataValue::Code(match mode {
                RestoreMode::InPlace => 1,
                RestoreMode::AsCopy => 2,
            }),
            crate::activity::PrivacyClass::PublicMetadata,
        )
        .map_err(RestoreError::Activity)?;
    let plan_event_id = activity.append(event).map_err(RestoreError::Activity)?;
    Ok(RestorePlan {
        id,
        checkpoint_id: checkpoint.id,
        backend_ref: checkpoint.backend_ref,
        checkpoint_scope: checkpoint.scope,
        checkpoint_transaction_id: checkpoint.transaction_id,
        mode,
        items,
        item_count: selected_objects.len() as u8,
        actor,
        provenance,
        context,
        created_at,
        correlation,
        plan_event_id,
    })
}

pub trait RestorePolicy {
    fn authorize(&self, actor: Actor, plan: &RestorePlan) -> Result<(), FailureCode>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoreBackendResult {
    succeeded: u8,
    failed: u8,
    failure: Option<FailureCode>,
    created_objects: [Option<ObjectId>; MAX_RESTORE_ITEMS],
    created_object_count: u8,
}

impl RestoreBackendResult {
    /// Report an in-place restore. Restore-as-copy backends must use
    /// `completed_copies` so each newly allocated ObjectId is returned.
    pub const fn completed(items: u8) -> Self {
        Self {
            succeeded: items,
            failed: 0,
            failure: None,
            created_objects: [None; MAX_RESTORE_ITEMS],
            created_object_count: 0,
        }
    }

    pub const fn failed(items: u8, failure: FailureCode) -> Self {
        Self {
            succeeded: 0,
            failed: items,
            failure: Some(failure),
            created_objects: [None; MAX_RESTORE_ITEMS],
            created_object_count: 0,
        }
    }

    pub const fn partial(succeeded: u8, failed: u8, failure: FailureCode) -> Self {
        Self {
            succeeded,
            failed,
            failure: Some(failure),
            created_objects: [None; MAX_RESTORE_ITEMS],
            created_object_count: 0,
        }
    }

    /// Construct a successful restore-as-copy result. Each generated object
    /// reference is returned to the caller and recorded in Activity.
    pub fn completed_copies(objects: &[ObjectId]) -> Result<Self, FailureCode> {
        if objects.len() > MAX_RESTORE_ITEMS {
            return Err(FailureCode::Capacity);
        }
        let mut created_objects = [None; MAX_RESTORE_ITEMS];
        for (slot, object) in created_objects.iter_mut().zip(objects.iter().copied()) {
            *slot = Some(object);
        }
        Ok(Self {
            succeeded: objects.len() as u8,
            failed: 0,
            failure: None,
            created_objects,
            created_object_count: objects.len() as u8,
        })
    }

    /// Construct a partial restore-as-copy result. `objects` must contain one
    /// generated reference for each successfully created copy.
    pub fn partial_copies(
        objects: &[ObjectId],
        failed: u8,
        failure: FailureCode,
    ) -> Result<Self, FailureCode> {
        if objects.len() > MAX_RESTORE_ITEMS
            || objects.len().saturating_add(failed as usize) > MAX_RESTORE_ITEMS
        {
            return Err(FailureCode::Capacity);
        }
        let mut created_objects = [None; MAX_RESTORE_ITEMS];
        for (slot, object) in created_objects.iter_mut().zip(objects.iter().copied()) {
            *slot = Some(object);
        }
        Ok(Self {
            succeeded: objects.len() as u8,
            failed,
            failure: Some(failure),
            created_objects,
            created_object_count: objects.len() as u8,
        })
    }

    pub const fn succeeded(self) -> u8 {
        self.succeeded
    }

    pub const fn failed_items(self) -> u8 {
        self.failed
    }

    pub const fn failure(self) -> Option<FailureCode> {
        self.failure
    }

    pub fn created_objects(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.created_objects[..self.created_object_count as usize]
            .iter()
            .filter_map(|object| *object)
    }

    pub const fn event_result(self, expected_items: u8) -> EventResult {
        if self.succeeded.saturating_add(self.failed) != expected_items {
            EventResult::Failed(FailureCode::BackendFailure)
        } else if self.failed == 0 {
            EventResult::Succeeded
        } else if self.succeeded == 0 {
            EventResult::Failed(match self.failure {
                Some(code) => code,
                None => FailureCode::BackendFailure,
            })
        } else {
            EventResult::Partial {
                succeeded: self.succeeded,
                failed: self.failed,
            }
        }
    }
}

pub trait RestoreBackend {
    fn kind(&self) -> RestoreBackendKind;

    /// Preserve the current states named by the in-place plan before mutation.
    /// The returned snapshot is recorded as a recovery checkpoint before
    /// `execute` is called. Backends must validate the expected revisions here.
    fn capture_before_restore(
        &mut self,
        plan: &RestorePlan,
    ) -> Result<SnapshotBackendRef, FailureCode>;

    /// Implementations must validate expected revisions and the referenced
    /// checkpoint before mutating. A backend may report partial completion.
    fn execute(&mut self, plan: &RestorePlan) -> RestoreBackendResult;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreBackendKind {
    NagiTarget,
    HostInMemorySandbox,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreBackendAvailability {
    NagiTarget,
    HostInMemorySandbox,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RestoreOutcome {
    event_id: EventId,
    result: EventResult,
    backend: RestoreBackendKind,
    backend_executed: bool,
    created_objects: [Option<ObjectId>; MAX_RESTORE_ITEMS],
    created_object_count: u8,
    recovery_checkpoint: Option<CheckpointId>,
}

impl RestoreOutcome {
    pub const fn event_id(self) -> EventId {
        self.event_id
    }

    pub const fn result(self) -> EventResult {
        self.result
    }

    pub const fn backend(self) -> RestoreBackendKind {
        self.backend
    }

    pub const fn backend_executed(self) -> bool {
        self.backend_executed
    }

    pub fn created_objects(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.created_objects[..self.created_object_count as usize]
            .iter()
            .filter_map(|object| *object)
    }

    pub const fn recovery_checkpoint(&self) -> Option<CheckpointId> {
        self.recovery_checkpoint
    }
}

fn checkpoint_failure_code(error: CheckpointError) -> FailureCode {
    match error {
        CheckpointError::Capacity | CheckpointError::Activity(ActivityError::Capacity) => {
            FailureCode::Capacity
        }
        CheckpointError::PermissionDenied => FailureCode::PermissionDenied,
        CheckpointError::NotFound => FailureCode::MissingCheckpoint,
        CheckpointError::Activity(_) => FailureCode::BackendFailure,
        _ => FailureCode::Validation,
    }
}

fn create_recovery_checkpoint<C: CheckpointWriteStore>(
    plan: &RestorePlan,
    occurred_at: Timestamp,
    backend_ref: SnapshotBackendRef,
    checkpoints: &mut C,
    activity: &mut dyn ActivitySink,
) -> Result<CheckpointId, FailureCode> {
    let mut draft = CheckpointDraft::new(
        occurred_at,
        plan.actor,
        plan.context,
        plan.checkpoint_scope,
        CheckpointOrigin::System,
        CheckpointReason::AutomaticHistory,
        backend_ref,
    )
    .with_provenance(plan.provenance);
    if let Some(transaction) = plan.checkpoint_transaction_id {
        draft = draft.with_transaction(transaction);
    }
    for item in plan.items() {
        let revision = item.current_revision.ok_or(FailureCode::MissingObject)?;
        draft = draft
            .with_object(CheckpointObject::new(item.object_id, revision))
            .map_err(checkpoint_failure_code)?;
    }
    checkpoints
        .create_checkpoint(draft, activity)
        .map(|(checkpoint, _)| checkpoint)
        .map_err(checkpoint_failure_code)
}

/// Execute only a separately reviewed/confirmed plan through a capability
/// policy and an explicit backend. In-place execution first captures and
/// records a recovery checkpoint, then appends the result to Activity.
pub fn execute_restore<C: CheckpointWriteStore>(
    confirmation: ConfirmedRestorePlan,
    occurred_at: Timestamp,
    policy: &dyn RestorePolicy,
    checkpoints: &mut C,
    backend: &mut dyn RestoreBackend,
    activity: &mut dyn ActivitySink,
) -> Result<RestoreOutcome, RestoreError> {
    let plan = confirmation.plan;
    let backend_kind = backend.kind();
    if !activity.can_append() {
        return Err(RestoreError::Capacity);
    }
    let mut recovery_checkpoint = None;
    let (backend_result, backend_executed) = match policy
        .authorize(confirmation.confirmed_by, &plan)
    {
        Err(failure) => (
            RestoreBackendResult::failed(plan.item_count, failure),
            false,
        ),
        Ok(()) if plan.mode == RestoreMode::AsCopy => (backend.execute(&plan), true),
        Ok(()) if !activity.can_append_events(2) || !checkpoints.can_create_checkpoint() => (
            RestoreBackendResult::failed(plan.item_count, FailureCode::Capacity),
            false,
        ),
        Ok(()) => match backend.capture_before_restore(&plan) {
            Err(failure) => (RestoreBackendResult::failed(plan.item_count, failure), true),
            Ok(backend_ref) => match create_recovery_checkpoint(
                &plan,
                occurred_at,
                backend_ref,
                checkpoints,
                activity,
            ) {
                Err(failure) => (RestoreBackendResult::failed(plan.item_count, failure), true),
                Ok(checkpoint) => {
                    recovery_checkpoint = Some(checkpoint);
                    if !activity.can_append() {
                        (
                            RestoreBackendResult::failed(plan.item_count, FailureCode::Capacity),
                            true,
                        )
                    } else {
                        (backend.execute(&plan), true)
                    }
                }
            },
        },
    };
    let refs_valid = match plan.mode {
        RestoreMode::InPlace => backend_result.created_object_count == 0,
        RestoreMode::AsCopy => {
            backend_result.created_object_count == backend_result.succeeded
                && backend_result.created_object_count
                    <= crate::activity::MAX_ACTIVITY_OBJECTS as u8
                && backend_result
                    .created_objects()
                    .enumerate()
                    .all(|(index, object)| {
                        backend_result
                            .created_objects()
                            .take(index)
                            .all(|previous| previous != object)
                    })
                && plan.items().all(|item| {
                    backend_result
                        .created_objects()
                        .all(|object| object != item.object_id)
                })
        }
    };
    let result = if refs_valid {
        backend_result.event_result(plan.item_count)
    } else {
        EventResult::Failed(FailureCode::BackendFailure)
    };
    let action = if result == EventResult::Succeeded {
        ActionKind::RestoreApplied
    } else {
        ActionKind::RestoreFailed
    };
    let failure = backend_result
        .failure
        .unwrap_or(FailureCode::BackendFailure);
    let mut event = ActivityDraft::new(occurred_at, plan.actor, plan.context, action)
        .with_provenance(plan.provenance)
        .with_correlation(plan.correlation)
        .with_parent(CausalParent(plan.plan_event_id))
        .with_result(result)
        .with_metadata(
            crate::activity::MetadataKey::ConfirmedByActor,
            crate::activity::MetadataValue::ActorReference(confirmation.confirmed_by.id()),
            crate::activity::PrivacyClass::PublicMetadata,
        )
        .map_err(RestoreError::Activity)?;
    if let Some(recovery) = recovery_checkpoint {
        event = event.with_checkpoint_before(recovery);
        if result == EventResult::Succeeded {
            event = event.with_checkpoint_after(plan.checkpoint_id);
        }
    } else if plan.mode == RestoreMode::AsCopy && result == EventResult::Succeeded {
        event = event.with_checkpoint_after(plan.checkpoint_id);
    }
    match plan.mode {
        RestoreMode::InPlace => {
            for (index, item) in plan.items().enumerate() {
                if index < crate::activity::MAX_ACTIVITY_OBJECTS {
                    event = event
                        .with_target(item.object_id)
                        .map_err(RestoreError::Activity)?;
                }
            }
        }
        RestoreMode::AsCopy => {
            for item in plan.items().take(crate::activity::MAX_ACTIVITY_OBJECTS) {
                event = event
                    .with_source(item.object_id)
                    .map_err(RestoreError::Activity)?;
            }
            for object in backend_result.created_objects() {
                event = event.with_target(object).map_err(RestoreError::Activity)?;
            }
        }
    }
    if result != EventResult::Succeeded {
        event = event
            .with_metadata(
                crate::activity::MetadataKey::FailureCode,
                crate::activity::MetadataValue::Code(failure_code_number(failure)),
                crate::activity::PrivacyClass::PublicMetadata,
            )
            .map_err(RestoreError::Activity)?;
        event = event
            .with_metadata(
                crate::activity::MetadataKey::RestoreMode,
                crate::activity::MetadataValue::Code(match plan.mode {
                    RestoreMode::InPlace => 1,
                    RestoreMode::AsCopy => 2,
                }),
                crate::activity::PrivacyClass::PublicMetadata,
            )
            .map_err(RestoreError::Activity)?;
    }
    let event_id = activity.append(event).map_err(RestoreError::Activity)?;
    Ok(RestoreOutcome {
        event_id,
        result,
        backend: backend_kind,
        backend_executed,
        created_objects: backend_result.created_objects,
        created_object_count: backend_result.created_object_count,
        recovery_checkpoint,
    })
}

const fn failure_code_number(failure: FailureCode) -> u32 {
    match failure {
        FailureCode::PermissionDenied => 1,
        FailureCode::StaleState => 2,
        FailureCode::MissingObject => 3,
        FailureCode::MissingCheckpoint => 4,
        FailureCode::Unsupported => 5,
        FailureCode::BackendFailure => 6,
        FailureCode::Capacity => 7,
        FailureCode::Validation => 8,
        FailureCode::Other(code) => code as u32,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    Capacity,
    MissingObject,
    RevisionMismatch,
    SnapshotTooLarge,
    MissingSnapshot,
}

pub trait SnapshotBackend {
    fn capture(
        &mut self,
        scope: CheckpointScope,
        objects: &[CheckpointObject],
    ) -> Result<SnapshotBackendRef, SnapshotError>;
}

pub fn render_restore_plan(
    plan: &RestorePlan,
    locale: crate::activity::UserLocale,
    backend: RestoreBackendAvailability,
    output: &mut [u8],
) -> Result<usize, crate::activity::RenderError> {
    let header: &[u8] = match (locale, plan.mode) {
        (crate::activity::UserLocale::EnUs, RestoreMode::InPlace) => b"Restore preview",
        (crate::activity::UserLocale::EnUs, RestoreMode::AsCopy) => b"Restore as copy preview",
        (crate::activity::UserLocale::JaJp, RestoreMode::InPlace) => "復元プレビュー".as_bytes(),
        (crate::activity::UserLocale::JaJp, RestoreMode::AsCopy) => {
            "コピーとして復元するプレビュー".as_bytes()
        }
    };
    let backend_label: &[u8] = match (locale, backend) {
        (crate::activity::UserLocale::EnUs, RestoreBackendAvailability::NagiTarget) => {
            b"Backend: Nagi target adapter available; capability validation still applies."
        }
        (crate::activity::UserLocale::EnUs, RestoreBackendAvailability::HostInMemorySandbox) => {
            b"Backend: host in-memory sandbox; this is not Nagi target restore."
        }
        (crate::activity::UserLocale::EnUs, RestoreBackendAvailability::Unavailable) => {
            b"Preview only: Nagi target restore backend unavailable."
        }
        (crate::activity::UserLocale::JaJp, RestoreBackendAvailability::NagiTarget) => {
            "バックエンド: Nagi実機adapterが利用可能です。権限検証が必要です。".as_bytes()
        }
        (crate::activity::UserLocale::JaJp, RestoreBackendAvailability::HostInMemorySandbox) => {
            "バックエンド: host in-memory sandbox。Nagi実機の復元ではありません。".as_bytes()
        }
        (crate::activity::UserLocale::JaJp, RestoreBackendAvailability::Unavailable) => {
            "プレビューのみ: Nagi実機の復元バックエンドは未対応です。".as_bytes()
        }
    };
    let affected_label: &[u8] = match locale {
        crate::activity::UserLocale::EnUs => b"Affected objects: ",
        crate::activity::UserLocale::JaJp => "対象オブジェクト数: ".as_bytes(),
    };
    let mut offset = 0;
    offset = append_text(output, offset, header)?;
    offset = append_text(output, offset, b"\n")?;
    offset = append_text(output, offset, backend_label)?;
    offset = append_text(output, offset, b"\n")?;
    offset = append_text(output, offset, affected_label)?;
    offset = append_decimal(output, offset, plan.item_count as u64)?;
    offset = append_text(output, offset, b"\n")?;
    for item in plan.items() {
        offset = match (locale, plan.mode) {
            (crate::activity::UserLocale::EnUs, RestoreMode::InPlace) => {
                append_text(output, offset, b"Object #")?
            }
            (crate::activity::UserLocale::JaJp, RestoreMode::InPlace) => {
                append_text(output, offset, "オブジェクト #".as_bytes())?
            }
            (crate::activity::UserLocale::EnUs, RestoreMode::AsCopy) => {
                append_text(output, offset, b"Copy of object #")?
            }
            (crate::activity::UserLocale::JaJp, RestoreMode::AsCopy) => {
                append_text(output, offset, "オブジェクト #".as_bytes())?
            }
        };
        offset = append_decimal(output, offset, item.object_id.0)?;
        if plan.mode == RestoreMode::InPlace {
            offset = match locale {
                crate::activity::UserLocale::EnUs => {
                    append_text(output, offset, b": current revision ")?
                }
                crate::activity::UserLocale::JaJp => {
                    append_text(output, offset, ": 現在のrevision ".as_bytes())?
                }
            };
            offset = match item.current_revision {
                Some(revision) => append_decimal(output, offset, revision.0)?,
                None => match locale {
                    crate::activity::UserLocale::EnUs => append_text(output, offset, b"missing")?,
                    crate::activity::UserLocale::JaJp => {
                        append_text(output, offset, "存在しません".as_bytes())?
                    }
                },
            };
            offset = match locale {
                crate::activity::UserLocale::EnUs => {
                    append_text(output, offset, b" -> checkpoint revision ")?
                }
                crate::activity::UserLocale::JaJp => {
                    append_text(output, offset, " -> checkpoint revision ".as_bytes())?
                }
            };
        } else {
            offset = match locale {
                crate::activity::UserLocale::EnUs => {
                    append_text(output, offset, b" from checkpoint revision ")?
                }
                crate::activity::UserLocale::JaJp => {
                    append_text(output, offset, " をcheckpoint revision ".as_bytes())?
                }
            };
        }
        offset = append_decimal(output, offset, item.target_revision.0)?;
        offset = append_text(output, offset, b"\n")?;
    }
    if plan.mode == RestoreMode::InPlace {
        offset = match locale {
            crate::activity::UserLocale::EnUs => append_text(
                output,
                offset,
                b"A recovery checkpoint is saved before applying this restore.\n",
            )?,
            crate::activity::UserLocale::JaJp => append_text(
                output,
                offset,
                "適用前に現在の状態を復旧用checkpointとして保存します。\n".as_bytes(),
            )?,
        };
    }
    offset = match locale {
        crate::activity::UserLocale::EnUs => append_text(
            output,
            offset,
            b"Confirmation required before execution.\nExternal effects are not reversed.",
        )?,
        crate::activity::UserLocale::JaJp => append_text(
            output,
            offset,
            "実行前に確認が必要です。\n外部への副作用は取り消されません。".as_bytes(),
        )?,
    };
    Ok(offset)
}

fn append_text(
    output: &mut [u8],
    offset: usize,
    text: &[u8],
) -> Result<usize, crate::activity::RenderError> {
    if output.len().saturating_sub(offset) < text.len() {
        return Err(crate::activity::RenderError::BufferTooSmall);
    }
    output[offset..offset + text.len()].copy_from_slice(text);
    Ok(offset + text.len())
}

fn append_decimal(
    output: &mut [u8],
    offset: usize,
    mut value: u64,
) -> Result<usize, crate::activity::RenderError> {
    let mut reverse = [0u8; 20];
    let mut length = 0;
    loop {
        reverse[length] = b'0' + (value % 10) as u8;
        length += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    if output.len().saturating_sub(offset) < length {
        return Err(crate::activity::RenderError::BufferTooSmall);
    }
    for index in 0..length {
        output[offset + index] = reverse[length - index - 1];
    }
    Ok(offset + length)
}

pub fn render_checkpoint_marker(
    checkpoint: &CheckpointRecord,
    locale: crate::activity::UserLocale,
    output: &mut [u8],
) -> Result<usize, crate::activity::RenderError> {
    let text: &[u8] = match locale {
        crate::activity::UserLocale::EnUs => {
            if checkpoint.pinned {
                b"Pinned checkpoint"
            } else {
                b"Checkpoint"
            }
        }
        crate::activity::UserLocale::JaJp => {
            if checkpoint.pinned {
                "固定したチェックポイント".as_bytes()
            } else {
                "チェックポイント".as_bytes()
            }
        }
    };
    if output.len() < text.len() {
        return Err(crate::activity::RenderError::BufferTooSmall);
    }
    output[..text.len()].copy_from_slice(text);
    Ok(text.len())
}

#[cfg(any(test, feature = "sandbox"))]
pub mod sandbox {
    use super::*;
    use crate::MAX_SNAPSHOT_BYTES;

    pub const MAX_SANDBOX_OBJECTS: usize = 16;
    pub const MAX_SANDBOX_SNAPSHOTS: usize = 16;

    fn partial_result(
        mode: RestoreMode,
        succeeded: u8,
        failed: u8,
        failure: FailureCode,
        copies: &[Option<ObjectId>; MAX_RESTORE_ITEMS],
    ) -> RestoreBackendResult {
        if mode == RestoreMode::AsCopy {
            let copy_ids: [ObjectId; MAX_RESTORE_ITEMS] =
                core::array::from_fn(|index| copies[index].unwrap_or(ObjectId(0)));
            RestoreBackendResult::partial_copies(&copy_ids[..succeeded as usize], failed, failure)
                .unwrap_or_else(|capacity| {
                    RestoreBackendResult::failed(succeeded + failed, capacity)
                })
        } else {
            RestoreBackendResult::partial(succeeded, failed, failure)
        }
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct SandboxObject {
        object_id: ObjectId,
        revision_id: RevisionId,
        bytes: [u8; MAX_SNAPSHOT_BYTES],
        length: usize,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct SandboxSnapshot {
        reference: SnapshotBackendRef,
        scope: CheckpointScope,
        objects: [Option<SandboxObject>; MAX_CHECKPOINT_OBJECTS],
        length: u8,
    }

    /// Explicit host/test backend. It is not a Nagi target snapshot backend.
    pub struct InMemoryRestoreSandbox {
        objects: [Option<SandboxObject>; MAX_SANDBOX_OBJECTS],
        snapshots: [Option<SandboxSnapshot>; MAX_SANDBOX_SNAPSHOTS],
        next_snapshot: u64,
        next_copy_object: u64,
        fail_after: Option<u8>,
    }

    impl InMemoryRestoreSandbox {
        pub const fn new() -> Self {
            Self {
                objects: [None; MAX_SANDBOX_OBJECTS],
                snapshots: [None; MAX_SANDBOX_SNAPSHOTS],
                next_snapshot: 1,
                next_copy_object: 1 << 63,
                fail_after: None,
            }
        }

        pub fn register_object(
            &mut self,
            object_id: ObjectId,
            revision_id: RevisionId,
            bytes: &[u8],
        ) -> Result<(), SnapshotError> {
            let object = make_sandbox_object(object_id, revision_id, bytes)?;
            if let Some(slot) = self
                .objects
                .iter_mut()
                .find(|slot| slot.is_some_and(|item| item.object_id == object_id))
            {
                *slot = Some(object);
                return Ok(());
            }
            let slot = self
                .objects
                .iter_mut()
                .find(|slot| slot.is_none())
                .ok_or(SnapshotError::Capacity)?;
            *slot = Some(object);
            Ok(())
        }

        pub fn update_object(
            &mut self,
            object_id: ObjectId,
            revision_id: RevisionId,
            bytes: &[u8],
        ) -> Result<(), SnapshotError> {
            self.register_object(object_id, revision_id, bytes)
        }

        pub fn current_revision(&self, object_id: ObjectId) -> Option<RevisionId> {
            self.objects
                .iter()
                .filter_map(|object| *object)
                .find(|object| object.object_id == object_id)
                .map(|object| object.revision_id)
        }

        pub fn object_bytes(&self, object_id: ObjectId) -> Option<&[u8]> {
            self.objects
                .iter()
                .filter_map(|object| object.as_ref())
                .find(|object| object.object_id == object_id)
                .map(|object| &object.bytes[..object.length])
        }

        pub fn object_count(&self) -> usize {
            self.objects
                .iter()
                .filter(|object| object.is_some())
                .count()
        }

        pub fn fail_after(&mut self, succeeded_items: Option<u8>) {
            self.fail_after = succeeded_items;
        }

        #[cfg(test)]
        pub(super) fn set_next_copy_object_for_test(&mut self, next: u64) {
            self.next_copy_object = next;
        }

        fn allocate_copy_object_id(&mut self) -> Result<ObjectId, SnapshotError> {
            // At most MAX_SANDBOX_OBJECTS identifiers can be occupied. Bound
            // probing and reject counter overflow rather than saturating into
            // an infinite loop at ObjectId(u64::MAX).
            for _ in 0..=MAX_SANDBOX_OBJECTS {
                let candidate = self.next_copy_object;
                self.next_copy_object = self
                    .next_copy_object
                    .checked_add(1)
                    .ok_or(SnapshotError::Capacity)?;
                let object = ObjectId(candidate);
                if self.current_revision(object).is_none() {
                    return Ok(object);
                }
            }
            Err(SnapshotError::Capacity)
        }

        fn snapshot(&self, reference: SnapshotBackendRef) -> Option<&SandboxSnapshot> {
            self.snapshots
                .iter()
                .filter_map(|snapshot| snapshot.as_ref())
                .find(|snapshot| snapshot.reference == reference)
        }
    }

    impl Default for InMemoryRestoreSandbox {
        fn default() -> Self {
            Self::new()
        }
    }

    impl SnapshotBackend for InMemoryRestoreSandbox {
        fn capture(
            &mut self,
            scope: CheckpointScope,
            objects: &[CheckpointObject],
        ) -> Result<SnapshotBackendRef, SnapshotError> {
            if objects.is_empty() || objects.len() > MAX_CHECKPOINT_OBJECTS {
                return Err(SnapshotError::Capacity);
            }
            let destination = self
                .snapshots
                .iter()
                .position(Option::is_none)
                .ok_or(SnapshotError::Capacity)?;
            let mut snapshot_objects = [None; MAX_CHECKPOINT_OBJECTS];
            for (index, reference) in objects.iter().copied().enumerate() {
                let object = self
                    .objects
                    .iter()
                    .filter_map(|object| *object)
                    .find(|object| object.object_id == reference.object_id)
                    .ok_or(SnapshotError::MissingObject)?;
                if object.revision_id != reference.revision_id {
                    return Err(SnapshotError::RevisionMismatch);
                }
                snapshot_objects[index] = Some(object);
            }
            let reference = SnapshotBackendRef(self.next_snapshot);
            self.next_snapshot = self.next_snapshot.saturating_add(1);
            self.snapshots[destination] = Some(SandboxSnapshot {
                reference,
                scope,
                objects: snapshot_objects,
                length: objects.len() as u8,
            });
            Ok(reference)
        }
    }

    impl RestoreBackend for InMemoryRestoreSandbox {
        fn kind(&self) -> RestoreBackendKind {
            RestoreBackendKind::HostInMemorySandbox
        }

        fn capture_before_restore(
            &mut self,
            plan: &RestorePlan,
        ) -> Result<SnapshotBackendRef, FailureCode> {
            let mut references = [None; MAX_RESTORE_ITEMS];
            for (index, item) in plan.items().enumerate() {
                let revision = item.current_revision.ok_or(FailureCode::MissingObject)?;
                references[index] = Some(CheckpointObject::new(item.object_id, revision));
            }
            let objects: [CheckpointObject; MAX_RESTORE_ITEMS] = core::array::from_fn(|index| {
                references[index].unwrap_or(CheckpointObject::new(ObjectId(0), RevisionId(0)))
            });
            SnapshotBackend::capture(
                self,
                plan.checkpoint_scope,
                &objects[..plan.item_count as usize],
            )
            .map_err(|error| match error {
                SnapshotError::Capacity => FailureCode::Capacity,
                SnapshotError::MissingObject => FailureCode::MissingObject,
                SnapshotError::RevisionMismatch => FailureCode::StaleState,
                SnapshotError::SnapshotTooLarge | SnapshotError::MissingSnapshot => {
                    FailureCode::BackendFailure
                }
            })
        }

        fn execute(&mut self, plan: &RestorePlan) -> RestoreBackendResult {
            let Some(snapshot) = self.snapshot(plan.backend_ref).copied() else {
                return RestoreBackendResult::failed(
                    plan.item_count,
                    FailureCode::MissingCheckpoint,
                );
            };
            if snapshot.scope == CheckpointScope::System {
                return RestoreBackendResult::failed(plan.item_count, FailureCode::Unsupported);
            }
            let available_snapshots = &snapshot.objects[..snapshot.length as usize];
            let mut new_objects_needed = 0usize;
            for item in plan.items() {
                let Some(target) = available_snapshots
                    .iter()
                    .filter_map(|object| *object)
                    .find(|object| object.object_id == item.object_id)
                else {
                    return RestoreBackendResult::failed(
                        plan.item_count,
                        FailureCode::MissingObject,
                    );
                };
                if target.revision_id != item.target_revision {
                    return RestoreBackendResult::failed(plan.item_count, FailureCode::Validation);
                }
                let current = self
                    .objects
                    .iter()
                    .filter_map(|object| *object)
                    .find(|object| object.object_id == item.object_id);
                match plan.mode {
                    RestoreMode::InPlace => {
                        if current.map(|object| object.revision_id) != item.current_revision {
                            return RestoreBackendResult::failed(
                                plan.item_count,
                                FailureCode::StaleState,
                            );
                        }
                        if current.is_none() {
                            new_objects_needed += 1;
                        }
                    }
                    RestoreMode::AsCopy => new_objects_needed += 1,
                }
            }
            if self
                .objects
                .iter()
                .filter(|object| object.is_none())
                .count()
                < new_objects_needed
            {
                return RestoreBackendResult::failed(plan.item_count, FailureCode::Capacity);
            }
            let mut succeeded = 0u8;
            let mut created_copies = [None; MAX_RESTORE_ITEMS];
            for item in plan.items() {
                if self.fail_after.is_some_and(|limit| succeeded >= limit) {
                    return partial_result(
                        plan.mode,
                        succeeded,
                        plan.item_count.saturating_sub(succeeded),
                        FailureCode::BackendFailure,
                        &created_copies,
                    );
                }
                let target = available_snapshots
                    .iter()
                    .filter_map(|object| *object)
                    .find(|object| object.object_id == item.object_id);
                let Some(target) = target else {
                    return partial_result(
                        plan.mode,
                        succeeded,
                        plan.item_count.saturating_sub(succeeded),
                        FailureCode::MissingObject,
                        &created_copies,
                    );
                };
                match plan.mode {
                    RestoreMode::InPlace => {
                        let current = self
                            .objects
                            .iter()
                            .filter_map(|object| *object)
                            .find(|object| object.object_id == item.object_id);
                        if current.map(|object| object.revision_id) != item.current_revision {
                            return partial_result(
                                plan.mode,
                                succeeded,
                                plan.item_count.saturating_sub(succeeded),
                                FailureCode::StaleState,
                                &created_copies,
                            );
                        }
                        if self
                            .register_object(
                                item.object_id,
                                target.revision_id,
                                &target.bytes[..target.length],
                            )
                            .is_err()
                        {
                            return partial_result(
                                plan.mode,
                                succeeded,
                                plan.item_count.saturating_sub(succeeded),
                                FailureCode::Capacity,
                                &created_copies,
                            );
                        }
                    }
                    RestoreMode::AsCopy => {
                        let copy_id = match self.allocate_copy_object_id() {
                            Ok(copy_id) => copy_id,
                            Err(_) => {
                                return partial_result(
                                    plan.mode,
                                    succeeded,
                                    plan.item_count.saturating_sub(succeeded),
                                    FailureCode::Capacity,
                                    &created_copies,
                                )
                            }
                        };
                        if self
                            .register_object(
                                copy_id,
                                target.revision_id,
                                &target.bytes[..target.length],
                            )
                            .is_err()
                        {
                            return partial_result(
                                plan.mode,
                                succeeded,
                                plan.item_count.saturating_sub(succeeded),
                                FailureCode::Capacity,
                                &created_copies,
                            );
                        }
                        created_copies[succeeded as usize] = Some(copy_id);
                    }
                }
                succeeded += 1;
            }
            match plan.mode {
                RestoreMode::InPlace => RestoreBackendResult::completed(succeeded),
                RestoreMode::AsCopy => {
                    let copy_ids: [ObjectId; MAX_RESTORE_ITEMS] =
                        core::array::from_fn(|index| created_copies[index].unwrap_or(ObjectId(0)));
                    RestoreBackendResult::completed_copies(&copy_ids[..succeeded as usize])
                        .unwrap_or_else(|failure| RestoreBackendResult::failed(succeeded, failure))
                }
            }
        }
    }

    impl DiffProvider for InMemoryRestoreSandbox {
        fn compare(
            &self,
            backend_ref: SnapshotBackendRef,
            object: ObjectId,
            current_revision: Option<RevisionId>,
            target_revision: RevisionId,
        ) -> Result<DiffSummary, DiffError> {
            let snapshot = self
                .snapshot(backend_ref)
                .ok_or(DiffError::MissingRevision)?;
            let target = snapshot.objects[..snapshot.length as usize]
                .iter()
                .filter_map(|object| *object)
                .find(|entry| entry.object_id == object)
                .filter(|entry| entry.revision_id == target_revision)
                .ok_or(DiffError::MissingRevision)?;
            let Some(current) = self
                .objects
                .iter()
                .filter_map(|entry| *entry)
                .find(|entry| entry.object_id == object)
            else {
                if current_revision.is_some() {
                    return Err(DiffError::BackendFailure);
                }
                return Ok(DiffSummary::MissingCurrent {
                    target_bytes: target.length as u32,
                });
            };
            if current_revision != Some(current.revision_id) {
                return Err(DiffError::BackendFailure);
            }
            if current.length == target.length
                && current.bytes[..current.length] == target.bytes[..target.length]
            {
                Ok(DiffSummary::Unchanged)
            } else {
                Ok(DiffSummary::Changed {
                    current_bytes: current.length as u32,
                    target_bytes: target.length as u32,
                })
            }
        }
    }

    fn make_sandbox_object(
        object_id: ObjectId,
        revision_id: RevisionId,
        bytes: &[u8],
    ) -> Result<SandboxObject, SnapshotError> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(SnapshotError::SnapshotTooLarge);
        }
        let mut stored = [0; MAX_SNAPSHOT_BYTES];
        stored[..bytes.len()].copy_from_slice(bytes);
        Ok(SandboxObject {
            object_id,
            revision_id,
            bytes: stored,
            length: bytes.len(),
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::activity::{
            ActivityLedger, ActivityQuery, ActivityTimeRange, ActorId, ActorKind, Provenance,
            UserLocale,
        };
        use crate::{AppId, AppSessionId, NodeId};

        const CONTEXT: ActivityContext = ActivityContext {
            app_id: AppId(1),
            app_session_id: AppSessionId(2),
            node_id: NodeId(3),
            surface_id: None,
            workspace_id: Some(WorkspaceId(4)),
        };
        const USER: Actor = Actor::new(ActorId(8), ActorKind::User);
        const AGENT: Actor = Actor::new(ActorId(9), ActorKind::Agent);

        fn time(seconds: i64) -> Timestamp {
            Timestamp::new(seconds, 0).unwrap()
        }

        struct AllowRestore;

        impl RestorePolicy for AllowRestore {
            fn authorize(&self, _actor: Actor, _plan: &RestorePlan) -> Result<(), FailureCode> {
                Ok(())
            }
        }

        struct DenyRestore;

        impl RestorePolicy for DenyRestore {
            fn authorize(&self, _actor: Actor, _plan: &RestorePlan) -> Result<(), FailureCode> {
                Err(FailureCode::PermissionDenied)
            }
        }

        fn add_checkpoint(
            sandbox: &mut InMemoryRestoreSandbox,
            store: &mut CheckpointStore,
            activity: &mut ActivityLedger,
            object_ids: &[ObjectId],
            pinned: bool,
            seconds: i64,
        ) -> CheckpointId {
            let mut objects = [None; MAX_CHECKPOINT_OBJECTS];
            for (index, object_id) in object_ids.iter().copied().enumerate() {
                objects[index] = Some(CheckpointObject::new(object_id, RevisionId(1)));
            }
            let refs: [CheckpointObject; MAX_CHECKPOINT_OBJECTS] = core::array::from_fn(|index| {
                objects[index].unwrap_or(CheckpointObject::new(ObjectId(0), RevisionId(0)))
            });
            let snapshot = sandbox
                .capture(CheckpointScope::Workspace, &refs[..object_ids.len()])
                .unwrap();
            let mut draft = CheckpointDraft::new(
                time(seconds),
                USER,
                CONTEXT,
                CheckpointScope::Workspace,
                CheckpointOrigin::User,
                CheckpointReason::UserRequested,
                snapshot,
            );
            for object_id in object_ids.iter().copied() {
                draft = draft
                    .with_object(CheckpointObject::new(object_id, RevisionId(1)))
                    .unwrap();
            }
            if pinned {
                draft = draft.pinned();
            }
            store.create(draft, activity).unwrap().0
        }

        fn build_plan(
            checkpoint: &CheckpointRecord,
            activity: &mut ActivityLedger,
            mode: RestoreMode,
            ids: &[ObjectId],
            current: &[CurrentObjectRevision],
        ) -> RestorePlan {
            prepare_restore_plan(
                RestorePlanId(5),
                checkpoint,
                USER,
                Provenance::Direct {
                    originating_intent: None,
                },
                CONTEXT,
                time(12),
                mode,
                ids,
                current,
                CorrelationId(77),
                activity,
            )
            .unwrap()
        }

        #[test]
        fn checkpoint_record_references_backend_state_and_queries_by_workspace_time_and_object() {
            let mut sandbox = InMemoryRestoreSandbox::new();
            sandbox
                .register_object(ObjectId(1), RevisionId(1), b"snapshot")
                .unwrap();
            let mut store = CheckpointStore::new();
            let mut activity = ActivityLedger::new();
            let checkpoint_id = add_checkpoint(
                &mut sandbox,
                &mut store,
                &mut activity,
                &[ObjectId(1)],
                false,
                10,
            );
            let record = store.get(checkpoint_id).unwrap();
            assert_eq!(record.backend_ref().0, 1);
            assert_eq!(record.objects().count(), 1);
            assert_eq!(record.validity(), CheckpointValidity::Available);
            let range = ActivityTimeRange::new(time(9), time(11)).unwrap();
            assert_eq!(
                store
                    .query(
                        CheckpointQuery::default()
                            .time_range(range)
                            .workspace(WorkspaceId(4))
                            .object(ObjectId(1)),
                    )
                    .count(),
                1
            );
            assert_eq!(activity.query(ActivityQuery::default()).count(), 1);
        }

        #[test]
        fn checkpoint_draft_materializes_existing_persisted_ids_for_adapters() {
            let draft = CheckpointDraft::new(
                time(6),
                USER,
                CONTEXT,
                CheckpointScope::Workspace,
                CheckpointOrigin::User,
                CheckpointReason::UserRequested,
                SnapshotBackendRef(5),
            )
            .with_object(CheckpointObject::new(ObjectId(7), RevisionId(8)))
            .unwrap()
            .with_transaction(TransactionId(9));
            let checkpoint_id = CheckpointId::new(44).unwrap();
            let record = draft
                .with_pinned_state(true)
                .into_record(checkpoint_id)
                .unwrap();
            assert_eq!(record.id(), checkpoint_id);
            assert_eq!(record.backend_ref(), SnapshotBackendRef(5));
            assert_eq!(record.transaction_id(), Some(TransactionId(9)));
            assert!(record.is_pinned());
            assert_eq!(
                record.object(ObjectId(7)).unwrap().revision_id(),
                RevisionId(8)
            );
            assert!(!draft
                .with_pinned_state(false)
                .into_record(checkpoint_id)
                .unwrap()
                .is_pinned());

            let event = draft
                .creation_activity(checkpoint_id)
                .unwrap()
                .into_event(crate::activity::EventId::new(12).unwrap())
                .unwrap();
            assert_eq!(event.checkpoint_after(), Some(checkpoint_id));
            assert_eq!(event.transaction_id(), Some(TransactionId(9)));
            assert_eq!(
                draft.into_record(CheckpointId(0)),
                Err(CheckpointError::InvalidId)
            );
        }

        #[test]
        fn opening_a_checkpoint_version_requires_visibility_and_object_authority() {
            struct Visible;
            impl CheckpointAccessPolicy for Visible {
                fn can_read(&self, _viewer: Actor, _checkpoint: &CheckpointRecord) -> bool {
                    true
                }
            }
            struct AllowOpen;
            impl CheckpointVersionOpenPolicy for AllowOpen {
                fn authorize_open(
                    &self,
                    _actor: Actor,
                    _checkpoint: &CheckpointRecord,
                    _object: ObjectId,
                ) -> Result<(), FailureCode> {
                    Ok(())
                }
            }
            struct DenyOpen;
            impl CheckpointVersionOpenPolicy for DenyOpen {
                fn authorize_open(
                    &self,
                    _actor: Actor,
                    _checkpoint: &CheckpointRecord,
                    _object: ObjectId,
                ) -> Result<(), FailureCode> {
                    Err(FailureCode::PermissionDenied)
                }
            }
            #[derive(Default)]
            struct RecordingOpener(Option<CheckpointVersionRequest>);
            impl CheckpointVersionOpener for RecordingOpener {
                fn open_version(
                    &mut self,
                    request: CheckpointVersionRequest,
                ) -> Result<(), FailureCode> {
                    self.0 = Some(request);
                    Ok(())
                }
            }

            let mut sandbox = InMemoryRestoreSandbox::new();
            sandbox
                .register_object(ObjectId(12), RevisionId(1), b"version")
                .unwrap();
            let mut store = CheckpointStore::new();
            let mut activity = ActivityLedger::new();
            let checkpoint = add_checkpoint(
                &mut sandbox,
                &mut store,
                &mut activity,
                &[ObjectId(12)],
                false,
                1,
            );
            let mut opener = RecordingOpener::default();
            assert_eq!(
                open_checkpoint_version(
                    &store,
                    checkpoint,
                    ObjectId(12),
                    USER,
                    &Visible,
                    &DenyOpen,
                    &mut opener,
                ),
                Err(CheckpointVersionOpenError::Denied(
                    FailureCode::PermissionDenied
                ))
            );
            assert_eq!(opener.0, None);
            open_checkpoint_version(
                &store,
                checkpoint,
                ObjectId(12),
                USER,
                &Visible,
                &AllowOpen,
                &mut opener,
            )
            .unwrap();
            let request = opener.0.unwrap();
            assert_eq!(request.checkpoint_id(), checkpoint);
            assert_eq!(request.object_id(), ObjectId(12));
            assert_eq!(request.revision_id(), RevisionId(1));
            assert_eq!(request.backend_ref().0, 1);
            assert_eq!(
                open_checkpoint_version(
                    &store,
                    checkpoint,
                    ObjectId(13),
                    USER,
                    &Visible,
                    &AllowOpen,
                    &mut opener,
                ),
                Err(CheckpointVersionOpenError::ObjectNotInCheckpoint)
            );
        }

        #[test]
        fn revision_index_preserves_parent_identity_and_queries_semantic_versions() {
            let mut revisions = RevisionStore::new();
            let first = revisions
                .append(RevisionDraft::new(
                    ObjectId(3),
                    None,
                    time(1),
                    USER,
                    CONTEXT,
                    SnapshotBackendRef(1),
                ))
                .unwrap();
            let second = revisions
                .append(
                    RevisionDraft::new(
                        ObjectId(3),
                        Some(first),
                        time(2),
                        USER,
                        CONTEXT,
                        SnapshotBackendRef(2),
                    )
                    .with_transaction(TransactionId(4)),
                )
                .unwrap();
            assert_eq!(revisions.get(second).unwrap().parent(), Some(first));
            let range = ActivityTimeRange::new(time(2), time(3)).unwrap();
            assert_eq!(
                revisions
                    .query(
                        RevisionQuery::default()
                            .object(ObjectId(3))
                            .workspace(WorkspaceId(4))
                            .transaction(TransactionId(4))
                            .time_range(range),
                    )
                    .count(),
                1
            );
            assert_eq!(
                revisions.append(RevisionDraft::new(
                    ObjectId(8),
                    Some(first),
                    time(3),
                    USER,
                    CONTEXT,
                    SnapshotBackendRef(3),
                )),
                Err(RevisionError::ParentObjectMismatch)
            );
        }

        #[test]
        fn revision_draft_materializes_existing_ids_and_write_contract_is_usable() {
            let draft = RevisionDraft::new(
                ObjectId(3),
                Some(RevisionId(2)),
                time(4),
                USER,
                CONTEXT,
                SnapshotBackendRef(6),
            )
            .with_transaction(TransactionId(7));
            let revision_id = RevisionId::new(31).unwrap();
            let record = draft.into_record(revision_id).unwrap();
            assert_eq!(record.id(), revision_id);
            assert_eq!(record.object_id(), ObjectId(3));
            assert_eq!(record.parent(), Some(RevisionId(2)));
            assert_eq!(record.backend_ref(), SnapshotBackendRef(6));
            assert_eq!(record.transaction_id(), Some(TransactionId(7)));
            assert!(matches!(
                draft.into_record(RevisionId(0)),
                Err(RevisionError::InvalidId)
            ));

            let mut store = RevisionStore::new();
            assert!(RevisionWriteStore::can_append_revision(&store));
            let stored = RevisionWriteStore::append_revision(
                &mut store,
                RevisionDraft::new(
                    ObjectId(10),
                    None,
                    time(5),
                    USER,
                    CONTEXT,
                    SnapshotBackendRef(8),
                ),
            )
            .unwrap();
            assert_eq!(store.get(stored).unwrap().object_id(), ObjectId(10));
        }

        #[test]
        fn sandbox_diff_provider_is_permission_gated_and_returns_summary_only() {
            struct AllowDiff;
            impl DiffAccessPolicy for AllowDiff {
                fn can_compare(&self, _viewer: Actor, _object: ObjectId) -> bool {
                    true
                }
            }
            struct DenyDiff;
            impl DiffAccessPolicy for DenyDiff {
                fn can_compare(&self, _viewer: Actor, _object: ObjectId) -> bool {
                    false
                }
            }
            let mut sandbox = InMemoryRestoreSandbox::new();
            sandbox
                .register_object(ObjectId(4), RevisionId(1), b"old")
                .unwrap();
            let snapshot = sandbox
                .capture(
                    CheckpointScope::Document,
                    &[CheckpointObject::new(ObjectId(4), RevisionId(1))],
                )
                .unwrap();
            sandbox
                .update_object(ObjectId(4), RevisionId(2), b"new")
                .unwrap();
            assert_eq!(
                compare_with_policy(
                    &sandbox,
                    &AllowDiff,
                    USER,
                    snapshot,
                    ObjectId(4),
                    Some(RevisionId(2)),
                    RevisionId(1),
                ),
                Ok(DiffSummary::Changed {
                    current_bytes: 3,
                    target_bytes: 3,
                })
            );
            assert_eq!(
                compare_with_policy(
                    &sandbox,
                    &DenyDiff,
                    USER,
                    snapshot,
                    ObjectId(4),
                    Some(RevisionId(2)),
                    RevisionId(1),
                ),
                Err(DiffError::PermissionDenied)
            );
        }

        #[test]
        fn pinned_checkpoints_survive_bounded_retention() {
            let mut store = CheckpointStore::new();
            let mut activity = ActivityLedger::new();
            let make_draft = |id: u64, pinned: bool, seconds: i64| {
                let draft = CheckpointDraft::new(
                    time(seconds),
                    USER,
                    CONTEXT,
                    CheckpointScope::Document,
                    CheckpointOrigin::User,
                    CheckpointReason::UserRequested,
                    SnapshotBackendRef(id),
                )
                .with_object(CheckpointObject::new(ObjectId(1), RevisionId(id)))
                .unwrap();
                if pinned {
                    draft.pinned()
                } else {
                    draft
                }
            };
            let pinned = store
                .create(make_draft(1, true, 0), &mut activity)
                .unwrap()
                .0;
            for index in 1..=MAX_CHECKPOINTS {
                store
                    .create(
                        make_draft(index as u64 + 1, false, index as i64),
                        &mut activity,
                    )
                    .unwrap();
            }
            assert_eq!(store.len(), MAX_CHECKPOINTS);
            assert!(store.get(pinned).unwrap().is_pinned());
            assert_eq!(
                store
                    .query(CheckpointQuery::default().pinned_only())
                    .count(),
                1
            );
        }

        #[test]
        fn pin_and_unpin_are_authorized_and_recorded_as_activity() {
            struct AllowPin;
            impl CheckpointMutationPolicy for AllowPin {
                fn can_change_pin(
                    &self,
                    _actor: Actor,
                    _checkpoint: &CheckpointRecord,
                    _pinned: bool,
                ) -> bool {
                    true
                }
            }
            struct DenyPin;
            impl CheckpointMutationPolicy for DenyPin {
                fn can_change_pin(
                    &self,
                    _actor: Actor,
                    _checkpoint: &CheckpointRecord,
                    _pinned: bool,
                ) -> bool {
                    false
                }
            }

            let mut sandbox = InMemoryRestoreSandbox::new();
            sandbox
                .register_object(ObjectId(6), RevisionId(1), b"state")
                .unwrap();
            let mut store = CheckpointStore::new();
            let mut activity = ActivityLedger::new();
            let id = add_checkpoint(
                &mut sandbox,
                &mut store,
                &mut activity,
                &[ObjectId(6)],
                false,
                1,
            );
            let pinned = store
                .set_pinned(
                    id,
                    true,
                    time(2),
                    USER,
                    Provenance::Direct {
                        originating_intent: None,
                    },
                    &AllowPin,
                    &mut activity,
                )
                .unwrap();
            let CheckpointPinChange::Changed(pin_event) = pinned else {
                panic!("pin must change an unpinned checkpoint");
            };
            assert!(store.get(id).unwrap().is_pinned());
            assert_eq!(
                activity.get(pin_event).unwrap().action(),
                ActionKind::CheckpointPinned
            );
            assert_eq!(
                store.set_pinned(
                    id,
                    true,
                    time(3),
                    USER,
                    Provenance::Direct {
                        originating_intent: None,
                    },
                    &AllowPin,
                    &mut activity,
                ),
                Ok(CheckpointPinChange::Unchanged)
            );
            let unpinned = store
                .set_pinned(
                    id,
                    false,
                    time(4),
                    USER,
                    Provenance::Direct {
                        originating_intent: None,
                    },
                    &AllowPin,
                    &mut activity,
                )
                .unwrap();
            let CheckpointPinChange::Changed(unpin_event) = unpinned else {
                panic!("unpin must change the pinned checkpoint");
            };
            assert!(!store.get(id).unwrap().is_pinned());
            assert_eq!(
                activity.get(unpin_event).unwrap().action(),
                ActionKind::CheckpointUnpinned
            );
            assert_eq!(
                store.set_pinned(
                    id,
                    true,
                    time(5),
                    USER,
                    Provenance::Direct {
                        originating_intent: None,
                    },
                    &DenyPin,
                    &mut activity,
                ),
                Err(CheckpointError::PermissionDenied)
            );
            assert!(!store.get(id).unwrap().is_pinned());
            assert_eq!(
                activity
                    .query(ActivityQuery::default().checkpoint(id))
                    .filter(|event| matches!(
                        event.action(),
                        ActionKind::CheckpointPinned | ActionKind::CheckpointUnpinned
                    ))
                    .count(),
                2
            );
            while activity.can_append() {
                activity
                    .append(ActivityDraft::new(
                        time(6),
                        USER,
                        CONTEXT,
                        ActionKind::ObjectAccessed,
                    ))
                    .unwrap();
            }
            assert_eq!(
                store.set_pinned(
                    id,
                    true,
                    time(7),
                    USER,
                    Provenance::Direct {
                        originating_intent: None,
                    },
                    &AllowPin,
                    &mut activity,
                ),
                Err(CheckpointError::Capacity)
            );
            assert!(!store.get(id).unwrap().is_pinned());
        }

        #[test]
        fn restore_plan_is_preview_only_supports_partial_workspace_and_copy_mode() {
            let mut sandbox = InMemoryRestoreSandbox::new();
            for id in [ObjectId(1), ObjectId(2)] {
                sandbox.register_object(id, RevisionId(1), b"old").unwrap();
            }
            let mut checkpoints = CheckpointStore::new();
            let mut activity = ActivityLedger::new();
            let checkpoint_id = add_checkpoint(
                &mut sandbox,
                &mut checkpoints,
                &mut activity,
                &[ObjectId(1), ObjectId(2)],
                false,
                1,
            );
            sandbox
                .update_object(ObjectId(1), RevisionId(2), b"new")
                .unwrap();
            sandbox
                .update_object(ObjectId(2), RevisionId(2), b"new")
                .unwrap();
            let current = [
                CurrentObjectRevision::new(ObjectId(1), RevisionId(2)),
                CurrentObjectRevision::new(ObjectId(2), RevisionId(2)),
            ];
            let plan = build_plan(
                checkpoints.get(checkpoint_id).unwrap(),
                &mut activity,
                RestoreMode::InPlace,
                &[ObjectId(1)],
                &current,
            );
            assert_eq!(plan.item_count(), 1);
            assert_eq!(sandbox.object_bytes(ObjectId(1)), Some(b"new".as_slice()));
            let mut preview = [0; 512];
            let length = render_restore_plan(
                &plan,
                UserLocale::EnUs,
                RestoreBackendAvailability::Unavailable,
                &mut preview,
            )
            .unwrap();
            assert!(core::str::from_utf8(&preview[..length])
                .unwrap()
                .contains("Preview only"));
            assert_eq!(
                plan.confirm(USER, false),
                Err(RestoreError::ExternalEffectsNotAcknowledged)
            );
            let copy_plan = build_plan(
                checkpoints.get(checkpoint_id).unwrap(),
                &mut activity,
                RestoreMode::AsCopy,
                &[ObjectId(1)],
                &current,
            );
            assert_eq!(copy_plan.mode(), RestoreMode::AsCopy);
            assert_eq!(
                copy_plan.confirm(AGENT, true),
                Err(RestoreError::HumanConfirmationRequired)
            );
        }

        #[test]
        fn mock_restore_success_updates_state_and_records_restore_activity() {
            let mut sandbox = InMemoryRestoreSandbox::new();
            sandbox
                .register_object(ObjectId(9), RevisionId(1), b"before")
                .unwrap();
            let mut checkpoints = CheckpointStore::new();
            let mut activity = ActivityLedger::new();
            let id = add_checkpoint(
                &mut sandbox,
                &mut checkpoints,
                &mut activity,
                &[ObjectId(9)],
                false,
                1,
            );
            sandbox
                .update_object(ObjectId(9), RevisionId(2), b"after")
                .unwrap();
            let current = [CurrentObjectRevision::new(ObjectId(9), RevisionId(2))];
            let plan = build_plan(
                checkpoints.get(id).unwrap(),
                &mut activity,
                RestoreMode::InPlace,
                &[ObjectId(9)],
                &current,
            );
            let plan_event_id = plan.plan_event_id();
            let confirmation = plan.confirm(USER, true).unwrap();
            let outcome = execute_restore(
                confirmation,
                time(13),
                &AllowRestore,
                &mut checkpoints,
                &mut sandbox,
                &mut activity,
            )
            .unwrap();
            assert_eq!(outcome.result(), EventResult::Succeeded);
            assert_eq!(sandbox.current_revision(ObjectId(9)), Some(RevisionId(1)));
            assert_eq!(
                sandbox.object_bytes(ObjectId(9)),
                Some(b"before".as_slice())
            );
            let recovery_id = outcome.recovery_checkpoint().unwrap();
            let recovery = checkpoints.get(recovery_id).unwrap();
            assert_eq!(recovery.reason(), CheckpointReason::AutomaticHistory);
            assert_eq!(
                recovery.object(ObjectId(9)).unwrap().revision_id(),
                RevisionId(2)
            );
            let event = activity.get(outcome.event_id()).unwrap();
            assert_eq!(event.action(), ActionKind::RestoreApplied);
            assert_eq!(event.checkpoint_before(), Some(recovery_id));
            assert_eq!(event.checkpoint_after(), Some(id));
            assert_eq!(event.causal_parent(), Some(CausalParent(plan_event_id)));
            let mut rendered = [0; 192];
            let rendered_len =
                crate::view::render_restore_outcome(outcome, UserLocale::EnUs, &mut rendered)
                    .unwrap();
            let rendered = core::str::from_utf8(&rendered[..rendered_len]).unwrap();
            assert!(rendered.contains("Host in-memory sandbox"));
            assert!(rendered.contains("not target restore"));
            assert!(rendered.contains("Recovery checkpoint #"));

            let rollback_plan = build_plan(
                recovery,
                &mut activity,
                RestoreMode::InPlace,
                &[ObjectId(9)],
                &[CurrentObjectRevision::new(ObjectId(9), RevisionId(1))],
            );
            let rollback = execute_restore(
                rollback_plan.confirm(USER, true).unwrap(),
                time(14),
                &AllowRestore,
                &mut checkpoints,
                &mut sandbox,
                &mut activity,
            )
            .unwrap();
            assert_eq!(rollback.result(), EventResult::Succeeded);
            assert_eq!(sandbox.current_revision(ObjectId(9)), Some(RevisionId(2)));
            assert_eq!(sandbox.object_bytes(ObjectId(9)), Some(b"after".as_slice()));
        }

        #[test]
        fn stale_state_and_permission_denial_do_not_claim_restore_success() {
            let mut sandbox = InMemoryRestoreSandbox::new();
            sandbox
                .register_object(ObjectId(4), RevisionId(1), b"old")
                .unwrap();
            let mut checkpoints = CheckpointStore::new();
            let mut activity = ActivityLedger::new();
            let id = add_checkpoint(
                &mut sandbox,
                &mut checkpoints,
                &mut activity,
                &[ObjectId(4)],
                false,
                1,
            );
            sandbox
                .update_object(ObjectId(4), RevisionId(2), b"new")
                .unwrap();
            let current = [CurrentObjectRevision::new(ObjectId(4), RevisionId(2))];
            let stale_plan = build_plan(
                checkpoints.get(id).unwrap(),
                &mut activity,
                RestoreMode::InPlace,
                &[ObjectId(4)],
                &current,
            );
            sandbox
                .update_object(ObjectId(4), RevisionId(3), b"changed again")
                .unwrap();
            let stale = execute_restore(
                stale_plan.confirm(USER, true).unwrap(),
                time(13),
                &AllowRestore,
                &mut checkpoints,
                &mut sandbox,
                &mut activity,
            )
            .unwrap();
            assert_eq!(stale.result(), EventResult::Failed(FailureCode::StaleState));
            assert_eq!(
                sandbox.object_bytes(ObjectId(4)),
                Some(b"changed again".as_slice())
            );

            let current = [CurrentObjectRevision::new(ObjectId(4), RevisionId(3))];
            let denied_plan = build_plan(
                checkpoints.get(id).unwrap(),
                &mut activity,
                RestoreMode::InPlace,
                &[ObjectId(4)],
                &current,
            );
            let denied = execute_restore(
                denied_plan.confirm(USER, true).unwrap(),
                time(14),
                &DenyRestore,
                &mut checkpoints,
                &mut sandbox,
                &mut activity,
            )
            .unwrap();
            assert_eq!(
                denied.result(),
                EventResult::Failed(FailureCode::PermissionDenied)
            );
            assert_eq!(
                sandbox.object_bytes(ObjectId(4)),
                Some(b"changed again".as_slice())
            );
            assert_eq!(
                activity.get(denied.event_id()).unwrap().action(),
                ActionKind::RestoreFailed
            );
        }

        #[test]
        fn sandbox_supports_restore_as_copy_and_reports_partial_failure() {
            let mut sandbox = InMemoryRestoreSandbox::new();
            for id in [ObjectId(20), ObjectId(21)] {
                sandbox
                    .register_object(id, RevisionId(1), b"snapshot")
                    .unwrap();
            }
            let mut checkpoints = CheckpointStore::new();
            let mut activity = ActivityLedger::new();
            let id = add_checkpoint(
                &mut sandbox,
                &mut checkpoints,
                &mut activity,
                &[ObjectId(20), ObjectId(21)],
                false,
                1,
            );
            let current = [
                CurrentObjectRevision::new(ObjectId(20), RevisionId(1)),
                CurrentObjectRevision::new(ObjectId(21), RevisionId(1)),
            ];
            let copy_plan = build_plan(
                checkpoints.get(id).unwrap(),
                &mut activity,
                RestoreMode::AsCopy,
                &[ObjectId(20)],
                &current,
            );
            let before = sandbox.object_count();
            let copied = execute_restore(
                copy_plan.confirm(USER, true).unwrap(),
                time(3),
                &AllowRestore,
                &mut checkpoints,
                &mut sandbox,
                &mut activity,
            )
            .unwrap();
            assert_eq!(sandbox.object_count(), before + 1);
            let copy_id = copied.created_objects().next().unwrap();
            assert_eq!(copied.created_objects().count(), 1);
            assert_ne!(copy_id, ObjectId(20));
            assert_eq!(sandbox.object_bytes(copy_id), Some(b"snapshot".as_slice()));
            let copy_event = activity.get(copied.event_id()).unwrap();
            assert_eq!(copy_event.sources().next(), Some(ObjectId(20)));
            assert_eq!(copy_event.targets().next(), Some(copy_id));
            let mut output = [0; 192];
            let output_length = crate::view::render_restore_outcome(
                copied,
                crate::activity::UserLocale::EnUs,
                &mut output,
            )
            .unwrap();
            assert!(core::str::from_utf8(&output[..output_length])
                .unwrap()
                .contains("Created copies:"));

            let partial_plan = build_plan(
                checkpoints.get(id).unwrap(),
                &mut activity,
                RestoreMode::AsCopy,
                &[ObjectId(20), ObjectId(21)],
                &current,
            );
            sandbox.fail_after(Some(1));
            let partial = execute_restore(
                partial_plan.confirm(USER, true).unwrap(),
                time(4),
                &AllowRestore,
                &mut checkpoints,
                &mut sandbox,
                &mut activity,
            )
            .unwrap();
            assert_eq!(
                partial.result(),
                EventResult::Partial {
                    succeeded: 1,
                    failed: 1
                }
            );
            assert_eq!(partial.created_objects().count(), 1);
            let partial_copy = partial.created_objects().next().unwrap();
            assert_eq!(
                sandbox.object_bytes(partial_copy),
                Some(b"snapshot".as_slice())
            );
            let partial_event = activity.get(partial.event_id()).unwrap();
            assert_eq!(partial_event.targets().next(), Some(partial_copy));
        }

        #[test]
        fn copy_id_allocation_fails_boundedly_at_identifier_exhaustion() {
            let mut sandbox = InMemoryRestoreSandbox::new();
            sandbox
                .register_object(ObjectId(30), RevisionId(1), b"snapshot")
                .unwrap();
            sandbox
                .register_object(ObjectId(u64::MAX), RevisionId(1), b"occupied")
                .unwrap();
            let mut checkpoints = CheckpointStore::new();
            let mut activity = ActivityLedger::new();
            let id = add_checkpoint(
                &mut sandbox,
                &mut checkpoints,
                &mut activity,
                &[ObjectId(30)],
                false,
                1,
            );
            sandbox.set_next_copy_object_for_test(u64::MAX);
            let current = [CurrentObjectRevision::new(ObjectId(30), RevisionId(1))];
            let plan = build_plan(
                checkpoints.get(id).unwrap(),
                &mut activity,
                RestoreMode::AsCopy,
                &[ObjectId(30)],
                &current,
            );
            let before = sandbox.object_count();
            let outcome = execute_restore(
                plan.confirm(USER, true).unwrap(),
                time(2),
                &AllowRestore,
                &mut checkpoints,
                &mut sandbox,
                &mut activity,
            )
            .unwrap();
            assert_eq!(outcome.result(), EventResult::Failed(FailureCode::Capacity));
            assert_eq!(sandbox.object_count(), before);
            assert_eq!(outcome.created_objects().count(), 0);
        }
    }
}
