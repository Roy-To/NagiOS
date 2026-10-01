# Nagi OS M20 — AI Runtime / Granite

**Status: PARTIAL**

## Provenance and scope

This work continues from the M18 browser acceptance base and selectively
reuses the existing Model Runtime / Model Store Foundation. The shared
`nagi-model-manager` contract is a user-space `no_std` library. Its providers
return untrusted output and do not grant operating-system authority.

The manager's fake test backend remains test-only. The Granite profile pins
IBM's official GGUF repository snapshot, the Q4_K_M file, its upstream
SHA-256/size metadata, and Apache-2.0 notice metadata. An earlier 2.24 GB
streamed digest check matched without retaining a file. On 2026-10-02, the
exact locked artifact was downloaded to the ignored local cache and its size
and SHA-256 were verified again; the file is not installed in the guest Model
Store or loaded by Nagi. `ModelRuntime::load` independently hashes artifact
bytes before calling any backend.

## Implemented and verified

- Strict, versioned model-manifest parsing and semantic validation.
- Capability, role, backend, artifact, context, and resource-aware deterministic
  model selection.
- Lifecycle checks that reject incompatible backend combinations and prevent
  unregistering active or transitional models.
- Bounded artifact reads, provider-neutral generation/session interfaces,
  cancellation/deadline results, and local Model Store install/update/removal
  metadata contracts. The guest Model Store now has a separate read-only GPT
  capability and a no_std FAT32 artifact reader; the writable User Data
  capability remains unchanged.
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
  the targeted patch test and current 135-unit/21-integration CLI suite pass
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

The Granite source bytes were initially streamed to a local SHA-256 process
without writing a 2.24 GB artifact file; the calculated digest matched the
pinned profile. The exact file was later downloaded and independently
verified in the ignored cache; it is not installed in the guest. The patch
applier was separately exercised against a temporary Git
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
   through tokenizer, memory/KV, mapping, loader, and supported model paths.
   Grammar parser/runtime failure propagation is now patched, host-tested, and
   its parser translation unit compiles for the Nagi target. Then build the CPU
   backend and connect it through the provider-neutral runtime. The latest
   fresh full-target attempt identifies 26 failing object targets and 55
   distinct source files after backend registration and the grammar parser
   compile; the checkout has no Nagi C ABI adapter, complete target build, or
   backend session.
2. Connect the artifact reader to a trusted Model Store catalog/installer and
   provide the external placement workflow. Model Store files use the root
   short name formed from the first 40 bits of `SHA256(UTF8(artifact_id))` as
   Crockford Base32 plus `.GGF`; the manifest digest remains authoritative.
   The current M30 image has an empty Model Store, and no installer or model
   bytes are bundled. The existing User Data VFS remains 1 KiB bounded and is
   not used for model artifacts.
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

## Backend-registration no-exception compile slice — 2026-10-01

The preceding fresh llama.cpp target build had one isolated failure in
`ggml-backend-reg.cpp`: its logging-only `path_str()` helper used `try/catch`
to turn filesystem encoding conversion errors into an empty diagnostic string.
That C++ exception syntax is unavailable to Nagi's no-unwinder target. Patch
`third_party/llama-cpp-patches/0002-nagi-backend-reg-noexceptions.patch` now
uses the filesystem's native UTF-8 bytes only under `__NAGI__`; other targets
retain the original `u8string()` conversion and exception fallback. This keeps
the logged path bytes unchanged for Nagi and leaves the clean pinned upstream
checkout untouched.

The target compiler reproduced the original exception-syntax error before the
patch, then compiled the same translation unit after it. `./nagi fetch` applied
both numbered patches and regenerated the checkout. A fresh CPU-only CMake
configuration with `GGML_BACKEND_DL=OFF` built the static Nagi-target `ggml`
target 31/31, including `ggml-base`, `ggml-cpu`, and `ggml-backend-reg.cpp`.
The full `llama` target was then built with Ninja keep-going: backend
registration no longer fails, but 28 object targets still fail. Exception
diagnostics span 63 source files; the same attempt also found two explicit RTTI
use sites and a missing target `PATH_MAX` definition. The build and CMake
evidence is preserved in
`out/evidence/m20-backend-reg-noexceptions-20261001/`, and the previous
generated checkout is preserved at
`out/cache/llama-cpp-nagi-before-backend-reg-20261001/`.

`./nagi fmt`, `./nagi lint`, `./nagi test`, `./nagi build`, and the three
focused llama patch/lock tests passed. M20 QEMU inference acceptance was not
run because the complete `llama` target and provider backend are not available;
no inference result is claimed. M20 remains `PARTIAL`.

## Nagi RTTI and path-capacity boundary — 2026-10-01

Added numbered patch
`third_party/llama-cpp-patches/0003-nagi-model-boundaries.patch`. Under
`__NAGI__`, model-base identification now uses a virtual query whose base
implementation returns null and whose `llama_model_base` implementation
returns itself. This preserves the prior invalid-model failure check without
requiring RTTI; other targets retain upstream `dynamic_cast` behavior. The
same patch makes `llama_path_max()` return 257 bytes for Nagi: the user VFS
accepts paths up to `MAX_PATH_LENGTH = 256`, and POSIX callers need one extra
byte for the NUL terminator. The CLI regression binds that constant to the
patch contract and passed after failing before patch 0003 existed.

`./nagi fetch` generated and validated the full numbered patch series while
the raw pinned llama.cpp checkout stayed clean. The fresh Nagi target build
used the existing static CPU configuration and Ninja keep-going. It no longer
reports the two RTTI failures or the missing `PATH_MAX`; 28 object targets
still fail on exception syntax, with 57 distinct source paths producing 289
`throw` and 15 `try` diagnostics. No throw was converted to abort and no model
source was omitted. The CMake log, patch, generated-checkout marker, cache,
toolchain, and SHA-256 manifest are preserved in
`out/evidence/m20-nagi-boundaries-0003-20261001/`.

The new boundary test and all CLI tests passed (145 unit and 21 integration
tests); warnings-denied Clippy, pinned-nightly formatting, `./nagi fmt`,
`./nagi test`, `./nagi lint`, `./nagi build`, and `./nagi fetch` passed.
Full llama target compilation still fails in the upstream model-loader,
grammar, memory/KV, Unicode, and related error paths because Nagi has no
exception unwinder. There is still no complete llama backend or real local
inference, so M20 remains `PARTIAL` and no QEMU inference acceptance is
claimed.

## Read-only guest Model Store and FAT32 artifact reader — 2026-10-01

Added `ADR-0014` for a separate Model Store capability. Kernel GPT parsing
continues to require exactly one User Data partition, permits at most one
Model Store partition, and validates both extents against all partition
bounds/overlap checks. Syscall block reads translate sectors relative to the
selected capability. Model Store reads are bounded by its exact extent;
block-write and flush still accept only the 8 MiB User Data capability. The
Model Store token is distinct from the writable token and is absent when the
GPT entry is absent. The current bootstrap process receives this capability
alongside existing platform caps; per-service authenticated authority is not
claimed.

`Fat32ArtifactReader` reads 512-byte sectors through a read-only
`ModelStoreSectorReader` trait, validates FAT32 geometry and root entries, and
supports bounded sequential and random reads across fragmented cluster chains.
Artifact names are stable 8.3 names derived from the artifact ID, not from a
host path. The reader returns no pre-verified integrity metadata, so
`ModelRuntime::load` must still hash the model's actual bytes before invoking
any backend. Host tests cover name stability, fragmented reads, offsets,
truncated chains, and absent files. A new integration fixture also passes the
FAT32 reader directly into `ModelRuntime::load`, verifies the matching SHA-256,
and proves a wrong digest is rejected before the backend is called.

The M30 init acceptance build probes the real Model Store GPT capability,
checks the FAT32 BPB/root directory, rejects a block write using the Model
Store token, and confirms the boot sector remains unchanged. If the Granite
GGUF is present it checks the file header; if it is absent, boot continues
successfully. After GPT/User Data initialization succeeds, a missing or
unreadable Model Store capability or invalid FAT32 volume produces a bounded
FAIL diagnostic but does not stop ordinary OS boot; structurally invalid GPT
metadata remains fail-closed. The M30 acceptance gate still requires the PASS
marker. The latest post-assembly two-boot QEMU run is
`out/evidence/m30-release-1790806831243045000/`. The immediately previous
accepted image is preserved at
`out/evidence/m30-release-1790806188358089000/reference-disk-before-runtime-verification.qcow2`.
The Model Store is empty in that image; this is capability and discovery
acceptance, not model loading or inference.

The focused Model Manager suite passes with 48 unit, 2 manifest/schema, and 1
Store API test. The pinned Nagi no_std target check, `./nagi fmt`, `./nagi
lint`, `./nagi test`, `./nagi build`, and two-boot `./nagi m30` acceptance pass.
An initial M30 run placed the new check before the M5 FPU-state gate and
failed; moving it after the FPU round-trip restored the existing startup
acceptance. The failed image and serial log are preserved under
`out/evidence/m30-release-1790804891726864000/`. M20 remains `PARTIAL` because
the 2.24 GB Granite artifact is absent, no installer/catalog service or
complete llama backend exists, and no real in-guest inference has been
accepted.

## Dedicated guest FAT32 reader fixture — 2026-10-01

Added a test-only `m20-fixture-acceptance` init feature and shared deterministic
5,000-byte artifact fixture with ID `nagi.m20.reader-fixture`. The disposable
fixture image writer derives the FAT32 short name using
`nagi-model-manager::model_store_short_name`; the normal reference/release
image writer still leaves Model Store empty. The payload begins with the
`GGUF` magic followed by deterministic bytes, but it is not a valid model.

`./nagi m30` booted the separate 64 GiB fixture qcow2, read the whole artifact
through the guest's read-only Model Store capability in 512-byte chunks,
verified every byte, reread a range across the 4 KiB cluster boundary, and
checked EOF behavior. Serial output contains both the Model Store capability
and fixture-read PASS markers. `qemu-img check` passed on the untouched source
release image, its persistence-test copy, and the fixture image. The source
image SHA-256 remains
`e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`.
README, serial logs, qcow2 files, OVMF variables, and `SHA256SUMS` are retained
under `out/evidence/m30-release-1790809848636521000/`.

This acceptance verifies guest FAT32 artifact discovery and bounded reads
only. It does not exercise GGUF parsing, digest/manifest loading, the llama.cpp
backend, or inference. Both the ordinary Model Store acceptance init and the
dedicated fixture init compiled for the Nagi target. The 146 CLI unit tests,
`./nagi fmt`, `./nagi test`, `./nagi lint`, `./nagi build`, M19/M22 QEMU
regressions, and the three-image M30 QEMU/image checks passed. M20 remains
`PARTIAL`.

## FAT32-to-runtime digest gate — 2026-10-01

`ModelRuntime::validate` previously treated a reader with no cached integrity
metadata as an immediate `IntegrityMismatch`. `Fat32ArtifactReader` correctly
returns no such metadata because the file bytes have not been read yet, so the
real runtime hash gate could never run for that reader. Runtime validation now
rejects cached integrity only when one is present and differs from the
manifest. `ModelRuntime::load` still streams and hashes the actual bytes before
invoking the backend, and missing manifest integrity remains an error.

A deterministic FAT32 fixture reaches `ModelRuntime::load`. The matching
manifest digest succeeds only after sector reads; a wrong digest also performs
the reads, returns `IntegrityMismatch`, and leaves the fake backend load count
at zero. This fake backend test covers orchestration and does not claim model
inference. The focused Model Manager suite passes 48 unit, 2 manifest/schema,
and 1 Store API tests; `no_std` target compilation, repository format/lint/
test/build, and a fresh two-boot M30 target rebuild pass. M20 remains
`PARTIAL` because the actual Granite artifact, complete llama backend,
production catalogue/installer, model service, and real guest inference are
still absent.

## Bootstrap workspace dependency closure — 2026-10-01

CI run `36789984546` failed on Ubuntu, Windows, and Nagi-target bootstrap
jobs because `tools/nagi-bootstrap` compiles the shared `nagi-cli` source with
its own manifest. The M20 fixture filename helper references
`nagi-model-manager`, but only the normal CLI manifest declared that direct
dependency. Added the same path dependency to the bootstrap manifest and
updated its lockfile. `cargo check --locked --offline` and the exact CI
`cargo run --locked --manifest-path tools/nagi-bootstrap/Cargo.toml -p
nagi-bootstrap -- fetch` both compile the bootstrap successfully; the 146 CLI
unit tests and bootstrap formatting check also pass.

The local fetch then stopped at the existing Servo checkout validation, which
reported a dirty pinned checkout and refused to modify it. No Servo files were
changed by this repair. A clean-run CI result is still pending, and this does
not change M20's `PARTIAL` status.


## Chat-template no-exception status — 2026-10-01

Patch `0004-nagi-chat-template-status.patch` moves unknown chat-template
lookup to the existing `LLM_CHAT_TEMPLATE_UNKNOWN` result in the Nagi build.
The `llama.cpp` caller checks that value and returns `-1`, preserving rejection
of unsupported formats. The host build keeps its existing `std::map::at`
exception behavior, while Nagi's detection path no longer compiles `try` or
`catch`.

The numbered 0001–0004 patch series was regenerated into the ignored
`out/cache/llama-cpp-nagi` checkout by `./nagi fetch`. The raw pinned
`third_party/llama.cpp` checkout remains clean. The fetch command subsequently
stopped at the already-dirty pinned Servo checkout and refused to modify it;
the previous generated llama.cpp checkout remains preserved at
`out/cache/llama-cpp-nagi-before-0004-chat/`.

Validation passed: the focused CLI patch-contract test, the complete CLI suite
(148 unit and 21 integration tests), warnings-denied CLI Clippy, repository
format/lint/test/build commands, host and Nagi-branch chat-template status
smokes, and a fresh Nagi-target compile of `llama-chat.cpp.obj`. The full
Nagi-target `llama` keep-going build still fails: 27 object steps and 56 unique
source paths report 289 `throw` plus 14 `try` diagnostics. Before this patch,
28 steps and 57 paths failed with 289 `throw` and 15 `try` diagnostics. The
failure log and smoke evidence are in
`out/evidence/m20-chat-template-0004-20261001/`.

No M20 inference gate was run: the complete backend still does not build and
the M30 Model Store contains no Granite artifact. This patch removes one
compile blocker without claiming a runnable model backend or inference. M20
remains `PARTIAL`.

## Granite artifact release pin — 2026-10-01

Added `[models.granite_4_2_3b]` to `third_party/models.lock` with the exact
`ibm.granite-4.2-3b` GGUF repository revision, filename, 2,244,011,552-byte
size, SHA-256, Apache-2.0 license/notice references, acknowledgement
requirement, and Model Store artifact ID. The revision remains pinned to the
Nagi Standard default; no model/provider default changed.

The new `granite_model_artifact_lock_matches_manifest_fixture` CLI test parses
the checked-in Granite manifest fixture and compares every lock field,
including source, digest, license notice, acknowledgement, and storage
identity. The focused test and complete CLI suite pass. No Granite bytes were
downloaded or inserted into the release image. This pins distribution
metadata only; it does not provide an installer, a complete llama.cpp Nagi
backend, or guest inference. M20 remains `PARTIAL`.

## Structured generation contract — 2026-10-02

`ModelRequest` now has an optional `StructuredOutputSchema`. Requests using
`structured.generate` must include one, and requests using another capability
must omit one. The typed schema parser accepts a bounded JSON Schema subset,
rejecting unknown keywords, arbitrary patterns, duplicate schema keys, excess
size, and excess nesting. It accepts the checked-in `NagiPlan@1` schema using
fixed recognizers for the three specific patterns in that schema. Structured
non-streaming responses are parsed with duplicate-key rejection and checked
against the schema before becoming a `ModelResponse`. The schema is limited to
16 KiB and the generated JSON to 64 KiB. Structured streaming returns
`UnsupportedCapability` until a complete response can be validated before any
chunk reaches its consumer. A global output walk also enforces depth, node,
container, and string limits in unconstrained schema subtrees. The three
patterns in `NagiPlan@1` declare string types explicitly, keeping the supported
pattern subset aligned with JSON Schema's type behavior.

The M21 `ModelManagerPlanAdapter` passes `schemas/NagiPlan@1.json` as this
constraint. The existing Planner parser and Validator still run afterward, so
the output schema does not supply capability, object visibility, or execution
authority. Tests cover accepted plans, wrong schema/type, duplicate keys,
range/shape failures, request/capability mismatch, and refusal to stream an
unvalidated structured result. Verification on 2026-10-02 passed 57 model
manager unit tests, 2 manifest/schema tests, 1 Store API test, 24 `nagi-ai`
tests, warnings-denied package Clippy, Nagi no-std target compilation,
`./nagi fmt/test/lint/build`, and the M22 three-boot QEMU regression. This is
contract/orchestration coverage only; no llama backend, model bytes, or guest
inference is claimed. The planner's 1,024-token output budget matches each
currently bundled model manifest. M20 remains `PARTIAL`.

## Grammar parser and runtime status propagation — 2026-10-02

Numbered patch `0005-nagi-grammar-status.patch` keeps the pinned llama.cpp
checkout clean and converts grammar parse failures into explicit status. It
checks decimal overflow and token-ID bounds, rejects reversed or excessive
repetition counts, malformed escapes, and undefined rule references, and
returns an empty parser state on failure. Runtime grammar acceptance latches
failure, sampler application blocks all candidates, and the sampler returns
`LLAMA_TOKEN_NULL` after a failed grammar transition. The failed state is copied
when cloning a grammar.

The patch intentionally retains upstream regex-triggered lazy grammar support.
The upstream `std::regex` constructor has no status-returning compile API in
this slice, so malformed trigger regex behavior under Nagi's no-exceptions
runtime remains unverified; this patch does not claim to fix that path.

Verification on 2026-10-02 passed `./nagi fetch` with all numbered patches,
three host CTest targets (`test-grammar-parser`, `test-grammar-integration`,
and `test-json-schema-to-grammar`), the focused CLI patch-contract test, and
the Nagi-target `llama-grammar.cpp` object compile with `-fno-exceptions`.
The regenerated grammar parser/integration tests cover integer overflow,
malformed escapes, undefined rules, repetition bounds, and fail-closed runtime
grammar state. The full `llama` build still fails: 26 object targets and 55
distinct source files report exception syntax, recorded in
`out/logs/m20-grammar-status-target-build-20261002.log`. M20 remains `PARTIAL`;
no complete backend or inference is claimed.

## Sampler failure sentinel consumers — 2026-10-02

Patch `0006-nagi-sampler-null-consumers.patch` documents the public
`llama_sampler_sample()` failure result as `LLAMA_TOKEN_NULL` and updates each
direct C++/Swift example caller. Callers stop before checking EOG status,
converting the token to text, or adding it to a subsequent evaluation batch.
The C++ command-line examples report the failure and return nonzero; the Swift
UI stops the response and the batched Swift example exits with an error.

The patch applies cleanly to the clean pinned llama.cpp revision, and its CLI
contract test, 157 CLI unit tests, 21 CLI integration tests, four C++ host
example builds, and both Swift typechecks passed. `./nagi fetch` did not reach
llama.cpp: it stopped while validating an existing generated Servo checkout
that has a mismatched patch state. That checkout was left untouched. M20 stays
`PARTIAL`; this patch does not provide a Nagi model consumer, backend, or
inference result.

## Hybrid state-restore rollback regression — 2026-10-02

Patch `0007-nagi-hybrid-state-restore-rollback.patch` adds a case to the
existing llama.cpp state-restore failure suite. For generated hybrid models,
the test removes the last byte of a serialized sequence state, causing the
recurrent suffix read to fail after the attention state has been restored. It
then verifies that the failed operation leaves sequence 0 empty and does not
change sequence 1 logits. The test also compares sequence 0's serialized state
size with its empty baseline so leftover attention entries cannot pass solely
because the recurrent position is empty. The source patch applies cleanly to
the pinned llama.cpp revision. The patched host test target builds, a generated
`granitehybrid-dense.gguf` fixture ran the new case and all nine state tests
passed; the log is `out/logs/m20-hybrid-state-restore-20261002.log`. This
confirms host rollback behavior only; it does not establish no-exceptions
status propagation or guest inference. M20 remains `PARTIAL`.

## Granite artifact download and patch 0007 checkout — 2026-10-02

Downloaded the exact Granite 4.2 3B Q4_K_M artifact pinned in
`third_party/models.lock` to the ignored cache at
`out/cache/models/granite-4.2-3b-Q4_K_M-c40945d71cd90f249a56985e8155551a9188dc30.gguf`.
Its verified size is 2,244,011,552 bytes and its SHA-256 is
`e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5`, matching
the lock. The evidence manifest at
`out/evidence/m20-granite-model-download-20261002/SHA256SUMS` covers the README
and downloaded bytes. The file is local cache only; it has not been added to a
Model Store image or loaded on Nagi.

Before regenerating, the checkout with patches 0001–0006 was preserved at
`out/cache/llama-cpp-nagi-before-0007-20261002/`. `./nagi fetch` generated a
fresh pinned checkout with patches 0001–0007 and a matching marker. It then
stopped with exit 4 at an existing mismatched generated Servo source state;
that checkout was not modified. The fetch log is
`out/logs/nagi-fetch-m20-0007-20261002.log`. Patch 0007 adds only host
hybrid-state rollback coverage; it does not resolve the target's exception
paths or establish inference.

The model is still absent from guest Model Store, the Nagi target `llama`
build remains incomplete, and there is no model service or inference result.
M20 remains `PARTIAL`.

The fresh patch-0007 host `test-save-load-state` target built and passed all
nine state tests against the generated `granitehybrid-dense.gguf` fixture.
This confirms host rollback behavior only. Its test log is
`out/logs/m20-hybrid-state-restore-patch0007-20261002.log`; the generated
`dump_state.bin` output, README, and checksums are preserved under
`out/evidence/m20-hybrid-state-restore-patch0007-20261002/`.

The first full target-build invocation omitted `NAGI_CXX_HEADERS` and failed
on missing C++ standard headers before reaching source diagnostics. Re-running
the same fresh patch-0007 checkout through the target wrapper with Homebrew
LLVM 19 and its matching libc++ headers compiled far enough to expose the
remaining no-exception blocker: 26 object targets fail with `throw`/`try`
diagnostics across 55 distinct source paths. The complete build output is
`out/logs/m20-target-build-patch0007-cxx19-headers-20261002.log`. No sources
were omitted and exception-dependent errors were not changed to aborts. M20
remains `PARTIAL`; no target inference is claimed.
