# M9 Display Input First Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Boot a real Nagi user-space Window Server that renders a bounded
surface and moves its focused window after real QEMU VirtIO mouse input.

**Architecture:** Kernel implements only bounded VirtIO input polling, GOP
scanout, Surface VMO mapping, and typed capability checks. User-space owns the
Window Server, compositor, focus, and window geometry. QEMU QMP is test
transport only.

**Tech Stack:** Rust `no_std`, existing x86-64 kernel, UEFI GOP framebuffer,
QEMU VirtIO VGA/keyboard/mouse, QMP JSON transport, PowerShell/Git Bash
acceptance scripts.

## Global Constraints

- Nagi is an independent OS and must not use host rendering or input APIs.
- High-level window policy remains in user space; the kernel exposes no
  `window_create` or compositor syscall.
- Capability rights may attenuate but never strengthen.
- The reference machine remains QEMU x86-64, q35, UEFI, 4 vCPU, 8 GB RAM.
- The default M0-M8 boot and acceptance paths must remain passing.

---

### Task 1: Publish M9 ABI and capability entry contract

**Files:**
- Modify: `crates/nagi-abi/src/lib.rs`
- Modify: `user/libnagi/src/lib.rs`
- Modify: `kernel/src/user_process.rs`
- Modify: `user/nagi-init/src/main.rs`
- Test: existing ABI/kernel/user process tests

**Interfaces:**
- `DisplayInfo`, `InputEvent`, `SYS_DISPLAY_INFO`, `SYS_DISPLAY_PRESENT`, and
  `SYS_INPUT_READ` are stable bounded contracts.
- `_start(block_capability, display_capability, input_capability)` receives
  typed bootstrap capabilities.

- [ ] Add fixed-width `repr(C)` display/input structs and syscall constants.
- [ ] Add no-std wrappers with explicit length and capability arguments.
- [ ] Add tests for struct sizes, event decoding, syscall numbers, and entry
  register publication.

### Task 2: Add bounded shared Surface VMO mapping

**Files:**
- Modify: `kernel/src/user_process.rs`
- Modify: `kernel/src/memory.rs` only if a mapping primitive needs a focused
  extension
- Modify: `kernel/src/display.rs` (create)
- Test: `kernel/src/user_process.rs`, `kernel/src/display.rs`

**Interfaces:**
- `USER_SURFACE_BASE`, `SURFACE_WIDTH`, `SURFACE_HEIGHT`, and
  `SURFACE_BYTES` define the fixed mapped surface.
- `display_info` reports the actual mapping; user code writes only inside it.

- [ ] Add aligned Surface VMO pages and a bounded page-table region.
- [ ] Validate read/write mapping and reject overflow/cross-boundary ranges.
- [ ] Add unit tests for dimensions, mapping flags, and checksum behavior.

### Task 3: Implement DisplayDevice scanout and VirtIO InputDevice

**Files:**
- Modify: `kernel/src/display.rs`
- Create: `kernel/src/input.rs`
- Modify: `kernel/src/lib.rs` or kernel module wiring as required
- Modify: `kernel/src/syscall.rs`
- Modify: `kernel/src/main.rs`
- Test: focused kernel device/ABI tests

**Interfaces:**
- `display::initialize(&BootInfo)`, `display::present_surface(...)`, and
  `input::initialize()` are kernel-only primitives.
- `input::read_event` returns raw `InputEvent` values from actual VirtIO input
  queues.

- [ ] Detect the QEMU VirtIO VGA/input devices using PCI config space.
- [ ] Initialize bounded legacy VirtIO queues and refill input descriptors.
- [ ] Poll actual event/configure status without host fallback.
- [ ] Copy only validated Surface VMO bytes to the GOP scanout.
- [ ] Dispatch the three syscalls with closed-fail capability checks.

### Task 4: Implement user-space Window Server and compositor

**Files:**
- Create: `user/nagi-init/src/window.rs`
- Modify: `user/nagi-init/src/main.rs`
- Modify: `user/nagi-init/Cargo.toml`
- Modify: `user/libnagi/src/lib.rs`
- Test: `user/nagi-init` host-safe parser/checksum tests where possible

**Interfaces:**
- `window::run(display_capability, input_capability) -> !` consumes only the
  published M9 wrappers and the shared Surface VMO.
- The scene has one focused window with bounded x/y coordinates.

- [ ] Draw deterministic desktop/window pixels in user space.
- [ ] Track focus and update position only from decoded mouse events.
- [ ] Route keyboard events only to the focused window.
- [ ] Emit readiness, input, position/checksum, and final acceptance markers
  only after real state transitions.

### Task 5: Add QEMU GUI transport and acceptance

**Files:**
- Modify: `tools/nagi-cli/src/image.rs`
- Modify: `tools/nagi-cli/src/commands.rs`
- Modify: `tools/nagi-cli/tests/cli.rs`
- Create: `tests/acceptance/m9_display_input_window.ps1`
- Create: `tests/acceptance/m9_display_input_window.sh`

**Interfaces:**
- `nagi gui` launches the M9 image with QMP and sends only virtual-device
  input events after the guest ready marker.

- [ ] Add bounded QMP handshake and input-send-event transport.
- [ ] Preserve serial logs and terminate only after the guest marker.
- [ ] Assert complete M0-M8 regression markers plus real M9 transitions.
- [ ] Run focused tests, all builds, both acceptance scripts, and M8
  regression acceptance before updating status.

### Task 6: Record M9 PASS and advance

**Files:**
- Modify: `docs/implementation_status.md`

- [ ] Record exact build/test/acceptance evidence only after PASS.
- [ ] Commit implementation and status separately.
- [ ] Set M10 as the next incomplete milestone.
