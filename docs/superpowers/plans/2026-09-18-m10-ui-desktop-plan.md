# M10 Nagi UI / Desktop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a real user-space Nagi desktop with a bounded toolkit, bitmap font service, Japanese text path, and four concurrently visible/focusable GUI apps.

**Architecture:** Keep the M9 kernel ABI and Surface VMO unchanged. Add focused no-std user modules for font, toolkit, and desktop application clients; extend the host QEMU GUI transport only to deliver deterministic virtual-device events and validate guest markers.

**Tech Stack:** Rust `no_std` Nagi user image, existing `libnagi` display/input ABI, QEMU VirtIO VGA/keyboard/mouse, QMP test transport, PowerShell and Git Bash acceptance scripts.

## Global Constraints

- Nagi is an independent OS; host rendering and host input APIs are not guest functionality.
- Kernel high-level window/widget/application policy is prohibited.
- Input and display operations remain capability-checked.
- The user image remains within the kernel's bounded 16-page bootstrap image limit.
- Acceptance must require guest-produced ordered markers and actual surface checksum/state changes.
- M8/M9 acceptance paths must remain working.

---

### Task 1: Font Service and Toolkit Contracts

**Files:**
- Create: `user/nagi-init/src/font.rs`
- Create: `user/nagi-init/src/ui.rs`
- Modify: `user/nagi-init/src/main.rs` only for module declarations

**Interfaces:**
- `font::draw_utf8(surface, stride, x, y, bytes, color)` renders bounded UTF-8.
- `ui::Painter::new(surface)` creates a clipped painter.
- `ui::Painter::fill`, `ui::Painter::frame`, and `ui::Painter::text` render widgets without host services.

- [ ] **Step 1: Write host-testable glyph and clipping tests** for ASCII, Japanese `あ`, replacement glyphs, and rectangles clipped to 320x200.
- [ ] **Step 2: Run `cargo test -p nagi-init --lib --locked` or the focused module test** and observe the new tests fail before implementation.
- [ ] **Step 3: Implement the fixed bitmap glyph table, UTF-8 decoder, `Rect`, `Painter`, and bounded layout helpers.** Use raw writes only after clipping has been computed.
- [ ] **Step 4: Run `cargo fmt --all -- --check` and the focused tests; require PASS with no new dependencies.**
- [ ] **Step 5: Cross-build `nagi-init` with `--features m10-desktop`; require a link result within the 16-page loader limit.**

### Task 2: Desktop and Application Clients

**Files:**
- Create: `user/nagi-init/src/desktop.rs`
- Modify: `user/nagi-init/src/main.rs`
- Modify: `user/nagi-init/Cargo.toml`

**Interfaces:**
- `desktop::run(display_capability, input_capability) -> !` starts the M10 desktop.
- `Desktop::render(surface)` draws all four app windows and returns a checksum.
- `Desktop::handle_event(event)` routes pointer/button/key events to focused clients.

- [ ] **Step 1: Add failing state tests** for four app windows, hit-testing, focus routing, Notes UTF-8 append, and checksum changes.
- [ ] **Step 2: Implement the 2x2 layout, Calculator/Notes/Files/Terminal client state, focus transitions, keyboard routing, and guest markers.** Do not print a PASS marker until the corresponding real event has been decoded and handled.
- [ ] **Step 3: Integrate `display_info`, Surface VMO drawing, `display_present`, and `input_read` in the bounded guest loop.**
- [ ] **Step 4: Build the M10 feature image and run focused kernel/user tests.**

### Task 3: Desktop CLI and QMP Acceptance

**Files:**
- Modify: `tools/nagi-cli/src/commands.rs`
- Modify: `tools/nagi-cli/src/image.rs`
- Modify: `tools/nagi-cli/tests/cli.rs`
- Create: `tests/acceptance/m10_ui_desktop.ps1`
- Create: `tests/acceptance/m10_ui_desktop.sh`

**Interfaces:**
- `nagi desktop` builds `nagi-0.1-m10-desktop.img`.
- `run_qemu_gui_with_events(config, ready_marker, events)` reuses the M9 QMP/VNC transport and accepts only guest serial markers.

- [ ] **Step 1: Add CLI parse tests for `desktop` and unexpected arguments.**
- [ ] **Step 2: Refactor the M9 GUI event transport into a reusable event-plan function without changing `nagi gui` behavior.**
- [ ] **Step 3: Add the deterministic mouse/key event plan that focuses Calculator, Notes, Files, and GUI Terminal and types into Notes.**
- [ ] **Step 4: Add both acceptance scripts asserting ordered guest markers, nonzero checksum, all four app markers, Japanese input PASS, and final M10 PASS.**

### Task 4: Verification, Regression, and Status

**Files:**
- Modify: `docs/implementation_status.md`

- [ ] **Step 1: Run `cargo test --workspace --locked`, both clippy commands, all relevant cross-builds, and `cargo fmt --all -- --check`.**
- [ ] **Step 2: Run M10 PowerShell and Git Bash acceptance scripts.**
- [ ] **Step 3: Rerun M8 and M9 acceptance scripts after M10.**
- [ ] **Step 4: Inspect logs and `git diff --check`; if every M10 criterion passes, record M10 PASS and advance Current milestone to M11.**
- [ ] **Step 5: Commit the implementation and status update with exact evidence.**
