# M9 Display, Input, and First Window Design

## Goal

Add the first real Nagi display/input path: a bounded shared Surface VMO,
software user-space composition, raw VirtIO keyboard/mouse input, focus, and a
movable window observable in the QEMU reference machine.

## Architecture

The kernel owns only device primitives, the shared surface mapping, capability
validation, and bounded input event delivery. It does not create windows,
choose focus, compose application content, or interpret keyboard shortcuts.

The M9 bootstrap remains a single user-space process for now. A feature-
selected `nagi-init` Window Server module acts as the first Display Service and
compositor until the general process-spawn/service model is introduced. It
draws a bounded RGBA8888 desktop and window into the shared Surface VMO,
tracks the focused window and position, and requests a scanout after damage.

QEMU already provides `virtio-vga`, `virtio-keyboard-pci`, and
`virtio-mouse-pci`. The loader's GOP handoff supplies the initial scanout
framebuffer for the VirtIO VGA device. M9 adds Nagi-owned low-level VirtIO
input polling and a DisplayDevice abstraction over that real framebuffer;
user-space never uses host graphics or host input APIs.

## Capability boundary

The bootstrap receives three typed capabilities in its entry registers:

- the existing block capability;
- a display capability derived from the validated GOP framebuffer;
- an input capability for the discovered VirtIO input devices.

New syscalls expose only bounded primitives:

- `display_info` returns the actual scanout dimensions and the fixed Surface
  VMO mapping;
- `display_present` validates the display capability and copies the mapped
  Surface VMO to the real guest framebuffer;
- `input_read` validates the input capability and returns one raw bounded
  keyboard/mouse event.

No `window_create`, `file_open`, host path, or unrestricted device operation is
added to the kernel.

## Surface and compositor

The Surface VMO is a kernel-owned, page-aligned, bounded RGBA8888 backing
store mapped read/write only into the bootstrap address space. The initial
logical surface is 320x200 with a 1280-byte stride. The user-space compositor
draws a solid desktop, a title-barred window, and a border; it clips movement
to the logical surface and presents only after changing the damaged scene.

The Window Server keeps a focused-window ID and x/y position. Relative mouse
events update the position; a left-button event inside the window establishes
focus. Keyboard events are accepted only by the focused window. The acceptance
path requires both a real mouse move/focus transition and a real keyboard event
before printing the final guest marker.

## Acceptance boundary

`nagi gui` builds the feature-selected M9 image and boots the real QEMU
reference machine with a non-null display backend and QMP control channel.
The host sends QMP virtual-device input events only as test transport. The
guest must print the ready marker, report the actual decoded mouse and keyboard
events, show a changed window position and surface checksum, and then print
`Nagi M9 acceptance PASS`. The host accepts only those guest-produced markers
and the serial log; it does not generate a window result.

The default `nagi run`, M8 `nagi shell`, and all M0-M8 acceptance paths remain
unchanged.
