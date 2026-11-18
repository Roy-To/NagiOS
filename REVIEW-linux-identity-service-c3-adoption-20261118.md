# A receiver / C3 endpoint adoption review — 2026-11-18

## Resume the first-owner screen — 2026-11-19

Use `out/evidence/a-owner-registration-ui-20261119/normal-registration.cpio` from A commit `1ca5daa7a` and the pinned `linux-boot-userland-20261115/tools/linux-boot/kernel-dev-20261115/kernel-fbdev-6.18.63-r1/bzImage` from the sibling boot worktree. The prepared Data source is `out/evidence/a-dev-identity-provision-20261119/development-copy/data.raw`. For persistent registration, attach a new private copy in an A-owned run directory and keep the source image detached. Reuse the copy only after a clean, known registration outcome.

The verified QEMU devices are q35 with TCG and `i8042=off`, 4 vCPUs, 8 GiB, `virtio-vga,xres=640,yres=480,edid=on`, and `virtio-keyboard-pci`. Attach Data as a raw Virtio block drive with serial `NAGI-DATA-1f89b8bf`. Use a visible QEMU display and append the following guest arguments.

```text
console=ttyS0,115200n8 rdinit=/init panic=-1
nagi.dev_data.boot=identity nagi.dev_data.serial=NAGI-DATA-1f89b8bf
nagi.dev_data.fs_uuid=1f89b8bf-cf0f-42cd-a6bb-2397dcc0c0be
nagi.dev_data.accounts=accounts nagi.dev_data.snapshots=snapshots
```

Wait for `NAGI_LINUX_OWNER_REGISTRATION_WAITING_FOR_INPUT`. Select the language, enter the owner name and password twice directly in the guest, and press Enter. The verified run pressed Enter at language selection only. Its screenshots and serial log are under `out/evidence/a-owner-registration-ui-20261119/`. Registration save, Session, Notes, and READY are pending.

## Task24 maintained-init assembly — 2026-11-19

The opt-in `linux_task24_owner_co_location` assembly compiles the unmodified C2 `8178f7d34e` module and gate helper beside C3 `cdfc2e87fb` (provenance `2501ea247`) in the normal PID 1 crate. The same C2 type reaches first-login checkout and `ProcessLifecycle<LinuxProcessOs, ServiceId, LaunchRecord, Task24EndpointLaunchLease<LaunchRecord>>`. The assembly retains the listener before the exec gate, lets C3 supply the live child's attestation after exec acknowledgement, activates the lease under C2's existing owner checks, and borrows it from the same slot. An activation error goes through C3's exact termination and C2's slot retirement callbacks.

The combined Linux musl build used the fixed C2 sources once, with `c2-issuer` on the canonical proof leaf and the exact C3 source patches in the isolated graph. A normal signed QEMU boot of that build reached C1 recovery, C3 continuation and the registration screen. The prepared Data hash matched before and after. See `out/evidence/a-task24-colocation-20261119/normal-combined-serial.log`.

The Task24 launch function has no normal caller yet. It requires a trusted installed login child and its pre-session route, the production consent source, and C2's sealed `Pid1ReadinessAtomicBoundary` implementation. The selected signed boot package supplies C1 recovery, and no production readiness implementation is present in fixed C2. Therefore this boot did not execute Task24's clone3/exec acknowledgement, lease activation, lease borrow, or initial checkout.

## Development Task24 login package — 2026-11-19

The development image selects `org.nagi.dev.task24-login` with a package SHA-256 pin at build time. PID 1 restores the installed index with the authenticated boot catalog's package signing key, verifies the package ID, canonical entry and exact installed executable, and retains its package generation. The Task24 launch function now builds the exact C3 launch and registers the Supervisor declaration from that retained package. The minimal freestanding child holds the inherited Unix control endpoint and has no credential input until the sandboxed input and broker transports are supplied. The existing development signer signs the package, and the C1 recovery catalog and signer input are byte-identical to the prior development image.

Normal QEMU boot verified the installed login package at generation 2 and reached first-owner input wait. It did not launch the Task24 child or submit credentials. C3's private tmpfs root does not expose the pathname used by C2's child bootstrap at `/run/nagi/ipc`. The route needs a jointly owned C2/C3 transport that preserves the child sandbox, in addition to C2's sealed readiness provider. The existing trusted consent UI records decisions only from an authenticated unlocked user. No consent event or connected-consent provider was fabricated for pre-session activation. See `out/evidence/a-task24-login-package-20261119/final/normal-serial.log`.

The A call path supplies C2 inventory checks from the retained and revalidated PackageService generation. Supervisor resolve and commit use the current launch registry and its existing grant lock. The first checkout can consume the already recovered C1 issuer, verifier, and matching boot handoff without a second recovery. The call still requires the canonical pending endpoint, consent connection, and readiness gate. The combined development build and normal boot are recorded under `out/evidence/a-task24-installed-owner-20261119/`.

Normal PID 1 now retains one `Pid1RunOwnerLease` and its directory, endpoint registry, and pidfd authority. The Task24 launch adapter borrows that run and a real OS-owned console connection. Connectivity requires a presented frame and live framebuffer/keyboard descriptors. The shared consent dialog accepts fresh key input only for a pending Supervisor request and an authenticated, unlocked Session. The console returns its answer to the existing owner and issues no grant. The normal QEMU run observed the retained run, connected console, and registration input wait without displaying a consent request or running a Task24 child. See `out/evidence/a-task24-consent-console-20261119/normal-serial.log`.

For the final Ready consumer, A exposes `with_ready_session` on the retained C1 issuer and `authorize_launch_set_and_commit` on the Supervisor. C2 can enter the exact C1 Session guard, then the same run's readiness boundary, then have the Supervisor validate the complete mandatory launch set and consume the Ready event under one registry lock. The final consumer must not reacquire any of these locks. The production C2 boundary and final consumer wiring are pending.

The first-login bootstrap has no dependency on an existing Session. ADR 0063 places owner login before the desktop and its capability consent flow. In fixed C2, `register_pid1_spawn_pre_session` registers the exact child with `session = None` and checks the real console connection. `accept_and_issue_initial_session_until` then checks the same socket and child before asking C1 to issue and attach the first Session. A now constrains this path to the pinned login package with zero declared grants. The pending listener and pre-Session requirements are bound from the same trusted route supplied by the route owner, with no application capability grants or user consent decisions during bootstrap. The authenticated Session and Supervisor consent queue remain required for later capability prompts.

No new product decision is needed for this sequence. The canonical initial-login route record, C2's run-bound readiness implementation, and a C3-to-C2 endpoint handoff that preserves the child's isolated root are still required for a live Task24 run. The normal QEMU regression on the restricted package path remains a registration-wait check. See `out/evidence/a-task24-pre-session-route-20261119/normal-serial.log`.

## Decision

Accept C3 commit `4db0a45a96878ecc1dabad4bb4d8b9d3d3dc02a4` as the transport and lifetime API reference. Do not merge or copy its `linux_init.rs` implementation into A yet, and do not enable an endpoint callsite. The fixed commit provides an opt-in child endpoint handoff, but it does not provide a production SystemService proof producer or a trusted service slot. A's current product source has no SystemService catalog record or launch callsite to connect.

This is a fail-closed adoption decision, not Linux production acceptance. No C3, C1, E, shared board, or public branch was modified.

## Inputs checked

- C3 worktree HEAD was `4db0a45a96878ecc1dabad4bb4d8b9d3d3dc02a4`, branch `codex/lane-c3-sandbox`. At inspection, `REVIEW.md`, `user/nagi-init/src/linux_init.rs`, and `user/nagi-init/src/pid1_child_reaper.rs` had uncommitted changes, with additional untracked files. Only the fixed commit tree was used for API review. No file in that worktree was edited.
- `git show --stat 4db0a45a96` shows the fixed commit changes C3 `REVIEW.md` and `user/nagi-init/src/linux_init.rs`. The C3 review reports 37/37 focused process-resource guest tests and 5/5 endpoint-consumer guest tests. It separately reports that the maintained init build is blocked by missing third-party manifests, production callsites use `ProcessEndpointPolicy::Disabled`, the C3 proof producer is not wired into the maintained process owner, and reaper coordination with PID 1's `wait4(-1)` has no concurrent guest acceptance.
- The A-local C3 contract snapshot in commit `24e3cf3eddb4bac43cd80c135236b6dc632b0e44` is byte-identical to C3 source commit `5b257f139fade408bc0d4d5e9df19c680be08701`. This provides the `SpawnedProcessPidfd` and proof v2 types as canonical local path crates; it does not import C3's process owner.

## API comparison

- C3 `LinuxProcessOs` uses `clone3(CLONE_PIDFD)`, retains the pidfd, and fails closed if an atomic pidfd is unavailable. Endpoint inheritance is opt-in. The child exposes its Unix endpoint at fd 3 and transfers the parent endpoint over the exec gate with one `SCM_RIGHTS` message; the parent validates framing and kernel peer credentials.
- `ProcessLifecycle::take_live_endpoint` first requires the exact current launch record and a non-reaped child, checks the child non-consumingly, then transfers the endpoint once and duplicates the lifecycle-owned pidfd. In the fixed C3 source, `ProcessEndpointResources` and `take_live_endpoint` are private to `linux_init.rs`; they are not a callable adapter API for A's receiver crate.
- The fixed C3 production source does not call `SystemServiceLaunchRecord::after_supervisor_record_validation` or `SystemServiceEndpointOwnerProof::authenticate`. Those calls occur in the `#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]` principal test module. The unsafe launch-record constructor remains correctly restricted to a C3 caller that first validates an exact live SystemService slot.
- C3's endpoint handoff authenticates a connected process peer; by itself it does not establish the Nagi SystemService role, trusted catalog mapping, package signer, or C1 private seal.

## A-side connection check

- A's receiver commit `24e3cf3` defines `TrustedSystemServiceCatalog`, `SystemServiceLaunchOwner`, and `C1SystemServiceRecoveryClient` ports. There is no concrete implementation of those ports in `user/nagi-init`.
- A's normal Linux `run()` selects `UnconnectedBackend` at `user/nagi-init/src/linux_init.rs:359-385`. Its only process lifecycle is in the feature-gated development preview path, which uses a fixture account, an interactive `Session`, and app `LaunchRecord` values. `user/nagi-init/src/supervisor.rs` documents and stores `ProcessId -> LaunchRecord` for applications; it has no SystemService role or trusted service catalog.
- Neither `user/nagi-init/Cargo.toml` nor `user/nagi-init-linux-tests/Cargo.toml` depends on `nagi-init-spawn-contract`, `nagi-system-principal-contract`, or `nagi-system-service-launch`. The standalone A receiver crate already depends on the canonical local `nagi-system-principal-contract` path. Adding unused dependencies to the product manifest would not connect a process owner or create a valid SystemService slot, so no manifest change was made.
- A's product input audit remains unchanged: the only first-party app manifest is `org.nagi.files.manifest`; no production Identity service manifest/catalog entry, executable digest, UID/GID mapping, or production signing key is selected. The pinned package key is explicitly a Developer Preview RFC 8032 test vector. No fixture value was promoted.

## Package-trust consumer baseline

- Read the fixed package consumer harness at commit `ef31dfb72b5e325f71487f16282e741b70ec4727` (parent `63b1bdee3b7107b2c75e08313ff7bfa835b955a5`), its `REVIEW.md`, `run.json`, and serial log. `run.json` records one ordered start/suite/exit marker set for the same nonce, 3 passed / 0 failed, QEMU `isa-debug-exit` status 33, no timeout, 3.764 seconds total against a 600-second cap. The serial log contains the same markers and all three test names.
- The harness runner verifies the pinned `Cargo.lock`, consumer library, and test source byte-for-byte against `63b1...`, and checks the public package/provider/model API tree is unchanged. A read-only `git diff --exit-code` of those three pinned files between `63b1...` and `ef31...` returned 0; current SHA-256 values match `run.json`.
- This is useful Linux guest evidence for the unchanged public `nagi-package` and `nagi-init-package-provider::package_trust` APIs used by A commit `24e3cf3`. It does not execute A's `bind_installed_service_package` launch boundary, nor does it prove product signing trust, a canonical catalog, SystemService role, authentication, authorization, or `READY`.
- Do not reuse this runner as an A launch-boundary test: it rejects any consumer source or dependency change from the `63b1...` fixture. A-specific Linux runtime coverage would need a separately scoped runner pinned to the A receiver sources and dependencies. No file in the package-consumer lane was changed.

## Owner boundaries and remaining gates

- C3's current dirty `pid1_child_reaper.rs` work is not adopted. The fixed C3 review says the 37/37 module run did not run concurrently with PID 1's `wait4(-1)` orphan reaper. The active C3 reaper change has owner-reported host 7/7 only and needs its own Linux guest ownership/race result.
- C1 owns its private SystemService seal/recovery adapter and is currently repairing P2, per the owner report. Candidate `001e3e0f24326419d5c2ba98382e57621f592a28` and its guest-118 result do not establish the current gate. A did not copy or adapt C1 source. Exact generation, revocation, and recovery-race semantics remain required.
- E's panic-after-partial-effect `OutcomeUnknown` and reconcile-before-retry fix remain outstanding. The existing Notes C1 glue stays disabled; A did not wire it into normal saves.
- Production service `READY` remains unconnected. Safe continuation requires a C3-maintained public/safe adapter that issues the exact proof from the validated launch slot, C3 guest acceptance for the shared wait/reaper ownership, C1's reviewed fixed seal adapter, and an owner-approved production catalog/package/trust/credential record. A's receiver can then be connected at the real init callsite without using the app fixture.

## Command/result record

- `git -C <C3> status --short --branch` — read-only result showed C3 branch `codex/lane-c3-sandbox`, ahead 7, with the three relevant tracked dirty paths noted above. No write was made there.
- `git -C <C3> show --stat --oneline 4db0a45a96` — `REVIEW.md` and `user/nagi-init/src/linux_init.rs` only.
- `git -C <C3> show 4db0a45a96:user/nagi-init/src/linux_init.rs | rg -n 'ProcessEndpointPolicy|take_live_endpoint|clone3_pid1_service_child|SCM_RIGHTS'` — confirmed opt-in endpoint API and private one-shot handoff.
- `git -C <C3> grep -n -E 'SystemServiceEndpointOwnerProof::authenticate|SystemServiceLaunchRecord::after_supervisor_record_validation' 4db0a45a96 -- user/nagi-init/src` — all matches are in `linux_x86_64_process.rs`'s test-only principal module; no production proof issue callsite.
- A source checks: `rg -n -C 3 'UnconnectedBackend::new|LinuxDevPreviewBackend::new' user/nagi-init/src/linux_init.rs` — normal path is unconnected; preview is separately gated. `rg -n -i 'trusted.*catalog|system_service|identity service' user/nagi-init/src` — no production catalog/Identity service callsite.
- A contract source comparison: `git diff --exit-code 5b257f139fade408bc0d4d5e9df19c680be08701 -- crates/nagi-init-spawn-contract crates/nagi-system-principal-contract` — exit 0, byte-identical.
