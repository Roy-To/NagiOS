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

use crate::container::{ModelError, Piece, PieceKind};

/// SentencePiece/HF unknown-token penalty relative to the lowest piece score.
const UNK_PENALTY: f32 = 10.0;
const METASPACE: char = '\u{2581}';
/// Longest normalized replacement accepted from a precompiled charsmap. The
/// pinned multilingual-e5-small map's longest replacement is 33 bytes; the cap
/// bounds how far one input character can expand during normalization.
pub const MAX_NORMALIZED_REPLACEMENT_BYTES: usize = 64;

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
        if !size.is_multiple_of(4) || size == 0 {
            return None;
        }
        let trie = blob.get(4..4usize.checked_add(size)?)?;
        let normalized = &blob[4 + size..];
        // A replacement is read from any trie-supplied offset up to the next
        // NUL (or the end), so the longest NUL-free run bounds every
        // replacement. Reject maps whose runs exceed the cap.
        if normalized
            .split(|b| *b == 0)
            .any(|run| run.len() > MAX_NORMALIZED_REPLACEMENT_BYTES)
        {
            return None;
        }
        let units = trie
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        Some(Self {
            units,
            normalized: normalized.to_vec(),
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

/// Most slots one piece insertion may examine in the piece hash table
/// (linear probing, load factor <= 1/2). The hash is a fixed, unkeyed FNV-1a,
/// so an unpinned hostile artifact can choose piece texts that all land in
/// one bucket; without a cap, `k` such pieces cost `k*(k+1)/2` slot visits to
/// insert and every missing-key lookup in their bucket walks the whole run.
/// The pinned multilingual-e5-small table needs at most 28 (249,997 normal
/// pieces in 524,288 slots), so 128 leaves 4.5x headroom.
pub const MAX_PIECE_PROBES: usize = 128;

/// Most slots the whole table build may examine for `pieces` pieces: four per
/// piece (the pinned table averages 1.45) plus room for one maximal run, so
/// building is linear in the piece count even when every insertion stays under
/// [`MAX_PIECE_PROBES`]. The pinned table uses 361,886 of 1,016,392.
pub const fn max_table_build_probes(pieces: usize) -> usize {
    pieces
        .saturating_mul(4)
        .saturating_add(MAX_PIECE_PROBES * MAX_PIECE_PROBES)
}

/// Open-addressing table from piece bytes to piece id. Only normal pieces are
/// inserted; control and unknown pieces are never produced by segmentation.
struct PieceTable {
    slots: Vec<u32>,
    mask: usize,
    /// Longest probe sequence any insertion took (<= [`MAX_PIECE_PROBES`]).
    /// Every stored key sits within this many slots of its home bucket, so a
    /// lookup that has examined this many slots can stop: the key is absent.
    max_probes: usize,
    /// Slots examined while building the table.
    build_probes: usize,
}

/// Probe statistics of a built piece table (diagnostics and regression tests).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PieceTableStats {
    pub slots: usize,
    pub max_probes: usize,
    pub build_probes: usize,
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
    ///
    /// Fails closed with `ModelError::Malformed("piece_table")` when building
    /// the piece hash table would exceed [`MAX_PIECE_PROBES`] for one piece or
    /// [`max_table_build_probes`] in total (hostile bucket collisions).
    pub fn new(
        artifact: &[u8],
        source_pieces: &[Piece],
        charsmap: &[u8],
        unk_id: u32,
        bos_id: u32,
        eos_id: u32,
    ) -> Result<Self, ModelError> {
        let mut work = 0;
        Self::build(
            artifact,
            source_pieces,
            charsmap,
            [unk_id, bos_id, eos_id],
            &mut work,
        )
    }

    /// [`Tokenizer::new`] plus the piece-table slots examined, also when the
    /// build fails closed (regression tests measure the work done before the
    /// bound trips).
    #[doc(hidden)]
    pub fn new_with_build_work(
        artifact: &[u8],
        source_pieces: &[Piece],
        charsmap: &[u8],
        unk_id: u32,
        bos_id: u32,
        eos_id: u32,
    ) -> (Result<Self, ModelError>, usize) {
        let mut work = 0;
        let result = Self::build(
            artifact,
            source_pieces,
            charsmap,
            [unk_id, bos_id, eos_id],
            &mut work,
        );
        (result, work)
    }

    fn build(
        artifact: &[u8],
        source_pieces: &[Piece],
        charsmap: &[u8],
        [unk_id, bos_id, eos_id]: [u32; 3],
        build_probes: &mut usize,
    ) -> Result<Self, ModelError> {
        let charsmap = CharsMap::parse(charsmap).ok_or(ModelError::Malformed("charsmap"))?;
        let mut bytes = Vec::new();
        let mut pieces = Vec::with_capacity(source_pieces.len());
        for piece in source_pieces {
            let text = piece
                .start
                .checked_add(piece.len)
                .and_then(|end| artifact.get(piece.start..end))
                .ok_or(ModelError::Malformed("pieces"))?;
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
        let build_budget = max_table_build_probes(pieces.len());
        *build_probes = 0;
        let mut max_probes = 0usize;
        for (id, piece) in pieces.iter().enumerate() {
            min_score = min_score.min(piece.score);
            if piece.kind != PieceKind::Normal {
                continue;
            }
            if piece.len > crate::container::MAX_NORMAL_PIECE_BYTES {
                return Err(ModelError::Malformed("piece_len"));
            }
            let key = &bytes[piece.start..piece.start + piece.len];
            max_piece_bytes = max_piece_bytes.max(piece.len);
            let mut slot = fnv1a(key) as usize & mask;
            let mut probes = 0usize;
            loop {
                probes += 1;
                *build_probes += 1;
                if probes > MAX_PIECE_PROBES || *build_probes > build_budget {
                    return Err(ModelError::Malformed("piece_table"));
                }
                match slots[slot] {
                    0 => {
                        let entry = u32::try_from(id)
                            .ok()
                            .and_then(|id| id.checked_add(1))
                            .ok_or(ModelError::Malformed("n_pieces"))?;
                        slots[slot] = entry;
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
            max_probes = max_probes.max(probes);
        }
        Ok(Self {
            bytes,
            pieces,
            table: PieceTable {
                slots,
                mask,
                max_probes,
                build_probes: *build_probes,
            },
            charsmap,
            max_piece_bytes,
            unk_score: min_score - UNK_PENALTY,
            unk_id,
            bos_id,
            eos_id,
        })
    }

    fn lookup(&self, key: &[u8]) -> Option<u32> {
        self.lookup_counted(key).0
    }

    /// Lookup plus the number of slots it examined. Examines at most
    /// `max_probes` slots (<= [`MAX_PIECE_PROBES`]): no stored key is farther
    /// than that from its home bucket.
    fn lookup_counted(&self, key: &[u8]) -> (Option<u32>, usize) {
        let mut slot = fnv1a(key) as usize & self.table.mask;
        for probes in 1..=self.table.max_probes {
            match self.table.slots[slot] {
                0 => return (None, probes),
                entry => {
                    let piece = &self.pieces[entry as usize - 1];
                    if &self.bytes[piece.start..piece.start + piece.len] == key {
                        return (Some(entry - 1), probes);
                    }
                }
            }
            slot = (slot + 1) & self.table.mask;
        }
        (None, self.table.max_probes)
    }

    /// Probe statistics of the piece table (diagnostics/regression tests).
    #[doc(hidden)]
    pub fn piece_table_stats(&self) -> PieceTableStats {
        PieceTableStats {
            slots: self.table.slots.len(),
            max_probes: self.table.max_probes,
            build_probes: self.table.build_probes,
        }
    }

    /// Like an internal piece lookup, also returning the slots examined
    /// (diagnostics/regression tests).
    #[doc(hidden)]
    pub fn lookup_probes(&self, key: &[u8]) -> (Option<u32>, usize) {
        self.lookup_counted(key)
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
        self.encode_checked(text, &mut || false)
            .expect("never interrupted")
    }

    /// Like [`Tokenizer::encode`], but polls `stop` after normalization and
    /// before segmenting each pre-tokenized word; returns `None` as soon as
    /// `stop` returns `true`. Polling is cooperative: the work between two
    /// polls (normalizing the whole input, or one word's Viterbi pass) is not
    /// interrupted.
    pub fn encode_checked(&self, text: &str, stop: &mut dyn FnMut() -> bool) -> Option<Vec<u32>> {
        let words = self.pre_tokenize(text);
        let mut ids = vec![self.bos_id];
        for word in words {
            if stop() {
                return None;
            }
            self.segment(&word, &mut ids);
        }
        ids.push(self.eos_id);
        Some(ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::{Piece, PieceKind};

    fn tokenizer(pieces: &[(&str, f32, PieceKind)]) -> Tokenizer {
        let mut bytes = Vec::new();
        let mut list = Vec::new();
        for (text, score, kind) in pieces {
            list.push(Piece {
                score: *score,
                kind: *kind,
                start: bytes.len(),
                len: text.len(),
            });
            bytes.extend_from_slice(text.as_bytes());
        }
        Tokenizer::new(&bytes, &list, &[], 3, 0, 2).expect("tokenizer")
    }

    fn base() -> Tokenizer {
        use PieceKind::*;
        tokenizer(&[
            ("<s>", 0.0, Control),
            ("<pad>", 0.0, Control),
            ("</s>", 0.0, Control),
            ("<unk>", 0.0, Unknown),
            ("\u{2581}", -2.0, Normal),
            ("\u{2581}a", -1.0, Normal),
            ("a", -3.0, Normal),
            ("b", -3.0, Normal),
            ("\u{2581}ab", -1.5, Normal),
            ("ab", -10.0, Normal),
        ])
    }

    #[test]
    fn viterbi_prefers_the_highest_total_score() {
        // "▁ab" (-1.5) beats "▁a" + "b" (-4.0).
        assert_eq!(base().encode("ab"), [0, 8, 2]);
        // Without a prefix-space match: "▁a","b" vs "▁","ab": -4.0 vs -12.0.
        assert_eq!(base().encode("a b"), [0, 5, 4, 7, 2]);
    }

    #[test]
    fn consecutive_unknown_characters_fuse_into_one_unk() {
        // "▁" then two unknown chars fused, then "a".
        assert_eq!(base().encode("xyа"), [0, 4, 3, 2]);
        assert_eq!(base().encode("xya"), [0, 4, 3, 6, 2]);
    }

    #[test]
    fn spaces_collapse_and_split_into_metaspace_words() {
        let t = base();
        assert_eq!(t.pre_tokenize("a   b"), ["\u{2581}a", "\u{2581}b"]);
        assert_eq!(t.pre_tokenize(" a"), ["\u{2581}a"]);
        assert_eq!(t.pre_tokenize("a "), ["\u{2581}a", "\u{2581}"]);
        assert!(t.pre_tokenize("").is_empty());
        assert_eq!(t.encode(""), [0, 2]);
    }

    #[test]
    fn control_pieces_are_never_matched_from_text() {
        let t = base();
        let ids = t.encode("<s>");
        assert!(!ids[1..ids.len() - 1].contains(&0));
    }

    #[test]
    fn lookups_stop_after_the_longest_stored_probe() {
        let t = base();
        let stats = t.piece_table_stats();
        assert!(stats.max_probes >= 1 && stats.max_probes <= MAX_PIECE_PROBES);
        for (id, piece) in t.pieces.iter().enumerate() {
            if piece.kind == PieceKind::Normal {
                let key = &t.bytes[piece.start..piece.start + piece.len];
                let (found, probes) = t.lookup_probes(key);
                assert_eq!(found, Some(id as u32));
                assert!(probes <= stats.max_probes);
            }
        }
        assert!(t.lookup_probes(b"missing").1 <= stats.max_probes);
        // A table without normal pieces examines no slot at all.
        let empty = tokenizer(&[
            ("<s>", 0.0, PieceKind::Control),
            ("<pad>", 0.0, PieceKind::Control),
            ("</s>", 0.0, PieceKind::Control),
            ("<unk>", 0.0, PieceKind::Unknown),
        ]);
        assert_eq!(empty.lookup_probes(b"a"), (None, 0));
        assert_eq!(empty.encode("a"), [0, 3, 2]);
    }

    #[test]
    fn build_budget_is_linear_with_room_for_one_full_run() {
        assert_eq!(
            max_table_build_probes(0),
            MAX_PIECE_PROBES * MAX_PIECE_PROBES
        );
        assert_eq!(max_table_build_probes(250_002), 1_016_392);
        assert_eq!(max_table_build_probes(usize::MAX), usize::MAX);
    }

    #[test]
    fn malformed_charsmap_is_rejected() {
        assert!(CharsMap::parse(&[1, 0, 0, 0]).is_none());
        assert!(CharsMap::parse(&[8, 0, 0, 0, 1, 2]).is_none());
        assert!(CharsMap::parse(&[]).is_some());
    }
}
