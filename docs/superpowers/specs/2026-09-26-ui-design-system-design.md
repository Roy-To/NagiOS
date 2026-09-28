# UI Design System Workstream Design

## Problem and intended outcome

First-party apps need one renderer-neutral presentation and interaction
contract. The existing no-std `nagi-ui` crate provides useful tokens and
headless state models, but it does not yet make several required behaviors
explicit: complete typography and focus tokens, text scaling, contained modal
focus with restoration, select behavior, shell regions, and a sizing policy
that can reject critical-text truncation.

## Current behavior

`user/nagi-ui` owns semantic palettes, spacing/density/radius/elevation/motion
values, button/toggle/text-field/dialog/focus/command-palette models,
localization-aware text measurement, and accessibility metadata. It is
`no_std`, allocation-free, and does not render pixels or invoke applications.
M10 has a narrow adapter to the shared color roles. The target desktop preview
currently stops in the M5 ELF loader before the UI starts.

## Scope and non-goals

Extend `user/nagi-ui` with typed visual and interaction semantics for the
complete first-party UI foundation: typography roles and scalable sizing,
control/border/focus tokens, structured interaction and feedback state,
modal focus containment/restoration, select navigation, bounded localized
text sizing, semantic error states, and an application-shell slot contract.
Map added component kinds to accessibility roles and test defaults,
transitions, invalid inputs, keyboard behavior, and English/Japanese text
fixtures.

Keep the crate backend-neutral, `no_std`, allocation-free, and free of new
dependencies. Preserve existing public behavior where possible. Do not add
renderer code, app-specific workflows, localization catalogs or services,
assistive-technology delivery, compositor/kernel/runtime changes, or
production 0.2 adoption. Keep the M10 guest acceptance blocked while its
loader prerequisite prevents UI startup.

## Design

- Keep palette, type, spacing, density, corner, elevation, border, control,
  focus-ring, and motion values as typed semantic tokens. Add secondary body,
  label, monospace, and code typography roles; system/monospace family is a
  semantic choice, not a proprietary font requirement. Provide bounded text
  scale steps and preserve readable line-height for Latin and CJK fixtures.
- Keep interaction state separate from feedback state so focus/selection and
  loading/success/warning/error can be represented together without unrelated
  booleans. Existing button, toggle, and text-field models remain the common
  Button/IconButton, Checkbox/Radio/Switch, and TextField/SearchField logic.
- Add a fixed-capacity modal focus scope that only accepts its own enabled
  targets, wraps Tab/Shift+Tab traversal within the scope, and returns the
  saved opener target when closed. Keep device events outside the crate.
- Add a select model that navigates enabled option indices with arrow keys,
  commits with Enter/Space, and cancels with Escape. It reports indices only;
  the caller owns option labels, persistence, and actions.
- Add an application-shell specification with a required localized title and
  primary content plus optional navigation, sidebar, toolbar, status, overlay,
  and dialog regions. It describes composition and accessibility labels but
  does not render or launch applications.
- Measure already-resolved UTF-8 text through the injected locale/font
  adapter. Add explicit min/max logical-size constraints. `Reject` overflow
  returns an error when the adapter reports truncation, so callers can prevent
  critical information from disappearing silently; wrapping and ellipsis
  remain explicit caller choices.
- Extend accessibility role mapping for the new primitives and keep names and
  descriptions as stable message keys. Pair an invalid input's accessible
  state with its stable error-message key and reject inconsistent relations.

## Compatibility and data impact

The change is additive Rust API in the existing local package. It adds no
syscalls, IPC schemas, generated bindings, external dependency, localization
catalog, persistent data, or new runtime service. Existing M10 input and
acceptance behavior stays intact. The repository's workstream branch and
registered ownership remain authoritative; target visual/input acceptance is
recorded only when the guest reaches the UI.

## Verification plan

Run `nagi-ui` host tests for token invariants, type scaling, all interaction
and feedback states, disabled behavior, focus traversal and modal restoration,
select keyboard semantics, shell required/optional regions, English/Japanese
measurement, min/max sizing, reject-on-truncation, accessibility mappings,
and invalid input. Build the public gallery, run package Clippy and formatting,
and compile the crate for the Nagi user target. Run repository/workstream
validation where the environment supports it. Keep host and target compile
results distinct from QEMU guest UI evidence; do not change M5 ownership to
clear the guest blocker.
