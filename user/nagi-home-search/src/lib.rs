//! User-space contracts and deterministic host implementation for Home and
//! Search. The host preview uses explicit in-memory fixtures; target runtime
//! adapters are intentionally separate from this preview backend.

pub mod actions;
pub mod fixtures;
pub mod home;
pub mod localization;
pub mod registry;
pub mod search;

pub use actions::{ActionAvailability, CapabilityContext, CapabilityId, TypedAction};
pub use home::{HomeApp, HomeController, HomeDataSource, HomeError, HomeSnapshot};
pub use localization::{Locale, LocalizationCatalog};
pub use registry::{
    AppAvailability, AppDescriptor, AppRegistry, CompositeAppRegistry, IconMetadata,
    PackageRegistryAdapter, RegistryError, RegistrySnapshot,
};
pub use search::{
    CancellationToken, MatchReason, ProviderDescriptor, ProviderError, ProviderId,
    ProviderSelection, SearchCandidate, SearchCategory, SearchContext, SearchCoordinator,
    SearchError, SearchIdentity, SearchProvider, SearchQuery, SearchRequestId, SearchResponse,
    SearchResult, SearchText,
};
