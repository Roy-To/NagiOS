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

- `cargo test --locked --offline -p nagi-model-manager --all-targets` — PASS:
  44 unit tests, 2 manifest/schema tests, and 1 store API test.
- `cargo clippy --locked --offline -p nagi-model-manager --all-targets -- -D warnings` — PASS.
- `cargo -Z build-std=core,alloc check --locked --offline -p nagi-model-manager --target targets/x86_64-unknown-nagi-user.json` — PASS.
- Formatting: `cargo fmt --package nagi-model-manager` — PASS.

Host tests use the existing orchestration-only in-memory artifact catalog and
synthetic integrity metadata. They do not install, download, or claim to verify
Qwen or Gemma model bytes.

## Remaining blockers

- Qwen3 4B and Gemma 3 1B are still illustrative, non-installable manifest
  examples. They lack pinned source revisions, exact artifact sizes, verified
  digests, and a verified installable license/notice package. No weights were
  added to this repository.
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
