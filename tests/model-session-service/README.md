# Ordinary-session model service checks

This standalone workspace checks the exact `nagi-init` model-service leaves
without changing the shared root workspace or init feature graph. It links no
host inference runtime and contains no synthetic inference FFI implementation.
The host tests exercise the real `libnagi::security::Session` binding only.
The Nagi target check compiles both the ordinary-session service and the
existing Granite acceptance against the extracted target backend; it does not
link the target C++ archives or run guest inference.

Run from the repository root with the pinned toolchain:

```sh
cargo test --locked -p nagi-model-manager -p nagi-ai
cargo clippy --locked -p nagi-model-manager --all-targets -- -D warnings
cargo test --locked --manifest-path tests/model-session-service/Cargo.toml
cargo clippy --locked --manifest-path tests/model-session-service/Cargo.toml \
  --features granite-acceptance --target targets/x86_64-unknown-nagi-user.json \
  -Zbuild-std=core,alloc -- -D warnings
cargo fmt --manifest-path tests/model-session-service/Cargo.toml -- --check
```

The `granite-acceptance` check also verifies that enabling the production and
acceptance features together shares one Rust backend module. Existing
`nagi-posix` dependency warnings remain visible; warnings in these leaves are
denied. The service orchestration tests under Model Manager explicitly use a
test backend and small synthetic artifact bytes. None of these host checks is
a substitute for a real normal-session Granite response.
