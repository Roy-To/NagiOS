# Rust standard source preparation

`prepare_nagi_rust_std_source` prepares the installed, pinned Rust sources
for Nagi's `build-std` invocations. M17, M18, M19 and the shared browser-init
build helper still invoke it before their existing Cargo builds.

The private `rust_std_source` module reuses `out/rust-src/library` only when
all of these match a completed preparation:

- The full `rustc --version --verbose` identity and canonical installed
  source location.
- SHA-256 of the complete installed source tree and the Nagi std patch.
- The preparation recipe version, including the existing libc path patch.
- SHA-256 of the complete prepared tree, including names, file types, file
  contents, permissions and empty directories. Timestamps are excluded
  from this digest and preserved on a verified reuse.

The local `out/rust-src-state.json` records the input identities and output
digest after successful preparation. It is cache bookkeeping, not a new
source of security authority. No dependency, target feature, Cargo flag,
acceptance assertion, timeout, failure exit or workflow gate is removed.
Cargo still decides whether flags, features or dependencies require a
rebuild. The cache does not substitute for guest evidence.

## Failure and interruption

An OS file lock serializes preparation in one output directory. Process
termination releases that lock; its empty lock file can remain. A fresh
copy is patched under `out/rust-src-preparing`, checked again against the
inputs, and then moved to the existing output location. The completed
state is published last.

Changed inputs, corrupt or extra output, missing output, unknown or
malformed state, and an interrupted staging tree all force fresh
preparation. A missing or failing patch returns an error even if an old
cache exists. Failed preparation cannot publish a completed state. A
subsequent call recovers from the partial staging tree. Source symlinks and
other nonregular entries are rejected; corrupt output symlinks are removed
without following their targets.

The installed toolchain and prepared directory remain local build inputs.
As with Cargo's other local caches, this mechanism does not protect against
a hostile process that can replace both cache metadata and build inputs.
Rebuild after changing the recipe by incrementing its version. Removing
`out/rust-src-state.json` also forces a fresh preparation.

## Build concurrency contract

Run one active Nagi/Cargo build command per checkout. The preparation lock
is released when this helper returns; it is not held while Cargo consumes
`out/rust-src`. A second process with changed inputs can replace that tree
while the first build is reading it. This helper therefore does not provide
concurrent-build safety, even if the consumers use different target dirs.

Use separate checkouts with separate `out` and target directories for
parallel builds. Sequential CI remains the measured and supported use here.
Extending this contract would require immutable prepared generations or a
consumer-held lock through Cargo completion; neither is implemented here.

## Timings and measured scope

The helper emits `TIMING rust-std-source` lines for toolchain discovery,
lock acquisition, input hashing, output validation, copying, patching,
verification, publication and total reuse/rebuild time. A verified hit does
not copy or patch sources. Failure messages remain errors.

A Linux cloud measurement used main `69fe92d3`, pinned
`nightly-2025-08-01`, the real Nagi patch, and the same small no-std library
with `-Zbuild-std=core,alloc --release` and the Nagi user target. Installed
sources contained 2,201 files (37,587,015 bytes).

| Preparation | Preparation time | Subsequent Cargo time | Cargo result |
| --- | ---: | ---: | --- |
| Old helper, first call | 0.150 s | 13.924 s | compiled |
| Old helper, repeated call | 0.112 s | 14.034 s | compiled again |
| New helper, fresh preparation | 0.247 s | 12.688 s | compiled |
| New helper, verified repeat | 0.121 s | 0.045 s | Fresh |
| New helper, second verified repeat | 0.115 s | 0.045 s | Fresh |
| Final source, verified repeat with `--locked` | 0.128 s | 0.049 s | Fresh |

The old repeated call changed source mtimes despite identical contents.
Cargo's verbose output reported `compiler_builtins` and `core` dirty due to
changed source timestamps; `alloc` was rebuilt through the dependency.
Verified reuse retained those timestamps, and Cargo reported all four
packages Fresh. Input/output verification adds work to a fresh preparation;
the measured benefit is avoiding an unnecessary subsequent compile.

These are local measurements for this small build, not a prediction for
Servo or the full CI suite. Clang is unavailable in this executor, so full
M17/M18/M19 native and guest comparison was not run locally. All existing
guest gates remain required. The unit tests cover deterministic content,
toolchain, patch, corruption and interruption invalidation, timestamp
preservation, lock release and recovery after patch failure.

Validation on the same baseline: all 293 unit and 32 CLI tests passed before
the change; all 303 unit and the same 32 CLI tests passed after it. The ten
new unit regressions include a Unix literal-backslash filename that must
not impersonate a missing nested source. Host Clippy with warnings denied
and formatting checks pass. The prepared source tree is byte-, name-,
type- and mode-identical to the old helper output (2,732 entries).
