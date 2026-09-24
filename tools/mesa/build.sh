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

for command_name in cargo make patch cbindgen meson ninja python3 llvm-ar llvm-ranlib llvm-nm llvm-objcopy llvm-strip; do
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

# Apply the narrow Nagi-owned build-host portability patch before relibc
# generates the target C headers. The patch is checked both before applying
# and after application so a changed vendored source is never rewritten
# silently.
relibc_header_patch="$repo_root/third_party/relibc-patches/0001-nagi-portable-header-find.patch"
relibc_makefile="$repo_root/third_party/relibc/Makefile"
if grep -Fq -- '-printf "%f\n"' "$relibc_makefile"; then
    if ! patch --dry-run --silent --batch --forward -p1 -d "$repo_root/third_party/relibc" < "$relibc_header_patch"; then
        echo "M17 Mesa build: relibc header portability patch does not match the pinned source" >&2
        exit 1
    fi
    patch --silent --batch --forward -p1 -d "$repo_root/third_party/relibc" < "$relibc_header_patch"
elif grep -Fq -- '-exec basename {} \;' "$relibc_makefile"; then
    :
else
    echo "M17 Mesa build: relibc header portability patch does not match the pinned source" >&2
    exit 1
fi

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
mesa_c_args="--target=x86_64-unknown-elf -D__NAGI__ -ffreestanding -fno-stack-protector -fno-builtin -fno-exceptions -fno-rtti -fno-asynchronous-unwind-tables -mcmodel=large -I$mesa_headers -I$relibc_headers"
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

# Mesa's core static target is intentionally not always a default Ninja target
# when only the Nagi EGL/Softpipe outputs are requested. Build the target named
# by Meson's graph explicitly so the real glthread implementation is present
# in the target-owned archive set before aggregation.
mesa_core_target=$(ninja -C "$mesa_build" -t targets all \
    | awk '{ target = $1; sub(/:$/, "", target); if (target ~ /(^|\/)libmesa\.a$/) { print target; exit } }')
if [[ -z "$mesa_core_target" ]]; then
    mesa_core_candidates=$(ninja -C "$mesa_build" -t targets all \
        | grep -E 'mesa|libmesa' \
        | head -n 40 \
        | tr '\n' ' ' \
        | cut -c1-3000)
    echo "::error title=M17 Mesa core target::Meson target graph has no libmesa.a output target; candidates=$mesa_core_candidates" >&2
    exit 1
fi
echo "M17 Mesa build: core target: $mesa_core_target"
mesa_core_log=$(mktemp)
if ! ninja -C "$mesa_build" "$mesa_core_target" 2>&1 | tee "$mesa_core_log"; then
    mesa_core_error=$(grep -E '(^| )(fatal )?error:' "$mesa_core_log" \
        | tail -n 20 \
        | tr '\n' ' ' \
        | cut -c1-3000)
    if [[ -z "$mesa_core_error" ]]; then
        mesa_core_error=$(tail -n 30 "$mesa_core_log" | tr '\n' ' ' | cut -c1-3000)
    fi
    echo "::error title=M17 Mesa core target build::target=$mesa_core_target; $mesa_core_error" >&2
    rm -f "$mesa_core_log"
    exit 1
fi
rm -f "$mesa_core_log"

# Mesa's default target graph does not necessarily materialize the auxiliary
# Gallium archive when only the Nagi EGL/Softpipe outputs are requested. The
# final user-init link reaches real state-tracker and postprocess entrypoints
# from that archive, so build the target explicitly as well.
mesa_gallium_target=$(ninja -C "$mesa_build" -t targets all \
    | awk '{ target = $1; sub(/:$/, "", target); if (target ~ /(^|\/)libgallium\.a$/) { print target; exit } }')
if [[ -z "$mesa_gallium_target" ]]; then
    echo "::error title=M17 Mesa Gallium target::Meson target graph has no libgallium.a output target" >&2
    exit 1
fi
echo "M17 Mesa build: Gallium target: $mesa_gallium_target"
mesa_gallium_log=$(mktemp)
if ! ninja -C "$mesa_build" "$mesa_gallium_target" 2>&1 | tee "$mesa_gallium_log"; then
    mesa_gallium_error=$(grep -E '(^| )(fatal )?error:' "$mesa_gallium_log" \
        | tail -n 20 \
        | tr '\n' ' ' \
        | cut -c1-3000)
    if [[ -z "$mesa_gallium_error" ]]; then
        mesa_gallium_error=$(tail -n 30 "$mesa_gallium_log" | tr '\n' ' ' | cut -c1-3000)
    fi
    echo "::error title=M17 Mesa Gallium target build::target=$mesa_gallium_target; $mesa_gallium_error" >&2
    rm -f "$mesa_gallium_log"
    exit 1
fi
rm -f "$mesa_gallium_log"

# The explicit user-init link also reaches Mesa's real GLSL linker helpers
# through shader-query/state-tracker code. Build the pinned libglsl target so
# its linker_util implementation is present in the aggregate archive.
mesa_glsl_target=$(ninja -C "$mesa_build" -t targets all \
    | awk '{ target = $1; sub(/:$/, "", target); if (target ~ /(^|\/)libglsl\.a$/) { print target; exit } }')
if [[ -z "$mesa_glsl_target" ]]; then
    echo "::error title=M17 Mesa GLSL target::Meson target graph has no libglsl.a output target" >&2
    exit 1
fi
echo "M17 Mesa build: GLSL target: $mesa_glsl_target"
mesa_glsl_log=$(mktemp)
if ! ninja -C "$mesa_build" "$mesa_glsl_target" 2>&1 | tee "$mesa_glsl_log"; then
    mesa_glsl_error=$(grep -E '(^| )(fatal )?error:' "$mesa_glsl_log" \
        | tail -n 20 \
        | tr '\n' ' ' \
        | cut -c1-3000)
    if [[ -z "$mesa_glsl_error" ]]; then
        mesa_glsl_error=$(tail -n 30 "$mesa_glsl_log" | tr '\n' ' ' | cut -c1-3000)
    fi
    echo "::error title=M17 Mesa GLSL target build::target=$mesa_glsl_target; $mesa_glsl_error" >&2
    rm -f "$mesa_glsl_log"
    exit 1
fi
rm -f "$mesa_glsl_log"

# The Nagi EGL target uses Mesa's static software loader path. These targets
# are intentionally build_by_default=false upstream, so the aggregate archive
# can otherwise contain the real Softpipe core while still omitting the
# loader/winsys objects that define sw_screen_create_vk and null_sw_create.
mesa_pipe_loader_target=$(ninja -C "$mesa_build" -t targets all \
    | awk '{ target = $1; sub(/:$/, "", target); if (target ~ /(^|\/)libpipe_loader_static\.a$/) { print target; exit } }')
if [[ -z "$mesa_pipe_loader_target" ]]; then
    echo "::error title=M17 Mesa pipe loader target::Meson target graph has no libpipe_loader_static.a output target" >&2
    exit 1
fi
echo "M17 Mesa build: static pipe loader target: $mesa_pipe_loader_target"
mesa_pipe_loader_log=$(mktemp)
if ! ninja -C "$mesa_build" "$mesa_pipe_loader_target" 2>&1 | tee "$mesa_pipe_loader_log"; then
    mesa_pipe_loader_error=$(grep -E '(^| )(fatal )?error:' "$mesa_pipe_loader_log" \
        | tail -n 20 \
        | tr '\n' ' ' \
        | cut -c1-3000)
    if [[ -z "$mesa_pipe_loader_error" ]]; then
        mesa_pipe_loader_error=$(tail -n 30 "$mesa_pipe_loader_log" | tr '\n' ' ' | cut -c1-3000)
    fi
    echo "::error title=M17 Mesa static pipe loader target build::target=$mesa_pipe_loader_target; $mesa_pipe_loader_error" >&2
    rm -f "$mesa_pipe_loader_log"
    exit 1
fi
rm -f "$mesa_pipe_loader_log"

mesa_null_winsys_target=$(ninja -C "$mesa_build" -t targets all \
    | awk '{ target = $1; sub(/:$/, "", target); if (target ~ /(^|\/)libws_null\.a$/) { print target; exit } }')
if [[ -z "$mesa_null_winsys_target" ]]; then
    echo "::error title=M17 Mesa null winsys target::Meson target graph has no libws_null.a output target" >&2
    exit 1
fi
echo "M17 Mesa build: null winsys target: $mesa_null_winsys_target"
mesa_null_winsys_log=$(mktemp)
if ! ninja -C "$mesa_build" "$mesa_null_winsys_target" 2>&1 | tee "$mesa_null_winsys_log"; then
    mesa_null_winsys_error=$(grep -E '(^| )(fatal )?error:' "$mesa_null_winsys_log" \
        | tail -n 20 \
        | tr '\n' ' ' \
        | cut -c1-3000)
    if [[ -z "$mesa_null_winsys_error" ]]; then
        mesa_null_winsys_error=$(tail -n 30 "$mesa_null_winsys_log" | tr '\n' ' ' | cut -c1-3000)
    fi
    echo "::error title=M17 Mesa null winsys target build::target=$mesa_null_winsys_target; $mesa_null_winsys_error" >&2
    rm -f "$mesa_null_winsys_log"
    exit 1
fi
rm -f "$mesa_null_winsys_log"

mesa_wrapper_winsys_target=$(ninja -C "$mesa_build" -t targets all \
    | awk '{ target = $1; sub(/:$/, "", target); if (target ~ /(^|\/)libwsw\.a$/) { print target; exit } }')
if [[ -z "$mesa_wrapper_winsys_target" ]]; then
    echo "::error title=M17 Mesa wrapper winsys target::Meson target graph has no libwsw.a output target" >&2
    exit 1
fi
echo "M17 Mesa build: wrapper winsys target: $mesa_wrapper_winsys_target"
mesa_wrapper_winsys_log=$(mktemp)
if ! ninja -C "$mesa_build" "$mesa_wrapper_winsys_target" 2>&1 | tee "$mesa_wrapper_winsys_log"; then
    mesa_wrapper_winsys_error=$(grep -E '(^| )(fatal )?error:' "$mesa_wrapper_winsys_log" \
        | tail -n 20 \
        | tr '\n' ' ' \
        | cut -c1-3000)
    if [[ -z "$mesa_wrapper_winsys_error" ]]; then
        mesa_wrapper_winsys_error=$(tail -n 30 "$mesa_wrapper_winsys_log" | tr '\n' ' ' | cut -c1-3000)
    fi
    echo "::error title=M17 Mesa wrapper winsys target build::target=$mesa_wrapper_winsys_target; $mesa_wrapper_winsys_error" >&2
    rm -f "$mesa_wrapper_winsys_log"
    exit 1
fi
rm -f "$mesa_wrapper_winsys_log"

mesa_nagi_roots_target=$(ninja -C "$mesa_build" -t targets all \
    | awk '{ target = $1; sub(/:$/, "", target); if (target ~ /(^|\/)libpipe_loader_nagi_roots\.a$/) { print target; exit } }')
if [[ -z "$mesa_nagi_roots_target" ]]; then
    echo "::error title=M17 Mesa Nagi helper target::Meson target graph has no libpipe_loader_nagi_roots.a output target" >&2
    exit 1
fi
echo "M17 Mesa build: Nagi helper target: $mesa_nagi_roots_target"
mesa_nagi_roots_log=$(mktemp)
if ! ninja -C "$mesa_build" "$mesa_nagi_roots_target" 2>&1 | tee "$mesa_nagi_roots_log"; then
    mesa_nagi_roots_error=$(grep -E '(^| )(fatal )?error:' "$mesa_nagi_roots_log" \
        | tail -n 20 \
        | tr '\n' ' ' \
        | cut -c1-3000)
    if [[ -z "$mesa_nagi_roots_error" ]]; then
        mesa_nagi_roots_error=$(tail -n 30 "$mesa_nagi_roots_log" | tr '\n' ' ' | cut -c1-3000)
    fi
    echo "::error title=M17 Mesa Nagi helper target build::target=$mesa_nagi_roots_target; $mesa_nagi_roots_error" >&2
    rm -f "$mesa_nagi_roots_log"
    exit 1
fi
rm -f "$mesa_nagi_roots_log"
ninja -C "$mesa_build"

# Keep the real glthread implementation reachable when the aggregated archive
# is scanned by rust-lld. Select the pinned Mesa archive by its defined symbol,
# then extract only the member that defines it; this remains valid across
# Meson's object-directory layout without forcing the whole Mesa archive out.
# Do not use llvm-nm's external-only filter here: Mesa compiles this target with
# hidden visibility, while the final Nagi link still needs the real object that
# owns the hidden implementation and its references.
mesa_glthread_archive=""
mesa_glthread_object=""
while IFS= read -r archive; do
    if llvm-nm --defined-only "$archive" 2>/dev/null \
        | grep -q '_mesa_glthread_finish'; then
        mesa_glthread_archive="$archive"
        break
    fi
done < <(find "$mesa_build" -type f -name '*.a' -print | sort)

if [[ -z "$mesa_glthread_archive" ]]; then
    # Some LLVM archive layouts do not expose hidden-visibility members to
    # archive-level nm enumeration even though the target object is present.
    # Inspect the objects emitted by the same Meson target before failing; this
    # still selects only the real pinned Mesa implementation.
    while IFS= read -r object; do
        if llvm-nm --defined-only "$object" 2>/dev/null \
            | grep -q '_mesa_glthread_finish'; then
            mesa_glthread_object="$object"
            break
        fi
    done < <(find "$mesa_build" -type f -name '*.o' -print | sort)
    if [[ -z "$mesa_glthread_object" ]]; then
        archive_candidates=$(find "$mesa_build" -type f -name '*.a' -exec basename {} \; | tr '\n' ' ' | cut -c1-1000)
        object_candidates=$(find "$mesa_build" -type f -name '*.o' -exec basename {} \; | tr '\n' ' ' | cut -c1-1000)
        echo "::error title=M17 Mesa glthread archive::no generated archive or object defines _mesa_glthread_finish; archives=${archive_candidates}; objects=${object_candidates}" >&2
        exit 1
    fi
    echo "M17 Mesa build: glthread source object: $mesa_glthread_object"
else
    echo "M17 Mesa build: glthread source archive: $mesa_glthread_archive"
fi

mesa_root_temp=$(mktemp -d)
trap 'rm -f "$archive_manifest"; rm -rf "$mesa_root_temp"' EXIT
if [[ -n "$mesa_glthread_archive" ]]; then
    member_index=0
    while IFS= read -r member; do
        candidate="$mesa_root_temp/member-$member_index.o"
        member_index=$((member_index + 1))
        if ! llvm-ar p "$mesa_glthread_archive" "$member" >"$candidate"; then
            rm -f "$candidate"
            continue
        fi
        if llvm-nm --defined-only "$candidate" 2>/dev/null \
            | grep -q '_mesa_glthread_finish'; then
            mesa_glthread_object="$candidate"
            break
        fi
    done < <(llvm-ar t "$mesa_glthread_archive")
fi

if [[ -z "$mesa_glthread_object" ]]; then
    echo "::error title=M17 Mesa glthread member::generated Mesa archive has no extractable _mesa_glthread_finish member" >&2
    exit 1
fi

mesa_roots_archive="$output_root/libnagi_mesa_roots.a"
rm -f "$mesa_roots_archive"
llvm-ar rcs "$mesa_roots_archive" "$mesa_glthread_object"
llvm-ranlib "$mesa_roots_archive"

# Cargo's final Nagi link must see the EGL, Gallium, Softpipe, and utility
# archives as one target-owned native library. Combining only libEGL.a would
# leave its static Gallium dependencies unresolved; combining the archives
# also avoids relying on host linker search paths or shared libraries.
mesa_archive="$output_root/libnagi_mesa.a"
archive_manifest=$(mktemp)
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
