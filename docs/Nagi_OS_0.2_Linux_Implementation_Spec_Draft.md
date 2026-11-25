# Nagi OS 0.2 — Platform Foundation (Linux revision draft)

Date: 2026-11-25
Status: REVIEW NOTE / PROVISIONAL — review draft, not activation authorization or replacement of the accepted specification.
Target: Linux kernel + Nagi-owned userland.

## 1. Authority and scope

The founder has authorized preparation of this revision, not adoption of new feature assignments or changes to safety, ownership, acceptance or activation boundaries. Read [operating rules](product-decisions/OPERATING_RULES.md), [decision index](product-decisions/README.md), [DEC-123](product-decisions/DEC-123-kernel-unification-and-linux-0.1.md), [DEC-111](product-decisions/DEC-111-implementation-readiness-triage.md), [ROADMAP](../ROADMAP.md), the [existing 0.2 specification](Nagi_OS_0.2_Codex_Implementation_Spec.md), [Linux addendum](Nagi_OS_0.2_Linux_Migration_Addendum.md), and [revised 0.3 roadmap](product-decisions/roadmaps/Nagi_OS_0.3_Architecture_Roadmap_Revised.md). Later confirmed DEC decisions prevail over older contradictory text. All original 0.2 deliverables and acceptance criteria remain applicable except where an explicit later decision supersedes them; Linux replacements for legacy implementation mechanisms must be reviewed and evidenced rather than silently dropping checks.

CONFIRMED (existing decisions):
- Linux is the sole kernel line, with Nagi-owned userland, not an existing distribution as the product base. Legacy M0–M30 evidence remains historical.
- DEC-123 replaces the old M30 PASS prerequisite with publication of Linux 0.1 Developer Preview, including creation and verification of distribution artifacts. Explicit release boundary and Integration Owner checkpoint, other activation conditions, workstream ownership and acceptance remain unchanged.
- Offline-first; usable without AI; AI is non-privileged and untrusted. Deterministic authorization, explicit grants, revocation, user/profile isolation, truthful results and public contracts apply to every app and model.
- DEC-111: do not create a new all-details-approved gate. Work within existing authorization; reversible technical experiments may proceed without premature product decisions.

PROVISIONAL: detailed 0.1 scope, L0–L12 numbering, the Linux-specific sequencing and lane grouping below, concrete runtime technologies, devices, performance thresholds and release timing. The root roadmap lists Albert, local AI, voice, semantic search and A/B recovery as 0.2 directions; this draft does not newly approve their exact minimum scope or silently narrow the existing 0.2 platform requirements.

## 2. Activation and evidence

Before runtime activation, verify published 0.1 artifact, source revision, manifest, hashes, acceptance and known limitations; record explicit 0.2 release boundary, owner checkpoint and registry state. Preserve 0.1 regression tests. Before activation only already-authorized Linux migration, isolated host foundation, contracts, fixtures and other approved preparatory work may proceed. This document grants no additional workstream or branch permissions.

Evidence snapshot (review note, public PR records inspected 2026-11-25):
- #163 merged: bounded Linux development-image acceptance for Task24, aggregate PID 1 READY, System RO/Data RW, WS-10 and measured QEMU performance. Not a verified boot chain, final product image, physical device or general app isolation acceptance.
- #237 merged: legacy kernel/build paths retired; historical records retained.
- #238/#240 merged: Files host foundation and durability; Files guest acceptance not established.
- #241/#242/#243/#244 merged: desktop launcher, session-bound profile storage, Settings, Calendar. Source-lane guest evidence does not establish combined main guest acceptance. #245 Files caller is draft, compile-only as reported.
- #246 NDP host pilots open; no target SDK/app acceptance inferred.
- Recheck current main, owning states and CI by exact SHA before starting work. This is not a complete live branch audit.

0.1 handoff checklist: release PID 1, authenticated login, selected Data, Notes and approved app paths; verified boot and app isolation per agreed 0.1 scope; no development entry points in release; source/kernel pins; negative and restart tests; integrated QEMU artifact. The exact 0.1 public scope and L11/L12 criteria still require owner confirmation under DEC-123; this list is a review aid, not a new gate.

## 3. Linux architecture and contracts

Linux kernel provides process/memory/device/network/filesystem primitives. Nagi PID 1 / service manager supervises services, authenticated local IPC and app lifecycle; final Init choice remains subject to DEC-111 F04 comparison. Nagi Core owns Identity, Permission, Settings, Jobs, Notifications, Update, Diagnostics, Model Runtime and platform providers. Public versioned contracts expose Resource, Document, Object, Action, Context, Workspace, Activity, Revision and Checkpoint. Nagi-owned shell and apps use the same public contract as third-party apps. No existing Linux UID, root, app signature or model identity substitutes for Nagi authorization.

Transport: evaluate existing Linux peer credentials and selected IPC mechanism against typed bounded request/response, request correlation, caller/session binding, version errors, cancellation and capability checks. Preserve logical IDs across process restart and resource moves where supported. Protect System from writes and scope Data to owner/profile/app; validate durable state, migration and interrupted mutation. Sandbox implementation must reject escape, revoked/expired sessions, unauthorized files and scope escalation; do not equate a test-only sandbox with product enforcement.

Reference target proposal: retain x86-64 QEMU, 4 vCPU/8 GiB baseline from prior specification, with kernel and image inputs pinned. DEC-116 governs provisional performance and minimum device targets; publish measurements, method, workload and limitations; no unmeasured guarantee. Physical and ARM64/Raspberry Pi work is separate evidence and requires explicit supported-device scope.

## 4. Legacy milestone crosswalk and Linux acceptance

This table is an additive review mapping, not a reset of old status or a new milestone approval. Original detailed deliverables, negative tests and acceptance in §32 of the existing specification remain required unless explicitly superseded. Linux target tests replace obsolete custom-kernel-specific paths only with equivalent reviewed evidence.

| Existing 0.2 ID | Proposed Linux work / dependency | Minimum evidence focus (in addition to original acceptance) |
|---|---|---|
| M00 | 0.1 publication and activation | DEC-123 gate, immutable baseline, registry verification and owner checkpoint |
| M01 | Development foundation | registry, resume/verify, ownership and fail-closed validation |
| M02 | Provenance | Linux kernel/config/toolchain/rootfs/artifact hashes and deterministic mismatch detection |
| M03 | SBOM/legal | Linux/OSS/model inventory, notices, unknown licenses, no private paths |
| M04 | Diagnostics | real Linux service/crash data, redaction, correlation and bounded sinks |
| M05 | Test platform | deterministic host fixtures, Linux guest adapter, immutable evidence, failure injection |
| M06 | Localization | en-US/ja-JP, locale fallback, catalog and representative UI tests |
| M07 | UI system | shared Linux shell/app primitives, keyboard focus, accessibility and localization |
| M08 | Identity/Permission | extend 0.1 session/storage foundation; wrong profile, expiry, revocation and delegation denial |
| M09 | OS Core/ABI | Linux transport/authority adapters, versioned public ABI and 0.1 regression; no new custom kernel |
| M10 | Service IPC | authenticated peer, bounded typed request, correlation, invalid version and real guest call |
| M11 | Settings | scoped typed settings, atomic persistence, migration, subscriber behavior and real consumers |
| M12 | Manifest/package | shared versioned manifest validation, declared permissions, package and SDK compatibility |
| M13 | Supervisor/Jobs | actual Linux managed service/app and background job, bounded restart, cancel, retry, no privilege bypass |
| M14 | Notifications | durable profile-scoped delivery, quiet policy, action revalidation and UI |
| M15 | Install/update | signed validated packages, atomic inventory, interruption and rollback, Data preservation; Linux A/B recovery scope per approved design |
| M16 | Model Runtime | provider-neutral store, digest checks, real local Granite inference, load/unload/cancel, bounded resources, offline fallback |
| M17 | Resource/Action | stable IDs, revision/provenance, typed actions and risk metadata, actual Files/Notes operations |
| M18 | Activity/Wayback | persistent actor/transaction/revision/checkpoint, truthful restore, no fake undo, redaction |
| M19 | Search/Workspace | permission-filtered lexical/metadata/temporal and semantic provider; deletion/restart; AI-off works |
| M20 | Context/session/surface | explicit app publication, logical session identity, filtered context, no memory scraping |
| M21 | SDK | published-only out-of-tree sample, validate/install/launch on Linux target with declared authority; NDP host results separate |
| M22 | Common app framework | actual cross-app adapters for shared IDs/Actions/Context/Activity/Wayback/Workspace |
| M23 | Files | actual Linux storage, create/copy/move/rename/trash/restore, policy and checkpoints |
| M24 | Notes | persistence/reopen, autosave failure retains dirty state, revision/restore, references |
| M25 | Activity/Wayback apps | visible actor/status, real diff/restore, recovery point, no undo for irreversible effects |
| M26 | Home/Search apps | actual registry/provider, permission filter, stale query suppression, typed result actions |
| M27 | Albert/Terminal | Servo retained; real HTTPS, history/session, context, downloads/uploads/clipboard/IME and permissions; Terminal same authority boundary |
| M28 | Integrated acceptance | real combined Linux target load, pressure/latency, restart, security, accessibility, AI-off, both locales |
| M29 | Hardening | clean checkout, docs/SDK, freeze, legal review, measured performance, hardware preparation |
| M30 | 0.2 release | all applicable required gates, immutable artifacts, hashes, clean reproduction, verified core experience and limitations |

Voice: root ROADMAP lists voice as a 0.2 direction. Proposed separate cross-cutting lane (not new milestone): Linux audio backend and permissioned local capture/playback, bounded STT/TTS provider, stop and text fallback, offline tests and resource-pressure tests. Exact provider and release-blocking scope require confirmation; no bypass of microphone or cloud consent. Keep original audio-continuity requirement.

## 5. Proposed parallel execution lanes

| Lane | Milestones | Dependencies and next evidence |
|---|---|---|
| A — integration/release | M00, M01–05, M28–30 | owner-controlled registry/shared paths, pinned baseline, reproducibility and acceptance harness |
| B — system/security | M08–10, M13 | 0.1 identity, storage and PID 1 baseline; authenticated IPC, isolation and failure tests |
| C — settings/UI | M06–07, M11, M14 | shared contract + identity/IPC before target service; host fixtures may prepare |
| D — install/recovery | M12, M15 | signed manifest, storage ownership and journal, atomic update, rollback, power-failure tests |
| E — AI/audio | M16 + voice candidate | provider contracts, network/mic policy, resource accounting; local inference/cancel/AI-off tests |
| F — semantic platform | M17–20 | stable owner/Resource/Action contracts, Activity/Wayback, authorized indexing and context |
| G — first-party apps | M22–27 | public contracts and target adapters; parallel app-specific host work and focused guest acceptance |
| H — developer platform | M21 | SDK/manifest/public ABI, NDP host integration, out-of-tree target sample |

Lane names are planning labels, not assignments. Integration Owner allocates actual branches/path ownership and explicit checkpoints. Do not wait for unrelated work; dependencies gate actual target activation, not authorized independent preparation. Shared Cargo/workflow/registry files remain owner-controlled. Do not create duplicate contracts. Reuse working implementations and verify exact dependencies.

Suggested order: freeze 0.1 release boundary; reconcile existing registry/CI and Linux contracts; run B/C/D/E/F/H independently where boundaries permit; connect G against versioned contracts; run M28 integrated tests and M29/M30 hardening/release. No date promised.

## 6. Forward compatibility and decision queue

0.3 must be able to bind User/Profile/Session/App/Caller and request correlation. Model Runtime does not own canonical Memory. Keep model-neutral Context, versioned actions, resource revisions, cancellation, actor/provenance and future Task metadata; permission-filter before context construction and revalidate at execution. Do not implement a privileged AI path or require AI for basic OS operation. Do not force 0.3 Agent features into 0.2. Revised 0.3 roadmap §26 specifies readiness of Identity, Settings, IPC, Model Runtime, Permission, Storage and Jobs as conditions for writing formal 0.3 spec (50–70% stability is a guide, not measured progress or activation permission).

| Decision | When needed | Can prepare before decision |
|---|---|---|
| 0.1 release scope and Linux milestones | before public release acceptance | existing app and release-hardening work |
| Init final choice (F04) | after comparable Linux evidence | bounded A/B experiments |
| 0.2 feature minimum / core apps, voice and A/B scope | before freezing revised release plan | adapters, tests, existing authorized foundations |
| Model/provider and resource targets | after representative measurements | provider-neutral implementation, test harness |
| PX-05 Action risk and human-only operations | before enabling side effects | typed actions, deny-by-default, negative tests |
| PX-08 important Memory confirmation | before enabling automatic learning | source/provenance and scope contracts |
| PX-09 retention/automatic deletion | before automatic deletion | decay/archive, explicit forget safety |
| PX-12 cross-app scope | before official UX acceptance | samples and public API |
| F16 backup/restore coverage | before product guarantee | durable per-service data and recovery tests |
| PX-11/14/15 1.0 apps/SDK/release criteria | before relevant release commitments | independent approved development |

DEC-124 System Skills, DEC-125 AI operations status and DEC-010 workspace modes are later confirmed high-level directions; exact version, schema, UI and acceptance remain provisional. Do not create new 0.2 requirements or grant unrestricted shell access.

## 7. Completion and review

For each authorized milestone: inspect registry/state and exact source; implement smallest correct change; format/build/focused tests; host and Linux target/guest tests as applicable; negative and restart/failure tests; record immutable SHA, artifact hashes, commands/config, limitations and actual results; independent review; push only owning branch; CI check; integration owner acceptance. Host PASS, compile-only, source-lane PASS, combined guest PASS and release PASS are distinct. Never use legacy evidence as Linux acceptance.

Before adoption of this revision, Integration Owner should reconcile all original §32 criteria, existing current Linux lane evidence and DEC details, confirm release scope and dependency/ownership changes if any, and review the normative text. This draft deliberately retains the original full acceptance specification by reference; it is not yet a standalone executable replacement. No product decision or gate is newly confirmed by this document.

## Revision history

- 2026-11-25: Initial Linux revision review draft; crosswalk M00–M30, activation correction per DEC-123, evidence snapshot, parallel lanes and decision queue. No implementation, test execution, release or new activation authorization.
