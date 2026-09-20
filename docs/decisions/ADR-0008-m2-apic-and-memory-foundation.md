# ADR-0008: M2 memory and interrupt foundation

- Status: Accepted
- Date: 2026-09-17

## Decision

M2 uses the UEFI-provided memory map as the initial source for a bounded
physical page allocator and configures the x86-64 Local APIC timer directly
from kernel code. IDT exception entry points are small assembly adapters that
call typed kernel handlers.

## Reason

This exercises the actual post-`ExitBootServices` hardware path required by
the M2 acceptance criteria while keeping allocation, exception, and timer
policy in the kernel's low-level boundary. It avoids host timers and avoids
adding high-level filesystem, socket, or process functionality to the kernel.

## Consequences

The allocator and interrupt tables are intentionally bounded for the M2
Developer Preview. SMP-specific ownership and scheduling are deferred to M3.
