# Nagi OS OSS release checklist

Status: **preparation checklist; release is not cleared**

Review date: **2026-10-14**

Audit: [LICENSE_AUDIT.md](LICENSE_AUDIT.md) · Proposal:
[APACHE_2_LICENSE_PROPOSAL.md](APACHE_2_LICENSE_PROPOSAL.md)

> 日本語要約: ここに挙げる項目が閉じるまで、ソース公開やバイナリ配布をライセンス面で承認済みと説明しないでください。Apache-2.0の採用決定とライセンスファイル追加は、権利者の承認後に別変更として行います。

This checklist distinguishes a source repository release from a system image,
installer, model bundle, or other binary/data distribution. Mark a checkbox
complete only when the cited evidence is recorded for the exact release
revision. Review this list with the maintainer and qualified counsel where
indicated; it is not a legal opinion.

## A. Project license decision and ownership

- [ ] Rights holders approve the project license and exact Nagi-owned scope.
- [ ] Review authorship, commit history, employer/contractor rights, assignments,
  copied snippets, generated code, examples, documentation, and Nagi patches.
- [ ] Resolve the existing `MIT OR Apache-2.0` declarations in
  `tools/nagi-cli`, `tools/legal`, `crates/nagi-embedding-provider`, and
  `crates/nagi-tts-provider`; record their authorized scope without silently
  changing or broadening them.
- [ ] Review contribution terms and patent-grant authority with counsel;
  document known patent commitments and confirm no one treats Apache-2.0 as a
  patent clearance.
- [ ] Review Nagi name/logo and other marks separately from copyright licenses.
- [ ] Only after approval, make the separate license-application change:
  exact root `LICENSE` text, README status, contributor guidance, any approved
  manifest metadata, and a reviewed `NOTICE` if needed. Do not replace
  third-party license files or notices.

## B. Source-tree license inventory

- [ ] Read the current `THIRD_PARTY_NOTICES.md`,
  `third_party/sources.lock`, root `Cargo.lock`, and every nested lockfile for
  the exact release revision.
- [ ] Use the existing `license-sbom` tooling documented in
  [`docs/legal/README.md`](../legal/README.md); do not create a second scanner
  or assume its output is legal clearance.
- [ ] Save and review the JSON inventory from
  `./nagi legal scan --json --output /tmp/nagi-legal-inventory.json` for the
  release revision. Resolve every unknown, malformed, conflicting, or
  manually flagged item selected for the release; record upstream revision,
  license text, NOTICE, selected `OR` option, and target inclusion evidence.
- [ ] Generate an SBOM and NOTICE candidate with the existing tool, then have
  a reviewer verify the exact package/file set and the notice content. Do not
  ship a generated candidate without that review.
- [ ] Separate production, build, test, host-only, and reference dependencies.
  Mark test/dev-only LGPL and fuzzing packages accurately; do not omit source
  files from a source release just because they are absent from the guest
  binary.
- [ ] Record review of `AND` and `OR` expressions, nested native libraries,
  copied headers, generated sources, and target-specific dependencies.

## C. High-risk upstream boundaries

- [ ] For Servo/Stylo and SpiderMonkey, retain MPL texts and notices, mark
  MPL-covered modifications, verify the source/binary boundary, and test that
  executable recipients can obtain the corresponding source at the pinned
  revisions.
- [ ] For Mesa Softpipe/Gallium, enumerate the exact compiled and copied files,
  inspect file SPDX tags and component licenses, and retain all applicable
  notices.
- [ ] For Rust std, review the exact `nightly-2025-08-01` source, target patch,
  `COPYRIGHT`/third-party inventory, and the subset linked into the release.
  Resolve the missing license field in `third_party/sources.lock` based on
  that evidence rather than a general Rust license summary.
- [ ] For FreeType, libpng, and bundled `dlg`, confirm the selected licensing
  routes, file scope, attribution, and notices; obtain legal review for any
  GPL/FTL question.
- [ ] For OpenLibm, confirm whether LGPL-marked tests appear in source archives
  or binaries; include full required terms/source or exclude unused test files
  only after defining the source archive contents.
- [ ] For `llama.cpp`/`ggml`, whisper.cpp, LLVM libc++, and every native bundle,
  inspect the exact pinned checkout and linked subcomponents. Preserve
  exceptions such as `LLVM-exception`.

## D. Models, fonts, data, and images

- [ ] Reconcile `docs/legal/model-assets.json` with `third_party/models.lock`,
  `tools/embedding/manifest.toml`, `tools/tts/tts-artifacts.lock`, and actual
  Model Store/image contents. Distinguish planned, downloadable, cached, and
  bundled artifacts.
- [ ] For each shipped model, preserve immutable revision and hashes; verify
  base-model and quantization/conversion provenance, tokenizer terms, license
  copy, notice, and actual distribution conditions.
- [ ] Decide whether Gemma weights are excluded or distributed under the
  applicable Gemma Terms of Use; for any non-hosted distribution, include the
  prescribed Notice and enforce/communicate the required restrictions.
- [ ] Keep the Whisper model license separate from whisper.cpp; verify MIT
  model terms and artifact provenance.
- [ ] Keep Noto font binaries with their OFL-1.1 texts and attribution; verify
  whether fonts are modified and follow Reserved Font Name conditions.
- [ ] For the Tohoku voice, retain creator attribution, CC-BY-4.0 link, and
  modification status; include full NAIST dictionary COPYING and TTS crate
  notices.
- [ ] Confirm the origin and license of embedded bitmap glyphs, logo artwork,
  each screenshot (including third-party content visible in browser captures),
  and both imported OpenLibm documentation PNGs. Remove or replace only after
  documenting the source-release scope and rights decision.
- [ ] Review certificates and other data assets, including CDLA-Permissive-2.0
  roots, under their data-specific terms.

## E. Source release gate

- [ ] Confirm the source archive contains the notices and license texts needed
  for every included third-party file, including vendored subdirectories and
  dev/test files.
- [ ] Confirm a recipient can identify Nagi-owned files versus separately
  licensed upstream material without interpreting an Apache root license as
  overriding upstream terms.
- [ ] Confirm every published source pin has a reproducible fetch and patch
  path, and that source offers/instructions for MPL-covered executable code
  point to the corresponding revision and patches.
- [ ] Ensure README and release notes describe the approved Nagi license scope
  and keep model/data/artwork exceptions visible.
- [ ] Record a reviewed inventory and evidence snapshot tied to the exact tag
  or commit; identify known exceptions and remaining limitations in the
  release notes.

## F. Binary, image, or installer release gate

- [ ] Build the release candidate from a clean checkout and record its commit,
  toolchain, configuration, and artifact hashes.
- [ ] Derive the actual target-specific component/file set from the image and
  build/link inputs; compare it with the source inventory and resolve any
  differences.
- [ ] Package all applicable license texts, copyright notices, attributions,
  Apache `NOTICE` material, font/data terms, and model-specific terms with the
  image or accessible accompanying materials.
- [ ] Verify MPL corresponding-source access from a recipient's perspective.
- [ ] Confirm model weights, fonts, voices, dictionaries, firmware, or other
  artifacts are included only when the release decision and notices cover
  those exact bytes.
- [ ] Review brand/trademark usage and screenshots in the actual release
  materials.
- [ ] Have a second reviewer sign off on the target SBOM, notices, and all
  release blockers before announcing redistribution availability.

## Current blockers

At the 2026-10-14 baseline, Sections A, B, C, and D remain open. The existing
Legal inventory reports 888 unknown license records, 875 without a located
NOTICE reference, and 911 manual-review records; the structural PASS is not
release approval. The exact target image, complete source/notices set, model
inclusion, and rights-holder approval have not been cleared. Keep the project
license status pending until the maintainer records the required decisions
and evidence.
