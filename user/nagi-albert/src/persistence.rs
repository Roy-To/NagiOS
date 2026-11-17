//! Versioned bounded records and a storage-service adapter contract.

pub const MAX_RECORD_BYTES: usize = 1_048_576;
pub const MAX_FIELD_BYTES: usize = 4096;
const HEADER_BYTES: usize = 16;
const FORMAT_VERSION: u16 = 1;
const MAGIC: &[u8; 4] = b"NGAB";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum RecordKind {
    Session = 1,
    History = 2,
    Bookmarks = 3,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum StorageRecord {
    Session,
    History,
    Bookmarks,
}

impl StorageRecord {
    pub const fn kind(self) -> RecordKind {
        match self {
            Self::Session => RecordKind::Session,
            Self::History => RecordKind::History,
            Self::Bookmarks => RecordKind::Bookmarks,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageWrite {
    pub record: StorageRecord,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageError {
    Denied,
    Unavailable,
    Io,
    Capacity,
}

/// The real adapter owns persistence and capability checks; this model never accepts a path.
pub trait BrowserStorage {
    fn read_record(&mut self, record: StorageRecord) -> Result<Option<Vec<u8>>, StorageError>;

    /// Implementations should commit the record set atomically.
    fn write_batch(&mut self, writes: &[StorageWrite]) -> Result<(), StorageError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecError {
    TooLarge,
    Truncated,
    InvalidMagic,
    UnsupportedVersion,
    WrongRecordKind,
    ChecksumMismatch,
    InvalidUtf8,
    InvalidValue,
    TrailingData,
}

pub(crate) struct Encoder {
    bytes: Vec<u8>,
}

impl Encoder {
    pub(crate) fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn string(&mut self, value: &str) -> Result<(), CodecError> {
        if value.len() > MAX_FIELD_BYTES || value.len() > u16::MAX as usize {
            return Err(CodecError::TooLarge);
        }
        self.u16(value.len() as u16);
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }

    pub(crate) fn optional_string(&mut self, value: Option<&str>) -> Result<(), CodecError> {
        match value {
            Some(value) => {
                self.u8(1);
                self.string(value)
            }
            None => {
                self.u8(0);
                Ok(())
            }
        }
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

pub(crate) struct Decoder<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl<'a> Decoder<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }

    pub(crate) fn u8(&mut self) -> Result<u8, CodecError> {
        let byte = *self.bytes.get(self.cursor).ok_or(CodecError::Truncated)?;
        self.cursor += 1;
        Ok(byte)
    }

    pub(crate) fn u16(&mut self) -> Result<u16, CodecError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, CodecError> {
        let bytes = self.take(8)?;
        Ok(u64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    pub(crate) fn string(&mut self) -> Result<String, CodecError> {
        let length = usize::from(self.u16()?);
        if length > MAX_FIELD_BYTES {
            return Err(CodecError::TooLarge);
        }
        String::from_utf8(self.take(length)?.to_vec()).map_err(|_| CodecError::InvalidUtf8)
    }

    pub(crate) fn optional_string(&mut self) -> Result<Option<String>, CodecError> {
        match self.u8()? {
            0 => Ok(None),
            1 => self.string().map(Some),
            _ => Err(CodecError::InvalidValue),
        }
    }

    pub(crate) fn finish(self) -> Result<(), CodecError> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err(CodecError::TrailingData)
        }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], CodecError> {
        let end = self
            .cursor
            .checked_add(length)
            .ok_or(CodecError::TooLarge)?;
        let bytes = self
            .bytes
            .get(self.cursor..end)
            .ok_or(CodecError::Truncated)?;
        self.cursor = end;
        Ok(bytes)
    }
}

pub(crate) fn encode_record(kind: RecordKind, payload: Vec<u8>) -> Result<Vec<u8>, CodecError> {
    encode_record_version(kind, payload, FORMAT_VERSION)
}

pub(crate) fn encode_record_version(
    kind: RecordKind,
    payload: Vec<u8>,
    version: u16,
) -> Result<Vec<u8>, CodecError> {
    if !(FORMAT_VERSION..=2).contains(&version) {
        return Err(CodecError::UnsupportedVersion);
    }
    let total = HEADER_BYTES
        .checked_add(payload.len())
        .ok_or(CodecError::TooLarge)?;
    if total > MAX_RECORD_BYTES || payload.len() > u32::MAX as usize {
        return Err(CodecError::TooLarge);
    }
    let mut record = Vec::with_capacity(total);
    record.extend_from_slice(MAGIC);
    record.extend_from_slice(&version.to_le_bytes());
    record.extend_from_slice(&(kind as u16).to_le_bytes());
    record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    record.extend_from_slice(&checksum(&payload).to_le_bytes());
    record.extend_from_slice(&payload);
    Ok(record)
}

pub(crate) fn decode_record(record: &[u8], expected: RecordKind) -> Result<&[u8], CodecError> {
    let (payload, version) = decode_record_version(record, expected)?;
    if version != FORMAT_VERSION {
        return Err(CodecError::UnsupportedVersion);
    }
    Ok(payload)
}

pub(crate) fn decode_record_version(
    record: &[u8],
    expected: RecordKind,
) -> Result<(&[u8], u16), CodecError> {
    if record.len() > MAX_RECORD_BYTES {
        return Err(CodecError::TooLarge);
    }
    if record.len() < HEADER_BYTES {
        return Err(CodecError::Truncated);
    }
    if &record[..4] != MAGIC {
        return Err(CodecError::InvalidMagic);
    }
    let version = u16::from_le_bytes([record[4], record[5]]);
    if !(FORMAT_VERSION..=2).contains(&version) {
        return Err(CodecError::UnsupportedVersion);
    }
    let kind = u16::from_le_bytes([record[6], record[7]]);
    if kind != expected as u16 {
        return Err(CodecError::WrongRecordKind);
    }
    let length = u32::from_le_bytes([record[8], record[9], record[10], record[11]]) as usize;
    let expected_checksum = u32::from_le_bytes([record[12], record[13], record[14], record[15]]);
    if HEADER_BYTES.checked_add(length) != Some(record.len()) {
        return Err(CodecError::Truncated);
    }
    let payload = &record[HEADER_BYTES..];
    if checksum(payload) != expected_checksum {
        return Err(CodecError::ChecksumMismatch);
    }
    Ok((payload, version))
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c9dc5_u32, |state, byte| {
        state.wrapping_mul(0x01000193) ^ u32::from(*byte)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_header_detects_truncation_corruption_and_wrong_record_type() {
        let encoded = encode_record(RecordKind::Session, vec![1, 2, 3]).unwrap();
        assert_eq!(
            decode_record(&encoded, RecordKind::Session).unwrap(),
            &[1, 2, 3]
        );
        assert_eq!(
            decode_record(&encoded[..encoded.len() - 1], RecordKind::Session),
            Err(CodecError::Truncated)
        );
        assert_eq!(
            decode_record(&encoded, RecordKind::History),
            Err(CodecError::WrongRecordKind)
        );
        let mut corrupted = encoded;
        *corrupted.last_mut().unwrap() ^= 1;
        assert_eq!(
            decode_record(&corrupted, RecordKind::Session),
            Err(CodecError::ChecksumMismatch)
        );
    }

    #[test]
    fn decoder_rejects_invalid_utf8_and_trailing_payload() {
        let mut payload = Encoder::new();
        payload.u16(1);
        payload.u8(0xff);
        let bytes = payload.finish();
        let mut decoder = Decoder::new(&bytes);
        assert_eq!(decoder.string(), Err(CodecError::InvalidUtf8));
        let mut decoder = Decoder::new(&[1, 0]);
        assert_eq!(decoder.u8(), Ok(1));
        assert_eq!(decoder.finish(), Err(CodecError::TrailingData));
    }
}
