//! Minimal Linux kernel peer-credential reader for an already-connected
//! `UnixStream`.
//!
//! This module yields only the kernel-reported PID/UID/GID attached to this
//! socket connection. It does not establish a Nagi `AppId`, app session,
//! capability, consent decision, service registration, or process liveness.
//! Those must be resolved by the init-owned live launch/session/registration
//! authority before a stream can be returned by
//! `AuthenticatedUnixStreamProvider`.

use std::error::Error;
use std::fmt;
use std::io;
use std::os::unix::net::UnixStream;

mod owned_peer_connection;
pub use owned_peer_connection::{
    OwnedPeerConnection, OwnedPeerStreamTransfer, PeerConnectionError, PeerConnectionIo,
};

mod accepted_peer;
pub use accepted_peer::{
    AcceptedPeerConnection, AcceptedPeerError, AcceptedPeerListener, LiveSessionServiceDispatcher,
};

#[cfg(any(target_os = "linux", test))]
use std::{
    mem::size_of,
    os::fd::{AsRawFd, RawFd},
};

/// Kernel credentials read from one exact local Unix socket endpoint.
///
/// Fields are private to prevent callers from constructing this value from
/// NIPC payload data. PID is a point-in-time kernel value and can be reused;
/// it is not a stable process-lifetime identifier. The socket cookie binds the
/// observation to the local socket object, not to a Nagi principal or a live
/// remote process.
#[derive(Debug, Eq, PartialEq)]
pub struct KernelPeerCredentials {
    pid: libc::pid_t,
    uid: libc::uid_t,
    gid: libc::gid_t,
    local_socket_cookie: u64,
}

impl KernelPeerCredentials {
    /// Kernel-reported peer PID captured for this Unix socket connection.
    pub const fn pid(&self) -> libc::pid_t {
        self.pid
    }

    /// Kernel-reported peer UID captured for this Unix socket connection.
    pub const fn uid(&self) -> libc::uid_t {
        self.uid
    }

    /// Kernel-reported peer GID captured for this Unix socket connection.
    pub const fn gid(&self) -> libc::gid_t {
        self.gid
    }

    /// Verify that `stream` is the same local kernel socket object from which
    /// these credentials were read. This does not check whether the peer is
    /// still alive or still authorized.
    pub fn require_same_connection(&self, stream: &UnixStream) -> Result<(), PeerCredentialError> {
        #[cfg(target_os = "linux")]
        {
            require_same_connection_with(self, stream, &mut NativeSocketOptions)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = stream;
            Err(PeerCredentialError::Unavailable)
        }
    }
}

/// Errors from reading or binding kernel peer credentials.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocketOption {
    PeerCredentials,
    SocketCookie,
}

#[derive(Debug)]
pub enum PeerCredentialError {
    /// The current target has no Linux `SO_PEERCRED` implementation.
    Unavailable,
    /// The supplied stream is not connected to a peer.
    NotConnected { source: io::Error },
    /// The stream peer-address lookup failed for an OS reason other than
    /// `ENOTCONN`; the original error remains available through `source()`.
    PeerAddressRead { source: io::Error },
    /// A kernel socket-option call failed.
    KernelRead {
        option: SocketOption,
        source: io::Error,
    },
    /// The kernel returned a size or value that does not match the Linux ABI.
    MalformedKernelResponse,
    /// The socket changed during the credential read.
    ConnectionChanged,
    /// Credentials were read from a different local socket object.
    ConnectionMismatch,
}

impl fmt::Display for PeerCredentialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("Linux peer credentials unavailable"),
            Self::NotConnected { source } => {
                write!(formatter, "Unix stream is not connected: {source}")
            }
            Self::PeerAddressRead { source } => {
                write!(formatter, "Unix peer-address lookup failed: {source}")
            }
            Self::KernelRead { option, source } => {
                write!(
                    formatter,
                    "kernel socket option {option:?} failed: {source}"
                )
            }
            Self::MalformedKernelResponse => {
                formatter.write_str("malformed kernel socket-option response")
            }
            Self::ConnectionChanged => formatter.write_str("socket changed during credential read"),
            Self::ConnectionMismatch => {
                formatter.write_str("credentials belong to a different local socket")
            }
        }
    }
}

impl Error for PeerCredentialError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::NotConnected { source }
            | Self::PeerAddressRead { source }
            | Self::KernelRead { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Read Linux kernel credentials from the exact already-connected stream.
///
/// On Linux this uses `SO_PEERCRED` and binds the result to `SO_COOKIE` on the
/// same file descriptor. On other Unix targets it fails closed with
/// `Unavailable`. The returned numbers are only lookup inputs for the separate
/// live-registry stage; they are never converted here into app/session or
/// capability authority.
pub fn read_kernel_peer_credentials(
    stream: &UnixStream,
) -> Result<KernelPeerCredentials, PeerCredentialError> {
    #[cfg(target_os = "linux")]
    {
        read_with(stream, &mut NativeSocketOptions)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = stream;
        Err(PeerCredentialError::Unavailable)
    }
}

#[cfg(any(target_os = "linux", test))]
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct RawPeerCredentials {
    pid: libc::pid_t,
    uid: libc::uid_t,
    gid: libc::gid_t,
}

#[cfg(any(target_os = "linux", test))]
trait SocketOptions {
    fn peer_credentials(
        &mut self,
        fd: RawFd,
    ) -> Result<(RawPeerCredentials, usize), PeerCredentialError>;

    fn socket_cookie(&mut self, fd: RawFd) -> Result<(u64, usize), PeerCredentialError>;
}

#[cfg(any(target_os = "linux", test))]
fn classify_peer_addr<T>(result: io::Result<T>) -> Result<(), PeerCredentialError> {
    match result {
        Ok(_) => Ok(()),
        Err(source) if source.kind() == io::ErrorKind::NotConnected => {
            Err(PeerCredentialError::NotConnected { source })
        }
        Err(source) => Err(PeerCredentialError::PeerAddressRead { source }),
    }
}

#[cfg(any(target_os = "linux", test))]
fn require_connected(stream: &UnixStream) -> Result<(), PeerCredentialError> {
    classify_peer_addr(stream.peer_addr())
}

#[cfg(any(target_os = "linux", test))]
fn retry_eintr<T>(
    option: SocketOption,
    mut syscall: impl FnMut() -> io::Result<T>,
) -> Result<T, PeerCredentialError> {
    loop {
        match syscall() {
            Ok(value) => return Ok(value),
            Err(source) if source.raw_os_error() == Some(libc::EINTR) => continue,
            Err(source) => return Err(PeerCredentialError::KernelRead { option, source }),
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn read_with(
    stream: &UnixStream,
    options: &mut impl SocketOptions,
) -> Result<KernelPeerCredentials, PeerCredentialError> {
    require_connected(stream)?;
    let fd = stream.as_raw_fd();

    let (cookie_before, cookie_before_len) = options.socket_cookie(fd)?;
    if cookie_before_len != size_of::<u64>() {
        return Err(PeerCredentialError::MalformedKernelResponse);
    }

    let (peer, peer_len) = options.peer_credentials(fd)?;
    if peer_len != size_of::<RawPeerCredentials>() || peer.pid <= 0 {
        return Err(PeerCredentialError::MalformedKernelResponse);
    }

    let (cookie_after, cookie_after_len) = options.socket_cookie(fd)?;
    if cookie_after_len != size_of::<u64>() {
        return Err(PeerCredentialError::MalformedKernelResponse);
    }
    if cookie_before != cookie_after {
        return Err(PeerCredentialError::ConnectionChanged);
    }

    Ok(KernelPeerCredentials {
        pid: peer.pid,
        uid: peer.uid,
        gid: peer.gid,
        local_socket_cookie: cookie_before,
    })
}

#[cfg(any(target_os = "linux", test))]
fn require_same_connection_with(
    credentials: &KernelPeerCredentials,
    stream: &UnixStream,
    options: &mut impl SocketOptions,
) -> Result<(), PeerCredentialError> {
    require_connected(stream)?;
    let (cookie, cookie_len) = options.socket_cookie(stream.as_raw_fd())?;
    if cookie_len != size_of::<u64>() {
        return Err(PeerCredentialError::MalformedKernelResponse);
    }
    if cookie != credentials.local_socket_cookie {
        return Err(PeerCredentialError::ConnectionMismatch);
    }
    Ok(())
}

#[cfg(target_os = "linux")]
struct NativeSocketOptions;

#[cfg(target_os = "linux")]
impl SocketOptions for NativeSocketOptions {
    fn peer_credentials(
        &mut self,
        fd: RawFd,
    ) -> Result<(RawPeerCredentials, usize), PeerCredentialError> {
        retry_eintr(SocketOption::PeerCredentials, || {
            let mut raw = std::mem::MaybeUninit::<libc::ucred>::zeroed();
            let mut len = size_of::<libc::ucred>() as libc::socklen_t;
            // SAFETY: `raw` points to a writable `ucred`, `len` is initialized
            // to that structure's size, and `fd` comes from a borrowed
            // UnixStream. Each retry uses fresh zeroed storage.
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
            // SAFETY: zero initialization makes every field initialized; the
            // returned length is validated by `read_with` before use.
            let raw = unsafe { raw.assume_init() };
            Ok((
                RawPeerCredentials {
                    pid: raw.pid,
                    uid: raw.uid,
                    gid: raw.gid,
                },
                len as usize,
            ))
        })
    }

    fn socket_cookie(&mut self, fd: RawFd) -> Result<(u64, usize), PeerCredentialError> {
        retry_eintr(SocketOption::SocketCookie, || {
            let mut cookie = 0u64;
            let mut len = size_of::<u64>() as libc::socklen_t;
            // SAFETY: `cookie` and `len` are writable buffers of the ABI types
            // expected for `SO_COOKIE`; `fd` comes from a borrowed UnixStream.
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
            Ok((cookie, len as usize))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;

    #[derive(Default)]
    struct FakeSocketOptions {
        peers: HashMap<RawFd, (RawPeerCredentials, usize)>,
        cookies: HashMap<RawFd, Vec<u64>>,
        cookie_lengths: HashMap<RawFd, usize>,
        fail_peer: bool,
        fail_cookie: bool,
    }

    impl FakeSocketOptions {
        fn successful(first: &UnixStream, second: &UnixStream) -> Self {
            let peer = RawPeerCredentials {
                pid: 17,
                uid: 1000,
                gid: 1000,
            };
            let mut fake = Self::default();
            fake.peers
                .insert(first.as_raw_fd(), (peer, size_of::<RawPeerCredentials>()));
            fake.cookies.insert(first.as_raw_fd(), vec![0x1111, 0x1111]);
            fake.cookies.insert(second.as_raw_fd(), vec![0x2222]);
            fake.cookie_lengths
                .insert(first.as_raw_fd(), size_of::<u64>());
            fake.cookie_lengths
                .insert(second.as_raw_fd(), size_of::<u64>());
            fake
        }
    }

    impl SocketOptions for FakeSocketOptions {
        fn peer_credentials(
            &mut self,
            fd: RawFd,
        ) -> Result<(RawPeerCredentials, usize), PeerCredentialError> {
            if self.fail_peer {
                return Err(PeerCredentialError::KernelRead {
                    option: SocketOption::PeerCredentials,
                    source: io::Error::from_raw_os_error(libc::EIO),
                });
            }
            self.peers
                .get(&fd)
                .copied()
                .ok_or_else(|| PeerCredentialError::KernelRead {
                    option: SocketOption::PeerCredentials,
                    source: io::Error::from_raw_os_error(libc::EBADF),
                })
        }

        fn socket_cookie(&mut self, fd: RawFd) -> Result<(u64, usize), PeerCredentialError> {
            if self.fail_cookie {
                return Err(PeerCredentialError::KernelRead {
                    option: SocketOption::SocketCookie,
                    source: io::Error::from_raw_os_error(libc::EIO),
                });
            }
            let cookies =
                self.cookies
                    .get_mut(&fd)
                    .ok_or_else(|| PeerCredentialError::KernelRead {
                        option: SocketOption::SocketCookie,
                        source: io::Error::from_raw_os_error(libc::EBADF),
                    })?;
            let cookie = if cookies.len() > 1 {
                cookies.remove(0)
            } else {
                *cookies
                    .first()
                    .ok_or_else(|| PeerCredentialError::KernelRead {
                        option: SocketOption::SocketCookie,
                        source: io::Error::from_raw_os_error(libc::EIO),
                    })?
            };
            let len = self
                .cookie_lengths
                .get(&fd)
                .copied()
                .unwrap_or(size_of::<u64>());
            Ok((cookie, len))
        }
    }

    #[cfg(not(target_os = "linux"))]
    #[test]
    fn unsupported_host_is_unavailable() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        assert!(matches!(
            read_kernel_peer_credentials(&stream),
            Err(PeerCredentialError::Unavailable)
        ));
    }

    #[test]
    fn peer_address_errors_preserve_not_connected_and_other_os_errors() {
        let not_connected =
            classify_peer_addr::<()>(Err(io::Error::from_raw_os_error(libc::ENOTCONN)))
                .unwrap_err();
        match not_connected {
            PeerCredentialError::NotConnected { source } => {
                assert_eq!(source.raw_os_error(), Some(libc::ENOTCONN));
            }
            other => panic!("unexpected peer-address error: {other:?}"),
        }

        let other_error =
            classify_peer_addr::<()>(Err(io::Error::from_raw_os_error(libc::EBADF))).unwrap_err();
        match other_error {
            PeerCredentialError::PeerAddressRead { source } => {
                assert_eq!(source.raw_os_error(), Some(libc::EBADF));
            }
            other => panic!("unexpected peer-address error: {other:?}"),
        }
    }

    #[test]
    fn peer_credential_read_failure_is_closed() {
        let (stream, other) = UnixStream::pair().unwrap();
        let mut fake = FakeSocketOptions::successful(&stream, &other);
        fake.fail_peer = true;
        let error = read_with(&stream, &mut fake).unwrap_err();
        match error {
            PeerCredentialError::KernelRead { option, source } => {
                assert_eq!(option, SocketOption::PeerCredentials);
                assert_eq!(source.raw_os_error(), Some(libc::EIO));
            }
            other => panic!("unexpected peer read error: {other:?}"),
        }
    }

    #[test]
    fn socket_cookie_read_failure_is_closed() {
        let (stream, other) = UnixStream::pair().unwrap();
        let mut fake = FakeSocketOptions::successful(&stream, &other);
        fake.fail_cookie = true;
        let error = read_with(&stream, &mut fake).unwrap_err();
        match error {
            PeerCredentialError::KernelRead { option, source } => {
                assert_eq!(option, SocketOption::SocketCookie);
                assert_eq!(source.raw_os_error(), Some(libc::EIO));
            }
            other => panic!("unexpected cookie read error: {other:?}"),
        }
    }

    #[test]
    fn malformed_peer_shape_is_rejected() {
        let (stream, other) = UnixStream::pair().unwrap();
        let mut fake = FakeSocketOptions::successful(&stream, &other);
        fake.peers.get_mut(&stream.as_raw_fd()).unwrap().1 -= 1;
        assert!(matches!(
            read_with(&stream, &mut fake),
            Err(PeerCredentialError::MalformedKernelResponse)
        ));
    }

    #[test]
    fn malformed_short_socket_cookie_length_is_rejected() {
        let (stream, other) = UnixStream::pair().unwrap();
        let mut fake = FakeSocketOptions::successful(&stream, &other);
        fake.cookie_lengths
            .insert(stream.as_raw_fd(), size_of::<u64>() - 1);
        assert!(matches!(
            read_with(&stream, &mut fake),
            Err(PeerCredentialError::MalformedKernelResponse)
        ));
    }

    #[test]
    fn invalid_kernel_pid_is_rejected() {
        let (stream, other) = UnixStream::pair().unwrap();
        let mut fake = FakeSocketOptions::successful(&stream, &other);
        fake.peers.get_mut(&stream.as_raw_fd()).unwrap().0.pid = 0;
        assert!(matches!(
            read_with(&stream, &mut fake),
            Err(PeerCredentialError::MalformedKernelResponse)
        ));
    }

    #[test]
    fn credentials_are_bound_to_the_same_local_socket() {
        let (stream, other) = UnixStream::pair().unwrap();
        let mut fake = FakeSocketOptions::successful(&stream, &other);
        let credentials = read_with(&stream, &mut fake).unwrap();
        assert_eq!(credentials.pid(), 17);
        assert_eq!(credentials.uid(), 1000);
        assert_eq!(credentials.gid(), 1000);
        assert!(matches!(
            require_same_connection_with(&credentials, &other, &mut fake),
            Err(PeerCredentialError::ConnectionMismatch)
        ));
    }

    #[test]
    fn socket_replacement_during_read_is_rejected() {
        let (stream, other) = UnixStream::pair().unwrap();
        let mut fake = FakeSocketOptions::successful(&stream, &other);
        fake.cookies
            .insert(stream.as_raw_fd(), vec![0x1111, 0x3333]);
        assert!(matches!(
            read_with(&stream, &mut fake),
            Err(PeerCredentialError::ConnectionChanged)
        ));
    }

    #[test]
    fn interrupted_socket_option_retries_and_succeeds() {
        let mut calls = 0;
        let result = retry_eintr::<u32>(SocketOption::PeerCredentials, || {
            calls += 1;
            if calls == 1 {
                Err(io::Error::from_raw_os_error(libc::EINTR))
            } else {
                Ok(0x1234)
            }
        })
        .unwrap();
        assert_eq!(result, 0x1234);
        assert_eq!(calls, 2);
    }

    #[test]
    fn interrupted_socket_option_retries_then_preserves_final_os_error() {
        let mut calls = 0;
        let error = retry_eintr::<u32>(SocketOption::SocketCookie, || {
            calls += 1;
            if calls == 1 {
                Err(io::Error::from_raw_os_error(libc::EINTR))
            } else {
                Err(io::Error::from_raw_os_error(libc::EACCES))
            }
        })
        .unwrap_err();
        assert_eq!(calls, 2);
        match error {
            PeerCredentialError::KernelRead { option, source } => {
                assert_eq!(option, SocketOption::SocketCookie);
                assert_eq!(source.raw_os_error(), Some(libc::EACCES));
            }
            other => panic!("unexpected retry error: {other:?}"),
        }
    }
}
