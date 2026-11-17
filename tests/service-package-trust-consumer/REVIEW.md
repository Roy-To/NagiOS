# Review record

## Scope

Standalone external consumer of public `nagi-package` and
`nagi-init-package-provider::package_trust` APIs. The test uses a public RFC 8032
Developer Preview vector and the existing deterministic alternate test key;
neither is a production credential or a product trust-store claim. No package
implementation, catalog, launch code, or prior artifact-preflight files were
changed.

## Commands and results

- Pinned-nightly host run:

  ```sh
  RUSTC="$(rustup which rustc --toolchain nightly-2025-08-01)" \
  RUSTDOC="$(rustup which rustdoc --toolchain nightly-2025-08-01)" \
  rustup run nightly-2025-08-01 cargo test \
    --manifest-path tests/service-package-trust-consumer/Cargo.toml \
    --offline --locked \
    --target-dir tests/service-package-trust-consumer/target
  ```

  Result: **3 integration tests passed**, unit/doc tests 0; no network fetch.
- Pinned-nightly Linux compile:

  ```sh
  RUSTC="$(rustup which rustc --toolchain nightly-2025-08-01)" \
  RUSTDOC="$(rustup which rustdoc --toolchain nightly-2025-08-01)" \
  rustup run nightly-2025-08-01 cargo check \
    --manifest-path tests/service-package-trust-consumer/Cargo.toml \
    --offline --locked --tests \
    --target x86_64-unknown-linux-musl \
    --target-dir tests/service-package-trust-consumer/target
  ```

  Result: **passed**. This is a Linux-target compile only, not Linux runtime
  execution.
- Pinned-nightly `cargo fmt --manifest-path tests/service-package-trust-consumer/Cargo.toml -- --check` with `RUSTFMT` resolved from rustup: **passed**.
- `git diff --check`: **passed**.
- The first bare Cargo host attempt selected the PATH Homebrew x86-64 toolchain
  on this arm64 macOS host and failed linking build scripts. Running pinned
  nightly Cargo with its `RUSTC` and `RUSTDOC` paths from `rustup which` passed.
- `entire session current`: no state exists for this current Codex session.
  Session remains **unrecorded**; no Entire settings were changed.

## Covered behavior and boundary

- Valid fixture signed by the injected test key installs and inspects through
  public APIs; the installed record and executable are checked.
- Manifest, executable, and signature-byte tampering; another signer; and
  missing signature return their expected public errors. The test compares the
  full installed inventory before and after both inspection and install
  rejection.
- A signed different AppId passed to `capture` returns `IdentityMismatch`
  without changing the inventory.
- `PackageSecurityProvider::inspect` has no caller-supplied expected identity
  or entry argument. `PackageServiceSessionOwner::capture` takes expected
  package ID but no expected entry. For an installed package, a same-ID signed
  entry substitution can be captured, and the lease rejects dispatch with
  `ManifestChanged` after comparing the full installed manifest. This does not
  prove a production catalog's canonical entry is wired to this API; no such
  expected-entry API or policy was invented here.

These are host consumer regressions and a Linux compile. They do not establish
production package signer trust, system role, user authentication, launch
authorization, or runtime `READY`.
