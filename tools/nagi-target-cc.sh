#!/usr/bin/env bash
set -euo pipefail

# cc-rs invokes target C build scripts with a host compiler unless the target
# compiler is supplied explicitly.  Nagi's user ABI is freestanding ELF, so
# compiling those objects against the host libc would silently give them the
# wrong pthread/layout and runtime contract.
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

resource_dir=$("$compiler" --target=x86_64-unknown-elf -print-resource-dir)
exec "$compiler" \
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
    "$@"
