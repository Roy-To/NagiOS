# Decision 0014: Preserve boot-stage marker boundaries

## Context

The boot renderer reports `PLATFORM` before M6, `CORE_SERVICES` after the M6
acceptance work, and `STORAGE` immediately after the M7 storage operation
succeeds. The existing M10 user entry point prints the aggregate `Nagi M7
acceptance PASS` marker only after those operations and after the renderer's
early stage calls.

The Task 4 draft described all boot markers as appearing after `Nagi M7
acceptance PASS`. That ordering cannot be true without either reporting a
stage before its corresponding work or moving the renderer out of the real
initialization boundaries.

## Decision

Keep the guest lifecycle boundaries and their serial output order authoritative.
Host acceptance contracts will enforce the actual ordered sequence:

1. `Nagi boot stage PLATFORM 15` occurs after the early kernel markers and
   before M6 service markers.
2. `Nagi boot stage CORE_SERVICES 30` occurs after the M6 service call marker
   and before M7 storage markers.
3. `Nagi boot stage STORAGE 50` occurs after the M7 persistence marker and
   before the aggregate M5/M6/M7 acceptance markers.
4. `GRAPHICS`, `SESSION`, lock/collapse/checksum markers occur after
   `Nagi M7 acceptance PASS` and before `Nagi M10 desktop READY`.

The CLI desktop result separately enforces the boot markers' own order and
requires them before desktop readiness. It does not invent a false relation
between the early boot stages and the later aggregate M7 marker.

## Consequences

- Acceptance scripts test the real initialization sequence rather than a
  reordered log.
- The renderer remains useful for diagnosing failures at the stage where they
  occur.
- No kernel, syscall, or M13 behavior changes are required.
