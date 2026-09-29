# Nagi OS M30 — Release

**Status: PARTIAL.** This work adds a release artifact assembly and integrity
preflight foundation. It does not mark M30 complete or assert release
readiness.

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
- [ ] Produce the complete self-contained 64 GiB reference qcow2 and matching
  kernel ELF from a clean source revision.
- [ ] Run the reference QEMU acceptance and complete M30 demo path; inspect
  logs and verify the image boots from the assembled artifact.
- [ ] Run assembly and verification from a clean supported build environment.

## Current limits and next steps

The first preflight attempt failed closed on the then-missing SDK README,
contribution guide, and roadmap. Those documents have since been added, but the
release-input audit found that the assembler also omitted §80's release notes
and only copied the architecture index. It now requires the release notes and
all checked-in architecture documents. Eight focused tests pass.

The target kernel output at `target/x86_64-unknown-nagi/release/nagi-kernel`
is a real x86-64 ELF. The current image builder emits raw FAT images: the M1
image is 1,474,560 bytes and the M18 image is 133,844,480 bytes; M19/M22 also
use separate persistent data disks. No self-contained 64 GiB qcow2 release
image or integrated ESP/system-slot/data/recovery/model-store image layout is
present. Converting an M-stage raw boot fixture to qcow2 would not implement
that layout and is not an acceptable release artifact. The candidate image
must be built by an actual release image path, then booted and accepted.

The §90 Definition of Done audit remains open across M18–M29: M18 lacks several
browser providers; M19–M26 lack their production guest integrations or real
providers; M27 lacks firmware-backed slots and the Recovery Environment; M28
has not measured the combined reference workload; and M29 lacks screenshots,
boot-time evidence, complete Settings/accessibility/localization, and binary
notice clearance. M22 remains blocked at authenticated AI mutation/Activity
Ledger integration. The M16 sample package acceptance and M17/M18 browser
acceptance do not close these gaps.

## Clean-tree preflight evidence — 2026-09-30

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

Do not run `./nagi clean` to simulate a clean checkout: it removes the
preserved M19/M22 disks and other acceptance outputs. The next useful
experiment is to implement the specified bootable reference-disk layout from
the real system-slot, persistent-data, recovery, and model-store components,
then build a self-contained 64 GiB qcow2 and rerun this preflight plus QEMU
acceptance. That work depends on the still-incomplete M18–M29 integrations.

Run the focused tests with:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover \
  -s tools/nagi-release -p 'test_*.py' -v
```

M30 remains partial until every unchecked release and guest acceptance item is
verified from real build outputs.
