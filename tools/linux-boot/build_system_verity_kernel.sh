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
make_path=$(command -v make)
make_version=$("$make_path" --version | sed -n '1p')
python3 - "$make_version" <<'PY'
import re, sys

match = re.fullmatch(r"GNU Make (\d+)\.(\d+)(?:\.\d+)?", sys.argv[1])
if not match or tuple(map(int, match.groups())) < (4, 0):
    raise SystemExit(f"GNU Make >=4.0 is required, found: {sys.argv[1]}")
PY
host_support_root=${NAGI_L11_HOST_TOOLS_ROOT:-}
host_support_kind=none
host_compat_root=
host_elfutils_root=
host_clang_path=
host_lld_path=
host_pkg_config_path=
host_cflags=
if [[ "$(uname -s)" == "Darwin" ]]; then
    [[ -n "$host_support_root" ]] || {
        echo "NAGI_L11_HOST_TOOLS_ROOT must point to the pinned Darwin kernel host tools" >&2
        exit 2
    }
    host_compat_root="$host_support_root/linux-kernel-darwin-compat"
    host_elfutils_root="$host_support_root/elfutils-0.197/install"
    [[ -x "$host_compat_root/bin/clang-host" && -f "$host_compat_root/include/darwin-system-preinclude.h" ]] || {
        echo "Darwin kernel host compatibility inputs are missing" >&2
        exit 2
    }
    [[ -f "$host_elfutils_root/include/libelf.h" && -f "$host_elfutils_root/lib/libelf.a" && -f "$host_elfutils_root/lib/pkgconfig/libelf.pc" ]] || {
        echo "pinned elfutils 0.197 libelf inputs are missing" >&2
        exit 2
    }
    host_clang_path=/opt/homebrew/opt/llvm@19/bin/clang
    host_lld_path=/opt/homebrew/bin/ld.lld
    host_pkg_config_path=$(command -v pkg-config)
    [[ -x "$host_clang_path" && -x "$host_lld_path" && -x "$host_pkg_config_path" ]] || {
        echo "pinned Darwin Clang/LLD/pkg-config tools are missing" >&2
        exit 2
    }
    export PKG_CONFIG_PATH="$host_elfutils_root/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    host_cflags="-Wno-macro-redefined -Wno-deprecated-declarations -U_FORTIFY_SOURCE -D_FORTIFY_SOURCE=0 -I$host_compat_root/include -I$host_elfutils_root/include"
    export HOSTCC="$host_compat_root/bin/clang-host"
    export HOSTCXX="$host_compat_root/bin/clang-host"
    export HOSTLD="$host_compat_root/bin/clang-host"
    export HOSTCFLAGS="$host_cflags"
    export HOSTLDFLAGS=-fuse-ld=lld
    export HOSTPKG_CONFIG="$host_pkg_config_path"
    export PATH="/opt/homebrew/bin:$PATH"
    host_support_kind=mount-lane-read-only-darwin-compat+elfutils-0.197
fi

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
    cp_bin=$(command -v cp)
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
    cat > "$compat_dir/cp" <<'SH'
#!/usr/bin/env python3
import os
import sys

args = sys.argv[1:]
if "-T" in args:
    index = args.index("-T")
    args.pop(index)
    if index < len(args) and args[index] == "--":
        args.pop(index)
os.execv(os.environ["NAGI_L11_BSD_CP"], ["cp", *args])
SH
    chmod 0555 "$compat_dir/cp"
    export NAGI_L11_BSD_CP="$cp_bin"
    printf 'Installed private BSD cp -T compatibility shim: %s\n' "$compat_dir/cp"
    cat > "$compat_dir/clang-host" <<'SH'
#!/usr/bin/env python3
import os
import sys

compiler = os.environ["NAGI_L11_HOST_CLANG_BASE"]
arguments = [
    argument.replace("-Wl,--fatal-warnings", "-Wl,-fatal_warnings")
    for argument in sys.argv[1:]
]
os.execv(compiler, [compiler, *arguments])
SH
    chmod 0555 "$compat_dir/clang-host"
    export NAGI_L11_HOST_CLANG_BASE="$host_compat_root/bin/clang-host"
    export HOSTCC="$compat_dir/clang-host"
    export HOSTCXX="$compat_dir/clang-host"
    export HOSTLD="$compat_dir/clang-host"
    printf 'Installed private Darwin host-link flag shim: %s\n' "$compat_dir/clang-host"
fi
build_dir="$output_dir/build"
mkdir "$build_dir"
cp "$base_config" "$build_dir/.config"
source_makefile_sha_before=$(shasum -a 256 "$source_dir/Makefile" | awk '{print $1}')
source_verity_sha_before=$(shasum -a 256 "$source_dir/drivers/md/dm-verity-target.c" | awk '{print $1}')

if [[ "$host_support_kind" != none ]]; then
    write_host_support_manifest() {
        python3 - "$host_support_root" "$host_compat_root" "$host_elfutils_root" "$host_clang_path" "$host_lld_path" "$host_pkg_config_path" "$output_dir/host-compat/clang-host" "$1" <<'PY'
import hashlib, json, sys
from pathlib import Path

root, compat, elfutils, clang, lld, pkg_config, link_shim, output = map(Path, sys.argv[1:])

def sha(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

files = {}
for key, directory in (("compat/include", compat / "include"), ("elfutils/include", elfutils / "include")):
    for path in sorted(directory.rglob("*")):
        if path.is_file():
            files[f"{key}/{path.relative_to(directory).as_posix()}"] = {
                "path": str(path),
                "resolved_path": str(path.resolve(strict=True)),
                "size_bytes": path.stat().st_size,
                "sha256": sha(path),
            }
for key, path in (
    ("clang_host", compat / "bin/clang-host"),
    ("darwin_preinclude", compat / "include/darwin-system-preinclude.h"),
    ("libelf_archive", elfutils / "lib/libelf.a"),
    ("libelf_pkgconfig", elfutils / "lib/pkgconfig/libelf.pc"),
    ("llvm_clang", clang),
    ("lld", lld),
    ("pkg_config", pkg_config),
    ("kernel_link_flag_shim", link_shim),
):
    files[key] = {
        "path": str(path),
        "resolved_path": str(path.resolve(strict=True)),
        "size_bytes": path.stat().st_size,
        "sha256": sha(path),
    }
output.write_text(json.dumps({"kind": "mount-lane-read-only-darwin-compat+elfutils-0.197", "files": files}, indent=2, sort_keys=True) + "\n", encoding="utf-8")
output.chmod(0o444)
PY
    }
    write_host_support_manifest "$output_dir/host-support-before.json"
    printf 'Darwin host support: %s; manifest=%s\n' "$host_support_kind" "$output_dir/host-support-before.json"
fi

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

if [[ "$host_support_kind" != none ]]; then
    write_host_support_manifest "$output_dir/host-support-after.json"
    if ! cmp -s "$output_dir/host-support-before.json" "$output_dir/host-support-after.json"; then
        echo "Darwin kernel host-support inputs changed during the build" >&2
        diff -u "$output_dir/host-support-before.json" "$output_dir/host-support-after.json" || true
        exit 1
    fi
    printf 'Darwin host-support inputs unchanged during build: PASS\n'
fi

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

python3 - "$repo_root" "$source_dir" "$output_dir" "$base_config" "$expected_base_sha" "$version" "$source_makefile_sha_before" "$source_verity_sha_before" "$kernel_sha" "$config_sha" "$make_path" "$make_version" "$host_support_root" "$host_support_kind" <<'PY'
import hashlib, json, os, sys
from pathlib import Path

repo, source, output, base = map(Path, sys.argv[1:5])
base_sha, version, makefile_sha, verity_sha, kernel_sha, config_sha, make_path, make_version, host_root, host_kind = sys.argv[5:]

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
    "build_tool": {
        "path": make_path,
        "version": make_version,
        "sha256": sha(Path(make_path)),
    },
    "host_support": {
        "kind": host_kind,
        "root": host_root or None,
        "manifest_path": "host-support-before.json" if host_root else None,
        "manifest_sha256": sha(output / "host-support-before.json") if host_root else None,
        "manifest": json.loads((output / "host-support-before.json").read_text(encoding="utf-8")) if host_root else None,
        "make_environment": {
            key: os.environ.get(key)
            for key in ("HOSTCC", "HOSTCXX", "HOSTLD", "HOSTCFLAGS", "HOSTLDFLAGS", "HOSTPKG_CONFIG", "PKG_CONFIG_PATH")
        },
    },
    "build_command": [make_path, "-C", str(source), f"O={output / 'build'}", "ARCH=x86_64", "LLVM=1", "-j4", "bzImage"],
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
