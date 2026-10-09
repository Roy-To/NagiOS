//! Host evaluation of the M25 Whisper provider on unseen Japanese speech.
//!
//! Runs the shared provider lifecycle (`provider_session.rs`) over the pinned,
//! patched whisper.cpp host build with one inference thread, and records
//! per-utterance transcript, character error rate, wall time, real-time
//! factor, and process RSS. Also exercises cancel, caller-error recovery,
//! output-too-small recovery, unload/reload, and a real model-read failure
//! during load followed by a successful reload.
//!
//! These are HOST measurements. They are not guest (Nagi/QEMU) results.
//!
//! Usage: m25-whisper-eval <eval-set.tsv> <model.bin> <model-bytes> <output.json>
//! The TSV (id, pcm path, reference) is produced by tests/m25-whisper/run-eval.sh
//! after it verifies each PCM SHA-256 against the committed manifest.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use nagi_audio::speech::{
    SpeechLanguage, SpeechOptions, SpeechProviderError, SpeechToTextProvider,
    MAX_SPEECH_PCM_CHUNK_BYTES, MAX_SPEECH_TRANSCRIPT_BYTES,
};
use nagi_audio::PcmFormat;
use nagi_m25_whisper_tests::host_engine::HostModelLoader;
use nagi_m25_whisper_tests::metrics::{character_errors, reset_peak_rss, rss_kib};
use nagi_m25_whisper_tests::provider_session::{WhisperSession, WhisperSessionState};

const JA: SpeechOptions = SpeechOptions {
    language: SpeechLanguage::Japanese,
    pcm_format: PcmFormat::mono_16khz(),
};

struct Clip {
    id: String,
    pcm: Vec<u8>,
    reference: String,
}

type Session = WhisperSession<HostModelLoader>;

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn rss() -> (u64, u64) {
    rss_kib().unwrap_or((0, 0))
}

fn transcribe(
    session: &mut Session,
    pcm: &[u8],
    output: &mut [u8],
) -> Result<usize, SpeechProviderError> {
    session.begin(JA)?;
    for chunk in pcm.chunks(MAX_SPEECH_PCM_CHUNK_BYTES) {
        session.push_pcm(PcmFormat::mono_16khz(), chunk)?;
    }
    session.finish(output)
}

struct Utterance {
    text: String,
    seconds: f64,
    peak_rss_kib: u64,
}

fn run_clip(session: &mut Session, clip: &Clip) -> Result<Utterance, SpeechProviderError> {
    reset_peak_rss();
    let mut output = [0_u8; MAX_SPEECH_TRANSCRIPT_BYTES];
    let start = Instant::now();
    let written = transcribe(session, &clip.pcm, &mut output)?;
    let seconds = start.elapsed().as_secs_f64();
    let text =
        String::from_utf8(output[..written].to_vec()).map_err(|_| SpeechProviderError::Failed)?;
    Ok(Utterance {
        text,
        seconds,
        peak_rss_kib: rss().1,
    })
}

fn check(checks: &mut Vec<(String, bool, String)>, name: &str, pass: bool, detail: String) {
    eprintln!("[{}] {name}: {detail}", if pass { "PASS" } else { "FAIL" });
    checks.push((name.to_string(), pass, detail));
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() != 5 {
        eprintln!("usage: m25-whisper-eval <eval-set.tsv> <model.bin> <model-bytes> <output.json>");
        std::process::exit(2);
    }
    let table = std::fs::read_to_string(&arguments[1]).expect("read eval set");
    let clips: Vec<Clip> = table
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.splitn(3, '\t');
            let id = fields.next().unwrap().to_string();
            let path = fields.next().expect("pcm path");
            let reference = fields.next().expect("reference").to_string();
            Clip {
                id,
                pcm: std::fs::read(path).expect("read pcm"),
                reference,
            }
        })
        .collect();
    assert!(clips.len() >= 4, "evaluation needs at least four clips");
    let model_bytes: u64 = arguments[3].parse().expect("model bytes");
    let mut session: Session = WhisperSession::new(HostModelLoader::new(
        PathBuf::from(&arguments[2]),
        model_bytes,
    ));
    let mut checks = Vec::new();
    let mut report = String::new();

    let (rss_before_load, _) = rss();
    reset_peak_rss();
    let load_start = Instant::now();
    session.load().expect("initial model load");
    let load_seconds = load_start.elapsed().as_secs_f64();
    let (rss_after_load, peak_load) = rss();

    // 1. Consecutive utterances on one context.
    let mut rows = Vec::new();
    let (mut total_edits, mut total_chars) = (0_usize, 0_usize);
    let (mut total_audio, mut total_time) = (0_f64, 0_f64);
    let mut peak_utterance = 0_u64;
    let mut first_transcripts = Vec::new();
    for clip in &clips {
        let audio_seconds = clip.pcm.len() as f64 / 32_000.0;
        match run_clip(&mut session, clip) {
            Ok(utterance) => {
                let (edits, chars) = character_errors(&clip.reference, &utterance.text);
                total_edits += edits;
                total_chars += chars;
                total_audio += audio_seconds;
                total_time += utterance.seconds;
                peak_utterance = peak_utterance.max(utterance.peak_rss_kib);
                eprintln!(
                    "{}: {:.1}s audio, {:.1}s, CER {:.3}: {}",
                    clip.id,
                    audio_seconds,
                    utterance.seconds,
                    edits as f64 / chars.max(1) as f64,
                    utterance.text
                );
                rows.push(format!(
                    "{{\"id\":{},\"audio_seconds\":{:.3},\"inference_seconds\":{:.3},\"rtf\":{:.3},\"edits\":{},\"reference_chars\":{},\"cer\":{:.4},\"peak_rss_kib\":{},\"reference\":{},\"hypothesis\":{}}}",
                    json_string(&clip.id), audio_seconds, utterance.seconds, utterance.seconds / audio_seconds,
                    edits, chars, edits as f64 / chars.max(1) as f64, utterance.peak_rss_kib,
                    json_string(&clip.reference), json_string(&utterance.text)
                ));
                first_transcripts.push(Some(utterance.text));
            }
            Err(error) => {
                eprintln!("{}: provider error {error:?}", clip.id);
                rows.push(format!(
                    "{{\"id\":{},\"error\":\"{error:?}\"}}",
                    json_string(&clip.id)
                ));
                first_transcripts.push(None);
            }
        }
    }
    let completed = first_transcripts
        .iter()
        .filter(|text| text.is_some())
        .count();
    check(
        &mut checks,
        "consecutive_utterances",
        completed == clips.len() && session.stats().loads == 1,
        format!(
            "{completed}/{} utterances on one context, loads={}",
            clips.len(),
            session.stats().loads
        ),
    );

    let rerun = |session: &mut Session, index: usize| -> (bool, String) {
        match run_clip(session, &clips[index]) {
            Ok(utterance) => {
                let same = first_transcripts[index].as_deref() == Some(utterance.text.as_str());
                (
                    same,
                    format!("rerun of {} identical={same}", clips[index].id),
                )
            }
            Err(error) => (
                false,
                format!("rerun of {} failed: {error:?}", clips[index].id),
            ),
        }
    };

    // 2. Cancel mid-utterance, then a full utterance.
    session.begin(JA).unwrap();
    let half = (clips[0].pcm.len() / 2) & !1;
    for chunk in clips[0].pcm[..half].chunks(MAX_SPEECH_PCM_CHUNK_BYTES) {
        session.push_pcm(PcmFormat::mono_16khz(), chunk).unwrap();
    }
    session.cancel();
    let idle = session.state() == WhisperSessionState::Idle && session.buffered_samples() == 0;
    let (same, detail) = rerun(&mut session, 1);
    check(
        &mut checks,
        "cancel_then_next_utterance",
        idle && same,
        format!("idle_after_cancel={idle}; {detail}"),
    );

    // 3. Caller error (odd-length PCM) fails only that utterance.
    session.begin(JA).unwrap();
    let rejected = session.push_pcm(PcmFormat::mono_16khz(), &clips[2].pcm[..3])
        == Err(SpeechProviderError::Failed);
    let (same, detail) = rerun(&mut session, 2);
    check(
        &mut checks,
        "resume_after_malformed_pcm",
        rejected && same && session.is_loaded(),
        format!("rejected={rejected}; {detail}"),
    );

    // 4. Output too small keeps the context; the next utterance succeeds.
    let mut tiny = [0_u8; 8];
    let too_small = transcribe(&mut session, &clips[3].pcm, &mut tiny)
        == Err(SpeechProviderError::OutputTooSmall);
    let still_loaded = session.is_loaded() && session.stats().consecutive_engine_failures == 0;
    let (same, detail) = rerun(&mut session, 3);
    check(
        &mut checks,
        "resume_after_output_too_small",
        too_small && still_loaded && same,
        format!("output_too_small={too_small}; context_kept={still_loaded}; {detail}"),
    );

    // 5. Unload releases memory; a real model-read failure during reload is
    // reported; the following reload succeeds and transcribes identically.
    let (rss_before_unload, _) = rss();
    session.unload();
    let (rss_after_unload, _) = rss();
    let unloaded = session.state() == WhisperSessionState::Unloaded
        && session.begin(JA) == Err(SpeechProviderError::Unavailable);
    let next_attempt = session.loader().attempts + 1;
    // The loader is owned by the session; rebuild it with an injected failure.
    let mut failing_loader = HostModelLoader::new(PathBuf::from(&arguments[2]), model_bytes);
    failing_loader.attempts = next_attempt - 1;
    failing_loader.inject_read_failure_on_attempt = Some(next_attempt);
    let previous_stats = session.stats();
    let mut session_after: Session = WhisperSession::new(failing_loader);
    let failed_load =
        session_after.load() == Err(SpeechProviderError::Unavailable) && !session_after.is_loaded();
    let (rss_after_failed_load, _) = rss();
    let reload_start = Instant::now();
    let reloaded = session_after.load().is_ok();
    let reload_seconds = reload_start.elapsed().as_secs_f64();
    let (same, detail) = if reloaded {
        rerun(&mut session_after, 0)
    } else {
        (false, "reload failed".into())
    };
    check(
        &mut checks,
        "unload_failed_load_reload",
        unloaded && failed_load && reloaded && same,
        format!(
            "unloaded={unloaded}; rss_kib {rss_before_unload}->{rss_after_unload}; injected_read_failure_reported={failed_load}; rss_after_failed_load_kib={rss_after_failed_load}; reload_ok={reloaded} in {reload_seconds:.2}s; {detail}"
        ),
    );
    let after = session_after.stats();
    drop(session_after);
    drop(session);

    let all_pass = checks.iter().all(|(_, pass, _)| *pass);
    let _ = writeln!(report, "{{");
    let _ = writeln!(report, "  \"kind\": \"host-unseen-eval\",");
    let _ = writeln!(report, "  \"counts_as_guest_result\": false,");
    let _ = writeln!(report, "  \"dataset\": \"google/fleurs ja_jp validation (CC BY 4.0), see tests/m25-whisper/eval/fleurs-ja-validation.json\",");
    let _ = writeln!(
        report,
        "  \"threads\": {},",
        nagi_m25_whisper_tests::provider_ffi::WHISPER_THREADS
    );
    let _ = writeln!(report, "  \"clips\": {},", clips.len());
    let _ = writeln!(report, "  \"audio_seconds\": {total_audio:.3},");
    let _ = writeln!(report, "  \"inference_seconds\": {total_time:.3},");
    let _ = writeln!(
        report,
        "  \"aggregate_rtf\": {:.3},",
        total_time / total_audio.max(f64::MIN_POSITIVE)
    );
    let _ = writeln!(
        report,
        "  \"aggregate_cer\": {:.4},",
        total_edits as f64 / total_chars.max(1) as f64
    );
    let _ = writeln!(report, "  \"edits\": {total_edits},");
    let _ = writeln!(report, "  \"reference_chars\": {total_chars},");
    let _ = writeln!(report, "  \"model_load_seconds\": {load_seconds:.3},");
    let _ = writeln!(report, "  \"rss_kib\": {{\"before_load\": {rss_before_load}, \"after_load\": {rss_after_load}, \"peak_during_load\": {peak_load}, \"peak_during_any_utterance\": {peak_utterance}, \"before_unload\": {rss_before_unload}, \"after_unload\": {rss_after_unload}, \"after_failed_load\": {rss_after_failed_load}}},");
    let _ = writeln!(report, "  \"session_stats\": {{\"first_session\": \"{previous_stats:?}\", \"reload_session\": \"{after:?}\"}},");
    let _ = writeln!(report, "  \"lifecycle_checks\": [");
    for (index, (name, pass, detail)) in checks.iter().enumerate() {
        let comma = if index + 1 == checks.len() { "" } else { "," };
        let _ = writeln!(
            report,
            "    {{\"name\": {}, \"pass\": {pass}, \"detail\": {}}}{comma}",
            json_string(name),
            json_string(detail)
        );
    }
    let _ = writeln!(report, "  ],");
    let _ = writeln!(report, "  \"lifecycle_pass\": {all_pass},");
    let _ = writeln!(report, "  \"utterances\": [");
    for (index, row) in rows.iter().enumerate() {
        let comma = if index + 1 == rows.len() { "" } else { "," };
        let _ = writeln!(report, "    {row}{comma}");
    }
    let _ = writeln!(report, "  ]");
    let _ = writeln!(report, "}}");
    std::fs::write(&arguments[4], report).expect("write report");
    std::process::exit(if all_pass { 0 } else { 1 });
}
