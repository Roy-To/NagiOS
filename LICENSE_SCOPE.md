# Nagi OS source-license scope

Effective in repository revisions containing this notice and the accompanying
[LICENSE](LICENSE). Scope decision recorded 2026-10-27.

## Operative source notice

The maintainer licenses the original Nagi OS source code described below under
**Mozilla Public License 2.0 (MPL-2.0)**, to the extent the maintainer holds the
rights to do so. This is a source-code license grant, not merely a proposed
license. The unmodified license text is in `LICENSE`.

For the included source, after applying the exceptions below, the MPL Exhibit A
notice is attached here alongside the root LICENSE:

> This Source Code Form is subject to the terms of the Mozilla Public
> License, v. 2.0. If a copy of the MPL was not distributed with this
> file, You can obtain one at https://mozilla.org/MPL/2.0/.

This notice identifies the source to which that license applies. It does not
modify MPL-2.0, add a non-commercial condition, revoke a previous grant, grant
someone else's rights, or certify that a complete OS image can be redistributed.

## Included source boundaries

Subject to the exceptions below, the grant covers the original Nagi software
implementation, interface definitions, build scripts and software configuration
under these repository-relative directories, recursively:

```text
apps/
crates/
idl/
kernel/
loader/
samples/
schemas/
sdk/
services/
targets/
tests/
tools/
user/
```

It also covers the original Nagi build/configuration files `Cargo.toml`, `nagi`,
`nagi.ps1`, `nagi.toml` and `rust-toolchain.toml`, and original build/workflow
configuration under `.cargo/` and `.github/`.

The grant is for source code and its software configuration, not every item
stored inside those directories. A path does not establish authorship.
Dependencies named in a manifest or lockfile do not acquire Nagi's license.
Generated source follows the rights in its inputs and templates; generation is
not proof that it is an original Nagi work.

This root notice accompanies the MPL text instead of mass-editing active source
files. When creating or independently distributing an in-scope source file, keep
its applicable notice; new original MPL-covered files should include
`SPDX-License-Identifier: MPL-2.0` in a suitable comment. Preserve this scope
notice and the license when distributing the source tree.

## Exceptions take precedence

| File or boundary | Treatment by this grant |
| --- | --- |
| `tools/nagi-cli/**` | Existing `MIT OR Apache-2.0` package declaration retained; not relicensed by this change. |
| `tools/legal/**` | Existing `MIT OR Apache-2.0` package declaration retained; not relicensed by this change. |
| `crates/nagi-embedding-provider/**` | Existing `MIT OR Apache-2.0` package declaration retained; not relicensed by this change. |
| `crates/nagi-tts-provider/**` | Existing `MIT OR Apache-2.0` package declaration retained; not relicensed by this change. |
| `tests/m24-embedding/**`, `tests/m25-tts/**` | No new grant in this change. Preserve any existing grants; package-linked tests outside the four package directories need separate scope review. |
| `user/nagi-init/src/font.rs` | **No new license grant.** Embedded glyph provenance remains unverified. This exclusion is not permission to redistribute it. |
| `third_party/**`, including Nagi patches/adapters inside imported upstream trees | Outside this grant. Existing upstream licenses, covered modifications and notices remain applicable. |
| `tools/tts/notices/**` and any other imported license, notice, attribution or third-party source material, wherever located | Preserve the applicable original terms; never replace them with MPL by directory inference. |
| Any file or portion already covered by a specific license/header/package declaration | Existing rights and conditions are preserved. This default does not silently replace or revoke them. |
| Model weights, tokenizer/model data, fonts, voice/dictionary data, firmware, images, screenshots, recordings, binary/test-data assets | Not relicensed. Their individual terms and provenance must be reviewed. |
| Non-code documentation, branding and `.dev/` records | Not automatically licensed by this OS-code decision. Existing permissions, if any, remain unchanged. |

All exclusions are exclusions from **this new grant**, not claims that the
excluded material has no license. Previously granted rights remain available
under their applicable terms. Where ownership or applicable terms are unclear,
record the uncertainty; do not invent permission from repository visibility.

The ordinary MPL-2.0 secondary-license provisions remain unchanged. Merely
including Exhibit B in the complete license text does not opt this code into
that exhibit. Nagi names and logos receive no trademark permission from this
source license.

## Models and executable distributions

Granite and Qwen artifacts are recorded as Apache-2.0 in
[`third_party/models.lock`](third_party/models.lock). Gemma has separate terms.
Those records are not a certification of artifact provenance or bundling rights.
This change neither downloads nor republishes model weights.

For an executable distribution containing MPL-covered source, provide the
corresponding covered source and tell recipients how to obtain it, as MPL
Section 3.2 requires. Preserve all other applicable licenses and notices.
Unresolved glyph/media provenance, imported-source notice gaps, model terms and
target-specific dependency/SBOM checks remain release work. **The complete Nagi
image and every file in this repository are not claimed to be license-cleared.**

See [the review record](docs/oss/mpl-2.0-scope-review.md) for evidence, limitations
and open items. Historical statements that the Nagi license was unselected are
superseded only for the source grant expressly made here; historical dependency
and provenance findings are not marked resolved by this notice.
