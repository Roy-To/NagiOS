//! Version 1 host seams. Implementations must enforce platform permission and
//! stale-revision checks; traits grant no authority and have no default success.
use crate::*;
pub const CONTRACT_VERSION: u16 = 1;
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Access {
    Read,
    Edit,
    Export,
    RecordActivity,
    Checkpoint,
}
pub trait PermissionBoundary {
    fn authorize(&self, actor: Actor, document: DocumentId, access: Access) -> Result<(), Error>;
}
pub trait DocumentStore {
    fn create(&mut self, document: &Document, actor: Actor) -> Result<(), Error>;
    fn open(&self, id: DocumentId, actor: Actor) -> Result<Document, Error>;
    fn save(
        &mut self,
        expected: RevisionId,
        document: &Document,
        actor: Actor,
    ) -> Result<(), Error>;
}
/// Content-free handoff; OS Activity never receives comment/text payloads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivityRecord {
    pub document: DocumentId,
    pub from: RevisionId,
    pub to: RevisionId,
    pub actor: Actor,
    pub time: u64,
    pub changes: Vec<(ChangeKind, Option<ObjectId>)>,
}
pub fn activity_record(changes: &ChangeSet) -> ActivityRecord {
    ActivityRecord {
        document: changes.document,
        from: changes.from,
        to: changes.to,
        actor: changes.provenance.actor,
        time: changes.provenance.time,
        changes: changes.changes.iter().map(|c| (c.kind, c.object)).collect(),
    }
}
pub trait ActivitySink {
    fn record(&mut self, record: &ActivityRecord) -> Result<(), Error>;
}
pub trait CheckpointAdapter {
    fn checkpoint(
        &mut self,
        document: &Document,
        actor: Actor,
    ) -> Result<nagi_history::activity::CheckpointId, Error>;
}
pub struct Unavailable;
impl ActivitySink for Unavailable {
    fn record(&mut self, _: &ActivityRecord) -> Result<(), Error> {
        Err(Error::Unavailable)
    }
}
impl CheckpointAdapter for Unavailable {
    fn checkpoint(
        &mut self,
        _: &Document,
        _: Actor,
    ) -> Result<nagi_history::activity::CheckpointId, Error> {
        Err(Error::Unavailable)
    }
}
impl DocumentStore for Unavailable {
    fn create(&mut self, _: &Document, _: Actor) -> Result<(), Error> {
        Err(Error::Unavailable)
    }
    fn open(&self, _: DocumentId, _: Actor) -> Result<Document, Error> {
        Err(Error::Unavailable)
    }
    fn save(&mut self, _: RevisionId, _: &Document, _: Actor) -> Result<(), Error> {
        Err(Error::Unavailable)
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexItem {
    pub document: DocumentId,
    pub revision: RevisionId,
    pub object: ObjectId,
    pub text: String,
}
/// Authorized projection only. Private comments/references are excluded.
pub fn index_items(
    document: &Document,
    actor: Actor,
    permission: &dyn PermissionBoundary,
) -> Result<Vec<IndexItem>, Error> {
    permission.authorize(actor, document.id, Access::Read)?;
    let mut items = vec![IndexItem {
        document: document.id,
        revision: document.revision,
        object: document.id.0,
        text: document.metadata.title.clone(),
    }];
    for b in document.sections.iter().flat_map(|s| &s.blocks) {
        if matches!(b.kind, BlockKind::Heading { .. } | BlockKind::Paragraph(_)) {
            items.push(IndexItem {
                document: document.id,
                revision: document.revision,
                object: b.id,
                text: b.kind.text().unwrap_or_default().into(),
            });
        }
    }
    Ok(items)
}
