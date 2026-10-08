#!/usr/bin/env bash
set -euo pipefail
calendar_manifest="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/Cargo.toml"
cargo fetch --manifest-path "$calendar_manifest" --locked
cargo fmt --manifest-path "$calendar_manifest" --package nagi-calendar-core -- --check
cargo test --manifest-path "$calendar_manifest" --locked --offline
cargo clippy --manifest-path "$calendar_manifest" --all-targets --locked --offline -- -D warnings
