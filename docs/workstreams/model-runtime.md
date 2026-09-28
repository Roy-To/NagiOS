# Model Runtime / Model Store Foundation Workstream

Status: `PASS` (Nagi 0.2 Model Runtime Foundation scope)

## Goal and boundaries

Implement a typed, user-space foundation for model manifests, discovery,
compatibility, selection, provider invocation contracts, and local Model Store
metadata. The workstream does not fetch model weights or implement the
production inference engine. M17, Servo, Capability policy, App SDK, Activity,
Wayback, and other concurrent workstreams remain separately owned. M20 stays
`NOT STARTED` until its milestone acceptance, including a real local Granite
response inside Nagi, is met. M17's status is independent and does not gate
this 0.2 Foundation result.

The accepted generative/decision architecture remains authoritative:

- `crates/nagi-model` continues to own common Nagi object and identity types.
- This workstream adds a separate user-space Model Manager contract.
- Generative providers use the local llama.cpp/GGUF path in Nagi 0.1; this
  contract does not require a particular engine and does not define Decision,
  Embedding, speech, or remote provider runtimes.
- Provider results remain untrusted candidates and carry no OS authority.
- No `jev` service, cloud service, host AI daemon, model download, or host
  filesystem access is required.

## Implementation design

1. Add `user/nagi-model-manager` as a `no_std` workspace crate. Its versioned
   JSON manifest uses strict typed deserialization and semantic validation.
   Unknown fields, unsupported schema/runtime API versions, malformed IDs,
   invalid resource bounds, and malformed integrity metadata are rejected.
2. Model identity, provider/family, artifact reference, capability, modality,
   context/resource requirements, backend compatibility, role, license, and
   NOTICE/terms metadata are independent fields. Artifact references are
   opaque model-store identifiers; they are not host paths. Local registry
   availability, installation, and loading require a verified SHA-256; a
   descriptive profile without a digest cannot be selected or installed.
3. The registry accepts one manifest at a time and records artifact,
   backend, and resource incompatibility as structured availability. A bad
   manifest cannot mutate already registered entries. Unregister removes only
   inactive entries and fails closed during loading or active transitions.
4. Selection filters on requested capability, optional role, context and
   available resources. A compatible user preference wins, then a separately
   supplied role default, then a stable model-ID order. The registry does not
   contain Granite/Qwen/Gemma ranking conditionals.
5. Backend and generative request/session interfaces stay user-space and
   capability-neutral. Artifact readers expose bounded reads so a production
   GGUF backend need not copy an entire model into another buffer. Cancellation
   and deadlines are explicit request inputs and backend errors.
6. Store metadata models discovery, install/update transitions, integrity,
   license acknowledgement, and removal eligibility. It does not implement a
   network store or filesystem service.
7. Host tests use a deterministic fake provider solely to verify orchestration
   and lifecycle contracts. Three small catalog fixtures demonstrate that
   Qwen3 4B, Granite 4.2 3B, and Gemma 3 1B fit the same schema; these
   non-installable examples use illustrative context/resource bounds and
   provider-term references, contain no weights, and make no distribution-ready
   source/hash or licensing claim.

## Verification plan

- Focused host tests for manifest parsing/validation, structured compatibility
  errors, license metadata, capability matching, resource filtering,
  deterministic selection/fallback, missing artifact/backend handling,
  registry isolation, store transitions, fake backend lifecycle,
  cancellation/timeout results, and an extensible `system_one` capability.
- Workspace formatting and focused Clippy/test commands.
- `./nagi test` and `./nagi doctor` when their scope and host environment are
  appropriate; report host checks separately from target/VM acceptance.
- Review the final diff, update this evidence record, commit, and push the
  dedicated branch.

## Evidence

### 2026-09-26 implementation checkpoint

Status: `PARTIAL`. The foundation slice is implemented and focused checks
pass. M20 remains `NOT STARTED`; this workstream has no production inference
backend or real target/VM inference acceptance.

- Added `user/nagi-model-manager`, a `no_std` crate with strict manifest v1
  parsing/validation, capability-oriented registry/selection, lifecycle and
  resource checks, a bounded-read backend/runtime interface, and local Store
  metadata/install/update/removal transitions.
- Added the v1 JSON Schema, three explicitly non-installable model profile
  fixtures, architecture documentation, and this workstream evidence record.
- The fixtures use no real artifact hash/source pin or license claim. Unit
  tests use synthetic integrity metadata only for in-memory orchestration.
- No M17, Servo, Capability, App SDK, Activity, Wayback, kernel, or third-party
  source files were changed. The inspected owner branch predates `.dev` and
  DF-01 state tooling; the current integration registry assigns this stream
  only `.dev/workstreams/model-runtime/**`, not the shared registry/schema.
  `implementation_status.md` keeps M20 `NOT STARTED`.

Verification:

- `cargo test --locked -p nagi-model-manager` — PASS: 25 host unit tests, 2
  host schema/contract tests, and doc tests (0 tests) passed using the pinned
  arm64 nightly toolchain.
- `cargo clippy --locked -p nagi-model-manager --all-targets -- -D warnings` —
  PASS.
- `./nagi fmt` — PASS.
- `./nagi test` — FAILS before suite completion in the existing
  `user/libnagi/src/lib.rs`: the arm64 host compiler rejects its x86_64 syscall
  registers (`rax`, `rdi`, `rsi`, `rcx`, `r11`). That unrelated architecture
  boundary was not modified.
- No target build or QEMU/VM test was run; this foundation does not include a
  real inference backend or weights.

Git: branch `codex/ws-model-runtime`; worktree
`/Users/tozawa/.codex/worktrees/nagi-model-runtime/NagiOS`. The commit SHA
and push result are reported in the workstream handoff.

### 2026-09-26 host-contract hardening

The source-branch Windows failure was reproduced with explicit CRLF input.
The malformed-manifest test now removes the required `source` field without
assuming LF line endings. Registry discovery now requires one backend
descriptor to satisfy both the runtime API and artifact/architecture
constraints; separate descriptors with the same backend ID cannot be combined
to report a false `Available`. `ModelStoreRecord` keeps its invariant-bearing
fields private, derives integrity and license data from its immutable manifest,
and exposes read-only accessors for consumers.

Verification on the dedicated worktree:

- `cargo test --locked -p nagi-model-manager --all-targets` — PASS: 28 unit
  tests, 2 manifest/schema tests, and 1 external Store API test (31 total).
- `cargo fmt --manifest-path user/nagi-model-manager/Cargo.toml -- --check` —
  PASS.
- `cargo clippy --locked -p nagi-model-manager --all-targets -- -D warnings` —
  PASS.
- Focused Windows-line-ending and split-backend regressions each failed before
  their fix and pass afterward.
- Unregister tests pass for unloaded/failed/disabled entries and verify that
  loading/ready/busy/unloading entries remain registered.
- The local Nagi-target `cargo check` was attempted but could not start because
  this worktree lacks the prepared `out/rust-src/library/Cargo.toml`; this is a
  generated host setup input, not a package compile error. The source-branch CI
  target job will provide the reproducible prepared-source check.

The manifest contract test verifies the schema file parses as JSON, spot-checks
its version/required/closed-object declarations, and checks the fixtures with
the strict typed parser. A Draft 2020-12 evaluator is not currently run. Adding
a locked validator dependency would also require the shared root `Cargo.lock`,
which the authoritative workstream registry lists as forbidden; this exact
schema-validation gap remains deferred until the lockfile boundary is
authorized. Existing typed parsing and semantic negative tests remain active.

### 2026-09-28 Nagi 0.2 Foundation acceptance

Status: `PASS` for workstream `MODEL-RT-01`. This result covers the provider-
neutral model descriptor, registry, runtime/session contract, deterministic
mock backend, lifecycle, capability and role selection, cancellation and
streaming, health/resource reporting, and the System One extension point. It
does not claim production inference, model weights, or M20 completion.

- Added optional manifest runtime/resource class IDs and generic backend
  capability/class declarations. Existing manifest v1 profiles without the
  additive fields remain accepted.
- Added provider-neutral generation options, stream sink/response, health and
  resource reports, and tests for streaming, mid-stream cancellation,
  unavailable backends, load failure, and repeated load/unload.
- Registry selection supports a configured Granite default when available and
  deterministic Standard-to-Lite fallback when the default is missing.
- System One remains an extension point expressed through open runtime-class
  and capability IDs. No System One model, DecisionProvider, or `jev` code was
  added.
- No weights, generated model caches, mandatory network dependency, or
  production inference implementation were added.

Verification on the pinned arm64 nightly toolchain:

- `cargo test --locked -p nagi-model-manager --all-targets` — PASS: 37 unit,
  2 manifest/schema, and 1 Store API test (40 total).
- `cargo fmt --manifest-path user/nagi-model-manager/Cargo.toml -- --check` —
  PASS.
- `cargo clippy --locked -p nagi-model-manager --all-targets -- -D warnings` —
  PASS.
- `cargo check --locked -p nagi-model-manager --lib` — PASS.
- `./nagi doctor` — PASS: 12 pass, 0 warnings, 0 failures.
- `git diff --check` — PASS.
- Nagi user-target package check was attempted with the repo-pinned patched
  Rust std source. It stops in existing `libc 0.2.174` target declarations
  (`time_t`, `suseconds_t`, and related types missing for `target_os = "nagi"`)
  before compiling this crate. This target issue is outside this Foundation
  acceptance. The Nagi 0.2 specification does not require that target build.
- Root `./nagi fmt` is blocked by existing Servo formatting drift; root
  `./nagi lint` and `./nagi test` fail in existing x86_64 syscall assembly
  under the local arm64 host compiler. The Model Manager's focused format,
  Clippy, and test checks pass. These unrelated workspace/host failures do not
  change the Foundation status.
- Existing CI run `36248290972` for parent commit
  `c9c7076e24b283361abb1f219143d0fa8fd3defd` passed its Ubuntu host and Windows
  launcher jobs. Its Nagi target job failed only at M17 first-web-pixel
  acceptance. M17 remains independently `BLOCKED`.
- Windows host CI run `36397861588` exposed a CRLF-sensitive string removal in
  the new backward-compatibility test for optional manifest classes. The test
  now explicitly builds CRLF input and removes properties without depending on
  line endings; the focused regression and full 40-test package suite pass
  after the fix.
- On implementation commit `b55159a7de475f95ebec2a99b0d91a96a5b9d8d1`, CI run
  `36398859291` passed both Ubuntu host and Windows launcher jobs, including
  workspace tests. At this evidence checkpoint, its separate Nagi target job
  was still building `nagi-init`; that job does not gate MODEL-RT-01.
- Full Draft 2020-12 schema evaluation remains deferred under the existing
  shared `Cargo.lock` ownership restriction; this check is not an acceptance
  item in the Nagi 0.2 Foundation specification.

M20 remains `NOT STARTED`, pending its own milestone sequence and real local
Granite response acceptance. The remaining runtime work is future production
integration described in the specification.
