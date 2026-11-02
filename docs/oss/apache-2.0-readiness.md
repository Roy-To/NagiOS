# Apache-2.0 readiness review — proposal only

Review date: **2026-10-11**. Source baseline:
[`379d19eadb6f8792c27b9bc3f147e1ea48eac4a2`](https://github.com/RT-NISH/NagiOS/commit/379d19eadb6f8792c27b9bc3f147e1ea48eac4a2).

**Decision requested later: Apache-2.0 for eligible Nagi-owned code and
documentation, with explicit third-party and asset exceptions.**
This is a preparation review, not approval, a license grant, or a complete
redistribution clearance. No root LICENSE, SPDX headers, package license
changes, or milestone changes are made here.

日本語要約: Apache-2.0 は Nagi 独自部分の第一候補です。第三者部分を一括して
再ライセンスできません。Servo／SpiderMonkey の MPL、OpenLibm の LGPL テスト、
FreeType の内包ライセンス、OFL フォント、Gemma の独自規約、TTS の CC-BY 表示、
画像・独自グリフの出自、既存 crate のライセンス表記を確認する必要があります。
公開リポジトリであることは OSS ライセンス付与を意味しません。
ルート LICENSE の追加は所有者の明示承認後の別変更です。

## Scope and evidence

Inspected the tracked tree, package manifests and lockfile locations,
[THIRD_PARTY_NOTICES](../../THIRD_PARTY_NOTICES.md),
[source pins](../../third_party/sources.lock),
[model pins](../../third_party/models.lock),
[font pins](../../third_party/fonts.lock),
[TTS pins](../../tools/tts/tts-artifacts.lock),
[embedding pins](../../tools/embedding/manifest.toml),
tracked upstream license texts and patch boundaries, and the existing
[Legal inventory tooling](../legal/README.md).

This review separates:

- tracked source/documentation/assets already in the public repository;
- fetched/derived sources, fonts, and model artifacts;
- a selected guest image or installer and its actual linked dependencies;
- host-only build, reference, and test tools.

A targeted native-source review downloaded the locked freetype-sys 0.23.0
archive and verified SHA-256
`eab537ce43cab850c64b4cdc390ce7e4f47f877485ddc323208e268280c308ae`.
Its wrapper LICENSE, FreeType LICENSE.TXT/FTL.TXT, libpng LICENSE and dlg
LICENSE were inspected in memory; no fetched source was installed or changed.
A complete fetched-source audit, target SBOM, and model download were not
performed for this documentation change. Declared metadata is evidence
of a publisher's claim, not verified permission for every contained file.
The 2026-09-30 audit recorded 673 external Cargo packages / 41 expressions;
that is historical metadata-only evidence, not a refreshed inventory or
the guest-image bill of materials. Root Cargo.lock is not the only graph:
loader, SDK/sample tooling, Legal, first-party crates, embedding, TTS and
Whisper tests also have standalone lockfiles. Embedding reference Python
requirements are a separate, host-only graph.

## Why Apache-2.0 is the leading candidate

It supports broad reuse with an explicit contributor patent grant. That fits
an independent OS with native APIs, ports, and shared developer tooling.
The grant only covers patent claims within its defined scope; it is not a
patent clearance. Redistribution requires license retention, applicable
notices, and changed-file notices. Upstream NOTICE material must be retained
when applicable. It does not grant trademark rights.

Reference: [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0),
especially §§3–6. Keep the authoritative English text if later approved.
A root NOTICE may be useful after the actual notices are reviewed; a candidate
generated inventory is not automatically a compliant NOTICE.

Apache-2.0 can describe Nagi's own eligible material without erasing the
licenses of separately licensed dependencies. It cannot make the entire
source tree, images, fonts, or model weights “Apache-2.0 only.”

## Code and dependency findings

| Component / boundary | Evidence at baseline | Concern and required action |
| --- | --- | --- |
| Nagi-owned packages | Most tracked first-party manifests have no license field; `tools/nagi-cli`, `tools/legal`, `crates/nagi-embedding-provider`, and `crates/nagi-tts-provider` declare `MIT OR Apache-2.0`. | Confirm authorship, contributor/employer rights, copied/generated code, and the scope of existing declarations. A manifest field alone is not a full text/provenance audit. Do not silently revoke existing permissions or rewrite these fields; resolve the scope with the maintainer in a separate approved change. |
| relibc and vendored libc/inventory | relibc MIT; libc and inventory MIT OR Apache-2.0; license texts are tracked. Nagi-specific adaptations and patches are identifiable. | Preserve upstream copyright/license text. Check imported subdirectories individually rather than treating relibc's MIT declaration as covering everything. |
| OpenLibm inside relibc | [LICENSE.md](../../third_party/relibc/openlibm/LICENSE.md) lists MIT, BSD, ISC, Sun notices, public-domain material, and LGPL tests. `test/test-double.c` and `test/test-float.c` carry LGPL-2.1-or-later headers. | Concrete copyleft exception in the tracked source tree. Verify the complete LGPL text and source-distribution requirements before presenting a source archive as license-ready; no separate COPYING was found in this import. Determine whether these tests enter any shipped binary. Preserve their terms; exclude them from a binary when unused, not from the audit. |
| Servo and SpiderMonkey (`mozjs-sys-nagi`) | MPL-2.0 in source pins, patched through `third_party/servo-patches/` and `third_party/mozjs-sys-nagi-patches/`. | MPL covered files and modifications retain MPL. For executable distribution make the corresponding covered source available and tell recipients how to obtain it. Keep an exact upstream revision plus reproducible patches/build instructions; inspect copied-code boundaries. Static linking does not itself relicense all Nagi source. |
| Surfman | MIT OR Apache-2.0 OR MPL-2.0 at pinned revision. | Verify exact file grants and record the selected alternative. A project-level OR declaration must not override differently licensed subfiles. |
| Mesa Softpipe/Gallium | Source lock explicitly says “MIT (core/Gallium); component notices required.” | Audit selected compiled files, bundled code, copied headers and generated files. This is not a single verified SPDX expression or blanket clearance. |
| FreeType/libpng via freetype-sys | Hash-verified 0.23.0 archive: MIT wrapper; FreeType LICENSE.TXT offers FTL or GPL-2.0-or-later; libpng LICENSE identifies PNG Reference Library License v2; bundled dlg LICENSE is BSL-1.0. FreeType also lists BDF/PCF, zlib and copied HarfBuzz exceptions. | Propose FTL for applicable FreeType files, preserving unchanged FTL text, source change documentation and binary attribution to the FreeType Team. Record the chosen path after file/build selection review; retain libpng/dlg and relevant subcomponent notices. Do not infer Apache-only or automatic GPL selection from the wrapper. |
| Rust std / rust-src | `nightly-2025-08-01` plus target patch; license metadata missing from sources.lock. | Locate the exact toolchain/source texts and embedded third-party notices. Preserve licenses for the built/redistributed selection. Do not fill this gap with an assumed Rust-wide expression. |
| llama.cpp and whisper.cpp | MIT source pins; generated checkouts and Nagi patch directories. | Audit linked ggml/native components, preserve upstream notices, and distinguish runtime code from model weights. Current Whisper patch boundary is `third_party/whisper-cpp-patches/`. |
| LLVM libc++ | 19.1.7, Apache-2.0 WITH LLVM-exception; linked statically by M20. | Include exact upstream LICENSE.TXT and exception. Keep the exception in the inventory. |
| smoltcp | 0BSD pin. | Confirm selected files and provenance; retain evidence even where attribution is not required. |
| Other pinned Rust adapters | mio, tokio, hyper-util MIT; socket2, tempfile, cc and libc variants MIT OR Apache-2.0. | Retain texts/attribution and changed-file notices where required by the chosen license; check native bundles and actual target inclusion. |
| Transitive Cargo/native/data | Existing notices flag aws-lc, zlib, getrandom, ipc-channel/dpi, webpki certificate data, r-efi, and libfuzzer. | Review all release graphs and bundled native files, not just flagged examples. Respect AND expressions; select an allowed OR branch and record it. r-efi's LGPL alternative need not be selected if an applicable permissive alternative exists. NCSA, Unicode, CDLA and data-specific notices still matter. |
| TTS runtime and dictionary | jbonsai/jpreprocess BSD-3-Clause; dictionary BSD-3-Clause pin and copied notices under `tools/tts/notices/`; [crate inventory](../../tests/m25-tts/CRATE_LICENSES.md). | Check component notices including jbonsai-NOTICE and all dictionary COPYING sections. The crate list is metadata-only and includes historical prose about a first-party license_file that is absent from the current nagi-audio manifest; do not treat it as a Nagi grant. |
| Host/reference tools | Separate Rust locks and embedding Python requirements; QEMU/OVMF/compiler installed on development hosts. | Distinguish use as tools from packaging those tools/firmware. If shipping firmware or a VM bundle containing them, audit their own licenses; Nagi's candidate license does not cover them. |

[MPL 2.0](https://www.mozilla.org/en-US/MPL/2.0/) §§1.10 and 3.1–3.4 define
covered modifications, source/executable obligations, and larger works.
Track file-level obligations rather than claiming that integration erases them.

## Fonts, model weights, voice and data

| Asset | Pin / declared terms | Distribution decision still needed |
| --- | --- | --- |
| Noto Sans Regular/Bold and Noto Sans JP | fonts.lock pins bytes and OFL texts; OFL-1.1; compiled into Servo-enabled init and placed in `/system/fonts/`. | Keep copyright and OFL with each shipped image; verify unmodified status. Do not sell fonts alone or relicense them as Apache. If modifying/subsetting later, review Reserved Font Names and OFL conditions. |
| Desktop bitmap glyph tables | `user/nagi-init/src/font.rs` contains embedded Latin/Japanese glyphs, distinct from Noto. | Maintainer should confirm original design or identify borrowed font sources and required terms. Lack of a font binary or upstream URL does not establish authorship. |
| Granite 4.2 3B Q4_K_M | models.lock: GGUF revision `c40945d71cd90f249a56985e8155551a9188dc30`, Apache-2.0; exact bytes/hash pinned. | Verify the exact quantized artifact's model/base provenance, license text and any actual upstream NOTICE. The generic Apache URL in notice_reference is not evidence that all model notices are collected. Lock acknowledgement flag is application metadata, not proof of a license requirement. |
| Qwen3 4B Q4_K_M | models.lock: revision `bc640142c66e1fdd12af0bd68f40445458f3869b`, Apache-2.0 and immutable LICENSE reference. | Review exact weights/tokenizer/conversion provenance and notices. Guest integrity/readability evidence is not proof of inference or redistribution clearance. |
| Gemma 3 1B IT Q4_K_M | models.lock: `ggml-org` revision `f9c28bcd85737ffc5aef028638d3341d49869c27`; Gemma Terms of Use. No obtained/used weights are claimed by the M26 record. | Separate optional download/bundle decision. Google's terms require downstream restrictions, a copy of terms, modification notices where applicable, and the prescribed Notice for non-hosted distributions. User acknowledgement alone does not satisfy all distributor obligations. Do not label it Apache-2.0 or imply unrestricted OSS model licensing. |
| Whisper small multilingual | models.lock: `5359861c739e955e79d9a303bcbc70fb988958b1`, MIT, 487,601,967-byte ggml artifact. | Check model-specific terms and converted artifact provenance independently of whisper.cpp. Metadata pins and local test artifacts do not mean weights are bundled in the tracked source tree. |
| multilingual-e5-small embedding | [manifest](../../tools/embedding/manifest.toml): `614241f622f53c4eeff9890bdc4f31cfecc418b3`, MIT. Deterministic conversion to `.nemb`; ONNX is reference-only. | Verify model, base, tokenizer and converted-artifact provenance and retain license/changes notices. Integrate evidence into a release inventory later; absent from current models.lock. Reference Python graph is not a guest runtime graph. |
| tohoku-f01 neutral HTS voice | [TTS lock](../../tools/tts/tts-artifacts.lock): `8e3306021db135c265f5eda5f062dc489707ddf8`, CC-BY-4.0, external cached bytes. | Preserve creator credit, license link and modification status; terms remain separate from code. Tracked COPYRIGHT summary and CC-BY legal code provide evidence. Verify packaged voice notices and any audio/demo attribution requirements. |
| naist-jdic binary dictionary | TTS lock: jpreprocess v0.15.0 / `07a78220c1e850a001998451bd1e7c88709ee379`, source dictionary `394af50de2da4a9c73b1bd04fab43e85bb6f03ff`. | Carry the complete [COPYING](../../tools/tts/notices/naist-jdic-COPYING.txt), not only the short README. Check derived binary provenance and every relevant copyright holder. |

Gemma source: [Terms of Use](https://ai.google.dev/gemma/terms), §3;
checked on 2026-10-11. Preserve the applicable terms/version at distribution
review. Model licensing must stay separate from OS licensing.

The [Legal model catalog](../legal/model-assets.json) still marks the LLM
entries fixture-only with opaque placeholder references. It is not synchronized
with the exact artifact pins in models.lock and must not override them.
Embedding and TTS pins live outside that central catalog. Reconcile catalogs
and actual release inclusion in a later scoped change, retaining unknowns
instead of automatically treating declarations as clearance.

## Images and documentation media

The tracked image inventory is **six files**, with no tracked video or model/font
binary found by the extension scan at this baseline:

| Asset | Provenance evidence | Remaining review |
| --- | --- | --- |
| `assets/nagi/nagi_logo_formal.svg` | Existing notices identify custom artwork added in `4dfe8ad`; references system font names without embedding their files. | Confirm original artwork/author and intended artwork/trademark scope. Referencing a font family is not redistributing that font, but it does not establish logo reuse rights. |
| `docs/assets/screenshots/nagi-m10-qemu-desktop.png` | Real QEMU 1280×800 capture, 2026-09-30; status record SHA-256 `061c02741343026c2b6974ae846fbbf26bde48b8e00d0160abe918da2744932e`. | Confirm capture ownership and reuse scope; shows bounded fixed desktop panels. |
| `docs/assets/screenshots/nagi-m18-qemu-browser.png` | Real QEMU 1280×800 capture, 2026-10-01; status SHA-256 `ede8a7967a1634c393aa53252749af22d8d98aa91e4d4711402de0e860c7e097`. | Contains third-party example-site content. Capturing a page does not relicense that content; check terms/attribution for documentation and demo reuse. |
| `docs/assets/screenshots/nagi-m29-settings-ja-jp.png` | Japanese Settings acceptance capture; status records byte identity with a later guest run and checksum manifest. | Confirm author/reuse scope; image does not demonstrate complete localization. |
| `third_party/relibc/openlibm/docs/images/arrow-down.png` and `octocat-small.png` | Imported documentation images, not Nagi artwork. | Verify image-specific permissions and Octocat trademark/reuse terms; an enclosing code license is not sufficient evidence. Resolve for source distributions too; omitting from a binary does not clear an already shipped source archive. |

See the [recording plan](demo-recording-plan.md) for capture/evidence boundaries.
The previous third-party image inventory predates the three QEMU screenshots;
its old “only other tracked images” statement is superseded.

## Open items and release gates

All rows below remain **OPEN**; proposed owners are responsibilities, not
assignments made by this branch.

| Priority / gate | Proposed responsible party | Concrete closure evidence |
| --- | --- | --- |
| Before formal Nagi license grant | Maintainer / rights holders | Written approval of Apache-2.0 scope; authorship/contributor rights review; explicit exceptions and treatment of existing dual-license declarations. |
| Before declaring source release license-ready | Maintainer + dependency reviewer | Exact tracked-file review, complete imported notices (including LGPL tests), and logo/glyph/image provenance decisions. Publish limitations until resolved. |
| Before binary/installer redistribution | Release maintainer + dependency reviewer | Target-specific SBOM, linked native/data review, license texts and attribution packaged with image, corresponding MPL source retrieval instructions tested, Rust/FreeType/Mesa gaps resolved. |
| Before bundling any weights/voice/data | Model/data reviewer | Exact artifact provenance + hashes + license/notice copies; Gemma distribution plan approved separately or artifact excluded; consistent model inventory. |
| Before OSS application claims | Applicant maintainer | License grant approved/applied in a separate change before describing Nagi as licensed OSS; truthful scope and maintenance evidence; current application terms reviewed. |

After explicit license approval, a separate implementation/documentation change
can add the authoritative root LICENSE, identify copyright holders, add
applicable headers/manifest fields without overwriting upstream grants,
produce a reviewed NOTICE, and update README wording. **None of these
license-applying steps is authorized by this document.**

For a future release audit, follow [Legal's setup](../legal/README.md), then
use scan/check/SBOM/NOTICE as evidence generation. Review outputs manually:
unknown expressions, fixture-only model entries, or a passing tool check are
not permission to redistribute. Keep generated candidate outputs outside the
tracked tree until reviewed, and preserve the tool's standalone lockfile.
