# M6 Init, Supervisor, and Service Registry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the bounded user-space M6 supervisor and service registry and prove that a real guest client discovers and calls `echo@1` through it.

**Architecture:** Add fixed-capacity `ServiceRegistry` and `Supervisor` types to `libnagi`, keeping service discovery and lifecycle policy out of the kernel. `nagi-init` will register a real echo handler, resolve it by name/version, call it through a generation-checked handle, verify the returned bytes, and then use the existing M5 process-exit syscall. The successful exit boundary emits the terminal M6 marker after the M5 exit markers, so the CLI waits for a result that is contingent on the real user-space check. Acceptance scripts validate the complete guest-generated sequence.

**Tech Stack:** Rust nightly-2025-08-01, `no_std` user library, fixed-size arrays, existing Nagi console/process-exit syscalls, QEMU x86-64/q35/UEFI/4 vCPU/8 GiB, PowerShell and Git Bash acceptance scripts.

## Global Constraints

- Nagi remains an independent OS; the service registry and supervisor are user-space components.
- Do not add Linux/POSIX production dependencies or high-level service/manifest syscalls to the kernel.
- Do not use host output, fake responses, hard-coded acceptance results, or disabled/weakened tests.
- Service identities are name bytes plus API version; clients do not depend on PIDs or executable names.
- Registry storage is fixed-capacity: eight services, four dependencies per manifest, and bounded request/response buffers.
- A resolved service handle is generation-checked; receiver operations cannot strengthen authority or revive stale entries.
- Restart policy is finite; exceeding the configured budget enters `CrashLoop` and never creates an infinite restart storm.
- Existing M5 user/kernel, SYSCALL/SYSRET, FPU, capability, and real-QEMU acceptance boundaries must continue to pass.

---

### Task 1: Define the fixed-capacity service protocol and registry

**Files:**
- Create: `user/libnagi/src/service.rs`
- Modify: `user/libnagi/src/lib.rs`
- Test: `user/libnagi/src/service.rs`

**Interfaces:**
- `ServiceId::new(name: &[u8], version: u16) -> Option<ServiceId>` creates a bounded identity.
- `ServiceManifest::new(id: ServiceId, dependencies: &[ServiceId], restart: RestartPolicy) -> Option<ServiceManifest>` creates a bounded manifest.
- `ServiceRegistry::new() -> ServiceRegistry` creates an empty registry.
- `ServiceRegistry::register(manifest: ServiceManifest, handler: ServiceHandler) -> Result<ServiceHandle, RegistryError>` registers one endpoint.
- `ServiceRegistry::resolve(id: ServiceId) -> Result<ServiceHandle, RegistryError>` finds an endpoint by API identity.
- `ServiceRegistry::call(handle: ServiceHandle, request: &[u8], response: &mut [u8]) -> Result<usize, RegistryError>` calls only a live, available endpoint.

- [x] **Step 1: Write failing registry tests**

Add tests for the not-yet-existing interfaces:

```rust
#[test]
fn resolves_and_calls_a_registered_echo_handler() {
    let id = ServiceId::new(b"echo", 1).expect("id");
    let manifest = ServiceManifest::new(id, &[], RestartPolicy::Never).expect("manifest");
    let mut registry = ServiceRegistry::new();
    let handle = registry.register(manifest, echo_handler).expect("register");
    let resolved = registry.resolve(id).expect("resolve");
    let mut response = [0; 16];
    let size = registry.call(resolved, b"nagi", &mut response).expect("call");
    assert_eq!(&response[..size], b"nagi");
    assert_eq!(handle, resolved);
}

#[test]
fn stale_handles_and_duplicate_identities_are_rejected() {
    let id = ServiceId::new(b"echo", 1).expect("id");
    let manifest = ServiceManifest::new(id, &[], RestartPolicy::Never).expect("manifest");
    let mut registry = ServiceRegistry::new();
    let handle = registry.register(manifest, echo_handler).expect("register");
    assert_eq!(registry.register(manifest, echo_handler), Err(RegistryError::Duplicate));
    assert!(registry.unregister(handle).is_ok());
    let mut response = [0; 4];
    assert_eq!(registry.call(handle, b"x", &mut response), Err(RegistryError::StaleHandle));
}
```

The test module defines only a deterministic `echo_handler` fixture; it must not invoke the host OS or a syscall instruction.

- [x] **Step 2: Run the focused tests and verify they fail**

Run `cargo test -p libnagi service::tests --locked`. Expected: compilation fails because the service module and interfaces do not exist.

- [x] **Step 3: Implement the registry**

Use `[Option<Entry>; 8]`, a monotonically incremented nonzero `u16` generation, and byte-wise `ServiceId` equality. Reject names longer than 32 bytes, zero versions, duplicate identities, capacity overflow, requests over 256 bytes, response buffers too small for the handler, stale handles, and `Failed`/`Restarting`/`CrashLoop` health. `unregister` must invalidate the generation before reusing a slot. The handler receives only the bounded request and response slices.

- [x] **Step 4: Run focused and full library tests**

Run `cargo test -p libnagi service::tests --locked` and `cargo test -p libnagi --locked`; expected result is all focused registry tests and the published syscall-number test passing.

- [x] **Step 5: Commit**

```text
git add user/libnagi/src/lib.rs user/libnagi/src/service.rs
git commit -m "feat: add bounded user service registry"
```

### Task 2: Implement supervisor manifests, dependency order, and restart health

**Files:**
- Modify: `user/libnagi/src/service.rs`
- Test: `user/libnagi/src/service.rs`

**Interfaces:**
- `Supervisor::new() -> Supervisor` creates an empty lifecycle table.
- `Supervisor::start_in_dependency_order(manifests: &[ServiceManifest], order: &mut [ServiceId]) -> Result<usize, SupervisorError>` returns a deterministic topological order and records `Starting` states.
- `Supervisor::mark_ready(id: ServiceId) -> Result<(), SupervisorError>` transitions `Starting` to `Ready`.
- `Supervisor::mark_healthy(id: ServiceId) -> Result<(), SupervisorError>` transitions `Ready` to `Healthy`.
- `Supervisor::record_failure(id: ServiceId) -> Result<ServiceHealth, SupervisorError>` applies finite restart policy and returns `Restarting` or `CrashLoop`.
- `Supervisor::health(id: ServiceId) -> Result<ServiceHealth, SupervisorError>` returns the current state.

- [x] **Step 1: Write failing lifecycle tests**

Add tests covering dependency order, missing dependencies, cycles, valid health transitions, and restart exhaustion:

```rust
#[test]
fn orders_dependencies_before_dependents_and_enters_healthy_state() {
    let base = ServiceId::new(b"base", 1).expect("base");
    let echo = ServiceId::new(b"echo", 1).expect("echo");
    let manifests = [
        ServiceManifest::new(echo, &[base], RestartPolicy::OnFailure { max_restarts: 2 }).expect("echo"),
        ServiceManifest::new(base, &[], RestartPolicy::Never).expect("base"),
    ];
    let mut supervisor = Supervisor::new();
    let mut order = [ServiceId::empty(); 2];
    assert_eq!(supervisor.start_in_dependency_order(&manifests, &mut order), Ok(2));
    assert_eq!(order, [base, echo]);
    supervisor.mark_ready(base).expect("base ready");
    supervisor.mark_healthy(base).expect("base healthy");
    supervisor.mark_ready(echo).expect("echo ready");
    supervisor.mark_healthy(echo).expect("echo healthy");
    assert_eq!(supervisor.health(echo), Ok(ServiceHealth::Healthy));
}

#[test]
fn restart_budget_ends_in_crash_loop() {
    let id = ServiceId::new(b"echo", 1).expect("id");
    let manifest = ServiceManifest::new(id, &[], RestartPolicy::OnFailure { max_restarts: 2 }).expect("manifest");
    let mut supervisor = Supervisor::new();
    let mut order = [ServiceId::empty(); 1];
    supervisor.start_in_dependency_order(&[manifest], &mut order).expect("start");
    assert_eq!(supervisor.record_failure(id), Ok(ServiceHealth::Restarting));
    assert_eq!(supervisor.record_failure(id), Ok(ServiceHealth::Restarting));
    assert_eq!(supervisor.record_failure(id), Ok(ServiceHealth::CrashLoop));
}
```

- [x] **Step 2: Run tests and verify the new lifecycle tests fail**

Run `cargo test -p libnagi service::tests --locked`; expected: compile/test failure for the absent supervisor API.

- [x] **Step 3: Implement deterministic lifecycle state**

Use fixed arrays and a Kahn pass: append any manifest whose dependencies are already in `order`, repeat until all are placed, and return `MissingDependency`, `DependencyCycle`, or `Capacity` when no valid next node exists. Enforce `Starting -> Ready -> Healthy`, set `Failed` before applying restart policy, and make restart count saturating and bounded by `max_restarts`.

- [x] **Step 4: Run tests and commit**

Run `cargo fmt --all -- --check`, `cargo test -p libnagi --locked`, and `cargo clippy -p libnagi --all-targets --locked -- -D warnings`; expected: all pass. Commit with `git commit -am "feat: add bounded service supervision"`.

### Task 3: Integrate a real `echo@1` service call in `nagi-init`

**Files:**
- Modify: `user/nagi-init/src/main.rs`
- Modify: `user/libnagi/src/service.rs`
- Test: `user/libnagi/src/service.rs` and the M6 guest acceptance script in Task 4

**Interfaces:**
- `nagi-init` owns `ServiceRegistry` and `Supervisor` values in user space.
- The `echo_handler(request, response)` copies a request into the bounded response and returns its length; it is not a canned response.

- [x] **Step 1: Add a failing integration contract test**

Add a host-testable registry test that registers the exact `echo@1` identity, resolves it, calls it with `b"nagi"`, and asserts byte-for-byte echo behavior. The test must not call `syscall` or use host I/O.

- [x] **Step 2: Implement the guest flow**

After the existing M5 FPU round-trip check and before `libnagi::exit(0)`, construct the manifest, start it in dependency order, mark it ready/healthy, register the handler, resolve the returned handle, call it with `b"nagi"`, and compare the response. Emit these messages through `libnagi::console_write` using the existing static-message/RIP-relative pattern:

```text
Nagi M6 supervisor START
Nagi M6 manifest dependency order PASS
Nagi M6 service health PASS
Nagi M6 service registry START
Nagi M6 echo@1 call PASS
```

The user process emits these progress markers and exits only after the
verified call. The existing kernel process-exit boundary then emits
`Nagi M5 syscall PASS`, `Nagi M5 acceptance PASS`, and finally
`Nagi M6 acceptance PASS`.

On any error, emit `Nagi M6 acceptance FAIL` and use the existing nonzero process-exit path. Do not print the progress PASS markers until the registry returned the verified echo response; the terminal acceptance marker is emitted by the successful process-exit boundary.

- [x] **Step 3: Build the real user ELF and inspect it**

Run `cargo build -p nagi-init --target targets/x86_64-unknown-nagi-user.json '-Zbuild-std=core,compiler_builtins' --release --locked` and inspect the resulting ELF entry and program headers with `llvm-readobj --file-headers --program-headers`. It must remain an ELF64 executable at the fixed user base and within the M5 eight-page image bound.

- [x] **Step 4: Run focused tests and commit**

Run `cargo test -p libnagi --locked` and the user-target build again. Commit with `git add user && git commit -m "feat: run echo service through user registry"`.

### Task 4: Gate the CLI and add M6 real-QEMU acceptance

**Files:**
- Modify: `tools/nagi-cli/src/image.rs`
- Modify: `tools/nagi-cli/src/commands.rs`
- Create: `tests/acceptance/m6_init_supervisor_registry.ps1`
- Create: `tests/acceptance/m6_init_supervisor_registry.sh`
- Modify: `tests/acceptance/m5_first_user_process.ps1`
- Modify: `tests/acceptance/m5_first_user_process.sh`

**Interfaces:**
- `nagi run` waits for `GUEST_ACCEPTANCE_MARKER = "Nagi M6 acceptance PASS"` after the M6 guest is integrated.
- M6 scripts invoke the existing `nagi run` flow and inspect `out/logs/m1-qemu-boot.log`; they never inject guest lines.

- [x] **Step 1: Add a failing marker-gate test**

Change the image unit test to assert `guest_reached_acceptance("Nagi M5 acceptance PASS\r\n") == false` and `guest_reached_acceptance("Nagi M6 acceptance PASS\r\n") == true`, then run the focused marker-gate test and record the expected failure before changing the marker implementation.

- [x] **Step 2: Implement the M6 marker gate**

Change the shared marker constant, helper, and `commands.rs` success/error messages to M6. Keep the existing artifact/log paths. M5 scripts must continue checking their complete M5 sequence and may observe the later M6 lines in the same guest log.

- [x] **Step 3: Add ordered M6 acceptance scripts**

Both scripts must require, in order, the M5 markers through
`Nagi M5 FPU state round-trip PASS`, then:

```text
Nagi M6 supervisor START
Nagi M6 manifest dependency order PASS
Nagi M6 service health PASS
Nagi M6 service registry START
Nagi M6 echo@1 call PASS
Nagi M5 syscall PASS
Nagi M5 acceptance PASS
Nagi M6 acceptance PASS
```

The exact user-visible service proof is the guest-generated `echo@1 call PASS` after the handler verified the returned request bytes.

- [x] **Step 4: Run both acceptance scripts**

Run sequentially:

```text
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tests\acceptance\m6_init_supervisor_registry.ps1
& 'C:\Program Files\Git\bin\bash.exe' ./tests/acceptance/m6_init_supervisor_registry.sh
```

Expected: both report PASS and the serial log contains the complete ordered M5→M6 sequence.

- [x] **Step 5: Commit**

```text
git add tools/nagi-cli/src/image.rs tools/nagi-cli/src/commands.rs tests/acceptance
git commit -m "test: add M6 service registry acceptance"
```

### Task 5: Complete M6 verification and update the Source of Truth

**Files:**
- Modify: `docs/implementation_status.md`
- Modify: `docs/superpowers/specs/2026-09-18-m6-init-supervisor-registry-design.md` only if implementation evidence requires clarification
- Modify: `docs/superpowers/plans/2026-09-18-m6-init-supervisor-registry-plan.md` checkboxes as tasks complete

- [x] **Step 1: Run the full verification set**

Run each command with exit-status gating:

```text
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --exclude nagi-kernel --exclude nagi-loader --locked -- -D warnings
cargo clippy -p nagi-kernel --lib --locked -- -D warnings
cargo build --workspace --exclude nagi-kernel --exclude nagi-loader --locked
cargo build -p nagi-init --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --release --locked
cargo build -p nagi-kernel --target targets/x86_64-unknown-nagi.json -Zbuild-std=core,compiler_builtins --release
cargo build -p nagi-loader --target x86_64-unknown-uefi --release --locked
```

Expected: all commands pass without changing existing tests or architecture boundaries.

- [x] **Step 2: Review the final guest log and Git state**

Confirm the final `out/logs/m1-qemu-boot.log` contains the ordered M5 and M6 markers, run `git diff --check`, inspect `git status --short`, and confirm no generated output was staged.

- [x] **Step 3: Update implementation status only after both acceptance scripts pass**

Set M6 to `PASS`, keep M7 as `NOT STARTED`, set Current milestone to M7, record the exact 2026-09-18 commands and guest markers, and update the handoff summary. If any acceptance or full verification command fails, leave M6 `PARTIAL` or `BLOCKED` with the concrete blocker and do not advance.

- [x] **Step 4: Commit the accepted milestone**

```text
git add docs/implementation_status.md docs/superpowers/plans/2026-09-18-m6-init-supervisor-registry-plan.md
git commit -m "docs: record M6 service registry acceptance and advance to M7"
```

## Self-Review Checklist

- Spec coverage: all six M6 deliverables are represented by the user-space supervisor, manifest, dependency-order, restart-policy, health, and registry tasks.
- Security coverage: bounded storage, generation handles, unavailable-state rejection, and no kernel or host authority expansion are explicit.
- Regression coverage: M6 scripts require the existing M5 guest sequence, and the full workspace/cross-build suite is rerun.
- Placeholder scan: no `TBD`, `TODO`, disabled-test, or fake-output instructions remain.
- Type consistency: all public type and method names used by later tasks are defined in Tasks 1-3.
