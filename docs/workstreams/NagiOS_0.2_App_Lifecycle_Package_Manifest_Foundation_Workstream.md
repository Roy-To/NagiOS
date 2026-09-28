# Nagi OS 0.2 — App Lifecycle / Package Manifest Foundation Workstream

## 0. Workstream identity

- **Workstream ID:** `APP-LC-01`
- **Recommended branch:** `codex/0.2-app-lifecycle-manifest`
- **Primary scope:** Nagi 0.2 application identity, package manifest contract, lifecycle state machine, validation, host-side reference runtime, tests and developer documentation
- **Repository:** `/Users/tozawa/Developer/NagiOS`
- **Execution model:** dedicated worktree / dedicated branch
- **Default status at start:** `PLANNED` or the actual state found in the repository

This document is the mandatory implementation specification for this workstream. The purpose is not to produce only a design, audit, TODO list, or proposal. Continue through implementation, tests, failure isolation, fixes, re-tests, workstream state updates, commit and push as far as the actual repository permits.

---

## 1. Goal

Create the Nagi 0.2 foundation that defines **what an application is** and **how the system manages its lifecycle**, without coupling the implementation to M18 browser internals or to one specific first-party app.

The result should provide a stable contract for:

- application identity;
- semantic version metadata;
- package metadata;
- executable/entrypoint declaration;
- required and optional capabilities;
- declared system-service dependencies;
- lifecycle transitions;
- launch / activate / suspend / resume / terminate semantics;
- validation and deterministic error reporting;
- forward-compatible manifest evolution;
- host-side tests and reference behavior.

This workstream should make it possible for future Nagi first-party and third-party applications to target one common application contract.

---

## 2. Non-goals / prohibited scope

Do **not** expand this workstream into unrelated implementation.

### Must not modify unless an unavoidable build fix is strictly local

- M18 main browser implementation;
- M18-A / M18-B / M18-C implementation;
- Servo / Mesa / relibc browser bring-up code;
- M19+ milestone product implementation;
- Files, Notes, Terminal, Activity, Wayback, Search or other first-party app feature code;
- Capability / Permission workstream internals beyond consuming its published/public contract;
- Model Runtime internals;
- Diagnostics internals;
- updater/install transaction implementation owned by `UPD-01`;
- shared registry ownership belonging to the Integration Owner.

### Shared registry rule

If `.dev/workstreams.json`, its schema, or another integration-owned registry must eventually be changed, **do not edit it directly unless the repository explicitly grants this workstream ownership**.

Instead:

1. implement and validate the workstream independently;
2. update this workstream's `state.json`;
3. add a machine-readable `registration-proposal.json` or equivalent proposal in this workstream's owned area;
4. document the exact integration action required from the Integration Owner.

A shared-registry ownership restriction alone must not stop the rest of the implementation.

---

## 3. Required repository inspection before editing

Inspect the actual repository and treat it as authoritative over old chat reports.

At minimum review:

- `AGENTS.md` and nested `AGENTS.md` files affecting edited paths;
- `docs/0.2/DEVELOPMENT_ARCHITECTURE.md`;
- relevant Nagi 0.2 specifications;
- `.dev/workstreams.json` and schemas, read-only unless owned;
- existing workstream state files;
- Capability / Permission public types and docs if present;
- App SDK or app-host related code if already implemented;
- existing manifest/package metadata formats anywhere in the tree;
- `tools/nagi-cli` commands related to development/application/package handling;
- existing test and CI conventions;
- current Git branch, worktree and dirty state.

Preserve unrelated work. Do not reset, checkout away, discard or rewrite other workstreams' changes.

---

## 4. Required architecture

Prefer a small, dependency-light Rust foundation that can be used by host tools and later by target/runtime code.

Use existing repository conventions where they exist. If no suitable crate exists, a dedicated crate or module may be added in a location consistent with the repository architecture.

### 4.1 Application identity

Implement a strongly typed application identity contract.

Minimum requirements:

- stable `AppId` or repository-equivalent type;
- canonical textual representation;
- validation rules;
- no path traversal or filesystem semantics embedded in IDs;
- deterministic parsing errors;
- equality/order/hash behavior suitable for registry use;
- tests for valid and invalid values.

Do not silently normalize ambiguous or unsafe IDs.

### 4.2 Version model

Implement or reuse a deterministic version representation.

Minimum requirements:

- parse and compare application versions;
- deterministic ordering;
- reject malformed versions;
- preserve the original version string only if useful, while comparisons use validated structure;
- avoid inventing Nagi-specific semantics when standard semantic-version behavior is sufficient.

If an existing version crate or internal type is already used, prefer that rather than introducing an incompatible second implementation.

### 4.3 Package manifest

Define the initial Nagi application manifest contract.

The exact serialization format should follow repository conventions. JSON, TOML or another already-established format is acceptable if justified by existing code.

The manifest must support at least:

- schema/manifest version;
- application ID;
- application version;
- human-readable display name;
- package/runtime entrypoint;
- application/runtime class if the architecture already distinguishes classes;
- required capabilities;
- optional capabilities where meaningful;
- required system services or service contracts;
- minimum Nagi/runtime compatibility field or a forward-compatible placeholder;
- optional metadata that can be ignored safely by older readers when allowed;
- developer/publisher metadata only if already supported by Nagi architecture.

Do not add signing/trust semantics here unless they already exist. Signing and install verification belong to installation/update policy and later security work.

### 4.4 Manifest validation

Validation must be explicit and testable.

Validate at least:

- manifest/schema version;
- mandatory fields;
- AppId correctness;
- version correctness;
- duplicate capability declarations;
- contradictory required/optional declarations;
- duplicate service dependencies;
- invalid/empty entrypoints;
- unsupported manifest versions;
- unsafe path-like entrypoints if the package architecture requires relative paths;
- unknown fields according to a documented forward-compatibility policy.

Validation errors must be structured enough that CLI/runtime callers do not need to parse prose strings.

### 4.5 Lifecycle model

Implement a deterministic application lifecycle state machine or equivalent model.

At minimum model:

- installed/known or equivalent pre-launch state if appropriate;
- starting;
- running/active;
- suspended;
- stopping/terminating;
- stopped/terminated;
- failed if the existing architecture requires it.

Minimum operations:

- launch/start;
- activate/focus if activation is a distinct concept;
- suspend;
- resume;
- request termination;
- mark terminated;
- failure transition.

Requirements:

- legal transitions are explicit;
- illegal transitions return structured errors;
- repeated/idempotent operations are defined intentionally, not accidentally;
- transition behavior is deterministic;
- transition tests cover success and rejection cases.

Do not implement a full GUI process manager unless such a host already exists and the minimal integration is clearly owned by this workstream.

### 4.6 Lifecycle event contract

Provide an event or observation contract suitable for later integration with Activity/Diagnostics without importing those systems' internals.

At minimum expose structured information for:

- app identity;
- previous lifecycle state;
- new lifecycle state;
- transition reason/category;
- failure category when applicable.

The core package must remain usable without Diagnostics or Activity being present.

### 4.7 Capability declaration integration

If the Capability / Permission foundation is present:

- consume its public capability identifier/declaration types when practical;
- do not duplicate capability semantics;
- ensure manifest validation can identify invalid or duplicate capability declarations;
- keep permission prompting/grant decisions outside this workstream.

If the capability workstream is not yet integrated into the base branch, define a narrow adapter boundary and document the integration proposal rather than blocking.

### 4.8 System-service dependency declaration

Applications must be able to declare service contracts they require.

Do not implement those services here. Use neutral identifiers or interfaces compatible with `SVC-IPC-01` once available.

If `SVC-IPC-01` is not yet integrated, use a small local abstraction and record the exact convergence point in docs/state. Do not wait idle for another branch.

---

## 5. Optional CLI integration

If `tools/nagi-cli` is the repository's standard developer surface, add a minimal command only if it fits the current architecture without taking ownership from another workstream.

Useful examples:

- validate a manifest;
- print normalized/parsed manifest information;
- exercise lifecycle state validation in tests/dev mode.

The CLI is not required if the library API plus tests already provide a clean foundation and adding CLI wiring would cause shared-file conflicts.

---

## 6. Tests

Add meaningful tests, not only compile checks.

Minimum test groups:

1. AppId valid cases;
2. AppId invalid/unsafe cases;
3. version parsing and ordering;
4. valid minimal manifest;
5. valid richer manifest;
6. missing required fields;
7. unsupported schema version;
8. duplicate capabilities;
9. duplicate services;
10. invalid entrypoint/path behavior;
11. legal lifecycle transitions;
12. illegal lifecycle transitions;
13. idempotency behavior where defined;
14. lifecycle failure behavior;
15. serialization/deserialization round-trip if serialization is used;
16. unknown-field / forward-compatibility behavior.

Use deterministic fixtures. Do not depend on network access.

---

## 7. Documentation

Add or update workstream documentation describing:

- manifest field reference;
- compatibility/versioning policy;
- lifecycle state diagram or textual transition table;
- integration boundary with Capability / Permission;
- integration boundary with System Service / IPC;
- non-goals;
- example minimal manifest;
- example validation failure;
- how future first-party apps consume the contract.

Update global documentation only where this workstream owns the file or where the change is small and clearly safe. Otherwise leave an Integration Owner proposal.

---

## 8. Workstream state

Create/update the dedicated workstream state using DF-01 conventions.

The state must record at least:

- workstream ID;
- branch;
- base SHA if known;
- current HEAD;
- status;
- completed acceptance items;
- remaining blockers;
- test commands and results;
- files/areas owned;
- integration proposals;
- external dependencies;
- latest meaningful failure if not PASS.

Use actual repository status values if a fixed enum/schema exists.

### PASS rule

Mark this workstream `PASS` only if:

- core implementation exists;
- required tests pass;
- formatting/lint/check requirements pass for owned code;
- docs/state are updated;
- no unresolved defect remains inside this workstream's owned scope;
- only integration-owner actions, if any, remain outside this workstream's ownership.

If a required shared-registry edit is forbidden, the local foundation may be complete while the registry integration remains a clearly recorded external action. Use the repository's existing status semantics rather than inventing a new status.

---

## 9. Verification expectations

Run the narrowest relevant commands first, then broader owned-scope verification.

Expected categories:

- crate/module unit tests;
- all-target tests when applicable;
- formatting check;
- Clippy/lint with warnings denied if repository convention uses it;
- package/workspace check as appropriate;
- `./nagi doctor` if it is expected to remain green and the command does not require unrelated target builds;
- any existing DF-01 `nagi dev verify` command applicable to the workstream.

Do not spend hours rebuilding unrelated Servo/Mesa target code merely to claim broad verification. Record unrelated failures accurately and continue owned-scope validation.

---

## 10. Long-running execution policy

After inspection, do not stop at a plan.

Continue in this loop:

1. identify the highest-priority incomplete acceptance item;
2. implement it;
3. run the most relevant tests;
4. if a test fails, isolate the cause;
5. fix owned-scope defects;
6. rerun the test;
7. expand acceptance coverage;
8. update docs/state when behavior stabilizes;
9. run final format/lint/check/tests;
10. commit;
11. push;
12. verify worktree cleanliness and local/remote HEAD state.

A failing test is normally the next work item, not a reason to stop and report immediately.

Stop only for a genuine ownership/security/credential/external dependency blocker that cannot be resolved within this workstream.

---

## 11. Git safety

- Never use destructive reset on unrelated work.
- Never discard another workstream's modifications.
- Do not rewrite remote history.
- Preserve existing worktrees.
- Prefer a dedicated worktree if one already exists for this branch.
- Commit only this workstream's changes.
- Push the dedicated branch.

Final report must include:

- final status;
- branch;
- pushed HEAD SHA;
- whether worktree is clean;
- tests/checks run and results;
- important implementation paths;
- any Integration Owner action still required.

---

## 12. Acceptance criteria

This workstream is complete when the repository contains a tested, documented foundation satisfying all applicable items below:

- [ ] Strong application identity type and validation
- [ ] Deterministic application version parsing/comparison
- [ ] Versioned package manifest schema/model
- [ ] Required application metadata and entrypoint declaration
- [ ] Capability requirement declarations
- [ ] System-service dependency declarations
- [ ] Structured manifest validation errors
- [ ] Forward-compatibility policy for manifest evolution
- [ ] Deterministic lifecycle state model
- [ ] Legal and illegal transition handling
- [ ] Suspend/resume/terminate semantics
- [ ] Structured lifecycle event/observation contract
- [ ] No direct dependency on M18 browser internals
- [ ] No duplication of permission-grant policy
- [ ] Comprehensive unit/fixture tests
- [ ] Developer documentation and minimal manifest example
- [ ] Workstream state updated
- [ ] Integration proposal recorded where shared ownership prevents direct registration
- [ ] Formatting/lint/check/test acceptance passes for owned scope
- [ ] Changes committed and pushed
