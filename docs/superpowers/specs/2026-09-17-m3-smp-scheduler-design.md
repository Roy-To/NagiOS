# M3 SMP / Scheduler / Threads Design

## Goal

Bring the four CPUs of the official QEMU reference machine online through
the guest Local APIC INIT/SIPI protocol, give each CPU bounded per-CPU state,
and run a real timer-driven scheduler workload on every online CPU.

## Boundaries

- CPU discovery uses the ACPI MADT referenced by the M1 `BootInfo`; host CPU
  counts and host threads are never used as guest evidence.
- The BSP copies a Nagi-owned 16/32/64-bit AP trampoline into a low
  conventional physical page and starts each AP with INIT/SIPI/SIPI.
- The real-mode trampoline uses a low, flat bootstrap GDT, then loads the
  validated active BSP GDT before enabling paging and long mode. APs reuse the
  BSP page-table context only after identity mappings for the trampoline,
  complete loaded kernel image, IDT, MADT-selected APIC, ACPI RSDP, state, and
  all stacks are verified. The real-mode and protected-mode bootstrap stages
  use a stack inside the trampoline page before switching to the per-CPU
  kernel stack.
- Per-CPU state is bounded to the four-CPU Developer Preview reference target.
  SMP-specific scheduler policy stays in the kernel and does not add POSIX or
  host-runtime dependencies.
- Timer IRQs preserve the interrupted register frame, align the temporary call
  stack for the kernel ABI, save the current thread's frame pointer, and
  return a different saved frame pointer. Each CPU has two real kernel thread
  contexts with dedicated stacks; the first thread starts runnable, the second
  starts blocked and is made runnable by the scheduler's wake path.
- MADT validation rejects malformed Local APIC entries, rejects x2APIC entries
  because M3 uses the xAPIC path, and applies a valid Local APIC address
  override before APIC initialization.

## Acceptance sequence

1. Parse the real ACPI MADT and identify the BSP plus three APs.
2. Start each AP using the real guest APIC startup sequence.
3. Each AP publishes its APIC ID and online state through shared atomic state
   after its IDT/APIC setup and before enabling timer interrupts.
4. Each CPU runs two real thread contexts, exercises a blocked-to-runnable
   wake transition, and records timer-driven preemption and context switches.
5. The BSP verifies all four CPU states and prints the M3 acceptance marker on
   the guest serial port.

## Testability

- ACPI table parsing and scheduler state transitions have host unit tests with
  synthetic tables/state.
- QEMU acceptance requires all four per-CPU serial/state markers and workload
  completion; a host-reported CPU count cannot satisfy the test.
