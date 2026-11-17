use super::{
    decode_stored, encode_stored_checked, same_candidate, summary, validate_candidate, NoteStore,
    NoteSummary, ObjectId, StoreError, MAX_STORED_NOTE_BYTES, TEMP_SEQUENCE,
};
use crate::domain::NoteDocument;
use crate::identity::{HostObjectIdSource, ObjectIdSource};
use crate::markdown::MarkdownError;
use rustix::fs::{
    fstat, fsync, linkat, mkdirat, openat, statat, unlinkat, AtFlags, Dir, FileType, Mode, OFlags,
};
use rustix::io::Errno;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

#[cfg(test)]
std::thread_local! {
    static FAIL_NEXT_REVISION_DIRECTORY_FSYNC: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Linux `NoteStore` rooted at a caller-supplied directory descriptor.
///
/// The caller must resolve and authorize the directory before passing its open
/// descriptor. This adapter does not issue application authority, resolve a
/// path, create the root, or map `UserDataVolume` capabilities. All child
/// operations are relative to the retained descriptor and reject symlinks.
pub struct LinuxDirectoryNoteStore {
    root: OwnedFd,
    ids: Arc<dyn ObjectIdSource>,
    gate: Mutex<()>,
}

impl LinuxDirectoryNoteStore {
    /// Consume an already-open directory fd supplied by the authorized caller.
    /// A caller may pass an `O_PATH` directory fd; this function opens and retains
    /// a read-capable descriptor relative to it. Write access is checked by the
    /// OS on each mutation. This function never resolves or creates a root path.
    pub fn from_root_fd(root: OwnedFd) -> Result<Self, StoreError> {
        let metadata = fstat(&root).map_err(io_error)?;
        if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
            return Err(StoreError::SandboxViolation(
                "root handle is not a directory".to_owned(),
            ));
        }
        // Retain the read-capable probe. Keeping the caller's original O_PATH fd
        // would make Dir::read_from and fsync fail after this constructor returned.
        let probe = openat(
            &root,
            ".",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(io_error)?;
        Ok(Self {
            root: probe,
            ids: Arc::new(HostObjectIdSource::new()),
            gate: Mutex::new(()),
        })
    }

    fn note_dir(&self, id: ObjectId, create: bool) -> Result<Option<OwnedFd>, StoreError> {
        let name = format!("{:016x}", id.0);
        if create {
            match mkdirat(&self.root, name.as_str(), Mode::from_raw_mode(0o700)) {
                Ok(()) | Err(Errno::EXIST) => {}
                Err(error) => return Err(io_error(error)),
            }
            // Also sync on EEXIST: another store instance may have just created
            // this entry and reached its first child write before syncing root.
            fsync(&self.root).map_err(io_error)?;
        }
        let directory = match openat(
            &self.root,
            name.as_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(directory) => directory,
            Err(Errno::NOENT) if !create => return Ok(None),
            Err(Errno::LOOP | Errno::NOTDIR) => {
                return Err(StoreError::SandboxViolation(
                    "note entry is not a regular directory".to_owned(),
                ));
            }
            Err(error) => return Err(io_error(error)),
        };
        let metadata = fstat(&directory).map_err(io_error)?;
        if !FileType::from_raw_mode(metadata.st_mode).is_dir() {
            return Err(StoreError::SandboxViolation(
                "note entry is not a regular directory".to_owned(),
            ));
        }
        Ok(Some(directory))
    }

    fn versions(&self, id: ObjectId) -> Result<Vec<(u64, String)>, StoreError> {
        let Some(directory) = self.note_dir(id, false)? else {
            return Ok(Vec::new());
        };
        let entries = Dir::read_from(&directory).map_err(io_error)?;
        let mut versions = Vec::new();
        for entry in entries {
            let entry = entry.map_err(io_error)?;
            let name = match entry.file_name().to_str() {
                Ok(name) => name,
                Err(_) => continue,
            };
            let Some(number) = name
                .strip_prefix("rev-")
                .and_then(|name| name.strip_suffix(".md"))
            else {
                continue;
            };
            if number.len() != 20 || !number.bytes().all(|byte| byte.is_ascii_digit()) {
                continue;
            }
            let revision = number
                .parse::<u64>()
                .map_err(|error| StoreError::Corrupt(error.to_string()))?;
            let metadata = statat(&directory, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io_error)?;
            let kind = FileType::from_raw_mode(metadata.st_mode);
            if kind.is_symlink() || !kind.is_file() {
                return Err(StoreError::SandboxViolation(
                    "revision entry is not a regular file".to_owned(),
                ));
            }
            versions.push((revision, name.to_owned()));
        }
        versions.sort_by_key(|(revision, _)| *revision);
        Ok(versions)
    }

    fn read_file(
        &self,
        id: ObjectId,
        revision: u64,
        name: &str,
    ) -> Result<NoteDocument, StoreError> {
        let directory = self
            .note_dir(id, false)?
            .ok_or_else(|| StoreError::Io("note directory disappeared".to_owned()))?;
        let fd = match openat(
            &directory,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => fd,
            Err(Errno::LOOP) => {
                return Err(StoreError::SandboxViolation(
                    "revision entry is not a regular file".to_owned(),
                ));
            }
            Err(error) => return Err(io_error(error)),
        };
        let metadata = fstat(&fd).map_err(io_error)?;
        if !FileType::from_raw_mode(metadata.st_mode).is_file() {
            return Err(StoreError::SandboxViolation(
                "revision entry is not a regular file".to_owned(),
            ));
        }
        if metadata.st_size < 0 || metadata.st_size as u64 > MAX_STORED_NOTE_BYTES as u64 {
            return Err(StoreError::Corrupt("note exceeds 16 MiB".to_owned()));
        }
        let mut bytes = Vec::with_capacity(metadata.st_size as usize);
        File::from(fd)
            .take((MAX_STORED_NOTE_BYTES as u64) + 1)
            .read_to_end(&mut bytes)
            .map_err(std_io_error)?;
        if bytes.len() > MAX_STORED_NOTE_BYTES {
            return Err(StoreError::Corrupt("note exceeds 16 MiB".to_owned()));
        }
        let input = std::str::from_utf8(&bytes)
            .map_err(|error| StoreError::Corrupt(format!("note is not valid UTF-8: {error}")))?;
        let parsed = decode_stored(input, self.ids.as_ref())
            .map_err(|error: MarkdownError| StoreError::Corrupt(error.to_string()))?;
        if parsed.id != id || parsed.revision != revision {
            return Err(StoreError::Corrupt(
                "file identity does not match its revision path".to_owned(),
            ));
        }
        validate_candidate(&parsed, revision)
            .map_err(|error| StoreError::Corrupt(error.to_string()))?;
        Ok(parsed)
    }

    fn latest(&self, id: ObjectId) -> Result<Option<NoteDocument>, StoreError> {
        let versions = self.versions(id)?;
        let Some((revision, name)) = versions.last() else {
            return Ok(None);
        };
        self.read_file(id, *revision, name).map(Some)
    }

    fn write_revision(&self, note: &NoteDocument, revision: u64) -> Result<(), StoreError> {
        let mut saved = note.clone();
        saved.revision = revision;
        let contents = encode_stored_checked(&saved).map_err(StoreError::InvalidDocument)?;
        if contents.len() > MAX_STORED_NOTE_BYTES {
            return Err(StoreError::InvalidDocument("note exceeds 16 MiB"));
        }
        let directory = self
            .note_dir(note.id, true)?
            .ok_or_else(|| StoreError::Io("failed to create note directory".to_owned()))?;
        let final_name = format!("rev-{revision:020}.md");
        if statat(&directory, final_name.as_str(), AtFlags::SYMLINK_NOFOLLOW).is_ok() {
            return Err(StoreError::RevisionConflict {
                expected: revision.saturating_sub(1),
                actual: revision,
            });
        }

        let (temporary_name, fd) = create_temp_file(&directory)?;
        let result = (|| {
            let mut file = File::from(fd);
            file.write_all(contents.as_bytes()).map_err(std_io_error)?;
            file.sync_all().map_err(std_io_error)?;
            drop(file);
            linkat(
                &directory,
                temporary_name.as_str(),
                &directory,
                final_name.as_str(),
                AtFlags::empty(),
            )
            .map_err(|error| {
                if error == Errno::EXIST {
                    StoreError::RevisionConflict {
                        expected: revision.saturating_sub(1),
                        actual: revision,
                    }
                } else {
                    io_error(error)
                }
            })?;
            unlinkat(&directory, temporary_name.as_str(), AtFlags::empty()).map_err(io_error)?;
            sync_revision_directory(&directory)?;
            fsync(&self.root).map_err(io_error)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = unlinkat(&directory, temporary_name.as_str(), AtFlags::empty());
        }
        result
    }
}

impl NoteStore for LinuxDirectoryNoteStore {
    fn list(&self, include_deleted: bool) -> Result<Vec<NoteSummary>, StoreError> {
        let entries = Dir::read_from(&self.root).map_err(io_error)?;
        let mut result = Vec::new();
        for entry in entries {
            let entry = entry.map_err(io_error)?;
            let name = match entry.file_name().to_str() {
                Ok(name) => name,
                Err(_) => continue,
            };
            if name.len() != 16 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                continue;
            }
            let Ok(id) = u64::from_str_radix(name, 16) else {
                continue;
            };
            if let Some(note) = self.latest(ObjectId(id))? {
                if include_deleted || !note.deleted {
                    result.push(summary(&note));
                }
            }
        }
        result.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.id.0.cmp(&right.id.0))
        });
        Ok(result)
    }

    fn load(&self, id: ObjectId) -> Result<Option<NoteDocument>, StoreError> {
        self.latest(id)
    }

    fn load_revision(
        &self,
        id: ObjectId,
        revision: u64,
    ) -> Result<Option<NoteDocument>, StoreError> {
        let versions = self.versions(id)?;
        let Some((_, name)) = versions.iter().find(|(number, _)| *number == revision) else {
            return Ok(None);
        };
        self.read_file(id, revision, name).map(Some)
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
            .map_err(|_| StoreError::Io("Linux fd store lock poisoned".to_owned()))?;
        let current = self.latest(note.id)?;
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
                let directory = self.note_dir(note.id, false)?.ok_or_else(|| {
                    StoreError::Io("committed note directory disappeared".to_owned())
                })?;
                sync_revision_directory(&directory)?;
                fsync(&self.root).map_err(io_error)?;
                return Ok(current.expect("checked above"));
            }
            return Err(StoreError::RevisionConflict {
                expected: expected_revision,
                actual,
            });
        }
        let revision = expected_revision
            .checked_add(1)
            .ok_or(StoreError::RevisionOverflow)?;
        self.write_revision(note, revision)?;
        let mut saved = note.clone();
        saved.revision = revision;
        Ok(saved)
    }
}

fn create_temp_file(directory: &OwnedFd) -> Result<(String, OwnedFd), StoreError> {
    for _ in 0..32 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!(".tmp-{}-{sequence:016x}", std::process::id());
        match openat(
            directory,
            name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::from_raw_mode(0o666),
        ) {
            Ok(fd) => return Ok((name, fd)),
            Err(Errno::EXIST) => continue,
            Err(error) => return Err(io_error(error)),
        }
    }
    Err(StoreError::Io(
        "could not allocate a unique temporary file".to_owned(),
    ))
}

fn io_error(error: Errno) -> StoreError {
    StoreError::Io(error.to_string())
}

fn sync_revision_directory(directory: &OwnedFd) -> Result<(), StoreError> {
    #[cfg(test)]
    if FAIL_NEXT_REVISION_DIRECTORY_FSYNC.with(|fail| fail.replace(false)) {
        return Err(StoreError::Io(
            "injected note-directory fsync failure".to_owned(),
        ));
    }
    fsync(directory).map_err(io_error)
}

#[cfg(test)]
fn fail_next_revision_directory_fsync() {
    FAIL_NEXT_REVISION_DIRECTORY_FSYNC.with(|fail| fail.set(true));
}

fn std_io_error(error: std::io::Error) -> StoreError {
    StoreError::Io(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Block, BlockKind, Timestamp};
    use nagi_model::WorkspaceId;
    use rustix::fs::open;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nagi-linux-fd-store-{}-{}",
                std::process::id(),
                NEXT_ROOT.fetch_add(1, AtomicOrdering::Relaxed)
            ));
            fs::create_dir(&path).expect("create candidate-only Linux fd store test root");
            Self(path)
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn store(root: &TempRoot) -> LinuxDirectoryNoteStore {
        let directory = File::open(&root.0).expect("open caller-selected directory handle");
        LinuxDirectoryNoteStore::from_root_fd(directory.into())
            .expect("accept caller-provided directory fd")
    }

    fn rich_note(id: ObjectId, body: &str) -> NoteDocument {
        let mut note = NoteDocument::new(id, "Linux fd store".to_owned(), Timestamp(10));
        note.workspace_id = Some(WorkspaceId(90));
        note.workspace_ids = vec![WorkspaceId(90), WorkspaceId(91)];
        note.tags = vec!["fd-store".to_owned(), "linux".to_owned()];
        note.blocks.push(Block::new(
            ObjectId(id.0 + 1),
            BlockKind::Paragraph(body.to_owned()),
        ));
        note.blocks.push(Block::new(
            ObjectId(id.0 + 2),
            BlockKind::Checklist {
                text: "keep completion".to_owned(),
                completed: true,
            },
        ));
        note
    }

    fn revision_path(root: &TempRoot, id: ObjectId, revision: u64) -> PathBuf {
        root.0
            .join(format!("{:016x}", id.0))
            .join(format!("rev-{revision:020}.md"))
    }

    #[test]
    fn fd_store_preserves_note_contract_revisions_and_corruption_errors() {
        let root = TempRoot::new();
        let id = ObjectId(0x4e41_4749_00b2_0001);
        let store = store(&root);
        let first = store
            .commit(&rich_note(id, "original body"), 0)
            .expect("commit first complete revision through directory fd");
        assert_eq!(first.revision, 1);

        let mut candidate = first.clone();
        candidate.title = "Linux fd store revision two".to_owned();
        candidate.updated_at = Timestamp(20);
        candidate.blocks[0].kind = BlockKind::Paragraph("new body".to_owned());
        let second = store
            .commit(&candidate, first.revision)
            .expect("commit second complete revision through directory fd");
        assert_eq!(second.revision, 2);
        assert_eq!(store.load_revision(id, 1).unwrap(), Some(first.clone()));
        assert_eq!(store.load(id).unwrap(), Some(second.clone()));
        let summaries = store.list(false).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, id);
        assert_eq!(summaries[0].title, second.title);

        assert!(matches!(
            store.commit(&first, 1),
            Err(StoreError::RevisionConflict {
                expected: 1,
                actual: 2
            })
        ));
        assert_eq!(store.load_revision(id, 1).unwrap(), Some(first.clone()));
        assert_eq!(store.load(id).unwrap(), Some(second.clone()));

        let second_path = revision_path(&root, id, 2);
        let original_v2 = fs::read(&second_path).unwrap();
        fs::write(&second_path, &original_v2[..original_v2.len() - 3]).unwrap();
        assert!(matches!(store.load(id), Err(StoreError::Corrupt(_))));
        assert_eq!(store.load_revision(id, 1).unwrap(), Some(first.clone()));

        fs::write(&second_path, &original_v2).unwrap();
        let body_start = original_v2
            .windows(b"\n---\n".len())
            .position(|window| window == b"\n---\n")
            .map(|position| position + b"\n---\n".len())
            .unwrap();
        let body_offset = original_v2[body_start..]
            .windows(b"new body".len())
            .position(|window| window == b"new body")
            .map(|position| body_start + position)
            .unwrap();
        let mut changed_v2 = original_v2.clone();
        changed_v2[body_offset] = b'x';
        fs::write(&second_path, changed_v2).unwrap();
        assert!(matches!(store.load(id), Err(StoreError::Corrupt(_))));
        assert_eq!(store.load_revision(id, 1).unwrap(), Some(first));

        fs::write(&second_path, &original_v2).unwrap();
        assert_eq!(store.load(id).unwrap(), Some(second));
    }

    #[test]
    fn fd_store_rejects_non_directory_roots_and_symlinked_note_entries() {
        let temp = TempRoot::new();
        let not_directory = temp.0.join("not-directory");
        fs::write(&not_directory, b"file").unwrap();
        let file = File::open(&not_directory).unwrap();
        assert!(matches!(
            LinuxDirectoryNoteStore::from_root_fd(file.into()),
            Err(StoreError::SandboxViolation(_))
        ));

        let root_path = temp.0.join("root");
        let outside = temp.0.join("outside");
        fs::create_dir(&root_path).unwrap();
        fs::create_dir(&outside).unwrap();
        let id = ObjectId(0x4e41_4749_00b2_0002);
        std::os::unix::fs::symlink(&outside, root_path.join(format!("{:016x}", id.0))).unwrap();
        let root_fd = File::open(&root_path).unwrap();
        let store = LinuxDirectoryNoteStore::from_root_fd(root_fd.into()).unwrap();
        assert!(matches!(
            store.load(id),
            Err(StoreError::SandboxViolation(_))
        ));
    }

    #[test]
    fn fd_store_upgrades_an_opath_root_to_a_syncable_directory_fd() {
        let temp = TempRoot::new();
        let id = ObjectId(0x4e41_4749_00b2_0003);
        let path_fd = open(
            &temp.0,
            OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("open an O_PATH directory descriptor");
        let store = LinuxDirectoryNoteStore::from_root_fd(path_fd)
            .expect("constructor retains a read-capable descriptor opened relative to O_PATH");

        let first = store
            .commit(&rich_note(id, "O_PATH root"), 0)
            .expect("commit through the upgraded directory descriptor");
        assert_eq!(first.revision, 1);
        assert_eq!(store.load(id).unwrap(), Some(first));
    }

    #[test]
    fn idempotent_retry_syncs_a_linked_revision_before_returning_ok() {
        let root = TempRoot::new();
        let id = ObjectId(0x4e41_4749_00b2_0004);
        let candidate = rich_note(id, "retry after link");
        let first_store = store(&root);

        fail_next_revision_directory_fsync();
        assert!(matches!(
            first_store.commit(&candidate, 0),
            Err(StoreError::Io(message)) if message.contains("injected note-directory fsync failure")
        ));
        let linked = first_store
            .load(id)
            .expect("read the linked revision left by the injected post-link failure")
            .expect("link succeeded before the injected failure");
        assert_eq!(linked.revision, 1);

        let retry_store = store(&root);
        fail_next_revision_directory_fsync();
        assert!(
            matches!(
                retry_store.commit(&candidate, 0),
                Err(StoreError::Io(message)) if message.contains("injected note-directory fsync failure")
            ),
            "an idempotent retry must attempt directory fsync before Ok"
        );

        let committed = retry_store
            .commit(&candidate, 0)
            .expect("retry succeeds after directory sync is available");
        assert_eq!(committed, linked);
        assert_eq!(retry_store.load_revision(id, 1).unwrap(), Some(linked));
    }

    #[test]
    fn fd_store_rejects_lossy_blocks_before_creating_note_entries() {
        let root = TempRoot::new();
        let cases = [
            BlockKind::Paragraph(String::new()),
            BlockKind::Paragraph("# heading".to_owned()),
            BlockKind::Table {
                rows: vec![vec!["a|b".to_owned()]],
            },
        ];

        for (offset, kind) in cases.into_iter().enumerate() {
            let id = ObjectId(0x4e41_4749_00b3_0000 + offset as u64);
            let store = store(&root);
            let mut note = rich_note(id, "lossy candidate");
            note.blocks[0].kind = kind;

            assert!(matches!(
                store.commit(&note, 0),
                Err(StoreError::InvalidDocument(_))
            ));
            assert_eq!(store.load(id).unwrap(), None);
            assert!(!root.0.join(format!("{:016x}", id.0)).exists());
        }
    }
}
