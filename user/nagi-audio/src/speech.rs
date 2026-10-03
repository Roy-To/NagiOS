//! Permission-bounded push-to-talk orchestration.
//!
//! This module accepts audio only through a service-owned capture source and
//! sends PCM only to a speech provider. It never returns raw PCM to an app.
//! The permission, indicator, and provider implementations are supplied by
//! trusted system services; this crate does not grant microphone authority.

#[cfg(target_os = "nagi")]
use crate::AudioService;
use crate::PcmFormat;

pub const MAX_SPEECH_PCM_CHUNK_BYTES: usize = 4096;
pub const MAX_SPEECH_UTTERANCE_BYTES: usize = 1_048_576;
pub const MAX_SPEECH_TRANSCRIPT_BYTES: usize = 1024;
pub const MAX_SPEECH_SYNTHESIS_TEXT_BYTES: usize = 1024;
pub const MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES: usize = 1_048_576;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechConsumer {
    Albert,
    NagiBar,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechLanguage {
    Auto,
    Japanese,
}

/// Language hint for a spoken output utterance. This is independent of the
/// system display locale and Albert conversation-language preference.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechSynthesisLanguage {
    Auto,
    English,
    Japanese,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeechOptions {
    pub language: SpeechLanguage,
    pub pcm_format: PcmFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeechSynthesisOptions {
    pub language: SpeechSynthesisLanguage,
    pub pcm_format: PcmFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechPermissionDecision {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechProviderError {
    Unavailable,
    Failed,
    OutputTooSmall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndicatorError {
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureSourceError {
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechError {
    PermissionRequired,
    PermissionDenied,
    AlreadyActive,
    NotActive,
    IndicatorUnavailable,
    CaptureFailed,
    InvalidPcmData,
    UtteranceTooLong,
    ProviderUnavailable,
    ProviderFailed,
    OutputTooSmall,
    EmptyTranscript,
    InvalidTranscript,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpeechSynthesisError {
    EmptyText,
    TextTooLong,
    InvalidText,
    ProviderUnavailable,
    ProviderFailed,
    ProviderOutputTooSmall,
    NoAudioProduced,
    InvalidPcmData,
    UtteranceTooLong,
    PlaybackUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaybackError {
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SynthesisPcmChunk {
    Data(usize),
    End,
}

/// The trusted policy adapter must check the authenticated session, trusted
/// foreground consumer, and explicit user action that invoked push-to-talk.
/// Only the system-owned input path (for example, Super+V) should call begin.
pub trait SpeechPermissionAuthority {
    fn authorize_push_to_talk(&mut self, consumer: SpeechConsumer) -> SpeechPermissionDecision;
}

/// The system-owned indicator must name the selected consumer and remain
/// visible for the full interval in which microphone capture can occur.
pub trait MicrophoneActivityIndicator {
    fn show(&mut self, consumer: SpeechConsumer) -> Result<(), IndicatorError>;
    /// Clears visible or partially initialized state; safe after `show` fails.
    fn hide(&mut self, consumer: SpeechConsumer);
}

/// A service-owned source. Implementations must not expose its buffer to apps.
pub trait PcmCaptureSource {
    fn pcm_format(&self) -> PcmFormat;
    fn capture_pcm(&mut self, destination: &mut [u8]) -> Result<usize, CaptureSourceError>;
}

/// Provider implementations receive bounded signed 16-bit little-endian PCM
/// and return bounded UTF-8 text. They receive no OS capability and must not
/// execute the transcript.
pub trait SpeechToTextProvider {
    fn begin(&mut self, options: SpeechOptions) -> Result<(), SpeechProviderError>;
    fn push_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), SpeechProviderError>;
    fn finish(&mut self, transcript: &mut [u8]) -> Result<usize, SpeechProviderError>;
    fn cancel(&mut self);
}

const SPEECH_RESAMPLER_TAPS: usize = 127;
const SPEECH_RESAMPLER_TAIL_FRAMES: usize = SPEECH_RESAMPLER_TAPS - 1;
const MAX_SPEECH_RESAMPLED_CHUNK_BYTES: usize = (MAX_SPEECH_PCM_CHUNK_BYTES / 4).div_ceil(3) * 2;

// 127-tap Blackman-windowed low-pass FIR, cutoff 6.72 kHz at 48 kHz, Q15.
// The coefficients are symmetric and sum exactly to 32768.
const SPEECH_RESAMPLER_COEFFICIENTS_Q15: [i32; SPEECH_RESAMPLER_TAPS] = [
    0, 0, 0, 0, 1, 1, 0, -2, -3, -1, 2, 6, 6, 0, -8, -13, -7, 7, 20, 21, 4, -22, -37, -25, 12, 49,
    55, 17, -45, -86, -66, 13, 100, 125, 54, -77, -174, -151, 0, 182, 255, 137, -115, -327, -318,
    -52, 311, 499, 324, -151, -610, -678, -208, 553, 1065, 836, -178, -1415, -1934, -944, 1660,
    5102, 8029, 9170, 8029, 5102, 1660, -944, -1934, -1415, -178, 836, 1065, 553, -208, -678, -610,
    -151, 324, 499, 311, -52, -318, -327, -115, 137, 255, 182, 0, -151, -174, -77, 54, 125, 100,
    13, -66, -86, -45, 17, 55, 49, 12, -25, -37, -22, 4, 21, 20, 7, -7, -13, -8, 0, 6, 6, 2, -1,
    -3, -2, 0, 1, 1, 0, 0, 0, 0,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PcmResampleError {
    InvalidInput,
    OutputTooSmall,
}

/// Fixed-memory streaming decimator for interleaved stereo S16LE at 48 kHz.
/// It averages channels, applies an anti-alias FIR, and emits mono S16LE at
/// 16 kHz. State is preserved across `convert` calls and cleared by `finish`
/// or `reset`.
pub struct Stereo48KhzToMono16Khz {
    history: [i16; SPEECH_RESAMPLER_TAPS],
    write_index: usize,
    phase: usize,
    has_input: bool,
}

impl Stereo48KhzToMono16Khz {
    pub const fn new() -> Self {
        Self {
            history: [0; SPEECH_RESAMPLER_TAPS],
            write_index: 0,
            phase: 0,
            has_input: false,
        }
    }

    pub fn reset(&mut self) {
        self.history.fill(0);
        self.write_index = 0;
        self.phase = 0;
        self.has_input = false;
    }

    /// Converts complete stereo frames into mono frames. Invalid input or an
    /// undersized destination is rejected before resampler state is changed.
    pub fn convert(
        &mut self,
        input_s16le_stereo: &[u8],
        output_s16le_mono: &mut [u8],
    ) -> Result<usize, PcmResampleError> {
        if input_s16le_stereo.len() & 3 != 0 {
            return Err(PcmResampleError::InvalidInput);
        }
        let frames = input_s16le_stereo.len() / 4;
        let required_samples = Self::output_count(frames, self.phase);
        let required_bytes = required_samples
            .checked_mul(2)
            .ok_or(PcmResampleError::OutputTooSmall)?;
        if output_s16le_mono.len() < required_bytes {
            return Err(PcmResampleError::OutputTooSmall);
        }

        let mut output_offset = 0;
        for frame in input_s16le_stereo.chunks_exact(4) {
            let left = i16::from_le_bytes([frame[0], frame[1]]);
            let right = i16::from_le_bytes([frame[2], frame[3]]);
            let mono = ((i32::from(left) + i32::from(right)) / 2) as i16;
            if let Some(sample) = self.push_mono_sample(mono) {
                let bytes = sample.to_le_bytes();
                output_s16le_mono[output_offset..output_offset + 2].copy_from_slice(&bytes);
                output_offset += 2;
            }
        }
        self.has_input |= frames != 0;
        Ok(output_offset / 2)
    }

    /// Appends the FIR tail as silence and clears all per-utterance state.
    pub fn finish(&mut self, output_s16le_mono: &mut [u8]) -> Result<usize, PcmResampleError> {
        if !self.has_input {
            self.reset();
            return Ok(0);
        }
        let required_samples = Self::output_count(SPEECH_RESAMPLER_TAIL_FRAMES, self.phase);
        let required_bytes = required_samples
            .checked_mul(2)
            .ok_or(PcmResampleError::OutputTooSmall)?;
        if output_s16le_mono.len() < required_bytes {
            return Err(PcmResampleError::OutputTooSmall);
        }

        let mut output_offset = 0;
        for _ in 0..SPEECH_RESAMPLER_TAIL_FRAMES {
            if let Some(sample) = self.push_mono_sample(0) {
                let bytes = sample.to_le_bytes();
                output_s16le_mono[output_offset..output_offset + 2].copy_from_slice(&bytes);
                output_offset += 2;
            }
        }
        self.reset();
        Ok(output_offset / 2)
    }

    fn output_count(frames: usize, phase: usize) -> usize {
        if frames == 0 {
            return 0;
        }
        let first_output = (2 + 3 - phase) % 3;
        if first_output >= frames {
            0
        } else {
            1 + (frames - 1 - first_output) / 3
        }
    }

    fn push_mono_sample(&mut self, sample: i16) -> Option<i16> {
        self.history[self.write_index] = sample;
        self.write_index = (self.write_index + 1) % SPEECH_RESAMPLER_TAPS;

        let result = if self.phase == 2 {
            let mut accumulator = 0_i64;
            for (tap, coefficient) in SPEECH_RESAMPLER_COEFFICIENTS_Q15.iter().enumerate() {
                let index =
                    (self.write_index + SPEECH_RESAMPLER_TAPS - 1 - tap) % SPEECH_RESAMPLER_TAPS;
                accumulator += i64::from(self.history[index]) * i64::from(*coefficient);
            }
            let rounded = if accumulator >= 0 {
                accumulator + (1 << 14)
            } else {
                accumulator - (1 << 14)
            };
            Some((rounded >> 15).clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16)
        } else {
            None
        };
        self.phase = (self.phase + 1) % 3;
        result
    }
}

impl Default for Stereo48KhzToMono16Khz {
    fn default() -> Self {
        Self::new()
    }
}

/// Adapts a 48 kHz stereo push-to-talk stream to the mono 16 kHz PCM format
/// expected by Whisper-class STT providers. The resampler and scratch buffer
/// are bounded and are erased on completion or cancellation.
pub struct ResamplingSpeechToTextProvider<P> {
    provider: P,
    resampler: Stereo48KhzToMono16Khz,
    resampled_chunk: [u8; MAX_SPEECH_RESAMPLED_CHUNK_BYTES],
}

impl<P> ResamplingSpeechToTextProvider<P>
where
    P: SpeechToTextProvider,
{
    pub const fn new(provider: P) -> Self {
        Self {
            provider,
            resampler: Stereo48KhzToMono16Khz::new(),
            resampled_chunk: [0; MAX_SPEECH_RESAMPLED_CHUNK_BYTES],
        }
    }

    pub fn inner(&self) -> &P {
        &self.provider
    }
}

impl<P> SpeechToTextProvider for ResamplingSpeechToTextProvider<P>
where
    P: SpeechToTextProvider,
{
    fn begin(&mut self, mut options: SpeechOptions) -> Result<(), SpeechProviderError> {
        self.resampler.reset();
        self.resampled_chunk.fill(0);
        if options.pcm_format != PcmFormat::stereo_48khz() {
            return Err(SpeechProviderError::Failed);
        }
        options.pcm_format = PcmFormat::mono_16khz();
        self.provider.begin(options)
    }

    fn push_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), SpeechProviderError> {
        if format != PcmFormat::stereo_48khz()
            || bytes.is_empty()
            || bytes.len() > MAX_SPEECH_PCM_CHUNK_BYTES
            || bytes.len() & 3 != 0
        {
            return Err(SpeechProviderError::Failed);
        }
        self.resampled_chunk.fill(0);
        let samples = self
            .resampler
            .convert(bytes, &mut self.resampled_chunk)
            .map_err(|_| SpeechProviderError::Failed)?;
        let bytes = samples * 2;
        let result = if bytes == 0 {
            Ok(())
        } else {
            self.provider
                .push_pcm(PcmFormat::mono_16khz(), &self.resampled_chunk[..bytes])
        };
        self.resampled_chunk.fill(0);
        result
    }

    fn finish(&mut self, transcript: &mut [u8]) -> Result<usize, SpeechProviderError> {
        self.resampled_chunk.fill(0);
        let tail_samples = self
            .resampler
            .finish(&mut self.resampled_chunk)
            .map_err(|_| SpeechProviderError::Failed)?;
        let tail_bytes = tail_samples * 2;
        if tail_bytes != 0 {
            let result = self
                .provider
                .push_pcm(PcmFormat::mono_16khz(), &self.resampled_chunk[..tail_bytes]);
            self.resampled_chunk.fill(0);
            result?;
        }
        self.resampled_chunk.fill(0);
        self.provider.finish(transcript)
    }

    fn cancel(&mut self) {
        self.resampler.reset();
        self.resampled_chunk.fill(0);
        self.provider.cancel();
    }
}

/// A replaceable local TTS engine. Implementations receive only bounded UTF-8
/// text and a format request; they have no OS authority. PCM is pulled in
/// caller-owned chunks and must be signed 16-bit little-endian samples.
pub trait TextToSpeechProvider {
    fn begin(
        &mut self,
        text: &str,
        options: SpeechSynthesisOptions,
    ) -> Result<(), SpeechProviderError>;
    fn next_pcm_chunk(
        &mut self,
        destination: &mut [u8],
    ) -> Result<SynthesisPcmChunk, SpeechProviderError>;
    fn cancel(&mut self);
}

/// The service-owned playback boundary. The sink owns any AudioService
/// capability and never exposes it to the provider or synthesis caller.
pub trait SpeechPlaybackSink {
    fn play_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), PlaybackError>;
}

/// Streams one bounded utterance from a TTS provider into the system audio
/// service without allocating an utterance-sized PCM buffer.
pub struct SpeechSynthesisService<P, S> {
    provider: P,
    sink: S,
    pcm_chunk: [u8; MAX_SPEECH_PCM_CHUNK_BYTES],
}

impl<P, S> SpeechSynthesisService<P, S>
where
    P: TextToSpeechProvider,
    S: SpeechPlaybackSink,
{
    pub const fn new(provider: P, sink: S) -> Self {
        Self {
            provider,
            sink,
            pcm_chunk: [0; MAX_SPEECH_PCM_CHUNK_BYTES],
        }
    }

    /// Synthesizes validated UTF-8 input as bounded PCM chunks. The total
    /// output is capped at 1 MiB and each chunk must contain whole stereo
    /// signed-16-bit frames. The returned count is bytes accepted by playback.
    pub fn speak(
        &mut self,
        text: &[u8],
        options: SpeechSynthesisOptions,
    ) -> Result<usize, SpeechSynthesisError> {
        if text.is_empty() {
            return Err(SpeechSynthesisError::EmptyText);
        }
        if text.len() > MAX_SPEECH_SYNTHESIS_TEXT_BYTES {
            return Err(SpeechSynthesisError::TextTooLong);
        }
        let text = core::str::from_utf8(text).map_err(|_| SpeechSynthesisError::InvalidText)?;
        if let Err(error) = self.provider.begin(text, options) {
            return Err(self.abort(map_synthesis_provider_error(error)));
        }

        let frame_bytes = usize::from(options.pcm_format.channels()) * 2;
        let mut total_bytes = 0usize;
        loop {
            self.pcm_chunk.fill(0);
            match self.provider.next_pcm_chunk(&mut self.pcm_chunk) {
                Ok(SynthesisPcmChunk::End) => {
                    self.pcm_chunk.fill(0);
                    if total_bytes == 0 {
                        return Err(self.abort(SpeechSynthesisError::NoAudioProduced));
                    }
                    return Ok(total_bytes);
                }
                Ok(SynthesisPcmChunk::Data(bytes)) => {
                    if bytes == 0 || bytes > self.pcm_chunk.len() || bytes % frame_bytes != 0 {
                        return Err(self.abort(SpeechSynthesisError::InvalidPcmData));
                    }
                    let Some(next_total) = total_bytes.checked_add(bytes) else {
                        return Err(self.abort(SpeechSynthesisError::UtteranceTooLong));
                    };
                    if next_total > MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES {
                        return Err(self.abort(SpeechSynthesisError::UtteranceTooLong));
                    }
                    if self
                        .sink
                        .play_pcm(options.pcm_format, &self.pcm_chunk[..bytes])
                        .is_err()
                    {
                        return Err(self.abort(SpeechSynthesisError::PlaybackUnavailable));
                    }
                    total_bytes = next_total;
                    self.pcm_chunk.fill(0);
                }
                Err(error) => return Err(self.abort(map_synthesis_provider_error(error))),
            }
        }
    }

    fn abort(&mut self, error: SpeechSynthesisError) -> SpeechSynthesisError {
        self.provider.cancel();
        self.pcm_chunk.fill(0);
        error
    }
}

#[cfg(target_os = "nagi")]
pub struct AudioServicePlaybackSink {
    audio: AudioService,
    stream_id: u32,
}

#[cfg(target_os = "nagi")]
impl AudioServicePlaybackSink {
    pub const fn new(audio: AudioService, stream_id: u32) -> Self {
        Self { audio, stream_id }
    }
}

#[cfg(target_os = "nagi")]
impl SpeechPlaybackSink for AudioServicePlaybackSink {
    fn play_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), PlaybackError> {
        // `usize::is_multiple_of` is not available on the pinned nightly.
        #[allow(unknown_lints, clippy::manual_is_multiple_of)]
        if format != PcmFormat::stereo_48khz() || bytes.is_empty() || bytes.len() % 4 != 0 {
            return Err(PlaybackError::Unavailable);
        }
        if self.audio.play(self.stream_id, bytes) {
            Ok(())
        } else {
            Err(PlaybackError::Unavailable)
        }
    }
}

/// The target adapter keeps the device capability inside AudioService and
/// binds capture to one service-selected stream.
#[cfg(target_os = "nagi")]
pub struct MicrophoneCapture {
    audio: AudioService,
    stream_id: u32,
}

#[cfg(target_os = "nagi")]
impl MicrophoneCapture {
    pub const fn new(audio: AudioService, stream_id: u32) -> Self {
        Self { audio, stream_id }
    }
}

#[cfg(target_os = "nagi")]
impl PcmCaptureSource for MicrophoneCapture {
    fn pcm_format(&self) -> PcmFormat {
        PcmFormat::stereo_48khz()
    }

    fn capture_pcm(&mut self, destination: &mut [u8]) -> Result<usize, CaptureSourceError> {
        let captured = self.audio.capture(self.stream_id, destination);
        if captured == 0 {
            Err(CaptureSourceError::Unavailable)
        } else {
            Ok(captured)
        }
    }
}

/// Coordinates a short-lived microphone session. It stores only one bounded
/// audio chunk; the provider owns its separately bounded streaming state.
pub struct PushToTalkService<C, A, I, P> {
    capture: C,
    authority: A,
    indicator: I,
    provider: P,
    active_consumer: Option<SpeechConsumer>,
    captured_bytes: usize,
    pcm_chunk: [u8; MAX_SPEECH_PCM_CHUNK_BYTES],
}

impl<C, A, I, P> PushToTalkService<C, A, I, P>
where
    C: PcmCaptureSource,
    A: SpeechPermissionAuthority,
    I: MicrophoneActivityIndicator,
    P: SpeechToTextProvider,
{
    pub const fn new(capture: C, authority: A, indicator: I, provider: P) -> Self {
        Self {
            capture,
            authority,
            indicator,
            provider,
            active_consumer: None,
            captured_bytes: 0,
            pcm_chunk: [0; MAX_SPEECH_PCM_CHUNK_BYTES],
        }
    }

    pub fn begin(
        &mut self,
        consumer: SpeechConsumer,
        language: SpeechLanguage,
    ) -> Result<(), SpeechError> {
        if self.active_consumer.is_some() {
            return Err(SpeechError::AlreadyActive);
        }
        match self.authority.authorize_push_to_talk(consumer) {
            SpeechPermissionDecision::Allow => {}
            SpeechPermissionDecision::Ask => return Err(SpeechError::PermissionRequired),
            SpeechPermissionDecision::Deny => return Err(SpeechError::PermissionDenied),
        }

        if self.indicator.show(consumer).is_err() {
            self.indicator.hide(consumer);
            return Err(SpeechError::IndicatorUnavailable);
        }

        let options = SpeechOptions {
            language,
            pcm_format: self.capture.pcm_format(),
        };
        if let Err(error) = self.provider.begin(options) {
            self.provider.cancel();
            self.indicator.hide(consumer);
            return Err(map_provider_error(error));
        }

        self.active_consumer = Some(consumer);
        self.captured_bytes = 0;
        Ok(())
    }

    /// Captures one bounded frame and streams it directly to the STT provider.
    /// PCM is erased from the service buffer before this method returns.
    pub fn capture_next_chunk(&mut self) -> Result<usize, SpeechError> {
        let Some(consumer) = self.active_consumer else {
            return Err(SpeechError::NotActive);
        };
        let remaining = MAX_SPEECH_UTTERANCE_BYTES.saturating_sub(self.captured_bytes);
        if remaining == 0 {
            self.abort_active(consumer);
            return Err(SpeechError::UtteranceTooLong);
        }

        self.pcm_chunk.fill(0);
        let destination_len = remaining.min(self.pcm_chunk.len());
        let captured = match self
            .capture
            .capture_pcm(&mut self.pcm_chunk[..destination_len])
        {
            Ok(captured) if captured > 0 && captured <= destination_len => captured,
            _ => {
                self.abort_active(consumer);
                return Err(SpeechError::CaptureFailed);
            }
        };
        let frame_bytes = usize::from(self.capture.pcm_format().channels()) * 2;
        if captured % frame_bytes != 0 {
            self.abort_active(consumer);
            return Err(SpeechError::InvalidPcmData);
        }

        let format = self.capture.pcm_format();
        let result = self.provider.push_pcm(format, &self.pcm_chunk[..captured]);
        self.pcm_chunk.fill(0);
        if let Err(error) = result {
            self.abort_active(consumer);
            return Err(map_provider_error(error));
        }
        self.captured_bytes += captured;
        Ok(captured)
    }

    /// Ends capture and returns only UTF-8 transcript bytes, never microphone
    /// samples. The output capacity is capped even if the caller gives a larger
    /// buffer. Provider output is discarded on every error.
    pub fn finish_into(&mut self, output: &mut [u8]) -> Result<usize, SpeechError> {
        let Some(consumer) = self.active_consumer else {
            return Err(SpeechError::NotActive);
        };
        output.fill(0);
        let transcript_capacity = output.len().min(MAX_SPEECH_TRANSCRIPT_BYTES);
        let result = self.provider.finish(&mut output[..transcript_capacity]);
        self.end_active(consumer);
        let length = match result {
            Ok(length) if length <= transcript_capacity => length,
            Ok(_) | Err(SpeechProviderError::OutputTooSmall) => {
                self.provider.cancel();
                output.fill(0);
                return Err(SpeechError::OutputTooSmall);
            }
            Err(error) => {
                self.provider.cancel();
                output.fill(0);
                return Err(map_provider_error(error));
            }
        };
        if length == 0 {
            self.provider.cancel();
            output.fill(0);
            return Err(SpeechError::EmptyTranscript);
        }
        if core::str::from_utf8(&output[..length]).is_err() {
            self.provider.cancel();
            output.fill(0);
            return Err(SpeechError::InvalidTranscript);
        }
        Ok(length)
    }

    pub fn cancel(&mut self) -> Result<(), SpeechError> {
        let Some(consumer) = self.active_consumer else {
            return Err(SpeechError::NotActive);
        };
        self.provider.cancel();
        self.end_active(consumer);
        Ok(())
    }

    fn abort_active(&mut self, consumer: SpeechConsumer) {
        self.provider.cancel();
        self.end_active(consumer);
    }

    fn end_active(&mut self, consumer: SpeechConsumer) {
        self.pcm_chunk.fill(0);
        self.captured_bytes = 0;
        self.active_consumer = None;
        self.indicator.hide(consumer);
    }
}

fn map_provider_error(error: SpeechProviderError) -> SpeechError {
    match error {
        SpeechProviderError::Unavailable => SpeechError::ProviderUnavailable,
        SpeechProviderError::Failed => SpeechError::ProviderFailed,
        SpeechProviderError::OutputTooSmall => SpeechError::OutputTooSmall,
    }
}

fn map_synthesis_provider_error(error: SpeechProviderError) -> SpeechSynthesisError {
    match error {
        SpeechProviderError::Unavailable => SpeechSynthesisError::ProviderUnavailable,
        SpeechProviderError::Failed => SpeechSynthesisError::ProviderFailed,
        SpeechProviderError::OutputTooSmall => SpeechSynthesisError::ProviderOutputTooSmall,
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::{cell::Cell, f64::consts::TAU, rc::Rc, vec, vec::Vec};

    fn stereo_constant(frames: usize, left: i16, right: i16) -> Vec<u8> {
        let mut input = Vec::with_capacity(frames * 4);
        for _ in 0..frames {
            input.extend_from_slice(&left.to_le_bytes());
            input.extend_from_slice(&right.to_le_bytes());
        }
        input
    }

    fn resample_in_chunks(input: &[u8], chunk_frames: usize) -> Vec<u8> {
        let mut resampler = Stereo48KhzToMono16Khz::new();
        let mut output = Vec::new();
        let mut offset = 0;
        while offset < input.len() {
            let chunk_bytes = (chunk_frames * 4).min(input.len() - offset);
            let mut chunk_output = [0; MAX_SPEECH_RESAMPLED_CHUNK_BYTES];
            let samples = resampler
                .convert(&input[offset..offset + chunk_bytes], &mut chunk_output)
                .unwrap();
            output.extend_from_slice(&chunk_output[..samples * 2]);
            offset += chunk_bytes;
        }
        let mut tail = [0; MAX_SPEECH_RESAMPLED_CHUNK_BYTES];
        let samples = resampler.finish(&mut tail).unwrap();
        output.extend_from_slice(&tail[..samples * 2]);
        output
    }

    #[test]
    fn stereo_48khz_resampler_is_chunk_boundary_independent() {
        let input = stereo_constant(751, 12_000, -4_000);
        let mut whole = Stereo48KhzToMono16Khz::new();
        let mut whole_output = vec![0; MAX_SPEECH_RESAMPLED_CHUNK_BYTES * 4];
        let samples = whole.convert(&input, &mut whole_output).unwrap();
        let mut tail = [0; MAX_SPEECH_RESAMPLED_CHUNK_BYTES];
        let tail_samples = whole.finish(&mut tail).unwrap();
        let mut expected = whole_output[..samples * 2].to_vec();
        expected.extend_from_slice(&tail[..tail_samples * 2]);

        assert_eq!(resample_in_chunks(&input, 1), expected);
        assert_eq!(resample_in_chunks(&input, 137), expected);
        assert_eq!(resample_in_chunks(&input, 1024), expected);
        assert_eq!(expected.len() / 2, (751 + SPEECH_RESAMPLER_TAIL_FRAMES) / 3);
    }

    #[test]
    fn stereo_48khz_resampler_validates_before_changing_state() {
        let input = stereo_constant(9, 10_000, 2_000);
        let mut resampler = Stereo48KhzToMono16Khz::new();
        let mut undersized = [0; 2];
        assert_eq!(
            resampler.convert(&input, &mut undersized),
            Err(PcmResampleError::OutputTooSmall)
        );
        assert_eq!(
            resampler.convert(&[1, 2, 3], &mut [0; 8]),
            Err(PcmResampleError::InvalidInput)
        );

        let mut actual = [0; MAX_SPEECH_RESAMPLED_CHUNK_BYTES];
        let count = resampler.convert(&input, &mut actual).unwrap();
        let mut fresh = Stereo48KhzToMono16Khz::new();
        let mut expected = [0; MAX_SPEECH_RESAMPLED_CHUNK_BYTES];
        let expected_count = fresh.convert(&input, &mut expected).unwrap();
        assert_eq!(count, expected_count);
        assert_eq!(&actual[..count * 2], &expected[..expected_count * 2]);
    }

    #[test]
    fn stereo_48khz_resampler_handles_silence_and_averages_channels() {
        let mut silence = Stereo48KhzToMono16Khz::new();
        let silent_input = stereo_constant(384, 0, 0);
        let mut output = vec![0; MAX_SPEECH_RESAMPLED_CHUNK_BYTES * 2];
        let samples = silence.convert(&silent_input, &mut output).unwrap();
        let mut tail = [0; MAX_SPEECH_RESAMPLED_CHUNK_BYTES];
        let tail_samples = silence.finish(&mut tail).unwrap();
        assert!(output[..samples * 2].iter().all(|byte| *byte == 0));
        assert!(tail[..tail_samples * 2].iter().all(|byte| *byte == 0));

        let mut converter = Stereo48KhzToMono16Khz::new();
        let input = stereo_constant(384, 10_000, 2_000);
        let samples = converter.convert(&input, &mut output).unwrap();
        let stable_sample =
            i16::from_le_bytes([output[(samples - 1) * 2], output[(samples - 1) * 2 + 1]]);
        assert!((stable_sample - 6_000).abs() <= 2);
    }

    #[test]
    fn stereo_48khz_resampler_rejects_stopband_aliases() {
        fn encode_tone(frequency_hz: f64) -> Vec<u8> {
            let frames = 24_000;
            let mut input = Vec::with_capacity(frames * 4);
            for frame in 0..frames {
                let phase = TAU * frequency_hz * frame as f64 / 48_000.0;
                let sample = (phase.sin() * 12_000.0) as i16;
                input.extend_from_slice(&sample.to_le_bytes());
                input.extend_from_slice(&sample.to_le_bytes());
            }
            input
        }

        fn output_rms(input: &[u8]) -> f64 {
            let mut resampler = Stereo48KhzToMono16Khz::new();
            let mut output = vec![0; input.len() / 3 + MAX_SPEECH_RESAMPLED_CHUNK_BYTES];
            let samples = resampler.convert(input, &mut output).unwrap();
            let mut sum = 0_f64;
            let mut count = 0;
            // Measure steady-state response; the finite signal's ending edge
            // is intentionally excluded from this stop-band check.
            for bytes in output[..samples.saturating_sub(100) * 2]
                .chunks_exact(2)
                .skip(100)
            {
                let sample = f64::from(i16::from_le_bytes([bytes[0], bytes[1]]));
                sum += sample * sample;
                count += 1;
            }
            (sum / count as f64).sqrt()
        }

        let passband_rms = output_rms(&encode_tone(1_000.0));
        let stopband_rms = output_rms(&encode_tone(12_000.0));
        assert!(passband_rms > 8_000.0);
        assert!(
            stopband_rms / passband_rms < 0.002,
            "passband RMS {passband_rms}, stopband RMS {stopband_rms}"
        );
    }

    struct RecordingSpeechProvider {
        begin_format: Option<PcmFormat>,
        last_format: Option<PcmFormat>,
        pushed_bytes: usize,
        push_calls: usize,
        cancels: usize,
    }

    impl SpeechToTextProvider for RecordingSpeechProvider {
        fn begin(&mut self, options: SpeechOptions) -> Result<(), SpeechProviderError> {
            self.begin_format = Some(options.pcm_format);
            Ok(())
        }

        fn push_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), SpeechProviderError> {
            self.last_format = Some(format);
            self.pushed_bytes += bytes.len();
            self.push_calls += 1;
            Ok(())
        }

        fn finish(&mut self, _transcript: &mut [u8]) -> Result<usize, SpeechProviderError> {
            Ok(0)
        }

        fn cancel(&mut self) {
            self.cancels += 1;
        }
    }

    #[test]
    fn resampling_provider_flushes_tail_and_resets_between_utterances() {
        let provider = RecordingSpeechProvider {
            begin_format: None,
            last_format: None,
            pushed_bytes: 0,
            push_calls: 0,
            cancels: 0,
        };
        let mut adapter = ResamplingSpeechToTextProvider::new(provider);
        let options = SpeechOptions {
            language: SpeechLanguage::Japanese,
            pcm_format: PcmFormat::stereo_48khz(),
        };
        adapter.begin(options).unwrap();
        assert_eq!(adapter.inner().begin_format, Some(PcmFormat::mono_16khz()));

        let input = stereo_constant(6, 8_000, 8_000);
        adapter.push_pcm(PcmFormat::stereo_48khz(), &input).unwrap();
        let pushed_before_finish = adapter.inner().pushed_bytes;
        assert_eq!(pushed_before_finish, 4);
        let mut transcript = [0; 16];
        adapter.finish(&mut transcript).unwrap();
        assert!(adapter.inner().pushed_bytes > pushed_before_finish);
        assert_eq!(adapter.inner().last_format, Some(PcmFormat::mono_16khz()));

        adapter.begin(options).unwrap();
        adapter.push_pcm(PcmFormat::stereo_48khz(), &input).unwrap();
        adapter.cancel();
        assert_eq!(adapter.inner().cancels, 1);
        assert!(adapter.resampled_chunk.iter().all(|byte| *byte == 0));
        assert!(adapter.resampler.history.iter().all(|sample| *sample == 0));
    }

    #[derive(Clone)]
    struct FixtureInput {
        bytes: &'static [u8],
        capture_calls: Rc<Cell<usize>>,
        indicator_active: Rc<Cell<bool>>,
    }

    impl PcmCaptureSource for FixtureInput {
        fn pcm_format(&self) -> PcmFormat {
            PcmFormat::stereo_48khz()
        }

        fn capture_pcm(&mut self, destination: &mut [u8]) -> Result<usize, CaptureSourceError> {
            assert!(self.indicator_active.get());
            self.capture_calls.set(self.capture_calls.get() + 1);
            if self.bytes.len() > destination.len() {
                return Err(CaptureSourceError::Unavailable);
            }
            destination[..self.bytes.len()].copy_from_slice(self.bytes);
            Ok(self.bytes.len())
        }
    }

    struct FixtureAuthority(SpeechPermissionDecision);

    impl SpeechPermissionAuthority for FixtureAuthority {
        fn authorize_push_to_talk(
            &mut self,
            _consumer: SpeechConsumer,
        ) -> SpeechPermissionDecision {
            self.0
        }
    }

    #[derive(Clone)]
    struct FixtureIndicator {
        visible: Rc<Cell<bool>>,
        fail_after_show: bool,
    }

    impl MicrophoneActivityIndicator for FixtureIndicator {
        fn show(&mut self, _consumer: SpeechConsumer) -> Result<(), IndicatorError> {
            self.visible.set(true);
            if self.fail_after_show {
                Err(IndicatorError::Unavailable)
            } else {
                Ok(())
            }
        }

        fn hide(&mut self, _consumer: SpeechConsumer) {
            self.visible.set(false);
        }
    }

    #[derive(Default)]
    struct FixtureProvider {
        begin_calls: usize,
        cancel_calls: usize,
        pushed_bytes: usize,
        digest: u64,
        language: Option<SpeechLanguage>,
        return_empty_transcript: bool,
        transcript: Option<&'static [u8]>,
    }

    impl SpeechToTextProvider for FixtureProvider {
        fn begin(&mut self, options: SpeechOptions) -> Result<(), SpeechProviderError> {
            self.begin_calls += 1;
            self.language = Some(options.language);
            Ok(())
        }

        fn push_pcm(
            &mut self,
            _format: PcmFormat,
            bytes: &[u8],
        ) -> Result<(), SpeechProviderError> {
            self.pushed_bytes += bytes.len();
            for byte in bytes {
                self.digest = self.digest.wrapping_mul(16_777_619) ^ u64::from(*byte);
            }
            Ok(())
        }

        fn finish(&mut self, transcript: &mut [u8]) -> Result<usize, SpeechProviderError> {
            // Configured fixture text validates handoff only; it is not STT
            // inference. No transcript remains the default fixture behavior.
            if self.return_empty_transcript {
                Ok(0)
            } else if let Some(fixture) = self.transcript {
                if fixture.len() > transcript.len() {
                    return Err(SpeechProviderError::OutputTooSmall);
                }
                transcript[..fixture.len()].copy_from_slice(fixture);
                Ok(fixture.len())
            } else {
                Err(SpeechProviderError::Unavailable)
            }
        }

        fn cancel(&mut self) {
            self.cancel_calls += 1;
        }
    }

    type FixtureService =
        PushToTalkService<FixtureInput, FixtureAuthority, FixtureIndicator, FixtureProvider>;
    type FixtureServiceState = (FixtureService, Rc<Cell<usize>>, Rc<Cell<bool>>);

    fn service(decision: SpeechPermissionDecision, fixture: &'static [u8]) -> FixtureServiceState {
        let calls = Rc::new(Cell::new(0));
        let indicator_active = Rc::new(Cell::new(false));
        let input = FixtureInput {
            bytes: fixture,
            capture_calls: Rc::clone(&calls),
            indicator_active: Rc::clone(&indicator_active),
        };
        let service = PushToTalkService::new(
            input,
            FixtureAuthority(decision),
            FixtureIndicator {
                visible: Rc::clone(&indicator_active),
                fail_after_show: false,
            },
            FixtureProvider::default(),
        );
        (service, calls, indicator_active)
    }

    #[test]
    fn indicator_failure_hides_partial_indicator_before_returning() {
        let (mut service, capture_calls, indicator_active) =
            service(SpeechPermissionDecision::Allow, &[1, 2, 3, 4]);
        service.indicator.fail_after_show = true;

        assert_eq!(
            service.begin(SpeechConsumer::NagiBar, SpeechLanguage::Japanese),
            Err(SpeechError::IndicatorUnavailable)
        );
        assert!(!indicator_active.get());
        assert_eq!(capture_calls.get(), 0);
        assert_eq!(service.provider.begin_calls, 0);
        assert_eq!(service.capture_next_chunk(), Err(SpeechError::NotActive));
    }

    #[test]
    fn permission_prompt_blocks_capture_provider_and_indicator() {
        let (mut service, capture_calls, indicator_active) =
            service(SpeechPermissionDecision::Ask, &[1, 2, 3, 4]);
        assert_eq!(
            service.begin(SpeechConsumer::Albert, SpeechLanguage::Japanese),
            Err(SpeechError::PermissionRequired)
        );
        assert_eq!(capture_calls.get(), 0);
        assert!(!indicator_active.get());
        assert_eq!(service.provider.begin_calls, 0);
        assert_eq!(service.capture_next_chunk(), Err(SpeechError::NotActive));
    }

    #[test]
    fn authorized_push_to_talk_streams_fixture_audio_but_no_fake_transcript() {
        static FIXTURE: [u8; 8] = [0x10, 0x20, 0x30, 0x40, 0xfe, 0xdc, 0xba, 0x98];
        let (mut service, capture_calls, indicator_active) =
            service(SpeechPermissionDecision::Allow, &FIXTURE);
        service
            .begin(SpeechConsumer::NagiBar, SpeechLanguage::Japanese)
            .expect("authorized PTT starts");
        assert!(indicator_active.get());
        assert_eq!(service.provider.language, Some(SpeechLanguage::Japanese));
        assert_eq!(service.capture_next_chunk(), Ok(FIXTURE.len()));
        assert_eq!(capture_calls.get(), 1);
        assert_eq!(service.provider.pushed_bytes, FIXTURE.len());
        assert_ne!(service.provider.digest, 0);
        assert!(service.pcm_chunk.iter().all(|byte| *byte == 0));

        let mut transcript = [0xaa; 32];
        assert_eq!(
            service.finish_into(&mut transcript),
            Err(SpeechError::ProviderUnavailable)
        );
        assert!(transcript.iter().all(|byte| *byte == 0));
        assert!(!indicator_active.get());
        assert_eq!(service.provider.cancel_calls, 1);
        assert_eq!(service.capture_next_chunk(), Err(SpeechError::NotActive));
    }

    #[test]
    fn successful_fixture_transcript_is_delivered_and_audio_state_is_cleared() {
        static FIXTURE: [u8; 8] = [0x10, 0x20, 0x30, 0x40, 0xfe, 0xdc, 0xba, 0x98];
        static TRANSCRIPT: &[u8] = "fixture transcript".as_bytes();
        let (mut service, _, indicator_active) = service(SpeechPermissionDecision::Allow, &FIXTURE);
        service.provider.transcript = Some(TRANSCRIPT);
        service
            .begin(SpeechConsumer::NagiBar, SpeechLanguage::Japanese)
            .expect("authorized PTT starts");
        service.capture_next_chunk().expect("fixture frame");

        let mut transcript = [0xa5; 32];
        assert_eq!(service.finish_into(&mut transcript), Ok(TRANSCRIPT.len()));
        assert_eq!(&transcript[..TRANSCRIPT.len()], TRANSCRIPT);
        assert!(transcript[TRANSCRIPT.len()..].iter().all(|byte| *byte == 0));
        assert!(!indicator_active.get());
        assert!(service.pcm_chunk.iter().all(|byte| *byte == 0));
        assert_eq!(service.provider.cancel_calls, 0);
        assert_eq!(service.capture_next_chunk(), Err(SpeechError::NotActive));
    }

    #[test]
    fn empty_transcript_is_rejected_and_provider_state_is_cleared() {
        static FIXTURE: [u8; 8] = [0x10, 0x20, 0x30, 0x40, 0xfe, 0xdc, 0xba, 0x98];
        let (mut service, _, indicator_active) = service(SpeechPermissionDecision::Allow, &FIXTURE);
        service.provider.return_empty_transcript = true;
        service
            .begin(SpeechConsumer::NagiBar, SpeechLanguage::Japanese)
            .expect("authorized PTT starts");
        service.capture_next_chunk().expect("fixture frame");

        let mut transcript = [0xaa; 32];
        assert_eq!(
            service.finish_into(&mut transcript),
            Err(SpeechError::EmptyTranscript)
        );
        assert!(transcript.iter().all(|byte| *byte == 0));
        assert!(!indicator_active.get());
        assert_eq!(service.provider.cancel_calls, 1);
        assert_eq!(service.capture_next_chunk(), Err(SpeechError::NotActive));
    }

    #[test]
    fn deny_and_malformed_audio_fail_closed() {
        let (mut denied, calls, indicator_active) =
            service(SpeechPermissionDecision::Deny, &[1, 2, 3, 4]);
        assert_eq!(
            denied.begin(SpeechConsumer::Albert, SpeechLanguage::Auto),
            Err(SpeechError::PermissionDenied)
        );
        assert_eq!(calls.get(), 0);
        assert!(!indicator_active.get());

        let (mut malformed, calls, indicator_active) =
            service(SpeechPermissionDecision::Allow, &[1, 2, 3, 4, 5, 6]);
        malformed
            .begin(SpeechConsumer::Albert, SpeechLanguage::Auto)
            .expect("authorized PTT starts");
        assert_eq!(
            malformed.capture_next_chunk(),
            Err(SpeechError::InvalidPcmData)
        );
        assert_eq!(calls.get(), 1);
        assert!(!indicator_active.get());
        assert!(malformed.pcm_chunk.iter().all(|byte| *byte == 0));
        assert_eq!(malformed.provider.cancel_calls, 1);
    }

    #[test]
    fn cancel_hides_indicator_and_discards_provider_state() {
        let (mut service, _, indicator_active) =
            service(SpeechPermissionDecision::Allow, &[1, 2, 3, 4]);
        service
            .begin(SpeechConsumer::Albert, SpeechLanguage::Japanese)
            .expect("authorized PTT starts");
        service.capture_next_chunk().expect("fixture frame");
        service.cancel().expect("cancel active PTT");
        assert_eq!(service.provider.cancel_calls, 1);
        assert!(!indicator_active.get());
        assert!(service.pcm_chunk.iter().all(|byte| *byte == 0));
        assert_eq!(service.captured_bytes, 0);
    }

    #[derive(Clone, Copy)]
    enum TtsFixtureMode {
        Normal,
        MalformedFrame,
        Endless,
        Empty,
    }

    struct FixtureTts {
        mode: TtsFixtureMode,
        next_chunk: usize,
        begin_calls: usize,
        cancel_calls: usize,
        text: Vec<u8>,
        language: Option<SpeechSynthesisLanguage>,
    }

    impl FixtureTts {
        fn new(mode: TtsFixtureMode) -> Self {
            Self {
                mode,
                next_chunk: 0,
                begin_calls: 0,
                cancel_calls: 0,
                text: Vec::new(),
                language: None,
            }
        }
    }

    impl TextToSpeechProvider for FixtureTts {
        fn begin(
            &mut self,
            text: &str,
            options: SpeechSynthesisOptions,
        ) -> Result<(), SpeechProviderError> {
            self.begin_calls += 1;
            self.text.extend_from_slice(text.as_bytes());
            self.language = Some(options.language);
            Ok(())
        }

        fn next_pcm_chunk(
            &mut self,
            destination: &mut [u8],
        ) -> Result<SynthesisPcmChunk, SpeechProviderError> {
            match self.mode {
                TtsFixtureMode::Empty => Ok(SynthesisPcmChunk::End),
                TtsFixtureMode::MalformedFrame if self.next_chunk == 0 => {
                    destination[..3].copy_from_slice(&[1, 2, 3]);
                    self.next_chunk += 1;
                    Ok(SynthesisPcmChunk::Data(3))
                }
                TtsFixtureMode::Endless => {
                    destination.fill(0x55);
                    Ok(SynthesisPcmChunk::Data(destination.len()))
                }
                _ => match self.next_chunk {
                    0 => {
                        destination[..4].copy_from_slice(&[1, 2, 3, 4]);
                        self.next_chunk += 1;
                        Ok(SynthesisPcmChunk::Data(4))
                    }
                    1 => {
                        destination[..4].copy_from_slice(&[5, 6, 7, 8]);
                        self.next_chunk += 1;
                        Ok(SynthesisPcmChunk::Data(4))
                    }
                    _ => Ok(SynthesisPcmChunk::End),
                },
            }
        }

        fn cancel(&mut self) {
            self.cancel_calls += 1;
        }
    }

    #[derive(Default)]
    struct FixturePlayback {
        calls: usize,
        bytes: Vec<u8>,
        fail: bool,
    }

    impl SpeechPlaybackSink for FixturePlayback {
        fn play_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), PlaybackError> {
            assert_eq!(format, PcmFormat::stereo_48khz());
            self.calls += 1;
            if self.fail {
                return Err(PlaybackError::Unavailable);
            }
            self.bytes.extend_from_slice(bytes);
            Ok(())
        }
    }

    fn tts_options() -> SpeechSynthesisOptions {
        SpeechSynthesisOptions {
            language: SpeechSynthesisLanguage::Japanese,
            pcm_format: PcmFormat::stereo_48khz(),
        }
    }

    #[test]
    fn tts_streams_bounded_japanese_text_as_pcm_chunks() {
        let mut service = SpeechSynthesisService::new(
            FixtureTts::new(TtsFixtureMode::Normal),
            FixturePlayback::default(),
        );
        assert_eq!(service.speak("こんにちは".as_bytes(), tts_options()), Ok(8));
        assert_eq!(service.provider.begin_calls, 1);
        assert_eq!(service.provider.text, "こんにちは".as_bytes());
        assert_eq!(
            service.provider.language,
            Some(SpeechSynthesisLanguage::Japanese)
        );
        assert_eq!(service.sink.calls, 2);
        assert_eq!(service.sink.bytes, [1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(service.provider.cancel_calls, 0);
        assert!(service.pcm_chunk.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn tts_rejects_empty_oversized_and_invalid_utf8_before_provider_start() {
        let mut service = SpeechSynthesisService::new(
            FixtureTts::new(TtsFixtureMode::Normal),
            FixturePlayback::default(),
        );
        assert_eq!(
            service.speak(b"", tts_options()),
            Err(SpeechSynthesisError::EmptyText)
        );
        assert_eq!(
            service.speak(&[0xff], tts_options()),
            Err(SpeechSynthesisError::InvalidText)
        );
        let oversized = [b'x'; MAX_SPEECH_SYNTHESIS_TEXT_BYTES + 1];
        assert_eq!(
            service.speak(&oversized, tts_options()),
            Err(SpeechSynthesisError::TextTooLong)
        );
        assert_eq!(service.provider.begin_calls, 0);
        assert_eq!(service.sink.calls, 0);
    }

    #[test]
    fn tts_rejects_malformed_pcm_and_clears_provider_buffer() {
        let mut service = SpeechSynthesisService::new(
            FixtureTts::new(TtsFixtureMode::MalformedFrame),
            FixturePlayback::default(),
        );
        assert_eq!(
            service.speak("短い文".as_bytes(), tts_options()),
            Err(SpeechSynthesisError::InvalidPcmData)
        );
        assert_eq!(service.sink.calls, 0);
        assert_eq!(service.provider.cancel_calls, 1);
        assert!(service.pcm_chunk.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn tts_does_not_report_success_when_provider_returns_no_audio() {
        let mut service = SpeechSynthesisService::new(
            FixtureTts::new(TtsFixtureMode::Empty),
            FixturePlayback::default(),
        );
        assert_eq!(
            service.speak(b"bounded", tts_options()),
            Err(SpeechSynthesisError::NoAudioProduced)
        );
        assert_eq!(service.sink.calls, 0);
        assert_eq!(service.provider.cancel_calls, 1);
        assert!(service.pcm_chunk.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn tts_caps_total_pcm_output_and_cancels_on_playback_failure() {
        let mut endless = SpeechSynthesisService::new(
            FixtureTts::new(TtsFixtureMode::Endless),
            FixturePlayback::default(),
        );
        assert_eq!(
            endless.speak(b"bounded", tts_options()),
            Err(SpeechSynthesisError::UtteranceTooLong)
        );
        assert_eq!(
            endless.sink.calls,
            MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES / 4096
        );
        assert_eq!(endless.provider.cancel_calls, 1);
        assert!(endless.pcm_chunk.iter().all(|byte| *byte == 0));

        let mut playback_failure = SpeechSynthesisService::new(
            FixtureTts::new(TtsFixtureMode::Normal),
            FixturePlayback {
                fail: true,
                ..FixturePlayback::default()
            },
        );
        assert_eq!(
            playback_failure.speak(b"bounded", tts_options()),
            Err(SpeechSynthesisError::PlaybackUnavailable)
        );
        assert_eq!(playback_failure.provider.cancel_calls, 1);
        assert!(playback_failure.pcm_chunk.iter().all(|byte| *byte == 0));
    }
}
