# Release bundle tracked license texts

## Problem

M30 release bundles currently include `THIRD_PARTY_NOTICES.md` but omit the
license and notice text files already tracked under `third_party/`. The
manifest therefore checksums only the summary, not those available texts.

## Scope

- Have release preflight and assembly enumerate tracked third-party files
  named `LICENSE`, `LICENCE`, `COPYING`, or `NOTICE`, including common
  extension forms and nested directories.
- Copy them byte-for-byte to `licenses/source-tree/<original path>` in stable
  order and record source path, package path, and SHA-256 in the build manifest.
- Make new bundles verify that their recorded tracked-license inventory is
  present and hashes correctly, while continuing to verify older schema-v1
  bundles that lack the additive inventory field.
- Add focused tests and document the precise limit of this inventory.

## Non-goals

This does not gather fetched-but-untracked upstream texts or Cargo transitive
license texts, choose a Nagi OS license, interpret redistribution terms, or
authorize a binary release. `THIRD_PARTY_NOTICES.md` remains the metadata
summary and the manual distribution review remains open.

## Verification

Test tracked-file discovery with a temporary Git repository, run the release
tool suite and repository checks, then assemble/verify a fresh clean-source
bundle and inspect its checksums. Confirm that an existing schema-v1 bundle
still verifies.
