#!/usr/bin/env sh
# Offline host protocol checks; never execute the child or a guest.
set -eu
cd "$(dirname "$0")/../../.."
cargo test --manifest-path tests/acceptance/bar-summary-wire/Cargo.toml --locked --offline
cargo clippy --manifest-path tests/acceptance/bar-summary-wire/Cargo.toml --all-targets --locked --offline -- -D warnings
