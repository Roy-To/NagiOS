#!/bin/sh
set -eu

usage() {
    cat <<'EOF'
Usage: m28_integration_stress.sh [--dry-run|--run|--self-test]

NAGI_M28_REPEAT_COUNT selects 1 through 5 repetitions (default: 2).
--dry-run validates existing M19/M22 serial logs and reports write collisions.
--run also repeats the M27 GPT A/B rollback, Recovery, and promotion gate.
--run refuses to start if a named image, OVMF vars copy, or serial log exists.
--self-test checks repeat-count and serial-marker validation without QEMU.
EOF
}

fail() {
    harness_failure_summary=$*
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

archive_root_for_run() {
    printf 'out/evidence/m28-run-%s\n' "$1"
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
        'Nagi M22 file.search Activity Ledger PASS' \
        'Nagi M22 AI Activity Ledger undo result PASS' \
        'Nagi M22 archive restart and restored files PASS' \
        'Nagi M13 acceptance PASS'
}

extract_m22_log() {
    extracted_log=$(printf '%s\n' "$1" \
        | sed -n 's/^PASS M21\/M22 guest fixture:.*(log \(.*\))$/\1/p' \
        | tail -n 1)
    [ -n "$extracted_log" ] || return 1
    printf '%s\n' "$extracted_log"
}

extract_m22_run_id() {
    extracted_run_id=$(printf '%s\n' "$1" \
        | sed -n 's#.*m22-history-bootstrap-\([0-9][0-9]*\)\.log.*#\1#p' \
        | tail -n 1)
    if [ -z "$extracted_run_id" ]; then
        extracted_run_id=$(printf '%s\n' "$1" \
            | sed -n 's#.*m22-history-\([0-9][0-9]*\)-boot-[123]\.log.*#\1#p' \
            | tail -n 1)
    fi
    if [ -z "$extracted_run_id" ]; then
        extracted_run_id=$(printf '%s\n' "$1" \
            | sed -n 's#.*nagi-0\.1-m22-history-\([0-9][0-9]*\)\.img.*#\1#p' \
            | tail -n 1)
    fi
    [ -n "$extracted_run_id" ] || return 1
    printf '%s\n' "$extracted_run_id"
}

m22_final_boot_log_path() {
    printf 'out/logs/m22-history-%s-boot-3.log' "$1"
}

extract_m27_evidence_path() {
    extracted_path=$(printf '%s\n' "$1" \
        | sed -n 's/^PASS M27 A\/B and Recovery:.*(evidence \(.*\))$/\1/p' \
        | tail -n 1)
    [ -n "$extracted_path" ] || return 1
    printf '%s\n' "$extracted_path"
}

extract_m27_diagnostic_evidence_path() {
    extracted_run_id=$(printf '%s\n' "$1" \
        | sed -n 's#.*m27-ab-rollback-\([0-9][0-9]*\).*#\1#p' \
        | tail -n 1)
    [ -n "$extracted_run_id" ] || return 1
    printf 'out/evidence/m27-ab-rollback-%s\n' "$extracted_run_id"
}

validate_m27_output() {
    case "$1" in
        *'PASS M27 A/B and Recovery:'*) return 0 ;;
        *) printf '%s\n' 'missing M27 A/B and Recovery acceptance result' >&2; return 1 ;;
    esac
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

write_evidence_manifest() {
    evidence_dir=$1
    if ! command -v shasum >/dev/null 2>&1 && ! command -v sha256sum >/dev/null 2>&1; then
        fail 'neither shasum nor sha256sum is available to write evidence checksums'
    fi
    (
        cd "$evidence_dir" || exit 1
        find . -type f ! -name SHA256SUMS -print | LC_ALL=C sort \
            | while IFS= read -r evidence_file; do
                if command -v shasum >/dev/null 2>&1; then
                    shasum -a 256 "$evidence_file"
                else
                    sha256sum "$evidence_file"
                fi
            done > SHA256SUMS
    ) || fail "cannot write evidence manifest: $evidence_dir/SHA256SUMS"
}

write_source_worktree_patch() {
    evidence_dir=$1
    git diff --binary HEAD >"$evidence_dir/source-worktree.diff" \
        || fail "cannot preserve source worktree diff: $evidence_dir/source-worktree.diff"
}

verify_evidence_manifest() {
    evidence_dir=$1
    if command -v shasum >/dev/null 2>&1; then
        (cd "$evidence_dir" && shasum -a 256 -c SHA256SUMS >/dev/null) \
            || fail "evidence checksum verification failed: $evidence_dir/SHA256SUMS"
    else
        (cd "$evidence_dir" && sha256sum -c SHA256SUMS >/dev/null) \
            || fail "evidence checksum verification failed: $evidence_dir/SHA256SUMS"
    fi
}

write_m27_evidence_metadata() {
    evidence_dir=$1
    parent_repetition=$2
    result_status=$3
    result_summary=$4
    if [ ! -e "$evidence_dir/README.md" ] && [ ! -e "$evidence_dir/SHA256SUMS" ]; then
        cat >"$evidence_dir/README.md" <<EOF
# M27 A/B and Recovery QEMU acceptance

- Parent M28 repetition: $parent_repetition
- Source revision: $(git rev-parse HEAD) on $(git branch --show-current)
- Result: $result_status. $result_summary
- QEMU reported that the host has no virtio-sound.in audio driver. This acceptance does not measure audio.
- SHA256SUMS covers this README and every generated file in this run directory.
EOF
        write_evidence_manifest "$evidence_dir"
    fi
    verify_evidence_manifest "$evidence_dir"
}

write_m28_evidence_metadata() {
    evidence_dir=$1
    local_run_id=$2
    source_revision=$3
    local_repeat_count=$4
    m27_evidence_paths=$5
    run_result=$6
    completed_repetitions=$7
    failed_repetition=$8
    failure_summary=$9
    write_source_worktree_patch "$evidence_dir"
    cat >"$evidence_dir/README.md" <<EOF
# M28 persistent Search/History/Recovery gate

- Run namespace: m28-run-$local_run_id
- Source revision: $source_revision on $(git branch --show-current)
- Scope: $local_repeat_count consecutive repetitions of M19 VFS/ObjectId/Search, M22 three-boot Move/Copy + NH16/NAL1 grouped Undo, and M27 GPT A/B/Recovery QEMU acceptance.
- Result: $run_result; $completed_repetitions complete repetition(s) passed. Per-repetition M19/M22 artifacts and logs are preserved in the matching repetition directory.
- This run emitted the host QEMU warning that virtio-sound.in is unavailable; these gates do not measure audio.
- OVMF startup timeouts remain intermittent and unexplained.
- Formal M28 Desktop/Files/Notes/Albert, real Granite inference, audio pressure, OOM, CPU fairness, and leak-soak criteria remain unmeasured; M28 remains PARTIAL.

## M27 sub-run evidence
EOF
    if [ -n "$failure_summary" ]; then
        printf '\n- Failed repetition: %s\n- Failure: %s\n' \
            "$failed_repetition" "$failure_summary" >>"$evidence_dir/README.md"
    fi
    if [ -n "$m27_evidence_paths" ]; then
        printf '%s\n' "$m27_evidence_paths" | while IFS= read -r evidence_path; do
            [ -n "$evidence_path" ] || continue
            printf -- '- `%s`\n' "$evidence_path"
        done >>"$evidence_dir/README.md"
    else
        printf '\nNo M27 sub-run evidence path was recorded.\n' >>"$evidence_dir/README.md"
    fi
    cat >>"$evidence_dir/README.md" <<'EOF'

The archive SHA256SUMS covers this README, `source-worktree.diff` (the tracked
changes from the source revision above), and all archived per-repetition files.
Each M27 sub-run directory has its own README and SHA256SUMS.

## Source worktree changes
EOF
    git diff --name-only | while IFS= read -r changed_path; do
        printf -- '- `%s`\n' "$changed_path"
    done >>"$evidence_dir/README.md"
    write_evidence_manifest "$evidence_dir"
    verify_evidence_manifest "$evidence_dir"
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
Nagi M22 file.search Activity Ledger PASS
Nagi M22 AI Activity Ledger undo result PASS
Nagi M22 archive restart and restored files PASS
Nagi M13 acceptance PASS
EOF
    validate_m19_log "$temporary_dir/m19.log" || fail 'valid M19 fixture rejected'
    validate_m22_log "$temporary_dir/m22.log" || fail 'valid M22 fixture rejected'
    m22_output="$(cat <<EOF
building M22 acceptance image
PASS M21/M22 guest fixture: VFS file.move and file.copy, NH16 Create/Move transactions, Activity Ledger, composite Undo, and restored state survived QEMU restarts (log $temporary_dir/m22-history-987654321-boot-3.log)
EOF
)"
    if extracted_m22_log=$(extract_m22_log "$m22_output"); then
        [ "$extracted_m22_log" = "$temporary_dir/m22-history-987654321-boot-3.log" ] \
            || fail 'M22 final serial log extraction returned the wrong path'
    else
        fail 'M22 final serial log path was not extracted from command output'
    fi
    m22_failure_output='FAIL m22: bootstrap boot timed out; diagnostics appended to /tmp/m22-history-bootstrap-987654321.log'
    [ "$(extract_m22_run_id "$m22_failure_output")" = 987654321 ] \
        || fail 'M22 run ID was not extracted from failure diagnostics'
    m22_boot_failure_output='FAIL m22: guest boot timed out; diagnostics appended to /tmp/m22-history-123456789-boot-1.log'
    [ "$(extract_m22_run_id "$m22_boot_failure_output")" = 123456789 ] \
        || fail 'M22 run ID was not extracted from a numbered boot failure'
    [ "$(m22_final_boot_log_path 123456789)" = \
        'out/logs/m22-history-123456789-boot-3.log' ] \
        || fail 'M22 final boot diagnostic path was not generated from the run ID'
    m27_output='PASS M27 A/B and Recovery: acceptance (evidence /tmp/m27-ab-rollback-self-test)'
    [ "$(extract_m27_evidence_path "$m27_output")" = '/tmp/m27-ab-rollback-self-test' ] \
        || fail 'M27 evidence path extraction returned the wrong path'
    m27_absolute_failure='FAIL m27: Recovery Undo guest fixture: QEMU timeout; diagnostics appended to /Users/test/NagiOS/out/evidence/m27-ab-rollback-123456789/recovery-committed-undo-fixture.log'
    [ "$(extract_m27_diagnostic_evidence_path "$m27_absolute_failure")" = \
        'out/evidence/m27-ab-rollback-123456789' ] \
        || fail 'M27 absolute diagnostic path did not resolve to its evidence directory'
    m27_relative_failure='FAIL m27: Recovery Undo guest fixture: QEMU timeout; diagnostics appended to out/evidence/m27-ab-rollback-987654321/recovery-committed-undo-fixture.log'
    [ "$(extract_m27_diagnostic_evidence_path "$m27_relative_failure")" = \
        'out/evidence/m27-ab-rollback-987654321' ] \
        || fail 'M27 relative diagnostic path did not resolve to its evidence directory'
    first_archive_root=$(archive_root_for_run self-test-run-1)
    second_archive_root=$(archive_root_for_run self-test-run-2)
    [ "$first_archive_root" != "$second_archive_root" ] || fail 'run archive namespaces collide'
    [ "$first_archive_root" != 'out/evidence/m28-repetition-1' ] \
        || fail 'run archive namespace reuses legacy repetition path'
    validate_m27_output 'PASS M27 A/B and Recovery: GPT slot rollback and promotion passed' \
        || fail 'valid M27 acceptance output rejected'
    if validate_m27_output 'FAIL M27: guest did not reach Recovery' >/dev/null 2>&1; then
        fail 'M27 failure output accepted as a completed gate'
    fi
    printf 'Nagi M19 guest search persistence PASS\n' >"$temporary_dir/incomplete.log"
    if validate_m19_log "$temporary_dir/incomplete.log" >/dev/null 2>&1; then
        fail 'incomplete M19 log accepted'
    fi
    mkdir "$temporary_dir/archive" "$temporary_dir/archive/repetition-1"
    printf 'M28 self-test evidence\n' >"$temporary_dir/archive/repetition-1/fixture.log"
    m27_self_test_paths="out/evidence/m27-ab-rollback-self-test
"
    write_m28_evidence_metadata \
        "$temporary_dir/archive" self-test-run self-test-revision 1 \
        "$m27_self_test_paths" PASS 1 '' ''
    verify_evidence_manifest "$temporary_dir/archive"
    [ -f "$temporary_dir/archive/README.md" ] || fail 'M28 archive README was not written'
    [ -f "$temporary_dir/archive/SHA256SUMS" ] || fail 'M28 archive manifest was not written'
    [ -f "$temporary_dir/archive/source-worktree.diff" ] \
        || fail 'M28 source worktree diff was not preserved'
    mkdir "$temporary_dir/m27"
    printf 'M27 self-test evidence\n' >"$temporary_dir/m27/fixture.log"
    write_m27_evidence_metadata "$temporary_dir/m27" 1 PASS 'fixture accepted'
    verify_evidence_manifest "$temporary_dir/m27"
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

run_id=$(date -u '+%Y%m%dT%H%M%SZ')-$$
archive_root=$(archive_root_for_run "$run_id")

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

latest_m22_log() {
    newest_m22_log=''
    for candidate_m22_log in out/logs/m22-history-boot-3.log out/logs/m22-history-*-boot-3.log; do
        [ -f "$candidate_m22_log" ] || continue
        if [ -z "$newest_m22_log" ] || [ "$candidate_m22_log" -nt "$newest_m22_log" ]; then
            newest_m22_log=$candidate_m22_log
        fi
    done
    if [ -n "$newest_m22_log" ]; then
        printf '%s\n' "$newest_m22_log"
    else
        printf '%s\n' "$m22_log"
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
    m22_log=$(latest_m22_log)
    if validate_m22_log "$m22_log"; then
        printf 'PASS existing M22 latest serial log: %s\n' "$m22_log"
    else
        printf 'UNVERIFIED existing M22 latest serial log: %s\n' "$m22_log"
    fi
    printf 'Planned real gate invocations (not executed):\n'
    iteration=1
    while [ "$iteration" -le "$repeat_count" ]; do
        printf '  %s: ./nagi m19, ./nagi m22, then ./nagi m27\n' "$iteration"
        iteration=$((iteration + 1))
    done
    printf 'Unique repetition archive namespace (if --run): %s/\n' "$archive_root"
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
        out/logs/m22-history-bootstrap.log \
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
    out/logs/m22-history-bootstrap.log \
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
[ ! -e "$archive_root" ] || fail "refusing to replace existing run evidence: $archive_root"

archive_iteration_outputs() {
    archived_iteration=$1
    archive_dir="$archive_root/repetition-$archived_iteration"
    mkdir -p "$archive_dir"
    for disk in "$m19_disk" "$m22_disk"; do
        disk_destination="$archive_dir/${disk##*/}"
        if [ -e "$disk_destination" ]; then
            if ! cmp -s "$disk" "$disk_destination"; then
                printf 'FAIL M28 harness: persistent disk changed while archiving repetition %s: %s\n' \
                    "$archived_iteration" "$disk" >&2
                return 1
            fi
        else
            if ! cp -p "$disk" "$disk_destination"; then
                printf 'FAIL M28 harness: cannot preserve persistent disk: %s\n' "$disk" >&2
                return 1
            fi
        fi
    done
    for output in \
        out/artifacts/nagi-0.1-m19-vfs-objectid.img \
        out/artifacts/nagi-0.1-m19-vfs-objectid-vars.fd \
        out/logs/m19-vfs-objectid-bootstrap.log \
        "$m19_initial_log" \
        "$m19_log" \
        out/artifacts/nagi-0.1-m22-history.img \
        out/artifacts/nagi-0.1-m22-history-vars.fd \
        out/logs/m22-history-bootstrap.log \
        out/logs/m22-history-boot-1.log \
        out/logs/m22-history-boot-2.log \
        out/artifacts/nagi-0.1-m22-history-"$m22_run_id".img \
        out/artifacts/nagi-0.1-m22-history-user-data-"$m22_run_id".img \
        out/artifacts/nagi-0.1-m22-history-vars-"$m22_run_id".fd \
        out/logs/m22-history-bootstrap-"$m22_run_id".log \
        out/logs/m22-history-"$m22_run_id"-boot-1.log \
        out/logs/m22-history-"$m22_run_id"-boot-2.log \
        "$(m22_final_boot_log_path "$m22_run_id")" \
        "$m22_log"; do
        if [ -e "$output" ]; then
            output_destination="$archive_dir/${output##*/}"
            if [ -e "$output_destination" ]; then
                output_destination="$output_destination.duplicate-$run_id"
            fi
            if ! mv "$output" "$output_destination"; then
                printf 'FAIL M28 harness: cannot preserve repetition output: %s\n' "$output" >&2
                return 1
            fi
        fi
    done
}

finalize_failed_run() {
    run_exit_status=$1
    trap - EXIT HUP INT TERM
    if [ "$run_exit_status" -eq 0 ]; then
        return 0
    fi
    if [ -n "${archive_root:-}" ] && [ -d "$archive_root" ]; then
        if ! archive_iteration_outputs "$iteration"; then
            run_exit_status=1
        fi
        if [ -z "${harness_failure_summary:-}" ]; then
            harness_failure_summary="M28 runner exited with status $run_exit_status"
        fi
        write_m28_evidence_metadata \
            "$archive_root" "$run_id" "$source_revision" "$repeat_count" \
            "$m27_evidence_paths" PARTIAL "$passed_repetitions" "$iteration" \
            "$harness_failure_summary"
    fi
    exit "$run_exit_status"
}

iteration=1
while [ "$iteration" -lt "$repeat_count" ]; do
    archive_dir="$archive_root/repetition-$iteration"
    [ ! -e "$archive_dir" ] || fail "refusing to replace existing repetition evidence: $archive_dir"
    iteration=$((iteration + 1))
done

if [ ! -x ./nagi ]; then
    fail 'repository ./nagi entry point is missing or not executable'
fi
mkdir "$archive_root" || fail "cannot reserve run evidence directory: $archive_root"
printf 'M28 repetition archive namespace: %s/\n' "$archive_root"

source_revision=$(git rev-parse HEAD)
m27_evidence_paths=''
iteration=1
passed_repetitions=0
m22_run_id=''
trap 'finalize_failed_run $?' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
while [ "$iteration" -le "$repeat_count" ]; do
    printf 'M28 repetition %s/%s: running ./nagi m19\n' "$iteration" "$repeat_count"
    ./nagi m19 || fail "./nagi m19 failed on repetition $iteration"
    validate_latest_m19_log || fail "M19 boot log failed on repetition $iteration"
    printf 'PASS M28 repetition %s M19 serial gate: %s\n' "$iteration" "$(latest_m19_log)"

    printf 'M28 repetition %s/%s: running ./nagi m22\n' "$iteration" "$repeat_count"
    if m22_output=$(./nagi m22 2>&1); then
        :
    else
        printf '%s\n' "$m22_output" >&2
        if m22_failure_run_id=$(extract_m22_run_id "$m22_output"); then
            m22_run_id=$m22_failure_run_id
        fi
        fail "./nagi m22 failed on repetition $iteration"
    fi
    printf '%s\n' "$m22_output"
    m22_log=$(extract_m22_log "$m22_output") \
        || fail "M22 command did not report its final serial log on repetition $iteration"
    case "$m22_log" in
        */out/logs/m22-history-*-boot-3.log) ;;
        *) fail "M22 command reported an unexpected final log path: $m22_log" ;;
    esac
    m22_run_id=${m22_log##*/m22-history-}
    m22_run_id=${m22_run_id%-boot-3.log}
    case "$m22_run_id" in
        ''|*[!0-9]*) fail "M22 command reported an invalid run ID in: $m22_log" ;;
    esac
    validate_m22_log "$m22_log" || fail "M22 final serial gate failed on repetition $iteration"
    printf 'PASS M28 repetition %s M22 serial gate: %s\n' "$iteration" "$m22_log"

    printf 'M28 repetition %s/%s: running ./nagi m27\n' "$iteration" "$repeat_count"
    if m27_output=$(./nagi m27 2>&1); then
        :
    else
        printf '%s\n' "$m27_output" >&2
        if m27_failure_path=$(extract_m27_diagnostic_evidence_path "$m27_output"); then
            case "$m27_failure_path" in
                "$repo_root"/out/evidence/m27-ab-rollback-*)
                    m27_failure_relative=${m27_failure_path#"$repo_root"/}
                    ;;
                out/evidence/m27-ab-rollback-*)
                    m27_failure_relative=$m27_failure_path
                    ;;
                *) m27_failure_relative='' ;;
            esac
            if [ -n "$m27_failure_relative" ] && [ -d "$m27_failure_path" ]; then
                m27_failure_summary=$(printf '%s\n' "$m27_output" \
                    | sed -n 's/^FAIL m27: //p' | tail -n 1)
                write_m27_evidence_metadata "$m27_failure_path" "$iteration" \
                    FAIL "$m27_failure_summary"
                m27_evidence_paths="${m27_evidence_paths}${m27_failure_relative}
"
            fi
        fi
        fail "./nagi m27 failed on repetition $iteration"
    fi
    if ! validate_m27_output "$m27_output"; then
        printf '%s\n' "$m27_output" >&2
        fail "M27 A/B and Recovery acceptance output failed on repetition $iteration"
    fi
    m27_evidence_path=$(extract_m27_evidence_path "$m27_output") \
        || fail "M27 acceptance did not report its evidence path on repetition $iteration"
    case "$m27_evidence_path" in
        "$repo_root"/out/evidence/m27-ab-rollback-*)
            m27_evidence_relative=${m27_evidence_path#"$repo_root"/}
            ;;
        out/evidence/m27-ab-rollback-*)
            m27_evidence_relative=$m27_evidence_path
            ;;
        *) fail "M27 reported an unexpected evidence path: $m27_evidence_path" ;;
    esac
    [ -d "$m27_evidence_path" ] || fail "M27 evidence directory is missing: $m27_evidence_path"
    write_m27_evidence_metadata "$m27_evidence_path" "$iteration" PASS \
        'Three malformed System B trials rolled back to persistent System A; healthy System B was promoted after guest readiness; Recovery preserved the journal and undid a committed M22 file.move group across restart.'
    m27_evidence_paths="${m27_evidence_paths}${m27_evidence_relative}
"
    printf '%s\n' "$m27_output"
    printf 'PASS M28 repetition %s M27 GPT A/B and Recovery gate\n' "$iteration"

    passed_repetitions=$iteration
    archive_iteration_outputs "$iteration"
    iteration=$((iteration + 1))
done

write_m28_evidence_metadata "$archive_root" "$run_id" "$source_revision" \
    "$repeat_count" "$m27_evidence_paths" PASS "$passed_repetitions" '' ''
print_unmeasured_workload
printf 'PASS M28 persistent Search/NH16 restart-gate repetitions: %s\n' "$repeat_count"
trap - EXIT HUP INT TERM
exit 0
