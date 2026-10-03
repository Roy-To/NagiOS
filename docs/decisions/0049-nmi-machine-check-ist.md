# ADR 0049: IST stacks for NMI and #MC

Status: accepted
Date: 2026-10-03
Milestones: M2/M3 (exceptions, SMP)
Builds on: ADR 0047 (BSP TSS) and ADR 0048 (per-AP TSS, #DF on IST1)

## Context

After ADR 0048, only #DF had an IST stack. NMI and #MC are asynchronous.
They can arrive at any instruction, including the few instructions after a
context switch where RSP is not yet valid. That window is the one the M3
frame defect exposed. Without an IST entry the CPU pushes the exception
frame onto the current stack. If that stack is invalid, the push faults and
the guest resets.

The APs also ran with CR4.MCE clear: the trampoline set only PAE and the
SSE bits. With CR4.MCE clear, a machine check shuts the processor down
instead of raising #MC, so an #MC gate alone would never run on an AP.

## Decision

1. **IST slots.** `cpu_tables` fixes one IST slot per exception:

   | Exception | Vector | IST slot |
   |-----------|--------|----------|
   | #DF       | 8      | IST1     |
   | NMI       | 2      | IST2     |
   | #MC       | 18     | IST3     |

   `exception_ist(vector)` selects the slot for every gate, so the BSP and
   AP IDTs cannot diverge. `TaskStateSegment::with_ist_stacks` fills
   IST1..=IST3.
2. **Stacks.** Each CPU has three dedicated 16 KiB IST stacks:
   - BSP: `BSP_IST_STACKS`;
   - each AP: `AP_IST_STACKS[cpu]`.

   No two exceptions share a stack, so an #MC that arrives while the NMI or
   #DF handler runs does not overwrite that handler's frame.
3. **AP CR4.MCE.** The AP trampoline now sets CR4.MCE, matching the BSP,
   where firmware already set it.
4. **Policy unchanged.** NMI and #MC still go to `exception_entry`, which
   reports the exception and halts that CPU. Neither handler returns, so a
   second NMI stays blocked and cannot re-enter IST2.

## Verification

- Kernel host tests: 154 pass, including a test of the IST slot map and
  `with_ist_stacks`.
- **Diagnostic feature `m3-ap-ist-probe`.** The last two APs disable
  interrupts and spin with RSP=0. The BSP sends an NMI IPI to the last AP.
  The QEMU monitor then injects `mce 2 1 0xb200000000000000 0x5 0 0` into
  CPU 2. QEMU/OVMF logged no triple fault, and the serial log shows:
  - `Nagi AP exception apic=3 vector=2 ... rsp=0x0`
  - `Nagi AP exception apic=2 vector=18 ... rsp=0x0`
- The ADR 0048 #DF probe still reports `vector=8` with no triple fault.
- 24 of 24 default-kernel boots under 8-way QEMU contention reach
  `Nagi M3 acceptance PASS`.

## Bounds

- The BSP gap before the M5 GDT switch is closed by ADR 0050.
- NMI is treated as fatal. No watchdog or profiling NMI source exists yet.
- #MC is reported but not decoded: the MCi_STATUS banks are not read.
