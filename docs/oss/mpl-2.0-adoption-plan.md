# Nagi OS — MPL-2.0 adoption decision and clearance plan

**Decision date:** 2026-10-26  
**Status:** Maintainer-selected licensing policy; **file-level application and redistribution clearance pending**  
**Scope:** Nagi-owned first-party source code only, after ownership/provenance review  
**Not a license grant:** This document does not license any source file or asset. The repository currently has no root `LICENSE` granting a project-wide license.

## Decision

The maintainer has selected **Mozilla Public License 2.0 (MPL-2.0)** as the intended license for **eligible Nagi-owned OS source code**. This supersedes Apache-2.0 as the proposed default for Nagi's own code; it does **not** replace the license of a third-party component, a model, an asset, or a prior grant.

The purpose is to allow OSS use (including commercial use) while requiring distribution of modifications to MPL-covered **files** under MPL-2.0. Separate files in a larger work may have different licenses. MPL-2.0 is **not** a non-commercial restriction and does not prevent a company from selling products built using Nagi. The owner may pursue separate commercial arrangements only for rights they actually control.

Authoritative references: [MPL-2.0 license](https://www.mozilla.org/en-US/MPL/2.0/) and [Mozilla's FAQ](https://www.mozilla.org/en-US/MPL/2.0/FAQ/). For rights or license compatibility uncertainties, obtain qualified legal review rather than automatically declaring clearance.

## Component-by-component boundary

| Material | Intended or existing treatment | Required action |
| --- | --- | --- |
| Original first-party Nagi OS source | **MPL-2.0**, once authorship and licensing authority are confirmed for each covered file | Identify the source file set, contributors and any prior/contractual rights |
| First-party packages already declaring `MIT OR Apache-2.0` | Preserve existing declarations and already granted rights until separately reviewed | Determine ownership and what future versions may be offered under MPL; do not rewrite metadata automatically |
| Servo, SpiderMonkey and other upstream MPL-covered material | Preserve the upstream MPL-2.0 terms and notices | Audit covered modifications and source-availability obligations |
| Other third-party software, including relibc, Mesa, FreeType, LLVM and native dependencies | Preserve each upstream license, exception and attribution | Use exact dependency/source revision and linked release inventory |
| IBM Granite 4.2 3B and Qwen3 4B model artifacts | Recorded as **Apache-2.0** in `third_party/models.lock` | Verify exact weights/quantization provenance, terms and notices before bundling |
| Google Gemma 3 1B | **Gemma Terms of Use**, not Apache-2.0 or MPL-2.0 | Review separate distribution terms or omit the artifact |
| Whisper, embedding, voice, dictionaries, fonts and images | Their individual recorded licenses and permissions | Audit exact content and distribution notices; do not infer a blanket Nagi grant |
| Nagi name, logo, project documentation and screenshots | **Not automatically relicensed by this code decision** | Record provenance and decide branding/media/documentation terms separately |

`third_party/models.lock` pins model **metadata**; it does not certify that weights are bundled, accepted on target, or cleared for redistribution.

## Application gates

The following must be completed **before** adding an operative root `LICENSE`, mass SPDX headers, or broad project-wide licensing statements:

1. **Provenance and ownership:** Review which first-party files can be licensed by the maintainer, contributions from others, imported/adapted sources, employment or contractual IP obligations where relevant, and any pre-existing grants. Record exceptions at file or component level.
2. **Scope matrix:** Enumerate paths to receive MPL-2.0, paths already under upstream licenses, dual-licensed Nagi crates, generated code, docs, images, fonts and model artifacts. Do not treat a directory name as proof of copyright ownership.
3. **Prior grants:** Do not purport to revoke existing MIT/Apache permissions for earlier distributions. For any future license change, verify the holders' authority and document the exact version/file scope.
4. **Notices and source obligations:** Retain upstream copyright/license texts, file-level notices, modified-file declarations, and required MPL covered-source availability for distributed executables.
5. **Release-specific review:** Produce an accurate source/binary SBOM and attribution/NOTICE candidate for the exact image, firmware, models and other artifacts actually shipped; keep unknowns explicit.
6. **Contributor policy:** Decide how future contributions will be licensed, whether any contribution agreement is needed for a future commercial dual-license plan, and how trademark/branding use will be handled. Do not imply contributor consent retroactively.
7. **Separate approval change:** Once the scope and owners are verified, prepare a *separate reviewed PR* with the exact MPL-2.0 legal text at the agreed location, appropriate `SPDX-License-Identifier: MPL-2.0` headers or Exhibit A notices on eligible source files, scoped manifests, notices and documentation changes.
8. **Publication check:** Only after that PR is approved and merged should the README or an OSS support application claim that Nagi's eligible source is actually distributed under MPL-2.0.

## Existing evidence and current blockers

- The [2026-10-11 Apache-2.0 readiness review](apache-2.0-readiness.md) remains useful as **historical component/provenance evidence**, but its *project license recommendation* is superseded by this decision.
- [Third-party notices](../../THIRD_PARTY_NOTICES.md) and [Legal/SBOM tooling](../legal/README.md) are existing evidence sources, not automatic clearance.
- The review identifies unresolved issues involving first-party authorship and some prior dual-license declarations, MPL upstream modifications, relibc/OpenLibm, FreeType, Rust/Mesa, font/image provenance and model-specific notices. None is marked cleared by this policy document.
- Nagi 0.1 implementation, acceptance, existing workstreams, build/runtime code and official milestones remain unchanged.

## Current completion state

- [x] Maintainer selected MPL-2.0 for eligible Nagi-owned OS code
- [x] Third-party software and models retain their own licenses
- [ ] Ownership and file-level scope verified
- [ ] Release-specific dependency, model and asset clearance complete
- [ ] Formal MPL-2.0 `LICENSE`/headers/notices applied through separate reviewed PR
- [ ] Public OSS-license claim and support-program application updated following application

**Practical meaning:** Nagi's source is publicly visible, but the project must not yet be described as already having an applied, project-wide MPL-2.0 license.
