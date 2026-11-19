use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread;
use std::time::{Duration, Instant};

use nagi_service_contract::{CancellationToken, RequestEnvelope, ResponseEnvelope, ServiceClient};

use crate::checked_call::{call_registered, CallError};

struct ThreadWake(thread::Thread);

impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Drives the asynchronous wait until completion, cancellation, or the
/// caller's deadline. The cancellation future registers this task's waker
/// independently of the transport, so a non-waking pending transport cannot
/// strand the synchronous caller. A deadline park bounds that wait without a
/// timer thread or polling loop.
///
/// This cannot preempt synchronous work performed while polling the future,
/// including `ServiceDiscovery::resolve`, `ServiceTransport::call` before it
/// returns its future, or a blocking future `poll`. Those stages must return
/// promptly; blocking I/O belongs in a cancellation-aware, deadline-aware
/// driver. The deadline is checked immediately after synchronous setup returns
/// so an expired request is not subsequently polled or dispatched.
fn block_on_bounded<T>(
    future: impl Future<Output = Result<T, CallError>>,
    cancellation: CancellationToken,
    deadline: Instant,
) -> Result<T, CallError> {
    if cancellation.is_cancelled() {
        return Err(CallError::Cancelled);
    }
    let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    let mut cancellation_future = Box::pin(cancellation.cancelled());
    loop {
        if Instant::now() >= deadline {
            cancellation.cancel();
            return Err(CallError::TimedOut);
        }
        if cancellation_future.as_mut().poll(&mut context).is_ready() {
            return Err(CallError::Cancelled);
        }
        match Pin::as_mut(&mut future).poll(&mut context) {
            Poll::Ready(result) => {
                if matches!(&result, Err(CallError::TimedOut)) {
                    return Err(CallError::TimedOut);
                }
                if Instant::now() >= deadline {
                    cancellation.cancel();
                    return Err(CallError::TimedOut);
                }
                if cancellation.is_cancelled() {
                    return Err(CallError::Cancelled);
                }
                return result;
            }
            Poll::Pending => {
                thread::park_timeout(deadline.saturating_duration_since(Instant::now()))
            }
        }
    }
}

/// Synchronous adapter for the existing `InitBackend::call_service` seam.
/// It preserves the contract's cancellation token and returns only a
/// registered, correlated, successful response. Expiry cancels the token and
/// drops the local transport future, but cannot prove that a remote side
/// effect did not occur before cancellation was observed.
pub fn call_registered_blocking(
    client: &ServiceClient,
    request: RequestEnvelope,
    cancellation: CancellationToken,
    timeout: Duration,
) -> Result<ResponseEnvelope, CallError> {
    if cancellation.is_cancelled() {
        return Err(CallError::Cancelled);
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(CallError::InvalidTimeout)?;
    block_on_bounded(
        call_registered(client, request, cancellation.clone(), deadline),
        cancellation,
        deadline,
    )
}
