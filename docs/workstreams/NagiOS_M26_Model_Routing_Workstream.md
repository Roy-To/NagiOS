# Nagi OS M26 — Model and Provider Routing Workstream

Status: `PARTIAL` (deterministic routing contract; bundled-model and guest runtime acceptance pending)

## Scope

M26 adds capability- and role-oriented routing across provider paths while
preserving IBM Granite 4.2 3B as the configured Standard default. The selector
must use manifest capabilities, roles, resource bounds, target compatibility,
offline policy, model lifecycle, and live provider availability. Model/vendor
identity is data supplied by manifests and policy; the routing implementation
does not rank IBM, Alibaba, Google, or future providers by name.

This workstream builds on `user/nagi-model-manager` and the generative and
decision provider contracts. Selection returns an advisory target only. It
does not grant authority or bypass deterministic validation, policy,
permissions, or execution boundaries.

## Implemented contract

`user/nagi-model-manager/src/routing.rs` adds:

- an explicit runtime availability registry keyed by provider identity;
  providers without a reported state, or reported unavailable/disabled, are
  excluded;
- deterministic candidate ordering: available providers before degraded
  providers, then a compatible user preference, the configured role default,
  the existing Standard/Lite role order, and stable provider/model IDs;
- capability, role, context, resource, architecture, local/offline, model
  lifecycle, artifact/backend availability, and provider-health checks;
- strict manual override validation. An override that is unknown, incompatible,
  or unavailable returns a typed error instead of silently choosing a different
  model;
- typed Generative and Decision route results. Decision routing can use a
  separately supplied generative-adapter selection only after specialized
  Decision candidates are exhausted. When neither path is available, automatic
  routing returns `DeterministicOrManual`;
- an explicit `ProviderHealth::Degraded` state, accepted only after all
  `Available` candidates in the same route have been considered.

Granite's Standard-default behavior comes from `RoleDefaultPolicy`, not a
vendor-specific branch in the selector. Qwen, Gemma, and future providers use
the same manifest and availability checks.

## Verification evidence

Focused selector tests cover configured Granite default selection, fallback
when its provider is unavailable, strict manual override behavior, specialized
Decision routing, the explicit generative Decision adapter fallback, and the
fail-closed behavior for an unreported provider.

- `./nagi test` — PASS for the full configured host workspace, including 57
  Model Manager unit tests, 2 manifest/schema tests, and 1 store API test.
- `nagi-cli::llama_cpp::tests::model_artifact_locks_match_manifest_fixtures`
  — PASS; all three lock entries match their manifest fixture metadata.
- `./nagi lint` — PASS with warnings denied for the configured host workspace.
- `cargo -Z build-std=core,alloc check --locked --offline -p
  nagi-model-manager --target targets/x86_64-unknown-nagi-user.json` — PASS.
- `./nagi fmt` and `./nagi build` — PASS.
- `./nagi m19` — PASS; live guest VFS metadata and stable Object ID survived
  rename, remount, and QEMU restart (`out/logs/m19-vfs-objectid-initial.log`).
- `./nagi m22` — PASS across three guest boots; file actions, grouped NH16
  transactions, Activity Ledger, Undo, and restoration passed
  (`out/logs/m22-history-1790873102354253000-boot-1.log` through
  `out/logs/m22-history-1790873102354253000-boot-3.log`).

Host tests use the existing orchestration-only in-memory artifact catalog and
synthetic integrity metadata. They do not install, download, or claim to verify
Qwen or Gemma model bytes.

## 2026-10-02 artifact pin checkpoint

`third_party/models.lock` and the Qwen/Gemma manifest fixtures now record
immutable upstream revisions, filenames, advertised byte sizes, SHA-256
values, and license/notice references. Regression checks pin the exact
revision, file, size, digest, and license values in the manifest fixtures and
ensure each lock entry matches its corresponding fixture.

- Qwen3 4B uses `Qwen/Qwen3-4B-GGUF` revision
  `bc640142c66e1fdd12af0bd68f40445458f3869b`, file
  `Qwen3-4B-Q4_K_M.gguf`, 2,497,280,256 bytes, and the repository-advertised
  SHA-256 `7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5`.
  The fixture references Apache-2.0 and does not require acknowledgement.
- Gemma 3 1B uses the public GGUF conversion `ggml-org/gemma-3-1b-it-GGUF`
  revision `f9c28bcd85737ffc5aef028638d3341d49869c27`, file
  `gemma-3-1b-it-Q4_K_M.gguf`, 806,058,240 bytes, and the repository-advertised
  SHA-256 `8ccc5cd1f1b3602548715ae25a66ed73fd5dc68a210412eea643eb20eb75a135`.
  The fixture links Google's Gemma Terms of Use and records that user
  acknowledgement is required.

The sizes and digests above came from upstream repository file metadata. The
model files were not downloaded, so neither digest has been independently
verified against streamed bytes. The weights are not included in this
repository or installed in a guest. The Gemma distribution terms/notice package
has not been completed or reviewed, and neither model has passed installation,
loading, or inference acceptance.

## Remaining blockers

- Qwen3 4B and Gemma 3 1B now have immutable source pins and repository-
  advertised artifact metadata, but the model bytes are not present and the
  digests are not independently verified. The manifests remain examples rather
  than installable packages; Gemma's required distribution terms/notice package
  also needs completion and review. No weights were added to this repository.
- Granite has a pinned upstream artifact revision and verified digest, but no
  installed guest artifact or production Nagi-target llama.cpp/GGUF backend is
  available for inference acceptance.
- The new selector is a Model Manager contract. It is not yet wired into a
  guest AI service that reports live provider health, switches loaded models,
  and dispatches the chosen path to the real Generative/Decision providers.
- M26's user-facing custom-model entry point and model-switching workflow have
  not been implemented or accepted in QEMU.
- No target/guest test has demonstrated Qwen, Gemma, automatic switching, or
  Decision-versus-Generative routing with real provider responses.

Therefore this workstream provides the deterministic selection foundation but
does not satisfy M26 acceptance; its status remains `PARTIAL`.
