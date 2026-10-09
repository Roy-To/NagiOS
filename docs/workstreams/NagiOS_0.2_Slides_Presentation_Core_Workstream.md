# Slides Presentation Core — slides-core-01

Base: `5b183660eee9babc1478fd05386e412026a0b87a`. Dedicated branch:
`codex/0.2-slides-core-01`; worktree: `/workspace/NagiOS-slides-core-01`.

The explicit delegated user instruction authorizes this standalone host
foundation before M30. The checked-in 0.1 status has M19–M30 PARTIAL; production
runtime requires M30 PASS and a separate explicit 0.2 owner checkpoint.
Host PASS is not product Slides acceptance or runtime PASS.

Owned paths: `crates/nagi-slides-core/**`, `tests/slides-core/**`, this document,
`.dev/workstreams/slides-core-01/**`. Shared registry/CI/root Cargo/lock/ABI,
0.1, existing 0.2 and ARM64 work remain unchanged. Registration is proposed in
the owned directory for the Integration Owner; no registration is claimed.

## Scope and implementation

First-party spec §59.2–59.7 / SLIDES-002, -003, -005, -015 defines the foundation.
Reuses canonical ObjectId/RevisionId and Writer/Calendar standalone conventions;
no new identity, Action, Job, authority or history service is introduced.

Presentation/Theme/Layout/Slide/Object, logical geometry, notes and source
references are bounded. Transactions support slide create/delete/duplicate/
reorder, object add/delete/replace, notes/layout/theme/title changes. Expected
revision conflicts, invalid IDs/geometry/references and limits fail atomically.
Tombstones prohibit reuse; duplication requires supplied fresh IDs, remaps copied
navigation references and preserves external resource/revision metadata.

Logical integer millipoints remain independent of desktop pixels. Themes define
fonts/colors/slide size; layout references include title/content, section header,
title only, blank and custom kinds. Source metadata supports Sheets chart/table,
Writer, Notes and Albert provenance. An injected revision reader reports current
or changed baselines without fetching or executing updates. Explicit object edits
can accept a selected baseline. Update/Compare/Keep-current UI is deferred.

Native v1 is deterministic, bounded, versioned and validates the complete model.
It preserves metadata, IDs, tombstones, geometry, layouts, notes and references.
UTF-8 lengths are bytes; NUL is rejected. Unknown versions never fall back. No
filesystem/clock/network/display calls occur. See the crate README for wire and
geometry details and hard limits.

## Host acceptance and evidence

Focused acceptance suite verifies stable identities, duplicate remapping,
reorder/delete references, rollback including issued IDs, stale revision,
geometry limits, theme/layout references, UTF-8/empty notes, count/byte/operation
bounds, deterministic native reopen, all truncated prefixes, invalid UTF-8/
lengths/versions/revisions, revision overflow and unavailable adapters.
Exact results and verified code SHA are in the owned DF-01 state and evidence.

```sh
bash crates/nagi-slides-core/verify.sh
git diff --check
```

Package-scoped fmt, locked offline tests and warnings-denied Clippy are required.

The first Host PASS claim was withdrawn after independent review identified
unbounded intermediate DuplicateSlide cloning. The correction measures exact
native sizes without allocating a buffer, checks every intermediate snapshot
before cloning content, charges dynamically resolved duplicate sources to a
cumulative batch budget, and removes the duplicate's second clone. Small-budget
tests cover repeated duplicate/delete cycles, a temporary oversize whose final
state would fit, exact and one-byte-short limits, and sources created/edited in
the same batch. Current corrected acceptance has 25 tests; superseded 21-test
logs remain historical only. See `bounds-review.md` and updated State for exact
verified corrected SHA. Publication remains on hold.
Owner registration and CI adoption are separate pending actions; no extra shared
workflow or duplicate guest CI is created. Standalone CI step proposal:
`bash crates/nagi-slides-core/verify.sh` after preparing locked dependencies.

## Publication and deferred gates

Local author and committer are both `NagiOS <2026nagios@gmail.com>` via per-command
environment, with no authentication or global configuration changes. Personal
RT-NISH GitHub access was verified read-only; no existing Slides branch/PR found.
Cloud Git default is known to use forbidden company authentication (403), so it
is not invoked. The personal connector create-commit schema has no author or
committer override and cannot preserve the required identity. Push/draft PR/CI
are blocked on a permitted identity-preserving personal publication route.
An exact local Git bundle, patch, sources/tests and evidence are retained as the
handoff; private Library saving is attempted and its actual result reported.

PPTX/PDF/rendering/media/table/grouping/presenter, guest storage, source provider
execution, AI generation and Activity/Wayback execution remain adapter/deferred
work. PDF/PPTX exports and missing source adapters return AdapterUnavailable.
The next owner action is registration/host CI review and personal publication,
followed by the separate runtime gate when explicitly activated.
