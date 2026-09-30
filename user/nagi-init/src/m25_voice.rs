use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

use nagi_audio::speech::{
    CaptureSourceError, IndicatorError, MicrophoneActivityIndicator, PcmCaptureSource,
    PlaybackError, PushToTalkService, SpeechConsumer, SpeechError, SpeechLanguage, SpeechOptions,
    SpeechPermissionAuthority, SpeechPermissionDecision, SpeechPlaybackSink, SpeechProviderError,
    SpeechSynthesisLanguage, SpeechSynthesisOptions, SpeechSynthesisService, SpeechToTextProvider,
    SynthesisPcmChunk, TextToSpeechProvider,
};
use nagi_audio::PcmFormat;

const PIPELINE_IDLE: u8 = 0;
const PERMISSION_ACCEPTED: u8 = 1;
const INDICATOR_SHOWN: u8 = 2;
const PROVIDER_ACTIVE: u8 = 3;
const PROVIDER_FINISHED: u8 = 4;
const CLEANED_UP: u8 = 5;
const INVALID_ORDER: u8 = u8::MAX;
const FIXTURE_PCM: [u8; 8] = [0x10, 0x20, 0x30, 0x40, 0xfe, 0xdc, 0xba, 0x98];

static PIPELINE_STAGE: AtomicU8 = AtomicU8::new(PIPELINE_IDLE);
static INDICATOR_VISIBLE: AtomicBool = AtomicBool::new(false);
static CAPTURE_CALLS: AtomicUsize = AtomicUsize::new(0);
static FORWARDED_BYTES: AtomicUsize = AtomicUsize::new(0);
static PCM_DIGEST: AtomicUsize = AtomicUsize::new(0);
static PROVIDER_CANCELS: AtomicUsize = AtomicUsize::new(0);
static TTS_PLAYBACK_CHUNKS: AtomicUsize = AtomicUsize::new(0);
static TTS_PLAYBACK_BYTES: AtomicUsize = AtomicUsize::new(0);
static TTS_PCM_DIGEST: AtomicUsize = AtomicUsize::new(0);
static TTS_PROVIDER_CANCELS: AtomicUsize = AtomicUsize::new(0);

const FIXTURE_TTS_PCM: [[u8; 4]; 2] = [[0x10, 0x20, 0x30, 0x40], [0x50, 0x60, 0x70, 0x80]];

struct FixtureAuthority {
    allow: bool,
}

impl SpeechPermissionAuthority for FixtureAuthority {
    fn authorize_push_to_talk(&mut self, consumer: SpeechConsumer) -> SpeechPermissionDecision {
        if !self.allow || consumer != SpeechConsumer::NagiBar {
            return SpeechPermissionDecision::Deny;
        }
        if PIPELINE_STAGE
            .compare_exchange(
                PIPELINE_IDLE,
                PERMISSION_ACCEPTED,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            SpeechPermissionDecision::Allow
        } else {
            SpeechPermissionDecision::Deny
        }
    }
}

struct FixtureIndicator;

impl MicrophoneActivityIndicator for FixtureIndicator {
    fn show(&mut self, consumer: SpeechConsumer) -> Result<(), IndicatorError> {
        if consumer != SpeechConsumer::NagiBar
            || PIPELINE_STAGE
                .compare_exchange(
                    PERMISSION_ACCEPTED,
                    INDICATOR_SHOWN,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_err()
        {
            return Err(IndicatorError::Unavailable);
        }
        INDICATOR_VISIBLE.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn hide(&mut self, _consumer: SpeechConsumer) {
        INDICATOR_VISIBLE.store(false, Ordering::Relaxed);
        if PIPELINE_STAGE
            .compare_exchange(
                PROVIDER_FINISHED,
                CLEANED_UP,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_err()
        {
            PIPELINE_STAGE.store(INVALID_ORDER, Ordering::Relaxed);
        }
    }
}

struct FixtureCapture;

impl PcmCaptureSource for FixtureCapture {
    fn pcm_format(&self) -> PcmFormat {
        PcmFormat::stereo_48khz()
    }

    fn capture_pcm(&mut self, destination: &mut [u8]) -> Result<usize, CaptureSourceError> {
        if PIPELINE_STAGE.load(Ordering::Relaxed) != PROVIDER_ACTIVE
            || !INDICATOR_VISIBLE.load(Ordering::Relaxed)
            || destination.len() < FIXTURE_PCM.len()
        {
            return Err(CaptureSourceError::Unavailable);
        }
        destination[..FIXTURE_PCM.len()].copy_from_slice(&FIXTURE_PCM);
        CAPTURE_CALLS.fetch_add(1, Ordering::Relaxed);
        Ok(FIXTURE_PCM.len())
    }
}

struct FixtureProvider;

impl SpeechToTextProvider for FixtureProvider {
    fn begin(&mut self, options: SpeechOptions) -> Result<(), SpeechProviderError> {
        if options.language != SpeechLanguage::Japanese
            || options.pcm_format != PcmFormat::stereo_48khz()
            || !INDICATOR_VISIBLE.load(Ordering::Relaxed)
            || PIPELINE_STAGE
                .compare_exchange(
                    INDICATOR_SHOWN,
                    PROVIDER_ACTIVE,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_err()
        {
            return Err(SpeechProviderError::Failed);
        }
        Ok(())
    }

    fn push_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), SpeechProviderError> {
        if format != PcmFormat::stereo_48khz()
            || !INDICATOR_VISIBLE.load(Ordering::Relaxed)
            || PIPELINE_STAGE.load(Ordering::Relaxed) != PROVIDER_ACTIVE
        {
            return Err(SpeechProviderError::Failed);
        }
        let mut digest = 0_usize;
        for byte in bytes {
            digest = digest.wrapping_mul(16_777_619) ^ usize::from(*byte);
        }
        FORWARDED_BYTES.store(bytes.len(), Ordering::Relaxed);
        PCM_DIGEST.store(digest, Ordering::Relaxed);
        Ok(())
    }

    fn finish(&mut self, _transcript: &mut [u8]) -> Result<usize, SpeechProviderError> {
        if PIPELINE_STAGE
            .compare_exchange(
                PROVIDER_ACTIVE,
                PROVIDER_FINISHED,
                Ordering::Relaxed,
                Ordering::Relaxed,
            )
            .is_err()
        {
            return Err(SpeechProviderError::Failed);
        }
        // This fixture verifies the target orchestration only. It deliberately
        // provides no transcript and does not represent STT model inference.
        Err(SpeechProviderError::Unavailable)
    }

    fn cancel(&mut self) {
        PROVIDER_CANCELS.fetch_add(1, Ordering::Relaxed);
    }
}

struct FixtureTtsProvider {
    next_chunk: usize,
}

impl TextToSpeechProvider for FixtureTtsProvider {
    fn begin(
        &mut self,
        text: &str,
        options: SpeechSynthesisOptions,
    ) -> Result<(), SpeechProviderError> {
        if text != "こんにちは"
            || options.language != SpeechSynthesisLanguage::Japanese
            || options.pcm_format != PcmFormat::stereo_48khz()
        {
            return Err(SpeechProviderError::Failed);
        }
        self.next_chunk = 0;
        Ok(())
    }

    fn next_pcm_chunk(
        &mut self,
        destination: &mut [u8],
    ) -> Result<SynthesisPcmChunk, SpeechProviderError> {
        let Some(chunk) = FIXTURE_TTS_PCM.get(self.next_chunk) else {
            return Ok(SynthesisPcmChunk::End);
        };
        if destination.len() < chunk.len() {
            return Err(SpeechProviderError::OutputTooSmall);
        }
        destination[..chunk.len()].copy_from_slice(chunk);
        self.next_chunk += 1;
        Ok(SynthesisPcmChunk::Data(chunk.len()))
    }

    fn cancel(&mut self) {
        TTS_PROVIDER_CANCELS.fetch_add(1, Ordering::Relaxed);
    }
}

struct FixtureAudioSink;

impl SpeechPlaybackSink for FixtureAudioSink {
    fn play_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), PlaybackError> {
        // `usize::is_multiple_of` is not available on the pinned nightly.
        #[allow(unknown_lints, clippy::manual_is_multiple_of)]
        if format != PcmFormat::stereo_48khz() || bytes.is_empty() || bytes.len() % 4 != 0 {
            return Err(PlaybackError::Unavailable);
        }
        let mut digest = TTS_PCM_DIGEST.load(Ordering::Relaxed);
        for byte in bytes {
            digest = digest.wrapping_mul(16_777_619) ^ usize::from(*byte);
        }
        TTS_PCM_DIGEST.store(digest, Ordering::Relaxed);
        TTS_PLAYBACK_BYTES.fetch_add(bytes.len(), Ordering::Relaxed);
        TTS_PLAYBACK_CHUNKS.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

type FixtureService =
    PushToTalkService<FixtureCapture, FixtureAuthority, FixtureIndicator, FixtureProvider>;

fn service(allow: bool) -> FixtureService {
    PushToTalkService::new(
        FixtureCapture,
        FixtureAuthority { allow },
        FixtureIndicator,
        FixtureProvider,
    )
}

fn marker(message: &[u8]) -> bool {
    libnagi::console_write(message) == message.len()
}

pub fn run() -> bool {
    PIPELINE_STAGE.store(PIPELINE_IDLE, Ordering::Relaxed);
    INDICATOR_VISIBLE.store(false, Ordering::Relaxed);
    CAPTURE_CALLS.store(0, Ordering::Relaxed);
    FORWARDED_BYTES.store(0, Ordering::Relaxed);
    PCM_DIGEST.store(0, Ordering::Relaxed);
    PROVIDER_CANCELS.store(0, Ordering::Relaxed);

    if !marker(b"Nagi M25 voice fixture START\r\n") {
        return false;
    }
    let mut denied = service(false);
    if denied.begin(SpeechConsumer::NagiBar, SpeechLanguage::Japanese)
        != Err(SpeechError::PermissionDenied)
        || PIPELINE_STAGE.load(Ordering::Relaxed) != PIPELINE_IDLE
        || INDICATOR_VISIBLE.load(Ordering::Relaxed)
        || CAPTURE_CALLS.load(Ordering::Relaxed) != 0
    {
        return false;
    }
    if !marker(b"Nagi M25 permission fail-closed PASS\r\n") {
        return false;
    }

    let mut service = service(true);
    if service
        .begin(SpeechConsumer::NagiBar, SpeechLanguage::Japanese)
        .is_err()
        || PIPELINE_STAGE.load(Ordering::Relaxed) != PROVIDER_ACTIVE
        || !INDICATOR_VISIBLE.load(Ordering::Relaxed)
    {
        return false;
    }
    if !marker(b"Nagi M25 indicator-before-provider PASS\r\n") {
        return false;
    }

    if service.capture_next_chunk() != Ok(FIXTURE_PCM.len())
        || CAPTURE_CALLS.load(Ordering::Relaxed) != 1
        || FORWARDED_BYTES.load(Ordering::Relaxed) != FIXTURE_PCM.len()
        || PCM_DIGEST.load(Ordering::Relaxed) == 0
        || !INDICATOR_VISIBLE.load(Ordering::Relaxed)
    {
        return false;
    }
    if !marker(b"Nagi M25 bounded PCM forwarding PASS\r\n") {
        return false;
    }

    let mut transcript = [0xa5; 32];
    if service.finish_into(&mut transcript) != Err(SpeechError::ProviderUnavailable)
        || transcript.iter().any(|byte| *byte != 0)
        || INDICATOR_VISIBLE.load(Ordering::Relaxed)
        || PIPELINE_STAGE.load(Ordering::Relaxed) != CLEANED_UP
        || PROVIDER_CANCELS.load(Ordering::Relaxed) != 1
    {
        return false;
    }
    if !marker(b"Nagi M25 unavailable cleanup PASS\r\n") {
        return false;
    }

    TTS_PLAYBACK_CHUNKS.store(0, Ordering::Relaxed);
    TTS_PLAYBACK_BYTES.store(0, Ordering::Relaxed);
    TTS_PCM_DIGEST.store(0, Ordering::Relaxed);
    TTS_PROVIDER_CANCELS.store(0, Ordering::Relaxed);
    let mut synthesis =
        SpeechSynthesisService::new(FixtureTtsProvider { next_chunk: 0 }, FixtureAudioSink);
    if synthesis.speak(
        "こんにちは".as_bytes(),
        SpeechSynthesisOptions {
            language: SpeechSynthesisLanguage::Japanese,
            pcm_format: PcmFormat::stereo_48khz(),
        },
    ) != Ok(FIXTURE_TTS_PCM.len() * FIXTURE_TTS_PCM[0].len())
        || TTS_PLAYBACK_CHUNKS.load(Ordering::Relaxed) != FIXTURE_TTS_PCM.len()
        || TTS_PLAYBACK_BYTES.load(Ordering::Relaxed)
            != FIXTURE_TTS_PCM.len() * FIXTURE_TTS_PCM[0].len()
        || TTS_PCM_DIGEST.load(Ordering::Relaxed) == 0
        || TTS_PROVIDER_CANCELS.load(Ordering::Relaxed) != 0
    {
        return false;
    }
    marker(b"Nagi M25 TTS provider contract PASS\r\n")
}
