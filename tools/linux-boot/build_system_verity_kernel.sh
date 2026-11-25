#!/usr/bin/env bash
set -euo pipefail

if (($# != 4)); then
    echo "Usage: build_system_verity_kernel.sh <linux-6.18.63-source> <base-.config> <base-config-sha256> <new-output-dir>" >&2
    exit 2
fi

repo_root=$(git rev-parse --show-toplevel)
source_dir=$(cd -- "$1" && pwd -P)
base_config=$(cd -- "$(dirname -- "$2")" && pwd)/$(basename -- "$2")
expected_base_sha=$3
output_parent=$(cd -- "$(dirname -- "$4")" && pwd)
output_dir="$output_parent/$(basename -- "$4")"
case "$output_dir" in "$repo_root"/out/*) ;; *) echo "output must be below this worktree's out/" >&2; exit 2 ;; esac
[[ ! -e "$output_dir" && ! -L "$output_dir" ]] || { echo "output directory already exists" >&2; exit 2; }
[[ -f "$source_dir/Makefile" && ! -L "$source_dir/Makefile" ]] || { echo "kernel source Makefile is missing or a symlink" >&2; exit 2; }
[[ -f "$base_config" && ! -L "$base_config" ]] || { echo "base kernel .config must be a regular non-symlink file" >&2; exit 2; }
[[ "$expected_base_sha" =~ ^[0-9a-f]{64}$ ]] || { echo "base config SHA-256 is malformed" >&2; exit 2; }
actual_base_sha=$(shasum -a 256 "$base_config" | awk '{print $1}')
[[ "$actual_base_sha" == "$expected_base_sha" ]] || { echo "base kernel .config SHA-256 mismatch" >&2; exit 2; }
version=$(awk -F'= ' '/^(VERSION|PATCHLEVEL|SUBLEVEL) = / { print $2 }' "$source_dir/Makefile" | paste -sd. -)
[[ "$version" == "6.18.63" ]] || { echo "expected Linux 6.18.63 source, found $version" >&2; exit 2; }

mkdir "$output_dir"
exec > >(tee "$output_dir/build.log") 2>&1
readlink_bin=$(command -v readlink)
if ! "$readlink_bin" -m -- "$source_dir/Makefile" >/dev/null 2>&1; then
    compat_dir="$output_dir/host-compat"
    mkdir "$compat_dir"
    cat > "$compat_dir/readlink" <<'SH'
#!/usr/bin/env python3
import os
import sys

args = sys.argv[1:]
if args and args[0] == "-m":
    args = args[1:]
    if args and args[0] == "--":
        args = args[1:]
    if not args:
        raise SystemExit(2)
    for path in args:
        print(os.path.realpath(path))
    raise SystemExit(0)
os.execv("/usr/bin/readlink", ["readlink", *sys.argv[1:]])
SH
    chmod 0555 "$compat_dir/readlink"
    export PATH="$compat_dir:$PATH"
    printf 'Installed private BSD readlink -m compatibility shim: %s\n' "$compat_dir/readlink"
fi
if [[ "$(uname -s)" == "Darwin" ]]; then
    sed_bin=$(command -v sed)
    compat_dir="$output_dir/host-compat"
    mkdir -p "$compat_dir"
    cat > "$compat_dir/sed" <<'SH'
#!/usr/bin/env python3
import os
import sys

args = sys.argv[1:]
if args and args[0] in ("-i", "--in-place"):
    args.insert(1, "")
os.execv(os.environ["NAGI_L11_BSD_SED"], ["sed", *args])
SH
    chmod 0555 "$compat_dir/sed"
    export NAGI_L11_BSD_SED="$sed_bin"
    export PATH="$compat_dir:$PATH"
    printf 'Installed private BSD sed -i compatibility shim: %s\n' "$compat_dir/sed"
fi
build_dir="$output_dir/build"
mkdir "$build_dir"
cp "$base_config" "$build_dir/.config"
source_makefile_sha_before=$(shasum -a 256 "$source_dir/Makefile" | awk '{print $1}')
source_verity_sha_before=$(shasum -a 256 "$source_dir/drivers/md/dm-verity-target.c" | awk '{print $1}')

printf 'Merging L11-3 DM config into Linux %s; base config SHA-256=%s\n' "$version" "$actual_base_sha"
bash "$source_dir/scripts/kconfig/merge_config.sh" -m -O "$build_dir" \
    "$build_dir/.config" "$repo_root/tools/linux-boot/system-verity.config"
make -C "$source_dir" O="$build_dir" ARCH=x86_64 LLVM=1 olddefconfig
python3 -B - "$repo_root/tools/linux-boot/system_verity.py" "$build_dir/.config" <<'PY'
import importlib.util, sys
script, config = sys.argv[1:]
spec = importlib.util.spec_from_file_location("system_verity", script)
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)
module.parse_config(open(config, "rb").read())
print("DM_INIT, DM_VERITY, SHA-256, virtio-blk and ext4 config: PASS")
PY
make -C "$source_dir" O="$build_dir" ARCH=x86_64 LLVM=1 -j4 bzImage

source_makefile_sha_after=$(shasum -a 256 "$source_dir/Makefile" | awk '{print $1}')
source_verity_sha_after=$(shasum -a 256 "$source_dir/drivers/md/dm-verity-target.c" | awk '{print $1}')
[[ "$source_makefile_sha_before" == "$source_makefile_sha_after" ]] || { echo "kernel source Makefile changed during the out-of-tree build" >&2; exit 1; }
[[ "$source_verity_sha_before" == "$source_verity_sha_after" ]] || { echo "kernel dm-verity source changed during the out-of-tree build" >&2; exit 1; }
kernel="$build_dir/arch/x86/boot/bzImage"
[[ -f "$kernel" && ! -L "$kernel" ]] || { echo "kernel bzImage output missing" >&2; exit 1; }
config="$build_dir/.config"
kernel_sha=$(shasum -a 256 "$kernel" | awk '{print $1}')
config_sha=$(shasum -a 256 "$config" | awk '{print $1}')
printf 'Linux kernel SHA-256: %s\nFinal config SHA-256: %s\n' "$kernel_sha" "$config_sha"

python3 - "$repo_root" "$source_dir" "$output_dir" "$base_config" "$expected_base_sha" "$version" "$source_makefile_sha_before" "$source_verity_sha_before" "$kernel_sha" "$config_sha" <<'PY'
import hashlib, json, sys
from pathlib import Path

repo, source, output, base = map(Path, sys.argv[1:5])
base_sha, version, makefile_sha, verity_sha, kernel_sha, config_sha = sys.argv[5:]

def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

kernel = output / "build/arch/x86/boot/bzImage"
config = output / "build/.config"
if sha(base) != base_sha or sha(kernel) != kernel_sha or sha(config) != config_sha:
    raise SystemExit("build inputs or outputs changed before receipt creation")
receipt = {
    "schema": 1,
    "purpose": "L11-3 development dm-verity kernel; not a signed product kernel",
    "source": {
        "path": str(source),
        "version": version,
        "makefile_sha256": makefile_sha,
        "dm_verity_target_sha256": verity_sha,
    },
    "base_config": {"path": str(base), "sha256": base_sha},
    "merge_fragment": {
        "path": "tools/linux-boot/system-verity.config",
        "sha256": sha(repo / "tools/linux-boot/system-verity.config"),
    },
    "final_config": {"path": "build/.config", "sha256": config_sha},
    "kernel": {
        "path": "build/arch/x86/boot/bzImage",
        "size_bytes": kernel.stat().st_size,
        "sha256": kernel_sha,
    },
    "build_command": ["make", "-C", str(source), f"O={output / 'build'}", "ARCH=x86_64", "LLVM=1", "-j4", "bzImage"],
    "source_unchanged_checks": {
        "makefile_sha256_before_after": makefile_sha,
        "dm_verity_target_sha256_before_after": verity_sha,
    },
}
receipt_path = output / "build-receipt.json"
receipt_path.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
receipt_path.chmod(0o444)
print(f"Kernel build receipt: {receipt_path}")
PY
