# Third-Party Notices

Nagi OS fetches, vendors, or builds against third-party components. This file
is a release-preparation inventory, not a replacement for the license and
notice files distributed by each upstream project. The authoritative Nagi
source pins and patch boundaries are recorded in
[`third_party/sources.lock`](third_party/sources.lock).

The inventory uses these inclusion classifications:

- **A — vendored source:** source is tracked in this repository.
- **B — reproducibly fetched source:** the snapshot tracks an exact revision,
  archive hash, or toolchain plus the Nagi patch boundary; fetched source is
  materialized by the documented bootstrap flow.
- **C — patch or integration boundary:** only Nagi-owned changes or integration
  metadata are tracked here; upstream source is fetched at build time.
- **D — lockfile/transitive dependency:** identified from `Cargo.lock` but not
  separately represented in `third_party/sources.lock`.

## Pinned components

This table covers every component, version/revision/toolchain pin, and declared
license expression in `third_party/sources.lock`. A `nagi-cli` unit test checks
that those fields remain represented here; rust-std is intentionally identified
as missing license metadata rather than assigned an unverified value.

| Component | Inclusion | Current source/pin | Declared metadata | Nagi boundary | Notice / redistribution status |
| --- | --- | --- | --- | --- | --- |
| relibc | A + B | `69bb008af1f6d93758631cf0df250500d53a065b` | MIT | `third_party/relibc`, `third_party/relibc/src/nagi.rs` | Upstream license files are tracked; verify the complete notice set for a binary release |
| libc | A + B | `0.2.174`, locked archive hash | MIT OR Apache-2.0 | `third_party/libc`, `third_party/libc/src/unix/nagi.rs` | Vendored license files are tracked; verify the complete notice set for a binary release |
| libc-nagi-servo | B + C | `0.2.189`, locked archive hash | MIT OR Apache-2.0 | `third_party/libc-servo`, `third_party/libc-servo-patches/` | Generated checkout contains both license texts; verify notice preservation for a binary release |
| rust-std (Rust std) | B + C | `nightly-2025-08-01` plus Nagi patch | Upstream redistribution metadata is not complete in the lock | `third_party/rust-std/patches/` | Human review required for patched rust-src redistribution and notices |
| smoltcp | B | `0.12.0`, revision `d2d647090d544b1e7c142571da9d55f7280f664b` | 0BSD | no Nagi patch | Verify upstream notice and binary redistribution obligations |
| Servo | B + C | `b820a9679a784877f91b4acc90c2c6e849f18d3b` | MPL-2.0 in the source lock | `third_party/servo-patches/` | Source is fetched/generated at build time; verify upstream notices and MPL boundary |
| Surfman (`surfman-nagi-mesa-surfaceless`) | B + C | `205778f497327c573929c7b471194390e15f331d` | MIT OR Apache-2.0 OR MPL-2.0 | `third_party/surfman-patches/` | Source is fetched/generated at build time; verify upstream notices |
| mesa-softpipe (Mesa Softpipe) | B + C | `f1f246cfda65eff82fba3be1caf2d23bdeda60cc` | MIT (core/Gallium); component notices required | `third_party/mesa-patches/` | Softpipe/Gallium and copied-header notices require human review before binary redistribution |
| freetype-sys | B + C | `0.23.0`, locked archive hash | MIT | `third_party/freetype-sys-patches/` | Source is fetched/generated at build time; verify bundled source notice |
| mio-servo | B + C | `1.2.3`, locked archive hash | MIT | `third_party/mio-servo-patches/` | Source is fetched/generated at build time; verify upstream notice |
| socket2-servo | B + C | `0.6.5`, locked archive hash | MIT OR Apache-2.0 | `third_party/socket2-servo-patches/` | Source is fetched/generated at build time; verify upstream notice |
| tokio-servo | B + C | `1.53.1`, locked archive hash | MIT | `third_party/tokio-servo-patches/` | Source is fetched/generated at build time; verify upstream notice |
| hyper-util-servo | B + C | `0.1.20`, locked archive hash | MIT | `third_party/hyper-util-servo-patches/` | Source is fetched/generated at build time; verify upstream notice |
| tempfile-nagi | B + C | `3.27.0`, locked archive hash | MIT OR Apache-2.0 | `third_party/tempfile-nagi`, `third_party/tempfile-nagi-patches/` | Generated checkout contains both license texts; verify notice preservation for a binary release |
| mozjs-sys-nagi | B + C | `153.0.0-2`, locked archive hash | MPL-2.0 | `third_party/mozjs-sys-nagi-patches/` | Generated source includes `third_party/mozjs-sys-nagi/mozjs/LICENSE`; review the exact linked-source boundary before redistribution |
| cc-nagi | B + C | `1.4.6`, locked archive hash | MIT OR Apache-2.0 | `third_party/cc-nagi`, `third_party/cc-nagi-patches/` | Generated checkout contains both license texts; verify notice preservation for a binary release |
| inventory-nagi | A + B + C | `0.3.24`, locked archive hash | MIT OR Apache-2.0 | `third_party/inventory-nagi`, `third_party/inventory-nagi-patches/` | Tracked source includes both license texts; verify notice preservation for a binary release |
| llama.cpp-ggml-cpu | B + C | revision `c85b92c69c955961621193cd51da194f3cbcedf3` | MIT | `third_party/llama.cpp`, `third_party/llama-cpp-patches/` | Generated checkout includes the upstream license and component-license directory; review linked components before binary redistribution |
| whisper.cpp-stt | B | revision `927cfce34f31707e17f2bff35c349632fb9e2c3a` | MIT | `third_party/whisper.cpp` | Source pin is fetched by `./nagi fetch`; review linked-source notices before binary redistribution |
| llvm-libcxx (LLVM libc++) | B + C | `19.1.7`, release tarball SHA-256 | Apache-2.0 WITH LLVM-exception | no Nagi patch; built by `tools/libcxx/build-nagi-target.sh` | Source is fetched by `./nagi m20-granite-inference`; the static `libc++.a` linked into M20 guest images carries the LLVM exception; preserve `libcxx/LICENSE.TXT` with any binary redistribution |

## External model artifact pins

`third_party/models.lock` pins the Whisper small multilingual artifact at
487,601,967 bytes with SHA-256
`1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b` and
records the model repository's MIT declaration at an immutable revision. Model
bytes are not fetched by `./nagi fetch`, stored in the source checkout, or
included in a Nagi image by this metadata-only step. The Model Store uses the
opaque artifact ID in the lock; it must verify the size and SHA-256 before a
future STT backend can load the bytes. Keep model licensing distinct from the
whisper.cpp source license and review both before redistribution.

## Cargo.lock transitive and native-source review

The following dependencies are present in the locked Cargo graph but are not
separate entries in `third_party/sources.lock`. They remain part of the
binary-distribution review even though they do not block publication of this
source-only snapshot:

| Dependency | Locked version(s) observed | Inclusion | Review status |
| --- | --- | --- | --- |
| aws-lc-rs / aws-lc-sys | `1.18.1` / `0.45.0` | D | Cargo metadata declares `ISC AND (Apache-2.0 OR ISC)` for aws-lc-rs and `ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0)` for aws-lc-sys; review bundled native source, full notices, and redistribution boundary |
| libz-sys | `1.1.29` | D | Cargo metadata declares MIT OR Apache-2.0; review bundled native source, full notices, and redistribution boundary |
| getrandom | `0.2.17`, `0.3.4`, `0.4.3` | D | Each Cargo metadata entry declares MIT OR Apache-2.0; review platform-specific source and notice requirements |
| ipc-channel / dpi | `0.23.0` / `0.1.2` | D | Cargo metadata declares MIT OR Apache-2.0 for ipc-channel and Apache-2.0 AND MIT for dpi; include both in the complete binary notice review |
| webpki-roots / webpki-root-certs | `1.0.9` / `1.0.9` | D | Both declare CDLA-Permissive-2.0; review root-certificate data notices and provenance in the selected target graph |
| r-efi | `5.3.0`, `6.0.0` | D | Declares MIT OR Apache-2.0 OR LGPL-2.1-or-later; determine the selected target dependency and preserve applicable notices |
| libfuzzer-sys | `0.4.13` | D | Declares (MIT OR Apache-2.0) AND NCSA; appears in the locked workspace graph, including development tooling, and must be checked against the release dependency graph |

These entries are intentionally explicit rather than silently treated as
covered by the direct Servo or Mesa entries. The source lock and Cargo.lock
must continue to be reviewed together when the dependency graph changes.

### Cargo metadata declaration audit — 2026-09-30

`python3 tools/audit_cargo_license_metadata.py` runs
`cargo metadata --locked --format-version 1` and reports that all 673 external
packages in the current Cargo graph declare a license expression (41 distinct
expressions). This graph includes dev and target-specific packages and is not a
bill of materials for the Nagi image. The command uses Cargo's locked graph and does not change
`Cargo.lock`; it may contact the configured registry if a package is not cached.
This checks publisher-supplied package metadata only. It does not inspect
license text, validate the declarations, enumerate all bundled native source,
or establish binary redistribution permission.

## Bundled system fonts

`third_party/fonts.lock` pins the fonts compiled into the Servo-enabled Nagi
init image and published read-only under `/system/fonts/` (ADR 0055).
`./nagi fetch` downloads them into the ignored `out/cache/fonts/` cache and
rejects any file whose size or SHA-256 differs from the lock. The OFL license
texts are pinned the same way and shipped beside the fonts in the image.

| Component | Files | Source revision | License |
| --- | --- | --- | --- |
| noto-sans (Noto Sans) | `NotoSans-Regular.ttf`, `NotoSans-Bold.ttf`, `OFL-NotoSans.txt` | `notofonts/notofonts.github.io` `86eb2ddc3a2e97cb9747fd9069ee5d47880e3305` | OFL-1.1 |
| noto-sans-jp (Noto Sans JP) | `NotoSansJP-Regular.otf` (JP region subset), `OFL-NotoSansCJK.txt` | `notofonts/noto-cjk` `f8d157532fbfaeda587e826d4cd5b21a49186f7c` | OFL-1.1 |

The fonts are redistributed unmodified with their license texts, as the SIL
Open Font License 1.1 permits; they are not sold by themselves.

## Project asset provenance

| Asset | Classification | Repository evidence | Follow-up |
| --- | --- | --- | --- |
| `assets/nagi/nagi_logo_formal.svg` | Project-original, maintainer confirmation required | Added by commit `4dfe8ad` as custom NAGI artwork; no upstream URL, external source, or bundled font file is present in the tracked asset | Confirm original-artwork provenance before a binary or branded release |
| `third_party/relibc/openlibm/docs/images/arrow-down.png` | Third-party documentation asset | Tracked with the relibc/openlibm import in commit `6fe1559`; it is not Nagi artwork | Preserve the applicable relibc/openlibm notice or remove the documentation asset before binary redistribution |
| `third_party/relibc/openlibm/docs/images/octocat-small.png` | Third-party documentation asset | Tracked with the relibc/openlibm import in commit `6fe1559`; it is not Nagi artwork | Preserve the applicable relibc/openlibm notice or remove the documentation asset before binary redistribution |

The SVG references common system font family names (`Segoe UI`, `Helvetica
Neue`, and `Arial`) for text rendering; no font files are redistributed by this
repository. The two PNGs above are the only other tracked image assets found
in the audited snapshot, and neither is used as Nagi branding.

## Source-only snapshot versus binary distribution

This public-snapshot preparation covers the tracked source tree, its exact
source and patch metadata, and the Nagi-owned integration boundaries. It does
not claim that a compiled Nagi image or installer is ready for redistribution.
Generated Servo, Mesa, Surfman, Rust std, and related sources are fetched at
build time from the pins recorded in `third_party/sources.lock` and are not
silently treated as part of this tracked snapshot.

Before distributing a binary, image, installer, or bundled third-party source,
re-run the complete dependency and asset notice review, include all required
license texts and attribution, and resolve the Rust std, Mesa component,
native-source, and asset follow-ups above.

The license for Nagi OS itself has not been selected. Nothing in this file
grants a license for Nagi OS or permission to redistribute a third-party
component beyond its own applicable terms.
