# Build provenance and artifact fingerprints

`nagi dev fingerprint` records the host-side inputs and output files associated
with a Nagi build. It provides evidence for later review and artifact reuse
decisions; it does not build, cache, publish, or reuse artifacts.

## Commands

```sh
./nagi dev fingerprint
./nagi dev fingerprint --output out/nagi-build.json
./nagi dev fingerprint --target targets/x86_64-unknown-nagi-user.json \
  --feature m17-servo --profile release \
  --build-command 'cargo build -p nagi-init --features m17-servo --release' \
  --artifact system-image=out/artifacts/nagi.img
./nagi dev fingerprint compare out/fingerprints/left.json out/fingerprints/right.json
./nagi dev fingerprint compare out/fingerprints/left.json out/fingerprints/right.json --json
```

Generation writes a versioned JSON document to stdout or to a new file. An
existing output file is never overwritten. Each requested artifact must be a
regular file and is recorded with its role, logical path, byte length, and
SHA-256 digest. When no role is supplied, the filename is used. Missing or
unsupported artifact paths fail the command.

`--input` adds a generated file or directory to the compatibility inputs.
Directory inventories use sorted relative file names and content digests;
directory symlinks are rejected and the inventory is limited to 20,000 files.
Environment-provided generated inputs used by the current target build are
captured by content. External paths are represented as `external/<filename>`
so the host's absolute path is not exposed.

## Fingerprint contract

The document has three top-level sections:

- `compatibility` contains `digest_algorithm`, `digest`, and normalized
  compatibility inputs.
- `artifacts` is a sorted list of output role/path/length/SHA-256 records. Output
  metadata is deliberately excluded from the compatibility digest.
- `provenance` contains the creation timestamp and fingerprint producer name
  and version. The timestamp is excluded from compatibility equality.

The compatibility digest is SHA-256 over the deterministic JSON serialization
of the versioned compatibility input structure. Maps are ordered, and feature,
generated-input, firmware, and artifact lists are sorted before serialization.
Schema version 1 rejects unknown fields, unsupported versions, invalid digests,
and a digest that does not match the normalized inputs.

The compatibility inputs record repository identity, Git commit and dirty-tree
state, the pinned Rust/Cargo toolchain and host triple, the selected build target,
features/profile/build-command and flag digests, relevant allowlisted
environment-input digests, third-party lock revisions and source hashes,
submodule revisions, Nagi patch/adapter inventory digest, build configuration
digest, generated input digests, and QEMU/firmware identity where available.
Custom target JSON references are recorded by their stable target name, while
the target JSON content is included in the build-configuration digest.

Environment values are never written verbatim. Values are selected from a
small allowlist plus Cargo profile settings and the selected target's linker,
runner, rustflags, and rustdocflags. Secret-like variable names or assignments
are omitted. Tool and firmware files are recorded by content rather than by
absolute path. Repository, home, and temporary-directory prefixes in allowed
flag values are normalized before hashing.

## Comparison

`compare` validates both documents before comparing them. Human output names
each mismatch class and field. `--json` returns a stable report with both
compatibility digests, input-compatibility and artifact-inventory booleans, and
the detailed mismatches. A mismatch returns a non-zero exit code, including
when inputs match but the artifact inventory differs.

Mismatch classes include source revision, dirty source state, toolchain,
target, features, build profile/command/flags, environment, third-party
source/patch, build configuration, generated inputs, firmware/runtime harness,
and artifact digest.

## Reuse and acceptance boundary

A matching fingerprint is **necessary but not sufficient** evidence for
declaring a milestone or acceptance PASS. Any future reuse mechanism must also
verify immutable source/fingerprint identity, the artifact checksum, applicable
acceptance evidence, that no host-built artifact substitutes for guest work,
and the current milestone policy. This command does not alter clean-build
verification or any M17/M18 acceptance gate. Nagi 0.2 runtime/product work
remains subject to the existing M30 and explicit integration gates.

For stable evidence, write fingerprints under ignored output storage such as
`out/`, not into the source tree; create nested output directories first. A
dirty-source digest includes tracked changes and non-ignored untracked files,
using repository-relative names and file content; it does not include ignored
build outputs.
