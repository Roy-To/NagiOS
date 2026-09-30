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
   intentionally read-only.

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
