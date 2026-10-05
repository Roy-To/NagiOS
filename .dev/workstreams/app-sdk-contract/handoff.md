# App SDK / First-Party App Contract Handoff

## Result

**PASS — host-side contract acceptance only.** The implementation source is
`0117bf51830fc77863b3468f0e8406b3eab4817b` on branch
`codex/0.2-app-sdk-contract`, in worktree
`/Users/tozawa/Developer/NagiOS-0.2-app-sdk`. The authoritative live status and
test evidence are in [`state.json`](state.json); this handoff is a concise
integration note, not a second state ledger.

The work adds the no-std Rust App Contract v1, App Manifest v1 schema and
fixtures, a strict host manifest loader/CLI, a caller-backed registry with
duplicate and numeric AppId collision rejection, and an end-to-end host
contract test. The SDK now covers identity, localization, lifecycle and
shutdown hooks, state versioning/migration, intents/deep links, IPC envelopes,
common errors, and capability request handoff. No runtime adapter, real app,
Capability policy, Wayback implementation, M17/M18–M30 change, or mainline
merge was made.

## Verified at the implementation commit

- SDK: 44 tests; focused format and Clippy passed.
- `nagi-pkg`: 7 loader/integration tests; four fixture CLI validations; focused
  format and Clippy passed.
- Manifest schema: 4 valid fixtures accepted and 14 malformed documents
  rejected. Cross-field state-version ordering and duplicate declaration IDs
  are checked by typed semantic validation, as portable JSON Schema cannot
  express those comparisons.
- Existing M16 package/signing regression: 5 tests passed; package Clippy
  passed with existing `third_party/libc-servo` `target_os=nagi` warnings.
- `./nagi dev status`, `./nagi dev resume` (no argument selects the current
  branch), and `./nagi dev verify` passed.

Two failures are recorded separately in state and are outside this contract's
owned paths: full `./nagi fmt` reports 416 formatting-diff files, all under
`third_party/servo` and none in this workstream; the M16 QEMU first-boot attempt
fails at M5 with `invalid-elf` and times out at 90 seconds. The M16 package
unit/regression tests pass. No vendored or runtime code was changed to mask
these results.

## Integration-owner review

DF-01 marks the registry and Cargo manifests/locks integration-owned. This
feature branch therefore carries only focused proposals in:

- `.dev/workstreams.json` — one appended row for this workstream.
- `tools/nagi-pkg/Cargo.toml` and `tools/nagi-pkg/Cargo.lock` — direct pinned
  `serde_json` dependency for the host manifest parser.

Before integrating either proposal, the integration owner should create the
required checkpoint, confirm registry/dependency consumers, and record the
expected migration/review plan. No merge to `main` or the DF branch is part of
this workstream.

## Gate and next action

Keep all runtime binding deferred until Nagi 0.1 M30 is PASS and an explicit
0.2 integration checkpoint exists. At that checkpoint, review the manifest
and registry contract with the app-host, Capability, filesystem/state, IPC,
and compositor owners before implementing adapters. After the final
state/handoff commit is pushed, verify the remote head and record relevant CI
evidence in `state.json`.
