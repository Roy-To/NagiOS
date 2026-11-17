#![forbid(unsafe_code)]

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use identity::{
    AuthorizationFailure, AuthorizationRequest, CallerContextId, Clock, CurrentIdentityResolver,
    GuestCleanupFailure, GuestProfileCleaner, IdSourceFailure, IdentityAuthorizer,
    IdentityIdSource, IdentityService, InMemorySnapshotStore, LocalUserMetadata,
    NoopIdentityEventSink, ProfileId, ProfileMetadata, ResolvedIdentity, SessionId,
    TrustedCallerContext, UnixMillis, UserId,
};
use svc::{
    AuthorizationContext, AuthorizationDecision, AuthorizationPolicy, CancellationToken,
    ContractVersion, InProcessReferenceTransport, OperationId, ProviderFuture, RequestEnvelope,
    RequestId, ServiceClient, ServiceDescriptor, ServiceId, ServiceOperation, ServiceProvider,
    ServiceRegistry, ServiceTransport,
};

// These values and providers are development fixtures, not authenticated peers.
pub(super) struct FixtureCaller {
    principal: identity::PrincipalId,
    context: CallerContextId,
    alternate: Option<(identity::PrincipalId, CallerContextId)>,
    switched: Arc<AtomicBool>,
}
impl FixtureCaller {
    pub(super) fn new(principal: &str, context: &str) -> Self {
        Self {
            principal: identity::PrincipalId::new(principal).unwrap(),
            context: CallerContextId::new(context).unwrap(),
            alternate: None,
            switched: Arc::new(AtomicBool::new(false)),
        }
    }
    #[cfg(test)]
    pub(super) fn arm_change(&mut self, principal: bool) -> Arc<AtomicBool> {
        self.alternate = Some((
            if principal {
                identity::PrincipalId::new("fixture.other").unwrap()
            } else {
                self.principal.clone()
            },
            if principal {
                self.context.clone()
            } else {
                CallerContextId::new("caller_other").unwrap()
            },
        ));
        self.switched.clone()
    }
}
impl TrustedCallerContext for FixtureCaller {
    fn principal_id(&self) -> &identity::PrincipalId {
        if self.switched.load(Ordering::SeqCst) {
            if let Some((principal, _)) = &self.alternate {
                return principal;
            }
        }
        &self.principal
    }
    fn context_id(&self) -> &CallerContextId {
        if self.switched.load(Ordering::SeqCst) {
            if let Some((_, context)) = &self.alternate {
                return context;
            }
        }
        &self.context
    }
}

// Same sized borrowed view demonstrated by e641; never reads caller from data.
struct BorrowedCaller<'a>(&'a dyn TrustedCallerContext);
impl TrustedCallerContext for BorrowedCaller<'_> {
    fn principal_id(&self) -> &identity::PrincipalId {
        self.0.principal_id()
    }
    fn context_id(&self) -> &CallerContextId {
        self.0.context_id()
    }
}

#[derive(Clone)]
pub(super) struct FixtureClock;
impl Clock for FixtureClock {
    fn now(&self) -> UnixMillis {
        UnixMillis::new(100)
    }
}
#[derive(Default)]
pub(super) struct FixtureIds {
    user: u64,
    profile: u64,
    session: u64,
}
impl IdentityIdSource for FixtureIds {
    fn next_user_id(&mut self) -> Result<UserId, IdSourceFailure> {
        self.user += 1;
        UserId::new(self.user).map_err(|_| IdSourceFailure::Unavailable)
    }
    fn next_profile_id(&mut self) -> Result<ProfileId, IdSourceFailure> {
        self.profile += 1;
        ProfileId::new(format!("profile_fixture{}", self.profile))
            .map_err(|_| IdSourceFailure::Unavailable)
    }
    fn next_session_id(&mut self) -> Result<SessionId, IdSourceFailure> {
        self.session += 1;
        SessionId::new(format!("session_fixture{}", self.session))
            .map_err(|_| IdSourceFailure::Unavailable)
    }
}
pub(super) struct FixtureIdentityPolicy;
impl IdentityAuthorizer for FixtureIdentityPolicy {
    fn authorize(&mut self, request: &AuthorizationRequest) -> Result<(), AuthorizationFailure> {
        if request.caller_principal.as_str() == "fixture.echo-client"
            && request.caller_context.as_str() == "caller_fixture"
        {
            Ok(())
        } else {
            Err(AuthorizationFailure::Denied)
        }
    }
}
pub(super) struct NoGuestFixture;
impl GuestProfileCleaner for NoGuestFixture {
    fn cleanup_guest_profile(&mut self, _: &ProfileId) -> Result<(), GuestCleanupFailure> {
        Err(GuestCleanupFailure::Unavailable)
    }
}
pub(super) type Issuer = IdentityService<
    InMemorySnapshotStore,
    FixtureIds,
    FixtureClock,
    FixtureIdentityPolicy,
    NoGuestFixture,
    NoopIdentityEventSink,
>;

pub(super) type Trace = Arc<Mutex<Vec<&'static str>>>;
pub(super) type PolicyHook = Arc<dyn Fn() + Send + Sync>;
pub(super) type PolicyHookSlot = Arc<Mutex<Option<PolicyHook>>>;

struct EchoProvider {
    descriptor: ServiceDescriptor,
    calls: Arc<AtomicUsize>,
    trace: Trace,
}
impl ServiceProvider for EchoProvider {
    fn descriptor(&self) -> &ServiceDescriptor {
        &self.descriptor
    }
    fn handle<'a>(&'a self, request: RequestEnvelope, _: svc::CallContext) -> ProviderFuture<'a> {
        self.trace.lock().unwrap().push("provider");
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { Ok(request.payload().to_vec()) })
    }
}
struct FixtureServicePolicy {
    calls: Arc<AtomicUsize>,
    hook: PolicyHookSlot,
    trace: Trace,
}
impl AuthorizationPolicy for FixtureServicePolicy {
    fn authorize(&self, context: &AuthorizationContext) -> AuthorizationDecision {
        self.trace.lock().unwrap().push("policy");
        self.calls.fetch_add(1, Ordering::SeqCst);
        let hook = self.hook.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook();
            self.trace.lock().unwrap().push("hook");
        }
        if context.caller().as_str() == "fixture.echo-client"
            && context.service_id().as_str() == "fixture.echo"
            && context.operation_id().as_str() == "echo"
        {
            AuthorizationDecision::Allow
        } else {
            AuthorizationDecision::Deny
        }
    }
}

pub(super) struct Fixture {
    pub(super) issuer: Option<Issuer>,
    pub(super) caller: FixtureCaller,
    pub(super) identity: ResolvedIdentity,
    pub(super) client: ServiceClient,
    pub(super) provider_calls: Arc<AtomicUsize>,
    pub(super) policy_calls: Arc<AtomicUsize>,
    #[cfg(test)]
    pub(super) policy_hook: PolicyHookSlot,
    pub(super) trace: Trace,
}
impl Fixture {
    pub(super) fn new() -> Self {
        let caller = FixtureCaller::new("fixture.echo-client", "caller_fixture");
        let borrowed = BorrowedCaller(&caller);
        let mut issuer = IdentityService::new(
            InMemorySnapshotStore::default(),
            FixtureIds::default(),
            FixtureClock,
            FixtureIdentityPolicy,
            NoGuestFixture,
            NoopIdentityEventSink,
        );
        issuer.recover(&borrowed).unwrap();
        let (user, profile) = issuer
            .create_local_user(
                &borrowed,
                LocalUserMetadata::new("Fixture User").unwrap(),
                ProfileMetadata::new("Fixture Profile", None).unwrap(),
            )
            .unwrap();
        issuer
            .login_local(
                &borrowed,
                user,
                profile,
                Some(UnixMillis::new(200)),
                identity::IdempotencyKey::new("fixture-login").unwrap(),
            )
            .unwrap();
        let identity = issuer.resolve_current_identity(&borrowed).unwrap();
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let policy_calls = Arc::new(AtomicUsize::new(0));
        let policy_hook = Arc::new(Mutex::new(None));
        let trace = Arc::new(Mutex::new(Vec::new()));
        let registry = Arc::new(ServiceRegistry::new());
        registry
            .register(Arc::new(EchoProvider {
                descriptor: ServiceDescriptor::new(
                    ServiceId::new("fixture.echo").unwrap(),
                    [ContractVersion::new(1, 0).unwrap()],
                    [ServiceOperation::new(
                        OperationId::new("echo").unwrap(),
                        None,
                    )],
                )
                .unwrap(),
                calls: provider_calls.clone(),
                trace: trace.clone(),
            }))
            .unwrap();
        let transport = InProcessReferenceTransport::new(
            registry,
            Arc::new(FixtureServicePolicy {
                calls: policy_calls.clone(),
                hook: policy_hook.clone(),
                trace: trace.clone(),
            }),
            // Supplied independently by fixture runtime, not converted from evidence.
            svc::PrincipalId::new("fixture.echo-client").unwrap(),
        );
        let client = ServiceClient::new(Arc::new(transport) as Arc<dyn ServiceTransport>);
        Self {
            issuer: Some(issuer),
            caller,
            identity,
            client,
            provider_calls,
            policy_calls,
            #[cfg(test)]
            policy_hook,
            trace,
        }
    }
}

#[derive(Debug, PartialEq)]
pub(super) enum FixtureFailure {
    StaleIdentity,
    CallerMismatch,
    Service(svc::IpcErrorCode),
}
pub(super) fn poll_ready<T>(future: impl Future<Output = T>) -> T {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut future = std::pin::pin!(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("this fixture echo must complete immediately"),
    }
}
pub(super) fn prepare_echo<'a>(
    identity: &ResolvedIdentity,
    runtime: &dyn TrustedCallerContext,
    client: &'a ServiceClient,
) -> Result<svc::ServiceFuture<'a>, FixtureFailure> {
    // Point-in-time admission only: no operation lease or atomic dispatch claim.
    let caller = BorrowedCaller(runtime);
    let live = identity.is_live();
    if identity.principal_id() != caller.principal_id()
        || identity.caller_context_id() != caller.context_id()
    {
        return Err(FixtureFailure::CallerMismatch);
    }
    if !live {
        return Err(FixtureFailure::StaleIdentity);
    }
    let request = RequestEnvelope::new(
        RequestId::new(1).unwrap(),
        None,
        None,
        ServiceId::new("fixture.echo").unwrap(),
        ContractVersion::new(1, 0).unwrap(),
        OperationId::new("echo").unwrap(),
        b"combined fixture echo".to_vec(),
    )
    .unwrap();
    // This real public future borrows only the client. It holds no Identity
    // operation lease and does not revalidate Identity when later polled.
    Ok(client.call(request, CancellationToken::new()))
}
pub(super) fn finish_echo(response: svc::ResponseEnvelope) -> Result<Vec<u8>, FixtureFailure> {
    response
        .result()
        .map(|payload| payload.to_vec())
        .map_err(|error| FixtureFailure::Service(error.code()))
}

pub(super) fn guarded_echo(
    identity: &ResolvedIdentity,
    runtime: &dyn TrustedCallerContext,
    client: &ServiceClient,
) -> Result<Vec<u8>, FixtureFailure> {
    finish_echo(poll_ready(prepare_echo(identity, runtime, client)?))
}
