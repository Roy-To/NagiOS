# Apache-2.0 license proposal

Status: **proposal only; no approval or license grant**

Review date: **2026-10-14**

Baseline: `bcb92dcf38d195498056f50a72f7d5ca12b0ea21`

> 日本語要約: Apache License 2.0をNagi独自の対象コード／文書の候補として提案します。採用は未決定です。Apache-2.0を選んでも第三者コード、フォント、モデル、音声データ、画像は各自の条件のままです。正式承認後の別変更までルート`LICENSE`やライセンス表示を追加しません。

## Candidate scope

The candidate is Apache License 2.0 for material that Nagi's rights holders
can license: original Nagi code and documentation, plus separately verified
Nagi-authored adapters and patches where the relevant rights are held. The
scope would need an explicit path/file policy and contributor-rights review.
It must exclude third-party source, copied/generated material without proven
rights, and non-code assets unless separately cleared.

This proposal does not apply Apache-2.0 to the entire repository, relicense
any upstream component, resolve existing manifest declarations, change
third-party notices, create a patent opinion, or claim the current repository
is licensed open source. No root `LICENSE` is added. The current README and
the more detailed [license audit](LICENSE_AUDIT.md) remain the accurate public
status.

## Why consider Apache-2.0

Apache-2.0 is a widely used permissive software license with explicit
copyright and contributor patent grants, conditions for redistribution, and
a mechanism for preserving relevant upstream `NOTICE` material. For an OS
project that combines an original kernel/user-space stack with separately
licensed ports, those features are useful if Nagi's contributors and
rights-holders can make the grants and if release notices are maintained.

The patent grant in §3 covers only patent claims each contributor can license
that are necessarily infringed by that contributor's contribution alone or
by the contribution combined with the work it was submitted to. The section
terminates the patent grants for a licensee that institutes patent litigation
against any entity alleging that the Work or a contribution incorporated in it
infringes. It does not prove ownership, clear third-party patents, or grant
patent rights held by non-contributors.
Apache-2.0 also does not grant trademark rights. See the
[authoritative license text](https://www.apache.org/licenses/LICENSE-2.0).

## Compatibility review

This is a source-layout and obligation review, not a legal opinion that every
target configuration is cleared.

| Component | Relationship to an Apache-2.0 Nagi license | Conditions and decision status |
| --- | --- | --- |
| Servo / Stylo — MPL-2.0 | An Apache-licensed larger work can contain separately licensed MPL files if the MPL-covered files remain under MPL and the MPL conditions are met. Apache does not replace MPL for those files. | Preserve the original license/notices, identify covered modifications, provide corresponding source for executable distribution, and tell recipients how to get it. Inspect pinned Servo/Stylo source and patches. This interpretation follows the MPL larger-work and distribution clauses and needs counsel review for the actual static-link/build boundary. |
| SpiderMonkey (`mozjs-sys-nagi`) — MPL-2.0 | Same file-level treatment as Servo. | Keep MPL text and covered-source path; inspect linked files and patch output. No Apache-only relabeling. |
| Mesa Softpipe/Gallium — mostly MIT, file/component exceptions | MIT-licensed selected files can remain under MIT notices alongside a separately Apache-licensed Nagi work. The Mesa project warns that individual files/components may differ. | Verify every compiled file, copied header, generated file, and component license. The source-lock summary is not a file-level clearance. |
| Rust standard library — generally MIT OR Apache-2.0 | Rust's standard library is generally available under either MIT or Apache-2.0, unless a file/project says otherwise. | Resolve the exact pinned `rust-src` contents, target patch, embedded third-party licenses, and shipped subset; retain all applicable texts. Do not assume that every bundled standard-library dependency uses the same expression. |
| `llama.cpp` / `ggml` — MIT | MIT components can coexist with Apache-licensed Nagi code when their required notices remain. | The pinned runtime license is not a license for its bundled dependencies or the GGUF weights. Review native and bundled subcomponents. |
| MIT / BSD / ISC / 0BSD crates and code | Generally can be distributed as separately licensed components in an Apache-licensed project. | Include required copyright/license text and notices; preserve attribution and disclaimer terms. Resolve `OR` choices and satisfy every part of `AND` expressions. |
| LLVM libc++ — Apache-2.0 WITH LLVM-exception | The exception modifies the applicable Apache terms for that component. | Preserve the exact exception and upstream text; do not simplify the declaration to plain Apache-2.0. |
| OpenLibm LGPL test sources | They are not made Apache-2.0 by association with Nagi or relibc. | Confirm source archive and binary inclusion. If distributed, preserve the LGPL terms and meet source obligations; otherwise exclude unused test files from a release archive only after the source-release scope is decided. |
| FreeType / libpng / bundled `dlg` | The `freetype-sys` package declaration does not settle nested licenses. FreeType offers FTL or GPL routes; libpng and `dlg` have their own terms. | Confirm the selected files and applicable FTL/GPL path, attribution, and subcomponent notices before binary release. Legal review required. |
| Fonts, models, voice, dictionaries, and images | These are data/media assets with their own terms, not automatically part of a code license. | Maintain OFL, Gemma Terms, CC-BY, BSD dictionary, and other asset conditions separately. Clear artwork, screenshot contents, and unknown glyph/image origins. |

References: [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0),
[MPL-2.0](https://www.mozilla.org/en-US/MPL/2.0/),
[Mesa licensing](https://docs.mesa3d.org/license.html),
[Rust license policy](https://rust-lang.org/policies/licenses/), and the
[pinned llama.cpp license](https://github.com/ggml-org/llama.cpp/blob/c85b92c69c955961621193cd51da194f3cbcedf3/LICENSE). The coexistence statements
above are a reading of the component license terms; they are not an external
compatibility certification.

## Model terms are separate from OS terms

The current model lock declares Apache-2.0 for the pinned Granite 4.2 3B and
Qwen3 4B GGUF artifacts, and Gemma Terms of Use for Gemma 3 1B. Confirm the
immutable artifact revision, base-model relationship, quantization/conversion,
tokenizer, license text, and notices before redistribution. Gemma's terms
include restrictions and prescribe a Notice for non-hosted distribution, so
it cannot be represented as an Apache-2.0 model. The model catalog is not
current evidence of actual image inclusion.

The system fonts are OFL-1.1; the Tohoku voice is CC-BY-4.0; the NAIST
dictionary and TTS crates declare BSD-3-Clause; the Whisper model declares
MIT. Keep their attribution and terms with the particular artifacts. See the
[license audit](LICENSE_AUDIT.md) for the artifact references and open checks.

## Patent and contributor review required

Before choosing Apache-2.0, the maintainer and legal reviewer should confirm:

1. The people or entities proposing the license own or can license the
   relevant Nagi contributions, including contributions made as employees or
   contractors.
2. The contribution process makes the intended copyright and patent grants
   clear and consistently records provenance. A repository setting or a
   contributor's act of submitting code is not a substitute for checking
   applicable agreements and authority.
3. Any existing patent promises, grants, assignments, or commitments are
   identified, and Apache-2.0's patent-termination clause is understood.
4. No one treats the Apache patent grant as a patent search, freedom-to-operate
   opinion, or clearance for third-party code and hardware interfaces.
5. The Nagi name and logo are reviewed separately because the license does
   not grant trademark rights.

These are legal and rights-holder questions; keep them open until confirmed.

## If approved later

Only after explicit approval by the rights holders should a separate change:

1. define the exact Apache-licensed Nagi path/file scope and exceptions;
2. add the authoritative Apache-2.0 text as the root `LICENSE` and update
   project license wording;
3. resolve the existing four package declarations without silently revoking
   or broadening their prior scope;
4. decide whether a reviewed root `NOTICE` is needed and preserve upstream
   notices without overwriting them;
5. update contribution documentation and any package metadata only for the
   files that are approved for that license; and
6. keep third-party, model, font, data, image, and trademark terms separate.

Until that separate approval and implementation, this file remains a
proposal. It grants no rights.
