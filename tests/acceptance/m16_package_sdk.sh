#!/usr/bin/env sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repository_root"

./nagi clean
output=$(./nagi m16 2>&1)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -F 'PASS M16 package/SDK:' >/dev/null
printf '%s\n' 'PASS M16 package SDK acceptance wrapper'
