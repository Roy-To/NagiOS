#!/usr/bin/env sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repository_root"

if [ -f out/mesa-venv/bin/activate ]; then
    # Use the pinned host-side Python generators required by the pinned Mesa source.
    . out/mesa-venv/bin/activate
fi

set +e
output=$(./nagi m18a 2>&1)
run_status=$?
set -e
printf '%s\n' "$output"
if [ "$run_status" -ne 0 ]; then
    printf 'FAIL M18-A command exited with status %s\n' "$run_status" >&2
    exit "$run_status"
fi
printf '%s\n' "$output" | grep -F 'PASS M18A remote web:' >/dev/null

serial_log="$repository_root/out/logs/m18a-servo.log"
if [ ! -f "$serial_log" ]; then
    printf 'FAIL M18-A serial log was not created: %s\n' "$serial_log" >&2
    exit 1
fi

for marker in \
    'Nagi M18A trace: network capability initialized' \
    'Nagi M18A trace: TLS fixture CA installed' \
    'Nagi M18A HTTPS download/upload PASS' \
    'Nagi M18A HTTPS untrusted certificate rejection PASS' \
    'Nagi M18A remote navigation fixture identity PASS' \
    'Nagi M18A remote web pixel checksum=0x' \
    'Nagi M18A remote web pixel PASS'; do
    grep -F "$marker" "$serial_log" >/dev/null || {
        printf 'FAIL M18-A serial log is missing marker: %s\n' "$marker" >&2
        exit 1
    }
done

if grep -F 'Nagi M17 first web pixel PASS' "$serial_log" >/dev/null; then
    printf 'FAIL M18-A remote run emitted the M17 local-page PASS marker\n' >&2
    exit 1
fi

printf '%s\n' 'PASS M18-A acceptance: verified transfers and untrusted-TLS rejection reached Nagi Surface'
