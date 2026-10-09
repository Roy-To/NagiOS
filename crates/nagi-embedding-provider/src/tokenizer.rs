//! XLM-RoBERTa style SentencePiece-Unigram tokenizer reproducing the pinned
//! Hugging Face `tokenizer.json` pipeline:
//!
//! 1. `Precompiled` normalizer (SentencePiece `nmt_nfkc` double-array map),
//!    applied per extended grapheme cluster like the reference implementation;
//! 2. `Replace` of two or more ASCII spaces by one space;
//! 3. `Metaspace` pre-tokenizer (`▁`, prefix space, split before each `▁`);
//! 4. Unigram Viterbi segmentation per word with fused unknown runs;
//! 5. `<s> … </s>` post-processing.
//!
//! Deliberate difference from the reference: literal special-token text in the
//! input (for example `<s>`) is tokenized as ordinary characters instead of
//! being promoted to a control token, so user text cannot inject control ids.

use alloc::{string::String, vec, vec::Vec};

use unicode_segmentation::UnicodeSegmentation;

use crate::container::{Piece, PieceKind};

/// SentencePiece/HF unknown-token penalty relative to the lowest piece score.
const UNK_PENALTY: f32 = 10.0;
const METASPACE: char = '\u{2581}';

/// Double-array trie of the precompiled normalization map.
pub struct CharsMap {
    units: Vec<u32>,
    normalized: Vec<u8>,
}

impl CharsMap {
    /// Returns `None` when the blob is structurally invalid.
    pub fn parse(blob: &[u8]) -> Option<Self> {
        if blob.is_empty() {
            return Some(Self {
                units: Vec::new(),
                normalized: Vec::new(),
            });
        }
        let size = u32::from_le_bytes(blob.get(0..4)?.try_into().ok()?) as usize;
        if size % 4 != 0 || size == 0 {
            return None;
        }
        let trie = blob.get(4..4usize.checked_add(size)?)?;
        let units = trie
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        Some(Self {
            units,
            normalized: blob[4 + size..].to_vec(),
        })
    }

    fn unit(&self, pos: usize) -> Option<u32> {
        self.units.get(pos).copied()
    }

    /// Normalized replacement of the shortest key that prefixes `chunk`,
    /// matching the reference `Precompiled::transform`.
    fn transform(&self, chunk: &[u8]) -> Option<&[u8]> {
        if self.units.is_empty() {
            return None;
        }
        let offset = |unit: u32| ((unit >> 10) << ((unit & (1 << 9)) >> 6)) as usize;
        let has_leaf = |unit: u32| (unit >> 8) & 1 == 1;
        let label = |unit: u32| unit & ((1 << 31) | 0xFF);
        let value = |unit: u32| (unit & ((1 << 31) - 1)) as usize;

        let mut node = 0usize;
        let mut unit = self.unit(node)?;
        node ^= offset(unit);
        for &byte in chunk {
            if byte == 0 {
                return None;
            }
            node ^= byte as usize;
            unit = self.unit(node)?;
            if label(unit) != u32::from(byte) {
                return None;
            }
            node ^= offset(unit);
            if has_leaf(unit) {
                let start = value(self.unit(node)?);
                let rest = self.normalized.get(start..)?;
                let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
                return Some(&rest[..end]);
            }
        }
        None
    }

    fn push_transformed(&self, chunk: &str, out: &mut String) -> bool {
        match self.transform(chunk.as_bytes()) {
            Some(replacement) => match core::str::from_utf8(replacement) {
                Ok(text) => {
                    out.push_str(text);
                    true
                }
                Err(_) => false,
            },
            None => false,
        }
    }

    pub fn normalize(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for grapheme in text.graphemes(true) {
            if grapheme.len() < 6 && self.push_transformed(grapheme, &mut out) {
                continue;
            }
            for (index, character) in grapheme.char_indices() {
                let part = &grapheme[index..index + character.len_utf8()];
                if !self.push_transformed(part, &mut out) {
                    out.push(character);
                }
            }
        }
        out
    }
}

/// Open-addressing table from piece bytes to piece id. Only normal pieces are
/// inserted; control and unknown pieces are never produced by segmentation.
struct PieceTable {
    slots: Vec<u32>,
    mask: usize,
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub struct Tokenizer {
    /// Concatenated UTF-8 text of every piece; `pieces[i].start` indexes it.
    bytes: Vec<u8>,
    pieces: Vec<Piece>,
    table: PieceTable,
    charsmap: CharsMap,
    max_piece_bytes: usize,
    unk_score: f32,
    unk_id: u32,
    bos_id: u32,
    eos_id: u32,
}

impl Tokenizer {
    /// Copies piece text and the normalizer out of `artifact`, so the
    /// tokenizer does not borrow the artifact buffer.
    pub fn new(
        artifact: &[u8],
        source_pieces: &[Piece],
        charsmap: &[u8],
        unk_id: u32,
        bos_id: u32,
        eos_id: u32,
    ) -> Option<Self> {
        let charsmap = CharsMap::parse(charsmap)?;
        let mut bytes = Vec::new();
        let mut pieces = Vec::with_capacity(source_pieces.len());
        for piece in source_pieces {
            let text = artifact.get(piece.start..piece.start.checked_add(piece.len)?)?;
            pieces.push(Piece {
                start: bytes.len(),
                ..piece.clone()
            });
            bytes.extend_from_slice(text);
        }
        let capacity = (pieces.len() * 2).next_power_of_two().max(16);
        let mut slots = vec![0u32; capacity];
        let mask = capacity - 1;
        let mut min_score = f32::INFINITY;
        let mut max_piece_bytes = 0;
        for (id, piece) in pieces.iter().enumerate() {
            min_score = min_score.min(piece.score);
            if piece.kind != PieceKind::Normal {
                continue;
            }
            let key = &bytes[piece.start..piece.start + piece.len];
            max_piece_bytes = max_piece_bytes.max(piece.len);
            let mut slot = fnv1a(key) as usize & mask;
            loop {
                match slots[slot] {
                    0 => {
                        slots[slot] = u32::try_from(id).ok()? + 1;
                        break;
                    }
                    existing => {
                        let other = &pieces[existing as usize - 1];
                        if &bytes[other.start..other.start + other.len] == key {
                            // Duplicate piece text: keep the first id like a map insert-if-absent.
                            break;
                        }
                        slot = (slot + 1) & mask;
                    }
                }
            }
        }
        Some(Self {
            bytes,
            pieces,
            table: PieceTable { slots, mask },
            charsmap,
            max_piece_bytes,
            unk_score: min_score - UNK_PENALTY,
            unk_id,
            bos_id,
            eos_id,
        })
    }

    fn lookup(&self, key: &[u8]) -> Option<u32> {
        let mut slot = fnv1a(key) as usize & self.table.mask;
        loop {
            match self.table.slots[slot] {
                0 => return None,
                entry => {
                    let piece = &self.pieces[entry as usize - 1];
                    if &self.bytes[piece.start..piece.start + piece.len] == key {
                        return Some(entry - 1);
                    }
                }
            }
            slot = (slot + 1) & self.table.mask;
        }
    }

    /// Normalized, pre-tokenized form (exposed for parity tests).
    pub fn pre_tokenize(&self, text: &str) -> Vec<String> {
        let normalized = self.charsmap.normalize(text);
        // Replace(" {2,}" -> " ").
        let mut collapsed = String::with_capacity(normalized.len());
        let mut previous_space = false;
        for character in normalized.chars() {
            if character == ' ' {
                if !previous_space {
                    collapsed.push(' ');
                }
                previous_space = true;
            } else {
                collapsed.push(character);
                previous_space = false;
            }
        }
        if collapsed.is_empty() {
            return Vec::new();
        }
        // Metaspace: replace, add prefix, split before every metaspace.
        let mut replaced = String::with_capacity(collapsed.len() + 3);
        if !collapsed.starts_with(' ') && !collapsed.starts_with(METASPACE) {
            replaced.push(METASPACE);
        }
        for character in collapsed.chars() {
            replaced.push(if character == ' ' {
                METASPACE
            } else {
                character
            });
        }
        let mut words = Vec::new();
        let mut current = String::new();
        for character in replaced.chars() {
            if character == METASPACE && !current.is_empty() {
                words.push(core::mem::take(&mut current));
            }
            current.push(character);
        }
        if !current.is_empty() {
            words.push(current);
        }
        words
    }

    fn segment(&self, word: &str, out: &mut Vec<u32>) {
        let bytes = word.as_bytes();
        let n = bytes.len();
        // best[i] = (score, start, id) of the best path ending at byte i.
        let mut best: Vec<Option<(f32, usize, u32)>> = vec![None; n + 1];
        best[0] = Some((0.0, 0, 0));
        let boundaries: Vec<usize> = word
            .char_indices()
            .map(|(i, _)| i)
            .chain(core::iter::once(n))
            .collect();
        for (bi, &start) in boundaries.iter().enumerate() {
            if start == n {
                break;
            }
            let Some((base, _, _)) = best[start] else {
                continue;
            };
            let single_end = boundaries[bi + 1];
            let mut has_single = false;
            for &end in &boundaries[bi + 1..] {
                if end - start > self.max_piece_bytes {
                    break;
                }
                if let Some(id) = self.lookup(&bytes[start..end]) {
                    let candidate = base + self.pieces[id as usize].score;
                    if best[end].is_none_or(|(score, _, _)| candidate > score) {
                        best[end] = Some((candidate, start, id));
                    }
                    if end == single_end {
                        has_single = true;
                    }
                }
            }
            if !has_single {
                let candidate = base + self.unk_score;
                if best[single_end].is_none_or(|(score, _, _)| candidate > score) {
                    best[single_end] = Some((candidate, start, self.unk_id));
                }
            }
        }
        let mut reversed = Vec::new();
        let mut end = n;
        while end > 0 {
            let (_, start, id) = best[end].expect("every boundary is reachable");
            reversed.push(id);
            end = start;
        }
        let mut previous_unk = false;
        for id in reversed.into_iter().rev() {
            let is_unk = id == self.unk_id;
            if !(is_unk && previous_unk) {
                out.push(id);
            }
            previous_unk = is_unk;
        }
    }

    /// Token ids including `<s>` and `</s>`.
    pub fn encode(&self, text: &str) -> Vec<u32> {
        let mut ids = vec![self.bos_id];
        for word in self.pre_tokenize(text) {
            self.segment(&word, &mut ids);
        }
        ids.push(self.eos_id);
        ids
    }
}
