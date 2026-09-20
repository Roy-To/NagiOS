# M10 Nagi UI / Desktop Design

## Goal

Extend the M9 user-space Window Server into a bounded UI toolkit with a font
service, text rendering, Japanese display/input coverage, widgets/layout, and
four simultaneously visible GUI application clients: Calculator, Notes, Files,
and GUI Terminal.

## Architecture

M10 remains entirely in user space. The kernel keeps only the M9 low-level
display/input capability boundary and does not gain widget, text, window, or
application syscalls. A feature-selected `nagi-init` desktop module owns a
`Desktop` compositor, a fixed array of application clients, focus routing,
damage redraw, and the existing Surface VMO presentation path.

The current developer-preview process model has one bounded bootstrap user
process and no general process-spawn ABI. Therefore the four M10 clients are
separate user-space application objects with independent state and input
handlers inside the Window Server. They are not claimed to be separate kernel
processes. This is the smallest honest implementation of simultaneous GUI
apps until the later process/service milestones provide general spawning.

## Toolkit and font boundary

`ui.rs` provides `Rect`, `Color`, clipping, a bounded `Painter`, window frames,
labels, buttons, and a deterministic layout helper. All surface writes use the
validated M9 fixed surface dimensions and avoid host graphics APIs.

`font.rs` provides a no-std bitmap Font Service. It renders a fixed set of
ASCII letters/digits/punctuation plus the Japanese glyphs used by the desktop
labels and input acceptance (`日本語`, `メモ`, and `あ`). Unsupported UTF-8 is
drawn as a visible replacement box and never causes an out-of-bounds read.
The Japanese path is real user-space UTF-8 decoding and glyph lookup: a
keyboard event routed to Notes appends the deterministic kana `あ` to its
bounded text buffer and redraws it.

## Applications and input flow

The desktop lays out four windows in a 2x2 grid. Each client renders its own
title and content:

- Calculator: `1 + 2 = 3` and a calculation button;
- Notes: `メモ` and the bounded Japanese text buffer;
- Files: the real persistent guest filename `nagi-persistent.txt`;
- GUI Terminal: a bounded `$ nagi ps` preview label.

Relative mouse events update the pointer only. A left-button event performs
hit-testing, changes focus, and calls the focused client's handler. Keyboard
events are delivered only to the focused client. Each client is marked once
when it has received focus; the final acceptance requires all four focus
transitions and the Japanese Notes input transition.

## Acceptance boundary

`nagi desktop` builds the `m10-desktop` user image, starts the real QEMU
reference machine with VirtIO VGA, VirtIO keyboard, and VirtIO mouse, and uses
QMP solely to send virtual-device events. The guest prints READY, a nonzero
surface checksum, four app-focus markers, Japanese input PASS, and the final
M10 PASS marker. The host validates only those guest-produced markers and
ordered state changes; it does not render or synthesize application results.

M9 `nagi gui` and M8 `nagi shell` remain supported regression paths.
