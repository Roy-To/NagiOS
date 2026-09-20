# Nagi Boot Sequence v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the supplied NAGI Boot Sequence v2 into the native M10 guest display path with a monotonic real-progress bridge, staged ring animation, lock-state transition, and real-QEMU acceptance evidence.

**Architecture:** Keep the progress state machine in the host-testable `libnagi` user-space library. Add a target-only M10 renderer that draws directly into the existing 320x200 Surface VMO with integer primitives, then report actual M5/M6/M7/display boundaries from `nagi-init`. Preserve the formal SVG as a repository asset while avoiding a new SVG runtime or kernel syscall.

**Tech Stack:** Rust 2021, `no_std` Nagi user space, existing `libnagi` display ABI, existing M10 software compositor, Cargo unit tests, PowerShell/Git Bash QEMU acceptance.

## Global Constraints

- Nagi is an independent OS; Windows/QEMU are development/reference hosts only.
- Files, windows, and boot-screen policy remain in user space; do not add a high-level kernel syscall.
- Do not use host rendering, host filesystem data, or copied host screenshots as guest behavior/evidence.
- Keep the current M13 Rust std/POSIX work out of this feature; `m13-posix` and `m13-std` must not link the M10 boot renderer.
- The official guest surface remains 320x200 RGBA8888 through the existing display capability.
- Visible progress is monotonic and capped at 99 until lock-state readiness; completion may set 100 only after readiness.
- Do not claim unavailable AI, account sync, or device-link stages as complete; use labels for actual M10 initialization boundaries.
- The formal SVG at `assets/nagi/nagi_logo_formal.svg` is a source asset; the native renderer uses bounded software primitives matching its geometry and colors.
- Existing POST/IPC/capability contracts, kernel boundaries, and current M10 desktop interaction behavior remain unchanged.

---

## File Map

- Create: `user/libnagi/src/boot.rs` — target-independent `BootPhase`, `BootMode`, `BootStage`, `BootState`, and `BootProgressBridge`.
- Modify: `user/libnagi/src/lib.rs` — export the boot state module.
- Create: `user/nagi-init/src/boot.rs` — M10-only surface renderer, animation frame pump, native logo, serial markers, and lock-state transition.
- Modify: `user/nagi-init/src/main.rs` — link the renderer only for M10 and report actual initialization stages.
- Modify: `user/nagi-init/src/font.rs` — add only the uppercase glyphs and `%` required by the phase labels and percentage.
- Create: `assets/nagi/nagi_logo_formal.svg` — exact supplied formal SVG source asset.
- Modify: `tools/nagi-cli/src/commands.rs` — require ordered boot markers in the desktop command result.
- Modify: `tools/nagi-cli/tests/cli.rs` — add the source-asset contract and command-surface regression coverage if needed by the implementation.
- Modify: `tests/acceptance/m10_ui_desktop.ps1` — assert ordered boot-to-desktop serial markers.
- Modify: `tests/acceptance/m10_ui_desktop.sh` — assert the same ordered markers in POSIX shell.
- Modify: `docs/implementation_status.md` — record the parallel boot-visual slice and exact verification evidence without changing M13 status.

## Interfaces

The state layer produces these stable interfaces for the renderer:

```rust
pub enum BootMode { Simulation, External }
pub enum BootPhase { SystemInit, CoreServices, StorageMount, GraphicsReady, SessionReady, Ready, Failed }
pub enum BootStage { Platform, CoreServices, Storage, Graphics, Session }

pub struct BootProgressBridge { /* monotonic state */ }

impl BootProgressBridge {
    pub const fn new(mode: BootMode) -> Self;
    pub fn set_mode(&mut self, mode: BootMode);
    pub fn set_progress(&mut self, progress: u8, phase: BootPhase) -> bool;
    pub fn advance(&mut self, stage: BootStage) -> bool;
    pub fn mark_lock_ready(&mut self) -> bool;
    pub fn complete(&mut self) -> bool;
    pub fn fail(&mut self) -> bool;
    pub const fn state(&self) -> BootState;
}
```

The M10 renderer consumes that bridge through:

```rust
pub struct BootScreen { /* capability, bridge, frame state */ }

impl BootScreen {
    pub fn new(display_capability: u64) -> Result<Self, BootError>;
    pub fn present_stage(&mut self, stage: BootStage) -> Result<u64, BootError>;
    pub fn finish_to_desktop(&mut self) -> Result<(u64, u64), BootError>;
    pub fn present_failure(&mut self) -> Result<u64, BootError>;
}
```

`present_stage` renders the stage's bounded frames and returns the final boot
frame checksum. `finish_to_desktop` marks the lock state ready, renders the
collapse and lock frame, logs completion, and returns `(boot_checksum,
lock_checksum)` for acceptance evidence. `BootError` is display-info,
surface, or present failure and never bypasses the existing fail-closed exit.

---

### Task 1: Add the host-testable monotonic boot state

**Files:**
- Create: `user/libnagi/src/boot.rs`
- Modify: `user/libnagi/src/lib.rs`

**Interfaces:**
- Consumes: no new dependencies; only `core`.
- Produces: the exact `BootMode`, `BootPhase`, `BootStage`, `BootState`, and
  `BootProgressBridge` API above for the native renderer and main integration.

- [ ] **Step 1: Write failing unit tests for the state contract**

Add a `#[cfg(test)] mod tests` in `user/libnagi/src/boot.rs` before the
implementation is complete:

```rust
#[test]
fn progress_never_moves_backward() {
    let mut bridge = BootProgressBridge::new(BootMode::External);
    assert!(bridge.set_progress(70, BootPhase::GraphicsReady));
    assert!(!bridge.set_progress(30, BootPhase::CoreServices));
    assert_eq!(bridge.state().progress(), 70);
    assert_eq!(bridge.state().phase(), BootPhase::GraphicsReady);
}

#[test]
fn progress_caps_at_ninety_nine_until_lock_is_ready() {
    let mut bridge = BootProgressBridge::new(BootMode::External);
    assert!(bridge.set_progress(100, BootPhase::Ready));
    assert_eq!(bridge.state().progress(), 99);
    assert!(!bridge.state().completed());
    assert!(bridge.mark_lock_ready());
    assert!(bridge.complete());
    assert_eq!(bridge.state().progress(), 100);
    assert!(bridge.state().completed());
}

#[test]
fn stage_targets_are_monotonic_and_named() {
    let mut bridge = BootProgressBridge::new(BootMode::External);
    assert!(bridge.advance(BootStage::Platform));
    assert_eq!(bridge.state().progress(), 15);
    assert_eq!(bridge.state().phase(), BootPhase::SystemInit);
    assert!(bridge.advance(BootStage::CoreServices));
    assert_eq!(bridge.state().progress(), 30);
    assert_eq!(bridge.state().phase(), BootPhase::CoreServices);
    assert!(bridge.advance(BootStage::Storage));
    assert_eq!(bridge.state().progress(), 50);
    assert_eq!(bridge.state().phase(), BootPhase::StorageMount);
}

#[test]
fn failure_is_terminal_and_does_not_complete() {
    let mut bridge = BootProgressBridge::new(BootMode::External);
    assert!(bridge.advance(BootStage::Storage));
    assert!(bridge.fail());
    assert!(bridge.state().failed());
    assert_eq!(bridge.state().phase(), BootPhase::Failed);
    assert!(!bridge.complete());
}
```

- [ ] **Step 2: Run the focused test and confirm it fails for the missing module**

Run:

```text
cargo test -p libnagi boot --locked
```

Expected: compilation failure because `user/libnagi/src/boot.rs` and
`pub mod boot;` do not yet exist.

- [ ] **Step 3: Implement the smallest complete state machine**

Add `pub mod boot;` to `user/libnagi/src/lib.rs`. Implement the enums and
state with private fields and these rules:

```rust
impl BootStage {
    pub const fn target(self) -> u8 { /* 15, 30, 50, 70, 90 */ }
    pub const fn phase(self) -> BootPhase { /* matching phase */ }
}

impl BootProgressBridge {
    pub fn set_progress(&mut self, requested: u8, phase: BootPhase) -> bool {
        if self.state.failed() || self.state.completed() { return false; }
        let next = requested.min(100).max(self.state.progress());
        let next = if !self.state.lock_ready() { next.min(99) } else { next };
        let changed = next != self.state.progress();
        if changed { self.state.progress = next; self.state.phase = phase; }
        changed
    }
    pub fn complete(&mut self) -> bool {
        if self.state.failed() || !self.state.lock_ready() { return false; }
        self.state.progress = 100;
        self.state.phase = BootPhase::Ready;
        self.state.completed = true;
        true
    }
}
```

The implementation must also preserve the first terminal failure and expose
`const` getters for all state fields used by the renderer and tests.

- [ ] **Step 4: Run the focused test and formatting check**

Run:

```text
cargo test -p libnagi boot --locked
cargo fmt --all -- --check
```

Expected: all boot state tests pass and formatting exits 0.

- [ ] **Step 5: Commit the isolated state layer**

```text
git add user/libnagi/src/boot.rs user/libnagi/src/lib.rs
git commit -m "feat: add monotonic boot progress bridge"
```

---

### Task 2: Add the native M10 renderer and formal logo asset

**Files:**
- Create: `assets/nagi/nagi_logo_formal.svg`
- Create: `user/nagi-init/src/boot.rs`
- Modify: `user/nagi-init/src/font.rs`
- Modify: `user/nagi-init/src/main.rs` only for the `mod boot;` declaration

**Interfaces:**
- Consumes: `libnagi::boot::{BootPhase, BootProgressBridge, BootStage}` and
  existing `DisplayInfo`, `display_info`, `display_present`, and `ui::rgba`.
- Produces: `BootError`, `BootScreen::new`, `present_stage`,
  `finish_to_desktop`, and `present_failure`.

- [ ] **Step 1: Add the supplied SVG without changing its geometry**

Create `assets/nagi/nagi_logo_formal.svg` with the exact contents of the
provided `<local-download>/nagi_logo_formal.svg`. Do not add a
host-side rasterization step or edit the path/text values.

- [ ] **Step 2: Implement the bounded renderer**

Implement `BootScreen` with these concrete behaviors:

```rust
const BACKGROUND: u32 = rgba(3, 8, 19);
const FOREGROUND: u32 = rgba(220, 236, 255);
const CYAN: u32 = rgba(125, 213, 255);
const LAVENDER: u32 = rgba(143, 162, 255);

pub fn present_stage(&mut self, stage: BootStage) -> Result<u64, BootError> {
    self.bridge.advance(stage);
    let frame_count = if self.bridge.state().reduced_motion() { 1 } else { 4 };
    for frame in 0..frame_count {
        self.render_boot_frame(frame, frame_count)?;
        self.present()?;
        delay_frame(frame_count);
    }
    self.log_stage(stage, self.bridge.state().progress());
    Ok(self.checksum())
}

pub fn finish_to_desktop(&mut self) -> Result<(u64, u64), BootError> {
    self.bridge.mark_lock_ready();
    self.render_boot_frame(0, 1)?;
    self.present()?;
    let boot_checksum = self.checksum();
    self.bridge.complete();
    self.render_collapse_frames()?;
    self.render_lock_frame();
    self.present()?;
    let lock_checksum = self.checksum();
    self.log_lock_transition(boot_checksum, lock_checksum);
    Ok((boot_checksum, lock_checksum))
}
```

`render_boot_frame` must clear the surface, draw three concentric rings with
stage-dependent radius/opacity, draw a progress arc and marker, draw the
formal mark proportions plus `NAGI`, draw the numeric percentage and one of
the supported phase labels, and never write outside the 320x200 surface.
`render_collapse_frames` must reduce ring radius/alpha over a fixed small
frame count. `render_lock_frame` must show a dark background, NAGI logo, and a
minimal lock-state label; it must not pretend to authenticate a user.

Add only the missing uppercase glyphs and `%` to `font.rs`, extend the lookup
bound consistently, and keep the existing Japanese glyph behavior intact.
`delay_frame` must use a bounded `core::hint::spin_loop` loop; it must not
call host APIs or add a syscall.

- [ ] **Step 3: Run focused host checks and the M10 cross-build**

Run:

```text
cargo test -p libnagi boot --locked
cargo fmt --all -- --check
cargo build -p nagi-init --features m10-desktop --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --release --locked
```

Expected: state and helper tests pass, formatting is clean, and the native
M10 image binary builds without adding a kernel dependency.

- [ ] **Step 4: Commit the renderer and asset**

```text
git add assets/nagi/nagi_logo_formal.svg user/nagi-init/src/boot.rs user/nagi-init/src/font.rs user/nagi-init/src/main.rs
git commit -m "feat: render native Nagi boot sequence"
```

---

### Task 3: Connect real M10 initialization stages

**Files:**
- Modify: `user/nagi-init/src/main.rs`
- Modify: `user/nagi-init/src/boot.rs` only if a narrowly scoped failure/logging helper is required

**Interfaces:**
- Consumes: `BootScreen` and the existing `run_m6_service_acceptance` and
  `run_m7_storage_acceptance` boundaries.
- Produces: ordered guest markers for platform, core services, storage,
  graphics, lock readiness, collapse completion, and desktop handoff.

- [ ] **Step 1: Add the M10-only boot screen construction**

In `_start`, after the initial FPU/user-bootstrap checks and before the M6/M7
acceptance work, add only under `#[cfg(feature = "m10-desktop")]`:

```rust
let mut boot_screen = match boot::BootScreen::new(display_capability) {
    Ok(screen) => screen,
    Err(_) => {
        libnagi::console_write(b"Nagi boot display FAIL\r\n");
        libnagi::exit(1);
    }
};
if boot_screen.present_stage(libnagi::boot::BootStage::Platform).is_err() {
    libnagi::console_write(b"Nagi boot platform FAIL\r\n");
    libnagi::exit(1);
}
```

Do not define or construct this value for `m13-posix`, `m13-std`, M11, or
M12-only builds.

- [ ] **Step 2: Report core-services and storage boundaries**

Immediately after the existing successful M6 service acceptance, call:

```rust
#[cfg(feature = "m10-desktop")]
if boot_screen
    .present_stage(libnagi::boot::BootStage::CoreServices)
    .is_err()
{
    libnagi::console_write(b"Nagi boot core services FAIL\r\n");
    libnagi::exit(1);
}
```

Immediately after `run_m7_storage_acceptance` returns `Some`, call the same
method with `BootStage::Storage`. Keep the existing failure path and its
serial marker unchanged when storage returns `None`.

- [ ] **Step 3: Complete the graphics/lock transition before desktop::run**

Inside the existing M10-only branch, before `desktop::run`, add:

```rust
if boot_screen
    .present_stage(libnagi::boot::BootStage::Graphics)
    .and_then(|_| boot_screen.present_stage(libnagi::boot::BootStage::Session))
    .and_then(|_| boot_screen.finish_to_desktop())
    .is_err()
{
    libnagi::console_write(b"Nagi boot transition FAIL\r\n");
    libnagi::exit(1);
}
desktop::run(display_capability, input_capability);
```

The renderer must log the final checksums before `Nagi M10 desktop READY` is
printed. The desktop implementation itself remains unchanged except for
receiving control after the boot frame sequence.

- [ ] **Step 4: Verify feature isolation and compile all affected paths**

Run:

```text
cargo check -p nagi-init --features m10-desktop --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --locked
cargo check -p nagi-init --features m13-posix --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --locked
cargo check -p nagi-init --features m13-std --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --locked
```

Expected: all three configurations compile; only M10 contains the boot
renderer symbols.

- [ ] **Step 5: Commit the integration**

```text
git add user/nagi-init/src/main.rs user/nagi-init/src/boot.rs
git commit -m "feat: connect boot progress to M10 initialization"
```

---

### Task 4: Add source and real-QEMU acceptance contracts

**Files:**
- Modify: `tools/nagi-cli/src/commands.rs`
- Modify: `tools/nagi-cli/tests/cli.rs`
- Modify: `tests/acceptance/m10_ui_desktop.ps1`
- Modify: `tests/acceptance/m10_ui_desktop.sh`

**Interfaces:**
- Consumes: the exact serial markers emitted by `BootScreen`.
- Produces: host-side ordered-marker enforcement before existing desktop
  input events and interactions.

- [ ] **Step 1: Add a source asset contract test**

Add a host test that resolves the repository root from
`env!("CARGO_MANIFEST_DIR")`, reads `assets/nagi/nagi_logo_formal.svg`, and
asserts it contains `<svg`, `nagiGrad`, and the exact NAGI path prefix
`M36 49.5C51.5 66.4`. This verifies the supplied formal asset is present
without rendering it on the host.

- [ ] **Step 2: Require the boot markers in the CLI desktop result**

Extend the existing `execute_desktop` marker list in order with:

```text
Nagi boot stage PLATFORM 15
Nagi boot stage CORE_SERVICES 30
Nagi boot stage STORAGE 50
Nagi boot stage GRAPHICS 70
Nagi boot stage SESSION 90
Nagi boot lock READY
Nagi boot collapse COMPLETE
Nagi boot frame checksum=
Nagi boot lock checksum=
```

Keep `Nagi M10 desktop READY` after those markers. If the current Rust CLI
loop only checks containment, change only this M10 marker validation to track
the prior match position and reject out-of-order markers, matching the
existing PowerShell and shell acceptance contracts.

- [ ] **Step 3: Update both acceptance scripts**

Insert the same boot markers, in the same order, between `Nagi M7 acceptance
PASS` and `Nagi M10 desktop READY` in both scripts. Preserve all existing M10
window/input markers and the QMP event timing, which remains keyed to desktop
readiness.

- [ ] **Step 4: Run focused host tests**

Run:

```text
cargo test -p nagi-cli --locked
cargo fmt --all -- --check
```

Expected: the source-asset test and all existing CLI tests pass.

- [ ] **Step 5: Commit acceptance contracts**

```text
git add tools/nagi-cli/src/commands.rs tools/nagi-cli/tests/cli.rs tests/acceptance/m10_ui_desktop.ps1 tests/acceptance/m10_ui_desktop.sh
git commit -m "test: verify native boot sequence before desktop"
```

---

### Task 5: Run milestone-scoped verification and record the parallel slice

**Files:**
- Modify: `docs/implementation_status.md`

**Interfaces:**
- Consumes: committed state, renderer, integration, and acceptance tests.
- Produces: reproducible evidence and a clean feature worktree ready for
  local integration into the user's M13 checkout.

- [ ] **Step 1: Run the repository-level focused checks**

Run:

```text
cargo fmt --all -- --check
cargo test -p libnagi --locked
cargo test -p nagi-cli --locked
cargo clippy -p libnagi --all-targets --locked -- -D warnings
cargo clippy -p nagi-cli --all-targets --locked -- -D warnings
```

Expected: every command exits 0.

- [ ] **Step 2: Run both real-QEMU M10 acceptance paths**

Run from the feature worktree:

```text
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tests\acceptance\m10_ui_desktop.ps1
C:\Program Files\Git\bin\bash.exe ./tests/acceptance/m10_ui_desktop.sh
```

Expected: both scripts pass, the serial logs contain the ordered boot
markers, and the existing calculator/notes/files/terminal interaction
markers still pass.

- [ ] **Step 3: Review guest logs and diff**

Inspect:

```text
Get-Content -LiteralPath out/logs/m10-desktop.log
git diff HEAD~4 --check
git status --short
```

Confirm that no boot marker claims an unavailable subsystem, no kernel file or
M13 path changed, and only expected feature files are modified.

- [ ] **Step 4: Record evidence without advancing M13**

Add a dated section to `docs/implementation_status.md` stating that the
parallel native boot-visual slice is `PASS` only if all focused and QEMU
commands pass. Include the commit IDs, marker names, and the explicit note
that `Current milestone: M13 - Rust std / POSIX` remains unchanged. If any
required command fails after targeted repair attempts, record the exact
failure and leave the slice `BLOCKED` instead of claiming success.

- [ ] **Step 5: Commit the verification record**

```text
git add docs/implementation_status.md
git commit -m "docs: record native boot sequence verification"
```

---

## Execution Notes

- Execute tasks in order because Task 2 depends on the state API from Task 1,
  Task 3 depends on both, and Task 4 must match the emitted markers.
- Use one fresh subagent per independent task and review its diff before the
  next task. Do not let two agents edit `main.rs`, `boot.rs`, or the same
  acceptance script concurrently.
- Keep the feature worktree at `<worktree>/nagi-boot-sequence`.
- Do not merge or push automatically. At handoff, report the feature branch,
  commits, test evidence, and the exact local merge boundary for the user's
  dirty M13 checkout.
