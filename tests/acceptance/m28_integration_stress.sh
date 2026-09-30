#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
Usage: m28_integration_stress.sh [--dry-run|--run|--self-test]

NAGI_M28_REPEAT_COUNT selects 1 through 5 repetitions (default: 2).
--dry-run validates existing M19/M22 serial logs and reports write collisions.
--run refuses to start if a named image, OVMF vars copy, or serial log exists.
--self-test checks repeat-count and serial-marker validation without QEMU.
EOF
}

fail() {
    printf 'FAIL M28 harness: %s\n' "$*" >&2
    exit 1
}

validate_repeat_count() {
    case "$1" in
        1|2|3|4|5) return 0 ;;
        *) return 1 ;;
    esac
}

validate_user_data_disk_size() {
    case "$1" in
        16777216|18874368) return 0 ;;
        *) return 1 ;;
    esac
}

validate_log() {
    log_path=$1
    shift
    if [ ! -f "$log_path" ]; then
        printf 'missing serial log: %s\n' "$log_path" >&2
        return 1
    fi
    for marker do
        if ! grep -F "$marker" "$log_path" >/dev/null 2>&1; then
            printf 'missing PASS marker `%s` in %s\n' "$marker" "$log_path" >&2
            return 1
        fi
    done
}

validate_m19_log() {
    validate_log "$1" \
        'Nagi Kernel started' \
        'Nagi M3 acceptance PASS' \
        'Nagi M7 VirtIO Block PASS' \
        'Nagi M13 Rust PAL PASS' \
        'Nagi M13 C POSIX PASS' \
        'Nagi M24 semantic index ready PASS' \
        'Nagi M24 semantic index persistence PASS' \
        'Nagi M19 live VFS file ObjectId rename/restart PASS' \
        'Nagi M19 previous-boot snapshot PASS' \
        'Nagi M19 guest search persistence PASS' \
        'Nagi M19 acceptance PASS' \
        'Nagi M13 acceptance PASS'
}

validate_m22_log() {
    validate_log "$1" \
        'Nagi Kernel started' \
        'Nagi M3 acceptance PASS' \
        'Nagi M7 VirtIO Block PASS' \
        'Nagi M13 C POSIX PASS' \
        'Nagi M24 semantic index ready PASS' \
        'Nagi M24 semantic index persistence PASS' \
        'Nagi M19 guest search persistence PASS' \
        'Nagi M22 AI Activity Ledger undo result PASS' \
        'Nagi M22 archive restart and restored files PASS' \
        'Nagi M13 acceptance PASS'
}

print_unmeasured_workload() {
    cat <<'EOF'
M28 reference-load items not measured by this Search/History slice:
  - Desktop and Files usability while the combined workload runs
  - Notes app activity and Albert with 3–5 concurrent browser tabs
  - real Granite inference, unload/reload, and CPU fairness under load
  - sustained audio playback and audio-underrun pressure
  - kernel OOM, handle growth, and memory-leak soak telemetry
EOF
}

self_test() {
    temporary_dir=$(mktemp -d "${TMPDIR:-/tmp}/nagi-m28-harness.XXXXXX") || fail 'mktemp failed'
    trap 'rm -rf "$temporary_dir"' EXIT HUP INT TERM

    validate_repeat_count 1 || fail 'repeat count 1 rejected'
    validate_repeat_count 5 || fail 'repeat count 5 rejected'
    if validate_repeat_count 0 || validate_repeat_count 6 || validate_repeat_count many; then
        fail 'repeat-count bounds accepted an invalid value'
    fi
    validate_user_data_disk_size 16777216 || fail 'legacy 16 MiB disk size rejected'
    validate_user_data_disk_size 18874368 || fail 'current 18 MiB GPT disk size rejected'
    if validate_user_data_disk_size 16777215 || validate_user_data_disk_size 18874369; then
        fail 'unsupported persistent disk size accepted'
    fi

    cat >"$temporary_dir/m19.log" <<'EOF'
Nagi Kernel started
Nagi M3 acceptance PASS
Nagi M7 VirtIO Block PASS
Nagi M13 Rust PAL PASS
Nagi M13 C POSIX PASS
Nagi M24 semantic index ready PASS
Nagi M24 semantic index persistence PASS
Nagi M19 live VFS file ObjectId rename/restart PASS
Nagi M19 previous-boot snapshot PASS
Nagi M19 guest search persistence PASS
Nagi M19 acceptance PASS
Nagi M13 acceptance PASS
EOF
    cat >"$temporary_dir/m22.log" <<'EOF'
Nagi Kernel started
Nagi M3 acceptance PASS
Nagi M7 VirtIO Block PASS
Nagi M13 C POSIX PASS
Nagi M24 semantic index ready PASS
Nagi M24 semantic index persistence PASS
Nagi M19 guest search persistence PASS
Nagi M22 AI Activity Ledger undo result PASS
Nagi M22 archive restart and restored files PASS
Nagi M13 acceptance PASS
EOF
    validate_m19_log "$temporary_dir/m19.log" || fail 'valid M19 fixture rejected'
    validate_m22_log "$temporary_dir/m22.log" || fail 'valid M22 fixture rejected'
    printf 'Nagi M19 guest search persistence PASS\n' >"$temporary_dir/incomplete.log"
    if validate_m19_log "$temporary_dir/incomplete.log" >/dev/null 2>&1; then
        fail 'incomplete M19 log accepted'
    fi
    printf 'PASS M28 harness self-test (marker checks and 1–5 repeat bounds; no QEMU)\n'
}

mode=${1:---dry-run}
if [ "$#" -gt 1 ]; then
    usage >&2
    exit 2
fi
case "$mode" in
    --dry-run|--run|--self-test) ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
esac

repeat_count=${NAGI_M28_REPEAT_COUNT:-2}
if ! validate_repeat_count "$repeat_count"; then
    fail "NAGI_M28_REPEAT_COUNT must be an integer from 1 through 5 (got $repeat_count)"
fi

if [ "$mode" = --self-test ]; then
    self_test
    exit 0
fi

script_dir=$(CDPATH= cd "$(dirname "$0")" && pwd)
repo_root=$(git -C "$script_dir/../.." rev-parse --show-toplevel 2>/dev/null) \
    || fail 'cannot locate repository root with git rev-parse'
cd "$repo_root"

m19_disk=out/artifacts/nagi-0.1-m19-vfs-objectid-user-data.img
m22_disk=out/artifacts/nagi-0.1-m22-history-user-data.img
m19_initial_log=out/logs/m19-vfs-objectid-initial.log
m19_log=out/logs/m19-vfs-objectid-restart.log
m22_log=out/logs/m22-history-boot-3.log

latest_m19_log() {
    if [ -f "$m19_log" ]; then
        printf '%s\n' "$m19_log"
    else
        printf '%s\n' "$m19_initial_log"
    fi
}

validate_latest_m19_log() {
    validate_m19_log "$(latest_m19_log)"
}

for disk in "$m19_disk" "$m22_disk"; do
    [ -f "$disk" ] || fail "required existing persistent guest disk is missing: $disk"
    disk_bytes=$(wc -c <"$disk" | tr -d '[:space:]')
    validate_user_data_disk_size "$disk_bytes" || fail "unexpected persistent disk size ($disk_bytes bytes; expected legacy 16 MiB or current 18 MiB GPT): $disk"
done

printf 'M28 slice repository: %s\n' "$repo_root"
printf 'M28 gate repetitions: %s\n' "$repeat_count"
printf 'Independent persistent disks: %s and %s\n' "$m19_disk" "$m22_disk"

if [ "$mode" = --dry-run ]; then
    if validate_latest_m19_log; then
        printf 'PASS existing M19 latest serial log: %s\n' "$(latest_m19_log)"
    else
        printf 'UNVERIFIED existing M19 latest serial log: %s\n' "$(latest_m19_log)"
    fi
    if validate_m22_log "$m22_log"; then
        printf 'PASS existing M22 latest serial log: %s\n' "$m22_log"
    else
        printf 'UNVERIFIED existing M22 latest serial log: %s\n' "$m22_log"
    fi
    printf 'Planned real gate invocations (not executed):\n'
    iteration=1
    while [ "$iteration" -le "$repeat_count" ]; do
        printf '  %s: ./nagi m19, then ./nagi m22\n' "$iteration"
        iteration=$((iteration + 1))
    done
    printf 'Named outputs that --run would overwrite if present:\n'
    collision=0
    for output in \
        out/artifacts/nagi-0.1-m19-vfs-objectid.img \
        out/artifacts/nagi-0.1-m19-vfs-objectid-vars.fd \
        out/logs/m19-vfs-objectid-bootstrap.log \
        out/logs/m19-vfs-objectid-initial.log \
        out/logs/m19-vfs-objectid-restart.log \
        out/artifacts/nagi-0.1-m22-history.img \
        out/artifacts/nagi-0.1-m22-history-vars.fd \
        out/logs/m22-history-boot-1.log \
        out/logs/m22-history-boot-2.log \
        out/logs/m22-history-boot-3.log; do
        if [ -e "$output" ]; then
            printf '  %s\n' "$output"
            collision=1
        fi
    done
    if [ "$collision" -eq 0 ]; then
        printf '  none\n'
    else
        printf 'Actual repetitions are guarded until those existing outputs are absent.\n'
    fi
    print_unmeasured_workload
    printf 'PASS M28 dry-run (no build, disk write, or QEMU invocation)\n'
    exit 0
fi

collisions=''
for output in \
    out/artifacts/nagi-0.1-m19-vfs-objectid.img \
    out/artifacts/nagi-0.1-m19-vfs-objectid-vars.fd \
    out/logs/m19-vfs-objectid-bootstrap.log \
    out/logs/m19-vfs-objectid-initial.log \
    out/logs/m19-vfs-objectid-restart.log \
    out/artifacts/nagi-0.1-m22-history.img \
    out/artifacts/nagi-0.1-m22-history-vars.fd \
    out/logs/m22-history-boot-1.log \
    out/logs/m22-history-boot-2.log \
    out/logs/m22-history-boot-3.log; do
    if [ -e "$output" ]; then
        collisions="$collisions\n  $output"
    fi
done
if [ -n "$collisions" ]; then
    printf 'Refusing --run; it would overwrite existing acceptance outputs:%b\n' "$collisions" >&2
    exit 1
fi

archive_iteration_outputs() {
    archived_iteration=$1
    archive_dir="out/evidence/m28-repetition-$archived_iteration"
    [ ! -e "$archive_dir" ] || fail "refusing to replace existing repetition evidence: $archive_dir"
    mkdir -p "$archive_dir"
    cp -p "$m19_disk" "$archive_dir/"
    cp -p "$m22_disk" "$archive_dir/"
    for output in \
        out/artifacts/nagi-0.1-m19-vfs-objectid.img \
        out/artifacts/nagi-0.1-m19-vfs-objectid-vars.fd \
        out/logs/m19-vfs-objectid-bootstrap.log \
        "$m19_initial_log" \
        "$m19_log" \
        out/artifacts/nagi-0.1-m22-history.img \
        out/artifacts/nagi-0.1-m22-history-vars.fd \
        out/logs/m22-history-boot-1.log \
        out/logs/m22-history-boot-2.log \
        "$m22_log"; do
        if [ -e "$output" ]; then
            mv "$output" "$archive_dir/" || fail "cannot preserve repetition output: $output"
        fi
    done
}

iteration=1
while [ "$iteration" -lt "$repeat_count" ]; do
    archive_dir="out/evidence/m28-repetition-$iteration"
    [ ! -e "$archive_dir" ] || fail "refusing to replace existing repetition evidence: $archive_dir"
    iteration=$((iteration + 1))
done

if [ ! -x ./nagi ]; then
    fail 'repository ./nagi entry point is missing or not executable'
fi

iteration=1
while [ "$iteration" -le "$repeat_count" ]; do
    printf 'M28 repetition %s/%s: running ./nagi m19\n' "$iteration" "$repeat_count"
    ./nagi m19 || fail "./nagi m19 failed on repetition $iteration"
    validate_latest_m19_log || fail "M19 boot log failed on repetition $iteration"
    printf 'PASS M28 repetition %s M19 serial gate: %s\n' "$iteration" "$(latest_m19_log)"

    printf 'M28 repetition %s/%s: running ./nagi m22\n' "$iteration" "$repeat_count"
    ./nagi m22 || fail "./nagi m22 failed on repetition $iteration"
    validate_m22_log "$m22_log" || fail "M22 final serial gate failed on repetition $iteration"
    printf 'PASS M28 repetition %s M22 serial gate: %s\n' "$iteration" "$m22_log"
    if [ "$iteration" -lt "$repeat_count" ]; then
        archive_iteration_outputs "$iteration"
    fi
    iteration=$((iteration + 1))
done

print_unmeasured_workload
printf 'PASS M28 persistent Search/NH16 restart-gate repetitions: %s\n' "$repeat_count"
