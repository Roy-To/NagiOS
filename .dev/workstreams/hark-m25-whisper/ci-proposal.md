# CI integration — hark-m25-whisper

Status: APPLIED as the workstream-owned dedicated workflow
`.github/workflows/hark-m25-whisper.yml` (user-assigned 2026-10-09). It does
not modify `ci.yml` and is not owner registration (the registry/schema row in
`registration-proposal.md` is still a proposal for the Codex integration owner).

## Gap in the shared CI (main `f74f128`; still true for `ci.yml` at `ef217b3`)

- `ci.yml` never builds or tests `tests/m25-whisper` (standalone crate, own
  `[workspace]`) or `tests/m25-whisper/guest-shape`.
- `ci.yml` job `nagi-target` builds `nagi-init` only with `--features m17-servo`
  (step "Build Nagi user init"); no job compiles
  `--features m25-whisper-inference-acceptance`, builds the Nagi-target
  whisper.cpp archives (`tools/whisper/build-nagi-target.sh`), or runs
  `./nagi m25-whisper-inference`.
- Its `target_classification` filter does match `user/**` and
  `third_party/**`, so a change to `user/nagi-init/src/m25_whisper.rs`
  triggers the m17-servo target build, which does not compile that module
  (it is `cfg(feature = …m25…)`-gated).
- `0.2-host-integration.yml` path-matches `.dev/**`, but its job is gated to
  a fixed `codex/*`/`claude/*` branch list and is `skipped` on this branch.

## Dedicated workflow `.github/workflows/hark-m25-whisper.yml`

Triggers: `push` to `hark/m25-whisper-production` and `pull_request`, both
path-filtered to the owned paths (`user/nagi-init/src/m25_whisper.rs`,
`tools/whisper/**`, `tests/m25-whisper/**`,
`.dev/workstreams/hark-m25-whisper/**`) plus the workflow file; and
`workflow_dispatch`. `permissions: contents: read`; every action pinned by
full commit SHA (checkout v4.2.2, cache v6.1.0, upload-artifact v7.0.1, the
same pins as `ci.yml` / `hark-m25-local-tts.yml`); no secrets.

| Job | What it runs | Counts as |
| --- | --- | --- |
| A host tests | rustfmt (harness, guest-shape, shared sources); clippy -D warnings without the real engine; 24 scripted-engine same-session tests incl. `tests/capacity.rs` (asserted 24 passed, capacity 4 passed); guest-shape clippy | host scripted-engine tests |
| B real host engine | pinned model (models.lock size + SHA-256) and 12 FLEURS ja clips (WAV + PCM SHA-256), pinned whisper.cpp 927cfce + patches 0001-0003 host build, clippy with `--features real-engine`, `run-eval.sh` (5 real-engine lifecycle checks, CER/RTF/RSS uploaded as artifact, no thresholds added), negatives: missing / mismatched model, corrupted / missing clip must FAIL | HOST measurement only |
| C COMPILE-ONLY Nagi-target | nagi-bootstrap fetch, relibc headers, Nagi-target whisper.cpp archives, `cargo build -p nagi-init --features m25-whisper-inference-acceptance --target targets/x86_64-unknown-nagi-user.json` with a synthetic silence PCM + placeholder text (build.rs requires them) | compile/link only; never run; NOT guest acceptance |

Guest fixture acceptance has no job (NOT_RUN; see below).

Recorded runs (all jobs green): head `ae4eeef` — push
[37884041763](https://github.com/RT-NISH/NagiOS/actions/runs/37884041763),
pull_request
[37884047013](https://github.com/RT-NISH/NagiOS/actions/runs/37884047013).
Per-job results and measurements: `evidence/ci-hark-m25-whisper-ae4eeef.json`.
Later heads carry their own push / pull_request / workflow_dispatch runs (PR #39
body lists the final head's runs).

## Guest acceptance (not proposed for hosted CI)

`./nagi m25-whisper-inference <pcm> <expected>` builds the image and boots it
under QEMU with a 3600 s timeout (`commands.rs`). The recorded PASS (run
`1790978330938078000`, short fixture containing `アルバートを開いて`) took about
37 minutes under TCG on the reference host. The fixture PCM is a local,
uncommitted input. Run it on a self-hosted or developer x86_64 host; record the
result separately from host measurements.

## Exact-head local evidence from this stream

See `state.json` (`last_verified`) for the commands, exit codes and commit of
each local run, including the local attempt of job B in this sandbox and its
result or blocker.
