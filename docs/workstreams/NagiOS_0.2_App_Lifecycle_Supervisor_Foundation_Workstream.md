# Nagi OS 0.2 — App Lifecycle / Supervisor Foundation Workstream

## 1. Workstream

- **Requested ID:** `APP-LC-01`
- **Name:** App Lifecycle / Supervisor Foundation
- **Proposed branch:** `codex/0.2-app-lifecycle-supervisor`
- **Recommended worktree:** `/Users/tozawa/Developer/NagiOS-0.2-app-lifecycle-supervisor`
- **State:** `.dev/workstreams/app-lifecycle-supervisor/state.json`
- **Target:** Nagi OS 0.2 host-testable foundation
- **Initial status:** `NOT_STARTED`

The repository already contains `docs/workstreams/NagiOS_0.2_App_Lifecycle_Package_Manifest_Foundation_Workstream.md`, which also names `APP-LC-01`. This document does not overwrite or silently replace it. Before either specification is registered or activated, the Integration Owner must resolve the duplicate ID and align their ownership. The supervisor scope below is a proposed independent contract; `.dev/workstreams.json` remains untouched until that decision.

This document is an implementation specification, not permission to begin runtime work. Nagi 0.2 runtime streams remain `NOT_STARTED` until Nagi 0.1 M30 passes and an explicit integration checkpoint opens this stream.

## 2. Purpose

Provide one user-space supervisor contract for launching and managing application execution instances. Keep application identity and package-manifest validation as the responsibility of the existing App Lifecycle / Package Manifest contract; consume its public `AppId` and launch metadata through a narrow adapter.

The foundation must define deterministic launch, suspend/resume, termination, abnormal-exit handling, bounded restart policy, duplicate-launch behavior, and safe identity mapping. It must be host-testable without pretending that a host process is a Nagi guest process.

## 3. Scope

- Lifecycle state machine: `Starting`, `Running`, `Suspended`, `Stopping`, `Exited`, `Crashed`, `Restarting`, and `CrashLoop` (or repository-equivalent states).
- `launch`, `suspend`, `resume`, graceful `terminate`, forced termination after a deadline, and final-exit reporting.
- Normal versus abnormal exit classification, exit reason, and deterministic recovery.
- Explicit restart policy with bounded attempts, deterministic delay/backoff, and exhaustion behavior. A user-requested termination must not trigger an automatic restart.
- Duplicate-launch policy, at minimum `Reject`, `FocusExisting`, and `AllowMultiple`; define the default and test each supported policy.
- Mapping among logical `AppId`, `AppSessionId`, `ExecutionInstanceId`, supervisor instance ID, and an opaque process handle/ID. Never treat an app ID, session ID, or PID as interchangeable.
- Process-backend adapter for launch/control/exit observation, with a deterministic in-memory fake for orchestration tests.
- Capability-principal adapter binding the exact app, execution instance, and process identity before execution is exposed.
- Structured lifecycle diagnostics hooks and correlation across a launch/recovery sequence.

## 4. Non-goals

- App package installation, manifest ownership, signature verification, or update transactions.
- Reimplementing or replacing the existing `APP-LC-01` package/manifest contract.
- Kernel process-management syscalls, kernel scheduler changes, or a new process ABI.
- A GUI task manager, application UI, or app-specific behavior.
- Capability grant policy, permission prompts, or universal app privileges.
- M18 browser/Servo work or any first-party product integration.
- Host OS process supervision as Nagi guest behavior.

## 5. Dependencies and contracts

- Existing AppId/package-manifest contract: consume a published version or a small adapter; do not define a competing manifest format.
- Capability / Permission: consume its public principal/authorization contract where available; otherwise use a narrow adapter and record the exact convergence point.
- Diagnostics / Observability: emit through a sink/trait without importing its internal implementation.
- Process execution: use existing Nagi process interfaces if present. If absent, define a backend trait and test fake; do not invent kernel calls.
- Activation gate: Nagi 0.1 M30 `PASS` plus an explicit Integration Owner checkpoint.

## 6. Allowed and forbidden edits

Allowed after activation:

- `crates/nagi-app-lifecycle-supervisor/**` (or a repository-approved equivalent owned crate/module);
- `tests/app-lifecycle-supervisor/**`;
- `.dev/workstreams/app-lifecycle-supervisor/**`;
- this workstream document and narrowly owned examples/fixtures.

Do not edit without explicit Integration Owner ownership:

- `Cargo.toml`, `Cargo.lock`, `.github/workflows/**`, shared IDL/ABI, `.dev/workstreams.json`, `.dev/schemas/**`;
- another workstream's source, tests, branch, state, or worktree;
- `docs/implementation_status.md` or M17/M18 acceptance records;
- kernel/process ABI, `user/nagi-init/**`, browser/Servo/Mesa/relibc sources, or first-party app features.

If workspace registration, shared interfaces, or CI wiring is needed, prepare a focused proposal under the owned `.dev/workstreams/app-lifecycle-supervisor/` directory and continue all independently testable work.

## 7. Architecture and API/data model

Use a small provider-neutral supervisor API. Suggested concepts (adapt to repository naming):

- `AppInstanceId` / `ExecutionInstanceId`: unique per execution; not derived from a PID.
- `ProcessHandle`: opaque backend-owned identity with stale-handle rejection.
- `LaunchRequest`: `AppId`, optional `AppSessionId`, validated launch descriptor, duplicate policy, restart policy, and caller principal.
- `LifecycleState` and `LifecycleEvent`: previous/next state, stable reason code, instance identity, timestamp/sequence where supported, and correlation ID.
- `ProcessBackend`: `spawn`, `request_suspend`, `request_resume`, `request_terminate(deadline)`, `force_terminate`, and exit observation.
- `CapabilityPrincipalBinder`: bind and revoke a principal for the exact execution instance; fail closed if binding cannot be confirmed.
- `DiagnosticsSink`: best-effort structured event output that cannot block lifecycle progress indefinitely.

The supervisor owns transition validation and restart accounting; the backend owns process-specific mechanics. State changes occur only after the backend reports the corresponding outcome. Public errors are typed and stable, including duplicate launch, invalid transition, backend unavailable, permission denied, timeout, abnormal exit, restart exhausted, and stale process handle.

## 8. Failure and recovery

- A failed spawn leaves no visible `Running` instance and releases any provisional principal binding.
- Backend loss or unknown exit status is recorded as an explicit failure, never silently treated as success.
- Graceful shutdown requests have a finite deadline; expiry triggers the configured forced-termination path and is separately diagnosed.
- Restart count and delay are bounded; `CrashLoop` stops retries until an explicit policy/user action resets it.
- If principal binding fails, terminate/reap the new process and report denial; do not launch with ambient authority.
- Supervisor restart/recovery must reconcile observed backend instances before issuing duplicate launches.
- Diagnostics sink failure must not crash the app or stall shutdown; retain only bounded safe evidence when available.

## 9. Security and capability hooks

- Bind each process to the least-authority principal for its app and execution instance before exposing it as running.
- Never infer or strengthen rights from an app ID, PID, parent process, or restart operation.
- Revoke or invalidate the prior execution principal after exit; a restarted process receives a new instance identity and a fresh authorization decision.
- Require explicit authorization hooks for launch, terminate, and cross-app operations. This workstream defines the hook boundary, not grant policy or user prompts.
- Do not log credentials, capability secrets, document contents, or arbitrary environment payloads.

## 10. Diagnostics hooks

Emit stable machine event codes for launch requested/accepted/failed, state transition, duplicate launch decision, shutdown deadline/forced termination, normal exit, abnormal exit, restart scheduled, restart exhausted, crash loop, and principal binding failure. Include app/instance/correlation IDs and safe reason codes; keep user-facing localized text outside the core contract. Diagnostics are best effort and must not become an Activity Ledger or permission store.

## 11. Acceptance Criteria

- [ ] The supervisor API and state machine are documented and typed.
- [ ] Legal and illegal transitions are deterministic and tested.
- [ ] Launch success/failure and app/process identity mapping are tested.
- [ ] Suspend/resume and graceful shutdown deadline/forced termination are tested.
- [ ] Normal exit, abnormal exit, crash, restart, bounded backoff, exhaustion, and crash-loop recovery are tested.
- [ ] Duplicate launch policies have explicit, tested outcomes.
- [ ] Capability principal binding fails closed and is renewed per execution instance.
- [ ] Diagnostics events are structured, correlated, privacy-safe, and non-blocking.
- [ ] In-memory backend tests exercise orchestration without claiming guest-process acceptance.
- [ ] Existing AppId/manifest and process contracts are consumed through adapters; no competing contract is introduced.
- [ ] No M18 main / M18-A / M18-B / M18-C or unrelated workstream implementation is modified.
- [ ] Workstream state and any registration proposal follow DF-01 schemas/ownership.
- [ ] Focused checks pass; changes are committed and pushed only on the owned branch.

## 12. Tests and CI

Cover lifecycle transition tables, stale handles, spawn rollback, shutdown timeout, repeated termination, normal/abnormal exits, each duplicate policy, restart limits/backoff, crash-loop behavior, principal bind/revoke failures, diagnostics sink failure, and privacy-safe event fields. Use deterministic fakes; do not use network access or host processes as guest acceptance.

Run focused package tests, formatting, Clippy with warnings denied where supported, and relevant contract/schema checks. Do not change shared CI workflows; provide a narrowly scoped CI proposal if a new job is required. M18/browser target acceptance is neither a dependency nor an allowed substitute.

## 13. State, commit, and push

Update only `.dev/workstreams/app-lifecycle-supervisor/state.json` using the current state schema: actual status, activation gate, checked commit, exact commands/results, blockers/evidence, acceptance checklist, dependencies, and next action. If the stream is not registered and the CLI rejects its state, preserve a registration proposal and record that precise limitation; do not edit the shared registry or claim `PASS`.

Commit only owned files on the assigned branch. Push that branch after a reviewable checkpoint, then verify the pushed SHA, remote equality, clean owned worktree, and CI result. Never merge to `main` without explicit direction.

## 14. Git and worktree safety

Inspect `git status`, `git worktree list`, branch, HEAD, and remote before acting. Reuse and preserve an existing dedicated worktree for this stream. Do not reset, clean, force-checkout, rebase, or switch branches in a worktree owned by another stream. If no dedicated worktree exists, use the Integration Owner-approved branch/worktree procedure; never guess a base SHA.
