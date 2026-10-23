use super::*;
use nagi_audio::speech::{
    PlaybackError, SpeechPlaybackSink, SpeechSynthesisError, SpeechSynthesisService,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Default)]
struct Probe {
    calls: Cell<usize>,
    end_calls: Cell<usize>,
    drops: Cell<usize>,
}

#[derive(Clone, Copy)]
enum Fault {
    None,
    Short(usize),
    Nan(usize),
    ExtraEnd,
}

struct SyntheticGenerator {
    frames: usize,
    cursor: usize,
    seed: i16,
    fault: Fault,
    probe: Rc<Probe>,
}

fn fixture_sample(seed: i16, frame: usize, index: usize) -> i16 {
    seed + ((frame * 17 + index * 13) % 509) as i16 - 254
}

impl FrameGenerator for SyntheticGenerator {
    fn generate_frame(&mut self, destination: &mut [f64; VOICE_FRAME_SAMPLES]) -> usize {
        self.probe.calls.set(self.probe.calls.get() + 1);
        if self.cursor == self.frames {
            self.probe.end_calls.set(self.probe.end_calls.get() + 1);
            return if matches!(self.fault, Fault::ExtraEnd) {
                destination.fill(999.0);
                VOICE_FRAME_SAMPLES
            } else {
                0
            };
        }
        let frame = self.cursor;
        self.cursor += 1;
        for (index, sample) in destination.iter_mut().enumerate() {
            *sample = f64::from(fixture_sample(self.seed, frame, index));
        }
        match self.fault {
            Fault::Short(at) if frame == at => VOICE_FRAME_SAMPLES - 1,
            Fault::Nan(at) if frame == at => {
                destination[VOICE_FRAME_SAMPLES / 2] = f64::NAN;
                VOICE_FRAME_SAMPLES
            }
            _ => VOICE_FRAME_SAMPLES,
        }
    }
}

impl Drop for SyntheticGenerator {
    fn drop(&mut self) {
        self.probe.drops.set(self.probe.drops.get() + 1);
    }
}

fn make_active(
    frames: usize,
    mode: OutputMode,
    seed: i16,
    fault: Fault,
    probe: Rc<Probe>,
) -> ActiveUtterance<SyntheticGenerator> {
    ActiveUtterance {
        generator: SyntheticGenerator {
            frames,
            cursor: 0,
            seed,
            fault,
            probe,
        },
        plan: OutputPlan {
            total_frames: frames,
            first_frame: 0,
            end_frame: frames,
            frames,
            bytes: frames * mode.bytes_per_frame() + mode.tail_bytes(),
            mode,
        },
        next_engine_frame: 0,
        emitted_frames: 0,
        emitted_bytes: 0,
    }
}

// The oracle assembles known integer fixture samples independently of
// produce_chunk/encode_stereo. The public converter supplies only the DSP
// reference; this tests stream framing/state, not independent DSP correctness.
fn reference_pcm(frames: usize, mode: OutputMode, seed: i16) -> Vec<u8> {
    let mut stereo = Vec::with_capacity(frames * STEREO_BYTES_PER_FRAME);
    for frame in 0..frames {
        for index in 0..VOICE_FRAME_SAMPLES {
            let bytes = fixture_sample(seed, frame, index).to_le_bytes();
            stereo.extend_from_slice(&bytes);
            stereo.extend_from_slice(&bytes);
        }
    }
    if matches!(mode, OutputMode::Stereo48Khz) {
        return stereo;
    }
    let mut converter = Stereo48KhzToMono16Khz::new();
    let mut mono = vec![0; stereo.len() + 512];
    let samples = converter.convert(&stereo, &mut mono).unwrap();
    let tail = converter.finish(&mut mono[samples * 2..]).unwrap();
    mono.truncate((samples + tail) * 2);
    mono
}

fn direct_output(
    active: &mut ActiveUtterance<SyntheticGenerator>,
    scratch: &mut Scratch,
    limit: usize,
) -> (Vec<u8>, Vec<usize>) {
    let mut output = Vec::new();
    let mut chunks = Vec::new();
    for _ in 0..active.plan.frames + 3 {
        let mut destination = vec![0xcc; limit];
        let progress = produce_chunk(active, scratch, &mut destination, limit).unwrap();
        assert!(progress.bytes <= limit);
        output.extend_from_slice(&destination[..progress.bytes]);
        chunks.push(progress.bytes);
        assert!(output.len() <= MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES);
        if progress.finished {
            return (output, chunks);
        }
        assert!(progress.bytes > 0, "stream made no progress");
    }
    panic!("stream did not finish within the fixture frame count");
}

fn drain(
    active: &mut Option<ActiveUtterance<SyntheticGenerator>>,
    scratch: &mut Scratch,
    limit: usize,
) -> Vec<u8> {
    let mut output = Vec::new();
    loop {
        let mut destination = vec![0xcc; limit];
        match next_stream_chunk(active, scratch, &mut destination).unwrap() {
            SynthesisPcmChunk::Data(bytes) => output.extend_from_slice(&destination[..bytes]),
            SynthesisPcmChunk::End => return output,
        }
    }
}

#[test]
fn produce_chunk_matches_reference_across_frame_and_chunk_boundaries() {
    for (mode, limits) in [
        (OutputMode::Stereo48Khz, vec![960, 1948, 4096]),
        (OutputMode::Mono16Khz, vec![160, 244, 320, 4096]),
    ] {
        let expected = reference_pcm(31, mode, 1000);
        for limit in limits {
            let probe = Rc::new(Probe::default());
            let mut active = make_active(31, mode, 1000, Fault::None, probe.clone());
            let (actual, _) = direct_output(&mut active, &mut Scratch::new(), limit);
            assert_eq!(actual, expected, "chunk limit {limit}");
            assert_eq!(active.emitted_frames, 31);
            assert_eq!(active.emitted_bytes, expected.len());
            assert_eq!(probe.calls.get(), 32);
            assert_eq!(probe.end_calls.get(), 1);
        }
    }
}

#[test]
fn fir_tail_is_carried_to_next_chunk_and_emitted_only_once() {
    let probe = Rc::new(Probe::default());
    let mut active = make_active(2, OutputMode::Mono16Khz, 1200, Fault::None, probe.clone());
    let (actual, sizes) = direct_output(&mut active, &mut Scratch::new(), 160);
    assert_eq!(sizes, [160, 160, 84]);
    assert_eq!(actual, reference_pcm(2, OutputMode::Mono16Khz, 1200));
    assert_eq!(probe.end_calls.get(), 1);

    let probe = Rc::new(Probe::default());
    let mut active = Some(make_active(
        2,
        OutputMode::Mono16Khz,
        1200,
        Fault::None,
        probe.clone(),
    ));
    let mut scratch = Scratch::new();
    assert_eq!(drain(&mut active, &mut scratch, 160), actual);
    assert!(active.is_none() && scratch.is_clear());
    for _ in 0..3 {
        assert_eq!(
            next_stream_chunk(&mut active, &mut scratch, &mut [0; 160]),
            Ok(SynthesisPcmChunk::End)
        );
    }
    assert_eq!(probe.calls.get(), 3);
    assert_eq!(probe.end_calls.get(), 1);
    assert_eq!(probe.drops.get(), 1);
}

#[test]
fn direct_producer_accepts_the_last_complete_frame_under_each_cap() {
    for (mode, frames) in [
        (OutputMode::Stereo48Khz, 1092),
        (OutputMode::Mono16Khz, 6553),
    ] {
        let probe = Rc::new(Probe::default());
        let mut active = make_active(frames, mode, 900, Fault::None, probe);
        let (actual, _) = direct_output(&mut active, &mut Scratch::new(), 4096);
        assert_eq!(actual, reference_pcm(frames, mode, 900));
        assert!(actual.len() <= MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES);
        assert!(actual.len() + mode.bytes_per_frame() > MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES);
    }
}

#[test]
fn cumulative_overflow_aborts_and_erases_the_failed_chunk() {
    for (mode, frames) in [
        (OutputMode::Stereo48Khz, 1093),
        (OutputMode::Mono16Khz, 6554),
    ] {
        let probe = Rc::new(Probe::default());
        let mut active = Some(make_active(frames, mode, 900, Fault::None, probe.clone()));
        let mut scratch = Scratch::new();
        let mut accepted = 0;
        let mut failed = false;
        for _ in 0..frames + 1 {
            let mut destination = [0xcc; MAX_SPEECH_PCM_CHUNK_BYTES];
            match next_stream_chunk(&mut active, &mut scratch, &mut destination) {
                Ok(SynthesisPcmChunk::Data(bytes)) => accepted += bytes,
                Err(SpeechProviderError::Failed) => {
                    assert!(destination.iter().all(|byte| *byte == 0));
                    failed = true;
                    break;
                }
                other => panic!("unexpected cap result: {other:?}"),
            }
        }
        assert!(failed);
        assert!(accepted <= MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES);
        assert!(active.is_none() && scratch.is_clear());
        assert_eq!(probe.drops.get(), 1);
        assert_eq!(
            next_stream_chunk(&mut active, &mut scratch, &mut [0; 4096]),
            Ok(SynthesisPcmChunk::End)
        );
    }
}

#[test]
fn tail_budget_is_checked_before_flush_even_with_an_invalid_internal_plan() {
    let probe = Rc::new(Probe::default());
    let mut stream = make_active(0, OutputMode::Mono16Khz, 0, Fault::None, probe.clone());
    stream.emitted_bytes = MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES - 40;
    stream.plan.bytes = stream.emitted_bytes + MONO_RESAMPLER_TAIL_BYTES;
    let mut active = Some(stream);
    let mut scratch = Scratch::new();
    let mut destination = [0xcc; 160];
    assert_eq!(
        next_stream_chunk(&mut active, &mut scratch, &mut destination),
        Err(SpeechProviderError::Failed)
    );
    assert_eq!(destination, [0; 160]);
    assert_eq!(probe.calls.get(), 0);
    assert_eq!(probe.drops.get(), 1);
    assert!(active.is_none() && scratch.is_clear());
}

#[test]
fn short_nan_and_extra_end_frames_erase_prior_output_and_terminate() {
    for mode in [OutputMode::Stereo48Khz, OutputMode::Mono16Khz] {
        for fault in [Fault::Short(1), Fault::Nan(1), Fault::ExtraEnd] {
            let probe = Rc::new(Probe::default());
            let mut active = Some(make_active(2, mode, 3000, fault, probe.clone()));
            let mut scratch = Scratch::new();
            let mut destination = [0xcc; MAX_SPEECH_PCM_CHUNK_BYTES + 16];
            assert_eq!(
                next_stream_chunk(&mut active, &mut scratch, &mut destination),
                Err(SpeechProviderError::Failed)
            );
            assert!(destination[..MAX_SPEECH_PCM_CHUNK_BYTES]
                .iter()
                .all(|byte| *byte == 0));
            assert_eq!(destination[MAX_SPEECH_PCM_CHUNK_BYTES..], [0xcc; 16]);
            assert!(active.is_none() && scratch.is_clear());
            let calls = probe.calls.get();
            assert_eq!(
                next_stream_chunk(&mut active, &mut scratch, &mut destination),
                Ok(SynthesisPcmChunk::End)
            );
            assert_eq!(probe.calls.get(), calls);
            assert_eq!(probe.drops.get(), 1);
        }
    }
}

#[test]
fn undersized_destination_preserves_the_stream_for_retry() {
    let probe = Rc::new(Probe::default());
    let mut active = Some(make_active(
        2,
        OutputMode::Mono16Khz,
        900,
        Fault::None,
        probe.clone(),
    ));
    let mut scratch = Scratch::new();
    let mut destination = [0xcc; 159];
    assert_eq!(
        next_stream_chunk(&mut active, &mut scratch, &mut destination),
        Err(SpeechProviderError::OutputTooSmall)
    );
    assert_eq!(destination, [0xcc; 159]);
    assert_eq!(probe.calls.get(), 0);
    assert_eq!(probe.drops.get(), 0);
    assert_eq!(
        drain(&mut active, &mut scratch, 160),
        reference_pcm(2, OutputMode::Mono16Khz, 900)
    );
}

struct DropResource {
    id: usize,
    drops: Rc<Cell<usize>>,
}

impl Drop for DropResource {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

fn warm_stream(
    probe: Rc<Probe>,
) -> (
    Option<ActiveUtterance<SyntheticGenerator>>,
    Scratch,
    Vec<u8>,
) {
    let mut active = Some(make_active(
        7,
        OutputMode::Mono16Khz,
        1000,
        Fault::None,
        probe,
    ));
    let mut scratch = Scratch::new();
    let mut first = [0; 160];
    assert_eq!(
        next_stream_chunk(&mut active, &mut scratch, &mut first),
        Ok(SynthesisPcmChunk::Data(160))
    );
    (active, scratch, first.to_vec())
}

#[test]
fn active_cancel_drops_generator_once_keeps_model_and_allows_clean_reuse() {
    let probe = Rc::new(Probe::default());
    let drops = Rc::new(Cell::new(0));
    let mut model = Some(DropResource {
        id: 1,
        drops: drops.clone(),
    });
    let (mut active, mut scratch, _) = warm_stream(probe.clone());
    cancel_stream(&mut active, &mut scratch);
    cancel_stream(&mut active, &mut scratch);
    assert_eq!(probe.drops.get(), 1);
    assert_eq!(drops.get(), 0);
    assert!(scratch.is_clear());
    assert_eq!(prepare_begin(&active, &mut scratch), Ok(()));
    active = Some(make_active(
        7,
        OutputMode::Mono16Khz,
        4000,
        Fault::None,
        probe.clone(),
    ));
    assert_eq!(
        drain(&mut active, &mut scratch, 160),
        reference_pcm(7, OutputMode::Mono16Khz, 4000)
    );
    assert_eq!(probe.drops.get(), 2);
    unload_state(&mut model, &mut active, &mut scratch);
    assert_eq!(drops.get(), 1);
}

#[test]
fn active_unload_drops_model_and_generator_exactly_once() {
    let probe = Rc::new(Probe::default());
    let drops = Rc::new(Cell::new(0));
    let mut model = Some(DropResource {
        id: 1,
        drops: drops.clone(),
    });
    let (mut active, mut scratch, _) = warm_stream(probe.clone());
    unload_state(&mut model, &mut active, &mut scratch);
    unload_state(&mut model, &mut active, &mut scratch);
    assert!(model.is_none() && active.is_none() && scratch.is_clear());
    assert_eq!(drops.get(), 1);
    assert_eq!(probe.drops.get(), 1);
}

#[test]
fn failed_reload_keeps_drop_counted_model_and_pending_fir_state() {
    let probe = Rc::new(Probe::default());
    let drops = Rc::new(Cell::new(0));
    let mut model = Some(DropResource {
        id: 1,
        drops: drops.clone(),
    });
    let (mut active, mut scratch, mut output) = warm_stream(probe.clone());
    assert_eq!(
        install_verified_replacement(
            &mut model,
            &mut active,
            &mut scratch,
            Err::<DropResource, _>(AssetError::VoiceMismatch)
        ),
        Err(AssetError::VoiceMismatch)
    );
    assert_eq!(model.as_ref().unwrap().id, 1);
    assert_eq!(drops.get(), 0);
    assert_eq!(probe.drops.get(), 0);
    output.extend_from_slice(&drain(&mut active, &mut scratch, 160));
    assert_eq!(output, reference_pcm(7, OutputMode::Mono16Khz, 1000));
    assert_eq!(probe.drops.get(), 1);
    assert_eq!(drops.get(), 0);
}

#[test]
fn successful_reload_drops_prior_resources_and_restarts_with_clean_fir() {
    let probe = Rc::new(Probe::default());
    let old_drops = Rc::new(Cell::new(0));
    let new_drops = Rc::new(Cell::new(0));
    let mut model = Some(DropResource {
        id: 1,
        drops: old_drops.clone(),
    });
    let (mut active, mut scratch, _) = warm_stream(probe.clone());
    assert_eq!(
        install_verified_replacement(
            &mut model,
            &mut active,
            &mut scratch,
            Ok::<_, AssetError>(DropResource {
                id: 2,
                drops: new_drops.clone()
            })
        ),
        Ok(())
    );
    assert_eq!(model.as_ref().unwrap().id, 2);
    assert_eq!(old_drops.get(), 1);
    assert_eq!(new_drops.get(), 0);
    assert_eq!(probe.drops.get(), 1);
    assert!(active.is_none() && scratch.is_clear());
    assert_eq!(prepare_begin(&active, &mut scratch), Ok(()));
    active = Some(make_active(
        7,
        OutputMode::Mono16Khz,
        4000,
        Fault::None,
        probe.clone(),
    ));
    assert_eq!(
        drain(&mut active, &mut scratch, 244),
        reference_pcm(7, OutputMode::Mono16Khz, 4000)
    );
    unload_state(&mut model, &mut active, &mut scratch);
    assert_eq!(old_drops.get(), 1);
    assert_eq!(new_drops.get(), 1);
    assert_eq!(probe.drops.get(), 2);
}

#[derive(Default)]
struct ServiceProbe {
    generator: Rc<Probe>,
    begins: Cell<usize>,
    cancels: Cell<usize>,
    clean_after_cancel: Cell<bool>,
}

struct SyntheticProvider {
    active: Option<ActiveUtterance<SyntheticGenerator>>,
    scratch: Scratch,
    probe: Rc<ServiceProbe>,
}

impl TextToSpeechProvider for SyntheticProvider {
    fn begin(
        &mut self,
        text: &str,
        options: SpeechSynthesisOptions,
    ) -> Result<(), SpeechProviderError> {
        prepare_begin(&self.active, &mut self.scratch)?;
        if !validate_request(text, options.language, options.pcm_format) {
            return Err(SpeechProviderError::Failed);
        }
        self.probe.begins.set(self.probe.begins.get() + 1);
        let (frames, seed) = if text == "first" {
            (90, 1000)
        } else {
            (7, 4000)
        };
        self.active = Some(make_active(
            frames,
            OutputMode::from_format(options.pcm_format).unwrap(),
            seed,
            Fault::None,
            self.probe.generator.clone(),
        ));
        Ok(())
    }

    fn next_pcm_chunk(
        &mut self,
        destination: &mut [u8],
    ) -> Result<SynthesisPcmChunk, SpeechProviderError> {
        next_stream_chunk(&mut self.active, &mut self.scratch, destination)
    }

    fn cancel(&mut self) {
        self.probe.cancels.set(self.probe.cancels.get() + 1);
        cancel_stream(&mut self.active, &mut self.scratch);
        self.probe
            .clean_after_cancel
            .set(self.active.is_none() && self.scratch.is_clear());
    }
}

#[derive(Default)]
struct SinkLog {
    attempts: usize,
    accepted: Vec<(PcmFormat, Vec<u8>)>,
    rejected: Vec<PcmFormat>,
}

// A host contract surrogate, not the cfg(target_os = "nagi") AudioService sink.
struct RecordingSink {
    log: Rc<RefCell<SinkLog>>,
    fail_on: Rc<Cell<Option<usize>>>,
    stereo_only: bool,
}

impl SpeechPlaybackSink for RecordingSink {
    fn play_pcm(&mut self, format: PcmFormat, bytes: &[u8]) -> Result<(), PlaybackError> {
        let mut log = self.log.borrow_mut();
        log.attempts += 1;
        let rejects_format = self.stereo_only
            && (format != PcmFormat::stereo_48khz()
                || bytes.is_empty()
                || !bytes.chunks_exact(4).remainder().is_empty());
        if rejects_format || self.fail_on.get() == Some(log.attempts) {
            log.rejected.push(format);
            return Err(PlaybackError::Unavailable);
        }
        log.accepted.push((format, bytes.to_vec()));
        Ok(())
    }
}

fn options(format: PcmFormat) -> SpeechSynthesisOptions {
    SpeechSynthesisOptions {
        language: SpeechSynthesisLanguage::Japanese,
        pcm_format: format,
    }
}

#[test]
fn service_playback_failure_cancels_active_stream_and_next_utterance_has_no_state_leak() {
    let probe = Rc::new(ServiceProbe::default());
    let log = Rc::new(RefCell::new(SinkLog::default()));
    let fail_on = Rc::new(Cell::new(Some(2)));
    let provider = SyntheticProvider {
        active: None,
        scratch: Scratch::new(),
        probe: probe.clone(),
    };
    let sink = RecordingSink {
        log: log.clone(),
        fail_on: fail_on.clone(),
        stereo_only: false,
    };
    let mut service = SpeechSynthesisService::new(provider, sink);
    assert_eq!(
        service.speak(b"first", options(PcmFormat::mono_16khz())),
        Err(SpeechSynthesisError::PlaybackUnavailable)
    );
    assert_eq!(probe.cancels.get(), 1);
    assert!(probe.clean_after_cancel.get());
    assert_eq!(probe.generator.drops.get(), 1);
    assert_eq!(
        probe.generator.end_calls.get(),
        0,
        "failure must interrupt an active generator"
    );
    assert_eq!(log.borrow().accepted.len(), 1);
    assert_eq!(
        log.borrow().accepted[0].1,
        reference_pcm(90, OutputMode::Mono16Khz, 1000)[..4000]
    );
    let prior_count = log.borrow().accepted.len();
    fail_on.set(None);
    let expected = reference_pcm(7, OutputMode::Mono16Khz, 4000);
    assert_eq!(
        service.speak(b"second", options(PcmFormat::mono_16khz())),
        Ok(expected.len())
    );
    let actual: Vec<u8> = log.borrow().accepted[prior_count..]
        .iter()
        .flat_map(|(_, bytes)| bytes.iter().copied())
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(probe.begins.get(), 2);
    assert_eq!(probe.cancels.get(), 1);
    assert_eq!(probe.generator.drops.get(), 2);
    assert_eq!(probe.generator.end_calls.get(), 1);
}

#[test]
fn stereo_sink_contract_rejects_mono_without_conversion_then_accepts_fresh_stereo() {
    let probe = Rc::new(ServiceProbe::default());
    let log = Rc::new(RefCell::new(SinkLog::default()));
    let provider = SyntheticProvider {
        active: None,
        scratch: Scratch::new(),
        probe: probe.clone(),
    };
    let sink = RecordingSink {
        log: log.clone(),
        fail_on: Rc::new(Cell::new(None)),
        stereo_only: true,
    };
    let mut service = SpeechSynthesisService::new(provider, sink);
    assert_eq!(
        service.speak(b"first", options(PcmFormat::mono_16khz())),
        Err(SpeechSynthesisError::PlaybackUnavailable)
    );
    assert_eq!(log.borrow().rejected, [PcmFormat::mono_16khz()]);
    assert!(log.borrow().accepted.is_empty());
    assert_eq!(probe.cancels.get(), 1);
    assert!(probe.clean_after_cancel.get());
    assert_eq!(probe.generator.drops.get(), 1);
    let expected = reference_pcm(7, OutputMode::Stereo48Khz, 4000);
    assert_eq!(
        service.speak(b"second", options(PcmFormat::stereo_48khz())),
        Ok(expected.len())
    );
    let actual: Vec<u8> = log
        .borrow()
        .accepted
        .iter()
        .flat_map(|(format, bytes)| {
            assert_eq!(*format, PcmFormat::stereo_48khz());
            bytes.iter().copied()
        })
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(probe.begins.get(), 2);
    assert_eq!(probe.cancels.get(), 1);
    assert_eq!(probe.generator.drops.get(), 2);
}

#[test]
fn invalid_plan_or_progress_is_rejected_before_generator_calls() {
    type Mutation = fn(&mut ActiveUtterance<SyntheticGenerator>);
    let cases: [(&str, Mutation); 10] = [
        ("reversed window", |s| s.plan.first_frame = 4),
        ("end beyond engine", |s| s.plan.total_frames = 2),
        ("frame count mismatch", |s| s.plan.frames = 2),
        ("byte budget mismatch", |s| s.plan.bytes -= 2),
        ("consistent over-cap plan", |s| {
            let frames = (MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES - s.plan.mode.tail_bytes())
                / s.plan.mode.bytes_per_frame()
                + 1;
            s.plan.total_frames = frames;
            s.plan.end_frame = frames;
            s.plan.frames = frames;
            s.plan.bytes = frames * s.plan.mode.bytes_per_frame() + s.plan.mode.tail_bytes();
        }),
        ("unbounded trimmed work", |s| {
            s.plan.total_frames = usize::MAX
        }),
        ("cursor past window", |s| s.next_engine_frame = 4),
        ("frame progress mismatch", |s| s.emitted_frames = 1),
        ("byte progress mismatch", |s| s.emitted_bytes = 1),
        ("empty output window", |s| {
            s.plan.end_frame = 0;
            s.plan.frames = 0;
            s.plan.bytes = s.plan.mode.tail_bytes();
        }),
    ];
    for mode in [OutputMode::Stereo48Khz, OutputMode::Mono16Khz] {
        for (name, mutate) in cases {
            let probe = Rc::new(Probe::default());
            let mut stream = make_active(3, mode, 1000, Fault::None, probe.clone());
            mutate(&mut stream);
            let mut active = Some(stream);
            let mut scratch = Scratch::new();
            let mut destination = [0xcc; MAX_SPEECH_PCM_CHUNK_BYTES];
            assert_eq!(
                next_stream_chunk(&mut active, &mut scratch, &mut destination),
                Err(SpeechProviderError::Failed),
                "{name}"
            );
            assert!(destination.iter().all(|byte| *byte == 0), "{name}");
            assert_eq!(probe.calls.get(), 0, "{name} reached the generator");
            assert_eq!(probe.drops.get(), 1, "{name}");
            assert!(active.is_none() && scratch.is_clear(), "{name}");
            assert_eq!(
                next_stream_chunk(&mut active, &mut scratch, &mut destination),
                Ok(SynthesisPcmChunk::End)
            );
            assert_eq!(probe.calls.get(), 0);
        }
    }
}

#[test]
fn valid_trimmed_plan_still_skips_edges_and_emits_only_its_window() {
    let probe = Rc::new(Probe::default());
    let mut active = make_active(6, OutputMode::Stereo48Khz, 2000, Fault::None, probe.clone());
    active.plan.first_frame = 1;
    active.plan.end_frame = 4;
    active.plan.frames = 3;
    active.plan.bytes = 3 * STEREO_BYTES_PER_FRAME;
    let (actual, chunks) = direct_output(&mut active, &mut Scratch::new(), STEREO_BYTES_PER_FRAME);
    let whole = reference_pcm(6, OutputMode::Stereo48Khz, 2000);
    assert_eq!(
        actual,
        whole[STEREO_BYTES_PER_FRAME..4 * STEREO_BYTES_PER_FRAME]
    );
    assert_eq!(chunks, [STEREO_BYTES_PER_FRAME; 3]);
    assert_eq!(probe.calls.get(), 4);
    assert_eq!(probe.end_calls.get(), 0);
}
