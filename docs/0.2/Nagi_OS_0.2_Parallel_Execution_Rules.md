# Nagi OS 0.2 Parallel Implementation Rules

## 1. Purpose

This document defines the execution rules for Nagi OS 0.2 workstreams that are implemented in parallel while Nagi OS 0.1 remains under active M17-M30 implementation/integration.

The primary product specification is:

- `Nagi_OS_0.2_Codex_Implementation_Spec.md`

This parallel work MUST accelerate 0.2 without destabilizing, redefining, or bypassing the active 0.1 release line.

## 2. Authority and precedence

Before editing code, read all of the following that exist in the checkout:

1. `AGENTS.md`
2. `Nagi_OS_0.2_Codex_Implementation_Spec.md`
3. `docs/implementation_status.md`
4. `docs/0.2/WORKSTREAMS.md`
5. `docs/0.2/DEVELOPMENT_ARCHITECTURE.md`
6. `.dev/workstreams.json`
7. relevant `.dev/workstreams/*/state.json`
8. relevant existing workstream MDs under `docs/workstreams/`

For product scope, milestone intent, and 0.2 acceptance, the master 0.2 specification is authoritative.

For current ownership, branch assignment, activation gates, shared-file ownership, and already-completed work, the live repository state is authoritative.

If these disagree, do not silently overwrite the newer repository state. Preserve current ownership and record the discrepancy in the workstream state/proposal.

## 3. Mandatory first audit

At the beginning of every parallel session:

1. Fetch/prune remote refs.
2. Inspect the current remote HEAD of `codex/integration-next-phase` and the active 0.1 implementation branches.
3. Inspect `.dev/workstreams.json` and all relevant workstream states.
4. Search remote branches for an existing branch for the requested workstream.
5. Search code/docs for an existing implementation under another workstream ID.
6. Confirm that the requested work is still independent of the active 0.1 M17-M30 implementation.
7. Confirm that the requested work does not require an M30 PASS + explicit 0.2 integration checkpoint.

If the requested branch already exists, resume it. Do not create a duplicate branch.

If the same responsibility is already owned by another workstream, do not create a competing implementation. Adapt to the existing owner or record a blocker/proposal.

## 4. Current repository snapshot to treat only as a starting hint

As of the preparation of this document, the following dedicated 0.2 branches already exist:

- `codex/0.2-app-lifecycle-manifest`
- `codex/0.2-app-lifecycle-supervisor`
- `codex/0.2-app-sdk-contract`
- `codex/0.2-build-provenance`
- `codex/0.2-capability-permissions`
- `codex/0.2-development-foundation`
- `codex/0.2-ident-01`
- `codex/0.2-job-01`
- `codex/0.2-license-sbom`
- `codex/0.2-notify-01`
- `codex/0.2-settings-configuration`
- `codex/0.2-system-service-ipc`
- `codex/0.2-test-plat-01`
- `codex/0.2-update-installation`
- `codex/0.2-wayback-ledger`

Other existing workstream branches include:

- `codex/ws-diagnostics`
- `codex/ws-model-runtime`
- `codex/ws-ui-design-system`
- `codex/ws-localization-i18n`
- `codex/app-files`
- `codex/app-home-search`
- `codex/app-notes`
- `codex/app-activity-wayback`
- `codex/first-party-integration`

At this snapshot, host-side foundation work was already complete or substantially complete for Development Foundation, Test Platform, Notifications, Update/Installation, App Lifecycle Manifest, License/SBOM, App SDK, Model Runtime, Diagnostics, Localization, and Wayback/Activity Ledger.

JOB-01, IDENT-01, Capability/Permissions, and First-Party Integration already had substantial host-only work and intentionally retained later runtime/target integration gates.

Settings and App Lifecycle Supervisor were explicitly gated from runtime implementation by M30 + an explicit 0.2 checkpoint. IPC/runtime ownership also requires gate/owner review.

Therefore: do not consume a new parallel slot by reimplementing these foundations unless the live state and master 0.2 specification show a concrete unfulfilled host-only Acceptance item.

## 5. Parallel-safe work definition

A work item is safe to execute before 0.1 M30 only when all of the following are true:

- it can be implemented and meaningfully tested on the host;
- it does not require target/QEMU acceptance to claim its Foundation result;
- it does not modify the active 0.1 kernel, boot, compositor/GPU, Servo/browser, AI-runtime target path, or M17-M30 milestone implementation;
- it does not require production runtime service registration;
- it does not require shared IPC/runtime wiring that is still gated;
- it does not require root/shared files owned by another workstream;
- it can expose contracts/adapters without redefining another owner's canonical IDs or policy engine;
- it can remain standalone until the future Integration Owner checkpoint.

## 6. Forbidden actions during pre-M30 parallel implementation

Unless the live Integration Owner state explicitly authorizes it, do NOT:

- merge to `main`;
- merge to `codex/integration-next-phase`;
- modify active 0.1 milestone implementation to make 0.2 tests pass;
- weaken or skip 0.1 acceptance tests;
- claim M30 or product/runtime acceptance;
- edit `.dev/workstreams.json` when it is Integration Owner-owned;
- edit shared schemas/IDL/ABI owned by another workstream;
- change root Cargo workspace/lockfile merely to integrate a standalone 0.2 crate;
- change shared CI merely to make a feature branch green;
- implement Settings/IPC/Supervisor production runtime while their gate is closed;
- create a duplicate canonical ID, manifest, capability, identity, storage, search, or lifecycle contract.

When registration or shared integration is needed, create a focused proposal in the owned workstream directory instead of crossing the ownership boundary.

## 7. Branch and worktree isolation

Each parallel workstream must use a dedicated branch and worktree.

Preferred branch naming:

- `codex/0.2-<workstream-id>`

Preferred worktree naming:

- `~/.codex/worktrees/<workstream-id>/NagiOS`

Use the current approved integration base discovered at execution time. Do not hard-code an old commit if the repository has advanced.

Never perform two workstreams in the same worktree.

## 8. State and registration

Every new workstream must maintain a DF-01-compatible state under an owned path such as:

- `.dev/workstreams/<workstream-id>/state.json`

If registration is Integration Owner-owned, also prepare a minimal registration proposal rather than editing the shared registry directly.

State must truthfully distinguish:

- `PASS`: the defined Foundation Acceptance for this workstream is complete;
- `PARTIAL`: meaningful implementation is complete but required Acceptance remains outside the current gate/ownership;
- `BLOCKED`: no further useful owned work can proceed until an external dependency/gate/owner action occurs;
- `NOT_STARTED`: implementation is intentionally not authorized yet.

Do not use `PASS` to imply production runtime, target, or Nagi 0.2 release acceptance unless those criteria were actually executed.

## 9. Non-stop implementation loop

Within the allowed scope, continue without stopping after the first successful edit.

Repeat:

1. inspect current code/contracts;
2. implement the next owned Acceptance slice;
3. add or extend deterministic tests;
4. run focused tests;
5. run formatting and warnings-denied linting where applicable;
6. run repository verification that is valid for the branch;
7. fix failures caused by this workstream;
8. repeat until all currently legal Foundation Acceptance is complete;
9. update state with exact evidence, blockers, and deferred runtime work;
10. run final diff/ownership audit;
11. commit only owned changes;
12. push the dedicated branch;
13. verify remote HEAD equality and clean worktree.

Do not stop merely because one test initially fails. Diagnose and fix in-scope failures.

Do stop before crossing a documented ownership boundary or closed activation gate. Record that boundary precisely instead of bypassing it.

## 10. Final report requirements

The final Codex report must include:

- workstream ID;
- final status;
- branch;
- pushed HEAD SHA;
- remote equality result;
- worktree cleanliness;
- implemented files/components;
- tests/checks and results;
- remaining blockers/deferred items;
- whether any failure belongs to the active 0.1 release line rather than this workstream;
- exact next Integration Owner action, if one remains.

