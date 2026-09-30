# Nagi release assembly preflight

This standard-library Python tool assembles an already-built Nagi 0.1
reference image and verifies the files in the resulting release directory. It
does not build the OS, run the guest acceptance path, or mark M30 complete.
Every assembled manifest records `m30_acceptance: NOT_EVALUATED`.

Run from the repository root after producing a clean source commit, the kernel
ELF, and a self-contained reference qcow2 image:

```sh
python3 tools/nagi-release/release.py preflight \
  --root . \
  --kernel target/x86_64-unknown-nagi/release/nagi-kernel \
  --image out/artifacts/Nagi-OS-0.1-devpreview.qcow2

python3 tools/nagi-release/release.py assemble \
  --root . \
  --kernel target/x86_64-unknown-nagi/release/nagi-kernel \
  --image out/artifacts/Nagi-OS-0.1-devpreview.qcow2 \
  --output out/release/Nagi-OS-0.1-devpreview

python3 tools/nagi-release/release.py verify \
  --directory out/release/Nagi-OS-0.1-devpreview
```

Assembly fails unless the Git tree is clean, the kernel is a real x86-64 ELF
inside the repository, the qcow2 image is self-contained and has the virtual
size in `nagi.toml`, all required documents exist, and the pinned sources and
tool versions can be recorded. The kernel build ID in the manifest is defined
as `sha256:<digest of the kernel ELF bytes>`; the build manifest also binds
the reference image by SHA-256. The Granite digest is the upstream Q4_K_M pin
from the M20 workstream; that workstream explicitly says
the model bytes are not included.

`./nagi image` and most milestone fixtures still produce raw FAT images with
separate persistent data disks. `./nagi m30` uses a dedicated writer to
produce the self-contained 64 GiB GPT qcow2 with ESP, System A/B, User Data,
Recovery, and Model Store partitions. Use that image as the release input; do
not convert an M-stage test image and treat it as the release layout.

Required source documents are `RELEASE_NOTES.md`, `THIRD_PARTY_NOTICES.md`,
all checked-in `docs/architecture/*.md` files, `sdk/README.md`,
`CONTRIBUTING.md`, and `ROADMAP.md`. The output contains those inputs, the
qcow2 image, a source revision record, a deterministic build manifest, a
release artifact index, and `SHA256SUMS`. The sums file covers every output
file except itself. Re-running with byte-identical inputs and the same
recorded tool versions produces byte-identical manifests and sums.

`verify` only checks artifact presence, provenance fields, and hashes. A real
QEMU boot and complete M30 demo acceptance must be run and reviewed separately
before anyone can claim a release is ready.
