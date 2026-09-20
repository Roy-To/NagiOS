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
    printf '%s\n' "FAIL M8 clean exited with $clean_exit" >&2
    exit "$clean_exit"
fi

set +e
run_output=$(cd "$repository_root" && printf '%s\n' \
    'pwd' \
    'ls' \
    'cat nagi-persistent.txt' \
    'nagi ps' \
    'nagi mem' \
    'nagi log' \
    'exit' | ./nagi shell 2>&1)
run_exit=$?
set -e
printf '%s\n' "$run_output"
if [ "$run_exit" -ne 0 ]; then
    printf '%s\n' "FAIL M8 shell launcher exited with $run_exit" >&2
    exit "$run_exit"
fi

serial_log="$repository_root/out/logs/m8-shell.log"
if [ ! -f "$serial_log" ]; then
    printf '%s\n' "FAIL M8 serial log was not created: $serial_log" >&2
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
    'Nagi M8 nsh START' \
    'Nagi M8 pwd PASS' \
    'Nagi M8 ls PASS' \
    'Nagi M8 cat PASS' \
    'Nagi M8 ps PASS' \
    'Nagi M8 mem PASS' \
    'Nagi M8 log PASS' \
    'Nagi M8 acceptance PASS'; do
    line=$(printf '%s\n' "$serial" | awk -v marker="$marker" -v start="$last_line" '
        NR > start && index($0, marker) > 0 { print NR; exit }
    ')
    if [ -z "$line" ]; then
        printf 'FAIL M8 serial log does not contain ordered marker: %s\n' "$marker" >&2
        exit 4
    fi
    last_line=$line
done

if ! grep -Fq 'Nagi OS persistent storage' "$serial_log"; then
    printf '%s\n' 'FAIL M8 cat did not print the real persistent guest file payload' >&2
    exit 4
fi
if ! grep -Fq 'nagi-persistent.txt' "$serial_log"; then
    printf '%s\n' 'FAIL M8 ls did not print the real persistent guest directory entry' >&2
    exit 4
fi
printf '%s\n' 'PASS M8 acceptance: real Nagi nsh inspected guest files and process/memory/log state'
