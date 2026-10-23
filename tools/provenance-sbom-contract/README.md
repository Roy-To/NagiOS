# Independent provenance / SBOM host contracts

Exercises the existing fingerprint public API and Legal library with independent wire fixtures and negative mutations. This standalone package is not adopted by the root workspace or shared CI.

```sh
cargo test --manifest-path tools/provenance-sbom-contract/Cargo.toml --locked --offline -j 1 -- --test-threads=1
cargo fmt --manifest-path tools/provenance-sbom-contract/Cargo.toml -- --check
cargo clippy --manifest-path tools/provenance-sbom-contract/Cargo.toml --locked --offline --all-targets -j 1 -- -D warnings
```

Coverage includes missing manifests and required fields, duplicate JSON keys (including every nested string map and escaped equivalent keys), artifact/input/package records, logical path traversal, tampered source digests, source revision and recorded artifact hash/size mismatches, and unknown-license preservation. Unknown licenses remain unknown / SPDX NOASSERTION and require review. Fingerprint comparison checks recorded metadata; it does not attest the current bytes of external artifacts or authorize artifact reuse.

Schema v1 represents the repository root with an empty relative path. Only directory inputs may use that name. A small Git-repository fixture verifies `--input .` generation and manifest read/compare; empty file/tool/artifact paths remain invalid.

Host validation does not activate runtime features or satisfy the M30 release gate.
