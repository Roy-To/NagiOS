use alloc::{string::String, vec::Vec};

use nagi_search::semantic::{
    Embedding, EmbeddingPurpose, EmbeddingSpaceId, MAX_SEMANTIC_CHUNK_BYTES,
    MAX_SEMANTIC_EMBEDDING_DIMENSIONS, MAX_SEMANTIC_QUERY_BYTES,
};
use sha2::{Digest, Sha256};

use crate::ProviderError;

pub const QUERY_PREFIX: &str = "query: ";
pub const PASSAGE_PREFIX: &str = "passage: ";

const SPACE_DOMAIN: &[u8] = b"nagi.embedding-space.v1\0";

/// Provider-owned input/output rules around a model-specific inference engine.
/// The engine and tokenizer stay outside this adapter boundary.
pub(crate) struct ProviderBoundary {
    dimensions: usize,
    query_limit: usize,
    passage_limit: usize,
    space: EmbeddingSpaceId,
}

impl ProviderBoundary {
    pub(crate) fn new(
        artifact_sha256: &[u8; 32],
        dimensions: usize,
        query_limit: usize,
        passage_limit: usize,
    ) -> Result<Self, ProviderError> {
        if dimensions == 0 || dimensions > MAX_SEMANTIC_EMBEDDING_DIMENSIONS {
            return Err(ProviderError::InvalidConfig("dimensions"));
        }
        if query_limit == 0 || query_limit > MAX_SEMANTIC_QUERY_BYTES {
            return Err(ProviderError::InvalidConfig("max_query_bytes"));
        }
        if passage_limit == 0 || passage_limit > MAX_SEMANTIC_CHUNK_BYTES {
            return Err(ProviderError::InvalidConfig("max_passage_bytes"));
        }

        Ok(Self {
            dimensions,
            query_limit,
            passage_limit,
            space: derive_space_id(artifact_sha256, dimensions),
        })
    }

    pub(crate) const fn space_id(&self) -> EmbeddingSpaceId {
        self.space
    }

    /// Reject a configured identity that cannot consume vectors from this
    /// artifact and the existing v1 embedding contract.
    pub(crate) fn validate_expected_space(
        &self,
        expected: Option<EmbeddingSpaceId>,
    ) -> Result<(), ProviderError> {
        if let Some(expected) = expected {
            if expected != self.space {
                return Err(ProviderError::SpaceMismatch {
                    expected,
                    actual: self.space,
                });
            }
        }
        Ok(())
    }

    /// Add the model's task prefix after validating the raw UTF-8 payload.
    /// The payload is otherwise passed through byte-for-byte.
    pub(crate) fn prepare(
        &self,
        purpose: EmbeddingPurpose,
        text: &str,
    ) -> Result<String, ProviderError> {
        if text.trim().is_empty() {
            return Err(ProviderError::EmptyInput);
        }

        let (prefix, limit) = match purpose {
            EmbeddingPurpose::Query => (QUERY_PREFIX, self.query_limit),
            EmbeddingPurpose::Passage => (PASSAGE_PREFIX, self.passage_limit),
        };
        if text.len() > limit {
            return Err(ProviderError::InputTooLong {
                bytes: text.len(),
                limit,
            });
        }

        let mut input = String::with_capacity(prefix.len() + text.len());
        input.push_str(prefix);
        input.push_str(text);
        Ok(input)
    }

    /// Enforce the model's declared output width, then use the shared
    /// constructor for finite, non-zero validation and L2 normalization.
    pub(crate) fn finish(&self, values: Vec<f32>) -> Result<Embedding, ProviderError> {
        if values.len() != self.dimensions {
            return Err(ProviderError::InvalidOutput);
        }
        Embedding::try_from_values_in_space(values, self.space)
            .map_err(|_| ProviderError::InvalidOutput)
    }
}

/// Stable vector-space identity. The byte sequence follows the existing
/// v1 descriptor so unchanged vector semantics retain their saved index ID.
pub(crate) fn derive_space_id(artifact_sha256: &[u8; 32], dimensions: usize) -> EmbeddingSpaceId {
    let mut descriptor = Vec::new();
    descriptor.extend_from_slice(SPACE_DOMAIN);
    descriptor.extend_from_slice(artifact_sha256);
    descriptor.extend_from_slice(&(dimensions as u32).to_le_bytes());
    descriptor.extend_from_slice(b"pooling=mean\0normalize=l2\0");
    descriptor.extend_from_slice(QUERY_PREFIX.as_bytes());
    descriptor.push(0);
    descriptor.extend_from_slice(PASSAGE_PREFIX.as_bytes());
    descriptor.push(0);
    EmbeddingSpaceId(Sha256::digest(&descriptor).into())
}

#[cfg(test)]
mod tests {
    use super::ProviderBoundary;
    use crate::{space_id_for, ProviderError};
    use nagi_search::semantic::{
        EmbeddingPurpose, EmbeddingSpaceId, MAX_SEMANTIC_CHUNK_BYTES,
        MAX_SEMANTIC_EMBEDDING_DIMENSIONS, MAX_SEMANTIC_QUERY_BYTES,
    };

    fn boundary(query_limit: usize, passage_limit: usize) -> ProviderBoundary {
        ProviderBoundary::new(&[0x2a; 32], 2, query_limit, passage_limit).unwrap()
    }

    #[test]
    fn purpose_selects_exact_prefix_and_limits_raw_utf8_bytes() {
        let adapter = boundary(2, 4);
        assert_eq!(
            adapter.prepare(EmbeddingPurpose::Query, "é").unwrap(),
            "query: é"
        );
        assert_eq!(
            adapter.prepare(EmbeddingPurpose::Passage, "é").unwrap(),
            "passage: é"
        );
        assert_eq!(
            adapter.prepare(EmbeddingPurpose::Query, "éx").unwrap_err(),
            ProviderError::InputTooLong { bytes: 3, limit: 2 }
        );
        assert_eq!(
            adapter
                .prepare(EmbeddingPurpose::Passage, "éxyz")
                .unwrap_err(),
            ProviderError::InputTooLong { bytes: 5, limit: 4 }
        );
    }

    #[test]
    fn empty_unicode_whitespace_is_rejected_without_prefixing() {
        let adapter = boundary(16, 16);
        assert_eq!(
            adapter
                .prepare(EmbeddingPurpose::Query, "　\t\n")
                .unwrap_err(),
            ProviderError::EmptyInput
        );
        assert_eq!(
            adapter.prepare(EmbeddingPurpose::Passage, "").unwrap_err(),
            ProviderError::EmptyInput
        );
    }

    #[test]
    fn caller_text_is_preserved_and_prefixes_are_not_inferred_or_removed() {
        let adapter = boundary(64, 64);
        assert_eq!(
            adapter
                .prepare(EmbeddingPurpose::Query, " passage: value ")
                .unwrap(),
            "query:  passage: value "
        );
        assert_eq!(
            adapter
                .prepare(EmbeddingPurpose::Passage, "query: value")
                .unwrap(),
            "passage: query: value"
        );
    }

    #[test]
    fn query_and_passage_configuration_limits_are_independent_and_bounded() {
        let adapter = boundary(8, 4);
        assert!(adapter.prepare(EmbeddingPurpose::Query, "12345").is_ok());
        assert_eq!(
            adapter
                .prepare(EmbeddingPurpose::Passage, "12345")
                .unwrap_err(),
            ProviderError::InputTooLong { bytes: 5, limit: 4 }
        );

        assert!(ProviderBoundary::new(&[0; 32], 2, 0, 1).is_err());
        assert!(ProviderBoundary::new(&[0; 32], 2, MAX_SEMANTIC_QUERY_BYTES + 1, 1).is_err());
        assert!(ProviderBoundary::new(&[0; 32], 2, 1, MAX_SEMANTIC_CHUNK_BYTES + 1).is_err());
        assert!(ProviderBoundary::new(&[0; 32], 0, 1, 1).is_err());
        assert!(
            ProviderBoundary::new(&[0; 32], MAX_SEMANTIC_EMBEDDING_DIMENSIONS + 1, 1, 1).is_err()
        );
    }

    #[test]
    fn output_must_match_model_dimensions_and_is_normalized_in_its_space() {
        let adapter = boundary(16, 16);
        let embedding = adapter.finish(alloc::vec![3.0, 4.0]).unwrap();
        assert_eq!(embedding.dimensions(), 2);
        assert_eq!(embedding.space_id(), Some(adapter.space_id()));
        assert!((embedding.values()[0] - 0.6).abs() < 1e-6);
        assert!((embedding.values()[1] - 0.8).abs() < 1e-6);

        assert_eq!(
            adapter.finish(alloc::vec![1.0]).unwrap_err(),
            ProviderError::InvalidOutput
        );
        assert_eq!(
            adapter.finish(alloc::vec![0.0, 0.0]).unwrap_err(),
            ProviderError::InvalidOutput
        );
        assert_eq!(
            adapter.finish(alloc::vec![f32::NAN, 1.0]).unwrap_err(),
            ProviderError::InvalidOutput
        );
    }

    #[test]
    fn existing_v1_space_fingerprint_remains_stable() {
        let expected = EmbeddingSpaceId([
            0x8b, 0x48, 0xee, 0x4c, 0xa6, 0x8d, 0x2d, 0x54, 0x3d, 0xe5, 0x14, 0xc3, 0x98, 0x00,
            0x27, 0x21, 0x76, 0x30, 0xf3, 0x73, 0xe1, 0x59, 0xaf, 0xbd, 0xba, 0x79, 0x95, 0xfa,
            0x1f, 0x84, 0x7a, 0xd7,
        ]);
        assert_eq!(space_id_for(&[0x2a; 32], 2), expected);
        assert_eq!(boundary(16, 16).space_id(), expected);
    }

    #[test]
    fn the_same_artifact_and_contract_accept_an_existing_space_id() {
        let original = boundary(16, 16);
        let reopened = ProviderBoundary::new(&[0x2a; 32], 2, 16, 16).unwrap();
        assert_eq!(reopened.space_id(), original.space_id());
        assert_eq!(
            reopened.validate_expected_space(Some(original.space_id())),
            Ok(())
        );
    }

    #[test]
    fn a_different_expected_space_is_rejected() {
        let adapter = boundary(16, 16);
        let foreign = EmbeddingSpaceId([0x07; 32]);
        assert_ne!(adapter.space_id(), foreign);
        assert_eq!(
            adapter.validate_expected_space(Some(foreign)),
            Err(ProviderError::SpaceMismatch {
                expected: foreign,
                actual: adapter.space_id(),
            })
        );
    }

    #[test]
    fn compatible_space_identity_changes_with_artifact_or_dimension() {
        let original = boundary(16, 16).space_id();
        let other_artifact = ProviderBoundary::new(&[0x2b; 32], 2, 16, 16)
            .unwrap()
            .space_id();
        let other_dimensions = ProviderBoundary::new(&[0x2a; 32], 3, 16, 16)
            .unwrap()
            .space_id();
        assert_ne!(original, other_artifact);
        assert_ne!(original, other_dimensions);
    }
}
