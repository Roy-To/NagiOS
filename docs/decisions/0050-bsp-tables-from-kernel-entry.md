# ADR 0050: BSP GDT, TSS and exception IDT from kernel entry

Status: accepted
Date: 2026-10-03
Milestones: M2/M3 (exceptions, SMP), M5 (user process)
Builds on: ADR 0047 (BSP TSS), ADR 0048 (per-AP TSS) and ADR 0049 (NMI/#MC IST)

## Context

Until the M5 GDT switch, the BSP ran on the firmware GDT (selectors
0x38/0x30) with no TSS. Before `interrupts::initialize` it used the firmware
IDT. After that it used a kernel IDT holding only the timer and M2 #PF
gates. Any of the following on the BSP during kernel entry through M4 reset
the guest without a diagnostic:

- an NMI or #MC;
- a #DF;
- any exception other than the M2 test page fault.

## Decision

1. **Early install.** `_start` calls `interrupts::install_early_bsp_tables`
   right after `serial_init`, before BootInfo validation. It installs the
   same BSP GDT, TSS and full exception IDT that M5 used:
   - selectors 0x08, 0x10 and TSS 0x28;
   - RSP0;
   - the #DF, NMI and #MC IST stacks.
2. **M2 #PF overlay.** `interrupts::initialize` routes vector 14 of the BSP
   IDT to the M2 expected-page-fault stub. M5's `install_syscall_gdt`
   rebuilds the full exception table. The kernel's separate shared M3 IDT
   is removed.
3. **M5 reinstall.** `install_syscall_gdt` still runs at M5. It rewrites the
   TSS descriptor as available before `ltr`, so loading it a second time
   is valid.
4. **Exception reports before SMP.** `is_bsp()` returns true until the first
   SIPI without reading the local APIC. An exception before the APIC is
   mapped therefore cannot fault again inside the report path.
5. **M3 selectors.** All M3 task frames use the kernel selectors, because
   every CPU now runs on a kernel GDT during M3. The firmware-selector
   readers `current_code_selector` and `current_stack_selector` are
   removed.
6. **AP IDT mapping check.** `idt_base()` now returns the AP IDT, which is
   the table the APs load. The M3 address-space check therefore verifies
   that the right table is identity mapped.

## Verification

- Kernel host tests: 154 pass.
- **Diagnostic feature `m2-bsp-ist-probe`.** The BSP disables interrupts and
  spins with RSP=0 right after `Nagi Kernel started`. On QEMU/OVMF:
  - a monitor `nmi` is reported as `Nagi kernel exception vector=2`;
  - a separate boot with `mce 0 1 0xb200000000000000 0x5 0 0` is reported
    as `vector=18`.

  QEMU logged no triple fault in either boot.
- **M5 reinstall check.** A scratch-only build called `install_syscall_gdt`
  a second time right after kernel entry, mirroring M5. It still reached
  `Nagi M3 acceptance PASS`. This shows that re-`ltr` after the descriptor
  rewrite is valid. That build is not committed.
- **M2 self-test unchanged.** The default kernel still reports
  `Nagi Page fault handled (vector 14)`, `Nagi M2 acceptance PASS` and
  `Nagi Page fault resume PASS`.
- **No AP regressions.** The ADR 0048 #DF probe and the ADR 0049 NMI/#MC
  probe still report with no triple fault.
- **Stress run.** 24 of 24 default-kernel boots under 8-way QEMU contention
  reach `Nagi M3 acceptance PASS`.

## Bounds

- The first few instructions of `_start`, before the install, still run on
  the firmware tables.
- In the kernel-only test image M7 fails before M5, so the real M5
  reinstall was not run end to end. The scratch reinstall check above
  covers that path.
