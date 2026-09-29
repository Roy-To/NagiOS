# Nagi OS 0.2 — Background Job / Task Scheduler Foundation Workstream

## 1. Workstream

- **Requested ID:** JOB-01
- **Registry ID:** job-01
- **Name:** Background Job / Task Scheduler Foundation
- **Assigned branch:** codex/0.2-job-01
- **Worktree:** /Users/tozawa/.codex/worktrees/job-01/NagiOS
- **State:** .dev/workstreams/job-01/state.json
- **Target:** Nagi OS 0.2 provider-neutral, host-testable job foundation
- **Initial status:** NOT_STARTED

The proposal baseline is commit 198b9f60b41eba14e74c2c04f51ccdcb4030adf9. At that baseline M17 is BLOCKED and M30 is NOT STARTED. This specification is preparatory and does not authorize production scheduling, system-service startup, or product runtime work.

## 2. Purpose and background

Define a bounded provider-neutral job contract that can later support Automations, Update, Search indexing, Wayback cleanup, model downloads, and other background maintenance. The foundation must make ownership, triggers, retries, cancellation, dependencies, persistence, recovery, and idempotency explicit while allowing deterministic host-only tests.

The scheduler coordinates registered work; it does not own domain behavior, grant authority, or promise exactly-once execution across crashes.

## 3. Scope

- Stable JobId and owner/principal reference, with distinct job type/handler identity and version.
- Trigger abstraction for explicit enqueue, lifecycle event, and approved scheduled trigger descriptions. No user-facing cron editor or hidden host cron dependency.
- Priority and fair ordering policy with deterministic tie-breaking.
- Constraints such as deadline, timeout, maximum runtime, permitted execution conditions, and offline/network policy.
- Typed state model for queued, waiting-on-dependency, running, pause-requested, paused, retry-wait, succeeded, failed, cancelled, timed-out, and interrupted/recovery-required states.
- RetryPolicy with finite attempts, deterministic backoff bounds, retryable failure classification, and no implicit infinite retry.
- enqueue, start, cancel, pause, resume, query, and progress-observation abstraction. A handler that cannot pause must return a typed unsupported result.
- Job dependency graph with cycle detection and explicit behavior for failed/cancelled prerequisites.
- Global and per-owner concurrency limits, bounded queues, and fairness/starvation policy.
- JobStore abstraction for versioned, atomic metadata/state persistence. Executable closures and credentials are never persisted.
- Restart/crash recovery policy with at-least-once semantics only where a handler is idempotent or resumes from a verified checkpoint.
- Idempotency/deduplication keys scoped to owner, handler, and request; explicit retention and replay rules.
- Cooperative cancellation token propagated to handlers and child work.
- Capability/permission hook for enqueue, inspect, cancel, pause/resume, and handler execution.
- Diagnostics/progress hooks with bounded safe metadata and deterministic host scheduler/test clock.

## 4. Non-goals

- Full Automations product, user-facing scheduler or cron UI.
- Cloud queue, distributed scheduler, multi-node coordination, or exactly-once execution claim.
- Email/calendar integration, mail sync, network-backed job providers, or domain-specific job implementations.
- Kernel scheduler changes, kernel work queues, or host OS background daemon behavior presented as Nagi behavior.
- Arbitrary shell command execution or deserializing persisted closures.
- Owning Update, Search, Wayback, model, notification, or app lifecycle product behavior.

## 5. Dependencies and activation gate

- **Required development contract:** development-foundation DF-01 state, ownership, and verification rules.
- **Authorization:** capability-permissions owns principal and authorization decisions. Its current state is PARTIAL; use an adapter/test fake until the exact public contract is integrated. Never bypass or invent a permissive authority path.
- **Observability:** diagnostics owns shared event vocabulary and sinks. Use a narrow adapter and safe local recorder until a versioned contract is available.
- **Execution/provider boundary:** the scheduler owns job metadata and orchestration only. Handler providers remain separate; future IPC integration consumes an approved SVC-IPC contract and must not create a second service bus.
- **Before activation:** typed contracts, deterministic in-memory scheduler, no-op/test handlers, and host-only reference behavior may be prepared only on an assigned branch and within approved paths. Such work must not start product jobs, call host daemons, perform external effects, or claim production runtime acceptance.
- **Runtime/integration gate:** persisted production queue, system-service wiring, real provider handlers, target scheduling, or shared workflow/workspace integration requires Nagi 0.1 M30 PASS and an explicit Integration Owner checkpoint naming JOB-01. A stricter registry gate takes precedence.

The registration proposal should list only registered hard dependencies. Unregistered future consumers/providers are contract relationships, not hard registry dependencies until their IDs and boundaries are approved.

## 6. Allowed and forbidden edits

Proposed allowed paths, subject to Integration Owner assignment:

- crates/nagi-job-scheduler/**
- tests/job-scheduler/**
- .dev/workstreams/job-01/**
- this specification

Do not edit without explicit Integration Owner ownership:

- Root Cargo.toml, root Cargo.lock, .github/workflows/**, .dev/workstreams.json, .dev/schemas/**, shared IDL/ABI, or generated bindings.
- kernel/**, loader/**, user/**, sdk/**, third_party/**, out/**, target/**, docs/implementation_status.md, or any other workstream's implementation/state/worktree.
- Update, Search, Wayback, app, notification, or model runtime product paths.
- Any file outside assigned owned paths.

A standalone host-only crate may keep its own manifest inside the owned directory. Root workspace membership, shared dependency changes, schemas, IDL, and CI changes require focused Integration Owner proposals under .dev/workstreams/job-01/.

## 7. Architecture and API/data model

Use a transport- and provider-neutral orchestration core:

- JobId: validated opaque ID, unique within the store.
- JobOwner: trusted principal and optional user/profile reference resolved through adapters; never accept an owner string as authority.
- JobKind / HandlerRef: stable provider-owned name and version; persisted records identify a handler, not a closure or executable path.
- Trigger: Manual, Lifecycle, or Scheduled descriptor with versioned semantics. Schedule evaluation uses injected clock/calendar adapters and does not spawn an OS cron process.
- Priority: bounded levels with documented fairness and deterministic FIFO/sequence tie-break.
- Constraints: deadline, timeout, resource class, permitted execution conditions, and offline/network requirements.
- JobState: explicit transitions; invalid transitions return typed errors.
- RetryPolicy: max attempts, initial/max delay, multiplier or bounded deterministic sequence, retryable error classes, and terminal action.
- JobDependency: prerequisite JobId and success condition; reject cycles and missing dependency references before enqueue.
- JobRecord: schema version, IDs, owner, handler ref, trigger, priority, constraints, state, attempt count, timestamps, dependency IDs, idempotency key, checkpoint reference, and bounded progress summary.
- JobStore: atomic put/update/list/recover contract with compare/revision check so two workers cannot claim the same record.
- JobHandler: registered provider API accepting bounded input, cooperative cancellation, checkpoint/progress sink, and least-authority context.
- Scheduler: deterministic selection, concurrency accounting, dependency gate, timeout/cancellation propagation, retry decision, persistence, and recovery reconciliation.
- IdempotencyKey: owner/handler/request-scoped value with defined deduplication window and result behavior; exactly-once is not promised.
- DeterministicScheduler: host-only scheduler with virtual time, fake store, and no external side effects.

Queue and progress payloads must be bounded. Persist only versioned metadata and explicitly approved checkpoint references. Do not persist executable code, secrets, ambient capabilities, or arbitrary closures.

## 8. Failure and recovery

- Invalid state transitions, missing handlers, unknown job types, dependency cycles, and unsupported schema versions fail with stable typed errors before scheduling.
- A dependency failure follows a documented policy: block, cancel dependent, or explicitly allow an alternate prerequisite outcome. Never silently treat failure as success.
- Retry occurs only for classified retryable outcomes and only while attempts/deadlines remain. Backoff is bounded and driven by the injected clock.
- Cancellation is idempotent and cooperative. The scheduler records cancellation request and final handler outcome separately; cancellation does not claim rollback of completed side effects.
- Timeout cancels child work, records deadline expiry, and avoids starting a retry after the deadline.
- On restart, a previously running job becomes recovery-required. Resume only when handler idempotency/checkpoint contract allows it; otherwise require explicit retry/repair and preserve failure evidence.
- Deduplication uses an atomic store check so concurrent duplicate enqueue does not create multiple active jobs within the defined scope.
- Store errors preserve the last committed record; partial updates are rejected or repaired from a validated prior snapshot.
- Queue saturation returns a stable bounded-capacity error and diagnostic event rather than allocating without limit.
- Diagnostics or progress sink failure cannot change authorization, job state, or cancellation semantics.

## 9. Security and capability hooks

- Authorize enqueue and ownership before accepting a record; authorize read/list separately from cancel, pause/resume, and administrative inspection.
- Resolve handler execution authority freshly for each run. A persisted JobOwner, retry, or restart does not preserve or strengthen capabilities.
- Pass least-authority scoped context to each handler; no global broker token or ambient owner privilege.
- Re-check current policy before retry/recovery after a permission change.
- Cancel and inspect requests must match the job owner or a separately authorized operator principal.
- Validate queue bounds and payload size before decoding; reject arbitrary shell commands and code-like handler payloads.
- Record safe IDs and reason codes only; redact user content, credentials, and capability secrets.

## 10. Diagnostics and observability hooks

Emit stable structured events for enqueue accepted/rejected, deduplication, dependency blocked/released, start, state transition, progress checkpoint, retry scheduled/exhausted, pause/resume, cancellation requested/completed, timeout, recovery decision, handler missing/failure, permission denial, queue saturation, and store corruption. Include JobId, handler reference, attempt, correlation ID, and bounded reason/progress codes. Never emit full job input or secret data. Progress delivery is rate- and size-bounded.

## 11. Acceptance Criteria

- [ ] JobId, owner, trigger, priority, constraints, state, retry, dependency, and handler-reference models are typed and versioned.
- [ ] enqueue/start/cancel and pause/resume abstraction have deterministic results and authorization hooks.
- [ ] Retry/backoff, deadlines, timeout, and cancellation propagation are bounded.
- [ ] Dependency ordering, failure policy, and cycle rejection are implemented and tested.
- [ ] Global/per-owner concurrency bounds, queue limits, and fairness are explicit.
- [ ] Job metadata persistence is abstracted and atomic; executable closures/secrets are excluded.
- [ ] Crash/restart recovery does not claim exactly-once and resumes only idempotent/checkpointed handlers.
- [ ] Idempotency/deduplication behavior is atomic, scoped, and covered by concurrent duplicate tests.
- [ ] Capability denial blocks enqueue/read/cancel/handler execution as appropriate; retries reauthorize.
- [ ] Diagnostics and progress are bounded, correlated, and redacted.
- [ ] Deterministic host scheduler runs offline and invokes only test handlers without host daemon or network side effects.
- [ ] No product Automation, Update, Search, Wayback, mail/calendar, kernel scheduler, or unrelated workstream code is changed.
- [ ] DF-01 state/proposal ownership is followed; shared registry/schema/root manifest/CI files remain untouched.
- [ ] Focused checks, owned-branch commit/push, remote equality, clean worktree, and CI status are recorded when branch/gates permit.

## 12. Tests and CI

Use a deterministic virtual clock and fake store/provider to cover model validation, transition table, enqueue dedupe races, dependency DAG/cycle/failure, priority fairness, global/per-owner concurrency, retry and max attempts, deadlines, timeout, cancellation propagation, pause/resume support and unsupported handlers, store interruption/corruption, crash/restart reconciliation, non-idempotent handler recovery, permission revocation before retry, queue saturation, progress bounds, diagnostics sink failure, and offline execution.

Run focused tests, supported formatting/static checks, and state/proposal validation. CI must be a host-only path with network disabled at test runtime. Do not edit shared CI workflows; submit a focused proposal when integration is needed. QEMU/target and product-provider acceptance remain gated and are not replaced by scheduler fakes.

## 13. State, commit, push, and report

Maintain only .dev/workstreams/job-01/state.json in the current DF-01 schema. Record live gate status, actual workstream status, checked commit, exact commands/results, CI run/head SHA, failure classes/evidence, attempted/prohibited fixes, acceptance checklist, dependency contract versions/status, deferred product handlers, and exact next action. If unregistered, prepare a focused registration proposal under .dev/workstreams/job-01/ and state that the root verifier excludes the state. Keep PARTIAL or BLOCKED where evidence requires it; do not mark production scheduling PASS from host-only fakes.

Commit only assigned files on the Integration Owner-approved branch. Push only that branch after a reviewable checkpoint and verify remote SHA equality and worktree cleanliness. If branch assignment, activation gate, or remote access prevents pushing, finish all permitted work, record the exact blocker and owner action, and do not use another stream's branch or main.

The final report must state workstream ID, branch, final HEAD SHA, remote equality, worktree clean/dirty, scheduler/API changes, Acceptance results, focused tests and CI, remaining blockers, and the exact Integration Owner next action.

## 14. Git and worktree safety

Before editing, inspect AGENTS.md, job state/proposal, registry, relevant contracts, implementation, Git status/diff, branch, HEAD, remote, and git worktree list. Reuse an existing dedicated JOB-01 worktree if present. Otherwise use only the approved branch/base procedure. Preserve all other worktrees; never reset, clean, force-checkout, rebase, or switch another stream.
