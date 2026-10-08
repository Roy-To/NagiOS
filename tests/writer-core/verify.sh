#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
manifest=crates/nagi-writer-core/Cargo.toml
cargo fmt --manifest-path "$manifest" --all -- --check
cargo build --manifest-path "$manifest" --offline --locked
cargo test --manifest-path "$manifest" --offline --locked
cargo clippy --manifest-path "$manifest" --all-targets --offline --locked -- -D warnings
RUSTDOCFLAGS='-D warnings' cargo doc --manifest-path "$manifest" --no-deps --offline --locked
git diff --check
