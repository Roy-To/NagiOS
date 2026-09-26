# Localization / i18n workstream status

**State:** `PASS`
**Branch:** `codex/ws-localization-i18n`
**Worktree:** `/Users/tozawa/.codex/worktrees/nagi-localization-i18n`
**Base:** `c1506888655123d819ec75be66891f0cd5477533`
**Implementation commit:** `efa7706baed554f65d2ec5c9adde61305479d607`
**Push:** successful to `origin/codex/ws-localization-i18n` (no force push)

## Scope and boundary

This workstream owns only the shared localization foundation, catalog
validation, formatting contract, caller guide, and focused tests. It does not
take ownership of M17/Servo, DF-01, Capability, App SDK, Activity, Wayback,
first-party application features, the UI Design System, Settings, IME, or
font/compositor internals. The current registry on `codex/integration-next-phase`
assigns this workstream its crate, architecture/guide, status, and durable-state
paths; the older source branch itself predates the registry, so it does not own
the registry or DF-01 schema.

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

- `PATH=/Users/tozawa/.rustup/toolchains/nightly-2025-08-01-aarch64-apple-darwin/bin:$PATH cargo test -p nagi-localization --locked` — PASS, 28 tests, including Gregorian century/leap-day, midnight/noon, `i64::MIN`, invalid public number options, and percent overflow boundaries.
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

Latest source-branch CI:

- GitHub Actions run `36215320154` for `67de0be987ddb1308d2360444e036c7a88dfab7d`
  passed the Ubuntu host suite, Windows host suite, and target builds through
  UEFI. It failed only in the independent M17 QEMU first-web-pixel acceptance.
  That target failure produced no localization failure and did not validate or
  invalidate runtime localization integration; M17 remains separately blocked.

## Remaining gates

No owner-scope implementation or verification gate remains. Repository-wide
host tests and formatting limitations are recorded above; neither failure is
in localization-owned code. Runtime QEMU validation remains for a future
integration that consumes this crate through the UI/App SDK boundary.
