# Nagi OS license audit

Audit date: **2026-10-14**

Repository baseline: **`bcb92dcf38d195498056f50a72f7d5ca12b0ea21`** (`main`)

> 日本語要約: Apache-2.0 は候補のままで、Nagi OS 全体へのライセンス付与は行われていません。第三者コード・モデル・フォント・画像などは個別の条件を保持します。既存のLegal inventoryではライセンス根拠がunknownの記録が888件あり、ソース／バイナリ公開前に確認が必要です。

## Scope and result

This is a source-tree preparation audit, not legal advice, a patent search, a
binary release clearance, or an approval to license Nagi OS. It reviews the
repository's `AGENTS.md`, `README.md`, `CONTRIBUTING.md`,
`THIRD_PARTY_NOTICES.md`, `third_party/sources.lock`, root and nested
`Cargo.lock` files, model/font/TTS/embedding pins, tracked license and notice
files, and the current legal inventory. The separate proposal and release
gates are in [APACHE_2_LICENSE_PROPOSAL.md](APACHE_2_LICENSE_PROPOSAL.md) and
[OSS_RELEASE_CHECKLIST.md](OSS_RELEASE_CHECKLIST.md).

**Apache-2.0 is a candidate, not a decision.** There is no project-wide Nagi
license grant in this change. No root `LICENSE`, SPDX headers, package license
fields, or third-party license edits are made. The current README correctly
says that Nagi's project-wide license is pending.

The review distinguishes tracked source from fetched sources, generated build
inputs, downloadable model assets, and a particular built image. A lock pin or
Cargo metadata declaration identifies a claim and an exact artifact; neither
alone proves the contents, all copyright holders, or redistribution rights.
The public repository's visibility is not a license grant.

## Existing license-inventory workstream

The existing `license-sbom` workstream owns implementation of the Legal
scanner, SPDX output, and NOTICE candidate generator. Its checkpoint and
limits are documented in [`docs/legal/README.md`](../legal/README.md), which
identifies `codex/0.2-license-sbom` as the owner branch and
`codex/0.2-integrate-provenance-sbom` as the integrated host checkpoint. This
audit reuses that implementation and does not change its owner state or add
another scanner, SBOM schema, or generated release inventory.

At this baseline, `./nagi legal check` reports **912 components**, **24 with a
declared license expression**, **888 with unknown, malformed, or conflicting
license evidence**, **886 with no license expression/file evidence**, and
**875 without a located NOTICE reference**, and **911 requiring manual
review**. The tool reports zero structural check errors. The unknown-license
findings group as 758 crates.io Cargo records, 115 Cargo records without a
resolved origin (including 48 project records), 11 Servo Stylo Git records, 2
NagiOS Git-source records, 1 Mesa source pin, and the separate Rust standard-
library pin. These are inventory findings, not 888 conclusions that the
artifacts are unlicensed. The tool could not obtain stable offline Cargo
metadata and deliberately leaves those fields unknown. The graph spans
tracked lockfiles and development/target-specific dependencies; it is not a
target image SBOM.

There are **24 tracked Cargo.lock files**. To regenerate evidence with the
existing tool, run:

```sh
./nagi legal scan --json --output /tmp/nagi-legal-inventory.json
./nagi legal check
```

Review every unknown and every selected file manually. The checker validates
its inventory structure and metadata evidence; a pass does not establish
permission to distribute. A missing NOTICE reference in this scan means no
reference was located, not that the corresponding upstream license requires a
NOTICE file.

## Nagi-owned code and third-party material

| Area | Repository boundary | Audit treatment |
| --- | --- | --- |
| Candidate Nagi work | `kernel/`, `loader/`, `services/`, `targets/`, `user/`, `apps/`, `crates/`, `idl/`, `schemas/`, `sdk/`, `tools/`, samples, project documentation and scripts | Treat as Nagi-owned only after confirming each contribution's author, employment/assignment terms, copied material, and generated inputs. No project-wide grant is made here. |
| Existing package declarations | `tools/nagi-cli`, `tools/legal`, `crates/nagi-embedding-provider`, and `crates/nagi-tts-provider` currently declare `MIT OR Apache-2.0` in their manifests | These declarations predate this audit. Their intended scope and rights-holder authority need maintainer review; they are not extended to other packages or the repository. Do not silently rewrite them. |
| Nagi adapters and patches | `third_party/*-patches/`, `third_party/relibc/src/nagi.rs`, and integration code beside fetched sources | Separate Nagi-authored changes from upstream files. Confirm authorship and patch provenance. Preserve the upstream terms for covered files. |
| Imported/vendored upstream source | `third_party/relibc/`, `third_party/libc/`, `third_party/inventory-nagi/`, plus imported subtrees such as OpenLibm | Keep upstream notices intact and audit nested files independently; a parent project's license label does not relicense an imported subcomponent. |
| Pinned fetched source | Components in [`third_party/sources.lock`](../../third_party/sources.lock), including Servo, Mesa, Rust source, llama.cpp, whisper.cpp, and LLVM libc++ | The lock and [THIRD_PARTY_NOTICES.md](../../THIRD_PARTY_NOTICES.md) are the starting inventory. Generated checkouts are not present in the tracked tree; inspect the exact pinned revision, patch set, selected files, and build output before binary distribution. |
| Cargo dependency graph | Root `Cargo.lock` plus the nested lockfiles listed below | Lockfiles establish versions, sources, checksums, and edges, not license expressions. Review all locks and narrow the result to the release target and feature set. |

The lockfile inventory includes paths under `loader/`, `tools/`, `crates/`,
`apps/`, `user/`, `samples/`, `tests/`, and `third_party/`; root `Cargo.lock`
does not represent the full repository graph.

## Pinned code and compatibility findings

| Component | Declared/source evidence | Compatibility and remaining work |
| --- | --- | --- |
| Servo and Stylo | `sources.lock` records Servo as MPL-2.0; current Servo source includes MPL-covered files, and 11 Stylo Cargo records remain unknown to the inventory | Apache-2.0 may cover eligible Nagi files in a larger work while MPL-covered files remain under MPL. Preserve MPL notices, mark covered modifications, provide corresponding source for executable distribution, and tell recipients how to obtain it. Do not relabel Servo or MPL-covered changes Apache-only. Review the exact pinned files and patch boundary. |
| SpiderMonkey / `mozjs-sys-nagi` | Pinned as MPL-2.0 with a Nagi patch directory | Apply the same file-level MPL obligations as Servo. Confirm the linked-source boundary, retained license text, patch set, and source access for any executable release. |
| Mesa Softpipe / Gallium | The source lock calls the selected core/Gallium code MIT and separately requires component notices | Mesa states that most code is MIT but individual files and components may carry other licenses. Audit the exact Softpipe build inputs, copied headers, generated code, and component license directory; do not treat the top-level label as clearance for all of Mesa. |
| Rust standard library / `rust-src` | Toolchain `nightly-2025-08-01` and a Nagi target patch; `sources.lock` has no license expression for this entry | Rust's compiler and standard library are generally dual-licensed MIT/Apache-2.0 unless a project/file says otherwise. Locate the exact toolchain's license and third-party inventory, inspect the selected `rust-src` and patch, and package required notices. Do not fill the lock's missing field by assumption. |
| `llama.cpp` / `ggml` | `llama.cpp` pin declares MIT; Nagi patches are separate | Retain the pinned upstream license and audit compiled `ggml`/native components and their own notices. Runtime code and model weights are separate works with separate terms. |
| Other permissive dependencies | `sources.lock` and manifests include MIT, BSD-3-Clause, ISC, 0BSD, and `MIT OR Apache-2.0` expressions; Surfman includes MPL-2.0 as an option | These licenses can remain on their own components alongside an Apache-2.0 Nagi license, subject to each license's conditions. Preserve copyright/license text and notices. For `OR`, record the selected option; for `AND`, satisfy every term. Inspect nested bundled code rather than relying only on package metadata. |
| OpenLibm tests inside relibc | `third_party/relibc/openlibm/LICENSE.md` identifies several upstream terms; `test/test-double.c` and `test/test-float.c` are marked LGPL-2.1-or-later | Confirm the complete LGPL text and whether either file enters a release source archive or binary. Preserve the obligations if distributed; do not infer coverage from relibc's MIT declaration. |
| FreeType and bundled components | `freetype-sys` wrapper metadata says MIT; the reviewed pinned archive includes FreeType FTL/GPL alternatives, libpng's PNG Reference Library License, and BSL-1.0 `dlg` | Decide and record the applicable FreeType licensing route for selected files with reviewer/counsel; retain subcomponent notices and required FreeType attribution. Wrapper metadata is not a complete notice set. |
| LLVM libc++ | Pin is Apache-2.0 WITH LLVM-exception; statically linked by the M20 guest image | Preserve both the Apache text and LLVM exception, plus the exact source notice, in any binary distribution that includes it. |

The official project references support these scope cautions: [MPL-2.0,
especially §§1.10 and 3.1–3.4](https://www.mozilla.org/en-US/MPL/2.0/),
[Mesa's file/component license guidance](https://docs.mesa3d.org/license.html),
[Rust licensing policy](https://rust-lang.org/policies/licenses/) and
[Rust compiler licensing notes](https://rustc-dev-guide.rust-lang.org/licenses.html),
and the [pinned llama.cpp license](https://github.com/ggml-org/llama.cpp/blob/c85b92c69c955961621193cd51da194f3cbcedf3/LICENSE).

## Models, fonts, voice, data, and images

| Asset | Tracked evidence | Open review / release treatment |
| --- | --- | --- |
| Granite 4.2 3B Q4_K_M | `third_party/models.lock` pins a GGUF artifact and declares Apache-2.0 | Verify the exact quantized artifact's provenance, license copy, base-model relationship, and notices. The generic license URL and lock entry do not prove that the same terms and notices cover every converted file. |
| Qwen3 4B Q4_K_M | `models.lock` pins a GGUF artifact and declares Apache-2.0 | Verify immutable weights/tokenizer provenance, conversion history, and any model-specific notice. |
| Gemma 3 1B IT Q4_K_M | `models.lock` declares Gemma Terms of Use, not Apache-2.0; the license model catalog still labels model entries fixture-only | Keep as a separate optional distribution decision. Google's terms include use restrictions and require a prescribed Notice for non-hosted distributions. Confirm the exact current terms and derivative status, or exclude the model from the distribution. |
| Whisper small multilingual | `models.lock` pins an external artifact and declares MIT | Review the exact model revision and notices separately from the MIT `whisper.cpp` runtime. The pin does not show that weights are in the repository or image. |
| multilingual-e5-small | `tools/embedding/manifest.toml` declares MIT and records source and converted-artifact hashes | Verify model, tokenizer and conversion provenance, preserve the upstream license, and reconcile inclusion with the release image. The reference ONNX file is marked reference-only. |
| Tohoku f01 voice and NAIST dictionary | `tools/tts/tts-artifacts.lock` records CC-BY-4.0 voice data and BSD-3-Clause dictionary/runtime terms; license, notice, attribution, and full COPYING files are tracked under `tools/tts/notices/` | Verify the artifact's actual packaged state. Preserve creator attribution, license link, modification status, and all dictionary/runtime notices. These terms do not become the Nagi code license. |
| Noto Sans and Noto Sans JP | `third_party/fonts.lock` pins OFL-1.1 font bytes and license texts; the Servo-enabled image includes them | Keep each OFL text and copyright notice with the font in an image. Confirm no modification and follow the OFL's reserved-name and no-sale-by-itself conditions if the fonts change. |
| Embedded desktop glyphs | `user/nagi-init/src/font.rs` contains bitmap glyph tables | Origin and author are not established by a font-file search. Identify original creation or source font and confirm the right to redistribute before a licensed source release. |
| Logo and screenshots | Six tracked images: `assets/nagi/nagi_logo_formal.svg`, three QEMU screenshots, and two imported OpenLibm documentation PNGs | Confirm logo artwork/branding rights, screenshot capture ownership and visible third-party web content, and image-specific permission/trademark terms for the imported PNGs. A source-tree code license does not resolve image rights. |
| Certificate and similar data | Cargo notices flag `webpki-roots` / `webpki-root-certs` under CDLA-Permissive-2.0 | Inspect the selected certificate data and retain data-specific notices in the target release inventory. |

The exact model pins are external artifact metadata; they do not mean those
weights are committed or bundled. [`docs/legal/model-assets.json`](../legal/model-assets.json)
is stale relative to the artifact locks and must be reconciled before the
release SBOM. Primary terms: [Gemma Terms of Use](https://ai.google.dev/gemma/terms),
[SIL OFL-1.1](https://openfontlicense.org/open-font-license-official-text/),
and [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/legalcode.en).

## Unresolved items, confirmation method, and risk

| Priority | Unresolved item | Confirmation method | Risk if unresolved |
| --- | --- | --- | --- |
| Release blocker | Rights to all Nagi-authored code, docs, adapters, patches, and generated inputs; ownership/employer terms and contributor history | Maintainer and each relevant rights holder review commit history, contribution agreements, employment/assignment obligations, and copied/generated source | Nagi may lack authority to grant the proposed license to part of the project. |
| Release blocker | Existing four `MIT OR Apache-2.0` manifest declarations and the intended scope of those earlier declarations | Maintainer/legal review of authorship, first-publication history, and package boundaries; record whether they are intentional | Conflicting or broader prior grants can make the project-wide policy inaccurate. |
| Release blocker | 888 Legal inventory unknown-license findings, including Cargo registry and Git records | Re-run `nagi legal scan --json`; for each release-selected package inspect its exact manifest/license/NOTICE at the locked revision and resolve all unknown/conflicting expressions | Restrictive or unobserved license terms may be redistributed without compliance; source-only metadata is insufficient. |
| High | Rust std, Mesa, FreeType, native bundles, and MPL source/patch boundaries | Materialize only pinned sources into a clean review environment; compare exact files, patch outputs, build inputs, and target links; ask counsel for file/combined-work questions | Missing notice or source offers, unintended copyleft obligations, or an unsupported compatibility assertion. |
| High | Model catalog and exact model/converted-artifact provenance, especially Gemma | Compare model IDs, immutable revisions, hashes, license files, notices, tokenizer/conversion inputs, and actual Model Store/image contents; review current Gemma terms | Model restrictions/attribution could be omitted, or users could be told a model is Apache-licensed when it is not. |
| High | Logo, bitmap glyphs, screenshots, and imported documentation images | Obtain author/source confirmations and reuse permission; review embedded third-party page content and trademark use | Copyright, privacy, or trademark claims may attach to source/docs/releases even when code licensing is correct. |
| High | Apache contributor patent grant scope and any existing patent commitments | Have counsel review contributor intake, rights-holder authority, contribution provenance, and relevant patent commitments; commission a separate search if needed | A contributor may be unable to grant the expected patent license; Apache-2.0 is not patent clearance. |
| Before binary release | Exact target-specific linked components and packaged notices/source | Build the release candidate, derive the selected target SBOM, inspect actual file/link inputs, and test that notices and MPL source access resolve from the shipped image/package | The source tree inventory may not match the binary and required recipient materials may be missing. |

The inventory is evidence for the follow-up work, not a legal conclusion. Keep
all unresolved rows open until the named evidence is recorded by the release
owner and reviewed where legal judgment is required.
