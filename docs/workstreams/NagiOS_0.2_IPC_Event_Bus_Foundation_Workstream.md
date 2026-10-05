# Nagi OS 0.2 — IPC / Event Bus Foundation Workstream

## 1. Workstream

- **Workstream ID:** `IPC-01`
- **Name:** IPC / Event Bus Foundation
- **Proposed branch:** `codex/0.2-ipc-event-bus`
- **Recommended worktree:** `/Users/tozawa/Developer/NagiOS-0.2-ipc-event-bus`
- **State:** `.dev/workstreams/ipc-event-bus/state.json`
- **Target:** Nagi OS 0.2 host-testable foundation
- **Initial status:** `NOT_STARTED`

The repository already has `SVC-IPC-01` at `docs/workstreams/NagiOS_0.2_System_Service_IPC_Contract_Foundation_Workstream.md`. `IPC-01` must reuse or adapt that service-call contract and must not introduce a competing `ServiceId`, request envelope, registry, or transport. Its distinct focus is the transport-neutral message/event contract and one-way event bus, with compatibility adapters to `SVC-IPC-01`.

This document does not activate runtime work. Nagi 0.2 runtime streams remain `NOT_STARTED` until Nagi 0.1 M30 passes and an explicit integration checkpoint opens this stream.

## 2. Purpose and scope

Define a typed, bounded IPC foundation usable by service clients/providers and a one-way event bus. Keep public APIs transport-neutral so later Nagi Channel IPC or another approved transport can replace the in-memory reference transport without changing service contracts.

Required capabilities:

- Typed message envelope with protocol/contract version, `ServiceId`, operation/event ID, caller principal adapter, bounded payload metadata, and optional tracing fields.
- Request/response with correlation ID, structured result/error, finite timeout/deadline, and cancellation propagation.
- One-way event publish/subscribe with stable event IDs, bounded queues, explicit delivery semantics, and unsubscribe/cancellation.
- Version negotiation and deterministic unsupported-version rejection.
- Permission/capability hook receiving caller, service, operation/event, and target context.
- In-memory transport and mock transport for unit/contract tests.
- Malformed message rejection, strict payload limit, and deterministic backpressure behavior.

## 3. Non-goals

- A second service discovery registry or duplicate service-call framework already owned by `SVC-IPC-01`.
- Kernel IPC primitive changes, new syscalls, or a new shared IDL/ABI without Integration Owner approval.
- Network RPC, cross-device transport, distributed consensus, internet gateway, or remote execution.
- Product service behavior (Files, Wayback, Search, Models, app management, or browser).
- Permission policy/grant storage or user-facing permission prompts.
- M18 main/A/B/C implementation or browser/Servo/Mesa/relibc work.

## 4. Dependencies and ownership

- `SVC-IPC-01`: source contract for service IDs, request/response, versioning, authorization context, errors, and provider/client adapters. Add compatibility tests; do not fork its semantics.
- Capability / Permission: public authorization hook if available, otherwise a narrow adapter that denies by default when required context is absent.
- Diagnostics: stable event/error hooks through a sink, without a runtime dependency on diagnostics internals.
- Activation gate: Nagi 0.1 M30 `PASS` plus an explicit Integration Owner checkpoint.

Allowed after activation: `crates/nagi-ipc-event-bus/**` (or repository-approved equivalent), `tests/ipc-event-bus/**`, `.dev/workstreams/ipc-event-bus/**`, and this specification. Shared `Cargo.toml`/`Cargo.lock`, `.github/workflows/**`, `.dev/workstreams.json`, `.dev/schemas/**`, shared IDL/ABI, other stream state, and M18/browser paths remain forbidden unless the Integration Owner grants ownership. Put required shared changes in a focused proposal and continue independent work.

## 5. Architecture and API/data model

Prefer small typed values and explicit bounds. Suggested model:

- `ServiceId` and service contract version: reuse the canonical `SVC-IPC-01` type.
- `MessageEnvelope<T>` or equivalent: protocol version, service, operation/event ID, caller, correlation ID, payload length/encoding, and optional deadline/trace context.
- `Request<T>` / `Response<T>`: typed payload and structured error; no prose parsing.
- `EventEnvelope<T>`: publisher, event ID/version, sequence/correlation, bounded payload, and privacy classification.
- `CancellationToken`/request cancellation: cooperative, idempotent, and observable by the provider.
- `Transport` adapter: request/reply and event publish/subscribe, with an in-memory reference implementation and deterministic mock.
- `AuthorizationHook`: allow/deny result for the full caller/service/operation context; permission policy remains outside this crate.

Set documented maximum envelope and payload sizes before allocation/decoding. Reject over-limit sizes before copying or decoding. Version negotiation must be explicit and deterministic; unsupported major versions fail with a typed error. Cancellation, timeout, permission denial, malformed input, queue-full/backpressure, missing service/operation, and provider failure must be distinguishable.

Event delivery must document whether it is at-most-once or another bounded guarantee. It must never claim durable delivery. Slow subscribers must not block unrelated publishers indefinitely; define queue limits, overflow response/drop policy, and observability.

## 6. Failure and recovery

- Malformed headers, unknown versions, invalid IDs, truncated data, and payloads over the configured maximum are rejected before handler dispatch.
- Timeout cancels outstanding provider work where supported and returns a distinct deadline error; late replies are discarded by correlation identity.
- Cancellation is idempotent; provider completion after cancellation cannot complete the caller twice.
- Transport/provider failure returns a structured error and releases queued resources.
- Event queue saturation follows the documented bounded policy and emits diagnostics; it cannot cause unbounded memory growth or indefinite blocking.
- Authorization denial occurs before provider invocation and is never converted to success by a mock or fallback.
- Transport replacement preserves envelope/version/error semantics through conformance tests.

## 7. Security and capability hooks

Pass caller principal, service ID, operation/event ID, and target context to the authorization hook. Deny by default if principal/context cannot be established. Do not let an event subscriber gain publisher authority or strengthen transferred handles. Enforce payload bounds before deserialization; avoid arbitrary code/object deserialization. Mark sensitive event payloads and keep secret material out of diagnostics and default export/inspection.

## 8. Diagnostics hooks

Expose stable codes for malformed envelope, unsupported version, payload too large, timeout, cancellation, permission denial, provider unavailable, transport failure, queue saturation, and subscriber failure. Include correlation and service IDs where safe; do not log payload contents, credentials, or capability secrets. Diagnostics sink errors must not break IPC cleanup or event delivery semantics.

## 9. Acceptance Criteria

- [ ] Typed envelope, request/response, and one-way event APIs are documented and implemented.
- [ ] Service and correlation IDs are validated and stable; existing `SVC-IPC-01` identities are reused.
- [ ] Version negotiation accepts documented compatible versions and rejects unsupported versions deterministically.
- [ ] Cancellation and timeout/deadline behavior are bounded and testable.
- [ ] Malformed headers/payloads and oversize messages are rejected before handler dispatch.
- [ ] Permission hook receives full context; deny prevents invocation.
- [ ] In-memory and mock transports exercise the same public contract.
- [ ] Event subscriptions support bounded queues, unsubscribe/cancel, and explicit overflow behavior.
- [ ] No competing service registry, service ID, or SVC-IPC envelope is introduced.
- [ ] M18 main/A/B/C and unrelated workstream files remain untouched.
- [ ] Workstream state/registration proposal follow DF-01 ownership and schema.
- [ ] Focused checks pass; changes are committed and pushed only on the owned branch.

## 10. Tests and CI

Test valid/invalid service and event IDs, envelope encode/decode or validation, version negotiation, success/error request-response, correlation isolation, timeout, cancellation before/during/after provider execution, malformed/truncated/oversize messages, permission allow/deny, transport failure, duplicate subscriptions, unsubscribe, queue saturation, slow subscribers, and diagnostics failure. Add compatibility fixtures against `SVC-IPC-01`; use no network access.

Run focused package tests, formatting, Clippy with warnings denied where supported, and schema/contract checks. Shared CI or IDL changes require Integration Owner proposals, not edits from this workstream. M18 browser acceptance is out of scope.

## 11. State, commit, and push

Update only `.dev/workstreams/ipc-event-bus/state.json` using the current DF-01 state schema. Record real status, gate, checked commit, commands and outcomes, failures/evidence, compatibility dependency status, acceptance items, and next action. If registration is absent and CLI verification rejects the state, write a focused registration proposal under `.dev/workstreams/ipc-event-bus/` and record the limitation; never edit the shared registry or claim unverified acceptance.

Commit only owned changes on the assigned branch; push reviewable checkpoints and verify remote SHA equality, clean worktree, and CI outcome. Do not merge to `main` without explicit direction.

## 12. Git and worktree safety

Inspect status, branch, HEAD, remotes, and `git worktree list` first. Reuse and preserve an existing dedicated IPC worktree. Do not reset, clean, force-checkout, rebase, or switch branches in another stream's worktree. If none exists, follow the Integration Owner-approved branch/worktree procedure and do not guess a base SHA.
