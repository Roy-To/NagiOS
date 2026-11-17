#!/bin/sh
set -eu

: "${NAGI_ZIG:?NAGI_ZIG must name the Zig executable}"
: "${ZIG_LIB_DIR:?ZIG_LIB_DIR must name the Zig lib directory}"
: "${ZIG_GLOBAL_CACHE_DIR:?ZIG_GLOBAL_CACHE_DIR must be isolated per build}"
: "${ZIG_LOCAL_CACHE_DIR:?ZIG_LOCAL_CACHE_DIR must be isolated per build}"

exec "$NAGI_ZIG" cc -target x86_64-linux-musl "$@"
