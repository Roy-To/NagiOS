# Nagi OS M20 — AI Runtime / Granite

**Status: PARTIAL**

## Provenance and scope

This work continues from the M18 browser acceptance base and selectively
reuses the existing Model Runtime / Model Store Foundation. The shared
`nagi-model-manager` contract is a user-space `no_std` library. Its providers
return untrusted output and do not grant operating-system authority.

The change does not claim that the manager's fake test backend is a production
runtime. Granite fixtures remain illustrative, non-installable metadata without
model weights, source/hash verification, or completed license/NOTICE
provenance.

## Implemented and verified

- Strict, versioned model-manifest parsing and semantic validation.
- Capability, role, backend, artifact, context, and resource-aware deterministic
  model selection.
- Lifecycle checks that reject incompatible backend combinations and prevent
  unregistering active or transitional models.
- Bounded artifact reads, provider-neutral generation/session interfaces,
  cancellation/deadline results, and local Model Store install/update/removal
  metadata contracts.
- The fake provider is exercised only by tests of orchestration and lifecycle.

## Verification evidence

Using the pinned arm64 nightly toolchain:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m20-host-arm64 \
/Users/tozawa/.cargo/bin/cargo test --locked --offline \
  -p nagi-model-manager --all-targets
```

Result: 37 unit tests, 2 manifest/schema tests, and 1 external Store API test
passed (40 total).

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m20-clippy-arm64 \
/Users/tozawa/.cargo/bin/cargo clippy --locked --offline \
  -p nagi-model-manager --all-targets -- -D warnings
```

Clippy passed with warnings denied. Package formatting passed:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
/Users/tozawa/.cargo/bin/cargo fmt \
  --manifest-path user/nagi-model-manager/Cargo.toml -- --check
```

The library compiled for the Nagi user target with `no_std` core/alloc:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m20-target \
/Users/tozawa/.cargo/bin/cargo -Z build-std=core,alloc check \
  --manifest-path Cargo.toml -p nagi-model-manager \
  --target targets/x86_64-unknown-nagi-user.json --locked --offline
```

No M20 QEMU inference acceptance was run.

## Remaining acceptance blockers

1. Pin and integrate a Nagi-compatible llama.cpp source revision and target
   backend. The M18 base has no llama.cpp source or backend adapter.
2. Provide a verified, licensed Granite 4.2 3B GGUF artifact in the Nagi Model
   Store. Current sample metadata is explicitly non-installable.
3. Activate a user-space model service that loads the model lazily, enforces
   bounded resource use, and fails safely when the model or backend is absent.
   Granite absence must not prevent Nagi from booting.
4. Add QEMU acceptance that obtains a real Granite response inside Nagi,
   exercises schema-constrained structured output and invalid-output rejection,
   unloads/reloads across restart, and records deterministic resource/failure
   behavior. Host mocks cannot satisfy this gate.

M20 remains `PARTIAL` until a real local Granite response is accepted inside
Nagi. The provider-neutral boundary leaves Decision Providers free to use
future runtimes and does not make Jev or cloud access a dependency.
