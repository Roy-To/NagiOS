use crate::{Failpoint, FailurePlan, HarnessError, ResourceBudget, ResourceKind, Result};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct FakeFilesystem {
    inner: Arc<Mutex<FsState>>,
    budget: ResourceBudget,
    failures: FailurePlan,
}

struct FsState {
    next_root: u64,
    roots: BTreeMap<u64, BTreeMap<String, FileEntry>>,
}

struct FileEntry {
    bytes: Vec<u8>,
    _file_lease: crate::resources::ResourceLease,
    _byte_lease: crate::resources::ResourceLease,
}

impl FakeFilesystem {
    pub fn new(budget: ResourceBudget, failures: FailurePlan) -> Self {
        Self {
            inner: Arc::new(Mutex::new(FsState {
                next_root: 1,
                roots: BTreeMap::new(),
            })),
            budget,
            failures,
        }
    }

    pub fn temporary_root(&self) -> Result<TemporaryRoot> {
        let handle_lease = self.budget.acquire(ResourceKind::Handles, 1)?;
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let root_id = state.next_root;
        state.next_root = state
            .next_root
            .checked_add(1)
            .ok_or(HarnessError::ArithmeticOverflow)?;
        state.roots.insert(root_id, BTreeMap::new());
        Ok(TemporaryRoot {
            inner: Arc::new(RootHandle {
                filesystem: self.clone(),
                root_id,
                _handle_lease: handle_lease,
            }),
        })
    }

    pub fn root_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .roots
            .len()
    }

    pub fn contains_root(&self, root_id: u64) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .roots
            .contains_key(&root_id)
    }

    fn remove_root(&self, root_id: u64) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .roots
            .remove(&root_id)
            .is_some()
    }
}

#[derive(Clone)]
pub struct TemporaryRoot {
    inner: Arc<RootHandle>,
}

struct RootHandle {
    filesystem: FakeFilesystem,
    root_id: u64,
    _handle_lease: crate::resources::ResourceLease,
}

impl Drop for RootHandle {
    fn drop(&mut self) {
        self.filesystem.remove_root(self.root_id);
    }
}

impl TemporaryRoot {
    pub fn write_atomic(&self, relative_path: &str, bytes: &[u8]) -> Result<()> {
        let path = normalize_path(relative_path)?;
        let file_exists = {
            let state = self
                .inner
                .filesystem
                .inner
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let files = state
                .roots
                .get(&self.inner.root_id)
                .ok_or(HarnessError::NotFound)?;
            files.contains_key(&path)
        };

        // Reserve quota before copying input into the fixture's bounded store.
        let file_lease = if file_exists {
            None
        } else {
            Some(
                self.inner
                    .filesystem
                    .budget
                    .acquire(ResourceKind::Files, 1)?,
            )
        };
        let byte_lease = self
            .inner
            .filesystem
            .budget
            .acquire(ResourceKind::Bytes, bytes.len() as u64)?;
        let staged = bytes.to_vec();

        if self
            .inner
            .filesystem
            .failures
            .trip(Failpoint::InterruptedWrite)
        {
            return Err(HarnessError::Injected(Failpoint::InterruptedWrite));
        }
        if self.inner.filesystem.failures.trip(Failpoint::CorruptState) {
            let mut state = self
                .inner
                .filesystem
                .inner
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let files = state
                .roots
                .get_mut(&self.inner.root_id)
                .ok_or(HarnessError::NotFound)?;
            let retained_file_lease = file_lease
                .or_else(|| files.get(&path).map(|entry| entry._file_lease.clone()))
                .ok_or(HarnessError::NotFound)?;
            files.insert(
                path,
                FileEntry {
                    bytes: staged,
                    _file_lease: retained_file_lease,
                    _byte_lease: byte_lease,
                },
            );
            return Err(HarnessError::CorruptStateInjected);
        }

        let mut state = self
            .inner
            .filesystem
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let files = state
            .roots
            .get_mut(&self.inner.root_id)
            .ok_or(HarnessError::NotFound)?;
        let retained_file_lease = file_lease
            .or_else(|| files.get(&path).map(|entry| entry._file_lease.clone()))
            .ok_or(HarnessError::NotFound)?;
        files.insert(
            path,
            FileEntry {
                bytes: staged,
                _file_lease: retained_file_lease,
                _byte_lease: byte_lease,
            },
        );
        Ok(())
    }

    pub fn read(&self, relative_path: &str) -> Result<FileData> {
        let path = normalize_path(relative_path)?;
        loop {
            let requested_size = {
                let state = self
                    .inner
                    .filesystem
                    .inner
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                let files = state
                    .roots
                    .get(&self.inner.root_id)
                    .ok_or(HarnessError::NotFound)?;
                files.get(&path).ok_or(HarnessError::NotFound)?.bytes.len() as u64
            };
            // Acquire before copying. If a concurrent atomic replacement grew the
            // file, release this reservation and retry with the new size.
            let lease = self
                .inner
                .filesystem
                .budget
                .acquire(ResourceKind::Bytes, requested_size)?;
            let state = self
                .inner
                .filesystem
                .inner
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let files = state
                .roots
                .get(&self.inner.root_id)
                .ok_or(HarnessError::NotFound)?;
            let entry = files.get(&path).ok_or(HarnessError::NotFound)?;
            if entry.bytes.len() as u64 > requested_size {
                drop(state);
                drop(lease);
                continue;
            }
            return Ok(FileData {
                bytes: entry.bytes.clone(),
                _lease: lease,
            });
        }
    }

    pub fn remove_file(&self, relative_path: &str) -> Result<bool> {
        let path = normalize_path(relative_path)?;
        let mut state = self
            .inner
            .filesystem
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let files = state
            .roots
            .get_mut(&self.inner.root_id)
            .ok_or(HarnessError::NotFound)?;
        Ok(files.remove(&path).is_some())
    }

    pub fn file_count(&self) -> usize {
        self.inner
            .filesystem
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .roots
            .get(&self.inner.root_id)
            .map(BTreeMap::len)
            .unwrap_or(0)
    }

    pub fn root_identity(&self) -> u64 {
        self.inner.root_id
    }

    pub fn handle_count(&self) -> usize {
        Arc::strong_count(&self.inner)
    }
}

pub struct FileData {
    bytes: Vec<u8>,
    _lease: crate::resources::ResourceLease,
}

impl FileData {
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }
}

fn normalize_path(path: &str) -> Result<String> {
    if path.is_empty()
        || path.len() > 256
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('\0')
    {
        return Err(HarnessError::InvalidPath);
    }
    let mut components = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => return Err(HarnessError::InvalidPath),
            ".." => return Err(HarnessError::PathEscapesRoot),
            value => components.push(value),
        }
    }
    Ok(components.join("/"))
}
