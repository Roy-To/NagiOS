use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, RwLock, RwLockReadGuard};

use crate::model::{
    Clock, GuestCleanupStatus, IdentitySnapshot, LocalUser, LocalUserMetadata, ProfileKind,
    ProfileMetadata, ProfileRecord, SessionEndReason, SessionKind, SessionLifecycle, SessionRecord,
    SnapshotError, UnixMillis, IDENTITY_SCHEMA_VERSION,
};
use crate::store::{IdentitySnapshotStore, SnapshotStoreError, SnapshotVersion};
use crate::{CallerContextId, IdempotencyKey, ProfileId, ProviderSubjectRef, SessionId, UserId};
use crate::{CapabilityPrincipalAdapter, PrincipalId, PrincipalMappingFailure};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityAction {
    CreateLocalUser,
    CreateProfile,
    LoginLocalUser,
    CreateLocalSession,
    CreateGuestSession,
    ResolveCurrentIdentity,
    AccessProfileRoot,
    EndSession,
    Recover,
    ValidateRecoveredState,
}

/// A trusted service-host context. App payloads must never be used to
/// construct this value; the runtime adapter supplies it from trusted IPC.
pub trait TrustedCallerContext {
    fn principal_id(&self) -> &crate::PrincipalId;
    fn context_id(&self) -> &CallerContextId;
}

/// Caller contexts may be unique only inside a principal namespace. The
/// service therefore binds every in-memory current-session key to both values.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct CallerKey {
    principal_id: PrincipalId,
    context_id: CallerContextId,
}

impl CallerKey {
    fn from_trusted(caller: &impl TrustedCallerContext) -> Self {
        Self {
            principal_id: caller.principal_id().clone(),
            context_id: caller.context_id().clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationRequest {
    pub action: IdentityAction,
    pub caller_principal: crate::PrincipalId,
    pub caller_context: CallerContextId,
    pub user_id: Option<UserId>,
    pub profile_id: Option<ProfileId>,
    pub session_id: Option<SessionId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationFailure {
    Denied,
    Unavailable,
}

/// The identity crate asks the owning host for a decision. It does not own a
/// capability policy or treat persisted identity data as authority.
pub trait IdentityAuthorizer {
    fn authorize(&mut self, request: &AuthorizationRequest) -> Result<(), AuthorizationFailure>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdSourceFailure {
    Unavailable,
}

/// Production implementations must draw from an approved unpredictable
/// source and check collisions against the durable snapshot.
pub trait IdentityIdSource {
    fn next_user_id(&mut self) -> Result<UserId, IdSourceFailure>;
    fn next_profile_id(&mut self) -> Result<ProfileId, IdSourceFailure>;
    fn next_session_id(&mut self) -> Result<SessionId, IdSourceFailure>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuestCleanupFailure {
    Unavailable,
}

/// Cleanup must be idempotent and scoped to exactly the supplied ephemeral
/// ProfileId. It must not accept a host path or traverse a neighboring profile.
pub trait GuestProfileCleaner {
    fn cleanup_guest_profile(&mut self, profile_id: &ProfileId) -> Result<(), GuestCleanupFailure>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityEventKind {
    LocalUserCreated,
    ProfileCreated,
    LocalSessionCreated,
    GuestSessionCreated,
    CurrentIdentityResolved,
    CurrentIdentityResolutionDenied,
    PrincipalMappingAllowed,
    SessionEnded,
    PersistenceCommitted,
    PersistenceCommitFailed,
    RecoveryUsedLastKnownGood,
    CorruptStateRejected,
    GuestCleanupFailed,
    PrincipalMappingDenied,
    StorageRootResolutionFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityEventReason {
    NoSession,
    Ended,
    Expired,
    Corrupt,
    Unavailable,
    Conflict,
    Denied,
    CleanupPending,
    Recovery,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityEvent {
    pub kind: IdentityEventKind,
    pub user_id: Option<UserId>,
    pub profile_id: Option<ProfileId>,
    pub session_id: Option<SessionId>,
    pub reason: Option<IdentityEventReason>,
}

pub trait IdentityEventSink {
    fn emit(&mut self, event: &IdentityEvent) -> bool;
}

#[derive(Default)]
pub struct NoopIdentityEventSink;

impl IdentityEventSink for NoopIdentityEventSink {
    fn emit(&mut self, _event: &IdentityEvent) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityError {
    NotRecovered,
    AuthorizationDenied,
    AuthorizationUnavailable,
    IdSourceUnavailable,
    IdCollision,
    InvalidMetadata,
    UserNotFound,
    ProfileNotFound,
    ProfileOwnershipMismatch,
    ProfileAlreadyActive,
    CallerAlreadyHasSession,
    IdempotencyConflict,
    ExpiryNotInFuture,
    SessionNotFound,
    SessionNotCurrent,
    StoreUnavailable,
    StoreConflict,
    CorruptState,
    UnsupportedSchemaVersion,
    RevisionExhausted,
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotRecovered => "identity state has not completed recovery",
            Self::AuthorizationDenied => "identity operation was denied",
            Self::AuthorizationUnavailable => "identity authorization is unavailable",
            Self::IdSourceUnavailable => "identity ID source is unavailable",
            Self::IdCollision => "identity ID collides with an existing record",
            Self::InvalidMetadata => "identity metadata is invalid",
            Self::UserNotFound => "local user was not found",
            Self::ProfileNotFound => "profile was not found",
            Self::ProfileOwnershipMismatch => "profile does not belong to the requested user",
            Self::ProfileAlreadyActive => "profile already has an active identity session",
            Self::CallerAlreadyHasSession => "caller already has a current identity session",
            Self::IdempotencyConflict => "idempotency key conflicts with an earlier request",
            Self::ExpiryNotInFuture => "session expiry must be later than its creation time",
            Self::SessionNotFound => "session was not found",
            Self::SessionNotCurrent => "session is not current for this caller",
            Self::StoreUnavailable => "identity snapshot store is unavailable",
            Self::StoreConflict => "identity snapshot changed since it was read",
            Self::CorruptState => "identity state is corrupt and requires repair",
            Self::UnsupportedSchemaVersion => "identity state schema version is unsupported",
            Self::RevisionExhausted => "identity snapshot revision is exhausted",
        })
    }
}

impl std::error::Error for IdentityError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResolutionError {
    NoSession,
    Ended,
    Expired,
    CorruptState,
    Unavailable,
    Denied,
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoSession => "no current identity session exists for this caller",
            Self::Ended => "the caller's current identity session has ended",
            Self::Expired => "the caller's current identity session has expired",
            Self::CorruptState => "identity state is corrupt and was rejected",
            Self::Unavailable => "identity state or its dependencies are unavailable",
            Self::Denied => "current identity resolution was denied",
        })
    }
}

impl std::error::Error for ResolutionError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoverySource {
    Current,
    LastKnownGood,
    Empty,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryReport {
    pub source: RecoverySource,
    pub interrupted_sessions_ended: usize,
    pub guest_cleanups_pending: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndSessionResult {
    Ended,
    AlreadyEnded,
    GuestCleanupPending,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SessionIntent {
    kind: SessionKind,
    user_id: Option<UserId>,
    profile_id: ProfileId,
    expires_at: Option<UnixMillis>,
}

#[derive(Clone, Debug)]
struct IdempotentSession {
    session_id: SessionId,
    intent: SessionIntent,
}

/// A resolved identity is an in-process, revocable observation. It is not a
/// durable login token and it carries no capability grant by itself.
pub struct ResolvedIdentity {
    session_id: SessionId,
    kind: SessionKind,
    user_id: Option<UserId>,
    profile_id: ProfileId,
    caller: CallerKey,
    expires_at: Option<UnixMillis>,
    clock_now: Arc<dyn Fn() -> UnixMillis + Send + Sync>,
    live: Arc<RwLock<bool>>,
}

impl fmt::Debug for ResolvedIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedIdentity")
            .field("session_id", &self.session_id)
            .field("kind", &self.kind)
            .field("user_id", &self.user_id)
            .field("profile_id", &self.profile_id)
            .field("caller", &self.caller)
            .field("expires_at", &self.expires_at)
            .field("live", &self.is_live())
            .finish()
    }
}

impl ResolvedIdentity {
    pub fn session_id(&self) -> &SessionId {
        &self.session_id
    }

    pub const fn kind(&self) -> SessionKind {
        self.kind
    }

    pub const fn user_id(&self) -> Option<UserId> {
        self.user_id
    }

    pub fn profile_id(&self) -> &ProfileId {
        &self.profile_id
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.caller.principal_id
    }

    pub fn caller_context_id(&self) -> &CallerContextId {
        &self.caller.context_id
    }

    pub fn is_live(&self) -> bool {
        self.acquire_live_lease().is_some()
    }

    pub(crate) fn acquire_live_lease(&self) -> Option<RwLockReadGuard<'_, bool>> {
        let guard = self.live.read().ok()?;
        if !*guard
            || self
                .expires_at
                .is_some_and(|expires_at| (self.clock_now)() >= expires_at)
        {
            return None;
        }
        Some(guard)
    }
}

/// The service-issued identity paired with the trusted caller presented for a
/// storage operation. Storage adapters re-check the pair on every operation.
pub struct IdentityAccessContext<'a> {
    pub(crate) identity: &'a ResolvedIdentity,
    pub(crate) caller: &'a dyn TrustedCallerContext,
}

impl<'a> IdentityAccessContext<'a> {
    pub fn new(identity: &'a ResolvedIdentity, caller: &'a impl TrustedCallerContext) -> Self {
        Self { identity, caller }
    }
}

pub trait CurrentIdentityResolver {
    fn resolve_current_identity(
        &mut self,
        caller: &impl TrustedCallerContext,
    ) -> Result<ResolvedIdentity, ResolutionError>;
}

/// Provider-neutral lookup seam. Local identity remains the stable UserId;
/// federation implementations may map an external reference to it later.
pub trait IdentityProvider {
    fn resolve_subject(
        &self,
        subject: &ProviderSubjectRef,
    ) -> Result<Option<UserId>, ProviderLookupFailure>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderLookupFailure {
    Unavailable,
    Unmapped,
}

pub struct IdentityService<S, I, C, A, G, E = NoopIdentityEventSink> {
    store: S,
    ids: I,
    clock: C,
    authorizer: A,
    guest_cleaner: G,
    events: E,
    snapshot: IdentitySnapshot,
    store_version: Option<SnapshotVersion>,
    recovered: bool,
    active_by_caller: HashMap<CallerKey, SessionId>,
    last_by_caller: HashMap<CallerKey, SessionId>,
    session_owners: HashMap<SessionId, CallerKey>,
    leases: HashMap<SessionId, Arc<RwLock<bool>>>,
    idempotency: HashMap<(CallerKey, IdempotencyKey), IdempotentSession>,
}

impl<S, I, C, A, G, E> IdentityService<S, I, C, A, G, E>
where
    S: IdentitySnapshotStore,
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    pub fn new(store: S, ids: I, clock: C, authorizer: A, guest_cleaner: G, events: E) -> Self {
        Self {
            store,
            ids,
            clock,
            authorizer,
            guest_cleaner,
            events,
            snapshot: IdentitySnapshot::empty(),
            store_version: None,
            recovered: false,
            active_by_caller: HashMap::new(),
            last_by_caller: HashMap::new(),
            session_owners: HashMap::new(),
            leases: HashMap::new(),
            idempotency: HashMap::new(),
        }
    }

    pub fn snapshot(&self) -> &IdentitySnapshot {
        &self.snapshot
    }

    pub fn into_parts(self) -> (S, I, C, A, G, E) {
        (
            self.store,
            self.ids,
            self.clock,
            self.authorizer,
            self.guest_cleaner,
            self.events,
        )
    }

    fn authorization_request(
        caller: &impl TrustedCallerContext,
        action: IdentityAction,
        user_id: Option<UserId>,
        profile_id: Option<ProfileId>,
        session_id: Option<SessionId>,
    ) -> AuthorizationRequest {
        AuthorizationRequest {
            action,
            caller_principal: caller.principal_id().clone(),
            caller_context: caller.context_id().clone(),
            user_id,
            profile_id,
            session_id,
        }
    }

    fn authorize(
        &mut self,
        caller: &impl TrustedCallerContext,
        action: IdentityAction,
        user_id: Option<UserId>,
        profile_id: Option<ProfileId>,
        session_id: Option<SessionId>,
    ) -> Result<(), IdentityError> {
        let request = Self::authorization_request(caller, action, user_id, profile_id, session_id);
        self.authorizer
            .authorize(&request)
            .map_err(|error| match error {
                AuthorizationFailure::Denied => IdentityError::AuthorizationDenied,
                AuthorizationFailure::Unavailable => IdentityError::AuthorizationUnavailable,
            })
    }

    fn emit(
        &mut self,
        kind: IdentityEventKind,
        user_id: Option<UserId>,
        profile_id: Option<ProfileId>,
        session_id: Option<SessionId>,
        reason: Option<IdentityEventReason>,
    ) {
        self.events.emit(&IdentityEvent {
            kind,
            user_id,
            profile_id,
            session_id,
            reason,
        });
    }

    fn ensure_recovered(&self) -> Result<(), IdentityError> {
        if self.recovered {
            Ok(())
        } else {
            Err(IdentityError::NotRecovered)
        }
    }

    fn save_candidate(&mut self, candidate: IdentitySnapshot) -> Result<(), IdentityError> {
        let bytes = candidate.to_json().map_err(map_snapshot_error)?;
        let next_store_version = match self.store.commit_atomic(self.store_version, &bytes) {
            Ok(version) => version,
            Err(error) => {
                self.emit(
                    IdentityEventKind::PersistenceCommitFailed,
                    None,
                    None,
                    None,
                    Some(if error == SnapshotStoreError::Conflict {
                        IdentityEventReason::Conflict
                    } else {
                        IdentityEventReason::Unavailable
                    }),
                );
                return Err(map_store_error(error));
            }
        };
        self.snapshot = candidate;
        self.store_version = Some(next_store_version);
        self.emit(
            IdentityEventKind::PersistenceCommitted,
            None,
            None,
            None,
            None,
        );
        Ok(())
    }

    fn next_revision(&self) -> Result<u64, IdentityError> {
        self.snapshot
            .revision
            .checked_add(1)
            .ok_or(IdentityError::RevisionExhausted)
    }

    fn read_valid_snapshot(&self, bytes: &[u8]) -> Result<IdentitySnapshot, IdentityError> {
        IdentitySnapshot::from_json(bytes).map_err(map_snapshot_error)
    }

    /// Recovery never resumes a saved session. It selects a valid current or
    /// last-known-good snapshot, ends all active sessions, and cleans only the
    /// exact guest profile IDs recorded in that snapshot.
    pub fn recover(
        &mut self,
        caller: &impl TrustedCallerContext,
    ) -> Result<RecoveryReport, IdentityError> {
        self.authorize(caller, IdentityAction::Recover, None, None, None)?;
        let current = self.store.load_current().map_err(map_store_error)?;
        let current_version = current.as_ref().map(|snapshot| snapshot.version);
        let current = current.map(|snapshot| snapshot.bytes);
        let last_known_good = self.store.load_last_known_good().map_err(map_store_error)?;
        self.store_version = current_version;

        let (mut candidate, source) = match current {
            Some(bytes) => match self.read_valid_snapshot(&bytes) {
                Ok(snapshot) => (snapshot, RecoverySource::Current),
                Err(_) => match last_known_good {
                    Some(snapshot) => match self.read_valid_snapshot(&snapshot.bytes) {
                        Ok(snapshot) => (snapshot, RecoverySource::LastKnownGood),
                        Err(_) => {
                            self.emit(
                                IdentityEventKind::CorruptStateRejected,
                                None,
                                None,
                                None,
                                Some(IdentityEventReason::Corrupt),
                            );
                            return Err(IdentityError::CorruptState);
                        }
                    },
                    None => {
                        self.emit(
                            IdentityEventKind::CorruptStateRejected,
                            None,
                            None,
                            None,
                            Some(IdentityEventReason::Corrupt),
                        );
                        return Err(IdentityError::CorruptState);
                    }
                },
            },
            None => match last_known_good {
                Some(snapshot) => (
                    self.read_valid_snapshot(&snapshot.bytes)?,
                    RecoverySource::LastKnownGood,
                ),
                None => (IdentitySnapshot::empty(), RecoverySource::Empty),
            },
        };

        let now = self.clock.now();
        let mut ended = 0;
        for session in &mut candidate.sessions {
            if matches!(session.lifecycle, SessionLifecycle::Active) {
                session.lifecycle = SessionLifecycle::Ended {
                    at: now,
                    reason: SessionEndReason::SystemRestart,
                };
                if session.kind == SessionKind::Guest {
                    session.guest_cleanup = GuestCleanupStatus::Pending;
                }
                ended += 1;
            }
        }
        let rewrite_snapshot = source != RecoverySource::Current || ended != 0;
        if rewrite_snapshot {
            candidate.revision = candidate
                .revision
                .checked_add(1)
                .ok_or(IdentityError::RevisionExhausted)?;
        }
        let guest_ids = candidate
            .sessions
            .iter()
            .filter(|session| {
                session.kind == SessionKind::Guest
                    && session.guest_cleanup == GuestCleanupStatus::Pending
            })
            .map(|session| session.profile_id.clone())
            .collect::<Vec<_>>();

        let lease_arcs = self.leases.values().cloned().collect::<Vec<_>>();
        let mut lease_guards = lease_arcs
            .iter()
            .map(|lease| {
                lease
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
            })
            .collect::<Vec<_>>();
        if rewrite_snapshot {
            self.save_candidate(candidate)?;
        } else {
            self.snapshot = candidate;
        }
        for lease in &mut lease_guards {
            **lease = false;
        }
        drop(lease_guards);
        for (caller_key, session_id) in self.active_by_caller.drain() {
            self.last_by_caller.insert(caller_key, session_id);
        }
        self.leases.clear();
        self.session_owners.clear();
        self.idempotency.clear();
        self.recovered = false;
        self.authorize(
            caller,
            IdentityAction::ValidateRecoveredState,
            None,
            None,
            None,
        )?;
        self.recovered = true;

        if source == RecoverySource::LastKnownGood {
            self.emit(
                IdentityEventKind::RecoveryUsedLastKnownGood,
                None,
                None,
                None,
                Some(IdentityEventReason::Recovery),
            );
        }

        let mut pending = 0;
        for profile_id in guest_ids {
            if self
                .guest_cleaner
                .cleanup_guest_profile(&profile_id)
                .is_ok()
            {
                let mut next = self.snapshot.clone();
                if let Some(session) = next.sessions.iter_mut().find(|s| {
                    s.profile_id == profile_id
                        && s.kind == SessionKind::Guest
                        && s.guest_cleanup == GuestCleanupStatus::Pending
                }) {
                    session.guest_cleanup = GuestCleanupStatus::Complete;
                    next.profiles.retain(|p| p.profile_id != profile_id);
                    next.revision = self.next_revision()?;
                    if self.save_candidate(next).is_err() {
                        pending += 1;
                    }
                }
            } else {
                pending += 1;
                self.emit(
                    IdentityEventKind::GuestCleanupFailed,
                    None,
                    Some(profile_id),
                    None,
                    Some(IdentityEventReason::CleanupPending),
                );
            }
        }

        Ok(RecoveryReport {
            source,
            interrupted_sessions_ended: ended,
            guest_cleanups_pending: pending,
        })
    }

    pub fn create_local_user(
        &mut self,
        caller: &impl TrustedCallerContext,
        user_metadata: LocalUserMetadata,
        profile_metadata: ProfileMetadata,
    ) -> Result<(UserId, ProfileId), IdentityError> {
        self.ensure_recovered()?;
        user_metadata
            .validate()
            .map_err(|_| IdentityError::InvalidMetadata)?;
        profile_metadata
            .validate()
            .map_err(|_| IdentityError::InvalidMetadata)?;
        self.authorize(caller, IdentityAction::CreateLocalUser, None, None, None)?;

        let user_id = self
            .ids
            .next_user_id()
            .map_err(|_| IdentityError::IdSourceUnavailable)?;
        let profile_id = self
            .ids
            .next_profile_id()
            .map_err(|_| IdentityError::IdSourceUnavailable)?;
        if self
            .snapshot
            .users
            .iter()
            .any(|user| user.user_id == user_id)
            || self
                .snapshot
                .profiles
                .iter()
                .any(|profile| profile.profile_id == profile_id)
        {
            return Err(IdentityError::IdCollision);
        }

        let mut candidate = self.snapshot.clone();
        candidate.revision = self.next_revision()?;
        candidate.users.push(LocalUser {
            schema_version: IDENTITY_SCHEMA_VERSION,
            user_id,
            created_at: self.clock.now(),
            metadata: user_metadata,
        });
        candidate.profiles.push(ProfileRecord {
            schema_version: IDENTITY_SCHEMA_VERSION,
            profile_id: profile_id.clone(),
            owner_user_id: Some(user_id),
            kind: ProfileKind::Persistent,
            created_at: self.clock.now(),
            metadata: profile_metadata,
        });
        self.save_candidate(candidate)?;
        self.emit(
            IdentityEventKind::LocalUserCreated,
            Some(user_id),
            Some(profile_id.clone()),
            None,
            None,
        );
        Ok((user_id, profile_id))
    }

    /// A user may own multiple profiles. Each session binds to exactly one;
    /// this API does not assume a profile-selection UI.
    pub fn create_profile(
        &mut self,
        caller: &impl TrustedCallerContext,
        user_id: UserId,
        metadata: ProfileMetadata,
    ) -> Result<ProfileId, IdentityError> {
        self.ensure_recovered()?;
        metadata
            .validate()
            .map_err(|_| IdentityError::InvalidMetadata)?;
        self.authorize(
            caller,
            IdentityAction::CreateProfile,
            Some(user_id),
            None,
            None,
        )?;
        if !self
            .snapshot
            .users
            .iter()
            .any(|user| user.user_id == user_id)
        {
            return Err(IdentityError::UserNotFound);
        }
        let profile_id = self
            .ids
            .next_profile_id()
            .map_err(|_| IdentityError::IdSourceUnavailable)?;
        if self
            .snapshot
            .profiles
            .iter()
            .any(|profile| profile.profile_id == profile_id)
        {
            return Err(IdentityError::IdCollision);
        }
        let mut candidate = self.snapshot.clone();
        candidate.revision = self.next_revision()?;
        candidate.profiles.push(ProfileRecord {
            schema_version: IDENTITY_SCHEMA_VERSION,
            profile_id: profile_id.clone(),
            owner_user_id: Some(user_id),
            kind: ProfileKind::Persistent,
            created_at: self.clock.now(),
            metadata,
        });
        self.save_candidate(candidate)?;
        self.emit(
            IdentityEventKind::ProfileCreated,
            Some(user_id),
            Some(profile_id.clone()),
            None,
            None,
        );
        Ok(profile_id)
    }

    /// Local login is offline and credential-free in this reference model.
    /// Authentication decisions remain an injected trusted-host responsibility.
    pub fn login_local(
        &mut self,
        caller: &impl TrustedCallerContext,
        user_id: UserId,
        profile_id: ProfileId,
        expires_at: Option<UnixMillis>,
        idempotency_key: IdempotencyKey,
    ) -> Result<SessionId, IdentityError> {
        self.ensure_recovered()?;
        self.authorize(
            caller,
            IdentityAction::LoginLocalUser,
            Some(user_id),
            Some(profile_id.clone()),
            None,
        )?;
        self.create_session(
            caller,
            SessionIntent {
                kind: SessionKind::LocalUser,
                user_id: Some(user_id),
                profile_id,
                expires_at,
            },
            idempotency_key,
            IdentityAction::CreateLocalSession,
        )
    }

    pub fn create_guest_session(
        &mut self,
        caller: &impl TrustedCallerContext,
        expires_at: Option<UnixMillis>,
        idempotency_key: IdempotencyKey,
    ) -> Result<SessionId, IdentityError> {
        self.ensure_recovered()?;
        if expires_at.is_some_and(|expires| expires <= self.clock.now()) {
            return Err(IdentityError::ExpiryNotInFuture);
        }
        self.authorize(caller, IdentityAction::CreateGuestSession, None, None, None)?;
        let caller_key = CallerKey::from_trusted(caller);
        let idempotency_scope = (caller_key.clone(), idempotency_key.clone());
        if let Some(existing) = self.idempotency.get(&idempotency_scope) {
            let same_guest_request = existing.intent.kind == SessionKind::Guest
                && existing.intent.user_id.is_none()
                && existing.intent.expires_at == expires_at;
            if same_guest_request {
                return Ok(existing.session_id.clone());
            }
            return Err(IdentityError::IdempotencyConflict);
        }
        let profile_id = self
            .ids
            .next_profile_id()
            .map_err(|_| IdentityError::IdSourceUnavailable)?;
        if self
            .snapshot
            .profiles
            .iter()
            .any(|profile| profile.profile_id == profile_id)
        {
            return Err(IdentityError::IdCollision);
        }
        let intent = SessionIntent {
            kind: SessionKind::Guest,
            user_id: None,
            profile_id: profile_id.clone(),
            expires_at,
        };
        self.create_session(
            caller,
            intent,
            idempotency_key,
            IdentityAction::CreateGuestSession,
        )
    }

    fn create_session(
        &mut self,
        caller: &impl TrustedCallerContext,
        intent: SessionIntent,
        idempotency_key: IdempotencyKey,
        action: IdentityAction,
    ) -> Result<SessionId, IdentityError> {
        let now = self.clock.now();
        if intent.expires_at.is_some_and(|expires| expires <= now) {
            return Err(IdentityError::ExpiryNotInFuture);
        }

        let caller_key = CallerKey::from_trusted(caller);
        let idempotency_scope = (caller_key.clone(), idempotency_key.clone());
        if let Some(existing) = self.idempotency.get(&idempotency_scope) {
            if existing.intent != intent {
                return Err(IdentityError::IdempotencyConflict);
            }
            return Ok(existing.session_id.clone());
        }
        if let Some(current) = self.active_by_caller.get(&caller_key).cloned() {
            if let Some(existing) = self
                .snapshot
                .sessions
                .iter()
                .find(|session| session.session_id == current)
                .cloned()
            {
                if matches!(existing.lifecycle, SessionLifecycle::Active)
                    && existing.expires_at.is_none_or(|expires| now < expires)
                {
                    return Err(IdentityError::CallerAlreadyHasSession);
                }
                if matches!(existing.lifecycle, SessionLifecycle::Active) {
                    self.authorize(
                        caller,
                        IdentityAction::EndSession,
                        existing.user_id,
                        Some(existing.profile_id.clone()),
                        Some(existing.session_id.clone()),
                    )?;
                    self.end_record(&caller_key, &existing.session_id, SessionEndReason::Expired)?;
                } else {
                    self.active_by_caller.remove(&caller_key);
                }
            } else {
                return Err(IdentityError::CorruptState);
            }
        }

        match intent.kind {
            SessionKind::LocalUser => {
                let user_id = intent.user_id.ok_or(IdentityError::UserNotFound)?;
                if !self
                    .snapshot
                    .users
                    .iter()
                    .any(|user| user.user_id == user_id)
                {
                    return Err(IdentityError::UserNotFound);
                }
                let profile = self
                    .snapshot
                    .profiles
                    .iter()
                    .find(|profile| profile.profile_id == intent.profile_id)
                    .ok_or(IdentityError::ProfileNotFound)?;
                if profile.owner_user_id != Some(user_id) || profile.kind != ProfileKind::Persistent
                {
                    return Err(IdentityError::ProfileOwnershipMismatch);
                }
            }
            SessionKind::Guest => {
                if intent.user_id.is_some() {
                    return Err(IdentityError::ProfileOwnershipMismatch);
                }
            }
        }
        self.authorize(
            caller,
            action,
            intent.user_id,
            Some(intent.profile_id.clone()),
            None,
        )?;

        if self.snapshot.sessions.iter().any(|session| {
            session.profile_id == intent.profile_id
                && matches!(session.lifecycle, SessionLifecycle::Active)
        }) {
            return Err(IdentityError::ProfileAlreadyActive);
        }

        let session_id = self
            .ids
            .next_session_id()
            .map_err(|_| IdentityError::IdSourceUnavailable)?;
        if self
            .snapshot
            .sessions
            .iter()
            .any(|session| session.session_id == session_id)
        {
            return Err(IdentityError::IdCollision);
        }

        let mut candidate = self.snapshot.clone();
        candidate.revision = self.next_revision()?;
        if intent.kind == SessionKind::Guest {
            candidate.profiles.push(ProfileRecord {
                schema_version: IDENTITY_SCHEMA_VERSION,
                profile_id: intent.profile_id.clone(),
                owner_user_id: None,
                kind: ProfileKind::GuestEphemeral,
                created_at: now,
                metadata: ProfileMetadata::new("Guest", None)
                    .map_err(|_| IdentityError::InvalidMetadata)?,
            });
        }
        candidate.sessions.push(SessionRecord {
            schema_version: IDENTITY_SCHEMA_VERSION,
            session_id: session_id.clone(),
            kind: intent.kind,
            user_id: intent.user_id,
            profile_id: intent.profile_id.clone(),
            created_at: now,
            expires_at: intent.expires_at,
            lifecycle: SessionLifecycle::Active,
            guest_cleanup: if intent.kind == SessionKind::Guest {
                GuestCleanupStatus::Pending
            } else {
                GuestCleanupStatus::NotApplicable
            },
        });
        self.save_candidate(candidate)?;
        self.session_owners
            .insert(session_id.clone(), caller_key.clone());

        // The saved session is not authority. Revalidate after creation before
        // publishing it as current to this trusted caller context.
        if let Err(error) = self.authorize(
            caller,
            IdentityAction::ResolveCurrentIdentity,
            intent.user_id,
            Some(intent.profile_id.clone()),
            Some(session_id.clone()),
        ) {
            let reason = SessionEndReason::Recovery;
            let _ = self.end_record(&caller_key, &session_id, reason);
            return Err(error);
        }

        self.active_by_caller
            .insert(caller_key.clone(), session_id.clone());
        self.last_by_caller
            .insert(caller_key.clone(), session_id.clone());
        self.leases
            .insert(session_id.clone(), Arc::new(RwLock::new(true)));
        self.idempotency.insert(
            idempotency_scope,
            IdempotentSession {
                session_id: session_id.clone(),
                intent: intent.clone(),
            },
        );
        self.emit(
            if intent.kind == SessionKind::Guest {
                IdentityEventKind::GuestSessionCreated
            } else {
                IdentityEventKind::LocalSessionCreated
            },
            intent.user_id,
            Some(intent.profile_id),
            Some(session_id.clone()),
            None,
        );
        Ok(session_id)
    }

    pub fn end_session(
        &mut self,
        caller: &impl TrustedCallerContext,
        session_id: &SessionId,
    ) -> Result<EndSessionResult, IdentityError> {
        self.ensure_recovered()?;
        self.authorize(
            caller,
            IdentityAction::EndSession,
            None,
            None,
            Some(session_id.clone()),
        )?;
        let caller_key = CallerKey::from_trusted(caller);
        if self.active_by_caller.get(&caller_key) != Some(session_id) {
            if self.session_owners.get(session_id) != Some(&caller_key) {
                return Err(IdentityError::SessionNotCurrent);
            }
            if let Some(profile_id) = self
                .snapshot
                .sessions
                .iter()
                .find(|session| {
                    &session.session_id == session_id
                        && session.kind == SessionKind::Guest
                        && session.guest_cleanup == GuestCleanupStatus::Pending
                })
                .map(|session| session.profile_id.clone())
            {
                return self.finish_guest_cleanup(session_id, &profile_id);
            }
            return Ok(EndSessionResult::AlreadyEnded);
        }
        self.end_record(&caller_key, session_id, SessionEndReason::UserLogout)
    }

    /// Resolve the canonical capability principal for the current identity.
    /// A mapping result is not an authorization decision or a grant.
    pub fn capability_principal_for_current(
        &mut self,
        caller: &impl TrustedCallerContext,
        adapter: &impl CapabilityPrincipalAdapter,
    ) -> Result<PrincipalId, PrincipalMappingFailure> {
        let identity = match self.resolve_current_identity(caller) {
            Ok(identity) => identity,
            Err(error) => {
                let mapping_error = match error {
                    ResolutionError::NoSession => PrincipalMappingFailure::MissingIdentity,
                    ResolutionError::Ended | ResolutionError::Expired => {
                        PrincipalMappingFailure::StaleSession
                    }
                    ResolutionError::CorruptState | ResolutionError::Unavailable => {
                        PrincipalMappingFailure::Unavailable
                    }
                    ResolutionError::Denied => PrincipalMappingFailure::Denied,
                };
                self.emit(
                    IdentityEventKind::PrincipalMappingDenied,
                    None,
                    None,
                    None,
                    Some(match mapping_error {
                        PrincipalMappingFailure::MissingIdentity => IdentityEventReason::NoSession,
                        PrincipalMappingFailure::StaleSession => IdentityEventReason::Ended,
                        PrincipalMappingFailure::Denied => IdentityEventReason::Denied,
                        PrincipalMappingFailure::Unavailable => IdentityEventReason::Unavailable,
                    }),
                );
                return Err(mapping_error);
            }
        };
        match crate::resolve_capability_principal(adapter, &identity) {
            Ok(principal) => {
                self.emit(
                    IdentityEventKind::PrincipalMappingAllowed,
                    identity.user_id(),
                    Some(identity.profile_id().clone()),
                    Some(identity.session_id().clone()),
                    None,
                );
                Ok(principal)
            }
            Err(error) => {
                let reason = match error {
                    PrincipalMappingFailure::MissingIdentity => IdentityEventReason::NoSession,
                    PrincipalMappingFailure::StaleSession => IdentityEventReason::Ended,
                    PrincipalMappingFailure::Denied => IdentityEventReason::Denied,
                    PrincipalMappingFailure::Unavailable => IdentityEventReason::Unavailable,
                };
                self.emit(
                    IdentityEventKind::PrincipalMappingDenied,
                    identity.user_id(),
                    Some(identity.profile_id().clone()),
                    Some(identity.session_id().clone()),
                    Some(reason),
                );
                Err(error)
            }
        }
    }

    fn end_record(
        &mut self,
        caller_key: &CallerKey,
        session_id: &SessionId,
        reason: SessionEndReason,
    ) -> Result<EndSessionResult, IdentityError> {
        let mut candidate = self.snapshot.clone();
        let session = candidate
            .sessions
            .iter_mut()
            .find(|session| &session.session_id == session_id)
            .ok_or(IdentityError::SessionNotFound)?;
        if matches!(session.lifecycle, SessionLifecycle::Ended { .. }) {
            return Ok(EndSessionResult::AlreadyEnded);
        }
        session.lifecycle = SessionLifecycle::Ended {
            at: self.clock.now(),
            reason,
        };
        if session.kind == SessionKind::Guest {
            session.guest_cleanup = GuestCleanupStatus::Pending;
        }
        let user_id = session.user_id;
        let profile_id = session.profile_id.clone();
        let is_guest = session.kind == SessionKind::Guest;
        candidate.revision = self.next_revision()?;
        let lease = self.leases.get(session_id).cloned();
        let mut lease_guard = lease.as_ref().map(|lease| {
            lease
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        });
        self.save_candidate(candidate)?;

        if let Some(lease) = lease_guard.as_mut() {
            **lease = false;
        }
        drop(lease_guard);
        self.active_by_caller.remove(caller_key);
        self.last_by_caller
            .insert(caller_key.clone(), session_id.clone());
        self.emit(
            IdentityEventKind::SessionEnded,
            user_id,
            Some(profile_id.clone()),
            Some(session_id.clone()),
            Some(if reason == SessionEndReason::Expired {
                IdentityEventReason::Expired
            } else {
                IdentityEventReason::Ended
            }),
        );

        if is_guest {
            return self.finish_guest_cleanup(session_id, &profile_id);
        }
        Ok(EndSessionResult::Ended)
    }

    fn finish_guest_cleanup(
        &mut self,
        session_id: &SessionId,
        profile_id: &ProfileId,
    ) -> Result<EndSessionResult, IdentityError> {
        if self
            .guest_cleaner
            .cleanup_guest_profile(profile_id)
            .is_err()
        {
            self.emit(
                IdentityEventKind::GuestCleanupFailed,
                None,
                Some(profile_id.clone()),
                Some(session_id.clone()),
                Some(IdentityEventReason::CleanupPending),
            );
            return Ok(EndSessionResult::GuestCleanupPending);
        }
        let mut cleaned = self.snapshot.clone();
        let Some(session) = cleaned
            .sessions
            .iter_mut()
            .find(|session| &session.session_id == session_id)
        else {
            return Err(IdentityError::SessionNotFound);
        };
        if session.kind != SessionKind::Guest
            || !matches!(session.lifecycle, SessionLifecycle::Ended { .. })
            || session.profile_id != *profile_id
        {
            return Err(IdentityError::ProfileOwnershipMismatch);
        }
        session.guest_cleanup = GuestCleanupStatus::Complete;
        cleaned
            .profiles
            .retain(|profile| profile.profile_id != *profile_id);
        cleaned.revision = self.next_revision()?;
        if self.save_candidate(cleaned).is_err() {
            return Ok(EndSessionResult::GuestCleanupPending);
        }
        Ok(EndSessionResult::Ended)
    }

    fn expire_current(
        &mut self,
        caller: &impl TrustedCallerContext,
        session_id: &SessionId,
    ) -> Result<(), ResolutionError> {
        self.authorize(
            caller,
            IdentityAction::EndSession,
            None,
            None,
            Some(session_id.clone()),
        )
        .map_err(map_resolution_identity_error)?;
        let caller_key = CallerKey::from_trusted(caller);
        self.end_record(&caller_key, session_id, SessionEndReason::Expired)
            .map(|_| ())
            .map_err(map_resolution_identity_error)
    }

    fn require_resolved(&self) -> Result<(), ResolutionError> {
        if self.recovered {
            Ok(())
        } else {
            Err(ResolutionError::Unavailable)
        }
    }
}

impl<S, I, C, A, G, E> CurrentIdentityResolver for IdentityService<S, I, C, A, G, E>
where
    S: IdentitySnapshotStore,
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    fn resolve_current_identity(
        &mut self,
        caller: &impl TrustedCallerContext,
    ) -> Result<ResolvedIdentity, ResolutionError> {
        self.require_resolved()?;
        if let Err(error) = self.authorize(
            caller,
            IdentityAction::ResolveCurrentIdentity,
            None,
            None,
            None,
        ) {
            self.emit(
                IdentityEventKind::CurrentIdentityResolutionDenied,
                None,
                None,
                None,
                Some(if error == IdentityError::AuthorizationDenied {
                    IdentityEventReason::Denied
                } else {
                    IdentityEventReason::Unavailable
                }),
            );
            return Err(map_resolution_identity_error(error));
        }

        let caller_key = CallerKey::from_trusted(caller);
        let session_id = match self.active_by_caller.get(&caller_key).cloned() {
            Some(session_id) => session_id,
            None => {
                let ended = self
                    .last_by_caller
                    .get(&caller_key)
                    .and_then(|id| self.snapshot.sessions.iter().find(|s| &s.session_id == id));
                let (error, user_id, profile_id, session_id, reason) = match ended {
                    Some(session) => match session.lifecycle {
                        SessionLifecycle::Ended {
                            reason: SessionEndReason::Expired,
                            ..
                        } => (
                            ResolutionError::Expired,
                            session.user_id,
                            Some(session.profile_id.clone()),
                            Some(session.session_id.clone()),
                            IdentityEventReason::Expired,
                        ),
                        SessionLifecycle::Ended { .. } => (
                            ResolutionError::Ended,
                            session.user_id,
                            Some(session.profile_id.clone()),
                            Some(session.session_id.clone()),
                            IdentityEventReason::Ended,
                        ),
                        SessionLifecycle::Active => (
                            ResolutionError::CorruptState,
                            session.user_id,
                            Some(session.profile_id.clone()),
                            Some(session.session_id.clone()),
                            IdentityEventReason::Corrupt,
                        ),
                    },
                    None => (
                        ResolutionError::NoSession,
                        None,
                        None,
                        None,
                        IdentityEventReason::NoSession,
                    ),
                };
                self.emit(
                    IdentityEventKind::CurrentIdentityResolved,
                    user_id,
                    profile_id,
                    session_id,
                    Some(reason),
                );
                return Err(error);
            }
        };
        let Some(session) = self
            .snapshot
            .sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .cloned()
        else {
            return Err(ResolutionError::CorruptState);
        };
        match session.lifecycle {
            SessionLifecycle::Ended {
                reason: SessionEndReason::Expired,
                ..
            } => return Err(ResolutionError::Expired),
            SessionLifecycle::Ended { .. } => return Err(ResolutionError::Ended),
            SessionLifecycle::Active => {}
        }
        if session
            .expires_at
            .is_some_and(|expires| self.clock.now() >= expires)
        {
            self.expire_current(caller, &session_id)?;
            self.emit(
                IdentityEventKind::CurrentIdentityResolved,
                session.user_id,
                Some(session.profile_id),
                Some(session_id),
                Some(IdentityEventReason::Expired),
            );
            return Err(ResolutionError::Expired);
        }
        let live = self
            .leases
            .entry(session_id.clone())
            .or_insert_with(|| Arc::new(RwLock::new(true)))
            .clone();
        let identity = ResolvedIdentity {
            session_id: session_id.clone(),
            kind: session.kind,
            user_id: session.user_id,
            profile_id: session.profile_id.clone(),
            caller: caller_key,
            expires_at: session.expires_at,
            clock_now: {
                let clock = self.clock.clone();
                Arc::new(move || clock.now())
            },
            live,
        };
        if !identity.is_live() {
            return Err(ResolutionError::Ended);
        }
        self.emit(
            IdentityEventKind::CurrentIdentityResolved,
            session.user_id,
            Some(session.profile_id),
            Some(session_id),
            None,
        );
        Ok(identity)
    }
}

fn map_store_error(error: SnapshotStoreError) -> IdentityError {
    match error {
        SnapshotStoreError::Unavailable => IdentityError::StoreUnavailable,
        SnapshotStoreError::Conflict => IdentityError::StoreConflict,
    }
}

fn map_snapshot_error(error: SnapshotError) -> IdentityError {
    match error {
        SnapshotError::UnsupportedVersion(_) => IdentityError::UnsupportedSchemaVersion,
        _ => IdentityError::CorruptState,
    }
}

fn map_resolution_identity_error(error: IdentityError) -> ResolutionError {
    match error {
        IdentityError::AuthorizationDenied => ResolutionError::Denied,
        IdentityError::CorruptState | IdentityError::UnsupportedSchemaVersion => {
            ResolutionError::CorruptState
        }
        IdentityError::NotRecovered
        | IdentityError::StoreUnavailable
        | IdentityError::StoreConflict
        | IdentityError::AuthorizationUnavailable
        | IdentityError::IdSourceUnavailable
        | IdentityError::IdCollision
        | IdentityError::InvalidMetadata
        | IdentityError::UserNotFound
        | IdentityError::ProfileNotFound
        | IdentityError::ProfileOwnershipMismatch
        | IdentityError::ProfileAlreadyActive
        | IdentityError::CallerAlreadyHasSession
        | IdentityError::IdempotencyConflict
        | IdentityError::ExpiryNotInFuture
        | IdentityError::SessionNotFound
        | IdentityError::SessionNotCurrent
        | IdentityError::RevisionExhausted => ResolutionError::Unavailable,
    }
}
