#![cfg(unix)]

//! Kernel-backed evidence and accepted-listener ownership for Unix peers.
//!
//! `KernelAuthenticatedUnixPeer` can only be created by reading Linux
//! `SO_PEERCRED` and `SO_COOKIE` from a connected stream. Its private fields
//! prevent callers from making a peer proof from NIPC bytes or numeric IDs.
//! The cookie binds the snapshot to the local socket object; callers must
//! still bind it to their registered endpoint and live process-instance owner.
//! `C2AcceptedPeerListenerOwner` owns the accepted listener and issues a
//! borrowed `AuthenticatedInteractivePeer` only for a stream it accepted.
//! Its public registration entrypoint is not itself proof of canonical C2
//! ownership; C2 must compare the proof's opaque owner key with its exact
//! spawn-validated endpoint record before forwarding it to C1.
//! The leaf does not create an AppId, PrincipalId, Session, capability,
//! consent decision, or service authorization; C1 must still apply its own
//! trusted account/profile mapping and live Session policy.

#![deny(unsafe_op_in_unsafe_fn)]

use std::fmt;
use std::num::NonZeroU32;
use std::os::fd::BorrowedFd;
use std::os::unix::net::UnixStream;

#[cfg(target_os = "linux")]
use std::io;
#[cfg(target_os = "linux")]
use std::mem::size_of;
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd, RawFd};

/// Which kernel socket option failed while making or rechecking a proof.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerOption {
    PeerCredentials,
    SocketCookie,
    SocketDomain,
    SocketType,
    SocketState,
    AcceptedListener,
}

/// A typed failure while reading kernel peer identity from a Unix stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeerAuthenticationError {
    /// This target does not implement the Linux peer-credential contract.
    Unavailable,
    /// The supplied stream has no connected peer.
    NotConnected,
    /// A kernel socket option failed.
    KernelRead {
        option: PeerOption,
        raw_os_error: Option<i32>,
    },
    /// The kernel returned a response with an unexpected ABI length or value.
    MalformedKernelResponse,
    /// The local socket cookie changed while credentials were being read.
    ConnectionChanged,
    /// The descriptor does not refer to an AF_UNIX stream socket.
    NotUnixStream,
    /// The proof belongs to a different local socket object.
    ConnectionMismatch,
    /// The registered accepted-listener owner has been revoked.
    OwnerRevoked,
    /// The registered accepted-listener owner or operation lease expired.
    OwnerExpired,
    /// The accepted-listener owner could not be revalidated.
    OwnerUnavailable,
    /// No connection was accepted before the absolute deadline.
    AcceptDeadlineExpired,
}

impl fmt::Display for PeerAuthenticationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("Linux peer authentication unavailable"),
            Self::NotConnected => formatter.write_str("Unix stream is not connected"),
            Self::KernelRead {
                option,
                raw_os_error,
            } => write!(
                formatter,
                "kernel peer option {option:?} failed (errno={raw_os_error:?})"
            ),
            Self::MalformedKernelResponse => {
                formatter.write_str("malformed kernel peer authentication response")
            }
            Self::ConnectionChanged => {
                formatter.write_str("socket changed during peer authentication")
            }
            Self::NotUnixStream => {
                formatter.write_str("descriptor is not an AF_UNIX stream socket")
            }
            Self::ConnectionMismatch => formatter.write_str("peer proof belongs to another socket"),
            Self::OwnerRevoked => formatter.write_str("accepted peer owner was revoked"),
            Self::OwnerExpired => formatter.write_str("accepted peer owner lease expired"),
            Self::OwnerUnavailable => formatter.write_str("accepted peer owner unavailable"),
            Self::AcceptDeadlineExpired => formatter.write_str("accepted peer deadline expired"),
        }
    }
}

impl std::error::Error for PeerAuthenticationError {}

/// Opaque local key for correlating a C1 revalidation handle with one
/// connection proof. It is derived from the kernel socket cookie and is a map
/// selector only; it is not identity, liveness, or authorization evidence.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PeerConnectionId(u64);

/// A non-forgeable snapshot of the Linux peer credentials on one local socket.
///
/// This value deliberately contains only kernel-reported process credentials
/// and a private local socket cookie. It is not a stable process-lifetime
/// proof: PID reuse, process liveness, app identity, endpoint ownership, and
/// current-writer status require their separate owners. The type is
/// non-cloneable so a runtime keeps one explicit proof per accepted connection.
///
/// ```compile_fail
/// use nagi_peer_authenticated_connection::KernelAuthenticatedUnixPeer;
/// use std::num::NonZeroU32;
/// let forged = KernelAuthenticatedUnixPeer {
///     pid: NonZeroU32::new(42).unwrap(),
///     uid: 1000,
///     gid: 1000,
///     socket_cookie: 17,
/// };
/// ```
pub struct KernelAuthenticatedUnixPeer {
    pid: NonZeroU32,
    uid: u32,
    gid: u32,
    socket_cookie: u64,
}

impl fmt::Debug for KernelAuthenticatedUnixPeer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KernelAuthenticatedUnixPeer")
            .field("pid", &self.pid)
            .field("uid", &self.uid)
            .field("gid", &self.gid)
            .field("socket_bound", &true)
            .finish()
    }
}

impl KernelAuthenticatedUnixPeer {
    /// Read credentials from this exact connected stream and bind them to its
    /// kernel socket cookie. Unsupported targets fail closed.
    pub fn authenticate(stream: &UnixStream) -> Result<Self, PeerAuthenticationError> {
        #[cfg(target_os = "linux")]
        {
            authenticate_linux(stream)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = stream;
            Err(PeerAuthenticationError::Unavailable)
        }
    }

    /// Kernel-reported peer PID. This value can be reused after process exit.
    pub const fn pid(&self) -> u32 {
        self.pid.get()
    }

    /// Kernel-reported peer UID. This value is not a Nagi principal.
    pub const fn uid(&self) -> u32 {
        self.uid
    }

    /// Kernel-reported peer GID. This value is not a Nagi capability.
    pub const fn gid(&self) -> u32 {
        self.gid
    }

    /// Return the opaque key derived from this proof's local socket cookie.
    /// Use it only to find state already created from this proof.
    pub const fn connection_id(&self) -> PeerConnectionId {
        PeerConnectionId(self.socket_cookie)
    }

    /// Check that `stream` is still the same local kernel socket object.
    /// This does not establish peer liveness or current authorization.
    pub fn require_same_connection(
        &self,
        stream: &UnixStream,
    ) -> Result<(), PeerAuthenticationError> {
        #[cfg(target_os = "linux")]
        {
            require_connected(stream)?;
            let cookie = read_socket_cookie(stream.as_raw_fd())?;
            if cookie != self.socket_cookie {
                return Err(PeerAuthenticationError::ConnectionMismatch);
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = stream;
            Err(PeerAuthenticationError::Unavailable)
        }
    }

    /// Revalidate a borrowed descriptor against this authenticated connection.
    ///
    /// The comparison uses the kernel socket cookie, not the process-local FD
    /// number, so a `dup` of the same socket is accepted and another socket
    /// with identical peer credentials is rejected. Peer credentials and
    /// socket liveness are re-read on Linux; unsupported targets fail closed.
    pub fn require_same_connection_fd(
        &self,
        endpoint: BorrowedFd<'_>,
    ) -> Result<(), PeerAuthenticationError> {
        #[cfg(target_os = "linux")]
        {
            let current = authenticate_linux_fd(endpoint.as_raw_fd())?;
            if current.pid != self.pid
                || current.uid != self.uid
                || current.gid != self.gid
                || current.socket_cookie != self.socket_cookie
            {
                return Err(PeerAuthenticationError::ConnectionMismatch);
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = endpoint;
            Err(PeerAuthenticationError::Unavailable)
        }
    }

    /// Re-read peer credentials and the socket cookie, rejecting a different
    /// peer snapshot or socket. A separate process-instance owner must still
    /// establish liveness and match the PID to the exact spawned child.
    pub fn revalidate(&self, stream: &UnixStream) -> Result<(), PeerAuthenticationError> {
        #[cfg(target_os = "linux")]
        {
            let current = authenticate_linux(stream)?;
            if current.pid != self.pid
                || current.uid != self.uid
                || current.gid != self.gid
                || current.socket_cookie != self.socket_cookie
            {
                return Err(PeerAuthenticationError::ConnectionMismatch);
            }
            Ok(())
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = stream;
            Err(PeerAuthenticationError::Unavailable)
        }
    }
}

mod interactive_peer_owner;
pub use interactive_peer_owner::{
    AcceptedInteractivePeerConnection, AcceptedInteractivePeerHandle, AuthenticatedInteractivePeer,
    AuthenticatedInteractivePeerSelector, C2AcceptedPeerListenerOwner, TrustedInteractivePeerKey,
};

#[cfg(target_os = "linux")]
#[repr(C)]
#[derive(Clone, Copy)]
struct RawPeerCredentials {
    pid: libc::pid_t,
    uid: libc::uid_t,
    gid: libc::gid_t,
}

#[cfg(target_os = "linux")]
fn authenticate_linux(
    stream: &UnixStream,
) -> Result<KernelAuthenticatedUnixPeer, PeerAuthenticationError> {
    require_connected(stream)?;
    authenticate_linux_fd(stream.as_raw_fd())
}

#[cfg(target_os = "linux")]
fn authenticate_linux_fd(
    fd: RawFd,
) -> Result<KernelAuthenticatedUnixPeer, PeerAuthenticationError> {
    require_live_unix_stream(fd)?;
    let cookie_before = read_socket_cookie(fd)?;
    let credentials = read_peer_credentials(fd)?;
    let cookie_after = read_socket_cookie(fd)?;
    if cookie_before != cookie_after {
        return Err(PeerAuthenticationError::ConnectionChanged);
    }
    let pid = u32::try_from(credentials.pid)
        .ok()
        .and_then(NonZeroU32::new)
        .ok_or(PeerAuthenticationError::MalformedKernelResponse)?;
    Ok(KernelAuthenticatedUnixPeer {
        pid,
        uid: credentials.uid,
        gid: credentials.gid,
        socket_cookie: cookie_before,
    })
}

#[cfg(target_os = "linux")]
fn require_live_unix_stream(fd: RawFd) -> Result<(), PeerAuthenticationError> {
    let domain = read_socket_option_i32(fd, libc::SO_DOMAIN, PeerOption::SocketDomain)
        .map_err(map_non_socket_error)?;
    let socket_type = read_socket_option_i32(fd, libc::SO_TYPE, PeerOption::SocketType)
        .map_err(map_non_socket_error)?;
    if domain != libc::AF_UNIX || socket_type != libc::SOCK_STREAM {
        return Err(PeerAuthenticationError::NotUnixStream);
    }

    let mut descriptor = libc::pollfd {
        fd,
        events: libc::POLLRDHUP,
        revents: 0,
    };
    let result = retry_eintr(PeerOption::SocketState, || {
        // SAFETY: `descriptor` is a live, initialized pollfd and `fd` is only
        // used as a syscall input. A closed or reused descriptor fails closed
        // in the socket-option checks before reaching this poll.
        let result = unsafe { libc::poll(&mut descriptor, 1, 0) };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(result)
        }
    })?;
    if result < 0
        || descriptor.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL | libc::POLLRDHUP)
            != 0
    {
        return Err(PeerAuthenticationError::NotConnected);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn map_non_socket_error(error: PeerAuthenticationError) -> PeerAuthenticationError {
    match error {
        PeerAuthenticationError::KernelRead {
            raw_os_error: Some(libc::ENOTSOCK),
            ..
        } => PeerAuthenticationError::NotUnixStream,
        other => other,
    }
}

#[cfg(target_os = "linux")]
fn read_socket_option_i32(
    fd: RawFd,
    option_name: libc::c_int,
    option: PeerOption,
) -> Result<libc::c_int, PeerAuthenticationError> {
    retry_eintr(option, || {
        let mut value = 0 as libc::c_int;
        let mut len = size_of::<libc::c_int>() as libc::socklen_t;
        // SAFETY: `value` and `len` are writable ABI buffers; `fd` is checked
        // by the kernel and invalid descriptors return an error.
        let result = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                option_name,
                (&mut value as *mut libc::c_int).cast(),
                &mut len,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        if len as usize != size_of::<libc::c_int>() {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        Ok(value)
    })
}

#[cfg(target_os = "linux")]
fn require_connected(stream: &UnixStream) -> Result<(), PeerAuthenticationError> {
    stream.peer_addr().map(|_| ()).map_err(|error| {
        if error.kind() == io::ErrorKind::NotConnected {
            PeerAuthenticationError::NotConnected
        } else {
            PeerAuthenticationError::KernelRead {
                option: PeerOption::PeerCredentials,
                raw_os_error: error.raw_os_error(),
            }
        }
    })
}

#[cfg(target_os = "linux")]
fn read_peer_credentials(fd: libc::c_int) -> Result<RawPeerCredentials, PeerAuthenticationError> {
    retry_eintr(PeerOption::PeerCredentials, || {
        let mut raw = std::mem::MaybeUninit::<libc::ucred>::zeroed();
        let mut len = size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: `raw` is writable storage for `ucred`; `fd` comes from the
        // live borrowed UnixStream and `len` is initialized to the ABI size.
        let result = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                raw.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        if len as usize != size_of::<libc::ucred>() {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        // SAFETY: zeroed storage is initialized, and successful getsockopt
        // wrote the complete structure checked above.
        let raw = unsafe { raw.assume_init() };
        Ok(RawPeerCredentials {
            pid: raw.pid,
            uid: raw.uid,
            gid: raw.gid,
        })
    })
}

#[cfg(target_os = "linux")]
fn read_socket_cookie(fd: libc::c_int) -> Result<u64, PeerAuthenticationError> {
    retry_eintr(PeerOption::SocketCookie, || {
        let mut cookie = 0_u64;
        let mut len = size_of::<u64>() as libc::socklen_t;
        // SAFETY: `cookie` and `len` are writable ABI buffers; `fd` comes
        // from the live borrowed UnixStream.
        let result = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_COOKIE,
                (&mut cookie as *mut u64).cast(),
                &mut len,
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        if len as usize != size_of::<u64>() {
            return Err(io::Error::from_raw_os_error(libc::EINVAL));
        }
        Ok(cookie)
    })
}

#[cfg(target_os = "linux")]
fn retry_eintr<T>(
    option: PeerOption,
    mut syscall: impl FnMut() -> io::Result<T>,
) -> Result<T, PeerAuthenticationError> {
    loop {
        match syscall() {
            Ok(value) => return Ok(value),
            Err(error) if error.raw_os_error() == Some(libc::EINTR) => continue,
            Err(error) => {
                return Err(PeerAuthenticationError::KernelRead {
                    option,
                    raw_os_error: error.raw_os_error(),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[cfg(target_os = "linux")]
    static FD_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[cfg(target_os = "linux")]
    #[test]
    fn proof_matches_only_the_socket_whose_kernel_credentials_were_read() {
        let _fd_lock = FD_TEST_LOCK.lock().unwrap();
        use std::os::fd::AsFd;

        let (stream, peer) = UnixStream::pair().unwrap();
        let (other, _other_peer) = UnixStream::pair().unwrap();
        let proof = KernelAuthenticatedUnixPeer::authenticate(&stream).unwrap();
        let other_proof = KernelAuthenticatedUnixPeer::authenticate(&other).unwrap();
        assert_eq!(proof.pid(), std::process::id());
        assert_eq!(proof.pid(), other_proof.pid());
        assert_eq!(proof.uid(), other_proof.uid());
        assert_eq!(proof.gid(), other_proof.gid());
        assert_ne!(proof.connection_id(), other_proof.connection_id());
        proof.require_same_connection(&stream).unwrap();
        proof.revalidate(&stream).unwrap();
        assert_eq!(
            proof.require_same_connection(&other),
            Err(PeerAuthenticationError::ConnectionMismatch)
        );
        assert_eq!(
            proof.revalidate(&other),
            Err(PeerAuthenticationError::ConnectionMismatch)
        );
        assert_eq!(
            proof.require_same_connection_fd(other.as_fd()),
            Err(PeerAuthenticationError::ConnectionMismatch)
        );
        drop(peer);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn borrowed_fd_comparison_accepts_dup_of_same_socket() {
        let _fd_lock = FD_TEST_LOCK.lock().unwrap();
        use std::os::fd::AsFd;

        let (stream, _peer) = UnixStream::pair().unwrap();
        let proof = KernelAuthenticatedUnixPeer::authenticate(&stream).unwrap();
        let duplicate = stream.as_fd().try_clone_to_owned().unwrap();
        proof.require_same_connection_fd(duplicate.as_fd()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn borrowed_fd_comparison_rejects_closed_peer_and_non_socket() {
        let _fd_lock = FD_TEST_LOCK.lock().unwrap();
        use std::os::fd::{AsFd, FromRawFd, OwnedFd};

        let (stream, peer) = UnixStream::pair().unwrap();
        let proof = KernelAuthenticatedUnixPeer::authenticate(&stream).unwrap();
        drop(peer);
        assert_eq!(
            proof.require_same_connection_fd(stream.as_fd()),
            Err(PeerAuthenticationError::NotConnected)
        );

        let mut pipe_fds = [-1; 2];
        // SAFETY: `pipe_fds` is writable storage for the two descriptors returned
        // by pipe2. Successful descriptors are immediately wrapped as OwnedFd.
        let pipe_result = unsafe { libc::pipe2(pipe_fds.as_mut_ptr(), libc::O_CLOEXEC) };
        assert_eq!(pipe_result, 0, "pipe2: {}", std::io::Error::last_os_error());
        // SAFETY: successful pipe2 returned two new descriptors owned by this test.
        let pipe_read = unsafe { OwnedFd::from_raw_fd(pipe_fds[0]) };
        // SAFETY: successful pipe2 returned two new descriptors owned by this test.
        let _pipe_write = unsafe { OwnedFd::from_raw_fd(pipe_fds[1]) };
        assert_eq!(
            proof.require_same_connection_fd(pipe_read.as_fd()),
            Err(PeerAuthenticationError::NotUnixStream)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn borrowed_fd_comparison_rejects_reused_number_for_same_credential_peer() {
        let _fd_lock = FD_TEST_LOCK.lock().unwrap();
        use std::os::fd::{AsFd, AsRawFd, FromRawFd, IntoRawFd};

        let (original, original_peer) = UnixStream::pair().unwrap();
        let proof = KernelAuthenticatedUnixPeer::authenticate(&original).unwrap();
        let original_fd = original.into_raw_fd();
        let (replacement_source, _replacement_peer) = UnixStream::pair().unwrap();
        assert_ne!(original_fd, replacement_source.as_raw_fd());

        // SAFETY: `original_fd` is an open descriptor relinquished from `original`;
        // `replacement_source` is a distinct live UnixStream. `dup2` atomically
        // replaces that descriptor number with a duplicate of the other socket.
        let duplicated = unsafe { libc::dup2(replacement_source.as_raw_fd(), original_fd) };
        if duplicated < 0 {
            // SAFETY: ownership of `original_fd` was transferred with into_raw_fd.
            unsafe { libc::close(original_fd) };
            panic!(
                "dup2 for stale-number fixture failed: {}",
                std::io::Error::last_os_error()
            );
        }
        assert_eq!(duplicated, original_fd);

        // SAFETY: dup2 installed a live duplicate at `original_fd`; this wrapper
        // takes its unique ownership for the rest of the test.
        let replacement = unsafe { UnixStream::from_raw_fd(original_fd) };
        let replacement_proof = KernelAuthenticatedUnixPeer::authenticate(&replacement).unwrap();
        assert_eq!(proof.pid(), std::process::id());
        assert_eq!(proof.pid(), replacement_proof.pid());
        assert_eq!(proof.uid(), replacement_proof.uid());
        assert_eq!(proof.gid(), replacement_proof.gid());
        assert_eq!(
            proof.require_same_connection_fd(replacement.as_fd()),
            Err(PeerAuthenticationError::ConnectionMismatch)
        );
        drop(original_peer);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn raw_fd_probe_rejects_closed_descriptor() {
        let _fd_lock = FD_TEST_LOCK.lock().unwrap();
        use std::os::fd::IntoRawFd;

        let (stream, _peer) = UnixStream::pair().unwrap();
        let closed_fd = stream.into_raw_fd();
        // SAFETY: `into_raw_fd` transfers unique ownership to this test. Close
        // it here so the private syscall helper can verify EBADF fail-closed.
        assert_eq!(unsafe { libc::close(closed_fd) }, 0);
        assert!(matches!(
            authenticate_linux_fd(closed_fd),
            Err(PeerAuthenticationError::KernelRead {
                option: PeerOption::SocketDomain,
                raw_os_error: Some(libc::EBADF),
            })
        ));
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn unsupported_targets_fail_closed() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        assert_eq!(
            KernelAuthenticatedUnixPeer::authenticate(&stream).unwrap_err(),
            PeerAuthenticationError::Unavailable
        );
    }
}
