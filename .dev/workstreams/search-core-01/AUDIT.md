# SEARCH-CORE-01 pre-implementation audit

Audit date: 2026-10-02. Base: `codex/integration-next-phase` at
`a235eb9f7d6c15a71873b431a57271cd1aeba260` (current remote HEAD at audit time).

## Inputs

| Document | SHA-256 | Location |
|---|---|---|
| `Nagi_OS_0.2_Codex_Implementation_Spec.md` | `096eeba4d4678e29130bbdd6c0250ba250aa3706d8f43a3e77b1b2a51634f3c3` | user-supplied; not yet in repository |
| `Nagi_OS_0.2_Parallel_Execution_Rules.md` | `3c5ec3f1b30cd138a9028d324bdaac19233c87e45a72d862bafed833a9436129` | user-supplied; not yet in repository |
| `Nagi_OS_0.2_SEARCH_CORE_01_Workstream.md` | `df14f223cded984ffb950ccd0cc743c9f83c4e8c7fdce6a6440eefd78dd791c7` | stored as `docs/workstreams/NagiOS_0.2_Search_Index_Core_Foundation_Workstream.md` (content verbatim; only the trailing blank line removed for `git diff --check`) |

Relevant master-spec sections: §8 identity, §10.2 `search.provider`
capability, §25 Search, §26 SDK Level 1, 0.2-M19 (Search / Index / Workspace
Platform), 0.2-M26 (Home + Search apps), §30.3 (0.1 M19 Search is the
predecessor), §37 (migration preferred to duplication).

## Duplicate-branch check

`git ls-remote` after `git fetch --prune`: no `codex/0.2-search-core-01` and
no `search-core` ID or text on any remote branch or in any
`.dev/workstreams.json`. A new branch was therefore created; nothing was
duplicated.

## Existing Search implementations (read-only inspection)

| Location | Contract | Owner / state | SEARCH-CORE-01 decision |
|---|---|---|---|
| `user/nagi-search` on `codex/m19-m22-continuation` (`de3092c6…`) | Canonical metadata index: `MetadataRecord`, `SearchQuery`, `SearchHit`, `SearchResponse`, `VisibilityFilter`, `AccessContext`, `SearchService`, `SnapshotBackend`, Workspace/Relation, semantic boundary | Active Nagi 0.1 M19-M22 line | **Reuse via pinned git dependency.** Never modified, copied, or redefined. |
| `user/nagi-search` on `codex/m19prep-semantic-search` | Earlier version of the same crate | 0.1 M19 prep | Superseded by the line above; not used. |
| `crates/nagi-model` (all branches) | `AppId`, `ObjectId`, `WorkspaceId`, `AppSessionId` | Shared canonical IDs | Reused from the same pinned revision so types unify. |
| `user/nagi-home-search` on `codex/integration-next-phase` | Query-time `SearchCoordinator`, string `ProviderId`, `SearchProvider` trait, cancellation, UI result model | M26 Home/Search app track | Not reused as a dependency (platform core must not depend on an app crate). Its `ProviderId` and SEARCH-CORE-01's `AppId` provider identity must be reconciled at integration; see proposal. |
| `apps/nagi-files/src/search.rs` on `codex/integration-next-phase` | Files-local search provider for Home/Search | Files app track | Untouched; future adopter of this ingestion contract (gated). |

Searches performed for `SearchDocument`, `Index`, `Query`, `ObjectId`,
provider, ranking, and result types found only the entries above.

## Ownership boundary decision

The master spec expects option 1 of the workstream doc §2 (**reuse through an
adapter**): 0.2-M19 lists "predecessor M19 Search code" as a primary input and
§37 forbids rewriting it. The canonical record/query/visibility/ranking types
belong to the 0.1 line. SEARCH-CORE-01 therefore owns only what the 0.1
contract does not define and what 0.2-M19 requires of a *provider-based*
service:

1. provider identity binding and authorization seam for writes;
2. per-object provider ownership/isolation;
3. revisioned incremental upsert/remove with stale/duplicate/conflict rules;
4. provider-scoped query that is applied before ranking and limits;
5. provider/revision provenance on results;
6. a versioned, checksummed ledger snapshot.

Explicitly not owned here: record fields, lexical matching or ranking (0.1),
body-text indexing (would extend the canonical record; proposal only),
Workspace APIs (0.1 canonical), semantic search (0.1 M19/M24), caller policy
(Capability/Permissions), the query-time coordinator (M26 Home/Search).

## Independence from active 0.1 work

No file under `user/**`, `kernel/**`, `loader/**`, `third_party/**`, any 0.1
milestone path, any shared schema/IDL/ABI, root `Cargo.toml`/`Cargo.lock`,
`.github/workflows/**`, or `.dev/workstreams.json` is changed. The 0.1 crate
is consumed read-only at a fixed revision.
