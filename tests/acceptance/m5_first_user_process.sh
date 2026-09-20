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
    printf '%s\n' "FAIL M5 QEMU launcher exited with $exit_code" >&2
    exit "$exit_code"
fi

serial_log="$repository_root/out/logs/m1-qemu-boot.log"
if [ ! -f "$serial_log" ]; then
    printf '%s\n' "FAIL M5 serial log was not created: $serial_log" >&2
    exit 4
fi

serial=$(sed 's/\r$//' "$serial_log")
last_line=0
check_marker() {
    marker=$1
    line=$(printf '%s\n' "$serial" | awk -v marker="$marker" -v start="$last_line" '
        NR > start && (marker == "Hello from user space" ? $0 == marker : index($0, marker) > 0) {
            print NR
            exit
        }
    ')
    if [ -z "$line" ]; then
        printf 'FAIL M5 serial log does not contain the ordered marker: %s\n' "$marker" >&2
        exit 4
    fi
    last_line=$line
}

check_marker 'Nagi Kernel started'
check_marker 'Nagi M2 page allocation/free PASS'
check_marker 'Nagi M3 ACPI discovery PASS'
check_marker 'Nagi M2 timer interrupts PASS'
check_marker 'Nagi Page fault handled (vector 14)'
check_marker 'Nagi invalid access diagnostic PASS'
check_marker 'Nagi M2 acceptance PASS'
check_marker 'Nagi Page fault resume PASS'
check_marker 'Nagi M3 SMP startup START'
check_marker 'Nagi M3 CPU 0 online/workload PASS'
check_marker 'Nagi M3 CPU 1 online/workload PASS'
check_marker 'Nagi M3 CPU 2 online/workload PASS'
check_marker 'Nagi M3 CPU 3 online/workload PASS'
check_marker 'Nagi M3 scheduler workloads PASS'
check_marker 'Nagi M3 acceptance PASS'
check_marker 'Nagi M4 handles/VMO/IPC START'
check_marker 'Nagi M4 VMO basics PASS'
check_marker 'Nagi M4 channel round-trip PASS'
check_marker 'Nagi M4 rights attenuation PASS'
check_marker 'Nagi M4 wait primitives PASS'
check_marker 'Nagi M4 acceptance PASS'
check_marker 'Nagi M5 user process START'
check_marker 'Nagi M5 FPU state initial PASS'
check_marker 'Hello from user space'
check_marker 'Nagi M5 FPU state round-trip PASS'
check_marker 'Nagi M5 syscall PASS'
check_marker 'Nagi M5 acceptance PASS'

printf '%s\n' 'PASS M5 acceptance: real user INIT.ELF printed through Nagi SYSCALL and exited successfully'
