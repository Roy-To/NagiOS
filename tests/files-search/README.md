# M27 VFS recovery host tests

This standalone crate imports the production `recovery.rs` and
`vfs_recovery.rs` modules and the six host recovery contract tests. The tests
use an in-memory fault-injecting block device; they do not inspect or modify a
real image, launch QEMU, or claim guest acceptance.

This crate has its own workspace, so root `cargo test --workspace` does not
include it. The Linux-only block adapter suite is a separate `libnagi`
integration target.

Run the focused host suites with:

```sh
cargo test --manifest-path tests/files-search/Cargo.toml --locked
cargo test --manifest-path user/libnagi/Cargo.toml --test block_write_adapter --locked
```

Formatting and warnings-denied Clippy can be run against the recovery manifest
when the host verification window is available.
