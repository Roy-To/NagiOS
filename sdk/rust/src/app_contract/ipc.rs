use crate::AppId;

use super::{AppError, CorrelationId, ErrorCode, RequestId};

pub const IPC_PROTOCOL_VERSION: u16 = 1;
pub const MAX_IPC_PAYLOAD_BYTES: usize = 1_048_576;
const MAGIC: &[u8; 4] = b"NIPC";
const FIXED_HEADER_BYTES: usize = 78;
const FLAG_DESTINATION: u8 = 1 << 0;
const FLAG_REQUEST_ID: u8 = 1 << 1;
const FLAG_TIMESTAMP: u8 = 1 << 2;
const FLAG_SEQUENCE: u8 = 1 << 3;
const KNOWN_FLAGS: u8 = FLAG_DESTINATION | FLAG_REQUEST_ID | FLAG_TIMESTAMP | FLAG_SEQUENCE;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MessageKind {
    Request = 1,
    Response = 2,
    Event = 3,
    Error = 4,
}

impl MessageKind {
    fn decode(value: u8) -> Result<Self, AppError> {
        match value {
            1 => Ok(Self::Request),
            2 => Ok(Self::Response),
            3 => Ok(Self::Event),
            4 => Ok(Self::Error),
            _ => Err(AppError::new(ErrorCode::InvalidIpcMessage)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OrderingMetadata {
    pub timestamp_micros: Option<u64>,
    pub sequence: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpcEnvelope<'a> {
    pub protocol_version: u16,
    pub source: AppId,
    pub destination: Option<AppId>,
    pub kind: MessageKind,
    pub correlation_id: CorrelationId,
    pub request_id: Option<RequestId>,
    pub ordering: OrderingMetadata,
    pub payload_type: &'a str,
    pub payload: &'a [u8],
}

impl IpcEnvelope<'_> {
    /// Encode a stable, little-endian NIPC v1 envelope. This is a contract
    /// codec only; delivery and ordering guarantees belong to the adapter.
    pub fn encode(&self, output: &mut [u8]) -> Result<usize, AppError> {
        if self.protocol_version != IPC_PROTOCOL_VERSION {
            return Err(AppError::new(ErrorCode::IpcProtocolMismatch));
        }
        if !valid_payload_type(self.payload_type)
            || self.payload_type.len() > u16::MAX as usize
            || self.payload.len() > MAX_IPC_PAYLOAD_BYTES
            || self.payload.len() > u32::MAX as usize
            || matches!(self.kind, MessageKind::Request | MessageKind::Response)
                && self.request_id.is_none()
        {
            return Err(AppError::new(ErrorCode::InvalidIpcMessage));
        }
        let payload_type = self.payload_type.as_bytes();
        let required = FIXED_HEADER_BYTES
            .checked_add(payload_type.len())
            .and_then(|value| value.checked_add(self.payload.len()))
            .ok_or(AppError::new(ErrorCode::InvalidIpcMessage))?;
        if output.len() < required {
            return Err(AppError::new(ErrorCode::BufferTooSmall));
        }
        let mut flags = 0;
        if self.destination.is_some() {
            flags |= FLAG_DESTINATION;
        }
        if self.request_id.is_some() {
            flags |= FLAG_REQUEST_ID;
        }
        if self.ordering.timestamp_micros.is_some() {
            flags |= FLAG_TIMESTAMP;
        }
        if self.ordering.sequence.is_some() {
            flags |= FLAG_SEQUENCE;
        }

        let mut offset = 0;
        output[offset..offset + 4].copy_from_slice(MAGIC);
        offset += 4;
        write_u16(output, &mut offset, self.protocol_version);
        output[offset] = self.kind as u8;
        offset += 1;
        output[offset] = flags;
        offset += 1;
        write_u64(output, &mut offset, self.source.0);
        write_u64(output, &mut offset, self.destination.map_or(0, |app| app.0));
        output[offset..offset + 16].copy_from_slice(&self.correlation_id.0);
        offset += 16;
        output[offset..offset + 16].copy_from_slice(&self.request_id.map_or([0; 16], |id| id.0));
        offset += 16;
        write_u64(
            output,
            &mut offset,
            self.ordering.timestamp_micros.unwrap_or_default(),
        );
        write_u64(
            output,
            &mut offset,
            self.ordering.sequence.unwrap_or_default(),
        );
        write_u16(output, &mut offset, payload_type.len() as u16);
        write_u32(output, &mut offset, self.payload.len() as u32);
        debug_assert_eq!(offset, FIXED_HEADER_BYTES);
        output[offset..offset + payload_type.len()].copy_from_slice(payload_type);
        offset += payload_type.len();
        output[offset..offset + self.payload.len()].copy_from_slice(self.payload);
        Ok(required)
    }

    pub fn decode(input: &'_ [u8]) -> Result<IpcEnvelope<'_>, AppError> {
        if input.len() < FIXED_HEADER_BYTES || &input[..4] != MAGIC {
            return Err(AppError::new(ErrorCode::InvalidIpcMessage));
        }
        let version = u16::from_le_bytes([input[4], input[5]]);
        if version != IPC_PROTOCOL_VERSION {
            return Err(AppError::new(ErrorCode::IpcProtocolMismatch));
        }
        let kind = MessageKind::decode(input[6])?;
        let flags = input[7];
        if flags & !KNOWN_FLAGS != 0 {
            return Err(AppError::new(ErrorCode::InvalidIpcMessage));
        }
        let mut offset = 8;
        let source = AppId(read_u64(input, &mut offset));
        let destination_value = read_u64(input, &mut offset);
        let mut correlation = [0; 16];
        correlation.copy_from_slice(&input[offset..offset + 16]);
        offset += 16;
        let mut request = [0; 16];
        request.copy_from_slice(&input[offset..offset + 16]);
        offset += 16;
        let timestamp = read_u64(input, &mut offset);
        let sequence = read_u64(input, &mut offset);
        let payload_type_len = usize::from(read_u16(input, &mut offset));
        let payload_len = read_u32(input, &mut offset) as usize;
        if payload_len > MAX_IPC_PAYLOAD_BYTES {
            return Err(AppError::new(ErrorCode::InvalidIpcMessage));
        }
        let expected = FIXED_HEADER_BYTES
            .checked_add(payload_type_len)
            .and_then(|value| value.checked_add(payload_len))
            .ok_or(AppError::new(ErrorCode::InvalidIpcMessage))?;
        if expected != input.len() {
            return Err(AppError::new(ErrorCode::InvalidIpcMessage));
        }
        if flags & FLAG_DESTINATION == 0 && destination_value != 0
            || flags & FLAG_REQUEST_ID == 0 && request != [0; 16]
            || flags & FLAG_TIMESTAMP == 0 && timestamp != 0
            || flags & FLAG_SEQUENCE == 0 && sequence != 0
            || matches!(kind, MessageKind::Request | MessageKind::Response)
                && flags & FLAG_REQUEST_ID == 0
        {
            return Err(AppError::new(ErrorCode::InvalidIpcMessage));
        }
        let payload_type_end = offset + payload_type_len;
        let payload_type = core::str::from_utf8(&input[offset..payload_type_end])
            .map_err(|_| AppError::new(ErrorCode::InvalidIpcMessage))?;
        if !valid_payload_type(payload_type) {
            return Err(AppError::new(ErrorCode::InvalidIpcMessage));
        }
        let payload = &input[payload_type_end..];
        Ok(IpcEnvelope {
            protocol_version: version,
            source,
            destination: (flags & FLAG_DESTINATION != 0).then_some(AppId(destination_value)),
            kind,
            correlation_id: CorrelationId(correlation),
            request_id: (flags & FLAG_REQUEST_ID != 0).then_some(RequestId(request)),
            ordering: OrderingMetadata {
                timestamp_micros: (flags & FLAG_TIMESTAMP != 0).then_some(timestamp),
                sequence: (flags & FLAG_SEQUENCE != 0).then_some(sequence),
            },
            payload_type,
            payload,
        })
    }
}

fn valid_payload_type(value: &str) -> bool {
    if value.is_empty() || value.len() > 128 {
        return false;
    }
    let mut separators = 0;
    value.bytes().all(|byte| {
        if byte == b'@' {
            separators += 1;
            separators == 1
        } else {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        }
    }) && value.split_once('@').is_some_and(|(name, version)| {
        !name.is_empty() && !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit())
    })
}

fn write_u16(buffer: &mut [u8], offset: &mut usize, value: u16) {
    buffer[*offset..*offset + 2].copy_from_slice(&value.to_le_bytes());
    *offset += 2;
}

fn write_u32(buffer: &mut [u8], offset: &mut usize, value: u32) {
    buffer[*offset..*offset + 4].copy_from_slice(&value.to_le_bytes());
    *offset += 4;
}

fn write_u64(buffer: &mut [u8], offset: &mut usize, value: u64) {
    buffer[*offset..*offset + 8].copy_from_slice(&value.to_le_bytes());
    *offset += 8;
}

fn read_u16(buffer: &[u8], offset: &mut usize) -> u16 {
    let value = u16::from_le_bytes([buffer[*offset], buffer[*offset + 1]]);
    *offset += 2;
    value
}

fn read_u32(buffer: &[u8], offset: &mut usize) -> u32 {
    let value = u32::from_le_bytes([
        buffer[*offset],
        buffer[*offset + 1],
        buffer[*offset + 2],
        buffer[*offset + 3],
    ]);
    *offset += 4;
    value
}

fn read_u64(buffer: &[u8], offset: &mut usize) -> u64 {
    let value = u64::from_le_bytes([
        buffer[*offset],
        buffer[*offset + 1],
        buffer[*offset + 2],
        buffer[*offset + 3],
        buffer[*offset + 4],
        buffer[*offset + 5],
        buffer[*offset + 6],
        buffer[*offset + 7],
    ]);
    *offset += 8;
    value
}

#[cfg(test)]
mod tests {
    use super::{IpcEnvelope, MessageKind, OrderingMetadata, IPC_PROTOCOL_VERSION};
    use crate::app_contract::{AppError, CorrelationId, ErrorCode, RequestId};
    use crate::AppId;

    fn request() -> IpcEnvelope<'static> {
        IpcEnvelope {
            protocol_version: IPC_PROTOCOL_VERSION,
            source: AppId(1),
            destination: Some(AppId(2)),
            kind: MessageKind::Request,
            correlation_id: CorrelationId([3; 16]),
            request_id: Some(RequestId([4; 16])),
            ordering: OrderingMetadata {
                timestamp_micros: Some(100),
                sequence: Some(8),
            },
            payload_type: "nagi.resource-reference@1",
            payload: b"opaque-resource-id",
        }
    }

    #[test]
    fn request_envelope_roundtrips_over_binary_wire_format() {
        let original = request();
        let mut bytes = [0; 256];
        let length = original.encode(&mut bytes).unwrap();
        assert_eq!(IpcEnvelope::decode(&bytes[..length]).unwrap(), original);
        assert_eq!(&bytes[..4], b"NIPC");
    }

    #[test]
    fn unsupported_protocol_version_is_rejected_on_encode_and_decode() {
        let mut bad = request();
        bad.protocol_version = 2;
        let mut bytes = [0; 256];
        assert_eq!(
            bad.encode(&mut bytes),
            Err(AppError::new(ErrorCode::IpcProtocolMismatch))
        );
        let length = request().encode(&mut bytes).unwrap();
        bytes[4..6].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(
            IpcEnvelope::decode(&bytes[..length]),
            Err(AppError::new(ErrorCode::IpcProtocolMismatch))
        );
    }

    #[test]
    fn malformed_flags_lengths_and_trailing_data_are_rejected() {
        let mut bytes = [0; 256];
        let length = request().encode(&mut bytes).unwrap();
        bytes[7] |= 0x80;
        assert_eq!(
            IpcEnvelope::decode(&bytes[..length]),
            Err(AppError::new(ErrorCode::InvalidIpcMessage))
        );
        bytes[7] &= !0x80;
        assert_eq!(
            IpcEnvelope::decode(&bytes[..length - 1]),
            Err(AppError::new(ErrorCode::InvalidIpcMessage))
        );
        assert_eq!(
            IpcEnvelope::decode(&bytes[..length + 1]),
            Err(AppError::new(ErrorCode::InvalidIpcMessage))
        );
    }

    #[test]
    fn request_requires_request_id_and_small_output_is_reported() {
        let mut bad = request();
        bad.request_id = None;
        let mut bytes = [0; 256];
        assert_eq!(
            bad.encode(&mut bytes),
            Err(AppError::new(ErrorCode::InvalidIpcMessage))
        );
        assert_eq!(
            request().encode(&mut [0; 8]),
            Err(AppError::new(ErrorCode::BufferTooSmall))
        );
    }
}
