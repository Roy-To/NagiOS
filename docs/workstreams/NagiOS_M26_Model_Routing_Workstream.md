# Nagi OS M26 — Model and Provider Routing Workstream

Status: `PARTIAL` (deterministic routing contract; Qwen artifact integrity verified in a disposable guest; Gemma acceptance and runtime/inference pending)

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

On 2026-10-02, `./nagi fmt`, `./nagi test`, `./nagi lint`, and `./nagi build`
passed. The configured `nagi-cli` target test run passed 167 unit tests and 21
integration tests. The Qwen and Gemma model-lock/manifest contract test passed.
The `nagi-init` Nagi target build passed for both M26 artifact-acceptance
features.

The M26 Qwen guest artifact acceptance and the M19/M22 guest regressions are
recorded under the 2026-10-02 checkpoints below. Host orchestration tests still
use the existing in-memory artifact catalog and synthetic integrity metadata;
they do not claim model loading or inference.

Fresh regression run evidence:

- `./nagi m19` verified the previous-boot snapshot, live VFS Object ID rename
  and restart, Search persistence, and semantic-index persistence. Its evidence
  and before/after fixed-name User Data copies are under
  `out/evidence/m19-qwen-regression-pass-20261002/`; the pre-existing M19 User
  Data file was restored byte-for-byte after the run.
- `./nagi m22` passed three boots covering VFS Move/Copy, grouped NH16
  transactions, Activity Ledger, Undo, and restored state. Its verified
  evidence manifest is at
  `out/evidence/m22-regression-m26-qwen-20261002/`.

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
Qwen artifact was subsequently downloaded at its locked revision into the
ignored host cache and independently hashed. On 2026-10-02,
`./nagi m26-qwen out/cache/models/Qwen3-4B-Q4_K_M-bc640142c66e1fdd12af0bd68f40445458f3869b.gguf`
passed: host size/SHA-256 validation, QEMU System A boot, guest read-only Model
Store size and GGUF-magic checks, and full guest SHA-256 verification. Evidence
and its verified four-file manifest are at
`out/evidence/m26-qwen-artifact-1790897766859340000/`; the disposable image is
`out/artifacts/nagi-0.1-m26-qwen-1790897766859340000.qcow2`. The Qwen weights
were not added to the repository or M30 release image. This proves artifact
integrity and guest readability only; it does not load a backend or perform
inference.

The Qwen/Gemma Model Store command and guest verification paths are covered by
`nagi-cli` tests. The M26 Gemma target feature compiled, but the Gemma weights
were not downloaded, copied into a guest, or used. The CLI requires
`--accept-gemma-terms` from its invoker before artifact acceptance; no such
acknowledgement was given during this run. Google's current Gemma Terms of Use
state that using or reproducing Gemma is subject to the agreement and require
an accompanying terms/use-restrictions package for distribution
([official terms](https://ai.google.dev/gemma/terms)). The Gemma terms and
notice distribution package has not been completed or reviewed.

## Remaining blockers

- Qwen3 4B has an independently verified digest and passed read-only guest
  Model Store acceptance, but it has not been loaded or used for inference. The
  model package and user-facing installation path remain incomplete. Gemma 3
  1B still needs a user terms acknowledgement before its weights can be
  obtained or tested, and its required distribution terms/notice package needs
  completion and review. No model weights were added to the repository or M30
  release image.
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
