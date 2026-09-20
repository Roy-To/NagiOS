#!/usr/bin/env sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repository_root"

./nagi clean
output=$(./nagi m13 2>&1)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -F 'PASS M13 unified acceptance: real QEMU POSIX/relibc and Rust std gates passed' >/dev/null
printf '%s\n' 'PASS M13 unified acceptance wrapper'
