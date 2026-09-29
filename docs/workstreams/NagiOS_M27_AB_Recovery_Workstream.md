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
- The persistence interface is platform-neutral and can be backed by UEFI
  variables or another durable firmware store. No platform adapter is present
  yet.

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

Host `clippy --all-targets` is not a usable check for the UEFI binary: its
target-only `uefi` dependency is unavailable in a host build. The UEFI target
release build above verifies the binary target instead.

The A/B tests cover repeated trial attempts across reconstructed journal
instances, rollback at the retry limit, success promotion, torn inactive-copy
writes, fallback after corruption of the newest copy, ambiguous/corrupt state,
and record checksum/invariant validation. These use an in-memory test store;
they do not establish firmware persistence or guest boot behavior.

## Remaining M27 work

1. Add a firmware-backed durable store adapter and verify its persistence across
   QEMU restarts with the same OVMF variables image.
2. Teach image production and the UEFI loader to verify and select matched
   kernel/init pairs from System A and System B, preserving user data outside
   those slots.
3. Connect a trustworthy system-readiness signal to `mark_boot_success`.
4. Add a bootable Recovery Environment with the specified slot selection,
   boot logs, filesystem check, important-file/history restore, advanced
   terminal, and basic repair operations.
5. Run the M27 acceptance: intentionally corrupt the inactive slot, observe its
   failed trial boots, and prove QEMU returns to the known-good slot while
   preserving user data.

Until those pieces and the acceptance pass, M27 remains PARTIAL.
