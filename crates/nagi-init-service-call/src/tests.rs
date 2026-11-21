use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::task::{Context, Poll};
use std::thread;
use std::time::Duration;

use nagi_abi::MAX_CHANNEL_INLINE_PAYLOAD;
use nagi_action_ipc::{decode_intent, encode_result, ActionResult, ActionStatus, WireError};
use nagi_service_contract::{
    AuthorizationContext, AuthorizationDecision, AuthorizationPolicy, CancellationToken,
    ContractVersion, CorrelationId, InProcessReferenceTransport, IpcError, IpcErrorCode,
    OperationId, PrincipalId, ProviderFuture, RegistrationHandle, RequestEnvelope, RequestId,
    ResponseEnvelope, ServiceAvailability, ServiceClient, ServiceDescriptor, ServiceDiscovery,
    ServiceFuture, ServiceId, ServiceOperation, ServiceProvider, ServiceRegistry, ServiceTransport,
    TraceId,
};

use crate::{call_action_blocking, call_registered_blocking, ActionCallError, CallError};

const SERVICE: &str = "example.echo";
const OPERATION: &str = "echo";
const CALL_TIMEOUT: Duration = Duration::from_secs(2);

struct AllowFixtureCalls;

impl AuthorizationPolicy for AllowFixtureCalls {
    fn authorize(&self, _context: &AuthorizationContext) -> AuthorizationDecision {
        AuthorizationDecision::Allow
    }
}

struct EchoProvider {
    descriptor: ServiceDescriptor,
    calls: Arc<AtomicUsize>,
    entered: Option<Mutex<mpsc::Sender<()>>>,
    wait_for_cancel: bool,
    fail_transport: bool,
}

impl EchoProvider {
    fn new(calls: Arc<AtomicUsize>) -> Self {
        Self::with_entry_signal(calls, None, false, false)
    }

    fn blocking(calls: Arc<AtomicUsize>, entered: mpsc::Sender<()>) -> Self {
        Self::with_entry_signal(calls, Some(Mutex::new(entered)), true, false)
    }

    fn transport_failure(calls: Arc<AtomicUsize>) -> Self {
        Self::with_entry_signal(calls, None, false, true)
    }

    fn with_entry_signal(
        calls: Arc<AtomicUsize>,
        entered: Option<Mutex<mpsc::Sender<()>>>,
        wait_for_cancel: bool,
        fail_transport: bool,
    ) -> Self {
        let descriptor = ServiceDescriptor::new(
            ServiceId::new(SERVICE).unwrap(),
            [ContractVersion::new(1, 0).unwrap()],
            [ServiceOperation::new(
                OperationId::new(OPERATION).unwrap(),
                None,
            )],
        )
        .unwrap();
        Self {
            descriptor,
            calls,
            entered,
            wait_for_cancel,
            fail_transport,
        }
    }
}

impl ServiceProvider for EchoProvider {
    fn descriptor(&self) -> &ServiceDescriptor {
        &self.descriptor
    }

    fn availability(&self) -> ServiceAvailability {
        ServiceAvailability::Available
    }

    fn handle<'a>(
        &'a self,
        request: RequestEnvelope,
        context: nagi_service_contract::CallContext,
    ) -> ProviderFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = &self.entered {
            if let Ok(signal) = entered.lock() {
                let _ = signal.send(());
            }
        }
        let cancellation = context.cancellation().clone();
        let wait_for_cancel = self.wait_for_cancel;
        let fail_transport = self.fail_transport;
        Box::pin(async move {
            if wait_for_cancel {
                cancellation.cancelled().await;
                return Err(nagi_service_contract::ProviderError::new(
                    nagi_service_contract::ProviderErrorCode::Cancelled,
                ));
            }
            if cancellation.is_cancelled() {
                return Err(nagi_service_contract::ProviderError::new(
                    nagi_service_contract::ProviderErrorCode::Cancelled,
                ));
            }
            if fail_transport {
                return Err(nagi_service_contract::ProviderError::new(
                    nagi_service_contract::ProviderErrorCode::TransportFailure,
                ));
            }
            Ok(request.payload().to_vec())
        })
    }
}

struct Fixture {
    registry: Arc<ServiceRegistry>,
    registration: RegistrationHandle,
    calls: Arc<AtomicUsize>,
    transport: Arc<dyn ServiceTransport>,
}

fn fixture() -> Fixture {
    // This uses the pinned host reference transport only. It is regression
    // evidence for consumer behavior, never production READY evidence.
    let registry = Arc::new(ServiceRegistry::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let registration = registry
        .register(Arc::new(EchoProvider::new(Arc::clone(&calls))))
        .unwrap();
    let transport: Arc<dyn ServiceTransport> = Arc::new(InProcessReferenceTransport::new(
        Arc::clone(&registry),
        Arc::new(AllowFixtureCalls),
        PrincipalId::new("candidate.fixture-client").unwrap(),
    ));
    Fixture {
        registry,
        registration,
        calls,
        transport,
    }
}

fn client(transport: Arc<dyn ServiceTransport>) -> ServiceClient {
    ServiceClient::new(transport)
}

fn request(id: u128, payload: &[u8]) -> RequestEnvelope {
    request_with_correlation(id, id + 100, payload)
}

fn request_with_correlation(id: u128, correlation_id: u128, payload: &[u8]) -> RequestEnvelope {
    RequestEnvelope::new(
        RequestId::new(id).unwrap(),
        Some(CorrelationId::new(correlation_id).unwrap()),
        None,
        ServiceId::new(SERVICE).unwrap(),
        ContractVersion::new(1, 0).unwrap(),
        OperationId::new(OPERATION).unwrap(),
        payload.to_vec(),
    )
    .unwrap()
}

fn request_for_operation(id: u128, operation: &str, payload: &[u8]) -> RequestEnvelope {
    RequestEnvelope::new(
        RequestId::new(id).unwrap(),
        Some(CorrelationId::new(id + 100).unwrap()),
        None,
        ServiceId::new(SERVICE).unwrap(),
        ContractVersion::new(1, 0).unwrap(),
        OperationId::new(operation).unwrap(),
        payload.to_vec(),
    )
    .unwrap()
}

#[test]
fn reference_dispatch_requires_registered_operation_and_matching_response() {
    let fixture = fixture();
    let response = call_registered_blocking(
        &client(Arc::clone(&fixture.transport)),
        request(1, b"typed request bytes"),
        CancellationToken::new(),
        CALL_TIMEOUT,
    )
    .unwrap();

    assert_eq!(response.result().unwrap(), b"typed request bytes");
    assert_eq!(response.metadata().request_id().get(), 1);
    assert_eq!(response.metadata().service_id().as_str(), SERVICE);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
}

struct CountingTransport {
    inner: Arc<dyn ServiceTransport>,
    resolves: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
}

impl ServiceDiscovery for CountingTransport {
    fn descriptors(&self) -> Vec<ServiceDescriptor> {
        self.inner.descriptors()
    }

    fn resolve(
        &self,
        service_id: &ServiceId,
        version: ContractVersion,
    ) -> Result<ServiceDescriptor, IpcError> {
        self.resolves.fetch_add(1, Ordering::SeqCst);
        self.inner.resolve(service_id, version)
    }
}

impl ServiceTransport for CountingTransport {
    fn call<'a>(
        &'a self,
        request: RequestEnvelope,
        cancellation: CancellationToken,
    ) -> ServiceFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.call(request, cancellation)
    }
}

#[test]
fn cancellation_before_dispatch_never_discovers_or_calls_a_service() {
    let fixture = fixture();
    let resolves = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let transport = Arc::new(CountingTransport {
        inner: Arc::clone(&fixture.transport),
        resolves: Arc::clone(&resolves),
        calls: Arc::clone(&calls),
    });
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let result = call_registered_blocking(
        &client(transport),
        request(2, b"cancel before dispatch"),
        cancellation,
        CALL_TIMEOUT,
    );

    assert_eq!(result, Err(CallError::Cancelled));
    assert_eq!(resolves.load(Ordering::SeqCst), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn invalid_or_expired_deadline_is_rejected_before_service_discovery() {
    let fixture = fixture();
    let resolves = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let transport = Arc::new(CountingTransport {
        inner: Arc::clone(&fixture.transport),
        resolves: Arc::clone(&resolves),
        calls: Arc::clone(&calls),
    });
    let client = client(transport);

    assert_eq!(
        call_registered_blocking(
            &client,
            request(23, b"zero timeout"),
            CancellationToken::new(),
            Duration::ZERO,
        ),
        Err(CallError::TimedOut)
    );
    assert_eq!(
        call_registered_blocking(
            &client,
            request(24, b"unrepresentable timeout"),
            CancellationToken::new(),
            Duration::MAX,
        ),
        Err(CallError::InvalidTimeout)
    );
    assert_eq!(resolves.load(Ordering::SeqCst), 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn unregistered_operation_is_rejected_before_provider_dispatch() {
    let fixture = fixture();
    let result = call_registered_blocking(
        &client(Arc::clone(&fixture.transport)),
        request_for_operation(8, "missing", b"unknown operation"),
        CancellationToken::new(),
        CALL_TIMEOUT,
    );

    assert_eq!(
        result,
        Err(CallError::Discovery(IpcError::new(
            IpcErrorCode::OperationNotFound
        )))
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

struct ReplayTransport {
    inner: Arc<dyn ServiceTransport>,
    last_response: Mutex<Option<ResponseEnvelope>>,
}

impl ServiceDiscovery for ReplayTransport {
    fn descriptors(&self) -> Vec<ServiceDescriptor> {
        self.inner.descriptors()
    }

    fn resolve(
        &self,
        service_id: &ServiceId,
        version: ContractVersion,
    ) -> Result<ServiceDescriptor, IpcError> {
        self.inner.resolve(service_id, version)
    }
}

impl ServiceTransport for ReplayTransport {
    fn call<'a>(
        &'a self,
        request: RequestEnvelope,
        cancellation: CancellationToken,
    ) -> ServiceFuture<'a> {
        Box::pin(async move {
            if let Some(replay) = self
                .last_response
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
            {
                return replay;
            }
            let response = self.inner.call(request, cancellation).await;
            *self
                .last_response
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(response.clone());
            response
        })
    }
}

#[test]
fn stale_or_wrong_response_is_rejected_before_the_caller_can_accept_it() {
    let fixture = fixture();
    let replay = Arc::new(ReplayTransport {
        inner: Arc::clone(&fixture.transport),
        last_response: Mutex::new(None),
    });
    let client = client(replay);

    call_registered_blocking(
        &client,
        request_with_correlation(3, 303, b"first call"),
        CancellationToken::new(),
        CALL_TIMEOUT,
    )
    .unwrap();
    let result = call_registered_blocking(
        &client,
        request_with_correlation(4, 303, b"second call"),
        CancellationToken::new(),
        CALL_TIMEOUT,
    );

    assert_eq!(result, Err(CallError::ResponseMismatch));
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
}

#[derive(Clone, Copy, Debug)]
enum ResponseMetadataField {
    CorrelationId,
    TraceId,
    ServiceId,
    ContractVersion,
    OperationId,
}

struct MetadataMismatchTransport {
    inner: Arc<dyn ServiceTransport>,
    field: ResponseMetadataField,
}

impl ServiceDiscovery for MetadataMismatchTransport {
    fn descriptors(&self) -> Vec<ServiceDescriptor> {
        self.inner.descriptors()
    }

    fn resolve(
        &self,
        service_id: &ServiceId,
        version: ContractVersion,
    ) -> Result<ServiceDescriptor, IpcError> {
        self.inner.resolve(service_id, version)
    }
}

impl ServiceTransport for MetadataMismatchTransport {
    fn call<'a>(
        &'a self,
        request: RequestEnvelope,
        cancellation: CancellationToken,
    ) -> ServiceFuture<'a> {
        let mut correlation_id = request.correlation_id();
        let mut trace_id = request.trace_id();
        let mut service_id = request.service_id().clone();
        let mut contract_version = request.contract_version();
        let mut operation_id = request.operation_id().clone();
        match self.field {
            ResponseMetadataField::CorrelationId => {
                correlation_id = Some(CorrelationId::new(90_001).unwrap());
            }
            ResponseMetadataField::TraceId => trace_id = Some(TraceId::new(90_002).unwrap()),
            ResponseMetadataField::ServiceId => {
                service_id = ServiceId::new("example.other").unwrap();
            }
            ResponseMetadataField::ContractVersion => {
                contract_version = ContractVersion::new(2, 0).unwrap();
            }
            ResponseMetadataField::OperationId => {
                operation_id = OperationId::new("other").unwrap();
            }
        }
        let altered_request = RequestEnvelope::new(
            request.request_id(),
            correlation_id,
            trace_id,
            service_id,
            contract_version,
            operation_id,
            request.payload().to_vec(),
        )
        .unwrap();
        self.inner.call(altered_request, cancellation)
    }
}

fn assert_response_metadata_mismatch_is_rejected(field: ResponseMetadataField) {
    let fixture = fixture();
    let transport: Arc<dyn ServiceTransport> = Arc::new(MetadataMismatchTransport {
        inner: Arc::clone(&fixture.transport),
        field,
    });
    let result = call_registered_blocking(
        &client(transport),
        request(20, b"individually altered response metadata"),
        CancellationToken::new(),
        CALL_TIMEOUT,
    );

    assert_eq!(result, Err(CallError::ResponseMismatch), "{field:?}");
}

#[test]
fn correlation_id_mismatch_is_rejected_independently() {
    assert_response_metadata_mismatch_is_rejected(ResponseMetadataField::CorrelationId);
}

#[test]
fn trace_id_mismatch_is_rejected_independently() {
    assert_response_metadata_mismatch_is_rejected(ResponseMetadataField::TraceId);
}

#[test]
fn service_id_mismatch_is_rejected_independently() {
    assert_response_metadata_mismatch_is_rejected(ResponseMetadataField::ServiceId);
}

#[test]
fn contract_version_mismatch_is_rejected_independently() {
    assert_response_metadata_mismatch_is_rejected(ResponseMetadataField::ContractVersion);
}

#[test]
fn operation_id_mismatch_is_rejected_independently() {
    assert_response_metadata_mismatch_is_rejected(ResponseMetadataField::OperationId);
}

struct WithdrawOnResolveTransport {
    inner: Arc<dyn ServiceTransport>,
    registry: Arc<ServiceRegistry>,
    registration: RegistrationHandle,
    did_withdraw: AtomicBool,
}

impl ServiceDiscovery for WithdrawOnResolveTransport {
    fn descriptors(&self) -> Vec<ServiceDescriptor> {
        self.inner.descriptors()
    }

    fn resolve(
        &self,
        service_id: &ServiceId,
        version: ContractVersion,
    ) -> Result<ServiceDescriptor, IpcError> {
        let descriptor = self.inner.resolve(service_id, version)?;
        if !self.did_withdraw.swap(true, Ordering::SeqCst) {
            self.registry.unregister(&self.registration)?;
        }
        Ok(descriptor)
    }
}

impl ServiceTransport for WithdrawOnResolveTransport {
    fn call<'a>(
        &'a self,
        request: RequestEnvelope,
        cancellation: CancellationToken,
    ) -> ServiceFuture<'a> {
        self.inner.call(request, cancellation)
    }
}

#[test]
fn service_withdrawal_between_discovery_and_dispatch_cannot_succeed() {
    let fixture = fixture();
    let transport = Arc::new(WithdrawOnResolveTransport {
        inner: Arc::clone(&fixture.transport),
        registry: Arc::clone(&fixture.registry),
        registration: fixture.registration.clone(),
        did_withdraw: AtomicBool::new(false),
    });
    let result = call_registered_blocking(
        &client(transport),
        request(5, b"withdraw between resolve and dispatch"),
        CancellationToken::new(),
        CALL_TIMEOUT,
    );

    assert_eq!(
        result,
        Err(CallError::Remote(IpcError::new(
            IpcErrorCode::ServiceNotFound
        )))
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn transport_reported_disconnect_never_returns_a_successful_response() {
    // The reference provider injects the contract's transport-failure result.
    // This is not a socket peer-death or production transport test.
    let registry = Arc::new(ServiceRegistry::new());
    let calls = Arc::new(AtomicUsize::new(0));
    registry
        .register(Arc::new(EchoProvider::transport_failure(Arc::clone(
            &calls,
        ))))
        .unwrap();
    let transport: Arc<dyn ServiceTransport> = Arc::new(InProcessReferenceTransport::new(
        registry,
        Arc::new(AllowFixtureCalls),
        PrincipalId::new("candidate.fixture-client").unwrap(),
    ));

    let result = call_registered_blocking(
        &client(transport),
        request(9, b"connection dropped"),
        CancellationToken::new(),
        CALL_TIMEOUT,
    );

    assert_eq!(
        result,
        Err(CallError::Remote(IpcError::new(
            IpcErrorCode::TransportFailure
        )))
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn already_withdrawn_service_is_not_discovered_or_called() {
    let fixture = fixture();
    fixture.registry.unregister(&fixture.registration).unwrap();
    let resolves = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let transport = Arc::new(CountingTransport {
        inner: Arc::clone(&fixture.transport),
        resolves: Arc::clone(&resolves),
        calls: Arc::clone(&calls),
    });

    let result = call_registered_blocking(
        &client(transport),
        request(6, b"service already withdrawn"),
        CancellationToken::new(),
        CALL_TIMEOUT,
    );

    assert_eq!(
        result,
        Err(CallError::Discovery(IpcError::new(
            IpcErrorCode::ServiceNotFound
        )))
    );
    assert_eq!(resolves.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn cancellation_after_provider_entry_discards_the_response() {
    let registry = Arc::new(ServiceRegistry::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let (entered_tx, entered_rx) = mpsc::channel();
    registry
        .register(Arc::new(EchoProvider::blocking(
            Arc::clone(&calls),
            entered_tx,
        )))
        .unwrap();
    let transport: Arc<dyn ServiceTransport> = Arc::new(InProcessReferenceTransport::new(
        registry,
        Arc::new(AllowFixtureCalls),
        PrincipalId::new("candidate.fixture-client").unwrap(),
    ));
    let client = client(transport);
    let cancellation = CancellationToken::new();
    let worker_token = cancellation.clone();
    let worker = thread::spawn(move || {
        call_registered_blocking(
            &client,
            request(7, b"cancel after provider entry"),
            worker_token,
            CALL_TIMEOUT,
        )
    });

    entered_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("provider should receive the dispatched request");
    cancellation.cancel();

    assert_eq!(worker.join().unwrap(), Err(CallError::Cancelled));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

struct NeverWakingFuture {
    entered: Option<mpsc::Sender<()>>,
    poll_count: Arc<AtomicUsize>,
    drop_count: Arc<AtomicUsize>,
}

impl Future for NeverWakingFuture {
    type Output = ResponseEnvelope;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        this.poll_count.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = this.entered.take() {
            let _ = entered.send(());
        }
        Poll::Pending
    }
}

impl Drop for NeverWakingFuture {
    fn drop(&mut self) {
        self.drop_count.fetch_add(1, Ordering::SeqCst);
    }
}

struct NeverWakingTransport {
    inner: Arc<dyn ServiceTransport>,
    entered: Mutex<Option<mpsc::Sender<()>>>,
    poll_count: Arc<AtomicUsize>,
    drop_count: Arc<AtomicUsize>,
}

impl ServiceDiscovery for NeverWakingTransport {
    fn descriptors(&self) -> Vec<ServiceDescriptor> {
        self.inner.descriptors()
    }

    fn resolve(
        &self,
        service_id: &ServiceId,
        version: ContractVersion,
    ) -> Result<ServiceDescriptor, IpcError> {
        self.inner.resolve(service_id, version)
    }
}

impl ServiceTransport for NeverWakingTransport {
    fn call<'a>(
        &'a self,
        _request: RequestEnvelope,
        _cancellation: CancellationToken,
    ) -> ServiceFuture<'a> {
        Box::pin(NeverWakingFuture {
            entered: self
                .entered
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take(),
            poll_count: Arc::clone(&self.poll_count),
            drop_count: Arc::clone(&self.drop_count),
        })
    }
}

fn never_waking_transport(
    inner: Arc<dyn ServiceTransport>,
) -> (
    Arc<dyn ServiceTransport>,
    mpsc::Receiver<()>,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
) {
    let (entered_tx, entered_rx) = mpsc::channel();
    let poll_count = Arc::new(AtomicUsize::new(0));
    let drop_count = Arc::new(AtomicUsize::new(0));
    let transport = Arc::new(NeverWakingTransport {
        inner,
        entered: Mutex::new(Some(entered_tx)),
        poll_count: Arc::clone(&poll_count),
        drop_count: Arc::clone(&drop_count),
    });
    (transport, entered_rx, poll_count, drop_count)
}

#[test]
fn cancellation_wakes_and_drops_a_pending_transport_future_that_never_wakes() {
    let fixture = fixture();
    let (transport, entered, poll_count, drop_count) =
        never_waking_transport(Arc::clone(&fixture.transport));
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let client = client(transport);
    let (result_tx, result_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = call_registered_blocking(
            &client,
            request(21, b"cancel a non-waking future"),
            worker_cancellation,
            Duration::from_secs(5),
        );
        result_tx.send(result).unwrap();
    });

    entered
        .recv_timeout(Duration::from_secs(1))
        .expect("non-waking future should be pending before cancellation");
    cancellation.cancel();
    assert_eq!(
        result_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        Err(CallError::Cancelled)
    );
    worker.join().unwrap();
    assert_eq!(poll_count.load(Ordering::SeqCst), 1);
    assert_eq!(drop_count.load(Ordering::SeqCst), 1);
}

#[test]
fn deadline_bounds_and_drops_a_pending_transport_future_without_waking() {
    let fixture = fixture();
    let (transport, entered, poll_count, drop_count) =
        never_waking_transport(Arc::clone(&fixture.transport));
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let client = client(transport);
    let (result_tx, result_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = call_registered_blocking(
            &client,
            request(22, b"timeout a non-waking future"),
            worker_cancellation,
            Duration::from_millis(40),
        );
        result_tx.send(result).unwrap();
    });

    entered
        .recv_timeout(Duration::from_secs(1))
        .expect("non-waking future should be pending before deadline");
    assert_eq!(
        result_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        Err(CallError::TimedOut)
    );
    worker.join().unwrap();
    assert!(cancellation.is_cancelled());
    assert_eq!(poll_count.load(Ordering::SeqCst), 1);
    assert_eq!(drop_count.load(Ordering::SeqCst), 1);
}

struct DelayedResolveTransport {
    inner: Arc<dyn ServiceTransport>,
    entered: Mutex<Option<mpsc::Sender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
    call_count: Arc<AtomicUsize>,
}

impl ServiceDiscovery for DelayedResolveTransport {
    fn descriptors(&self) -> Vec<ServiceDescriptor> {
        self.inner.descriptors()
    }

    fn resolve(
        &self,
        service_id: &ServiceId,
        version: ContractVersion,
    ) -> Result<ServiceDescriptor, IpcError> {
        if let Some(entered) = self
            .entered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = entered.send(());
        }
        self.release
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recv()
            .expect("resolver release should arrive");
        self.inner.resolve(service_id, version)
    }
}

impl ServiceTransport for DelayedResolveTransport {
    fn call<'a>(
        &'a self,
        request: RequestEnvelope,
        cancellation: CancellationToken,
    ) -> ServiceFuture<'a> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        self.inner.call(request, cancellation)
    }
}

#[test]
fn expired_deadline_does_not_preempt_sync_resolve_or_dispatch_after_it_returns() {
    let fixture = fixture();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let call_count = Arc::new(AtomicUsize::new(0));
    let transport: Arc<dyn ServiceTransport> = Arc::new(DelayedResolveTransport {
        inner: Arc::clone(&fixture.transport),
        entered: Mutex::new(Some(entered_tx)),
        release: Mutex::new(release_rx),
        call_count: Arc::clone(&call_count),
    });
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let client = client(transport);
    let (result_tx, result_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = call_registered_blocking(
            &client,
            request(25, b"delayed synchronous resolve"),
            worker_cancellation,
            Duration::from_millis(40),
        );
        result_tx.send(result).unwrap();
    });

    entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("resolver should enter its blocking section");
    assert!(matches!(
        result_rx.recv_timeout(Duration::from_millis(80)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    release_tx.send(()).unwrap();
    assert_eq!(
        result_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        Err(CallError::TimedOut)
    );
    worker.join().unwrap();
    assert!(cancellation.is_cancelled());
    assert_eq!(call_count.load(Ordering::SeqCst), 0);
}

struct DelayedCallInitializationTransport {
    inner: Arc<dyn ServiceTransport>,
    entered: Mutex<Option<mpsc::Sender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
    poll_count: Arc<AtomicUsize>,
    drop_count: Arc<AtomicUsize>,
    call_count: Arc<AtomicUsize>,
}

impl ServiceDiscovery for DelayedCallInitializationTransport {
    fn descriptors(&self) -> Vec<ServiceDescriptor> {
        self.inner.descriptors()
    }

    fn resolve(
        &self,
        service_id: &ServiceId,
        version: ContractVersion,
    ) -> Result<ServiceDescriptor, IpcError> {
        self.inner.resolve(service_id, version)
    }
}

impl ServiceTransport for DelayedCallInitializationTransport {
    fn call<'a>(
        &'a self,
        _request: RequestEnvelope,
        _cancellation: CancellationToken,
    ) -> ServiceFuture<'a> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        if let Some(entered) = self
            .entered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            let _ = entered.send(());
        }
        self.release
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .recv()
            .expect("transport future initialization release should arrive");
        Box::pin(NeverWakingFuture {
            entered: None,
            poll_count: Arc::clone(&self.poll_count),
            drop_count: Arc::clone(&self.drop_count),
        })
    }
}

#[test]
fn expired_deadline_drops_a_future_created_by_a_slow_sync_call_method_before_polling() {
    let fixture = fixture();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let poll_count = Arc::new(AtomicUsize::new(0));
    let drop_count = Arc::new(AtomicUsize::new(0));
    let call_count = Arc::new(AtomicUsize::new(0));
    let transport: Arc<dyn ServiceTransport> = Arc::new(DelayedCallInitializationTransport {
        inner: Arc::clone(&fixture.transport),
        entered: Mutex::new(Some(entered_tx)),
        release: Mutex::new(release_rx),
        poll_count: Arc::clone(&poll_count),
        drop_count: Arc::clone(&drop_count),
        call_count: Arc::clone(&call_count),
    });
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let client = client(transport);
    let (result_tx, result_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let result = call_registered_blocking(
            &client,
            request(26, b"delayed transport future creation"),
            worker_cancellation,
            Duration::from_millis(40),
        );
        result_tx.send(result).unwrap();
    });

    entered_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("transport call method should enter before the deadline");
    assert!(matches!(
        result_rx.recv_timeout(Duration::from_millis(80)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    release_tx.send(()).unwrap();
    assert_eq!(
        result_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        Err(CallError::TimedOut)
    );
    worker.join().unwrap();
    assert!(cancellation.is_cancelled());
    assert_eq!(call_count.load(Ordering::SeqCst), 1);
    assert_eq!(poll_count.load(Ordering::SeqCst), 0);
    assert_eq!(drop_count.load(Ordering::SeqCst), 1);
}

const ACTION_SERVICE: &str = "example.action";
const ACTION_OPERATION: &str = "run";

struct ActionReplyProvider {
    descriptor: ServiceDescriptor,
    observed: Arc<Mutex<Option<(u128, String)>>>,
}

impl ServiceProvider for ActionReplyProvider {
    fn descriptor(&self) -> &ServiceDescriptor {
        &self.descriptor
    }

    fn handle<'a>(
        &'a self,
        request: RequestEnvelope,
        _context: nagi_service_contract::CallContext,
    ) -> ProviderFuture<'a> {
        let observed = Arc::clone(&self.observed);
        Box::pin(async move {
            let intent = decode_intent(request.payload())
                .expect("adapter must use action@1 intent encoding")
                .to_owned();
            *observed.lock().unwrap() = Some((request.request_id().get(), intent));

            let reply = ActionResult::status_only(ActionStatus::Succeeded);
            let mut payload = [0_u8; MAX_CHANNEL_INLINE_PAYLOAD];
            let payload_len = encode_result(&reply, &mut payload).unwrap();
            Ok(payload[..payload_len].to_vec())
        })
    }
}

struct ActionFixture {
    client: ServiceClient,
    _registration: RegistrationHandle,
    observed: Arc<Mutex<Option<(u128, String)>>>,
}

fn action_fixture() -> ActionFixture {
    let registry = Arc::new(ServiceRegistry::new());
    let observed = Arc::new(Mutex::new(None));
    let descriptor = ServiceDescriptor::new(
        ServiceId::new(ACTION_SERVICE).unwrap(),
        [ContractVersion::new(1, 0).unwrap()],
        [ServiceOperation::new(
            OperationId::new(ACTION_OPERATION).unwrap(),
            None,
        )],
    )
    .unwrap();
    let registration = registry
        .register(Arc::new(ActionReplyProvider {
            descriptor,
            observed: Arc::clone(&observed),
        }))
        .unwrap();
    let transport: Arc<dyn ServiceTransport> = Arc::new(InProcessReferenceTransport::new(
        registry,
        Arc::new(AllowFixtureCalls),
        PrincipalId::new("candidate.fixture-client").unwrap(),
    ));
    ActionFixture {
        client: client(transport),
        _registration: registration,
        observed,
    }
}

fn action_target() -> (ServiceId, ContractVersion, OperationId) {
    (
        ServiceId::new(ACTION_SERVICE).unwrap(),
        ContractVersion::new(1, 0).unwrap(),
        OperationId::new(ACTION_OPERATION).unwrap(),
    )
}

#[test]
fn typed_action_adapter_reuses_action_codec_and_widens_request_id_losslessly() {
    let fixture = action_fixture();
    let (service_id, version, operation_id) = action_target();
    let reply = call_action_blocking(
        &fixture.client,
        u64::MAX,
        service_id,
        version,
        operation_id,
        "move the selected item",
        CancellationToken::new(),
        CALL_TIMEOUT,
    )
    .unwrap();

    assert_eq!(reply, ActionResult::status_only(ActionStatus::Succeeded));
    assert_eq!(
        *fixture.observed.lock().unwrap(),
        Some((u128::from(u64::MAX), "move the selected item".to_owned()))
    );
}

#[test]
fn typed_action_adapter_rejects_zero_id_and_invalid_intent_before_dispatch() {
    let fixture = action_fixture();
    let (service_id, version, operation_id) = action_target();

    assert_eq!(
        call_action_blocking(
            &fixture.client,
            0,
            service_id.clone(),
            version,
            operation_id.clone(),
            "valid intent",
            CancellationToken::new(),
            CALL_TIMEOUT,
        ),
        Err(ActionCallError::InvalidRequestId)
    );
    assert_eq!(
        call_action_blocking(
            &fixture.client,
            1,
            service_id,
            version,
            operation_id,
            "",
            CancellationToken::new(),
            CALL_TIMEOUT,
        ),
        Err(ActionCallError::EncodeIntent(WireError::EmptyIntent))
    );
    assert_eq!(*fixture.observed.lock().unwrap(), None);
}

#[test]
fn typed_action_adapter_rejects_malformed_typed_reply_payload() {
    let fixture = fixture();
    let result = call_action_blocking(
        &client(Arc::clone(&fixture.transport)),
        10,
        ServiceId::new(SERVICE).unwrap(),
        ContractVersion::new(1, 0).unwrap(),
        OperationId::new(OPERATION).unwrap(),
        "valid intent",
        CancellationToken::new(),
        CALL_TIMEOUT,
    );

    assert_eq!(
        result,
        Err(ActionCallError::DecodeResult(WireError::InvalidStatus))
    );
    assert_eq!(fixture.calls.load(Ordering::SeqCst), 1);
}
