# M11 Login / Permissions / Security Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a real user-space local-account and Permission Broker path and pass the M11 malicious-app denial acceptance in QEMU.

**Architecture:** Keep all policy in `user/libnagi/src/security.rs`; use fixed-size no_std data structures and fail-closed decisions. Add a feature-selected M11 guest acceptance path to `nagi-init` and a top-level `nagi security` QEMU command. Do not add high-level kernel authority.

**Tech Stack:** Rust no_std user library, existing Cargo workspace, existing FAT12/QEMU/serial acceptance harness, PowerShell and Git Bash scripts.

## Global Constraints

- Nagi is an independent OS and must not use host filesystem, credentials, microphone, or network behavior as guest behavior.
- Capability rights may be attenuated but never strengthened; Developer Mode is not security bypass.
- High-level filesystem, audio, AI, and permission behavior remains in user space.
- Acceptance passes only on guest-produced ordered markers.

---

### Task 1: Security policy tests

**Files:**
- Create: `user/libnagi/src/security.rs`
- Modify: `user/libnagi/src/lib.rs`

- [ ] Define `Role`, `AppTrust`, `Resource`, `PermissionDecision`, `PermissionRequest`, `Session`, and fixed-size `AccountStore`/`PermissionBroker` types.
- [ ] Add host unit tests for password non-match, lock state, Owner-only Developer Mode, trusted `Ask`, and untrusted file/microphone `Deny` even in Developer Mode.
- [ ] Run `cargo test -p libnagi security --locked`; expected: tests pass.

### Task 2: Guest acceptance path

**Files:**
- Modify: `user/nagi-init/Cargo.toml`
- Modify: `user/nagi-init/src/main.rs`
- Modify: `user/libnagi/src/security.rs`

- [ ] Add `m11-security` feature and invoke `security::run()` after the existing M5-M7 bootstrap checks.
- [ ] Authenticate Owner and Standard accounts, lock/unlock a session, enable Developer Mode through Owner, and exercise the broker with real request objects.
- [ ] Print ordered markers only after each policy result matches the expected decision; print a FAIL marker and exit on any mismatch.
- [ ] Cross-build the feature with the Nagi user target.

### Task 3: CLI and acceptance transport

**Files:**
- Modify: `tools/nagi-cli/src/commands.rs`
- Modify: `tools/nagi-cli/tests/cli.rs`
- Create: `tests/acceptance/m11_permissions_security.ps1`
- Create: `tests/acceptance/m11_permissions_security.sh`

- [ ] Add `Command::Security`, `nagi security`, M11 image naming, persistent first boot, real QEMU serial execution, and ordered marker validation.
- [ ] Add parser/arity tests and acceptance scripts that run `clean`, `security`, and inspect the guest log.
- [ ] Run focused CLI tests and both acceptance scripts.

### Task 4: Milestone verification and handoff

**Files:**
- Modify: `docs/implementation_status.md`

- [ ] Run fmt, workspace tests, applicable clippy, user/kernel/loader builds, and M8-M10 regression acceptance.
- [ ] Review QEMU logs for the denied file/microphone requests and absence of unauthorized PASS markers.
- [ ] Commit implementation, then record M11 PASS and advance current milestone to M12 only after all checks pass.
