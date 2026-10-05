use alloc::{format, string::ToString, vec, vec::Vec};

use nagi_model::ObjectId;
use nagi_model_manager::CapabilityId;
use nagi_search::{
    AccessContext, ObjectKind, SearchQuery, SearchService, SnapshotBackend, VisibilityFilter,
};

use crate::{
    ActionDescriptor, ActionHandler, ActionInvocation, ActionOutput, ActionPolicy, ActionRegistry,
    HandlerError, ObjectAccess, ParameterKind, ParameterRule, RegistryError,
    MAX_ACTION_STRING_BYTES, MAX_OUTPUT_OBJECTS,
};

pub const FILE_SEARCH_ACTION_ID: &str = "file.search";
pub const FILE_SEARCH_CAPABILITY_ID: &str = "files.search";
pub const FILE_SEARCH_QUERY_MAX_BYTES: usize = 128;

/// Register the deterministic Search Service as the `file.search` action.
/// The caller must provide a SearchService with a trusted visibility filter;
/// this adapter forwards only its authorized Object IDs to the plan result.
pub fn register_file_search_action<P, B, V>(
    registry: &mut ActionRegistry<P>,
    service: SearchService<B, V>,
) -> Result<(), RegistryError>
where
    P: ActionPolicy,
    B: SnapshotBackend + 'static,
    V: VisibilityFilter + 'static,
{
    let capability = CapabilityId::new(FILE_SEARCH_CAPABILITY_ID)
        .expect("static file search capability is valid");
    let descriptor = ActionDescriptor::new(
        FILE_SEARCH_ACTION_ID,
        vec![capability],
        ObjectAccess::Read,
        0,
        0,
        vec![ParameterRule::new(
            "query",
            ParameterKind::String {
                max_bytes: FILE_SEARCH_QUERY_MAX_BYTES.min(MAX_ACTION_STRING_BYTES),
            },
            true,
        )],
    )?;
    registry.register(descriptor, FileSearchAction { service })
}

struct FileSearchAction<B, V> {
    service: SearchService<B, V>,
}

impl<P, B, V> ActionHandler<P> for FileSearchAction<B, V>
where
    P: ActionPolicy,
    B: SnapshotBackend,
    V: VisibilityFilter,
{
    fn execute(
        &mut self,
        invocation: ActionInvocation<'_, P>,
    ) -> Result<ActionOutput, HandlerError> {
        let query_text = invocation
            .parameters()
            .get("query")
            .and_then(serde_json::Value::as_str)
            .ok_or(HandlerError::Failed)?;
        let caller = invocation.caller();
        let query = SearchQuery {
            text: Some(query_text.to_string()),
            kind: Some(ObjectKind::File),
            limit: MAX_OUTPUT_OBJECTS,
            ..SearchQuery::default()
        };
        let response = self
            .service
            .search(
                AccessContext::for_application(caller.app_id, caller.app_session_id),
                &query,
            )
            .map_err(|error| match error {
                nagi_search::SearchError::Storage => HandlerError::Unavailable,
                _ => HandlerError::Failed,
            })?;
        let object_ids: Vec<ObjectId> = response
            .objects
            .into_iter()
            .map(|hit| hit.record.object_id)
            .collect();
        Ok(ActionOutput {
            summary: format!("Found {} visible object(s).", object_ids.len()),
            object_ids,
        })
    }
}
