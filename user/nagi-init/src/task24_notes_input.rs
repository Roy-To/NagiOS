//! Task24's explicit Notes save input and its durable confirmation boundary.

use libnagi::{InputEvent, INPUT_EVENT_KEY, INPUT_KEY_ENTER, INPUT_KEY_ESCAPE};
use notes_event_bridge_candidate::ScopedNotesEventBridge;
use notes_files_codec_candidate::{NoteDocument, NoteStore};

// Linux evdev KEY_KPENTER is distinct from KEY_ENTER (28). Both are explicit
// save actions in the Task24 console; neither is treated as text input.
const KEY_KPENTER: u16 = 96;

pub(crate) const fn is_enter_code(code: u16) -> bool {
    code == INPUT_KEY_ENTER || code == KEY_KPENTER
}

/// Route one event through the same Notes bridge used by the Task24 console.
/// A save is reported only after the current session's document is read back
/// exactly from that bridge's authorized NoteStore. If autosave already made
/// the session clean, this confirms the existing commit without writing again.
pub(crate) fn handle_event(
    bridge: &mut ScopedNotesEventBridge<'_>,
    store: &dyn NoteStore,
    event: InputEvent,
    has_unconfirmed_user_edit: &mut bool,
) -> Result<Option<NoteDocument>, String> {
    if event.event_type == INPUT_EVENT_KEY && is_enter_code(event.code) {
        if event.value != 1 || !*has_unconfirmed_user_edit {
            return Ok(None);
        }

        // Dirty drafts are committed by retry_active. A clean session returns
        // None, so use its current document and verify that exact version from
        // the same authorized store before reporting the explicit save.
        let candidate = bridge
            .retry_active()
            .map_err(|error| error.to_string())?
            .or_else(|| bridge.current_note());
        let Some(candidate) = candidate else {
            return Ok(None);
        };
        let persisted = store
            .load(candidate.id)
            .map_err(|error| error.to_string())?;
        if persisted.as_ref() != Some(&candidate) {
            return Err("explicit Notes save did not match durable readback".to_owned());
        }
        *has_unconfirmed_user_edit = false;
        return Ok(persisted);
    }

    let result = bridge
        .handle_event(event)
        .map_err(|error| error.to_string())?;
    if event.event_type == INPUT_EVENT_KEY
        && event.value == 1
        && event.code != INPUT_KEY_ESCAPE
        && result.is_some()
    {
        *has_unconfirmed_user_edit = true;
    }
    Ok(result)
}

pub(crate) fn is_escape_press(event: InputEvent) -> bool {
    event.event_type == INPUT_EVENT_KEY && event.code == INPUT_KEY_ESCAPE && event.value == 1
}

/// Only a second distinct Esc press confirms an unpersisted close.
/// Key releases leave the confirmation pending; another key cancels it.
#[derive(Default)]
pub(crate) struct UnsavedCloseConfirmation {
    pending: bool,
}

impl UnsavedCloseConfirmation {
    pub(crate) fn pending(&self) -> bool {
        self.pending
    }

    pub(crate) fn on_input(&mut self, event: InputEvent) -> bool {
        if event.event_type != INPUT_EVENT_KEY || event.value != 1 {
            return false;
        }
        std::mem::take(&mut self.pending) && is_escape_press(event)
    }

    pub(crate) fn after_failed_escape(&mut self, permanently_denied: bool) -> bool {
        self.pending = !permanently_denied;
        permanently_denied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notes_event_bridge_candidate::NotesEventBridge;
    use notes_files_codec_candidate::{InMemoryNoteStore, NoteSummary, StoreError, Timestamp};
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    fn key(code: u16, value: i32) -> InputEvent {
        InputEvent {
            event_type: INPUT_EVENT_KEY,
            code,
            value,
        }
    }

    fn seeded_bridge(
        store: &Arc<InMemoryNoteStore>,
        note_id: notes_files_codec_candidate::ObjectId,
    ) -> NotesEventBridge {
        let initial = NoteDocument::new(note_id, "Task24 explicit save", Timestamp(1));
        let saved = store.commit(&initial, 0).expect("seed the existing note");
        assert_eq!(saved.revision, 1);
        let mut bridge = NotesEventBridge::new(Arc::clone(store));
        let opened = bridge.open_note(note_id).expect("open selected note");
        assert_eq!(opened, saved);
        bridge
    }

    fn wait_for_autosave(
        bridge: &NotesEventBridge,
        store: &dyn NoteStore,
        note_id: notes_files_codec_candidate::ObjectId,
        revision: u64,
    ) -> NoteDocument {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(persisted) = store.load(note_id).expect("read current durable note") {
                if persisted.revision >= revision
                    && bridge.current_note().as_ref() == Some(&persisted)
                {
                    return persisted;
                }
            }
            assert!(Instant::now() < deadline, "Notes autosave did not complete");
            // The production Notes debounce is 700 ms; waiting here exercises
            // the clean-session path seen after ordinary, slower typing.
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn ordinary_return_confirms_autosave_once_and_keypad_enter_saves_dirty_text() {
        let store = Arc::new(InMemoryNoteStore::new());
        let note_id = notes_files_codec_candidate::ObjectId(0x5441_534b_3234_0001);
        let mut bridge = seeded_bridge(&store, note_id);
        let mut pending = false;

        assert!(
            handle_event(&mut bridge, store.as_ref(), key(30, 1), &mut pending)
                .expect("type ordinary text")
                .is_some()
        );
        assert!(pending);

        let autosaved = wait_for_autosave(&bridge, store.as_ref(), note_id, 2);
        assert_eq!(autosaved.search_text(), "a");

        let confirmed = handle_event(
            &mut bridge,
            store.as_ref(),
            key(INPUT_KEY_ENTER, 1),
            &mut pending,
        )
        .expect("ordinary Return confirms the durable save")
        .expect("emit exactly one save result");
        assert_eq!(confirmed, autosaved);
        assert!(!pending);
        assert_eq!(store.load(note_id).unwrap().unwrap().revision, 2);
        assert!(handle_event(
            &mut bridge,
            store.as_ref(),
            key(INPUT_KEY_ENTER, 1),
            &mut pending,
        )
        .expect("duplicate Return is ignored")
        .is_none());
        assert_eq!(store.load(note_id).unwrap().unwrap().revision, 2);

        assert!(
            handle_event(&mut bridge, store.as_ref(), key(48, 1), &mut pending)
                .expect("type another character")
                .is_some()
        );
        let keypad_saved = handle_event(
            &mut bridge,
            store.as_ref(),
            key(KEY_KPENTER, 1),
            &mut pending,
        )
        .expect("keypad Enter saves the dirty draft")
        .expect("emit the keypad save result");
        assert_eq!(keypad_saved.revision, 3);
        assert_eq!(keypad_saved.search_text(), "ab");
        assert!(!pending);
        assert_eq!(store.load(note_id).unwrap().unwrap(), keypad_saved);
    }

    #[test]
    fn empty_note_does_not_produce_an_explicit_save_result() {
        let store = Arc::new(InMemoryNoteStore::new());
        let mut bridge = NotesEventBridge::new(Arc::clone(&store));
        let note_id = bridge
            .create_note("Task24 empty note")
            .expect("create initial empty note");
        let mut pending = false;
        let initial = wait_for_autosave(&bridge, store.as_ref(), note_id, 1);
        assert_eq!(initial.revision, 1);
        assert!(initial.search_text().is_empty());

        assert!(handle_event(
            &mut bridge,
            store.as_ref(),
            key(INPUT_KEY_ENTER, 1),
            &mut pending,
        )
        .expect("empty Enter is harmless")
        .is_none());
        assert_eq!(store.load(note_id).unwrap().unwrap().revision, 1);
        assert!(!pending);
    }

    struct FailingCommitStore {
        inner: Arc<InMemoryNoteStore>,
    }

    impl NoteStore for FailingCommitStore {
        fn list(&self, include_deleted: bool) -> Result<Vec<NoteSummary>, StoreError> {
            self.inner.list(include_deleted)
        }

        fn load(
            &self,
            id: notes_files_codec_candidate::ObjectId,
        ) -> Result<Option<NoteDocument>, StoreError> {
            self.inner.load(id)
        }

        fn load_revision(
            &self,
            id: notes_files_codec_candidate::ObjectId,
            revision: u64,
        ) -> Result<Option<NoteDocument>, StoreError> {
            self.inner.load_revision(id, revision)
        }

        fn commit(
            &self,
            _note: &NoteDocument,
            _expected_revision: u64,
        ) -> Result<NoteDocument, StoreError> {
            Err(StoreError::Io("synthetic commit failure".to_owned()))
        }
    }

    #[test]
    fn transient_save_failure_requires_a_second_escape() {
        let inner = Arc::new(InMemoryNoteStore::new());
        let note_id = notes_files_codec_candidate::ObjectId(0x5441_534b_3234_0003);
        let initial = NoteDocument::new(note_id, "Task24 explicit save", Timestamp(1));
        let initial = inner.commit(&initial, 0).expect("seed the existing note");
        let store = Arc::new(FailingCommitStore {
            inner: inner.clone(),
        });
        let mut bridge = NotesEventBridge::new(store.clone());
        bridge.open_note(note_id).expect("open selected note");
        let mut pending = false;

        assert!(
            handle_event(&mut bridge, store.as_ref(), key(30, 1), &mut pending)
                .expect("type text before failure")
                .is_some()
        );
        let result = handle_event(
            &mut bridge,
            store.as_ref(),
            key(INPUT_KEY_ENTER, 1),
            &mut pending,
        );
        assert!(result.is_err());
        assert!(pending, "the unsaved draft remains eligible for retry");
        let escape = key(INPUT_KEY_ESCAPE, 1);
        assert!(handle_event(&mut bridge, store.as_ref(), escape, &mut pending).is_err());
        let mut confirmation = UnsavedCloseConfirmation::default();
        assert!(!confirmation.on_input(escape));
        assert!(!confirmation.after_failed_escape(false));
        assert!(confirmation.pending());
        assert!(!confirmation.on_input(key(INPUT_KEY_ESCAPE, 0)));
        assert!(!confirmation.on_input(key(INPUT_KEY_ESCAPE, 2)));
        assert!(confirmation.on_input(escape));
        assert!(!confirmation.pending());
        let draft = bridge.current_note().unwrap();
        assert_eq!(draft.id, initial.id);
        assert_ne!(draft.blocks, initial.blocks);
        drop(bridge);

        assert_eq!(inner.load(note_id).unwrap().unwrap(), initial);
    }

    #[test]
    fn edit_cancels_confirmation_and_permanent_denial_skips_it() {
        let escape = key(INPUT_KEY_ESCAPE, 1);
        let mut confirmation = UnsavedCloseConfirmation::default();
        assert!(!confirmation.after_failed_escape(false));
        assert!(!confirmation.on_input(key(30, 1)));
        assert!(!confirmation.on_input(escape));

        assert!(confirmation.after_failed_escape(true));
        assert!(!confirmation.pending());
    }
}
