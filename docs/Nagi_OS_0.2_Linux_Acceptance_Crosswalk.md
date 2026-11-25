# Linux 0.2 acceptance crosswalk

Status: REVIEW NOTE / PROVISIONAL. Companion to the [Linux 0.2 revision draft](Nagi_OS_0.2_Linux_Implementation_Spec_Draft.md). The original deliverables, acceptance, gates and ownership remain the baseline subject to the explicit supersession in DEC-123. This mapping gives reviewable Linux test boundaries and candidate dependencies.

Source: [existing 0.2 specification §32](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1190) and its [second copy](0.2/Nagi_OS_0.2_Codex_Implementation_Spec.md). At the pinned main SHA the only difference between the two files is the relative links in the opening migration note. Each `Mxx.nn` below identifies the corresponding original acceptance bullet in order; the baseline bullets are reproduced to make omissions detectable. Follow the pinned section link for Deliver, Gate, and Primary workstreams. The evidence and proposed Linux test are scoped to that group; no row grants a milestone PASS.

Runtime dependencies below describe candidate contract or acceptance edges, in addition to M00 wherever the existing runtime gate applies. See the [lane handoffs](Nagi_OS_0.2_Linux_Implementation_Spec_Draft.md#5-proposed-parallel-execution-lanes) and [registry snapshot](Nagi_OS_0.2_Linux_Implementation_Spec_Draft.md#registry-and-tooling). Read the owning state and exact branch SHA again before reusing it.

M08–M10 share one jointly versioned authority/ABI/IPC seam. Establish the rights contract first, then test the adapters together. The matrix does not require three independently completed milestones as mutual prerequisites.

| Legacy milestone | Candidate lane | Proposed contract or acceptance prerequisites |
|---|---|---|
| [M00 · Release-Line Activation and 0.1 Baseline Freeze](#02-m00--release-line-activation-and-01-baseline-freeze) | A | — |
| [M01 · Development Foundation](#02-m01--development-foundation) | A | — |
| [M02 · Build Provenance and Artifact Fingerprint](#02-m02--build-provenance-and-artifact-fingerprint) | A | M01 |
| [M03 · License / SBOM / Release Compliance Foundation](#02-m03--license--sbom--release-compliance-foundation) | A | M01; M02 (artifact inputs) |
| [M04 · Diagnostics / Observability Foundation](#02-m04--diagnostics--observability-foundation) | A | M01; M02 (fingerprint) |
| [M05 · System Integration Test Platform](#02-m05--system-integration-test-platform) | A | M01–M02 |
| [M06 · Localization / i18n Platform](#02-m06--localization--i18n-platform) | C | M01 |
| [M07 · UI Design System Foundation](#02-m07--ui-design-system-foundation) | C | M06 (locale); M09 (Linux surface) |
| [M08 · Identity, Session, Capability and Permission Integration](#02-m08--identity-session-capability-and-permission-integration) | B | M00; M09–M10 (joint seam) |
| [M09 · OS Core and Public ABI Stabilization](#02-m09--os-core-and-public-abi-stabilization) | B | M00; M08 (rights contract) |
| [M10 · System Service IPC Contract](#02-m10--system-service-ipc-contract) | B | M08–M09 (caller and ABI seam) |
| [M11 · Settings / Configuration Service](#02-m11--settings--configuration-service) | C | M06; M08; M10 |
| [M12 · Application Manifest / Package Lifecycle Contract](#02-m12--application-manifest--package-lifecycle-contract) | D | M08–M10 (identity/ABI); M02–M03 (provenance/legal) |
| [M13 · App Supervisor and Background Job Runtime](#02-m13--app-supervisor-and-background-job-runtime) | B | M08; M10; M12 |
| [M14 · Notification Service](#02-m14--notification-service) | C | M07–M08; M10–M11; M13 (lifecycle) |
| [M15 · Update / Installation Runtime](#02-m15--update--installation-runtime) | D | M02–M03; M08; M12–M13 |
| [M16 · Model Runtime / Model Store Integration](#02-m16--model-runtime--model-store-integration) | E | M08; M10; M13; M17 (action boundary) |
| [M17 · Platform Resource / Document / Object / Action Contracts](#02-m17--platform-resource--document--object--action-contracts) | F | M08; M10 |
| [M18 · Activity / Transaction / Revision / Checkpoint / Wayback Platform](#02-m18--activity--transaction--revision--checkpoint--wayback-platform) | F | M08; M10; M17 |
| [M19 · Search / Index / Workspace Platform](#02-m19--search--index--workspace-platform) | F | M08; M17–M18; M16 (semantic only) |
| [M20 · Context / App Session / Surface Foundations](#02-m20--context--app-session--surface-foundations) | F | M07–M08; M10; M17 |
| [M21 · SDK / App Ecosystem Contract](#02-m21--sdk--app-ecosystem-contract) | H | M08–M10; M12; M17–M20 (deeper SDK levels) |
| [M22 · First-Party Common Framework Integration](#02-m22--first-party-common-framework-integration) | G | M08; M17–M21 |
| [M23 · Files 0.2](#02-m23--files-02) | G | M08; M17–M20; M22 |
| [M24 · Notes 0.2](#02-m24--notes-02) | G | M08; M17–M20; M22 |
| [M25 · Activity + Wayback Applications](#02-m25--activity--wayback-applications) | G | M17–M18; M22–M24 |
| [M26 · Home + Search Applications](#02-m26--home--search-applications) | G | M12–M13; M17; M19; M22 |
| [M27 · Albert / Terminal / Core Experience Platform Adoption](#02-m27--albert--terminal--core-experience-platform-adoption) | G | M07–M10; M17–M22; M26 |
| [M28 · Full Platform Integration / Security / Stress / Accessibility](#02-m28--full-platform-integration--security--stress--accessibility) | A | Applicable M01–M27 target and cross-lane checks |
| [M29 · Release Hardening / Documentation / Reference Hardware Preparation](#02-m29--release-hardening--documentation--reference-hardware-preparation) | A | M02–M03; M21; M28 |
| [M30 · Nagi OS 0.2 Release](#02-m30--nagi-os-02-release) | A | All required M00–M29 gates |

## 0.2-M00 — Release-Line Activation and 0.1 Baseline Freeze

[Source §32 M00](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1198). Lane A. Dependencies: —.

- `M00.01` `./nagi dev verify` passes;
- `M00.02` 0.1 release artifacts and source revision are known;
- `M00.03` no uncommitted 0.1 acceptance weakening;
- `M00.04` 0.2 Integration Owner checkpoint explicitly authorizes runtime activation.

**Current evidence.** Registry state is historical (IN_PROGRESS); GitHub Releases was empty at review. [integration-next-phase/state.json](../.dev/workstreams/integration-next-phase/state.json) · [nagi-release/release.py](../tools/nagi-release/release.py)

**Linux acceptance work.** Record Linux 0.1 publication receipt with artifact/hash/source and immutable reference. Verify preserved regression suite, migration inventory, selected integration branch, clean registry and explicit 0.2 boundary/checkpoint. Apply DEC-123 to the old M30 input only.

## 0.2-M01 — Development Foundation

[Source §32 M01](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1231). Lane A. Dependencies: —.

- `M01.01` registry parses;
- `M01.02` all registered IDs/branches/dependencies are unique and valid;
- `M01.03` state files validate;
- `M01.04` status/resume works from repository state without chat history;
- `M01.05` malformed/unsafe records fail closed;
- `M01.06` another developer can resume an active workstream from checked-in state.

**Current evidence.** DF-01 foundation PASS in state; current main CI ran `./nagi dev verify`. [development-foundation/state.json](../.dev/workstreams/development-foundation/state.json) · [src/development.rs](../tools/nagi-cli/src/development.rs)

**Linux acceptance work.** Replay status/resume from a clean checkout, validate uniqueness/dependencies/state, reject malformed ownership and have a second developer resume from checked-in evidence.

## 0.2-M02 — Build Provenance and Artifact Fingerprint

[Source §32 M02](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1265). Lane A. Dependencies: M01.

- `M02.01` identical compatible inputs produce a deterministic match;
- `M02.02` source/toolchain/target/feature/artifact differences are classified;
- `M02.03` requested missing artifacts fail closed;
- `M02.04` secret-like values are redacted;
- `M02.05` shared CI recognizes the registered workstream;
- `M02.06` a release artifact can be traced back to exact compatible inputs.

**Current evidence.** Host checkpoint PASS. Older `build-provenance` state still says BLOCKED; the registration and host adoption are present, with no Linux release receipt. [development/fingerprint.rs](../tools/nagi-cli/src/development/fingerprint.rs) · [tools/provenance-sbom-contract](../tools/provenance-sbom-contract) · [provenance-sbom-integration/state.json](../.dev/workstreams/provenance-sbom-integration/state.json)

**Linux acceptance work.** Exercise deterministic comparison, classified mismatches, missing inputs and redaction in CI. Pin kernel/config, toolchain, lockfiles, rootfs, patches and image inputs, then trace a real release artifact to them.

## 0.2-M03 — License / SBOM / Release Compliance Foundation

[Source §32 M03](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1296). Lane A. Dependencies: M01; M02 (artifact inputs).

- `M03.01` all legal metadata files parse;
- `M03.02` deterministic SBOM fixture tests pass;
- `M03.03` repository inventory executes offline after sources are present;
- `M03.04` unknown/conflicting metadata remains explicit;
- `M03.05` generated output contains no host secrets/absolute private paths;
- `M03.06` release workflow can generate an SBOM/NOTICE candidate from an immutable source revision.

**Current evidence.** Host foundation PASS. [tools/legal](../tools/legal) · [license-sbom/state.json](../.dev/workstreams/license-sbom/state.json)

**Linux acceptance work.** Run offline inventory with sources present, deterministic fixtures and parser/redaction negatives. Include kernel, rootfs, apps and model terms and generate candidate SBOM/NOTICE from immutable release source with unknown/conflicting entries visible.

## 0.2-M04 — Diagnostics / Observability Foundation

[Source §32 M04](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1327). Lane A. Dependencies: M01; M02 (fingerprint).

- `M04.01` secrets are redacted in tests;
- `M04.02` sink failure does not crash the producer;
- `M04.03` bounded storage/rate behavior is deterministic;
- `M04.04` crash/health snapshot can be generated;
- `M04.05` diagnostic report identifies source/build fingerprint;
- `M04.06` debug events are not silently represented as user Activity.

**Current evidence.** Diagnostics PARTIAL, host/tooling state. [src/development.rs](../tools/nagi-cli/src/development.rs) · [diagnostics/state.json](../.dev/workstreams/diagnostics/state.json)

**Linux acceptance work.** Show fault injection, redaction, bounded sinks and crash/health snapshots on Linux with immutable build correlation. Keep diagnostic events separate from Activity.

## 0.2-M05 — System Integration Test Platform

[Source §32 M05](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1357). Lane A. Dependencies: M01–M02.

- `M05.01` host harness runs deterministically;
- `M05.02` failure injection is reproducible;
- `M05.03` result records source/artifact/run identity;
- `M05.04` target tests cannot be reported PASS from host fixtures;
- `M05.05` QEMU adapter can be added without changing product contracts;
- `M05.06` release runs are non-canceling and attached to immutable source/artifact identity.

**Current evidence.** TEST-PLAT-01 IN_PROGRESS; development runner/parser and bounded #163 guest receipts exist. [linux-boot/boot_acceptance.py](../tools/linux-boot/boot_acceptance.py) · [linux-boot/task24_interactive_acceptance.py](../tools/linux-boot/task24_interactive_acceptance.py) · [test-plat-01/state.json](../.dev/workstreams/test-plat-01/state.json)

**Linux acceptance work.** Keep harness deterministic and machine-readable. Adopt real Linux guest adapter, inject failures, bind runs to source/artifacts, distinguish fixture/compile/guest, and retain non-canceling immutable release runs.

## 0.2-M06 — Localization / i18n Platform

[Source §32 M06](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1387). Lane C. Dependencies: M01.

- `M06.01` selected locale resolves expected text;
- `M06.02` missing Japanese resource falls back to English rather than raw key/blank text;
- `M06.03` malformed catalogs fail validation;
- `M06.04` English and Japanese representative screens/contracts are testable;
- `M06.05` internal IDs remain locale-neutral.

**Current evidence.** Host foundation PASS; #243 has Settings-specific language integration. [crates/nagi-i18n](../crates/nagi-i18n) · [user/nagi-localization](../user/nagi-localization) · [localization-i18n/state.json](../.dev/workstreams/localization-i18n/state.json)

**Linux acceptance work.** Smoke-test real English/Japanese screens and cross-app Settings propagation, plus fallback, bad catalogs and invariant IDs.

## 0.2-M07 — UI Design System Foundation

[Source §32 M07](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1416). Lane C. Dependencies: M06 (locale); M09 (Linux surface).

- `M07.01` shared components render/behave consistently in reference tests;
- `M07.02` keyboard navigation and focus state are testable;
- `M07.03` English/Japanese text does not require app-specific layout hacks;
- `M07.04` accessibility metadata exists on representative controls;
- `M07.05` first-party apps can consume the system without private widget forks.

**Current evidence.** UI foundation PASS; #241 development-shell evidence on source lane. [user/nagi-ui](../user/nagi-ui) · [src/linux_task24_desktop_shell.rs](../user/nagi-init/src/linux_task24_desktop_shell.rs) · [ui-design-system/state.json](../.dev/workstreams/ui-design-system/state.json)

**Linux acceptance work.** Exercise shared components/focus/a11y and both locales on real shell plus app clients, comparing fallback/rendering without private widget forks.

## 0.2-M08 — Identity, Session, Capability and Permission Integration

[Source §32 M08](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1445). Lane B. Dependencies: M00; M09–M10 (joint seam).

- `M08.01` cross-profile reads fail;
- `M08.02` expired/invalid session authority fails;
- `M08.03` capability denial is deterministic;
- `M08.04` delegated authority cannot be strengthened;
- `M08.05` guest/ephemeral cleanup semantics are testable;
- `M08.06` AI/background work cannot self-elevate;
- `M08.07` representative service adapters bind requests to authenticated caller/session context.

**Current evidence.** Capability state PARTIAL; #163 and #242 cover limited dev owner/session/storage paths. [crates/nagi-identity-session](../crates/nagi-identity-session) · [crates/nagi-c1-session-runtime](../crates/nagi-c1-session-runtime) · [crates/nagi-capability](../crates/nagi-capability) · [crates/nagi-peer-authenticated-connection](../crates/nagi-peer-authenticated-connection)

**Linux acceptance work.** Real service requests bind principal and session; run two-profile, invalid/expired/revoked, attenuation, guest cleanup, AI/job denial and cross-app Linux enforcement tests.

## 0.2-M09 — OS Core and Public ABI Stabilization

[Source §32 M09](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1478). Lane B. Dependencies: M00; M08 (rights contract).

- `M09.01` 0.1 core behavior still passes regression;
- `M09.02` published ABI changes are versioned;
- `M09.03` rights attenuation remains release-blocking;
- `M09.04` no high-level product policy migrates into the kernel;
- `M09.05` x86-64 QEMU reference target remains stable.

**Current evidence.** Linux boot/spawn path exists; no `os-core` state file on main and legacy guest evidence is historical. [crates/nagi-abi](../crates/nagi-abi) · [crates/nagi-init-spawn-contract](../crates/nagi-init-spawn-contract) · [src/linux_init.rs](../user/nagi-init/src/linux_init.rs)

**Linux acceptance work.** Publish ABI/IDL diffs, run actual Linux rights-attenuation and boot/regression suites against frozen 0.1. Keep product policy in user space.

## 0.2-M10 — System Service IPC Contract

[Source §32 M10](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1504). Lane B. Dependencies: M08–M09 (caller and ABI seam).

- `M10.01` a reference service is discoverable and callable;
- `M10.02` incompatible versions fail explicitly;
- `M10.03` malformed/oversized requests fail closed;
- `M10.04` caller identity cannot be forged through ordinary payload fields;
- `M10.05` cancellation/failure results remain correlated;
- `M10.06` host contract tests and Nagi target service call both pass.

**Current evidence.** Task24 scoped endpoint and READY are evidence for one development path. [crates/nagi-nipc-v1-stream](../crates/nagi-nipc-v1-stream) · [crates/nagi-init-service-call](../crates/nagi-init-service-call) · [crates/nagi-service-contract](../crates/nagi-service-contract)

**Linux acceptance work.** Test published discovery/call on Linux, bad version/size/shape, caller spoof rejection and correlated cancel/failure; retain host contract tests.

## 0.2-M11 — Settings / Configuration Service

[Source §32 M11](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1535). Lane C. Dependencies: M06; M08; M10.

- `M11.01` invalid values are rejected;
- `M11.02` unauthorized scope writes fail;
- `M11.03` profile isolation passes;
- `M11.04` atomic write/recovery tests pass;
- `M11.05` subscribers receive versioned changes without duplicate ambiguity;
- `M11.06` language/notification/model representative settings use the common service.

**Current evidence.** Settings L9 (#243) provides a bounded caller, with no common 0.2 service acceptance. [linux_task24_desktop_shell/settings.rs](../user/nagi-init/src/linux_task24_desktop_shell/settings.rs)

**Linux acceptance work.** Wire common typed scopes to language, notifications and model consumers. Test invalid and cross-profile writes, atomic recovery, versioned subscription and restart.

## 0.2-M12 — Application Manifest / Package Lifecycle Contract

[Source §32 M12](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1567). Lane D. Dependencies: M08–M10 (identity/ABI); M02–M03 (provenance/legal).

- `M12.01` valid fixture parses and validates;
- `M12.02` malformed IDs/versions/capabilities fail;
- `M12.03` duplicate/conflicting declarations fail deterministically;
- `M12.04` manifest can be consumed by SDK, installer, Home/App Registry and Supervisor without private copies;
- `M12.05` existing 0.1 package format is migrated rather than silently forked.

**Current evidence.** App SDK contract foundation PASS; system slot manifest and app/package validation serve distinct layers. [crates/nagi-slot-manifest](../crates/nagi-slot-manifest) · [tools/nagi-pkg](../tools/nagi-pkg) · [user/nagi-package](../user/nagi-package) · [app-sdk-contract/state.json](../.dev/workstreams/app-sdk-contract/state.json)

**Linux acceptance work.** Agree one public manifest with installer/SDK/Home/Supervisor and a versioned 0.1 migration. Test ID/version/capability and duplicate/conflict negatives, then validate real consumers against one schema.

## 0.2-M13 — App Supervisor and Background Job Runtime

[Source §32 M13](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1599). Lane B. Dependencies: M08; M10; M12.

- `M13.01` app start/ready/stop/crash paths are observable;
- `M13.02` repeated crashing does not loop forever;
- `M13.03` service dependency failures are explicit;
- `M13.04` job retry/backoff is deterministic with virtual clock tests;
- `M13.05` cancellation is cooperative and observable;
- `M13.06` background jobs cannot bypass capabilities/session ownership;
- `M13.07` target runtime launches a reference managed app and background job.

**Current evidence.** Scheduler/launch code and limited Identity dev service lifecycle exist. [crates/nagi-job-scheduler](../crates/nagi-job-scheduler) · [crates/nagi-system-service-launch](../crates/nagi-system-service-launch) · [src/linux_task24_endpoint_launch.rs](../user/nagi-init/src/linux_task24_endpoint_launch.rs)

**Linux acceptance work.** Exercise actual managed user app and scoped background job, crash backoff and dependency failure, observation, virtual-time retry, cancel and restart without privilege escalation.

## 0.2-M14 — Notification Service

[Source §32 M14](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1635). Lane C. Dependencies: M07–M08; M10–M11; M13 (lifecycle).

- `M14.01` host foundation tests remain PASS;
- `M14.02` target service publish/query/read/dismiss works;
- `M14.03` cross-profile isolation passes;
- `M14.04` sensitive content is redacted at storage/query/export boundaries;
- `M14.05` notification action revalidates current authority;
- `M14.06` quiet/focus policy behaves deterministically;
- `M14.07` notification service survives restart with durable state;
- `M14.08` representative first-party notification can be displayed through the UI adapter.

**Current evidence.** Host notification store/tests present. [crates/nagi-notification-core](../crates/nagi-notification-core) · [tests/notification-core](../tests/notification-core)

**Linux acceptance work.** Prove durable publish/query/read/dismiss after restart in Linux, profile isolation, redact at storage/query/export and fresh authorization on actions. Connect quiet policy and real UI.

## 0.2-M15 — Update / Installation Runtime

[Source §32 M15](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1669). Lane D. Dependencies: M02–M03; M08; M12–M13.

- `M15.01` existing 24+ host foundation tests remain PASS;
- `M15.02` target/reference package installs and launches;
- `M15.03` interrupted install recovers;
- `M15.04` failed switch rolls back;
- `M15.05` unsafe paths/symlinks are rejected;
- `M15.06` user data survives app/system update where contract requires;
- `M15.07` active inventory reflects durable installed state;
- `M15.08` package provenance can be reported.

**Current evidence.** Installer and recovery contracts exist. Legacy installer/update tests and development Data recovery cover only their own scopes. [tools/nagi-installer](../tools/nagi-installer) · [user/nagi-init-package-provider](../user/nagi-init-package-provider) · [crates/nagi-persistence-contract](../crates/nagi-persistence-contract) · [crates/nagi-system-service-recovery-contract](../crates/nagi-system-service-recovery-contract)

**Linux acceptance work.** Keep old host tests; install/launch real reference package, inject interruption and failed switch, verify inventory/provenance, safe paths and Data preservation. Treat system A/B and app package rollback as separate contracts.

## 0.2-M16 — Model Runtime / Model Store Integration

[Source §32 M16](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1705). Lane E. Dependencies: M08; M10; M13; M17 (action boundary).

- `M16.01` invalid/digest-mismatched artifact fails;
- `M16.02` model registry survives restart;
- `M16.03` provider can load/unload under policy;
- `M16.04` local Granite produces a real target response;
- `M16.05` cancellation/timeouts are observable;
- `M16.06` model runtime cannot invoke privileged actions directly;
- `M16.07` OS remains usable when model runtime is absent or unloaded.

**Current evidence.** `model-runtime` PARTIAL on the predecessor path; no real Granite inference on current Linux release image evidenced. [user/nagi-model-manager](../user/nagi-model-manager) · [crates/nagi-model](../crates/nagi-model) · [services/nagi-ai](../services/nagi-ai) · [model-runtime/state.json](../.dev/workstreams/model-runtime/state.json)

**Linux acceptance work.** Wire digest-verified data/store/registry and real session-scoped Granite target call. Prove timeout/cancel/load/unload, restart, resource quotas, no direct privilege and AI-off fallback.

## 0.2-M17 — Platform Resource / Document / Object / Action Contracts

[Source §32 M17](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1738). Lane F. Dependencies: M08; M10.

- `M17.01` identity survives representative rename/move/reopen scenarios where provider supports it;
- `M17.02` Action schema rejects malformed inputs;
- `M17.03` unknown risk is not treated as safe;
- `M17.04` public contract has no first-party-only secret APIs;
- `M17.05` representative Files/Notes operations can be expressed through the shared model.

**Current evidence.** Provider/domain and typed action seams present; no whole public model target acceptance. [crates/nagi-action-ipc](../crates/nagi-action-ipc) · [apps/nagi-files](../apps/nagi-files) · [user/nagi-notes](../user/nagi-notes) · [idl](../idl)

**Linux acceptance work.** Freeze versioned stable IDs with both providers. Roundtrip rename/move/reopen, reject malformed/unknown-risk Actions, and verify public-only Files/Notes consumption.

## 0.2-M18 — Activity / Transaction / Revision / Checkpoint / Wayback Platform

[Source §32 M18](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1770). Lane F. Dependencies: M08; M10; M17.

- `M18.01` USER and AGENT actions are distinguishable;
- `M18.02` failed/partial work is not marked success;
- `M18.03` target Activity survives restart;
- `M18.04` secret payloads are not copied by default;
- `M18.05` supported document/resource can checkpoint and restore;
- `M18.06` in-place restore preserves a recovery point;
- `M18.07` restore itself records Activity;
- `M18.08` irreversible external effects do not expose fake Undo.

**Current evidence.** Wayback/Activity foundation PASS; runtime ownership still gated. [crates/nagi-wayback](../crates/nagi-wayback) · [user/nagi-history](../user/nagi-history) · [wayback-activity-ledger/state.json](../.dev/workstreams/wayback-activity-ledger/state.json)

**Linux acceptance work.** Run durable Activity and checkpoints through real providers, preserve restore recovery point, record actor/result and restore Activity, test redaction and absence of Undo for irreversible effects.

## 0.2-M19 — Search / Index / Workspace Platform

[Source §32 M19](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1805). Lane F. Dependencies: M08; M17–M18; M16 (semantic only).

- `M19.01` exact filename/title queries work without AI;
- `M19.02` unauthorized resources do not appear or leak existence;
- `M19.03` deletion/update propagates incrementally;
- `M19.04` Activity/temporal query works;
- `M19.05` Workspace can reference resources without physically moving them;
- `M19.06` semantic provider can be disabled without breaking basic search;
- `M19.07` target persistence survives restart.

**Current evidence.** SearchCore PARTIAL host; Calendar provider adapter has scoped host CI. [crates/nagi-search-core](../crates/nagi-search-core) · [crates/nagi-search-ipc](../crates/nagi-search-ipc) · [user/nagi-search](../user/nagi-search) · [search-core-01/state.json](../.dev/workstreams/search-core-01/state.json)

**Linux acceptance work.** Connect permission-filtered Linux providers/index and Workspace references; test no existence leak, incremental update/delete, temporal and lexical queries, restart and semantic-off.

## 0.2-M20 — Context / App Session / Surface Foundations

[Source §32 M20](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1838). Lane F. Dependencies: M07–M08; M10; M17.

- `M20.01` app publishes explicit context;
- `M20.02` core does not inspect arbitrary app memory;
- `M20.03` process restart does not force logical session identity loss where supported;
- `M20.04` context access is permission/capability filtered;
- `M20.05` desktop surface works without hard-coding future device classes into public contracts.

**Current evidence.** App shell / browser state and service seams are inputs; unified Context/Session/Surface target contract not evidenced. [crates/nagi-service-contract](../crates/nagi-service-contract) · [user/nagi-albert](../user/nagi-albert) · [src/linux_task24_desktop_shell.rs](../user/nagi-init/src/linux_task24_desktop_shell.rs)

**Linux acceptance work.** Run explicit bounded publication through app consumers, grant/revoke and restart logical sessions, with no app memory reads. Exercise desktop surface via device-neutral contract.

## 0.2-M21 — SDK / App Ecosystem Contract

[Source §32 M21](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1868). Lane H. Dependencies: M08–M10; M12; M17–M20 (deeper SDK levels).

- `M21.01` out-of-tree sample builds using published SDK only;
- `M21.02` package validates/installs/launches;
- `M21.03` sample app receives only declared authority;
- `M21.04` SDK API versions are explicit;
- `M21.05` generated bindings roundtrip contract fixtures;
- `M21.06` sample can opt into Level 0 then deeper integration without private first-party APIs.

**Current evidence.** Existing SDK contract foundation PASS; #246 host-only and dedicated CI red. [sdk/rust](../sdk/rust) · [sdk/c](../sdk/c) · [tools/nagi-idl](../tools/nagi-idl) · [app-sdk-contract/state.json](../.dev/workstreams/app-sdk-contract/state.json)

**Linux acceptance work.** Retain Rust SDK/C ABI and levels 0–3. Generate and roundtrip bindings; from a clean external tree validate/install/launch on Linux with declared rights only and versioned opt-in to deeper APIs.

## 0.2-M22 — First-Party Common Framework Integration

[Source §32 M22](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1899). Lane G. Dependencies: M08; M17–M21.

- `M22.01` Files/Notes/Home/Search/Activity/Wayback use common contracts rather than duplicated local types where public contracts exist;
- `M22.02` host reference tests remain PASS;
- `M22.03` target adapters are real, not fixtures;
- `M22.04` cross-app test proves Resource/Object/Action/Activity identity consistency;
- `M22.05` no first-party app gets undeclared hidden authority.

**Current evidence.** Host first-party integration PARTIAL; registry row is for dedicated 0.1 integration. [tests/first-party-integration](../tests/first-party-integration) · [first-party-integration/state.json](../.dev/workstreams/first-party-integration/state.json)

**Linux acceptance work.** Preserve host fixtures, replace them with real Linux adapters for the acceptance run, and prove one cross-app identity/Action/Activity roundtrip without private authority.

## 0.2-M23 — Files 0.2

[Source §32 M23](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1933). Lane G. Dependencies: M08; M17–M20; M22.

- `M23.01` create/copy/move/rename/duplicate/trash/restore/permanent-delete work on target;
- `M23.02` destructive permanent delete requires correct confirmation/policy;
- `M23.03` same-resource move preserves identity where the provider supports it;
- `M23.04` selected-resource Context publishes correctly;
- `M23.05` Workspace add/remove does not move the file;
- `M23.06` Search returns authorized resources;
- `M23.07` agent/app mutation records Activity;
- `M23.08` supported mutations create truthful checkpoint/restore behavior.

**Current evidence.** Host durability #238/#240; #245 scoped dev caller still draft and red (E5). [apps/nagi-files](../apps/nagi-files) · [tools/nagi-files-c1-session-consumer](../tools/nagi-files-c1-session-consumer)

**Linux acceptance work.** Run all target mutations incl. duplicate/permanent delete under policy, same identity, real Context, Workspace and Search, and Activity/checkpoint restore. Exercise wrong-profile/denied/revoked and recovery cases.

## 0.2-M24 — Notes 0.2

[Source §32 M24](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L1969). Lane G. Dependencies: M08; M17–M20; M22.

- `M24.01` create/edit/save/close/reopen persists;
- `M24.02` autosave failure retains dirty state and does not claim success;
- `M24.03` revisions remain immutable/ordered;
- `M24.04` restore/restore-as-copy works where supported;
- `M24.05` Search finds note metadata/content according to policy;
- `M24.06` Activity contains meaningful edit groups rather than every keystroke;
- `M24.07` a typed reference from another app can roundtrip without flattening identity.

**Current evidence.** Task24 real Notes save/readback on fixed development image (E1). [user/nagi-notes](../user/nagi-notes) · [src/linux_task24_notes_console.rs](../user/nagi-init/src/linux_task24_notes_console.rs)

**Linux acceptance work.** Repeat on target plus autosave fault, immutable revisions, restore/as-copy, searchable authorized content, grouped Activity and cross-app typed-reference roundtrip.

## 0.2-M25 — Activity + Wayback Applications

[Source §32 M25](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L2004). Lane G. Dependencies: M17–M18; M22–M24.

- `M25.01` USER/AGENT/AUTOMATION/SYSTEM are visibly distinguishable;
- `M25.02` failed/partial transaction is truthful;
- `M25.03` target resource opens from Activity;
- `M25.04` reversible event routes to supported undo/restore;
- `M25.05` irreversible event does not show fake Undo;
- `M25.06` Wayback can restore a supported document;
- `M25.07` restore-as-copy succeeds;
- `M25.08` pinned checkpoint survives cleanup;
- `M25.09` secret content is not indiscriminately rendered from Activity metadata.

**Current evidence.** Host ledger/reference coverage; no real target app acceptance established. [crates/nagi-wayback](../crates/nagi-wayback) · [tests/first-party-integration](../tests/first-party-integration)

**Linux acceptance work.** Drive actual Activity→resource→restore/as-copy, assert visible actors, accurate failure, no false Undo, pinned checkpoint retention and redaction.

## 0.2-M26 — Home + Search Applications

[Source §32 M26](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L2042). Lane G. Dependencies: M12–M13; M17; M19; M22.

- `M26.01` installed/available apps are listed from the real registry;
- `M26.02` unlaunchable package is not falsely shown as successfully launchable;
- `M26.03` Search queries real providers;
- `M26.04` unauthorized private result is filtered;
- `M26.05` stale/cancelled query does not overwrite newer results;
- `M26.06` lexical search works with AI disabled;
- `M26.07` result actions use typed platform Actions.

**Current evidence.** 0.1 launcher and host registry/provider foundations cover limited paths. [user/nagi-home-search](../user/nagi-home-search) · [src/linux_task24_desktop_shell.rs](../user/nagi-init/src/linux_task24_desktop_shell.rs)

**Linux acceptance work.** Prove real app availability/install/launch state and lexical/permission-filtered queries with AI off, stale/cancel rejection and typed actions in Linux UI.

## 0.2-M27 — Albert / Terminal / Core Experience Platform Adoption

[Source §32 M27](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L2073). Lane G. Dependencies: M07–M10; M17–M22; M26.

- `M27.01` Albert renders representative real HTTPS sites on Nagi target;
- `M27.02` current page/selection can be published as bounded untrusted Context;
- `M27.03` browser history/bookmark/session persistence remains intact;
- `M27.04` a browser page/reference can be consumed by another first-party app through public contracts;
- `M27.05` Terminal runs without bypassing capability boundaries;
- `M27.06` platform integration does not regress 0.1 browser acceptance.

**Current evidence.** Main host CI exercises Albert state/input adapters; old guest paths do not transfer. [user/nagi-albert](../user/nagi-albert) · [user/nagi-albert-linux-tests](../user/nagi-albert-linux-tests)

**Linux acceptance work.** Pin and test Servo with real HTTPS on Linux, session/bookmark/history, bounded untrusted Context and public references, permissions on transfers/clipboard/IME and Terminal. If a Linux 0.1 browser ships, regress its accepted path. Otherwise submit behavior-equivalence fixtures and frozen Linux 0.1 platform regressions for Integration Owner review as a proposed equivalent for M27.06.

## 0.2-M28 — Full Platform Integration / Security / Stress / Accessibility

[Source §32 M28](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L2103). Lane A. Dependencies: Applicable M01–M27 target and cross-lane checks.

- `M28.01` no kernel OOM under reference load;
- `M28.02` desktop remains interactive;
- `M28.03` no sustained audio failure caused by background/platform work;
- `M28.04` model runtime does not monopolize the machine;
- `M28.05` no major handle/memory leak;
- `M28.06` denied capabilities remain denied across restart;
- `M28.07` one crashed service does not silently corrupt unrelated service state;
- `M28.08` persisted user data survives service/system restart;
- `M28.09` first-party core remains usable without model runtime;
- `M28.10` keyboard/focus path works on representative core apps;
- `M28.11` English/Japanese smoke tests pass;
- `M28.12` integration result is tied to immutable source/build fingerprint.

**Current evidence.** Current-main CI and #163 bounded guest image are inputs only. [tools/linux-boot](../tools/linux-boot) · [workflows/ci.yml](../.github/workflows/ci.yml) · [ci-acceptance/state.json](../.dev/workstreams/ci-acceptance/state.json)

**Linux acceptance work.** Use one immutable Linux image and old reference load. Inject crash + restart, compare UX and audio with competing AI/jobs, run AI-off, denied-capability persistence, durable data, a11y, both locales, memory/handles/OOM and fingerprints.

Use the existing reference workload: Desktop, Home, Files, Notes, Albert (3–5 tabs), Terminal, Activity, Wayback, Search, Notification service, background jobs, Granite available, and audio playback on x86-64 QEMU (4 vCPU, 8 GiB). Record host/guest resource limits, image identity, workloads and latency distributions. Compare with DEC-116; the Task24 measurements cover a narrower workload.

## 0.2-M29 — Release Hardening / Documentation / Reference Hardware Preparation

[Source §32 M29](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L2159). Lane A. Dependencies: M02–M03; M21; M28.

- `M29.01` clean checkout can reproduce the build;
- `M29.02` public contracts have version documentation;
- `M29.03` known limitations are explicit;
- `M29.04` release SBOM/NOTICE candidates are generated;
- `M29.05` no release-blocking unresolved security/authority defect;
- `M29.06` no hidden requirement for broad physical hardware support;
- `M29.07` x86-64 reference-device plan does not leak device-specific behavior into high-level services.

**Current evidence.** Release tooling and docs exist; release instructions retain legacy assumptions. [tools/legal](../tools/legal) · [nagi-release/README.md](../tools/nagi-release/README.md) · [sdk/README.md](../sdk/README.md)

**Linux acceptance work.** Produce clean reproduction and candidate notices, freeze versioned API/docs, document limitations/security disposition and isolate architecture/device-specific adapters. Do not turn hardware preparation into broad hardware acceptance.

## 0.2-M30 — Nagi OS 0.2 Release

[Source §32 M30](https://github.com/Roy-To/NagiOS/blob/1999e58b31c7c93817a420cfc5f8dc40f75fecec/docs/Nagi_OS_0.2_Codex_Implementation_Spec.md#L2203). Lane A. Dependencies: All required M00–M29 gates.

- `M30.01` M00-M29 required release gates PASS;
- `M30.02` immutable release source commit;
- `M30.03` release QEMU acceptance PASS;
- `M30.04` artifact hashes verified;
- `M30.05` clean-environment reproduction verified;
- `M30.06` 0.2 first-party core acceptance PASS;
- `M30.07` release documentation matches actual capability;
- `M30.08` 0.3 roadmap is documented but not falsely presented as 0.2 functionality.

**Current evidence.** No Linux 0.2 release evidence in the reviewed source or GitHub Releases. [nagi-release/release.py](../tools/nagi-release/release.py) · [ROADMAP.md](../ROADMAP.md)

**Linux acceptance work.** Make and verify immutable Linux release image, hashes, source/build manifest, SBOM/notices and docs; repeat clean reproduction and QEMU first-party core suite. Label 0.3 as roadmap.
