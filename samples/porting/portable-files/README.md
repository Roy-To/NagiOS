# Portable Files core migration sample

This isolated Cargo package adapts the host-independent `Location` and `FileName` validation from `apps/nagi-files/src/model.rs`. Its catalog uses in-memory relative paths only. It has no filesystem backend, Nagi runtime binding, or requested capability.

Run `cargo test --manifest-path samples/porting/portable-files/Cargo.toml` from the repository root. The adjacent `manifest.json` is a third-party, zero-capability host validator fixture; it is not an installable Nagi application package.

See [`../../../docs/porting/app-migration/README.md`](../../../docs/porting/app-migration/README.md) for the migration guide.
