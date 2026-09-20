# M1 UEFI -> Kernel Implementation Plan

> **For agentic workers:** Use subagent-driven-development for independently
> reviewable work and verification-before-completion before claiming PASS.

**Goal:** Implement and verify the first real UEFI-to-Nagi-kernel boot path.

**Architecture:** Shared `nagi-bootinfo` contract, fixed-address ELF64 kernel,
Rust UEFI loader, actual FAT12 ESP image, QEMU/OVMF serial acceptance.

## Global constraints

- Nagi remains an independent OS; Linux/Windows are build hosts only.
- No host filesystem, host process, fake serial line, or hard-coded acceptance
  result may stand in for guest behavior.
- No high-level filesystem or device syscall is added to the kernel.
- UEFI is used only by the pre-exit loader. The kernel receives plain data and
  does not depend on the UEFI crate at runtime.

## Task 1: M1 contract and target setup

- [x] Add `crates/nagi-bootinfo` with stable `repr(C)` BootInfo, memory-map,
  framebuffer, and ACPI fields.
- [x] Add host unit tests for valid/invalid handoff validation.
- [x] Add and pin the custom `x86_64-unknown-nagi` target and kernel linker
  script; install the pinned `x86_64-unknown-uefi` target.
- [x] Observe the focused tests/build fail before implementation where useful.

## Task 2: Kernel

- [x] Add a `no_std`, `no_main` ELF64 kernel entry.
- [x] Implement COM1 serial output using port I/O owned by the guest kernel.
- [x] Validate BootInfo and print the exact M1 acceptance line.
- [x] Add the explicit QEMU debug-exit instruction only as a test-device exit
  after the real serial output.
- [x] Build the kernel with `-Zbuild-std=core,compiler_builtins`.

## Task 3: Loader

- [x] Add a `no_std` Rust UEFI application using pinned `uefi` crate `0.37.0`.
- [x] Read the kernel ELF from the UEFI FAT volume.
- [x] Add checked ELF parsing, fixed-address segment allocation, copy, and BSS
  zeroing with unit tests for malformed/truncated inputs.
- [x] Capture GOP, ACPI, and the final memory map.
- [x] Call `ExitBootServices` and transfer control to the kernel entry.

## Task 4: Image and run orchestration

- [x] Add a deterministic FAT12 image writer to the host CLI with directory
  entries for the UEFI fallback path and kernel payload.
- [x] Implement `nagi image` to build both guest artifacts and emit an ESP
  image under `out/artifacts`.
- [x] Implement `nagi run` to invoke QEMU with the configured OVMF pair and
  serial log, bounded by a timeout and with no host fallback.
- [x] Add the M1 acceptance script and log checks.

## Task 5: Verification and status

- [x] Run focused host tests after each implementation step.
- [x] Run formatting, clippy, host build/test, cross-target builds, image
  inspection, and the real QEMU serial acceptance test.
- [x] Fix evidence-backed failures up to ten meaningful attempts.
- [x] Update `docs/implementation_status.md` and mark M1 `PASS` only after the
  exact QEMU serial acceptance criterion passes; then advance to M2.
