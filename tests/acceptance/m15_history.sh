#!/usr/bin/env sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repository_root"

./nagi clean
output=$(./nagi m15 2>&1)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -F 'PASS M15 history: real guest create/edit/move/delete/restore/undo and persistent ledger passed' >/dev/null
printf '%s\n' 'PASS M15 history acceptance wrapper'
