# PR42 source integration on current main

## Approved integration
Base: main d67897b4c27566ecc4353e3b7d9b16efd59bb559.
Source candidate: 321d1b89997fa0e1357df47f5ab6e2b3a83ff656.
Public PR42: d817c0a619fb035a65e8096016de4f89a2e1e36b (Draft).

Apply only c170ebf1..321d1b89's source/test/doc/workflow increment to current main, using three-way integration. Preserve main's QMP, fetch diagnostics, Search allocation/read-boundary changes, CI change-scope guard, Calendar regressions and M27 evidence retention. Combine both independent status histories. Keep both the Files runtime synchronization gate and main's fallible title allocation API. Retain borrowed object comparison and fallible scalar stale-record scanning from main; do not restore cloning collection APIs.

This is a normal single-parent source commit on current main, not ancestry integration, publication or a merge commit. Frozen candidates, shared branches and public head remain unchanged. Public-head Files/Trash/UI/Recovery/diagnostic feature accounting remains documented in the prior integration spec. The obsolete writable-buffer workaround stays replaced by the kernel buffer-direction contract and readable-buffer adapter coverage.

## Verification
Use pinned nightly-2025-08-01, offline jobs=1, existing Mac host/Rosetta only and dedicated SSD artifacts. Run formal libnagi tests, Recovery tests, actual-source Runtime/Files/Search host harness (including diagnostic features), changed Search tests, CI Python gates, and focused strict Clippy/fmt. Add a direct regression combining unsynchronized refusal with main's title API after synchronization. Compare main-only source blobs, QMP fixture, fetch-progress and Search code byte-for-byte; inspect combined workflow requirements.

## Limits
Independent review, Linux/hosted CI on this new tree, production nagi-init target/guest and media acceptance are pending. No target build, VM, real-volume repair, transfer/upload, public changes or process-shared ownership retry. Unknown sector-zero compatibility, cross-process coordination, arbitrary media persistence, partial replace/write outcomes and pre-existing damaged journal limitations remain unresolved. Earlier candidate results are not evidence for this new tree.

## Integration result
The title method retains its synchronized guard and delegates to try_workspace_file_titles; stale-record enumeration uses try_stale_producer_records and metadata comparison uses get_object_ref. The direct production-feature regression requires None before first synchronization and after a consumed injected read error, then the same title/identity after a successful retry. Existing Search allocation-failure tests run unchanged from main. A test-only const thread-local initializer addresses a standard Clippy lint; production lock logic is unchanged.

The new test initially passed ObjectId values to the u64 UI interface and did not compile. Its explicit identity conversion is now compiled and executed. Package formatting passes; the wider --all traversal reaches pre-existing Servo/relibc formatting differences and is recorded as failed, without rewriting dependencies. No old host-test denial is re-executed or bypassed; this is the specifically authorized host-only successor run.
