# ADR 0025: Select Nagi Softpipe During EGL Initialization

- Status: Accepted for M17 implementation
- Date: 2026-09-25
- Milestone: M17 — Servo Bootstrap

## Context

CI run #183 (`36099071216`) passed the M17 two-boot persistence gate and reached
Servo software GL context construction, then timed out after 120 seconds. The
last existing serial marker was `Nagi M17 trace: GL context creation started`.
This evidence locates the stall inside the Servo/Surfman/Mesa context path but
does not identify the specific call.

Follow-up CI run #184 (`36106455335`) passed the same persistence gate and
repeated the timeout at the same application-level marker. None of the new
EGL or Servo/Surfman stage messages appeared in the guest serial log. This
does not prove whether the call path was missed or the stderr logging route
failed to expose the messages.

CI run #185 (`36113604201`) did not reach M17 acceptance. The Ubuntu host,
Windows, and target jobs all stopped during `nagi-bootstrap fetch` because the
direct `libc` dependency added to `servo-paint-api` was not yet present in
Servo's lockfile. The `--locked` check correctly rejected that incomplete
source patch set. Patch `0009` now records the corresponding lockfile entry.

CI run #186 (`36114799741`) passed pinned Servo bootstrap after patch `0009`,
then failed at the root workspace lock boundary: Ubuntu Clippy and the target
dependency feature check both found that the root `Cargo.lock` also lacked the
`servo-paint-api -> libc` edge. At that point both lockfiles recorded the
direct dependency; the callback repair below removes it because the trace no
longer needs a direct `libc` dependency.

CI run #187 (`36115897284`) passed the locked host checks, target dependency
validation, Mesa Softpipe build, Nagi user-init link, and UEFI loader, then
timed out during the real two-boot M17 QEMU acceptance. The serial log again
ended at `Nagi M17 trace: GL context creation started`. None of the Servo
checkpoints written through `libc::write` appeared, so their absence cannot
show whether the patched Servo constructor was entered or whether that output
route failed.

The target Mesa build compiles Gallium Softpipe as its only renderer and links
EGL and Softpipe statically into the guest. Surfman already requests its
software adapter. Mesa EGL, however, normally derives software selection from
`LIBGL_ALWAYS_SOFTWARE` and can try a Zink override or hardware-related paths
before falling back to swrast. Nagi should enter the renderer path its build
actually provides.

## Decision

Under `__NAGI__`, EGL initialization sets `ForceSoftware` and clears `Zink` so
Mesa follows its surfaceless software-rendering path backed by the statically
linked Softpipe driver. Other targets retain Mesa's existing
environment-controlled policy.

Nagi-only warning-level diagnostics bracket EGL device discovery, driver
initialization, surfaceless software and no-DRM probes, and DRI screen creation.
Servo's Nagi patch adds checkpoints through Surfman device/context creation,
GL function loading, surface binding, make-current, and swap-chain creation.
These logs are diagnostic and do not change rendering, surface ownership, or
M17 acceptance criteria.

After CI #184, Servo's trace helper wrote directly to Nagi descriptor 2 through
`libc::write` rather than Rust stdio. CI #187 still produced no such trace, so
that route did not provide reliable evidence. The diagnostic patch now calls a
Nagi-only callback in the Albert adapter, which emits the stage through the
already working `libnagi::console_write` syscall. Other targets retain a no-op
helper; this callback is diagnostic-only and adds no runtime rendering fallback.

## Consequences

- Nagi will not request a hardware or Zink renderer that its static Mesa build
  does not provide. Mesa can still probe software-compatible DRM devices before
  falling back to no-DRM swrast; the new trace points make that path visible.
- Other Mesa targets keep their current renderer-selection behavior.
- The next target CI run will verify that Servo reached the patched constructor
  and locate the first Surfman stage after the application-level GL-context
  marker using Nagi's proven console syscall path.
- M17 remains `BLOCKED` until real Servo content is rendered, presented to the
  Nagi surface, and produces a nonzero pixel checksum. M18 remains `NOT STARTED`.
