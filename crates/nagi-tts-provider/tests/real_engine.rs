//! Real Japanese synthesis on the host with the pinned voice and dictionary.
//!
//! These tests are `#[ignore]`d so a plain `cargo test` runs only the
//! model-free tests and reports these as ignored (never as passed). Run them
//! with `tests/m25-tts/acceptance.sh`, which fetches the pinned artifacts and
//! runs `cargo test --test real_engine -- --ignored`. In that mode a missing
//! `NAGI_TTS_VOICE` / `NAGI_TTS_DICT` is a FAIL, not a skip.
//!
//! Host synthesis is not guest (Nagi/QEMU) evidence.
#![cfg(feature = "engine-jbonsai")]

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use nagi_audio::speech::{
    PlaybackError, SpeechPlaybackSink, SpeechProviderError, SpeechSynthesisError,
    SpeechSynthesisLanguage, SpeechSynthesisOptions, SpeechSynthesisService, SynthesisPcmChunk,
    TextToSpeechProvider, MAX_SPEECH_PCM_CHUNK_BYTES,
};
use nagi_audio::PcmFormat;
use nagi_tts_provider::jbonsai_backend::{
    load_provider, DictionaryBytes, JbonsaiBackend, JbonsaiProvider, LoadError,
};
use nagi_tts_provider::{predicted_pcm_bytes, LocalTtsProvider};

/// Missing artifacts fail the acceptance run.
fn artifacts() -> (PathBuf, PathBuf) {
    let voice = std::env::var_os("NAGI_TTS_VOICE")
        .expect("FAIL: NAGI_TTS_VOICE is not set (run tests/m25-tts/acceptance.sh)");
    let dict = std::env::var_os("NAGI_TTS_DICT")
        .expect("FAIL: NAGI_TTS_DICT is not set (run tests/m25-tts/acceptance.sh)");
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

fn shared() -> MutexGuard<'static, JbonsaiProvider> {
    static PROVIDER: OnceLock<Mutex<JbonsaiProvider>> = OnceLock::new();
    PROVIDER
        .get_or_init(|| {
            let (voice, dict) = artifacts();
            Mutex::new(load_provider(&voice, &dict).expect("pinned artifacts load"))
        })
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
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

fn samples(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

fn rms(values: &[i16]) -> f64 {
    let sum: f64 = values.iter().map(|v| f64::from(*v).powi(2)).sum();
    (sum / values.len().max(1) as f64).sqrt()
}

const PHRASE: &str = "こんにちは、ナギです。";

#[test]
#[ignore = "real synthesis acceptance: run tests/m25-tts/acceptance.sh"]
fn synthesizes_audible_japanese_stereo_48k() {
    let mut provider = shared();
    provider
        .begin(PHRASE, options(PcmFormat::stereo_48khz()))
        .unwrap();
    let pcm = drain(&mut *provider, MAX_SPEECH_PCM_CHUNK_BYTES).unwrap();
    assert!(provider.is_clear());
    assert_eq!(pcm.len() % 4, 0);
    let seconds = pcm.len() as f64 / (48_000.0 * 4.0);
    assert!(
        (0.8..5.0).contains(&seconds),
        "unexpected duration {seconds}"
    );
    let all = samples(&pcm);
    let left: Vec<i16> = all.iter().step_by(2).copied().collect();
    let right: Vec<i16> = all.iter().skip(1).step_by(2).copied().collect();
    assert_eq!(left, right);
    // Speech energy: the loudest 100 ms window is far above silence.
    let window = 4_800;
    let loudest = left.chunks(window).map(rms).fold(0.0f64, f64::max);
    assert!(loudest > 500.0, "loudest window RMS {loudest}");
    // And the waveform is not a constant or a trivial tone: many distinct
    // values and zero crossings.
    let crossings = left.windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count();
    assert!(crossings > 200, "zero crossings {crossings}");
}

#[test]
#[ignore = "real synthesis acceptance: run tests/m25-tts/acceptance.sh"]
fn deterministic_across_reuse_and_chunk_sizes() {
    let mut provider = shared();
    let format = options(PcmFormat::stereo_48khz());
    provider.begin(PHRASE, format).unwrap();
    let first = drain(&mut *provider, 4096).unwrap();
    for chunk in [4usize, 1000, 4096] {
        provider.begin(PHRASE, format).unwrap();
        assert_eq!(
            drain(&mut *provider, chunk).unwrap(),
            first,
            "chunk {chunk}"
        );
    }
    // A different sentence in between does not disturb the next result.
    provider.begin("アルバートを開いて", format).unwrap();
    let other = drain(&mut *provider, 4096).unwrap();
    assert!(!other.is_empty() && other != first);
    provider.begin(PHRASE, format).unwrap();
    assert_eq!(drain(&mut *provider, 4096).unwrap(), first);
}

#[test]
#[ignore = "real synthesis acceptance: run tests/m25-tts/acceptance.sh"]
fn mono_16k_request_is_a_third_of_the_rate() {
    let mut provider = shared();
    provider
        .begin(PHRASE, options(PcmFormat::stereo_48khz()))
        .unwrap();
    let stereo = drain(&mut *provider, 4096).unwrap();
    provider
        .begin(PHRASE, options(PcmFormat::mono_16khz()))
        .unwrap();
    let mono = drain(&mut *provider, 4096).unwrap();
    let stereo_frames = stereo.len() / 4;
    let mono_frames = mono.len() / 2;
    let expected = stereo_frames / 3;
    assert!(
        mono_frames.abs_diff(expected) <= 64,
        "mono {mono_frames} vs {expected}"
    );
    assert!(rms(&samples(&mono)) > 100.0);
}

#[test]
#[ignore = "real synthesis acceptance: run tests/m25-tts/acceptance.sh"]
fn cancel_mid_stream_then_next_request_succeeds() {
    let mut provider = shared();
    let format = options(PcmFormat::stereo_48khz());
    provider.begin(PHRASE, format).unwrap();
    let mut buffer = [0u8; 4096];
    assert!(matches!(
        provider.next_pcm_chunk(&mut buffer),
        Ok(SynthesisPcmChunk::Data(_))
    ));
    provider.cancel();
    assert!(provider.is_clear());
    assert_eq!(
        provider.next_pcm_chunk(&mut buffer),
        Err(SpeechProviderError::Failed)
    );
    provider.begin("はい", format).unwrap();
    assert!(!drain(&mut *provider, 4096).unwrap().is_empty());
}

#[test]
#[ignore = "real synthesis acceptance: run tests/m25-tts/acceptance.sh"]
fn english_and_unspeakable_text_do_not_fake_audio() {
    let mut provider = shared();
    let english = SpeechSynthesisOptions {
        language: SpeechSynthesisLanguage::English,
        pcm_format: PcmFormat::stereo_48khz(),
    };
    assert_eq!(
        provider.begin("hello", english),
        Err(SpeechProviderError::Unavailable)
    );
    provider
        .begin("、。　", options(PcmFormat::stereo_48khz()))
        .unwrap();
    assert!(drain(&mut *provider, 4096).unwrap().is_empty());
    assert!(provider.is_clear());
}

#[test]
#[ignore = "real synthesis acceptance: run tests/m25-tts/acceptance.sh"]
fn long_text_within_input_limit_is_rejected_before_audio() {
    let mut provider = shared();
    // ~990 bytes of Japanese: far more than 5.46 s of stereo 48 kHz audio.
    let text = "今日はとても良い天気ですね。".repeat(23);
    assert!(text.len() <= 1024, "{}", text.len());
    // Predicted over the 1 MiB budget: rejected at begin, before any audio.
    assert_eq!(
        provider.begin(&text, options(PcmFormat::stereo_48khz())),
        Err(SpeechProviderError::Failed)
    );
    assert!(provider.is_clear());
    let mut buffer = [0u8; 4096];
    assert_eq!(
        provider.next_pcm_chunk(&mut buffer),
        Err(SpeechProviderError::Failed),
        "no partial audio is available after the rejection"
    );
}

struct CountingSink {
    bytes: std::rc::Rc<std::cell::Cell<usize>>,
}

impl SpeechPlaybackSink for CountingSink {
    fn play_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), PlaybackError> {
        assert_eq!(format, PcmFormat::stereo_48khz());
        assert!(!bytes.is_empty() && bytes.len().is_multiple_of(4) && bytes.len() <= 4096);
        self.bytes.set(self.bytes.get() + bytes.len());
        Ok(())
    }
}

#[test]
#[ignore = "real synthesis acceptance: run tests/m25-tts/acceptance.sh"]
fn speaks_through_the_synthesis_service_contract() {
    let (voice, dict) = artifacts();
    let provider = load_provider(&voice, &dict).unwrap();
    let played = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let mut service = SpeechSynthesisService::new(
        provider,
        CountingSink {
            bytes: played.clone(),
        },
    );
    let accepted = service
        .speak(PHRASE.as_bytes(), options(PcmFormat::stereo_48khz()))
        .unwrap();
    assert!(accepted > 48_000 * 4 / 2);
    assert_eq!(played.get(), accepted);
    assert_eq!(
        service.speak(&[0xe3, 0x81], options(PcmFormat::stereo_48khz())),
        Err(SpeechSynthesisError::InvalidText)
    );
    assert_eq!(
        service.speak(&[b'a'; 1025], options(PcmFormat::stereo_48khz())),
        Err(SpeechSynthesisError::TextTooLong)
    );
    let long = "今日はとても良い天気ですね。".repeat(23);
    let played_before = played.get();
    assert_eq!(
        service.speak(long.as_bytes(), options(PcmFormat::stereo_48khz())),
        Err(SpeechSynthesisError::ProviderFailed)
    );
    assert_eq!(
        played.get(),
        played_before,
        "nothing played for the rejected text"
    );
    // The provider is reusable after the failure.
    assert!(service
        .speak("はい".as_bytes(), options(PcmFormat::stereo_48khz()))
        .is_ok());
}

#[test]
#[ignore = "real synthesis acceptance: run tests/m25-tts/acceptance.sh"]
fn missing_and_invalid_artifacts_are_reported() {
    let (voice, dict) = artifacts();
    let nowhere = PathBuf::from("/nonexistent/nagi-tts");
    assert_eq!(
        JbonsaiBackend::load(&nowhere.join("v.htsvoice"), &dict).err(),
        Some(LoadError::VoiceMissing)
    );
    assert_eq!(
        JbonsaiBackend::load(&voice, &nowhere).err(),
        Some(LoadError::DictionaryMissing)
    );
    assert_eq!(
        JbonsaiBackend::from_bytes(b"not a voice", DictionaryBytes::read_dir(&dict).unwrap()).err(),
        Some(LoadError::VoiceInvalid)
    );
    let scratch = std::env::temp_dir().join(format!("nagi-tts-bad-dict-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join("metadata.json"), b"{}").unwrap();
    // Incomplete dictionary directory.
    assert_eq!(
        JbonsaiBackend::load(&voice, &scratch).err(),
        Some(LoadError::DictionaryMissing)
    );
    // Complete but corrupt dictionary.
    for name in DictionaryBytes::FILES {
        std::fs::write(scratch.join(name), b"corrupt").unwrap();
    }
    assert_eq!(
        JbonsaiBackend::load(&voice, &scratch).err(),
        Some(LoadError::DictionaryInvalid)
    );
    std::fs::remove_dir_all(&scratch).unwrap();
    let voice_bytes = std::fs::read(&voice).unwrap();
    assert_eq!(
        JbonsaiBackend::from_bytes(&voice_bytes, DictionaryBytes::default()).err(),
        Some(LoadError::DictionaryMissing)
    );
    assert_eq!(
        JbonsaiBackend::from_bytes(&[], DictionaryBytes::read_dir(&dict).unwrap()).err(),
        Some(LoadError::VoiceMissing)
    );
}

#[test]
#[ignore = "real synthesis acceptance: run tests/m25-tts/acceptance.sh"]
fn unload_reports_unavailable_and_reload_restores() {
    let (voice, dict) = artifacts();
    let mut provider: JbonsaiProvider =
        LocalTtsProvider::new(JbonsaiBackend::load(&voice, &dict).unwrap()).unwrap();
    let format = options(PcmFormat::stereo_48khz());
    provider.begin("はい", format).unwrap();
    let before = drain(&mut provider, 4096).unwrap();
    provider.backend_mut().unload();
    assert_eq!(
        provider.begin("はい", format),
        Err(SpeechProviderError::Unavailable)
    );
    assert!(provider.is_clear());
    provider.backend_mut().reload(&voice, &dict).unwrap();
    provider.begin("はい", format).unwrap();
    assert_eq!(drain(&mut provider, 4096).unwrap(), before);
}

#[test]
#[ignore = "real synthesis acceptance: run tests/m25-tts/acceptance.sh"]
fn edge_silence_is_trimmed_but_speech_is_kept() {
    let (voice, dict) = artifacts();
    let format = options(PcmFormat::stereo_48khz());
    let mut trimmed = load_provider(&voice, &dict).unwrap();
    trimmed.begin(PHRASE, format).unwrap();
    let short = drain(&mut trimmed, 4096).unwrap();

    let mut backend = JbonsaiBackend::load(&voice, &dict).unwrap();
    backend.set_silence_trim(nagi_tts_provider::jbonsai_backend::SilenceTrim::DISABLED);
    let mut untrimmed = LocalTtsProvider::new(backend).unwrap();
    untrimmed.begin(PHRASE, format).unwrap();
    let full = drain(&mut untrimmed, 4096).unwrap();

    let removed = (full.len() - short.len()) as f64 / (48_000.0 * 4.0);
    assert!((0.5..1.5).contains(&removed), "removed {removed} s");
    let left = |pcm: &[u8]| -> Vec<i16> { samples(pcm).into_iter().step_by(2).collect() };
    let short_left = left(&short);
    // Voiced audio starts within the first 100 ms of the trimmed output.
    let onset = short_left
        .chunks(240)
        .position(|frame| rms(frame) > 64.0)
        .unwrap();
    assert!(onset <= 30, "onset frame {onset}");
    // The trimmed signal is a contiguous slice of the untrimmed one.
    let full_left = left(&full);
    let found = full_left
        .windows(short_left.len())
        .any(|window| window == short_left.as_slice());
    assert!(found, "trimmed audio must be an exact sub-slice");
}

#[test]
#[ignore = "real synthesis acceptance: run tests/m25-tts/acceptance.sh"]
fn bytes_loader_matches_path_loader() {
    let (voice, dict) = artifacts();
    let format = options(PcmFormat::stereo_48khz());
    let mut from_path = load_provider(&voice, &dict).unwrap();
    from_path.begin(PHRASE, format).unwrap();
    let expected = drain(&mut from_path, 4096).unwrap();
    drop(from_path);

    // Bytes as a read-only model capability would deliver them.
    let voice_bytes = std::fs::read(&voice).unwrap();
    let dictionary = DictionaryBytes::read_dir(&dict).unwrap();
    assert!(dictionary.total_len() > 70_000_000);
    let backend = JbonsaiBackend::from_bytes(&voice_bytes, dictionary).unwrap();
    let mut from_bytes = LocalTtsProvider::new(backend).unwrap();
    from_bytes.begin(PHRASE, format).unwrap();
    assert_eq!(drain(&mut from_bytes, 4096).unwrap(), expected);
}

#[test]
#[ignore = "real synthesis acceptance: run tests/m25-tts/acceptance.sh"]
fn predicted_length_equals_emitted_length() {
    let (voice, dict) = artifacts();
    let backend = JbonsaiBackend::load(&voice, &dict).unwrap();
    let texts = [
        PHRASE,
        "アルバートを開いて",
        "会議は午後三時から、参加者は12人です。",
        "日本語の音声合成が、ローカルで動作しています。",
    ];
    let plans: Vec<_> = texts
        .iter()
        .map(|text| backend.plan(text).unwrap().unwrap().0)
        .collect();
    let mut provider = LocalTtsProvider::new(backend).unwrap();
    for (text, plan) in texts.iter().zip(&plans) {
        for format in [PcmFormat::stereo_48khz(), PcmFormat::mono_16khz()] {
            provider.begin(text, options(format)).unwrap();
            let pcm = drain(&mut provider, 4096).unwrap();
            assert_eq!(
                Some(pcm.len()),
                predicted_pcm_bytes(format, plan.emitted_frames(), 240),
                "{text} {format:?}"
            );
        }
    }
}
