use std::collections::HashSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nagi_model::ObjectId;

use crate::activity::{ActivityEvent, ActivityKind, ActivityOrigin, ActivitySink};
use crate::domain::{Block, BlockKind, NoteDocument, Timestamp};
use crate::store::{NoteStore, StoreError};

static SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

#[derive(Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SaveError {
    pub source: StoreError,
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(f)
    }
}

impl std::error::Error for SaveError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SaveStatus {
    Saved,
    Dirty,
    Saving,
    Failed(SaveError),
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionError {
    Closed,
    WorkerUnavailable(String),
    SaveFailed(SaveError),
    BlockNotFound(ObjectId),
    DuplicateBlockId(ObjectId),
    InvalidPosition,
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("note session is closed"),
            Self::WorkerUnavailable(message) => {
                write!(f, "Notes autosave worker could not start: {message}")
            }
            Self::SaveFailed(error) => write!(f, "note save failed: {error}"),
            Self::BlockNotFound(id) => write!(f, "block {:016x} was not found", id.0),
            Self::DuplicateBlockId(id) => {
                write!(f, "block ID {:016x} is already present", id.0)
            }
            Self::InvalidPosition => f.write_str("block position is outside the note"),
        }
    }
}

impl std::error::Error for SessionError {}

struct SessionState {
    note: NoteDocument,
    generation: u64,
    dirty: bool,
    deadline: Option<Instant>,
    status: SaveStatus,
    in_flight: bool,
    in_flight_generation: Option<u64>,
    closing: bool,
    closed: bool,
    created_pending: bool,
    persisted_title: Option<String>,
    origin: ActivityOrigin,
}

struct Inner {
    state: Mutex<SessionState>,
    changed: Condvar,
    store: Arc<dyn NoteStore>,
    clock: Arc<dyn Clock>,
    activity: Arc<dyn ActivitySink>,
    debounce: Duration,
    activity_failures: AtomicU64,
}

pub struct NoteSession {
    inner: Arc<Inner>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl NoteSession {
    pub(crate) fn start(
        note: NoteDocument,
        store: Arc<dyn NoteStore>,
        clock: Arc<dyn Clock>,
        activity: Arc<dyn ActivitySink>,
        debounce: Duration,
        new_note: bool,
        origin: ActivityOrigin,
    ) -> Result<Arc<Self>, SessionError> {
        let dirty = new_note;
        let initial_status = if dirty {
            SaveStatus::Dirty
        } else {
            SaveStatus::Saved
        };
        let inner = Arc::new(Inner {
            state: Mutex::new(SessionState {
                persisted_title: (!new_note).then(|| note.title.clone()),
                note,
                generation: 0,
                dirty,
                deadline: dirty.then(|| Instant::now() + debounce),
                status: initial_status,
                in_flight: false,
                in_flight_generation: None,
                closing: false,
                closed: false,
                created_pending: new_note,
                origin,
            }),
            changed: Condvar::new(),
            store,
            clock,
            activity,
            debounce,
            activity_failures: AtomicU64::new(0),
        });
        let worker_inner = Arc::clone(&inner);
        let sequence = SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let worker = thread::Builder::new()
            .name(format!("nagi-notes-autosave-{sequence}"))
            .spawn(move || autosave_worker(worker_inner))
            .map_err(|error| SessionError::WorkerUnavailable(error.to_string()))?;
        Ok(Arc::new(Self {
            inner,
            worker: Mutex::new(Some(worker)),
        }))
    }

    pub fn note(&self) -> NoteDocument {
        lock(&self.inner.state).note.clone()
    }

    pub fn status(&self) -> SaveStatus {
        lock(&self.inner.state).status.clone()
    }

    pub fn is_dirty(&self) -> bool {
        lock(&self.inner.state).dirty
    }

    pub fn is_closed(&self) -> bool {
        lock(&self.inner.state).closed
    }

    pub fn activity_failures(&self) -> u64 {
        self.inner.activity_failures.load(Ordering::Relaxed)
    }

    pub fn set_title(&self, title: impl Into<String>) -> Result<(), SessionError> {
        self.set_title_by(ActivityOrigin::User, title)
    }

    pub fn set_title_by(
        &self,
        origin: ActivityOrigin,
        title: impl Into<String>,
    ) -> Result<(), SessionError> {
        let title = title.into();
        self.mutate(origin, |note| {
            if note.title == title {
                Ok(false)
            } else {
                note.title = title;
                Ok(true)
            }
        })
    }

    pub fn append_block(&self, block: Block) -> Result<(), SessionError> {
        self.append_block_by(ActivityOrigin::User, block)
    }

    pub fn append_block_by(
        &self,
        origin: ActivityOrigin,
        block: Block,
    ) -> Result<(), SessionError> {
        self.mutate(origin, |note| {
            if note.blocks.iter().any(|existing| existing.id == block.id) {
                return Err(SessionError::DuplicateBlockId(block.id));
            }
            note.blocks.push(block);
            Ok(true)
        })
    }

    pub fn insert_block(&self, index: usize, block: Block) -> Result<(), SessionError> {
        self.insert_block_by(ActivityOrigin::User, index, block)
    }

    pub fn insert_block_by(
        &self,
        origin: ActivityOrigin,
        index: usize,
        block: Block,
    ) -> Result<(), SessionError> {
        self.mutate(origin, |note| {
            if index > note.blocks.len() {
                return Err(SessionError::InvalidPosition);
            }
            if note.blocks.iter().any(|existing| existing.id == block.id) {
                return Err(SessionError::DuplicateBlockId(block.id));
            }
            note.blocks.insert(index, block);
            Ok(true)
        })
    }

    pub fn replace_blocks(&self, blocks: Vec<Block>) -> Result<(), SessionError> {
        self.replace_blocks_by(ActivityOrigin::User, blocks)
    }

    pub fn replace_blocks_by(
        &self,
        origin: ActivityOrigin,
        blocks: Vec<Block>,
    ) -> Result<(), SessionError> {
        self.mutate(origin, |note| {
            let mut seen = HashSet::with_capacity(blocks.len());
            for block in &blocks {
                if !seen.insert(block.id.0) {
                    return Err(SessionError::DuplicateBlockId(block.id));
                }
            }
            if note.blocks == blocks {
                Ok(false)
            } else {
                note.blocks = blocks;
                Ok(true)
            }
        })
    }

    pub fn update_block(&self, id: ObjectId, kind: BlockKind) -> Result<(), SessionError> {
        self.update_block_by(ActivityOrigin::User, id, kind)
    }

    pub fn update_block_by(
        &self,
        origin: ActivityOrigin,
        id: ObjectId,
        kind: BlockKind,
    ) -> Result<(), SessionError> {
        self.mutate(origin, |note| {
            let block = note.block_mut(id).ok_or(SessionError::BlockNotFound(id))?;
            if block.kind == kind {
                Ok(false)
            } else {
                block.kind = kind;
                Ok(true)
            }
        })
    }

    pub fn delete_block(&self, id: ObjectId) -> Result<(), SessionError> {
        self.delete_block_by(ActivityOrigin::User, id)
    }

    pub fn delete_block_by(
        &self,
        origin: ActivityOrigin,
        id: ObjectId,
    ) -> Result<(), SessionError> {
        self.mutate(origin, |note| {
            let Some(index) = note.blocks.iter().position(|block| block.id == id) else {
                return Err(SessionError::BlockNotFound(id));
            };
            note.blocks.remove(index);
            Ok(true)
        })
    }

    pub fn move_block(&self, id: ObjectId, to: usize) -> Result<(), SessionError> {
        self.move_block_by(ActivityOrigin::User, id, to)
    }

    pub fn move_block_by(
        &self,
        origin: ActivityOrigin,
        id: ObjectId,
        to: usize,
    ) -> Result<(), SessionError> {
        self.mutate(origin, |note| {
            if to >= note.blocks.len() {
                return Err(SessionError::InvalidPosition);
            }
            let Some(from) = note.blocks.iter().position(|block| block.id == id) else {
                return Err(SessionError::BlockNotFound(id));
            };
            if from == to {
                return Ok(false);
            }
            let block = note.blocks.remove(from);
            note.blocks.insert(to, block);
            Ok(true)
        })
    }

    pub fn set_workspace(
        &self,
        workspace: Option<crate::domain::WorkspaceId>,
    ) -> Result<(), SessionError> {
        self.set_workspace_by(ActivityOrigin::User, workspace)
    }

    pub fn set_workspace_by(
        &self,
        origin: ActivityOrigin,
        workspace: Option<crate::domain::WorkspaceId>,
    ) -> Result<(), SessionError> {
        self.mutate(origin, |note| {
            if note.workspace_id == workspace {
                Ok(false)
            } else {
                note.workspace_id = workspace;
                if let Some(workspace) = workspace {
                    if !note.workspace_ids.contains(&workspace) {
                        note.workspace_ids.push(workspace);
                    }
                }
                Ok(true)
            }
        })
    }

    pub fn add_to_workspace(
        &self,
        origin: ActivityOrigin,
        workspace: crate::domain::WorkspaceId,
    ) -> Result<(), SessionError> {
        self.mutate(origin, |note| {
            if note.workspace_ids.contains(&workspace) {
                Ok(false)
            } else {
                note.workspace_ids.push(workspace);
                if note.workspace_id.is_none() {
                    note.workspace_id = Some(workspace);
                }
                Ok(true)
            }
        })
    }

    pub fn remove_from_workspace(
        &self,
        origin: ActivityOrigin,
        workspace: crate::domain::WorkspaceId,
    ) -> Result<(), SessionError> {
        self.mutate(origin, |note| {
            let Some(index) = note
                .workspace_ids
                .iter()
                .position(|item| *item == workspace)
            else {
                return Ok(false);
            };
            note.workspace_ids.remove(index);
            if note.workspace_id == Some(workspace) {
                note.workspace_id = note.workspace_ids.first().copied();
            }
            Ok(true)
        })
    }

    pub fn replace_workspaces(
        &self,
        origin: ActivityOrigin,
        workspaces: Vec<crate::domain::WorkspaceId>,
    ) -> Result<(), SessionError> {
        self.mutate(origin, |note| {
            let mut unique = Vec::with_capacity(workspaces.len());
            for workspace in workspaces {
                if !unique.contains(&workspace) {
                    unique.push(workspace);
                }
            }
            if note.workspace_ids == unique {
                Ok(false)
            } else {
                note.workspace_ids = unique;
                if note
                    .workspace_id
                    .is_some_and(|current| !note.workspace_ids.contains(&current))
                {
                    note.workspace_id = note.workspace_ids.first().copied();
                }
                Ok(true)
            }
        })
    }

    pub fn set_tags(&self, tags: Vec<String>) -> Result<(), SessionError> {
        self.set_tags_by(ActivityOrigin::User, tags)
    }

    pub fn set_tags_by(
        &self,
        origin: ActivityOrigin,
        tags: Vec<String>,
    ) -> Result<(), SessionError> {
        let mut normalized = Vec::new();
        for tag in tags {
            let tag = tag.trim().to_lowercase();
            if !tag.is_empty() && !normalized.contains(&tag) {
                normalized.push(tag);
            }
        }
        self.mutate(origin, |note| {
            if note.tags == normalized {
                Ok(false)
            } else {
                note.tags = normalized;
                Ok(true)
            }
        })
    }

    fn mutate(
        &self,
        origin: ActivityOrigin,
        operation: impl FnOnce(&mut NoteDocument) -> Result<bool, SessionError>,
    ) -> Result<(), SessionError> {
        let mut state = lock(&self.inner.state);
        if state.closed || state.closing || state.note.deleted {
            return Err(SessionError::Closed);
        }
        let changed = operation(&mut state.note)?;
        if !changed {
            return Ok(());
        }
        if !state.dirty || state.in_flight_generation == Some(state.generation) {
            state.origin = origin;
        } else if state.origin != origin {
            state.origin = ActivityOrigin::Mixed;
        }
        state.note.updated_at = self.inner.clock.now();
        state.generation = state.generation.wrapping_add(1);
        state.dirty = true;
        state.deadline = Some(Instant::now() + self.inner.debounce);
        if !state.in_flight {
            state.status = SaveStatus::Dirty;
        }
        self.inner.changed.notify_all();
        Ok(())
    }

    /// Force a pending or previously failed save and wait for its real backend
    /// result. A failed save leaves the in-memory document dirty.
    pub fn flush(&self) -> Result<NoteDocument, SessionError> {
        let mut state = lock(&self.inner.state);
        if state.closed {
            return Ok(state.note.clone());
        }
        if state.dirty {
            if let SaveStatus::Failed(error) = &state.status {
                return Err(SessionError::SaveFailed(error.clone()));
            }
            state.deadline = Some(Instant::now());
            if !state.in_flight {
                state.status = SaveStatus::Dirty;
            }
            self.inner.changed.notify_all();
        }
        loop {
            if !state.dirty {
                return Ok(state.note.clone());
            }
            if let SaveStatus::Failed(error) = &state.status {
                return Err(SessionError::SaveFailed(error.clone()));
            }
            state = self
                .inner
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    pub fn retry(&self) -> Result<NoteDocument, SessionError> {
        let mut state = lock(&self.inner.state);
        if state.closed || state.closing {
            return Err(SessionError::Closed);
        }
        if state.dirty {
            state.deadline = Some(Instant::now());
            state.status = SaveStatus::Dirty;
            self.inner.changed.notify_all();
        }
        drop(state);
        self.flush()
    }

    /// Closing waits for persistence. On failure, the session stays open and
    /// dirty so the caller can retry or export its content.
    pub fn close(&self) -> Result<NoteDocument, SessionError> {
        let note = self.flush()?;
        {
            let mut state = lock(&self.inner.state);
            if state.closed {
                return Ok(state.note.clone());
            }
            state.closing = true;
            self.inner.changed.notify_all();
        }
        if let Some(worker) = lock(&self.worker).take() {
            let _ = worker.join();
        }
        let state = lock(&self.inner.state);
        if state.closed {
            Ok(state.note.clone())
        } else {
            Ok(note)
        }
    }
}

impl Drop for NoteSession {
    fn drop(&mut self) {
        if self.close().is_err() {
            let mut state = lock(&self.inner.state);
            state.closing = true;
            self.inner.changed.notify_all();
            drop(state);
            if let Some(worker) = lock(&self.worker).take() {
                let _ = worker.join();
            }
        }
    }
}

fn autosave_worker(inner: Arc<Inner>) {
    let mut state = lock(&inner.state);
    loop {
        if state.closing && (!state.dirty || matches!(state.status, SaveStatus::Failed(_))) {
            state.closed = true;
            state.status = SaveStatus::Closed;
            inner.changed.notify_all();
            return;
        }
        if !state.dirty || matches!(state.status, SaveStatus::Failed(_)) {
            state = inner
                .changed
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            continue;
        }
        if let Some(deadline) = state.deadline {
            let now = Instant::now();
            if deadline > now {
                let (next, _) = inner
                    .changed
                    .wait_timeout(state, deadline - now)
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                state = next;
                continue;
            }
        }

        let snapshot = state.note.clone();
        let generation = state.generation;
        let was_new = state.created_pending;
        let old_title = state.persisted_title.clone();
        let origin = state.origin;
        let expected_revision = snapshot.revision;
        state.in_flight = true;
        state.in_flight_generation = Some(generation);
        state.status = SaveStatus::Saving;
        state.deadline = None;
        drop(state);
        let result = inner.store.commit(&snapshot, expected_revision);
        state = lock(&inner.state);
        state.in_flight = false;
        state.in_flight_generation = None;

        match result {
            Ok(saved) => {
                state.note.revision = saved.revision;
                state.persisted_title = Some(saved.title.clone());
                state.created_pending = false;
                let unchanged = state.generation == generation;
                if unchanged {
                    state.note.updated_at = saved.updated_at;
                    state.dirty = false;
                    state.status = SaveStatus::Saved;
                    state.deadline = None;
                    state.origin = ActivityOrigin::User;
                } else {
                    state.dirty = true;
                    state.status = SaveStatus::Dirty;
                    state.deadline = Some(Instant::now());
                }
                let mut events = Vec::new();
                if was_new {
                    events.push(ActivityKind::Created);
                } else {
                    events.push(ActivityKind::Edited);
                    if old_title
                        .as_deref()
                        .is_some_and(|title| title != saved.title)
                    {
                        events.push(ActivityKind::Renamed);
                    }
                }
                events.push(ActivityKind::Saved);
                inner.changed.notify_all();
                drop(state);
                for kind in events {
                    let event = ActivityEvent {
                        object_id: saved.id,
                        kind,
                        origin,
                        revision: saved.revision,
                        workspace_id: saved.workspace_id,
                        occurred_at: saved.updated_at,
                    };
                    if inner.activity.record(event).is_err() {
                        inner.activity_failures.fetch_add(1, Ordering::Relaxed);
                    }
                }
                state = lock(&inner.state);
            }
            Err(source) => {
                state.dirty = true;
                state.deadline = None;
                state.status = SaveStatus::Failed(SaveError { source });
                inner.changed.notify_all();
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
