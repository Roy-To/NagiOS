//! Japanese HTS backend: jpreprocess (Open JTalk front end, naist-jdic) feeding
//! jbonsai (hts_engine API rewrite) with an `.htsvoice` model.
//!
//! The voice and dictionary are pinned by `tools/tts/tts-artifacts.lock` and
//! fetched by `tools/tts/fetch.sh`. Nothing is downloaded at build time.

use std::path::Path;

use jbonsai::speech::SpeechGenerator;
use jbonsai::Engine;
use jpreprocess::{DefaultTokenizer, Dictionary, JPreprocess};
use lindera_dictionary::dictionary::character_definition::CharacterDefinition;
use lindera_dictionary::dictionary::connection_cost_matrix::ConnectionCostMatrix;
use lindera_dictionary::dictionary::metadata::Metadata;
use lindera_dictionary::dictionary::prefix_dictionary::PrefixDictionary;
use lindera_dictionary::dictionary::unknown_dictionary::UnknownDictionary;
use nagi_audio::speech::SpeechSynthesisLanguage;

use crate::{
    BackendError, FrameSource, LocalTtsProvider, SilenceTrim, SilenceTrimmer, SynthesisBackend,
    ENGINE_SAMPLE_RATE, MAX_ENGINE_FRAME_SAMPLES,
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

/// Upper bound accepted for any single dictionary component. The largest
/// pinned component (`dict.da`) is 33,049,069 bytes.
pub const MAX_DICTIONARY_COMPONENT_BYTES: usize = 64 * 1024 * 1024;

/// The eight files of a jpreprocess/lindera system dictionary, as bytes.
/// This is the form a read-only Model Store capability can deliver on Nagi;
/// the path loader reads the same files into this struct first.
#[derive(Default)]
pub struct DictionaryBytes {
    pub metadata_json: Vec<u8>,
    pub char_def_bin: Vec<u8>,
    pub matrix_mtx: Vec<u8>,
    pub dict_da: Vec<u8>,
    pub dict_vals: Vec<u8>,
    pub dict_wordsidx: Vec<u8>,
    pub dict_words: Vec<u8>,
    pub unk_bin: Vec<u8>,
}

impl DictionaryBytes {
    /// File names inside the dictionary directory, in field order.
    pub const FILES: [&'static str; 8] = [
        "metadata.json",
        "char_def.bin",
        "matrix.mtx",
        "dict.da",
        "dict.vals",
        "dict.wordsidx",
        "dict.words",
        "unk.bin",
    ];

    /// Reads the eight dictionary files from a directory.
    pub fn read_dir(dictionary: &Path) -> Result<Self, LoadError> {
        if !dictionary.is_dir() {
            return Err(LoadError::DictionaryMissing);
        }
        let mut parts: [Vec<u8>; 8] = Default::default();
        for (slot, name) in parts.iter_mut().zip(Self::FILES) {
            let path = dictionary.join(name);
            let metadata = std::fs::metadata(&path).map_err(|_| LoadError::DictionaryMissing)?;
            if !metadata.is_file() {
                return Err(LoadError::DictionaryMissing);
            }
            if metadata.len() > MAX_DICTIONARY_COMPONENT_BYTES as u64 {
                return Err(LoadError::DictionaryInvalid);
            }
            *slot = std::fs::read(&path).map_err(|_| LoadError::DictionaryMissing)?;
        }
        let [metadata_json, char_def_bin, matrix_mtx, dict_da, dict_vals, dict_wordsidx, dict_words, unk_bin] =
            parts;
        Ok(Self {
            metadata_json,
            char_def_bin,
            matrix_mtx,
            dict_da,
            dict_vals,
            dict_wordsidx,
            dict_words,
            unk_bin,
        })
    }

    fn sizes(&self) -> [usize; 8] {
        [
            self.metadata_json.len(),
            self.char_def_bin.len(),
            self.matrix_mtx.len(),
            self.dict_da.len(),
            self.dict_vals.len(),
            self.dict_wordsidx.len(),
            self.dict_words.len(),
            self.unk_bin.len(),
        ]
    }

    /// Total bytes held.
    pub fn total_len(&self) -> usize {
        self.sizes().iter().sum()
    }

    fn into_dictionary(self) -> Result<Dictionary, LoadError> {
        let sizes = self.sizes();
        if sizes.contains(&0) {
            return Err(LoadError::DictionaryMissing);
        }
        if sizes
            .iter()
            .any(|size| *size > MAX_DICTIONARY_COMPONENT_BYTES)
        {
            return Err(LoadError::DictionaryInvalid);
        }
        let invalid = |_| LoadError::DictionaryInvalid;
        let metadata = Metadata::load(&self.metadata_json).map_err(invalid)?;
        // Both loaders copy into a 16-byte-aligned buffer before rkyv access.
        let character_definition =
            CharacterDefinition::load(&self.char_def_bin).map_err(invalid)?;
        let unknown_dictionary = UnknownDictionary::load(&self.unk_bin).map_err(invalid)?;
        let connection_cost_matrix =
            ConnectionCostMatrix::load(self.matrix_mtx).map_err(invalid)?;
        let prefix_dictionary = PrefixDictionary::load(
            self.dict_da,
            self.dict_vals,
            self.dict_wordsidx,
            self.dict_words,
            true,
        )
        .map_err(invalid)?;
        Ok(Dictionary {
            prefix_dictionary,
            connection_cost_matrix,
            character_definition,
            unknown_dictionary,
            metadata,
        })
    }
}

/// The loaded front end and voice. Both persist across utterances until
/// [`JbonsaiBackend::unload`] is called.
pub struct JbonsaiBackend {
    loaded: Option<Loaded>,
    frame_len: usize,
    trim: SilenceTrim,
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
        let dictionary = DictionaryBytes::read_dir(dictionary)?;
        Self::from_bytes(&voice_bytes, dictionary)
    }

    /// Loads the voice and dictionary from bytes already obtained through a
    /// read-only model capability. No filesystem path is involved; this is
    /// the loader a Nagi guest uses.
    pub fn from_bytes(voice: &[u8], dictionary: DictionaryBytes) -> Result<Self, LoadError> {
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
        let frontend = JPreprocess::with_dictionaries(dictionary.into_dictionary()?, None);
        Ok(Self {
            loaded: Some(Loaded { engine, frontend }),
            frame_len,
            trim: SilenceTrim::HTS_48K,
        })
    }

    /// Sets edge-silence trimming. The default, [`SilenceTrim::HTS_48K`],
    /// removes the ~0.5 s of HTS `sil` before and after each utterance so
    /// more of the 1 MiB output budget carries speech.
    pub fn set_silence_trim(&mut self, trim: SilenceTrim) {
        self.trim = trim;
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

fn is_silence(label: &jlabel::Label) -> bool {
    matches!(label.phoneme.c.as_deref(), None | Some("sil") | Some("pau"))
}

impl SynthesisBackend for JbonsaiBackend {
    type Source = SilenceTrimmer<JbonsaiSource>;

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
        Ok(Some(SilenceTrimmer::new(
            JbonsaiSource { generator },
            self.frame_len,
            self.trim,
        )))
    }
}

/// The M25 Japanese TTS provider: [`LocalTtsProvider`] over [`JbonsaiBackend`].
pub type JbonsaiProvider = LocalTtsProvider<JbonsaiBackend>;

/// Loads the pinned voice and dictionary and wraps them in a provider.
pub fn load_provider(voice: &Path, dictionary: &Path) -> Result<JbonsaiProvider, LoadError> {
    let backend = JbonsaiBackend::load(voice, dictionary)?;
    LocalTtsProvider::new(backend).map_err(|_| LoadError::UnsupportedFramePeriod)
}
