# Portability matrix — audited baseline

Status is a static classification, **not** ARM64 runtime proof. All links below
are immutable baseline anchors. The complete lexical inventory, source hashes,
and BootInfo field references are in [source-inventory.json](evidence/source-inventory.json).
No match does not prove portability; indirect build/upstream assumptions remain
an explicit verification item.

| Subsystem | Classification | Evidence | Assumption / migration obligation |
|---|---|---|---|
| `kernel/src/lib.rs` | mixed | [kernel/src/lib.rs:6](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/lib.rs#L6) | Aggregates portable mechanisms and hardware; compiling this library is not ARM64 portability. |
| `kernel/src/main.rs` | arch-specific | [kernel/src/main.rs:148](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/main.rs#L148) | RCX entry, GDT/IDT/TSS/IST, x86 COM1 and panic/halt. Preserve early NMI/#DF/#MC coverage. |
| `kernel/src/interrupts.rs` | arch-specific | [kernel/src/interrupts.rs:10](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/interrupts.rs#L10) | IDT/GDT, APIC MMIO, MSRs, periodic APIC timer and x86 exception frames. |
| `kernel/src/cpu_tables.rs` | arch-specific | [kernel/src/cpu_tables.rs:1](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/cpu_tables.rs#L1) | x86 TSS descriptors and IST stacks; ARM vector stacks need separate failure guarantees. |
| `kernel/src/smp.rs` | arch-specific | [kernel/src/smp.rs:319](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/smp.rs#L319) | Exactly four CPUs; real-mode trampoline, INIT/SIPI, 32-bit CR3/entry/stack limits, per-CPU scheduler context. |
| `kernel/src/acpi.rs` | platform-specific | [kernel/src/acpi.rs:5](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/acpi.rs#L5) | ACPI RSDP/MADT and APIC hardware IDs; fixed-capacity topology and mapped-table validation. |
| `kernel/src/memory.rs` | mixed | [kernel/src/memory.rs:8](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/memory.rs#L8) | UEFI conventional-page allocator policy plus x86 PML4/huge pages/CR3/TLB instruction semantics. |
| `kernel/src/user_process.rs` | arch-specific | [kernel/src/user_process.rs:63](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/user_process.rs#L63) | User image/stack/TLS/mmap/surface virtual layout, NX/RW/user flags, CR3, ring-3 entry and FS/TLS. |
| `kernel/src/user_child.rs` | arch-specific | [kernel/src/user_child.rs:4](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/user_child.rs#L4) | Separate child page tables, bounded ELF/stack and process isolation; included inside user_process. |
| `kernel/src/syscall.rs` | arch-specific | [kernel/src/syscall.rs:315](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/syscall.rs#L315) | MSRs, SYSCALL/SYSRET, x86 register save/restore and trusted caller selection. |
| `kernel/src/user_elf.rs` | mixed | [kernel/src/user_elf.rs:81](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/user_elf.rs#L81) | ELF64 little endian machine 62, fixed user VA, W^X, TLS/entry/bounds checks. |
| `kernel/src/handles.rs` | portable | [kernel/src/handles.rs:8](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/handles.rs#L8) | ABI rights, process-local handles, generation/reuse and attenuation; no CPU instruction dependency. |
| `kernel/src/ipc.rs` | portable | [kernel/src/ipc.rs:574](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/ipc.rs#L574) | Bounded local channel payload/transfer/wait/event/timer semantics; caller identity stays kernel-origin. |
| `kernel/src/user_ipc.rs` | portable | [kernel/src/user_ipc.rs:26](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/user_ipc.rs#L26) | Per-process authority tables, caller ID and escrow; spinlock ordering still needs ARM SMP validation. |
| `kernel/src/vmo.rs` | mixed | [kernel/src/vmo.rs:6](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/vmo.rs#L6) | Portable bounded backing/rights policy, but hardcoded 48-bit user address limit and 4-KiB pages. |
| `kernel/src/process_exit.rs` | portable | [kernel/src/process_exit.rs:11](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/process_exit.rs#L11) | Monotonic process IDs and bounded exit/reap state; retain failure and generation behavior. |
| `kernel/src/scheduler.rs` | portable | [kernel/src/scheduler.rs:1](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/scheduler.rs#L1) | Bounded scheduler state machine; runtime context switch and hardware timer are elsewhere. |
| `kernel/src/m4.rs` | mixed | [kernel/src/m4.rs:1](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/m4.rs#L1) | Mechanism acceptance harness depends on memory/page allocation and the reference boot path. |
| `kernel/src/gpt.rs` | portable | [kernel/src/gpt.rs:40](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/gpt.rs#L40) | Bounded GPT parser/CRC and disk arithmetic; real block transport remains platform-bound. |
| `kernel/src/boot_control.rs` | platform-specific | [kernel/src/boot_control.rs:40](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/boot_control.rs#L40) | Trusted UEFI Runtime Services SetVariable callback; signed A/B readiness/staging cannot be removed. |
| `kernel/src/display.rs` | mixed | [kernel/src/display.rs:10](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/display.rs#L10) | Reference framebuffer and 320x200 ABI surface, direct mapping and shared pixels. |
| `kernel/src/virtio.rs` | platform-specific | [kernel/src/virtio.rs:6](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/virtio.rs#L6) | Legacy PCI port I/O block transport, PFN queue addressing, identity DMA and SeqCst fences. |
| `kernel/src/net.rs` | platform-specific | [kernel/src/net.rs:6](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/net.rs#L6) | Reference VirtIO NIC queues tied to PCI ports and coherent identity DMA. |
| `kernel/src/input.rs` | platform-specific | [kernel/src/input.rs:8](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/input.rs#L8) | Legacy/modern PCI VirtIO input, MMIO capability discovery and DMA queues; not general multitouch. |
| `kernel/src/audio.rs` | platform-specific | [kernel/src/audio.rs:6](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/audio.rs#L6) | VirtIO sound PCI discovery/transport and DMA; user audio policy remains outside kernel. |
| `kernel/src/random.rs` | platform-specific | [kernel/src/random.rs:9](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/src/random.rs#L9) | Real VirtIO RNG PCI I/O and bounded queue; do not substitute insecure deterministic entropy. |
| `loader/src/main.rs` | platform-specific | [loader/src/main.rs:31](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/loader/src/main.rs#L31) | UEFI GOP/ACPI required, sub-4-GiB init allocation, win64 entry and signed slot verification. |
| `loader/src/elf.rs` | mixed | [loader/src/elf.rs:8](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/loader/src/elf.rs#L8) | Machine 62, identity-mapped aligned load segments and executable entry checks. |
| `crates/nagi-bootinfo/src/lib.rs` | mixed | [crates/nagi-bootinfo/src/lib.rs:6](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/crates/nagi-bootinfo/src/lib.rs#L6) | v5 C-layout (136 bytes), ACPI/framebuffer mandatory and trusted A/B callback; not packed. |
| `crates/nagi-abi/src/lib.rs` | mixed | [crates/nagi-abi/src/lib.rs:3](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/crates/nagi-abi/src/lib.rs#L3) | Stable syscall numbers and layouts, rights plus reference display/thread capacities. |
| `user/libnagi/src/lib.rs` | arch-specific | [user/libnagi/src/lib.rs:128](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/user/libnagi/src/lib.rs#L128) | x86 syscall trampolines/register/clobber ABI; meaning and rights must remain identical. |
| `targets/x86_64-unknown-nagi.json` | arch-specific | [targets/x86_64-unknown-nagi.json:2](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/targets/x86_64-unknown-nagi.json#L2) | x86 data layout, 64-bit little endian, no red zone, static kernel code model. |
| `targets/x86_64-unknown-nagi-user.json` | arch-specific | [targets/x86_64-unknown-nagi-user.json:2](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/targets/x86_64-unknown-nagi-user.json#L2) | x86 user target, Unix-family/PAL assumptions and large code model. |
| `kernel/linker.ld` | platform-specific | [kernel/linker.ld:11](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/kernel/linker.ld#L11) | Kernel identity VA/PA chosen for OVMF reservations; not a generic ARM load address. |
| `user/nagi-init/linker.ld` | arch-specific | [user/nagi-init/linker.ld:14](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/user/nagi-init/linker.ld#L14) | Fixed VA and ELF TLS segments; kernel/user linker and mmap contract migrate together. |
| `.cargo/config.toml` | arch-specific | [.cargo/config.toml:1](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/.cargo/config.toml#L1) | x86 target flags/link scripts and UEFI portable curve25519 backend selection. |
| `tools/nagi-target-cc.sh` | arch-specific | [tools/nagi-target-cc.sh:33](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/tools/nagi-target-cc.sh#L33) | Native C/PAL compile pipeline hardcodes x86 ELF ABI and resource headers. |
| `tools/nagi-cli/src/commands.rs` | platform-specific | [tools/nagi-cli/src/commands.rs:4847](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/tools/nagi-cli/src/commands.rs#L4847) | Builds x86 kernel/UEFI/user binaries and reference QEMU; test host gating is explicit. |
| `user/nagi-posix/src/abi.rs` | mixed | [user/nagi-posix/src/abi.rs:22](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/user/nagi-posix/src/abi.rs#L22) | User-space POSIX/PAL, TLS and libc bridge; depends on libnagi/thread ABI, not Linux kernel services. |
| `idl/application.nidl` | portable | [idl/application.nidl:7](https://github.com/RT-NISH/NagiOS/blob/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2/idl/application.nidl#L7) | Accepted adaptive application model already preserves compact/medium/expanded contexts. |

## Coverage and unknowns

Every production kernel module (including the nested child module), loader
entry/ELF path, ABI/BootInfo library and primary toolchain path is classified.
The inventory additionally scans all tracked user/crate/SDK/tool/test/patch
sources for CPU, firmware, transport, address and reference assumptions.
Device features are mixed: block/net/RNG retain legacy port transports, while
input/audio contain modern PCI MMIO support. None is an implemented ARM
VirtIO-MMIO platform. Rust atomic fences order memory, but do not clean caches,
translate DMA addresses, negotiate IOMMU mappings or prove non-coherent DMA.

Unknown/unverified: ARM upstream Servo/Mesa/relibc/std/llama.cpp/whisper native
builds, SIMD/code-generation and TLS compatibility; ARM cache maintenance and
SMP litmus behavior; GIC interrupts, PSCI CPU startup, MMU permissions/TLB
shootdown, physical drivers and power-state restoration. QEMU's coherent TCG
spike proves none of these. Existing source/model pins remain authoritative.

BootInfo v5 field coverage includes magic/version/size validation; memory-map
consumers in allocator and ACPI; framebuffer loader/display initialization;
ACPI topology and SMP identity checks; init-image user-process loading;
realtime epoch handoff; trusted boot-control writer and readiness/staging.
The inventory retains all field-reference lines, including fixtures/layout tests;
ordinary variables with the same name are candidate references, not all consumers.
The production layout is `repr(C)`, **not packed**; the attachment's warning
against in-place layout mutation applies without describing it as packed.
