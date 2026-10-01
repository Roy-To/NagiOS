# M29 Desktop Settings and Localization Slice

## Goal

Add a small, usable Settings surface to the existing M10 desktop acceptance UI
and establish the shared `en-US` / `ja-JP` localization contract used by that
surface. This is an M29 completion-sweep slice, not a new milestone or a claim
that the desktop shell is complete.

## Current behavior

The M10 desktop renders four fixed acceptance windows with English titles and
no language setting. Nagi's language architecture requires stable English
keys, shared locale resources, UTF-8, and `en-US` fallback. The desktop has no
localization library or Settings entry point today.

## Design

- Add a `no_std` `nagi-localization` library with embedded UTF-8 `en-US.lang`
  and `ja-JP.lang` catalogs, exact locale-code parsing, stable key lookup,
  selected-locale lookup, and English fallback. Unknown keys return a safe
  English message rather than exposing a key.
- Add a Settings button to the desktop top bar. It opens a compact overlay with
  English and Japanese language choices. Selecting a locale updates the
  desktop's localized Settings labels and first-party panel titles for the
  current run; selection is intentionally in-memory because a settings service
  and persistent preferences are not available yet.
- Keep region, keyboard/input language, and Albert conversation language out of
  this control. They remain separate settings concepts per the language
  architecture.
- Preserve the existing M10 input, focus, and acceptance path. A dedicated
  `nagi m29` QEMU acceptance will replay the M10 interaction, open Settings,
  select Japanese, require the guest locale marker, and capture the accepted
  Settings screen.

## Error and validation behavior

- Unsupported locale codes fail parsing; callers retain their existing locale.
- Missing Japanese entries fall back to English; missing entries in both
  catalogs show `Text unavailable.` and never display the key.
- Host tests cover locale parsing, UTF-8 Japanese, complete first-party
  translation keys, English fallback, and safe unknown-key handling.
- M10 desktop acceptance remains a regression gate. M29 target build and QEMU
  acceptance must prove that selection changes the rendered guest UI.

## Scope limits

This slice does not add settings persistence, a system settings service,
cross-process locale propagation, an accessibility tree or keyboard focus
model, full application localization, region/input/Albert controls, or an
installer/onboarding flow. M29 remains `PARTIAL` until its broader acceptance
and review requirements are met.
