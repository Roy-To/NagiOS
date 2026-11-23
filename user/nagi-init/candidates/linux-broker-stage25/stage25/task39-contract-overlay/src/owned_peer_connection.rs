//! Own one accepted Unix stream together with its connection-time kernel
//! credential snapshot. This transport-lifetime helper does not establish a
//! Nagi identity, session, capability, consent, or registration.

#[cfg(target_os = "linux")]
use crate::read_kernel_peer_credentials;
use crate::{KernelPeerCredentials, PeerCredentialError};
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Instant;

#[cfg(target_os = "linux")]
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, RawFd};

/// Errors while keeping a kernel credential snapshot tied to one owned stream.
#[derive(Debug)]
pub enum PeerConnectionError {
    /// This target does not provide the Linux peer-credential contract.
    Unavailable,
    /// The task-39 kernel credential reader rejected the stream or its cookie.
    CredentialRead(PeerCredentialError),
    /// A non-consuming probe observed EOF on the peer connection.
    PeerDisconnected,
    /// The non-consuming connection-state probe failed.
    ProbeRead { source: io::Error },
    /// A caller-supplied live authority hook rejected the operation.
    ///
    /// This is only a transport for an independent authority decision. The
    /// adapter does not implement or infer that decision.
    AuthorityRejected,
    /// A nonblocking socket mode could not be configured.
    NonblockingSetup { source: io::Error },
    /// The accepted socket does not have the Linux per-message credential
    /// receiver enabled, or a read did not carry one valid writer identity.
    CurrentWriterCredentialsUnavailable,
    /// Reading the accepted socket's `SO_PASSCRED` setting failed.
    CurrentWriterCredentialsOptionRead { source: io::Error },
    /// The caller's absolute operation deadline elapsed at a guard boundary.
    DeadlineExceeded,
    /// A read/write/flush failed with an error other than a retryable
    /// `Interrupted` or `WouldBlock`.
    IoFailure {
        kind: io::ErrorKind,
        raw_os_error: Option<i32>,
        message: String,
    },
    /// This connection was already observed to be closed, rejected, or failed.
    Stale,
}

impl fmt::Display for PeerConnectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("owned Linux peer connection unavailable"),
            Self::CredentialRead(source) => {
                write!(formatter, "peer credential check failed: {source}")
            }
            Self::PeerDisconnected => formatter.write_str("Unix peer disconnected"),
            Self::ProbeRead { source } => {
                write!(formatter, "Unix peer state probe failed: {source}")
            }
            Self::AuthorityRejected => {
                formatter.write_str("independent peer authority check rejected the operation")
            }
            Self::NonblockingSetup { source } => {
                write!(formatter, "could not set Unix stream nonblocking: {source}")
            }
            Self::CurrentWriterCredentialsUnavailable => {
                formatter.write_str("Unix stream current-writer credentials unavailable")
            }
            Self::CurrentWriterCredentialsOptionRead { source } => {
                write!(formatter, "could not read SO_PASSCRED: {source}")
            }
            Self::DeadlineExceeded => formatter.write_str("Unix peer operation deadline exceeded"),
            Self::IoFailure {
                kind,
                raw_os_error,
                message,
            } => write!(
                formatter,
                "Unix stream I/O failed ({kind:?}, errno={raw_os_error:?}): {message}"
            ),
            Self::Stale => formatter.write_str("Unix peer connection snapshot is stale"),
        }
    }
}

impl Error for PeerConnectionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CredentialRead(source) => Some(source),
            Self::ProbeRead { source } => Some(source),
            Self::NonblockingSetup { source } => Some(source),
            Self::CurrentWriterCredentialsOptionRead { source } => Some(source),
            _ => None,
        }
    }
}

/// A temporary, guarded read/write view of the owned stream.
///
/// The adapter checks connection identity, peer state, and the mandatory
/// caller-supplied authority hook before and after every `read`, `write`, and
/// `flush` primitive. It also checks before and after the whole callback. A
/// failure is latched even if callback code ignores the returned I/O error.
/// A blocking syscall itself cannot be interrupted by these boundary checks,
/// and bytes already consumed or written cannot be rolled back. The wrapper
/// exposes no raw FD, `UnixStream`, `AsRawFd`, or `try_clone()` method.
///
/// ```compile_fail
/// use linux_peer_credentials_supplier::OwnedPeerConnection;
/// fn retain_an_fd(mut connection: OwnedPeerConnection) {
///     let _fd = connection.with_connection_io_checked(|| Ok(()), |io, _| io.try_clone());
/// }
/// ```
pub struct PeerConnectionIo<'connection> {
    stream: &'connection mut UnixStream,
    credentials: &'connection KernelPeerCredentials,
    require_current_writer_credentials: bool,
    check: &'connection mut dyn FnMut(
        &KernelPeerCredentials,
        &UnixStream,
    ) -> Result<(), PeerConnectionError>,
    failure: &'connection mut Option<PeerConnectionError>,
}

impl PeerConnectionIo<'_> {
    /// Re-run connection identity, peer-state, and caller authority checks at
    /// a logical processing boundary selected by the callback. Use this before
    /// each sensitive side effect after parsing or handling buffered data.
    /// The check is not atomic with work performed afterward; a service commit
    /// still needs its own live-authority synchronization.
    pub fn recheck_before_side_effect(&mut self) -> io::Result<()> {
        self.check_before_io()
    }

    fn check_before_io(&mut self) -> io::Result<()> {
        if let Some(error) = self.failure.as_ref() {
            return Err(callback_error(error));
        }
        match (self.check)(self.credentials, self.stream) {
            Ok(()) => Ok(()),
            Err(error) => {
                *self.failure = Some(error);
                Err(callback_error(
                    self.failure.as_ref().expect("failure just stored"),
                ))
            }
        }
    }

    fn perform_io<T>(
        &mut self,
        operation: impl FnOnce(&mut UnixStream) -> io::Result<T>,
    ) -> io::Result<T> {
        self.check_before_io()?;
        let result = operation(self.stream);

        self.finish_io_result(result)
    }

    fn finish_io_result<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        // Always check after the syscall, including WouldBlock/Interrupted.
        // Those two I/O errors are retryable and do not alone stale a stream;
        // a separate connection or authority failure still propagates here.
        match (self.check)(self.credentials, self.stream) {
            Err(error) => {
                let callback_error = callback_error(&error);
                *self.failure = Some(error);
                Err(callback_error)
            }
            Ok(()) => match result {
                Err(source) if !is_retryable_io(&source) => {
                    let error = PeerConnectionError::IoFailure {
                        kind: source.kind(),
                        raw_os_error: source.raw_os_error(),
                        message: source.to_string(),
                    };
                    let returned = io::Error::new(source.kind(), source.to_string());
                    *self.failure = Some(error);
                    Err(returned)
                }
                result => result,
            },
        }
    }
}

impl Read for PeerConnectionIo<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if !self.require_current_writer_credentials {
            return self.perform_io(|stream| stream.read(buffer));
        }

        self.check_before_io()?;
        let received = receive_from_current_writer(self.stream, self.credentials, buffer.len());
        // Keep received bytes private until both the writer credential and
        // the normal post-syscall owner check have succeeded.
        let received = self.finish_io_result(received)?;
        let count = received.len();
        buffer[..count].copy_from_slice(&received);
        Ok(count)
    }
}

impl Write for PeerConnectionIo<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.perform_io(|stream| stream.write(buffer))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.perform_io(UnixStream::flush)
    }
}

/// Owns the exact stream from which its non-cloneable credential snapshot was
/// read. Dropping this value closes its owned descriptor; any alias created
/// earlier may keep the kernel socket alive. A disconnected or rejected stream
/// is latched stale; a reconnect must be accepted into a new value and gets a
/// new snapshot. Credentials are connection metadata, not Nagi authority. The
/// constructor caller must transfer its sole owned FD: aliases that existed
/// before transfer cannot be discovered or revoked here.
pub struct OwnedPeerConnection {
    pub(super) stream: UnixStream,
    pub(super) credentials: KernelPeerCredentials,
    pub(super) require_current_writer_credentials: bool,
    pub(super) invalidated: bool,
}

/// A one-shot, deadline-bound transfer of the exact stream owned by an
/// `OwnedPeerConnection`.
///
/// The transfer retains the credential snapshot privately until
/// `consume_with_stream()` rechecks that it still belongs to this same live
/// socket.
/// The stream cannot be returned as the transfer API's callback result: the
/// callback receives its ownership and returns only `()`. This limits direct
/// result escape, but the callback can move the stream into captured state or
/// call `try_clone()` and retain an alias; aliases made before transfer cannot
/// be revoked. The callback is not proof that its recipient is the registered
/// endpoint owner; task-41 must invoke it only from that owner. This is
/// transport continuity, not Nagi identity or process liveness, session,
/// capability, consent, or service authorization. Dropping this value closes
/// its owned descriptor.
#[must_use = "consume the transfer with consume_with_stream(); dropping it closes the socket"]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct OwnedPeerStreamTransfer {
    stream: Option<UnixStream>,
    credentials: Option<KernelPeerCredentials>,
    require_current_writer_credentials: bool,
    deadline: Instant,
}

impl fmt::Debug for OwnedPeerStreamTransfer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedPeerStreamTransfer")
            .field("stream", &"owned")
            .field("snapshot", &"private and connection-bound")
            .field(
                "current_writer_credentials",
                &self.require_current_writer_credentials,
            )
            .field("deadline", &self.deadline)
            .finish()
    }
}

impl OwnedPeerStreamTransfer {
    /// The exact absolute deadline supplied when this stream was transferred.
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Revalidate and hand the exact owned stream to the endpoint-owner
    /// callback without returning it from this API.
    ///
    /// The connection-time PID/UID/GID snapshot is intentionally discarded;
    /// the receiving authority must obtain any evidence it needs from its own
    /// live owner and may not treat the old snapshot as Nagi authorization.
    /// This method consumes the transfer and the callback cannot return a
    /// stream as its return value, but can move the stream into captured state
    /// or call `try_clone()` and retain an alias. This API does not claim
    /// sole-descriptor enforcement or registration proof. The task-41 endpoint
    /// owner must keep its callback bounded by the supplied absolute deadline
    /// and bind the stream to its registered endpoint before it can authorize
    /// any request.
    ///
    /// Returning the raw stream as the callback result is rejected:
    ///
    /// ```compile_fail
    /// use linux_peer_credentials_supplier::OwnedPeerStreamTransfer;
    /// fn return_stream(transfer: OwnedPeerStreamTransfer) {
    ///     let _ = transfer.consume_with_stream(|stream, _| Ok(stream));
    /// }
    /// ```
    ///
    /// A consumed transfer cannot be used for a second callback:
    ///
    /// ```compile_fail
    /// use linux_peer_credentials_supplier::OwnedPeerStreamTransfer;
    /// fn consume_twice(transfer: OwnedPeerStreamTransfer) {
    ///     let _first = transfer.consume_with_stream(|_, _| Ok(()));
    ///     let _second = transfer.consume_with_stream(|_, _| Ok(()));
    /// }
    /// ```
    ///
    /// Re-entering through a captured reference to the same transfer is also
    /// rejected because this callback consumes the transfer before it runs:
    ///
    /// ```compile_fail
    /// use linux_peer_credentials_supplier::OwnedPeerStreamTransfer;
    /// fn reenter_from_callback(transfer: OwnedPeerStreamTransfer) {
    ///     let _ = transfer.consume_with_stream(|_, _| {
    ///         let _again = transfer.consume_with_stream(|_, _| Ok(()));
    ///         Ok(())
    ///     });
    /// }
    /// ```
    pub fn consume_with_stream(
        self,
        receive: impl FnOnce(UnixStream, Instant) -> Result<(), PeerConnectionError>,
    ) -> Result<(), PeerConnectionError> {
        #[cfg(target_os = "linux")]
        {
            self.consume_with_stream_using(
                Instant::now,
                |credentials, stream| {
                    credentials
                        .require_same_connection(stream)
                        .map_err(map_credential_error)?;
                    probe_peer(stream)
                },
                receive,
            )
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (self, receive);
            Err(PeerConnectionError::Unavailable)
        }
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn consume_with_stream_using(
        mut self,
        mut now: impl FnMut() -> Instant,
        validate: impl FnOnce(&KernelPeerCredentials, &UnixStream) -> Result<(), PeerConnectionError>,
        receive: impl FnOnce(UnixStream, Instant) -> Result<(), PeerConnectionError>,
    ) -> Result<(), PeerConnectionError> {
        if now() >= self.deadline {
            return Err(PeerConnectionError::DeadlineExceeded);
        }
        let credentials = self
            .credentials
            .as_ref()
            .ok_or(PeerConnectionError::Stale)?;
        let stream = self.stream.as_ref().ok_or(PeerConnectionError::Stale)?;
        #[cfg(target_os = "linux")]
        if self.require_current_writer_credentials {
            require_passcred(stream.as_raw_fd())?;
        }
        validate(credentials, stream)?;
        if now() >= self.deadline {
            return Err(PeerConnectionError::DeadlineExceeded);
        }
        // Empty the consumed transfer before user code runs. If the callback
        // panics or returns an error, this object cannot expose the stream or
        // snapshot again; only aliases the callback explicitly retained can
        // survive the unwind/return.
        drop(self.credentials.take());
        let stream = self.stream.take().ok_or(PeerConnectionError::Stale)?;
        if now() >= self.deadline {
            drop(stream);
            return Err(PeerConnectionError::DeadlineExceeded);
        }
        receive(stream, self.deadline)
    }
}

impl OwnedPeerConnection {
    /// Take ownership of an already-connected stream and capture credentials
    /// from that exact connection. The caller must not retain an alias created
    /// before this call; already escaped descriptors cannot be revoked here.
    /// Unsupported targets fail closed.
    pub fn from_connected(stream: UnixStream) -> Result<Self, PeerConnectionError> {
        #[cfg(target_os = "linux")]
        {
            let credentials =
                read_kernel_peer_credentials(&stream).map_err(map_credential_error)?;
            credentials
                .require_same_connection(&stream)
                .map_err(map_credential_error)?;
            probe_peer(&stream)?;
            Ok(Self {
                stream,
                credentials,
                require_current_writer_credentials: false,
                invalidated: false,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = stream;
            Err(PeerConnectionError::Unavailable)
        }
    }

    /// Take ownership of an accepted stream whose listener enabled
    /// `SO_PASSCRED` before clients could connect. Every later read uses
    /// `recvmsg` and accepts bytes only when the kernel's per-write
    /// `SCM_CREDENTIALS` PID/UID/GID matches this connection's `SO_PEERCRED`
    /// snapshot. Unsupported targets and sockets without inherited
    /// `SO_PASSCRED` fail closed.
    pub fn from_connected_with_current_writer_credentials(
        stream: UnixStream,
    ) -> Result<Self, PeerConnectionError> {
        #[cfg(target_os = "linux")]
        {
            // Do not enable SO_PASSCRED here: setting it after accept would
            // not prove that this connection inherited the listener's
            // receive flag when the client connected. The listener owner must
            // configure the option before exposing/listening on the endpoint.
            require_passcred(stream.as_raw_fd())?;
            let mut connection = Self::from_connected(stream)?;
            connection.require_current_writer_credentials = true;
            Ok(connection)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = stream;
            Err(PeerConnectionError::Unavailable)
        }
    }

    /// Validate and transfer this exact owned socket once, before an absolute
    /// deadline. A registered endpoint owner can use this to pass the accepted
    /// stream into its private route/endpoint checkout constructor without
    /// accepting a caller-supplied raw FD. The returned transfer retains the
    /// deadline and a private same-socket snapshot check until the next owner
    /// consumes it. No PID/UID/GID evidence is returned or promoted to
    /// authorization.
    ///
    /// This consumes `self`: after success there is no task-39 owner that can
    /// continue I/O or be asked for another take. The receiving callback gets
    /// this owner's descriptor and is responsible for its own live authority
    /// checks. Aliases created before this call remain outside task-39's
    /// control and are not revoked.
    ///
    /// ```compile_fail
    /// use linux_peer_credentials_supplier::OwnedPeerConnection;
    /// use std::time::Instant;
    /// fn take_twice(connection: OwnedPeerConnection, deadline: Instant) {
    ///     let _first = connection.into_stream_transfer_until(deadline);
    ///     let _second = connection.into_stream_transfer_until(deadline);
    /// }
    /// ```
    pub fn into_stream_transfer_until(
        self,
        deadline: Instant,
    ) -> Result<OwnedPeerStreamTransfer, PeerConnectionError> {
        #[cfg(target_os = "linux")]
        {
            self.into_stream_transfer_until_using(deadline, Instant::now, |credentials, stream| {
                credentials
                    .require_same_connection(stream)
                    .map_err(map_credential_error)?;
                probe_peer(stream)
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = (self, deadline);
            Err(PeerConnectionError::Unavailable)
        }
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn into_stream_transfer_until_using(
        self,
        deadline: Instant,
        mut now: impl FnMut() -> Instant,
        validate: impl FnOnce(&KernelPeerCredentials, &UnixStream) -> Result<(), PeerConnectionError>,
    ) -> Result<OwnedPeerStreamTransfer, PeerConnectionError> {
        if self.invalidated {
            return Err(PeerConnectionError::Stale);
        }
        if now() >= deadline {
            return Err(PeerConnectionError::DeadlineExceeded);
        }
        validate(&self.credentials, &self.stream)?;
        if now() >= deadline {
            return Err(PeerConnectionError::DeadlineExceeded);
        }
        #[cfg(target_os = "linux")]
        if self.require_current_writer_credentials {
            require_passcred(self.stream.as_raw_fd())?;
        }
        let Self {
            stream,
            credentials,
            require_current_writer_credentials,
            invalidated: _,
        } = self;
        let transfer = OwnedPeerStreamTransfer {
            stream: Some(stream),
            credentials: Some(credentials),
            require_current_writer_credentials,
            deadline,
        };
        if now() >= deadline {
            drop(transfer);
            return Err(PeerConnectionError::DeadlineExceeded);
        }
        Ok(transfer)
    }

    /// Run a temporary I/O callback with an independent authority hook.
    ///
    /// The required `authority_check` is called before and after the callback
    /// and on both sides of every read/write/flush. It must be a quick check
    /// against the caller's live session/consent/registration authority and
    /// return `AuthorityRejected` (or a more specific connection error) on
    /// denial. It is only a propagation hook: a no-op hook provides no
    /// authorization, and kernel PID/UID/GID/SO_COOKIE are never converted to
    /// Nagi authority here. Hook checks cannot run while a blocking syscall is
    /// blocked. If a check rejects after a partial write, the bytes already
    /// sent remain sent; the callback result is discarded and this adapter
    /// becomes stale.
    pub fn with_connection_io_checked<T>(
        &mut self,
        mut authority_check: impl FnMut() -> Result<(), PeerConnectionError>,
        operation: impl for<'connection> FnOnce(
            &mut PeerConnectionIo<'connection>,
            &KernelPeerCredentials,
        ) -> T,
    ) -> Result<T, PeerConnectionError> {
        let Self {
            stream,
            credentials,
            require_current_writer_credentials,
            invalidated,
        } = self;
        with_connection_io_using_mode(
            invalidated,
            credentials,
            stream,
            *require_current_writer_credentials,
            |credentials, stream| {
                credentials
                    .require_same_connection(stream)
                    .map_err(map_credential_error)?;
                probe_peer(stream)?;
                authority_check()
            },
            operation,
        )
    }

    /// Run checked I/O while letting the caller compare a borrowed handle to
    /// this exact connection at every callback and I/O boundary. The handle is
    /// scoped to the check callback and is never returned or retained by this
    /// adapter. Use this to bind an independent opaque peer proof to the same
    /// descriptor whose reads enforce per-write credentials.
    #[cfg(target_os = "linux")]
    pub fn with_connection_io_checked_with_fd<T>(
        &mut self,
        mut connection_check: impl for<'fd> FnMut(
            BorrowedFd<'fd>,
            &KernelPeerCredentials,
        ) -> Result<(), PeerConnectionError>,
        operation: impl for<'connection> FnOnce(
            &mut PeerConnectionIo<'connection>,
            &KernelPeerCredentials,
        ) -> T,
    ) -> Result<T, PeerConnectionError> {
        let Self {
            stream,
            credentials,
            require_current_writer_credentials,
            invalidated,
        } = self;
        with_connection_io_using_mode(
            invalidated,
            credentials,
            stream,
            *require_current_writer_credentials,
            |credentials, stream| {
                credentials
                    .require_same_connection(stream)
                    .map_err(map_credential_error)?;
                probe_peer(stream)?;
                connection_check(stream.as_fd(), credentials)
            },
            operation,
        )
    }

    /// Run an operation on a nonblocking stream with an absolute deadline.
    ///
    /// This sets the owned stream to nonblocking mode before callback entry
    /// and leaves it nonblocking for the remainder of this connection. Each
    /// syscall is therefore nonblocking; `WouldBlock` is returned to the
    /// callback, which may yield and retry while the deadline remains. The
    /// adapter checks the deadline, socket identity/state, and caller hook at
    /// callback and I/O boundaries. Crossing the deadline latches this peer
    /// stale and rejects later operations. It does not preempt arbitrary
    /// callback computation, and any bytes sent before expiry remain sent.
    /// The deadline is not a Nagi authorization source; missing authority must
    /// still be returned by the required hook.
    pub fn with_nonblocking_io_until<T>(
        &mut self,
        deadline: Instant,
        mut authority_check: impl FnMut() -> Result<(), PeerConnectionError>,
        operation: impl for<'connection> FnOnce(
            &mut PeerConnectionIo<'connection>,
            &KernelPeerCredentials,
        ) -> T,
    ) -> Result<T, PeerConnectionError> {
        self.with_nonblocking_io_until_peer_check(deadline, |_| authority_check(), operation)
    }

    /// Run bounded nonblocking I/O while rechecking a caller-owned live peer
    /// session using this connection's kernel credential snapshot.
    ///
    /// This exposes no stream or descriptor to the authority callback. The
    /// callback must resolve credentials through the existing init-owned live
    /// process/session/consent/registration owners; returning `Ok(())` based
    /// only on PID/UID/GID, socket path, or request bytes is not authorization.
    pub fn with_nonblocking_io_until_peer_check<T>(
        &mut self,
        deadline: Instant,
        mut authority_check: impl FnMut(&KernelPeerCredentials) -> Result<(), PeerConnectionError>,
        operation: impl for<'connection> FnOnce(
            &mut PeerConnectionIo<'connection>,
            &KernelPeerCredentials,
        ) -> T,
    ) -> Result<T, PeerConnectionError> {
        let Self {
            stream,
            credentials,
            require_current_writer_credentials,
            invalidated,
        } = self;
        with_nonblocking_io_until_using_mode(
            invalidated,
            credentials,
            stream,
            deadline,
            *require_current_writer_credentials,
            |credentials, stream| {
                credentials
                    .require_same_connection(stream)
                    .map_err(map_credential_error)?;
                probe_peer(stream)?;
                authority_check(credentials)
            },
            operation,
        )
    }

    /// Run bounded nonblocking I/O with a descriptor-aware owner check at
    /// every callback and I/O boundary. The borrowed descriptor cannot be
    /// returned from the check callback; all request reads still pass through
    /// this connection's `PeerConnectionIo`, including per-write credential
    /// validation when the accepted listener enabled `SO_PASSCRED`.
    #[cfg(target_os = "linux")]
    pub fn with_nonblocking_io_until_peer_check_with_fd<T>(
        &mut self,
        deadline: Instant,
        mut connection_check: impl for<'fd> FnMut(
            BorrowedFd<'fd>,
            &KernelPeerCredentials,
        ) -> Result<(), PeerConnectionError>,
        operation: impl for<'connection> FnOnce(
            &mut PeerConnectionIo<'connection>,
            &KernelPeerCredentials,
        ) -> T,
    ) -> Result<T, PeerConnectionError> {
        let Self {
            stream,
            credentials,
            require_current_writer_credentials,
            invalidated,
        } = self;
        with_nonblocking_io_until_using_mode(
            invalidated,
            credentials,
            stream,
            deadline,
            *require_current_writer_credentials,
            |credentials, stream| {
                credentials
                    .require_same_connection(stream)
                    .map_err(map_credential_error)?;
                probe_peer(stream)?;
                connection_check(stream.as_fd(), credentials)
            },
            operation,
        )
    }

    /// Check that a candidate FD is the same local socket object as this
    /// snapshot. This does not transfer ownership or expose credentials for
    /// the candidate; a different connection fails with `ConnectionMismatch`.
    pub fn require_same_socket(&self, candidate: &UnixStream) -> Result<(), PeerConnectionError> {
        if self.invalidated {
            return Err(PeerConnectionError::Stale);
        }
        self.credentials
            .require_same_connection(candidate)
            .map_err(map_credential_error)
    }
}

fn map_credential_error(error: PeerCredentialError) -> PeerConnectionError {
    match error {
        PeerCredentialError::Unavailable => PeerConnectionError::Unavailable,
        PeerCredentialError::NotConnected { .. } => PeerConnectionError::PeerDisconnected,
        other => PeerConnectionError::CredentialRead(other),
    }
}

fn callback_error(error: &PeerConnectionError) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, error.to_string())
}

fn is_retryable_io(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::Interrupted || error.kind() == io::ErrorKind::WouldBlock
}

fn with_connection_io_using<T>(
    invalidated: &mut bool,
    credentials: &KernelPeerCredentials,
    stream: &mut UnixStream,
    mut check: impl FnMut(&KernelPeerCredentials, &UnixStream) -> Result<(), PeerConnectionError>,
    operation: impl for<'connection> FnOnce(
        &mut PeerConnectionIo<'connection>,
        &KernelPeerCredentials,
    ) -> T,
) -> Result<T, PeerConnectionError> {
    with_connection_io_using_mode(invalidated, credentials, stream, false, check, operation)
}

fn with_connection_io_using_mode<T>(
    invalidated: &mut bool,
    credentials: &KernelPeerCredentials,
    stream: &mut UnixStream,
    require_current_writer_credentials: bool,
    mut check: impl FnMut(&KernelPeerCredentials, &UnixStream) -> Result<(), PeerConnectionError>,
    operation: impl for<'connection> FnOnce(
        &mut PeerConnectionIo<'connection>,
        &KernelPeerCredentials,
    ) -> T,
) -> Result<T, PeerConnectionError> {
    if *invalidated {
        return Err(PeerConnectionError::Stale);
    }
    if let Err(error) = check(credentials, stream) {
        *invalidated = true;
        return Err(error);
    }

    let mut failure = None;
    let result = {
        let mut io = PeerConnectionIo {
            stream,
            credentials,
            require_current_writer_credentials,
            check: &mut check,
            failure: &mut failure,
        };
        operation(&mut io, credentials)
    };

    // Preserve callback-boundary checks even if the callback swallowed a
    // per-operation error. The first failure remains the reported cause.
    let post_check = check(credentials, stream);
    if let Some(error) = failure {
        *invalidated = true;
        return Err(error);
    }
    if let Err(error) = post_check {
        *invalidated = true;
        return Err(error);
    }
    Ok(result)
}

pub(super) fn with_nonblocking_io_until_using<T>(
    invalidated: &mut bool,
    credentials: &KernelPeerCredentials,
    stream: &mut UnixStream,
    deadline: Instant,
    mut check: impl FnMut(&KernelPeerCredentials, &UnixStream) -> Result<(), PeerConnectionError>,
    operation: impl for<'connection> FnOnce(
        &mut PeerConnectionIo<'connection>,
        &KernelPeerCredentials,
    ) -> T,
) -> Result<T, PeerConnectionError> {
    with_nonblocking_io_until_using_mode(
        invalidated,
        credentials,
        stream,
        deadline,
        false,
        check,
        operation,
    )
}

fn with_nonblocking_io_until_using_mode<T>(
    invalidated: &mut bool,
    credentials: &KernelPeerCredentials,
    stream: &mut UnixStream,
    deadline: Instant,
    require_current_writer_credentials: bool,
    mut check: impl FnMut(&KernelPeerCredentials, &UnixStream) -> Result<(), PeerConnectionError>,
    operation: impl for<'connection> FnOnce(
        &mut PeerConnectionIo<'connection>,
        &KernelPeerCredentials,
    ) -> T,
) -> Result<T, PeerConnectionError> {
    if *invalidated {
        return Err(PeerConnectionError::Stale);
    }
    if let Err(source) = stream.set_nonblocking(true) {
        *invalidated = true;
        return Err(PeerConnectionError::NonblockingSetup { source });
    }
    with_connection_io_using_mode(
        invalidated,
        credentials,
        stream,
        require_current_writer_credentials,
        |credentials, stream| {
            if Instant::now() >= deadline {
                return Err(PeerConnectionError::DeadlineExceeded);
            }
            check(credentials, stream)
        },
        operation,
    )
}

#[cfg(target_os = "linux")]
/// Enable and verify per-write Unix credentials on an AF_UNIX listener before
/// calling `listen` or making the endpoint reachable. Accepted sockets verify
/// that they inherited the option; enabling it after `accept` is insufficient.
pub fn enable_passcred(fd: RawFd) -> io::Result<()> {
    let enabled: libc::c_int = 1;
    // SAFETY: `fd` is borrowed from a live AF_UNIX socket and `enabled` is a
    // valid SOL_SOCKET/SO_PASSCRED integer option value.
    let set_result = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PASSCRED,
            (&enabled as *const libc::c_int).cast(),
            std::mem::size_of_val(&enabled) as libc::socklen_t,
        )
    };
    if set_result != 0 {
        return Err(io::Error::last_os_error());
    }

    verify_passcred(fd)
}

#[cfg(target_os = "linux")]
fn verify_passcred(fd: RawFd) -> io::Result<()> {
    let mut observed = 0;
    let mut length = std::mem::size_of_val(&observed) as libc::socklen_t;
    // SAFETY: output pointers refer to initialized writable storage of the
    // exact size declared in `length`; `fd` remains borrowed and live.
    let get_result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PASSCRED,
            (&mut observed as *mut libc::c_int).cast(),
            &mut length,
        )
    };
    if get_result != 0 {
        return Err(io::Error::last_os_error());
    }
    if length as usize != std::mem::size_of_val(&observed) || observed != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "listener did not retain SO_PASSCRED",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn require_passcred(fd: RawFd) -> Result<(), PeerConnectionError> {
    match verify_passcred(fd) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::InvalidData => {
            Err(PeerConnectionError::CurrentWriterCredentialsUnavailable)
        }
        Err(source) => Err(PeerConnectionError::CurrentWriterCredentialsOptionRead { source }),
    }
}

#[cfg(target_os = "linux")]
fn receive_from_current_writer(
    stream: &UnixStream,
    expected: &KernelPeerCredentials,
    requested: usize,
) -> io::Result<Vec<u8>> {
    const MAX_READ_BYTES: usize = 64 * 1024;
    const CONTROL_BYTES: usize = 8192;

    if requested == 0 {
        return Ok(Vec::new());
    }
    require_passcred(stream.as_raw_fd())
        .map_err(|error| io::Error::new(io::ErrorKind::PermissionDenied, error.to_string()))?;
    let mut bytes = vec![0u8; requested.min(MAX_READ_BYTES)];
    let control_words = CONTROL_BYTES.div_ceil(std::mem::size_of::<libc::c_long>());
    let mut control = vec![0 as libc::c_long; control_words];
    let mut vector = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
    message.msg_iov = &mut vector;
    message.msg_iovlen = 1;
    message.msg_control = control.as_mut_ptr().cast();
    let control_capacity = std::mem::size_of_val(control.as_slice());
    // libc exposes this field as size_t on glibc and as a 32-bit length on
    // musl. The ancillary buffer is fixed and bounded to at most 8192 bytes.
    message.msg_controllen = control_capacity as _;

    // SAFETY: the iovec and control buffer remain writable and alive for the
    // syscall. MSG_CMSG_CLOEXEC prevents received descriptors from surviving
    // exec; all received SCM_RIGHTS descriptors are closed before return.
    let received = unsafe {
        libc::recvmsg(
            stream.as_raw_fd(),
            &mut message,
            libc::MSG_DONTWAIT | libc::MSG_CMSG_CLOEXEC,
        )
    };
    if received < 0 {
        return Err(io::Error::last_os_error());
    }
    let received = received as usize;
    validate_current_writer_control(
        &mut message,
        &control,
        control_capacity,
        received,
        bytes.len(),
        expected,
    )?;
    bytes.truncate(received);
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn validate_current_writer_control(
    message: &mut libc::msghdr,
    control: &[libc::c_long],
    control_capacity: usize,
    received: usize,
    byte_capacity: usize,
    expected: &KernelPeerCredentials,
) -> io::Result<()> {
    let mut credential = None;
    // Convert the ABI-sized result without truncation, then clamp before
    // passing the message back through the CMSG traversal macros.
    let reported_control_length = usize::try_from(message.msg_controllen).unwrap_or(usize::MAX);
    let mut malformed = received > byte_capacity || reported_control_length > control_capacity;
    let mut unexpected = false;
    let control_length = reported_control_length.min(control_capacity);
    message.msg_controllen = control_length as _;
    let message_ptr = message as *const libc::msghdr;
    let mut header = unsafe { libc::CMSG_FIRSTHDR(message_ptr) };
    while !header.is_null() {
        // SAFETY: CMSG_FIRSTHDR/CMSG_NXTHDR point only within the returned
        // control range, which was clamped to our owned buffer above.
        let item = unsafe { &*header };
        let item_length = item.cmsg_len as usize;
        let header_length = unsafe { libc::CMSG_LEN(0) as usize };
        let header_offset = (header as usize).saturating_sub(control.as_ptr() as usize);
        let item_end = header_offset.checked_add(item_length);
        if item_length < header_length || item_end.is_none() || item_end.unwrap() > control_length {
            malformed = true;
            break;
        }
        let data_length = item_length - header_length;
        let data = unsafe { libc::CMSG_DATA(header) };
        match (item.cmsg_level, item.cmsg_type) {
            (libc::SOL_SOCKET, libc::SCM_CREDENTIALS) => {
                if data_length != std::mem::size_of::<libc::ucred>() || credential.is_some() {
                    malformed = true;
                } else {
                    // SAFETY: exact payload length was checked and unaligned
                    // reads are permitted for kernel-provided ancillary data.
                    credential =
                        Some(unsafe { std::ptr::read_unaligned(data.cast::<libc::ucred>()) });
                }
            }
            (libc::SOL_SOCKET, libc::SCM_RIGHTS) => {
                unexpected = true;
                let descriptor_count = data_length / std::mem::size_of::<libc::c_int>();
                for index in 0..descriptor_count {
                    // SAFETY: each descriptor read stays within the validated
                    // SCM_RIGHTS payload. Close every delivered descriptor.
                    let descriptor = unsafe {
                        std::ptr::read_unaligned(
                            data.add(index * std::mem::size_of::<libc::c_int>())
                                .cast::<libc::c_int>(),
                        )
                    };
                    if descriptor >= 0 {
                        // SAFETY: this descriptor was installed for this
                        // recvmsg control message and is now owned by us.
                        unsafe { libc::close(descriptor) };
                    }
                }
                if data_length % std::mem::size_of::<libc::c_int>() != 0 {
                    malformed = true;
                }
            }
            _ => unexpected = true,
        }
        header = unsafe { libc::CMSG_NXTHDR(message_ptr, header) };
    }

    if message.msg_flags & (libc::MSG_CTRUNC | libc::MSG_TRUNC) != 0 || malformed || unexpected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "malformed or unexpected Unix stream ancillary data",
        ));
    }
    match credential {
        Some(actual)
            if actual.pid == expected.pid
                && actual.uid == expected.uid
                && actual.gid == expected.gid => {}
        Some(_) => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Unix stream writer credentials differ from accepted peer",
            ));
        }
        None if received == 0 => return Ok(()),
        None => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Unix stream read omitted current-writer credentials",
            ));
        }
    }
    Ok(())
}

/// Exercise the production ancillary-data validator with malformed Linux
/// control-message inputs. The C2 guest fixture separately exercises the
/// successful path through a real accepted Unix peer.
#[cfg(all(target_os = "linux", feature = "linux-runtime-fixture"))]
#[doc(hidden)]
pub fn run_current_writer_credential_negative_fixture() -> Result<(), String> {
    use std::mem::{size_of, size_of_val};

    #[derive(Clone, Copy, Debug)]
    enum Input {
        Missing,
        Duplicate,
        TruncatedPayload,
        TruncatedControl,
        OverreportedLength,
    }

    fn check(input: Input, expected_kind: io::ErrorKind) -> Result<(), String> {
        let credentials = KernelPeerCredentials {
            pid: unsafe { libc::getpid() },
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
            local_socket_cookie: 1,
        };
        let payload_size = size_of::<libc::ucred>();
        let payload_size_arg = payload_size as _;
        let space = unsafe { libc::CMSG_SPACE(payload_size_arg) as usize };
        let truncated_space = unsafe { libc::CMSG_SPACE((payload_size - 1) as _) as usize };
        let control_bytes = match input {
            Input::Missing => 0,
            Input::Duplicate => space.checked_mul(2).ok_or("control size overflow")?,
            Input::TruncatedPayload => space,
            Input::TruncatedControl => space,
            Input::OverreportedLength => space,
        };
        let mut control =
            vec![0 as libc::c_long; control_bytes.div_ceil(size_of::<libc::c_long>())];
        let allocated_bytes = size_of_val(control.as_slice());
        let mut message: libc::msghdr = unsafe { std::mem::zeroed() };
        message.msg_control = control.as_mut_ptr().cast();
        message.msg_controllen = control_bytes as _;
        message.msg_flags = if matches!(input, Input::TruncatedControl) {
            libc::MSG_CTRUNC
        } else {
            0
        };

        if !matches!(input, Input::Missing) {
            let header = unsafe { libc::CMSG_FIRSTHDR(&message) };
            if header.is_null() {
                return Err("fixture could not allocate first control header".into());
            }
            unsafe {
                (*header).cmsg_level = libc::SOL_SOCKET;
                (*header).cmsg_type = libc::SCM_CREDENTIALS;
                (*header).cmsg_len = match input {
                    Input::TruncatedPayload => libc::CMSG_LEN((payload_size - 1) as _) as _,
                    _ => libc::CMSG_LEN(payload_size_arg) as _,
                };
                if !matches!(input, Input::TruncatedPayload) {
                    std::ptr::write_unaligned(
                        libc::CMSG_DATA(header).cast::<libc::ucred>(),
                        libc::ucred {
                            pid: credentials.pid,
                            uid: credentials.uid,
                            gid: credentials.gid,
                        },
                    );
                }
            }
            if matches!(input, Input::Duplicate) {
                let duplicate = unsafe { libc::CMSG_NXTHDR(&message, header) };
                if duplicate.is_null() {
                    return Err("fixture could not allocate duplicate control header".into());
                }
                unsafe {
                    (*duplicate).cmsg_level = libc::SOL_SOCKET;
                    (*duplicate).cmsg_type = libc::SCM_CREDENTIALS;
                    (*duplicate).cmsg_len = libc::CMSG_LEN(payload_size_arg) as _;
                    std::ptr::write_unaligned(
                        libc::CMSG_DATA(duplicate).cast::<libc::ucred>(),
                        libc::ucred {
                            pid: credentials.pid,
                            uid: credentials.uid,
                            gid: credentials.gid,
                        },
                    );
                }
            }
        }

        // Keep the truncated-payload control range aligned to the actual
        // CMSG_SPACE allocation, while the cmsg_len advertises one short byte.
        if matches!(input, Input::TruncatedPayload) {
            message.msg_controllen = truncated_space.max(control_bytes) as _;
        }
        if matches!(input, Input::OverreportedLength) {
            message.msg_controllen = allocated_bytes
                .checked_add(1)
                .ok_or("overreported control length overflow")?
                as _;
        }
        let result = validate_current_writer_control(
            &mut message,
            &control,
            allocated_bytes,
            1,
            1,
            &credentials,
        );
        match result {
            Err(error) if error.kind() == expected_kind => Ok(()),
            Err(error) => Err(format!(
                "{input:?} returned {:?}, expected {expected_kind:?}: {error}",
                error.kind()
            )),
            Ok(()) => Err(format!("{input:?} unexpectedly accepted")),
        }
    }

    check(Input::Missing, io::ErrorKind::PermissionDenied)?;
    check(Input::Duplicate, io::ErrorKind::InvalidData)?;
    check(Input::TruncatedPayload, io::ErrorKind::InvalidData)?;
    check(Input::TruncatedControl, io::ErrorKind::InvalidData)?;
    check(Input::OverreportedLength, io::ErrorKind::InvalidData)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn probe_peer(stream: &UnixStream) -> Result<(), PeerConnectionError> {
    let fd = stream.as_raw_fd();
    probe_with(|| {
        let mut byte = 0u8;
        // SAFETY: `byte` is writable for one byte and `fd` is borrowed from
        // the owned UnixStream. PEEK|DONTWAIT does not consume stream data or
        // change the stream's blocking mode.
        let result = unsafe {
            libc::recv(
                fd,
                (&mut byte as *mut u8).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(result as isize)
        }
    })
}

#[cfg(not(target_os = "linux"))]
fn probe_peer(_stream: &UnixStream) -> Result<(), PeerConnectionError> {
    Err(PeerConnectionError::Unavailable)
}

#[cfg(any(target_os = "linux", test))]
fn probe_with(mut recv_peek: impl FnMut() -> io::Result<isize>) -> Result<(), PeerConnectionError> {
    loop {
        match recv_peek() {
            Ok(0) => return Err(PeerConnectionError::PeerDisconnected),
            Ok(_) => return Ok(()),
            Err(source) if source.raw_os_error() == Some(libc::EINTR) => continue,
            Err(source)
                if source.raw_os_error() == Some(libc::EAGAIN)
                    || source.raw_os_error() == Some(libc::EWOULDBLOCK) =>
            {
                return Ok(())
            }
            Err(source) => return Err(PeerConnectionError::ProbeRead { source }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::collections::VecDeque;
    use std::net::Shutdown;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    fn snapshot(cookie: u64) -> KernelPeerCredentials {
        KernelPeerCredentials {
            pid: 42,
            uid: 1000,
            gid: 1000,
            local_socket_cookie: cookie,
        }
    }

    fn injected_cookie_check(
        cookies: &HashMap<i32, u64>,
    ) -> impl FnMut(&KernelPeerCredentials, &UnixStream) -> Result<(), PeerConnectionError> + '_
    {
        move |credentials, stream| match cookies.get(&stream.as_raw_fd()) {
            Some(cookie) if *cookie == credentials.local_socket_cookie => Ok(()),
            Some(_) => Err(PeerConnectionError::CredentialRead(
                PeerCredentialError::ConnectionMismatch,
            )),
            None => Err(PeerConnectionError::CredentialRead(
                PeerCredentialError::KernelRead {
                    option: crate::SocketOption::SocketCookie,
                    source: io::Error::from_raw_os_error(libc::EBADF),
                },
            )),
        }
    }

    fn injected_socket_probe(stream: &UnixStream) -> Result<(), PeerConnectionError> {
        let fd = stream.as_raw_fd();
        probe_with(|| {
            let mut byte = 0u8;
            // SAFETY: test-only non-consuming one-byte peek into a live Unix FD.
            let result = unsafe {
                libc::recv(
                    fd,
                    (&mut byte as *mut u8).cast(),
                    1,
                    libc::MSG_PEEK | libc::MSG_DONTWAIT,
                )
            };
            if result < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(result as isize)
            }
        })
    }

    fn peek_first_byte(stream: &UnixStream) -> io::Result<u8> {
        let mut byte = 0u8;
        // SAFETY: test-only non-consuming one-byte peek into a live Unix FD.
        let result = unsafe {
            libc::recv(
                stream.as_raw_fd(),
                (&mut byte as *mut u8).cast(),
                1,
                libc::MSG_PEEK | libc::MSG_DONTWAIT,
            )
        };
        match result {
            1 => Ok(byte),
            0 => Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            _ => Err(io::Error::last_os_error()),
        }
    }

    struct InjectedReadFaults<'io, 'connection> {
        inner: &'io mut PeerConnectionIo<'connection>,
        errors: VecDeque<io::ErrorKind>,
        max_chunk: usize,
    }

    impl Read for InjectedReadFaults<'_, '_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if let Some(kind) = self.errors.pop_front() {
                return Err(io::Error::from(kind));
            }
            let limit = buffer.len().min(self.max_chunk);
            self.inner.read(&mut buffer[..limit])
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn non_linux_adapter_is_unavailable() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        assert!(matches!(
            OwnedPeerConnection::from_connected(stream),
            Err(PeerConnectionError::Unavailable)
        ));
    }

    #[test]
    fn different_connection_cookie_cannot_reuse_snapshot_or_run_callback() {
        let (old_stream, _old_peer) = UnixStream::pair().unwrap();
        let (mut new_stream, _new_peer) = UnixStream::pair().unwrap();
        let mut cookies = HashMap::new();
        cookies.insert(old_stream.as_raw_fd(), 0x1111);
        cookies.insert(new_stream.as_raw_fd(), 0x2222);
        let mut invalidated = false;
        let called = std::cell::Cell::new(false);
        let result = with_connection_io_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut new_stream,
            injected_cookie_check(&cookies),
            |_, _| called.set(true),
        );
        assert!(matches!(
            result,
            Err(PeerConnectionError::CredentialRead(
                PeerCredentialError::ConnectionMismatch
            ))
        ));
        assert!(invalidated);
        assert!(!called.get());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn borrowed_fd_connection_check_runs_at_every_current_writer_io_boundary() {
        let (stream, mut writer) = UnixStream::pair().unwrap();
        let expected_cookie = read_kernel_peer_credentials(&stream)
            .unwrap()
            .local_socket_cookie;
        enable_passcred(stream.as_raw_fd()).unwrap();
        let mut connection =
            OwnedPeerConnection::from_connected_with_current_writer_credentials(stream).unwrap();
        writer.write_all(b"w").unwrap();

        let mut checked_boundaries = 0;
        let mut output = [0xa5; 1];
        let result = connection.with_connection_io_checked_with_fd(
            |fd, credentials| {
                checked_boundaries += 1;
                let mut observed_cookie = 0u64;
                let mut length = std::mem::size_of_val(&observed_cookie) as libc::socklen_t;
                // SAFETY: the borrowed handle stays live for this callback,
                // and both getsockopt outputs have the declared ABI sizes.
                let read = unsafe {
                    libc::getsockopt(
                        fd.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_COOKIE,
                        (&mut observed_cookie as *mut u64).cast(),
                        &mut length,
                    )
                };
                if read != 0 || length as usize != std::mem::size_of::<u64>() {
                    return Err(PeerConnectionError::AuthorityRejected);
                }
                if observed_cookie != expected_cookie
                    || observed_cookie != credentials.local_socket_cookie
                {
                    return Err(PeerConnectionError::AuthorityRejected);
                }
                Ok(())
            },
            |io, _| io.read(&mut output),
        );

        assert!(matches!(result, Ok(Ok(1))));
        assert_eq!(output, *b"w");
        assert_eq!(checked_boundaries, 4);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn borrowed_fd_connection_check_rejects_disconnected_peer_before_callback() {
        let (stream, peer) = UnixStream::pair().unwrap();
        enable_passcred(stream.as_raw_fd()).unwrap();
        let mut connection =
            OwnedPeerConnection::from_connected_with_current_writer_credentials(stream).unwrap();
        drop(peer);

        let mut callback_called = false;
        let result = connection
            .with_connection_io_checked_with_fd(|_, _| Ok(()), |_, _| callback_called = true);

        assert!(matches!(result, Err(PeerConnectionError::PeerDisconnected)));
        assert!(!callback_called);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn current_writer_credentials_reject_a_different_process_writer() {
        let (stream, mut writer) = UnixStream::pair().unwrap();
        enable_passcred(stream.as_raw_fd()).unwrap();
        let mut connection =
            OwnedPeerConnection::from_connected_with_current_writer_credentials(stream).unwrap();
        let receiver_fd = connection.stream.as_raw_fd();
        let writer_fd = writer.as_raw_fd();

        // This Linux-only test forks one child that writes through an inherited
        // socket FD. The child uses only raw close/write/pause/_exit syscalls;
        // the parent performs assertions and reaps it before returning.
        let child = unsafe { libc::fork() };
        assert!(child >= 0, "fork failed: {}", io::Error::last_os_error());
        if child == 0 {
            unsafe {
                libc::close(receiver_fd);
                let payload = b"w";
                if libc::write(writer_fd, payload.as_ptr().cast(), payload.len()) != 1 {
                    libc::_exit(2);
                }
                loop {
                    libc::pause();
                }
            }
        }

        drop(writer);
        let mut readiness = libc::pollfd {
            fd: receiver_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut readiness, 1, 5_000) };
        let mut output = [0xa5; 1];
        let result =
            if ready > 0 {
                Some(connection.with_connection_io_checked_with_fd(
                    |_, _| Ok(()),
                    |io, _| io.read(&mut output),
                ))
            } else {
                None
            };

        unsafe {
            libc::kill(child, libc::SIGKILL);
        }
        let mut child_status = 0;
        let waited = unsafe { libc::waitpid(child, &mut child_status, 0) };

        assert!(ready > 0, "child writer did not make the socket readable");
        assert_eq!(waited, child);
        assert!(libc::WIFSIGNALED(child_status));
        assert!(
            matches!(
                result.as_ref(),
                Some(Err(PeerConnectionError::IoFailure {
                    kind: io::ErrorKind::PermissionDenied,
                    ..
                }))
            ),
            "different writer was not denied: {result:?}"
        );
        assert_eq!(output, [0xa5]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn borrowed_fd_connection_check_rejection_precedes_payload_read() {
        let (stream, mut writer) = UnixStream::pair().unwrap();
        enable_passcred(stream.as_raw_fd()).unwrap();
        let mut connection =
            OwnedPeerConnection::from_connected_with_current_writer_credentials(stream).unwrap();
        writer.write_all(b"w").unwrap();

        let mut callback_called = false;
        let mut output = [0xa5; 1];
        let result = connection.with_connection_io_checked_with_fd(
            |_, _| Err(PeerConnectionError::AuthorityRejected),
            |io, _| {
                callback_called = true;
                io.read(&mut output)
            },
        );

        assert!(matches!(
            result,
            Err(PeerConnectionError::AuthorityRejected)
        ));
        assert!(!callback_called);
        assert_eq!(output, [0xa5]);
    }

    #[test]
    fn observed_eof_invalidates_snapshot_and_future_access() {
        let (mut stream, _peer) = UnixStream::pair().unwrap();
        let mut cookies = HashMap::new();
        cookies.insert(stream.as_raw_fd(), 0x1111);
        let mut invalidated = false;
        let first = with_connection_io_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            |_, _| Err(PeerConnectionError::PeerDisconnected),
            |_, _| panic!("a disconnected peer must not reach the callback"),
        );
        assert!(matches!(first, Err(PeerConnectionError::PeerDisconnected)));
        assert!(matches!(
            with_connection_io_using(
                &mut invalidated,
                &snapshot(0x1111),
                &mut stream,
                |_, _| panic!("invalidated connection must not be revalidated"),
                |_, _| panic!("invalidated snapshot must not be exposed"),
            ),
            Err(PeerConnectionError::Stale)
        ));
        let _ = cookies;
    }

    #[test]
    fn disconnect_during_callback_discards_result_and_latches_stale() {
        let (mut stream, _peer) = UnixStream::pair().unwrap();
        let mut checks = 0;
        let mut invalidated = false;
        let mut callback_ran = false;
        let result = with_connection_io_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            |_, _| {
                checks += 1;
                if checks == 2 {
                    Err(PeerConnectionError::PeerDisconnected)
                } else {
                    Ok(())
                }
            },
            |_, _| {
                callback_ran = true;
                "response"
            },
        );
        assert!(callback_ran);
        assert!(invalidated);
        assert!(matches!(result, Err(PeerConnectionError::PeerDisconnected)));
        assert_eq!(checks, 2);
    }

    #[test]
    fn queued_data_delays_eof_until_io_postcheck_after_callback_consumes_it() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(&[0xa5]).unwrap();
        drop(peer);
        let mut invalidated = false;
        let mut checks = 0;
        let mut consumed = 0u8;
        let result = with_connection_io_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            |_, stream| {
                checks += 1;
                if checks == 3 {
                    injected_socket_probe(stream)
                } else {
                    Ok(())
                }
            },
            |io, _| io.read_exact(std::slice::from_mut(&mut consumed)),
        );
        assert!(matches!(result, Err(PeerConnectionError::PeerDisconnected)));
        assert_eq!(consumed, 0xa5);
        assert!(invalidated);
        assert_eq!(checks, 4); // outer pre, I/O pre/post, outer post
    }

    #[test]
    fn queued_data_can_mask_eof_through_all_non_consuming_checks() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(&[0xa5]).unwrap();
        drop(peer);
        let mut invalidated = false;
        let mut checks = 0;
        let result = with_connection_io_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            |_, stream| {
                checks += 1;
                injected_socket_probe(stream)
            },
            |_, _| "probe saw pending data",
        );
        assert_eq!(result.unwrap(), "probe saw pending data");
        assert_eq!(checks, 2);
        assert!(!invalidated);
        let mut buffered = [0u8; 1];
        stream.read_exact(&mut buffered).unwrap();
        assert_eq!(buffered, [0xa5]);
    }

    #[test]
    fn per_primitive_checks_surround_partial_reads_and_writes() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(b"ab").unwrap();
        let mut checks = 0;
        let mut invalidated = false;
        let result = with_connection_io_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            |_, _| {
                checks += 1;
                Ok(())
            },
            |io, _| {
                let mut first = [0u8; 1];
                let mut second = [0u8; 1];
                let first_read = io.read(&mut first).unwrap();
                let second_read = io.read(&mut second).unwrap();
                let first_write = io.write(b"x").unwrap();
                let second_write = io.write(b"y").unwrap();
                (
                    first_read,
                    second_read,
                    first,
                    second,
                    first_write,
                    second_write,
                )
            },
        );
        assert_eq!(result.unwrap(), (1, 1, [b'a'], [b'b'], 1, 1));
        assert_eq!(checks, 10); // callback pre/post plus two checks per primitive
        let mut written = [0u8; 2];
        peer.read_exact(&mut written).unwrap();
        assert_eq!(&written, b"xy");
        assert!(!invalidated);
    }

    #[test]
    fn would_block_is_returned_without_invalidating_peer() {
        let (mut stream, _peer) = UnixStream::pair().unwrap();
        let mut checks = 0;
        let mut invalidated = false;
        let result = with_nonblocking_io_until_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            Instant::now() + Duration::from_secs(2),
            |_, _| {
                checks += 1;
                Ok(())
            },
            |io, _| {
                let mut byte = [0u8; 1];
                io.read(&mut byte)
            },
        );
        assert!(matches!(result, Ok(Err(ref error)) if error.kind() == io::ErrorKind::WouldBlock));
        assert_eq!(checks, 4);
        assert!(!invalidated);
    }

    #[test]
    fn partial_nonblocking_write_then_would_block_keeps_connection_usable() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(b"ab").unwrap();
        let payload = vec![0x5a; 8 * 1024 * 1024];
        let mut checks = 0;
        let mut invalidated = false;
        let result = with_nonblocking_io_until_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            Instant::now() + Duration::from_secs(2),
            |_, _| {
                checks += 1;
                Ok(())
            },
            |io, _| -> io::Result<(usize, [u8; 8], usize, bool, bool, usize)> {
                let mut input = [0u8; 8];
                let read_count = io.read(&mut input)?;
                let mut bytes = 0usize;
                let mut partial = false;
                let mut would_block = false;
                let mut attempts = 0usize;
                for _ in 0..64 {
                    attempts += 1;
                    match io.write(&payload) {
                        Ok(count) => {
                            bytes += count;
                            partial |= count < payload.len();
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            would_block = true;
                            break;
                        }
                        Err(error) => return Err(error),
                    }
                }
                Ok((read_count, input, bytes, partial, would_block, attempts))
            },
        );
        let (read_count, input, bytes, partial, would_block, attempts) = result.unwrap().unwrap();
        assert_eq!(read_count, 2);
        assert_eq!(&input[..read_count], b"ab");
        assert!(read_count < input.len());
        assert!(bytes > 0);
        assert!(partial);
        assert!(would_block);
        assert_eq!(checks, 2 + (attempts + 1) * 2);
        assert!(!invalidated);
    }

    #[test]
    fn expired_deadline_skips_callback_and_stales_next_operation() {
        let (mut stream, _peer) = UnixStream::pair().unwrap();
        let mut invalidated = false;
        let mut checked = false;
        let mut callback_ran = false;
        let result = with_nonblocking_io_until_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            Instant::now() - Duration::from_millis(1),
            |_, _| {
                checked = true;
                Ok(())
            },
            |_, _| callback_ran = true,
        );
        assert!(matches!(result, Err(PeerConnectionError::DeadlineExceeded)));
        assert!(!checked);
        assert!(!callback_ran);
        assert!(invalidated);
        assert!(matches!(
            with_nonblocking_io_until_using(
                &mut invalidated,
                &snapshot(0x1111),
                &mut stream,
                Instant::now() + Duration::from_secs(2),
                |_, _| panic!("stale adapter must not call checks"),
                |_, _| panic!("stale adapter must not reach callback"),
            ),
            Err(PeerConnectionError::Stale)
        ));
    }

    #[test]
    fn deadline_expiry_after_write_preserves_sent_bytes_and_denies_next_write() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        let mut invalidated = false;
        let deadline = Instant::now() + Duration::from_millis(250);
        let after_deadline_denied = std::cell::Cell::new(false);
        let result = with_nonblocking_io_until_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            deadline,
            |_, stream| injected_socket_probe(stream),
            |io, _| {
                let first_write = io.write(b"sent");
                std::thread::sleep(Duration::from_millis(300));
                let after_deadline = io.write(b"late");
                after_deadline_denied.set(after_deadline.is_err());
                (first_write, after_deadline)
            },
        );
        assert!(matches!(result, Err(PeerConnectionError::DeadlineExceeded)));
        assert!(after_deadline_denied.get());
        assert!(invalidated);
        let mut sent = [0u8; 4];
        peer.read_exact(&mut sent).unwrap();
        assert_eq!(&sent, b"sent");
        peer.set_nonblocking(true).unwrap();
        let mut late = [0u8; 4];
        assert!(matches!(
            peer.read(&mut late),
            Err(ref error) if error.kind() == io::ErrorKind::WouldBlock
        ));
        assert!(matches!(
            with_nonblocking_io_until_using(
                &mut invalidated,
                &snapshot(0x1111),
                &mut stream,
                Instant::now() + Duration::from_secs(2),
                |_, _| panic!("stale adapter must not recheck"),
                |_, _| panic!("stale adapter must not write after expiry"),
            ),
            Err(PeerConnectionError::Stale)
        ));
    }

    #[test]
    fn nonblocking_socketpair_handles_would_block_until_deadline_then_stops() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        let deadline = Instant::now() + Duration::from_millis(150);
        let mut invalidated = false;
        let mut read_attempts = 0usize;
        let late_write_denied = std::cell::Cell::new(false);
        let result = with_nonblocking_io_until_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            deadline,
            |_, stream| injected_socket_probe(stream),
            |io, _| {
                let mut byte = [0u8; 1];
                while Instant::now() < deadline {
                    match io.read(&mut byte) {
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            read_attempts += 1;
                        }
                        other => panic!("empty socket read was not WouldBlock: {other:?}"),
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                late_write_denied.set(io.write(b"after deadline").is_err());
            },
        );
        assert!(read_attempts > 0);
        assert!(matches!(result, Err(PeerConnectionError::DeadlineExceeded)));
        assert!(late_write_denied.get());
        assert!(invalidated);
        peer.set_nonblocking(true).unwrap();
        let mut received = [0u8; 1];
        assert!(matches!(
            peer.read(&mut received),
            Err(ref error) if error.kind() == io::ErrorKind::WouldBlock
        ));
        assert!(matches!(
            with_nonblocking_io_until_using(
                &mut invalidated,
                &snapshot(0x1111),
                &mut stream,
                Instant::now() + Duration::from_secs(2),
                |_, _| panic!("expired adapter must not check again"),
                |_, _| panic!("expired adapter must not perform another operation"),
            ),
            Err(PeerConnectionError::Stale)
        ));
    }

    #[test]
    fn revoked_hook_rejects_write_after_finite_would_block_reads() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut invalidated = false;
        let revoked = std::cell::Cell::new(false);
        let mut read_attempts = 0usize;
        let write_was_denied = std::cell::Cell::new(false);
        let result = with_nonblocking_io_until_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            deadline,
            |_, stream| {
                injected_socket_probe(stream)?;
                if revoked.get() {
                    Err(PeerConnectionError::AuthorityRejected)
                } else {
                    Ok(())
                }
            },
            |io, _| {
                let mut byte = [0u8; 1];
                for _ in 0..3 {
                    match io.read(&mut byte) {
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            read_attempts += 1;
                        }
                        other => panic!("empty socket read was not WouldBlock: {other:?}"),
                    }
                }
                revoked.set(true);
                write_was_denied.set(io.write(b"must not be sent").is_err());
            },
        );
        assert!(matches!(
            result,
            Err(PeerConnectionError::AuthorityRejected)
        ));
        assert_eq!(read_attempts, 3);
        assert!(write_was_denied.get());
        assert!(invalidated);
        peer.set_nonblocking(true).unwrap();
        let mut received = [0u8; 1];
        assert!(matches!(
            peer.read(&mut received),
            Err(ref error) if error.kind() == io::ErrorKind::WouldBlock
        ));
        assert!(matches!(
            with_nonblocking_io_until_using(
                &mut invalidated,
                &snapshot(0x1111),
                &mut stream,
                Instant::now() + Duration::from_secs(2),
                |_, _| panic!("authority-rejected adapter must not recheck"),
                |_, _| panic!("authority-rejected adapter must not write"),
            ),
            Err(PeerConnectionError::Stale)
        ));
    }

    #[test]
    fn timed_out_partial_frame_cannot_be_completed_by_the_next_request() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(b"He").unwrap();
        let deadline = Instant::now() + Duration::from_millis(150);
        let mut invalidated = false;
        let mut first_frame = [0u8; 4];
        let next_read_denied = std::cell::Cell::new(false);
        let result = with_nonblocking_io_until_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            deadline,
            |_, stream| injected_socket_probe(stream),
            |io, _| {
                let partial = io.read_exact(&mut first_frame);
                assert!(matches!(
                    partial,
                    Err(ref error) if error.kind() == io::ErrorKind::WouldBlock
                ));
                assert_eq!(&first_frame[..2], b"He");
                peer.write_all(b"cdNEXT").unwrap();
                std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
                let mut next_request = [0u8; 4];
                next_read_denied.set(io.read(&mut next_request).is_err());
            },
        );
        assert!(matches!(result, Err(PeerConnectionError::DeadlineExceeded)));
        assert!(next_read_denied.get());
        assert_eq!(&first_frame[..2], b"He");
        assert!(invalidated);
        assert_eq!(peek_first_byte(&stream).unwrap(), b'c');
        assert!(matches!(
            with_nonblocking_io_until_using(
                &mut invalidated,
                &snapshot(0x1111),
                &mut stream,
                Instant::now() + Duration::from_secs(2),
                |_, _| panic!("timed-out frame must not revalidate"),
                |_, _| panic!("timed-out frame must not consume the next request"),
            ),
            Err(PeerConnectionError::Stale)
        ));
    }

    #[test]
    fn eof_after_partial_frame_stales_connection_before_next_request() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(b"He").unwrap();
        peer.shutdown(Shutdown::Write).unwrap();
        let mut invalidated = false;
        let mut frame = [0u8; 4];
        let result = with_nonblocking_io_until_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            Instant::now() + Duration::from_secs(2),
            |_, stream| injected_socket_probe(stream),
            |io, _| io.read_exact(&mut frame),
        );
        assert!(matches!(result, Err(PeerConnectionError::PeerDisconnected)));
        assert_eq!(&frame[..2], b"He");
        assert!(invalidated);
        assert!(matches!(
            with_nonblocking_io_until_using(
                &mut invalidated,
                &snapshot(0x1111),
                &mut stream,
                Instant::now() + Duration::from_secs(2),
                |_, _| panic!("EOF after a partial frame must not revalidate"),
                |_, _| panic!("EOF after a partial frame must not start another request"),
            ),
            Err(PeerConnectionError::Stale)
        ));
    }

    #[test]
    fn independent_authority_rejection_propagates_before_eof_is_observed() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        let mut checks = 0;
        let mut invalidated = false;
        let mut callback_saw_io_error = false;
        let result = with_connection_io_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            |_, _| {
                checks += 1;
                if checks == 3 {
                    Err(PeerConnectionError::AuthorityRejected)
                } else {
                    Ok(())
                }
            },
            |io, _| {
                if io.write_all(b"req").is_err() {
                    callback_saw_io_error = true;
                }
            },
        );
        assert!(matches!(
            result,
            Err(PeerConnectionError::AuthorityRejected)
        ));
        assert!(callback_saw_io_error);
        assert!(invalidated);
        assert_eq!(checks, 4); // includes the callback-boundary post-check
        let mut request = [0u8; 3];
        peer.read_exact(&mut request).unwrap();
        assert_eq!(&request, b"req"); // completed writes cannot be rolled back
        peer.write_all(b"still-open").unwrap(); // refusal was not based on EOF
    }

    #[test]
    fn explicit_side_effect_recheck_propagates_without_io_or_eof() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        let mut checks = 0;
        let mut invalidated = false;
        let mut callback_saw_rejection = false;
        let result = with_connection_io_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            |_, _| {
                checks += 1;
                if checks == 2 {
                    Err(PeerConnectionError::AuthorityRejected)
                } else {
                    Ok(())
                }
            },
            |io, _| callback_saw_rejection = io.recheck_before_side_effect().is_err(),
        );
        assert!(matches!(
            result,
            Err(PeerConnectionError::AuthorityRejected)
        ));
        assert!(callback_saw_rejection);
        assert!(invalidated);
        assert_eq!(checks, 3);
        peer.write_all(b"still-open").unwrap();
    }

    #[test]
    fn peer_half_close_is_detected_after_queued_data_is_consumed() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(b"h").unwrap();
        peer.shutdown(Shutdown::Write).unwrap();
        let mut invalidated = false;
        let mut consumed = 0;
        let result = with_connection_io_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            |_, stream| injected_socket_probe(stream),
            |io, _| io.read_exact(std::slice::from_mut(&mut consumed)),
        );
        assert!(matches!(result, Err(PeerConnectionError::PeerDisconnected)));
        assert_eq!(consumed, b'h');
        assert!(invalidated);
    }

    #[test]
    fn peer_read_half_close_does_not_pretend_to_revoke_write_side() {
        let (mut stream, peer) = UnixStream::pair().unwrap();
        peer.shutdown(Shutdown::Read).unwrap();
        let mut invalidated = false;
        let result = with_connection_io_using(
            &mut invalidated,
            &snapshot(0x1111),
            &mut stream,
            |_, _| Ok(()),
            |io, _| io.write(b"after peer read shutdown"),
        );
        match result {
            Ok(Ok(count)) if count > 0 => assert!(!invalidated),
            Err(PeerConnectionError::IoFailure { kind, .. }) => {
                assert!(
                    kind == io::ErrorKind::BrokenPipe || kind == io::ErrorKind::ConnectionReset
                );
                assert!(invalidated);
            }
            other => panic!("unexpected peer read half-close outcome: {other:?}"),
        }
    }

    #[test]
    fn nonblocking_probe_retries_eintr_and_accepts_would_block_as_open() {
        let mut calls = 0;
        let result = probe_with(|| {
            calls += 1;
            match calls {
                1 => Err(io::Error::from_raw_os_error(libc::EINTR)),
                _ => Err(io::Error::from_raw_os_error(libc::EAGAIN)),
            }
        });
        assert!(result.is_ok());
        assert_eq!(calls, 2);
    }

    #[test]
    fn nonblocking_probe_reports_eof_and_preserves_other_errors() {
        assert!(matches!(
            probe_with(|| Ok(0)),
            Err(PeerConnectionError::PeerDisconnected)
        ));
        match probe_with(|| Err(io::Error::from_raw_os_error(libc::EBADF))) {
            Err(PeerConnectionError::ProbeRead { source }) => {
                assert_eq!(source.raw_os_error(), Some(libc::EBADF));
            }
            other => panic!("unexpected probe outcome: {other:?}"),
        }
    }

    #[test]
    fn consumer_harness_retries_injected_eintr_would_block_and_short_reads() {
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        peer.write_all(b"frame").unwrap();
        let mut invalidated = false;
        let result = with_connection_io_using(
            &mut invalidated,
            &snapshot(0x4141),
            &mut stream,
            |_, stream| injected_socket_probe(stream),
            |connection_io, credentials| -> io::Result<([u8; 5], usize, usize, usize)> {
                // Credentials are visible here only as connection metadata.
                // This transport harness performs no authorization or effect.
                assert_eq!(credentials.pid(), 42);
                let mut reader = InjectedReadFaults {
                    inner: connection_io,
                    errors: VecDeque::from([io::ErrorKind::Interrupted, io::ErrorKind::WouldBlock]),
                    max_chunk: 2,
                };
                let mut frame = [0; 5];
                let mut offset = 0;
                let mut interrupted = 0;
                let mut would_block = 0;
                let mut short_reads = 0;
                while offset < frame.len() {
                    let remaining = frame.len() - offset;
                    match reader.read(&mut frame[offset..]) {
                        Ok(0) => {
                            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                        }
                        Ok(count) => {
                            short_reads += usize::from(count < remaining);
                            offset += count;
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                            interrupted += 1;
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            would_block += 1;
                        }
                        Err(error) => return Err(error),
                    }
                }
                Ok((frame, interrupted, would_block, short_reads))
            },
        );
        let (frame, interrupted, would_block, short_reads) = result.unwrap().unwrap();
        assert_eq!(&frame, b"frame");
        assert_eq!(interrupted, 1);
        assert_eq!(would_block, 1);
        assert!(short_reads >= 2);
        assert!(!invalidated);
    }

    #[test]
    fn one_shot_transfer_keeps_the_exact_socket_and_deadline() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let fd = stream.as_raw_fd();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut cookies = HashMap::new();
        cookies.insert(fd, 0x5151);
        let connection = OwnedPeerConnection {
            stream,
            credentials: snapshot(0x5151),
            require_current_writer_credentials: false,
            invalidated: false,
        };

        let transfer = connection
            .into_stream_transfer_until_using(deadline, Instant::now, |credentials, stream| {
                injected_cookie_check(&cookies)(credentials, stream)?;
                injected_socket_probe(stream)
            })
            .unwrap();
        assert_eq!(transfer.deadline(), deadline);
        let debug = format!("{transfer:?}");
        let before_deadline = debug.split("deadline").next().unwrap();
        assert!(before_deadline.contains("private and connection-bound"));
        assert!(!before_deadline.contains("42"));
        assert!(!before_deadline.contains("1000"));

        transfer
            .consume_with_stream_using(
                Instant::now,
                |credentials, stream| {
                    injected_cookie_check(&cookies)(credentials, stream)?;
                    injected_socket_probe(stream)
                },
                |mut next_owner, callback_deadline| {
                    assert_eq!(callback_deadline, deadline);
                    next_owner.write_all(b"same-socket").unwrap();
                    let mut received = [0; 11];
                    peer.read_exact(&mut received).unwrap();
                    assert_eq!(&received, b"same-socket");
                    Ok(())
                },
            )
            .unwrap();
    }

    #[test]
    fn alias_created_before_transfer_remains_usable_after_callback_handoff() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let mut preexisting_alias = stream.try_clone().unwrap();
        let fd = stream.as_raw_fd();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut cookies = HashMap::new();
        cookies.insert(fd, 0x5252);
        let connection = OwnedPeerConnection {
            stream,
            credentials: snapshot(0x5252),
            require_current_writer_credentials: false,
            invalidated: false,
        };
        let transfer = connection
            .into_stream_transfer_until_using(deadline, Instant::now, |credentials, stream| {
                injected_cookie_check(&cookies)(credentials, stream)?;
                injected_socket_probe(stream)
            })
            .unwrap();

        transfer
            .consume_with_stream_using(
                Instant::now,
                |credentials, stream| {
                    injected_cookie_check(&cookies)(credentials, stream)?;
                    injected_socket_probe(stream)
                },
                |stream, _| {
                    drop(stream);
                    Ok(())
                },
            )
            .unwrap();

        preexisting_alias.write_all(b"preexisting-alias").unwrap();
        let mut received = [0; 17];
        peer.read_exact(&mut received).unwrap();
        assert_eq!(&received, b"preexisting-alias");
    }

    #[test]
    fn callback_can_clone_an_alias_that_outlives_transfer_consumption() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let fd = stream.as_raw_fd();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut cookies = HashMap::new();
        cookies.insert(fd, 0x5353);
        let connection = OwnedPeerConnection {
            stream,
            credentials: snapshot(0x5353),
            require_current_writer_credentials: false,
            invalidated: false,
        };
        let transfer = connection
            .into_stream_transfer_until_using(deadline, Instant::now, |credentials, stream| {
                injected_cookie_check(&cookies)(credentials, stream)?;
                injected_socket_probe(stream)
            })
            .unwrap();
        let mut retained_alias = None;

        transfer
            .consume_with_stream_using(
                Instant::now,
                |credentials, stream| {
                    injected_cookie_check(&cookies)(credentials, stream)?;
                    injected_socket_probe(stream)
                },
                |stream, _| {
                    retained_alias = Some(stream.try_clone().unwrap());
                    Ok(())
                },
            )
            .unwrap();

        retained_alias
            .take()
            .unwrap()
            .write_all(b"callback-alias")
            .unwrap();
        let mut received = [0; 14];
        peer.read_exact(&mut received).unwrap();
        assert_eq!(&received, b"callback-alias");
    }

    #[test]
    fn retained_alias_can_reenter_transport_as_a_distinct_untrusted_transfer() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let alias = stream.try_clone().unwrap();
        let alias_fd = alias.as_raw_fd();
        let fd = stream.as_raw_fd();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut cookies = HashMap::new();
        cookies.insert(fd, 0x5354);
        cookies.insert(alias_fd, 0x5354);
        let connection = OwnedPeerConnection {
            stream,
            credentials: snapshot(0x5354),
            require_current_writer_credentials: false,
            invalidated: false,
        };
        let mut nested_alias = Some(alias);
        let transfer = connection
            .into_stream_transfer_until_using(deadline, Instant::now, |credentials, stream| {
                injected_cookie_check(&cookies)(credentials, stream)?;
                injected_socket_probe(stream)
            })
            .unwrap();
        let outer_callback_entries = std::cell::Cell::new(0usize);
        let nested_callback_entries = std::cell::Cell::new(0usize);

        transfer
            .consume_with_stream_using(
                Instant::now,
                |credentials, stream| {
                    injected_cookie_check(&cookies)(credentials, stream)?;
                    injected_socket_probe(stream)
                },
                |_stream, _| {
                    outer_callback_entries.set(outer_callback_entries.get() + 1);
                    // Model a separately retained FD alias creating a new
                    // transport owner. This is a new transfer, not reuse of
                    // the consumed transfer; it carries no endpoint/session
                    // authorization or service effect.
                    let nested_owner = OwnedPeerConnection {
                        stream: nested_alias.take().ok_or(PeerConnectionError::Stale)?,
                        credentials: snapshot(0x5354),
                        require_current_writer_credentials: false,
                        invalidated: false,
                    };
                    let nested_transfer = nested_owner.into_stream_transfer_until_using(
                        deadline,
                        Instant::now,
                        |credentials, stream| {
                            injected_cookie_check(&cookies)(credentials, stream)?;
                            injected_socket_probe(stream)
                        },
                    )?;
                    nested_transfer.consume_with_stream_using(
                        Instant::now,
                        |credentials, stream| {
                            injected_cookie_check(&cookies)(credentials, stream)?;
                            injected_socket_probe(stream)
                        },
                        |_, _| {
                            nested_callback_entries.set(nested_callback_entries.get() + 1);
                            Ok(())
                        },
                    )?;
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!(outer_callback_entries.get(), 1);
        assert_eq!(nested_callback_entries.get(), 1);
        let mut byte = [0; 1];
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn panicking_transfer_callback_is_one_shot_and_drops_its_owned_stream() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let fd = stream.as_raw_fd();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut cookies = HashMap::new();
        cookies.insert(fd, 0x5454);
        let connection = OwnedPeerConnection {
            stream,
            credentials: snapshot(0x5454),
            require_current_writer_credentials: false,
            invalidated: false,
        };
        let transfer = connection
            .into_stream_transfer_until_using(deadline, Instant::now, |credentials, stream| {
                injected_cookie_check(&cookies)(credentials, stream)?;
                injected_socket_probe(stream)
            })
            .unwrap();
        let callback_entries = std::cell::Cell::new(0usize);

        let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = transfer.consume_with_stream_using(
                Instant::now,
                |credentials, stream| {
                    injected_cookie_check(&cookies)(credentials, stream)?;
                    injected_socket_probe(stream)
                },
                |_stream, _| {
                    callback_entries.set(callback_entries.get() + 1);
                    panic!("injected transfer callback panic")
                },
            );
        }));

        assert!(unwind.is_err());
        assert_eq!(callback_entries.get(), 1);
        let mut byte = [0; 1];
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn transfer_callback_error_is_returned_and_owned_stream_is_not_retried() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let fd = stream.as_raw_fd();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut cookies = HashMap::new();
        cookies.insert(fd, 0x5555);
        let connection = OwnedPeerConnection {
            stream,
            credentials: snapshot(0x5555),
            require_current_writer_credentials: false,
            invalidated: false,
        };
        let transfer = connection
            .into_stream_transfer_until_using(deadline, Instant::now, |credentials, stream| {
                injected_cookie_check(&cookies)(credentials, stream)?;
                injected_socket_probe(stream)
            })
            .unwrap();
        let callback_entries = std::cell::Cell::new(0usize);

        let result = transfer.consume_with_stream_using(
            Instant::now,
            |credentials, stream| {
                injected_cookie_check(&cookies)(credentials, stream)?;
                injected_socket_probe(stream)
            },
            |_stream, _| {
                callback_entries.set(callback_entries.get() + 1);
                Err(PeerConnectionError::AuthorityRejected)
            },
        );

        assert!(matches!(
            result,
            Err(PeerConnectionError::AuthorityRejected)
        ));
        assert_eq!(callback_entries.get(), 1);
        let mut byte = [0; 1];
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn a_different_socket_cannot_replace_the_owned_transfer() {
        let (stream, mut original_peer) = UnixStream::pair().unwrap();
        let (replacement, mut replacement_peer) = UnixStream::pair().unwrap();
        let original_fd = stream.as_raw_fd();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut cookies = HashMap::new();
        cookies.insert(original_fd, 0x6161);
        cookies.insert(replacement.as_raw_fd(), 0x6262);
        let credentials = snapshot(0x6161);

        assert!(matches!(
            injected_cookie_check(&cookies)(&credentials, &replacement),
            Err(PeerConnectionError::CredentialRead(
                PeerCredentialError::ConnectionMismatch
            ))
        ));

        let connection = OwnedPeerConnection {
            stream,
            credentials,
            require_current_writer_credentials: false,
            invalidated: false,
        };
        let transfer = connection
            .into_stream_transfer_until_using(deadline, Instant::now, |credentials, stream| {
                injected_cookie_check(&cookies)(credentials, stream)?;
                injected_socket_probe(stream)
            })
            .unwrap();
        transfer
            .consume_with_stream_using(
                Instant::now,
                |credentials, stream| {
                    injected_cookie_check(&cookies)(credentials, stream)?;
                    injected_socket_probe(stream)
                },
                |mut next_owner, _| {
                    next_owner.write_all(b"original").unwrap();
                    Ok(())
                },
            )
            .unwrap();

        let mut original = [0; 8];
        original_peer.read_exact(&mut original).unwrap();
        assert_eq!(&original, b"original");
        replacement_peer.set_nonblocking(true).unwrap();
        let mut substituted = [0; 1];
        assert!(matches!(
            replacement_peer.read(&mut substituted),
            Err(ref error) if error.kind() == io::ErrorKind::WouldBlock
        ));
    }

    #[test]
    fn peer_close_after_transfer_drops_the_stream_and_rejects_take() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let fd = stream.as_raw_fd();
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut cookies = HashMap::new();
        cookies.insert(fd, 0x7171);
        let connection = OwnedPeerConnection {
            stream,
            credentials: snapshot(0x7171),
            require_current_writer_credentials: false,
            invalidated: false,
        };
        let transfer = connection
            .into_stream_transfer_until_using(deadline, Instant::now, |credentials, stream| {
                injected_cookie_check(&cookies)(credentials, stream)?;
                injected_socket_probe(stream)
            })
            .unwrap();

        peer.shutdown(Shutdown::Write).unwrap();
        assert!(matches!(
            transfer.consume_with_stream_using(
                Instant::now,
                |credentials, stream| {
                    injected_cookie_check(&cookies)(credentials, stream)?;
                    injected_socket_probe(stream)
                },
                |_, _| panic!("disconnected peer must not receive the stream"),
            ),
            Err(PeerConnectionError::PeerDisconnected)
        ));
        let mut byte = [0; 1];
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn invalidated_owner_rejects_transfer_but_cannot_revoke_existing_alias() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let mut preexisting_alias = stream.try_clone().unwrap();
        let connection = OwnedPeerConnection {
            stream,
            credentials: snapshot(0x7272),
            require_current_writer_credentials: false,
            invalidated: true,
        };

        assert!(matches!(
            connection.into_stream_transfer_until_using(
                Instant::now() + Duration::from_secs(1),
                Instant::now,
                |_, _| panic!("invalidated connection must not validate or transfer")
            ),
            Err(PeerConnectionError::Stale)
        ));
        preexisting_alias.write_all(b"not-revoked").unwrap();
        let mut received = [0; 11];
        peer.read_exact(&mut received).unwrap();
        assert_eq!(&received, b"not-revoked");
    }

    #[test]
    fn expired_transfer_deadline_drops_the_stream_before_next_owner() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let fd = stream.as_raw_fd();
        let start = Instant::now();
        let deadline = start + Duration::from_secs(1);
        let mut cookies = HashMap::new();
        cookies.insert(fd, 0x8181);
        let connection = OwnedPeerConnection {
            stream,
            credentials: snapshot(0x8181),
            require_current_writer_credentials: false,
            invalidated: false,
        };
        let transfer = connection
            .into_stream_transfer_until_using(
                deadline,
                || start,
                |credentials, stream| {
                    injected_cookie_check(&cookies)(credentials, stream)?;
                    injected_socket_probe(stream)
                },
            )
            .unwrap();

        assert_eq!(transfer.deadline(), deadline);
        assert!(matches!(
            transfer.consume_with_stream_using(
                || deadline,
                |_, _| Ok(()),
                |_, _| panic!("expired transfer must not reach the receiver"),
            ),
            Err(PeerConnectionError::DeadlineExceeded)
        ));
        let mut byte = [0; 1];
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
    }

    #[test]
    fn deadline_crossing_during_transfer_validation_drops_the_stream() {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let fd = stream.as_raw_fd();
        let start = Instant::now();
        let deadline = start + Duration::from_millis(5);
        let clock = std::cell::Cell::new(start);
        let mut cookies = HashMap::new();
        cookies.insert(fd, 0x9191);
        let connection = OwnedPeerConnection {
            stream,
            credentials: snapshot(0x9191),
            require_current_writer_credentials: false,
            invalidated: false,
        };

        assert!(matches!(
            connection.into_stream_transfer_until_using(
                deadline,
                || clock.get(),
                |credentials, stream| {
                    injected_cookie_check(&cookies)(credentials, stream)?;
                    injected_socket_probe(stream)?;
                    clock.set(deadline);
                    Ok(())
                }
            ),
            Err(PeerConnectionError::DeadlineExceeded)
        ));
        let mut byte = [0; 1];
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
    }
}
