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

- `out/artifacts/nagi-0.1-m19-vfs-objectid-user-data.img`
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
  post-run persistent disks are in
  `out/evidence/m28-repetition-1-before-m21-file-move-20260930/`; repetition
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

## Isolated artifact namespace continuation — 2026-09-30

The M19 acceptance now uses the isolated `nagi-0.1-m19-vfs-objectid-*` image,
disk, and OVMF vars names, with `m19-vfs-objectid-*.log` serial outputs. The
M28 harness follows those names and requires the live VFS file/ObjectId marker
in addition to the previous-boot Search markers. This preserves the earlier
`m19-search-*` evidence. The harness self-test and dry-run passed with the new
M19 namespace and the existing M22 disk/logs; the dry-run reported the
expected collision guards without writing or booting anything. Full repeated
M28 invocation requires archiving generated image/log outputs first. Before
the real check, the accepted M19 and M22 images, vars, logs, and disks were
copied to `out/evidence/pre-m28-continuation-20260930/`; generated image/log
files were moved there to clear the runner's collision guard. A real
`NAGI_M28_REPEAT_COUNT=1 ... --run` then passed the current M19 gate and the
three-boot M22 gate. M28 remains PARTIAL because this Search/History slice does
not measure its combined reference workload.

## M21 guest action integration continuation — 2026-09-30

Before rerunning the combined harness after adding M21 `file.search`, the
current M19/M22 images, OVMF vars, serial logs, and both persistent disks were
preserved under `out/evidence/pre-m28-m21-action-20260930/`. A real
`NAGI_M28_REPEAT_COUNT=1 ... --run` passed: M19 executed the guest
Plan/Validate/Action Registry/Executor path against the persistent Search
Service, then M22 passed all three NH16 guest boots with that same M19/M21
path active before its transaction-state checks. Those outputs were later
preserved under `out/evidence/pre-m28-m21-file-move-20260930/` before the next
run. The harness shell check, self-test, and collision-free dry-run also
passed. M28 remains PARTIAL; that run did not exercise its desktop/model/audio
reference workload.

## M21 file.move / M22 transaction continuation — 2026-09-30

The M22 initial three-file mutation now runs through the M21 guest
Plan/Validate/Capability/Executor path and persists its NH16 Prepared and
Committed states around real VFS renames. A fresh-disk `./nagi m22` run passed
that action on boot 1, grouped reverse-order Undo on boot 2, and restored files
plus Undone state on boot 3. The fresh action run's logs and pre-M28 disk
snapshot are retained in
`out/evidence/pre-m28-m21-file-move-20260930/`.

After preserving the current M19/M22 images, vars, logs, and disk snapshots,
the updated `NAGI_M28_REPEAT_COUNT=1 ... --run` passed both M19 Search and the
three-boot M22 NH16 restart gate. Its final serial logs remain at
`out/logs/m19-vfs-objectid-initial.log` and
`out/logs/m22-history-boot-1.log` through `m22-history-boot-3.log`; its final
images and OVMF vars are in `out/artifacts/`. QEMU again reported that no host
virtio-sound input driver is available; this repetition does not exercise
audio. M28 stays PARTIAL because Desktop/Files/Notes/Albert, Granite inference,
audio pressure, OOM behavior, CPU fairness, and leak soak were not measured.

## M22 AI Activity Ledger continuation — 2026-09-30

Before the M22 ledger run and updated M28 regression, the existing M19/M22
generated images, OVMF vars, serial logs, and persistent disks were preserved
under `out/evidence/pre-m22-ai-activity-ledger-m28-20260930/`. The fresh-disk
M22 three-boot run passed the M21 `file.move` action, reopened the separate
NAL1 Committed record, persisted the `UndoPending`/`Undone` transitions, and
verified restored files plus the ledger after restart. Its logs and prior
inputs remain in the evidence directory.

The updated `NAGI_M28_REPEAT_COUNT=1 ... --run` then passed the real M19
ObjectId/Search gate and all three M22 boots with the new Activity Ledger
marker required by the harness. Latest M19 and M22 serial logs remain in
`out/logs/`, and the generated boot images/vars are in `out/artifacts/`.
`bash -n` and `--self-test` pass. QEMU again reported no host virtio-sound input
driver; this Search/History run does not measure audio. M28 remains PARTIAL
because its combined desktop/model/audio workload and stability criteria are
still unmeasured.

## Commands

```sh
./tests/acceptance/m28_integration_stress.sh --self-test
NAGI_M28_REPEAT_COUNT=2 ./tests/acceptance/m28_integration_stress.sh --dry-run
NAGI_M28_REPEAT_COUNT=2 ./tests/acceptance/m28_integration_stress.sh --run
```

The final command proceeds only when the required persistent disks exist and
the named generated outputs are absent. M28 remains PARTIAL until the combined
reference workload and its stability criteria are exercised and recorded.
