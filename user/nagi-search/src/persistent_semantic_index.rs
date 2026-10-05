//! Bounded durable exact-search index for provider-produced embeddings.
//!
//! The snapshot stores ObjectIds, byte ranges, and vectors only. Source text
//! stays with its producer and is never copied into this index.

use alloc::{
    collections::{BTreeMap, BTreeSet},
    vec::Vec,
};
use nagi_model::ObjectId;

use crate::{
    semantic::{
        Embedding, EmbeddingSpaceId, IndexedChunk, VectorIndex, VectorIndexError, VectorMatch,
        MAX_SEMANTIC_CHUNKS_PER_OBJECT, MAX_SEMANTIC_EMBEDDING_DIMENSIONS, MAX_SEMANTIC_RESULTS,
        MAX_SEMANTIC_SOURCE_BYTES,
    },
    BackendError, SnapshotBackend,
};

const SNAPSHOT_MAGIC: &[u8; 4] = b"NVS1";
const SNAPSHOT_VERSION: u16 = 1;
const HEADER_BYTES: usize = 52;
const CHECKSUM_BYTES: usize = 8;
const MAX_INDEXED_CHUNKS: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistentVectorIndexError {
    Backend(BackendError),
    CorruptSnapshot,
    UnsupportedVersion(u16),
}

#[derive(Clone)]
struct StoredChunk {
    ordinal: u16,
    start_byte: u32,
    end_byte: u32,
    embedding: Embedding,
}

#[derive(Clone, Default)]
struct IndexState {
    space_id: Option<EmbeddingSpaceId>,
    dimensions: Option<usize>,
    chunk_count: usize,
    objects: BTreeMap<ObjectId, Vec<StoredChunk>>,
}

/// A deterministic linear-scan index with complete-snapshot persistence.
///
/// The backend is expected to provide an atomic snapshot commit. The index
/// commits a cloned candidate state first and publishes it in memory only
/// after persistence succeeds.
pub struct PersistentVectorIndex<B: SnapshotBackend> {
    backend: B,
    state: IndexState,
}

impl<B: SnapshotBackend> PersistentVectorIndex<B> {
    pub fn open(mut backend: B) -> Result<Self, PersistentVectorIndexError> {
        let state = match backend
            .load_snapshot()
            .map_err(PersistentVectorIndexError::Backend)?
        {
            Some(snapshot) => decode_snapshot(&snapshot)?,
            None => IndexState::default(),
        };
        Ok(Self { backend, state })
    }

    fn commit(&mut self, next: IndexState) -> Result<(), VectorIndexError> {
        let snapshot = encode_snapshot(&next).map_err(|_| VectorIndexError::Capacity)?;
        self.backend
            .write_snapshot(&snapshot)
            .map_err(|error| match error {
                BackendError::Io => VectorIndexError::Unavailable,
                BackendError::SnapshotTooLarge => VectorIndexError::Capacity,
            })?;
        self.state = next;
        Ok(())
    }
}

impl<B: SnapshotBackend> VectorIndex for PersistentVectorIndex<B> {
    fn dimensions(&self) -> Option<usize> {
        self.state.dimensions
    }

    fn replace_object(
        &mut self,
        object_id: ObjectId,
        chunks: &[IndexedChunk],
    ) -> Result<(), VectorIndexError> {
        if chunks.is_empty() {
            return self.remove_object(object_id);
        }
        if chunks.len() > MAX_SEMANTIC_CHUNKS_PER_OBJECT {
            return Err(VectorIndexError::Capacity);
        }

        let (space_id, dimensions) = validate_chunks(object_id, chunks)?;
        let mut next = self.state.clone();
        let removed_chunks = next
            .objects
            .remove(&object_id)
            .map_or(0, |previous| previous.len());
        next.chunk_count = next
            .chunk_count
            .checked_sub(removed_chunks)
            .ok_or(VectorIndexError::Unavailable)?;

        if !next.objects.is_empty() {
            if next.space_id != Some(space_id) {
                return Err(VectorIndexError::EmbeddingSpaceMismatch);
            }
            if next.dimensions != Some(dimensions) {
                return Err(VectorIndexError::DimensionMismatch);
            }
        }

        next.chunk_count = next
            .chunk_count
            .checked_add(chunks.len())
            .ok_or(VectorIndexError::Capacity)?;
        if next.chunk_count > MAX_INDEXED_CHUNKS
            || next.objects.len() >= crate::store::MAX_OBJECT_RECORDS
        {
            return Err(VectorIndexError::Capacity);
        }
        next.space_id = Some(space_id);
        next.dimensions = Some(dimensions);
        next.objects.insert(
            object_id,
            chunks
                .iter()
                .map(|indexed| StoredChunk {
                    ordinal: indexed.chunk.ordinal,
                    start_byte: indexed.chunk.start_byte,
                    end_byte: indexed.chunk.end_byte,
                    embedding: indexed.embedding.clone(),
                })
                .collect(),
        );
        self.commit(next)
    }

    fn remove_object(&mut self, object_id: ObjectId) -> Result<(), VectorIndexError> {
        if !self.state.objects.contains_key(&object_id) {
            return Ok(());
        }
        let mut next = self.state.clone();
        if let Some(chunks) = next.objects.remove(&object_id) {
            next.chunk_count = next
                .chunk_count
                .checked_sub(chunks.len())
                .ok_or(VectorIndexError::Unavailable)?;
        }
        if next.objects.is_empty() {
            next.space_id = None;
            next.dimensions = None;
        }
        self.commit(next)
    }

    fn search(
        &self,
        query: &Embedding,
        permitted_objects: &[ObjectId],
        limit: usize,
    ) -> Result<Vec<VectorMatch>, VectorIndexError> {
        if limit == 0 || limit > MAX_SEMANTIC_RESULTS {
            return Err(VectorIndexError::InvalidLimit);
        }
        let query_space = query
            .space_id()
            .ok_or(VectorIndexError::InvalidEmbeddingSpace)?;
        if let Some(index_space) = self.state.space_id {
            if query_space != index_space {
                return Err(VectorIndexError::EmbeddingSpaceMismatch);
            }
            if query.dimensions() != self.state.dimensions.unwrap_or_default() {
                return Err(VectorIndexError::DimensionMismatch);
            }
        }

        if permitted_objects.len() > crate::store::MAX_OBJECT_RECORDS {
            return Err(VectorIndexError::Capacity);
        }
        let permitted: BTreeSet<ObjectId> = permitted_objects.iter().copied().collect();
        let mut matches = Vec::new();
        for (object_id, chunks) in &self.state.objects {
            if !permitted.contains(object_id) {
                continue;
            }
            let mut best: Option<VectorMatch> = None;
            for chunk in chunks {
                let candidate = VectorMatch {
                    object_id: *object_id,
                    chunk_ordinal: chunk.ordinal,
                    start_byte: chunk.start_byte,
                    end_byte: chunk.end_byte,
                    similarity: cosine_similarity(query, &chunk.embedding),
                };
                let replace = best.is_none_or(|current| {
                    candidate.similarity > current.similarity
                        || (candidate.similarity == current.similarity
                            && candidate.chunk_ordinal < current.chunk_ordinal)
                });
                if replace {
                    best = Some(candidate);
                }
            }
            if let Some(best) = best {
                matches.push(best);
            }
        }
        matches.sort_by(|left, right| {
            right
                .similarity
                .total_cmp(&left.similarity)
                .then_with(|| left.object_id.cmp(&right.object_id))
                .then_with(|| left.chunk_ordinal.cmp(&right.chunk_ordinal))
        });
        matches.truncate(limit);
        Ok(matches)
    }
}

fn validate_chunks(
    object_id: ObjectId,
    chunks: &[IndexedChunk],
) -> Result<(EmbeddingSpaceId, usize), VectorIndexError> {
    let first = chunks
        .first()
        .ok_or(VectorIndexError::InvalidChunkMetadata)?;
    let space_id = first
        .embedding
        .space_id()
        .ok_or(VectorIndexError::InvalidEmbeddingSpace)?;
    let dimensions = first.embedding.dimensions();
    let mut expected_start = 0_u32;
    for (index, indexed) in chunks.iter().enumerate() {
        if indexed.chunk.object_id != object_id
            || usize::from(indexed.chunk.ordinal) != index
            || indexed.chunk.start_byte != expected_start
            || indexed.chunk.start_byte >= indexed.chunk.end_byte
            || (indexed.chunk.end_byte - indexed.chunk.start_byte) as usize
                != indexed.chunk.text.len()
            || indexed.chunk.end_byte as usize > MAX_SEMANTIC_SOURCE_BYTES
        {
            return Err(VectorIndexError::InvalidChunkMetadata);
        }
        if indexed.embedding.space_id().is_none() {
            return Err(VectorIndexError::InvalidEmbeddingSpace);
        }
        if indexed.embedding.space_id() != Some(space_id) {
            return Err(VectorIndexError::EmbeddingSpaceMismatch);
        }
        if indexed.embedding.dimensions() != dimensions {
            return Err(VectorIndexError::DimensionMismatch);
        }
        expected_start = indexed.chunk.end_byte;
    }
    Ok((space_id, dimensions))
}

fn cosine_similarity(left: &Embedding, right: &Embedding) -> f32 {
    let dot = left
        .values()
        .iter()
        .zip(right.values())
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum::<f64>();
    dot.clamp(-1.0, 1.0) as f32
}

fn encode_snapshot(state: &IndexState) -> Result<Vec<u8>, ()> {
    if state.objects.len() > crate::store::MAX_OBJECT_RECORDS
        || state.chunk_count > MAX_INDEXED_CHUNKS
    {
        return Err(());
    }
    let (dimensions, space_id) = if state.objects.is_empty() {
        (0_usize, None)
    } else {
        (state.dimensions.ok_or(())?, Some(state.space_id.ok_or(())?))
    };
    if dimensions > MAX_SEMANTIC_EMBEDDING_DIMENSIONS {
        return Err(());
    }

    let mut payload = Vec::new();
    for (object_id, chunks) in &state.objects {
        if chunks.is_empty() || chunks.len() > MAX_SEMANTIC_CHUNKS_PER_OBJECT {
            return Err(());
        }
        payload.extend_from_slice(&object_id.0.to_le_bytes());
        payload.extend_from_slice(&u16::try_from(chunks.len()).map_err(|_| ())?.to_le_bytes());
        for chunk in chunks {
            if chunk.embedding.dimensions() != dimensions || chunk.embedding.space_id() != space_id
            {
                return Err(());
            }
            payload.extend_from_slice(&chunk.ordinal.to_le_bytes());
            payload.extend_from_slice(&chunk.start_byte.to_le_bytes());
            payload.extend_from_slice(&chunk.end_byte.to_le_bytes());
            for value in chunk.embedding.values() {
                payload.extend_from_slice(&value.to_bits().to_le_bytes());
            }
            if payload.len() > crate::store::MAX_SNAPSHOT_BYTES {
                return Err(());
            }
        }
    }
    if state.objects.values().map(Vec::len).sum::<usize>() != state.chunk_count {
        return Err(());
    }

    let mut bytes = Vec::with_capacity(HEADER_BYTES + payload.len() + CHECKSUM_BYTES);
    bytes.extend_from_slice(SNAPSHOT_MAGIC);
    bytes.extend_from_slice(&SNAPSHOT_VERSION.to_le_bytes());
    bytes.extend_from_slice(&u16::try_from(dimensions).map_err(|_| ())?.to_le_bytes());
    bytes.extend_from_slice(
        &u32::try_from(state.objects.len())
            .map_err(|_| ())?
            .to_le_bytes(),
    );
    bytes.extend_from_slice(
        &u32::try_from(state.chunk_count)
            .map_err(|_| ())?
            .to_le_bytes(),
    );
    bytes.extend_from_slice(&u32::try_from(payload.len()).map_err(|_| ())?.to_le_bytes());
    bytes.extend_from_slice(&space_id.map_or([0; 32], |space| space.0));
    debug_assert_eq!(bytes.len(), HEADER_BYTES);
    bytes.extend_from_slice(&payload);
    bytes.extend_from_slice(&checksum(&bytes).to_le_bytes());
    if bytes.len() > crate::store::MAX_SNAPSHOT_BYTES {
        return Err(());
    }
    Ok(bytes)
}

fn decode_snapshot(bytes: &[u8]) -> Result<IndexState, PersistentVectorIndexError> {
    if bytes.len() > crate::store::MAX_SNAPSHOT_BYTES
        || bytes.len() < HEADER_BYTES + CHECKSUM_BYTES
        || bytes.get(..4) != Some(SNAPSHOT_MAGIC)
    {
        return Err(PersistentVectorIndexError::CorruptSnapshot);
    }
    let version = read_u16(bytes, 4).ok_or(PersistentVectorIndexError::CorruptSnapshot)?;
    if version != SNAPSHOT_VERSION {
        return Err(PersistentVectorIndexError::UnsupportedVersion(version));
    }
    let dimensions =
        usize::from(read_u16(bytes, 6).ok_or(PersistentVectorIndexError::CorruptSnapshot)?);
    let object_count =
        usize::try_from(read_u32(bytes, 8).ok_or(PersistentVectorIndexError::CorruptSnapshot)?)
            .map_err(|_| PersistentVectorIndexError::CorruptSnapshot)?;
    let chunk_count =
        usize::try_from(read_u32(bytes, 12).ok_or(PersistentVectorIndexError::CorruptSnapshot)?)
            .map_err(|_| PersistentVectorIndexError::CorruptSnapshot)?;
    let payload_length =
        usize::try_from(read_u32(bytes, 16).ok_or(PersistentVectorIndexError::CorruptSnapshot)?)
            .map_err(|_| PersistentVectorIndexError::CorruptSnapshot)?;
    let mut raw_space_id = [0; 32];
    raw_space_id.copy_from_slice(
        bytes
            .get(20..52)
            .ok_or(PersistentVectorIndexError::CorruptSnapshot)?,
    );
    let expected_length = HEADER_BYTES
        .checked_add(payload_length)
        .and_then(|length| length.checked_add(CHECKSUM_BYTES))
        .ok_or(PersistentVectorIndexError::CorruptSnapshot)?;
    if expected_length != bytes.len()
        || object_count > crate::store::MAX_OBJECT_RECORDS
        || chunk_count > MAX_INDEXED_CHUNKS
        || dimensions > MAX_SEMANTIC_EMBEDDING_DIMENSIONS
        || (chunk_count == 0 && (object_count != 0 || dimensions != 0 || raw_space_id != [0; 32]))
        || (chunk_count > 0
            && (object_count == 0 || dimensions == 0 || dimensions > u16::MAX as usize))
    {
        return Err(PersistentVectorIndexError::CorruptSnapshot);
    }
    let checksum_offset = bytes.len() - CHECKSUM_BYTES;
    let stored_checksum =
        read_u64(bytes, checksum_offset).ok_or(PersistentVectorIndexError::CorruptSnapshot)?;
    if stored_checksum != checksum(&bytes[..checksum_offset]) {
        return Err(PersistentVectorIndexError::CorruptSnapshot);
    }

    if chunk_count == 0 {
        if payload_length != 0 {
            return Err(PersistentVectorIndexError::CorruptSnapshot);
        }
        return Ok(IndexState::default());
    }

    let space_id = EmbeddingSpaceId(raw_space_id);
    let payload_end = HEADER_BYTES + payload_length;
    let mut cursor = HEADER_BYTES;
    let mut objects = BTreeMap::new();
    let mut observed_chunks = 0_usize;
    for _ in 0..object_count {
        let object_id =
            ObjectId(read_u64(bytes, cursor).ok_or(PersistentVectorIndexError::CorruptSnapshot)?);
        cursor = cursor
            .checked_add(8)
            .ok_or(PersistentVectorIndexError::CorruptSnapshot)?;
        let per_object_count = usize::from(
            read_u16(bytes, cursor).ok_or(PersistentVectorIndexError::CorruptSnapshot)?,
        );
        cursor = cursor
            .checked_add(2)
            .ok_or(PersistentVectorIndexError::CorruptSnapshot)?;
        if per_object_count == 0 || per_object_count > MAX_SEMANTIC_CHUNKS_PER_OBJECT {
            return Err(PersistentVectorIndexError::CorruptSnapshot);
        }
        let mut chunks = Vec::with_capacity(per_object_count);
        for expected_ordinal in 0..per_object_count {
            let ordinal =
                read_u16(bytes, cursor).ok_or(PersistentVectorIndexError::CorruptSnapshot)?;
            cursor += 2;
            let start_byte =
                read_u32(bytes, cursor).ok_or(PersistentVectorIndexError::CorruptSnapshot)?;
            cursor += 4;
            let end_byte =
                read_u32(bytes, cursor).ok_or(PersistentVectorIndexError::CorruptSnapshot)?;
            cursor += 4;
            let vector_bytes = dimensions
                .checked_mul(4)
                .ok_or(PersistentVectorIndexError::CorruptSnapshot)?;
            let vector_end = cursor
                .checked_add(vector_bytes)
                .filter(|end| *end <= payload_end)
                .ok_or(PersistentVectorIndexError::CorruptSnapshot)?;
            let mut values = Vec::with_capacity(dimensions);
            while cursor < vector_end {
                let bits =
                    read_u32(bytes, cursor).ok_or(PersistentVectorIndexError::CorruptSnapshot)?;
                cursor += 4;
                values.push(f32::from_bits(bits));
            }
            let embedding = Embedding::try_from_values_in_space(values.clone(), space_id)
                .map_err(|_| PersistentVectorIndexError::CorruptSnapshot)?;
            if usize::from(ordinal) != expected_ordinal
                || embedding
                    .values()
                    .iter()
                    .zip(values.iter())
                    .any(|(normalized, stored)| (normalized - stored).abs() > 0.002)
            {
                return Err(PersistentVectorIndexError::CorruptSnapshot);
            }
            chunks.push(StoredChunk {
                ordinal,
                start_byte,
                end_byte,
                embedding,
            });
        }
        validate_stored_chunks(&chunks).map_err(|_| PersistentVectorIndexError::CorruptSnapshot)?;
        observed_chunks = observed_chunks
            .checked_add(chunks.len())
            .ok_or(PersistentVectorIndexError::CorruptSnapshot)?;
        if observed_chunks > MAX_INDEXED_CHUNKS || objects.insert(object_id, chunks).is_some() {
            return Err(PersistentVectorIndexError::CorruptSnapshot);
        }
    }
    if cursor != payload_end || observed_chunks != chunk_count {
        return Err(PersistentVectorIndexError::CorruptSnapshot);
    }
    Ok(IndexState {
        space_id: Some(space_id),
        dimensions: Some(dimensions),
        chunk_count,
        objects,
    })
}

fn validate_stored_chunks(chunks: &[StoredChunk]) -> Result<(), VectorIndexError> {
    if chunks.is_empty() || chunks.len() > MAX_SEMANTIC_CHUNKS_PER_OBJECT {
        return Err(VectorIndexError::InvalidChunkMetadata);
    }
    let space_id = chunks[0]
        .embedding
        .space_id()
        .ok_or(VectorIndexError::InvalidEmbeddingSpace)?;
    let dimensions = chunks[0].embedding.dimensions();
    let mut expected_start = 0_u32;
    for (index, chunk) in chunks.iter().enumerate() {
        if usize::from(chunk.ordinal) != index
            || chunk.start_byte != expected_start
            || chunk.start_byte >= chunk.end_byte
            || chunk.end_byte as usize > MAX_SEMANTIC_SOURCE_BYTES
        {
            return Err(VectorIndexError::InvalidChunkMetadata);
        }
        if chunk.embedding.space_id() != Some(space_id) {
            return Err(VectorIndexError::EmbeddingSpaceMismatch);
        }
        if chunk.embedding.dimensions() != dimensions {
            return Err(VectorIndexError::DimensionMismatch);
        }
        expected_start = chunk.end_byte;
    }
    Ok(())
}

fn checksum(bytes: &[u8]) -> u64 {
    // FNV-1a detects accidental snapshot damage; it is not an authority or
    // authenticity mechanism.
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x1000_0000_01b3)
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
    ))
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{
        semantic::{SemanticError, TextChunk},
        BackendError,
    };

    #[derive(Clone, Default)]
    struct MemoryBackend {
        shared: Arc<Mutex<MemoryState>>,
    }

    #[derive(Default)]
    struct MemoryState {
        snapshot: Option<Vec<u8>>,
        fail_write: bool,
    }

    impl SnapshotBackend for MemoryBackend {
        fn load_snapshot(&mut self) -> Result<Option<Vec<u8>>, BackendError> {
            Ok(self.shared.lock().expect("lock").snapshot.clone())
        }

        fn write_snapshot(&mut self, snapshot: &[u8]) -> Result<(), BackendError> {
            let mut state = self.shared.lock().expect("lock");
            if state.fail_write {
                return Err(BackendError::Io);
            }
            state.snapshot = Some(snapshot.to_vec());
            Ok(())
        }
    }

    const SPACE_A: EmbeddingSpaceId = EmbeddingSpaceId([0x24; 32]);
    const SPACE_B: EmbeddingSpaceId = EmbeddingSpaceId([0x26; 32]);

    fn indexed(
        object_id: ObjectId,
        ordinal: u16,
        start_byte: u32,
        end_byte: u32,
        values: Vec<f32>,
        space: EmbeddingSpaceId,
    ) -> IndexedChunk {
        IndexedChunk {
            chunk: TextChunk {
                object_id,
                ordinal,
                start_byte,
                end_byte,
                text: "x".repeat((end_byte - start_byte) as usize),
            },
            embedding: Embedding::try_from_values_in_space(values, space).expect("embedding"),
        }
    }

    fn open_index(backend: MemoryBackend) -> PersistentVectorIndex<MemoryBackend> {
        PersistentVectorIndex::open(backend).expect("index")
    }

    #[test]
    fn snapshots_reopen_and_search_exactly_with_visible_object_allowlist() {
        let backend = MemoryBackend::default();
        let mut index = open_index(backend.clone());
        index
            .replace_object(
                ObjectId(1),
                &[indexed(ObjectId(1), 0, 0, 12, vec![2.0, 0.0], SPACE_A)],
            )
            .expect("replace first");
        index
            .replace_object(
                ObjectId(2),
                &[indexed(ObjectId(2), 0, 0, 9, vec![0.0, 3.0], SPACE_A)],
            )
            .expect("replace second");

        let query = Embedding::try_from_values_in_space(vec![1.0, 0.0], SPACE_A).expect("query");
        let permitted = [ObjectId(2), ObjectId(1)];
        let found = index.search(&query, &permitted, 2).expect("search");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].object_id, ObjectId(1));
        assert!((found[0].similarity - 1.0).abs() < 0.0001);
        assert_eq!(found[1].object_id, ObjectId(2));

        let reopened = open_index(backend);
        assert_eq!(reopened.dimensions(), Some(2));
        let hidden = reopened
            .search(&query, &[ObjectId(2)], 2)
            .expect("filtered search");
        assert_eq!(hidden.len(), 1);
        assert_eq!(hidden[0].object_id, ObjectId(2));
    }

    #[test]
    fn removing_last_object_clears_embedding_space() {
        let mut index = open_index(MemoryBackend::default());
        index
            .replace_object(
                ObjectId(1),
                &[indexed(ObjectId(1), 0, 0, 4, vec![1.0, 0.0], SPACE_A)],
            )
            .expect("replace");
        index.remove_object(ObjectId(1)).expect("remove last");
        assert_eq!(index.dimensions(), None);
        index
            .replace_object(
                ObjectId(2),
                &[indexed(ObjectId(2), 0, 0, 4, vec![0.0, 1.0, 0.0], SPACE_B)],
            )
            .expect("new space after removal");
        assert_eq!(index.dimensions(), Some(3));
    }

    #[test]
    fn corrupt_and_unsupported_snapshots_are_rejected() {
        let backend = MemoryBackend::default();
        let mut index = open_index(backend.clone());
        index
            .replace_object(
                ObjectId(1),
                &[indexed(ObjectId(1), 0, 0, 4, vec![1.0, 0.0], SPACE_A)],
            )
            .expect("replace");
        {
            let mut state = backend.shared.lock().expect("lock");
            state.snapshot.as_mut().expect("snapshot")[53] ^= 0x80;
        }
        assert_eq!(
            PersistentVectorIndex::open(backend.clone()).err(),
            Some(PersistentVectorIndexError::CorruptSnapshot)
        );
        {
            let mut state = backend.shared.lock().expect("lock");
            let bytes = state.snapshot.as_mut().expect("snapshot");
            bytes[4..6].copy_from_slice(&2_u16.to_le_bytes());
            let new_checksum = checksum(&bytes[..bytes.len() - CHECKSUM_BYTES]);
            let checksum_offset = bytes.len() - CHECKSUM_BYTES;
            bytes[checksum_offset..].copy_from_slice(&new_checksum.to_le_bytes());
        }
        assert_eq!(
            PersistentVectorIndex::open(backend).err(),
            Some(PersistentVectorIndexError::UnsupportedVersion(2))
        );
    }

    #[test]
    fn failed_snapshot_write_keeps_previous_in_memory_and_durable_state() {
        let backend = MemoryBackend::default();
        let mut index = open_index(backend.clone());
        index
            .replace_object(
                ObjectId(1),
                &[indexed(ObjectId(1), 0, 0, 4, vec![1.0, 0.0], SPACE_A)],
            )
            .expect("initial replace");
        let old_snapshot = backend.shared.lock().expect("lock").snapshot.clone();
        backend.shared.lock().expect("lock").fail_write = true;
        assert_eq!(
            index.replace_object(
                ObjectId(2),
                &[indexed(ObjectId(2), 0, 0, 4, vec![0.0, 1.0], SPACE_A)]
            ),
            Err(VectorIndexError::Unavailable)
        );
        assert_eq!(index.state.objects.len(), 1);
        assert_eq!(backend.shared.lock().expect("lock").snapshot, old_snapshot);
        backend.shared.lock().expect("lock").fail_write = false;
        let reopened = open_index(backend);
        assert_eq!(reopened.state.objects.len(), 1);
    }

    #[test]
    fn rejects_invalid_metadata_unspecified_space_and_incompatible_vectors() {
        let mut index = open_index(MemoryBackend::default());
        let gap = [
            indexed(ObjectId(1), 0, 0, 4, vec![1.0, 0.0], SPACE_A),
            indexed(ObjectId(1), 1, 5, 9, vec![1.0, 0.0], SPACE_A),
        ];
        assert_eq!(
            index.replace_object(ObjectId(1), &gap),
            Err(VectorIndexError::InvalidChunkMetadata)
        );
        let unspecified = IndexedChunk {
            embedding: Embedding::try_from_values(vec![1.0, 0.0]).expect("embedding"),
            ..indexed(ObjectId(1), 0, 0, 4, vec![1.0, 0.0], SPACE_A)
        };
        assert_eq!(
            index.replace_object(ObjectId(1), &[unspecified]),
            Err(VectorIndexError::InvalidEmbeddingSpace)
        );
        index
            .replace_object(
                ObjectId(1),
                &[indexed(ObjectId(1), 0, 0, 4, vec![1.0, 0.0], SPACE_A)],
            )
            .expect("replace first");
        assert_eq!(
            index.replace_object(
                ObjectId(2),
                &[indexed(ObjectId(2), 0, 0, 4, vec![1.0, 0.0], SPACE_B)]
            ),
            Err(VectorIndexError::EmbeddingSpaceMismatch)
        );
        assert_eq!(
            index.replace_object(
                ObjectId(2),
                &[indexed(ObjectId(2), 0, 0, 4, vec![1.0, 0.0, 0.0], SPACE_A)]
            ),
            Err(VectorIndexError::DimensionMismatch)
        );
        let mismatched_object = indexed(ObjectId(2), 0, 0, 4, vec![1.0, 0.0], SPACE_A);
        assert_eq!(
            index.replace_object(ObjectId(1), &[mismatched_object]),
            Err(VectorIndexError::InvalidChunkMetadata)
        );
        let query = Embedding::try_from_values_in_space(vec![1.0, 0.0], SPACE_B).expect("query");
        assert_eq!(
            index.search(&query, &[ObjectId(1)], 1),
            Err(VectorIndexError::EmbeddingSpaceMismatch)
        );
        let bad_query = Embedding::try_from_values(vec![1.0, 0.0]).expect("query");
        assert_eq!(
            index.search(&bad_query, &[ObjectId(1)], 1),
            Err(VectorIndexError::InvalidEmbeddingSpace)
        );
        let good_query =
            Embedding::try_from_values_in_space(vec![1.0, 0.0], SPACE_A).expect("query");
        assert_eq!(
            index.search(&good_query, &[ObjectId(1)], 0),
            Err(VectorIndexError::InvalidLimit)
        );
        assert_eq!(
            SemanticError::InvalidEmbedding,
            Embedding::try_from_values(vec![f32::NAN]).unwrap_err()
        );
    }

    #[test]
    fn returns_one_best_chunk_per_object_with_stable_ties() {
        let mut index = open_index(MemoryBackend::default());
        let object_one = [
            indexed(ObjectId(1), 0, 0, 5, vec![1.0, 0.0], SPACE_A),
            indexed(ObjectId(1), 1, 5, 10, vec![0.0, 1.0], SPACE_A),
        ];
        index
            .replace_object(ObjectId(1), &object_one)
            .expect("replace one");
        index
            .replace_object(
                ObjectId(2),
                &[indexed(ObjectId(2), 0, 0, 5, vec![1.0, 0.0], SPACE_A)],
            )
            .expect("replace two");
        let query = Embedding::try_from_values_in_space(vec![1.0, 0.0], SPACE_A).expect("query");
        let found = index
            .search(&query, &[ObjectId(2), ObjectId(1), ObjectId(1)], 10)
            .expect("search");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].object_id, ObjectId(1));
        assert_eq!(found[0].chunk_ordinal, 0);
        assert_eq!(found[1].object_id, ObjectId(2));
        assert_eq!(found[1].chunk_ordinal, 0);
    }
}
