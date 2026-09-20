#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)

set +e
clean_output=$(cd "$repository_root" && ./nagi clean 2>&1)
clean_exit=$?
set -e
printf '%s\n' "$clean_output"
if [ "$clean_exit" -ne 0 ]; then
    printf 'FAIL M11 clean exited with %s\n' "$clean_exit" >&2
    exit "$clean_exit"
fi

set +e
run_output=$(cd "$repository_root" && ./nagi security 2>&1)
run_exit=$?
set -e
printf '%s\n' "$run_output"
if [ "$run_exit" -ne 0 ]; then
    printf 'FAIL M11 security launcher exited with %s\n' "$run_exit" >&2
    exit "$run_exit"
fi

serial_log="$repository_root/out/logs/m11-security.log"
if [ ! -f "$serial_log" ]; then
    printf 'FAIL M11 serial log was not created: %s\n' "$serial_log" >&2
    exit 4
fi

serial=$(sed 's/\r$//' "$serial_log")
last_line=0
for marker in \
    'Nagi Kernel started' \
    'Nagi M2 acceptance PASS' \
    'Nagi M3 acceptance PASS' \
    'Nagi M4 acceptance PASS' \
    'Nagi M7 VirtIO Block PASS' \
    'Nagi M5 user process START' \
    'Nagi M6 echo@1 call PASS' \
    'Nagi M7 ext2 mount PASS' \
    'Nagi M7 persistent read PASS' \
    'Nagi M5 syscall PASS' \
    'Nagi M6 acceptance PASS' \
    'Nagi M7 acceptance PASS' \
    'Nagi M11 local login PASS' \
    'Nagi M11 lock screen PASS' \
    'Nagi M11 Developer Mode PASS' \
    'Nagi M11 trusted dialog ASK PASS' \
    'Nagi M11 malicious file DENIED' \
    'Nagi M11 malicious microphone DENIED' \
    'Nagi M11 acceptance PASS'; do
    line=$(printf '%s\n' "$serial" | awk -v marker="$marker" -v start="$last_line" '
        NR > start && index($0, marker) > 0 { print NR; exit }
    ')
    if [ -z "$line" ]; then
        printf 'FAIL M11 serial log does not contain ordered marker: %s\n' "$marker" >&2
        exit 4
    fi
    last_line=$line
done

printf '%s\n' 'PASS M11 acceptance: real QEMU guest enforced login, lock screen, and Permission Broker denial'
