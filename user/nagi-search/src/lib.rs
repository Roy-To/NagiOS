#![no_std]

extern crate alloc;
#[cfg(not(target_os = "nagi"))]
extern crate std;

mod codec;
mod model;
mod search;
mod store;

pub mod adapters;
#[cfg(not(target_os = "nagi"))]
pub mod host;

pub use model::{
    AccessContext, AttributeMatch, MetadataRecord, ModelError, ObjectKind, Relation,
    RelationDirection, RelationKind, RelationProvenance, SearchError, SearchHit, SearchMatch,
    SearchQuery, SearchResponse, SearchSort, SearchTimeRange, VisibilityFilter, VisibilityScope,
    Workspace, WorkspaceGroup, WorkspaceHit, WorkspaceSession, MAX_SEARCH_RESULTS,
};
pub use search::{DenyAllVisibility, SearchService};
pub use store::{BackendError, MetadataStoreError, SnapshotBackend, CURRENT_STORE_VERSION};

#[cfg(test)]
mod tests;
