# A receiver / C3 endpoint adoption review — 2026-11-18

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
