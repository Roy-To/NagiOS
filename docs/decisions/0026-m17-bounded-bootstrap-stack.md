# Decision 0026: Bounded M17 bootstrap stack

**Status:** Accepted
**Date:** 2026-09-25

## Context

The M17 target build and UEFI loader pass, but the real QEMU acceptance times
out during `SoftwareRenderingContext::new`. CI run #189 printed an Albert
console-callback self-test immediately before the call, but did not print the
patched constructor-entry checkpoint. The guest log has no user page-fault
diagnostic, so this does not prove the root cause.

The bootstrap process currently receives eight 4 KiB stack pages (32 KiB).
Servo's software rendering and Mesa/Softpipe initialization add a substantially
deeper native call path than the earlier init and audio paths. The current
address-space layout leaves a 4 MiB gap between the stack and TLS, and the stack
has one 512-entry page table.

## Decision

Increase the fixed bootstrap-process stack to 512 pages (2 MiB). This uses the
existing single stack page table, stays below the TLS region, and remains a
bounded allocation. The kernel already stores the stack's backing pages in its
static bootstrap storage; the change adds 2 MiB of kernel BSS on the 8 GiB QEMU
reference machine. The change is implemented in `kernel/src/user_process.rs`;
the kernel library suite passes locally (92/92). The Nagi-target release build
and QEMU run remain the required validation.

Keep the constructor and Surfman checkpoints. The next target run must verify
that it reaches the constructor and identify any later stop. If the checkpoint
still does not appear, revisit target symbol/cfg and entry-code evidence rather
than treating stack exhaustion as proven.

## Consequences

- The M17 bootstrap process can use up to 2 MiB of native stack.
- No dynamic or unbounded stack allocation mechanism is introduced.
- The fixed BSS cost is 2 MiB; TLS and mmap virtual addresses remain unchanged.
- M17 remains `BLOCKED` until real Servo content is presented to the Nagi
  surface and produces a nonzero checksum. M18 remains `NOT STARTED`.
