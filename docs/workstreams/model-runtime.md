# Model Runtime / Model Store Foundation Workstream

Status: `PARTIAL` (provider-neutral contract foundation; production model discovery/install lifecycle remains; M20 inference acceptance is recorded in the Granite workstream)

## Goal and boundaries

Implement a typed, user-space foundation for model manifests, discovery,
compatibility, selection, provider invocation contracts, and local Model Store
metadata. The workstream does not fetch model weights or implement the
production inference engine. M17, Servo, Capability policy, App SDK, Activity,
Wayback, and other concurrent workstreams remain separately owned. The M20
target integration now passes real local Granite inference inside Nagi. This
contract-only workstream remains `PARTIAL` because it does not provide a
production model discovery, installation, or service lifecycle. See
`NagiOS_M20_AI_Runtime_Granite_Workstream.md` for the acceptance evidence.

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
   and lifecycle contracts. Qwen3 4B and Gemma 3 1B remain non-installable
   examples with illustrative context/resource bounds. The Granite 4.2 3B
   profile separately pins IBM's Q4_K_M GGUF repository revision, upstream
   byte length and SHA-256 metadata, and Apache-2.0 notice; the model bytes are
   not present in this source tree.

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

Status: `PARTIAL`. The foundation slice is implemented and focused checks pass.
This workstream has no production inference backend or real target/VM inference
acceptance, so it does not satisfy M20 by itself.

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
  `implementation_status.md` keeps M20 `PARTIAL` until local Granite inference
  is accepted in Nagi.

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

### 2026-09-30 artifact verification boundary

The Granite 4.2 3B profile now identifies the official Q4_K_M GGUF artifact at
Hugging Face snapshot `c40945d71cd90f249a56985e8155551a9188dc30`. Resolved file
metadata reports `2,244,011,552` bytes and SHA-256
`e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5`; the
profile records Apache-2.0. The pinned upstream file was streamed through a
SHA-256 process, and its observed digest matched. The large artifact was not
retained locally or installed.

`ModelRuntime::load` independently hashes the bytes supplied by its
`ModelArtifactReader`, using a fixed 8 KiB buffer and rejecting mismatches
before backend load. The pinned llama.cpp commit
`c85b92c69c955961621193cd51da194f3cbcedf3` is registered in
`third_party/sources.lock` and fetched into an ignored clean checkout by
`nagi fetch`. Target C++ integration, a guest store capable of supplying a
2.24 GB artifact, and real Granite inference remain unimplemented; M20 remains
`PARTIAL`.

### 2026-10-08 Granite inference acceptance update

The separate M20 target runtime now passed `./nagi m20-granite-inference` in
QEMU run `1791393002742512000`. It loaded the locked Granite artifact through
the read-only Model Store, generated a structured Japanese response inside
Nagi, and passed the `ModelRuntime` schema check. This closes the M20
acceptance criterion; the provider-neutral foundation described by this
document remains partial for production discovery and installation lifecycle.
Full logs and checksums are under
`out/evidence/m20-granite-inference-1791393002742512000/`.
