# Nagi OS M22 — AI Safety / Undo Integration

**Status: BLOCKED**

## Acceptance target

Use the validated M21 Executor and existing History/Transaction machinery to
move three guest files as one authorized transaction, record the app/session/
Node/workspace/Object context in the Activity Ledger, undo the three moves,
restart, and verify both restored state and ledger. Undo must resolve within
the original caller boundary. Irreversible operations must not advertise undo.

## Existing History API and independent extension

`user/nagi-history/src/lib.rs` retains the bounded M15 Create, Edit, Move,
Delete, and Restore API. `undo_last()` remains a compatibility path for a
single operation and refuses to split a grouped transaction.

The independent M22 History contract now adds an `NH16` versioned, bounded,
checksummed archive. It stores full-width AppId, AppSessionId, NodeId,
SurfaceId, WorkspaceId, ObjectId, TransactionId, source/destination names,
before/after payload bytes, and transaction state. `record_move_group` creates
a `Prepared` group so the caller can persist it before applying external
moves. The caller commits only after the complete forward group succeeds.
Undo is restricted to the originating AppId/AppSessionId, persists
`UndoPending`, yields inverses in reverse order, and marks the group `Undone`
after application. Restoring an `UndoPending` archive yields the same batch for
retry. The legacy `serialize()` still emits metadata-only `NH15` for M15; it
does not restore undo data, and the current M15 guest acceptance is not yet
wired to `NH16`.

This caller comparison is an identity consistency check, not an authority
source. Production must supply caller context from the authenticated M21
policy/capability boundary.

## M21 integration prerequisite

The current `services/nagi-ai` contract has no registered production action
handlers and no authenticated target `ActionPolicy`/`ContextAuthority`
adapter. Its host test handlers cannot perform or authorize guest VFS moves.
Thus there is no safe M21 mutation to wrap in a transaction or connect to the
M15 ledger. Adding an AI-owned journal would duplicate the existing History
system and would not satisfy the required integration.

## Verification and blocker

- `cargo test --locked --offline -p nagi-history` — PASS, 7 tests covering
  grouped three-move ordering, full-width context restoration, caller denial,
  prepared/committed state, pending-undo restart recovery, archive
  corruption/version rejection, and atomic group validation.
- `cargo clippy --locked --offline -p nagi-history --all-targets -- -D warnings`
  — PASS.
- `cargo fmt --manifest-path user/nagi-history/Cargo.toml -- --check` — PASS.
- `cargo -Z build-std=core,alloc check --locked --offline -p nagi-history
  --target targets/x86_64-unknown-nagi-user.json` — PASS.
- M22's three-file AI move/undo/restart guest acceptance was not run because
  the real authorized M21 file-move action does not exist, and the `NH16`
  archive does not yet have a durable guest VFS adapter.
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
- The NH16 tests verify a serialization/recovery contract only. They do not
  prove guest VFS persistence, authenticated policy, file mutation, or QEMU
  restart behavior.

M22 remains `BLOCKED` only at its dependent guest acceptance: the History-side
group/archive/undo contract is implemented, but production M21 action and
authenticated policy integration and durable guest archive persistence remain
missing. Continue independent M23-M30 work while those dependencies are
tracked; do not treat host orchestration fixtures or NH16 contract tests as
guest acceptance.
