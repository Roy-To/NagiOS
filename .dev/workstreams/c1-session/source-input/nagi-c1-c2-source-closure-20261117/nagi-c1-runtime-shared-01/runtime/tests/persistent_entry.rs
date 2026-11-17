#![cfg(target_os = "linux")]
#![forbid(unsafe_code)]

#[path = "../../consumer/src/fixture.rs"]
#[allow(dead_code)]
mod fixture;

use fixture::{poll_ready, Fixture, FixtureIdentityPolicy, NoGuestFixture};
use identity::{
    CurrentIdentityResolver, IdempotencyKey, NoopIdentityEventSink, ProviderSubjectRef, UnixMillis,
};
use libnagi::security::{AccountStore, Role};
use nagi_init_session_supply_candidate::{
    ConfiguredBinding, InitialState, PersistentRuntime, SuppliedAccounts,
};
use std::fs::File;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU64, Ordering};
use svc::{ContractVersion, OperationId, RequestEnvelope, RequestId, ServiceId};

#[test]
fn external_consumer_uses_only_public_candidate_and_real_sdk_api() {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!(
            "public-runtime-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
    std::fs::create_dir(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let fixture = Fixture::new();
    let caller: &dyn identity::TrustedCallerContext = &fixture.caller;
    let mut accounts = AccountStore::new();
    accounts
        .add_account(b"public-test", Role::Standard, b"<REDACTED_TEST_CREDENTIAL>")
        .unwrap();
    let binding = ConfiguredBinding::new(
        b"public-test".to_vec(),
        ProviderSubjectRef::new("test-issuer", "explicit-public-test").unwrap(),
        fixture.identity.user_id().unwrap(),
        fixture.identity.profile_id().clone(),
    )
    .unwrap();
    let initial = InitialState::new(
        1,
        vec![binding],
        fixture.issuer.as_ref().unwrap().snapshot().clone(),
    );
    let mut runtime = PersistentRuntime::start(
        File::open(&path).unwrap(),
        SuppliedAccounts::new(1, accounts).unwrap(),
        Some(initial),
        FixtureIdentityPolicy,
        NoGuestFixture,
        NoopIdentityEventSink,
        caller,
    )
    .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let session = runtime
        .authenticate_and_login(
            caller,
            b"public-test",
            b"<REDACTED_TEST_CREDENTIAL>",
            UnixMillis::new(now + 60_000),
            IdempotencyKey::new("public-login").unwrap(),
        )
        .unwrap();
    let context = runtime.interactive_session(caller).unwrap();
    assert_eq!(context.identity().unwrap().session_id(), &session);
    assert!(context.access_context().is_ok());
    let observation = runtime.resolve_current_identity(&fixture.caller).unwrap();
    assert_eq!(observation.session_id(), &session);
    let ingress = runtime.bind_service(caller, &fixture.client).unwrap();
    let request = || {
        RequestEnvelope::new(
            RequestId::new(1).unwrap(),
            None,
            None,
            ServiceId::new("fixture.echo").unwrap(),
            ContractVersion::new(1, 0).unwrap(),
            OperationId::new("echo").unwrap(),
            b"public runtime".to_vec(),
        )
        .unwrap()
    };
    assert_eq!(
        poll_ready(ingress.call(request()).unwrap())
            .unwrap()
            .result()
            .unwrap(),
        b"public runtime"
    );
    assert_eq!(fixture.provider_calls.load(Ordering::SeqCst), 1);
    runtime.end_session(caller, &session).unwrap();
    assert!(context.check().is_err());
    assert!(!observation.is_live());
    assert!(ingress.call(request()).is_err());
    runtime.stop();
    drop(runtime);
    assert!(ingress.call(request()).is_err());
    drop(ingress);
    std::fs::remove_dir_all(path).unwrap();
}

#[test]
fn public_configuration_constructors_do_not_accept_missing_generation_or_name() {
    assert!(SuppliedAccounts::new(0, AccountStore::new()).is_err());
    assert!(ConfiguredBinding::new(
        Vec::new(),
        ProviderSubjectRef::new("test-issuer", "test-subject").unwrap(),
        identity::UserId::new(1).unwrap(),
        identity::ProfileId::new("profile_test").unwrap()
    )
    .is_err());
}
