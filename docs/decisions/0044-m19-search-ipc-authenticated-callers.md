# ADR 0044: M19 SearchService over `search@1` with launch-record authority

Status: accepted for M19
Date: 2026-10-03
Milestone: M19 — Semantic Layer / Search
Builds on: ADR 0043 (isolated app process and kernel-stamped identity)

## Context

The M19 guest SearchService ran only in-process inside `nagi-init`. It
answered queries for a hard-coded fixture `AccessContext`. M19 remained
`PARTIAL` because Search was not exposed as an IPC service with an
authenticated, capability-bound caller context. ADR 0043 now lets the
Supervisor spawn a real ELF into its own address space. Every Channel message
from that process carries its kernel-stamped Process ID.

## Decision

1. **Wire format.** Add `crates/nagi-search-ipc`, an allocation-free
   `search@1` codec:
   - **Request:** a kind filter plus a UTF-8 query of at most 126 bytes.
   - **Response:** a status, a visible-match total, and up to 15 Object IDs.
   - **Single message:** each direction fits one inline Channel message.
   - **No identity field:** a request cannot claim an identity at all.
   - **Strict decoding:** malformed, truncated, trailing, or
     non-UTF-8 payloads are rejected.
2. **Authority.** The init-hosted service resolves each request's caller
   only through the Supervisor launch record:
   `sender_process_id -> AccessContext(AppId, AppSessionId)`.
   - With no record, it answers `UnknownCaller` without reading the index.
   - Otherwise it runs the normal `SearchService::search` with the resolved
     context, so the existing `VisibilityFilter` decides what is returned.
3. **Clients.** `nagi-m19-search-client` is a separate isolated ELF in
   `user/nagi-isolated-app`. It uses only raw Channel syscalls and the
   `search@1` codec, and it has no device capability.
4. **Acceptance.** The `m19-search-ipc` init feature, now used by
   `./nagi m19`, launches the same client ELF twice with the same query:
   - launched as the M19 application session, it must receive exactly the
     live VFS file's stable Object ID;
   - launched as a foreign application, it must receive zero visible matches;
   - a sender without a launch record must get `UnknownCaller`.

## Bounds and non-goals

- One isolated client at a time (ADR 0043's single slot). Clients run
  sequentially; the slot is reused after each exit.
- The launch record lives in the acceptance Supervisor code. A general
  Supervisor launch registry and manifest-driven app identity are later work.
- The M21 `file.search` action path still uses its fixture `CallerIdentity`.
  Moving M21/M22 action callers to launch-record identity is the next step.
- Files and Browser producers are still not live sources for the index.

## Verification

- `cargo test -p nagi-search-ipc` covers the codec round trip and rejection
  cases.
- `./nagi m19` passed locally on QEMU/OVMF with a fresh User Data disk on
  2026-10-03. Bootstrap, initial, and restart boots all ran. Both boots
  printed:
  - `Nagi M19 Search IPC authorized isolated client PASS`
  - `Nagi M19 Search IPC foreign isolated client hidden PASS`
  - `Nagi M19 Search IPC authenticated caller PASS`
