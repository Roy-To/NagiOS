#!/usr/bin/env sh
# HOST ONLY: compile the actual Albert library, without resolving Servo/guest deps.
set -eu
cd "$(dirname "$0")/../../.."
toolchain=nightly-2025-08-01
cargo=$(rustup which cargo --toolchain "$toolchain")
rustc=$(rustup which rustc --toolchain "$toolchain")
rustfmt=$(rustup which rustfmt --toolchain "$toolchain")
rustdoc=$(rustup which rustdoc --toolchain "$toolchain")
"$rustfmt" --check --edition 2021 --config skip_children=true \
  user/nagi-albert/src/lib.rs user/nagi-albert/src/runtime_owner.rs user/nagi-albert/src/runtime_owner_tests.rs \
  user/nagi-albert/src/engine_step.rs user/nagi-albert/src/engine_step_tests.rs
RUSTC="$rustc" RUSTDOC="$rustdoc" "$cargo" check --manifest-path user/nagi-albert/host-tests/Cargo.toml --all-targets --all-features --locked --offline
RUSTC="$rustc" RUSTDOC="$rustdoc" "$cargo" test --manifest-path user/nagi-albert/host-tests/Cargo.toml --all-features --locked --offline
RUSTC="$rustc" RUSTDOC="$rustdoc" "$cargo" clippy --manifest-path user/nagi-albert/host-tests/Cargo.toml --all-targets --all-features --locked --offline -- -D warnings
