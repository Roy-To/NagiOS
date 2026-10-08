# Nagi OS dependency inventory and license evidence

`tools/legal` provides a host-only, offline-first inventory and SBOM tool. It
reads tracked Cargo manifests and lockfiles, `third_party/sources.lock`, the
tracked license/notice filenames, the existing `THIRD_PARTY_NOTICES.md`, and
the model-license metadata catalog in this directory. It does not fetch source
or model assets and does not edit upstream sources.

Prepare the pinned host dependencies once when using a fresh Cargo cache:

```sh
cargo fetch --manifest-path tools/legal/Cargo.toml --locked
```

This prepares Cargo packages only; it does not fetch upstream project sources,
model assets or license evidence. Subsequent Legal execution stays offline.
The root/bootstrap lockfile alone does not prepare Legal's separate lock graph.

Run from the repository root:

```sh
cargo run --manifest-path tools/legal/Cargo.toml --locked --offline -- scan
cargo run --manifest-path tools/legal/Cargo.toml --locked --offline -- sbom --output /tmp/nagi.spdx.json
cargo run --manifest-path tools/legal/Cargo.toml --locked --offline -- notice --output /tmp/NOTICE.candidate.md
cargo run --manifest-path tools/legal/Cargo.toml --locked --offline -- check
cargo run --manifest-path tools/legal/Cargo.toml --locked --offline -- diff --before old-inventory.json --after new-inventory.json
```

The approved host checkpoint exposes these same commands through
`./nagi legal ...` (or `./nagi.ps1 legal ...` on Windows). The host CLI delegates
to the existing standalone manifest with `cargo run --quiet --locked --offline`,
preserving stdout, stderr and the exit code. Its standalone dependency graph
and lockfile remain intact; the POSIX/PowerShell launchers use bootstrap mode
for Legal, so no root target dependency materialization is required.

## Evidence rules

- Cargo package identity, version, registry source, checksum, and dependency
  edges come from checked-in `Cargo.lock` files. Direct dependency scope and
  package license fields come from stable `cargo metadata --no-deps` output
  and checked-in package manifests. Host Cargo caches are not scraped, so
  absent transitive license metadata remains unknown and identical checkouts
  do not depend on machine-local cache contents.
- Third-party pins come from `third_party/sources.lock`. License and notice
  references come from Git's tracked-file index: its output is capped at
  64 MiB and 100,000 paths, metadata file reads at 8 MiB, and per-component
  evidence at 256 files. License files can be inherited from a parent
  directory inside a pinned `third_party` tree. Generated checkouts, build
  trees, caches, and ignored files are not traversed. Non-Git fixture scans
  cap directory walking at 30,000 entries and depth 8.
- A valid explicit SPDX expression is preserved as declared. Opaque provider
  terms, prose, absent metadata, and conflicting declarations remain unknown
  or require manual review. No license is inferred from a filename or vague
  prose.
- SPDX license identifiers and exceptions are checked against the pinned
  [SPDX License List 3.29.0](https://github.com/spdx/license-list-data/tree/v3.29.0).
  `LicenseRef-*` expressions remain unclassified because this generator does
  not synthesize extracted license text or license-reference records.
- SPDX JSON uses SPDX 2.3. The generated document is checked against the
  required SPDX 2.3 fields and cross-reference rules used by this generator;
  this is a focused profile check, not a legal determination or a replacement
  for a full independent SPDX validator. The SPDX 2.3 specification describes
  the mandatory document identity and package fields in its
  [SPDX-Lite profile](https://spdx.github.io/spdx-spec/v2.3/SPDX-Lite/).
- The NOTICE output is a candidate evidence report. It references source text
  and repository metadata, marks manual review, and never rewrites or copies
  upstream license text.
- Planned model manifests are represented separately from included SBOM
  components. The current model fixture terms and notice references are
  placeholders; they are not treated as authoritative licenses or proof that
  model weights are bundled.

## Registered host integration

`license-sbom` is registered by the user-approved BP-SBOM-HOST-20261008
checkpoint on `codex/0.2-integrate-provenance-sbom`; its owner branch remains
`codex/0.2-license-sbom`. The owner's imported State remains unchanged.
The checkpoint State records adoption, shared CLI compatibility and exact CI
results separately. No Legal source algorithm or release acceptance was changed.

The dedicated Ubuntu/Windows host workflow runs the standalone Legal format,
warning-denied Clippy and locked offline tests, plus shared CLI scan/check/SBOM/
NOTICE/diff tests. The general 0.1 CI and target gates retain their existing
behavior on all other branches. Runtime activation and release compliance
claims remain outside this host checkpoint.
