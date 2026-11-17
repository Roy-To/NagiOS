//! C1 reference contracts for local identity and user sessions.
//!
//! This crate is intentionally outside the root Cargo workspace. It contains
//! no runtime login policy or capability policy. In-memory implementations
//! are test fakes; the Linux durable store requires an authorized directory
//! handle supplied by its integrating service.

#![forbid(unsafe_code)]

mod capability;
mod ids;
mod migration;
mod model;
mod service;
mod storage;
mod store;

pub use capability::{
    resolve_capability_principal, CapabilityPrincipalAdapter, DenyCapabilityPrincipalAdapter,
    PrincipalMappingFailure,
};
pub use ids::{
    CallerContextId, IdError, IdempotencyKey, PrincipalId, ProfileId, ProviderSubjectRef,
    SessionId, UserId,
};
pub use migration::{
    decode_snapshot_with_migration, IdentitySnapshotMigrator, MigrationFailure,
    NoIdentitySnapshotMigrator,
};
pub use model::{
    Clock, GuestCleanupStatus, IdentitySnapshot, LocalUser, LocalUserMetadata, MetadataError,
    ProfileKind, ProfileMetadata, ProfileRecord, SessionEndReason, SessionKind, SessionLifecycle,
    SessionRecord, SnapshotError, TryClockError, UnixMillis, IDENTITY_SCHEMA_VERSION,
};
pub use service::{
    AuthenticatedInteractiveCaller, AuthenticatedSystemServicePrincipal, AuthorizationFailure,
    AuthorizationRequest, CurrentIdentityResolver, EndSessionResult, GuestCleanupFailure,
    GuestProfileCleaner, IdSourceFailure, IdentityAccessContext, IdentityAction,
    IdentityAuthorizer, IdentityError, IdentityEvent, IdentityEventKind, IdentityEventReason,
    IdentityEventSink, IdentityIdSource, IdentityProvider, IdentityService,
    InteractiveSessionContext, InteractiveSessionError, LiveSnapshot, LiveStatus,
    NoopIdentityEventSink, ProviderLookupFailure, RecoveryReport, RecoverySource, ResolutionError,
    ResolvedIdentity, SessionLeaseRevalidationHandle, TrustedCallerContext, TryLiveError,
};
pub use storage::{
    NoopStorageRootEventSink, ProfileRootAuthorizer, ProfileStorageBackend, StorageRootEventSink,
    StorageRootFailure, StorageRootHandle, UserStorageRootAdapter,
};
pub use store::{
    CurrentSnapshotReader, IdentitySnapshotStore, InMemorySnapshotStore, SnapshotStoreError,
    SnapshotStoreState, SnapshotVersion, StoredSnapshot,
};
#[cfg(target_os = "linux")]
pub use store::{DurableStoreError, LinuxIdentitySnapshotStore};
