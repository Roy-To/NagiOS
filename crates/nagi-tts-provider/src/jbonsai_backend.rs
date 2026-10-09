//! Japanese HTS backend: jpreprocess (Open JTalk front end, naist-jdic) feeding
//! jbonsai (hts_engine API rewrite) with an `.htsvoice` model.
//!
//! The voice and dictionary are pinned by `tools/tts/tts-artifacts.lock` and
//! fetched by `tools/tts/fetch.sh`. Nothing is downloaded at build time.

use std::path::Path;

use jbonsai::speech::SpeechGenerator;
use jbonsai::Engine;
use jpreprocess::{DefaultTokenizer, JPreprocess, SystemDictionaryConfig};
use nagi_audio::speech::SpeechSynthesisLanguage;

use crate::{
    BackendError, FrameSource, LocalTtsProvider, SynthesisBackend, ENGINE_SAMPLE_RATE,
    MAX_ENGINE_FRAME_SAMPLES,
};

/// Reasons a voice or dictionary could not be loaded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoadError {
    VoiceMissing,
    VoiceInvalid,
    DictionaryMissing,
    DictionaryInvalid,
    UnsupportedSampleRate,
    UnsupportedFramePeriod,
}

/// Upper bound accepted for an `.htsvoice` file. The pinned voice is
/// 2,154,716 bytes; the bound rejects unexpected inputs before parsing.
pub const MAX_VOICE_BYTES: usize = 16 * 1024 * 1024;

/// The loaded front end and voice. Both persist across utterances until
/// [`JbonsaiBackend::unload`] is called.
pub struct JbonsaiBackend {
    loaded: Option<Loaded>,
    frame_len: usize,
}

struct Loaded {
    engine: Engine,
    frontend: JPreprocess<DefaultTokenizer>,
}

/// One utterance being vocoded frame by frame.
pub struct JbonsaiSource {
    generator: SpeechGenerator,
}

impl FrameSource for JbonsaiSource {
    fn next_frame(&mut self, out: &mut [f64]) -> Result<usize, BackendError> {
        if out.len() < self.generator.fperiod() {
            return Err(BackendError::Failed);
        }
        Ok(self.generator.generate_step(out))
    }
}

impl JbonsaiBackend {
    /// Loads the voice from a file and the dictionary from a jpreprocess
    /// dictionary directory.
    pub fn load(voice: &Path, dictionary: &Path) -> Result<Self, LoadError> {
        let voice_bytes = read_voice(voice)?;
        Self::from_voice_bytes(&voice_bytes, dictionary)
    }

    /// Loads the voice from bytes already obtained through a read-only model
    /// capability, plus the dictionary directory.
    pub fn from_voice_bytes(voice: &[u8], dictionary: &Path) -> Result<Self, LoadError> {
        if voice.is_empty() {
            return Err(LoadError::VoiceMissing);
        }
        if voice.len() > MAX_VOICE_BYTES {
            return Err(LoadError::VoiceInvalid);
        }
        let engine = Engine::load_from_bytes([voice]).map_err(|_| LoadError::VoiceInvalid)?;
        let rate = engine.condition.get_sampling_frequency();
        if rate != ENGINE_SAMPLE_RATE as usize {
            return Err(LoadError::UnsupportedSampleRate);
        }
        let frame_len = engine.condition.get_fperiod();
        if frame_len == 0 || frame_len > MAX_ENGINE_FRAME_SAMPLES {
            return Err(LoadError::UnsupportedFramePeriod);
        }
        let frontend = load_frontend(dictionary)?;
        Ok(Self {
            loaded: Some(Loaded { engine, frontend }),
            frame_len,
        })
    }

    /// Drops the voice and dictionary (memory-pressure unload). Subsequent
    /// `begin` calls report `Unavailable` until [`Self::reload`] succeeds.
    pub fn unload(&mut self) {
        self.loaded = None;
    }

    /// Reloads after [`Self::unload`]. The new voice must keep the frame
    /// period the provider was created with.
    pub fn reload(&mut self, voice: &Path, dictionary: &Path) -> Result<(), LoadError> {
        let fresh = Self::load(voice, dictionary)?;
        if fresh.frame_len != self.frame_len {
            return Err(LoadError::UnsupportedFramePeriod);
        }
        self.loaded = fresh.loaded;
        Ok(())
    }

    /// Runs only the text front end and returns the number of full-context
    /// labels. Diagnostic helper for tests and measurements.
    pub fn label_count(&self, text: &str) -> Result<usize, BackendError> {
        let loaded = self.loaded.as_ref().ok_or(BackendError::Unavailable)?;
        loaded
            .frontend
            .extract_fullcontext(text)
            .map(|labels| labels.len())
            .map_err(|_| BackendError::Failed)
    }
}

fn read_voice(path: &Path) -> Result<Vec<u8>, LoadError> {
    let metadata = std::fs::metadata(path).map_err(|_| LoadError::VoiceMissing)?;
    if !metadata.is_file() {
        return Err(LoadError::VoiceMissing);
    }
    if metadata.len() > MAX_VOICE_BYTES as u64 {
        return Err(LoadError::VoiceInvalid);
    }
    std::fs::read(path).map_err(|_| LoadError::VoiceMissing)
}

fn load_frontend(dictionary: &Path) -> Result<JPreprocess<DefaultTokenizer>, LoadError> {
    if !dictionary.is_dir() {
        return Err(LoadError::DictionaryMissing);
    }
    // lindera reads `metadata.json` first; a directory without it is not a
    // dictionary.
    if !dictionary.join("metadata.json").is_file() {
        return Err(LoadError::DictionaryMissing);
    }
    let system = SystemDictionaryConfig::File(dictionary.to_path_buf())
        .load()
        .map_err(|_| LoadError::DictionaryInvalid)?;
    Ok(JPreprocess::with_dictionaries(system, None))
}

fn is_silence(label: &jlabel::Label) -> bool {
    matches!(label.phoneme.c.as_deref(), None | Some("sil") | Some("pau"))
}

impl SynthesisBackend for JbonsaiBackend {
    type Source = JbonsaiSource;

    fn sample_rate(&self) -> u32 {
        ENGINE_SAMPLE_RATE
    }

    fn frame_len(&self) -> usize {
        self.frame_len
    }

    fn is_loaded(&self) -> bool {
        self.loaded.is_some()
    }

    fn start(
        &mut self,
        text: &str,
        language: SpeechSynthesisLanguage,
    ) -> Result<Option<Self::Source>, BackendError> {
        match language {
            SpeechSynthesisLanguage::Japanese | SpeechSynthesisLanguage::Auto => {}
            // The pinned voice is Japanese only; never pretend to speak
            // English with it.
            SpeechSynthesisLanguage::English => return Err(BackendError::UnsupportedLanguage),
        }
        let loaded = self.loaded.as_ref().ok_or(BackendError::Unavailable)?;
        let labels = loaded
            .frontend
            .extract_fullcontext(text)
            .map_err(|_| BackendError::Failed)?;
        if labels.iter().all(is_silence) {
            return Ok(None);
        }
        let generator = loaded
            .engine
            .generator(labels)
            .map_err(|_| BackendError::Failed)?;
        if generator.fperiod() != self.frame_len {
            return Err(BackendError::Failed);
        }
        Ok(Some(JbonsaiSource { generator }))
    }
}

/// The M25 Japanese TTS provider: [`LocalTtsProvider`] over [`JbonsaiBackend`].
pub type JbonsaiProvider = LocalTtsProvider<JbonsaiBackend>;

/// Loads the pinned voice and dictionary and wraps them in a provider.
pub fn load_provider(voice: &Path, dictionary: &Path) -> Result<JbonsaiProvider, LoadError> {
    let backend = JbonsaiBackend::load(voice, dictionary)?;
    LocalTtsProvider::new(backend).map_err(|_| LoadError::UnsupportedFramePeriod)
}
