#!/usr/bin/env bash
set -euo pipefail

# Apple's Clang delegates an x86_64-unknown-elf link to the macOS gcc shim,
# which injects Mach-O options and host library search paths. Keep the target
# object files produced by Clang, but invoke ELF LLD directly for this link.
# Mesa's Darwin cross-file and the Nagi target compiler wrapper select this
# adapter for target links on macOS; Linux keeps its normal lld driver path.
target_linker=${NAGI_TARGET_LD:-ld.lld}
if [[ "$target_linker" != */* ]]; then
    target_linker=$(command -v "$target_linker") || {
        echo "Nagi ELF linker adapter: target linker not found: ${NAGI_TARGET_LD:-ld.lld}" >&2
        exit 2
    }
fi

linker_args=(-m elf_x86_64)
skip=0
for argument in "$@"; do
    if (( skip > 0 )); then
        skip=$((skip - 1))
        continue
    fi

    case "$argument" in
        # Host-only macOS driver arguments. Never search the host SDK or host
        # library directories when linking Nagi target probes. The current Mesa
        # cross configuration supplies no target library search directories.
        -arch|-lto_library|-syslibroot|-isysroot|-L)
            skip=1
            ;;
        -L*)
            ;;
        -platform_version)
            skip=3
            ;;
        -dynamic|-dylib|-no_weak_imports|-no_warn_duplicate_libraries|-dead_strip_dylibs)
            ;;
        -fatal_warnings)
            linker_args+=(--fatal-warnings)
            ;;
        -dead_strip)
            linker_args+=(--gc-sections)
            ;;
        -undefined)
            # Meson may add Apple's dynamic_lookup mode when probing the host
            # compiler. For a freestanding Nagi probe, preserve its intent as
            # unresolved ELF references rather than requesting Mach-O behavior.
            skip=1
            linker_args+=(--unresolved-symbols=ignore-all)
            ;;
        -mllvm)
            # This is a compiler option injected by the macOS driver.
            skip=1
            ;;
        *)
            linker_args+=("$argument")
            ;;
    esac
done

exec "$target_linker" "${linker_args[@]}"
