// M25 Whisper speech-to-text provider for Nagi.
//
// Production path: `ModelStoreWhisperLoader` opens the locked Whisper Small
// artifact through the read-only Model Store capability and hands it to the
// shared provider lifecycle (`tools/whisper/provider_session.rs`), which owns
// utterance buffering, cancel, unload, and failure recovery. Nothing in the
// production path reads fixture input or expected text.
//
// Fixture path: `fixture_acceptance` is the only place that embeds the
// build-staged PCM sample and its expected text. Its result is reported as a
// fixture check only and is not evidence of recognition on unseen speech.

use core::ffi::c_void;

use nagi_model_manager::{ArtifactId, Fat32ArtifactReader, ModelArtifactReader};

use super::SyscallModelStoreReader;

// Shared lifecycle; the guest acceptance exercises a subset of its API.
#[allow(dead_code)]
#[path = "../../../tools/whisper/provider_session.rs"]
mod provider_session;

#[path = "../../../tools/whisper/provider_ffi.rs"]
mod provider_ffi;

use provider_ffi::AdapterEngine;
use provider_session::{WhisperEngineLoader, WhisperSession};

const MODEL_STORE_SECTORS: u64 = 67_108_864;
const WHISPER_MODEL_ID: &str = "openai.whisper-small-multilingual";
const WHISPER_MODEL_BYTES: u64 = 487_601_967;

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

/// Opens the locked Whisper artifact through the Model Store capability each
/// time the session needs a fresh context.
struct ModelStoreWhisperLoader {
    model_store_capability: u64,
}

impl WhisperEngineLoader for ModelStoreWhisperLoader {
    type Engine = AdapterEngine;

    fn load(&mut self) -> Option<AdapterEngine> {
        if self.model_store_capability == 0 {
            return None;
        }
        let artifact_id = ArtifactId::new(WHISPER_MODEL_ID).ok()?;
        let artifact = Fat32ArtifactReader::open(
            SyscallModelStoreReader(self.model_store_capability),
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
        let engine = unsafe {
            AdapterEngine::init(
                (&mut reader as *mut GuestWhisperModelReader).cast(),
                model_read,
                model_eof,
                model_close,
            )
        };
        // A context built from a short or failed read is dropped (freed) here.
        if reader.failed {
            return None;
        }
        engine
    }
}

/// The production Nagi Whisper provider.
type WhisperSpeechToTextProvider = WhisperSession<ModelStoreWhisperLoader>;

fn provider(model_store_capability: u64) -> WhisperSpeechToTextProvider {
    WhisperSession::new(ModelStoreWhisperLoader {
        model_store_capability,
    })
}

/// Build-staged fixture check. The PCM sample and expected text exist only in
/// this module.
mod fixture_acceptance {
    use nagi_audio::{
        speech::{
            SpeechLanguage, SpeechOptions, SpeechToTextProvider, MAX_SPEECH_PCM_CHUNK_BYTES,
            MAX_SPEECH_TRANSCRIPT_BYTES, MAX_SPEECH_UTTERANCE_BYTES,
        },
        PcmFormat,
    };

    use super::provider_session::WhisperSessionState;

    const PCM_FIXTURE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/m25-whisper-input.pcm"));
    const EXPECTED_TEXT: &str = include_str!(concat!(env!("OUT_DIR"), "/m25-whisper-expected.txt"));

    // The build script already rejects out-of-range fixtures; keep the
    // provider limits as a compile-time invariant of this module.
    // Evaluating these at compile time is the point of the assertion.
    #[allow(clippy::const_is_empty)]
    const _: () = assert!(
        !PCM_FIXTURE.is_empty()
            && PCM_FIXTURE.len() <= MAX_SPEECH_UTTERANCE_BYTES
            && PCM_FIXTURE.len().is_multiple_of(2)
            && !EXPECTED_TEXT.is_empty()
            && EXPECTED_TEXT.len() <= MAX_SPEECH_TRANSCRIPT_BYTES
    );

    const OPTIONS: SpeechOptions = SpeechOptions {
        language: SpeechLanguage::Japanese,
        pcm_format: PcmFormat::mono_16khz(),
    };

    pub fn run(model_store_capability: u64) -> bool {
        if model_store_capability == 0 || relibc::nagi_backend_probe() != 0x4e41_4749 {
            return false;
        }
        let mut provider = super::provider(model_store_capability);
        if provider.load().is_err() || provider.begin(OPTIONS).is_err() {
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
        if !core::str::from_utf8(&transcript[..written])
            .is_ok_and(|recognized| recognized.contains(EXPECTED_TEXT))
        {
            return false;
        }

        // Lifecycle checks that need no second inference: a cancelled
        // utterance leaves the loaded engine idle with no buffered PCM, and
        // unload releases the context so capture cannot begin until reload.
        let first_chunk = &PCM_FIXTURE[..PCM_FIXTURE.len().min(MAX_SPEECH_PCM_CHUNK_BYTES)];
        if provider.begin(OPTIONS).is_err()
            || provider
                .push_pcm(PcmFormat::mono_16khz(), first_chunk)
                .is_err()
        {
            return false;
        }
        provider.cancel();
        if provider.state() != WhisperSessionState::Idle || provider.buffered_samples() != 0 {
            return false;
        }
        provider.unload();
        let stats = provider.stats();
        provider.state() == WhisperSessionState::Unloaded
            && provider.begin(OPTIONS).is_err()
            && stats.loads == 1
            && stats.unloads == 1
            && stats.utterances_completed == 1
            && stats.utterances_cancelled == 1
            && stats.utterances_failed == 0
    }
}

/// Runs the build-staged fixture acceptance. A `true` result is a fixture
/// check, not recognition evidence on unseen speech.
pub fn run(model_store_capability: u64) -> bool {
    fixture_acceptance::run(model_store_capability)
}
