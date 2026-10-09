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
    last_budget: usize,
}

impl TestBackend {
    fn new(plan: Plan) -> Self {
        Self {
            loaded: true,
            plan,
            starts: 0,
            rate: ENGINE_SAMPLE_RATE,
            last_budget: 0,
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
        max_frames: usize,
    ) -> Result<Option<TestSource>, BackendError> {
        if language == SpeechSynthesisLanguage::English {
            return Err(BackendError::UnsupportedLanguage);
        }
        self.starts += 1;
        self.last_budget = max_frames;
        // Like a predicting backend: a known length over budget is rejected
        // before any frame is produced.
        if let Plan::Frames(frames) = self.plan {
            if frames > max_frames {
                return Err(BackendError::TooLong);
            }
        }
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

#[test]
fn budget_matches_output_cap_for_each_format() {
    let mut p = provider(Plan::Frames(1));
    p.begin("a", stereo()).unwrap();
    assert_eq!(p.backend().last_budget, 1092);
    assert_eq!(
        max_frames(PcmFormat::stereo_48khz(), FRAME),
        Some(MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES / (FRAME * 4))
    );
    p.cancel();
    p.begin("a", mono()).unwrap();
    let mono_budget = p.backend().last_budget;
    p.cancel();
    let predicted = predicted_pcm_bytes(PcmFormat::mono_16khz(), mono_budget, FRAME).unwrap();
    assert!(predicted <= MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES);
    assert!(predicted > MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES - 4096);
}

#[test]
fn predicted_bytes_match_emitted_bytes() {
    for frames in [1usize, 2, 3, 7, 40, 1092] {
        for options in [stereo(), mono()] {
            let mut p = provider(Plan::Frames(frames));
            p.begin("a", options).unwrap();
            let out = drain(&mut p, 4096).unwrap();
            assert_eq!(
                Some(out.len()),
                predicted_pcm_bytes(options.pcm_format, frames, FRAME),
                "frames {frames} {:?}",
                options.pcm_format
            );
        }
    }
}

#[test]
fn over_budget_utterance_fails_before_any_audio() {
    let frames = MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES / (FRAME * 4) + 1;
    let mut p = provider(Plan::Frames(frames));
    assert_eq!(p.begin("長い", stereo()), Err(SpeechProviderError::Failed));
    assert!(p.is_clear());
    let mut service =
        SpeechSynthesisService::new(provider(Plan::Frames(frames)), RecordingSink::default());
    assert_eq!(
        service.speak("長い".as_bytes(), stereo()),
        Err(SpeechSynthesisError::ProviderFailed)
    );
    // The same length fits as 16 kHz mono, whose byte rate is 6x lower.
    let mut mono_provider = provider(Plan::Frames(frames));
    mono_provider.begin("長い", mono()).unwrap();
    assert!(!drain(&mut mono_provider, 4096).unwrap().is_empty());
}

#[test]
fn playback_sink_sees_no_audio_for_over_budget_text() {
    struct PanicSink;
    impl SpeechPlaybackSink for PanicSink {
        fn play_pcm(&mut self, _: PcmFormat, _: &[u8]) -> Result<(), PlaybackError> {
            panic!("over-budget utterance must not reach playback");
        }
    }
    let frames = MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES / (FRAME * 4) + 1;
    let mut service = SpeechSynthesisService::new(provider(Plan::Frames(frames)), PanicSink);
    assert_eq!(
        service.speak("長い".as_bytes(), stereo()),
        Err(SpeechSynthesisError::ProviderFailed)
    );
}

// ---------------------------------------------------------------------------
// Lifecycle after errors: no retained PCM or engine state, reuse matches a
// fresh provider byte for byte.
// ---------------------------------------------------------------------------

fn fresh_output(plan: Plan, options: SpeechSynthesisOptions) -> Vec<u8> {
    let mut p = provider(plan);
    p.begin("基準", options).unwrap();
    drain(&mut p, 1000).unwrap()
}

#[test]
fn begin_errors_leave_no_state_and_reuse_matches_fresh_provider() {
    let reference_stereo = fresh_output(Plan::Frames(7), stereo());
    let reference_mono = fresh_output(Plan::Frames(7), mono());
    let english = SpeechSynthesisOptions {
        language: SpeechSynthesisLanguage::English,
        pcm_format: PcmFormat::stereo_48khz(),
    };
    let mut p = provider(Plan::Frames(usize::MAX));
    for options in [stereo(), mono()] {
        // Over budget: rejected by the backend before any audio.
        assert_eq!(p.begin("長い", options), Err(SpeechProviderError::Failed));
        assert!(p.is_clear());
    }
    assert_eq!(
        p.begin("英語", english),
        Err(SpeechProviderError::Unavailable)
    );
    assert!(p.is_clear());
    assert_eq!(p.begin("", stereo()), Err(SpeechProviderError::Failed));
    assert!(p.is_clear());
    p.backend_mut().loaded = false;
    assert_eq!(p.begin("未", mono()), Err(SpeechProviderError::Unavailable));
    assert!(p.is_clear());
    p.backend_mut().loaded = true;
    // None of the failed begins counted as an utterance.
    assert_eq!(p.stats(), ProviderStats::default());

    p.backend_mut().plan = Plan::Frames(7);
    for _ in 0..3 {
        p.begin("基準", stereo()).unwrap();
        assert_eq!(drain(&mut p, 1000).unwrap(), reference_stereo);
        p.begin("基準", mono()).unwrap();
        assert_eq!(drain(&mut p, 1000).unwrap(), reference_mono);
        assert!(p.is_clear());
    }
}

#[test]
fn mono_failure_mid_stream_leaves_no_decimator_state() {
    let reference = fresh_output(Plan::Frames(7), mono());
    for fail_after in [0, 1, 3, 40] {
        let mut p = provider(Plan::FailAfter(fail_after));
        p.begin("失敗", mono()).unwrap();
        assert_eq!(drain(&mut p, 1000), Err(SpeechProviderError::Failed));
        assert!(p.is_clear());
        assert_eq!(p.stats().aborted_utterances, 1);
        // The next utterance starts from a reset filter: identical to a
        // provider that never saw the failed one.
        p.backend_mut().plan = Plan::Frames(7);
        p.begin("基準", mono()).unwrap();
        assert_eq!(
            drain(&mut p, 1000).unwrap(),
            reference,
            "fail_after={fail_after}"
        );
    }
}

#[test]
fn no_pcm_is_written_after_end_error_or_cancel() {
    let mut p = provider(Plan::Frames(3));
    let mut buffer = [0xA5u8; 512];
    let untouched = |buffer: &[u8]| buffer.iter().all(|b| *b == 0xA5);

    // After End.
    p.begin("一", stereo()).unwrap();
    drain(&mut p, 4096).unwrap();
    assert_eq!(
        p.next_pcm_chunk(&mut buffer),
        Err(SpeechProviderError::Failed)
    );
    assert!(untouched(&buffer));

    // After a mid-stream error.
    p.backend_mut().plan = Plan::NotFinite;
    p.begin("二", stereo()).unwrap();
    let mut scratch = [0u8; 4096];
    assert_eq!(
        p.next_pcm_chunk(&mut scratch),
        Err(SpeechProviderError::Failed)
    );
    assert!(scratch.iter().all(|b| *b == 0), "failed chunk is zeroed");
    assert_eq!(
        p.next_pcm_chunk(&mut buffer),
        Err(SpeechProviderError::Failed)
    );
    assert!(untouched(&buffer));

    // After an undersized destination aborted the utterance.
    p.backend_mut().plan = Plan::Frames(3);
    p.begin("三", mono()).unwrap();
    assert_eq!(
        p.next_pcm_chunk(&mut [0u8; 1]),
        Err(SpeechProviderError::OutputTooSmall)
    );
    assert_eq!(
        p.next_pcm_chunk(&mut buffer),
        Err(SpeechProviderError::Failed)
    );
    assert!(untouched(&buffer));

    // After cancel.
    p.begin("四", stereo()).unwrap();
    p.cancel();
    p.cancel();
    assert_eq!(
        p.next_pcm_chunk(&mut buffer),
        Err(SpeechProviderError::Failed)
    );
    assert!(untouched(&buffer));
    assert!(p.is_clear());
}

#[test]
fn unload_after_error_reports_unavailable_until_reloaded() {
    let reference = fresh_output(Plan::Frames(4), stereo());
    let mut p = provider(Plan::FailAfter(2));
    p.begin("失敗", stereo()).unwrap();
    assert_eq!(drain(&mut p, 4096), Err(SpeechProviderError::Failed));
    p.backend_mut().loaded = false;
    assert_eq!(
        p.begin("再", stereo()),
        Err(SpeechProviderError::Unavailable)
    );
    assert!(p.is_clear());
    p.backend_mut().loaded = true;
    p.backend_mut().plan = Plan::Frames(4);
    p.begin("基準", stereo()).unwrap();
    assert_eq!(drain(&mut p, 4096).unwrap(), reference);
}
