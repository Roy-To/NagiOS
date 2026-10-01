use crate::{HistoryError, HistoryService};

pub const GUEST_ARCHIVE_FILE_BYTES: usize = 1024;
const GUEST_ARCHIVE_HEADER_BYTES: usize = 36;
pub const MAX_GUEST_ARCHIVE_BYTES: usize = GUEST_ARCHIVE_FILE_BYTES - GUEST_ARCHIVE_HEADER_BYTES;
const GUEST_ARCHIVE_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ArchiveSlot {
    A,
    B,
}

impl ArchiveSlot {
    pub(crate) const fn index(self) -> u8 {
        match self {
            Self::A => 0,
            Self::B => 1,
        }
    }

    pub(crate) const fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

/// The target adapter supplies single-file operations backed by guest VFS.
/// Every slot fits one VFS block so a partially written new slot cannot
/// destroy the previously committed archive.
pub trait HistoryArchiveFileStore {
    fn read_file(
        &mut self,
        slot: ArchiveSlot,
        buffer: &mut [u8; GUEST_ARCHIVE_FILE_BYTES],
    ) -> Result<Option<usize>, HistoryError>;

    fn write_file(&mut self, slot: ArchiveSlot, bytes: &[u8]) -> Result<(), HistoryError>;

    fn flush(&mut self) -> Result<(), HistoryError>;
}

/// Crash-recoverable two-slot storage for the bounded `NH16` archive.
pub struct HistoryArchiveBackend<F: HistoryArchiveFileStore> {
    files: F,
}

impl<F: HistoryArchiveFileStore> HistoryArchiveBackend<F> {
    pub const fn new(files: F) -> Self {
        Self { files }
    }

    pub fn file_store_mut(&mut self) -> &mut F {
        &mut self.files
    }

    pub fn into_file_store(self) -> F {
        self.files
    }

    fn inspect_slot(
        &mut self,
        slot: ArchiveSlot,
    ) -> Result<(bool, Option<SlotInfo>), HistoryError> {
        let mut bytes = [0; GUEST_ARCHIVE_FILE_BYTES];
        let Some(length) = self.files.read_file(slot, &mut bytes)? else {
            return Ok((false, None));
        };
        if length > bytes.len() {
            return Ok((true, None));
        }
        Ok((true, decode_slot(slot, &bytes[..length])))
    }
}

/// Implementations expose only complete, checksum-verified recoverable
/// archives. Writes target the inactive slot and flush it before returning.
pub trait HistoryArchiveStore {
    fn load_archive(&mut self, output: &mut [u8]) -> Result<Option<usize>, HistoryError>;
    fn write_archive(&mut self, archive: &[u8]) -> Result<(), HistoryError>;
}

impl<F: HistoryArchiveFileStore> HistoryArchiveStore for HistoryArchiveBackend<F> {
    fn load_archive(&mut self, output: &mut [u8]) -> Result<Option<usize>, HistoryError> {
        let (a_present, a) = self.inspect_slot(ArchiveSlot::A)?;
        let (b_present, b) = self.inspect_slot(ArchiveSlot::B)?;
        let selected = match (a, b) {
            (Some(a), Some(b)) if b.generation > a.generation => Some((ArchiveSlot::B, b)),
            (Some(a), Some(_)) => Some((ArchiveSlot::A, a)),
            (Some(a), None) => Some((ArchiveSlot::A, a)),
            (None, Some(b)) => Some((ArchiveSlot::B, b)),
            (None, None) if a_present || b_present => return Err(HistoryError::CorruptArchive),
            (None, None) => return Ok(None),
        };
        let Some((slot, info)) = selected else {
            return Ok(None);
        };
        if output.len() < info.archive_length {
            return Err(HistoryError::BufferTooSmall);
        }
        let mut bytes = [0; GUEST_ARCHIVE_FILE_BYTES];
        let length = self
            .files
            .read_file(slot, &mut bytes)?
            .ok_or(HistoryError::CorruptArchive)?;
        if length > bytes.len() {
            return Err(HistoryError::CorruptArchive);
        }
        let Some(decoded) = decode_slot(slot, &bytes[..length]) else {
            return Err(HistoryError::CorruptArchive);
        };
        let archive_start = GUEST_ARCHIVE_HEADER_BYTES;
        let archive_end = archive_start + decoded.archive_length;
        output[..decoded.archive_length].copy_from_slice(&bytes[archive_start..archive_end]);
        Ok(Some(decoded.archive_length))
    }

    fn write_archive(&mut self, archive: &[u8]) -> Result<(), HistoryError> {
        if archive.len() > MAX_GUEST_ARCHIVE_BYTES {
            return Err(HistoryError::Capacity);
        }
        HistoryService::restore_recoverable(archive)?;

        let (a_present, a) = self.inspect_slot(ArchiveSlot::A)?;
        let (b_present, b) = self.inspect_slot(ArchiveSlot::B)?;
        let (active_slot, generation) = match (a, b) {
            (Some(a), Some(b)) if b.generation > a.generation => (ArchiveSlot::B, b.generation),
            (Some(a), Some(_)) => (ArchiveSlot::A, a.generation),
            (Some(a), None) => (ArchiveSlot::A, a.generation),
            (None, Some(b)) => (ArchiveSlot::B, b.generation),
            (None, None) if a_present || b_present => return Err(HistoryError::CorruptArchive),
            (None, None) => (ArchiveSlot::B, 0),
        };
        let generation = generation.checked_add(1).ok_or(HistoryError::Capacity)?;
        let target = active_slot.other();
        let mut bytes = [0; GUEST_ARCHIVE_FILE_BYTES];
        let length = encode_slot(target, generation, archive, &mut bytes)?;
        self.files.write_file(target, &bytes[..length])?;
        self.files.flush()
    }
}

#[derive(Clone, Copy)]
struct SlotInfo {
    generation: u64,
    archive_length: usize,
}

fn encode_slot(
    slot: ArchiveSlot,
    generation: u64,
    archive: &[u8],
    output: &mut [u8; GUEST_ARCHIVE_FILE_BYTES],
) -> Result<usize, HistoryError> {
    if generation == 0 || archive.len() > MAX_GUEST_ARCHIVE_BYTES {
        return Err(HistoryError::Capacity);
    }
    output[..4].copy_from_slice(b"NHA1");
    output[4..6].copy_from_slice(&GUEST_ARCHIVE_VERSION.to_le_bytes());
    output[6] = slot.index();
    output[7] = 0;
    output[8..16].copy_from_slice(&generation.to_le_bytes());
    output[16..20].copy_from_slice(&(archive.len() as u32).to_le_bytes());
    output[20..28].copy_from_slice(&checksum(archive).to_le_bytes());
    let header_checksum = checksum(&output[..28]);
    output[28..36].copy_from_slice(&header_checksum.to_le_bytes());
    let end = GUEST_ARCHIVE_HEADER_BYTES + archive.len();
    output[GUEST_ARCHIVE_HEADER_BYTES..end].copy_from_slice(archive);
    Ok(end)
}

fn decode_slot(slot: ArchiveSlot, bytes: &[u8]) -> Option<SlotInfo> {
    if bytes.len() < GUEST_ARCHIVE_HEADER_BYTES
        || bytes.len() > GUEST_ARCHIVE_FILE_BYTES
        || bytes.get(..4) != Some(b"NHA1")
        || read_u16(bytes, 4)? != GUEST_ARCHIVE_VERSION
        || *bytes.get(6)? != slot.index()
        || *bytes.get(7)? != 0
        || read_u64(bytes, 28)? != checksum(bytes.get(..28)?)
    {
        return None;
    }
    let generation = read_u64(bytes, 8)?;
    let archive_length = usize::try_from(read_u32(bytes, 16)?).ok()?;
    let archive_end = GUEST_ARCHIVE_HEADER_BYTES.checked_add(archive_length)?;
    let archive = bytes.get(GUEST_ARCHIVE_HEADER_BYTES..archive_end)?;
    if generation == 0
        || archive_length > MAX_GUEST_ARCHIVE_BYTES
        || archive_end != bytes.len()
        || read_u64(bytes, 20)? != checksum(archive)
        || HistoryService::restore_recoverable(archive).is_err()
    {
        return None;
    }
    Some(SlotInfo {
        generation,
        archive_length,
    })
}

fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x1000_0000_01b3)
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    let bytes = bytes.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    let bytes = bytes.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let bytes = bytes.get(offset..offset.checked_add(8)?)?;
    Some(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

#[cfg(test)]
mod tests {
    use alloc::{vec, vec::Vec};
    use std::collections::BTreeMap;

    use crate::{
        ActivityContext, AppId, AppSessionId, HistoryError, HistoryService, MoveRecord, NodeId,
        ObjectId,
    };

    use super::{
        ArchiveSlot, HistoryArchiveBackend, HistoryArchiveFileStore, HistoryArchiveStore,
        GUEST_ARCHIVE_FILE_BYTES,
    };

    #[derive(Default)]
    struct MemoryFiles {
        slots: BTreeMap<ArchiveSlot, Vec<u8>>,
    }

    impl HistoryArchiveFileStore for MemoryFiles {
        fn read_file(
            &mut self,
            slot: ArchiveSlot,
            buffer: &mut [u8; GUEST_ARCHIVE_FILE_BYTES],
        ) -> Result<Option<usize>, HistoryError> {
            let Some(bytes) = self.slots.get(&slot) else {
                return Ok(None);
            };
            if bytes.len() > buffer.len() {
                return Ok(Some(usize::MAX));
            }
            buffer[..bytes.len()].copy_from_slice(bytes);
            Ok(Some(bytes.len()))
        }

        fn write_file(&mut self, slot: ArchiveSlot, bytes: &[u8]) -> Result<(), HistoryError> {
            self.slots.insert(slot, bytes.into());
            Ok(())
        }

        fn flush(&mut self) -> Result<(), HistoryError> {
            Ok(())
        }
    }

    fn archive(state: bool) -> Vec<u8> {
        let context = ActivityContext {
            app_id: AppId(1),
            app_session_id: AppSessionId(2),
            node_id: NodeId(3),
            surface_id: None,
            workspace_id: None,
        };
        let mut history = HistoryService::new();
        let transaction = history
            .record_move_group(
                context,
                &[MoveRecord {
                    object_id: ObjectId(4),
                    from_name: b"before",
                    to_name: b"after",
                    source_contents: b"moved file contents",
                }],
            )
            .unwrap();
        if state {
            history.commit_transaction(transaction, context).unwrap();
        }
        let mut bytes = vec![0; crate::MAX_ARCHIVE_BYTES];
        let length = history.serialize_recoverable(&mut bytes).unwrap();
        bytes.truncate(length);
        bytes
    }

    #[test]
    fn two_slot_archive_restores_latest_and_falls_back_after_new_slot_corruption() {
        let mut backend = HistoryArchiveBackend::new(MemoryFiles::default());
        let prepared = archive(false);
        backend.write_archive(&prepared).unwrap();
        let committed = archive(true);
        backend.write_archive(&committed).unwrap();

        let mut restored = [0; GUEST_ARCHIVE_FILE_BYTES];
        let length = backend.load_archive(&mut restored).unwrap().unwrap();
        assert_eq!(&restored[..length], committed);
        assert_eq!(
            HistoryService::restore_recoverable(&restored[..length])
                .unwrap()
                .record_at(0)
                .unwrap()
                .transaction_state,
            crate::TransactionState::Committed
        );

        let newest = backend
            .file_store_mut()
            .slots
            .get_mut(&ArchiveSlot::B)
            .expect("second generation slot");
        newest[20] ^= 1;
        let fallback_length = backend.load_archive(&mut restored).unwrap().unwrap();
        assert_eq!(&restored[..fallback_length], prepared);
        assert_eq!(
            HistoryService::restore_recoverable(&restored[..fallback_length])
                .unwrap()
                .record_at(0)
                .unwrap()
                .transaction_state,
            crate::TransactionState::Prepared
        );
    }

    #[test]
    fn guest_archive_backend_rejects_oversized_and_non_recoverable_data() {
        let mut backend = HistoryArchiveBackend::new(MemoryFiles::default());
        assert_eq!(
            backend.write_archive(&vec![0; GUEST_ARCHIVE_FILE_BYTES]),
            Err(HistoryError::Capacity)
        );
        assert_eq!(
            backend.write_archive(b"NH15"),
            Err(HistoryError::CorruptArchive)
        );
        assert_eq!(backend.load_archive(&mut [0; 32]), Ok(None));
    }
}
