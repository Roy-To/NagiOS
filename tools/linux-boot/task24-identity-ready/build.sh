#!/bin/sh
set -eu

: "${CARGO_TARGET_DIR:?set the fixed lane Cargo target directory}"

directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
target_libdir=$(rustc --print target-libdir --target x86_64-unknown-linux-musl)
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C relocation-model=static -C link-self-contained=no -L native=$target_libdir/self-contained -l static=c"

exec cargo build --offline --locked --release --target x86_64-unknown-linux-musl --manifest-path "$directory/Cargo.toml"
