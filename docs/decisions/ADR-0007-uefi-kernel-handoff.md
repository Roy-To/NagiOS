# ADR-0007: UEFI Loader and Plain BootInfo Handoff

**Status:** Accepted for M1

Nagi's M1 loader uses UEFI only before `ExitBootServices`. It hands the kernel
a versioned `repr(C)` `BootInfo` containing raw, stride-aware memory-map data,
GOP framebuffer information, and an ACPI RSDP address. The kernel has no UEFI
runtime dependency. This keeps the kernel boundary explicit and leaves page
tables, interrupt setup, and allocator policy for later milestones.

The loader accepts only identity-mapped ELF load segments in M1 and allocates
their physical pages with UEFI Boot Services. A later milestone may introduce
virtual relocation after the architecture and memory model are established.

