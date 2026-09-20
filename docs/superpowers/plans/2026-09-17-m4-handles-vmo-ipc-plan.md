# M4 Handles / VMO / IPC Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add bounded capability handles, VMO/AddressSpace metadata, Channel/Event/Timer/wait primitives, and prove a real Process A -> Process B message with rights attenuation.

**Architecture:** Keep the implementation in the `no_std` `nagi-kernel` library. `ObjectRegistry` owns generation-checked object identities and references, `HandleTable` is process-local authority, Process-gated object methods are the operation boundary, and `ChannelPair` moves capabilities into queue-owned escrow before receive-time installation. M4's guest acceptance calls these same structures from the kernel while M5 remains responsible for user/kernel separation.

**Tech Stack:** Rust nightly pinned by `rust-toolchain.toml`, `core`-only kernel library, existing custom x86-64 kernel target, QEMU/OVMF acceptance through `nagi run`.

## Global Constraints

- Nagi is an independent OS and must not route guest work through the host OS.
- Linux/POSIX compatibility remains user-space and must not be added to the production kernel.
- Rights attenuation must never permit a receiver to strengthen a transferred handle.
- Kernel high-level files, sockets, windows, audio, package, and AI operations remain outside the kernel boundary; M4 adds only kernel authority/IPC primitives.
- No generated/fake response, hard-coded acceptance result, deleted test, or weakened security check may satisfy acceptance.
- All bounded storage uses explicit fixed capacities; no new production dependency is added.

---

### Task 1: Capability handles and independent Process tables

**Files:**
- Create: `kernel/src/handles.rs`
- Modify: `kernel/src/lib.rs`

**Interfaces:**
- `Rights`, `Handle`, `ObjectKind`, `ObjectId`, `ObjectRegistry`, `HandleTable<const N: usize = 32>`, and `Process<const N: usize = 32>` are public library types.
- `HandleTable::insert`, `resolve`, `require`, `close`, `begin_move`, and receive-time `install_token` are the authority paths; object creation and reference changes go through `ObjectRegistry`.

- [x] **Step 1: Write failing tests** for round-trip encoding, stale handles after close/reuse, subset-only attenuation, and full-table rejection.
- [x] **Step 2: Run `cargo test -p nagi-kernel --lib --locked` and verify the new tests fail because the types do not exist.**
- [x] **Step 3: Implement fixed-slot `HandleTable` with generation increment on close and subset-only move checks.**
- [x] **Step 4: Re-run the focused tests and then `cargo clippy -p nagi-kernel --lib --locked -- -D warnings`.**
- [x] **Step 5: Commit `feat: add M4 capability handle tables`.**

### Task 2: VMO and AddressSpace backing

**Files:**
- Create: `kernel/src/vmo.rs`
- Modify: `kernel/src/lib.rs`

**Interfaces:**
- `Vmo::anonymous`, `Vmo::shared`, and Process-gated `map_vmo`, `unmap_vmo`, `protect_vmo`, `read_vmo`, and `write_vmo` provide page-aligned bounded zeroed backing and mapping operations.
- Mapping/protection requests accept only rights already present on the VMO and reject overlapping or unaligned ranges.

- [x] **Step 1: Write failing tests** for page alignment, map/unmap, protection attenuation, overlap, and VMO-right rejection.
- [x] **Step 2: Run the focused kernel tests and verify the expected missing-type failures.**
- [x] **Step 3: Implement the bounded VMO and mapping table using `Rights` from Task 1.**
- [x] **Step 4: Re-run focused tests, full kernel tests, and clippy.**
- [x] **Step 5: Commit `feat: add M4 VMO address-space primitives`.**

### Task 3: Channel, Event, Timer, and wait_many

**Files:**
- Create: `kernel/src/ipc.rs`
- Modify: `kernel/src/lib.rs`

**Interfaces:**
- `ChannelPair::install_endpoints`, `send`, and `receive` connect two process-local endpoint handles; `send` escrows moved capabilities, notifies peer READABLE waiters, and `receive` installs them transactionally.
- `OutgoingMessage`, `ReceivedMessage`, and `MessageHeader` use fixed payload/transfer bounds.
- `Event`, `Timer`, `Waiter`, `WaitRegistry`, `wait`, and `wait_many` expose bounded blocked/runnable registration from typed guest object state without reading host state; signal/expiry dispatches wake records through the registry.

- [x] **Step 1: Write failing tests** for header/payload round-trip, queue-full rejection, endpoint-right checks, Event signal/clear, Timer tick firing, and wait_many index selection.
- [x] **Step 2: Run focused kernel tests and verify they fail before IPC types exist.**
- [x] **Step 3: Implement bounded queues and sender escrow with receive-time transactional installation.**
- [x] **Step 4: Re-run focused tests, full kernel tests, and clippy.**
- [x] **Step 5: Commit `feat: add M4 channel wait primitives`.**

### Task 4: Guest acceptance and developer interface

**Files:**
- Create: `kernel/src/m4.rs`
- Create: `tests/acceptance/m4_handles_vmo_ipc.ps1`
- Create: `tests/acceptance/m4_handles_vmo_ipc.sh`
- Modify: `kernel/src/main.rs`, `tools/nagi-cli/src/commands.rs`, `tools/nagi-cli/src/image.rs`

**Interfaces:**
- `m4::run_acceptance() -> bool` must execute Process A -> Process B channel delivery and reject receiver WRITE escalation.
- Guest serial markers include `Nagi M4 channel round-trip PASS`, `Nagi M4 rights attenuation PASS`, and `Nagi M4 acceptance PASS`.

- [x] **Step 1: Write the acceptance scripts and guest test call with required markers; run them and verify they fail because M4 markers are absent.**
- [x] **Step 2: Implement `m4::run_acceptance` against the real library objects and add the M4 markers in `main.rs`.**
- [x] **Step 3: Update the run wait marker to M4 while preserving M2/M3 marker checks in their scripts.**
- [x] **Step 4: Build kernel/loader, run workspace tests, run both M4 acceptance scripts, and inspect the guest serial log.**
- [x] **Step 5: Commit `feat: add M4 guest handles and IPC acceptance`.**

### Task 5: Milestone verification and status handoff

**Files:**
- Modify: `docs/implementation_status.md`

- [x] **Step 1: Run `cargo fmt --all -- --check`.**
- [x] **Step 2: Run `cargo clippy --workspace --all-targets --exclude nagi-kernel --exclude nagi-loader --locked -- -D warnings` and kernel-library clippy.**
- [x] **Step 3: Run `cargo build --workspace --exclude nagi-kernel --exclude nagi-loader --locked`, both cross-target builds, and `cargo test --workspace --locked`.**
- [x] **Step 4: Run `tests/acceptance/m4_handles_vmo_ipc.ps1` and `.sh` with no concurrent QEMU instances.**
- [x] **Step 5: Only if every M4 criterion passes, record M4 `PASS`, exact commit/evidence, and advance the current milestone to M5.**
