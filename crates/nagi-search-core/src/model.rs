use alloc::{collections::BTreeSet, vec::Vec};

use nagi_model::AppId;
use nagi_search::{
    BackendError, MetadataStoreError, ModelError, SearchError, SearchHit, SearchQuery,
    WorkspaceGroup, WorkspaceHit,
};

use crate::ledger::LedgerError;

/// Version of the provider-facing SEARCH-CORE contract defined in this crate.
pub const SEARCH_CORE_CONTRACT_VERSION: u16 = 1;

/// Upper bound on concurrently registered providers and on the size of an
/// explicit provider scope.
pub const MAX_PROVIDERS: usize = 256;

/// Provider-assigned, strictly increasing revision of one indexed object.
/// Zero is reserved so an absent revision can never be confused with a real
/// one.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DocumentRevision(u64);

impl DocumentRevision {
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

/// Injected authorization seam for provider writes. Production integration
/// must back this with the Capability/Permissions decision for the
/// `search.provider` platform capability; this crate never grants authority
/// itself. The decision is re-evaluated on every write so revocation applies
/// immediately.
pub trait ProviderAuthority {
    fn may_publish(&self, provider: AppId) -> bool;
}

/// Fail-closed default authority.
#[derive(Clone, Copy, Debug, Default)]
pub struct DenyAllProviders;

impl ProviderAuthority for DenyAllProviders {
    fn may_publish(&self, _provider: AppId) -> bool {
        false
    }
}

/// Which providers a query may draw object results from. The scope is applied
/// before field matching, ranking, and the result limit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderScope {
    /// Every currently registered provider.
    All,
    /// Only the listed providers. Must be non-empty and bounded.
    Only(BTreeSet<AppId>),
}

impl ProviderScope {
    pub fn only(providers: impl IntoIterator<Item = AppId>) -> Self {
        Self::Only(providers.into_iter().collect())
    }
}

/// A canonical `nagi_search::SearchQuery` plus the 0.2 provider scope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopedQuery {
    pub query: SearchQuery,
    pub providers: ProviderScope,
}

impl ScopedQuery {
    pub fn all(query: SearchQuery) -> Self {
        Self {
            query,
            providers: ProviderScope::All,
        }
    }

    pub fn only(query: SearchQuery, providers: impl IntoIterator<Item = AppId>) -> Self {
        Self {
            query,
            providers: ProviderScope::only(providers),
        }
    }
}

/// One caller-visible object hit with its provenance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderHit {
    pub provider: AppId,
    pub revision: DocumentRevision,
    pub hit: SearchHit,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScopedSearchResponse {
    /// Caller-visible, in-scope object hits in the canonical deterministic
    /// order of `nagi_search`.
    pub objects: Vec<ProviderHit>,
    /// Workspace metadata hits. Workspaces are not provider-owned, so they are
    /// only returned for `ProviderScope::All`.
    pub workspaces: Vec<WorkspaceHit>,
    /// Visible Workspace memberships restricted to the returned objects.
    pub workspace_groups: Vec<WorkspaceGroup>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IngestOutcome {
    /// The record was written to the index at the given revision.
    Applied,
    /// The identical record at the identical revision was already applied.
    Unchanged,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoveOutcome {
    Removed,
    /// The object was already removed at this exact revision.
    AlreadyRemoved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchCoreError {
    /// The injected authority denied the provider.
    ProviderNotAuthorized,
    /// The provider is not registered with this index.
    UnknownProvider,
    ProviderAlreadyRegistered,
    ProviderCapacity,
    LedgerCapacity,
    /// The record payload claims a source application other than the trusted
    /// provider identity.
    SourceMismatch,
    /// The record payload carries a tombstone; removal uses `remove`.
    TombstonedInput,
    /// The object identity is owned by another provider.
    ObjectOwnedByOtherProvider,
    /// The object is unknown to (or not owned by) this provider.
    UnknownObject,
    /// The update is older than the applied revision.
    StaleRevision,
    /// A different payload was submitted with an already applied revision.
    RevisionConflict,
    EmptyProviderScope,
    ProviderScopeTooLarge,
    InvalidRecord(ModelError),
    Index(MetadataStoreError),
    Search(SearchError),
    Ledger(LedgerError),
    /// The ledger snapshot could not be read while opening the index.
    LedgerLoad(BackendError),
    /// The index change is committed and the in-memory ledger reflects it, but
    /// the ledger snapshot could not be persisted. Retrying the same request
    /// or calling `persist_ledger` completes durability.
    LedgerPersist(BackendError),
}
