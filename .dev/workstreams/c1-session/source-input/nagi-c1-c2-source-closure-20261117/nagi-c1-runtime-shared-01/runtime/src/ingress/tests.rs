use super::*;
use crate::session_fixture::{poll_ready, Fixture, Issuer};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Wake, Waker};
use svc::{
    AuthorizationContext, AuthorizationDecision, AuthorizationPolicy, ContractVersion,
    InProcessReferenceTransport, IpcErrorCode, OperationId, ProviderFuture, RequestId,
    ServiceDescriptor, ServiceId, ServiceOperation, ServiceProvider, ServiceRegistry,
};

fn request() -> RequestEnvelope {
    RequestEnvelope::new(
        RequestId::new(1).unwrap(),
        None,
        None,
        ServiceId::new("fixture.echo").unwrap(),
        ContractVersion::new(1, 0).unwrap(),
        OperationId::new("echo").unwrap(),
        b"init ingress fixture".to_vec(),
    )
    .unwrap()
}

#[derive(Clone, Default)]
struct Counts {
    policy: Arc<AtomicUsize>,
    provider: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}
struct DropCount(Arc<AtomicUsize>);
impl Drop for DropCount {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
struct Provider {
    descriptor: ServiceDescriptor,
    counts: Counts,
    pending: bool,
}
impl ServiceProvider for Provider {
    fn descriptor(&self) -> &ServiceDescriptor {
        &self.descriptor
    }
    fn handle<'a>(
        &'a self,
        request: RequestEnvelope,
        context: svc::CallContext,
    ) -> ProviderFuture<'a> {
        self.counts.provider.fetch_add(1, Ordering::SeqCst);
        let drop_count = DropCount(self.counts.drops.clone());
        Box::pin(async move {
            let _drop_count = drop_count;
            if self.pending {
                context.cancellation().cancelled().await;
            }
            Ok(request.payload().to_vec())
        })
    }
}
type Hook = Arc<dyn Fn() + Send + Sync>;
struct Policy {
    counts: Counts,
    decision: AuthorizationDecision,
    hook: Option<Hook>,
}
impl AuthorizationPolicy for Policy {
    fn authorize(&self, context: &AuthorizationContext) -> AuthorizationDecision {
        self.counts.policy.fetch_add(1, Ordering::SeqCst);
        assert_eq!(context.caller().as_str(), "fixture.echo-client");
        if let Some(hook) = &self.hook {
            hook();
        }
        self.decision
    }
}
fn client(
    pending: bool,
    decision: AuthorizationDecision,
    hook: Option<Hook>,
) -> (ServiceClient, Counts) {
    let counts = Counts::default();
    let registry = Arc::new(ServiceRegistry::new());
    registry
        .register(Arc::new(Provider {
            descriptor: ServiceDescriptor::new(
                ServiceId::new("fixture.echo").unwrap(),
                [ContractVersion::new(1, 0).unwrap()],
                [ServiceOperation::new(
                    OperationId::new("echo").unwrap(),
                    None,
                )],
            )
            .unwrap(),
            counts: counts.clone(),
            pending,
        }))
        .unwrap();
    // Test-only trusted runtime association. Never derived from a captured
    // principal string or presented as production peer authentication.
    let transport = InProcessReferenceTransport::new(
        registry,
        Arc::new(Policy {
            counts: counts.clone(),
            decision,
            hook,
        }),
        svc::PrincipalId::new("fixture.echo-client").unwrap(),
    );
    (ServiceClient::new(Arc::new(transport)), counts)
}
struct WakeCount(AtomicUsize);
impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn ingress_live_and_denied_calls_preserve_real_transport_authorization() {
    for allow in [true, false] {
        let mut fixture = Fixture::new();
        let decision = if allow {
            AuthorizationDecision::Allow
        } else {
            AuthorizationDecision::Deny
        };
        let (client, counts) = client(false, decision, None);
        let ingress = RuntimeIngress::supply(
            fixture.issuer.as_mut().unwrap(),
            &fixture.caller,
            &client,
            CancellationToken::new(),
        )
        .unwrap();
        let response = poll_ready(ingress.call(request()).unwrap()).unwrap();
        if allow {
            assert_eq!(response.result().unwrap(), b"init ingress fixture");
        } else {
            assert_eq!(
                response.result().unwrap_err().code(),
                IpcErrorCode::PermissionDenied
            );
        }
        assert_eq!(counts.policy.load(Ordering::SeqCst), 1);
        assert_eq!(counts.provider.load(Ordering::SeqCst), usize::from(allow));
        assert_eq!(counts.drops.load(Ordering::SeqCst), usize::from(allow));
    }
}

#[test]
fn ingress_revocation_before_first_poll_never_enters_authorizer_or_provider() {
    for cancelled in [true, false] {
        let mut fixture = Fixture::new();
        let (client, counts) = client(false, AuthorizationDecision::Allow, None);
        let ingress = RuntimeIngress::supply(
            fixture.issuer.as_mut().unwrap(),
            &fixture.caller,
            &client,
            CancellationToken::new(),
        )
        .unwrap();
        let future = ingress.call(request()).unwrap();
        assert_eq!(counts.policy.load(Ordering::SeqCst), 0);
        if cancelled {
            ingress.revoke();
        } else {
            drop(fixture.issuer.take());
        }
        let expected = if cancelled {
            AdmissionFailure::Cancelled
        } else {
            AdmissionFailure::StaleIdentity
        };
        assert!(matches!(poll_ready(future), Err(reason) if reason == expected));
        assert!(matches!(ingress.call(request()), Err(reason) if reason == expected));
        assert_eq!(counts.policy.load(Ordering::SeqCst), 0);
        assert_eq!(counts.provider.load(Ordering::SeqCst), 0);
        assert_eq!(counts.drops.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn ingress_pending_caller_change_drops_provider_future_and_waiter() {
    for principal in [true, false] {
        let mut fixture = Fixture::new();
        let switch = fixture.caller.arm_change(principal);
        let (client, counts) = client(true, AuthorizationDecision::Allow, None);
        let ingress = RuntimeIngress::supply(
            fixture.issuer.as_mut().unwrap(),
            &fixture.caller,
            &client,
            CancellationToken::new(),
        )
        .unwrap();
        let mut future = Box::pin(ingress.call(request()).unwrap());
        let wakes = Arc::new(WakeCount(AtomicUsize::new(0)));
        let waker = Waker::from(wakes.clone());
        let mut context = Context::from_waker(&waker);
        assert!(future.as_mut().poll(&mut context).is_pending());
        assert_eq!(counts.policy.load(Ordering::SeqCst), 1);
        assert_eq!(counts.provider.load(Ordering::SeqCst), 1);
        assert_eq!(counts.drops.load(Ordering::SeqCst), 0);
        switch.store(true, Ordering::SeqCst);
        assert!(matches!(
            future.as_mut().poll(&mut context),
            Poll::Ready(Err(AdmissionFailure::CallerMismatch))
        ));
        assert_eq!(counts.drops.load(Ordering::SeqCst), 1);
        ingress.revoke();
        assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
        assert_eq!(counts.provider.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn ingress_pending_issuer_loss_has_no_automatic_wake_but_next_poll_rejects() {
    let mut fixture = Fixture::new();
    let (client, counts) = client(true, AuthorizationDecision::Allow, None);
    let ingress = RuntimeIngress::supply(
        fixture.issuer.as_mut().unwrap(),
        &fixture.caller,
        &client,
        CancellationToken::new(),
    )
    .unwrap();
    let mut future = Box::pin(ingress.call(request()).unwrap());
    let wakes = Arc::new(WakeCount(AtomicUsize::new(0)));
    let waker = Waker::from(wakes.clone());
    let mut context = Context::from_waker(&waker);
    assert!(future.as_mut().poll(&mut context).is_pending());
    drop(fixture.issuer.take());
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
    assert!(matches!(
        future.as_mut().poll(&mut context),
        Poll::Ready(Err(AdmissionFailure::StaleIdentity))
    ));
    assert_eq!(counts.provider.load(Ordering::SeqCst), 1);
    assert_eq!(counts.drops.load(Ordering::SeqCst), 1);
    ingress.revoke();
    assert_eq!(wakes.0.load(Ordering::SeqCst), 0);
}

#[test]
fn ingress_cancel_inside_svc_authorization_prevents_provider_dispatch() {
    let mut fixture = Fixture::new();
    let token = CancellationToken::new();
    let in_policy = token.clone();
    let (client, counts) = client(
        false,
        AuthorizationDecision::Allow,
        Some(Arc::new(move || in_policy.cancel())),
    );
    let ingress = RuntimeIngress::supply(
        fixture.issuer.as_mut().unwrap(),
        &fixture.caller,
        &client,
        token,
    )
    .unwrap();
    assert!(matches!(
        poll_ready(ingress.call(request()).unwrap()),
        Err(AdmissionFailure::Cancelled)
    ));
    assert_eq!(counts.policy.load(Ordering::SeqCst), 1);
    assert_eq!(counts.provider.load(Ordering::SeqCst), 0);
    assert_eq!(counts.drops.load(Ordering::SeqCst), 0);
}

#[test]
fn ingress_issuer_loss_inside_authorization_exposes_non_atomic_dispatch_limit() {
    let mut fixture = Fixture::new();
    let owner = Arc::new(Mutex::new(None::<Issuer>));
    let from_policy = owner.clone();
    let (client, counts) = client(
        false,
        AuthorizationDecision::Allow,
        Some(Arc::new(move || drop(from_policy.lock().unwrap().take()))),
    );
    let ingress = RuntimeIngress::supply(
        fixture.issuer.as_mut().unwrap(),
        &fixture.caller,
        &client,
        CancellationToken::new(),
    )
    .unwrap();
    *owner.lock().unwrap() = fixture.issuer.take();
    assert!(matches!(
        poll_ready(ingress.call(request()).unwrap()),
        Err(AdmissionFailure::StaleIdentity)
    ));
    // A poll-boundary guard detects the loss, but cannot prevent the real
    // transport's provider entry between its authorization and this check.
    assert_eq!(counts.policy.load(Ordering::SeqCst), 1);
    assert_eq!(counts.provider.load(Ordering::SeqCst), 1);
    assert_eq!(counts.drops.load(Ordering::SeqCst), 1);
    assert!(matches!(
        ingress.call(request()),
        Err(AdmissionFailure::StaleIdentity)
    ));
    assert_eq!(counts.provider.load(Ordering::SeqCst), 1);
}

#[test]
fn ingress_revoke_wakes_pending_work_and_drops_only_the_local_provider_future() {
    let mut fixture = Fixture::new();
    let (client, counts) = client(true, AuthorizationDecision::Allow, None);
    let ingress = RuntimeIngress::supply(
        fixture.issuer.as_mut().unwrap(),
        &fixture.caller,
        &client,
        CancellationToken::new(),
    )
    .unwrap();
    let mut future = Box::pin(ingress.call(request()).unwrap());
    let wakes = Arc::new(WakeCount(AtomicUsize::new(0)));
    let waker = Waker::from(wakes.clone());
    let mut context = Context::from_waker(&waker);
    assert!(future.as_mut().poll(&mut context).is_pending());
    ingress.revoke();
    assert_eq!(wakes.0.load(Ordering::SeqCst), 1);
    assert!(matches!(
        future.as_mut().poll(&mut context),
        Poll::Ready(Err(AdmissionFailure::Cancelled))
    ));
    assert_eq!(counts.policy.load(Ordering::SeqCst), 1);
    assert_eq!(counts.provider.load(Ordering::SeqCst), 1);
    assert_eq!(counts.drops.load(Ordering::SeqCst), 1);
    assert!(fixture.identity.is_live());
}
