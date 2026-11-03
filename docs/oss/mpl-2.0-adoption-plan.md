# Nagi OS MPL-2.0 adoption record

Policy selected: 2026-10-26. Scoped source grant prepared: 2026-10-27.

The maintainer selected MPL-2.0 for original Nagi OS code and requested its
implementation. The operative grant and its exceptions are now in
[LICENSE_SCOPE.md](../../LICENSE_SCOPE.md), with the full legal text in
[LICENSE](../../LICENSE). In revisions containing these files, this is no
longer merely a candidate-license plan.

## What the grant does

It applies to the maintainer's licensable original software source and build
configuration within the identified Nagi code boundaries. It does not relicense
third-party components, prior MIT/Apache grants, model weights, media or unknown
provenance material. The four previously dual-licensed packages retain their
existing declarations. The embedded glyph file and related held-out assets are
not made license-cleared by this decision.

See [the scope review](mpl-2.0-scope-review.md) for exactly what was inspected,
the difference between a licensing decision and independent ownership evidence,
and unresolved items. A directory rule is not an exhaustive provenance audit.

## Gates that remain open

- Confirmation of excluded glyph, logo, screenshot, documentation and data rights.
- Rights held by other people or employers, where applicable; no implied
  contributor consent or retroactive revocation of earlier permissions.
- Release-specific dependencies, native libraries, firmware, model provenance,
  required notices, covered-source access and SBOM review.
- A truthful application reflecting the merged scope and current support-program
  criteria. No OpenAI or Anthropic application has been submitted by this change.
- Nagi 0.1 runtime and release acceptance, unchanged by source licensing.

These are not all prerequisites to granting the maintainer's own identified
code; they remain prerequisites wherever the affected material is distributed
or a broader clearance claim would be made.

The [2026-10-11 Apache review](apache-2.0-readiness.md) is historical evidence.
Its Apache recommendation and unselected-project-license statements are
superseded by this record and LICENSE_SCOPE.md. Its third-party and provenance
findings are retained, not declared resolved.
