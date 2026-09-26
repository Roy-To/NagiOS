# Nagi UI Design System Contract

Status: Nagi 0.1 shared user-space UI foundation

The reusable Rust API is in [`user/nagi-ui`](../../user/nagi-ui). It is a
backend-neutral `no_std` crate with no external dependencies. Its job is to
give first-party applications one stable vocabulary for semantic visual roles
and deterministic interaction state. It does not draw pixels, own app data,
read localization catalogs, invoke services, or perform accessibility calls.

## Tokens

Use `palette(ThemeMode::{Light,Dark})` and request `ColorRole` values such as
`Canvas`, `SurfaceRaised`, `TextPrimary`, `Accent`, `Focus`, and `Danger`.
Applications should receive `ThemeMode` from the system appearance setting.
The M10 reference desktop currently selects Light explicitly because M10 has
no system appearance service yet. Renderers map a role to their own pixel or
brush format; `Color::to_pixel` is provided for the existing M10 painter.

Use the typed typography, spacing, density, radius, elevation, icon-size, and
motion tokens instead of copying raw values into each app. Motion-sensitive
transitions call `motion_duration_ms` with the system reduced-motion setting;
the reduced path always returns zero duration.

## Component and input contracts

`ComponentKind` covers the initial common vocabulary: Button/IconButton,
TextField/SearchField, Checkbox/Radio/Switch, List/ListItem, Sidebar, Toolbar,
TabSegment, Menu/ContextMenu, Dialog/Sheet/Popover, window and Settings
frames, progress/empty/status/tooltip/divider/scroll primitives, and Command
Palette query/result/section/shortcut concepts. Sidebar, toolbar, settings
pages, and similar structures compose the shared controls and layout tokens;
they do not introduce a second state system.

Use `ButtonModel`, `ToggleModel`, `TextFieldModel`, `FocusManager`,
`DialogModel`, and `CommandPalette` for their tested transitions. The Nagi
input adapter translates device input into these high-level events. It keeps
raw `InputEvent`, capabilities, and display syscalls outside this crate.
Models report activation or command IDs to the application; they do not
execute app behavior. In particular, palette results are presentation data,
not authorization to run commands.

## Localization and text layout

Give UI text a stable English-based `MessageKey`, resolve it through the
shared localization service via `TextResolver`, then pass the resulting
`ResolvedText` to a font/layout adapter. Components never use displayed text
as an identifier. `TextConstraints` explicitly select line wrapping and
overflow behavior. The renderer/font adapter measures the actual selected
locale, applies Unicode shaping and line-break rules, and returns bounded
metrics. Japanese expansion is included in the unit fixtures. This crate does
not supply locale catalogs, fallback policy, an IME, or a font service.

## Accessibility metadata

Attach `AccessibleNode` metadata to rendered controls: semantic role, stable
name and optional description keys, state, focusability, and keyboard
operation. This makes renderer and future accessibility-service adapters
possible. It does not claim that a Nagi accessibility service exists or that
the current M10 renderer exposes this metadata to assistive technology.

## First-party integration

Add `nagi-ui` as a local path dependency. At the app boundary:

1. Obtain appearance and reduced-motion preferences from the appropriate
   system setting/service when available.
2. Resolve label keys through the shared localization interface.
3. Convert input-service events into `ButtonEvent`/`KeyCode` and let the
   component model decide state transitions.
4. Map semantic colors, density metrics, and returned actions in the current
   renderer and app.
5. Include role/state metadata alongside the rendered component.

When an underlying service is unavailable, inject a test adapter or keep the
app's existing behavior behind the same local interface. Do not add a kernel
syscall or make the shared UI crate depend on a compositor, localization
catalog, AI, Capability policy, or Servo.

## Gallery and verification

`cargo run -p nagi-ui --example component_gallery` builds and runs a small
public-API contract gallery with semantic theme values, a keyboard-activated
button, component vocabulary, and a Japanese command query. It is a host-side
API/demo harness, not a target renderer or pixel-golden test. The existing M10
QEMU acceptance remains the target-side visual/input regression path when its
dependencies and runner are available.
