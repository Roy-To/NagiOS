//! Notes revision adapter over the shared Nagi Storage Provider.
//!
//! The provider owns the physical descriptor and serializes every consumer
//! over one VFS instance. This module keeps Notes identity, codec, revision,
//! and immutable-history policy at the Notes boundary.

use super::{
    decode_stored, encode_stored_checked, same_candidate, summary, validate_candidate, NoteStore,
    NoteSummary, StoreError, MAX_STORED_NOTE_BYTES,
};
use crate::domain::NoteDocument;
use crate::identity::{HostObjectIdSource, ObjectIdSource};
use libnagi::storage::{
    DirectoryEntry, StorageError, Vfs, BLOCK_SIZE, MAX_DIRECTORY_ENTRIES, MAX_FILE_SIZE,
};
use nagi_model::ObjectId;
use nagi_storage_provider::{LinuxBlockDevice, SharedVfs};
use std::collections::HashSet;
use std::sync::Mutex;

#[cfg(test)]
use nagi_storage_provider::{
    StorageProviderError, StorageProviderOwner, MIN_VFS_BYTES, SECTOR_SIZE,
};

const NOTES_DIRECTORY: &[u8] = b"/notes";
const DIRECTORY_RECORD_MIN_BYTES: usize = 8;
const MAX_DIRECTORY_LIST_ENTRIES: usize = BLOCK_SIZE / DIRECTORY_RECORD_MIN_BYTES;

struct RevisionIndex {
    committed: Vec<(u64, Vec<u8>)>,
    pending: HashSet<u64>,
}

/// Notes `NoteStore` persisted as immutable revision files inside the same VFS
/// object passed to `Desktop::handle_event`. The adapter uses Notes' stored
/// Markdown codec from this crate, keeping ID/revision/body integrity checks.
///
/// The resource key is `NoteDocument.id` (`ObjectId`), the revision is the
/// unchanged `NoteDocument.revision` (`u64`), and canonical content is the
/// complete document, including its title and blocks. `search_text()` is only
/// a derived view. `commit` requires the caller's expected prior revision and
/// returns a document only after the VFS publish and cleanup flushes succeed.
/// A `StoreError` is not a success acknowledgement or permission to replay an
/// Action; the UI remains responsible for preserving its dirty session.
pub struct VfsNoteStore {
    volume: SharedVfs,
    ids: HostObjectIdSource,
    gate: Mutex<()>,
}

impl VfsNoteStore {
    pub fn new(volume: SharedVfs) -> Self {
        Self {
            volume,
            ids: HostObjectIdSource::new(),
            gate: Mutex::new(()),
        }
    }

    fn note_directory(id: ObjectId) -> Vec<u8> {
        format!("/notes/{:016x}", id.0).into_bytes()
    }

    fn revision_path(id: ObjectId, revision: u64) -> Vec<u8> {
        format!("/notes/{:016x}/rev-{revision:020}.md", id.0).into_bytes()
    }

    fn pending_revision_path(id: ObjectId, revision: u64) -> Vec<u8> {
        // Keep the prepare marker compact so the largest revision directory
        // can fit both the marker and the revision being published in one
        // VFS directory block.
        format!("/notes/{:016x}/p{revision:016x}", id.0).into_bytes()
    }

    fn list_directory_entries(
        volume: &mut Vfs<LinuxBlockDevice>,
        directory: &[u8],
    ) -> Result<Vec<DirectoryEntry>, StoreError> {
        let mut capacity = MAX_DIRECTORY_ENTRIES.min(MAX_DIRECTORY_LIST_ENTRIES);
        let mut entries = Vec::new();

        loop {
            entries.try_reserve_exact(capacity).map_err(|error| {
                StoreError::Io(format!("allocate bounded VFS directory buffer: {error}"))
            })?;
            entries.resize(capacity, DirectoryEntry::empty());

            match volume.list_directory_path(directory, &mut entries) {
                Ok(count) => {
                    entries.truncate(count);
                    return Ok(entries);
                }
                Err(StorageError::BufferTooSmall) if capacity < MAX_DIRECTORY_LIST_ENTRIES => {
                    entries.clear();
                    let next_capacity = capacity.saturating_mul(2).min(MAX_DIRECTORY_LIST_ENTRIES);
                    if next_capacity <= capacity {
                        return Err(storage_error(StorageError::BufferTooSmall));
                    }
                    capacity = next_capacity;
                }
                Err(error) => return Err(storage_error(error)),
            }
        }
    }

    fn versions(
        volume: &mut Vfs<LinuxBlockDevice>,
        id: ObjectId,
    ) -> Result<RevisionIndex, StoreError> {
        let directory = Self::note_directory(id);
        match volume.metadata_path(&directory) {
            Ok(metadata) if metadata.mode & 0xf000 == 0x4000 => {}
            Ok(_) => {
                return Err(StoreError::SandboxViolation(
                    "VFS note identity is not a directory".to_owned(),
                ));
            }
            Err(StorageError::NotFound) => {
                return Ok(RevisionIndex {
                    committed: Vec::new(),
                    pending: HashSet::new(),
                });
            }
            Err(error) => return Err(storage_error(error)),
        }

        let entries = Self::list_directory_entries(volume, &directory)?;
        let mut committed = Vec::new();
        let mut pending = HashSet::new();
        for entry in &entries {
            let name = std::str::from_utf8(entry.name())
                .map_err(|error| StoreError::Corrupt(error.to_string()))?;
            let (revision_text, is_pending) = if let Some(revision) = name
                .strip_prefix("rev-")
                .and_then(|value| value.strip_suffix(".md"))
            {
                (revision, false)
            } else if let Some(revision) = name.strip_prefix('p') {
                (revision, true)
            } else {
                return Err(StoreError::Corrupt(
                    "unrecognized entry in VFS note directory".to_owned(),
                ));
            };
            let revision = if is_pending {
                if revision_text.len() != 16
                    || !revision_text.bytes().all(|byte| byte.is_ascii_hexdigit())
                {
                    return Err(StoreError::Corrupt(
                        "invalid VFS pending revision name".to_owned(),
                    ));
                }
                u64::from_str_radix(revision_text, 16)
                    .map_err(|error| StoreError::Corrupt(error.to_string()))?
            } else {
                if revision_text.len() != 20
                    || !revision_text.bytes().all(|byte| byte.is_ascii_digit())
                {
                    return Err(StoreError::Corrupt(
                        "invalid VFS note revision name".to_owned(),
                    ));
                }
                revision_text
                    .parse::<u64>()
                    .map_err(|error| StoreError::Corrupt(error.to_string()))?
            };
            if is_pending {
                if !pending.insert(revision) {
                    return Err(StoreError::Corrupt(
                        "duplicate pending VFS note revision".to_owned(),
                    ));
                }
            } else {
                committed.push((
                    revision,
                    [directory.as_slice(), b"/", name.as_bytes()].concat(),
                ));
            }
        }
        committed.sort_by_key(|(revision, _)| *revision);
        if committed.windows(2).any(|pair| pair[0].0 == pair[1].0) {
            return Err(StoreError::Corrupt(
                "duplicate VFS note revision".to_owned(),
            ));
        }
        Ok(RevisionIndex { committed, pending })
    }

    fn read_file(
        volume: &mut Vfs<LinuxBlockDevice>,
        id: ObjectId,
        revision: u64,
        path: &[u8],
        ids: &dyn ObjectIdSource,
    ) -> Result<Option<NoteDocument>, StoreError> {
        let metadata = match volume.metadata_path(path) {
            Ok(metadata) => metadata,
            Err(StorageError::NotFound) => return Ok(None),
            Err(error) => return Err(storage_error(error)),
        };
        if metadata.mode & 0xf000 != 0x8000 {
            return Err(StoreError::SandboxViolation(
                "VFS note revision is not a regular file".to_owned(),
            ));
        }
        let length = metadata.size as usize;
        if length > MAX_FILE_SIZE || length > MAX_STORED_NOTE_BYTES {
            return Err(StoreError::Corrupt(
                "stored note exceeds VFS capacity".to_owned(),
            ));
        }
        let handle = volume.open_path(path).map_err(storage_error)?;
        let mut bytes = vec![0; length];
        let read = volume.read(handle, &mut bytes).map_err(storage_error)?;
        if read != length {
            return Err(StoreError::Corrupt(
                "VFS note length changed during read".to_owned(),
            ));
        }
        let input = std::str::from_utf8(&bytes)
            .map_err(|error| StoreError::Corrupt(format!("note is not valid UTF-8: {error}")))?;
        let note = decode_stored(input, ids)?;
        if note.id != id || note.revision != revision {
            return Err(StoreError::Corrupt(
                "VFS note identity does not match its revision path".to_owned(),
            ));
        }
        validate_candidate(&note, revision)
            .map_err(|error| StoreError::Corrupt(error.to_string()))?;
        Ok(Some(note))
    }

    fn latest(
        volume: &mut Vfs<LinuxBlockDevice>,
        id: ObjectId,
        ids: &dyn ObjectIdSource,
    ) -> Result<Option<NoteDocument>, StoreError> {
        let versions = Self::versions(volume, id)?;
        for (revision, path) in versions.committed.iter().rev() {
            match Self::read_indexed_revision(
                volume,
                id,
                *revision,
                path,
                versions.pending.contains(revision),
                ids,
            )? {
                Some(note) => return Ok(Some(note)),
                None => continue,
            }
        }
        Ok(None)
    }

    fn read_indexed_revision(
        volume: &mut Vfs<LinuxBlockDevice>,
        id: ObjectId,
        revision: u64,
        path: &[u8],
        has_pending_marker: bool,
        ids: &dyn ObjectIdSource,
    ) -> Result<Option<NoteDocument>, StoreError> {
        let note = match Self::read_file(volume, id, revision, path, ids) {
            Ok(note) => note,
            Err(StoreError::Corrupt(_)) if has_pending_marker => return Ok(None),
            Err(error) => return Err(error),
        };
        let Some(note) = note else {
            return Ok(None);
        };
        if !has_pending_marker {
            return Ok(Some(note));
        }

        let pending_path = Self::pending_revision_path(id, revision);
        match Self::read_file(volume, id, revision, &pending_path, ids) {
            Ok(Some(candidate)) if candidate == note => Ok(Some(note)),
            Ok(_) | Err(StoreError::Corrupt(_)) => Err(StoreError::Corrupt(
                "published VFS note has no matching prepared revision".to_owned(),
            )),
            Err(error) => Err(error),
        }
    }

    fn ensure_directory(volume: &mut Vfs<LinuxBlockDevice>, path: &[u8]) -> Result<(), StoreError> {
        match volume.ensure_directory_path(path) {
            Ok(()) => Ok(()),
            Err(error) => Err(storage_error(error)),
        }
    }

    fn install_revision(
        volume: &mut Vfs<LinuxBlockDevice>,
        note: &NoteDocument,
        revision: u64,
    ) -> Result<(), StoreError> {
        let mut saved = note.clone();
        saved.revision = revision;
        let contents = encode_stored_checked(&saved).map_err(StoreError::InvalidDocument)?;
        if contents.len() > MAX_FILE_SIZE || contents.len() > MAX_STORED_NOTE_BYTES {
            return Err(StoreError::InvalidDocument(
                "stored note exceeds the VFS file capacity",
            ));
        }
        Self::ensure_directory(volume, NOTES_DIRECTORY)?;
        Self::ensure_directory(volume, &Self::note_directory(note.id))?;
        let path = Self::revision_path(note.id, revision);
        let index = Self::versions(volume, note.id)?;
        let pending_exists = index.pending.contains(&revision);
        let final_exists = match volume.metadata_path(&path) {
            Ok(_) => true,
            Err(StorageError::NotFound) => false,
            Err(error) => return Err(storage_error(error)),
        };
        if final_exists && !pending_exists {
            return Err(StoreError::RevisionConflict {
                expected: revision.saturating_sub(1),
                actual: revision,
            });
        }

        // `commit` reaches this point only after `latest` returned the prior
        // revision. A pending marker therefore makes a same-number final file
        // an incomplete, unacknowledged attempt; remove only those artifacts,
        // never a revision with no pending marker.
        if final_exists {
            volume.remove_path(&path).map_err(storage_error)?;
        }
        if pending_exists {
            volume
                .remove_path(&Self::pending_revision_path(note.id, revision))
                .map_err(storage_error)?;
        }
        if final_exists || pending_exists {
            volume.flush().map_err(storage_error)?;
        }

        let pending_path = Self::pending_revision_path(note.id, revision);
        let pending_handle = volume.create_path(&pending_path).map_err(storage_error)?;
        powercut_checkpoint("after-pending-create-before-write");
        volume
            .write(pending_handle, contents.as_bytes())
            .map_err(storage_error)?;
        powercut_checkpoint("after-pending-write-before-flush");
        volume.flush().map_err(storage_error)?;
        powercut_checkpoint("after-pending-flush-before-publish");

        let handle = volume.create_path(&path).map_err(storage_error)?;
        powercut_checkpoint("after-create-before-write");
        volume
            .write(handle, contents.as_bytes())
            .map_err(storage_error)?;
        powercut_checkpoint("after-write-before-flush");
        volume.flush().map_err(storage_error)?;
        powercut_checkpoint("after-flush-before-pending-remove");

        Self::cleanup_pending_through(volume, note.id, revision)?;
        powercut_checkpoint("after-flush-before-ack");
        Ok(())
    }

    fn cleanup_pending_through(
        volume: &mut Vfs<LinuxBlockDevice>,
        id: ObjectId,
        revision: u64,
    ) -> Result<(), StoreError> {
        let index = Self::versions(volume, id)?;
        for pending_revision in index
            .pending
            .into_iter()
            .filter(|pending| *pending <= revision)
        {
            volume
                .remove_path(&Self::pending_revision_path(id, pending_revision))
                .map_err(storage_error)?;
        }
        // Keep the acknowledgement boundary even when retrying an idempotent
        // commit: a previous publish may have been visible but not flushed.
        volume.flush().map_err(storage_error)
    }
}

impl NoteStore for VfsNoteStore {
    fn list(&self, include_deleted: bool) -> Result<Vec<NoteSummary>, StoreError> {
        self.volume.with_volume(|volume| {
            let metadata = match volume.metadata_path(NOTES_DIRECTORY) {
                Ok(metadata) if metadata.mode & 0xf000 == 0x4000 => metadata,
                Ok(_) => {
                    return Err(StoreError::SandboxViolation(
                        "VFS Notes root is not a directory".to_owned(),
                    ));
                }
                Err(StorageError::NotFound) => return Ok(Vec::new()),
                Err(error) => return Err(storage_error(error)),
            };
            let _ = metadata;
            let entries = Self::list_directory_entries(volume, NOTES_DIRECTORY)?;
            let mut summaries = Vec::new();
            for entry in &entries {
                let name = std::str::from_utf8(entry.name())
                    .map_err(|error| StoreError::Corrupt(error.to_string()))?;
                if name.len() != 16 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(StoreError::Corrupt(
                        "invalid VFS note identity directory".to_owned(),
                    ));
                }
                let id = u64::from_str_radix(name, 16)
                    .map_err(|error| StoreError::Corrupt(error.to_string()))?;
                let note = Self::latest(volume, ObjectId(id), &self.ids)?;
                if let Some(note) = note {
                    if include_deleted || !note.deleted {
                        summaries.push(summary(&note));
                    }
                }
            }
            summaries.sort_by(|left, right| {
                right
                    .updated_at
                    .cmp(&left.updated_at)
                    .then_with(|| left.id.0.cmp(&right.id.0))
            });
            Ok(summaries)
        })
    }

    fn load(&self, id: ObjectId) -> Result<Option<NoteDocument>, StoreError> {
        self.volume
            .with_volume(|volume| Self::latest(volume, id, &self.ids))
    }

    fn load_revision(
        &self,
        id: ObjectId,
        revision: u64,
    ) -> Result<Option<NoteDocument>, StoreError> {
        self.volume.with_volume(|volume| {
            let versions = Self::versions(volume, id)?;
            let Some((_, path)) = versions
                .committed
                .iter()
                .find(|(number, _)| *number == revision)
            else {
                return Ok(None);
            };
            Self::read_indexed_revision(
                volume,
                id,
                revision,
                path,
                versions.pending.contains(&revision),
                &self.ids,
            )
        })
    }

    fn commit(
        &self,
        note: &NoteDocument,
        expected_revision: u64,
    ) -> Result<NoteDocument, StoreError> {
        validate_candidate(note, expected_revision)?;
        let _guard = self
            .gate
            .lock()
            .map_err(|_| StoreError::Io("VFS Notes lock poisoned".to_owned()))?;
        self.volume.with_volume(|volume| {
            let current = Self::latest(volume, note.id, &self.ids)?;
            let actual = current
                .as_ref()
                .map(|document| document.revision)
                .unwrap_or(0);
            if actual != expected_revision {
                if actual == expected_revision.saturating_add(1)
                    && current
                        .as_ref()
                        .is_some_and(|saved| same_candidate(saved, note))
                {
                    let saved = current.expect("checked above");
                    // This revision may be visible only because an earlier
                    // publish flush failed. Make it durable while its pending
                    // marker still exists before allowing cleanup to remove it.
                    volume.flush().map_err(storage_error)?;
                    Self::cleanup_pending_through(volume, note.id, saved.revision)?;
                    return Ok(saved);
                }
                return Err(StoreError::RevisionConflict {
                    expected: expected_revision,
                    actual,
                });
            }
            let revision = expected_revision
                .checked_add(1)
                .ok_or(StoreError::RevisionOverflow)?;
            Self::install_revision(volume, note, revision)?;
            let mut saved = note.clone();
            saved.revision = revision;
            Ok(saved)
        })
    }
}

fn storage_error(error: StorageError) -> StoreError {
    StoreError::Io(format!("Nagi VFS operation failed: {error:?}"))
}

#[cfg(feature = "powercut-test-hooks")]
fn powercut_checkpoint(stage: &str) {
    use std::io::Write;

    if std::fs::read("/powercut-hook")
        .ok()
        .is_some_and(|contents| contents == stage.as_bytes())
    {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        let _ = writeln!(stdout, "POWERCUT_REACHED={stage}");
        let _ = stdout.flush();
        loop {
            std::thread::park();
        }
    }
}

#[cfg(not(feature = "powercut-test-hooks"))]
fn powercut_checkpoint(_stage: &str) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct FailOneDirectoryBufferAllocation;

    static DIRECTORY_BUFFER_ALLOCATION_OBSERVED: AtomicBool = AtomicBool::new(false);
    static DIRECTORY_BUFFER_LAYOUT_SIZE: AtomicUsize = AtomicUsize::new(0);
    static DIRECTORY_BUFFER_LAYOUT_ALIGN: AtomicUsize = AtomicUsize::new(0);

    thread_local! {
        static FAIL_NEXT_DIRECTORY_BUFFER: Cell<bool> = const { Cell::new(false) };
    }

    unsafe impl GlobalAlloc for FailOneDirectoryBufferAllocation {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let target_layout = layout.size()
                == DIRECTORY_BUFFER_LAYOUT_SIZE.load(Ordering::SeqCst)
                && layout.align() == DIRECTORY_BUFFER_LAYOUT_ALIGN.load(Ordering::SeqCst);
            let fail_this_allocation = target_layout
                && FAIL_NEXT_DIRECTORY_BUFFER
                    .try_with(|armed| armed.replace(false))
                    .unwrap_or(false);
            if fail_this_allocation {
                DIRECTORY_BUFFER_ALLOCATION_OBSERVED.store(true, Ordering::SeqCst);
                return std::ptr::null_mut();
            }
            // SAFETY: forwards the exact layout to the system allocator.
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            // SAFETY: this allocator pairs every successful allocation with System.
            unsafe { System.dealloc(pointer, layout) }
        }

        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
            // SAFETY: forwards the original allocation and requested size to System.
            unsafe { System.realloc(pointer, layout, new_size) }
        }
    }

    #[global_allocator]
    static TEST_ALLOCATOR: FailOneDirectoryBufferAllocation = FailOneDirectoryBufferAllocation;

    fn temp_path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "nagi-notes-{name}-{}-{}.img",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn create_blank_disk(path: &Path) {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        file.set_len(MIN_VFS_BYTES).unwrap();
        file.sync_all().unwrap();
    }

    fn mount_vfs(
        path: &Path,
    ) -> Result<(StorageProviderOwner, SharedVfs, bool), StorageProviderError> {
        let (owner, formatted) = StorageProviderOwner::mount_or_format(path)?;
        let consumer = owner.consumer_handle();
        Ok((owner, consumer, formatted))
    }

    #[test]
    fn incomplete_published_revision_is_hidden_and_explicit_retry_finishes_it() {
        let path = temp_path("incomplete-revision");
        create_blank_disk(&path);
        let (owner, shared, formatted) = mount_vfs(&path).unwrap();
        assert!(formatted);
        let store = VfsNoteStore::new(shared.clone());
        let id = ObjectId(0x4e41_4749_0000_0191);
        let initial = NoteDocument::new(id, "durable old", crate::domain::Timestamp(1));
        let old = store.commit(&initial, 0).unwrap();

        let mut candidate = old.clone();
        candidate.title = "candidate after interrupted write".to_owned();
        candidate.updated_at = crate::domain::Timestamp(2);
        let mut prepared = candidate.clone();
        prepared.revision = 2;
        let bytes = encode_stored_checked(&prepared).unwrap();
        let pending_path = VfsNoteStore::pending_revision_path(id, 2);
        let final_path = VfsNoteStore::revision_path(id, 2);
        shared
            .with_volume(|volume| {
                let pending = volume.create_path(&pending_path)?;
                volume.write(pending, bytes.as_bytes())?;
                volume.flush()?;
                let _incomplete_final = volume.create_path(&final_path)?;
                volume.flush()
            })
            .unwrap();

        drop(store);
        drop(shared);
        drop(owner);
        let (reopen_owner, reopened, formatted) = mount_vfs(&path).unwrap();
        assert!(!formatted);
        let store = VfsNoteStore::new(reopened.clone());
        assert_eq!(store.load(id).unwrap(), Some(old.clone()));
        assert_eq!(store.load_revision(id, 2).unwrap(), None);

        let saved = store.commit(&candidate, 1).unwrap();
        assert_eq!(saved.revision, 2);
        assert_eq!(saved.title, candidate.title);
        assert_eq!(store.load(id).unwrap(), Some(saved.clone()));
        assert_eq!(store.load_revision(id, 1).unwrap(), Some(old));

        drop(store);
        drop(reopened);
        drop(reopen_owner);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn later_revision_reaps_pending_markers_left_by_publish_flush_failure() {
        let path = temp_path("reap-stale-pending-revision");
        create_blank_disk(&path);
        let (owner, shared, formatted) = mount_vfs(&path).unwrap();
        assert!(formatted);
        let store = VfsNoteStore::new(shared.clone());
        let id = ObjectId(0x4e41_4749_0000_0194);
        let initial = NoteDocument::new(id, "revision one", crate::domain::Timestamp(1));
        let old = store.commit(&initial, 0).unwrap();

        let mut second = old.clone();
        second.title = "revision two published without acknowledgement".to_owned();
        second.updated_at = crate::domain::Timestamp(2);
        nagi_storage_provider::fail_flush_after_for_test(1);
        assert!(store.commit(&second, 1).is_err());
        let observed_second = store
            .load(id)
            .unwrap()
            .expect("published and prepared revision remains readable in the live VFS");
        assert_eq!(observed_second.revision, 2);
        assert_eq!(observed_second.title, second.title);

        let mut third = observed_second.clone();
        third.title = "revision three".to_owned();
        third.updated_at = crate::domain::Timestamp(3);
        let saved = store.commit(&third, 2).unwrap();
        assert_eq!(saved.revision, 3);
        assert_eq!(store.load(id).unwrap(), Some(saved.clone()));
        assert_eq!(store.load_revision(id, 1).unwrap(), Some(old));
        assert_eq!(store.load_revision(id, 2).unwrap(), Some(observed_second));

        let pending = shared
            .with_volume(|volume| {
                let index = VfsNoteStore::versions(volume, id)?;
                Ok::<_, StoreError>(index.pending)
            })
            .unwrap();
        assert!(
            pending.is_empty(),
            "successful revision 3 must reap pending-2 and pending-3"
        );

        let revision_two = VfsNoteStore::revision_path(id, 2);
        shared
            .with_volume(|volume| {
                let handle = volume.open_path(&revision_two)?;
                volume.truncate(handle, 0)?;
                volume.write(handle, b"corrupt revision")?;
                volume.flush()
            })
            .unwrap();
        assert!(matches!(
            store.load_revision(id, 2),
            Err(StoreError::Corrupt(_))
        ));

        drop(store);
        drop(shared);
        drop(owner);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn idempotent_retry_flushes_publish_before_pending_cleanup() {
        let path = temp_path("idempotent-publish-barrier");
        create_blank_disk(&path);
        let (owner, shared, formatted) = mount_vfs(&path).unwrap();
        assert!(formatted);
        let store = VfsNoteStore::new(shared.clone());
        let id = ObjectId(0x4e41_4749_0000_01a1);
        let initial = NoteDocument::new(id, "old durable value", crate::domain::Timestamp(1));
        let old = store.commit(&initial, 0).unwrap();

        let mut candidate = old.clone();
        candidate.title = "publish flush failed; retry same commit".to_owned();
        candidate.updated_at = crate::domain::Timestamp(2);

        // The prepare flush succeeds and the first publish flush fails.
        nagi_storage_provider::fail_flush_after_for_test(1);
        assert!(store.commit(&candidate, 1).is_err());
        let pending_after_publish_failure = shared
            .with_volume(|volume| {
                let index = VfsNoteStore::versions(volume, id)?;
                Ok::<_, StoreError>(index.pending)
            })
            .unwrap();
        assert!(pending_after_publish_failure.contains(&2));

        // On an idempotent retry the publish barrier fails. Cleanup must not
        // run before this barrier succeeds, so pending-2 must still be present.
        nagi_storage_provider::fail_flush_after_for_test(0);
        assert!(store.commit(&candidate, 1).is_err());
        let pending_after_retry_barrier_failure = shared
            .with_volume(|volume| {
                let index = VfsNoteStore::versions(volume, id)?;
                Ok::<_, StoreError>(index.pending)
            })
            .unwrap();
        assert!(
            pending_after_retry_barrier_failure.contains(&2),
            "failed publish barrier must leave pending-2 in place"
        );

        // Only after a successful barrier may the idempotent retry reap it.
        let saved = store.commit(&candidate, 1).unwrap();
        assert_eq!(saved.revision, 2);
        assert_eq!(saved.title, candidate.title);
        assert_eq!(store.load(id).unwrap(), Some(saved));
        let pending_after_success = shared
            .with_volume(|volume| {
                let index = VfsNoteStore::versions(volume, id)?;
                Ok::<_, StoreError>(index.pending)
            })
            .unwrap();
        assert!(pending_after_success.is_empty());

        drop(store);
        drop(shared);
        drop(owner);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn matching_prepared_and_published_revisions_reopen_as_new() {
        let path = temp_path("published-revision-pending-marker");
        create_blank_disk(&path);
        let (owner, shared, formatted) = mount_vfs(&path).unwrap();
        assert!(formatted);
        let store = VfsNoteStore::new(shared.clone());
        let id = ObjectId(0x4e41_4749_0000_0192);
        let initial = NoteDocument::new(id, "old", crate::domain::Timestamp(1));
        let old = store.commit(&initial, 0).unwrap();
        let mut candidate = old.clone();
        candidate.title = "fully written new".to_owned();
        candidate.revision = 2;
        candidate.updated_at = crate::domain::Timestamp(2);
        let bytes = encode_stored_checked(&candidate).unwrap();
        let pending_path = VfsNoteStore::pending_revision_path(id, 2);
        let final_path = VfsNoteStore::revision_path(id, 2);
        shared
            .with_volume(|volume| {
                let pending = volume.create_path(&pending_path)?;
                volume.write(pending, bytes.as_bytes())?;
                volume.flush()?;
                let published = volume.create_path(&final_path)?;
                volume.write(published, bytes.as_bytes())?;
                volume.flush()
            })
            .unwrap();

        drop(store);
        drop(shared);
        drop(owner);
        let (reopen_owner, reopened, formatted) = mount_vfs(&path).unwrap();
        assert!(!formatted);
        let store = VfsNoteStore::new(reopened.clone());
        assert_eq!(store.load(id).unwrap(), Some(candidate.clone()));
        assert_eq!(store.load_revision(id, 2).unwrap(), Some(candidate));

        drop(store);
        drop(reopened);
        drop(reopen_owner);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn mismatched_prepared_revision_is_unknown_instead_of_silently_falling_back() {
        let path = temp_path("mismatched-prepared-revision");
        create_blank_disk(&path);
        let (owner, shared, formatted) = mount_vfs(&path).unwrap();
        assert!(formatted);
        let store = VfsNoteStore::new(shared.clone());
        let id = ObjectId(0x4e41_4749_0000_0193);
        let initial = NoteDocument::new(id, "old", crate::domain::Timestamp(1));
        let _old = store.commit(&initial, 0).unwrap();

        let mut prepared = NoteDocument::new(id, "prepared candidate", crate::domain::Timestamp(2));
        prepared.revision = 2;
        let prepared_bytes = encode_stored_checked(&prepared).unwrap();
        let mut published = prepared.clone();
        published.title = "different published bytes".to_owned();
        let published_bytes = encode_stored_checked(&published).unwrap();
        let pending_path = VfsNoteStore::pending_revision_path(id, 2);
        let final_path = VfsNoteStore::revision_path(id, 2);
        shared
            .with_volume(|volume| {
                let pending = volume.create_path(&pending_path)?;
                volume.write(pending, prepared_bytes.as_bytes())?;
                let final_handle = volume.create_path(&final_path)?;
                volume.write(final_handle, published_bytes.as_bytes())?;
                volume.flush()
            })
            .unwrap();
        assert!(matches!(store.load(id), Err(StoreError::Corrupt(_))));

        drop(store);
        drop(shared);
        drop(owner);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn directory_buffer_reservation_failure_returns_error_without_partial_load() {
        let path = temp_path("directory-allocation-failure");
        create_blank_disk(&path);
        let (owner, shared, formatted) = mount_vfs(&path).unwrap();
        assert!(formatted);
        let store = VfsNoteStore::new(shared.clone());
        let id = ObjectId(0x4e41_4749_0000_0101);
        let initial = NoteDocument::new(id, "allocation failure", crate::domain::Timestamp(1));
        let saved = store.commit(&initial, 0).unwrap();

        let layout = Layout::array::<DirectoryEntry>(MAX_DIRECTORY_ENTRIES).unwrap();
        DIRECTORY_BUFFER_LAYOUT_SIZE.store(layout.size(), Ordering::SeqCst);
        DIRECTORY_BUFFER_LAYOUT_ALIGN.store(layout.align(), Ordering::SeqCst);
        DIRECTORY_BUFFER_ALLOCATION_OBSERVED.store(false, Ordering::SeqCst);
        FAIL_NEXT_DIRECTORY_BUFFER.with(|armed| armed.set(true));

        let failed_load = store.load(id);
        FAIL_NEXT_DIRECTORY_BUFFER.with(|armed| armed.set(false));
        assert!(
            DIRECTORY_BUFFER_ALLOCATION_OBSERVED.load(Ordering::SeqCst),
            "the test allocator did not intercept the directory buffer reservation"
        );
        assert!(matches!(
            &failed_load,
            Err(StoreError::Io(message))
                if message.contains("allocate bounded VFS directory buffer")
        ));
        assert_eq!(store.load(id).unwrap(), Some(saved));

        drop(store);
        drop(shared);
        drop(owner);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn bounded_listing_propagates_vfs_errors_and_near_full_revision_directory() {
        let path = temp_path("directory-upper-bound");
        create_blank_disk(&path);
        let (owner, shared, formatted) = mount_vfs(&path).unwrap();
        assert!(formatted);
        let store = VfsNoteStore::new(shared.clone());
        let id = ObjectId(0x4e41_4749_0000_0102);
        let initial = NoteDocument::new(id, "target revision 1", crate::domain::Timestamp(1));
        let mut saved = store.commit(&initial, 0).unwrap();

        let missing = shared
            .with_volume(|volume| VfsNoteStore::list_directory_entries(volume, b"/notes/missing"))
            .unwrap_err();
        assert!(matches!(
            &missing,
            StoreError::Io(message) if message.contains("NotFound")
        ));

        for revision in 2..=27 {
            let mut candidate = saved.clone();
            candidate.title = format!("target revision {revision}");
            candidate.updated_at = crate::domain::Timestamp(revision);
            saved = store.commit(&candidate, saved.revision).unwrap_or_else(|error| {
                panic!("target revision {revision} failed before the VFS limit: {error}")
            });
        }
        assert_eq!(saved.revision, 27);
        assert_eq!(store.load(id).unwrap(), Some(saved.clone()));
        assert_eq!(store.load_revision(id, 1).unwrap().unwrap().revision, 1);
        assert_eq!(
            MAX_DIRECTORY_LIST_ENTRIES,
            BLOCK_SIZE / DIRECTORY_RECORD_MIN_BYTES
        );

        let mut over_limit = saved.clone();
        over_limit.title = "target revision 28".to_owned();
        over_limit.updated_at = crate::domain::Timestamp(28);
        let error = store.commit(&over_limit, saved.revision).unwrap_err();
        assert!(matches!(
            &error,
            StoreError::Io(message) if message.contains("DirectoryFull")
        ));
        assert_eq!(store.load(id).unwrap(), Some(saved.clone()));
        assert_eq!(store.load_revision(id, 28).unwrap(), None);

        drop(store);
        drop(shared);
        drop(owner);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn vfs_notes_revisions_survive_file_backed_remount_and_reject_stale_writes() {
        let path = temp_path("vfs-revisions");
        create_blank_disk(&path);
        let (owner, shared, formatted) = mount_vfs(&path).unwrap();
        assert!(formatted);
        let store = VfsNoteStore::new(shared.clone());
        let id = ObjectId(0x4e41_4749_0000_0001);
        let initial = NoteDocument::new(id, "VFS Notes", crate::domain::Timestamp(1));
        let first = store.commit(&initial, 0).unwrap();
        assert_eq!(first.revision, 1);
        let mut second_candidate = first.clone();
        second_candidate.blocks.push(crate::domain::Block::new(
            ObjectId(0x424c_4f43_4b00_0001),
            crate::domain::BlockKind::Paragraph("persisted".to_owned()),
        ));
        let second = store.commit(&second_candidate, 1).unwrap();
        assert_eq!(second.revision, 2);
        assert_eq!(store.commit(&second_candidate, 1).unwrap(), second.clone());
        let mut conflicting_stale = second_candidate.clone();
        conflicting_stale.title = "conflicting stale title".to_owned();
        assert!(matches!(
            store.commit(&conflicting_stale, 1),
            Err(StoreError::RevisionConflict {
                expected: 1,
                actual: 2
            })
        ));

        drop(store);
        drop(shared);
        drop(owner);
        let before_reopen = std::fs::read(&path).unwrap();
        let (remount_owner, remounted, formatted) = mount_vfs(&path).unwrap();
        assert!(
            !formatted,
            "a valid nonblank VFS must be mounted, not reformatted"
        );
        let reopened = VfsNoteStore::new(remounted);
        assert_eq!(reopened.load(id).unwrap(), Some(second.clone()));
        assert_eq!(reopened.load_revision(id, 1).unwrap(), Some(first));
        drop(reopened);
        drop(remount_owner);
        assert_eq!(std::fs::read(&path).unwrap(), before_reopen);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn mismatched_stored_note_identity_is_rejected_by_revision_path() {
        let path = temp_path("codec-mismatch");
        create_blank_disk(&path);
        let (owner, shared, formatted) = mount_vfs(&path).unwrap();
        assert!(formatted);

        let expected_id = ObjectId(0x4e41_4749_0000_0041);
        let wrong_id = ObjectId(0x4e41_4749_0000_0042);
        let mut wrong_note =
            NoteDocument::new(wrong_id, "Wrong identity", crate::domain::Timestamp(1));
        wrong_note.revision = 1;
        let encoded = encode_stored_checked(&wrong_note).unwrap();
        shared
            .with_volume(|volume| {
                volume.ensure_directory_path(NOTES_DIRECTORY)?;
                volume.ensure_directory_path(&VfsNoteStore::note_directory(expected_id))?;
                let revision_path = VfsNoteStore::revision_path(expected_id, 1);
                let handle = volume.create_path(&revision_path)?;
                volume.write(handle, encoded.as_bytes())?;
                volume.flush()
            })
            .unwrap();

        let store = VfsNoteStore::new(shared.clone());
        assert!(matches!(
            store.load(expected_id),
            Err(StoreError::Corrupt(_))
        ));
        drop(store);
        drop(shared);
        drop(owner);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn undersized_development_disk_is_rejected_without_formatting() {
        let path = temp_path("short-vfs");
        let original = vec![0x5au8; 2048];
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.write_all(&original).unwrap();
        file.sync_all().unwrap();
        drop(file);

        assert!(StorageProviderOwner::mount_or_format(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), original);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn nonblank_invalid_development_disk_is_rejected_without_formatting() {
        let path = temp_path("invalid-vfs");
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        file.set_len(MIN_VFS_BYTES).unwrap();
        file.write_all(&[0x5a; SECTOR_SIZE]).unwrap();
        file.sync_all().unwrap();
        drop(file);

        let before = std::fs::read(&path).unwrap();
        assert!(StorageProviderOwner::mount_or_format(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        std::fs::remove_file(path).unwrap();
    }
}
