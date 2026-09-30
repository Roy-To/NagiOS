# Nagi OS M27 — A/B and Recovery Workstream

## Current state: PARTIAL

The UEFI loader now persists its A/B decision in two Nagi-namespaced firmware
variables and, in the feature-scoped acceptance image, loads matched kernel and
init files from the selected System A or System B directory. The QEMU
acceptance has exercised three malformed System B trials, rollback to System
A, and a read of the same persistent user-data disk after rollback. The normal
release image still uses the fixed `KERNEL.ELF` and `INIT.ELF` pair. A positive
readiness signal, the Recovery Environment, and the final partitioned release
layout remain incomplete, so M27 remains PARTIAL.

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
- The `m27-broken-slot-acceptance` loader feature seeds a pending B trial only
  when the journal is empty, then opens both kernel and init from the selected
  slot directory. The CLI `m27` fixture keeps one OVMF variables file and one
  separate persistent user-data disk across five QEMU launches. Three malformed
  B kernels are rejected before loading; the next launch selects A and reaches
  the guest's persistent-data read marker; a fifth launch confirms A remains
  selected.

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
- The UEFI release build with `--features m27-broken-slot-acceptance` and
  warnings-denied target Clippy passed.
- CLI tests passed (117 unit and 18 integration); warnings-denied Clippy and
  formatting passed.
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
- The QEMU host logged that it has no `virtio-sound.in` audio driver. The M27
  acceptance does not exercise audio; all boot-control and persistent-data
  markers passed.

Host `clippy --all-targets` is not a usable check for the UEFI binary: its
target-only `uefi` dependency is unavailable in a host build. The UEFI target
release build above verifies the binary target instead.

The A/B tests cover repeated trial attempts across reconstructed journal
instances, rollback at the retry limit, success promotion, torn inactive-copy
writes, fallback after corruption of the newest copy, ambiguous/corrupt state,
and record checksum/invariant validation. These use an in-memory test store;
they do not establish firmware persistence or guest boot behavior.

## Remaining M27 work

1. Connect a trustworthy system-readiness signal to `mark_boot_success` so a
   viable update can become the confirmed slot.
2. Add a bootable Recovery Environment with the specified slot selection,
   boot logs, filesystem check, important-file/history restore, advanced
   terminal, and basic repair operations.
3. Integrate the paired slots and separate user data into the final
   partitioned release layout, and validate a viable update alongside the
   intentionally malformed-slot rollback acceptance.

Until those pieces and the acceptance pass, M27 remains PARTIAL.

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

The acceptance passed with the preserved evidence above. It proves firmware-
backed selection of paired payloads and rollback to A while preserving data on
the separate user-data disk. It does not provide the final partitioned release
layout, a positive health/readiness signal for a viable update, authenticated
slot manifests, or the Recovery Environment; those remain separate M27/M30
work.
