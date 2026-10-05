# ADR 0041: M18 browser memory window

Status: accepted for M18
Date: 2026-09-29
Milestone: M18 — Albert Browser

## Context

After the M18-only POSIX heap increase to 128 MiB, QEMU rendered both
`example.com` and `example.org` with verified TLS and real Servo frames. Loading
the third site then failed while Servo created a new script pipeline. The
kernel reported a denied 512-page (2 MiB) user stack mapping with only 260 mmap
pages free and a largest contiguous run of 256 pages. The existing 256 MiB
window was therefore insufficient for the M18 combination of the bounded C
heap, Servo worker stacks, and other guest mappings.

The M17 bootstrap uses the same user address-space layout and has already
accepted a 256 MiB mapping window. That behavior must remain unchanged.

## Decision

- Keep the default and M17 mmap window at 256 MiB.
- Add a kernel `m18-browser-memory` feature that also enables the existing
  `m18-browser-threads` feature and selects a 512 MiB M18 mmap window.
- Enable that feature only in the `./nagi m18` kernel build. Keep the same
  mmap ownership validation, per-thread 2 MiB stack size, rights checks,
  failure behavior, and maximum 64 thread slots.
- The M18 mmap range crosses the existing 1 GiB page-directory boundary.
  Give only the M18 address space a second page directory for the final four
  2 MiB entries; keep M17's single-directory mapping unchanged.
- Keep the M18 POSIX heap at 128 MiB as bounded by ADR 0040. The remaining
  address-space capacity is available to thread stacks and other mappings.

## Consequences

The M18 kernel's bootstrap storage reserves an additional 256 MiB of backing
pages compared with M17. This fits the official QEMU target's 8 GiB RAM budget
and avoids weakening the per-thread stack size or the allocator's failure
checks. M17 continues to use its existing feature graph, 256 MiB map window,
and 64 MiB POSIX heap.

## Verification

The kernel tests must validate the feature-specific mmap-window bound, the
M18 page-directory boundary, and range checks. `./nagi m18` must boot the
512 MiB M18 image and render all three real HTTPS pages, while the M17 kernel
continues to build with the 256 MiB window and its original page-directory
layout.
