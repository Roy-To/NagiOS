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
  1 KiB, checks PCM frame alignment and UTF-8 output, rejects empty transcripts,
  and clears its PCM buffer after delivery or cancellation. Providers receive
  no OS capability and cannot execute transcript text.
- `SpeechOptions` separates `Auto` and Japanese transcription hints from
  system language and locale. `MicrophoneCapture` adapts the target
  `AudioService` and a system-selected stream ID. The device adapter and
  `libnagi` dependency are compiled only for `target_os = "nagi"`; host tests
  exercise the orchestration contract with an injected fixture source.
- `user/nagi-init/src/m25_whisper.rs` now implements a target
  `SpeechToTextProvider` backed by the pinned whisper.cpp CPU library. It reads
  the exact Whisper Small artifact through the guest's read-only Model Store
  capability, accepts bounded mono 16 kHz PCM, and returns bounded UTF-8
  transcript bytes through the existing push-to-talk coordinator. The
  opt-in `./nagi m25-whisper-inference` gate requires real guest inference to
  contain the expected Japanese fixture phrase; it does not execute it.
- The C++ adapter sets whisper.cpp's supported per-utterance audio context to
  the sample-derived duration plus a one-second zero tail, capped at the model
  limit. Compile-time boundary checks cover empty, short, and capped inputs.
  The normal M30 image and default memory limits remain unchanged.
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
  bounded audio forwarding, provider-unavailable cleanup, empty-transcript
  rejection/output clearing, malformed PCM, and cancellation cleanup. TTS tests
  cover bounded UTF-8 input, chunked playback, malformed-frame rejection, output
  caps, failure cancellation, and buffer clearing.

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

Result: 14 tests passed, including 5 push-to-talk and 5 TTS contract tests.
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

A real Nagi-target whisper.cpp transcription now passes the short Japanese
fixture acceptance: QEMU loaded the digest-verified Whisper Small artifact,
ran `whisper_full`, and matched the expected phrase. That 1.48-second fixture
took about 37 minutes under QEMU TCG, so this is a correctness check for one
short sample, not a usable latency or sustained-resource result. It uses a
local PCM fixture and does not verify microphone capture.

1. Connect `SpeechPermissionAuthority` to authenticated user/session policy,
   trusted foreground consumer selection, and explicit push-to-talk state.
   No production adapter or IPC registration exists yet.
2. Connect `MicrophoneActivityIndicator` to the system-owned, localized UI and
   register the system push-to-talk shortcut. The current contract tests use
   an in-memory indicator fixture.
3. Add a concrete local TTS engine behind the provider contract, choose it using
   the documented quality/CPU/RAM/portability/license criteria, and verify
   real playback. The contract and target AudioService sink exist, but there is
   no synthesis engine yet.
4. Add QEMU acceptance with actual guest microphone capture and a spoken
   Japanese request routed through deterministic validation, permission,
   executor, transaction, and Activity Ledger boundaries before it launches
   Albert or performs a basic Nagi command. Transcript text must never execute
   directly. The host currently provides no `virtio-sound.in` backend.

M25 remains `PARTIAL` pending authenticated permission, the visible system
indicator and shortcut, real capture and safe command integration, local TTS,
and practical inference latency/resource evidence.
## Nagi-owned target compatibility patch — 2026-10-01

The pinned upstream checkout remains unchanged and clean. The reproducible
Nagi-owned patch is
`third_party/whisper-cpp-patches/0001-nagi-whisper-target-noexceptions.patch`;
`./nagi fetch` applies it to the separate ignored
`out/cache/whisper-cpp-nagi/` checkout and records a marker binding the pinned
revision, patch fingerprint, and generated tree state. Modified upstream files
are therefore preserved as a patch instead of edits to the fetched source.

The patch enables the CPU backend for `__NAGI__`, rejects dynamic backend
loading on Nagi, adds bounded no-exception error paths to whisper model loading,
and releases any partially initialized model contexts on load failure. It
replaces the GGUF exception parser/writer path with bounded metadata checks and
explicit I/O error reporting. It adds an opt-in host regression test for valid
parsing, parser limits, and writer failure handling. The host test uses a
reduced 4 KiB metadata budget to exercise limits; the Nagi target default
remains 64 MiB.

Verification on 2026-10-01:

- `./nagi fetch`, `./nagi fmt`, `./nagi lint`, `./nagi test`, and `./nagi build`
  passed on the continuation branch. The CLI regressions check numeric patch
  order, reject an empty patch set and tampered generated source, and preserve
  the raw upstream checkout while applying patches.
- `cmake --build out/nagi-whisper-target-nagi --target whisper --parallel 4`
  passed with the freestanding Nagi x86-64 target wrapper, LLVM 19 libc++
  headers, and generated relibc headers. The build output and CMake compile
  configuration are preserved in
  `out/evidence/m25-whisper-target-compile-20261001/`.
- The host `test-gguf-nagi` target and its CTest case passed. Its verified
  compile commands and CTest/build logs are in the same evidence directory.
- `./nagi m25` passed the guest orchestration markers. QEMU reported that this
  host has no `virtio-sound.in` driver. Before/after boot image, persistent
  disk, OVMF variables, and serial logs with SHA-256 manifests are preserved
  under `out/evidence/m25-whisper-noexceptions-pre-final-rerun-20261001/` and
  `out/evidence/m25-whisper-noexceptions-final-pass-20261001/`.

The pinned model's immutable metadata still records 487,601,967 bytes and the
SHA-256 above, but the model bytes have not been downloaded or independently
hashed in this worktree. Building the library does not demonstrate model
loading or inference. Authenticated voice permission, a connected Japanese
STT provider, local TTS, the system microphone indicator, and spoken-command
acceptance remain incomplete; M25 stays `PARTIAL`.

## Empty transcript rejection and fresh-run artifacts — 2026-10-01

`PushToTalkService::finish_into` now rejects a provider's successful zero-byte
result as `SpeechError::EmptyTranscript`, cancels provider state, clears the
caller output buffer, and finishes the capture/indicator lifecycle. A focused
host regression covers this fail-closed behavior. The target orchestration
fixture separately returns an empty result to prove the same cleanup path; its
normal unavailable-provider case still produces no synthetic transcript and
does not claim STT inference.

Verification passed:

- `cargo test --locked --offline -p nagi-audio --all-targets` — 14 tests passed;
  `cargo test --locked --offline -p nagi-cli` — 152 unit and 21 integration
  tests passed.
- Nagi-target checks and warnings-denied, changed-package Clippy passed for
  `nagi-audio` and `nagi-init` with `m25-voice-acceptance`; `./nagi lint`
  passed.
- A fresh `./nagi m25` QEMU run passed the permission, indicator, bounded PCM,
  unavailable-provider cleanup, empty-transcript rejection, TTS contract, and
  overall orchestration markers. QEMU reported no host `virtio-sound.in`
  driver; the fixture does not depend on host audio, and no real microphone,
  STT model, or TTS engine was used.
- The run uses unique image, User Data, OVMF variables, and serial log names and
  refuses to overwrite existing run artifacts. The five-file SHA-256 manifest
  at `out/evidence/m25-empty-transcript-1790854850975922000/manifest.sha256`
  verifies the image, disk, variables, bootstrap log, and voice log.

Authenticated permission, real Japanese STT and inference, concrete local TTS,
the system microphone indicator, and spoken-command QEMU acceptance remain
incomplete. M25 remains `PARTIAL`.

## Fixture transcript delivery — 2026-10-02

The fixture provider now has an opt-in fixed Japanese transcript result while
its default remains `Unavailable`; the empty-result case remains independently
covered. The host regression confirms successful bounded transcript delivery,
cleared output tail and internal PCM buffer, hidden activity indicator, and
completed capture lifecycle. The target fixture passes the fixed phrase
`アルバートを開いて` through `PushToTalkService::finish_into` and checks the
output bytes, cleared tail, indicator state, captured/forwarded PCM counts,
and cleanup state. The QEMU gate now requires the additional
`Nagi M25 fixture transcript delivery PASS` marker.

This fixture proves only provider-to-caller orchestration. It does not run
whisper.cpp, infer Japanese, execute Albert or another command, use a real
microphone, or synthesize speech. `./nagi m25` passed on 2026-10-02; its
run-stamped image, User Data disk, OVMF variables, bootstrap log, and voice log
are preserved with a verified manifest at
`out/evidence/m25-fixture-transcript-1790868293848130000/manifest.sha256`.
The full host results are 15 `nagi-audio` tests and 155 `nagi-cli` unit plus 21
integration tests; `./nagi fmt`, `./nagi test`, `./nagi lint`, and
`./nagi build` passed. QEMU reports that this host has no `virtio-sound.in`
driver, which this orchestration fixture does not need.

Authenticated permission and system-indicator wiring, a real Japanese STT
provider/inference, concrete local TTS, and spoken-command acceptance remain
incomplete. M25 remains `PARTIAL`.

## Pinned model download verification — 2026-10-02

Downloaded the exact Whisper small multilingual artifact pinned by
`third_party/models.lock` to the ignored local cache at
`out/cache/whisper-models/ggml-small-5359861c739e955e79d9a303bcbc70fb988958b1.bin`.
Its size is 487,601,967 bytes and its SHA-256 is
`1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b`; both
match the immutable lock. The evidence manifest at
`out/evidence/m25-whisper-model-download-20261002/SHA256SUMS` covers the
README and cached model bytes.

This makes the pinned artifact available for future integration work only.
It has not been copied into guest storage, registered with the guest Model
Store, loaded on Nagi, or used for inference. M25 remains `PARTIAL`.

## Disposable guest artifact digest acceptance — 2026-10-02

Added `./nagi m25-whisper <artifact.bin>` as an opt-in fixture acceptance
command. It validates the requested file against the exact immutable model
lock, streams it into a separate disposable GPT Model Store image, and boots
the Nagi guest with the read-only Model Store capability. The guest checks the
locked 487,601,967-byte length, GGML magic, and SHA-256
`1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b` before
emitting `Nagi M25 Whisper artifact digest PASS`. The separate acceptance
image passed `qemu-img check`; evidence and the four-entry SHA-256 manifest
are under `out/evidence/m25-whisper-artifact-1790895162586172000/`, and the
image is `out/artifacts/nagi-0.1-m25-whisper-1790895162586172000.qcow2`.

The shared Model Store runner was regression-tested with the full pinned
2,244,011,552-byte Granite artifact. QEMU emitted both the Granite digest and
Model Store capability PASS markers, and `qemu-img check` passed for its
separate image. Evidence is under
`out/evidence/m20-granite-artifact-1790895384092435000/`.

The changed CLI passed 165 unit and 21 integration tests, warnings-denied
Clippy, and formatting. `./nagi test`, `./nagi lint`, and `./nagi build`
passed, and the `m25-whisper-artifact-acceptance` feature compiled for the
Nagi target. QEMU regressions `./nagi m25`, `./nagi m19`, and `./nagi m22`
also passed. The QEMU host still has no `virtio-sound.in` audio driver; the
artifact check does not need audio.

This acceptance verifies artifact bytes in a disposable guest Model Store
only. It does not load the model through whisper.cpp, run STT inference, use a
microphone, or claim speech recognition. The regular M30 Model Store remains
unchanged, and M25 remains `PARTIAL` pending authenticated permission/UI, a
real Japanese STT provider and inference, concrete local TTS, and
spoken-command acceptance.

## Standard model loader read-count hardening — 2026-10-02

Added the ordered Nagi patch
`third_party/whisper-cpp-patches/0002-nagi-whisper-model-read-counts.patch`.
The standard Whisper model loader now accumulates positive short reads, checks
every scalar and payload read, rejects stalled or over-reported reads, and
reports the actual byte count from its file adapter. It reads each three-field
tensor header as one 12-byte unit. EOF is accepted only when the loader reads
zero bytes at the beginning of that header and confirms EOF; partial headers
are rejected. The separate VAD loader path is unchanged.

The host `test-whisper-buffer-loader` regression passed. It loads a synthetic
minimal no-tensor model with chunked reads, accepts clean EOF at the tensor
section, rejects a truncated hyperparameter, and rejects every partial tensor
header length from 1 through 11 bytes. The Nagi-target CMake `whisper` library
build passed. `./nagi fmt`, `./nagi test`, and `./nagi lint` also passed.

`./nagi fetch` applied patch 0002 to a fresh generated checkout and wrote a
matching checkout marker, then stopped with exit 4 at the existing modified
generated Servo checkout. The pinned raw `third_party/whisper.cpp` remains
clean. The pre-change generated checkout is preserved with a verified
SHA-256 manifest at
`out/evidence/m25-whisper-read-count-20261002/pre-change-SHA256SUMS`.

The synthetic model test does not load the 487,601,967-byte artifact or run
inference. A connected Japanese STT provider, authenticated permission/UI,
local TTS, system microphone indicator, and spoken-command acceptance remain
incomplete; M25 remains `PARTIAL`.

## Clean-checkout patch correction — 2026-10-02

GitHub CI runs for commits `6d15f43` and `1107fba` failed at clean-source
application because patch 0002's zero-context hunks did not match the pinned
whisper.cpp checkout. Patch 0002 now uses contextual unified-diff hunks, and
`.gitattributes` scopes the blank context-line whitespace exception to this
patch. Forward and reverse `git apply --check` both pass against the preserved
pre-change and prior generated checkouts. A fresh clone of pinned revision
`927cfce34f31707e17f2bff35c349632fb9e2c3a` also accepted both numbered
patches in order. `./nagi fetch` regenerated the Whisper checkout with patch
fingerprint `fnv1a64:0752e44c02fe91e8` and
checkout fingerprint `fnv1a64:5a3c628c90d404d3`; it then stopped safely at the
existing modified generated Servo checkout. The old generated Whisper
checkout was moved intact and hash-manifested at
`out/evidence/m25-whisper-old-generated-checkout-20261002/`.

The focused `test-whisper-buffer-loader` CTest passed (1/1), and the Nagi-target
CMake library build passed. A local CLI patch-contract `cargo test` could not
link because this host's installed Command Line Tools do not provide an
x86_64-compatible `libxcrun`; the clean-host CI result remains pending. The
loader test uses synthetic data only, and M25 remains `PARTIAL`.

## Streaming STT format adapter — 2026-10-03

Added `Stereo48KhzToMono16Khz`, a fixed-memory, 127-tap Q15 Blackman FIR
decimator for interleaved stereo S16LE. It averages the channels, emits mono
S16LE at 16 kHz, retains its FIR history and 3:1 phase across bounded capture
chunks, flushes the FIR tail at end of utterance, and resets/erases state on
finish or cancel. `ResamplingSpeechToTextProvider` now adapts the existing
48 kHz capture contract to the 16 kHz mono format expected at a Whisper-class
provider boundary. This provides the sample-rate conversion seam; there is
still no connected Whisper provider.

The 20 `nagi-audio` tests pass, including split-versus-whole chunk equivalence,
state preservation after rejected buffers, silence, stereo averaging, reset,
tail flush, and steady-state rejection of a 12 kHz alias. Warnings-denied
Clippy and `./nagi fmt` pass. QEMU run `1790954944032231000` passed the target
M25 voice fixture, including converted PCM delivery before provider finish,
permission/indicator ordering, cleanup, and fixed transcript handoff. The
guest log, bootstrap log, images, OVMF variables, prior failing log, and
`SHA256SUMS` are preserved under
`out/evidence/m25-resampler-20261003/`. The first QEMU attempt exposed an old
fixture assertion that expected the original capture bytes; the fixture was
updated to use 128 stereo frames and assert the decimated output count before
the passing rerun.

This fixture does not access a physical microphone, run Whisper inference,
synthesize speech, or execute the fixture transcript. QEMU still reports no
host `virtio-sound.in` driver. Authenticated permission/UI, a real Japanese STT
provider and inference, concrete local TTS, and spoken-command acceptance
remain incomplete; M25 stays `PARTIAL`.

## Guest Whisper provider and inference attempt — 2026-10-03

The target provider is implemented in `user/nagi-init/src/m25_whisper.rs` and
is wired to the existing `SpeechToTextProvider` interface. The provider opens
the pinned `openai.whisper-small-multilingual` artifact by stable Model Store
ID through a read-only guest capability; the whisper.cpp loader receives
bounded read/EOF/close callbacks and has no host or guest path interface. The
provider supplies the requested Japanese language and converted 16 kHz mono
fixture to the real `whisper_full` API. The current Nagi patch keeps this CPU
path single-threaded until a target worker-pool capability is available. The
provider does not execute transcript content.

The opt-in `./nagi m25-whisper-inference <artifact.bin> <pcm> <expected>`
command validates the model lock and bounded raw PCM format, builds the pinned
Nagi-target Whisper library, installs the artifact only on a disposable
reference image, and boots with M25-only memory bounds. The guest verifies the
artifact digest through Model Store before whisper.cpp loads it. On 2026-10-03,
QEMU logged the exact locked model metadata, completed weight loading and
state/compute-buffer allocation, and entered the CPU inference call. The
optimized run completed in about 37 minutes under QEMU TCG and emitted
`Nagi M25 Whisper Japanese fixture inference PASS`; the gate checks that the
real `whisper_full` transcript contains `アルバートを開いて`. The preceding
markers verify the guest artifact digest, Model Store capability, and
persistent User Data read. `qemu-img check` reported no image errors, and all
eight entries in the evidence `SHA256SUMS` verify from the evidence directory.
The complete evidence is under
`out/evidence/m25-whisper-inference-1790978330938078000/`. An earlier run
without the shorter per-utterance audio context ran for more than two hours and
was stopped. The optimized context keeps all fixture samples and adds a
one-second tail; only this short fixture is verified.

`./nagi fmt`, `./nagi test`, `./nagi lint`, and `./nagi build` pass. The 16
`nagi-posix` tests pass with the M25 heap feature, the focused M25 kernel
page-table-bound test passes, the three focused `nagi-cli` M25 Whisper tests
pass, and the `nagi-init` Nagi-target check passes with
`m25-whisper-inference-acceptance`. The target library and guest image build
and boot for the QEMU acceptance. The pinned artifact and fixture are local
ignored test inputs; the regular M30 image is unchanged.

Authenticated microphone permission and activity UI, real audio capture,
concrete local TTS, real-device inference, and safe voice-command integration
remain incomplete. M25 stays `PARTIAL`.

## Current-source M25 orchestration regression — 2026-10-03

On source commit `4fae6875d64752db8fbe0508a932c28da246e8af`, `./nagi m25`
passed QEMU run `1790986320984425000`: fail-closed permission, indicator order,
bounded PCM forwarding, provider cleanup, empty-transcript rejection, Japanese
fixture transcript handoff, and the TTS provider/playback contract. First-boot
User Data setup and persistent read after restart also passed. This remains a
fixture-only orchestration result; no microphone, STT model, TTS engine, or
transcript action was used. The host QEMU reports no `virtio-sound.in` driver.
The image, data disk, OVMF variables, bootstrap/voice logs, README, and verified
six-entry `SHA256SUMS` are under
`out/evidence/m25-voice-current-source-1790986320984425000/`.

The 2026-10-03 CI checkpoint for commit `4fae687` also found two host-check
issues: current Clippy rejects the PCM fixture's modulo-based even-length check,
and Windows host tests lacked an import for `PermissionPromptLabels`. The build
script now uses `usize::is_multiple_of`, and the Albert chrome test imports its
label type. The focused `nagi-albert` suite passes 60/60 and `./nagi lint`
passes locally; the full Linux/Windows CI rerun is pending on the next pushed
checkpoint.
