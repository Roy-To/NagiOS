# Nagi CI and Servo Bootstrap Normalization Design

**Status:** Approved for implementation on 2026-09-19

## Goal

Restore trustworthy CI for Nagi OS without discarding the current M17 work,
and make the CI architecture distinguish host tooling from Nagi target
artifacts while preserving the pinned Servo source boundary.

## Context and current checkpoint

The protected `main` worktree is at commit `66f0a068` (`M16_3`) with the
following pre-existing state:

- one uncommitted blank-line change in `user/nagi-servo/src/lib.rs`;
- an untracked, separately initialized Servo checkout at
  `third_party/servo/`, including local M17 adapter work;
- no destructive Git operation is authorized against either item.

The implementation branch is based on the committed tree only, so CI changes
can be developed and reviewed without altering that checkpoint. The current
milestone remains M17 Servo Bootstrap and remains `NOT STARTED` until its own
acceptance criteria pass.

## Selected architecture

### Servo source management

Nagi will use a reproducible fetch/bootstrap model rather than committing the
large Servo source tree or using a submodule. `third_party/sources.lock` is the
authoritative source of the repository URL, exact revision, source identity,
license, and Nagi patch boundary.

The repository will track `third_party/servo-patches/` as the Nagi-owned patch
boundary. The fetched Servo checkout at `third_party/servo/` is generated
developer/CI state and is not part of the parent repository's tracked source.
The lock entry will therefore refer to `third_party/servo-patches`, while the
fetched checkout remains at the existing path expected by future M17 build
adapters.

`nagi fetch` will:

1. validate the complete Servo lock metadata without requiring a checkout;
2. fetch the root Cargo registry sources with `Cargo.lock`;
3. clone/fetch the exact Servo revision into a temporary directory when the
   checkout is absent;
4. verify the resulting detached `HEAD` and write the generated `REVISION`
   marker;
5. verify the tracked patch boundary and run `cargo fetch --locked` inside the
   Servo checkout;
6. refuse to overwrite an existing wrong-revision or dirty checkout.

The command will leave a failed temporary fetch isolated from any existing
checkout. A source checkout already at the requested clean revision is reused.
The validator used by ordinary host unit tests checks lock metadata only; the
fetch command and target CI perform the checkout-level validation.

### CI job boundaries

The workflow will have three explicit jobs:

1. `ubuntu-host`: install host dependencies, check format, run clippy/build/
   test for host-compatible workspace packages while excluding the kernel and
   loader binaries, and run the POSIX M0 doctor/launcher acceptance scripts.
2. `windows-launcher`: install the pinned Rust toolchain, run the same
   host-compatible package build/test boundary, then validate `nagi.ps1`
   help, truthful missing-dependency doctor mode, launcher exit-code
   propagation, and PowerShell command failure propagation.
3. `nagi-target`: bootstrap the pinned Servo source, then build the kernel for
   `targets/x86_64-unknown-nagi.json`, `nagi-init` for
   `targets/x86_64-unknown-nagi-user.json`, and the loader for
   `x86_64-unknown-uefi`, using the pinned nightly and `-Zbuild-std` only for
   the Nagi targets that require it.

No job will delete format, lint, test, acceptance, or source-boundary checks.
No job will use `continue-on-error` or `|| true` to hide a failure. Every
PowerShell multi-command step will explicitly propagate `$LASTEXITCODE` or
use a terminating wrapper whose exit code is checked.

The existing pinned checkout action remains pinned. The Node.js 20 warning is
tracked separately from the build failure and will only be changed if a
repository-compatible pinned action revision is available during
implementation.

## Components and interfaces

- `tools/nagi-cli/src/config.rs`
  - validates lock-file Servo metadata;
  - exposes checkout validation for the fetch/build boundary;
  - tests lock validation without requiring an untracked checkout.
- `tools/nagi-cli/src/commands.rs`
  - owns the cross-platform `nagi fetch` bootstrap orchestration;
  - executes Git directly with exact arguments and checks every exit code;
  - preserves existing build/test package boundaries.
- `third_party/sources.lock`
  - records the exact Servo revision and tracked patch boundary.
- `third_party/servo-patches/README.md`
  - documents the generated checkout boundary and patch application contract.
- `.gitignore`
  - ignores only the generated Servo checkout, not the tracked lock or patch
    directory.
- `.github/workflows/ci.yml`
  - expresses host, launcher, and target responsibilities as separate jobs.
- `nagi.ps1`
  - retains explicit launcher exit propagation for Windows validation.
- `docs/decisions/0017-servo-pin-and-adapter-boundary.md`
  - describes fetch/bootstrap reality instead of claiming the source is
    committed in the parent repository.
- `docs/implementation_status.md`
  - records the CI normalization checkpoint and preserves M17 `NOT STARTED`
    until real Servo bootstrap acceptance is complete.

## Error handling and safety

Source acquisition is fail-closed. Missing lock metadata, a failed Git command,
a mismatched revision, an unexpected checkout state, or a missing patch
boundary returns a non-zero result with the concrete path/revision involved.
The implementation must never remove or reset an existing checkout to recover
from a mismatch. Temporary clone directories are created under repository-owned
output and are removed only when they are known to be the newly created failed
temporary checkout.

Host CI does not imply that a host build is a Nagi guest build. Kernel,
loader, no_std, and target-specific artifacts are validated only by the target
job with their intended target triples and linker configuration.

## Verification strategy

The implementation will use focused red/green tests for lock parsing, clean
checkout reuse, mismatch/dirty refusal, Git failure propagation, and launcher
exit propagation. It will then run:

- `cargo fmt --all -- --check`;
- host clippy/build/test with the documented package exclusions;
- the M0 POSIX doctor and launcher acceptance scripts;
- the Windows launcher checks where PowerShell is available;
- the exact Nagi kernel, Nagi-user, and UEFI target builds;
- `nagi fetch` against the pinned Servo revision in a clean disposable
  checkout;
- `git diff --check` and a final status/diff review.

The CI branch will not claim GitHub Actions Green without a fresh remote run.
If remote execution is unavailable, local evidence will be reported with that
boundary explicitly. After CI normalization is verified, work returns to the
protected M17 checkpoint without changing earlier milestone statuses.
