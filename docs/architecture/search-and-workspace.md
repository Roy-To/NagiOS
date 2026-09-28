# Search and Workspace Foundation

The M19-PREP implementation lives in the isolated `user/nagi-search` crate.
It reuses `nagi-model::ObjectId`, `WorkspaceId`, `AppId`, and `AppSessionId`;
the shared newtypes gain ordering/hash traits only. A metadata record is keyed
solely by `ObjectId`. Its optional `location` is descriptive producer data,
never an identity or lookup key.

## Persistence boundary

`SnapshotBackend` is the persistence interface. The crate's version-1 format
is a bounded deterministic binary snapshot with a magic value, explicit
version, payload length, and checksum. Records, relation edges, Workspace
membership, and app-session references are persisted together. Malformed,
truncated, oversized, checksum-invalid, unsupported-version, and broken
reference snapshots fail closed.

`HostFileBackend` is compiled only outside `target_os = "nagi"`. It provides
atomic host-file replacement for host-side restart and corruption tests. It is
not a Nagi VFS implementation or evidence of guest persistence. The guest VFS
at this baseline is block-backed and its file payload limit is 1 KiB; a later
guest adapter needs bounded chunking/multiple files or a separately reviewed
storage change.

## Relations and Workspace

Relations are directed `ObjectId` edges with explicit user/application/AI
suggestion provenance. Duplicate insertion is idempotent. Object deletion
retains a tombstone and removes its incident relations and Workspace
memberships. Re-indexing the same `ObjectId` revives the metadata record but
does not recreate those links.

Workspace identity is the existing `WorkspaceId`, independent of paths,
process IDs, windows, surfaces, or node-local layout. A Workspace can contain
logical `(AppId, AppSessionId)` references and many `ObjectId`s; one object may
belong to multiple Workspaces.

## Search and authorization

Search provides case-insensitive UTF-8 literal exact/prefix/substring title
matching, tags/attributes, kind/source filters, inclusive created/modified/
observed ranges, direct relation filters, Workspace title search, owner-app
filtering, and Workspace grouping. Ranking and tie breaks use documented stable
rules. Every result carries match rationale; AI-suggested relation provenance
stays visible in relation rationale.

Every `SearchService` instance requires an injected `VisibilityFilter`; the
default available implementation denies all. The filter's caller context is
descriptive and is not itself authority. A production adapter must bind it to
the trusted caller and capability state. Objects and Workspaces are filtered
before matching, result limiting, rationale construction, and grouping.
Workspace member objects and app-session references are filtered again before
they are returned. Unknown and denied IDs use the same empty/absent result.

Files, page/history, and Workspace producer adapters map source summaries to
the shared model. They do not pull Albert, Files UI, networking, Servo, or AI
runtime into this crate.
