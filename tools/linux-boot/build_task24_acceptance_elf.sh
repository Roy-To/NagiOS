#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: build_task24_acceptance_elf.sh [--online] [--release | --desktop-launcher] [--files] <new-output-dir>

Build the standalone Linux x86_64-musl Task24 PID 1. Both profiles require
NAGI_LINUX_BOOT_CATALOG_PUBLIC_KEY (32 raw public bytes). Development
acceptance also requires NAGI_LINUX_DEV_LOGIN_PACKAGE_SHA256 (lowercase
SHA-256) for the signed development login package.

Install nightly-2025-08-01 with the x86_64-unknown-linux-musl target.
Materialize the pinned source inputs before building:
  cargo +nightly-2025-08-01 run --locked --manifest-path tools/nagi-bootstrap/Cargo.toml -p nagi-bootstrap -- fetch
Cargo resolution is locked and offline by default. Set CARGO_TARGET_DIR
to reuse your build target.

The output directory receives nagi-init and build-receipt.json.
--release enables all the same cfg flags and requires zero development
marker occurrences in the linked ELF.
Development builds include the L7 desktop shell, L9 Settings, Calendar and Files by default.
--desktop-launcher is accepted for existing development invocations.
--calendar is accepted for existing Calendar development invocations.
--files is accepted for existing development invocations.
USAGE
}

profile=debug
offline=true
desktop_launcher_requested=false
calendar_requested=false
files_requested=false
while (($#)); do
    case "$1" in
        --release) profile=release ;;
        --online) offline=false ;;
        --desktop-launcher) desktop_launcher_requested=true ;;
        --calendar) calendar_requested=true ;;
        --files) files_requested=true ;;
        -h|--help) usage; exit 0 ;;
        --*) usage >&2; exit 2 ;;
        *) break ;;
    esac
    shift
done
if (($# != 1)) || { [[ "$profile" == release ]] && { "$desktop_launcher_requested" || "$calendar_requested" || "$files_requested"; }; }; then
    usage >&2
    exit 2
fi

output_parent=$(cd -- "$(dirname -- "$1")" && pwd)
output_dir="$output_parent/$(basename -- "$1")"
script=tools/linux-boot/build_task24_acceptance_elf.sh
gate_helper=tools/linux-boot/task24_release_gate.py
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.."
git diff --quiet HEAD -- || {
    echo 'Task24 build requires a clean tracked source tree' >&2
    exit 1
}
source_head=$(git rev-parse HEAD)
: "${NAGI_LINUX_BOOT_CATALOG_PUBLIC_KEY:?set the catalog public key path}"
if [[ "$profile" == debug ]]; then
    : "${NAGI_LINUX_DEV_LOGIN_PACKAGE_SHA256:?set the development login package SHA-256}"
else
    unset NAGI_LINUX_DEV_LOGIN_PACKAGE_SHA256
fi
export PYTHONDONTWRITEBYTECODE=1
python3 "$gate_helper" check-rustflags --repo-root "$PWD"
target_root=${CARGO_TARGET_DIR:-"$PWD/target"}
isolated_target_dir=$(python3 "$gate_helper" prepare-build-paths \
    --output-dir "$output_dir" --target-root "$target_root")
cleanup_isolated_target() {
    local status=$?
    trap - EXIT
    if ! python3 "$gate_helper" cleanup-target \
        --target-root "$target_root" --isolated-target-dir "$isolated_target_dir"; then
        status=1
    fi
    exit "$status"
}
trap cleanup_isolated_target EXIT
export CARGO_TARGET_DIR="$isolated_target_dir"
python3 "$gate_helper" check-output --output-dir "$output_dir"

toolchain=nightly-2025-08-01
target=x86_64-unknown-linux-musl
manifest=user/nagi-init/Cargo.toml
features=linux-service-action-bridge,m10-desktop,desktop-login,system-service-readiness,production-session
if [[ "$profile" == debug ]]; then
    features+=,linux-dev-data-selection
fi
cfgs=(
    linux_boot_identity_runtime linux_boot_owner_registration
    linux_task24_owner_co_location linux_task24_initial_login_runtime
    linux_task24_notes_consumer linux_dev_ws10 linux_dev_notes_performance
    linux_dev_system_mount
)
if [[ "$profile" == debug ]]; then
    cfgs+=(linux_task24_desktop_shell linux_task24_settings linux_task24_calendar_app linux_task24_files_console)
    features+=,linux-files-desktop
fi
rustc_args=()
for cfg in "${cfgs[@]}"; do
    rustc_args+=(--cfg "$cfg")
done
cargo_options=(--locked)
if "$offline"; then
    cargo_options+=(--offline)
fi
if [[ "$profile" == release ]]; then
    cargo_options+=(--release)
fi
cargo_bin=$(rustup which --toolchain "$toolchain" cargo)
export RUSTC
RUSTC=$(rustup which --toolchain "$toolchain" rustc)
sysroot=$("$RUSTC" --print sysroot)
host=$("$RUSTC" -vV | sed -n 's/^host: //p')
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER:-$sysroot/lib/rustlib/$host/bin/rust-lld}"

"$cargo_bin" rustc --manifest-path "$manifest" "${cargo_options[@]}" \
    --target "$target" --no-default-features --features "$features" \
    --bin nagi-init --message-format=json-render-diagnostics -- "${rustc_args[@]}" \
    > "$output_dir/cargo-artifacts.jsonl"
python3 "$gate_helper" check-output --output-dir "$output_dir" --expected cargo-artifacts.jsonl

python3 - "$output_dir" "$script" "$source_head" "$toolchain" "$target" "$profile" \
    "$features" "$isolated_target_dir" "${cfgs[@]}" <<'PY'
import hashlib
import json
import os
import re
import sys
from pathlib import Path

path, script, head, toolchain, target, profile, features, isolated_target_dir, *cfgs = sys.argv[1:]
output = Path(path)
artifact_log = output / "cargo-artifacts.jsonl"
with artifact_log.open() as stream:
    reports = [json.loads(line) for line in stream]
sys.path.insert(0, str(Path(script).resolve().parent))
from task24_release_gate import (  # noqa: E402
    EXPECTED_MARKERS,
    EXPECTED_MARKER_COUNT,
    check_rustflags,
    validate_profile,
    validate_artifacts,
    validate_marker_counts,
    validate_output_files,
)

validate_profile(profile)
validate_output_files(output, ["cargo-artifacts.jsonl"])
binary_artifact, artifact_summary, original_executable_path = validate_artifacts(
    reports, profile, target, Path(isolated_target_dir)
)
artifact_profile = binary_artifact.get("profile") or {}
test_harness = artifact_profile.get("test")
opt_level = artifact_profile.get("opt_level")
debug_assertions = artifact_profile.get("debug_assertions")
data = Path(binary_artifact["executable"]).resolve(strict=True).read_bytes()
if data[:6] != b"\x7fELF\x02\x01" or data[18:20] != b"\x3e\x00":
    sys.exit("Task24 output is not a little-endian x86_64 ELF")

counts = {marker: data.count(marker.encode("ascii")) for marker in EXPECTED_MARKERS}
for marker, count in counts.items():
    print(f"{marker}: {count}")
development_marker_total = validate_marker_counts(counts, profile)
print(f"Development-only entry marker total: {development_marker_total}")
print(
    "Cargo artifact profile: "
    f"opt_level={opt_level} debug_assertions={debug_assertions} "
    f"test_harness={test_harness} test_profile_artifacts={artifact_summary['test_profile_artifact_count']} "
    f"checked_linked_artifacts={artifact_summary['linked_artifact_count']}"
)

public_key = os.environ.get("NAGI_LINUX_BOOT_CATALOG_PUBLIC_KEY")
key = Path(public_key).read_bytes() if public_key else b""
login_pin = os.environ.get("NAGI_LINUX_DEV_LOGIN_PACKAGE_SHA256") or None
if len(key) != 32:
    sys.exit("catalog public key must contain 32 bytes")
if profile == "debug" and not re.fullmatch(r"[0-9a-f]{64}", login_pin or ""):
    sys.exit("development login package SHA-256 is invalid")

def sha256(data):
    return hashlib.sha256(data).hexdigest()

rustflags_summary = check_rustflags(Path.cwd())
receipt = {
    "source_head": head,
    "script": {"path": script, "sha256": sha256(Path(script).read_bytes())},
    "toolchain": toolchain,
    "target": target,
    "profile": profile,
    "cargo_artifact_profile": {
        "opt_level": opt_level,
        "debug_assertions": debug_assertions,
        "test_harness": test_harness,
        "test_profile_artifact_count": artifact_summary["test_profile_artifact_count"],
        "fresh": binary_artifact["fresh"],
        "original_executable_path": original_executable_path,
        "original_executable_path_is_relative_to": "isolated CARGO_TARGET_DIR",
    },
    "cargo_artifact_validation": artifact_summary,
    "rustflags_validation": rustflags_summary,
    "build_isolation": {
        "target_dir_isolated_and_empty_at_start": True,
        "output_dir_fresh_at_start": True,
        "output_files": ["nagi-init", "build-receipt.json"],
    },
    "features": features.split(","),
    "cfgs": cfgs,
    "cargo_lock_sha256": sha256(Path("user/nagi-init/Cargo.lock").read_bytes()),
    "sources_lock_sha256": sha256(Path("third_party/sources.lock").read_bytes()),
    "catalog_public_key_sha256": sha256(key) if key else None,
    "login_package_sha256": login_pin,
    "elf": {"path": "nagi-init", "size_bytes": len(data), "sha256": sha256(data)},
    "development_marker_counts": counts,
    "development_marker_names": list(EXPECTED_MARKERS),
    "development_marker_expected_count": EXPECTED_MARKER_COUNT,
    "development_marker_total": development_marker_total,
}
elf = output / "nagi-init"
with elf.open("xb") as stream:
    stream.write(data)
elf.chmod(0o755)
with (output / "build-receipt.json").open("x", encoding="utf-8") as stream:
    stream.write(json.dumps(receipt, indent=2) + "\n")
artifact_log.unlink()
validate_output_files(output, ["nagi-init", "build-receipt.json"])
print(f"Task24 ELF SHA-256: {receipt['elf']['sha256']}")
PY
