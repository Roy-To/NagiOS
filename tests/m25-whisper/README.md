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
  PCM capacity grows geometrically but never above `MAX_WHISPER_SAMPLES`
  (524,288 f32 = 2 MiB). `cancel`/`finish`/failed utterances zero and clear
  the buffer but keep the allocation for reuse; `unload`, the automatic
  release after repeated engine failures, and drop free it with the context.

**Cancellation limit.** `cancel` only discards an utterance that is still
being captured. `finish` runs whisper.cpp to completion with no abort
callback, so in-flight inference cannot be cancelled. Production cancellation
is therefore NOT complete; the tests below cover capture-time cancel only.
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
cargo test --locked --offline            # 24 tests
cargo fmt -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
```

- `tests/lifecycle.rs` (15) — SCRIPTED ENGINE, SAME SESSION (orchestration
  only, not speech recognition): consecutive utterances, capture-time cancel,
  cancel after `finish` being a no-op (no inference abort), unload/reload of
  the same `WhisperSession`, load failure and retry, malformed PCM, oversize
  utterance, output too small, repeated engine failure releasing the context,
  empty/invalid transcripts.
- `tests/capacity.rs` (4) — SCRIPTED ENGINE, SAME SESSION: irregular chunks
  up to the limit (256 × 4094 B + 512 B) keep capacity ≤ `MAX_WHISPER_SAMPLES`
  (before the fix: 1,048,064); an odd chunk mix stays bounded; cancel/finish
  retain the zeroed allocation; unload frees it (capacity 0).
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
transcript, CER, wall time, real-time factor, and RSS, and checks (REAL
ENGINE): consecutive utterances, capture-time cancel, malformed-PCM recovery
and output-too-small recovery on the same session; then
`unload_then_new_session_failed_load_reload`: unload of the first session,
followed by an injected model-read failure and a successful reload in a NEW
`WhisperSession` (not a same-session reload) that reproduces the earlier
transcript. Each check's `session_scope` is recorded in the output JSON.

Evaluation audio: FLEURS (google/fleurs, ja_jp validation, revision
70bb2e84b976b7e960aa89f1c648e09c59f894dd), CC BY 4.0. Conneau et al., "FLEURS:
Few-shot Learning Evaluation of Universal Representations of Speech",
arXiv:2205.12446 (2022). Audio is fetched, not committed; it was converted from
32-bit float WAV to 16-bit PCM without other modification.

Results and environment: `.dev/workstreams/hark-m25-whisper/evidence/`.
