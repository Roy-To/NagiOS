use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::Duration;

use nagi_service_contract::{
    AuthorizationContext, AuthorizationDecision, AuthorizationPolicy, CancellationToken,
    CapabilityId, ContractVersion, CorrelationId, InProcessReferenceTransport, IpcError,
    IpcErrorCode, OperationId, PrincipalId, ProviderError, ProviderErrorCode, ProviderFuture,
    RequestEnvelope, RequestId, ServiceAvailability, ServiceClient, ServiceDescriptor,
    ServiceDiscovery, ServiceId, ServiceOperation, ServiceProvider, ServiceRegistry,
    ServiceTransport, TraceId, MAX_SERVICE_PAYLOAD_BYTES,
};

#[derive(Clone, Copy)]
enum ProviderBehavior {
    Echo,
    RejectEmpty,
    Fail,
    CancelWhileHandling,
    WaitForCancellation,
    OversizedResponse,
}

struct TestService {
    descriptor: ServiceDescriptor,
    calls: AtomicUsize,
    behavior: ProviderBehavior,
    availability: ServiceAvailability,
    started: Option<mpsc::Sender<()>>,
}

impl TestService {
    fn new(behavior: ProviderBehavior) -> Self {
        Self::with_availability(behavior, ServiceAvailability::Available)
    }

    fn with_availability(behavior: ProviderBehavior, availability: ServiceAvailability) -> Self {
        Self::with_started(behavior, availability, None)
    }

    fn with_started(
        behavior: ProviderBehavior,
        availability: ServiceAvailability,
        started: Option<mpsc::Sender<()>>,
    ) -> Self {
        let version = version(1, 0);
        let operation = ServiceOperation::new(
            operation("echo"),
            Some(CapabilityId::new("system.echo.call").expect("capability id")),
        );
        let descriptor = ServiceDescriptor::new(service_id(), [version], [operation])
            .expect("valid service descriptor");
        Self {
            descriptor,
            calls: AtomicUsize::new(0),
            behavior,
            availability,
            started,
        }
    }
}

impl ServiceProvider for TestService {
    fn descriptor(&self) -> &ServiceDescriptor {
        &self.descriptor
    }

    fn availability(&self) -> ServiceAvailability {
        self.availability
    }

    fn handle<'a>(
        &'a self,
        request: RequestEnvelope,
        context: nagi_service_contract::CallContext,
    ) -> ProviderFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(started) = &self.started {
            let _ = started.send(());
        }
        let behavior = self.behavior;
        let cancellation = context.cancellation().clone();
        Box::pin(async move {
            match behavior {
                ProviderBehavior::Echo => Ok(request.payload().to_vec()),
                ProviderBehavior::RejectEmpty if request.payload().is_empty() => {
                    Err(ProviderError::new(ProviderErrorCode::InvalidRequest))
                }
                ProviderBehavior::RejectEmpty => Ok(request.payload().to_vec()),
                ProviderBehavior::Fail => Err(ProviderError::new(ProviderErrorCode::Failure)),
                ProviderBehavior::CancelWhileHandling => {
                    cancellation.cancel();
                    cancellation.cancelled().await;
                    Err(ProviderError::new(ProviderErrorCode::Cancelled))
                }
                ProviderBehavior::WaitForCancellation => {
                    cancellation.cancelled().await;
                    Ok(Vec::new())
                }
                ProviderBehavior::OversizedResponse => Ok(vec![0; MAX_SERVICE_PAYLOAD_BYTES + 1]),
            }
        })
    }
}

#[derive(Clone, Copy)]
struct TestPolicy(AuthorizationDecision);

impl AuthorizationPolicy for TestPolicy {
    fn authorize(&self, _context: &AuthorizationContext) -> AuthorizationDecision {
        self.0
    }
}

struct RecordingPolicy {
    decision: AuthorizationDecision,
    contexts: Mutex<Vec<AuthorizationContext>>,
}

impl AuthorizationPolicy for RecordingPolicy {
    fn authorize(&self, context: &AuthorizationContext) -> AuthorizationDecision {
        self.contexts
            .lock()
            .expect("policy context lock")
            .push(context.clone());
        self.decision
    }
}

fn setup(
    behavior: ProviderBehavior,
    decision: AuthorizationDecision,
) -> (
    Arc<ServiceRegistry>,
    Arc<TestService>,
    Arc<InProcessReferenceTransport>,
    ServiceClient,
) {
    setup_with_policy(behavior, Arc::new(TestPolicy(decision)))
}

fn setup_with_policy(
    behavior: ProviderBehavior,
    policy: Arc<dyn AuthorizationPolicy>,
) -> (
    Arc<ServiceRegistry>,
    Arc<TestService>,
    Arc<InProcessReferenceTransport>,
    ServiceClient,
) {
    let registry = Arc::new(ServiceRegistry::new());
    let provider = Arc::new(TestService::new(behavior));
    registry
        .register(provider.clone() as Arc<dyn ServiceProvider>)
        .expect("provider registration");
    let transport = Arc::new(InProcessReferenceTransport::new(
        Arc::clone(&registry),
        policy,
        test_caller(),
    ));
    let client = ServiceClient::new(Arc::clone(&transport) as Arc<dyn ServiceTransport>);
    (registry, provider, transport, client)
}

fn request(request_id: u128, operation_id: &str, payload: &[u8]) -> RequestEnvelope {
    request_to(
        request_id,
        service_id(),
        version(1, 0),
        operation_id,
        payload,
    )
}

fn request_to(
    request_id: u128,
    service: ServiceId,
    contract: ContractVersion,
    operation_id: &str,
    payload: &[u8],
) -> RequestEnvelope {
    RequestEnvelope::new(
        RequestId::new(request_id).expect("nonzero request id"),
        Some(CorrelationId::new(77).expect("nonzero correlation id")),
        Some(TraceId::new(88).expect("nonzero trace id")),
        service,
        contract,
        operation(operation_id),
        payload.to_vec(),
    )
    .expect("valid request")
}

fn service_id() -> ServiceId {
    ServiceId::new("example.echo").expect("service id")
}

fn test_caller() -> PrincipalId {
    PrincipalId::new("example.echo-client").expect("principal id")
}

fn operation(value: &str) -> OperationId {
    OperationId::new(value).expect("operation id")
}

fn version(major: u16, minor: u16) -> ContractVersion {
    ContractVersion::new(major, minor).expect("contract version")
}

fn block_on<T>(future: impl Future<Output = T>) -> T {
    struct ThreadWake(thread::Thread);
    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::park(),
        }
    }
}

#[test]
fn identifiers_are_validated_canonical_and_never_paths() {
    assert_eq!(service_id().as_str(), "example.echo");
    for invalid in [
        "",
        ".echo",
        "echo.",
        "example..echo",
        "Example.echo",
        "../echo",
        "a/b",
        "echo\\service",
    ] {
        assert!(
            ServiceId::new(invalid).is_err(),
            "accepted service id {invalid}"
        );
    }
    assert!(ServiceId::new("a".repeat(129)).is_err());
    assert!(OperationId::new("echo_request").is_ok());
    for invalid in [
        "",
        "Echo",
        "echo.",
        "echo/name",
        "_private",
        "echo__request",
    ] {
        assert!(
            OperationId::new(invalid).is_err(),
            "accepted operation id {invalid}"
        );
    }
    assert_eq!(RequestId::new(0), None);
    assert_eq!(CorrelationId::new(0), None);
    assert!(ContractVersion::new(0, 1).is_err());
}

#[test]
fn descriptor_requires_versions_and_unique_operations() {
    let op = ServiceOperation::new(operation("echo"), None);
    assert!(ServiceDescriptor::new(service_id(), [], [op.clone()]).is_err());
    assert!(ServiceDescriptor::new(service_id(), [version(1, 0)], []).is_err());
    assert_eq!(
        ServiceDescriptor::new(service_id(), [version(1, 0), version(1, 0)], [op.clone()])
            .unwrap_err(),
        nagi_service_contract::DescriptorError::DuplicateContractVersion
    );
    assert!(ServiceDescriptor::new(service_id(), [version(1, 0)], [op.clone(), op]).is_err());
}

#[test]
fn request_payload_is_bounded_at_construction() {
    let built = RequestEnvelope::new(
        RequestId::new(2).unwrap(),
        None,
        None,
        service_id(),
        version(1, 0),
        operation("echo"),
        vec![0; MAX_SERVICE_PAYLOAD_BYTES + 1],
    );
    assert!(built.is_err());
}

#[test]
fn oversized_provider_response_is_replaced_with_a_structured_error() {
    let (_, _, _, client) = setup(
        ProviderBehavior::OversizedResponse,
        AuthorizationDecision::Allow,
    );
    let response = block_on(client.call(request(3, "echo", b"request"), CancellationToken::new()));
    assert_eq!(
        response.result().unwrap_err().code(),
        IpcErrorCode::ResponseTooLarge
    );
}

#[test]
fn registration_duplicate_unregister_and_stale_replacement_are_deterministic() {
    let registry = ServiceRegistry::new();
    let first = Arc::new(TestService::new(ProviderBehavior::Echo));
    let first_handle = registry
        .register(first.clone() as Arc<dyn ServiceProvider>)
        .expect("first registration");
    let duplicate = registry.register(first.clone() as Arc<dyn ServiceProvider>);
    assert_eq!(
        duplicate.unwrap_err().code(),
        IpcErrorCode::DuplicateRegistration
    );
    registry
        .unregister(&first_handle)
        .expect("unregister first");

    let replacement = Arc::new(TestService::new(ProviderBehavior::Fail));
    let replacement_handle = registry
        .register(replacement as Arc<dyn ServiceProvider>)
        .expect("replacement registration");
    assert_eq!(
        registry.unregister(&first_handle).unwrap_err().code(),
        IpcErrorCode::StaleRegistration
    );
    assert_eq!(registry.descriptors().len(), 1);
    registry
        .unregister(&replacement_handle)
        .expect("unregister replacement");
    assert_eq!(registry.descriptors().len(), 0);
}

#[test]
fn separate_registry_instances_do_not_share_providers() {
    let (left, _, left_transport, _) = setup(ProviderBehavior::Echo, AuthorizationDecision::Allow);
    let right = ServiceRegistry::new();
    assert_eq!(left_transport.descriptors().len(), 1);
    assert!(right.descriptors().is_empty());
    assert!(left.descriptors().len() > right.descriptors().len());
}

#[test]
fn client_resolves_exact_versions_and_reports_missing_services_and_versions() {
    let (_, _, _, client) = setup(ProviderBehavior::Echo, AuthorizationDecision::Allow);
    let descriptor = client
        .resolve(&service_id(), version(1, 0))
        .expect("matching descriptor");
    assert!(descriptor.supports_version(version(1, 0)));
    assert_eq!(client.descriptors().len(), 1);
    assert_eq!(
        descriptor.operations().next().unwrap().id(),
        &operation("echo")
    );

    let mismatch = client.resolve(&service_id(), version(1, 1)).unwrap_err();
    assert_eq!(mismatch.code(), IpcErrorCode::UnsupportedContractVersion);
    assert_eq!(mismatch.requested_version(), Some(version(1, 1)));
    assert_eq!(mismatch.supported_versions(), &[version(1, 0)]);

    let missing = ServiceId::new("example.missing").unwrap();
    assert_eq!(
        client.resolve(&missing, version(1, 0)).unwrap_err().code(),
        IpcErrorCode::ServiceNotFound
    );
}

#[test]
fn call_failures_include_service_and_version_errors_with_request_identity() {
    let (_, _, _, client) = setup(ProviderBehavior::Echo, AuthorizationDecision::Allow);
    let missing_id = ServiceId::new("example.missing").unwrap();
    let missing = block_on(client.call(
        request_to(21, missing_id, version(1, 0), "echo", b"missing"),
        CancellationToken::new(),
    ));
    assert_eq!(
        missing.result().unwrap_err().code(),
        IpcErrorCode::ServiceNotFound
    );
    assert_eq!(missing.metadata().request_id().get(), 21);
    assert_eq!(missing.metadata().correlation_id().unwrap().get(), 77);

    let unsupported = block_on(client.call(
        request_to(22, service_id(), version(1, 1), "echo", b"version"),
        CancellationToken::new(),
    ));
    let error = unsupported.result().unwrap_err();
    assert_eq!(error.code(), IpcErrorCode::UnsupportedContractVersion);
    assert_eq!(error.requested_version(), Some(version(1, 1)));
    assert_eq!(error.supported_versions(), &[version(1, 0)]);
    assert_eq!(unsupported.metadata().request_id().get(), 22);
}

#[test]
fn client_calls_provider_and_preserves_request_correlation_and_trace_identity() {
    let (_, _, _, client) = setup(ProviderBehavior::Echo, AuthorizationDecision::Allow);
    let response = block_on(client.call(request(9, "echo", b"nagi"), CancellationToken::new()));
    assert_eq!(response.result().expect("echo response"), b"nagi");
    assert_eq!(response.metadata().request_id().get(), 9);
    assert_eq!(response.metadata().correlation_id().unwrap().get(), 77);
    assert_eq!(response.metadata().trace_id().unwrap().get(), 88);
    assert_eq!(response.metadata().service_id(), &service_id());
    assert_eq!(response.metadata().operation_id(), &operation("echo"));
    assert_eq!(response.metadata().contract_version(), version(1, 0));
}

#[test]
fn operation_not_found_and_invalid_provider_requests_are_structured() {
    let (_, _, _, client) = setup(ProviderBehavior::RejectEmpty, AuthorizationDecision::Allow);
    let missing = block_on(client.call(request(10, "missing", b"x"), CancellationToken::new()));
    assert_eq!(
        missing.result().unwrap_err().code(),
        IpcErrorCode::OperationNotFound
    );

    let invalid = block_on(client.call(request(11, "echo", b""), CancellationToken::new()));
    assert_eq!(
        invalid.result().unwrap_err().code(),
        IpcErrorCode::InvalidRequest
    );
}

#[test]
fn provider_failures_do_not_expose_provider_prose() {
    let (_, _, _, client) = setup(ProviderBehavior::Fail, AuthorizationDecision::Allow);
    let response = block_on(client.call(
        request(12, "echo", b"secret provider details"),
        CancellationToken::new(),
    ));
    let error = response.result().unwrap_err();
    assert_eq!(error.code(), IpcErrorCode::ProviderFailure);
    assert_eq!(error.to_string(), "provider_failure");
}

#[test]
fn authorization_hook_receives_context_and_denial_never_calls_provider() {
    let policy = Arc::new(RecordingPolicy {
        decision: AuthorizationDecision::Deny,
        contexts: Mutex::new(Vec::new()),
    });
    let (_, provider, _, client) = setup_with_policy(ProviderBehavior::Echo, policy.clone());
    let response = block_on(client.call(request(13, "echo", b"denied"), CancellationToken::new()));
    assert_eq!(
        response.result().unwrap_err().code(),
        IpcErrorCode::PermissionDenied
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    let contexts = policy.contexts.lock().unwrap();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0].caller().as_str(), "example.echo-client");
    assert_eq!(contexts[0].service_id(), &service_id());
    assert_eq!(contexts[0].operation_id(), &operation("echo"));
    assert_eq!(
        contexts[0].required_capability().unwrap().as_str(),
        "system.echo.call"
    );
}

#[test]
fn cancellation_is_deterministic_before_and_during_provider_work() {
    let (_, provider, _, client) = setup(ProviderBehavior::Echo, AuthorizationDecision::Allow);
    let token = CancellationToken::new();
    token.cancel();
    let response = block_on(client.call(request(14, "echo", b"skip"), token));
    assert_eq!(
        response.result().unwrap_err().code(),
        IpcErrorCode::Cancelled
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);

    let (_, _, _, client) = setup(
        ProviderBehavior::CancelWhileHandling,
        AuthorizationDecision::Allow,
    );
    let response = block_on(client.call(request(15, "echo", b"cancel"), CancellationToken::new()));
    assert_eq!(
        response.result().unwrap_err().code(),
        IpcErrorCode::Cancelled
    );
}

#[test]
fn external_cancellation_wakes_a_pending_provider_call() {
    let (started_tx, started_rx) = mpsc::channel();
    let registry = Arc::new(ServiceRegistry::new());
    let provider = Arc::new(TestService::with_started(
        ProviderBehavior::WaitForCancellation,
        ServiceAvailability::Available,
        Some(started_tx),
    ));
    registry
        .register(provider as Arc<dyn ServiceProvider>)
        .expect("provider registration");
    let transport = Arc::new(InProcessReferenceTransport::new(
        registry,
        Arc::new(TestPolicy(AuthorizationDecision::Allow)),
        test_caller(),
    ));
    let client = ServiceClient::new(transport as Arc<dyn ServiceTransport>);
    let cancellation = CancellationToken::new();
    let cancel_from_client = cancellation.clone();
    let worker =
        thread::spawn(move || block_on(client.call(request(18, "echo", b"wait"), cancellation)));

    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("provider entered its pending future");
    cancel_from_client.cancel();
    let response = worker.join().expect("cancelled call completes");
    assert_eq!(
        response.result().unwrap_err().code(),
        IpcErrorCode::Cancelled
    );
}

#[test]
fn cancellation_future_wakes_waiters_and_dropped_waiters_are_removed() {
    struct WakeCounter(AtomicUsize);
    impl Wake for WakeCounter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let token = CancellationToken::new();
    let counter = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&counter));
    let mut context = Context::from_waker(&waker);
    let mut wait = Box::pin(token.cancelled());
    assert!(wait.as_mut().poll(&mut context).is_pending());
    token.cancel();
    assert_eq!(counter.0.load(Ordering::SeqCst), 1);
    assert!(wait.as_mut().poll(&mut context).is_ready());

    let dropped_token = CancellationToken::new();
    let dropped_counter = Arc::new(WakeCounter(AtomicUsize::new(0)));
    let dropped_waker = Waker::from(Arc::clone(&dropped_counter));
    let mut dropped_context = Context::from_waker(&dropped_waker);
    let mut wait = Box::pin(dropped_token.cancelled());
    assert!(wait.as_mut().poll(&mut dropped_context).is_pending());
    drop(wait);
    dropped_token.cancel();
    assert_eq!(dropped_counter.0.load(Ordering::SeqCst), 0);
}

#[test]
fn provider_availability_maps_to_busy_or_unavailable() {
    for (availability, expected) in [
        (ServiceAvailability::Busy, IpcErrorCode::Busy),
        (ServiceAvailability::Unavailable, IpcErrorCode::Unavailable),
    ] {
        let registry = Arc::new(ServiceRegistry::new());
        let provider = Arc::new(TestService::with_availability(
            ProviderBehavior::Echo,
            availability,
        ));
        registry
            .register(provider.clone() as Arc<dyn ServiceProvider>)
            .unwrap();
        let transport = Arc::new(InProcessReferenceTransport::new(
            registry,
            Arc::new(TestPolicy(AuthorizationDecision::Allow)),
            test_caller(),
        ));
        let client = ServiceClient::new(transport as Arc<dyn ServiceTransport>);
        let response = block_on(client.call(
            request(16, "echo", b"unavailable"),
            CancellationToken::new(),
        ));
        assert_eq!(response.result().unwrap_err().code(), expected);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn concurrent_calls_share_registry_without_serializing_provider_work() {
    let (_, provider, transport, _) = setup(ProviderBehavior::Echo, AuthorizationDecision::Allow);
    let workers = (1..=8)
        .map(|id| {
            let transport = Arc::clone(&transport);
            thread::spawn(move || {
                block_on(transport.call(request(id, "echo", b"parallel"), CancellationToken::new()))
                    .result()
                    .expect("parallel echo")
                    .to_vec()
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        assert_eq!(worker.join().expect("worker result"), b"parallel");
    }
    assert_eq!(provider.calls.load(Ordering::SeqCst), 8);
}

#[test]
fn provider_errors_are_restricted_to_structured_categories() {
    let error = IpcError::new(IpcErrorCode::TransportFailure);
    assert_eq!(error.code().as_str(), "transport_failure");
}
