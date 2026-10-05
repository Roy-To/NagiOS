use alloc::vec::Vec;

use crate::{BackendError, SnapshotBackend};

pub const GUEST_FILE_BYTES: usize = 1024;
const MANIFEST_BYTES: usize = 36;
const MANIFEST_VERSION: u16 = 1;
const MAX_GUEST_CHUNKS: usize = 4;
pub const MAX_GUEST_SNAPSHOT_BYTES: usize = GUEST_FILE_BYTES * MAX_GUEST_CHUNKS;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SnapshotSlot {
    A,
    B,
}

impl SnapshotSlot {
    const fn index(self) -> u8 {
        match self {
            Self::A => 0,
            Self::B => 1,
        }
    }

    const fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SnapshotFile {
    Manifest,
    Chunk(u8),
}

/// File operations supplied by the target VFS adapter. A read returns `None`
/// only for a missing file; I/O errors must remain distinguishable from
/// missing or corrupt snapshot data.
pub trait SnapshotFileStore {
    fn read_file(
        &mut self,
        slot: SnapshotSlot,
        file: SnapshotFile,
        buffer: &mut [u8; GUEST_FILE_BYTES],
    ) -> Result<Option<usize>, BackendError>;
    fn write_file(
        &mut self,
        slot: SnapshotSlot,
        file: SnapshotFile,
        bytes: &[u8],
    ) -> Result<(), BackendError>;
    fn remove_file(&mut self, slot: SnapshotSlot, file: SnapshotFile) -> Result<(), BackendError>;
    fn flush(&mut self) -> Result<(), BackendError>;
}

struct SlotSnapshot {
    generation: u64,
    bytes: Vec<u8>,
}

/// A crash-recoverable bounded snapshot backend for the current Nagi VFS.
///
/// It keeps two generations in separate file slots. Chunk files are written
/// and flushed before the slot manifest is replaced; the manifest is the
/// commit record. If the newest slot is incomplete or corrupt, loading falls
/// back to the older verified slot. The current VFS stores one 1 KiB block
/// per file and has a small inode table, so this adapter deliberately caps a
/// snapshot at 4 KiB even though the general search contract permits larger
/// host snapshots.
pub struct GuestSnapshotBackend<F: SnapshotFileStore> {
    files: F,
}

impl<F: SnapshotFileStore> GuestSnapshotBackend<F> {
    pub const fn new(files: F) -> Self {
        Self { files }
    }

    pub fn into_file_store(self) -> F {
        self.files
    }

    fn inspect_slot(
        &mut self,
        slot: SnapshotSlot,
    ) -> Result<(bool, Option<SlotSnapshot>), BackendError> {
        let Some(manifest) = self.read_file(slot, SnapshotFile::Manifest)? else {
            return Ok((false, None));
        };
        let Ok((generation, length, chunk_count, expected_digest)) =
            decode_slot_manifest(slot.index(), &manifest)
        else {
            return Ok((true, None));
        };
        let mut bytes = Vec::with_capacity(length);
        for index in 0..chunk_count {
            let Some(chunk) = self.read_file(slot, SnapshotFile::Chunk(index as u8))? else {
                return Ok((true, None));
            };
            let expected_length = (length - bytes.len()).min(GUEST_FILE_BYTES);
            if chunk.len() != expected_length {
                return Ok((true, None));
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() != length || checksum(&bytes) != expected_digest {
            return Ok((true, None));
        }
        Ok((true, Some(SlotSnapshot { generation, bytes })))
    }

    fn read_file(
        &mut self,
        slot: SnapshotSlot,
        file: SnapshotFile,
    ) -> Result<Option<Vec<u8>>, BackendError> {
        let mut buffer = [0; GUEST_FILE_BYTES];
        let Some(length) = self.files.read_file(slot, file, &mut buffer)? else {
            return Ok(None);
        };
        if length > GUEST_FILE_BYTES {
            return Err(BackendError::Io);
        }
        Ok(Some(buffer[..length].into()))
    }

    fn write_file(
        &mut self,
        slot: SnapshotSlot,
        file: SnapshotFile,
        bytes: &[u8],
    ) -> Result<(), BackendError> {
        if bytes.len() > GUEST_FILE_BYTES {
            return Err(BackendError::SnapshotTooLarge);
        }
        self.files.write_file(slot, file, bytes)
    }

    fn remove_file_if_present(
        &mut self,
        slot: SnapshotSlot,
        file: SnapshotFile,
    ) -> Result<(), BackendError> {
        self.files.remove_file(slot, file)
    }
}

impl<F: SnapshotFileStore> SnapshotBackend for GuestSnapshotBackend<F> {
    fn load_snapshot(&mut self) -> Result<Option<Vec<u8>>, BackendError> {
        let (a_present, a) = self.inspect_slot(SnapshotSlot::A)?;
        let (b_present, b) = self.inspect_slot(SnapshotSlot::B)?;
        let selected = match (a, b) {
            (Some(a), Some(b)) if b.generation > a.generation => Some(b),
            (Some(a), Some(_)) => Some(a),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) if a_present || b_present => return Err(BackendError::Io),
            (None, None) => return Ok(None),
        };
        Ok(selected.map(|snapshot| snapshot.bytes))
    }

    fn write_snapshot(&mut self, snapshot: &[u8]) -> Result<(), BackendError> {
        if snapshot.len() > MAX_GUEST_SNAPSHOT_BYTES {
            return Err(BackendError::SnapshotTooLarge);
        }
        let (a_present, a) = self.inspect_slot(SnapshotSlot::A)?;
        let (b_present, b) = self.inspect_slot(SnapshotSlot::B)?;
        let (active_slot, generation) = match (a, b) {
            (Some(a), Some(b)) if b.generation > a.generation => (SnapshotSlot::B, b.generation),
            (Some(a), Some(b)) => (SnapshotSlot::A, a.generation.max(b.generation)),
            (Some(a), None) => (SnapshotSlot::A, a.generation),
            (None, Some(b)) => (SnapshotSlot::B, b.generation),
            (None, None) if a_present || b_present => return Err(BackendError::Io),
            (None, None) => (SnapshotSlot::B, 0),
        };
        let generation = generation.checked_add(1).ok_or(BackendError::Io)?;
        let target_slot = active_slot.other();
        let chunk_count = snapshot.len().div_ceil(GUEST_FILE_BYTES);

        for index in 0..MAX_GUEST_CHUNKS {
            let file = SnapshotFile::Chunk(index as u8);
            if index < chunk_count {
                let start = index * GUEST_FILE_BYTES;
                let end = (start + GUEST_FILE_BYTES).min(snapshot.len());
                self.write_file(target_slot, file, &snapshot[start..end])?;
            } else {
                self.remove_file_if_present(target_slot, file)?;
            }
        }
        self.files.flush()?;

        let manifest = encode_slot_manifest(target_slot.index(), generation, snapshot)?;
        self.write_file(target_slot, SnapshotFile::Manifest, &manifest)?;
        self.files.flush()
    }
}

fn encode_slot_manifest(
    slot: u8,
    generation: u64,
    snapshot: &[u8],
) -> Result<[u8; MANIFEST_BYTES], BackendError> {
    if slot > 1 || snapshot.len() > MAX_GUEST_SNAPSHOT_BYTES {
        return Err(BackendError::SnapshotTooLarge);
    }
    let chunk_count = snapshot.len().div_ceil(GUEST_FILE_BYTES);
    let mut bytes = [0; MANIFEST_BYTES];
    bytes[..4].copy_from_slice(b"NSG1");
    bytes[4..6].copy_from_slice(&MANIFEST_VERSION.to_le_bytes());
    bytes[6] = slot;
    bytes[7] = chunk_count as u8;
    bytes[8..16].copy_from_slice(&generation.to_le_bytes());
    bytes[16..20].copy_from_slice(&(snapshot.len() as u32).to_le_bytes());
    bytes[20..28].copy_from_slice(&checksum(snapshot).to_le_bytes());
    let manifest_checksum = checksum(&bytes[..28]);
    bytes[28..36].copy_from_slice(&manifest_checksum.to_le_bytes());
    Ok(bytes)
}

fn decode_slot_manifest(slot: u8, bytes: &[u8]) -> Result<(u64, usize, usize, u64), BackendError> {
    if bytes.len() != MANIFEST_BYTES
        || bytes.get(..4) != Some(b"NSG1")
        || read_u16(bytes, 4)? != MANIFEST_VERSION
        || *bytes.get(6).ok_or(BackendError::Io)? != slot
        || read_u64(bytes, 28)? != checksum(&bytes[..28])
    {
        return Err(BackendError::Io);
    }
    let generation = read_u64(bytes, 8)?;
    let length = usize::try_from(read_u32(bytes, 16)?).map_err(|_| BackendError::Io)?;
    let chunk_count = usize::from(*bytes.get(7).ok_or(BackendError::Io)?);
    if generation == 0
        || length > MAX_GUEST_SNAPSHOT_BYTES
        || chunk_count != length.div_ceil(GUEST_FILE_BYTES)
        || chunk_count > MAX_GUEST_CHUNKS
    {
        return Err(BackendError::Io);
    }
    Ok((generation, length, chunk_count, read_u64(bytes, 20)?))
}

fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x1000_0000_01b3)
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, BackendError> {
    let bytes = bytes.get(offset..offset + 2).ok_or(BackendError::Io)?;
    Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, BackendError> {
    let bytes = bytes.get(offset..offset + 4).ok_or(BackendError::Io)?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, BackendError> {
    let bytes = bytes.get(offset..offset + 8).ok_or(BackendError::Io)?;
    Ok(u64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}

#[cfg(test)]
mod tests {
    use alloc::{collections::BTreeMap, sync::Arc, vec, vec::Vec};
    use std::sync::Mutex;

    use crate::{
        guest::{
            GuestSnapshotBackend, SnapshotFile, SnapshotFileStore, SnapshotSlot, GUEST_FILE_BYTES,
        },
        BackendError, SnapshotBackend,
    };

    #[derive(Clone, Default)]
    struct MemoryFiles(Arc<Mutex<MemoryState>>);

    #[derive(Default)]
    struct MemoryState {
        files: BTreeMap<(SnapshotSlot, SnapshotFile), Vec<u8>>,
        fail_next_manifest_write: bool,
    }

    impl MemoryFiles {
        fn corrupt(&self, slot: SnapshotSlot, byte_index: usize) {
            self.0
                .lock()
                .expect("memory files")
                .files
                .get_mut(&(slot, SnapshotFile::Manifest))
                .expect("manifest")
                .get_mut(byte_index)
                .map(|byte| *byte ^= 1)
                .expect("manifest byte");
        }

        fn fail_next_manifest_write(&self) {
            self.0
                .lock()
                .expect("memory files")
                .fail_next_manifest_write = true;
        }
    }

    impl SnapshotFileStore for MemoryFiles {
        fn read_file(
            &mut self,
            slot: SnapshotSlot,
            file: SnapshotFile,
            buffer: &mut [u8; GUEST_FILE_BYTES],
        ) -> Result<Option<usize>, BackendError> {
            let state = self.0.lock().map_err(|_| BackendError::Io)?;
            let Some(bytes) = state.files.get(&(slot, file)) else {
                return Ok(None);
            };
            if bytes.len() > buffer.len() {
                return Err(BackendError::Io);
            }
            buffer[..bytes.len()].copy_from_slice(bytes);
            Ok(Some(bytes.len()))
        }

        fn write_file(
            &mut self,
            slot: SnapshotSlot,
            file: SnapshotFile,
            bytes: &[u8],
        ) -> Result<(), BackendError> {
            let mut state = self.0.lock().map_err(|_| BackendError::Io)?;
            if file == SnapshotFile::Manifest && state.fail_next_manifest_write {
                state.fail_next_manifest_write = false;
                return Err(BackendError::Io);
            }
            state.files.insert((slot, file), bytes.to_vec());
            Ok(())
        }

        fn remove_file(
            &mut self,
            slot: SnapshotSlot,
            file: SnapshotFile,
        ) -> Result<(), BackendError> {
            self.0
                .lock()
                .map_err(|_| BackendError::Io)?
                .files
                .remove(&(slot, file));
            Ok(())
        }

        fn flush(&mut self) -> Result<(), BackendError> {
            Ok(())
        }
    }

    #[test]
    fn two_slot_snapshot_recovers_previous_generation_after_corruption() {
        let files = MemoryFiles::default();
        let mut backend = GuestSnapshotBackend::new(files.clone());
        let first = b"first persisted search index";
        let second = vec![0x5a; 2500];
        assert_eq!(backend.load_snapshot().unwrap(), None);
        backend.write_snapshot(first).unwrap();
        backend.write_snapshot(&second).unwrap();

        let mut remounted = GuestSnapshotBackend::new(files.clone());
        assert_eq!(remounted.load_snapshot().unwrap(), Some(second.clone()));
        files.corrupt(SnapshotSlot::B, 20);

        let mut recovered = GuestSnapshotBackend::new(files);
        assert_eq!(recovered.load_snapshot().unwrap(), Some(first.to_vec()));
    }

    #[test]
    fn failed_manifest_commit_keeps_the_previous_snapshot_recoverable() {
        let files = MemoryFiles::default();
        let mut backend = GuestSnapshotBackend::new(files.clone());
        let first = b"first generation";
        let second = b"second generation";
        let attempted = b"uncommitted third generation";
        backend.write_snapshot(first).unwrap();
        backend.write_snapshot(second).unwrap();
        files.fail_next_manifest_write();
        assert_eq!(backend.write_snapshot(attempted), Err(BackendError::Io));

        let mut recovered = GuestSnapshotBackend::new(files);
        assert_eq!(recovered.load_snapshot().unwrap(), Some(second.to_vec()));
    }

    #[test]
    fn guest_snapshot_limit_does_not_replace_the_last_valid_generation() {
        let files = MemoryFiles::default();
        let mut backend = GuestSnapshotBackend::new(files.clone());
        let valid = b"still recoverable";
        backend.write_snapshot(valid).unwrap();
        assert_eq!(
            backend.write_snapshot(&vec![0; super::MAX_GUEST_SNAPSHOT_BYTES + 1]),
            Err(BackendError::SnapshotTooLarge)
        );
        let mut remounted = GuestSnapshotBackend::new(files);
        assert_eq!(remounted.load_snapshot().unwrap(), Some(valid.to_vec()));
    }
}
