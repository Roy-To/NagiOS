# M18-B Albert UI / Browser State Workstream

**Status: PARTIAL**

## Fixed starting point

- Repository: `RT-NISH/NagiOS`
- Fixed M17 PASS base: `94e9a027618182b10c0ac2315e94673543f22423`
- Branch: `codex/m18b-albert-ui-browser-state`
- Verified implementation checkpoint: `ee49b812fa69c943c34ca076fe795e6ba92e504f`
- Branch HEAD before this status-only evidence checkpoint: `ee49b812fa69c943c34ca076fe795e6ba92e504f`
- M17 CI evidence at the base: run `36355494134`, execution head `31bf815b7230f2658f654643e6d6c898d9881d77`

The dedicated worktree was created directly from the fixed base; `main` was not used. The older `codex/ws-ui-design-system` branch was inspected as non-authoritative reference only. No commits or files from it were transplanted.

## Progress by phase

- **A — Audit and state model:** Complete. Reviewed repository instructions/specification, M17 embedder, M10 input/UI paths, M16 boundaries, existing acceptance scripts, and the earlier UI branch.
- **B — Tabs, address bar, navigation:** Deterministic typed models, per-tab navigation/history cursors, safe address normalization, history traversal, reload/stop, and typed chrome action dispatch are implemented with unit coverage. Address focus/edit/delete and IME composition actions update the model. A failed-address reload regression was found and fixed. Guest input events and navigation requests are not yet connected to the actual Albert/Servo event loop.
- **C — History and bookmarks:** Bounded history/bookmark state, stable IDs, duplicate bookmark update behavior, and versioned persistence codecs are implemented. Durable guest storage is not wired: `BrowserStorage` is an atomic batch interface with a memory implementation used by tests.
- **D — Session restore:** Session/history/bookmark codecs and safe restore behavior are implemented. Corrupt/missing records are handled, and restored URLs produce normal typed navigation requests. Permissions, clipboard, downloads, and upload selections are deliberately reset.
- **E — Permissions and transfer/clipboard state:** Typed permission, clipboard, upload, and download interfaces/state machines are implemented and covered with tests. A connected Nagi File Picker, File Service, Permission Broker, and clipboard adapter still need service integration.
- **F — IME/text path:** UTF-8 selection and composition commit/cancel are implemented and reachable through typed chrome actions. Text input is bounded; a rejected over-capacity commit preserves its preedit for recovery. The input service/Servo event path is not yet connected.
- **G — Guest/UI verification:** The chrome renderer overlays the real Servo RGBA frame and presents it through the existing capability-checked Nagi Surface. It renders active tabs (including an active tab beyond the first three), address text, status indicator, and page title. GitHub Actions run `36375495228` successfully built the Nagi target and UEFI loader, passed M17 first-web-pixel QEMU acceptance, and then passed M18-B chrome acceptance by finding `Nagi M18B Albert chrome presented` in the same boot's serial log. This proves the chrome was composed and presented; it does not prove physical keyboard/IME actions are connected or browser state survives a guest restart. Local QEMU remains unavailable because the macOS Mesa cross-link probe selects an incompatible Darwin linker.

## Verification so far

- `cargo test --manifest-path user/nagi-albert/Cargo.toml --locked`: **40 passed, 0 failed**.
- `cargo clippy --manifest-path user/nagi-albert/Cargo.toml --all-targets --locked -- -D warnings`: **PASS**.
- `cargo fmt --manifest-path user/nagi-albert/Cargo.toml -- --check`: **PASS**.
- `git diff --check`: **PASS** at the implementation checkpoint.
- `sh -n tests/acceptance/m18b_albert_ui_browser_state.sh`: **PASS**.
- GitHub Actions run [`36375495228`](https://github.com/RT-NISH/NagiOS/actions/runs/36375495228), head `ee49b812fa69c943c34ca076fe795e6ba92e504f`: **PASS**. Ubuntu host format/lint/build/test and M0 acceptance passed; Windows launcher build/tests passed; Ubuntu target/Mesa/Servo/kernel/init/UEFI builds passed; M17 first-web-pixel QEMU acceptance passed; M18-B QEMU chrome marker acceptance passed.
- Reload regression: observed failing against the prior successful URL, then passed after switching reload to the displayed request URL.
- Chrome renderer regression: observed failing because the selected overflow tab and status/title were absent, then passed after rendering them.
- URL length regression: observed canonical percent-encoding expand a valid input beyond the address/persistence bound, then passed after normalization rejects oversized results.
- `./nagi m17`: **FAIL before target build/QEMU** on this macOS host. Mesa Meson line 1271 reports `C shared or static library 'atomic' not found`; the prior 64-bit atomic compile probe succeeds, while its link probe invokes Darwin `ld64.lld` with GNU ELF flags and fails on `--entry=0` and `--unresolved-symbols=ignore-all`. Python Mako/PyYAML checks pass in `out/mesa-venv`. Retrying Homebrew LLVM 19 still selected the Darwin linker. This host limitation is confirmed by inspection; the repository Ubuntu target job is required for target/QEMU evidence.
- Earlier CI run `36375030426` was canceled when the newer checkpoint was pushed; it is superseded by successful run `36375495228`.
- The workflow runs `tests/acceptance/m18b_albert_ui_browser_state.sh` after M17 first-web-pixel acceptance, reusing that boot's serial log. This is a narrow shared CI change so M18-B checks the chrome marker alongside the existing authoritative QEMU run without booting the image twice.

## Acceptance criteria result

- **Browser state: PASS** — tab/navigation/address state and typed action behavior have focused unit coverage.
- **Persistence: PARTIAL** — versioned codecs and safe restore behavior are tested, but the guest has no durable `BrowserStorage` adapter, so history/bookmarks/session data are not proven to survive restart.
- **User interaction: PARTIAL** — permission-aware clipboard/transfer state and Japanese IME composition boundaries are modeled and tested, but the M17 Albert launch does not pass its input capability into the browser event loop, and real service adapters are not connected.
- **Verification: PASS for available checks** — focused tests, lint/format, target build, and QEMU chrome-presentation acceptance pass. QEMU currently checks presentation only, not live keyboard or restart persistence.
- **Overall M18-B: PARTIAL** — the persistence and live interaction gaps above block PASS.

## Build attempt and environment notes

Local `./nagi m17` attempts now pass the Mesa Python generator checks using the ignored `out/mesa-venv` installed from `tools/mesa/requirements.txt`. The target build then stops before Mesa compilation/QEMU: the macOS Darwin linker selected for the Nagi ELF cross-link probes rejects `-Wl,--entry=0` and `-Wl,--unresolved-symbols=ignore-all` (`ld64.lld: error: unknown argument`). That makes Mesa's 64-bit atomic link probe report failure and incorrectly advances to `find_library('atomic')`, which cannot exist for this Nagi target. A retry with Homebrew LLVM 19 still selected the Darwin linker. This is a host toolchain mismatch; the repository's Ubuntu target job uses clang-19/lld-19 and is the appropriate next verifier. QEMU was not reached locally.

`tools/mesa/build.sh` applies the tracked relibc portability patch while generating headers. That exact third-party cache change has been restored to the fixed-base version after each completed attempt and is not part of the M18-B diff.

## Scope and shared-file changes

- Owned implementation is under `user/nagi-albert/src/`; focused QEMU acceptance is `tests/acceptance/m18b_albert_ui_browser_state.sh`.
- Shared `user/nagi-albert/src/lib.rs` adds module wiring and composes Albert chrome over the real Servo frame immediately before the existing Surface copy/present. The M17 raw Servo-frame checksum remains computed before composition.
- Shared `user/nagi-albert/Cargo.toml` moves `libnagi` and the Servo platform adapter to Nagi-target-only dependencies so host state tests do not compile x86 guest syscall assembly.
- `docs/implementation_status.md`, Cargo lockfiles, M17 runner sources, M18-A paths, and third-party patches have not been intentionally changed. `.github/workflows/ci.yml` adds only the M18-B serial-log acceptance step described above.

## Remaining blockers and exact next actions

1. Connect Nagi input capability delivery to the Albert event loop so address editing, tab actions, navigation, keyboard/IME composition, and focus changes drive the typed chrome actions in the guest. The current init launch passes the display capability only, so the Albert UI has no guest input stream.
2. Add a durable `BrowserStorage` adapter backed by the Nagi File Service, with capability checks, atomic batch semantics, and guest restart/restore acceptance. The current production boundary has no wired durable adapter; the in-memory implementation is for tests.
3. Connect the typed permission, clipboard, upload, and download interfaces to the existing Nagi service adapters when those capabilities are available; preserve deny-by-default behavior and reset transient grants on restore.
4. Extend guest/QEMU acceptance to exercise real input and session persistence after those in-scope service boundaries are available. Current CI proves target build and chrome presentation only.
5. Update this file after each verified integration checkpoint. Keep changes on `codex/m18b-albert-ui-browser-state`; never force-push or merge to `main`.

M18-B remains **PARTIAL**: browser-state unit tests and target/QEMU chrome presentation pass, but guest input/action delivery and durable state persistence remain unimplemented, and the permission/clipboard/file adapters are not connected to guest services. M18 as a whole still requires M18-A integration and real HTTPS-site acceptance.
