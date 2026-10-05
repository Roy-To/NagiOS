# ADR 0050: Concurrent isolated processes

Status: accepted
Date: 2026-10-03
Builds on: ADR 0043, ADR 0047, and ADR 0048

## Context

The kernel had exactly one isolated-process slot. As a result, two
user-space services or applications could never run isolated at the same
time, for example a Files provider beside a Search client. The single slot
was recorded as a known limit in ADR 0043.

## Decision

- **Slots.** `process_exit::MAX_LIVE_PROCESSES = 2` sets the number of
  concurrent isolated processes, and `user_process::child` follows it. Each
  slot has its own bounded address space: a PML4, a 1 MiB image, and a
  64 KiB stack. A slot is claimed by storing its Process ID atomically and
  is released only for that ID.
- **Address-space switching and pointer checks.** CR3 switching looks up
  the PML4 of the scheduled thread's owner (`cr3_of(pid)`). User-pointer
  checks use the active process's own slot. A process can never address
  another child's pages, because the same virtual layout is backed by
  different page tables.
- **Exit table.** `ExitTable` tracks every live process with its own
  optional waiter, and a thread may wait on only one process. Spawning
  reserves an exit-record slot for every live process plus the new one, so
  concurrent exits can never drop a status.
- **IPC and scheduler.** The user IPC manager holds init plus
  `MAX_LIVE_PROCESSES` process slots. Scheduler thread slots already carry
  their owner (ADR 0043).

## Verification

- Kernel host tests (158) cover independent waiters per concurrent process,
  record hold-back for live processes, ID monotonicity with two live
  processes, and two concurrent IPC process slots.
- `./nagi isolated-process` on QEMU/OVMF:
  - two faulting apps were live at once as PIDs 3 and 4;
  - a third direct spawn was refused;
  - PID 3's page fault was contained while PID 4 stayed live; PID 4 then
    raised #UD;
  - a third launch reused a freed slot as PID 5 (#GP);
  - each status was read through `SYS_PROCESS_WAIT`.

## Bounds

- Two slots cost about 2.3 MiB of kernel BSS. Raising the bound only
  requires changing `MAX_LIVE_PROCESSES` and memory.
- Scheduling of ring 3 remains cooperative (ADR 0029).
