//! Versioned binary encoding of [`ClipboardContent`] (format v1).
//!
//! This is a self-describing content envelope suitable for a future bulk
//! transfer buffer. It is not the system-service IPC message format, which is
//! owned by the system-service IPC workstream. Unknown versions, truncated or
//! trailing bytes, unknown tags, and bound violations are rejected; nothing is
//! reinterpreted or truncated.
//!
//! Layout (little-endian): `b"NCLP"`, `u16` version, `u8` intent, `u8` claim
//! flags, optional claimed app `u64`, session `u64`, label `str`, `u32`
//! metadata count with `str` key/value pairs, `u32` item count, and for each
//! item a `u32` representation count followed by `str` media type, `u8` kind,
//! and a `bytes` payload (or `u64` object id). `str`/`bytes` are a `u32`
//! length followed by that many bytes.

use std::collections::BTreeMap;
use std::fmt;

use nagi_model::{AppId, AppSessionId, ObjectId};

use crate::limits::{ClipboardLimits, MAX_METADATA_KEY_BYTES, MAX_ORIGIN_LABEL_BYTES};
use crate::media_type::{MediaType, MAX_MEDIA_TYPE_BYTES};
use crate::model::{
    ClaimedOrigin, ClipboardContent, ClipboardItem, ContentError, MetadataKey, Payload,
    Representation, TransferIntent,
};

/// Envelope magic.
pub const ENCODING_MAGIC: [u8; 4] = *b"NCLP";
/// The only envelope version this foundation reads or writes.
pub const ENCODING_VERSION: u16 = 1;

const FLAG_APP: u8 = 0b001;
const FLAG_SESSION: u8 = 0b010;
const FLAG_LABEL: u8 = 0b100;
const KIND_TEXT: u8 = 0;
const KIND_BINARY: u8 = 1;
const KIND_OBJECT: u8 = 2;

/// Why an envelope could not be decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// Missing or wrong magic.
    BadMagic,
    /// Envelope version is not supported.
    UnsupportedVersion(u16),
    /// Input ended early.
    Truncated,
    /// Bytes remained after a complete envelope.
    TrailingBytes,
    /// Unknown intent, flag, or payload tag.
    UnknownTag,
    /// A declared count or length exceeds its bound.
    BoundExceeded,
    /// The same metadata key appeared twice.
    DuplicateMetadataKey,
    /// Text was not valid UTF-8.
    InvalidUtf8,
    /// Media type identifier is invalid.
    InvalidMediaType,
    /// Decoded content failed validation.
    Invalid(ContentError),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "clipboard envelope rejected: {self:?}")
    }
}

impl std::error::Error for DecodeError {}

/// Encodes validated content. Invalid content is rejected, not encoded.
pub fn encode_content(
    content: &ClipboardContent,
    limits: &ClipboardLimits,
) -> Result<Vec<u8>, ContentError> {
    content.validate(limits)?;
    let mut out = Vec::with_capacity(64 + content.total_inline_bytes());
    out.extend_from_slice(&ENCODING_MAGIC);
    out.extend_from_slice(&ENCODING_VERSION.to_le_bytes());
    out.push(match content.intent() {
        TransferIntent::Copy => 0,
        TransferIntent::Move => 1,
    });
    let claim = content.claimed_origin();
    let mut flags = 0;
    if claim.app.is_some() {
        flags |= FLAG_APP;
    }
    if claim.app_session.is_some() {
        flags |= FLAG_SESSION;
    }
    if claim.label.is_some() {
        flags |= FLAG_LABEL;
    }
    out.push(flags);
    if let Some(app) = claim.app {
        out.extend_from_slice(&app.0.to_le_bytes());
    }
    if let Some(session) = claim.app_session {
        out.extend_from_slice(&session.0.to_le_bytes());
    }
    if let Some(label) = &claim.label {
        put_bytes(&mut out, label.as_bytes());
    }
    put_len(&mut out, content.metadata().len());
    for (key, value) in content.metadata() {
        put_bytes(&mut out, key.as_str().as_bytes());
        put_bytes(&mut out, value.as_bytes());
    }
    put_len(&mut out, content.items().len());
    for item in content.items() {
        put_len(&mut out, item.representations().len());
        for representation in item.representations() {
            put_bytes(&mut out, representation.media_type().as_str().as_bytes());
            match representation.payload() {
                Payload::Text(text) => {
                    out.push(KIND_TEXT);
                    put_bytes(&mut out, text.as_bytes());
                }
                Payload::Binary(bytes) => {
                    out.push(KIND_BINARY);
                    put_bytes(&mut out, bytes);
                }
                Payload::ObjectReference(object) => {
                    out.push(KIND_OBJECT);
                    out.extend_from_slice(&object.0.to_le_bytes());
                }
            }
        }
    }
    Ok(out)
}

/// Decodes and validates an envelope against `limits`. Declared lengths are
/// checked before any allocation.
pub fn decode_content(
    input: &[u8],
    limits: &ClipboardLimits,
) -> Result<ClipboardContent, DecodeError> {
    let mut reader = Reader { input, offset: 0 };
    if reader.take(4).map_err(|_| DecodeError::BadMagic)? != ENCODING_MAGIC {
        return Err(DecodeError::BadMagic);
    }
    let version = reader.u16()?;
    if version != ENCODING_VERSION {
        return Err(DecodeError::UnsupportedVersion(version));
    }
    let intent = match reader.u8()? {
        0 => TransferIntent::Copy,
        1 => TransferIntent::Move,
        _ => return Err(DecodeError::UnknownTag),
    };
    let flags = reader.u8()?;
    if flags & !(FLAG_APP | FLAG_SESSION | FLAG_LABEL) != 0 {
        return Err(DecodeError::UnknownTag);
    }
    let mut claim = ClaimedOrigin::default();
    if flags & FLAG_APP != 0 {
        claim.app = Some(AppId(reader.u64()?));
    }
    if flags & FLAG_SESSION != 0 {
        claim.app_session = Some(AppSessionId(reader.u64()?));
    }
    if flags & FLAG_LABEL != 0 {
        claim.label = Some(reader.string(MAX_ORIGIN_LABEL_BYTES)?);
    }
    let metadata_count = reader.count(limits.max_metadata_entries)?;
    let mut metadata = BTreeMap::new();
    for _ in 0..metadata_count {
        let key = MetadataKey::new(reader.string(MAX_METADATA_KEY_BYTES)?)
            .map_err(DecodeError::Invalid)?;
        let value = reader.string(limits.max_metadata_value_bytes)?;
        if metadata.insert(key, value).is_some() {
            // Duplicate keys would otherwise be silently collapsed.
            return Err(DecodeError::DuplicateMetadataKey);
        }
    }
    let item_count = reader.count(limits.max_items)?;
    let mut items = Vec::with_capacity(item_count);
    for _ in 0..item_count {
        let representation_count = reader.count(limits.max_representations_per_item)?;
        let mut representations = Vec::with_capacity(representation_count);
        for _ in 0..representation_count {
            let media_type = MediaType::new(reader.string(MAX_MEDIA_TYPE_BYTES)?)
                .map_err(|_| DecodeError::InvalidMediaType)?;
            let payload = match reader.u8()? {
                KIND_TEXT => Payload::Text(reader.string(limits.max_representation_bytes)?),
                KIND_BINARY => {
                    Payload::Binary(reader.bytes(limits.max_representation_bytes)?.to_vec())
                }
                KIND_OBJECT => Payload::ObjectReference(ObjectId(reader.u64()?)),
                _ => return Err(DecodeError::UnknownTag),
            };
            representations
                .push(Representation::new(media_type, payload).map_err(DecodeError::Invalid)?);
        }
        items.push(ClipboardItem::new(representations));
    }
    if reader.offset != input.len() {
        return Err(DecodeError::TrailingBytes);
    }
    let mut decoded = ClipboardContent::new(items)
        .with_intent(intent)
        .with_claimed_origin(claim);
    for (key, value) in metadata {
        decoded = decoded.with_metadata(key, value);
    }
    decoded.validate(limits).map_err(DecodeError::Invalid)?;
    Ok(decoded)
}

fn put_len(out: &mut Vec<u8>, length: usize) {
    // Validated content is bounded far below u32::MAX by hard caps.
    let length = u32::try_from(length).expect("length bounded by hard caps");
    out.extend_from_slice(&length.to_le_bytes());
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    put_len(out, bytes.len());
    out.extend_from_slice(bytes);
}

struct Reader<'a> {
    input: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], DecodeError> {
        let end = self
            .offset
            .checked_add(length)
            .filter(|&end| end <= self.input.len())
            .ok_or(DecodeError::Truncated)?;
        let bytes = &self.input[self.offset..end];
        self.offset = end;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, DecodeError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, DecodeError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn u64(&mut self) -> Result<u64, DecodeError> {
        let bytes = self.take(8)?;
        let mut array = [0; 8];
        array.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(array))
    }

    fn count(&mut self, bound: usize) -> Result<usize, DecodeError> {
        let count = usize::try_from(self.u32()?).map_err(|_| DecodeError::BoundExceeded)?;
        if count > bound {
            return Err(DecodeError::BoundExceeded);
        }
        Ok(count)
    }

    fn bytes(&mut self, bound: usize) -> Result<&'a [u8], DecodeError> {
        let length = self.count(bound)?;
        self.take(length)
    }

    fn string(&mut self, bound: usize) -> Result<String, DecodeError> {
        let bytes = self.bytes(bound)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| DecodeError::InvalidUtf8)
    }
}
