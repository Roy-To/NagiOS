#!/usr/bin/env sh
# HOST orchestration and Nagi COMPILE-ONLY checks. Never guest inference.
set -eu
cd "$(dirname "$0")/../.."
manifest=tests/session-ui/Cargo.toml
desktop_host=tests/session-ui/desktop-host/Cargo.toml
cargo fmt --manifest-path "$manifest" -- --check
cargo fmt --manifest-path "$desktop_host" --package nagi-desktop-lock-host-tests --package nagi-desktop-host-libnagi -- --check
cargo test --manifest-path "$manifest" --locked --target x86_64-unknown-linux-gnu
cargo test --manifest-path "$desktop_host" --locked --target x86_64-unknown-linux-gnu desktop::regression::
cargo test --manifest-path "$desktop_host" --locked --no-default-features --features desktop-login,desktop-login-acceptance --target x86_64-unknown-linux-gnu desktop::regression::
cargo clippy --manifest-path "$desktop_host" --locked --package nagi-desktop-host-libnagi --target x86_64-unknown-linux-gnu -- -D warnings
cargo clippy --manifest-path "$manifest" --locked --all-targets --target x86_64-unknown-linux-gnu -- -D warnings
cargo clippy --manifest-path "$manifest" --locked --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,alloc -- -D warnings
cargo check --manifest-path "$manifest" --locked --features desktop-check --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,alloc
