#!/usr/bin/env bash
set -euo pipefail

if (($# != 2)); then
    echo "Usage: build_system_verity_elf.sh <image.json> <new-output-dir>" >&2
    exit 2
fi

repo_root=$(git rev-parse --show-toplevel)
image_json=$(cd -- "$(dirname -- "$1")" && pwd)/$(basename -- "$1")
output_parent=$(cd -- "$(dirname -- "$2")" && pwd)
output_dir="$output_parent/$(basename -- "$2")"
case "$image_json" in "$repo_root"/*) ;; *) echo "image receipt must be inside this worktree" >&2; exit 2 ;; esac
case "$output_dir" in "$repo_root"/out/*) ;; *) echo "output must be below this worktree's out/" >&2; exit 2 ;; esac
[[ -f "$image_json" && ! -L "$image_json" ]] || { echo "image receipt must be a regular non-symlink file" >&2; exit 2; }
[[ ! -e "$output_dir" && ! -L "$output_dir" ]] || { echo "output directory already exists" >&2; exit 2; }

ssd_root=$(cd -- "$repo_root/../.." && pwd -P)
fixed_target="$ssd_root/cargo-target/l11-3-system-dm-verity-20261125"
[[ "${CARGO_TARGET_DIR:-}" == "$fixed_target" ]] || {
    echo "CARGO_TARGET_DIR must be the fixed L11-3 SSD target: $fixed_target" >&2
    exit 2
}
git -C "$repo_root" diff --quiet HEAD -- || { echo "tracked source changes must be committed before the reproducible build" >&2; exit 1; }
git -C "$repo_root" diff --cached --quiet || { echo "staged source changes must be committed before the reproducible build" >&2; exit 1; }

mkdir "$output_dir"
exec > >(tee "$output_dir/build.log") 2>&1
cd "$repo_root"

read -r root_hash data_blocks salt < <(python3 - "$image_json" <<'PY'
import json, re, sys
from pathlib import Path

image = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
verity = image.get("verity")
if not isinstance(verity, dict):
    raise SystemExit("image receipt has no verity section")
root = str(verity.get("root_digest", ""))
salt = str(verity.get("salt", ""))
blocks = verity.get("num_data_blocks")
if not re.fullmatch(r"[0-9a-f]{64}", root) or not re.fullmatch(r"[0-9a-f]{64}", salt):
    raise SystemExit("image receipt digest/salt is malformed")
if type(blocks) is not int or blocks <= 0:
    raise SystemExit("image receipt data-block count is malformed")
print(root, blocks, salt)
PY
)
export NAGI_DEV_SYSTEM_VERITY_ROOT_HASH="$root_hash"
export NAGI_DEV_SYSTEM_VERITY_DATA_BLOCKS="$data_blocks"
export NAGI_DEV_SYSTEM_VERITY_SALT="$salt"

toolchain=nightly-2025-08-01
target=x86_64-unknown-linux-musl
cargo_bin=$(rustup which --toolchain "$toolchain" cargo)
rustc_bin=$(rustup which --toolchain "$toolchain" rustc)
sysroot=$("$rustc_bin" --print sysroot)
host=$($rustc_bin -vV | sed -n 's/^host: //p')
linker="$sysroot/lib/rustlib/$host/bin/rust-lld"
[[ -x "$linker" ]] || { echo "pinned rust-lld not found: $linker" >&2; exit 2; }
export RUSTC="$rustc_bin"
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER="$linker"

cfgs=(linux_dev_system_mount linux_dev_system_verity)
rustc_args=()
for cfg in "${cfgs[@]}"; do
    rustc_args+=(--cfg "$cfg")
done
printf 'Building PID 1: toolchain=%s target=%s source_head=%s\n' "$toolchain" "$target" "$(git rev-parse HEAD)"
"$cargo_bin" rustc \
    --manifest-path user/nagi-init/Cargo.toml \
    --package nagi-init --bin nagi-init \
    --target "$target" --no-default-features --locked --offline \
    -- "${rustc_args[@]}" -C panic=abort

artifact="$CARGO_TARGET_DIR/$target/debug/nagi-init"
[[ -f "$artifact" && ! -L "$artifact" ]] || { echo "expected PID 1 artifact missing: $artifact" >&2; exit 1; }
cp "$artifact" "$output_dir/nagi-init"
chmod 0555 "$output_dir/nagi-init"

python3 - "$repo_root" "$image_json" "$output_dir" "$toolchain" "$target" "${cfgs[@]}" <<'PY'
import hashlib, json, subprocess, sys
from pathlib import Path

repo, image_path, output_path, toolchain, target, *cfgs = sys.argv[1:]
repo, image_path, output = Path(repo), Path(image_path), Path(output_path)
def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()
image = json.loads(image_path.read_text(encoding="utf-8"))
elf = output / "nagi-init"
data = elf.read_bytes()
if data[:6] != b"\x7fELF\x02\x01" or data[18:20] != b"\x3e\0":
    raise SystemExit("built PID 1 is not a little-endian x86_64 ELF")
for value in (image["verity"]["root_digest"], str(image["verity"]["num_data_blocks"]), image["verity"]["salt"]):
    if value.encode() not in data:
        raise SystemExit("built PID 1 is missing the fixed image verity parameter")
receipt = {
    "schema": 1,
    "source_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip(),
    "build_script": {"path": "tools/linux-boot/build_system_verity_elf.sh",
                     "sha256": sha(repo / "tools/linux-boot/build_system_verity_elf.sh")},
    "toolchain": toolchain,
    "target": target,
    "features": [],
    "cfgs": cfgs,
    "cargo_lock_sha256": sha(repo / "user/nagi-init/Cargo.lock"),
    "sources_lock_sha256": sha(repo / "third_party/sources.lock"),
    "system_source_sha256": image["source"]["sha256"],
    "system_root_digest": image["verity"]["root_digest"],
    "system_data_blocks": image["verity"]["num_data_blocks"],
    "system_salt": image["verity"]["salt"],
    "elf": {"path": elf.name, "size_bytes": len(data), "sha256": sha(elf)},
}
(output / "build-receipt.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
(output / "build-receipt.json").chmod(0o444)
print(f"PID 1 ELF SHA-256: {receipt['elf']['sha256']}")
PY
