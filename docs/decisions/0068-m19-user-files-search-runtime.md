# ADR 0068: M19 user Files search runtime

Status: accepted
Date: 2026-10-08
Milestone: M19 — Semantic Layer / Search
Builds on: ADR 0044 (authenticated Search IPC), ADR 0046 (Supervisor grants), ADR 0057 (stable producer identity), ADR 0067 (Browser History identity)

## Context

M19's guest acceptance proves persistent file and page search, but its file
publisher scans a fixed acceptance fixture. The normal M10 desktop currently
has a mock Files panel and stores owner credentials and system preferences in
the root of the User Data volume. Indexing the whole root would risk publishing
security and system metadata as ordinary user files.

## Decision

1. **User file namespace.** Owner documents live below
   `/home/owner/files/` on the User Data VFS. Search indexes only regular files
   in that directory. It never scans the volume root, `/var/lib`, credentials,
   preferences, or file contents. Search text metadata is UTF-8, so a file
   whose name is not valid UTF-8 is omitted from text search while the runtime
   remains available. The initial one-owner namespace matches the current M29
   account model; account-specific roots can replace it when M30 adds multiple
   accounts.
2. **Runtime placement.** After owner sign-in, init opens the existing
   `SearchService` in user space. Its bounded two-slot snapshot stays on the
   same User Data capability under `/var/lib/nagi-search`. No Search or file
   syscall is added to the kernel.
3. **File identity and lifecycle.** The Files producer key is the VFS
   `(inode, generation)` pair. Search allocates and persists the ObjectId for
   that key. Rename updates descriptive metadata without changing the ObjectId;
   deletion tombstones it; inode reuse receives a different ObjectId. The
   runtime reconciles only the dedicated user Files directory and reports a
   capacity error instead of silently dropping records.
4. **Authority.** Owner-facing search receives a private init-created access
   context only after sign-in. It can read only the Files producer's private
   records and Workspace. Other application callers continue to require the
   signed Supervisor launch record and explicit `search.query` plus
   source-specific grants before M19 IPC exposes records.
5. **Scope.** This ADR does not grant applications blanket User Data access or
   make the Owner role a permanent root capability. Browser Pages continue to
   use the source-specific grant and profile identity defined by ADR 0067.

## Consequences

- Search can follow Files lifecycle changes without exposing root-level
  account or system metadata.
- Existing root-level M19 acceptance fixtures remain isolated and are not
  migrated automatically.
- The initial desktop has one owner; per-account directory selection is
  required before multi-account search can be enabled.

## Verification

Implementation must verify ordinary desktop startup after sign-in, bounded
index persistence across restart, the specific acceptance file's ObjectId
restoration, ObjectId stability across rename, tombstone on deletion, a new ID
after inode reuse, and denial of root-level system metadata. It must also
verify that a non-UTF-8 filename does not disable Search. M19 remains `PARTIAL`
until these guest checks and Browser lifecycle integration pass.
