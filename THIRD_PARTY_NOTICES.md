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

| Component | Inclusion | Current source/pin | Declared metadata | Nagi boundary | Notice / redistribution status |
| --- | --- | --- | --- | --- | --- |
| relibc | A + B | `69bb008af1f6d93758631cf0df250500d53a065b` | MIT | `third_party/relibc`, `third_party/relibc/src/nagi.rs` | Upstream license files are tracked; verify the complete notice set for a binary release |
| libc | A + B | `0.2.174`, locked archive hash | MIT OR Apache-2.0 | `third_party/libc`, `third_party/libc/src/unix/nagi.rs` | Vendored license files are tracked; verify the complete notice set for a binary release |
| Rust std | B + C | `nightly-2025-08-01` plus Nagi patch | Upstream redistribution metadata is not complete in the lock | `third_party/rust-std/patches/` | Human review required for patched rust-src redistribution and notices |
| smoltcp | B | revision `d2d647090d544b1e7c142571da9d55f7280f664b` | 0BSD | no Nagi patch | Verify upstream notice and binary redistribution obligations |
| Servo | B + C | `b820a9679a784877f91b4acc90c2c6e849f18d3b` | MPL-2.0 in the source lock | `third_party/servo-patches/` | Source is fetched/generated at build time; verify upstream notices and MPL boundary |
| Surfman | B + C | `205778f497327c573929c7b471194390e15f331d` | MIT OR Apache-2.0 OR MPL-2.0 | `third_party/surfman-patches/` | Source is fetched/generated at build time; verify upstream notices |
| Mesa Softpipe | B + C | `f1f246cfda65eff82fba3be1caf2d23bdeda60cc` | MIT core/Gallium; component notices required | `third_party/mesa-patches/` | Softpipe/Gallium and copied-header notices require human review before binary redistribution |
| freetype-sys | B + C | `0.23.0`, locked archive hash | MIT | `third_party/freetype-sys-patches/` | Source is fetched/generated at build time; verify bundled source notice |
| mio | B + C | `1.2.3`, locked archive hash | MIT | `third_party/mio-servo-patches/` | Source is fetched/generated at build time; verify upstream notice |
| socket2 | B + C | `0.6.5`, locked archive hash | MIT OR Apache-2.0 | `third_party/socket2-servo-patches/` | Source is fetched/generated at build time; verify upstream notice |
| Tokio | B + C | `1.53.1`, locked archive hash | MIT | `third_party/tokio-servo-patches/` | Source is fetched/generated at build time; verify upstream notice |
| hyper-util | B + C | `0.1.20`, locked archive hash | MIT | `third_party/hyper-util-servo-patches/` | Source is fetched/generated at build time; verify upstream notice |

## Cargo.lock transitive and native-source review

The following dependencies are present in the locked Cargo graph but are not
separate entries in `third_party/sources.lock`. They remain part of the
binary-distribution review even though they do not block publication of this
source-only snapshot:

| Dependency | Locked version(s) observed | Inclusion | Review status |
| --- | --- | --- | --- |
| aws-lc-rs / aws-lc-sys | `1.18.1` / `0.45.0` | D | Review upstream license, notices, native bundled source, and redistribution boundary |
| libz-sys | `1.1.29` | D | Review upstream license, notices, native bundled source, and redistribution boundary |
| getrandom | `0.2.17`, `0.3.4`, `0.4.3` | D | Review each locked registry package and its platform-specific source/notice requirements |
| ipc-channel and dpi | locked Cargo graph | D | Include in the complete Cargo dependency notice review before binary distribution |

These entries are intentionally explicit rather than silently treated as
covered by the direct Servo or Mesa entries. The source lock and Cargo.lock
must continue to be reviewed together when the dependency graph changes.

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
