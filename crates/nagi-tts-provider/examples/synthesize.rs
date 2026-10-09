//! Host measurement tool: synthesizes Japanese text through the provider
//! contract and writes a WAV file.
//!
//! ```sh
//! cargo run --release --example synthesize -- VOICE DICT OUT.wav [stereo48k|mono16k] TEXT...
//! ```
//!
//! Prints load time, synthesis wall time, audio duration, and real-time
//! factor. Run under `/usr/bin/time -v` for peak RSS and CPU time. This is
//! host evidence only; it does not run on Nagi.

use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use nagi_audio::speech::{
    SpeechSynthesisLanguage, SpeechSynthesisOptions, SynthesisPcmChunk, TextToSpeechProvider,
    MAX_SPEECH_PCM_CHUNK_BYTES,
};
use nagi_audio::PcmFormat;
use nagi_tts_provider::jbonsai_backend::load_provider;

fn main() {
    let mut args = std::env::args().skip(1);
    let usage = "usage: synthesize VOICE DICT OUT.wav [stereo48k|mono16k] TEXT...";
    let voice = PathBuf::from(args.next().expect(usage));
    let dict = PathBuf::from(args.next().expect(usage));
    let out = PathBuf::from(args.next().expect(usage));
    let format_name = args.next().expect(usage);
    let format = match format_name.as_str() {
        "stereo48k" => PcmFormat::stereo_48khz(),
        "mono16k" => PcmFormat::mono_16khz(),
        _ => panic!("{usage}"),
    };
    let texts: Vec<String> = args.collect();
    assert!(!texts.is_empty(), "{usage}");

    let load_start = Instant::now();
    let mut provider = load_provider(&voice, &dict).expect("load voice and dictionary");
    let load_seconds = load_start.elapsed().as_secs_f64();
    println!("load_seconds={load_seconds:.3}");

    let options = SpeechSynthesisOptions {
        language: SpeechSynthesisLanguage::Japanese,
        pcm_format: format,
    };
    let bytes_per_second = f64::from(format.sample_rate()) * f64::from(format.channels()) * 2.0;
    let mut all_pcm = Vec::new();
    let mut chunk = vec![0u8; MAX_SPEECH_PCM_CHUNK_BYTES];
    for (index, text) in texts.iter().enumerate() {
        let start = Instant::now();
        provider.begin(text, options).expect("begin");
        let mut first_chunk_seconds = None;
        let mut pcm = Vec::new();
        loop {
            match provider.next_pcm_chunk(&mut chunk) {
                Ok(SynthesisPcmChunk::Data(n)) => {
                    first_chunk_seconds.get_or_insert(start.elapsed().as_secs_f64());
                    pcm.extend_from_slice(&chunk[..n]);
                }
                Ok(SynthesisPcmChunk::End) => break,
                Err(error) => panic!("utterance {index} failed: {error:?}"),
            }
        }
        let wall = start.elapsed().as_secs_f64();
        let audio = pcm.len() as f64 / bytes_per_second;
        println!(
            "utterance={index} text_bytes={} pcm_bytes={} audio_seconds={audio:.3} \
             first_chunk_seconds={:.4} synth_seconds={wall:.4} rtf={:.4}",
            text.len(),
            pcm.len(),
            first_chunk_seconds.unwrap_or(0.0),
            wall / audio.max(1e-9),
        );
        all_pcm.extend_from_slice(&pcm);
    }
    write_wav(&out, format, &all_pcm);
    println!("wav={} format={format_name}", out.display());
}

fn write_wav(path: &PathBuf, format: PcmFormat, pcm: &[u8]) {
    let channels = format.channels();
    let rate = format.sample_rate();
    let block_align = channels * 2;
    let byte_rate = rate * u32::from(block_align);
    let mut file = std::fs::File::create(path).expect("create wav");
    let data_len = u32::try_from(pcm.len()).expect("wav size");
    let mut header = Vec::with_capacity(44);
    header.extend_from_slice(b"RIFF");
    header.extend_from_slice(&(36 + data_len).to_le_bytes());
    header.extend_from_slice(b"WAVEfmt ");
    header.extend_from_slice(&16u32.to_le_bytes());
    header.extend_from_slice(&1u16.to_le_bytes());
    header.extend_from_slice(&channels.to_le_bytes());
    header.extend_from_slice(&rate.to_le_bytes());
    header.extend_from_slice(&byte_rate.to_le_bytes());
    header.extend_from_slice(&block_align.to_le_bytes());
    header.extend_from_slice(&16u16.to_le_bytes());
    header.extend_from_slice(b"data");
    header.extend_from_slice(&data_len.to_le_bytes());
    file.write_all(&header).expect("write header");
    file.write_all(pcm).expect("write pcm");
}
