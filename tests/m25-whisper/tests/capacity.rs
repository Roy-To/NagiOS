//! Orchestration-only (scripted engine) tests of PCM buffer memory: the
//! retained sample capacity must never exceed `MAX_WHISPER_SAMPLES`, and
//! cancel/unload memory behaviour is pinned here. Same-session tests: one
//! `WhisperSession` value is reused throughout; no real engine is involved.

use std::cell::Cell;
use std::rc::Rc;

use nagi_audio::speech::{
    SpeechLanguage, SpeechOptions, SpeechProviderError, SpeechToTextProvider,
    MAX_SPEECH_PCM_CHUNK_BYTES,
};
use nagi_audio::PcmFormat;
use nagi_m25_whisper_tests::provider_session::{
    WhisperEngine, WhisperEngineError, WhisperEngineLoader, WhisperLanguage, WhisperSession,
    WhisperSessionState, MAX_WHISPER_SAMPLES,
};

struct EchoEngine(Rc<Cell<i32>>);

impl WhisperEngine for EchoEngine {
    fn transcribe(
        &mut self,
        _samples: &[f32],
        _language: WhisperLanguage,
        output: &mut [u8],
    ) -> Result<usize, WhisperEngineError> {
        output[0] = b'x';
        Ok(1)
    }
}

impl Drop for EchoEngine {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}

struct EchoLoader(Rc<Cell<i32>>);

impl WhisperEngineLoader for EchoLoader {
    type Engine = EchoEngine;
    fn load(&mut self) -> Option<EchoEngine> {
        self.0.set(self.0.get() + 1);
        Some(EchoEngine(self.0.clone()))
    }
}

const JA: SpeechOptions = SpeechOptions {
    language: SpeechLanguage::Japanese,
    pcm_format: PcmFormat::mono_16khz(),
};

fn loaded() -> (WhisperSession<EchoLoader>, Rc<Cell<i32>>) {
    let live = Rc::new(Cell::new(0));
    let mut session = WhisperSession::new(EchoLoader(live.clone()));
    session.load().unwrap();
    (session, live)
}

/// Pushes `chunks` (byte lengths) and returns the capacity after each push.
fn push_all(session: &mut WhisperSession<EchoLoader>, chunks: &[usize]) -> Vec<usize> {
    let pcm = [0x22_u8; MAX_SPEECH_PCM_CHUNK_BYTES];
    chunks
        .iter()
        .map(|len| {
            session
                .push_pcm(PcmFormat::mono_16khz(), &pcm[..*len])
                .unwrap();
            assert!(
                session.retained_sample_capacity() <= MAX_WHISPER_SAMPLES,
                "capacity {} > MAX_WHISPER_SAMPLES {} after {} samples",
                session.retained_sample_capacity(),
                MAX_WHISPER_SAMPLES,
                session.buffered_samples()
            );
            session.retained_sample_capacity()
        })
        .collect()
}

#[test]
fn irregular_chunks_up_to_the_limit_never_overallocate() {
    // 256 x 4094-byte chunks (524,032 samples) then one 512-byte chunk
    // reaches exactly MAX_WHISPER_SAMPLES (524,288).
    let (mut session, _live) = loaded();
    session.begin(JA).unwrap();
    let mut chunks = vec![4094; 256];
    chunks.push(512);
    let capacities = push_all(&mut session, &chunks);
    assert_eq!(session.buffered_samples(), MAX_WHISPER_SAMPLES);
    assert_eq!(*capacities.last().unwrap(), MAX_WHISPER_SAMPLES);
    // One more sample is rejected and the utterance is discarded.
    assert_eq!(
        session.push_pcm(PcmFormat::mono_16khz(), &[0, 0]),
        Err(SpeechProviderError::Failed)
    );
    assert_eq!(session.buffered_samples(), 0);
}

#[test]
fn odd_sized_chunk_mix_stays_bounded() {
    let (mut session, _live) = loaded();
    session.begin(JA).unwrap();
    let pattern = [2, 4094, 1000, 3, 4096, 2048, 6, 4092];
    let mut chunks = Vec::new();
    let mut total = 0;
    'fill: loop {
        for len in pattern {
            let len = len & !1;
            if len == 0 {
                continue;
            }
            if total + len > MAX_WHISPER_SAMPLES * 2 {
                break 'fill;
            }
            total += len;
            chunks.push(len);
        }
    }
    let remainder = MAX_WHISPER_SAMPLES * 2 - total;
    if remainder > 0 {
        chunks.push(remainder);
    }
    push_all(&mut session, &chunks);
    assert_eq!(session.buffered_samples(), MAX_WHISPER_SAMPLES);
}

#[test]
fn cancel_and_finish_retain_the_zeroed_buffer_for_reuse() {
    let (mut session, _live) = loaded();
    session.begin(JA).unwrap();
    push_all(&mut session, &[4096; 16]);
    let retained = session.retained_sample_capacity();
    assert!(retained >= 16 * 2048);
    session.cancel();
    assert_eq!(session.buffered_samples(), 0);
    assert_eq!(
        session.retained_sample_capacity(),
        retained,
        "cancel keeps the (zeroed) allocation for the next utterance"
    );
    session.begin(JA).unwrap();
    push_all(&mut session, &[4096; 4]);
    let mut out = [0_u8; 8];
    assert_eq!(session.finish(&mut out), Ok(1));
    assert_eq!(session.retained_sample_capacity(), retained);
    assert_eq!(session.state(), WhisperSessionState::Idle);
}

#[test]
fn unload_frees_the_pcm_buffer_and_the_context() {
    let (mut session, live) = loaded();
    session.begin(JA).unwrap();
    push_all(&mut session, &[4096; 16]);
    session.unload();
    assert_eq!(live.get(), 0, "engine context dropped");
    assert_eq!(session.buffered_samples(), 0);
    assert_eq!(
        session.retained_sample_capacity(),
        0,
        "unload releases the PCM allocation"
    );
    session.load().unwrap();
    session.begin(JA).unwrap();
    push_all(&mut session, &[4096]);
    assert_eq!(session.retained_sample_capacity(), 2048);
}
