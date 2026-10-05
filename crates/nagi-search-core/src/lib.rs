#![no_std]
//! SEARCH-CORE-01: host-side provider/index foundation for the Nagi 0.2
//! Search platform (0.2-M19).
//!
//! This crate does not define a second search model. Object records, queries,
//! hits, Workspace grouping, caller visibility, and snapshot persistence are
//! the canonical `nagi-search` contract owned by the Nagi 0.1 M19 line; IDs
//! are the canonical `nagi-model` types. Both are re-exported so consumers use
//! the identical types.
//!
//! What this crate adds is the 0.2 provider boundary in front of that index:
//!
//! - trusted provider identity (an `AppId` supplied by the integration layer,
//!   never by payload fields) behind an injected, fail-closed authority seam;
//! - per-object provider ownership and isolation;
//! - revisioned incremental updates with stale/duplicate/conflict handling;
//! - provider-scoped queries that are filtered before ranking and limits;
//! - result provenance (provider + revision) for every returned hit;
//! - a versioned, checksummed ledger snapshot that rejects unknown versions.
//!
//! No daemon, IPC endpoint, production persistence, or target integration is
//! provided here; those remain gated on Nagi 0.1 M30 PASS and an explicit 0.2
//! Integration Owner checkpoint.

extern crate alloc;

mod backend;
mod index;
mod ledger;
mod model;

pub use backend::MemorySnapshotBackend;
pub use index::SearchCore;
pub use ledger::{
    LedgerEntry, LedgerError, CURRENT_LEDGER_VERSION, LEDGER_MAGIC, MAX_LEDGER_ENTRIES,
    MAX_LEDGER_SNAPSHOT_BYTES,
};
pub use model::{
    DenyAllProviders, DocumentRevision, IngestOutcome, ProviderAuthority, ProviderHit,
    ProviderScope, RemoveOutcome, ScopedQuery, ScopedSearchResponse, SearchCoreError,
    MAX_PROVIDERS, SEARCH_CORE_CONTRACT_VERSION,
};

/// Canonical ID contract (re-exported, not redefined).
pub use nagi_model;
/// Canonical 0.1 Search/Index contract (re-exported, not redefined).
pub use nagi_search;
