# ADR 0051: Link-time exception tables from the first kernel instruction

Status: accepted
Date: 2026-10-03
Milestones: M1/M2 (kernel entry, exceptions)
Builds on: ADR 0050 (BSP tables from kernel entry)

## Context

ADR 0050 installed the BSP GDT, TSS and exception IDT right after
`serial_init`. Every instruction before that ran on the firmware GDT and
IDT: the function prologue, `serial_init`, and the Rust table construction
itself. An NMI, #MC or other exception in that window used firmware
handlers and could reset the guest.

Kernel code cannot build these tables before it runs. x86 descriptors split
addresses into 16/8/32-bit pieces that the assembler cannot relocate.

## Decision

1. **Assembly entry.** `_start` is now assembly in `.text.entry` and is
   still the ELF entry point. The loader contract is unchanged: win64 ABI,
   with BootInfo in RCX and the loader's stack. The first three
   instructions load static tables in order:
   1. `lgdt`;
   2. `ltr 0x28` (the TSS, which carries the IST stacks);
   3. `lidt`.

   `_start` then reloads CS and the data segments and jumps to the Rust
   entry `nagi_kernel_entry`. That function runs `serial_init` and then
   `install_early_bsp_tables` (ADR 0050).
2. **Link-time tables.** The tables live in `kernel/src/main.rs` and are
   fully built at link time:
   - **GDT.** It uses the BSP layout: kernel code at 0x08, kernel data at
     0x10, and the TSS at 0x28.
   - **TSS.** IST1–IST3 point at three dedicated 8 KiB early stacks, used
     for #DF, NMI and #MC.
   - **IDT.** It has 32 entries. Only NMI (IST2), #DF (IST1) and #MC (IST3)
     are present. Any other early exception hits a missing gate and
     escalates to #DF, which is reported.

   `kernel/linker.ld` computes the split address fields (the TSS
   descriptor base and the three gate offsets) as absolute symbols.
3. **Handlers.** The gates reuse `nagi_fault_stub_{2,8,18}` and
   `exception_entry`. `is_bsp()` does not touch the APIC before the first
   SIPI.

## Verification

- Kernel host tests: 154 pass.
- **Link-time layout.** A check of the linked ELF showed:
  - the TSS descriptor resolves to `nagi_early_tss` (limit 103, type 0x89);
  - IST1–IST3 point at the tops of the early stacks;
  - gates 2, 8 and 18 point at the matching stubs on selector 0x08 with
    IST 2, 1 and 3;
  - the other gates are empty;
  - the GDTR and IDTR limits are 55 and 511.
- **Diagnostic feature `entry-ist-probe`.** It parks the BSP with RSP=0
  before `install_early_bsp_tables`, so on the link-time tables only. On
  QEMU/OVMF, a monitor `nmi` and, in a separate boot, `mce 0 …` are
  reported as `vector=2` and `vector=18`. QEMU logged no triple fault.
- **Escalation check.** A scratch-only build ran `ud2` on the link-time
  tables. QEMU showed #UD, then #GP, then #GP, then #DF, which was reported
  as `vector=8` with no triple fault. That build is not committed.
- **Regressions.** The ADR 0050 BSP probe, the ADR 0049 AP NMI/#MC probe
  and the M2 self-test markers are unchanged. 24 of 24 default-kernel boots
  under 8-way QEMU contention reach `Nagi M3 acceptance PASS`.

## Bounds

- An NMI or #MC that arrives during `lgdt` or `ltr`, the two instructions
  before `lidt`, still uses the firmware IDT. Some instruction has to run
  first, so no kernel can make this window empty.
- Early exceptions other than NMI, #DF and #MC are reported only as #DF.
  The original vector is lost until the full tables are installed a few
  microseconds later.
