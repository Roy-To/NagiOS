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
must already exist and have a supported size: legacy 16 MiB or current 18 MiB
GPT-backed User Data. The M19/M22 commands migrate supported legacy disks
before guest use while preserving the old raw image. `NAGI_M28_REPEAT_COUNT`
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

## Legacy User Data migration regression — 2026-09-30

The Completion Sweep reran M19 and M22 against preserved 16 MiB User Data
images. Both guest acceptances passed and `ensure_persistent_disk` migrated the
legacy images to the current 18 MiB GPT-backed form, preserving the original
images in the pre-run evidence directory. This exposed a stale 16 MiB-only size
check in the M28 dry-run harness. The guard now accepts only the two supported
sizes, and its self-test checks both boundaries. After the fix, `sh -n`,
`--self-test`, and `--dry-run` all passed against the migrated disks and latest
M19/M22 logs without building, writing disks, or invoking QEMU. The full M28
slice remains PARTIAL because the desktop/model/audio stress workload is still
unmeasured.

## Cross-workstream regression — 2026-09-30

After the M25 TTS provider contract and QEMU fixture changed the shared audio
package, the Completion Sweep reran `./nagi m19` and `./nagi m22`; Search/ObjectId
persistence and three-boot grouped Undo both passed. The shell check, harness
self-test, and dry-run passed again against those latest logs and current GPT
User Data images. The dry-run remains a marker/overwrite preflight only and
does not claim the combined M28 desktop/model/audio stress criteria.

## M24 persistent semantic-index continuation — 2026-10-01

The Completion Sweep added M24 semantic-index ready and persistence markers to
the M28 M19/M22 log gates and self-test fixtures. `sh -n`, `--self-test`, and
`--dry-run` pass against the updated M19 restart log and M22 third-boot log.
The underlying `./nagi m19` run passed two boots, and `./nagi m22` passed three
boots; the latter's third boot verified both the semantic index and the
existing NH16/NAL1 Undo state. This run refreshed the guest Search/History
slice only and did not execute M28's combined reference workload. Logs,
artifacts, and disks are preserved under
`out/evidence/m24-persistent-semantic-index-20261001/`.

## Commands

```sh
./tests/acceptance/m28_integration_stress.sh --self-test
NAGI_M28_REPEAT_COUNT=2 ./tests/acceptance/m28_integration_stress.sh --dry-run
NAGI_M28_REPEAT_COUNT=2 ./tests/acceptance/m28_integration_stress.sh --run
```

The final command proceeds only when the required persistent disks exist and
the named generated outputs are absent. M28 remains PARTIAL until the combined
reference workload and its stability criteria are exercised and recorded.

## GPT Recovery integration repetition — 2026-10-01

The M28 runner now includes `./nagi m27` after the Search and grouped-Undo
gates in every repetition. It requires the CLI's `PASS M27 A/B and Recovery:`
result and its self-test rejects missing/failure output. The dry-run reports
the three-gate sequence and the collision guard now includes the M22 bootstrap
log as well as the guest boot logs, images, and OVMF variables.

Before the real run, the prior M19/M22 fixed-name outputs and both current
User Data disks were preserved in
`out/evidence/pre-m28-m27-gpt-repeat-20261001/`. Then
`NAGI_M28_REPEAT_COUNT=1 sh tests/acceptance/m28_integration_stress.sh --run`
passed M19 live VFS/Search/semantic-index persistence, M22 three-boot grouped
Undo and Activity Ledger restore, and M27 GPT A/B rollback/Recovery/healthy-B
promotion. The available M19/M22 generated artifacts and serial logs are
copied with verified hashes under
`out/evidence/m28-gpt-recovery-integration-20261001/`; M27's full QEMU logs
remain in `out/evidence/m27-ab-rollback-1790783216496413000/`.

The runner self-test, dry-run, and shell syntax check pass. M28 remains PARTIAL:
this integrated repetition does not include its desktop, multi-tab browser,
real Granite, audio-pressure, OOM, fairness, or leak-soak workload.

## Two-repetition GPT Recovery attempt — 2026-10-01

Before the run, fixed-name M19/M22 images, OVMF variables, serial logs, and
both persistent User Data disks were preserved and hash-verified under
`out/evidence/pre-m28-three-gate-repeat-20261001/`.
`NAGI_M28_REPEAT_COUNT=2 ... --run` completed all three gates in repetition
1: M19 Search/ObjectId persistence, M22 three-boot grouped Undo/Activity
Ledger, and M27 GPT rollback/Recovery/healthy-B promotion. Repetition 2 passed
M19, then its M22 boot 1 timed out after 90 seconds. That serial log contains
only the 87-byte UEFI screen-clear sequence; no kernel marker was printed.
The failed inputs and log are preserved under
`out/evidence/m28-repetition-2-m22-timeout-20261001/`. The ESP image and
User Data disk matched repetition 1's accepted files byte-for-byte.

Two disposable QMP replays from copies of those M22 inputs reached the guest
kernel within seconds, including with the same CoreAudio/virtio-sound QEMU
configuration. The subsequent standalone `./nagi m22` rerun passed all three
boots. Repetition 2 then passed the M27 healthy-B trial boot, but its promotion
boot timed out before BDS output. A QMP replay using the saved OVMF state
consumed the readiness record, confirmed B, and reached M10 desktop READY.
The full `./nagi m27` retry later passed its earlier stages and timed out at
Recovery boot before BDS output; both full-run records are documented in the
M27 workstream. Diagnostic screenshots, serial logs, copied disks, OVMF
variables, and SHA-256 manifests are retained with those evidence directories.

The repeated evidence points to intermittent local QEMU/OVMF startup stalls
before guest code, but does not establish their root cause. Therefore this
two-repetition harness attempt did not pass and is not counted as one. The
single complete integrated repetition remains valid evidence. M28 stays
PARTIAL until the full combined workload and stability criteria are measured.
