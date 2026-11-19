use std::future::Future;
use std::os::unix::net::UnixStream;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use nagi_nipc_v1_stream_candidate::{
    CallError as CarrierCallError, CallReply, FrameError as CarrierFrameError,
};
use nagi_sdk::app_contract::{
    CorrelationId, ErrorEnvelope, IpcEnvelope, MessageKind, OrderingMetadata, RequestId,
    IPC_PROTOCOL_VERSION,
};
use nagi_sdk::AppId;
use nagi_service_contract::{
    CancellationToken, ContractVersion, IpcError, IpcErrorCode, OperationId, RequestEnvelope,
    ResponseEnvelope, ServiceDescriptor, ServiceDiscovery, ServiceFuture, ServiceId,
    ServiceTransport,
};

use crate::unix_driver::{call_unix_stream_until, UnixCallError};

/// An explicit conversion from an SVC request to the existing NIPC v1 fields,
/// plus the owner-provided typed reply decoder. This mapping owns every ID,
/// AppId, and payload type decision; the adapter derives none from UID/PID or
/// service names. Its methods must be bounded, deterministic CPU work over
/// bounded payloads and must not perform blocking I/O or wait on other work.
/// The worker cannot preempt these callbacks. Dropping a pending call cancels
/// socket I/O and joins its worker, so it waits for an active callback to
/// return; implementations must keep callback execution bounded.
pub trait ServiceNipcMapping: Send + Sync {
    fn map_request(&self, request: &RequestEnvelope) -> Result<MappedNipcRequest, IpcError>;

    fn decode_response(
        &self,
        request: &RequestEnvelope,
        response: &IpcEnvelope<'_>,
    ) -> Result<Vec<u8>, IpcError>;

    fn decode_error(&self, request: &RequestEnvelope, error: &ErrorEnvelope<'_>) -> IpcError;
}

/// Supplies one already-connected stream only after the runtime's trusted
/// peer authenticator and live service-registration owner have approved this
/// exact service/version/operation. `try_take_authenticated_single_call_stream`
/// must be nonblocking and must recheck registration at checkout. Returned
/// streams are single-call owned: the transport drops them when that call
/// finishes, and must be connected blocking-mode streams because the existing
/// carrier does not wait for readiness (`WouldBlock` is returned as an I/O
/// failure). This interface does not create a PrincipalId or treat process
/// credentials as authority. Discovery methods must also be bounded and must
/// not perform blocking socket I/O.
pub trait AuthenticatedUnixStreamProvider: ServiceDiscovery + Send + Sync {
    fn try_take_authenticated_single_call_stream(
        &self,
        service_id: &ServiceId,
        version: ContractVersion,
        operation_id: &OperationId,
    ) -> Result<Option<UnixStream>, IpcError>;
}

/// The caller-selected NIPC fields for one SVC request. NIPC v1 and Request
/// kind are fixed to the existing SDK contract; all identities, routing,
/// payload type, and request bytes are supplied explicitly by a mapping.
#[derive(Clone, Debug)]
pub struct MappedNipcRequest {
    source: AppId,
    destination: AppId,
    correlation_id: CorrelationId,
    request_id: RequestId,
    ordering: OrderingMetadata,
    payload_type: String,
    payload: Vec<u8>,
}

impl MappedNipcRequest {
    pub fn new(
        source: AppId,
        destination: AppId,
        correlation_id: CorrelationId,
        request_id: RequestId,
        ordering: OrderingMetadata,
        payload_type: impl Into<String>,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            source,
            destination,
            correlation_id,
            request_id,
            ordering,
            payload_type: payload_type.into(),
            payload,
        }
    }

    fn envelope(&self) -> IpcEnvelope<'_> {
        IpcEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            source: self.source,
            destination: Some(self.destination),
            kind: MessageKind::Request,
            correlation_id: self.correlation_id,
            request_id: Some(self.request_id),
            ordering: self.ordering,
            payload_type: &self.payload_type,
            payload: &self.payload,
        }
    }

    fn matches_reply_route(&self, reply: &IpcEnvelope<'_>) -> bool {
        reply.source == self.destination && reply.destination == Some(self.source)
    }

    fn matches_reply_ids(&self, reply: &IpcEnvelope<'_>) -> bool {
        reply.correlation_id == self.correlation_id && reply.request_id == Some(self.request_id)
    }
}

/// A `ServiceTransport` adapter that keeps synchronous UnixStream I/O off
/// `ServiceTransport::call` and `Future::poll`. A call checks out a single-use
/// authenticated stream and maps the request synchronously, then a dedicated
/// owned worker runs the existing blocking NIPC carrier. Dropping the future
/// cancels the same token and joins the worker, so timeout/cancellation cannot
/// be reported while an I/O worker remains detached.
pub struct UnixNipcServiceTransport {
    peer_provider: Option<Arc<dyn AuthenticatedUnixStreamProvider>>,
    mapping: Arc<dyn ServiceNipcMapping>,
    default_timeout: Duration,
}

impl UnixNipcServiceTransport {
    /// Construct the fail-closed form used until an authenticated peer and
    /// registration supplier is wired in. Discovery is empty and calls fail
    /// with `Unavailable`; no raw stream is accepted as authority.
    pub fn without_peer_authority(
        mapping: Arc<dyn ServiceNipcMapping>,
        default_timeout: Duration,
    ) -> Result<Self, UnixCallError> {
        Self::new(None, mapping, default_timeout)
    }

    /// Construct with the runtime-owned supplier of preauthenticated,
    /// currently registered, single-call streams. The supplied implementation
    /// is the authority boundary and must keep discovery/checkout nonblocking.
    pub fn with_peer_provider(
        provider: Arc<dyn AuthenticatedUnixStreamProvider>,
        mapping: Arc<dyn ServiceNipcMapping>,
        default_timeout: Duration,
    ) -> Result<Self, UnixCallError> {
        Self::new(Some(provider), mapping, default_timeout)
    }

    fn new(
        peer_provider: Option<Arc<dyn AuthenticatedUnixStreamProvider>>,
        mapping: Arc<dyn ServiceNipcMapping>,
        default_timeout: Duration,
    ) -> Result<Self, UnixCallError> {
        if default_timeout.is_zero() || Instant::now().checked_add(default_timeout).is_none() {
            return Err(UnixCallError::InvalidTimeout);
        }
        Ok(Self {
            peer_provider,
            mapping,
            default_timeout,
        })
    }

    fn failure_future<'a>(request: &RequestEnvelope, code: IpcErrorCode) -> ServiceFuture<'a> {
        let response = ResponseEnvelope::failure_for_request(request, IpcError::new(code));
        Box::pin(std::future::ready(response))
    }

    fn call_until<'a>(
        &'a self,
        request: RequestEnvelope,
        cancellation: CancellationToken,
        deadline: Instant,
    ) -> ServiceFuture<'a> {
        if cancellation.is_cancelled() {
            return Self::failure_future(&request, IpcErrorCode::Cancelled);
        }
        if Instant::now() >= deadline {
            cancellation.cancel();
            return Self::failure_future(&request, IpcErrorCode::DeadlineExceeded);
        }

        let Some(provider) = self.peer_provider.as_ref() else {
            return Self::failure_future(&request, IpcErrorCode::Unavailable);
        };

        let mapped = match self.mapping.map_request(&request) {
            Ok(mapped) => mapped,
            Err(error) => {
                return Box::pin(std::future::ready(ResponseEnvelope::failure_for_request(
                    &request, error,
                )))
            }
        };

        if let Some(response) = cancellation_or_deadline_failure(&request, &cancellation, deadline)
        {
            return Box::pin(std::future::ready(response));
        }

        let mut stream = match provider.try_take_authenticated_single_call_stream(
            request.service_id(),
            request.contract_version(),
            request.operation_id(),
        ) {
            Ok(Some(stream)) => stream,
            Ok(None) => return Self::failure_future(&request, IpcErrorCode::Unavailable),
            Err(error) => {
                return Box::pin(std::future::ready(ResponseEnvelope::failure_for_request(
                    &request, error,
                )))
            }
        };

        if let Some(response) = cancellation_or_deadline_failure(&request, &cancellation, deadline)
        {
            return Box::pin(std::future::ready(response));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            cancellation.cancel();
            return Self::failure_future(&request, IpcErrorCode::DeadlineExceeded);
        }

        let shared = Arc::new(Mutex::new(WorkerState::default()));
        let worker_shared = Arc::clone(&shared);
        let worker_request = request.clone();
        let worker_mapping = Arc::clone(&self.mapping);
        let worker_cancellation = cancellation.clone();
        let worker = match thread::Builder::new()
            .name("nagi-svc-nipc-call".to_owned())
            .spawn(move || {
                let response = catch_unwind(AssertUnwindSafe(|| {
                    run_call_worker(
                        &worker_request,
                        &mapped,
                        &mut stream,
                        worker_mapping.as_ref(),
                        &worker_cancellation,
                        deadline,
                    )
                }))
                .unwrap_or_else(|_| {
                    ResponseEnvelope::failure_for_request(
                        &worker_request,
                        IpcError::new(IpcErrorCode::TransportFailure),
                    )
                });
                publish_worker_result(&worker_shared, response);
            }) {
            Ok(worker) => worker,
            Err(_) => return Self::failure_future(&request, IpcErrorCode::Unavailable),
        };

        Box::pin(WorkerCallFuture {
            shared,
            request,
            cancellation,
            deadline,
            worker: Some(worker),
            completed: false,
        })
    }
}

impl ServiceDiscovery for UnixNipcServiceTransport {
    fn descriptors(&self) -> Vec<ServiceDescriptor> {
        self.peer_provider
            .as_ref()
            .map_or_else(Vec::new, |provider| provider.descriptors())
    }

    fn resolve(
        &self,
        service_id: &ServiceId,
        version: ContractVersion,
    ) -> Result<ServiceDescriptor, IpcError> {
        self.peer_provider.as_ref().map_or_else(
            || Err(IpcError::new(IpcErrorCode::Unavailable)),
            |provider| provider.resolve(service_id, version),
        )
    }
}

impl ServiceTransport for UnixNipcServiceTransport {
    fn call<'a>(
        &'a self,
        request: RequestEnvelope,
        cancellation: CancellationToken,
    ) -> ServiceFuture<'a> {
        match Instant::now().checked_add(self.default_timeout) {
            Some(deadline) => self.call_until(request, cancellation, deadline),
            None => Self::failure_future(&request, IpcErrorCode::DeadlineExceeded),
        }
    }

    fn call_with_deadline<'a>(
        &'a self,
        request: RequestEnvelope,
        cancellation: CancellationToken,
        deadline: Instant,
    ) -> ServiceFuture<'a> {
        self.call_until(request, cancellation, deadline)
    }
}

#[derive(Default)]
struct WorkerState {
    response: Option<ResponseEnvelope>,
    waker: Option<Waker>,
}

struct WorkerCallFuture {
    shared: Arc<Mutex<WorkerState>>,
    request: RequestEnvelope,
    cancellation: CancellationToken,
    deadline: Instant,
    worker: Option<JoinHandle<()>>,
    completed: bool,
}

impl Future for WorkerCallFuture {
    type Output = ResponseEnvelope;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let shared = Arc::clone(&self.shared);
        let request = self.request.clone();
        let cancellation = self.cancellation.clone();
        let deadline = self.deadline;
        let mut state = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(response) = state.response.take() {
            drop(state);
            self.completed = true;
            return Poll::Ready(
                cancellation_or_deadline_failure(&request, &cancellation, deadline)
                    .unwrap_or(response),
            );
        }
        if state
            .waker
            .as_ref()
            .is_none_or(|waker| !waker.will_wake(context.waker()))
        {
            state.waker = Some(context.waker().clone());
        }
        Poll::Pending
    }
}

impl Drop for WorkerCallFuture {
    fn drop(&mut self) {
        if !self.completed {
            self.cancellation.cancel();
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn publish_worker_result(shared: &Mutex<WorkerState>, response: ResponseEnvelope) {
    let waker = {
        let mut state = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.response = Some(response);
        state.waker.take()
    };
    if let Some(waker) = waker {
        waker.wake();
    }
}

fn run_call_worker(
    request: &RequestEnvelope,
    mapped: &MappedNipcRequest,
    stream: &mut UnixStream,
    mapping: &dyn ServiceNipcMapping,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> ResponseEnvelope {
    if let Some(response) = cancellation_or_deadline_failure(request, cancellation, deadline) {
        return response;
    }
    let nipc_request = mapped.envelope();
    let mut reply_storage = Vec::new();
    let result = call_unix_stream_until(
        stream,
        &nipc_request,
        &mut reply_storage,
        cancellation,
        deadline,
    );
    if let Some(response) = cancellation_or_deadline_failure(request, cancellation, deadline) {
        return response;
    }

    match result {
        Ok(CallReply::Response(reply)) => {
            if reply.kind != MessageKind::Response
                || !mapped.matches_reply_ids(&reply)
                || !mapped.matches_reply_route(&reply)
            {
                return failure(request, IpcErrorCode::TransportFailure);
            }
            let decoded = mapping.decode_response(request, &reply);
            if let Some(response) =
                cancellation_or_deadline_failure(request, cancellation, deadline)
            {
                return response;
            }
            match decoded {
                Ok(payload) => ResponseEnvelope::success_for_request(request, payload),
                Err(error) => ResponseEnvelope::failure_for_request(request, error),
            }
        }
        Ok(CallReply::Error { envelope, details }) => {
            if envelope.kind != MessageKind::Error
                || !mapped.matches_reply_ids(&envelope)
                || !mapped.matches_reply_route(&envelope)
                || details.correlation_id != mapped.correlation_id
            {
                return failure(request, IpcErrorCode::TransportFailure);
            }
            let error = mapping.decode_error(request, &details);
            if let Some(response) =
                cancellation_or_deadline_failure(request, cancellation, deadline)
            {
                return response;
            }
            ResponseEnvelope::failure_for_request(request, error)
        }
        Err(error) => failure(request, map_driver_error(error)),
    }
}

fn cancellation_or_deadline_failure(
    request: &RequestEnvelope,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Option<ResponseEnvelope> {
    if Instant::now() >= deadline {
        cancellation.cancel();
        Some(failure(request, IpcErrorCode::DeadlineExceeded))
    } else if cancellation.is_cancelled() {
        Some(failure(request, IpcErrorCode::Cancelled))
    } else {
        None
    }
}

fn failure(request: &RequestEnvelope, code: IpcErrorCode) -> ResponseEnvelope {
    ResponseEnvelope::failure_for_request(request, IpcError::new(code))
}

fn map_driver_error(error: UnixCallError) -> IpcErrorCode {
    match error {
        UnixCallError::Cancelled => IpcErrorCode::Cancelled,
        UnixCallError::TimedOut | UnixCallError::InvalidTimeout => IpcErrorCode::DeadlineExceeded,
        UnixCallError::Io(_) | UnixCallError::Carrier(CarrierCallError::Closed) => {
            IpcErrorCode::TransportFailure
        }
        UnixCallError::Carrier(CarrierCallError::Frame(CarrierFrameError::Io(_))) => {
            IpcErrorCode::TransportFailure
        }
        UnixCallError::Carrier(CarrierCallError::Frame(CarrierFrameError::Contract(_)))
        | UnixCallError::Carrier(CarrierCallError::InvalidErrorPayload(_)) => {
            IpcErrorCode::SerializationFailure
        }
        UnixCallError::Carrier(
            CarrierCallError::InvalidRequestKind | CarrierCallError::MissingRequestId,
        ) => IpcErrorCode::InvalidRequest,
        UnixCallError::Carrier(
            CarrierCallError::UnexpectedReplyKind(_)
            | CarrierCallError::CorrelationIdMismatch
            | CarrierCallError::RequestIdMismatch
            | CarrierCallError::ErrorCorrelationIdMismatch,
        ) => IpcErrorCode::TransportFailure,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::Read;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::task::{Context, Poll, Wake, Waker};
    use std::thread;
    use std::time::{Duration, Instant};

    use nagi_nipc_v1_stream_candidate::unix::{receive, send};
    use nagi_sdk::app_contract::{
        CorrelationId as NipcCorrelationId, ErrorCode as NipcErrorCode, ErrorEnvelope, IpcEnvelope,
        MessageKind, OrderingMetadata, RequestId as NipcRequestId, IPC_PROTOCOL_VERSION,
    };
    use nagi_sdk::AppId;
    use nagi_service_contract::{
        CancellationToken, ContractVersion, IpcError, IpcErrorCode, OperationId, RequestEnvelope,
        RequestId, ResponseEnvelope, ServiceClient, ServiceDescriptor, ServiceDiscovery, ServiceId,
        ServiceOperation,
    };

    use super::{
        AuthenticatedUnixStreamProvider, MappedNipcRequest, ServiceNipcMapping,
        UnixNipcServiceTransport,
    };

    const TEST_SERVICE: &str = "candidate.echo";
    const TEST_OPERATION: &str = "roundtrip";
    const NIPC_SOURCE: AppId = AppId::from_identifier(b"candidate.init");
    const NIPC_DESTINATION: AppId = AppId::from_identifier(b"candidate.echo");

    fn svc_request(id: u128, payload: &[u8]) -> RequestEnvelope {
        RequestEnvelope::new(
            RequestId::new(id).unwrap(),
            None,
            None,
            ServiceId::new(TEST_SERVICE).unwrap(),
            ContractVersion::new(1, 0).unwrap(),
            OperationId::new(TEST_OPERATION).unwrap(),
            payload.to_vec(),
        )
        .unwrap()
    }

    struct ExplicitTestMapping {
        reject: bool,
    }

    impl ServiceNipcMapping for ExplicitTestMapping {
        fn map_request(&self, request: &RequestEnvelope) -> Result<MappedNipcRequest, IpcError> {
            if self.reject
                || request.service_id().as_str() != TEST_SERVICE
                || request.contract_version() != ContractVersion::new(1, 0).unwrap()
                || request.operation_id().as_str() != TEST_OPERATION
            {
                return Err(IpcError::new(IpcErrorCode::OperationNotFound));
            }
            Ok(MappedNipcRequest::new(
                NIPC_SOURCE,
                NIPC_DESTINATION,
                NipcCorrelationId([0x5a; 16]),
                NipcRequestId([0x49; 16]),
                OrderingMetadata {
                    timestamp_micros: None,
                    sequence: None,
                },
                "candidate.echo.request@1",
                request.payload().to_vec(),
            ))
        }

        fn decode_response(
            &self,
            _request: &RequestEnvelope,
            response: &IpcEnvelope<'_>,
        ) -> Result<Vec<u8>, IpcError> {
            if response.payload_type != "candidate.echo.reply@1" {
                return Err(IpcError::new(IpcErrorCode::SerializationFailure));
            }
            Ok(response.payload.to_vec())
        }

        fn decode_error(&self, _request: &RequestEnvelope, error: &ErrorEnvelope<'_>) -> IpcError {
            IpcError::new(match error.code {
                NipcErrorCode::PermissionDenied => IpcErrorCode::PermissionDenied,
                NipcErrorCode::RuntimeUnavailable => IpcErrorCode::Unavailable,
                _ => IpcErrorCode::ProviderFailure,
            })
        }
    }

    struct TestPeerProvider {
        descriptor: ServiceDescriptor,
        streams: Mutex<VecDeque<UnixStream>>,
        checked_out: AtomicUsize,
    }

    impl TestPeerProvider {
        fn new(stream: UnixStream) -> Self {
            Self {
                descriptor: ServiceDescriptor::new(
                    ServiceId::new(TEST_SERVICE).unwrap(),
                    [ContractVersion::new(1, 0).unwrap()],
                    [ServiceOperation::new(
                        OperationId::new(TEST_OPERATION).unwrap(),
                        None,
                    )],
                )
                .unwrap(),
                streams: Mutex::new(VecDeque::from([stream])),
                checked_out: AtomicUsize::new(0),
            }
        }

        fn drop_unused_streams(&self) {
            self.streams
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clear();
        }
    }

    impl ServiceDiscovery for TestPeerProvider {
        fn descriptors(&self) -> Vec<ServiceDescriptor> {
            vec![self.descriptor.clone()]
        }

        fn resolve(
            &self,
            service_id: &ServiceId,
            version: ContractVersion,
        ) -> Result<ServiceDescriptor, IpcError> {
            if self.descriptor.id() != service_id {
                return Err(IpcError::new(IpcErrorCode::ServiceNotFound));
            }
            if !self.descriptor.supports_version(version) {
                return Err(IpcError::unsupported_version(
                    version,
                    self.descriptor.versions(),
                ));
            }
            Ok(self.descriptor.clone())
        }
    }

    impl AuthenticatedUnixStreamProvider for TestPeerProvider {
        fn try_take_authenticated_single_call_stream(
            &self,
            service_id: &ServiceId,
            version: ContractVersion,
            operation_id: &OperationId,
        ) -> Result<Option<UnixStream>, IpcError> {
            if self.descriptor.id() != service_id
                || !self.descriptor.supports_version(version)
                || self.descriptor.operation(operation_id).is_none()
            {
                return Err(IpcError::new(IpcErrorCode::PermissionDenied));
            }
            let stream = self
                .streams
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop_front();
            if stream.is_some() {
                self.checked_out.fetch_add(1, Ordering::SeqCst);
            }
            Ok(stream)
        }
    }

    struct ThreadWake(thread::Thread);

    impl Wake for ThreadWake {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    fn poll_once<F: std::future::Future>(mut future: std::pin::Pin<&mut F>) -> Poll<F::Output> {
        let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
        let mut context = Context::from_waker(&waker);
        std::future::Future::poll(future.as_mut(), &mut context)
    }

    fn poll_ready<F: std::future::Future>(future: F) -> F::Output {
        let mut future = Box::pin(future);
        let stop_at = Instant::now() + Duration::from_secs(3);
        loop {
            match poll_once(future.as_mut()) {
                Poll::Ready(output) => return output,
                Poll::Pending => {
                    let remaining = stop_at.saturating_duration_since(Instant::now());
                    assert!(!remaining.is_zero(), "worker response deadline expired");
                    thread::park_timeout(remaining.min(Duration::from_millis(20)));
                }
            }
        }
    }

    fn transport(
        stream: UnixStream,
        timeout: Duration,
        mapping: ExplicitTestMapping,
    ) -> (UnixNipcServiceTransport, Arc<TestPeerProvider>) {
        let provider = Arc::new(TestPeerProvider::new(stream));
        let transport = UnixNipcServiceTransport::with_peer_provider(
            provider.clone(),
            Arc::new(mapping),
            timeout,
        )
        .unwrap();
        (transport, provider)
    }

    fn respond_once(
        mut peer: UnixStream,
        reply_payload: &'static [u8],
        reply_type: &'static str,
        source: AppId,
        destination: AppId,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let frame = receive(&mut peer).unwrap().unwrap();
            let request = IpcEnvelope::decode(&frame).unwrap();
            let response = IpcEnvelope {
                protocol_version: IPC_PROTOCOL_VERSION,
                source,
                destination: Some(destination),
                kind: MessageKind::Response,
                correlation_id: request.correlation_id,
                request_id: request.request_id,
                ordering: request.ordering,
                payload_type: reply_type,
                payload: reply_payload,
            };
            send(&mut peer, &response).unwrap();
            let mut byte = [0_u8; 1];
            assert_eq!(peer.read(&mut byte).unwrap(), 0);
        })
    }

    fn ready_response(request: &RequestEnvelope, response: ResponseEnvelope) -> ResponseEnvelope {
        assert_eq!(response.metadata().request_id(), request.request_id());
        assert_eq!(response.metadata().service_id(), request.service_id());
        assert_eq!(
            response.metadata().contract_version(),
            request.contract_version()
        );
        assert_eq!(response.metadata().operation_id(), request.operation_id());
        response
    }

    #[test]
    fn unix_service_transport_keeps_socket_io_off_call_and_poll_and_reaps_worker() {
        let (client_stream, peer_stream) = UnixStream::pair().unwrap();
        let peer = respond_once(
            peer_stream,
            b"roundtrip reply",
            "candidate.echo.reply@1",
            NIPC_DESTINATION,
            NIPC_SOURCE,
        );
        let (transport, provider) = transport(
            client_stream,
            Duration::from_secs(2),
            ExplicitTestMapping { reject: false },
        );
        let request = svc_request(44, b"roundtrip request");
        let service_client = ServiceClient::new(Arc::new(transport));
        service_client
            .resolve(request.service_id(), request.contract_version())
            .unwrap();
        let response = poll_ready(service_client.call_with_deadline(
            request.clone(),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(2),
        ));
        let response = ready_response(&request, response);
        assert_eq!(response.result().unwrap(), b"roundtrip reply");
        peer.join().unwrap();
        assert_eq!(provider.checked_out.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn missing_peer_authority_fails_closed_without_dispatch() {
        let transport = UnixNipcServiceTransport::without_peer_authority(
            Arc::new(ExplicitTestMapping { reject: false }),
            Duration::from_secs(1),
        )
        .unwrap();
        let client = ServiceClient::new(Arc::new(transport));
        let request = svc_request(45, b"no authority");
        assert_eq!(client.descriptors(), Vec::new());
        assert_eq!(
            client.resolve(request.service_id(), request.contract_version()),
            Err(IpcError::new(IpcErrorCode::Unavailable))
        );
        let response = poll_ready(client.call(request.clone(), CancellationToken::new()));
        assert_eq!(response.metadata().request_id(), request.request_id());
        assert_eq!(
            response.result().unwrap_err().code(),
            IpcErrorCode::Unavailable
        );
    }

    #[test]
    fn mapping_rejection_does_not_consume_a_session_or_start_io() {
        let (client_stream, mut peer_stream) = UnixStream::pair().unwrap();
        let (transport, provider) = transport(
            client_stream,
            Duration::from_secs(1),
            ExplicitTestMapping { reject: true },
        );
        let client = ServiceClient::new(Arc::new(transport));
        let request = svc_request(46, b"reject mapping");
        let response = poll_ready(client.call(request.clone(), CancellationToken::new()));
        assert_eq!(response.metadata().request_id(), request.request_id());
        assert_eq!(
            response.result().unwrap_err().code(),
            IpcErrorCode::OperationNotFound
        );
        assert_eq!(provider.checked_out.load(Ordering::SeqCst), 0);
        provider.drop_unused_streams();
        let mut byte = [0_u8; 1];
        assert_eq!(peer_stream.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn mismatched_response_route_never_returns_success() {
        let (client_stream, peer_stream) = UnixStream::pair().unwrap();
        let peer = respond_once(
            peer_stream,
            b"wrong route",
            "candidate.echo.reply@1",
            AppId::from_identifier(b"candidate.other"),
            NIPC_SOURCE,
        );
        let (transport, _) = transport(
            client_stream,
            Duration::from_secs(2),
            ExplicitTestMapping { reject: false },
        );
        let client = ServiceClient::new(Arc::new(transport));
        let request = svc_request(47, b"wrong route");
        let response = poll_ready(client.call_with_deadline(
            request.clone(),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(2),
        ));
        assert_eq!(response.metadata().request_id(), request.request_id());
        assert_eq!(
            response.result().unwrap_err().code(),
            IpcErrorCode::TransportFailure
        );
        peer.join().unwrap();
    }

    #[test]
    fn cancellation_drops_future_shuts_socket_and_joins_io_worker() {
        let (client_stream, mut peer_stream) = UnixStream::pair().unwrap();
        let (request_seen_tx, request_seen_rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            let frame = receive(&mut peer_stream).unwrap().unwrap();
            let _request = IpcEnvelope::decode(&frame).unwrap();
            request_seen_tx.send(()).unwrap();
            let mut byte = [0_u8; 1];
            assert_eq!(peer_stream.read(&mut byte).unwrap(), 0);
        });
        let (transport, provider) = transport(
            client_stream,
            Duration::from_secs(5),
            ExplicitTestMapping { reject: false },
        );
        let client = ServiceClient::new(Arc::new(transport));
        let cancellation = CancellationToken::new();
        let mut future = Box::pin(client.call(svc_request(48, b"cancel me"), cancellation.clone()));

        assert!(matches!(poll_once(future.as_mut()), Poll::Pending));
        request_seen_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        cancellation.cancel();
        drop(future);
        peer.join().unwrap();
        assert_eq!(provider.checked_out.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn absolute_deadline_reaches_driver_and_reaps_worker() {
        let (client_stream, mut peer_stream) = UnixStream::pair().unwrap();
        let (request_seen_tx, request_seen_rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            let frame = receive(&mut peer_stream).unwrap().unwrap();
            let _request = IpcEnvelope::decode(&frame).unwrap();
            request_seen_tx.send(()).unwrap();
            let mut byte = [0_u8; 1];
            assert_eq!(peer_stream.read(&mut byte).unwrap(), 0);
        });
        let (transport, _) = transport(
            client_stream,
            Duration::from_secs(5),
            ExplicitTestMapping { reject: false },
        );
        let client = ServiceClient::new(Arc::new(transport));
        let deadline = Instant::now() + Duration::from_millis(100);
        let response = poll_ready(client.call_with_deadline(
            svc_request(49, b"deadline"),
            CancellationToken::new(),
            deadline,
        ));
        request_seen_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        // Poll until the worker's deadline watcher has shut down the stream.
        assert_eq!(
            response.result().unwrap_err().code(),
            IpcErrorCode::DeadlineExceeded
        );
        peer.join().unwrap();
    }

    #[test]
    fn caller_deadline_cancels_same_token_and_joins_worker_before_return() {
        let (client_stream, mut peer_stream) = UnixStream::pair().unwrap();
        let (request_seen_tx, request_seen_rx) = mpsc::channel();
        let peer = thread::spawn(move || {
            let frame = receive(&mut peer_stream).unwrap().unwrap();
            let _request = IpcEnvelope::decode(&frame).unwrap();
            request_seen_tx.send(()).unwrap();
            let mut byte = [0_u8; 1];
            assert_eq!(peer_stream.read(&mut byte).unwrap(), 0);
        });
        let (transport, _) = transport(
            client_stream,
            Duration::from_secs(5),
            ExplicitTestMapping { reject: false },
        );
        let client = ServiceClient::new(Arc::new(transport));
        let cancellation = CancellationToken::new();
        let result = crate::call_registered_blocking(
            &client,
            svc_request(52, b"outer deadline"),
            cancellation.clone(),
            Duration::from_millis(100),
        );

        assert_eq!(result, Err(crate::CallError::TimedOut));
        assert!(cancellation.is_cancelled());
        request_seen_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        // call_registered_blocking drops the worker future; its Drop joins the
        // socket worker, so peer EOF is observed before the caller returns.
        peer.join().unwrap();
    }

    #[test]
    fn peer_disconnect_returns_failure_and_closes_owned_worker() {
        let (client_stream, mut peer_stream) = UnixStream::pair().unwrap();
        let peer = thread::spawn(move || {
            let frame = receive(&mut peer_stream).unwrap().unwrap();
            let _request = IpcEnvelope::decode(&frame).unwrap();
            // Dropping the peer after request receipt produces an EOF response.
        });
        let (transport, _) = transport(
            client_stream,
            Duration::from_secs(2),
            ExplicitTestMapping { reject: false },
        );
        let client = ServiceClient::new(Arc::new(transport));
        let request = svc_request(50, b"disconnect");
        let response = poll_ready(client.call_with_deadline(
            request.clone(),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(2),
        ));
        assert_eq!(response.metadata().request_id(), request.request_id());
        assert!(response.result().is_err());
        peer.join().unwrap();
    }

    #[test]
    fn unsupported_target_mapping_is_rejected_before_stream_checkout() {
        let (client_stream, mut peer_stream) = UnixStream::pair().unwrap();
        let (transport, provider) = transport(
            client_stream,
            Duration::from_secs(2),
            ExplicitTestMapping { reject: false },
        );
        let client = ServiceClient::new(Arc::new(transport));
        let unsupported = RequestEnvelope::new(
            RequestId::new(51).unwrap(),
            None,
            None,
            ServiceId::new("candidate.unmapped").unwrap(),
            ContractVersion::new(1, 0).unwrap(),
            OperationId::new(TEST_OPERATION).unwrap(),
            b"not mapped".to_vec(),
        )
        .unwrap();
        let response = poll_ready(client.call(unsupported.clone(), CancellationToken::new()));
        assert_eq!(response.metadata().request_id(), unsupported.request_id());
        assert_eq!(
            response.result().unwrap_err().code(),
            IpcErrorCode::OperationNotFound
        );
        assert_eq!(provider.checked_out.load(Ordering::SeqCst), 0);
        provider.drop_unused_streams();
        let mut byte = [0_u8; 1];
        assert_eq!(peer_stream.read(&mut byte).unwrap(), 0);
    }
}
