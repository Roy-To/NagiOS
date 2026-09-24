# ADR 0022: Load the M17 Servo image through the real user ELF path

Status: accepted for the M17 Servo bootstrap continuation on 2026-09-25

## Context

Public CI run `36049471002` (#175, head
`01f6d3f42768ba2ba8d9fa6734474a54026c3055`) passed the M17 user-init target
link and UEFI loader build, then stopped before QEMU because the linked init
ELF was 127,747,368 bytes while the dedicated 8 MiB FAT12 ESP can store at most
8,372,224 bytes per file. The prior M5 bootstrap also caps the loader's static
init buffer at 4 MiB, the kernel's accepted init file at 4 MiB, and the user
image mapping at 1 MiB through a single page table. Enlarging the disk alone
would therefore move the failure into UEFI loading or kernel ELF validation.

M17 now has a real statically linked Servo/Mesa/relibc init executable. The
bootstrap must load its ELF segments inside Nagi while preserving the existing
W^X policy, static TLS contract in ADR 0021, and user capability boundaries.

## Decision

- Keep the legacy 1.44 MiB FAT12 image for earlier milestones. Build M17's
  separate FAT12 ESP with 32 KiB clusters and the largest valid FAT12 data
  cluster count, providing about 127.6 MiB of per-file capacity for the current
  127.7 MB init ELF plus the loader and kernel.
- Remove the UEFI loader's 4 MiB static init buffer. Determine the file size,
  allocate page-backed `LOADER_DATA` below 4 GiB, and read the file directly
  into that allocation. Retain a bounded 128 MiB file-size limit and report
  allocation, size, and short-read failures explicitly.
- Keep the loaded ELF allocation alive through the user process lifetime. The
  kernel validates its complete physical range against the active identity
  map before parsing it. Fully file-backed PT_LOAD pages map the corresponding
  loader-owned ELF pages into the user address space with the segment's
  existing permissions. A partial final page and zero-fill pages use
  allocator-owned, identity-checked pages that are cleared before use and
  receive only the segment's existing permissions.
- Expand the M17 user image window to 512 MiB using 256 page tables under the
  existing user PML4/PDPT/PD hierarchy. Keep the window bounded; place the
  existing stack, static TLS slots, display surface, and mmap area after it
  within the same 1 GiB page-directory span. Preserve ELF validation, W^X,
  range checks, TLS layout, and earlier milestone acceptance behavior.
- Use the kernel's conventional-memory page allocator for private PT_LOAD
  pages. Restrict allocations to identity-mapped physical memory below 4 GiB,
  validate each candidate mapping before dereferencing it, and retain allocated
  frames for the one-shot M17 process lifetime.
- Reject overlapping direct-mapped file pages when their segments have
  different write or execute flags, so zero-copy mappings cannot weaken ELF
  segment permissions through a physical alias.

This changes only the bootstrap capacity needed to load M17's real executable.
It does not add host rendering, host runtime dependencies, a fake pixel path,
or M18 browser behavior. The M17 acceptance gate remains a nonzero checksum
from Servo pixels presented through the guest Surface path in QEMU.

## Verification

CI #175 verified the target link and UEFI loader build but did not start QEMU;
it provides no guest loading or pixel evidence. The next authoritative target
run must verify the enlarged FAT12 image, UEFI file read, kernel ELF mapping,
and the existing real QEMU first-web-pixel acceptance. M17 remains `BLOCKED`
until that acceptance passes; M18 remains `NOT STARTED`.
