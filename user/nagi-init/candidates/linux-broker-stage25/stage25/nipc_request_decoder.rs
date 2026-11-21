//! Incremental decoder for the canonical NIPC v1 SDK envelope.
//!
//! Nagi's existing stream helper uses `read_exact`; this decoder instead keeps
//! one bounded frame buffer, one fixed deadline, and the same byte offset over
//! `Interrupted` and `WouldBlock`. It accepts bytes only through `AuthorizedPeer::read_nipc_request`
//! and does not turn decoded identity, route, payload, or capability fields into
//! authorization.

use super::{
    CompleteServiceRequestFrame, PeerDenial, PendingServiceRequestFrame, ServiceRequestFingerprint,
    ServiceRequestFrameProgress, StableServiceRequestId,
};
#[cfg(test)]
use nagi_sdk::app_contract::{CorrelationId, OrderingMetadata};
use nagi_sdk::app_contract::{
    IpcEnvelope, MessageKind, RequestId, IPC_PROTOCOL_VERSION, MAX_IPC_PAYLOAD_BYTES,
};
use std::io::{self, ErrorKind, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const FIXED_HEADER_BYTES: usize = 78;
const MAX_PAYLOAD_TYPE_BYTES: usize = 128;
const MAX_FRAME_BYTES: usize = FIXED_HEADER_BYTES + MAX_PAYLOAD_TYPE_BYTES + MAX_IPC_PAYLOAD_BYTES;
const MAGIC: &[u8; 4] = b"NIPC";
const REQUEST_ID_OFFSET: usize = 40;
const FLAGS_OFFSET: usize = 7;
const PAYLOAD_TYPE_LEN_OFFSET: usize = 72;
const PAYLOAD_LEN_OFFSET: usize = 74;
const REQUEST_ID_FLAG: u8 = 1 << 1;
const KNOWN_FLAGS: u8 = 0x0f;

/// One decoded request frame. Its fields come from the SDK codec, while the
/// source AppId is checked against the current registered launch record. The
/// payload remains untrusted input for a trusted route/operation mapper.
#[derive(Debug)]
pub struct DecodedNipcRequest {
    frame: CompleteServiceRequestFrame,
    wire_request_id: StableServiceRequestId,
    fingerprint: ServiceRequestFingerprint,
    source_app_id: u64,
    destination_app_id: Option<u64>,
    correlation_id: [u8; 16],
    payload_type: String,
    payload: Vec<u8>,
    canonical_frame: Arc<[u8]>,
}

impl DecodedNipcRequest {
    pub const fn wire_request_id(&self) -> StableServiceRequestId {
        self.wire_request_id
    }

    pub(super) const fn fingerprint(&self) -> ServiceRequestFingerprint {
        self.fingerprint
    }

    pub const fn source_app_id(&self) -> u64 {
        self.source_app_id
    }

    pub const fn destination_app_id(&self) -> Option<u64> {
        self.destination_app_id
    }

    pub const fn correlation_id(&self) -> [u8; 16] {
        self.correlation_id
    }

    pub fn payload_type(&self) -> &str {
        &self.payload_type
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    pub(crate) fn into_complete_frame(self) -> CompleteServiceRequestFrame {
        let mut frame = self.frame;
        frame.fingerprint = Some(self.fingerprint);
        frame.canonical_bytes = Some(self.canonical_frame);
        frame
    }
}

/// Result from one nonblocking advance of an authorized connection's frame.
/// `Pending` means the caller should yield and retry with the same decoder;
/// its request identity, offset, and absolute deadline remain fixed.
#[derive(Debug)]
pub enum NipcRequestRead {
    Pending {
        received_bytes: usize,
        expected_bytes: Option<usize>,
    },
    Complete(DecodedNipcRequest),
}

/// Single-connection, single-request incremental NIPC decoder. Only the
/// owning `AuthorizedPeer` can create it and call `read_step`.
#[derive(Debug)]
pub struct NipcRequestDecoder {
    pub(super) request_frame: Arc<Mutex<Option<ServiceRequestFrameProgress>>>,
    pub(super) connection_stale: Arc<AtomicBool>,
    expected_source_app_id: u64,
    deadline: Instant,
    bytes: Vec<u8>,
    expected_bytes: Option<usize>,
    wire_request_id: Option<StableServiceRequestId>,
    pending: Option<PendingServiceRequestFrame>,
    terminal: bool,
}

impl NipcRequestDecoder {
    pub(super) fn for_authorized_peer(
        request_frame: Arc<Mutex<Option<ServiceRequestFrameProgress>>>,
        connection_stale: Arc<AtomicBool>,
        expected_source_app_id: u64,
        deadline: Instant,
    ) -> Self {
        Self {
            request_frame,
            connection_stale,
            expected_source_app_id,
            deadline,
            bytes: Vec::with_capacity(FIXED_HEADER_BYTES),
            expected_bytes: None,
            wire_request_id: None,
            pending: None,
            terminal: false,
        }
    }

    pub(super) fn fail_connection(&mut self) {
        self.terminal = true;
        self.connection_stale.store(true, Ordering::SeqCst);
    }

    pub(super) fn read_step<R: Read>(
        &mut self,
        reader: &mut R,
    ) -> Result<NipcRequestRead, PeerDenial> {
        if self.terminal || self.connection_stale.load(Ordering::SeqCst) {
            return Err(PeerDenial::StaleConnection);
        }

        loop {
            if Instant::now() >= self.deadline {
                self.fail_connection();
                if let Some(pending) = self.pending.as_mut() {
                    let _ = pending.record_read_result(Err(io::Error::new(
                        ErrorKind::TimedOut,
                        "NIPC request deadline expired",
                    )));
                }
                return Err(PeerDenial::RequestDeadlineExpired);
            }

            if self.expected_bytes.is_none() && self.bytes.len() == FIXED_HEADER_BYTES {
                self.bind_header()?;
            }

            if let Some(expected) = self.expected_bytes {
                if self.bytes.len() == expected {
                    return self.finish_frame();
                }
            }

            let remaining = match self.expected_bytes {
                Some(expected) => expected.saturating_sub(self.bytes.len()),
                None => FIXED_HEADER_BYTES.saturating_sub(self.bytes.len()),
            };
            if remaining == 0 {
                self.fail_connection();
                return Err(PeerDenial::InvalidNipcRequest);
            }

            let mut chunk = [0_u8; 4096];
            let read_capacity = remaining.min(chunk.len());
            match reader.read(&mut chunk[..read_capacity]) {
                Ok(0) => {
                    self.fail_connection();
                    return Err(PeerDenial::StaleConnection);
                }
                Ok(count) => {
                    self.bytes.extend_from_slice(&chunk[..count]);
                    if self.expected_bytes.is_some() {
                        let pending = self.pending.as_mut().ok_or(PeerDenial::Unavailable)?;
                        pending.record_read_result(Ok(count))?;
                    }
                }
                Err(error)
                    if matches!(error.kind(), ErrorKind::Interrupted | ErrorKind::WouldBlock) =>
                {
                    let received_bytes = self
                        .pending
                        .as_ref()
                        .map(PendingServiceRequestFrame::received_bytes)
                        .transpose()?
                        .unwrap_or(self.bytes.len());
                    return Ok(NipcRequestRead::Pending {
                        received_bytes,
                        expected_bytes: self.expected_bytes,
                    });
                }
                Err(error) => {
                    self.fail_connection();
                    return Err(PeerDenial::PeerConnection(Arc::new(
                        linux_peer_credentials_supplier::PeerConnectionError::IoFailure {
                            kind: error.kind(),
                            raw_os_error: error.raw_os_error(),
                            message: error.to_string(),
                        },
                    )));
                }
            }
        }
    }

    fn bind_header(&mut self) -> Result<(), PeerDenial> {
        let header: [u8; FIXED_HEADER_BYTES] = self.bytes[..FIXED_HEADER_BYTES]
            .try_into()
            .map_err(|_| PeerDenial::InvalidNipcRequest)?;
        if &header[..4] != MAGIC
            || u16::from_le_bytes([header[4], header[5]]) != IPC_PROTOCOL_VERSION
            || header[6] != MessageKind::Request as u8
            || header[FLAGS_OFFSET] & REQUEST_ID_FLAG == 0
            || header[FLAGS_OFFSET] & !KNOWN_FLAGS != 0
        {
            self.fail_connection();
            return Err(PeerDenial::InvalidNipcRequest);
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
        if payload_type_len == 0 || payload_type_len > MAX_PAYLOAD_TYPE_BYTES {
            self.fail_connection();
            return Err(PeerDenial::InvalidNipcRequest);
        }
        if payload_len > MAX_IPC_PAYLOAD_BYTES {
            self.fail_connection();
            return Err(PeerDenial::InvalidNipcRequest);
        }
        let expected_bytes = FIXED_HEADER_BYTES
            .checked_add(payload_type_len)
            .and_then(|value| value.checked_add(payload_len))
            .filter(|value| *value <= MAX_FRAME_BYTES)
            .ok_or_else(|| {
                self.fail_connection();
                PeerDenial::InvalidNipcRequest
            })?;
        let mut id = [0_u8; 16];
        id.copy_from_slice(&header[REQUEST_ID_OFFSET..REQUEST_ID_OFFSET + 16]);
        let wire_request_id = StableServiceRequestId::from_protocol_decoder(id);
        let mut pending = self.start_pending_frame(wire_request_id, expected_bytes)?;
        pending.record_read_result(Ok(FIXED_HEADER_BYTES))?;
        self.pending = Some(pending);
        self.expected_bytes = Some(expected_bytes);
        self.wire_request_id = Some(wire_request_id);
        Ok(())
    }

    fn start_pending_frame(
        &mut self,
        wire_request_id: StableServiceRequestId,
        expected_bytes: usize,
    ) -> Result<PendingServiceRequestFrame, PeerDenial> {
        let mut progress = self
            .request_frame
            .try_lock()
            .map_err(|_| PeerDenial::Unavailable)?;
        let can_start = match progress.as_ref() {
            None => {
                *progress = Some(ServiceRequestFrameProgress {
                    wire_request_id,
                    expected_bytes,
                    received_bytes: 0,
                    authority_io_deadline: None,
                });
                true
            }
            Some(_) => false,
        };
        drop(progress);
        if !can_start {
            self.fail_connection();
            return Err(PeerDenial::ConnectionRequestIdentityMismatch);
        }
        Ok(PendingServiceRequestFrame {
            progress: Arc::clone(&self.request_frame),
            connection_stale: Arc::clone(&self.connection_stale),
            wire_request_id,
        })
    }

    fn finish_frame(&mut self) -> Result<NipcRequestRead, PeerDenial> {
        let envelope = match IpcEnvelope::decode(&self.bytes) {
            Ok(envelope) => envelope,
            Err(_) => {
                self.fail_connection();
                return Err(PeerDenial::InvalidNipcRequest);
            }
        };
        let Some(request_id) = envelope.request_id else {
            self.fail_connection();
            return Err(PeerDenial::InvalidNipcRequest);
        };
        let Some(wire_request_id) = self.wire_request_id else {
            self.fail_connection();
            return Err(PeerDenial::Unavailable);
        };
        if envelope.kind != MessageKind::Request
            || request_id != RequestId(wire_request_id.as_bytes())
            || envelope.source.0 != self.expected_source_app_id
        {
            self.fail_connection();
            return Err(PeerDenial::InvalidNipcRequest);
        }
        let pending = self.pending.take().ok_or(PeerDenial::Unavailable)?;
        let frame = pending.finish()?;
        let decoded = DecodedNipcRequest {
            frame,
            wire_request_id,
            fingerprint: ServiceRequestFingerprint::from_validated_nipc_frame(&self.bytes),
            source_app_id: envelope.source.0,
            destination_app_id: envelope.destination.map(|app| app.0),
            correlation_id: envelope.correlation_id.0,
            payload_type: envelope.payload_type.to_owned(),
            payload: envelope.payload.to_vec(),
            canonical_frame: Arc::from(self.bytes.as_slice()),
        };
        self.terminal = true;
        Ok(NipcRequestRead::Complete(decoded))
    }
}

#[cfg(test)]
pub(super) fn decode_test_request(
    source_app_id: u64,
    destination_app_id: Option<u64>,
    request_id: [u8; 16],
    payload_type: &str,
    payload: &[u8],
) -> Result<DecodedNipcRequest, PeerDenial> {
    decode_test_request_for_connection(
        Arc::new(Mutex::new(None)),
        Arc::new(AtomicBool::new(false)),
        source_app_id,
        destination_app_id,
        request_id,
        payload_type,
        payload,
    )
}

#[cfg(test)]
pub(super) fn decode_test_request_for_connection(
    request_frame: Arc<Mutex<Option<ServiceRequestFrameProgress>>>,
    connection_stale: Arc<AtomicBool>,
    source_app_id: u64,
    destination_app_id: Option<u64>,
    request_id: [u8; 16],
    payload_type: &str,
    payload: &[u8],
) -> Result<DecodedNipcRequest, PeerDenial> {
    let envelope = IpcEnvelope {
        protocol_version: IPC_PROTOCOL_VERSION,
        source: nagi_sdk::AppId(source_app_id),
        destination: destination_app_id.map(nagi_sdk::AppId),
        kind: MessageKind::Request,
        correlation_id: CorrelationId([3; 16]),
        request_id: Some(RequestId(request_id)),
        ordering: OrderingMetadata {
            timestamp_micros: None,
            sequence: None,
        },
        payload_type,
        payload,
    };
    let mut bytes = vec![0; FIXED_HEADER_BYTES + payload_type.len() + payload.len()];
    let length = envelope
        .encode(&mut bytes)
        .map_err(|_| PeerDenial::InvalidNipcRequest)?;
    bytes.truncate(length);
    let mut decoder = NipcRequestDecoder::for_authorized_peer(
        request_frame,
        connection_stale,
        source_app_id,
        Instant::now() + std::time::Duration::from_secs(5),
    );
    match decoder.read_step(&mut io::Cursor::new(&bytes))? {
        NipcRequestRead::Complete(request) => Ok(request),
        NipcRequestRead::Pending { .. } => Err(PeerDenial::IncompleteRequestFrame),
    }
}

impl Drop for NipcRequestDecoder {
    fn drop(&mut self) {
        if !self.terminal {
            self.fail_connection();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nagi_sdk::app_contract::{
        CorrelationId, IpcEnvelope, OrderingMetadata, IPC_PROTOCOL_VERSION,
    };
    use nagi_sdk::AppId;
    use std::collections::VecDeque;

    enum Chunk {
        Bytes(Vec<u8>),
        Interrupted,
        WouldBlock,
        Eof,
    }

    struct ScriptedReader(VecDeque<Chunk>);

    impl Read for ScriptedReader {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            match self.0.pop_front().unwrap_or(Chunk::Eof) {
                Chunk::Bytes(bytes) => {
                    let count = bytes.len().min(output.len());
                    output[..count].copy_from_slice(&bytes[..count]);
                    if count < bytes.len() {
                        self.0.push_front(Chunk::Bytes(bytes[count..].to_vec()));
                    }
                    Ok(count)
                }
                Chunk::WouldBlock => Err(io::Error::from(ErrorKind::WouldBlock)),
                Chunk::Interrupted => Err(io::Error::from(ErrorKind::Interrupted)),
                Chunk::Eof => Ok(0),
            }
        }
    }

    fn encoded_request(source: u64, request_id: [u8; 16]) -> Vec<u8> {
        let envelope = IpcEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            source: AppId(source),
            destination: Some(AppId(2)),
            kind: MessageKind::Request,
            correlation_id: CorrelationId([4; 16]),
            request_id: Some(RequestId(request_id)),
            ordering: OrderingMetadata {
                timestamp_micros: None,
                sequence: None,
            },
            payload_type: "svc.call@1",
            payload: b"bounded body",
        };
        let mut bytes =
            vec![0; FIXED_HEADER_BYTES + envelope.payload_type.len() + envelope.payload.len()];
        let length = envelope.encode(&mut bytes).unwrap();
        bytes.truncate(length);
        bytes
    }

    fn new_decoder(expected_source_app_id: u64, deadline: Instant) -> NipcRequestDecoder {
        NipcRequestDecoder::for_authorized_peer(
            Arc::new(Mutex::new(None)),
            Arc::new(AtomicBool::new(false)),
            expected_source_app_id,
            deadline,
        )
    }

    #[test]
    fn canonical_request_preserves_partial_header_and_body_across_would_block() {
        let bytes = encoded_request(77, [9; 16]);
        let reader_bytes = vec![
            Chunk::Bytes(bytes[..50].to_vec()),
            Chunk::WouldBlock,
            Chunk::Bytes(bytes[50..FIXED_HEADER_BYTES].to_vec()),
            Chunk::WouldBlock,
            Chunk::Bytes(bytes[FIXED_HEADER_BYTES..].to_vec()),
        ];
        let mut reader = ScriptedReader(reader_bytes.into());
        let mut decoder = new_decoder(77, Instant::now() + std::time::Duration::from_secs(5));

        assert!(matches!(
            decoder.read_step(&mut reader).unwrap(),
            NipcRequestRead::Pending {
                received_bytes: 50,
                expected_bytes: None
            }
        ));
        assert!(matches!(
            decoder.read_step(&mut reader).unwrap(),
            NipcRequestRead::Pending {
                received_bytes: FIXED_HEADER_BYTES,
                expected_bytes: Some(_)
            }
        ));
        let NipcRequestRead::Complete(request) = decoder.read_step(&mut reader).unwrap() else {
            panic!("complete NIPC request expected");
        };
        assert_eq!(request.wire_request_id().as_bytes(), [9; 16]);
        assert_eq!(request.source_app_id(), 77);
        assert_eq!(request.destination_app_id(), Some(2));
        assert_eq!(request.payload_type(), "svc.call@1");
        assert_eq!(request.payload(), b"bounded body");
    }

    #[test]
    fn interrupted_read_returns_pending_and_resumes_same_frame_without_spinning() {
        let bytes = encoded_request(77, [7; 16]);
        let mut reader = ScriptedReader(
            vec![
                Chunk::Interrupted,
                Chunk::Bytes(bytes[..50].to_vec()),
                Chunk::Interrupted,
                Chunk::Bytes(bytes[50..].to_vec()),
            ]
            .into(),
        );
        let mut decoder = new_decoder(77, Instant::now() + std::time::Duration::from_secs(5));

        assert!(matches!(
            decoder.read_step(&mut reader).unwrap(),
            NipcRequestRead::Pending {
                received_bytes: 0,
                expected_bytes: None
            }
        ));
        assert!(matches!(
            decoder.read_step(&mut reader).unwrap(),
            NipcRequestRead::Pending {
                received_bytes: 50,
                expected_bytes: None
            }
        ));
        let NipcRequestRead::Complete(request) = decoder.read_step(&mut reader).unwrap() else {
            panic!("complete NIPC request expected");
        };
        assert_eq!(request.wire_request_id().as_bytes(), [7; 16]);
        assert_eq!(request.payload(), b"bounded body");
    }

    #[test]
    fn source_mismatch_and_expired_deadline_stale_the_connection() {
        let bytes = encoded_request(77, [8; 16]);
        let mut reader = ScriptedReader(vec![Chunk::Bytes(bytes)].into());
        let mut decoder = new_decoder(88, Instant::now() + std::time::Duration::from_secs(5));
        assert!(matches!(
            decoder.read_step(&mut reader),
            Err(PeerDenial::InvalidNipcRequest)
        ));
        assert!(decoder.connection_stale.load(Ordering::SeqCst));

        let stale = Arc::new(AtomicBool::new(false));
        let mut decoder = NipcRequestDecoder::for_authorized_peer(
            Arc::new(Mutex::new(None)),
            Arc::clone(&stale),
            77,
            Instant::now()
                .checked_sub(std::time::Duration::from_secs(1))
                .unwrap(),
        );
        let mut empty = ScriptedReader(VecDeque::new());
        assert!(matches!(
            decoder.read_step(&mut empty),
            Err(PeerDenial::RequestDeadlineExpired)
        ));
        assert!(stale.load(Ordering::SeqCst));
    }

    #[test]
    fn unsupported_version_and_oversized_schema_or_payload_are_rejected_at_header() {
        let deadline = || Instant::now() + std::time::Duration::from_secs(5);

        let mut wrong_version = encoded_request(77, [5; 16]);
        wrong_version[4..6].copy_from_slice(&(IPC_PROTOCOL_VERSION + 1).to_le_bytes());
        let mut decoder = new_decoder(77, deadline());
        let mut reader =
            ScriptedReader(vec![Chunk::Bytes(wrong_version[..FIXED_HEADER_BYTES].to_vec())].into());
        assert!(matches!(
            decoder.read_step(&mut reader),
            Err(PeerDenial::InvalidNipcRequest)
        ));

        let mut oversized_schema = encoded_request(77, [6; 16]);
        oversized_schema[PAYLOAD_TYPE_LEN_OFFSET..PAYLOAD_TYPE_LEN_OFFSET + 2]
            .copy_from_slice(&((MAX_PAYLOAD_TYPE_BYTES as u16) + 1).to_le_bytes());
        let mut decoder = new_decoder(77, deadline());
        let mut reader = ScriptedReader(
            vec![Chunk::Bytes(
                oversized_schema[..FIXED_HEADER_BYTES].to_vec(),
            )]
            .into(),
        );
        assert!(matches!(
            decoder.read_step(&mut reader),
            Err(PeerDenial::InvalidNipcRequest)
        ));

        let mut oversized_payload = encoded_request(77, [7; 16]);
        oversized_payload[PAYLOAD_LEN_OFFSET..PAYLOAD_LEN_OFFSET + 4]
            .copy_from_slice(&((MAX_IPC_PAYLOAD_BYTES as u32) + 1).to_le_bytes());
        let mut decoder = new_decoder(77, deadline());
        let mut reader = ScriptedReader(
            vec![Chunk::Bytes(
                oversized_payload[..FIXED_HEADER_BYTES].to_vec(),
            )]
            .into(),
        );
        assert!(matches!(
            decoder.read_step(&mut reader),
            Err(PeerDenial::InvalidNipcRequest)
        ));
    }

    #[test]
    fn eof_and_non_request_envelopes_are_terminal() {
        let stale = Arc::new(AtomicBool::new(false));
        let mut decoder = NipcRequestDecoder::for_authorized_peer(
            Arc::new(Mutex::new(None)),
            Arc::clone(&stale),
            77,
            Instant::now() + std::time::Duration::from_secs(5),
        );
        let mut eof = ScriptedReader(VecDeque::new());
        assert!(matches!(
            decoder.read_step(&mut eof),
            Err(PeerDenial::StaleConnection)
        ));
        assert!(stale.load(Ordering::SeqCst));

        let response = IpcEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            source: AppId(77),
            destination: Some(AppId(2)),
            kind: MessageKind::Response,
            correlation_id: CorrelationId([4; 16]),
            request_id: Some(RequestId([3; 16])),
            ordering: OrderingMetadata {
                timestamp_micros: None,
                sequence: None,
            },
            payload_type: "svc.reply@1",
            payload: b"no",
        };
        let mut bytes =
            vec![0; FIXED_HEADER_BYTES + response.payload_type.len() + response.payload.len()];
        let length = response.encode(&mut bytes).unwrap();
        bytes.truncate(length);
        let mut reader = ScriptedReader(vec![Chunk::Bytes(bytes)].into());
        let mut decoder = new_decoder(77, Instant::now() + std::time::Duration::from_secs(5));
        assert!(matches!(
            decoder.read_step(&mut reader),
            Err(PeerDenial::InvalidNipcRequest)
        ));
        assert!(decoder.connection_stale.load(Ordering::SeqCst));
    }
}
