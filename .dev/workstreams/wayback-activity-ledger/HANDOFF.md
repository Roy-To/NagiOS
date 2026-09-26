# Wayback / AI Activity Ledger Foundation — Handoff

- Workstream: `wayback-activity-ledger`
- Branch: `codex/0.2-wayback-ledger`
- HEAD: `d52edd3ab2fe29cc32bde92720476a622f2688d3`
- Base: `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9` (DF-01)
- Status: `IN_PROGRESS` — host-side implementation and local acceptance pass; registry/state handoff checkpoint, push, and CI review remain.

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

CI: not yet run at this handoff checkpoint; the dedicated branch has not been pushed yet.

## Deferred integration

The following remain gated on Nagi 0.1 M30 PASS and an explicit 0.2 integration checkpoint: filesystem change hooks, app-state hooks, AI/tool action hooks, a durable snapshot backend, restore execution, Activity/Wayback UI, and Capability/principal lookup. The exact adapter contracts and future acceptance checks are recorded in `docs/0.2/wayback/architecture.md` and `state.json`.

## Dependencies and integration conflicts

- Dependency: DF-01 development-foundation contract v1, PASS at `ab9a580c04f0fa7c18ff6b996ac370ce15cd8df9`.
- `.dev/workstreams.json` has one appended Wayback registry entry. It is the only shared source/config file changed.
- The standalone package and registry entry need owner review when a future 0.2 integration branch reconciles workspace membership and ownership. No other workstream implementation was copied or modified.
- M17's known graphics/acceptance status is separate from this host-side acceptance.

## Next exact action

Commit the registry/state/handoff checkpoint, push `codex/0.2-wayback-ledger` to `origin` with a normal push, then inspect the branch CI. Record the remote HEAD and CI result in `state.json`; classify any M17-only failure separately from Wayback acceptance.
