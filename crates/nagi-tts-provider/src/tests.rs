//! Orchestration tests over a deterministic test-only backend. They exercise
//! the provider's limits, chunking, cancellation, and cleanup; they make no
//! claim about speech. Real synthesis is covered by `tests/real_engine.rs`.

use super::*;
use alloc::vec;
use nagi_audio::speech::{
    PlaybackError, SpeechPlaybackSink, SpeechSynthesisError, SpeechSynthesisService,
};

const FRAME: usize = 240;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Plan {
    /// Produce `frames` frames of a ramp.
    Frames(usize),
    /// Produce `frames` frames, then fail.
    FailAfter(usize),
    /// Text had nothing speakable.
    Nothing,
    /// Produce NaN samples.
    NotFinite,
    /// Produce frames forever.
    Endless,
}

struct TestBackend {
    loaded: bool,
    plan: Plan,
    starts: u32,
    rate: u32,
}

impl TestBackend {
    fn new(plan: Plan) -> Self {
        Self {
            loaded: true,
            plan,
            starts: 0,
            rate: ENGINE_SAMPLE_RATE,
        }
    }
}

struct TestSource {
    plan: Plan,
    produced: usize,
}

impl FrameSource for TestSource {
    fn next_frame(&mut self, out: &mut [f64]) -> Result<usize, BackendError> {
        let limit = match self.plan {
            Plan::Frames(n) => Some(n),
            Plan::FailAfter(n) => {
                if self.produced == n {
                    return Err(BackendError::Failed);
                }
                None
            }
            Plan::Endless => None,
            Plan::NotFinite => {
                out[..FRAME].fill(f64::NAN);
                return Ok(FRAME);
            }
            Plan::Nothing => Some(0),
        };
        if limit.is_some_and(|limit| self.produced >= limit) {
            return Ok(0);
        }
        for (index, sample) in out[..FRAME].iter_mut().enumerate() {
            *sample = ((self.produced * FRAME + index) % 2000) as f64 - 1000.0;
        }
        self.produced += 1;
        Ok(FRAME)
    }
}

impl SynthesisBackend for TestBackend {
    type Source = TestSource;
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn frame_len(&self) -> usize {
        FRAME
    }
    fn is_loaded(&self) -> bool {
        self.loaded
    }
    fn start(
        &mut self,
        _text: &str,
        language: SpeechSynthesisLanguage,
    ) -> Result<Option<TestSource>, BackendError> {
        if language == SpeechSynthesisLanguage::English {
            return Err(BackendError::UnsupportedLanguage);
        }
        self.starts += 1;
        if self.plan == Plan::Nothing {
            return Ok(None);
        }
        Ok(Some(TestSource {
            plan: self.plan,
            produced: 0,
        }))
    }
}

fn stereo() -> SpeechSynthesisOptions {
    SpeechSynthesisOptions {
        language: SpeechSynthesisLanguage::Japanese,
        pcm_format: PcmFormat::stereo_48khz(),
    }
}

fn mono() -> SpeechSynthesisOptions {
    SpeechSynthesisOptions {
        language: SpeechSynthesisLanguage::Japanese,
        pcm_format: PcmFormat::mono_16khz(),
    }
}

fn provider(plan: Plan) -> LocalTtsProvider<TestBackend> {
    LocalTtsProvider::new(TestBackend::new(plan)).unwrap()
}

fn drain(
    provider: &mut LocalTtsProvider<TestBackend>,
    chunk: usize,
) -> Result<Vec<u8>, SpeechProviderError> {
    let mut out = Vec::new();
    let mut buffer = vec![0u8; chunk];
    loop {
        match provider.next_pcm_chunk(&mut buffer)? {
            SynthesisPcmChunk::Data(n) => out.extend_from_slice(&buffer[..n]),
            SynthesisPcmChunk::End => return Ok(out),
        }
    }
}

#[derive(Default)]
struct RecordingSink {
    bytes: Vec<u8>,
    chunks: Vec<usize>,
    fail_after: Option<usize>,
}

impl SpeechPlaybackSink for RecordingSink {
    fn play_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), PlaybackError> {
        assert_eq!(format, PcmFormat::stereo_48khz());
        if self.fail_after == Some(self.chunks.len()) {
            return Err(PlaybackError::Unavailable);
        }
        self.chunks.push(bytes.len());
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
}

#[test]
fn rejects_empty_and_oversized_text_before_backend_start() {
    let mut p = provider(Plan::Frames(4));
    assert_eq!(p.begin("", stereo()), Err(SpeechProviderError::Failed));
    let at_limit = "a".repeat(MAX_SPEECH_SYNTHESIS_TEXT_BYTES);
    let over = "a".repeat(MAX_SPEECH_SYNTHESIS_TEXT_BYTES + 1);
    assert_eq!(p.begin(&over, stereo()), Err(SpeechProviderError::Failed));
    // Multi-byte text is measured in bytes, not characters.
    let japanese_over = "あ".repeat(MAX_SPEECH_SYNTHESIS_TEXT_BYTES / 3 + 1);
    assert!(japanese_over.len() > MAX_SPEECH_SYNTHESIS_TEXT_BYTES);
    assert_eq!(
        p.begin(&japanese_over, stereo()),
        Err(SpeechProviderError::Failed)
    );
    assert_eq!(p.backend().starts, 0);
    assert!(p.is_clear());
    assert_eq!(p.begin(&at_limit, stereo()), Ok(()));
    assert_eq!(p.backend().starts, 1);
    p.cancel();
    assert!(p.is_clear());
}

#[test]
fn rejects_unsupported_formats_and_languages() {
    let mut p = provider(Plan::Frames(4));
    let bad = SpeechSynthesisOptions {
        language: SpeechSynthesisLanguage::English,
        pcm_format: PcmFormat::stereo_48khz(),
    };
    assert_eq!(p.begin("hello", bad), Err(SpeechProviderError::Unavailable));
    assert!(p.is_clear());
    let auto = SpeechSynthesisOptions {
        language: SpeechSynthesisLanguage::Auto,
        pcm_format: PcmFormat::stereo_48khz(),
    };
    assert_eq!(p.begin("こんにちは", auto), Ok(()));
    p.cancel();
}

#[test]
fn stereo_chunks_are_bounded_whole_frames_and_concatenate_exactly() {
    let frames = 37;
    let mut whole = provider(Plan::Frames(frames));
    whole.begin("テスト", stereo()).unwrap();
    let reference = drain(&mut whole, MAX_SPEECH_PCM_CHUNK_BYTES).unwrap();
    assert_eq!(reference.len(), frames * FRAME * 4);
    // Left and right channels are identical.
    for frame in reference.chunks_exact(4) {
        assert_eq!(frame[..2], frame[2..]);
    }
    for chunk in [4usize, 6, 100, 960, 961, 4095, 4096, 8192] {
        let mut p = provider(Plan::Frames(frames));
        p.begin("テスト", stereo()).unwrap();
        let mut buffer = vec![0u8; chunk];
        let mut out = Vec::new();
        while let SynthesisPcmChunk::Data(n) = p.next_pcm_chunk(&mut buffer).unwrap() {
            assert!(n > 0 && n.is_multiple_of(4), "chunk {chunk} gave {n}");
            assert!(n <= MAX_SPEECH_PCM_CHUNK_BYTES.min(chunk));
            out.extend_from_slice(&buffer[..n]);
        }
        assert_eq!(out, reference, "chunk size {chunk}");
        assert!(p.is_clear());
    }
}

#[test]
fn undersized_destination_fails_and_clears() {
    let mut p = provider(Plan::Frames(3));
    p.begin("テスト", stereo()).unwrap();
    let mut tiny = [0u8; 3];
    assert_eq!(
        p.next_pcm_chunk(&mut tiny),
        Err(SpeechProviderError::OutputTooSmall)
    );
    assert!(p.is_clear());
    assert_eq!(p.stats().aborted_utterances, 1);
}

#[test]
fn mono_16k_output_matches_shared_decimator() {
    let frames = 25;
    let mut p = provider(Plan::Frames(frames));
    p.begin("テスト", mono()).unwrap();
    let mono_bytes = drain(&mut p, 1000).unwrap();
    assert!(mono_bytes.len().is_multiple_of(2));

    let mut s = provider(Plan::Frames(frames));
    s.begin("テスト", stereo()).unwrap();
    let stereo_bytes = drain(&mut s, 4096).unwrap();
    let mut reference = vec![0u8; stereo_bytes.len()];
    let mut decimator = Stereo48KhzToMono16Khz::new();
    let n = decimator.convert(&stereo_bytes, &mut reference).unwrap();
    let mut tail = vec![0u8; 4096];
    let t = decimator.finish(&mut tail).unwrap();
    let mut expected = reference[..n * 2].to_vec();
    expected.extend_from_slice(&tail[..t * 2]);
    assert_eq!(mono_bytes, expected);
    assert!(p.is_clear());
}

#[test]
fn output_cap_fails_closed_without_truncated_success() {
    let mut p = provider(Plan::Endless);
    p.begin("長い文", stereo()).unwrap();
    let mut buffer = [0u8; MAX_SPEECH_PCM_CHUNK_BYTES];
    let mut total = 0usize;
    let error = loop {
        match p.next_pcm_chunk(&mut buffer) {
            Ok(SynthesisPcmChunk::Data(n)) => total += n,
            Ok(SynthesisPcmChunk::End) => panic!("endless source must not end"),
            Err(error) => break error,
        }
    };
    assert_eq!(error, SpeechProviderError::Failed);
    assert!(total <= MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES);
    assert_eq!(total, MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES);
    assert!(buffer.iter().all(|b| *b == 0), "failed chunk is erased");
    assert!(p.is_clear());
    assert_eq!(p.stats().aborted_utterances, 1);
}

#[test]
fn exactly_at_cap_succeeds() {
    // 1 MiB / (240 samples * 4 bytes) is not integral; pick frames so the
    // total lands at or under the cap.
    let frames = MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES / (FRAME * 4);
    let mut p = provider(Plan::Frames(frames));
    p.begin("文", stereo()).unwrap();
    let out = drain(&mut p, 4096).unwrap();
    assert_eq!(out.len(), frames * FRAME * 4);
    assert!(out.len() <= MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES);
}

#[test]
fn cancel_mid_stream_clears_and_provider_is_reusable() {
    let mut p = provider(Plan::Frames(50));
    p.begin("一つ目", stereo()).unwrap();
    let mut buffer = [0u8; 1000];
    assert!(matches!(
        p.next_pcm_chunk(&mut buffer),
        Ok(SynthesisPcmChunk::Data(_))
    ));
    p.cancel();
    assert!(p.is_clear());
    assert_eq!(
        p.next_pcm_chunk(&mut buffer),
        Err(SpeechProviderError::Failed),
        "no stale audio after cancel"
    );
    p.begin("二つ目", stereo()).unwrap();
    let second = drain(&mut p, 4096).unwrap();
    assert_eq!(second.len(), 50 * FRAME * 4);
    assert_eq!(p.backend().starts, 2);
    assert_eq!(p.stats().completed_utterances, 1);
    assert_eq!(p.stats().aborted_utterances, 1);
}

#[test]
fn reuse_across_many_requests_is_deterministic() {
    let mut p = provider(Plan::Frames(9));
    p.begin("同じ", stereo()).unwrap();
    let first = drain(&mut p, 4096).unwrap();
    for _ in 0..20 {
        p.begin("同じ", stereo()).unwrap();
        assert_eq!(drain(&mut p, 4096).unwrap(), first);
        assert!(p.is_clear());
    }
    assert_eq!(p.stats().completed_utterances, 21);
}

#[test]
fn backend_failure_mid_stream_cleans_up() {
    let mut p = provider(Plan::FailAfter(5));
    p.begin("失敗", stereo()).unwrap();
    let mut buffer = [0u8; 4096];
    let error = loop {
        match p.next_pcm_chunk(&mut buffer) {
            Ok(SynthesisPcmChunk::Data(_)) => {}
            Ok(SynthesisPcmChunk::End) => panic!("must fail"),
            Err(error) => break error,
        }
    };
    assert_eq!(error, SpeechProviderError::Failed);
    assert!(p.is_clear());
    assert!(buffer.iter().all(|b| *b == 0));
}

#[test]
fn non_finite_samples_are_rejected() {
    let mut p = provider(Plan::NotFinite);
    p.begin("数値", stereo()).unwrap();
    let mut buffer = [0u8; 4096];
    assert_eq!(
        p.next_pcm_chunk(&mut buffer),
        Err(SpeechProviderError::Failed)
    );
    assert!(p.is_clear());
}

#[test]
fn missing_or_unloaded_model_reports_unavailable() {
    let mut backend = TestBackend::new(Plan::Frames(1));
    backend.loaded = false;
    let mut p = LocalTtsProvider::new(backend).unwrap();
    assert_eq!(
        p.begin("こんにちは", stereo()),
        Err(SpeechProviderError::Unavailable)
    );
    assert!(p.is_clear());
    assert_eq!(p.backend().starts, 0);
    p.backend_mut().loaded = true;
    assert_eq!(p.begin("こんにちは", stereo()), Ok(()));
}

#[test]
fn wrong_engine_rate_is_rejected() {
    let mut backend = TestBackend::new(Plan::Frames(1));
    backend.rate = 22_050;
    assert!(LocalTtsProvider::new(backend).is_err());
}

#[test]
fn double_begin_aborts_first_utterance() {
    let mut p = provider(Plan::Frames(5));
    p.begin("一", stereo()).unwrap();
    assert_eq!(p.begin("二", stereo()), Err(SpeechProviderError::Failed));
    assert!(p.is_clear());
    p.begin("三", stereo()).unwrap();
    assert_eq!(drain(&mut p, 4096).unwrap().len(), 5 * FRAME * 4);
}

#[test]
fn nothing_speakable_ends_without_audio() {
    let mut p = provider(Plan::Nothing);
    p.begin("、。", stereo()).unwrap();
    assert_eq!(drain(&mut p, 4096).unwrap().len(), 0);
    let mut service =
        SpeechSynthesisService::new(provider(Plan::Nothing), RecordingSink::default());
    assert_eq!(
        service.speak("、。".as_bytes(), stereo()),
        Err(SpeechSynthesisError::NoAudioProduced)
    );
}

#[test]
fn integrates_with_synthesis_service_and_playback_sink() {
    let frames = 30;
    let mut service =
        SpeechSynthesisService::new(provider(Plan::Frames(frames)), RecordingSink::default());
    assert_eq!(
        service.speak("こんにちは".as_bytes(), stereo()),
        Ok(frames * FRAME * 4)
    );
    let mut endless =
        SpeechSynthesisService::new(provider(Plan::Endless), RecordingSink::default());
    assert_eq!(
        endless.speak("長い".as_bytes(), stereo()),
        Err(SpeechSynthesisError::ProviderFailed)
    );
    let failing_sink = RecordingSink {
        fail_after: Some(2),
        ..RecordingSink::default()
    };
    let mut broken = SpeechSynthesisService::new(provider(Plan::Frames(frames)), failing_sink);
    assert_eq!(
        broken.speak("再生".as_bytes(), stereo()),
        Err(SpeechSynthesisError::PlaybackUnavailable)
    );
}

/// Frame script: `true` = voiced (value = index + 1000), `false` = silent (0).
struct ScriptSource {
    script: Vec<bool>,
    next: usize,
}

impl FrameSource for ScriptSource {
    fn next_frame(&mut self, out: &mut [f64]) -> Result<usize, BackendError> {
        let Some(voiced) = self.script.get(self.next).copied() else {
            return Ok(0);
        };
        let value = if voiced {
            1000.0 + self.next as f64
        } else {
            self.next as f64 * 1e-3
        };
        out[..FRAME].fill(value);
        self.next += 1;
        Ok(FRAME)
    }
}

fn trimmed(script: &[bool], config: SilenceTrim) -> Vec<usize> {
    let mut trimmer = SilenceTrimmer::new(
        ScriptSource {
            script: script.to_vec(),
            next: 0,
        },
        FRAME,
        config,
    );
    let mut out = vec![0.0; FRAME];
    let mut indices = Vec::new();
    while trimmer.next_frame(&mut out).unwrap() == FRAME {
        // Recover the source index from the encoded value.
        let index = if out[0] >= 1000.0 {
            (out[0] - 1000.0) as usize
        } else {
            (out[0] * 1e3).round() as usize
        };
        indices.push(index);
    }
    indices
}

#[test]
fn silence_trim_keeps_padding_and_internal_pauses() {
    let config = SilenceTrim {
        rms_threshold: 64.0,
        pad_frames: 2,
        max_hold_frames: 4,
    };
    // 6 silent, 3 voiced, 3 silent (internal pause), 2 voiced, 4 silent
    // (trailing run within the hold limit).
    let mut script = vec![false; 6];
    script.extend([true; 3]);
    script.extend([false; 3]);
    script.extend([true; 2]);
    script.extend([false; 4]);
    let got = trimmed(&script, config);
    let expected: Vec<usize> = (4..16).collect();
    assert_eq!(got, expected);
}

#[test]
fn silence_trim_releases_long_internal_pauses_in_order() {
    let config = SilenceTrim {
        rms_threshold: 64.0,
        pad_frames: 1,
        max_hold_frames: 3,
    };
    let mut script = vec![true; 2];
    script.extend([false; 8]);
    script.push(true);
    script.extend([false; 3]);
    let got = trimmed(&script, config);
    let expected: Vec<usize> = (0..12).collect();
    assert_eq!(got, expected, "every internal frame kept, order preserved");
    // A trailing run longer than the hold limit releases its oldest frames
    // before the end is known; only the final held run is trimmed.
    let mut long_tail = vec![true; 1];
    long_tail.extend([false; 5]);
    assert_eq!(trimmed(&long_tail, config), vec![0, 1, 2, 3]);
}

#[test]
fn silence_trim_all_silent_and_disabled() {
    let config = SilenceTrim::HTS_48K;
    assert!(trimmed(&[false; 50], config).is_empty());
    let script = [false, true, false];
    assert_eq!(trimmed(&script, SilenceTrim::DISABLED), vec![0, 1, 2]);
}

#[test]
fn no_std_sqrt_matches_reference() {
    for value in [1e-9, 0.25, 1.0, 2.0, 4096.0, 1e6, 1.073741824e9, 1e12] {
        let got = libm_sqrt(value);
        let want = value.sqrt();
        assert!(
            (got - want).abs() <= want * 1e-12,
            "{value}: {got} vs {want}"
        );
    }
    assert_eq!(libm_sqrt(0.0), 0.0);
    assert_eq!(libm_sqrt(-1.0), 0.0);
}
