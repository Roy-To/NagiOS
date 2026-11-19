#!/bin/sh
set -eu

usage() {
  echo "usage: $0 <new-target-label> [--online] [--plan] [--recovery-ui-smoke]" >&2
  echo "Build the Linux Notes ELF with rustc/cargo 1.98.1; offline by default." >&2
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
recovery_ui_smoke=false
for option in "$@"; do
  case "$option" in
    --online) online=true ;;
    --plan) plan=true ;;
    --recovery-ui-smoke) recovery_ui_smoke=true ;;
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
lld=/opt/homebrew/bin/ld.lld
target_linker_env=CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER
target_linker_value=${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER-}

# Cargo gives CARGO_ENCODED_RUSTFLAGS precedence over RUSTFLAGS. Do not let a
# caller silently replace the flags this entry promises to use. The target
# linker is pinned below; reject a conflicting caller value instead of
# concealing it.
if [ "${CARGO_ENCODED_RUSTFLAGS+x}" = x ]; then
  echo "refusing caller CARGO_ENCODED_RUSTFLAGS; it overrides the helper's pinned RUSTFLAGS" >&2
  exit 1
fi
if [ "${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER+x}" = x ] &&
   [ "$target_linker_value" != "$cc" ]; then
  echo "refusing conflicting $target_linker_env; expected the helper's pinned linker" >&2
  exit 1
fi

[ "$(uname -s)" = "Darwin" ] && [ "$(uname -m)" = "arm64" ] || {
  echo "build host must be macOS arm64" >&2
  exit 1
}
for tool in "$cargo" "$rustc" "$rustdoc" "$cc" "$lld"; do
  [ -x "$tool" ] || { echo "required tool missing: $tool" >&2; exit 1; }
done
case $("$cargo" --version) in "cargo 1.98.1 "*) ;; *) echo "Cargo 1.98.1 required" >&2; exit 1 ;; esac
case $("$rustc" --version) in "rustc 1.98.1 "*) ;; *) echo "rustc 1.98.1 required" >&2; exit 1 ;; esac
cc_version=$("$cc" --version | /usr/bin/head -n 1)
[ "$cc_version" = "Apple clang version 21.0.0 (clang-2100.3.34.2)" ] || {
  echo "unexpected C linker driver: $cc_version" >&2
  exit 1
}
case $("$lld" --version) in "Homebrew LLD 19.1.7"*) ;; *) echo "Homebrew LLD 19.1.7 required" >&2; exit 1 ;; esac

PATH="/usr/bin:/opt/homebrew/bin:$PATH"
export PATH
[ "$(command -v cc)" = "$cc" ] && [ "$(command -v ld.lld)" = "$lld" ] || {
  echo "linker resolution differs from the pinned clang/LLD pair" >&2
  exit 1
}

cargo_home="${NAGI_CARGO_STORE:-$HOME/.cargo}"
target_dir="$script_dir/target-notes-elf-rust198-$label"
if [ -e "$target_dir" ]; then
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
export CARGO_RESOLVER_LOCKFILE_PATH="$script_dir/Cargo.lock"
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
export RUSTFLAGS='--cfg getrandom_backend="linux_getrandom" -C link-arg=--target=x86_64-unknown-linux-musl -C link-arg=-fuse-ld=lld'
features=dev-host-preview
if [ "$recovery_ui_smoke" = true ]; then
  features=dev-host-preview,linux-recovery-ui-smoke
fi
set -- "$cargo" build \
  --verbose \
  --manifest-path "$script_dir/Cargo.toml" \
  -p nagi-linux-notes-desktop-dev --locked \
  --release --target x86_64-unknown-linux-musl --features "$features"
if [ "$online" = false ]; then
  set -- "$@" --offline
fi
echo "rustc=$("$rustc" --version)"
echo "cargo=$("$cargo" --version)"
echo "cc=$cc_version"
echo "lld=$("$lld" --version)"
echo "effective_cargo=$cargo rustc=$RUSTC rustdoc=$RUSTDOC linker=$CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER"
echo "effective_rustflags=$RUSTFLAGS encoded_rustflags=unset wrappers=disabled"
echo "cargo_home=$CARGO_HOME target_dir=$CARGO_TARGET_DIR target=x86_64-unknown-linux-musl"
echo "recovery_ui_smoke=$recovery_ui_smoke features=$features"
if [ "$plan" = true ]; then
  printf 'command='
  printf ' %s' "$@"
  printf '\n'
  echo "output=$target_dir/x86_64-unknown-linux-musl/release/nagi-linux-notes-desktop-dev"
  exit 0
fi
mkdir -p "$cargo_home"
"$@"

output="$target_dir/x86_64-unknown-linux-musl/release/nagi-linux-notes-desktop-dev"
[ -f "$output" ] || { echo "Notes ELF missing: $output" >&2; exit 1; }
description=$(/usr/bin/file "$output")
printf '%s\n' "$description" | /usr/bin/grep -Eq 'ELF 64-bit LSB pie executable, x86-64.*static-pie linked' || {
  echo "unexpected Notes ELF format: $description" >&2
  exit 1
}
echo "notes_elf_build=PASS output=$output"
