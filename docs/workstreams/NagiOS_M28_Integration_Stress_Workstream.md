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
keeps a run from overwriting existing acceptance evidence. Every `--run`
reserves a unique `out/evidence/m28-run-<UTC timestamp>-<PID>/` namespace.
Between successful repetitions, the harness moves generated images, vars, and
logs to that run's `repetition-N/` subdirectory and snapshots both persistent
disks before starting the next gate. A later invocation never reuses an older
run namespace. If a prior M19 snapshot already exists, its guest
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

## QEMU timeout diagnostics and fresh three-gate repetition — 2026-10-01

A fresh one-repetition run first passed M19 and M22, then the M27 Recovery
launch with a pending System B trial timed out after 90 seconds. That GUI/VNC
run ended without a guest acceptance marker; its evidence is preserved under
`out/evidence/m27-ab-rollback-1790787806085590000/`. The fixed-name M19/M22
outputs, OVMF variables, logs, and both User Data disks were preserved and
hash-verified before the next attempt under
`out/evidence/m28-qmp-diagnostics-pre-run-20261001/`.

The QEMU marker waiters now attempt to collect `query-status` and CPU-register
output once when a headless or GUI/VNC run times out. Both requests share a
three-second total time budget, each QMP response line is capped at 64 KiB,
and the response, query error, or skipped request is appended to the guest
serial log. The timeout still fails acceptance. Host tests cover both queries,
bounded lines, and serial-log append behavior. The change did not alter marker
acceptance.

The next fresh one-repetition run passed all three gates: M19 live VFS/Search
and semantic-index persistence, M22 three-boot grouped Undo/Activity Ledger,
and M27 GPT A/B rollback, Recovery, and healthy-B promotion. Full QEMU
evidence is under `out/evidence/m27-ab-rollback-1790788040114277000/`; the
M19/M22 images, vars, data disks, and logs were copied with verified hashes to
`out/evidence/m28-qmp-diagnostics-pass-20261001/` and remain in `out/logs/` and
`out/artifacts/`. The prior timed-out attempt is retained as a failure and not
counted as a pass. The cause is unconfirmed, and the new
timeout query path was not exercised by the successful rerun. M28 remains
PARTIAL because this three-gate repetition did not measure its Desktop,
multi-tab browser, real Granite, audio-pressure, OOM, CPU-fairness, or leak
soak workload. Before the rerun, its fixed-name inputs were preserved and
hash-verified under
`out/evidence/m28-qmp-diagnostics-pre-rerun-20261001/`.

## Unique repetition evidence and M27 SMP timeout — 2026-10-01

The runner now reserves a unique UTC timestamp/PID archive root for every
`--run`; successful intermediate repetitions are retained below
`repetition-N/`. This removes the fixed `m28-repetition-N/` collision that
prevented another run after earlier evidence had been preserved. The namespace
self-test, `sh -n`, and a two-repetition dry-run passed. The dry-run remained a
preflight and did not build or boot a guest.

Before the real two-repetition attempt, the fixed-name M19/M22 outputs and
both persistent disks were preserved under
`out/evidence/pre-m28-repeat-20261001-0a331f6/` with verified hashes. M19 and
M22 passed in repetition 1; the M27 boot 4 acceptance timed out after rollback
to confirmed A. Its serial log reached three M3 AP-online markers but stopped
before the scheduler workload marker. QMP reported shutdown and CPU#0 RIP
`0x4005d90`, which maps to `nagi_kernel::smp::thread_entry`. That narrows the
area for diagnosis but does not identify the fault. M27 and M28 do not receive
a pass from this attempt.

M27's input images, OVMF variables, guest data, serial logs, and QMP details
are preserved with verified hashes under
`out/evidence/m27-ab-rollback-1790796067483623000/`. The M19/M22 partial
outputs from repetition 1 and the runner log are preserved under
`out/evidence/m28-run-failure-20261001-0a331f6/`. The next useful experiment is
bounded per-CPU timer/workload progress around M3 startup, followed by a replay
from the saved rollback inputs before changing scheduler behavior. M28's
combined Desktop/Files/Notes/Albert, real Granite, audio, OOM, fairness, and
leak workload remains unmeasured.

The standalone `./nagi m27` acceptance sequence was then rerun with fresh
inputs and OVMF state and passed at
`out/evidence/m27-ab-rollback-1790800250176976000/`; the earlier M3 timeout
did not recur. This does not convert the failed integrated M28 repetition into
a pass, and it does not establish the timeout's root cause. The next integrated
M28 run should still preserve its inputs and exercise both repetitions.

## Two-repetition reruns and repeated firmware-start timeout — 2026-10-01

Two fresh `NAGI_M28_REPEAT_COUNT=2 tests/acceptance/m28_integration_stress.sh
--run` attempts each completed repetition 1 across M19 ObjectId/Search,
three-boot M22 grouped Undo/Activity Ledger, and M27 GPT A/B/Recovery. In both
attempts, repetition 2 passed M19 and then timed out during M22 boot 1 before
`Nagi Kernel started`. QMP reported `status=running` and CPU#0 RIP
`0x7eb84171`, with the same register/stack layout on both failures; serial
output stopped after the 87-byte UEFI screen-clear sequence. This is outside
the Nagi kernel's linked address range and indicates a repeatable firmware
startup failure, but the precise OVMF instruction and root cause have not been
established.

The M22 boot image hash is identical across successful and failed repetitions,
and the M22 User Data hash after each timeout matches that attempt's successful
repetition-1 snapshot. The guest therefore did not reach the M22 acceptance
path or change that disk on either failed boot. Both run archives, including
their partial M19/M22 outputs and disk snapshots, have verified manifests at
`out/evidence/m28-run-20260930T203759Z-48306/` and
`out/evidence/m28-run-20260930T204402Z-48900/`. The fixed-name M19/M22 outputs
were moved into those archives before retrying, and the pre-run persistent
disks are preserved at
`out/evidence/pre-m28-repeat-20261001-m27-replay/`.

Neither of those two attempts completed repetition 2. The combined
Desktop/Files/Notes/Albert, real Granite, audio-pressure, OOM, CPU-fairness,
and leak-soak workload also remained unmeasured; the prior M27 boot-4 failure
remained unexplained despite three subsequent full M27 passes at that point.

## Two-repetition M19/M22/M27 gate passed — 2026-10-01

A third `NAGI_M28_REPEAT_COUNT=2 tests/acceptance/m28_integration_stress.sh
--run` completed both repetitions. Each repetition passed M19 ObjectId/Search,
all three M22 NH16 grouped Undo/Activity Ledger boots, and the full M27 GPT
A/B/Recovery acceptance. The artifacts for both repetitions, including the
fixed-name outputs from repetition 2, are archived and hash-verified at
`out/evidence/m28-run-20260930T211250Z-51667/`; all 22 files verify against
its `SHA256SUMS`. The M27 sub-runs at
`out/evidence/m27-ab-rollback-1790802782930760000/` and
`out/evidence/m27-ab-rollback-1790802894479757000/` each have a verified
31-file manifest.

The two earlier repetition-2 M22 boot-1 firmware timeouts did not recur; the
reason for those hangs is still unknown. Timeout diagnostics now request a
bounded 12-instruction window at `$rip` in addition to QMP status and registers.
A QMP fixture test and live HMP smoke passed, recorded at
`qmp-instruction-smoke.log`; this run did not trigger timeout diagnostics.

This closes the runner's two-repetition M19/M22/M27 gate, not the formal full
M28 workload. Desktop/Files/Notes/Albert with concurrent tabs, real Granite
load/unload and CPU fairness, audio pressure, OOM, and memory/handle leak soak
remain unmeasured. M28 remains `PARTIAL`.

## Fresh two-repetition M19/M22/M27 gate — 2026-10-01

Before the run, the fixed-name M19/M22 artifacts, OVMF variables, serial logs,
and both starting User Data disks were preserved under
`out/evidence/pre-m28-m29-persistence-20261001T012622Z/`; its README and
10-entry SHA-256 manifest verify. The collision-free dry-run, shell syntax,
and self-test passed before starting QEMU.

`NAGI_M28_REPEAT_COUNT=2 tests/acceptance/m28_integration_stress.sh --run`
then completed both repetitions. Each passed the real M19 VFS/ObjectId/Search
restart gate, all three M22 NH16 grouped-Undo and NAL1 Activity Ledger boots,
and the full M27 GPT A/B/Recovery gate. The run archive is
`out/evidence/m28-run-20261001T012645Z-75535/`; all 20 files in its
`SHA256SUMS` verify. M27 acceptance runs
`out/evidence/m27-ab-rollback-1790818018301818000/` and
`out/evidence/m27-ab-rollback-1790818130409530000/` each have a verified
37-entry manifest.

The prior intermittent pre-guest OVMF/QEMU startup timeouts were not observed
in these two repetitions; their cause remains unknown. QEMU reported no host
`virtio-sound.in` input driver, and these gates do not measure audio. The
Desktop/Files/Notes/Albert combined load, real Granite load/unload, audio
pressure, OOM, CPU fairness, and memory/handle leak soak remain unmeasured.
This advances the repeated integration slice only; M28 remains `PARTIAL`.

## Completion Sweep repeated gate and evidence finalization — 2026-10-01

The harness now parses the run-stamped M22 boot-3 path reported by `./nagi
m22`, validates that exact log, and archives each repetition's M19/M22 images,
OVMF variables, serial logs, and User Data snapshot, including the final
repetition. It writes a run README and SHA-256 manifest after successful or
failed gates; a failed M27 run also gets its own README and manifest. Failure
cleanup preserves the active repetition, and run-ID extraction handles M22
bootstrap and numbered-boot timeout diagnostics. The self-test covers those
paths, including the newline-terminated M27 evidence list that previously
interrupted README generation.

`NAGI_M28_REPEAT_COUNT=2 ... --run` completed both M19 Search/ObjectId, M22
three-boot NH16/NAL1 grouped Undo, and M27 GPT A/B/Recovery gates. Its complete
archive is `out/evidence/m28-run-20261001T130826Z-2523/`; all 28 manifest
entries verify. M27's two sub-run manifests at
`out/evidence/m27-ab-rollback-1790860121579655000/` and
`out/evidence/m27-ab-rollback-1790860238754593000/` each verify all 31 files.
A further one-repetition run passed all three QEMU gates at
`out/evidence/m28-run-20261001T132819Z-4650/`. Its initial shell exit was
caused by a README-generation `set -e` edge case; the archived README and
manifest were corrected and verify all 15 files.

Subsequent attempts exposed intermittent OVMF startup loops at RIP
`0x7eb84171`, both during M27 Recovery and M22 bootstrap/boot 1. They are
preserved with QMP diagnostics and are not acceptance passes:
`out/evidence/m28-run-20261001T131752Z-3830/`,
`out/evidence/m28-run-20261001T133248Z-5339/`, and
`out/evidence/m28-run-20261001T133549Z-5668/`. Their M19/M22 artifacts and
serial logs have verified SHA-256 manifests. The observed instruction loop is
in OVMF, before the affected guest's acceptance path; its root cause remains
unconfirmed.

The corrected runner passes `sh -n`, `--self-test`, and `--dry-run`. Its
two-repetition M19/M22/M27 gate passes on the recorded complete run, but formal
M28 remains `PARTIAL`: Desktop/Files/Notes/Albert concurrency, real Granite,
audio pressure, OOM, CPU fairness, and handle/memory leak soak remain
unmeasured.

## Completion Sweep: require Search activity marker — 2026-10-01

The M28 M22-log validator and its self-test now require
`Nagi M22 file.search Activity Ledger PASS`, proving the completed M22 logs
reopened the M19 Search record alongside NH16/NAL1 Move/Copy Undo. The
three-boot `./nagi m22` gate passed with the marker on each boot, and the M28
shell syntax, self-test, and dry-run pass. The combined two-repetition M28
QEMU gate has not yet been rerun after this M19-to-NAL1 addition; the earlier
recorded two-repetition run remains valid for its earlier source state only.
The dry-run reports the archived fixed-name M19 log as unverified; `--run`
creates a new M19 log before testing it. Formal M28 remains `PARTIAL`.

## Completion Sweep follow-up: current two-repetition result — 2026-10-01

The two-repetition gate was rerun after the M19 Search-to-NAL1 change at
`out/evidence/m28-run-20261001T141428Z-9953/`. Repetition 1 passed M19 Search,
the three-boot M22 Search/Move/Copy ledger and grouped Undo, and M27 A/B
rollback, promotion, Recovery, and file.move Undo. Repetition 2 passed M19,
then M22 bootstrap timed out after 90 seconds before guest acceptance. QMP
reported QEMU still running at RIP `0x7eb84171`; the failure log and all
repetition artifacts are in the SHA-256-verified archive, and the M27 sub-run
manifest also verifies.

A preceding run,
`out/evidence/m28-run-20261001T140924Z-9410/`, timed out on M22 boot 3 at the
same RIP after M22 boots 1 and 2 passed. Its final boot diagnostic is now
included in that archive's verified manifest. A new standalone `./nagi m22`
run passed all three boots after the timeout, confirming the new Search record
and undo fixture still pass independently. The M28 harness now archives the
run-ID-specific final boot log on failure, and its self-test checks that path.
Shell syntax, self-test, and dry-run pass. The two-consecutive-repetition
acceptance remains unfulfilled, and formal M28 remains `PARTIAL`.

## Current two-repetition attempts and standalone regressions — 2026-10-02

The harness self-test, shell syntax, and two-repetition dry-run passed. Two
fresh `NAGI_M28_REPEAT_COUNT=2 ... --run` attempts were then made. In
`out/evidence/m28-run-20261001T153942Z-38390/`, repetition 1 passed M19 and
M22 boot 1; M22 boot 2 timed out after 90 seconds at RIP `0x7eb84171` before a
guest marker. Its 13-entry archive manifest verifies. In
`out/evidence/m28-run-20261001T154452Z-38953/`, repetition 1 passed M19, then
M22 bootstrap timed out at the same RIP before guest acceptance; its 11-entry
manifest verifies. These are firmware startup failures, not M22 acceptance
passes, and neither run completed a repetition or reached M27.

Fresh standalone checks after those failures passed `./nagi m22` across all
three boots and `./nagi m27` A/B/Recovery. M22 evidence is at
`out/evidence/m22-standalone-1790869348631156000/`; M27 evidence is at
`out/evidence/m27-ab-rollback-1790869370028356000/`. These isolated passes
confirm the fixture paths still work but do not satisfy the consecutive
combined M28 gate. M28 remains `PARTIAL`; Desktop/Files/Notes/Albert
concurrency, real Granite, audio pressure, OOM, CPU fairness, and memory/handle
leak soak remain unmeasured.

## Completion Sweep — M27 failure evidence capture and replay (2026-10-02)

The M28 M27-failure parser now extracts the M27 run ID from absolute or
relative QEMU timeout diagnostics and reconstructs the repository-relative
evidence path. The self-test covers both forms. This fixes a real artifact gap:
the failed M27 run was previously absent from the M28 README and had no local
README or checksum even though QEMU preserved its log. Shell syntax, self-test,
and dry-run pass.

Fresh run `out/evidence/m28-run-20261001T205917Z-97811/` passed M19 and the
three-boot M22 gate in both repetitions; repetition 1 also passed the M27 gate.
Repetition 2's M27 Recovery GUI timed out before guest output at OVMF RIP
`0x7eb84171`. The fixed harness recorded the failed sub-run and its own
SHA-256 manifest. Replaying copies of its exact Recovery image, User Data, and
OVMF variables reached the Recovery menu and command console and passed
`Nagi M27 Recovery command help PASS`. This does not retroactively pass the
original repetition. The archive and both M27 sub-run manifests verify.

The prior run `out/evidence/m28-run-20261001T204558Z-96383/` passed repetition
1 and timed out before the M13 marker in repetition 2's M27 Recovery Undo
fixture. A replay from that exact boot image and User Data with the pinned
OVMF template passed the M13/M21/M22 markers. Both integrated runs remain
`PARTIAL`; a two-consecutive-repetition pass has not been recorded. Formal
Desktop/Files/Notes/Albert load, real Granite inference, audio pressure, OOM,
CPU fairness, and leak soak remain unmeasured.

## Completion Sweep — second M27 boot-5 timeout and diagnostic replay (2026-10-02)

Fresh two-repetition run
`out/evidence/m28-run-20261001T211548Z-133/` completed repetition 1 across
M19 Search, the three-boot M22 transaction/Undo gate, and M27 A/B/Recovery.
Repetition 2 passed M19 and M22, then its M27 run
`1790889480617050000` timed out on boot 5 before guest output. QMP recorded
RIP `0x7eb84171` in the OVMF loop `jmp 0x7eb84150`. The M28 archive and failed
M27 sub-run have verified manifests.

A diagnostic replay used the saved A/B boot image and User Data, plus the
post-failure OVMF variables. The variable file was not captured before boot 5,
so this was not an exact-state replay. The replay reached `confirmed slot=A`
and `Nagi M7 acceptance PASS`; it does not retroactively pass repetition 2.
Evidence is under
`out/evidence/m27-replay-system-a-1790889480617050000/`. M28 remains `PARTIAL`:
there is still no two-consecutive-repetition pass, and the formal desktop,
model, resource pressure, fairness, and leak-soak criteria remain unmeasured.

## Completion Sweep — two current consecutive repetitions (2026-10-02)

The current branch was rerun after preserving the fixed-name M19 outputs and
both persistent input disks at
`out/evidence/pre-m28-m25-whisper-20261002/`. The archive
`out/evidence/m28-run-20261001T230607Z-12562/` completed two consecutive
repetitions. Each passed M19 VFS/ObjectId/Search, all three M22 Move/Copy,
Activity Ledger, grouped Undo and restart boots, and M27's malformed System B
rollback, healthy System B readiness/promotion, and Recovery Undo. Its
SHA256SUMS verifies all archived M19/M22 images, variables, disks, and logs;
the M27 sub-run manifests also verify at
`out/evidence/m27-ab-rollback-1790895983578341000/` and
`out/evidence/m27-ab-rollback-1790896104678487000/`.

This passes the harness's repeated Search/History/Recovery slice at source
`4d5dd02`; it does not pass the formal combined M28 workload. The host still
has no virtio-sound input driver, and no Desktop/Files/Notes/Albert concurrent
load, real Granite inference, audio-pressure, OOM, CPU-fairness, or leak-soak
acceptance was measured. The earlier OVMF startup loop remains unexplained.
M28 remains `PARTIAL`.

## Completion Sweep one-repetition run after M26 artifact acceptance — 2026-10-02

Before running, existing M19/M22 fixed-name outputs and both persistent User
Data disks were copied, byte-checked, and hash-preserved under
`out/evidence/m28-pre-m26-qwen-20261001T235658Z/`. The guarded one-repetition
runner then passed M19 live Search/ObjectId persistence and the M22 three-boot
Move/Copy, NH16/NAL1, and grouped-Undo restart gate. M27 timed out on rollback
boot 4 after three AP-online markers but before its scheduler-start marker;
QMP reported shutdown and RIP `0x40060c0` in `smp::thread_entry`. The complete
M28 archive is `out/evidence/m28-run-20261001T235755Z-18930/`; the M27 failure
is preserved under `out/evidence/m27-ab-rollback-1790899090521806000/`. Both
manifests verify. This repetition failed M27 and is not a complete M28 pass.

The follow-up M27 run, after moving the scheduler-start marker before `sti`,
reached M3 scheduler completion on its rollback and Recovery boots, then
encountered a separate pre-guest OVMF loop on the healthy-B readiness trial.
It is preserved at `out/evidence/m27-ab-rollback-1790899532394169000/`. The
one-repetition Search/Undo integration passed; Recovery integration and full
M28 reference-load acceptance remain incomplete, so M28 stays PARTIAL.

## Completion Sweep — two current Search/History/Recovery repetitions (2026-10-02)

Before the run, both persistent M19/M22 User Data disks and the latest verified
M22 third-boot log were preserved with a SHA-256 manifest under
`out/evidence/m28-pre-two-repetition-20261002T002320Z/`. The latest M19 log was
absent at preflight, so it was treated as unverified; both repetitions then
generated and passed fresh M19 logs.

`NAGI_M28_REPEAT_COUNT=2 sh tests/acceptance/m28_integration_stress.sh --run`
passed two consecutive repetitions at source
`f718117009c23cd0b3ecbb0f328277fe7f608200`. Each repetition passed M19
VFS/ObjectId/Search, all three M22 Move/Copy/NH16/NAL1 grouped-Undo restart
boots, and M27 malformed-System-B rollback, healthy-System-B readiness and
promotion, and Recovery journal/Undo checks. The complete archive is
`out/evidence/m28-run-20261002T002354Z-22197/`; its manifest, both M27
sub-run manifests, and the pre-run snapshot manifest verify. The harness
self-test, two-repetition dry-run, and shell syntax check also passed.

QEMU reported that the host has no `virtio-sound.in` input driver; this run did
not exercise audio. Desktop/Files/Notes/Albert concurrent use, real Granite
inference, memory pressure, CPU fairness, and handle/memory leak soak remain
unmeasured. This passes the repeated integration slice only; M28 stays
`PARTIAL`.

## Completion Sweep — fresh two-repetition run (2026-10-03)

Before the run, the harness's collision guard found three existing generated
M19 outputs: its raw image, OVMF variables, and initial serial log. They were
moved intact to `out/evidence/pre-m28-repeat-20261002T224528Z-22372/`; all
three hashes verify. The source revision for the run is
`78b7655efe72e73340e699fae3509cb48a68f8c8`.

After shell syntax, self-test, and two-repetition dry-run checks, the real
`NAGI_M28_REPEAT_COUNT=2
./tests/acceptance/m28_integration_stress.sh --run` passed both consecutive
repetitions. Each passed fresh M19 VFS/ObjectId/Search, the three-boot M22
Move/Copy/NH16/NAL1 grouped-Undo gate, and M27's malformed-System-B rollback,
healthy-System-B readiness/promotion, and Recovery journal/Undo gate. The
complete archive is `out/evidence/m28-run-20261002T224535Z-22420/`; its
manifest, both M27 sub-run manifests, and the preserved pre-run manifest
verify. All four run-stamped GPT images pass `qemu-img check`.

This run did not reproduce the earlier OVMF startup loops at RIP `0x7eb84171`,
but their cause remains unknown. QEMU still reports that the host lacks
`virtio-sound.in`; this gate does not exercise audio. Desktop/Files/Notes/Albert
concurrent load, real Granite inference, audio pressure, OOM, CPU fairness, and
leak soak remain unmeasured. M28 remains `PARTIAL`.

## Completion Sweep — bounded scheduler fairness regression (2026-10-03)

The M3 preemptive scheduler self-test now runs two busy kernel tasks per CPU
for 32 work steps and counts timer dispatches. A CPU passes only when both
tasks ran and their dispatch counts differ by at most one; successful M19/M22
acceptance logs include `Nagi M3 CPU scheduler fairness PASS`. The M28 harness
requires that marker. A host regression also runs 131,072 cooperative yields
with all 64 bootstrap slots runnable and checks for starvation or excess skew.
The focused kernel scheduler suite passed 13 tests; callback adapter tests
passed 3 tests. The M28 shell syntax check, harness self-test (including
rejection of a missing fairness marker), and two-repetition dry-run passed.

The current-source one-repetition QEMU run
`out/evidence/m28-run-20261003T004328Z-37332/` passed M19, the three-boot M22
gate, and M27 A/B rollback, promotion, and Recovery. Its M27 sub-run is
`out/evidence/m27-ab-rollback-1790988224518631000/`; the archive and sub-run
manifests verify, and both M27 GPT images pass `qemu-img check`.

Two additional two-repetition attempts did not complete: run
`out/evidence/m28-run-20261003T002909Z-34387/` passed repetition 1, then
repetition 2's M19 boot timed out before guest output; an immediate standalone
M19 retry passed. Run `out/evidence/m28-run-20261003T003659Z-36156/` passed
repetition 1 M19 and M22, then its first M27 System B boot timed out before
guest acceptance. QMP reported the recurring OVMF loop at RIP `0x7eb84171`.
These attempts remain incomplete and are not counted as passes. CPU fairness
during real Granite inference and the broader Desktop/Files/Notes/Albert,
audio, OOM, and leak-soak workload remain unmeasured. M28 remains `PARTIAL`.

## Completion Sweep — repeated M27 startup timeout follow-up (2026-10-03)

Current-source run `out/evidence/m28-run-20261003T011836Z-42073/` passed the
full M19/M22/M27 sequence in repetition 1. Repetition 2 passed M19 and all
three M22 boots, then M27 timed out on boot 5 while checking the persisted
System A confirmation decision at the existing 90-second limit. The failed
run and its M27 sub-run evidence remain hash-verified; this attempt is not a
two-repetition pass.

The repeated-journal decision boot timeout is now 180 seconds. The guest
acceptance markers and boot decision checks are unchanged. A fresh standalone
`./nagi m27` run passed at
`out/evidence/m27-ab-rollback-1790991014827320000/`; its logs, source diff,
and seven run-stamped images have a SHA-256 manifest. Both GPT images pass
`qemu-img check`, and the Recovery log includes the M3 CPU scheduler fairness
marker. A new two-repetition integration run is still required. M28 remains
`PARTIAL` because its Desktop/Files/Notes/Albert, real Granite, audio-pressure,
OOM, and leak-soak workload is unmeasured.

## Completion Sweep — pre-guest firmware timeout follow-up (2026-10-03)

Run `out/evidence/m28-run-20261003T013706Z-44453/` ended in repetition 1
when M19 reached its 90-second timeout before guest acceptance. The serial log
contains the UEFI screen-clear sequence only; QMP reported `status=running` at
RIP `0x7eb84171`, matching the earlier pre-guest firmware stalls. The failed
attempt is preserved with its SHA-256 manifest and is not counted as a pass.

M19's initial/restart boots and M22's bootstrap/three history boots now allow
180 seconds to reach the existing guest markers. The two-repetition QEMU gate
will be rerun at this source state. This timeout adjustment does not measure
the formal Desktop/Files/Notes/Albert, real Granite, audio, OOM, or leak-soak
workload; M28 remains `PARTIAL`.

Run `out/evidence/m28-run-20261003T014330Z-45609/` then passed M19 and all
three M22 boots in repetition 1, but M27 readiness-promotion boot 3 timed out
at 90 seconds while waiting for confirmed System B plus `Nagi M10 desktop
READY`. QMP remained running at RIP `0x7eb84171`; the sub-run and parent
manifests verify. The three readiness-promotion boots now have a 180-second
budget with the same decision and desktop markers. The integrated
two-repetition gate is still outstanding.

## Completion Sweep — bounded M22 pre-guest firmware retry (2026-10-03)

Run `out/evidence/m28-run-20261003T020520Z-47383/` completed repetition 1
across M19, M22's three boots, and M27. Repetition 2 passed M19, then M22's
bootstrap timed out after 180 seconds before `Nagi Kernel started`. Its serial
stream had only the 87-byte UEFI screen-clear prefix before QMP diagnostics;
QMP reported `status=running` at RIP `0x7eb84171`. The parent and M27
sub-run manifests verify. Only one repetition completed, so the run is not a
two-repetition pass.

The M22 bootstrap now performs one bounded retry only for this pre-guest
timeout shape: the QEMU timeout is present, the guest kernel-start marker is
absent, and QMP captured a running status plus CPU registers and the
instruction window. The failed serial log and OVMF variables are preserved;
the retry starts with a fresh copy of the configured OVMF variables template.
No retry occurs after the guest kernel-start marker, and M22 acceptance
markers are unchanged. The harness includes both retry artifacts in its
per-repetition archive. Classifier tests, top-level format/test/lint/build,
shell syntax, and the harness self-test pass. A standalone M22 QEMU run
`1790993965845089000` passed all three boots but did not trigger the retry.
A new two-repetition run is still needed; M28 remains `PARTIAL` for this
repeated gate and its unmeasured combined Desktop/Files/Notes/Albert, real
Granite, audio, OOM, and leak-soak workload.

## Completion Sweep — current-source M27 firmware retry (2026-10-03)

Standalone M27 run `out/evidence/m27-ab-rollback-1790994710275400000/` passed
the A/B and Recovery acceptance after one narrowly classified pre-guest
firmware timeout on boot 5. The first attempt retained the same QMP running
state and OVMF instruction-loop signature seen in earlier failures. One retry
reused the same OVMF variables, reached the original guest marker, and
preserved the failed-attempt serial and variables in evidence sidecars. This
does not resolve the firmware loop's root cause. A new two-repetition M28 run
with the current retry changes remains necessary; the reference combined-load
and resource-pressure workload is still unmeasured.

## Completion Sweep — retry journal-state correction (2026-10-03)

Run `out/evidence/m28-run-20261003T024610Z-52844/` passed the M19 and M22
sub-gates, then exited with status 130 after the harness printed its M27 start
line but before it recorded an M27 result. The run was deliberately
interrupted after review found that the previous M27 retry could consume a
second persistent trial count if `begin_boot()` completed before the guest
kernel-start marker. The archive preserves the M19/M22 outputs and its SHA-256
manifest verifies; zero complete repetitions passed, so it is not an M28
acceptance. The unarchived M27 directory from that interrupted invocation is
unverified and not counted.

M27 retry now saves the exact pre-attempt OVMF variables, preserves the
post-failure state separately, checks for evidence collisions before the
initial boot, and restores the saved state before retry. Corrected standalone
M27 run `out/evidence/m27-ab-rollback-1790995870248251000/` passed, but did not
trigger the retry. M28's current-source two-repetition gate and its formal
combined-load/resource-pressure measurements remain outstanding.
