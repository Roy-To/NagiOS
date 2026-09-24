use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;

use nagi_model::{
    AppId, AppSessionId, NodeId, ObjectId, SurfaceId, TransactionId, UserId, WorkspaceId,
};
use serde::{Deserialize, Serialize};

pub const ACTIVITY_EVENT_SCHEMA_VERSION: u16 = 1;
pub const MAX_SUMMARY_KEY_BYTES: usize = 128;
pub const MAX_METADATA_ITEMS: usize = 32;
pub const MAX_METADATA_KEY_BYTES: usize = 64;
pub const MAX_METADATA_TEXT_BYTES: usize = 256;
pub const MAX_ACTOR_MODEL_ID_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct Timestamp {
    pub unix_seconds: i64,
    pub nanoseconds: u32,
}

impl Timestamp {
    pub const fn from_unix_seconds(unix_seconds: i64) -> Self {
        Self {
            unix_seconds,
            nanoseconds: 0,
        }
    }

    pub const fn new(unix_seconds: i64, nanoseconds: u32) -> Option<Self> {
        if nanoseconds >= 1_000_000_000 {
            None
        } else {
            Some(Self {
                unix_seconds,
                nanoseconds,
            })
        }
    }

    pub const fn is_valid(self) -> bool {
        self.nanoseconds < 1_000_000_000
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct EventId(pub u128);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ActionId(pub u128);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct CorrelationId(pub u128);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct SnapshotId(pub u128);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct ActorId(pub u128);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct OpaqueId(pub u128);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct StorageObjectId(pub u128);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ActorKind {
    User,
    Ai,
    Application,
    System,
    Automation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ActorIdentity {
    User(UserId),
    Application(AppId),
    Opaque(ActorId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActorRef {
    pub kind: ActorKind,
    pub identity: Option<ActorIdentity>,
    /// The application that initiated or contains this activity, if known.
    pub application_id: Option<AppId>,
    /// AI provider/model label. This identifies provenance, not authority.
    pub model_id: Option<String>,
    /// AI delegated identity remains separate from `kind = Ai`.
    pub delegated_for: Option<UserId>,
}

impl ActorRef {
    pub fn user(user_id: UserId) -> Self {
        Self {
            kind: ActorKind::User,
            identity: Some(ActorIdentity::User(user_id)),
            application_id: None,
            model_id: None,
            delegated_for: None,
        }
    }

    pub fn ai(
        agent_id: u128,
        model_id: Option<String>,
        application_id: Option<AppId>,
        delegated_for: Option<UserId>,
    ) -> Self {
        Self {
            kind: ActorKind::Ai,
            identity: Some(ActorIdentity::Opaque(ActorId(agent_id))),
            application_id,
            model_id,
            delegated_for,
        }
    }

    pub fn application(app_id: AppId) -> Self {
        Self {
            kind: ActorKind::Application,
            identity: Some(ActorIdentity::Application(app_id)),
            application_id: Some(app_id),
            model_id: None,
            delegated_for: None,
        }
    }

    pub fn system(service_id: u128) -> Self {
        Self::opaque(ActorKind::System, service_id)
    }

    pub fn automation(automation_id: u128) -> Self {
        Self::opaque(ActorKind::Automation, automation_id)
    }

    fn opaque(kind: ActorKind, id: u128) -> Self {
        Self {
            kind,
            identity: Some(ActorIdentity::Opaque(ActorId(id))),
            application_id: None,
            model_id: None,
            delegated_for: None,
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        matches!(
            (self.kind, self.identity),
            (_, None)
                | (ActorKind::User, Some(ActorIdentity::User(_)))
                | (ActorKind::Ai, Some(ActorIdentity::Opaque(_)))
                | (ActorKind::Application, Some(ActorIdentity::Application(_)))
                | (ActorKind::System, Some(ActorIdentity::Opaque(_)))
                | (ActorKind::Automation, Some(ActorIdentity::Opaque(_)))
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RetentionClass {
    Standard,
    RecentDetailed,
    OlderSummary,
    UserPinned,
    Disposable,
}

impl Default for RetentionClass {
    fn default() -> Self {
        Self::Standard
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EventContext {
    pub timestamp: Timestamp,
    pub actor: ActorRef,
    pub origin: ActivityOrigin,
    pub transaction_id: Option<TransactionId>,
    pub parent_action_id: Option<ActionId>,
    pub correlation_id: Option<CorrelationId>,
    pub causation_id: Option<EventId>,
    pub retention: RetentionClass,
}

/// Logical Nagi execution/presentation context; these IDs are not process or
/// window IDs and do not grant authority.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActivityOrigin {
    pub node_id: Option<NodeId>,
    pub app_session_id: Option<AppSessionId>,
    pub surface_id: Option<SurfaceId>,
    pub workspace_id: Option<WorkspaceId>,
}

impl EventContext {
    pub fn new(timestamp: Timestamp, actor: ActorRef) -> Self {
        Self {
            timestamp,
            actor,
            origin: ActivityOrigin::default(),
            transaction_id: None,
            parent_action_id: None,
            correlation_id: None,
            causation_id: None,
            retention: RetentionClass::Standard,
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        self.timestamp.is_valid()
            && self.actor.is_valid()
            && self
                .actor
                .model_id
                .as_ref()
                .is_none_or(|value| value.len() <= MAX_ACTOR_MODEL_ID_BYTES)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ActionType {
    UserRequest,
    AssistantPlan,
    FileCreate,
    FileMove,
    FileRename,
    FileDelete,
    DocumentEdit,
    FileEdit,
    SettingChange,
    ApplicationStateChange,
    Restore,
    Custom(String),
}

impl ActionType {
    pub(crate) fn is_valid(&self) -> bool {
        match self {
            Self::Custom(value) => valid_key(value, MAX_SUMMARY_KEY_BYTES),
            _ => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum TargetKind {
    File,
    Directory,
    Document,
    ApplicationState,
    Setting,
    Workspace,
    DeviceState,
    SystemState,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum TargetId {
    Object(ObjectId),
    Opaque(OpaqueId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TargetRef {
    pub kind: TargetKind,
    pub id: TargetId,
    pub parent: Option<TargetId>,
}

impl TargetRef {
    pub const fn object(kind: TargetKind, id: ObjectId, parent: Option<TargetId>) -> Self {
        Self {
            kind,
            id: TargetId::Object(id),
            parent,
        }
    }

    pub const fn opaque(kind: TargetKind, id: OpaqueId, parent: Option<TargetId>) -> Self {
        Self {
            kind,
            id: TargetId::Opaque(id),
            parent,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MetadataValue {
    Boolean(bool),
    Integer(i64),
    Identifier(OpaqueId),
    Text(String),
}

pub type ActivityMetadata = BTreeMap<String, MetadataValue>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ContentHash(pub [u8; 32]);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ObjectVersion(pub u128);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StorageReference {
    /// Stable backend name, never a host path.
    pub backend_id: String,
    pub object_id: StorageObjectId,
    pub revision: Option<StorageObjectId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StateReference {
    pub content_hash: Option<ContentHash>,
    pub object_version: Option<ObjectVersion>,
    pub snapshot: Option<SnapshotId>,
    pub delta: Option<StorageReference>,
    pub metadata: ActivityMetadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ReversibilityKind {
    FullyReversible,
    ReversibleWithSnapshot,
    PartiallyReversible,
    Irreversible,
    Unknown,
}

impl ReversibilityKind {
    pub const fn is_reversible(self) -> bool {
        matches!(
            self,
            Self::FullyReversible | Self::ReversibleWithSnapshot | Self::PartiallyReversible
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Reversibility {
    pub kind: ReversibilityKind,
    pub reason_key: Option<String>,
}

impl Reversibility {
    pub fn new(kind: ReversibilityKind) -> Self {
        Self {
            kind,
            reason_key: None,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AuthorizationRef {
    pub capability_id: Option<OpaqueId>,
    pub decision_id: Option<OpaqueId>,
    pub delegated_authority_id: Option<OpaqueId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    pub request_action_id: Option<ActionId>,
    pub plan_id: Option<OpaqueId>,
    pub provider_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionDraft {
    pub action_type: ActionType,
    pub target: TargetRef,
    pub summary_key: String,
    pub metadata: ActivityMetadata,
    pub before: Option<StateReference>,
    pub after: Option<StateReference>,
    pub reversibility: Reversibility,
    pub related_snapshot: Option<SnapshotId>,
    pub authorization: Option<AuthorizationRef>,
    pub provenance: Provenance,
}

impl ActionDraft {
    pub fn simple(
        action_type: ActionType,
        target: TargetRef,
        summary_key: &str,
        reversibility: ReversibilityKind,
    ) -> Self {
        Self {
            action_type,
            target,
            summary_key: String::from(summary_key),
            metadata: ActivityMetadata::new(),
            before: None,
            after: None,
            reversibility: Reversibility::new(reversibility),
            related_snapshot: None,
            authorization: None,
            provenance: Provenance::default(),
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        valid_key(&self.summary_key, MAX_SUMMARY_KEY_BYTES)
            && self.action_type.is_valid()
            && self
                .reversibility
                .reason_key
                .as_ref()
                .is_none_or(|key| valid_key(key, MAX_SUMMARY_KEY_BYTES))
            && metadata_is_valid(&self.metadata)
            && self
                .before
                .as_ref()
                .is_none_or(|state| metadata_is_valid(&state.metadata))
            && self
                .after
                .as_ref()
                .is_none_or(|state| metadata_is_valid(&state.metadata))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActionData {
    pub action_id: ActionId,
    pub action_type: ActionType,
    pub target: TargetRef,
    pub summary_key: String,
    pub metadata: ActivityMetadata,
    pub before: Option<StateReference>,
    pub after: Option<StateReference>,
    pub reversibility: Reversibility,
    pub related_snapshot: Option<SnapshotId>,
    pub authorization: Option<AuthorizationRef>,
    pub provenance: Provenance,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionRecord {
    pub event_id: EventId,
    pub context: EventContext,
    pub action_id: ActionId,
    pub action_type: ActionType,
    pub target: TargetRef,
    pub summary_key: String,
    pub metadata: ActivityMetadata,
    pub before: Option<StateReference>,
    pub after: Option<StateReference>,
    pub reversibility: Reversibility,
    pub related_snapshot: Option<SnapshotId>,
    pub authorization: Option<AuthorizationRef>,
    pub provenance: Provenance,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TransactionStatus {
    InProgress,
    Completed,
    Failed,
    PartiallyCompleted,
    Reverted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ActionOutcome {
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RevertOutcome {
    Applied,
    PartiallyApplied,
    Failed,
    RejectedIrreversible,
    RejectedUnknown,
    RejectedMissingSnapshot,
    AlreadyReverted,
    AlreadyInProgress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevertExecutionResult {
    Applied,
    PartiallyApplied,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SnapshotScope {
    Target(TargetRef),
    Transaction(TransactionId),
    Workspace(WorkspaceId),
    System,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SnapshotReference {
    pub id: SnapshotId,
    pub created_at: Timestamp,
    pub scope: SnapshotScope,
    pub parent_snapshot: Option<SnapshotId>,
    pub reason_key: Option<String>,
    pub related_transaction: Option<TransactionId>,
    pub storage: StorageReference,
    pub retention: RetentionClass,
    pub pinned: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotDraft {
    pub scope: SnapshotScope,
    pub parent_snapshot: Option<SnapshotId>,
    pub reason_key: Option<String>,
    pub related_transaction: Option<TransactionId>,
    pub storage: StorageReference,
    pub retention: RetentionClass,
    pub pinned: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ActivityEventKind {
    TransactionStarted {
        transaction_id: TransactionId,
        reason_key: Option<String>,
    },
    ActionCreated(Box<ActionData>),
    ActionOutcomeRecorded {
        action_id: ActionId,
        outcome: ActionOutcome,
        result_key: Option<String>,
    },
    ActionRevertRequested {
        action_id: ActionId,
    },
    TransactionStatusChanged {
        transaction_id: TransactionId,
        status: TransactionStatus,
        result_key: Option<String>,
    },
    ActionRevertRecorded {
        action_id: ActionId,
        outcome: RevertOutcome,
        reason_key: Option<String>,
    },
    SnapshotReferenceCreated(Box<SnapshotReference>),
    RedactionRecorded {
        target_event_id: EventId,
        reason_key: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActivityEvent {
    pub schema_version: u16,
    pub event_id: EventId,
    pub context: EventContext,
    pub kind: ActivityEventKind,
}

impl ActivityEvent {
    pub fn new(event_id: EventId, context: EventContext, kind: ActivityEventKind) -> Self {
        Self {
            schema_version: ACTIVITY_EVENT_SCHEMA_VERSION,
            event_id,
            context,
            kind,
        }
    }
}

pub(crate) fn metadata_is_valid(metadata: &ActivityMetadata) -> bool {
    metadata.len() <= MAX_METADATA_ITEMS
        && metadata.iter().all(|(key, value)| {
            valid_key(key, MAX_METADATA_KEY_BYTES)
                && match value {
                    MetadataValue::Text(text) => text.len() <= MAX_METADATA_TEXT_BYTES,
                    MetadataValue::Boolean(_)
                    | MetadataValue::Integer(_)
                    | MetadataValue::Identifier(_) => true,
                }
        })
}

pub(crate) fn valid_key(key: &str, max_bytes: usize) -> bool {
    !key.is_empty()
        && key.len() <= max_bytes
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
