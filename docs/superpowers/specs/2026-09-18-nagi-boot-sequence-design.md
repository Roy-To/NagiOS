# Nagi Boot Sequence v2 Design

## Scope

Port the supplied `nagi_boot_sequence_v2.html` behavior into the native Nagi
user-space display path. The implementation is a parallel visual slice for
the existing M10 desktop path. It does not change the current M13 work,
advance the M13 status, or add a high-level kernel boot-screen syscall.

The supplied formal SVG is preserved as the source asset at
`assets/nagi/nagi_logo_formal.svg`. The current native M10 compositor exposes
a fixed 320x200 software surface and has no SVG renderer, so the guest boot
renderer will reproduce the same mark and wordmark with bounded native
software primitives. The SVG remains the authoritative reusable asset for
future higher-resolution UI paths.

## Goals and non-goals

Goals:

- Show a dark, restrained NAGI boot surface in the M10 guest path.
- Render linked circular rings, a progress arc, a moving marker, the NAGI
  logo, percentage, and a compact phase label.
- Stage motion as quiet from 0-30%, accelerated from 30-80%, and convergent
  from 80-99%.
- Never move visible progress backward or show 100% before the lock-state
  surface is ready.
- Collapse the ring object at completion, show a minimal lock-state frame,
  then hand control to the existing desktop compositor.
- Keep all boot progression in user space and expose a small typed bridge for
  real initialization events.
- Provide deterministic state tests and real-QEMU acceptance evidence.

Non-goals:

- No kernel syscall for drawing, animation, or boot policy.
- No host filesystem, browser, or host-rendered screenshot in the guest path.
- No replacement for M11 authentication or the eventual interactive lock
  screen. The M10 lock-state frame is a visual transition only.
- No attempt to claim unavailable AI, network-account, or hardware stages as
  complete. Labels describe only initialization that actually ran.
- No change to `m13-posix` or `m13-std` execution paths.

## Architecture

The feature is split into three layers:

1. `user/libnagi/src/boot.rs` contains the target-independent boot state and
   progress bridge. It owns monotonic progress, weighted stage completion,
   the 99% lock-state cap, completion gating, and failure state. It is small,
   integer-only, and unit-testable on the host.
2. `user/nagi-init/src/boot.rs` contains the M10 native renderer and animation
   loop. It maps the state model onto the existing `DisplayInfo` surface,
   uses integer circle/arc primitives, and calls the existing
   `display_present` capability. It does not call the host or add a kernel
   interface.
3. `user/nagi-init/src/main.rs` creates the bridge at the first usable display
   point and reports real stage boundaries around the existing M5/M6/M7
   bootstrap work. `desktop::run` receives the prepared display surface only
   after the boot transition completes.

The formal SVG is copied into the repository as an asset and checked by a
small source-contract test. The native renderer uses the same gradient colors
and the same mark proportions, but avoids embedding an unimplemented SVG
runtime.

## State and data flow

The bridge supports two producers:

- `Simulation`: deterministic preview frames for development and visual
  smoke tests when no real stage events are available.
- `External`: actual init code calls `set_progress` or `complete` through the
  typed bridge. External updates cancel simulation and remain monotonic.

The real M10 path uses these bounded stages:

| Stage | Target progress | Evidence |
| --- | ---: | --- |
| platform / user process | 15 | display-capability setup and user bootstrap reached |
| core services | 30 | M6 supervisor/registry acceptance completed |
| storage mount | 50 | M7 mount or format/read path completed |
| graphics ready | 70 | surface mapped and first present succeeds |
| session ready | 90 | lock-state frame prepared |
| desktop ready | 100 | lock-state transition completed and desktop starts |

The bridge caps all normal progress at 99 until `mark_lock_ready` is called.
`complete` is ignored unless that flag is set. A failure records a stable
failure phase and renders a recovery/retry frame where display access still
exists; the caller then follows the existing fail-closed exit path.

## Rendering and motion

The renderer uses the fixed M10 surface dimensions and a dark navy background.
It draws three thin circular rings, a tick ring, a progress arc, a centered
NAGI mark/wordmark, the percentage, and a small phase label. Ring angular
velocity is selected from the progress band:

- 0-30: slow, low-opacity rotation;
- 30-80: increased rotation and opacity;
- 80-99: aligned faster rings with reduced radius and stronger convergence;
- completion: a short deterministic scale-down/collapse frame sequence.

No loud flash is used. The renderer exposes a reduced-motion mode in the
state API; the native preview defaults to full motion and uses the same final
state semantics when reduced motion is selected.

Because the current display ABI has no timer/sleep syscall, the animation
uses a bounded frame pump with `core::hint::spin_loop` between presents. The
frame count and stage values are deterministic, bounded, and kept short so
boot cannot stall indefinitely. This is an interim M10 presentation detail;
future session timing can replace the delay mechanism without changing the
state bridge.

## Error handling and boundaries

- Invalid display dimensions, unmapped surface, or failed present produces a
  boot failure marker and exits through the existing user-process failure
  path.
- Progress values are clamped to 0-100, cannot decrease, and cannot reach
  100 before lock-state readiness.
- The visual lock-state frame does not authenticate users. M11 remains the
  authority for Owner/Standard/Guest sessions and permission checks.
- M13 paths return before the M10 renderer is linked, preserving the active
  Rust std/POSIX work and its acceptance contracts.

## Verification

Focused verification will include:

- `cargo test -p libnagi --locked` for monotonic progress, 99% capping,
  completion gating, failure state, and reduced-motion state behavior.
- `cargo fmt --all -- --check` and focused clippy for changed crates.
- Existing M10 build and real-QEMU acceptance in both PowerShell and Git Bash
  environments.
- Ordered serial markers for platform, core services, storage, lock-state
  ready, collapse complete, and desktop ready.
- Boot-frame and lock-frame checksum markers to prove the guest surface was
  rendered before desktop handoff, without claiming a host screenshot is
  guest evidence.

The implementation status file will retain M13 as the current milestone and
will record this as a parallel boot-visual slice only after the focused and
QEMU checks pass.
