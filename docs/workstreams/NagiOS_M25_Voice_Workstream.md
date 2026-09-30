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
- The deterministic fixture provider records the input frame and returns
  `Unavailable` from finalization. It does not invent or hardcode a transcript.
  Tests cover permission prompt/denial, indicator-before-capture ordering,
  bounded audio forwarding, provider-unavailable cleanup, malformed PCM, and
  cancellation cleanup.

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

Result: 8 tests passed, including 4 push-to-talk fixture tests. Warnings-denied
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
and provider-unavailable cleanup without creating a transcript. The command
boots the guest with its persistent VFS disk and requires the M25 serial markers
alongside the existing kernel, M2–M7, and M13 acceptance markers.

Verification passed:

- `./nagi m25` — QEMU printed all four M25 fixture checks and
  `Nagi M25 voice orchestration PASS`. Logs are
  `out/logs/m25-voice-bootstrap.log` and `out/logs/m25-voice.log`; the boot
  image, data disk, EFI variables, and serial logs are preserved in
  `out/evidence/m25-voice-fixture-20260930/`.
- Target `nagi-init` build and `cargo clippy --no-deps` with
  `m25-voice-acceptance` — passed. Existing warnings from the separately
  compiled `relibc` and `nagi-posix` dependencies are not treated as new
  M25 diagnostics.
- `nagi-audio`: 8 tests passed; `nagi-cli`: 119 unit and 18 integration tests
  passed. Changed-package rustfmt checks passed.
- `./nagi m19` and `./nagi m22` QEMU regressions passed after the M25 changes.
  Their pre-regression disk snapshots, EFI variables, and logs are preserved
  in `out/evidence/pre-m25-regressions-20260930/`.

This fixture acceptance does not use a real microphone or STT model and does
not demonstrate authenticated user/session permission, speech recognition,
text-to-speech, or command execution. QEMU on this host reports that it cannot
open `virtio-sound.in`; the voice fixture intentionally verifies orchestration
without relying on host audio.

## Remaining acceptance blockers

1. Connect `SpeechPermissionAuthority` to authenticated user/session policy and
   explicit press state. No production adapter or IPC registration exists yet.
2. Connect `MicrophoneActivityIndicator` to the system-owned, localized UI and
   register the system push-to-talk shortcut. The contract test uses an
   in-memory indicator fixture.
3. Add the pinned whisper.cpp small multilingual Japanese STT backend and model
   artifact/license metadata. No concrete provider or Japanese transcription
   is implemented.
4. Add a local TTS provider behind the Speech API and implement playback.
5. Add QEMU acceptance proving real guest microphone capture, spoken Japanese
   command handling (including launching Albert or a basic Nagi command),
   failure behavior, and the visible consumer indicator. The new guest fixture
   acceptance proves orchestration only; it does not claim device or
   speech-model behavior.

M25 remains `PARTIAL` until authenticated permission, real Japanese STT, local
TTS, the system indicator, and real guest voice-command acceptance are
connected and verified.
