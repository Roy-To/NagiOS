//! Candidate-only stream framing for the canonical NIPC v1 SDK codec.
//!
//! Source provenance: NagiOS worktree commit
//! 384367f80f123859df5a57852a5c1f5693a3b703,
//! sdk/rust/docs/APP_SDK_CONTRACT.md and sdk/rust/src/app_contract/ipc.rs.
//! The 78-byte fixed header and the payload-type length bound mirror that
//! existing codec implementation.
//!
//! Candidate Linux transport choice: AF_UNIX SOCK_STREAM, which carries the
//! existing NIPC bytes without an outer prefix. The current Nagi kernel Channel
//! ABI has 128-byte inline payloads and no NIPC fragmentation contract. Any OS
//! peer credential remains separate from PrincipalId. This module only
//! delimits NIPC v1 envelopes and delegates semantic validation/encoding to
//! nagi-sdk; it does not authenticate peers, dispatch providers, define
//! cancellation, or observe service readiness.

use std::io::{self, Read, Write};

use nagi_sdk::app_contract::{
    AppError, ErrorCode, IpcEnvelope, MessageKind, IPC_PROTOCOL_VERSION, MAX_IPC_PAYLOAD_BYTES,
};

const FIXED_HEADER_BYTES: usize = 78;
const MAX_PAYLOAD_TYPE_BYTES: usize = 128;
const MAGIC: &[u8; 4] = b"NIPC";
const PAYLOAD_TYPE_LEN_OFFSET: usize = 72;
const PAYLOAD_LEN_OFFSET: usize = 74;

#[derive(Debug)]
pub enum FrameError {
    Io(io::Error),
    Contract(AppError),
}

/// A single reply correlated to the request sent by [`unix::call`]. The
/// envelope and decoded SDK error borrow `reply_storage` supplied to `call`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallReply<'a> {
    Response(IpcEnvelope<'a>),
    Error {
        envelope: IpcEnvelope<'a>,
        details: nagi_sdk::app_contract::ErrorEnvelope<'a>,
    },
}

/// A stream, framing, or correlation failure for one synchronous call.
#[derive(Debug)]
pub enum CallError {
    Frame(FrameError),
    Closed,
    InvalidRequestKind,
    MissingRequestId,
    UnexpectedReplyKind(MessageKind),
    CorrelationIdMismatch,
    RequestIdMismatch,
    InvalidErrorPayload(AppError),
    ErrorCorrelationIdMismatch,
}

impl From<io::Error> for FrameError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<AppError> for FrameError {
    fn from(error: AppError) -> Self {
        Self::Contract(error)
    }
}

/// Read one complete NIPC v1 envelope from a byte stream.
///
/// Ok(None) means EOF before any byte of the next header. A truncated header
/// or body is an I/O UnexpectedEof. The envelope codec validates all fields
/// after the bounded body is read. No outer length prefix is added.
pub fn read_frame<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>, FrameError> {
    let mut header = [0_u8; FIXED_HEADER_BYTES];
    loop {
        match reader.read(&mut header[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(FrameError::Io(error)),
        }
    }
    reader.read_exact(&mut header[1..])?;

    if &header[..4] != MAGIC {
        return Err(invalid_message());
    }
    let version = u16::from_le_bytes([header[4], header[5]]);
    if version != IPC_PROTOCOL_VERSION {
        return Err(FrameError::Contract(AppError::new(
            ErrorCode::IpcProtocolMismatch,
        )));
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
        return Err(invalid_message());
    }

    let frame_len = FIXED_HEADER_BYTES
        .checked_add(payload_type_len)
        .and_then(|length| length.checked_add(payload_len))
        .ok_or_else(invalid_message)?;
    let mut frame = vec![0_u8; frame_len];
    frame[..FIXED_HEADER_BYTES].copy_from_slice(&header);
    reader.read_exact(&mut frame[FIXED_HEADER_BYTES..])?;

    IpcEnvelope::decode(&frame)?;
    Ok(Some(frame))
}

/// Encode and write one NIPC v1 envelope, handling short writes with
/// write_all. The SDK codec remains the source of truth for field encoding.
pub fn write_frame<W: Write>(
    writer: &mut W,
    envelope: &IpcEnvelope<'_>,
) -> Result<usize, FrameError> {
    if envelope.payload_type.len() > MAX_PAYLOAD_TYPE_BYTES
        || envelope.payload.len() > MAX_IPC_PAYLOAD_BYTES
    {
        return Err(invalid_message());
    }
    let frame_len = FIXED_HEADER_BYTES
        .checked_add(envelope.payload_type.len())
        .and_then(|length| length.checked_add(envelope.payload.len()))
        .ok_or_else(invalid_message)?;
    let mut frame = vec![0_u8; frame_len];
    let encoded_len = envelope.encode(&mut frame)?;
    writer.write_all(&frame[..encoded_len])?;
    Ok(encoded_len)
}

fn invalid_message() -> FrameError {
    FrameError::Contract(AppError::new(ErrorCode::InvalidIpcMessage))
}

/// AF_UNIX stream entry points for the candidate Linux carrier. Peer
/// authentication and SVC dispatch remain outside this framing adapter.
#[cfg(unix)]
pub mod unix {
    use std::io;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;

    use nagi_sdk::app_contract::{ErrorEnvelope, IpcEnvelope, MessageKind};

    use crate::{read_frame, write_frame, CallError, CallReply, FrameError};

    pub use std::os::unix::net::{UnixListener as Listener, UnixStream as Stream};

    pub fn bind(path: impl AsRef<Path>) -> io::Result<UnixListener> {
        UnixListener::bind(path)
    }

    pub fn connect(path: impl AsRef<Path>) -> io::Result<UnixStream> {
        UnixStream::connect(path)
    }

    pub fn receive(stream: &mut UnixStream) -> Result<Option<Vec<u8>>, FrameError> {
        read_frame(stream)
    }

    pub fn send(stream: &mut UnixStream, envelope: &IpcEnvelope<'_>) -> Result<usize, FrameError> {
        write_frame(stream, envelope)
    }

    /// Send one NIPC v1 request and receive its corresponding response or
    /// ErrorEnvelope. This serial call does not multiplex requests or
    /// authenticate the peer. The caller-provided buffer owns the returned
    /// frame for as long as the borrowed SDK types are used.
    pub fn call<'a>(
        stream: &mut UnixStream,
        request: &IpcEnvelope<'_>,
        reply_storage: &'a mut Vec<u8>,
    ) -> Result<CallReply<'a>, CallError> {
        if request.kind != MessageKind::Request {
            return Err(CallError::InvalidRequestKind);
        }
        let request_id = request.request_id.ok_or(CallError::MissingRequestId)?;
        let correlation_id = request.correlation_id;

        send(stream, request).map_err(CallError::Frame)?;
        *reply_storage = receive(stream)
            .map_err(CallError::Frame)?
            .ok_or(CallError::Closed)?;

        let reply = IpcEnvelope::decode(reply_storage)
            .map_err(|error| CallError::Frame(FrameError::Contract(error)))?;
        if reply.correlation_id != correlation_id {
            return Err(CallError::CorrelationIdMismatch);
        }

        match reply.kind {
            MessageKind::Response => {
                if reply.request_id != Some(request_id) {
                    return Err(CallError::RequestIdMismatch);
                }
                Ok(CallReply::Response(reply))
            }
            MessageKind::Error => {
                if reply
                    .request_id
                    .is_some_and(|reply_id| reply_id != request_id)
                {
                    return Err(CallError::RequestIdMismatch);
                }
                let details =
                    ErrorEnvelope::decode(reply.payload).map_err(CallError::InvalidErrorPayload)?;
                if details.correlation_id != correlation_id {
                    return Err(CallError::ErrorCorrelationIdMismatch);
                }
                Ok(CallReply::Error {
                    envelope: reply,
                    details,
                })
            }
            kind => Err(CallError::UnexpectedReplyKind(kind)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Read, Write};

    use nagi_sdk::app_contract::{
        AppError, CorrelationId, ErrorCode, ErrorEnvelope, IpcEnvelope, MessageKind,
        OrderingMetadata, RequestId, IPC_PROTOCOL_VERSION,
    };
    use nagi_sdk::AppId;

    use super::{read_frame, write_frame, FrameError, FIXED_HEADER_BYTES, MAGIC};

    fn request() -> IpcEnvelope<'static> {
        IpcEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            source: AppId::from_identifier(b"com.nagi.client"),
            destination: Some(AppId::from_identifier(b"com.nagi.service")),
            kind: MessageKind::Request,
            correlation_id: CorrelationId([1; 16]),
            request_id: Some(RequestId([2; 16])),
            ordering: OrderingMetadata {
                timestamp_micros: Some(3),
                sequence: Some(4),
            },
            payload_type: "nagi.test@1",
            payload: b"request bytes",
        }
    }

    fn response() -> IpcEnvelope<'static> {
        let mut envelope = request();
        envelope.kind = MessageKind::Response;
        envelope.payload = b"response bytes";
        envelope
    }

    fn contract_error(error: FrameError) -> AppError {
        match error {
            FrameError::Contract(error) => error,
            FrameError::Io(error) => panic!("expected contract error, got I/O error: {error}"),
        }
    }

    struct ChunkedReader<'a> {
        inner: Cursor<&'a [u8]>,
        maximum_chunk: usize,
    }

    impl Read for ChunkedReader<'_> {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let limit = output.len().min(self.maximum_chunk);
            self.inner.read(&mut output[..limit])
        }
    }

    struct ChunkedWriter {
        bytes: Vec<u8>,
        maximum_chunk: usize,
    }

    impl Write for ChunkedWriter {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            let count = input.len().min(self.maximum_chunk);
            self.bytes.extend_from_slice(&input[..count]);
            Ok(count)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn stream_reader_handles_partial_io_and_concatenated_envelopes() {
        let first = request();
        let second = response();
        let mut wire = ChunkedWriter {
            bytes: Vec::new(),
            maximum_chunk: 2,
        };
        write_frame(&mut wire, &first).unwrap();
        write_frame(&mut wire, &second).unwrap();

        let mut reader = ChunkedReader {
            inner: Cursor::new(&wire.bytes),
            maximum_chunk: 1,
        };
        for expected in [&first, &second] {
            let frame = read_frame(&mut reader).unwrap().unwrap();
            let decoded = IpcEnvelope::decode(&frame).unwrap();
            assert_eq!(decoded, *expected);
        }
        assert!(read_frame(&mut reader).unwrap().is_none());
    }

    #[test]
    fn partial_header_and_body_fail_closed_as_unexpected_eof() {
        let mut partial_header = Cursor::new(&b"NIPC"[..]);
        assert!(matches!(
            read_frame(&mut partial_header),
            Err(FrameError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof
        ));

        let mut header = [0_u8; FIXED_HEADER_BYTES];
        header[..4].copy_from_slice(MAGIC);
        header[4..6].copy_from_slice(&IPC_PROTOCOL_VERSION.to_le_bytes());
        header[6] = MessageKind::Request as u8;
        header[7] = 1 << 1;
        header[72..74].copy_from_slice(&3_u16.to_le_bytes());
        header[74..78].copy_from_slice(&4_u32.to_le_bytes());
        let mut truncated_body = header.to_vec();
        truncated_body.extend_from_slice(b"a@1x");
        let mut reader = Cursor::new(truncated_body);
        assert!(matches!(
            read_frame(&mut reader),
            Err(FrameError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof
        ));
    }

    #[test]
    fn oversized_declared_payload_is_rejected_before_body_read() {
        let mut header = [0_u8; FIXED_HEADER_BYTES];
        header[..4].copy_from_slice(MAGIC);
        header[4..6].copy_from_slice(&IPC_PROTOCOL_VERSION.to_le_bytes());
        header[6] = MessageKind::Request as u8;
        header[7] = 1 << 1;
        header[72..74].copy_from_slice(&3_u16.to_le_bytes());
        header[74..78].copy_from_slice(&1_048_577_u32.to_le_bytes());

        let mut reader = Cursor::new(header);
        assert_eq!(
            contract_error(read_frame(&mut reader).unwrap_err()),
            AppError::new(ErrorCode::InvalidIpcMessage)
        );
        assert_eq!(reader.position(), FIXED_HEADER_BYTES as u64);
    }

    #[test]
    fn unknown_message_kind_is_rejected_by_the_canonical_codec() {
        let mut wire = ChunkedWriter {
            bytes: Vec::new(),
            maximum_chunk: usize::MAX,
        };
        write_frame(&mut wire, &request()).unwrap();
        wire.bytes[6] = 5;

        let mut reader = Cursor::new(wire.bytes);
        assert_eq!(
            contract_error(read_frame(&mut reader).unwrap_err()),
            AppError::new(ErrorCode::InvalidIpcMessage)
        );
    }

    #[test]
    fn response_without_request_id_is_rejected_by_the_sdk_codec() {
        let mut invalid = response();
        invalid.request_id = None;
        let error = write_frame(&mut Vec::new(), &invalid).unwrap_err();
        assert_eq!(
            contract_error(error),
            AppError::new(ErrorCode::InvalidIpcMessage)
        );
    }

    #[test]
    fn sdk_error_payload_roundtrips_inside_an_error_envelope() {
        let expected = ErrorEnvelope {
            correlation_id: CorrelationId([9; 16]),
            code: ErrorCode::PermissionDenied,
            retryable: false,
            message_key: Some("errors.permission_denied"),
        };
        let mut payload = [0_u8; 128];
        let payload_len = expected.encode(&mut payload).unwrap();
        let error_envelope = IpcEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            source: AppId::from_identifier(b"com.nagi.service"),
            destination: Some(AppId::from_identifier(b"com.nagi.client")),
            kind: MessageKind::Error,
            correlation_id: expected.correlation_id,
            request_id: Some(RequestId([2; 16])),
            ordering: OrderingMetadata {
                timestamp_micros: None,
                sequence: None,
            },
            payload_type: "nagi.error@1",
            payload: &payload[..payload_len],
        };

        let mut wire = Vec::new();
        write_frame(&mut wire, &error_envelope).unwrap();
        let decoded_frame = read_frame(&mut Cursor::new(wire)).unwrap().unwrap();
        let decoded = IpcEnvelope::decode(&decoded_frame).unwrap();
        assert_eq!(decoded.kind, MessageKind::Error);
        assert_eq!(ErrorEnvelope::decode(decoded.payload).unwrap(), expected);
    }
}
