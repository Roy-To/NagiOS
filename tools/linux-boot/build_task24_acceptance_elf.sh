#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'USAGE'
Usage: build_task24_acceptance_elf.sh [--online] [--release] <new-output-dir>

Build the standalone Linux x86_64-musl Task24 PID 1. The development
build requires NAGI_LINUX_BOOT_CATALOG_PUBLIC_KEY (32 raw public bytes)
and NAGI_LINUX_DEV_LOGIN_PACKAGE_SHA256 (lowercase SHA-256). Use the
same public key and signed login package digest as the acceptance image.

Install nightly-2025-08-01 with the x86_64-unknown-linux-musl target.
Materialize the pinned source inputs before building:
  cargo +nightly-2025-08-01 run --locked --manifest-path tools/nagi-bootstrap/Cargo.toml -p nagi-bootstrap -- fetch
Cargo resolution is locked and offline by default. Set CARGO_TARGET_DIR
to reuse your build target.

The output directory receives nagi-init and build-receipt.json.
--release enables all the same cfg flags and requires zero development
marker occurrences in the linked ELF.
USAGE
}

profile=debug
offline=true
while (($#)); do
    case "$1" in
        --release) profile=release ;;
        --online) offline=false ;;
        -h|--help) usage; exit 0 ;;
        --*) usage >&2; exit 2 ;;
        *) break ;;
    esac
    shift
done
if (($# != 1)); then
    usage >&2
    exit 2
fi

output_parent=$(cd -- "$(dirname -- "$1")" && pwd)
output_dir="$output_parent/$(basename -- "$1")"
script=tools/linux-boot/build_task24_acceptance_elf.sh
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.."
git diff --quiet HEAD -- || {
    echo 'Task24 build requires a clean tracked source tree' >&2
    exit 1
}
source_head=$(git rev-parse HEAD)

toolchain=nightly-2025-08-01
target=x86_64-unknown-linux-musl
manifest=user/nagi-init/Cargo.toml
features=linux-service-action-bridge,m10-desktop,desktop-login,system-service-readiness,production-session
cfgs=(
    linux_boot_identity_runtime linux_boot_owner_registration
    linux_task24_owner_co_location linux_task24_initial_login_runtime
    linux_task24_notes_consumer linux_dev_ws10 linux_dev_notes_performance
    linux_dev_system_mount
)
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
else
    : "${NAGI_LINUX_BOOT_CATALOG_PUBLIC_KEY:?set the acceptance catalog public key path}"
    : "${NAGI_LINUX_DEV_LOGIN_PACKAGE_SHA256:?set the signed development login package SHA-256}"
fi

cargo_bin=$(rustup which --toolchain "$toolchain" cargo)
export RUSTC
RUSTC=$(rustup which --toolchain "$toolchain" rustc)
sysroot=$("$RUSTC" --print sysroot)
host=$("$RUSTC" -vV | sed -n 's/^host: //p')
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="${CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER:-$sysroot/lib/rustlib/$host/bin/rust-lld}"

mkdir -- "$output_dir"
"$cargo_bin" rustc --manifest-path "$manifest" "${cargo_options[@]}" \
    --target "$target" --no-default-features --features "$features" \
    --bin nagi-init --message-format=json-render-diagnostics -- "${rustc_args[@]}" \
    > "$output_dir/cargo-artifacts.jsonl"

python3 - "$output_dir" "$script" "$source_head" "$toolchain" "$target" "$profile" \
    "$features" "${cfgs[@]}" <<'PY'
import hashlib
import json
import os
import re
import sys
from pathlib import Path

path, script, head, toolchain, target, profile, features, *cfgs = sys.argv[1:]
output = Path(path)
artifact_log = output / "cargo-artifacts.jsonl"
with artifact_log.open() as stream:
    reports = [json.loads(line) for line in stream]
executables = [
    Path(report["executable"])
    for report in reports
    if report.get("reason") == "compiler-artifact"
    and report.get("executable")
    and report["target"]["name"] == "nagi-init"
    and report["target"]["kind"] == ["bin"]
]
if len(executables) != 1:
    sys.exit("expected exactly one linked Task24 PID 1 ELF")
data = executables[0].read_bytes()
if data[:6] != b"\x7fELF\x02\x01" or data[18:20] != b"\x3e\x00":
    sys.exit("Task24 output is not a little-endian x86_64 ELF")

markers = (
    "nagi.dev_ws10",
    "NAGI_DEV_WS10",
    "NAGI_A_INPUT_TO_PRESENT",
    "NAGI_A_GUEST_MEM",
    "SYSTEM_DATA_MOUNT",
)
counts = {marker: data.count(marker.encode("ascii")) for marker in markers}
for marker, count in counts.items():
    print(f"{marker}: {count}")
if profile == "release" and any(counts.values()):
    sys.exit("release ELF retained a Task24 development marker")
if profile == "debug" and not all(counts.values()):
    sys.exit("acceptance ELF is missing a Task24 development marker")

public_key = os.environ.get("NAGI_LINUX_BOOT_CATALOG_PUBLIC_KEY")
key = Path(public_key).read_bytes() if public_key else b""
login_pin = os.environ.get("NAGI_LINUX_DEV_LOGIN_PACKAGE_SHA256") or None
if profile == "debug" and (len(key) != 32 or not re.fullmatch(r"[0-9a-f]{64}", login_pin or "")):
    sys.exit("acceptance public key or signed login package SHA-256 is invalid")

def sha256(data):
    return hashlib.sha256(data).hexdigest()

receipt = {
    "source_head": head,
    "script": {"path": script, "sha256": sha256(Path(script).read_bytes())},
    "toolchain": toolchain,
    "target": target,
    "profile": profile,
    "features": features.split(","),
    "cfgs": cfgs,
    "cargo_lock_sha256": sha256(Path("user/nagi-init/Cargo.lock").read_bytes()),
    "sources_lock_sha256": sha256(Path("third_party/sources.lock").read_bytes()),
    "catalog_public_key_sha256": sha256(key) if key else None,
    "login_package_sha256": login_pin,
    "elf": {"path": "nagi-init", "size_bytes": len(data), "sha256": sha256(data)},
    "development_marker_counts": counts,
}
elf = output / "nagi-init"
elf.write_bytes(data)
elf.chmod(0o755)
(output / "build-receipt.json").write_text(json.dumps(receipt, indent=2) + "\n")
artifact_log.unlink()
print(f"Task24 ELF SHA-256: {receipt['elf']['sha256']}")
PY
