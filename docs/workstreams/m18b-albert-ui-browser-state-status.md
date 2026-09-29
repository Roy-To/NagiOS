# M18-B Albert UI / Browser State Workstream

**Status: PARTIAL**

## Fixed starting point

- Repository: `RT-NISH/NagiOS`
- Fixed M17 PASS base: `94e9a027618182b10c0ac2315e94673543f22423`
- Branch: `codex/m18b-albert-ui-browser-state`
- Verified implementation checkpoint: `5ca3a0f1ca66ae276e0ed350a3c8ccc90bd37a19`
- Current branch HEAD: `5ca3a0f1ca66ae276e0ed350a3c8ccc90bd37a19` (status update and push checkpoint follows)
- M17 CI evidence at the base: run `36355494134`, execution head `31bf815b7230f2658f654643e6d6c898d9881d77`

The dedicated worktree was created directly from the fixed base; `main` was not used. The older `codex/ws-ui-design-system` branch was inspected as non-authoritative reference only. No commits or files from it were transplanted.

## Current main-worktree state (2026-09-29)

The continuation is running in `codex/m18-main-albert-browser`. Its integrated
`./nagi m18` target image and real QEMU HTTPS Acceptance now **PASS locally on
macOS**. The guest accepts address-bar input for `example.com`, then completes
TLS chain and hostname verification, renders Servo frames, composes Albert
chrome, and presents three pages (`example.com`, `example.org`, and
`example.net`) through Nagi Surface. The final serial evidence is
`out/logs/m18-albert.log.live10-mmap512-service-boundaries-three-https-pass-20260929T104534`.
The current fresh rerun also passes at `out/logs/m18-albert.log`; the CLI
reports that all three allowlisted pages were TLS verified and presented.
The Darwin-only ELF linker adapter lets Mesa's existing `-latomic` target
probe and target build complete; Linux retains its existing Clang/LLD path.
The subsequent `./nagi m17` real-QEMU first-web-pixel regression also passes.

M18-B's `BrowserState`, per-tab Servo view wiring, chrome composition,
address-bar input/navigation, and VFS-backed bounded snapshot have now been
compiled in the target; the acceptance guest exercised primary-tab navigation.
The latest focused Albert host suite passes 50 tests. Runtime services remain incomplete: the
clipboard delegate fails closed without a provider; file-picker requests are
dismissed because Servo exposes host paths and Nagi has no capability-safe
picker; IME controls have no input-service text events; site-permission
requests are recorded and denied until a trusted prompt/broker is available;
downloads have no pinned Servo callback or Nagi destination service, and
uploads have no Nagi selection service. These hooks do not claim successful
transfers or permission grants. The first pushed CI run (`36510598517`) stopped
all three platform bootstrap jobs at Servo patch `0021`; that patch now adds
its cfg-gated atomic import instead of assuming it exists. The follow-up run
(`36512090928`) passed Servo bootstrap on Windows and Ubuntu but found three
separate issues: Mesa's Nagi `secure_getenv` fallback collided with `getenv`,
host Clippy found cfg-inactive POSIX helpers and a pinned-nightly lint mismatch,
and a Windows source-contract test depended on rustfmt whitespace. The current
working tree fixes all three. Fresh local checks pass for all 112 `nagi-cli`
unit tests and 18 CLI tests, 50 M18 Albert tests, 16 `nagi-posix` tests, the
affected Clippy targets, pinned format checks, and local M17/M18 real-QEMU
acceptance. The new Ubuntu CI run is pending the next push.

## Progress by phase

- **A — Audit and state model:** Complete. Reviewed repository instructions/specification, M17 embedder, M10 input/UI paths, M16 boundaries, existing acceptance scripts, and the earlier UI branch.
- **B — Tabs, address bar, navigation:** Deterministic typed models, per-tab navigation/history cursors, safe address normalization, history traversal, reload/stop, and typed chrome action dispatch are implemented with unit coverage. Address focus/edit/delete and IME composition actions update the model. A failed-address reload regression was found and fixed. The main M18 integration connects Nagi input events, address-bar edits, typed navigation requests, and per-tab Servo WebViews; this target path compiled and the address-bar route was exercised in QEMU.
- **C — History and bookmarks:** Bounded history/bookmark state, stable IDs, duplicate bookmark update behavior, and versioned persistence codecs are implemented. The main M18 integration backs `BrowserStorage` with a pathless Nagi POSIX snapshot service and VFS pending-file/replace commits. Its ABI is enabled only by the M18 feature; CI checks that M17 excludes it and M18 includes it. The current VFS limits each file to 1 KiB, so the combined snapshot is bounded and larger collections can return `Capacity`.
- **D — Session restore:** Session/history/bookmark codecs and safe restore behavior are implemented. Corrupt/missing records are handled, and restored URLs produce normal typed navigation requests. Permissions, clipboard, downloads, and upload selections are deliberately reset.
- **E — Permissions and transfer/clipboard state:** Typed permission, clipboard, upload, and download interfaces/state machines are implemented and covered with tests. Servo permission and clipboard hooks are connected to fail-closed guest boundaries. Real clipboard, file/object picker, download destination, upload selection, and trusted permission-prompt providers are still absent.
- **F — IME/text path:** UTF-8 selection and composition commit/cancel are implemented and reachable through typed chrome actions. Text input is bounded; a rejected over-capacity commit preserves its preedit for recovery. Servo IME controls are recognized, but no Nagi input-service text/composition events reach them.
- **G — Guest/UI verification:** The chrome renderer overlays the real Servo RGBA frame and presents it through the existing capability-checked Nagi Surface. The fresh `./nagi m18` path passed local target build and real-QEMU acceptance on 2026-09-29: address-bar navigation to `example.com` and three TLS-verified HTTPS pages (`example.com`, `example.org`, `example.net`) produced Servo frames with chrome presented on Nagi Surface. A fresh `./nagi m17` real-QEMU first-web-pixel regression also passes. CI's Servo patch-order failure and subsequent Mesa/host checks are repaired; the next Ubuntu target/QEMU run is pending.

## Verification so far

- `cargo test --manifest-path user/nagi-albert/Cargo.toml --features m18-acceptance --locked --offline`: **50 passed, 0 failed** in the current main M18 worktree.
- `cargo clippy --manifest-path user/nagi-albert/Cargo.toml --features m18-acceptance --all-targets --locked --offline -- -D warnings`: **PASS** in the main M18 worktree.
- `cargo fmt --manifest-path user/nagi-albert/Cargo.toml -- --check`: **PASS**.
- `git diff --check`: **PASS** at the implementation checkpoint.
- `sh -n tests/acceptance/m18b_albert_ui_browser_state.sh`: **PASS**.
- Reload regression: observed failing against the prior successful URL, then passed after switching reload to the displayed request URL.
- Chrome renderer regression: observed failing because the selected overflow tab and status/title were absent, then passed after rendering them.
- URL length regression: observed canonical percent-encoding expand a valid input beyond the address/persistence bound, then passed after normalization rejects oversized results.
- Earlier `./nagi m17` attempts stopped before target build/QEMU because the Darwin linker rejected Mesa's GNU ELF `-latomic` probe flags. The Darwin-only ELF linker adapter fixes that host/target mismatch without removing the probe; a fresh local M17 first-web-pixel acceptance passes.
- CI run `36510598517` failed during Servo bootstrap on Windows and Ubuntu because patch `0021` required a cfg import that no earlier patch added. The patch now adds that import. Follow-up run `36512090928` passed bootstrap but exposed the Mesa `secure_getenv` fallback collision, three host Clippy errors, and the Windows source-contract test's whitespace assertion. The current working tree repairs these, and affected local tests/Clippy/format checks pass. A new Ubuntu CI result is pending the next push.
- Historical CI run `36375030426` targeted checkpoint `d8fc25c67c3865a8fed29abf20a777861ade5d5f`, before the current integrated branch and acceptance step. The current push will launch the authoritative Ubuntu run for this combined state.
- The workflow runs `tests/acceptance/m18b_albert_ui_browser_state.sh` after M17 first-web-pixel acceptance, reusing that boot's serial log, then runs `./nagi m18` for the three-site HTTPS acceptance.

## Build attempt and environment notes

Earlier local builds stopped before Mesa compilation/QEMU because Darwin's `ld64.lld` rejected GNU ELF probe flags. The 2026-09-29 repair adds a Darwin-only ELF linker adapter for target links, leaving the Linux cross file and normal Clang/LLD path unchanged; the Mesa `-latomic` check remains enabled. Fresh M18 and M17 QEMU runs pass locally. This host's Apple Clang 21/SDK libc++ is incompatible with the target's pinned libc++ flags, so the local full build used the installed Homebrew LLVM 19 compiler and matching libc++ headers; this is a separate host-toolchain selection issue from the Darwin linker fix.

Earlier Mesa attempts temporarily modified their generated relibc checkout; that generated-cache edit was restored. The current combined M18 branch intentionally includes its separate tracked `third_party/relibc/src/nagi.rs` portability changes and reproducible Servo patch files.

## Scope and shared-file changes

- Owned implementation is under `user/nagi-albert/src/`; focused QEMU acceptance is `tests/acceptance/m18b_albert_ui_browser_state.sh`.
- Shared `user/nagi-albert/src/lib.rs` adds module wiring and composes Albert chrome over the real Servo frame immediately before the existing Surface copy/present. The M17 raw Servo-frame checksum remains computed before composition.
- Shared `user/nagi-albert/Cargo.toml` moves `libnagi` and the Servo platform adapter to Nagi-target-only dependencies so host state tests do not compile x86 guest syscall assembly.
- The combined main-worktree change intentionally includes M18-B browser state/chrome, M18-A network and firmware-clock support, shared init/kernel/BootInfo integration, Darwin target-link portability, and Ubuntu CI acceptance steps. It preserves the M17 feature path and does not extend into later milestones.

## Remaining blockers and exact next actions

1. Commit and push the compatibility/CI repairs on `codex/m18-main-albert-browser`, then inspect the new Ubuntu target/QEMU CI run; record whether the unchanged Ubuntu Clang/LLD path and M18 acceptance pass.
2. Implement real provider connections only when the corresponding Nagi capability-safe service APIs exist: clipboard, object-based File Picker/upload, download destination, IME text/composition events, and trusted site-permission prompts. Preserve fail-closed behavior until then.
3. Keep this workstream `PARTIAL` while those M18 deliverables remain outstanding, even though the basic-browser HTTPS/QEMU Acceptance now passes. Do not force-push or merge to `main`.

M18-B remains **PARTIAL** until the required runtime services are integrated. The main worktree has a target-tested input/navigation loop, bounded persistent browser snapshot, and a passing real-QEMU three-site HTTPS Acceptance. Ubuntu CI confirmation and the capability-safe service providers remain outstanding.
