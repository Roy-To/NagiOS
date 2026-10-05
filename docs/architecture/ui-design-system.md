# Nagi UI Design System Contract

Status: shared user-space design and interaction foundation. The host-side
contract is testable independently of the Nagi renderer. Target visual and
input acceptance remains blocked until the M5 ELF loader starts the desktop.

The reusable Rust API is in [`user/nagi-ui`](../../user/nagi-ui). It is a
backend-neutral `no_std`, allocation-free crate with no external dependencies.
It gives first-party applications common visual roles and deterministic
interaction contracts. It does not draw pixels, own app data, read catalogs,
invoke services, or perform accessibility calls.

## Visual tokens and density

Resolve semantic colors through `palette(ThemeMode::{Light,Dark})` and use
roles such as `Canvas`, `SurfaceRaised`, `TextPrimary`, `Accent`, `Focus`, and
`Danger`. Applications receive theme and reduced-motion preferences from the
system setting/service when available. The M10 preview currently selects
Light explicitly because its appearance service is not implemented.

Typography roles cover caption, secondary body, labels, body, strong body,
heading, title, monospace, and code. `FontFamily` chooses a generic system or
monospace family; Nagi does not require a proprietary font. `TextScale` offers
bounded user scaling while keeping line height larger than glyph size for
Latin and CJK text. Spacing, control dimensions, density, radius, border
widths, elevation, icon size, focus treatment, and motion duration are typed
tokens. Reduced motion resolves every transition duration to zero.

## Components and state

`ComponentKind` provides the common vocabulary for Button/IconButton,
TextField/SearchField, Select, Checkbox/Radio/Switch, lists, menus, tabs,
toolbars, Dialog, Panel, progress, empty/error states, tooltips, and Command
Palette. Existing models supply shared behavior: `ButtonModel`, `ToggleModel`,
`TextFieldModel`, `NavigationModel`, `SelectModel`, `DialogModel`, and
`CommandPalette`. Apps provide option labels and decide what returned actions
mean. No model invokes an app action or grants authority.

`ComponentState` keeps the interaction axis (idle/normal, hover, pressed,
focused, selected, invalid, busy, disabled) separate from feedback state
(ready, loading, success, warning, error). This allows combinations such as a
focused control with a warning without accumulating unrelated boolean flags.

## Keyboard and focus

The input adapter converts device events into high-level `KeyCode` values.
`FocusManager` and `NavigationModel` define forward/reverse traversal, wrapping,
arrow navigation, Home/End, and activation behavior. `FocusScope` represents a
modal's fixed-capacity focus ring: Tab and Shift+Tab wrap among enabled modal
targets, background IDs are rejected, and closing returns the opener ID for
the adapter to restore if it is still available. Dialog Enter/Escape behavior
is deterministic; the app handles the resulting action.

Raw device input, capabilities, display syscalls, and focus-service calls stay
outside this crate.

## Application shell and surfaces

`ApplicationShellSpec` requires a stable title key and primary-content region.
It can describe navigation, sidebar, toolbar, status area, overlay, and dialog
regions, each with a stable accessible label. Its content feedback state can
represent ready, loading, success, warning, or error presentation. Layout,
surface, empty-state, error-state, tooltip, scroll, progress, and settings-row
types provide additional data-only contracts. The renderer and app choose the
actual placement, contents, and behavior.

## Localization-aware layout

Give UI text a stable English-based `MessageKey`, resolve it through the shared
localization service, then pass the actual selected-locale UTF-8 text as
`ResolvedText` to a font/layout adapter. Components never use displayed text
as identity. `TextConstraints` explicitly choose wrapping and overflow;
`TextOverflow::Reject` returns an error if the adapter reports truncation.
`TextSizeConstraints` applies minimum and maximum logical dimensions to the
measured result and reports content that exceeds the maximum instead of
silently clipping it. The adapter handles shaping and locale-appropriate line
breaks for both `en-US` and `ja-JP`. Catalogs, fallback policy, IME, and font
resolution remain separate services.

## Accessibility metadata

Attach `AccessibleNode` metadata to rendered controls: semantic role, stable
name and optional description keys, state, focusability, and keyboard
operation. `AccessibleField` associates an invalid input with a stable error
message key and rejects inconsistent or non-input relations. Shell regions
and error surfaces also carry stable message keys. This supports future
renderer and accessibility-service adapters; it does not claim that Nagi has
an assistive-technology service or that current M10 exposes metadata to one.

## First-party integration

Add `nagi-ui` as a local path dependency. At the app boundary:

1. Obtain appearance, reduced-motion, and text-scale preferences from the
   appropriate system setting/service when available.
2. Resolve label and error keys through the shared localization interface.
3. Convert input-service events into component keys and let the models decide
   state transitions.
4. Apply returned focus restoration only if the opener is still a valid
   target; map semantic tokens and shell regions in the current renderer.
5. Include role/state metadata alongside rendered components.

When a service is unavailable, inject a test adapter or preserve the app's
current behavior behind the same local interface. Do not add a kernel syscall
or make the shared UI crate depend on a compositor, localization catalog, AI,
Capability policy, or Servo.

## Verification boundary

`cargo run -p nagi-ui --example component_gallery` builds and runs the public
API gallery, including keyboard activation and a Japanese command query. It
is a host-side contract/demo harness, not target rendering evidence. Host
tests, Nagi target compilation, and QEMU guest acceptance are reported
separately. The current M10 QEMU preview stops at the existing M5 empty-PT_TLS
loader failure before UI startup; this workstream does not change kernel or
loader ownership or claim target visual/input acceptance.
