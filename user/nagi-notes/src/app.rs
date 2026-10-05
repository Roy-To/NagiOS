use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use nagi_model::ObjectId;

use crate::activity::{
    ActivityEvent, ActivityKind, ActivityOrigin, ActivitySink, NoopActivitySink,
};
use crate::domain::{AlbertReference, Block, BlockKind, NoteDocument, NoteSummary};
use crate::identity::{HostObjectIdSource, ObjectIdSource};
use crate::markdown::{export_markdown, import_markdown};
use crate::references::valid_web_url;
use crate::session::{Clock, NoteSession, SessionError, SystemClock};
use crate::store::{NoteStore, StoreError};

pub type OpenDocument = Arc<NoteSession>;

pub struct NotesApp {
    store: Arc<dyn NoteStore>,
    ids: Arc<dyn ObjectIdSource>,
    clock: Arc<dyn Clock>,
    activity: Arc<dyn ActivitySink>,
    debounce: Duration,
    open_documents: Mutex<OpenDocuments>,
    activity_failures: AtomicU64,
}

#[derive(Default)]
struct OpenDocuments {
    sessions: HashMap<u64, OpenDocument>,
    busy: HashSet<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppError {
    Store(StoreError),
    Session(SessionError),
    NotFound(ObjectId),
    Deleted(ObjectId),
    DocumentBusy(ObjectId),
    IdAllocationFailed,
    InvalidWebReference,
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => error.fmt(f),
            Self::Session(error) => error.fmt(f),
            Self::NotFound(id) => write!(f, "note {:016x} was not found", id.0),
            Self::Deleted(id) => write!(f, "note {:016x} is in the trash", id.0),
            Self::DocumentBusy(id) => {
                write!(f, "note {:016x} is closing or changing state", id.0)
            }
            Self::IdAllocationFailed => f.write_str("could not allocate a unique ObjectId"),
            Self::InvalidWebReference => {
                f.write_str("web references must use an http or https URL")
            }
        }
    }
}

impl std::error::Error for AppError {}

impl From<StoreError> for AppError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<SessionError> for AppError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

impl NotesApp {
    pub fn new(store: Arc<dyn NoteStore>) -> Self {
        Self::with_providers(
            store,
            Arc::new(HostObjectIdSource::new()),
            Arc::new(SystemClock),
            Arc::new(NoopActivitySink),
            Duration::from_millis(700),
        )
    }

    pub fn with_providers(
        store: Arc<dyn NoteStore>,
        ids: Arc<dyn ObjectIdSource>,
        clock: Arc<dyn Clock>,
        activity: Arc<dyn ActivitySink>,
        debounce: Duration,
    ) -> Self {
        Self {
            store,
            ids,
            clock,
            activity,
            debounce,
            open_documents: Mutex::new(OpenDocuments::default()),
            activity_failures: AtomicU64::new(0),
        }
    }

    pub fn list_notes(&self) -> Result<Vec<NoteSummary>, AppError> {
        let mut notes = self.store.list(false)?;
        let mut indexes = notes
            .iter()
            .enumerate()
            .map(|(index, note)| (note.id.0, index))
            .collect::<HashMap<_, _>>();
        for session in lock(&self.open_documents).sessions.values() {
            let note = session.note();
            if note.deleted || session.is_closed() {
                continue;
            }
            let summary = NoteSummary {
                id: note.id,
                title: note.title,
                updated_at: note.updated_at,
                revision: note.revision,
                workspace_id: note.workspace_id,
                workspace_ids: note.workspace_ids,
                tags: note.tags,
                deleted: note.deleted,
            };
            if let Some(index) = indexes.get(&summary.id.0).copied() {
                notes[index] = summary;
            } else {
                indexes.insert(summary.id.0, notes.len());
                notes.push(summary);
            }
        }
        notes.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        Ok(notes)
    }

    pub fn list_trash(&self) -> Result<Vec<NoteSummary>, AppError> {
        Ok(self
            .store
            .list(true)?
            .into_iter()
            .filter(|summary| summary.deleted)
            .collect())
    }

    pub fn get_note(&self, id: ObjectId) -> Result<NoteDocument, AppError> {
        let open = lock(&self.open_documents);
        if open.busy.contains(&id.0) {
            return Err(AppError::DocumentBusy(id));
        }
        if let Some(session) = open.sessions.get(&id.0).cloned() {
            if !session.is_closed() {
                return Ok(session.note());
            }
        }
        drop(open);
        self.store.load(id)?.ok_or(AppError::NotFound(id))
    }

    pub fn create_note(&self, title: impl Into<String>) -> Result<OpenDocument, AppError> {
        self.create_note_by(ActivityOrigin::User, title)
    }

    /// Create a note through the lightweight capture path used by Quick Note.
    /// The caller supplies the localized title and active workspace context;
    /// captured text is stored as an ordinary editable paragraph block.
    pub fn create_quick_note(
        &self,
        title: impl Into<String>,
        text: impl Into<String>,
        workspace_id: Option<crate::domain::WorkspaceId>,
    ) -> Result<OpenDocument, AppError> {
        let session = self.create_note(title)?;
        let text = text.into();
        if !text.is_empty() {
            session.append_block(Block::new(
                self.ids.next_object_id(),
                BlockKind::Paragraph(text),
            ))?;
        }
        if workspace_id.is_some() {
            session.set_workspace(workspace_id)?;
        }
        Ok(session)
    }

    pub fn create_note_by(
        &self,
        origin: ActivityOrigin,
        title: impl Into<String>,
    ) -> Result<OpenDocument, AppError> {
        let title = title.into();
        let mut open = lock(&self.open_documents);
        let mut selected = None;
        for _ in 0..128 {
            let id = self.ids.next_object_id();
            if !open.sessions.contains_key(&id.0)
                && !open.busy.contains(&id.0)
                && self.store.load(id)?.is_none()
            {
                selected = Some(id);
                break;
            }
        }
        let id = selected.ok_or(AppError::IdAllocationFailed)?;
        let note = NoteDocument::new(id, title, self.clock.now());
        let session = NoteSession::start(
            note,
            Arc::clone(&self.store),
            Arc::clone(&self.clock),
            Arc::clone(&self.activity),
            self.debounce,
            true,
            origin,
        )?;
        open.sessions.insert(id.0, Arc::clone(&session));
        Ok(session)
    }

    pub fn open_note(&self, id: ObjectId) -> Result<OpenDocument, AppError> {
        self.open_note_by(ActivityOrigin::User, id)
    }

    pub fn open_note_by(
        &self,
        origin: ActivityOrigin,
        id: ObjectId,
    ) -> Result<OpenDocument, AppError> {
        let mut open = lock(&self.open_documents);
        if open.busy.contains(&id.0) {
            return Err(AppError::DocumentBusy(id));
        }
        if let Some(session) = open.sessions.get(&id.0) {
            if !session.is_closed() {
                return Ok(Arc::clone(session));
            }
        }
        let note = self.store.load(id)?.ok_or(AppError::NotFound(id))?;
        if note.deleted {
            return Err(AppError::Deleted(id));
        }
        let session = NoteSession::start(
            note.clone(),
            Arc::clone(&self.store),
            Arc::clone(&self.clock),
            Arc::clone(&self.activity),
            self.debounce,
            false,
            origin,
        )?;
        open.sessions.insert(id.0, Arc::clone(&session));
        drop(open);
        self.record_activity(ActivityEvent {
            object_id: id,
            kind: ActivityKind::Opened,
            origin,
            revision: note.revision,
            workspace_id: note.workspace_id,
            occurred_at: self.clock.now(),
        });
        Ok(session)
    }

    pub fn close_note(&self, id: ObjectId) -> Result<NoteDocument, AppError> {
        let Some(session) = self.reserve_document(id)? else {
            self.finish_document_operation(id, false, false);
            return Err(AppError::NotFound(id));
        };
        let result = session.close().map_err(AppError::from);
        self.finish_document_operation(id, result.is_ok(), false);
        result
    }

    pub fn delete_note(&self, id: ObjectId) -> Result<NoteDocument, AppError> {
        let session = self.reserve_document(id)?;
        if let Some(session) = session {
            if let Err(error) = session.close() {
                self.finish_document_operation(id, false, false);
                return Err(error.into());
            }
        }
        self.finish_document_operation(id, true, true);
        let result = (|| {
            let mut note = self.store.load(id)?.ok_or(AppError::NotFound(id))?;
            if note.deleted {
                return Err(AppError::Deleted(id));
            }
            let expected = note.revision;
            note.deleted = true;
            note.updated_at = self.clock.now();
            self.store.commit(&note, expected).map_err(AppError::from)
        })();
        self.finish_document_operation(id, false, false);
        let saved = result?;
        self.record_activity(ActivityEvent {
            object_id: id,
            kind: ActivityKind::Deleted,
            origin: ActivityOrigin::User,
            revision: saved.revision,
            workspace_id: saved.workspace_id,
            occurred_at: saved.updated_at,
        });
        Ok(saved)
    }

    pub fn restore_note(&self, id: ObjectId) -> Result<NoteDocument, AppError> {
        let session = self.reserve_document(id)?;
        if let Some(session) = &session {
            if !session.is_closed() {
                let note = session.note();
                self.finish_document_operation(id, false, false);
                return Ok(note);
            }
        }
        let result = (|| {
            let mut note = self.store.load(id)?.ok_or(AppError::NotFound(id))?;
            if !note.deleted {
                return Ok((note, false));
            }
            let expected = note.revision;
            note.deleted = false;
            note.updated_at = self.clock.now();
            self.store
                .commit(&note, expected)
                .map(|saved| (saved, true))
                .map_err(AppError::from)
        })();
        self.finish_document_operation(id, true, false);
        let (saved, was_deleted) = result?;
        if was_deleted {
            self.record_activity(ActivityEvent {
                object_id: id,
                kind: ActivityKind::Restored,
                origin: ActivityOrigin::User,
                revision: saved.revision,
                workspace_id: saved.workspace_id,
                occurred_at: saved.updated_at,
            });
        }
        Ok(saved)
    }

    /// Restore is itself a new revision, so a later restore can return to the
    /// pre-restore content through the same revision provider.
    pub fn restore_revision(&self, id: ObjectId, revision: u64) -> Result<NoteDocument, AppError> {
        let session = self.reserve_document(id)?;
        if let Some(session) = session {
            if let Err(error) = session.close() {
                self.finish_document_operation(id, false, false);
                return Err(error.into());
            }
        }
        self.finish_document_operation(id, true, true);
        let result = (|| {
            let current = self.store.load(id)?.ok_or(AppError::NotFound(id))?;
            let prior =
                self.store
                    .load_revision(id, revision)?
                    .ok_or(StoreError::InvalidDocument(
                        "requested revision is unavailable",
                    ))?;
            let mut restored = current.clone();
            restored.title = prior.title;
            restored.blocks = prior.blocks;
            restored.workspace_id = prior.workspace_id;
            restored.workspace_ids = prior.workspace_ids;
            restored.tags = prior.tags;
            restored.deleted = false;
            restored.updated_at = self.clock.now();
            self.store
                .commit(&restored, current.revision)
                .map_err(AppError::from)
        })();
        self.finish_document_operation(id, false, false);
        let saved = result?;
        self.record_activity(ActivityEvent {
            object_id: id,
            kind: ActivityKind::Restored,
            origin: ActivityOrigin::User,
            revision: saved.revision,
            workspace_id: saved.workspace_id,
            occurred_at: saved.updated_at,
        });
        Ok(saved)
    }

    pub fn restore_as_copy(&self, id: ObjectId, revision: u64) -> Result<NoteDocument, AppError> {
        let prior = self
            .store
            .load_revision(id, revision)?
            .ok_or(StoreError::InvalidDocument(
                "requested revision is unavailable",
            ))?;
        let copy = self.create_note(format!("{} (copy)", prior.title))?;
        let mut blocks = prior.blocks;
        for block in &mut blocks {
            block.id = self.ids.next_object_id();
        }
        copy.replace_blocks(blocks)?;
        copy.set_workspace(prior.workspace_id)?;
        copy.replace_workspaces(ActivityOrigin::User, prior.workspace_ids)?;
        copy.set_tags(prior.tags)?;
        copy.flush()?;
        Ok(copy.note())
    }

    pub fn import_markdown(&self, markdown: &str) -> Result<NoteDocument, AppError> {
        let session = self.create_note("Untitled")?;
        let current = session.note();
        let imported = import_markdown(current.id, markdown, self.ids.as_ref(), self.clock.now());
        session.set_title(imported.title)?;
        session.replace_blocks(imported.blocks)?;
        session.flush()?;
        Ok(session.note())
    }

    pub fn export_note(&self, id: ObjectId) -> Result<String, AppError> {
        let note = self.get_note(id)?;
        Ok(export_markdown(&note))
    }

    pub fn add_file_reference(
        &self,
        session: &NoteSession,
        object_id: ObjectId,
        label: impl Into<String>,
    ) -> Result<ObjectId, AppError> {
        let id = self.ids.next_object_id();
        session.append_block(Block::new(
            id,
            BlockKind::FileReference {
                label: label.into(),
                object_id,
            },
        ))?;
        Ok(id)
    }

    pub fn add_web_reference(
        &self,
        session: &NoteSession,
        title: impl Into<String>,
        url: impl Into<String>,
    ) -> Result<ObjectId, AppError> {
        let url = url.into();
        if !valid_web_url(&url) {
            return Err(AppError::InvalidWebReference);
        }
        let id = self.ids.next_object_id();
        session.append_block(Block::new(
            id,
            BlockKind::WebReference {
                title: title.into(),
                url,
            },
        ))?;
        Ok(id)
    }

    pub fn add_albert_reference(
        &self,
        session: &NoteSession,
        reference: AlbertReference,
    ) -> Result<ObjectId, AppError> {
        if !valid_web_url(&reference.url) {
            return Err(AppError::InvalidWebReference);
        }
        let id = self.ids.next_object_id();
        session.append_block(Block::new(id, BlockKind::AlbertReference(reference)))?;
        Ok(id)
    }

    pub fn new_block_id(&self) -> ObjectId {
        self.ids.next_object_id()
    }

    pub fn activity_failures(&self) -> u64 {
        self.activity_failures.load(Ordering::Relaxed)
    }

    fn record_activity(&self, event: ActivityEvent) {
        if self.activity.record(event).is_err() {
            self.activity_failures.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn reserve_document(&self, id: ObjectId) -> Result<Option<OpenDocument>, AppError> {
        let mut open = lock(&self.open_documents);
        if !open.busy.insert(id.0) {
            return Err(AppError::DocumentBusy(id));
        }
        Ok(open.sessions.get(&id.0).cloned())
    }

    fn finish_document_operation(&self, id: ObjectId, remove_session: bool, keep_busy: bool) {
        let mut open = lock(&self.open_documents);
        if remove_session {
            open.sessions.remove(&id.0);
        }
        if !keep_busy {
            open.busy.remove(&id.0);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
