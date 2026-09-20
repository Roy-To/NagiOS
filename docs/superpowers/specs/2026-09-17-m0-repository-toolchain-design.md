# M0 Repository / Toolchain / CI Design

**Status:** Approved for implementation by the explicit M0 implementation request

## Goal

Establish a reproducible Nagi monorepo foundation and a host-side Rust
orchestrator that can inspect the development environment and expose the
top-level build workflow without pretending that future guest functionality
already exists.

## Scope

This design covers M0 only: repository layout, Cargo workspace, toolchain
manifest, host diagnostics, command dispatch, test/CI skeletons, documentation
skeleton, and cache/output conventions. It does not implement a kernel,
loader, guest filesystem, or host-assisted guest substitute.

## Options considered

1. **Rust CLI with thin platform launchers (selected).** `tools/nagi-cli` owns
   command parsing, diagnostics, and host orchestration. `nagi` and
   `nagi.ps1` only locate and invoke it. This matches the specification and
   keeps the CLI testable and portable.
2. **Python primary CLI.** This would be convenient on the current host, but
   would violate the specified Rust-based host orchestrator and leave the
   Cargo workspace unexercised.
3. **PowerShell-only CLI.** This would couple the developer interface to one
   host shell and make Linux/CI behavior divergent, so it is rejected.

## Architecture

The root Cargo workspace contains `tools/nagi-cli`. The CLI uses a small
dependency-free command dispatcher so the M0 host tool remains easy to build
on a clean machine. `doctor` runs explicit, side-effect-free probes for Git,
Rust, LLVM/Clang/LLD, QEMU, OVMF, CMake, Meson, Ninja, and Python, reporting
each result and returning exit code `10` when a required tool is missing or
incompatible. Platform-specific unavailable tools are WARN results and do not
fail the command; all checks are still reported. OVMF is validated as a
readable CODE/VARS pair rather than as an executable. The
probe implementation accepts an injected process lookup/executor in tests;
production probes inspect the host only and never represent host tools as
guest services.

`build`, `test`, `fmt`, and `lint` run the corresponding repository checks.
`fetch` validates the source lock and reports that no external sources are
currently pinned. `image` and `run` fail explicitly as not yet available;
they do not create a fake image or boot a host process as Nagi. `clean` only
removes the repository-owned `target/` and `out/` paths after resolving them
from the repository root.

The root launchers forward arguments to `cargo run -p nagi-cli --` and do not
contain business logic. `nagi.toml` records the supported reference-machine
and tool expectations. `rust-toolchain.toml` pins a dated Rust nightly rather
than `latest`. `third_party/sources.lock` is intentionally empty until a
third-party source is introduced; later entries must include repository,
revision, hash, and patch information.

## Error handling

Every command prints a stable `PASS`, `WARN`, or `FAIL` line with an actionable
message. Unknown commands and missing arguments return exit code `2`.
Malformed configuration returns `4`. Missing tools are never hidden. Future
commands return exit code `3` and explain the next milestone that owns them.
The complete contract is: `0` success, `2` usage error, `3` not implemented,
`4` invalid project configuration, and `10` required host dependency failure.
A doctor run evaluates every check before returning its aggregate code.

## Testing and acceptance

Unit tests cover command classification, required-tool failure behavior, WARN
handling, OVMF pair validation, and the exact M0 command surface using
deterministic injected probes. M0 acceptance invokes the real POSIX launcher
on Ubuntu and the real PowerShell launcher on Windows. Each checks that
`doctor` reports every dependency and exits successfully on a prepared host.
Repository verification runs formatting, tests, lint, and both launcher paths;
missing external host dependencies remain a documented environment blocker
rather than a fabricated pass.
