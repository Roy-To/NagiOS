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
    fn load_current(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
        let state = self.0.lock().unwrap();
        if state.fail_reads {
            return Err(SnapshotStoreError::Unavailable);
        }
        Ok(state.current.clone())
    }

    fn load_last_known_good(&self) -> Result<Option<StoredSnapshot>, SnapshotStoreError> {
        let state = self.0.lock().unwrap();
        if state.fail_reads {
            return Err(SnapshotStoreError::Unavailable);
        }
        Ok(state.last_good.clone())
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
    reads: Arc<AtomicUsize>,
    writes: Arc<AtomicUsize>,
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
        self.reads.fetch_add(1, Ordering::SeqCst);
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
        self.writes.fetch_add(1, Ordering::SeqCst);
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

struct SessionFixture {
    service: TestService,
    clock: TestClock,
    caller: TestCaller,
    user: UserId,
    profile: ProfileId,
    session: SessionId,
    identity: ResolvedIdentity,
}

impl SessionFixture {
    fn new(store: SharedStore, context: &str) -> Self {
        let (mut service, clock) = new_service(store, TestCleaner::default());
        let caller = TestCaller::new(context);
        recover(&mut service, &caller);
        let (user, profile) = service
            .create_local_user(&caller, metadata("User").0, metadata("Home").1)
            .unwrap();
        let session = service
            .login_local(
                &caller,
                user,
                profile.clone(),
                Some(UnixMillis::new(200)),
                nagi_identity_session::IdempotencyKey::new("fixture-login").unwrap(),
            )
            .unwrap();
        let identity = service.resolve_current_identity(&caller).unwrap();
        Self {
            service,
            clock,
            caller,
            user,
            profile,
            session,
            identity,
        }
    }
}

#[test]
fn failed_logout_commit_revokes_evidence_without_changing_saved_bytes() {
    let store = SharedStore::default();
    let mut f = SessionFixture::new(store.clone(), "failed-logout");
    let mut roots = UserStorageRootAdapter::new(MemoryProfileStorage::default());
    let handle = roots
        .resolve_root(
            &IdentityAccessContext::new(&f.identity, &f.caller),
            &f.profile,
            &mut AllowProfileRoot,
        )
        .unwrap();
    let saved = store.current().unwrap();
    store.fail_next_commit();
    assert_eq!(
        f.service.end_session(&f.caller, &f.session),
        Err(IdentityError::StoreUnavailable)
    );
    assert_eq!(store.current().unwrap(), saved);
    assert!(!f.identity.is_live());
    assert_eq!(
        f.service.resolve_current_identity(&f.caller).unwrap_err(),
        ResolutionError::Unavailable
    );
    assert_eq!(
        roots.write_object(
            &IdentityAccessContext::new(&f.identity, &f.caller),
            &handle,
            &f.profile,
            ObjectId(1),
            b"must not write",
            &mut AllowProfileRoot,
        ),
        Err(StorageRootFailure::StaleSession)
    );
    // Recovery, rather than a failed write, may re-establish service readiness;
    // it never resumes the previously issued evidence.
    f.service.recover(&f.caller).unwrap();
    assert!(!f.identity.is_live());
    assert_eq!(
        f.service.resolve_current_identity(&f.caller).unwrap_err(),
        ResolutionError::Ended
    );
}

#[test]
fn failed_recovery_commit_revokes_evidence_before_returning_the_error() {
    let store = SharedStore::default();
    let mut f = SessionFixture::new(store.clone(), "failed-recovery-commit");
    let saved = store.current().unwrap();
    store.fail_next_commit();
    assert_eq!(
        f.service.recover(&f.caller),
        Err(IdentityError::StoreUnavailable)
    );
    assert_eq!(store.current().unwrap(), saved);
    assert!(!f.identity.is_live());
    assert_eq!(
        f.service.resolve_current_identity(&f.caller).unwrap_err(),
        ResolutionError::Unavailable
    );
}

#[test]
fn failed_recovery_read_or_corruption_cannot_keep_previous_evidence_live() {
    for corrupt in [false, true] {
        let store = SharedStore::default();
        let mut f = SessionFixture::new(store.clone(), "failed-recovery-read");
        {
            let mut state = store.0.lock().unwrap();
            if corrupt {
                state.current.as_mut().unwrap().bytes = b"invalid current".to_vec();
                state.last_good.as_mut().unwrap().bytes = b"invalid backup".to_vec();
            } else {
                state.fail_reads = true;
            }
        }
        let saved = store.current().unwrap();
        assert_eq!(
            f.service.recover(&f.caller),
            Err(if corrupt {
                IdentityError::CorruptState
            } else {
                IdentityError::StoreUnavailable
            })
        );
        assert_eq!(store.current().unwrap(), saved);
        assert!(!f.identity.is_live());
        assert_eq!(
            f.service.resolve_current_identity(&f.caller).unwrap_err(),
            ResolutionError::Unavailable
        );
    }
}

#[test]
fn persistence_conflict_invalidates_the_stale_services_evidence() {
    let store = SharedStore::default();
    let mut f = SessionFixture::new(store.clone(), "lease-conflict");
    let version = store.0.lock().unwrap().current.as_ref().unwrap().version;
    let mut concurrent = f.service.snapshot().clone();
    concurrent.revision += 1;
    store
        .clone()
        .commit_atomic(Some(version), &concurrent.to_json().unwrap())
        .unwrap();
    let saved = store.current().unwrap();
    assert_eq!(
        f.service
            .create_profile(&f.caller, f.user, metadata("Another").1),
        Err(IdentityError::StoreConflict)
    );
    assert_eq!(store.current().unwrap(), saved);
    assert!(!f.identity.is_live());
    assert_eq!(
        f.service.resolve_current_identity(&f.caller).unwrap_err(),
        ResolutionError::Unavailable
    );
}

#[test]
fn extracting_or_dropping_the_service_revokes_its_issued_evidence() {
    let f = SessionFixture::new(SharedStore::default(), "extract-service");
    let _parts = f.service.into_parts();
    assert!(!f.identity.is_live());
    let f = SessionFixture::new(SharedStore::default(), "drop-service");
    drop(f.service);
    assert!(!f.identity.is_live());
}

#[test]
fn capability_mapping_rechecks_evidence_after_the_adapter_returns() {
    struct ExpiringMapper(TestClock);
    impl CapabilityPrincipalAdapter for ExpiringMapper {
        fn principal_for(
            &self,
            _: &ResolvedIdentity,
        ) -> Result<PrincipalId, PrincipalMappingFailure> {
            self.0.set(200);
            Ok(PrincipalId::new("user:mapped").unwrap())
        }
    }
    let f = SessionFixture::new(SharedStore::default(), "mapping-expiry");
    assert_eq!(
        resolve_capability_principal(&ExpiringMapper(f.clock), &f.identity),
        Err(PrincipalMappingFailure::StaleSession)
    );

    struct RevokingMapper {
        service: Mutex<TestService>,
        caller: TestCaller,
        session: SessionId,
    }
    impl CapabilityPrincipalAdapter for RevokingMapper {
        fn principal_for(
            &self,
            _: &ResolvedIdentity,
        ) -> Result<PrincipalId, PrincipalMappingFailure> {
            self.service
                .lock()
                .unwrap()
                .end_session(&self.caller, &self.session)
                .unwrap();
            Ok(PrincipalId::new("user:mapped").unwrap())
        }
    }
    let f = SessionFixture::new(SharedStore::default(), "mapping-revocation");
    let mapper = RevokingMapper {
        service: Mutex::new(f.service),
        caller: f.caller,
        session: f.session,
    };
    assert_eq!(
        resolve_capability_principal(&mapper, &f.identity),
        Err(PrincipalMappingFailure::StaleSession)
    );
}

#[test]
fn a_storage_handle_cannot_be_used_with_another_adapter_instance() {
    let f = SessionFixture::new(SharedStore::default(), "root-issuer");
    let access = IdentityAccessContext::new(&f.identity, &f.caller);
    let mut first = UserStorageRootAdapter::new(MemoryProfileStorage::default());
    let mut second = UserStorageRootAdapter::new(MemoryProfileStorage::default());
    let first_handle = first
        .resolve_root(&access, &f.profile, &mut AllowProfileRoot)
        .unwrap();
    let second_handle = second
        .resolve_root(&access, &f.profile, &mut AllowProfileRoot)
        .unwrap();
    second
        .write_object(
            &access,
            &second_handle,
            &f.profile,
            ObjectId(1),
            b"second adapter",
            &mut AllowProfileRoot,
        )
        .unwrap();
    assert_eq!(
        second.read_object(
            &access,
            &first_handle,
            &f.profile,
            ObjectId(1),
            &mut AllowProfileRoot
        ),
        Err(StorageRootFailure::InvalidHandle)
    );
}

#[test]
fn same_looking_evidence_cannot_reanimate_a_previous_sessions_root_handle() {
    let mut first = SessionFixture::new(SharedStore::default(), "same-evidence-ids");
    let second = SessionFixture::new(SharedStore::default(), "same-evidence-ids");
    assert_eq!(first.session, second.session);
    assert_eq!(first.profile, second.profile);
    assert_eq!(
        first.identity.principal_id(),
        second.identity.principal_id()
    );
    let mut roots = UserStorageRootAdapter::new(MemoryProfileStorage::default());
    let access = IdentityAccessContext::new(&first.identity, &first.caller);
    let handle = roots
        .resolve_root(&access, &first.profile, &mut AllowProfileRoot)
        .unwrap();
    roots
        .write_object(
            &access,
            &handle,
            &first.profile,
            ObjectId(1),
            b"original root",
            &mut AllowProfileRoot,
        )
        .unwrap();
    let fresh = first
        .service
        .resolve_current_identity(&first.caller)
        .unwrap();
    assert_eq!(
        roots
            .read_object(
                &IdentityAccessContext::new(&fresh, &first.caller),
                &handle,
                &first.profile,
                ObjectId(1),
                &mut AllowProfileRoot,
            )
            .unwrap(),
        b"original root",
    );
    first
        .service
        .end_session(&first.caller, &first.session)
        .unwrap();
    assert!(!first.identity.is_live());
    assert!(second.identity.is_live());
    assert_eq!(
        roots.read_object(
            &IdentityAccessContext::new(&second.identity, &second.caller),
            &handle,
            &second.profile,
            ObjectId(1),
            &mut AllowProfileRoot,
        ),
        Err(StorageRootFailure::CrossProfile)
    );
}

#[test]
fn unauthorized_recovery_does_not_revoke_the_existing_session() {
    struct RecoveryAuthorizer(Arc<std::sync::atomic::AtomicBool>);
    impl IdentityAuthorizer for RecoveryAuthorizer {
        fn authorize(
            &mut self,
            request: &AuthorizationRequest,
        ) -> Result<(), AuthorizationFailure> {
            if request.action == IdentityAction::Recover && self.0.load(Ordering::SeqCst) {
                Err(AuthorizationFailure::Denied)
            } else {
                Ok(())
            }
        }
    }
    let deny = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut service = IdentityService::new(
        SharedStore::default(),
        TestIds::default(),
        TestClock(Arc::new(AtomicU64::new(100))),
        RecoveryAuthorizer(deny.clone()),
        TestCleaner::default(),
        NoopIdentityEventSink,
    );
    let caller = TestCaller::new("denied-recovery");
    service.recover(&caller).unwrap();
    let (user, profile) = service
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    service
        .login_local(
            &caller,
            user,
            profile,
            None,
            nagi_identity_session::IdempotencyKey::new("denied-recovery-login").unwrap(),
        )
        .unwrap();
    let identity = service.resolve_current_identity(&caller).unwrap();
    deny.store(true, Ordering::SeqCst);
    assert_eq!(
        service.recover(&caller),
        Err(IdentityError::AuthorizationDenied)
    );
    assert!(identity.is_live());
    assert!(service.resolve_current_identity(&caller).is_ok());
}

#[test]
fn logout_waits_for_the_in_flight_storage_lease_before_completing() {
    use std::sync::mpsc;
    use std::time::Duration;

    struct BlockingStorage {
        data: MemoryProfileStorage,
        entered: mpsc::Sender<()>,
        release: mpsc::Receiver<()>,
    }
    impl ProfileStorageBackend for BlockingStorage {
        type Root = MemoryRoot;
        fn open_profile_root(
            &mut self,
            user: Option<UserId>,
            profile: &ProfileId,
        ) -> Result<Self::Root, StorageRootFailure> {
            self.data.open_profile_root(user, profile)
        }
        fn read_object(
            &mut self,
            root: &Self::Root,
            object: ObjectId,
        ) -> Result<Vec<u8>, StorageRootFailure> {
            self.entered
                .send(())
                .map_err(|_| StorageRootFailure::Unavailable)?;
            self.release
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| StorageRootFailure::Unavailable)?;
            self.data.read_object(root, object)
        }
        fn write_object(
            &mut self,
            root: &Self::Root,
            object: ObjectId,
            bytes: &[u8],
        ) -> Result<(), StorageRootFailure> {
            self.data.write_object(root, object, bytes)
        }
    }
    let mut f = SessionFixture::new(SharedStore::default(), "in-flight-lease");
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let mut roots = UserStorageRootAdapter::new(BlockingStorage {
        data: MemoryProfileStorage::default(),
        entered: entered_tx,
        release: release_rx,
    });
    let handle = roots
        .resolve_root(
            &IdentityAccessContext::new(&f.identity, &f.caller),
            &f.profile,
            &mut AllowProfileRoot,
        )
        .unwrap();
    roots
        .write_object(
            &IdentityAccessContext::new(&f.identity, &f.caller),
            &handle,
            &f.profile,
            ObjectId(1),
            b"in flight",
            &mut AllowProfileRoot,
        )
        .unwrap();
    let reader_caller = f.caller.clone();
    let reader = thread::spawn(move || {
        let result = roots.read_object(
            &IdentityAccessContext::new(&f.identity, &reader_caller),
            &handle,
            &f.profile,
            ObjectId(1),
            &mut AllowProfileRoot,
        );
        (result, f.identity)
    });
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let writer = thread::spawn(move || {
        started_tx.send(()).unwrap();
        done_tx
            .send(f.service.end_session(&f.caller, &f.session))
            .unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let early = done_rx.recv_timeout(Duration::from_millis(100));
    let completed_while_reading = early.is_ok();
    release_tx.send(()).unwrap();
    let ended = early.unwrap_or_else(|_| done_rx.recv_timeout(Duration::from_secs(2)).unwrap());
    let (read, identity) = reader.join().unwrap();
    writer.join().unwrap();
    assert!(!completed_while_reading);
    assert_eq!(read.unwrap(), b"in flight");
    assert_eq!(ended.unwrap(), EndSessionResult::Ended);
    assert!(!identity.is_live());
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DisposalCallback {
    Open,
    Read,
    Write,
}

struct DisposingStorage {
    data: MemoryProfileStorage,
    service: Option<TestService>,
    callback: DisposalCallback,
    into_parts: bool,
}

impl DisposingStorage {
    fn dispose_on(&mut self, callback: DisposalCallback) {
        if callback != self.callback {
            return;
        }
        let service = self.service.take().expect("callback runs once");
        println!(
            "entered {callback:?} callback; into_parts={}",
            self.into_parts
        );
        if self.into_parts {
            let _parts = service.into_parts();
        } else {
            drop(service);
        }
        println!("service disposal returned from {callback:?} callback");
    }
}

impl ProfileStorageBackend for DisposingStorage {
    type Root = MemoryRoot;

    fn open_profile_root(
        &mut self,
        user: Option<UserId>,
        profile: &ProfileId,
    ) -> Result<Self::Root, StorageRootFailure> {
        self.dispose_on(DisposalCallback::Open);
        self.data.open_profile_root(user, profile)
    }

    fn read_object(
        &mut self,
        root: &Self::Root,
        object: ObjectId,
    ) -> Result<Vec<u8>, StorageRootFailure> {
        self.dispose_on(DisposalCallback::Read);
        self.data.read_object(root, object)
    }

    fn write_object(
        &mut self,
        root: &Self::Root,
        object: ObjectId,
        bytes: &[u8],
    ) -> Result<(), StorageRootFailure> {
        self.dispose_on(DisposalCallback::Write);
        self.data.write_object(root, object, bytes)
    }
}

// Only the supervising tests invoke this entry point, with an explicit case
// and deadline. A deadlock must not strand the test runner or a child process.
#[test]
#[ignore = "entry point for the supervised callback-disposal regressions"]
fn backend_callback_disposal_child() {
    let case = std::env::var("NAGI_IDENTITY_DISPOSAL_CASE").expect("supervised case");
    let callback = match case.as_str() {
        "open-drop" | "open-into-parts" => DisposalCallback::Open,
        "read-drop" | "read-into-parts" => DisposalCallback::Read,
        "write-drop" | "write-into-parts" => DisposalCallback::Write,
        _ => panic!("unknown callback-disposal case"),
    };
    let f = SessionFixture::new(SharedStore::default(), "callback-disposal");
    let mut roots = UserStorageRootAdapter::new(DisposingStorage {
        data: MemoryProfileStorage::default(),
        service: Some(f.service),
        callback,
        into_parts: case.ends_with("into-parts"),
    });
    let access = IdentityAccessContext::new(&f.identity, &f.caller);
    let handle = roots
        .resolve_root(&access, &f.profile, &mut AllowProfileRoot)
        .unwrap();
    if callback != DisposalCallback::Open {
        roots
            .write_object(
                &access,
                &handle,
                &f.profile,
                ObjectId(1),
                b"in-flight operation",
                &mut AllowProfileRoot,
            )
            .unwrap();
    }
    if callback == DisposalCallback::Read {
        assert_eq!(
            roots
                .read_object(
                    &access,
                    &handle,
                    &f.profile,
                    ObjectId(1),
                    &mut AllowProfileRoot,
                )
                .unwrap(),
            b"in-flight operation",
        );
    }
    // The operation already holding a lease can finish, but disposal must
    // invalidate its evidence and every subsequent use or root resolution.
    assert!(!f.identity.is_live());
    assert_eq!(
        roots.read_object(
            &access,
            &handle,
            &f.profile,
            ObjectId(1),
            &mut AllowProfileRoot,
        ),
        Err(StorageRootFailure::StaleSession),
    );
    assert_eq!(
        roots.write_object(
            &access,
            &handle,
            &f.profile,
            ObjectId(1),
            b"must not write",
            &mut AllowProfileRoot,
        ),
        Err(StorageRootFailure::StaleSession),
    );
    assert_eq!(
        roots
            .resolve_root(&access, &f.profile, &mut AllowProfileRoot)
            .unwrap_err(),
        StorageRootFailure::StaleSession,
    );
}

fn supervise_callback_disposal(case: &str) {
    supervise_callback_case("backend_callback_disposal_child", case);
}

fn supervise_callback_case(entry_point: &str, case: &str) {
    use std::io::Read;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    struct ReapChild(Child);
    impl Drop for ReapChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    let mut child = ReapChild(
        Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", entry_point, "--nocapture"])
            .env("NAGI_IDENTITY_DISPOSAL_CASE", case)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    let (status, timed_out) = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break (status, false);
        }
        if Instant::now() >= deadline {
            child.0.kill().unwrap();
            break (child.0.wait().unwrap(), true);
        }
        thread::sleep(Duration::from_millis(10));
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    child
        .0
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    println!("case={case}; timed_out={timed_out}; reaped=true; status={status}\n{stdout}{stderr}");
    assert!(!timed_out, "callback-disposal child exceeded its deadline");
    assert!(status.success(), "callback-disposal child failed");
}

#[test]
fn backend_callback_open_can_drop_the_issuing_service() {
    supervise_callback_disposal("open-drop");
}

#[test]
fn backend_callback_open_can_extract_the_issuing_service() {
    supervise_callback_disposal("open-into-parts");
}

#[test]
fn backend_callback_read_can_drop_the_issuing_service() {
    supervise_callback_disposal("read-drop");
}

#[test]
fn backend_callback_read_can_extract_the_issuing_service() {
    supervise_callback_disposal("read-into-parts");
}

#[test]
fn backend_callback_write_can_drop_the_issuing_service() {
    supervise_callback_disposal("write-drop");
}

#[test]
fn backend_callback_write_can_extract_the_issuing_service() {
    supervise_callback_disposal("write-into-parts");
}

struct LifecycleStorage {
    data: MemoryProfileStorage,
    service: TestService,
    caller: TestCaller,
    session: SessionId,
    store: SharedStore,
    saved: Vec<u8>,
    callback: DisposalCallback,
    recover: bool,
    calls: usize,
}

impl LifecycleStorage {
    fn lifecycle_on(&mut self, callback: DisposalCallback) {
        if callback != self.callback {
            return;
        }
        self.calls += 1;
        println!(
            "entered {callback:?} lifecycle callback; recover={}",
            self.recover
        );
        let result = if self.recover {
            self.service.recover(&self.caller).map(|_| ())
        } else {
            self.service
                .end_session(&self.caller, &self.session)
                .map(|_| ())
        };
        assert_eq!(result, Err(IdentityError::NotRecovered));
        assert_eq!(self.store.current().unwrap(), self.saved);
        assert_eq!(self.service.snapshot().to_json().unwrap(), self.saved);
        assert_eq!(
            self.service
                .resolve_current_identity(&self.caller)
                .unwrap_err(),
            ResolutionError::Unavailable,
        );
        // The service must retain the outstanding lease: retrying recovery
        // from this callback still cannot complete the lifecycle boundary.
        assert_eq!(
            self.service.recover(&self.caller),
            Err(IdentityError::NotRecovered),
        );
        assert_eq!(
            self.service.end_session(&self.caller, &self.session),
            Err(IdentityError::NotRecovered),
        );
        assert_eq!(self.store.current().unwrap(), self.saved);
        println!("lifecycle failed closed without waiting on its own callback");
    }
}

impl ProfileStorageBackend for LifecycleStorage {
    type Root = MemoryRoot;

    fn open_profile_root(
        &mut self,
        user: Option<UserId>,
        profile: &ProfileId,
    ) -> Result<Self::Root, StorageRootFailure> {
        self.lifecycle_on(DisposalCallback::Open);
        self.data.open_profile_root(user, profile)
    }

    fn read_object(
        &mut self,
        root: &Self::Root,
        object: ObjectId,
    ) -> Result<Vec<u8>, StorageRootFailure> {
        self.lifecycle_on(DisposalCallback::Read);
        self.data.read_object(root, object)
    }

    fn write_object(
        &mut self,
        root: &Self::Root,
        object: ObjectId,
        bytes: &[u8],
    ) -> Result<(), StorageRootFailure> {
        self.lifecycle_on(DisposalCallback::Write);
        self.data.write_object(root, object, bytes)
    }
}

#[test]
#[ignore = "entry point for the supervised callback-lifecycle regressions"]
fn backend_callback_lifecycle_child() {
    let case = std::env::var("NAGI_IDENTITY_DISPOSAL_CASE").expect("supervised case");
    let callback = match case.as_str() {
        "open-end" | "open-recover" => DisposalCallback::Open,
        "read-end" | "read-recover" => DisposalCallback::Read,
        "write-end" | "write-recover" => DisposalCallback::Write,
        _ => panic!("unknown callback-lifecycle case"),
    };
    let store = SharedStore::default();
    let f = SessionFixture::new(store.clone(), "callback-lifecycle");
    let mut roots = UserStorageRootAdapter::new(LifecycleStorage {
        data: MemoryProfileStorage::default(),
        service: f.service,
        caller: f.caller.clone(),
        session: f.session,
        saved: store.current().unwrap(),
        store,
        callback,
        recover: case.ends_with("recover"),
        calls: 0,
    });
    let access = IdentityAccessContext::new(&f.identity, &f.caller);
    let handle = roots
        .resolve_root(&access, &f.profile, &mut AllowProfileRoot)
        .unwrap();
    if callback != DisposalCallback::Open {
        roots
            .write_object(
                &access,
                &handle,
                &f.profile,
                ObjectId(1),
                b"in flight",
                &mut AllowProfileRoot,
            )
            .unwrap();
    }
    if callback == DisposalCallback::Read {
        assert_eq!(
            roots
                .read_object(
                    &access,
                    &handle,
                    &f.profile,
                    ObjectId(1),
                    &mut AllowProfileRoot,
                )
                .unwrap(),
            b"in flight",
        );
    }
    assert!(!f.identity.is_live());
    assert_eq!(
        roots.read_object(
            &access,
            &handle,
            &f.profile,
            ObjectId(1),
            &mut AllowProfileRoot
        ),
        Err(StorageRootFailure::StaleSession),
    );
    assert_eq!(
        roots.write_object(
            &access,
            &handle,
            &f.profile,
            ObjectId(1),
            b"must not write",
            &mut AllowProfileRoot,
        ),
        Err(StorageRootFailure::StaleSession),
    );
    assert_eq!(
        roots
            .resolve_root(&access, &f.profile, &mut AllowProfileRoot)
            .unwrap_err(),
        StorageRootFailure::StaleSession,
    );
    let mut backend = roots.into_backend();
    assert_eq!(backend.calls, 1);
    assert_eq!(backend.store.current().unwrap(), backend.saved);
    // Once the callback's operation guard has gone, an explicit recovery can
    // drain the retained lease, end the saved session and restore readiness.
    backend.service.recover(&backend.caller).unwrap();
    assert!(!f.identity.is_live());
    assert_eq!(
        backend
            .service
            .resolve_current_identity(&backend.caller)
            .unwrap_err(),
        ResolutionError::Ended,
    );
}

#[test]
fn backend_callback_lifecycle_read_can_end_session() {
    supervise_callback_case("backend_callback_lifecycle_child", "read-end");
}

#[test]
fn backend_callback_lifecycle_read_can_recover() {
    supervise_callback_case("backend_callback_lifecycle_child", "read-recover");
}

#[test]
fn backend_callback_lifecycle_open_can_end_session() {
    supervise_callback_case("backend_callback_lifecycle_child", "open-end");
}

#[test]
fn backend_callback_lifecycle_open_can_recover() {
    supervise_callback_case("backend_callback_lifecycle_child", "open-recover");
}

#[test]
fn backend_callback_lifecycle_write_can_end_session() {
    supervise_callback_case("backend_callback_lifecycle_child", "write-end");
}

#[test]
fn backend_callback_lifecycle_write_can_recover() {
    supervise_callback_case("backend_callback_lifecycle_child", "write-recover");
}

fn attempt_storage_boundary(
    roots: &mut UserStorageRootAdapter<MemoryProfileStorage>,
    access: &IdentityAccessContext<'_>,
    handle: &nagi_identity_session::StorageRootHandle,
    profile: &ProfileId,
    object: ObjectId,
    authorizer: &mut impl ProfileRootAuthorizer,
    operation: &str,
) -> Result<(), StorageRootFailure> {
    match operation {
        "open" => roots.resolve_root(access, profile, authorizer).map(|_| ()),
        "read" => roots
            .read_object(access, handle, profile, object, authorizer)
            .map(|_| ()),
        "write" => roots.write_object(
            access,
            handle,
            profile,
            object,
            b"must not overwrite",
            authorizer,
        ),
        _ => panic!("unknown storage-boundary test operation"),
    }
}

fn storage_call_counts(counts: &[Arc<AtomicUsize>; 3]) -> [usize; 3] {
    counts.each_ref().map(|count| count.load(Ordering::SeqCst))
}

struct SwitchingCaller {
    before: TestCaller,
    after: TestCaller,
    switched: Arc<std::sync::atomic::AtomicBool>,
}

impl TrustedCallerContext for SwitchingCaller {
    fn principal_id(&self) -> &PrincipalId {
        if self.switched.load(Ordering::SeqCst) {
            self.after.principal_id()
        } else {
            self.before.principal_id()
        }
    }

    fn context_id(&self) -> &CallerContextId {
        if self.switched.load(Ordering::SeqCst) {
            self.after.context_id()
        } else {
            self.before.context_id()
        }
    }
}

// This callback models a trusted host context changing while an injected
// profile authorizer runs. It is not a production authorization provider.
struct SwitchingRootAuthorizer(Arc<std::sync::atomic::AtomicBool>);

impl ProfileRootAuthorizer for SwitchingRootAuthorizer {
    fn authorize_profile_root(
        &mut self,
        _: &dyn TrustedCallerContext,
        _: &ResolvedIdentity,
        _: &ProfileId,
    ) -> Result<(), StorageRootFailure> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn changed_caller_during_storage_authorization(operation: &str, principal: bool) {
    let f = SessionFixture::new(SharedStore::default(), "storage-late-caller");
    let backend = MemoryProfileStorage::default();
    let counts = [
        backend.opens.clone(),
        backend.reads.clone(),
        backend.writes.clone(),
    ];
    let mut roots = UserStorageRootAdapter::new(backend);
    let handle = roots
        .resolve_root(
            &IdentityAccessContext::new(&f.identity, &f.caller),
            &f.profile,
            &mut AllowProfileRoot,
        )
        .unwrap();
    roots
        .write_object(
            &IdentityAccessContext::new(&f.identity, &f.caller),
            &handle,
            &f.profile,
            ObjectId(9),
            b"original private content",
            &mut AllowProfileRoot,
        )
        .unwrap();
    let before = storage_call_counts(&counts);
    let switched = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut after = f.caller.clone();
    if principal {
        after.principal = PrincipalId::new("other.storage-test").unwrap();
    } else {
        after.context = CallerContextId::new("caller_other-storage-test").unwrap();
    }
    let caller = SwitchingCaller {
        before: f.caller.clone(),
        after,
        switched: switched.clone(),
    };
    assert_eq!(
        attempt_storage_boundary(
            &mut roots,
            &IdentityAccessContext::new(&f.identity, &caller),
            &handle,
            &f.profile,
            ObjectId(9),
            &mut SwitchingRootAuthorizer(switched),
            operation,
        ),
        Err(StorageRootFailure::CallerMismatch),
        "{operation}; principal_changed={principal}",
    );
    assert_eq!(storage_call_counts(&counts), before);
}

#[test]
fn storage_boundary_open_rechecks_principal_after_authorizer() {
    changed_caller_during_storage_authorization("open", true);
}

#[test]
fn storage_boundary_read_rechecks_principal_after_authorizer() {
    changed_caller_during_storage_authorization("read", true);
}

#[test]
fn storage_boundary_write_rechecks_principal_after_authorizer() {
    changed_caller_during_storage_authorization("write", true);
}

#[test]
fn storage_boundary_open_rechecks_context_after_authorizer() {
    changed_caller_during_storage_authorization("open", false);
}

#[test]
fn storage_boundary_read_rechecks_context_after_authorizer() {
    changed_caller_during_storage_authorization("read", false);
}

#[test]
fn storage_boundary_write_rechecks_context_after_authorizer() {
    changed_caller_during_storage_authorization("write", false);
}

struct BoundaryRootDecision {
    result: Result<(), StorageRootFailure>,
    calls: usize,
}

impl ProfileRootAuthorizer for BoundaryRootDecision {
    fn authorize_profile_root(
        &mut self,
        _: &dyn TrustedCallerContext,
        _: &ResolvedIdentity,
        _: &ProfileId,
    ) -> Result<(), StorageRootFailure> {
        self.calls += 1;
        self.result
    }
}

#[test]
fn storage_boundary_reauthorizes_each_operation_after_handle_resolution() {
    let f = SessionFixture::new(SharedStore::default(), "storage-policy-change");
    let backend = MemoryProfileStorage::default();
    let counts = [
        backend.opens.clone(),
        backend.reads.clone(),
        backend.writes.clone(),
    ];
    let mut roots = UserStorageRootAdapter::new(backend);
    let access = IdentityAccessContext::new(&f.identity, &f.caller);
    let handle = roots
        .resolve_root(&access, &f.profile, &mut AllowProfileRoot)
        .unwrap();
    let before = storage_call_counts(&counts);
    let mut policy = BoundaryRootDecision {
        result: Ok(()),
        calls: 0,
    };
    for reason in [StorageRootFailure::Denied, StorageRootFailure::Unavailable] {
        policy.result = Err(reason);
        for operation in ["open", "read", "write"] {
            assert_eq!(
                attempt_storage_boundary(
                    &mut roots,
                    &access,
                    &handle,
                    &f.profile,
                    ObjectId(99),
                    &mut policy,
                    operation
                ),
                Err(reason),
            );
            assert_eq!(storage_call_counts(&counts), before);
        }
    }
    assert_eq!(policy.calls, 6);
    policy.result = Ok(());
    roots
        .write_object(
            &access,
            &handle,
            &f.profile,
            ObjectId(99),
            b"allowed now",
            &mut policy,
        )
        .unwrap();
    assert_eq!(
        roots
            .read_object(&access, &handle, &f.profile, ObjectId(99), &mut policy)
            .unwrap(),
        b"allowed now"
    );
    assert_eq!(policy.calls, 8);
}

#[test]
fn storage_boundary_rejects_wrong_access_pairs_before_authorizer() {
    let f = SessionFixture::new(SharedStore::default(), "storage-wrong-pair");
    let backend = MemoryProfileStorage::default();
    let counts = [
        backend.opens.clone(),
        backend.reads.clone(),
        backend.writes.clone(),
    ];
    let mut roots = UserStorageRootAdapter::new(backend);
    let handle = roots
        .resolve_root(
            &IdentityAccessContext::new(&f.identity, &f.caller),
            &f.profile,
            &mut AllowProfileRoot,
        )
        .unwrap();
    let before = storage_call_counts(&counts);
    let mut other_principal = f.caller.clone();
    other_principal.principal = PrincipalId::new("other.boundary-test").unwrap();
    let other_context = TestCaller::new("other-boundary-context");
    let other_profile = ProfileId::new("profile_other-boundary").unwrap();
    let mut policy = BoundaryRootDecision {
        result: Ok(()),
        calls: 0,
    };
    for (caller, profile, failure) in [
        (
            &other_principal,
            &f.profile,
            StorageRootFailure::CallerMismatch,
        ),
        (
            &other_context,
            &f.profile,
            StorageRootFailure::CallerMismatch,
        ),
        (&f.caller, &other_profile, StorageRootFailure::CrossProfile),
    ] {
        for operation in ["open", "read", "write"] {
            assert_eq!(
                attempt_storage_boundary(
                    &mut roots,
                    &IdentityAccessContext::new(&f.identity, caller),
                    &handle,
                    profile,
                    ObjectId(99),
                    &mut policy,
                    operation
                ),
                Err(failure)
            );
            assert_eq!(storage_call_counts(&counts), before);
        }
    }
    assert_eq!(policy.calls, 0);
}

#[test]
fn storage_boundary_retired_root_cannot_be_replayed_for_another_object() {
    let f = SessionFixture::new(SharedStore::default(), "storage-retired-root");
    let backend = MemoryProfileStorage::default();
    let counts = [
        backend.opens.clone(),
        backend.reads.clone(),
        backend.writes.clone(),
    ];
    let mut roots = UserStorageRootAdapter::new(backend);
    let access = IdentityAccessContext::new(&f.identity, &f.caller);
    let handle = roots
        .resolve_root(&access, &f.profile, &mut AllowProfileRoot)
        .unwrap();
    roots.revoke_session(&f.session);
    assert!(f.identity.is_live());
    let before = storage_call_counts(&counts);
    for object in [ObjectId(9), ObjectId(99)] {
        for operation in ["read", "write"] {
            assert_eq!(
                attempt_storage_boundary(
                    &mut roots,
                    &access,
                    &handle,
                    &f.profile,
                    object,
                    &mut AllowProfileRoot,
                    operation
                ),
                Err(StorageRootFailure::InvalidHandle)
            );
            assert_eq!(storage_call_counts(&counts), before);
        }
    }
}

#[test]
fn storage_boundary_rechecks_lease_after_authorizer_callback() {
    struct ExpiringRoot(TestClock);
    impl ProfileRootAuthorizer for ExpiringRoot {
        fn authorize_profile_root(
            &mut self,
            _: &dyn TrustedCallerContext,
            _: &ResolvedIdentity,
            _: &ProfileId,
        ) -> Result<(), StorageRootFailure> {
            self.0.set(200);
            Ok(())
        }
    }
    struct EndingRoot {
        service: TestService,
        caller: TestCaller,
        session: SessionId,
    }
    impl ProfileRootAuthorizer for EndingRoot {
        fn authorize_profile_root(
            &mut self,
            _: &dyn TrustedCallerContext,
            _: &ResolvedIdentity,
            _: &ProfileId,
        ) -> Result<(), StorageRootFailure> {
            self.service
                .end_session(&self.caller, &self.session)
                .unwrap();
            Ok(())
        }
    }
    for operation in ["open", "read", "write"] {
        for expire in [false, true] {
            let f = SessionFixture::new(SharedStore::default(), "storage-authorizer-revocation");
            let backend = MemoryProfileStorage::default();
            let counts = [
                backend.opens.clone(),
                backend.reads.clone(),
                backend.writes.clone(),
            ];
            let mut roots = UserStorageRootAdapter::new(backend);
            let access = IdentityAccessContext::new(&f.identity, &f.caller);
            let handle = roots
                .resolve_root(&access, &f.profile, &mut AllowProfileRoot)
                .unwrap();
            let before = storage_call_counts(&counts);
            let result = if expire {
                attempt_storage_boundary(
                    &mut roots,
                    &access,
                    &handle,
                    &f.profile,
                    ObjectId(9),
                    &mut ExpiringRoot(f.clock),
                    operation,
                )
            } else {
                attempt_storage_boundary(
                    &mut roots,
                    &access,
                    &handle,
                    &f.profile,
                    ObjectId(9),
                    &mut EndingRoot {
                        service: f.service,
                        caller: f.caller.clone(),
                        session: f.session,
                    },
                    operation,
                )
            };
            assert_eq!(result, Err(StorageRootFailure::StaleSession));
            assert_eq!(storage_call_counts(&counts), before);
        }
    }
}

// Test-only authorization callback: the request is authorized for the first
// caller, then the trusted host view changes before the decision returns.
struct SwitchingIdentityAuthorizer {
    action: IdentityAction,
    allowed: TestCaller,
    armed: Arc<std::sync::atomic::AtomicBool>,
    switched: Arc<std::sync::atomic::AtomicBool>,
    requests: Arc<Mutex<Vec<AuthorizationRequest>>>,
}

impl IdentityAuthorizer for SwitchingIdentityAuthorizer {
    fn authorize(&mut self, request: &AuthorizationRequest) -> Result<(), AuthorizationFailure> {
        if !self.armed.load(Ordering::SeqCst) || request.action != self.action {
            return Ok(());
        }
        self.requests.lock().unwrap().push(request.clone());
        if request.caller_principal != self.allowed.principal
            || request.caller_context != self.allowed.context
        {
            return Err(AuthorizationFailure::Denied);
        }
        self.switched.store(true, Ordering::SeqCst);
        Ok(())
    }
}

fn invoke_identity_boundary(
    service: &mut IdentityService<
        SharedStore,
        TestIds,
        TestClock,
        SwitchingIdentityAuthorizer,
        TestCleaner,
        NoopIdentityEventSink,
    >,
    caller: &impl TrustedCallerContext,
    session: &SessionId,
    action: IdentityAction,
) -> Result<(), IdentityError> {
    match action {
        IdentityAction::ResolveCurrentIdentity => match service.resolve_current_identity(caller) {
            Ok(_) => Ok(()),
            // Normalize only the existing denial errors for this test helper.
            Err(ResolutionError::Denied) => Err(IdentityError::AuthorizationDenied),
            Err(other) => panic!("unexpected resolution failure: {other:?}"),
        },
        IdentityAction::EndSession => service.end_session(caller, session).map(|_| ()),
        IdentityAction::Recover => service.recover(caller).map(|_| ()),
        _ => panic!("unexpected identity-boundary test action"),
    }
}

fn changed_caller_during_identity_authorization(action: IdentityAction, principal: bool) {
    let before = TestCaller::new("identity-late-caller");
    let mut after = before.clone();
    if principal {
        after.principal = PrincipalId::new("other.identity-boundary-test").unwrap();
    } else {
        after.context = CallerContextId::new("caller_other-identity-boundary").unwrap();
    }
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let switched = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let store = SharedStore::default();
    let mut service = IdentityService::new(
        store.clone(),
        TestIds::default(),
        TestClock(Arc::new(AtomicU64::new(100))),
        SwitchingIdentityAuthorizer {
            action,
            allowed: before.clone(),
            armed: armed.clone(),
            switched: switched.clone(),
            requests: requests.clone(),
        },
        TestCleaner::default(),
        NoopIdentityEventSink,
    );
    service.recover(&before).unwrap();
    let mut identities = Vec::new();
    let mut sessions = Vec::new();
    for (caller, label, key) in [
        (&before, "First", "identity-boundary-first"),
        (&after, "Second", "identity-boundary-second"),
    ] {
        let (user, profile) = service
            .create_local_user(caller, metadata(label).0, metadata(label).1)
            .unwrap();
        sessions.push(
            service
                .login_local(
                    caller,
                    user,
                    profile,
                    Some(UnixMillis::new(200)),
                    nagi_identity_session::IdempotencyKey::new(key).unwrap(),
                )
                .unwrap(),
        );
        identities.push(service.resolve_current_identity(caller).unwrap());
    }
    let saved = store.current().unwrap();
    armed.store(true, Ordering::SeqCst);
    // The second caller is really denied by this callback's decision. It must
    // not inherit the first caller's earlier successful decision.
    assert_eq!(
        invoke_identity_boundary(&mut service, &after, &sessions[1], action),
        Err(IdentityError::AuthorizationDenied),
    );
    requests.lock().unwrap().clear();
    let caller = SwitchingCaller {
        before: before.clone(),
        after,
        switched: switched.clone(),
    };
    assert_eq!(
        invoke_identity_boundary(&mut service, &caller, &sessions[1], action),
        Err(IdentityError::AuthorizationDenied),
        "{action:?}; principal_changed={principal}",
    );
    assert!(switched.load(Ordering::SeqCst));
    let seen = requests.lock().unwrap();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].caller_principal, before.principal);
    assert_eq!(seen[0].caller_context, before.context);
    assert_eq!(store.current().unwrap(), saved);
    assert!(identities.iter().all(ResolvedIdentity::is_live));
}

#[test]
fn identity_authorization_resolve_rechecks_principal_after_callback() {
    changed_caller_during_identity_authorization(IdentityAction::ResolveCurrentIdentity, true);
}

#[test]
fn identity_authorization_resolve_rechecks_context_after_callback() {
    changed_caller_during_identity_authorization(IdentityAction::ResolveCurrentIdentity, false);
}

#[test]
fn identity_authorization_end_rechecks_principal_after_callback() {
    changed_caller_during_identity_authorization(IdentityAction::EndSession, true);
}

#[test]
fn identity_authorization_end_rechecks_context_after_callback() {
    changed_caller_during_identity_authorization(IdentityAction::EndSession, false);
}

#[test]
fn identity_authorization_recover_rechecks_principal_after_callback() {
    changed_caller_during_identity_authorization(IdentityAction::Recover, true);
}

#[test]
fn identity_authorization_recover_rechecks_context_after_callback() {
    changed_caller_during_identity_authorization(IdentityAction::Recover, false);
}

// Test-only host callbacks. The clock remains the same deterministic source
// across clones; its selected callback changes only the presented caller view.
#[derive(Clone)]
struct CallerBoundaryClock {
    remaining: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
    now: Arc<AtomicU64>,
    expire_on_trigger: Arc<std::sync::atomic::AtomicBool>,
    switched: Arc<std::sync::atomic::AtomicBool>,
}

impl CallerBoundaryClock {
    fn arm(&self, call: usize) {
        self.calls.store(0, Ordering::SeqCst);
        self.remaining.store(call, Ordering::SeqCst);
    }
}

impl Clock for CallerBoundaryClock {
    fn now(&self) -> UnixMillis {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self
            .remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            == Ok(1)
        {
            if self.expire_on_trigger.load(Ordering::SeqCst) {
                self.now.store(200, Ordering::SeqCst);
            }
            self.switched.store(true, Ordering::SeqCst);
        }
        UnixMillis::new(self.now.load(Ordering::SeqCst))
    }
}

struct CallerBoundaryEvents {
    armed: Arc<std::sync::atomic::AtomicBool>,
    calls: Arc<AtomicUsize>,
    switched: Arc<std::sync::atomic::AtomicBool>,
    records: Arc<Mutex<Vec<nagi_identity_session::IdentityEvent>>>,
    switch_on_end: Arc<std::sync::atomic::AtomicBool>,
}

impl nagi_identity_session::IdentityEventSink for CallerBoundaryEvents {
    fn emit(&mut self, event: &nagi_identity_session::IdentityEvent) -> bool {
        self.records.lock().unwrap().push(event.clone());
        if event.kind == nagi_identity_session::IdentityEventKind::CurrentIdentityResolved {
            self.calls.fetch_add(1, Ordering::SeqCst);
        }
        let switch_kind = if self.switch_on_end.load(Ordering::SeqCst) {
            nagi_identity_session::IdentityEventKind::SessionEnded
        } else {
            nagi_identity_session::IdentityEventKind::CurrentIdentityResolved
        };
        if event.kind == switch_kind && self.armed.swap(false, Ordering::SeqCst) {
            self.switched.store(true, Ordering::SeqCst);
        }
        true
    }
}

#[derive(Clone, Default)]
struct RecordingBoundaryAuthorizer(Arc<Mutex<Vec<AuthorizationRequest>>>);

impl IdentityAuthorizer for RecordingBoundaryAuthorizer {
    fn authorize(&mut self, request: &AuthorizationRequest) -> Result<(), AuthorizationFailure> {
        self.0.lock().unwrap().push(request.clone());
        Ok(())
    }
}

type CallerBoundaryService = IdentityService<
    SharedStore,
    TestIds,
    CallerBoundaryClock,
    RecordingBoundaryAuthorizer,
    TestCleaner,
    CallerBoundaryEvents,
>;

struct CallerBoundaryFixture {
    service: CallerBoundaryService,
    store: SharedStore,
    clock: CallerBoundaryClock,
    event_armed: Arc<std::sync::atomic::AtomicBool>,
    event_calls: Arc<AtomicUsize>,
    event_records: Arc<Mutex<Vec<nagi_identity_session::IdentityEvent>>>,
    event_switch_on_end: Arc<std::sync::atomic::AtomicBool>,
    authorizations: RecordingBoundaryAuthorizer,
    caller: SwitchingCaller,
    identities: Vec<ResolvedIdentity>,
    sessions: Vec<SessionId>,
}

impl CallerBoundaryFixture {
    fn new(principal: bool) -> Self {
        Self::with_second_expiry(principal, 200)
    }

    fn with_second_expiry(principal: bool, second_expiry: u64) -> Self {
        let before = TestCaller::new("clock-event-boundary");
        let mut after = before.clone();
        if principal {
            after.principal = PrincipalId::new("other.clock-event-boundary").unwrap();
        } else {
            after.context = CallerContextId::new("caller_other-clock-event-boundary").unwrap();
        }
        let switched = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let clock = CallerBoundaryClock {
            remaining: Arc::new(AtomicUsize::new(0)),
            calls: Arc::new(AtomicUsize::new(0)),
            now: Arc::new(AtomicU64::new(100)),
            expire_on_trigger: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            switched: switched.clone(),
        };
        let event_armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let event_calls = Arc::new(AtomicUsize::new(0));
        let event_records = Arc::new(Mutex::new(Vec::new()));
        let event_switch_on_end = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let store = SharedStore::default();
        let authorizations = RecordingBoundaryAuthorizer::default();
        let mut service = IdentityService::new(
            store.clone(),
            TestIds::default(),
            clock.clone(),
            authorizations.clone(),
            TestCleaner::default(),
            CallerBoundaryEvents {
                armed: event_armed.clone(),
                calls: event_calls.clone(),
                switched: switched.clone(),
                records: event_records.clone(),
                switch_on_end: event_switch_on_end.clone(),
            },
        );
        service.recover(&before).unwrap();
        let mut sessions = Vec::new();
        let mut identities = Vec::new();
        for (caller, label, key, expiry) in [
            (&before, "Before", "callback-boundary-before", 200),
            (&after, "After", "callback-boundary-after", second_expiry),
        ] {
            let (user, profile) = service
                .create_local_user(caller, metadata(label).0, metadata(label).1)
                .unwrap();
            sessions.push(
                service
                    .login_local(
                        caller,
                        user,
                        profile,
                        Some(UnixMillis::new(expiry)),
                        nagi_identity_session::IdempotencyKey::new(key).unwrap(),
                    )
                    .unwrap(),
            );
            identities.push(service.resolve_current_identity(caller).unwrap());
        }
        event_calls.store(0, Ordering::SeqCst);
        event_records.lock().unwrap().clear();
        Self {
            service,
            store,
            clock,
            event_armed,
            event_calls,
            event_records,
            event_switch_on_end,
            authorizations,
            caller: SwitchingCaller {
                before,
                after,
                switched,
            },
            identities,
            sessions,
        }
    }
}

fn resolver_caller_changes_after_authorization(principal: bool, event: bool) {
    // Include the clock in the final post-diagnostic liveness check.
    for clock_call in if event { vec![0] } else { vec![1, 2, 3] } {
        let mut f = CallerBoundaryFixture::new(principal);
        let saved = f.store.current().unwrap();
        if event {
            f.event_armed.store(true, Ordering::SeqCst);
        } else {
            f.clock.arm(clock_call);
        }
        assert_eq!(
            f.service.resolve_current_identity(&f.caller).unwrap_err(),
            ResolutionError::Denied,
            "principal_changed={principal}; event={event}; clock_call={clock_call}",
        );
        assert!(f.caller.switched.load(Ordering::SeqCst));
        // The third clock runs after diagnostic delivery; earlier mismatches
        // must not report a successful resolution diagnostic.
        assert_eq!(
            f.event_calls.load(Ordering::SeqCst),
            usize::from(event || clock_call == 3),
        );
        let mut expected_events = Vec::new();
        if event || clock_call == 3 {
            expected_events.push(nagi_identity_session::IdentityEvent {
                kind: nagi_identity_session::IdentityEventKind::CurrentIdentityResolved,
                user_id: f.identities[0].user_id(),
                profile_id: Some(f.identities[0].profile_id().clone()),
                session_id: Some(f.sessions[0].clone()),
                reason: None,
            });
        }
        expected_events.push(nagi_identity_session::IdentityEvent {
            kind: nagi_identity_session::IdentityEventKind::CurrentIdentityResolutionDenied,
            user_id: f.identities[0].user_id(),
            profile_id: Some(f.identities[0].profile_id().clone()),
            session_id: Some(f.sessions[0].clone()),
            reason: Some(nagi_identity_session::IdentityEventReason::Denied),
        });
        assert_eq!(*f.event_records.lock().unwrap(), expected_events);
        assert_eq!(f.store.current().unwrap(), saved);
        assert!(f.identities.iter().all(ResolvedIdentity::is_live));
        let current = f.service.resolve_current_identity(&f.caller.after).unwrap();
        assert_eq!(current.profile_id(), f.identities[1].profile_id());
        assert_eq!(current.principal_id(), f.caller.after.principal_id());
    }
}

#[test]
fn callback_boundary_resolver_clock_rechecks_principal() {
    resolver_caller_changes_after_authorization(true, false);
}

#[test]
fn callback_boundary_resolver_clock_rechecks_context() {
    resolver_caller_changes_after_authorization(false, false);
}

#[test]
fn callback_boundary_resolver_event_rechecks_principal() {
    resolver_caller_changes_after_authorization(true, true);
}

#[test]
fn callback_boundary_resolver_event_rechecks_context() {
    resolver_caller_changes_after_authorization(false, true);
}

fn storage_caller_changes_in_live_lease_clock(operation: &str, principal: bool) {
    let mut f = CallerBoundaryFixture::new(principal);
    let backend = MemoryProfileStorage::default();
    let counts = [
        backend.opens.clone(),
        backend.reads.clone(),
        backend.writes.clone(),
    ];
    let mut roots = UserStorageRootAdapter::new(backend);
    let identity = &f.identities[0];
    let access = IdentityAccessContext::new(identity, &f.caller);
    let handle = roots
        .resolve_root(&access, identity.profile_id(), &mut AllowProfileRoot)
        .unwrap();
    roots
        .write_object(
            &access,
            &handle,
            identity.profile_id(),
            ObjectId(9),
            b"unchanged",
            &mut AllowProfileRoot,
        )
        .unwrap();
    let before = storage_call_counts(&counts);
    // First clock is authorize_scope's is_live; the second is inside the
    // acquired operation guard. The existing post-lease check must reject it.
    f.clock.arm(2);
    assert_eq!(
        attempt_storage_boundary(
            &mut roots,
            &access,
            &handle,
            identity.profile_id(),
            ObjectId(9),
            &mut AllowProfileRoot,
            operation,
        ),
        Err(StorageRootFailure::CallerMismatch),
    );
    assert_eq!(f.clock.calls.load(Ordering::SeqCst), 2);
    assert!(f.caller.switched.load(Ordering::SeqCst));
    assert_eq!(storage_call_counts(&counts), before);
    // The denied operation must also release its completion guard.
    assert_eq!(
        f.service
            .end_session(&f.caller.before, &f.sessions[0])
            .unwrap(),
        EndSessionResult::Ended,
    );
    assert!(!f.identities[0].is_live());
    assert!(f.identities[1].is_live());
}

#[test]
fn callback_boundary_storage_open_clock_rechecks_principal() {
    storage_caller_changes_in_live_lease_clock("open", true);
}

#[test]
fn callback_boundary_storage_open_clock_rechecks_context() {
    storage_caller_changes_in_live_lease_clock("open", false);
}

#[test]
fn callback_boundary_storage_read_clock_rechecks_principal() {
    storage_caller_changes_in_live_lease_clock("read", true);
}

#[test]
fn callback_boundary_storage_read_clock_rechecks_context() {
    storage_caller_changes_in_live_lease_clock("read", false);
}

#[test]
fn callback_boundary_storage_write_clock_rechecks_principal() {
    storage_caller_changes_in_live_lease_clock("write", true);
}

#[test]
fn callback_boundary_storage_write_clock_rechecks_context() {
    storage_caller_changes_in_live_lease_clock("write", false);
}

fn expiry_clock_cannot_end_a_session_for_a_changed_caller(principal: bool) {
    let mut f = CallerBoundaryFixture::with_second_expiry(principal, 400);
    let saved = f.store.current().unwrap();
    f.authorizations.0.lock().unwrap().clear();
    f.clock.now.store(200, Ordering::SeqCst);
    f.clock.arm(1);
    let result = f.service.resolve_current_identity(&f.caller).map(|_| ());
    let denied_events = f.event_records.lock().unwrap().clone();
    let end_requests: Vec<_> = f
        .authorizations
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.action == IdentityAction::EndSession)
        .cloned()
        .collect();
    let second = f
        .service
        .resolve_current_identity(&f.caller.after)
        .map(|identity| identity.session_id().clone());
    assert_eq!(
        result,
        Err(ResolutionError::Denied),
        "end_requests={end_requests:?}; second_current={second:?}",
    );
    assert!(end_requests.is_empty());
    assert_eq!(
        denied_events,
        vec![nagi_identity_session::IdentityEvent {
            kind: nagi_identity_session::IdentityEventKind::CurrentIdentityResolutionDenied,
            user_id: f.identities[0].user_id(),
            profile_id: Some(f.identities[0].profile_id().clone()),
            session_id: Some(f.sessions[0].clone()),
            reason: Some(nagi_identity_session::IdentityEventReason::Denied),
        }],
    );
    assert_eq!(second, Ok(f.sessions[1].clone()));
    assert_eq!(f.store.current().unwrap(), saved);
    assert!(f.identities[1].is_live());
    // A stable original caller can still expire its own session, and must not
    // consume the second caller's current-session index or authorization.
    assert_eq!(
        f.service
            .resolve_current_identity(&f.caller.before)
            .unwrap_err(),
        ResolutionError::Expired,
    );
    let correct_end_requests: Vec<_> = f
        .authorizations
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|request| request.action == IdentityAction::EndSession)
        .cloned()
        .collect();
    assert_eq!(correct_end_requests.len(), 1);
    assert_eq!(
        correct_end_requests[0].caller_principal,
        f.caller.before.principal,
    );
    assert_eq!(
        correct_end_requests[0].caller_context,
        f.caller.before.context,
    );
    assert_eq!(
        correct_end_requests[0].session_id.as_ref(),
        Some(&f.sessions[0]),
    );
    assert_eq!(
        f.service
            .resolve_current_identity(&f.caller.after)
            .unwrap()
            .session_id(),
        &f.sessions[1],
    );
}

#[test]
fn resolver_termination_expiry_rechecks_principal() {
    expiry_clock_cannot_end_a_session_for_a_changed_caller(true);
}

#[test]
fn resolver_termination_expiry_rechecks_context() {
    expiry_clock_cannot_end_a_session_for_a_changed_caller(false);
}

fn second_clock_cannot_return_another_callers_ended_status(principal: bool) {
    let mut f = CallerBoundaryFixture::with_second_expiry(principal, 400);
    let saved = f.store.current().unwrap();
    f.authorizations.0.lock().unwrap().clear();
    f.clock.expire_on_trigger.store(true, Ordering::SeqCst);
    f.clock.arm(2);
    let result = f.service.resolve_current_identity(&f.caller).map(|_| ());
    assert_eq!(result, Err(ResolutionError::Denied));
    assert_eq!(f.clock.calls.load(Ordering::SeqCst), 2);
    assert!(f.caller.switched.load(Ordering::SeqCst));
    assert_eq!(f.store.current().unwrap(), saved);
    assert!(!f
        .authorizations
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|request| request.action == IdentityAction::EndSession));
    assert_eq!(
        *f.event_records.lock().unwrap(),
        vec![nagi_identity_session::IdentityEvent {
            kind: nagi_identity_session::IdentityEventKind::CurrentIdentityResolutionDenied,
            user_id: f.identities[0].user_id(),
            profile_id: Some(f.identities[0].profile_id().clone()),
            session_id: Some(f.sessions[0].clone()),
            reason: Some(nagi_identity_session::IdentityEventReason::Denied),
        }],
    );
    assert!(f.identities[1].is_live());
    assert_eq!(
        f.service
            .resolve_current_identity(&f.caller.after)
            .unwrap()
            .session_id(),
        &f.sessions[1],
    );
}

#[test]
fn resolver_termination_second_clock_rechecks_principal() {
    second_clock_cannot_return_another_callers_ended_status(true);
}

#[test]
fn resolver_termination_second_clock_rechecks_context() {
    second_clock_cannot_return_another_callers_ended_status(false);
}

struct ResolutionExpiryEvents {
    clock: TestClock,
    armed: Arc<std::sync::atomic::AtomicBool>,
    expire: bool,
    succeeds: bool,
}

impl nagi_identity_session::IdentityEventSink for ResolutionExpiryEvents {
    fn emit(&mut self, event: &nagi_identity_session::IdentityEvent) -> bool {
        if event.kind == nagi_identity_session::IdentityEventKind::CurrentIdentityResolved
            && self.armed.swap(false, Ordering::SeqCst)
            && self.expire
        {
            self.clock.set(200);
        }
        self.succeeds
    }
}

fn resolve_with_diagnostic_clock(expire: bool, succeeds: bool) {
    let store = SharedStore::default();
    let clock = TestClock(Arc::new(AtomicU64::new(100)));
    let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut service = IdentityService::new(
        store.clone(),
        TestIds::default(),
        clock.clone(),
        TestAuthorizer::default(),
        TestCleaner::default(),
        ResolutionExpiryEvents {
            clock,
            armed: armed.clone(),
            expire,
            succeeds,
        },
    );
    let caller = TestCaller::new("diagnostic-expiry-boundary");
    service.recover(&caller).unwrap();
    let (user, profile) = service
        .create_local_user(&caller, metadata("User").0, metadata("Home").1)
        .unwrap();
    let session = service
        .login_local(
            &caller,
            user,
            profile,
            Some(UnixMillis::new(200)),
            nagi_identity_session::IdempotencyKey::new("diagnostic-expiry").unwrap(),
        )
        .unwrap();
    let previous = service.resolve_current_identity(&caller).unwrap();
    let saved = store.current().unwrap();
    armed.store(true, Ordering::SeqCst);
    let result = service.resolve_current_identity(&caller);
    if expire {
        // This is the same existing error used when issued evidence loses
        // liveness during resolution, rather than a new public error policy.
        assert_eq!(result.unwrap_err(), ResolutionError::Ended);
        assert!(!previous.is_live());
        assert_eq!(store.current().unwrap(), saved);
        assert_eq!(
            service.resolve_current_identity(&caller).unwrap_err(),
            ResolutionError::Expired,
        );
        assert_eq!(
            service.end_session(&caller, &session).unwrap(),
            EndSessionResult::AlreadyEnded,
        );
    } else {
        let identity = result.unwrap();
        assert!(identity.is_live());
        assert_eq!(identity.session_id(), &session);
        assert!(previous.is_live());
        assert_eq!(store.current().unwrap(), saved);
    }
}

#[test]
fn resolver_termination_diagnostic_expiry_is_not_returned_as_live() {
    for succeeds in [false, true] {
        resolve_with_diagnostic_clock(true, succeeds);
    }
}

#[test]
fn resolver_termination_failed_diagnostic_keeps_live_resolution() {
    resolve_with_diagnostic_clock(false, false);
}

fn post_authorization_expiry_callback_keeps_original_scope(principal: bool) {
    for event in [false, true] {
        let mut f = CallerBoundaryFixture::with_second_expiry(principal, 400);
        let second_before = f
            .service
            .snapshot()
            .sessions
            .iter()
            .find(|session| session.session_id == f.sessions[1])
            .unwrap()
            .clone();
        f.authorizations.0.lock().unwrap().clear();
        f.clock.now.store(200, Ordering::SeqCst);
        if event {
            f.event_switch_on_end.store(true, Ordering::SeqCst);
            f.event_armed.store(true, Ordering::SeqCst);
        } else {
            // The first clock selects the expired session; this second clock
            // is the end_record timestamp, after EndSession authorization.
            f.clock.arm(2);
        }
        let result = f.service.resolve_current_identity(&f.caller);
        println!("postauth principal_changed={principal}; event={event}; result={result:?}");
        // The contract does not settle the terminal error's classification
        // after a committed lifecycle callback. No identity may be returned.
        assert!(result.is_err());
        assert!(f.caller.switched.load(Ordering::SeqCst));
        let end_requests: Vec<_> = f
            .authorizations
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.action == IdentityAction::EndSession)
            .cloned()
            .collect();
        assert_eq!(end_requests.len(), 1);
        assert_eq!(end_requests[0].caller_principal, f.caller.before.principal);
        assert_eq!(end_requests[0].caller_context, f.caller.before.context);
        assert_eq!(end_requests[0].session_id.as_ref(), Some(&f.sessions[0]));
        let first = f
            .service
            .snapshot()
            .sessions
            .iter()
            .find(|session| session.session_id == f.sessions[0])
            .unwrap();
        assert!(matches!(
            first.lifecycle,
            nagi_identity_session::SessionLifecycle::Ended {
                reason: nagi_identity_session::SessionEndReason::Expired,
                ..
            }
        ));
        let second_after = f
            .service
            .snapshot()
            .sessions
            .iter()
            .find(|session| session.session_id == f.sessions[1])
            .unwrap();
        assert_eq!(second_after, &second_before);
        assert!(f.identities[1].is_live());
        assert_eq!(
            f.service
                .resolve_current_identity(&f.caller.after)
                .unwrap()
                .session_id(),
            &f.sessions[1],
        );
    }
}

#[test]
fn resolver_postauth_expiry_preserves_scope_for_principal_change() {
    post_authorization_expiry_callback_keeps_original_scope(true);
}

#[test]
fn resolver_postauth_expiry_preserves_scope_for_context_change() {
    post_authorization_expiry_callback_keeps_original_scope(false);
}

#[test]
fn storage_boundary_identical_write_retry_reauthorizes_after_success() {
    let f = SessionFixture::new(SharedStore::default(), "storage-write-retry");
    let backend = MemoryProfileStorage::default();
    let counts = [
        backend.opens.clone(),
        backend.reads.clone(),
        backend.writes.clone(),
    ];
    let mut roots = UserStorageRootAdapter::new(backend);
    let access = IdentityAccessContext::new(&f.identity, &f.caller);
    let mut policy = BoundaryRootDecision {
        result: Ok(()),
        calls: 0,
    };
    let handle = roots
        .resolve_root(&access, &f.profile, &mut policy)
        .unwrap();
    assert_eq!(policy.calls, 1);
    assert_eq!(storage_call_counts(&counts), [1, 0, 0]);
    let object = ObjectId(99);
    let payload = b"same write arguments";
    roots
        .write_object(&access, &handle, &f.profile, object, payload, &mut policy)
        .unwrap();
    assert_eq!(policy.calls, 2);
    assert_eq!(storage_call_counts(&counts), [1, 0, 1]);

    // The earlier success and identical arguments cannot cache authorization
    // or substitute for a fresh decision on the current call.
    for (reason, expected_calls) in [
        (StorageRootFailure::Denied, 3),
        (StorageRootFailure::Unavailable, 4),
    ] {
        policy.result = Err(reason);
        assert_eq!(
            roots.write_object(&access, &handle, &f.profile, object, payload, &mut policy),
            Err(reason)
        );
        assert_eq!(policy.calls, expected_calls);
        assert_eq!(storage_call_counts(&counts), [1, 0, 1]);
    }

    policy.result = Ok(());
    // Retry the identical write immediately after the failed attempts, before
    // any read can change the authorization or backend state.
    // This API has no operation ID or deduplication contract: another allowed
    // call reaches the backend again rather than returning a cached success.
    roots
        .write_object(&access, &handle, &f.profile, object, payload, &mut policy)
        .unwrap();
    assert_eq!(policy.calls, 5);
    assert_eq!(storage_call_counts(&counts), [1, 0, 2]);
    assert_eq!(
        roots
            .read_object(&access, &handle, &f.profile, object, &mut policy)
            .unwrap(),
        payload
    );
    assert_eq!(policy.calls, 6);
    assert_eq!(storage_call_counts(&counts), [1, 1, 2]);
}

fn assert_canonical_identifier_roundtrip<T>(id: &T, expected: &str)
where
    T: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + std::fmt::Display + Eq,
{
    assert_eq!(id.to_string(), expected);
    let encoded = serde_json::to_string(id).unwrap();
    assert_eq!(encoded, format!("\"{expected}\""));
    let restored: T = serde_json::from_str(&encoded).unwrap();
    assert_eq!(&restored, id);
    assert_eq!(serde_json::to_string(&restored).unwrap(), encoded);
}

#[test]
fn canonical_identity_ids_roundtrip_at_the_128_byte_limit() {
    let principal_text = format!("a._:/-{}", "a".repeat(122));
    let profile_text = format!("profile_{}", "a0_-".repeat(30));
    let session_text = format!("session_{}", "a0_-".repeat(30));
    for text in [&principal_text, &profile_text, &session_text] {
        assert_eq!(text.len(), 128);
    }
    for text in ["a", principal_text.as_str()] {
        let id = PrincipalId::new(text).unwrap();
        assert_eq!(id.as_str(), text);
        assert_canonical_identifier_roundtrip(&id, text);
    }
    for text in ["profile_a", profile_text.as_str()] {
        let id = ProfileId::new(text).unwrap();
        assert_eq!(id.as_str(), text);
        assert_canonical_identifier_roundtrip(&id, text);
    }
    for text in ["session_a", session_text.as_str()] {
        let id = SessionId::new(text).unwrap();
        assert_eq!(id.as_str(), text);
        assert_canonical_identifier_roundtrip(&id, text);
    }
    let principal_overflow = format!("{principal_text}a");
    let profile_overflow = format!("{profile_text}a");
    let session_overflow = format!("{session_text}a");
    assert_eq!(
        PrincipalId::new(principal_overflow.clone()),
        Err(nagi_capability::IdentifierError::TooLong)
    );
    assert_eq!(
        ProfileId::new(profile_overflow.clone()),
        Err(nagi_identity_session::IdError::TooLong)
    );
    assert_eq!(
        SessionId::new(session_overflow.clone()),
        Err(nagi_identity_session::IdError::TooLong)
    );
    assert!(serde_json::from_value::<PrincipalId>(principal_overflow.into()).is_err());
    assert!(serde_json::from_value::<ProfileId>(profile_overflow.into()).is_err());
    assert!(serde_json::from_value::<SessionId>(session_overflow.into()).is_err());
}

#[test]
fn canonical_identity_ids_preserve_separator_distinctions() {
    let spellings = [
        "app:alpha",
        "app/alpha",
        "app.alpha",
        "app_alpha",
        "app-alpha",
    ];
    let principals: Vec<_> = spellings
        .iter()
        .map(|text| PrincipalId::new(*text).unwrap())
        .collect();
    let unique: std::collections::HashSet<_> = principals.iter().collect();
    assert_eq!(unique.len(), spellings.len());
    for (id, text) in principals.iter().zip(spellings) {
        assert_canonical_identifier_roundtrip(id, text);
    }
    assert_ne!(
        ProfileId::new("profile_a-b").unwrap(),
        ProfileId::new("profile_a_b").unwrap()
    );
    assert_ne!(
        SessionId::new("session_a-b").unwrap(),
        SessionId::new("session_a_b").unwrap()
    );
}

#[test]
fn canonical_identity_ids_reject_noncanonical_json_and_cross_namespaces() {
    for text in ["OwnerA", " app:alpha", "app:alpha ", "app:naïve"] {
        assert!(PrincipalId::new(text).is_err());
        assert!(serde_json::from_value::<PrincipalId>(text.into()).is_err());
    }
    for text in [
        "home",
        "profile_",
        "profile_A",
        "profile_a:b",
        "profile_a/b",
    ] {
        assert!(ProfileId::new(text).is_err());
        assert!(serde_json::from_value::<ProfileId>(text.into()).is_err());
    }
    for text in ["session_", "session_A", "session_a:b", "session_a/b"] {
        assert!(SessionId::new(text).is_err());
        assert!(serde_json::from_value::<SessionId>(text.into()).is_err());
    }
    let profile_json = serde_json::to_string(&ProfileId::new("profile_a").unwrap()).unwrap();
    let session_json = serde_json::to_string(&SessionId::new("session_a").unwrap()).unwrap();
    assert!(serde_json::from_str::<SessionId>(&profile_json).is_err());
    assert!(serde_json::from_str::<ProfileId>(&session_json).is_err());
    assert!(serde_json::from_value::<PrincipalId>(serde_json::json!(7)).is_err());
    assert!(serde_json::from_value::<ProfileId>(serde_json::json!(7)).is_err());
    assert!(serde_json::from_value::<SessionId>(serde_json::json!(7)).is_err());
}

#[test]
fn canonical_identity_session_codec_roundtrip_is_syntax_only() {
    let longest = format!("session_{}", "a".repeat(120));
    for text in ["session_a", longest.as_str()] {
        let identity_id = SessionId::new(text).unwrap();
        let encoded = serde_json::to_string(&identity_id).unwrap();
        let capability_id: nagi_capability::SessionId = serde_json::from_str(&encoded).unwrap();
        assert_eq!(capability_id.as_str(), text);
        assert_eq!(serde_json::to_string(&capability_id).unwrap(), encoded);
        let returned: SessionId = serde_json::from_str(&encoded).unwrap();
        assert_eq!(returned, identity_id);
    }
    // The broader capability-session syntax is not the Identity namespace.
    // Successful string decoding is no shared lifecycle or live authority.
    for text in ["Session:A", "session:external", "session_A", "profile_a"] {
        let capability_id = nagi_capability::SessionId::new(text).unwrap();
        let encoded = serde_json::to_string(&capability_id).unwrap();
        assert!(serde_json::from_str::<SessionId>(&encoded).is_err());
    }
}

// Test-only external consumer: this module imports the real library's public
// types. It neither authenticates a peer nor implements the unprovided SVC.
mod ipc_consumer_fixture {
    use nagi_identity_session::{
        CallerContextId, CurrentIdentityResolver, PrincipalId, ResolutionError, ResolvedIdentity,
        TrustedCallerContext,
    };

    // A sized, borrowed view lets an existing runtime trait object use APIs
    // taking &impl TrustedCallerContext. No principal/context is invented or
    // captured here; production provenance still belongs to that runtime.
    pub(super) struct BorrowedCaller<'a> {
        runtime: &'a dyn TrustedCallerContext,
    }

    impl<'a> BorrowedCaller<'a> {
        pub(super) fn new(runtime: &'a dyn TrustedCallerContext) -> Self {
            Self { runtime }
        }
    }

    impl TrustedCallerContext for BorrowedCaller<'_> {
        fn principal_id(&self) -> &PrincipalId {
            self.runtime.principal_id()
        }

        fn context_id(&self) -> &CallerContextId {
            self.runtime.context_id()
        }
    }

    pub(super) fn resolve<R: CurrentIdentityResolver>(
        resolver: &mut R,
        runtime: &dyn TrustedCallerContext,
    ) -> Result<ResolvedIdentity, ResolutionError> {
        let caller = BorrowedCaller::new(runtime);
        resolver.resolve_current_identity(&caller)
    }
}

#[test]
fn ipc_consumer_borrowed_caller_uses_public_recover_resolver_and_context() {
    let runtime = TestCaller::new("ipc-borrowed");
    let transport: &dyn TrustedCallerContext = &runtime;
    let caller = ipc_consumer_fixture::BorrowedCaller::new(transport);
    assert!(std::ptr::eq(
        caller.principal_id(),
        transport.principal_id()
    ));
    assert!(std::ptr::eq(caller.context_id(), transport.context_id()));

    // All providers are existing host fixtures, not production IPC providers.
    let (mut service, _) = new_service(SharedStore::default(), TestCleaner::default());
    assert_eq!(
        ipc_consumer_fixture::resolve(&mut service, transport).unwrap_err(),
        ResolutionError::Unavailable
    );
    service.recover(&caller).unwrap();
    let (user, profile) = service
        .create_local_user(&caller, metadata("IPC User").0, metadata("IPC Home").1)
        .unwrap();
    let session = service
        .login_local(
            &caller,
            user,
            profile.clone(),
            None,
            nagi_identity_session::IdempotencyKey::new("ipc-borrowed-login").unwrap(),
        )
        .unwrap();

    // The helper's temporary caller view has returned. The service still owns
    // the issuer; the returned observation is not an owned authentication token.
    let identity = ipc_consumer_fixture::resolve(&mut service, transport).unwrap();
    assert_eq!(identity.principal_id(), transport.principal_id());
    assert_eq!(identity.caller_context_id(), transport.context_id());
    assert_eq!(identity.user_id(), Some(user));
    assert_eq!(identity.profile_id(), &profile);
    assert_eq!(identity.session_id(), &session);
    assert_eq!(identity.kind(), service.snapshot().sessions[0].kind);
    assert!(identity.is_live());

    let access = IdentityAccessContext::new(&identity, &caller);
    let backend = MemoryProfileStorage::default();
    let opens = backend.opens.clone();
    let mut roots = UserStorageRootAdapter::new(backend);
    roots
        .resolve_root(&access, &profile, &mut AllowProfileRoot)
        .unwrap();
    assert_eq!(opens.load(Ordering::SeqCst), 1);

    drop(service);
    assert!(!identity.is_live());
    assert_eq!(
        roots
            .resolve_root(&access, &profile, &mut AllowProfileRoot)
            .unwrap_err(),
        StorageRootFailure::StaleSession
    );
    assert_eq!(opens.load(Ordering::SeqCst), 1);
}

#[test]
fn ipc_consumer_borrowed_caller_forwards_live_pair_without_capturing_authority() {
    for switch_principal in [true, false] {
        let before = TestCaller::new("ipc-changing");
        let mut after = before.clone();
        if switch_principal {
            after.principal = PrincipalId::new("system.other-ipc-fixture").unwrap();
        } else {
            after.context = CallerContextId::new("caller_other-ipc-fixture").unwrap();
        }
        let switched = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let runtime = SwitchingCaller {
            before,
            after,
            switched: switched.clone(),
        };
        let transport: &dyn TrustedCallerContext = &runtime;
        let caller = ipc_consumer_fixture::BorrowedCaller::new(transport);
        let (mut service, _) = new_service(SharedStore::default(), TestCleaner::default());
        service.recover(&caller).unwrap();
        let (user, profile) = service
            .create_local_user(&caller, metadata("IPC User").0, metadata("IPC Home").1)
            .unwrap();
        let session = service
            .login_local(
                &caller,
                user,
                profile.clone(),
                None,
                nagi_identity_session::IdempotencyKey::new("ipc-changing-login").unwrap(),
            )
            .unwrap();
        let identity = ipc_consumer_fixture::resolve(&mut service, transport).unwrap();
        let backend = MemoryProfileStorage::default();
        let opens = backend.opens.clone();
        let mut roots = UserStorageRootAdapter::new(backend);

        // Reuse the same borrowed view after the runtime changes either half
        // of the caller pair. Cached earlier metadata must not authorize it.
        switched.store(true, Ordering::SeqCst);
        assert_eq!(caller.principal_id(), runtime.after.principal_id());
        assert_eq!(caller.context_id(), runtime.after.context_id());
        assert_eq!(
            ipc_consumer_fixture::resolve(&mut service, transport).unwrap_err(),
            ResolutionError::NoSession
        );
        assert_eq!(
            roots
                .resolve_root(
                    &IdentityAccessContext::new(&identity, &caller),
                    &profile,
                    &mut AllowProfileRoot,
                )
                .unwrap_err(),
            StorageRootFailure::CallerMismatch
        );
        assert_eq!(opens.load(Ordering::SeqCst), 0);

        switched.store(false, Ordering::SeqCst);
        assert_eq!(
            ipc_consumer_fixture::resolve(&mut service, transport)
                .unwrap()
                .session_id(),
            &session
        );
    }
}
