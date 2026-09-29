# ADR 0040: M18 POSIX allocation budget

Status: accepted for M18
Date: 2026-09-29
Milestone: M18 — Albert Browser

## Context

The local M18 QEMU run completed real certificate-chain and hostname
verification for two HTTPS origins, then Mesa Softpipe reported repeated
`POSIX allocator returned no block` failures. QMP sampling stayed in
`tgsi_ureg::ureg_property` with its `ureg_program` argument set to null. Mesa's
`ureg_create()` returns null when its allocation fails, matching the allocator
diagnostics. Nagi's C/POSIX heap is currently fixed at 64 MiB. At this
decision's checkpoint the bootstrap process had a bounded 256 MiB
anonymous-mapping window; ADR 0041 later raises it for M18 only.

## Decision

- Keep the existing 64 MiB POSIX heap for M17 and builds without the M18
  `browser-storage` feature.
- Give the M18 browser feature a bounded 128 MiB POSIX heap for Servo, MozJS,
  and Mesa C allocations.
- Keep the heap bounded at 128 MiB. The M18 mmap-window capacity is specified
  separately by ADR 0041.
- Keep all allocation guest-owned; do not introduce host allocation or kernel
  browser services.

## Consequences

M18 C-library allocations have twice the current budget while retaining a
fixed upper bound and the existing fail-closed allocation behavior. M17's heap
size and memory-map path remain unchanged. If the 128 MiB budget still cannot
render the acceptance pages, use the next QEMU allocator evidence to revise the
bounded M18 budget without changing the M17 path.

## Verification

The POSIX heap-budget unit test must check both feature configurations, the
M18 target build must include `browser-storage`, and `./nagi m18` must complete
all TLS, Servo-frame, chrome, and Nagi Surface acceptance markers.
