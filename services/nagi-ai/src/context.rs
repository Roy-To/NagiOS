use alloc::vec::Vec;

use nagi_model::{AppId, AppSessionId, NodeId, ObjectId, WorkspaceId};

pub const MAX_CONTEXT_OBJECTS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CallerIdentity {
    pub app_id: AppId,
    pub app_session_id: AppSessionId,
    /// Logical origin metadata. NodeId is never an authority grant.
    pub node_id: NodeId,
    pub workspace_id: Option<WorkspaceId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextRequest {
    pub caller: CallerIdentity,
    /// The user's current selection, if any. It is independently checked
    /// against the same trusted visibility authority as every other object.
    pub selected_object: Option<ObjectId>,
    pub candidate_objects: Vec<ObjectId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedContext {
    caller: CallerIdentity,
    selected_object: Option<ObjectId>,
    visible_objects: Vec<ObjectId>,
    browser_page: Option<crate::UntrustedBrowserContext>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextError {
    TooManyObjects,
    DuplicateObject,
    SelectedObjectNotVisible,
    WorkspaceNotVisible,
}

/// The context authority must come from a trusted service. It filters before
/// any context is supplied to a probabilistic provider.
pub trait ContextAuthority {
    fn can_read_object(&self, caller: CallerIdentity, object_id: ObjectId) -> bool;

    /// Workspace IDs are context only after the trusted authority confirms
    /// the caller may disclose that workspace to an AI provider.
    fn can_read_workspace(&self, _caller: CallerIdentity, _workspace_id: WorkspaceId) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ContextResolver;

impl ContextResolver {
    pub fn resolve(
        &self,
        request: ContextRequest,
        authority: &impl ContextAuthority,
    ) -> Result<ResolvedContext, ContextError> {
        if request
            .caller
            .workspace_id
            .is_some_and(|workspace_id| !authority.can_read_workspace(request.caller, workspace_id))
        {
            return Err(ContextError::WorkspaceNotVisible);
        }
        let mut candidates = request.candidate_objects;
        if let Some(selected_object) = request.selected_object {
            if !candidates.contains(&selected_object) {
                candidates.push(selected_object);
            }
        }
        if candidates.len() > MAX_CONTEXT_OBJECTS {
            return Err(ContextError::TooManyObjects);
        }

        let mut seen = alloc::collections::BTreeSet::new();
        let mut visible_objects = Vec::new();
        for object_id in candidates {
            if !seen.insert(object_id) {
                return Err(ContextError::DuplicateObject);
            }
            if authority.can_read_object(request.caller, object_id) {
                visible_objects.push(object_id);
            }
        }
        if request
            .selected_object
            .is_some_and(|selected| !visible_objects.contains(&selected))
        {
            return Err(ContextError::SelectedObjectNotVisible);
        }
        Ok(ResolvedContext {
            caller: request.caller,
            selected_object: request.selected_object,
            visible_objects,
            browser_page: None,
        })
    }
}

impl ResolvedContext {
    pub(crate) fn with_browser_page(mut self, page: crate::UntrustedBrowserContext) -> Self {
        self.browser_page = Some(page);
        self
    }
}

impl ResolvedContext {
    pub fn caller(&self) -> CallerIdentity {
        self.caller
    }

    pub fn visible_objects(&self) -> &[ObjectId] {
        &self.visible_objects
    }

    pub fn selected_object(&self) -> Option<ObjectId> {
        self.selected_object
    }

    /// Page data is untrusted text. It is not an Object ID, capability, or
    /// authority grant and must be treated as user-supplied content by AI.
    pub fn browser_page(&self) -> Option<&crate::UntrustedBrowserContext> {
        self.browser_page.as_ref()
    }

    pub fn contains_object(&self, object_id: ObjectId) -> bool {
        self.visible_objects.contains(&object_id)
    }
}
