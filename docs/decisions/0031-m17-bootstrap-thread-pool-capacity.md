# M17 bootstrap user-thread pool capacity

Status: accepted for the M17 bootstrap
Date: 2026-09-27
Milestone: M17 — Servo Bootstrap

## Context

Actions run 36289243570 (run #286, head
`f99faa4ae73b7ef35710b7c16b13753be42f7992`) verified that the M17 guest now
mounts the existing persistent volume into POSIX, initializes the guest
filesystem, and prepares `/tmp`. Servo then started both resource and storage
thread groups. The QEMU trace showed child thread IDs 1 through 15 occupied;
the next `pthread_create` failed in `SYS_THREAD_CREATE` because the fixed pool
defined by ADR 0029 had only 16 total slots, including initial thread 0. Servo
panicked with `WouldBlock` at `third_party/servo/components/storage/cache_storage.rs:239`.
No first-web-pixel checksum or PASS marker was produced.

The failure is pool capacity, not a path, stack validation, or host-thread
fallback. The 16-slot limit in ADR 0029 was chosen from the earlier single
worker trace and is too small for the actual pinned Servo startup topology.

## Decision

- Keep the single-process, cooperative, bounded user-thread scheduler and
  increase the bootstrap pool to 32 total thread slots: ID 0 remains
  `nagi-init`, and IDs 1–31 are child slots.
- Keep the existing per-thread register/FPU context, static TLS pair, stack
  mapping validation, syscall checks, and slot-reuse rules. Keep the 2 MiB
  default stack, 128 MiB process mmap window, and 64 mmap-region descriptors.
- The new capacity permits at most 31 concurrent child stacks, or 62 MiB at
  the default stack size, within the existing mmap window while leaving room
  for Servo and Mesa mappings. The scheduler remains cooperative and returns
  `EAGAIN` if its bounded pool is genuinely full.
- This is only the M17 bootstrap capacity. It does not claim unbounded or
  preemptive POSIX threads, change the single-process/capability boundary, or
  authorize M18 work.

## Verification

Capacity assertions and scheduler tests now allocate all 31 child slots,
observe bounded exhaustion, and verify released-slot reuse. The standalone
host harness compiled from the production scheduler source passed 9/9 tests.
The standalone POSIX thread-index harness compiled from the production source
passed 2/2 tests against the shared ABI capacity.
The kernel and POSIX custom Nagi target `cargo check` commands, all CI format
checks, and `git diff --check` pass. The TLS test now checks all 32 slots for
expected in-range, non-aliasing control pages. The POSIX target check reports
five pre-existing warnings. The full host workspace test command cannot run
on this ARM64 macOS host because `libnagi` contains x86-64 syscall-register
assembly; this remains covered by CI host jobs. Public `nagi-target` QEMU is
still authoritative for verifying that Servo startup advances to the real
first-web-pixel checksum and PASS marker. Keep M17 `BLOCKED` until those
markers are observed; M18 remains `NOT STARTED`.
