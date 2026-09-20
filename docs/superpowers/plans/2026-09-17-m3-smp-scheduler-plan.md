# M3 SMP / Scheduler / Threads Implementation Plan

## Task 1: CPU discovery

- Add checked ACPI RSDP/RSDT/XSDT/MADT parsing.
- Extract enabled Local APIC processor entries and identify the BSP.
- Add host tests for valid discovery, checksum failure, and bounded CPU count.

## Task 2: AP startup

- Add a reproducible Nagi-owned AP trampoline and per-AP stacks.
- Save the active BSP CR3, copy the trampoline to a low conventional page,
  patch its entry/data fields, and issue INIT/SIPI/SIPI through the xAPIC ICR.
- Use a low bootstrap GDT for the real-mode transition, load the active GDT in
  protected mode, and validate every identity-mapped region needed by AP code
  before dispatch. Keep a bootstrap stack in the copied trampoline for the
  real/protected-mode call-free address calculation and transition.
- Make AP entry publish online state only after entering long mode and setting
  up its IDT/APIC state; enable timer interrupts only afterward.

## Task 3: Per-CPU scheduler foundation

- Add bounded per-CPU state, atomic online/work/preemption/context-switch
  counters, thread states, and a blocked-to-runnable wake path.
- Run two real kernel thread contexts on every CPU; the timer handler saves the
  interrupted frame for the current task and returns the selected saved frame
  to perform preemptive switching with an ABI-aligned Rust call path.
- Keep all CPU state in Nagi kernel memory; do not use host threads.

## Task 4: Verification

- Run format, focused parser/scheduler tests, host lint/build/test, and both
  cross-target builds.
- Run PowerShell and Git Bash QEMU acceptance and inspect the guest serial log.
- Update `docs/implementation_status.md` only after all four CPUs report online
  and all workload checks pass.
