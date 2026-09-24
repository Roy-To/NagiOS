# M5 User Bootstrap Architecture Decision

Status: accepted for Nagi OS 0.1 M5

## Decision

M5 boots exactly one `nagi-init` ELF as a real x86-64 ring-3 process. The loader stores the ELF in UEFI `LOADER_DATA` pages and passes its physical address and size through the versioned `BootInfo` structure when M5 is packaged. The general version-2 boot contract permits an absent init image during pre-M5 compatibility, so M1-M4 memory and ACPI consumers use `BootInfo::validate()` without requiring one. M5 user-process entry must call `BootInfo::validate_for_user_bootstrap()`, which requires a nonzero address and size, before validating the ELF and copying it into a separate user address space.

`InitImageInfo` is appended to the C-compatible `BootInfo` layout. `BootInfo::new()` initializes it to `{ address: 0, size: 0 }`; this represents an absent image and is not a valid M5 image. No kernel path may trust the bytes until the later M5 ELF validation step succeeds.

The M5-only bootstrap address range is:

- image base: `0x0000_4000_0000_0000`;
- image limit: image base plus `8 * 4096` bytes;
- stack page: image base plus `0x0020_0000`, with the initial stack pointer eight bytes below its upper edge so the SysV user entry convention presents `RSP % 16 == 8`;
- Static TLS data page: image base plus `0x0040_0000`, followed by an FS-base
  control page. FS points to the second page so x86-64 variant-II TLS data is
  addressed backward from the thread pointer. The fixed image supports one
  bounded `PT_TLS` template; M17 reserves a second isolated page pair for the
  one native child thread. See [ADR 0021](0021-nagi-static-elf-tls.md).

The kernel creates a new PML4, retains the current kernel mappings, and adds user mappings at PML4 index 128. Image, stack, and TLS backing pages are kernel-owned aligned bootstrap storage. M6 may replace this bounded storage with a general process/address-space allocator; M5 does not claim to implement that allocator.

## Syscall boundary

M5 publishes only two native syscall numbers:

- `1`: bounded console write, at most 256 bytes, with a validated user-readable range and a kernel-side copy before serial output;
- `2`: process exit, which records the M5 acceptance markers and halts the bootstrap guest.

These are diagnostic/bootstrap primitives, not file, socket, window, audio, package, AI, Linux, or POSIX kernel APIs. Unknown numbers have no side effects and return an error value.

SYSCALL/SYSRET uses kernel selectors `0x08/0x10` and user selectors `0x20/0x18` through a dedicated GDT and STAR/LSTAR/FMASK MSRs. M5 enters the first process with interrupts cleared because the existing kernel has no TSS-backed ring transition stack yet; M6 is responsible for integrating user-thread scheduling and safe interrupt entry.

The ring-3 transition establishes a deterministic floating-point boundary:
x87 state is initialized, MXCSR is set to `0x1f80`, and XMM0-XMM15 are zeroed
before `IRETQ`. A syscall entry saves the complete user FPU/SIMD image with
`FXSAVE64`, runs the kernel dispatcher with a sanitized state, and restores
the user image with `FXRSTOR64` before `SYSRETQ`. The M5 init process checks
the initial state and the state after a real console syscall round trip.

## Security and reproducibility

The loader and kernel reject empty, oversized, malformed, out-of-range, overlapping, W+X, or non-executable-entry user ELF segments. The user process cannot choose a CR3, GDT, MSR, kernel pointer, or arbitrary syscall. No host process, host filesystem, or host serial output is used to satisfy the acceptance test.
