//! Thread bridge only, not an implementation of C2's private SessionLease.
//! Co-located Task41/init callbacks must verify the trusted LaunchRecord and
//! retain the actual C1 context for the request/lease lifetime. Every check must
//! recheck that context. Numeric request IDs and channel messages are not grants.
//! L and D are the owner's real lease/error types; they must be Send to cross
//! this bridge. No borrowed C1 context crosses it, is serialized, or is cached
//! as a boolean authority. The owner pumps requests without blocking itself.
//!
//! LIMIT: the requester waits in recv_timeout. This experiment is bounded but
//! BLOCKING and must not implement C2's nonblocking LiveSessionAuthority. Its
//! real SessionLease/PeerDenial types and trusted Supervisor API are not linked.

use std::num::NonZeroUsize;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, ThreadId};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProxyFailure<D> {
    OwnerThread,
    Busy,
    Unavailable,
    TimedOut,
    Authority(D),
}

enum Request<L, D> {
    Capture {
        app: u64,
        app_session: u64,
        deadline: Instant,
        reply: Sender<Result<L, D>>,
        response: Arc<Mutex<CaptureResponseState>>,
    },
    Check {
        lease: L,
        deadline: Instant,
        reply: Sender<Result<bool, D>>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CaptureResponseState {
    Pending,
    Cancelled,
    Delivered,
}

fn lock_response(response: &Mutex<CaptureResponseState>) -> MutexGuard<'_, CaptureResponseState> {
    response
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Called only after `recv_timeout` reports Timeout. The shared mutex
/// linearizes this cancellation with the owner's nonblocking reply send:
/// either cancellation wins and the owner releases the lease, or delivery wins
/// and the reply is drained even if the timeout edge raced with the send.
fn resolve_capture_timeout<L, D>(
    reply: Receiver<Result<L, D>>,
    response: &Mutex<CaptureResponseState>,
) -> Result<L, ProxyFailure<D>> {
    let mut state = lock_response(response);
    match *state {
        CaptureResponseState::Pending => {
            *state = CaptureResponseState::Cancelled;
            Err(ProxyFailure::TimedOut)
        }
        CaptureResponseState::Cancelled => Err(ProxyFailure::TimedOut),
        CaptureResponseState::Delivered => {
            drop(state);
            reply
                .recv()
                .map_err(|_| ProxyFailure::Unavailable)?
                .map_err(ProxyFailure::Authority)
        }
    }
}

/// Send+Sync when the real C2 L/D are Send. Stores a channel, never a context.
pub struct SharedAuthorityProxy<L, D> {
    requests: SyncSender<Request<L, D>>,
    owner: ThreadId,
    timeout: Duration,
}

impl<L, D> Clone for SharedAuthorityProxy<L, D> {
    fn clone(&self) -> Self {
        Self {
            requests: self.requests.clone(),
            owner: self.owner,
            timeout: self.timeout,
        }
    }
}

pub struct OwnerRequests<L, D> {
    requests: Receiver<Request<L, D>>,
    owner: ThreadId,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnerStep {
    CaptureHandled,
    CheckHandled,
    Expired,
    UnclaimedCaptureReleased,
}

pub fn channel<L: Send, D: Send>(
    capacity: NonZeroUsize,
    timeout: Duration,
) -> (SharedAuthorityProxy<L, D>, OwnerRequests<L, D>) {
    let (sender, receiver) = mpsc::sync_channel(capacity.get());
    let owner = thread::current().id();
    (
        SharedAuthorityProxy {
            requests: sender,
            owner,
            timeout,
        },
        OwnerRequests {
            requests: receiver,
            owner,
        },
    )
}

impl<L: Send, D: Send> SharedAuthorityProxy<L, D> {
    fn deadline(&self) -> Result<Instant, ProxyFailure<D>> {
        if thread::current().id() == self.owner {
            return Err(ProxyFailure::OwnerThread);
        }
        if self.timeout.is_zero() {
            return Err(ProxyFailure::TimedOut);
        }
        Instant::now()
            .checked_add(self.timeout)
            .ok_or(ProxyFailure::TimedOut)
    }

    fn send(&self, request: Request<L, D>) -> Result<(), ProxyFailure<D>> {
        self.requests
            .try_send(request)
            .map_err(|error| match error {
                TrySendError::Full(_) => ProxyFailure::Busy,
                TrySendError::Disconnected(_) => ProxyFailure::Unavailable,
            })
    }

    fn receive<T>(
        &self,
        reply: Receiver<Result<T, D>>,
        deadline: Instant,
    ) -> Result<T, ProxyFailure<D>> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        reply
            .recv_timeout(remaining)
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => ProxyFailure::TimedOut,
                mpsc::RecvTimeoutError::Disconnected => ProxyFailure::Unavailable,
            })?
            .map_err(ProxyFailure::Authority)
    }

    pub fn capture_request(&self, app: u64, app_session: u64) -> Result<L, ProxyFailure<D>> {
        let deadline = self.deadline()?;
        let (reply, receiver) = mpsc::channel();
        let response = Arc::new(Mutex::new(CaptureResponseState::Pending));
        self.send(Request::Capture {
            app,
            app_session,
            deadline,
            reply,
            response: Arc::clone(&response),
        })?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(remaining) {
            Ok(result) => result.map_err(ProxyFailure::Authority),
            Err(mpsc::RecvTimeoutError::Timeout) => resolve_capture_timeout(receiver, &response),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(ProxyFailure::Unavailable),
        }
    }

    pub fn check_request(&self, lease: L) -> Result<bool, ProxyFailure<D>> {
        let deadline = self.deadline()?;
        let (reply, receiver) = mpsc::channel();
        self.send(Request::Check {
            lease,
            deadline,
            reply,
        })?;
        self.receive(receiver, deadline)
    }
}

impl<L: Send, D: Send> OwnerRequests<L, D> {
    /// Run on the creator/owner thread. Callbacks may borrow the real !Send C1
    /// context and trusted Supervisor. `release_unclaimed` cleans up only a new
    /// lease whose result timed out or could not be delivered to its requester.
    /// Normal C2 request cleanup remains the co-located owner's responsibility.
    pub fn poll_one(
        &mut self,
        mut capture: impl FnMut(u64, u64) -> Result<L, D>,
        mut check: impl FnMut(L) -> Result<bool, D>,
        mut release_unclaimed: impl FnMut(L),
    ) -> Result<Option<OwnerStep>, ProxyFailure<D>> {
        if thread::current().id() != self.owner {
            return Err(ProxyFailure::OwnerThread);
        }
        let request = match self.requests.try_recv() {
            Ok(request) => request,
            Err(TryRecvError::Empty) => return Ok(None),
            Err(TryRecvError::Disconnected) => return Err(ProxyFailure::Unavailable),
        };
        match request {
            Request::Capture {
                app,
                app_session,
                deadline,
                reply,
                response,
            } => {
                if Instant::now() >= deadline {
                    *lock_response(&response) = CaptureResponseState::Cancelled;
                    return Ok(Some(OwnerStep::Expired));
                }
                let result = capture(app, app_session);
                let mut state = lock_response(&response);
                if Instant::now() >= deadline {
                    *state = CaptureResponseState::Cancelled;
                    drop(state);
                    if let Ok(lease) = result {
                        release_unclaimed(lease);
                    }
                    return Ok(Some(OwnerStep::Expired));
                }
                if *state == CaptureResponseState::Cancelled {
                    drop(state);
                    if let Ok(lease) = result {
                        release_unclaimed(lease);
                    }
                    return Ok(Some(OwnerStep::UnclaimedCaptureReleased));
                }
                match reply.send(result) {
                    Ok(()) => {
                        *state = CaptureResponseState::Delivered;
                        Ok(Some(OwnerStep::CaptureHandled))
                    }
                    Err(error) => {
                        *state = CaptureResponseState::Cancelled;
                        drop(state);
                        if let Ok(lease) = error.0 {
                            release_unclaimed(lease);
                        }
                        Ok(Some(OwnerStep::UnclaimedCaptureReleased))
                    }
                }
            }
            Request::Check {
                lease,
                deadline,
                reply,
            } => {
                if Instant::now() >= deadline {
                    return Ok(Some(OwnerStep::Expired));
                }
                let result = check(lease);
                if Instant::now() >= deadline {
                    return Ok(Some(OwnerStep::Expired));
                }
                let _ = reply.send(result);
                Ok(Some(OwnerStep::CheckHandled))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{lock_response, resolve_capture_timeout, CaptureResponseState};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    #[test]
    fn timeout_after_owner_send_drains_the_delivered_lease() {
        let response = Arc::new(Mutex::new(CaptureResponseState::Pending));
        let (sender, receiver) = mpsc::channel();
        let mut state = lock_response(&response);
        sender.send(Ok::<_, &'static str>(17_u8)).unwrap();
        *state = CaptureResponseState::Delivered;
        drop(state);

        assert_eq!(resolve_capture_timeout(receiver, &response), Ok(17));
    }

    #[test]
    fn timeout_before_owner_send_cancels_the_reply() {
        let response = Arc::new(Mutex::new(CaptureResponseState::Pending));
        let (_sender, receiver) = mpsc::channel::<Result<u8, &'static str>>();

        assert_eq!(
            resolve_capture_timeout(receiver, &response),
            Err(super::ProxyFailure::TimedOut)
        );
        assert_eq!(*lock_response(&response), CaptureResponseState::Cancelled);
    }
}
