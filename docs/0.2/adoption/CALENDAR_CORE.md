# Stage 2: Calendar Core host adoption

Authorization: the current user explicitly approves host-only Calendar adoption,
verification, commits, push, separate PR and merge after all applicable checks
PASS. The fresh adoption branch begins at the merged Stage 1 main checkpoint.
The feature source branch remains unchanged.

Reviewed sources: fb00ad7eab1a5c981cb53a019e938323e81da759 (published owner
checkpoint), 2ef5c494afe2c0555ca18c8ef74cf1c5f9cc9c96 (verified implementation).
Adopt only the Calendar-owned delta against registered owner activation base
cd871f905bd06ef406ccaa9f80c484f19158209f. Keep original Calendar State and
published verification artifacts byte-identical; current adoption evidence belongs
to `.dev/workstreams/calendar-core-adoption/state.json`. Retain current main's
shared registry, CLI, dependencies, CI and other app specifications.

C1-C5 code review covers validated local CRUD, immutable IDs/revisions and bounded
in-memory tombstones; explicit temporal domains and injected DST resolution;
bounded recurrence with original-key exceptions; explicit unknown availability;
and external invites that always remain unavailable even after confirmation.
Owner fields are metadata, not authenticated authority. No providers, real
notifications/jobs, host timezone lookup or runtime service is enabled.

The current canonical `nagi-model` API differs from the old Git pin only by
additive Hash/Ord derives on existing IDs. Use the current in-repository path
model at version 0.1.0 and regenerate only the Calendar standalone lock. Root
Cargo files remain unchanged. Preserve all 61 original tests and algorithms.
Package-scoped Calendar rustfmt checks all Calendar modules without traversing
unowned parent workspace/third-party paths on Windows. Clippy remains
warnings-denied across all targets; tests remain locked and offline after package
preparation. The owner verify script and shared dedicated host CI use that same
Calendar-only formatter scope; other app format rules remain unchanged.

M30 remains PARTIAL. This host acceptance does not activate 0.2 guest/runtime,
claim product Calendar acceptance, modify Kernel/Loader/init or third-party
sources, alter 0.1 acceptance, touch Hark branches, implement other app cores,
or begin 0.3. Real timezone providers, durable offline storage, GUI, authenticated
caller/policy adapters, Search/People/Mail and external provider delivery remain
separate later gates.

Stage 1 [PR #34](https://github.com/RT-NISH/NagiOS/pull/34) merged at
`b4f42ec91a533fc11691490c04d38c4cba0ae396` after full PR CI
[37801690965](https://github.com/RT-NISH/NagiOS/actions/runs/37801690965)
and Ubuntu/Windows host CI [37801690899](https://github.com/RT-NISH/NagiOS/actions/runs/37801690899)
PASS on reviewed source `52a21862dcbf0495ff6bdcdf996736e9535a2c6d`.
The previous M27 VNC collision was resolved by a fresh runner; no guest
source or acceptance criterion was changed. Stage 2 begins from that exact
merged main tree, verified identical to the tested Stage 1 head.

Host validation on exact published source
`99e5e0bcfd48611155aef8b5e97b4feccad5ade7`: original 61 tests (1 unit +
60 acceptance), package format, warnings-denied all-target Clippy, shared
CLI/Legal and Registry/State checks PASS on both Ubuntu and Windows in
[37813693162](https://github.com/RT-NISH/NagiOS/actions/runs/37813693162).
Local Legal structural check and schema/immutable-owner/ownership verification
also PASS. Review completed; [PR #35](https://github.com/RT-NISH/NagiOS/pull/35)
may merge only after every latest applicable host regression check PASS.
The current acceptance State records verified source evidence separately
from the PR merge gate.
