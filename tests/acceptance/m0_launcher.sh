#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)

if invalid_output=$("$repository_root/nagi" doctor --unexpected 2>&1); then
    status=0
else
    status=$?
fi
if [ "$status" -ne 2 ]; then
    printf '%s\n' "$invalid_output" >&2
    printf 'FAIL M0 POSIX launcher usage exit: %s\n' "$status" >&2
    exit 1
fi

if help_output=$("$repository_root/nagi" --help 2>&1); then
    status=0
else
    status=$?
fi
if [ "$status" -ne 0 ]; then
    printf '%s\n' "$help_output" >&2
    printf 'FAIL M0 POSIX launcher help success exit: %s\n' "$status" >&2
    exit 1
fi
if ! printf '%s\n' "$help_output" | grep -Fq 'Nagi OS developer orchestrator'; then
    printf '%s\n' "$help_output" >&2
    printf '%s\n' 'FAIL M0 POSIX launcher help output was not recognized' >&2
    exit 1
fi

printf '%s\n' 'PASS M0 POSIX launcher exit propagation'
