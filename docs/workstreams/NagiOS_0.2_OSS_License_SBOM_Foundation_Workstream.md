# Nagi OS 0.2 — OSS License / SBOM Foundation Workstream

## 0. Status and scope

This document defines one isolated Nagi OS foundation workstream for **open-source dependency inventory, license compliance metadata, NOTICE generation support, and Software Bill of Materials (SBOM) generation**.

This is a host-side compliance/developer-tooling workstream. It must not activate Nagi 0.2 runtime development, alter M18 behavior, or interfere with existing parallel implementation branches.

The objective is a working implementation with tests and durable state, not a planning-only document.

---

## 1. Purpose

Nagi includes or is expected to include multiple third-party components across Rust crates, browser/graphics dependencies, firmware/tooling, vendored source, and model-related assets.

The project needs a repeatable way to answer:

- what third-party components are present;
- which versions/revisions are used;
- where they came from;
- what licenses apply;
- whether a license file or NOTICE text is present;
- which components require manual review;
- what must be shipped with a release;
- whether dependency/license state changed between revisions.

This workstream builds the host-side foundation for that process and produces a machine-readable SBOM without making unsupported legal conclusions.

---

## 2. Non-goals

This workstream must **not**:

- provide legal advice or declare that Nagi is legally compliant in every jurisdiction;
- reinterpret license text beyond deterministic metadata/classification rules;
- modify third-party source merely to simplify license scanning;
- change M18 runtime/browser code;
- change model/runtime behavior;
- fetch unpinned arbitrary software from the network during normal validation if repository metadata is sufficient;
- automatically accept unknown or ambiguous licenses;
- silently suppress missing attribution requirements;
- merge to `main`;
- overwrite files owned by other active workstreams.

---

## 3. Required preflight

Before editing, inspect the real repository:

- `AGENTS.md`;
- current Git branch, HEAD, worktree list, remotes, dirty state;
- `docs/implementation_status.md`;
- `docs/0.2/DEVELOPMENT_ARCHITECTURE.md` if present;
- `.dev/workstreams.json` and workstream state conventions if present;
- Rust workspace manifests and lockfiles;
- `third_party/**` structure;
- existing `LICENSE*`, `NOTICE*`, `COPYING*`, attribution files;
- Servo/Mesa source metadata;
- any model-license/NOTICE files already tracked;
- package/SDK metadata relevant to redistribution;
- existing host tooling commands.

Repository files and Git state are authoritative.

---

## 4. Dedicated branch and worktree

Preferred branch:

`codex/0.2-license-sbom`

Preferred worktree:

`../NagiOS-0.2-license-sbom`

If an existing repository naming convention differs, adapt without colliding with another active workstream.

---

## 5. Ownership boundary

### Preferred allowed paths

Use the narrowest host-side structure consistent with the repository:

- `tools/nagi-cli/**`
- `tools/nagi-dev/**` if present
- `tools/legal/**` or `tools/sbom/**` if a dedicated module is cleaner
- `docs/0.2/**`
- `docs/legal/**` if present or newly established
- `.dev/workstreams/license-sbom/**`
- host-side tests for the legal/SBOM tooling

### Shared/integration-owned paths

Treat as integration-owned unless explicitly assigned:

- root `Cargo.toml`
- root `Cargo.lock`
- `.dev/workstreams.json`
- `.dev/schemas/**`
- `.github/workflows/**`
- `docs/implementation_status.md`
- top-level release packaging manifests

If integration changes are needed, prepare exact proposals in owned docs/state rather than silently changing shared files.

### Forbidden paths

Do not edit implementation under:

- `kernel/**`
- `loader/**`
- `user/nagi-init/**`
- `user/nagi-albert/**`
- `user/nagi-posix/**`
- `third_party/**` except, only if explicitly permitted, adding non-invasive metadata outside vendored upstream content; default is read-only
- generated `target/**`, `out/**`, caches

---

## 6. Functional requirements

### FR-1: Component inventory model

Define a stable machine-readable component record containing, where available:

- component name;
- package/ecosystem type;
- version, revision, or commit;
- source repository/origin;
- local path or package identity;
- declared license expression;
- detected license files;
- copyright/NOTICE file references;
- direct vs transitive classification where known;
- vendored vs registry-resolved classification;
- runtime/build/dev/tooling scope where determinable;
- checksum or immutable identifier when appropriate;
- review status;
- evidence source.

Unknown values must remain explicitly unknown, not guessed.

### FR-2: Rust dependency inventory

Parse the Rust workspace dependency graph using repository/lockfile metadata or a stable Cargo metadata path.

Capture at least:

- package name;
- version;
- source;
- checksum where available;
- declared license/license-file metadata where available;
- dependency relationship/scope where practical.

The implementation must not depend on unstable human-formatted Cargo output.

### FR-3: Vendored/third-party source inventory

Scan known vendored/pinned third-party areas such as Servo and Mesa without modifying them.

Capture:

- local component identity;
- pinned revision/source if available;
- root license files;
- NOTICE/COPYING files;
- obvious nested licensing metadata where the repository already tracks it.

Do not recursively ingest enormous source trees without bounds. Define and test a bounded discovery strategy.

### FR-4: Model/license metadata readiness

Support license records for model assets and future bundled models without requiring those assets to exist in this workstream.

The schema should be capable of representing at least the project's planned bundled model families and their accompanying license/NOTICE obligations as repository metadata becomes available.

Do not download models in order to pass this workstream.

### FR-5: SBOM generation

Generate at least one standard SBOM format.

Preferred:

- SPDX JSON; or
- CycloneDX JSON.

Supporting both is welcome but not required if it creates unnecessary scope.

The output must:

- have deterministic ordering;
- include project identity/version/commit where appropriate;
- include collected component identifiers;
- avoid secrets and host-private paths;
- clearly mark incomplete/unknown license data.

### FR-6: Human-readable report

Provide a concise human-readable report showing:

- total components;
- licenses seen;
- unknown/ambiguous license records;
- missing license/NOTICE evidence;
- components requiring manual review;
- changes compared with a prior baseline if comparison is supported.

### FR-7: NOTICE candidate generation

Generate a **NOTICE candidate**, not an authoritative legal determination.

The generated result should aggregate attribution/license file references for components where repository metadata indicates they may need to be shipped or reviewed.

It must clearly distinguish:

- detected source text/reference;
- generated project metadata;
- manual review required.

Do not rewrite third-party license text.

### FR-8: Policy/check mode

Provide a check command suitable for local development/CI.

Preferred UX:

```sh
./nagi legal scan
./nagi legal sbom --output <path>
./nagi legal notice --output <path>
./nagi legal check
```

Equivalent names may be used if repository CLI conventions differ.

`check` should fail on clearly defined structural issues such as:

- dependency record with no resolvable identity;
- malformed license expression when the metadata claims one;
- vendored component with no located license evidence and no explicit review waiver/record;
- duplicate/conflicting component identity;
- generated SBOM schema failure.

Unknown license semantics should be surfaced for review rather than guessed.

### FR-9: Baseline/diff capability

If scope permits without shared ownership conflict, allow comparison between two inventories/SBOMs and report:

- added components;
- removed components;
- version/revision changes;
- license metadata changes;
- new missing-license conditions.

This is highly desirable but secondary to correct scan/SBOM generation.

### FR-10: Offline and deterministic operation

Normal scan/check tests must operate from checked-out repository metadata and local package metadata.

Do not require arbitrary live network access for acceptance.

---

## 7. License handling rules

The tool may normalize well-known SPDX identifiers when the source metadata is explicit.

It must not invent an SPDX identifier from vague prose with low confidence.

Examples:

- explicit `MIT` -> record `MIT`;
- explicit `Apache-2.0 OR MIT` -> preserve the expression;
- `license-file = "LICENSE"` with no expression -> record file evidence and mark expression unknown unless reliably parsed;
- conflicting metadata -> mark conflict/manual review.

The tool should record evidence so humans can inspect why a result was produced.

---

## 8. Tests

Add focused automated tests for at least:

1. deterministic component ordering;
2. deterministic SBOM output excluding intentionally volatile metadata;
3. Rust package metadata parsing;
4. SPDX expression preservation;
5. unknown license stays unknown;
6. license-file evidence without guessed license ID;
7. vendored component license discovery;
8. bounded scan does not traverse build/cache trees;
9. duplicate/conflicting component detection;
10. missing license evidence triggers review/check behavior;
11. NOTICE candidate aggregation;
12. host absolute paths and credentials are not emitted;
13. offline fixture-based operation;
14. SBOM schema/shape validation;
15. comparison/diff behavior if implemented.

Run formatting, lint/static checks, focused tests, and relevant existing host-side tooling tests.

---

## 9. Acceptance criteria

This workstream is **PASS** only when:

- a stable component inventory structure exists;
- Rust dependencies are inventoried from stable metadata;
- vendored/pinned third-party components can be represented and scanned;
- license evidence is recorded without unsupported guesses;
- unknown/conflicting license data is surfaced explicitly;
- at least one standard SBOM format is generated;
- SBOM output is deterministic enough for review/diffing;
- a human-readable scan/check report exists;
- NOTICE candidate generation exists or an equivalent evidence export satisfies the same review need;
- secret/absolute-host-path leakage tests pass;
- offline fixture tests pass;
- relevant format/lint/static checks pass;
- existing host tooling tests remain green;
- durable workstream state is updated with exact evidence;
- changes are committed to the dedicated branch;
- branch is pushed;
- final worktree is clean;
- no runtime/M18/third-party implementation source was modified;
- no shared integration-owned file was changed without explicit ownership.

---

## 10. Durable state requirements

Use the repository's `.dev/workstreams/<id>/state.json` convention if available.

Record:

- workstream ID;
- branch;
- status;
- last verified commit;
- exact commands/tests;
- scan/SBOM evidence;
- unresolved license-review items;
- failure classifications;
- CI evidence if applicable;
- blocker if any;
- next action;
- integration proposals;
- deferred work.

Never rely on the chat thread as the only record of compliance findings.

---

## 11. Failure handling

When a scan or test fails:

1. capture the exact failing component/input;
2. distinguish parser/tool bug from genuinely incomplete license metadata;
3. repair only owned tooling/docs/state;
4. do not edit upstream vendored license text to make the test green;
5. rerun focused tests;
6. continue through broader host validation.

Do not stop just because one component needs manual review. The tool should support representing that condition while the implementation work continues.

Stop only for a genuine ownership/external dependency blocker.

---

## 12. Commit and push policy

Use coherent commits.

Before final push:

- run format/lint;
- run focused test suite;
- run relevant broader host checks;
- verify generated outputs are intentional and bounded;
- inspect for copied build caches or large generated artifacts;
- verify no secrets or machine-local paths are staged;
- update durable state.

Push only the dedicated workstream branch.

Do not merge to `main`.

---

## 13. Final completion report

Finish with:

- PASS / BLOCKED;
- branch;
- pushed HEAD SHA;
- implementation summary;
- component count from the test/repository scan when available;
- generated SBOM format;
- unresolved manual-review items;
- tests/checks and results;
- CI URL/result if triggered;
- files/areas changed;
- integration proposals;
- confirmation that worktree is clean.

If blocked, include the exact blocker and the next executable action.
