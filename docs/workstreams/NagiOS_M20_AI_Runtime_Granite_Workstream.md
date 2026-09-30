# Nagi OS M20 — AI Runtime / Granite

**Status: PARTIAL**

## Provenance and scope

This work continues from the M18 browser acceptance base and selectively
reuses the existing Model Runtime / Model Store Foundation. The shared
`nagi-model-manager` contract is a user-space `no_std` library. Its providers
return untrusted output and do not grant operating-system authority.

The manager's fake test backend remains test-only. The Granite profile now
pins IBM's official GGUF repository snapshot, the Q4_K_M file, its upstream
SHA-256/size metadata, and Apache-2.0 notice metadata. The 2.24 GB artifact
was streamed from the pinned revision directly into SHA-256 verification; the
observed digest matched the profile. The model file was not retained or
installed. `ModelRuntime::load` independently performs that check on artifact
bytes before calling any backend.

## Implemented and verified

- Strict, versioned model-manifest parsing and semantic validation.
- Capability, role, backend, artifact, context, and resource-aware deterministic
  model selection.
- Lifecycle checks that reject incompatible backend combinations and prevent
  unregistering active or transitional models.
- Bounded artifact reads, provider-neutral generation/session interfaces,
  cancellation/deadline results, and local Model Store install/update/removal
  metadata contracts.
- Runtime load streams artifact bytes through a fixed 8 KiB SHA-256 buffer and
  rejects a content mismatch before invoking the backend. Tests cover changed
  bytes, digest comparison, and short reads over a multi-chunk artifact.
- `third_party/sources.lock` pins llama.cpp release commit
  `c85b92c69c955961621193cd51da194f3cbcedf3`; `nagi fetch` retrieves that exact
  clean source checkout. Nagi-owned patches now have a numbered patch
  directory and deterministic application path: the pinned upstream checkout
  remains clean, while a generated patched tree is kept under ignored
  `out/cache/llama-cpp-nagi` and validated against both the patch fingerprint
  and generated tree state. The patch directory currently contains no
  compatibility patch, so this is patch infrastructure, not a claim that the
  C++ backend is built for Nagi.
- A 2026-09-30 CMake configuration probe using the Nagi x86-64 target compiler
  and CPU-only/static options passed compiler detection and configuration.
  Building target `llama` then failed in upstream `ggml/src/gguf.cpp`: its
  parser and writer use C++ exception syntax, while the Nagi target wrapper
  intentionally passes `-fno-exceptions` because Nagi has no exception
  unwinder. Exact configure/build logs are retained under
  `out/m20-llama-target-probe-2026-09-30/`. The pinned checkout remains clean;
  no target library was linked and no runtime/inference is claimed.
- A scan of the configured target's 73-entry `compile_commands.json` found
  exception syntax/tokens in 20 selected CPU-path translation units, including
  `gguf.cpp`, `llama-context.cpp`, `llama-grammar.cpp`,
  `llama-model-loader.cpp`, and `unicode.cpp`. The checkout has no
  `GGML_NO_EXCEPTIONS` compatibility branch. The failure therefore extends
  beyond the first parser file; removing catches or turning throws into
  no-ops would discard upstream allocation, parse, and I/O error handling.
  No Nagi-owned compatibility patch was made because a correct conversion
  needs an explicit no-exception error path across the selected loader/inference
  sources, not a syntax shim. The earlier count of 22 was not accurate. The next safe
  experiment is to adapt one bounded upstream API
  boundary with explicit status returns, then rebuild and test that slice
  before expanding the patch.
- Granite Q4_K_M source metadata pins repository commit
  `c40945d71cd90f249a56985e8155551a9188dc30`, upstream size
  `2,244,011,552` bytes, digest
  `e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5`, and
  Apache-2.0 notice metadata. A fresh streamed fetch produced that exact
  SHA-256; no model bytes are retained in the worktree or bundled.
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

Result: 40 unit tests, 2 manifest/schema tests, and 1 external Store API test
passed (43 total). The CLI regression suite also passed 114 unit and 18
integration tests.

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

The Granite source bytes were streamed to a local SHA-256 process without
writing a 2.24 GB artifact file; the calculated digest matched the pinned
profile. `./nagi fetch` also passed and validated the clean llama.cpp checkout
at the locked revision with the currently empty Nagi patch directory. The new
patch applier was separately exercised against a temporary Git source tree:
numeric-order patches applied to an isolated clone, the pristine source stayed
unchanged, and tampering with the generated tree was rejected. Three focused
llama.cpp patch/lock tests, all 119 CLI unit tests, and all 18 CLI integration
tests passed; CLI Clippy with warnings denied, formatting, `git diff --check`,
and `./nagi fetch` passed. No M20 QEMU inference acceptance was run. The
runtime hash check was additionally tested against deterministic host artifacts
and compiled for the Nagi `no_std` target.

## Remaining acceptance blockers

1. Add and test a reproducible Nagi-owned no-exception adaptation for the
   pinned llama.cpp source, then build its CPU backend and connect it through
   the provider-neutral runtime. The current parser compile failure is concrete
   evidence for this compatibility work; the checkout has no Nagi C ABI
   adapter, target build, or backend session.
2. Add a large-artifact Model Store path. Current guest VFS files are 1 KiB
   bounded and cannot contain the pinned 2.24 GB model; no guest model artifact
   reader or installer is registered.
3. Activate a user-space model service that loads the model lazily, enforces
   measured bounded resource use, and fails safely when the model or backend is absent.
   Granite absence must not prevent Nagi from booting.
4. Add QEMU acceptance that obtains a real Granite response inside Nagi,
   exercises schema-constrained structured output and invalid-output rejection,
   unloads/reloads across restart, and records deterministic resource/failure
   behavior. Host mocks cannot satisfy this gate.

M20 remains `PARTIAL` until a real local Granite response is accepted inside
Nagi. The provider-neutral boundary leaves Decision Providers free to use
future runtimes and does not make Jev or cloud access a dependency.
