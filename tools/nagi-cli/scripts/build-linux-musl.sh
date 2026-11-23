#!/bin/sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../../.." && pwd)

zig_path=${NAGI_ZIG:-${ZIG:-}}
if [ -z "$zig_path" ] || ! command -v "$zig_path" >/dev/null 2>&1; then
    printf '%s\n' 'Set NAGI_ZIG (or ZIG) to the Zig executable.' >&2
    exit 2
fi
if [ -z "${ZIG_LIB_DIR:-}" ] || [ ! -d "$ZIG_LIB_DIR" ]; then
    printf '%s\n' 'Set ZIG_LIB_DIR to the lib directory for NAGI_ZIG.' >&2
    exit 2
fi
export NAGI_ZIG=$zig_path
export ZIG_LIB_DIR

if [ -n "${NAGI_RUST_TOOLCHAIN_BIN:-}" ]; then
    PATH=$NAGI_RUST_TOOLCHAIN_BIN:$PATH
    export PATH
    CARGO=${CARGO:-$NAGI_RUST_TOOLCHAIN_BIN/cargo}
    RUSTC=${RUSTC:-$NAGI_RUST_TOOLCHAIN_BIN/rustc}
    RUSTDOC=${RUSTDOC:-$NAGI_RUST_TOOLCHAIN_BIN/rustdoc}
fi
CARGO=${CARGO:-cargo}
RUSTC=${RUSTC:-rustc}
RUSTDOC=${RUSTDOC:-rustdoc}
PYTHON=${PYTHON:-python3}

if [ -x "$CARGO" ]; then
    cargo_bin=$CARGO
else
    cargo_bin=$(command -v "$CARGO" || true)
fi
if [ -z "$cargo_bin" ] || ! command -v "$RUSTC" >/dev/null 2>&1 || ! command -v "$RUSTDOC" >/dev/null 2>&1; then
    printf '%s\n' 'Cargo, rustc, and rustdoc must be available from the selected Rust toolchain.' >&2
    exit 2
fi
if ! command -v "$PYTHON" >/dev/null 2>&1; then
    printf '%s\n' 'Python 3 is required to record and verify Linux build provenance.' >&2
    exit 2
fi
export RUSTC RUSTDOC

build_script_sha256=$("$PYTHON" -c 'import hashlib,sys; print(hashlib.sha256(open(sys.argv[1], "rb").read()).hexdigest())' "$script_dir/build-linux-musl.sh")
build_id=$(printf '%s' "$build_script_sha256" | cut -c 1-40)

build_root=${NAGI_BUILD_ROOT:-$repo_root/target/nagi-cli-linux-musl}
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$build_root/cargo-target}
export ZIG_GLOBAL_CACHE_DIR=${ZIG_GLOBAL_CACHE_DIR:-$build_root/zig-global-cache}
export ZIG_LOCAL_CACHE_DIR=${ZIG_LOCAL_CACHE_DIR:-$build_root/zig-local-cache}
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=$script_dir/zig-cc-linux-musl.sh

musl_rustflags=${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS:-${RUSTFLAGS:-}}
if [ -n "${CARGO_ENCODED_RUSTFLAGS:-}" ]; then
    rustflags_separator=$(printf '\037')
    encoded_rustflags=$CARGO_ENCODED_RUSTFLAGS
    case "$rustflags_separator$encoded_rustflags$rustflags_separator" in
        *"$rustflags_separator-C$rustflags_separator-link-self-contained=off$rustflags_separator"*) ;;
        *)
            export CARGO_ENCODED_RUSTFLAGS="${encoded_rustflags}${encoded_rustflags:+$rustflags_separator}-C${rustflags_separator}link-self-contained=off"
            ;;
    esac
    export CARGO_ENCODED_RUSTFLAGS="${CARGO_ENCODED_RUSTFLAGS}${CARGO_ENCODED_RUSTFLAGS:+$rustflags_separator}-C${rustflags_separator}link-arg=-Wl,--build-id=0x${build_id}"
else
    case " $musl_rustflags " in
        *" -C link-self-contained=off "*) ;;
        *)
            if [ -n "$musl_rustflags" ]; then
                musl_rustflags="$musl_rustflags "
            fi
            musl_rustflags="${musl_rustflags}-C link-self-contained=off"
            ;;
    esac
    # Cargo supports RUSTFLAGS (not CARGO_TARGET_<triple>_RUSTFLAGS). Both
    # commands below pass --target, so Cargo applies this only to Linux target
    # crates and leaves host build scripts and proc macros unchanged.
    musl_rustflags="${musl_rustflags}${musl_rustflags:+ }-C link-arg=-Wl,--build-id=0x${build_id}"
    export RUSTFLAGS=$musl_rustflags
fi

mkdir -p "$CARGO_TARGET_DIR" "$ZIG_GLOBAL_CACHE_DIR" "$ZIG_LOCAL_CACHE_DIR"
cd "$repo_root"

# Check the complete workspace lock closure before starting any target build.
# This remains read-only: --locked rejects drift and --offline avoids fetching.
"$PYTHON" "$script_dir/verify-cargo-lock.py" \
    --cargo "$cargo_bin" --manifest-path "$repo_root/Cargo.toml"

"$cargo_bin" build --locked --offline --release --manifest-path "$repo_root/Cargo.toml" \
    -p nagi-cli --bin nagi --target x86_64-unknown-linux-musl
"$cargo_bin" build --locked --offline --manifest-path "$repo_root/Cargo.toml" \
    -p nagi-cli --bin nagi --target x86_64-unknown-linux-musl
"$cargo_bin" test --locked --offline --manifest-path "$repo_root/Cargo.toml" \
    -p nagi-cli --no-run --target x86_64-unknown-linux-musl

"$PYTHON" "$script_dir/pack-linux-musl-guest.py" \
    --record-build-manifest --build-root "$build_root" \
    --cargo-target-dir "$CARGO_TARGET_DIR"

printf 'DEBUG_CLI=%s\n' "$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/debug/nagi"
printf 'STATIC_CLI=%s\n' "$CARGO_TARGET_DIR/x86_64-unknown-linux-musl/release/nagi"
