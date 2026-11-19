#!/bin/sh
set -eu

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd -P)
crate_dir=$(CDPATH= cd "$script_dir/.." && pwd -P)
worktree_root=$(git -C "$crate_dir" rev-parse --show-toplevel)
target_root="${NAGI_JOB_LINUX_TARGET_ROOT:-$worktree_root/target/job-scheduler-linux-zig}"
cargo_target_dir="${NAGI_JOB_CARGO_TARGET_DIR:-$target_root/cargo-target-root-lock}"
job_cargo_home="${NAGI_JOB_CARGO_HOME:-$target_root/cargo-home-root-lock}"
vendor_dir="${NAGI_JOB_CARGO_VENDOR_DIR:-$target_root/vendor-root-lock}"
registry_cargo_home="${NAGI_JOB_REGISTRY_CARGO_HOME:-$HOME/.cargo}"
nagi_rust_target="${NAGI_JOB_RUST_TARGET:-x86_64-unknown-linux-musl}"
nagi_zig="${NAGI_ZIG:-${1:-}}"

if [ -n "${CARGO_BUILD_BUILD_DIR:-}" ]; then
    echo "Refusing inherited CARGO_BUILD_BUILD_DIR; unset it to keep Cargo output inside the dedicated target." >&2
    exit 2
fi

if [ -z "$nagi_zig" ] || [ ! -x "$nagi_zig" ]; then
    echo "Set NAGI_ZIG or pass the executable path to Zig." >&2
    exit 2
fi
nagi_zig_dir=$(CDPATH= cd "$(dirname "$nagi_zig")" && pwd -P)
nagi_zig="$nagi_zig_dir/$(basename "$nagi_zig")"

nagi_rustup="${NAGI_RUSTUP:-rustup}"
if [ -n "${NAGI_RUST_TOOLCHAIN:-}" ]; then
    rust_toolchain="$NAGI_RUST_TOOLCHAIN"
else
    rust_toolchain=$(cd "$worktree_root" && "$nagi_rustup" show active-toolchain | awk 'NR == 1 {print $1}')
fi
if [ -n "${NAGI_CARGO_BIN:-}" ]; then
    cargo_bin="$NAGI_CARGO_BIN"
else
    cargo_bin=$("$nagi_rustup" which --toolchain "$rust_toolchain" cargo)
fi
if [ -n "${NAGI_RUSTC_BIN:-}" ]; then
    rustc_bin="$NAGI_RUSTC_BIN"
else
    rustc_bin=$("$nagi_rustup" which --toolchain "$rust_toolchain" rustc)
fi
if [ -n "${NAGI_RUSTDOC_BIN:-}" ]; then
    rustdoc_bin="$NAGI_RUSTDOC_BIN"
else
    rustdoc_bin=$("$nagi_rustup" which --toolchain "$rust_toolchain" rustdoc)
fi

mkdir -p "$target_root" "$cargo_target_dir" "$job_cargo_home"
export NAGI_ZIG="$nagi_zig"
export CARGO_TARGET_DIR="$cargo_target_dir"
export ZIG_GLOBAL_CACHE_DIR="${NAGI_JOB_ZIG_GLOBAL_CACHE_DIR:-$target_root/zig-global}"
export ZIG_LOCAL_CACHE_DIR="${NAGI_JOB_ZIG_LOCAL_CACHE_DIR:-$target_root/zig-local}"
export RUSTC="$rustc_bin"
export RUSTDOC="$rustdoc_bin"
case "$nagi_rust_target" in
    x86_64-unknown-linux-musl)
        export NAGI_ZIG_TARGET="${NAGI_ZIG_TARGET:-x86_64-linux-musl}"
        export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$script_dir/zig-linux-linker.sh"
        export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }-C link-self-contained=no"
        ;;
    x86_64-unknown-linux-gnu)
        export NAGI_ZIG_TARGET="${NAGI_ZIG_TARGET:-x86_64-linux-gnu.2.17.0}"
        export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER="$script_dir/zig-linux-linker.sh"
        ;;
    *)
        echo "Unsupported NAGI_JOB_RUST_TARGET: $nagi_rust_target" >&2
        exit 2
        ;;
esac
mkdir -p "$ZIG_GLOBAL_CACHE_DIR" "$ZIG_LOCAL_CACHE_DIR"

isolated_workspace=$(mktemp -d "$target_root/isolate.XXXXXX")
cp "$crate_dir/Cargo.toml" "$isolated_workspace/Cargo.toml"
printf '\n[workspace]\n' >> "$isolated_workspace/Cargo.toml"
cp -R "$crate_dir/src" "$isolated_workspace/src"
cp -R "$crate_dir/tests" "$isolated_workspace/tests"
printf '\n[patch.crates-io]\nlibc = { path = "%s" }\n' "$worktree_root/third_party/libc-servo" >> "$isolated_workspace/Cargo.toml"
cp "$worktree_root/Cargo.lock" "$isolated_workspace/Cargo.lock"
diff -qr "$crate_dir/src" "$isolated_workspace/src" >/dev/null
diff -qr "$crate_dir/tests" "$isolated_workspace/tests" >/dev/null

run_registry_cargo() {
    CARGO_HOME="$registry_cargo_home" "$cargo_bin" "$@"
}

run_registry_cargo metadata --offline --format-version 1 \
    --manifest-path "$isolated_workspace/Cargo.toml" >/dev/null
python3 "$script_dir/verify-linux-lock-closure.py" --self-test
cargo_tree="$isolated_workspace/cargo-tree.txt"
run_registry_cargo tree --offline --locked --target all --no-dedupe --edges normal,build,dev \
    --prefix depth --format '{p}' --color never \
    --manifest-path "$isolated_workspace/Cargo.toml" \
    -p nagi-job-scheduler --features linux-development-runtime > "$cargo_tree"
python3 "$script_dir/verify-linux-lock-closure.py" \
    --root-lock "$worktree_root/Cargo.lock" \
    --projected-lock "$isolated_workspace/Cargo.lock" \
    --cargo-tree "$cargo_tree" \
    --projected-manifest "$isolated_workspace/Cargo.toml"
run_registry_cargo vendor --no-delete --offline --locked \
    --manifest-path "$isolated_workspace/Cargo.toml" "$vendor_dir" >/dev/null

cat > "$job_cargo_home/config.toml" <<EOF
[source.crates-io]
replace-with = "vendored-sources"

[source.vendored-sources]
directory = "$vendor_dir"
EOF

export CARGO_HOME="$job_cargo_home"
cd "$worktree_root"
"$cargo_bin" test --no-run --offline --locked \
    --target "$nagi_rust_target" \
    --features linux-development-runtime \
    --manifest-path "$isolated_workspace/Cargo.toml" \
    -p nagi-job-scheduler
