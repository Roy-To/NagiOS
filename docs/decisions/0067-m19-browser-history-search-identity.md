# ADR 0067: M19 Browser history identity and Search publication

Status: accepted
Date: 2026-10-07
Milestone: M19 — Semantic Layer / Search
Builds on: ADR 0044 (authenticated Search IPC), ADR 0046 (Supervisor grants), ADR 0057 (stable producer identity)

## Context

M19's guest Page acceptance used a literal `ProducerObject`; it did not read
Albert's persisted visit history. Albert already stores each committed HTTP(S)
visit as a `HistoryEntry` with a browser-local entry ID, URL, title, and
monotonic tick value. That entry ID is not a global `ObjectId`, and browser
storage can be reset independently of the Search index.

## Decision

1. **Source and granularity.** A Page represents one committed HTTP(S)
   `HistoryEntry` visit. Repeated visits to the same URL remain distinct.
   Page metadata contains only the title and URL; page body, selection, and
   other web content are never published. The Browser profile has a persisted
   nonzero namespace ID. The producer key is `(Albert AppId, profile ID,
   HistoryEntryId)` and is separate from `ObjectId`.
2. **Stable Object IDs.** The trusted Search producer allocates the `ObjectId`
   and stores the producer key in the same durable metadata record. A
   subsequent publication for that key reuses the stored ID. The Search
   service's allocator considers existing records, including tombstones, so
   it does not reuse an ID after deletion. A profile reset receives a new
   random namespace from Nagi's RNG; failure to obtain one disables indexing
   for that run without blocking browser navigation or history persistence.
3. **Publication and lifecycle.** The producer runs after navigation history
   has been committed. It indexes the newest bounded set that fits the M19
   Search snapshot, updates records by producer key, groups visible visits in
   the logical Albert History Workspace, and tombstones records no longer in
   the published set. Monotonic `visited_at` ticks are not copied into Search
   wall-clock fields.
4. **Authority.** Search requests require a live `search.query` grant. File
   records additionally require `files.search`; Albert Page records
   additionally require `albert.history.search`. The manifest and owner's
   consent are checked through the Supervisor launch record. Unknown,
   foreign, and revoked callers receive no private metadata. The Search
   database stays behind init's existing User Data block capability; no
   kernel file or search syscall is added.
5. **Compatibility.** Existing M19 fixture records remain usable by the
   acceptance harness. Browser session records gain a versioned profile ID;
   version-1 sessions restore with no ID and receive a new one before their
   next persisted visit.

## Consequences

- Browser history is private by default and only a consented Search client
  with the dedicated Albert history grant can discover it.
- The first integration uses the actual persisted `BrowserState` after the
  M18 QEMU scenario, the guest VFS-backed SearchService, and isolated
  `search@1` clients. Its bounded acceptance publication keeps the original
  `example.com` history entry alongside the three newest visits, so the
  restart check searches for that same producer key and Object ID even when
  M18 creates new visits during the second boot. It does not publish page
  content or let clients choose producer keys or Object IDs.
- The guest metadata snapshot is intentionally bounded. The publisher must
  report capacity failure and preserve normal browsing; increasing capacity
  requires a separately verified VFS/storage change.

## Verification

`./nagi m19` will exercise the same persisted HTTPS Browser history entry,
stable Search Object ID and Workspace membership across the following QEMU
restart, and the source-specific Search grant against authorized and foreign
isolated clients. Host tests cover session-format migration and producer
identity.
