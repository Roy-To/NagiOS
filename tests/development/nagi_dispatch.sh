#!/usr/bin/env sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
scratch=$(mktemp -d "${TMPDIR:-/tmp}/nagi-dispatch.XXXXXX")
trap 'rm -rf "$scratch"' EXIT HUP INT TERM
mkdir -p "$scratch/bin"

cat > "$scratch/bin/cargo" <<'SH'
#!/usr/bin/env sh
printf '%s\n' "$@" > "$NAGI_LAUNCHER_TEST_LOG"
SH
chmod +x "$scratch/bin/cargo"

run_launcher() {
    log_path=$1
    shift
    PATH="$scratch/bin:/usr/bin:/bin" \
        NAGI_LAUNCHER_TEST_LOG="$log_path" \
        "$repo_root/nagi" "$@"
}

run_launcher "$scratch/dev.log" dev verify
grep -Fx -- "$repo_root/tools/nagi-bootstrap/Cargo.toml" "$scratch/dev.log" >/dev/null
grep -Fx -- nagi-bootstrap "$scratch/dev.log" >/dev/null
grep -Fx -- dev "$scratch/dev.log" >/dev/null
grep -Fx -- verify "$scratch/dev.log" >/dev/null

run_launcher "$scratch/build.log" build
grep -Fx -- "$repo_root/Cargo.toml" "$scratch/build.log" >/dev/null
grep -Fx -- nagi-cli "$scratch/build.log" >/dev/null

printf '%s\n' 'PASS nagi launcher: dev uses bootstrap workspace; build uses root workspace'
