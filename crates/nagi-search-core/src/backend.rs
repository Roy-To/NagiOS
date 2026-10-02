use alloc::{rc::Rc, vec::Vec};
use core::cell::RefCell;

use nagi_search::{BackendError, SnapshotBackend};

#[derive(Debug, Default)]
struct MemorySlot {
    snapshot: Option<Vec<u8>>,
    fail_loads: bool,
    fail_writes: bool,
    writes: usize,
}

/// Deterministic in-memory reference implementation of the canonical
/// `nagi_search::SnapshotBackend`. Clones share one slot, so a test can keep a
/// handle, reopen an index over the same bytes, corrupt them, or inject
/// failures. This is host reference storage, not production persistence.
#[derive(Clone, Debug, Default)]
pub struct MemorySnapshotBackend {
    slot: Rc<RefCell<MemorySlot>>,
}

impl MemorySnapshotBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> Option<Vec<u8>> {
        self.slot.borrow().snapshot.clone()
    }

    pub fn set_snapshot(&self, snapshot: Option<Vec<u8>>) {
        self.slot.borrow_mut().snapshot = snapshot;
    }

    pub fn set_fail_loads(&self, fail: bool) {
        self.slot.borrow_mut().fail_loads = fail;
    }

    pub fn set_fail_writes(&self, fail: bool) {
        self.slot.borrow_mut().fail_writes = fail;
    }

    /// Number of successful snapshot replacements.
    pub fn writes(&self) -> usize {
        self.slot.borrow().writes
    }
}

impl SnapshotBackend for MemorySnapshotBackend {
    fn load_snapshot(&mut self) -> Result<Option<Vec<u8>>, BackendError> {
        let slot = self.slot.borrow();
        if slot.fail_loads {
            return Err(BackendError::Io);
        }
        Ok(slot.snapshot.clone())
    }

    fn write_snapshot(&mut self, snapshot: &[u8]) -> Result<(), BackendError> {
        let mut slot = self.slot.borrow_mut();
        if slot.fail_writes {
            return Err(BackendError::Io);
        }
        slot.snapshot = Some(snapshot.to_vec());
        slot.writes += 1;
        Ok(())
    }
}
