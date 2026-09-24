#!/usr/bin/env bash
set -euo pipefail

# cc-rs invokes C and C++ build scripts with a host compiler unless the target
# compiler is supplied explicitly. Nagi's user ABI is freestanding ELF, so
# compiling those objects against host headers/runtime would give them the
# wrong pthread/layout and ABI contract.
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
target=${NAGI_TARGET:-x86_64-unknown-nagi-user}
compiler=${NAGI_TARGET_CLANG:-clang}

if [[ -n "${NAGI_RELIBC_HEADERS:-}" ]]; then
    relibc_headers=$NAGI_RELIBC_HEADERS
elif [[ -n "${NAGI_MESA_BUILD:-}" ]]; then
    mesa_build=$(cd "$NAGI_MESA_BUILD" && pwd)
    relibc_headers="${mesa_build%/mesa-build}/relibc-target/$target/include"
else
    relibc_headers="$repo_root/out/m17-mesa/relibc-target/$target/include"
fi

if [[ ! -f "$relibc_headers/pthread.h" ]]; then
    echo "Nagi target C compiler: generated relibc pthread.h not found: $relibc_headers" >&2
    exit 2
fi

if [[ -n "${NAGI_CXX_HEADERS:-}" ]]; then
    if [[ ! -f "$NAGI_CXX_HEADERS/cstddef" ]]; then
        echo "Nagi target C compiler: configured C++ headers missing cstddef: $NAGI_CXX_HEADERS" >&2
        exit 2
    fi
fi

resource_dir=$("$compiler" --target=x86_64-unknown-elf -print-resource-dir)
target_compile_definition=""
for argument in "$@"; do
    # Servo's bundled SQLite explicitly enables its dynamic extension loader,
    # but the guest is fully static and has no dynamic loader namespace. Keep
    # that compile-time feature omitted only for SQLite's amalgamation; other
    # target code retains its normal headers and APIs.
    if [[ "$argument" == */libsqlite3-sys-*/sqlite3/sqlite3.c ]]; then
        target_compile_definition=-DSQLITE_OMIT_LOAD_EXTENSION=1
        break
    fi
done

# Build a nonempty argument vector using conditional appends. This also works
# with macOS's system Bash 3.2, whose `set -u` handling rejects expansion of an
# empty array.
compiler_args=(
    --target=x86_64-unknown-elf \
    -D__NAGI__ \
    -ffreestanding \
    -fno-stack-protector \
    -fno-builtin \
    -mcmodel=large \
    -nostdinc \
)
if [[ -n "${NAGI_CXX_HEADERS:-}" ]]; then
    compiler_args+=(-isystem "$NAGI_CXX_HEADERS")
    # libc++ owns the C++ standard headers. Keep the Nagi/Mesa compatibility
    # headers after libc++ so include_next in libc++ reaches relibc instead of
    # selecting Mesa's intentionally minimal C++ shims.
    compiler_args+=(-idirafter "$repo_root/tools/mesa/nagi-headers" -idirafter "$relibc_headers")
else
    compiler_args+=(-I "$repo_root/tools/mesa/nagi-headers" -I "$relibc_headers")
fi
compiler_args+=(-isystem "$resource_dir/include")
if [[ -n "$target_compile_definition" ]]; then
    compiler_args+=("$target_compile_definition")
fi
exec "$compiler" "${compiler_args[@]}" "$@"
