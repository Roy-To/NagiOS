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
    printf '%s\n' "FAIL M7 clean exited with $clean_exit" >&2
    exit "$clean_exit"
fi

set +e
run_output=$(cd "$repository_root" && ./nagi run 2>&1)
run_exit=$?
set -e
printf '%s\n' "$run_output"
if [ "$run_exit" -ne 0 ]; then
    printf '%s\n' "FAIL M7 QEMU launcher exited with $run_exit" >&2
    exit "$run_exit"
fi

first_log="$repository_root/out/logs/m7-first-boot.log"
final_log="$repository_root/out/logs/m1-qemu-boot.log"
data_disk="$repository_root/out/artifacts/nagi-0.1-user-data.img"
for path in "$first_log" "$final_log" "$data_disk"; do
    if [ ! -f "$path" ]; then
        printf 'FAIL M7 required output was not created: %s\n' "$path" >&2
        exit 4
    fi
done

disk_size=$(wc -c < "$data_disk" | tr -d '[:space:]')
if [ "$disk_size" -ne 16777216 ]; then
    printf 'FAIL M7 persistent data disk has size %s, expected 16777216\n' "$disk_size" >&2
    exit 4
fi

check_ordered_markers() {
    log_path=$1
    label=$2
    shift 2
    serial=$(sed 's/\r$//' "$log_path")
    last_line=0
    for marker in "$@"; do
        line=$(printf '%s\n' "$serial" | awk -v marker="$marker" -v start="$last_line" '
            NR > start && (marker == "Hello from user space" ? $0 == marker : index($0, marker) > 0) {
                print NR
                exit
            }
        ')
        if [ -z "$line" ]; then
            printf 'FAIL %s does not contain the ordered marker: %s\n' "$label" "$marker" >&2
            exit 4
        fi
        last_line=$line
    done
}

common_markers='Nagi Kernel started
Nagi M2 page allocation/free PASS
Nagi M3 ACPI discovery PASS
Nagi M2 timer interrupts PASS
Nagi Page fault handled (vector 14)
Nagi invalid access diagnostic PASS
Nagi M2 acceptance PASS
Nagi Page fault resume PASS
Nagi M3 SMP startup START
Nagi M3 CPU 0 online/workload PASS
Nagi M3 CPU 1 online/workload PASS
Nagi M3 CPU 2 online/workload PASS
Nagi M3 CPU 3 online/workload PASS
Nagi M3 scheduler workloads PASS
Nagi M3 acceptance PASS
Nagi M4 handles/VMO/IPC START
Nagi M4 VMO basics PASS
Nagi M4 channel round-trip PASS
Nagi M4 rights attenuation PASS
Nagi M4 wait primitives PASS
Nagi M4 acceptance PASS
Nagi M7 storage START
Nagi M7 VirtIO Block PASS
Nagi M5 user process START
Nagi M5 FPU state initial PASS
Hello from user space
Nagi M5 FPU state round-trip PASS
Nagi M6 supervisor START
Nagi M6 manifest dependency order PASS
Nagi M6 service health PASS
Nagi M6 service registry START
Nagi M6 echo@1 call PASS'

set --
while IFS= read -r marker; do set -- "$@" "$marker"; done <<EOF
$common_markers
Nagi M7 ext2 format PASS
Nagi M7 file create PASS
Nagi M7 file write PASS
Nagi M7 file-backed mmap PASS
Nagi M7 persistent write PASS
EOF
check_ordered_markers "$first_log" 'M7 first-boot serial log' "$@"

set --
while IFS= read -r marker; do set -- "$@" "$marker"; done <<EOF
$common_markers
Nagi M7 ext2 mount PASS
Nagi M7 directory lookup PASS
Nagi M7 file read PASS
Nagi M7 file-backed mmap PASS
Nagi M7 persistent read PASS
Nagi M5 syscall PASS
Nagi M5 acceptance PASS
Nagi M6 acceptance PASS
Nagi M7 acceptance PASS
EOF
check_ordered_markers "$final_log" 'M7 final serial log' "$@"

printf '%s\n' 'PASS M7 acceptance: ext2 file data survived a real QEMU reboot through the VirtIO Block disk'
