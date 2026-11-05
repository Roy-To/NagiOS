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

Read-only machine-readable inventory and preflight:

```sh
python3 tools/nagi-release/m30_preflight.py --root . \
  --revision 65108d6db2ec3e0a6f793c31fc0dc36e27be5354 \
  --image out/artifacts/Nagi-OS-0.1-devpreview.qcow2 \
  --evidence evidence/session.json \
  --bundle out/release/Nagi-OS-0.1-devpreview > /tmp/m30-preflight.json
```

Replace the revision with the full expected source commit when checking another
checkout. These paths are the defaults, except `--root` and `--revision`, which
are required. Select one preserved input set inside the root; there is no search
through unrelated runs or workers' outputs. The collector only reads files and
writes its JSON report to stdout; it does not build, run QEMU, fetch artifacts,
create receipts, copy evidence, or modify the formal `./nagi m30` gate.

Report schema version 1 contains `checks` and a sorted `artifacts` inventory
(relative path, actual size and SHA-256). Check statuses are `MISSING`, `INVALID`,
or `NOT_VERIFIED`, with a reason. `content_integrity=VALIDATED_CONTRACT` means
the existing image/session/release content validator accepted the supplied
bytes; `FILE_HASHED` means only a nonempty distribution file was inventoried.
`NOT_CHECKED` means content validation did not finish or no applicable contract
exists. Each required distribution input is checked independently, then the
whole bundle is validated, including its source and selected image bindings.
Session validation retains its ordered events, log hashes, one session/image/
disk identity, restart persistence, offline and stress negative controls.
Missing referenced phase logs are identified before full session validation.
Duplicate JSON keys and nonfinite numbers are rejected in bundle manifests too.
Each validator reports its first failure; fix it and rerun to expose later faults.

All content-consistent inputs remain `NOT_VERIFIED` for acceptance: hashes and
self-declared profiles do not authenticate a producer, signed-in guest, model
execution or image contents. In particular, the existing session producer API
is proposed and its synthetic tests prove validator orchestration only. CLI run
receipts record host command outcomes; they cannot replace session evidence.
The collector separately reports unavailable execution-origin proof, the lack
of an independent voice-operation evidence contract, and distribution review.
It never invents signatures, timestamps, credentials or guest results. Its JSON
always says `m30_acceptance=NOT_EVALUATED`, `formal_gate=BLOCKED`, and its exit
code is always **1**, including with a consistent synthetic fixture. This is a
diagnostic command, not a replacement CI or milestone gate. Use stable preserved
inputs: the collector does not lock them against concurrent modification.

Regression commands (host only):

```sh
python3 -m unittest discover -s tools/nagi-release -p 'test_m30_preflight.py' -v
python3 -m unittest discover -s tools/nagi-release -p 'test_*.py' -v
```
