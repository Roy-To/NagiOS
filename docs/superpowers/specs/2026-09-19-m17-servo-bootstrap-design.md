# M17 Servo Bootstrap Design

**Status:** Approved design, implementation pending

**Goal:** Run the pinned Servo revision inside the Nagi guest path and produce the first real web pixel through Nagi's software-rendered Surface VMO.

## Scope and acceptance boundary

M17 is the first independent phase after M16. It delivers Servo compilation for the Nagi target, Servo initialization, `about:blank`, a guest-owned local HTML fixture, CSS, JavaScript, and mouse/keyboard/scroll input. M17 is complete only when a real QEMU guest log proves a non-zero web-pixel checksum produced by Servo and the complete acceptance sequence. Host rendering, a second browser engine, a custom HTML renderer, or a host-to-guest screenshot bridge cannot satisfy the gate.

M18 remains separate and starts only after the M17 Acceptance Gate passes. The M18 browser chrome, tabs, navigation, history, bookmarks, downloads, permissions, and real HTTPS behavior are not part of M17.

## Architecture

The exact Servo revision in `third_party/sources.lock` remains the source of truth. The generated checkout under `third_party/servo/` is validated but is not committed into the parent repository. Nagi-owned Servo changes are kept in the tracked `third_party/servo-patches/` boundary and in a small Nagi adapter crate; the existing dirty checkout, stash, and unrelated worktrees are never reset, cleaned, replaced, or removed.

The existing `user/nagi-servo` crate remains the Nagi-owned Surface/input boundary. Its bounded RGBA frame copy targets the capability-checked 320x200 Surface VMO, its input bridge translates Nagi input events, and its event-loop signal wakes the Servo-owned loop. The Servo embedder supplies a Nagi `RenderingContext` that writes software RGBA output into that Surface and calls the existing display-present authority. No high-level browser syscall is added to the kernel.

The guest bootstrap owns the Servo object graph and the local fixture bytes. It constructs Servo and a WebView at `about:blank`, navigates only to the bundled guest-local page for the M17 fixture, pumps Servo's event loop, presents the resulting frame through Nagi, and emits serial markers only after the corresponding guest-side observation succeeds. Web content is not given filesystem, raw device, AI, or kernel authority.

## Component boundaries

- `user/nagi-servo/`: no-std Nagi Surface, input, and event-loop primitives consumed by the embedder.
- Servo patch/adapter boundary: the pinned Servo integration needed to compile and render without X11, Wayland, a host window, or a host socket.
- Nagi Servo bootstrap: Servo builder, WebView, delegate, rendering context, navigation, and bounded local fixture handling.
- `user/nagi-init/`: guest orchestration and ordered M17 markers, behind an explicit M17 feature so M0-M16 paths remain unchanged.
- Acceptance wrappers and fixtures: PowerShell and Git Bash QEMU tests that inspect guest-generated markers and pixel checksum.

## Data flow

```text
guest input service
       -> user/nagi-servo::InputBridge
       -> Servo WebView
       -> Servo software RenderingContext
       -> user/nagi-servo::NagiSurface
       -> capability-checked display_present
       -> QEMU VirtIO/VNC scanout
```

The local HTML fixture is compiled into or copied through the guest image as bounded read-only data. It is not read from the host filesystem while the guest is running. Pixel evidence is a checksum or marker calculated from the guest Surface after Servo has rendered; the host only reads the guest serial log and may drive QMP input events.

## Failure handling

The bootstrap fails closed on an invalid Surface description, an invalid frame size/stride, an unavailable Servo adapter, a wrong or dirty Servo checkout, a wrong source revision, or a missing fixture. Existing generated Servo state is validated before use and is never silently reset or replaced. A target compile failure remains a blocker for M17; it is not converted into a host-only test result.

The work keeps the existing root and nested Cargo configurations distinct. If a nested Servo Cargo configuration causes duplicate Nagi linker arguments or stale fixed-target binaries during acceptance, the verification procedure uses a disposable or temporarily isolated configuration, restores it in a finally-equivalent cleanup path, rebuilds the expected target artifact, and verifies that no backup remains.

## Test and acceptance strategy

Implementation follows red-green-refactor for each bounded contract:

1. Adapter contract tests prove Surface bounds, frame copying, event translation, wake consumption, and delegate/rendering behavior.
2. A Servo bootstrap test proves the initial URL and the ordered local fixture stages without using a host window or host network.
3. Target-specific builds compile the pinned Servo adapter and Nagi guest image for the intended Nagi target.
4. The M17 PowerShell and Git Bash wrappers boot the real QEMU reference machine and require ordered guest markers for Servo initialization, `about:blank`, local HTML, CSS, JavaScript, input, a non-zero first-web-pixel checksum, and `Nagi M17 acceptance PASS`.
5. Existing M0-M16 focused acceptance tests, workspace checks, formatting/lint checks, and target builds are rerun before M17 is marked `PASS`.

The status document records the exact commands, logs, revision, and any unavailable evidence. M17 is recorded as `PASS` only after the real guest gate; otherwise it remains `PARTIAL` or `BLOCKED` with the concrete blocker.

## Rejected alternatives

- Host Servo rendering with a copied screenshot is rejected because it violates the independent-OS and guest-rendering boundary.
- Chromium, WebKit, NetSurf, X11, and Wayland are rejected because Servo is the specified browser engine and the Nagi path must be native.
- A small replacement HTML/CSS/JavaScript renderer is rejected because it would fake the Servo acceptance rather than integrate Servo.
