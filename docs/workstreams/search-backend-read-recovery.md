# Search backend read allocation and uncertain-commit recovery

Status: PARTIAL. Local candidate based on main
`d67897b4c27566ecc4353e3b7d9b16efd59bb559`. M19 and M24 acceptance are unchanged.

## Problem and behavior

The two-slot `GuestSnapshotBackend` loaded manifests/chunks with infallible
slice-to-Vec copies and assembled snapshots with `Vec::with_capacity`.
These now reserve fallibly and return `BackendError::Capacity`. Allocation
failure is propagated, rather than being treated as a corrupt slot and silently
selecting an older generation. The host reference backend also uses bounded
read buffers and fallible growth, with a distinct capacity error.

A manifest write can return an error after taking effect. The final flush can
also fail either before or after persistence. Previously both looked like a
failed mutation, while the store retained its old memory state and could send
another complete snapshot derived from that state, losing a possibly committed
update. A host negative control reproduces acceptance of that next mutation.

Manifest-write errors and final-flush errors now return
`BackendError::RecoveryRequired`. The backend latches that state before the
manifest call. All later writes on that instance stop before storage callbacks.
The metadata store and persistent vector index also latch the error and reject
every mutation, including nominal no-ops, before cloning or encoding old state.
The vector boundary exposes `VectorIndexError::RecoveryRequired`; the service's
generic semantic adapter maps it to its existing `IndexUnavailable` error.

`requires_reload()` exposes whether the last confirmed in-memory read view may
be stale. `reload()` stages an entire new state and installs it only after load
and decode succeed. Failed flush, read, allocation or decode leaves mutations
blocked and the last confirmed view intact. Callers must not mistake that view
for the current committed state.

On the latched backend, load first retries the flush fence, then inspects both
slots. A failed fence or load keeps its latch set. Only a successful load clears
the backend latch; the owning store additionally requires successful decode.
Chunk writes and their pre-manifest flush retain their previous ordering and
errors. Their failures do not publish the candidate manifest.

No NSG1 or NVS1 bytes, version, checksum, generation ordering or slot selection
rules change. These errors are local API distinctions, not a format migration.

## Host evidence

`user/nagi-search/tests/backend_recovery.rs` exercises the real backend
algorithm with an allocation-free file-store fixture containing separate visible
and durable byte arrays. It is a deterministic fault model, not real storage.

- Negative control on the original main source: denial of the first 36-byte
  manifest copy aborts with SIGABRT. A separate original-source test accepts the
  next metadata mutation after a final-flush failure and therefore fails its
  write-blocking assertion.
- All 49 observed load allocation positions across 13 guest-backend/host-reference
  cases are denied in both one-shot and persistent modes (98 null-denial probes).
  Another ten allocation-free denial probes complete normally. Failed partial
  loads release their allocated bytes; retries return the expected snapshot or
  original error. Normal guest-backend load probes leave both images unchanged,
  with write/remove/flush callback counts all zero.
- The two-slot write preflight has 11 allocations. All 22 denial probes return
  Capacity without file mutations or flushes and succeed after denial is removed.
- Recovery tests cover manifest failure before/after its visible effect and
  final flush failure before/after its durable effect. Metadata, vector and
  direct-backend users reject subsequent writes with unchanged bytes and callback
  counters after the uncertain failure. Failed recovery fences, reads, OOM and
  malformed decoded payloads keep the relevant guards latched. Successful reload
  installs the selected state and allows subsequent mutations.
- A pre-manifest flush failure remains retryable without candidate publication.
  Host reference tests retain missing/empty distinctions, the 16 MiB limit and
  I/O errors, and preserve file contents and modification time during load.
- Isolated exact-source host suite: 42 Search unit tests, 9 new integration
  tests and 3 Model tests pass. The existing opt-in timing test and the allocation
  child entry point remain ignored by the normal runner; the parent explicitly
  executes the allocation child for every probe. Warnings-denied all-target
  Search Clippy and the Nagi user target library check pass.

The source patch also applies without conflicts to #42 head
`d817c0a619fb035a65e8096016de4f89a2e1e36b`; its isolated 30 Search unit,
9 new integration and 3 Model tests pass. An isolated compatibility overlay on
reviewed candidate `8965e6f941a0840819cd5849d321c0a7da3a617f` preserves its
fallible embedding clone and Vec-based state. Its different `remove_object`
body needs the same guard inserted ahead of its binary search, rather than the
main patch's BTreeMap context. That overlay passes 38 Search unit, 9 new
integration, both existing decode/update allocation parent tests and 3 Model
tests. These checks do not publish or adopt either integration on a public ref.

## Limits

Normal unlatched backend loads are read-only. A recovery load deliberately
flushes pending writes, so it may change durable bytes even if a later allocation
or decode fails. No rollback or byte-invariance claim is made for the original
uncertain commit or its recovery fence. Subsequent blocked mutations do not
change the bytes that exist after that failure.

The latch belongs to each retained instance. Dropping/reconstructing the backend
or extracting its file store does not preserve it as a durable marker. This is
not multi-instance writer coordination, a storage ownership protocol or crash
recovery evidence. Callers retaining an uncertain instance must use its recovery
path rather than discarding the requirement.

The file-store/VFS adapter's own allocations, metadata decoding/BTree updates,
provider/caller allocations, complete startup OOM behavior and final native
linking remain outside this change. Earlier semantic update/decode candidates
are separate work; this main-based slice does not claim their adoption.
VM/guest execution, real-media durability, power-loss/atomic-sector behavior,
writer exclusion, publication and permission changes were not performed.
