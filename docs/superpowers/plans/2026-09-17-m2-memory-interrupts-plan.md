# M2 Memory / Exceptions / Interrupts Implementation Plan

## Task 1: Memory foundation

- Add a no_std physical page allocator that consumes the M1 UEFI memory-map
  descriptor stride and admits only conventional pages.
- Add checked allocation/free behavior and host tests for reuse, alignment,
  invalid frees, and range exhaustion.
- Add page-table entry and kernel-heap primitives as foundations without moving
  high-level policy into the kernel boundary.

## Task 2: Exceptions and interrupts

- Add IDT entry encoding and runtime `lidt` installation.
- Add assembly stubs for a page fault with error code and a timer IRQ.
- Add Local APIC initialization, periodic timer configuration, EOI, and PIC
  masking.
- Keep all handlers diagnosable over COM1 and terminate only through the real
  QEMU test device after diagnostics.

## Task 3: Kernel integration

- Preserve the M1 BootInfo validation and exact boot line.
- Exercise page allocate/free, wait for real timer ticks, then trigger the
  deliberate invalid access so the real page-fault path is observable.

## Task 4: Verification

- Run focused unit tests, formatting, host lint/build/test, and the custom
  kernel target build.
- Run the QEMU acceptance script and inspect its serial log for timer and
  invalid-access diagnostics.
- Update `docs/implementation_status.md` only after the complete M2
  acceptance passes; then advance to M3.
