# UI Design System Workstream

**Workstream:** `ui-design-system` (continues existing registration)
**Branch:** `codex/ws-ui-design-system`
**Base SHA:** `c1506888655123d819ec75be66891f0cd5477533`
**Status:** `PASS` for the independent host-side design foundation.
**Target UI attachment:** `BLOCKED` before UI startup by the M5 ELF loader.
**Design:** `docs/superpowers/specs/2026-09-26-ui-design-system-design.md`

This is the existing workstream and branch. The separate `DIAG-01` diagnostics
workstream is not part of this scope. Nagi 0.1 milestone history, including
M10's recorded status, is unchanged. The shared UI foundation can pass
independently of target rendering because its renderer/input interfaces are
host-testable and the future attachment boundary is documented.

## Acceptance evidence

| Criterion | State | Evidence |
|---|---|---|
| Typed semantic colors, spacing, control size, borders, radius, elevation, icons, focus, motion, and density | PASS | Typed token APIs and invariant tests in `user/nagi-ui/src/tokens.rs` |
| Typography roles, including body, secondary body, caption, label, heading, title, monospace, and code | PASS | `TypeRole`, generic font-family semantics, CJK-friendly line heights, and token tests |
| Scalable text and reduced motion | PASS | Bounded `TextScale`; every reduced-motion duration resolves to zero |
| Structured interaction and feedback states | PASS | `ComponentState` composes normal/hover/pressed/focused/selected/disabled with ready/loading/success/warning/error |
| Keyboard/focus behavior | PASS | Button/toggle activation, list/tab navigation, Select arrow/Enter/Space/Escape, Tab/Shift+Tab modal containment, and opener restoration tests |
| Bilingual and localization-aware layout | PASS | Resolved English and Japanese fixtures are measured independently; wrapping and bounded min/max sizing are tested |
| Critical information is not silently truncated | PASS | `TextOverflow::Reject` errors when the layout adapter reports truncation |
| Accessibility metadata and input/error relation | PASS | Role/state/keyboard metadata plus `AccessibleField` validation and stable error-key tests |
| Reusable primitives and application shell contract | PASS | Shared component vocabulary, reusable behavior models, and required title/content plus optional shell-region slots |
| Focused host suite | PASS | `cargo test -p nagi-ui --all-targets --locked --offline --config 'build.target="aarch64-apple-darwin"'`: 41 passed; gallery target compiled |
| Warning-free Clippy and formatting | PASS | Package Clippy with `--all-targets -- -D warnings`; `cargo fmt --package nagi-ui -- --check` |
| Public API gallery | PASS | Gallery exercised button, Command Palette, Select, shell slots, accessible text scale, and Japanese text |
| Nagi user-target compile | PASS | `cargo check -p nagi-ui --lib --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core --locked --offline` |
| UEFI compile | PASS | `cargo check -p nagi-ui --lib --no-default-features --target x86_64-unknown-uefi --locked --offline` |
| M10 QEMU guest UI attachment | BLOCKED | `./nagi desktop` built the target image; guest log stops at M5 `invalid-elf` before `nagi-init`/UI startup. No visual or input result is claimed. |
| Documentation and state | PASS | Architecture/workstream docs and the integration-owned registry state record the same acceptance evidence |
| Commit, push, clean checkout | PASS | Implementation commit `91465588a13e0f9a66afe9c99b75c2fb1a1c394c` is published to `origin/codex/ws-ui-design-system`; the branch checkout is clean |

## Architecture boundary

The reusable API lives in `user/nagi-ui`: a backend-neutral, allocation-free
`no_std` crate. Applications inject selected appearance/text scale, resolved
UTF-8 labels, input and rendering adapters, and app-owned actions. The crate
does not render, execute commands, access capabilities, or create services.
The M10 adapter remains a narrow consumer of semantic colors. Runtime theme
settings, localization service wiring, text shaping/font resolution, renderer
drawing, and assistive-technology delivery remain integration points.

## Verification log

- Existing branch base was confirmed from Git with
  `git merge-base codex/ws-ui-design-system origin/main`:
  `c1506888655123d819ec75be66891f0cd5477533`.
- The system PATH's Homebrew Cargo/Rust pair ignored the repository's pinned
  nightly and reported a missing Apple target. Checks were rerun with the
  installed `nightly-2025-08-01` toolchain; no toolchain or repository config
  was changed.
- The first desktop command passed an `RUSTFLAGS` override that masked the
  repository's `getrandom_backend="custom"` target cfg and failed at
  `getrandom`. The documented command was rerun without overriding Cargo's
  target flags.
- `./nagi desktop` then built the target image and ran QEMU. The serial log
  `out/logs/m10-desktop.log` reports `Nagi M5 user address space FAIL` and
  `reason: invalid-elf`; the init ELF has the existing empty `PT_TLS` header
  rejected by `kernel/src/user_elf.rs::validate_tls_segment`. The UI did not
  start. Kernel/loader changes are outside this workstream.
- Push of implementation commit `91465588a13e0f9a66afe9c99b75c2fb1a1c394c`
  started GitHub Actions run `36380321571`; its host and target jobs are
  tracked in the integration-owned state file.

The workstream PASS covers the tested host/target contracts only. The QEMU
run is retained as a target attachment failure and does not establish desktop
rendering, input handling, M10 acceptance, or any Nagi 0.1 milestone change.
