# M25 local TTS — host measurements, streaming behaviour, std/Nagi status

Everything here is **host evidence** (Linux 6.12 aarch64, Neoverse-V2,
2 vCPU, 8 GB, shared and loaded sandbox: load average ≈ 2). Nothing in this
file was run on Nagi or QEMU, and no audio went through
`AudioServicePlaybackSink`. Host results are not guest PASS.

Raw outputs:

- `evidence/host-aarch64-20261009.txt` — `tests/m25-tts/measure.sh` output
  (release build, `codegen-units = 1`, single thread; whisper.cpp
  `ggml-base` used only as an independent intelligibility check).
- `evidence/acceptance-run1/*.txt` — `tests/m25-tts/acceptance.sh` logs
  (`*.log` renamed to `*.txt` because `*.log` is git-ignored).
- `evidence/acceptance-final/*.txt` — the same runner re-run on the
  committed tree (see §6).
- `evidence/nagi-target-20261009.txt` — Nagi target check/build log (§5).

Pinned inputs: voice `tohoku-f01-neutral.htsvoice` SHA-256
`ded6acb4…0b46`, dictionary `naist-jdic-jpreprocess.tar.gz` SHA-256
`8a930bbc…17b8` (`tools/tts/tts-artifacts.lock`).

## 1. Correctness (host)

| check | result |
|---|---|
| model-free unit tests (`cargo test`, artifacts unset) | 20 passed; the 12 real-engine tests reported **ignored**, not passed |
| real-engine tests (`--test real_engine -- --ignored`, pinned artifacts) | 12 passed |
| real suite without artifacts | fails (exit 101, `FAIL: NAGI_TTS_VOICE is not set`) — no silent skip |
| `cargo fmt --check`, `clippy -D warnings` (engine on and `--no-default-features`) | pass |

The real-engine tests cover audible stereo 48 kHz output, determinism across
reuse and chunk sizes, mono 16 kHz = one third of the rate, cancel mid-stream
then reuse, no fake audio for English/unspeakable text, over-length rejection
before audio, the `SpeechSynthesisService` contract, missing/invalid
artifacts, unload/reload, edge-silence trimming, bytes loader == path loader,
and predicted length == emitted length.

## 2. Latency and real-time factor (host, release, 1 thread)

| case | format | audio s | first chunk s | synth s | RTF |
|---|---|---|---|---|---|
| こんにちは、ナギです。 | stereo 48k | 1.695 | 0.0081 | 0.0245 | 0.0145 |
| アルバートを開いて | stereo 48k | 1.375 | 0.0076 | 0.0210 | 0.0153 |
| 今日の天気は晴れです。 | stereo 48k | 1.720 | 0.0079 | 0.0248 | 0.0144 |
| ファイルを保存しました。 | stereo 48k | 1.640 | 0.0079 | 0.0240 | 0.0146 |
| こんにちは、ナギです。 | mono 16k | 1.698 | 0.0091 | 0.0277 | 0.0163 |
| 会議は午後三時から、参加者は12人です。 | stereo 48k | 3.695 | 0.0102 | 0.0468 | 0.0127 |
| 日本語の音声合成が、ローカルで動作しています。 | stereo 48k | 3.990 | 0.0110 | 0.0505 | 0.0127 |
| 今日はとても良い天気ですね。×12 (504 B) | mono 16k | 31.608 | 0.0398 | 0.4107 (drain) | 0.013 |
| あ×341 (1023 B, largest accepted input) | mono 16k | 32.533 | 0.0406 | 0.4350 (drain) | 0.013 |

Load (voice + dictionary from files into memory, then `from_bytes`):
0.065–0.097 s.

Whisper (ggml-base) transcripts of the WAVs match the input text except
「ナギ」→「何」 and 「合成」→「後声」, i.e. the speech is intelligible
Japanese; numbers are read correctly (「午後3時」「12人」).

## 3. Memory (host, Linux `/proc` VmRSS / VmHWM)

| point | value |
|---|---|
| resident after load (voice + dictionary + engine) | 86,392–86,440 KiB (≈ 86 MB / 84.4 MiB) |
| peak during load (VmHWM; file bytes + parsed dictionary briefly coexist) | 120,540–120,732 KiB (≈ 118 MiB) |
| largest accepted utterance, after `begin` (あ×341, mono 16k) | 92,308 KiB, i.e. **+5,876 KiB (≈ +6 MB)** over the loaded state |
| 2-sentence stereo utterance after `begin` | 87,316 KiB (+884 KiB) |
| process peak across every case | 120,732 KiB — no utterance exceeded the load peak |

Per-utterance memory is the jbonsai parameter/duration state built at
`begin`; the provider's own PCM staging is fixed (4 KiB) and the PCM chunk is
caller-owned. Rejected inputs allocate no PCM.

## 4. Streaming behaviour

### 4.1 Cancel checkpoints

The provider API is synchronous. A caller can stop an utterance only between
calls:

1. after `begin` returns (front end + duration model + parameter
   generation for the whole utterance happen inside `begin`; there is no
   checkpoint inside it),
2. after any `next_pcm_chunk` returns (each call vocodes at most one 4 KiB
   chunk, ≈ 4–5 HTS frames of 5 ms),
3. `cancel()` itself erases all per-utterance state (staging zeroed, source
   dropped); the next `begin` starts clean (`cancel_mid_stream_*` tests).

`SpeechSynthesisService::speak` (in `user/nagi-audio`, read-only here)
drives the loop to completion in one call and offers no external cancel
point; a guest that needs barge-in must drive the provider chunk by chunk or
the service needs a cancel hook (contract owner decision).

### 4.2 Measured first-chunk and cancel latency (worst bounded inputs)

| input | format | begin s | first chunk s | max chunk s | cancel() s | worst cancel latency s |
|---|---|---|---|---|---|---|
| あ×341 (1023 B) | mono 16k | 0.0346 | 0.0406 | 0.00584 | 0.000307 | 0.0349 |
| sentence×12 (504 B) | mono 16k | 0.0337 | 0.0398 | 0.00600 | 0.000308 | 0.0340 |
| sentence×2 (84 B) | stereo 48k | 0.0066 | 0.0115 | 0.00484 | 0.000060 | 0.0067 |
| 、×341 (nothing speakable) | stereo 48k | 0.0003 | — | — | 0 | 0.0003 |

Worst cancel latency = max(begin, max chunk) + cancel(). Rejected worst-case
inputs (1 KiB of sentences, kanji, digits, Latin, mixed) are refused in
0.0001–0.0037 s with no audio.

### 4.3 Output cap: explicit failure, never truncation

- Cap: `MAX_SPEECH_SYNTHESIS_UTTERANCE_BYTES` = 1 MiB per utterance.
  Stereo 48 kHz S16LE = 192,000 B/s → **max 5.46 s** (budget 1,092 HTS
  frames). Mono 16 kHz = 32,000 B/s → max 32.77 s (6,552 frames).
- At `begin` the provider runs the front end and duration model, predicts
  the exact emitted length after edge-silence trimming, and rejects an
  over-budget utterance **before any PCM exists**. The provider returns
  `SpeechProviderError::Failed`; `SpeechSynthesisService::speak` reports it
  as **`ProviderFailed`** (the contract has no provider-level
  "too long"; it is not `UtteranceTooLong`). The playback sink receives
  nothing.
- Measured: sentence×2 (4.985 s stereo) accepted, sentence×3 (predicted
  1,536 frames) rejected; sentence×12 mono accepted (31.6 s), ×13 rejected;
  the 96-byte sentence 「日本語の音声合成が、ローカルで正しく動作しているかを確認します。」
  is rejected in stereo 48k. Every 1 KiB worst-case input except 1 KiB of
  hiragana in mono is rejected.
- If emitted bytes ever passed the cap mid-stream (a prediction mismatch),
  the provider zeroes the chunk and fails with `Failed`; it never returns
  truncated audio as success.

### 4.4 Playback is not atomic

`SpeechSynthesisService::speak` hands each chunk to the sink as soon as it
is produced. The over-length case cannot produce partial audio (it fails at
`begin`), but any other failure after the first chunk (engine error,
playback error) leaves the chunks already given to the sink played. There
is no all-or-nothing guarantee for an utterance on the guest.

## 5. std and the Nagi target

### 5.1 What was built (host machine, Nagi target triple)

Target: `targets/x86_64-unknown-nagi-user.json`, toolchain
`nightly-2025-08-01`. Log: `evidence/nagi-target-20261009.txt`.

| step | command (abridged) | result |
|---|---|---|
| no_std core check (engine off) | `cargo check --no-default-features --target …nagi-user.json -Zbuild-std=core,alloc` | exit 0 (check only) |
| engine-enabled check | `__CARGO_TESTS_ONLY_SRC_ROOT=out/rust-src/library cargo check --lib --target …nagi-user.json -Zbuild-std=std,panic_abort` | exit 0 (5 m 39 s at `-j1`, std rebuilt) |
| engine-enabled release build | same with `cargo build --release --lib` | exit 0, `libnagi_tts_provider.rlib` 223,418 bytes (rlib only) |

Script: `tests/m25-tts/nagi-target.sh`. On the Nagi target the patched libc
emits 1 warning (`FD_CLOEXEC` never used); no warning comes from this crate.

- `out/rust-src` is the pinned toolchain's `rust-src` with
  `third_party/rust-std/patches/0001-nagi-target-support.patch` applied and
  the `libc = { path = "../../../third_party/libc" }` patch appended to
  `library/Cargo.toml`, as `prepare_nagi_rust_std_source` in
  `tools/nagi-cli` does for `m13-std`.
- The crate itself carries `[patch.crates-io] libc = { path =
  "../../third_party/libc" }` because `memmap2` (the only libc user in the
  graph) must see `target_os = "nagi"`. On host builds this patched libc
  emits **4 warnings** (unexpected `cfg` value `nagi` ×2, `setgroups` never
  used, `setgroups` redeclared with a different signature); they are in the
  dependency and do not fail `clippy -D warnings` for this crate.
- **Nothing was linked into `nagi-init`, no image was built, and nothing
  ran in the guest.** A target rlib proves the code compiles for Nagi's std;
  it does not prove the std calls behave on Nagi.

### 5.2 std facilities the engine graph references

From `STD_INVENTORY.txt` (static scan of the 94 crates of the
engine-enabled graph; references, not proven call paths):

- **fs / path / io / env**: lindera-dictionary (fs×15, path×20, io×46,
  env×8), jbonsai (fs×6), jpreprocess (path×7), jpreprocess-dictionary,
  glob, anyhow. On the guest load path (`JbonsaiBackend::from_bytes`) the
  provider reads no file; this crate's own `std::fs` use is limited to the
  host convenience `JbonsaiBackend::load` / `DictionaryBytes::read_dir`.
- **mmap**: `memmap2` 0.9.11 (libc `mmap`/`munmap`/`madvise`/`mlock`/
  `mprotect`/`mremap`/`msync`/`sysconf`) is compiled in only because
  jpreprocess, jpreprocess-core and jpreprocess-dictionary depend on
  `lindera` with default features, and lindera's default feature `mmap`
  enables `lindera-dictionary/mmap`. The bytes loader never maps a file.
  **Proposal:** disable it upstream (`default-features = false` on
  `lindera` in jpreprocess) or via a pinned `[patch]`; it cannot be turned
  off from this crate's manifest because the dependents request the default
  feature.
- threads / process / net appear in transitive crates (regex-automata,
  once_cell, log, rkyv, bincode…) but the synthesis path is single-threaded
  and uses no process or network API (inferred from the provider code; not
  traced in the guest).

### 5.3 Allocator, stack, memory on Nagi

- **Allocator**: the provider uses `alloc` only; on Nagi it would use
  whatever global allocator the `nagi-init` std build provides. Not
  exercised on Nagi.
- **Stack**: **unmeasured.** No stack high-water figure exists for host or
  guest. The provider keeps its staging buffer (4 KiB) inside the struct and
  jbonsai keeps per-utterance parameters on the heap, but jbonsai's
  generation recursion/stack frames were not measured. A guest run must
  measure it before the thread stack size is fixed.
- **Memory budget** (host numbers, §3): ≈ 86 MB resident after load,
  ≈ 118 MiB peak while loading, +≈ 6 MB for the largest accepted
  utterance. Loading the dictionary as bytes from the Model Store would
  replace the file read but keeps the same parsed structures.

### 5.4 Proposed guest feature (shared-owner change)

`m25-tts-std = ["m13-std", "dep:nagi-tts-provider"]` in
`user/nagi-init/Cargo.toml`, built exactly like `m13-std`
(`--target targets/x86_64-unknown-nagi-user.json
-Zbuild-std=std,panic_abort --release --locked --offline` with the patched
rust-src), plus the root `[patch.crates-io] libc` mirror. Exact text:
`.dev/workstreams/hark-m25-tts/integration-proposal.md` §2f.

## 6. Re-verification on the committed tree

`tests/m25-tts/acceptance.sh` was re-run after committing; logs are in
`evidence/acceptance-final/`.
