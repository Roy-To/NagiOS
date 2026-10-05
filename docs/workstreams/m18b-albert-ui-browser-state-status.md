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
The formal M18 Acceptance is **PASS locally and in public CI for the previous
implementation commit**. CI run
[`36517686132`](https://github.com/RT-NISH/NagiOS/actions/runs/36517686132),
head `4dce31514f759646b801879b5200dbb2f5a04099`, completed all three jobs
successfully. The `nagi-target` job passed Mesa Softpipe, M16 package, kernel,
Nagi init, UEFI loader, M17 QEMU first-web-pixel acceptance, M18-B chrome
acceptance, and the three-site M18 HTTPS/QEMU acceptance.

M18-B's `BrowserState`, per-tab Servo view wiring, chrome composition,
address-bar input/navigation, and VFS-backed bounded snapshot have now been
compiled in the target; the acceptance guest exercised primary-tab navigation.
The latest focused Albert host suite passes 51 tests. Runtime services remain
incomplete: the clipboard delegate fails closed without a provider; file-picker requests are
dismissed because Servo exposes host paths and Nagi has no capability-safe
picker; IME controls have no input-service text events; site-permission
requests carry the requesting document's serialized origin (including opaque
`null`). The opt-in M18 acceptance path keeps the real Servo request pending
behind a localized first-party prompt and applies Allow/Deny/Cancel only after
user input; missing input, rendering failure, or timeout denies. The general
production browser path still has no authenticated policy/IPC provider, and
the three-site scenario does not request a permission. Downloads have no
pinned Servo callback or Nagi destination service, and uploads have no Nagi
selection service. These hooks do not claim successful transfers or permission
grants. The first pushed CI run
(`36510598517`) stopped all three platform bootstrap jobs at Servo patch `0021`; that patch now adds
its cfg-gated atomic import instead of assuming it exists. The follow-up run
(`36512090928`) passed Servo bootstrap on Windows and Ubuntu but found three
separate issues: Mesa's Nagi `secure_getenv` fallback collided with `getenv`,
host Clippy found cfg-inactive POSIX helpers, a pinned-nightly lint mismatch,
and a constant thread-capacity assertion; a Windows source-contract test also
depended on rustfmt whitespace. The current working tree fixes these issues.
Fresh local checks pass for all 113 `nagi-cli`
unit tests and 18 CLI tests, 51 M18 Albert tests, 16 `nagi-posix` tests, the
affected Clippy targets, pinned format checks, and local M17/M18 real-QEMU
acceptance. The subsequent CI run `36517686132` passed Windows launcher,
Ubuntu host, and `nagi-target`; its M18 Acceptance printed `PASS M18 Albert:
three verified HTTPS pages rendered to Nagi Surface and QEMU`.
The requester-origin patch's first CI run (`36530525632`) failed during clean
source bootstrap because the second hunk in patch `0025` did not match the
pinned `webview_delegate.rs` context; no compilation or QEMU acceptance ran.
The hunk now anchors on the existing `feature()` method and applies in order
after `0024` to a fresh pinned-source fixture. Fresh corrected-patch `./nagi
m18` HTTPS/QEMU acceptance and `./nagi m17` first-web-pixel regression both
pass locally. The corrected patch is pushed as `eb22702`, and CI run
`36533931477` passed clean-source bootstrap and Ubuntu target acceptance.

## Progress by phase

- **A — Audit and state model:** Complete. Reviewed repository instructions/specification, M17 embedder, M10 input/UI paths, M16 boundaries, existing acceptance scripts, and the earlier UI branch.
- **B — Tabs, address bar, navigation:** Deterministic typed models, per-tab navigation/history cursors, safe address normalization, history traversal, reload/stop, and typed chrome action dispatch are implemented with unit coverage. Address focus/edit/delete and IME composition actions update the model. A failed-address reload regression was found and fixed. The main M18 integration connects Nagi input events, address-bar edits, typed navigation requests, and per-tab Servo WebViews; this target path compiled and the address-bar route was exercised in QEMU.
- **C — History and bookmarks:** Bounded history/bookmark state, stable IDs, duplicate bookmark update behavior, and versioned persistence codecs are implemented. The main M18 integration backs `BrowserStorage` with a pathless Nagi POSIX snapshot service and VFS pending-file/replace commits. Its ABI is enabled only by the M18 feature; CI checks that M17 excludes it and M18 includes it. The current VFS limits each file to 1 KiB, so the combined snapshot is bounded and larger collections can return `Capacity`.
- **D — Session restore:** Session/history/bookmark codecs and safe restore behavior are implemented. Corrupt/missing records are handled, and restored URLs produce normal typed navigation requests. Permissions, clipboard, downloads, and upload selections are deliberately reset.
- **E — Permissions and transfer/clipboard state:** Typed permission, clipboard, upload, and download interfaces/state machines are implemented and covered with tests. Servo permission requests carry the requesting document's serialized origin, including opaque `null`. The M18 acceptance-only Servo delegate holds requests pending, renders localized origin/feature/action copy above an opaque scrim, resolves mouse Allow/Deny and Escape Cancel from fresh input, and denies on failure or timeout. The regular browser runtime still lacks authenticated policy/IPC wiring and the M18 HTTPS scenario does not exercise a permission request. Servo clipboard hooks and address-bar Ctrl+C/X/V now use the gesture-bound `nagi-clipboard` service (ADR 0043) and pass real QEMU copy/paste acceptance on 2026-10-05; file/object picker, download destination, and upload selection providers are still absent.
- **F — IME/text path:** UTF-8 selection and composition commit/cancel are implemented and reachable through typed chrome actions. Text input is bounded; a rejected over-capacity commit preserves its preedit for recovery. Servo IME controls are recognized, but no Nagi input-service text/composition events reach them.
- **G — Guest/UI verification:** The chrome renderer overlays the real Servo RGBA frame and presents it through the existing capability-checked Nagi Surface. The fresh `./nagi m18` path passed local target build and real-QEMU acceptance on 2026-09-29: address-bar navigation to `example.com` and three TLS-verified HTTPS pages (`example.com`, `example.org`, `example.net`) produced Servo frames with chrome presented on Nagi Surface. A 2026-10-03 rerun after the permission-prompt changes also passed all three HTTPS pages. Its boot image, User Data, OVMF variables, serial log, screenshot, and SHA256 manifest are preserved under `out/evidence/m29-browser-1790978167816192000/`; the scenario did not request a permission. A fresh `./nagi m17` real-QEMU first-web-pixel regression also passes. Public CI run `36533931477` validates clean Servo patch bootstrap, the unchanged Ubuntu Clang/LLD path, target image build, M17 QEMU regression, M18-B chrome acceptance, and M18 three-site HTTPS/QEMU acceptance for corrected commit `eb22702`.

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
- CI run `36510598517` failed during Servo bootstrap on Windows and Ubuntu because patch `0021` required a cfg import that no earlier patch added. The patch now adds that import. Follow-up run `36512090928` passed bootstrap but exposed the Mesa `secure_getenv` fallback collision, three POSIX host Clippy errors, a `manual_is_multiple_of` lint, and the Windows source-contract test's whitespace assertion. The next run (`36516544370`) passed Mesa build setup and Windows checks but Clippy found a constant thread-count assertion; it now asserts the exact M17/M18 contract separately. All affected local tests, Clippy targets, and pinned format checks pass. The subsequent run `36517686132` is the successful full CI result below.
- CI run `36517686132` at `4dce31514f759646b801879b5200dbb2f5a04099` passed the initial integrated Windows launcher, Ubuntu host, and `nagi-target` gates, including Mesa's `-latomic` probe and M18 QEMU acceptance.
- Corrected commit `eb22702da8e832126c32e420c8fde579b05f8a67` passed CI run [`36533931477`](https://github.com/RT-NISH/NagiOS/actions/runs/36533931477) across Windows launcher, Ubuntu host, and `nagi-target`. Clean Servo bootstrap applied patch `0025`; the target job passed M17 first-web-pixel, M18-B chrome, and `./nagi m18` with three verified HTTPS pages presented on Nagi Surface/QEMU. This is the authoritative full-CI result for the corrected requester-origin patch.
- Historical CI run `36375030426` targeted checkpoint `d8fc25c67c3865a8fed29abf20a777861ade5d5f`, before the current integrated branch and acceptance step.
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

1. The compatibility repairs and corrected Servo requester-origin patch are committed and pushed on `codex/m18-main-albert-browser`; CI run `36533931477` confirms clean-source patch application, the unchanged Ubuntu Clang/LLD route, M17 regression, and M18 Acceptance pass.
2. Servo patch `0025` carries the requesting document's origin, including opaque `null`, into Albert. The first CI run `36530525632` exposed a context mismatch; the corrected patch now passes fresh-source bootstrap and full CI. The remaining production browser hooks have no safe provider to connect: repository audits found no capability-safe download destination, File Picker/upload handle, shared clipboard provider, IME text/composition event source, or authenticated permission policy/IPC provider. The new modal runs only in the opt-in M18 acceptance delegate. The existing M6 ServiceRegistry is in-process only; site-permission requests retain their origin and fail closed without an authenticated provider.
3. Keep this workstream `PARTIAL` while those M18 deliverables remain outstanding. The formal basic-browser HTTPS/QEMU Acceptance is `PASS`. Do not force-push or merge to `main`.

M18-B remains **PARTIAL** until the required runtime services are integrated. The main worktree has a target-tested input/navigation loop, bounded persistent browser snapshot, and passing local and Ubuntu CI real-QEMU three-site HTTPS Acceptance. Capability-safe service providers remain outstanding.

## Completion Sweep — localized site-permission prompt (2026-10-03)

Added a bounded opaque modal to the existing chrome surface with English and
Japanese localized title, requesting origin, feature name, and Allow/Deny/Cancel
actions. The M18 acceptance Servo delegate keeps the actual `PermissionRequest`
pending until a fresh mouse click or Escape event; queued events are drained
when the modal first appears. Missing input, presentation failure, timeout,
duplicate requests, and other resolution failures deny the request. Surface
frame bounds, origin width, button hit areas, and the Japanese prompt glyph set
have focused tests. `nagi-albert` with `m18-acceptance` passed 60 host tests,
and the 2026-10-03 `./nagi m18` QEMU run passed the three HTTPS pages after a
clean pinned Servo regeneration. The captured run does not generate a site
permission request, so interactive QEMU Allow/Deny/Cancel remains unverified;
production authenticated policy/IPC, clipboard, picker, IME, download, and
upload providers also remain. M18 stays `PARTIAL`.

## Clipboard provider (2026-10-05)

`user/nagi-clipboard` now backs Albert's clipboard. Reads need a one-shot
paste gesture and writes a recent activation, both per tab and recorded only
by Albert's device-input routing. The QEMU bring-up also fixed Albert's
keyboard focus: the active WebView receives Servo focus, and address
submission or a page click moves focus from the address bar to the page.
`./nagi m18` run `1791179851416415000` passed three HTTPS pages plus
QMP-driven copy/paste and an ungestured-read denial. IME, picker,
download/upload, and production permission/IPC providers remain; M18-B stays
**PARTIAL**.
