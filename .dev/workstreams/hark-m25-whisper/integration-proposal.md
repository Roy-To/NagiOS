# Integration proposal — hark-m25-whisper

Status: PROPOSED. Nothing below is applied outside this stream's owned paths.

## Base and branch

- Base: `f74f128abe40e9e99d7cb05f3f8dc7b90b1448c9` (main after PR #36). Work
  started at `edad2e7`; the intervening main commits touch no owned path and
  no dependency of this stream (only `.dev/workstreams.json` changed among
  shared files).
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

2. CI (shared workflows; owner: CI/integration). Proposed host job, no model
   download, Linux and macOS/Windows host runners:

   ```yaml
   m25-whisper-host:
     runs-on: ubuntu-latest
     steps:
       - uses: actions/checkout@v4
       - run: rustup show
       - working-directory: tests/m25-whisper
         run: |
           cargo fmt -- --check
           cargo test --locked
           cargo clippy --locked --all-targets -- -D warnings
       - working-directory: tests/m25-whisper/guest-shape
         run: |
           cargo fmt -- --check
           cargo clippy --locked -- -D warnings
   ```

   The real-engine evaluation (`tests/m25-whisper/run-eval.sh`) needs the
   487.6 MB model and ~4.5 MB of FLEURS audio; propose it as a manual
   (`workflow_dispatch`) job only, with the model cached by its
   `models.lock` hash.

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
