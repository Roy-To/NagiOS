# ADR-0014: Search Provider Identity and 0.2-M19 Search Ownership

- Status: Accepted for Nagi 0.2 host contracts
- Date: 2026-10-02
- Decided by: repository owner (recorded by the integration owner)

## Context

Two Search contracts identified providers differently:

| Contract | Owner | Provider identity |
|---|---|---|
| `crates/nagi-search-core` (SEARCH-CORE-01) | 0.2-M19 ingestion/index boundary | canonical `nagi_model::AppId`, supplied by the trusted integration layer |
| `user/nagi-home-search` `SearchCoordinator` | 0.2-M26 Home/Search app track | app-local string `ProviderId` |

The master 0.2 specification makes Search a provider participation contract
gated by the `search.provider` platform capability (section 10.2), and that
capability is held by an application. Two unrelated provider identities would
let the authorization subject and the routing label drift apart.

The master specification also lists `platform-apis` (unassigned) as a primary
workstream for 0.2-M19, while the registered SEARCH-CORE-01 stream already owns
the integrated Search/Index provider foundation.

## Decision

1. **Provider identity is the canonical `AppId`.** The authority subject for
   publishing into the Search index, for provider scoping, and for result
   provenance is `nagi_model::AppId`. It is derived from the trusted caller,
   never from payload fields.
2. **`ProviderId` in Home/Search is a label, not an identity.** When
   `nagi-home-search` adopts the SEARCH-CORE-01 ingestion boundary, each
   string `ProviderId` must map to exactly one `AppId`; it may remain as a
   display/routing label but must not be used for authorization. No second
   canonical provider ID type is added to `nagi-model`.
3. **SEARCH-CORE-01 owns the 0.2-M19 Search/Index provider boundary.**
   `platform-apis` keeps the remaining 0.2-M19 scope (Workspace model and
   membership/reference APIs) and the other platform contracts it is mapped to.
   Lexical matching, ranking, the metadata record, and Workspace persistence
   stay with the canonical `nagi-search` crate until it is moved onto the 0.2
   integration base.

## Consequences

- No code changes are required now; SEARCH-CORE-01 already uses `AppId`.
- Home/Search adoption (gated on Nagi 0.1 M30 PASS and an explicit checkpoint)
  must add the `ProviderId` to `AppId` mapping and a negative test proving a
  provider label cannot select another application's authority.
- An application that later needs several independent indexes must request a
  versioned contract change (for example, an index name scoped under its
  `AppId`) rather than a new top-level provider identity.
- This ADR does not open any runtime gate.
