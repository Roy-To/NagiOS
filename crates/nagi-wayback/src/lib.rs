//! Host-side Wayback and AI Activity Ledger contracts.
//!
//! This crate plans and validates metadata. It deliberately has no filesystem,
//! kernel, app UI, capability policy, or restore execution integration.

mod ledger;
mod model;
mod restore;
mod store;

pub use ledger::{ActivityLedger, ActivityQuery, LedgerError, TimeRange};
pub use model::*;
pub use restore::{
    ExecutionBoundary, PlanBlocker, RestorePlan, RestoreStep, RestoreTarget, UndoCandidate,
};
pub use store::{
    InMemoryLedgerStore, InMemorySnapshotStore, LedgerStore, SnapshotStore, StoreError,
    StoredLedger,
};

#[cfg(test)]
mod tests;
