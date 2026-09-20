# CI and Servo Bootstrap Normalization Implementation Plan

> For agentic workers: REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Make Nagi CI validate host tooling, Windows launcher behavior, Nagi target builds, and the pinned Servo bootstrap boundary without discarding the protected M17 worktree.

**Architecture:** Servo remains outside the parent Git tree. The lock file is validated in ordinary host tests; nagi fetch acquires and verifies the exact detached revision into generated third_party/servo state, while tracked third_party/servo-patches holds Nagi changes. GitHub Actions is split into Ubuntu host, Windows launcher, and Nagi target jobs.

**Tech Stack:** Rust nightly nightly-2025-08-01, Cargo workspace, Git CLI, PowerShell, POSIX shell, GitHub Actions, Nagi JSON targets, UEFI target, QEMU/OVMF.

## Global Constraints

- Preserve main at 66f0a068 and its untracked Servo checkout. Never reset, checkout, clean, or recursively delete it.
- Host jobs must not substitute host runtime behavior for guest behavior.
- Kernel, loader, no_std, and Nagi-target components are built only with their intended targets.
- Servo remains revision b820a9679a784877f91b4acc90c2c6e849f18d3b from https://github.com/servo/servo.git, license MPL-2.0.
- Do not remove format, lint, build, test, acceptance, target-build, or Servo boundary checks.
- Do not add continue-on-error or || true.
- Every PowerShell multi-command step checks $LASTEXITCODE after each external command.
- Do not mark M17 PASS from host-only evidence.

---

### Task 1: Format and host-boundary baseline

Files:
- Modify: kernel/src/audio.rs
- Modify: kernel/src/syscall.rs
- Modify: user/libnagi/src/storage.rs
- Modify: user/nagi-pal/src/time.rs
- Modify: tools/nagi-cli/src/commands.rs
- Test: tools/nagi-cli/tests/cli.rs

Interfaces:
- Consumes the current workspace package graph and CLI parser.
- Produces a rustfmt-clean baseline and a shared host command-argument contract.

- [ ] Reproduce the baseline from `<temporary-checkout>/nagi-ci-normalization`:

~~~powershell
cargo fmt --all -- --check
if ($LASTEXITCODE -eq 0) { throw 'Expected format failure' }
cargo test -p nagi-cli --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
~~~

Expected: format reports the known audio, syscall, storage, and time differences; CLI tests pass.

- [ ] Add a failing test in tools/nagi-cli/tests/cli.rs:

~~~rust
#[test]
fn host_workspace_commands_exclude_guest_binaries() {
    let args = nagi_cli::commands::host_workspace_args("build");
    assert!(args.windows(2).any(|pair| pair == ["--exclude", "nagi-kernel"]));
    assert!(args.windows(2).any(|pair| pair == ["--exclude", "nagi-loader"]));
}
~~~

Run cargo test -p nagi-cli host_workspace_commands_exclude_guest_binaries --locked. Expected: failure because the helper is absent.

- [ ] Add this public helper in tools/nagi-cli/src/commands.rs and route Build, Test, and Lint through it:

~~~rust
pub fn host_workspace_args(command: &str) -> Vec<&'static str> {
    match command {
        "build" => vec![
            "build", "--workspace", "--exclude", "nagi-kernel",
            "--exclude", "nagi-loader", "--locked",
        ],
        "test" => vec![
            "test", "--workspace", "--exclude", "nagi-kernel",
            "--exclude", "nagi-loader", "--locked",
        ],
        "clippy" => vec![
            "clippy", "--workspace", "--all-targets",
            "--exclude", "nagi-kernel", "--exclude", "nagi-loader",
            "--locked", "--", "-D", "warnings",
        ],
        _ => panic!("unsupported host workspace command: {command}"),
    }
}
~~~

- [ ] Run rustup run nightly-2025-08-01 cargo fmt --all, review only the four known format files, then run cargo fmt --all -- --check and cargo test -p nagi-cli host_workspace --locked. Both must exit 0.

---

### Task 2: Lock-only Servo validation and tracked patch boundary

Files:
- Create: tools/nagi-cli/src/servo.rs
- Modify: tools/nagi-cli/src/lib.rs
- Modify: tools/nagi-cli/src/config.rs
- Modify: tools/nagi-cli/tests/cli.rs
- Modify: third_party/sources.lock
- Create: third_party/servo-patches/README.md
- Modify: .gitignore

Interfaces:
- Consumes third_party/sources.lock and the existing validator call site.
- Produces ServoSourceSpec, lock validation, checkout validation, and safe bootstrap functions.

- [ ] Write failing tests in servo.rs for lock validation without a checkout, wrong REVISION rejection, and dirty nested-Git checkout rejection. Each test uses and removes only its own unique temp directory.
- [ ] Run cargo test -p nagi-cli servo --locked. Expected: failure because the new module/functions are absent.
- [ ] Implement these interfaces:

~~~rust
pub(crate) struct ServoSourceSpec {
    pub(crate) repository: String,
    pub(crate) revision: String,
    pub(crate) source_hash: String,
    pub(crate) license: String,
    pub(crate) vendored_path: PathBuf,
    pub(crate) patch_path: PathBuf,
}

pub(crate) fn load_servo_source_spec(root: &Path) -> Result<ServoSourceSpec, String>;
pub(crate) fn validate_servo_source_lock(root: &Path) -> Result<ServoSourceSpec, String>;
pub(crate) fn validate_servo_checkout(
    root: &Path,
    spec: &ServoSourceSpec,
) -> Result<PathBuf, String>;
pub(crate) fn ensure_servo_checkout(root: &Path) -> Result<PathBuf, String>;
~~~

Require the exact URL, revision, git:revision source hash, MPL-2.0, third_party/servo, and third_party/servo-patches. Lock validation must not require REVISION or a source checkout.

- [ ] Validate generated checkout path, Cargo.toml, REVISION, patch directory, nested Git repository, exact HEAD, and clean status with git -C checkout rev-parse HEAD and git -C checkout status --porcelain. Errors must not mutate the checkout.
- [ ] Change the lock entry to nagi_patch = "third_party/servo-patches". Add README describing the tracked patch boundary and generated third_party/servo checkout. Add exactly /third_party/servo/ to .gitignore.
- [ ] Replace repository_contains_the_pinned_servo_source_boundary with a lock-only test.
- [ ] Run cargo test -p nagi-cli servo --locked and require exit 0.

---

### Task 3: Exact-revision Servo bootstrap and host commands

Files:
- Modify: tools/nagi-cli/src/commands.rs
- Modify: tools/nagi-cli/src/lib.rs
- Modify: tools/nagi-cli/tests/cli.rs

Interfaces:
- Consumes ensure_servo_checkout, lock validation, and existing Cargo process helpers.
- Produces fail-closed nagi fetch behavior and host-only build/test/lint commands.

- [ ] Add tests for clean-checkout reuse, absent-checkout command construction, wrong revision refusal, dirty checkout refusal, and nonzero child-command propagation. Use an injected runner seam for fake commands; production uses real Git.
- [ ] Run cargo test -p nagi-cli servo_bootstrap --locked and observe the red state.
- [ ] For an absent checkout, create out/cache/servo-fetch-<pid>, run direct git processes for init, remote add origin URL, fetch --depth 1 --filter=blob:none origin SHA, and checkout --detach FETCH_HEAD. Write REVISION, add /REVISION to nested .git/info/exclude, validate, and atomically rename into third_party/servo.
- [ ] If the destination exists, validate it first and refuse to reset, replace, or delete it. Remove only a newly created temporary directory after a failed bootstrap.
- [ ] Keep root cargo fetch --locked and smoltcp lock checks. Replace the unconditional Servo cargo fetch with ensure_servo_checkout and cargo fetch --locked in the validated path. Preserve nonzero Git/Cargo exit codes.
- [ ] Run, separately, cargo test -p nagi-cli --locked, cargo build --workspace --exclude nagi-kernel --exclude nagi-loader --locked, and cargo test --workspace --exclude nagi-kernel --exclude nagi-loader --locked. Stop immediately on any nonzero exit.

---

### Task 4: Split GitHub Actions responsibilities

Files:
- Modify: .github/workflows/ci.yml
- Modify: nagi.ps1 only if a focused launcher test proves a defect
- Create: tests/acceptance/m0_launcher_exit.ps1 only if existing acceptance cannot express the checks

Interfaces:
- Consumes host command boundaries, nagi fetch, target JSONs, and existing wrappers.
- Produces Ubuntu host, Windows launcher, and Nagi target jobs.

- [ ] Ubuntu keeps checkout/toolchain/dependencies/format and uses:
  cargo clippy --workspace --all-targets --exclude nagi-kernel --exclude nagi-loader --locked -- -D warnings
  cargo build --workspace --exclude nagi-kernel --exclude nagi-loader --locked
  cargo test --workspace --exclude nagi-kernel --exclude nagi-loader --locked
  ./tests/acceptance/m0_doctor.sh
  ./tests/acceptance/m0_launcher.sh
- [ ] Add nagi-target. Install host dependencies and nightly with rust-src; add x86_64-unknown-uefi; run:

~~~yaml
- name: Bootstrap pinned Servo source
  run: cargo run --locked -p nagi-cli -- fetch
- name: Build Nagi kernel
  run: cargo +nightly-2025-08-01 build -p nagi-kernel --target targets/x86_64-unknown-nagi.json -Zbuild-std=core,compiler_builtins --release --locked
- name: Build Nagi user init
  run: cargo +nightly-2025-08-01 build -p nagi-init --features m16-package --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --release --locked
- name: Build UEFI loader
  run: cargo +nightly-2025-08-01 build -p nagi-loader --target x86_64-unknown-uefi --release --locked
~~~

This is target compilation evidence, not QEMU acceptance.
- [ ] Windows uses host exclusions and immediate checks:

~~~powershell
cargo build --workspace --exclude nagi-kernel --exclude nagi-loader --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
cargo test --workspace --exclude nagi-kernel --exclude nagi-loader --locked
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
.\nagi.ps1 --help
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
.\nagi.ps1 doctor --allow-missing
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
.\tests\acceptance\m0_launcher.ps1
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
~~~

- [ ] Keep checkout pinned. Do not add failure masking or remove checks.
- [ ] Verify with rg that three jobs, guest exclusions, LASTEXITCODE checks, target commands, and no forbidden masking tokens exist.

---

### Task 5: Align ADR, status, and bootstrap docs

Files:
- Modify: docs/decisions/0017-servo-pin-and-adapter-boundary.md
- Modify: docs/implementation_status.md
- Modify: docs/development/README.md only if it claims Servo is committed

Interfaces:
- Consumes the final bootstrap path and workflow job names.
- Produces docs consistent with lock, CLI, CI, and M17 status.

- [ ] Update ADR 0017 to say exact revision in sources.lock, generated checkout at third_party/servo, tracked patches at third_party/servo-patches, clean/wrong-revision refusal, and nagi fetch for developer/CI bootstrap. Preserve Servo-only and software-rendering decisions.
- [ ] Add a dated checkpoint to implementation_status.md: M17 remains NOT STARTED for guest Servo acceptance; host, Windows, and target CI responsibilities are now explicit; list exact local verification and any unavailable remote evidence.
- [ ] Run:

~~~powershell
rg -n "pinned source is present|third_party/servo/patches|third_party/servo-patches|nagi fetch|nagi-target|workspace --locked" docs tools .github third_party/sources.lock
~~~

Update only contradictions.

---

### Task 6: Verify, commit, and resume M17

Files:
- Modify: none unless a scoped verification failure requires it

Interfaces:
- Consumes Tasks 1-5 and protected main.
- Produces fresh evidence and a clean, resumable CI branch.

- [ ] Run separately and inspect every exit code:

~~~powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --exclude nagi-kernel --exclude nagi-loader --locked -- -D warnings
cargo build --workspace --exclude nagi-kernel --exclude nagi-loader --locked
cargo test --workspace --exclude nagi-kernel --exclude nagi-loader --locked
./tests/acceptance/m0_doctor.sh
./tests/acceptance/m0_launcher.sh
cargo +nightly-2025-08-01 build -p nagi-kernel --target targets/x86_64-unknown-nagi.json -Zbuild-std=core,compiler_builtins --release --locked
cargo +nightly-2025-08-01 build -p nagi-init --features m16-package --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --release --locked
cargo +nightly-2025-08-01 build -p nagi-loader --target x86_64-unknown-uefi --release --locked
~~~

Run nagi fetch in a clean disposable generated-source directory, verify exact Servo HEAD, then run Servo Cargo fetch. Record unavailable dependencies as exact blockers.
- [ ] Run git diff --check, git status --short --branch, and git diff --stat. Commit with a CI/Servo-bootstrap message; do not merge or push automatically.
- [ ] Verify protected main still has the original one-line libnagi-servo diff and pre-existing Servo checkout.
- [ ] Resume M17 by inspecting the nested Servo checkout, adapter, and patch boundary without discarding them. Do not set M17 PASS until target build, real guest bootstrap, and first web-pixel acceptance evidence exist.

## Self-review

Every requested CI responsibility maps to Task 4. Lock, source acquisition, patch boundary, failure behavior, and bootstrap map to Tasks 2-3. Host and target commands are distinct. No step obtains Green by deleting checks, masking failures, resetting work, or discarding implementation. M17 is resumed after CI verification but cannot be falsely marked complete by host-only evidence.
