# Nagi OS M25 — Voice

**Status: PARTIAL**

## Implemented contract

- `user/nagi-audio/src/speech.rs` adds a `no_std` push-to-talk coordinator.
  `SpeechPermissionAuthority` must allow the request; `Ask` and `Deny` fail
  before capture, provider activation, or microphone-indicator activation.
  The system input handler is responsible for invoking `begin` only for a
  direct user action such as Super+V. Its trusted permission adapter must
  verify the authenticated session, trusted foreground consumer, and explicit
  consent.
- The system-owned microphone indicator is shown with the selected consumer
  before provider activation and capture. It is hidden on finish, cancel, or
  any error after activation.
- PCM comes from a service-owned `PcmCaptureSource`, is streamed directly to
  `SpeechToTextProvider`, and is never returned to the caller. The coordinator
  holds one 4 KiB frame, caps an utterance at 1 MiB and transcript output at
  1 KiB, checks PCM frame alignment and UTF-8 output, and clears its PCM buffer
  after delivery or cancellation. Providers receive no OS capability and
  cannot execute transcript text.
- `SpeechOptions` separates `Auto` and Japanese transcription hints from
  system language and locale. `MicrophoneCapture` adapts the target
  `AudioService` and a system-selected stream ID. The device adapter and
  `libnagi` dependency are compiled only for `target_os = "nagi"`; host tests
  exercise the orchestration contract with an injected fixture source.
- `TextToSpeechProvider` is a replaceable, authority-free provider contract.
  It accepts at most 1 KiB of validated UTF-8 plus an explicit utterance
  language, and returns signed 16-bit little-endian PCM in 4 KiB chunks. The
  `no_std` synthesis service validates frame alignment, caps total output at
  1 MiB, streams through a service-owned playback sink, and cancels/clears its
  chunk buffer on provider or playback errors. `AudioServicePlaybackSink`
  keeps the device capability inside `AudioService`; utterance language stays
  independent of system locale and Albert conversation language.
- The deterministic fixture provider records the input frame and returns
  `Unavailable` from finalization. It does not invent or hardcode a transcript.
  Tests cover permission prompt/denial, indicator-before-capture ordering,
  bounded audio forwarding, provider-unavailable cleanup, malformed PCM, and
  cancellation cleanup. TTS tests cover bounded UTF-8 input, chunked playback,
  malformed-frame rejection, output caps, failure cancellation, and buffer
  clearing.

## Verification

Pinned arm64 host toolchain:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m25-host-arm64 \
/Users/tozawa/.cargo/bin/cargo test --locked --offline \
  -p nagi-audio --all-targets
```

Result: 13 tests passed, including 4 push-to-talk and 5 TTS contract tests.
Warnings-denied
Clippy passed with `CARGO_TARGET_DIR=/tmp/nagi-m25-clippy-arm64`:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m25-clippy-arm64 \
/Users/tozawa/.cargo/bin/cargo clippy --locked --offline \
  -p nagi-audio --all-targets -- -D warnings
```

The Nagi user target compiled with `core` and `alloc` using
`CARGO_TARGET_DIR=/tmp/nagi-m25-target`:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m25-target \
/Users/tozawa/.cargo/bin/cargo -Z build-std=core,alloc check --locked --offline \
  -p nagi-audio --target targets/x86_64-unknown-nagi-user.json
```

Package formatting check passed:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
/Users/tozawa/.cargo/bin/cargo fmt --manifest-path user/nagi-audio/Cargo.toml -- --check
```

The host-library verification commands used separate target directories.

## Guest orchestration fixture acceptance — 2026-09-30

Added the opt-in `m25-voice-acceptance` feature to `nagi-init` and exposed it
through `./nagi m25`. The guest checks permission denial before capture,
indicator visibility before provider activation, bounded fixture PCM delivery,
and provider-unavailable cleanup without creating a transcript. It also sends
fixture Japanese text through a fixture TTS provider and bounded PCM playback
sink. The command boots the guest with its persistent VFS disk and requires the
M25 serial markers alongside the existing kernel, M2–M7, and M13 acceptance
markers.

Verification passed:

- `./nagi m25` — QEMU printed all five M25 fixture checks, including
  `Nagi M25 TTS provider contract PASS`, and
  `Nagi M25 voice orchestration PASS`. Logs are
  `out/logs/m25-voice-bootstrap.log` and `out/logs/m25-voice.log`; the boot
  image, data disk, EFI variables, and serial logs are preserved in
  `out/evidence/m25-voice-fixture-20260930/`. The immediately prior M25 image,
  16 MiB User Data image, OVMF variables, and logs were also hash-preserved in
  `out/evidence/m25-tts-contract-20260930/pre-run/` before the rerun; the
  legacy disk migration completed and the fixture passed on the new GPT disk.
- Target `nagi-init` build and `cargo clippy --no-deps` with
  `m25-voice-acceptance` — passed. Existing warnings from the separately
  compiled `relibc` and `nagi-posix` dependencies are not treated as new
  M25 diagnostics.
- `nagi-audio`: 13 tests passed; `nagi-cli`: 133 unit and 18 integration tests
  passed. Changed-package rustfmt checks passed.
- Nagi-target Clippy with warnings denied passed for `nagi-audio` and
  `nagi-init` (using `--no-deps` to keep unrelated third-party warnings out of
  the changed-package check). Target compilation and the `./nagi m25` QEMU
  acceptance passed after the empty-output guard was added.
- A full-dependency target Clippy attempt with `-D warnings` still fails on
  existing `libnagi` diagnostics (`needless_return` and missing `# Safety`
  sections); `relibc` and `nagi-posix` also emit existing warnings. These are
  outside the M25 change and were not suppressed or edited as part of this
  contract work.
- `./nagi m19` and `./nagi m22` QEMU regressions passed after the M25 changes.
  Their pre-regression disk snapshots, EFI variables, and logs are preserved
  in `out/evidence/pre-m25-regressions-20260930/`. A second Completion Sweep
  rerun after the TTS contract also passed both acceptances; its prior images,
  User Data disks, variables, and logs are preserved and hash-verified in
  `out/evidence/m25-tts-contract-20260930/pre-m19-m22/`.

The TTS fixture acceptance verifies only provider/playback orchestration; it
does not synthesize speech. The acceptance does not use a real microphone or
STT model and does not demonstrate authenticated user/session permission,
speech recognition, real text-to-speech, or command execution. QEMU on this
host reports that it cannot open `virtio-sound.in`; the voice fixture
intentionally verifies orchestration without relying on host audio.

## Remaining acceptance blockers

1. Connect `SpeechPermissionAuthority` to authenticated user/session policy and
   explicit press state. No production adapter or IPC registration exists yet.
2. Connect `MicrophoneActivityIndicator` to the system-owned, localized UI and
   register the system push-to-talk shortcut. The contract test uses an
   in-memory indicator fixture.
3. Implement the target whisper.cpp provider and Japanese transcription. The
   source is now pinned in `third_party/sources.lock` at
   `927cfce34f31707e17f2bff35c349632fb9e2c3a`, and `./nagi fetch` materializes
   and validates that clean upstream checkout. The multilingual Whisper small
   artifact is pinned in `third_party/models.lock` to immutable Hugging Face
   revision `5359861c739e955e79d9a303bcbc70fb988958b1`, size 487,601,967 bytes,
   SHA-256
   `1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b`, and
   the upstream MIT metadata. This metadata does not download the model, place
   it into Model Store, or implement inference.
4. Add a concrete local TTS engine behind the new provider contract, select it
   using the documented quality/CPU/RAM/portability/license criteria, and
   verify real playback. The contract and target AudioService sink exist, but
   there is no synthesis engine yet.
5. Add QEMU acceptance proving real guest microphone capture, spoken Japanese
   command handling (including launching Albert or a basic Nagi command),
   failure behavior, and the visible consumer indicator. The new guest fixture
   acceptance proves orchestration only; it does not claim device or
   speech-model behavior.

M25 remains `PARTIAL` until authenticated permission, real Japanese STT, local
TTS, the system indicator, and real guest voice-command acceptance are
connected and verified.

## Reproducible STT source and artifact pins — 2026-10-01

`./nagi fetch` fetches the exact whisper.cpp commit into the ignored
`third_party/whisper.cpp/` generated-source path. It validates the commit,
upstream MIT `LICENSE`, `CMakeLists.txt`, and clean checkout state; it preserves
any existing mismatched or dirty checkout. `third_party/models.lock` records
the small multilingual model file name, immutable source revision, byte size,
SHA-256, license declaration, and opaque Model Store artifact ID. Fetch
validates the metadata but deliberately does not download model bytes. This
step establishes reproducible provenance and placement metadata only; no STT
provider, target inference, or Japanese speech acceptance is claimed.

### Verification and current target blocker

- `./nagi fetch` passed and fetched whisper.cpp commit
  `927cfce34f31707e17f2bff35c349632fb9e2c3a`; the ignored source checkout is
  clean and its `LICENSE` and `CMakeLists.txt` validate.
- `./nagi test` passed with 140 CLI unit tests and 21 integration tests;
  `./nagi fmt`, `./nagi lint`, and `./nagi build` passed.
- `./nagi m25` passed the guest orchestration markers. QEMU reported no host
  `virtio-sound.in` driver, so the run did not exercise microphone or speaker
  hardware. The boot image, persistent disk, OVMF variables, and logs were
  SHA-256 preserved before and after the run in
  `out/evidence/m25-whisper-pin-pre-rerun-20261001/` and
  `out/evidence/m25-whisper-pin-pass-20261001/`.
- A CPU-only Nagi cross-CMake configure for whisper.cpp 1.9.4 succeeded using
  `tools/nagi-target-cc.sh`, the LLVM 19 libc++ headers, and generated relibc
  headers. Building target `whisper` then failed in upstream
  `ggml/src/gguf.cpp`: 18 uses of `try`, `catch`, and `throw` are incompatible
  with Nagi's `-fno-exceptions` ABI. The first errors are at lines 429, 551,
  637, 1590, 1598, and 1668. The existing llama.cpp GGUF patch does not apply
  to this newer ggml source. No edits were made in the fetched checkout; a
  Nagi-owned patch and host regression suite are required before this library
  can build for Nagi.

The model digest and size are pinned from immutable upstream metadata; the
488 MB model bytes have not been downloaded or independently hashed in this
worktree. A backend, guest inference, and spoken Japanese command acceptance
remain unimplemented.
