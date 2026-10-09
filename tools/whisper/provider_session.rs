// Nagi-owned M25 Whisper provider session.
//
// This file is the production speech-to-text lifecycle shared by the guest
// provider (`user/nagi-init/src/m25_whisper.rs`, via `#[path]`) and the host
// test crate (`tests/m25-whisper`). It contains no fixture input, no expected
// text, and no OS authority: the engine is reached only through
// `WhisperEngineLoader`, and transcript bytes are returned to the caller
// without being interpreted or executed.
//
// The session keeps the existing single-threaded design: inference runs
// synchronously inside `finish`, and the engine is used by one utterance at a
// time.

use alloc::vec::Vec;

use nagi_audio::{
    speech::{
        SpeechLanguage, SpeechOptions, SpeechProviderError, SpeechToTextProvider,
        MAX_SPEECH_PCM_CHUNK_BYTES, MAX_SPEECH_UTTERANCE_BYTES,
    },
    PcmFormat,
};

/// Largest utterance accepted by the provider, in mono 16 kHz samples.
pub const MAX_WHISPER_SAMPLES: usize = MAX_SPEECH_UTTERANCE_BYTES / 2;

/// Consecutive engine failures after which the session releases the engine.
/// The next utterance then requires an explicit `load` (a fresh context from
/// the Model Store) instead of reusing a context that keeps failing.
pub const MAX_CONSECUTIVE_ENGINE_FAILURES: u32 = 2;

/// Language hint passed to the engine. Separate from system locale.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WhisperLanguage {
    Auto,
    Japanese,
}

impl WhisperLanguage {
    /// NUL-terminated whisper.cpp language code.
    pub const fn code(self) -> &'static [u8] {
        match self {
            Self::Auto => b"auto\0",
            Self::Japanese => b"ja\0",
        }
    }
}

/// Errors reported by the engine boundary. Values mirror the C adapter's
/// return codes in `tools/whisper/nagi-provider-adapter.cpp`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WhisperEngineError {
    /// Adapter rejected its arguments (code 1).
    InvalidInput,
    /// `whisper_full` failed (code 2).
    InferenceFailed,
    /// A segment could not be read (code 3).
    InvalidSegment,
    /// The transcript does not fit the destination (code 4).
    OutputTooSmall,
    /// Inference produced no text (code 5).
    EmptyTranscript,
    /// Any other adapter status.
    Unknown,
}

impl WhisperEngineError {
    pub const fn from_status(status: i32) -> Option<Self> {
        match status {
            0 => None,
            1 => Some(Self::InvalidInput),
            2 => Some(Self::InferenceFailed),
            3 => Some(Self::InvalidSegment),
            4 => Some(Self::OutputTooSmall),
            5 => Some(Self::EmptyTranscript),
            _ => Some(Self::Unknown),
        }
    }
}

/// A loaded Whisper context. Dropping it releases the context.
pub trait WhisperEngine {
    /// Transcribes one utterance into `output`, returning the bytes written.
    fn transcribe(
        &mut self,
        samples: &[f32],
        language: WhisperLanguage,
        output: &mut [u8],
    ) -> Result<usize, WhisperEngineError>;
}

/// Creates engines from the locked model artifact. On Nagi this opens the
/// artifact through the read-only Model Store capability.
pub trait WhisperEngineLoader {
    type Engine: WhisperEngine;

    fn load(&mut self) -> Option<Self::Engine>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WhisperSessionStats {
    pub loads: u32,
    pub load_failures: u32,
    pub unloads: u32,
    pub utterances_completed: u32,
    pub utterances_cancelled: u32,
    pub utterances_failed: u32,
    pub consecutive_engine_failures: u32,
    pub last_engine_error: Option<WhisperEngineError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WhisperSessionState {
    Unloaded,
    Idle,
    Capturing,
}

pub struct WhisperSession<L: WhisperEngineLoader> {
    loader: L,
    engine: Option<L::Engine>,
    samples: Vec<f32>,
    capturing: bool,
    language: WhisperLanguage,
    stats: WhisperSessionStats,
}

impl<L: WhisperEngineLoader> WhisperSession<L> {
    /// Creates an unloaded session. No model bytes are read until `load`.
    pub const fn new(loader: L) -> Self {
        Self {
            loader,
            engine: None,
            samples: Vec::new(),
            capturing: false,
            language: WhisperLanguage::Japanese,
            stats: WhisperSessionStats {
                loads: 0,
                load_failures: 0,
                unloads: 0,
                utterances_completed: 0,
                utterances_cancelled: 0,
                utterances_failed: 0,
                consecutive_engine_failures: 0,
                last_engine_error: None,
            },
        }
    }

    /// Loads the engine if it is not loaded. Idempotent.
    pub fn load(&mut self) -> Result<(), SpeechProviderError> {
        if self.engine.is_some() {
            return Ok(());
        }
        match self.loader.load() {
            Some(engine) => {
                self.engine = Some(engine);
                self.stats.loads = self.stats.loads.saturating_add(1);
                self.stats.consecutive_engine_failures = 0;
                Ok(())
            }
            None => {
                self.stats.load_failures = self.stats.load_failures.saturating_add(1);
                Err(SpeechProviderError::Unavailable)
            }
        }
    }

    /// Cancels any utterance in progress and releases the engine context.
    pub fn unload(&mut self) {
        self.discard_utterance();
        if self.engine.take().is_some() {
            self.stats.unloads = self.stats.unloads.saturating_add(1);
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.engine.is_some()
    }

    pub fn state(&self) -> WhisperSessionState {
        match (self.engine.is_some(), self.capturing) {
            (false, _) => WhisperSessionState::Unloaded,
            (true, false) => WhisperSessionState::Idle,
            (true, true) => WhisperSessionState::Capturing,
        }
    }

    pub fn stats(&self) -> WhisperSessionStats {
        self.stats
    }

    /// Samples buffered for the current utterance (zero when idle).
    pub fn buffered_samples(&self) -> usize {
        self.samples.len()
    }

    /// Capacity retained between utterances; bounded by `MAX_WHISPER_SAMPLES`.
    pub fn retained_sample_capacity(&self) -> usize {
        self.samples.capacity()
    }

    pub fn loader(&self) -> &L {
        &self.loader
    }

    fn discard_utterance(&mut self) {
        self.samples.fill(0.0);
        self.samples.clear();
        self.capturing = false;
    }

    fn fail_utterance(&mut self) {
        self.discard_utterance();
        self.stats.utterances_failed = self.stats.utterances_failed.saturating_add(1);
    }

    fn record_engine_failure(&mut self, error: WhisperEngineError) {
        self.stats.last_engine_error = Some(error);
        // An undersized caller buffer is the caller's error, not an engine
        // fault, so it does not count towards releasing the context.
        if error == WhisperEngineError::OutputTooSmall {
            return;
        }
        self.stats.consecutive_engine_failures =
            self.stats.consecutive_engine_failures.saturating_add(1);
        if self.stats.consecutive_engine_failures >= MAX_CONSECUTIVE_ENGINE_FAILURES
            && self.engine.take().is_some()
        {
            self.stats.unloads = self.stats.unloads.saturating_add(1);
        }
    }
}

/// Removes ASCII whitespace around `output[..written]` in place, zeroes the
/// rest of `output`, and returns the trimmed length.
pub fn trim_transcript(output: &mut [u8], written: usize) -> usize {
    let written = written.min(output.len());
    let start = output[..written]
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(written);
    let end = output[..written]
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |index| index + 1);
    let length = end - start;
    output.copy_within(start..end, 0);
    output[length..].fill(0);
    length
}

impl<L: WhisperEngineLoader> SpeechToTextProvider for WhisperSession<L> {
    fn begin(&mut self, options: SpeechOptions) -> Result<(), SpeechProviderError> {
        if self.capturing || options.pcm_format != PcmFormat::mono_16khz() {
            return Err(SpeechProviderError::Failed);
        }
        if self.engine.is_none() {
            return Err(SpeechProviderError::Unavailable);
        }
        self.discard_utterance();
        self.language = match options.language {
            SpeechLanguage::Auto => WhisperLanguage::Auto,
            SpeechLanguage::Japanese => WhisperLanguage::Japanese,
        };
        self.capturing = true;
        Ok(())
    }

    fn push_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), SpeechProviderError> {
        if !self.capturing {
            return Err(SpeechProviderError::Failed);
        }
        if format != PcmFormat::mono_16khz()
            || bytes.is_empty()
            || bytes.len() > MAX_SPEECH_PCM_CHUNK_BYTES
            || !bytes.len().is_multiple_of(2)
        {
            self.fail_utterance();
            return Err(SpeechProviderError::Failed);
        }
        let Some(next_len) = self
            .samples
            .len()
            .checked_add(bytes.len() / 2)
            .filter(|length| *length <= MAX_WHISPER_SAMPLES)
        else {
            self.fail_utterance();
            return Err(SpeechProviderError::Failed);
        };
        if self
            .samples
            .try_reserve(next_len - self.samples.len())
            .is_err()
        {
            self.fail_utterance();
            return Err(SpeechProviderError::Unavailable);
        }
        for sample in bytes.chunks_exact(2) {
            self.samples
                .push(f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32768.0);
        }
        Ok(())
    }

    fn finish(&mut self, transcript: &mut [u8]) -> Result<usize, SpeechProviderError> {
        transcript.fill(0);
        if !self.capturing || self.samples.is_empty() || transcript.is_empty() {
            self.fail_utterance();
            return Err(SpeechProviderError::Failed);
        }
        let Some(engine) = self.engine.as_mut() else {
            self.fail_utterance();
            return Err(SpeechProviderError::Unavailable);
        };
        let result = engine.transcribe(&self.samples, self.language, transcript);
        self.discard_utterance();
        let written = match result {
            Ok(written) if written != 0 && written <= transcript.len() => written,
            Ok(_) => {
                transcript.fill(0);
                self.stats.utterances_failed = self.stats.utterances_failed.saturating_add(1);
                self.record_engine_failure(WhisperEngineError::EmptyTranscript);
                return Err(SpeechProviderError::Failed);
            }
            Err(error) => {
                transcript.fill(0);
                self.stats.utterances_failed = self.stats.utterances_failed.saturating_add(1);
                self.record_engine_failure(error);
                return Err(match error {
                    WhisperEngineError::OutputTooSmall => SpeechProviderError::OutputTooSmall,
                    _ => SpeechProviderError::Failed,
                });
            }
        };
        if core::str::from_utf8(&transcript[..written]).is_err() {
            transcript.fill(0);
            self.stats.utterances_failed = self.stats.utterances_failed.saturating_add(1);
            self.record_engine_failure(WhisperEngineError::InvalidSegment);
            return Err(SpeechProviderError::Failed);
        }
        let trimmed = trim_transcript(transcript, written);
        if trimmed == 0 {
            self.stats.utterances_failed = self.stats.utterances_failed.saturating_add(1);
            self.record_engine_failure(WhisperEngineError::EmptyTranscript);
            return Err(SpeechProviderError::Failed);
        }
        self.stats.consecutive_engine_failures = 0;
        self.stats.utterances_completed = self.stats.utterances_completed.saturating_add(1);
        Ok(trimmed)
    }

    fn cancel(&mut self) {
        if self.capturing {
            self.stats.utterances_cancelled = self.stats.utterances_cancelled.saturating_add(1);
        }
        self.discard_utterance();
    }
}

impl<L: WhisperEngineLoader> Drop for WhisperSession<L> {
    fn drop(&mut self) {
        self.discard_utterance();
        self.engine = None;
    }
}
