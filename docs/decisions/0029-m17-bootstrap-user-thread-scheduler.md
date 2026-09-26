# ADR 0029: Bounded M17 bootstrap user-thread scheduler

Status: accepted for M17 bootstrap
Date: 2026-09-26
Milestone: M17 — Servo Bootstrap

## Context

Public CI run 36232073962 (#238, head
18ee17af21a2dcf567497832a7e261e0d5f201c1) passed the target build and created
the real Mesa/EGL context. Servo then failed to create its memory-profiler
thread. The diagnostic trace reached `SYS_THREAD_CREATE` and identified the
single bootstrap child slot as occupied. The stack mapping and kernel thread
validation succeeded up to the occupied-slot check. Servo and Mesa both use
background threads before the first page can be rendered, so a one-child,
join-only context switch cannot satisfy M17.

The bootstrap currently enters ring 3 with interrupts disabled. It has no
TSS-backed privilege-transition stack for asynchronous user-mode interrupts.
The M3 timer scheduler acceptance exercises kernel tasks separately and does
not schedule these bootstrap user threads.

## Decision

- Keep one initial `nagi-init` process and its existing address-space and
  capability boundary. Extend that process to a fixed pool of 16 native user
  threads: thread ID 0 is the initial thread, and IDs 1–15 are reusable child
  slots.
- Give each slot an independent saved register/FPU context, static TLS page
  pair and FS base. Validate every entry as executable and every stack as a
  mapped, writable, page-aligned range in the process mmap window. Reset a
  reused slot's TLS from the validated ELF template.
- Schedule the process's threads cooperatively on the BSP. A new child becomes
  runnable; `SYS_THREAD_SLEEP` with zero yields; nonzero sleep and join block
  the caller; exit wakes a waiting joiner; and detach releases a completed
  slot. POSIX mutex contention yields and condition waits sleep through Nagi
  syscalls. The scheduler never switches to a host thread or host runtime.
- Keep ring-3 interrupts disabled until a TSS-backed interrupt path is
  implemented. This bounded cooperative scheduler is the bootstrap process
  path needed by M17; it does not claim general preemptive POSIX scheduling.
- Support child stacks up to 2 MiB, rounded to page boundaries; a thread with
  no explicit size receives a 2 MiB Nagi-owned stack. Increase the bootstrap
  mmap region descriptor bound to 64 so simultaneous stacks and Servo mappings
  can be tracked within the existing 128 MiB mmap window.
- Keep dynamic TLS modules, multiple processes, host pthreads, and host-rendered
  content unsupported. Preserve all existing syscall range, mapping, and
  capability checks.

This supersedes only the one-child and two-TLS-slot limits established for the
M5 bootstrap in ADRs 0005 and 0021. Their ELF, address-space, and static-TLS
validation rules remain in force. M18 remains `NOT STARTED` until M17's real
guest-rendered first-web-pixel acceptance passes.

## Verification

The implementation and focused checks are present in the local continuation
branch. The CI formatting command, kernel test-source check, Nagi kernel
release build, POSIX test-source check, Nagi POSIX target check, standalone
scheduler tests (9), standalone thread-helper tests (2), and `git diff
--check` passed on 2026-09-26. The complete Nagi user-init link was not
available locally because the Rust source and Mesa build directories are not
present. The POSIX test binary was not run on Apple Silicon; the Nagi syscall
assembly requires x86 registers. Public Ubuntu CI must run the host tests and
complete the target link.

Public Ubuntu `nagi-target` QEMU acceptance remains authoritative. It must
produce the real first-web-pixel checksum and pass marker before M17 can be
marked `PASS`; M18 stays `NOT STARTED` until then.
