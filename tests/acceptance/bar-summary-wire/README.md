# Staged Bar summary transport checks

Run `sh tests/acceptance/bar-summary-wire/check.sh` with the installed repository
toolchain and approved offline Cargo cache. The harness uses the actual protocol
and client source through path dependencies. Host client main is intentionally
empty: host compilation does not execute its Nagi syscall path.

Target type check (existing installed target/build-std prerequisites only):

```sh
cargo check --manifest-path tests/acceptance/bar-summary-wire/Cargo.toml \
  --lib --bin nagi-bar-client-source-check \
  --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core --locked --offline
```

Standalone target ELF link, from the repository root:

```sh
RUSTFLAGS="-C link-arg=-T$(pwd)/user/nagi-init/linker.ld --cfg getrandom_backend=\"custom\"" \
  cargo build --manifest-path tests/acceptance/bar-summary-wire/Cargo.toml \
  --bin nagi-bar-client-source-check --target targets/x86_64-unknown-nagi-user.json \
  -Zbuild-std=core,compiler_builtins -Zbuild-std-features=compiler-builtins-mem \
  --release --locked --offline
```

The absolute linker-script path uses this same checkout because Cargo changes the
link working directory. This local-only harness does not build the production
package, sign/embed/launch it, boot a guest, or verify inference. No positive model
result is defined by this protocol. A child exit status of zero only means it
consumed a negative transport terminal. Desktop endpoint and retirement cases live
in `tests/session-ui/check.sh` and `tests/acceptance/supervisor-receive`.
