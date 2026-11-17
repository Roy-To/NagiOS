# Linux service broker candidate source preservation

Status: candidate build/test integration. Nothing in this candidate is wired into the maintained Linux init path.

`stage25/` preserves the selected Stage 25 broker, peer, NIPC, process-instance, and runtime-fixture source. The candidate references the Task24 spawn contract and identical lifecycle/reaper helpers from the sibling `lane-c3-sandbox` worktree; duplicate copies of those files were removed. For the checks below, that worktree was at `93424aa679b519c7cfafd11313943b96298200ba` with tracked files clean. The candidate-only manifest points into that sibling checkout, so building this candidate requires the sibling worktree in the expected location. The Task41 owner candidate remains because its exact-cleanup path adds a C2-owned launch-lease check that the current C3 owner does not contain. The candidate is not wired into maintained Linux init.

The supplied candidate reports a Stage 25 Linux guest 11 run and a real clone3/pidfd fixture success. Those earlier results were not rerun here and are not acceptance evidence for this commit.

The original candidate manifests and locks were excluded because they contain workstation-specific absolute dependency paths. This lane adds candidate-only manifests with relative paths into the C3 sibling worktree for the shared contract. Only the Task25/37 Rust sources needed by the direct dependency graph are retained. Generated target/cache trees, ELF/cpio/disk/log outputs, and the private review document were also excluded.

Host verification on the x86_64 macOS target passed 148 unit tests and 15 doc tests. Twelve tests that read the signed Task25 XAPP were filtered because `NAGI_DEV_PREVIEW_PACKAGE` is unset and no `.xapp` is present in this worktree. Linux-target `cargo check --locked --offline --target x86_64-unknown-linux-gnu --tests --features task24-owner-co-location,linux-runtime-fixture` passed; this compiles the Linux adapter and QEMU harness but does not execute them. No Linux guest run was performed.

The canonical Notes codec/client was not obtained, and no broker or Notes provider is enabled; maintained init stays fail-closed with `Unavailable`.
