# Ordinary Files/Search host acceptance

This standalone suite imports the actual `nagi-init` Files, UI state and
Search runtime leaves, `libnagi::storage::Vfs`, and the actual guest snapshot
adapter. Its block device and injected Search backend are explicit test
fixtures. It does not use the host filesystem to simulate a guest disk.

```sh
cargo fmt --manifest-path tests/files-search/Cargo.toml -- --check
cargo clippy --manifest-path tests/files-search/Cargo.toml --all-targets --locked -- -D warnings
cargo test --manifest-path tests/files-search/Cargo.toml --locked
```

The suite checks actual content/inode/generation/ObjectId retention across
trash, VFS remount and restoration, UTF-8 and maximum names, conflict without
overwrite, repeated requests, stale selection after inode reuse, Search
write/capacity failures, trash capacity, corrupt journal fallback/fail-closed,
flush failure, 192 sector-failure/remount scenarios (short and maximum-safe
directories), successful crowded-directory lifecycle, private
caller visibility, nested Search, and UI input/cancellation/error behavior.

Fixture PASS here is host orchestration/storage evidence. Guest production
PASS requires the signed Files child, live grants and UI operations on QEMU;
see the shared-owner proposals in
`docs/workstreams/m19-files-search-operations-handoff.md`. The existing
`./nagi login` scenario checks previous Search/lifecycle acceptance, but does
not yet activate the new ordinary Files management controls.

Imported evidence reports 24 tests with no ignores for the recovered VFS fix;
fresh recovery validation also passed all 24 tests without ignores.
The previous create alias is now covered by
`interrupted_create_never_aliases_the_next_inode`; historical failed evidence
is retained separately. VFS in-file tests also cover persistent interruption,
volatile/durable caches, recovery write/flush faults, checksum rejection and
instance serialization. See `docs/workstreams/m19-vfs-create-recovery-proposal.md`.
