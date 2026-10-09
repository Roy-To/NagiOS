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

use jbonsai::duration::DurationEstimator;
use jbonsai::label::ToLabels;
use jbonsai::model::Models;

use crate::{
    BackendError, FrameSource, LocalTtsProvider, SynthesisBackend, ENGINE_SAMPLE_RATE,
    MAX_ENGINE_FRAME_SAMPLES,
};

/// Edge-silence trimming, in engine frames. Frames belonging to the leading
/// and trailing silence labels (`sil`/`pau`) are vocoded (the vocoder filter
/// state must advance) but not emitted, except `pad_frames` next to speech.
/// Internal pauses are kept. Because trimming is decided from the predicted
/// per-label durations, the emitted length is known before synthesis.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SilenceTrim {
    pub enabled: bool,
    pub pad_frames: usize,
}

impl SilenceTrim {
    /// 50 ms of padding at 5 ms HTS frames.
    pub const HTS_48K: Self = Self {
        enabled: true,
        pad_frames: 10,
    };
    /// Emit every frame.
    pub const DISABLED: Self = Self {
        enabled: false,
        pad_frames: 0,
    };
}

/// Length prediction made by [`JbonsaiBackend::plan`] from the label
/// durations, before any acoustic parameters are generated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UtterancePlan {
    /// Number of full-context labels from the text front end.
    pub labels: usize,
    /// Frames the engine will vocode.
    pub total_frames: usize,
    /// First emitted frame index.
    pub first_frame: usize,
    /// One past the last emitted frame index.
    pub end_frame: usize,
}

impl UtterancePlan {
    /// Frames the provider will emit.
    pub fn emitted_frames(&self) -> usize {
        self.end_frame - self.first_frame
    }
}

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

/// One utterance being vocoded frame by frame. Only frames in
/// `[first_frame, end_frame)` are emitted.
pub struct JbonsaiSource {
    generator: SpeechGenerator,
    next: usize,
    first_frame: usize,
    end_frame: usize,
    total_frames: usize,
}

impl FrameSource for JbonsaiSource {
    fn next_frame(&mut self, out: &mut [f64]) -> Result<usize, BackendError> {
        let frame = self.generator.fperiod();
        if out.len() < frame {
            return Err(BackendError::Failed);
        }
        while self.next < self.end_frame {
            let produced = self.generator.generate_step(&mut out[..frame]);
            if produced == 0 {
                // The generator disagrees with the duration plan.
                return Err(BackendError::Failed);
            }
            let index = self.next;
            self.next += 1;
            if index >= self.first_frame {
                return Ok(produced);
            }
        }
        // Remaining trailing-silence frames are not emitted. The generator
        // must not hold more frames than planned.
        if self.next == self.end_frame && self.end_frame == self.total_frames {
            let mut scratch = [0.0f64; MAX_ENGINE_FRAME_SAMPLES];
            if self.generator.generate_step(&mut scratch[..frame]) != 0 {
                return Err(BackendError::Failed);
            }
        }
        out[..frame].fill(0.0);
        Ok(0)
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
        check_voice_layout(voice)?;
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

    /// Runs the text front end and the duration model only, and predicts the
    /// emitted length. `Ok(None)` means nothing speakable.
    pub fn plan(
        &self,
        text: &str,
    ) -> Result<Option<(UtterancePlan, Vec<jlabel::Label>)>, BackendError> {
        let loaded = self.loaded.as_ref().ok_or(BackendError::Unavailable)?;
        let labels = loaded
            .frontend
            .extract_fullcontext(text)
            .map_err(|_| BackendError::Failed)?;
        if labels.iter().all(is_silence) {
            return Ok(None);
        }
        let condition = &loaded.engine.condition;
        if condition.get_phoneme_alignment_flag() {
            // Alignment comes from label times, which text labels do not carry.
            return Err(BackendError::Failed);
        }
        let parsed = labels
            .clone()
            .to_labels(condition)
            .map_err(|_| BackendError::Failed)?;
        let models = Models::new(
            parsed.labels(),
            &loaded.engine.voices,
            condition.get_interporation_weight(),
        );
        let nstate = models.nstate();
        let durations =
            DurationEstimator::new(models.duration(), nstate).create(condition.get_speed());
        if nstate == 0 || durations.len() != labels.len() * nstate {
            return Err(BackendError::Failed);
        }
        let per_label: Vec<usize> = durations.chunks(nstate).map(|d| d.iter().sum()).collect();
        let total_frames: usize = per_label.iter().sum();
        let (mut first_frame, mut end_frame) = (0, total_frames);
        if self.trim.enabled {
            let leading: usize = labels
                .iter()
                .zip(&per_label)
                .take_while(|(label, _)| is_silence(label))
                .map(|(_, frames)| frames)
                .sum();
            let trailing: usize = labels
                .iter()
                .zip(&per_label)
                .rev()
                .take_while(|(label, _)| is_silence(label))
                .map(|(_, frames)| frames)
                .sum();
            first_frame = leading.saturating_sub(self.trim.pad_frames);
            end_frame = (total_frames - trailing + self.trim.pad_frames).min(total_frames);
        }
        if first_frame >= end_frame {
            return Ok(None);
        }
        let plan = UtterancePlan {
            labels: labels.len(),
            total_frames,
            first_frame,
            end_frame,
        };
        Ok(Some((plan, labels)))
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

/// Rejects an `.htsvoice` whose `[POSITION]` ranges point outside its
/// `[DATA]` section, before jbonsai parses it.
///
/// jbonsai 0.4.2 slices the data section with the header's ranges without
/// bounds checks, so a truncated or tampered voice (for example a partial
/// Model Store read) would panic inside the parser instead of returning an
/// error. This check turns that case into [`LoadError::VoiceInvalid`]. It
/// validates layout only; content integrity is the SHA-256 pin in
/// `tools/tts/tts-artifacts.lock`, checked by whoever delivers the bytes.
fn check_voice_layout(voice: &[u8]) -> Result<(), LoadError> {
    const POSITION: &[u8] = b"[POSITION]\n";
    const DATA: &[u8] = b"[DATA]\n";
    let find = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).position(|w| w == needle);
    let position_start = find(voice, POSITION).ok_or(LoadError::VoiceInvalid)? + POSITION.len();
    let after_position = &voice[position_start..];
    let position_len = find(after_position, b"\n[").ok_or(LoadError::VoiceInvalid)?;
    let header = &after_position[..position_len];
    let rest = &after_position[position_len..];
    let newlines = rest.iter().take_while(|&&b| b == b'\n').count();
    let rest = &rest[newlines..];
    if !rest.starts_with(DATA) {
        return Err(LoadError::VoiceInvalid);
    }
    let data_len = rest.len() - DATA.len();
    let header = core::str::from_utf8(header).map_err(|_| LoadError::VoiceInvalid)?;
    let mut ranges = 0usize;
    for line in header.lines().filter(|line| !line.trim().is_empty()) {
        let (_, value) = line.split_once(':').ok_or(LoadError::VoiceInvalid)?;
        for range in value.split(',') {
            let (start, end) = range
                .trim()
                .split_once('-')
                .ok_or(LoadError::VoiceInvalid)?;
            let start: usize = start.trim().parse().map_err(|_| LoadError::VoiceInvalid)?;
            let end: usize = end.trim().parse().map_err(|_| LoadError::VoiceInvalid)?;
            if start > end || end >= data_len {
                return Err(LoadError::VoiceInvalid);
            }
            ranges += 1;
        }
    }
    if ranges == 0 {
        return Err(LoadError::VoiceInvalid);
    }
    Ok(())
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
        max_frames: usize,
    ) -> Result<Option<Self::Source>, BackendError> {
        match language {
            SpeechSynthesisLanguage::Japanese | SpeechSynthesisLanguage::Auto => {}
            // The pinned voice is Japanese only; never pretend to speak
            // English with it.
            SpeechSynthesisLanguage::English => return Err(BackendError::UnsupportedLanguage),
        }
        let Some((plan, labels)) = self.plan(text)? else {
            return Ok(None);
        };
        if plan.emitted_frames() > max_frames {
            // Rejected before parameter generation: no audio is produced and
            // no per-frame acoustic parameters are allocated.
            return Err(BackendError::TooLong);
        }
        let loaded = self.loaded.as_ref().ok_or(BackendError::Unavailable)?;
        let generator = loaded
            .engine
            .generator(labels)
            .map_err(|_| BackendError::Failed)?;
        if generator.fperiod() != self.frame_len {
            return Err(BackendError::Failed);
        }
        Ok(Some(JbonsaiSource {
            generator,
            next: 0,
            first_frame: plan.first_frame,
            end_frame: plan.end_frame,
            total_frames: plan.total_frames,
        }))
    }
}

/// The M25 Japanese TTS provider: [`LocalTtsProvider`] over [`JbonsaiBackend`].
pub type JbonsaiProvider = LocalTtsProvider<JbonsaiBackend>;

/// Loads the pinned voice and dictionary and wraps them in a provider.
pub fn load_provider(voice: &Path, dictionary: &Path) -> Result<JbonsaiProvider, LoadError> {
    let backend = JbonsaiBackend::load(voice, dictionary)?;
    LocalTtsProvider::new(backend).map_err(|_| LoadError::UnsupportedFramePeriod)
}

#[cfg(test)]
mod layout_tests {
    use super::{check_voice_layout, LoadError};

    fn voice(position: &str, data_len: usize) -> Vec<u8> {
        let mut bytes =
            format!("[GLOBAL]\nA:1\n[STREAM]\nB:2\n[POSITION]\n{position}\n[DATA]\n").into_bytes();
        bytes.resize(bytes.len() + data_len, 0xA5);
        bytes
    }

    #[test]
    fn in_bounds_layout_is_accepted() {
        let bytes = voice("DURATION_PDF:0-9\nSTREAM_WIN[MCP]:10-12,13-19", 20);
        assert_eq!(check_voice_layout(&bytes), Ok(()));
    }

    #[test]
    fn truncated_data_is_rejected_not_panicking() {
        let bytes = voice("DURATION_PDF:0-9\nSTREAM_TREE[MCP]:10-1743120", 99_123);
        assert_eq!(check_voice_layout(&bytes), Err(LoadError::VoiceInvalid));
        let full = voice("DURATION_PDF:0-9", 10);
        assert_eq!(
            check_voice_layout(&full[..full.len() - 1]),
            Err(LoadError::VoiceInvalid)
        );
    }

    #[test]
    fn malformed_positions_are_rejected() {
        for position in [
            "DURATION_PDF:9-0",
            "DURATION_PDF:0-x",
            "DURATION_PDF",
            "DURATION_PDF:0-18446744073709551615",
        ] {
            assert_eq!(
                check_voice_layout(&voice(position, 20)),
                Err(LoadError::VoiceInvalid),
                "{position}"
            );
        }
        assert_eq!(
            check_voice_layout(b"not a voice"),
            Err(LoadError::VoiceInvalid)
        );
        assert_eq!(
            check_voice_layout(b"[POSITION]\nA:0-1\n"),
            Err(LoadError::VoiceInvalid)
        );
    }
}
