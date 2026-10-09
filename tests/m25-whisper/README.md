# M25 Whisper provider — host harness

Standalone crate (own `[workspace]` and `Cargo.lock`; not a root-workspace
member). It compiles the same lifecycle source the Nagi guest provider uses:

- `tools/whisper/provider_session.rs` — `WhisperSession<L: WhisperEngineLoader>`:
  explicit `load`/`unload`, `SpeechToTextProvider` (`begin`/`push_pcm`/
  `finish`/`cancel`), bounded buffering (1 MiB utterance, 4 KiB chunks),
  transcript trimming and UTF-8 checks, failure accounting
  (`WhisperSessionStats`), and release of the context after
  `MAX_CONSECUTIVE_ENGINE_FAILURES` (2) consecutive engine-side failures.
  Single-threaded; inference runs synchronously in `finish`.
- `tools/whisper/provider_ffi.rs` — `AdapterEngine` over
  `tools/whisper/nagi-provider-adapter.cpp`, `WHISPER_THREADS = 1`.

Fixture input and expected text exist only in
`user/nagi-init/src/m25_whisper.rs`'s `fixture_acceptance` module. The guest
fixture check never counts as recognition evidence; unseen-speech evaluation
uses the FLEURS set below, and the harness refuses a clip whose hash equals
`NAGI_M25_WHISPER_FIXTURE_SHA256` when that is set.

## Tests (no model, no network)

```sh
cd tests/m25-whisper
cargo test --locked --offline            # 19 tests
cargo fmt -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
```

- `tests/lifecycle.rs` — orchestration only (scripted test engine, not speech
  recognition): consecutive utterances, cancel, unload/reload, load failure and
  retry, malformed PCM, oversize utterance, output too small, repeated engine
  failure releasing the context, empty/invalid transcripts.
- `tests/fixture_separation.rs` — source guards for fixture/production
  separation and the licensed evaluation manifest.
- `tests/metrics.rs` — CER normalization and edit distance.

`guest-shape/` type-checks `user/nagi-init/src/m25_whisper.rs` against the real
`nagi-audio` and `nagi-model-manager` APIs on the host with a `relibc` stub and
placeholder build-staged files (`cargo check`/`clippy` only; never linked or
run; not a guest result):

```sh
cd tests/m25-whisper/guest-shape
cargo clippy --locked --offline -- -D warnings
```

## Unseen Japanese evaluation (real engine, HOST measurement)

```sh
tools/whisper/build-host-eval.sh            # pinned 927cfce + patches 0001-0003
tools/whisper/fetch-fleurs-ja-eval.py       # 12 clips, hash-verified
tests/m25-whisper/run-eval.sh [output.json]
```

`run-eval.sh` verifies the model against `third_party/models.lock` and every
PCM against `eval/fleurs-ja-validation.json`, then runs `m25-whisper-eval`
(`--features real-engine`) under `nice -n 19`. It records per-utterance
transcript, CER, wall time, real-time factor, and RSS, and checks cancel,
malformed-PCM recovery, output-too-small recovery, unload, an injected model
read failure during reload (real adapter load-failure path), and a successful
reload that reproduces the earlier transcript.

Evaluation audio: FLEURS (google/fleurs, ja_jp validation, revision
70bb2e84b976b7e960aa89f1c648e09c59f894dd), CC BY 4.0. Conneau et al., "FLEURS:
Few-shot Learning Evaluation of Universal Representations of Speech",
arXiv:2205.12446 (2022). Audio is fetched, not committed; it was converted from
32-bit float WAV to 16-bit PCM without other modification.

Results and environment: `.dev/workstreams/hark-m25-whisper/evidence/`.
