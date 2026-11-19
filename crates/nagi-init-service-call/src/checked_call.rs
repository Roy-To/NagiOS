#![allow(dead_code)] // Consumed by the private linux_init adapter when integrated.

use nagi_service_contract::{
    CancellationToken, IpcError, IpcErrorCode, RequestEnvelope, ResponseEnvelope, ServiceClient,
};
use std::time::Instant;

/// A consumer-side failure before init may accept an IPC result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CallError {
    Discovery(IpcError),
    Cancelled,
    TimedOut,
    InvalidTimeout,
    ResponseMismatch,
    Remote(IpcError),
}

/// Call the already connected runtime transport only after discovery has
/// confirmed the exact service version and operation. The injected production
/// transport's `resolve` must reflect its live provider registration. The
/// response must match every request identity field and be successful before
/// it is returned.
///
/// `ServiceClient::resolve` and `ServiceClient::call` are synchronous trait
/// calls before a `ServiceFuture` is available. This function checks the
/// deadline after each returns and will not start or poll an expired call, but
/// cannot preempt either synchronous method. Production implementations must
/// keep those stages bounded and put blocking I/O in a cancellation-aware,
/// deadline-aware driver.
///
/// The private `InitBackend::call_service` adapter can call this after its
/// owner-supplied typed Action/Reply mapping builds the `RequestEnvelope` and
/// decodes the successful response payload. This function never constructs a
/// `PrincipalId`, converts a UID/PID, emits a `LifecycleEvent`, or creates
/// READY authority. `ServiceTransport` remains responsible for deriving
/// caller identity from its trusted runtime boundary.
pub(crate) async fn call_registered(
    client: &ServiceClient,
    request: RequestEnvelope,
    cancellation: CancellationToken,
    deadline: Instant,
) -> Result<ResponseEnvelope, CallError> {
    check_deadline(&cancellation, deadline)?;

    let resolved = client.resolve(request.service_id(), request.contract_version());
    check_deadline(&cancellation, deadline)?;
    let descriptor = resolved.map_err(CallError::Discovery)?;
    if descriptor.id() != request.service_id() {
        return Err(CallError::Discovery(IpcError::new(
            IpcErrorCode::ServiceNotFound,
        )));
    }
    if !descriptor.supports_version(request.contract_version()) {
        return Err(CallError::Discovery(IpcError::unsupported_version(
            request.contract_version(),
            descriptor.versions(),
        )));
    }
    if descriptor.operation(request.operation_id()).is_none() {
        return Err(CallError::Discovery(IpcError::new(
            IpcErrorCode::OperationNotFound,
        )));
    }
    check_deadline(&cancellation, deadline)?;

    let call = client.call_with_deadline(request.clone(), cancellation.clone(), deadline);
    check_deadline(&cancellation, deadline)?;
    let response = call.await;
    check_deadline(&cancellation, deadline)?;
    if !response_matches(&request, &response) {
        return Err(CallError::ResponseMismatch);
    }
    if let Err(error) = response.result() {
        return Err(CallError::Remote(error.clone()));
    }
    Ok(response)
}

fn check_deadline(cancellation: &CancellationToken, deadline: Instant) -> Result<(), CallError> {
    if Instant::now() >= deadline {
        cancellation.cancel();
        return Err(CallError::TimedOut);
    }
    if cancellation.is_cancelled() {
        return Err(CallError::Cancelled);
    }
    Ok(())
}

fn response_matches(request: &RequestEnvelope, response: &ResponseEnvelope) -> bool {
    let metadata = response.metadata();
    metadata.request_id() == request.request_id()
        && metadata.correlation_id() == request.correlation_id()
        && metadata.trace_id() == request.trace_id()
        && metadata.service_id() == request.service_id()
        && metadata.contract_version() == request.contract_version()
        && metadata.operation_id() == request.operation_id()
}
