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
- `EmbeddingProvider` separates query and passage inputs without selecting a
  vendor or model. `VectorIndex` defines atomic per-object replacement,
  removal, bounded search, and an allowlist parameter for permitted ObjectIds.
- `SearchService::index_semantic_text` requires the object to be visible to the
  supplied access context before embedding or replacing its chunks.
  `SearchService::semantic_search` derives the live, non-tombstoned visible
  ObjectId set before calling the index, then checks returned IDs and scores
  again before returning metadata. Invalid limits and oversized queries fail
  closed. The access context must still come from the authenticated caller
  boundary; this crate does not establish that authority.
- Focused fixtures test Japanese chunk boundaries, input bounds, embedding
  validation, visible-only indexing, pre-filtering of IDs sent to an index,
  and rejection of a deliberately non-compliant index's hidden result.

## Verification

Pinned nightly host tests, isolated from other worktree builds:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
CARGO_TARGET_DIR=/tmp/nagi-m24-host \
/Users/tozawa/.cargo/bin/cargo test --locked --offline -p nagi-search
```

Result: 23 tests passed.

Clippy also passed for all package targets with warnings denied:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
CARGO_TARGET_DIR=/tmp/nagi-m24-clippy \
/Users/tozawa/.cargo/bin/cargo clippy --locked --offline \
  -p nagi-search --all-targets -- -D warnings
```

The crate also compiled for the Nagi no-std user target:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
CARGO_TARGET_DIR=/tmp/nagi-m24-target \
/Users/tozawa/.cargo/bin/cargo -Z build-std=core,alloc check --locked --offline \
  -p nagi-search --target targets/x86_64-unknown-nagi-user.json
```

Result: pass. Formatting was run with `cargo fmt --package nagi-search`.

## Remaining acceptance

No multilingual embedding model or model artifact is implemented, and there is
no production vector-index backend or durable semantic-index storage. Content
producers do not yet extract and synchronize text for supported files, pages,
HTML, and PDFs. Hybrid lexical/metadata/time/workspace ranking, user-facing
match explanations, stale-index invalidation, and the M24 natural-language
QEMU acceptance are also absent. The provider and index traits are contracts,
not evidence that embedding inference or semantic retrieval is running.

M24 remains `PARTIAL`; the foundation tests do not satisfy the full acceptance
target (`the Servo article I looked at yesterday`).
