//! Bounded first-login wire for the C2 Task24 accepted AF_UNIX stream.
//!
//! The outer frame is the canonical NIPC v1 SDK envelope: its fixed 78-byte
//! header carries the protocol version, payload-type length, and payload
//! length. The payload type is `nagi.auth.initial-login@1`; its payload is
//! `u16 version=1`, `u16 account-name byte length`, `u32 secret byte length`,
//! then the two UTF-8 fields without terminators. Source and destination IDs
//! are routing metadata only. They are not identity or authentication.
//!
//! Credentials are held in zeroizing buffers, are never formatted, and are
//! only delivered to the A-root C1 adapter after C2 revalidates the kernel
//! peer and its same-connection proof on this accepted stream.

use std::io;
use std::marker::PhantomData;
use std::net::Shutdown;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Instant;

use nagi_c2_accepted_connection_proof_contract::C2AcceptedConnectionProof;
use nagi_peer_authenticated_connection::KernelAuthenticatedUnixPeer;
use nagi_sdk::app_contract::{
    CorrelationId, IpcEnvelope, MessageKind, OrderingMetadata, RequestId, IPC_PROTOCOL_VERSION,
};
use nagi_sdk::AppId;
use zeroize::Zeroizing;

use super::{PeerConnectionError, PeerDenial};

const NIPC_FIXED_HEADER_BYTES: usize = 78;
const NIPC_PAYLOAD_TYPE_LENGTH_OFFSET: usize = 72;
const NIPC_PAYLOAD_LENGTH_OFFSET: usize = 74;
const NIPC_MAGIC: &[u8; 4] = b"NIPC";
const NIPC_REQUEST_KIND: u8 = MessageKind::Request as u8;
const LOGIN_PAYLOAD_TYPE: &str = "nagi.auth.initial-login@1";
const LOGIN_PAYLOAD_VERSION: u16 = 1;
const LOGIN_PAYLOAD_PREFIX_BYTES: usize = 8;

/// Maximum UTF-8 byte sizes accepted by the initial-login schema.
pub const MAX_LOGIN_ACCOUNT_NAME_BYTES: usize = 256;
pub const MAX_LOGIN_SECRET_BYTES: usize = 4096;

const MAX_LOGIN_PAYLOAD_BYTES: usize =
    LOGIN_PAYLOAD_PREFIX_BYTES + MAX_LOGIN_ACCOUNT_NAME_BYTES + MAX_LOGIN_SECRET_BYTES;

/// NIPC request metadata for the initial-login request.
///
/// These fields are protocol routing/correlation metadata. C2 and C1 must
/// derive authentication from the accepted kernel peer and the account
/// verifier, never from these values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InitialLoginRequestMetadata {
    pub source: AppId,
    pub destination: Option<AppId>,
    pub correlation_id: CorrelationId,
    pub request_id: RequestId,
    pub ordering: OrderingMetadata,
}

/// An initial-login request read by C2 from its exact owner-checked accepted
/// connection. This type is move-only, has no `Debug` or `Clone` impl, and
/// zeroizes account and secret storage when dropped. It does not authenticate
/// an account or authorize creation of a Session.
pub struct AcceptedInitialLoginRequest<'peer> {
    account_name: Zeroizing<String>,
    secret: Zeroizing<String>,
    _same_accepted_connection: PhantomData<(&'peer UnixStream, &'peer KernelAuthenticatedUnixPeer)>,
}

impl AcceptedInitialLoginRequest<'_> {
    /// Account-name text received in the bounded NIPC login payload.
    pub fn account_name(&self) -> &str {
        self.account_name.as_str()
    }

    /// Secret text received in the bounded NIPC login payload.
    ///
    /// Callers should pass this borrow directly to the C1 account verifier and
    /// must not copy, format, log, or persist it.
    pub fn secret(&self) -> &str {
        self.secret.as_str()
    }

    /// Password text, named for callers that use account/password terminology.
    pub fn password(&self) -> &str {
        self.secret()
    }
}

/// Error from the bounded initial-login sender API. It contains no request
/// fields or credential bytes.
#[derive(Debug)]
pub enum LoginWireError {
    Io(io::Error),
    DeadlineExpired,
    InvalidInput,
    InvalidFrame,
    InvalidPayload,
    InvalidUtf8,
    Capacity,
}

impl From<io::Error> for LoginWireError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Encode and send one NIPC v1 initial-login request on the caller's existing
/// Unix stream, handling partial writes until the absolute deadline. The
/// stream must be the client end connected to the Task24-owned initial-login
/// endpoint; this function never opens or substitutes a connection. A send
/// failure shuts down the stream so a partial credential frame is not reused.
pub fn send_initial_login_request_until(
    stream: &mut UnixStream,
    metadata: InitialLoginRequestMetadata,
    account_name: &str,
    secret: &str,
    deadline: Instant,
) -> Result<(), LoginWireError> {
    if Instant::now() >= deadline {
        return Err(LoginWireError::DeadlineExpired);
    }
    let payload = encode_login_payload(account_name, secret)?;
    let frame_len = NIPC_FIXED_HEADER_BYTES
        .checked_add(LOGIN_PAYLOAD_TYPE.len())
        .and_then(|length| length.checked_add(payload.len()))
        .ok_or(LoginWireError::InvalidInput)?;
    let mut frame = Zeroizing::new(Vec::new());
    frame
        .try_reserve_exact(frame_len)
        .map_err(|_| LoginWireError::Capacity)?;
    frame.resize(frame_len, 0);

    let encoded_len = {
        let envelope = IpcEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            source: metadata.source,
            destination: metadata.destination,
            kind: MessageKind::Request,
            correlation_id: metadata.correlation_id,
            request_id: Some(metadata.request_id),
            ordering: metadata.ordering,
            payload_type: LOGIN_PAYLOAD_TYPE,
            payload: &payload,
        };
        envelope
            .encode(frame.as_mut_slice())
            .map_err(|_| LoginWireError::InvalidFrame)?
    };
    drop(payload);

    if let Err(error) = write_all_until(stream, &frame[..encoded_len], deadline) {
        let _ = stream.shutdown(Shutdown::Both);
        return Err(error);
    }
    Ok(())
}

/// C2-only receive path. The caller reaches this only after validating the
/// retained Task24 checkout, its endpoint owner, spawn identity, and socket
/// cookie. This function additionally binds the result to a revalidated peer
/// and C2 proof on the exact borrowed stream before and after reading.
pub(crate) fn receive_from_c2_accepted_stream<'peer>(
    stream: &'peer UnixStream,
    peer: &'peer KernelAuthenticatedUnixPeer,
    proof: &C2AcceptedConnectionProof<'peer>,
    deadline: Instant,
) -> Result<AcceptedInitialLoginRequest<'peer>, PeerDenial> {
    peer.revalidate(stream)
        .map_err(|error| PeerDenial::KernelPeerAuthentication(Arc::new(error)))?;
    if !proof.matches_kernel_peer(peer) {
        return Err(PeerDenial::SessionMismatch);
    }

    let frame = read_bounded_login_frame(stream, deadline).map_err(map_receive_error)?;
    let (account_name, secret) = decode_login_frame(&frame).map_err(map_receive_error)?;
    drop(frame);

    peer.revalidate(stream)
        .map_err(|error| PeerDenial::KernelPeerAuthentication(Arc::new(error)))?;
    if !proof.matches_kernel_peer(peer) {
        return Err(PeerDenial::SessionMismatch);
    }

    Ok(AcceptedInitialLoginRequest {
        account_name,
        secret,
        _same_accepted_connection: PhantomData,
    })
}

fn encode_login_payload(
    account_name: &str,
    secret: &str,
) -> Result<Zeroizing<Vec<u8>>, LoginWireError> {
    let account = account_name.as_bytes();
    let secret = secret.as_bytes();
    if account.is_empty()
        || account.len() > MAX_LOGIN_ACCOUNT_NAME_BYTES
        || secret.len() > MAX_LOGIN_SECRET_BYTES
    {
        return Err(LoginWireError::InvalidInput);
    }
    let account_len = u16::try_from(account.len()).map_err(|_| LoginWireError::InvalidInput)?;
    let secret_len = u32::try_from(secret.len()).map_err(|_| LoginWireError::InvalidInput)?;
    let payload_len = LOGIN_PAYLOAD_PREFIX_BYTES
        .checked_add(account.len())
        .and_then(|length| length.checked_add(secret.len()))
        .ok_or(LoginWireError::InvalidInput)?;

    let mut payload = Zeroizing::new(Vec::new());
    payload
        .try_reserve_exact(payload_len)
        .map_err(|_| LoginWireError::Capacity)?;
    payload.extend_from_slice(&LOGIN_PAYLOAD_VERSION.to_le_bytes());
    payload.extend_from_slice(&account_len.to_le_bytes());
    payload.extend_from_slice(&secret_len.to_le_bytes());
    payload.extend_from_slice(account);
    payload.extend_from_slice(secret);
    Ok(payload)
}

fn read_bounded_login_frame(
    stream: &UnixStream,
    deadline: Instant,
) -> Result<Zeroizing<Vec<u8>>, LoginWireError> {
    let mut header = [0_u8; NIPC_FIXED_HEADER_BYTES];
    read_exact_until(stream, &mut header, deadline)?;
    if &header[..4] != NIPC_MAGIC
        || u16::from_le_bytes([header[4], header[5]]) != IPC_PROTOCOL_VERSION
        || header[6] != NIPC_REQUEST_KIND
    {
        return Err(LoginWireError::InvalidFrame);
    }

    let payload_type_len = usize::from(u16::from_le_bytes([
        header[NIPC_PAYLOAD_TYPE_LENGTH_OFFSET],
        header[NIPC_PAYLOAD_TYPE_LENGTH_OFFSET + 1],
    ]));
    let payload_len = usize::try_from(u32::from_le_bytes([
        header[NIPC_PAYLOAD_LENGTH_OFFSET],
        header[NIPC_PAYLOAD_LENGTH_OFFSET + 1],
        header[NIPC_PAYLOAD_LENGTH_OFFSET + 2],
        header[NIPC_PAYLOAD_LENGTH_OFFSET + 3],
    ]))
    .map_err(|_| LoginWireError::InvalidFrame)?;
    if payload_type_len != LOGIN_PAYLOAD_TYPE.len()
        || !(LOGIN_PAYLOAD_PREFIX_BYTES..=MAX_LOGIN_PAYLOAD_BYTES).contains(&payload_len)
    {
        return Err(LoginWireError::InvalidFrame);
    }

    let frame_len = NIPC_FIXED_HEADER_BYTES
        .checked_add(payload_type_len)
        .and_then(|length| length.checked_add(payload_len))
        .ok_or(LoginWireError::InvalidFrame)?;
    let mut frame = Zeroizing::new(Vec::new());
    frame
        .try_reserve_exact(frame_len)
        .map_err(|_| LoginWireError::Capacity)?;
    frame.resize(frame_len, 0);
    frame[..NIPC_FIXED_HEADER_BYTES].copy_from_slice(&header);
    read_exact_until(stream, &mut frame[NIPC_FIXED_HEADER_BYTES..], deadline)?;
    let payload_type_end = NIPC_FIXED_HEADER_BYTES + payload_type_len;
    if &frame[NIPC_FIXED_HEADER_BYTES..payload_type_end] != LOGIN_PAYLOAD_TYPE.as_bytes() {
        return Err(LoginWireError::InvalidFrame);
    }
    Ok(frame)
}

fn decode_login_frame(
    frame: &[u8],
) -> Result<(Zeroizing<String>, Zeroizing<String>), LoginWireError> {
    let envelope = IpcEnvelope::decode(frame).map_err(|_| LoginWireError::InvalidFrame)?;
    if envelope.protocol_version != IPC_PROTOCOL_VERSION
        || envelope.kind != MessageKind::Request
        || envelope.request_id.is_none()
        || envelope.payload_type != LOGIN_PAYLOAD_TYPE
    {
        return Err(LoginWireError::InvalidFrame);
    }

    let payload = envelope.payload;
    if payload.len() < LOGIN_PAYLOAD_PREFIX_BYTES
        || u16::from_le_bytes([payload[0], payload[1]]) != LOGIN_PAYLOAD_VERSION
    {
        return Err(LoginWireError::InvalidPayload);
    }
    let account_len = usize::from(u16::from_le_bytes([payload[2], payload[3]]));
    let secret_len = usize::try_from(u32::from_le_bytes([
        payload[4], payload[5], payload[6], payload[7],
    ]))
    .map_err(|_| LoginWireError::InvalidPayload)?;
    if account_len == 0
        || account_len > MAX_LOGIN_ACCOUNT_NAME_BYTES
        || secret_len > MAX_LOGIN_SECRET_BYTES
    {
        return Err(LoginWireError::InvalidPayload);
    }
    let account_end = LOGIN_PAYLOAD_PREFIX_BYTES
        .checked_add(account_len)
        .ok_or(LoginWireError::InvalidPayload)?;
    let secret_end = account_end
        .checked_add(secret_len)
        .ok_or(LoginWireError::InvalidPayload)?;
    if secret_end != payload.len() {
        return Err(LoginWireError::InvalidPayload);
    }
    let account_text = std::str::from_utf8(&payload[LOGIN_PAYLOAD_PREFIX_BYTES..account_end])
        .map_err(|_| LoginWireError::InvalidUtf8)?;
    let secret_text = std::str::from_utf8(&payload[account_end..secret_end])
        .map_err(|_| LoginWireError::InvalidUtf8)?;

    let mut account_name = Zeroizing::new(String::new());
    account_name
        .try_reserve_exact(account_text.len())
        .map_err(|_| LoginWireError::Capacity)?;
    account_name.push_str(account_text);
    let mut secret = Zeroizing::new(String::new());
    secret
        .try_reserve_exact(secret_text.len())
        .map_err(|_| LoginWireError::Capacity)?;
    secret.push_str(secret_text);
    Ok((account_name, secret))
}

fn write_all_until(
    stream: &mut UnixStream,
    bytes: &[u8],
    deadline: Instant,
) -> Result<(), LoginWireError> {
    let mut written = 0;
    while written < bytes.len() {
        wait_for(stream.as_raw_fd(), libc::POLLOUT, deadline)?;
        // SAFETY: the byte slice remains valid for this call. MSG_DONTWAIT
        // keeps the syscall within the absolute deadline after poll readiness;
        // MSG_NOSIGNAL turns a closed peer into EPIPE instead of a process signal.
        let result = unsafe {
            libc::send(
                stream.as_raw_fd(),
                bytes[written..].as_ptr().cast(),
                bytes.len() - written,
                libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
            )
        };
        if result == 0 {
            return Err(LoginWireError::Io(io::Error::new(
                io::ErrorKind::WriteZero,
                "initial-login request stream closed",
            )));
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.kind(),
                io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
            ) {
                continue;
            }
            return Err(LoginWireError::Io(error));
        }
        match usize::try_from(result) {
            Ok(count) => written += count,
            Err(_) => {
                return Err(LoginWireError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "initial-login write length is invalid",
                )))
            }
        }
    }
    Ok(())
}

fn read_exact_until(
    stream: &UnixStream,
    bytes: &mut [u8],
    deadline: Instant,
) -> Result<(), LoginWireError> {
    let mut read = 0;
    while read < bytes.len() {
        wait_for(stream.as_raw_fd(), libc::POLLIN, deadline)?;
        // SAFETY: the mutable tail is valid for this call. MSG_DONTWAIT keeps
        // the syscall bounded by the readiness poll and absolute deadline.
        let result = unsafe {
            libc::recv(
                stream.as_raw_fd(),
                bytes[read..].as_mut_ptr().cast(),
                bytes.len() - read,
                libc::MSG_DONTWAIT,
            )
        };
        if result == 0 {
            return Err(LoginWireError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "initial-login request stream closed",
            )));
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if matches!(
                error.kind(),
                io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
            ) {
                continue;
            }
            return Err(LoginWireError::Io(error));
        }
        match usize::try_from(result) {
            Ok(count) => read += count,
            Err(_) => {
                return Err(LoginWireError::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "initial-login read length is invalid",
                )))
            }
        }
    }
    Ok(())
}

fn wait_for(
    fd: std::os::fd::RawFd,
    events: libc::c_short,
    deadline: Instant,
) -> Result<(), LoginWireError> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(LoginWireError::DeadlineExpired);
        }
        let timeout_ms = remaining
            .as_millis()
            .saturating_add(1)
            .min(i32::MAX as u128) as i32;
        let mut descriptor = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        // SAFETY: `descriptor` is a valid, initialized pollfd for the duration
        // of the call, and the count is exactly one.
        let result = unsafe { libc::poll(&mut descriptor, 1, timeout_ms) };
        if result == 0 {
            continue;
        }
        if result < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(LoginWireError::Io(error));
        }
        if Instant::now() >= deadline {
            return Err(LoginWireError::DeadlineExpired);
        }
        if descriptor.revents & events != 0 {
            return Ok(());
        }
        if descriptor.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(LoginWireError::Io(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "initial-login request stream is no longer usable",
            )));
        }
    }
}

fn map_receive_error(error: LoginWireError) -> PeerDenial {
    match error {
        LoginWireError::DeadlineExpired => PeerDenial::RequestDeadlineExpired,
        LoginWireError::Capacity => PeerDenial::Capacity,
        LoginWireError::Io(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
            PeerDenial::StaleConnection
        }
        LoginWireError::Io(error) => {
            PeerDenial::PeerConnection(Arc::new(PeerConnectionError::IoFailure {
                kind: error.kind(),
                raw_os_error: error.raw_os_error(),
                message: "bounded initial-login stream I/O failed".to_owned(),
            }))
        }
        LoginWireError::InvalidInput
        | LoginWireError::InvalidFrame
        | LoginWireError::InvalidPayload
        | LoginWireError::InvalidUtf8 => PeerDenial::InvalidNipcRequest,
    }
}
