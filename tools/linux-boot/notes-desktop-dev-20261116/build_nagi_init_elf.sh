#!/bin/sh
set -eu

usage() {
  echo "usage: $0 <new-target-label> [--online] [--plan]" >&2
  echo "Build the development Linux nagi-init ELF with Rust/Cargo 1.98.1; offline by default." >&2
  exit 2
}

[ "$#" -ge 1 ] || usage
label=$1
shift
case "$label" in
  ""|*[!A-Za-z0-9_-]*) usage ;;
esac
online=false
plan=false
for option in "$@"; do
  case "$option" in
    --online) online=true ;;
    --plan) plan=true ;;
    *) usage ;;
  esac
done

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../../.." && pwd)
toolchain_bin="${RUSTUP_HOME:-$HOME/.rustup}/toolchains/1.98.1-aarch64-apple-darwin/bin"
cargo="$toolchain_bin/cargo"
rustc="$toolchain_bin/rustc"
rustdoc="$toolchain_bin/rustdoc"
cc=/usr/bin/cc
clang=/usr/bin/clang
lld=/opt/homebrew/bin/ld.lld
target_linker_env=CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER
target_linker_value=${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER-}

# Cargo gives CARGO_ENCODED_RUSTFLAGS precedence over RUSTFLAGS. Do not let a
# caller silently alter the development cfgs or target link arguments.
if [ "${CARGO_ENCODED_RUSTFLAGS+x}" = x ]; then
  echo "refusing caller CARGO_ENCODED_RUSTFLAGS; it overrides the helper's pinned RUSTFLAGS" >&2
  exit 1
fi
if [ "${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER+x}" = x ] &&
   [ "$target_linker_value" != "$cc" ]; then
  echo "refusing conflicting $target_linker_env; expected the helper's pinned linker" >&2
  exit 1
fi
if [ "${NAGI_TARGET_CLANG+x}" = x ] &&
   [ "$NAGI_TARGET_CLANG" != "$clang" ] &&
   [ "$NAGI_TARGET_CLANG" != "clang" ]; then
  echo "refusing conflicting NAGI_TARGET_CLANG; expected the helper's pinned compiler" >&2
  exit 1
fi
if env | /usr/bin/awk -F= '$1 ~ /^CARGO_PROFILE_DEV_/ { found = 1 } END { exit !found }'; then
  echo "refusing caller CARGO_PROFILE_DEV_*; this entry fixes the debug profile" >&2
  exit 1
fi

[ "$(uname -s)" = "Darwin" ] && [ "$(uname -m)" = "arm64" ] || {
  echo "build host must be macOS arm64" >&2
  exit 1
}
for tool in "$cargo" "$rustc" "$rustdoc" "$cc" "$clang" "$lld"; do
  [ -x "$tool" ] || { echo "required tool missing: $tool" >&2; exit 1; }
done
case $("$cargo" --version) in "cargo 1.98.1 "*) ;; *) echo "Cargo 1.98.1 required" >&2; exit 1 ;; esac
case $("$rustc" --version) in "rustc 1.98.1 "*) ;; *) echo "rustc 1.98.1 required" >&2; exit 1 ;; esac
cc_version=$("$cc" --version | /usr/bin/head -n 1)
[ "$cc_version" = "Apple clang version 21.0.0 (clang-2100.3.34.2)" ] || {
  echo "unexpected target linker driver: $cc_version" >&2
  exit 1
}
clang_version=$("$clang" --version | /usr/bin/head -n 1)
[ "$clang_version" = "$cc_version" ] || {
  echo "target C/C++ build compiler differs from the pinned Apple clang driver" >&2
  exit 1
}
case $("$lld" --version) in "Homebrew LLD 19.1.7"*) ;; *) echo "Homebrew LLD 19.1.7 required" >&2; exit 1 ;; esac

PATH="/usr/bin:/opt/homebrew/bin:$PATH"
export PATH
[ "$(command -v cc)" = "$cc" ] && [ "$(command -v clang)" = "$clang" ] &&
  [ "$(command -v ld.lld)" = "$lld" ] || {
  echo "tool resolution differs from the pinned clang/LLD pair" >&2
  exit 1
}

cargo_home="${NAGI_CARGO_STORE:-$HOME/.cargo}"
target_dir="$script_dir/target-nagi-init-rust198-$label"
build_dir_value=${CARGO_BUILD_BUILD_DIR-}
if [ "${CARGO_BUILD_BUILD_DIR+x}" = x ] && [ "$build_dir_value" != "$target_dir" ]; then
  echo "refusing conflicting CARGO_BUILD_BUILD_DIR; expected the helper's pinned build directory" >&2
  exit 1
fi
if [ -e "$target_dir" ] || [ -L "$target_dir" ]; then
  echo "refusing to reuse existing target: $target_dir" >&2
  exit 1
fi
for path in "$cargo_home" "$cargo_home/registry" "$cargo_home/git"; do
  if [ -L "$path" ]; then
    echo "refusing symlinked Cargo home path: $path" >&2
    exit 1
  fi
done

export CARGO_HOME="$cargo_home"
export CARGO_TARGET_DIR="$target_dir"
export CARGO_BUILD_TARGET_DIR="$target_dir"
# Cargo's build.build-dir defaults to build.target-dir, but its environment
# override can redirect intermediates independently of the target directory.
# Pin it here so inherited or Cargo-home config cannot escape the lane target.
export CARGO_BUILD_BUILD_DIR="$target_dir"
# This Linux preview uses the sibling manifest and root Linux-only lock graph;
# the product manifest retains Nagi-target Servo patches and its own lockfile.
export CARGO_RESOLVER_LOCKFILE_PATH="$repo_root/Cargo.lock"
export CARGO="$cargo"
export RUSTC="$rustc"
export CARGO_BUILD_RUSTC="$rustc"
export RUSTDOC="$rustdoc"
export CARGO_BUILD_RUSTDOC="$rustdoc"
export RUSTC_WRAPPER=
export CARGO_BUILD_RUSTC_WRAPPER=
export RUSTC_WORKSPACE_WRAPPER=
export CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER=
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$cc"
export NAGI_TARGET_CLANG="$clang"
export RUSTFLAGS='--cfg linux_dev_preview_smoke --cfg linux_desktop_dev_smoke --cfg getrandom_backend="linux_getrandom" -C link-arg=--target=x86_64-unknown-linux-musl -C link-arg=-fuse-ld=lld'

set -- "$cargo" build \
  --verbose \
  --manifest-path "$repo_root/user/nagi-init-linux-tests/Cargo.toml" \
  --package nagi-init \
  --locked \
  --no-default-features \
  --features linux-service-action-bridge,m10-desktop,linux-dev-data-selection \
  --target x86_64-unknown-linux-musl
if [ "$online" = false ]; then
  set -- "$@" --offline
fi

echo "rustc=$("$rustc" --version)"
echo "cargo=$("$cargo" --version)"
echo "cc=$cc_version clang=$clang_version lld=$("$lld" --version)"
echo "effective_cargo=$cargo rustc=$RUSTC rustdoc=$RUSTDOC linker=$CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER nagi_target_clang=$NAGI_TARGET_CLANG"
echo 'effective_rustflags=linux_dev_preview_smoke,linux_desktop_dev_smoke,getrandom_backend=linux_getrandom,static-musl-target,LLD19 encoded_rustflags=unset wrappers=disabled'
echo "cargo_home=$CARGO_HOME cargo_lock=$CARGO_RESOLVER_LOCKFILE_PATH target_dir=$CARGO_TARGET_DIR build_dir=$CARGO_BUILD_BUILD_DIR target=x86_64-unknown-linux-musl profile=debug features=linux-service-action-bridge,m10-desktop,linux-dev-data-selection"
if [ "$plan" = true ]; then
  printf 'command='
  printf ' %s' "$@"
  printf '\n'
  echo "output=$target_dir/x86_64-unknown-linux-musl/debug/nagi-init"
  exit 0
fi

mkdir -p "$cargo_home"
"$@"

output="$target_dir/x86_64-unknown-linux-musl/debug/nagi-init"
[ -f "$output" ] && [ ! -L "$output" ] || {
  echo "nagi-init ELF missing or not a regular file: $output" >&2
  exit 1
}
description=$(/usr/bin/file "$output")
printf '%s\n' "$description" | /usr/bin/grep -Eq 'ELF 64-bit LSB pie executable, x86-64.*static-pie linked' || {
  echo "unexpected nagi-init ELF format: $description" >&2
  exit 1
}
echo "nagi_init_build=PASS output=$output"
