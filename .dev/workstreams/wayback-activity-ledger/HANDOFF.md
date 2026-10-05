# Wayback / AI Activity Ledger Foundation — Handoff

- Workstream: `wayback-activity-ledger`
- Branch: `codex/0.2-wayback-ledger`
- Last verified implementation commit: `d52edd3ab2fe29cc32bde92720476a622f2688d3`
- Registry/state/handoff checkpoint: `6af4849f4d1b1cd9b2c01ddf2421608d9f15b62c`; pushed branch was verified at `f95ddab4e0498c84bc831213512365d7269bfaac` before this final handoff update.
- Base: `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9` (DF-01)
- Status: `PASS` — host-side Wayback acceptance criteria pass. The CI failure below is an inherited DF-01 state/test mismatch outside this workstream's ownership.

The linked [`state.json`](state.json) is the canonical machine-readable status.

## Completed

- Added the standalone host contract crate `crates/nagi-wayback` with versioned activity entries, generic actor/provenance, transaction lifecycle, reversibility, append/query serialization, and chained integrity verification.
- Added metadata-only snapshot storage, snapshot-parent dependency resolution, restore points, transaction undo candidates, restore planning, and explicit non-executable blockers. No restore is executed.
- Added Draft 2020-12 schemas, privacy/migration/ownership documentation, 20 unit tests, integration scenarios A–D, and the repeatable `tests/wayback/verify.sh` harness.
- Added restore-point coverage checks so later exact actions outside snapshot-covered scopes are represented and unsafe or uncommitted work blocks the plan.
- Kept filesystem/runtime hooks, M17/M18–M30, UI, App SDK, Capability policy, root Cargo manifests, and third-party sources outside the implementation diff.

Last verified code commit: `d52edd3ab2fe29cc32bde92720476a622f2688d3`.

## Verification

| Check | Command | Result |
|---|---|---|
| Wayback acceptance harness | `CARGO_TARGET_DIR=/Users/tozawa/Developer/NagiOS-0.2-wayback/target ./tests/wayback/verify.sh` | PASS, exit 0: fmt, 20 unit tests, 4 integration scenarios, Clippy `-D warnings`, schema positives and negatives, M15 regression, DF-01 verify |
| Existing M15 history regression | `cargo test --manifest-path user/nagi-history/Cargo.toml --locked` (inside the harness, pinned toolchain) | PASS, exit 0: 4 tests |
| Schema validation | `tests/wayback/validate_schemas.py` via harness | PASS, exit 0: 6 serialized contracts, unknown-version negatives, empty-ID and recorded-secret negatives, DF-01 registry/state |
| DF-01 workstream verify | `./nagi dev verify` (inside the harness) | PASS, exit 0 |
| Formatting and whitespace | `cargo fmt --manifest-path crates/nagi-wayback/Cargo.toml --all -- --check`; `git diff --check` | PASS |

The initial direct `rustc --test` attempt for M15 failed because it did not link the path dependency `nagi-model`. DF-01 `diagnose` recorded that failure at `/tmp/nagi-wayback-m15-standalone-rustc.log` (SHA-256 `dd6e049f7fc2b267bc6515f907a6b70002ac8ce86ea185eab4d4b081b5d95f4a`). The harness now uses the package's Cargo manifest; the 4 M15 tests pass. Earlier Clippy and Python schema setup failures were fixed and the final harness passes.

This worktree does not contain fetched root `third_party` patch sources. The successful DF-01 CLI checks used temporary symlinks to the same pinned source directories in the clean DF-01 prep worktree, then removed those links. A subsequent check after removal failed at `third_party/cc-nagi/Cargo.toml`; DF-01 `diagnose` classified it as `HOST_ENV` (log `/tmp/nagi-wayback-missing-third-party.log`, SHA-256 `67fed48104287ae2b21284ad645003bba04aef94b6e37c2cd2f4e11c97c653c5`). This is the fresh-worktree bootstrap prerequisite, not a Wayback compile failure. `./nagi fetch` is the normal source preparation step for a fresh checkout. No symlink or third-party source change is part of this branch.

## CI result and ownership classification

- GitHub Actions run [36213800837](https://github.com/RT-NISH/NagiOS/actions/runs/36213800837), at `f95ddab4e0498c84bc831213512365d7269bfaac`, completed with failure in both `ubuntu-host` and `windows-launcher` at `Test host-compatible workspace`. Both fail the same DF-01 test, `development::tests::registry_and_active_workstream_state_validate_from_repository`: `tools/nagi-cli/src/development.rs:1277` expects `development-foundation` state `IN_PROGRESS`, while that state is `PASS`.
- The mismatch is present in the selected DF-01 base: commit `ab9a580` changes `.dev/workstreams/development-foundation/state.json` from `IN_PROGRESS` to `PASS` without updating the assertion. The earlier run [36117826259](https://github.com/RT-NISH/NagiOS/actions/runs/36117826259) on `18b364dc` had both host jobs pass before that DF-01 state change. This is an inherited foundation test/state inconsistency, outside Wayback's ownership (`tools/nagi-cli/**` is not in this stream's allowed paths). No foundation code or test was changed here.
- `nagi-target` was skipped after the host jobs failed. This CI run therefore reports no M17 result; the known M17 graphics gate remains separately owned and is not a Wayback failure.
- DF-01 `diagnose` recorded the filtered host-test failure at `/tmp/nagi-wayback-ci-foundation-diagnostic.json` (log SHA-256 `bf93510411a430fccbba7b57cca7124bb52f049d0a1b85874ea52bb4a73d6f89`). It preserved the declared `BUILD` class and suggested `UNKNOWN`; the concrete cause was identified by the assertion and the `18b364d..ab9a580` state diff.

## Deferred integration

The following remain gated on Nagi 0.1 M30 PASS and an explicit 0.2 integration checkpoint: filesystem change hooks, app-state hooks, AI/tool action hooks, a durable snapshot backend, restore execution, Activity/Wayback UI, and Capability/principal lookup. The exact adapter contracts and future acceptance checks are recorded in `docs/0.2/wayback/architecture.md` and `state.json`.

## Dependencies and integration conflicts

- Dependency: DF-01 development-foundation contract v1, PASS at `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9`.
- `.dev/workstreams.json` has one appended Wayback registry entry. It is the only shared source/config file changed.
- The standalone package and registry entry need owner review when a future 0.2 integration branch reconciles workspace membership and ownership. No other workstream implementation was copied or modified.
- M17's known graphics/acceptance status is separate from this host-side acceptance.

## Resume and integration boundary

Resume this host-side workstream with `./nagi dev resume wayback-activity-ledger`. No in-scope implementation remains. Filesystem/app/AI hooks, durable guest snapshots, restore execution, UI, and principal lookup remain deferred until Nagi 0.1 M30 PASS and an explicit 0.2 integration checkpoint. Do not resolve the inherited DF-01 test/state mismatch from this branch; it belongs to the foundation owner.
