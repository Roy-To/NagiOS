use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use nagi_identity_session::{
    decode_snapshot_with_migration, resolve_capability_principal, AuthorizationFailure,
    AuthorizationRequest, CallerContextId, CapabilityPrincipalAdapter, Clock,
    CurrentIdentityResolver, DenyCapabilityPrincipalAdapter, EndSessionResult, GuestCleanupFailure,
    GuestProfileCleaner, IdSourceFailure, IdentityAccessContext, IdentityAction,
    IdentityAuthorizer, IdentityError, IdentityIdSource, IdentityProvider, IdentityService,
    IdentitySnapshot, IdentitySnapshotMigrator, IdentitySnapshotStore, InMemorySnapshotStore,
    LocalUserMetadata, MigrationFailure, NoIdentitySnapshotMigrator, NoopIdentityEventSink,
    PrincipalId, PrincipalMappingFailure, ProfileId, ProfileMetadata, ProfileRootAuthorizer,
    ProfileStorageBackend, ProviderLookupFailure, ProviderSubjectRef, RecoverySource,
    ResolutionError, ResolvedIdentity, SessionId, SnapshotStoreError, SnapshotVersion,
    StorageRootFailure, StoredSnapshot, TrustedCallerContext, UnixMillis, UserId,
    UserStorageRootAdapter,
};
use nagi_model::{AppSessionId, ExecutionInstanceId, ObjectId, SurfaceId};

#[derive(Clone)]
struct TestCaller {
    principal: PrincipalId,
    context: CallerContextId,
}

impl TestCaller {
    fn new(context: &str) -> Self {
        Self::with_principal(context, "system.identity-test")
    }

    fn with_principal(context: &str, principal: &str) -> Self {
        Self {
            principal: PrincipalId::new(principal).unwrap(),
            context: CallerContextId::new(format!("caller_{context}")).unwrap(),
        }
    }
}

impl TrustedCallerContext for TestCaller {
    fn principal_id(&self) -> &PrincipalId {
        &self.principal
    }

    fn context_id(&self) -> &CallerContextId {
        &self.context
    }
}

#[derive(Clone, Default)]
struct SharedStore(Arc<Mutex<StoreState>>);

#[derive(Default)]
struct StoreState {
    current: Option<StoredSnapshot>,
    last_good: Option<StoredSnapshot>,
    fail_next_commit: bool,
    fail_reads: bool,
    read_barrier: Option<Arc<std::sync::Barrier>>,
}

impl SharedStore {
    fn fail_next_commit(&self) {
        self.0.lock().unwrap().fail_next_commit = true;
    }

    fn corrupt_current(&self, bytes: Vec<u8>) {
        let mut state = self.0.lock().unwrap();
        let version = state
            .current
            .as_ref()
            .map(|snapshot| snapshot.version)
            .unwrap_or(SnapshotVersion::new(1));
        state.current = Some(StoredSnapshot { bytes, version });
    }

    fn current(&self) -> Option<Vec<u8>> {
        self.0
            .lock()
            .unwrap()
            .current
            .as_ref()
            .map(|snapshot| snapshot.bytes.clone())
    }
}

impl IdentitySnapshotStore for SharedStore {
    fn current_snapshot_reader(&self) -> Option<nagi_identity_session::CurrentSnapshotReader> {
        let store = self.clone();
        Some(Arc::new(move || store.load_current()))
    }

    fn load_current(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
        let state = self.0.lock().unwrap();
        if state.fail_reads {
            return Err(SnapshotStoreError::Unavailable);
        }
        let current = state.current.clone();
        let barrier = state.read_barrier.clone();
        drop(state);
        if let Some(barrier) = barrier {
            barrier.wait();
        }
        Ok(current)
    }

    fn load_last_known_good(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
        Ok(self.0.lock().unwrap().last_good.clone())
    }

    fn commit_atomic(
        &mut self,
        expected_version: Option<SnapshotVersion>,
        bytes: &[u8],
    ) -> Result<SnapshotVersion, SnapshotStoreError> {
        IdentitySnapshot::from_json(bytes).map_err(|_| SnapshotStoreError::Unavailable)?;
        let mut state = self.0.lock().unwrap();
        if state.fail_next_commit {
            state.fail_next_commit = false;
            return Err(SnapshotStoreError::Unavailable);
        }
        if state.current.as_ref().map(|snapshot| snapshot.version) != expected_version {
            return Err(SnapshotStoreError::Conflict);
        }
        let next_version = SnapshotVersion::new(match expected_version {
            Some(version) => version
                .get()
                .checked_add(1)
                .ok_or(SnapshotStoreError::Unavailable)?,
            None => 1,
        });
        if let Some(current) = &state.current {
            if IdentitySnapshot::from_json(&current.bytes).is_ok() {
                state.last_good = Some(current.clone());
            }
        }
        state.current = Some(StoredSnapshot {
            bytes: bytes.to_vec(),
            version: next_version,
        });
        Ok(next_version)
    }
}

#[derive(Clone)]
struct TestClock(Arc<AtomicU64>);

impl TestClock {
    fn set(&self, now: u64) {
        self.0.store(now, Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now(&self) -> UnixMillis {
        UnixMillis::new(self.0.load(Ordering::SeqCst))
    }
}

#[derive(Default)]
struct TestIds {
    user: u64,
    profile: u64,
    session: u64,
    fail: bool,
}

impl IdentityIdSource for TestIds {
    fn next_user_id(&mut self) -> Result<UserId, IdSourceFailure> {
        if self.fail {
            return Err(IdSourceFailure::Unavailable);
        }
        self.user += 1;
        UserId::new(self.user).map_err(|_| IdSourceFailure::Unavailable)
    }

    fn next_profile_id(&mut self) -> Result<ProfileId, IdSourceFailure> {
        if self.fail {
            return Err(IdSourceFailure::Unavailable);
        }
        self.profile += 1;
        ProfileId::new(format!("profile_p{}", self.profile))
            .map_err(|_| IdSourceFailure::Unavailable)
    }

    fn next_session_id(&mut self) -> Result<SessionId, IdSourceFailure> {
        if self.fail {
            return Err(IdSourceFailure::Unavailable);
        }
        self.session += 1;
        SessionId::new(format!("session_s{}", self.session))
            .map_err(|_| IdSourceFailure::Unavailable)
    }
}

#[derive(Default)]
struct TestAuthorizer {
    denied: Option<IdentityAction>,
    unavailable: Option<IdentityAction>,
}

impl IdentityAuthorizer for TestAuthorizer {
    fn authorize(&mut self, request: &AuthorizationRequest) -> Result<(), AuthorizationFailure> {
        if self.unavailable == Some(request.action) {
            return Err(AuthorizationFailure::Unavailable);
        }
        if self.denied == Some(request.action) {
            return Err(AuthorizationFailure::Denied);
        }
        Ok(())
    }
}

#[derive(Clone, Default)]
struct TestCleaner {
    cleaned: Arc<Mutex<Vec<ProfileId>>>,
    fail: Arc<std::sync::atomic::AtomicBool>,
}

impl GuestProfileCleaner for TestCleaner {
    fn cleanup_guest_profile(&mut self, profile_id: &ProfileId) -> Result<(), GuestCleanupFailure> {
        if self.fail.load(Ordering::SeqCst) {
            return Err(GuestCleanupFailure::Unavailable);
        }
        self.cleaned.lock().unwrap().push(profile_id.clone());
        Ok(())
    }
}

type TestService = IdentityService<
    SharedStore,
    TestIds,
    TestClock,
    TestAuthorizer,
    TestCleaner,
    NoopIdentityEventSink,
>;

fn new_service(store: SharedStore, cleaner: TestCleaner) -> (TestService, TestClock) {
    new_service_with_authorizer(store, cleaner, TestAuthorizer::default())
}

fn new_service_with_authorizer(
    store: SharedStore,
    cleaner: TestCleaner,
    authorizer: TestAuthorizer,
) -> (TestService, TestClock) {
    let clock = TestClock(Arc::new(AtomicU64::new(100)));
    let service = IdentityService::new(
        store,
        TestIds::default(),
        clock.clone(),
        authorizer,
        cleaner,
        NoopIdentityEventSink,
    );
    (service, clock)
}

fn metadata(name: &str) -> (LocalUserMetadata, ProfileMetadata) {
    (
        LocalUserMetadata::new(name).unwrap(),
        ProfileMetadata::new(name, Some("en-US".to_owned())).unwrap(),
    )
}

fn recover(service: &mut TestService, caller: &TestCaller) {
    service.recover(caller).unwrap();
}

struct AllowProfileRoot;

impl ProfileRootAuthorizer for AllowProfileRoot {
    fn authorize_profile_root(
        &mut self,
        _caller: &dyn TrustedCallerContext,
        _identity: &ResolvedIdentity,
        _requested_profile: &ProfileId,
    ) -> Result<(), StorageRootFailure> {
        Ok(())
    }
}

struct DenyProfileRoot;

impl ProfileRootAuthorizer for DenyProfileRoot {
    fn authorize_profile_root(
        &mut self,
        _caller: &dyn TrustedCallerContext,
        _identity: &ResolvedIdentity,
        _requested_profile: &ProfileId,
    ) -> Result<(), StorageRootFailure> {
        Err(StorageRootFailure::Denied)
    }
}

#[derive(Clone, Debug)]
struct MemoryRoot(ProfileId);

#[derive(Clone, Default)]
struct MemoryProfileStorage {
    data: HashMap<(ProfileId, u64), Vec<u8>>,
    opens: Arc<AtomicUsize>,
}

impl ProfileStorageBackend for MemoryProfileStorage {
    type Root = MemoryRoot;

    fn open_profile_root(
        &mut self,
        _user_id: Option<UserId>,
        profile_id: &ProfileId,
    ) -> Result<Self::Root, StorageRootFailure> {
        self.opens.fetch_add(1, Ordering::SeqCst);
        Ok(MemoryRoot(profile_id.clone()))
    }

    fn read_object(
        &mut self,
        root: &Self::Root,
        object_id: ObjectId,
    ) -> Result<Vec<u8>, StorageRootFailure> {
        self.data
            .get(&(root.0.clone(), object_id.0))
            .cloned()
            .ok_or(StorageRootFailure::Unavailable)
    }

    fn write_object(
        &mut self,
        root: &Self::Root,
        object_id: ObjectId,
        data: &[u8],
    ) -> Result<(), StorageRootFailure> {
        self.data
            .insert((root.0.clone(), object_id.0), data.to_vec());
        Ok(())
    }
}

struct FakePrincipalAdapter(Result<PrincipalId, PrincipalMappingFailure>);

impl CapabilityPrincipalAdapter for FakePrincipalAdapter {
    fn principal_for(
        &self,
        identity: &ResolvedIdentity,
    ) -> Result<PrincipalId, PrincipalMappingFailure> {
        if !identity.is_live() {
            return Err(PrincipalMappingFailure::StaleSession);
        }
        self.0.clone()
    }
}

#[test]
fn identity_ids_are_validated_and_remain_distinct_from_app_runtime_ids() {
    assert!(UserId::new(0).is_err());
    assert!(ProfileId::new("session_s1").is_err());
    assert!(SessionId::new("profile_p1").is_err());
    assert!(SessionId::new("session_é").is_err());
    assert!(PrincipalId::new("Owner:Root").is_err());

    let user = UserId::new(7).unwrap();
    assert_eq!(UserId::from_model(user.to_model()).unwrap(), user);
    let app_session = AppSessionId(7);
    let execution = ExecutionInstanceId(7);
    let surface = SurfaceId(7);
    assert_eq!(app_session.0, execution.0);
    assert_eq!(execution.0, surface.0);
    // The equal payloads above remain different Rust types and cannot be
    // passed to APIs taking UserId, ProfileId, or SessionId.
    assert_eq!(user.get(), 7);
}

#[test]
fn offline_local_user_can_own_multiple_profiles_and_login_logout_is_idempotent() {
    let store = SharedStore::default();
    let (mut service, _) = new_service(store, TestCleaner::default());
    let caller = TestCaller::new("desktop");
    recover(&mut service, &caller);
    let (user_id, profile_a) = service
        .create_local_user(&caller, metadata("Nagi User").0, metadata("Home").1)
        .unwrap();
    let profile_b = service
        .create_profile(
            &caller,
            user_id,
            ProfileMetadata::new("Work", Some("ja-JP".into())).unwrap(),
        )
        .unwrap();
    assert_ne!(profile_a, profile_b);

    let session_id = service
        .login_local(
            &caller,
            user_id,
            profile_a.clone(),
            None,
            nagi_identity_session::IdempotencyKey::new("login-1").unwrap(),
        )
        .unwrap();
    let identity = service.resolve_current_identity(&caller).unwrap();
    assert_eq!(identity.user_id(), Some(user_id));
    assert_eq!(identity.profile_id(), &profile_a);
    assert_eq!(identity.session_id(), &session_id);

    assert_eq!(
        service.end_session(&caller, &session_id).unwrap(),
        EndSessionResult::Ended
    );
    assert!(!identity.is_live());
    assert_eq!(
        service.end_session(&caller, &session_id).unwrap(),
        EndSessionResult::AlreadyEnded
    );
    assert_eq!(
        service
            .login_local(
                &caller,
                user_id,
                profile_a,
                None,
                nagi_identity_session::IdempotencyKey::new("login-1").unwrap(),
            )
            .unwrap(),
        session_id
    );
    assert_eq!(
        service.resolve_current_identity(&caller).unwrap_err(),
        ResolutionError::Ended
    );
}

#[test]
fn guest_cleanup_only_targets_its_ephemeral_profile() {
    let cleaner = TestCleaner::default();
    let cleaned = cleaner.cleaned.clone();
    let cleanup_fails = cleaner.fail.clone();
    let store = SharedStore::default();
    let (mut service, _) = new_service(store, cleaner);
    let local_caller = TestCaller::new("local");
    let guest_caller = TestCaller::new("guest");
    recover(&mut service, &local_caller);
    let (user_id, persistent_profile) = service
        .create_local_user(&local_caller, metadata("Owner").0, metadata("Main").1)
        .unwrap();
    let guest_session = service
        .create_guest_session(
            &guest_caller,
            None,
            nagi_identity_session::IdempotencyKey::new("guest-1").unwrap(),
        )
        .unwrap();
    assert_eq!(
        service
            .create_guest_session(
                &guest_caller,
                None,
                nagi_identity_session::IdempotencyKey::new("guest-1").unwrap(),
            )
            .unwrap(),
        guest_session
    );
    let guest_profile = service
        .snapshot()
        .sessions
        .iter()
        .find(|session| session.session_id == guest_session)
        .unwrap()
        .profile_id
        .clone();
    assert_ne!(persistent_profile, guest_profile);

    cleanup_fails.store(true, Ordering::SeqCst);
    assert_eq!(
        service.end_session(&guest_caller, &guest_session).unwrap(),
        EndSessionResult::GuestCleanupPending
    );
    cleanup_fails.store(false, Ordering::SeqCst);
    assert_eq!(
        service.end_session(&guest_caller, &guest_session).unwrap(),
        EndSessionResult::Ended
    );
    assert_eq!(*cleaned.lock().unwrap(), vec![guest_profile.clone()]);
    assert!(service.snapshot().profiles.iter().any(|profile| {
        profile.profile_id == persistent_profile && profile.owner_user_id == Some(user_id)
    }));
    assert!(!service
        .snapshot()
        .profiles
        .iter()
        .any(|profile| profile.profile_id == guest_profile));
}

#[test]
fn pending_guest_cleanup_can_retry_after_the_same_caller_starts_another_session() {
    let cleaner = TestCleaner::default();
    let cleaned = cleaner.cleaned.clone();
    let cleanup_fails = cleaner.fail.clone();
    let (mut service, _) = new_service(SharedStore::default(), cleaner);
    let caller = TestCaller::new("guest-retry-after-next");
    recover(&mut service, &caller);
    let guest_a = service
        .create_guest_session(
            &caller,
            None,
            nagi_identity_session::IdempotencyKey::new("guest-a").unwrap(),
        )
        .unwrap();
    let profile_a = service
        .snapshot()
        .sessions
        .iter()
        .find(|session| session.session_id == guest_a)
        .unwrap()
        .profile_id
        .clone();
    cleanup_fails.store(true, Ordering::SeqCst);
    assert_eq!(
        service.end_session(&caller, &guest_a).unwrap(),
        EndSessionResult::GuestCleanupPending
    );

    let guest_b = service
        .create_guest_session(
            &caller,
            None,
            nagi_identity_session::IdempotencyKey::new("guest-b").unwrap(),
        )
        .unwrap();
    let profile_b = service
        .snapshot()
        .sessions
        .iter()
        .find(|session| session.session_id == guest_b)
        .unwrap()
        .profile_id
        .clone();
    assert_ne!(profile_a, profile_b);

    cleanup_fails.store(false, Ordering::SeqCst);
    assert_eq!(
        service.end_session(&caller, &guest_a).unwrap(),
        EndSessionResult::Ended
    );
    assert_eq!(*cleaned.lock().unwrap(), vec![profile_a.clone()]);
    assert!(!service
        .snapshot()
        .profiles
        .iter()
        .any(|profile| profile.profile_id == profile_a));
    assert!(service
        .snapshot()
        .profiles
        .iter()
        .any(|profile| profile.profile_id == profile_b));
    assert_eq!(
        service
            .resolve_current_identity(&caller)
            .unwrap()
            .session_id(),
        &guest_b
    );

    assert_eq!(
        service.end_session(&caller, &guest_b).unwrap(),
        EndSessionResult::Ended
    );
    assert_eq!(*cleaned.lock().unwrap(), vec![profile_a, profile_b]);
}

#[test]
fn storage_handle_is_profile_and_session_scoped_and_stale_use_is_denied() {
    let store = SharedStore::default();
    let (mut service, _) = new_service(store, TestCleaner::default());
    let owner = TestCaller::new("owner");
    let second = TestCaller::new("second");
    recover(&mut service, &owner);
    let (user_id, profile_a) = service
        .create_local_user(&owner, metadata("User").0, metadata("A").1)
        .unwrap();
    let profile_b = service
        .create_profile(&owner, user_id, ProfileMetadata::new("B", None).unwrap())
        .unwrap();
    let session_a = service
        .login_local(
            &owner,
            user_id,
            profile_a.clone(),
            None,
            nagi_identity_session::IdempotencyKey::new("login-a").unwrap(),
        )
        .unwrap();
    let identity_a = service.resolve_current_identity(&owner).unwrap();
    let mut roots = UserStorageRootAdapter::new(MemoryProfileStorage::default());
    let mut root_auth = AllowProfileRoot;
    let handle_a = roots
        .resolve_root(
            &IdentityAccessContext::new(&identity_a, &owner),
            &profile_a,
            &mut root_auth,
        )
        .unwrap();
    assert_eq!(
        service.login_local(
            &second,
            user_id,
            profile_a.clone(),
            None,
            nagi_identity_session::IdempotencyKey::new("same-profile").unwrap(),
        ),
        Err(IdentityError::ProfileAlreadyActive)
    );
    roots
        .write_object(
            &IdentityAccessContext::new(&identity_a, &owner),
            &handle_a,
            &profile_a,
            ObjectId(9),
            b"profile-a",
            &mut root_auth,
        )
        .unwrap();
    service.end_session(&owner, &session_a).unwrap();

    assert_eq!(
        roots.read_object(
            &IdentityAccessContext::new(&identity_a, &owner),
            &handle_a,
            &profile_a,
            ObjectId(9),
            &mut root_auth,
        ),
        Err(StorageRootFailure::StaleSession)
    );

    let session_b = service
        .login_local(
            &second,
            user_id,
            profile_b.clone(),
            None,
            nagi_identity_session::IdempotencyKey::new("login-b").unwrap(),
        )
        .unwrap();
    let identity_b = service.resolve_current_identity(&second).unwrap();
    assert_eq!(
        roots.read_object(
            &IdentityAccessContext::new(&identity_b, &second),
            &handle_a,
            &profile_b,
            ObjectId(9),
            &mut root_auth,
        ),
        Err(StorageRootFailure::CrossProfile)
    );
    assert_eq!(
        roots
            .resolve_root(
                &IdentityAccessContext::new(&identity_b, &second),
                &profile_a,
                &mut root_auth,
            )
            .unwrap_err(),
        StorageRootFailure::CrossProfile
    );
    assert_eq!(roots.into_backend().opens.load(Ordering::SeqCst), 1);
    service.end_session(&second, &session_b).unwrap();
}

#[test]
fn issued_identity_and_profile_handle_expire_without_a_fresh_resolve() {
    let (mut service, clock) = new_service(SharedStore::default(), TestCleaner::default());
    let caller = TestCaller::new("deadline");
    recover(&mut service, &caller);
    let (user_id, profile_id) = service
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    service
        .login_local(
            &caller,
            user_id,
            profile_id.clone(),
            Some(UnixMillis::new(200)),
            nagi_identity_session::IdempotencyKey::new("deadline-login").unwrap(),
        )
        .unwrap();
    let identity = service.resolve_current_identity(&caller).unwrap();
    let backend = MemoryProfileStorage::default();
    let opens = backend.opens.clone();
    let mut roots = UserStorageRootAdapter::new(backend);
    let mut root_auth = AllowProfileRoot;
    let handle = roots
        .resolve_root(
            &IdentityAccessContext::new(&identity, &caller),
            &profile_id,
            &mut root_auth,
        )
        .unwrap();
    roots
        .write_object(
            &IdentityAccessContext::new(&identity, &caller),
            &handle,
            &profile_id,
            ObjectId(1),
            b"before-expiry",
            &mut root_auth,
        )
        .unwrap();

    clock.set(200);
    assert!(!identity.is_live());
    assert_eq!(
        roots.read_object(
            &IdentityAccessContext::new(&identity, &caller),
            &handle,
            &profile_id,
            ObjectId(1),
            &mut root_auth,
        ),
        Err(StorageRootFailure::StaleSession)
    );
    assert_eq!(
        roots.write_object(
            &IdentityAccessContext::new(&identity, &caller),
            &handle,
            &profile_id,
            ObjectId(2),
            b"after-expiry",
            &mut root_auth,
        ),
        Err(StorageRootFailure::StaleSession)
    );
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

#[test]
fn same_service_recovery_revokes_issued_identity_and_storage_leases() {
    let (mut service, _) = new_service(SharedStore::default(), TestCleaner::default());
    let caller = TestCaller::new("same-service-recovery");
    recover(&mut service, &caller);
    let (user_id, profile_id) = service
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    service
        .login_local(
            &caller,
            user_id,
            profile_id.clone(),
            None,
            nagi_identity_session::IdempotencyKey::new("recovery-login").unwrap(),
        )
        .unwrap();
    let identity = service.resolve_current_identity(&caller).unwrap();
    let backend = MemoryProfileStorage::default();
    let opens = backend.opens.clone();
    let mut roots = UserStorageRootAdapter::new(backend);
    let mut root_auth = AllowProfileRoot;
    let handle = roots
        .resolve_root(
            &IdentityAccessContext::new(&identity, &caller),
            &profile_id,
            &mut root_auth,
        )
        .unwrap();

    let report = service.recover(&caller).unwrap();
    assert_eq!(report.interrupted_sessions_ended, 1);
    assert!(!identity.is_live());
    assert_eq!(
        service.resolve_current_identity(&caller).unwrap_err(),
        ResolutionError::Ended
    );
    assert_eq!(
        roots.read_object(
            &IdentityAccessContext::new(&identity, &caller),
            &handle,
            &profile_id,
            ObjectId(3),
            &mut root_auth,
        ),
        Err(StorageRootFailure::StaleSession)
    );
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

#[test]
fn shared_store_compare_and_swap_rejects_a_stale_service_write() {
    let store = SharedStore::default();
    let (mut setup, _) = new_service(store.clone(), TestCleaner::default());
    let setup_caller = TestCaller::new("cas-setup");
    recover(&mut setup, &setup_caller);
    let (user_id, profile_id) = setup
        .create_local_user(&setup_caller, metadata("User").0, metadata("Home").1)
        .unwrap();

    let (mut service_a, _) = new_service(store.clone(), TestCleaner::default());
    let (mut service_b, _) = new_service(store.clone(), TestCleaner::default());
    let caller_a = TestCaller::new("cas-a");
    let caller_b = TestCaller::new("cas-b");
    recover(&mut service_a, &caller_a);
    recover(&mut service_b, &caller_b);

    let created = service_a
        .login_local(
            &caller_a,
            user_id,
            profile_id.clone(),
            None,
            nagi_identity_session::IdempotencyKey::new("cas-login-a").unwrap(),
        )
        .unwrap();
    assert_eq!(
        service_b.login_local(
            &caller_b,
            user_id,
            profile_id,
            None,
            nagi_identity_session::IdempotencyKey::new("cas-login-b").unwrap(),
        ),
        Err(IdentityError::StoreConflict)
    );
    let committed = IdentitySnapshot::from_json(&store.current().unwrap()).unwrap();
    let active = committed
        .sessions
        .iter()
        .filter(|session| {
            matches!(
                session.lifecycle,
                nagi_identity_session::SessionLifecycle::Active
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].session_id, created);
}

#[test]
fn caller_context_keys_are_bound_to_their_principal() {
    let (mut service, _) = new_service(SharedStore::default(), TestCleaner::default());
    let caller = TestCaller::new("same-context");
    let other_principal = TestCaller::with_principal("same-context", "system.other");
    recover(&mut service, &caller);
    let (user_id, profile_id) = service
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    service
        .login_local(
            &caller,
            user_id,
            profile_id.clone(),
            None,
            nagi_identity_session::IdempotencyKey::new("caller-binding").unwrap(),
        )
        .unwrap();
    let identity = service.resolve_current_identity(&caller).unwrap();
    assert_eq!(
        service
            .resolve_current_identity(&other_principal)
            .unwrap_err(),
        ResolutionError::NoSession
    );

    let backend = MemoryProfileStorage::default();
    let opens = backend.opens.clone();
    let mut roots = UserStorageRootAdapter::new(backend);
    assert_eq!(
        roots
            .resolve_root(
                &IdentityAccessContext::new(&identity, &other_principal),
                &profile_id,
                &mut AllowProfileRoot,
            )
            .unwrap_err(),
        StorageRootFailure::CallerMismatch
    );
    assert_eq!(opens.load(Ordering::SeqCst), 0);
}

#[test]
fn principal_mapping_fails_closed_and_never_grants_by_identifier_alone() {
    let store = SharedStore::default();
    let (mut service, _) = new_service(store, TestCleaner::default());
    let caller = TestCaller::new("principal");
    recover(&mut service, &caller);
    let (user_id, profile_id) = service
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    service
        .login_local(
            &caller,
            user_id,
            profile_id,
            None,
            nagi_identity_session::IdempotencyKey::new("principal-login").unwrap(),
        )
        .unwrap();
    let identity = service.resolve_current_identity(&caller).unwrap();
    assert_eq!(
        resolve_capability_principal(&DenyCapabilityPrincipalAdapter, &identity),
        Err(PrincipalMappingFailure::Unavailable)
    );
    let allowed_id = PrincipalId::new("user:opaque-1").unwrap();
    assert_eq!(
        resolve_capability_principal(&FakePrincipalAdapter(Ok(allowed_id.clone())), &identity),
        Ok(allowed_id)
    );
    service.end_session(&caller, identity.session_id()).unwrap();
    assert_eq!(
        resolve_capability_principal(
            &FakePrincipalAdapter(Ok(PrincipalId::new("user:opaque-1").unwrap())),
            &identity,
        ),
        Err(PrincipalMappingFailure::StaleSession)
    );
}

#[test]
fn expiry_is_deterministic_and_stale_sessions_do_not_resolve() {
    let store = SharedStore::default();
    let (mut service, clock) = new_service(store, TestCleaner::default());
    let caller = TestCaller::new("expiry");
    recover(&mut service, &caller);
    let (user_id, profile_id) = service
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    let session = service
        .login_local(
            &caller,
            user_id,
            profile_id,
            Some(UnixMillis::new(200)),
            nagi_identity_session::IdempotencyKey::new("expiry-login").unwrap(),
        )
        .unwrap();
    clock.set(200);
    assert_eq!(
        service.resolve_current_identity(&caller).unwrap_err(),
        ResolutionError::Expired
    );
    assert_eq!(
        service.end_session(&caller, &session).unwrap(),
        EndSessionResult::AlreadyEnded
    );
}

#[test]
fn duplicate_concurrent_login_returns_one_current_session_and_end_is_idempotent() {
    let store = SharedStore::default();
    let (mut service, _) = new_service(store, TestCleaner::default());
    let caller = TestCaller::new("concurrent");
    recover(&mut service, &caller);
    let (user_id, profile_id) = service
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    let service = Arc::new(Mutex::new(service));
    let threads = (0..8)
        .map(|_| {
            let service = Arc::clone(&service);
            let caller = caller.clone();
            let profile_id = profile_id.clone();
            thread::spawn(move || {
                service
                    .lock()
                    .unwrap()
                    .login_local(
                        &caller,
                        user_id,
                        profile_id,
                        None,
                        nagi_identity_session::IdempotencyKey::new("same-request").unwrap(),
                    )
                    .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let sessions = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect::<Vec<_>>();
    assert!(sessions.iter().all(|session| session == &sessions[0]));
    assert_eq!(
        service
            .lock()
            .unwrap()
            .snapshot()
            .sessions
            .iter()
            .filter(|s| {
                s.kind == nagi_identity_session::SessionKind::LocalUser
                    && matches!(s.lifecycle, nagi_identity_session::SessionLifecycle::Active)
            })
            .count(),
        1
    );
    assert_eq!(
        service
            .lock()
            .unwrap()
            .end_session(&caller, &sessions[0])
            .unwrap(),
        EndSessionResult::Ended
    );
    assert_eq!(
        service
            .lock()
            .unwrap()
            .end_session(&caller, &sessions[0])
            .unwrap(),
        EndSessionResult::AlreadyEnded
    );
}

#[test]
fn interrupted_commit_preserves_the_previous_complete_snapshot() {
    let store = SharedStore::default();
    let (mut service, _) = new_service(store.clone(), TestCleaner::default());
    let caller = TestCaller::new("write");
    recover(&mut service, &caller);
    let before = store.current().unwrap();
    store.fail_next_commit();
    assert_eq!(
        service.create_local_user(&caller, metadata("User").0, metadata("Home").1),
        Err(IdentityError::StoreUnavailable)
    );
    assert_eq!(store.current().unwrap(), before);
    assert!(service.snapshot().users.is_empty());
}

#[test]
fn corrupt_and_unknown_state_is_rejected_or_recovers_only_from_last_known_good() {
    let broken = IdentitySnapshot::from_json(b"{\"schema_version\":1").unwrap_err();
    assert_eq!(broken, nagi_identity_session::SnapshotError::Malformed);

    let mut unsupported = serde_json::to_value(IdentitySnapshot::empty()).unwrap();
    unsupported["schema_version"] = serde_json::json!(99);
    let unsupported = serde_json::to_vec(&unsupported).unwrap();
    assert_eq!(
        IdentitySnapshot::from_json(&unsupported).unwrap_err(),
        nagi_identity_session::SnapshotError::UnsupportedVersion(99)
    );

    let store = SharedStore::default();
    let (mut service, _) = new_service(store.clone(), TestCleaner::default());
    let caller = TestCaller::new("recover");
    recover(&mut service, &caller);
    service
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    store.corrupt_current(b"truncated".to_vec());

    let (mut recovered, _) = new_service(store, TestCleaner::default());
    let report = recovered.recover(&caller).unwrap();
    assert_eq!(report.source, RecoverySource::LastKnownGood);
    assert!(recovered.snapshot().users.is_empty());
    assert_eq!(
        recovered.resolve_current_identity(&caller).unwrap_err(),
        ResolutionError::NoSession
    );
}

#[test]
fn corrupt_state_without_a_valid_backup_does_not_guess_an_identity() {
    let store = SharedStore::default();
    store.corrupt_current(b"not-json".to_vec());
    let (mut service, _) = new_service(store, TestCleaner::default());
    let caller = TestCaller::new("corrupt");
    assert_eq!(service.recover(&caller), Err(IdentityError::CorruptState));
    assert_eq!(
        service.resolve_current_identity(&caller).unwrap_err(),
        ResolutionError::Unavailable
    );
}

#[test]
fn restart_ends_sessions_and_does_not_reuse_old_session_ids() {
    let store = SharedStore::default();
    let cleaner = TestCleaner::default();
    let cleaned = cleaner.cleaned.clone();
    let (mut service, _) = new_service(store.clone(), cleaner);
    let caller = TestCaller::new("restart");
    recover(&mut service, &caller);
    let (user_id, profile_id) = service
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    let old_session = service
        .login_local(
            &caller,
            user_id,
            profile_id.clone(),
            None,
            nagi_identity_session::IdempotencyKey::new("before-restart").unwrap(),
        )
        .unwrap();

    let (mut after_restart, _) = new_service(store, TestCleaner::default());
    let report = after_restart.recover(&caller).unwrap();
    assert_eq!(report.source, RecoverySource::Current);
    assert_eq!(report.interrupted_sessions_ended, 1);
    assert_eq!(
        after_restart.resolve_current_identity(&caller).unwrap_err(),
        ResolutionError::NoSession
    );
    assert_eq!(
        after_restart.login_local(
            &caller,
            user_id,
            profile_id,
            None,
            nagi_identity_session::IdempotencyKey::new("after-restart").unwrap(),
        ),
        Err(IdentityError::IdCollision)
    );
    assert_eq!(old_session.as_str(), "session_s1");
    assert!(cleaned.lock().unwrap().is_empty());
}

#[test]
fn entropy_failure_and_cross_user_profile_use_fail_closed() {
    let store = SharedStore::default();
    let (mut service, _) = new_service(store, TestCleaner::default());
    let caller_a = TestCaller::new("a");
    let caller_b = TestCaller::new("b");
    service.recover(&caller_a).unwrap();
    let (user_a, profile_a) = service
        .create_local_user(&caller_a, metadata("A").0, metadata("A").1)
        .unwrap();
    let (user_b, _profile_b) = service
        .create_local_user(&caller_b, metadata("B").0, metadata("B").1)
        .unwrap();
    assert_ne!(user_a, user_b);
    assert_eq!(
        service.login_local(
            &caller_b,
            user_b,
            profile_a,
            None,
            nagi_identity_session::IdempotencyKey::new("cross-user").unwrap(),
        ),
        Err(IdentityError::ProfileOwnershipMismatch)
    );

    let (store, _ids, _clock, mut policy, cleaner, events) = service.into_parts();
    let mut service = IdentityService::new(
        store,
        FailingIds,
        TestClock(Arc::new(AtomicU64::new(100))),
        std::mem::take(&mut policy),
        cleaner,
        events,
    );
    service.recover(&caller_a).unwrap();
    assert_eq!(
        service.create_profile(
            &caller_a,
            user_a,
            ProfileMetadata::new("Entropy", None).unwrap(),
        ),
        Err(IdentityError::IdSourceUnavailable)
    );
}

struct FailingIds;

impl IdentityIdSource for FailingIds {
    fn next_user_id(&mut self) -> Result<UserId, IdSourceFailure> {
        Err(IdSourceFailure::Unavailable)
    }
    fn next_profile_id(&mut self) -> Result<ProfileId, IdSourceFailure> {
        Err(IdSourceFailure::Unavailable)
    }
    fn next_session_id(&mut self) -> Result<SessionId, IdSourceFailure> {
        Err(IdSourceFailure::Unavailable)
    }
}

#[test]
fn test_fake_store_keeps_schema_bounded_and_rejects_unknown_fields() {
    let mut store = InMemorySnapshotStore::default();
    let snapshot = IdentitySnapshot::empty();
    store
        .commit_atomic(None, &snapshot.to_json().unwrap())
        .unwrap();
    let mut unknown = serde_json::to_value(snapshot).unwrap();
    unknown["credential"] = serde_json::json!("not-allowed");
    assert_eq!(
        IdentitySnapshot::from_json(&serde_json::to_vec(&unknown).unwrap()).unwrap_err(),
        nagi_identity_session::SnapshotError::Malformed
    );
}

#[test]
fn denied_profile_root_access_never_calls_the_storage_backend() {
    let (mut service, _) = new_service(SharedStore::default(), TestCleaner::default());
    let caller = TestCaller::new("denied-root");
    recover(&mut service, &caller);
    let (user_id, profile_id) = service
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    service
        .login_local(
            &caller,
            user_id,
            profile_id.clone(),
            None,
            nagi_identity_session::IdempotencyKey::new("root-login").unwrap(),
        )
        .unwrap();
    let identity = service.resolve_current_identity(&caller).unwrap();
    let backend = MemoryProfileStorage::default();
    let opens = backend.opens.clone();
    let mut roots = UserStorageRootAdapter::new(backend);
    assert_eq!(
        roots
            .resolve_root(
                &IdentityAccessContext::new(&identity, &caller),
                &profile_id,
                &mut DenyProfileRoot,
            )
            .unwrap_err(),
        StorageRootFailure::Denied
    );
    assert_eq!(opens.load(Ordering::SeqCst), 0);
}

#[test]
fn provider_subject_resolution_preserves_the_stable_local_user_id() {
    struct OfflineProvider(UserId);
    impl IdentityProvider for OfflineProvider {
        fn resolve_subject(
            &self,
            subject: &ProviderSubjectRef,
        ) -> Result<Option<UserId>, ProviderLookupFailure> {
            assert_eq!(subject.provider(), "local-fixture");
            assert_eq!(subject.subject(), "fixture-subject");
            Ok(Some(self.0))
        }
    }

    let stable = UserId::new(42).unwrap();
    let external = ProviderSubjectRef::new("local-fixture", "fixture-subject").unwrap();
    assert!(!format!("{external:?}").contains("fixture-subject"));
    assert_eq!(
        OfflineProvider(stable).resolve_subject(&external),
        Ok(Some(stable))
    );
    assert_eq!(stable.to_model().0, 42);
}

#[test]
fn login_authorization_denial_publishes_no_session() {
    let store = SharedStore::default();
    let (mut setup, _) = new_service(store.clone(), TestCleaner::default());
    let caller = TestCaller::new("denied-login");
    recover(&mut setup, &caller);
    let (user_id, profile_id) = setup
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    let (mut service, _) = new_service_with_authorizer(
        store,
        TestCleaner::default(),
        TestAuthorizer {
            denied: Some(IdentityAction::LoginLocalUser),
            unavailable: None,
        },
    );
    service.recover(&caller).unwrap();
    assert_eq!(
        service.login_local(
            &caller,
            user_id,
            profile_id,
            None,
            nagi_identity_session::IdempotencyKey::new("denied").unwrap(),
        ),
        Err(IdentityError::AuthorizationDenied)
    );
    assert!(service.snapshot().sessions.is_empty());
}

#[test]
fn expired_session_is_ended_before_a_new_session_becomes_current() {
    let (mut service, clock) = new_service(SharedStore::default(), TestCleaner::default());
    let caller = TestCaller::new("replace-expired");
    recover(&mut service, &caller);
    let (user_id, profile_id) = service
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    let old = service
        .login_local(
            &caller,
            user_id,
            profile_id.clone(),
            Some(UnixMillis::new(150)),
            nagi_identity_session::IdempotencyKey::new("old-session").unwrap(),
        )
        .unwrap();
    let stale_identity = service.resolve_current_identity(&caller).unwrap();
    clock.set(150);
    let new = service
        .login_local(
            &caller,
            user_id,
            profile_id,
            None,
            nagi_identity_session::IdempotencyKey::new("new-session").unwrap(),
        )
        .unwrap();
    assert_ne!(old, new);
    assert!(!stale_identity.is_live());
    assert_eq!(
        service
            .resolve_current_identity(&caller)
            .unwrap()
            .session_id(),
        &new
    );
}

#[test]
fn migration_requires_an_explicit_converter_and_validates_its_output() {
    struct TestMigrator;
    impl IdentitySnapshotMigrator for TestMigrator {
        fn migrate_to_current(
            &self,
            source_version: u32,
            source: &[u8],
        ) -> Result<Vec<u8>, MigrationFailure> {
            if source_version != 0 {
                return Err(MigrationFailure::UnsupportedVersion);
            }
            let mut value: serde_json::Value =
                serde_json::from_slice(source).map_err(|_| MigrationFailure::InvalidLegacyState)?;
            value["schema_version"] = serde_json::json!(1);
            serde_json::to_vec(&value).map_err(|_| MigrationFailure::InvalidLegacyState)
        }
    }

    let mut legacy = serde_json::to_value(IdentitySnapshot::empty()).unwrap();
    legacy["schema_version"] = serde_json::json!(0);
    let legacy = serde_json::to_vec(&legacy).unwrap();
    assert_eq!(
        decode_snapshot_with_migration(&legacy, &TestMigrator)
            .unwrap()
            .schema_version,
        1
    );
    assert_eq!(
        decode_snapshot_with_migration(&legacy, &NoIdentitySnapshotMigrator),
        Err(nagi_identity_session::SnapshotError::MigrationFailed)
    );
}

// Mapping adapters may execute callbacks; success must not publish a stale identity.
struct MappingCallback<F: Fn()> {
    callback: F,
    calls: Arc<AtomicUsize>,
}

impl<F: Fn()> CapabilityPrincipalAdapter for MappingCallback<F> {
    fn principal_for(
        &self,
        _identity: &ResolvedIdentity,
    ) -> Result<PrincipalId, PrincipalMappingFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        (self.callback)();
        PrincipalId::new("system.mapped").map_err(|_| PrincipalMappingFailure::Denied)
    }
}

#[test]
fn mapping_callback_expiry_must_reject_success() {
    let (mut service, clock) = new_service(SharedStore::default(), TestCleaner::default());
    let caller = TestCaller::new("mapping-expiry");
    recover(&mut service, &caller);
    service
        .create_guest_session(
            &caller,
            Some(UnixMillis::new(200)),
            nagi_identity_session::IdempotencyKey::new("map-expiry").unwrap(),
        )
        .unwrap();
    let identity = service.resolve_current_identity(&caller).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let adapter = MappingCallback {
        callback: || clock.set(200),
        calls: calls.clone(),
    };
    let result = resolve_capability_principal(&adapter, &identity);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!identity.is_live());
    assert_eq!(result, Err(PrincipalMappingFailure::StaleSession));
}

#[test]
fn mapping_callback_logout_must_reject_success() {
    let (mut service, _) = new_service(SharedStore::default(), TestCleaner::default());
    let caller = TestCaller::new("mapping-logout");
    recover(&mut service, &caller);
    let session = service
        .create_guest_session(
            &caller,
            None,
            nagi_identity_session::IdempotencyKey::new("map-logout").unwrap(),
        )
        .unwrap();
    let identity = service.resolve_current_identity(&caller).unwrap();
    let service = std::cell::RefCell::new(service);
    let calls = Arc::new(AtomicUsize::new(0));
    let adapter = MappingCallback {
        callback: || {
            service.borrow_mut().end_session(&caller, &session).unwrap();
        },
        calls: calls.clone(),
    };
    let result = resolve_capability_principal(&adapter, &identity);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(!identity.is_live());
    assert_eq!(result, Err(PrincipalMappingFailure::StaleSession));
}

#[derive(Default)]
struct CountingBackend {
    inner: MemoryProfileStorage,
    reads: usize,
    writes: usize,
}
impl ProfileStorageBackend for CountingBackend {
    type Root = MemoryRoot;
    fn open_profile_root(
        &mut self,
        user: Option<UserId>,
        profile: &ProfileId,
    ) -> Result<Self::Root, StorageRootFailure> {
        self.inner.open_profile_root(user, profile)
    }
    fn read_object(
        &mut self,
        root: &Self::Root,
        object: ObjectId,
    ) -> Result<Vec<u8>, StorageRootFailure> {
        self.reads += 1;
        self.inner.read_object(root, object)
    }
    fn write_object(
        &mut self,
        root: &Self::Root,
        object: ObjectId,
        data: &[u8],
    ) -> Result<(), StorageRootFailure> {
        self.writes += 1;
        self.inner.write_object(root, object, data)
    }
}
struct CountingPrincipal(Arc<AtomicUsize>);
impl CapabilityPrincipalAdapter for CountingPrincipal {
    fn principal_for(&self, _: &ResolvedIdentity) -> Result<PrincipalId, PrincipalMappingFailure> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(PrincipalId::new("system.probe").unwrap())
    }
}
#[test]
fn recovery_by_b_must_revoke_a_before_backend_access() {
    let store = SharedStore::default();
    let (mut a, _) = new_service(store.clone(), TestCleaner::default());
    let caller = TestCaller::new("cross-instance");
    recover(&mut a, &caller);
    let (user, profile) = a
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    a.login_local(
        &caller,
        user,
        profile.clone(),
        None,
        nagi_identity_session::IdempotencyKey::new("cross-instance").unwrap(),
    )
    .unwrap();
    let identity = a.resolve_current_identity(&caller).unwrap();
    let access = IdentityAccessContext::new(&identity, &caller);
    let mut roots = UserStorageRootAdapter::new(CountingBackend::default());
    let mut auth = AllowProfileRoot;
    let handle = roots.resolve_root(&access, &profile, &mut auth).unwrap();
    roots
        .write_object(
            &access,
            &handle,
            &profile,
            ObjectId(1),
            b"before",
            &mut auth,
        )
        .unwrap();
    let (mut b, _) = new_service(store.clone(), TestCleaner::default());
    assert_eq!(b.recover(&caller).unwrap().interrupted_sessions_ended, 1);
    let saved = IdentitySnapshot::from_json(&store.current().unwrap()).unwrap();
    assert!(matches!(
        saved.sessions[0].lifecycle,
        nagi_identity_session::SessionLifecycle::Ended { .. }
    ));
    let read = roots.read_object(&access, &handle, &profile, ObjectId(1), &mut auth);
    let write = roots.write_object(&access, &handle, &profile, ObjectId(1), b"after", &mut auth);
    let calls = Arc::new(AtomicUsize::new(0));
    let mapping = resolve_capability_principal(&CountingPrincipal(calls.clone()), &identity);
    let current_mapping =
        a.capability_principal_for_current(&caller, &CountingPrincipal(calls.clone()));
    let backend = roots.into_backend();
    eprintln!("B persisted Ended; A live={}; read={read:?}; write={write:?}; mapping={mapping:?}; current_mapping={current_mapping:?}; reads={}; writes={} (1 setup write); mapping_calls={}", identity.is_live(), backend.reads, backend.writes, calls.load(Ordering::SeqCst));
    assert_eq!(
        (
            read,
            write,
            mapping,
            current_mapping,
            backend.reads,
            backend.writes,
            calls.load(Ordering::SeqCst)
        ),
        (
            Err(StorageRootFailure::StaleSession),
            Err(StorageRootFailure::StaleSession),
            Err(PrincipalMappingFailure::StaleSession),
            Err(PrincipalMappingFailure::StaleSession),
            0,
            1,
            0
        )
    );
}

#[test]
fn store_read_failure_must_deny_existing_identity_before_backend() {
    let store = SharedStore::default();
    let (mut service, _) = new_service(store.clone(), TestCleaner::default());
    let caller = TestCaller::new("unreadable-store");
    recover(&mut service, &caller);
    let (user, profile) = service
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    service
        .login_local(
            &caller,
            user,
            profile.clone(),
            None,
            nagi_identity_session::IdempotencyKey::new("unreadable-store").unwrap(),
        )
        .unwrap();
    let identity = service.resolve_current_identity(&caller).unwrap();
    let access = IdentityAccessContext::new(&identity, &caller);
    let mut roots = UserStorageRootAdapter::new(CountingBackend::default());
    let mut auth = AllowProfileRoot;
    let handle = roots.resolve_root(&access, &profile, &mut auth).unwrap();
    store.0.lock().unwrap().fail_reads = true;
    assert_eq!(
        service.resolve_current_identity(&caller).unwrap_err(),
        ResolutionError::Unavailable
    );
    let read = roots.read_object(&access, &handle, &profile, ObjectId(1), &mut auth);
    let write = roots.write_object(&access, &handle, &profile, ObjectId(1), b"after", &mut auth);
    let calls = Arc::new(AtomicUsize::new(0));
    let mapping = resolve_capability_principal(&CountingPrincipal(calls.clone()), &identity);
    let backend = roots.into_backend();
    assert_eq!(
        (
            read,
            write,
            mapping,
            backend.reads,
            backend.writes,
            calls.load(Ordering::SeqCst)
        ),
        (
            Err(StorageRootFailure::StaleSession),
            Err(StorageRootFailure::StaleSession),
            Err(PrincipalMappingFailure::StaleSession),
            0,
            0,
            0
        )
    );
}

struct CallbackRootAuthorizer<F: FnMut()>(F);
impl<F: FnMut()> ProfileRootAuthorizer for CallbackRootAuthorizer<F> {
    fn authorize_profile_root(
        &mut self,
        _: &dyn TrustedCallerContext,
        _: &ResolvedIdentity,
        _: &ProfileId,
    ) -> Result<(), StorageRootFailure> {
        (self.0)();
        Ok(())
    }
}

#[test]
fn recovery_in_storage_authorizer_denies_open_read_write_before_backend() {
    for operation in 0..3 {
        let store = SharedStore::default();
        let (mut a, _) = new_service(store.clone(), TestCleaner::default());
        let caller = TestCaller::new("recovery-callback");
        recover(&mut a, &caller);
        let (user, profile) = a
            .create_local_user(&caller, metadata("User").0, metadata("Home").1)
            .unwrap();
        a.login_local(
            &caller,
            user,
            profile.clone(),
            None,
            nagi_identity_session::IdempotencyKey::new("callback-login").unwrap(),
        )
        .unwrap();
        let identity = a.resolve_current_identity(&caller).unwrap();
        let access = IdentityAccessContext::new(&identity, &caller);
        let backend = CountingBackend::default();
        let opens = backend.inner.opens.clone();
        let mut roots = UserStorageRootAdapter::new(backend);
        let handle = roots
            .resolve_root(&access, &profile, &mut AllowProfileRoot)
            .unwrap();
        let (mut b, _) = new_service(store.clone(), TestCleaner::default());
        let mut calls = 0;
        let mut authorizer = CallbackRootAuthorizer(|| {
            calls += 1;
            assert_eq!(b.recover(&caller).unwrap().interrupted_sessions_ended, 1);
        });
        let result = match operation {
            0 => roots
                .resolve_root(&access, &profile, &mut authorizer)
                .map(|_| ()),
            1 => roots
                .read_object(&access, &handle, &profile, ObjectId(1), &mut authorizer)
                .map(|_| ()),
            _ => roots.write_object(
                &access,
                &handle,
                &profile,
                ObjectId(1),
                b"after",
                &mut authorizer,
            ),
        };
        assert_eq!(result, Err(StorageRootFailure::StaleSession));
        assert_eq!(calls, 1);
        let backend = roots.into_backend();
        assert_eq!(
            (opens.load(Ordering::SeqCst), backend.reads, backend.writes),
            (1, 0, 0)
        );
    }
}

#[test]
fn recovery_in_principal_callback_denies_direct_and_service_mapping() {
    for service_method in [false, true] {
        let store = SharedStore::default();
        let (mut a, _) = new_service(store.clone(), TestCleaner::default());
        let caller = TestCaller::new("mapping-recovery");
        recover(&mut a, &caller);
        a.create_guest_session(
            &caller,
            None,
            nagi_identity_session::IdempotencyKey::new("mapping-recovery").unwrap(),
        )
        .unwrap();
        let identity = a.resolve_current_identity(&caller).unwrap();
        let (b, _) = new_service(store, TestCleaner::default());
        let b = std::cell::RefCell::new(b);
        let calls = Arc::new(AtomicUsize::new(0));
        let adapter = MappingCallback {
            callback: || {
                assert_eq!(
                    b.borrow_mut()
                        .recover(&caller)
                        .unwrap()
                        .interrupted_sessions_ended,
                    1
                );
            },
            calls: calls.clone(),
        };
        let result = if service_method {
            a.capability_principal_for_current(&caller, &adapter)
        } else {
            resolve_capability_principal(&adapter, &identity)
        };
        assert_eq!(result, Err(PrincipalMappingFailure::StaleSession));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(!identity.is_live());
    }
}

#[test]
fn current_store_corruption_cannot_use_last_known_good_for_authority() {
    let store = SharedStore::default();
    let (mut service, _) = new_service(store.clone(), TestCleaner::default());
    let caller = TestCaller::new("live-corruption");
    recover(&mut service, &caller);
    service
        .create_guest_session(
            &caller,
            None,
            nagi_identity_session::IdempotencyKey::new("live-corruption").unwrap(),
        )
        .unwrap();
    let identity = service.resolve_current_identity(&caller).unwrap();
    store.corrupt_current(b"not-a-snapshot".to_vec());
    assert!(!identity.is_live());
    let calls = Arc::new(AtomicUsize::new(0));
    assert_eq!(
        resolve_capability_principal(&CountingPrincipal(calls.clone()), &identity),
        Err(PrincipalMappingFailure::StaleSession)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn in_memory_current_reader_tracks_commit_corruption_and_failure() {
    let mut store = InMemorySnapshotStore::default();
    let reader = store.current_snapshot_reader().unwrap();
    let bytes = IdentitySnapshot::empty().to_json().unwrap();
    let version = store.commit_atomic(None, &bytes).unwrap();
    assert_eq!(reader().unwrap().unwrap().bytes, bytes);
    let independent = store.clone();
    store.corrupt_current_for_test(b"corrupt".to_vec());
    assert_eq!(reader().unwrap().unwrap().bytes, b"corrupt");
    assert_eq!(
        independent.load_current().unwrap().unwrap().version,
        version
    );
    assert_eq!(
        independent.current_snapshot_reader().unwrap()()
            .unwrap()
            .unwrap()
            .bytes,
        bytes
    );
    store.fail_reads();
    assert_eq!(reader(), Err(SnapshotStoreError::Unavailable));
}

#[test]
fn parallel_recovery_conflict_revokes_old_handles_and_new_session_still_works() {
    let store = SharedStore::default();
    let (mut a, clock) = new_service(store.clone(), TestCleaner::default());
    let caller = TestCaller::new("parallel-recover");
    recover(&mut a, &caller);
    let (user, profile) = a
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    a.login_local(
        &caller,
        user,
        profile.clone(),
        None,
        nagi_identity_session::IdempotencyKey::new("parallel-recover").unwrap(),
    )
    .unwrap();
    let old_identity = a.resolve_current_identity(&caller).unwrap();
    let access = IdentityAccessContext::new(&old_identity, &caller);
    let mut roots = UserStorageRootAdapter::new(CountingBackend::default());
    let old_handle = roots
        .resolve_root(&access, &profile, &mut AllowProfileRoot)
        .unwrap();
    let (b, _) = new_service(store.clone(), TestCleaner::default());
    let (c, _) = new_service(store.clone(), TestCleaner::default());
    store.0.lock().unwrap().read_barrier = Some(Arc::new(std::sync::Barrier::new(2)));
    let mut results = Vec::new();
    thread::scope(|scope| {
        let caller = &caller;
        let rb = scope.spawn(move || {
            let mut service = b;
            service.recover(caller)
        });
        let rc = scope.spawn(move || {
            let mut service = c;
            service.recover(caller)
        });
        results.push(rb.join().unwrap());
        results.push(rc.join().unwrap());
    });
    store.0.lock().unwrap().read_barrier = None;
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Ok(report) if report.interrupted_sessions_ended == 1))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|r| matches!(r, Err(IdentityError::StoreConflict)))
            .count(),
        1
    );
    let read = roots.read_object(
        &access,
        &old_handle,
        &profile,
        ObjectId(1),
        &mut AllowProfileRoot,
    );
    let write = roots.write_object(
        &access,
        &old_handle,
        &profile,
        ObjectId(1),
        b"old",
        &mut AllowProfileRoot,
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let mapping = a.capability_principal_for_current(&caller, &CountingPrincipal(calls.clone()));
    assert_eq!(
        (read, write, mapping, calls.load(Ordering::SeqCst)),
        (
            Err(StorageRootFailure::StaleSession),
            Err(StorageRootFailure::StaleSession),
            Err(PrincipalMappingFailure::StaleSession),
            0
        )
    );
    let mut fresh = IdentityService::new(
        store,
        TestIds {
            session: 100,
            ..TestIds::default()
        },
        clock,
        TestAuthorizer::default(),
        TestCleaner::default(),
        NoopIdentityEventSink,
    );
    fresh.recover(&caller).unwrap();
    fresh
        .login_local(
            &caller,
            user,
            profile.clone(),
            None,
            nagi_identity_session::IdempotencyKey::new("new-session").unwrap(),
        )
        .unwrap();
    let current = fresh.resolve_current_identity(&caller).unwrap();
    let current_access = IdentityAccessContext::new(&current, &caller);
    let new_handle = roots
        .resolve_root(&current_access, &profile, &mut AllowProfileRoot)
        .unwrap();
    roots
        .write_object(
            &current_access,
            &new_handle,
            &profile,
            ObjectId(1),
            b"new",
            &mut AllowProfileRoot,
        )
        .unwrap();
    assert_eq!(
        roots.read_object(
            &current_access,
            &new_handle,
            &profile,
            ObjectId(1),
            &mut AllowProfileRoot
        ),
        Ok(b"new".to_vec())
    );
    assert!(fresh
        .capability_principal_for_current(&caller, &CountingPrincipal(calls.clone()))
        .is_ok());
    assert!(!old_identity.is_live());
    let backend = roots.into_backend();
    assert_eq!((backend.reads, backend.writes), (1, 1));
}

#[test]
fn failed_recovery_commit_does_not_publish_end_and_successful_retry_revokes() {
    let store = SharedStore::default();
    let (mut a, _) = new_service(store.clone(), TestCleaner::default());
    let caller = TestCaller::new("recover-commit-fail");
    recover(&mut a, &caller);
    a.create_guest_session(
        &caller,
        None,
        nagi_identity_session::IdempotencyKey::new("commit-fail").unwrap(),
    )
    .unwrap();
    let identity = a.resolve_current_identity(&caller).unwrap();
    let (mut b, _) = new_service(store.clone(), TestCleaner::default());
    store.fail_next_commit();
    assert_eq!(b.recover(&caller), Err(IdentityError::StoreUnavailable));
    assert!(identity.is_live());
    assert!(matches!(
        IdentitySnapshot::from_json(&store.current().unwrap())
            .unwrap()
            .sessions[0]
            .lifecycle,
        nagi_identity_session::SessionLifecycle::Active
    ));
    b.recover(&caller).unwrap();
    assert!(!identity.is_live());
}

#[test]
fn missing_reader_defaults_to_unavailable_without_cached_authority() {
    struct WithoutReader(SharedStore);
    impl IdentitySnapshotStore for WithoutReader {
        fn load_current(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
            self.0.load_current()
        }
        fn load_last_known_good(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
            self.0.load_last_known_good()
        }
        fn commit_atomic(
            &mut self,
            version: Option<SnapshotVersion>,
            bytes: &[u8],
        ) -> Result<SnapshotVersion, SnapshotStoreError> {
            self.0.commit_atomic(version, bytes)
        }
    }
    let mut service = IdentityService::new(
        WithoutReader(SharedStore::default()),
        TestIds::default(),
        TestClock(Arc::new(AtomicU64::new(100))),
        TestAuthorizer::default(),
        TestCleaner::default(),
        NoopIdentityEventSink,
    );
    let caller = TestCaller::new("no-reader");
    service.recover(&caller).unwrap();
    service
        .create_guest_session(
            &caller,
            None,
            nagi_identity_session::IdempotencyKey::new("no-reader").unwrap(),
        )
        .unwrap();
    assert_eq!(
        service.resolve_current_identity(&caller).unwrap_err(),
        ResolutionError::Unavailable
    );
    assert_eq!(
        service.capability_principal_for_current(
            &caller,
            &CountingPrincipal(Arc::new(AtomicUsize::new(0)))
        ),
        Err(PrincipalMappingFailure::Unavailable)
    );
}

#[test]
fn review_failed_rerecovery_must_not_rebase_stale_snapshot_or_resurrect_identity() {
    let store = SharedStore::default();
    let caller = TestCaller::new("independent-failure-atomicity");
    let (mut a, _) = new_service(store.clone(), TestCleaner::default());
    recover(&mut a, &caller);
    a.create_guest_session(
        &caller,
        None,
        nagi_identity_session::IdempotencyKey::new("old-session").unwrap(),
    )
    .unwrap();
    let old = a.resolve_current_identity(&caller).unwrap();
    let access = IdentityAccessContext::new(&old, &caller);
    let mut roots = UserStorageRootAdapter::new(CountingBackend::default());
    let mut auth = AllowProfileRoot;
    let handle = roots
        .resolve_root(&access, old.profile_id(), &mut auth)
        .unwrap();
    let mut b = IdentityService::new(
        store.clone(),
        TestIds {
            user: 100,
            profile: 100,
            session: 100,
            fail: false,
        },
        TestClock(Arc::new(AtomicU64::new(100))),
        TestAuthorizer::default(),
        TestCleaner::default(),
        NoopIdentityEventSink,
    );
    b.recover(&caller).unwrap();
    let (new_user, _) = b
        .create_local_user(
            &caller,
            metadata("B committed user").0,
            metadata("B committed home").1,
        )
        .unwrap();
    let new_session = b
        .create_guest_session(
            &caller,
            None,
            nagi_identity_session::IdempotencyKey::new("new-session").unwrap(),
        )
        .unwrap();
    assert!(!old.is_live());
    let before = store.current().unwrap();
    store.fail_next_commit();
    assert_eq!(a.recover(&caller), Err(IdentityError::StoreUnavailable));
    assert_eq!(store.current().unwrap(), before);
    let followup = a.create_local_user(&caller, metadata("A followup").0, metadata("A home").1);
    let after = IdentitySnapshot::from_json(&store.current().unwrap()).unwrap();
    let b_user_retained = after.users.iter().any(|u| u.user_id == new_user);
    let b_session_retained = after.sessions.iter().any(|s| s.session_id == new_session);
    eprintln!("followup={followup:?}; B user retained={b_user_retained}; B session retained={b_session_retained}; OLD IDENTITY LIVE AGAIN={}", old.is_live());
    let read = roots.read_object(&access, &handle, old.profile_id(), ObjectId(1), &mut auth);
    let write = roots.write_object(
        &access,
        &handle,
        old.profile_id(),
        ObjectId(1),
        b"stale",
        &mut auth,
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let mapping = resolve_capability_principal(&CountingPrincipal(calls.clone()), &old);
    let current_mapping =
        a.capability_principal_for_current(&caller, &CountingPrincipal(calls.clone()));
    let backend = roots.into_backend();
    eprintln!(
        "old handle read={read:?}; backend reads={}; old principal mapping={mapping:?}",
        backend.reads
    );
    assert_eq!(followup, Err(IdentityError::StoreConflict));
    assert_eq!(
        (backend.reads, backend.writes, calls.load(Ordering::SeqCst)),
        (0, 0, 0),
        "committed-ended identity must not reach backend or mapping adapter"
    );
    assert_eq!(write, Err(StorageRootFailure::StaleSession));
    assert_eq!(current_mapping, Err(PrincipalMappingFailure::StaleSession));
    assert_eq!(read, Err(StorageRootFailure::StaleSession));
    assert_eq!(mapping, Err(PrincipalMappingFailure::StaleSession));
    assert!(
        !old.is_live(),
        "failed recovery followed by ordinary write resurrected a committed-ended identity"
    );
    assert!(
        b_user_retained,
        "failed recovery must not cause loss of committed user"
    );
    assert!(
        b_session_retained,
        "failed recovery must not lose B session"
    );
}
