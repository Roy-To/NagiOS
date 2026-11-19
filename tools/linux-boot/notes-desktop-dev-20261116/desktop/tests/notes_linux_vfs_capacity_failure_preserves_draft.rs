#![cfg(target_os = "linux")]

use libnagi::{InputEvent, INPUT_EVENT_KEY, INPUT_KEY_ENTER, INPUT_KEY_TAB};
use nagi_desktop_notes_integration_candidate::desktop::{new_linux_development_volume, Desktop};
use nagi_localization::Locale;
use notes_event_bridge_candidate::NotesEventBridge;
use notes_files_codec_candidate::{
    Block, BlockKind, HostObjectIdSource, NoopActivitySink, NoteDocument, NoteStore, NotesApp,
    NotesUiController, ObjectId, SaveStatus, SessionError, StoreError, SystemClock, Timestamp,
    VfsNoteStore,
};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

const DISK_BYTES: u64 = 8 * 1024 * 1024;
const FILLER_BYTES: usize = 260_000;
const MIN_SUCCESSFUL_FILLER_COMMITS: usize = 28;
const MIN_SUCCESSFUL_FILLER_BYTES: usize = 7_280_000;
const AUTOSAVE_DEBOUNCE: Duration = Duration::from_secs(24 * 60 * 60);
const NOTE_ID: ObjectId = ObjectId(0x4e41_4749_0000_00a1);
const FIRST_FILLER_ID: u64 = 0x4e41_4749_1000_0000;
const FIRST_BLOCK_ID: u64 = 0x424c_4f43_4b10_0000;
const KEY_A: u16 = 30;
static DISK_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct TestDisk(PathBuf);

impl TestDisk {
    fn create() -> io::Result<Self> {
        let sequence = DISK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nagi-notes-vfs-capacity-{}-{sequence}.raw",
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

fn key(code: u16) -> InputEvent {
    InputEvent {
        event_type: INPUT_EVENT_KEY,
        code,
        value: 1,
    }
}

fn next_filler(current: Option<&NoteDocument>, filler_index: u64, version: usize) -> NoteDocument {
    let id = ObjectId(FIRST_FILLER_ID + filler_index);
    let mut candidate = if let Some(current) = current {
        current.clone()
    } else {
        let mut note =
            NoteDocument::new(id, format!("Capacity filler {filler_index}"), Timestamp(1));
        note.blocks.push(Block::new(
            ObjectId(FIRST_BLOCK_ID + filler_index),
            BlockKind::Paragraph(String::new()),
        ));
        note
    };
    candidate.updated_at = Timestamp(version as u64 + 2);
    let mut body = vec![b'x'; FILLER_BYTES];
    body[0] = b'a' + (version % 26) as u8;
    candidate.blocks[0].kind =
        BlockKind::Paragraph(String::from_utf8(body).expect("ASCII filler is valid UTF-8"));
    candidate
}

fn fill_until_vfs_capacity(
    store: &VfsNoteStore,
    filler_count: usize,
) -> Result<(StoreError, usize), String> {
    // The fixture is one bounded 8 MiB file. Each successful commit persists
    // another real immutable VFS note revision; no host-wide filler is used.
    // At least 28 successful 260,000-byte revisions (7.28 MB of payload) are
    // required before Capacity, below the 31 successful revisions observed in
    // the same-size seven-filler capacity fixture.
    let mut current = vec![None::<NoteDocument>; filler_count];
    let mut successful_commits = 0usize;
    for version in 0..8usize {
        for (filler_index, note) in current.iter_mut().enumerate() {
            let expected = note.as_ref().map_or(0, |saved| saved.revision);
            let candidate = next_filler(note.as_ref(), filler_index as u64, version);
            match store.commit(&candidate, expected) {
                Ok(saved) => {
                    *note = Some(saved);
                    successful_commits += 1;
                }
                Err(error) => {
                    return match &error {
                        StoreError::Io(message) if message.contains("Capacity") => {
                            Ok((error, successful_commits))
                        }
                        _ => Err(format!(
                            "filler stopped for a non-capacity reason after {:?}: {error:?}",
                            note.as_ref().map(|saved| saved.revision)
                        )),
                    };
                }
            }
        }
    }
    Err("filler limit ended before the real VFS reported Capacity".to_owned())
}

#[test]
fn notes_linux_vfs_capacity_failure_preserves_draft() -> Result<(), Box<dyn std::error::Error>> {
    let disk = TestDisk::create()?;
    let (mut volume, formatted) = new_linux_development_volume(disk.path(), true)?;
    assert!(formatted, "fresh 8 MiB test file was not formatted");

    let provider = Arc::new(VfsNoteStore::new(volume.clone()));
    let initial = NoteDocument::new(NOTE_ID, "UI note survives full VFS", Timestamp(1));
    let first = provider.commit(&initial, 0)?;
    assert_eq!(first.revision, 1);

    let app = NotesApp::with_providers(
        provider.clone(),
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

    let (fill_error, successful_filler_commits) = fill_until_vfs_capacity(&provider, 8)
        .unwrap_or_else(|error| panic!("real VFS did not reach capacity: {error}"));
    assert!(
        successful_filler_commits >= MIN_SUCCESSFUL_FILLER_COMMITS,
        "capacity arrived after only {successful_filler_commits} successful filler revisions"
    );
    assert!(
        successful_filler_commits * FILLER_BYTES >= MIN_SUCCESSFUL_FILLER_BYTES,
        "successful filler payload was below the calibrated 7.28 MB minimum"
    );
    println!(
        "CAPACITY_FILL_PROGRESS successful_revisions={successful_filler_commits} payload_bytes={}",
        successful_filler_commits * FILLER_BYTES
    );
    assert!(matches!(
        &fill_error,
        StoreError::Io(message) if message.contains("Capacity")
    ));
    assert_eq!(provider.load(NOTE_ID)?, Some(first.clone()));

    assert!(desktop.handle_event(key(KEY_A), &mut volume));
    let payload = "S".repeat(FILLER_BYTES);
    session.append_block(Block::new(
        ObjectId(0x424c_4f43_4b00_ffff),
        BlockKind::Paragraph(payload.clone()),
    ))?;
    let expected_draft = session.note();
    assert_eq!(expected_draft.revision, first.revision);
    assert!(session.is_dirty());

    // Desktop's event API reports whether it consumed Enter; the real save
    // outcome is observed separately through the public NoteSession state.
    assert!(desktop.handle_event(key(INPUT_KEY_ENTER), &mut volume));
    let save_error = match session.status() {
        SaveStatus::Failed(error) => error,
        status => panic!("ordinary Desktop Enter did not fail at VFS capacity: {status:?}"),
    };
    assert!(matches!(
        &save_error.source,
        StoreError::Io(message) if message.contains("Capacity")
    ));

    let draft = desktop
        .current_notes_document()
        .expect("failed save must leave the open Desktop draft available")
        .clone();
    assert_eq!(draft, expected_draft);
    assert_eq!(draft, session.note());
    assert_eq!(draft.revision, first.revision);
    assert!(session.is_dirty());
    assert!(draft.search_text().contains(&payload));

    let flush_result = session.flush();
    assert!(matches!(
        flush_result,
        Err(SessionError::SaveFailed(ref error)) if error == &save_error
    ));

    assert_eq!(provider.load(NOTE_ID)?, Some(first.clone()));
    assert_eq!(
        provider.load_revision(NOTE_ID, first.revision)?,
        Some(first.clone())
    );
    let failed_revision = first.revision + 1;
    assert_eq!(provider.load_revision(NOTE_ID, failed_revision)?, None);

    drop(desktop);
    drop(session);
    drop(provider);
    drop(volume);

    let (reopened_volume, formatted) = new_linux_development_volume(disk.path(), false)?;
    assert!(!formatted, "valid VFS was reformatted on reopen");
    let reopened = VfsNoteStore::new(reopened_volume);
    assert_eq!(reopened.load(NOTE_ID)?, Some(first.clone()));
    assert_eq!(
        reopened.load_revision(NOTE_ID, first.revision)?,
        Some(first)
    );
    assert_eq!(reopened.load_revision(NOTE_ID, failed_revision)?, None);

    Ok(())
}
