//! Adversarial loading and lifecycle checks against the pinned voice and
//! dictionary, on the host.
//!
//! Corrupted copies of the real artifacts (as a partial or tampered Model
//! Store read would deliver them) must be rejected or must synthesize; they
//! must never panic. Lifecycle after load and synthesis errors must leave no
//! retained state: reuse is byte-identical to a fresh provider.
//!
//! Like `tests/real_engine.rs` these tests are `#[ignore]`d and need
//! `NAGI_TTS_VOICE` / `NAGI_TTS_DICT` (from `tools/tts/fetch.sh`); a missing
//! artifact is a FAIL, not a skip:
//!
//! ```text
//! NAGI_TTS_VOICE=.../tohoku-f01-neutral.htsvoice NAGI_TTS_DICT=.../naist-jdic \
//!   cargo test --manifest-path crates/nagi-tts-provider/Cargo.toml \
//!   --test real_adversarial -- --ignored --test-threads=1
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
use nagi_tts_provider::jbonsai_backend::{
    load_provider, DictionaryBytes, JbonsaiBackend, JbonsaiProvider, LoadError,
};
use nagi_tts_provider::LocalTtsProvider;

const PHRASE: &str = "こんにちは、ナギです。";
const SECOND: &str = "東京都日野市で、ランニングをしました！";

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

fn options(format: PcmFormat) -> SpeechSynthesisOptions {
    SpeechSynthesisOptions {
        language: SpeechSynthesisLanguage::Japanese,
        pcm_format: format,
    }
}

fn drain(
    provider: &mut impl TextToSpeechProvider,
    chunk: usize,
) -> Result<Vec<u8>, SpeechProviderError> {
    let mut buffer = vec![0u8; chunk];
    let mut out = Vec::new();
    loop {
        match provider.next_pcm_chunk(&mut buffer)? {
            SynthesisPcmChunk::Data(n) => out.extend_from_slice(&buffer[..n]),
            SynthesisPcmChunk::End => return Ok(out),
        }
    }
}

fn speak(
    provider: &mut JbonsaiProvider,
    text: &str,
    format: PcmFormat,
) -> Result<Vec<u8>, SpeechProviderError> {
    provider.begin(text, options(format))?;
    drain(provider, 4096)
}

/// The eight dictionary components, read once and copied per trial.
struct Dict([Vec<u8>; 8]);

impl Dict {
    fn read(path: &std::path::Path) -> Self {
        let d = DictionaryBytes::read_dir(path).expect("pinned dictionary reads");
        Self([
            d.metadata_json,
            d.char_def_bin,
            d.matrix_mtx,
            d.dict_da,
            d.dict_vals,
            d.dict_wordsidx,
            d.dict_words,
            d.unk_bin,
        ])
    }

    fn with(&self, component: usize, bytes: Vec<u8>) -> DictionaryBytes {
        let mut parts = self.0.clone();
        parts[component] = bytes;
        let [metadata_json, char_def_bin, matrix_mtx, dict_da, dict_vals, dict_wordsidx, dict_words, unk_bin] =
            parts;
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
    }

    fn copy(&self) -> DictionaryBytes {
        self.with(0, self.0[0].clone())
    }
}

/// Loads and, if loading succeeds, speaks two sentences. Returns the load
/// error, or the per-sentence results. Panics are reported with `what`.
fn exercise(
    voice: &[u8],
    dictionary: DictionaryBytes,
    what: &str,
) -> Result<[Result<usize, SpeechProviderError>; 2], LoadError> {
    catch_unwind(AssertUnwindSafe(|| {
        let backend = JbonsaiBackend::from_bytes(voice, dictionary)?;
        let mut provider = LocalTtsProvider::new(backend).expect("frame shape");
        let mut run = |text: &str| -> Result<usize, SpeechProviderError> {
            let result = speak(&mut provider, text, PcmFormat::stereo_48khz()).map(|pcm| pcm.len());
            assert!(provider.is_clear(), "state retained after {text}");
            result
        };
        Ok([run(PHRASE), run(SECOND)])
    }))
    .unwrap_or_else(|_| panic!("panic for {what}"))
}

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

fn replace_after(bytes: &[u8], anchor: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len());
    let start = bytes
        .windows(anchor.len())
        .position(|w| w == anchor)
        .expect("anchor");
    let at = start
        + bytes[start..]
            .windows(from.len())
            .position(|w| w == from)
            .expect("pattern");
    let mut out = bytes.to_vec();
    out[at..at + from.len()].copy_from_slice(to);
    out
}

fn replace_text(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
    let at = bytes
        .windows(from.len())
        .position(|w| w == from.as_bytes())
        .expect("pattern");
    [&bytes[..at], to.as_bytes(), &bytes[at + from.len()..]].concat()
}

#[test]
#[ignore = "real-artifact adversarial checks: needs NAGI_TTS_VOICE/NAGI_TTS_DICT"]
fn corrupted_voice_bytes_fail_closed_without_panic() {
    let (voice_path, dict_path) = artifacts();
    let voice = std::fs::read(&voice_path).unwrap();
    let dict = Dict::read(&dict_path);

    let reference = exercise(&voice, dict.copy(), "pinned voice").expect("pinned voice loads");
    assert!(reference.iter().all(|r| matches!(r, Ok(n) if *n > 0)));

    let data = voice.windows(7).position(|w| w == b"[DATA]\n").unwrap() + 7;
    // The first LPF tree claims state 9, so state 2 has no LPF tree.
    let lpf = voice
        .windows(b"\"lpf_s2_1\"".len())
        .position(|w| w == b"\"lpf_s2_1\"")
        .unwrap();
    let header = voice[..lpf]
        .windows(6)
        .rposition(|w| w == b"{*}[2]")
        .unwrap();
    let mut state9 = voice.clone();
    state9[header..header + 6].copy_from_slice(b"{*}[9]");
    let targeted: Vec<(&str, Vec<u8>)> = vec![
        (
            "unknown question in a duration node",
            replace_after(&voice, b"{*}[2]\n{\n", b"C-Phone_Muon", b"C-Phone_Muoz"),
        ),
        (
            "reference to a missing node",
            replace_after(&voice, b"{*}[2]\n{\n", b"-108", b"-999"),
        ),
        (
            "LPF leaf index 0",
            replace_after(&voice, b"\"lpf_s2_1\"", b"\"lpf_s2_1\"", b"\"lpf_s2_0\""),
        ),
        (
            "LPF leaf index past the PDF count",
            replace_after(&voice, b"\"lpf_s2_1\"", b"\"lpf_s2_1\"", b"\"lpf_s2_9\""),
        ),
        ("LPF tree for a state that does not exist", state9),
        (
            "vector length overflow",
            replace_text(
                &voice,
                "VECTOR_LENGTH[MCP]:35\n",
                "VECTOR_LENGTH[MCP]:9223372036854775807\n",
            ),
        ),
        (
            "header number overflow",
            replace_text(
                &voice,
                "NUM_STATES:5\n",
                "NUM_STATES:99999999999999999999\n",
            ),
        ),
        (
            "zero states",
            replace_text(&voice, "NUM_STATES:5\n", "NUM_STATES:0\n"),
        ),
    ];
    for (case, bytes) in &targeted {
        assert_eq!(
            exercise(bytes, dict.copy(), case).err(),
            Some(LoadError::VoiceInvalid),
            "{case}"
        );
    }

    for len in [0, 1, 512, data, voice.len() / 3, voice.len() - 1] {
        let result = exercise(&voice[..len], dict.copy(), &format!("truncation to {len}"));
        assert!(result.is_err(), "truncation to {len} must not load");
    }

    // Seeded random corruption: any outcome but a panic is acceptable; a
    // voice that still loads must speak or fail cleanly.
    let mut rng = XorShift(0x4d32_3554_5453);
    let mut loaded = 0;
    for trial in 0..32 {
        let mut bytes = voice.clone();
        for _ in 0..4 {
            let at = rng.below(bytes.len());
            bytes[at] ^= 1 << rng.below(8);
        }
        if exercise(&bytes, dict.copy(), &format!("bit-flip trial {trial}")).is_ok() {
            loaded += 1;
        }
    }
    eprintln!("bit-flip trials that still loaded: {loaded}/32");
}

#[test]
#[ignore = "real-artifact adversarial checks: needs NAGI_TTS_VOICE/NAGI_TTS_DICT"]
fn corrupted_dictionary_bytes_fail_closed_without_panic() {
    let (voice_path, dict_path) = artifacts();
    let voice = std::fs::read(&voice_path).unwrap();
    let dict = Dict::read(&dict_path);
    const MATRIX: usize = 2;
    const VALS: usize = 4;
    const WORDSIDX: usize = 5;
    const WORDS: usize = 6;

    let matrix = &dict.0[MATRIX];
    let vals = &dict.0[VALS];
    let index = &dict.0[WORDSIDX];
    let mut targeted: Vec<(&str, usize, Vec<u8>)> = vec![
        (
            "matrix truncated by one cell",
            MATRIX,
            matrix[..matrix.len() - 2].to_vec(),
        ),
        (
            "matrix odd length",
            MATRIX,
            matrix[..matrix.len() - 1].to_vec(),
        ),
        ("matrix half", MATRIX, matrix[..matrix.len() / 2].to_vec()),
        (
            "matrix trailing bytes",
            MATRIX,
            [matrix.clone(), vec![0, 0]].concat(),
        ),
        ("vals partial entry", VALS, vals[..vals.len() - 1].to_vec()),
        (
            "word index partial offset",
            WORDSIDX,
            index[..index.len() - 1].to_vec(),
        ),
    ];
    let mut big_shape = matrix.clone();
    big_shape[2..6].copy_from_slice(&[0x00, 0x7D, 0x00, 0x7D]);
    targeted.push(("matrix shape larger than data", MATRIX, big_shape));
    let mut negative = matrix.clone();
    negative[2..4].copy_from_slice(&(-2i16).to_le_bytes());
    targeted.push(("matrix negative size", MATRIX, negative));
    let mut ids = vals.clone();
    for entry in ids.chunks_exact_mut(10).step_by(997) {
        entry[6..10].copy_from_slice(&[0xE8, 0xFD, 0xE8, 0xFD]);
    }
    targeted.push(("context ids outside the matrix", VALS, ids));
    let mut swapped = index.clone();
    for i in (4..swapped.len() - 8).step_by(4 * 101) {
        let (a, b) = (swapped[i..i + 4].to_vec(), swapped[i + 4..i + 8].to_vec());
        if a != b {
            swapped[i..i + 4].copy_from_slice(&b);
            swapped[i + 4..i + 8].copy_from_slice(&a);
        }
    }
    targeted.push(("word offsets out of order", WORDSIDX, swapped));
    let mut past = index.clone();
    let last = past.len() - 4;
    past[last..].copy_from_slice(&u32::MAX.to_le_bytes());
    targeted.push(("word offset past the data", WORDSIDX, past));
    let mut preamble = dict.0[WORDS].clone();
    preamble[0] = 0xFF;
    targeted.push(("dictionary preamble not jpreprocess", WORDS, preamble));
    for (case, component, bytes) in targeted {
        assert_eq!(
            exercise(&voice, dict.with(component, bytes), case).err(),
            Some(LoadError::DictionaryInvalid),
            "{case}"
        );
    }

    // Seeded random corruption of every component (bounded: 2 trials each).
    let mut rng = XorShift(0x6469_6374_5453);
    for component in 0..8 {
        for trial in 0..2 {
            let mut bytes = dict.0[component].clone();
            for _ in 0..8 {
                let at = rng.below(bytes.len());
                bytes[at] = rng.next() as u8;
            }
            let what = format!(
                "component {} trial {trial}",
                DictionaryBytes::FILES[component]
            );
            let _ = exercise(&voice, dict.with(component, bytes), &what);
        }
    }
}

#[test]
#[ignore = "real-artifact adversarial checks: needs NAGI_TTS_VOICE/NAGI_TTS_DICT"]
fn lifecycle_after_load_and_synthesis_errors_retains_nothing() {
    let (voice_path, dict_path) = artifacts();
    let voice = std::fs::read(&voice_path).unwrap();
    let dict = Dict::read(&dict_path);
    let stereo = PcmFormat::stereo_48khz();
    let mono = PcmFormat::mono_16khz();

    let mut fresh = load_provider(&voice_path, &dict_path).unwrap();
    let reference = speak(&mut fresh, PHRASE, stereo).unwrap();
    let reference_mono = speak(&mut fresh, PHRASE, mono).unwrap();
    drop(fresh);

    // A failed bytes load leaves nothing behind; the next load is identical.
    let truncated = &voice[..voice.len() / 2];
    assert_eq!(
        JbonsaiBackend::from_bytes(truncated, dict.copy()).err(),
        Some(LoadError::VoiceInvalid)
    );
    let mut matrix = dict.0[2].clone();
    matrix.pop();
    assert_eq!(
        JbonsaiBackend::from_bytes(&voice, dict.with(2, matrix)).err(),
        Some(LoadError::DictionaryInvalid)
    );
    let mut provider =
        LocalTtsProvider::new(JbonsaiBackend::from_bytes(&voice, dict.copy()).unwrap()).unwrap();
    assert_eq!(speak(&mut provider, PHRASE, stereo).unwrap(), reference);

    // Synthesis-time errors: over budget, unsupported language, undersized
    // destination, cancel mid-stream. Each leaves the provider clear and the
    // next utterance byte-identical.
    let long = "今日はとても良い天気ですね。".repeat(23);
    assert_eq!(
        speak(&mut provider, &long, stereo),
        Err(SpeechProviderError::Failed)
    );
    assert!(provider.is_clear());
    let english = SpeechSynthesisOptions {
        language: SpeechSynthesisLanguage::English,
        pcm_format: stereo,
    };
    assert_eq!(
        provider.begin("hello", english),
        Err(SpeechProviderError::Unavailable)
    );
    assert!(provider.is_clear());
    provider.begin(PHRASE, options(mono)).unwrap();
    assert_eq!(
        provider.next_pcm_chunk(&mut [0u8; 1]),
        Err(SpeechProviderError::OutputTooSmall)
    );
    assert!(provider.is_clear());
    assert_eq!(speak(&mut provider, PHRASE, mono).unwrap(), reference_mono);
    provider.begin(PHRASE, options(mono)).unwrap();
    let mut chunk = [0u8; 4096];
    assert!(matches!(
        provider.next_pcm_chunk(&mut chunk),
        Ok(SynthesisPcmChunk::Data(_))
    ));
    provider.cancel();
    assert!(provider.is_clear());
    let mut untouched = [0xA5u8; 256];
    assert_eq!(
        provider.next_pcm_chunk(&mut untouched),
        Err(SpeechProviderError::Failed)
    );
    assert!(
        untouched.iter().all(|b| *b == 0xA5),
        "no stale PCM after cancel"
    );
    assert_eq!(speak(&mut provider, PHRASE, mono).unwrap(), reference_mono);
    assert_eq!(speak(&mut provider, PHRASE, stereo).unwrap(), reference);

    // Unload after an error, a failed reload, then a good reload.
    assert_eq!(
        speak(&mut provider, &long, stereo),
        Err(SpeechProviderError::Failed)
    );
    provider.backend_mut().unload();
    assert_eq!(
        speak(&mut provider, PHRASE, stereo),
        Err(SpeechProviderError::Unavailable)
    );
    let nowhere = PathBuf::from("/nonexistent/nagi-tts/v.htsvoice");
    assert_eq!(
        provider.backend_mut().reload(&nowhere, &dict_path),
        Err(LoadError::VoiceMissing)
    );
    assert_eq!(
        speak(&mut provider, PHRASE, stereo),
        Err(SpeechProviderError::Unavailable)
    );
    assert!(provider.is_clear());
    provider
        .backend_mut()
        .reload(&voice_path, &dict_path)
        .unwrap();
    assert_eq!(speak(&mut provider, PHRASE, stereo).unwrap(), reference);
    assert_eq!(provider.stats().completed_utterances, 5);
}
