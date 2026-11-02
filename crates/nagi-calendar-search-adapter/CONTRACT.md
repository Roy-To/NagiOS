# Calendar Search host adapter contract

Scope: first-party specification §61.9 / CAL-009, 0.2 §25 and
SEARCH-CORE-01. This is a host foundation, not CAL-009 product acceptance.
It adds no runtime service or guest wiring. Calendar and Search source changes
are limited to the already-reviewed snapshots and package-owned adapter.

## Projection and revisions

A trusted integration caller injects the Calendar provider identity and a stable
EventId → existing ObjectId resolver. No ObjectId allocator or Search schema is
introduced. Live Calendar revisions become canonical DocumentRevision values.
Calendar EventStore::delete returns the last live revision unchanged; remove
accepts that value and converts it to a checked successor for the Search
tombstone. Identical remove retries map to the same tombstone and return
AlreadyRemoved. Stale deletes cannot replace newer live records. Deleted
EventIds/ObjectIds must not be reused; actual Calendar recreation receives a
fresh EventId and mapping.

Integration must call deletion_revision before deleting from Calendar. At
u64::MAX it returns DeletionRevisionOverflow without mutating Search; integration
must retain the live Calendar event rather than deleting it. u64::MAX-1 can be
removed at tombstone u64::MAX. There is no saturating or wrapping conversion.
This bridge is not a transaction across Calendar and Search: integration must
retain and retry failed index operations. Delete-before-first-index and dirty
ledger recovery remain inherited SearchCore integration limitations.

Projection uses ApplicationData, title, location and participant tags. The
canonical MetadataRecord validator rejects overlong fields without truncation.
Descriptions/notes, dates, workspace membership and runtime integration are
outside this first bounded projection. No sensitive notes are indexed. The
canonical VisibilityFilter applies before matching, ranking, result limits,
counts and metadata lookup. SearchCore rechecks publish permission for every
upsert/delete. The adapter accepts no payload provider identity and does not
register runtime providers.

## Canonical type graphs

The standalone SearchCore manifest pins both nagi-model and nagi-search to
revision de3092c6a0125e45ea0b1f5f70c6d62c0c33f7fa. Its locked graph is tested
independently. In the adapter's package-owned manifest, the local patch maps
those dependencies to this base's crates/nagi-model and user/nagi-search.
Metadata and cargo tree checks must show exactly one nagi-model and one
nagi-search in each graph, with the adapter graph resolving both from those
local paths.

The main-versus-pin comparison adds Search producer metadata keys and APIs and
adds derives to an existing model ID; the fields and enum variants used by this
adapter remain compatible. The adapter does not adopt Search's current
PRODUCER_ID_ATTRIBUTE / PRODUCER_KEY_ATTRIBUTE, producer_records,
producer_object_id, or next_object_id contract. Its injected EventObjects map
has no persistence or producer-identity guarantee. Do not claim restart-safe
mapping based on these host tests.

## Toolchain, verification, and CI

Main has replaced the old is_multiple_of calls in Calendar time validation with
% checks and pins nightly-2025-08-01 for host CI. The previous Homebrew Rust
1.86 incompatibility note and Rust 1.97.1 recommendation are obsolete. Use the
pinned nightly for reproducible checks. For local runs, put its bin directory
first on PATH so Cargo, rustc, rustdoc, rustfmt and cargo-clippy resolve from
the same toolchain.

Run standalone manifests from the dedicated SSD worktree root with
CARGO_TARGET_DIR=$PWD/target/j1hosttest and CARGO_BUILD_JOBS=1. Keep root
Cargo.toml and Cargo.lock unchanged.

```sh
cargo fmt --manifest-path crates/nagi-calendar-core/Cargo.toml -p nagi-calendar-core -- --check
cargo test --manifest-path crates/nagi-calendar-core/Cargo.toml --locked --offline -j 1
cargo clippy --manifest-path crates/nagi-calendar-core/Cargo.toml --all-targets --locked --offline -j 1 -- -D warnings

cargo fmt --manifest-path crates/nagi-calendar-search-adapter/Cargo.toml -p nagi-calendar-search-adapter -- --check
cargo test --manifest-path crates/nagi-calendar-search-adapter/Cargo.toml --locked --offline -j 1
cargo clippy --manifest-path crates/nagi-calendar-search-adapter/Cargo.toml --all-targets --locked --offline -j 1 -- -D warnings

cargo test --manifest-path crates/nagi-search-core/Cargo.toml --locked --offline -j 1
cargo clippy --manifest-path crates/nagi-search-core/Cargo.toml --all-targets --locked --offline -j 1 -- -D warnings
```

The dedicated calendar-search-host workflow tests the adapter and pinned
SearchCore with standalone/package-scoped format, clippy, and test commands.
The adapter path-patched contract suite compiles canonical user/nagi-search
as a local dependency and checks the single-package type graph. A separate root
workspace test of user/nagi-search is not part of this host-only workflow: a
clean checkout needs the ignored third_party/cc-nagi source for the root Cargo
patch, and the official ./nagi fetch materializes many unrelated pinned
components as well. The path filter includes both package manifests, direct
local dependency sources, SearchCore acceptance test, root workspace files,
and the workflow itself. Existing host CI continues to cover Calendar core.

## Runtime integration HOLD

Do not wire this adapter or snapshot restore into runtime callers until each
item below has a separate design and regression coverage:

- Restoring an older snapshot can roll back Calendar revisions while newer
  Search content remains indexed; Search then rejects the restored older
  revision and stale content may remain.
- An EventId allocated after a snapshot can be reused after restoring that
  earlier snapshot, colliding with its old Search mapping.
- Restoring a live Calendar event after Search recorded its deletion leaves a
  Search tombstone inconsistent with the restored event.
- Calendar permits titles up to 4096 bytes and 256 participants of up to
  4096 bytes each; Search permits titles up to 1024 bytes and 128 tags of up to
  256 bytes each. The adapter rejects rather than truncates these values.
- EventObjects mapping persistence, trust boundaries, and reconciliation with
  Search's producer_* identity contract are unresolved.
- Authenticated provider injection and the required call ordering must be
  defined; deletion_revision must be preflighted before Calendar delete.
- Search failure retention, retry, and recovery semantics across Calendar and
  Search must be defined; this adapter is not a cross-store transaction.

This HOLD does not add epochs, a persistent ID ledger, durable guest storage,
VM acceptance, or changes to EventObjects persistence. Those remain outside
this host-only candidate.

## Review focus and covered checks

Tests cover type convergence and projection, invalid fields, unmapped identity,
denied read disclosure, update/delete, stale/conflicting replay, provider
spoofing and publish revocation, UTF-8 limits, reopen behavior, Calendar CRUD,
and deletion revision boundaries. Calendar core checks include snapshot
contract cases and existing availability/recurrence suites. SearchCore's pinned
graph and the adapter's local-patch graph are checked separately. The adapter's
contract suite currently includes 15 cases.

No shared manifest, root lockfile, IDL, ABI, runtime, or guest path changes are
part of this candidate. The dedicated CI addition is an explicit integration
change for host verification only.
