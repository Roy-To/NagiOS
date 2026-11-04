# MPL-2.0 scope review and implementation record

Review date: 2026-10-27.

Reviewed main: `9edb2ae92526f3f97ed07bd873a337017ce37b2a`.
Original PR #84 candidate: `bfa8e5ef77d9d2def3dd58eecc280ecaf6cd6fca`.

## Review result

The original PR supplied the MPL text and an uncompleted checklist, but did not
identify an effective source grant. The maintainer requested review, definition
of scope and implementation of the previously selected MPL-2.0 policy.

This revision implements that instruction **only for the maintainer's licensable
original Nagi code** in [LICENSE_SCOPE.md](../../LICENSE_SCOPE.md). It preserves
prior grants and third-party material and explicitly excludes known provenance
holds. It is not a statement that every file's chain of title was independently
verified, nor a complete-image redistribution approval.

## Evidence actually examined

- Main root tree, `AGENTS.md`, root `Cargo.toml`, `kernel/Cargo.toml`, the current
  status introduction, English/Japanese README and CONTRIBUTING.
- The existing main `docs/oss/apache-2.0-readiness.md` review (2026-10-11).
- PR #60's `docs/oss/LICENSE_AUDIT.md` at
  `07f2bd1e77989a5d1bbbc22e5c046d909630b240` (2026-10-14). This is branch evidence;
  it was not silently merged and is not assumed to be the current main inventory.
- All four manifests previously identified as dual-licensed, re-read at the
  reviewed main revision, rather than inferred from the old reports.
- The first 100 lines of `user/nagi-init/src/font.rs`, confirming embedded glyph
  tables, not establishing their original author or source.
- PR #84 metadata, absence of existing review comments, and its LICENSE text.
- Mozilla's current [MPL text](https://www.mozilla.org/en-US/MPL/2.0/),
  [FAQ](https://www.mozilla.org/en-US/MPL/2.0/FAQ/) and
  [header guidance](https://www.mozilla.org/MPL/headers/).

### Exact preserved declarations

| Manifest | Blob SHA at reviewed main | Existing declaration |
| --- | --- | --- |
| `tools/nagi-cli/Cargo.toml` | `c4214f8350cd5bd62d8fdc7631b570b30d13900a` | MIT OR Apache-2.0 |
| `tools/legal/Cargo.toml` | `a8b74b742b05bcafc759deaa33e3f7d576bc2b64` | MIT OR Apache-2.0 |
| `crates/nagi-embedding-provider/Cargo.toml` | `1895cde7feadf256f651ccec6336adc0f1b887ba` | MIT OR Apache-2.0 |
| `crates/nagi-tts-provider/Cargo.toml` | `bca739ea8d7cbbf5d0a29a50a3e62e4aba1ed3c4` | MIT OR Apache-2.0 |

The embedding manifest references `tests/m24-embedding/contract.rs`,
`real_inference.rs` and `adversarial.rs` outside its package directory.
The m24/m25 test boundaries are therefore held out of this new grant rather
than assuming that package-directory exceptions exhaust earlier permissions.
The package declarations and all source bytes remain unchanged.

## Decisions made, not inferred facts

- The maintainer's instruction is the authority for choosing and granting MPL
  for rights they hold. Public visibility or a GitHub username is not proof of
  copyright ownership, contributor consent, or employment/assignment rights.
- The scope is a documented directory/file-boundary **licensing rule**, not a
  newly generated exhaustive file-by-file provenance inventory. A path match
  cannot override a more specific license or upstream origin.
- Known unresolved glyphs are excluded, not silently deemed original. Original
  Nagi software source is separated from model/media/documentation assets.
- Separate files with other licenses remain separate. No non-commercial term,
  proprietary exception, trademark grant, or new patent warranty is added.
- Source licensing and full binary redistribution review are different gates.
  Unfinished image/model reviews do not become completed merely because LICENSE
  is present; conversely the grant of the maintainer's own code is not described
  as contingent on clearing every external dependency first.

## Limitations and open items

This environment could read GitHub resources but could not clone/download the
repository for an exhaustive local scan. GitHub code search returned an
incomplete empty result; it was **not used as proof of absence** of other
licenses. No full history/author audit, external contributor agreement review,
employment-contract review, patent clearance, fetched dependency audit, model
artifact audit, or target SBOM generation was performed in this change.

The following remain open and must not be reported as cleared:

1. `user/nagi-init/src/font.rs` glyph origin/permission. Confirm origin and rights
   or replace with independently verified material in a separate implementation
   change, then update its scope deliberately.
2. Logo, screenshots and third-party media; model/voice/font/tokenizer artifact
   provenance and exact notices; any documentation licensing decision.
3. Exact release dependencies and native/source obligations, including the
   OpenLibm LGPL test notices and the Rust/Mesa/FreeType items in the earlier
   audits. Those audits' missing metadata counts are historical tool findings,
   not a refreshed count or proof that each dependency is unlicensed.
4. Any rights held by independent contributors or employers cannot be licensed
   merely by this maintainer instruction. Obtain the appropriate authority
   before representing those contributions as covered. Prior grants persist.
5. Reconcile application wording with the actually merged source revision and
   current program requirements. No support application is sent by this PR.

## Verification boundary

The change is license/scope/documentation only. No runtime code, manifests,
lockfiles, tests, workflows, milestones or workstream registration is changed.
LICENSE reuses the reviewed existing blob
`ee6256cdb62a765749a71aae3abea32884301cd1` without editing the legal text.
Review the final PR diff and the immutable head's CI separately. Repository
build, QEMU acceptance and the existing Legal scanner were not run locally.
A successful documentation/CI check is not legal clearance.
