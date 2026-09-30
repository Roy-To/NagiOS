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
  and generated tree state. Patch `0001-nagi-gguf-noexceptions.patch` is now a
  bounded adaptation of `ggml/src/gguf.cpp` plus its upstream `test-gguf.cpp`
  tests; the pinned upstream checkout remains clean.
- The GGUF adaptation applies stricter Nagi-only limits (1 MiB strings, 1 MiB
  array elements, 32K tensors, 4K key/value pairs, and 64 MiB serialized
  metadata), checks allocation sizes before resizing, and returns parser
  failures without C++ exceptions. File writing reports `fwrite` and `fflush`
  failures and copies tensor data in 8 KiB chunks rather than allocating a
  whole-tensor temporary buffer. A closed-descriptor regression verifies a
  buffered flush error is surfaced. STL allocator exhaustion still cannot be
  recovered safely under the current no-unwinder ABI, so these bounds do not
  establish full model-loader OOM safety.
- On 2026-09-30, `./nagi fetch` applied and validated the numbered patch while
  leaving `third_party/llama.cpp` clean. CPU-only Nagi-target CMake build of
  `ggml-base` passed with `-fno-exceptions`; host `test-gguf` passed 101/101,
  and a host build with `__NAGI__` enabled exercised the target-only count
  limits and passed 103/103. Logs are retained under
  `out/evidence/m20-gguf-noexcept-20260930/`.
- The generated llama.cpp patch clone normalizes source and destination paths
  for Git on Windows while keeping extended paths available to filesystem
  APIs. Its regression fixture canonicalizes the temporary repository root;
  the targeted patch test and current 135-unit/20-integration CLI suite pass
  on the host.
- The full Nagi-target `llama` build still fails. With `ninja -k 0`, 29 object
  targets failed and diagnostics covered 57 distinct source files, including
  backend registration, shared model loading, vocabulary/tokenizer parsing,
  grammar, memory/KV-cache, mmap, and model constructors. Unity builds repeat
  some diagnostics; the exact compiler log is
  `out/evidence/m20-gguf-noexcept-20260930/build-llama-attempt2.log`. These
  paths include ordinary malformed-model and runtime failure handling. A
  blanket throw-to-abort conversion, disabled checks, or omitted model sources
  would not preserve the required behavior. The correct next step is explicit
  no-exception status propagation across those APIs; no complete target backend
  or inference is claimed.
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
profile. The patch applier was separately exercised against a temporary Git
source tree: numeric-order patches applied to an isolated clone, the pristine
source stayed unchanged, and tampering with the generated tree was rejected.
Three focused llama.cpp patch/lock tests, all 119 CLI unit tests, and all 18
CLI integration tests passed; CLI Clippy with warnings denied, formatting,
`git diff --check`, and `./nagi fetch` passed. The 2026-09-30 GGUF parser and
writer slice added afterward passed the `test-gguf` results above, and
`ggml-base` compiled for the Nagi target. Full `llama` target compilation and
M20 QEMU inference acceptance have not passed. The runtime hash check was
additionally tested against deterministic host artifacts and compiled for the
Nagi `no_std` target.

## Remaining acceptance blockers

1. Extend the tested GGUF slice into explicit no-exception status propagation
   through the pinned llama.cpp backend registration, tokenizer, grammar,
   memory/KV, mapping, loader, and supported model paths. Then build the CPU
   backend and connect it through the provider-neutral runtime. The current
   full-target diagnostics identify 29 failing object targets; the checkout has
   no Nagi C ABI adapter, complete target build, or backend session.
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
