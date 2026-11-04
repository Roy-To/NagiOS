#!/usr/bin/env sh
# HOST-ONLY regressions. No Nagi target link, native model build or guest run.
set -eu
cd "$(dirname "$0")/../.."
toolchain=nightly-2025-08-01
cargo=$(rustup which cargo --toolchain "$toolchain")
rustc=$(rustup which rustc --toolchain "$toolchain")
rustfmt=$(rustup which rustfmt --toolchain "$toolchain")
rustdoc=$(rustup which rustdoc --toolchain "$toolchain")
"$rustfmt" --check --edition 2021 --config skip_children=true \
  user/nagi-init/src/desktop.rs \
  user/nagi-init/src/font.rs \
  user/nagi-init/src/m19_files_client.rs \
  user/nagi-init/src/bar_adapter.rs \
  user/nagi-init/src/bar_panel.rs \
  user/nagi-init/src/session_ui.rs \
  tests/session-ui/build.rs \
  tests/session-ui/path_guard.rs \
  tests/session-ui/src/lib.rs \
  tests/session-ui/src/session_services.rs \
  tests/session-ui/src/tests.rs \
  tests/session-ui/src/font_tests.rs \
  tests/session-ui/font-host.rs \
  tests/session-ui/src/summary_tests.rs \
  tests/session-ui/desktop-host/src/lib.rs \
  tests/session-ui/desktop-host/libnagi/src/lib.rs \
  tests/session-ui/desktop-host/src/session_services.rs \
  tests/session-ui/desktop-host/src/desktop_tests.rs
RUSTC="$rustc" RUSTDOC="$rustdoc" "$cargo" test --manifest-path tests/session-ui/Cargo.toml --locked -- --test-threads=1
desktop_login_tree=$("$cargo" tree --manifest-path tests/session-ui/desktop-host/Cargo.toml \
  --no-default-features --features desktop-login-acceptance --locked --prefix none)
if printf '%s\n' "$desktop_login_tree" | grep -q '^nagi-model-manager '; then
  printf '%s\n' "desktop-login-acceptance unexpectedly enables nagi-model-manager" >&2
  exit 1
fi
RUSTC="$rustc" RUSTDOC="$rustdoc" "$cargo" test --manifest-path tests/session-ui/desktop-host/Cargo.toml --locked -- --test-threads=1
RUSTC="$rustc" RUSTDOC="$rustdoc" "$cargo" test --manifest-path tests/session-ui/desktop-host/Cargo.toml --no-default-features --features desktop-login-acceptance --locked -- --test-threads=1
