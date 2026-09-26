use std::fmt;

use nagi_model::{AppSessionId, ObjectId, WorkspaceId};

use crate::domain::Timestamp;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityKind {
    Created,
    Opened,
    Edited,
    Saved,
    Renamed,
    Deleted,
    Restored,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActivityOrigin {
    User,
    Agent(AppSessionId),
    Mixed,
}

/// Activity payloads identify the semantic operation and affected object.
/// They intentionally exclude note titles and body text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActivityEvent {
    pub object_id: ObjectId,
    pub kind: ActivityKind,
    pub origin: ActivityOrigin,
    pub revision: u64,
    pub workspace_id: Option<WorkspaceId>,
    pub occurred_at: Timestamp,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityError(pub String);

impl fmt::Display for ActivityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ActivityError {}

pub trait ActivitySink: Send + Sync {
    fn record(&self, event: ActivityEvent) -> Result<(), ActivityError>;
}

#[derive(Default)]
pub struct NoopActivitySink;

impl ActivitySink for NoopActivitySink {
    fn record(&self, _event: ActivityEvent) -> Result<(), ActivityError> {
        Ok(())
    }
}
