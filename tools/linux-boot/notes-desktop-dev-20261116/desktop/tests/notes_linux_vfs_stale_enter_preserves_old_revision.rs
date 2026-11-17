#![cfg(target_os = "linux")]

use libnagi::{InputEvent, INPUT_EVENT_KEY, INPUT_KEY_ENTER, INPUT_KEY_TAB};
use nagi_desktop_notes_integration_candidate::desktop::{new_linux_development_volume, Desktop};
use nagi_localization::Locale;
use notes_event_bridge_candidate::NotesEventBridge;
use notes_files_codec_candidate::{
    HostObjectIdSource, NoopActivitySink, NoteDocument, NoteStore, NoteSummary, NotesApp,
    NotesUiController, ObjectId, SaveError, SaveStatus, StoreError, SystemClock, Timestamp,
    VfsNoteStore,
};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DISK_BYTES: u64 = 8 * 1024 * 1024;
const AUTOSAVE_DEBOUNCE: Duration = Duration::from_secs(24 * 60 * 60);
const NOTE_ID: ObjectId = ObjectId(0x4e41_4749_0000_00a1);
const KEY_A: u16 = 30;
static DISK_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct TestDisk(PathBuf);

impl TestDisk {
    fn create() -> io::Result<Self> {
        let sequence = DISK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nagi-notes-vfs-enter-{}-{sequence}.raw",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        let disk = Self(path);
        file.set_len(DISK_BYTES)?;
        file.sync_all()?;
        drop(file);
        Ok(disk)
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
    candidate_revision: u64,
    result: Result<NoteDocument, StoreError>,
}

/// Records commit outcomes while delegating every operation to real LinuxVfs.
struct ObservedStore {
    provider: Arc<VfsNoteStore>,
    commits: Mutex<Vec<CommitObservation>>,
}

impl ObservedStore {
    fn new(provider: Arc<VfsNoteStore>) -> Self {
        Self {
            provider,
            commits: Mutex::new(Vec::new()),
        }
    }

    fn commits(&self) -> io::Result<Vec<CommitObservation>> {
        self.commits
            .lock()
            .map(|commits| commits.clone())
            .map_err(|_| io::Error::other("commit observer lock was poisoned"))
    }
}

impl NoteStore for ObservedStore {
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
        let attempted_at = Instant::now();
        let result = self.provider.commit(note, expected_revision);
        self.commits
            .lock()
            .map_err(|_| StoreError::Io("commit observer lock was poisoned".to_owned()))?
            .push(CommitObservation {
                attempted_at,
                expected_revision,
                candidate_revision: note.revision,
                result: result.clone(),
            });
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

#[test]
fn notes_linux_vfs_stale_enter_preserves_old_revision() -> Result<(), Box<dyn std::error::Error>> {
    const KEY_A_SCANCODE: u16 = KEY_A;

    let disk = TestDisk::create()?;
    let (mut volume, formatted) = new_linux_development_volume(disk.path(), true)?;
    assert!(formatted, "fresh test file was not formatted");

    let provider = Arc::new(VfsNoteStore::new(volume.clone()));
    let initial = NoteDocument::new(NOTE_ID, "stale Enter", Timestamp(1));
    let first = provider.commit(&initial, 0)?;
    assert_eq!(first.revision, 1);

    let ui_store = Arc::new(ObservedStore::new(Arc::clone(&provider)));
    let app = NotesApp::with_providers(
        ui_store.clone(),
        Arc::new(HostObjectIdSource::new()),
        Arc::new(SystemClock),
        Arc::new(NoopActivitySink),
        AUTOSAVE_DEBOUNCE,
    );
    let session = app.open_note(NOTE_ID)?;
    assert_eq!(session.status(), SaveStatus::Saved);
    let bridge = NotesEventBridge::from_controller(NotesUiController::from_app(app));
    let mut desktop = Desktop::new_with_notes(Locale::EnUs, bridge, NOTE_ID)?;

    for _ in 0..3 {
        assert!(desktop.handle_event(key(INPUT_KEY_TAB), &mut volume));
    }
    assert!(desktop.handle_event(key(INPUT_KEY_ENTER), &mut volume));
    assert!(desktop.linux_development_notes_window_active());

    // A consumed clean Enter is not evidence that a stale save was attempted.
    assert!(desktop.handle_event(key(INPUT_KEY_ENTER), &mut volume));
    assert_eq!(session.status(), SaveStatus::Saved);
    assert!(ui_store.commits()?.is_empty());

    let competing_store = VfsNoteStore::new(volume.clone());
    let mut winner_candidate = first.clone();
    winner_candidate.title = "concurrent winner".to_owned();
    winner_candidate.updated_at = Timestamp(2);
    let winner = competing_store.commit(&winner_candidate, 1)?;
    assert_eq!(winner.revision, 2);

    assert!(desktop.handle_event(key(KEY_A_SCANCODE), &mut volume));
    let edit_returned_at = Instant::now();
    let dirty_before_enter = desktop.current_notes_document().unwrap();
    assert_eq!(dirty_before_enter.revision, 1);
    assert_eq!(dirty_before_enter.search_text(), "a");
    assert_eq!(session.status(), SaveStatus::Dirty);
    assert!(ui_store.commits()?.is_empty());

    let bytes_before = fs::read(disk.path())?;
    assert!(edit_returned_at.elapsed() < AUTOSAVE_DEBOUNCE);
    let enter_started_at = Instant::now();
    assert!(desktop.handle_event(key(INPUT_KEY_ENTER), &mut volume));
    let enter_returned_at = Instant::now();
    assert!(enter_returned_at.duration_since(edit_returned_at) < AUTOSAVE_DEBOUNCE);

    let commits = ui_store.commits()?;
    assert_eq!(commits.len(), 1, "dirty Enter must attempt one UI commit");
    let attempt = &commits[0];
    assert!(attempt.attempted_at >= enter_started_at);
    assert!(attempt.attempted_at <= enter_returned_at);
    assert_eq!(attempt.expected_revision, 1);
    assert_eq!(attempt.candidate_revision, 1);
    assert_eq!(
        attempt.result,
        Err(StoreError::RevisionConflict {
            expected: 1,
            actual: 2,
        })
    );
    assert_eq!(
        session.status(),
        SaveStatus::Failed(SaveError {
            source: StoreError::RevisionConflict {
                expected: 1,
                actual: 2,
            },
        })
    );

    assert_eq!(fs::read(disk.path())?, bytes_before);
    let dirty_after_enter = desktop.current_notes_document().unwrap();
    assert_eq!(dirty_after_enter.revision, 1);
    assert_eq!(dirty_after_enter.search_text(), "a");
    assert_eq!(provider.load(NOTE_ID)?, Some(winner.clone()));
    assert_eq!(provider.load_revision(NOTE_ID, 1)?, Some(first.clone()));
    assert_eq!(provider.load_revision(NOTE_ID, 2)?, Some(winner.clone()));
    assert_eq!(provider.load_revision(NOTE_ID, 3)?, None);

    drop(desktop);
    drop(session);
    drop(ui_store);
    drop(provider);
    drop(competing_store);
    drop(volume);

    let (reopened_volume, formatted) = new_linux_development_volume(disk.path(), false)?;
    assert!(!formatted, "initialized VFS was reformatted on reopen");
    let reopened_store = VfsNoteStore::new(reopened_volume);
    assert_eq!(reopened_store.load(NOTE_ID)?, Some(winner.clone()));
    assert_eq!(reopened_store.load_revision(NOTE_ID, 1)?, Some(first));
    assert_eq!(reopened_store.load_revision(NOTE_ID, 2)?, Some(winner));
    assert_eq!(reopened_store.load_revision(NOTE_ID, 3)?, None);

    Ok(())
}
