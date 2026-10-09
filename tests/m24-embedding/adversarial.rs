//! M24 adversarial artifact regression tests (no real model needed).
//!
//! These feed deliberately malformed or hostile `.nemb` artifacts to the
//! structural parser, the tokenizer and the provider, and check that every
//! case fails closed with a `ModelError` instead of panicking, wrapping an
//! offset, or allocating memory out of proportion to the artifact and input.
//! Every case is deterministic. The synthetic artifacts are fixtures only;
//! NOTHING here is real-inference evidence (see `real_inference.rs`).
//!
//! The binary installs a counting global allocator so allocation
//! amplification (a few header bytes requesting megabytes) is measured, not
//! guessed. Tests serialize on one lock so the peak belongs to one case.

use std::alloc::{GlobalAlloc, Layout, System};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

use nagi_embedding_provider::container::{self, ModelError};
use nagi_embedding_provider::tokenizer::CharsMap;
use nagi_embedding_provider::{
    CancelSignal, Checkpoint, E5Provider, ProviderConfig, ProviderError,
};
use nagi_search::EmbeddingPurpose;

// ---------------------------------------------------------------------------
// Counting allocator.

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let live = LIVE.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(live, Ordering::SeqCst);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        LIVE.fetch_sub(layout.size(), Ordering::SeqCst);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new = unsafe { System.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            if new_size >= layout.size() {
                let grow = new_size - layout.size();
                let live = LIVE.fetch_add(grow, Ordering::SeqCst) + grow;
                PEAK.fetch_max(live, Ordering::SeqCst);
            } else {
                LIVE.fetch_sub(layout.size() - new_size, Ordering::SeqCst);
            }
        }
        new
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Bytes allocated at peak by `f` beyond what was live when it started.
fn peak_extra<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = LIVE.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let out = f();
    let peak = PEAK.load(Ordering::SeqCst);
    (out, peak.saturating_sub(base))
}

const MIB: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// Synthetic artifact builder (same layout as tools/embedding/convert_e5.py).

const HIDDEN: usize = 8;
const HEADS: u32 = 2;
const INTER: usize = 16;
const POSITIONS: u32 = 16;

/// Byte offset of header u32 field `i` (after magic, version, header_len).
const fn field(i: usize) -> usize {
    16 + 4 * i
}
const F_HIDDEN: usize = 0;
const F_LAYERS: usize = 1;
const F_VOCAB_ROWS: usize = 6;
const F_N_PIECES: usize = 13;
const F_PIECES_LEN: usize = 14;
const F_CHARSMAP_LEN: usize = 15;
const F_N_TENSORS: usize = 16;
const HEADER_END: usize = 16 + 17 * 4 + 4 + 32 + 32 + 40;

fn base_pieces() -> Vec<(Vec<u8>, f32, u8)> {
    [
        ("<s>", 0.0, 1u8),
        ("<pad>", 0.0, 1),
        ("</s>", 0.0, 1),
        ("<unk>", 0.0, 2),
        ("\u{2581}", -2.0, 0),
        ("\u{2581}a", -1.0, 0),
        ("a", -3.0, 0),
        ("b", -3.0, 0),
        ("\u{2581}query", -1.0, 0),
        ("\u{2581}passage", -1.0, 0),
        (":", -3.0, 0),
    ]
    .iter()
    .map(|(t, s, k)| (t.as_bytes().to_vec(), *s, *k))
    .collect()
}

struct Spec {
    pieces: Vec<(Vec<u8>, f32, u8)>,
    charsmap: Vec<u8>,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            pieces: base_pieces(),
            charsmap: Vec::new(),
        }
    }
}

fn tensor_list(vocab: usize) -> Vec<(String, Vec<usize>)> {
    let h = HIDDEN;
    let mut tensors: Vec<(String, Vec<usize>)> = vec![
        ("embeddings.word_embeddings.weight".into(), vec![vocab, h]),
        (
            "embeddings.position_embeddings.weight".into(),
            vec![POSITIONS as usize, h],
        ),
        ("embeddings.token_type_embeddings.weight".into(), vec![2, h]),
        ("embeddings.LayerNorm.weight".into(), vec![h]),
        ("embeddings.LayerNorm.bias".into(), vec![h]),
    ];
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
        ("intermediate.dense.weight", vec![INTER, h]),
        ("intermediate.dense.bias", vec![INTER]),
        ("output.dense.weight", vec![h, INTER]),
        ("output.dense.bias", vec![h]),
        ("output.LayerNorm.weight", vec![h]),
        ("output.LayerNorm.bias", vec![h]),
    ] {
        tensors.push((format!("encoder.layer.0.{suffix}"), shape));
    }
    tensors
}

fn build(spec: &Spec) -> Vec<u8> {
    let mut pieces = Vec::new();
    for (text, score, kind) in &spec.pieces {
        pieces.extend_from_slice(&score.to_le_bytes());
        pieces.push(*kind);
        pieces.extend_from_slice(&(text.len() as u16).to_le_bytes());
        pieces.extend_from_slice(text);
    }
    let vocab = spec.pieces.len();
    let tensors = tensor_list(vocab);
    let mut header = Vec::new();
    for value in [
        HIDDEN as u32,
        1,
        HEADS,
        INTER as u32,
        POSITIONS,
        2,
        vocab as u32,
        3,
        0,
        2,
        1,
        1,
        1,
        vocab as u32,
        pieces.len() as u32,
        spec.charsmap.len() as u32,
        tensors.len() as u32,
    ] {
        header.extend_from_slice(&value.to_le_bytes());
    }
    header.extend_from_slice(&1e-12f32.to_le_bytes());
    header.extend_from_slice(&[0x11; 32]);
    header.extend_from_slice(&[0x22; 32]);
    header.extend_from_slice(&[b'0'; 40]);

    let mut out = b"NAGIEMB\0".to_vec();
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&header);
    assert_eq!(out.len(), HEADER_END);
    out.extend_from_slice(&pieces);
    out.extend_from_slice(&spec.charsmap);
    let mut state = 0x1234_5678u32;
    for (name, shape) in &tensors {
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
        for _ in 0..shape.iter().product::<usize>() {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let value = (state as f32 / u32::MAX as f32) - 0.5;
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out
}

/// A one-entry SentencePiece precompiled charsmap mapping the byte `key` to
/// `replacement` (double-array trie: root offset 1, leaf at `1 ^ key`).
fn charsmap(key: u8, replacement: &[u8]) -> Vec<u8> {
    assert!(key != 0 && !replacement.contains(&0));
    let node = 1usize ^ key as usize;
    let mut units = vec![0u32; node + 2];
    units[0] = 1 << 10; // offset 1
    units[node] = (1 << 10) | (1 << 8) | u32::from(key); // label, has_leaf, offset 1
    units[node ^ 1] = 0; // value: replacement starts at normalized[0]
    let mut blob = ((units.len() * 4) as u32).to_le_bytes().to_vec();
    for unit in units {
        blob.extend_from_slice(&unit.to_le_bytes());
    }
    blob.extend_from_slice(replacement);
    blob.push(0);
    blob
}

fn unpinned() -> ProviderConfig {
    ProviderConfig {
        expected_artifact_sha256: None,
        max_inference_nanos: None,
        clock: None,
        ..ProviderConfig::default()
    }
}

fn load(artifact: Vec<u8>) -> Result<E5Provider, ProviderError> {
    E5Provider::from_artifact(artifact, unpinned())
}

fn put_u32(artifact: &mut [u8], at: usize, value: u32) {
    artifact[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn is_model_error(result: &Result<E5Provider, ProviderError>) -> bool {
    matches!(result, Err(ProviderError::Model(_)))
}

/// Load must neither panic nor allocate more than `limit` bytes beyond the
/// artifact itself; returns the result for further checks.
fn load_bounded(label: &str, artifact: Vec<u8>, limit: usize) -> Result<E5Provider, ProviderError> {
    let (result, extra) = peak_extra(|| catch_unwind(AssertUnwindSafe(|| load(artifact))));
    let result = result.unwrap_or_else(|_| panic!("{label}: load panicked"));
    assert!(
        extra <= limit,
        "{label}: load allocated {extra} bytes at peak (limit {limit})"
    );
    result
}

// ---------------------------------------------------------------------------
// Baseline.

#[test]
fn baseline_fixture_loads_and_embeds() {
    let _s = serial();
    let p = load(build(&Spec::default())).expect("baseline loads");
    assert!(p.try_embed(EmbeddingPurpose::Query, "ab a").is_ok());
    let p = load(build(&Spec {
        charsmap: charsmap(b'x', b"ab"),
        ..Spec::default()
    }))
    .expect("baseline with charsmap loads");
    // "x" normalizes to "ab" through the synthetic charsmap.
    assert_eq!(p.tokenize("x"), p.tokenize("ab"));
}

// ---------------------------------------------------------------------------
// Truncation and header corruption.

#[test]
fn truncation_at_every_offset_fails_with_a_model_error() {
    let _s = serial();
    for spec in [
        Spec::default(),
        Spec {
            charsmap: charsmap(b'x', b"ab"),
            ..Spec::default()
        },
    ] {
        let artifact = build(&spec);
        for len in 0..artifact.len() {
            let result = load(artifact[..len].to_vec());
            assert!(
                is_model_error(&result),
                "truncated at {len}: accepted or wrong error"
            );
            if len < 8 {
                assert_eq!(
                    result.err(),
                    Some(ProviderError::Model(ModelError::Truncated("magic")))
                );
            }
        }
    }
}

#[test]
fn bad_magic_and_version_are_rejected() {
    let _s = serial();
    let base = build(&Spec::default());
    for index in 0..8 {
        let mut a = base.clone();
        a[index] ^= 0x80;
        assert_eq!(
            container::parse(&a).err(),
            Some(ModelError::BadMagic),
            "magic byte {index}"
        );
    }
    for version in [0u32, 2, u32::MAX] {
        let mut a = base.clone();
        put_u32(&mut a, 8, version);
        assert_eq!(
            container::parse(&a).err(),
            Some(ModelError::UnsupportedVersion(version))
        );
    }
    for header_len in [0u32, 211, 213, u32::MAX] {
        let mut a = base.clone();
        put_u32(&mut a, 12, header_len);
        assert_eq!(
            container::parse(&a).err(),
            Some(ModelError::Malformed("header_len"))
        );
    }
}

/// Every header u32 field set to every boundary value must fail closed (or
/// load an equivalent artifact) without a panic and without allocating more
/// than a small fixed amount beyond the artifact.
#[test]
fn every_header_field_at_boundary_values_fails_closed_without_amplification() {
    let _s = serial();
    let base = build(&Spec::default());
    let original: Vec<u32> = (0..17)
        .map(|i| u32::from_le_bytes(base[field(i)..field(i) + 4].try_into().unwrap()))
        .collect();
    let values = [
        0u32,
        1,
        2,
        3,
        4,
        255,
        4096,
        65_535,
        1_000_000,
        1_000_001,
        0x7fff_ffff,
        0x8000_0000,
        u32::MAX - 1,
        u32::MAX,
    ];
    for (index, &before) in original.iter().enumerate() {
        for &value in &values {
            let mut a = base.clone();
            put_u32(&mut a, field(index), value);
            let label = format!("field {index} = {value}");
            let result = load_bounded(&label, a, 2 * MIB);
            if value != before && result.is_ok() {
                // Only fields that do not change the layout may still load.
                // heads (2) only changes the head split; special ids
                // (7..=10) only pick other existing pieces.
                assert!(
                    matches!(index, 2 | 7..=10),
                    "{label}: inconsistent header was accepted"
                );
            }
        }
    }
}

/// A header that claims many pieces while its pieces section is tiny must be
/// rejected before the piece table is reserved: 1,000,000 claimed pieces used
/// to reserve ~24 MiB for a few-kilobyte artifact.
#[test]
fn piece_count_larger_than_the_pieces_section_is_rejected_before_allocation() {
    let _s = serial();
    let mut a = build(&Spec::default());
    put_u32(&mut a, field(F_VOCAB_ROWS), 1_000_000);
    put_u32(&mut a, field(F_N_PIECES), 1_000_000);
    let (result, extra) = peak_extra(|| container::parse(&a).map(|_| ()));
    assert_eq!(result, Err(ModelError::Malformed("n_pieces")));
    assert!(extra < 64 * 1024, "parse allocated {extra} bytes at peak");
}

#[test]
fn section_lengths_that_overflow_or_overrun_are_rejected() {
    let _s = serial();
    let base = build(&Spec::default());
    for (f, value) in [
        (F_PIECES_LEN, u32::MAX),
        (F_PIECES_LEN, 0),
        (F_CHARSMAP_LEN, u32::MAX),
        (F_CHARSMAP_LEN, base.len() as u32),
        (F_N_TENSORS, u32::MAX),
        (F_N_TENSORS, 0),
        (F_LAYERS, 64),
        (F_HIDDEN, 4096),
    ] {
        let mut a = base.clone();
        put_u32(&mut a, field(f), value);
        let result = load_bounded(&format!("field {f} = {value}"), a, 2 * MIB);
        assert!(is_model_error(&result), "field {f} = {value}");
    }
}

// ---------------------------------------------------------------------------
// Tensor records.

/// Offset of the first tensor record (word embeddings) in a default artifact.
fn first_tensor(artifact: &[u8]) -> usize {
    let n_pieces = u32::from_le_bytes(artifact[field(F_PIECES_LEN)..][..4].try_into().unwrap());
    let charsmap = u32::from_le_bytes(artifact[field(F_CHARSMAP_LEN)..][..4].try_into().unwrap());
    HEADER_END + n_pieces as usize + charsmap as usize
}

#[test]
fn hostile_tensor_records_are_rejected() {
    let _s = serial();
    let base = build(&Spec::default());
    let t = first_tensor(&base);
    let name_len = u16::from_le_bytes([base[t], base[t + 1]]) as usize;
    let ndim_at = t + 2 + name_len;
    let dims_at = ndim_at + 4;
    type Mutation = Box<dyn Fn(&mut Vec<u8>)>;
    let cases: Vec<(&str, Mutation)> = vec![
        ("ndim 0", Box::new(move |a| put_u32(a, ndim_at, 0))),
        ("ndim 5", Box::new(move |a| put_u32(a, ndim_at, 5))),
        ("ndim max", Box::new(move |a| put_u32(a, ndim_at, u32::MAX))),
        ("dim0 zero", Box::new(move |a| put_u32(a, dims_at, 0))),
        ("dim1 zero", Box::new(move |a| put_u32(a, dims_at + 4, 0))),
        (
            "dims overflow",
            Box::new(move |a| {
                put_u32(a, dims_at, u32::MAX);
                put_u32(a, dims_at + 4, u32::MAX);
            }),
        ),
        (
            "data overruns artifact",
            Box::new(move |a| {
                put_u32(a, dims_at, 0x4000_0000);
                put_u32(a, dims_at + 4, 0x4000_0000);
            }),
        ),
        ("wrong rows", Box::new(move |a| put_u32(a, dims_at, 10))),
        ("dtype", Box::new(move |a| put_u32(a, dims_at + 8, 1))),
        ("name not utf8", Box::new(move |a| a[t + 2] = 0xff)),
        (
            "name len max",
            Box::new(move |a| a[t..t + 2].copy_from_slice(&u16::MAX.to_le_bytes())),
        ),
        ("renamed", Box::new(move |a| a[t + 2] = b'E')),
    ];
    for (label, mutate) in cases {
        let mut a = base.clone();
        mutate(&mut a);
        let result = load_bounded(label, a, 2 * MIB);
        assert!(is_model_error(&result), "{label}: {:?}", result.err());
    }
}

// ---------------------------------------------------------------------------
// Pieces.

/// A normal piece longer than `MAX_NORMAL_PIECE_BYTES` (the pinned vocabulary
/// tops out at 48 bytes) is rejected at load. Before this fix a single
/// 4000-byte piece made one 2040-byte word's Viterbi pass take ~1.5 s on the
/// host (vs ~1.5 ms with a 40-byte piece), with no cancellation poll inside
/// it; the cost grows with piece length times word length.
#[test]
fn normal_pieces_longer_than_the_cap_are_rejected() {
    let _s = serial();
    let cap = container::MAX_NORMAL_PIECE_BYTES;
    assert!(
        cap >= 48,
        "cap must admit the pinned model's 48-byte pieces"
    );
    let mut at_cap = Spec::default();
    at_cap.pieces.push((vec![b'b'; cap], -1.0, 0));
    let p = load(build(&at_cap)).expect("piece at the cap loads");
    assert_eq!(p.tokenize(&"b".repeat(cap)).len(), 4); // <s> ▁ bbb… </s>
    for len in [cap + 1, 4000, u16::MAX as usize] {
        let mut spec = Spec::default();
        spec.pieces.push((vec![b'b'; len], -1.0, 0));
        let result = load_bounded(&format!("piece {len}"), build(&spec), 2 * MIB);
        assert_eq!(
            result.err(),
            Some(ProviderError::Model(ModelError::Malformed("piece_len"))),
            "piece {len}"
        );
    }
    // Control/unknown pieces are never segmented, so their length is not capped.
    let mut control = Spec::default();
    control.pieces.push((vec![b'c'; 1000], 0.0, 1));
    assert!(load(build(&control)).is_ok());
}

#[test]
fn hostile_pieces_are_rejected() {
    let _s = serial();
    type Edit = fn(&mut Vec<(Vec<u8>, f32, u8)>);
    let cases: [(&str, Edit); 6] = [
        ("empty piece", |p| p[6].0.clear()),
        ("non-utf8 piece", |p| p[6].0 = vec![0xc3]),
        ("nan score", |p| p[6].1 = f32::NAN),
        ("infinite score", |p| p[6].1 = f32::NEG_INFINITY),
        ("bad kind", |p| p[6].2 = 3),
        ("unk not unknown", |p| p[3].2 = 0),
    ];
    for (label, edit) in cases {
        let mut spec = Spec::default();
        edit(&mut spec.pieces);
        let result = load_bounded(label, build(&spec), 2 * MIB);
        assert!(is_model_error(&result), "{label}: {:?}", result.err());
    }
}

// ---------------------------------------------------------------------------
// Normalizer (precompiled charsmap).

#[test]
fn malformed_charsmap_blobs_are_rejected() {
    let _s = serial();
    let good = charsmap(b'x', b"ab");
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("short size", vec![1, 0]),
        ("size zero", vec![0, 0, 0, 0, 0]),
        ("size unaligned", vec![3, 0, 0, 0, 1, 2, 3]),
        ("size overruns", {
            let mut b = good.clone();
            b[..4].copy_from_slice(&u32::MAX.to_le_bytes());
            b
        }),
        ("size overruns by 4", {
            let mut b = good.clone();
            let size = u32::from_le_bytes(b[..4].try_into().unwrap()) + 4 + good.len() as u32;
            b[..4].copy_from_slice(&size.to_le_bytes());
            b
        }),
    ];
    for (label, blob) in cases {
        assert!(CharsMap::parse(&blob).is_none(), "{label}");
        let result = load_bounded(
            label,
            build(&Spec {
                charsmap: blob,
                ..Spec::default()
            }),
            2 * MIB,
        );
        assert_eq!(
            result.err(),
            Some(ProviderError::Model(ModelError::Malformed("charsmap"))),
            "{label}"
        );
    }
}

/// The longest replacement in the pinned multilingual-e5-small charsmap is 33
/// bytes; a hostile charsmap with a long replacement made every input
/// character expand into that many bytes of normalized text, bounded only by
/// the artifact size (measured before this fix on an unpinned 64 KiB
/// replacement: a 256-byte query peaked at ~688 MB of transient allocations,
/// so a 4096-byte query would need ~11 GB). Such a charsmap is now rejected
/// at load.
#[test]
fn charsmap_replacement_longer_than_the_cap_is_rejected_at_load() {
    let _s = serial();
    let cap = nagi_embedding_provider::tokenizer::MAX_NORMALIZED_REPLACEMENT_BYTES;
    assert!(
        cap >= 33,
        "cap must admit the pinned model's 33-byte replacement"
    );
    let over = vec![b'b'; cap + 1];
    assert!(CharsMap::parse(&charsmap(b'x', &over)).is_none());
    // Also when the blob is not NUL-terminated at the end.
    let mut unterminated = charsmap(b'x', &over);
    unterminated.pop();
    assert!(CharsMap::parse(&unterminated).is_none());
    let result = load_bounded(
        "hostile replacement",
        build(&Spec {
            charsmap: charsmap(b'x', &vec![b'b'; 4096]),
            ..Spec::default()
        }),
        2 * MIB,
    );
    assert_eq!(
        result.err(),
        Some(ProviderError::Model(ModelError::Malformed("charsmap")))
    );
}

/// At the cap, normalization of a maximum-size query stays bounded.
#[test]
fn charsmap_replacement_at_the_cap_loads_and_stays_bounded() {
    let _s = serial();
    let cap = nagi_embedding_provider::tokenizer::MAX_NORMALIZED_REPLACEMENT_BYTES;
    for replacement in [vec![b'b'; 33], vec![b'b'; cap]] {
        let p = load(build(&Spec {
            charsmap: charsmap(b'x', &replacement),
            ..Spec::default()
        }))
        .expect("replacement within the cap loads");
        let tokens = p.tokenize("x");
        assert!(tokens.len() >= 3);
        // A 4096-byte query of mapped characters: its normalized form is at
        // most 4096 * cap bytes; keep the transient peak within a fixed
        // multiple of that bound.
        let text = "x".repeat(4096);
        let bound = 4096 * cap * 48 + 4 * MIB;
        let (_, extra) = peak_extra(|| p.try_embed(EmbeddingPurpose::Query, &text));
        assert!(extra <= bound, "peak {extra} > {bound}");
    }
}

// ---------------------------------------------------------------------------
// Inference-time boundaries with hostile-but-valid artifacts.

struct CancelAfter(AtomicUsize);

impl CancelSignal for CancelAfter {
    fn is_cancelled(&self) -> bool {
        self.0.fetch_sub(1, Ordering::SeqCst) == 0
    }
}

/// Cancelling at the k-th poll returns `Cancelled` at exactly the k-th
/// checkpoint for every k, including during tokenization of many words, and
/// never returns a vector.
#[test]
fn cancellation_at_every_poll_of_a_many_word_input_is_exact() {
    let _s = serial();
    let p = load(build(&Spec::default())).unwrap();
    let text = "a b a b";
    // Collect the poll sequence.
    struct Record(Mutex<Vec<()>>);
    impl CancelSignal for Record {
        fn is_cancelled(&self) -> bool {
            self.0.lock().unwrap().push(());
            false
        }
    }
    let record = Record(Mutex::new(Vec::new()));
    assert!(p
        .try_embed_cancellable(EmbeddingPurpose::Query, text, &record)
        .is_ok());
    let polls = record.0.lock().unwrap().len();
    // Start + one Tokenizing per word ("▁query:" splits into 2, plus 4 words)
    // + Tokenized + 2 per layer + Encoded + Pooled.
    assert!(polls >= 1 + 4 + 1 + 2 + 2, "polls {polls}");
    let mut seen = Vec::new();
    for k in 0..polls {
        let signal = CancelAfter(AtomicUsize::new(k));
        match p.try_embed_cancellable(EmbeddingPurpose::Query, text, &signal) {
            Err(ProviderError::Cancelled { at }) => seen.push(at),
            other => panic!("cancel at poll {k}: {other:?}"),
        }
    }
    assert_eq!(seen.first(), Some(&Checkpoint::Start));
    assert_eq!(seen.last(), Some(&Checkpoint::Pooled));
    assert!(seen.contains(&Checkpoint::Tokenizing));
    assert!(seen.contains(&Checkpoint::LayerMid(0)));
    let signal = CancelAfter(AtomicUsize::new(polls));
    assert!(p
        .try_embed_cancellable(EmbeddingPurpose::Query, text, &signal)
        .is_ok());
}

/// Token ids beyond the word-embedding rows cannot be produced by the
/// tokenizer of a validated artifact; inputs that tokenize past the context
/// fail with `TooManyTokens` before the encoder allocates activations.
#[test]
fn over_context_inputs_fail_before_encoding() {
    let _s = serial();
    let p = load(build(&Spec::default())).unwrap();
    let text = "a ".repeat(2000);
    let (result, extra) = peak_extra(|| p.try_embed(EmbeddingPurpose::Query, &text));
    match result {
        Err(ProviderError::TooManyTokens { tokens, limit }) => {
            assert_eq!(limit, POSITIONS as usize);
            assert!(tokens > limit);
        }
        other => panic!("expected TooManyTokens, got {other:?}"),
    }
    assert!(extra < 4 * MIB, "peak {extra}");
}
