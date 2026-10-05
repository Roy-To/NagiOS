use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nagi_model::ObjectId;

use crate::domain::{NoteDocument, NoteSummary};
use crate::identity::{HostObjectIdSource, ObjectIdSource};
use crate::markdown::{decode_stored, encode_stored, MarkdownError};

const MAX_STORED_NOTE_BYTES: usize = 16 * 1024 * 1024;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StoreError {
    Io(String),
    Corrupt(String),
    RevisionConflict { expected: u64, actual: u64 },
    RevisionOverflow,
    InvalidDocument(&'static str),
    SandboxViolation(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(f, "storage I/O failed: {message}"),
            Self::Corrupt(message) => write!(f, "stored note is invalid: {message}"),
            Self::RevisionConflict { expected, actual } => {
                write!(f, "revision conflict: expected {expected}, found {actual}")
            }
            Self::RevisionOverflow => f.write_str("note revision is exhausted"),
            Self::InvalidDocument(message) => write!(f, "invalid note document: {message}"),
            Self::SandboxViolation(message) => write!(f, "host sandbox rejected path: {message}"),
        }
    }
}

impl std::error::Error for StoreError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SandboxError {
    Io(String),
    NotDirectory,
    Unavailable,
}

impl fmt::Display for SandboxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(message) => write!(f, "could not prepare preview directory: {message}"),
            Self::NotDirectory => f.write_str("preview root is not a directory"),
            Self::Unavailable => f.write_str("preview root could not be resolved"),
        }
    }
}

impl std::error::Error for SandboxError {}

pub trait NoteStore: Send + Sync {
    fn list(&self, include_deleted: bool) -> Result<Vec<NoteSummary>, StoreError>;
    fn load(&self, id: ObjectId) -> Result<Option<NoteDocument>, StoreError>;
    fn load_revision(
        &self,
        id: ObjectId,
        revision: u64,
    ) -> Result<Option<NoteDocument>, StoreError>;
    fn commit(
        &self,
        note: &NoteDocument,
        expected_revision: u64,
    ) -> Result<NoteDocument, StoreError>;
}

/// Volatile backend intended for unit tests and explicit in-memory preview.
/// It stores real documents and revision snapshots but does not claim durable
/// persistence.
#[derive(Default)]
pub struct InMemoryNoteStore {
    notes: Mutex<HashMap<u64, Vec<NoteDocument>>>,
}

impl InMemoryNoteStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn latest(&self, id: ObjectId) -> Result<Option<NoteDocument>, StoreError> {
        Ok(self
            .notes
            .lock()
            .map_err(|_| StoreError::Io("in-memory store lock poisoned".to_owned()))?
            .get(&id.0)
            .and_then(|versions| versions.last())
            .cloned())
    }
}

impl NoteStore for InMemoryNoteStore {
    fn list(&self, include_deleted: bool) -> Result<Vec<NoteSummary>, StoreError> {
        let notes = self
            .notes
            .lock()
            .map_err(|_| StoreError::Io("in-memory store lock poisoned".to_owned()))?;
        let mut result = notes
            .values()
            .filter_map(|versions| versions.last())
            .filter(|note| include_deleted || !note.deleted)
            .map(summary)
            .collect::<Vec<_>>();
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
        let notes = self
            .notes
            .lock()
            .map_err(|_| StoreError::Io("in-memory store lock poisoned".to_owned()))?;
        Ok(notes
            .get(&id.0)
            .and_then(|versions| versions.iter().find(|note| note.revision == revision))
            .cloned())
    }

    fn commit(
        &self,
        note: &NoteDocument,
        expected_revision: u64,
    ) -> Result<NoteDocument, StoreError> {
        validate_candidate(note, expected_revision)?;
        let mut notes = self
            .notes
            .lock()
            .map_err(|_| StoreError::Io("in-memory store lock poisoned".to_owned()))?;
        let versions = notes.entry(note.id.0).or_default();
        let actual = versions.last().map(|current| current.revision).unwrap_or(0);
        if actual != expected_revision {
            if actual == expected_revision.saturating_add(1)
                && versions
                    .last()
                    .is_some_and(|current| same_candidate(current, note))
            {
                return Ok(versions.last().cloned().expect("checked above"));
            }
            return Err(StoreError::RevisionConflict {
                expected: expected_revision,
                actual,
            });
        }
        let revision = expected_revision
            .checked_add(1)
            .ok_or(StoreError::RevisionOverflow)?;
        let mut saved = note.clone();
        saved.revision = revision;
        if encode_stored(&saved).len() > MAX_STORED_NOTE_BYTES {
            return Err(StoreError::InvalidDocument("note exceeds 16 MiB"));
        }
        versions.push(saved.clone());
        Ok(saved)
    }
}

/// Host preview persistence rooted at one explicitly selected directory.
/// Each successful save writes a new immutable Markdown revision and atomically
/// installs the fully flushed temporary file without replacing an existing
/// revision. Note paths are derived only from ObjectId values; callers cannot
/// provide arbitrary file paths.
pub struct HostPreviewStore {
    root: PathBuf,
    ids: Arc<dyn ObjectIdSource>,
    gate: Mutex<()>,
}

impl HostPreviewStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, SandboxError> {
        let root = root.as_ref();
        fs::create_dir_all(root).map_err(|error| SandboxError::Io(error.to_string()))?;
        let canonical = fs::canonicalize(root).map_err(|_| SandboxError::Unavailable)?;
        let metadata = fs::symlink_metadata(&canonical)
            .map_err(|error| SandboxError::Io(error.to_string()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(SandboxError::NotDirectory);
        }
        Ok(Self {
            root: canonical,
            ids: Arc::new(HostObjectIdSource::new()),
            gate: Mutex::new(()),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn note_dir(&self, id: ObjectId, create: bool) -> Result<Option<PathBuf>, StoreError> {
        let path = self.root.join(format!("{:016x}", id.0));
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(StoreError::SandboxViolation(
                        "note entry is not a regular directory".to_owned(),
                    ));
                }
                Ok(Some(path))
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && !create => Ok(None),
            Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::create_dir(&path) {
                Ok(()) => Ok(Some(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let metadata = fs::symlink_metadata(&path)
                        .map_err(|error| StoreError::Io(error.to_string()))?;
                    if metadata.file_type().is_symlink() || !metadata.is_dir() {
                        return Err(StoreError::SandboxViolation(
                            "note entry is not a regular directory".to_owned(),
                        ));
                    }
                    Ok(Some(path))
                }
                Err(error) => Err(StoreError::Io(error.to_string())),
            },
            Err(error) => Err(StoreError::Io(error.to_string())),
        }
    }

    fn versions(&self, id: ObjectId) -> Result<Vec<(u64, PathBuf)>, StoreError> {
        let Some(directory) = self.note_dir(id, false)? else {
            return Ok(Vec::new());
        };
        let entries = fs::read_dir(directory).map_err(|error| StoreError::Io(error.to_string()))?;
        let mut versions = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| StoreError::Io(error.to_string()))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
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
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|error| StoreError::Io(error.to_string()))?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(StoreError::SandboxViolation(
                    "revision entry is not a regular file".to_owned(),
                ));
            }
            versions.push((revision, entry.path()));
        }
        versions.sort_by_key(|(revision, _)| *revision);
        Ok(versions)
    }

    fn read_file(
        &self,
        id: ObjectId,
        revision: u64,
        path: &Path,
    ) -> Result<NoteDocument, StoreError> {
        let metadata =
            fs::symlink_metadata(path).map_err(|error| StoreError::Io(error.to_string()))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(StoreError::SandboxViolation(
                "revision entry is not a regular file".to_owned(),
            ));
        }
        if metadata.len() > MAX_STORED_NOTE_BYTES as u64 {
            return Err(StoreError::Corrupt("note exceeds 16 MiB".to_owned()));
        }
        let input = fs::read_to_string(path).map_err(|error| StoreError::Io(error.to_string()))?;
        let parsed = decode_stored(&input, self.ids.as_ref())
            .map_err(|error| StoreError::Corrupt(error.to_string()))?;
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
        let Some((revision, path)) = versions.last() else {
            return Ok(None);
        };
        self.read_file(id, *revision, path).map(Some)
    }

    fn write_revision(&self, note: &NoteDocument, revision: u64) -> Result<(), StoreError> {
        let directory = self
            .note_dir(note.id, true)?
            .ok_or_else(|| StoreError::Io("failed to create note directory".to_owned()))?;
        let mut saved = note.clone();
        saved.revision = revision;
        let contents = encode_stored(&saved);
        if contents.len() > MAX_STORED_NOTE_BYTES {
            return Err(StoreError::InvalidDocument("note exceeds 16 MiB"));
        }
        let final_path = directory.join(format!("rev-{revision:020}.md"));
        if final_path.exists() {
            return Err(StoreError::RevisionConflict {
                expected: revision.saturating_sub(1),
                actual: revision,
            });
        }
        let (temporary_path, mut file) = create_temp_file(&directory)?;
        let result = (|| {
            file.write_all(contents.as_bytes())
                .map_err(|error| StoreError::Io(error.to_string()))?;
            file.sync_all()
                .map_err(|error| StoreError::Io(error.to_string()))?;
            drop(file);
            fs::hard_link(&temporary_path, &final_path).map_err(|error| {
                if error.kind() == io::ErrorKind::AlreadyExists {
                    StoreError::RevisionConflict {
                        expected: revision.saturating_sub(1),
                        actual: revision,
                    }
                } else {
                    StoreError::Io(error.to_string())
                }
            })?;
            fs::remove_file(&temporary_path).map_err(|error| StoreError::Io(error.to_string()))?;
            File::open(&directory)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| StoreError::Io(error.to_string()))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result
    }
}

impl NoteStore for HostPreviewStore {
    fn list(&self, include_deleted: bool) -> Result<Vec<NoteSummary>, StoreError> {
        let mut result = Vec::new();
        for entry in fs::read_dir(&self.root).map_err(|error| StoreError::Io(error.to_string()))? {
            let entry = entry.map_err(|error| StoreError::Io(error.to_string()))?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.len() != 16 || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                continue;
            }
            let Ok(id) = u64::from_str_radix(&name, 16) else {
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
        let Some((_, path)) = versions.iter().find(|(number, _)| *number == revision) else {
            return Ok(None);
        };
        self.read_file(id, revision, path).map(Some)
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
            .map_err(|_| StoreError::Io("host store lock poisoned".to_owned()))?;
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

fn validate_candidate(note: &NoteDocument, expected_revision: u64) -> Result<(), StoreError> {
    if note.revision != expected_revision {
        return Err(StoreError::InvalidDocument(
            "candidate revision differs from expected revision",
        ));
    }
    if note.title.contains('\0') || note.title.len() > 64 * 1024 {
        return Err(StoreError::InvalidDocument("invalid title"));
    }
    if note.tags.len() > 64
        || note.tags.iter().any(|tag| {
            tag.is_empty()
                || tag.trim() != tag
                || tag.len() > 128
                || tag.chars().any(char::is_control)
        })
    {
        return Err(StoreError::InvalidDocument("invalid tags"));
    }
    if note.workspace_ids.len() > 32
        || note.workspace_ids.iter().enumerate().any(|(index, id)| {
            note.workspace_ids[..index]
                .iter()
                .any(|previous| previous == id)
        })
        || note
            .workspace_id
            .is_some_and(|workspace| !note.workspace_ids.contains(&workspace))
    {
        return Err(StoreError::InvalidDocument(
            "invalid workspace associations",
        ));
    }
    let mut block_ids = HashSet::with_capacity(note.blocks.len());
    if note
        .blocks
        .iter()
        .any(|block| !block_ids.insert(block.id.0))
    {
        return Err(StoreError::InvalidDocument("duplicate block identity"));
    }
    Ok(())
}

fn same_candidate(saved: &NoteDocument, candidate: &NoteDocument) -> bool {
    let mut candidate = candidate.clone();
    candidate.revision = saved.revision;
    saved == &candidate
}

fn summary(note: &NoteDocument) -> NoteSummary {
    NoteSummary {
        id: note.id,
        title: note.title.clone(),
        updated_at: note.updated_at,
        revision: note.revision,
        workspace_id: note.workspace_id,
        workspace_ids: note.workspace_ids.clone(),
        tags: note.tags.clone(),
        deleted: note.deleted,
    }
}

fn create_temp_file(directory: &Path) -> Result<(PathBuf, File), StoreError> {
    for _ in 0..32 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!(".tmp-{}-{sequence:016x}", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(StoreError::Io(error.to_string())),
        }
    }
    Err(StoreError::Io(
        "could not allocate a unique temporary file".to_owned(),
    ))
}

impl From<SandboxError> for StoreError {
    fn from(error: SandboxError) -> Self {
        StoreError::Io(error.to_string())
    }
}

impl From<MarkdownError> for StoreError {
    fn from(error: MarkdownError) -> Self {
        StoreError::Corrupt(error.to_string())
    }
}
