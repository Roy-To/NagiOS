# nagi-search-core

SEARCH-CORE-01 host-side Foundation for the Nagi 0.2 Search platform
(0.2-M19). Status and evidence: `.dev/workstreams/search-core-01/state.json`.

## Relationship to the canonical Search contract

The canonical Search/Index contract is `nagi-search`, owned by the active
Nagi 0.1 M19 Search line. Its object record (`MetadataRecord`), query
(`SearchQuery`), hits, Workspace grouping, caller visibility seam
(`VisibilityFilter`), snapshot backend (`SnapshotBackend`), and ranking are
**reused, not redefined**. Canonical IDs come from `nagi-model`.

`nagi-search` is not yet present on the 0.2 integration base, so this crate
pins both crates to one exact 0.1 source revision through a Cargo git
dependency (see `Cargo.toml`). When the Integration Owner brings
`user/nagi-search` onto the integration base, the git pins become path
dependencies; no source change is required.

## What this crate adds

| Concern | Mechanism |
|---|---|
| Trusted provider identity | `AppId` passed by the integration layer; payload `source_app` must match or is bound to it (`SourceMismatch`) |
| Provider authorization | injected `ProviderAuthority`, re-checked on every write; `DenyAllProviders` default |
| Provider isolation | per-object ownership ledger; foreign writes fail, foreign removes look like unknown objects |
| Incremental updates | `DocumentRevision` (non-zero, strictly increasing); stale, duplicate, and conflicting replays are classified |
| Deletion propagation | revisioned `remove`; `unregister_provider` tombstones every live object it owns |
| Source scoping | `ProviderScope` applied inside the canonical visibility check, i.e. before matching, ranking, and limits |
| Result provenance | every hit carries provider and revision |
| Versioned persistence | `NSCL` v1 ledger snapshot with FNV-1a checksum; unknown versions and corrupt input are rejected |

Caller visibility is still decided only by the injected canonical
`VisibilityFilter`; this crate does not implement or fork the
Capability/Permissions policy engine.

## Not included (gated)

No daemon, IPC endpoint, production/guest persistence, provider adoption by
Files/Notes/Activity/Wayback/Albert, Home/Search UI, or semantic/model path.
These require Nagi 0.1 M30 PASS and an explicit Integration Owner checkpoint.

## Verification

```text
cargo fmt --manifest-path crates/nagi-search-core/Cargo.toml -- --check
cargo clippy --manifest-path crates/nagi-search-core/Cargo.toml --all-targets --locked --offline -- -D warnings
cargo test --manifest-path crates/nagi-search-core/Cargo.toml --locked --offline
```

The first build needs network access once to fetch the pinned git revision;
afterwards `--offline` works from the Cargo cache.
