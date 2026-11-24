//! Minimal revision-checked storage contract for opaque application snapshots.
//!
//! The contract intentionally does not select a provider, storage root, wire
//! format, authorization mechanism, serialization format, or durability level.

#![no_std]

extern crate alloc;

use alloc::vec::Vec;
use nagi_model::ObjectId;

/// Per-object revision token used for optimistic snapshot updates.
///
/// Tokens are compared for equality only. A provider must not reuse a token for
/// the same `ObjectId` during that object's storage lifetime. This type does
/// not promise an ordering between revisions or objects.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Revision(u64);

impl Revision {
    /// Construct a revision token. Zero is reserved as invalid.
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 {
            None
        } else {
            Some(Self(value))
        }
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One atomically observed snapshot body and its storage revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshot {
    pub revision: Revision,
    pub bytes: Vec<u8>,
}

impl Snapshot {
    pub fn new(revision: Revision, bytes: Vec<u8>) -> Self {
        Self { revision, bytes }
    }
}

/// Result of a conditional replacement attempt.
#[derive(Debug, Eq, PartialEq)]
pub enum ConditionalWriteOutcome<E> {
    /// The provider applied the replacement and assigned a new revision.
    /// This does not by itself promise durable media flush or crash survival.
    Applied { revision: Revision },
    /// The current revision did not match the expectation; no change was made.
    Conflict { current_revision: Option<Revision> },
    /// The provider knows the replacement did not take effect.
    NotApplied { error: E },
    /// The replacement may or may not have taken effect; reconcile by reading
    /// the object before retrying.
    OutcomeUnknown { error: E },
}

/// Read and compare-and-replace one opaque snapshot object.
///
/// Authorization and selection of the already-authorized storage scope belong
/// to the caller/provider composition. A successful read returns bytes and
/// revision from the same observation. `compare_and_replace` must atomically
/// compare and replace one object across all handles to the same storage scope.
///
/// `expected_revision == None` means the object must be absent. `Some(r)` means
/// the object's current revision must equal `r`. A conflict reports the
/// observed current revision (`None` means absent) and leaves the object alone.
/// An applied replacement returns a nonzero token different from the expected
/// token, if any. The contract does not define durability, flush/fsync,
/// serialization, object-ID allocation, deletion, or multi-object transactions.
pub trait SnapshotStorage {
    type Error;

    fn read(&self, key: ObjectId) -> Result<Option<Snapshot>, Self::Error>;

    fn compare_and_replace(
        &mut self,
        key: ObjectId,
        expected_revision: Option<Revision>,
        bytes: &[u8],
    ) -> ConditionalWriteOutcome<Self::Error>;
}
