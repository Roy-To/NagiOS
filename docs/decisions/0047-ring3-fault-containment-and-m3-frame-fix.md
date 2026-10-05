# ADR 0047: Ring-3 fault containment and the M3 self-test frame fix

Status: accepted
Date: 2026-10-03
Milestones: M2/M3 (exceptions, SMP), M5 (user process); prerequisite for
isolated M18–M23 services
Builds on: ADR 0029 (bootstrap scheduler) and ADR 0043 (isolated process)

## Context

There were two separate defects.

**No containment of ring-3 faults.** The BSP ran ring 3 without a Task
State Segment. When a CPU exception occurred at CPL 3, the CPU had no RSP0
to switch to and triple-faulted the machine. Any fault in the
ADR 0043 isolated process therefore took down the whole OS, and ADR 0043
recorded this as a known limit. The shared M3 IDT also uses the UEFI
code selector, which is not valid in the five-entry M5 GDT. A BSP
exception after the GDT switch therefore had no usable gate either.

**Intermittent M3 SMP stall.** QEMU acceptances (M22, M27, M28) sometimes
stopped during `Nagi M3 scheduler workload START`. QMP reported the guest
`shutdown`, with:

- RIP at `smp::thread_entry`;
- RSP=0;
- CR2=0xffff_ffff_ffff_fff8.

The root cause was in `smp.rs`. Each initial self-test task frame had 18
words: 15 GPRs plus RIP, CS, and RFLAGS. In 64-bit mode `iretq` always
also pops RSP and SS, so it read two words past the frame. A task
therefore started with an arbitrary RSP, usually 0, and interrupts
enabled. A timer interrupt arriving before `thread_entry` set its own
stack pushed to address -8 and triple-faulted. Whether this happened
depended on timing, which made the stall intermittent.

## Decision

1. **M3 frame.** `scheduler::m3_initial_task_frame` builds the full 20-word
   frame with an explicit RSP and SS, the current stack selector.
   `smp.rs` uses it. A lib unit test pins the layout, and CI runs it.
2. **BSP TSS.** `cpu_tables` defines the 104-byte 64-bit TSS and its
   descriptor encoding, with host tests. `install_syscall_gdt` adds:
   - the TSS descriptor at selector 0x28, loaded with `ltr`;
   - RSP0, a dedicated 16 KiB kernel fault stack;
   - IST1, a separate 16 KiB stack for #DF;
   - no I/O permission bitmap.
3. **BSP exception IDT.** After the GDT switch, the BSP loads its own IDT.
   All 32 exception vectors go to per-vector stubs that normalize the error
   code, all using selector 0x08; #DF uses IST1. The timer gate is mirrored.
   The APs keep the shared M3 IDT.
4. **Policy in `exception_entry`.**
   - **CPL 0 (kernel fault)** — report vector, error, RIP and CR2, then
     halt. Still fatal, but now diagnosed instead of a triple fault.
   - **CPL 3 in init** — report and halt. init is the Supervisor; losing it
     is not recoverable.
   - **CPL 3 in an isolated process** — terminate only that process with
     exit code 128 + vector. This uses the same
     `terminate_isolated_process` path as `SYS_PROCESS_EXIT`: close its
     handles, remove its thread, restore init's CR3, scrub its pages. The
     next runnable thread then resumes through `nagi_resume_user_context`,
     which uses the same `sysretq` sequence as the syscall return path.
     The Supervisor sees the peer endpoint become unreachable, reaps the
     launch, and revokes its grants (ADR 0046).

## Verification

- Kernel host tests: 152 passed, including the TSS layout and descriptor
  encoding, the gate and error-code tables, and the M3 frame layout.
- `./nagi isolated-process` on QEMU/OVMF, using the new
  `nagi-faulting-app` ELF under its own manifest. Three launches raised
  #PF (write to unmapped init TLS), #UD (`ud2`) and #GP (`hlt` at CPL 3).
  The kernel reported each fault and terminated only the child (exit codes
  142, 134 and 141). The Supervisor reaped each launch, the slot was
  reused, and init continued its boot.
- Repeated `./nagi run` boots were used to check the M3 fix; see the
  implementation status.

## Bounds

- Only the BSP runs user code, so only the BSP has a user-fault path. APs
  received their own TSS, #DF IST1 stack, and exception IDT in ADR 0048.
- Faults in init and in the kernel remain fatal by design.
- The Supervisor learns of the exit through peer closure. A process-exit
  wait or exit-status query for the Supervisor is later work.
