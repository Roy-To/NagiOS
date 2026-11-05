#!/usr/bin/env sh
# Host-only exact Albert source checks; no Servo, guest or inference evidence.
set -eu
cd "$(dirname "$0")/../.."
rustfmt --check --edition 2021 --config skip_children=true \
  user/nagi-albert/src/assistant_context.rs user/nagi-albert/src/lib.rs tests/albert-context/src/lib.rs
cargo test --manifest-path tests/albert-context/Cargo.toml --locked --offline
cargo clippy --manifest-path tests/albert-context/Cargo.toml --all-targets --locked --offline -- -D warnings
