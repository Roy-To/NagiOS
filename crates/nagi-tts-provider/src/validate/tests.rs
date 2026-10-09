//! Model-free adversarial tests for the voice and dictionary validators.
//!
//! A small synthetic `.htsvoice` (three streams, one state) is built in
//! memory so jbonsai can parse and synthesize it without the pinned
//! artifacts; every malformed variant must come back as an error, never a
//! panic, an abort or a hang.

use std::panic::{catch_unwind, AssertUnwindSafe};

use jbonsai::Engine;

use super::*;
use crate::jbonsai_backend::{DictionaryBytes, JbonsaiBackend, LoadError};

/// Full-context labels from jbonsai's own test sentence (HTS_TTS_JPN).
const LABELS: [&str; 4] = [
    "xx^xx-sil+b=o/A:xx+xx+xx/B:xx-xx_xx/C:xx_xx+xx/D:xx+xx_xx/E:xx_xx!xx_xx-xx/F:xx_xx#xx_xx@xx_xx|xx_xx/G:4_4%0_xx_xx/H:xx_xx/I:xx-xx@xx+xx&xx-xx|xx+xx/J:1_4/K:1+1-4",
    "xx^sil-b+o=N/A:-3+1+4/B:xx-xx_xx/C:02_xx+xx/D:xx+xx_xx/E:xx_xx!xx_xx-xx/F:4_4#0_xx@1_1|1_4/G:xx_xx%xx_xx_xx/H:xx_xx/I:1-4@1+1&1-1|1+4/J:xx_xx/K:1+1-4",
    "sil^b-o+N=s/A:-3+1+4/B:xx-xx_xx/C:02_xx+xx/D:xx+xx_xx/E:xx_xx!xx_xx-xx/F:4_4#0_xx@1_1|1_4/G:xx_xx%xx_xx_xx/H:xx_xx/I:1-4@1+1&1-1|1+4/J:xx_xx/K:1+1-4",
    "b^o-N+s=a/A:-2+2+3/B:xx-xx_xx/C:02_xx+xx/D:xx+xx_xx/E:xx_xx!xx_xx-xx/F:4_4#0_xx@1_1|1_4/G:xx_xx%xx_xx_xx/H:xx_xx/I:1-4@1+1&1-1|1+4/J:xx_xx/K:1+1-4",
];

const QUESTIONS: &str = "QS C-Phone_o { \"*-o+*\" }\nQS C-Phone_b { \"*-b+*\" }\n\n";

/// One model of the synthetic voice: tree text plus PDF counts and values.
#[derive(Clone)]
struct ModelBytes {
    tree: String,
    counts: Vec<u32>,
    pdf_len: usize,
}

impl ModelBytes {
    fn pdf(&self, value: impl Fn(usize) -> f32) -> Vec<u8> {
        let mut bytes = Vec::new();
        for count in &self.counts {
            bytes.extend_from_slice(&count.to_le_bytes());
        }
        let total: usize = self.counts.iter().map(|c| *c as usize).sum();
        for index in 0..total * self.pdf_len {
            bytes.extend_from_slice(&value(index % self.pdf_len).to_le_bytes());
        }
        bytes
    }
}

/// Builder for a minimal valid three-stream, one-state voice.
#[derive(Clone)]
struct Voice {
    global: String,
    stream: String,
    duration: ModelBytes,
    streams: [ModelBytes; 3],
    windows: [&'static str; 3],
    /// How many times each stream's `STREAM_WIN` range is listed.
    window_repeat: [usize; 3],
    extra_position: String,
    /// `(pdf value index, value)` written into every duration PDF.
    duration_value: Option<(usize, f32)>,
    /// `(stream, pdf value index, value)` written into every PDF of a stream.
    stream_value: Option<(usize, usize, f32)>,
}

fn two_leaf_tree(prefix: &str) -> String {
    format!(
        "{QUESTIONS}{{*}}[2]\n{{\n   0 C-Phone_o  -1  \"{prefix}_s2_1\"\n  -1 C-Phone_b  \"{prefix}_s2_2\"  \"{prefix}_s2_1\"\n}}\n"
    )
}

impl Voice {
    fn valid() -> Self {
        let model = |prefix: &str, pdf_len: usize| ModelBytes {
            tree: two_leaf_tree(prefix),
            counts: vec![2],
            pdf_len,
        };
        Self {
            global: "HTS_VOICE_VERSION:1.0\nSAMPLING_FREQUENCY:48000\nFRAME_PERIOD:240\nNUM_STATES:1\nNUM_STREAMS:3\nSTREAM_TYPE:MCP,LF0,LPF\nFULLCONTEXT_FORMAT:HTS_TTS_JPN\nFULLCONTEXT_VERSION:1.0\nGV_OFF_CONTEXT:\"*-sil+*\"\nCOMMENT:\n".into(),
            stream: "VECTOR_LENGTH[MCP]:3\nVECTOR_LENGTH[LF0]:1\nVECTOR_LENGTH[LPF]:1\nIS_MSD[MCP]:0\nIS_MSD[LF0]:1\nIS_MSD[LPF]:0\nNUM_WINDOWS[MCP]:1\nNUM_WINDOWS[LF0]:1\nNUM_WINDOWS[LPF]:1\nUSE_GV[MCP]:0\nUSE_GV[LF0]:0\nUSE_GV[LPF]:0\nOPTION[MCP]:ALPHA=0.55\nOPTION[LF0]:\nOPTION[LPF]:\n".into(),
            duration: model("dur", 2),
            streams: [model("mcp", 6), model("lf0", 3), model("lpf", 2)],
            windows: ["1 1.0\n", "1 1.0\n", "1 1.0\n"],
            window_repeat: [1; 3],
            extra_position: String::new(),
            duration_value: None,
            stream_value: None,
        }
    }

    fn bytes(&self) -> Vec<u8> {
        // Means: duration 5 frames, spectrum/LPF small, LF0 voiced ~ log(150).
        let duration_value = self.duration_value;
        let duration = move |i: usize| match duration_value {
            Some((at, value)) if at == i => value,
            _ if i == 0 => 5.0,
            _ => 1.0,
        };
        let override_value = self.stream_value;
        let stream_value = |stream: usize| {
            move |i: usize| match (override_value, stream, i) {
                (Some((s, at, value)), _, _) if s == stream && at == i => value,
                (_, 1, 0) => 5.0,
                (_, 1, 2) => 1.0,
                (_, _, 0) => 0.1,
                _ => 1.0,
            }
        };
        let mut data = Vec::new();
        let mut position = String::new();
        fn push_position(position: &mut String, name: &str, ranges: &str) {
            position.push_str(&format!("{name}:{ranges}\n"));
        }
        let push = |position: &mut String, name: &str, bytes: &[u8], data: &mut Vec<u8>| {
            let start = data.len();
            data.extend_from_slice(bytes);
            push_position(position, name, &format!("{start}-{}", data.len() - 1));
        };
        push(
            &mut position,
            "DURATION_PDF",
            &self.duration.pdf(duration),
            &mut data,
        );
        push(
            &mut position,
            "DURATION_TREE",
            self.duration.tree.as_bytes(),
            &mut data,
        );
        for (index, name) in ["MCP", "LF0", "LPF"].iter().enumerate() {
            let start = data.len();
            data.extend_from_slice(self.windows[index].as_bytes());
            let range = format!("{start}-{}", data.len() - 1);
            let listed = vec![range; self.window_repeat[index]].join(",");
            push_position(&mut position, &format!("STREAM_WIN[{name}]"), &listed);
            push(
                &mut position,
                &format!("STREAM_PDF[{name}]"),
                &self.streams[index].pdf(stream_value(index)),
                &mut data,
            );
            push(
                &mut position,
                &format!("STREAM_TREE[{name}]"),
                self.streams[index].tree.as_bytes(),
                &mut data,
            );
        }
        let mut out = format!(
            "[GLOBAL]\n{}[STREAM]\n{}[POSITION]\n{}{}[DATA]\n",
            self.global, self.stream, position, self.extra_position
        )
        .into_bytes();
        out.extend_from_slice(&data);
        out
    }
}

/// What a byte string does when handed to the provider's loader and then,
/// if it loads, to jbonsai's synthesis of the fixed labels.
#[derive(Debug, PartialEq)]
enum Outcome {
    Rejected(LoadError),
    /// Passed validation and parsing; synthesis returned this many samples
    /// or an engine error.
    Synthesized(Result<usize, ()>),
}

fn outcome(voice: &[u8]) -> Outcome {
    match JbonsaiBackend::from_bytes(voice, DictionaryBytes::default()) {
        // An empty dictionary is reported only after the voice was fully
        // validated, parsed and its rate/frame period accepted.
        Err(LoadError::DictionaryMissing) => {}
        Err(error) => return Outcome::Rejected(error),
        Ok(_) => panic!("an empty dictionary must not load"),
    }
    let engine = Engine::load_from_bytes([voice]).expect("validated voice parses");
    let labels: Vec<jlabel::Label> = LABELS.iter().map(|l| l.parse().unwrap()).collect();
    let samples = engine.generator(labels).map(|mut generator| {
        // The provider rejects frame periods above MAX_ENGINE_FRAME_SAMPLES
        // at load, so this buffer is bounded.
        let mut frame = vec![0.0f64; generator.fperiod()];
        let mut total = 0usize;
        loop {
            let produced = generator.generate_step(&mut frame);
            if produced == 0 {
                break total;
            }
            total += produced;
            assert!(total <= 48_000 * 60, "synthesis must terminate");
        }
    });
    Outcome::Synthesized(samples.map_err(|_| ()))
}

#[test]
fn synthetic_voice_validates_parses_and_synthesizes() {
    let voice = Voice::valid().bytes();
    assert_eq!(check_voice_structure(&voice), Ok(()));
    match outcome(&voice) {
        Outcome::Synthesized(Ok(samples)) => assert!(samples > 0),
        other => panic!("synthetic voice must synthesize: {other:?}"),
    }
}

fn assert_invalid(voice: &Voice, case: &str) {
    let bytes = voice.bytes();
    assert_eq!(
        check_voice_structure(&bytes),
        Err(LoadError::VoiceInvalid),
        "{case}: validator"
    );
    assert_eq!(
        JbonsaiBackend::from_bytes(&bytes, DictionaryBytes::default()).err(),
        Some(LoadError::VoiceInvalid),
        "{case}: loader"
    );
}

/// Each case below makes jbonsai 0.4.2 panic (while parsing or during
/// synthesis) when the validator is bypassed.
#[test]
fn malformed_trees_are_rejected_not_panicking() {
    let edit_tree = |model: usize, from: &str, to: &str| {
        let mut voice = Voice::valid();
        let target = if model == 0 {
            &mut voice.duration
        } else {
            &mut voice.streams[model - 1]
        };
        assert!(target.tree.contains(from), "{from}");
        target.tree = target.tree.replacen(from, to, 1);
        voice
    };
    let cases = [
        // Node question not declared: `unwrap()` on None in the converter.
        (
            edit_tree(0, "0 C-Phone_o", "0 C-Phone_x"),
            "unknown question",
        ),
        // Node reference to a missing node: `unwrap()` on None.
        (
            edit_tree(1, "-1  \"mcp_s2_1\"", "-7  \"mcp_s2_1\""),
            "missing node",
        ),
        // PDF index 0: `pdf_index - 1` underflows.
        (edit_tree(2, "\"lf0_s2_2\"", "\"lf0_s2_0\""), "leaf index 0"),
        // PDF index beyond the PDF count: index out of bounds.
        (
            edit_tree(3, "\"lpf_s2_2\"", "\"lpf_s2_9\""),
            "leaf index past count",
        ),
        // Tree for a state synthesis never asks for; state 2 has no tree:
        // `todo!("index not found!")`.
        (edit_tree(0, "{*}[2]", "{*}[3]"), "missing duration state"),
        (edit_tree(1, "{*}[2]", "{*}[7]"), "missing stream state"),
        // Child pointing back at the root: the tree search never ends.
        (
            edit_tree(2, "\"lf0_s2_2\"  \"lf0_s2_1\"", "0  \"lf0_s2_1\""),
            "cycle",
        ),
        (
            edit_tree(3, "-1 C-Phone_b", "-0 C-Phone_b"),
            "duplicate node id",
        ),
        (
            edit_tree(1, "QS C-Phone_b", "QS C-Phone_o"),
            "duplicate question name",
        ),
        (edit_tree(0, "{*}[2]", "{*}[-2]"), "negative state"),
        (
            edit_tree(0, "C-Phone_b  \"dur", "C-Phone_\u{e9}  \"dur"),
            "non-ASCII question",
        ),
        (
            edit_tree(0, "\"dur_s2_2\"", "\"dur_s2_x\""),
            "leaf without index",
        ),
    ];
    for (voice, case) in &cases {
        assert_invalid(voice, case);
    }

    // A single node whose branches are the same node: `todo!()`.
    let mut voice = Voice::valid();
    voice.streams[0].tree = format!("{QUESTIONS}{{*}}[2]\n{{\n   0 C-Phone_o  0  0\n}}\n");
    assert_invalid(&voice, "single self-referencing node");

    // Non-UTF-8 tree text.
    let mut bytes = Voice::valid().bytes();
    let at = bytes.windows(8).position(|w| w == b"C-Phone_").unwrap();
    bytes[at] = 0xFF;
    assert_eq!(check_voice_structure(&bytes), Err(LoadError::VoiceInvalid));
}

#[test]
fn malformed_headers_and_pdfs_are_rejected_not_panicking() {
    let edit = |field: &str, from: &str, to: &str| {
        let mut voice = Voice::valid();
        let target = if field == "global" {
            &mut voice.global
        } else {
            &mut voice.stream
        };
        assert!(target.contains(from), "{from}");
        *target = target.replacen(from, to, 1);
        voice
    };
    let mut cases = vec![
        // jbonsai's header deserializer multiplies without overflow checks.
        (
            edit("global", "NUM_STATES:1", "NUM_STATES:99999999999999999999"),
            "digit overflow",
        ),
        (
            edit(
                "stream",
                "VECTOR_LENGTH[MCP]:3",
                "VECTOR_LENGTH[MCP]:999999999999999999",
            ),
            "vector length overflow",
        ),
        (
            edit("stream", "NUM_WINDOWS[LF0]:1", "NUM_WINDOWS[LF0]:17"),
            "too many windows",
        ),
        (
            edit("global", "NUM_STATES:1", "NUM_STATES:0"),
            "zero states",
        ),
        (
            edit("global", "NUM_STATES:1", "NUM_STATES:33"),
            "too many states",
        ),
        (edit("stream", "IS_MSD[LF0]:1", "IS_MSD[LF0]:2"), "bad flag"),
        (
            edit("stream", "VECTOR_LENGTH[LPF]:1\n", ""),
            "missing stream key",
        ),
        (
            edit("global", "STREAM_TYPE:MCP,LF0,LPF", "STREAM_TYPE:MCP,LF0"),
            "fewer than three streams",
        ),
        (
            edit(
                "global",
                "STREAM_TYPE:MCP,LF0,LPF",
                "STREAM_TYPE:MCP,LF0,,LPF",
            ),
            "empty stream name",
        ),
        (
            edit("global", "NUM_STATES:1\n", "NUM_STATES:1\nNUM_STATES:1\n"),
            "duplicate key",
        ),
        (
            edit("stream", "USE_GV[MCP]:0", "USE_GV[MCP]:1"),
            "GV without GV positions",
        ),
    ];
    // PDF counts that disagree with the PDF bytes.
    let mut voice = Voice::valid();
    voice.streams[1].counts = vec![1];
    let mut bytes_voice = voice.clone();
    bytes_voice.streams[1].counts = vec![2];
    let good_len = bytes_voice.bytes().len();
    assert_eq!(voice.bytes().len(), good_len - 3 * 4);
    cases.push((voice, "short PDF section"));
    let mut voice = Voice::valid();
    voice.duration.counts = vec![2, 0];
    cases.push((voice, "PDF count for a missing tree"));
    for (voice, case) in &cases {
        assert_invalid(voice, case);
    }

    // Section layout.
    let good = Voice::valid().bytes();
    let text = String::from_utf8_lossy(&good).into_owned();
    for (from, to, case) in [
        ("[STREAM]\n", "[STREEM]\n", "unknown section"),
        ("[GLOBAL]\n", "[GLOBAL]\n[GLOBAL]\n", "duplicate section"),
        (
            "HTS_VOICE_VERSION:1.0\n",
            "HTS_VOICE_VERSION\n",
            "line without colon",
        ),
    ] {
        let bytes = text.replacen(from, to, 1).into_bytes();
        assert_eq!(
            JbonsaiBackend::from_bytes(&bytes, DictionaryBytes::default()).err(),
            Some(LoadError::VoiceInvalid),
            "{case}"
        );
    }
}

/// Deterministic xorshift so failures reproduce exactly.
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Every truncation and a fixed set of byte mutations of the synthetic voice
/// either is rejected at load time or synthesizes to completion. No input
/// may panic (a panic aborts the provider on Nagi) or hang.
#[test]
fn bounded_voice_mutations_never_panic() {
    let good = Voice::valid().bytes();
    let mut survivors = 0usize;
    let mut check = |bytes: &[u8], what: &dyn Fn() -> String| {
        let result = catch_unwind(AssertUnwindSafe(|| outcome(bytes)));
        match result {
            Ok(Outcome::Rejected(_)) => {}
            Ok(Outcome::Synthesized(_)) => survivors += 1,
            Err(_) => panic!("panic for {}", what()),
        }
    };
    for len in 0..good.len() {
        check(&good[..len], &|| format!("truncation to {len} bytes"));
    }
    let mut rng = XorShift(0x6e61_6769_2d74_7473);
    const SYMBOLS: &[u8] = b"0123456789-{}[]\":,\n _*QSx\xff";
    for trial in 0..1_500 {
        let mut bytes = good.clone();
        for _ in 0..1 + rng.below(3) {
            let at = rng.below(bytes.len());
            bytes[at] = if rng.below(2) == 0 {
                SYMBOLS[rng.below(SYMBOLS.len())]
            } else {
                rng.next() as u8
            };
        }
        check(&bytes, &|| format!("mutation trial {trial}"));
    }
    // Most mutations hit PDF values and must still synthesize.
    assert!(survivors > 0);
}

/// What happens to `bytes` when the provider's validator is *bypassed*:
/// jbonsai parses them and synthesizes the fixed labels. Panics are caught
/// and described. Used to document the pre-validation behaviour of each
/// regression in its failure message.
fn unvalidated(bytes: &[u8]) -> String {
    let engine = match Engine::load_from_bytes([bytes]) {
        Ok(engine) => engine,
        Err(error) => return format!("jbonsai parse error, no panic ({error})"),
    };
    let labels: Vec<jlabel::Label> = LABELS.iter().map(|l| l.parse().unwrap()).collect();
    let run = catch_unwind(AssertUnwindSafe(|| {
        let mut generator = engine.generator(labels).map_err(|e| e.to_string())?;
        let mut frame = vec![0.0f64; generator.fperiod()];
        let (mut samples, mut non_finite) = (0usize, 0usize);
        while generator.generate_step(&mut frame) != 0 {
            samples += frame.len();
            non_finite += frame.iter().filter(|s| !s.is_finite()).count();
            if samples > 48_000 * 60 {
                return Err("still synthesizing after 60 s".to_owned());
            }
        }
        Ok((samples, non_finite))
    }));
    match run {
        Ok(Ok((samples, non_finite))) => {
            format!("synthesized {samples} samples ({non_finite} non-finite)")
        }
        Ok(Err(error)) => format!("engine error, no panic ({error})"),
        Err(payload) => {
            let message = payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default();
            format!("PANIC: {message}")
        }
    }
}

/// Checks every case is rejected by both the validator and the loader, and
/// reports all misses at once with what jbonsai does without the check.
fn assert_all_invalid(cases: &[(Voice, &str)]) {
    let mut misses = Vec::new();
    for (voice, case) in cases {
        let bytes = voice.bytes();
        let validator = check_voice_structure(&bytes);
        let loader = JbonsaiBackend::from_bytes(&bytes, DictionaryBytes::default()).err();
        if validator != Err(LoadError::VoiceInvalid) || loader != Some(LoadError::VoiceInvalid) {
            misses.push(format!(
                "{case}: validator {validator:?}, loader {loader:?}; unvalidated: {}",
                unvalidated(&bytes)
            ));
        }
    }
    assert!(
        misses.is_empty(),
        "accepted malformed voices:\n{}",
        misses.join("\n")
    );
}

fn assert_synthesizes(voice: &Voice, case: &str) {
    let bytes = voice.bytes();
    assert_eq!(check_voice_structure(&bytes), Ok(()), "{case}: validator");
    match outcome(&bytes) {
        Outcome::Synthesized(Ok(samples)) if samples > 0 => {}
        other => panic!("{case}: must synthesize, got {other:?}"),
    }
}

/// Review finding 1: the duration PDF values were not range-checked. jbonsai
/// 0.4.2 computes each state duration as `mean.round().max(1.0) as usize`,
/// so `f32::MAX` or `+inf` becomes `usize::MAX` frames, and the provider's
/// planner (and jbonsai's own generator) then sums those durations.
#[test]
fn duration_pdf_values_must_be_finite_and_bounded() {
    // NUM_STATES is 1: value 0 is the state mean, value 1 its variance.
    let with = |at: usize, value: f32| {
        let mut voice = Voice::valid();
        voice.duration_value = Some((at, value));
        voice
    };
    assert_all_invalid(&[
        (with(0, f32::MAX), "duration mean f32::MAX"),
        (with(0, f32::INFINITY), "duration mean +inf"),
        (with(0, f32::NAN), "duration mean NaN"),
        (with(0, -5.0), "duration mean negative"),
        (with(0, f32::NEG_INFINITY), "duration mean -inf"),
        (
            with(0, MAX_DURATION_MEAN_FRAMES * 2.0),
            "duration mean above the bound",
        ),
        (with(1, f32::NAN), "duration variance NaN"),
        (with(1, f32::INFINITY), "duration variance +inf"),
        (with(1, 0.0), "duration variance zero"),
        (with(1, -1.0), "duration variance negative"),
    ]);
    // The bounds themselves stay usable.
    for (at, value) in [(0, 0.0), (0, 1.0), (0, 400.0), (1, MAX_DURATION_VARIANCE)] {
        assert_synthesizes(&with(at, value), &format!("duration value {at}={value}"));
    }
}

/// Stream and GV PDFs must be finite, with non-negative variances and MSD
/// weights in [0, 1]. Zero variances stay accepted: the pinned LPF stream
/// has them.
#[test]
fn stream_pdf_values_must_be_finite() {
    let with = |stream: usize, at: usize, value: f32| {
        let mut voice = Voice::valid();
        voice.stream_value = Some((stream, at, value));
        voice
    };
    assert_all_invalid(&[
        (with(0, 0, f32::NAN), "spectrum mean NaN"),
        (with(0, 0, f32::INFINITY), "spectrum mean +inf"),
        (with(0, 3, -1.0), "spectrum variance negative"),
        (with(1, 0, f32::NEG_INFINITY), "log F0 mean -inf"),
        (with(1, 2, 1.5), "MSD weight above 1"),
        (with(1, 2, -0.5), "MSD weight negative"),
        (with(1, 2, f32::NAN), "MSD weight NaN"),
        (with(2, 1, f32::NAN), "LPF variance NaN"),
    ]);
    assert_synthesizes(&with(2, 1, 0.0), "LPF zero variance");
    assert_synthesizes(&with(1, 2, 0.0), "MSD weight 0 (unvoiced)");
    assert_synthesizes(&with(0, 0, -1.0e6), "large finite spectrum mean");
}

/// Review finding 2a: jbonsai's SpeechGenerator::new asserts that the log F0
/// stream (index 1) has a one-element static vector and the LPF stream
/// (index 2) an odd one; both were accepted by the validator.
#[test]
fn vocoder_stream_shapes_are_role_checked() {
    let shaped = |stream: usize, from: &str, to: &str, pdf_len: usize| {
        let mut voice = Voice::valid();
        assert!(voice.stream.contains(from), "{from}");
        voice.stream = voice.stream.replacen(from, to, 1);
        voice.streams[stream].pdf_len = pdf_len;
        voice
    };
    assert_all_invalid(&[
        (
            shaped(1, "VECTOR_LENGTH[LF0]:1", "VECTOR_LENGTH[LF0]:2", 2 * 2 + 1),
            "log F0 vector length 2",
        ),
        (
            shaped(2, "VECTOR_LENGTH[LPF]:1", "VECTOR_LENGTH[LPF]:2", 2 * 2),
            "LPF vector length 2 (even)",
        ),
    ]);
    assert_synthesizes(
        &shaped(2, "VECTOR_LENGTH[LPF]:1", "VECTOR_LENGTH[LPF]:3", 3 * 2),
        "LPF vector length 3",
    );
    assert_synthesizes(
        &shaped(0, "VECTOR_LENGTH[MCP]:3", "VECTOR_LENGTH[MCP]:4", 4 * 2),
        "even spectrum vector length",
    );
}

/// Review finding 2b: the number of STREAM_WIN rows was not compared with
/// NUM_WINDOWS. MlpgAdjust::create reads the PDF at
/// `vector_length * window + i` for every listed row, so an extra (for
/// example duplicated) row indexes past the PDF.
#[test]
fn window_rows_must_match_num_windows() {
    let repeated = |stream: usize, times: usize| {
        let mut voice = Voice::valid();
        voice.window_repeat[stream] = times;
        voice
    };
    let row = |stream: usize, text: &'static str| {
        let mut voice = Voice::valid();
        voice.windows[stream] = text;
        voice
    };
    let mut fewer = Voice::valid();
    fewer.stream = fewer
        .stream
        .replacen("NUM_WINDOWS[MCP]:1", "NUM_WINDOWS[MCP]:2", 1);
    fewer.streams[0].pdf_len = 3 * 2 * 2;
    assert_all_invalid(&[
        (repeated(0, 2), "spectrum window row duplicated"),
        (repeated(2, 2), "LPF window row duplicated"),
        (repeated(1, 3), "log F0 window row listed three times"),
        (fewer, "fewer window rows than NUM_WINDOWS"),
        (row(0, "2 0.5 0.5\n"), "even window width"),
        (row(0, "1 inf\n"), "non-finite window coefficient"),
        (row(0, "3 1.0\n"), "fewer coefficients than declared"),
        (row(0, "1 1.0 2.0\n"), "more coefficients than declared"),
        (row(0, "17 1.0\n"), "window wider than the bound"),
        (row(0, "x 1.0\n"), "window without a count"),
    ]);
    // Rows may repeat as long as their count matches NUM_WINDOWS.
    let mut matching = repeated(0, 3);
    matching.stream = matching
        .stream
        .replacen("NUM_WINDOWS[MCP]:1", "NUM_WINDOWS[MCP]:3", 1);
    matching.streams[0].pdf_len = 3 * 3 * 2;
    assert_synthesizes(&matching, "three rows for three windows");
}

// ---------------------------------------------------------------------------
// Header numbers that size upstream allocations (run in a limited child)
// ---------------------------------------------------------------------------

/// Header edits whose upstream consequence can be an allocation sized by the
/// header itself. Each is checked in a child process under `ulimit -v`
/// (address space) and `ulimit -t` (CPU seconds), so a regression that let
/// one through to jbonsai cannot exhaust the host's memory or hang the run.
const HEADER_CASES: &[(&str, &str, &str, &str)] = &[
    // (case, field, from, to)
    ("num-streams-0", "global", "NUM_STREAMS:3", "NUM_STREAMS:0"),
    ("num-streams-2", "global", "NUM_STREAMS:3", "NUM_STREAMS:2"),
    ("num-streams-4", "global", "NUM_STREAMS:3", "NUM_STREAMS:4"),
    (
        "num-streams-huge",
        "global",
        "NUM_STREAMS:3",
        "NUM_STREAMS:999999999999999999",
    ),
    ("num-streams-missing", "global", "NUM_STREAMS:3\n", ""),
    (
        "gamma-huge",
        "stream",
        "OPTION[MCP]:ALPHA=0.55",
        "OPTION[MCP]:ALPHA=0.55,GAMMA=999999999999999999",
    ),
    (
        "gamma-1e14",
        "stream",
        "OPTION[MCP]:ALPHA=0.55",
        "OPTION[MCP]:ALPHA=0.55,GAMMA=100000000000000",
    ),
    (
        "gamma-1",
        "stream",
        "OPTION[MCP]:ALPHA=0.55",
        "OPTION[MCP]:ALPHA=0.55,GAMMA=1",
    ),
    (
        "gamma-negative",
        "stream",
        "OPTION[MCP]:ALPHA=0.55",
        "OPTION[MCP]:ALPHA=0.55,GAMMA=-1",
    ),
    (
        "ln-gain-2",
        "stream",
        "OPTION[MCP]:ALPHA=0.55",
        "OPTION[MCP]:ALPHA=0.55,LN_GAIN=2",
    ),
    ("alpha-inf", "stream", "ALPHA=0.55", "ALPHA=inf"),
    ("alpha-nan", "stream", "ALPHA=0.55", "ALPHA=NaN"),
    ("alpha-1", "stream", "ALPHA=0.55", "ALPHA=1.0"),
    ("alpha-negative", "stream", "ALPHA=0.55", "ALPHA=-0.5"),
    ("alpha-text", "stream", "ALPHA=0.55", "ALPHA=x"),
    (
        "alpha-duplicate",
        "stream",
        "ALPHA=0.55",
        "ALPHA=0.55,ALPHA=0.42",
    ),
];

/// In-range edits of the same fields that must still synthesize.
const HEADER_CONTROLS: &[(&str, &str, &str, &str)] = &[
    (
        "gamma-0",
        "stream",
        "OPTION[MCP]:ALPHA=0.55",
        "OPTION[MCP]:ALPHA=0.55,GAMMA=0",
    ),
    (
        "ln-gain-1",
        "stream",
        "OPTION[MCP]:ALPHA=0.55",
        "OPTION[MCP]:ALPHA=0.55,LN_GAIN=1",
    ),
    ("alpha-0.42", "stream", "ALPHA=0.55", "ALPHA=0.42"),
    ("alpha-0", "stream", "ALPHA=0.55", "ALPHA=0"),
    (
        "no-options",
        "stream",
        "OPTION[MCP]:ALPHA=0.55",
        "OPTION[MCP]:",
    ),
];

const CHILD_CASE: &str = "NAGI_TTS_CHILD_CASE";
const CHILD_MODE: &str = "NAGI_TTS_CHILD_MODE";
/// Address-space limit for the child, in KiB (the debug test binary and a
/// synthesis of the synthetic voice need well under 1 GiB).
const CHILD_MEMORY_KIB: u64 = 2 * 1024 * 1024;
const CHILD_CPU_SECONDS: u64 = 60;

fn header_voice(case: &str) -> Voice {
    let (_, field, from, to) = HEADER_CASES
        .iter()
        .chain(HEADER_CONTROLS)
        .find(|(name, ..)| *name == case)
        .unwrap_or_else(|| panic!("unknown case {case}"));
    let mut voice = Voice::valid();
    let target = if *field == "global" {
        &mut voice.global
    } else {
        &mut voice.stream
    };
    assert!(target.contains(from), "{case}: {from}");
    *target = target.replacen(from, to, 1);
    voice
}

/// Child entry point. Without the environment variables it does nothing
/// (and passes), so it never counts as ignored in the model-free run.
///
/// - mode `validate`: prints `CHILD-RESULT: <validator> <loader>`;
///   only the provider's own checks run before the loader returns.
/// - mode `bypass`: hands the bytes straight to jbonsai (the pre-fix path)
///   and prints what happened; used to record upstream behaviour.
#[test]
fn header_case_child() {
    let (Ok(case), Ok(mode)) = (std::env::var(CHILD_CASE), std::env::var(CHILD_MODE)) else {
        return;
    };
    let bytes = header_voice(&case).bytes();
    match mode.as_str() {
        "validate" => {
            let validator = check_voice_structure(&bytes);
            let loader = JbonsaiBackend::from_bytes(&bytes, DictionaryBytes::default()).err();
            println!("CHILD-RESULT: {validator:?} {loader:?}");
        }
        "bypass" => println!("CHILD-RESULT: {}", unvalidated(&bytes)),
        "synthesize" => {
            assert_eq!(check_voice_structure(&bytes), Ok(()), "{case}");
            println!("CHILD-RESULT: {:?}", outcome(&bytes));
        }
        other => panic!("unknown mode {other}"),
    }
}

/// Runs [`header_case_child`] for `case` in a child process limited by
/// `ulimit -v` / `ulimit -t`. Returns the child's `CHILD-RESULT` line, or a
/// description of how it ended without one (panic, abort, signal).
fn run_limited_child(case: &str, mode: &str) -> String {
    let exe = std::env::current_exe().expect("test binary path");
    let output = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(format!(
            "ulimit -v {CHILD_MEMORY_KIB} && ulimit -t {CHILD_CPU_SECONDS} && exec \"$0\" \"$@\""
        ))
        .arg(exe)
        .args([
            "--exact",
            "validate::tests::header_case_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_CASE, case)
        .env(CHILD_MODE, mode)
        .output()
        .expect("spawn limited child");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // libtest prints the line after "test <name> ... " with --nocapture.
    if let Some(line) = stdout
        .lines()
        .find_map(|l| l.split_once("CHILD-RESULT: ").map(|(_, rest)| rest))
    {
        return line.to_owned();
    }
    let reason = stderr
        .lines()
        .chain(stdout.lines())
        .find(|l| {
            l.contains("panicked at") || l.contains("memory allocation") || l.contains("capacity")
        })
        .unwrap_or("");
    let next = stderr
        .lines()
        .chain(stdout.lines())
        .skip_while(|l| !l.contains("panicked at"))
        .nth(1)
        .unwrap_or("");
    format!("NO RESULT (status {:?}) {reason} {next}", output.status)
}

/// Review findings 3 and 4: `NUM_STREAMS` was not compared with
/// `STREAM_TYPE` (jbonsai sizes its per-stream condition arrays with it and
/// indexes them 0..=2 during synthesis), and the spectrum stream's `GAMMA`,
/// `LN_GAIN` and `ALPHA` options were not checked (`GAMMA` is jbonsai's
/// MGLSA stage count, which sizes `vec![vec![0.0; nmcp]; stage]`).
#[cfg(unix)]
#[test]
fn stream_count_and_spectrum_options_are_checked_in_a_limited_child() {
    let mut misses = Vec::new();
    for (case, ..) in HEADER_CASES {
        let result = run_limited_child(case, "validate");
        if result != "Err(VoiceInvalid) Some(VoiceInvalid)" {
            misses.push(format!("{case}: {result}"));
        }
    }
    for (case, ..) in HEADER_CONTROLS {
        let result = run_limited_child(case, "synthesize");
        if !result.starts_with("Synthesized(Ok(") {
            misses.push(format!("control {case}: {result}"));
        }
    }
    assert!(misses.is_empty(), "{}", misses.join("\n"));
}

/// Records what jbonsai does with each header case when the provider's
/// checks are bypassed. Run explicitly; prints, never asserts:
/// `NAGI_TTS_RECORD_BYPASS=1 cargo test header_cases_bypass_record -- --nocapture`
#[cfg(unix)]
#[test]
fn header_cases_bypass_record() {
    if std::env::var_os("NAGI_TTS_RECORD_BYPASS").is_none() {
        return;
    }
    for (case, ..) in HEADER_CASES {
        println!("BYPASS {case}: {}", run_limited_child(case, "bypass"));
    }
}

// ---------------------------------------------------------------------------
// Dictionary components
// ---------------------------------------------------------------------------

fn matrix(forward: i16, backward: i16, cells: usize) -> Vec<u8> {
    let mut bytes = Vec::new();
    for value in [-1i16, forward, backward] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.resize(6 + cells * 2, 0);
    bytes
}

fn entry(word_id: u32, left: u16, right: u16) -> Vec<u8> {
    let mut bytes = word_id.to_le_bytes().to_vec();
    bytes.extend_from_slice(&0i16.to_le_bytes());
    bytes.extend_from_slice(&left.to_le_bytes());
    bytes.extend_from_slice(&right.to_le_bytes());
    bytes
}

fn words(preamble: &[u8], records: &[&[u8]]) -> (Vec<u8>, Vec<u8>) {
    let mut data = preamble.to_vec();
    let mut index = vec![(data.len() as u32).to_le_bytes()];
    for record in records {
        data.extend_from_slice(record);
        index.push((data.len() as u32).to_le_bytes());
    }
    index.pop();
    (index.concat(), data)
}

#[test]
fn connection_matrix_must_match_its_declared_shape() {
    assert!(check_matrix(&matrix(3, 2, 6)).is_ok());
    // Old (untransposed) layout.
    let mut old = vec![];
    for value in [3i16, 2] {
        old.extend_from_slice(&value.to_le_bytes());
    }
    old.resize(4 + 12, 0);
    assert!(check_matrix(&old).is_ok());

    let invalid = Err(LoadError::DictionaryInvalid);
    let full = matrix(3, 2, 6);
    for (bytes, case) in [
        (
            full[..full.len() - 2].to_vec(),
            "truncated (cost lookup past the end)",
        ),
        (
            full[..full.len() - 1].to_vec(),
            "odd length (loader length assertion)",
        ),
        ([full.clone(), vec![0, 0]].concat(), "trailing cells"),
        (
            matrix(-2, 2, 6),
            "negative forward size (multiply overflow)",
        ),
        (matrix(3, 0, 0), "zero backward size"),
        (matrix(i16::MAX, i16::MAX, 6), "huge declared shape"),
        (vec![0xFF], "one byte"),
        (vec![0xFF, 0xFF, 0x03], "header cut"),
        (Vec::new(), "empty"),
    ] {
        assert_eq!(check_matrix(&bytes).map(|_| ()), invalid, "{case}");
    }
}

#[test]
fn word_entries_must_be_whole_and_inside_the_matrix() {
    let shape = check_matrix(&matrix(3, 2, 6)).unwrap();
    let vals = [entry(0, 1, 2), entry(1, 0, 0)].concat();
    assert_eq!(check_word_entries(&vals, shape), Ok(()));
    let invalid = Err(LoadError::DictionaryInvalid);
    // right_id indexes the forward axis, left_id the backward axis.
    assert_eq!(check_word_entries(&entry(0, 2, 0), shape), invalid);
    assert_eq!(check_word_entries(&entry(0, 0, 3), shape), invalid);
    assert_eq!(
        check_word_entries(&entry(0, u16::MAX, u16::MAX), shape),
        invalid
    );
    assert_eq!(check_word_entries(&vals[..vals.len() - 1], shape), invalid);
}

#[test]
fn word_index_must_be_monotonic_in_bounds_and_jpreprocess() {
    let (index, data) = words(b"jpreprocess 0.15", &[b"abc", b"de", b"", b"f"]);
    assert_eq!(check_words(&index, &data), Ok(()));
    let invalid = Err(LoadError::DictionaryInvalid);

    // Offsets out of order: jpreprocess slices `start..end` with start > end.
    let mut swapped = index.clone();
    swapped[4..8].copy_from_slice(&index[8..12]);
    swapped[8..12].copy_from_slice(&index[4..8]);
    assert_eq!(check_words(&swapped, &data), invalid);
    // Offset past the record data.
    let mut past = index.clone();
    past[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(check_words(&past, &data), invalid);
    // Partial offset, empty index.
    assert_eq!(check_words(&index[..index.len() - 1], &data), invalid);
    assert_eq!(check_words(&[], &data), invalid);
    // Truncated record data.
    assert_eq!(check_words(&index, &data[..data.len() - 3]), invalid);
    // A non-jpreprocess preamble would route tokens to lindera's own
    // decoder, which slices jpreprocess records without bounds checks.
    for preamble in [&b""[..], b"lindera", b"jpre\xffprocess"] {
        let (index, data) = words(preamble, &[b"abc"]);
        assert_eq!(check_words(&index, &data), invalid, "{preamble:?}");
    }
    let (index, data) = words(b"JPreprocess", &[b"abc"]);
    assert_eq!(check_words(&index, &data), Ok(()));
}

#[test]
fn dictionary_bytes_are_checked_before_any_loader() {
    let (wordsidx, dict_words) = words(b"jpreprocess", &[b"abc"]);
    let base = || DictionaryBytes {
        metadata_json: b"{}".to_vec(),
        char_def_bin: vec![0; 16],
        matrix_mtx: matrix(2, 2, 4),
        dict_da: vec![0; 16],
        dict_vals: entry(0, 1, 1),
        dict_wordsidx: wordsidx.clone(),
        dict_words: dict_words.clone(),
        unk_bin: vec![0; 16],
    };
    assert!(check_dictionary_bytes(&base()).is_ok());
    let mut cases = Vec::new();
    let mut bad = base();
    bad.matrix_mtx.pop();
    cases.push(bad);
    let mut bad = base();
    bad.dict_vals = entry(0, 2, 0);
    cases.push(bad);
    let mut bad = base();
    bad.dict_wordsidx = u32::MAX.to_le_bytes().to_vec();
    cases.push(bad);
    for bad in cases {
        assert_eq!(
            check_dictionary_bytes(&bad).map(|_| ()),
            Err(LoadError::DictionaryInvalid)
        );
        // The loader reports the same error without touching the parsers.
        assert_eq!(
            JbonsaiBackend::from_bytes(&Voice::valid().bytes(), bad).err(),
            Some(LoadError::DictionaryInvalid)
        );
    }
}
