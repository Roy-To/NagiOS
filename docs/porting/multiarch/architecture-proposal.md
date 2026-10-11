# ADR proposal: shared mechanisms, CPU architecture and platform boundaries

Status: **Proposed, not Accepted**. Scope: PORT-MULTIARCH-01 A/B only.
No production interface, layout, syscall number or accepted decision changes.
Requires os-core/Integration Owner and affected ABI/loader/platform owners.

## Decision proposed

Retain Nagi's capability-based hybrid kernel (ADR-0001). Separate mechanisms
by dependency direction, before choosing directory moves. Shared mechanism
code may consume validated CPU-independent descriptions; it may not know
APIC IDs, firmware pointers, page-table flag encodings or device port numbers.
CPU implementations may not grant authority or embed high-level service policy.
Platform implementations perform discovery and transport, not permission policy.

Use compile-time modules, concrete types and bounded storage. First introduce
small typed calls at existing hardware call sites; avoid runtime trait objects,
a universal HAL, wholesale file moves or a second policy/ID system. Keep
architecture context structures private; opaque address-space handles are
kernel-only identities, never pointers transferred to apps.

| Boundary | Proposed operations | Required obligations |
|---|---|---|
| CPU local | current logical CPU, interrupt mask save/restore, idle/wake | Nested mask restoration; bounded logical IDs; wake-before-idle race proof |
| Traps/clock | install early/full vectors, acknowledge IRQ, monotonic time, scheduler tick | Faults before full startup remain diagnosable; monotonic units/overflow and one timekeeper; do not equate counter access with timer interrupt support |
| Address space | validated page source, create space, map/unmap/protect, invalidate local/remote translations | W^X, user/kernel separation, guard pages, checked arithmetic, TLB completion before reuse; backend supplies translation granule/VA/PA widths |
| Context/syscalls | private saved context, resume kernel/user, normalize trap request | Kernel-origin caller, preserved registers/TLS, no privileged flags accepted from EL0, existing syscall meaning/error/rights unchanged |
| Topology | validated unique hardware IDs → dense logical IDs; start/online handshake | capacity bounds, missing BSP rejection, 1..N future policy; x86 official acceptance continues to require four |
| Early console | bounded write/panic with available platform transport | Works before allocator/normal IRQ setup; no infinite UART wait; panic cannot depend on a held ordinary lock |
| Discovery | typed firmware description + validated memory regions/device resources | ACPI and DT are distinct; untrusted lengths/pointers checked; firmware discovery grants no app authority |
| DMA/VirtIO | transport registers, queue selection/notify, allocation + sync-for-device/CPU | Device-visible addresses explicit; alignment/size/bounds; ordering and cache maintenance separate; legacy PCI path preserved |
| Presentation/power | optional boot display; idle and low-level wake/clock hooks | Headless first ARM boot; display/input/power/AI policy in user services; no implied suspend correctness |

## Boot and syscall contract migration

BootInfo v5 stays exactly as it is: C-layout size 136, version 5, mandatory
framebuffer and ACPI, current realtime and trusted signed-update context.
The experimental validator for a future v6 is a semantic fixture, without
`repr(C)`, an encoder, firmware parser or trusted pointer dereference. It cannot
be used as a production boot protocol. The spike does not consume BootInfo.

A future accepted v6+ must have an explicit architecture/platform discriminant,
size/version and reserved-field rules, separate typed ACPI/DT references,
optional display with stride/format/range validation, normalized memory ownership,
init image and signed-update/realtime contracts. Specify max table lengths and
how the loader reserves every referenced buffer. Version unknown, truncated
layout, overflow, wrong platform/firmware pair and invalid trusted callback must
fail closed. Choose dual-format parsing or a coordinated loader/kernel update
at the checkpoint, with old image/slot compatibility tests. Never put DT in
`acpi_rsdp`, relax v5 display checks, or discard authenticated A/B staging.

Published syscall numbers, errors, rights, object kinds and wire layouts stay
stable. x86 RAX/RDI/RSI/RDX/R10/R8/R9 and clobbers remain unchanged. Proposed
ARM64 convention: SVC #0, x8 number, x0..x5 arguments, x0 result; this remains
unaccepted until a precise trap frame, preserved registers, pointer-width/error
encoding, EL0 entry, TLS and consumer fixtures are agreed. Do not create a
competing canonical syscall enum in this experiment.

ELF dispatch must explicitly bind the requested architecture to machine 62/183,
ELF64 little endian and accepted executable type, while retaining all bounds,
load/entry/TLS/overlap/W^X checks. Accepting machine 183 alone is not a port.

## Security and device independence

Preserve process-local handle generations, attenuation, transfer escrow, bounded
IPC, kernel-origin caller IDs, zeroed VMO backing and mapping protection limits.
ARM page permissions must prove user/kernel isolation and executable/write
separation. Host mechanism tests do not prove these MMU properties. Device
resources remain kernel-controlled and narrowly delegated; apps and AI receive
no blanket MMIO/DMA access. AI, VFS/POSIX, network, browser, power policy and
sync remain user-space. Kernel channels remain local.

Keep ADR 0014's NodeId/AppId/AppSessionId/ExecutionInstanceId/SurfaceId distinct.
Compact/Medium/Expanded, DPI, orientation, safe areas, rotation, pointer/pen and
multitouch contact lifecycle belong to capability/presentation contracts. The
320x200 reference buffer is retained for existing acceptance, not generalized
into a mobile display law. Suspend/device resume ordering, clocks, thermal and
battery budgets need separate owner milestones. AI scheduling uses existing
policy/priority boundaries; no new privileged AI hook. Wi-Fi/modem/GPU/camera
and Pi 5 firmware/RP1 drivers are separate later hardware work.

## Alternatives and tradeoffs

Reject a Linux/Android replacement or host-service shim: neither preserves the
independent OS/security requirement. Reject a full kernel move before tests:
it creates review/conflict risk across the active 0.1 line. Reject treating
QEMU virt as Pi 5: discovery, firmware, interrupts and devices differ. Reject
copying the rights/IPC implementation into the experiment: tests include actual
production source instead. A compile-time boundary duplicates small CPU glue,
but makes authority and unsupported architecture paths explicit.

Migration steps, rollback and exact proposed edits are in
[acceptance-plan.md](acceptance-plan.md). This proposal has no automatic adoption.
