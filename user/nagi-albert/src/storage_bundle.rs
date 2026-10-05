//! Bounded envelope for the guest POSIX browser-storage service.

use crate::persistence::{StorageError, StorageRecord, StorageWrite};

/// Must match `nagi-posix` `BROWSER_STORAGE_MAX_BYTES` (ADR 0056 lifted the
/// former single-block 1 KiB limit).
pub(crate) const MAX_STORAGE_BUNDLE_BYTES: usize = 16 * 1024;
const HEADER_BYTES: usize = 16;
const MAGIC: &[u8; 4] = b"NGBS";
const VERSION: u16 = 1;

pub(crate) fn encode(writes: &[StorageWrite]) -> Result<Vec<u8>, StorageError> {
    if writes.len() > 3 {
        return Err(StorageError::Capacity);
    }

    let mut payload = Vec::new();
    let mut seen = [false; 3];
    for write in writes {
        let tag = record_tag(write.record);
        let Some(slot) = seen.get_mut(usize::from(tag - 1)) else {
            return Err(StorageError::Io);
        };
        if *slot {
            return Err(StorageError::Io);
        }
        *slot = true;
        let Ok(length) = u32::try_from(write.bytes.len()) else {
            return Err(StorageError::Capacity);
        };
        payload.extend_from_slice(&tag.to_le_bytes());
        payload.extend_from_slice(&length.to_le_bytes());
        payload.extend_from_slice(&write.bytes);
        if HEADER_BYTES.saturating_add(payload.len()) > MAX_STORAGE_BUNDLE_BYTES {
            return Err(StorageError::Capacity);
        }
    }

    let payload_length = u32::try_from(payload.len()).map_err(|_| StorageError::Capacity)?;
    let mut bundle = Vec::with_capacity(HEADER_BYTES + payload.len());
    bundle.extend_from_slice(MAGIC);
    bundle.extend_from_slice(&VERSION.to_le_bytes());
    bundle.extend_from_slice(&(writes.len() as u16).to_le_bytes());
    bundle.extend_from_slice(&payload_length.to_le_bytes());
    bundle.extend_from_slice(&checksum(&payload).to_le_bytes());
    bundle.extend_from_slice(&payload);
    Ok(bundle)
}

pub(crate) fn decode(bundle: &[u8]) -> Result<Vec<StorageWrite>, StorageError> {
    if bundle.len() < HEADER_BYTES || bundle.len() > MAX_STORAGE_BUNDLE_BYTES {
        return Err(StorageError::Io);
    }
    if &bundle[..4] != MAGIC || read_u16(bundle, 4) != VERSION {
        return Err(StorageError::Io);
    }
    let count = usize::from(read_u16(bundle, 6));
    if count > 3 {
        return Err(StorageError::Io);
    }
    let payload_length = usize::try_from(read_u32(bundle, 8)).map_err(|_| StorageError::Io)?;
    if HEADER_BYTES.checked_add(payload_length) != Some(bundle.len()) {
        return Err(StorageError::Io);
    }
    let payload = &bundle[HEADER_BYTES..];
    if checksum(payload) != read_u32(bundle, 12) {
        return Err(StorageError::Io);
    }

    let mut writes = Vec::with_capacity(count);
    let mut seen = [false; 3];
    let mut cursor = 0;
    for _ in 0..count {
        let tag = take_u16(payload, &mut cursor)?;
        let record = record_from_tag(tag).ok_or(StorageError::Io)?;
        let slot = usize::from(tag - 1);
        if seen[slot] {
            return Err(StorageError::Io);
        }
        seen[slot] = true;
        let length =
            usize::try_from(take_u32(payload, &mut cursor)?).map_err(|_| StorageError::Io)?;
        let end = cursor.checked_add(length).ok_or(StorageError::Io)?;
        let bytes = payload.get(cursor..end).ok_or(StorageError::Io)?.to_vec();
        cursor = end;
        writes.push(StorageWrite { record, bytes });
    }
    if cursor != payload.len() {
        return Err(StorageError::Io);
    }
    Ok(writes)
}

fn record_tag(record: StorageRecord) -> u16 {
    match record {
        StorageRecord::Session => 1,
        StorageRecord::History => 2,
        StorageRecord::Bookmarks => 3,
    }
}

fn record_from_tag(tag: u16) -> Option<StorageRecord> {
    match tag {
        1 => Some(StorageRecord::Session),
        2 => Some(StorageRecord::History),
        3 => Some(StorageRecord::Bookmarks),
        _ => None,
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn take_u16(bytes: &[u8], cursor: &mut usize) -> Result<u16, StorageError> {
    let end = cursor.checked_add(2).ok_or(StorageError::Io)?;
    let value = bytes.get(*cursor..end).ok_or(StorageError::Io)?;
    *cursor = end;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn take_u32(bytes: &[u8], cursor: &mut usize) -> Result<u32, StorageError> {
    let end = cursor.checked_add(4).ok_or(StorageError::Io)?;
    let value = bytes.get(*cursor..end).ok_or(StorageError::Io)?;
    *cursor = end;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn checksum(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c9dc5_u32, |state, byte| {
        state.wrapping_mul(0x01000193) ^ u32::from(*byte)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_writes() -> [StorageWrite; 3] {
        [
            StorageWrite {
                record: StorageRecord::Session,
                bytes: b"session".to_vec(),
            },
            StorageWrite {
                record: StorageRecord::History,
                bytes: b"history".to_vec(),
            },
            StorageWrite {
                record: StorageRecord::Bookmarks,
                bytes: b"bookmarks".to_vec(),
            },
        ]
    }

    #[test]
    fn bundle_round_trips_all_browser_records() {
        let writes = sample_writes();
        let bundle = encode(&writes).unwrap();
        assert_eq!(decode(&bundle).unwrap(), writes);
    }

    #[test]
    fn bundle_rejects_corruption_trailing_bytes_and_duplicate_records() {
        let writes = sample_writes();
        let bundle = encode(&writes).unwrap();
        let mut corrupt = bundle.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert_eq!(decode(&corrupt), Err(StorageError::Io));

        let mut trailing = bundle;
        trailing.push(0);
        assert_eq!(decode(&trailing), Err(StorageError::Io));

        let duplicate = [writes[0].clone(), writes[0].clone()];
        assert_eq!(encode(&duplicate), Err(StorageError::Io));
    }

    #[test]
    fn bundle_rejects_records_that_do_not_fit_the_guest_vfs_file() {
        let oversized = StorageWrite {
            record: StorageRecord::Session,
            bytes: vec![0; MAX_STORAGE_BUNDLE_BYTES],
        };
        assert_eq!(encode(&[oversized]), Err(StorageError::Capacity));
    }
}
