mod assets;

pub use assets::{AssetError, DictionaryAssets, LOCKED_VOICE_BYTES};

use jbonsai::{
    duration::DurationEstimator,
    engine::{Engine, EngineError},
    model::Models,
    speech::SpeechGenerator,
};
use jpreprocess::{DefaultTokenizer, JPreprocess};
use nagi_audio::{
    speech::{
        SpeechProviderError, SpeechSynthesisLanguage, SpeechSynthesisOptions,
        Stereo48KhzToMono16Khz, SynthesisPcmChunk, TextToSpeechProvider,
        MAX_SPEECH_PCM_CHUNK_BYTES, MAX_SPEECH_SYNTHESIS_TEXT_BYTES,
        MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES,
    },
    PcmFormat,
};

const VOICE_SAMPLE_RATE: usize = 48_000;
const VOICE_FRAME_SAMPLES: usize = 240;
const MAX_FULL_CONTEXT_LABELS: usize = 4 * MAX_SPEECH_SYNTHESIS_TEXT_BYTES;
const MONO_SAMPLES_PER_FRAME: usize = VOICE_FRAME_SAMPLES / 3;
const MONO_RESAMPLER_TAIL_FRAMES: usize = 126;
const MONO_RESAMPLER_TAIL_BYTES: usize = (MONO_RESAMPLER_TAIL_FRAMES / 3) * 2;
const STEREO_BYTES_PER_FRAME: usize = VOICE_FRAME_SAMPLES * 4;
const MONO_BYTES_PER_FRAME: usize = MONO_SAMPLES_PER_FRAME * 2;
const EDGE_SILENCE_PAD_FRAMES: usize = 10;
const MAX_TRIMMED_EDGE_FRAMES: usize = 4_000;

type Frontend = JPreprocess<DefaultTokenizer>;

struct LoadedEngine {
    frontend: Frontend,
    engine: Engine,
}

#[derive(Clone, Copy)]
enum OutputMode {
    Stereo48Khz,
    Mono16Khz,
}

impl OutputMode {
    fn from_format(format: PcmFormat) -> Option<Self> {
        if format == PcmFormat::stereo_48khz() {
            Some(Self::Stereo48Khz)
        } else if format == PcmFormat::mono_16khz() {
            Some(Self::Mono16Khz)
        } else {
            None
        }
    }

    fn bytes_per_frame(self) -> usize {
        match self {
            Self::Stereo48Khz => STEREO_BYTES_PER_FRAME,
            Self::Mono16Khz => MONO_BYTES_PER_FRAME,
        }
    }

    fn tail_bytes(self) -> usize {
        match self {
            Self::Stereo48Khz => 0,
            Self::Mono16Khz => MONO_RESAMPLER_TAIL_BYTES,
        }
    }
}

struct OutputPlan {
    total_frames: usize,
    first_frame: usize,
    end_frame: usize,
    frames: usize,
    bytes: usize,
    mode: OutputMode,
}

trait FrameGenerator {
    fn generate_frame(&mut self, destination: &mut [f64; VOICE_FRAME_SAMPLES]) -> usize;
}

impl FrameGenerator for SpeechGenerator {
    fn generate_frame(&mut self, destination: &mut [f64; VOICE_FRAME_SAMPLES]) -> usize {
        self.generate_step(destination)
    }
}

struct ActiveUtterance<G = SpeechGenerator> {
    generator: G,
    plan: OutputPlan,
    next_engine_frame: usize,
    emitted_frames: usize,
    emitted_bytes: usize,
}

struct Scratch {
    engine_samples: [f64; VOICE_FRAME_SAMPLES],
    stereo_frame: [u8; STEREO_BYTES_PER_FRAME],
    mono_frame: [u8; VOICE_FRAME_SAMPLES * 2],
    converter: Stereo48KhzToMono16Khz,
}

impl Scratch {
    fn new() -> Self {
        Self {
            engine_samples: [0.0; VOICE_FRAME_SAMPLES],
            stereo_frame: [0; STEREO_BYTES_PER_FRAME],
            mono_frame: [0; VOICE_FRAME_SAMPLES * 2],
            converter: Stereo48KhzToMono16Khz::new(),
        }
    }

    fn clear(&mut self) {
        self.engine_samples.fill(0.0);
        self.stereo_frame.fill(0);
        self.mono_frame.fill(0);
        self.converter.reset();
    }

    fn is_clear(&self) -> bool {
        self.engine_samples.iter().all(|sample| *sample == 0.0)
            && self.stereo_frame.iter().all(|byte| *byte == 0)
            && self.mono_frame.iter().all(|byte| *byte == 0)
    }
}

/// Isolated candidate for the Nagi M25 local Japanese TTS provider contract.
///
/// The provider owns no playback capability. The pinned engine and dictionary
/// are supplied as bytes from the caller's read-only Model Store boundary.
pub struct LocalTtsProvider {
    loaded: Option<LoadedEngine>,
    active: Option<ActiveUtterance>,
    scratch: Scratch,
}

impl LocalTtsProvider {
    /// Creates an unavailable provider. It is useful while assets are unloaded.
    pub fn unloaded() -> Self {
        Self {
            loaded: None,
            active: None,
            scratch: Scratch::new(),
        }
    }

    /// Validates pinned assets before calling third-party parsers.
    pub fn from_bytes(voice: &[u8], dictionary: DictionaryAssets) -> Result<Self, AssetError> {
        let loaded = load_verified_engine(voice, dictionary)?;
        Ok(Self {
            loaded: Some(loaded),
            active: None,
            scratch: Scratch::new(),
        })
    }

    /// Drops all loaded assets, including after a memory-pressure request.
    pub fn unload(&mut self) {
        unload_state(&mut self.loaded, &mut self.active, &mut self.scratch);
    }

    /// A replacement is installed only after its voice, dictionary, and engine
    /// have all passed validation. Errors leave the current stream and assets intact.
    pub fn reload(&mut self, voice: &[u8], dictionary: DictionaryAssets) -> Result<(), AssetError> {
        let replacement = load_verified_engine(voice, dictionary);
        install_verified_replacement(
            &mut self.loaded,
            &mut self.active,
            &mut self.scratch,
            replacement,
        )
    }

    pub fn is_clear(&self) -> bool {
        self.active.is_none() && self.scratch.is_clear()
    }

    fn cancel_inner(&mut self) {
        cancel_stream(&mut self.active, &mut self.scratch);
    }
}

fn load_verified_engine(
    voice: &[u8],
    dictionary: DictionaryAssets,
) -> Result<LoadedEngine, AssetError> {
    assets::verify_voice(voice)?;
    let dictionary = dictionary.verify_and_decode()?;
    let frontend = JPreprocess::with_dictionaries(dictionary, None);
    let engine = Engine::load_from_bytes([voice]).map_err(|_| AssetError::VoiceDecode)?;
    if engine.condition.get_sampling_frequency() != VOICE_SAMPLE_RATE
        || engine.condition.get_fperiod() != VOICE_FRAME_SAMPLES
    {
        return Err(AssetError::UnsupportedVoice);
    }

    Ok(LoadedEngine { frontend, engine })
}

fn prepare_begin<T>(active: &Option<T>, scratch: &mut Scratch) -> Result<(), SpeechProviderError> {
    if active.is_some() {
        return Err(SpeechProviderError::Failed);
    }
    scratch.clear();
    Ok(())
}

fn cancel_stream<U>(active: &mut Option<U>, scratch: &mut Scratch) {
    *active = None;
    scratch.clear();
}

fn unload_state<L, U>(loaded: &mut Option<L>, active: &mut Option<U>, scratch: &mut Scratch) {
    cancel_stream(active, scratch);
    *loaded = None;
}

fn install_verified_replacement<T, E, U>(
    loaded: &mut Option<T>,
    active: &mut Option<U>,
    scratch: &mut Scratch,
    candidate: Result<T, E>,
) -> Result<(), E> {
    let candidate = candidate?;
    cancel_stream(active, scratch);
    *loaded = Some(candidate);
    Ok(())
}

impl Default for LocalTtsProvider {
    fn default() -> Self {
        Self::unloaded()
    }
}

impl TextToSpeechProvider for LocalTtsProvider {
    fn begin(
        &mut self,
        text: &str,
        options: SpeechSynthesisOptions,
    ) -> Result<(), SpeechProviderError> {
        prepare_begin(&self.active, &mut self.scratch)?;

        if !validate_request(text, options.language, options.pcm_format) {
            return Err(SpeechProviderError::Failed);
        }
        let mode =
            OutputMode::from_format(options.pcm_format).ok_or(SpeechProviderError::Failed)?;
        let loaded = self
            .loaded
            .as_ref()
            .ok_or(SpeechProviderError::Unavailable)?;

        let labels = loaded
            .frontend
            .extract_fullcontext(text)
            .map_err(|_| SpeechProviderError::Failed)?;
        if labels.is_empty() || labels.len() > MAX_FULL_CONTEXT_LABELS {
            return Err(SpeechProviderError::Failed);
        }

        let plan = plan_output(&loaded.engine, &labels, mode).ok_or(SpeechProviderError::Failed)?;
        let generator = loaded.engine.generator(labels).map_err(map_engine_error)?;

        self.active = Some(ActiveUtterance {
            generator,
            plan,
            next_engine_frame: 0,
            emitted_frames: 0,
            emitted_bytes: 0,
        });
        Ok(())
    }

    fn next_pcm_chunk(
        &mut self,
        destination: &mut [u8],
    ) -> Result<SynthesisPcmChunk, SpeechProviderError> {
        next_stream_chunk(&mut self.active, &mut self.scratch, destination)
    }

    fn cancel(&mut self) {
        self.cancel_inner();
    }
}

fn next_stream_chunk<G: FrameGenerator>(
    active: &mut Option<ActiveUtterance<G>>,
    scratch: &mut Scratch,
    destination: &mut [u8],
) -> Result<SynthesisPcmChunk, SpeechProviderError> {
    let Some(current) = active.as_ref() else {
        return Ok(SynthesisPcmChunk::End);
    };
    let chunk_limit = checked_chunk_limit(destination.len(), current.plan.mode)?;
    destination[..chunk_limit].fill(0);
    let progress = match active.as_mut() {
        Some(current) => produce_chunk(current, scratch, destination, chunk_limit),
        None => return Ok(SynthesisPcmChunk::End),
    };
    match progress {
        Ok(ChunkProgress {
            bytes: 0,
            finished: true,
        }) => {
            cancel_stream(active, scratch);
            Ok(SynthesisPcmChunk::End)
        }
        Ok(ChunkProgress { bytes, finished }) => {
            if finished {
                cancel_stream(active, scratch);
            }
            Ok(SynthesisPcmChunk::Data(bytes))
        }
        Err(error) => {
            destination[..chunk_limit].fill(0);
            cancel_stream(active, scratch);
            Err(error)
        }
    }
}

struct ChunkProgress {
    bytes: usize,
    finished: bool,
}

// Prepared windows and resume counters must agree before engine work or PCM emission.
fn validate_stream_progress<G>(active: &ActiveUtterance<G>) -> Result<(), SpeechProviderError> {
    let plan = &active.plan;
    let frames = plan
        .end_frame
        .checked_sub(plan.first_frame)
        .filter(|frames| *frames != 0)
        .ok_or(SpeechProviderError::Failed)?;
    let bytes =
        checked_synthesis_output_bytes(frames, plan.mode).ok_or(SpeechProviderError::Failed)?;
    let work_limit = MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES
        .checked_sub(plan.mode.tail_bytes())
        .and_then(|bytes| bytes.checked_div(plan.mode.bytes_per_frame()))
        .and_then(|frames| frames.checked_add(MAX_TRIMMED_EDGE_FRAMES))
        .ok_or(SpeechProviderError::Failed)?;
    if plan.end_frame > plan.total_frames
        || plan.total_frames > work_limit
        || frames != plan.frames
        || bytes != plan.bytes
        || bytes > MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES
        || active.next_engine_frame > plan.end_frame
    {
        return Err(SpeechProviderError::Failed);
    }
    let emitted_frames = active.next_engine_frame.saturating_sub(plan.first_frame);
    let emitted_bytes = checked_output_bytes(emitted_frames, VOICE_FRAME_SAMPLES, plan.mode)
        .ok_or(SpeechProviderError::Failed)?;
    if active.emitted_frames != emitted_frames || active.emitted_bytes != emitted_bytes {
        return Err(SpeechProviderError::Failed);
    }
    Ok(())
}

fn produce_chunk<G: FrameGenerator>(
    active: &mut ActiveUtterance<G>,
    scratch: &mut Scratch,
    destination: &mut [u8],
    chunk_limit: usize,
) -> Result<ChunkProgress, SpeechProviderError> {
    validate_stream_progress(active)?;
    let frame_bytes = active.plan.mode.bytes_per_frame();
    let mut output_offset = 0usize;

    loop {
        if active.next_engine_frame == active.plan.end_frame {
            if active.plan.end_frame > active.plan.total_frames
                || active.emitted_frames != active.plan.frames
            {
                return Err(SpeechProviderError::Failed);
            }
            let expected_tail_bytes = active.plan.mode.tail_bytes();
            if chunk_limit - output_offset < expected_tail_bytes {
                return Ok(ChunkProgress {
                    bytes: output_offset,
                    finished: false,
                });
            }
            let next_total = active
                .emitted_bytes
                .checked_add(expected_tail_bytes)
                .filter(|bytes| {
                    *bytes == active.plan.bytes && *bytes <= MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES
                })
                .ok_or(SpeechProviderError::Failed)?;
            if active.plan.end_frame == active.plan.total_frames {
                scratch.engine_samples.fill(0.0);
                if active.generator.generate_frame(&mut scratch.engine_samples) != 0 {
                    return Err(SpeechProviderError::Failed);
                }
            }
            let tail_bytes = match active.plan.mode {
                OutputMode::Stereo48Khz => 0,
                OutputMode::Mono16Khz => {
                    let tail_samples = scratch
                        .converter
                        .finish(&mut scratch.mono_frame)
                        .map_err(|_| SpeechProviderError::Failed)?;
                    let tail_bytes = tail_samples
                        .checked_mul(2)
                        .filter(|bytes| *bytes == MONO_RESAMPLER_TAIL_BYTES)
                        .ok_or(SpeechProviderError::Failed)?;
                    let end = output_offset
                        .checked_add(tail_bytes)
                        .ok_or(SpeechProviderError::Failed)?;
                    if end > chunk_limit {
                        return Err(SpeechProviderError::Failed);
                    }
                    destination[output_offset..end]
                        .copy_from_slice(&scratch.mono_frame[..tail_bytes]);
                    tail_bytes
                }
            };
            if tail_bytes != expected_tail_bytes {
                return Err(SpeechProviderError::Failed);
            }
            active.emitted_bytes = next_total;
            return Ok(ChunkProgress {
                bytes: output_offset + tail_bytes,
                finished: true,
            });
        }

        let remaining = chunk_limit - output_offset;
        let should_emit = active.next_engine_frame >= active.plan.first_frame;
        if should_emit && remaining < frame_bytes {
            break;
        }
        if active.next_engine_frame > active.plan.end_frame
            || active.next_engine_frame >= active.plan.total_frames
        {
            return Err(SpeechProviderError::Failed);
        }

        scratch.engine_samples.fill(0.0);
        let written = active.generator.generate_frame(&mut scratch.engine_samples);
        if written != VOICE_FRAME_SAMPLES
            || scratch
                .engine_samples
                .iter()
                .any(|sample| !sample.is_finite())
        {
            return Err(SpeechProviderError::Failed);
        }

        let frame_index = active.next_engine_frame;
        active.next_engine_frame += 1;
        if frame_index < active.plan.first_frame {
            continue;
        }

        let next_bytes = active
            .emitted_bytes
            .checked_add(frame_bytes)
            .ok_or(SpeechProviderError::Failed)?;
        if next_bytes > active.plan.bytes || next_bytes > MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES {
            return Err(SpeechProviderError::Failed);
        }

        match active.plan.mode {
            OutputMode::Stereo48Khz => {
                encode_stereo(
                    &scratch.engine_samples,
                    &mut destination[output_offset..output_offset + frame_bytes],
                )?;
            }
            OutputMode::Mono16Khz => {
                encode_stereo(&scratch.engine_samples, &mut scratch.stereo_frame)?;
                let converted_samples = scratch
                    .converter
                    .convert(&scratch.stereo_frame, &mut scratch.mono_frame)
                    .map_err(|_| SpeechProviderError::Failed)?;
                let converted_bytes = converted_samples
                    .checked_mul(2)
                    .filter(|bytes| *bytes == frame_bytes)
                    .ok_or(SpeechProviderError::Failed)?;
                destination[output_offset..output_offset + frame_bytes]
                    .copy_from_slice(&scratch.mono_frame[..converted_bytes]);
            }
        }

        active.emitted_frames += 1;
        active.emitted_bytes = next_bytes;
        output_offset += frame_bytes;
        scratch.engine_samples.fill(0.0);
        scratch.stereo_frame.fill(0);
        scratch.mono_frame.fill(0);
    }

    Ok(ChunkProgress {
        bytes: output_offset,
        finished: false,
    })
}

fn encode_stereo(
    samples: &[f64; VOICE_FRAME_SAMPLES],
    output: &mut [u8],
) -> Result<(), SpeechProviderError> {
    if output.len() != STEREO_BYTES_PER_FRAME {
        return Err(SpeechProviderError::Failed);
    }
    for (sample, frame) in samples.iter().zip(output.chunks_exact_mut(4)) {
        let encoded = encode_sample(*sample)?;
        let bytes = encoded.to_le_bytes();
        frame[..2].copy_from_slice(&bytes);
        frame[2..].copy_from_slice(&bytes);
    }
    Ok(())
}

fn encode_sample(sample: f64) -> Result<i16, SpeechProviderError> {
    if !sample.is_finite() {
        return Err(SpeechProviderError::Failed);
    }
    Ok(sample.round().clamp(i16::MIN as f64, i16::MAX as f64) as i16)
}

fn plan_output(engine: &Engine, labels: &[jlabel::Label], mode: OutputMode) -> Option<OutputPlan> {
    if engine.condition.get_sampling_frequency() != VOICE_SAMPLE_RATE
        || engine.condition.get_fperiod() != VOICE_FRAME_SAMPLES
        || engine.condition.get_speed() != 1.0
        || engine.condition.get_phoneme_alignment_flag()
    {
        return None;
    }

    let models = Models::new(
        labels,
        &engine.voices,
        engine.condition.get_interporation_weight(),
    );
    let state_count = models.nstate();
    let expected_states = labels.len().checked_mul(state_count)?;
    let durations = models.duration();
    if durations.len() != expected_states || state_count == 0 {
        return None;
    }
    let state_durations =
        DurationEstimator::new(durations, state_count).create(engine.condition.get_speed());
    if state_durations.len() != expected_states {
        return None;
    }
    let per_label_frames = state_durations
        .chunks_exact(state_count)
        .map(|states| {
            states
                .iter()
                .try_fold(0usize, |sum, frames| sum.checked_add(*frames))
        })
        .collect::<Option<Vec<_>>>()?;
    let silence = labels
        .iter()
        .map(|label| is_silence_phoneme(label.phoneme.c.as_deref()))
        .collect::<Vec<_>>();
    plan_frame_window(&per_label_frames, &silence, mode)
}

fn is_silence_phoneme(phoneme: Option<&str>) -> bool {
    matches!(phoneme, None | Some("sil") | Some("pau"))
}

fn plan_frame_window(
    per_label_frames: &[usize],
    silence: &[bool],
    mode: OutputMode,
) -> Option<OutputPlan> {
    if per_label_frames.is_empty()
        || per_label_frames.len() != silence.len()
        || silence.iter().all(|is_silence| *is_silence)
    {
        return None;
    }

    let output_frame_limit = MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES.checked_sub(mode.tail_bytes())?
        / mode.bytes_per_frame();
    let total_frame_limit = output_frame_limit.checked_add(MAX_TRIMMED_EDGE_FRAMES)?;
    let total_frames = per_label_frames.iter().try_fold(0usize, |sum, frames| {
        let next = sum.checked_add(*frames)?;
        (next <= total_frame_limit).then_some(next)
    })?;
    let leading_frames = per_label_frames
        .iter()
        .zip(silence)
        .take_while(|(_, is_silence)| **is_silence)
        .try_fold(0usize, |sum, (frames, _)| sum.checked_add(*frames))?;
    let trailing_frames = per_label_frames
        .iter()
        .zip(silence)
        .rev()
        .take_while(|(_, is_silence)| **is_silence)
        .try_fold(0usize, |sum, (frames, _)| sum.checked_add(*frames))?;

    let first_frame = leading_frames.saturating_sub(EDGE_SILENCE_PAD_FRAMES);
    let end_frame = total_frames
        .saturating_sub(trailing_frames)
        .saturating_add(EDGE_SILENCE_PAD_FRAMES)
        .min(total_frames);
    let frames = end_frame.checked_sub(first_frame)?;
    let bytes = checked_synthesis_output_bytes(frames, mode)?;
    if frames == 0 || bytes == 0 || bytes > MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES {
        return None;
    }

    Some(OutputPlan {
        total_frames,
        first_frame,
        end_frame,
        frames,
        bytes,
        mode,
    })
}

fn checked_synthesis_output_bytes(frames: usize, mode: OutputMode) -> Option<usize> {
    checked_output_bytes(frames, VOICE_FRAME_SAMPLES, mode)?.checked_add(mode.tail_bytes())
}

fn checked_chunk_limit(
    destination_len: usize,
    mode: OutputMode,
) -> Result<usize, SpeechProviderError> {
    let limit = destination_len.min(MAX_SPEECH_PCM_CHUNK_BYTES);
    if limit < mode.bytes_per_frame() {
        Err(SpeechProviderError::OutputTooSmall)
    } else {
        Ok(limit)
    }
}

fn checked_output_bytes(
    frames: usize,
    samples_per_frame: usize,
    mode: OutputMode,
) -> Option<usize> {
    if samples_per_frame != VOICE_FRAME_SAMPLES {
        return None;
    }
    frames.checked_mul(mode.bytes_per_frame())
}

fn map_engine_error(_error: EngineError) -> SpeechProviderError {
    SpeechProviderError::Failed
}

fn validate_request(text: &str, language: SpeechSynthesisLanguage, format: PcmFormat) -> bool {
    !text.is_empty()
        && text.len() <= MAX_SPEECH_SYNTHESIS_TEXT_BYTES
        && matches!(
            language,
            SpeechSynthesisLanguage::Auto | SpeechSynthesisLanguage::Japanese
        )
        && OutputMode::from_format(format).is_some()
}

#[cfg(test)]
mod boundary_tests;

#[cfg(test)]
mod tests {
    use super::{
        checked_chunk_limit, checked_output_bytes, checked_synthesis_output_bytes, encode_sample,
        install_verified_replacement, is_silence_phoneme, plan_frame_window, prepare_begin,
        validate_request, LocalTtsProvider, OutputMode, MONO_RESAMPLER_TAIL_BYTES,
        STEREO_BYTES_PER_FRAME, VOICE_FRAME_SAMPLES,
    };
    use nagi_audio::speech::{
        SpeechProviderError, SpeechSynthesisLanguage, Stereo48KhzToMono16Khz, TextToSpeechProvider,
        MAX_SPEECH_PCM_CHUNK_BYTES, MAX_SPEECH_SYNTHESIS_TEXT_BYTES,
        MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES,
    };
    use nagi_audio::PcmFormat;

    #[test]
    fn stereo_plan_fails_before_crossing_the_service_cap() {
        assert_eq!(
            checked_output_bytes(1_092, VOICE_FRAME_SAMPLES, OutputMode::Stereo48Khz),
            Some(1_048_320)
        );
        assert_eq!(
            checked_output_bytes(1_093, VOICE_FRAME_SAMPLES, OutputMode::Stereo48Khz),
            Some(1_049_280)
        );
        assert!(
            checked_output_bytes(1_093, VOICE_FRAME_SAMPLES, OutputMode::Stereo48Khz).unwrap()
                > MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES
        );
    }

    #[test]
    fn mono_plan_accounts_for_the_three_to_one_rate_conversion() {
        assert_eq!(
            checked_output_bytes(6_553, VOICE_FRAME_SAMPLES, OutputMode::Mono16Khz),
            Some(1_048_480)
        );
        assert_eq!(
            checked_output_bytes(6_554, VOICE_FRAME_SAMPLES, OutputMode::Mono16Khz),
            Some(1_048_640)
        );
        assert_eq!(MONO_RESAMPLER_TAIL_BYTES, 84);
        assert_eq!(
            checked_synthesis_output_bytes(6_553, OutputMode::Mono16Khz),
            Some(1_048_564)
        );
        assert_eq!(
            checked_synthesis_output_bytes(6_554, OutputMode::Mono16Khz),
            Some(1_048_724)
        );
        assert!(
            checked_synthesis_output_bytes(6_553, OutputMode::Mono16Khz).unwrap()
                <= MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES
        );
        assert!(
            checked_synthesis_output_bytes(6_554, OutputMode::Mono16Khz).unwrap()
                > MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES
        );
    }

    #[test]
    fn mono_plan_accounts_for_the_converter_fir_tail() {
        let mut converter = Stereo48KhzToMono16Khz::new();
        let input = [0u8; 4 * VOICE_FRAME_SAMPLES];
        let mut output = [0u8; 2 * VOICE_FRAME_SAMPLES];
        assert_eq!(converter.convert(&input, &mut output), Ok(80));
        assert_eq!(converter.finish(&mut output), Ok(42));
        assert_eq!(42 * 2, MONO_RESAMPLER_TAIL_BYTES);
    }

    #[test]
    fn frame_plan_uses_checked_arithmetic_and_pinned_frame_shape() {
        assert_eq!(
            checked_output_bytes(usize::MAX, VOICE_FRAME_SAMPLES, OutputMode::Stereo48Khz),
            None
        );
        assert_eq!(
            checked_output_bytes(1, VOICE_FRAME_SAMPLES - 1, OutputMode::Stereo48Khz),
            None
        );
    }

    #[test]
    fn request_gate_preserves_only_supported_language_and_formats() {
        assert!(validate_request(
            "こんにちは",
            SpeechSynthesisLanguage::Japanese,
            PcmFormat::stereo_48khz()
        ));
        assert!(validate_request(
            "こんにちは",
            SpeechSynthesisLanguage::Auto,
            PcmFormat::mono_16khz()
        ));
        assert!(!validate_request(
            "hello",
            SpeechSynthesisLanguage::English,
            PcmFormat::stereo_48khz()
        ));
        assert!(!validate_request(
            "",
            SpeechSynthesisLanguage::Japanese,
            PcmFormat::stereo_48khz()
        ));
        assert!(!validate_request(
            "x".repeat(MAX_SPEECH_SYNTHESIS_TEXT_BYTES + 1).as_str(),
            SpeechSynthesisLanguage::Japanese,
            PcmFormat::stereo_48khz()
        ));
        assert!(validate_request(
            "ok",
            SpeechSynthesisLanguage::Japanese,
            PcmFormat::stereo_48khz(),
        ));
    }

    #[test]
    fn non_finite_engine_samples_fail_and_finite_values_saturate() {
        assert_eq!(encode_sample(12.4), Ok(12));
        assert_eq!(encode_sample(i16::MAX as f64 + 100.0), Ok(i16::MAX));
        assert_eq!(encode_sample(i16::MIN as f64 - 100.0), Ok(i16::MIN));
        assert_eq!(encode_sample(f64::NAN), Err(SpeechProviderError::Failed));
        assert_eq!(
            encode_sample(f64::INFINITY),
            Err(SpeechProviderError::Failed)
        );
    }

    #[test]
    fn cancel_clears_scratch_and_resets_the_streaming_converter() {
        let mut provider = LocalTtsProvider::unloaded();
        provider.scratch.engine_samples.fill(7.0);
        provider.scratch.stereo_frame.fill(0xaa);
        provider.scratch.mono_frame.fill(0xbb);

        let warmup = [0x55u8; 4 * (VOICE_FRAME_SAMPLES - 1)];
        let mut converted = [0u8; 2 * VOICE_FRAME_SAMPLES];
        provider
            .scratch
            .converter
            .convert(&warmup, &mut converted)
            .unwrap();

        provider.cancel();
        assert!(provider.is_clear());

        let mut fresh = Stereo48KhzToMono16Khz::new();
        let mut actual_output = [0u8; 2 * VOICE_FRAME_SAMPLES];
        let mut fresh_output = [0u8; 2 * VOICE_FRAME_SAMPLES];
        let input = [0x17u8; 4 * VOICE_FRAME_SAMPLES];
        let actual_len = provider
            .scratch
            .converter
            .convert(&input, &mut actual_output)
            .unwrap();
        let fresh_len = fresh.convert(&input, &mut fresh_output).unwrap();
        assert_eq!(actual_len, fresh_len);
        assert_eq!(&actual_output[..actual_len], &fresh_output[..fresh_len]);
    }

    #[test]
    fn insufficient_destination_is_rejected_before_frame_generation() {
        assert_eq!(
            checked_chunk_limit(2, OutputMode::Stereo48Khz),
            Err(SpeechProviderError::OutputTooSmall)
        );
        assert_eq!(
            checked_chunk_limit(STEREO_BYTES_PER_FRAME - 1, OutputMode::Stereo48Khz),
            Err(SpeechProviderError::OutputTooSmall)
        );
        assert_eq!(
            checked_chunk_limit(STEREO_BYTES_PER_FRAME, OutputMode::Stereo48Khz),
            Ok(STEREO_BYTES_PER_FRAME)
        );
        assert_eq!(
            checked_chunk_limit(MAX_SPEECH_PCM_CHUNK_BYTES + 1, OutputMode::Stereo48Khz),
            Ok(MAX_SPEECH_PCM_CHUNK_BYTES)
        );
    }

    #[test]
    fn cancel_on_unloaded_provider_is_idempotent() {
        let mut provider = LocalTtsProvider::unloaded();
        provider.cancel();
        provider.cancel();
        assert!(provider.is_clear());
    }

    #[test]
    fn silence_only_labels_have_no_output_window() {
        let silence = [None, Some("sil"), Some("pau")].map(|phoneme| is_silence_phoneme(phoneme));
        assert!(silence.iter().all(|is_silence| *is_silence));
        assert!(plan_frame_window(&[7, 5, 3], &silence, OutputMode::Stereo48Khz).is_none());
    }

    #[test]
    fn sil_and_pau_edges_are_trimmed_with_the_existing_padding() {
        let silence = [
            Some("sil"),
            Some("pau"),
            Some("a"),
            Some("pau"),
            Some("sil"),
        ]
        .map(|phoneme| is_silence_phoneme(phoneme));
        assert_eq!(silence, [true, true, false, true, true]);

        let plan =
            plan_frame_window(&[20, 15, 30, 12, 18], &silence, OutputMode::Stereo48Khz).unwrap();
        assert_eq!(plan.total_frames, 95);
        assert_eq!(plan.first_frame, 25);
        assert_eq!(plan.end_frame, 75);
        assert_eq!(plan.frames, 50);
    }

    #[test]
    fn an_internal_pause_stays_inside_the_output_window() {
        let silence = [Some("sil"), Some("a"), Some("pau"), Some("a"), Some("sil")]
            .map(|phoneme| is_silence_phoneme(phoneme));
        let plan =
            plan_frame_window(&[20, 8, 40, 9, 20], &silence, OutputMode::Stereo48Khz).unwrap();

        let internal_pause_start = 20 + 8;
        let internal_pause_end = internal_pause_start + 40;
        assert_eq!(plan.first_frame, 10);
        assert_eq!(plan.end_frame, 87);
        assert!(plan.first_frame < internal_pause_start);
        assert!(plan.end_frame > internal_pause_end);
    }

    #[test]
    fn trimming_and_the_utterance_cap_share_one_frame_budget() {
        for (mode, output_limit) in [
            (OutputMode::Stereo48Khz, 1_092usize),
            (OutputMode::Mono16Khz, 6_553usize),
        ] {
            let speech_frames = output_limit - 20;
            let edge_frames = [1_999, 2_000];
            let at_limit = [edge_frames[0], speech_frames, edge_frames[1]];
            let silence = [true, false, true];
            let plan = plan_frame_window(&at_limit, &silence, mode).unwrap();
            assert_eq!(plan.frames, output_limit);
            assert!(plan.bytes <= MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES);

            let above_limit = [edge_frames[0], speech_frames + 1, edge_frames[1]];
            assert!(plan_frame_window(&above_limit, &silence, mode).is_none());
        }
    }

    #[test]
    fn rejected_begin_keeps_active_operation_and_scratch_unchanged() {
        let active = Some(String::from("first utterance"));
        let before = active.clone();
        let mut scratch = super::Scratch::new();
        scratch.engine_samples.fill(7.0);
        scratch.stereo_frame.fill(0xaa);
        scratch.mono_frame.fill(0xbb);

        let warmup = [0x55u8; 4 * (VOICE_FRAME_SAMPLES - 1)];
        let mut discarded = [0u8; 2 * VOICE_FRAME_SAMPLES];
        scratch.converter.convert(&warmup, &mut discarded).unwrap();

        assert_eq!(
            prepare_begin(&active, &mut scratch),
            Err(SpeechProviderError::Failed)
        );
        assert_eq!(active, before);
        assert!(scratch.engine_samples.iter().all(|sample| *sample == 7.0));
        assert!(scratch.stereo_frame.iter().all(|byte| *byte == 0xaa));
        assert!(scratch.mono_frame.iter().all(|byte| *byte == 0xbb));

        let mut control = Stereo48KhzToMono16Khz::new();
        control.convert(&warmup, &mut discarded).unwrap();
        let input = [0x17u8; 4 * VOICE_FRAME_SAMPLES];
        let mut actual_output = [0u8; 2 * VOICE_FRAME_SAMPLES];
        let mut control_output = [0u8; 2 * VOICE_FRAME_SAMPLES];
        let actual = scratch
            .converter
            .convert(&input, &mut actual_output)
            .unwrap();
        let expected = control.convert(&input, &mut control_output).unwrap();
        assert_eq!(actual, expected);
        assert_eq!(&actual_output[..actual], &control_output[..expected]);
    }

    #[test]
    fn invalid_replacement_keeps_the_verified_model_and_current_stream() {
        let mut current_model = Some("verified old model");
        let mut active = Some("current utterance");
        let mut scratch = super::Scratch::new();
        scratch.engine_samples.fill(9.0);
        scratch.stereo_frame.fill(0xcc);
        scratch.mono_frame.fill(0xdd);
        let invalid_voice = super::assets::verify_voice(&[]).map(|()| "unverified new model");

        assert_eq!(
            install_verified_replacement(
                &mut current_model,
                &mut active,
                &mut scratch,
                invalid_voice
            ),
            Err(super::AssetError::VoiceMismatch)
        );
        assert_eq!(current_model, Some("verified old model"));
        assert_eq!(active, Some("current utterance"));
        assert!(scratch.engine_samples.iter().all(|sample| *sample == 9.0));
        assert!(scratch.stereo_frame.iter().all(|byte| *byte == 0xcc));
        assert!(scratch.mono_frame.iter().all(|byte| *byte == 0xdd));
    }

    #[test]
    fn verified_replacement_swaps_model_and_cancels_current_stream() {
        let mut current_model = Some("verified old model");
        let mut active = Some("current utterance");
        let mut scratch = super::Scratch::new();
        scratch.engine_samples.fill(9.0);

        assert_eq!(
            install_verified_replacement(
                &mut current_model,
                &mut active,
                &mut scratch,
                Ok::<_, super::AssetError>("verified new model")
            ),
            Ok(())
        );
        assert_eq!(current_model, Some("verified new model"));
        assert_eq!(active, None);
        assert!(scratch.is_clear());
    }
}
