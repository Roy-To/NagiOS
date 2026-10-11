#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")"
cargo fmt -- --check
cargo test --locked --offline
cargo clippy --all-targets --locked --offline -- -D warnings
# Run ABI and BootInfo tests from their original sources, without root workspace resolution.
tmp_dir=$(mktemp -d)
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM
rustc --edition=2021 --test ../../crates/nagi-bootinfo/src/lib.rs -o "$tmp_dir/bootinfo-tests"
"$tmp_dir/bootinfo-tests"
rustc --edition=2021 --test ../../crates/nagi-abi/src/lib.rs -o "$tmp_dir/abi-tests"
"$tmp_dir/abi-tests"
