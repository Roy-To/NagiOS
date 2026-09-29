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
- The M19 QEMU fixture now composes this action on the guest: it parses a
  bounded `NagiPlan@1`, resolves fixture context, validates the registered
  action, checks the fixture capability, executes against the real persistent
  VFS-backed SearchService, and verifies the returned stable ObjectId. A
  foreign fixture caller is denied. This policy remains local to the
  acceptance fixture; kernel Channels are not exposed to user processes, so
  there is still no authenticated production caller provider.
- The M22 QEMU fixture now registers a fixture-scoped `file.move` Action. One
  bounded plan names three stable Object IDs and three fixed destination
  basenames. ContextResolver supplies only those fixture objects; Validator
  checks the registered Modify action, `files.move` capability, objects, and
  bounded parameters; Executor acquires the private fixture grant and trusted
  fixture handles before invoking the handler. The handler writes NH16
  Prepared before the three real guest VFS renames and writes Committed after
  flush. A fresh-disk guest run verifies the committed archive after remount,
  then the next boot undoes the three moves through History. This is
  deterministic acceptance input, not model inference, and the caller/policy
  is private to this fixture rather than authenticated production authority.
- No production guest init service currently constructs this registry. General
  app launch, file copy/move, and volume handlers remain absent; the production
  target policy and Context authorities are not connected.

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

Result: 23 orchestration and SearchService integration tests passed; no doc
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
for the Nagi `no_std` user target. The later 2026-09-30 integration run passed
`./nagi m19` with the M21 Plan/Validate/Execute marker in
`out/logs/m19-vfs-objectid-initial.log`. The three-boot `./nagi m22`
regression also passed and included the action marker on every boot in
`out/logs/m22-history-boot-1.log` through `m22-history-boot-3.log`. This uses a
fixture-scoped policy, not an authenticated app identity or production
capability provider; no local model inference was involved.

## Remaining acceptance blockers

1. Expose user-space Channel endpoints and bind `ActionPolicy` and
   `ContextAuthority` to authenticated guest caller capabilities and object
   handles. The library intentionally has no allow-all production provider.
2. Register `file.search` and general first-party actions in the running
   production AI service with that authenticated provider. The bounded M22
   fixture `file.move` action is not a production service handler. Add real
   `app.launch`, `file.copy`, `file.move`, and `system.volume.set` handlers
   against their existing first-party services.
3. Connect Context Resolver and Planner to the running Nagi AI/model service,
   including provider-unavailability fallback in the UI/service path.
4. Add guest acceptance for malformed and unsupported plans, capability and
   object denial, successful real action execution, and partial failure.

M21 stays `PARTIAL`; test-only policy or action mocks cannot satisfy the guest
acceptance gate.
