# UI Design System Workstream Design

## Problem and intended outcome

M10 already has a user-space painter, bitmap font path, and Japanese text
coverage, but app presentation still uses local color values and local input
handling. Add a shared Nagi UI contract that first-party applications can use
without binding the contract to the current renderer or input service.

## Current behavior

`user/nagi-init/src/ui.rs` owns `Rect`, `Painter`, clipping, and text drawing.
`user/nagi-init/src/desktop.rs` owns M10-specific geometry, colors, and focus
handling. The M10 GUI decision keeps widgets in user space; the kernel exposes
only display and input primitives. `docs/architecture/language-architecture.md`
requires equal first-class `en-US` and `ja-JP` support and stable message keys.

## Scope and non-goals

Add a backend-neutral, allocation-free `no_std` crate with semantic tokens,
shared component state contracts, focus behavior, accessibility metadata, and
localization-ready text layout policy. Adapt the existing M10 desktop to
consume semantic theme roles as a narrow first-party integration.

Do not change compositor, kernel, display/input ABI, localization catalogs or
service, application business logic, Servo/M17, or the M10 acceptance path.
This contract does not claim a full accessibility service or renderer.

## Design

- Put public contracts in `user/nagi-ui`; use only `core` and fixed-capacity
  state so the crate remains usable by the Nagi target and host tests.
- Resolve colors by semantic role from complete light and dark palettes. Keep
  spacing, typography, icon size, corner radius, density, and motion values in
  typed code tokens. Reduced motion resolves transition duration to zero.
- Model activation, selection, focus traversal, dialog default/cancel/Escape,
  and Command Palette loading/empty/error/ready states as deterministic state
  transitions. The palette returns a selected command index; the app owns its
  execution and authorization.
- Keep localization outside the crate. Component labels use stable message
  keys; a caller supplies already resolved UTF-8 text and a text-measurement
  adapter. Layout policy supports wrapping or truncation after measuring the
  selected locale, including Japanese expansion.
- Expose accessibility role, name/description keys, state, focusability, and
  keyboard operation metadata without depending on an accessibility service.
- Reuse the existing M10 painter. Map its sample desktop colors to semantic
  roles; do not move raw Nagi input events or display capabilities into the
  shared crate.

## Compatibility and data impact

The change adds a local Rust package and additive public types. It adds no
syscalls, IPC schema, generated binding, external dependency, catalog, or
persistent data. Existing M10 event and acceptance behavior remains intact.
First-party code injects theme, resolved text, input events, and renderer
adapters at its boundary.

## Verification plan

Run `nagi-ui` unit tests for palette completeness, interaction/focus state,
disabled activation, dialog actions, text expansion and Japanese UTF-8,
reduced motion, and palette execution gating. Build the public gallery example,
run formatting and workspace checks, and run the existing M10 host/target
verification available in this environment. Keep host tests, Nagi target
builds, and QEMU acceptance results distinct.
