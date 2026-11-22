# Nagi CODEMAP — read this first

Short entry map for AI agents (Codex / Claude / DeepSeek) and new contributors.
Goal: start work without re-reading the whole repository. Snapshot of
`codex/linux-integration-20261117-clean` (PR #163 line) on 2026-11-22; many Linux paths below exist on that branch until #163 merges.
When this file and the code disagree, the code wins; fix this file.

## 1. Read budget (important — these files are huge)

| File | Size | How to read |
|---|---|---|
| `docs/implementation_status.md` | ~720 KB / 11.6k lines | **Never read whole.** Recent entries are mostly near the top (not strictly ordered): read the first ~60 lines and `grep -n` for your lane/task/date. |
| `docs/product-decisions/README.md` | ~116 KB | Do not read whole. `ls docs/product-decisions/` and open only the `DEC-*.md` you need. |
| `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md` | ~80 KB | 0.1 baseline. Read only the section you touch. |
| `AGENTS.md` | ~12 KB | The top "Linux kernel direction" block overrides the older 0.1 text below it ("not Linux-based", strict M0→M30 order). DEC-049: speed over process. |
| `user/nagi-init/candidates/linux-broker-stage25/` | ~30k lines | Candidate code, but Task24 includes parts of it via `#[path]` (pidfd_process_instance, spawn_owned_listener_bridge, initial_login_wire, pid1_live_peer_supplier). Read only the module you touch. |

## 2. Direction in one paragraph

Linux kernel + Nagi-owned userland (DEC-042/043/044/047). PID 1 is `nagi-init`
built for `x86_64-unknown-linux-nagi` (musl). AI stays unprivileged; typed
Public Actions, capability policy and user-data integrity still apply.
Product decisions live in `docs/product-decisions/` (DEC-050); record new user
decisions there. Host-only results are not Linux-image acceptance.

## 3. Goal ① — the end-to-end chain

"Real peer → session → save, with production-authenticated service READY in QEMU"
(DEC-112: Identity as an independent required service; Data/Notes via the PID 1 owner for now).

Owner labels used in code: **A** = init / service-launch receiver,
**C1** = identity & session, **C2** = trusted caller / peer connection,
**C3** = process owner, sandbox, directory handles, login UI app.

```
Linux boot
 └─ PID 1  user/nagi-init/src/linux_init.rs  (12.6k lines; production run() is still fail-closed)
     ├─ Data mount        src/linux_mount_prerequisites/{mod,data_mount_owner}.rs
     ├─ C1 boot recovery  src/linux_c1_boot_recovery.rs
     ├─ C3 launch slot    src/linux_init_c3_system_service_owner.rs (clone3, pidfd, reap)
     │                    src/linux_c3_boot_launch.rs, src/linux_init_c3_slot_journal.rs
     │                    crates/nagi-system-service-launch (A's sequencing contract)
     ├─ C2 endpoint       src/linux_task24_endpoint_launch.rs
     │                    crates/nagi-peer-authenticated-connection (SO_PEERCRED + SO_COOKIE)
     ├─ Login             src/login_screen.rs, src/linux_owner_registration_console.rs,
     │                    src/linux_initial_login_transaction.rs
     │                    user/nagi-linux-account-verifier (password vs stored credential)
     ├─ Session (C1)      crates/nagi-c1-session-runtime (Linux adapter)
     │                    crates/nagi-identity-session (service.rs 14.8k lines; store/linux.rs durable store)
     ├─ Handshake / call  crates/nagi-nipc-v1-stream (78-byte header framing)
     │                    crates/nagi-init-service-call, crates/nagi-service-contract
     ├─ READY             serial line `nagi-init: state=ready`
     └─ Notes save → reboot same disk → readback   user/nagi-notes
```

Standalone crates (own Cargo workspace, excluded from root): `nagi-capability`,
`nagi-identity-session`, `nagi-init-spawn-contract`, `nagi-system-principal-contract`,
`nagi-system-service-launch`, `nagi-service-contract`, `nagi-nipc-v1-stream`,
`nagi-init-service-call`, `user/nagi-init`. Build/test them with
`cargo test --manifest-path <crate>/Cargo.toml`.

**Status (implementation_status.md, 2026-11-19):** BLOCKED at C1 first-owner
provisioning. Registration returns `Identity(StoreUnavailable)`: the C1 journal
is `RuntimeActive` after recovery handoff and the generic recovery guard
quarantines that phase. Session attach, Notes save, reboot readback and READY
are unverified on this path. (2026-11-16 dev-only READY used a development NIPC
fixture and does not count.) Newer serial evidence
(`out/evidence/a-task24-files-consent-regression-20261120/signed/write-serial.log` in
worktree `linux-integration-public-clean-20261117`) reaches session → handshake → `state=ready`,
then Notes save stops at `NAGI_DEV_TASK24_FILES_POLICY_UNAVAILABLE` /
`NAGI_A_FILES_CONSENT_MATCHED_SCOPED_BROKER_UNAVAILABLE` and PID 1 ends in
"fail-closed wait; service readiness is not asserted". Check the top of implementation_status.md for newer entries.

## 4. QEMU evidence markers (Task24)

Defined in `tools/linux-boot/task24_recovery_serials.py`. Expected order in the save phase:
`NAGI_DEV_IDENTITY_DATA_MOUNTED retained_fd=true` → `NAGI_DEV_TASK24_UNLOCK_WAITING_FOR_INPUT`
→ `NAGI_DEV_TASK24_SESSION_ATTACHED` → `NAGI_DEV_TASK24_IDENTITY_HANDSHAKE_COMPLETED`
→ `nagi-init: state=ready` → `NAGI_DEV_NOTES_INPUT_READY` → `NAGI_DEV_NOTES_SAVED object=… revision=…`
→ (read phase) `NAGI_DEV_NOTES_REOPENED …`.
Quick "how far did it get" summary: `python3 tools/linux-boot/serial_progress.py <serial.log>`.

Related tools in `tools/linux-boot/`: `run_qemu_child.py` (QEMU + QMP with hard timeout),
`task24_run_preflight.py`, `task24_recovery_serials.py`, `boot_acceptance.py` (offline checks),
`notes-desktop-dev-20261116/run_persistence_acceptance.py` (dev-only save/readback smoke),
`task24-login/`, `task24-identity-ready/` (build scripts).

## 5. Build and test

- Top level: `./nagi doctor | fetch | build | image | run | test`.
- CI (`.github/workflows/ci.yml`, job `ubuntu-host`): `cargo clippy --workspace --all-targets --exclude nagi-kernel --locked -- -D warnings`,
  `cargo build --workspace --exclude nagi-kernel --locked`, `python3 -m unittest discover -s tests -p 'test_*.py'`,
  `./nagi test --acceptance --host-only --ci`, `./nagi dev verify`, `cargo test --locked -p nagi-albert-linux-tests`.
  Per-crate workflows exist for identity-session, job-scheduler, notification-core, automation-core, etc.
- In the maintainer's SSD workspace, run commands through `nagi-ssd.py exec WORKSPACE -- CMD`
  and create worktrees with `nagi-ssd.py new-worktree NAME REF` (see the workspace-root `AGENTS.md`).

## 6. Where things are

- Maintainer workspace: main clone `Developer/NagiOS` (origin `Roy-To/NagiOS`). Lane worktrees: `worktrees/`, `codex-worktrees/` — use `git worktree list`; the lane ledger is Dot's `board.md`.
- Active lane branches (2026-11-22): `codex/lane-c1-session-ssd-*`, `codex/lane-c2-broker`, `codex/lane-c3-sandbox`,
  `codex/lane-data-service-*`, `codex/lane-linux-mount-prereqs`, `codex/lane-package-fs-guest-*`,
  `codex/lane-qemu-smoke-cli-*`, `codex/lane-system-ro-image-*`, `codex/{calendar,settings,lane-files}-linux-*`,
  integration `codex/linux-integration-20261117-clean` (#163), `codex/pr163-*` repair lanes.
- Other big areas: `tools/nagi-cli` (45k lines, the `./nagi` CLI), `tools/linux-boot` (22k), `kernel/` (0.1 own kernel, legacy),
  `user/nagi-servo` + `third_party/servo` (Albert browser), `services/nagi-ai`, `crates/nagi-*-core` (first-party app cores).
- Decisions: `docs/product-decisions/DEC-*.md` (product), `docs/decisions/NNNN-*.md` (ADRs).

## 7. Reviews

- Auth/session/IPC diffs: a review checklist and baseline findings are kept by the maintainer outside the repository for now.
- Paste `tools/linux-boot/serial_progress.py` output instead of whole serial logs into AI chats.
