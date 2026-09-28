# Nagi OS 0.2 — Update / Installation Foundation Workstream

## 0. Workstream identity

- **Workstream ID:** `UPD-01`
- **Recommended branch:** `codex/0.2-update-installation`
- **Primary scope:** local application package staging, validation boundary, install/update/uninstall transaction model, atomic commit/rollback semantics, version policy, package inventory, recovery tests and integration adapters
- **Repository:** `/Users/tozawa/Developer/NagiOS`
- **Execution model:** dedicated worktree / dedicated branch
- **Default status at start:** `PLANNED` or actual repository state

This workstream must proceed beyond planning. Implement the independent foundation, test it, debug failures, update state, commit and push.

---

## 1. Goal

Build the Nagi 0.2 host-testable foundation for installing, updating and uninstalling application packages safely.

The foundation should eventually support a Nagi-native application package format (for example a future `.nagiapp` container), but **must not prematurely lock the project to a container/archive encoding if the package format is not yet standardized**.

The central goal is a deterministic transaction engine and package validation boundary that later Store, first-party app and offline installation flows can reuse.

---

## 2. Parallel-workstream constraint

`APP-LC-01` may be developed in parallel and is expected to own the canonical application manifest model.

Therefore:

- do not duplicate a second permanent Nagi app manifest specification;
- if the canonical manifest types are already available, consume them through their public API;
- if they are not yet merged, introduce a narrow `PackageMetadata` / manifest-adapter boundary or test fixture representation sufficient for this workstream;
- clearly document the adapter that must later bind to `APP-LC-01`;
- do not stop and wait for `APP-LC-01` if transaction, inventory, rollback and validation work can proceed independently.

This workstream should be mergeable with minimal convergence work later.

---

## 3. Non-goals / prohibited scope

Do not expand into:

- M18 browser / Servo / Mesa / relibc work;
- Store UI or store catalog backend;
- Internet download client;
- payment/licensing service;
- software signing PKI design from scratch;
- enterprise MDM;
- OS-wide self-update of the Nagi kernel/runtime unless an existing architecture explicitly places it here;
- capability grant UX/prompting;
- SBOM/license scanning internals already owned by the License/SBOM workstream;
- Build Provenance fingerprint internals;
- first-party application feature code;
- shared registry edits not owned by this workstream.

This foundation may **consume** SBOM/license/provenance metadata via adapters, but must not take ownership of those systems.

---

## 4. Required repository inspection

Before editing, inspect:

- relevant `AGENTS.md` files;
- `docs/0.2/DEVELOPMENT_ARCHITECTURE.md`;
- existing package/app/installer/updater code;
- existing filesystem atomic-write utilities;
- Files/VFS abstractions that are safe for host-side use;
- Capability / Permission public contracts;
- License/SBOM foundation outputs and metadata format;
- Build Provenance/fingerprint proposal or public outputs if present;
- App Lifecycle/Manifest public contract if present;
- `.dev` workstream state/schema;
- CI/test conventions;
- current Git/worktree state.

Use actual repository behavior over prior chat summaries.

---

## 5. Required architecture

Separate the concerns below so policy can evolve without rewriting transaction mechanics:

1. **package source/staging**;
2. **metadata/manifest validation adapter**;
3. **install plan**;
4. **transaction execution**;
5. **inventory/current-version state**;
6. **atomic commit**;
7. **rollback/recovery**;
8. **policy hooks** for permissions, provenance, license/SBOM or future trust checks.

Prefer a host-testable Rust library/module with deterministic temporary-directory tests.

---

## 6. Package source abstraction

Define a package source/staged package abstraction independent of final archive encoding.

It should provide at least:

- package root or virtual package content access;
- manifest/metadata access through an adapter;
- file enumeration where needed;
- deterministic file metadata sufficient for install planning;
- safe handling of relative paths.

Explicitly reject:

- absolute install paths from untrusted package data;
- `..` traversal;
- path escape from staging root;
- duplicate/conflicting destination paths;
- unsupported path encodings according to repository conventions.

If archive extraction is implemented, defend against zip-slip/path traversal and test it. Archive support is optional if it would prematurely freeze the final package container format.

---

## 7. Package metadata adapter

Define the minimum metadata the installer needs:

- app ID;
- version;
- package/manifest schema version;
- entrypoint reference if required for validation;
- required capabilities for policy comparison;
- optional provenance/license/SBOM references when present.

If `APP-LC-01` is integrated, use its public manifest types. Otherwise use an explicit temporary adapter interface and document that it is **not** a competing permanent manifest format.

---

## 8. Installed package inventory

Implement a deterministic installed-package inventory abstraction.

Minimum operations:

- query installed app by ID;
- enumerate installed apps if appropriate;
- record installed version and active location;
- distinguish staged/pending vs committed state if transaction recovery requires it;
- remove inventory entry after successful uninstall;
- recover from incomplete transaction markers.

The inventory storage format should be versioned or forward-compatible.

Avoid machine-specific absolute paths in portable test snapshots unless paths are intentionally runtime-local.

---

## 9. Install plan

Before mutating the active installation, build a validated install plan.

The plan should capture:

- target app ID;
- source version;
- current installed version if any;
- operation kind: fresh install/update/reinstall/downgrade if supported;
- staged destination;
- files/actions to commit;
- validation/policy checks required;
- rollback metadata.

Invalid plans fail before active state is modified.

---

## 10. Version/update policy

Implement explicit version decision behavior.

At minimum:

- fresh install allowed when no version exists;
- upgrade behavior;
- same-version reinstall behavior explicitly allowed or rejected;
- downgrade behavior explicitly rejected by default unless an override policy exists;
- malformed version rejection;
- incompatible package/manifest schema rejection.

Do not hide policy in string comparison.

---

## 11. Transaction model

Implement a transaction state model with deterministic transitions.

Suggested phases, adapted to repository conventions:

- created;
- validating;
- staged;
- ready-to-commit;
- committing;
- committed;
- rolling-back;
- rolled-back;
- failed.

Requirements:

- illegal transitions rejected;
- transaction ID/correlation ID;
- durable marker/journal if required to test crash recovery;
- clear boundary between staged files and active install;
- no partially active install on ordinary failure;
- structured error categories.

---

## 12. Atomic commit

Implement atomic or best-available commit semantics using repository/platform primitives.

The normal successful sequence should prevent consumers from seeing a half-installed package.

A common acceptable host reference approach is:

1. validate source;
2. copy/extract into a unique staging directory;
3. validate staged tree;
4. create transaction marker;
5. atomically switch active package pointer/directory using rename/replace semantics where supported;
6. update inventory;
7. clear transaction marker;
8. preserve/remove prior version according to rollback policy.

The exact layout may differ if the repository already defines app storage.

Test failure injection around commit boundaries.

---

## 13. Rollback and crash recovery

Rollback is a core acceptance item, not optional documentation.

Implement and test recovery for meaningful interruption points such as:

- failure before staging completes;
- validation failure after staging;
- failure before active switch;
- failure after active switch but before inventory finalization;
- failed update with a previous version available;
- interrupted uninstall if the model supports transactional uninstall.

Requirements:

- recovery is deterministic;
- previous known-good version is restored or active state remains unchanged where possible;
- orphan staging directories/markers can be identified and safely cleaned;
- tests use explicit failure injection rather than relying on random crashes.

---

## 14. Uninstall

Implement a safe uninstall foundation.

Minimum behavior:

- verify target app exists;
- identify active installation owned by that AppId;
- remove/deactivate package files without path escape;
- update inventory consistently;
- preserve user data unless architecture explicitly says application data belongs to uninstall scope;
- return structured not-found/in-use/policy errors as appropriate.

Do not invent destructive user-data deletion behavior.

---

## 15. Capability change hook

When updating an app, compare old vs new required capability declarations when that metadata is available.

Expose policy information such as:

- unchanged capabilities;
- added required capabilities;
- removed capabilities.

Do **not** grant permissions automatically and do not implement permission prompts. Return/emit a policy decision point that Capability / Permission UX can handle later.

If canonical capability types are unavailable, use an adapter and test with neutral identifiers.

---

## 16. SBOM / license / provenance hooks

The installer should be able to receive validation results from existing/future foundation workstreams without depending on their internal implementation.

Provide hooks/adapters for optional checks such as:

- SBOM present/parseable;
- license metadata present;
- artifact fingerprint/provenance match;
- trust/signature result in the future.

For 0.2 foundation:

- do not invent a new legal policy;
- do not infer unknown licenses;
- do not implement a PKI;
- allow tests to inject allow/deny/error decisions.

A failed mandatory policy check must prevent commit before the active install is modified.

---

## 17. Structured errors

Define errors sufficient for callers and tests, including categories such as:

- invalid package;
- invalid manifest/metadata;
- unsafe path;
- version conflict;
- downgrade rejected;
- already installed/not installed;
- staging failure;
- commit failure;
- rollback failure;
- inventory failure;
- policy denied;
- unsupported schema;
- I/O failure.

Do not require callers to parse human-readable strings.

---

## 18. Tests

Minimum deterministic test matrix:

1. fresh install success;
2. valid upgrade success;
3. same-version behavior;
4. downgrade rejected by default;
5. malformed metadata rejected;
6. unsafe absolute path rejected;
7. `..` traversal rejected;
8. duplicate destination conflict rejected;
9. validation failure leaves active install unchanged;
10. staging failure leaves active install unchanged;
11. commit failure triggers rollback/recovery;
12. previous version restored after failed update;
13. interrupted transaction marker recovery;
14. uninstall success;
15. uninstall missing app behavior;
16. uninstall does not delete unrelated/user data;
17. capability-delta calculation;
18. mandatory policy hook denial prevents commit;
19. inventory round-trip/reload;
20. two different app IDs remain isolated;
21. cleanup of orphan staging data;
22. no writes outside test root.

Where practical, add property/edge tests for path normalization and transaction transitions.

---

## 19. Optional CLI/dev surface

If `tools/nagi-cli` ownership allows minimal integration, useful commands may include:

- package inspect/validate;
- install from local staged directory in development mode;
- list installed package inventory in a temporary/dev root;
- recover incomplete transactions in a test/dev root.

Avoid wiring a production user-facing installer UI in this workstream.

If shared CLI files are contentious, keep the library complete and leave a small integration proposal rather than blocking.

---

## 20. Documentation

Document:

- installation directory model;
- package source/manifest adapter boundary;
- transaction state model;
- update version policy;
- atomic commit strategy;
- rollback/recovery behavior;
- uninstall/user-data boundary;
- capability-change hook;
- SBOM/license/provenance integration hooks;
- security/path traversal assumptions;
- how `APP-LC-01` should replace any temporary manifest adapter.

---

## 21. Workstream state

Update dedicated state following DF-01 conventions.

Include:

- workstream ID;
- branch;
- base/current HEAD;
- status;
- completed acceptance items;
- test commands/results;
- owned paths;
- external integration dependencies;
- manifest-adapter convergence note;
- shared-registry registration proposal if required;
- latest unresolved failure if not PASS.

Do not mark PASS while rollback/recovery behavior required by this specification is unimplemented.

---

## 22. Verification

Run:

- focused unit tests;
- transaction/integration tests with temporary roots;
- format;
- warnings-denied lint/Clippy per repository conventions;
- package/workspace check;
- relevant DF-01 verification;
- `./nagi doctor` if appropriate and independent from heavy unrelated target builds.

No network is required for acceptance. This foundation must be verifiable offline.

---

## 23. Long-running execution policy

Do not stop after discovering missing dependencies.

Use this execution loop:

1. inspect actual repository;
2. choose the highest-priority independent acceptance item;
3. implement;
4. test;
5. on failure, isolate the exact failure boundary;
6. fix owned code;
7. rerun;
8. add negative/failure-injection coverage;
9. update state/docs;
10. run final quality gates;
11. commit;
12. push;
13. verify remote HEAD and clean worktree.

If `APP-LC-01` is not yet merged, continue behind the adapter boundary. If License/SBOM/Provenance integration is unavailable, use mockable policy hooks. These are not reasons to remain in planning mode.

---

## 24. Git safety

- Keep unrelated worktrees intact.
- No destructive reset.
- No force push.
- Do not rewrite another workstream's branch.
- Do not edit shared integration-owner files merely to make status look green.
- Commit only owned changes.
- Push the dedicated branch.

Final report must include:

- final status;
- branch;
- pushed HEAD SHA;
- clean/dirty worktree;
- tests/checks and results;
- major implementation paths;
- remaining Integration Owner actions/dependencies.

---

## 25. Acceptance criteria

- [ ] Safe package source/staging abstraction
- [ ] No permanent duplicate of App Lifecycle manifest contract
- [ ] Manifest/metadata adapter with clear convergence point
- [ ] Installed package inventory
- [ ] Explicit install/update/uninstall plans
- [ ] Deterministic version/update policy
- [ ] Transaction state model
- [ ] Atomic/best-available commit semantics
- [ ] Rollback implementation
- [ ] Crash/interruption recovery implementation
- [ ] Safe uninstall preserving unrelated/user data
- [ ] Path traversal/escape protection
- [ ] Structured installer errors
- [ ] Capability-delta policy hook
- [ ] SBOM/license/provenance validation hooks
- [ ] Failure injection tests
- [ ] Offline deterministic integration tests
- [ ] Documentation complete
- [ ] Workstream state complete
- [ ] Shared integration proposal recorded where needed
- [ ] Format/lint/check/tests pass for owned scope
- [ ] Commit and push complete
