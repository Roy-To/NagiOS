#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
fixture_root="$repository_root/tests/fixtures/m12"
python_cmd=$(command -v python.exe || command -v python3 || command -v python)
server_pid=''
cleanup() {
    if [ -n "$server_pid" ]; then
        kill "$server_pid" 2>/dev/null || true
        wait "$server_pid" 2>/dev/null || true
    fi
}
trap cleanup EXIT

"$python_cmd" -m http.server 18080 --bind 0.0.0.0 --directory "$fixture_root" >/tmp/nagi-m13-http.log 2>&1 &
server_pid=$!
sleep 0.5

set +e
clean_output=$(cd "$repository_root" && ./nagi clean 2>&1)
clean_exit=$?
set -e
printf '%s\n' "$clean_output"
if [ "$clean_exit" -ne 0 ]; then
    printf 'FAIL M13 clean exited with %s\n' "$clean_exit" >&2
    exit "$clean_exit"
fi

set +e
run_output=$(cd "$repository_root" && ./nagi posix 2>&1)
run_exit=$?
set -e
printf '%s\n' "$run_output"
if [ "$run_exit" -ne 0 ]; then
    printf 'FAIL M13 posix launcher exited with %s\n' "$run_exit" >&2
    exit "$run_exit"
fi

serial_log="$repository_root/out/logs/m13-posix.log"
if [ ! -f "$serial_log" ]; then
    printf 'FAIL M13 serial log was not created: %s\n' "$serial_log" >&2
    exit 1
fi

last_line=0
for marker in \
    'Nagi Kernel started' \
    'Nagi M2 acceptance PASS' \
    'Nagi M3 acceptance PASS' \
    'Nagi M4 acceptance PASS' \
    'Nagi M7 VirtIO Block PASS' \
    'Nagi M12 VirtIO Net PASS' \
    'Nagi M5 user process START' \
    'Nagi M6 echo@1 call PASS' \
    'Nagi M7 ext2 mount PASS' \
    'Nagi M7 persistent read PASS' \
    'Nagi M5 syscall PASS' \
    'Nagi M6 acceptance PASS' \
    'Nagi M7 acceptance PASS' \
    'Nagi M13 Rust PAL PASS' \
    'Nagi M13 C POSIX PASS' \
    'Nagi M13 socket/DNS PASS' \
    'Nagi M13 mmap PASS' \
    'Nagi M13 time/sleep PASS' \
    'Nagi M13 poll PASS' \
    'Nagi M13 thread/TLS PASS' \
    'Nagi M13 native spawn PASS' \
    'Nagi M13 relibc C PASS' \
    'Nagi M13 OSS library PASS' \
    'Nagi M13 acceptance PASS'; do
    found_line=$(awk -v start="$last_line" -v needle="$marker" 'NR > start && index($0, needle) { print NR; exit }' "$serial_log")
    if [ -z "$found_line" ]; then
        printf 'FAIL M13 serial log does not contain ordered marker: %s\n' "$marker" >&2
        exit 1
    fi
    last_line=$found_line
done

printf '%s\n' 'PASS M13 acceptance: real QEMU guest exercised Nagi PAL, C POSIX, and relibc over guest VFS/network'
