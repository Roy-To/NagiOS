use alloc::{collections::BTreeMap, string::String, vec::Vec};

use nagi_model::{AppId, AppSessionId, ObjectId, WorkspaceId};

use crate::{MetadataRecord, ModelError, ObjectKind, VisibilityScope, Workspace};

/// Narrow producer payload shared by Files and page/history adapters. Its
/// location is descriptive input; stable `ObjectId` remains the key.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProducerObject {
    pub object_id: ObjectId,
    pub title: String,
    pub location: Option<String>,
    pub source_app: Option<AppId>,
    pub source_session: Option<AppSessionId>,
    pub created_at: Option<i64>,
    pub modified_at: Option<i64>,
    pub observed_at: Option<i64>,
    pub tags: Vec<String>,
    pub attributes: BTreeMap<String, String>,
    pub visibility: VisibilityScope,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FilesProducerAdapter;

impl FilesProducerAdapter {
    pub fn to_record(&self, input: ProducerObject) -> Result<MetadataRecord, ModelError> {
        map_object(input, ObjectKind::File)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PageProducerAdapter;

impl PageProducerAdapter {
    pub fn to_record(&self, input: ProducerObject) -> Result<MetadataRecord, ModelError> {
        map_object(input, ObjectKind::Page)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct WorkspaceProducerAdapter;

impl WorkspaceProducerAdapter {
    pub fn create(
        &self,
        workspace_id: WorkspaceId,
        title: impl Into<String>,
        owner_app: Option<AppId>,
        visibility: VisibilityScope,
    ) -> Result<Workspace, ModelError> {
        let mut workspace = Workspace::new(workspace_id, title);
        workspace.owner_app = owner_app;
        workspace.visibility = visibility;
        workspace.validate()?;
        Ok(workspace)
    }
}

fn map_object(input: ProducerObject, kind: ObjectKind) -> Result<MetadataRecord, ModelError> {
    let mut record = MetadataRecord::new(input.object_id, kind, input.title);
    record.location = input.location;
    record.source_app = input.source_app;
    record.source_session = input.source_session;
    record.created_at = input.created_at;
    record.modified_at = input.modified_at;
    record.observed_at = input.observed_at;
    record.tags = input.tags;
    record.attributes = input.attributes;
    record.visibility = input.visibility;
    record.validate()?;
    Ok(record)
}
