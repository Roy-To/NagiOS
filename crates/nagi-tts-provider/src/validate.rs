//! Structural validation of voice and dictionary bytes before they reach the
//! upstream parsers.
//!
//! jbonsai 0.4.2 and lindera-dictionary 3.0.7 trust their inputs: several
//! malformed-but-well-framed inputs make them panic (index out of bounds,
//! `unwrap` on `None`, `todo!`, arithmetic overflow) either while loading or
//! later, during synthesis of an ordinary sentence. On Nagi the engine is
//! built with `panic = abort`, so a panic would take the whole provider
//! process down; on the host it would poison the caller. A partial or
//! corrupted Model Store read must instead surface as
//! [`LoadError::VoiceInvalid`] / [`LoadError::DictionaryInvalid`] at load
//! time.
//!
//! These checks are deliberately conservative: they accept every input the
//! upstream parsers accept *and* use safely for the pinned artifacts, and
//! reject the inconsistencies that lead to a panic. They validate structure
//! only. Content integrity (that the bytes are the pinned voice/dictionary)
//! is the SHA-256 pin in `tools/tts/tts-artifacts.lock`, checked by whoever
//! delivers the bytes.

use std::collections::{BTreeMap, BTreeSet};

use jpreprocess::Dictionary;

use crate::jbonsai_backend::{DictionaryBytes, LoadError};

/// Largest `NUM_STATES` accepted (the pinned voice uses 5).
pub(crate) const MAX_VOICE_STATES: usize = 32;
/// Streams jbonsai's vocoder indexes unconditionally (spectrum, log F0,
/// low-pass filter).
pub(crate) const MIN_VOICE_STREAMS: usize = 3;
/// Largest number of streams accepted (the pinned voice uses 3).
pub(crate) const MAX_VOICE_STREAMS: usize = 8;
/// Largest `VECTOR_LENGTH[*]` accepted (the pinned voice uses at most 35).
pub(crate) const MAX_VOICE_VECTOR_LENGTH: usize = 4_096;
/// Largest `NUM_WINDOWS[*]` accepted (the pinned voice uses at most 3).
pub(crate) const MAX_VOICE_WINDOWS: usize = 16;
/// Longest run of decimal digits accepted anywhere in the voice header.
/// jbonsai's header deserializer multiplies without overflow checks; 18
/// digits always fit in a `u64`.
pub(crate) const MAX_HEADER_DIGITS: usize = 18;
/// Largest coefficient count accepted for one `STREAM_WIN` row (the pinned
/// voice uses 1 and 3).
pub(crate) const MAX_WINDOW_WIDTH: usize = 15;
/// Largest duration-PDF mean accepted, in frames per HMM state (the pinned
/// voice's largest is about 94; 2,000 frames is 10 s at 5 ms frames).
/// jbonsai 0.4.2 rounds the mean and casts it to `usize` with saturation, so
/// `+inf` or `f32::MAX` would become a `usize::MAX`-frame state.
pub(crate) const MAX_DURATION_MEAN_FRAMES: f32 = 2_000.0;
/// Largest duration-PDF variance accepted (the pinned voice's largest is
/// about 2,203).
pub(crate) const MAX_DURATION_VARIANCE: f32 = 1.0e6;
/// Stream index jbonsai's vocoder reads as log F0; its static vector must
/// have exactly one element (`SpeechGenerator::new` panics otherwise).
const LF0_STREAM: usize = 1;
/// Stream index jbonsai's vocoder reads as low-pass filter coefficients; its
/// static vector length must be odd (`SpeechGenerator::new` panics
/// otherwise).
const LPF_STREAM: usize = 2;

// ---------------------------------------------------------------------------
// Voice (.htsvoice)
// ---------------------------------------------------------------------------

type Range = (usize, usize);

/// One model (duration, stream or GV) to validate: its tree text range, its
/// PDF binary range, the number of `f32` values per PDF, and the HMM states
/// that synthesis will look up in it.
struct ModelSpec {
    tree: Range,
    pdf: Range,
    pdf_len: usize,
    states: core::ops::RangeInclusive<usize>,
    role: PdfRole,
}

/// How the `f32` values of one PDF are used, which decides their numeric
/// invariants. Every PDF is `half` means, then `half` variances, then (MSD
/// streams only) one voiced weight.
#[derive(Clone, Copy)]
enum PdfRole {
    /// State durations: means become frame counts, variances divide.
    Duration,
    /// Spectrum / log F0 / LPF statistics, optionally with an MSD weight.
    Stream { msd: bool },
    /// Global variance statistics.
    GlobalVariance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TreeIndex {
    Node(i64),
    Pdf(u64),
}

struct Node {
    id: i64,
    question: String,
    yes: TreeIndex,
    no: TreeIndex,
}

struct Tree {
    state: usize,
    nodes: Vec<Node>,
}

/// Validates the structure of an `.htsvoice` whose `[POSITION]` ranges were
/// already checked to lie inside `[DATA]` (see `check_voice_layout`).
pub(crate) fn check_voice_structure(voice: &[u8]) -> Result<(), LoadError> {
    let invalid = LoadError::VoiceInvalid;
    const DATA: &[u8] = b"\n[DATA]\n";
    let data_marker = voice
        .windows(DATA.len())
        .position(|window| window == DATA)
        .ok_or(invalid)?;
    let header = core::str::from_utf8(&voice[..data_marker]).map_err(|_| invalid)?;
    let data = &voice[data_marker + DATA.len()..];
    check_header_digits(header)?;

    let mut sections: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
    let mut order = Vec::new();
    let mut current: Option<&str> = None;
    for line in header.split('\n') {
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if sections.insert(name, Vec::new()).is_some() {
                return Err(invalid);
            }
            order.push(name);
            current = Some(name);
            continue;
        }
        let section = current.ok_or(invalid)?;
        let (key, value) = line.split_once(':').ok_or(invalid)?;
        sections
            .get_mut(section)
            .ok_or(invalid)?
            .push((key.trim(), value.trim()));
    }
    if order != ["GLOBAL", "STREAM", "POSITION"] {
        return Err(invalid);
    }
    let lookup = |section: &str, key: &str| -> Option<&str> {
        let entries = sections.get(section)?;
        let mut found = entries.iter().filter(|(k, _)| *k == key).map(|(_, v)| *v);
        let first = found.next()?;
        // Duplicate keys are ambiguous; jbonsai's map would keep one of them.
        if found.next().is_some() {
            return Some("\u{0}duplicate");
        }
        Some(first)
    };
    let number = |section: &str, key: &str, max: usize| -> Result<usize, LoadError> {
        let value: usize = lookup(section, key)
            .ok_or(invalid)?
            .parse()
            .map_err(|_| invalid)?;
        if value == 0 || value > max {
            return Err(invalid);
        }
        Ok(value)
    };
    let flag = |section: &str, key: &str| -> Result<bool, LoadError> {
        match lookup(section, key) {
            None => Ok(false),
            Some("0") => Ok(false),
            Some("1") => Ok(true),
            Some(_) => Err(invalid),
        }
    };
    let range = |key: &str| -> Result<Range, LoadError> {
        let value = lookup("POSITION", key).ok_or(invalid)?;
        let (start, end) = value.split_once('-').ok_or(invalid)?;
        let start: usize = start.trim().parse().map_err(|_| invalid)?;
        let end: usize = end.trim().parse().map_err(|_| invalid)?;
        if start > end || end >= data.len() {
            return Err(invalid);
        }
        Ok((start, end))
    };

    let num_states = number("GLOBAL", "NUM_STATES", MAX_VOICE_STATES)?;
    let stream_types: Vec<&str> = lookup("GLOBAL", "STREAM_TYPE")
        .ok_or(invalid)?
        .split(',')
        .map(str::trim)
        .collect();
    // jbonsai's vocoder reads streams 0 (spectrum), 1 (log F0) and 2
    // (low-pass filter) unconditionally.
    if stream_types.len() < MIN_VOICE_STREAMS
        || stream_types.len() > MAX_VOICE_STREAMS
        || stream_types.iter().any(|name| name.is_empty())
    {
        return Err(invalid);
    }
    // jbonsai sizes its per-stream condition arrays (GV weights, MSD
    // thresholds, interpolation weights) with NUM_STREAMS and indexes them
    // 0..=2 during synthesis; it must agree with STREAM_TYPE.
    let num_streams = number("GLOBAL", "NUM_STREAMS", MAX_VOICE_STREAMS)?;
    if num_streams != stream_types.len() {
        return Err(invalid);
    }
    check_spectrum_options(lookup("STREAM", &format!("OPTION[{}]", stream_types[0])))?;
    let last_state = num_states.checked_add(1).ok_or(invalid)?;

    let mut models = vec![ModelSpec {
        tree: range("DURATION_TREE")?,
        pdf: range("DURATION_PDF")?,
        pdf_len: num_states.checked_mul(2).ok_or(invalid)?,
        states: 2..=2,
        role: PdfRole::Duration,
    }];
    let ranges = |key: &str| -> Result<Vec<Range>, LoadError> {
        let value = lookup("POSITION", key).ok_or(invalid)?;
        value
            .split(',')
            .map(|range| {
                let (start, end) = range.split_once('-').ok_or(invalid)?;
                let start: usize = start.trim().parse().map_err(|_| invalid)?;
                let end: usize = end.trim().parse().map_err(|_| invalid)?;
                if start > end || end >= data.len() {
                    return Err(invalid);
                }
                Ok((start, end))
            })
            .collect()
    };
    for (index, name) in stream_types.iter().enumerate() {
        let key = |field: &str| format!("{field}[{name}]");
        let vector_length = number("STREAM", &key("VECTOR_LENGTH"), MAX_VOICE_VECTOR_LENGTH)?;
        let windows = number("STREAM", &key("NUM_WINDOWS"), MAX_VOICE_WINDOWS)?;
        let is_msd = flag("STREAM", &key("IS_MSD"))?;
        let use_gv = flag("STREAM", &key("USE_GV"))?;
        // Role-specific static vector shapes jbonsai's SpeechGenerator::new
        // asserts (it panics instead of returning an error).
        if (index == LF0_STREAM && vector_length != 1)
            || (index == LPF_STREAM && vector_length.is_multiple_of(2))
        {
            return Err(invalid);
        }
        // MlpgAdjust indexes the PDF at `vector_length * window + i` for
        // every window row listed; more rows than NUM_WINDOWS (for example a
        // duplicated range) would index past the PDF, fewer would silently
        // drop dynamic features.
        let window_ranges = ranges(&key("STREAM_WIN"))?;
        if window_ranges.len() != windows {
            return Err(invalid);
        }
        for (start, end) in window_ranges {
            check_window(&data[start..=end])?;
        }
        let pdf_len = vector_length
            .checked_mul(windows)
            .and_then(|n| n.checked_mul(2))
            .and_then(|n| n.checked_add(usize::from(is_msd)))
            .ok_or(invalid)?;
        models.push(ModelSpec {
            tree: range(&key("STREAM_TREE"))?,
            pdf: range(&key("STREAM_PDF"))?,
            pdf_len,
            states: 2..=last_state,
            role: PdfRole::Stream { msd: is_msd },
        });
        if use_gv {
            models.push(ModelSpec {
                tree: range(&key("GV_TREE"))?,
                pdf: range(&key("GV_PDF"))?,
                pdf_len: vector_length.checked_mul(2).ok_or(invalid)?,
                states: 2..=2,
                role: PdfRole::GlobalVariance,
            });
        }
    }
    for model in &models {
        check_model(data, model)?;
    }
    Ok(())
}

/// The spectrum stream's options, which jbonsai's `Condition::load_model`
/// reads: `GAMMA` is the MGLSA stage count (it sizes
/// `vec![vec![0.0; nmcp]; stage]`), `LN_GAIN` a flag, `ALPHA` the
/// frequency-warping factor. The supported contract is the pinned voice's
/// mel-cepstral (MLSA) vocoder: `GAMMA` absent or exactly `0`, `LN_GAIN`
/// absent, `0` or `1`, `ALPHA` absent or finite in [0, 1). A key may appear
/// once. Other keys are ignored by jbonsai and accepted here.
fn check_spectrum_options(value: Option<&str>) -> Result<(), LoadError> {
    let invalid = LoadError::VoiceInvalid;
    let Some(value) = value else {
        return Ok(());
    };
    if value.starts_with('\u{0}') {
        // Duplicate OPTION line (see `lookup`).
        return Err(invalid);
    }
    let mut seen = BTreeSet::new();
    for option in value.split(',') {
        let Some((key, setting)) = option.split_once('=') else {
            // jbonsai skips options without `=`.
            continue;
        };
        let (key, setting) = (key.trim(), setting.trim());
        if !seen.insert(key) {
            return Err(invalid);
        }
        let ok = match key {
            "GAMMA" => setting == "0",
            "LN_GAIN" => setting == "0" || setting == "1",
            "ALPHA" => setting
                .parse::<f64>()
                .is_ok_and(|alpha| alpha.is_finite() && (0.0..1.0).contains(&alpha)),
            _ => true,
        };
        if !ok {
            return Err(invalid);
        }
    }
    Ok(())
}

/// One `STREAM_WIN` row as jbonsai parses it: a coefficient count followed
/// by that many decimal coefficients, separated by spaces, then trailing
/// separators. Accepts only odd widths up to [`MAX_WINDOW_WIDTH`] and finite
/// coefficients.
fn check_window(text: &[u8]) -> Result<(), LoadError> {
    let invalid = LoadError::VoiceInvalid;
    let text = core::str::from_utf8(text).map_err(|_| invalid)?;
    let body = text.trim_end_matches([' ', '\n']);
    if body.contains(['\n', '\t', '\r']) {
        return Err(invalid);
    }
    let mut tokens = body.split(' ').filter(|token| !token.is_empty());
    let count = tokens.next().ok_or(invalid)?;
    if count.is_empty() || !count.bytes().all(|b| b.is_ascii_digit()) || count.len() > 3 {
        return Err(invalid);
    }
    let count: usize = count.parse().map_err(|_| invalid)?;
    if count == 0 || count > MAX_WINDOW_WIDTH || count.is_multiple_of(2) {
        return Err(invalid);
    }
    let mut seen = 0usize;
    for token in tokens {
        let value: f64 = token.parse().map_err(|_| invalid)?;
        if !value.is_finite() {
            return Err(invalid);
        }
        seen += 1;
    }
    if seen != count {
        return Err(invalid);
    }
    Ok(())
}

/// Numeric invariants of one model's PDF values (`values` holds whole PDFs
/// of `pdf_len` floats each).
fn check_pdf_values(values: &[u8], pdf_len: usize, role: PdfRole) -> Result<(), LoadError> {
    let invalid = LoadError::VoiceInvalid;
    let msd = matches!(role, PdfRole::Stream { msd: true });
    let half = (pdf_len - usize::from(msd)) / 2;
    let floats: Vec<f32> = values
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    for pdf in floats.chunks_exact(pdf_len) {
        if pdf.iter().any(|value| !value.is_finite()) {
            return Err(invalid);
        }
        let (means, rest) = pdf.split_at(half);
        let (variances, weight) = rest.split_at(half);
        let ok = match role {
            // Means become per-state frame counts; variances are divisors
            // when the speaking rate is changed.
            PdfRole::Duration => {
                means
                    .iter()
                    .all(|mean| (0.0..=MAX_DURATION_MEAN_FRAMES).contains(mean))
                    && variances
                        .iter()
                        .all(|vari| *vari > 0.0 && *vari <= MAX_DURATION_VARIANCE)
            }
            // The pinned LPF stream carries zero variances; jbonsai maps them
            // to a large inverse variance, so only negatives are rejected.
            PdfRole::Stream { .. } | PdfRole::GlobalVariance => {
                variances.iter().all(|vari| *vari >= 0.0)
                    && weight.iter().all(|w| (0.0..=1.0).contains(w))
            }
        };
        if !ok {
            return Err(invalid);
        }
    }
    Ok(())
}

fn check_header_digits(header: &str) -> Result<(), LoadError> {
    let mut run = 0usize;
    for byte in header.bytes() {
        if byte.is_ascii_digit() {
            run += 1;
            if run > MAX_HEADER_DIGITS {
                return Err(LoadError::VoiceInvalid);
            }
        } else {
            run = 0;
        }
    }
    Ok(())
}

fn check_model(data: &[u8], spec: &ModelSpec) -> Result<(), LoadError> {
    let invalid = LoadError::VoiceInvalid;
    let text = core::str::from_utf8(&data[spec.tree.0..=spec.tree.1]).map_err(|_| invalid)?;
    let (questions, trees) = parse_tree_section(text)?;
    if trees.is_empty() {
        return Err(invalid);
    }

    // PDF section: one u32 count per tree, then count * pdf_len f32 per tree.
    let pdf = &data[spec.pdf.0..=spec.pdf.1];
    let header_bytes = trees.len().checked_mul(4).ok_or(invalid)?;
    if pdf.len() < header_bytes {
        return Err(invalid);
    }
    let counts: Vec<u64> = pdf[..header_bytes]
        .chunks_exact(4)
        .map(|c| u64::from(u32::from_le_bytes([c[0], c[1], c[2], c[3]])))
        .collect();
    let pdf_bytes = spec.pdf_len.checked_mul(4).ok_or(invalid)?;
    let mut expected = header_bytes;
    for count in &counts {
        let count = usize::try_from(*count).map_err(|_| invalid)?;
        expected = count
            .checked_mul(pdf_bytes)
            .and_then(|bytes| expected.checked_add(bytes))
            .ok_or(invalid)?;
    }
    if expected != pdf.len() {
        return Err(invalid);
    }
    check_pdf_values(&pdf[header_bytes..], spec.pdf_len, spec.role)?;

    // Every state synthesis looks up must have a tree.
    for state in spec.states.clone() {
        if !trees.iter().any(|tree| tree.state == state) {
            return Err(invalid);
        }
    }

    for (tree, count) in trees.iter().zip(&counts) {
        check_tree(tree, *count, &questions)?;
    }
    Ok(())
}

fn check_tree(tree: &Tree, pdf_count: u64, questions: &BTreeSet<&str>) -> Result<(), LoadError> {
    let invalid = LoadError::VoiceInvalid;
    if tree.nodes.is_empty() {
        return Err(invalid);
    }
    let leaf_ok = |index: u64| index >= 1 && index <= pdf_count;
    let single = &tree.nodes[0];
    if tree.nodes.len() == 1 && single.yes == single.no {
        // jbonsai turns this into a single leaf; a node reference here is a
        // `todo!()` in its tree converter.
        return match single.yes {
            TreeIndex::Pdf(index) if leaf_ok(index) => Ok(()),
            _ => Err(invalid),
        };
    }
    let mut position = BTreeMap::new();
    for (index, node) in tree.nodes.iter().enumerate() {
        if position.insert(node.id, index).is_some() {
            return Err(invalid);
        }
    }
    let mut children = Vec::with_capacity(tree.nodes.len());
    for node in &tree.nodes {
        if !questions.contains(node.question.as_str()) {
            return Err(invalid);
        }
        let mut edges = [None, None];
        for (slot, target) in edges.iter_mut().zip([node.yes, node.no]) {
            match target {
                TreeIndex::Node(id) => *slot = Some(*position.get(&id).ok_or(invalid)?),
                TreeIndex::Pdf(index) if leaf_ok(index) => {}
                TreeIndex::Pdf(_) => return Err(invalid),
            }
        }
        children.push(edges);
    }
    // The search loop follows node links until it reaches a leaf; a cycle
    // would never terminate.
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Open,
        Done,
    }
    let mut marks = vec![Mark::New; tree.nodes.len()];
    for root in 0..tree.nodes.len() {
        if marks[root] != Mark::New {
            continue;
        }
        let mut stack = vec![(root, 0usize)];
        marks[root] = Mark::Open;
        while let Some((node, edge)) = stack.last_mut() {
            let node = *node;
            if *edge == 2 {
                marks[node] = Mark::Done;
                stack.pop();
                continue;
            }
            let next = children[node][*edge];
            *edge += 1;
            if let Some(child) = next {
                match marks[child] {
                    Mark::Open => return Err(invalid),
                    Mark::Done => {}
                    Mark::New => {
                        marks[child] = Mark::Open;
                        stack.push((child, 0));
                    }
                }
            }
        }
    }
    Ok(())
}

/// Splits on the separators jbonsai uses (space and newline only) and
/// detaches braces that its grammar allows to touch a neighbouring token
/// (`{0 ...`, `... -3}`), except in the `{*}` tree header.
fn tokenize(text: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    for token in text.split([' ', '\n']).filter(|t| !t.is_empty()) {
        if token.starts_with("{*}") {
            tokens.push(token);
            continue;
        }
        let mut rest = token;
        if rest.len() > 1 && rest.starts_with('{') {
            tokens.push("{");
            rest = &rest[1..];
        }
        if rest.len() > 1 && rest.ends_with('}') {
            tokens.push(&rest[..rest.len() - 1]);
            tokens.push("}");
        } else {
            tokens.push(rest);
        }
    }
    tokens
}

/// Parses the question list and trees of one model's tree section.
fn parse_tree_section(text: &str) -> Result<(BTreeSet<&str>, Vec<Tree>), LoadError> {
    let invalid = LoadError::VoiceInvalid;
    let tokens = tokenize(text);
    let mut at = 0usize;
    let mut next = || -> Result<&str, LoadError> {
        let token = tokens.get(at).copied().ok_or(invalid)?;
        at += 1;
        Ok(token)
    };
    let mut questions = BTreeSet::new();
    let mut pending = None;
    while let Ok(token) = next() {
        if token != "QS" {
            pending = Some(token);
            break;
        }
        let name = next()?;
        if !name.is_ascii() || !questions.insert(name) {
            // Duplicate names: jbonsai's lookup keeps the last one; reject
            // the ambiguity.
            return Err(invalid);
        }
        if next()? != "{" {
            return Err(invalid);
        }
        while next()? != "}" {}
    }

    let mut trees = Vec::new();
    while let Some(token) = pending.take().or_else(|| next().ok()) {
        let rest = token.strip_prefix("{*}").ok_or(invalid)?;
        let state_token = if rest.is_empty() { next()? } else { rest };
        let state: usize = state_token
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
            .ok_or(invalid)?
            .parse()
            .map_err(|_| invalid)?;
        let body = next()?;
        if body != "{" {
            let leaf = parse_index(body)?;
            trees.push(Tree {
                state,
                nodes: vec![Node {
                    id: 0,
                    question: String::new(),
                    yes: leaf,
                    no: leaf,
                }],
            });
            continue;
        }
        let mut nodes = Vec::new();
        loop {
            let first = next()?;
            if first == "}" {
                break;
            }
            let id = parse_node_id(first)?;
            let question = next()?;
            if !question.is_ascii() {
                return Err(invalid);
            }
            let no = parse_index(next()?)?;
            let yes = parse_index(next()?)?;
            nodes.push(Node {
                id,
                question: question.to_owned(),
                yes,
                no,
            });
        }
        trees.push(Tree { state, nodes });
    }
    Ok((questions, trees))
}

fn parse_node_id(token: &str) -> Result<i64, LoadError> {
    let digits = token.strip_prefix('-').unwrap_or(token);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(LoadError::VoiceInvalid);
    }
    token.parse().map_err(|_| LoadError::VoiceInvalid)
}

/// Mirrors jbonsai's tree index grammar: signed digits are a node reference
/// (quoted or not); any other identifier is a PDF leaf whose index is its
/// trailing run of digits.
fn parse_index(token: &str) -> Result<TreeIndex, LoadError> {
    let invalid = LoadError::VoiceInvalid;
    let inner = match token.strip_prefix('"') {
        Some(rest) => rest.strip_suffix('"').ok_or(invalid)?,
        None => token,
    };
    if parse_node_id(inner).is_ok() {
        return parse_node_id(inner).map(TreeIndex::Node);
    }
    if inner.is_empty()
        || !inner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(invalid);
    }
    let digits = inner.len() - inner.bytes().rev().take_while(u8::is_ascii_digit).count();
    let index: u64 = inner[digits..].parse().map_err(|_| invalid)?;
    Ok(TreeIndex::Pdf(index))
}

// ---------------------------------------------------------------------------
// Dictionary (jpreprocess / lindera system dictionary)
// ---------------------------------------------------------------------------

/// Serialized size of one lindera `WordEntry` in `dict.vals`.
const WORD_ENTRY_BYTES: usize = 10;

/// Connection matrix dimensions parsed from `matrix.mtx`.
#[derive(Clone, Copy)]
pub(crate) struct MatrixShape {
    forward: u32,
    backward: u32,
}

/// Checks the raw dictionary components that the lindera/jpreprocess
/// loaders index without bounds checks. Runs before any loader.
pub(crate) fn check_dictionary_bytes(dict: &DictionaryBytes) -> Result<MatrixShape, LoadError> {
    let shape = check_matrix(&dict.matrix_mtx)?;
    check_word_entries(&dict.dict_vals, shape)?;
    check_words(&dict.dict_wordsidx, &dict.dict_words)?;
    Ok(shape)
}

/// `matrix.mtx`: the cost table length must equal the declared shape, or
/// lindera's loader and its per-pair cost lookup index past the table.
fn check_matrix(matrix: &[u8]) -> Result<MatrixShape, LoadError> {
    let invalid = LoadError::DictionaryInvalid;
    let read = |at: usize| -> Option<i16> {
        matrix
            .get(at..at + 2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
    };
    let first = read(0).ok_or(invalid)?;
    let (forward, backward, header) = if first == -1 {
        (read(2).ok_or(invalid)?, read(4).ok_or(invalid)?, 6usize)
    } else {
        (first, read(2).ok_or(invalid)?, 4usize)
    };
    if forward <= 0 || backward <= 0 {
        return Err(invalid);
    }
    let (forward, backward) = (forward as u32, backward as u32);
    let cells = (forward as usize) * (backward as usize);
    let expected = cells
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_add(header))
        .ok_or(invalid)?;
    if matrix.len() != expected {
        return Err(invalid);
    }
    Ok(MatrixShape { forward, backward })
}

/// `dict.vals`: whole entries only, and every entry's context ids inside
/// the connection matrix.
fn check_word_entries(vals: &[u8], shape: MatrixShape) -> Result<(), LoadError> {
    if !vals.len().is_multiple_of(WORD_ENTRY_BYTES) {
        return Err(LoadError::DictionaryInvalid);
    }
    for entry in vals.chunks_exact(WORD_ENTRY_BYTES) {
        let left = u32::from(u16::from_le_bytes([entry[6], entry[7]]));
        let right = u32::from(u16::from_le_bytes([entry[8], entry[9]]));
        if !context_ids_ok(left, right, shape) {
            return Err(LoadError::DictionaryInvalid);
        }
    }
    Ok(())
}

/// lindera looks up `cost(prev.right_id, next.left_id)` at
/// `right + left * forward`.
fn context_ids_ok(left: u32, right: u32, shape: MatrixShape) -> bool {
    right < shape.forward && left < shape.backward
}

/// `dict.wordsidx` / `dict.words`: offsets must be whole u32s, non-decreasing
/// and inside `dict.words` (jpreprocess slices `words[idx[i]..idx[i + 1]]`),
/// and the preamble must identify a jpreprocess dictionary. Any other
/// identification sends tokens down lindera's own detail decoder, which
/// slices the jpreprocess-encoded records without bounds checks.
fn check_words(index: &[u8], words: &[u8]) -> Result<(), LoadError> {
    let invalid = LoadError::DictionaryInvalid;
    if !index.len().is_multiple_of(4) || index.len() < 4 {
        return Err(invalid);
    }
    let mut previous = 0usize;
    for offset in index.chunks_exact(4) {
        let offset = u32::from_le_bytes([offset[0], offset[1], offset[2], offset[3]]) as usize;
        if offset < previous || offset > words.len() {
            return Err(invalid);
        }
        previous = offset;
    }
    let preamble_end = u32::from_le_bytes([index[0], index[1], index[2], index[3]]) as usize;
    let preamble = core::str::from_utf8(&words[..preamble_end]).map_err(|_| invalid)?;
    if !preamble.to_lowercase().starts_with("jpreprocess") {
        return Err(invalid);
    }
    Ok(())
}

/// Checks the loaded unknown-word dictionary and character definitions,
/// whose cross references lindera indexes without bounds checks.
pub(crate) fn check_loaded_dictionary(
    dictionary: &Dictionary,
    shape: MatrixShape,
) -> Result<(), LoadError> {
    let invalid = LoadError::DictionaryInvalid;
    let matrix = &dictionary.connection_cost_matrix;
    let cells = (matrix.forward_size as usize).checked_mul(matrix.backward_size as usize);
    if matrix.forward_size != shape.forward
        || matrix.backward_size != shape.backward
        || cells != Some(matrix.costs_data.len())
    {
        return Err(invalid);
    }
    let unknown = &dictionary.unknown_dictionary;
    for entry in &unknown.costs {
        if !context_ids_ok(entry.left_id(), entry.right_id(), shape) {
            return Err(invalid);
        }
    }
    let categories = dictionary.character_definition.category_definitions.len();
    if categories == 0
        || dictionary.character_definition.category_names.len() != categories
        || unknown.category_references.len() < categories
    {
        return Err(invalid);
    }
    let unknown_words = unknown.costs.len();
    if unknown
        .category_references
        .iter()
        .flatten()
        .any(|word| *word as usize >= unknown_words)
    {
        return Err(invalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
