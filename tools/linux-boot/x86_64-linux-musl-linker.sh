#!/bin/sh
exec /usr/bin/clang --target=x86_64-unknown-linux-musl -fuse-ld=lld "$@"
