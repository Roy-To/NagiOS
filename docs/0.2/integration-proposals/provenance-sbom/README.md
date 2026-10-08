# Build Provenance / SBOM host checkpoint proposal

Current status: **User-approved host integration verified**. The newer user
instruction authorizes checkpoint BP-SBOM-HOST-20261008. Five streams are now
registered and the three app cores have explicit host-only activation. See
`HOST_INTEGRATION_CHECKPOINT.md` and the owned integration State for current
source/CI evidence. The original pre-approval proposal below is retained as
historical audit evidence; its BLOCKED/approval-needed statements are superseded.

## Historical pre-approval audit
Audit date: 2026-10-08. This is a proposal record, not an approved checkpoint,
registry entry, owner State, M30 PASS, or release/runtime acceptance.
The user explicitly confirmed that no separate approved integration checkpoint
has been designated. No shared source, registry, Cargo, CI, owner State, Hark,
kernel/loader, third-party source, or 0.1 acceptance file was changed.

## Immutable inputs and adoption audit

| Input | Inspected SHA | Finding |
| --- | --- | --- |
| Integration base | `80ea6246dd394deb5158c44aa7f0012d76a9c409` | 21 streams / 14 State files; BP and Legal absent |
| Build Provenance | `c7e0ec0d13b467ec91b477b7dfd25ef6daf219c3` | Two unadopted commits; fingerprint module and wiring absent on base |
| License/SBOM | `108b3c50e183f187a0d097206286d2902ba31244` | Four unadopted commits; standalone Legal tool absent on base |
| Both feature merge bases | `198b9f60b41eba14e74c2c04f51ccdcb4030adf9` | Use feature deltas, not whole-branch replacement |
| Current main | `9e06967640776298dfdcbceb55085eb9d2b717c5` | M18 PASS; M19–M30 PARTIAL; runtime gate closed |

The integration State and its status document still describe September's M17
blocker; current main's October status describes M19 as the earliest incomplete
milestone. Preserve both histories; do not infer a current release PASS from
the older integration record. Root AGENTS and relevant 0.1/0.2 specifications,
DF-01 registry/schema, development architecture, parallel rules, source State,
registration proposals, source code, current refs, and actual CI jobs/logs were
inspected. Only root AGENTS exists in the inspected target trees.

No duplicate fingerprint/SBOM implementation was found on the integration base.
BP adds the fingerprint module plus small `development.rs` / `main.rs` wiring;
those wiring deltas apply cleanly to the audited base. Legal adds only
`tools/legal/**`, `docs/legal/**`, and its own State. No existing shared
`commands.rs` or CLI manifest delta must be overwritten to adopt either source.
Do not cherry-pick whole histories or revert newer CLIP/SEARCH registrations.
The original BP BLOCKED and Legal PASS States remain untouched.

Hark refs were read only. `hark/desktop-login-m10-m29` at
`4d9cd884c5b0385ffa59a74f6febf8308e9c6445` owns desktop/login work;
`hark/desktop-login-m27-fat12` at `67c55c49601403fcabd7ff98f7c41f0da6fd9dee`,
`hark/m19-files-search-client` at `ddf02d4c9211d0db7d30201f1d43b65537195988`,
and `hark/password-change` at `2be2d17d4bfb6a56dc243acee6aad04a433bb81c`
are also protected. Their differences include kernel, init, shared CLI,
acceptance, and third-party patches. This proposal changes only its own docs
directory; later shared CLI work must stay on the approved host checkpoint.

## Concrete shared-change proposal

1. Approve `checkpoint-proposal.json`: exact dedicated branch, immutable base,
   host-only scope, source adoption SHAs, and bounded edit paths. The registry
   currently assigns the integration owner to `codex/integration-next-phase`,
   not this branch. Approve an explicit branch exception or a separate narrow
   integration stream; do not silently repoint the existing owner row.
2. `registry.patch` adds the two exact existing source proposals. Both rows
   validate against the current schema, with unique IDs/branches/state paths.
   BP's broad CLI paths overlap developer tooling/diagnostics/CI owners;
   checkpoint ownership covers shared wiring only. After approval, adopt source
   files and State verbatim at the reviewed SHAs and record adoption in the
   integration checkpoint, without rewriting source PASS/BLOCKED evidence.
3. `fingerprint-wiring.patch` reuses the source branch's existing routing.
   Adopt `tools/nagi-cli/src/development/fingerprint.rs` and its fingerprint
   contract document from the exact BP SHA after reviewing their source delta.
   No second digest implementation or artifact-reuse mechanism is proposed.
4. `legal-delegation.sketch.patch` proposes the existing Legal `run_from` entry
   point with preserved stdout/stderr and exit status. It is **not compiled or
   applied**. Add the same Legal path dependency to CLI and bootstrap: both
   compile the shared CLI source. Route Legal through bootstrap in both host
   launchers. The sketch includes the POSIX route; Windows routing, help,
   consumer tests, and generated lockfiles must be completed after approval.
   Keep Legal standalone; no root workspace membership is required.
5. Regenerate root and bootstrap locks with Cargo after approved source
   materialization; do not hand-edit resolved checksums. Check the standalone
   Legal lock too. Validate legacy command surface, fingerprint JSON plus
   nonzero mismatch exits, all five Legal commands, invalid/missing input,
   both launchers, registry State, and bootstrap compatibility.
6. Approve host CI changes separately: run standalone Legal fmt/Clippy/tests
   on Ubuntu and Windows and fingerprint/CLI compatibility coverage. Current
   target classification treats root locks, CLI, bootstrap, and launchers as
   target-sensitive. Keep those general target gates intact. A bounded
   checkpoint-specific host policy needs an explicit owner review of exact
   paths and dependency impact; do not blanket-exclude shared CLI or lockfiles.
   No target dispatch, runtime activation, cache reuse, or release claim is
   requested by this proposal. Until that policy is approved, host CI wiring
   and the full BP-SBOM-H08 integration acceptance remain incomplete.

All patches are text proposals stored here. `git apply --check` passes against
the immutable base; no patch was applied. The fingerprint module and Legal
sources are referenced at their owner SHAs rather than copied into the proposal.

## Parallel app registration and implementation gates

The uploaded Calendar and Writer MDs were read completely and compared with
first-party §61 / §57 and current contracts. The third added attachment is
byte-identical to the original integration MD. At final fetch, the new
`docs/add-nagi-0.2-parallel-workstreams` branch at
`cd88de5eba57fc3a294dfd740b122fdfda20a6ec` supplied all three dedicated MDs,
including Sheets. Sheets was then read completely and compared with §58.
This later unmerged docs branch supplies proposal details, not activation.

| Stream | Proposed owner branch / paths | Registration and start condition |
| --- | --- | --- |
| `calendar-core-01` | Codex ②; `codex/0.2-calendar-core-01`; `crates/nagi-calendar-core/**`, owned State/spec; optional `tests/calendar-core/**` | Draft row schema-valid; approve row, optional tests, dependency seams, and explicit host activation; then C1–C5 / CAL-H01–12 |
| `writer-core-01` | Claude ②; `claude/0.2-writer-core-01`; `crates/nagi-writer-core/**`, owned State/spec; optional `tests/writer-core/**` | Draft row fails current `codex/` schema and Rust verifier; approve branch rename retaining Claude ownership OR a coordinated schema/parser/compatibility-test migration; then approve host activation for W1–W5 / WRITER-H01–12 |
| `sheets-calc-01` | Claude ①; `claude/0.2-sheets-calc-01`; `crates/nagi-sheets-core/**`, owned State/spec; optional `tests/sheets-core/**` | Draft row also fails the codex-only schema/verifier; resolve branch policy and approve explicit host activation for S1–S5 / SHEETS-H01–12 |

The three proposed app dependency lists (`development-foundation`,
`app-sdk-contract`) are **proposed** integration choices, not claims that their
MDs declare these registry IDs. Confirm contract/fixture versions before start.
Jobs, Notifications, Identity, Search and Wayback runtime are optional adapter
consumers, not permission to start those services or fake their results.

`claude-branch-exceptions.sketch.patch` shows the second branch-policy option:
permit only the exact proposed Writer/Sheets Claude branches in schema and Rust validation,
preserving existing codex rules. It is unapplied and uncompiled. Approval must
also cover positive Writer/Sheets, legacy codex, malformed-branch and other-Claude
negative compatibility tests; checking patch applicability is insufficient.

Calendar must keep its domain Event identity distinct from
`nagi-history::activity::EventId` (ledger event); inject Clock and resolver rather
than using the host timezone or a second authentication engine. Existing Notes
has a Clock trait but its timestamp/session semantics are not a calendar API.
Writer reuses `nagi-model::ObjectId` / `WorkspaceId` and reviews existing Notes
blocks, history RevisionId/Wayback contracts and SDK semantics. No separate
canonical Object/Activity/Revision authority is approved. Track Changes is not
the OS ledger. Event/Document cores were not found at the proposed paths on
any fetched branch's docs/State inventory; no owner branch currently exists.

Sheets has no existing Workbook/Cell/Formula engine on the integration base.
Reuse model Object/Workspace IDs and SDK Action seams; do not create new
permission or Activity services. Its Clock/date serial policy must be explicit,
not inferred from host timezone or Calendar internals. Incremental dependency
recalculation and bounded parser/cycle rejection are domain-owned tests;
XLSX/CSV, UI, persistence, Search/Wayback runtime and AI remain deferred.

The broad integration owner paths overlap both proposed cores; the dedicated
core paths do not overlap each other. No Notes, History, Search, Jobs, Identity,
or Wayback source edits are authorized. Each app needs its own standalone
manifest/lock, deterministic negative fixtures and owned State after approval;
root Cargo/CI and runtime wiring stay outside its boundary.

**Codex ② handoff:** Calendar is not activated. Starting base candidate is
`80ea6246dd394deb5158c44aa7f0012d76a9c409`, branch proposal is
`codex/0.2-calendar-core-01`, and draft registry validation passes. No Calendar
implementation SHA or test PASS exists. After approval, re-fetch the base,
record the actual starting SHA, create the owner branch/State, and implement
C1 with injected Clock, stable IDs and stale EventRevision rejection.

## Verified host evidence and limitations

- BP at `c7e0ec0d`: pinned-toolchain bootstrap compiles the exact shared source;
  21 fingerprint tests PASS, fmt PASS, warning-denied Clippy PASS. Actual CLI
  identical inputs match; changed artifact bytes and missing requested artifact
  fail closed with exit 4. No private absolute paths in sampled output.
- Legal at `108b3c50`: 13 unit + 17 acceptance tests PASS, locked offline re-run
  PASS, fmt and warning-denied Clippy PASS. Actual repository scan/check/SBOM/
  NOTICE/diff PASS. Unknown licenses and candidate NOTICE remain explicit.
  With `SOURCE_DATE_EPOCH=1780272000`, two SPDX-2.3 outputs are byte-identical,
  SHA-256 `5eb9cb25cbd87e3a36550ec28d71ef604cf925ac6d75f6df8f71868a06d6f5f3`.
  Both were generated from exact Legal source SHA above. This is a focused SPDX
  profile check, not full independent SPDX certification or legal clearance.
- Base `./nagi dev verify` PASS: 21 streams / 14 State files. Proposal branch
  `dev status` and `dev resume` exit 4 because it is not registered; expected
  until owner approval, not bypassed. Base CLI bootstrap suite: 107 PASS / 1
  HOST_ENV failure, missing `third_party/mozjs-sys-nagi/build.rs`.
  Root workspace tests also require source materialization; do not edit or fetch
  third-party sources under this request to make that unrelated test pass.
- Rust was initially absent. Pinned nightly-2025-08-01, fmt and Clippy were
  installed only in ignored local audit output with scoped Cargo/Rustup roots.
  Packages were populated once online and focused verification rerun offline.
- Source CI failures were re-read through GitHub: BP run `36412412685` and
  Legal run `36419566860` fail the existing unregistered-branch CLI test;
  each target job is skipped. Both Ubuntu and Windows job metadata was checked;
  actual Ubuntu logs confirm the precise missing registry rows.
- Integration run `36967426363` at `150f4362` passes both hosts, target skipped;
  manual run `36969218693` at the same SHA passes both hosts but fails M17
  acceptance. Neither is CI for base `80ea6246` or this proposal HEAD.
  CLI `gh` API requests receive Forbidden in this environment; the connected
  GitHub read tool supplied actual runs/jobs/logs instead.

Acceptance: H02–H07 have existing-source host evidence; H01 has proposal schema
validation only; H08 shared CLI/Windows integration is blocked; H09 retains exact
scope/SHA/CI; H10 requires final pushed-HEAD/clean-tree audit. No integrated host
PASS is claimed. Raw local command logs and generated artifacts are under
ignored `out/host-integration-audit/`; compact reproducible evidence is in
`audit-state.json` and `ci-evidence.json`.

## Required next owner decisions

Approve the dedicated host checkpoint and exact shared edit scope; approve BP
and Legal registration/adoption and host CI policy; approve Calendar row and
host activation; resolve Writer/Sheets branch/schema policy and approve their
rows and host activations. M30 PASS plus an explicit release boundary
remains mandatory for all future 0.2 runtime work.
