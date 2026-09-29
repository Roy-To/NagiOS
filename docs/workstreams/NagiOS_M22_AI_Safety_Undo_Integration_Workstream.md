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

`user/nagi-history/src/guest.rs` adds a two-slot `HistoryArchiveStore`
adapter contract. Each checksummed slot is bounded to one 1 KiB guest VFS
file; writes go to the inactive generation and flush before returning. The
target-only `m22-history` init feature connects that contract to two persistent
guest VFS files. Its acceptance fixture writes a prepared three-move group,
applies the guest VFS renames, persists Committed, prepares and persists
UndoPending, applies inverse moves in reverse order, then persists Undone.
UndoPending replay is idempotent across a reboot partway through the inverse
batch.

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

- `cargo test --locked --offline -p nagi-history` — PASS, 9 tests covering
  grouped three-move ordering, full-width context restoration, caller denial,
  prepared/committed state, pending-undo restart recovery, archive
  corruption/version rejection, atomic group validation, two-slot guest
  archive selection, and corruption fallback.
- `cargo clippy --locked --offline -p nagi-history --all-targets -- -D warnings`
  — PASS.
- Changed-file format checks for `nagi-history`, `nagi-init`, and `nagi-cli` —
  PASS.
- `cargo -Z build-std=core,alloc check --locked --offline -p nagi-init
  --features m22-history --target targets/x86_64-unknown-nagi-user.json` — PASS.
- `cargo test --locked --offline -p nagi-cli` — PASS, 114 unit tests and 18
  integration tests, including the `m22` command surface.
- `./nagi m22` — PASS for durable guest History recovery across QEMU boots.
  The first attempt wrote and committed all three VFS moves, then exposed an
  unrelated M7 acceptance limit: its root-directory lookup buffer held only
  eight entries. After bounding that buffer by the ext2 64-inode limit, a new
  QEMU boot restored the same Committed NH16 archive, applied and persisted
  composite undo, and two further boots verified Undone state and original
  file contents. The final acceptance output is recorded in
  `out/logs/m22-history-boot-1.log` through `m22-history-boot-3.log`; the first
  forward-move marker was observed before the root-listing fix.
- Regression rerun on 2026-09-30 passed all three QEMU boots. The prior disk,
  OVMF vars, bootstrap log, and three serial logs were copied before the run to
  `out/evidence/m22-before-sweep-rerun-20260930/`.
- This is a guest VFS/persistence fixture only. It is not the required
  authenticated M21 Executor action and does not prove production capability
  checks or linkage to the M15 NH15 ledger.
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
- The NH16 host tests verify its serialization and recovery contract. The
  separate `./nagi m22` QEMU fixture verifies guest VFS persistence and undo
  across restarts; neither layer proves authenticated policy, a production
  M21 file mutation, or Activity Ledger linkage.

M22 remains `BLOCKED` at its dependent AI acceptance. The durable NH16 guest
archive and restart-restorable composite undo now pass a private QEMU fixture.
Production M21 move handlers, authenticated caller policy, real AI-to-History
transaction linkage, and the production Activity Ledger/restart acceptance
remain missing. Do not treat the fixture as M21-authorized mutation or an
M22 milestone PASS.
