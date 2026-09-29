# Nagi OS M22 — AI Safety / Undo Integration

**Status: BLOCKED**

## Acceptance target

Use the validated M21 Executor and existing History/Transaction machinery to
move three guest files as one authorized transaction, record the app/session/
Node/workspace/Object context in the Activity Ledger, undo the three moves,
restart, and verify both restored state and ledger. Undo must resolve within
the original caller boundary. Irreversible operations must not advertise undo.

## Existing History API audit

`user/nagi-history/src/lib.rs` provides a bounded `HistoryService` with
independent Create, Edit, Move, Delete, and Restore entries. Each entry has a
monotonic sequence and logical ActivityContext. `undo_last()` removes and
returns one in-memory inverse action; the caller must apply it. The API has no
`TransactionId`, grouping, composite undo, authorization-bound undo lookup,
or restart restore operation.

`serialize()` emits the `NH15` marker and a fixed 32-byte record per entry.
It preserves Object ID, operation, sequence, selected caller identifiers, and
lengths, but writes only name/snapshot lengths rather than the name/content
bytes. There is no deserializer. `user/nagi-init/src/m15_history.rs` writes
this compact metadata ledger to the persistent VFS and checks that it can be
read; it does not restore a HistoryService or undo payload after restart.

## M21 integration prerequisite

The current `services/nagi-ai` contract has no registered production action
handlers and no authenticated target `ActionPolicy`/`ContextAuthority`
adapter. Its host test handlers cannot perform or authorize guest VFS moves.
Thus there is no safe M21 mutation to wrap in a transaction or connect to the
M15 ledger. Adding an AI-owned journal would duplicate the existing History
system and would not satisfy the required integration.

## Verification and blocker

- `nagi-history` source inspection confirms the API limits above.
- M22's three-file AI move/undo/restart guest acceptance was not run because
  the real authorized M21 file-move action does not exist.
- A local `./nagi m15` regression was attempted. The empty `PT_TLS` parser
  fix allowed the first boot to persist its M7 test data and the second boot
  to load the init ELF and pass M5/M6/M7. The first run exposed that the M13 C
  POSIX fixture declared an 8-byte IPv4 socket address while Nagi's C ABI
  requires the 16-byte `sockaddr_in` layout. After adding the reserved bytes,
  the real C socket/DNS/HTTP, mmap, timing, polling, threading, spawn, relibc,
  and remaining M13 checks passed. M14 playback passed, but capture printed
  `Nagi M14 capture FAIL`; QEMU reported `Can not open virtio-sound.in (no
  host audio driver)`. The 75-second wrapper therefore timed out before
  `m15_history::run`. This is a host audio input limitation, not M15 or M22
  History acceptance.
- The loader parser regression has 15 standalone module tests passing,
  including the empty-`PT_TLS` case. The whole kernel host test target cannot
  be built on this arm64 macOS host because its x86 inline-assembly register
  constraints are unavailable; the intended Nagi target and CI are the
  relevant kernel build checks.
- The `nagi-history` host suite passes all four existing tests. This verifies
  only its current in-memory operation contract, not grouped AI transactions
  or persistent undo restoration.

M22 remains `BLOCKED` by a required M21 production dependency, and by the
missing transaction grouping and restart-restorable undo data in the current
History API. Once M21 supplies real caller-bound actions, extend the existing
History/Transaction service with a versioned grouped and recoverable format,
then run the specified guest acceptance. Do not treat host orchestration
fixtures or metadata-only serialization as that acceptance.
