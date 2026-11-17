# Linux service broker candidate source preservation

Status: candidate build/test integration. Nothing in this candidate is wired into the maintained Linux init path.

`stage25/` preserves the selected Stage 25 broker, peer, NIPC, process-instance, and runtime-fixture source. The candidate references the Task24 spawn contract and identical lifecycle/reaper helpers from the sibling `lane-c3-sandbox` worktree; duplicate copies of those files were removed. For the checks below, that worktree was at `93424aa679b519c7cfafd11313943b96298200ba` with tracked files clean. The candidate-only manifest points into that sibling checkout, so building this candidate requires the sibling worktree in the expected location. The Task41 owner candidate remains because its exact-cleanup path adds a C2-owned launch-lease check that the current C3 owner does not contain. The candidate is not wired into maintained Linux init.

The supplied candidate reports a Stage 25 Linux guest 11 run and a real clone3/pidfd fixture success. Those earlier results were not rerun here and are not acceptance evidence for this commit.

The original candidate manifests and locks were excluded because they contain workstation-specific absolute dependency paths. This lane adds candidate-only manifests with relative paths into the C3 sibling worktree for the shared contract. Only the Task25/37 Rust sources needed by the direct dependency graph are retained. Generated target/cache trees, ELF/cpio/disk/log outputs, and the private review document were also excluded.

Host verification on the x86_64 macOS target passed 149 unit tests and 15 doc tests. Twelve tests that read the signed Task25 XAPP were filtered because `NAGI_DEV_PREVIEW_PACKAGE` is unset and no `.xapp` is present in this worktree. Reproduce the filtered host run from this directory with:

```sh
RUSTC="$(rustup which --toolchain nightly-2025-08-01-aarch64-apple-darwin rustc)" \
RUSTDOC="$(rustup which --toolchain nightly-2025-08-01-aarch64-apple-darwin rustdoc)" \
CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER=/usr/bin/clang \
rustup run nightly-2025-08-01-aarch64-apple-darwin cargo test --locked --offline \
  --target x86_64-apple-darwin \
  --features task24-owner-co-location,linux-runtime-fixture -- \
  --skip boot_devpreview_owner_is_borrowed \
  --skip package_service_adapter_rejects_generation_change_after_capture \
  --skip package_service_adapter_revalidates_full_manifest_and_signed_grants \
  --skip package_service_adapter_uses_nonblocking_catalog_reads \
  --skip package_service_owner_poison_fails_closed \
  --skip package_service_read_lease_blocks_same_instance_mutation_through_callback \
  --skip serialized_package_owner_records_one_shot_generation_receipt_only \
  --skip boot_package_owner_lease_covers_the_authorized_peer_effect_commit \
  --skip package_generation_effect_wrapper_rejects_required_consent_without_event \
  --skip run_end_revokes_registered_package_receipts_but_keeps_unknown_tombstones \
  --skip supervisor_retirement_revokes_registered_package_receipts_and_releases_terminal_slots \
  --skip task25_receipt_view_is_scoped_to_connection_session_revocation_and_run
```

Linux-target `cargo check --locked --offline --target x86_64-unknown-linux-gnu --tests --features task24-owner-co-location,linux-runtime-fixture` passed; this compiles the Linux adapter and QEMU harness but does not execute them. No Linux guest run was performed.

The canonical Notes codec/client was not obtained, and no broker or Notes provider is enabled; maintained init stays fail-closed with `Unavailable`.
