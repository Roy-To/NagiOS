//! M24 embedding provider contract tests that do not need the real model.
//!
//! They exercise loading, validation and failure handling with a tiny
//! synthetic `.nemb` written below. The synthetic artifact is an
//! orchestration fixture only: its vectors are meaningless and NOTHING here
//! counts as real-inference evidence (see `real_inference.rs`).

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use nagi_embedding_provider::{
    space_id_for, to_semantic, CancelFlag, CancelSignal, Checkpoint, Clock, E5Provider, ModelError,
    ProviderConfig, ProviderError, DEFAULT_INFERENCE_BUDGET_NANOS, E5_SMALL_NEMB_SHA256,
};
use nagi_model::ObjectId;
use nagi_search::{
    BackendError, EmbeddingProvider, EmbeddingPurpose, EmbeddingSpaceId, IndexedChunk,
    PersistentVectorIndex, SemanticError, SnapshotBackend, TextChunk, VectorIndex,
    VectorIndexError,
};

const HIDDEN: u32 = 8;
const LAYERS: u32 = 1;
const HEADS: u32 = 2;
const INTER: u32 = 16;
const POSITIONS: u32 = 16;
const PIECES: &[(&str, f32, u8)] = &[
    ("<s>", 0.0, 1),
    ("<pad>", 0.0, 1),
    ("</s>", 0.0, 1),
    ("<unk>", 0.0, 2),
    ("\u{2581}", -2.0, 0),
    ("\u{2581}a", -1.0, 0),
    ("a", -3.0, 0),
    ("b", -3.0, 0),
    ("\u{2581}ab", -1.5, 0),
    ("q", -3.0, 0),
    ("u", -3.0, 0),
    ("e", -3.0, 0),
    ("r", -3.0, 0),
    ("y", -3.0, 0),
    (":", -3.0, 0),
    ("\u{2581}query", -1.0, 0),
    ("\u{2581}passage", -1.0, 0),
];

/// Deterministic pseudo-random weights; `seed` changes every value.
fn weights(count: usize, seed: u32) -> Vec<f32> {
    let mut state = 0x9e37_79b9u32 ^ seed.wrapping_mul(0x85eb_ca6b);
    (0..count)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / u32::MAX as f32) - 0.5
        })
        .collect()
}

fn tiny_artifact(seed: u32) -> Vec<u8> {
    let h = HIDDEN as usize;
    let mut pieces = Vec::new();
    for (text, score, kind) in PIECES {
        pieces.extend_from_slice(&score.to_le_bytes());
        pieces.push(*kind);
        pieces.extend_from_slice(&(text.len() as u16).to_le_bytes());
        pieces.extend_from_slice(text.as_bytes());
    }
    let mut tensors: Vec<(String, Vec<usize>)> = vec![
        (
            "embeddings.word_embeddings.weight".into(),
            vec![PIECES.len(), h],
        ),
        (
            "embeddings.position_embeddings.weight".into(),
            vec![POSITIONS as usize, h],
        ),
        ("embeddings.token_type_embeddings.weight".into(), vec![2, h]),
        ("embeddings.LayerNorm.weight".into(), vec![h]),
        ("embeddings.LayerNorm.bias".into(), vec![h]),
    ];
    for layer in 0..LAYERS {
        let p = format!("encoder.layer.{layer}");
        for (suffix, shape) in [
            ("attention.self.query.weight", vec![h, h]),
            ("attention.self.query.bias", vec![h]),
            ("attention.self.key.weight", vec![h, h]),
            ("attention.self.key.bias", vec![h]),
            ("attention.self.value.weight", vec![h, h]),
            ("attention.self.value.bias", vec![h]),
            ("attention.output.dense.weight", vec![h, h]),
            ("attention.output.dense.bias", vec![h]),
            ("attention.output.LayerNorm.weight", vec![h]),
            ("attention.output.LayerNorm.bias", vec![h]),
            ("intermediate.dense.weight", vec![INTER as usize, h]),
            ("intermediate.dense.bias", vec![INTER as usize]),
            ("output.dense.weight", vec![h, INTER as usize]),
            ("output.dense.bias", vec![h]),
            ("output.LayerNorm.weight", vec![h]),
            ("output.LayerNorm.bias", vec![h]),
        ] {
            tensors.push((format!("{p}.{suffix}"), shape));
        }
    }
    let mut header = Vec::new();
    for value in [
        HIDDEN,
        LAYERS,
        HEADS,
        INTER,
        POSITIONS,
        2,
        PIECES.len() as u32,
        3,
        0,
        2,
        1,
        1,
        1,
        PIECES.len() as u32,
        pieces.len() as u32,
        0,
        tensors.len() as u32,
    ] {
        header.extend_from_slice(&value.to_le_bytes());
    }
    header.extend_from_slice(&1e-12f32.to_le_bytes());
    header.extend_from_slice(&[0x11; 32]);
    header.extend_from_slice(&[0x22; 32]);
    header.extend_from_slice(b"0000000000000000000000000000000000000000");

    let mut out = b"NAGIEMB\0".to_vec();
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&header);
    out.extend_from_slice(&pieces);
    for (index, (name, shape)) in tensors.iter().enumerate() {
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(shape.len() as u32).to_le_bytes());
        for dim in shape {
            out.extend_from_slice(&(*dim as u32).to_le_bytes());
        }
        out.extend_from_slice(&0u32.to_le_bytes());
        while !out.len().is_multiple_of(4) {
            out.push(0);
        }
        let count: usize = shape.iter().product();
        for value in weights(count, seed.wrapping_add(index as u32)) {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out
}

fn unpinned() -> ProviderConfig {
    ProviderConfig {
        expected_artifact_sha256: None,
        ..ProviderConfig::default()
    }
}

fn tiny_provider(seed: u32) -> E5Provider {
    E5Provider::from_artifact(tiny_artifact(seed), unpinned()).expect("tiny artifact loads")
}

#[test]
fn missing_model_is_reported_as_unavailable() {
    let path = std::env::temp_dir().join("nagi-m24-definitely-missing.nemb");
    let _ = std::fs::remove_file(&path);
    let error = E5Provider::from_path(&path, ProviderConfig::default())
        .err()
        .expect("missing model must fail");
    assert_eq!(error, ProviderError::Model(ModelError::Missing));
    assert_eq!(
        to_semantic(EmbeddingPurpose::Query, &error),
        SemanticError::ProviderUnavailable
    );
    assert!(error.to_string().contains("missing"));
}

#[test]
fn artifact_that_is_not_the_pinned_digest_is_rejected() {
    let error = E5Provider::from_artifact(tiny_artifact(1), ProviderConfig::default())
        .err()
        .expect("digest pin must reject");
    match error {
        ProviderError::Model(ModelError::ChecksumMismatch { expected, .. }) => {
            assert_eq!(expected, E5_SMALL_NEMB_SHA256)
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn every_truncation_of_an_artifact_fails_cleanly() {
    let artifact = tiny_artifact(1);
    for len in 0..artifact.len() {
        let result = E5Provider::from_artifact(artifact[..len].to_vec(), unpinned());
        assert!(result.is_err(), "truncated length {len} was accepted");
    }
}

#[test]
fn corrupt_header_fields_fail_cleanly() {
    let base = tiny_artifact(1);
    type Mutation = Box<dyn Fn(&mut Vec<u8>)>;
    let cases: Vec<(&str, Mutation)> = vec![
        ("magic", Box::new(|a: &mut Vec<u8>| a[0] = b'X')),
        ("version", Box::new(|a: &mut Vec<u8>| a[8] = 2)),
        (
            "hidden=0",
            Box::new(|a: &mut Vec<u8>| a[16..20].copy_from_slice(&0u32.to_le_bytes())),
        ),
        (
            "heads",
            Box::new(|a: &mut Vec<u8>| a[24..28].copy_from_slice(&3u32.to_le_bytes())),
        ),
        (
            "layers",
            Box::new(|a: &mut Vec<u8>| a[20..24].copy_from_slice(&2u32.to_le_bytes())),
        ),
        (
            "unk_id",
            Box::new(|a: &mut Vec<u8>| a[44..48].copy_from_slice(&999u32.to_le_bytes())),
        ),
        (
            "pooling",
            Box::new(|a: &mut Vec<u8>| a[60..64].copy_from_slice(&7u32.to_le_bytes())),
        ),
        (
            "eps",
            Box::new(|a: &mut Vec<u8>| a[84..88].copy_from_slice(&f32::NAN.to_le_bytes())),
        ),
        ("trailing", Box::new(|a: &mut Vec<u8>| a.push(0))),
        (
            "nan weight",
            Box::new(|a: &mut Vec<u8>| {
                let n = a.len();
                a[n - 4..].copy_from_slice(&f32::NAN.to_le_bytes());
            }),
        ),
    ];
    for (label, mutate) in cases {
        let mut artifact = base.clone();
        mutate(&mut artifact);
        let result = E5Provider::from_artifact(artifact, unpinned());
        assert!(
            matches!(result, Err(ProviderError::Model(_))),
            "{label}: {:?}",
            result.err()
        );
    }
}

#[test]
fn artifact_size_cap_is_enforced() {
    let config = ProviderConfig {
        max_artifact_bytes: 64,
        ..unpinned()
    };
    assert!(matches!(
        E5Provider::from_artifact(tiny_artifact(1), config),
        Err(ProviderError::Model(ModelError::TooLarge { limit: 64, .. }))
    ));
}

#[test]
fn empty_input_is_rejected() {
    let p = tiny_provider(1);
    for text in ["", " ", "\n\t "] {
        assert_eq!(
            p.try_embed(EmbeddingPurpose::Query, text),
            Err(ProviderError::EmptyInput)
        );
        assert_eq!(
            p.embed(EmbeddingPurpose::Passage, text),
            Err(SemanticError::EmptyText)
        );
    }
}

#[test]
fn over_limit_input_is_rejected_not_truncated() {
    let p = tiny_provider(1);
    let query = "a".repeat(4097);
    assert!(matches!(
        p.try_embed(EmbeddingPurpose::Query, &query),
        Err(ProviderError::InputTooLong {
            bytes: 4097,
            limit: 4096
        })
    ));
    assert_eq!(
        p.embed(EmbeddingPurpose::Query, &query),
        Err(SemanticError::QueryTooLong)
    );
    let passage = "a".repeat(2049);
    assert_eq!(
        p.embed(EmbeddingPurpose::Passage, &passage),
        Err(SemanticError::SourceTooLong)
    );
    // Token cap: the tiny model has 16 positions, so cap at 8 tokens.
    let capped = E5Provider::from_artifact(
        tiny_artifact(1),
        ProviderConfig {
            max_tokens: Some(8),
            ..unpinned()
        },
    )
    .unwrap();
    match capped.try_embed(EmbeddingPurpose::Query, "a b a b a b a b") {
        Err(ProviderError::TooManyTokens { tokens, limit: 8 }) => assert!(tokens > 8),
        other => panic!("expected TooManyTokens, got {other:?}"),
    }
    assert!(capped.try_embed(EmbeddingPurpose::Query, "ab").is_ok());
}

#[test]
fn invalid_config_is_rejected() {
    for config in [
        ProviderConfig {
            max_tokens: Some(2),
            ..unpinned()
        },
        ProviderConfig {
            max_tokens: Some(513),
            ..unpinned()
        },
        ProviderConfig {
            max_inference_nanos: Some(10),
            clock: None,
            ..unpinned()
        },
    ] {
        assert!(matches!(
            E5Provider::from_artifact(tiny_artifact(1), config),
            Err(ProviderError::InvalidConfig(_))
        ));
    }
    // max_tokens above the model's 16 positions.
    assert!(matches!(
        E5Provider::from_artifact(
            tiny_artifact(1),
            ProviderConfig {
                max_tokens: Some(64),
                ..unpinned()
            }
        ),
        Err(ProviderError::InvalidConfig("max_tokens"))
    ));
}

struct StepClock(AtomicU64);

impl Clock for StepClock {
    fn now_nanos(&self) -> u64 {
        self.0.fetch_add(1_000, Ordering::SeqCst)
    }
}

#[test]
fn deadline_is_enforced_from_call_entry() {
    let config = ProviderConfig {
        max_inference_nanos: Some(500),
        clock: Some(Box::new(StepClock(AtomicU64::new(0)))),
        ..unpinned()
    };
    let p = E5Provider::from_artifact(tiny_artifact(1), config).unwrap();
    let error = p.try_embed(EmbeddingPurpose::Query, "ab").unwrap_err();
    // The budget starts at call entry; the first poll (`Start`, before
    // tokenization) already observes the expiry.
    assert_eq!(
        error,
        ProviderError::DeadlineExceeded {
            budget_nanos: 500,
            at: Checkpoint::Start
        }
    );
    assert_eq!(
        to_semantic(EmbeddingPurpose::Query, &error),
        SemanticError::ProviderUnavailable
    );
    let generous = ProviderConfig {
        max_inference_nanos: Some(u64::MAX / 2),
        clock: Some(Box::new(StepClock(AtomicU64::new(0)))),
        ..unpinned()
    };
    let p = E5Provider::from_artifact(tiny_artifact(1), generous).unwrap();
    assert!(p.try_embed(EmbeddingPurpose::Query, "ab").is_ok());
}

#[test]
fn space_id_tracks_the_artifact_and_tags_vectors() {
    let a = tiny_provider(1);
    let b = tiny_provider(2);
    assert_ne!(a.space_id(), b.space_id());
    assert_eq!(a.space_id(), tiny_provider(1).space_id());
    assert_eq!(
        a.space_id(),
        space_id_for(&a.artifact_info().sha256, a.dimensions())
    );
    let v = a.embed(EmbeddingPurpose::Query, "ab").unwrap();
    assert_eq!(v.space_id(), Some(a.space_id()));
    assert_eq!(v.dimensions(), HIDDEN as usize);
    let norm: f32 = v.values().iter().map(|x| x * x).sum();
    assert!((norm - 1.0).abs() < 1e-5);
}

#[test]
fn expected_space_mismatch_fails_load() {
    let config = ProviderConfig {
        expected_space: Some(EmbeddingSpaceId([7; 32])),
        ..unpinned()
    };
    match E5Provider::from_artifact(tiny_artifact(1), config) {
        Err(error @ ProviderError::SpaceMismatch { .. }) => assert_eq!(
            to_semantic(EmbeddingPurpose::Query, &error),
            SemanticError::EmbeddingSpaceMismatch
        ),
        other => panic!("expected SpaceMismatch, got {:?}", other.err()),
    }
    let expected = tiny_provider(1).space_id();
    assert!(E5Provider::from_artifact(
        tiny_artifact(1),
        ProviderConfig {
            expected_space: Some(expected),
            ..unpinned()
        }
    )
    .is_ok());
}

struct MemoryBackend(Option<Vec<u8>>);

impl SnapshotBackend for MemoryBackend {
    fn load_snapshot(&mut self) -> Result<Option<Vec<u8>>, BackendError> {
        Ok(self.0.clone())
    }
    fn write_snapshot(&mut self, snapshot: &[u8]) -> Result<(), BackendError> {
        self.0 = Some(snapshot.to_vec());
        Ok(())
    }
}

#[test]
fn canonical_index_rejects_vectors_from_another_space() {
    let a = tiny_provider(1);
    let b = tiny_provider(2);
    let mut index = PersistentVectorIndex::open(MemoryBackend(None)).unwrap();
    let chunk = TextChunk {
        object_id: ObjectId(1),
        ordinal: 0,
        start_byte: 0,
        end_byte: 2,
        text: "ab".into(),
    };
    index
        .replace_object(
            ObjectId(1),
            &[IndexedChunk {
                embedding: a.embed(EmbeddingPurpose::Passage, "ab").unwrap(),
                chunk: chunk.clone(),
            }],
        )
        .unwrap();
    let foreign = b.embed(EmbeddingPurpose::Query, "ab").unwrap();
    assert_eq!(
        index.search(&foreign, &[ObjectId(1)], 1),
        Err(VectorIndexError::EmbeddingSpaceMismatch)
    );
    assert_eq!(
        index.replace_object(
            ObjectId(2),
            &[IndexedChunk {
                embedding: b.embed(EmbeddingPurpose::Passage, "ab").unwrap(),
                chunk: TextChunk {
                    object_id: ObjectId(2),
                    ..chunk
                },
            }],
        ),
        Err(VectorIndexError::EmbeddingSpaceMismatch)
    );
    let same = a.embed(EmbeddingPurpose::Query, "ab").unwrap();
    assert_eq!(index.search(&same, &[ObjectId(1)], 1).unwrap().len(), 1);
}

#[test]
fn literal_special_token_text_cannot_inject_control_ids() {
    let p = tiny_provider(1);
    let ids = p.tokenize("query: <s></s><pad><unk>");
    assert_eq!(ids.first(), Some(&0));
    assert_eq!(ids.last(), Some(&2));
    assert!(ids[1..ids.len() - 1]
        .iter()
        .all(|id| ![0, 1, 2].contains(id)));
}

/// Cancels on the `fire_at`-th poll (1-based) and counts polls.
struct CancelAt {
    polls: AtomicUsize,
    fire_at: usize,
}

impl CancelSignal for CancelAt {
    fn is_cancelled(&self) -> bool {
        self.polls.fetch_add(1, Ordering::SeqCst) + 1 >= self.fire_at
    }
}

/// Every checkpoint of one call on the tiny model, in poll order.
fn checkpoint_sequence(p: &E5Provider, text: &str) -> Vec<Checkpoint> {
    let mut sequence = Vec::new();
    for fire_at in 1..100 {
        let cancel = CancelAt {
            polls: AtomicUsize::new(0),
            fire_at,
        };
        match p.try_embed_cancellable(EmbeddingPurpose::Query, text, &cancel) {
            Err(ProviderError::Cancelled { at }) => sequence.push(at),
            Ok(_) => return sequence,
            Err(other) => panic!("unexpected error {other:?}"),
        }
    }
    panic!("call never completed");
}

#[test]
fn default_config_enables_the_deadline_with_the_host_clock() {
    let config = ProviderConfig::default();
    assert_eq!(
        config.max_inference_nanos,
        Some(DEFAULT_INFERENCE_BUDGET_NANOS)
    );
    assert!(config.clock.is_some());
    // Default byte caps and the pinned checksum are unchanged.
    assert_eq!(config.expected_artifact_sha256, Some(E5_SMALL_NEMB_SHA256));
    assert_eq!(
        config.max_query_bytes,
        nagi_search::MAX_SEMANTIC_QUERY_BYTES
    );
    assert_eq!(
        config.max_passage_bytes,
        nagi_search::MAX_SEMANTIC_CHUNK_BYTES
    );
    // A deadline without a clock is rejected instead of silently disabled.
    let no_clock = ProviderConfig {
        clock: None,
        ..unpinned()
    };
    assert!(matches!(
        E5Provider::from_artifact(tiny_artifact(1), no_clock),
        Err(ProviderError::InvalidConfig("clock"))
    ));
    // Explicit opt-out is allowed.
    let opt_out = ProviderConfig {
        clock: None,
        max_inference_nanos: None,
        ..unpinned()
    };
    let p = E5Provider::from_artifact(tiny_artifact(1), opt_out).unwrap();
    assert!(p.try_embed(EmbeddingPurpose::Query, "ab").is_ok());
}

#[test]
fn checkpoints_cover_tokenization_every_layer_and_pooling_in_order() {
    let p = tiny_provider(1);
    let sequence = checkpoint_sequence(&p, "ab ab");
    // "query: ab ab" pre-tokenizes into three words.
    assert_eq!(
        sequence,
        vec![
            Checkpoint::Start,
            Checkpoint::Tokenizing,
            Checkpoint::Tokenizing,
            Checkpoint::Tokenizing,
            Checkpoint::Tokenized,
            Checkpoint::LayerStart(0),
            Checkpoint::LayerMid(0),
            Checkpoint::Encoded,
            Checkpoint::Pooled,
        ]
    );
}

#[test]
fn cancel_flag_stops_before_tokenization_and_can_be_reset() {
    let p = tiny_provider(1);
    let flag = CancelFlag::new();
    flag.cancel();
    let error = p
        .try_embed_cancellable(EmbeddingPurpose::Passage, "ab", &flag)
        .unwrap_err();
    assert_eq!(
        error,
        ProviderError::Cancelled {
            at: Checkpoint::Start
        }
    );
    assert_eq!(
        to_semantic(EmbeddingPurpose::Passage, &error),
        SemanticError::ProviderUnavailable
    );
    flag.reset();
    let v = p
        .try_embed_cancellable(EmbeddingPurpose::Passage, "ab", &flag)
        .unwrap();
    assert_eq!(v.space_id(), Some(p.space_id()));
    // Input validation runs before the first checkpoint.
    flag.cancel();
    assert!(matches!(
        p.try_embed_cancellable(EmbeddingPurpose::Query, " ", &flag),
        Err(ProviderError::EmptyInput)
    ));
}

/// Returns `0` for the first `live` reads, then `expired` forever.
struct ExpireAfter {
    reads: AtomicUsize,
    live: usize,
    expired: u64,
}

impl Clock for ExpireAfter {
    fn now_nanos(&self) -> u64 {
        if self.reads.fetch_add(1, Ordering::SeqCst) < self.live {
            0
        } else {
            self.expired
        }
    }
}

fn expiring(live: usize, budget: u64, expired: u64) -> E5Provider {
    let config = ProviderConfig {
        max_inference_nanos: Some(budget),
        clock: Some(Box::new(ExpireAfter {
            reads: AtomicUsize::new(0),
            live,
            expired,
        })),
        ..unpinned()
    };
    E5Provider::from_artifact(tiny_artifact(1), config).unwrap()
}

#[test]
fn deadline_is_observed_at_every_checkpoint() {
    let sequence = checkpoint_sequence(&tiny_provider(1), "ab ab");
    // Read 0 fixes the deadline; read k (k >= 1) is the k-th checkpoint.
    for (index, expected) in sequence.iter().enumerate() {
        let p = expiring(index + 1, 100, 100);
        assert_eq!(
            p.try_embed(EmbeddingPurpose::Query, "ab ab").unwrap_err(),
            ProviderError::DeadlineExceeded {
                budget_nanos: 100,
                at: *expected
            },
            "expiry before checkpoint {index}"
        );
    }
    // Expiring only after the last checkpoint (`Pooled`) still succeeds.
    let p = expiring(sequence.len() + 1, 100, 100);
    assert!(p.try_embed(EmbeddingPurpose::Query, "ab ab").is_ok());
}

#[test]
fn deadline_boundary_is_inclusive() {
    // now == start + budget is expired ...
    let p = expiring(1, 100, 100);
    assert!(matches!(
        p.try_embed(EmbeddingPurpose::Query, "ab"),
        Err(ProviderError::DeadlineExceeded {
            at: Checkpoint::Start,
            ..
        })
    ));
    // ... one nanosecond earlier is not.
    let p = expiring(1, 100, 99);
    assert!(p.try_embed(EmbeddingPurpose::Query, "ab").is_ok());
    // A budget near u64::MAX saturates instead of wrapping into the past.
    let p = expiring(1, u64::MAX, u64::MAX - 1);
    assert!(p.try_embed(EmbeddingPurpose::Query, "ab").is_ok());
}

#[test]
fn cancellation_takes_precedence_over_an_expired_deadline() {
    let p = expiring(1, 100, 100);
    let flag = Arc::new(CancelFlag::new());
    flag.cancel();
    assert_eq!(
        p.try_embed_cancellable(EmbeddingPurpose::Query, "ab", flag.as_ref())
            .unwrap_err(),
        ProviderError::Cancelled {
            at: Checkpoint::Start
        }
    );
}

#[test]
fn cancel_from_another_thread_is_observed() {
    struct Gate(AtomicBool, CancelFlag);
    impl CancelSignal for Gate {
        fn is_cancelled(&self) -> bool {
            // Signal the first poll, then wait until the other thread cancels.
            if !self.0.swap(true, Ordering::SeqCst) {
                while !self.1.is_cancelled() {
                    std::thread::yield_now();
                }
                return false;
            }
            self.1.is_cancelled()
        }
    }
    let p = tiny_provider(1);
    let gate = Arc::new(Gate(AtomicBool::new(false), CancelFlag::new()));
    let remote = Arc::clone(&gate);
    let canceller = std::thread::spawn(move || {
        while !remote.0.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        remote.1.cancel();
    });
    let error = p
        .try_embed_cancellable(EmbeddingPurpose::Query, "ab", gate.as_ref())
        .unwrap_err();
    canceller.join().unwrap();
    // The first poll (`Start`) passed; the next one sees the cancel.
    assert_eq!(
        error,
        ProviderError::Cancelled {
            at: Checkpoint::Tokenizing
        }
    );
}
