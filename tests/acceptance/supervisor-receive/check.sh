#!/usr/bin/env sh
# Exact Supervisor host boundary checks; neither guest syscalls nor Browser work.
set -eu
cd "$(dirname "$0")/../../.."
rustfmt --check --edition 2021 --config skip_children=true \
  user/nagi-init/src/supervisor.rs tests/acceptance/supervisor-receive/src/lib.rs
cargo test --manifest-path tests/acceptance/supervisor-receive/Cargo.toml --locked --offline -- --test-threads=1
cargo clippy --manifest-path tests/acceptance/supervisor-receive/Cargo.toml --locked --offline --all-targets -- -D warnings
