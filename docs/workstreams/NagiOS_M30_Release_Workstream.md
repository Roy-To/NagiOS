# Nagi OS M30 — Release

**Status: PARTIAL.** The self-contained reference disk is now built and has
passed a real System A QEMU boot and User Data persistence check. M30 is not
complete and this evidence does not assert release readiness.

## Implemented

- `tools/nagi-release/release.py` validates required release inputs, a clean
  Git source revision, an x86-64 kernel ELF, pinned source revisions, the
  upstream Granite digest, and a self-contained qcow2 image at the configured
  reference disk size.
- Assembly copies the qcow2, third-party notices, release notes, contribution
  guide, roadmap, source revision, and build metadata into a deterministic
  directory layout. It includes every checked-in architecture Markdown
  document, writes sorted JSON and SHA-256 records, records files' source paths
  and hashes, and refuses to overwrite an output directory.
- Verification checks required artifacts, manifest provenance fields, exact
  checksum coverage, and file hashes. Both generated manifests keep M30 guest
  acceptance at `NOT_EVALUATED`; integrity verification cannot turn fixture
  inputs into a readiness claim.
- `tools/nagi-release/test_release.py` covers missing documents, checksum
  tampering, source/kernel provenance, fixture handling, and deterministic
  manifest/checksum output.
- `./nagi m30` builds a sparse, self-contained 64 GiB qcow2 with a protective
  MBR and primary/backup GPT. It contains a 512 MiB ESP, 4 GiB System A and B,
  16 GiB User Data, 4 GiB Recovery, and 32 GiB Model Store, with 1 MiB aligned
  partitions and deterministic Nagi unique partition GUIDs. System A/B and
  Recovery contain their own kernel/init files; Recovery uses the separate
  `m27-recovery` init. The image writer preserves unallocated space and
  publishes the completed qcow2 without replacing an existing artifact.
- The UEFI loader locates GPT volumes by unique partition GUID and loads
  System A's root `KERNEL.ELF` and `INIT.ELF`; the legacy M-stage directory
  image path remains available for regression acceptance. The kernel checks
  both GPT headers and entry-array checksums, bounds and overlap, then gives
  user storage only an 8 MiB User Data-relative block capability. Invalid GPT
  metadata fails closed. M-stage persistent disks migrate to a GPT data
  partition while retaining the original raw image as `.legacy-raw`.
- The M30 QEMU path stops on either first-format or existing-data acceptance,
  restarts with the same OVMF variables, and requires a successful VFS
  persistent read and M7 acceptance marker.

## Preflight checklist

- [x] Deterministic manifest and `SHA256SUMS` generation.
- [x] Missing required inputs fail closed; existing output is never overwritten.
- [x] Kernel and qcow2 formats, source pins, and model digest are checked against
  repository provenance.
- [x] Focused standard-library tests cover failure paths and reproducibility.
- [x] Include the §80 release notes, all architecture docs, third-party
  notices, SDK documentation, `CONTRIBUTING.md`, and `ROADMAP.md`.
- [x] Add and locally link-check the M29 SDK, contribution, roadmap, and
  Developer Preview documentation.
- [x] Produce the self-contained 64 GiB reference qcow2 with all six required
  partitions and boot it from GPT System A.
- [x] Verify User Data format/write and persistent read after QEMU restart;
  review both serial logs and run `qemu-img check`.
- [ ] Exercise System B, Recovery, and authenticated update from the GPT image
  using the persistent M27 boot-control policy.
- [ ] Re-run release preflight, assembly, and verification from a clean
  committed revision, then boot the assembled image read-only and verify that
  `SHA256SUMS` still passes.

## Current limits and next steps

The first preflight attempt failed closed on the then-missing SDK README,
contribution guide, and roadmap. Those documents have since been added, but the
release-input audit found that the assembler also omitted §80's release notes
and only copied the architecture index. It now requires the release notes and
all checked-in architecture documents. Eight focused tests pass.

The target kernel output at `target/x86_64-unknown-nagi/release/nagi-kernel`
is a real x86-64 ELF. The reference image now comes from a dedicated GPT/FAT32
writer, not conversion of the M1 or M18 raw FAT fixture. The current VFS
remains fixed at 8 MiB inside the 16 GiB User Data partition. System A and B
currently carry identical payloads, and the default release loader boots
System A. Journal-driven B selection, Recovery selection, and update
acceptance have not yet been exercised from this GPT image.

The first assembled bundle passed `verify` before QEMU testing. Two writable
QEMU boots reached M7 acceptance but changed the qcow2 bytes in User Data, so
the post-boot `verify` correctly rejected its stale checksum. The test-mutated
bundle remains preserved at
`out/evidence/m30-release-bundle-writable-boot-mutated-93b1d25/`. Reassemble
from the committed reference image and use read-only boot acceptance for the
final package integrity check.

The §90 Definition of Done audit remains open across M18–M29: M18 lacks several
browser providers; M19–M26 lack their production guest integrations or real
providers; M27 lacks account-authenticated readiness and GPT-integrated update
acceptance; M28 has not measured the combined reference workload; and M29
lacks screenshots, boot-time evidence, complete Settings/accessibility/
localization, and binary notice clearance. M22 remains blocked at
authenticated AI mutation/Activity Ledger integration. The M16 sample package
acceptance and M17/M18 browser acceptance do not close these gaps.

## Historical clean-tree preflight evidence — 2026-09-30

On clean commit `71e3a9de4ca17c773953358ee8b6bad4364e0d89`, ran:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 tools/nagi-release/release.py preflight \
  --root . \
  --kernel target/x86_64-unknown-nagi/release/nagi-kernel \
  --image out/artifacts/Nagi-OS-0.1-devpreview.qcow2
```

The command exited `2` with:

```text
release preflight failed: missing release qcow2 image: out/artifacts/Nagi-OS-0.1-devpreview.qcow2
```

The required release documents, clean Git provenance, kernel ELF, and pinned
source/model metadata were checked before the missing image stopped preflight.
The M-stage images were inspected with `qemu-img info` and are raw FAT images;
M19/M22 persistence is on separate disks. Converting or renaming those files
would not create the missing release layout. No release image was assembled
or booted, and no release readiness claim is made.

That historical failure occurred before the reference disk existed. Do not run
`./nagi clean` to simulate a clean checkout: it removes preserved M19/M22
disks and other acceptance outputs. The next steps are release preflight,
assembly, and verification from a clean committed source revision and GPT
System B/Recovery acceptance.

Run the focused tests with:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/nagi-release -p 'test_*.py' -v
```

M30 remains partial until every unchecked release and guest acceptance item is
verified from real build outputs.

## GPT reference image and QEMU evidence — 2026-09-30

The dedicated path built
`out/artifacts/Nagi-OS-0.1-devpreview.qcow2`. `qemu-img info --output=json`
reported `format=qcow2` and `virtual-size=68719476736` (64 GiB), and
`qemu-img check -f qcow2` reported no errors. The host allocation at inspection
was about 3.9 MiB because unused partition space is sparse.

`./nagi m30` booted the image through UEFI, selected System A by GPT unique
GUID, and passed M7 storage acceptance across QEMU restarts. The initial
format/write log is preserved at
`out/evidence/m30-release-1790743338835079000/reference-disk-boot.log`. The
latest first-boot and restart logs, both showing GPT System A, VFS mount,
persistent read, and M7 acceptance, are under
`out/evidence/m30-release-1790743948417762000/`.

Verification on the pinned macOS ARM host: `cargo test -p nagi-cli --locked
--target aarch64-apple-darwin` passed 133 unit and 18 CLI integration tests;
warnings-denied CLI Clippy passed; the x86-64 Nagi kernel and default/M27 UEFI
loader target builds passed; all 8 standard-library release-tool tests
passed. On clean source commit
`93b1d258669d4fa5f1ff83623a293be68334617f`, initial release preflight,
assembly, and verification passed. The packaged qcow2 booted twice and both
boots verified GPT System A, VFS mount, persistent read, and M7 acceptance;
logs are in `out/evidence/m30-release-bundle-boot-93b1d25/`. Those writable
boots changed the package image, and post-boot `verify` reported a SHA-256
mismatch. The mutated package is preserved at
`out/evidence/m30-release-bundle-writable-boot-mutated-93b1d25/`; a fresh
assembly and read-only package boot are still required. The generated release
manifest deliberately retains `m30_acceptance: NOT_EVALUATED`; QEMU evidence
is a separate test record. GPT System B/Recovery/update acceptance and
license/notice review remain open.
