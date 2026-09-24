#!/usr/bin/env sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repository_root"

if [ -f out/mesa-venv/bin/activate ]; then
    # The pinned Python generators are part of the guest Mesa build toolchain.
    . out/mesa-venv/bin/activate
fi

m17_status=0
output=$(./nagi m17 2>&1) || m17_status=$?
printf '%s\n' "$output"
if [ "$m17_status" -ne 0 ]; then
    printf 'FAIL M17 command exited with status %s\n' "$m17_status" >&2
    exit "$m17_status"
fi
printf '%s\n' "$output" | grep -F 'PASS M17 first web pixel:' >/dev/null

serial_log="$repository_root/out/logs/m17-servo.log"
if [ ! -f "$serial_log" ]; then
    printf 'FAIL M17 serial log was not created: %s\n' "$serial_log" >&2
    exit 1
fi
grep -F 'Nagi M17 first web pixel checksum=0x' "$serial_log" >/dev/null
grep -F 'Nagi M17 first web pixel PASS' "$serial_log" >/dev/null
printf '%s\n' 'PASS M17 first web pixel acceptance: real Servo guest frame reached Nagi Surface and QEMU'
