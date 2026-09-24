# Wayback Activity Ledger Core

**Status:** foundational core implemented; full Wayback remains future work.

This document defines the independent Activity Ledger and snapshot-reference
core in `user/nagi-activity`. It does not change the M15 filesystem history
service, Files/Notes, AI runtime, or M17 browser work.

## Ledger and snapshots

The **Activity Ledger** records what happened: actions, transaction state
changes, action outcomes, revert attempts, snapshot references, and redaction
markers. It is append-oriented. Normal operation adds events; it does not
rewrite prior events.

The existing M15 `user/nagi-history` remains the bounded guest implementation
for its filesystem acceptance flow. It keeps small before/after byte buffers
for immediate undo. This new core is separate: it records state references and
does not change or replace M15 history.

A **snapshot** represents a captured state elsewhere. The ledger stores only a
`SnapshotReference` with an opaque storage locator, scope, creation time,
parent, reason, related transaction, and retention hint. It never copies file
contents into each activity event. A content hash, object version, snapshot
ID, or delta reference can be attached as before/after state metadata.

## Actors, actions, and provenance

Actors distinguish User, AI, Application, System, and Automation. Optional
identity metadata includes the logical `UserId`/`AppId`, an opaque actor ID,
the calling application, an AI model ID, and `delegated_for`. An AI action on
behalf of a user remains `actor = AI`; delegation is recorded separately.

Each action has a stable `ActionId`, typed action kind, target reference,
localization summary key, bounded-shape metadata values, before/after
references, reversibility classification, optional snapshot and authorization
references, and provenance. Parent action, correlation, causation, actor, time,
transaction, and retention are carried by the event context. Targets use
logical object or opaque IDs and a resource kind; they are not filesystem
paths.

Metadata is structured rather than an unrestricted JSON payload. Callers must
not put document bodies, credentials, API keys, hidden chain-of-thought, or
unredacted sensitive context in summaries or metadata. Summary keys are
localized by the consuming UI.

## Transactions and reversibility

A transaction starts with an actor and start time, gathers related action
events, and ends with Completed, Failed, or PartiallyCompleted. A later
append-only state event may mark it Reverted. Transaction summaries derive
their action list and reversible state from the ledger events.

Reversibility distinguishes FullyReversible, ReversibleWithSnapshot,
PartiallyReversible, Irreversible, and Unknown. The `RevertExecutor` boundary
receives a recorded action and an optional resolved snapshot reference. It
does not grant authority or perform filesystem operations by itself. Every
revert attempt, including rejection, failure, and partial success, is appended
as an event; an already-applied revert cannot run twice.

## Storage, query, and capability boundary

`ActivityStore` is the append/read persistence boundary. The supplied
`InMemoryActivityStore` is suitable for tests and orchestration fixtures; it
does not imply a durable database choice. Events have a schema version and
Serde/Postcard round-trip support so another backend can persist each event.
`ActivityLedger` exposes `begin_transaction`, `record_action`, transaction
completion/failure, `create_snapshot_reference`, `query`, `transaction_events`,
and `request_revert` / `complete_revert_request`. A revert request is written
before the executor runs and its result is a second event; the request ID lets
a backend executor use an idempotency key after interruption.

Existing `nagi-model` identities (`UserId`, `NodeId`, `AppId`,
`AppSessionId`, `SurfaceId`, `WorkspaceId`, `ObjectId`, and `TransactionId`)
remain the logical identity vocabulary and gain Serde representation. The new
action, event, correlation, snapshot, and storage-reference IDs are typed
opaque values. No shared event-ID allocator existed, so `ActivityIdSource` is
injected. `SequentialIdSource` is intended for in-memory tests; a durable Nagi
service must provide a collision-resistant namespace and persist its sequence.

Structured queries filter time range, actor, application, app session, Node,
workspace, action kind, target, transaction, and known-reversible actions.
Every public query, transaction enumeration, and event read requires an
`ActivityAccessPolicy` for query authorization and per-event filtering. The
ledger is a trusted service boundary: applications must not receive
unrestricted store access. Capability decision, capability, and
delegated-authority IDs are optional references only; this core does not
perform capability authorization.

Retention class, user-pinned state, and redaction markers leave room for
recent detailed history, older summaries, snapshots, and disposable events.
`RetentionStore` is the explicit physical-erasure extension for privacy or
retention policy. Cryptographic tamper proofing, a retention engine, access
control UI, natural-language search, and snapshot data storage are outside
this implementation.

## Validation and compatibility

The core rejects invalid timestamps, malformed localization/metadata keys,
oversized metadata values, duplicate event/action/transaction/snapshot IDs,
missing parent actions, actions recorded outside an in-progress transaction,
invalid transaction transitions, and unresolved parent snapshots. Revert
requests for irreversible, unknown, already reverted, or snapshot-missing
actions are recorded with a rejection outcome and never invoke the executor.

The crate is `no_std` plus `alloc`, has no filesystem or application
dependency, and is not wired into Files, Notes, Albert, M17, or the AI runtime.
The initial serialized event schema is version 1. Later schema evolution and a
durable store remain explicit follow-up work. This foundation does not mark
full M15, M22, or Wayback acceptance complete.

## Verification scope

The focused suite covers action and actor serialization, transaction status
and grouping, parent/correlation links, direct and delegated AI actors,
reversible/irreversible/partial/snapshot-required revert paths, authorized
queries, chronology and filters, snapshot linkage, redaction visibility, and
an in-memory three-file organize-and-revert fixture with no filesystem I/O.

## Relationship to existing M15 history

`user/nagi-history` remains the bounded M15 guest implementation used by its
acceptance flow. The new crate supplies a broader, target-independent model
and in-memory service for future integrations. It does not replace or rewrite
M15 behavior. Full AI integration and user-facing Wayback remain part of
future milestones.
