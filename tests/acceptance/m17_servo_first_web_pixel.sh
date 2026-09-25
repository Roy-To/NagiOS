#!/usr/bin/env sh
set -eu

repository_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$repository_root"

if [ -f out/mesa-venv/bin/activate ]; then
    # The pinned Python generators are part of the guest Mesa build toolchain.
    . out/mesa-venv/bin/activate
fi

mkdir -p out/logs
acceptance_stdout="$repository_root/out/logs/m17-acceptance.stdout.log"
acceptance_stderr="$repository_root/out/logs/m17-acceptance.stderr.log"
diagnostic_run="${GITHUB_RUN_ID:-local}-attempt-${GITHUB_RUN_ATTEMPT:-1}-$$"
diagnostic_report="$repository_root/out/logs/m17-diagnostics-$diagnostic_run.json"
m17_process_status=0
m17_acceptance_status=0
checks_log="$repository_root/out/logs/m17-acceptance-checks.log"
: > "$checks_log"

record_check_failure() {
    printf '%s\n' "$1" | tee -a "$checks_log" >&2
    m17_acceptance_status=1
}

if ./nagi m17 >"$acceptance_stdout" 2>"$acceptance_stderr"; then
    :
else
    m17_process_status=$?
    m17_acceptance_status=$m17_process_status
fi
cat "$acceptance_stdout"
cat "$acceptance_stderr" >&2
if [ "$m17_process_status" -ne 0 ]; then
    record_check_failure "FAIL M17 command exited with status $m17_process_status"
fi

serial_log="$repository_root/out/logs/m17-servo.log"
if [ ! -f "$serial_log" ]; then
    record_check_failure "FAIL M17 serial log was not created: $serial_log"
else
    if ! grep -F 'Nagi M17 first web pixel checksum=0x' "$serial_log" >/dev/null; then
        record_check_failure 'FAIL M17 serial log has no real pixel checksum'
    fi
    if ! grep -F 'Nagi M17 first web pixel PASS' "$serial_log" >/dev/null; then
        record_check_failure 'FAIL M17 serial log has no guest pixel pass marker'
    fi
fi
if ! grep -F 'PASS M17 first web pixel:' "$acceptance_stdout" >/dev/null; then
    record_check_failure 'FAIL M17 command output has no acceptance pass marker'
fi

if ./nagi dev diagnose \
    --stage m17-first-web-pixel-acceptance \
    --exit-code "$m17_process_status" \
    --log out/logs/m17-acceptance.stdout.log \
    --log out/logs/m17-acceptance.stderr.log \
    --log out/logs/m17-acceptance-checks.log \
    --log out/logs/m17-first-boot.log \
    --log out/logs/m17-servo.log \
    --log out/logs/m17-nagi-user-init.log \
    --log out/logs/m17-bootstrap.log \
    --artifact out/artifacts/nagi-0.1-m17-servo.img \
    --artifact out/artifacts/nagi-0.1-m17-user-data.img \
    --output "$diagnostic_report"; then
    :
else
    diagnostic_status=$?
    record_check_failure "FAIL M17 diagnostic report command exited with status $diagnostic_status"
    if [ "$m17_acceptance_status" -eq 0 ]; then
        m17_acceptance_status=$diagnostic_status
    fi
fi

if [ "$m17_acceptance_status" -ne 0 ]; then
    printf 'FAIL M17 first web pixel acceptance: process exit status %s; acceptance status %s\n' \
        "$m17_process_status" "$m17_acceptance_status" >&2
    exit "$m17_acceptance_status"
fi
printf '%s\n' 'PASS M17 first web pixel acceptance: real Servo guest frame reached Nagi Surface and QEMU'
