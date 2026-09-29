# Nagi OS M22 — AI Safety / Undo Integration

**Status: PARTIAL**

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

The separate `NAL1` AI Activity Ledger records bounded user intent, optional
selected model, action ID, plan summary, logical caller context, Object IDs,
transaction ID, and monotonic result transitions. It stores no hidden
chain-of-thought. Its `NLA1` two-slot VFS backend checksums each slot and falls
back to the previous valid generation. The M21 Executor now passes the
validated plan intent to the action handler; the private M22 guest fixture
writes `Prepared` to NAL1 before the VFS mutation, records `Committed` after
the NH16 transaction commit, and persists `UndoPending`/`Undone` during
restart recovery. NH16 undo data and NAL1 activity remain separate archives.

`user/nagi-history/src/guest.rs` adds a two-slot `HistoryArchiveStore`
adapter contract. Each checksummed slot is bounded to one 1 KiB guest VFS
file; writes go to the inactive generation and flush before returning. The
target-only `m22-history` init feature connects that contract to two persistent
guest VFS files. The initial grouped move now enters through a bounded M21
`file.move` Plan / Validate / Execute action. Its handler persists Prepared
before guest VFS renames, flushes the three moves, and persists Committed
before returning to Executor. On the next boot the fixture prepares and
persists UndoPending, applies inverse moves in reverse order, then persists
Undone. UndoPending replay is idempotent across a reboot partway through the
inverse batch.

This caller comparison is an identity consistency check, not an authority
source. Production must supply caller context from the authenticated M21
policy/capability boundary.

## M21 integration prerequisite

The M22 init fixture now registers one bounded guest `file.move` handler and
exercises it with actual VFS objects. Its fixed caller, Object IDs,
`files.move` capability, handle resolver, and deterministic plan are private
acceptance policy. They do not authenticate application processes or authorize
general files. The production `services/nagi-ai` registry remains unwired in a
running guest service. The NAL1 connection exists only inside this
deterministic fixture; there is no authenticated target
`ActionPolicy`/`ContextAuthority` adapter or production Activity Ledger
service. This advances real mutation, History/Undo, and separate-ledger
persistence while leaving those production boundaries open.

## Verification and blocker

- `cargo test --locked --offline -p nagi-ai -p nagi-history --all-targets` —
  PASS, 24 orchestration tests and 14 History/Activity Ledger tests. NAL1 tests
  cover context/model/transaction/result round-trip, malformed text and
  transitions, object limits, archive corruption/version rejection, and
  two-slot corruption fallback.
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
  file contents. At that time the final acceptance output was recorded in
  `out/logs/m22-history-boot-1.log` through `m22-history-boot-3.log`; its
  pre-sweep inputs and logs were preserved under
  `out/evidence/m22-before-sweep-rerun-20260930/`.
- Regression rerun on 2026-09-30 passed all three QEMU boots. The prior disk,
  OVMF vars, bootstrap log, and three serial logs were copied before the run to
  `out/evidence/m22-before-sweep-rerun-20260930/`.
- After the M21 `file.search` guest composition, the three-boot `./nagi m22`
  regression passed again on 2026-09-30. Each boot passed M19/M21 Search before
  the NH16 state check. Previous accepted M22 images, data disk, vars, and logs
  were preserved at
  `out/evidence/m22-before-m21-action-regression-20260930/`.
- A fresh-disk `./nagi m22` run on 2026-09-30 passed the guest M21 `file.move`
  Plan/Validate/Execute action on boot 1. It reopened the three-object NH16
  Committed transaction and a separate NAL1 Committed record, then boot 2
  applied reverse-order Undo and persisted NAL1 `UndoPending`/`Undone`. Boot 3
  reopened both archives and verified the original files plus complete NAL1
  outcome history. Fresh-disk serial logs and their pre-run images, persistent
  disks, OVMF vars, and logs are preserved under
  `out/evidence/pre-m22-ai-activity-ledger-m28-20260930/`. The subsequent M28
  repetition's latest M22 serial logs remain in `out/logs/`.
- This remains a deterministic guest acceptance fixture, not real AI
  inference, authenticated production capability authority, or a separate
  production Activity Ledger integration. NH16 preserves AppId, AppSessionId,
  NodeId, SurfaceId, WorkspaceId, ObjectIds, and TransactionId for this fixture
  caller.
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
- The NH16 host tests verify its serialization and recovery contract. NAL1
  host tests verify its separate archive/store contract, and `./nagi m22`
  verifies the fixture's guest VFS ledger persistence across restarts. These
  do not prove authenticated policy, production M21 file mutation, real AI
  inference, or a production Activity Ledger service.

M22 is `PARTIAL`. The NH16 archive, a real guest `file.move` Executor action,
grouped three-file transaction, separate NAL1 activity persistence, composite
Undo, and restart verification now pass through a private QEMU fixture. Real
AI inference, authenticated caller and capability providers, general
production move actions, and production Activity Ledger/restart acceptance
remain incomplete, so the formal milestone is not PASS.
