//! Candidate Unix listener adapter for a future Linux init-owned server.
//!
//! The source Linux init tree currently has no production Unix listener or
//! accepted-stream callsite. This adapter is not wired into `linux_init::run`.
//! Its sole purpose is to bind an accepted stream to the existing Linux
//! credential snapshot and bounded NIPC reader. A missing live authority
//! supplier returns `Unavailable` before reading or dispatching a request.

use crate::{
    KernelPeerCredentials, OwnedPeerConnection, OwnedPeerStreamTransfer, PeerConnectionError,
    PeerConnectionIo,
};
use nagi_sdk::app_contract::{
    IpcEnvelope, MessageKind, IPC_PROTOCOL_VERSION, MAX_IPC_PAYLOAD_BYTES,
};
use std::error::Error;
use std::fmt;
use std::io::{self, Read};
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::{Duration, Instant};

const FIXED_HEADER_BYTES: usize = 78;
const MAX_PAYLOAD_TYPE_BYTES: usize = 128;
const PAYLOAD_TYPE_LEN_OFFSET: usize = 72;
const PAYLOAD_LEN_OFFSET: usize = 74;
const MAGIC: &[u8; 4] = b"NIPC";

/// Errors from listener acceptance, bounded NIPC framing, or the delegated
/// live-session dispatcher.
#[derive(Debug)]
pub enum AcceptedPeerError {
    /// No production peer/session/consent/registration supplier is connected.
    Unavailable,
    /// Accepting a Unix stream from the listener failed.
    Listener(io::Error),
    /// No connection was accepted before the absolute accept deadline.
    DeadlineExceeded,
    /// The listener repeatedly returned `EINTR` beyond the bounded retry cap.
    InterruptedRetryLimit { attempts: u32, source: io::Error },
    /// Linux credential snapshot or connection-lifetime validation failed.
    Peer(PeerConnectionError),
    /// The stream stopped, timed out, or failed while a frame was being read.
    FrameIo(io::Error),
    /// The NIPC v1 header, bounded lengths, or SDK envelope was malformed.
    InvalidFrame,
    /// The accepted server stream contained a non-request NIPC envelope.
    NotRequest,
    /// The existing live-authority/dispatch owner rejected the request.
    Dispatch(PeerConnectionError),
}

impl fmt::Display for AcceptedPeerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("accepted peer dispatch unavailable"),
            Self::Listener(source) => write!(formatter, "Unix listener accept failed: {source}"),
            Self::DeadlineExceeded => formatter.write_str("Unix listener accept deadline exceeded"),
            Self::InterruptedRetryLimit { attempts, source } => write!(
                formatter,
                "Unix listener accept interrupted {attempts} times: {source}"
            ),
            Self::Peer(source) => write!(formatter, "accepted peer connection failed: {source}"),
            Self::FrameIo(source) => write!(formatter, "NIPC request read failed: {source}"),
            Self::InvalidFrame => formatter.write_str("invalid NIPC v1 request frame"),
            Self::NotRequest => formatter.write_str("accepted NIPC envelope is not a request"),
            Self::Dispatch(source) => write!(formatter, "live peer dispatch rejected: {source}"),
        }
    }
}

impl Error for AcceptedPeerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Listener(source)
            | Self::FrameIo(source)
            | Self::InterruptedRetryLimit { source, .. } => Some(source),
            Self::Peer(source) | Self::Dispatch(source) => Some(source),
            Self::Unavailable | Self::DeadlineExceeded | Self::InvalidFrame | Self::NotRequest => {
                None
            }
        }
    }
}

/// Owns a Unix listener configured for nonblocking accept.
///
/// The caller transfers the listener into this wrapper so the nonblocking
/// mode is established once at the ownership boundary. Do not retain a cloned
/// listener that can race this accept loop.
pub struct AcceptedPeerListener {
    listener: UnixListener,
    current_writer_credentials_enabled: bool,
}

impl AcceptedPeerListener {
    /// Take ownership of a listener, enable and verify `SO_PASSCRED`, then set
    /// it nonblocking before the bounded accept loop. Production spawn wiring
    /// enables `SO_PASSCRED` before `listen`; connections accepted from a
    /// listener configured later are rejected unless the accepted socket also
    /// reports the inherited option.
    pub fn new(listener: UnixListener) -> Result<Self, AcceptedPeerError> {
        #[cfg(target_os = "linux")]
        crate::enable_passcred(listener.as_raw_fd()).map_err(AcceptedPeerError::Listener)?;
        listener
            .set_nonblocking(true)
            .map_err(AcceptedPeerError::Listener)?;
        Ok(Self {
            listener,
            current_writer_credentials_enabled: cfg!(target_os = "linux"),
        })
    }

    /// Whether this listener is configured to request per-write Linux
    /// credentials. This is a transport property only; the spawn owner must
    /// separately establish that the child cannot forge those credentials.
    pub const fn current_writer_credentials_enabled(&self) -> bool {
        self.current_writer_credentials_enabled
    }

    /// Accept one stream before `deadline`, then bind the kernel credential
    /// snapshot to that exact accepted connection.
    pub fn accept_until(
        &mut self,
        deadline: Instant,
    ) -> Result<AcceptedPeerConnection, AcceptedPeerError> {
        #[cfg(target_os = "linux")]
        let build = OwnedPeerConnection::from_connected_with_current_writer_credentials;
        #[cfg(not(target_os = "linux"))]
        let build = OwnedPeerConnection::from_connected;
        accept_until_with(
            deadline,
            || match self.listener.accept() {
                Ok((stream, _peer_address)) => Ok(Some(stream)),
                Err(source) if source.kind() == io::ErrorKind::WouldBlock => Ok(None),
                Err(source) if source.kind() == io::ErrorKind::Interrupted => Err(source),
                Err(source) => Err(source),
            },
            build,
        )
    }
}

fn accept_until_with(
    deadline: Instant,
    try_accept: impl FnMut() -> io::Result<Option<UnixStream>>,
    build: impl FnOnce(UnixStream) -> Result<OwnedPeerConnection, PeerConnectionError>,
) -> Result<AcceptedPeerConnection, AcceptedPeerError> {
    accept_until_with_clock(
        deadline,
        Instant::now,
        std::thread::sleep,
        try_accept,
        build,
    )
}

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(1);
const ACCEPT_MAX_BACKOFF: Duration = Duration::from_micros(800);
const ACCEPT_EINTR_RETRY_LIMIT: u32 = 32;

fn accept_until_with_clock(
    deadline: Instant,
    mut now: impl FnMut() -> Instant,
    mut wait: impl FnMut(Duration),
    mut try_accept: impl FnMut() -> io::Result<Option<UnixStream>>,
    build: impl FnOnce(UnixStream) -> Result<OwnedPeerConnection, PeerConnectionError>,
) -> Result<AcceptedPeerConnection, AcceptedPeerError> {
    let mut interrupted_retries = 0;
    let stream = loop {
        if now() >= deadline {
            return Err(AcceptedPeerError::DeadlineExceeded);
        }
        match try_accept() {
            Ok(Some(stream)) => {
                // The pre-call deadline check does not bound time spent inside
                // an accept attempt or a scheduler pause. Recheck before
                // accepting this late result into the connection owner.
                if now() >= deadline {
                    return Err(AcceptedPeerError::DeadlineExceeded);
                }
                break stream;
            }
            Ok(None) => {
                interrupted_retries = 0;
                if !wait_before_deadline(deadline, &mut now, &mut wait, ACCEPT_POLL_INTERVAL) {
                    return Err(AcceptedPeerError::DeadlineExceeded);
                }
            }
            Err(source) if source.kind() == io::ErrorKind::WouldBlock => {
                interrupted_retries = 0;
                if !wait_before_deadline(deadline, &mut now, &mut wait, ACCEPT_POLL_INTERVAL) {
                    return Err(AcceptedPeerError::DeadlineExceeded);
                }
            }
            Err(source) if source.kind() == io::ErrorKind::Interrupted => {
                interrupted_retries += 1;
                if interrupted_retries >= ACCEPT_EINTR_RETRY_LIMIT {
                    return Err(AcceptedPeerError::InterruptedRetryLimit {
                        attempts: interrupted_retries,
                        source,
                    });
                }
                let multiplier = 1_u64 << interrupted_retries.saturating_sub(1).min(4);
                let backoff = Duration::from_micros(50 * multiplier).min(ACCEPT_MAX_BACKOFF);
                if !wait_before_deadline(deadline, &mut now, &mut wait, backoff) {
                    return Err(AcceptedPeerError::DeadlineExceeded);
                }
            }
            Err(source) => return Err(AcceptedPeerError::Listener(source)),
        }
    };
    let peer = build(stream).map_err(AcceptedPeerError::Peer)?;
    if now() >= deadline {
        return Err(AcceptedPeerError::DeadlineExceeded);
    }
    Ok(AcceptedPeerConnection {
        peer,
        state: AcceptedConnectionState::Ready,
        #[cfg(test)]
        test_connection_check: None,
    })
}

fn wait_before_deadline(
    deadline: Instant,
    now: &mut impl FnMut() -> Instant,
    wait: &mut impl FnMut(Duration),
    preferred_delay: Duration,
) -> bool {
    let before = now();
    if before >= deadline {
        return false;
    }
    let delay = deadline
        .saturating_duration_since(before)
        .min(preferred_delay);
    if delay.is_zero() {
        return false;
    }
    wait(delay);
    now() < deadline
}

/// The existing init-owned authority and single-call dispatcher required by
/// an accepted peer. This trait adds no authority source and has no production
/// implementation in this task overlay.
///
/// Implementations must map the kernel snapshot to a live process-lifetime
/// record and exact Nagi app session, then enforce current capability,
/// consent, service-registration generation, and the existing request ledger
/// at dispatch. PID/UID/GID, socket path/cookie, or caller-controlled NIPC
/// fields cannot establish identity. `dispatch_authorized_request` must use
/// the canonical task-41 admission/commit owner rather than add another
/// ledger or authority check with different semantics.
pub trait LiveSessionServiceDispatcher: Send + Sync {
    /// Recheck that this exact peer still maps to a live Nagi app session.
    fn check_live_session(&self, peer: &KernelPeerCredentials) -> Result<(), PeerConnectionError>;

    /// Let the existing live authority/request owner authorize and dispatch
    /// this one complete NIPC request. The bytes contain request data only;
    /// the peer identity must come from `peer` plus the live init registry.
    fn dispatch_authorized_request(
        &self,
        peer: &KernelPeerCredentials,
        request_frame: &[u8],
    ) -> Result<(), PeerConnectionError>;
}

/// One accepted stream and its non-cloneable Linux kernel credential snapshot.
/// A connection handles at most one request. Its credentials are transport
/// metadata only and do not themselves authorize service dispatch.
pub struct AcceptedPeerConnection {
    peer: OwnedPeerConnection,
    state: AcceptedConnectionState,
    #[cfg(test)]
    test_connection_check: Option<
        Box<dyn FnMut(&KernelPeerCredentials, &UnixStream) -> Result<(), PeerConnectionError>>,
    >,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AcceptedConnectionState {
    Ready,
    Consumed,
}

impl AcceptedPeerConnection {
    /// Consume this accepted connection and transfer its exact socket through
    /// a one-shot stream callback, under the same absolute deadline.
    ///
    /// The transfer carries a private same-socket credential check, not the
    /// PID/UID/GID snapshot itself. This transport API does not prove that the
    /// callback is a registered endpoint owner and does not create an
    /// `EndpointCheckout`, session, capability, consent, or authorization.
    /// The caller must invoke it only from its registered endpoint owner and
    /// perform its independent live Nagi authority checks.
    ///
    /// Taking the transfer consumes the accepted adapter; it cannot also be
    /// replayed through the dispatch candidate:
    ///
    /// ```compile_fail
    /// use linux_peer_credentials_supplier::AcceptedPeerConnection;
    /// use std::time::Instant;
    /// fn transfer_then_reenter(adapter: AcceptedPeerConnection, deadline: Instant) {
    ///     let _transfer = adapter.into_stream_transfer_until(deadline);
    ///     let _reentry = adapter.run_task41_bridge_candidate_until(deadline, None);
    /// }
    /// ```
    pub fn into_stream_transfer_until(
        mut self,
        deadline: Instant,
    ) -> Result<OwnedPeerStreamTransfer, AcceptedPeerError> {
        self.consume_once()?;
        self.peer
            .into_stream_transfer_until(deadline)
            .map_err(AcceptedPeerError::Peer)
    }

    /// Read one bounded NIPC v1 request and hand it to the existing live
    /// session/consent/registration dispatcher.
    ///
    /// With `dispatcher == None`, the accepted connection is invalidated and
    /// `Unavailable` is returned before reading a frame or invoking dispatch.
    /// The delegated dispatch owner still needs its own atomic live-authority
    /// check at the actual service commit point; these transport-boundary
    /// checks cannot cancel work already committed or prevent PID reuse.
    pub fn receive_and_dispatch_until(
        &mut self,
        deadline: Instant,
        dispatcher: Option<&dyn LiveSessionServiceDispatcher>,
    ) -> Result<(), AcceptedPeerError> {
        self.consume_once()?;
        let Some(dispatcher) = dispatcher else {
            self.peer.invalidated = true;
            return Err(AcceptedPeerError::Unavailable);
        };

        #[cfg(test)]
        let result = if self.test_connection_check.is_some() {
            self.run_with_test_connection_check(deadline, dispatcher)
        } else {
            self.peer.with_nonblocking_io_until_peer_check(
                deadline,
                |credentials| dispatcher.check_live_session(credentials),
                |io, credentials| dispatch_one_request(io, credentials, deadline, dispatcher),
            )
        };
        #[cfg(not(test))]
        let result = self.peer.with_nonblocking_io_until_peer_check(
            deadline,
            |credentials| dispatcher.check_live_session(credentials),
            |io, credentials| dispatch_one_request(io, credentials, deadline, dispatcher),
        );
        // A single-call stream is never reused, including on malformed input,
        // authority rejection, or timeout. Recovery requires a new accept and
        // a new kernel snapshot.
        self.peer.invalidated = true;
        match result {
            Err(error) => Err(AcceptedPeerError::Peer(error)),
            Ok(Err(error)) => Err(error),
            Ok(Ok(())) => Ok(()),
        }
    }

    /// Candidate dispatch entry toward a task-41 live authority bridge.
    ///
    /// This keeps the accepted connection, connection-time credential
    /// snapshot, one-shot state, and absolute deadline together while using
    /// the current dispatcher seam. It is not a conversion to task-41's
    /// `EndpointCheckout` or `AuthorizedPeer`: their construction remains
    /// inside task-41's endpoint owner. Until that owner supplies a bridge
    /// that consumes this exact connection, `authority == None` remains
    /// `Unavailable`; the credentials are never promoted to authorization.
    pub fn run_task41_bridge_candidate_until(
        &mut self,
        deadline: Instant,
        authority: Option<&dyn LiveSessionServiceDispatcher>,
    ) -> Result<(), AcceptedPeerError> {
        self.receive_and_dispatch_until(deadline, authority)
    }

    fn consume_once(&mut self) -> Result<(), AcceptedPeerError> {
        match self.state {
            AcceptedConnectionState::Ready => {
                // Latch before any session callback, frame read, or dispatch
                // callback can panic or unwind.
                self.state = AcceptedConnectionState::Consumed;
                Ok(())
            }
            AcceptedConnectionState::Consumed => {
                self.peer.invalidated = true;
                Err(AcceptedPeerError::Peer(PeerConnectionError::Stale))
            }
        }
    }

    #[cfg(test)]
    fn run_with_test_connection_check(
        &mut self,
        deadline: Instant,
        dispatcher: &dyn LiveSessionServiceDispatcher,
    ) -> Result<Result<(), AcceptedPeerError>, PeerConnectionError> {
        let checker = self
            .test_connection_check
            .as_mut()
            .expect("test checker was selected");
        let peer = &mut self.peer;
        crate::owned_peer_connection::with_nonblocking_io_until_using(
            &mut peer.invalidated,
            &peer.credentials,
            &mut peer.stream,
            deadline,
            |credentials, stream| {
                checker(credentials, stream)
                    .and_then(|()| dispatcher.check_live_session(credentials))
            },
            |io, credentials| dispatch_one_request(io, credentials, deadline, dispatcher),
        )
    }
}

fn dispatch_one_request(
    io: &mut PeerConnectionIo<'_>,
    peer: &KernelPeerCredentials,
    deadline: Instant,
    dispatcher: &dyn LiveSessionServiceDispatcher,
) -> Result<(), AcceptedPeerError> {
    let frame = read_nipc_request_frame_until(io, deadline)?;
    // Recheck at the logical boundary after parsing potentially buffered data.
    // The dispatcher must repeat/serialize authorization at its own commit.
    io.recheck_before_side_effect()
        .map_err(AcceptedPeerError::FrameIo)?;
    dispatcher
        .dispatch_authorized_request(peer, &frame)
        .map_err(AcceptedPeerError::Dispatch)
}

fn read_nipc_request_frame_until(
    reader: &mut impl Read,
    deadline: Instant,
) -> Result<Vec<u8>, AcceptedPeerError> {
    let mut header = [0_u8; FIXED_HEADER_BYTES];
    read_exact_until(reader, &mut header, deadline).map_err(AcceptedPeerError::FrameIo)?;
    if &header[..4] != MAGIC || u16::from_le_bytes([header[4], header[5]]) != IPC_PROTOCOL_VERSION {
        return Err(AcceptedPeerError::InvalidFrame);
    }

    let payload_type_len = usize::from(u16::from_le_bytes([
        header[PAYLOAD_TYPE_LEN_OFFSET],
        header[PAYLOAD_TYPE_LEN_OFFSET + 1],
    ]));
    let payload_len = u32::from_le_bytes([
        header[PAYLOAD_LEN_OFFSET],
        header[PAYLOAD_LEN_OFFSET + 1],
        header[PAYLOAD_LEN_OFFSET + 2],
        header[PAYLOAD_LEN_OFFSET + 3],
    ]) as usize;
    if payload_type_len == 0
        || payload_type_len > MAX_PAYLOAD_TYPE_BYTES
        || payload_len > MAX_IPC_PAYLOAD_BYTES
    {
        return Err(AcceptedPeerError::InvalidFrame);
    }
    let frame_len = FIXED_HEADER_BYTES
        .checked_add(payload_type_len)
        .and_then(|length| length.checked_add(payload_len))
        .ok_or(AcceptedPeerError::InvalidFrame)?;
    let mut frame = vec![0_u8; frame_len];
    frame[..FIXED_HEADER_BYTES].copy_from_slice(&header);
    read_exact_until(reader, &mut frame[FIXED_HEADER_BYTES..], deadline)
        .map_err(AcceptedPeerError::FrameIo)?;

    let envelope = IpcEnvelope::decode(&frame).map_err(|_| AcceptedPeerError::InvalidFrame)?;
    if envelope.kind != MessageKind::Request || envelope.request_id.is_none() {
        return Err(AcceptedPeerError::NotRequest);
    }
    Ok(frame)
}

fn read_exact_until(
    reader: &mut impl Read,
    output: &mut [u8],
    deadline: Instant,
) -> io::Result<()> {
    let mut offset = 0;
    while offset < output.len() {
        let now = Instant::now();
        if now >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "NIPC frame deadline",
            ));
        }
        match reader.read(&mut output[offset..]) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Ok(read) => offset += read,
            Err(source) if source.kind() == io::ErrorKind::Interrupted => continue,
            Err(source) if source.kind() == io::ErrorKind::WouldBlock => {
                std::thread::park_timeout(
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(1)),
                );
            }
            Err(source) => return Err(source),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PeerCredentialError, SocketOption};
    use nagi_sdk::app_contract::{CorrelationId, OrderingMetadata, RequestId};
    use nagi_sdk::AppId;
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

    fn valid_request_frame() -> Vec<u8> {
        let envelope = IpcEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            source: AppId::from_identifier(b"com.nagi.client"),
            destination: Some(AppId::from_identifier(b"com.nagi.service")),
            kind: MessageKind::Request,
            correlation_id: CorrelationId([1; 16]),
            request_id: Some(RequestId([2; 16])),
            ordering: OrderingMetadata {
                timestamp_micros: None,
                sequence: None,
            },
            payload_type: "nagi.test@1",
            payload: b"test request",
        };
        let mut frame =
            vec![0; FIXED_HEADER_BYTES + envelope.payload_type.len() + envelope.payload.len()];
        let length = envelope.encode(&mut frame).unwrap();
        frame.truncate(length);
        frame
    }

    fn owned_for_test(stream: UnixStream, cookie: u64) -> OwnedPeerConnection {
        OwnedPeerConnection {
            stream,
            credentials: KernelPeerCredentials {
                pid: 42,
                uid: 1000,
                gid: 1000,
                local_socket_cookie: cookie,
            },
            require_current_writer_credentials: false,
            invalidated: false,
        }
    }

    fn fake_accept_with_stream(stream: UnixStream, cookie: u64) -> AcceptedPeerConnection {
        let mut stream = Some(stream);
        let mut adapter = accept_until_with(
            Instant::now() + Duration::from_secs(1),
            || Ok(stream.take()),
            |stream| Ok(owned_for_test(stream, cookie)),
        )
        .unwrap();
        let fd = adapter.peer.stream.as_raw_fd();
        let mut cookies = HashMap::new();
        cookies.insert(fd, cookie);
        adapter.test_connection_check = Some(Box::new(move |credentials, stream| {
            check_cookie(&cookies, credentials, stream)
        }));
        adapter
    }

    fn check_cookie(
        cookies: &HashMap<i32, u64>,
        credentials: &KernelPeerCredentials,
        stream: &UnixStream,
    ) -> Result<(), PeerConnectionError> {
        match cookies.get(&stream.as_raw_fd()) {
            Some(cookie) if *cookie == credentials.local_socket_cookie => injected_probe(stream),
            Some(_) => Err(PeerConnectionError::CredentialRead(
                PeerCredentialError::ConnectionMismatch,
            )),
            None => Err(PeerConnectionError::CredentialRead(
                PeerCredentialError::KernelRead {
                    option: SocketOption::SocketCookie,
                    source: io::Error::from_raw_os_error(libc::EBADF),
                },
            )),
        }
    }

    fn injected_probe(stream: &UnixStream) -> Result<(), PeerConnectionError> {
        let fd = stream.as_raw_fd();
        let mut byte = 0_u8;
        // SAFETY: test-only non-consuming peek from a live Unix stream.
        let result = unsafe {
            libc::recv(
                fd,
                (&mut byte as *mut u8).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        if result == 0 {
            Err(PeerConnectionError::PeerDisconnected)
        } else if result > 0 {
            Ok(())
        } else {
            let source = io::Error::last_os_error();
            if source.raw_os_error() == Some(libc::EAGAIN)
                || source.raw_os_error() == Some(libc::EWOULDBLOCK)
            {
                Ok(())
            } else {
                Err(PeerConnectionError::ProbeRead { source })
            }
        }
    }

    struct TestDispatcher {
        checks: AtomicUsize,
        dispatches: AtomicUsize,
        expected_frame: Vec<u8>,
        reject_session: bool,
    }

    impl LiveSessionServiceDispatcher for TestDispatcher {
        fn check_live_session(
            &self,
            _peer: &KernelPeerCredentials,
        ) -> Result<(), PeerConnectionError> {
            self.checks.fetch_add(1, AtomicOrdering::SeqCst);
            if self.reject_session {
                Err(PeerConnectionError::AuthorityRejected)
            } else {
                Ok(())
            }
        }

        fn dispatch_authorized_request(
            &self,
            _peer: &KernelPeerCredentials,
            request_frame: &[u8],
        ) -> Result<(), PeerConnectionError> {
            self.dispatches.fetch_add(1, AtomicOrdering::SeqCst);
            assert_eq!(request_frame, self.expected_frame);
            Ok(())
        }
    }

    #[test]
    fn socketpair_reads_one_bounded_frame_and_dispatches_through_test_owner() {
        let (stream, mut client) = UnixStream::pair().unwrap();
        let frame = valid_request_frame();
        client.write_all(&frame).unwrap();
        let cookie = 0x1234;
        let mut adapter = fake_accept_with_stream(stream, cookie);
        let dispatcher = TestDispatcher {
            checks: AtomicUsize::new(0),
            dispatches: AtomicUsize::new(0),
            expected_frame: frame,
            reject_session: false,
        };

        adapter
            .receive_and_dispatch_until(Instant::now() + Duration::from_secs(1), Some(&dispatcher))
            .unwrap();
        assert!(dispatcher.checks.load(AtomicOrdering::SeqCst) >= 2);
        assert_eq!(dispatcher.dispatches.load(AtomicOrdering::SeqCst), 1);
        assert!(adapter.peer.invalidated);
    }

    #[test]
    fn fake_accept_rejects_missing_authority_before_read_or_dispatch() {
        let (stream, mut client) = UnixStream::pair().unwrap();
        let frame = valid_request_frame();
        client.write_all(&frame).unwrap();
        let cookie = 0x5678;
        let mut adapter = fake_accept_with_stream(stream, cookie);
        let dispatcher = TestDispatcher {
            checks: AtomicUsize::new(0),
            dispatches: AtomicUsize::new(0),
            expected_frame: frame,
            reject_session: false,
        };

        assert!(matches!(
            adapter
                .run_task41_bridge_candidate_until(Instant::now() + Duration::from_secs(1), None,),
            Err(AcceptedPeerError::Unavailable)
        ));
        assert_eq!(dispatcher.dispatches.load(AtomicOrdering::SeqCst), 0);
        let mut first = [0_u8; 1];
        let peek = unsafe {
            libc::recv(
                adapter.peer.stream.as_raw_fd(),
                first.as_mut_ptr().cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        assert_eq!(peek, 1);
        assert_eq!(&first, b"N");
        assert!(matches!(
            adapter.run_task41_bridge_candidate_until(
                Instant::now() + Duration::from_secs(1),
                Some(&dispatcher),
            ),
            Err(AcceptedPeerError::Peer(PeerConnectionError::Stale))
        ));
        assert_eq!(dispatcher.dispatches.load(AtomicOrdering::SeqCst), 0);
        drop(adapter);
        drop(client);
    }

    struct DenyOnlyCounterDispatcher {
        checks: AtomicUsize,
        dispatches: AtomicUsize,
        effects: AtomicUsize,
    }

    impl LiveSessionServiceDispatcher for DenyOnlyCounterDispatcher {
        fn check_live_session(
            &self,
            _peer: &KernelPeerCredentials,
        ) -> Result<(), PeerConnectionError> {
            self.checks.fetch_add(1, AtomicOrdering::SeqCst);
            Err(PeerConnectionError::AuthorityRejected)
        }

        fn dispatch_authorized_request(
            &self,
            _peer: &KernelPeerCredentials,
            _request_frame: &[u8],
        ) -> Result<(), PeerConnectionError> {
            self.dispatches.fetch_add(1, AtomicOrdering::SeqCst);
            self.effects.fetch_add(1, AtomicOrdering::SeqCst);
            Ok(())
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn real_listener_without_live_authority_fails_closed_before_frame_read() {
        static NEXT_SOCKET: AtomicUsize = AtomicUsize::new(0);

        let valid = valid_request_frame();
        let partial = valid[..20].to_vec();
        let mut malformed = vec![0; FIXED_HEADER_BYTES];
        malformed[..4].copy_from_slice(b"BAD!");
        let mut oversized = vec![0; FIXED_HEADER_BYTES];
        oversized[..4].copy_from_slice(MAGIC);
        oversized[4..6].copy_from_slice(&IPC_PROTOCOL_VERSION.to_le_bytes());
        oversized[PAYLOAD_TYPE_LEN_OFFSET..PAYLOAD_TYPE_LEN_OFFSET + 2]
            .copy_from_slice(&1_u16.to_le_bytes());
        let oversized_payload = u32::try_from(MAX_IPC_PAYLOAD_BYTES).unwrap() + 1;
        oversized[PAYLOAD_LEN_OFFSET..PAYLOAD_LEN_OFFSET + 4]
            .copy_from_slice(&oversized_payload.to_le_bytes());

        let cases = [
            ("partial header", partial.clone(), false),
            ("malformed header", malformed, true),
            ("oversized payload header", oversized, true),
            ("partial header followed by EOF", partial, true),
        ];

        for (name, request_bytes, close_write) in cases {
            let socket_path = std::env::temp_dir().join(format!(
                "nagi-c2-peer-{}-{}.sock",
                std::process::id(),
                NEXT_SOCKET.fetch_add(1, AtomicOrdering::Relaxed)
            ));
            let listener = UnixListener::bind(&socket_path).unwrap();
            let mut listener = AcceptedPeerListener::new(listener).unwrap();
            let mut client = UnixStream::connect(&socket_path).unwrap();
            client.write_all(&request_bytes).unwrap();
            if close_write {
                client.shutdown(std::net::Shutdown::Write).unwrap();
            }

            let mut accepted = listener
                .accept_until(Instant::now() + Duration::from_secs(1))
                .unwrap_or_else(|error| panic!("{name}: accept failed: {error}"));
            assert_eq!(
                accepted.peer.credentials.pid,
                std::process::id() as i32,
                "{name}: accepted peer must carry the kernel's actual process credential"
            );

            let counters = DenyOnlyCounterDispatcher {
                checks: AtomicUsize::new(0),
                dispatches: AtomicUsize::new(0),
                effects: AtomicUsize::new(0),
            };
            assert!(
                matches!(
                    accepted
                        .receive_and_dispatch_until(Instant::now() + Duration::from_secs(1), None,),
                    Err(AcceptedPeerError::Unavailable)
                ),
                "{name}: missing C1 authority must default-deny"
            );
            assert!(
                accepted.peer.invalidated,
                "{name}: denied stream must be invalidated"
            );
            assert_eq!(counters.checks.load(AtomicOrdering::SeqCst), 0, "{name}");
            assert_eq!(
                counters.dispatches.load(AtomicOrdering::SeqCst),
                0,
                "{name}"
            );
            assert_eq!(counters.effects.load(AtomicOrdering::SeqCst), 0, "{name}");

            let mut observed = vec![0_u8; request_bytes.len()];
            // SAFETY: MSG_PEEK observes the accepted live stream without consuming
            // any bytes; the output buffer is valid for its full length.
            let peek = unsafe {
                libc::recv(
                    accepted.peer.stream.as_raw_fd(),
                    observed.as_mut_ptr().cast(),
                    observed.len(),
                    libc::MSG_PEEK | libc::MSG_DONTWAIT,
                )
            };
            assert_eq!(
                peek,
                request_bytes.len() as isize,
                "{name}: request was consumed"
            );
            assert_eq!(observed, request_bytes, "{name}: request bytes changed");

            assert!(
                matches!(
                    accepted.receive_and_dispatch_until(
                        Instant::now() + Duration::from_secs(1),
                        Some(&counters),
                    ),
                    Err(AcceptedPeerError::Peer(PeerConnectionError::Stale))
                ),
                "{name}: a consumed accepted connection must reject reuse"
            );
            assert_eq!(counters.checks.load(AtomicOrdering::SeqCst), 0, "{name}");
            assert_eq!(
                counters.dispatches.load(AtomicOrdering::SeqCst),
                0,
                "{name}"
            );
            assert_eq!(counters.effects.load(AtomicOrdering::SeqCst), 0, "{name}");

            drop(accepted);
            drop(listener);
            drop(client);
            std::fs::remove_file(socket_path).unwrap();
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn stream_transfer_fails_closed_without_linux_credentials() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let accepted = fake_accept_with_stream(stream, 0x5679);

        assert!(matches!(
            accepted.into_stream_transfer_until(Instant::now() + Duration::from_secs(1)),
            Err(AcceptedPeerError::Peer(PeerConnectionError::Unavailable))
        ));
        let mut byte = [0; 1];
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn expired_bridge_deadline_rejects_before_session_check_or_dispatch() {
        let (stream, mut client) = UnixStream::pair().unwrap();
        let frame = valid_request_frame();
        client.write_all(&frame).unwrap();
        let mut adapter = fake_accept_with_stream(stream, 0xabc1);
        let dispatcher = TestDispatcher {
            checks: AtomicUsize::new(0),
            dispatches: AtomicUsize::new(0),
            expected_frame: frame,
            reject_session: false,
        };

        assert!(matches!(
            adapter.run_task41_bridge_candidate_until(
                Instant::now() - Duration::from_millis(1),
                Some(&dispatcher),
            ),
            Err(AcceptedPeerError::Peer(
                PeerConnectionError::DeadlineExceeded
            ))
        ));
        assert_eq!(dispatcher.checks.load(AtomicOrdering::SeqCst), 0);
        assert_eq!(dispatcher.dispatches.load(AtomicOrdering::SeqCst), 0);
        assert!(adapter.peer.invalidated);
        let mut first = [0_u8; 1];
        // SAFETY: test-only non-consuming peek verifies that an expired
        // handoff did not consume bytes before returning fail closed.
        let peek = unsafe {
            libc::recv(
                adapter.peer.stream.as_raw_fd(),
                first.as_mut_ptr().cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        assert_eq!(peek, 1);
        assert_eq!(&first, b"N");
    }

    #[test]
    fn disconnected_connection_snapshot_is_stale_and_cannot_reach_dispatch() {
        let (stream, client) = UnixStream::pair().unwrap();
        let mut adapter = fake_accept_with_stream(stream, 0xabc2);
        let dispatcher = TestDispatcher {
            checks: AtomicUsize::new(0),
            dispatches: AtomicUsize::new(0),
            expected_frame: Vec::new(),
            reject_session: false,
        };
        drop(client);

        assert!(matches!(
            adapter.run_task41_bridge_candidate_until(
                Instant::now() + Duration::from_secs(1),
                Some(&dispatcher),
            ),
            Err(AcceptedPeerError::Peer(
                PeerConnectionError::PeerDisconnected
            ))
        ));
        assert_eq!(dispatcher.dispatches.load(AtomicOrdering::SeqCst), 0);
        assert!(adapter.peer.invalidated);
        assert!(matches!(
            adapter.receive_and_dispatch_until(
                Instant::now() + Duration::from_secs(1),
                Some(&dispatcher),
            ),
            Err(AcceptedPeerError::Peer(PeerConnectionError::Stale))
        ));
        assert_eq!(dispatcher.dispatches.load(AtomicOrdering::SeqCst), 0);
    }

    struct ProcessExitAtCommitDispatcher {
        checks: AtomicUsize,
        commit_checks: AtomicUsize,
        commits: AtomicUsize,
        live_process: std::sync::atomic::AtomicBool,
    }

    impl LiveSessionServiceDispatcher for ProcessExitAtCommitDispatcher {
        fn check_live_session(
            &self,
            _peer: &KernelPeerCredentials,
        ) -> Result<(), PeerConnectionError> {
            self.checks.fetch_add(1, AtomicOrdering::SeqCst);
            if self.live_process.load(AtomicOrdering::SeqCst) {
                Ok(())
            } else {
                Err(PeerConnectionError::AuthorityRejected)
            }
        }

        fn dispatch_authorized_request(
            &self,
            _peer: &KernelPeerCredentials,
            _request_frame: &[u8],
        ) -> Result<(), PeerConnectionError> {
            // Model process exit after the transport's last live check but
            // immediately before task-41's service-owner commit boundary.
            self.live_process.store(false, AtomicOrdering::SeqCst);
            self.commit_checks.fetch_add(1, AtomicOrdering::SeqCst);
            if self.live_process.load(AtomicOrdering::SeqCst) {
                self.commits.fetch_add(1, AtomicOrdering::SeqCst);
                Ok(())
            } else {
                Err(PeerConnectionError::AuthorityRejected)
            }
        }
    }

    #[test]
    fn process_exit_observed_at_final_commit_boundary_prevents_effect() {
        let (stream, mut client) = UnixStream::pair().unwrap();
        client.write_all(&valid_request_frame()).unwrap();
        let mut adapter = fake_accept_with_stream(stream, 0xabc3);
        let authority = ProcessExitAtCommitDispatcher {
            checks: AtomicUsize::new(0),
            commit_checks: AtomicUsize::new(0),
            commits: AtomicUsize::new(0),
            live_process: std::sync::atomic::AtomicBool::new(true),
        };

        assert!(matches!(
            adapter.run_task41_bridge_candidate_until(
                Instant::now() + Duration::from_secs(1),
                Some(&authority),
            ),
            Err(AcceptedPeerError::Dispatch(
                PeerConnectionError::AuthorityRejected
            )) | Err(AcceptedPeerError::Peer(
                PeerConnectionError::AuthorityRejected
            ))
        ));
        assert!(authority.checks.load(AtomicOrdering::SeqCst) >= 2);
        assert_eq!(authority.commit_checks.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(authority.commits.load(AtomicOrdering::SeqCst), 0);
        assert!(!authority.live_process.load(AtomicOrdering::SeqCst));
        assert!(adapter.peer.invalidated);
        assert!(matches!(
            adapter.receive_and_dispatch_until(
                Instant::now() + Duration::from_secs(1),
                Some(&authority),
            ),
            Err(AcceptedPeerError::Peer(PeerConnectionError::Stale))
        ));
        assert_eq!(authority.commit_checks.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(authority.commits.load(AtomicOrdering::SeqCst), 0);
    }

    #[test]
    fn fake_accept_dispatches_one_valid_request_through_test_owner() {
        let (stream, mut client) = UnixStream::pair().unwrap();
        let frame = valid_request_frame();
        client.write_all(&frame).unwrap();
        let cookie = 0x789a;
        let mut adapter = fake_accept_with_stream(stream, cookie);
        let dispatcher = TestDispatcher {
            checks: AtomicUsize::new(0),
            dispatches: AtomicUsize::new(0),
            expected_frame: frame,
            reject_session: false,
        };

        adapter
            .receive_and_dispatch_until(Instant::now() + Duration::from_secs(1), Some(&dispatcher))
            .unwrap();
        assert!(dispatcher.checks.load(AtomicOrdering::SeqCst) >= 2);
        assert_eq!(dispatcher.dispatches.load(AtomicOrdering::SeqCst), 1);
        assert!(adapter.peer.invalidated);
    }

    #[test]
    fn fake_listener_would_block_until_accept_deadline_then_stops() {
        let clock = Cell::new(Instant::now());
        let deadline = clock.get() + Duration::from_millis(10);
        let attempts = AtomicUsize::new(0);
        let result = accept_until_with_clock(
            deadline,
            || clock.get(),
            |delay| clock.set(clock.get() + delay),
            || {
                attempts.fetch_add(1, AtomicOrdering::SeqCst);
                Err(io::Error::from(io::ErrorKind::WouldBlock))
            },
            |_stream| panic!("deadline must stop before peer construction"),
        );
        assert!(matches!(result, Err(AcceptedPeerError::DeadlineExceeded)));
        assert!(attempts.load(AtomicOrdering::SeqCst) > 0);
        assert_eq!(clock.get(), deadline);
    }

    #[test]
    fn accept_success_returned_after_deadline_is_rejected() {
        let (stream, mut client) = UnixStream::pair().unwrap();
        let mut accepted_stream = Some(stream);
        let clock = Cell::new(Instant::now());
        let deadline = clock.get() + Duration::from_millis(5);
        let result = accept_until_with_clock(
            deadline,
            || clock.get(),
            |_| {},
            || {
                clock.set(deadline + Duration::from_millis(1));
                Ok(accepted_stream.take())
            },
            |_stream| panic!("late accepted stream must not reach peer construction"),
        );
        assert!(matches!(result, Err(AcceptedPeerError::DeadlineExceeded)));
        assert!(accepted_stream.is_none());
        let mut byte = [0_u8; 1];
        assert_eq!(client.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn deadline_crossing_during_snapshot_construction_drops_accepted_stream() {
        let (stream, mut client) = UnixStream::pair().unwrap();
        let mut accepted_stream = Some(stream);
        let clock = Cell::new(Instant::now());
        let deadline = clock.get() + Duration::from_millis(5);
        let builds = AtomicUsize::new(0);
        let result = accept_until_with_clock(
            deadline,
            || clock.get(),
            |_| {},
            || Ok(accepted_stream.take()),
            |stream| {
                builds.fetch_add(1, AtomicOrdering::SeqCst);
                clock.set(deadline + Duration::from_millis(1));
                Ok(owned_for_test(stream, 0x2468))
            },
        );
        assert!(matches!(result, Err(AcceptedPeerError::DeadlineExceeded)));
        assert_eq!(builds.load(AtomicOrdering::SeqCst), 1);
        assert!(accepted_stream.is_none());
        let mut byte = [0_u8; 1];
        assert_eq!(client.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn interrupted_accept_backs_off_and_recovers_before_deadline() {
        let (stream, mut client) = UnixStream::pair().unwrap();
        let mut accepted_stream = Some(stream);
        let clock = Cell::new(Instant::now());
        let start = clock.get();
        let deadline = start + Duration::from_secs(1);
        let attempts = Cell::new(0_u32);
        let waits = Cell::new(0_u32);
        let waited = Cell::new(Duration::ZERO);
        let adapter = accept_until_with_clock(
            deadline,
            || clock.get(),
            |delay| {
                waits.set(waits.get() + 1);
                waited.set(waited.get() + delay);
                clock.set(clock.get() + delay);
            },
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() <= 2 {
                    Err(io::Error::from(io::ErrorKind::Interrupted))
                } else {
                    Ok(accepted_stream.take())
                }
            },
            |stream| Ok(owned_for_test(stream, 0x3579)),
        )
        .unwrap();

        assert_eq!(attempts.get(), 3);
        assert_eq!(waits.get(), 2);
        assert_eq!(waited.get(), Duration::from_micros(150));
        assert!(clock.get() < deadline);
        drop(adapter);
        let mut byte = [0_u8; 1];
        assert_eq!(client.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn persistent_interrupted_accept_stops_at_deadline_without_extending_it() {
        let clock = Cell::new(Instant::now());
        let deadline = clock.get() + Duration::from_micros(350);
        let attempts = Cell::new(0_u32);
        let waited = Cell::new(Duration::ZERO);
        let result = accept_until_with_clock(
            deadline,
            || clock.get(),
            |delay| {
                waited.set(waited.get() + delay);
                clock.set(clock.get() + delay);
            },
            || {
                attempts.set(attempts.get() + 1);
                Err(io::Error::from(io::ErrorKind::Interrupted))
            },
            |_stream| panic!("interrupted accept must not build a peer"),
        );
        assert!(matches!(result, Err(AcceptedPeerError::DeadlineExceeded)));
        assert_eq!(clock.get(), deadline);
        assert_eq!(waited.get(), Duration::from_micros(350));
        assert!(attempts.get() > 1);
        assert!(attempts.get() < ACCEPT_EINTR_RETRY_LIMIT);
    }

    #[test]
    fn frozen_clock_continuous_interrupted_accept_stops_at_retry_cap() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(30);
        let attempts = Cell::new(0_u32);
        let waited = Cell::new(Duration::ZERO);
        let result = accept_until_with_clock(
            deadline,
            || now,
            |delay| waited.set(waited.get() + delay),
            || {
                attempts.set(attempts.get() + 1);
                Err(io::Error::from_raw_os_error(libc::EINTR))
            },
            |_stream| panic!("interrupted accept must not build a peer"),
        );
        assert!(matches!(
            result,
            Err(AcceptedPeerError::InterruptedRetryLimit { attempts: cap, .. })
                if cap == ACCEPT_EINTR_RETRY_LIMIT
        ));
        assert_eq!(attempts.get(), ACCEPT_EINTR_RETRY_LIMIT);
        assert!(waited.get() <= ACCEPT_MAX_BACKOFF * ACCEPT_EINTR_RETRY_LIMIT);
    }

    struct PanickingDispatcher {
        dispatches: AtomicUsize,
    }

    impl LiveSessionServiceDispatcher for PanickingDispatcher {
        fn check_live_session(
            &self,
            _peer: &KernelPeerCredentials,
        ) -> Result<(), PeerConnectionError> {
            Ok(())
        }

        fn dispatch_authorized_request(
            &self,
            _peer: &KernelPeerCredentials,
            _request_frame: &[u8],
        ) -> Result<(), PeerConnectionError> {
            self.dispatches.fetch_add(1, AtomicOrdering::SeqCst);
            panic!("injected dispatcher panic")
        }
    }

    #[test]
    fn panic_then_reentry_through_bridge_candidate_cannot_reuse_connection() {
        let (stream, mut client) = UnixStream::pair().unwrap();
        client.write_all(&valid_request_frame()).unwrap();
        let cookie = 0x1357;
        let mut adapter = fake_accept_with_stream(stream, cookie);
        let dispatcher = PanickingDispatcher {
            dispatches: AtomicUsize::new(0),
        };

        let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = adapter.receive_and_dispatch_until(
                Instant::now() + Duration::from_secs(1),
                Some(&dispatcher),
            );
        }));
        assert!(first.is_err());
        assert_eq!(dispatcher.dispatches.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(adapter.state, AcceptedConnectionState::Consumed);

        assert!(matches!(
            // Re-enter through the other public dispatch route after unwinding.
            adapter.run_task41_bridge_candidate_until(
                Instant::now() + Duration::from_secs(1),
                Some(&dispatcher),
            ),
            Err(AcceptedPeerError::Peer(PeerConnectionError::Stale))
        ));
        assert_eq!(dispatcher.dispatches.load(AtomicOrdering::SeqCst), 1);
    }

    #[test]
    fn fake_accept_session_mismatch_never_reaches_dispatch() {
        let (stream, mut client) = UnixStream::pair().unwrap();
        client.write_all(&valid_request_frame()).unwrap();
        let cookie = 0x9abc;
        let mut adapter = fake_accept_with_stream(stream, cookie);
        let dispatcher = TestDispatcher {
            checks: AtomicUsize::new(0),
            dispatches: AtomicUsize::new(0),
            expected_frame: Vec::new(),
            reject_session: true,
        };

        assert!(matches!(
            adapter.receive_and_dispatch_until(
                Instant::now() + Duration::from_secs(1),
                Some(&dispatcher),
            ),
            Err(AcceptedPeerError::Peer(
                PeerConnectionError::AuthorityRejected
            ))
        ));
        assert_eq!(dispatcher.dispatches.load(AtomicOrdering::SeqCst), 0);
        let mut first = [0_u8; 1];
        let peek = unsafe {
            libc::recv(
                adapter.peer.stream.as_raw_fd(),
                first.as_mut_ptr().cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        assert_eq!(peek, 1);
        assert_eq!(&first, b"N");
        assert!(adapter.peer.invalidated);
    }
}
