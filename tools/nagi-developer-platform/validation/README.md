# NDP-08/09 Manifest validation adapter

This host-only Stage 1 adapter invokes the existing `nagi-pkg manifest validate`
command. It does not implement the Manifest schema, build or run an inspected
application, create packages, sign, install, or grant capabilities. The canonical
SDK contract is [App Contract v1](../../../sdk/rust/docs/APP_SDK_CONTRACT.md).

## Prepare the trusted tool

From the repository root, use the repository-pinned `nightly-2025-08-01`:

```sh
cargo build --locked --manifest-path tools/nagi-pkg/Cargo.toml \
  --target-dir out/ndp-host/target --jobs 2
```

Preparation can fetch only the dependencies pinned by that tool's lockfile;
it is a separate developer action and never runs implicitly in Stage 1.
The adapter only uses `out/ndp-host/target/debug/nagi-pkg` in its own checkout.
There is no arbitrary executable, shell, Cargo command or command-template option.
The checkout, adapter and prepared binary are trusted developer tools; inspected
Manifest contents are data. A missing tool yields `unavailable` without fetching.

## Run

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-developer-platform/validation/validate.py \
  --root tests/developer-platform/validation minimal-third-party.json
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/nagi-developer-platform/validation -p 'test_*.py' -v
```

The input is one relative regular file below the explicitly selected directory.
No-follow directory/file descriptors reject traversal, symlinks and FIFO input;
reads are capped at the canonical loader's 1 MiB bound. The selected bytes are
copied to a private temporary `manifest.json` and passed to the real validator.
Original inputs are never rewritten. Scratch output is removed after the run.
This Unix descriptor/process-group pilot is not a native Windows runner.

Stdout is an internal JSON result. Stable reasons distinguish rejected metadata,
invalid tool output, missing tools, unreadable input, timeout, cancellation,
output overflow and I/O failure. Actual return codes are retained. Successful
output must match the actual tool's documented CLI shape; a zero exit alone is
insufficient. Raw stdout/stderr, input text, original paths and environment are
not copied into result logs. Diagnostic capture is bounded to 64 KiB total;
overflow stops the process group. The default timeout is 10 seconds (maximum
60), covering inherited output pipes even when the direct child exits early.

`validate(..., cancel=threading.Event())` supports cancellation before and during
a run; Ctrl-C is also handled by the CLI. Cleanup kills the created process
group and reaps the direct child. No cancellation triggers a later stage.

CLI exit values: 0 passed, 1 failed/overflow/I/O error, 2 unavailable,
3 not run, 124 timeout, 130 cancelled. `--stage 2` and `--stage 3` explicitly
return `not_run` and read/run nothing. Host build/test orchestration and Nagi
image acceptance remain to be connected under their respective scopes.

## Evidence and remaining scope — 2026-11-17

On macOS arm64, Clang 21, Python 3.9.6, pinned nightly Rust 1.90.0-nightly:
11 adapter/transport tests pass, including actual validator acceptance/rejection
and CLI invocation; the canonical `nagi-pkg` suite passes all 7 tests. Process
transport tests use controlled Python children only for orchestration cases.
The synthetic third-party Manifest has no requested capabilities, intents,
resources or background services. Its declared artifact is not launched or
checked for existence by metadata validation. Incompatible SDK version, unsafe
entry and unknown secret-bearing field are rejected without leaking values.

The Mac's Rust install/cache is isolated under ignored `.nagi-host-tools/`;
its shell configuration and global PATH were not changed. For this checkout's
isolated installation, prefix the build/test recipe with:

```sh
export CARGO_HOME="$PWD/.nagi-host-tools/cargo"
export RUSTUP_HOME="$PWD/.nagi-host-tools/rustup"
export PATH="$CARGO_HOME/bin:$PATH"
```

Rustup bootstrap 1.28.2 SHA-256:
`20ef5516c31b1ac2290084199ba77dbbcaa1406c45c1d978ca68558ef5964ef5`.
The toolchain pin remains `rust-toolchain.toml`; lockfiles and SDK/Package source
are unchanged. Python 3.10+ normal host readiness, SDK delivery-owner agreement,
Stage 2/3, generator, package/runtime integration and target acceptance are not
proven by this result.
