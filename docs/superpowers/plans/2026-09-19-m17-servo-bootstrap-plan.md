# M17 Servo Bootstrap Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the first real Servo web surface inside Nagi user space and pass the M17 `First Web Pixel on Nagi` acceptance gate, or record an evidence-backed `BLOCKED`/`PARTIAL` result when the pinned guest rendering dependency cannot be built reproducibly.

**Architecture:** Keep `user/nagi-servo` as a small `no_std` boundary for capability-checked Surface VMO copies, input translation, and event-loop wake signaling. Put Servo and its `std`/relibc-facing embedder in a separate user-space adapter. Servo must render through a Nagi-owned target adapter and pinned guest Mesa Softpipe/GL path, then read pixels into the Nagi Surface and present them through the existing display service. Local M17 content is guest-bundled and loaded through a `data:` URL or a bounded Nagi protocol handler; it never uses the host filesystem or host rendering.

**Tech Stack:** Rust nightly pinned by `rust-toolchain.toml`, the exact Servo revision in `third_party/sources.lock`, Nagi x86-64 user target, Servo `ServoBuilder`/`WebViewBuilder`/`RenderingContext`/`WebViewDelegate` APIs, Mesa Softpipe or an equivalent pinned guest software GL implementation, QEMU/OVMF/q35, and PowerShell/Git Bash acceptance wrappers.

## Global Constraints

- Work only in `<m17-worktree>`; do not modify the main worktree or its protected nested Servo checkout.
- Keep the current M0-M16 behavior and acceptance contracts unchanged except for the smallest feature-gated M17 integration.
- Implement in specification order and do not mark M17 `PASS` without a real guest-generated web pixel and serial evidence from QEMU.
- Do not use X11, Wayland, Chromium, WebKit, NetSurf, a host browser, host screenshots, a host filesystem, host sockets, or a no-op/fake GL implementation.
- Keep all Nagi-specific Servo changes as numbered tracked patches under `third_party/servo-patches/`; generated checkout contents remain generated and disposable.
- Keep the exact Servo revision, repository, source hash, license, vendored path, and patch directory validated against `third_party/sources.lock`.
- Preserve capability checks, Surface VMO bounds, input authority, and the user-space/kernel boundary. Servo never receives arbitrary kernel authority.
- Use failing focused tests before implementation changes. Tests may mock orchestration-only seams, but rendering acceptance must exercise the real guest path.
- A failed host `cargo test --workspace --locked` caused by the missing MSVC `link.exe` is an environment baseline, not M17 evidence; use package-level tests, target builds, and QEMU evidence where available and report the boundary explicitly.

## Task 1: Make pinned Servo fetch and patch application reproducible

**Files:**

- Modify: `tools/nagi-cli/src/servo.rs`
- Modify: `tools/nagi-cli/src/commands.rs`
- Modify: `tools/nagi-cli/tests/cli.rs` or the existing Servo CLI test module
- Modify: `third_party/servo-patches/README.md`
- Create: `third_party/servo-patches/0001-nagi-target-rendering-boundary.patch` when the first Servo source change is required
- Update: `third_party/sources.lock` only when a reviewed exact source or patch fingerprint changes

**Interfaces and invariants:**

- `ensure_servo_checkout(root)` fetches the exact revision into a unique temporary directory, applies sorted numbered patches, validates the resulting tree, and atomically installs it only when the destination is absent or is a previously generated matching checkout.
- `validate_servo_checkout` accepts only the exact pinned revision plus the exact Nagi patch result. It refuses unrelated modified, staged, untracked, wrong-revision, or missing-marker state.
- Patch application is deterministic: lexical patch order, `git apply --check` before `git apply`, explicit failure text naming the patch, and a recorded patch fingerprint derived from the tracked patch files.
- An existing protected checkout or a non-generated dirty checkout is never deleted, overwritten, or silently repaired. The fetch command reports the exact path and remediation.

- [ ] Add failing tests for sorted patch discovery, malformed patch rejection, patch-application failure, patch fingerprint mismatch, wrong Servo revision refusal, unrelated dirty-file refusal, and safe reuse of an already patched generated checkout.
- [ ] Add a test proving that a nonempty destination directory cannot be replaced merely because fetch is requested; the test must preserve the destination and assert the diagnostic.
- [ ] Implement patch discovery, fingerprinting, checked application, generated-checkout marker validation, and atomic installation without recursive deletion of an existing user path.
- [ ] Make the current fetch collision diagnostic actionable and verify that a clean generated checkout can be reused without re-fetching.
- [ ] Run the focused `nagi-cli` Servo tests, `cargo test -p nagi-cli`, `cargo fmt --all -- --check` where the workspace metadata permits it, and `git diff --check`.

**Commands and evidence:**

```powershell
cargo test -p nagi-cli servo
cargo test -p nagi-cli --locked
.\nagi.ps1 fetch
git -C third_party/servo rev-parse HEAD
git -C third_party/servo status --short
```

Expected evidence is the exact pinned revision, an explicit patch fingerprint/marker, no unrelated checkout changes, and a second fetch that either safely reuses the generated checkout or refuses without mutation.

## Task 2: Establish the real Nagi software-rendering dependency boundary

**Files:**

- Inspect and modify through tracked patches: `third_party/servo-patches/*.patch`
- Modify: `third_party/sources.lock` when adding exact Mesa/Softpipe source metadata
- Create: the Nagi-owned source-lock entry and fetch/bootstrap metadata for the selected pinned Mesa/Softpipe revision, if required by the build
- Modify: `targets/x86_64-unknown-nagi-user.json`
- Modify: `user/nagi-pal/` only for the documented Nagi user-space graphics/runtime ABI required by the adapter
- Modify: Servo paint/Surfman/WebRender source only through numbered patch files

**Required technical result:**

- Servo's `RenderingContext` contract is implemented by a real Nagi-native context. The implementation supplies valid GL/GLES operations required by WebRender, creates and resizes the guest framebuffer, reads it back as RGBA, and exposes the result to `NagiSurface`.
- The software path is guest-side and reproducible. Mesa Softpipe or an equivalent pinned implementation is built for Nagi; it is not a host OpenGL context or a screenshot bridge.
- The adapter explicitly selects the Nagi target backend and does not enable Surfman's X11/Wayland features for Nagi.
- If the current Servo/WebRender/Surfman API cannot be adapted with the pinned sources and available Nagi ABI, stop this task after the documented repair attempts and record the exact build blocker instead of weakening the rendering contract.

- [ ] Write a failing adapter contract test that rejects a context with missing GL/readback/present behavior and proves the required 320x200 RGBA surface shape.
- [ ] Add the smallest Nagi target/backend patch needed to compile Servo's paint path without X11 or Wayland.
- [ ] Add the pinned Mesa/Softpipe source and reproducible build metadata if it is required; include exact revision, source hash, license, vendored path, and Nagi patch path in the source lock.
- [ ] Implement framebuffer creation, resize, readback, and present through the real guest software path, then copy into the existing bounded `NagiSurface`.
- [ ] Verify that no target feature or dependency resolves to host `surfman` X11/Wayland behavior for `target_os = "nagi"`.
- [ ] Build the adapter for the Nagi target and run the focused contract/readback tests. Treat a no-op GL implementation, synthetic HTML pixels, or host-only success as failure.

**Commands and evidence:**

```powershell
cargo test -p nagi-servo-adapter
cargo build -p nagi-servo-adapter --target targets/x86_64-unknown-nagi-user.json
rg -n "sm-x11|sm-wayland|x11|wayland|target_os = \"nagi\"" third_party/servo-patches third_party/sources.lock user/nagi-pal targets
git diff -- third_party/servo-patches third_party/sources.lock
```

The required evidence is a successful target build and a traceable guest software-rendering dependency. If no reproducible Mesa/Softpipe source can be built for Nagi after targeted fixes, record `M17 BLOCKED` with the exact command, error, attempted fixes, and next experiment.

## Task 3: Implement the tracked Servo embedder and frame/input bridge

**Files:**

- Create or patch-track: `apps/albert/servo-nagi/Cargo.toml`, `apps/albert/servo-nagi/src/lib.rs`, and related adapter modules
- Keep or update the generated-checkout bootstrap seam: `third_party/servo/nagi-adapter/` through the patch/bootstrap mechanism, not as an undocumented vendored edit
- Modify: `user/nagi-servo/src/lib.rs` only for narrowly required stable boundary APIs; preserve its existing validation and tests
- Modify: root `Cargo.toml` and app manifests to register the adapter without adding Servo to the `no_std` crate
- Create tests: adapter unit/contract tests for wake, frame delivery, navigation, and input conversion

**Interfaces:**

- `ServoBuilder::default().event_loop_waker(Box<dyn EventLoopWaker>).build()` creates the Servo instance.
- `WebViewBuilder::new(&servo, Rc<dyn RenderingContext>).delegate(delegate).url(Url::parse(...)?).build()` creates the first view at `about:blank` and then loads the guest-bundled local page.
- `WebViewDelegate::notify_new_frame_ready` calls `WebView::paint`, reads the real frame from the context, copies it through `NagiSurface`, and calls the display present boundary.
- `EventLoopWaker` maps Servo wakeups to the existing atomic `EventLoopSignal` without busy-looping.
- `BrowserInput` is converted to Servo `InputEvent` values with bounded mouse, button, key, and scroll handling. Unrepresentable input is rejected and logged as a deterministic diagnostic.
- File, clipboard, network, dialog, and permission callbacks remain denied or bounded for M17; no callback may access a host path or arbitrary OS authority.

- [ ] Add failing tests for bootstrap state, about:blank creation, local URL load, frame callback ordering, input conversion, and denied unsupported callbacks.
- [ ] Implement the adapter against the exact pinned Servo APIs and make the first frame path use the real `RenderingContext` readback.
- [ ] Add a guest-bundled HTML fixture containing visible text, CSS-dependent layout/color, JavaScript-generated content, and a deterministic marker element. Keep the fixture independent of host files and network.
- [ ] Verify that the adapter remains `std`-enabled and separate from `user/nagi-servo`'s `no_std` crate.
- [ ] Run adapter tests and the target build, then inspect the generated Servo diff to ensure every Nagi source change is represented by a patch.

## Task 4: Integrate M17 into the Nagi init guest without regressing M0-M16

**Files:**

- Create: `user/nagi-init/src/m17_servo.rs`
- Modify: `user/nagi-init/Cargo.toml`
- Modify: `user/nagi-init/src/main.rs`
- Create: `tests/fixtures/m17/index.html` and any generated/bundled resource file required by the guest image
- Modify: `tools/nagi-cli/src/commands.rs` to add the M17 build/image/run/acceptance entry point
- Create: `tools/nagi-cli/src/m17.rs` or equivalent focused orchestration module if the existing command module would otherwise become coupled

**Guest behavior:**

- M17 is feature-gated and selected before the current `m13_std::run` fast path only when the M17 acceptance feature is enabled. Existing M13-M16 feature combinations retain their current entry behavior.
- The guest prints ordered English markers for Servo initialization, `about:blank`, local HTML load, CSS application, JavaScript execution, input delivery, first nonzero web-pixel checksum, and successful present.
- The fixture is compiled/bundled into the guest image. It is not read from the host filesystem at runtime, and its JavaScript cannot call Nagi privileged services.
- The first nonzero pixel checksum must be computed from Servo's actual readback buffer after the Surface capability and display present path, not from a hardcoded expected value.
- Pointer, keyboard, and scroll events are injected through the Nagi input path. They must produce observable fixture state changes or deterministic event markers.

- [ ] Write failing guest orchestration tests for marker ordering, checksum nonzero validation, feature selection, and preservation of the current M16 package branch.
- [ ] Implement the M17 module and feature wiring, keeping the current `_start` ABI and capability arguments intact.
- [ ] Bundle the fixture through the existing image/build mechanism and add deterministic checksum/event diagnostics.
- [ ] Build the Nagi user image with the intended restricted-`std`/relibc configuration and inspect the linker/map output for accidental host dependencies.
- [ ] Run the existing focused M16 package test and M17 guest build after integration.

## Task 5: Add real QEMU M17 acceptance and input injection

**Files:**

- Create: `tests/acceptance/m17_servo_bootstrap.ps1`
- Create: `tests/acceptance/m17_servo_bootstrap.sh`
- Create or modify: `tools/nagi-cli/src/commands.rs` for `nagi m17` and/or `nagi acceptance m17`
- Create: `out/logs/m17-servo-bootstrap.log` only as a generated ignored artifact

**Acceptance sequence:**

1. `nagi doctor --allow-missing` and source-lock validation pass.
2. The exact Servo checkout and patch fingerprint are validated.
3. The Nagi image is built for x86-64 user mode and booted under QEMU q35/UEFI with the reference 4-vCPU/8-GB, VirtIO block/network/GPU/sound/RNG device shape.
4. Guest serial output reports Servo initialization and `about:blank`.
5. The bundled local page reports HTML, CSS, and JavaScript markers.
6. Real Nagi input events reach the WebView and produce the fixture's pointer/keyboard/scroll markers.
7. The frame callback reads a nonzero RGBA buffer, presents it through the Nagi Surface, and reports `First Web Pixel on Nagi` with checksum and dimensions.
8. The log ends with `Nagi M17 acceptance PASS`; any missing marker, host access, fake pixel, timeout, or unexpected Servo dirty state fails the wrapper.

- [ ] Write the acceptance wrappers and command parser tests first, including marker-order and failure-diagnostic tests.
- [ ] Implement serial-log capture, timeout handling, QEMU process cleanup, and optional QMP input injection using only the guest input/device route.
- [ ] Make the wrappers work from both PowerShell and Git Bash with paths containing spaces and with a clean temporary artifact directory.
- [ ] Run the acceptance against a fresh generated Servo checkout and inspect the full serial log, QEMU exit status, image checksum, and source status.
- [ ] Repeat once from the reusable generated checkout to prove reproducibility without mutating the protected main checkout.

**Commands:**

```powershell
.\nagi.ps1 doctor --allow-missing
.\nagi.ps1 fetch
.\nagi.ps1 m17
.\tests\acceptance\m17_servo_bootstrap.ps1
```

```bash
./nagi fetch
./nagi m17
./tests/acceptance/m17_servo_bootstrap.sh
```

## Task 6: Verification, status, and milestone handoff

**Files:**

- Modify: `docs/implementation_status.md`
- Create or modify: `docs/decisions/0019-m17-servo-rendering-status.md`
- Modify: `docs/architecture/` documentation only where the implemented adapter changes an existing boundary
- Update: `docs/superpowers/plans/2026-09-19-m17-servo-bootstrap-plan.md` checkboxes as work completes

- [ ] Run focused package tests for `nagi-cli`, `nagi-servo-adapter`, the Servo adapter, and `nagi-init` orchestration.
- [ ] Run `cargo fmt --all -- --check` or document the unrelated workspace metadata limitation without changing the preserved pre-existing blank-line state.
- [ ] Run `cargo clippy` for the affected packages/target where the toolchain can link them.
- [ ] Run the M17 target build and both acceptance wrappers when the real backend is available.
- [ ] Run the relevant M0-M16 regression gates, especially M16 package/SDK acceptance, after M17 feature wiring.
- [ ] Review `git diff`, `git diff --check`, generated checkout status, source-lock metadata, serial logs, and QEMU artifacts.
- [ ] Update status to `PASS` only with the full first-web-pixel evidence. Use `PARTIAL` for a working subset without the acceptance gate and `BLOCKED` for a persistent dependency/build blocker, including exact command, error, hypothesis, attempted fixes, and next experiment.
- [ ] Commit the resumable M17 changes locally with focused commit messages. Do not push, merge into main, or remove worktrees without an explicit user instruction.

**Final evidence record:**

- pinned Servo revision and patch fingerprint;
- target build command and result;
- focused test commands and result;
- QEMU configuration and acceptance log path;
- ordered marker list including nonzero checksum and `First Web Pixel on Nagi`;
- source/worktree cleanliness and any pre-existing environment limitation;
- status transition and the next milestone only if M17 is genuinely `PASS`.
