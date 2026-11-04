# Ordinary Files probe artifact check

`check_ordinary_files_probe.py` checks a trusted local QEMU collector's bounded
Files probe. It is deliberately separate from formal M30 acceptance. It does
not execute guest operations, synthesize session events, prove inference or
distribution acceptance, or register a replacement CI gate.

Supply four **stopped** QEMU runs in order: fresh format, ordinary signed-in
Files lifecycle, restart, and offline boot. Each directory contains the pristine
image reference/source and start/end disk hashes in `result.json`, the exact
reference-machine command in `command.json`, the mutable `disk.qcow2`, serial
log, captured PPM frames and input transcript. The collector must stop QEMU
before copying its disk and OVMF variables into the next run. A live qcow2 file
copy is not a restart snapshot.

```sh
python3 tests/acceptance/check_ordinary_files_probe.py --root . \
  out/evidence/FORMAT out/evidence/SESSION \
  out/evidence/RESTART out/evidence/OFFLINE > out/files-probe-chain.json
```

The current probe covers ordinary Owner sign-in, create/rename/create/Trash/
restore through the Files panel, presented signed Search grants, child cleanup,
Bar failure and cleared state after lock/relogin, persisted Files/Search after
restart, and Files/Search with the VM network link disabled before guest code
runs. Screenshots still require review of the actual filename/result count and
Bar state. The script checks artifact consistency and observed serial markers;
it cannot establish collector honesty or replace that review.

The JSON report binds source, image, disk chain, commands, input and frames. A
mixed source/image, changed disk snapshot, live/incomplete probe or broken hash
chain is rejected. Its output retains `M30: BLOCKED`, inference and ObjectId/
content-hash acceptance `NOT_EVALUATED`. Trash/restore is not History/Undo, and a
real worker's failure reply is not successful model inference. The complete
ordinary session, resident Browser/History/voice/stress and distribution gates
remain required.
