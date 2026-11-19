#![cfg(target_os = "linux")]

use nagi_desktop_notes_integration_candidate::desktop::new_linux_development_volume;
use notes_files_codec_candidate::{NoteDocument, NoteStore, ObjectId, Timestamp, VfsNoteStore};
use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const DISK_BYTES: u64 = 8 * 1024 * 1024;
const ROOT_NOTE_COUNT: usize = 9;
const TARGET_ID: ObjectId = ObjectId(0x4e41_4749_0000_0101);
const FIRST_NOTE_ID: u64 = 0x4e41_4749_0000_0101;
static DISK_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct TestDisk(PathBuf);

impl TestDisk {
    fn create() -> io::Result<Self> {
        let sequence = DISK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nagi-notes-vfs-revisions-{}-{sequence}.raw",
            std::process::id()
        ));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)?;
        // Establish the cleanup guard immediately after create_new so setup
        // failures do not leave an empty file in the guest temp directory.
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

fn revised(previous: &NoteDocument, revision: u64) -> NoteDocument {
    let mut candidate = previous.clone();
    candidate.title = format!("target revision {revision}");
    candidate.updated_at = Timestamp(revision);
    candidate
}

fn assert_revision(
    store: &VfsNoteStore,
    revision: u64,
    context: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let latest = store
        .load(TARGET_ID)?
        .ok_or_else(|| format!("{context}: latest note disappeared"))?;
    assert_eq!(latest.revision, revision);
    assert_eq!(latest.title, format!("target revision {revision}"));

    let first = store
        .load_revision(TARGET_ID, 1)?
        .ok_or_else(|| format!("{context}: revision 1 disappeared"))?;
    assert_eq!(first.revision, 1);
    assert_eq!(first.title, "target revision 1");

    let requested = store
        .load_revision(TARGET_ID, revision)?
        .ok_or_else(|| format!("{context}: revision {revision} disappeared"))?;
    assert_eq!(requested, latest);

    let summaries = store.list(false)?;
    assert_eq!(summaries.len(), ROOT_NOTE_COUNT);
    let summary = summaries
        .iter()
        .find(|summary| summary.id == TARGET_ID)
        .ok_or_else(|| format!("{context}: target note missing from root listing"))?;
    assert_eq!(summary.revision, revision);
    assert_eq!(summary.title, latest.title);
    println!(
        "{context}_PASS latest={revision} old=1 root_notes={}",
        summaries.len()
    );
    Ok(())
}

#[test]
fn nine_and_seventeen_revisions_survive_lookup_and_remount(
) -> Result<(), Box<dyn std::error::Error>> {
    let disk = TestDisk::create()?;
    let (volume, formatted) = new_linux_development_volume(disk.path(), true)?;
    assert!(formatted, "fresh 8 MiB fixture was not formatted");
    let store = VfsNoteStore::new(volume.clone());

    let mut target = None;
    for index in 0..ROOT_NOTE_COUNT {
        let id = ObjectId(FIRST_NOTE_ID + index as u64);
        let note = NoteDocument::new(
            id,
            if id == TARGET_ID {
                "target revision 1"
            } else {
                "root list filler"
            },
            Timestamp(1),
        );
        let saved = store.commit(&note, 0)?;
        if id == TARGET_ID {
            target = Some(saved);
        }
    }

    let mut current = target.ok_or("target note was not seeded")?;
    for revision in 2..=9 {
        current = store.commit(&revised(&current, revision), current.revision)?;
    }
    assert_revision(&store, 9, "NINE_REVISION")?;
    drop(store);
    drop(volume);

    let (volume, formatted) = new_linux_development_volume(disk.path(), false)?;
    assert!(!formatted, "valid VFS was reformatted after revision 9");
    let store = VfsNoteStore::new(volume.clone());
    assert_revision(&store, 9, "NINE_REVISION_REMOUNT")?;

    let mut current = store
        .load(TARGET_ID)?
        .ok_or("target note disappeared before revision 17")?;
    for revision in 10..=17 {
        current = store.commit(&revised(&current, revision), current.revision)?;
    }
    assert_revision(&store, 17, "SEVENTEEN_REVISION")?;
    drop(store);
    drop(volume);

    let (volume, formatted) = new_linux_development_volume(disk.path(), false)?;
    assert!(!formatted, "valid VFS was reformatted after revision 17");
    let store = VfsNoteStore::new(volume);
    assert_revision(&store, 17, "SEVENTEEN_REVISION_REMOUNT")?;
    println!("VFS_NOTE_REVISION_ENUMERATION_REGRESSION=PASS");
    Ok(())
}
