# Nagi OS 0.2 — SEARCH-CORE-01 Unified Search / Index Foundation

## 1. Goal

Implement the host-side Foundation for the Nagi OS 0.2 unified Search/Index core while Nagi OS 0.1 is still being completed.

This workstream exists to provide one stable search/index contract that future consumers can use across areas such as Files, Notes, Albert, Settings, installed apps, Activity, and Wayback without each product inventing its own incompatible index API.

The authoritative product/milestone requirements come from:

- `Nagi_OS_0.2_Codex_Implementation_Spec.md`

Also obey:

- `Nagi_OS_0.2_Parallel_Execution_Rules.md`
- `AGENTS.md`
- current DF-01 workstream ownership/state

## 2. Critical collision rule with Nagi 0.1 Search

Nagi OS 0.1 already has active Search/Semantic milestone work.

SEARCH-CORE-01 MUST NOT rewrite, replace, or destabilize that implementation while it is active.

Before coding, inspect the current 0.1 Search implementation and determine whether the master 0.2 specification expects:

1. reuse through an adapter;
2. extraction of a host-independent contract later; or
3. a separate 0.2 host-only foundation.

If a canonical search/index contract is already owned by the active 0.1 workstream, do not create a competing contract. Implement only the non-conflicting adapter/test/foundation surface allowed by the master specification, or record the workstream as blocked by ownership.

## 3. Suggested branch/worktree

If no existing remote branch owns this workstream:

- branch: `codex/0.2-search-core-01`
- worktree: `~/.codex/worktrees/search-core-01/NagiOS`

Base it on the current approved integration base discovered at execution time.

Do not modify the active 0.1 Search branch/worktree.

## 4. Pre-implementation audit

Before writing code:

- read the 0.2 master specification sections for Search, Workspace, Object/Action, Context, Files, Notes, Activity, Wayback, Home, Albert, and SDK;
- inspect current 0.1 Search/Semantic source and public contracts read-only;
- search for existing `SearchDocument`, `Index`, `Query`, `ObjectId`, provider, ranking, or result types;
- inspect `app-home-search`, `app-files`, `app-notes`, `app-activity-wayback`, and first-party integration branches read-only where useful;
- inspect Capability/Permissions contracts read-only;
- determine the exact ownership boundary before adding new types.

The result of this audit must be recorded in SEARCH-CORE-01 state/proposal.

## 5. Foundation scope

Implement only the host-side foundation that is both required by the master 0.2 specification and independent of the active 0.1 runtime.

Expected categories, when not already canonically owned elsewhere, include:

### 5.1 Stable search document contract

Provide typed, bounded, versionable data structures for an indexable document/object, including only fields justified by the master specification.

Typical concerns include:

- stable document/object identity;
- provider/source identity;
- title/display text;
- searchable text or token input;
- structured metadata/fields;
- update/version marker;
- visibility/authorization metadata through an adapter boundary;
- optional correlation/reference IDs to existing canonical objects.

Reuse existing canonical IDs instead of redefining them.

### 5.2 Provider/index boundary

Define a host-testable contract for:

- add/upsert;
- remove;
- replace/update;
- provider/source isolation;
- deterministic query;
- bounded result count;
- stable result identity;
- stale/duplicate update behavior.

Do not wire a production daemon, IPC endpoint, or target service before the activation gate.

### 5.3 Query and result contract

Implement the minimum query model required by the 0.2 master specification.

Prefer deterministic lexical/structured behavior for the Foundation unless the master spec explicitly requires vector/semantic behavior here.

Do not duplicate the 0.1 semantic runtime or model inference path.

Result objects should preserve enough provenance to identify source/provider and stable object identity without leaking private backend data.

### 5.4 Authorization seam

Search must not become a side channel around permissions.

Provide an injected/testable authorization or visibility seam that can fail closed.

Do not implement or fork the Capability/Permissions policy engine.

Host tests must prove that denied documents cannot appear in result payloads, counts, snippets, or metadata returned to the caller.

### 5.5 Reference backend

Provide a deterministic in-memory/reference backend sufficient to exercise the contract and tests.

A durable guest index, target daemon, production persistence, and large-scale optimization are deferred unless the current gate explicitly authorizes them.

### 5.6 Versioning and failure behavior

Where serialization/versioned snapshots are in scope, define explicit versioning and safe rejection of unsupported/corrupt input.

Never silently reinterpret unknown versions.

## 6. Required host tests

Add deterministic tests covering all implemented Acceptance, including at minimum where applicable:

- insert then query;
- upsert same identity;
- update/replacement behavior;
- delete/removal;
- duplicate/stale revision handling;
- multiple providers/sources;
- source scoping;
- bounded result count;
- stable ordering/tie behavior;
- empty query/empty index behavior;
- malformed/oversized input rejection;
- unauthorized document exclusion;
- denied metadata/snippet non-leakage;
- unknown/unsupported version rejection;
- deterministic results across repeated runs.

If ranking exists, tests must define deterministic tie-breaking.

## 7. Explicitly deferred before M30 + checkpoint

Unless the live repository explicitly opens the gate, do NOT implement:

- target/QEMU search daemon;
- guest durable search database/index;
- shared IPC runtime wiring;
- production Files/Notes/Activity/Wayback provider registration;
- GUI/Home/Search results UI;
- Albert runtime integration;
- global keyboard/search shell behavior;
- production capability enforcement wiring;
- semantic/model inference that belongs to the active 0.1 AI/Search milestones;
- changes to active 0.1 Search/Semantic code merely to satisfy this workstream.

## 8. Ownership constraints

Prefer a standalone host-side crate/module if the master specification permits it.

Do not change root workspace/lockfile, shared CI, shared IDL/ABI, or `.dev/workstreams.json` without explicit ownership.

If registration is not owned here, create:

- `.dev/workstreams/search-core-01/state.json`
- `.dev/workstreams/search-core-01/registration-proposal.json`

using the repository's current DF-01 schema and conventions.

## 9. Acceptance rule

SEARCH-CORE-01 may be marked Foundation `PASS` only when all host-side Acceptance defined for this Foundation by the master 0.2 specification is complete and tested.

If the master specification requires a target/runtime item for this workstream's own PASS, retain `PARTIAL` and clearly distinguish completed host Foundation from gated production/runtime Acceptance.

Do not claim Nagi 0.2 release acceptance.

## 10. Completion loop

Continue through implementation, tests, fixes, state update, commit, and push without stopping while in-scope work remains.

Before final commit:

- run focused tests;
- run formatter;
- run Clippy/lint with warnings denied where applicable;
- run valid DF-01/repository verification;
- run `git diff --check`;
- audit changed paths against ownership;
- update state with exact commands/results;
- commit only SEARCH-CORE-01 owned files;
- push the dedicated branch;
- verify remote HEAD equality;
- verify clean worktree.
