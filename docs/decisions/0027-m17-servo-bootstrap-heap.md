# Decision 0027: Expand the Bounded M17 Bootstrap Heap

**Status:** Accepted
**Date:** 2026-09-26
**Milestone:** M17 — Servo Bootstrap

## Context

GitHub Actions run #232 (`36220293827`, head `35efaf6`) passed both host jobs,
all target builds, the two-boot persistence check, EGL/Softpipe initialization,
and GL context creation. Servo construction then reported
`memory allocation of 512 bytes failed`; the guest did not exit and QEMU timed
out after 120 seconds. No first-web-pixel checksum was produced. The failure
occurred after the previous guest random failure and GL-context blocker, so it
identifies a later bootstrap stage.

The bootstrap POSIX allocator currently backs Rust/C++ `malloc` with one
8 MiB Nagi anonymous mapping. The kernel's mmap region is 16 MiB total and is
backed by static bootstrap pages. A failing small allocation during Servo
construction is consistent with the shared heap having no fitting block, but
the current trace does not rule out allocator corruption or a failed backing
mapping.

The official QEMU reference machine has 8 GiB RAM. M17 must run the real guest
Servo and Mesa paths without a host allocator or a fake rendering result.

## Decision

Keep the POSIX heap bounded and backed only by guest `SYS_MEMORY_MAP`, but
increase its capacity from 8 MiB to 64 MiB. Increase the bootstrap mmap window
from 16 MiB to 128 MiB, leaving 64 MiB of address and backing capacity for
other mappings such as JIT reservations. Keep the existing four-region limit
and allocator/free-list behavior. Replace the per-page temporary bitmap in
`mmap_user` with a first-fit scan over the four registered regions: the syscall
uses a 16 KiB kernel stack, and a bitmap for a 128 MiB window would exceed it.

When `nagi_posix_malloc` fails, print a static serial marker that distinguishes
an unavailable heap mapping from a mapped heap whose allocator returned no
block.
This adds no allocation, fallback, or success behavior to the allocator.

The next public QEMU acceptance is the deciding evidence. It must reach Servo
construction and produce the real nonzero frame checksum and M17 PASS marker;
the acceptance criteria are unchanged.

## Consequences

- The first-fit POSIX heap remains finite, guest-backed, and shared by the
  target Rust/C++ allocation boundary.
- The bootstrap mmap backing grows from 16 MiB to 128 MiB, adding 112 MiB of
  kernel BSS on the 8 GiB reference machine.
- Mmap first-fit search uses bounded region metadata rather than stack space
  proportional to the window size.
- Up to 64 MiB remains available in the mmap window after the POSIX heap's
  initial mapping, within the existing four-region limit.
- This is a resource-budget correction for M17 bootstrap, not an unbounded
  allocator or a host-memory escape.
- M17 remains `BLOCKED` until the real guest frame is accepted; M18 remains
  `NOT STARTED`.
