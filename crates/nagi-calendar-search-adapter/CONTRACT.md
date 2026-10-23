# Calendar Search host adapter contract

Scope: first-party specification §61.9 / CAL-009, 0.2 §25 and
SEARCH-CORE-01. This is a host foundation, not CAL-009 product acceptance.
The existing Calendar and Search sources remain unchanged.

A trusted integration caller injects the Calendar provider identity and a stable
EventId -> existing ObjectId resolver. No ObjectId allocator or Search schema is
introduced. Live Calendar revisions become canonical DocumentRevision values. Calendar
EventStore::delete returns the last live revision unchanged; remove accepts that
last live value and converts it to a checked successor for the Search tombstone.
Identical remove retries map to the same tombstone and return AlreadyRemoved.
Stale deletes cannot replace newer live records. Deleted EventIds/ObjectIds must
not be reused; actual Calendar recreation receives a fresh ID and mapping.

Integration must call deletion_revision before deleting from Calendar. At
u64::MAX it returns DeletionRevisionOverflow without mutating Search; integration
must retain the live Calendar event rather than deleting it. u64::MAX-1 can be
removed at tombstone u64::MAX. There is no saturating or wrapping conversion.
This bridge is not a transaction across Calendar and Search: integration must
retain and retry failed index operations. Delete-before-first-index and dirty
ledger recovery remain inherited SearchCore integration limitations.
Projection uses ApplicationData, title, location and participant tags; the
canonical MetadataRecord validator rejects overlong fields without truncation.
Descriptions/notes, dates, workspace membership and runtime integration are
outside this first bounded projection and are not claimed as complete §61.9.
No sensitive notes are indexed. The caller's canonical VisibilityFilter applies
before matching, ranking, result limits, counts and metadata lookup. Publish
permission is rechecked by SearchCore for every upsert/delete. The adapter
accepts no payload provider identity and does not register runtime providers.

Negative tests precede implementation: identity/type convergence, invalid fields,
unmapped identity, rejected read disclosure, update/delete, stale and conflicting
replay, provider spoofing and publish revocation. Focused Cargo tests, regressions
for Calendar/SearchCore/canonical Search, fmt and warning-denied Clippy run with
standalone manifests, an isolated SSD target and jobs=1. No shared manifest,
lockfile, registry, CI, IDL or ABI changes are permitted.

## Validation and review packet

Dependency metadata confirms exactly one local canonical nagi-model and one
nagi-search for Calendar, SearchCore and the adapter. The package-owned patch
and lock are sufficient; no shared-manifest exception is required.

Use an installed Rust 1.97.1 toolchain with its bin directory first on PATH
(Cargo, rustc, rustdoc and cargo-clippy must agree). The default host Homebrew
Rust 1.86 cannot compile the existing Calendar is_multiple_of calls. No
existing source was changed to accommodate the host toolchain.

Run with a dedicated SSD CARGO_TARGET_DIR and the official SSD launcher:

```sh
cargo test --offline --locked --manifest-path crates/nagi-calendar-search-adapter/Cargo.toml -p nagi-calendar-search-adapter -p nagi-calendar-core -p nagi-search-core -p nagi-search -j 1
cargo fmt --manifest-path crates/nagi-calendar-search-adapter/Cargo.toml -p nagi-calendar-search-adapter -- --check
cargo clippy --offline --locked --manifest-path crates/nagi-calendar-search-adapter/Cargo.toml --all-targets -j 1 -- -D warnings
```

Review focus: trust and lifetime of the injected stable/injective EventObjects
mapping; authenticated provider input; deny-before-disclosure policy; propagation
of SearchCore durability errors; canonical type convergence under the package
patch. Equal revisions compare the selected metadata projection, not omitted
Calendar notes or scheduling fields. Dates, workspaces, notes policy, runtime
registration, IPC, UI, guest/VM acceptance and product CAL-009 remain unverified
and out of scope. Existing GitHub CI is not disabled or skipped.


## Deletion revision regression

Independent review reproduced a real Calendar create/publish/delete/remove
failure against the original PR head: removal passed an equal revision and
SearchCore retained the live metadata with RevisionConflict. The adapter now
bridges the returned last live revision to its successor. The real-store test
covers create/update/delete, metadata/search removal, duplicate remove, delayed
replay and fresh-ID recreation. Boundary tests cover terminal overflow without
writes, the final usable successor, and stale removal. Calendar/Search sources
and the original independent review evidence are preserved.
