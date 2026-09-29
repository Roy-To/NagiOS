#![no_std]

extern crate alloc;
#[cfg(not(target_os = "nagi"))]
extern crate std;

mod codec;
pub mod guest;
mod model;
mod search;
mod store;

pub mod adapters;
#[cfg(not(target_os = "nagi"))]
pub mod host;
pub mod semantic;

pub use guest::{
    GuestSnapshotBackend, SnapshotFile, SnapshotFileStore, SnapshotSlot, GUEST_FILE_BYTES,
    MAX_GUEST_SNAPSHOT_BYTES,
};
pub use model::{
    AccessContext, AttributeMatch, MetadataRecord, ModelError, ObjectKind, Relation,
    RelationDirection, RelationKind, RelationProvenance, SearchError, SearchHit, SearchMatch,
    SearchQuery, SearchResponse, SearchSort, SearchTimeRange, VisibilityFilter, VisibilityScope,
    Workspace, WorkspaceGroup, WorkspaceHit, WorkspaceSession, MAX_SEARCH_RESULTS,
};
pub use search::{DenyAllVisibility, SearchService};
pub use semantic::{
    chunk_text, chunk_text_with_limit, Embedding, EmbeddingProvider, EmbeddingPurpose,
    IndexedChunk, SemanticError, SemanticHit, TextChunk, VectorIndex, VectorIndexError,
    VectorMatch, DEFAULT_SEMANTIC_CHUNK_BYTES, MAX_SEMANTIC_CHUNKS_PER_OBJECT,
    MAX_SEMANTIC_CHUNK_BYTES, MAX_SEMANTIC_EMBEDDING_DIMENSIONS, MAX_SEMANTIC_QUERY_BYTES,
    MAX_SEMANTIC_RESULTS, MAX_SEMANTIC_SOURCE_BYTES, MIN_SEMANTIC_CHUNK_BYTES,
};
pub use store::{BackendError, MetadataStoreError, SnapshotBackend, CURRENT_STORE_VERSION};

#[cfg(test)]
mod tests;
