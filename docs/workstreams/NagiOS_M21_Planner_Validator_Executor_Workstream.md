# Nagi OS M21 — Planner / Validator / Executor

**Status: PARTIAL**

## Implemented contract

- `schemas/NagiPlan@1.json` defines the versioned, bounded plan envelope.
  Runtime parsing rejects unknown fields, incomplete JSON, unsupported
  versions, oversized documents, and plans outside step/object/intent limits.
- The `no_std` `services/nagi-ai` library resolves caller context through a
  required visibility authority and supplies only visible stable Object IDs to
  a provider. Prompts contain only caller-filtered, bounded Action schemas.
- `ModelManagerPlanAdapter` invokes the existing untrusted
  `GenerativeProvider`; complete output remains a candidate until Validator
  accepts it. `LlmDecisionAdapter` returns only a candidate from the bounded
  action set. Confidence changes fallback routing only.
- Validator checks registered actions, action-specific parameter names/types/
  bounds, allowed object IDs, visibility and capability policy before any
  executor step begins. Paths and shell/command parameters are excluded.
- Executor obtains fresh capability grants and object handles for each action,
  bounds the returned result, checks result Object IDs for visibility, and
  reports success/failure/partial completion. It does not claim rollback.
- `register_file_search_action` binds the existing M19 `SearchService` to the
  real `file.search` Action Registry entry. It limits queries to 128 bytes,
  returns at most 64 visible Object IDs, and delegates visibility to the
  SearchService's injected filter. The host integration test runs a plan
  through validation and execution and confirms another app's private file is
  omitted. Its in-memory backend and filter are test-only.
- No guest init service currently constructs this registry. App launch, file
  copy/move, and volume handlers remain absent; the production target policy
  and Context authorities are also not connected.

## Verification

Using the pinned aarch64 macOS Rust toolchain:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m21-host-arm64 \
/Users/tozawa/.cargo/bin/cargo test --locked --offline -p nagi-ai
```

Result: 16 orchestration and SearchService integration tests passed; no doc
tests are defined.

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m21-clippy-arm64 \
/Users/tozawa/.cargo/bin/cargo clippy --locked --offline \
  -p nagi-ai --all-targets -- -D warnings
```

Clippy passed with warnings denied. Formatting passed:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
/Users/tozawa/.cargo/bin/cargo fmt \
  --manifest-path services/nagi-ai/Cargo.toml -- --check
```

The service compiled for Nagi `no_std` user target:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m21-target \
/Users/tozawa/.cargo/bin/cargo -Z build-std=core,alloc check \
  --manifest-path Cargo.toml -p nagi-ai \
  --target targets/x86_64-unknown-nagi-user.json --locked --offline
```

The `nagi-ai` service, including the SearchService action adapter, compiles
for the Nagi `no_std` user target. No M21 QEMU acceptance was run. The adapter
is a real service composition, but its test's in-memory backend does not claim
guest filesystem action execution.

## Remaining acceptance blockers

1. Register the `file.search` action in the running guest AI service and
   connect it to a SearchService with authenticated capability-bound caller
   context. Add real `app.launch`, `file.copy`, `file.move`, and
   `system.volume.set` handlers against their existing first-party services.
2. Bind `ActionPolicy` and `ContextAuthority` to authenticated guest caller
   capabilities and object handles. The library intentionally has no
   allow-all production provider.
3. Connect Context Resolver and Planner to the running Nagi AI/model service,
   including provider-unavailability fallback in the UI/service path.
4. Add guest acceptance for malformed and unsupported plans, capability and
   object denial, successful real action execution, and partial failure.

M21 stays `PARTIAL`; test-only policy or action mocks cannot satisfy the guest
acceptance gate.
