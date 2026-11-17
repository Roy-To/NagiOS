#!/usr/bin/env bash
# Nagi target check/build evidence for nagi-tts-provider (no link, no run).
#
#   tests/m25-tts/nagi-target.sh > LOG
#
# Prerequisite: out/rust-src prepared like tools/nagi-cli
# prepare_nagi_rust_std_source (pinned rust-src + 0001-nagi-target-support.patch
# + third_party/libc patch in library/Cargo.toml), e.g. by a prior m13-std build.
set -uo pipefail
cd "$(dirname "$0")/../.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
M=crates/nagi-tts-provider/Cargo.toml
T=targets/x86_64-unknown-nagi-user.json
echo "date=$(python3 -c 'import datetime;print(datetime.datetime.now().astimezone().isoformat(timespec="seconds"))')"
echo "commit=$(git rev-parse HEAD) dirty=$(git status --porcelain -- crates/nagi-tts-provider | wc -l)"
echo "rustc=$(rustc --version)"
echo "rust_src_root=out/rust-src/library (rust-src of the pinned toolchain + third_party/rust-std/patches/0001-nagi-target-support.patch)"
run() { echo "\$ $*"; "$@" 2>&1 | grep -E "^(warning|error)|Finished|Compiling nagi-tts|Checking nagi-tts|generated" ; echo "exit=${PIPESTATUS[0]}"; }
echo "== 1 no_std core (engine disabled)"
run nice -n 19 cargo check -j1 --manifest-path $M --no-default-features --target $T -Zbuild-std=core,alloc --target-dir out/target-nagi --locked
echo "== 2 engine-enabled check (std via build-std)"
export __CARGO_TESTS_ONLY_SRC_ROOT=$PWD/out/rust-src/library
run nice -n 19 cargo check -j1 --manifest-path $M --lib --target $T -Zbuild-std=std,panic_abort --target-dir out/target-nagi-std --locked
echo "== 3 engine-enabled release build (rlib only)"
run nice -n 19 cargo build -j1 --release --manifest-path $M --lib --target $T -Zbuild-std=std,panic_abort --target-dir out/target-nagi-std --locked
ls -l out/target-nagi-std/x86_64-unknown-nagi-user/release/libnagi_tts_provider.rlib | awk '{print "rlib_bytes="$5}'
echo "NOTE: rlib only; nothing linked into nagi-init, nothing run on Nagi/QEMU."
