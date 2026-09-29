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
    pub candidate_objects: Vec<ObjectId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedContext {
    caller: CallerIdentity,
    visible_objects: Vec<ObjectId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextError {
    TooManyObjects,
    DuplicateObject,
}

/// The context authority must come from a trusted service. It filters before
/// any context is supplied to a probabilistic provider.
pub trait ContextAuthority {
    fn can_read_object(&self, caller: CallerIdentity, object_id: ObjectId) -> bool;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ContextResolver;

impl ContextResolver {
    pub fn resolve(
        &self,
        request: ContextRequest,
        authority: &impl ContextAuthority,
    ) -> Result<ResolvedContext, ContextError> {
        if request.candidate_objects.len() > MAX_CONTEXT_OBJECTS {
            return Err(ContextError::TooManyObjects);
        }

        let mut seen = alloc::collections::BTreeSet::new();
        let mut visible_objects = Vec::new();
        for object_id in request.candidate_objects {
            if !seen.insert(object_id) {
                return Err(ContextError::DuplicateObject);
            }
            if authority.can_read_object(request.caller, object_id) {
                visible_objects.push(object_id);
            }
        }
        Ok(ResolvedContext {
            caller: request.caller,
            visible_objects,
        })
    }
}

impl ResolvedContext {
    pub fn caller(&self) -> CallerIdentity {
        self.caller
    }

    pub fn visible_objects(&self) -> &[ObjectId] {
        &self.visible_objects
    }

    pub fn contains_object(&self, object_id: ObjectId) -> bool {
        self.visible_objects.contains(&object_id)
    }
}
