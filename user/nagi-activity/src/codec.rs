use alloc::vec::Vec;

use crate::model::{ActivityEvent, ACTIVITY_EVENT_SCHEMA_VERSION};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecError {
    Encode,
    Decode,
    UnsupportedSchemaVersion,
    InvalidContext,
}

/// Encode one versioned activity event. Event content references remain
/// metadata; the codec never fetches or embeds referenced object contents.
pub fn encode_event(event: &ActivityEvent) -> Result<Vec<u8>, CodecError> {
    if event.schema_version != ACTIVITY_EVENT_SCHEMA_VERSION {
        return Err(CodecError::UnsupportedSchemaVersion);
    }
    if !event.context.is_valid() {
        return Err(CodecError::InvalidContext);
    }
    postcard::to_allocvec(event).map_err(|_| CodecError::Encode)
}

pub fn decode_event(bytes: &[u8]) -> Result<ActivityEvent, CodecError> {
    let event: ActivityEvent = postcard::from_bytes(bytes).map_err(|_| CodecError::Decode)?;
    if event.schema_version != ACTIVITY_EVENT_SCHEMA_VERSION {
        return Err(CodecError::UnsupportedSchemaVersion);
    }
    if !event.context.is_valid() {
        return Err(CodecError::InvalidContext);
    }
    Ok(event)
}
