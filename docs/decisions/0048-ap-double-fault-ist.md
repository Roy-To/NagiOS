# ADR 0048: Per-AP TSS and #DF handler on IST1

Status: accepted
Date: 2026-10-03
Milestones: M2/M3 (exceptions, SMP)
Builds on: ADR 0047 (BSP TSS and exception IDT)

## Context

ADR 0047 gave the BSP a TSS, an IST1 stack for #DF, and its own exception
IDT. The APs kept the shared M3 IDT, which had only the timer gate and the
M2 test #PF gate, on the firmware code selector. They also had no TSS.

So any kernel stack fault on an AP still escalated #PF → #DF → triple fault.
That is the M3 failure class that ADR 0047 fixed only at its source. The
fault took down the whole guest and left no diagnostic: QMP showed only
`shutdown`.

## Decision

1. **Per-AP GDT and TSS.** In `interrupts::initialize_ap(index)`, each AP
   loads its own kernel-only GDT from `cpu_tables::ap_gdt`:
   - null;
   - kernel code at 0x08 and kernel data at 0x10;
   - null user slots;
   - its own 104-byte TSS at 0x28.

   The AP then reloads CS, SS, DS and ES, and runs `ltr 0x28`. The selector
   layout matches the BSP's M5 GDT, so the same gate selector and TSS
   selector work on every CPU. The user slots stay null because APs never
   enter ring 3. For the same reason the AP TSS has no RSP0.
2. **IST1 #DF stack per AP.** Each AP has a dedicated 16 KiB #DF stack,
   referenced by IST1 in its TSS.
3. **Shared AP exception IDT.** The BSP fills `AP_IDT` once in
   `interrupts::initialize`, before any SIPI. The table is read-only after
   that. It contains:
   - all 32 exception stubs, on selector 0x08;
   - #DF on IST1, which each AP's own TSS resolves to that AP's stack;
   - the timer gate.
4. **Reporting.** `exception_entry` checks `interrupts::is_bsp()` first. On
   an AP it reports `Nagi AP exception apic=… vector=… error=… rip=… rsp=…
   cr2=…` and halts that AP with interrupts disabled. It does not touch the
   BSP's thread table.
5. **M3 task selectors.** AP self-test task frames now use the AP GDT
   selectors (0x08/0x10). BSP task frames keep the firmware selectors that
   are active on the BSP during M3.
6. **AP SSE state.** The trampoline now sets CR4.OSFXSR and CR4.OSXMMEXCPT,
   plus CR0.MP and CR0.NE, alongside PAE and PG. Before this change, APs ran
   compiled kernel code with SSE disabled (CR4 = 0x20), while firmware had
   already enabled SSE on the BSP. Writing the per-AP TSS was the first AP
   path where the compiler emitted SSE (`xorps`), and it raised #UD. This
   was a latent defect, not a new requirement.

## Verification

- Kernel host tests: 153 pass. These include `ap_gdt` layout and descriptor
  values. The ADR 0047 source-order test now checks that `cli` precedes
  `load_kernel_gdt` and that the helper performs `lgdt` without `sti`.
- **Diagnostic feature `m3-ap-double-fault-probe`.** The last AP sets RSP=0
  and pushes, which reproduces the original failure. On QEMU/OVMF (q35,
  4 vCPU) the guest no longer triple-faults. It prints:
  `Nagi AP exception apic=3 vector=8 … rsp=0x0 cr2=0xfffffffffffffff8`.
  The BSP then reports `Nagi M3 AP online timeout`. The feature is
  diagnostic only and is never part of a product image.
- The default kernel was stress-booted under 8-way parallel QEMU contention;
  see the implementation status.

## Bounds

- A fault on an AP is still fatal for that AP. It is reported instead of
  resetting the guest. The BSP notices the missing CPU only through its
  existing M3 timeouts.
- The report uses the shared serial lock. If an AP faults while holding
  that lock, no report is printed and other CPUs' serial output stalls. The
  BSP path from ADR 0047 has the same limit.
- NMI and #MC still use the current stack. Only #DF has an IST entry.
