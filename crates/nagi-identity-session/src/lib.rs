//! C1 reference contracts for local identity and user sessions.
//!
//! This crate is intentionally outside the root Cargo workspace. It contains
//! no runtime login policy or capability policy. In-memory implementations
//! are test fakes; the Linux durable store requires an authorized directory
//! handle supplied by its integrating service.

#![forbid(unsafe_code)]

#[cfg(test)]
extern crate self as nagi_c1_session_end_adapter;
#[cfg(test)]
extern crate self as nagi_c1_session_runtime;
#[cfg(test)]
extern crate self as nagi_identity_session;

mod capability;
mod ids;
mod migration;
mod model;
mod service;
mod storage;
mod store;

#[cfg(all(feature = "linux-interactive-account", target_os = "linux"))]
mod linux_interactive_account;

#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
mod linux_runtime;

#[cfg(test)]
pub(crate) use service::caller_proof_seal;

#[cfg(test)]
#[path = "../../../tests/identity-session/contract.rs"]
mod contract;

// Keep consumer integration coverage in the C1 crate's test build so fixture
// proofs can reach the crate-private seal without exposing a production token.
#[cfg(all(test, target_os = "linux"))]
#[path = "../../../crates/nagi-c1-session-runtime/src/lib.rs"]
mod runtime_source;
#[cfg(all(test, target_os = "linux"))]
pub use runtime_source::*;
#[cfg(test)]
#[path = "../../../crates/nagi-c1-session-end-adapter/src/lib.rs"]
mod end_adapter_source;
#[cfg(test)]
pub use end_adapter_source::*;

#[cfg(all(test, target_os = "linux"))]
#[path = "../../../crates/nagi-c1-session-runtime/tests/durable_auth.rs"]
mod runtime_durable_auth_tests;
#[cfg(test)]
#[path = "../../../crates/nagi-c1-session-end-adapter/tests/routing.rs"]
mod session_end_adapter_tests;

pub use capability::{
    resolve_capability_principal, CapabilityPrincipalAdapter, DenyCapabilityPrincipalAdapter,
    PrincipalMappingFailure,
};
pub use ids::{
    CallerContextId, IdError, IdempotencyKey, PrincipalId, ProfileId, ProviderSubjectRef,
    SessionId, UserId,
};
#[cfg(all(feature = "linux-interactive-account", target_os = "linux"))]
pub use linux_interactive_account::{
    authenticate_account_for_accepted_peer,
    stage_initial_interactive_session_from_c2_owner_checked_peer, C1AccountPeerBindingError,
    C1AuthenticatedAccountPeerBinding, C1AuthenticatedInteractiveCaller,
    C1InitialInteractiveSessionError, C1IssuedInteractiveSession,
    C1PendingInitialInteractiveSession,
};
#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
pub use linux_runtime::{
    c1_linux_boot_identity_service_from_store, c1_linux_boot_identity_service_with_guest_cleaner,
    open_c1_linux_boot_identity_service, C1LinuxBootIdentityAuthorizer, C1LinuxBootIdentityService,
    C1LinuxBootIdentityServiceWithGuestCleaner, C1LinuxGuestProfileCleanupUnavailable,
    LinuxIdentityClock, LinuxIdentityEventBatch, LinuxIdentityEventReadError,
    LinuxIdentityEventReader, LinuxIdentityEventRecord, LinuxIdentityEventSink,
    LinuxIdentityIdSource,
};
#[cfg(all(
    feature = "system-service-recovery-contract",
    feature = "linux-interactive-account",
    target_os = "linux",
    target_arch = "x86_64"
))]
pub use linux_runtime::{
    provision_c1_linux_first_owner_binding,
    provision_c1_linux_first_owner_binding_with_runtime_admission, C1LinuxFirstOwnerProvisionError,
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
    AuthenticatedInteractiveCaller, AuthenticatedProvisioningCaller,
    AuthenticatedSystemServicePrincipal, AuthorizationFailure, AuthorizationRequest,
    C1PendingSessionEffectBinding, C1SessionAdmissionError, C1SessionAdmissionGate,
    C1SessionEffectBindingAbort, C1SessionEffectBindingActivation, C1SessionEffectBindingGate,
    C1SystemServiceRecoveryTerminalReceipt, CurrentIdentityResolver, EndSessionResult,
    GuestCleanupFailure, GuestProfileCleaner, IdSourceFailure, IdentityAccessContext,
    IdentityAction, IdentityAuthorizer, IdentityError, IdentityEvent, IdentityEventKind,
    IdentityEventReason, IdentityEventSink, IdentityIdSource, IdentityProvider, IdentityService,
    InteractiveSessionContext, InteractiveSessionError, LiveSnapshot, LiveStatus,
    NoopIdentityEventSink, ProviderLookupFailure, RecoveryReport, RecoverySource, ResolutionError,
    ResolvedIdentity, SessionLeaseRevalidationHandle, TrustedCallerContext, TryLiveError,
};
#[cfg(all(
    feature = "system-service-recovery-contract",
    target_os = "linux",
    target_arch = "x86_64"
))]
pub use service::{
    C1PreviousLaunchTerminalGate, C1SystemServicePrincipalIssueError,
    C1SystemServiceRuntimeAdmission, C1SystemServiceRuntimeHandoff,
    C1VerifiedBootSystemServicePrincipal,
};
#[cfg(feature = "system-service-recovery-contract")]
pub use service::{
    C1SystemServiceRecoveryEffectGate, C1SystemServiceRecoveryJournal,
    C1SystemServiceRecoveryReceiptAdapter, RecoveryEffectGateError,
};
pub use storage::{
    NoopStorageRootEventSink, ProfileRootAuthorizer, ProfileRootEffect, ProfileStorageBackend,
    StorageRootEventSink, StorageRootFailure, StorageRootHandle, UserStorageRootAdapter,
};
pub use store::{
    C1RecoveryOperationIdentity, CurrentSnapshotReader, IdentitySnapshotStore,
    InMemorySnapshotStore, SnapshotStoreError, SnapshotStoreState, SnapshotVersion, StoredSnapshot,
};
#[cfg(target_os = "linux")]
pub use store::{DurableStoreError, LinuxIdentitySnapshotStore};
