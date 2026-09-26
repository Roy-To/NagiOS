//! Append-only, bounded Activity contracts layered beside the M15 undo/history API.
//!
//! Activity records describe operations and references only. Arbitrary payloads,
//! document contents, credentials, and model reasoning are never retained here.

use crate::{ActivityContext, ObjectId, WorkspaceId};
pub use nagi_model::TransactionId;

pub const MAX_ACTIVITY_EVENTS: usize = 128;
pub const MAX_ACTIVITY_OBJECTS: usize = 4;
pub const MAX_ACTIVITY_METADATA: usize = 8;
pub const NANOS_PER_SECOND: u32 = 1_000_000_000;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp {
    seconds: i64,
    nanos: u32,
}

impl Timestamp {
    pub fn new(seconds: i64, nanos: u32) -> Result<Self, ActivityError> {
        if nanos >= NANOS_PER_SECOND {
            return Err(ActivityError::InvalidTimestamp);
        }
        Ok(Self { seconds, nanos })
    }

    pub const fn seconds(self) -> i64 {
        self.seconds
    }

    pub const fn nanos(self) -> u32 {
        self.nanos
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EventId(u64);

impl EventId {
    pub fn new(value: u64) -> Result<Self, ActivityError> {
        if value == 0 || value == u64::MAX {
            return Err(ActivityError::InvalidEventId);
        }
        Ok(Self(value))
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActorId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActorKind {
    User,
    Agent,
    App,
    System,
    Automation,
    RemoteDevice,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Actor {
    id: ActorId,
    kind: ActorKind,
}

impl Actor {
    pub const fn new(id: ActorId, kind: ActorKind) -> Self {
        Self { id, kind }
    }

    pub const fn id(self) -> ActorId {
        self.id
    }

    pub const fn kind(self) -> ActorKind {
        self.kind
    }
}

/// Groups technical Activity records into one user-understandable action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionGroupId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CorrelationId(pub u64);

/// Optional device identity supplied by a device-aware producer. `NodeId`
/// remains the required execution-origin identity from the M15 context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeviceId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IntentId(pub u64);

/// Opaque reference to an authority decision; it carries no rights by itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AuthorityRef(pub u64);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct CheckpointId(pub u64);

impl CheckpointId {
    pub fn new(value: u64) -> Result<Self, ActivityError> {
        if value == 0 || value == u64::MAX {
            return Err(ActivityError::InvalidCheckpointId);
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RevisionId(pub u64);

impl RevisionId {
    pub fn new(value: u64) -> Result<Self, ActivityError> {
        if value == 0 || value == u64::MAX {
            return Err(ActivityError::InvalidRevisionId);
        }
        Ok(Self(value))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CausalParent(pub EventId);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Provenance {
    Direct {
        originating_intent: Option<IntentId>,
    },
    Delegated {
        requested_by: Actor,
        originating_intent: IntentId,
        delegated_authority: AuthorityRef,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionKind {
    ObjectCreated,
    ObjectChanged,
    ObjectMoved,
    ObjectDeleted,
    ObjectRestored,
    ObjectAccessed,
    CheckpointCreated,
    CheckpointPinned,
    CheckpointUnpinned,
    RestorePlanned,
    RestoreApplied,
    RestoreFailed,
    UndoApplied,
    OperationFailed,
    Custom(u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureCode {
    PermissionDenied,
    StaleState,
    MissingObject,
    MissingCheckpoint,
    Unsupported,
    BackendFailure,
    Capacity,
    Validation,
    Other(u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventResult {
    Pending,
    Succeeded,
    Failed(FailureCode),
    Partial { succeeded: u8, failed: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InverseKind {
    RemoveCreatedObject,
    RestoreObjectRevision,
    MoveToPreviousLocation,
    RestoreDeletedObject,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UndoDescriptor {
    inverse: InverseKind,
    expected_current: RevisionId,
    target_revision: RevisionId,
}

impl UndoDescriptor {
    pub const fn new(
        inverse: InverseKind,
        expected_current: RevisionId,
        target_revision: RevisionId,
    ) -> Self {
        Self {
            inverse,
            expected_current,
            target_revision,
        }
    }

    pub const fn inverse(self) -> InverseKind {
        self.inverse
    }

    pub const fn expected_current(self) -> RevisionId {
        self.expected_current
    }

    pub const fn target_revision(self) -> RevisionId {
        self.target_revision
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reversibility {
    Unknown,
    Irreversible,
    Reversible(UndoDescriptor),
    ConditionallyReversible(UndoDescriptor),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WaybackTarget {
    Checkpoint(CheckpointId),
    ObjectRevision {
        object_id: ObjectId,
        revision_id: RevisionId,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrivacyClass {
    PublicMetadata,
    Sensitive,
    Secret,
    Content,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataKey {
    ItemCount,
    ByteCount,
    ContentTypeCode,
    FailureCode,
    Credential,
    Password,
    AccessToken,
    AuthenticationData,
    DocumentContent,
    SensitivePayload,
    ConfirmedByActor,
    RestoreMode,
    Other(u16),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataValue {
    Count(u32),
    Bytes(u64),
    Code(u32),
    Flag(bool),
    ActorReference(ActorId),
    Redacted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetadataEntry {
    key: MetadataKey,
    privacy: PrivacyClass,
    value: MetadataValue,
}

impl MetadataEntry {
    pub const fn key(self) -> MetadataKey {
        self.key
    }

    pub const fn privacy(self) -> PrivacyClass {
        self.privacy
    }

    pub const fn value(self) -> MetadataValue {
        self.value
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ObjectRefs {
    values: [Option<ObjectId>; MAX_ACTIVITY_OBJECTS],
    length: u8,
}

impl ObjectRefs {
    const EMPTY: Self = Self {
        values: [None; MAX_ACTIVITY_OBJECTS],
        length: 0,
    };

    fn push(&mut self, value: ObjectId) -> Result<(), ActivityError> {
        if self.contains(value) {
            return Ok(());
        }
        if self.length as usize == MAX_ACTIVITY_OBJECTS {
            return Err(ActivityError::TooManyObjects);
        }
        self.values[self.length as usize] = Some(value);
        self.length += 1;
        Ok(())
    }

    fn contains(self, value: ObjectId) -> bool {
        self.values[..self.length as usize]
            .iter()
            .any(|item| *item == Some(value))
    }

    fn iter(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.values[..self.length as usize]
            .iter()
            .filter_map(|value| *value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivityDraft {
    occurred_at: Timestamp,
    actor: Actor,
    context: ActivityContext,
    action: ActionKind,
    result: EventResult,
    reversibility: Reversibility,
    provenance: Provenance,
    targets: ObjectRefs,
    sources: ObjectRefs,
    action_group_id: Option<ActionGroupId>,
    transaction_id: Option<TransactionId>,
    correlation_id: Option<CorrelationId>,
    device_id: Option<DeviceId>,
    causal_parent: Option<CausalParent>,
    checkpoint_before: Option<CheckpointId>,
    checkpoint_after: Option<CheckpointId>,
    metadata: [Option<MetadataEntry>; MAX_ACTIVITY_METADATA],
    metadata_length: u8,
}

impl ActivityDraft {
    pub const fn new(
        occurred_at: Timestamp,
        actor: Actor,
        context: ActivityContext,
        action: ActionKind,
    ) -> Self {
        Self {
            occurred_at,
            actor,
            context,
            action,
            result: EventResult::Pending,
            reversibility: Reversibility::Unknown,
            provenance: Provenance::Direct {
                originating_intent: None,
            },
            targets: ObjectRefs::EMPTY,
            sources: ObjectRefs::EMPTY,
            action_group_id: None,
            transaction_id: None,
            correlation_id: None,
            device_id: None,
            causal_parent: None,
            checkpoint_before: None,
            checkpoint_after: None,
            metadata: [None; MAX_ACTIVITY_METADATA],
            metadata_length: 0,
        }
    }

    /// Validate and materialize a persisted immutable event with its stored ID.
    /// Durable Activity adapters use this when decoding a record.
    pub fn into_event(self, id: EventId) -> Result<ActivityEvent, ActivityError> {
        let draft = self.validate()?;
        Ok(ActivityEvent {
            id,
            occurred_at: draft.occurred_at,
            actor: draft.actor,
            context: draft.context,
            action: draft.action,
            result: draft.result,
            reversibility: draft.reversibility,
            provenance: draft.provenance,
            targets: draft.targets,
            sources: draft.sources,
            action_group_id: draft.action_group_id,
            transaction_id: draft.transaction_id,
            correlation_id: draft.correlation_id,
            device_id: draft.device_id,
            causal_parent: draft.causal_parent,
            checkpoint_before: draft.checkpoint_before,
            checkpoint_after: draft.checkpoint_after,
            metadata: draft.metadata,
            metadata_length: draft.metadata_length,
        })
    }

    pub fn with_target(mut self, object: ObjectId) -> Result<Self, ActivityError> {
        self.targets.push(object)?;
        Ok(self)
    }

    pub fn with_source(mut self, object: ObjectId) -> Result<Self, ActivityError> {
        self.sources.push(object)?;
        Ok(self)
    }

    pub const fn with_transaction(mut self, id: TransactionId) -> Self {
        self.transaction_id = Some(id);
        self
    }

    pub const fn with_action_group(mut self, id: ActionGroupId) -> Self {
        self.action_group_id = Some(id);
        self
    }

    pub const fn with_correlation(mut self, id: CorrelationId) -> Self {
        self.correlation_id = Some(id);
        self
    }

    pub const fn with_device(mut self, id: DeviceId) -> Self {
        self.device_id = Some(id);
        self
    }

    pub const fn with_parent(mut self, parent: CausalParent) -> Self {
        self.causal_parent = Some(parent);
        self
    }

    pub const fn with_provenance(mut self, provenance: Provenance) -> Self {
        self.provenance = provenance;
        self
    }

    pub const fn with_result(mut self, result: EventResult) -> Self {
        self.result = result;
        self
    }

    pub const fn with_reversibility(mut self, reversibility: Reversibility) -> Self {
        self.reversibility = reversibility;
        self
    }

    pub const fn with_checkpoint_before(mut self, id: CheckpointId) -> Self {
        self.checkpoint_before = Some(id);
        self
    }

    pub const fn with_checkpoint_after(mut self, id: CheckpointId) -> Self {
        self.checkpoint_after = Some(id);
        self
    }

    /// Add a typed scalar. Values classified as private or secret are retained only as
    /// a redaction marker; credentials and content keys are always redacted.
    pub fn with_metadata(
        mut self,
        key: MetadataKey,
        value: MetadataValue,
        privacy: PrivacyClass,
    ) -> Result<Self, ActivityError> {
        let safe_value = if privacy != PrivacyClass::PublicMetadata || key.is_sensitive() {
            MetadataValue::Redacted
        } else {
            value
        };
        self.push_metadata(MetadataEntry {
            key,
            privacy,
            value: safe_value,
        })?;
        Ok(self)
    }

    /// Record that a payload existed without copying any of its bytes into the event.
    pub fn with_redacted_payload(
        mut self,
        key: MetadataKey,
        privacy: PrivacyClass,
        _payload: &[u8],
    ) -> Result<Self, ActivityError> {
        self.push_metadata(MetadataEntry {
            key,
            privacy,
            value: MetadataValue::Redacted,
        })?;
        Ok(self)
    }

    fn push_metadata(&mut self, field: MetadataEntry) -> Result<(), ActivityError> {
        if self.metadata_length as usize == MAX_ACTIVITY_METADATA {
            return Err(ActivityError::TooManyMetadataFields);
        }
        self.metadata[self.metadata_length as usize] = Some(field);
        self.metadata_length += 1;
        Ok(())
    }

    pub(crate) fn validate(self) -> Result<Self, ActivityError> {
        if self.actor.kind == ActorKind::Agent
            && !matches!(self.provenance, Provenance::Delegated { .. })
        {
            return Err(ActivityError::MissingDelegationProvenance);
        }
        if let Provenance::Delegated {
            delegated_authority,
            ..
        } = self.provenance
        {
            if delegated_authority.0 == 0 {
                return Err(ActivityError::InvalidAuthorityReference);
            }
        }
        Ok(self)
    }
}

impl MetadataKey {
    const fn is_sensitive(self) -> bool {
        matches!(
            self,
            Self::Credential
                | Self::Password
                | Self::AccessToken
                | Self::AuthenticationData
                | Self::DocumentContent
                | Self::SensitivePayload
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivityEvent {
    id: EventId,
    occurred_at: Timestamp,
    actor: Actor,
    context: ActivityContext,
    action: ActionKind,
    result: EventResult,
    reversibility: Reversibility,
    provenance: Provenance,
    targets: ObjectRefs,
    sources: ObjectRefs,
    action_group_id: Option<ActionGroupId>,
    transaction_id: Option<TransactionId>,
    correlation_id: Option<CorrelationId>,
    device_id: Option<DeviceId>,
    causal_parent: Option<CausalParent>,
    checkpoint_before: Option<CheckpointId>,
    checkpoint_after: Option<CheckpointId>,
    metadata: [Option<MetadataEntry>; MAX_ACTIVITY_METADATA],
    metadata_length: u8,
}

impl ActivityEvent {
    pub const fn id(&self) -> EventId {
        self.id
    }

    pub const fn occurred_at(&self) -> Timestamp {
        self.occurred_at
    }

    pub const fn actor(&self) -> Actor {
        self.actor
    }

    pub const fn context(&self) -> ActivityContext {
        self.context
    }

    pub const fn action(&self) -> ActionKind {
        self.action
    }

    pub const fn result(&self) -> EventResult {
        self.result
    }

    pub const fn reversibility(&self) -> Reversibility {
        self.reversibility
    }

    pub const fn provenance(&self) -> Provenance {
        self.provenance
    }

    pub const fn transaction_id(&self) -> Option<TransactionId> {
        self.transaction_id
    }

    pub const fn action_group_id(&self) -> Option<ActionGroupId> {
        self.action_group_id
    }

    pub const fn correlation_id(&self) -> Option<CorrelationId> {
        self.correlation_id
    }

    pub const fn device_id(&self) -> Option<DeviceId> {
        self.device_id
    }

    pub const fn causal_parent(&self) -> Option<CausalParent> {
        self.causal_parent
    }

    pub const fn checkpoint_before(&self) -> Option<CheckpointId> {
        self.checkpoint_before
    }

    pub const fn checkpoint_after(&self) -> Option<CheckpointId> {
        self.checkpoint_after
    }

    pub fn targets(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.targets.iter()
    }

    pub fn sources(&self) -> impl Iterator<Item = ObjectId> + '_ {
        self.sources.iter()
    }

    pub fn metadata(&self) -> impl Iterator<Item = MetadataEntry> + '_ {
        self.metadata[..self.metadata_length as usize]
            .iter()
            .filter_map(|entry| *entry)
    }
}

/// Resolve the most useful Wayback destination from a semantic event. A
/// before-checkpoint wins; reversible events can otherwise link to their
/// declared inverse revision. No path or guessed resource identifier is used.
pub fn wayback_target(event: &ActivityEvent) -> Option<WaybackTarget> {
    if let Some(checkpoint) = event.checkpoint_before.or(event.checkpoint_after) {
        return Some(WaybackTarget::Checkpoint(checkpoint));
    }
    let descriptor = match event.reversibility {
        Reversibility::Reversible(descriptor)
        | Reversibility::ConditionallyReversible(descriptor) => descriptor,
        Reversibility::Unknown | Reversibility::Irreversible => return None,
    };
    let mut targets = event.targets();
    let object_id = targets.next()?;
    targets
        .next()
        .is_none()
        .then_some(WaybackTarget::ObjectRevision {
            object_id,
            revision_id: descriptor.target_revision(),
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityError {
    Capacity,
    InvalidEventId,
    InvalidCheckpointId,
    InvalidRevisionId,
    InvalidTimestamp,
    InvalidTimeRange,
    TooManyObjects,
    TooManyMetadataFields,
    MissingDelegationProvenance,
    InvalidAuthorityReference,
}

pub trait ActivitySink {
    fn can_append(&self) -> bool;

    /// Report whether a sequence of events can be appended without an
    /// intervening writer. Adapters that support multi-event operations must
    /// override this check; the default only permits a single event.
    fn can_append_events(&self, count: usize) -> bool {
        count == 0 || (count == 1 && self.can_append())
    }

    fn append(&mut self, draft: ActivityDraft) -> Result<EventId, ActivityError>;
}

pub struct ActivityLedger {
    events: [Option<ActivityEvent>; MAX_ACTIVITY_EVENTS],
    length: usize,
    next_event_id: u64,
}

/// Deterministic bounded store for host previews and contract tests. It does not
/// claim durable Nagi storage; production persistence is supplied by an adapter.
pub type InMemoryActivityStore = ActivityLedger;

impl ActivityLedger {
    pub const fn new() -> Self {
        Self {
            events: [None; MAX_ACTIVITY_EVENTS],
            length: 0,
            next_event_id: 1,
        }
    }

    pub const fn len(&self) -> usize {
        self.length
    }

    pub const fn is_empty(&self) -> bool {
        self.length == 0
    }

    pub const fn remaining_capacity(&self) -> usize {
        MAX_ACTIVITY_EVENTS - self.length
    }

    pub fn can_append(&self) -> bool {
        self.remaining_capacity() > 0 && self.next_event_id != u64::MAX
    }

    pub(crate) fn get(&self, id: EventId) -> Option<&ActivityEvent> {
        self.events[..self.length]
            .iter()
            .filter_map(|event| event.as_ref())
            .find(|event| event.id == id)
    }

    pub fn get_visible(
        &self,
        id: EventId,
        viewer: Actor,
        policy: &dyn ActivityAccessPolicy,
    ) -> Option<&ActivityEvent> {
        self.get(id).filter(|event| policy.can_read(viewer, event))
    }

    pub fn append(&mut self, draft: ActivityDraft) -> Result<EventId, ActivityError> {
        if !self.can_append() {
            return Err(ActivityError::Capacity);
        }
        let id = EventId(self.next_event_id);
        let event = draft.into_event(id)?;
        self.events[self.length] = Some(event);
        self.length += 1;
        self.next_event_id += 1;
        Ok(id)
    }

    #[cfg(test)]
    pub(crate) fn query(&self, filter: ActivityQuery) -> ActivityQueryIter<'_> {
        ActivityQueryIter {
            ledger: self,
            filter,
            viewer: None,
            policy: None,
            cursor: None,
        }
    }

    /// User-facing query path. The policy is mandatory so UI/search consumers
    /// cannot accidentally treat private event visibility as an unconditional grant.
    pub fn query_visible<'a>(
        &'a self,
        filter: ActivityQuery,
        viewer: Actor,
        policy: &'a dyn ActivityAccessPolicy,
    ) -> ActivityQueryIter<'a> {
        ActivityQueryIter {
            ledger: self,
            filter,
            viewer: Some(viewer),
            policy: Some(policy),
            cursor: None,
        }
    }
}

impl Default for ActivityLedger {
    fn default() -> Self {
        Self::new()
    }
}

impl ActivitySink for ActivityLedger {
    fn can_append(&self) -> bool {
        ActivityLedger::can_append(self)
    }

    fn can_append_events(&self, count: usize) -> bool {
        count <= self.remaining_capacity()
            && (count as u64) <= u64::MAX.saturating_sub(self.next_event_id)
    }

    fn append(&mut self, draft: ActivityDraft) -> Result<EventId, ActivityError> {
        ActivityLedger::append(self, draft)
    }
}

pub trait ActivityAccessPolicy {
    fn can_read(&self, viewer: Actor, event: &ActivityEvent) -> bool;
}

/// Read contract for durable Activity adapters. The bounded in-memory ledger
/// implements it today; an OS service can provide an iterator without
/// changing event producers or the Activity view/search contracts. `viewer`
/// and `policy` must come from the trusted authenticated service boundary;
/// these caller-supplied values are not themselves an OS capability check.
pub trait ActivityReadStore {
    type QueryIter<'a>: Iterator<Item = &'a ActivityEvent>
    where
        Self: 'a;

    fn get_visible(
        &self,
        id: EventId,
        viewer: Actor,
        policy: &dyn ActivityAccessPolicy,
    ) -> Option<&ActivityEvent>;

    fn query_visible<'a>(
        &'a self,
        filter: ActivityQuery,
        viewer: Actor,
        policy: &'a dyn ActivityAccessPolicy,
    ) -> Self::QueryIter<'a>;
}

/// Combined writer/reader boundary for the Activity service. `ActivityLedger`
/// is the bounded host/test implementation; target persistence adapters can
/// implement this contract while retaining the same policy-filtered queries.
pub trait ActivityStore: ActivitySink + ActivityReadStore {}

impl<T: ActivitySink + ActivityReadStore> ActivityStore for T {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TargetOpenRequest {
    event_id: EventId,
    actor: Actor,
    context: crate::ActivityContext,
    target: ObjectId,
    correlation_id: Option<CorrelationId>,
}

impl TargetOpenRequest {
    pub const fn event_id(self) -> EventId {
        self.event_id
    }

    pub const fn actor(self) -> Actor {
        self.actor
    }

    pub const fn context(self) -> crate::ActivityContext {
        self.context
    }

    pub const fn target(self) -> ObjectId {
        self.target
    }

    pub const fn correlation_id(self) -> Option<CorrelationId> {
        self.correlation_id
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TargetOpenError {
    EventUnavailable,
    TargetUnavailable,
    Denied(FailureCode),
}

pub trait ActivityTargetOpenPolicy {
    fn authorize_open(
        &self,
        viewer: Actor,
        event: &ActivityEvent,
        target: ObjectId,
    ) -> Result<(), FailureCode>;
}

/// App-specific adapter that resolves an ObjectId through its capability-aware
/// resource provider. It must not treat an ObjectId as a host path.
pub trait ActivityTargetOpener {
    fn open(&mut self, request: TargetOpenRequest) -> Result<(), FailureCode>;
}

/// Open one event target only after both event visibility and target authority
/// succeed. The resource adapter owns app navigation and its own Activity event.
pub fn open_event_target<S: ActivityReadStore>(
    ledger: &S,
    event_id: EventId,
    target_index: usize,
    viewer: Actor,
    visibility: &dyn ActivityAccessPolicy,
    target_policy: &dyn ActivityTargetOpenPolicy,
    opener: &mut dyn ActivityTargetOpener,
) -> Result<(), TargetOpenError> {
    let event = ledger
        .get_visible(event_id, viewer, visibility)
        .ok_or(TargetOpenError::EventUnavailable)?;
    let target = event
        .targets()
        .nth(target_index)
        .ok_or(TargetOpenError::TargetUnavailable)?;
    target_policy
        .authorize_open(viewer, event, target)
        .map_err(TargetOpenError::Denied)?;
    opener
        .open(TargetOpenRequest {
            event_id,
            actor: viewer,
            context: event.context,
            target,
            correlation_id: event.correlation_id,
        })
        .map_err(TargetOpenError::Denied)
}

/// Search-facing contract. Consumers apply the same permission filter as the
/// Activity UI; a separate search provider may not widen event visibility.
pub trait TemporalActivitySearch: ActivityReadStore {
    fn search<'a>(
        &'a self,
        filter: ActivityQuery,
        viewer: Actor,
        policy: &'a dyn ActivityAccessPolicy,
    ) -> Self::QueryIter<'a> {
        self.query_visible(filter, viewer, policy)
    }
}

impl<T: ActivityReadStore> TemporalActivitySearch for T {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivityTimeRange {
    start_inclusive: Timestamp,
    end_exclusive: Timestamp,
}

impl ActivityTimeRange {
    pub fn new(
        start_inclusive: Timestamp,
        end_exclusive: Timestamp,
    ) -> Result<Self, ActivityError> {
        if start_inclusive > end_exclusive {
            return Err(ActivityError::InvalidTimeRange);
        }
        Ok(Self {
            start_inclusive,
            end_exclusive,
        })
    }

    pub const fn start_inclusive(self) -> Timestamp {
        self.start_inclusive
    }

    pub const fn end_exclusive(self) -> Timestamp {
        self.end_exclusive
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ActivityQuery {
    time_range: Option<ActivityTimeRange>,
    actor_id: Option<ActorId>,
    actor_kind: Option<ActorKind>,
    action: Option<ActionKind>,
    target: Option<ObjectId>,
    device_id: Option<DeviceId>,
    action_group_id: Option<ActionGroupId>,
    workspace: Option<WorkspaceId>,
    transaction_id: Option<TransactionId>,
    correlation_id: Option<CorrelationId>,
    checkpoint_id: Option<CheckpointId>,
}

impl ActivityQuery {
    pub const fn time_range(mut self, range: ActivityTimeRange) -> Self {
        self.time_range = Some(range);
        self
    }

    pub const fn actor_id(mut self, actor_id: ActorId) -> Self {
        self.actor_id = Some(actor_id);
        self
    }

    pub const fn actor_kind(mut self, actor_kind: ActorKind) -> Self {
        self.actor_kind = Some(actor_kind);
        self
    }

    pub const fn action(mut self, action: ActionKind) -> Self {
        self.action = Some(action);
        self
    }

    pub const fn target(mut self, object: ObjectId) -> Self {
        self.target = Some(object);
        self
    }

    pub const fn device(mut self, device: DeviceId) -> Self {
        self.device_id = Some(device);
        self
    }

    pub const fn action_group(mut self, action_group: ActionGroupId) -> Self {
        self.action_group_id = Some(action_group);
        self
    }

    pub const fn workspace(mut self, workspace: WorkspaceId) -> Self {
        self.workspace = Some(workspace);
        self
    }

    pub const fn transaction(mut self, transaction: TransactionId) -> Self {
        self.transaction_id = Some(transaction);
        self
    }

    pub const fn correlation(mut self, correlation: CorrelationId) -> Self {
        self.correlation_id = Some(correlation);
        self
    }

    pub const fn checkpoint(mut self, checkpoint: CheckpointId) -> Self {
        self.checkpoint_id = Some(checkpoint);
        self
    }

    pub fn matches(self, event: &ActivityEvent) -> bool {
        if let Some(range) = self.time_range {
            if event.occurred_at < range.start_inclusive || event.occurred_at >= range.end_exclusive
            {
                return false;
            }
        }
        self.actor_id.is_none_or(|id| event.actor.id == id)
            && self.actor_kind.is_none_or(|kind| event.actor.kind == kind)
            && self.action.is_none_or(|action| event.action == action)
            && self
                .target
                .is_none_or(|target| event.targets.contains(target))
            && self
                .device_id
                .is_none_or(|device| event.device_id == Some(device))
            && self
                .action_group_id
                .is_none_or(|group| event.action_group_id == Some(group))
            && self
                .workspace
                .is_none_or(|workspace| event.context.workspace_id == Some(workspace))
            && self
                .transaction_id
                .is_none_or(|id| event.transaction_id == Some(id))
            && self
                .correlation_id
                .is_none_or(|id| event.correlation_id == Some(id))
            && self.checkpoint_id.is_none_or(|id| {
                event.checkpoint_before == Some(id) || event.checkpoint_after == Some(id)
            })
    }
}

pub struct ActivityQueryIter<'a> {
    ledger: &'a ActivityLedger,
    filter: ActivityQuery,
    viewer: Option<Actor>,
    policy: Option<&'a dyn ActivityAccessPolicy>,
    cursor: Option<(Timestamp, EventId)>,
}

impl<'a> Iterator for ActivityQueryIter<'a> {
    type Item = &'a ActivityEvent;

    fn next(&mut self) -> Option<Self::Item> {
        let mut next: Option<&ActivityEvent> = None;
        for event in self.ledger.events[..self.ledger.length]
            .iter()
            .filter_map(|event| event.as_ref())
        {
            let key = (event.occurred_at, event.id);
            if self.cursor.is_some_and(|cursor| key <= cursor)
                || !self.filter.matches(event)
                || self
                    .policy
                    .zip(self.viewer)
                    .is_some_and(|(policy, viewer)| !policy.can_read(viewer, event))
            {
                continue;
            }
            if next.is_none_or(|current| key < (current.occurred_at, current.id)) {
                next = Some(event);
            }
        }
        if let Some(event) = next {
            self.cursor = Some((event.occurred_at, event.id));
        }
        next
    }
}

impl ActivityReadStore for ActivityLedger {
    type QueryIter<'a>
        = ActivityQueryIter<'a>
    where
        Self: 'a;

    fn get_visible(
        &self,
        id: EventId,
        viewer: Actor,
        policy: &dyn ActivityAccessPolicy,
    ) -> Option<&ActivityEvent> {
        ActivityLedger::get_visible(self, id, viewer, policy)
    }

    fn query_visible<'a>(
        &'a self,
        filter: ActivityQuery,
        viewer: Actor,
        policy: &'a dyn ActivityAccessPolicy,
    ) -> Self::QueryIter<'a> {
        ActivityLedger::query_visible(self, filter, viewer, policy)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserLocale {
    EnUs,
    JaJp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RenderError {
    BufferTooSmall,
}

pub fn render_summary(
    event: &ActivityEvent,
    locale: UserLocale,
    output: &mut [u8],
) -> Result<usize, RenderError> {
    let text: &[u8] = match (locale, event.action) {
        (UserLocale::EnUs, ActionKind::ObjectCreated) => b"Created an item",
        (UserLocale::EnUs, ActionKind::ObjectChanged) => b"Changed an item",
        (UserLocale::EnUs, ActionKind::ObjectMoved) => b"Moved an item",
        (UserLocale::EnUs, ActionKind::ObjectDeleted) => b"Deleted an item",
        (UserLocale::EnUs, ActionKind::ObjectRestored) => b"Restored an item",
        (UserLocale::EnUs, ActionKind::ObjectAccessed) => b"Opened an item",
        (UserLocale::EnUs, ActionKind::CheckpointCreated) => b"Created a checkpoint",
        (UserLocale::EnUs, ActionKind::CheckpointPinned) => b"Pinned a checkpoint",
        (UserLocale::EnUs, ActionKind::CheckpointUnpinned) => b"Unpinned a checkpoint",
        (UserLocale::EnUs, ActionKind::RestorePlanned) => b"Prepared a restore preview",
        (UserLocale::EnUs, ActionKind::RestoreApplied) => b"Restored a previous state",
        (UserLocale::EnUs, ActionKind::RestoreFailed) => b"Restore did not complete",
        (UserLocale::EnUs, ActionKind::UndoApplied) => b"Undid an operation",
        (UserLocale::EnUs, ActionKind::OperationFailed) => b"An operation failed",
        (UserLocale::EnUs, ActionKind::Custom(_)) => b"Performed an application action",
        (UserLocale::JaJp, ActionKind::ObjectCreated) => "項目を作成".as_bytes(),
        (UserLocale::JaJp, ActionKind::ObjectChanged) => "項目を変更".as_bytes(),
        (UserLocale::JaJp, ActionKind::ObjectMoved) => "項目を移動".as_bytes(),
        (UserLocale::JaJp, ActionKind::ObjectDeleted) => "項目を削除".as_bytes(),
        (UserLocale::JaJp, ActionKind::ObjectRestored) => "項目を復元".as_bytes(),
        (UserLocale::JaJp, ActionKind::ObjectAccessed) => "項目を開く".as_bytes(),
        (UserLocale::JaJp, ActionKind::CheckpointCreated) => "チェックポイントを作成".as_bytes(),
        (UserLocale::JaJp, ActionKind::CheckpointPinned) => "チェックポイントを固定".as_bytes(),
        (UserLocale::JaJp, ActionKind::CheckpointUnpinned) => {
            "チェックポイントの固定を解除".as_bytes()
        }
        (UserLocale::JaJp, ActionKind::RestorePlanned) => "復元プレビューを作成".as_bytes(),
        (UserLocale::JaJp, ActionKind::RestoreApplied) => "以前の状態を復元".as_bytes(),
        (UserLocale::JaJp, ActionKind::RestoreFailed) => "復元を完了できませんでした".as_bytes(),
        (UserLocale::JaJp, ActionKind::UndoApplied) => "操作を取り消しました".as_bytes(),
        (UserLocale::JaJp, ActionKind::OperationFailed) => "操作に失敗しました".as_bytes(),
        (UserLocale::JaJp, ActionKind::Custom(_)) => "アプリの操作を実行".as_bytes(),
    };
    copy_text(text, output)
}

pub fn render_actor(
    actor: Actor,
    locale: UserLocale,
    output: &mut [u8],
) -> Result<usize, RenderError> {
    let text: &[u8] = match (locale, actor.kind) {
        (UserLocale::EnUs, ActorKind::User) => b"You",
        (UserLocale::EnUs, ActorKind::Agent) => b"AI assistant",
        (UserLocale::EnUs, ActorKind::App) => b"Application",
        (UserLocale::EnUs, ActorKind::System) => b"Nagi",
        (UserLocale::EnUs, ActorKind::Automation) => b"Automation",
        (UserLocale::EnUs, ActorKind::RemoteDevice) => b"Remote device",
        (UserLocale::JaJp, ActorKind::User) => "あなた".as_bytes(),
        (UserLocale::JaJp, ActorKind::Agent) => "AIアシスタント".as_bytes(),
        (UserLocale::JaJp, ActorKind::App) => "アプリケーション".as_bytes(),
        (UserLocale::JaJp, ActorKind::System) => "Nagi".as_bytes(),
        (UserLocale::JaJp, ActorKind::Automation) => "自動処理".as_bytes(),
        (UserLocale::JaJp, ActorKind::RemoteDevice) => "リモートデバイス".as_bytes(),
    };
    copy_text(text, output)
}

pub fn render_empty_state(locale: UserLocale, output: &mut [u8]) -> Result<usize, RenderError> {
    let text: &[u8] = match locale {
        UserLocale::EnUs => b"No activity yet.",
        UserLocale::JaJp => "履歴はまだありません。".as_bytes(),
    };
    copy_text(text, output)
}

fn copy_text(text: &[u8], output: &mut [u8]) -> Result<usize, RenderError> {
    if output.len() < text.len() {
        return Err(RenderError::BufferTooSmall);
    }
    output[..text.len()].copy_from_slice(text);
    Ok(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppId, AppSessionId, NodeId};

    const CONTEXT: ActivityContext = ActivityContext {
        app_id: AppId(11),
        app_session_id: AppSessionId(12),
        node_id: NodeId(13),
        surface_id: None,
        workspace_id: Some(WorkspaceId(14)),
    };
    const USER: Actor = Actor::new(ActorId(1), ActorKind::User);
    const AGENT: Actor = Actor::new(ActorId(2), ActorKind::Agent);

    fn time(seconds: i64) -> Timestamp {
        Timestamp::new(seconds, 0).unwrap()
    }

    fn draft(seconds: i64, actor: Actor, action: ActionKind) -> ActivityDraft {
        ActivityDraft::new(time(seconds), actor, CONTEXT, action)
            .with_result(EventResult::Succeeded)
    }

    fn delegated() -> Provenance {
        Provenance::Delegated {
            requested_by: USER,
            originating_intent: IntentId(22),
            delegated_authority: AuthorityRef(33),
        }
    }

    #[test]
    fn new_activity_drafts_remain_pending_until_completion_is_reported() {
        let mut ledger = ActivityLedger::new();
        let event_id = ledger
            .append(ActivityDraft::new(
                time(1),
                USER,
                CONTEXT,
                ActionKind::ObjectChanged,
            ))
            .unwrap();
        assert_eq!(ledger.get(event_id).unwrap().result(), EventResult::Pending);
    }

    #[test]
    fn durable_event_materialization_preserves_typed_fields_and_stored_identity() {
        let event = draft(8, USER, ActionKind::ObjectChanged)
            .with_target(ObjectId(91))
            .unwrap()
            .with_transaction(TransactionId(14))
            .with_correlation(CorrelationId(15))
            .with_metadata(
                MetadataKey::ItemCount,
                MetadataValue::Count(3),
                PrivacyClass::PublicMetadata,
            )
            .unwrap()
            .into_event(EventId::new(77).unwrap())
            .unwrap();
        assert_eq!(event.id().get(), 77);
        assert_eq!(event.occurred_at(), time(8));
        assert_eq!(event.action(), ActionKind::ObjectChanged);
        assert_eq!(event.transaction_id(), Some(TransactionId(14)));
        assert_eq!(event.correlation_id(), Some(CorrelationId(15)));
        assert_eq!(
            event.targets().collect::<std::vec::Vec<_>>(),
            [ObjectId(91)]
        );
        assert_eq!(
            event.metadata().next().unwrap().value(),
            MetadataValue::Count(3)
        );
        assert_eq!(EventId::new(0), Err(ActivityError::InvalidEventId));
        assert_eq!(EventId::new(u64::MAX), Err(ActivityError::InvalidEventId));
    }

    #[test]
    fn appends_immutable_identity_and_orders_ties_by_event_id() {
        let mut ledger = ActivityLedger::new();
        let later_id = ledger
            .append(draft(9, USER, ActionKind::ObjectChanged))
            .unwrap();
        let first_tie = ledger
            .append(draft(5, USER, ActionKind::ObjectCreated))
            .unwrap();
        let second_tie = ledger
            .append(draft(5, USER, ActionKind::ObjectDeleted))
            .unwrap();
        assert_eq!(later_id.get(), 1);
        assert_eq!(first_tie.get(), 2);
        assert_eq!(second_tie.get(), 3);
        let ordered: [u64; 3] = core::array::from_fn(|index| {
            ledger
                .query(ActivityQuery::default())
                .nth(index)
                .unwrap()
                .id()
                .get()
        });
        assert_eq!(ordered, [2, 3, 1]);
        assert_eq!(
            ledger.get(first_tie).unwrap().action(),
            ActionKind::ObjectCreated
        );
        assert_eq!(ledger.len(), 3);
    }

    #[test]
    fn validates_actor_classification_and_human_initiated_ai_provenance() {
        let mut ledger = ActivityLedger::new();
        assert_eq!(
            ledger.append(draft(1, AGENT, ActionKind::ObjectChanged)),
            Err(ActivityError::MissingDelegationProvenance)
        );
        let id = ledger
            .append(
                draft(1, AGENT, ActionKind::ObjectChanged)
                    .with_provenance(delegated())
                    .with_correlation(CorrelationId(41)),
            )
            .unwrap();
        let event = ledger.get(id).unwrap();
        assert_eq!(event.actor().kind(), ActorKind::Agent);
        assert_eq!(event.provenance(), delegated());
        assert_eq!(event.correlation_id(), Some(CorrelationId(41)));
    }

    #[test]
    fn failed_operations_and_transaction_grouping_are_preserved() {
        let mut ledger = ActivityLedger::new();
        let tx = TransactionId(7);
        for action in [ActionKind::ObjectChanged, ActionKind::ObjectMoved] {
            ledger
                .append(draft(2, USER, action).with_transaction(tx))
                .unwrap();
        }
        ledger
            .append(
                draft(3, USER, ActionKind::OperationFailed)
                    .with_transaction(tx)
                    .with_result(EventResult::Failed(FailureCode::BackendFailure)),
            )
            .unwrap();
        let events: [EventResult; 3] = core::array::from_fn(|index| {
            ledger
                .query(ActivityQuery::default().transaction(tx))
                .nth(index)
                .unwrap()
                .result()
        });
        assert_eq!(events[0], EventResult::Succeeded);
        assert_eq!(events[1], EventResult::Succeeded);
        assert_eq!(events[2], EventResult::Failed(FailureCode::BackendFailure));
    }

    #[test]
    fn redacts_sensitive_metadata_and_never_copies_payload_bytes() {
        let secret = b"api-key=never-persist-this";
        let draft = draft(1, USER, ActionKind::ObjectAccessed)
            .with_redacted_payload(MetadataKey::AccessToken, PrivacyClass::Secret, secret)
            .unwrap()
            .with_metadata(
                MetadataKey::ItemCount,
                MetadataValue::Count(3),
                PrivacyClass::PublicMetadata,
            )
            .unwrap()
            .with_metadata(
                MetadataKey::ByteCount,
                MetadataValue::Bytes(4096),
                PrivacyClass::Sensitive,
            )
            .unwrap();
        let mut ledger = ActivityLedger::new();
        let id = ledger.append(draft).unwrap();
        let metadata: [MetadataEntry; 3] =
            core::array::from_fn(|index| ledger.get(id).unwrap().metadata().nth(index).unwrap());
        assert_eq!(metadata[0].value(), MetadataValue::Redacted);
        assert_eq!(metadata[1].value(), MetadataValue::Count(3));
        assert_eq!(metadata[2].value(), MetadataValue::Redacted);
        let stored = std::format!("{:?}", ledger.get(id).unwrap());
        assert!(!stored.contains(core::str::from_utf8(secret).unwrap()));
    }

    #[test]
    fn query_filters_by_time_actor_action_object_workspace_transaction_and_correlation() {
        let mut ledger = ActivityLedger::new();
        let target = ObjectId(77);
        let tx = TransactionId(88);
        let correlation = CorrelationId(99);
        let action_group = ActionGroupId(67);
        let device = DeviceId(56);
        ledger
            .append(
                draft(3, USER, ActionKind::ObjectChanged)
                    .with_target(target)
                    .unwrap()
                    .with_action_group(action_group)
                    .with_transaction(tx)
                    .with_correlation(correlation)
                    .with_device(device)
                    .with_checkpoint_before(CheckpointId(101)),
            )
            .unwrap();
        ledger
            .append(
                draft(4, USER, ActionKind::ObjectChanged).with_provenance(Provenance::Direct {
                    originating_intent: Some(IntentId(23)),
                }),
            )
            .unwrap();
        let range = ActivityTimeRange::new(time(3), time(4)).unwrap();
        let filter = ActivityQuery::default()
            .time_range(range)
            .actor_id(USER.id())
            .actor_kind(ActorKind::User)
            .action(ActionKind::ObjectChanged)
            .target(target)
            .action_group(action_group)
            .device(device)
            .transaction(tx)
            .correlation(correlation)
            .checkpoint(CheckpointId(101));
        let results: std::vec::Vec<_> = ledger.query(filter).collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].occurred_at(), time(3));
    }

    struct OwnerOnly;

    impl ActivityAccessPolicy for OwnerOnly {
        fn can_read(&self, viewer: Actor, event: &ActivityEvent) -> bool {
            viewer.id() == event.actor.id() || event.actor.kind() == ActorKind::System
        }
    }

    #[test]
    fn visible_query_requires_and_applies_an_access_policy() {
        let mut ledger = ActivityLedger::new();
        ledger
            .append(draft(1, USER, ActionKind::ObjectCreated))
            .unwrap();
        ledger
            .append(draft(2, AGENT, ActionKind::ObjectChanged).with_provenance(delegated()))
            .unwrap();
        assert_eq!(ledger.query(ActivityQuery::default()).count(), 2);
        assert_eq!(
            ledger
                .query_visible(ActivityQuery::default(), USER, &OwnerOnly)
                .count(),
            1
        );
        assert_eq!(
            ledger
                .search(ActivityQuery::default(), USER, &OwnerOnly)
                .count(),
            1
        );
    }

    #[test]
    fn reversibility_is_explicit_and_localized_summary_is_not_persisted_text() {
        let undo = UndoDescriptor::new(
            InverseKind::RestoreObjectRevision,
            RevisionId(9),
            RevisionId(8),
        );
        let mut ledger = ActivityLedger::new();
        let reversible = ledger
            .append(
                draft(1, USER, ActionKind::ObjectChanged)
                    .with_reversibility(Reversibility::Reversible(undo)),
            )
            .unwrap();
        let irreversible = ledger
            .append(
                draft(2, USER, ActionKind::OperationFailed)
                    .with_reversibility(Reversibility::Irreversible),
            )
            .unwrap();
        assert_eq!(
            ledger.get(reversible).unwrap().reversibility(),
            Reversibility::Reversible(undo)
        );
        assert_eq!(
            ledger.get(irreversible).unwrap().reversibility(),
            Reversibility::Irreversible
        );
        let mut en = [0; 64];
        let en_len =
            render_summary(ledger.get(reversible).unwrap(), UserLocale::EnUs, &mut en).unwrap();
        let mut ja = [0; 64];
        let ja_len =
            render_summary(ledger.get(reversible).unwrap(), UserLocale::JaJp, &mut ja).unwrap();
        assert_eq!(&en[..en_len], b"Changed an item");
        assert_eq!(&ja[..ja_len], "項目を変更".as_bytes());
        assert_ne!(&en[..en_len], &ja[..ja_len]);
    }

    #[test]
    fn reversible_activity_resolves_a_wayback_destination() {
        let undo = UndoDescriptor::new(
            InverseKind::RestoreObjectRevision,
            RevisionId(9),
            RevisionId(8),
        );
        let mut ledger = ActivityLedger::new();
        let object_event = ledger
            .append(
                draft(1, USER, ActionKind::ObjectChanged)
                    .with_target(ObjectId(7))
                    .unwrap()
                    .with_reversibility(Reversibility::Reversible(undo)),
            )
            .unwrap();
        let checkpoint_event = ledger
            .append(
                draft(2, USER, ActionKind::ObjectChanged)
                    .with_target(ObjectId(7))
                    .unwrap()
                    .with_reversibility(Reversibility::Irreversible)
                    .with_checkpoint_before(CheckpointId(19)),
            )
            .unwrap();
        let irreversible_event = ledger
            .append(draft(3, USER, ActionKind::ObjectDeleted))
            .unwrap();
        assert_eq!(
            wayback_target(ledger.get(object_event).unwrap()),
            Some(WaybackTarget::ObjectRevision {
                object_id: ObjectId(7),
                revision_id: RevisionId(8),
            })
        );
        assert_eq!(
            wayback_target(ledger.get(checkpoint_event).unwrap()),
            Some(WaybackTarget::Checkpoint(CheckpointId(19)))
        );
        assert_eq!(
            wayback_target(ledger.get(irreversible_event).unwrap()),
            None
        );
    }

    #[test]
    fn target_open_checks_visibility_and_object_authority_before_adapter() {
        struct AllowTarget;
        impl ActivityTargetOpenPolicy for AllowTarget {
            fn authorize_open(
                &self,
                _viewer: Actor,
                _event: &ActivityEvent,
                _target: ObjectId,
            ) -> Result<(), FailureCode> {
                Ok(())
            }
        }
        struct DenyTarget;
        impl ActivityTargetOpenPolicy for DenyTarget {
            fn authorize_open(
                &self,
                _viewer: Actor,
                _event: &ActivityEvent,
                _target: ObjectId,
            ) -> Result<(), FailureCode> {
                Err(FailureCode::PermissionDenied)
            }
        }
        #[derive(Default)]
        struct RecordingOpener(Option<TargetOpenRequest>);
        impl ActivityTargetOpener for RecordingOpener {
            fn open(&mut self, request: TargetOpenRequest) -> Result<(), FailureCode> {
                self.0 = Some(request);
                Ok(())
            }
        }

        let mut ledger = ActivityLedger::new();
        let id = ledger
            .append(
                draft(1, USER, ActionKind::ObjectChanged)
                    .with_target(ObjectId(41))
                    .unwrap()
                    .with_correlation(CorrelationId(42)),
            )
            .unwrap();
        let mut opener = RecordingOpener::default();
        assert_eq!(
            open_event_target(&ledger, id, 0, USER, &OwnerOnly, &DenyTarget, &mut opener,),
            Err(TargetOpenError::Denied(FailureCode::PermissionDenied))
        );
        assert_eq!(opener.0, None);
        open_event_target(&ledger, id, 0, USER, &OwnerOnly, &AllowTarget, &mut opener).unwrap();
        let request = opener.0.unwrap();
        assert_eq!(request.event_id(), id);
        assert_eq!(request.target(), ObjectId(41));
        assert_eq!(request.correlation_id(), Some(CorrelationId(42)));
        assert_eq!(
            open_event_target(&ledger, id, 1, USER, &OwnerOnly, &AllowTarget, &mut opener,),
            Err(TargetOpenError::TargetUnavailable)
        );
        let hidden = ledger
            .append(
                draft(2, AGENT, ActionKind::ObjectChanged)
                    .with_target(ObjectId(99))
                    .unwrap()
                    .with_provenance(delegated()),
            )
            .unwrap();
        let previous_request = opener.0;
        assert_eq!(
            open_event_target(
                &ledger,
                hidden,
                0,
                USER,
                &OwnerOnly,
                &AllowTarget,
                &mut opener,
            ),
            Err(TargetOpenError::EventUnavailable)
        );
        assert_eq!(opener.0, previous_request);
    }

    #[test]
    fn bounded_ledger_fails_closed_when_full() {
        let mut ledger = ActivityLedger::new();
        for index in 0..MAX_ACTIVITY_EVENTS {
            ledger
                .append(draft(index as i64, USER, ActionKind::ObjectAccessed))
                .unwrap();
        }
        assert!(!ledger.can_append());
        assert_eq!(ledger.len(), MAX_ACTIVITY_EVENTS);
        assert_eq!(
            ledger.append(draft(500, USER, ActionKind::ObjectAccessed)),
            Err(ActivityError::Capacity)
        );
    }

    #[test]
    fn empty_state_is_localized_and_buffer_errors_are_explicit() {
        let mut en = [0; 32];
        let length = render_empty_state(UserLocale::EnUs, &mut en).unwrap();
        assert_eq!(&en[..length], b"No activity yet.");
        let mut ja = [0; 64];
        let length = render_empty_state(UserLocale::JaJp, &mut ja).unwrap();
        assert_eq!(&ja[..length], "履歴はまだありません。".as_bytes());
        assert_eq!(
            render_empty_state(UserLocale::EnUs, &mut [0; 2]),
            Err(RenderError::BufferTooSmall)
        );
    }
}
