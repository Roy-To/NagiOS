#!/usr/bin/env bash
set -euo pipefail

if (($# != 5)); then
    echo "Usage: build_system_verity_kernel.sh <linux-6.18.63-source> <pinned-linux-6.18.63.tar.xz> <base-.config> <base-config-sha256> <new-output-dir>" >&2
    exit 2
fi

repo_root=$(git rev-parse --show-toplevel)
source_dir=$(cd -- "$1" && pwd -P)
source_archive=$(cd -- "$(dirname -- "$2")" && pwd -P)/$(basename -- "$2")
base_config=$(cd -- "$(dirname -- "$3")" && pwd -P)/$(basename -- "$3")
expected_base_sha=$4
output_parent=$(cd -- "$(dirname -- "$5")" && pwd -P)
output_dir="$output_parent/$(basename -- "$5")"
script_path=$(cd -- "$(dirname -- "$0")" && pwd -P)/$(basename -- "$0")
source_verifier="$repo_root/tools/linux-boot/verify_linux_source.py"
build_inputs_helper="$repo_root/tools/linux-boot/system_verity_build_inputs.py"
config_validator="$repo_root/tools/linux-boot/system_verity.py"
zoffset_parser="$repo_root/tools/linux-boot/linux_nm_zoffset.py"
merge_fragment="$repo_root/tools/linux-boot/system-verity.config"
case "$output_dir" in "$repo_root"/out/*) ;; *) echo "output must be below this worktree's out/" >&2; exit 2 ;; esac
resume_existing=${NAGI_L11_RESUME_EXISTING:-0}
[[ "$resume_existing" == 0 || "$resume_existing" == 1 ]] || { echo "NAGI_L11_RESUME_EXISTING must be 0 or 1" >&2; exit 2; }
if [[ "$resume_existing" == 1 ]]; then
    [[ -d "$output_dir" && ! -L "$output_dir" && -d "$output_dir/build" && ! -L "$output_dir/build" ]] || {
        echo "resume output must be an existing directory with a regular build directory" >&2
        exit 2
    }
    [[ -f "$output_dir/build/.config" && ! -L "$output_dir/build/.config" && ! -e "$output_dir/build-receipt.json" && ! -L "$output_dir/build-receipt.json" ]] || {
        echo "resume output must have a regular Kconfig and no completed build receipt" >&2
        exit 2
    }
    [[ -f "$output_dir/resume-inputs.json" && ! -L "$output_dir/resume-inputs.json" ]] || {
        echo "resume output is missing its immutable input manifest; use a new output directory" >&2
        exit 2
    }
    if [[ -n "${NAGI_L11_HOST_TOOLS_ROOT:-}" ]]; then
        [[ -f "$output_dir/host-support-before.json" && ! -L "$output_dir/host-support-before.json" ]] || {
            echo "resume output is missing its initial host-support manifest" >&2
            exit 2
        }
    fi
else
    [[ ! -e "$output_dir" && ! -L "$output_dir" ]] || { echo "output directory already exists" >&2; exit 2; }
fi
[[ -f "$source_dir/Makefile" && ! -L "$source_dir/Makefile" ]] || { echo "kernel source Makefile is missing or a symlink" >&2; exit 2; }
[[ -f "$base_config" && ! -L "$base_config" ]] || { echo "base kernel .config must be a regular non-symlink file" >&2; exit 2; }
[[ "$expected_base_sha" =~ ^[0-9a-f]{64}$ ]] || { echo "base config SHA-256 is malformed" >&2; exit 2; }
actual_base_sha=$(shasum -a 256 "$base_config" | awk '{print $1}')
[[ "$actual_base_sha" == "$expected_base_sha" ]] || { echo "base kernel .config SHA-256 mismatch" >&2; exit 2; }
version=$(awk -F'= ' '/^(VERSION|PATCHLEVEL|SUBLEVEL) = / { print $2 }' "$source_dir/Makefile" | paste -sd. -)
[[ "$version" == "6.18.63" ]] || { echo "expected Linux 6.18.63 source, found $version" >&2; exit 2; }
source_identity_json=$(python3 -B "$source_verifier" verify "$source_dir" "$source_archive") || {
    echo "Linux source tree does not match the pinned source archive" >&2
    exit 2
}
printf 'Pinned Linux source archive and full extracted tree: PASS (%s)\n' "$source_archive"
make_path=$(python3 -c 'import pathlib,sys; print(pathlib.Path(sys.argv[1]).resolve(strict=True))' "$(command -v make)")
make_version=$("$make_path" --version | sed -n '1p')
python3 - "$make_version" <<'PY'
import re, sys

match = re.fullmatch(r"GNU Make (\d+)\.(\d+)(?:\.\d+)?", sys.argv[1])
if not match or tuple(map(int, match.groups())) < (4, 0):
    raise SystemExit(f"GNU Make >=4.0 is required, found: {sys.argv[1]}")
PY
resolve_executable() {
    local requested=$1 candidate
    if [[ "$requested" == */* ]]; then
        candidate=$requested
    else
        candidate=$(command -v -- "$requested") || return 1
    fi
    python3 - "$candidate" <<'PY'
import os, stat, sys
from pathlib import Path

try:
    path = Path(sys.argv[1]).resolve(strict=True)
    info = path.stat()
except OSError as error:
    raise SystemExit(f"tool path is unavailable: {error}")
if not stat.S_ISREG(info.st_mode) or not os.access(path, os.X_OK):
    raise SystemExit("tool path must resolve to a regular executable")
print(path)
PY
}
if [[ "$resume_existing" == 1 ]]; then
    rustc_path=$(python3 -B - "$repo_root" "$output_dir/resume-inputs.json" <<'PY'
import sys
from pathlib import Path
sys.path.insert(0, str(Path(sys.argv[1]) / "tools/linux-boot"))
from system_verity_build_inputs import resume_tool_path
print(resume_tool_path(Path(sys.argv[2]), "rustc"))
PY
) || exit 2
    requested_rustc=${NAGI_L11_RUSTC:-${RUSTC:-}}
    if [[ -n "$requested_rustc" ]]; then
        requested_rustc_path=$(resolve_executable "$requested_rustc") || {
            echo "requested RUSTC is unavailable" >&2
            exit 2
        }
        [[ "$requested_rustc_path" == "$rustc_path" ]] || {
            echo "requested RUSTC differs from the immutable resume input" >&2
            exit 2
        }
    fi
else
    requested_rustc=${NAGI_L11_RUSTC:-${RUSTC:-$(command -v rustc || true)}}
    [[ -n "$requested_rustc" ]] || { echo "rustc is required to resolve Linux Kconfig version symbols" >&2; exit 2; }
    rustc_path=$(resolve_executable "$requested_rustc") || {
        echo "RUSTC must resolve to a regular executable" >&2
        exit 2
    }
fi
if [[ "$rustc_path" =~ [[:space:]] ]]; then
    echo "pinned RUSTC path cannot contain whitespace in Linux Kbuild command variables" >&2
    exit 2
fi
rustc_version=$("$rustc_path" --version --verbose) || { echo "pinned RUSTC version query failed" >&2; exit 2; }
rustc_sha=$(shasum -a 256 "$rustc_path" | awk '{print $1}')
[[ "$rustc_sha" =~ ^[0-9a-f]{64}$ && -n "$rustc_version" ]] || { echo "pinned RUSTC identity is malformed" >&2; exit 2; }
printf 'Pinned RUSTC for Kconfig and kernel make: %s (SHA-256 %s)\n' "$rustc_path" "$rustc_sha"
host_support_root=${NAGI_L11_HOST_TOOLS_ROOT:-}
host_make_args=()
host_support_kind=none
host_compat_root=
host_elfutils_root=
host_clang_path=
host_lld_path=
host_pkg_config_path=
host_libintl_path=
host_macos_version=
host_cflags=
host_support_before_path="$output_dir/host-support-before.json"
host_support_after_path="$output_dir/host-support-after.json"
initial_host_support_path="$output_dir/host-support-before.json"
if [[ "$resume_existing" == 1 ]]; then
    host_support_before_path="$output_dir/host-support-resume-before.json"
    host_support_after_path="$output_dir/host-support-resume-after.json"
fi
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
    host_libintl_path=/opt/homebrew/lib/libintl.dylib
    host_pkg_config_path=$(command -v pkg-config)
    host_product_version=$(sw_vers -productVersion)
    host_macos_version=$(awk -F. 'NF >= 2 { print $1 "." $2 }' <<< "$host_product_version")
    [[ -x "$host_clang_path" && -x "$host_lld_path" && -x "$host_pkg_config_path" && -f "$host_libintl_path" && "$host_macos_version" =~ ^[0-9]+\.[0-9]+$ ]] || {
        echo "pinned Darwin Clang/LLD/pkg-config/libintl tools or macOS version are unavailable" >&2
        exit 2
    }
    export MACOSX_DEPLOYMENT_TARGET="$host_macos_version"
    export PKG_CONFIG_PATH="$host_elfutils_root/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
    host_cflags="-Wno-macro-redefined -Wno-deprecated-declarations -U_FORTIFY_SOURCE -D_FORTIFY_SOURCE=0 -I$host_compat_root/include -I$host_elfutils_root/include -I$output_dir/build/arch/x86/include/generated/uapi -I$source_dir/arch/x86/include/uapi -I$output_dir/build/include/generated/uapi -idirafter $source_dir/include/uapi"
    export HOSTCC="$host_compat_root/bin/clang-host"
    export HOSTCXX="$host_compat_root/bin/clang-host"
    export HOSTLD="$host_compat_root/bin/clang-host"
    export HOSTCFLAGS="$host_cflags"
    export HOSTLDFLAGS=-fuse-ld=lld
    export HOSTPKG_CONFIG="$host_pkg_config_path"
    host_make_args=("HOSTCC=$host_compat_root/bin/clang-host" "HOSTCXX=$host_compat_root/bin/clang-host"
        "HOSTLD=$host_compat_root/bin/clang-host" "HOSTCFLAGS=$host_cflags"
        "HOSTLDFLAGS=-fuse-ld=lld" "HOSTPKG_CONFIG=$host_pkg_config_path")
    export PATH="/opt/homebrew/opt/llvm@19/bin:/opt/homebrew/bin:$PATH"
    host_support_kind=mount-lane-read-only-darwin-compat+elfutils-0.197
fi

if [[ "$resume_existing" != 1 ]]; then
    mkdir "$output_dir"
fi
exec > >(tee -a "$output_dir/build.log") 2>&1
readlink_bin=$(command -v readlink)
if ! "$readlink_bin" -m -- "$source_dir/Makefile" >/dev/null 2>&1; then
    compat_dir="$output_dir/host-compat"
    mkdir -p "$compat_dir"
    if [[ -f "$compat_dir/readlink" && ! -L "$compat_dir/readlink" ]]; then
        chmod u+w "$compat_dir/readlink"
    fi
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
    compat_dir="$output_dir/host-compat"
    native_path="$PATH"
    case "$native_path" in
        "$compat_dir":*) native_path=${native_path#"$compat_dir":} ;;
    esac
    sed_bin=$(PATH="$native_path" command -v sed)
    cp_bin=$(PATH="$native_path" command -v cp)
    install_bin=$(PATH="$native_path" command -v install)
    export NAGI_L11_ZOFFSET_PARSER="$repo_root/tools/linux-boot/linux_nm_zoffset.py"
    mkdir -p "$compat_dir"
    for shim in sed cp clang-host install; do
        if [[ -f "$compat_dir/$shim" && ! -L "$compat_dir/$shim" ]]; then
            chmod u+w "$compat_dir/$shim"
        fi
    done
    cat > "$compat_dir/sed" <<'SH'
#!/usr/bin/env python3
import os
import re
import sys

args = sys.argv[1:]
if len(args) >= 3 and args[0] == "-i" and args[1] == r"s/\x00\+$/\x00/g":
    for path in args[2:]:
        with open(path, "rb") as stream:
            data = stream.read()
        data = re.sub(rb"\x00+(?=\n|$)", b"\x00", data, flags=re.MULTILINE)
        with open(path, "wb") as stream:
            stream.write(data)
    raise SystemExit(0)
if len(args) >= 3 and args[0] == "-n" and args[1] == "-e" and "VO_" in args[2] and r"\|" in args[2]:
    pattern = re.compile(rb"([0-9a-fA-F]+) [ABbCDGRSTtVW] (_text|__start_rodata|_sinittext|__inittext_end|__bss_start|_end)")
    required = {b"_text", b"__start_rodata", b"_sinittext", b"__inittext_end", b"__bss_start", b"_end"}
    seen = set()
    for line in sys.stdin.buffer:
        match = pattern.fullmatch(line.rstrip(b"\n"))
        if match:
            address, symbol = match.groups()
            sys.stdout.buffer.write(b"#define VO_" + symbol + b" _AC(0x" + address + b",UL)\n")
            seen.add(symbol)
    if seen != required:
        raise SystemExit(f"Darwin sed shim found incomplete x86 VOFFSET symbols: {sorted(required - seen)!r}")
    raise SystemExit(0)
if "ZO_" in " ".join(args) and "efi.._stub_entry" in " ".join(args):
    parser = os.environ.get("NAGI_L11_ZOFFSET_PARSER")
    if not parser:
        raise SystemExit("Darwin sed shim is missing NAGI_L11_ZOFFSET_PARSER")
    sys.stderr.write("Darwin sed shim: generating x86 zoffset symbols from llvm-nm input\n")
    os.execv(sys.executable, [sys.executable, parser])
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
    cat > "$compat_dir/install" <<'SH'
#!/usr/bin/env python3
import os
import sys

arguments = sys.argv[1:]
if len(arguments) >= 4 and not arguments[0].startswith("-") and arguments[1] == "-m":
    source, _, mode, *tail = arguments
    arguments = ["-m", mode, source, *tail]
os.execv(os.environ["NAGI_L11_BSD_INSTALL"], ["install", *arguments])
SH
    chmod 0555 "$compat_dir/install"
    export NAGI_L11_BSD_INSTALL="$install_bin"
    host_make_args=("HOSTCC=$compat_dir/clang-host" "HOSTCXX=$compat_dir/clang-host"
        "HOSTLD=$compat_dir/clang-host" "HOSTCFLAGS=$host_cflags"
        "HOSTLDFLAGS=-fuse-ld=lld" "HOSTPKG_CONFIG=$host_pkg_config_path"
        "INSTALL=$compat_dir/install")
    export INSTALL="$compat_dir/install"
    printf 'Installed private Darwin host-link flag shim: %s\n' "$compat_dir/clang-host"
    printf 'Installed private Darwin install argument shim: %s\n' "$compat_dir/install"
fi
build_dir="$output_dir/build"
if [[ "$resume_existing" != 1 ]]; then
    mkdir "$build_dir"
    cp "$base_config" "$build_dir/.config"
fi
source_makefile_sha_before=$(shasum -a 256 "$source_dir/Makefile" | awk '{print $1}')
source_verity_sha_before=$(shasum -a 256 "$source_dir/drivers/md/dm-verity-target.c" | awk '{print $1}')

if [[ "$host_support_kind" != none ]]; then
    write_host_support_manifest() {
        python3 - "$host_support_root" "$host_compat_root" "$host_elfutils_root" "$host_clang_path" "$host_lld_path" "$host_pkg_config_path" "$host_libintl_path" "$output_dir/host-compat/clang-host" "$output_dir/host-compat/install" "$output_dir/host-compat/sed" "$1" <<'PY'
import hashlib, json, sys
from pathlib import Path

root, compat, elfutils, clang, lld, pkg_config, libintl, link_shim, install_shim, sed_shim, output = map(Path, sys.argv[1:])

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
    ("libintl", libintl),
    ("kernel_link_flag_shim", link_shim),
    ("install_argument_shim", install_shim),
    ("sed_compat_shim", sed_shim),
):
    files[key] = {
        "path": str(path),
        "resolved_path": str(path.resolve(strict=True)),
        "size_bytes": path.stat().st_size,
        "sha256": sha(path),
    }
for name in ("llvm-ar", "llvm-nm", "llvm-objcopy", "llvm-objdump", "llvm-readelf", "llvm-strip"):
    path = clang.parent / name
    files[name.replace("-", "_")] = {
        "path": str(path),
        "resolved_path": str(path.resolve(strict=True)),
        "size_bytes": path.stat().st_size,
        "sha256": sha(path),
    }
payload = json.dumps({"kind": "mount-lane-read-only-darwin-compat+elfutils-0.197", "files": files}, indent=2, sort_keys=True) + "\n"
if output.exists():
    if output.is_symlink() or not output.is_file():
        raise SystemExit("host-support manifest output must be a regular file")
    output.chmod(0o600)
output.write_text(payload, encoding="utf-8")
output.chmod(0o444)
PY
    }
    write_host_support_manifest "$host_support_before_path"
    printf 'Darwin host support: %s; manifest=%s\n' "$host_support_kind" "$host_support_before_path"
    if [[ "$resume_existing" == 1 ]] && ! cmp -s "$initial_host_support_path" "$host_support_before_path"; then
        echo "resume Darwin host-support manifest differs from the original build input" >&2
        exit 2
    fi
fi

prepare_resume_inputs() {
    local mode=$1
    local host_manifest_arg=""
    if [[ "$host_support_kind" != none ]]; then
        host_manifest_arg=$initial_host_support_path
    fi
    python3 -B - "$repo_root" "$mode" "$output_dir/resume-inputs.json" \
        "$build_dir/.config" "$base_config" "$merge_fragment" "$script_path" \
        "$source_verifier" "$build_inputs_helper" "$config_validator" "$zoffset_parser" "$make_path" "$rustc_path" \
        "$source_dir" "$source_archive" "$source_identity_json" "$host_manifest_arg" "$rustc_version" <<'PY'
import json, sys
from pathlib import Path

repo, mode, manifest = sys.argv[1:4]
sys.path.insert(0, str(Path(repo) / "tools/linux-boot"))
from system_verity_build_inputs import validate_resume_manifest, write_resume_manifest

config, base, fragment, builder, verifier, helper, validator, zoffset, make, rustc = map(Path, sys.argv[4:14])
source_dir, archive = map(Path, sys.argv[14:16])
identity = json.loads(sys.argv[16])
host_support = Path(sys.argv[17]) if sys.argv[17] else None
rustc_version = sys.argv[18]
paths = {
    "base_config": base,
    "final_config": config,
    "merge_fragment": fragment,
    "builder_script": builder,
    "source_verifier": verifier,
    "build_inputs_helper": helper,
    "config_validator": validator,
    "zoffset_parser": zoffset,
    "make": make,
    "rustc": rustc,
}
if mode == "write":
    digest = write_resume_manifest(
        Path(manifest), paths, source_dir, archive, identity, host_support,
        {"rustc": rustc_version},
    )
else:
    digest = validate_resume_manifest(
        Path(manifest), paths, source_dir, archive, identity, host_support,
        {"rustc": rustc_version},
    )
print(digest)
PY
}

if [[ "$resume_existing" == 1 ]]; then
    resume_inputs_sha=$(prepare_resume_inputs validate) || exit 2
    printf 'Resume inputs match the original build receipt: PASS\n'
else
    printf 'Merging L11-3 DM config into Linux %s; base config SHA-256=%s\n' "$version" "$actual_base_sha"
    bash "$source_dir/scripts/kconfig/merge_config.sh" -m -O "$build_dir" \
        "$build_dir/.config" "$merge_fragment"
    make -C "$source_dir" "${host_make_args[@]}" "RUSTC=$rustc_path" O="$build_dir" ARCH=x86_64 LLVM=1 olddefconfig
fi
python3 -B - "$config_validator" "$build_dir/.config" <<'PY'
import importlib.util, sys
script, config = sys.argv[1:]
spec = importlib.util.spec_from_file_location("system_verity", script)
module = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = module
spec.loader.exec_module(module)
module.parse_config(open(config, "rb").read())
print("DM_INIT, DM_VERITY, SHA-256, virtio-blk and ext4 config: PASS")
PY
if [[ "$resume_existing" != 1 ]]; then
    resume_inputs_sha=$(prepare_resume_inputs write) || exit 2
    printf 'Immutable resume input manifest: %s (SHA-256 %s)\n' "$output_dir/resume-inputs.json" "$resume_inputs_sha"
fi
make -C "$source_dir" "${host_make_args[@]}" "RUSTC=$rustc_path" O="$build_dir" ARCH=x86_64 LLVM=1 -j4 bzImage

if [[ "$host_support_kind" != none ]]; then
    write_host_support_manifest "$host_support_after_path"
    if ! cmp -s "$initial_host_support_path" "$host_support_after_path"; then
        echo "Darwin kernel host-support inputs changed during the build" >&2
        diff -u "$initial_host_support_path" "$host_support_after_path" || true
        exit 1
    fi
    printf 'Darwin host-support inputs unchanged during build: PASS\n'
fi

source_makefile_sha_after=$(shasum -a 256 "$source_dir/Makefile" | awk '{print $1}')
source_verity_sha_after=$(shasum -a 256 "$source_dir/drivers/md/dm-verity-target.c" | awk '{print $1}')
[[ "$source_makefile_sha_before" == "$source_makefile_sha_after" ]] || { echo "kernel source Makefile changed during the out-of-tree build" >&2; exit 1; }
[[ "$source_verity_sha_before" == "$source_verity_sha_after" ]] || { echo "kernel dm-verity source changed during the out-of-tree build" >&2; exit 1; }
source_identity_after=$(python3 -B "$source_verifier" verify "$source_dir" "$source_archive") || {
    echo "pinned Linux source tree or archive changed during the build" >&2
    exit 1
}
[[ "$source_identity_json" == "$source_identity_after" ]] || { echo "pinned Linux source identity changed during the build" >&2; exit 1; }
resume_inputs_sha_after=$(prepare_resume_inputs validate) || exit 1
[[ "$resume_inputs_sha" == "$resume_inputs_sha_after" ]] || { echo "resume build input receipt changed during the build" >&2; exit 1; }
kernel="$build_dir/arch/x86/boot/bzImage"
[[ -f "$kernel" && ! -L "$kernel" ]] || { echo "kernel bzImage output missing" >&2; exit 1; }
config="$build_dir/.config"
kernel_sha=$(shasum -a 256 "$kernel" | awk '{print $1}')
config_sha=$(shasum -a 256 "$config" | awk '{print $1}')
printf 'Linux kernel SHA-256: %s\nFinal config SHA-256: %s\n' "$kernel_sha" "$config_sha"

python3 - "$repo_root" "$source_dir" "$source_archive" "$output_dir" "$base_config" "$expected_base_sha" "$version" "$source_makefile_sha_before" "$source_verity_sha_before" "$source_identity_json" "$resume_inputs_sha" "$script_path" "$source_verifier" "$build_inputs_helper" "$zoffset_parser" "$kernel_sha" "$config_sha" "$make_path" "$make_version" "$host_support_root" "$host_support_kind" "$initial_host_support_path" "$rustc_path" "$rustc_sha" "$rustc_version" "${#host_make_args[@]}" "${host_make_args[@]}" <<'PY'
import hashlib, json, os, sys
from pathlib import Path

repo, source, source_archive, output, base = map(Path, sys.argv[1:6])
base_sha, version, makefile_sha, verity_sha, source_identity_json, resume_inputs_sha, builder, verifier, input_helper, zoffset_parser = sys.argv[6:16]
kernel_sha, config_sha, make_path, make_version, host_root, host_kind, host_manifest_path = sys.argv[16:23]
rustc_path, rustc_sha, rustc_version = sys.argv[23:26]
host_make_args_count = int(sys.argv[26])
host_make_args = sys.argv[27 : 27 + host_make_args_count]
if len(host_make_args) != host_make_args_count:
    raise SystemExit("kernel host make argument receipt is truncated")

def sha(path):
    path = Path(path)
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()

kernel = output / "build/arch/x86/boot/bzImage"
config = output / "build/.config"
source_identity = json.loads(source_identity_json)
resume_manifest = output / "resume-inputs.json"
if (sha(base) != base_sha or sha(kernel) != kernel_sha or sha(config) != config_sha
        or sha(resume_manifest) != resume_inputs_sha):
    raise SystemExit("build inputs or outputs changed before receipt creation")
receipt = {
    "schema": 2,
    "purpose": "L11-3 development dm-verity kernel; not a signed product kernel",
    "source": {
        "path": str(source),
        "version": version,
        "archive_path": str(source_archive),
        "archive_sha256": source_identity["archive_sha256"],
        "tree_sha256": source_identity["tree_sha256"],
        "source_verifier": {"path": str(verifier), "sha256": sha(verifier)},
        "makefile_sha256": makefile_sha,
        "dm_verity_target_sha256": verity_sha,
    },
    "build_script": {"path": str(builder), "sha256": sha(builder)},
    "build_input_helper": {"path": str(input_helper), "sha256": sha(input_helper)},
    "zoffset_parser": {"path": str(zoffset_parser), "sha256": sha(zoffset_parser)},
    "resume_input_manifest": {
        "path": str(resume_manifest.relative_to(output)),
        "sha256": resume_inputs_sha,
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
    "rustc": {
        "path": rustc_path,
        "version": rustc_version,
        "sha256": rustc_sha,
    },
    "host_support": {
        "kind": host_kind,
        "root": host_root or None,
        "manifest_path": str(Path(host_manifest_path).relative_to(output)) if host_root else None,
        "manifest_sha256": sha(Path(host_manifest_path)) if host_root else None,
        "manifest": json.loads(Path(host_manifest_path).read_text(encoding="utf-8")) if host_root else None,
        "make_environment": {
            key: os.environ.get(key)
            for key in ("HOSTCC", "HOSTCXX", "HOSTLD", "HOSTCFLAGS", "HOSTLDFLAGS", "HOSTPKG_CONFIG", "PKG_CONFIG_PATH", "MACOSX_DEPLOYMENT_TARGET")
        },
        "make_overrides": host_make_args,
    },
    "config_command": [make_path, "-C", str(source), *host_make_args, f"RUSTC={rustc_path}", f"O={output / 'build'}", "ARCH=x86_64", "LLVM=1", "olddefconfig"],
    "build_command": [make_path, "-C", str(source), *host_make_args, f"RUSTC={rustc_path}", f"O={output / 'build'}", "ARCH=x86_64", "LLVM=1", "-j4", "bzImage"],
    "source_unchanged_checks": {
        "makefile_sha256_before_after": makefile_sha,
        "dm_verity_target_sha256_before_after": verity_sha,
        "full_archive_and_tree_verified_before_after": True,
    },
}
receipt_path = output / "build-receipt.json"
receipt_path.write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8")
receipt_path.chmod(0o444)
print(f"Kernel build receipt: {receipt_path}")
PY
