#!/usr/bin/env sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repository_root"

./nagi clean
output=$(./nagi m14 2>&1)
printf '%s\n' "$output"
printf '%s\n' "$output" | grep -F 'PASS M14 audio: real VirtIO Sound playback, capture, mixer, volume/mute, and session gates passed' >/dev/null
printf '%s\n' 'PASS M14 audio acceptance wrapper'
