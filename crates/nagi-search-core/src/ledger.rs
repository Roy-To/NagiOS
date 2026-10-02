//! Provider ownership/revision ledger and its versioned snapshot codec.
//!
//! Layout (all integers little-endian), version 1:
//!
//! ```text
//! magic "NSCL" | version u16 | reserved u16 (0)
//! provider_count u32 | provider AppId u64 * n        (strictly ascending)
//! entry_count u32    | entry * m                      (strictly ascending ObjectId)
//!   entry = object u64 | provider u64 | revision u64 (non-zero)
//!         | flags u8 (bit 0 = removed, other bits 0) | fingerprint u64
//! checksum u64 (FNV-1a 64 over every preceding byte)
//! ```
//!
//! Decoding never reinterprets unknown versions, tolerates no trailing bytes,
//! and rejects any structural inconsistency.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};

use nagi_model::{AppId, ObjectId};
use nagi_search::MetadataRecord;

use crate::model::{DocumentRevision, MAX_PROVIDERS};

pub const LEDGER_MAGIC: [u8; 4] = *b"NSCL";
pub const CURRENT_LEDGER_VERSION: u16 = 1;
/// Matches the canonical `nagi_search` object-record capacity.
pub const MAX_LEDGER_ENTRIES: usize = 65_536;

const HEADER_BYTES: usize = 4 + 2 + 2;
const ENTRY_BYTES: usize = 8 + 8 + 8 + 1 + 8;
const FLAG_REMOVED: u8 = 0b0000_0001;
pub const MAX_LEDGER_SNAPSHOT_BYTES: usize =
    HEADER_BYTES + 4 + MAX_PROVIDERS * 8 + 4 + MAX_LEDGER_ENTRIES * ENTRY_BYTES + 8;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerError {
    Corrupt,
    UnsupportedVersion(u16),
    TooLarge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LedgerEntry {
    pub provider: AppId,
    pub revision: DocumentRevision,
    pub removed: bool,
    /// Content fingerprint of the applied record, used only to distinguish an
    /// idempotent replay from a conflicting payload at the same revision.
    pub fingerprint: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Ledger {
    pub providers: BTreeSet<AppId>,
    pub entries: BTreeMap<ObjectId, LedgerEntry>,
}

impl Ledger {
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            HEADER_BYTES + 4 + self.providers.len() * 8 + 4 + self.entries.len() * ENTRY_BYTES + 8,
        );
        out.extend_from_slice(&LEDGER_MAGIC);
        out.extend_from_slice(&CURRENT_LEDGER_VERSION.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(self.providers.len() as u32).to_le_bytes());
        for provider in &self.providers {
            out.extend_from_slice(&provider.0.to_le_bytes());
        }
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (object, entry) in &self.entries {
            out.extend_from_slice(&object.0.to_le_bytes());
            out.extend_from_slice(&entry.provider.0.to_le_bytes());
            out.extend_from_slice(&entry.revision.get().to_le_bytes());
            out.push(if entry.removed { FLAG_REMOVED } else { 0 });
            out.extend_from_slice(&entry.fingerprint.to_le_bytes());
        }
        let checksum = fnv1a(&out);
        out.extend_from_slice(&checksum.to_le_bytes());
        out
    }

    pub(crate) fn decode(bytes: &[u8]) -> Result<Self, LedgerError> {
        if bytes.len() > MAX_LEDGER_SNAPSHOT_BYTES {
            return Err(LedgerError::TooLarge);
        }
        let mut reader = Reader { bytes, offset: 0 };
        if reader.take(4)? != LEDGER_MAGIC {
            return Err(LedgerError::Corrupt);
        }
        let version = reader.u16()?;
        if version != CURRENT_LEDGER_VERSION {
            return Err(LedgerError::UnsupportedVersion(version));
        }
        if reader.u16()? != 0 {
            return Err(LedgerError::Corrupt);
        }
        // Verify the checksum before trusting any count field.
        if bytes.len() < HEADER_BYTES + 8 {
            return Err(LedgerError::Corrupt);
        }
        let (body, trailer) = bytes.split_at(bytes.len() - 8);
        let mut expected = [0u8; 8];
        expected.copy_from_slice(trailer);
        if fnv1a(body) != u64::from_le_bytes(expected) {
            return Err(LedgerError::Corrupt);
        }
        let mut reader = Reader {
            bytes: body,
            offset: reader.offset,
        };

        let provider_count = reader.u32()? as usize;
        if provider_count > MAX_PROVIDERS {
            return Err(LedgerError::Corrupt);
        }
        let mut providers = BTreeSet::new();
        let mut previous: Option<u64> = None;
        for _ in 0..provider_count {
            let value = reader.u64()?;
            if previous.is_some_and(|prior| value <= prior) {
                return Err(LedgerError::Corrupt);
            }
            previous = Some(value);
            providers.insert(AppId(value));
        }

        let entry_count = reader.u32()? as usize;
        if entry_count > MAX_LEDGER_ENTRIES {
            return Err(LedgerError::Corrupt);
        }
        let mut entries = BTreeMap::new();
        let mut previous: Option<u64> = None;
        for _ in 0..entry_count {
            let object = reader.u64()?;
            if previous.is_some_and(|prior| object <= prior) {
                return Err(LedgerError::Corrupt);
            }
            previous = Some(object);
            let provider = AppId(reader.u64()?);
            let revision = DocumentRevision::new(reader.u64()?).ok_or(LedgerError::Corrupt)?;
            let flags = reader.u8()?;
            if flags & !FLAG_REMOVED != 0 {
                return Err(LedgerError::Corrupt);
            }
            let fingerprint = reader.u64()?;
            entries.insert(
                ObjectId(object),
                LedgerEntry {
                    provider,
                    revision,
                    removed: flags & FLAG_REMOVED != 0,
                    fingerprint,
                },
            );
        }
        if reader.offset != body.len() {
            return Err(LedgerError::Corrupt);
        }
        Ok(Self { providers, entries })
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], LedgerError> {
        let end = self.offset.checked_add(len).ok_or(LedgerError::Corrupt)?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or(LedgerError::Corrupt)?;
        self.offset = end;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, LedgerError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, LedgerError> {
        let mut raw = [0u8; 2];
        raw.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(raw))
    }

    fn u32(&mut self) -> Result<u32, LedgerError> {
        let mut raw = [0u8; 4];
        raw.copy_from_slice(self.take(4)?);
        Ok(u32::from_le_bytes(raw))
    }

    fn u64(&mut self) -> Result<u64, LedgerError> {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(raw))
    }
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// Deterministic, length-prefixed content fingerprint of a record as it will
/// be indexed. `tombstoned_at` is excluded because ingestion rejects it.
pub(crate) fn record_fingerprint(record: &MetadataRecord) -> u64 {
    let mut buf = Vec::new();
    buf.extend_from_slice(&record.object_id.0.to_le_bytes());
    buf.push(record.kind as u8);
    put_str(&mut buf, &record.title);
    put_opt_str(&mut buf, record.location.as_deref());
    put_opt_u64(&mut buf, record.source_app.map(|id| id.0));
    put_opt_u64(&mut buf, record.source_session.map(|id| id.0));
    for time in [record.created_at, record.modified_at, record.observed_at] {
        put_opt_u64(&mut buf, time.map(|value| value as u64));
    }
    buf.extend_from_slice(&(record.tags.len() as u64).to_le_bytes());
    for tag in &record.tags {
        put_str(&mut buf, tag);
    }
    buf.extend_from_slice(&(record.attributes.len() as u64).to_le_bytes());
    for (key, value) in &record.attributes {
        put_str(&mut buf, key);
        put_str(&mut buf, value);
    }
    buf.push(record.visibility as u8);
    fnv1a(&buf)
}

fn put_str(buf: &mut Vec<u8>, value: &str) {
    buf.extend_from_slice(&(value.len() as u64).to_le_bytes());
    buf.extend_from_slice(value.as_bytes());
}

fn put_opt_str(buf: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(value) => {
            buf.push(1);
            put_str(buf, value);
        }
        None => buf.push(0),
    }
}

fn put_opt_u64(buf: &mut Vec<u8>, value: Option<u64>) {
    match value {
        Some(value) => {
            buf.push(1);
            buf.extend_from_slice(&value.to_le_bytes());
        }
        None => buf.push(0),
    }
}
