# Localization / i18n workstream status

**State:** `IN_PROGRESS` (implementation verified; commit and push pending)
**Branch:** `codex/ws-localization-i18n`
**Worktree:** `/Users/tozawa/.codex/worktrees/nagi-localization-i18n`
**Base:** `c1506888655123d819ec75be66891f0cd5477533`

## Scope and boundary

This workstream owns only the shared localization foundation, catalog
validation, formatting contract, caller guide, and focused tests. It does not
take ownership of M17/Servo, DF-01, Capability, App SDK, Activity, Wayback,
first-party application features, the UI Design System, Settings, IME, or
font/compositor internals. No `.dev/workstreams` registry or DF-01 state schema
was present at the inspected base revision.

## Implemented

- Added `crates/nagi-localization` with canonical locale parsing, distinct
  presentation-language/region inputs, stable `MessageId`, deterministic
  lookup/fallback, named interpolation, structured diagnostics, catalog
  metadata, formatting APIs, and pseudo-localization.
- Added equal first-class bundled `en-US` and `ja-JP` catalogs, including
  direction and font fallback script metadata.
- Added duplicate-aware JSON parsing, catalog parity checks, placeholder
  and namespace validation, namespaced catalog merging, and
  `localization-check` developer binary.
- Added the integration guide and updated the normative language architecture
  implementation boundary.

## Verification evidence

Host checks:

- `PATH=/Users/tozawa/.rustup/toolchains/nightly-2025-08-01-aarch64-apple-darwin/bin:$PATH cargo test -p nagi-localization --locked` — PASS, 26 tests.
- `PATH=/Users/tozawa/.rustup/toolchains/nightly-2025-08-01-aarch64-apple-darwin/bin:$PATH cargo clippy -p nagi-localization --all-targets --locked -- -D warnings` — PASS.
- `PATH=/Users/tozawa/.rustup/toolchains/nightly-2025-08-01-aarch64-apple-darwin/bin:$PATH cargo fmt --manifest-path crates/nagi-localization/Cargo.toml -- --check` — PASS.
- `PATH=/Users/tozawa/.rustup/toolchains/nightly-2025-08-01-aarch64-apple-darwin/bin:$PATH cargo run --locked -p nagi-localization --bin localization-check` — PASS, 2 locales and 14 messages.
- `./nagi fetch` with the pinned nightly on `PATH` — PASS; pinned repository dependencies were materialized in this isolated worktree to satisfy workspace Cargo path patches.

Target compile:

- With the repository's patched Rust source prepared at `out/rust-src`,
  `PATH=/Users/tozawa/.rustup/toolchains/nightly-2025-08-01-aarch64-apple-darwin/bin:$PATH __CARGO_TESTS_ONLY_SRC_ROOT=/Users/tozawa/.codex/worktrees/nagi-localization-i18n/out/rust-src/library cargo check -p nagi-localization --target targets/x86_64-unknown-nagi-user.json --lib --locked --offline -Zbuild-std=std,panic_abort` — PASS.
- This is a target compile only; no QEMU/runtime test was run. The crate is not
  wired into a Nagi user process because that integration belongs to another
  owner's UI/App SDK workstream.

Broader repository checks and limitations:

- `PATH=/Users/tozawa/.rustup/toolchains/nightly-2025-08-01-aarch64-apple-darwin/bin:$PATH ./nagi test` — FAIL outside this workstream: on the AArch64 host, existing `user/libnagi` x86-64 syscall assembly rejects registers such as `rax` and `rdi`. The failure includes 136 invalid-register errors; no unrelated runtime code was changed.
- `PATH=/Users/tozawa/.rustup/toolchains/nightly-2025-08-01-aarch64-apple-darwin/bin:$PATH cargo fmt --all -- --check` — FAIL because the command also formats the fetched pinned Servo checkout and reports its existing formatting differences. No third-party source was modified. The localization crate's own format check passes.
- `git diff --check` — PASS. A source scan found no host locale or process-environment API references in the localization crate.

## Remaining gates

- Review the final diff, commit the verified workstream state, and push this
  branch without force. Then record the final git result here.
