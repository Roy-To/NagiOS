use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationError(pub &'static str);

impl std::fmt::Display for ValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for ValidationError {}

fn validate_token(value: &str, label: &'static str, max_len: usize) -> Result<(), ValidationError> {
    if value.is_empty() || value.len() > max_len || value.chars().any(char::is_control) {
        return Err(ValidationError(label));
    }
    Ok(())
}

fn validate_summary(value: &str) -> Result<(), ValidationError> {
    if value.trim().is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(ValidationError(
            "summary must be non-empty, bounded, and printable",
        ));
    }
    Ok(())
}

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = <String as Deserialize>::deserialize(deserializer)?;
                Self::new(value).map_err(<D::Error as serde::de::Error>::custom)
            }
        }

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ValidationError> {
                let value = value.into();
                validate_token(
                    &value,
                    "identifier must be non-empty, bounded, and printable",
                    160,
                )?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn validate(&self) -> Result<(), ValidationError> {
                validate_token(
                    &self.0,
                    "identifier must be non-empty, bounded, and printable",
                    160,
                )
            }
        }
    };
}

opaque_id!(EntryId);
opaque_id!(TransactionId);
opaque_id!(CorrelationId);
opaque_id!(SnapshotId);
opaque_id!(RestorePointId);

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrincipalReference {
    pub namespace: String,
    pub id: String,
}

impl PrincipalReference {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_token(&self.namespace, "principal namespace is invalid", 96)?;
        validate_token(&self.id, "principal id is invalid", 160)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    User,
    Ai,
    System,
    Application,
    Automation,
    RemoteDevice,
}

/// References authority/identity owned by other workstreams; it never grants it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorMetadata {
    pub kind: ActorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub principal: Option<PrincipalReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegated_for: Option<PrincipalReference>,
    /// Optional selected-model reference; never a place for prompts or chain-of-thought.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

impl ActorMetadata {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if let Some(principal) = &self.principal {
            principal.validate()?;
        }
        if let Some(principal) = &self.delegated_for {
            principal.validate()?;
        }
        if let Some(app_id) = &self.app_id {
            validate_token(app_id, "app id is invalid", 160)?;
        }
        if let Some(model_id) = &self.model_id {
            validate_token(model_id, "model id is invalid", 160)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionType {
    pub namespace: String,
    pub name: String,
}

impl ActionType {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_token(&self.namespace, "action namespace is invalid", 96)?;
        validate_token(&self.name, "action type is invalid", 128)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetReference {
    pub resource_type: String,
    /// Opaque object/resource identifier, never an authority-bearing host path.
    pub external_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
}

impl TargetReference {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_token(&self.resource_type, "target resource type is invalid", 96)?;
        validate_token(&self.external_id, "target reference is invalid", 256)?;
        if let Some(app_id) = &self.app_id {
            validate_token(app_id, "target app id is invalid", 160)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum HumanSummary {
    /// Callers attest that this bounded human-readable text is safe to retain.
    Public(String),
    Redacted {
        reason: String,
    },
    Omitted {
        reason: String,
    },
}

impl HumanSummary {
    pub fn public_text(&self) -> Option<&str> {
        match self {
            Self::Public(text) => Some(text),
            Self::Redacted { .. } | Self::Omitted { .. } => None,
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::Public(text) => validate_summary(text),
            Self::Redacted { reason } | Self::Omitted { reason } => validate_summary(reason),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldSensitivity {
    Public,
    Personal,
    Sensitive,
    Secret,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldDisposition {
    Recorded,
    Redacted,
    Omitted,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum MetadataValue {
    Text(String),
    Integer(i64),
    Boolean(bool),
    PrincipalReference(PrincipalReference),
    SnapshotReference(SnapshotId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataField {
    pub sensitivity: FieldSensitivity,
    pub disposition: FieldDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<MetadataValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl MetadataField {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match (self.disposition, self.value.as_ref()) {
            (FieldDisposition::Recorded, Some(value)) => {
                if matches!(
                    self.sensitivity,
                    FieldSensitivity::Sensitive | FieldSensitivity::Secret
                ) {
                    return Err(ValidationError(
                        "sensitive or secret metadata must be redacted, omitted, or referenced",
                    ));
                }
                if let MetadataValue::Text(text) = value {
                    validate_summary(text)?;
                }
                if let MetadataValue::PrincipalReference(principal) = value {
                    principal.validate()?;
                }
                if let MetadataValue::SnapshotReference(snapshot_id) = value {
                    snapshot_id.validate()?;
                }
                Ok(())
            }
            (FieldDisposition::Recorded, None) => {
                Err(ValidationError("recorded metadata requires a value"))
            }
            (_, Some(_)) => Err(ValidationError(
                "redacted, omitted, or unavailable metadata cannot contain a value",
            )),
            (_, None) => {
                if let Some(reason) = &self.reason {
                    validate_summary(reason)?;
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadAvailability {
    NotCaptured,
    Redacted,
    Omitted,
    Unavailable,
    SnapshotReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PayloadMetadata {
    pub availability: PayloadAvailability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<SnapshotId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PayloadMetadata {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.availability == PayloadAvailability::SnapshotReference && self.snapshot_id.is_none()
        {
            return Err(ValidationError(
                "snapshot-backed payload metadata requires a snapshot id",
            ));
        }
        if let Some(snapshot_id) = &self.snapshot_id {
            snapshot_id.validate()?;
        }
        if self.availability != PayloadAvailability::SnapshotReference && self.snapshot_id.is_some()
        {
            return Err(ValidationError(
                "only snapshot-backed payload metadata may carry a snapshot id",
            ));
        }
        if let Some(reason) = &self.reason {
            validate_summary(reason)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum Reversibility {
    ExactReversible,
    CompensationRequired { contract_id: String },
    SnapshotRequired { snapshot_id: Option<SnapshotId> },
    Irreversible { reason: String },
    Unknown { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReversibilityClass {
    ExactReversible,
    CompensationRequired,
    SnapshotRequired,
    Irreversible,
    Unknown,
}

impl Reversibility {
    pub fn class(&self) -> ReversibilityClass {
        match self {
            Self::ExactReversible => ReversibilityClass::ExactReversible,
            Self::CompensationRequired { .. } => ReversibilityClass::CompensationRequired,
            Self::SnapshotRequired { .. } => ReversibilityClass::SnapshotRequired,
            Self::Irreversible { .. } => ReversibilityClass::Irreversible,
            Self::Unknown { .. } => ReversibilityClass::Unknown,
        }
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        match self {
            Self::ExactReversible => Ok(()),
            Self::CompensationRequired { contract_id } => {
                validate_token(contract_id, "compensation contract id is invalid", 160)
            }
            Self::SnapshotRequired { snapshot_id } => {
                if let Some(snapshot_id) = snapshot_id {
                    snapshot_id.validate()?;
                }
                Ok(())
            }
            Self::Irreversible { reason } | Self::Unknown { reason } => validate_summary(reason),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiating_event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_entry_id: Option<EntryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause_entry_id: Option<EntryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<CorrelationId>,
}

impl Provenance {
    pub fn validate(&self, entry_id: &EntryId) -> Result<(), ValidationError> {
        entry_id.validate()?;
        if let Some(parent) = &self.parent_entry_id {
            parent.validate()?;
        }
        if let Some(cause) = &self.cause_entry_id {
            cause.validate()?;
        }
        if let Some(correlation) = &self.correlation_id {
            correlation.validate()?;
        }
        if self.parent_entry_id.as_ref() == Some(entry_id)
            || self.cause_entry_id.as_ref() == Some(entry_id)
        {
            return Err(ValidationError(
                "an entry cannot be its own parent or cause",
            ));
        }
        if let Some(event) = &self.initiating_event {
            validate_token(event, "initiating event reference is invalid", 256)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntryIntegrity {
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_entry_hash: Option<String>,
    pub entry_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityEntryDraft {
    pub schema_version: u32,
    pub id: EntryId,
    pub timestamp_ms: i64,
    pub actor: ActorMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    pub action_type: ActionType,
    pub target: TargetReference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<TransactionId>,
    pub provenance: Provenance,
    pub summary: HumanSummary,
    #[serde(default)]
    pub metadata: BTreeMap<String, MetadataField>,
    pub reversibility: Reversibility,
    #[serde(default)]
    pub snapshot_refs: Vec<SnapshotId>,
    #[serde(default)]
    pub restore_point_refs: Vec<RestorePointId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<PayloadMetadata>,
}

impl ActivityEntryDraft {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError("unsupported activity entry version"));
        }
        if self.timestamp_ms < 0 {
            return Err(ValidationError("timestamp must be Unix milliseconds >= 0"));
        }
        self.id.validate()?;
        if let Some(transaction_id) = &self.transaction_id {
            transaction_id.validate()?;
        }
        self.actor.validate()?;
        self.action_type.validate()?;
        self.target.validate()?;
        self.provenance.validate(&self.id)?;
        self.summary.validate()?;
        self.reversibility.validate()?;
        if let Some(app_id) = &self.app_id {
            validate_token(app_id, "entry app id is invalid", 160)?;
        }
        if let Reversibility::SnapshotRequired {
            snapshot_id: Some(snapshot_id),
        } = &self.reversibility
        {
            if !self.snapshot_refs.contains(snapshot_id) {
                return Err(ValidationError(
                    "required snapshot id must appear in snapshot references",
                ));
            }
        }
        let mut snapshot_ids = self.snapshot_refs.clone();
        snapshot_ids.sort();
        snapshot_ids.dedup();
        if snapshot_ids.len() != self.snapshot_refs.len() {
            return Err(ValidationError("duplicate snapshot reference"));
        }
        for snapshot_id in &self.snapshot_refs {
            snapshot_id.validate()?;
        }
        let mut point_ids = self.restore_point_refs.clone();
        point_ids.sort();
        point_ids.dedup();
        if point_ids.len() != self.restore_point_refs.len() {
            return Err(ValidationError("duplicate restore point reference"));
        }
        for point_id in &self.restore_point_refs {
            point_id.validate()?;
        }
        for (key, field) in &self.metadata {
            if key.is_empty()
                || key.len() > 128
                || !key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "._-".contains(c))
            {
                return Err(ValidationError(
                    "metadata key must be a bounded lowercase identifier",
                ));
            }
            field.validate()?;
        }
        if let Some(payload) = &self.payload {
            payload.validate()?;
            if let Some(snapshot_id) = &payload.snapshot_id {
                if !self.snapshot_refs.contains(snapshot_id) {
                    return Err(ValidationError(
                        "payload snapshot must appear in snapshot references",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActivityEntry {
    pub schema_version: u32,
    pub id: EntryId,
    pub timestamp_ms: i64,
    pub actor: ActorMetadata,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    pub action_type: ActionType,
    pub target: TargetReference,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<TransactionId>,
    pub provenance: Provenance,
    pub summary: HumanSummary,
    #[serde(default)]
    pub metadata: BTreeMap<String, MetadataField>,
    pub reversibility: Reversibility,
    #[serde(default)]
    pub snapshot_refs: Vec<SnapshotId>,
    #[serde(default)]
    pub restore_point_refs: Vec<RestorePointId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<PayloadMetadata>,
    pub integrity: EntryIntegrity,
}

impl ActivityEntry {
    pub fn draft(&self) -> ActivityEntryDraft {
        ActivityEntryDraft {
            schema_version: self.schema_version,
            id: self.id.clone(),
            timestamp_ms: self.timestamp_ms,
            actor: self.actor.clone(),
            app_id: self.app_id.clone(),
            action_type: self.action_type.clone(),
            target: self.target.clone(),
            transaction_id: self.transaction_id.clone(),
            provenance: self.provenance.clone(),
            summary: self.summary.clone(),
            metadata: self.metadata.clone(),
            reversibility: self.reversibility.clone(),
            snapshot_refs: self.snapshot_refs.clone(),
            restore_point_refs: self.restore_point_refs.clone(),
            payload: self.payload.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AtomicityExpectation {
    AtomicLocal,
    BestEffortPartial,
    ExternalCompensation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionDraft {
    pub schema_version: u32,
    pub id: TransactionId,
    pub started_at_ms: i64,
    pub actor: ActorMetadata,
    pub summary: HumanSummary,
    pub atomicity: AtomicityExpectation,
}

impl TransactionDraft {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError("unsupported transaction version"));
        }
        if self.started_at_ms < 0 {
            return Err(ValidationError("transaction timestamp is invalid"));
        }
        self.id.validate()?;
        self.actor.validate()?;
        self.summary.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionEndKind {
    Committed,
    Aborted,
    PartiallyApplied,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionFinished {
    pub schema_version: u32,
    pub id: TransactionId,
    pub finished_at_ms: i64,
    pub outcome: TransactionEndKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<HumanSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum TransactionState {
    Open,
    Committed {
        at_ms: i64,
    },
    Aborted {
        at_ms: i64,
        reason: Option<HumanSummary>,
    },
    PartiallyApplied {
        at_ms: i64,
        failure: HumanSummary,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub schema_version: u32,
    pub id: TransactionId,
    pub started_at_ms: i64,
    pub actor: ActorMetadata,
    pub summary: HumanSummary,
    pub atomicity: AtomicityExpectation,
    pub entry_ids: Vec<EntryId>,
    pub state: TransactionState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerBoundary {
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_hash: Option<String>,
}

impl LedgerBoundary {
    pub fn validate(&self) -> Result<(), ValidationError> {
        match (self.sequence, self.record_hash.as_deref()) {
            (0, None) => Ok(()),
            (0, Some(_)) | (_, None) => {
                Err(ValidationError("ledger boundary sequence/hash mismatch"))
            }
            (_, Some(hash)) if is_sha256_hex(hash) => Ok(()),
            _ => Err(ValidationError("ledger boundary digest is invalid")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeReference {
    pub scope_type: String,
    pub external_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
}

impl ScopeReference {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_token(&self.scope_type, "scope type is invalid", 96)?;
        validate_token(&self.external_id, "scope reference is invalid", 256)?;
        if let Some(app_id) = &self.app_id {
            validate_token(app_id, "scope app id is invalid", 160)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestorePointDraft {
    pub schema_version: u32,
    pub id: RestorePointId,
    pub timestamp_ms: i64,
    pub label: HumanSummary,
    pub snapshot_ids: Vec<SnapshotId>,
    pub ledger_boundary: LedgerBoundary,
    pub scopes: Vec<ScopeReference>,
    pub compatibility_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_transaction: Option<TransactionId>,
}

impl RestorePointDraft {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError("unsupported restore point version"));
        }
        if self.timestamp_ms < 0 || self.snapshot_ids.is_empty() || self.scopes.is_empty() {
            return Err(ValidationError(
                "restore point requires a valid timestamp, snapshots, and scopes",
            ));
        }
        self.id.validate()?;
        if let Some(transaction_id) = &self.source_transaction {
            transaction_id.validate()?;
        }
        self.label.validate()?;
        self.ledger_boundary.validate()?;
        validate_token(
            &self.compatibility_version,
            "restore point compatibility version is invalid",
            96,
        )?;
        let mut snapshots = self.snapshot_ids.clone();
        snapshots.sort();
        snapshots.dedup();
        if snapshots.len() != self.snapshot_ids.len() {
            return Err(ValidationError("restore point has duplicate snapshots"));
        }
        for snapshot_id in &self.snapshot_ids {
            snapshot_id.validate()?;
        }
        let mut scopes = self.scopes.clone();
        scopes.sort();
        scopes.dedup();
        if scopes.len() != self.scopes.len() {
            return Err(ValidationError("restore point has duplicate scopes"));
        }
        for scope in &self.scopes {
            scope.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotManifest {
    pub schema_version: u32,
    pub id: SnapshotId,
    pub scope: ScopeReference,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_transaction: Option<TransactionId>,
    pub source_ledger_boundary: LedgerBoundary,
    /// Digest/reference to content held by a separately owned backend; no content is stored here.
    pub content_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_count: Option<u64>,
    pub backend_id: String,
    pub compatibility_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_snapshot: Option<SnapshotId>,
}

impl SnapshotManifest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ValidationError("unsupported snapshot manifest version"));
        }
        if self.created_at_ms < 0 || !is_sha256_hex(&self.content_digest) {
            return Err(ValidationError(
                "snapshot timestamp or content digest is invalid",
            ));
        }
        self.id.validate()?;
        if let Some(transaction_id) = &self.source_transaction {
            transaction_id.validate()?;
        }
        if let Some(parent) = &self.parent_snapshot {
            parent.validate()?;
        }
        self.scope.validate()?;
        self.source_ledger_boundary.validate()?;
        validate_token(&self.backend_id, "snapshot backend id is invalid", 128)?;
        validate_token(
            &self.compatibility_version,
            "snapshot compatibility version is invalid",
            96,
        )?;
        if self.parent_snapshot.as_ref() == Some(&self.id) {
            return Err(ValidationError("snapshot cannot parent itself"));
        }
        Ok(())
    }
}

pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum LedgerPayload {
    ActivityEntry(Box<ActivityEntry>),
    TransactionStarted(TransactionDraft),
    TransactionFinished(TransactionFinished),
    RestorePointCreated(RestorePointDraft),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerRecord {
    pub schema_version: u32,
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_hash: Option<String>,
    pub payload: LedgerPayload,
    pub record_hash: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerCheckpoint {
    pub last_sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LedgerDocument {
    pub schema_version: u32,
    pub records: Vec<LedgerRecord>,
    /// Detects accidental truncation when the serialized record list is shortened.
    pub checkpoint: LedgerCheckpoint,
}
