# Nagi OS — Long-Term Multi-Device Outlook

**Status:** Forward-looking proposal; not an active milestone or an acceptance claim  
**Recorded:** 2026-10-11  
**Scope:** Architecture direction and candidate hardware only; no release date or device support is promised.

This document records a user-selected direction without changing Nagi OS 0.1, 0.2, or 0.3 acceptance criteria. For actual progress, see [implementation status](../implementation_status.md) and the [0.1 roadmap](../../ROADMAP.md). For the accepted cross-device architecture, see [ADR 0014](../decisions/0014-unified-device-application-model.md).

## 1. Decision summary / 方針

- **PC版が最優先。** Complete and stabilize the independent x86-64/QEMU Nagi PC experience first.
- **ARM64は並行して準備。** Architecture analysis and an isolated ARM64/QEMU proof of concept may progress in parallel where existing ownership boundaries, activation gates, and integration approvals permit.
- **Raspberry Pi 5** is a desirable first physical ARM64 target after the ARM64 virtual-machine foundations are validated. Hardware purchase or delivery dates are not assumed.
- **Nagi Phoneの第一候補はGoogle Pixel 9a。** The project owner already owns one, so no new phone is required for planning. Actual phone porting and flashing are deferred until the PC version is sufficiently mature.
- PC, Raspberry Pi, tablet, and phone are envisioned as **one Nagi OS product and application model**, not unrelated desktop/mobile editions.

This is a future direction, not an instruction to start mobile hardware development immediately.

## 2. Sequencing and dependency boundaries

| Stage | Direction | Priority / gate | Acceptance posture |
| --- | --- | --- | --- |
| PC foundation | Nagi 0.1 independent x86-64 kernel, core apps, Servo/Albert, local AI, release acceptance | **Current top priority** | Only actual milestone evidence can establish PASS. |
| PC evolution | 0.2 OS services and 0.3 profile, model-independent memory, learning, and migration architecture | After applicable workstream gates | No mobile port is implied by these versions. |
| ARM64 virtual foundation | Separate architecture boundaries, boot contract, ARM64 kernel/user ABI, QEMU aarch64/virt experiments | Parallel only inside authorized paths | QEMU ARM64 proof of concept is **not** phone compatibility. |
| Physical ARM64 | Raspberry Pi 5 boot and basic device services, subject to feasibility | After sufficient ARM64 virtual proof; optional parallel follow-up | Physical-board tests are required before support claims. |
| Nagi Phone | Pixel 9a feasibility review, then independent-kernel boot, display/touch, storage, power and radios as separate steps | **Deferred until the PC version is stable** | No Pixel 9a support is currently claimed. |
| Other tablets and phones | Broader device-capability-driven adaptation and supported-device expansion | Later; device-specific assessment | No blanket Android-device compatibility promise. |

The ARM64 work does **not** bypass the active 0.1 milestones or the existing 0.2 integration/ownership rules. Kernel or shared ABI changes require the relevant owner checkpoint. A stand-alone proof of concept must not masquerade as a completed Nagi kernel port.

**Version policy:** Pixel 9a, Raspberry Pi, and mobile support are not mandatory conditions for Nagi OS 0.1, 0.2, 0.3 or a PC-first 1.0/OSS release unless a later explicit product decision changes the gates.

## 3. Pixel 9a target profile (tentative)

| Item | Planning baseline |
| --- | --- |
| First phone target | Google Pixel 9a (specific user-owned unit; bootloader eligibility still to be checked) |
| SoC / ISA | Google Tensor G4, ARM64 |
| RAM | **8 GB** on the Pixel 9a; plan a memory-constrained local-AI profile rather than assuming desktop RAM |
| Storage | 128 GB or 256 GB variants; 128 GB as the initial planning floor |
| Display/input | Adaptive small-screen UI; touch, DPI, rotation, safe areas and software keyboard |
| AI | Model-independent user memory and profile; load/unload model runtimes as appropriate to memory/thermal/power limits |
| 12 GB+ devices | Optional future high-memory performance class; **not** required for the first Pixel 9a target |
| 4 GB devices | Exploratory lightweight tier only; **not** a promised full Nagi Phone requirement |

Google's published [Pixel 9a specifications](https://store.google.com/jp/product/pixel_9a_specs?hl=ja) state 8 GB RAM, Tensor G4 and 128/256 GB storage. These are hardware facts, not Nagi performance or driver-compatibility evidence.

A bootloader-unlockable unit is a **necessary but insufficient** condition for an independent Nagi kernel. Exact retail/carrier variant eligibility, verified boot, partition/boot-image format, hardware documentation, firmware distribution rights, GPU/display, touch, storage, power management, cellular modem, Wi-Fi, and camera support must be assessed separately. No working Nagi Pixel 9a port is assumed.

## 4. Safety and feasibility guardrails

1. **Do not unlock, flash, repartition, factory-reset, or otherwise modify the owner's Pixel 9a as part of planning.** Any such action needs a separate explicit decision, backup/recovery plan, understanding of data-loss risk, and a device-specific feasibility assessment. Google warns that bootloader unlocking can reduce security and installing factory images erases device data: [Google factory images](https://developers.google.com/android/images).
2. Nagi must remain an **independent kernel**. Android or Linux may be useful as documentation/development references, but booting Android/Linux underneath is not acceptance for Nagi Phone.
3. Vendor boot assets, binary firmware, and driver/source licensing require independent review before redistribution. Availability of a factory image does not authorize arbitrary redistribution.
4. Use staged evidence: architecture feasibility -> non-destructive hardware inventory -> ARM64 QEMU kernel boot -> target-specific boot feasibility -> real display/touch -> storage and lifecycle -> networking, power and thermal management -> security/recovery -> optional cellular and camera. Each stage requires observed evidence; do not mark downstream stages PASS early.
5. The existing x86-64/QEMU reference remains the regression baseline. Keep capability boundaries, local authorization, and AI isolation intact.

## 5. One app, one logical user environment

Follow [ADR 0014](../decisions/0014-unified-device-application-model.md) and the [unified device/application architecture](../architecture/unified-device-application-model.md):

- Preserve **AppId**, **AppSessionId**, **UserId**, **NodeId**, **ExecutionInstanceId**, and **SurfaceId** semantics across future machines; do not fork applications into unrelated desktop and mobile identities.
- Adapt presentation to device capabilities (including Compact/Medium/Expanded, touch, orientation, DPI and safe regions) without assuming that a desktop Window is the universal application root.
- Keep AI Profile/Memory outside a particular model runtime. Moving between PC and phone must not require treating LLM weights or a model KV cache as the user's memory.
- Cross-node synchronization, remote pairing, and shared identity are **future opt-in designs**, not features of the current single-node release. Device membership never confers implicit kernel authority.

## 6. Revisit triggers

Reassess the Pixel 9a plan when all of the following become relevant:

- The PC release line has reached an explicitly agreed stability checkpoint.
- ARM64 kernel/ABI and QEMU target evidence justify physical hardware work.
- The exact Pixel 9a unit's bootloader/boot-chain feasibility is documented.
- Required device documentation, firmware licensing and practical recovery paths are identified.
- Available capacity justifies the substantial independent-driver work.

Until then, preserve this as a **roadmap note only**. Do not silently register a mobile release milestone, lower PC acceptance standards, promise a schedule, or buy new hardware.
