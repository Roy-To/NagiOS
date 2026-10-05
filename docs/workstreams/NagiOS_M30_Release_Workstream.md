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
- Assembly copies tracked `LICENSE`, `LICENCE`, `COPYING`, and `NOTICE` text
  files under `third_party/` into `licenses/source-tree/`, preserving their
  paths and recording SHA-256 digests in the build manifest. Verification
  checks this exact inventory. Fetched or ignored sources, transitive/native
  license texts, and binary redistribution clearance remain outside this
  inventory and require further review.
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
  user storage an 8 MiB writable User Data-relative block capability and a
  separate read-only capability for the exact Model Store extent. Invalid or
  missing Model Store entries expose no read capability. M-stage persistent
  disks migrate to a GPT data partition while retaining the original raw image
  as `.legacy-raw`.
- The M30 QEMU path copies the reference qcow2 into its run evidence directory,
  stops on either first-format or existing-data acceptance, restarts the copy
  with the same OVMF variables, and requires a successful VFS persistent read
  and M7 acceptance marker. QEMU writes cannot mutate the release input image.
- The GitHub Actions Nagi target job runs `./nagi m30` after M27 acceptance,
  so the reference disk build/validation, two guest boots, persistent User Data
  read, and qcow2 checks are part of the target CI gate.
- The release image builds the loader with `m27-ab-slot-boot-control`, so the
  persistent M27 journal selects GPT System A/B/Recovery in production. The
  release build does not include the acceptance-only empty-journal pending-B
  seed; a blank journal boots confirmed System A.
- `./nagi m27` now exercises the same six-partition GPT layout with malformed
  and healthy System B variants. It covers three failed B trials, Recovery
  without changing a pending journal, rollback to A, healthy-B readiness
  promotion across Recovery, and a confirmed-B boot. This does not establish
  authenticated update or slot-manifest verification.

## Preflight checklist

- [x] Deterministic manifest and `SHA256SUMS` generation.
- [x] Missing required inputs fail closed; existing output is never overwritten.
- [x] Kernel and qcow2 formats, source pins, and model digest are checked against
  repository provenance.
- [x] Focused standard-library tests cover failure paths and reproducibility.
- [x] Bundle and verify the tracked third-party license/notice text files.
- [ ] Complete human license and binary redistribution review; tracked source
  texts do not establish permission to redistribute the assembled image.
- [x] Include the §80 release notes, all architecture docs, third-party
  notices, SDK documentation, `CONTRIBUTING.md`, and `ROADMAP.md`.
- [x] Add and locally link-check the M29 SDK, contribution, roadmap, and
  Developer Preview documentation.
- [x] Produce the self-contained 64 GiB reference qcow2 with all six required
  partitions and boot it from GPT System A.
- [x] Verify User Data format/write and persistent read after QEMU restart;
  review both serial logs and run `qemu-img check`.
- [x] Exercise GPT System B selection, Recovery, retry preservation, rollback,
  and readiness-based promotion using the persistent M27 boot-control policy.
- [ ] Produce and install an authenticated GPT update with authenticated slot
  manifests. Authenticated slot manifests are verified by the loader for
  System A, System B and Recovery (ADR 0061, 2026-10-06); the in-guest
  installer and staging request remain.
- [x] Run release preflight, assembly, and verification from a clean committed
  revision. Boot a byte-identical disposable qcow2 copy twice, then verify the
  untouched assembled package checksums and qcow2 structure.

## Current limits and next steps

The first preflight attempt failed closed on the then-missing SDK README,
contribution guide, and roadmap. Those documents have since been added, but the
release-input audit found that the assembler also omitted §80's release notes
and only copied the architecture index. It now requires the release notes and
all checked-in architecture documents. Eight focused tests pass.

The target kernel output at `target/x86_64-unknown-nagi/release/nagi-kernel`
is a real x86-64 ELF. The reference image now comes from a dedicated GPT/FAT32
writer, not conversion of the M1 or M18 raw FAT fixture. The current VFS
remains fixed at 8 MiB inside the 16 GiB User Data partition. The normal M30
image carries identical A/B payloads and starts with confirmed System A; the
separate M27 GPT fixtures substitute malformed or healthy System B payloads to
verify journal-driven rollback and promotion. No authenticated update
acceptance exists yet.

The first assembled bundle passed `verify` before QEMU testing. Two writable
QEMU boots reached M7 acceptance but changed the qcow2 bytes in User Data, so
the post-boot `verify` correctly rejected the stale checksum. The test-mutated
bundle remains preserved at
`out/evidence/m30-release-bundle-writable-boot-mutated-93b1d25/`. A clean
assembly, two boots of a byte-identical disposable copy, post-boot `verify`,
and `qemu-img check` then passed. Direct read-only QEMU boot reaches GPT
System A but M7 deliberately excludes a read-only block device from writable
storage discovery; failure evidence is at
`out/evidence/m30-release-bundle-readonly-bb61283/`.

The §90 Definition of Done audit remains open across M18–M29: M18 lacks several
browser providers; M19–M26 lack their production guest integrations or real
providers; M27 lacks account-authenticated readiness, authenticated slot
manifests, and an authorized GPT update installer; M28 has not measured the
combined reference workload; and M29 now has an in-session Desktop language
preview, QEMU screenshot, and persistent-boot timing sample but lacks a
first-run flow, persistent settings, complete localization/accessibility, and
binary notice clearance. M22 remains blocked at authenticated AI
mutation/Activity Ledger integration. The M16 sample package acceptance and
M17/M18 browser acceptance do not close these gaps.

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
disks and other acceptance outputs. The later clean-commit assembly and package
acceptance are recorded below; the authenticated update path remains open.

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
`out/evidence/m30-release-bundle-writable-boot-mutated-93b1d25/`. A later
clean assembly and two boots of its byte-identical disposable copy passed;
post-boot `verify` and `qemu-img check` passed on the untouched assembled
package. The generated release manifest deliberately retains
`m30_acceptance: NOT_EVALUATED`; QEMU evidence is a separate test record. GPT
System B/Recovery/update acceptance and license/notice review remain open.

## Clean committed release and package-copy acceptance — 2026-09-30

After commit `144cc0d` was pushed, the prior generated qcow2 was preserved at
`out/evidence/m30-clean-release-144cc0d/preexisting-artifact.qcow2`. A new
blank reference image was built with `./nagi m30`. Its disposable copy passed
System A selection, User Data ext2 format/write, and then a second boot with
mount/read and `Nagi M7 acceptance PASS`. Logs and both qcow2 checks are under
`out/evidence/m30-release-1790751471624505000/`; the pristine image SHA-256 is
`461c644d48e4b0d33b937ce6852eb9a6034abe391ea74c4c24e1ac0b99ca2d43`.

On the clean committed tree, `release.py preflight`, `assemble`, and `verify`
all passed. The assembled package image was copied byte-for-byte to
`out/evidence/m30-clean-release-144cc0d/release-package-qemu-copy.qcow2`; that
copy booted twice, passed first-boot format/write and restart persistence, and
changed its own SHA-256 to
`7e3266b576f129dabe2848bfc1c76f0a52b6ee19c4ab49aac85bc65867437725`. The
untouched package retained SHA-256
`461c644d48e4b0d33b937ce6852eb9a6034abe391ea74c4c24e1ac0b99ca2d43`; final
`release.py verify` and `qemu-img check` passed on it. The two package-copy
serial logs are `release-package-first-boot-1.log` and
`release-package-first-boot-2.log` in the same evidence directory. As intended,
the generated release manifest leaves `m30_acceptance` as `NOT_EVALUATED`; the
external QEMU record is separate. M30 remains PARTIAL for authenticated update
installation, remaining M18–M29 gates, and binary license/notice review.

## Completion Sweep reference-disk rerun — 2026-10-01

On source `16c0b37`, `./nagi m30` accepted the existing validated 64 GiB GPT
qcow2. The disposable QEMU copy passed System A selection, first-boot User Data
format/write, and persistent read after restart. Both the untouched release
input and the test copy passed `qemu-img check`. The source image retained SHA-
256 `461c644d48e4b0d33b937ce6852eb9a6034abe391ea74c4c24e1ac0b99ca2d43`; the
copy's SHA-256 changed as expected after guest writes. Evidence is in
`out/evidence/m30-release-1790782378594368000/`.

This rerun used the already-built release input; it did not rebuild the image
or repeat clean-tree preflight/assembly. Those separate release checks remain
the evidence recorded above. M30 remains PARTIAL for authenticated updates,
remaining M18–M29 acceptance, and binary license/notice review.


## Clean-source release assembly/QEMU checkpoint — 2026-10-01

From clean source commit `284dbf1`, release preflight, assembly, and
verification passed. The current-head bundle is
`out/artifacts/m30-release-bundle-284dbf1/`; its image has the same SHA-256 as
the validated reference input:
`461c644d48e4b0d33b937ce6852eb9a6034abe391ea74c4c24e1ac0b99ca2d43`.
`qemu-img check` found no errors on the bundled disk. The generated release
manifest correctly leaves `m30_acceptance` at `NOT_EVALUATED`.

A current-head `./nagi m30` rerun passed System A selection, first-boot User
Data format/write, and persistent read after a fresh QEMU restart. The
command booted a disposable image copy and confirmed both qcow2 structures.
The invocation log is
`out/logs/m30-284dbf1-qemu.log`; QEMU copies and serial logs are preserved at
`out/evidence/m30-release-1790795417285158000/`. All eight release-tool tests
passed.

The release assembly and QEMU rerun reused the existing kernel and reference
qcow2; they did not rebuild either payload. They establish current-source
release provenance/integrity plus current-head acceptance of the byte-identical
disk, not a clean payload rebuild. Authenticated GPT update installation,
M18–M29 remaining acceptance, and binary license/notice review remain
unverified. M30 remains `PARTIAL`.

## Completion sweep current-source payload and release rebuild — 2026-10-01

The prior fixed-path reference image was moved to
`out/evidence/m30-pre-current-rebuild-20261001/original-reference.qcow2` before
the rebuild, with verified SHA-256
`461c644d48e4b0d33b937ce6852eb9a6034abe391ea74c4c24e1ac0b99ca2d43`. The
fixed path was then absent, so `./nagi m30` rebuilt the current kernel, loader,
init images, and dedicated six-partition 64 GiB GPT qcow2 from clean revision
`d2cbff3ed9bde82bf6dde910b3b5bf5e30c6cfb7`. The new reference image SHA-256
is `f76088e25cea65176035940930dd3b9fd2df796614d0e554fb8d345271558bb9` and
the target kernel build ID is
`sha256:2c899885d4569d58cb29ab028bc9923e44519c8fa25009f6fce836cc6e982343`.

Two current-source `./nagi m30` runs passed System A selection, User Data
format/write, and persistent read after restart; the second run followed
release assembly and used a disposable copy of the reference image. The
reference image and packaged image have the same SHA-256. Its serial logs,
QEMU copy, README, and verified `SHA256SUMS` are in
`out/evidence/m30-release-1790802137948877000/`. The eight release-tool tests
passed. On the clean source revision, preflight, assembly to
`out/artifacts/m30-release-bundle-d2cbff3/`, and verification passed. After
QEMU acceptance, all 15 bundle checksums passed again, release verification
passed, and `qemu-img check` found no errors on the bundled 64 GiB qcow2.

The generated `release-manifest.json` deliberately retains
`m30_acceptance: NOT_EVALUATED`; the real guest QEMU result remains a separate
acceptance record. The host's missing `virtio-sound.in` driver was reported by
QEMU, so this evidence does not cover audio. Authenticated update installation,
M18–M29 remaining gates, and binary license/notice review remain incomplete;
M30 stays `PARTIAL`.

## M20 read-only Model Store QEMU gate — 2026-10-01

The M30 acceptance init is now built with `m20-model-store-acceptance`. It
receives the sixth bootstrap argument as the Model Store capability, reads the
32 GiB partition's FAT32 boot sector through the capability-relative block
syscall, and passes the root directory through `Fat32ArtifactReader`. The
current image has no Granite file, which is an accepted optional-model state;
when present, this gate checks the `GGUF` header. A block-write attempt using
the Model Store capability is rejected, and a follow-up read confirms the
sector is unchanged. User Data retains its existing writable capability.

The first rebuilt image placed this check before the M5 FPU-state gate, so QEMU
reported M20 PASS and then failed `Nagi M5 FPU state`. That failed image and
serial log are preserved under
`out/evidence/m30-release-1790804891726864000/`. The check was moved after the
initial and round-trip FPU checks. A later reliability adjustment made a
missing/unreadable Model Store capability or invalid FAT32 volume emit an M20
FAIL marker without terminating ordinary boot after GPT/User Data
initialization; structurally invalid GPT metadata remains fail-closed. The M30
CLI still requires the M20 PASS marker. The latest
two-boot `./nagi m30` run passed System A selection, M20 read-only/FAT32
discovery, User Data format/write/restart-read, and M7 acceptance. Both serial
logs and the disposable QEMU copy are in
`out/evidence/m30-release-1790805673208395000/`. The immediately previous
accepted reference image is preserved there as
`reference-disk-before-nonfatal-store.qcow2`. The latest image hash is
`e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`; `qemu-img
check` passed on source and acceptance copy.

The previous fixed-path image and release bundle were copied, SHA-256 checked,
and bundle-verified before rebuilding. They are preserved at
`out/evidence/m30-pre-m20-store-20260930T214713Z/`. The latest rebuilt image
has not yet been assembled into a clean-source release bundle. Release
manifest acceptance remains `NOT_EVALUATED`; this gate verifies Model Store
capability and discovery only, not artifact installation, model loading,
authenticated updates, or inference. M30 remains `PARTIAL`.

## Clean-source Model Store release bundle and QEMU acceptance — 2026-10-01

After commit `7144949` was pushed, the clean-source release preflight passed
with the release kernel and current 64 GiB reference qcow2. The eight release
tool tests passed, and assembly/verification produced
`out/artifacts/m30-release-bundle-7144949/`. Its qcow2 is byte-identical to
the current reference image (SHA-256
`e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`) and
reports a 64 GiB virtual size.

After assembly, `./nagi m30` passed both QEMU boots using a disposable copy of
the same reference image. System A, Model Store capability-relative read and
write denial, empty-root artifact discovery, User Data format/write and
restart persistence, and M7 acceptance all passed. The QEMU copy, serial logs,
README, and local `SHA256SUMS` are preserved under
`out/evidence/m30-release-1790806188358089000/`. Post-QEMU `release.py verify`,
all 15 bundle checksums, and `qemu-img check` passed on the untouched bundle.
The generated `m30_acceptance` field remains `NOT_EVALUATED`; the external
guest acceptance is recorded separately. QEMU warned that the host has no
`virtio-sound.in` driver, so this run does not verify host audio capture.

The source and reference image are real build outputs. The Model Store is
empty, so this acceptance does not claim artifact installation, Granite load,
or inference. M30 remains `PARTIAL` for authenticated updates, incomplete
M18–M29 acceptance, and binary license/notice review.

## Runtime digest-gate source release — 2026-10-01

After the FAT32-to-`ModelRuntime` digest test and runtime validation fix were
committed as `0d0ae8a`, clean-source release preflight passed again. The eight
release-tool tests passed, and assembly plus verification produced
`out/artifacts/m30-release-bundle-0d0ae8a/`. Its 64 GiB qcow2 matches the
reference-image SHA-256
`e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`.

The post-assembly `./nagi m30` run passed two QEMU boots using a disposable
copy: System A, M20 capability-relative Model Store read/write denial, empty
root artifact discovery, User Data format/write/restart persistence, and M7
acceptance. Evidence, README, and checksums are in
`out/evidence/m30-release-1790806831243045000/`. Post-boot release verification,
all 15 package checksums, and `qemu-img check` passed on the untouched bundle.
The release manifest keeps `m30_acceptance=NOT_EVALUATED`; the separate QEMU
acceptance is not converted into a release-manifest claim. QEMU again reported
the missing host `virtio-sound.in` driver.

The FAT32 reader/runtime integration remains an orchestration test using a
fake backend. The M30 image still contains no model artifact; this bundle does
not claim Granite load or inference. M30 remains `PARTIAL` for authenticated
updates, remaining M18–M29 acceptance, and binary license/notice review.


## Tracked third-party license text bundle — 2026-10-01

Commit `16e0cd4` adds deterministic inclusion of non-empty, tracked
`LICENSE`/`LICENCE`/`COPYING`/`NOTICE` files under `third_party/`. The release
tool records each source path, package path, and SHA-256 in the build manifest,
copies each text byte-for-byte under `licenses/source-tree/`, and checks the
complete file set while verifying the package. Twelve release-tool tests
passed, including stable discovery, hash tampering, symlink rejection, and
legacy schema-v1 verification without the additive inventory field.

Clean-source preflight and assembly passed for
`out/artifacts/m30-release-bundle-16e0cd4/`. It contains eight tracked license
texts and 24 total files; all 23 `SHA256SUMS` entries verified. Its 64 GiB
qcow2 has the same SHA-256 as the reference image,
`e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`, and
`qemu-img check` found no errors. `./nagi m30` passed the two-boot System A and
User Data persistence acceptance using a disposable copy of that identical
image. Logs and the verified evidence manifest are in
`out/evidence/m30-release-1790807691542412000/`; post-boot verification and
all package checksums passed on the untouched bundle.

This bundles only license texts already tracked in this checkout. Fetched or
ignored source texts, Cargo/native component notices, and human redistribution
review remain open. The release manifest retains
`m30_acceptance=NOT_EVALUATED`; M30 remains `PARTIAL`.

## Separate M20 guest FAT32 reader fixture — 2026-10-01

`./nagi m30` continued to boot the existing 64 GiB reference image without
modifying it, then booted a separate Model Store fixture image built through
the same GPT/FAT32 disk writer. The reference image SHA-256 is still
`e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`; the
release-image copy passed User Data format/write/restart-read acceptance.

The dedicated fixture boot emitted `Nagi M20 Model Store capability PASS` and
`Nagi M20 FAT32 fixture read PASS`. It read and byte-checked the 5,000-byte
`nagi.m20.reader-fixture` artifact, including a range across a FAT32 cluster
boundary and EOF. This test-only payload has a GGUF magic prefix but is not a
valid model. `qemu-img check` passed for the reference image, the mutable
release acceptance copy, and the fixture qcow2. Evidence and checksums are in
`out/evidence/m30-release-1790809848636521000/`.

This is guest artifact-reader evidence only. It does not make
`m30_acceptance` anything other than `NOT_EVALUATED`, and it does not claim
artifact installation, GGUF loading, Granite, or inference. Host audio capture
was unavailable (`virtio-sound.in`); M30 remains `PARTIAL`.

## M29 completion-sweep release regression — 2026-10-01

After the Desktop Settings/localization change, `./nagi m30` passed again on
the same self-contained 64 GiB GPT qcow2. Two disposable-copy boots passed
System A, Model Store read-only/FAT32 discovery, User Data write and
restart-read, and M7 acceptance. `qemu-img check` reported no image errors;
the reference qcow2 SHA-256 remains
`e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`.
Evidence is under `out/evidence/m30-release-1790812803135254000/`. The
separate M20 fixture again passed the 5,000-byte Model Store reader test; it
remains test data rather than an installable model or inference result. M30
remains `PARTIAL` for release preflight on the current commit, authenticated
updates, M18–M29 gaps, and binary license review.

## Clean-source release bundle copy acceptance — 2026-10-01

On clean commit `637f5755a57eaa14adc379fdddfaeebb5dea6387`, release preflight,
assembly, and verification passed. The release-tool suite passed all 12 tests.
The bundle is `out/artifacts/m30-release-bundle-637f575/`; it contains 23
`SHA256SUMS` entries and records the full source revision. The bundled qcow2
SHA-256 is
`e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`.

The bundled qcow2 was copied byte-for-byte to
`out/evidence/m30-clean-release-637f575/package-qemu-copy.qcow2` and booted
twice with the same writable OVMF variables. The first boot selected GPT
System A, passed the read-only Model Store and M27 journal checks, and
formatted/wrote User Data. The second boot mounted and read the persisted data
and printed `Nagi M7 acceptance PASS`. The disposable copy changed to SHA-256
`0eafa697f6a918486eeb9bd3b6cf673d3792d1698461c76ca3ea88a47b26940a` after
guest writes; the assembled package remained at its original digest. Both
qcow2 images passed `qemu-img check`, and post-boot `release.py verify` passed
on the untouched package. Serial logs, QEMU stderr, OVMF variables, the exact
runner, and before/after hashes are preserved in the evidence directory.

The bundle's `m30_acceptance` remains `NOT_EVALUATED`; the separate two-boot
QEMU evidence does not promote the manifest field. M30 remains `PARTIAL` for
authenticated GPT update installation, outstanding M18–M29 acceptance, and
human binary redistribution review.

## GPT Recovery partition boot acceptance — 2026-10-02

The M30 QEMU gate now boots Recovery from the Recovery partition in the same
self-contained GPT release image, checks the Recovery VFS and command help,
and requires the loader to leave the A/B journal unchanged. It then restarts
and verifies confirmed System A plus the persistent User Data read. System B
remains unstaged in the release image; this does not claim update acceptance.

On clean source commit
`74369f7997bff8b877980841ecd9bcb03ae66f06`, fresh `./nagi m30` passed the
System A and User Data checks, GPT Recovery selection and Recovery console,
post-Recovery System A restart, and separate M20 fixture read. The pristine
64 GiB qcow2 SHA-256 remains
`54390507a4e975ad30ee94d7efb7b4c81758854ccbbdcc7f12b39bfb70fc6748`; the
`.build-info` binds it to the full source revision. `qemu-img check` passed on
the pristine image, writable QEMU copy, fixture, and bundle. Release preflight,
assembly, verification, all 23 package checksums, and bundle-image
byte-identity passed. The evidence manifest is
`out/evidence/m30-release-1790885396024787000/SHA256SUMS`, and the assembled
package is `out/artifacts/m30-release-bundle-74369f7/`.

All 14 release-tool tests pass. QEMU reported no host `virtio-sound.in` input
driver, so host audio I/O remains untested. The release manifest continues to
record `m30_acceptance=NOT_EVALUATED`; M30 remains `PARTIAL` pending
authenticated GPT updates, System B update acceptance, remaining M18–M29
gates, and human binary redistribution review.

## System-language persistence regression — 2026-10-01

After the M29 Desktop began persisting System language, `./nagi m30` passed
again on the existing self-contained 64 GiB reference qcow2. The two disposable
copy boots passed System A, the read-only Model Store capability check, User
Data format/write and restart-read, and M7 acceptance. The separate M20
fixture again passed the 5,000-byte FAT32 reader check, including a cluster
boundary and EOF range. The source image stayed at SHA-256
`e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`, and
`qemu-img check` found no errors in the disposable reference copy or fixture.

Run evidence and a verified seven-file SHA-256 manifest are in
`out/evidence/m30-release-1790817095155131000/`. The host again lacked
`virtio-sound.in`; audio playback/capture is not established. This regression
does not change `m30_acceptance=NOT_EVALUATED` or M30's `PARTIAL` status.

## Current clean-commit bundle acceptance — 2026-10-01

After `805f2bb` was pushed, release preflight, assembly, and verification
passed on the clean tree. The bundle at
`out/artifacts/m30-release-bundle-805f2bb/` contains 23 verified checksum
entries and records the full source revision. Its qcow2 is byte-identical to
the reference image (`e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`).

A disposable byte-identical package copy booted twice with shared OVMF
variables. The first boot passed System A, Model Store read-only, M27
confirmed-A, and User Data write markers. The second boot read the persisted
User Data and reached `Nagi M7 acceptance PASS`. The copy's final SHA-256 is
`29ce659593b9dd9335f4408cbcbdca28c13ca66f3724516f75b646414dbef853`; the
untouched package retained its original hash. Both package verification and
all checksums passed after the boots, and `qemu-img check` reported no errors
in the package copy. Evidence and a verified nine-file manifest are under
`out/evidence/m30-clean-release-805f2bb/`. QEMU reported missing host audio
input (`virtio-sound.in`); host audio is not covered. The package manifest
still records `m30_acceptance=NOT_EVALUATED`, and M30 remains `PARTIAL` for
authenticated updates, remaining M18–M29 acceptance, and human redistribution
review.

## Current-source release rebuild and bundle acceptance — 2026-10-01

On clean commit `5ee985f64da4b01837ef37df39693d35bce09135`, `./nagi m30`
rebuilt the 64 GiB reference qcow2 from current payloads. The preceding image
was preserved at
`out/evidence/m30-release-1790848594984804000/reference-disk-before-current-source-rebuild.qcow2`;
its SHA-256 remained
`e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`. The new
image SHA-256 is
`4155639e8866bff430738d996b40eab350a514777a50a16511701476108f3714`.

The current-source M30 run passed GPT System A selection, the read-only Model
Store capability, User Data format/write/restart-read, and M7 acceptance. The
separate 5,000-byte FAT32 fixture passed its cross-cluster and EOF checks.
`qemu-img check` passed for the source image, mutable acceptance copy, and
fixture. The README and verified checksum manifest are in
`out/evidence/m30-release-1790848708675783000/`.

Release preflight, assembly, and verification passed on that clean revision.
The package at `out/artifacts/m30-release-bundle-5ee985f/` contains 23
checksum-covered files and pins the full source revision. A byte-identical
copy booted twice with shared OVMF variables: first-boot System A, Model Store,
User Data write, then restart persistence and `Nagi M7 acceptance PASS`. The
copy changed from the package digest to
`6bba38fd25cd795db3ac9a792aa36bb1614cfeaabcc8bb607350dd2aff64193c`; the
untouched package retained
`4155639e8866bff430738d996b40eab350a514777a50a16511701476108f3714`. Post-boot
package verification, checksums, and both qcow2 checks passed. Evidence and a
verified ten-file manifest are in
`out/evidence/m30-clean-release-5ee985f/`. All 12 release-tool tests passed.

The release bundle still records `m30_acceptance=NOT_EVALUATED`; M30 remains
`PARTIAL` for authenticated GPT update installation, remaining M18–M29
acceptance, and human binary redistribution review. QEMU reported no host
`virtio-sound.in` input driver, so audio I/O is not established.

## Current-source M29 keyboard-navigation release rebuild — 2026-10-01

After M29 gained keyboard navigation, the previous reference qcow2 was still
present at the fixed artifact path, so `./nagi m30` would have reused the older
payload. Its SHA-256
(`4155639e8866bff430738d996b40eab350a514777a50a16511701476108f3714`) was
preserved in `out/evidence/m30-release-1790848708675783000/`; that evidence
manifest continues to verify. A fresh image was then built from clean commit
`bb43b7f36eb8d4ccdbceabada720e68b71dc113c`. The current qcow2 SHA-256 is
`ca4c04f5c540bf59cef13b93b301a9fe31134080650977b1a57e760567ce4cff` and its
virtual size is 64 GiB.

The fresh `./nagi m30` QEMU acceptance passed GPT System A, the read-only Model
Store capability, User Data format/write and restart-read, and M7 acceptance.
The separate M20 FAT32 fixture passed a 5,000-byte read across a cluster
boundary and EOF. `qemu-img check` passed for the pristine image, mutable
acceptance copy, and fixture. Logs, disk images, OVMF variables, and the
pristine image are covered by the verified evidence manifest at
`out/evidence/m30-release-1790851367488764000/`.

From the clean committed source, release preflight, assembly, and verification
passed for `out/artifacts/m30-release-bundle-bb43b7f/`; all 12 release-tool
tests passed and the bundle has 23 verified checksums. Its image is
byte-identical to the current reference image. A disposable package copy booted
twice with shared OVMF variables. Boot 1 passed System A, read-only Model Store,
confirmed-A, and User Data format/write; boot 2 read the persisted data and
reported `Nagi M7 acceptance PASS`. The copy changed to SHA-256
`474ea42ea2f99c95b98488b425ee6e9b0a9fdee78e69e7ec03a5caddaacb84e9`; the
untouched package remained at
`ca4c04f5c540bf59cef13b93b301a9fe31134080650977b1a57e760567ce4cff`.
Post-boot `release.py verify`, package checksums, and `qemu-img check` passed.
The 20-entry evidence manifest verifies at
`out/evidence/m30-clean-release-bb43b7f/`.

The release manifest continues to set `m30_acceptance=NOT_EVALUATED`. This
QEMU coverage does not authenticate updates or slot manifests. M30 remains
`PARTIAL` for authenticated GPT update installation, remaining M18–M29
acceptance, and human binary redistribution review. QEMU reported no host
`virtio-sound.in` input driver, so audio I/O is not covered.

## Current-source release rebuild and QEMU acceptance — 2026-10-01

On clean commit `9db7e0f8d083c7d7ef32d641f7a4c48c45a598bb`, `./nagi m30`
rebuilt the kernel, loader, init payloads, and self-contained 64 GiB GPT
reference qcow2. Before the rebuild, the prior accepted image was moved to
`out/evidence/m30-current-source-rebuild-pre-9db7e0f/` and its preserved bytes
verified against SHA-256
`ca4c04f5c540bf59cef13b93b301a9fe31134080650977b1a57e760567ce4cff`. The new
image SHA-256 is
`cb63509d4d221320cf2dec1637b5acdcf662cd22e6436717660185389624d5f5`.

The fresh QEMU copy passed GPT System A selection, Model Store capability
checks, User Data format/write, and persistent read after restart. The separate
M20 FAT32 fixture passed the bounded 5,000-byte read across a cluster boundary
and EOF. All 12 release-tool tests passed. On the clean revision, release
preflight, assembly to `out/artifacts/m30-release-bundle-9db7e0f/`, and
verification passed. The bundle image is byte-identical to the pristine
QEMU-tested reference image; release checksums and `qemu-img check` passed for
the pristine image, QEMU copy, fixture, and bundled qcow2. The 15-entry run
manifest verifies at `out/evidence/m30-release-1790855765560052000/SHA256SUMS`.

The bundle still records `m30_acceptance=NOT_EVALUATED`: release integrity and
guest acceptance are recorded separately from full release readiness. M30
remains `PARTIAL` for authenticated GPT updates, remaining M18–M29 acceptance,
and human binary redistribution review. QEMU reported no host
`virtio-sound.in` driver, so audio I/O remains untested.

## Current-source rebuild, release bundle, and QEMU acceptance — 2026-10-02

The first `./nagi m30` run reused the fixed-path qcow2 from the previous clean
source commit `9db7e0f8d083c7d7ef32d641f7a4c48c45a598bb`, identified by SHA-256
`cb63509d4d221320cf2dec1637b5acdcf662cd22e6436717660185389624d5f5`. Before
rebuilding, that exact file was copied, checksum-verified, and passed through
`qemu-img check`; its recoverable copy and manifest are at
`out/evidence/m30-pre-current-rebuild-39234a6/`.

After clearing only the generated fixed-path artifact, `./nagi m30` rebuilt
the self-contained 64 GiB GPT reference qcow2 from clean source commit
`39234a68208fe849461904f5a68cea1daae2f772`. The pristine image SHA-256 is
`839eee861a444f2dea447c1f0b5a9dd2a0db4672fa2d8c7ee6290ab5cc3648cf`. QEMU
passed GPT System A, read-only Model Store capability checks, User Data format
and write, and persistent read after restart. A separate M20 guest fixture
passed its 5,000-byte FAT32 read across a cluster boundary and EOF. `qemu-img
check` passed for the pristine image, mutable QEMU acceptance copy, and fixture.
The mutable copy's digest differs because the guest wrote User Data; the
pristine reference image remained unchanged.

On this same clean source revision, release preflight and assembly passed for
`out/artifacts/m30-release-bundle-39234a6/`; all 12 release-tool tests passed.
`release.py verify`, the package SHA-256 list, and `qemu-img check` passed. The
bundled qcow2 is byte-identical to the pristine QEMU-tested reference image.
The 13-entry run manifest covers the pristine image, QEMU copy, boot logs,
OVMF variables, M20 fixture, and release provenance manifests at
`out/evidence/m30-release-1790868798244504000/SHA256SUMS`.

The release manifest still records `m30_acceptance=NOT_EVALUATED`; these
separate guest checks do not authenticate updates or slot manifests. M30
remains `PARTIAL` for authenticated GPT update installation, remaining M18–M29
acceptance, and human binary redistribution review. QEMU reported no host
`virtio-sound.in` driver, so audio I/O remains untested.

## Completion Sweep — source-bound image reuse guard (2026-10-02)

The M30 CLI now requires a clean committed source tree and writes an adjacent
`.build-info` sidecar after creating the reference image. The record binds the
full Git revision to the exact qcow2 SHA-256. Existing images are reused only
when both fields match; an older image without a sidecar, or one built from a
different revision, is rejected before QEMU acceptance. Release preflight and
assembly validate the same sidecar, and the generated `build-manifest.json`
records its values. Existing assembled manifests without the additive field
remain verifiable; new assemblies require the sidecar.

The 14 release-tool tests pass, including missing/stale provenance and image
tampering; all 159 `nagi-cli` unit and 21 integration tests, warnings-denied
Clippy, formatting, and whitespace checks pass. The full target build and
current-commit M30 run passed on clean commit
`567afa42ed5b3f6f374b176b11f4a6524175eeb2`. The fresh reference qcow2 SHA-256
is `54390507a4e975ad30ee94d7efb7b4c81758854ccbbdcc7f12b39bfb70fc6748`; its
sidecar records the same full source revision and image digest. QEMU passed
System A boot, read-only Model Store capability, User Data write and
restart-read, and M7 acceptance. The separate M20 fixture passed its 5,000-byte
FAT32 read across a cluster boundary and EOF. The pristine image, mutable QEMU
copy, fixture, and bundled image passed `qemu-img check`.

On that clean revision, release preflight and assembly passed for
`out/artifacts/m30-release-bundle-567afa4/`; `release.py verify`, all 21 bundle
checksums, and image byte-identity checks passed. The bundle manifest records
the sidecar provenance and keeps `m30_acceptance=NOT_EVALUATED`. A bundle
created before the additive provenance field still verifies. All 14
release-tool tests passed. The 10-entry QEMU evidence manifest verifies at
`out/evidence/m30-release-1790883362672919000/` and includes a preserved copy
of the pristine image and its sidecar. QEMU reported no host
`virtio-sound.in` input driver, so audio I/O is not covered. M30 remains
`PARTIAL` for authenticated updates, remaining M18–M29 acceptance, and human
binary redistribution review.

## Completion Sweep — quiescent M13/M19/M22 restart and Recovery gate — 2026-10-02

The M30 restart gate now waits for `Nagi M13 acceptance PASS` before it stops
System A. M13 runs after the M19 Search and M22 History fixtures and writes
their state to User Data; stopping at the earlier M7 marker left an incomplete
inode, which correctly caused Recovery's read-only VFS check to fail. The
restart also starts the M13 HTTP fixture and requires M19 Search and M22
Activity Ledger/Move/Copy markers. The first failure and mutable copy are
preserved at `out/evidence/m30-release-1790885889238875000/`.

A subsequent run showed the post-Recovery System A restart also needs the M13
HTTP fixture and must require M13/M19/M22 completion markers. Its M13 HTTP
failure and the prior gate's premature success summary are preserved, not
counted as acceptance, at
`out/evidence/m30-release-1790886615318484000/`. Both failed-run evidence
manifests verify. The post-Recovery runner now starts the same HTTP fixture and
requires the M22 Activity Ledger Undo/composite Undo and M13 completion
markers.

On clean source revision
`b66fabb1388e67eb4e35fa9cf72d231e61bf097f`, fresh `./nagi m30` run
`1790886957142079000` passed System A and User Data persistence, M19 Search,
M22 grouped Move/Copy and Activity Ledger, Recovery from the GPT Recovery
partition (`files=20 directories=5`), Recovery help and unchanged A/B journal,
rejection of unstaged System B, post-Recovery persistent read and M22 Undo,
M13 completion, and the separate M20 5,000-byte FAT32 fixture. The verified
17-entry evidence manifest is
`out/evidence/m30-release-1790886957142079000/SHA256SUMS`.

The pristine source-bound image has SHA-256
`54390507a4e975ad30ee94d7efb7b4c81758854ccbbdcc7f12b39bfb70fc6748` and
virtual size 64 GiB. `qemu-img check` passed on the pristine, mutable,
fixture, and bundle images. All 14 release-tool tests passed; preflight,
assembly to `out/artifacts/m30-release-bundle-b66fabb/`, verification, all 23
checksums, and pristine/bundle byte-identity passed. The bundle correctly
records `m30_acceptance=NOT_EVALUATED`. QEMU lacked host `virtio-sound.in`, so
host audio input remains untested. M30 remains `PARTIAL` for authenticated
updates and System B acceptance, remaining M18–M29 work, and human binary
redistribution review.

## Completion Sweep — current-source regression after M20 artifact acceptance (2026-10-02)

On clean source commit `0e7756336cc0f05d28734633df2a9eac12557b5c`, fresh
`./nagi m30` run `1790893780350727000` passed System A, User Data restart
persistence, M19 Search, M22 Move/Copy and Activity Ledger, Recovery VFS/help,
unstaged-System-B rejection, and post-Recovery M22 Undo. The separate M20
fixture passed its bounded FAT32 cross-cluster/EOF read. `qemu-img check`
passed independently for the pristine release image, acceptance copy, and
fixture image. The release qcow2 digest matches its source-bound build-info.

The 13-entry run evidence manifest is
`out/evidence/m30-release-1790893780350727000/SHA256SUMS`. The ordinary
Model Store remains empty; this run exercised the capability but did not load
Granite or perform inference. M30 remains `PARTIAL` for authenticated updates,
System B acceptance, remaining M18–M29 acceptance, and distribution review.

## Completion Sweep — current M25/M28/M29 source release regression (2026-10-02)

Before rebuilding, the fixed-path image from source `0e77563` and its
`.build-info` were moved intact to
`out/evidence/m30-pre-m25-whisper-current-source-20261002/`; both the original
and preserved qcow2 passed `qemu-img check`, and the three-entry manifest
verifies.

On current clean source `b4385e1dac8b35e3f86a13f34ae5d806cbd0e40d`,
`./nagi m30` rebuilt the self-contained 64 GiB GPT reference image. Its SHA-256
is `8260512ffd8dfae98539699163c3bee39e91acd9f2dc88137f0e45cb553dbff7`,
matching the new source-bound `.build-info`. QEMU passed System A, read-only
Model Store access, User Data write/restart-read, M19 Search, M22 Activity
Ledger/Move/Copy, Recovery, unchanged A/B journal, unstaged System B
rejection, post-Recovery System A with persisted M22 Undo, M13 completion,
and the separate M20 FAT32 reader fixture. `qemu-img check` passed for the
pristine image, mutable QEMU copy, and fixture; 13 evidence checksums verify
under `out/evidence/m30-release-1790896634463366000/SHA256SUMS`.

All 14 release-tool tests passed. Clean-source preflight and assembly to
`out/artifacts/m30-release-bundle-b4385e1/` passed, as did `release.py verify`,
the bundle image's `qemu-img check`, and byte identity with the pristine image.
The bundle manifest still says `m30_acceptance=NOT_EVALUATED`; this QEMU run
does not perform authenticated System B update acceptance. No model was
loaded, the M20 fixture is not Granite, and no inference is claimed. QEMU had
no `virtio-sound.in` host driver. M30 remains `PARTIAL`.

## Completion Sweep release checkpoint on b11858b — 2026-10-02

The prior source-bound reference image from `b4385e1` was preserved with its
build-info under `out/evidence/m30-pre-b11858b-current-source-20261002/`; both
the qcow2 and sidecar have verified hashes, and `qemu-img check` passed before
and after preservation.

From clean source commit `b11858baad8beea9955f4e52433380c24f302c11`,
`./nagi m30` rebuilt the self-contained 64 GiB GPT reference image. Its SHA-256
is `95577eb6f46f9cafeeaf4adc7ea48963483f31c0bcf63a281486e8ec3c8b747d`,
matching the source-bound `.build-info`. Run
`1790900017117346000` passed System A boot, User Data write/restart-read, M19
Search, M22 grouped Move/Copy and Activity Ledger Undo, Recovery VFS/help with
unchanged A/B journal, unstaged-System-B rejection, post-Recovery persistence,
and the separate M20 FAT32 Model Store reader fixture. The pristine image,
mutable QEMU copy, and fixture passed `qemu-img check`.

All 14 release-tool tests passed. Clean-source `release.py preflight`,
assembly to `out/artifacts/m30-release-bundle-b11858b/`, and `verify` passed.
The packaged qcow2 passed `qemu-img check` and is byte-identical to the pristine
reference. The 16-entry evidence checksum manifest verifies at
`out/evidence/m30-release-1790900017117346000/SHA256SUMS`. The bundle records
`m30_acceptance=NOT_EVALUATED`; the separate QEMU evidence does not change that
field. No model was loaded or used for inference. QEMU reported that this host
has no `virtio-sound.in` input driver, so this run does not test audio.

M30 remains PARTIAL for authenticated System B updates, remaining M18–M29
acceptance, and human binary redistribution review.

## Completion Sweep release checkpoint on acffe0b — 2026-10-02

After the M29 selected-locale text cue changed the target source, the prior
`b11858b` image and sidecar were preserved under
`out/evidence/m30-pre-acffe0b-current-source-20261002/`. From clean committed
source `acffe0bdc38571cbec007c8de9af69078033e393`, `./nagi m30` rebuilt the
self-contained 64 GiB reference qcow2. Its SHA-256 is
`47615fce4e0b7442f1d016add408eb84c120b6fb5ad0dcd85b00c517e4de2d41`; the
adjacent `.qcow2.build-info` binds that digest to the full source revision.

QEMU run `1790901835370320000` passed System A boot, User Data first write and
restart persistence, Recovery, rejection of unstaged System B, the
post-Recovery restart, and the separate M20 FAT32 Model Store reader fixture.
The 20-entry evidence manifest verifies at
`out/evidence/m30-release-1790901835370320000/SHA256SUMS`. `qemu-img check`
passed on the pristine reference, the writable QEMU acceptance copy, the
Model Store fixture, and the assembled bundle image.

On the same clean source, release preflight, assembly to
`out/artifacts/m30-release-bundle-acffe0b/`, and release-tool verification
passed. The 14 release-tool unit tests passed. All 24 bundle checksums verify,
and the bundle image is byte-identical to the pristine reference image. QEMU
tested a disposable copy of that reference; the assembled bundle remained
untouched and verified. The generated release manifest deliberately retains
`m30_acceptance=NOT_EVALUATED`, so this external guest evidence is not a release
readiness declaration. QEMU had no `virtio-sound.in` input backend; audio was
not tested and no model inference is claimed. M30 remains `PARTIAL` for
authenticated updates, remaining M18–M29 acceptance, and human binary
redistribution review.

## Completion Sweep release checkpoint on 25e0b54 — 2026-10-02

The fixed-path image from `acffe0bdc38571cbec007c8de9af69078033e393` did not
match current-source provenance, so `./nagi m30` correctly refused to reuse
it. The old qcow2 and sidecar were moved intact to
`out/evidence/m30-preserved-stale-25e0b54/`; their SHA-256 manifest verifies.
From clean commit `25e0b5443363f87a4e503a3031cb9804f3e29c07`, `./nagi m30`
rebuilt the source-bound 64 GiB GPT reference qcow2. Its SHA-256 is
`47615fce4e0b7442f1d016add408eb84c120b6fb5ad0dcd85b00c517e4de2d41`, and
the sidecar binds it to that full source revision.

QEMU run `1790904245966571000` passed System A, User Data persistence after
restart, M19 VFS Search/ObjectId persistence, M21 `file.search` fixture,
M22 grouped Move/Copy and Activity Ledger Undo, Recovery, unstaged-System-B
rejection, post-Recovery Search/Undo, and the separate M20 FAT32 Model Store
reader fixture. The 12-entry manifest verifies at
`out/evidence/m30-release-1790904245966571000/SHA256SUMS`; source, mutable
acceptance copy, Model Store fixture, and assembled bundle all passed
`qemu-img check`.

All 14 release-tool tests passed. Clean-source preflight, assembly to
`out/artifacts/m30-release-bundle-25e0b54/`, release verification, all 22
bundle checksums, and byte identity between the bundled and pristine qcow2
passed. The build manifest binds source `25e0b54` and the image digest; the
release manifest correctly retains `m30_acceptance=NOT_EVALUATED`. No model
was loaded and no inference is claimed. The host has no `virtio-sound.in`
backend. M30 remains `PARTIAL` for authenticated System B updates, remaining
M18–M29 acceptance, and human redistribution review.

## Completion Sweep — bundle-wide symlink rejection (2026-10-02)

`verify_release()` and the standalone checksum verifier now reject a symlink
release-bundle root and every symlink entry before reading metadata or hashing
payloads. The recursive walk uses `followlinks=False`, so an untracked
directory symlink cannot hide external files outside checksum and manifest
inventories. The new regression failed before the change and passes afterward;
all 16 release-tool tests pass. `./nagi fmt`, `./nagi lint`, `./nagi test`, and
`./nagi build` pass.

From commit `65f4d6f8773e0b373f237067960738f66e454f3b`, `./nagi m30` run
`1790926329664045000` passed System A, User Data restart persistence, M19
Search, M22 Ledger/Move/Copy, GPT Recovery, unchanged A/B journal, unstaged
System B rejection, post-Recovery restart, and the separate M20 Model Store
reader fixture. The source-bound 64 GiB qcow2 SHA-256 is
`bda4e15274f52b9005497d05dff0ff732d01aac93d8fec5cd26fe87b50929258`; its
build-info binds the image to the source commit. The host has no
`virtio-sound.in` driver, though the configured sound device did not prevent
acceptance.

Clean-source release preflight passed. Assembly to
`out/artifacts/m30-release-bundle-65f4d6f/`, release verification, all 23
bundle checksums, byte identity between bundle and pristine qcow2, and both
`qemu-img check` runs passed. The 17-entry run manifest verifies at
`out/evidence/m30-release-1790926329664045000/SHA256SUMS`. The stale fixed-path
image bound to `25e0b54` was preserved with its sidecar and a verified manifest
at `out/evidence/m30-release-symlink-stale-image-65f4d6f/`.

The release manifest retains `m30_acceptance=NOT_EVALUATED`; no model was
loaded and no inference is claimed. M30 remains `PARTIAL` for authenticated
System B updates, remaining M18–M29 acceptance, and human redistribution
review.

## Current-source M20/M22/M30 release regression — 2026-10-03

On clean source `4fae6875d64752db8fbe0508a932c28da246e8af`, `./nagi m30` rebuilt
the self-contained 64 GiB GPT reference image with SHA-256
`1e81c7a89b4295bcadebfd835d4942ad53849ee1f81be3cb7ff5cc05395f379d`. QEMU run
`1790985901890315000` passed System A boot, User Data persistence across
restart, M19 Search/ObjectId persistence, M22 Activity Ledger/Move/Copy/Undo,
Recovery, unstaged System B rejection, and post-Recovery restart. The separate
M20 Model Store reader fixture passed. All QEMU images pass `qemu-img check`.

Release preflight, assembly to `out/artifacts/m30-release-bundle-4fae687/`, and
verification passed from the clean committed source. All 23 bundle checksums
verify; its reference image is byte-identical to the source image, and the
bundled qcow2 passes `qemu-img check`. The manifest records
`m30_acceptance=NOT_EVALUATED`, which remains correct because this does not
establish authenticated update acceptance or human redistribution approval.

The stale pre-run image was bound to `65f4d6f8773e0b373f237067960738f66e454f3b`.
Its qcow2 and sidecar were preserved at
`out/evidence/m30-stale-image-pre-4fae687-20261003/`, where the image checksum
and qcow2 structure verify. Current run evidence and its 15-file SHA-256
manifest are at `out/evidence/m30-release-1790985901890315000/`. The QEMU host
has no `virtio-sound.in` backend; the guest sound initialization marker passed,
but this adds no microphone/audio-capture evidence.

## Current-source release acceptance — 2026-10-03

On clean source commit `9b16eaae729b8c61403aa929912de5ab5da19d4b`, `./nagi m30`
passed System A initialization, User Data persistence across restart,
Recovery with the boot journal unchanged, rejection of an unstaged System B,
and the post-Recovery System A restart. The separate disposable M20 Model
Store fixture passed its guest FAT32 read check; this does not claim inference.
The 64 GiB GPT reference image SHA-256 is
`e815da59636642c91b06fb6d9f75b038113eabadf7dfd2eb4a251cd61ac2f349`.
QEMU's mutable acceptance copy and the source image pass `qemu-img check`.

The 16 release-tool tests passed. Clean-source preflight, assembly to
`out/artifacts/m30-release-bundle-9b16eaa/`, verification, all 23 bundle
checksums, byte identity between the bundled and source image, and bundled
`qemu-img check` passed. The release manifest correctly retains
`m30_acceptance=NOT_EVALUATED`; integrity verification remains separate from
guest acceptance. Run evidence and its 11-entry verified SHA-256 manifest are
under `out/evidence/m30-release-1790988888019354000/`.

The previous fixed-path image bound to `4fae687` was preserved with its
build-info sidecar under
`out/evidence/m30-stale-image-pre-9b16eaa-20261003/`; its hashes and qcow2
structure verify. QEMU reported that this host has no `virtio-sound.in` input
driver, so this run adds no audio-capture evidence. Authenticated updates and
human binary redistribution review remain open; M30 stays `PARTIAL`.
