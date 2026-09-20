# M0 Repository / Toolchain / CI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the M0 repository, Rust host CLI, toolchain diagnostics, command skeleton, tests, and CI required before kernel work.

**Architecture:** `tools/nagi-cli` is the Rust host orchestrator. Root `nagi` and `nagi.ps1` are thin launchers. Diagnostics are truthful host probes; future guest commands fail explicitly until their milestones implement them.

**Tech Stack:** Rust nightly pinned by date, Cargo workspace, PowerShell 5.1 launcher, GitHub Actions, PowerShell acceptance tests.

## Global Constraints

- Nagi is an independent OS and is not Linux-based.
- Host tools may build and test Nagi but must not fake guest filesystem, networking, rendering, or AI behavior.
- Kernel high-level file, socket, window, audio, package, and AI operations remain user-space services.
- Third-party sources are pinned in `third_party/sources.lock`; no large upstream source is vendored.
- The official reference target is QEMU x86-64, q35, UEFI/OVMF, 4 vCPU, 8 GB RAM.
- M0 must not claim M1 boot behavior.

## File Map

- Create `Cargo.toml`: root workspace declaration.
- Create `rust-toolchain.toml`: dated Rust nightly and required components.
- Create `nagi.toml`: reference target, tool expectations, cache/output paths.
- Create `.gitattributes`: stable text endings and executable launcher policy.
- Create `nagi`: POSIX launcher.
- Create `nagi.ps1`: Windows launcher.
- Create `tools/nagi-cli/Cargo.toml`: CLI package metadata.
- Create `tools/nagi-cli/src/main.rs`: command dispatch and process entry.
- Create `tools/nagi-cli/src/commands.rs`: command surface and outcomes.
- Create `tools/nagi-cli/src/doctor.rs`: deterministic host dependency probes.
- Create `tools/nagi-cli/src/paths.rs`: repository-root and owned-path handling.
- Create `tools/nagi-cli/tests/cli.rs`: unit/integration coverage for M0 behavior.
- Create `tests/acceptance/m0_doctor.ps1`: real Windows launcher acceptance test.
- Create `tests/acceptance/m0_doctor.sh`: real POSIX launcher acceptance test.
- Create `.github/workflows/ci.yml`: host formatting, lint, build, and tests.
- Create `.gitignore` and `.editorconfig`: reproducible local hygiene.
- Create `third_party/sources.lock`: versioned empty source-lock schema for M0.
- Track `AGENTS.md`, `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`, and `docs/implementation_status.md` in the initial repository baseline.
- Create `docs/architecture/README.md`, `docs/development/README.md`, `docs/porting/README.md`, `docs/testing/README.md`, and `docs/decisions/ADR-0001` through `ADR-0006`: documentation skeleton and fixed architecture records.
- Modify `docs/implementation_status.md`: record M0 evidence and move current milestone only after acceptance passes.

## Task 1: Repository and documentation skeleton

- [ ] Write `.gitignore`, `.editorconfig`, Cargo manifests, toolchain/config manifests, source-lock schema, documentation READMEs, and ADRs.
- [ ] Define CLI exit codes: `0` success, `2` usage error, `3` not implemented, `4` invalid configuration, and `10` required dependency failure.
- [ ] Verify `git status --short` lists only intended M0 files.
- [ ] Commit the repository foundation.

## Task 2: CLI behavior tests first

- [ ] Add tests that assert all commands are recognized, unknown commands fail with `2`, missing required probes fail with `10` after every check is reported, WARN-only reports return `0`, invalid OVMF pairs fail with `10`, and future guest commands report `3`.
- [ ] Run `cargo test -p nagi-cli` and observe the expected failure before implementation.

## Task 3: Implement the Rust CLI

- [ ] Implement the minimal command dispatcher and explicit exit codes.
- [ ] Implement truthful dependency probes for Git, Rust/Cargo, LLVM/Clang/LLD, QEMU, OVMF, CMake, Meson, Ninja, and Python.
- [ ] Implement repository-owned path resolution for `clean` without deleting outside `target/` or `out/`.
- [ ] Run focused tests and confirm they pass.

## Task 4: Launchers and repository commands

- [ ] Add `nagi.ps1` and `nagi` forwarding only to the Rust CLI, preserving arguments and child exit codes.
- [ ] Verify `nagi.ps1 --help` and `nagi.ps1 doctor` use the workspace CLI; verify `./nagi doctor` on POSIX.

## Task 5: Acceptance and CI

- [ ] Add and run `tests/acceptance/m0_doctor.ps1` on the prepared Windows host and `tests/acceptance/m0_doctor.sh` on the prepared Ubuntu host.
- [ ] Add `.github/workflows/ci.yml` that prepares Rust, LLVM/LLD, QEMU, OVMF, CMake, Meson, Ninja, and Python on Ubuntu, then runs the formal `./nagi doctor` acceptance; keep Windows launcher validation in a separate job.
- [ ] Run `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo build --workspace --locked`, `cargo test --workspace --locked`, and both M0 acceptance paths.

## Task 6: Status and handoff

- [ ] Review command output and relevant logs.
- [ ] Mark M0 `PASS` only if build, tests, and `nagi doctor` acceptance pass; otherwise record `PARTIAL` or `BLOCKED` with exact evidence.
- [ ] Leave the repository clean and resumable, then continue to M1 only after M0 is `PASS`.
