use std::fs::{self, File};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use libnagi::credential::{AccountRecord, Credential};
use nagi_c1_session_runtime::{
    LinuxIdentityClock, LinuxIdentityIdSource, LinuxSessionRuntime, LocalAccountBinding,
    RuntimeError,
};
use nagi_identity_session::{
    AuthorizationFailure, AuthorizationRequest, CallerContextId, GuestCleanupFailure,
    GuestProfileCleaner, IdentityAction, IdentityAuthorizer, IdentityError, IdentityService,
    LinuxIdentitySnapshotStore, LocalUserMetadata, NoopIdentityEventSink, PrincipalId, ProfileId,
    ProfileMetadata, RecoverySource, SessionId, SessionLifecycle, TrustedCallerContext, UnixMillis,
    UserId,
};

static NEXT_DIR: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "nagi-c1-session-runtime-{}-{sequence}",
            std::process::id()
        ));
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700).create(&path).unwrap();
        Self(path)
    }

    fn open(&self) -> File {
        File::open(&self.0).unwrap()
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct TestCaller {
    principal: PrincipalId,
    context: CallerContextId,
}

impl TestCaller {
    fn service() -> Self {
        Self {
            principal: PrincipalId::new("c1.system-service").unwrap(),
            context: CallerContextId::new("caller_issuer_runtime").unwrap(),
        }
    }

    fn interactive() -> Self {
        Self {
            principal: PrincipalId::new("c1.interactive").unwrap(),
            context: CallerContextId::new("caller_user_runtime").unwrap(),
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

struct ScopedAuthorizer {
    service: PrincipalId,
    interactive: PrincipalId,
}

impl ScopedAuthorizer {
    fn for_callers(service: &TestCaller, interactive: &TestCaller) -> Self {
        Self {
            service: service.principal.clone(),
            interactive: interactive.principal.clone(),
        }
    }
}

impl IdentityAuthorizer for ScopedAuthorizer {
    fn authorize(&mut self, request: &AuthorizationRequest) -> Result<(), AuthorizationFailure> {
        let service_action = matches!(
            request.action,
            IdentityAction::CreateLocalUser
                | IdentityAction::Recover
                | IdentityAction::ValidateRecoveredState
        );
        let interactive_action = matches!(
            request.action,
            IdentityAction::LoginLocalUser
                | IdentityAction::CreateLocalSession
                | IdentityAction::ResolveCurrentIdentity
                | IdentityAction::EndSession
        );
        if (request.caller_principal == self.service && service_action)
            || (request.caller_principal == self.interactive && interactive_action)
        {
            Ok(())
        } else {
            Err(AuthorizationFailure::Denied)
        }
    }
}

#[derive(Default)]
struct NoGuestCleanup;

impl GuestProfileCleaner for NoGuestCleanup {
    fn cleanup_guest_profile(
        &mut self,
        _profile_id: &ProfileId,
    ) -> Result<(), GuestCleanupFailure> {
        Ok(())
    }
}

fn identity_service(
    directory: &TestDirectory,
    service_caller: &TestCaller,
    interactive_caller: &TestCaller,
) -> IdentityService<
    LinuxIdentitySnapshotStore,
    LinuxIdentityIdSource,
    LinuxIdentityClock,
    ScopedAuthorizer,
    NoGuestCleanup,
    NoopIdentityEventSink,
> {
    IdentityService::new(
        LinuxIdentitySnapshotStore::open(directory.open()).unwrap(),
        LinuxIdentityIdSource,
        LinuxIdentityClock::open().unwrap(),
        ScopedAuthorizer::for_callers(service_caller, interactive_caller),
        NoGuestCleanup,
        NoopIdentityEventSink,
    )
}

fn test_account() -> AccountRecord {
    let credential = Credential::derive(b"test", [0x53; 16], 10_000).unwrap();
    AccountRecord::new(b"owner", credential).unwrap()
}

fn expires_later() -> UnixMillis {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    UnixMillis::new(now + 60_000)
}

fn prepare_persistent_user(
    directory: &TestDirectory,
    service_caller: &TestCaller,
    interactive_caller: &TestCaller,
) -> (UserId, ProfileId) {
    let mut service = identity_service(directory, service_caller, interactive_caller);
    service.recover(service_caller).unwrap();
    service
        .create_local_user(
            service_caller,
            LocalUserMetadata::new("Owner").unwrap(),
            ProfileMetadata::new("Main", Some("en-US".into())).unwrap(),
        )
        .unwrap()
}

fn runtime(
    directory: &TestDirectory,
    service_caller: &TestCaller,
    interactive_caller: &TestCaller,
    account: AccountRecord,
    user_id: UserId,
    profile_id: ProfileId,
) -> LinuxSessionRuntime<ScopedAuthorizer, NoGuestCleanup, NoopIdentityEventSink> {
    LinuxSessionRuntime::start(
        directory.open(),
        vec![LocalAccountBinding::new(account, user_id, profile_id)],
        ScopedAuthorizer::for_callers(service_caller, interactive_caller),
        NoGuestCleanup,
        NoopIdentityEventSink,
        service_caller,
    )
    .unwrap()
}

fn login(
    runtime: &mut LinuxSessionRuntime<ScopedAuthorizer, NoGuestCleanup, NoopIdentityEventSink>,
    caller: &TestCaller,
) -> SessionId {
    runtime
        .authenticate_and_login(
            caller,
            b"owner",
            b"test",
            expires_later(),
            &nagi_identity_session::IdempotencyKey::new("runtime-login").unwrap(),
        )
        .unwrap()
}

fn corrupt_current_and_optional_lkg(directory: &TestDirectory, corrupt_lkg: bool) {
    let path = directory.0.join("issuer.json");
    let mut state: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    state["current"]["bytes"] = serde_json::json!([1, 2, 3]);
    if corrupt_lkg {
        state["last_known_good"]["bytes"] = serde_json::json!([4, 5, 6]);
    }
    fs::write(path, serde_json::to_vec(&state).unwrap()).unwrap();
}

#[test]
fn actual_account_record_authenticates_a_durable_c1_session_and_logout_revokes_it() {
    let directory = TestDirectory::new();
    let service_caller = TestCaller::service();
    let caller = TestCaller::interactive();
    let account = test_account();
    let (user_id, profile_id) = prepare_persistent_user(&directory, &service_caller, &caller);
    let mut runtime = runtime(
        &directory,
        &service_caller,
        &caller,
        account,
        user_id,
        profile_id,
    );

    assert_eq!(
        runtime.authenticate_and_login(
            &caller,
            b"owner",
            b"incorrect credential",
            expires_later(),
            &nagi_identity_session::IdempotencyKey::new("wrong-password").unwrap(),
        ),
        Err(RuntimeError::AuthenticationFailed)
    );
    let session_id = login(&mut runtime, &caller);
    let context = runtime.interactive_session(&caller).unwrap();
    assert_eq!(context.identity().unwrap().session_id(), &session_id);
    runtime.revalidate(&context).unwrap();

    runtime.logout_interactive(&caller).unwrap();
    assert!(runtime.revalidate(&context).is_err());
    assert!(runtime
        .snapshot()
        .sessions
        .iter()
        .any(|session| session.session_id == session_id
            && matches!(session.lifecycle, SessionLifecycle::Ended { .. })));
}

#[test]
fn restart_ends_the_saved_session_and_reauthentication_persists_user_profile_without_id_reuse() {
    let directory = TestDirectory::new();
    let service_caller = TestCaller::service();
    let caller = TestCaller::interactive();
    let account = test_account();
    let (user_id, profile_id) = prepare_persistent_user(&directory, &service_caller, &caller);
    let mut first = runtime(
        &directory,
        &service_caller,
        &caller,
        account,
        user_id,
        profile_id.clone(),
    );
    let first_session = login(&mut first, &caller);
    let stale_context = first.interactive_session(&caller).unwrap();
    assert!(stale_context.check().is_ok());

    drop(first);
    assert!(stale_context.check().is_err());

    let mut restarted = runtime(
        &directory,
        &service_caller,
        &caller,
        account,
        user_id,
        profile_id.clone(),
    );
    let saved = restarted
        .snapshot()
        .sessions
        .iter()
        .find(|session| session.session_id == first_session)
        .unwrap();
    assert!(matches!(saved.lifecycle, SessionLifecycle::Ended { .. }));
    let second_session = login(&mut restarted, &caller);
    assert_ne!(first_session, second_session);
    assert_eq!(restarted.snapshot().users.len(), 1);
    assert_eq!(restarted.snapshot().profiles.len(), 1);
    assert_eq!(restarted.snapshot().users[0].user_id, user_id);
    assert_eq!(restarted.snapshot().profiles[0].profile_id, profile_id);
    assert!(restarted
        .interactive_session(&caller)
        .unwrap()
        .check()
        .is_ok());
}

#[test]
fn corrupt_current_uses_lkg_only_for_recovery_and_requires_fresh_authentication() {
    let directory = TestDirectory::new();
    let service_caller = TestCaller::service();
    let caller = TestCaller::interactive();
    let account = test_account();
    let (user_id, profile_id) = prepare_persistent_user(&directory, &service_caller, &caller);
    let mut first = runtime(
        &directory,
        &service_caller,
        &caller,
        account,
        user_id,
        profile_id.clone(),
    );
    let old_session = login(&mut first, &caller);
    let old_context = first.interactive_session(&caller).unwrap();
    assert!(old_context.check().is_ok());
    drop(first);
    assert!(old_context.check().is_err());

    corrupt_current_and_optional_lkg(&directory, false);
    let mut recovered = runtime(
        &directory,
        &service_caller,
        &caller,
        account,
        user_id,
        profile_id,
    );
    assert_eq!(
        recovered.recovery_report().source,
        RecoverySource::LastKnownGood
    );
    assert!(!recovered
        .snapshot()
        .sessions
        .iter()
        .any(|session| session.session_id == old_session
            && matches!(session.lifecycle, SessionLifecycle::Active)));

    let new_session = login(&mut recovered, &caller);
    assert_ne!(old_session, new_session);
    assert!(recovered
        .interactive_session(&caller)
        .unwrap()
        .check()
        .is_ok());
}

#[test]
fn failed_durable_recovery_from_corrupt_current_and_lkg_issues_no_session() {
    let directory = TestDirectory::new();
    let service_caller = TestCaller::service();
    let caller = TestCaller::interactive();
    let account = test_account();
    let (user_id, profile_id) = prepare_persistent_user(&directory, &service_caller, &caller);
    let mut first = runtime(
        &directory,
        &service_caller,
        &caller,
        account,
        user_id,
        profile_id.clone(),
    );
    let old_session = login(&mut first, &caller);
    drop(first);
    corrupt_current_and_optional_lkg(&directory, true);

    let result = LinuxSessionRuntime::start(
        directory.open(),
        vec![LocalAccountBinding::new(account, user_id, profile_id)],
        ScopedAuthorizer::for_callers(&service_caller, &caller),
        NoGuestCleanup,
        NoopIdentityEventSink,
        &service_caller,
    );
    assert!(matches!(
        result,
        Err(RuntimeError::Identity(IdentityError::CorruptState))
    ));
    assert!(!old_session.as_str().is_empty());
}

#[test]
fn supplied_account_mapping_must_match_a_persisted_user_and_owned_profile() {
    let directory = TestDirectory::new();
    let service_caller = TestCaller::service();
    let caller = TestCaller::interactive();
    let mut service = identity_service(&directory, &service_caller, &caller);
    service.recover(&service_caller).unwrap();
    drop(service);
    let account = test_account();
    assert!(matches!(
        LinuxSessionRuntime::start(
            directory.open(),
            vec![LocalAccountBinding::new(
                account,
                UserId::new(12).unwrap(),
                ProfileId::new("profile_unprovisioned").unwrap(),
            )],
            ScopedAuthorizer::for_callers(&service_caller, &caller),
            NoGuestCleanup,
            NoopIdentityEventSink,
            &service_caller,
        ),
        Err(RuntimeError::InvalidAccountMapping)
    ));
}
