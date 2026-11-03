#!/usr/bin/env bash
set -euo pipefail
slides_manifest="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/Cargo.toml"
cargo fmt --manifest-path "$slides_manifest" --package nagi-slides-core -- --check
cargo test --manifest-path "$slides_manifest" --locked --offline
cargo clippy --manifest-path "$slides_manifest" --all-targets --locked --offline -- -D warnings
