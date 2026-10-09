# CI integration proposal — hark-m25-whisper

Status: PROPOSED for the Codex integration / CI owner. Not applied. This stream
owns no workflow file; `.github/workflows/**` is shared. Nothing here is a CI
result.

## Gap in the current shared CI (main `f74f128`)

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

## Proposed workflow `.github/workflows/hark-m25-whisper.yml`

Three jobs. A and B need no model; C is manual because it downloads the
487.6 MB model and FLEURS audio. The guest QEMU acceptance is deliberately not
in CI (about 37 min under TCG on the reference host, needs a local fixture PCM
that is not committed); see "Guest acceptance" below.

```yaml
# Dedicated CI for the M25 Whisper provider (hark/m25-whisper-production).
# A host-tests    standalone harness tests, fmt, clippy; guest-shape check
# B nagi-target   relibc headers + Nagi-target whisper.cpp + nagi-init build
#                 with m25-whisper-inference-acceptance (no image, no QEMU)
# C real-engine   manual: pinned model + FLEURS ja eval (HOST measurement)
# No secrets, no write tokens; public pinned downloads verified by SHA-256.
name: M25 Whisper (hark)

on:
  push:
    branches:
      - hark/m25-whisper-production
    paths:
      - 'user/nagi-init/src/m25_whisper.rs'
      - 'user/nagi-init/build.rs'
      - 'user/nagi-init/Cargo.toml'
      - 'user/nagi-audio/**'
      - 'user/nagi-model-manager/**'
      - 'tools/whisper/**'
      - 'tools/nagi-target-cc.sh'
      - 'tests/m25-whisper/**'
      - 'third_party/whisper-cpp-patches/**'
      - 'third_party/sources.lock'
      - 'third_party/models.lock'
      - 'third_party/relibc/**'
      - 'targets/x86_64-unknown-nagi-user.json'
      - 'rust-toolchain.toml'
      - '.github/workflows/hark-m25-whisper.yml'
  pull_request:
    paths:
      - 'user/nagi-init/src/m25_whisper.rs'
      - 'user/nagi-init/build.rs'
      - 'user/nagi-init/Cargo.toml'
      - 'user/nagi-audio/**'
      - 'user/nagi-model-manager/**'
      - 'tools/whisper/**'
      - 'tools/nagi-target-cc.sh'
      - 'tests/m25-whisper/**'
      - 'third_party/whisper-cpp-patches/**'
      - 'third_party/sources.lock'
      - 'third_party/models.lock'
      - 'third_party/relibc/**'
      - 'targets/x86_64-unknown-nagi-user.json'
      - 'rust-toolchain.toml'
      - '.github/workflows/hark-m25-whisper.yml'
  workflow_dispatch:
    inputs:
      real_engine:
        description: 'Run the real-engine FLEURS ja host evaluation (downloads 487.6 MB model)'
        type: boolean
        default: false

permissions:
  contents: read

concurrency:
  group: hark-m25-whisper-${{ github.ref }}
  cancel-in-progress: true

env:
  CARGO_TERM_COLOR: never
  TOOLCHAIN: nightly-2025-08-01

jobs:
  host-tests:
    name: A host tests (scripted engine) and guest-shape check
    runs-on: ubuntu-24.04
    timeout-minutes: 30
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
      - name: Install pinned Rust toolchain
        run: rustup toolchain install "$TOOLCHAIN" --profile minimal --component rustfmt --component clippy
      - name: Standalone harness
        working-directory: tests/m25-whisper
        run: |
          cargo +"$TOOLCHAIN" fmt -- --check
          cargo +"$TOOLCHAIN" test --locked
          cargo +"$TOOLCHAIN" clippy --locked --all-targets -- -D warnings
      - name: Guest-shape type check (check only, relibc stub, not a guest result)
        working-directory: tests/m25-whisper/guest-shape
        run: |
          cargo +"$TOOLCHAIN" fmt -- --check
          cargo +"$TOOLCHAIN" clippy --locked -- -D warnings
      - name: Shared-source formatting
        run: rustfmt +"$TOOLCHAIN" --edition 2021 --check user/nagi-init/src/m25_whisper.rs tools/whisper/provider_session.rs tools/whisper/provider_ffi.rs

  nagi-target:
    name: B Nagi-target build of nagi-init with m25-whisper-inference-acceptance
    runs-on: ubuntu-24.04
    timeout-minutes: 90
    env:
      NAGI_TARGET_CLANG: clang-19
      NAGI_CXX_HEADERS: /usr/lib/llvm-19/include/c++/v1
      NAGI_LLVM_AR: llvm-ar-19
      NAGI_LLVM_RANLIB: llvm-ranlib-19
      NAGI_RELIBC_HEADERS: ${{ github.workspace }}/out/m25-relibc/relibc-target/x86_64-unknown-nagi-user/include
      NAGI_WHISPER_SOURCE: ${{ github.workspace }}/out/cache/whisper-cpp-nagi
      NAGI_WHISPER_BUILD: ${{ github.workspace }}/out/m25-whisper-target
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
      - name: Install target build dependencies
        run: |
          sudo apt-get update
          sudo apt-get install --no-install-recommends -y clang clang-19 lld lld-19 llvm llvm-19 libc++-19-dev cmake ninja-build make python3
      - name: Install pinned Rust target toolchain and cbindgen
        run: |
          rustup toolchain install "$TOOLCHAIN" --profile minimal --component rustfmt --component clippy --component rust-src
          cargo install cbindgen --version 0.28.0 --locked
      - name: Bootstrap pinned source dependencies (root workspace [patch] paths)
        run: cargo run --locked --manifest-path tools/nagi-bootstrap/Cargo.toml -p nagi-bootstrap -- fetch
      - name: Generate relibc target headers
        run: |
          RUST_TARGET_PATH="$PWD/targets" RUSTUP_TOOLCHAIN="$TOOLCHAIN" \
          CARGO_TARGET_DIR="$PWD/out/m25-relibc/relibc-target" \
          make -C third_party/relibc TARGET=x86_64-unknown-nagi-user \
            BUILD="$PWD/out/m25-relibc/relibc-target/x86_64-unknown-nagi-user" \
            TARGET_HEADERS="$NAGI_RELIBC_HEADERS" headers
          test -f "$NAGI_RELIBC_HEADERS/pthread.h"
      - name: Pinned and patched whisper.cpp source
        run: |
          rev=$(awk '/^\[sources.whisper_cpp\]/{s=1;next} /^\[/{s=0} s&&/^revision/{gsub(/"/,"",$3);print $3}' third_party/sources.lock)
          repo=$(awk '/^\[sources.whisper_cpp\]/{s=1;next} /^\[/{s=0} s&&/^repository/{gsub(/"/,"",$3);print $3}' third_party/sources.lock)
          git clone --quiet --no-checkout "$repo" "$NAGI_WHISPER_SOURCE"
          git -C "$NAGI_WHISPER_SOURCE" checkout --quiet "$rev"
          for p in third_party/whisper-cpp-patches/*.patch; do git -C "$NAGI_WHISPER_SOURCE" apply --whitespace=nowarn "$PWD/$p"; done
      - name: Nagi-target whisper.cpp archives
        run: NAGI_BUILD_JOBS=4 bash tools/whisper/build-nagi-target.sh
      - name: Build nagi-init (m25-whisper-inference-acceptance)
        env:
          # Build-only placeholder inputs: the acceptance feature stages a PCM
          # fixture and expected text at build time. A generated silent PCM is
          # used here so the build is reproducible; it is never run, and this
          # job is not a fixture-acceptance or recognition result.
          NAGI_M25_WHISPER_PCM_FIXTURE: ${{ github.workspace }}/out/m25-ci-placeholder.pcm
          NAGI_M25_WHISPER_EXPECTED_TEXT_FILE: ${{ github.workspace }}/out/m25-ci-placeholder.txt
        run: |
          head -c 32000 /dev/zero > "$NAGI_M25_WHISPER_PCM_FIXTURE"
          printf 'placeholder' > "$NAGI_M25_WHISPER_EXPECTED_TEXT_FILE"
          cargo +"$TOOLCHAIN" build -p nagi-init --features m25-whisper-inference-acceptance \
            --target targets/x86_64-unknown-nagi-user.json \
            -Zbuild-std=core,alloc,compiler_builtins --release --locked
          ls -l target/x86_64-unknown-nagi-user/release/nagi-init

  real-engine:
    name: C real-engine host evaluation (manual, HOST measurement only)
    if: github.event_name == 'workflow_dispatch' && inputs.real_engine
    runs-on: ubuntu-24.04
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
      - name: Install dependencies
        run: |
          sudo apt-get update
          sudo apt-get install --no-install-recommends -y cmake g++ python3
          rustup toolchain install "$TOOLCHAIN" --profile minimal
      - name: Cache locked model
        uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
        with:
          path: out/cache/whisper-models
          key: whisper-small-1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b
      - name: Pinned sources, model, FLEURS audio
        run: |
          rev=$(awk '/^\[sources.whisper_cpp\]/{s=1;next} /^\[/{s=0} s&&/^revision/{gsub(/"/,"",$3);print $3}' third_party/sources.lock)
          repo=$(awk '/^\[sources.whisper_cpp\]/{s=1;next} /^\[/{s=0} s&&/^repository/{gsub(/"/,"",$3);print $3}' third_party/sources.lock)
          git clone --quiet "$repo" out/cache/whisper.cpp-pinned
          git -C out/cache/whisper.cpp-pinned checkout --quiet "$rev"
          mkdir -p out/cache/whisper-models
          m=out/cache/whisper-models/ggml-small.bin
          test -f "$m" || curl -fL --retry 3 -o "$m" \
            https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-small.bin
          echo "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b  $m" | sha256sum -c -
          python3 tools/whisper/fetch-fleurs-ja-eval.py
      - name: Build and evaluate (1 thread)
        run: |
          tools/whisper/build-host-eval.sh
          tests/m25-whisper/run-eval.sh out/m25-whisper-eval/host-eval.json
      - uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4.6.2
        if: always()
        with:
          name: m25-whisper-host-eval-${{ github.run_attempt }}
          path: out/m25-whisper-eval/host-eval.json
```

Notes for the owner:

- The action SHAs are the ones already pinned in `ci.yml` /
  `hark-m25-local-tts.yml` (checkout v4.2.2, cache v6.1.0); the
  upload-artifact SHA must be checked against the repository's pinning policy
  before use.
- Job B commands mirror `./nagi m25-whisper-inference`'s target stage
  (`tools/nagi-cli/src/commands.rs`, `execute_m25_whisper_inference`) and
  `tools/libcxx/build-nagi-target.sh`'s relibc header step, with the image
  and QEMU stages removed. `fetch-fleurs-ja-eval.py`'s exact Python
  dependencies must be checked before enabling job C.
- The silent placeholder PCM in job B is a build input only. Fixture
  acceptance needs the real local fixture and a guest run.
- Registering this workflow also needs the registry/schema row in
  `registration-proposal.md`.

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
