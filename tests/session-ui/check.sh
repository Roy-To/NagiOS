#!/usr/bin/env sh
# HOST orchestration and Nagi COMPILE-ONLY checks. Never guest inference.
set -eu
cd "$(dirname "$0")/../.."
manifest=tests/session-ui/Cargo.toml
cargo fmt --manifest-path "$manifest" -- --check
cargo test --manifest-path "$manifest" --locked --target x86_64-unknown-linux-gnu
cargo clippy --manifest-path "$manifest" --locked --all-targets --target x86_64-unknown-linux-gnu -- -D warnings
cargo clippy --manifest-path "$manifest" --locked --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,alloc -- -D warnings
cargo check --manifest-path "$manifest" --locked --features desktop-check --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,alloc
