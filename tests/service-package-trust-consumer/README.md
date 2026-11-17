# External package-trust consumer regressions

This standalone Cargo workspace exercises the public APIs in
`nagi-package` and `nagi-init-package-provider::package_trust` from an external
consumer crate. It does not include or recompile either implementation through
private source paths. The existing `package-identity-negative-20261115`
workspace tests a different legacy identity adapter and is not duplicated here.

The fixture signer uses the repository's public RFC 8032 Developer Preview
test vector plus the provider tests' deterministic alternate test key. These
are test fixtures only. The Developer Preview public key is not a product trust
store, and these tests do not establish production signer trust, system role,
user authentication, authorization, or runtime `READY`.

## Host run (offline, isolated target)

```sh
RUSTC="$(rustup which rustc --toolchain nightly-2025-08-01)" \
RUSTDOC="$(rustup which rustdoc --toolchain nightly-2025-08-01)" \
rustup run nightly-2025-08-01 cargo test \
  --manifest-path tests/service-package-trust-consumer/Cargo.toml \
  --offline --locked \
  --target-dir tests/service-package-trust-consumer/target
```

## Linux compile (separate from host execution)

```sh
RUSTC="$(rustup which rustc --toolchain nightly-2025-08-01)" \
RUSTDOC="$(rustup which rustdoc --toolchain nightly-2025-08-01)" \
rustup run nightly-2025-08-01 cargo check \
  --manifest-path tests/service-package-trust-consumer/Cargo.toml \
  --offline --locked \
  --target x86_64-unknown-linux-musl \
  --target-dir tests/service-package-trust-consumer/target
```

Coverage includes a valid signed package control, manifest/executable/signature
tampering, another signer, missing signature, expected AppId substitution,
same-AppId entry substitution, expected public errors, and inventory equality
before/after rejected inspection/install attempts.

`PackageSecurityProvider::inspect` authenticates a package and returns its
signed manifest, but has no caller-supplied expected identity or entry
parameter. `PackageServiceSessionOwner::capture` accepts an expected package ID;
it does not accept a separate expected entry. For an installed package, the
existing lease API compares the full manifest and rejects an entry substitution
with `ManifestChanged`. This is a host consumer contract check, not proof that a
production catalog or launch path binds a canonical expected entry.
