#!/usr/bin/env sh
set -eu

root_dir=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
crate_manifest="$root_dir/crates/nagi-wayback/Cargo.toml"
m15_manifest="$root_dir/user/nagi-history/Cargo.toml"
toolchain=$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$root_dir/rust-toolchain.toml")
rustup_path=$(command -v rustup || true)
if [ -z "$toolchain" ] || [ -z "$rustup_path" ]; then
    printf '%s\n' 'FAIL Wayback verification requires the repository Rust toolchain and rustup' >&2
    exit 4
fi
pinned_cargo=$("$rustup_path" which cargo --toolchain "$toolchain" 2>/dev/null || true)
pinned_rustc=$("$rustup_path" which rustc --toolchain "$toolchain" 2>/dev/null || true)
pinned_rustdoc=$("$rustup_path" which rustdoc --toolchain "$toolchain" 2>/dev/null || true)
if [ -z "$pinned_cargo" ] || [ -z "$pinned_rustc" ] || [ -z "$pinned_rustdoc" ]; then
    printf '%s\n' 'FAIL repository Rust toolchain is incomplete; install rustc, rustdoc, and Cargo for rust-toolchain.toml' >&2
    exit 4
fi
toolchain_bin=$(dirname -- "$pinned_cargo")
PATH="$toolchain_bin:$PATH"
export PATH
export RUSTC="$pinned_rustc"
export RUSTDOC="$pinned_rustdoc"

temp_dir=$(mktemp -d "${TMPDIR:-/tmp}/nagi-wayback-verify.XXXXXX")
trap 'rm -rf "$temp_dir"' EXIT HUP INT TERM

"$pinned_cargo" fmt --manifest-path "$crate_manifest" -- --check
"$pinned_cargo" test --manifest-path "$crate_manifest" --locked
"$pinned_cargo" clippy --manifest-path "$crate_manifest" --all-targets --locked -- -D warnings
"$pinned_cargo" run --manifest-path "$crate_manifest" --example contract_fixtures --locked > "$temp_dir/contracts.json"
schema_python_path=${PYTHONPATH:-}
if ! python3 -c 'import importlib.metadata; assert importlib.metadata.version("jsonschema") == "4.26.0"' >/dev/null 2>&1; then
    python3 -m pip install --disable-pip-version-check --no-input --quiet \
        --target "$temp_dir/schema-python" \
        --requirement "$root_dir/tests/wayback/requirements.txt"
    schema_python_path="$temp_dir/schema-python${schema_python_path:+:$schema_python_path}"
fi
PYTHONPATH="$schema_python_path" python3 "$root_dir/tests/wayback/validate_schemas.py" "$root_dir" "$temp_dir/contracts.json"

"$pinned_cargo" test --manifest-path "$m15_manifest" --locked
"$root_dir/nagi" dev verify
