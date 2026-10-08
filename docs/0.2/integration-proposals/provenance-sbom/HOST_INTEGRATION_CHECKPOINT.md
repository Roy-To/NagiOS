# Approved Build Provenance / SBOM host integration

Checkpoint: BP-SBOM-HOST-20261008. Branch: `codex/0.2-integrate-provenance-sbom`.
The current user instruction explicitly approves the previously concrete host
proposal and requests continued execution without further permission stops.
The dedicated registry row and checkpoint approval record bound that authority.

## Registration and app activation

Registration/activation checkpoint `cd871f905bd06ef406ccaa9f80c484f19158209f`
adds Build Provenance, License/SBOM, Calendar Core, Writer Core, Sheets Core,
and the dedicated Integration Owner row. There are 27 registered streams.
Registry tests (18), legacy CLI tests (26), schemas, fmt and Clippy passed.
Ubuntu/Windows host CI run
[`37777255126`](https://github.com/RT-NISH/NagiOS/actions/runs/37777255126)
passed for that exact SHA. The separate legacy workflow is deliberately skipped
on the explicitly named host-only branches; its general 0.1 gates are unchanged.

| Owner | Registered branch | Starting commit | Authorized work |
| --- | --- | --- | --- |
| Codex ② | `codex/0.2-calendar-core-01` | `cd871f905bd06ef406ccaa9f80c484f19158209f` | C1–C5 / CAL-H01–12, `crates/nagi-calendar-core/**`, owned State/spec and `tests/calendar-core/**` |
| Claude ② | `claude/0.2-writer-core-01` | `cd871f905bd06ef406ccaa9f80c484f19158209f` | W1–W5 / WRITER-H01–12, `crates/nagi-writer-core/**`, owned State/spec and `tests/writer-core/**` |
| Claude ① | `claude/0.2-sheets-calc-01` | `cd871f905bd06ef406ccaa9f80c484f19158209f` | S1–S5 / SHEETS-H01–12, `crates/nagi-sheets-core/**`, owned State/spec and `tests/sheets-core/**` |

All three remote branches were created at the starting commit. Implementations
and owned State creation belong to those owners. Registration/activation is
complete; no app code or acceptance PASS is claimed. Existing model IDs and
SDK contract seams are dependencies; optional Jobs/Notification/Search/Wayback
adapters do not authorize actual runtime providers. The exact two named Claude
branches are accepted by matching schema/parser rules and positive/negative
compatibility cases; no general Claude namespace exception was added.

## Reused source and minimal shared wiring

Fingerprint source is adopted from
`c7e0ec0d13b467ec91b477b7dfd25ef6daf219c3`; Legal source from
`108b3c50e183f187a0d097206286d2902ba31244`. The new source modules, fixtures,
standalone locks and owner State snapshots are adopted byte-for-byte. No whole
branch merge or overwrite of newer registry/CLIP/SEARCH records was performed.
The source owner's original BLOCKED/PASS State is immutable adoption history;
current integration facts belong in `.dev/workstreams/provenance-sbom-integration/state.json`.
Legal README alone is adapted to describe the now-registered CLI interface.

`commands::execute` routes fingerprint to the existing implementation, so binary
and library users share one dispatcher. This preserves structured comparison
output with nonzero mismatch exits and avoids the original binary-only bypass.

`nagi legal ...` runs the existing standalone Cargo manifest with fixed argv,
`--quiet --locked --offline`, inherited stdout/stderr and its exit status. Both
launchers route Legal through bootstrap. Cargo is already a required developer
host tool. This thin delegation avoids new shared dependencies or changes to
root Cargo.toml/Cargo.lock and bootstrap lock; no second legal/digest algorithm
is introduced. The earlier direct-link sketch is superseded by this smaller
implementation. `nagi-bootstrap` also compiles the existing CLI integration
tests, giving host CI old-command compatibility coverage without target sources.

## Host verification and CI boundary

Focused checks pass on Linux: 39 development/fingerprint tests, 29 CLI tests
(26 existing + 3 shared-dispatch/Legal tests), and 30 standalone Legal tests.
Both affected manifests pass fmt and warning-denied Clippy. Schema validation
and `./nagi dev verify` pass with the registered ownership and imported States.
The shared CLI tests exercise every Legal command, JSON stdout, invalid command/
arguments, missing inventory input, deterministic SPDX output, candidate NOTICE,
and missing-artifact failure through both fingerprint dispatch paths.

`.github/workflows/0.2-host-integration.yml` runs this host scope on Ubuntu and
Windows, including each app core only when its owner manifest exists. Missing
app implementations print NOT STARTED. The pre-existing full-workspace/target
workflow excludes only these four exact host branches, whose explicitly approved
scope is host-only. Other branches keep the original host and target gates.
No 0.1 test was deleted, weakened or altered. Full target-dependent workspace
tests require third-party sources and remain an independent 0.1 gate.

The limited checkpoint does not dispatch target/QEMU jobs, cache or reuse release
artifacts, activate guest services, change M30 status, or claim release success.
Hark/main, kernel/loader/init, third-party sources and 0.1 acceptance are protected.
M30 remains PARTIAL; every future 0.2 runtime integration still requires M30 PASS
and an explicit release boundary. Final exact source SHA, generated artifact
digests, CI runs and any material failure are recorded in the integration State.
