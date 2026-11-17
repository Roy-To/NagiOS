use super::*;
use crate::session_fixture::{
    poll_ready, Fixture, FixtureCaller, FixtureIdentityPolicy, NoGuestFixture,
};
use identity::{
    CurrentIdentityResolver, IdempotencyKey, IdentitySnapshotStore, NoopIdentityEventSink,
    ResolutionError, SessionEndReason, SessionLifecycle, UnixMillis,
};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use svc::{ContractVersion, OperationId, RequestEnvelope, RequestId, ServiceId};

type Runtime = PersistentRuntime<FixtureIdentityPolicy, NoGuestFixture, NoopIdentityEventSink>;

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!(
                "persistent-fixture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
    fn directory(&self) -> File {
        File::open(&self.0).unwrap()
    }
    fn bytes(&self) -> Vec<u8> {
        std::fs::read(self.0.join("issuer.json")).unwrap()
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn supplied(generation: u64) -> SuppliedAccounts {
    let mut accounts = AccountStore::new();
    accounts
        .add_account(
            b"persist-test",
            libnagi::security::Role::Standard,
            b"<REDACTED_TEST_CREDENTIAL>",
        )
        .unwrap();
    SuppliedAccounts {
        generation,
        accounts,
    }
}
fn initial(fixture: &Fixture) -> InitialState {
    InitialState {
        generation: 7,
        bindings: vec![BindingRow {
            account_name: b"persist-test".to_vec(),
            provider: "test-issuer".into(),
            subject: "explicit-test-account".into(),
            user: fixture.identity.user_id().unwrap(),
            profile: fixture.identity.profile_id().clone(),
        }],
        snapshot: fixture.issuer.as_ref().unwrap().snapshot().clone(),
    }
}
fn start(sandbox: &Sandbox, initial: Option<InitialState>, caller: &FixtureCaller) -> Runtime {
    Runtime::start(
        sandbox.directory(),
        supplied(7),
        initial,
        FixtureIdentityPolicy,
        NoGuestFixture,
        NoopIdentityEventSink,
        caller,
    )
    .unwrap()
}
fn login(
    runtime: &mut Runtime,
    caller: &FixtureCaller,
    key: &str,
) -> Result<SessionId, PersistentFailure> {
    let expiry = runtime.clock.checked_now().unwrap().get() + 60_000;
    runtime.login(
        caller,
        Credentials {
            name: b"persist-test",
            password: b"<REDACTED_TEST_CREDENTIAL>",
        },
        LoginIntent {
            expires_at: Some(UnixMillis::new(expiry)),
            idempotency_key: IdempotencyKey::new(key).unwrap(),
        },
    )
}
fn request() -> RequestEnvelope {
    RequestEnvelope::new(
        RequestId::new(1).unwrap(),
        None,
        None,
        ServiceId::new("fixture.echo").unwrap(),
        ContractVersion::new(1, 0).unwrap(),
        OperationId::new("echo").unwrap(),
        b"durable issuer".to_vec(),
    )
    .unwrap()
}

#[test]
fn file_restart_readback_preserves_ids_ends_old_session_and_allows_fresh_auth() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let original = initial(&fixture);
    let original_user = original.bindings[0].user;
    let original_profile = original.bindings[0].profile.clone();
    let mut runtime = start(&sandbox, Some(original), &fixture.caller);
    let first = login(&mut runtime, &fixture.caller, "first").unwrap();
    let ingress = runtime.ingress(&fixture.caller, &fixture.client).unwrap();
    assert_eq!(
        poll_ready(ingress.call(request()).unwrap())
            .unwrap()
            .result()
            .unwrap(),
        b"durable issuer"
    );
    let before: serde_json::Value = serde_json::from_slice(&sandbox.bytes()).unwrap();
    drop(runtime);
    assert!(ingress.call(request()).is_err());
    let mut restarted = start(&sandbox, None, &fixture.caller);
    let snapshot = restarted.service.as_ref().unwrap().snapshot();
    assert!(snapshot
        .users
        .iter()
        .any(|user| user.user_id == original_user));
    assert!(snapshot
        .profiles
        .iter()
        .any(|profile| profile.profile_id == original_profile));
    assert!(snapshot
        .sessions
        .iter()
        .any(|session| session.session_id == first
            && matches!(
                session.lifecycle,
                SessionLifecycle::Ended {
                    reason: SessionEndReason::SystemRestart,
                    ..
                }
            )));
    assert!(matches!(
        restarted.ingress(&fixture.caller, &fixture.client),
        Err(AdmissionFailure::Resolution(ResolutionError::NoSession))
    ));
    let after: serde_json::Value = serde_json::from_slice(&sandbox.bytes()).unwrap();
    assert_ne!(before["issuer_epoch"], after["issuer_epoch"]);
    let second = login(&mut restarted, &fixture.caller, "second").unwrap();
    assert_ne!(first, second);
    let ingress = restarted.ingress(&fixture.caller, &fixture.client).unwrap();
    assert!(poll_ready(ingress.call(request()).unwrap()).is_ok());
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 2);
}

#[test]
fn separately_started_process_recovers_file_without_resuming_saved_session() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    login(&mut runtime, &fixture.caller, "parent-login").unwrap();
    drop(runtime);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "ingress::account_binding::persistent::tests::restart_child",
            "--nocapture",
        ])
        .env("NAGI_TEST_RESTART_DIRECTORY", &sandbox.0)
        .spawn()
        .unwrap();
    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if started.elapsed() > std::time::Duration::from_secs(10) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("restart child timed out");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[test]
fn restart_child() {
    let Some(path) = std::env::var_os("NAGI_TEST_RESTART_DIRECTORY") else {
        return;
    };
    // Child only consumes the parent's explicitly supplied sandbox directory.
    let fixture = Fixture::new();
    let mut runtime = Runtime::start(
        File::open(path).unwrap(),
        supplied(7),
        None,
        FixtureIdentityPolicy,
        NoGuestFixture,
        NoopIdentityEventSink,
        &fixture.caller,
    )
    .unwrap();
    assert!(runtime
        .service
        .as_ref()
        .unwrap()
        .snapshot()
        .sessions
        .iter()
        .all(|session| !matches!(session.lifecycle, SessionLifecycle::Active)));
    assert!(matches!(
        runtime.ingress(&fixture.caller, &fixture.client),
        Err(AdmissionFailure::Resolution(ResolutionError::NoSession))
    ));
    login(&mut runtime, &fixture.caller, "child-fresh-login").unwrap();
    assert!(poll_ready(
        runtime
            .ingress(&fixture.caller, &fixture.client)
            .unwrap()
            .call(request())
            .unwrap()
    )
    .is_ok());
}

#[test]
fn missing_corrupt_future_or_generation_mismatch_never_bootstraps_implicitly() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    assert!(matches!(
        store::FileSnapshotStore::open(sandbox.directory(), 7, None),
        Err(PersistentFailure::Unavailable)
    ));
    drop(start(&sandbox, Some(initial(&fixture)), &fixture.caller));
    assert!(matches!(
        store::FileSnapshotStore::open(sandbox.directory(), 8, None),
        Err(PersistentFailure::GenerationMismatch)
    ));
    let bytes = sandbox.bytes();
    for replacement in [
        b"{truncated".to_vec(),
        {
            let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            value["schema"] = 2.into();
            serde_json::to_vec(&value).unwrap()
        },
        {
            let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            value["bindings"][0]["user"] = 999.into();
            serde_json::to_vec(&value).unwrap()
        },
    ] {
        std::fs::write(sandbox.0.join("issuer.json"), replacement).unwrap();
        assert!(matches!(
            store::FileSnapshotStore::open(sandbox.directory(), 7, None),
            Err(PersistentFailure::Corrupt)
        ));
    }
}

#[test]
fn interrupted_write_preserves_previous_file_and_latches_failure_until_reopen() {
    for fault in [store::Fault::PartialWrite, store::Fault::BeforeRename] {
        let sandbox = Sandbox::new();
        let fixture = Fixture::new();
        let mut store =
            store::FileSnapshotStore::open(sandbox.directory(), 7, Some(initial(&fixture)))
                .unwrap();
        let before = sandbox.bytes();
        let saved = store.load_current().unwrap().unwrap();
        let mut next = IdentitySnapshot::from_json(&saved.bytes).unwrap();
        next.revision += 1;
        *store.fault.lock().unwrap() = Some(fault);
        assert_eq!(
            store.commit_atomic(Some(saved.version), &next.to_json().unwrap()),
            Err(identity::SnapshotStoreError::Unavailable)
        );
        assert_eq!(sandbox.bytes(), before);
        assert!(store.load_current().is_err());
        assert!(store
            .commit_atomic(Some(saved.version), &next.to_json().unwrap())
            .is_err());
        drop(store);
        let reopened = store::FileSnapshotStore::open(sandbox.directory(), 7, None).unwrap();
        assert_eq!(reopened.load_current().unwrap().unwrap().bytes, saved.bytes);
    }
}

#[test]
fn unknown_commit_is_error_stops_issuer_and_restart_ends_possibly_committed_session() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    *runtime.fault.lock().unwrap() = Some(store::Fault::AfterRename);
    assert_eq!(
        login(&mut runtime, &fixture.caller, "unknown"),
        Err(PersistentFailure::Login(BoundLoginFailure::Identity(
            IdentityError::StoreUnavailable
        )))
    );
    assert!(runtime.service.is_none());
    let persisted: serde_json::Value = serde_json::from_slice(&sandbox.bytes()).unwrap();
    assert_eq!(
        persisted["current"]["snapshot"]["sessions"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    drop(runtime);
    let mut restarted = start(&sandbox, None, &fixture.caller);
    assert!(restarted
        .service
        .as_ref()
        .unwrap()
        .snapshot()
        .sessions
        .iter()
        .all(|session| !matches!(session.lifecycle, SessionLifecycle::Active)));
    login(&mut restarted, &fixture.caller, "fresh-after-unknown").unwrap();
}

#[test]
fn successful_commit_keeps_last_known_good_and_exclusive_writer_rejects_second_open() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut store =
        store::FileSnapshotStore::open(sandbox.directory(), 7, Some(initial(&fixture))).unwrap();
    assert!(store::FileSnapshotStore::open(sandbox.directory(), 7, None).is_err());
    let old = store.load_current().unwrap().unwrap();
    let mut next = IdentitySnapshot::from_json(&old.bytes).unwrap();
    next.revision += 1;
    let version = store
        .commit_atomic(Some(old.version), &next.to_json().unwrap())
        .unwrap();
    assert!(version.get() > old.version.get());
    assert_eq!(store.load_last_known_good().unwrap(), Some(old.clone()));
    assert_eq!(
        store.commit_atomic(Some(old.version), &old.bytes),
        Err(identity::SnapshotStoreError::Conflict)
    );
}

#[test]
fn real_auth_denial_logout_relogin_and_runtime_drop_remain_fail_closed() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    let expiry = Some(UnixMillis::new(
        runtime.clock.checked_now().unwrap().get() + 60_000,
    ));
    let result = runtime.login(
        &fixture.caller,
        Credentials {
            name: b"persist-test",
            password: b"wrong",
        },
        LoginIntent {
            expires_at: expiry,
            idempotency_key: IdempotencyKey::new("wrong").unwrap(),
        },
    );
    assert_eq!(
        result,
        Err(PersistentFailure::Login(BoundLoginFailure::Credential(
            libnagi::security::LoginError::WrongPassword
        )))
    );
    assert!(runtime
        .login(
            &fixture.caller,
            Credentials {
                name: b"persist-test",
                password: b"<REDACTED_TEST_CREDENTIAL>"
            },
            LoginIntent {
                expires_at: None,
                idempotency_key: IdempotencyKey::new("unbounded").unwrap()
            }
        )
        .is_err());
    let session = login(&mut runtime, &fixture.caller, "valid").unwrap();
    let ingress = runtime.ingress(&fixture.caller, &fixture.client).unwrap();
    runtime.end_session(&fixture.caller, &session).unwrap();
    assert!(ingress.call(request()).is_err());
    login(&mut runtime, &fixture.caller, "relogin").unwrap();
    let next = runtime.ingress(&fixture.caller, &fixture.client).unwrap();
    next.revoke();
    assert!(next.call(request()).is_err());
    drop(runtime);
    assert!(next.call(request()).is_err());
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn external_mutation_or_future_clock_floor_is_rejected_without_issuance() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    login(&mut runtime, &fixture.caller, "before-mutation").unwrap();
    let ingress = runtime.ingress(&fixture.caller, &fixture.client).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&sandbox.bytes()).unwrap();
    value["generation"] = 8.into();
    std::fs::write(
        sandbox.0.join("issuer.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let sid = ingress.binding.identity().unwrap().session_id().clone();
    assert_eq!(
        runtime.end_session(&fixture.caller, &sid),
        Err(PersistentFailure::Identity(IdentityError::StoreConflict))
    );
    assert!(ingress.binding.identity().is_err());
    drop(runtime);
    value["generation"] = 7.into();
    value["floor_millis"] = u64::MAX.into();
    std::fs::write(
        sandbox.0.join("issuer.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        store::FileSnapshotStore::open(sandbox.directory(), 7, None),
        Err(PersistentFailure::ClockUnavailable)
    ));
}

#[test]
fn wrong_caller_cannot_recover_or_issue_from_persisted_ids() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let caller = FixtureCaller::new("fixture.other", "caller_other");
    assert!(matches!(
        Runtime::start(
            sandbox.directory(),
            supplied(7),
            Some(initial(&fixture)),
            FixtureIdentityPolicy,
            NoGuestFixture,
            NoopIdentityEventSink,
            &caller
        ),
        Err(PersistentFailure::Identity(
            IdentityError::AuthorizationDenied
        ))
    ));
    let mut runtime = start(&sandbox, None, &fixture.caller);
    assert_eq!(
        login(&mut runtime, &caller, "wrong-caller"),
        Err(PersistentFailure::Login(BoundLoginFailure::Identity(
            IdentityError::AuthorizationDenied
        )))
    );
    assert_eq!(
        runtime
            .service
            .as_mut()
            .unwrap()
            .resolve_current_identity(&caller)
            .err(),
        Some(ResolutionError::Denied)
    );
}

#[test]
fn cancelled_runtime_refuses_fresh_credential_issuance_without_writing() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    let before = sandbox.bytes();
    runtime.cancellation.cancel();
    assert_eq!(
        login(&mut runtime, &fixture.caller, "cancelled-login"),
        Err(PersistentFailure::Stopped)
    );
    assert_eq!(sandbox.bytes(), before);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn clock_failure_expires_existing_evidence_and_blocks_new_issuance() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    login(&mut runtime, &fixture.caller, "before-clock-failure").unwrap();
    let ingress = runtime.ingress(&fixture.caller, &fixture.client).unwrap();
    let before = sandbox.bytes();
    runtime.clock.fail_for_test();
    assert_eq!(
        ingress.binding.check(),
        Err(AdmissionFailure::StaleIdentity)
    );
    assert_eq!(
        runtime.authenticate_and_login(
            &fixture.caller,
            b"persist-test",
            b"<REDACTED_TEST_CREDENTIAL>",
            UnixMillis::new(u64::MAX - 1),
            IdempotencyKey::new("clock-failed").unwrap()
        ),
        Err(PersistentFailure::ClockUnavailable)
    );
    assert_eq!(sandbox.bytes(), before);
    assert!(runtime.service.is_none());
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn private_directory_and_no_symlink_state_are_required() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    std::fs::set_permissions(&sandbox.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        store::FileSnapshotStore::open(sandbox.directory(), 7, Some(initial(&fixture))).is_err()
    );
    std::fs::set_permissions(&sandbox.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    let other = sandbox.0.join("other");
    std::fs::write(&other, b"untouched").unwrap();
    std::os::unix::fs::symlink(&other, sandbox.0.join("issuer.json")).unwrap();
    assert!(
        store::FileSnapshotStore::open(sandbox.directory(), 7, Some(initial(&fixture))).is_err()
    );
    assert_eq!(std::fs::read(&other).unwrap(), b"untouched");
}

#[test]
fn borrowed_dyn_caller_change_in_login_authorizer_is_rejected_before_commit() {
    struct SwitchingPolicy(std::sync::Arc<std::sync::atomic::AtomicBool>);
    impl identity::IdentityAuthorizer for SwitchingPolicy {
        fn authorize(
            &mut self,
            request: &identity::AuthorizationRequest,
        ) -> Result<(), identity::AuthorizationFailure> {
            if request.action == identity::IdentityAction::LoginLocalUser {
                self.0.store(true, Ordering::SeqCst);
                Ok(())
            } else {
                FixtureIdentityPolicy.authorize(request)
            }
        }
    }
    for principal in [false, true] {
        let sandbox = Sandbox::new();
        let mut fixture = Fixture::new();
        let switch = fixture.caller.arm_change(principal);
        let caller: &dyn TrustedCallerContext = &fixture.caller;
        let mut runtime = PersistentRuntime::start(
            sandbox.directory(),
            supplied(7),
            Some(initial(&fixture)),
            SwitchingPolicy(switch),
            NoGuestFixture,
            NoopIdentityEventSink,
            caller,
        )
        .unwrap();
        let before = sandbox.bytes();
        let expiry = UnixMillis::new(runtime.clock.checked_now().unwrap().get() + 60_000);
        assert_eq!(
            runtime.authenticate_and_login(
                caller,
                b"persist-test",
                b"<REDACTED_TEST_CREDENTIAL>",
                expiry,
                IdempotencyKey::new("switching-login").unwrap()
            ),
            Err(PersistentFailure::Login(BoundLoginFailure::Identity(
                IdentityError::AuthorizationDenied
            )))
        );
        assert_eq!(sandbox.bytes(), before);
        assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn ws02_logout_receipt_replay_and_relogin_do_not_revive_old_context_or_call() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    let expiry = UnixMillis::new(runtime.clock.checked_now().unwrap().get() + 60_000);
    let authenticate = |runtime: &mut Runtime| {
        runtime.authenticate_and_login(
            &fixture.caller,
            b"persist-test",
            b"<REDACTED_TEST_CREDENTIAL>",
            expiry,
            IdempotencyKey::new("ws02-logout-receipt").unwrap(),
        )
    };
    let first = authenticate(&mut runtime).unwrap();
    let old = runtime.interactive_session(&fixture.caller).unwrap();
    let ingress = runtime
        .bind_service(&fixture.caller, &fixture.client)
        .unwrap();
    let unpolled = ingress.call(request()).unwrap();
    runtime.end_session(&fixture.caller, &first).unwrap();
    let saved = sandbox.bytes();
    assert_eq!(old.check(), Err(AdmissionFailure::StaleIdentity));
    // The existing SDK can replay an idempotent SessionId receipt. That ID is
    // deliberately not accepted by any interactive-context constructor.
    assert_eq!(authenticate(&mut runtime).unwrap(), first);
    assert_eq!(sandbox.bytes(), saved);
    assert!(matches!(
        runtime.interactive_session(&fixture.caller),
        Err(AdmissionFailure::Resolution(ResolutionError::Ended))
    ));
    let second = login(&mut runtime, &fixture.caller, "ws02-fresh-after-logout").unwrap();
    assert_ne!(first, second);
    let current = runtime.interactive_session(&fixture.caller).unwrap();
    assert_eq!(current.identity().unwrap().session_id(), &second);
    assert_eq!(old.check(), Err(AdmissionFailure::StaleIdentity));
    assert!(old.access_context().is_err());
    assert!(matches!(
        poll_ready(unpolled),
        Err(AdmissionFailure::StaleIdentity)
    ));
    assert_eq!(fixture.policy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
    let current_ingress = runtime
        .bind_service(&fixture.caller, &fixture.client)
        .unwrap();
    assert!(poll_ready(current_ingress.call(request()).unwrap()).is_ok());
    assert_eq!(fixture.policy_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);
    assert_eq!(old.check(), Err(AdmissionFailure::StaleIdentity));
}

#[test]
fn ws02_real_expiry_boundary_and_relogin_keep_old_authority_revoked() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    let expires = UnixMillis::new(runtime.clock.checked_now().unwrap().get() + 500);
    let first = runtime
        .authenticate_and_login(
            &fixture.caller,
            b"persist-test",
            b"<REDACTED_TEST_CREDENTIAL>",
            expires,
            IdempotencyKey::new("ws02-expiring").unwrap(),
        )
        .unwrap();
    let old = runtime.interactive_session(&fixture.caller).unwrap();
    let ingress = runtime
        .bind_service(&fixture.caller, &fixture.client)
        .unwrap();
    let unpolled = ingress.call(request()).unwrap();
    assert!(old.check().is_ok());
    let limit = std::time::Instant::now();
    while runtime.clock.checked_now().unwrap() < expires {
        assert!(limit.elapsed() < std::time::Duration::from_secs(3));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(old.check(), Err(AdmissionFailure::StaleIdentity));
    assert!(matches!(
        runtime.interactive_session(&fixture.caller),
        Err(AdmissionFailure::Resolution(ResolutionError::Expired))
    ));
    assert!(runtime
        .service
        .as_ref()
        .unwrap()
        .snapshot()
        .sessions
        .iter()
        .any(|session| session.session_id == first
            && matches!(
                session.lifecycle,
                SessionLifecycle::Ended {
                    reason: SessionEndReason::Expired,
                    ..
                }
            )));
    let second = login(&mut runtime, &fixture.caller, "ws02-fresh-after-expiry").unwrap();
    assert_ne!(first, second);
    let current = runtime.interactive_session(&fixture.caller).unwrap();
    assert!(current.check().is_ok());
    assert_eq!(old.check(), Err(AdmissionFailure::StaleIdentity));
    assert!(matches!(
        poll_ready(unpolled),
        Err(AdmissionFailure::StaleIdentity)
    ));
    assert_eq!(fixture.policy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn ws02_authorized_recovery_and_relogin_never_restore_prior_context() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    let first = login(&mut runtime, &fixture.caller, "ws02-recovery-login").unwrap();
    let old = runtime.interactive_session(&fixture.caller).unwrap();
    let user = old.identity().unwrap().user_id();
    let profile = old.identity().unwrap().profile_id().clone();
    let ingress = runtime
        .bind_service(&fixture.caller, &fixture.client)
        .unwrap();
    let unpolled = ingress.call(request()).unwrap();
    assert_eq!(
        runtime
            .recover(&fixture.caller)
            .unwrap()
            .interrupted_sessions_ended,
        1
    );
    assert_eq!(old.check(), Err(AdmissionFailure::StaleIdentity));
    // In-process recovery retains the SDK's diagnostic last-session entry.
    // A new process has NoSession; this same issuer correctly reports Ended.
    assert_eq!(
        runtime.interactive_session(&fixture.caller).err(),
        Some(AdmissionFailure::Resolution(ResolutionError::Ended))
    );
    assert_eq!(
        runtime
            .recover(&fixture.caller)
            .unwrap()
            .interrupted_sessions_ended,
        0
    );
    let second = login(&mut runtime, &fixture.caller, "ws02-recovery-login").unwrap();
    assert_ne!(first, second);
    let current = runtime.interactive_session(&fixture.caller).unwrap();
    assert_eq!(current.identity().unwrap().user_id(), user);
    assert_eq!(current.identity().unwrap().profile_id(), &profile);
    assert_eq!(
        runtime.end_session(&fixture.caller, &first),
        Err(PersistentFailure::Identity(
            IdentityError::SessionNotCurrent
        ))
    );
    assert!(current.check().is_ok());
    assert_eq!(old.check(), Err(AdmissionFailure::StaleIdentity));
    assert!(matches!(
        poll_ready(unpolled),
        Err(AdmissionFailure::StaleIdentity)
    ));
    assert_eq!(fixture.policy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn ws02_recovery_commit_failure_stops_old_context_before_file_restart() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    let first = login(
        &mut runtime,
        &fixture.caller,
        "ws02-before-recovery-failure",
    )
    .unwrap();
    let old = runtime.interactive_session(&fixture.caller).unwrap();
    let ingress = runtime
        .bind_service(&fixture.caller, &fixture.client)
        .unwrap();
    let unpolled = ingress.call(request()).unwrap();
    let saved = sandbox.bytes();
    *runtime.fault.lock().unwrap() = Some(store::Fault::BeforeRename);
    assert!(matches!(
        runtime.recover(&fixture.caller),
        Err(PersistentFailure::Identity(IdentityError::StoreUnavailable))
    ));
    assert_eq!(sandbox.bytes(), saved);
    assert!(runtime.service.is_none());
    assert_eq!(old.check(), Err(AdmissionFailure::Cancelled));
    drop(runtime);
    let mut restarted = start(&sandbox, None, &fixture.caller);
    assert!(matches!(
        restarted.interactive_session(&fixture.caller),
        Err(AdmissionFailure::Resolution(ResolutionError::NoSession))
    ));
    let second = login(
        &mut restarted,
        &fixture.caller,
        "ws02-restart-after-failure",
    )
    .unwrap();
    assert_ne!(first, second);
    assert!(restarted
        .interactive_session(&fixture.caller)
        .unwrap()
        .check()
        .is_ok());
    assert_eq!(old.check(), Err(AdmissionFailure::Cancelled));
    assert!(matches!(
        poll_ready(unpolled),
        Err(AdmissionFailure::Cancelled)
    ));
    assert_eq!(fixture.policy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn ws02_wrong_principal_or_endpoint_context_cannot_recover_or_reuse_session() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    let session = login(&mut runtime, &fixture.caller, "ws02-owner").unwrap();
    let context = runtime.interactive_session(&fixture.caller).unwrap();
    let saved = sandbox.bytes();
    for (principal, endpoint) in [
        ("fixture.other", "caller_fixture"),
        ("fixture.echo-client", "caller_other"),
    ] {
        // Scoped negative peer fixture, never a production OS-credential claim.
        let other = FixtureCaller::new(principal, endpoint);
        assert!(matches!(
            runtime.interactive_session(&other),
            Err(AdmissionFailure::Resolution(ResolutionError::Denied))
        ));
        assert!(matches!(
            runtime.recover(&other),
            Err(PersistentFailure::Identity(
                IdentityError::AuthorizationDenied
            ))
        ));
        assert_eq!(
            runtime.end_session(&other, &session),
            Err(PersistentFailure::Identity(
                IdentityError::AuthorizationDenied
            ))
        );
        assert_eq!(
            runtime.lock_interactive_session(&other),
            Err(PersistentFailure::Resolution(ResolutionError::Denied))
        );
        assert_eq!(sandbox.bytes(), saved);
        assert!(context.check().is_ok());
    }
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn ws02_lock_revokes_current_context_and_requires_new_authenticated_session() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    let first = login(&mut runtime, &fixture.caller, "ws02-before-lock").unwrap();
    let old = runtime.interactive_session(&fixture.caller).unwrap();
    let user = old.identity().unwrap().user_id();
    let profile = old.identity().unwrap().profile_id().clone();
    let ingress = runtime
        .bind_service(&fixture.caller, &fixture.client)
        .unwrap();
    let unpolled = ingress.call(request()).unwrap();
    runtime.lock_interactive_session(&fixture.caller).unwrap();
    assert_eq!(old.check(), Err(AdmissionFailure::StaleIdentity));
    assert_eq!(
        runtime.interactive_session(&fixture.caller).err(),
        Some(AdmissionFailure::Resolution(ResolutionError::Ended))
    );
    assert!(matches!(
        poll_ready(unpolled),
        Err(AdmissionFailure::StaleIdentity)
    ));
    assert_eq!(fixture.policy_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
    drop(runtime);
    let mut restarted = start(&sandbox, None, &fixture.caller);
    assert_eq!(
        restarted.interactive_session(&fixture.caller).err(),
        Some(AdmissionFailure::Resolution(ResolutionError::NoSession))
    );
    let second = login(&mut restarted, &fixture.caller, "ws02-unlock-fresh-auth").unwrap();
    assert_ne!(first, second);
    let current = restarted.interactive_session(&fixture.caller).unwrap();
    assert_eq!(current.identity().unwrap().user_id(), user);
    assert_eq!(current.identity().unwrap().profile_id(), &profile);
    assert_eq!(old.check(), Err(AdmissionFailure::Cancelled));
    assert!(current.check().is_ok());
}

#[test]
fn ws02_corrupt_current_and_active_lkg_cannot_reconstruct_old_authority() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    let first = login(&mut runtime, &fixture.caller, "ws02-before-corrupt").unwrap();
    let old = runtime.interactive_session(&fixture.caller).unwrap();
    // An explicit unrelated test record creates a complete LKG containing the
    // still-active Session. This is real SDK creation, never production setup.
    runtime
        .service
        .as_mut()
        .unwrap()
        .create_local_user(
            &fixture.caller,
            identity::LocalUserMetadata::new("WS02 unrelated test user").unwrap(),
            identity::ProfileMetadata::new("WS02 unrelated test profile", None).unwrap(),
        )
        .unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&sandbox.bytes()).unwrap();
    assert!(value["previous"]["snapshot"]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .any(
            |session| session["session_id"].as_str() == Some(first.as_str())
                && session["lifecycle"]["state"] == "active"
        ));
    value["current"]["snapshot"]["schema_version"] = 999.into();
    std::fs::write(
        sandbox.0.join("issuer.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        runtime.recover(&fixture.caller),
        Err(PersistentFailure::Identity(IdentityError::StoreConflict))
    ));
    assert_eq!(old.check(), Err(AdmissionFailure::Cancelled));
    drop(runtime);
    assert!(matches!(
        Runtime::start(
            sandbox.directory(),
            supplied(7),
            None,
            FixtureIdentityPolicy,
            NoGuestFixture,
            NoopIdentityEventSink,
            &fixture.caller
        ),
        Err(PersistentFailure::Corrupt)
    ));
    assert_eq!(old.check(), Err(AdmissionFailure::Cancelled));
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn shared_entry_borrowed_comes_from_real_authentication_and_canonical_sdk_lease() {
    fn shared<T: Send + Sync>() {}
    shared::<shared::SharedObservation<'static>>();
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    let session = runtime
        .authenticate_and_login(
            &fixture.caller,
            b"persist-test",
            b"<REDACTED_TEST_CREDENTIAL>",
            UnixMillis::new(runtime.clock.checked_now().unwrap().get() + 60_000),
            IdempotencyKey::new("shared-public-auth").unwrap(),
        )
        .unwrap();
    let ordinary = runtime.interactive_session(&fixture.caller).unwrap();
    let observed = runtime.shared_interactive_session(&fixture.caller).unwrap();
    assert_eq!(observed.session_id(), &session);
    assert_eq!(
        observed.profile_id(),
        ordinary.identity().unwrap().profile_id()
    );
    let expected = ordinary.identity().unwrap().try_live_snapshot().unwrap();
    assert_eq!(observed.check(), Ok(expected));
    std::thread::scope(|scope| {
        let observed = &observed;
        scope
            .spawn(move || assert_eq!(observed.check(), Ok(expected)))
            .join()
            .unwrap();
    });
}

#[test]
fn shared_entry_owned_retains_actual_caller_but_issuer_drop_cancels_it() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let initial = initial(&fixture);
    let caller = std::sync::Arc::new(fixture.caller);
    let mut runtime = start(&sandbox, Some(initial), caller.as_ref());
    login(&mut runtime, caller.as_ref(), "shared-owned").unwrap();
    let weak = std::sync::Arc::downgrade(&caller);
    let observed: shared::SharedObservation<'static> = runtime
        .shared_interactive_session_owned(caller.clone())
        .unwrap();
    drop(caller);
    let observed = std::thread::spawn(move || {
        assert_eq!(
            observed.check().unwrap().status(),
            identity::LiveStatus::Live
        );
        observed
    })
    .join()
    .unwrap();
    assert!(weak.upgrade().is_some());
    drop(runtime);
    assert_eq!(observed.check(), Err(shared::SharedCheckFailure::Cancelled));
    drop(observed);
    assert!(weak.upgrade().is_none());
}

#[test]
fn shared_entry_uses_original_ingress_cancellation_for_every_issued_view() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    login(&mut runtime, &fixture.caller, "shared-token").unwrap();
    let ordinary = runtime.interactive_session(&fixture.caller).unwrap();
    let a = runtime.shared_interactive_session(&fixture.caller).unwrap();
    let b = runtime.shared_interactive_session(&fixture.caller).unwrap();
    let ingress = runtime
        .bind_service(&fixture.caller, &fixture.client)
        .unwrap();
    ingress.revoke();
    assert_eq!(a.check(), Err(shared::SharedCheckFailure::Cancelled));
    assert_eq!(b.check(), Err(shared::SharedCheckFailure::Cancelled));
    assert_eq!(ordinary.check(), Err(AdmissionFailure::Cancelled));
    assert!(matches!(
        runtime.shared_interactive_session(&fixture.caller),
        Err(SharedIssueFailure::Resolution(ResolutionError::Unavailable))
    ));
}

#[test]
fn shared_entry_lock_logout_recovery_do_not_rebind_old_views_after_fresh_login() {
    for kind in 0..3 {
        let sandbox = Sandbox::new();
        let fixture = Fixture::new();
        let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
        let first = login(&mut runtime, &fixture.caller, "shared-before-end").unwrap();
        let ordinary = runtime.interactive_session(&fixture.caller).unwrap();
        let old = runtime.shared_interactive_session(&fixture.caller).unwrap();
        let epoch = old.check().unwrap().generation();
        match kind {
            0 => runtime.lock_interactive_session(&fixture.caller).unwrap(),
            1 => runtime.end_session(&fixture.caller, &first).unwrap(),
            _ => {
                runtime.recover(&fixture.caller).unwrap();
            }
        }
        assert_eq!(old.check().unwrap().status(), identity::LiveStatus::Revoked);
        assert_eq!(ordinary.check(), Err(AdmissionFailure::StaleIdentity));
        let second = login(&mut runtime, &fixture.caller, "shared-after-end").unwrap();
        assert_ne!(first, second);
        let fresh = runtime.shared_interactive_session(&fixture.caller).unwrap();
        assert_eq!(fresh.check().unwrap().status(), identity::LiveStatus::Live);
        assert!(fresh.check().unwrap().generation() > epoch);
        assert_eq!(old.check().unwrap().status(), identity::LiveStatus::Revoked);
        assert_eq!(old.check().unwrap().generation(), epoch);
    }
}

#[test]
fn shared_entry_expiry_is_driven_by_the_actual_persistent_issuer_clock() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    let expires = UnixMillis::new(runtime.clock.checked_now().unwrap().get() + 500);
    runtime
        .authenticate_and_login(
            &fixture.caller,
            b"persist-test",
            b"<REDACTED_TEST_CREDENTIAL>",
            expires,
            IdempotencyKey::new("shared-expiry").unwrap(),
        )
        .unwrap();
    let old = runtime.shared_interactive_session(&fixture.caller).unwrap();
    let epoch = old.check().unwrap().generation();
    let limit = std::time::Instant::now();
    while runtime.clock.checked_now().unwrap() < expires {
        assert!(limit.elapsed() < std::time::Duration::from_secs(3));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(old.check().unwrap().status(), identity::LiveStatus::Expired);
    assert_eq!(old.check().unwrap().generation(), epoch);
}

#[test]
fn shared_entry_clock_contention_returns_busy_from_the_same_clock_without_waiting() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    login(&mut runtime, &fixture.caller, "shared-busy").unwrap();
    let observed = runtime.shared_interactive_session(&fixture.caller).unwrap();
    runtime.clock.while_locked_for_test(|| {
        assert_eq!(
            observed.check(),
            Err(shared::SharedCheckFailure::Unavailable(
                identity::TryLiveError::Clock(identity::TryClockError::Busy)
            ))
        );
    });
    assert_eq!(
        observed.check().unwrap().status(),
        identity::LiveStatus::Live
    );
}

#[test]
fn shared_entry_clock_failure_is_unavailable_then_stops_future_issuance() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    login(&mut runtime, &fixture.caller, "shared-clock-failure").unwrap();
    let observed = runtime.shared_interactive_session(&fixture.caller).unwrap();
    runtime.clock.fail_for_test();
    assert_eq!(
        observed.check(),
        Err(shared::SharedCheckFailure::Unavailable(
            identity::TryLiveError::Clock(identity::TryClockError::Unavailable)
        ))
    );
    assert!(matches!(
        runtime.shared_interactive_session(&fixture.caller),
        Err(SharedIssueFailure::Resolution(ResolutionError::Unavailable))
    ));
    assert_eq!(observed.check(), Err(shared::SharedCheckFailure::Cancelled));
}

#[test]
fn shared_entry_file_restart_keeps_old_view_cancelled_and_requires_fresh_auth() {
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mut runtime = start(&sandbox, Some(initial(&fixture)), &fixture.caller);
    let first = login(&mut runtime, &fixture.caller, "shared-pre-restart").unwrap();
    let old = runtime.shared_interactive_session(&fixture.caller).unwrap();
    let epoch = old.check().unwrap().generation();
    drop(runtime);
    let mut restarted = start(&sandbox, None, &fixture.caller);
    assert!(matches!(
        restarted.shared_interactive_session(&fixture.caller),
        Err(SharedIssueFailure::Resolution(ResolutionError::NoSession))
    ));
    let second = login(&mut restarted, &fixture.caller, "shared-post-restart").unwrap();
    let fresh = restarted
        .shared_interactive_session(&fixture.caller)
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(old.check(), Err(shared::SharedCheckFailure::Cancelled));
    assert_eq!(fresh.check().unwrap().status(), identity::LiveStatus::Live);
    assert!(fresh.check().unwrap().generation() > epoch);
    assert_eq!(old.profile_id(), fresh.profile_id());
}

#[test]
fn shared_entry_denied_and_unavailable_resolution_never_publish_a_view() {
    struct Gate(std::sync::Arc<std::sync::atomic::AtomicU8>);
    impl IdentityAuthorizer for Gate {
        fn authorize(
            &mut self,
            request: &identity::AuthorizationRequest,
        ) -> Result<(), identity::AuthorizationFailure> {
            if request.action == identity::IdentityAction::ResolveCurrentIdentity {
                match self.0.load(Ordering::SeqCst) {
                    1 => return Err(identity::AuthorizationFailure::Denied),
                    2 => return Err(identity::AuthorizationFailure::Unavailable),
                    _ => {}
                }
            }
            FixtureIdentityPolicy.authorize(request)
        }
    }
    let sandbox = Sandbox::new();
    let fixture = Fixture::new();
    let mode = std::sync::Arc::new(std::sync::atomic::AtomicU8::new(0));
    let mut runtime = PersistentRuntime::start(
        sandbox.directory(),
        supplied(7),
        Some(initial(&fixture)),
        Gate(mode.clone()),
        NoGuestFixture,
        NoopIdentityEventSink,
        &fixture.caller,
    )
    .unwrap();
    let expires = UnixMillis::new(runtime.clock.checked_now().unwrap().get() + 60_000);
    runtime
        .authenticate_and_login(
            &fixture.caller,
            b"persist-test",
            b"<REDACTED_TEST_CREDENTIAL>",
            expires,
            IdempotencyKey::new("shared-policy").unwrap(),
        )
        .unwrap();
    for (blocked, expected) in [
        (1, ResolutionError::Denied),
        (2, ResolutionError::Unavailable),
    ] {
        mode.store(blocked, Ordering::SeqCst);
        assert!(
            matches!(runtime.shared_interactive_session(&fixture.caller),
            Err(SharedIssueFailure::Resolution(actual)) if actual == expected)
        );
    }
}
