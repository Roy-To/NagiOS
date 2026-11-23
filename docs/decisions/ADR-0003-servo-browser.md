# ADR-0003: Servo Browser Engine

**Status:** Accepted

Albert uses Servo as its browser engine. Chromium, WebKit, and NetSurf are not
fallback engines for Nagi 0.1.

## Related decision (2026-11-21)

- [DEC-115](../product-decisions/DEC-115-web-compatibility-engine-drm-and-codecs.md) — For the Linux edition, the restriction on fallback engines is lifted. Servo remains the baseline for site testing, and the fallback approach will be decided after measurement. This ADR records the Nagi 0.1 decision.
