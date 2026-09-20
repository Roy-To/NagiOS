#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
set +e
output=$(cd "$repository_root" && ./nagi run 2>&1)
exit_code=$?
set -e
printf '%s\n' "$output"
if [ "$exit_code" -ne 0 ]; then
    printf '%s\n' "FAIL M1 QEMU launcher exited with $exit_code" >&2
    exit "$exit_code"
fi

serial_log="$repository_root/out/logs/m1-qemu-boot.log"
if [ ! -f "$serial_log" ]; then
    printf '%s\n' "FAIL M1 serial log was not created: $serial_log" >&2
    exit 4
fi
if ! grep -Fq 'Nagi Kernel started' "$serial_log"; then
    printf '%s\n' 'FAIL M1 serial log does not contain the kernel acceptance line' >&2
    exit 4
fi
printf '%s\n' 'PASS M1 acceptance: QEMU serial log contains Nagi Kernel started'
