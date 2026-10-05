# Nagi OS 0.2 — Build Provenance / Artifact Fingerprint Foundation Workstream

## 0. Status and scope

This document defines one isolated Nagi OS foundation workstream for **build provenance, artifact fingerprinting, reproducibility metadata, and safe artifact reuse prerequisites**.

This workstream is intentionally **host-side / development-foundation only**. It must not activate Nagi 0.2 runtime development, must not bypass the Nagi 0.1 milestone sequence, and must not interfere with the currently active M18 implementation or any existing parallel workstream.

The goal is not merely to document a future design. The assigned Codex thread must implement the bounded foundation described here, add focused tests, update its durable workstream state, and commit and push the completed workstream branch.

---

## 1. Purpose

Nagi builds increasingly expensive target artifacts involving Rust, Servo, Mesa/Softpipe, target-specific patches, generated inputs, firmware/QEMU configuration, and acceptance images/logs.

A successful build artifact must never be reused merely because a directory exists or because a partial cache key matches.

This workstream establishes a **machine-readable, deterministic build fingerprint and artifact provenance contract** so later work can safely decide whether two build products are actually equivalent.

Primary outcomes:

- deterministic build fingerprint generation;
- complete source/toolchain/configuration provenance capture;
- artifact checksum inventory;
- host-readable validation of fingerprint compatibility;
- explicit mismatch reporting;
- immutable evidence suitable for CI and future cache/artifact reuse;
- no weakening of current clean-build or milestone acceptance gates.

---

## 2. Non-goals

This workstream must **not**:

- implement or change M18 browser/runtime behavior;
- modify Servo, Mesa, relibc, kernel, loader, or target runtime code;
- introduce build artifact reuse into production CI unless the existing acceptance boundary already explicitly permits it;
- claim reproducible builds merely because fingerprints match;
- add broad caches keyed only by branch, Cargo.lock, target name, or source SHA;
- change M17/M18 acceptance definitions;
- merge any branch into `main`;
- reset, rebase, clean, or overwrite another active worktree;
- change shared integration-owned files unless this workstream explicitly owns them or records a proposal instead.

---

## 3. Required preflight

Before editing anything, inspect the real repository and preserve all active work.

At minimum inspect:

- `AGENTS.md`;
- `docs/implementation_status.md`;
- `docs/0.2/DEVELOPMENT_ARCHITECTURE.md` if present;
- `.dev/workstreams.json` and relevant schema/state files if present;
- current branch, HEAD, remotes, worktree list, and dirty state;
- existing `./nagi dev` commands;
- existing build scripts and artifact generation paths;
- current CI workflow only for understanding, not opportunistic rewriting;
- existing diagnostic metadata and artifact hashing helpers.

Treat repository state as authoritative over prior chat reports.

If another worktree already owns a file this workstream would need to edit, do not overwrite it. Prefer an alternate owned path or record an integration proposal in this workstream state/docs.

---

## 4. Dedicated branch and worktree

Preferred branch:

`codex/0.2-build-provenance`

Preferred dedicated worktree:

`../NagiOS-0.2-build-provenance`

If the repository already contains a registered naming convention that differs, preserve the repository convention rather than inventing a conflicting duplicate.

Do not reuse the M18 worktree.

---

## 5. Ownership boundary

### Preferred allowed paths

Use the narrowest existing structure possible. Preferred ownership:

- `tools/nagi-cli/**`
- `tools/nagi-dev/**` if present
- `tools/build-provenance/**` if a dedicated host-side tool is cleaner
- `docs/0.2/**`
- `.dev/workstreams/build-provenance/**`
- tests owned by the host-side tooling area

### Shared/integration-owned paths

Treat these as integration-owned unless repository rules explicitly grant this workstream ownership:

- `.dev/workstreams.json`
- `.dev/schemas/**`
- root `Cargo.toml`
- root `Cargo.lock`
- `.github/workflows/**`
- `docs/implementation_status.md`

If a shared change is required, prepare the implementation in an owned location and record an exact integration proposal instead of silently editing shared ownership.

### Forbidden paths

Do not edit:

- `kernel/**`
- `loader/**`
- `third_party/**`
- `user/nagi-init/**`
- `user/nagi-albert/**`
- `user/nagi-posix/**`
- M18-owned source paths
- Servo/Mesa patches
- generated `out/**`, `target/**`, or cache directories as committed source

---

## 6. Functional requirements

### FR-1: Canonical build fingerprint schema

Define a stable versioned machine-readable schema, preferably JSON.

The fingerprint must support at least:

- schema version;
- repository identifier;
- Git commit SHA;
- dirty/clean state;
- relevant submodule or pinned third-party revisions when applicable;
- toolchain versions;
- Rust compiler version and host triple;
- target triple;
- enabled features;
- build profile;
- relevant compiler/linker flags;
- environment inputs that materially alter the produced target;
- generated input digests where relevant;
- Nagi patch/configuration digests;
- Servo revision/digest when in the build graph;
- Mesa/Softpipe revision/digest when in the build graph;
- firmware/QEMU identity where relevant to the acceptance environment;
- deterministic artifact inventory;
- SHA-256 and byte length for recorded artifacts;
- creation timestamp as provenance metadata, but **not as part of equality semantics**;
- producer command/version.

Do not include secrets or raw credentials.

### FR-2: Deterministic canonicalization

Two semantically identical input states must produce the same **compatibility fingerprint/digest**, even if generated at different times or from different absolute host paths.

Host-specific absolute paths, timestamps, usernames, temporary directories, and volatile process identifiers must not cause false mismatches unless they materially affect the build.

Separate:

1. provenance metadata;
2. compatibility-critical normalized inputs;
3. artifact output metadata.

### FR-3: CLI integration

Provide a clear host-side command through the existing Nagi development CLI if consistent with the repository architecture.

Preferred UX:

```sh
./nagi dev fingerprint
./nagi dev fingerprint --output <path>
./nagi dev fingerprint --artifact <path> [...]
./nagi dev fingerprint compare <a.json> <b.json>
```

Equivalent naming is acceptable if existing command conventions require it.

At minimum support:

- emit to stdout;
- optional JSON file output;
- compare two fingerprints;
- non-zero exit for incompatible build identities;
- human-readable mismatch summary;
- machine-readable output suitable for CI.

### FR-4: Mismatch classification

Comparison output must identify why fingerprints differ.

Useful classes include:

- source revision;
- dirty source state;
- toolchain;
- target;
- features;
- build flags;
- third-party source/patch;
- generated inputs;
- environment;
- firmware/runtime harness;
- artifact digest.

Do not collapse all mismatches into a generic `different` result.

### FR-5: Artifact manifest

The tool must be able to record one or more artifact files with:

- normalized logical role/name;
- relative or explicitly classified path;
- size;
- SHA-256;
- optional format/type if reliably detectable without heavy dependencies.

Missing requested artifacts must be an error, not silently skipped.

### FR-6: Safe future reuse contract

Document and encode that a fingerprint match is a **necessary but not sufficient** condition for declaring a milestone or acceptance PASS.

Future artifact reuse must additionally verify:

- immutable source/fingerprint identity;
- artifact checksum match;
- applicable acceptance evidence;
- no forbidden host-built/target-built substitution;
- current milestone policy.

This workstream must not weaken clean-environment verification.

### FR-7: Redaction and secret safety

Environment capture must be allowlisted or sanitized.

Never write values for likely credentials or secrets, including but not limited to:

- tokens;
- API keys;
- passwords;
- private signing secrets;
- auth cookies;
- arbitrary full environment dumps.

### FR-8: Portable host behavior

Host-side tests should work on macOS and Linux where practical, and avoid assumptions that make Windows integration impossible.

Paths must be normalized carefully.

---

## 7. Implementation guidance

Prefer extending the existing Rust-based Nagi development tooling rather than adding an unrelated scripting stack.

Keep dependencies small. If SHA-256 support already exists in the repository, reuse it.

Use deterministic serialization where the compatibility digest depends on serialized content.

Avoid hashing huge source trees blindly if a stable pinned revision plus patch/configuration digests already represent the real input. Where a tree digest is needed, define exactly what files are included and excluded.

Do not hash build outputs as build inputs.

---

## 8. Tests

Add focused automated tests for at least:

1. identical normalized inputs -> identical compatibility digest;
2. timestamp difference -> no compatibility mismatch;
3. absolute worktree path difference -> no compatibility mismatch;
4. Git/source SHA change -> mismatch;
5. feature change -> mismatch;
6. target change -> mismatch;
7. toolchain change -> mismatch;
8. patch/generated-input digest change -> mismatch;
9. artifact content change -> artifact digest mismatch;
10. requested missing artifact -> failure;
11. secret-like environment variables are not emitted;
12. comparison produces stable mismatch classes;
13. JSON schema/version validation if a schema is introduced.

Also run the existing relevant host-side test suite and formatting/lint checks.

Do not trigger an expensive full target build solely to test the fingerprint implementation unless required by existing repository acceptance rules. Use fixtures for host-side unit/integration tests.

---

## 9. Acceptance criteria

This workstream reaches **PASS** only when all of the following are true:

- a versioned fingerprint/provenance representation exists;
- compatibility-critical fields are separated from volatile provenance metadata;
- deterministic compatibility digest behavior is tested;
- artifact SHA-256 inventory is implemented;
- compare operation reports specific incompatibilities;
- secret/redaction tests pass;
- missing artifacts fail closed;
- focused unit/integration tests pass;
- relevant formatting/lint/static checks pass;
- existing host-side tooling tests remain green;
- durable workstream state records the verified commit and commands;
- changes are committed to the dedicated branch;
- the branch is pushed;
- the final worktree is clean;
- no M18/runtime/third-party source was modified;
- no shared integration-owned file was changed without explicit ownership.

---

## 10. Durable state requirements

Create or update the workstream state under the repository's established `.dev/workstreams/<id>/state.json` convention when available.

State should record:

- workstream ID;
- branch;
- current status;
- verified commit SHA;
- acceptance checklist;
- exact test commands and results;
- failure classifications;
- CI evidence if CI was triggered;
- blocker, if any;
- next action;
- deferred integration proposals;
- prohibited or rejected fixes where relevant.

Do not use chat history as project state.

---

## 11. Failure handling

Do not retry the same failing command unchanged without a revised hypothesis.

For persistent failure:

1. capture exact command and output;
2. classify the failure;
3. identify the smallest root cause;
4. make the smallest owned-path correction;
5. rerun focused validation first;
6. continue until PASS or a genuine external blocker is proven.

A blocker is genuine only when further progress requires something outside this workstream's ownership or an unavailable external dependency. A difficult bug is not, by itself, a reason to stop.

---

## 12. Commit and push policy

Use coherent checkpoints rather than one commit per tiny change.

Before final push:

- format;
- run focused tests;
- run relevant broader host checks;
- verify state;
- inspect diff and ownership boundary;
- ensure no generated cache/output files are staged.

Push only the dedicated workstream branch.

Do not merge to `main`.

---

## 13. Final completion report

The Codex thread should finish with a concise report containing:

- final status: PASS / BLOCKED;
- branch;
- pushed HEAD SHA;
- implementation summary;
- tests executed and counts/results;
- CI run URL/result if applicable;
- files/areas changed;
- integration proposals, if any;
- confirmation that the worktree is clean.

If PASS is not achieved, report the exact blocker and the next executable action rather than a generic summary.
