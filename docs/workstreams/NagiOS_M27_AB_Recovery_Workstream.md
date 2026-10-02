# Nagi OS M27 — A/B and Recovery Workstream

## Current state: PARTIAL

The UEFI loader now persists its A/B decision in two Nagi-namespaced firmware
variables and, in the feature-scoped acceptance image, loads matched kernel and
init files from the selected System A or System B directory. QEMU acceptance
covers three malformed System B trials and rollback to A, plus a healthy B
trial whose guest readiness record is consumed before B is confirmed. A third
launch retains confirmed B, and all relevant boots read the same persistent
user-data disk. The reference release image now has separate GPT System A/B
and Recovery partitions, and its production loader applies the M27 journal
policy without staging a synthetic update on an empty journal. The completion
sweep also added GPT acceptance images using the same reference layout:
malformed System B is tried three times, Recovery preserves the pending
journal, then System A rolls back; a healthy System B trial persists readiness,
survives Recovery, and is later confirmed. The acceptance-only empty-journal
seed is not part of the release loader. The current readiness gate is
successful M10 desktop surface presentation after M6/M7 checks; Nagi 0.1 has
no account login flow. Authenticated slot manifests and an authenticated GPT
update installer remain incomplete, so M27 remains PARTIAL.

## Read-only VFS integrity-check slice

`libnagi::storage::Vfs::check_existing` inspects the existing Nagi VFS through
the separate `ReadOnlyBlockDevice` interface. It validates the fixed
superblock and group-descriptor geometry, reserved and allocated bitmap bits,
free counts, inode/block ownership, the reachable directory tree, directory
records, and `.` / `..` links. It reports counts or `Corrupt`; it has no
formatting, repair, write, or flush path and must run while the volume is
quiescent. This is a bounded checker for Nagi's current custom ext2-like
layout, not a general ext2 `fsck` implementation.

The `m27-ro-vfs-check` init feature runs this check before `mount_or_format`
on the paired-slot QEMU fixture's persistent user-data disk. The refreshed
`./nagi m27` acceptance passed: rollback boot 4, confirmed boot 5, the healthy
trial, promotion, and stable confirmed-B boots each reported
`Nagi M27 read-only VFS check PASS` before the normal M7 mount and
persistent-data read. Host tests prove valid nested volumes pass, malformed
superblocks, accounting, directory records, inode bitmap bits, and orphan
parent chains are rejected, and image bytes plus write/flush counters remain
unchanged by successful and failed checks. This does not yet provide a
general repair tool; Recovery now reuses the checker as described below.

## Recovery boot and persistent Undo slice

The feature-scoped UEFI menu offers System A, System B, and Recovery for three
seconds, then preserves the prior automatic A/B policy. Recovery uses separate
kernel and init payloads and remains bootable when both system-slot kernels in
the acceptance image are intentionally malformed. Selecting Recovery bypasses
`stage_update`, `begin_boot`, and `mark_boot_success`; the following automatic
boot proves the pending System B trial is still at attempt 1.

The Recovery init runs `Vfs::check_existing` before `mount_existing`, which
does not format an unknown volume. Its bounded serial console provides
`help`, `check`, `log`, `files`, `slots`, and explicit `undo` commands. Undo
loads the existing two-copy NH16 archive, writes `UndoPending` before applying
the inverse group, flushes the volume, and then writes `Undone`. The log command
shows only the current kernel's bounded in-memory ring; corrupt volumes stay
unmounted and are not repaired automatically.

Fresh `./nagi m27` QEMU acceptance creates a real three-file M21/M22
`file.move` transaction in the guest and records NH16/NAL1 Committed state. It
then boots Recovery from an image with invalid System A and B kernels and
issues the explicit Undo command. A subsequent guest restart verifies all
files returned to their original names and the Activity Ledger records
Undone. The Recovery boot journal remained unchanged, and the next automatic
boot began System B trial attempt 1. Logs and the shared user-data disk are
preserved in
`out/evidence/m27-ab-rollback-1790740066889678000/`.

The fixture also exposed an assumption that the checked user volume had exactly
one file and one directory. The M27 checker now accepts additional valid user
data while the M7 acceptance still verifies its persistent marker byte for
byte. Recovery remains a bounded console, not a polished repair environment;
historical boot-log retrieval, filesystem repair, authenticated update
manifests, and updater installation remain open. GPT release-layout slot
selection and Recovery behavior are covered by the integration acceptance
below.

The completion-sweep rerun after the init entry-point and CLI request refactors
passed on 2026-09-30. It repeated three malformed-B rollbacks, healthy-B
readiness promotion, Recovery with both system kernels invalid, Recovery Undo
of the committed guest transaction, restart verification, and the untouched
trial-1/B default boot. Evidence is in
`out/evidence/m27-ab-rollback-1790740066889678000/`.

## Conflict-safe and restartable Recovery Undo

`undo_latest` now checks every inverse action against the current guest files
before it serializes `UndoPending` or applies any mutation. Delete and restore
actions compare file bytes with their recorded snapshots. Edit actions accept
only the recorded forward or inverse bytes; the forward snapshot is borrowed
from NH16 by record sequence and is not copied into the undo batch. New NH16
Move records store a SHA-256 digest of the source file in the existing `before`
snapshot field. MoveBack checks that exactly one of the source and destination
paths exists and that the present file matches this digest. The NH16 version
and serialized layout are unchanged. Older Move records with an empty `before`
snapshot retain the legacy path-only check.

If any preflight action conflicts, Recovery returns without changing a file
or persisting `UndoPending`. On retry after a process restart, an existing
`UndoPending` transaction takes priority over later `Committed` transactions.
The normal inverse actions accept their already-inverted file state, allowing
Recovery to finish a partially applied Undo idempotently.

The M27-only `m27-recovery-undo-acceptance` init feature exposes a hidden
fixture command; production Recovery images keep the ordinary
`m27-recovery` feature. The QEMU fixture conflicts the last action in a
three-file MoveBack batch and verifies that earlier actions remain forward
and the persisted transaction remains `Committed`. It then persists
`UndoPending`, applies the first inverse, and invokes Recovery Undo again as a
simulated restart. The second pass completes and persists `Undone`. Fresh
`./nagi m27` evidence with both markers is in
`out/evidence/m27-ab-rollback-1790857169644407000/`.

The 2026-10-02 Completion Sweep adds a same-path replacement-content conflict
to the Recovery fixture. It mutates a moved file without changing its name,
verifies that preflight reports only that action as `Conflict` while the
transaction remains `Committed`, restores the file, then continues the
existing name-conflict and restartable Undo checks. The host `nagi-history`
tests cover digest persistence, tampered-content rejection, exact digest
length, and recovery of older zero-length records. `./nagi m22` passes all
three guest boots with digest-bearing Move records. The fresh `./nagi m27`
attempt printed the new content-conflict marker at
`out/evidence/m27-ab-rollback-1790881641572651000/recovery-boot.log`, then
timed out before guest output on a later Recovery boot with pending System B;
QMP again reported the OVMF loop at RIP `0x7eb84171` in
`recovery-preserved-trial-journal.log`. Thus the new subtest passed, but this
full rerun is not an M27 acceptance pass.

## Implemented

- The confirmed slot is retained while the other slot is staged as pending.
- A candidate boot attempt is durably recorded before `begin_boot` returns
  that candidate to its caller.
- The retry budget is fixed at three candidate boots. If all three fail to
  reach the caller's success gate, the next call clears the pending update and
  selects the last confirmed slot.
- `mark_boot_success` promotes only the pending slot passed by its caller;
  unexpected slot confirmations fail closed.
- The record format has a magic value, version, fixed retry-policy field,
  generation, reserved-byte checks, and CRC-32. Two alternating copies keep a
  prior valid generation while the next record is written and flushed.
- Invalid records, same-generation disagreement, and exhausted generation
  counters fail closed.
- `UefiVariableBootControlStore` backs both journal copies with separate UEFI
  non-volatile variables under a Nagi-owned vendor GUID. It requires
  non-volatile, boot-service, and runtime attributes and fails closed on
  missing/unsupported operations or unexpected attributes.
- The `m27-ab-slot-acceptance` loader feature seeds a pending B trial only when
  the journal is empty; it also enables production
  `m27-ab-slot-boot-control`. The release image enables only the production
  feature, so an empty release journal does not stage an update. The CLI `m27`
  fixture preserves malformed-B and healthy A/B images for rollback and
  promotion tests.
- BootInfo v4 carries the candidate slot, attempt, journal generation, and UEFI
  Runtime Services `SetVariable` entry point only on trial boots. The kernel
  validates the pointer against a runtime-code memory descriptor. Its
  no-argument `SYS_BOOT_READY` derives all record data from BootInfo, writes a
  versioned and checksummed `NagiBootReady` variable once, and exposes no
  caller-selected firmware operation. See ADR-0011.
- Init reports readiness only after the first M10 surface present succeeds.
  On the next loader entry, only a record matching the pending slot, attempt,
  generation, and required UEFI attributes can call the existing
  `mark_boot_success`; stale or malformed records are discarded. Persistence
  failure keeps the trial pending and stops init before desktop-ready.
- The current guest has no authentication/login flow; this readiness slice
  proves core checks and initial desktop presentation only. The release gate
  must extend it to the specification's full session readiness point.

## UEFI persistence foundation

The current `BootControlJournal` originally had only an in-memory test store.
This slice connects its existing two-copy record interface to two Nagi-owned UEFI
non-volatile variables in a dedicated vendor namespace. Each write uses
`NON_VOLATILE | BOOTSERVICE_ACCESS | RUNTIME_ACCESS`; reads with unexpected
attributes, unsupported variable operations, or oversized values fail closed
as storage errors. `set_variable` is synchronous at the UEFI API boundary, so
the adapter's `flush` has no deferred buffer to drain.

A prior journal-only slice used the `m27-boot-control-smoke` feature to verify
the persisted decision before slot payload selection was connected. Its logs
and variables remain preserved in
`out/evidence/m27-uefi-persistence-1790725732395208000/` as historical evidence.

The firmware store remains the durable journal backend. It does not itself
select payloads or mark a system ready; those behaviors are handled by the
loader and acceptance work described below.

## Verification

Using the pinned `nightly-2025-08-01` toolchain:

- `cargo test --manifest-path loader/Cargo.toml --locked --lib` — passed, 10
  tests total (6 new A/B tests and 4 existing ELF parser tests).
- `cargo build --manifest-path loader/Cargo.toml --target x86_64-unknown-uefi
  --release --locked` — passed.
- `cargo fmt --manifest-path loader/Cargo.toml -- --check` — passed.
- `cargo clippy --manifest-path loader/Cargo.toml --lib --locked -- -D warnings
  -A clippy::derivable_impls` — passed; the allow covers an existing manual
  `Default` implementation in `loader/src/elf.rs`.
- The UEFI release build with `--features m27-ab-slot-acceptance` and
  warnings-denied target Clippy passed.
- CLI tests passed (117 unit and 18 integration); warnings-denied Clippy and
  formatting passed.
- `libnagi` storage regression tests passed on the x86-64 macOS target (34
  unit and 2 integration tests); warnings-denied Clippy passed. The Nagi user
  target check passed for `nagi-init --features m27-ro-vfs-check`.
- Fresh `./nagi m27` acceptance passed on 2026-09-30. The bootstrap wrote the
  persistent guest disk. QEMU boots 1–3 selected B and printed
  `Nagi M27 trial payload rejected slot=B` followed by
  `Nagi Loader: invalid ELF`; none reached `Nagi Kernel started`. Boot 4
  selected rollback slot A, read the saved user-data marker, and reached M7
  acceptance; boot 5 again selected confirmed A and verified the same data.
  The shared OVMF variables, separate user-data disk, and all six serial logs
  are preserved in `out/evidence/m27-ab-rollback-1790727880353946000/`.
- A review found the earlier runner stopped on the pre-failure rejection
  marker. It now waits for the loader's invalid-ELF diagnostic, requires both
  failure markers, and rejects any trial log containing `Nagi Kernel started`.
  A CLI regression test covers those conditions; the fresh QEMU run above
  passed with all three full failure paths observed.
- The latest `./nagi m27` run built the checker into the slot init and
  confirmed it on both rollback and confirmed boots. Evidence is preserved in
  `out/evidence/m27-ab-rollback-1790729662262827000/`.
- The completion-sweep `./nagi m27` run passed on 2026-09-30 with BootInfo v4
  and `SYS_BOOT_READY`. The healthy System B trial persisted slot B, attempt 1,
  generation 3 before `Nagi M10 desktop READY`; the next launch printed
  `Nagi M27 readiness record consumed slot=B PASS` before the confirmed-B
  decision; the following launch still selected confirmed B. The same run
  repeated the three-malformed-B rollback path. All nine launch logs, OVMF
  variables, and the persistent data disk are in
  `out/evidence/m27-ab-rollback-1790734306736600000/`.
- Focused host tests passed: BootInfo (15), ABI (4), CLI (121 unit and 18
  integration), and loader (10). The feature-enabled UEFI loader Clippy build,
  loader release build, Nagi kernel target build, and
  `nagi-init --features m10-desktop,m27-ro-vfs-check` target build passed.
- Host `libnagi` unit tests could not compile on this arm64 macOS host because
  the crate's x86-64 syscall register names are invalid for the host target.
  Host kernel unit tests also fail on the pinned nightly because existing
  kernel sources use unstable `unsigned_is_multiple_of` APIs without the crate
  feature gate. The new kernel readiness path is exercised by the QEMU target
  acceptance above.
- A warnings-denied kernel target Clippy attempt reports existing diagnostics
  in audio, scheduler, user_process, smp, syscall, and main; none point to the
  new `boot_control.rs` readiness implementation. The target build and QEMU
  acceptance pass.
- The follow-up `./nagi m27` run also passed after the CLI gate began checking
  the ordering of guest persistence before desktop readiness and loader record
  consumption before promotion. The loader now also requires exact UEFI
  variable attributes on the readiness record. Current evidence is in
  `out/evidence/m27-ab-rollback-1790735315934886000/`.
- The completion-sweep regression after the M30 GPT loader/kernel changes
  passed the full malformed-B rollback, healthy-B readiness promotion,
  Recovery Undo, and restart checks again. Fresh evidence is in
  `out/evidence/m27-ab-rollback-1790744128241974000/`.
- A follow-up exposed that the M27 fixture attaches a separate GPT User Data
  disk beside its legacy FAT boot image. Loader fallback detection now counts
  only Nagi ESP/System/Recovery partition GUIDs, so that data disk does not
  masquerade as a GPT boot layout; a GPT boot layout with the requested Nagi
  volume missing still fails closed. Default and M27 UEFI target builds,
  loader tests, warnings-denied target Clippy, and the complete M27 QEMU
  acceptance passed after this fix. Evidence is in
  `out/evidence/m27-ab-rollback-1790744869754176000/`.
- The QEMU host logged that it has no `virtio-sound.in` audio driver. The M27
  acceptance does not exercise audio; all boot-control and persistent-data
  markers passed.

## GPT partition A/B and Recovery acceptance

The completion sweep builds malformed-B and healthy-B qcow2 images with the
same six-partition reference GPT layout used by M30. In both images the loader
selects payloads by the documented unique partition GUIDs, the kernel exposes
the bounded User Data capability, and the guest uses the GPT User Data
partition rather than a separate raw data disk. The malformed image passes
three System B attempts, Recovery with the pending journal unchanged, rollback
to System A, and a confirmed-A restart. The healthy image persists the guest
readiness record on System B, selects Recovery without losing the record, then
boots confirmed System B. Recovery also passes the read-only VFS check and
console commands. Full evidence is under
`out/evidence/m27-ab-rollback-1790750679606495000/gpt-integration/`.

The M30 production loader uses `m27-ab-slot-boot-control`; only the acceptance
feature seeds the pending trial needed for the rollback/promotion fixture.
`./nagi m27` passed end-to-end on 2026-09-30 after the QEMU runner began
requesting a QMP `quit` at serial acceptance, so GPT User Data writes are
flushed before the next QEMU process. Account-authenticated readiness,
authenticated update manifests/installation, prior-boot log retrieval, and
filesystem repair remain outside this completed acceptance slice.

Host `clippy --all-targets` is not a usable check for the UEFI binary: its
target-only `uefi` dependency is unavailable in a host build. The UEFI target
release build above verifies the binary target instead.

The A/B tests cover repeated trial attempts across reconstructed journal
instances, rollback at the retry limit, success promotion, torn inactive-copy
writes, fallback after corruption of the newest copy, ambiguous/corrupt state,
and record checksum/invariant validation. These use an in-memory test store;
they do not establish firmware persistence or guest boot behavior.

## Remaining M27 work

1. Add authenticated slot manifests and a viable GPT update/installation path;
   the current GPT images are fixture-built and do not prove updater
   authorization or artifact authenticity.
2. Extend the current core-check + first-desktop readiness signal to the full
   authenticated session readiness gate when login/authentication exists.
3. Extend Recovery beyond its current bounded console with persistent boot-log
   retrieval, the remaining important-file/history restore operations, an
   advanced terminal, and basic filesystem repair; the current checker is
   intentionally read-only. Older NH16 Move records without a digest still
   validate path occupancy only.

Until those pieces pass their acceptance criteria, M27 remains PARTIAL.

## Matched-payload rollback slice (completed)

The acceptance used two explicit directories in a dedicated FAT fixture:
`EFI/NAGI/SYSTEMA/{KERNEL,INIT}.ELF` and
`EFI/NAGI/SYSTEMB/{KERNEL,INIT}.ELF`. System A contains the current working
payload; System B has a deliberately invalid kernel ELF. A normal loader image
first bootstraps the guest's separate persistent data disk. The test loader
then stages B, increments the same UEFI journal before each trial, and opens
both files from the selected slot directory. The malformed B ELF was reported
as a rejected trial on each of three separate QEMU processes. The next launch
selected A and read its M7 persistent-storage marker. The same OVMF variables
and user-data disk were reused throughout.

The original malformed-payload acceptance passed with the preserved evidence
above. It proves firmware-backed selection of paired payloads and rollback to
A while preserving data on the separate user-data disk. The later readiness
promotion slice adds a positive guest signal for the current M10 gate. GPT
partition selection is now covered by the separate integration acceptance
above. Authenticated slot manifests and the full login readiness gate remain
open.

## Completion Sweep GPT rerun — 2026-10-01

After the M24 completion-sweep checkpoint, `./nagi m27` passed again with the
repository's pinned nightly on the ARM64 host. The run repeated three malformed
System B trials and rollback to persistent System A, then exercised GPT
Recovery without journal mutation, the post-Recovery rollback/confirmed-A
boots, a healthy System B readiness promotion across Recovery, and confirmed
System B. The legacy A/B + Recovery path and its guest M22 Undo/restart checks
also passed. All outputs are in
`out/evidence/m27-ab-rollback-1790782251041169000/`.

The first invocation exposed a host PATH issue: Homebrew Cargo was ahead of the
rustup shim and tried to link the host CLI for x86_64. The M0 POSIX launcher
was corrected to select Cargo and rustc from the rustup shim directory; its
mocked toolchain-selection regression and full image acceptance passed. This
is a host launcher correction, not a change to the M27 boot policy.

## Completion sweep QEMU reruns — 2026-10-01

The current source passed the full M27 gate as repetition 1 of the integrated
M28 run; evidence is at
`out/evidence/m27-ab-rollback-1790784019407773000/`. A subsequent standalone
run timed out on the healthy-B readiness promotion boot 2. Its serial log
contains only the 87-byte UEFI screen-clear sequence and no BDS or Nagi kernel
marker. The same saved A/B image and post-trial OVMF variables were then booted
from a disposable copy with the equivalent QEMU devices; that run consumed the
readiness record, selected confirmed B, and reached M10 desktop READY. This
targeted replay is preserved under
`out/evidence/m27-ab-rollback-1790784716290030000/readiness-promotion-diagnostic/`.

A full M27 rerun then passed the preceding A/B and M22 transaction stages but
timed out before BDS output during the Recovery QEMU launch. Its 87-byte log
and full run evidence remain at
`out/evidence/m27-ab-rollback-1790785017110028000/`. These two failures are
host-QEMU/OVMF startup timeouts observed before guest code ran; the targeted
replay shows the healthy-B journal state remains valid. Their root cause is
not established, so they are recorded as flaky local boot evidence rather than
counted as additional M27 passes. Authenticated manifests, updater
authorization, and the remaining Recovery functions continue to keep M27
PARTIAL.

## Completion Sweep rerun after GUI timeout — 2026-10-01

The next integrated one-repetition M28 run passed the complete M27 GPT A/B and
Recovery gate, including malformed-B rollback, pending-journal Recovery,
healthy-B readiness promotion, and confirmed-B startup. Its QEMU evidence is
at `out/evidence/m27-ab-rollback-1790788040114277000/`. The preceding
90-second GUI Recovery timeout remains preserved at
`out/evidence/m27-ab-rollback-1790787806085590000/`; its root cause remains
unconfirmed. Host QEMU timeout handling now attempts to record bounded status
and CPU-register details on future headless or GUI timeouts; this passing run
did not exercise that failure path. M27 remains PARTIAL for the authenticated
update, full session-readiness, and Recovery features listed above.

## Latest integrated timeout diagnostic — 2026-10-01

The latest M28 two-repetition run passed M19 Search/ObjectId persistence and
the three-boot M22 grouped-Undo/Activity-Ledger gate in repetition 1, then
timed out during M27 boot 4 after malformed-B rollback selected confirmed A.
The guest printed three M3 AP-online markers but no scheduler-workload marker.
The bounded QMP record reported `status=shutdown`, `running=false`, and CPU#0
RIP `0x4005d90`; `llvm-addr2line` maps that address to
`nagi_kernel::smp::thread_entry`. This evidence localizes investigation to the
M3 SMP/scheduler transition but does not establish a specific root cause.

The run's M27 input images, OVMF variables, user-data image, boot logs, and
QMP output with a verified SHA-256 manifest are preserved in
`out/evidence/m27-ab-rollback-1790796067483623000/`. The M27 run remains
`PARTIAL`; the most recent successful complete QEMU acceptance is still
`out/evidence/m27-ab-rollback-1790788040114277000/`. A useful next diagnostic
is bounded per-CPU timer and workload progress on this boot path, followed by
a replay from the preserved rollback inputs before modifying scheduler code.

## Completion sweep rollback-sequence replay — 2026-10-01

A fresh `./nagi m27` run completed successfully at
`out/evidence/m27-ab-rollback-1790800250176976000/`. It passed the three
malformed-System-B rollback attempts, confirmed-A boots, healthy-System-B
readiness and promotion, GPT Recovery, persistent User Data checks, and the
Recovery restart/Undo of the committed M22 grouped `file.move` transaction.
The invocation output, serial logs, and OVMF state have a verified
`SHA256SUMS` manifest in that directory.

This replay generated new inputs and OVMF state; it was not a byte-for-byte
replay of the preceding failed boot's firmware state. The prior boot-4 timeout
did not recur, so the source of its `CR2=0xfffffffffffffff8` snapshot remains
unconfirmed. The ELF extracted from the preserved failing System A image maps
RIP `0x4005d90` to the first instruction of `smp::thread_entry`; that
instruction only checks its two arguments. This narrows the snapshot but does
not establish that this instruction caused the fault. M27 remains `PARTIAL`
for authenticated slot/update authority and the remaining Recovery features.

The subsequent M28 integration attempts also passed the full M27 GPT gate in
repetition 1 twice, at
`out/evidence/m27-ab-rollback-1790800692151516000/` and
`out/evidence/m27-ab-rollback-1790801054740594000/`. Together with the fresh
standalone pass above, the boot-4 timeout has not recurred in three new full
acceptance sequences. These runs used fresh input/firmware state and do not
identify the earlier timeout's cause.

The next two complete M27 gates passed as both repetitions of the successful
M28 run: `out/evidence/m27-ab-rollback-1790802782930760000/` and
`out/evidence/m27-ab-rollback-1790802894479757000/`. Each archive now has a
README and verified 31-file `SHA256SUMS` manifest. These additional fresh
acceptance runs passed three-trial rollback, readiness-based B promotion,
Recovery journal preservation, persistent User Data, and Recovery Undo of the
M22 grouped transaction. The original boot-4 fault remains unexplained; M27
remains `PARTIAL` for account-authenticated readiness, authenticated slot
manifests/update installation, and remaining Recovery repair/log features.

## Read-only NH16 history view — 2026-10-01

Recovery now accepts `history`. It loads the checksummed NH16 archive through
the existing read-only path and prints at most the latest 16 entries with
sequence, transaction ID, operation, transaction state, and Object ID. It does
not update the archive or the A/B boot journal. The M27 QEMU acceptance first
creates three actual guest `file.move` records, requires the new history PASS
marker before issuing `undo`, then verifies the undo from a subsequent guest
restart. Output showed all three rows as `MOVE / COMMITTED`; the complete run
is preserved at `out/evidence/m27-ab-rollback-1790810805162353000/`. This adds
Recovery diagnostics, not filesystem repair or authenticated slot/update
authority; M27 remains `PARTIAL`.

## M29 completion-sweep regression — 2026-10-01

After adding the shared localization dependency to the M10 desktop build,
`./nagi m27` passed again. The fresh acceptance repeated three malformed
System B trials and rollback, healthy System B readiness/promotion, GPT
Recovery, persistent User Data checks, and Recovery Undo of the committed
three-file M22 group across restart. The last verification boot is
`out/evidence/m27-ab-rollback-1790812676859868000/boot-5.log`; the complete
run's serial logs, OVMF variables, and User Data image are in that directory.
The prior boot-4 timeout remains unexplained, and the authenticated update and
remaining Recovery work keep M27 `PARTIAL`.

## System-language persistence regression — 2026-10-01

After adding System language persistence to the M10 Desktop, a fresh
`./nagi m27` acceptance passed the full GPT A/B and Recovery path at
`out/evidence/m27-ab-rollback-1790816764088451000/`. It rejected three malformed
System B trials and rolled back to A, promoted a healthy B after guest
readiness, booted Recovery without changing the journal, and undid the
committed three-file M22 group across restart. The directory README and
`SHA256SUMS` record the acceptance inputs, logs, firmware variables, and User
Data; the manifest was verified.

A preceding fresh run (`1790816606753646000`) timed out during the initial
malformed-System-B trial before guest output. QMP reported a running VM with
CPU#0 in an OVMF instruction loop. The successful run used new image and
firmware state, so the earlier timeout's cause remains unknown. M27 remains
`PARTIAL` for authenticated slot/update authority, full session readiness,
and remaining Recovery features.

## M28 repeated-gate sub-runs — 2026-10-01

The M28 two-repetition Search/Undo/Recovery run passed both full M27 QEMU
acceptances. Run `1790818018301818000` and run `1790818130409530000` each
verified three malformed System B rollbacks to persistent A, promoted healthy
B after guest readiness, booted Recovery without changing the journal, and
undid the committed M22 `file.move` group across restart. Both evidence
directories include a README and verified 37-entry SHA-256 manifest. Their
logs and images are cross-linked from
`out/evidence/m28-run-20261001T012645Z-75535/`. The separate earlier OVMF
startup timeout remains unexplained; M27 remains `PARTIAL` for authenticated
slot/update authority, full session readiness, and remaining Recovery
features.

## Current-branch Completion Sweep acceptance — 2026-10-01

On source commit `fa9b73a9c20434b52f414a02f969140b5f2e1ac6`, a fresh
`./nagi m27` run passed both the legacy and GPT acceptance paths. The GPT
fixtures rejected three malformed System B trials and rolled back to persistent
System A; a healthy B trial persisted guest readiness, survived a Recovery boot
with the journal unchanged, and was confirmed on the following boot. Recovery
also checked the VFS and undid the committed M22 `file.move` group across
restart. The 37-file SHA-256 manifest and run notes are in
`out/evidence/m27-ab-rollback-1790855312225301000/`.

QEMU again reported that this host could not open `virtio-sound.in`; this run
does not cover host audio. M27 remains `PARTIAL` for authenticated readiness
and update authority, authenticated slot manifests, and the remaining Recovery
features.

## Bootstrap completion marker and repeated integration — 2026-10-01

The M27 bootstrap QEMU launcher previously waited on `Nagi M7 persistent
write PASS`, which appears before `Nagi M7 reboot required PASS`. On one
timeout, QEMU stopped between those serial lines, leaving a truncated log and
making M27 report a false bootstrap failure. The launcher now waits for the
later reboot-required marker, and a host regression requires both markers in
the captured bootstrap log.

After this change, both M27 sub-runs in
`out/evidence/m28-run-20261001T130826Z-2523/` passed the complete GPT A/B and
Recovery gate; their 31-file manifests verify at
`out/evidence/m27-ab-rollback-1790860121579655000/` and
`out/evidence/m27-ab-rollback-1790860238754593000/`. A later standalone M27
sub-run also passed at
`out/evidence/m27-ab-rollback-1790861315791167000/` with a verified manifest.
Those runs confirm the bootstrap marker handoff and M27 acceptance, not the
remaining authenticated update/readiness or Recovery requirements.

Intermittent firmware startup failures still recur in separate attempts. For
example, `out/evidence/m27-ab-rollback-1790860688634532000/recovery-boot.log`
records a running QEMU VM at RIP `0x7eb84171` looping over `jmp 0x7eb84150`
before the Recovery acceptance marker. That failure is preserved and is not
counted as a pass. M27 remains `PARTIAL`.

## Standalone A/B and Recovery rerun — 2026-10-02

A fresh `./nagi m27` run passed three malformed System B rollback trials to
persistent System A, healthy System B promotion after guest readiness, Recovery
with an unchanged journal, and M22 `file.move` group Undo across restart. Its
38-entry `SHA256SUMS` covers the nested logs, variables, User Data, and seven
run-stamped images retained under `out/artifacts/`:
`out/evidence/m27-ab-rollback-1790869370028356000/`. The run does not complete
authenticated update/readiness authority or the remaining Recovery acceptance;
M27 remains `PARTIAL`.

## Completion Sweep — pending-trial Recovery timeout replay — 2026-10-02

The fresh full run `1790884455740214000` again timed out after 90 seconds before
Nagi guest serial output during Recovery with pending System B. QMP reported a
running guest at RIP `0x7eb84171`, looping on `jmp 0x7eb84150`. Its complete
failure evidence and 15-entry manifest are under
`out/evidence/m27-ab-rollback-1790884455740214000/`. Together with run
`1790881641572651000`, this is two consecutive full-run failures at that stage;
neither is counted as an acceptance pass.

To isolate the preserved state, the Recovery image, User Data disk, and
post-timeout OVMF variables were copied into
`out/evidence/m27-pending-b-replay-20261002/fresh-run-1790884455740214000/`.
With the same QEMU 11.1.1 and OVMF pair, the copied state booted Recovery in
2.313 seconds, reported `confirmed=A pending=B`, left the boot journal
unchanged, and passed VFS and command-help checks. Its eight-entry manifest
verifies. The replay establishes that the copied post-timeout state can boot;
it does not identify the full-run failure cause or replace full M27
acceptance. QEMU reported no host `virtio-sound.in` input driver. M27 remains
`PARTIAL` for authenticated update/readiness authority, authenticated slot
manifests, and remaining Recovery work.

## Completion Sweep — M28 Recovery startup replay (2026-10-02)

M28's first M27 sub-run in `out/evidence/m28-run-20261001T205917Z-97811/`
passed the full A/B, Recovery, and committed M22 Undo gate. The second M27
sub-run timed out in the Recovery GUI boot before guest output; QMP again
reported the OVMF loop at RIP `0x7eb84171`. The failure directory
`out/evidence/m27-ab-rollback-1790888490534092000/` retains the exact Recovery
image, User Data, OVMF variables, and QMP diagnostic log with a verified
manifest.

A replay using copies of that exact image, User Data, and OVMF variable state
reached the Recovery menu, received the `r` key events and command batch, and
passed the Recovery help marker. Evidence and script are under
`out/evidence/m27-replay-recovery-gui-1790888490534092000/` with a verified
manifest. A separate M28 failure at the Recovery Undo fixture also replayed
successfully from its preserved boot image/User Data and fresh pinned OVMF
variables, reaching M13/M21/M22 markers. Neither replay changes the original
M28 result. These failures are consistent with an intermittent pre-guest OVMF
startup issue, but the firmware root cause remains unknown. M27 remains
`PARTIAL` for authenticated update/readiness authority, authenticated slot
manifests, and remaining Recovery work.

## M28 final System A boot timeout replay — 2026-10-02

In M28 run `out/evidence/m28-run-20261001T211548Z-133/`, repetition 2 passed
M19 and the three-boot M22 gate before the M27 A/B sequence timed out at boot 5.
The required final `confirmed slot=A` guest marker was not reached. QMP recorded
RIP `0x7eb84171` in the same OVMF loop seen in earlier pre-guest timeouts. The
failed sub-run is
`out/evidence/m27-ab-rollback-1790889480617050000/` with its own manifest.

A diagnostic replay used a byte-for-byte copy of the failed run's A/B image and
User Data. The saved OVMF variable file was the post-failure copy; a pre-boot
snapshot was unavailable, so the replay is not an exact-state reconstruction.
With fresh copies of those saved inputs, QEMU reached
`Nagi M27 persistence decision: confirmed slot=A` and `Nagi M7 acceptance
PASS`. The replay manifest is at
`out/evidence/m27-replay-system-a-1790889480617050000/SHA256SUMS`. This does
not convert the original M27 stage or M28 repetition to a pass, and the root
cause remains unconfirmed. M27 remains `PARTIAL`.

## Completion Sweep scheduler handoff diagnostics — 2026-10-02

A one-repetition M28 run passed M19 Search/ObjectId persistence and the three-
boot M22 grouped Undo/Activity Ledger gate. M27 then timed out on rollback boot
4 after the guest printed three M3 AP-online markers. QMP reported shutdown and
CPU#0 RIP `0x40060c0`, which maps to `nagi_kernel::smp::thread_entry`. The full
M28 run evidence is `out/evidence/m28-run-20261001T235755Z-18930/`; the M27
sub-run logs and QMP output are under
`out/evidence/m27-ab-rollback-1790899090521806000/`. Their SHA-256 manifests
were verified. This M28 repetition failed and is not counted as an acceptance
pass.

The BSP scheduler-start serial marker previously followed `sti`, so an
immediate timer handoff could occur before the marker. The marker now prints
before enabling BSP interrupts; scheduler behavior is unchanged. The focused
kernel rustfmt check passed, and the subsequent target build/QEMU attempt
reported `Nagi M3 scheduler workload START`, `DONE`, and M3 acceptance in the
bootstrap, rollback, Recovery, and confirmed-A guest logs.

That M27 attempt later timed out before the healthy-B readiness-trial guest
started. Its serial log contains only UEFI screen-clear bytes; QMP reported
`status=running`, RIP `0x7eb84171`, and an OVMF instruction loop outside the
kernel address range. The exact evidence, including OVMF variables, User Data,
all logs, and verified manifest, is at
`out/evidence/m27-ab-rollback-1790899532394169000/`. This is a firmware-start
timeout with unknown root cause; it does not establish a kernel scheduler
failure or a complete M27 pass. M27 and M28 remain PARTIAL.

## Completion Sweep — two M27 gates in the current M28 run (2026-10-02)

Both M27 sub-runs in
`out/evidence/m28-run-20261002T002354Z-22197/` passed the malformed System B
rollback, healthy System B readiness/promotion, and Recovery journal/committed
M22 Undo checks. Their evidence directories are
`out/evidence/m27-ab-rollback-1790900649030767000/` and
`out/evidence/m27-ab-rollback-1790900764105897000/`; both SHA-256 manifests
verify. This repeated QEMU result does not supply authenticated update or slot
manifest authority, so M27 remains `PARTIAL`.

## Standalone A/B and Recovery rerun — 2026-10-03

A fresh `./nagi m27` run `1790980651240874000` passed the complete legacy and
GPT A/B/Recovery acceptance after the previous M28 repetition-2 OVMF timeout.
Three malformed System B trials rolled back to persistent System A; the
healthy B trial was promoted only after guest readiness; Recovery preserved
the journal and undid the committed M22 `file.move` group across restart. Both
GPT slot images pass `qemu-img check`. The run's README and 38-entry
`SHA256SUMS` cover all 30 local evidence files and seven run-stamped images;
all entries verify from
`out/evidence/m27-ab-rollback-1790980651240874000/`.

The isolated fresh-state rerun did not reproduce the earlier OVMF startup
loop, but it does not reconstruct that M28 repetition's exact pre-boot state
and does not resolve the intermittent firmware failure. The host again lacked
`virtio-sound.in`; this gate does not exercise audio. M27 remains `PARTIAL`
for authenticated update/readiness authority, authenticated slot manifests,
full session readiness, and remaining Recovery features.

## M28 repeated A/B and Recovery acceptance — 2026-10-03

Both M27 sub-runs in the fresh two-repetition M28 gate passed the GPT
malformed-System-B rollback, healthy-System-B readiness/promotion, Recovery
journal preservation, and committed M22 `file.move` Undo across restart. Their
evidence is under `out/evidence/m27-ab-rollback-1790981151205790000/` and
`out/evidence/m27-ab-rollback-1790981267226055000/`; both SHA-256 manifests
verify. All four run-stamped GPT slot images passed `qemu-img check`.

The repeated fresh-state passes did not reproduce the earlier OVMF startup
loop, but they do not determine its cause. Authenticated update/readiness
authority, authenticated slot manifests, full session readiness, and
remaining Recovery work keep M27 `PARTIAL`.
