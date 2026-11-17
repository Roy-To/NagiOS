//! Host worst-case measurement for one bounded input.
//!
//! ```sh
//! worst_case VOICE DICT stereo48k|mono16k KIND
//! ```
//!
//! KIND selects a built-in input of at most 1 KiB (see `text_for`), or
//! `fit:<N>` repeats a short sentence N times. Reports, all wall-clock on
//! the host:
//!
//! - `plan_seconds`: text front end + duration model only,
//! - `begin_seconds`: full `begin` (plan + parameter generation, or rejection),
//! - `first_chunk_seconds`, `max_chunk_seconds`, `drain_seconds`,
//! - `cancel_seconds`: cost of `cancel()` after the first chunk, measured in
//!   a second run of the same input,
//! - `vm_hwm_kib`: peak resident set of this process (Linux `/proc`).
//!
//! The provider API is synchronous: a caller can only cancel between
//! `begin`/`next_pcm_chunk` calls, so the worst-case cancel latency is
//! `max(begin_seconds, max_chunk_seconds) + cancel_seconds`.
//! Exit status: 0 accepted and fully drained, 3 rejected by the provider
//! (bounded error, no audio), 1 usage/load error.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use nagi_audio::speech::{
    SpeechSynthesisLanguage, SpeechSynthesisOptions, SynthesisPcmChunk, TextToSpeechProvider,
    MAX_SPEECH_PCM_CHUNK_BYTES, MAX_SPEECH_SYNTHESIS_TEXT_BYTES,
};
use nagi_audio::PcmFormat;
use nagi_tts_provider::jbonsai_backend::{JbonsaiBackend, JbonsaiProvider};
use nagi_tts_provider::{max_frames, LocalTtsProvider};

fn fill(unit: &str) -> String {
    let mut text = String::new();
    while text.len() + unit.len() <= MAX_SPEECH_SYNTHESIS_TEXT_BYTES {
        text.push_str(unit);
    }
    text
}

fn text_for(kind: &str) -> Option<String> {
    Some(match kind {
        "sentences" => fill("今日はとても良い天気ですね。"),
        "kanji" => fill("東京特許許可局"),
        "hiragana" => fill("あ"),
        "katakana" => fill("ア"),
        "digits" => fill("1234567890"),
        "latin" => fill("a"),
        "punctuation" => fill("、"),
        "mixed" => fill("Nagi OSで12時に会議、"),
        _ => {
            let count: usize = kind.strip_prefix("fit:")?.parse().ok()?;
            "今日はとても良い天気ですね。".repeat(count)
        }
    })
}

fn proc_status_kib(key: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix(key))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
}

fn vm_hwm_kib() -> Option<u64> {
    proc_status_kib("VmHWM:")
}

fn vm_rss_kib() -> Option<u64> {
    proc_status_kib("VmRSS:")
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 4 {
        eprintln!("usage: worst_case VOICE DICT stereo48k|mono16k KIND");
        return ExitCode::from(1);
    }
    let (voice, dict) = (PathBuf::from(&args[0]), PathBuf::from(&args[1]));
    let format = match args[2].as_str() {
        "stereo48k" => PcmFormat::stereo_48khz(),
        "mono16k" => PcmFormat::mono_16khz(),
        _ => {
            eprintln!("unknown format");
            return ExitCode::from(1);
        }
    };
    let Some(text) = text_for(&args[3]) else {
        eprintln!("unknown kind");
        return ExitCode::from(1);
    };
    if text.len() > MAX_SPEECH_SYNTHESIS_TEXT_BYTES {
        eprintln!("input over 1 KiB");
        return ExitCode::from(1);
    }
    let backend = match JbonsaiBackend::load(&voice, &dict) {
        Ok(backend) => backend,
        Err(error) => {
            eprintln!("load error: {error:?}");
            return ExitCode::from(1);
        }
    };
    println!(
        "kind={} format={} text_bytes={}",
        args[3],
        args[2],
        text.len()
    );
    println!(
        "vm_hwm_after_load_kib={} vm_rss_after_load_kib={}",
        vm_hwm_kib().unwrap_or(0),
        vm_rss_kib().unwrap_or(0)
    );

    let start = Instant::now();
    let plan = backend.plan(&text);
    let plan_seconds = start.elapsed().as_secs_f64();
    match &plan {
        Ok(Some((plan, _))) => println!(
            "plan_seconds={plan_seconds:.4} labels={} total_frames={} emitted_frames={} budget_frames={}",
            plan.labels,
            plan.total_frames,
            plan.emitted_frames(),
            max_frames(format, 240).unwrap_or(0)
        ),
        Ok(None) => println!("plan_seconds={plan_seconds:.4} nothing_speakable=true"),
        Err(error) => println!("plan_seconds={plan_seconds:.4} plan_error={error:?}"),
    }
    drop(plan);

    let mut provider: JbonsaiProvider = LocalTtsProvider::new(backend).expect("provider");
    let options = SpeechSynthesisOptions {
        language: SpeechSynthesisLanguage::Japanese,
        pcm_format: format,
    };
    let start = Instant::now();
    let begun = provider.begin(&text, options);
    let begin_seconds = start.elapsed().as_secs_f64();
    if let Err(error) = begun {
        println!("begin_seconds={begin_seconds:.4} result=rejected error={error:?} pcm_bytes=0");
        println!("vm_hwm_kib={}", vm_hwm_kib().unwrap_or(0));
        return ExitCode::from(3);
    }
    println!("vm_rss_after_begin_kib={}", vm_rss_kib().unwrap_or(0));
    let mut chunk = vec![0u8; MAX_SPEECH_PCM_CHUNK_BYTES];
    let mut first_chunk_seconds = None;
    let mut max_chunk_seconds = 0.0f64;
    let mut total = 0usize;
    loop {
        let call = Instant::now();
        let next = provider.next_pcm_chunk(&mut chunk);
        max_chunk_seconds = max_chunk_seconds.max(call.elapsed().as_secs_f64());
        match next {
            Ok(SynthesisPcmChunk::Data(n)) => {
                first_chunk_seconds.get_or_insert(start.elapsed().as_secs_f64());
                total += n;
            }
            Ok(SynthesisPcmChunk::End) => break,
            Err(error) => {
                println!("result=failed_mid_stream error={error:?} pcm_bytes={total}");
                println!("vm_hwm_kib={}", vm_hwm_kib().unwrap_or(0));
                return ExitCode::from(4);
            }
        }
    }
    let drain_seconds = start.elapsed().as_secs_f64();
    let bytes_per_second = f64::from(format.sample_rate()) * f64::from(format.channels()) * 2.0;
    println!(
        "begin_seconds={begin_seconds:.4} result=accepted first_chunk_seconds={:.4} \
         max_chunk_seconds={max_chunk_seconds:.5} drain_seconds={drain_seconds:.4} \
         pcm_bytes={total} audio_seconds={:.3}",
        first_chunk_seconds.unwrap_or(0.0),
        total as f64 / bytes_per_second
    );

    // Second run: cancel right after the first chunk.
    provider.begin(&text, options).expect("second begin");
    let _ = provider.next_pcm_chunk(&mut chunk);
    let cancel = Instant::now();
    provider.cancel();
    let cancel_seconds = cancel.elapsed().as_secs_f64();
    assert!(provider.is_clear());
    println!(
        "cancel_seconds={cancel_seconds:.6} worst_cancel_latency_seconds={:.4}",
        begin_seconds.max(max_chunk_seconds) + cancel_seconds
    );
    println!("vm_hwm_kib={}", vm_hwm_kib().unwrap_or(0));
    ExitCode::SUCCESS
}
