# ADR 0059: Physical frame pool for the bootstrap mmap window

Status: accepted
Date: 2026-10-05
Milestones: M20 (AI Runtime / Granite); affects M17, M18, M25 memory variants

## Context

The bootstrap process's anonymous `mmap` window was backed by a static
`[PageBytes; USER_MMAP_PAGES]` array in kernel BSS, and its page-directory
hierarchy was spelled out per memory feature (one extra directory for
`m18-browser-memory`, two for `m25-whisper-memory`). `m20-llama-memory`
sizes the window at 4 GiB so Granite 4.2 3B (2.2 GB of weights plus context
and compute buffers) fits. With static backing that is 4 GiB of BSS: the
kernel no longer links (32-bit relocations into BSS overflow), and even if
it did, QEMU q35 places only about 2 GiB of RAM below 4 GiB. The hierarchy
also had no directories for a window that spans five 1 GiB PDPT slots. This
variant had never been linked before because the guest C++ runtime blocked
the init link first (ADR 0058).

## Decision

- The mmap window is backed by physical frames, not BSS. `BootstrapStorage`
  records one frame address per window page and keeps a pool of free frames.
  After the bootstrap address space is built, `prepare` moves pages from the
  boot page allocator into the pool, up to the window size, accepting only
  frames the active address space identity-maps (the kernel zeroes them
  through that mapping; OVMF identity-maps memory above 4 GiB as well). The
  allocator has no other user once the bootstrap process is prepared.
- `mmap` takes zeroed frames for every page of a new reservation or fails
  with `PhysicalMemoryExhausted` without side effects; `munmap` and failed
  `mmap` return them. `PROT_NONE` and `mprotect` keep a page's frame, so
  reserve/commit callers see their data again.
- The extra page directories are an array sized from the window at compile
  time, so every memory variant maps its full window from the same code.
- Boot logs `Nagi M5 trace: mmap frame pool pages=N`.

## Consequences

- Kernel BSS no longer grows with the mmap window; every variant reserves
  the same physical memory it did before, now from conventional memory.
- The 4 GiB `m20-llama-memory` kernel links, and the window is limited by
  guest RAM instead of the kernel image.
- The pool belongs to the single bootstrap process. A general VM service
  that shares frames between processes remains future work.
