# Accessibility

## Goal

We want Nagi OS and its project materials to become usable by people with
different abilities, input methods, languages, and assistive technologies.
Accessibility barriers are valid bugs or improvement proposals and should be
described without requiring someone to disclose a diagnosis or other personal
information.

These are community goals. They do not create a new confirmed product contract,
accessibility certification, or release gate. Product decisions remain governed
by [docs/product-decisions/OPERATING_RULES.md][operating-rules].

## Current scope and documented evidence

This statement was checked against the repository documentation on 2026-11-15.

- The current Nagi OS 0.1 Developer Preview reference target is QEMU x86-64
  with UEFI/OVMF, q35, four virtual CPUs, and 8 GiB of RAM. Physical-PC support
  is not a 0.1 completion requirement. This describes the documented
  development target, not an accessibility certification.
- The project documents ordinary GUI, keyboard, and pointer interaction as
  important. The [M29 workstream][m29] remains `PARTIAL` and records QEMU
  acceptance for a narrow, fixed Desktop and Settings surface: Tab focus,
  Enter/Space activation,
  Settings locale controls with Tab/arrow/Enter/Space/Escape input, a visible
  focus ring, a localized selection cue, and mouse input.
- This is not complete keyboard-only support. The M29 workstream says the rest
  of the Desktop lacks keyboard focus support and that there is no system-wide
  focus service or accessibility tree. Assistive-technology acceptance remains
  unverified.
- English and Japanese are documented as equal first-class user languages. The
  README records a Japanese text path and a QEMU setting-selection/restart
  check, while complete translation and accessibility acceptance remain open.
- [DEC-042][dec-042] and [DEC-043][dec-043] confirm the product direction of a
  Linux kernel with a Nagi-owned userland. This direction does not mean the
  current 0.1 implementation has migrated or establish accessibility behavior
  for a future Linux-based target. The exact UI backend and target acceptance
  remain subject to their owning specifications and decisions.

The repository has not published a complete accessibility audit. The
following areas are unverified unless an owning acceptance document says
otherwise: screen-reader names, roles, and announcements; keyboard-only
navigation and focus order across the full interface; text scaling and contrast
modes; reduced motion; captions or text alternatives for audio; and
accessibility behavior on physical hardware or a future Linux-based target.
Please treat unverified behavior as unknown rather than supported.

## Report a barrier

Use the [Bug report or Feature request form][issue-chooser]. Describe the
affected screen or task, the steps that lead to the barrier, what you expected,
and the environment (for example, host tooling or a Nagi guest in QEMU). Mention
an input method or assistive technology only if you are comfortable sharing it;
no diagnosis or personal details are needed.

Issues are public. Remove names, account details, private documents,
credentials, identifying local paths, chat identifiers, and any other private
material from descriptions, logs, and screenshots. For a community conduct
concern, follow [CODE_OF_CONDUCT.md][code-of-conduct]. For a security
vulnerability, follow [SECURITY.md][security] and do not disclose it in a
public issue.

## Contributing

See [CONTRIBUTING.md][contributing] for the contribution workflow. Accessibility
reports and suggestions are reviewed against the current specifications and
workstream ownership; a proposal is not an accepted feature until it is
reviewed and recorded through the normal decision process.

[operating-rules]: docs/product-decisions/OPERATING_RULES.md
[m29]: docs/workstreams/NagiOS_M29_Developer_Preview_Polish_Workstream.md
[dec-042]: docs/product-decisions/DEC-042-linux-kernel-transition.md
[dec-043]: docs/product-decisions/DEC-043-nagi-userland-independent-os.md
[issue-chooser]: https://github.com/Roy-To/NagiOS/issues/new/choose
[code-of-conduct]: CODE_OF_CONDUCT.md
[security]: SECURITY.md
[contributing]: CONTRIBUTING.md
