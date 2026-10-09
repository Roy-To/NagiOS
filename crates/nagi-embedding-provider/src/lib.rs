//! Model-backed [`EmbeddingProvider`] for Nagi OS 0.1 M24.
//!
//! The provider runs `intfloat/multilingual-e5-small` (Hugging Face revision
//! `614241f622f53c4eeff9890bdc4f31cfecc418b3`, MIT) through a Nagi-owned
//! `no_std` + `alloc` BERT encoder and SentencePiece-Unigram tokenizer. The
//! weights come from a deterministic `.nemb` conversion of the pinned upstream
//! files (`tools/embedding/convert_e5.py`).
//!
//! It implements the canonical 0.1 contract from `nagi-search` without
//! copying it: query and passage inputs receive the `query: ` / `passage: `
//! prefixes the model was trained with, every vector is tagged with an
//! [`EmbeddingSpaceId`] derived from the artifact digest and the
//! pooling/prefix scheme, and every failure is bounded and explicit.
//!
//! # Deadline and cancellation guarantees
//!
//! Every embedding call is bounded by a **cooperative** deadline and an
//! optional caller-owned [`CancelSignal`]. This is not strict preemption:
//! the provider never interrupts a running computation, it polls between
//! bounded units of work. Precisely:
//!
//! 1. The budget starts when [`E5Provider::try_embed`] (or
//!    [`EmbeddingProvider::embed`]) is entered, before prefixing and
//!    tokenization, so tokenization time counts against it.
//! 2. The deadline and the cancel signal are polled at every [`Checkpoint`],
//!    in this order: `Start` (before tokenization), `Tokenizing` (after
//!    normalization, before each pre-tokenized word), `Tokenized` (after
//!    tokenization and the token cap check), `LayerStart(i)` and
//!    `LayerMid(i)` (before each encoder layer and between its attention and
//!    feed-forward halves), `Encoded` (after the last layer, before pooling)
//!    and `Pooled` (after pooling, before normalization and space tagging).
//! 3. The deadline has passed when `clock.now_nanos() >= start + budget`.
//!    The first checkpoint that observes this returns
//!    [`ProviderError::DeadlineExceeded`]; a set cancel signal returns
//!    [`ProviderError::Cancelled`]. Cancellation is checked before the
//!    deadline at each checkpoint. Partial results are dropped; no vector is
//!    returned and no state is kept between calls.
//! 4. A successful result means the `Pooled` checkpoint was passed before
//!    the deadline and without cancellation. Only L2 normalization and space
//!    tagging (O(dimensions)) run after it.
//! 5. The overrun after expiry is bounded by the longest uninterrupted unit:
//!    normalizing the whole (byte-capped) input, one word's Viterbi pass, the
//!    embedding lookup, or half an encoder layer at up to 512 tokens. The
//!    host measurement is in `tests/m24-embedding/EVIDENCE.md`; guest timing
//!    is not measured.
//! 6. Loading ([`E5Provider::from_artifact`]) is not covered by the
//!    deadline; it is bounded by the artifact size cap instead.
//!
//! The default configuration enables a deadline of
//! [`DEFAULT_INFERENCE_BUDGET_NANOS`]. With the `std` feature the default
//! also supplies the host monotonic clock; without it (Nagi guest builds)
//! the caller must supply a [`Clock`] or explicitly set
//! `max_inference_nanos: None`, otherwise loading fails with
//! `InvalidConfig("clock")`. The deadline is never silently disabled.
#![no_std]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod container;
mod encoder;
pub mod tokenizer;

use alloc::{boxed::Box, string::String, vec::Vec};
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

use nagi_search::semantic::{
    Embedding, EmbeddingProvider, EmbeddingPurpose, EmbeddingSpaceId, SemanticError,
    MAX_SEMANTIC_CHUNK_BYTES, MAX_SEMANTIC_QUERY_BYTES,
};
use sha2::{Digest, Sha256};

pub use container::ModelError;
use encoder::{EncodeError, Encoder, Interrupt};
use tokenizer::Tokenizer;

/// SHA-256 of `multilingual-e5-small.nemb` produced by
/// `tools/embedding/convert_e5.py` from the pinned upstream files.
pub const E5_SMALL_NEMB_SHA256: [u8; 32] =
    hex32("7fb0a34528feecae52e13a3cb0ef6a0edcbab981ae8926c585373b1a4d71a287");
/// Size in bytes of the pinned converted artifact.
pub const E5_SMALL_NEMB_BYTES: u64 = 474_604_256;
/// Upstream revision the artifact was converted from.
pub const E5_SMALL_REVISION: &str = "614241f622f53c4eeff9890bdc4f31cfecc418b3";

pub const QUERY_PREFIX: &str = "query: ";
pub const PASSAGE_PREFIX: &str = "passage: ";
/// Model context length, including `<s>` and `</s>`.
pub const MAX_TOKENS: usize = 512;

const SPACE_DOMAIN: &[u8] = b"nagi.embedding-space.v1\0";

const fn hex32(text: &str) -> [u8; 32] {
    const fn nibble(c: u8) -> u8 {
        match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            _ => panic!("invalid hex digit"),
        }
    }
    let raw = text.as_bytes();
    assert!(raw.len() == 64);
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 32 {
        out[i] = (nibble(raw[2 * i]) << 4) | nibble(raw[2 * i + 1]);
        i += 1;
    }
    out
}

/// Default per-call inference budget (30 s). Host measurement: 490 tokens
/// take 2.0-2.6 s single-threaded on aarch64 (`EVIDENCE.md`); the margin
/// leaves room for slower targets without allowing unbounded calls.
pub const DEFAULT_INFERENCE_BUDGET_NANOS: u64 = 30_000_000_000;

/// Point in an embedding call where the deadline and cancel signal are polled.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Checkpoint {
    /// Call entry, after the byte cap check and before tokenization.
    Start,
    /// During tokenization, before segmenting a pre-tokenized word.
    Tokenizing,
    /// After tokenization and the token cap check, before the encoder.
    Tokenized,
    /// Before encoder layer `i` (0-based).
    LayerStart(usize),
    /// Between the attention and feed-forward halves of layer `i`.
    LayerMid(usize),
    /// After the last encoder layer, before mean pooling.
    Encoded,
    /// After mean pooling, before L2 normalization and space tagging.
    Pooled,
}

/// Caller-owned cooperative cancellation signal, polled at every
/// [`Checkpoint`] of [`E5Provider::try_embed_cancellable`].
pub trait CancelSignal: Sync {
    fn is_cancelled(&self) -> bool;
}

/// Simple atomic [`CancelSignal`]; `cancel` may be called from any thread.
#[derive(Debug, Default)]
pub struct CancelFlag(AtomicBool);

impl CancelFlag {
    pub const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn reset(&self) {
        self.0.store(false, Ordering::Release);
    }
}

impl CancelSignal for CancelFlag {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

struct NotCancelled;

impl CancelSignal for NotCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
}

/// Monotonic clock used to bound inference time.
pub trait Clock: Send + Sync {
    fn now_nanos(&self) -> u64;
}

/// Host monotonic clock.
#[cfg(feature = "std")]
pub struct StdClock(std::time::Instant);

#[cfg(feature = "std")]
impl Default for StdClock {
    fn default() -> Self {
        Self(std::time::Instant::now())
    }
}

#[cfg(feature = "std")]
impl Clock for StdClock {
    fn now_nanos(&self) -> u64 {
        u64::try_from(self.0.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

/// Load-time and per-call bounds.
pub struct ProviderConfig {
    /// Pinned artifact digest. `None` disables the digest check (structural
    /// validation still runs); production callers keep the default pin.
    pub expected_artifact_sha256: Option<[u8; 32]>,
    /// When set, loading fails unless the artifact yields this space.
    pub expected_space: Option<EmbeddingSpaceId>,
    pub max_artifact_bytes: usize,
    /// Token cap including `<s>`/`</s>`. `None` uses the model context
    /// (512 for multilingual-e5-small); an explicit cap above it is invalid.
    pub max_tokens: Option<usize>,
    pub max_query_bytes: usize,
    pub max_passage_bytes: usize,
    /// Per-call budget measured from call entry and polled at every
    /// [`Checkpoint`]. Defaults to [`DEFAULT_INFERENCE_BUDGET_NANOS`];
    /// `None` explicitly disables the deadline (cancellation still works).
    pub max_inference_nanos: Option<u64>,
    /// Required whenever `max_inference_nanos` is set. Defaults to the host
    /// monotonic clock with `std`; guest builds must supply one.
    pub clock: Option<Box<dyn Clock>>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            expected_artifact_sha256: Some(E5_SMALL_NEMB_SHA256),
            expected_space: None,
            max_artifact_bytes: container::MAX_ARTIFACT_BYTES,
            max_tokens: None,
            max_query_bytes: MAX_SEMANTIC_QUERY_BYTES,
            max_passage_bytes: MAX_SEMANTIC_CHUNK_BYTES,
            max_inference_nanos: Some(DEFAULT_INFERENCE_BUDGET_NANOS),
            clock: default_clock(),
        }
    }
}

#[cfg(feature = "std")]
fn default_clock() -> Option<Box<dyn Clock>> {
    Some(Box::new(StdClock::default()))
}

#[cfg(not(feature = "std"))]
fn default_clock() -> Option<Box<dyn Clock>> {
    None
}

/// Detailed provider error. [`EmbeddingProvider::embed`] maps it onto the
/// coarser [`SemanticError`].
#[derive(Clone, Debug, PartialEq)]
pub enum ProviderError {
    EmptyInput,
    InputTooLong {
        bytes: usize,
        limit: usize,
    },
    TooManyTokens {
        tokens: usize,
        limit: usize,
    },
    DeadlineExceeded {
        budget_nanos: u64,
        at: Checkpoint,
    },
    Cancelled {
        at: Checkpoint,
    },
    SpaceMismatch {
        expected: EmbeddingSpaceId,
        actual: EmbeddingSpaceId,
    },
    InvalidConfig(&'static str),
    InvalidOutput,
    Model(ModelError),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyInput => write!(f, "embedding input is empty"),
            Self::InputTooLong { bytes, limit } => {
                write!(f, "embedding input is {bytes} bytes (limit {limit})")
            }
            Self::TooManyTokens { tokens, limit } => {
                write!(f, "embedding input is {tokens} tokens (limit {limit})")
            }
            Self::DeadlineExceeded { budget_nanos, at } => {
                write!(
                    f,
                    "embedding inference exceeded its {budget_nanos} ns budget (at {at:?})"
                )
            }
            Self::Cancelled { at } => write!(f, "embedding inference was cancelled (at {at:?})"),
            Self::SpaceMismatch { .. } => {
                write!(
                    f,
                    "embedding model does not produce the expected embedding space"
                )
            }
            Self::InvalidConfig(field) => write!(f, "embedding provider config {field} is invalid"),
            Self::InvalidOutput => write!(f, "embedding model produced an invalid vector"),
            Self::Model(error) => error.fmt(f),
        }
    }
}

impl From<ModelError> for ProviderError {
    fn from(error: ModelError) -> Self {
        Self::Model(error)
    }
}

/// Provenance read from the artifact header.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactInfo {
    pub sha256: [u8; 32],
    pub source_weights_sha256: [u8; 32],
    pub source_tokenizer_sha256: [u8; 32],
    pub source_revision: String,
    pub dimensions: usize,
    pub layers: usize,
    pub vocabulary: usize,
}

/// multilingual-e5-small embedding provider.
pub struct E5Provider {
    artifact: Vec<u8>,
    tokenizer: Tokenizer,
    encoder: Encoder,
    space: EmbeddingSpaceId,
    info: ArtifactInfo,
    max_tokens: usize,
    config: ProviderConfig,
}

/// Per-call deadline + cancel state, polled at every [`Checkpoint`].
struct Guard<'a> {
    deadline: Option<(&'a dyn Clock, u64, u64)>,
    cancel: &'a dyn CancelSignal,
}

impl Guard<'_> {
    fn check(&self, at: Checkpoint) -> Result<(), ProviderError> {
        if self.cancel.is_cancelled() {
            return Err(ProviderError::Cancelled { at });
        }
        if let Some((clock, deadline, budget_nanos)) = self.deadline {
            if clock.now_nanos() >= deadline {
                return Err(ProviderError::DeadlineExceeded { budget_nanos, at });
            }
        }
        Ok(())
    }
}

/// Bridges the encoder's interrupt hook to [`Guard`], remembering why it stopped.
struct GuardInterrupt<'a> {
    guard: &'a Guard<'a>,
    reason: core::cell::Cell<Option<ProviderError>>,
}

impl Interrupt for GuardInterrupt<'_> {
    fn should_stop(&self, at: Checkpoint) -> bool {
        match self.guard.check(at) {
            Ok(()) => false,
            Err(error) => {
                self.reason.set(Some(error));
                true
            }
        }
    }
}

/// Compute the provider-neutral space fingerprint for an artifact digest.
pub fn space_id_for(artifact_sha256: &[u8; 32], dimensions: usize) -> EmbeddingSpaceId {
    let mut h = Sha256::new();
    h.update(SPACE_DOMAIN);
    h.update(artifact_sha256);
    h.update((dimensions as u32).to_le_bytes());
    h.update(b"pooling=mean\0normalize=l2\0");
    h.update(QUERY_PREFIX.as_bytes());
    h.update([0]);
    h.update(PASSAGE_PREFIX.as_bytes());
    h.update([0]);
    EmbeddingSpaceId(h.finalize().into())
}

impl E5Provider {
    /// Load from an owned artifact buffer.
    pub fn from_artifact(artifact: Vec<u8>, config: ProviderConfig) -> Result<Self, ProviderError> {
        if config
            .max_tokens
            .is_some_and(|cap| !(3..=MAX_TOKENS).contains(&cap))
        {
            return Err(ProviderError::InvalidConfig("max_tokens"));
        }
        if config.max_inference_nanos.is_some() && config.clock.is_none() {
            return Err(ProviderError::InvalidConfig("clock"));
        }
        let limit = config.max_artifact_bytes.min(container::MAX_ARTIFACT_BYTES);
        if artifact.len() > limit {
            return Err(ModelError::TooLarge {
                size: artifact.len() as u64,
                limit: limit as u64,
            }
            .into());
        }
        let sha256: [u8; 32] = Sha256::digest(&artifact).into();
        if let Some(expected) = config.expected_artifact_sha256 {
            if expected != sha256 {
                return Err(ModelError::ChecksumMismatch {
                    expected,
                    actual: sha256,
                }
                .into());
            }
        }
        let parsed = container::parse(&artifact)?;
        let header = &parsed.header;
        let max_tokens = match config.max_tokens {
            Some(cap) if cap > header.max_positions => {
                return Err(ProviderError::InvalidConfig("max_tokens"))
            }
            Some(cap) => cap,
            None => header.max_positions.min(MAX_TOKENS),
        };
        let tokenizer = Tokenizer::new(
            &artifact,
            &parsed.pieces,
            &artifact[parsed.charsmap.clone()],
            header.unk_id,
            header.bos_id,
            header.eos_id,
        )?;
        let encoder = Encoder::new(&artifact, header, &parsed.tensors)?;
        let space = space_id_for(&sha256, header.hidden);
        if let Some(expected) = config.expected_space {
            if expected != space {
                return Err(ProviderError::SpaceMismatch {
                    expected,
                    actual: space,
                });
            }
        }
        let info = ArtifactInfo {
            sha256,
            source_weights_sha256: header.source_weights_sha256,
            source_tokenizer_sha256: header.source_tokenizer_sha256,
            source_revision: String::from_utf8_lossy(&header.source_revision).into(),
            dimensions: header.hidden,
            layers: header.layers,
            vocabulary: header.n_pieces,
        };
        Ok(Self {
            artifact,
            tokenizer,
            encoder,
            space,
            info,
            max_tokens,
            config,
        })
    }

    /// Load from a host path (development host only; the guest supplies the
    /// bytes from its ModelStore through [`E5Provider::from_artifact`]).
    #[cfg(feature = "std")]
    pub fn from_path(
        path: &std::path::Path,
        config: ProviderConfig,
    ) -> Result<Self, ProviderError> {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => ModelError::Missing,
            _ => ModelError::Io,
        })?;
        let size = file.metadata().map_err(|_| ModelError::Io)?.len();
        let limit = config.max_artifact_bytes.min(container::MAX_ARTIFACT_BYTES) as u64;
        if size > limit {
            return Err(ModelError::TooLarge { size, limit }.into());
        }
        let mut artifact = Vec::new();
        artifact
            .try_reserve_exact(size as usize)
            .map_err(|_| ModelError::Io)?;
        file.take(limit + 1)
            .read_to_end(&mut artifact)
            .map_err(|_| ModelError::Io)?;
        Self::from_artifact(artifact, config)
    }

    pub fn space_id(&self) -> EmbeddingSpaceId {
        self.space
    }

    pub fn dimensions(&self) -> usize {
        self.encoder.hidden()
    }

    /// Effective token cap including `<s>`/`</s>`.
    pub fn max_tokens(&self) -> usize {
        self.max_tokens
    }

    pub fn artifact_info(&self) -> &ArtifactInfo {
        &self.info
    }

    /// The loaded tokenizer (diagnostics and regression tests).
    #[doc(hidden)]
    pub fn tokenizer(&self) -> &Tokenizer {
        &self.tokenizer
    }

    /// Token ids (with `<s>`/`</s>`) for already-prefixed text.
    pub fn tokenize(&self, text: &str) -> Vec<u32> {
        self.tokenizer.encode(text)
    }

    fn prefixed(&self, purpose: EmbeddingPurpose, text: &str) -> Result<String, ProviderError> {
        if text.trim().is_empty() {
            return Err(ProviderError::EmptyInput);
        }
        let (prefix, limit) = match purpose {
            EmbeddingPurpose::Query => (QUERY_PREFIX, self.config.max_query_bytes),
            EmbeddingPurpose::Passage => (PASSAGE_PREFIX, self.config.max_passage_bytes),
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

    /// Embed with detailed errors, bounded by the configured deadline.
    pub fn try_embed(
        &self,
        purpose: EmbeddingPurpose,
        text: &str,
    ) -> Result<Embedding, ProviderError> {
        self.try_embed_cancellable(purpose, text, &NotCancelled)
    }

    /// Embed with detailed errors, bounded by the configured deadline and by
    /// `cancel`. See the crate docs for the exact guarantees.
    pub fn try_embed_cancellable(
        &self,
        purpose: EmbeddingPurpose,
        text: &str,
        cancel: &dyn CancelSignal,
    ) -> Result<Embedding, ProviderError> {
        let deadline = match (&self.config.clock, self.config.max_inference_nanos) {
            (Some(clock), Some(budget)) => Some((
                clock.as_ref(),
                clock.now_nanos().saturating_add(budget),
                budget,
            )),
            _ => None,
        };
        let guard = Guard { deadline, cancel };
        let input = self.prefixed(purpose, text)?;
        guard.check(Checkpoint::Start)?;
        let mut stopped = None;
        let ids = self.tokenizer.encode_checked(&input, &mut || match guard
            .check(Checkpoint::Tokenizing)
        {
            Ok(()) => false,
            Err(error) => {
                stopped = Some(error);
                true
            }
        });
        let Some(ids) = ids else {
            return Err(stopped.unwrap_or(ProviderError::Cancelled {
                at: Checkpoint::Tokenizing,
            }));
        };
        if ids.len() > self.max_tokens {
            return Err(ProviderError::TooManyTokens {
                tokens: ids.len(),
                limit: self.max_tokens,
            });
        }
        guard.check(Checkpoint::Tokenized)?;
        let interrupt = GuardInterrupt {
            guard: &guard,
            reason: core::cell::Cell::new(None),
        };
        let pooled = self
            .encoder
            .encode(&self.artifact, &ids, &interrupt)
            .map_err(|error| match error {
                EncodeError::Interrupted(at) => interrupt
                    .reason
                    .take()
                    .unwrap_or(ProviderError::Cancelled { at }),
                EncodeError::TooManyTokens => ProviderError::TooManyTokens {
                    tokens: ids.len(),
                    limit: self.encoder.max_positions(),
                },
                EncodeError::InvalidToken => {
                    ProviderError::Model(ModelError::Malformed("token_id"))
                }
            })?;
        Embedding::try_from_values_in_space(pooled, self.space)
            .map_err(|_| ProviderError::InvalidOutput)
    }
}

impl EmbeddingProvider for E5Provider {
    fn embed(&self, purpose: EmbeddingPurpose, text: &str) -> Result<Embedding, SemanticError> {
        self.try_embed(purpose, text)
            .map_err(|error| to_semantic(purpose, &error))
    }
}

/// Map a detailed error onto the canonical coarse error.
pub fn to_semantic(purpose: EmbeddingPurpose, error: &ProviderError) -> SemanticError {
    match error {
        ProviderError::EmptyInput => SemanticError::EmptyText,
        ProviderError::InputTooLong { .. } | ProviderError::TooManyTokens { .. } => match purpose {
            EmbeddingPurpose::Query => SemanticError::QueryTooLong,
            EmbeddingPurpose::Passage => SemanticError::SourceTooLong,
        },
        ProviderError::SpaceMismatch { .. } => SemanticError::EmbeddingSpaceMismatch,
        ProviderError::InvalidOutput => SemanticError::InvalidEmbedding,
        ProviderError::DeadlineExceeded { .. }
        | ProviderError::Cancelled { .. }
        | ProviderError::InvalidConfig(_)
        | ProviderError::Model(_) => SemanticError::ProviderUnavailable,
    }
}
