# Nagi OS 0.2 — CLIP-01 Clipboard / Data Transfer Foundation

## 1. Goal

Implement the host-side Foundation for Nagi OS 0.2 clipboard and application data-transfer semantics while Nagi OS 0.1 M17-M30 implementation continues independently.

The authoritative product/milestone requirements come from:

- `Nagi_OS_0.2_Codex_Implementation_Spec.md`

Also obey:

- `Nagi_OS_0.2_Parallel_Execution_Rules.md`
- `AGENTS.md`
- current DF-01 workstream ownership/state

CLIP-01 is intentionally a host-side contract/reference implementation first. Production shell/compositor/IPC integration is deferred until the release/integration gate allows it.

## 2. Suggested branch/worktree

If no existing remote branch owns this workstream:

- branch: `codex/0.2-clip-01`
- worktree: `~/.codex/worktrees/clip-01/NagiOS`

Base it on the current approved integration base discovered at execution time.

If the branch or responsibility already exists under another ID, resume/reuse it rather than creating a duplicate.

## 3. Pre-implementation audit

Before writing code:

- read all clipboard/data-transfer/drag-drop/action/object sections in the 0.2 master specification;
- search the repository for existing clipboard, pasteboard, data-offer, MIME/media-type, drag/drop, transfer, and selection contracts;
- inspect App SDK and Capability/Permissions contracts read-only;
- inspect canonical app/session/execution/object IDs and reuse them where required;
- verify that no active 0.1 milestone owns the same contract;
- document ownership and dependencies in CLIP-01 state/proposal.

## 4. Foundation scope

Implement the minimum safe, versionable, host-testable clipboard/data-transfer foundation required by the 0.2 master specification.

Expected categories, when not already canonically owned elsewhere, include:

### 4.1 Clipboard item and format model

Provide typed, bounded models for clipboard content and available representations.

Where required by the master specification, support concepts such as:

- clipboard revision/generation;
- one or more ordered items;
- representation/media type identifier;
- text/binary/reference payload distinction;
- source/origin metadata using existing canonical app/session/execution IDs;
- operation hint such as copy where appropriate;
- bounded metadata.

Do not redefine AppId, session IDs, capability principals, ObjectId, or storage handles.

### 4.2 Clipboard service contract

Provide a host-testable contract for the required operations, typically including:

- write/replace clipboard contents;
- read available formats/metadata;
- read a selected representation;
- clear;
- current revision/generation;
- stale-revision behavior where relevant.

Keep behavior deterministic and explicitly specify replacement semantics.

### 4.3 Reference implementation

Provide a deterministic in-memory/reference implementation for host acceptance.

The reference backend must not be presented as production persistence.

Clipboard history is not implied by the basic Foundation. Do not create a durable history store unless the 0.2 master specification explicitly assigns it to CLIP-01.

### 4.4 Authorization seam

Clipboard reads/writes must support an injected authorization boundary suitable for future Capability integration.

Do not fork the Capability/Permissions engine.

Host tests should prove fail-closed behavior for denied read, denied write, denied clear, and restricted representation access if the spec defines representation-level access.

### 4.5 Payload safety and bounds

Define explicit bounds and validation for any payloads/metadata handled directly by this Foundation.

Reject invalid/unsupported media type identifiers, malformed versions, and oversized inline payloads according to the master specification or conservative documented limits.

Do not silently truncate sensitive data.

### 4.6 Data-transfer semantics

If the master 0.2 specification assigns generic app-to-app data-transfer semantics to CLIP-01, expose them as a host-only contract without production IPC/compositor wiring.

If cut/move is represented, it must remain a non-destructive intent/hint at this layer unless another owner explicitly defines transactional move semantics. CLIP-01 must not directly delete source files or app data.

## 5. Required host tests

Add deterministic tests for all implemented Acceptance, including at minimum where applicable:

- empty clipboard read;
- write then read;
- replacement of old content;
- clear;
- multiple representations of the same item;
- multiple ordered items if supported;
- unsupported format behavior;
- invalid media type/metadata rejection;
- payload bound enforcement;
- revision/generation increment behavior;
- stale revision rejection if supported;
- denied write;
- denied read;
- denied clear;
- origin metadata preservation without granting authority from untrusted metadata;
- malformed/unsupported serialized version rejection if serialization is in scope;
- repeated deterministic behavior.

## 6. Security/privacy rules

Clipboard metadata supplied by an application is not proof of caller identity.

Caller authority must come from the injected trusted boundary, not from a claimed AppId/source field in clipboard data.

Do not log clipboard payload contents into Diagnostics, Activity, Wayback, AI context, or tests by default.

If structured diagnostics are required, emit only stable codes and safe bounded metadata.

Do not introduce cloud synchronization, cross-device clipboard, telemetry upload, or AI ingestion in this Foundation.

## 7. Explicitly deferred before M30 + checkpoint

Unless explicitly opened by current Integration Owner state, do NOT implement:

- compositor/window-manager clipboard ownership;
- keyboard shortcut handling;
- selection/primary-selection platform behavior;
- drag-and-drop GUI plumbing;
- production IPC transport;
- target/QEMU clipboard service;
- durable clipboard history;
- Wayback/Activity recording of payloads;
- cloud/cross-device clipboard;
- first-party app adoption;
- shell UI/toasts/paste menus;
- OS-wide capability enforcement wiring.

## 8. Ownership constraints

Prefer a standalone host-side crate/module if the master specification permits it.

Do not modify root workspace/lockfile, shared CI, shared IDL/ABI, compositor, shell, App SDK canonical IDs, Capability policy engine, or `.dev/workstreams.json` without explicit ownership.

If registration is Integration Owner-owned, create:

- `.dev/workstreams/clip-01/state.json`
- `.dev/workstreams/clip-01/registration-proposal.json`

using the current DF-01 schema/conventions.

## 9. Acceptance rule

CLIP-01 may be marked Foundation `PASS` only when every host-side Acceptance assigned to this Foundation by the master 0.2 specification is implemented and tested.

If its formal PASS requires target/runtime integration, retain `PARTIAL` and clearly mark the host Foundation as complete while the gated Acceptance remains deferred.

Do not claim overall Nagi 0.2 release acceptance.

## 10. Completion loop

Continue implementation -> test -> fix -> verify -> state update -> commit -> push without stopping while legal in-scope work remains.

Before final commit:

- run focused tests;
- run formatter;
- run Clippy/lint with warnings denied where applicable;
- run valid DF-01/repository verification;
- run `git diff --check`;
- audit changed paths against ownership;
- update state with exact commands/results;
- commit only CLIP-01 owned files;
- push the dedicated branch;
- verify remote HEAD equality;
- verify clean worktree.
