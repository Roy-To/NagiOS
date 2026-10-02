use alloc::vec::Vec;
use core::ffi::c_void;

use nagi_audio::{
    speech::{
        SpeechLanguage, SpeechOptions, SpeechProviderError, SpeechToTextProvider,
        MAX_SPEECH_PCM_CHUNK_BYTES, MAX_SPEECH_TRANSCRIPT_BYTES, MAX_SPEECH_UTTERANCE_BYTES,
    },
    PcmFormat,
};
use nagi_model_manager::{ArtifactId, Fat32ArtifactReader, ModelArtifactReader};

use super::SyscallModelStoreReader;

const MODEL_STORE_SECTORS: u64 = 67_108_864;
const WHISPER_MODEL_ID: &str = "openai.whisper-small-multilingual";
const WHISPER_MODEL_BYTES: u64 = 487_601_967;
// The pinned Nagi patch keeps whisper.cpp's CPU path synchronous until the
// target exposes a worker-pool capability.
const WHISPER_THREADS: i32 = 1;
const MAX_WHISPER_SAMPLES: usize = MAX_SPEECH_UTTERANCE_BYTES / 2;

const PCM_FIXTURE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/m25-whisper-input.pcm"));
const EXPECTED_TEXT: &str = include_str!(concat!(env!("OUT_DIR"), "/m25-whisper-expected.txt"));

type WhisperReadCallback = unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> usize;
type WhisperEofCallback = unsafe extern "C" fn(*mut c_void) -> bool;
type WhisperCloseCallback = unsafe extern "C" fn(*mut c_void);

unsafe extern "C" {
    fn nagi_m25_whisper_init(
        reader_context: *mut c_void,
        read_callback: WhisperReadCallback,
        eof_callback: WhisperEofCallback,
        close_callback: WhisperCloseCallback,
    ) -> *mut c_void;
    fn nagi_m25_whisper_transcribe(
        context: *mut c_void,
        samples: *const f32,
        sample_count: i32,
        thread_count: i32,
        language: *const core::ffi::c_char,
        destination: *mut i8,
        destination_size: usize,
        written: *mut usize,
    ) -> i32;
    fn nagi_m25_whisper_free(context: *mut c_void);
}

struct GuestWhisperModelReader {
    artifact: Fat32ArtifactReader<SyscallModelStoreReader>,
    offset: u64,
    failed: bool,
}

unsafe extern "C" fn model_read(
    context: *mut c_void,
    destination: *mut c_void,
    read_size: usize,
) -> usize {
    if context.is_null() || (destination.is_null() && read_size != 0) {
        return 0;
    }
    let reader = unsafe { &mut *context.cast::<GuestWhisperModelReader>() };
    if reader.failed || read_size == 0 {
        return 0;
    }
    let remaining = ModelArtifactReader::len(&reader.artifact).saturating_sub(reader.offset);
    let wanted = read_size.min(usize::try_from(remaining).unwrap_or(usize::MAX));
    if wanted == 0 {
        return 0;
    }
    let output = unsafe { core::slice::from_raw_parts_mut(destination.cast::<u8>(), wanted) };
    match ModelArtifactReader::read_at(&mut reader.artifact, reader.offset, output) {
        Ok(read) if read != 0 && read <= wanted => {
            reader.offset = reader.offset.saturating_add(read as u64);
            read
        }
        _ => {
            reader.failed = true;
            0
        }
    }
}

unsafe extern "C" fn model_eof(context: *mut c_void) -> bool {
    if context.is_null() {
        return true;
    }
    let reader = unsafe { &*context.cast::<GuestWhisperModelReader>() };
    reader.offset >= ModelArtifactReader::len(&reader.artifact)
}

unsafe extern "C" fn model_close(_context: *mut c_void) {}

struct WhisperSpeechToTextProvider {
    context: *mut c_void,
    samples: Vec<f32>,
    active: bool,
    automatic_language: bool,
}

impl WhisperSpeechToTextProvider {
    fn load(model_store_capability: u64) -> Option<Self> {
        let artifact_id = ArtifactId::new(WHISPER_MODEL_ID).ok()?;
        let artifact = Fat32ArtifactReader::open(
            SyscallModelStoreReader(model_store_capability),
            MODEL_STORE_SECTORS,
            artifact_id,
        )
        .ok()?;
        if ModelArtifactReader::len(&artifact) != WHISPER_MODEL_BYTES {
            return None;
        }
        let mut reader = GuestWhisperModelReader {
            artifact,
            offset: 0,
            failed: false,
        };
        let context = unsafe {
            nagi_m25_whisper_init(
                (&mut reader as *mut GuestWhisperModelReader).cast(),
                model_read,
                model_eof,
                model_close,
            )
        };
        if context.is_null() || reader.failed {
            if !context.is_null() {
                unsafe { nagi_m25_whisper_free(context) };
            }
            return None;
        }
        Some(Self {
            context,
            samples: Vec::new(),
            active: false,
            automatic_language: false,
        })
    }

    fn clear_samples(&mut self) {
        self.samples.fill(0.0);
        self.samples.clear();
        self.active = false;
    }
}

impl SpeechToTextProvider for WhisperSpeechToTextProvider {
    fn begin(&mut self, options: SpeechOptions) -> Result<(), SpeechProviderError> {
        if self.context.is_null() || self.active || options.pcm_format != PcmFormat::mono_16khz() {
            return Err(SpeechProviderError::Failed);
        }
        self.clear_samples();
        self.automatic_language = matches!(options.language, SpeechLanguage::Auto);
        self.active = true;
        Ok(())
    }

    fn push_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), SpeechProviderError> {
        if !self.active
            || format != PcmFormat::mono_16khz()
            || bytes.is_empty()
            || bytes.len() > MAX_SPEECH_PCM_CHUNK_BYTES
            || bytes.len() & 1 != 0
        {
            return Err(SpeechProviderError::Failed);
        }
        let next_len = self
            .samples
            .len()
            .checked_add(bytes.len() / 2)
            .filter(|length| *length <= MAX_WHISPER_SAMPLES)
            .ok_or(SpeechProviderError::Failed)?;
        self.samples
            .try_reserve(next_len - self.samples.len())
            .map_err(|_| SpeechProviderError::Unavailable)?;
        for sample in bytes.chunks_exact(2) {
            self.samples
                .push(f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32768.0);
        }
        Ok(())
    }

    fn finish(&mut self, transcript: &mut [u8]) -> Result<usize, SpeechProviderError> {
        transcript.fill(0);
        if !self.active || self.samples.is_empty() {
            self.clear_samples();
            return Err(SpeechProviderError::Failed);
        }
        let language = if self.automatic_language {
            b"auto\0".as_ptr().cast::<core::ffi::c_char>()
        } else {
            b"ja\0".as_ptr().cast::<core::ffi::c_char>()
        };
        let mut written = 0;
        let status = unsafe {
            nagi_m25_whisper_transcribe(
                self.context,
                self.samples.as_ptr(),
                self.samples.len() as i32,
                WHISPER_THREADS,
                language,
                transcript.as_mut_ptr().cast(),
                transcript.len(),
                &mut written,
            )
        };
        self.clear_samples();
        match status {
            0 if written != 0 && written <= transcript.len() => {
                if core::str::from_utf8(&transcript[..written]).is_ok() {
                    Ok(written)
                } else {
                    transcript.fill(0);
                    Err(SpeechProviderError::Failed)
                }
            }
            4 => {
                transcript.fill(0);
                Err(SpeechProviderError::OutputTooSmall)
            }
            _ => {
                transcript.fill(0);
                Err(SpeechProviderError::Failed)
            }
        }
    }

    fn cancel(&mut self) {
        self.clear_samples();
    }
}

impl Drop for WhisperSpeechToTextProvider {
    fn drop(&mut self) {
        self.clear_samples();
        if !self.context.is_null() {
            unsafe { nagi_m25_whisper_free(self.context) };
            self.context = core::ptr::null_mut();
        }
    }
}

pub fn run(model_store_capability: u64) -> bool {
    if model_store_capability == 0
        || PCM_FIXTURE.is_empty()
        || PCM_FIXTURE.len() > MAX_SPEECH_UTTERANCE_BYTES
        || PCM_FIXTURE.len() & 1 != 0
        || EXPECTED_TEXT.is_empty()
        || EXPECTED_TEXT.len() > MAX_SPEECH_TRANSCRIPT_BYTES
        || relibc::nagi_backend_probe() != 0x4e41_4749
    {
        return false;
    }
    let Some(mut provider) = WhisperSpeechToTextProvider::load(model_store_capability) else {
        return false;
    };
    if provider
        .begin(SpeechOptions {
            language: SpeechLanguage::Japanese,
            pcm_format: PcmFormat::mono_16khz(),
        })
        .is_err()
    {
        return false;
    }
    for chunk in PCM_FIXTURE.chunks(MAX_SPEECH_PCM_CHUNK_BYTES) {
        if provider.push_pcm(PcmFormat::mono_16khz(), chunk).is_err() {
            provider.cancel();
            return false;
        }
    }
    let mut transcript = [0_u8; MAX_SPEECH_TRANSCRIPT_BYTES];
    let Ok(written) = provider.finish(&mut transcript) else {
        return false;
    };
    core::str::from_utf8(&transcript[..written])
        .is_ok_and(|recognized| recognized.contains(EXPECTED_TEXT))
}
