#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
if output=$("$repository_root/nagi" doctor 2>&1); then
    exit_code=0
else
    exit_code=$?
fi
printf '%s\n' "$output"
if [ "$exit_code" -ne 0 ]; then
    printf '%s\n' "FAIL M0 POSIX doctor acceptance: exit $exit_code" >&2
    exit "$exit_code"
fi

for dependency in \
    'Git' 'Rust (rustc)' 'Cargo' 'Rustup' 'LLVM/Clang' 'LLD' 'QEMU' \
    'OVMF CODE/VARS' 'CMake' 'Meson' 'Ninja' 'Python'; do
    if ! printf '%s\n' "$output" | grep -Fq "PASS $dependency:"; then
        printf '%s\n' "FAIL M0 POSIX doctor acceptance: missing PASS for $dependency" >&2
        exit 10
    fi
done
if ! printf '%s\n' "$output" | grep -Fq 'PASS doctor:'; then
    printf '%s\n' 'FAIL M0 POSIX doctor acceptance: missing PASS summary' >&2
    exit 10
fi
printf '%s\n' 'PASS M0 POSIX doctor acceptance'

