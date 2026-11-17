use nagi_c1_session_end_adapter::end_owner::{
    CleanupFailure, CleanupProgress, CleanupState, DeliveryState, EndAttemptState, EndFailure,
    EndOwner,
};
use nagi_c1_session_end_adapter::owner_proxy::{
    channel, OwnerRequests, OwnerStep, ProxyFailure, SharedAuthorityProxy,
};
use nagi_c1_session_end_adapter::{bind_exact_child, ChildBindingFailure, RouteError};
use nagi_identity_session::{
    AuthenticatedInteractiveCaller, AuthenticatedProvisioningCaller,
    AuthenticatedSystemServicePrincipal, AuthorizationFailure, AuthorizationRequest,
    CallerContextId, Clock, EndSessionResult, GuestCleanupFailure, GuestProfileCleaner,
    IdSourceFailure, IdentityAction, IdentityAuthorizer, IdentityError, IdentityIdSource,
    IdentityService, InMemorySnapshotStore, InteractiveSessionContext, InteractiveSessionError,
    LocalUserMetadata, NoopIdentityEventSink, PrincipalId, ProfileId, ProfileMetadata,
    ResolutionError, SessionId, TrustedCallerContext, UnixMillis, UserId,
};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

struct TestCaller {
    primary_principal: PrincipalId,
    alternate_principal: PrincipalId,
    primary_context: CallerContextId,
    alternate_context: CallerContextId,
    changed: Arc<AtomicBool>,
}

impl TestCaller {
    fn new() -> Self {
        Self::with_change_flag(Arc::new(AtomicBool::new(false)))
    }

    fn with_change_flag(changed: Arc<AtomicBool>) -> Self {
        Self {
            primary_principal: PrincipalId::new("system.identity-test").unwrap(),
            alternate_principal: PrincipalId::new("system.identity-alternate").unwrap(),
            primary_context: CallerContextId::new("caller_primary").unwrap(),
            alternate_context: CallerContextId::new("caller_alternate").unwrap(),
            changed,
        }
    }
}

impl TrustedCallerContext for TestCaller {
    fn principal_id(&self) -> &PrincipalId {
        if self.changed.load(Ordering::SeqCst) {
            &self.alternate_principal
        } else {
            &self.primary_principal
        }
    }

    fn context_id(&self) -> &CallerContextId {
        if self.changed.load(Ordering::SeqCst) {
            &self.alternate_context
        } else {
            &self.primary_context
        }
    }
}

impl AuthenticatedInteractiveCaller for TestCaller {}
impl nagi_identity_session::caller_proof_seal::Interactive for TestCaller {}

struct TestProvisioningCaller {
    principal: PrincipalId,
    context: CallerContextId,
}

impl TestProvisioningCaller {
    fn new() -> Self {
        Self {
            principal: PrincipalId::new("system.test-provisioner").unwrap(),
            context: CallerContextId::new("caller_test_provisioner").unwrap(),
        }
    }
}

impl TrustedCallerContext for TestProvisioningCaller {
    fn principal_id(&self) -> &PrincipalId {
        &self.principal
    }

    fn context_id(&self) -> &CallerContextId {
        &self.context
    }
}

impl AuthenticatedProvisioningCaller for TestProvisioningCaller {}
impl nagi_identity_session::caller_proof_seal::Provisioning for TestProvisioningCaller {}

struct TestSystemCaller {
    principal: PrincipalId,
    context: CallerContextId,
}

impl TestSystemCaller {
    fn new() -> Self {
        Self {
            principal: PrincipalId::new("system.test-service").unwrap(),
            context: CallerContextId::new("caller_test_service").unwrap(),
        }
    }
}

impl TrustedCallerContext for TestSystemCaller {
    fn principal_id(&self) -> &PrincipalId {
        &self.principal
    }

    fn context_id(&self) -> &CallerContextId {
        &self.context
    }
}

impl AuthenticatedSystemServicePrincipal for TestSystemCaller {}
impl nagi_identity_session::caller_proof_seal::SystemService for TestSystemCaller {}

#[derive(Clone)]
struct TestClock(Arc<AtomicU64>);

impl TestClock {
    fn new(now: u64) -> Self {
        Self(Arc::new(AtomicU64::new(now)))
    }

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
}

impl IdentityIdSource for TestIds {
    fn next_user_id(&mut self) -> Result<UserId, IdSourceFailure> {
        self.user += 1;
        UserId::new(self.user).map_err(|_| IdSourceFailure::Unavailable)
    }

    fn next_profile_id(&mut self) -> Result<ProfileId, IdSourceFailure> {
        self.profile += 1;
        ProfileId::new(format!("profile_p{}", self.profile))
            .map_err(|_| IdSourceFailure::Unavailable)
    }

    fn next_session_id(&mut self) -> Result<SessionId, IdSourceFailure> {
        self.session += 1;
        SessionId::new(format!("session_s{}", self.session))
            .map_err(|_| IdSourceFailure::Unavailable)
    }
}

#[derive(Default)]
struct TestAuthorizer {
    deny_end_once: bool,
    switch_caller_on_end: Option<Arc<AtomicBool>>,
    end_calls: Option<Arc<AtomicUsize>>,
}

impl IdentityAuthorizer for TestAuthorizer {
    fn authorize(&mut self, request: &AuthorizationRequest) -> Result<(), AuthorizationFailure> {
        if request.action == IdentityAction::EndSession {
            if let Some(calls) = &self.end_calls {
                calls.fetch_add(1, Ordering::SeqCst);
            }
            if let Some(changed) = &self.switch_caller_on_end {
                changed.store(true, Ordering::SeqCst);
                return Err(AuthorizationFailure::Denied);
            }
            if self.deny_end_once {
                self.deny_end_once = false;
                return Err(AuthorizationFailure::Denied);
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct TestCleaner;

impl GuestProfileCleaner for TestCleaner {
    fn cleanup_guest_profile(
        &mut self,
        _profile_id: &ProfileId,
    ) -> Result<(), GuestCleanupFailure> {
        Ok(())
    }
}

type Service = IdentityService<
    InMemorySnapshotStore,
    TestIds,
    TestClock,
    TestAuthorizer,
    TestCleaner,
    NoopIdentityEventSink,
>;

fn service(
    _caller: &TestCaller,
    clock: TestClock,
    authorizer: TestAuthorizer,
) -> (Service, UserId, ProfileId) {
    let mut service = IdentityService::new(
        InMemorySnapshotStore::default(),
        TestIds::default(),
        clock,
        authorizer,
        TestCleaner,
        NoopIdentityEventSink,
    );
    service.recover(&TestSystemCaller::new()).unwrap();
    let (user_id, profile_id) = service
        .create_local_user(
            &TestProvisioningCaller::new(),
            LocalUserMetadata::new("Test User").unwrap(),
            ProfileMetadata::new("Home", Some("en-US".to_owned())).unwrap(),
        )
        .unwrap();
    (service, user_id, profile_id)
}

fn login(
    service: &mut Service,
    caller: &TestCaller,
    user: UserId,
    profile: &ProfileId,
    key: &str,
    expiry: Option<u64>,
) -> SessionId {
    service
        .login_local(
            caller,
            user,
            profile.clone(),
            expiry.map(UnixMillis::new),
            nagi_identity_session::IdempotencyKey::new(key).unwrap(),
        )
        .unwrap()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TestTransition {
    Lock,
    Logout,
}

#[derive(Default)]
struct UiProbe {
    dirty: String,
    events: Vec<&'static str>,
    new_operations: usize,
    delivery_attempts: usize,
    cleanup_requests: usize,
    cleanup_observations: usize,
}

impl UiProbe {
    fn new() -> Self {
        Self {
            dirty: "unsaved local state".to_owned(),
            ..Self::default()
        }
    }
}

fn map_transition(transition: TestTransition) -> &'static str {
    match transition {
        TestTransition::Lock => "locked",
        TestTransition::Logout => "logged_out",
    }
}

#[test]
fn real_session_end_stops_operations_and_delivers_once_to_its_attachment() {
    let caller = TestCaller::new();
    let (mut service, user, profile) =
        service(&caller, TestClock::new(100), TestAuthorizer::default());
    let session_id = login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-one",
        Some(1_000),
    );
    let mut ui = UiProbe::new();
    let mut owner = EndOwner::attach(
        service.interactive_session(&caller).unwrap(),
        &caller,
        &mut ui,
    )
    .unwrap();

    owner
        .with_live_receiver(|ui, context| {
            assert!(context.check().is_ok());
            ui.new_operations += 1;
        })
        .unwrap();
    let delivery = owner.end(&mut service, TestTransition::Logout).unwrap();
    assert_eq!(service.snapshot().sessions[0].session_id, session_id);
    assert_eq!(
        service.interactive_session(&caller).err(),
        Some(InteractiveSessionError::Identity(ResolutionError::Ended))
    );
    assert_eq!(owner.end_attempt_state(), EndAttemptState::Ended);
    assert_eq!(
        owner.admit_new_operation(),
        Err(RouteError::OperationsStopped)
    );

    owner
        .deliver(&delivery, map_transition, |ui, event| ui.events.push(event))
        .unwrap();
    assert_eq!(owner.delivery_state(), DeliveryState::CallbackReturned);
    assert_eq!(owner.receiver().events, vec!["logged_out"]);
    assert_eq!(owner.receiver().dirty, "unsaved local state");
    assert_eq!(owner.receiver().new_operations, 1);
    assert_eq!(
        owner.deliver(&delivery, map_transition, |_, _| panic!(
            "duplicate delivery"
        )),
        Err(RouteError::DuplicateNotice)
    );
}

#[test]
fn public_delivery_cannot_rebind_to_a_new_generation_for_the_same_ui() {
    let caller = TestCaller::new();
    let (mut service, user, profile) =
        service(&caller, TestClock::new(100), TestAuthorizer::default());
    login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-generation",
        Some(1_000),
    );
    let mut first_ui = UiProbe::new();
    let mut second_ui = UiProbe::new();
    let mut first = EndOwner::attach(
        service.interactive_session(&caller).unwrap(),
        &caller,
        &mut first_ui,
    )
    .unwrap();
    let mut second = EndOwner::attach(
        service.interactive_session(&caller).unwrap(),
        &caller,
        &mut second_ui,
    )
    .unwrap();
    let delivery = first.end(&mut service, TestTransition::Lock).unwrap();

    assert_eq!(
        second.deliver(&delivery, map_transition, |_, _| panic!(
            "replacement touched"
        )),
        Err(RouteError::StaleGeneration)
    );
    assert!(second.receiver().events.is_empty());
    first
        .deliver(&delivery, map_transition, |ui, event| ui.events.push(event))
        .unwrap();
    assert_eq!(first.receiver().events, vec!["locked"]);
}

#[test]
fn delivery_from_an_old_session_cannot_be_applied_to_a_fresh_attachment() {
    let caller = TestCaller::new();
    let (mut service, user, profile) =
        service(&caller, TestClock::new(100), TestAuthorizer::default());
    let old_session = login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-old",
        Some(1_000),
    );
    let mut ui = UiProbe::new();
    let mut old_owner = EndOwner::attach(
        service.interactive_session(&caller).unwrap(),
        &caller,
        &mut ui,
    )
    .unwrap();
    let delivery = old_owner.end(&mut service, TestTransition::Lock).unwrap();
    let ui = old_owner.into_receiver();
    let new_session = login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-new",
        Some(2_000),
    );
    assert_ne!(old_session, new_session);
    let mut current =
        EndOwner::attach(service.interactive_session(&caller).unwrap(), &caller, ui).unwrap();

    assert_eq!(
        current.deliver(&delivery, map_transition, |_, _| panic!(
            "old delivery applied"
        )),
        Err(RouteError::StaleGeneration)
    );
    assert!(current.admit_new_operation().is_ok());
    assert!(current.receiver().events.is_empty());
    assert_eq!(current.receiver().dirty, "unsaved local state");
}

#[test]
fn pre_effect_denial_retries_only_end_and_keeps_operations_stopped() {
    let caller = TestCaller::new();
    let authorizer = TestAuthorizer {
        deny_end_once: true,
        ..TestAuthorizer::default()
    };
    let (mut service, user, profile) = service(&caller, TestClock::new(100), authorizer);
    login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-retry",
        Some(1_000),
    );
    let mut ui = UiProbe::new();
    let mut owner = EndOwner::attach(
        service.interactive_session(&caller).unwrap(),
        &caller,
        &mut ui,
    )
    .unwrap();

    assert!(matches!(
        owner.end(&mut service, TestTransition::Logout),
        Err(EndFailure::Lifecycle(IdentityError::AuthorizationDenied))
    ));
    assert_eq!(
        owner.end_attempt_state(),
        EndAttemptState::RetryablePreEffect
    );
    assert_eq!(
        owner.admit_new_operation(),
        Err(RouteError::OperationsStopped)
    );
    let delivery = owner.end(&mut service, TestTransition::Logout).unwrap();
    assert_eq!(owner.end_attempt_state(), EndAttemptState::Ended);
    owner
        .deliver(&delivery, map_transition, |ui, event| ui.events.push(event))
        .unwrap();
    assert_eq!(owner.receiver().events, vec!["logged_out"]);
}

#[test]
fn failed_end_after_caller_change_is_outcome_unknown_and_never_retried() {
    let changed = Arc::new(AtomicBool::new(false));
    let caller = TestCaller::with_change_flag(Arc::clone(&changed));
    let end_calls = Arc::new(AtomicUsize::new(0));
    let authorizer = TestAuthorizer {
        switch_caller_on_end: Some(Arc::clone(&changed)),
        end_calls: Some(Arc::clone(&end_calls)),
        ..TestAuthorizer::default()
    };
    let (mut service, user, profile) = service(&caller, TestClock::new(100), authorizer);
    login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-changed-caller",
        Some(1_000),
    );
    let mut ui = UiProbe::new();
    let mut owner = EndOwner::attach(
        service.interactive_session(&caller).unwrap(),
        &caller,
        &mut ui,
    )
    .unwrap();

    assert!(matches!(
        owner.end(&mut service, TestTransition::Logout),
        Err(EndFailure::Lifecycle(IdentityError::AuthorizationDenied))
    ));
    assert_eq!(owner.end_attempt_state(), EndAttemptState::OutcomeUnknown);
    assert!(matches!(
        owner.end(&mut service, TestTransition::Logout),
        Err(EndFailure::AlreadyAttempted)
    ));
    assert_eq!(end_calls.load(Ordering::SeqCst), 1);
    assert!(owner.receiver().events.is_empty());
}

#[test]
fn end_owner_cannot_mutate_a_different_service_with_a_colliding_session_id() {
    let caller = TestCaller::new();
    let (mut issuer, issuer_user, issuer_profile) =
        service(&caller, TestClock::new(100), TestAuthorizer::default());
    let (mut other, other_user, other_profile) =
        service(&caller, TestClock::new(100), TestAuthorizer::default());
    let issuer_session = login(
        &mut issuer,
        &caller,
        issuer_user,
        &issuer_profile,
        "issuer-login",
        Some(1_000),
    );
    let other_session = login(
        &mut other,
        &caller,
        other_user,
        &other_profile,
        "other-login",
        Some(1_000),
    );
    assert_eq!(issuer_session, other_session);
    let mut ui = UiProbe::new();
    let mut owner = EndOwner::attach(
        issuer.interactive_session(&caller).unwrap(),
        &caller,
        &mut ui,
    )
    .unwrap();

    assert!(matches!(
        owner.end(&mut other, TestTransition::Logout),
        Err(EndFailure::WrongIssuer)
    ));
    assert_eq!(owner.end_attempt_state(), EndAttemptState::NotAttempted);
    assert!(issuer.interactive_session(&caller).unwrap().check().is_ok());
    assert!(other.interactive_session(&caller).unwrap().check().is_ok());

    owner.end(&mut issuer, TestTransition::Logout).unwrap();
    assert_eq!(
        issuer.interactive_session(&caller).err(),
        Some(InteractiveSessionError::Identity(ResolutionError::Ended))
    );
    assert!(other.interactive_session(&caller).unwrap().check().is_ok());
}

#[test]
fn dropping_the_issuing_service_invalidates_its_retained_context() {
    let caller = TestCaller::new();
    let (mut issuer, user, profile) =
        service(&caller, TestClock::new(100), TestAuthorizer::default());
    login(
        &mut issuer,
        &caller,
        user,
        &profile,
        "drop-issuer",
        Some(1_000),
    );
    let context = issuer.interactive_session(&caller).unwrap();
    drop(issuer);
    assert_eq!(
        context.check(),
        Err(InteractiveSessionError::Identity(
            ResolutionError::Unavailable
        ))
    );
}

#[test]
fn callback_panic_leaves_delivery_unconfirmed_and_never_replays_it() {
    let caller = TestCaller::new();
    let (mut service, user, profile) =
        service(&caller, TestClock::new(100), TestAuthorizer::default());
    login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-callback-panic",
        Some(1_000),
    );
    let mut ui = UiProbe::new();
    let mut owner = EndOwner::attach(
        service.interactive_session(&caller).unwrap(),
        &caller,
        &mut ui,
    )
    .unwrap();
    let delivery = owner.end(&mut service, TestTransition::Logout).unwrap();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = owner.deliver(&delivery, map_transition, |ui, _| {
            ui.delivery_attempts += 1;
            panic!("receiver failed");
        });
    }));
    assert!(result.is_err());
    assert_eq!(owner.delivery_state(), DeliveryState::Unconfirmed);
    assert_eq!(owner.receiver().delivery_attempts, 1);
    assert_eq!(owner.receiver().dirty, "unsaved local state");
    assert_eq!(
        owner.deliver(&delivery, map_transition, |_, _| panic!("automatic retry")),
        Err(RouteError::DuplicateNotice)
    );
    assert_eq!(owner.receiver().delivery_attempts, 1);
}

#[test]
fn expiry_stops_existing_context_and_cleanup_is_requested_once_then_polled() {
    let caller = TestCaller::new();
    let clock = TestClock::new(100);
    let (mut service, user, profile) = service(&caller, clock.clone(), TestAuthorizer::default());
    login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-expiry",
        Some(200),
    );
    let mut ui = UiProbe::new();
    let mut owner = EndOwner::attach(
        service.interactive_session(&caller).unwrap(),
        &caller,
        &mut ui,
    )
    .unwrap();
    clock.set(200);

    assert_eq!(
        owner.with_live_receiver(|ui, _| ui.dirty.clear()),
        Err(RouteError::Context(InteractiveSessionError::Identity(
            ResolutionError::Expired
        )))
    );
    assert!(owner.operations_stopped());
    assert_eq!(
        owner.request_cleanup_once(|ui, reason| {
            assert_eq!(
                reason,
                InteractiveSessionError::Identity(ResolutionError::Expired)
            );
            ui.cleanup_requests += 1;
            Ok::<_, ()>(())
        }),
        Ok(())
    );
    assert_eq!(owner.cleanup_state(), CleanupState::Requested);
    assert_eq!(
        owner.poll_cleanup(|ui, _| {
            ui.cleanup_observations += 1;
            Ok::<_, ()>(CleanupProgress::Pending)
        }),
        Ok(CleanupProgress::Pending)
    );
    assert_eq!(
        owner.poll_cleanup(|ui, _| {
            ui.cleanup_observations += 1;
            Ok::<_, ()>(CleanupProgress::Reaped)
        }),
        Ok(CleanupProgress::Reaped)
    );
    assert_eq!(owner.receiver().cleanup_requests, 1);
    assert_eq!(owner.receiver().cleanup_observations, 2);
    assert_eq!(owner.receiver().dirty, "unsaved local state");
    assert_eq!(
        owner
            .request_cleanup_once(|_, _| -> Result<(), ()> { panic!("duplicate cleanup request") }),
        Err(CleanupFailure::AlreadyAttempted)
    );
}

#[test]
fn ambiguous_cleanup_request_and_unknown_observation_allow_poll_only_retry() {
    let caller = TestCaller::new();
    let (mut service, user, profile) =
        service(&caller, TestClock::new(100), TestAuthorizer::default());
    login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-unknown-cleanup",
        Some(1_000),
    );
    let mut ui = UiProbe::new();
    let mut owner = EndOwner::attach(
        service.interactive_session(&caller).unwrap(),
        &caller,
        &mut ui,
    )
    .unwrap();
    owner.end(&mut service, TestTransition::Logout).unwrap();

    assert_eq!(
        owner.request_cleanup_once(|ui, _| {
            ui.cleanup_requests += 1;
            Err::<(), _>("request outcome unknown")
        }),
        Err(CleanupFailure::Receiver("request outcome unknown"))
    );
    assert_eq!(owner.cleanup_state(), CleanupState::Unknown);
    assert_eq!(
        owner.poll_cleanup(|ui, _| {
            ui.cleanup_observations += 1;
            Ok::<_, ()>(CleanupProgress::Unknown)
        }),
        Ok(CleanupProgress::Unknown)
    );
    assert_eq!(
        owner.poll_cleanup(|ui, _| {
            ui.cleanup_observations += 1;
            Ok::<_, ()>(CleanupProgress::Reaped)
        }),
        Ok(CleanupProgress::Reaped)
    );
    assert_eq!(owner.cleanup_state(), CleanupState::Requested);
    assert_eq!(owner.receiver().cleanup_requests, 1);
    assert_eq!(owner.receiver().cleanup_observations, 2);
    assert_eq!(
        owner.request_cleanup_once(|_, _| -> Result<(), ()> {
            panic!("cleanup callback must not be re-run")
        }),
        Err(CleanupFailure::AlreadyAttempted)
    );
}

#[test]
fn recovery_and_relogin_do_not_revive_the_old_c1_context() {
    let caller = TestCaller::new();
    let (mut service, user, profile) =
        service(&caller, TestClock::new(100), TestAuthorizer::default());
    let old_session = login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-before-recovery",
        Some(1_000),
    );
    let mut ui = UiProbe::new();
    let mut old_owner = EndOwner::attach(
        service.interactive_session(&caller).unwrap(),
        &caller,
        &mut ui,
    )
    .unwrap();

    service.recover(&TestSystemCaller::new()).unwrap();
    let new_session = login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-after-recovery",
        Some(2_000),
    );
    assert_ne!(old_session, new_session);
    assert_eq!(
        old_owner.request_cleanup_once(|ui, reason| {
            assert_eq!(
                reason,
                InteractiveSessionError::Identity(ResolutionError::Ended)
            );
            ui.cleanup_requests += 1;
            Ok::<_, ()>(())
        }),
        Ok(())
    );
    assert_eq!(
        old_owner.poll_cleanup(|ui, _| {
            ui.cleanup_observations += 1;
            Ok::<_, ()>(CleanupProgress::Pending)
        }),
        Ok(CleanupProgress::Pending)
    );
    assert_eq!(old_owner.receiver().cleanup_requests, 1);
    assert_eq!(
        service
            .interactive_session(&caller)
            .unwrap()
            .identity()
            .unwrap()
            .session_id(),
        &new_session
    );
}

#[test]
fn child_selector_is_not_called_for_an_expired_context() {
    let caller = TestCaller::new();
    let clock = TestClock::new(100);
    let (mut service, user, profile) = service(&caller, clock.clone(), TestAuthorizer::default());
    login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-expired-child",
        Some(200),
    );
    let context = service.interactive_session(&caller).unwrap();
    clock.set(200);
    let mut receiver = 0_usize;
    let mut selected = 0_usize;

    let result = bind_exact_child(context, &caller, |_| {
        selected += 1;
        Ok::<_, ()>(&mut receiver)
    });
    assert_eq!(
        result.err(),
        Some(ChildBindingFailure::Context(RouteError::Context(
            InteractiveSessionError::Identity(ResolutionError::Expired)
        )))
    );
    assert_eq!(selected, 0);
    assert_eq!(receiver, 0);
}

fn pump_proxy(
    owner: &mut OwnerRequests<SessionId, InteractiveSessionError>,
    context: &InteractiveSessionContext<'_>,
    session: &SessionId,
) -> OwnerStep {
    let limit = Instant::now();
    loop {
        let step = owner
            .poll_one(
                |application, application_session| {
                    if (application, application_session) != (41, 9) {
                        return Err(InteractiveSessionError::CallerMismatch);
                    }
                    context.check()?;
                    Ok(context.identity()?.session_id().clone())
                },
                |lease| {
                    if &lease != session {
                        return Err(InteractiveSessionError::CallerMismatch);
                    }
                    Ok(context.check().is_ok())
                },
                |_| panic!("unexpected unclaimed lease"),
            )
            .unwrap();
        if let Some(step) = step {
            return step;
        }
        assert!(limit.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn owner_proxy_revalidates_real_c1_context_after_end_and_relogin() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<SharedAuthorityProxy<SessionId, InteractiveSessionError>>();

    let caller = TestCaller::new();
    let (mut service, user, profile) =
        service(&caller, TestClock::new(100), TestAuthorizer::default());
    let session = login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-proxy",
        Some(1_000),
    );
    let context = service.interactive_session(&caller).unwrap();
    let (proxy, mut owner) = channel::<SessionId, InteractiveSessionError>(
        std::num::NonZeroUsize::new(1).unwrap(),
        Duration::from_secs(2),
    );
    let (captured, captured_rx) = std::sync::mpsc::channel();
    let (resume, resume_rx) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        let result = proxy.capture_request(41, 9).unwrap();
        captured.send(result.clone()).unwrap();
        resume_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(proxy.check_request(result), Ok(false));
        assert_eq!(
            proxy.capture_request(41, 9),
            Err(ProxyFailure::Authority(InteractiveSessionError::Identity(
                ResolutionError::Ended
            )))
        );
    });
    assert_eq!(
        pump_proxy(&mut owner, &context, &session),
        OwnerStep::CaptureHandled
    );
    assert_eq!(
        captured_rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        session
    );

    assert_eq!(
        service.end_session(&caller, &session).unwrap(),
        EndSessionResult::Ended
    );
    let fresh = login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-proxy-fresh",
        Some(2_000),
    );
    assert_ne!(fresh, session);
    resume.send(()).unwrap();
    assert_eq!(
        pump_proxy(&mut owner, &context, &session),
        OwnerStep::CheckHandled
    );
    assert_eq!(
        pump_proxy(&mut owner, &context, &session),
        OwnerStep::CaptureHandled
    );
    worker.join().unwrap();
}

#[test]
fn owner_proxy_rejects_self_wait_and_skips_an_expired_queued_capture() {
    let (proxy, mut owner) = channel::<u8, ()>(
        std::num::NonZeroUsize::new(1).unwrap(),
        Duration::from_millis(20),
    );
    assert_eq!(proxy.capture_request(41, 9), Err(ProxyFailure::OwnerThread));
    let worker = thread::spawn(move || proxy.capture_request(41, 9));
    assert_eq!(worker.join().unwrap(), Err(ProxyFailure::TimedOut));
    assert_eq!(
        owner.poll_one(
            |_, _| panic!("expired capture ran"),
            |_| panic!("unexpected check"),
            |_| panic!("there is no lease to release"),
        ),
        Ok(Some(OwnerStep::Expired))
    );
}

#[test]
fn late_proxy_capture_releases_the_unclaimed_lease_once() {
    let caller = TestCaller::new();
    let (mut service, user, profile) =
        service(&caller, TestClock::new(100), TestAuthorizer::default());
    let session = login(
        &mut service,
        &caller,
        user,
        &profile,
        "login-late-proxy",
        Some(1_000),
    );
    let context = service.interactive_session(&caller).unwrap();
    let (proxy, mut owner) = channel::<SessionId, InteractiveSessionError>(
        std::num::NonZeroUsize::new(1).unwrap(),
        Duration::from_millis(150),
    );
    let worker = thread::spawn(move || proxy.capture_request(41, 9));
    let mut released = Vec::new();
    let limit = Instant::now();
    loop {
        let result = owner
            .poll_one(
                |_, _| {
                    context.check()?;
                    thread::sleep(Duration::from_millis(220));
                    Ok(context.identity()?.session_id().clone())
                },
                |_| panic!("unexpected check"),
                |lease| released.push(lease),
            )
            .unwrap();
        if let Some(step) = result {
            assert_eq!(step, OwnerStep::Expired);
            break;
        }
        assert!(limit.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(worker.join().unwrap(), Err(ProxyFailure::TimedOut));
    assert_eq!(released, vec![session]);
}
