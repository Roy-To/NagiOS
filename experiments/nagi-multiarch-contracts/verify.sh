#!/usr/bin/env sh
set -eu
cd "$(dirname "$0")"

# Ensure Cargo, rustc, rustdoc and Rust tools all use the repository-pinned toolchain.
toolchain=nightly-2025-08-01
rustc_path=$(rustup which rustc --toolchain "$toolchain")
toolchain_bin=${rustc_path%/*}
PATH="$toolchain_bin:$PATH"
RUSTUP_TOOLCHAIN=$toolchain
RUSTC=$toolchain_bin/rustc
RUSTDOC=$toolchain_bin/rustdoc
RUSTFMT=$toolchain_bin/rustfmt
export PATH RUSTUP_TOOLCHAIN RUSTC RUSTDOC RUSTFMT
python3 -m unittest test_audit test_records -v
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
