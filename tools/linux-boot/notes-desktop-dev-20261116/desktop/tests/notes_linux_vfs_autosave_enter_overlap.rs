#![cfg(target_os = "linux")]

use libnagi::{InputEvent, INPUT_EVENT_KEY, INPUT_KEY_ENTER, INPUT_KEY_TAB};
use nagi_desktop_notes_integration_candidate::desktop::{new_linux_development_volume, Desktop};
use nagi_localization::Locale;
use notes_event_bridge_candidate::NotesEventBridge;
use notes_files_codec_candidate::{
    NoteDocument, NoteStore, NoteSummary, NotesApp, NotesUiController, ObjectId, SaveStatus,
    StoreError, Timestamp, VfsNoteStore,
};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const DISK_BYTES: u64 = 8 * 1024 * 1024;
const AUTOSAVE_DEBOUNCE: Duration = Duration::from_millis(700);
const NOTE_ID: ObjectId = ObjectId(0x4e41_4749_0000_00a2);
const KEY_A: u16 = 30;
const KEY_S: u16 = 31;
static DISK_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct TestDisk(PathBuf);

impl TestDisk {
    fn create() -> io::Result<Self> {
        let sequence = DISK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nagi-notes-autosave-enter-{}-{sequence}.raw",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.set_len(DISK_BYTES)?;
        file.sync_all()?;
        drop(file);
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDisk {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[derive(Clone)]
struct CommitObservation {
    attempted_at: Instant,
    expected_revision: u64,
    candidate: NoteDocument,
    result: Option<Result<NoteDocument, StoreError>>,
}

#[derive(Default)]
struct GateState {
    attempts: Vec<CommitObservation>,
    released_attempts: usize,
}

/// Pauses the NoteStore call before it reaches VfsNoteStore, then delegates unchanged.
struct GatedStore {
    provider: Arc<VfsNoteStore>,
    state: Mutex<GateState>,
    changed: Condvar,
}

impl GatedStore {
    fn new(provider: Arc<VfsNoteStore>) -> Self {
        Self {
            provider,
            state: Mutex::new(GateState::default()),
            changed: Condvar::new(),
        }
    }

    fn wait_for_attempts(
        &self,
        count: usize,
        timeout: Duration,
    ) -> io::Result<Vec<CommitObservation>> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("commit observer lock was poisoned"))?;
        while state.attempts.len() < count {
            let now = Instant::now();
            if now >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "only {} of {count} commit attempts started",
                        state.attempts.len()
                    ),
                ));
            }
            let (next, timed_out) = self
                .changed
                .wait_timeout(state, deadline - now)
                .map_err(|_| io::Error::other("commit observer lock was poisoned"))?;
            state = next;
            if timed_out.timed_out() && state.attempts.len() < count {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "only {} of {count} commit attempts started",
                        state.attempts.len()
                    ),
                ));
            }
        }
        Ok(state.attempts.clone())
    }

    fn observations(&self) -> io::Result<Vec<CommitObservation>> {
        self.state
            .lock()
            .map(|state| state.attempts.clone())
            .map_err(|_| io::Error::other("commit observer lock was poisoned"))
    }

    fn release_through(&self, count: usize) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("commit observer lock was poisoned"))?;
        state.released_attempts = state.released_attempts.max(count);
        self.changed.notify_all();
        Ok(())
    }
}

impl NoteStore for GatedStore {
    fn list(&self, include_deleted: bool) -> Result<Vec<NoteSummary>, StoreError> {
        self.provider.list(include_deleted)
    }

    fn load(&self, id: ObjectId) -> Result<Option<NoteDocument>, StoreError> {
        self.provider.load(id)
    }

    fn load_revision(
        &self,
        id: ObjectId,
        revision: u64,
    ) -> Result<Option<NoteDocument>, StoreError> {
        self.provider.load_revision(id, revision)
    }

    fn commit(
        &self,
        note: &NoteDocument,
        expected_revision: u64,
    ) -> Result<NoteDocument, StoreError> {
        let attempt_index = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| StoreError::Io("commit observer lock was poisoned".to_owned()))?;
            let index = state.attempts.len();
            state.attempts.push(CommitObservation {
                attempted_at: Instant::now(),
                expected_revision,
                candidate: note.clone(),
                result: None,
            });
            self.changed.notify_all();
            while state.released_attempts <= index {
                state = self
                    .changed
                    .wait(state)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
            }
            index
        };

        let result = self.provider.commit(note, expected_revision);
        let mut state = self
            .state
            .lock()
            .map_err(|_| StoreError::Io("commit observer lock was poisoned".to_owned()))?;
        state.attempts[attempt_index].result = Some(result.clone());
        self.changed.notify_all();
        result
    }
}

fn key(code: u16) -> InputEvent {
    InputEvent {
        event_type: INPUT_EVENT_KEY,
        code,
        value: 1,
    }
}

fn spawn_enter_releaser(
    store: Arc<GatedStore>,
    required_attempts: usize,
    release_count: usize,
) -> (SyncSender<()>, JoinHandle<io::Result<Instant>>) {
    let (enter_tx, enter_rx) = std::sync::mpsc::sync_channel::<()>(0);
    let releaser = thread::spawn(move || -> io::Result<Instant> {
        enter_rx.recv().map_err(|error| {
            io::Error::other(format!("Enter dispatch handshake failed: {error}"))
        })?;
        // This channel is sent before Desktop::handle_event, not from inside its
        // handler. The 300 ms delay is only a scheduling window, not a barrier to
        // NoteSession::flush arrival. The store gate is before VfsNoteStore::commit,
        // not during a VFS write.
        thread::sleep(Duration::from_millis(300));
        store.release_through(1)?;

        if required_attempts > 1 {
            if let Err(error) = store.wait_for_attempts(required_attempts, Duration::from_secs(10))
            {
                let _ = store.release_through(usize::MAX);
                return Err(error);
            }
            store.release_through(release_count)?;
        }
        Ok(Instant::now())
    });
    (enter_tx, releaser)
}

#[test]
fn notes_linux_vfs_standard_autosave_edit_during_gated_commit_saves_next_revision(
) -> Result<(), Box<dyn std::error::Error>> {
    let disk = TestDisk::create()?;
    let (mut volume, formatted) = new_linux_development_volume(disk.path(), true)?;
    assert!(formatted, "fresh test file was not formatted");

    let provider = Arc::new(VfsNoteStore::new(volume.clone()));
    let initial = NoteDocument::new(NOTE_ID, "700 ms autosave", Timestamp(1));
    let first = provider.commit(&initial, 0)?;
    assert_eq!(first.revision, 1);

    let observed_store = Arc::new(GatedStore::new(Arc::clone(&provider)));
    let app = NotesApp::new(observed_store.clone());
    let session = app.open_note(NOTE_ID)?;
    assert_eq!(session.status(), SaveStatus::Saved);
    let bridge = NotesEventBridge::from_controller(NotesUiController::from_app(app));
    let mut desktop = Desktop::new_with_notes(Locale::EnUs, bridge, NOTE_ID)?;

    for _ in 0..3 {
        assert!(desktop.handle_event(key(INPUT_KEY_TAB), &mut volume));
    }
    assert!(desktop.handle_event(key(INPUT_KEY_ENTER), &mut volume));
    assert!(desktop.linux_development_notes_window_active());

    let first_edit_started_at = Instant::now();
    assert!(desktop.handle_event(key(KEY_A), &mut volume));
    let first_attempts = observed_store.wait_for_attempts(1, Duration::from_secs(10))?;
    assert!(
        first_attempts[0]
            .attempted_at
            .duration_since(first_edit_started_at)
            >= AUTOSAVE_DEBOUNCE,
        "NotesApp::new must use its regular 700 ms autosave debounce"
    );
    assert_eq!(first_attempts[0].expected_revision, 1);
    assert_eq!(first_attempts[0].candidate.search_text(), "a");
    assert!(
        first_attempts[0].result.is_none(),
        "first autosave is gated in flight"
    );

    // Edit after the worker started its first snapshot but before the real
    // VfsNoteStore::commit call. The session must retain this newer generation.
    assert!(desktop.handle_event(key(KEY_S), &mut volume));
    assert_eq!(session.note().search_text(), "as");
    assert_eq!(session.note().revision, 1);
    assert!(session.is_dirty());
    assert_eq!(observed_store.observations()?.len(), 1);

    let (enter_tx, releaser) = spawn_enter_releaser(Arc::clone(&observed_store), 2, 2);

    enter_tx.send(())?;
    let enter_started_at = Instant::now();
    assert!(desktop.handle_event(key(INPUT_KEY_ENTER), &mut volume));
    let enter_returned_at = Instant::now();
    let second_release_at = releaser
        .join()
        .map_err(|_| io::Error::other("commit release thread panicked"))??;

    assert!(enter_returned_at >= second_release_at);
    assert!(
        enter_returned_at.duration_since(enter_started_at) >= Duration::from_millis(250),
        "this run's Desktop event call did not span the release handoff"
    );

    let commits = observed_store.observations()?;
    assert_eq!(commits.len(), 2, "one snapshot plus one pending generation");
    assert_eq!(commits[0].expected_revision, 1);
    assert_eq!(commits[0].candidate.search_text(), "a");
    assert_eq!(
        commits[0].result,
        Some(Ok({
            let mut saved = commits[0].candidate.clone();
            saved.revision = 2;
            saved
        }))
    );
    assert_eq!(commits[1].expected_revision, 2);
    assert_eq!(commits[1].candidate.search_text(), "as");
    assert_eq!(
        commits[1].result,
        Some(Ok({
            let mut saved = commits[1].candidate.clone();
            saved.revision = 3;
            saved
        }))
    );

    assert_eq!(session.status(), SaveStatus::Saved);
    assert!(!session.is_dirty());
    assert_eq!(session.note().revision, 3);
    assert_eq!(session.note().search_text(), "as");
    assert_eq!(desktop.current_notes_document().unwrap().revision, 3);
    assert_eq!(
        desktop.current_notes_document().unwrap().search_text(),
        "as"
    );
    assert_eq!(provider.load(NOTE_ID)?, Some(session.note()));
    assert_eq!(provider.load_revision(NOTE_ID, 1)?, Some(first));
    assert_eq!(
        provider.load_revision(NOTE_ID, 2)?.unwrap().search_text(),
        "a"
    );
    assert_eq!(
        provider.load_revision(NOTE_ID, 3)?.unwrap().search_text(),
        "as"
    );
    assert_eq!(provider.load_revision(NOTE_ID, 4)?, None);

    drop(desktop);
    drop(session);
    drop(provider);
    drop(observed_store);
    drop(volume);
    Ok(())
}

#[test]
fn notes_linux_vfs_standard_autosave_conflict_preserves_pending_edit(
) -> Result<(), Box<dyn std::error::Error>> {
    let disk = TestDisk::create()?;
    let (mut volume, formatted) = new_linux_development_volume(disk.path(), true)?;
    assert!(formatted, "fresh test file was not formatted");

    let provider = Arc::new(VfsNoteStore::new(volume.clone()));
    let initial = NoteDocument::new(NOTE_ID, "700 ms autosave conflict", Timestamp(1));
    let first = provider.commit(&initial, 0)?;
    assert_eq!(first.revision, 1);

    let observed_store = Arc::new(GatedStore::new(Arc::clone(&provider)));
    let app = NotesApp::new(observed_store.clone());
    let session = app.open_note(NOTE_ID)?;
    let bridge = NotesEventBridge::from_controller(NotesUiController::from_app(app));
    let mut desktop = Desktop::new_with_notes(Locale::EnUs, bridge, NOTE_ID)?;

    for _ in 0..3 {
        assert!(desktop.handle_event(key(INPUT_KEY_TAB), &mut volume));
    }
    assert!(desktop.handle_event(key(INPUT_KEY_ENTER), &mut volume));
    assert!(desktop.linux_development_notes_window_active());

    let first_edit_started_at = Instant::now();
    assert!(desktop.handle_event(key(KEY_A), &mut volume));
    let first_attempts = observed_store.wait_for_attempts(1, Duration::from_secs(10))?;
    assert!(
        first_attempts[0]
            .attempted_at
            .duration_since(first_edit_started_at)
            >= AUTOSAVE_DEBOUNCE
    );
    assert_eq!(first_attempts[0].expected_revision, 1);
    assert_eq!(first_attempts[0].candidate.search_text(), "a");

    assert!(desktop.handle_event(key(KEY_S), &mut volume));
    assert_eq!(session.note().search_text(), "as");
    assert_eq!(session.note().revision, 1);
    assert!(session.is_dirty());

    // A second writer wins after the regular autosave worker starts its stale snapshot.
    let competitor = VfsNoteStore::new(volume.clone());
    let mut winning_candidate = first.clone();
    winning_candidate.title = "external winner".to_owned();
    winning_candidate.updated_at = Timestamp(2);
    let winner = competitor.commit(&winning_candidate, 1)?;
    assert_eq!(winner.revision, 2);

    let (enter_tx, releaser) = spawn_enter_releaser(Arc::clone(&observed_store), 1, 1);
    enter_tx.send(())?;
    let enter_started_at = Instant::now();
    assert!(desktop.handle_event(key(INPUT_KEY_ENTER), &mut volume));
    let enter_returned_at = Instant::now();
    let release_at = releaser
        .join()
        .map_err(|_| io::Error::other("commit release thread panicked"))??;

    assert!(enter_returned_at >= release_at);
    assert!(
        enter_returned_at.duration_since(enter_started_at) >= Duration::from_millis(250),
        "this run's Desktop event call did not span the release handoff"
    );
    let commits = observed_store.observations()?;
    assert_eq!(
        commits.len(),
        1,
        "failed in-flight autosave does not issue another commit"
    );
    assert_eq!(commits[0].expected_revision, 1);
    assert_eq!(commits[0].candidate.search_text(), "a");
    assert_eq!(
        commits[0].result,
        Some(Err(StoreError::RevisionConflict {
            expected: 1,
            actual: 2,
        }))
    );

    assert!(matches!(
        session.status(),
        SaveStatus::Failed(ref error)
            if error.source == StoreError::RevisionConflict { expected: 1, actual: 2 }
    ));
    assert!(session.is_dirty());
    assert_eq!(session.note().revision, 1);
    assert_eq!(session.note().search_text(), "as");
    assert_eq!(desktop.current_notes_document().unwrap().revision, 1);
    assert_eq!(
        desktop.current_notes_document().unwrap().search_text(),
        "as"
    );
    assert_eq!(provider.load(NOTE_ID)?, Some(winner.clone()));
    assert_eq!(provider.load_revision(NOTE_ID, 1)?, Some(first));
    assert_eq!(provider.load_revision(NOTE_ID, 2)?, Some(winner));
    assert_eq!(provider.load_revision(NOTE_ID, 3)?, None);

    drop(desktop);
    drop(session);
    drop(provider);
    drop(competitor);
    drop(observed_store);
    drop(volume);
    Ok(())
}
