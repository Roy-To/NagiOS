# Nagi OS M24 — Embedding / Semantic AI

**Status: PARTIAL**

## Implemented foundation

- `user/nagi-search/src/semantic.rs` provides bounded UTF-8 chunking for
  multilingual text. It prefers paragraph, sentence, and whitespace boundaries
  while keeping byte ranges exact; text without spaces falls back to safe UTF-8
  boundaries. Source size is capped at 256 KiB, chunks at 512 per object, and
  configurable chunk size at 4 bytes through 2 KiB.
- `Embedding` accepts only finite, non-zero vectors up to 4096 dimensions and
  normalizes provider output. The normalization uses bounded no-std arithmetic;
  this does not add a host math-library dependency.
- `EmbeddingSpaceId` identifies compatible vectors with an opaque 32-byte,
  provider-neutral fingerprint. Existing unscoped vectors remain useful for
  orchestration fixtures, while the durable index rejects them. An index will
  not mix dimensions or embedding spaces; removing its last object clears the
  active identity before a different space can be added.
- `EmbeddingProvider` separates query and passage inputs without selecting a
  vendor or model. `VectorIndex` defines atomic per-object replacement,
  removal, bounded search, and an allowlist parameter for permitted ObjectIds.
- `PersistentVectorIndex` is a bounded exact cosine-search implementation over
  `SnapshotBackend`. It stores only ObjectIds, chunk ordinals, byte ranges, and
  normalized vectors; source text stays with its producer. Snapshots are
  versioned and checksummed, reject malformed or mixed-space contents, cap the
  index at 4096 chunks and the general snapshot at 16 MiB, and publish a new
  in-memory state only after the backend commits it. Search returns one best
  chunk per permitted object with stable score/ObjectId ordering. The checksum
  detects accidental damage and does not authenticate a snapshot.
- `SearchService::index_semantic_text` requires the object to be visible to the
  supplied access context before embedding or replacing its chunks.
  `SearchService::semantic_search` derives the live, non-tombstoned visible
  ObjectId set before calling the index, then checks returned IDs and scores
  again before returning metadata. Invalid limits and oversized queries fail
  closed. The access context must still come from the authenticated caller
  boundary; this crate does not establish that authority.
- Focused fixtures test Japanese chunk boundaries, input bounds, embedding
  validation, visible-only indexing, pre-filtering of IDs sent to an index,
  and rejection of a deliberately non-compliant index's hidden result. The
  persistent-index tests cover reopen/search, allowlist enforcement, atomic
  replacement failures, corrupt and unsupported snapshots, embedding-space
  changes, metadata bounds, and deterministic unique-object ranking.
- The M19 guest fixture stores the exact index through a separate
  `GuestSnapshotBackend` VFS namespace. A test-only deterministic provider
  checks result ordering and hidden-ObjectId filtering before and after QEMU
  restart. It is storage and authorization-orchestration evidence, not model
  inference or natural-language quality evidence.

## Verification

Pinned nightly host tests, isolated from other worktree builds:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
CARGO_TARGET_DIR=/tmp/nagi-m24-index-host \
/Users/tozawa/.cargo/bin/cargo test --locked --offline -p nagi-search
```

Result: 29 tests passed.

Clippy also passed for all package targets with warnings denied:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
CARGO_TARGET_DIR=/tmp/nagi-m24-index-clippy \
/Users/tozawa/.cargo/bin/cargo clippy --locked --offline \
  -p nagi-search --all-targets -- -D warnings
```

The crate also compiled for the Nagi no-std user target:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
CARGO_TARGET_DIR=/tmp/nagi-m24-index-target \
/Users/tozawa/.cargo/bin/cargo -Z build-std=core,alloc check --locked --offline \
  -p nagi-search --target targets/x86_64-unknown-nagi-user.json
```

Result: pass. Formatting was run for `nagi-search`, `nagi-init`, and
`nagi-cli`.

Guest regressions passed with the pinned nightly and the normal repository
`target/` output directory (the image builder reads the init binary from that
directory):

```sh
./nagi m19
./nagi m22
```

M19 passed after two QEMU boots, with the semantic-index restore marker on the
second boot. M22 passed all three boots; boot 3 verified the restored semantic
index together with the existing NH16/NAL1 Undo state. Logs, images, user-data
disks, and OVMF variables are preserved under
`out/evidence/m24-persistent-semantic-index-20261001/`.

## Remaining acceptance

No multilingual embedding model or model artifact is implemented. The guest
provider is a deterministic fixture; it does not establish model inference or
natural-language relevance. Content producers do not yet extract and
synchronize text for supported files, pages, HTML, and PDFs. Hybrid
lexical/metadata/time/workspace ranking, user-facing match explanations, and
stale-index invalidation are absent. The persistent index performs exact
linear search and has not been measured at reference-scale corpus sizes.

M24 remains `PARTIAL`; the deterministic guest fixture does not satisfy the
full acceptance target (`the Servo article I looked at yesterday`) or prove
plausible model-backed results with explanations.
