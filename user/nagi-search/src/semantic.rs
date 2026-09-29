//! Bounded, provider-neutral contracts for semantic content indexing.
//!
//! This module deliberately contains no embedding implementation. Model
//! providers produce normalized vectors, while vector-index implementations
//! own their persistence and approximate-search strategy.

use alloc::{string::String, vec::Vec};

use nagi_model::ObjectId;

pub const DEFAULT_SEMANTIC_CHUNK_BYTES: usize = 512;
pub const MIN_SEMANTIC_CHUNK_BYTES: usize = 4;
pub const MAX_SEMANTIC_CHUNK_BYTES: usize = 2048;
pub const MAX_SEMANTIC_SOURCE_BYTES: usize = 256 * 1024;
pub const MAX_SEMANTIC_CHUNKS_PER_OBJECT: usize = 512;
pub const MAX_SEMANTIC_EMBEDDING_DIMENSIONS: usize = 4096;
pub const MAX_SEMANTIC_QUERY_BYTES: usize = 4096;
pub const MAX_SEMANTIC_RESULTS: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SemanticError {
    EmptyText,
    QueryTooLong,
    SourceTooLong,
    InvalidChunkSize,
    TooManyChunks,
    InvalidEmbedding,
    DimensionMismatch,
    InvalidIndexResult,
    InvalidLimit,
    ObjectNotVisible,
    ProviderUnavailable,
    IndexUnavailable,
    IndexCapacity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmbeddingPurpose {
    Query,
    Passage,
}

/// A finite, non-zero, unit-length vector. Construction normalizes provider
/// output once so indexes can use cosine similarity consistently.
#[derive(Clone, Debug, PartialEq)]
pub struct Embedding {
    values: Vec<f32>,
}

impl Embedding {
    pub fn try_from_values(mut values: Vec<f32>) -> Result<Self, SemanticError> {
        if values.is_empty()
            || values.len() > MAX_SEMANTIC_EMBEDDING_DIMENSIONS
            || values.iter().any(|value| !value.is_finite())
        {
            return Err(SemanticError::InvalidEmbedding);
        }

        let mut scale = 0.0_f64;
        for value in &values {
            let magnitude = f64::from(*value).abs();
            if magnitude > scale {
                scale = magnitude;
            }
        }
        if scale == 0.0 || !scale.is_finite() {
            return Err(SemanticError::InvalidEmbedding);
        }
        // Scaling first keeps the sum in [1, dimensions], avoiding overflow
        // for large but finite provider values. The bounded Newton iteration
        // avoids requiring a host-only libm sqrt in the no_std guest build.
        let mut scaled_squared_norm = 0.0_f64;
        for value in &values {
            let scaled = f64::from(*value) / scale;
            scaled_squared_norm += scaled * scaled;
        }
        let scaled_norm = bounded_sqrt(scaled_squared_norm);
        let norm = scale * scaled_norm;
        for value in &mut values {
            *value = (f64::from(*value) / norm) as f32;
        }
        Ok(Self { values })
    }

    pub fn values(&self) -> &[f32] {
        &self.values
    }

    pub fn dimensions(&self) -> usize {
        self.values.len()
    }
}

fn bounded_sqrt(value: f64) -> f64 {
    debug_assert!((1.0..=MAX_SEMANTIC_EMBEDDING_DIMENSIONS as f64).contains(&value));
    let mut estimate = value;
    for _ in 0..16 {
        estimate = 0.5 * (estimate + value / estimate);
    }
    estimate
}

/// A byte-exact UTF-8 chunk from one producer-owned object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextChunk {
    pub object_id: ObjectId,
    pub ordinal: u16,
    pub start_byte: u32,
    pub end_byte: u32,
    pub text: String,
}

/// Split UTF-8 text without cutting a code point. The algorithm prefers
/// paragraph, sentence, then whitespace boundaries near the byte limit and
/// falls back to a UTF-8 boundary for scripts without spaces.
pub fn chunk_text(object_id: ObjectId, text: &str) -> Result<Vec<TextChunk>, SemanticError> {
    chunk_text_with_limit(object_id, text, DEFAULT_SEMANTIC_CHUNK_BYTES)
}

pub fn chunk_text_with_limit(
    object_id: ObjectId,
    text: &str,
    max_chunk_bytes: usize,
) -> Result<Vec<TextChunk>, SemanticError> {
    if !(MIN_SEMANTIC_CHUNK_BYTES..=MAX_SEMANTIC_CHUNK_BYTES).contains(&max_chunk_bytes) {
        return Err(SemanticError::InvalidChunkSize);
    }
    if text.len() > MAX_SEMANTIC_SOURCE_BYTES {
        return Err(SemanticError::SourceTooLong);
    }
    if text.is_empty() {
        return Ok(Vec::new());
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < text.len() {
        if chunks.len() >= MAX_SEMANTIC_CHUNKS_PER_OBJECT {
            return Err(SemanticError::TooManyChunks);
        }

        let hard_end = utf8_boundary_at_or_before(text, start.saturating_add(max_chunk_bytes));
        let end = if hard_end <= start {
            text[start..]
                .char_indices()
                .nth(1)
                .map_or(text.len(), |(offset, _)| start + offset)
        } else {
            preferred_boundary(text, start, hard_end)
        };
        debug_assert!(end > start);
        debug_assert!(text.is_char_boundary(end));

        let ordinal = u16::try_from(chunks.len()).map_err(|_| SemanticError::TooManyChunks)?;
        let start_byte = u32::try_from(start).map_err(|_| SemanticError::SourceTooLong)?;
        let end_byte = u32::try_from(end).map_err(|_| SemanticError::SourceTooLong)?;
        chunks.push(TextChunk {
            object_id,
            ordinal,
            start_byte,
            end_byte,
            text: text[start..end].into(),
        });
        start = end;
    }
    Ok(chunks)
}

fn utf8_boundary_at_or_before(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn preferred_boundary(text: &str, start: usize, hard_end: usize) -> usize {
    let soft_start = start + (hard_end - start) / 2;
    let mut last_paragraph = None;
    let mut last_sentence = None;
    let mut last_whitespace = None;

    for (relative, character) in text[start..hard_end].char_indices() {
        let end = start + relative + character.len_utf8();
        if end < soft_start {
            continue;
        }
        if character == '\n' {
            last_paragraph = Some(end);
        } else if matches!(character, '.' | '!' | '?' | '。' | '！' | '？') {
            last_sentence = Some(end);
        } else if character.is_whitespace() {
            last_whitespace = Some(end);
        }
    }

    last_paragraph
        .or(last_sentence)
        .or(last_whitespace)
        .unwrap_or(hard_end)
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexedChunk {
    pub chunk: TextChunk,
    pub embedding: Embedding,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VectorMatch {
    pub object_id: ObjectId,
    pub chunk_ordinal: u16,
    pub start_byte: u32,
    pub end_byte: u32,
    /// Cosine similarity in the inclusive range [-1, 1].
    pub similarity: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VectorIndexError {
    Unavailable,
    Capacity,
    DimensionMismatch,
}

/// A replaceable inference boundary. A multilingual model may translate the
/// purpose into model-specific query/passage prefixes internally.
pub trait EmbeddingProvider {
    fn embed(&self, purpose: EmbeddingPurpose, text: &str) -> Result<Embedding, SemanticError>;
}

/// A persistence/search boundary for semantic vectors. `replace_object` must
/// replace all chunks for an ObjectId atomically. `search` must use the
/// supplied allowlist before returning candidates, return at most `limit`
/// unique ObjectIds, and use normalized cosine similarity.
pub trait VectorIndex {
    fn dimensions(&self) -> Option<usize>;

    fn replace_object(
        &mut self,
        object_id: ObjectId,
        chunks: &[IndexedChunk],
    ) -> Result<(), VectorIndexError>;

    fn remove_object(&mut self, object_id: ObjectId) -> Result<(), VectorIndexError>;

    fn search(
        &self,
        query: &Embedding,
        permitted_objects: &[ObjectId],
        limit: usize,
    ) -> Result<Vec<VectorMatch>, VectorIndexError>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticHit {
    pub record: crate::MetadataRecord,
    pub chunk_ordinal: u16,
    pub start_byte: u32,
    pub end_byte: u32,
    pub similarity: f32,
}

#[cfg(test)]
mod tests {
    use alloc::string::String;

    use super::*;

    #[test]
    fn chunks_multilingual_text_at_sentence_or_utf8_boundaries() {
        let source = "日本語の段落です。次の段落もここにあります。最後の段落です。";
        let chunks = chunk_text_with_limit(ObjectId(7), source, 24).expect("chunks");
        assert!(chunks.len() > 1);
        assert_eq!(
            chunks
                .iter()
                .map(|chunk| chunk.text.as_str())
                .collect::<String>(),
            source
        );
        for (index, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.ordinal as usize, index);
            assert!(chunk.text.len() <= 24);
            assert_eq!(
                chunk.start_byte as usize,
                chunks[..index].iter().map(|part| part.text.len()).sum()
            );
            assert_eq!(chunk.end_byte - chunk.start_byte, chunk.text.len() as u32);
        }
    }

    #[test]
    fn chunking_rejects_unbounded_input_and_invalid_limits() {
        let too_long = "x".repeat(MAX_SEMANTIC_SOURCE_BYTES + 1);
        assert_eq!(
            chunk_text(ObjectId(1), &too_long),
            Err(SemanticError::SourceTooLong)
        );
        assert_eq!(
            chunk_text_with_limit(ObjectId(1), "text", MAX_SEMANTIC_CHUNK_BYTES + 1),
            Err(SemanticError::InvalidChunkSize)
        );
        assert_eq!(
            chunk_text_with_limit(ObjectId(1), "日本語", MIN_SEMANTIC_CHUNK_BYTES - 1),
            Err(SemanticError::InvalidChunkSize)
        );
        assert!(chunk_text(ObjectId(1), "").expect("empty text").is_empty());
    }

    #[test]
    fn embedding_normalization_rejects_invalid_vectors() {
        let embedding = Embedding::try_from_values(alloc::vec![3.0, 4.0]).expect("finite vector");
        assert_eq!(embedding.values(), [0.6, 0.8]);
        assert_eq!(
            Embedding::try_from_values(alloc::vec![0.0, 0.0]),
            Err(SemanticError::InvalidEmbedding)
        );
        assert_eq!(
            Embedding::try_from_values(alloc::vec![f32::NAN]),
            Err(SemanticError::InvalidEmbedding)
        );
    }
}
