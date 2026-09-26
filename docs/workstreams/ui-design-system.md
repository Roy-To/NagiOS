# UI Design System Workstream

**Workstream:** `ui-design-system`
**Branch:** `codex/ws-ui-design-system`
**Status:** `BLOCKED` — implementation and focused checks pass; M10 QEMU
integration stops in the existing M5 ELF loader before the UI starts.
**Baseline:** M10 remains `PASS` per `docs/implementation_status.md`; this
workstream extends the shared UI foundation and does not revise M10's result.
**Design:** `docs/superpowers/specs/2026-09-26-ui-design-system-design.md`

## Acceptance evidence

| Criterion | State | Evidence |
|---|---|---|
| Semantic tokens and light/dark themes | PASS | `nagi-ui` palette roles and completeness tests |
| Typography, spacing, radius, density, elevation, and motion tokens | PASS | Typed token APIs and unit tests |
| Reusable component and deterministic state contracts | PASS | 31 component kinds; button, toggle, text field, layout, progress, and status contracts |
| Keyboard focus and activation behavior | PASS | Focus/navigation traversal, Enter/Space, dialog default/cancel tests |
| Accessibility metadata hooks | PASS | Role, stable name/description keys, state, focus, and keyboard metadata tests |
| Localization-ready sizing and Japanese fixture | PASS | Injected resolver/layout adapter and Japanese expansion fixture |
| Reduced-motion behavior | PASS | Reduced preference resolves all motion durations to zero |
| Command Palette primitives | PASS | Query/result/section/shortcut, loading/empty/error/ready, selection and execute-gating tests |
| First-party integration guidance and M10 adapter | PASS | Architecture guide and M10 semantic palette adapter compile in target image build |
| Focused host verification | PASS | `cargo test -p nagi-ui --locked`: 27 passed; Clippy and formatting passed |
| Nagi target and M10 guest verification | BLOCKED | `nagi-ui` target check passed; QEMU guest stopped at M5 `invalid-elf` before UI startup |
| Commit and push | PASS | Implementation commit `69de514` and this evidence update are published on `origin/codex/ws-ui-design-system` |

## Repository boundaries

The source implementation branch predates DF-01 and was created without a
`.dev/workstreams.json` registry or `docs/0.2/DEVELOPMENT_ARCHITECTURE.md`.
The Integration Owner has registered the workstream in the root registry and
records its host and target evidence in
`.dev/workstreams/ui-design-system/state.json`. The shared crate does not own
localization resources, compositor or kernel behavior, M17/Servo, or app
business logic.

## Verification log

- Preflight: source `main` was clean at `c1506888655123d819ec75be66891f0cd5477533`.
- Created isolated worktree `/Users/tozawa/.codex/worktrees/nagi-ui-design-system`
  on `codex/ws-ui-design-system`.
- The ordinary `./nagi fetch` initially used the Intel Homebrew Rust 1.86
  compiler and failed on the existing `unsigned_is_multiple_of` API use. The
  repository's pinned Rustup nightly, explicit `RUSTC`, and the required
  feature-gate `RUSTFLAGS` successfully fetched the pinned dependencies.
- `cargo test -p nagi-ui --locked`: 27 passed; doc tests passed (0 tests).
- `cargo clippy -p nagi-ui --all-targets --locked -- -D warnings`: passed.
- `cargo fmt --package nagi-ui --package nagi-init -- --check`: passed.
- `cargo run --locked -p nagi-ui --example component_gallery`: passed; public
  API gallery activated a focused button, selected a palette command, and
  printed the Japanese query `設定を開く`.
- `cargo check -p nagi-ui --target targets/x86_64-unknown-nagi-user.json
  -Zbuild-std=core --locked`: passed for the Nagi x86-64 target.
- `./nagi desktop` built the Nagi target image, then timed out after 45 seconds
  because the guest stopped before desktop startup. `out/logs/m10-first-boot.log`
  reports M5 `invalid-elf`. `llvm-readelf -l
  target/x86_64-unknown-nagi-user/release/nagi-init` shows an empty `PT_TLS`
  header (zero virtual address, file size, memory size, and alignment);
  `kernel/src/user_elf.rs::validate_tls_segment` rejects `memory_size == 0`.
  This kernel/loader boundary belongs to another workstream and was not changed.
- `cargo test --workspace --exclude nagi-kernel --locked` was attempted on this
  Apple Silicon host and failed because host compilation of `libnagi` uses
  x86-64 inline-assembly registers (`rax`, `rdi`, `rsi`, `rcx`, `r11`) that are
  unavailable on the aarch64 host. This is not claimed as a passing test.
- `./nagi --help` initially could not resolve the generated `third_party/cc-nagi`
  source before dependency fetch; the supported bootstrap fetch later completed.
