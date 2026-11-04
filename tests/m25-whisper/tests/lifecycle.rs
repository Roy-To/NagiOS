//! Orchestration-only tests of the shared provider lifecycle. The engine here
//! is a scripted test double: these tests verify buffering, cancel, unload,
//! and failure recovery, not speech recognition.
//!
//! Scope label: SCRIPTED ENGINE, SAME SESSION. Every unload/reload here
//! reloads the same `WhisperSession` value. The real-engine evaluation binary
//! (`src/bin/m25-whisper-eval.rs`) instead reloads in a NEW session after an
//! injected model-read failure; the two are not interchangeable evidence.
//!
//! Cancellation scope: `cancel` discards capture only. `finish` is
//! synchronous with no inference abort, so these tests do not (and cannot)
//! show cancellation of in-flight inference.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use nagi_audio::speech::{
    SpeechLanguage, SpeechOptions, SpeechProviderError, SpeechToTextProvider,
    MAX_SPEECH_PCM_CHUNK_BYTES, MAX_SPEECH_UTTERANCE_BYTES,
};
use nagi_audio::PcmFormat;
use nagi_m25_whisper_tests::provider_session::{
    trim_transcript, WhisperEngine, WhisperEngineError, WhisperEngineLoader, WhisperLanguage,
    WhisperSession, WhisperSessionState, MAX_CONSECUTIVE_ENGINE_FAILURES,
};

type Reply = Result<&'static [u8], WhisperEngineError>;

#[derive(Default)]
struct Script {
    replies: VecDeque<Reply>,
    seen_samples: Vec<usize>,
    seen_languages: Vec<WhisperLanguage>,
    live_engines: i32,
}

struct ScriptedEngine(Rc<RefCell<Script>>);

impl WhisperEngine for ScriptedEngine {
    fn transcribe(
        &mut self,
        samples: &[f32],
        language: WhisperLanguage,
        output: &mut [u8],
    ) -> Result<usize, WhisperEngineError> {
        let mut script = self.0.borrow_mut();
        script.seen_samples.push(samples.len());
        script.seen_languages.push(language);
        let text = script.replies.pop_front().expect("unscripted utterance")?;
        if text.len() > output.len() {
            return Err(WhisperEngineError::OutputTooSmall);
        }
        output[..text.len()].copy_from_slice(text);
        Ok(text.len())
    }
}

impl Drop for ScriptedEngine {
    fn drop(&mut self) {
        self.0.borrow_mut().live_engines -= 1;
    }
}

struct ScriptedLoader {
    script: Rc<RefCell<Script>>,
    fail_loads: u32,
}

impl WhisperEngineLoader for ScriptedLoader {
    type Engine = ScriptedEngine;
    fn load(&mut self) -> Option<ScriptedEngine> {
        if self.fail_loads > 0 {
            self.fail_loads -= 1;
            return None;
        }
        self.script.borrow_mut().live_engines += 1;
        Some(ScriptedEngine(self.script.clone()))
    }
}

const JA: SpeechOptions = SpeechOptions {
    language: SpeechLanguage::Japanese,
    pcm_format: PcmFormat::mono_16khz(),
};

fn session(
    replies: &[Reply],
    fail_loads: u32,
) -> (WhisperSession<ScriptedLoader>, Rc<RefCell<Script>>) {
    let script = Rc::new(RefCell::new(Script {
        replies: replies.iter().copied().collect(),
        ..Script::default()
    }));
    let session = WhisperSession::new(ScriptedLoader {
        script: script.clone(),
        fail_loads,
    });
    (session, script)
}

fn utterance(
    session: &mut WhisperSession<ScriptedLoader>,
    samples: usize,
    output: &mut [u8],
) -> Result<usize, SpeechProviderError> {
    session.begin(JA)?;
    let pcm = vec![0x11_u8; samples * 2];
    for chunk in pcm.chunks(MAX_SPEECH_PCM_CHUNK_BYTES) {
        session.push_pcm(PcmFormat::mono_16khz(), chunk)?;
    }
    session.finish(output)
}

#[test]
fn begin_requires_explicit_load() {
    let (mut session, script) = session(&[], 0);
    assert_eq!(session.state(), WhisperSessionState::Unloaded);
    assert_eq!(session.begin(JA), Err(SpeechProviderError::Unavailable));
    assert_eq!(script.borrow().live_engines, 0);
    session.load().unwrap();
    session.load().unwrap();
    assert_eq!(session.stats().loads, 1, "load is idempotent");
    assert_eq!(script.borrow().live_engines, 1);
}

#[test]
fn consecutive_utterances_reuse_one_context_and_reset_buffers() {
    let (mut session, script) = session(
        &[
            Ok("  一つ目 ".as_bytes()),
            Ok("二つ目".as_bytes()),
            Ok("三つ目".as_bytes()),
        ],
        0,
    );
    session.load().unwrap();
    let mut out = [0_u8; 1024];
    for (samples, expected) in [(16_000, "一つ目"), (3_000, "二つ目"), (40_000, "三つ目")]
    {
        let written = utterance(&mut session, samples, &mut out).unwrap();
        assert_eq!(core::str::from_utf8(&out[..written]).unwrap(), expected);
        assert!(out[written..].iter().all(|byte| *byte == 0));
        assert_eq!(session.state(), WhisperSessionState::Idle);
        assert_eq!(session.buffered_samples(), 0);
    }
    assert_eq!(script.borrow().seen_samples, vec![16_000, 3_000, 40_000]);
    assert_eq!(
        script.borrow().seen_languages,
        vec![WhisperLanguage::Japanese; 3]
    );
    let stats = session.stats();
    assert_eq!((stats.loads, stats.utterances_completed), (1, 3));
    assert!(session.retained_sample_capacity() <= MAX_SPEECH_UTTERANCE_BYTES / 2);
}

#[test]
fn cancel_discards_pcm_and_next_utterance_is_independent() {
    let (mut session, script) = session(&[Ok("次".as_bytes())], 0);
    session.load().unwrap();
    session.begin(JA).unwrap();
    session
        .push_pcm(PcmFormat::mono_16khz(), &[1; 4096])
        .unwrap();
    assert_eq!(session.buffered_samples(), 2048);
    session.cancel();
    assert_eq!(session.state(), WhisperSessionState::Idle);
    assert_eq!(session.buffered_samples(), 0);
    let mut out = [0_u8; 64];
    let written = utterance(&mut session, 1_000, &mut out).unwrap();
    assert_eq!(&out[..written], "次".as_bytes());
    assert_eq!(
        script.borrow().seen_samples,
        vec![1_000],
        "cancelled PCM never reached the engine"
    );
    assert_eq!(session.stats().utterances_cancelled, 1);
}

#[test]
fn scripted_same_session_unload_releases_context_and_reload_resumes() {
    let (mut session, script) = session(&[Ok(b"a"), Ok(b"b")], 0);
    session.load().unwrap();
    let mut out = [0_u8; 16];
    utterance(&mut session, 100, &mut out).unwrap();
    session.begin(JA).unwrap();
    session.push_pcm(PcmFormat::mono_16khz(), &[0; 64]).unwrap();
    session.unload();
    assert_eq!(script.borrow().live_engines, 0);
    assert_eq!(session.state(), WhisperSessionState::Unloaded);
    assert_eq!(session.buffered_samples(), 0);
    assert_eq!(session.begin(JA), Err(SpeechProviderError::Unavailable));
    session.load().unwrap();
    assert_eq!(utterance(&mut session, 100, &mut out), Ok(1));
    assert_eq!(&out[..1], b"b");
    let stats = session.stats();
    assert_eq!((stats.loads, stats.unloads), (2, 1));
}

#[test]
fn load_failure_is_reported_and_retry_succeeds() {
    let (mut session, script) = session(&[Ok(b"ok")], 1);
    assert_eq!(session.load(), Err(SpeechProviderError::Unavailable));
    assert_eq!(session.state(), WhisperSessionState::Unloaded);
    session.load().unwrap();
    let mut out = [0_u8; 8];
    assert_eq!(utterance(&mut session, 10, &mut out), Ok(2));
    assert_eq!(session.stats().load_failures, 1);
    assert_eq!(script.borrow().live_engines, 1);
}

#[test]
fn malformed_pcm_fails_only_that_utterance() {
    let (mut session, _script) = session(&[Ok(b"fine")], 0);
    session.load().unwrap();
    session.begin(JA).unwrap();
    session.push_pcm(PcmFormat::mono_16khz(), &[0; 10]).unwrap();
    assert_eq!(
        session.push_pcm(PcmFormat::mono_16khz(), &[0; 3]),
        Err(SpeechProviderError::Failed)
    );
    assert_eq!(session.state(), WhisperSessionState::Idle);
    assert_eq!(session.buffered_samples(), 0);
    assert_eq!(
        session.push_pcm(PcmFormat::mono_16khz(), &[0; 2]),
        Err(SpeechProviderError::Failed)
    );
    let mut out = [0_u8; 8];
    assert_eq!(utterance(&mut session, 10, &mut out), Ok(4));
    assert_eq!(session.stats().utterances_failed, 1);
    assert_eq!(session.stats().consecutive_engine_failures, 0);
}

#[test]
fn oversize_utterance_is_rejected_without_engine_call() {
    let (mut session, script) = session(&[], 0);
    session.load().unwrap();
    session.begin(JA).unwrap();
    let chunk = [0_u8; MAX_SPEECH_PCM_CHUNK_BYTES];
    let mut result = Ok(());
    for _ in 0..=(MAX_SPEECH_UTTERANCE_BYTES / MAX_SPEECH_PCM_CHUNK_BYTES) {
        result = session.push_pcm(PcmFormat::mono_16khz(), &chunk);
        if result.is_err() {
            break;
        }
    }
    assert_eq!(result, Err(SpeechProviderError::Failed));
    assert_eq!(session.buffered_samples(), 0);
    assert!(script.borrow().seen_samples.is_empty());
}

#[test]
fn output_too_small_keeps_context_and_next_utterance_succeeds() {
    let (mut session, script) =
        session(&[Ok("長い文字起こし".as_bytes()), Ok("短い".as_bytes())], 0);
    session.load().unwrap();
    let mut small = [0xAA_u8; 4];
    assert_eq!(
        utterance(&mut session, 100, &mut small),
        Err(SpeechProviderError::OutputTooSmall)
    );
    assert_eq!(small, [0; 4], "partial output is cleared");
    assert!(session.is_loaded());
    assert_eq!(session.stats().consecutive_engine_failures, 0);
    let mut out = [0_u8; 32];
    let written = utterance(&mut session, 100, &mut out).unwrap();
    assert_eq!(&out[..written], "短い".as_bytes());
    assert_eq!(script.borrow().live_engines, 1);
}

#[test]
fn scripted_same_session_repeated_engine_failures_release_context_until_reload() {
    let (mut session, script) = session(
        &[
            Err(WhisperEngineError::InferenceFailed),
            Err(WhisperEngineError::InferenceFailed),
            Ok(b"back"),
        ],
        0,
    );
    session.load().unwrap();
    let mut out = [0_u8; 16];
    assert_eq!(
        utterance(&mut session, 10, &mut out),
        Err(SpeechProviderError::Failed)
    );
    assert!(session.is_loaded(), "one failure keeps the context");
    assert_eq!(
        utterance(&mut session, 10, &mut out),
        Err(SpeechProviderError::Failed)
    );
    assert_eq!(MAX_CONSECUTIVE_ENGINE_FAILURES, 2);
    assert!(
        !session.is_loaded(),
        "second consecutive failure releases the context"
    );
    assert_eq!(script.borrow().live_engines, 0);
    assert_eq!(session.begin(JA), Err(SpeechProviderError::Unavailable));
    session.load().unwrap();
    assert_eq!(utterance(&mut session, 10, &mut out), Ok(4));
    let stats = session.stats();
    assert_eq!(
        (stats.loads, stats.unloads, stats.utterances_failed),
        (2, 1, 2)
    );
    assert_eq!(
        stats.last_engine_error,
        Some(WhisperEngineError::InferenceFailed)
    );
}

#[test]
fn empty_or_whitespace_or_invalid_utf8_transcripts_fail() {
    let (mut session, _script) = session(&[Ok(b"   "), Ok(&[0xFF, 0xFE]), Ok(b"x")], 0);
    session.load().unwrap();
    let mut out = [0_u8; 16];
    assert_eq!(
        utterance(&mut session, 10, &mut out),
        Err(SpeechProviderError::Failed)
    );
    assert_eq!(out, [0; 16]);
    assert_eq!(
        utterance(&mut session, 10, &mut out),
        Err(SpeechProviderError::Failed)
    );
    assert_eq!(out, [0; 16]);
    assert!(
        !session.is_loaded(),
        "two consecutive engine-side failures release the context"
    );
    session.load().unwrap();
    assert_eq!(utterance(&mut session, 10, &mut out), Ok(1));
}

#[test]
fn begin_rejects_other_formats_and_double_begin() {
    let (mut session, _script) = session(&[], 0);
    session.load().unwrap();
    let other = SpeechOptions {
        language: SpeechLanguage::Auto,
        pcm_format: PcmFormat::stereo_48khz(),
    };
    assert_eq!(session.begin(other), Err(SpeechProviderError::Failed));
    session.begin(JA).unwrap();
    assert_eq!(session.begin(JA), Err(SpeechProviderError::Failed));
    assert_eq!(
        session.finish(&mut [0; 8]),
        Err(SpeechProviderError::Failed),
        "no PCM"
    );
    assert_eq!(session.state(), WhisperSessionState::Idle);
}

#[test]
fn auto_language_hint_reaches_engine() {
    let (mut session, script) = session(&[Ok(b"x")], 0);
    session.load().unwrap();
    session
        .begin(SpeechOptions {
            language: SpeechLanguage::Auto,
            pcm_format: PcmFormat::mono_16khz(),
        })
        .unwrap();
    session.push_pcm(PcmFormat::mono_16khz(), &[0; 2]).unwrap();
    session.finish(&mut [0; 4]).unwrap();
    assert_eq!(script.borrow().seen_languages, vec![WhisperLanguage::Auto]);
    assert_eq!(WhisperLanguage::Auto.code(), b"auto\0");
    assert_eq!(WhisperLanguage::Japanese.code(), b"ja\0");
}

#[test]
fn trim_transcript_handles_edges() {
    let mut buffer = *b"  ab \n\0\0";
    assert_eq!(trim_transcript(&mut buffer, 6), 2);
    assert_eq!(&buffer, b"ab\0\0\0\0\0\0");
    let mut blank = *b"   ";
    assert_eq!(trim_transcript(&mut blank, 3), 0);
    let mut short = *b"xy";
    assert_eq!(trim_transcript(&mut short, 99), 2);
}

#[test]
fn engine_status_codes_match_adapter() {
    assert_eq!(WhisperEngineError::from_status(0), None);
    assert_eq!(
        WhisperEngineError::from_status(1),
        Some(WhisperEngineError::InvalidInput)
    );
    assert_eq!(
        WhisperEngineError::from_status(2),
        Some(WhisperEngineError::InferenceFailed)
    );
    assert_eq!(
        WhisperEngineError::from_status(3),
        Some(WhisperEngineError::InvalidSegment)
    );
    assert_eq!(
        WhisperEngineError::from_status(4),
        Some(WhisperEngineError::OutputTooSmall)
    );
    assert_eq!(
        WhisperEngineError::from_status(5),
        Some(WhisperEngineError::EmptyTranscript)
    );
    assert_eq!(
        WhisperEngineError::from_status(-1),
        Some(WhisperEngineError::Unknown)
    );
}

#[test]
fn cancel_only_affects_capture_not_completed_inference() {
    // Documents the limit: there is no abort path into the engine. `finish`
    // runs the engine to completion; a later `cancel` is a no-op on an idle
    // session and is not counted as a cancelled utterance.
    let (mut session, script) = session(&[Ok(b"done")], 0);
    session.load().unwrap();
    let mut out = [0_u8; 16];
    assert_eq!(utterance(&mut session, 500, &mut out), Ok(4));
    session.cancel();
    assert_eq!(script.borrow().seen_samples, vec![500]);
    assert_eq!(session.stats().utterances_cancelled, 0);
    assert_eq!(session.stats().utterances_completed, 1);
    assert_eq!(session.state(), WhisperSessionState::Idle);
}
