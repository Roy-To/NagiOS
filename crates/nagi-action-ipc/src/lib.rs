//! `action@1`: bounded Channel wire format for AI action requests (ADR 0045).
//!
//! An isolated client asks the action service to carry out a user intent.
//! The service plans, deterministically validates, and executes it with the
//! caller identity it resolves from the kernel-stamped sender Process ID and
//! the Supervisor launch record. No message carries a caller identity,
//! capability, or Object ID chosen by the client.
#![no_std]

use nagi_abi::MAX_CHANNEL_INLINE_PAYLOAD;

pub const PROTOCOL_ID: u16 = 0x4143;
pub const PROTOCOL_VERSION: u16 = 1;
/// Supervisor -> client launch argument (like argv): the intent to request.
pub const OPCODE_LAUNCH_INTENT: u16 = 1;
/// Client -> service: request execution of an intent.
pub const OPCODE_REQUEST: u16 = 2;
/// Service -> client: outcome of the request.
pub const OPCODE_RESULT: u16 = 3;

/// Intent layout: `[len u8][UTF-8 ...]`.
pub const MAX_INTENT_BYTES: usize = MAX_CHANNEL_INLINE_PAYLOAD - 1;
/// Result layout: `[status u8][count u8][ids u64 LE ...]`.
pub const MAX_RESULT_OBJECTS: usize = (MAX_CHANNEL_INLINE_PAYLOAD - 2) / 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionStatus {
    Succeeded = 0,
    /// Validation or policy rejected the plan for the resolved caller.
    Denied = 1,
    /// The intent could not be decoded or planned.
    InvalidRequest = 2,
    /// A validated plan failed during execution.
    Failed = 3,
    /// The sender has no Supervisor launch record.
    UnknownCaller = 4,
}

impl ActionStatus {
    fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Succeeded),
            1 => Some(Self::Denied),
            2 => Some(Self::InvalidRequest),
            3 => Some(Self::Failed),
            4 => Some(Self::UnknownCaller),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ActionResult {
    pub status: ActionStatus,
    pub count: usize,
    pub object_ids: [u64; MAX_RESULT_OBJECTS],
}

impl ActionResult {
    pub const fn status_only(status: ActionStatus) -> Self {
        Self {
            status,
            count: 0,
            object_ids: [0; MAX_RESULT_OBJECTS],
        }
    }

    /// A successful result listing the affected objects.
    pub fn succeeded(object_ids: &[u64]) -> Option<Self> {
        if object_ids.len() > MAX_RESULT_OBJECTS {
            return None;
        }
        let mut result = Self::status_only(ActionStatus::Succeeded);
        result.object_ids[..object_ids.len()].copy_from_slice(object_ids);
        result.count = object_ids.len();
        Some(result)
    }

    pub fn object_ids(&self) -> &[u64] {
        &self.object_ids[..self.count]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WireError {
    TooLong,
    Truncated,
    TrailingBytes,
    EmptyIntent,
    InvalidUtf8,
    InvalidStatus,
}

pub fn encode_intent(
    intent: &str,
    output: &mut [u8; MAX_CHANNEL_INLINE_PAYLOAD],
) -> Result<usize, WireError> {
    let bytes = intent.as_bytes();
    if bytes.is_empty() {
        return Err(WireError::EmptyIntent);
    }
    if bytes.len() > MAX_INTENT_BYTES {
        return Err(WireError::TooLong);
    }
    output[0] = bytes.len() as u8;
    output[1..1 + bytes.len()].copy_from_slice(bytes);
    Ok(1 + bytes.len())
}

pub fn decode_intent(payload: &[u8]) -> Result<&str, WireError> {
    let Some(&length) = payload.first() else {
        return Err(WireError::Truncated);
    };
    let length = usize::from(length);
    if length == 0 {
        return Err(WireError::EmptyIntent);
    }
    if length > MAX_INTENT_BYTES {
        return Err(WireError::TooLong);
    }
    let end = 1 + length;
    if payload.len() < end {
        return Err(WireError::Truncated);
    }
    if payload.len() > end {
        return Err(WireError::TrailingBytes);
    }
    core::str::from_utf8(&payload[1..end]).map_err(|_| WireError::InvalidUtf8)
}

pub fn encode_result(
    result: &ActionResult,
    output: &mut [u8; MAX_CHANNEL_INLINE_PAYLOAD],
) -> Result<usize, WireError> {
    if result.count > MAX_RESULT_OBJECTS
        || (result.status != ActionStatus::Succeeded && result.count != 0)
    {
        return Err(WireError::InvalidStatus);
    }
    output[0] = result.status as u8;
    output[1] = result.count as u8;
    for (index, id) in result.object_ids().iter().enumerate() {
        let start = 2 + index * 8;
        output[start..start + 8].copy_from_slice(&id.to_le_bytes());
    }
    Ok(2 + result.count * 8)
}

pub fn decode_result(payload: &[u8]) -> Result<ActionResult, WireError> {
    if payload.len() < 2 {
        return Err(WireError::Truncated);
    }
    let status = ActionStatus::from_byte(payload[0]).ok_or(WireError::InvalidStatus)?;
    let count = usize::from(payload[1]);
    if count > MAX_RESULT_OBJECTS || (status != ActionStatus::Succeeded && count != 0) {
        return Err(WireError::InvalidStatus);
    }
    let end = 2 + count * 8;
    if payload.len() < end {
        return Err(WireError::Truncated);
    }
    if payload.len() > end {
        return Err(WireError::TrailingBytes);
    }
    let mut result = ActionResult::status_only(status);
    for (index, id) in result.object_ids.iter_mut().take(count).enumerate() {
        let start = 2 + index * 8;
        let mut bytes = [0_u8; 8];
        bytes.copy_from_slice(&payload[start..start + 8]);
        *id = u64::from_le_bytes(bytes);
    }
    result.count = count;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_round_trips_and_rejects_malformed_payloads() {
        let mut buffer = [0_u8; MAX_CHANNEL_INLINE_PAYLOAD];
        let length = encode_intent("move the three M22 fixture files", &mut buffer).unwrap();
        assert_eq!(
            decode_intent(&buffer[..length]),
            Ok("move the three M22 fixture files")
        );
        assert_eq!(
            decode_intent(&buffer[..length - 1]),
            Err(WireError::Truncated)
        );
        assert_eq!(
            decode_intent(&buffer[..length + 1]),
            Err(WireError::TrailingBytes)
        );
        assert_eq!(decode_intent(&[]), Err(WireError::Truncated));
        assert_eq!(decode_intent(&[0]), Err(WireError::EmptyIntent));
        assert_eq!(decode_intent(&[1, 0xff]), Err(WireError::InvalidUtf8));
        let long = core::str::from_utf8(&[b'x'; MAX_INTENT_BYTES + 1]).unwrap();
        assert_eq!(encode_intent(long, &mut buffer), Err(WireError::TooLong));
        assert_eq!(encode_intent("", &mut buffer), Err(WireError::EmptyIntent));
    }

    #[test]
    fn results_round_trip_and_only_success_carries_objects() {
        let mut buffer = [0_u8; MAX_CHANNEL_INLINE_PAYLOAD];
        let ids = [0x2211, 0x2212, 0x2213];
        let result = ActionResult::succeeded(&ids).unwrap();
        let length = encode_result(&result, &mut buffer).unwrap();
        assert_eq!(decode_result(&buffer[..length]), Ok(result));
        assert_eq!(decode_result(&buffer[..length]).unwrap().object_ids(), &ids);

        let denied = ActionResult::status_only(ActionStatus::Denied);
        let length = encode_result(&denied, &mut buffer).unwrap();
        assert_eq!(decode_result(&buffer[..length]), Ok(denied));

        let mut forged = denied;
        forged.count = 1;
        assert_eq!(
            encode_result(&forged, &mut buffer),
            Err(WireError::InvalidStatus)
        );
        assert_eq!(
            decode_result(&[1, 1, 0, 0, 0, 0, 0, 0, 0, 0]),
            Err(WireError::InvalidStatus)
        );
        assert_eq!(decode_result(&[9, 0]), Err(WireError::InvalidStatus));
        assert_eq!(decode_result(&[0, 1, 0]), Err(WireError::Truncated));
        assert!(ActionResult::succeeded(&[0; MAX_RESULT_OBJECTS + 1]).is_none());
        let full = ActionResult::succeeded(&[7; MAX_RESULT_OBJECTS]).unwrap();
        let length = encode_result(&full, &mut buffer).unwrap();
        assert!(length <= MAX_CHANNEL_INLINE_PAYLOAD);
    }
}
