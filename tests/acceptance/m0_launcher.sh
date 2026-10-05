#!/usr/bin/env sh
set -eu

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)

temp_root=$(mktemp -d)
trap 'rm -rf "$temp_root"' EXIT HUP INT TERM
mkdir -p "$temp_root/rustup-bin" "$temp_root/fallback-bin"
cat > "$temp_root/rustup-bin/rustup" <<'EOF'
#!/usr/bin/env sh
exit 99
EOF
cat > "$temp_root/rustup-bin/cargo" <<'EOF'
#!/usr/bin/env sh
command -v rustc > "$NAGI_LAUNCHER_SELECTION_LOG"
EOF
cat > "$temp_root/fallback-bin/cargo" <<'EOF'
#!/usr/bin/env sh
printf '%s\n' path-cargo > "$NAGI_LAUNCHER_SELECTION_LOG"
EOF
cat > "$temp_root/rustup-bin/rustc" <<'EOF'
#!/usr/bin/env sh
exit 99
EOF
chmod +x \
    "$temp_root/rustup-bin/rustup" \
    "$temp_root/rustup-bin/cargo" \
    "$temp_root/rustup-bin/rustc" \
    "$temp_root/fallback-bin/cargo"

selection_log=$temp_root/cargo-selection
verify_rustup_shim_selection() {
    if ! PATH="$temp_root/fallback-bin:$temp_root/rustup-bin:$PATH" \
        NAGI_LAUNCHER_SELECTION_LOG="$selection_log" \
        "$repository_root/nagi" --help >/dev/null 2>&1; then
        printf '%s\n' 'FAIL POSIX launcher toolchain-selection probe' >&2
        exit 1
    fi
    if [ "$(cat "$selection_log")" != "$temp_root/rustup-bin/rustc" ]; then
        printf '%s\n' 'FAIL POSIX launcher did not prefer the Rust toolchain shims beside rustup' >&2
        exit 1
    fi
}
verify_rustup_shim_selection
mv "$temp_root/rustup-bin/cargo" "$temp_root/rustup-bin/cargo.exe"
verify_rustup_shim_selection
rm -rf "$temp_root"
trap - EXIT HUP INT TERM

if invalid_output=$("$repository_root/nagi" doctor --unexpected 2>&1); then
    status=0
else
    status=$?
fi
if [ "$status" -ne 2 ]; then
    printf '%s\n' "$invalid_output" >&2
    printf 'FAIL M0 POSIX launcher usage exit: %s\n' "$status" >&2
    exit 1
fi

if help_output=$("$repository_root/nagi" --help 2>&1); then
    status=0
else
    status=$?
fi
if [ "$status" -ne 0 ]; then
    printf '%s\n' "$help_output" >&2
    printf 'FAIL M0 POSIX launcher help success exit: %s\n' "$status" >&2
    exit 1
fi
if ! printf '%s\n' "$help_output" | grep -Fq 'Nagi OS developer orchestrator'; then
    printf '%s\n' "$help_output" >&2
    printf '%s\n' 'FAIL M0 POSIX launcher help output was not recognized' >&2
    exit 1
fi

printf '%s\n' 'PASS M0 POSIX launcher exit propagation'
