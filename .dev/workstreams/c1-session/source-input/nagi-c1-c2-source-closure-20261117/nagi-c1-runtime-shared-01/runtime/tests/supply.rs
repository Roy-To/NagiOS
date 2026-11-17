#![forbid(unsafe_code)]

#[path = "../../consumer/src/fixture.rs"]
#[allow(dead_code)]
mod fixture;

use consumer::session_binding::AdmissionFailure;
use fixture::{Fixture, FixtureCaller};
use identity::{
    CurrentIdentityResolver, ProfileId, ProfileRootAuthorizer, ProfileStorageBackend,
    ResolutionError, ResolvedIdentity, StorageRootFailure, TrustedCallerContext, UserId,
    UserStorageRootAdapter,
};
use model::ObjectId;
use nagi_init_session_supply_candidate::{supply_current, SupplyFailure};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use svc::CancellationToken;

// Test-only backend and decision. Neither is a production VFS/consent provider.
struct Roots {
    calls: Arc<AtomicUsize>,
    hook: Option<Box<dyn FnOnce()>>,
    drops: Arc<AtomicUsize>,
}
impl Drop for Roots {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
impl ProfileStorageBackend for Roots {
    type Root = ProfileId;
    fn open_profile_root(
        &mut self,
        _: Option<UserId>,
        profile: &ProfileId,
    ) -> Result<Self::Root, StorageRootFailure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(hook) = self.hook.take() {
            hook();
        }
        Ok(profile.clone())
    }
    fn read_object(&mut self, _: &Self::Root, _: ObjectId) -> Result<Vec<u8>, StorageRootFailure> {
        Err(StorageRootFailure::Unavailable)
    }
    fn write_object(
        &mut self,
        _: &Self::Root,
        _: ObjectId,
        _: &[u8],
    ) -> Result<(), StorageRootFailure> {
        Err(StorageRootFailure::Unavailable)
    }
}

struct Decision {
    calls: usize,
    result: Result<(), StorageRootFailure>,
    hook: Option<Box<dyn FnOnce()>>,
}
impl Decision {
    fn new(result: Result<(), StorageRootFailure>) -> Self {
        Self {
            calls: 0,
            result,
            hook: None,
        }
    }
}
impl ProfileRootAuthorizer for Decision {
    fn authorize_profile_root(
        &mut self,
        _: &dyn TrustedCallerContext,
        identity: &ResolvedIdentity,
        profile: &ProfileId,
    ) -> Result<(), StorageRootFailure> {
        self.calls += 1;
        assert_eq!(profile, identity.profile_id());
        if let Some(hook) = self.hook.take() {
            hook();
        }
        self.result
    }
}

fn roots(
    hook: Option<Box<dyn FnOnce()>>,
) -> (
    UserStorageRootAdapter<Roots>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
) {
    let calls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    (
        UserStorageRootAdapter::new(Roots {
            calls: calls.clone(),
            hook,
            drops: drops.clone(),
        }),
        calls,
        drops,
    )
}

#[test]
fn current_session_and_separate_profile_decision_supply_actual_sdk_handle() {
    let mut fixture = Fixture::new();
    let (roots, calls, drops) = roots(None);
    let mut decision = Decision::new(Ok(()));
    let mut supplied = supply_current(
        fixture.issuer.as_mut().unwrap(),
        &fixture.caller,
        CancellationToken::new(),
        roots,
        &mut decision,
    )
    .unwrap();
    {
        let (session, _root, _roots) = supplied.parts();
        let _access = session.access_context().unwrap();
    }
    assert_eq!(
        supplied.session().identity().unwrap().profile_id(),
        fixture.identity.profile_id()
    );
    assert_eq!(decision.calls, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(supplied);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn profile_denied_or_unavailable_never_opens_backend() {
    for reason in [StorageRootFailure::Denied, StorageRootFailure::Unavailable] {
        let mut fixture = Fixture::new();
        let (roots, calls, drops) = roots(None);
        let mut decision = Decision::new(Err(reason));
        let result = supply_current(
            fixture.issuer.as_mut().unwrap(),
            &fixture.caller,
            CancellationToken::new(),
            roots,
            &mut decision,
        );
        assert!(matches!(result, Err(SupplyFailure::Profile(found)) if found == reason));
        assert_eq!(decision.calls, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn no_current_login_does_not_reach_profile_policy_or_backend() {
    // Fresh real service, recovered but never logged in. Providers below are
    // explicitly the shared test-only providers, not production authentication.
    let caller = FixtureCaller::new("fixture.echo-client", "caller_fixture");
    let mut issuer = identity::IdentityService::new(
        identity::InMemorySnapshotStore::default(),
        fixture::FixtureIds::default(),
        fixture::FixtureClock,
        fixture::FixtureIdentityPolicy,
        fixture::NoGuestFixture,
        identity::NoopIdentityEventSink,
    );
    issuer.recover(&caller).unwrap();
    let (roots, calls, drops) = roots(None);
    let mut decision = Decision::new(Ok(()));
    let result = supply_current(
        &mut issuer,
        &caller,
        CancellationToken::new(),
        roots,
        &mut decision,
    );
    assert!(matches!(
        result,
        Err(SupplyFailure::Session(AdmissionFailure::Resolution(
            ResolutionError::NoSession
        )))
    ));
    assert_eq!(decision.calls, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn resolver_denial_and_preexisting_cancellation_do_not_reach_profile_policy() {
    let mut fixture = Fixture::new();
    let other = FixtureCaller::new("fixture.other", "caller_fixture");
    let (roots, calls, drops) = roots(None);
    let mut decision = Decision::new(Ok(()));
    let result = supply_current(
        fixture.issuer.as_mut().unwrap(),
        &other,
        CancellationToken::new(),
        roots,
        &mut decision,
    );
    assert!(matches!(
        result,
        Err(SupplyFailure::Session(AdmissionFailure::Resolution(
            ResolutionError::Denied
        )))
    ));
    assert_eq!(decision.calls, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let (roots, calls, drops) = self::roots(None);
    let token = CancellationToken::new();
    token.cancel();
    let result = supply_current(
        fixture.issuer.as_mut().unwrap(),
        &fixture.caller,
        token,
        roots,
        &mut decision,
    );
    assert!(matches!(
        result,
        Err(SupplyFailure::Session(AdmissionFailure::Cancelled))
    ));
    assert_eq!(decision.calls, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

// Resolve through the real issuer, then let later callbacks own/drop it.
struct TakeIssuer(Rc<RefCell<Option<fixture::Issuer>>>);
impl CurrentIdentityResolver for TakeIssuer {
    fn resolve_current_identity(
        &mut self,
        caller: &impl TrustedCallerContext,
    ) -> Result<ResolvedIdentity, ResolutionError> {
        self.0
            .borrow_mut()
            .as_mut()
            .unwrap()
            .resolve_current_identity(caller)
    }
}

#[test]
fn revocation_in_profile_decision_prevents_backend_entry() {
    let mut fixture = Fixture::new();
    let owner = Rc::new(RefCell::new(fixture.issuer.take()));
    let from_policy = owner.clone();
    let mut resolver = TakeIssuer(owner);
    let (roots, calls, drops) = roots(None);
    let mut decision = Decision::new(Ok(()));
    decision.hook = Some(Box::new(move || drop(from_policy.borrow_mut().take())));
    let result = supply_current(
        &mut resolver,
        &fixture.caller,
        CancellationToken::new(),
        roots,
        &mut decision,
    );
    assert!(matches!(
        result,
        Err(SupplyFailure::Profile(StorageRootFailure::StaleSession))
    ));
    assert_eq!(decision.calls, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn issuer_loss_in_backend_does_not_deliver_stale_supply() {
    let mut fixture = Fixture::new();
    let owner = Rc::new(RefCell::new(fixture.issuer.take()));
    let from_backend = owner.clone();
    let mut resolver = TakeIssuer(owner);
    let (roots, calls, drops) = roots(Some(Box::new(move || {
        drop(from_backend.borrow_mut().take())
    })));
    let mut decision = Decision::new(Ok(()));
    let result = supply_current(
        &mut resolver,
        &fixture.caller,
        CancellationToken::new(),
        roots,
        &mut decision,
    );
    assert!(matches!(
        result,
        Err(SupplyFailure::Session(AdmissionFailure::StaleIdentity))
    ));
    assert_eq!(decision.calls, 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn caller_switch_or_cancellation_during_open_prevents_delivery() {
    for principal in [true, false] {
        let mut fixture = Fixture::new();
        let switch = fixture.caller.arm_change(principal);
        let (roots, calls, drops) =
            roots(Some(Box::new(move || switch.store(true, Ordering::SeqCst))));
        let mut decision = Decision::new(Ok(()));
        let result = supply_current(
            fixture.issuer.as_mut().unwrap(),
            &fixture.caller,
            CancellationToken::new(),
            roots,
            &mut decision,
        );
        assert!(matches!(
            result,
            Err(SupplyFailure::Session(AdmissionFailure::CallerMismatch))
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
    let mut fixture = Fixture::new();
    let token = CancellationToken::new();
    let in_backend = token.clone();
    let (roots, calls, drops) = roots(Some(Box::new(move || in_backend.cancel())));
    let mut decision = Decision::new(Ok(()));
    let result = supply_current(
        fixture.issuer.as_mut().unwrap(),
        &fixture.caller,
        token,
        roots,
        &mut decision,
    );
    assert!(matches!(
        result,
        Err(SupplyFailure::Session(AdmissionFailure::Cancelled))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
