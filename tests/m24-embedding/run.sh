#!/usr/bin/env bash
# M24 embedding acceptance runner (HOST inference only).
#
# 1. fetch pinned upstream files (SHA-256 verified)      tools/embedding/fetch.sh
# 2. convert deterministically and verify the pinned .nemb digest
# 3. fmt / clippy -D warnings / model-free contract tests
# 4. real-inference tests (--ignored; they FAIL if the model is absent)
# 5. Nagi user-target compile of the no_std provider (if rust-src is present)
#
# This never runs guest inference. A pass here is host evidence only.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
crate="$repo/crates/nagi-embedding-provider"
manifest="$repo/tools/embedding/manifest.toml"
export NAGI_EMBEDDING_CACHE=${NAGI_EMBEDDING_CACHE:-$HOME/.cache/nagi-embedding}
log_dir=${NAGI_M24_EVIDENCE_DIR:-$NAGI_EMBEDDING_CACHE/evidence}
mkdir -p "$log_dir"
log="$log_dir/m24-host-$(date -u +%Y%m%dT%H%M%SZ).log"
exec > >(tee "$log") 2>&1

echo "m24: repo $(git -C "$repo" rev-parse HEAD) $(uname -sm)"
input=$("$repo/tools/embedding/fetch.sh" | tail -n 1)
artifact="$NAGI_EMBEDDING_CACHE/multilingual-e5-small.nemb"
pinned=$(python3 -c "import tomllib,sys;print(tomllib.load(open(sys.argv[1],'rb'))['converted']['sha256'])" "$manifest")
if [[ ! -f "$artifact" ]] || [[ $(sha256sum "$artifact" | cut -d' ' -f1) != "$pinned" ]]; then
    python3 "$repo/tools/embedding/convert_e5.py" --input "$input" --output "$artifact"
fi
actual=$(sha256sum "$artifact" | cut -d' ' -f1)
if [[ "$actual" != "$pinned" ]]; then
    echo "m24: converted artifact digest $actual != pinned $pinned" >&2
    exit 1
fi
echo "m24: artifact $artifact sha256 $actual (pinned)"

cargo fmt --manifest-path "$crate/Cargo.toml" -- --check
cargo clippy --manifest-path "$crate/Cargo.toml" --all-targets --locked -- -D warnings
cargo clippy --manifest-path "$crate/Cargo.toml" --lib --no-default-features --locked -- -D warnings
cargo test --manifest-path "$crate/Cargo.toml" --locked
NAGI_EMBEDDING_MODEL="$artifact" cargo test --manifest-path "$crate/Cargo.toml" --locked \
    --test m24-embedding-real-inference -- --ignored --test-threads=1 --nocapture

if rustup component list --installed 2>/dev/null | grep -q '^rust-src'; then
    (cd "$crate" && CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$crate/target}/nagi-user" \
        cargo -Z build-std=core,alloc build --release --locked --no-default-features \
        --target "$repo/targets/x86_64-unknown-nagi-user.json")
    echo "m24: Nagi user-target (x86_64-unknown-nagi-user) library build: PASS (compile only, not executed)"
else
    echo "m24: rust-src missing; Nagi user-target build NOT RUN"
fi
echo "m24: HOST acceptance PASS (guest inference not run; M24 remains PARTIAL). Log: $log"
