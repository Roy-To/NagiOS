//! Real-inference acceptance for the M24 embedding provider (HOST inference).
//!
//! These tests run the pinned multilingual-e5-small artifact through the
//! Nagi-owned encoder. They are `#[ignore]`d by default because they need the
//! 475 MB converted artifact; run them with
//!
//! ```sh
//! NAGI_EMBEDDING_MODEL=/path/to/multilingual-e5-small.nemb \
//!   cargo test --manifest-path crates/nagi-embedding-provider/Cargo.toml \
//!   --test m24-embedding-real-inference -- --ignored
//! ```
//!
//! When run with `--ignored` and the artifact is absent, every test FAILS; a
//! missing model is never reported as a real-function pass. No test here uses
//! fixture vectors: every vector comes from the model.

use std::{
    path::PathBuf,
    sync::OnceLock,
    time::{Duration, Instant},
};

use nagi_embedding_provider::{E5Provider, ProviderConfig, ProviderError, StdClock};
use nagi_model::ObjectId;
use nagi_search::{
    chunk_text, BackendError, EmbeddingProvider, EmbeddingPurpose, IndexedChunk,
    PersistentVectorIndex, SnapshotBackend, VectorIndex,
};

fn model_path() -> PathBuf {
    let path = std::env::var_os("NAGI_EMBEDDING_MODEL").unwrap_or_else(|| {
        panic!(
            "NAGI_EMBEDDING_MODEL is not set: real-inference tests require the pinned \
             artifact (tools/embedding/fetch.sh + convert_e5.py); refusing to pass without it"
        )
    });
    PathBuf::from(path)
}

fn provider() -> &'static E5Provider {
    static PROVIDER: OnceLock<E5Provider> = OnceLock::new();
    PROVIDER.get_or_init(|| {
        let started = Instant::now();
        let provider = E5Provider::from_path(&model_path(), ProviderConfig::default())
            .unwrap_or_else(|error| panic!("pinned artifact failed to load: {error}"));
        eprintln!(
            "m24: loaded {} (rev {}) in {:?}",
            hex(&provider.artifact_info().sha256),
            provider.artifact_info().source_revision,
            started.elapsed()
        );
        provider
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum()
}

fn split_purpose(text: &str) -> (EmbeddingPurpose, &str) {
    if let Some(rest) = text.strip_prefix("query: ") {
        (EmbeddingPurpose::Query, rest)
    } else if let Some(rest) = text.strip_prefix("passage: ") {
        (EmbeddingPurpose::Passage, rest)
    } else {
        panic!("corpus entry without prefix: {text}")
    }
}

struct ReferenceItem {
    text: String,
    ids: Vec<u32>,
    embedding: Vec<f32>,
}

fn reference_items() -> Vec<ReferenceItem> {
    let raw = include_str!("parity_reference.json");
    json::parse_reference(raw)
}

mod json {
    use super::ReferenceItem;

    struct P<'a> {
        s: &'a [u8],
        i: usize,
    }

    impl P<'_> {
        fn ws(&mut self) {
            while self.i < self.s.len() && (self.s[self.i] as char).is_ascii_whitespace() {
                self.i += 1;
            }
        }
        fn expect(&mut self, c: u8) {
            self.ws();
            assert_eq!(self.s[self.i], c, "json: expected {}", c as char);
            self.i += 1;
        }
        fn peek(&mut self) -> u8 {
            self.ws();
            self.s[self.i]
        }
        fn string(&mut self) -> String {
            self.expect(b'"');
            let mut out = Vec::new();
            loop {
                let c = self.s[self.i];
                self.i += 1;
                match c {
                    b'"' => break,
                    b'\\' => {
                        let e = self.s[self.i];
                        self.i += 1;
                        match e {
                            b'n' => out.push(b'\n'),
                            b't' => out.push(b'\t'),
                            b'r' => out.push(b'\r'),
                            b'"' => out.push(b'"'),
                            b'\\' => out.push(b'\\'),
                            b'/' => out.push(b'/'),
                            b'u' => {
                                let mut code = self.hex4();
                                if (0xD800..0xDC00).contains(&code) {
                                    assert_eq!(&self.s[self.i..self.i + 2], b"\\u");
                                    self.i += 2;
                                    let low = self.hex4();
                                    code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                }
                                let ch = char::from_u32(code).expect("json: scalar");
                                let mut buf = [0u8; 4];
                                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                            }
                            _ => panic!("json: escape"),
                        }
                    }
                    _ => out.push(c),
                }
            }
            String::from_utf8(out).expect("json: utf8")
        }
        fn hex4(&mut self) -> u32 {
            let text = std::str::from_utf8(&self.s[self.i..self.i + 4]).unwrap();
            self.i += 4;
            u32::from_str_radix(text, 16).expect("json: hex")
        }
        fn number(&mut self) -> f64 {
            self.ws();
            let start = self.i;
            while self.i < self.s.len() && b"+-0123456789.eE".contains(&self.s[self.i]) {
                self.i += 1;
            }
            std::str::from_utf8(&self.s[start..self.i])
                .unwrap()
                .parse()
                .expect("json: number")
        }
        fn numbers(&mut self) -> Vec<f64> {
            self.expect(b'[');
            let mut out = Vec::new();
            if self.peek() == b']' {
                self.i += 1;
                return out;
            }
            loop {
                out.push(self.number());
                if self.peek() == b',' {
                    self.i += 1;
                } else {
                    self.expect(b']');
                    return out;
                }
            }
        }
    }

    pub fn parse_reference(raw: &str) -> Vec<ReferenceItem> {
        let mut p = P {
            s: raw.as_bytes(),
            i: 0,
        };
        let mut items = Vec::new();
        p.expect(b'{');
        loop {
            let key = p.string();
            p.expect(b':');
            if key == "items" {
                p.expect(b'[');
                loop {
                    p.expect(b'{');
                    let (mut text, mut ids, mut embedding) = (String::new(), vec![], vec![]);
                    loop {
                        let field = p.string();
                        p.expect(b':');
                        match field.as_str() {
                            "text" => text = p.string(),
                            "ids" => ids = p.numbers().into_iter().map(|v| v as u32).collect(),
                            "embedding" => {
                                embedding = p.numbers().into_iter().map(|v| v as f32).collect()
                            }
                            _ => panic!("json: field {field}"),
                        }
                        if p.peek() == b',' {
                            p.i += 1;
                        } else {
                            p.expect(b'}');
                            break;
                        }
                    }
                    items.push(ReferenceItem {
                        text,
                        ids,
                        embedding,
                    });
                    if p.peek() == b',' {
                        p.i += 1;
                    } else {
                        p.expect(b']');
                        break;
                    }
                }
            } else {
                p.string();
            }
            if p.peek() == b',' {
                p.i += 1;
            } else {
                p.expect(b'}');
                break;
            }
        }
        items
    }
}

/// Token ids and vectors match the upstream ONNX export + HF tokenizer.
#[test]
#[ignore = "requires NAGI_EMBEDDING_MODEL (pinned multilingual-e5-small .nemb)"]
fn parity_with_upstream_onnx_reference() {
    let provider = provider();
    let items = reference_items();
    assert!(items.len() >= 20);
    let mut worst = 1.0f64;
    let mut worst_abs = 0.0f32;
    for item in &items {
        let ours_ids = provider.tokenize(&item.text);
        let (purpose, text) = split_purpose(&item.text);
        if item.text.contains("<s>") {
            // Deliberate: literal special-token text is not promoted to a
            // control id. The reference does promote it.
            assert!(item.ids[1..item.ids.len() - 1].contains(&0));
            assert!(!ours_ids[1..ours_ids.len() - 1].contains(&0));
            continue;
        }
        assert_eq!(ours_ids, item.ids, "token ids differ for {:?}", item.text);
        let ours = provider.try_embed(purpose, text).expect("embed");
        let cos = cosine(ours.values(), &item.embedding);
        let max_abs = ours
            .values()
            .iter()
            .zip(&item.embedding)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        eprintln!(
            "m24 parity: cos={cos:.9} max_abs={max_abs:.2e} {:?}",
            item.text
        );
        worst = worst.min(cos);
        worst_abs = worst_abs.max(max_abs);
    }
    eprintln!("m24 parity: worst cosine {worst:.9}, worst max |diff| {worst_abs:.2e}");
    assert!(worst > 0.99999, "worst cosine {worst}");
    assert!(worst_abs < 1e-4, "worst abs diff {worst_abs}");
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

const DOCS: &[(u64, &str)] = &[
    (1, "Servo is a web browser engine written in Rust. Its layout runs in parallel across CPU cores."),
    (2, "昨日読んだ記事：Rust製のブラウザエンジンServoが、並列レイアウトと埋め込みAPIを改善した。"),
    (3, "明日の東京は晴れのち曇り、最高気温は24度の予報です。"),
    (4, "Tomorrow's forecast for Osaka: heavy rain in the afternoon with strong winds."),
    (5, "カボチャの煮物の作り方：砂糖と醤油で弱火でじっくり煮込みます。"),
    (6, "To bake sourdough bread, feed the starter the night before and proof the dough for 12 hours."),
    (7, "The marathon training plan builds weekly mileage gradually and adds one long run each Sunday."),
    (8, "特許出願の明細書には、発明の課題と解決手段、実施形態を記載する。"),
    (9, "Invoices are due within 30 days; late payments incur a 2% monthly fee."),
    (10, "猫はこたつで丸くなり、犬は雪の中を喜んで駆け回る。"),
];

fn build_index() -> PersistentVectorIndex<MemoryBackend> {
    let provider = provider();
    let mut index = PersistentVectorIndex::open(MemoryBackend(None)).expect("index");
    for (id, text) in DOCS {
        let object = ObjectId(*id);
        let chunks = chunk_text(object, text).expect("chunk");
        let indexed: Vec<IndexedChunk> = chunks
            .into_iter()
            .map(|chunk| IndexedChunk {
                embedding: provider
                    .embed(EmbeddingPurpose::Passage, &chunk.text)
                    .expect("passage embedding"),
                chunk,
            })
            .collect();
        index.replace_object(object, &indexed).expect("replace");
    }
    index
}

fn ranked(index: &PersistentVectorIndex<MemoryBackend>, query: &str) -> Vec<(u64, f32)> {
    let q = provider()
        .embed(EmbeddingPurpose::Query, query)
        .expect("query embedding");
    let all: Vec<ObjectId> = DOCS.iter().map(|(id, _)| ObjectId(*id)).collect();
    index
        .search(&q, &all, all.len())
        .expect("search")
        .into_iter()
        .map(|m| (m.object_id.0, m.similarity))
        .collect()
}

/// Japanese and English queries retrieve the semantically matching document
/// (including cross-lingual pairs) through the canonical persistent index.
#[test]
#[ignore = "requires NAGI_EMBEDDING_MODEL (pinned multilingual-e5-small .nemb)"]
fn japanese_and_english_semantic_neighbors() {
    let index = build_index();
    // (query, acceptable ids, required rank). Same-language queries must
    // rank a matching document first. Cross-lingual queries (query language
    // differs from the matching document) must place it within the top 2:
    // e5-small is measurably weaker cross-lingually (e.g. "how to cook
    // pumpkin" ranks the Japanese pumpkin recipe 2nd behind an English bread
    // recipe, 0.8026 vs 0.8215), and the evidence log records every rank.
    let cases: &[(&str, &[u64], usize)] = &[
        ("the Servo article I looked at yesterday", &[1, 2], 1),
        ("昨日見たServoの記事", &[1, 2], 1),
        ("Rust browser engine parallel layout", &[1, 2], 1),
        ("東京の明日の天気", &[3], 1),
        ("カボチャの煮物の作り方", &[5], 1),
        ("how to bake bread", &[6], 1),
        ("マラソンの練習計画", &[7], 2),
        ("how to write a patent specification", &[8], 2),
        ("特許明細書の書き方", &[8], 1),
        ("請求書の支払い期限", &[9], 2),
        ("invoice payment due date", &[9], 1),
        ("cats and dogs in winter", &[10], 2),
        ("こたつで丸くなる猫", &[10], 1),
        ("weather forecast for Tokyo tomorrow", &[3], 2),
        ("how to cook pumpkin", &[5], 2),
        ("パンの焼き方", &[6], 2),
    ];
    let mut failures = Vec::new();
    for (query, expected, required_rank) in cases {
        let ranking = ranked(&index, query);
        let rank = ranking
            .iter()
            .position(|(id, _)| expected.contains(id))
            .map_or(usize::MAX, |p| p + 1);
        eprintln!(
            "m24 neighbors: {query:?} -> expected {expected:?} at rank {rank} (required <= {required_rank}); top {:?}",
            &ranking[..3]
        );
        if rank > *required_rank {
            failures.push(format!("{query:?}: rank {rank}, ranking {ranking:?}"));
        }
    }
    assert!(failures.is_empty(), "neighbor failures: {failures:#?}");
}

/// Cross-lingual paraphrases are closer than unrelated sentences.
#[test]
#[ignore = "requires NAGI_EMBEDDING_MODEL (pinned multilingual-e5-small .nemb)"]
fn cross_lingual_paraphrase_is_closer_than_unrelated() {
    let p = provider();
    let embed = |t: &str| p.embed(EmbeddingPurpose::Query, t).expect("embed");
    let ja = embed("Rustの所有権システムはガベージコレクタなしでメモリ安全性を保証する。");
    let en = embed("Rust's ownership system guarantees memory safety without a garbage collector.");
    let unrelated = embed("明日は雨なので傘を持って出かけてください。");
    let paraphrase = cosine(ja.values(), en.values());
    let other = cosine(ja.values(), unrelated.values());
    eprintln!("m24 cross-lingual: paraphrase {paraphrase:.4} unrelated {other:.4}");
    assert!(paraphrase > other + 0.05, "{paraphrase} vs {other}");
}

/// Query and passage prefixes are applied (they change the vector), and both
/// vectors share one embedding space.
#[test]
#[ignore = "requires NAGI_EMBEDDING_MODEL (pinned multilingual-e5-small .nemb)"]
fn query_and_passage_prefixes_differ_in_one_space() {
    let p = provider();
    let q = p.embed(EmbeddingPurpose::Query, "Servo").unwrap();
    let d = p.embed(EmbeddingPurpose::Passage, "Servo").unwrap();
    assert_eq!(q.space_id(), Some(p.space_id()));
    assert_eq!(d.space_id(), Some(p.space_id()));
    assert_eq!(q.dimensions(), 384);
    let c = cosine(q.values(), d.values());
    assert!(c < 0.9999, "prefix had no effect: {c}");
    assert_eq!(
        p.tokenize("query: Servo")[..4],
        p.tokenize("query: Servo browser")[..4]
    );
}

/// Over-limit and empty inputs fail with explicit errors on the real model.
#[test]
#[ignore = "requires NAGI_EMBEDDING_MODEL (pinned multilingual-e5-small .nemb)"]
fn real_model_bounds_inputs() {
    let p = provider();
    assert_eq!(
        p.try_embed(EmbeddingPurpose::Query, "  \n\t"),
        Err(ProviderError::EmptyInput)
    );
    // Within the byte cap but beyond the 512-token context: each emoji
    // with a variation selector is several tokens.
    let long = "😀 ".repeat(600);
    assert!(long.len() <= 4096);
    match p.try_embed(EmbeddingPurpose::Query, &long) {
        Err(ProviderError::TooManyTokens { tokens, limit: 512 }) => assert!(tokens > 512),
        other => panic!("expected TooManyTokens, got {other:?}"),
    }
    let bytes = "a".repeat(5000);
    assert!(matches!(
        p.try_embed(EmbeddingPurpose::Query, &bytes),
        Err(ProviderError::InputTooLong { limit: 4096, .. })
    ));
}

/// A deadline aborts inference between encoder layers.
#[test]
#[ignore = "requires NAGI_EMBEDDING_MODEL (pinned multilingual-e5-small .nemb)"]
fn deadline_aborts_real_inference() {
    let config = ProviderConfig {
        max_inference_nanos: Some(1),
        clock: Some(Box::new(StdClock::default())),
        ..ProviderConfig::default()
    };
    let p = E5Provider::from_path(&model_path(), config).expect("load");
    assert!(matches!(
        p.try_embed(EmbeddingPurpose::Passage, "Servo is a browser engine."),
        Err(ProviderError::DeadlineExceeded { .. })
    ));
}

/// Latency evidence for 128-token and 512-token inputs (host, release-opt).
#[test]
#[ignore = "requires NAGI_EMBEDDING_MODEL (pinned multilingual-e5-small .nemb)"]
fn host_latency_evidence() {
    let p = provider();
    for (label, text) in [
        ("short", "東京の明日の天気".to_string()),
        (
            "~128 tok",
            "Servo is a web browser engine written in Rust. ".repeat(12),
        ),
        (
            "~500 tok",
            "Servo is a web browser engine written in Rust. ".repeat(44),
        ),
    ] {
        let tokens = p.tokenize(&format!("query: {text}")).len();
        let started = Instant::now();
        p.embed(EmbeddingPurpose::Query, &text).expect("embed");
        let elapsed = started.elapsed();
        eprintln!("m24 latency: {label}: {tokens} tokens in {elapsed:?}");
        assert!(elapsed < Duration::from_secs(60));
    }
}
