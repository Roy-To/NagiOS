# TEST-PLAT-01 host-only harness

Run the deterministic fixture acceptance suite from the repository root:

```sh
PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path crates/nagi-test-harness/Cargo.toml --offline
```

The standalone crate has no third-party dependencies and its manifest declares
its own Cargo workspace, so this command does not modify or resolve the root
workspace. It is designed to run without network access after Rust is installed.
The PATH prefix selects the repository-pinned Rustup toolchain if another Rust
installation is earlier in PATH.
It does not invoke QEMU, product services, target code, or the shared CI runner.

The suite checks only the orchestration contracts represented by its fakes. A
pass is not provider acceptance, target acceptance, product acceptance, or M30
acceptance. Shared CI wiring remains owned by the Integration Owner and the
ci-acceptance workstream.
