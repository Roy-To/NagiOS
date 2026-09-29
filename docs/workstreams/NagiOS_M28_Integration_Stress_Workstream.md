# Nagi OS M28 — Integration / Stress Workstream

## Current state: PARTIAL

The formal M28 workload combines a 4-vCPU/8-GB guest, Desktop, Files, Notes,
Albert with 3–5 tabs, Granite, audio playback, and Semantic Search. Its
acceptance also requires usable desktop behavior, no kernel OOM, no sustained
audio underrun, CPU fairness during Granite inference, and no major
handle/memory leaks. No combined reference-load run has been performed, so
M28 is not accepted.

## Repeatable guest restart slice

`tests/acceptance/m28_integration_stress.sh` provides a bounded integration
slice using the existing independent persistent disks:

- `out/artifacts/nagi-0.1-m19-search-user-data.img`
- `out/artifacts/nagi-0.1-m22-history-user-data.img`

For each repetition, `--run` invokes the real `./nagi m19` and `./nagi m22`
commands, then validates the latest M19 boot log and M22 third-boot log for
their required kernel, VFS, Search, NH16, and restart PASS markers. Both disks
must already exist and have the expected 16 MiB size. `NAGI_M28_REPEAT_COUNT`
accepts 1–5 (default 2).

The runner uses `git rev-parse` from its own location to find the repository.
It refuses to run when the commands' fixed-name image, OVMF-vars, or serial-log
outputs already exist, because those commands would replace those files. This
keeps a run from overwriting existing acceptance evidence. Between repetitions,
the harness moves generated images, vars, and logs to
`out/evidence/m28-repetition-N/` and snapshots both persistent disks before
starting the next gate. If a prior M19 snapshot already exists, its guest
verifies the previous-boot generation in the initial boot log and may not
create a separate restart log; the harness accepts the fresh initial or restart
log. `--dry-run` reports collisions and validates an existing latest log
without building or booting; `--self-test` checks marker validation and
repeat-count bounds without QEMU.

## Checkpoint evidence — 2026-09-30

- Before the run, the pre-existing fixed-name M19/M22 images, OVMF vars, logs,
  and both persistent disks were preserved under
  `out/evidence/m28-pre-repeat-2026-09-30/`. The first validation attempt also
  produced a passing M19 guest boot but exposed the harness's restart-log-only
  assumption; that output and the updated M19 disk were preserved under
  `out/evidence/m28-first-attempt-2026-09-30/`.
- After correcting log selection and adding per-repetition output archival,
  `NAGI_M28_REPEAT_COUNT=2 tests/acceptance/m28_integration_stress.sh --run`
  completed two real repetitions. Each M19 guest gate printed the
  previous-boot snapshot, guest-search persistence, and M13 acceptance markers.
  Each M22 invocation completed its three QEMU boots and verified restored
  files and NH16 archive state. Repetition 1's images, vars, serial logs, and
  post-run persistent disks are in `out/evidence/m28-repetition-1/`; repetition
  2's final logs remain under `out/logs/`.
- QEMU emitted the existing warning that the host has no virtio-sound input
  driver. The M19/M22 repetitions do not exercise microphone input or measure
  audio playback/underruns, so this warning does not affect these markers and
  leaves the M28 audio-pressure gate unmeasured.
- `sh -n`, `--self-test`, `--dry-run`, and the two-repetition `--run` all pass.
  The first `--run` attempt correctly stopped at a log-selection assumption;
  the fresh M19 guest itself passed, and the harness was corrected before the
  final two-repetition run.

## Workload not covered by this slice

The Search/History gates do not exercise or measure:

- Desktop and Files usability under concurrent load;
- Notes activity or Albert with 3–5 browser tabs;
- actual Granite inference, unload/reload, or CPU scheduling fairness;
- sustained audio playback or underrun pressure;
- kernel OOM behavior or handle/memory-leak soak telemetry.

The M19 guest Search path remains a private deterministic fixture. The M22
guest NH16 flow remains a VFS persistence fixture without authenticated M21
mutation authority. Passing these repeated guest restart checks advances only
this integration slice; it does not satisfy M28's complete reference-load
acceptance.

## Commands

```sh
./tests/acceptance/m28_integration_stress.sh --self-test
NAGI_M28_REPEAT_COUNT=2 ./tests/acceptance/m28_integration_stress.sh --dry-run
NAGI_M28_REPEAT_COUNT=2 ./tests/acceptance/m28_integration_stress.sh --run
```

The final command proceeds only when the required persistent disks exist and
the named generated outputs are absent. M28 remains PARTIAL until the combined
reference workload and its stability criteria are exercised and recorded.
