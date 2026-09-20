#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
fixture_root="$repository_root/tests/fixtures/m12"
python_cmd=$(command -v python || command -v python3)
"$python_cmd" -m http.server 18080 --bind 0.0.0.0 --directory "$fixture_root" >/tmp/nagi-m12-http.log 2>&1 &
server_pid=$!
cleanup() {
    kill "$server_pid" 2>/dev/null || true
}
trap cleanup EXIT INT TERM
sleep 1

set +e
clean_output=$(cd "$repository_root" && ./nagi clean 2>&1)
clean_exit=$?
set -e
printf '%s\n' "$clean_output"
if [ "$clean_exit" -ne 0 ]; then
    printf 'FAIL M12 clean exited with %s\n' "$clean_exit" >&2
    exit "$clean_exit"
fi

set +e
run_output=$(cd "$repository_root" && ./nagi network 2>&1)
run_exit=$?
set -e
printf '%s\n' "$run_output"
if [ "$run_exit" -ne 0 ]; then
    printf 'FAIL M12 network launcher exited with %s\n' "$run_exit" >&2
    exit "$run_exit"
fi

serial_log="$repository_root/out/logs/m12-network.log"
if [ ! -f "$serial_log" ]; then
    printf 'FAIL M12 serial log was not created: %s\n' "$serial_log" >&2
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
    'Nagi M12 VirtIO Net PASS' \
    'Nagi M5 user process START' \
    'Nagi M6 echo@1 call PASS' \
    'Nagi M7 ext2 mount PASS' \
    'Nagi M7 persistent read PASS' \
    'Nagi M5 syscall PASS' \
    'Nagi M6 acceptance PASS' \
    'Nagi M7 acceptance PASS' \
    'Nagi M12 network READY' \
    'Nagi M12 DHCP PASS' \
    'Nagi M12 ICMP PASS' \
    'Nagi M12 UDP/DNS PASS' \
    'Nagi M12 ARP PASS' \
    'Nagi M12 TCP handshake PASS' \
    'Nagi M12 HTTP response PASS' \
    'Nagi M12 acceptance PASS'; do
    line=$(printf '%s\n' "$serial" | awk -v marker="$marker" -v start="$last_line" '
        NR > start && index($0, marker) > 0 { print NR; exit }
    ')
    if [ -z "$line" ]; then
        printf 'FAIL M12 serial log does not contain ordered marker: %s\n' "$marker" >&2
        exit 4
    fi
    last_line=$line
done

printf '%s\n' 'PASS M12 acceptance: real QEMU guest performed DHCP, ICMP, UDP/DNS, ARP, TCP, and HTTP through its own nagi-net stack'
