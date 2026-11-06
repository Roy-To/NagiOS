# Nagi 0.1 session and release preflight

The normal image producer is:

```sh
./nagi session-image [path/to/pinned/Granite.gguf]
```

It builds a self-contained GPT qcow2 with Recovery, owner first-run login and
one signed product Files/Search client. It does not embed acceptance packages,
an owner password, or private User Data. Optional Granite input must match the
existing size/hash pin before and after image creation. Its current profile is
`signed-in-files-v1`: model presence does not establish resident AI inference.
Servo, History/Undo, voice, simultaneous stress and model distribution are open.
This image is a partial development session and cannot be assembled as 0.1.

`./nagi m30-layout` preserves the old System A, persistence, Recovery, unstaged
B rejection and Model Store reader fixtures, with their own image filename.
`./nagi m30` reports the unavailable formal release gate as a failure. Neither
command converts individual milestone evidence into a complete ordinary session.

Version 2 `.qcow2.build-info` records source revision, pristine image digest,
profile, init features, integrated components and Model Store model digest.
Preflight/assembly reject legacy layout records, partial profiles, fixture
features, incomplete component inventories and absent/wrong Granite bytes. A
future implemented `production-session-v1` configuration is required. The
production component inventory is login/files/search/servo/local-ai/history/voice.
This is a configuration check; source/profile hashes never prove runtime behavior.

```sh
./nagi session-acceptance evidence/session.json out/artifacts/production.qcow2
python3 tools/nagi-release/release.py preflight --root . \
  --kernel target/x86_64-unknown-nagi/release/nagi-kernel \
  --image out/artifacts/Nagi-OS-0.1-devpreview.qcow2
python3 tools/nagi-release/release.py assemble --root . \
  --kernel target/x86_64-unknown-nagi/release/nagi-kernel \
  --image out/artifacts/Nagi-OS-0.1-devpreview.qcow2 \
  --output out/release/Nagi-OS-0.1-devpreview
python3 tools/nagi-release/release.py verify \
  --directory out/release/Nagi-OS-0.1-devpreview
```

Session evidence validation is a separate runtime gate and still does not assert
M30 distribution acceptance. Assembly records `m30_acceptance: NOT_EVALUATED`
and `distribution_review: NOT_EVALUATED`. It is local preparation, not publication
or legal approval. A full clean source tree, x86-64 kernel ELF, self-contained
reference-size qcow2, matching current-source profile provenance, pinned sources,
tool versions and required documents are required.

The bundle includes pristine image, source revision, build manifest, architecture
and SDK docs, contribution guide, roadmap, release notes, notices and SHA256SUMS.
Tracked third-party license texts are copied with source paths and hashes; this
inventory excludes ignored/fetched caches and is insufficient for complete
linked Rust/native/model/font redistribution review. Project license, source
offers, extra model packages and Gemma terms remain separate decisions.

Use disposable image copies for VM checks; guest writes change the image hash.
Preserve the assembled image untouched and verify it again after validation.
