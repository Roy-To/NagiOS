# ADR 0023: Move the fixed kernel image above the UEFI low-memory reservations

Status: accepted for the M17 Servo bootstrap continuation on 2026-09-25

## Context

CI run `36076724861` (#179, head
`2821d0156841c9afe7fa11b3755dbfd0f09b9e13`) built the kernel, the real Servo
user init, and the UEFI loader, then failed before the kernel entry point. The
loader reported that PT_LOAD segment 2 could not be allocated at `0x219000`;
the segment spans `0x120d820` bytes (4,622 pages). The same failure reproduced
locally with QEMU 11.1 and OVMF after the kernel and loader were rebuilt from
the same source.

The UEFI memory map showed why the fixed range could not be reserved:

- conventional memory: `0x219000`–`0x800000`;
- ACPI non-volatile memory descriptors around `0x800000`–`0x900000`;
- boot-services data: `0x900000`–`0x1780000`.

The current kernel's 18.9 MiB writable PT_LOAD includes static mmap backing
storage and the additional M17 image page tables. Starting at the old 2 MiB
link base, it crosses those firmware-owned descriptors. The loader correctly
uses `AllocateAddress` and cannot place a fixed-address ELF over them.

## Decision

- Keep the kernel a fixed-address ELF with identity-mapped physical/virtual
  addresses and no runtime relocation step.
- Move its link base from 2 MiB to 64 MiB (`0x04000000`). This keeps the
  existing contiguous PT_LOAD layout intact and places the current kernel
  image beyond the observed UEFI low-memory reservations in the official 8
  GiB QEMU reference machine.
- Keep the loader's exact-address allocation and error reporting. Do not
  overwrite firmware-reserved memory or treat non-conventional descriptors as
  free.

This is a boot-layout adjustment only. It does not change kernel/user
authority, the physical page allocator, or the M17 pixel acceptance gate.
M17 remains `BLOCKED` until the public target acceptance produces the real
Servo frame checksum through Nagi Surface; M18 remains `NOT STARTED`.

## Verification

Before this change, local QEMU/OVMF reproduced the exact allocation failure
and exposed the overlapping memory descriptors on the serial log. After the
change, the local QEMU boot printed `Nagi Kernel started` and passed M2, M3,
and M4 acceptance. The default non-Servo init then failed M5 ELF validation on
its zero-sized PT_TLS header, so GitHub Actions `nagi-target` remains the
authoritative check for the real M17 image and unchanged pixel acceptance.
