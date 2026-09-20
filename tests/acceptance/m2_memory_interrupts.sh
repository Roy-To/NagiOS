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
    printf '%s\n' "FAIL M2 QEMU launcher exited with $exit_code" >&2
    exit "$exit_code"
fi

serial_log="$repository_root/out/logs/m1-qemu-boot.log"
if [ ! -f "$serial_log" ]; then
    printf '%s\n' "FAIL M2 serial log was not created: $serial_log" >&2
    exit 4
fi
for marker in \
    'Nagi Kernel started' \
    'Nagi M2 page allocation/free PASS' \
    'Nagi M2 timer interrupts PASS' \
    'Nagi Page fault handled (vector 14)' \
    'Nagi invalid access diagnostic PASS' \
    'Nagi M2 acceptance PASS'
do
    if ! grep -Fq "$marker" "$serial_log"; then
        printf 'FAIL M2 serial log does not contain required marker: %s\n' "$marker" >&2
        exit 4
    fi
done
printf '%s\n' 'PASS M2 acceptance: real QEMU guest passed memory, timer, and page-fault checks'
