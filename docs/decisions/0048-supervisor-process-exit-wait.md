# ADR 0048: Unique isolated Process IDs and Supervisor exit wait/status

Status: accepted
Date: 2026-10-03
Builds on: ADR 0043 (isolated process), ADR 0046 (launch registry), and
ADR 0047 (fault containment)

## Context

Before this ADR, the Supervisor learned that an isolated process had ended
only indirectly. It yielded up to eight times and probed whether its
endpoint's peer had become unreachable. That had three problems:

- the Supervisor could not tell a clean exit from a fault, or read the exit
  code;
- the probe depended on scheduling luck;
- every child was PID 2, so an ID could not name one specific process.

## Decision

1. **Unique IDs.** The kernel's `process_exit::ExitTable` hands out
   monotonically increasing Process IDs starting at 2. IDs are never
   reused. The user IPC manager keys its process slots by ID instead of by
   `pid - 1`.
2. **Exit records.** When an isolated process exits through
   `SYS_PROCESS_EXIT`, or is terminated by a fault (ADR 0047), the kernel
   records `{process_id, code, Exited | Faulted{vector}}`. At most four
   records stay unconsumed, and a spawn is refused while all four slots are
   full, so a status is never dropped.
3. **`SYS_PROCESS_WAIT(pid, *ProcessExitStatus)`**, init only:
   - **Exited process** — returns its status and consumes the record.
   - **Live process** — registers the caller as the single waiter and
     blocks it. The exit wakes it exactly once with `PROCESS_WAIT_RETRY`,
     and the libnagi wrapper calls again to consume the status.
   - **No other runnable thread** — the wait is withdrawn and fails instead
     of deadlocking.
   - **Unknown or consumed ID** — fails.
4. **`supervisor::reap`** now waits for the kernel status. It then checks
   that the peer endpoint became unreachable, revokes the launch record,
   closes the endpoint, and returns the status. Search and action clients
   must end with a clean exit 0.

## Verification

- Kernel host tests cover:
  - monotonic, never-reused IDs;
  - consume-once records;
  - a single waiter woken with the fault status;
  - cancelled waits and stale exits;
  - record-table bounds;
  - ID-keyed IPC slots.
- `./nagi isolated-process` on QEMU/OVMF:
  - the identity probe exited as PID 2 with code 0;
  - waiting again on PID 2, or on an unknown ID, failed;
  - the faulting apps ran as PIDs 3, 4, and 5. Init blocked in
    `SYS_PROCESS_WAIT` each time ("process-wait" trace), was woken by the
    fault, and read `Faulted` with vectors 14, 6, and 13 and codes 142, 134,
    and 141.
