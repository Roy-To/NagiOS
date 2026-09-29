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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpeechOptions {
    pub language: SpeechLanguage,
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
    InvalidTranscript,
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
    fn hide(&mut self, consumer: SpeechConsumer);
}

/// A service-owned source. Implementations must not expose its buffer to apps.
pub trait PcmCaptureSource {
    fn pcm_format(&self) -> PcmFormat;
    fn capture_pcm(&mut self, destination: &mut [u8]) -> Result<usize, CaptureSourceError>;
}

/// Provider implementations receive bounded PCM and return bounded UTF-8
/// text. They receive no OS capability and must not execute the transcript.
pub trait SpeechToTextProvider {
    fn begin(&mut self, options: SpeechOptions) -> Result<(), SpeechProviderError>;
    fn push_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), SpeechProviderError>;
    fn finish(&mut self, transcript: &mut [u8]) -> Result<usize, SpeechProviderError>;
    fn cancel(&mut self);
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

        self.indicator
            .show(consumer)
            .map_err(|IndicatorError::Unavailable| SpeechError::IndicatorUnavailable)?;

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
                output.fill(0);
                return Err(SpeechError::OutputTooSmall);
            }
            Err(error) => {
                output.fill(0);
                return Err(map_provider_error(error));
            }
        };
        if core::str::from_utf8(&output[..length]).is_err() {
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

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::{cell::Cell, rc::Rc};

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
    struct FixtureIndicator(Rc<Cell<bool>>);

    impl MicrophoneActivityIndicator for FixtureIndicator {
        fn show(&mut self, _consumer: SpeechConsumer) -> Result<(), IndicatorError> {
            self.0.set(true);
            Ok(())
        }

        fn hide(&mut self, _consumer: SpeechConsumer) {
            self.0.set(false);
        }
    }

    #[derive(Default)]
    struct FixtureProvider {
        begin_calls: usize,
        cancel_calls: usize,
        pushed_bytes: usize,
        digest: u64,
        language: Option<SpeechLanguage>,
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

        fn finish(&mut self, _transcript: &mut [u8]) -> Result<usize, SpeechProviderError> {
            // The fixture validates orchestration and audio delivery only. It
            // deliberately does not invent a transcript.
            Err(SpeechProviderError::Unavailable)
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
            FixtureIndicator(Rc::clone(&indicator_active)),
            FixtureProvider::default(),
        );
        (service, calls, indicator_active)
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
}
