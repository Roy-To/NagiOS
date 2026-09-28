# Nagi OS dependency inventory and license evidence

`tools/legal` provides a host-only, offline-first inventory and SBOM tool. It
reads tracked Cargo manifests and lockfiles, `third_party/sources.lock`, the
tracked license/notice filenames, the existing `THIRD_PARTY_NOTICES.md`, and
the model-license metadata catalog in this directory. It does not fetch source
or model assets and does not edit upstream sources.

Run from the repository root:

```sh
cargo run --manifest-path tools/legal/Cargo.toml --locked --offline -- scan
cargo run --manifest-path tools/legal/Cargo.toml --locked --offline -- sbom --output /tmp/nagi.spdx.json
cargo run --manifest-path tools/legal/Cargo.toml --locked --offline -- notice --output /tmp/NOTICE.candidate.md
cargo run --manifest-path tools/legal/Cargo.toml --locked --offline -- check
cargo run --manifest-path tools/legal/Cargo.toml --locked --offline -- diff --before old-inventory.json --after new-inventory.json
```

The package can be wired into `./nagi legal` after integration ownership is
assigned. Until then, `nagi-legal` is an independent Rust host tool and does
not change the shared `tools/nagi-cli` command surface.

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

## Integration proposal

The workstream registry `.dev/workstreams.json` has no `license-sbom` entry
and is integration-owned. Add this row after the `development-foundation`
entry when the integration owner accepts the workstream:

```json
{
  "id": "license-sbom",
  "owner": "Codex OSS license and SBOM workstream",
  "owner_branch": "codex/0.2-license-sbom",
  "recommended_worktree": "../NagiOS-0.2-license-sbom",
  "state_file": ".dev/workstreams/license-sbom/state.json",
  "dependencies": [],
  "allowed_paths": [
    "tools/legal/**",
    "docs/legal/**",
    ".dev/workstreams/license-sbom/**"
  ],
  "forbidden_paths": [
    ".dev/workstreams.json",
    ".dev/schemas/**",
    "Cargo.toml",
    "Cargo.lock",
    ".github/workflows/**",
    "docs/implementation_status.md",
    "kernel/**",
    "loader/**",
    "user/**",
    "third_party/**",
    "out/**",
    "target/**"
  ],
  "activation_gate": "Host-side compliance tooling only; do not activate Nagi 0.2 runtime work or change M18 behavior.",
  "merge_boundary": "Review on codex/integration-next-phase after integration-owned CLI and CI changes are approved; never merge directly to main."
}
```

To expose the preferred `./nagi legal ...` UX, the integration owner can add
`nagi-legal = { path = "../legal" }` to `tools/nagi-cli/Cargo.toml`, add a
`Legal` command accepting trailing arguments in `tools/nagi-cli/src/commands.rs`,
and dispatch `args[1..]` to `nagi_legal::cli::run_from(..., root)`. Update the
root `Cargo.lock`, the CLI surface tests in `tools/nagi-cli/tests/cli.rs`, and
the root CI workflow to run the standalone legal crate's locked offline tests.
The existing `nagi` and `nagi.ps1` wrappers already forward command arguments.
All of these integration-owned paths remain unchanged in this workstream; the
standalone command above remains usable while that proposal is pending.
