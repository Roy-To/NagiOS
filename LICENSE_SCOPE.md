# Proposed Nagi OS license scope — review required

**Status:** Review-only draft. Do not merge the root `LICENSE` solely because the text is available.  
**Decision:** MPL-2.0 for eligible **Nagi-owned OS source code**, after source-specific rights verification.  
**Reference:** [MPL-2.0 adoption plan](docs/oss/mpl-2.0-adoption-plan.md) (the documentation-policy PR must merge first or this link will initially point to an absent path).

The root `LICENSE` in this proposed change contains the unmodified MPL-2.0 legal text sourced from the SPDX license list. It is **not** an assertion that the repository's entire tree is Nagi-owned, that all files are covered, or that redistribution rights for the entire release are cleared.

## Required before merging this licensing PR

- [ ] Identify the copyright holder(s) and confirm authority to grant the MPL-2.0 license for the exact first-party source files.
- [ ] Identify the intended MPL-covered files and attach the official Exhibit A notice (or an appropriately located notice per Exhibit A), with `SPDX-License-Identifier: MPL-2.0` where appropriate.
- [ ] Preserve **existing** `MIT OR Apache-2.0` or other source grants unless the rights holder authorizes a documented subsequent licensing arrangement. This license file must not be used to revoke earlier grants.
- [ ] Exclude or preserve separately licensed third-party source (including Servo/SpiderMonkey, relibc, Mesa, FreeType, LLVM and pinned dependencies), generated upstream code, and notices.
- [ ] Preserve separate model licenses: pinned IBM Granite and Qwen artifacts are recorded as Apache-2.0; Gemma follows its own Google terms. Model weights, fonts, voice data, screenshots, and Nagi branding are **not** implicitly MPL-covered.
- [ ] Confirm the required MPL-covered source availability and attribution procedures for any distributed binaries, plus release-specific SBOM and unresolved license inventory.
- [ ] Update README.md, README.ja.md, CONTRIBUTING.md, OSS application materials and the adoption plan *together* when effective licensing is actually approved. Do not claim that a staged text alone makes all of Nagi OSS.

A named author's permission cannot be inferred from the repository being public or from the fact that a file exists. Unverified provenance and third-party redistribution obligations must be reviewed as part of the release process.

For authoritative MPL definitions and requirements see <https://www.mozilla.org/en-US/MPL/2.0/> (including Sections 1.4, 2 and 3 and Exhibit A).
