//! Hostile-but-well-framed edits of the pinned voice, on the host.
//!
//! Each case keeps the `.htsvoice` structurally consistent (same PDF sizes,
//! same tree references) and changes only values the upstream engine trusts:
//! duration PDF means/variances (review finding 1) and `STREAM_WIN` rows
//! (review finding 2b). Every case must be rejected at load time as
//! [`LoadError::VoiceInvalid`]; none may panic. In-range edits of the same
//! values must still load and speak, so the checks do not over-reject.
//!
//! Like `tests/real_engine.rs` these tests are `#[ignore]`d and need
//! `NAGI_TTS_VOICE` / `NAGI_TTS_DICT` (from `tools/tts/fetch.sh`); a missing
//! artifact is a FAIL, not a skip:
//!
//! ```text
//! NAGI_TTS_VOICE=.../tohoku-f01-neutral.htsvoice NAGI_TTS_DICT=.../naist-jdic \
//!   cargo test --manifest-path crates/nagi-tts-provider/Cargo.toml \
//!   --test real_hostile_model -- --ignored --test-threads=1
//! ```
//!
//! Host only; not guest (Nagi/QEMU) evidence.
#![cfg(feature = "engine-jbonsai")]

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;

use nagi_audio::speech::{
    SpeechProviderError, SpeechSynthesisLanguage, SpeechSynthesisOptions, SynthesisPcmChunk,
    TextToSpeechProvider,
};
use nagi_audio::PcmFormat;
use nagi_tts_provider::jbonsai_backend::{DictionaryBytes, JbonsaiBackend, LoadError};
use nagi_tts_provider::LocalTtsProvider;

const PHRASE: &str = "こんにちは、ナギです。";

fn artifacts() -> (PathBuf, PathBuf) {
    let voice = std::env::var_os("NAGI_TTS_VOICE")
        .expect("FAIL: NAGI_TTS_VOICE is not set (run tools/tts/fetch.sh)");
    let dict = std::env::var_os("NAGI_TTS_DICT")
        .expect("FAIL: NAGI_TTS_DICT is not set (run tools/tts/fetch.sh)");
    let (voice, dict) = (PathBuf::from(voice), PathBuf::from(dict));
    assert!(
        voice.is_file(),
        "FAIL: voice missing at {}",
        voice.display()
    );
    assert!(
        dict.is_dir(),
        "FAIL: dictionary missing at {}",
        dict.display()
    );
    (voice, dict)
}

/// What loading `voice` and speaking [`PHRASE`] did. Panics are caught.
#[derive(Debug, PartialEq)]
enum Outcome {
    Rejected(LoadError),
    Spoke(Result<usize, SpeechProviderError>),
    Panicked(String),
}

fn exercise(voice: &[u8], dictionary: DictionaryBytes) -> Outcome {
    let run = catch_unwind(AssertUnwindSafe(|| {
        let backend = match JbonsaiBackend::from_bytes(voice, dictionary) {
            Ok(backend) => backend,
            Err(error) => return Outcome::Rejected(error),
        };
        let mut provider = LocalTtsProvider::new(backend).expect("frame shape");
        let options = SpeechSynthesisOptions {
            language: SpeechSynthesisLanguage::Japanese,
            pcm_format: PcmFormat::stereo_48khz(),
        };
        let spoken = provider.begin(PHRASE, options).and_then(|()| {
            let mut buffer = vec![0u8; 4096];
            let mut total = 0usize;
            loop {
                match provider.next_pcm_chunk(&mut buffer)? {
                    SynthesisPcmChunk::Data(n) => total += n,
                    SynthesisPcmChunk::End => return Ok(total),
                }
            }
        });
        assert!(provider.is_clear(), "state retained");
        Outcome::Spoke(spoken)
    }));
    run.unwrap_or_else(|payload| {
        Outcome::Panicked(
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .unwrap_or_default(),
        )
    })
}

/// `[POSITION]` ranges of the voice, relative to the `[DATA]` section.
struct Layout {
    data: usize,
    header: String,
}

impl Layout {
    fn of(voice: &[u8]) -> Self {
        let data = voice.windows(8).position(|w| w == b"\n[DATA]\n").unwrap() + 8;
        let header = String::from_utf8(voice[..data].to_vec()).unwrap();
        Self { data, header }
    }

    fn value(&self, key: &str) -> &str {
        let line = self
            .header
            .lines()
            .find(|line| line.starts_with(&format!("{key}:")))
            .unwrap_or_else(|| panic!("{key}"));
        &line[key.len() + 1..]
    }

    fn range(&self, key: &str) -> (usize, usize) {
        let (start, end) = self.value(key).split_once('-').unwrap();
        (start.parse().unwrap(), end.parse().unwrap())
    }
}

/// Writes `value` at float `at` of every duration PDF (`at < NUM_STATES` is
/// a state mean, `NUM_STATES + s` that state's variance).
fn duration_edit(voice: &[u8], at: usize, value: f32) -> Vec<u8> {
    let layout = Layout::of(voice);
    let states: usize = layout.value("NUM_STATES").parse().unwrap();
    let (tree_start, tree_end) = layout.range("DURATION_TREE");
    let trees = String::from_utf8_lossy(&voice[layout.data + tree_start..=layout.data + tree_end])
        .matches("{*}")
        .count();
    let (pdf_start, pdf_end) = layout.range("DURATION_PDF");
    let pdf = layout.data + pdf_start;
    let values = pdf + trees * 4;
    let pdf_bytes = 2 * states * 4;
    assert_eq!((layout.data + pdf_end + 1 - values) % pdf_bytes, 0);
    let mut out = voice.to_vec();
    let mut edited = 0usize;
    let mut offset = values;
    while offset < layout.data + pdf_end + 1 {
        let slot = offset + at * 4;
        out[slot..slot + 4].copy_from_slice(&value.to_le_bytes());
        offset += pdf_bytes;
        edited += 1;
    }
    assert!(edited > 300, "every duration PDF edited ({edited})");
    out
}

/// Lists the first `STREAM_WIN[stream]` range `extra` more times.
fn duplicate_window(voice: &[u8], stream: &str, extra: usize) -> Vec<u8> {
    let key = format!("STREAM_WIN[{stream}]:");
    let text = String::from_utf8_lossy(voice);
    let start = text.find(&key).unwrap() + key.len();
    let end = start + text[start..].find('\n').unwrap();
    let first = text[start..end].split(',').next().unwrap().to_owned();
    let listed = format!("{}{}", &text[start..end], format!(",{first}").repeat(extra));
    [&voice[..start], listed.as_bytes(), &voice[end..]].concat()
}

#[test]
#[ignore = "real-artifact hostile-model checks: needs NAGI_TTS_VOICE/NAGI_TTS_DICT"]
fn hostile_duration_pdfs_and_window_rows_fail_closed() {
    let (voice_path, dict_path) = artifacts();
    let voice = std::fs::read(&voice_path).unwrap();
    let dictionary = DictionaryBytes::read_dir(&dict_path).unwrap();
    let parts = [
        dictionary.metadata_json,
        dictionary.char_def_bin,
        dictionary.matrix_mtx,
        dictionary.dict_da,
        dictionary.dict_vals,
        dictionary.dict_wordsidx,
        dictionary.dict_words,
        dictionary.unk_bin,
    ];
    let dict = || {
        let [metadata_json, char_def_bin, matrix_mtx, dict_da, dict_vals, dict_wordsidx, dict_words, unk_bin] =
            parts.clone();
        DictionaryBytes {
            metadata_json,
            char_def_bin,
            matrix_mtx,
            dict_da,
            dict_vals,
            dict_wordsidx,
            dict_words,
            unk_bin,
        }
    };

    let reference = exercise(&voice, dict());
    assert!(
        matches!(reference, Outcome::Spoke(Ok(n)) if n > 0),
        "pinned voice must speak: {reference:?}"
    );

    // The pinned voice has 5 states: floats 0..5 are means, 5..10 variances;
    // float 2 is the third state's mean, float 7 its variance, in every PDF.
    let hostile = [
        (
            "state-3 duration mean f32::MAX",
            duration_edit(&voice, 2, f32::MAX),
        ),
        (
            "state-3 duration mean +inf",
            duration_edit(&voice, 2, f32::INFINITY),
        ),
        (
            "state-3 duration mean NaN",
            duration_edit(&voice, 2, f32::NAN),
        ),
        (
            "state-3 duration mean negative",
            duration_edit(&voice, 2, -5.0),
        ),
        (
            "state-3 duration mean 1e9 frames",
            duration_edit(&voice, 2, 1.0e9),
        ),
        (
            "state-3 duration variance NaN",
            duration_edit(&voice, 7, f32::NAN),
        ),
        (
            "state-3 duration variance +inf",
            duration_edit(&voice, 7, f32::INFINITY),
        ),
        (
            "LPF window row duplicated",
            duplicate_window(&voice, "LPF", 1),
        ),
        (
            "fourth spectrum window row",
            duplicate_window(&voice, "MCP", 1),
        ),
        (
            "fourth log F0 window row",
            duplicate_window(&voice, "LF0", 1),
        ),
    ];
    let mut misses = Vec::new();
    for (case, bytes) in &hostile {
        let outcome = exercise(bytes, dict());
        eprintln!("{case}: {outcome:?}");
        if outcome != Outcome::Rejected(LoadError::VoiceInvalid) {
            misses.push(format!("{case}: {outcome:?}"));
        }
    }
    assert!(
        misses.is_empty(),
        "hostile voices not rejected:\n{}",
        misses.join("\n")
    );

    // In-range edits of the same values still load and speak.
    for (case, bytes) in [
        (
            "state-3 duration mean 8 frames",
            duration_edit(&voice, 2, 8.0),
        ),
        (
            "state-3 duration variance 10",
            duration_edit(&voice, 7, 10.0),
        ),
    ] {
        let outcome = exercise(&bytes, dict());
        assert!(
            matches!(outcome, Outcome::Spoke(Ok(n)) if n > 0),
            "{case}: {outcome:?}"
        );
    }
}
