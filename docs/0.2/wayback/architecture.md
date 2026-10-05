# Wayback / AI Activity Ledger Foundation

Status: host-side contract and reference planner. This is not the Nagi runtime
Wayback service, a filesystem snapshot engine, or a restore executor.

## Scope and ownership

The v1 reference implementation is the standalone Rust package
`crates/nagi-wayback`. It is intentionally outside the root Cargo workspace:
DF-01 keeps the root manifests and dependency lock integration-owned, and this
workstream must not connect to the product runtime before the release gate.
The package depends only on Serde, JSON, and SHA-256 and runs on the host for
contract tests. It does not access host files as Nagi objects.

The existing M15 `user/nagi-history` remains the bounded guest history and
undo implementation. Wayback v1 adapts its concepts into a larger semantic
contract; it neither changes nor replaces that code. Activity/Wayback UI,
App SDK, and Capability identity/policy remain owned by their workstreams.

## Activity entry and provenance

`src/model.rs` defines `ActivityEntryDraft` and the persisted `ActivityEntry`.
An entry has a caller-stable `EntryId`, Unix-millisecond timestamp, generic
actor metadata, optional external app/principal references, typed action and
opaque target, transaction link, initiating event, parent/cause/correlation
links, a bounded human summary, structured metadata, reversibility, snapshot
and restore-point references, and integrity metadata.

Actors distinguish user, AI, system, application, automation, and remote
device. Principal IDs and app IDs are references only. The model neither
resolves identity nor makes capability decisions. An AI entry uses the same
actor and provenance model as other work; optional model metadata is an
identifier, never prompt text or hidden reasoning.

Entries can point only to earlier parent/cause entries. Transaction and
correlation references make an AI request → AI action → app/tool action →
resulting state change chain queryable. Target IDs are opaque identifiers,
not host paths.

## Append ledger and transactions

`src/ledger.rs` implements append, ordered query/traversal, transaction
projection, serialization, deserialization, and integrity verification.
`src/store.rs` defines `LedgerStore`; `InMemoryLedgerStore` is the deterministic
host fixture. A future persistent implementation must append the record and
its checkpoint atomically, and keep the checkpoint independently enough to
detect tail truncation.

The ledger stores transaction start and finish records rather than rewriting
prior transaction rows. A transaction view derives ordered entry IDs and one
of `open`, `committed`, `aborted`, or `partially_applied`. Abort is rejected
after an action was recorded; a transaction with recorded work and a failure
must remain explicitly partial. Atomicity expectation is metadata and does not
claim a cross-service ACID transaction.

Each record hash is SHA-256 over deterministic JSON with the schema version,
sequence, previous hash, and canonical struct payload. Metadata maps use
`BTreeMap`. Activity entries carry the same sequence/hash as the enclosing
record. Exported data includes a record-count/head-hash checkpoint. Verification
rejects edits, reorderings, broken links, unsupported versions, and truncated
record lists whose checkpoint remains. This detects corruption; it is not an
authenticated tamper-proof log. An attacker able to rewrite the ledger and its
external anchor is outside this v1 integrity contract.

All externally serialized records are versioned. V1 has no implicit migration.
Unknown versions and unknown enum variants fail closed. A future incompatible
shape must add an explicit migration that preserves original evidence; it must
not silently drop or reinterpret fields.

## Reversibility and short-term Undo

`Reversibility` distinguishes:

- `exact_reversible`: an exact inverse can be described;
- `compensation_required`: a named compensation contract is required and is
  not an exact rollback;
- `snapshot_required`: a referenced, compatible snapshot must resolve;
- `irreversible`: restoration is blocked;
- `unknown`: restoration is blocked until assessed.

`undo_candidates` derives recent committed transactions from the ledger and
returns a plan plus any blocker reasons. It does not maintain a separate
application-local command stack. Partial, open, aborted, empty, irreversible,
unknown, missing-snapshot, and incompatible-snapshot cases are not presented
as executable undo.

## Snapshot metadata and restore points

`SnapshotManifest` describes an opaque snapshot held elsewhere: ID, app/object/
workspace/system scope, creation time, source transaction and ledger boundary,
content digest, optional item/byte counts, backend identifier, compatibility
version, and optional parent snapshot. It contains no content bytes. The
`SnapshotStore` interface resolves manifests only; the in-memory implementation
is a deterministic planning fixture, not a snapshot backend. When a manifest
names a parent snapshot, the planner resolves that dependency first and blocks
missing, incompatible, or cyclic ancestry instead of offering a partial chain.

A `RestorePoint` combines a user-readable label, one or more snapshot IDs,
scope, compatibility version, and the ledger boundary it represents. Its
boundary must resolve to the exact earlier chain hash. The planner checks
snapshot existence, digest shape, scope, compatibility, and that a
snapshot-required action's snapshot predates that action.

## Restore planning and execution boundary

`src/restore.rs` returns a versioned `RestorePlan` with target, ordered steps,
required snapshots, affected scopes, executable decision, and explicit blockers.
Plans may contain exact undo, compensation, or snapshot-restore steps. An
irreversible/unknown action, partial transaction, missing snapshot/reference,
bad manifest, scope mismatch, stale snapshot, or compatibility mismatch blocks
the plan. Restore-point plans reverse later exact actions outside snapshot-covered
scopes, retain required compensation steps, and block uncommitted transactions or
snapshot-required scopes that the restore point did not capture.

`executable = true` means only that the plan is structurally safe to offer to a
future authorized executor. `ExecutionBoundary::PlanOnlyFutureAuthorizedExecutorRequired`
is always present. This crate has no executor, performs no restore, and makes no
permission decision. Any eventual executor must re-check authority and current
state, treat the plan as untrusted input, and append the restore itself as a
new activity/transaction.

## Privacy and payload handling

The ledger accepts metadata and opaque references, not file/clipboard payload
bytes. Metadata fields declare sensitivity and disposition; sensitive/secret
raw values are rejected, while redacted, omitted, unavailable, and
snapshot-reference states remain expressible. `HumanSummary::Public` means the
caller has attested the bounded text is safe to retain. Callers must use a
redacted or omitted summary when that assertion cannot be made. This is a
storage contract, not DLP or encryption.

## Query model

`ActivityQuery` filters time range, actor kind, principal, app, target, action
type, transaction, correlation, cause/parent, reversibility, and restore point.
Results default to append order; newest-first is explicit. The fields preserve
the metadata needed for a future deterministic query builder to translate a
request such as “restore what Notes removed yesterday evening” into temporal,
actor, app, target, and restore-plan filters. No LLM is called here; any future
natural-language interpretation must be validated before it reaches this API.

Short-term Undo is derived from recent committed transactions. Long-term
history uses the same event IDs, transactions, provenance, scopes, and snapshot
manifest references plus a future storage/retention provider. A list of recent
commands is not a separate source of truth.

## Deferred runtime integration

The following hooks are intentionally not connected before Nagi 0.1 M30 PASS
and an explicit 0.2 integration checkpoint:

| Hook | Future boundary | Acceptance when integration opens |
|---|---|---|
| Filesystem changes | Adapter supplies opaque object/version references to `ActivityEntryDraft`; no file bodies | Append/query real guest changes; verify provenance, snapshot requirement, and denial behavior without host filesystem substitution |
| App state | Adapter supplies external AppId/session IDs and state snapshot refs | App/workspace plan scope is exact; no app-private API or window-layout identity |
| AI/tool actions | Adapter maps request, actor, parent/cause/correlation, transaction, and result refs | Full causal chain and permission denial are attributable; no chain-of-thought or authority is added |
| Snapshot backend | `SnapshotStore` metadata contract and manifest validation | Guest-owned snapshot bytes/digests are real; missing, corrupt, stale, and incompatible refs block |
| Restore executor | Consumes previewed plan after deterministic capability/policy checks | Re-checks current state/authority; rejected and successful restore attempts append new activity; no duplicate execution |
| Activity/Wayback UI | Separate first-party app consumer of query and plan contracts | Shows transaction groups and explicit blockers; never shows fake Undo |
| Principal lookup | Adapter resolves external references; no local identity clone | Missing identity and denied permission remain explicit; rights cannot be strengthened |

These are integration checkpoints, not unfinished host-side implementation
items. The machine-readable workstream state tracks them separately.

## Schemas and verification

`schemas/wayback/contracts-v1.schema.json` validates the versioned ledger,
activity entry, transaction view, snapshot manifest, restore point, and restore
plan. `examples/contract_fixtures.rs` supplies real Serde output for schema
validation. `tests/wayback/verify.sh` runs this crate's unit/integration tests,
formatting, Clippy, schema checks, the existing M15 history regression, and
DF-01 `dev verify`. The schema harness uses the pinned `jsonschema` requirement
in `tests/wayback/requirements.txt`; when Python cannot import it, the harness
installs it into a temporary directory and removes that directory on exit.

The root `Cargo.toml` and `Cargo.lock` remain unchanged in this branch. The
standalone package and the one appended workstream registry row are explicit
integration review points; the integration owner must reconcile registry and
workspace ownership before any product consumer imports this package.
