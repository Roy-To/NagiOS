//! Production semantic staging calls with the shared host allocator meter.
//! Chunking, provider internals and persistent-index allocations are separate boundaries.
use super::*;
use crate::semantic::{Embedding, EmbeddingPurpose, SemanticError, TextChunk};

struct Provider<'a> {
    expected: &'a [TextChunk],
    calls: Cell<usize>,
    fail_at: Option<usize>,
    mismatch_at: Option<usize>,
}
impl EmbeddingProvider for Provider<'_> {
    fn embed(&self, purpose: EmbeddingPurpose, text: &str) -> Result<Embedding, SemanticError> {
        let call = self.calls.get();
        self.calls.set(call + 1);
        assert_eq!(purpose, EmbeddingPurpose::Passage);
        assert_eq!(text, self.expected[call].text);
        if self.fail_at == Some(call) {
            return Err(SemanticError::ProviderUnavailable);
        }
        Embedding::try_from_values(if self.mismatch_at == Some(call) {
            vec![1.0, 0.0, 0.0]
        } else {
            vec![1.0, 0.0]
        })
    }
}
struct Index<'a> {
    expected: &'a [TextChunk],
    dimensions: Option<usize>,
    dimension_calls: Cell<usize>,
    replacements: usize,
}
impl VectorIndex for Index<'_> {
    fn dimensions(&self) -> Option<usize> {
        self.dimension_calls.set(self.dimension_calls.get() + 1);
        self.dimensions
    }
    fn replace_object(
        &mut self,
        object_id: ObjectId,
        chunks: &[IndexedChunk],
    ) -> Result<(), VectorIndexError> {
        self.replacements += 1;
        assert_eq!(object_id, ObjectId(1));
        assert_eq!(chunks.len(), self.expected.len());
        for (actual, expected) in chunks.iter().zip(self.expected) {
            assert_eq!(&actual.chunk, expected);
            assert_eq!(actual.embedding.values(), &[1.0, 0.0]);
        }
        Ok(())
    }
    fn remove_object(&mut self, _: ObjectId) -> Result<(), VectorIndexError> {
        panic!("indexing must not remove before replacement")
    }
    fn search(
        &self,
        _: &Embedding,
        _: &[ObjectId],
        _: usize,
    ) -> Result<Vec<VectorMatch>, VectorIndexError> {
        panic!("indexing must not query the index")
    }
}
fn boundaries(expected: &[TextChunk], dimensions: Option<usize>) -> (Provider<'_>, Index<'_>) {
    (
        Provider {
            expected,
            calls: Cell::new(0),
            fail_at: None,
            mismatch_at: None,
        },
        Index {
            expected,
            dimensions,
            dimension_calls: Cell::new(0),
            replacements: 0,
        },
    )
}
fn source() -> String {
    "日本語の段落です。English passage.\n".repeat(60)
}

#[test]
fn staging_allocation_failure_precedes_provider_and_index_and_releases_chunks() {
    let service = fixture();
    let text = source();
    // Measure the unchanged chunker separately to target only the next allocation:
    // the staging Vec. Do not inject into its infallible allocations.
    let (chunks, chunk_meter, baseline) = measured(0, false, || chunk_text(ObjectId(1), &text));
    let expected = chunks.unwrap();
    assert!(expected.len() > 2);
    let stage_ordinal = chunk_meter.calls + 1;
    drop(expected);
    assert_eq!(live(), baseline);
    let expected = chunk_text(ObjectId(1), &text).unwrap();
    for persistent in [false, true] {
        let (provider, mut index) = boundaries(&expected, None);
        let (result, meter, baseline) = measured(stage_ordinal, persistent, || {
            service.index_semantic_text(ACCESS, ObjectId(1), &text, &provider, &mut index)
        });
        assert_eq!(result, Err(SemanticError::IndexCapacity));
        assert_eq!(meter.calls, stage_ordinal);
        assert_eq!(meter.denials, 1);
        assert_eq!(provider.calls.get(), 0);
        assert_eq!(index.dimension_calls.get(), 0);
        assert_eq!(index.replacements, 0);
        assert_eq!(live(), baseline, "chunk array and strings must be released");
        assert_eq!(
            service.index_semantic_text(ACCESS, ObjectId(1), &text, &provider, &mut index),
            Ok(expected.len())
        );
        assert_eq!(index.replacements, 1);
    }
    std::println!(
        "staging allocation ordinal {stage_ordinal}; {} chunks; both denial modes cleaned up",
        expected.len()
    );
}

#[test]
fn semantic_staging_preserves_multilingual_order_ranges_vectors_and_count() {
    let service = fixture();
    for text in [source(), "x".repeat(MAX_SEMANTIC_SOURCE_BYTES)] {
        let expected = chunk_text(ObjectId(1), &text).unwrap();
        for dimensions in [None, Some(2)] {
            let (provider, mut index) = boundaries(&expected, dimensions);
            let (result, meter, baseline) = measured(0, false, || {
                service.index_semantic_text(ACCESS, ObjectId(1), &text, &provider, &mut index)
            });
            assert_eq!(result, Ok(expected.len()));
            assert_eq!(provider.calls.get(), expected.len());
            assert_eq!(index.dimension_calls.get(), 1);
            assert_eq!(index.replacements, 1);
            assert_eq!(meter.denials, 0);
            assert_eq!(live(), baseline);
        }
    }
}

#[test]
fn semantic_staging_mid_failure_skips_replace_and_drops_partial_embeddings() {
    let service = fixture();
    let text = source();
    let expected = chunk_text(ObjectId(1), &text).unwrap();
    for at in 0..expected.len() {
        for mismatch in [false, true] {
            for dimensions in [None, Some(2)] {
                let (mut provider, mut index) = boundaries(&expected, dimensions);
                if mismatch {
                    provider.mismatch_at = Some(at);
                } else {
                    provider.fail_at = Some(at);
                }
                let (result, _, baseline) = measured(0, false, || {
                    service.index_semantic_text(ACCESS, ObjectId(1), &text, &provider, &mut index)
                });
                assert_eq!(
                    result,
                    Err(if mismatch {
                        SemanticError::DimensionMismatch
                    } else {
                        SemanticError::ProviderUnavailable
                    })
                );
                // With no initial dimensions, a different first vector establishes
                // the dimension and is rejected on the following normal vector.
                let calls = if mismatch && dimensions.is_none() && at == 0 {
                    2
                } else {
                    at + 1
                };
                assert_eq!(provider.calls.get(), calls);
                assert_eq!(index.replacements, 0);
                assert_eq!(
                    live(),
                    baseline,
                    "partial staging and embeddings must be freed"
                );
            }
        }
    }
}

#[test]
fn semantic_staging_keeps_authorization_before_allocation_or_provider() {
    let service = fixture();
    for (access, id) in [
        (ACCESS, 3),
        (ACCESS, 4),
        (ACCESS, 99),
        (AccessContext::anonymous(), 1),
    ] {
        let (provider, mut index) = boundaries(&[], None);
        let (result, meter, baseline) = measured(1, true, || {
            service.index_semantic_text(access, ObjectId(id), "private text", &provider, &mut index)
        });
        assert_eq!(result, Err(SemanticError::ObjectNotVisible));
        assert_eq!(meter.calls, 0);
        assert_eq!(provider.calls.get(), 0);
        assert_eq!(index.dimension_calls.get(), 0);
        assert_eq!(index.replacements, 0);
        assert_eq!(live(), baseline);
    }
}
