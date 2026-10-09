//! Real Japanese synthesis on the host with the pinned voice and dictionary.
//!
//! Requires `tools/tts/fetch.sh` and the environment variables
//! `NAGI_TTS_VOICE` (path to `tohoku-f01-neutral.htsvoice`) and
//! `NAGI_TTS_DICT` (path to the unpacked `naist-jdic` directory). Without
//! them every test prints `SKIP (artifacts absent)` and returns; a skip is
//! not a pass of real synthesis. Host synthesis is not guest evidence.
#![cfg(feature = "engine-jbonsai")]

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock};

use nagi_audio::speech::{
    PlaybackError, SpeechPlaybackSink, SpeechProviderError, SpeechSynthesisError,
    SpeechSynthesisLanguage, SpeechSynthesisOptions, SpeechSynthesisService, SynthesisPcmChunk,
    TextToSpeechProvider, MAX_SPEECH_PCM_CHUNK_BYTES, MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES,
};
use nagi_audio::PcmFormat;
use nagi_tts_provider::jbonsai_backend::{
    load_provider, JbonsaiBackend, JbonsaiProvider, LoadError,
};
use nagi_tts_provider::LocalTtsProvider;

fn artifacts() -> Option<(PathBuf, PathBuf)> {
    let voice = std::env::var_os("NAGI_TTS_VOICE")?;
    let dict = std::env::var_os("NAGI_TTS_DICT")?;
    Some((PathBuf::from(voice), PathBuf::from(dict)))
}

fn shared() -> Option<MutexGuard<'static, JbonsaiProvider>> {
    static PROVIDER: OnceLock<Option<Mutex<JbonsaiProvider>>> = OnceLock::new();
    let slot = PROVIDER.get_or_init(|| {
        let (voice, dict) = artifacts()?;
        Some(Mutex::new(
            load_provider(&voice, &dict).expect("pinned artifacts load"),
        ))
    });
    match slot {
        Some(mutex) => Some(mutex.lock().unwrap_or_else(|poison| poison.into_inner())),
        None => {
            eprintln!("SKIP (artifacts absent): set NAGI_TTS_VOICE and NAGI_TTS_DICT");
            None
        }
    }
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
fn synthesizes_audible_japanese_stereo_48k() {
    let Some(mut provider) = shared() else { return };
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
fn deterministic_across_reuse_and_chunk_sizes() {
    let Some(mut provider) = shared() else { return };
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
fn mono_16k_request_is_a_third_of_the_rate() {
    let Some(mut provider) = shared() else { return };
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
fn cancel_mid_stream_then_next_request_succeeds() {
    let Some(mut provider) = shared() else { return };
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
fn english_and_unspeakable_text_do_not_fake_audio() {
    let Some(mut provider) = shared() else { return };
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
fn long_text_within_input_limit_fails_closed_at_output_cap() {
    let Some(mut provider) = shared() else { return };
    // ~990 bytes of Japanese: far more than 5.46 s of stereo 48 kHz audio.
    let text = "今日はとても良い天気ですね。".repeat(23);
    assert!(text.len() <= 1024, "{}", text.len());
    provider
        .begin(&text, options(PcmFormat::stereo_48khz()))
        .unwrap();
    let mut buffer = [0u8; 4096];
    let mut total = 0usize;
    let result = loop {
        match provider.next_pcm_chunk(&mut buffer) {
            Ok(SynthesisPcmChunk::Data(n)) => total += n,
            Ok(SynthesisPcmChunk::End) => break Ok(()),
            Err(error) => break Err(error),
        }
    };
    assert_eq!(result, Err(SpeechProviderError::Failed));
    assert_eq!(total, MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES);
    assert!(provider.is_clear());
}

struct CountingSink {
    bytes: usize,
    chunks: usize,
}

impl SpeechPlaybackSink for CountingSink {
    fn play_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), PlaybackError> {
        assert_eq!(format, PcmFormat::stereo_48khz());
        assert!(!bytes.is_empty() && bytes.len().is_multiple_of(4) && bytes.len() <= 4096);
        self.bytes += bytes.len();
        self.chunks += 1;
        Ok(())
    }
}

#[test]
fn speaks_through_the_synthesis_service_contract() {
    let Some((voice, dict)) = artifacts() else {
        eprintln!("SKIP (artifacts absent)");
        return;
    };
    let provider = load_provider(&voice, &dict).unwrap();
    let mut service = SpeechSynthesisService::new(
        provider,
        CountingSink {
            bytes: 0,
            chunks: 0,
        },
    );
    let accepted = service
        .speak(PHRASE.as_bytes(), options(PcmFormat::stereo_48khz()))
        .unwrap();
    assert!(accepted > 48_000 * 4 / 2);
    assert_eq!(
        service.speak(&[0xe3, 0x81], options(PcmFormat::stereo_48khz())),
        Err(SpeechSynthesisError::InvalidText)
    );
    assert_eq!(
        service.speak(&[b'a'; 1025], options(PcmFormat::stereo_48khz())),
        Err(SpeechSynthesisError::TextTooLong)
    );
    let long = "今日はとても良い天気ですね。".repeat(23);
    assert_eq!(
        service.speak(long.as_bytes(), options(PcmFormat::stereo_48khz())),
        Err(SpeechSynthesisError::ProviderFailed)
    );
    // The provider is reusable after the failure.
    assert!(service
        .speak("はい".as_bytes(), options(PcmFormat::stereo_48khz()))
        .is_ok());
}

#[test]
fn missing_and_invalid_artifacts_are_reported() {
    let Some((voice, dict)) = artifacts() else {
        eprintln!("SKIP (artifacts absent)");
        return;
    };
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
        JbonsaiBackend::from_voice_bytes(b"not a voice", &dict).err(),
        Some(LoadError::VoiceInvalid)
    );
    let scratch = std::env::temp_dir().join(format!("nagi-tts-bad-dict-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join("metadata.json"), b"{}").unwrap();
    assert_eq!(
        JbonsaiBackend::load(&voice, &scratch).err(),
        Some(LoadError::DictionaryInvalid)
    );
    std::fs::remove_dir_all(&scratch).unwrap();
}

#[test]
fn unload_reports_unavailable_and_reload_restores() {
    let Some((voice, dict)) = artifacts() else {
        eprintln!("SKIP (artifacts absent)");
        return;
    };
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
