# M18-B Albert UI / Browser State Workstream

**Status: PARTIAL**

## Fixed starting point

- Repository: `RT-NISH/NagiOS`
- Fixed M17 PASS base: `94e9a027618182b10c0ac2315e94673543f22423`
- Branch: `codex/m18b-albert-ui-browser-state`
- Verified implementation checkpoint: `76bbe60dfb712186ea1b970e139be08be6f84e17`
- Branch HEAD at that verification: `76bbe60dfb712186ea1b970e139be08be6f84e17` (the status/acceptance checkpoint is being recorded separately)
- M17 CI evidence at the base: run `36355494134`, execution head `31bf815b7230f2658f654643e6d6c898d9881d77`

The dedicated worktree was created directly from the fixed base; `main` was not used. The older `codex/ws-ui-design-system` branch was inspected as non-authoritative reference only. No commits or files from it were transplanted.

## Progress by phase

- **A — Audit and state model:** Complete. Reviewed repository instructions/specification, M17 embedder, M10 input/UI paths, M16 boundaries, existing acceptance scripts, and the earlier UI branch.
- **B — Tabs, address bar, navigation:** Deterministic typed models, per-tab navigation/history cursors, safe address normalization, history traversal, reload/stop, and UI action dispatch are implemented with unit coverage. A failed-address reload regression was found and fixed. Guest input events are not connected to Albert yet.
- **C — History and bookmarks:** Bounded history/bookmark state, stable IDs, duplicate bookmark update behavior, and versioned persistence codecs are implemented. Durable guest storage is not wired: `BrowserStorage` is an atomic batch interface with a memory implementation used by tests.
- **D — Session restore:** Session/history/bookmark codecs and safe restore behavior are implemented. Corrupt/missing records are handled, and restored URLs produce normal typed navigation requests. Permissions, clipboard, downloads, and upload selections are deliberately reset.
- **E — Permissions and transfer/clipboard state:** Typed permission, clipboard, upload, and download interfaces/state machines are implemented and covered with tests. A connected Nagi File Picker, File Service, Permission Broker, and clipboard adapter still need service integration.
- **F — IME/text path:** UTF-8 selection and composition commit/cancel are implemented. Text input is bounded; a rejected over-capacity commit preserves its preedit for recovery. The input service/Servo event path is not yet connected.
- **G — Guest/UI verification:** The chrome renderer overlays the real Servo RGBA frame and presents it through the existing capability-checked Nagi Surface. It renders active tabs (including an active tab beyond the first three), address text, status indicator, and page title. The added M18-B QEMU acceptance script requires the chrome post-present marker. Local target build attempts stop during Mesa Meson configuration on the macOS ELF-linker mismatch; QEMU was not reached and no M18-B QEMU result is recorded yet.

## Verification so far

- `cargo test --manifest-path user/nagi-albert/Cargo.toml --locked`: **39 passed, 0 failed**.
- `cargo clippy --manifest-path user/nagi-albert/Cargo.toml --all-targets --locked -- -D warnings`: **PASS**.
- `cargo fmt --manifest-path user/nagi-albert/Cargo.toml -- --check`: **PASS**.
- `git diff --check`: **PASS**.
- Reload regression: observed failing against the prior successful URL, then passed after switching reload to the displayed request URL.
- Chrome renderer regression: observed failing because the selected overflow tab and status/title were absent, then passed after rendering them.
- URL length regression: observed canonical percent-encoding expand a valid input beyond the address/persistence bound, then passed after normalization rejects oversized results.
- `./nagi m17`: **FAIL before target build/QEMU** on this macOS host. Mesa Meson line 1271 reports `C shared or static library 'atomic' not found`; the prior 64-bit atomic compile probe succeeds, while its link probe invokes Darwin `ld64.lld` with GNU ELF flags and fails on `--entry=0` and `--unresolved-symbols=ignore-all`. Python Mako/PyYAML checks pass in `out/mesa-venv`. Retrying Homebrew LLVM 19 still selected the Darwin linker. This host limitation is confirmed by inspection; the repository Ubuntu target job is required for target/QEMU evidence.
- Public CI is pending the branch push. The M18-B QEMU script has syntax/command dependencies checked but has not run because the local target build cannot pass the host linker probe.

## Build attempt and environment notes

Local `./nagi m17` attempts now pass the Mesa Python generator checks using the ignored `out/mesa-venv` installed from `tools/mesa/requirements.txt`. The target build then stops before Mesa compilation/QEMU: the macOS Darwin linker selected for the Nagi ELF cross-link probes rejects `-Wl,--entry=0` and `-Wl,--unresolved-symbols=ignore-all` (`ld64.lld: error: unknown argument`). That makes Mesa's 64-bit atomic link probe report failure and incorrectly advances to `find_library('atomic')`, which cannot exist for this Nagi target. A retry with Homebrew LLVM 19 still selected the Darwin linker. This is a host toolchain mismatch; the repository's Ubuntu target job uses clang-19/lld-19 and is the appropriate next verifier. QEMU was not reached locally.

`tools/mesa/build.sh` applies the tracked relibc portability patch while generating headers. That exact third-party cache change has been restored to the fixed-base version after each completed attempt and is not part of the M18-B diff.

## Scope and shared-file changes

- Owned implementation is under `user/nagi-albert/src/`; focused QEMU acceptance is `tests/acceptance/m18b_albert_ui_browser_state.sh`.
- Shared `user/nagi-albert/src/lib.rs` adds module wiring and composes Albert chrome over the real Servo frame immediately before the existing Surface copy/present. The M17 raw Servo-frame checksum remains computed before composition.
- Shared `user/nagi-albert/Cargo.toml` moves `libnagi` and the Servo platform adapter to Nagi-target-only dependencies so host state tests do not compile x86 guest syscall assembly.
- `docs/implementation_status.md`, Cargo lockfiles, M17 runner sources, M18-A paths, third-party patches, and CI workflows have not been intentionally changed.

## Remaining blockers and exact next actions

1. Commit the status/acceptance checkpoint and push both commits on `codex/m18b-albert-ui-browser-state`.
2. Inspect the push-triggered Ubuntu target/QEMU CI run. If target and M17 QEMU succeed, run `tests/acceptance/m18b_albert_ui_browser_state.sh` there or add it to a follow-up in-scope CI step only if the current run does not execute it. Record target/QEMU serial evidence.
3. Continue the in-scope guest input/action loop and durable storage/service adapters after the CI result; keep the typed state and UI independent of M18-A transport internals.
4. Update this status file with the final commit/CI/QEMU evidence and exact remaining integration work. Never force-push or merge to `main`.

M18-B remains **PARTIAL** until the guest has a connected user-input/action loop and durable storage/service adapters, and until target/QEMU evidence confirms the presented chrome. M18 as a whole still requires M18-A integration and real HTTPS-site acceptance.
