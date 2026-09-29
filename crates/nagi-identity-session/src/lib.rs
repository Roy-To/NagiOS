//! Host-only reference contracts for local identity and user sessions.
//!
//! This crate is intentionally outside the root Cargo workspace. It contains
//! no runtime login service, filesystem backend, capability policy, or host
//! path identity. In-memory implementations are test fakes only.

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
    SessionRecord, SnapshotError, UnixMillis, IDENTITY_SCHEMA_VERSION,
};
pub use service::{
    AuthorizationFailure, AuthorizationRequest, CurrentIdentityResolver, EndSessionResult,
    GuestCleanupFailure, GuestProfileCleaner, IdSourceFailure, IdentityAccessContext,
    IdentityAction, IdentityAuthorizer, IdentityError, IdentityEvent, IdentityEventKind,
    IdentityEventReason, IdentityEventSink, IdentityIdSource, IdentityProvider, IdentityService,
    NoopIdentityEventSink, ProviderLookupFailure, RecoveryReport, RecoverySource, ResolutionError,
    ResolvedIdentity, TrustedCallerContext,
};
pub use storage::{
    NoopStorageRootEventSink, ProfileRootAuthorizer, ProfileStorageBackend, StorageRootEventSink,
    StorageRootFailure, StorageRootHandle, UserStorageRootAdapter,
};
pub use store::{
    IdentitySnapshotStore, InMemorySnapshotStore, SnapshotStoreError, SnapshotVersion,
    StoredSnapshot,
};
