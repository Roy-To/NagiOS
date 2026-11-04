#!/usr/bin/env sh
set -eu
if [ "$#" -ne 4 ]; then
  echo "usage: $0 SOURCE_COMMIT_SHA SOURCE_TREE_SHA MAIN_COMMIT_SHA OUTPUT_DIR" >&2
  exit 2
fi
source_commit=$1
source_tree=$2
main_commit=$3
output_dir=$4
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
cargo fmt -- --check
cargo clippy --release --locked --offline -- -D warnings
python3 -m unittest test_runner test_attestation -v
python3 attest.py \
  --source-commit "$source_commit" \
  --source-tree "$source_tree" \
  --main "$main_commit" \
  --output "$output_dir"
python3 verify_attestation.py \
  --attestation "$output_dir/attestation.json" \
  --source-commit "$source_commit" \
  --source-tree "$source_tree" \
  --main "$main_commit"
