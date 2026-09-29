# Nagi OS M27 — A/B and Recovery Workstream

## Current state: PARTIAL

This checkpoint adds a deterministic A/B boot-control state machine and a
checksummed, two-copy record journal to `loader/src/ab.rs`. It is a reusable
foundation only. The current UEFI loader still loads the single fixed
`KERNEL.ELF` and `INIT.ELF` pair, and the image builder does not create A/B
system slots. No broken-slot QEMU acceptance or Recovery Environment is
claimed.

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
- The CLI `m27` smoke fixture seeds a pending B trial only when the journal is
  empty. It keeps one initialized OVMF variables file across separate QEMU
  processes and verifies persisted trial counts followed by the journal's
  rollback decision and confirmed A state. It does not select a different
  payload.

## UEFI persistence smoke slice design

The current `BootControlJournal` originally had only an in-memory test store.
This slice connects its existing two-copy record interface to two Nagi-owned UEFI
non-volatile variables in a dedicated vendor namespace. Each write uses
`NON_VOLATILE | BOOTSERVICE_ACCESS | RUNTIME_ACCESS`; reads with unexpected
attributes, unsupported variable operations, or oversized values fail closed
as storage errors. `set_variable` is synchronous at the UEFI API boundary, so
the adapter's `flush` has no deferred buffer to drain.

A loader feature named `m27-boot-control-smoke` seeds a pending B trial only on
an empty journal, then calls `begin_boot` and prints the persisted decision.
The CLI `m27` acceptance builds that loader fixture and launches it
once for the existing guest's M7 persistent-storage bootstrap, then four more
times while reusing one freshly initialized OVMF variable file. It will
checks attempts 1, 2, and 3 across resets, then checks that the fourth
post-bootstrap invocation clears the failed trial and reports confirmed slot
A. Each post-bootstrap run also has to reach the existing guest acceptance
marker.

This is a firmware persistence and journal-state smoke test. The normal loader
continues loading the fixed `KERNEL.ELF` and `INIT.ELF`; the smoke feature does
not select a slot, mark a system ready, or claim a successful rollback boot.
Matched A/B payloads, readiness, recovery UI, and broken-slot boot acceptance
remain outside this slice. Generated vars and serial logs will use a unique
evidence directory per invocation so subsequent runs do not overwrite them.

Verification for this slice: loader host tests, UEFI release build with the
smoke feature, CLI unit tests and formatting, and the five-boot QEMU acceptance
using a single OVMF variable image.

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
- The UEFI release build with `--features m27-boot-control-smoke` and
  warnings-denied target Clippy passed.
- CLI tests pass (115 unit and 18 integration); warnings-denied Clippy and
  formatting pass.
- `./nagi m27` passed locally on 2026-09-30. Its M7 storage bootstrap first
  wrote the persistent guest disk, then four fresh QEMU processes reused the
  same OVMF variables file. The journal reported trial attempts 2 and 3,
  rollback to confirmed A, then confirmed A on the following boot; every
  post-bootstrap process reached the M7 guest acceptance marker. Complete vars,
  user-data disk, and serial logs are preserved in
  `out/evidence/m27-uefi-persistence-1790725732395208000/`. The initial runner
  attempt timed out because it waited for the post-bootstrap marker during the
  storage bootstrap boot; the acceptance now checks the bootstrap marker
  separately and then verifies the four restart decisions.

Host `clippy --all-targets` is not a usable check for the UEFI binary: its
target-only `uefi` dependency is unavailable in a host build. The UEFI target
release build above verifies the binary target instead.

The A/B tests cover repeated trial attempts across reconstructed journal
instances, rollback at the retry limit, success promotion, torn inactive-copy
writes, fallback after corruption of the newest copy, ambiguous/corrupt state,
and record checksum/invariant validation. These use an in-memory test store;
they do not establish firmware persistence or guest boot behavior.

## Remaining M27 work

1. Teach image production and the UEFI loader to verify and select matched
   kernel/init pairs from System A and System B, preserving user data outside
   those slots.
2. Connect a trustworthy system-readiness signal to `mark_boot_success`.
3. Add a bootable Recovery Environment with the specified slot selection,
   boot logs, filesystem check, important-file/history restore, advanced
   terminal, and basic repair operations.
4. Run the M27 acceptance: intentionally corrupt the inactive slot, observe its
   failed trial boots, and prove QEMU returns to the known-good slot while
   preserving user data.

Until those pieces and the acceptance pass, M27 remains PARTIAL.
