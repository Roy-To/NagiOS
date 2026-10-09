//! Parser for the Nagi `.nemb` v1 container written by
//! `tools/embedding/convert_e5.py`. Every offset and length is validated
//! before use; malformed input yields [`ModelError`], never a panic.

use alloc::{string::String, vec::Vec};
use core::fmt;

pub const MAGIC: &[u8; 8] = b"NAGIEMB\0";
pub const VERSION: u32 = 1;
/// Upper bound accepted for an artifact (the pinned file is 474,604,256 bytes).
pub const MAX_ARTIFACT_BYTES: usize = 768 * 1024 * 1024;

const HEADER_U32_FIELDS: usize = 17;
const HEADER_BYTES: usize = HEADER_U32_FIELDS * 4 + 4 + 32 + 32 + 40;

// Structural limits that keep a malformed header from requesting huge
// allocations or quadratic work.
const MAX_HIDDEN: u32 = 4096;
const MAX_LAYERS: u32 = 64;
const MAX_INTERMEDIATE: u32 = 16384;
const MAX_POSITIONS: u32 = 8192;
const MAX_VOCAB: u32 = 1_000_000;
const MAX_TENSOR_DIMS: u32 = 4;

/// Errors raised while loading or validating an artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelError {
    /// No artifact at the configured location.
    Missing,
    /// The artifact could not be read.
    Io,
    /// The artifact exceeds [`MAX_ARTIFACT_BYTES`] or the configured cap.
    TooLarge {
        size: u64,
        limit: u64,
    },
    /// The SHA-256 of the artifact differs from the pinned digest.
    ChecksumMismatch {
        expected: [u8; 32],
        actual: [u8; 32],
    },
    BadMagic,
    UnsupportedVersion(u32),
    /// A section ends beyond the end of the artifact.
    Truncated(&'static str),
    /// A field is outside the supported range or inconsistent.
    Malformed(&'static str),
    /// A required tensor is absent or has an unexpected shape.
    Tensor(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(f, "embedding model artifact is missing"),
            Self::Io => write!(f, "embedding model artifact could not be read"),
            Self::TooLarge { size, limit } => {
                write!(
                    f,
                    "embedding model artifact is {size} bytes (limit {limit})"
                )
            }
            Self::ChecksumMismatch { .. } => {
                write!(
                    f,
                    "embedding model artifact SHA-256 does not match the pinned digest"
                )
            }
            Self::BadMagic => write!(f, "embedding model artifact has an unknown format"),
            Self::UnsupportedVersion(v) => {
                write!(f, "embedding model artifact version {v} is not supported")
            }
            Self::Truncated(section) => {
                write!(f, "embedding model artifact is truncated in {section}")
            }
            Self::Malformed(field) => {
                write!(f, "embedding model artifact field {field} is invalid")
            }
            Self::Tensor(name) => {
                write!(f, "embedding model tensor {name} is missing or malformed")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PieceKind {
    Normal,
    Control,
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub hidden: usize,
    pub layers: usize,
    pub heads: usize,
    pub intermediate: usize,
    pub max_positions: usize,
    pub type_vocab: usize,
    pub vocab_rows: usize,
    pub unk_id: u32,
    pub bos_id: u32,
    pub eos_id: u32,
    pub pad_id: u32,
    pub n_pieces: usize,
    pub layer_norm_eps: f32,
    pub source_weights_sha256: [u8; 32],
    pub source_tokenizer_sha256: [u8; 32],
    pub source_revision: [u8; 40],
}

/// A tensor located inside the artifact bytes (f32 little-endian).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TensorRef {
    pub name: String,
    pub shape: Vec<usize>,
    pub offset: usize,
    pub elements: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub score: f32,
    pub kind: PieceKind,
    /// Byte range of the UTF-8 piece inside the artifact.
    pub start: usize,
    pub len: usize,
}

#[derive(Debug)]
pub struct Parsed {
    pub header: Header,
    pub pieces: Vec<Piece>,
    pub charsmap: core::ops::Range<usize>,
    pub tensors: Vec<TensorRef>,
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize, section: &'static str) -> Result<&'a [u8], ModelError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or(ModelError::Truncated(section))?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or(ModelError::Truncated(section))?;
        self.pos = end;
        Ok(slice)
    }

    fn u32(&mut self, section: &'static str) -> Result<u32, ModelError> {
        let raw = self.take(4, section)?;
        Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
    }

    fn u16(&mut self, section: &'static str) -> Result<u16, ModelError> {
        let raw = self.take(2, section)?;
        Ok(u16::from_le_bytes([raw[0], raw[1]]))
    }

    fn f32(&mut self, section: &'static str) -> Result<f32, ModelError> {
        Ok(f32::from_bits(self.u32(section)?))
    }
}

fn bounded(value: u32, min: u32, max: u32, field: &'static str) -> Result<usize, ModelError> {
    if (min..=max).contains(&value) {
        Ok(value as usize)
    } else {
        Err(ModelError::Malformed(field))
    }
}

pub fn parse(bytes: &[u8]) -> Result<Parsed, ModelError> {
    let mut c = Cursor { bytes, pos: 0 };
    if c.take(8, "magic")? != MAGIC {
        return Err(ModelError::BadMagic);
    }
    let version = c.u32("version")?;
    if version != VERSION {
        return Err(ModelError::UnsupportedVersion(version));
    }
    let header_len = c.u32("header")? as usize;
    if header_len != HEADER_BYTES {
        return Err(ModelError::Malformed("header_len"));
    }
    let mut f = [0u32; HEADER_U32_FIELDS];
    for slot in f.iter_mut() {
        *slot = c.u32("header")?;
    }
    let hidden = bounded(f[0], 1, MAX_HIDDEN, "hidden")?;
    let layers = bounded(f[1], 1, MAX_LAYERS, "layers")?;
    let heads = bounded(f[2], 1, f[0], "heads")?;
    if hidden % heads != 0 {
        return Err(ModelError::Malformed("heads"));
    }
    let intermediate = bounded(f[3], 1, MAX_INTERMEDIATE, "intermediate")?;
    let max_positions = bounded(f[4], 3, MAX_POSITIONS, "max_positions")?;
    let type_vocab = bounded(f[5], 1, 16, "type_vocab")?;
    let vocab_rows = bounded(f[6], 4, MAX_VOCAB, "vocab_rows")?;
    let (unk_id, bos_id, eos_id, pad_id) = (f[7], f[8], f[9], f[10]);
    if f[11] != 1 {
        return Err(ModelError::Malformed("pooling"));
    }
    if f[12] != 1 {
        return Err(ModelError::Malformed("normalize"));
    }
    let n_pieces = bounded(f[13], 4, f[6], "n_pieces")?;
    for id in [unk_id, bos_id, eos_id, pad_id] {
        if id as usize >= n_pieces {
            return Err(ModelError::Malformed("special_ids"));
        }
    }
    let pieces_len = f[14] as usize;
    let charsmap_len = f[15] as usize;
    let n_tensors = f[16] as usize;
    let layer_norm_eps = c.f32("header")?;
    if !(layer_norm_eps.is_finite() && layer_norm_eps > 0.0 && layer_norm_eps < 1.0) {
        return Err(ModelError::Malformed("layer_norm_eps"));
    }
    let mut source_weights_sha256 = [0u8; 32];
    source_weights_sha256.copy_from_slice(c.take(32, "header")?);
    let mut source_tokenizer_sha256 = [0u8; 32];
    source_tokenizer_sha256.copy_from_slice(c.take(32, "header")?);
    let mut source_revision = [0u8; 40];
    source_revision.copy_from_slice(c.take(40, "header")?);

    // Pieces.
    let pieces_start = c.pos;
    let pieces_end = pieces_start
        .checked_add(pieces_len)
        .filter(|end| *end <= bytes.len())
        .ok_or(ModelError::Truncated("pieces"))?;
    let mut pieces = Vec::new();
    pieces
        .try_reserve_exact(n_pieces)
        .map_err(|_| ModelError::Malformed("n_pieces"))?;
    for _ in 0..n_pieces {
        let score = c.f32("pieces")?;
        let kind = match c.take(1, "pieces")?[0] {
            0 => PieceKind::Normal,
            1 => PieceKind::Control,
            2 => PieceKind::Unknown,
            _ => return Err(ModelError::Malformed("piece_kind")),
        };
        let len = c.u16("pieces")? as usize;
        let start = c.pos;
        let raw = c.take(len, "pieces")?;
        if len == 0 || core::str::from_utf8(raw).is_err() || !score.is_finite() {
            return Err(ModelError::Malformed("piece"));
        }
        pieces.push(Piece {
            score,
            kind,
            start,
            len,
        });
    }
    if c.pos != pieces_end {
        return Err(ModelError::Malformed("pieces_len"));
    }
    if pieces[unk_id as usize].kind != PieceKind::Unknown {
        return Err(ModelError::Malformed("unk_id"));
    }

    // Normalizer.
    let charsmap_start = c.pos;
    c.take(charsmap_len, "charsmap")?;
    let charsmap = charsmap_start..c.pos;

    // Tensors.
    if n_tensors != 5 + 16 * layers {
        return Err(ModelError::Malformed("n_tensors"));
    }
    let mut tensors = Vec::with_capacity(n_tensors);
    for _ in 0..n_tensors {
        let name_len = c.u16("tensor")? as usize;
        let name = core::str::from_utf8(c.take(name_len, "tensor")?)
            .map_err(|_| ModelError::Malformed("tensor_name"))?;
        let ndim = c.u32("tensor")?;
        if ndim == 0 || ndim > MAX_TENSOR_DIMS {
            return Err(ModelError::Tensor(name.into()));
        }
        let mut shape = Vec::with_capacity(ndim as usize);
        let mut elements = 1usize;
        for _ in 0..ndim {
            let dim = c.u32("tensor")? as usize;
            elements = elements
                .checked_mul(dim)
                .ok_or_else(|| ModelError::Tensor(name.into()))?;
            shape.push(dim);
        }
        if c.u32("tensor")? != 0 {
            return Err(ModelError::Tensor(name.into()));
        }
        let pad = (4 - c.pos % 4) % 4;
        c.take(pad, "tensor")?;
        let offset = c.pos;
        let byte_len = elements
            .checked_mul(4)
            .ok_or_else(|| ModelError::Tensor(name.into()))?;
        c.take(byte_len, "tensor data")?;
        tensors.push(TensorRef {
            name: name.into(),
            shape,
            offset,
            elements,
        });
    }
    if c.pos != bytes.len() {
        return Err(ModelError::Malformed("trailing_bytes"));
    }

    Ok(Parsed {
        header: Header {
            hidden,
            layers,
            heads,
            intermediate,
            max_positions,
            type_vocab,
            vocab_rows,
            unk_id,
            bos_id,
            eos_id,
            pad_id,
            n_pieces,
            layer_norm_eps,
            source_weights_sha256,
            source_tokenizer_sha256,
            source_revision,
        },
        pieces,
        charsmap,
        tensors,
    })
}

/// Decode `elements` little-endian f32 values starting at `offset`.
pub fn read_f32s(bytes: &[u8], offset: usize, elements: usize) -> Vec<f32> {
    bytes[offset..offset + elements * 4]
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}
