#![forbid(unsafe_code)]

#[path = "../consumer/src/fixture.rs"]
#[allow(dead_code)]
mod fixture;

use fixture::{Fixture, FixtureCaller, FixtureIdentityPolicy, FixtureIds, NoGuestFixture};
use identity::{
    Clock, CurrentIdentityResolver, IdempotencyKey, IdentityService, InMemorySnapshotStore,
    LiveStatus, LocalUserMetadata, NoopIdentityEventSink, ProfileMetadata, ResolvedIdentity,
    TryClockError, TryLiveError, UnixMillis,
};
use nagi_c1_shared_live_candidate::{SharedCheckFailure, SharedObservation};
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use svc::CancellationToken;

// Injection is confined to host tests. These are not production caller,
// authentication, store, ID or consent suppliers.
type Hook = Arc<dyn Fn() + Send + Sync>;
#[derive(Clone)]
struct TestClock {
    now: Arc<AtomicU64>,
    mode: Arc<AtomicU8>,
    normal_calls: Arc<AtomicUsize>,
    try_calls: Arc<AtomicUsize>,
    hook: Arc<Mutex<Option<Hook>>>,
}
impl TestClock {
    fn new() -> Self {
        Self {
            now: Arc::new(AtomicU64::new(100)),
            mode: Arc::new(AtomicU8::new(0)),
            normal_calls: Arc::new(AtomicUsize::new(0)),
            try_calls: Arc::new(AtomicUsize::new(0)),
            hook: Arc::new(Mutex::new(None)),
        }
    }
}
impl Clock for TestClock {
    fn now(&self) -> UnixMillis {
        self.normal_calls.fetch_add(1, Ordering::SeqCst);
        UnixMillis::new(self.now.load(Ordering::SeqCst))
    }
    fn try_now(&self) -> Result<UnixMillis, TryClockError> {
        self.try_calls.fetch_add(1, Ordering::SeqCst);
        let hook = self
            .hook
            .try_lock()
            .map_err(|_| TryClockError::Busy)?
            .clone();
        if let Some(hook) = hook {
            hook();
        }
        match self.mode.load(Ordering::SeqCst) {
            0 => Ok(UnixMillis::new(self.now.load(Ordering::SeqCst))),
            1 => Err(TryClockError::Busy),
            _ => Err(TryClockError::Unavailable),
        }
    }
}
type Issuer<C> = IdentityService<
    InMemorySnapshotStore,
    FixtureIds,
    C,
    FixtureIdentityPolicy,
    NoGuestFixture,
    NoopIdentityEventSink,
>;

fn issue<C: Clock>(clock: C, caller: &FixtureCaller) -> (Issuer<C>, ResolvedIdentity) {
    issue_until(clock, caller, UnixMillis::new(200))
}

fn issue_until<C: Clock>(
    clock: C,
    caller: &FixtureCaller,
    expires: UnixMillis,
) -> (Issuer<C>, ResolvedIdentity) {
    let mut issuer = IdentityService::new(
        InMemorySnapshotStore::default(),
        FixtureIds::default(),
        clock,
        FixtureIdentityPolicy,
        NoGuestFixture,
        NoopIdentityEventSink,
    );
    issuer.recover(caller).unwrap();
    let (user, profile) = issuer
        .create_local_user(
            caller,
            LocalUserMetadata::new("Synthetic User").unwrap(),
            ProfileMetadata::new("Synthetic Profile", None).unwrap(),
        )
        .unwrap();
    issuer
        .login_local(
            caller,
            user,
            profile,
            Some(expires),
            IdempotencyKey::new("initial-test-login").unwrap(),
        )
        .unwrap();
    let identity = issuer.resolve_current_identity(caller).unwrap();
    (issuer, identity)
}

#[test]
fn legacy_clock_is_explicitly_unsupported_and_never_assumed_nonwaiting() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.identity.try_live_status(),
        Err(TryLiveError::Clock(TryClockError::Unsupported))
    );
    assert!(fixture.identity.is_live());
}

#[test]
fn clock_busy_and_failure_are_not_live_or_expired_and_never_call_legacy_now() {
    let caller = FixtureCaller::new("fixture.echo-client", "caller_fixture");
    let clock = TestClock::new();
    let (_issuer, identity) = issue(clock.clone(), &caller);
    let calls = clock.normal_calls.load(Ordering::SeqCst);
    let start = clock.try_calls.load(Ordering::SeqCst);
    for (mode, error) in [(1, TryClockError::Busy), (2, TryClockError::Unavailable)] {
        clock.mode.store(mode, Ordering::SeqCst);
        assert_eq!(identity.try_live_status(), Err(TryLiveError::Clock(error)));
    }
    assert_eq!(clock.normal_calls.load(Ordering::SeqCst), calls);
    assert_eq!(clock.try_calls.load(Ordering::SeqCst), start + 2);
    clock.mode.store(0, Ordering::SeqCst);
    assert_eq!(identity.try_live_status(), Ok(LiveStatus::Live));
}

#[test]
fn expiry_boundary_uses_the_issuer_clock_without_cached_live_result() {
    let caller = FixtureCaller::new("fixture.echo-client", "caller_fixture");
    let clock = TestClock::new();
    let (_issuer, identity) = issue(clock.clone(), &caller);
    clock.now.store(199, Ordering::SeqCst);
    let before = identity.try_live_snapshot().unwrap();
    assert_eq!(before.status(), LiveStatus::Live);
    clock.now.store(200, Ordering::SeqCst);
    let after = identity.try_live_snapshot().unwrap();
    assert_eq!(after.status(), LiveStatus::Expired);
    assert_eq!(before.generation(), after.generation());
    assert!(!identity.is_live());
}

#[test]
fn actual_linux_issuer_clock_drives_sdk_expiry_and_owner_drop_revocation() {
    use nagi_c1_shared_live_candidate::linux_clock::LinuxClock;
    let clock = LinuxClock::open(0).unwrap();
    let expires = UnixMillis::new(clock.try_now().unwrap().get().checked_add(1000).unwrap());
    let caller = FixtureCaller::new("fixture.echo-client", "caller_fixture");
    let (issuer, identity) = issue_until(clock.clone(), &caller, expires);
    assert_eq!(identity.try_live_status(), Ok(LiveStatus::Live));
    let limit = std::time::Instant::now();
    while clock.try_now().unwrap() < expires {
        assert!(limit.elapsed() < std::time::Duration::from_secs(3));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(identity.try_live_status(), Ok(LiveStatus::Expired));
    drop(issuer);
    assert_eq!(identity.try_live_status(), Ok(LiveStatus::Revoked));
}

#[test]
fn shared_observation_is_send_sync_and_other_thread_reads_actual_sdk_revocation() {
    fn shared<T: Send + Sync>() {}
    shared::<ResolvedIdentity>();
    shared::<SharedObservation<'static>>();
    let caller = FixtureCaller::new("fixture.echo-client", "caller_fixture");
    let (mut issuer, identity) = issue(TestClock::new(), &caller);
    let session = identity.session_id().clone();
    let observation =
        SharedObservation::from_resolved(identity, &caller, CancellationToken::new()).unwrap();
    let original = observation.check().unwrap();
    let (go, receive) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let observed = &observation;
        let worker = scope.spawn(move || {
            receive
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap();
            let snapshot = observed.check().unwrap();
            assert_eq!(snapshot.status(), LiveStatus::Revoked);
            assert_eq!(snapshot.generation(), original.generation());
        });
        issuer.end_session(&caller, &session).unwrap();
        go.send(()).unwrap();
        worker.join().unwrap();
    });
}

#[test]
fn owned_real_caller_supports_static_shared_storage_without_extending_issuer_lifetime() {
    let caller = Arc::new(FixtureCaller::new("fixture.echo-client", "caller_fixture"));
    let (issuer, identity) = issue(TestClock::new(), caller.as_ref());
    let weak = Arc::downgrade(&caller);
    let shared: Arc<SharedObservation<'static>> = Arc::new(
        SharedObservation::from_owned(identity, caller, CancellationToken::new()).unwrap(),
    );
    assert!(weak.upgrade().is_some());
    let shared = std::thread::spawn(move || {
        assert_eq!(shared.check().unwrap().status(), LiveStatus::Live);
        shared
    })
    .join()
    .unwrap();
    drop(issuer);
    assert_eq!(shared.check().unwrap().status(), LiveStatus::Revoked);
    drop(shared);
    assert!(weak.upgrade().is_none());
}

#[test]
fn issuer_drop_in_clock_callback_defeats_earlier_live_read_even_on_clock_failure() {
    for mode in [0, 1, 2] {
        let caller = FixtureCaller::new("fixture.echo-client", "caller_fixture");
        let clock = TestClock::new();
        let (issuer, identity) = issue(clock.clone(), &caller);
        let generation = identity.try_live_snapshot().unwrap().generation();
        let owner = Arc::new(Mutex::new(Some(issuer)));
        let drop_owner = owner.clone();
        *clock.hook.lock().unwrap() = Some(Arc::new(move || {
            drop(drop_owner.lock().unwrap().take());
        }));
        clock.mode.store(mode, Ordering::SeqCst);
        let snapshot = identity.try_live_snapshot().unwrap();
        assert_eq!(snapshot.status(), LiveStatus::Revoked);
        assert_eq!(snapshot.generation(), generation);
        assert!(owner.lock().unwrap().is_none());
        *clock.hook.lock().unwrap() = None;
        let calls = clock.try_calls.load(Ordering::SeqCst);
        assert_eq!(identity.try_live_status(), Ok(LiveStatus::Revoked));
        assert_eq!(clock.try_calls.load(Ordering::SeqCst), calls);
    }
}

#[test]
fn caller_pair_is_checked_after_the_real_sdk_clock_callback() {
    for principal in [false, true] {
        let mut caller = FixtureCaller::new("fixture.echo-client", "caller_fixture");
        let changed = caller.arm_change(principal);
        let clock = TestClock::new();
        let (_issuer, identity) = issue(clock.clone(), &caller);
        let observation =
            SharedObservation::from_resolved(identity, &caller, CancellationToken::new()).unwrap();
        *clock.hook.lock().unwrap() = Some(Arc::new(move || {
            changed.store(true, Ordering::SeqCst);
        }));
        assert_eq!(observation.check(), Err(SharedCheckFailure::CallerMismatch));
    }
}

#[test]
fn cancellation_from_original_supplied_token_is_rechecked_after_clock_callback() {
    let caller = FixtureCaller::new("fixture.echo-client", "caller_fixture");
    let clock = TestClock::new();
    let (_issuer, identity) = issue(clock.clone(), &caller);
    let cancellation = CancellationToken::new();
    let observation =
        SharedObservation::from_resolved(identity, &caller, cancellation.clone()).unwrap();
    *clock.hook.lock().unwrap() = Some(Arc::new(move || cancellation.cancel()));
    assert_eq!(observation.check(), Err(SharedCheckFailure::Cancelled));
}

#[test]
fn shared_factory_never_publishes_a_cancelled_expired_or_revoked_observation() {
    let caller = FixtureCaller::new("fixture.echo-client", "caller_fixture");
    let clock = TestClock::new();
    let (_issuer, identity) = issue(clock.clone(), &caller);
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let calls = clock.try_calls.load(Ordering::SeqCst);
    assert!(matches!(
        SharedObservation::from_resolved(identity, &caller, cancellation),
        Err(SharedCheckFailure::Cancelled)
    ));
    assert_eq!(clock.try_calls.load(Ordering::SeqCst), calls);

    let (issuer, identity) = issue(TestClock::new(), &caller);
    drop(issuer);
    assert!(matches!(
        SharedObservation::from_resolved(identity, &caller, CancellationToken::new()),
        Err(SharedCheckFailure::Stale(LiveStatus::Revoked))
    ));

    let clock = TestClock::new();
    let (_issuer, identity) = issue(clock.clone(), &caller);
    clock.now.store(200, Ordering::SeqCst);
    assert!(matches!(
        SharedObservation::from_resolved(identity, &caller, CancellationToken::new()),
        Err(SharedCheckFailure::Stale(LiveStatus::Expired))
    ));
}

#[test]
fn same_session_resolves_same_revision_but_logout_and_recovery_never_reuse_it() {
    let caller = FixtureCaller::new("fixture.echo-client", "caller_fixture");
    let (mut issuer, first) = issue(TestClock::new(), &caller);
    let original = first.try_live_snapshot().unwrap().generation();
    let again = issuer.resolve_current_identity(&caller).unwrap();
    assert_eq!(again.try_live_snapshot().unwrap().generation(), original);
    issuer.end_session(&caller, first.session_id()).unwrap();
    issuer
        .login_local(
            &caller,
            first.user_id().unwrap(),
            first.profile_id().clone(),
            Some(UnixMillis::new(200)),
            IdempotencyKey::new("after-logout").unwrap(),
        )
        .unwrap();
    let second = issuer.resolve_current_identity(&caller).unwrap();
    let next = second.try_live_snapshot().unwrap().generation();
    assert!(next > original);
    issuer.recover(&caller).unwrap();
    issuer
        .login_local(
            &caller,
            first.user_id().unwrap(),
            first.profile_id().clone(),
            Some(UnixMillis::new(200)),
            IdempotencyKey::new("after-recover").unwrap(),
        )
        .unwrap();
    let third = issuer.resolve_current_identity(&caller).unwrap();
    assert!(third.try_live_snapshot().unwrap().generation() > next);
    assert_eq!(first.try_live_status(), Ok(LiveStatus::Revoked));
    assert_eq!(second.try_live_status(), Ok(LiveStatus::Revoked));
}

// Exact owner-supplied C2 SessionLease excerpt (Stage22/23). Its crate-private
// constructor requires co-location. This is an excerpt compilation, NOT the
// C2 crate/PeerDenial compilation: its dependent external error types/source
// are not available in this cloud candidate and are not replaced with stubs.
mod c2_session_lease_excerpt {
    use std::num::NonZeroU64;
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct SessionLease {
        app_id: u64,
        app_session_id: u64,
        user_session_generation: NonZeroU64,
    }
    impl SessionLease {
        pub(crate) const fn from_live_session_authority(
            app_id: u64,
            app_session_id: u64,
            user_session_generation: u64,
        ) -> Option<Self> {
            let Some(user_session_generation) = NonZeroU64::new(user_session_generation) else {
                return None;
            };
            Some(Self {
                app_id,
                app_session_id,
                user_session_generation,
            })
        }
    }
    // Test-only co-location glue; no real Supervisor record is supplied here.
    pub fn captures_real_snapshot(
        app: u64,
        session: u64,
        snapshot: identity::LiveSnapshot,
    ) -> SessionLease {
        assert_eq!(snapshot.status(), identity::LiveStatus::Live);
        SessionLease::from_live_session_authority(app, session, snapshot.generation().get())
            .unwrap()
    }
    pub fn matches(
        lease: SessionLease,
        app: u64,
        session: u64,
        snapshot: identity::LiveSnapshot,
    ) -> bool {
        lease.app_id == app
            && lease.app_session_id == session
            && lease.user_session_generation == snapshot.generation()
            && snapshot.status() == identity::LiveStatus::Live
    }
}

#[test]
fn c2_exact_lease_excerpt_uses_actual_sdk_instance_not_notice_or_session_profile_pair() {
    use c2_session_lease_excerpt::{captures_real_snapshot, matches};
    let caller = FixtureCaller::new("fixture.echo-client", "caller_fixture");
    let (mut issuer, first) = issue(TestClock::new(), &caller);
    let lease = captures_real_snapshot(41, 9, first.try_live_snapshot().unwrap());
    assert!(matches(lease, 41, 9, first.try_live_snapshot().unwrap()));
    assert!(!matches(lease, 41, 10, first.try_live_snapshot().unwrap()));
    issuer.end_session(&caller, first.session_id()).unwrap();
    assert!(!matches(lease, 41, 9, first.try_live_snapshot().unwrap()));
    issuer
        .login_local(
            &caller,
            first.user_id().unwrap(),
            first.profile_id().clone(),
            Some(UnixMillis::new(200)),
            IdempotencyKey::new("c2-new-session").unwrap(),
        )
        .unwrap();
    let fresh = issuer.resolve_current_identity(&caller).unwrap();
    assert!(!matches(lease, 41, 9, fresh.try_live_snapshot().unwrap()));
}
