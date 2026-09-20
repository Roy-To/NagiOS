#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)

if "$repository_root/nagi" doctor --unexpected >/dev/null 2>&1; then
    status=0
else
    status=$?
fi
if [ "$status" -ne 2 ]; then
    printf 'FAIL M0 POSIX launcher usage exit: %s\n' "$status" >&2
    exit 1
fi

if image_output=$("$repository_root/nagi" image 2>&1); then
    status=0
else
    status=$?
fi
if [ "$status" -ne 0 ]; then
    printf '%s\n' "$image_output" | sed -n '1,160p' >&2
    printf '%s\n' "$image_output" | tail -n 160 >&2
    printf 'FAIL M0 POSIX launcher image success exit: %s\n' "$status" >&2
    exit 1
fi

printf '%s\n' 'PASS M0 POSIX launcher exit propagation'
