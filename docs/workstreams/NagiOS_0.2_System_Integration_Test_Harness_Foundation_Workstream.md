# Nagi OS 0.2 — System Integration Test Harness Foundation Workstream

## 1. Workstream

- **Requested ID:** TEST-PLAT-01
- **Registry ID:** test-plat-01
- **Name:** System Integration Test Harness Foundation
- **Owner:** Codex TEST-PLAT-01 workstream owner, assigned by the Integration Owner
- **Assigned branch:** codex/0.2-test-plat-01
- **Assigned base:** 198b9f60b41eba14e74c2c04f51ccdcb4030adf9
- **Assigned worktree:** /Users/tozawa/Developer/NagiOS-0.2-test-plat-01
- **State:** .dev/workstreams/test-plat-01/state.json
- **Target:** Nagi OS 0.2 host-only, deterministic integration-test foundation
- **Status:** IN_PROGRESS — host-only foundation active; product/runtime gates remain closed

The Integration Owner registered and assigned this workstream on 2026-09-29 from commit 198b9f60b41eba14e74c2c04f51ccdcb4030adf9. M17 remains BLOCKED and M30 remains NOT STARTED. The owner checkpoint authorizes this isolated host-only foundation before M30; it does not open runtime, target/QEMU, product-acceptance, or shared CI/workspace gates.

## 2. Purpose and background

Provide reusable host-side fixtures for testing interactions among multiple services and workstreams under deterministic conditions. Each test must control time, inputs, failures, resource limits, and cleanup so a failure can be reproduced without QEMU, network access, or reliance on the host's user data.

This harness verifies orchestration and service contracts. It does not substitute for a provider's own acceptance, target behavior, or product-level tests.

## 3. Scope

- Fake clock with monotonic virtual time, explicit advance, scheduled callbacks, and no real-time sleeps in deterministic tests.
- Isolated fake filesystem and temporary-root fixture with bounded file/byte quotas, path confinement, cleanup checks, and hooks for interrupted writes and corrupted state.
- Fake IPC transport/service bus with deterministic delivery, timeout, cancellation, malformed message, provider failure, queue saturation, and configurable drop/duplicate/reorder cases.
- Fake capability broker with per-principal and per-operation allow/deny rules; deny by default when a rule or caller context is missing.
- Fake diagnostics sink that captures ordered structured events and tests redaction without requiring a production diagnostics backend.
- Temporary user/profile/session fixtures that use opaque identities and isolated data roots; these fixtures must not claim to implement or replace Local Identity.
- Deterministic failure injection by named failpoint, including process/service crash, restart, interrupted write, corrupt state, timeout, cancellation, and resource exhaustion.
- Deterministic scheduling hooks for queued work and explicit service startup/shutdown order.
- Multi-service harness that starts, probes, stops, and restarts registered test services and reports partial startup and cleanup outcomes.
- Offline/no-network policy that fails closed if a test attempts network access.
- Bounded resource accounting for services, timers, tasks, messages, events, files, bytes, and handles; every test can verify cleanup.
- Reusable fixture APIs and a host-only CI invocation proposal. The harness itself must remain runnable without network access after its dependencies are prepared.

## 4. Non-goals

- QEMU, target, guest-kernel, or real device integration.
- Product feature implementation or an alternate runtime.
- Claiming another workstream's Acceptance Criteria on its behalf.
- Replacing the existing CI acceptance runner, changing cancellation policy, or owning CI workflow files.
- Using host processes, sockets, files, or scheduling as if they were Nagi guest behavior.
- Timing-sensitive benchmarks or nondeterministic stress results presented as deterministic acceptance.

## 5. Dependencies and activation gate

- **Required development contract:** development-foundation DF-01 state, ownership, and verification rules.
- **Optional adapters:** diagnostics, capability-permissions, IPC/event-bus, App Lifecycle, Identity, Job Scheduler, and Notification contracts. Until a provider contract is registered, versioned, and available on the approved integration base, keep the harness adapter local and use an explicitly test-only fake.
- **CI relationship:** ci-acceptance owns shared CI behavior. This workstream may define a host-only command and submit a workflow proposal; it does not edit .github/workflows/**.
- **Host-only activation:** the Integration Owner registration checkpoint activates implementation of the standalone host-only harness and test-only fakes on the assigned branch and within the approved path boundary. M30 is not required for this host-only foundation. Do not add it to product/runtime paths or use it to claim M30 or provider acceptance.
- **Runtime/integration gate:** any production service, target, QEMU, product acceptance, or shared CI/workspace integration requires Nagi 0.1 M30 PASS and an explicit Integration Owner checkpoint naming TEST-PLAT-01. A stricter registry gate takes precedence.

The registry proposal should have only currently registered hard dependencies. Future/unregistered streams are optional contract adapters, not invented registry IDs or blocking runtime dependencies.

## 6. Allowed and forbidden edits

Assigned allowed paths:

- crates/nagi-test-harness/**
- tests/system-integration-harness/**
- .dev/workstreams/test-plat-01/**
- this specification

Do not edit without explicit Integration Owner ownership:

- Root Cargo.toml, root Cargo.lock, .github/workflows/**, .dev/workstreams.json, or .dev/schemas/**.
- Shared IDL/ABI, generated bindings, system-wide schemas, docs/implementation_status.md, or another workstream's source, tests, state, branch, or worktree.
- kernel/**, loader/**, user/**, third_party/**, out/**, target/**, M18 main/A/B/C paths, or pure-app feature paths.
- Any file outside the assigned allowed paths.

A standalone crate may keep its own manifest and lockfile inside the approved owned crate directory. Root workspace registration, shared dependencies, or CI wiring must be a focused proposal under .dev/workstreams/test-plat-01/registration-proposal.json or a similarly scoped proposal; never edit the shared owner file directly.

## 7. Architecture and fixture API

Keep the public harness host-only and provider-neutral. Suggested APIs and values:

- HarnessBuilder / HarnessConfig: compose clock, filesystem, bus, capability broker, diagnostics, service factories, quotas, and offline policy.
- DeterministicClock: now, advance, schedule, cancel, and drain-due-work operations over virtual monotonic time.
- FakeFilesystem / TemporaryRoot: isolated roots, bounded reads/writes, atomic-write behavior, and named interrupted/corrupt-write failpoints.
- FakeServiceBus: typed request/event adapters, deterministic queues, finite deadlines, cancellation, and explicit delivery fault configuration.
- FakeCapabilityBroker: test principal plus operation/resource allow-deny rules; default deny.
- DiagnosticsRecorder: ordered event capture, bounded retention, and redaction assertions.
- UserProfileFixture: temporary opaque UserId/ProfileId/SessionId values and teardown verification, without owning identity semantics.
- FailurePlan / Failpoint: named deterministic failure selected by the test, never ambient random failure.
- ServiceHarness: dependency-aware startup, health probe, shutdown, crash/restart, and cleanup report for multiple fake or host-only test services.
- ResourceBudget / LeakReport: per-test limits and a report of timers, tasks, open handles, queued messages, files, bytes, and services left after teardown.
- HarnessResult: stage, virtual time, bounded failure inventory, redacted captured events, cleanup status, and stable failure class.

Avoid arbitrary object deserialization. Ensure quota checks happen before allocation or copying. Do not let test fixture types leak into product APIs.

## 8. Failure and recovery

- A failpoint is repeatable from a test name and explicit configuration; no hidden random seed or wall-clock race is required.
- Interrupted writes preserve the previous committed fixture state where the fixture contract says writes are atomic. Corrupt-state fixtures retain the exact injected bytes for diagnosis.
- A crashed service is marked unavailable before restart. Restart creates a new service instance identity and does not silently reuse stale handles or messages.
- Timeouts and cancellation complete within bounded virtual steps. Late responses are discarded or reported as late, never delivered twice.
- Startup failure shuts down already-started services in reverse dependency order and reports cleanup failures without hiding the original cause.
- Quota exhaustion returns a stable error before unbounded allocation. Teardown reports every leaked resource and performs bounded best-effort cleanup.
- Diagnostics sink failure must not change the system under test's outcome or prevent cleanup.
- Network attempts under offline policy fail deterministically and are recorded without opening a socket.

## 9. Security and capability hooks

- Capability checks are explicit and default-deny; no fake broker grants universal authority implicitly.
- A test may deliberately configure allow rules, but authorization context must identify the exact principal, operation, and resource.
- Fixture principals and test handles are not valid product credentials and cannot be serialized as authority.
- Temporary roots are unique per test, confined to their fixture directory, and never point at user data or repository source.
- Network access is disabled by default. Do not treat host filesystem, process, clock, or socket access as guest functionality.
- Captured diagnostics redact secret-like values and never retain credentials or capability secrets.

## 10. Diagnostics and observability hooks

Capture stable event IDs for test start/end, virtual-time advance, service lifecycle, IPC delivery/failure, authorization decision, failpoint activation, timeout/cancel, quota breach, offline network denial, and resource leak. Each event includes a test and correlation identity, virtual timestamp, stage, and stable reason code where applicable. Payloads are omitted or redacted. Retention is bounded and test results identify truncation.

## 11. Acceptance Criteria

- [x] Virtual time advances deterministically and tests do not require real sleeps.
- [x] Fake and temporary filesystem fixtures are isolated, bounded, path-confined, and cleanup-verifiable.
- [x] IPC fake covers allowlisted delivery plus timeout, cancellation, malformed input, provider failure, and queue limits.
- [x] Capability fake supports explicit allow and deny rules and denies missing context.
- [x] Diagnostics fake captures event ordering and validates redaction.
- [x] Temporary user/profile/session fixtures cannot cross-contaminate tests.
- [x] Named failure injection covers crash/restart, interrupted write, corrupt state, timeout/cancel, and resource exhaustion.
- [x] Deterministic scheduling and multi-service startup/shutdown are reusable APIs with bounded cleanup.
- [x] Offline policy blocks network use; resource budgets report leaks and enforce configured limits.
- [x] Host-only CI invocation is documented and does not require QEMU or network at test time.
- [x] Harness results clearly state that fakes test orchestration/contracts only and do not confer product acceptance.
- [x] Implementation changes stay within assigned paths; the Integration Owner registration checkpoint is the sole shared-registry exception, and no M18 or unrelated workstream state is modified.
- [x] State and registration proposals follow DF-01 ownership/schema; TEST-PLAT-01 implementation leaves shared schema, CI, and workspace files untouched.
- [x] Focused checks, owned-branch commit/push, remote equality, clean worktree, and CI status are recorded when branch/gates permit.

## 12. Tests and CI

Cover virtual-time ordering, cancellation, deterministic scheduling, filesystem isolation and quotas, partial/corrupt writes, IPC ordering/fault modes, default-deny capability behavior, event redaction, multi-service partial startup, crash/restart identity, cleanup after normal and failing tests, leak detection, and no-network enforcement. Re-run the same failing scenario without changing inputs to prove reproducibility; then vary one explicit failpoint or hypothesis when diagnosing.

Provide a host-only test command and an offline mode. Run focused tests and supported formatting/static checks. CI workflow edits, cache changes, target jobs, QEMU acceptance, and M30 criteria are outside this workstream; submit a focused CI proposal to the Integration Owner if a shared workflow change is needed.

## 13. State, commit, push, and report

Maintain only .dev/workstreams/test-plat-01/state.json using the current DF-01 schema. Record the live activation gate, actual status, checked commit, exact commands and outcomes, CI run/head SHA, failures and evidence, attempted and prohibited fixes, every Acceptance Criterion, dependency status, deferred items, and one exact next action. If unregistered, include a focused registration proposal and state that DF-01 verification does not validate an unregistered state. Never mark provider/product acceptance PASS because a fake passed.

Commit only assigned files on the Integration Owner-approved branch. Push only that owned branch, never main or another workstream branch. Verify remote head equals the commit and the owned worktree is clean. If assignment, gate, or remote permission prevents a push, finish all permitted local work, record the concrete blocker and next owner action, and do not push from an unrelated branch.

The final report must state workstream ID, branch, final HEAD SHA, remote equality, worktree clean/dirty, harness/API changes, each Acceptance result, focused test and CI results, remaining blockers, and exact Integration Owner action.

## 14. Git and worktree safety

Before editing, inspect branch, HEAD, remote, Git status/diff, and git worktree list. Reuse a suitable dedicated worktree for this stream and preserve all existing worktrees. If none exists, use the exact branch/base and procedure assigned by the Integration Owner. Never infer a safe base, change another workstream's branch, or use reset --hard, unreviewed clean, force checkout, rebase, or destructive recovery commands.
