# ADR 0038: M18 Servo worker capacity

Status: accepted for M18
Date: 2026-09-29
Milestone: M18 — Albert Browser

The 256 MiB M18 mmap-window bound in this decision is superseded by
[ADR 0041](0041-m18-browser-memory-window.md). M17 retains the 256 MiB window.

## Context

M18 Acceptance reached the real HTTPS navigation path after correcting a
cooperative-scheduler starvation in Albert's event loop. The next QEMU run
then failed while Servo created the HTTPS pipeline: `SYS_THREAD_CREATE`
reported that the 32-slot bootstrap pool was full, and Servo aborted with
`WouldBlock`. M18 starts the pinned Servo networking and storage workers in
addition to the Constellation and page-script threads required for multiple
real websites.

ADR 0031 intentionally set 32 total slots for the M17 first-web-pixel
bootstrap. That limit remains appropriate for M17 and must not be silently
changed by M18.

## Decision

- Preserve the default 32 total slots for M17 and earlier builds.
- Add an explicit `m18-browser-threads` ABI/kernel feature that gives the M18
  browser image 64 total cooperative slots, including initial user-init slot
  zero. `./nagi m18` enables the feature for both the user image and kernel.
- Keep the existing per-thread context, static TLS isolation, stack validation,
  reusable-slot rules, 2 MiB default stack, and 8 MiB per-thread maximum.
- Scale the bounded mmap-region descriptor count with the selected thread
  capacity at two descriptors per thread slot. The existing 256 MiB user mmap
  window remains the hard bound on actual stack and mapping memory; exhausted
  mappings continue to return the existing resource error.
- Keep scheduling cooperative and bounded. This decision does not add
  preemption, increase authority, or change the M17 feature graph.

## Consequences

- M17 continues to use 32 thread slots and the existing 64 mapping descriptors.
- M18 can start the additional Servo workers needed by multiple browser
  pipelines, with a maximum of 63 child slots and at most 126 MiB in default
  2 MiB stacks before other mappings are counted.
- M18 can still fail cleanly if explicit large stacks or other mappings exhaust
  the fixed 256 MiB address window.

## Verification

The M18 target dependency graph must include `m18-browser-threads`; the M17
graph must exclude it. Kernel TLS and scheduler tests must exercise the
selected count. M17 and M18 QEMU acceptance remain the runtime checks for the
separate capacities.
