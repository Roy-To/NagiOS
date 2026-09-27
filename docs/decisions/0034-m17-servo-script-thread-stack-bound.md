# ADR 0034: M17 Servo ScriptThread stack bound

Status: accepted for the M17 bootstrap
Date: 2026-09-27
Milestone: M17 — Servo Bootstrap

## Context

Actions run #289 (`36299606900`, head
`c14cd2f59a978c6f64fc8c8a0db6ac680aa4ad1f`) passed the target build through
the UEFI loader and QEMU completed AWS-LC TLS prewarm, Servo construction, and
WebView construction. The `Constellation` thread then panicked in the pinned
Rust std Unix thread backend at `pthread_attr_setstacksize`, which returned
`EINVAL` (22) twice. The second call is Rust std's page-alignment retry; the
requested stack was already aligned, so it retried the same size. No real
first-web-pixel checksum or PASS marker was produced.

The pinned Servo `ScriptThread` explicitly requests an 8 MiB stack in
`third_party/servo/components/script/event_loop/script_thread.rs`. Nagi's
POSIX stack helper and the `SYS_THREAD_CREATE` kernel validator both capped
each child at 2 MiB. The 2 MiB default from ADRs 0029 and 0031 is suitable for
workers that do not specify a larger stack, but it cannot be the maximum for
the pinned Servo call.

## Decision

- Keep the default child stack at 2 MiB and keep the existing bounded,
  cooperative thread pool.
- Set the M17 maximum child stack to 8 MiB, matching the explicit stack size
  used by the pinned Servo `ScriptThread`.
- Publish the page size and minimum, default, and maximum child stack sizes in
  `nagi-abi`. Make POSIX attribute normalization and kernel
  `SYS_THREAD_CREATE` validation use that same contract.
- Keep allocating stacks from the existing guest POSIX mmap window. Preserve
  the kernel checks for page alignment, in-window ranges, and writable mapped
  memory. If the bounded mmap window, mapping descriptors, or thread slots
  cannot satisfy a creation request, return the existing resource failure;
  accepting an 8 MiB attribute does not reserve or fabricate memory.
- Keep the 128 MiB process mmap window, 64 mmap-region descriptors, 32 thread
  slots, and the M17 first-web-pixel acceptance conditions unchanged.

## Consequences

- A thread without an explicit stack size still receives 2 MiB.
- POSIX accepts a requested size through 8 MiB when its page-rounded mapping
  fits the bound; a larger request is rejected with `EINVAL`.
- The kernel independently enforces the shared 8 MiB maximum and retains its
  mapped-writable-range validation.
- Aggregate stack usage remains limited by the existing guest address-space
  and thread-pool bounds. M17 remains `BLOCKED` until the real first-web-pixel
  checksum and PASS marker are produced; M18 remains `NOT STARTED`.

## Verification

Local verification passes: the `nagi-abi` unit tests (2), a standalone test
harness compiling the actual POSIX thread helper (3), the custom Nagi-target
kernel check, the custom Nagi-target POSIX check, affected-package nightly
rustfmt, and `git diff --check`. The POSIX target check has five existing
warnings in unrelated declarations; the stack-limit warning introduced during
development has been removed. Full POSIX package tests cannot run on the ARM64
Mac because `libnagi` contains x86-64 syscall-register assembly. Public target
CI remains required to prove that Servo's real `ScriptThread` is created and
that QEMU reaches the unmodified first-web-pixel checksum and M17 PASS marker.

The first CI run for this repair (`36303605684`, head `7cfb7f1`) passed Format
but its Ubuntu host Clippy step rejected the helper's manual min/max comparison
as `manual_range_contains`. The helper now uses an inclusive range check, and
isolated `cargo clippy -p nagi-abi --all-targets --locked -- -D warnings`
passes. The target job had not reached kernel, user-init, UEFI, or QEMU before
the lint correction; a new CI run must validate the corrected commit.
