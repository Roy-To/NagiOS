use std::future::Future;
use std::io;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::task::{Context, Wake, Waker};
use std::thread;
use std::time::{Duration, Instant};

use nagi_nipc_v1_stream_candidate::{CallError as CarrierCallError, CallReply};
use nagi_sdk::app_contract::{IpcEnvelope, MessageKind};
use nagi_service_contract::CancellationToken;

const NONE_INTERRUPTED: u8 = 0;
const CALL_CANCELLED: u8 = 1;
const CALL_TIMED_OUT: u8 = 2;

struct ThreadUnparker(thread::Thread);

impl Wake for ThreadUnparker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// Errors from the existing NIPC v1 UnixStream call path.
#[derive(Debug)]
pub enum UnixCallError {
    Cancelled,
    TimedOut,
    InvalidTimeout,
    Io(io::Error),
    Carrier(CarrierCallError),
}

/// Drive one already-mapped NIPC request over an already-connected Unix stream
/// using the carrier's synchronous call path.
///
/// The caller supplies the stream only after its own peer/registration checks,
/// and supplies a fully constructed SDK envelope. This function does not map
/// SVC identifiers, choose an AppId, authenticate peers, or create PrincipalId.
/// The relative timeout is converted once to an absolute deadline at entry.
/// Synchronous validation, stream cloning, and watcher creation are not
/// preempted, but each stage rechecks that original deadline before carrier
/// I/O; callers which already own a deadline can pass the same `Instant` to
/// the internal deadline entry point. The function does not change socket
/// mode or wait for readiness; a nonblocking stream can return `WouldBlock`.
/// Socket shutdown wakes a blocked read/write on cancellation or the overall
/// deadline. A socket-level timeout or nonblocking `WouldBlock` before that
/// deadline is returned as a carrier I/O error, not as expiry of the call
/// deadline. Pre-send envelope validation errors leave the stream open; any
/// error after entering the carrier call closes it because send progress may
/// be unknown.
pub fn call_unix_stream<'a>(
    stream: &mut UnixStream,
    request: &IpcEnvelope<'_>,
    reply_storage: &'a mut Vec<u8>,
    cancellation: &CancellationToken,
    timeout: Duration,
) -> Result<CallReply<'a>, UnixCallError> {
    if cancellation.is_cancelled() {
        return Err(UnixCallError::Cancelled);
    }
    if timeout.is_zero() {
        return Err(UnixCallError::TimedOut);
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(UnixCallError::InvalidTimeout)?;
    call_unix_stream_until(stream, request, reply_storage, cancellation, deadline)
}

/// Drive one carrier call against the caller's original absolute deadline.
/// Keeping the `Instant` intact prevents worker scheduling delay from being
/// converted into a fresh relative timeout.
pub(crate) fn call_unix_stream_until<'a>(
    stream: &mut UnixStream,
    request: &IpcEnvelope<'_>,
    reply_storage: &'a mut Vec<u8>,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<CallReply<'a>, UnixCallError> {
    if cancellation.is_cancelled() {
        return Err(UnixCallError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(UnixCallError::TimedOut);
    }
    validate_before_send(request)?;
    if cancellation.is_cancelled() {
        return Err(UnixCallError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(UnixCallError::TimedOut);
    }

    let abort_stream = stream.try_clone().map_err(UnixCallError::Io)?;
    if cancellation.is_cancelled() {
        return Err(UnixCallError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(UnixCallError::TimedOut);
    }

    let interrupted = Arc::new(AtomicU8::new(NONE_INTERRUPTED));
    let stop_watcher = Arc::new(AtomicBool::new(false));
    let watcher_interrupted = Arc::clone(&interrupted);
    let watcher_stop = Arc::clone(&stop_watcher);
    let watcher_cancellation = cancellation.clone();
    let watcher = match thread::Builder::new()
        .name("nagi-nipc-call-deadline".to_owned())
        .spawn(move || {
            let worker = thread::current();
            let waker = Waker::from(Arc::new(ThreadUnparker(worker.clone())));
            let mut context = Context::from_waker(&waker);
            let mut cancellation_future = Box::pin(watcher_cancellation.cancelled());
            loop {
                if watcher_stop.load(Ordering::Acquire) {
                    break;
                }
                let cancellation_ready = cancellation_future.as_mut().poll(&mut context).is_ready();
                let reason = if cancellation_ready {
                    Some(CALL_CANCELLED)
                } else if Instant::now() >= deadline {
                    Some(CALL_TIMED_OUT)
                } else {
                    None
                };
                if let Some(reason) = reason {
                    let _ = watcher_interrupted.compare_exchange(
                        NONE_INTERRUPTED,
                        reason,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    let _ = abort_stream.shutdown(std::net::Shutdown::Both);
                    break;
                }
                thread::park_timeout(deadline.saturating_duration_since(Instant::now()));
            }
        }) {
        Ok(watcher) => watcher,
        Err(error) => return Err(UnixCallError::Io(error)),
    };

    if cancellation.is_cancelled() || Instant::now() >= deadline {
        stop_watcher.store(true, Ordering::Release);
        watcher.thread().unpark();
        if watcher.join().is_err() {
            return Err(UnixCallError::Io(io::Error::other(
                "NIPC deadline watcher panicked",
            )));
        }
        return if cancellation.is_cancelled() {
            Err(UnixCallError::Cancelled)
        } else {
            Err(UnixCallError::TimedOut)
        };
    }

    let result = nagi_nipc_v1_stream_candidate::unix::call(stream, request, reply_storage);

    if cancellation.is_cancelled() {
        let _ = interrupted.compare_exchange(
            NONE_INTERRUPTED,
            CALL_CANCELLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    } else if Instant::now() >= deadline {
        let _ = interrupted.compare_exchange(
            NONE_INTERRUPTED,
            CALL_TIMED_OUT,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
    if interrupted.load(Ordering::Acquire) != NONE_INTERRUPTED || result.is_err() {
        let _ = stream.shutdown(std::net::Shutdown::Both);
    }

    stop_watcher.store(true, Ordering::Release);
    watcher.thread().unpark();
    if watcher.join().is_err() {
        return Err(UnixCallError::Io(io::Error::other(
            "NIPC deadline watcher panicked",
        )));
    }

    match interrupted.load(Ordering::Acquire) {
        CALL_CANCELLED => Err(UnixCallError::Cancelled),
        CALL_TIMED_OUT => Err(UnixCallError::TimedOut),
        _ => match result {
            Ok(reply) => Ok(reply),
            Err(error) => Err(UnixCallError::Carrier(error)),
        },
    }
}

fn validate_before_send(request: &IpcEnvelope<'_>) -> Result<(), UnixCallError> {
    if request.kind != MessageKind::Request {
        return Err(UnixCallError::Carrier(CarrierCallError::InvalidRequestKind));
    }
    if request.request_id.is_none() {
        return Err(UnixCallError::Carrier(CarrierCallError::MissingRequestId));
    }
    nagi_nipc_v1_stream_candidate::write_frame(&mut io::sink(), request)
        .map(|_| ())
        .map_err(|error| UnixCallError::Carrier(CarrierCallError::Frame(error)))
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use nagi_nipc_v1_stream_candidate::{CallError, CallReply, FrameError};
    use nagi_sdk::app_contract::{
        CorrelationId, IpcEnvelope, MessageKind, OrderingMetadata, RequestId, IPC_PROTOCOL_VERSION,
        MAX_IPC_PAYLOAD_BYTES,
    };
    use nagi_sdk::AppId;
    use nagi_service_contract::CancellationToken;

    use super::{call_unix_stream, call_unix_stream_until, UnixCallError};

    fn request() -> IpcEnvelope<'static> {
        IpcEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            source: AppId::from_identifier(b"com.nagi.client"),
            destination: Some(AppId::from_identifier(b"com.nagi.service")),
            kind: MessageKind::Request,
            correlation_id: CorrelationId([0x31; 16]),
            request_id: Some(RequestId([0x42; 16])),
            ordering: OrderingMetadata {
                timestamp_micros: None,
                sequence: None,
            },
            payload_type: "nagi.test@1",
            payload: b"typed request",
        }
    }

    fn response(request: &IpcEnvelope<'_>) -> IpcEnvelope<'static> {
        IpcEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            source: request.destination.expect("test request destination"),
            destination: Some(request.source),
            kind: MessageKind::Response,
            correlation_id: request.correlation_id,
            request_id: request.request_id,
            ordering: request.ordering,
            payload_type: "nagi.test.reply@1",
            payload: b"typed response",
        }
    }

    fn large_request(payload: &[u8]) -> IpcEnvelope<'_> {
        IpcEnvelope {
            payload,
            ..request()
        }
    }

    #[test]
    fn socket_call_uses_the_existing_carrier_for_request_and_response() {
        let (mut client, mut peer) = UnixStream::pair().unwrap();
        let server = thread::spawn(move || {
            let frame = nagi_nipc_v1_stream_candidate::unix::receive(&mut peer)
                .unwrap()
                .unwrap();
            let decoded = IpcEnvelope::decode(&frame).unwrap();
            let response = response(&decoded);
            nagi_nipc_v1_stream_candidate::unix::send(&mut peer, &response).unwrap();
        });

        let request = request();
        let mut reply_storage = Vec::new();
        let reply = call_unix_stream(
            &mut client,
            &request,
            &mut reply_storage,
            &CancellationToken::new(),
            Duration::from_secs(2),
        )
        .unwrap();

        match reply {
            CallReply::Response(envelope) => {
                assert_eq!(envelope.kind, MessageKind::Response);
                assert_eq!(envelope.payload, b"typed response");
            }
            CallReply::Error { .. } => panic!("expected typed response"),
        }
        server.join().unwrap();
    }

    #[test]
    fn peer_disconnect_is_returned_as_a_closed_carrier_call() {
        let (mut client, mut peer) = UnixStream::pair().unwrap();
        let server = thread::spawn(move || {
            let _request = nagi_nipc_v1_stream_candidate::unix::receive(&mut peer)
                .unwrap()
                .unwrap();
        });
        let request = request();
        let mut reply_storage = Vec::new();
        let result = call_unix_stream(
            &mut client,
            &request,
            &mut reply_storage,
            &CancellationToken::new(),
            Duration::from_secs(2),
        );

        assert!(matches!(
            result,
            Err(UnixCallError::Carrier(CallError::Closed))
        ));
        server.join().unwrap();
    }

    #[test]
    fn socket_call_rejects_a_response_with_the_wrong_correlation_id() {
        let (mut client, mut peer) = UnixStream::pair().unwrap();
        let server = thread::spawn(move || {
            let frame = nagi_nipc_v1_stream_candidate::unix::receive(&mut peer)
                .unwrap()
                .unwrap();
            let decoded = IpcEnvelope::decode(&frame).unwrap();
            let mut response = response(&decoded);
            response.correlation_id = CorrelationId([0x99; 16]);
            nagi_nipc_v1_stream_candidate::unix::send(&mut peer, &response).unwrap();
            let mut byte = [0_u8; 1];
            assert_eq!(peer.read(&mut byte).unwrap(), 0);
        });
        let request = request();
        let mut reply_storage = Vec::new();
        let result = call_unix_stream(
            &mut client,
            &request,
            &mut reply_storage,
            &CancellationToken::new(),
            Duration::from_secs(2),
        );

        assert!(matches!(
            result,
            Err(UnixCallError::Carrier(CallError::CorrelationIdMismatch))
        ));
        server.join().unwrap();
    }

    #[test]
    fn would_block_before_the_global_deadline_is_a_carrier_io_error() {
        let (mut client, mut peer) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        let request = request();
        let mut reply_storage = Vec::new();
        let started = Instant::now();
        let result = call_unix_stream(
            &mut client,
            &request,
            &mut reply_storage,
            &CancellationToken::new(),
            Duration::from_secs(2),
        );

        assert!(matches!(
            result,
            Err(UnixCallError::Carrier(CallError::Frame(FrameError::Io(error))))
                if error.kind() == io::ErrorKind::WouldBlock
        ));
        assert!(started.elapsed() < Duration::from_secs(1));
        let mut drained = Vec::new();
        peer.read_to_end(&mut drained).unwrap();
    }

    #[test]
    fn expired_absolute_deadline_is_not_rebased_when_driver_starts_late() {
        let (mut client, mut peer) = UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        let request = request();
        let mut reply_storage = Vec::new();
        let deadline = Instant::now() + Duration::from_millis(40);
        thread::sleep(Duration::from_millis(60));

        assert!(matches!(
            call_unix_stream_until(
                &mut client,
                &request,
                &mut reply_storage,
                &CancellationToken::new(),
                deadline,
            ),
            Err(UnixCallError::TimedOut)
        ));

        let mut byte = [0_u8; 1];
        assert!(matches!(
            peer.read(&mut byte),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock
        ));
    }

    #[test]
    fn envelope_encode_error_before_any_write_keeps_the_socket_open() {
        let (mut client, mut peer) = UnixStream::pair().unwrap();
        let mut invalid_request = request();
        invalid_request.protocol_version = IPC_PROTOCOL_VERSION + 1;
        let mut reply_storage = Vec::new();
        let result = call_unix_stream(
            &mut client,
            &invalid_request,
            &mut reply_storage,
            &CancellationToken::new(),
            Duration::from_secs(2),
        );

        assert!(matches!(
            result,
            Err(UnixCallError::Carrier(CallError::Frame(
                FrameError::Contract(_)
            )))
        ));
        client.write_all(b"still open").unwrap();
        let mut probe = [0_u8; 10];
        peer.read_exact(&mut probe).unwrap();
        assert_eq!(&probe, b"still open");
    }

    #[test]
    fn cancellation_shuts_down_a_blocked_socket_read() {
        let (mut client, mut peer) = UnixStream::pair().unwrap();
        let (request_received, received) = mpsc::channel();
        let server = thread::spawn(move || {
            let _request = nagi_nipc_v1_stream_candidate::unix::receive(&mut peer)
                .unwrap()
                .unwrap();
            request_received.send(()).unwrap();
            let mut byte = [0_u8; 1];
            let _ = peer.read(&mut byte);
        });
        let cancellation = CancellationToken::new();
        let cancel_token = cancellation.clone();
        let canceller = thread::spawn(move || {
            received
                .recv_timeout(Duration::from_secs(2))
                .expect("peer should receive the request before cancellation");
            cancel_token.cancel();
        });
        let request = request();
        let mut reply_storage = Vec::new();
        let result = call_unix_stream(
            &mut client,
            &request,
            &mut reply_storage,
            &cancellation,
            Duration::from_secs(3),
        );

        assert!(matches!(result, Err(UnixCallError::Cancelled)));
        canceller.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn cancellation_shuts_down_a_blocked_large_request_write() {
        let (mut client, mut peer) = UnixStream::pair().unwrap();
        let (first_byte_tx, first_byte_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let mut first_byte = [0_u8; 1];
            peer.read_exact(&mut first_byte).unwrap();
            first_byte_tx.send(()).unwrap();
            release_rx
                .recv_timeout(Duration::from_secs(3))
                .expect("blocked-write peer should be released");
        });
        let payload = vec![0x5a; MAX_IPC_PAYLOAD_BYTES];
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let request = large_request(&payload);
            let mut reply_storage = Vec::new();
            let result = call_unix_stream(
                &mut client,
                &request,
                &mut reply_storage,
                &worker_cancellation,
                Duration::from_secs(5),
            );
            result_tx
                .send(matches!(result, Err(UnixCallError::Cancelled)))
                .unwrap();
        });

        first_byte_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("peer should observe the beginning of the large write");
        assert!(matches!(
            result_rx.recv_timeout(Duration::from_millis(60)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        cancellation.cancel();
        assert!(result_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn call_deadline_shuts_down_a_blocked_large_request_write() {
        let (mut client, mut peer) = UnixStream::pair().unwrap();
        let (first_byte_tx, first_byte_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let mut first_byte = [0_u8; 1];
            peer.read_exact(&mut first_byte).unwrap();
            first_byte_tx.send(()).unwrap();
            release_rx
                .recv_timeout(Duration::from_secs(3))
                .expect("blocked-write peer should be released");
        });
        let payload = vec![0x5a; MAX_IPC_PAYLOAD_BYTES];
        let (result_tx, result_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let request = large_request(&payload);
            let mut reply_storage = Vec::new();
            let result = call_unix_stream(
                &mut client,
                &request,
                &mut reply_storage,
                &CancellationToken::new(),
                Duration::from_millis(200),
            );
            result_tx
                .send(matches!(result, Err(UnixCallError::TimedOut)))
                .unwrap();
        });

        first_byte_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("peer should observe the beginning of the large write");
        assert!(matches!(
            result_rx.recv_timeout(Duration::from_millis(60)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert!(result_rx.recv_timeout(Duration::from_secs(1)).unwrap());
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn cancellation_wakes_a_partial_response_read() {
        let (mut client, mut peer) = UnixStream::pair().unwrap();
        let (partial_reply_tx, partial_reply_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = thread::spawn(move || {
            let frame = nagi_nipc_v1_stream_candidate::unix::receive(&mut peer)
                .unwrap()
                .unwrap();
            let decoded = IpcEnvelope::decode(&frame).unwrap();
            let response = response(&decoded);
            let mut encoded = vec![0_u8; 78 + response.payload_type.len() + response.payload.len()];
            let encoded_len = response.encode(&mut encoded).unwrap();
            peer.write_all(&encoded[..encoded_len.min(20)]).unwrap();
            partial_reply_tx.send(()).unwrap();
            release_rx
                .recv_timeout(Duration::from_secs(3))
                .expect("partial-reply peer should be released");
        });
        let cancellation = CancellationToken::new();
        let cancel_token = cancellation.clone();
        let canceller = thread::spawn(move || {
            partial_reply_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("peer should send a partial reply before cancellation");
            cancel_token.cancel();
        });
        let request = request();
        let mut reply_storage = Vec::new();
        let result = call_unix_stream(
            &mut client,
            &request,
            &mut reply_storage,
            &cancellation,
            Duration::from_secs(3),
        );

        assert!(matches!(result, Err(UnixCallError::Cancelled)));
        canceller.join().unwrap();
        release_tx.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn whole_call_deadline_shuts_down_a_peer_that_never_replies() {
        let (mut client, mut peer) = UnixStream::pair().unwrap();
        let server = thread::spawn(move || {
            let _request = nagi_nipc_v1_stream_candidate::unix::receive(&mut peer)
                .unwrap()
                .unwrap();
            thread::sleep(Duration::from_millis(150));
            let mut byte = [0_u8; 1];
            let _ = peer.write(&mut byte);
        });
        let request = request();
        let mut reply_storage = Vec::new();
        let result = call_unix_stream(
            &mut client,
            &request,
            &mut reply_storage,
            &CancellationToken::new(),
            Duration::from_millis(40),
        );

        assert!(matches!(result, Err(UnixCallError::TimedOut)));
        server.join().unwrap();
    }
}
