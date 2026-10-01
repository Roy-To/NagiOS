# Nagi OS M21 — Planner / Validator / Executor

**Status: PARTIAL**

## Implemented contract

- `schemas/NagiPlan@1.json` defines the versioned, bounded plan envelope.
  Runtime parsing rejects unknown fields, incomplete JSON, unsupported
  versions, oversized documents, and plans outside step/object/intent limits.
- The `no_std` `services/nagi-ai` library resolves caller context through a
  required visibility authority and supplies only visible stable Object IDs to
  a provider. Prompts contain only caller-filtered, bounded Action schemas.
- `ModelManagerPlanAdapter` invokes the existing untrusted
  `GenerativeProvider`; complete output remains a candidate until Validator
  accepts it. `LlmDecisionAdapter` returns only a candidate from the bounded
  action set. Confidence changes fallback routing only.
- Validator checks registered actions, action-specific parameter names/types/
  bounds, allowed object IDs, visibility and capability policy before any
  executor step begins. Paths and shell/command parameters are excluded.
- Executor obtains fresh capability grants and object handles for each action,
  bounds the returned result, checks result Object IDs for visibility, and
  reports success/failure/partial completion. It does not claim rollback.
- `register_file_search_action` binds the existing M19 `SearchService` to the
  real `file.search` Action Registry entry. It limits queries to 128 bytes,
  returns at most 64 visible Object IDs, and delegates visibility to the
  SearchService's injected filter. The host integration test runs a plan
  through validation and execution and confirms another app's private file is
  omitted. Its in-memory backend and filter are test-only.
- The M19 QEMU fixture now composes this action on the guest: it parses a
  bounded `NagiPlan@1`, resolves fixture context, validates the registered
  action, checks the fixture capability, executes against the real persistent
  VFS-backed SearchService, and verifies the returned stable ObjectId. A
  foreign fixture caller is denied. This policy remains local to the
  acceptance fixture; kernel Channels are not exposed to user processes, so
  there is still no authenticated production caller provider.
- The M22 QEMU fixture now registers a fixture-scoped `file.move` Action. One
  bounded plan names three stable Object IDs and three fixed destination
  basenames. ContextResolver supplies only those fixture objects; Validator
  checks the registered Modify action, `files.move` capability, objects, and
  bounded parameters; Executor acquires the private fixture grant and trusted
  fixture handles before invoking the handler. The handler writes NH16
  Prepared before the three real guest VFS renames and writes Committed after
  flush. A fresh-disk guest run verifies the committed archive after remount,
  then the next boot undoes the three moves through History. This is
  deterministic acceptance input, not model inference, and the caller/policy
  is private to this fixture rather than authenticated production authority.
- The same fresh-disk fixture now also registers a bounded `file.copy` Action.
  It accepts one resolved source Object ID, requires the separate
  `files.copy` fixture capability, reads the already-moved source from guest
  VFS, and writes only the fixed `m22-copy` destination for this acceptance
  file (at most 512 bytes). The fixture rejects a denied copy capability and
  `../outside`, persists an NH16 Prepared Create and NAL1 Prepared record
  before VFS creation, then commits and verifies both archives after flush.
  Executor visibility for the newly created Object ID is enabled only after
  the VFS create succeeds. This is still a private, deterministic fixture; it
  is not a general Files service handler or production Action Registry.
- On a fresh-disk M22 boot, the guest also verifies malformed/incomplete JSON,
  unsupported plan version, unregistered action, out-of-context Object ID,
  policy-denied Modify access, and denied `files.delete` capability. The
  validator returns the expected typed error for each parsed plan, and the
  test-only registered handlers record zero executions for all rejected
  plans. A separate two-step orchestration probe succeeds once, then returns
  `HandlerError::Unavailable`; the guest checks `ExecutionStatus::Partial`, the
  completed step, failing step index, error, and both handler calls. These
  handlers have no product side effects and do not represent production
  actions.
- No production guest init service currently constructs this registry. General
  app launch, file copy/move, and volume handlers remain absent; the production
  target policy and Context authorities are not connected.

## Verification

Using the pinned aarch64 macOS Rust toolchain:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m21-host-arm64 \
/Users/tozawa/.cargo/bin/cargo test --locked --offline -p nagi-ai
```

Result: 24 orchestration and SearchService integration tests passed; no doc
tests are defined.

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m21-clippy-arm64 \
/Users/tozawa/.cargo/bin/cargo clippy --locked --offline \
  -p nagi-ai --all-targets -- -D warnings
```

Clippy passed with warnings denied. Formatting passed:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
/Users/tozawa/.cargo/bin/cargo fmt \
  --manifest-path services/nagi-ai/Cargo.toml -- --check
```

The service compiled for Nagi `no_std` user target:

```sh
PATH=/Users/tozawa/.cargo/bin:/usr/bin:/bin:/usr/local/bin \
RUSTUP_TOOLCHAIN=nightly-2025-08-01-aarch64-apple-darwin \
RUSTC=/Users/tozawa/.cargo/bin/rustc \
RUSTDOC=/Users/tozawa/.cargo/bin/rustdoc \
CARGO_TARGET_DIR=/tmp/nagi-m21-target \
/Users/tozawa/.cargo/bin/cargo -Z build-std=core,alloc check \
  --manifest-path Cargo.toml -p nagi-ai \
  --target targets/x86_64-unknown-nagi-user.json --locked --offline
```

The `nagi-ai` service, including the SearchService action adapter, compiles
for the Nagi `no_std` user target. On 2026-09-30, `./nagi m19` passed the
guest Search/ObjectId persistence regression. A fresh-disk `./nagi m22` run
passed the M21 rejection and partial-failure checks on boot 1, the real
fixture `file.move` transaction and NAL1 commit, reverse-order Undo on boot 2,
and restored-file/NH16/NAL1 verification on boot 3. The M21 markers are
required on the first fresh boot, not on the later recovery boots. Logs, image,
OVMF vars, and persistent data are preserved under
`out/evidence/m22-m21-negative-and-partial-pass-20260930/`. This uses a
fixture-scoped policy, not an authenticated app identity or production
capability provider; no local model inference was involved.

On 2026-10-01, a fresh-disk `./nagi m22` run also passed the bounded fixture
`file.copy` action and its NH16 Create/NAL1 Activity record, then persisted
Undo and verified the restored source files and absent copy after restart.
The command now requires the copy acceptance marker and allocates unique
image, User Data, OVMF variables, and log paths for each run, refusing to
overwrite an existing path. The fresh run's seven-file SHA-256 manifest is at
`out/evidence/m22-file-copy-1790854068023718000/manifest.sha256`; its hashed
files are the run image, User Data disk, OVMF vars, bootstrap log, and three
boot logs under `out/artifacts/` and `out/logs/`.

## Remaining acceptance blockers

1. Bind `ActionPolicy` and `ContextAuthority` to authenticated guest caller
   capabilities and object handles through isolated processes and
   supervisor-authorized endpoint delivery. The bootstrap Channel ABI only
   exposes the shared `nagi-init` Process (PID 1) and cannot authenticate app
   identity. The library intentionally has no allow-all production provider.
2. Register `file.search` and general first-party actions in the running
   production AI service with that authenticated provider. The bounded M22
   fixture `file.move` action is not a production service handler. Add real
   `app.launch`, `file.copy`, `file.move`, and `system.volume.set` handlers
   against their existing first-party services.
3. Connect Context Resolver and Planner to the running Nagi AI/model service,
   including provider-unavailability fallback in the UI/service path.
4. Extend authenticated production-path acceptance to malformed and
   unsupported plans, object/capability denial, successful real action
   execution, and partial failure of a real subsystem action. The current
   guest checks prove the Validator/Executor orchestration boundary only.

M21 stays `PARTIAL`; test-only policy or action mocks cannot satisfy the guest
acceptance gate.

## Completion sweep Priority A audit — 2026-10-01

The existing M4 Channel implementation is a kernel primitive with rights
attenuation and sender-process stamping, and the kernel M4 acceptance exercises
it. The 2026-10-02 Completion Sweep adds user ABI syscalls for bounded Channel
create, send, nonblocking receive, and handle close. Its M19 QEMU check covers
payload round-trip, kernel-stamped process identity, READ-only handle transfer,
queue limits, and handle/channel reuse.

The user ABI manager still contains only the single shared-address-space
`nagi-init` Process (PID 1). Every sender is that same Process. The user-space
ServiceRegistry in libnagi stores handler function pointers and invokes them
directly. Its guest echo acceptance registers and calls a handler inside
nagi-init; it is not cross-process IPC or an application service boundary. The
AI CallerIdentity contains caller-provided logical AppId and AppSessionId
fields, while ContextAuthority and ActionPolicy are intentionally
trusted-provider interfaces with no allow-all production implementation.

Therefore adding a production ActionPolicy by trusting plan/request identity,
or treating either the bootstrap Channel ABI or in-process registry as
authenticated service IPC, would weaken the capability boundary. The remaining
prerequisite is the production process and launch authority boundary: distinct
process identities and address spaces, supervisor-authorized endpoint
creation/transfer, and a provider that derives policy from kernel-authenticated
handles and the launch record. Keep the current guest M19/M22 fixtures as
orchestration evidence only. The bootstrap ABI does not authenticate an
application or session.

## Completion Sweep: record the M19 `file.search` result — 2026-10-01

After the M19 M21 `file.search` plan succeeds through the guest Validator and
Executor, M19 returns a typed bounded event containing its occurrence tick,
fixture App/Session/Node/Workspace context, user intent, query summary, and the
single Object ID returned by the action. M13 passes that actual event into the
M22 fixture; M22 does not reconstruct a synthetic search result.

M22 persists it in NAL1 as `file.search`, with no transaction ID and the normal
Prepared-to-Committed result path. The three-boot QEMU acceptance verifies the
record at boot 1 and reopens the same record at boots 2 and 3. This is an
end-to-end orchestration and archive regression only: the caller identity and
policy remain fixture values, and no authenticated production service or
caller boundary is claimed. M21 remains `PARTIAL`.

## Structured output request — 2026-10-02

`ModelManagerPlanAdapter` now parses and attaches the authoritative
`schemas/NagiPlan@1.json` to each `ModelRequest` as a structured-output schema.
The model-manager runtime rejects structured capability requests without a
schema, rejects schema use with another capability, checks the full backend
JSON response (including duplicate keys) against the supported bounded schema
subset, and does not stream unvalidated structured bytes. The adapter repeats
the schema check at its generic provider boundary, since provider
implementations may not use `LoadedSession`; its 1,024-token request budget
matches all three current model manifests. `NagiPlan::parse_complete` and
`validate_plan` remain independent post-provider checks and continue to control
accepted actions, parameters, caller context, and object visibility.

On 2026-10-02, 24 `nagi-ai` tests and 57 model-manager unit tests plus 3
integration tests passed, as did warnings-denied Clippy and Nagi no-std target
compilation. The fresh M22 three-boot regression also passed guest search,
move/copy, persisted NAL1/NH16 state, grouped Undo, and restart restoration;
its evidence is under
`out/evidence/m22-structured-output-regression-1790871324761891000/`. No model
backend is connected to this adapter yet, and the QEMU fixture does not claim
inference. Production authenticated service IPC and caller authority remain
absent; M21 remains `PARTIAL`.

## Bootstrap Channel ABI user path — 2026-10-02

The 2026-10-02 Completion Sweep exposes bounded Channel create, send,
nonblocking receive, and handle close through the user syscall ABI. Its fresh
M19 QEMU acceptance passes payload round-trip, kernel-stamped PID 1 despite
forged payload bytes, READ-only transferred-handle behavior, full-queue
rejection, and close/reuse. The manager supports only the single bootstrap
Process and has no blocking wait. This is IPC plumbing evidence; it does not
bind the fixture's App/Session identity to a trusted supervisor, production
ActionPolicy, or production ContextAuthority. M21 remains `PARTIAL`.

## Completion Sweep — shared IPC and capability boundary audit (2026-10-02)

The cross-milestone audit in `docs/implementation_status.md` confirms that the
current `ServiceRegistry` is an in-process function-pointer registry and the
guest ActionPolicy caller context is fixture data. The published bootstrap
Channel syscalls do not expose waiting, isolated processes, or
supervisor-authorized endpoint delivery. Production handlers require
authenticated process/launch context and capability-backed service endpoints;
a caller-supplied PID or App ID is insufficient. The current Search and VFS
mutation fixtures remain valid, and M21 stays `PARTIAL`.
