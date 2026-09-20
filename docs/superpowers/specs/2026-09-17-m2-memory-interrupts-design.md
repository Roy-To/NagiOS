# M2 Memory / Exceptions / Interrupts Design

## Goal

Extend the real M1 UEFI-to-kernel path with a physical page allocator, IDT
exception dispatch, a Local APIC periodic timer, and a deliberate invalid
access diagnostic. All evidence must come from the Nagi guest serial port.

## Boundaries

- The kernel consumes the UEFI memory map captured in `BootInfo`; it does not
  call UEFI after `ExitBootServices`.
- The allocator owns only `EfiConventionalMemory` pages and returns aligned
  physical page addresses. It has a bounded in-kernel range table and a small
  freed-page stack for the M2 acceptance path.
- The IDT is installed by the kernel with a runtime code-segment selector. The
  page-fault handler reports the vector/error diagnostic and terminates the
  test guest through the existing QEMU debug-exit device.
- The timer uses the guest Local APIC MMIO interface at the architectural
  xAPIC base, with a periodic vector and EOI. PIC lines are masked before
  interrupts are enabled.
- The kernel keeps its existing fixed-address ELF and serial output. No host
  memory, host signal, host timer, or host process is used as guest evidence.

## Acceptance sequence

1. Print the M1 boot line.
2. Construct the allocator from the real UEFI memory map, allocate a page,
   free it, allocate again, and print the allocation/free PASS marker.
3. Install the IDT and Local APIC timer, enable interrupts, and wait for real
   timer vectors while halted.
4. Print the timer PASS marker after multiple timer interrupts.
5. Read from an intentionally invalid canonical address. The vector-14 page
   fault stub prints an invalid-access diagnostic and the page-fault handler
   exits the QEMU test guest.
6. Print `Nagi M2 acceptance PASS` only after steps 2-4; the invalid-access
   diagnostic is emitted by the real fault handler in step 5.

## Testability

- The page allocator range and allocation/free logic have host unit tests with
  synthetic UEFI descriptor data.
- IDT gate encoding and APIC register programming are kept in small functions
  that can be checked without enabling interrupts on the host.
- The QEMU acceptance script requires the real serial log to contain the M2
  markers and rejects a launcher failure or missing log.
