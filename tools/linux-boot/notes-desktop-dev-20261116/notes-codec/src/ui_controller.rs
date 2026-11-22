use crate::app::{AppError, NotesApp, ScopedNotesApp, ScopedOpenDocument};
use crate::domain::{Block, BlockKind, NoteDocument, NoteSummary};
use crate::store::NoteStore;
use nagi_model::ObjectId;
use std::sync::Arc;
use std::thread::Scope;

/// Candidate public-library action controller built from the real Notes APIs.
/// A product UI is not wired to this controller.
pub type NotesUiController = ScopedNotesUiController<'static>;

pub struct ScopedNotesUiController<'scope> {
    app: ScopedNotesApp<'scope>,
    active: Option<ScopedOpenDocument<'scope>>,
}

impl ScopedNotesUiController<'static> {
    /// Build the controller around any existing Notes store implementation.
    pub fn new<S: NoteStore + 'static>(store: Arc<S>) -> Self {
        Self::from_app(NotesApp::new(store))
    }

    /// Use an already configured NotesApp, preserving its injected providers.
    pub fn from_app(app: NotesApp) -> Self {
        Self { app, active: None }
    }
}

impl<'scope> ScopedNotesUiController<'scope> {
    pub fn new_scoped<'env: 'scope, S: NoteStore + 'scope>(
        store: Arc<S>,
        scope: &'scope Scope<'scope, 'env>,
    ) -> Self {
        Self {
            app: ScopedNotesApp::new_scoped(store, scope),
            active: None,
        }
    }

    pub fn list_notes(&self) -> Result<Vec<NoteSummary>, AppError> {
        self.app.list_notes()
    }

    pub fn create_note(&mut self, title: impl Into<String>) -> Result<ObjectId, AppError> {
        self.close_active()?;
        let session = self.app.create_note(title)?;
        let id = session.note().id;
        self.active = Some(session);
        Ok(id)
    }

    pub fn open_note(&mut self, id: ObjectId) -> Result<(), AppError> {
        self.close_active()?;
        self.active = Some(self.app.open_note(id)?);
        Ok(())
    }

    /// Apply one decoded text-input action through the real NoteSession API.
    pub fn input_text(&self, text: &str) -> Result<bool, AppError> {
        let Some(session) = self.active.as_ref() else {
            return Ok(false);
        };

        if text.is_empty() {
            return Ok(false);
        }

        let note = session.note();
        if let Some(last) = note.blocks.last() {
            if let BlockKind::Paragraph(current_text) = &last.kind {
                let mut updated_text = current_text.clone();
                updated_text.push_str(text);
                session.update_block(last.id, BlockKind::Paragraph(updated_text))?;
                return Ok(true);
            }
        }

        session.append_block(Block::new(
            self.app.new_block_id(),
            BlockKind::Paragraph(text.to_owned()),
        ))?;
        Ok(true)
    }

    /// Apply a title action through the active real NoteSession.
    pub fn set_title(&self, title: impl Into<String>) -> Result<bool, AppError> {
        let Some(session) = self.active.as_ref() else {
            return Ok(false);
        };
        session.set_title(title).map_err(AppError::from)?;
        Ok(true)
    }

    pub fn save_active(&self) -> Result<Option<NoteDocument>, AppError> {
        let Some(session) = self.active.as_ref() else {
            return Ok(None);
        };
        if !session.is_dirty() {
            return Ok(None);
        }
        session.flush().map(Some).map_err(AppError::from)
    }

    /// Explicitly retry persistence for the active session after the caller
    /// has re-established its input and chosen to retry. This targets the
    /// existing session and store; it does not create a replacement session.
    pub fn retry_active(&self) -> Result<Option<NoteDocument>, AppError> {
        let Some(session) = self.active.as_ref() else {
            return Ok(None);
        };
        if !session.is_dirty() {
            return Ok(None);
        }
        session.retry().map(Some).map_err(AppError::from)
    }

    pub fn current_note(&self) -> Option<NoteDocument> {
        self.active.as_ref().map(|session| session.note())
    }

    pub fn close_active(&mut self) -> Result<Option<NoteDocument>, AppError> {
        let Some(id) = self.active.as_ref().map(|session| session.note().id) else {
            return Ok(None);
        };
        let saved = self.app.close_note(id)?;
        self.active.take();
        Ok(Some(saved))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{InMemoryNoteStore, NoteStore};

    #[test]
    fn save_active_does_not_report_a_clean_session_as_saved() {
        let store = Arc::new(InMemoryNoteStore::new());
        let mut controller = NotesUiController::new(store.clone());
        let note_id = controller.create_note("Task24 save marker").unwrap();
        let initial = controller
            .save_active()
            .unwrap()
            .expect("new note is persisted");
        assert_eq!(initial.revision, 1);

        let persisted_before = store.load(note_id).unwrap().expect("saved note is present");
        assert!(controller.save_active().unwrap().is_none());
        let persisted_after = store
            .load(note_id)
            .unwrap()
            .expect("saved note remains present");
        assert_eq!(persisted_after.revision, persisted_before.revision);

        controller.close_active().unwrap();
    }

    #[test]
    fn retry_active_does_not_report_a_clean_session_as_saved() {
        let store = Arc::new(InMemoryNoteStore::new());
        let mut controller = NotesUiController::new(store);
        controller.create_note("Task24 save marker").unwrap();
        controller
            .save_active()
            .unwrap()
            .expect("new note is persisted");

        assert!(controller.retry_active().unwrap().is_none());
        controller.close_active().unwrap();
    }
}
