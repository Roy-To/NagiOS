#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
target=${NAGI_TARGET:-x86_64-unknown-nagi-user}
rust_toolchain=${NAGI_RUST_TOOLCHAIN:-nightly-2025-08-01}
target_clang=${NAGI_TARGET_CLANG:-clang}
output_root=${NAGI_MESA_OUT:-"$repo_root/out/m17-mesa"}
relibc_target_dir="$output_root/relibc-target"
relibc_build="$relibc_target_dir/$target"
relibc_headers="$relibc_build/include"
mesa_build="$output_root/mesa-build"

for command_name in cargo make cbindgen meson ninja python3 llvm-ar llvm-ranlib llvm-nm llvm-objcopy llvm-strip; do
    command -v "$command_name" >/dev/null 2>&1 || {
        echo "M17 Mesa build: required command not found: $command_name" >&2
        exit 2
    }
done

case "$(cbindgen --version)" in
    'cbindgen 0.28.0'*) ;;
    *)
        echo "M17 Mesa build: cbindgen 0.28.0 is required" >&2
        exit 2
        ;;
esac

if [[ ! -d "$repo_root/third_party/mesa" || ! -f "$repo_root/third_party/mesa/.nagi-mesa-checkout" ]]; then
    echo "M17 Mesa build: run the pinned Nagi fetch/bootstrap before this script" >&2
    exit 2
fi

mkdir -p "$output_root"

# relibc is the Nagi C ABI boundary. The generated headers come from the same
# target Rust backend used by Nagi user space; no host /usr/include is used as
# a replacement.
RUST_TARGET_PATH="$repo_root/targets" \
RUSTUP_TOOLCHAIN="$rust_toolchain" \
CARGO_TARGET_DIR="$relibc_target_dir" \
make -C "$repo_root/third_party/relibc" \
    TARGET="$target" \
    BUILD="$relibc_build" \
    TARGET_HEADERS="$relibc_headers" \
    headers

if [[ ! -f "$relibc_headers/stdlib.h" || ! -f "$relibc_headers/time.h" ]]; then
    echo "M17 Mesa build: relibc did not generate the required C ABI headers" >&2
    exit 1
fi

# Keep the C ABI check on the generated target headers.  In particular, do not
# let a host pthread.h satisfy a dependency that will be linked into Nagi.
resource_dir=$("$target_clang" --target=x86_64-unknown-elf -print-resource-dir)
"$target_clang" \
    --target=x86_64-unknown-elf \
    -D__NAGI__ \
    -ffreestanding \
    -fno-stack-protector \
    -fno-builtin \
    -mcmodel=large \
    -nostdinc \
    -isystem "$resource_dir/include" \
    -I "$repo_root/tools/mesa/nagi-headers" \
    -I "$relibc_headers" \
    -fsyntax-only \
    "$repo_root/tools/mesa/nagi-pthread-header-check.c"

"$target_clang" \
    --target=x86_64-unknown-elf \
    -D__NAGI__ \
    -ffreestanding \
    -fno-stack-protector \
    -fno-builtin \
    -mcmodel=large \
    -nostdinc \
    -isystem "$resource_dir/include" \
    -I "$repo_root/tools/mesa/nagi-headers" \
    -I "$relibc_headers" \
    -fsyntax-only \
    "$repo_root/tools/mesa/nagi-c11-header-check.c"

mesa_headers="$repo_root/tools/mesa/nagi-headers"
mesa_c_args="--target=x86_64-unknown-elf -D__NAGI__ -ffreestanding -fno-stack-protector -fno-builtin -mcmodel=large -I$mesa_headers -I$relibc_headers"
meson setup --wipe "$mesa_build" "$repo_root/third_party/mesa" \
    --cross-file "$repo_root/tools/mesa/nagi-x86_64-user.meson.cross" \
    -Dgallium-drivers=softpipe \
    -Dvulkan-drivers= \
    -Dplatforms=nagi \
    -Degl-native-platform=surfaceless \
    -Dglx=disabled \
    -Dllvm=disabled \
    -Dshared-glapi=enabled \
    -Dosmesa=false \
    -Dopengl=true \
    -Dzlib=disabled \
    -Dzstd=disabled \
    -Dexpat=disabled \
    -Dxmlconfig=disabled \
    -Dshader-cache=disabled \
    -Dbuild-tests=false \
    -Denable-glcpp-tests=false \
    -Dbuild-aco-tests=false \
    "-Dc_args=$mesa_c_args" \
    "-Dcpp_args=$mesa_c_args" \
    "-Dprefix=$output_root/staging"

ninja -C "$mesa_build"

# Cargo's final Nagi link must see the EGL, Gallium, Softpipe, and utility
# archives as one target-owned native library. Combining only libEGL.a would
# leave its static Gallium dependencies unresolved; combining the archives
# also avoids relying on host linker search paths or shared libraries.
mesa_archive="$output_root/libnagi_mesa.a"
archive_manifest=$(mktemp)
trap 'rm -f "$archive_manifest"' EXIT
printf 'create %s\n' "$mesa_archive" >"$archive_manifest"
while IFS= read -r archive; do
    printf 'addlib %s\n' "$archive" >>"$archive_manifest"
done < <(find "$mesa_build" -type f -name '*.a' -print | sort)
printf 'save\nend\n' >>"$archive_manifest"
llvm-ar -M <"$archive_manifest"

if [[ ! -s "$mesa_archive" ]]; then
    echo "M17 Mesa build: native archive aggregation produced no archive" >&2
    exit 1
fi

echo "PASS M17 Mesa static Softpipe build: $mesa_build (archive: $mesa_archive)"
