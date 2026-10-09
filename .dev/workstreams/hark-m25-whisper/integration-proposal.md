# Integration proposal — hark-m25-whisper

Status: PROPOSED. Nothing below is applied outside this stream's owned paths.

## Base and branch

- Base: `f74f128abe40e9e99d7cb05f3f8dc7b90b1448c9` (main after PR #36). Work
  started at `edad2e7`; the rebase onto `f74f128` was accepted by the user.
  The `edad2e7..f74f128` delta is **not** registry-only: it is PR #36
  Writer/Sheets host adoption, 48 files (+7856/−3): new standalone crates
  `crates/nagi-writer-core/**` and `crates/nagi-sheets-core/**` (own
  `[workspace]` + `Cargo.lock`, not members of the root workspace),
  `tests/writer-core/**`, `tests/sheets-core/**`,
  `.github/workflows/0.2-host-integration.yml`,
  `docs/0.2/adoption/WRITER_SHEETS_CORE.md`, `.dev/workstreams.json` (new
  `writer-sheets-core-adoption` row) and `.dev/workstreams/{writer-core-01,
  sheets-calc-01,writer-sheets-core-adoption}/**`.
- History audit (2026-10-09 10:50 JST): `git diff --name-status f74f128 HEAD`
  and `git diff --name-status origin/main origin/hark/m25-whisper-production`
  list only owned paths (28 entries: 27 added, `m25_whisper.rs` modified), so
  nothing from main is dropped or reverted. No interaction with this stream:
  the delta does not touch root `Cargo.toml`/`Cargo.lock`, `user/**`,
  `kernel/**`, `third_party/**`, `tools/whisper/**`, `tools/nagi-cli/**` or
  `.github/workflows/ci.yml`; neither new crate is a dependency of
  `nagi-init`, and none references whisper. The new workflow's `pull_request`
  path filter matches `.dev/**`, but its only job is gated to a fixed list of
  `codex/*`/`claude/*` branches, so it is skipped on this branch (run
  37868348688 at 0d280f2: `skipped`).
- Branch: `hark/m25-whisper-production` (draft PR to `main`).
- Owned paths changed: `user/nagi-init/src/m25_whisper.rs`, `tools/whisper/**`,
  `tests/m25-whisper/**`, `.dev/workstreams/hark-m25-whisper/**`.

## APIs provided

`tools/whisper/provider_session.rs` (no_std + alloc; included by `#[path]`):

| Item | Contract |
|---|---|
| `trait WhisperEngineLoader { type Engine; fn load(&mut self) -> Option<Engine> }` | Builds a fresh context; on Nagi only from the Model Store capability. |
| `trait WhisperEngine { fn transcribe(&mut self, &[f32], WhisperLanguage, &mut [u8]) -> Result<usize, WhisperEngineError> }` | One synchronous utterance. |
| `WhisperSession<L>` | `new` (unloaded), `load` (idempotent), `unload`, `is_loaded`, `state` (`Unloaded`/`Idle`/`Capturing`), `stats`, `buffered_samples`, `retained_sample_capacity`, `loader`; implements `nagi_audio::speech::SpeechToTextProvider`. |
| `MAX_CONSECUTIVE_ENGINE_FAILURES = 2` | Two consecutive engine-side failures (inference, invalid segment, empty or invalid UTF-8 output) release the context; `begin` then returns `Unavailable` until an explicit `load`. `OutputTooSmall` and caller errors do not count. |
| `WhisperEngineError::from_status` | Maps adapter return codes 1–5. |

`tools/whisper/provider_ffi.rs`: `AdapterEngine` (unsafe `init` over reader
callbacks, `Drop` frees the context), `WHISPER_THREADS = 1`. The C ABI in
`nagi-provider-adapter.cpp` is unchanged.

Provider behaviour changes visible to callers:

- `finish` now trims leading/trailing ASCII whitespace from the transcript
  (whisper.cpp segments start with a space). The coordinator's UTF-8 and
  non-empty checks are unchanged.
- `begin` on an unloaded session returns `Unavailable` (previously the
  provider could not exist unloaded).
- Malformed PCM, oversize utterances, and allocation failure in `push_pcm`
  now end the utterance (buffer cleared, state `Idle`) instead of leaving it
  open.

- PCM capacity is capped at `MAX_WHISPER_SAMPLES` (geometric growth with
  exact, capped reservations). Before `06ff021`, `try_reserve` could round a
  near-limit request up to double the previous capacity: 256 × 4094-byte
  chunks then one 512-byte chunk left 1,048,064 f32 (≈4 MiB) of capacity for
  524,288 samples (reproduced by `tests/capacity.rs` on `0d280f2`). Cancel,
  `finish` and failed utterances keep the zeroed allocation (≤ 2 MiB) for
  reuse; `unload`, the automatic release after repeated engine failures, and
  drop free it.

### Cancellation limitation (production cancellation NOT complete)

`cancel` works only while an utterance is being captured (between `begin`
and `finish`): it zeroes and drops the buffered PCM. `finish` calls
`whisper_full` synchronously on the single provider thread with no abort
callback (`nagi-provider-adapter.cpp` sets none), so an inference in progress
cannot be cancelled, and `cancel` after `finish` returns is a no-op. A real
abort needs an adapter/ABI change (whisper.cpp `abort_callback`) plus a way
for another thread or the scheduler to signal it; that is outside this
stream's owned paths and the single-thread design, and is proposed only.

### Test scope labels

- Scripted engine, same session: `tests/m25-whisper/tests/lifecycle.rs`,
  `tests/capacity.rs` — every unload/reload reuses one `WhisperSession`.
- Real engine (host whisper.cpp): `src/bin/m25-whisper-eval.rs` — four checks
  on one session/context, then `unload_then_new_session_failed_load_reload`,
  where the failed load and the reload run in a NEW `WhisperSession`
  (renamed from `unload_failed_load_reload` in `06ff021`; each check's
  `session_scope` is in the evidence JSON).

The guest fixture check (`m25_whisper::run`, unchanged signature and serial
markers) now also asserts, without a second inference, that cancel leaves the
engine idle with no buffered PCM and that unload makes `begin` fail.

## Requested shared changes (minimal diffs, for their owners)

1. Registry (`.dev/workstreams.json`) and schema
   (`.dev/schemas/workstreams.schema.json`): see `registration-proposal.md`.
   The schema's `owner_branch` pattern admits only `codex/*` and two
   `claude/0.2-*` branches, so a `hark/*` owner branch needs:

   ```diff
   -"pattern": "^(codex/[a-z0-9][a-z0-9./-]*|claude/0\\.2-(writer-core-01|sheets-calc-01))$"
   +"pattern": "^(codex/[a-z0-9][a-z0-9./-]*|hark/[a-z0-9][a-z0-9./-]*|claude/0\\.2-(writer-core-01|sheets-calc-01))$"
   ```

   (Alternative without a schema change: register the row under a
   `codex/*` integration branch that imports this branch.)

2. CI (shared workflows; owner: CI/integration): full workflow YAML and
   the current gap are in `ci-proposal.md` (host tests + guest-shape, a
   Nagi-target `nagi-init` build with `m25-whisper-inference-acceptance`,
   and a manual real-engine evaluation job). No workflow file is applied.

3. `user/nagi-init/Cargo.toml`, `build.rs`, `main.rs` (owner: nagi-init /
   Codex integration). Optional follow-up to make the fixture/production split
   structural rather than module-level:
   - add a feature `m25-whisper-provider` that compiles `m25_whisper`'s
     production loader without staging fixture files;
   - keep `m25-whisper-inference-acceptance = ["m25-whisper-provider", …]` as
     the only feature that makes `build.rs` stage
     `NAGI_M25_WHISPER_PCM_FIXTURE` / `NAGI_M25_WHISPER_EXPECTED_TEXT_FILE`;
   - `main.rs`: `#[cfg(all(target_os = "nagi", any(feature = "m25-whisper-provider", …)))] mod m25_whisper;`.
   No change is needed for the current acceptance to build.

4. `THIRD_PARTY_NOTICES.md` / `third_party/sources.lock` (owner: license/SBOM).
   FLEURS audio is not redistributed (fetched at evaluation time), so no
   notice is strictly required. If evaluation audio is ever cached in CI
   artifacts or images, add:

   > FLEURS (google/fleurs, ja_jp validation, revision
   > 70bb2e84b976b7e960aa89f1c648e09c59f894dd), CC BY 4.0. Conneau et al.,
   > arXiv:2205.12446. Converted from float32 WAV to 16-bit PCM.

5. `docs/implementation_status.md` / M25 workstream doc (owner: docs). Proposed
   addition under M25 "Remaining acceptance blockers", host evidence only:

   > Host (aarch64, 1 thread) evaluation of the shared Whisper provider on 12
   > unseen FLEURS ja clips (134.7 s): CER 10.25 %, RTF 1.54, peak RSS
   > 773 MiB. Guest timing/RAM on unseen speech not yet measured.

No change is proposed to `third_party/models.lock`, the whisper.cpp revision
or patches, the adapter ABI, `WHISPER_THREADS`, or the ADR 0042 memory budget.

## ADR 0042 budget check (host evidence, proposal only)

Host peak RSS for a 29.5 s utterance (near the 32.8 s / 1 MiB cap) was
791,300 KiB (≈773 MiB), below the 1.25 GiB guest heap cap. Host RSS includes
the binary and libc and excludes nothing guest-specific, so it is indicative
only; the guest budget is unchanged and must be re-verified by a guest run.

## Verification commands

```sh
cd tests/m25-whisper && cargo fmt -- --check && cargo test --locked --offline \
  && cargo clippy --locked --offline --all-targets -- -D warnings
cd tests/m25-whisper/guest-shape && cargo clippy --locked --offline -- -D warnings
cargo fmt -p nagi-init -- --check
tools/whisper/build-host-eval.sh && tools/whisper/fetch-fleurs-ja-eval.py \
  && tests/m25-whisper/run-eval.sh
# Integration owner, x86_64 host with Nagi toolchain:
cargo clippy -p nagi-init --target targets/x86_64-unknown-nagi-user.json \
  --features m25-whisper-inference-acceptance -- -D warnings
./nagi m25-whisper-inference
```

## Evidence

- `.dev/workstreams/hark-m25-whisper/evidence/host-eval-20261009.json`
  (sha256 `1e53ec45438c54e85048e8ff6fec5d556e92e804ab0f73c0aa86d476d6603044`; host only).
- `tests/m25-whisper/eval/fleurs-ja-validation.json` (dataset revision,
  license, citation, per-clip source/PCM SHA-256).

## Open gates

- Nagi-target build of `nagi-init` with `m25-whisper-inference-acceptance`
  on this branch: NOT RUN (no x86_64 Nagi toolchain in this environment).
- Guest QEMU `./nagi m25-whisper-inference` after the refactor: NOT RUN.
- nagi-init Clippy (x86_64 inline asm): left to CI.
- Guest unseen-speech accuracy/time/RAM: NOT MEASURED.
- Registry/schema/CI registration: proposed, not applied.
- Merge: requires separate user confirmation.
