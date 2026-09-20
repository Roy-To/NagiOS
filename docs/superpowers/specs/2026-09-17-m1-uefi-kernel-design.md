# M1 UEFI -> Kernel Design

**Status:** Approved for implementation after M0 acceptance

## Goal

Boot a real Nagi kernel through UEFI on the QEMU x86-64 reference machine and
prove the loader-to-kernel handoff over the QEMU serial device.

## Architecture

The workspace adds three guest-facing components:

- `crates/nagi-bootinfo`: a small `no_std`, `repr(C)` handoff contract shared
  by the loader and kernel;
- `kernel`: an ELF64 `x86_64-unknown-nagi` binary linked at a fixed identity-
  mapped physical address;
- `loader`: a Rust UEFI application built for `x86_64-unknown-uefi`.

The loader reads `\\EFI\\NAGI\\KERNEL.ELF` through UEFI's Simple File System
protocol. It validates the ELF header and every `PT_LOAD` segment, allocates
the requested physical pages using UEFI Boot Services, copies file bytes, and
zeroes the remaining memory. M1 deliberately requires identity-mapped
`p_vaddr == p_paddr`; virtual memory policy belongs to M2.

Before `ExitBootServices`, the loader captures the GOP framebuffer and the
ACPI 2.0 (or legacy ACPI) RSDP pointer. The final UEFI memory map returned by
`ExitBootServices` is passed as a raw, stride-aware descriptor view in
`BootInfo`. All UEFI-owned protocol handles and the memory-map owner are
forgotten only after their raw data has been transferred to the kernel; no UEFI
call is made by the kernel.

The host-side `nagi image` command builds the kernel and UEFI loader and writes
an actual FAT12 ESP image containing `EFI/BOOT/BOOTX64.EFI` and
`EFI/NAGI/KERNEL.ELF`. `nagi run` boots that image with the installed OVMF
CODE/VARS pair, four vCPUs, 8 GiB RAM, q35, and a serial log. The kernel emits
`Nagi Kernel started` through the emulated COM1 port and exits QEMU through the
explicit `isa-debug-exit` test device; the serial string, not a host-generated
substitute, is the acceptance signal.

## Error handling and safety

Malformed ELF input, arithmetic overflow, unsupported program headers, failed
UEFI allocation, absent GOP/ACPI, or a missing image file must stop the boot
with a visible UEFI error status. No fallback host execution or fake serial
response is permitted. The kernel validates the `BootInfo` magic/version and
reports a distinct failure string before halting if the handoff is invalid.

## Testing and acceptance

Host tests cover ELF bounds/identity checks, BootInfo validation, FAT12 image
layout, and exact QEMU argument/serial acceptance helpers. The M1 acceptance
test builds an image, runs QEMU with OVMF, reads the serial log, and requires
the exact line `Nagi Kernel started`.

