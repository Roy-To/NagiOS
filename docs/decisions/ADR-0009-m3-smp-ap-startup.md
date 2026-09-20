# ADR-0009: M3 guest SMP startup and timer-frame scheduler

- Status: Accepted
- Date: 2026-09-17

## Decision

M3 starts APs with ACPI MADT processor discovery and xAPIC INIT/SIPI/SIPI.
The AP trampoline is copied into guest conventional memory and transitions the
AP through a low bootstrap GDT, protected mode, and the validated active BSP
GDT into long mode before entering the Rust kernel. A bounded four-CPU state
table records online, workload, preemption, wake, and context-switch progress.
Timer interrupts return selected saved register frames, allowing two
dedicated-stack kernel thread contexts per CPU to run preemptively.

## Reason

This is the smallest path that demonstrates real guest SMP rather than a host
CPU-count claim, while matching the QEMU reference target and preserving the
kernel/user-space boundary. The scheduler workload remains deliberately small
until later IPC and user-process milestones provide richer runnable entities.

## Consequences

The M3 implementation is tied to the official four-vCPU Developer Preview
target and rejects malformed or over-capacity ACPI CPU descriptions. The AP
path rejects an address space that does not identity-map the complete loaded
kernel image and every other required guest region, rather than guessing that
the current firmware mappings are usable. A richer dynamic CPU topology and
production scheduler are deferred beyond this milestone.
