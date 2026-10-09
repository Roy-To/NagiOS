//! Local text-to-speech provider for the Nagi M25 speech contract.
//!
//! [`LocalTtsProvider`] implements [`nagi_audio::speech::TextToSpeechProvider`]
//! on top of a replaceable [`SynthesisBackend`]. It owns everything the
//! contract requires of a provider and nothing more:
//!
//! - bounded input (at most [`MAX_SPEECH_SYNTHESIS_TEXT_BYTES`] of UTF-8),
//! - bounded output (at most [`MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES`] of PCM per
//!   utterance; exceeding it fails closed instead of truncating),
//! - whole-frame signed 16-bit little-endian PCM in caller-owned chunks,
//!   either stereo 48 kHz (the `AudioServicePlaybackSink` format) or mono
//!   16 kHz (through the shared `Stereo48KhzToMono16Khz` decimator),
//! - cancellation and failure cleanup that erase all per-utterance state,
//! - reuse of the loaded backend across utterances, and explicit
//!   unload/missing-model reporting.
//!
//! The provider receives no OS capability. The concrete Japanese engine
//! (jpreprocess + jbonsai, feature `engine-jbonsai`) lives in
//! [`jbonsai_backend`]; the rest of this crate is `no_std` + `alloc`.

#![cfg_attr(not(feature = "engine-jbonsai"), no_std)]

extern crate alloc;

#[cfg(feature = "engine-jbonsai")]
pub mod jbonsai_backend;

use alloc::vec::Vec;

use nagi_audio::speech::{
    SpeechProviderError, SpeechSynthesisLanguage, SpeechSynthesisOptions, Stereo48KhzToMono16Khz,
    SynthesisPcmChunk, TextToSpeechProvider, MAX_SPEECH_PCM_CHUNK_BYTES,
    MAX_SPEECH_SYNTHESIS_TEXT_BYTES, MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES,
};
use nagi_audio::PcmFormat;

/// The only engine sample rate the provider accepts. Playback is stereo
/// 48 kHz; 16 kHz mono is derived by decimation.
pub const ENGINE_SAMPLE_RATE: u32 = 48_000;

/// Upper bound on the number of engine samples a backend may produce per
/// frame. HTS voices use 240 (5 ms at 48 kHz). The bound keeps the provider's
/// staging buffer fixed and small.
pub const MAX_ENGINE_FRAME_SAMPLES: usize = 1_024;

/// Size of the provider's staging buffer: one maximal engine frame rendered as
/// stereo S16LE.
const STAGING_BYTES: usize = MAX_ENGINE_FRAME_SAMPLES * 4;

/// Failure reported by a [`SynthesisBackend`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendError {
    /// The model/voice is not loaded (missing, unloaded under memory
    /// pressure, or never configured).
    Unavailable,
    /// The language cannot be spoken by the loaded voice.
    UnsupportedLanguage,
    /// Text analysis or acoustic synthesis failed.
    Failed,
}

/// One utterance in progress inside a backend. Frames are pulled one at a
/// time so the provider never buffers a whole utterance.
pub trait FrameSource {
    /// Writes the next frame (exactly `frame_len()` samples of the backend)
    /// into `out` and returns the number of samples written, or `Ok(0)` when
    /// the utterance is complete.
    fn next_frame(&mut self, out: &mut [f64]) -> Result<usize, BackendError>;
}

/// A replaceable local synthesis engine. Implementations hold their loaded
/// model across utterances; per-utterance state lives in the returned
/// [`FrameSource`].
pub trait SynthesisBackend {
    type Source: FrameSource;

    /// Sample rate of produced frames. Must equal [`ENGINE_SAMPLE_RATE`].
    fn sample_rate(&self) -> u32;
    /// Samples produced per frame; at most [`MAX_ENGINE_FRAME_SAMPLES`].
    fn frame_len(&self) -> usize;
    /// Whether a model is currently loaded.
    fn is_loaded(&self) -> bool;
    /// Analyses `text` and prepares synthesis. `Ok(None)` means the text
    /// contains nothing speakable (for example only punctuation).
    fn start(
        &mut self,
        text: &str,
        language: SpeechSynthesisLanguage,
    ) -> Result<Option<Self::Source>, BackendError>;
}

/// Edge-silence trimming parameters, in engine frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SilenceTrim {
    /// Frames whose RMS (on the 16-bit sample scale) is below this are silent.
    pub rms_threshold: f64,
    /// Silent frames kept immediately before the first and after the last
    /// voiced frame, so onsets and releases are not clipped.
    pub pad_frames: usize,
    /// Longest silent run held back while waiting to see whether speech
    /// resumes. Longer runs are emitted (internal pauses are preserved).
    pub max_hold_frames: usize,
}

impl SilenceTrim {
    /// 50 ms padding and 600 ms hold for 5 ms HTS frames at 48 kHz.
    pub const HTS_48K: Self = Self {
        rms_threshold: 64.0,
        pad_frames: 10,
        max_hold_frames: 120,
    };

    /// Pass every frame through unchanged.
    pub const DISABLED: Self = Self {
        rms_threshold: 0.0,
        pad_frames: 0,
        max_hold_frames: 0,
    };
}

/// Removes leading and trailing silence from a [`FrameSource`] while keeping
/// internal pauses. Memory is bounded by `max_hold_frames` frames.
pub struct SilenceTrimmer<S> {
    inner: S,
    config: SilenceTrim,
    frame_len: usize,
    /// Held silent frames, oldest first, stored contiguously.
    held: alloc::collections::VecDeque<Vec<f64>>,
    spare: Vec<Vec<f64>>,
    voiced_seen: bool,
    inner_done: bool,
    /// Frames to emit from `held` before pulling more (after speech resumed
    /// or at the end of the utterance).
    flush_remaining: usize,
}

impl<S: FrameSource> SilenceTrimmer<S> {
    pub fn new(inner: S, frame_len: usize, config: SilenceTrim) -> Self {
        Self {
            inner,
            config,
            frame_len,
            held: alloc::collections::VecDeque::new(),
            spare: Vec::new(),
            voiced_seen: false,
            inner_done: false,
            flush_remaining: 0,
        }
    }

    fn is_silent(&self, frame: &[f64]) -> bool {
        let energy: f64 = frame.iter().map(|sample| sample * sample).sum();
        let rms = libm_sqrt(energy / frame.len().max(1) as f64);
        rms < self.config.rms_threshold
    }

    fn take_buffer(&mut self) -> Vec<f64> {
        self.spare
            .pop()
            .unwrap_or_else(|| alloc::vec![0.0; self.frame_len])
    }

    fn recycle(&mut self, mut buffer: Vec<f64>) {
        buffer.fill(0.0);
        self.spare.push(buffer);
    }

    fn emit_held(&mut self, out: &mut [f64]) -> usize {
        let frame = self.held.pop_front().expect("held frame");
        out[..self.frame_len].copy_from_slice(&frame);
        self.recycle(frame);
        self.flush_remaining -= 1;
        self.frame_len
    }
}

/// `f64::sqrt` lives in `std`; this crate's core is `no_std`.
fn libm_sqrt(value: f64) -> f64 {
    if value <= 0.0 {
        return 0.0;
    }
    // Newton iteration from a bit-level initial guess; converges to full
    // double precision well within 8 steps for the RMS range used here.
    let mut x = f64::from_bits((value.to_bits() >> 1) + (1023u64 << 51));
    for _ in 0..8 {
        x = 0.5 * (x + value / x);
    }
    x
}

impl<S: FrameSource> FrameSource for SilenceTrimmer<S> {
    fn next_frame(&mut self, out: &mut [f64]) -> Result<usize, BackendError> {
        if out.len() < self.frame_len {
            return Err(BackendError::Failed);
        }
        loop {
            if self.flush_remaining > 0 {
                return Ok(self.emit_held(out));
            }
            if self.inner_done {
                return Ok(0);
            }
            let mut frame = self.take_buffer();
            let produced = match self.inner.next_frame(&mut frame) {
                Ok(produced) => produced,
                Err(error) => {
                    self.recycle(frame);
                    return Err(error);
                }
            };
            if produced == 0 {
                self.recycle(frame);
                self.inner_done = true;
                // Trailing silence: keep only the padding.
                let keep = if self.voiced_seen {
                    self.held.len().min(self.config.pad_frames)
                } else {
                    0
                };
                while self.held.len() > keep {
                    let dropped = self.held.pop_back().expect("held frame");
                    self.recycle(dropped);
                }
                self.flush_remaining = self.held.len();
                continue;
            }
            if produced != self.frame_len {
                self.recycle(frame);
                return Err(BackendError::Failed);
            }
            if self.is_silent(&frame) {
                if !self.voiced_seen {
                    // Leading silence: keep only the last `pad_frames`.
                    self.held.push_back(frame);
                    while self.held.len() > self.config.pad_frames {
                        let dropped = self.held.pop_front().expect("held frame");
                        self.recycle(dropped);
                    }
                    continue;
                }
                self.held.push_back(frame);
                if self.held.len() > self.config.max_hold_frames {
                    // A long internal pause: release the oldest held frame.
                    self.flush_remaining = 1;
                }
                continue;
            }
            // Voiced frame: release everything held (padding or an internal
            // pause), then this frame.
            self.voiced_seen = true;
            self.held.push_back(frame);
            self.flush_remaining = self.held.len();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputMode {
    Stereo48k,
    Mono16k,
}

impl OutputMode {
    fn from_format(format: PcmFormat) -> Option<Self> {
        if format == PcmFormat::stereo_48khz() {
            Some(Self::Stereo48k)
        } else if format == PcmFormat::mono_16khz() {
            Some(Self::Mono16k)
        } else {
            None
        }
    }

    const fn frame_bytes(self) -> usize {
        match self {
            Self::Stereo48k => 4,
            Self::Mono16k => 2,
        }
    }
}

/// Observable per-provider counters, for tests and diagnostics. They carry no
/// utterance content.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProviderStats {
    /// Utterances that reached `End` with audio.
    pub completed_utterances: u32,
    /// Utterances that were cancelled or failed after `begin`.
    pub aborted_utterances: u32,
    /// PCM bytes emitted in the current utterance.
    pub current_utterance_bytes: usize,
}

/// Streaming TTS provider over a [`SynthesisBackend`].
pub struct LocalTtsProvider<B: SynthesisBackend> {
    backend: B,
    source: Option<B::Source>,
    mode: OutputMode,
    active: bool,
    source_done: bool,
    tail_flushed: bool,
    frame: Vec<f64>,
    staging: Vec<u8>,
    staged_start: usize,
    staged_end: usize,
    decimator: Stereo48KhzToMono16Khz,
    emitted: usize,
    stats: ProviderStats,
}

impl<B: SynthesisBackend> LocalTtsProvider<B> {
    /// Wraps a backend. Fails if the backend's frame shape is outside the
    /// provider's fixed bounds.
    pub fn new(backend: B) -> Result<Self, BackendError> {
        let frame_len = backend.frame_len();
        if frame_len == 0 || frame_len > MAX_ENGINE_FRAME_SAMPLES {
            return Err(BackendError::Failed);
        }
        if backend.is_loaded() && backend.sample_rate() != ENGINE_SAMPLE_RATE {
            return Err(BackendError::Failed);
        }
        Ok(Self {
            backend,
            source: None,
            mode: OutputMode::Stereo48k,
            active: false,
            source_done: false,
            tail_flushed: false,
            frame: alloc::vec![0.0; frame_len],
            staging: alloc::vec![0; STAGING_BYTES],
            staged_start: 0,
            staged_end: 0,
            decimator: Stereo48KhzToMono16Khz::new(),
            emitted: 0,
            stats: ProviderStats::default(),
        })
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    pub fn stats(&self) -> ProviderStats {
        self.stats
    }

    /// True while an utterance is in progress.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// True when no per-utterance audio or engine state is retained.
    pub fn is_clear(&self) -> bool {
        !self.active
            && self.source.is_none()
            && self.staged_start == self.staged_end
            && self.emitted == 0
            && self.staging.iter().all(|byte| *byte == 0)
            && self.frame.iter().all(|sample| *sample == 0.0)
    }

    fn reset_utterance(&mut self) {
        self.source = None;
        self.active = false;
        self.source_done = false;
        self.tail_flushed = false;
        self.staging.fill(0);
        self.frame.fill(0.0);
        self.staged_start = 0;
        self.staged_end = 0;
        self.decimator.reset();
        self.emitted = 0;
        self.stats.current_utterance_bytes = 0;
    }

    fn fail(&mut self, error: SpeechProviderError) -> SpeechProviderError {
        if self.active {
            self.stats.aborted_utterances = self.stats.aborted_utterances.saturating_add(1);
        }
        self.reset_utterance();
        error
    }

    /// Renders one engine frame into the staging buffer in the requested
    /// output format. Returns false when the source is exhausted.
    fn stage_next_frame(&mut self) -> Result<bool, SpeechProviderError> {
        debug_assert_eq!(self.staged_start, self.staged_end);
        self.staged_start = 0;
        self.staged_end = 0;
        if self.source_done {
            if self.mode == OutputMode::Mono16k && !self.tail_flushed {
                self.tail_flushed = true;
                let samples = self
                    .decimator
                    .finish(&mut self.staging)
                    .map_err(|_| SpeechProviderError::Failed)?;
                self.staged_end = samples * 2;
                return Ok(self.staged_end != 0);
            }
            return Ok(false);
        }
        let Some(source) = self.source.as_mut() else {
            return Err(SpeechProviderError::Failed);
        };
        self.frame.fill(0.0);
        let produced = source
            .next_frame(&mut self.frame)
            .map_err(map_backend_error)?;
        if produced == 0 {
            self.source_done = true;
            self.source = None;
            return self.stage_next_frame();
        }
        if produced > self.frame.len() {
            return Err(SpeechProviderError::Failed);
        }
        let mut stereo_len = 0;
        for sample in &self.frame[..produced] {
            if !sample.is_finite() {
                return Err(SpeechProviderError::Failed);
            }
            let value = to_i16(*sample).to_le_bytes();
            self.staging[stereo_len..stereo_len + 2].copy_from_slice(&value);
            self.staging[stereo_len + 2..stereo_len + 4].copy_from_slice(&value);
            stereo_len += 4;
        }
        self.frame.fill(0.0);
        match self.mode {
            OutputMode::Stereo48k => self.staged_end = stereo_len,
            OutputMode::Mono16k => {
                // Decimate in place: the mono output is always shorter than
                // the stereo input it is produced from, so copy the stereo
                // frame out first to keep the conversion non-aliasing.
                let mut stereo = [0u8; STAGING_BYTES];
                stereo[..stereo_len].copy_from_slice(&self.staging[..stereo_len]);
                self.staging.fill(0);
                let samples = self
                    .decimator
                    .convert(&stereo[..stereo_len], &mut self.staging)
                    .map_err(|_| SpeechProviderError::Failed)?;
                stereo.fill(0);
                self.staged_end = samples * 2;
            }
        }
        Ok(true)
    }
}

fn to_i16(sample: f64) -> i16 {
    let rounded = if sample >= 0.0 {
        sample + 0.5
    } else {
        sample - 0.5
    };
    if rounded >= i16::MAX as f64 {
        i16::MAX
    } else if rounded <= i16::MIN as f64 {
        i16::MIN
    } else {
        rounded as i16
    }
}

fn map_backend_error(error: BackendError) -> SpeechProviderError {
    match error {
        BackendError::Unavailable | BackendError::UnsupportedLanguage => {
            SpeechProviderError::Unavailable
        }
        BackendError::Failed => SpeechProviderError::Failed,
    }
}

impl<B: SynthesisBackend> TextToSpeechProvider for LocalTtsProvider<B> {
    fn begin(
        &mut self,
        text: &str,
        options: SpeechSynthesisOptions,
    ) -> Result<(), SpeechProviderError> {
        if self.active {
            // A provider serves one utterance at a time; a second begin
            // without cancel/End is a caller error and aborts the first.
            return Err(self.fail(SpeechProviderError::Failed));
        }
        self.reset_utterance();
        if text.is_empty() || text.len() > MAX_SPEECH_SYNTHESIS_TEXT_BYTES {
            return Err(SpeechProviderError::Failed);
        }
        let Some(mode) = OutputMode::from_format(options.pcm_format) else {
            return Err(SpeechProviderError::Failed);
        };
        if !self.backend.is_loaded() {
            return Err(SpeechProviderError::Unavailable);
        }
        if self.backend.sample_rate() != ENGINE_SAMPLE_RATE
            || self.backend.frame_len() != self.frame.len()
        {
            return Err(SpeechProviderError::Failed);
        }
        self.mode = mode;
        let source = self
            .backend
            .start(text, options.language)
            .map_err(map_backend_error)?;
        self.active = true;
        match source {
            Some(source) => self.source = Some(source),
            None => self.source_done = true,
        }
        // Nothing speakable is reported as an empty utterance; the synthesis
        // service turns that into `NoAudioProduced`.
        self.tail_flushed = self.source.is_none();
        Ok(())
    }

    fn next_pcm_chunk(
        &mut self,
        destination: &mut [u8],
    ) -> Result<SynthesisPcmChunk, SpeechProviderError> {
        if !self.active {
            return Err(SpeechProviderError::Failed);
        }
        let frame_bytes = self.mode.frame_bytes();
        let capacity = destination.len().min(MAX_SPEECH_PCM_CHUNK_BYTES);
        let capacity = capacity - capacity % frame_bytes;
        if capacity == 0 {
            return Err(self.fail(SpeechProviderError::OutputTooSmall));
        }
        let mut written = 0;
        while written < capacity {
            if self.staged_start == self.staged_end {
                match self.stage_next_frame() {
                    Ok(true) => continue,
                    Ok(false) => break,
                    Err(error) => {
                        destination.fill(0);
                        return Err(self.fail(error));
                    }
                }
            }
            let take = (self.staged_end - self.staged_start).min(capacity - written);
            destination[written..written + take]
                .copy_from_slice(&self.staging[self.staged_start..self.staged_start + take]);
            self.staging[self.staged_start..self.staged_start + take].fill(0);
            self.staged_start += take;
            written += take;
        }
        if written == 0 {
            if self.emitted != 0 {
                self.stats.completed_utterances = self.stats.completed_utterances.saturating_add(1);
            }
            self.reset_utterance();
            return Ok(SynthesisPcmChunk::End);
        }
        let Some(total) = self.emitted.checked_add(written) else {
            destination.fill(0);
            return Err(self.fail(SpeechProviderError::Failed));
        };
        if total > MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES {
            // Fail closed: never emit a truncated utterance as success.
            destination.fill(0);
            return Err(self.fail(SpeechProviderError::Failed));
        }
        self.emitted = total;
        self.stats.current_utterance_bytes = total;
        Ok(SynthesisPcmChunk::Data(written))
    }

    fn cancel(&mut self) {
        if self.active {
            self.stats.aborted_utterances = self.stats.aborted_utterances.saturating_add(1);
        }
        self.reset_utterance();
    }
}

#[cfg(test)]
mod tests;
