#!/bin/sh
set -eu

: "${CARGO_TARGET_DIR:?set the fixed lane Cargo target directory}"

directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
toolchain=nightly-2025-08-01
target=x86_64-unknown-linux-musl
cargo_bin=$(rustup which --toolchain "$toolchain" cargo)
rustc_bin=$(rustup which --toolchain "$toolchain" rustc)
sysroot=$("$rustc_bin" --print sysroot)
host=$("$rustc_bin" -vV | sed -n 's/^host: //p')
target_libdir=$("$rustc_bin" --print target-libdir --target "$target")
export RUSTC="$rustc_bin"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$sysroot/lib/rustlib/$host/bin/rust-lld"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C relocation-model=static -C link-self-contained=no -L native=$target_libdir/self-contained -l static=c"

exec "$cargo_bin" build --release --locked --target "$target" --manifest-path "$directory/Cargo.toml"
