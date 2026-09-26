# Decision 0028: Support POSIX Over-Aligned M17 Allocations

**Status:** Accepted
**Date:** 2026-09-26
**Milestone:** M17 — Servo Bootstrap

## Context

GitHub Actions run #233 (`36223836342`, head `1993a45`) passed the host jobs
and all target builds. Its real QEMU acceptance created the Mesa GL context,
entered Servo construction, and then reported `memory allocation of 512 bytes
failed`. The run produced no first-web-pixel checksum. The 64 MiB POSIX heap
failure markers were absent, so the trace did not indicate that the backing
heap itself had run out of blocks.

The pinned Rust standard library's Unix `System` allocator calls `malloc` for
ordinary layouts and calls `posix_memalign` for layouts requiring stronger
alignment. Nagi's current weak `posix_memalign` implementation allocates with
the normal 16-byte-aligned `malloc`, then returns `ENOMEM` whenever that
pointer does not meet the requested alignment. It neither supplies a stronger
alignment nor releases the temporary allocation. That is a deterministic
failure path for Rust layouts aligned beyond 16 bytes and is consistent with
the run's small-allocation OOM, though the serial output did not include the
layout alignment and the next target run must verify this diagnosis.

## Decision

Keep all allocation inside the finite guest-backed POSIX heap. For valid
alignments up to 16 bytes, use the existing allocator directly. For larger
alignments, reserve `size + alignment - 1 + 32` bytes from that same allocator,
align the returned payload inside the reserved extent, and store an in-heap
header immediately before it containing the raw pointer, alignment, requested
size, and a distinct magic value.

`nagi_posix_free` validates the aligned header and frees the original heap
allocation. `malloc_usable_size` reports the requested payload size, and the
requested size remains at the existing `pointer - 16` location used by the
target `realloc` implementation. Invalid alignments and arithmetic overflow
fail with an error; no host allocator or fallback is introduced. The target
C++ ABI's aligned throwing and nothrow `operator new` / `operator new[]`
overloads pass their `std::align_val_t` to this same allocator, and the
existing aligned delete overloads return those pointers through
`nagi_posix_free`. `posix_memalign` follows its POSIX return convention by
returning `EINVAL` or `ENOMEM` directly and leaving `errno` untouched on
failure.

## Consequences

- Rust `System` layouts with valid power-of-two alignment above 16 bytes can
  use the Nagi POSIX heap.
- C++ over-aligned allocations honor the alignment supplied by
  `std::align_val_t` and use the same bounded POSIX heap.
- An over-aligned request adds `alignment - 1 + 32` bytes before the existing
  allocator rounds the payload to 16 bytes. Requests that exceed the finite
  heap still fail explicitly.
- Freeing and allocator introspection recover the original allocation from
  Nagi-owned metadata; returned pointers remain inside the guest mapping.
- M17 remains `BLOCKED` until public target CI produces the real frame checksum
  and PASS marker. M18 remains `NOT STARTED`.

## Public QEMU result from CI run #237

Actions run 36228589557 (#237, head
c54af8a046bd510f63aa5f88e2ecbe66dc737101) passed the target builds, UEFI,
persistent storage, and Mesa GL context creation. It advanced beyond the prior
512-byte allocation failure and then stopped when Servo's memory-profiler
thread creation returned EAGAIN. The trace does not include the failed
allocation's layout or alignment, so this run is consistent with the allocator
repair advancing the path but does not independently prove the exact
over-aligned request was served. There is still no first-web-pixel checksum or
PASS marker; M17 remains BLOCKED.
