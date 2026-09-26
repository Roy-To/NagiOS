# Model Runtime / Model Store Foundation Workstream

Status: `IN_PROGRESS`

## Goal and boundaries

Implement a typed, user-space foundation for model manifests, discovery,
compatibility, selection, provider invocation contracts, and local Model Store
metadata. The workstream does not fetch model weights or implement the
production inference engine. M17, Servo, Capability policy, App SDK, Activity,
Wayback, and other concurrent workstreams remain separately owned. M20 stays
`NOT STARTED` until its milestone acceptance, including a real local Granite
response inside Nagi, is met.

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
   manifest cannot mutate already registered entries.
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

Status: `IN_PROGRESS`. The foundation slice is implemented and focused checks
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
  source files were changed. `.dev` and DF-01 workstream state tooling were not
  present in the inspected checkout. `implementation_status.md` keeps M20
  `NOT STARTED`.

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
