use std::collections::BTreeSet;

use nagi_history::activity::{CheckpointId, EventId};
use nagi_model::{AppId, ObjectId, WorkspaceId};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct CapabilityId(String);

impl CapabilityId {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 96
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'-' | b'_')
            })
        {
            return Err("capability identifiers must be lowercase ASCII tokens");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The caller supplies only capabilities already granted by the platform.
/// An empty context denies every protected provider, result, and action.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapabilityContext {
    granted: BTreeSet<CapabilityId>,
}

impl CapabilityContext {
    /// Build the UI's capability projection from the authenticated caller's
    /// visible capability set. This is a presentation filter only; providers
    /// and OS services must still enforce their own authorization boundaries.
    pub fn from_visible_grants(grants: impl IntoIterator<Item = CapabilityId>) -> Self {
        Self {
            granted: grants.into_iter().collect(),
        }
    }

    pub(crate) fn from_grants(grants: impl IntoIterator<Item = CapabilityId>) -> Self {
        Self::from_visible_grants(grants)
    }

    pub fn allows(&self, capability: Option<&CapabilityId>) -> bool {
        capability.is_none_or(|capability| self.granted.contains(capability))
    }

    pub fn grants(&self) -> impl Iterator<Item = &CapabilityId> {
        self.granted.iter()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TypedAction {
    LaunchApp { app_id: AppId },
    OpenObject { object_id: ObjectId, app_id: AppId },
    OpenWorkspace { workspace_id: WorkspaceId },
    OpenActivityEvent { event_id: EventId },
    OpenCheckpoint { checkpoint_id: CheckpointId },
    InvokeAction { action_id: String },
    OpenSearch,
    OpenIntentEntry,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionAvailability {
    Ready,
    HostPreviewOnly,
    ComingSoon { reason_key: String },
    Unavailable { reason_key: String },
    PermissionRequired { capability: CapabilityId },
    Unlaunchable { reason_key: String },
}

impl ActionAvailability {
    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Ready | Self::HostPreviewOnly)
    }
}

#[cfg(test)]
mod tests {
    use super::{CapabilityContext, CapabilityId};

    #[test]
    fn empty_capability_context_denies_protected_access() {
        let files_read = CapabilityId::new("files.metadata.read").unwrap();
        let context = CapabilityContext::default();
        assert!(!context.allows(Some(&files_read)));
        assert!(context.allows(None));
    }

    #[test]
    fn capability_identifiers_are_stable_internal_ascii_tokens() {
        assert!(CapabilityId::new("search.provider").is_ok());
        assert!(CapabilityId::new("Search.Provider").is_err());
    }
}
