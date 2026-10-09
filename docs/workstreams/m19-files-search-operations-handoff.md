# M19 ordinary Files/Search operations handoff

Historical Files base: `edad2e7a87aa0fe08982a5c5249bc4a3ad7de7d4` (PR #31 merged).
Historical owner branch: `codex/0.1-files-search-operations`.
Publication candidate base: `ef217b30c6074833ed81ff7a9a20e1a8fb0ed8b8`.
Proposed publication branch: `codex/recovery-files-vfs-20261009` (not published).

Owned changes: `desktop.rs`, `m19_runtime.rs`, new Files/panel/test leaves,
Files-only entries in the shared en-US/ja-JP guest catalogs, dedicated
`tests/files-search`, ADR 0070 and M19 status/handoff/own state. The 0.2 Search State,
Writer/Sheets, model and voice fixtures are untouched. No changes to main.rs,
any shared Cargo/lock file, CLI, CI, acceptance Registry or image generation.

## Implemented seam

`m19_runtime::files::{initialize,list,trash_entries,create,rename,trash,restore}`
accept the real `Vfs<D: BlockDevice>`. Mutation selections are `Entry { name,
inode, generation }`, not unchecked paths. `Error` distinguishes invalid
names, conflict, stale selection, capacity, corrupt trash metadata, storage
failure and uncertain durability. The UI remains signed-in owner-only and
uses the existing Search route on query submission. F2 while Files is focused
or the Files F2 button opens management. Tab/Enter, Up/Down and mouse controls
support creation, renaming, trash view and restoration; Escape cancels editing
or closes the panel. A repeated Enter after completion does not mutate the
next list entry. Trash/restore behavior is documented in ADR 0070.

## Proposals for shared owner

1. Register `m19-files-search-operations` with branch above and own
   `.dev/workstreams/m19-files-search-operations/state.json`; do not replace the
   unrelated 0.2 `search-core-01` State. This branch does not edit the Registry.
2. Add a host CI step (Ubuntu and Windows where feasible):
   `cargo fmt --manifest-path tests/files-search/Cargo.toml -- --check`,
   `cargo clippy --manifest-path tests/files-search/Cargo.toml --all-targets --locked -- -D warnings`,
   `cargo test --manifest-path tests/files-search/Cargo.toml --locked`.
   It is standalone so root Cargo registration is unnecessary.
3. Extend the existing login QMP scenario before password-change completion:
   focus Files, F2, Tab -> New, Enter, type `ui-file.txt`, Enter; rename the
   selected file to `ui-renamed.txt`; trash it; inspect the Trash view; restart
   the same disk; restore and query through the signed Files child. Assert
   `Nagi Files UI operation PASS operation=create|rename|trash|restore`, Search
   exclusion while trashed, retained ObjectId on restoration, conflict UI,
   repeated activation and child cleanup. Existing lifecycle fixture markers
   do not prove these new normal controls were exercised.
4. A production image currently requires shared
   `execute_image_with_m19_files_search_product`. For an additional reusable
   acceptance entry point expose the ordinary feature set
   `desktop-login,m19-files-search-production` separately from login fixtures.

## BrowserHistory ownership/API proposal

Actual ordinary callbacks in `user/nagi-albert/src/lib.rs` currently only log
URL/load changes; they do not commit ordinary navigation to `BrowserState` or
publish it. The `FnOnce(&BrowserState)` callback in `m18_acceptance.rs` and
`m19_search::run_with_browser_state` are acceptance-scoped. Editing those
fixtures alone cannot establish normal history synchronization.

Please assign/confirm the Albert leaves (`lib.rs`, `browser_state.rs`,
`persistence.rs`/`nagi_storage.rs`) to a Browser owner before editing them.
Suggested contract: an OS-owned bounded callback after durable history commit
and load/reopen, carrying the profile namespace and current stable history
entry IDs, URL/title metadata, and explicit removals. The Search owner can
implement a generic producer reconciliation leaf, keyed by
`(AppId=org.nagi.albert, profile namespace, HistoryEntryId)`, retaining ObjectIds
and the private profile Workspace. Page queries must continue to require live
`search.query` plus `albert.history.search` and profile visibility. Empty
snapshots must reconcile removals; stale/missing namespaces fail closed.
Search failure must not fail browser navigation. Shared startup/service
routing and a normal Browser callback belong to the shared/Browser owners.
No proposed interface grants web content filesystem or Search authority.

Model and voice providers are outside this change; no fixture expansion or
provider-specific interface is introduced.

## Recovered VFS scope and evidence limits

The recovered storage leaf implements bounded checked undo recovery for regular-file
create and same-directory rename. Temporary Files capacity guards are removed.
The dedicated fixtures cover the prior create alias and crowded-directory interruption
paths. See `m19-vfs-create-recovery-proposal.md` for the journal, compatibility,
exclusive block authority and read-only RecoveryRequired contract.

Imported component evidence is preserved privately and is not included in this
publication candidate. It does not establish the claimed old local commit
identities. Fresh recovery validation is recorded separately. Normal guest
UI/Search operations and Browser History synchronization remain pending; M19
stays PARTIAL.

## Fresh recovery validation

24 Files tests, 93 libnagi unit plus 2 renderer tests, 40 Search/IPC/localization
regressions, focused formatting and warnings-denied Clippy passed. The actual
exact-base VFS wrote a synthetic 8 MiB disk accepted by the recovered read-only
checker/mount/read_at with zero writes and flushes and identical bytes. Clean
recovered create/rename output remained readable by the exact-base VFS. No real
User Data was formatted. Pending undo requires the new recovery path; old-reader
compatibility is claimed only for clean completed state.

Native target compilation and normal guest UI/Search/restart acceptance remain
blocked or unverified. Existing imported target/VM logs are historical evidence.
