# 1. Current status

**Current milestone:** `M30 — Nagi OS 0.1 Release`
**Milestone status:** M19 `PARTIAL`, M20 `PARTIAL`, M21–M22 `PARTIAL`,
M23–M30 `PARTIAL`.
**0.1 / 0.2 line merge, 2026-10-05:** `main` (0.1 release line) and
`codex/integration-next-phase` (0.2 workstreams) were merged on
`claude/integrate-main-0.2`; conflict decisions are recorded in
`docs/decisions/0052-merge-0.1-release-line-into-0.2-integration.md`. Host
fmt, warning-denied Clippy, host tests, standalone crate checks, the
localization catalog check, and M0 launcher acceptance pass locally (arm64
macOS host). Target acceptance (M17–M30) is verified by the PR's target CI.

**In-guest system update installer (ADR 0062), 2026-10-06:** A running
System A now installs a signed update into System B, the loader re-verifies
and trials it, and B is confirmed after readiness.

- **Boot context and capability.** BootInfo v5 marks confirmed boots as
  update-stageable. On those boots the kernel exposes only the inactive
  slot as a one-shot, init-only capability (`SYS_UPDATE_SLOT_CLAIM`).
- **Staging request.** `SYS_UPDATE_SLOT_STAGE` writes a CRC-protected
  `NagiBootStage` variable. The loader honors it only when the generation
  matches, nothing is pending, the manifest verifies, and the rollback
  index is not lowered.
- **Installer.** The installer (`m30-update-install`) verifies the bundle
  before writing, then formats the slot as FAT32 (`crates/nagi-fat32`),
  flushes it, re-verifies a read-back, and stages.
- **Result.** `./nagi m30-update` passed (evidence
  `out/evidence/m30-update-1791242245330653000`):
  - signed bundle: install, then trial `slot=B rollback-index=2` with
    readiness persisted, then `confirmed slot=B`;
  - tampered bundle: refused before any write, and A stayed confirmed.
- **Regressions.** `./nagi m27` still passes. The workspace has 862 host
  tests, plus 163 kernel, 16 bootinfo, 7 slot-manifest and 4 FAT32 tests.
  Warning-denied Clippy is clean.

Still open:

- network delivery of updates;
- an update UI and user consent;
- production key provisioning.

**Authenticated slot manifests (ADR 0061), 2026-10-06:** The loader now
verifies a signed `SLOT.MAN` for System A, System B and Recovery before it
trusts any payload.

- **Format and checks.** The manifest is Ed25519-signed with a domain
  separator and pins the SHA-256 and size of `KERNEL.ELF` and `INIT.ELF`.
  A trial slot must not lower the confirmed slot's rollback index. These
  checks live in the `no_std` `crates/nagi-slot-manifest`, which has five
  host tests.
- **Rejection path.** A rejection prints `Nagi slot manifest REJECTED
  slot=<S> reason=<…>` and consumes an M27 trial attempt.
- **M27 GPT fixture.** System B now has a bootable ELF, but its manifest is
  signed by an untrusted key.
- **Soft-float build.** The UEFI target uses curve25519-dalek's `serial`
  backend because it is soft-float.
- **Result.** On the arm64 macOS host, `./nagi m27` passed (evidence
  `out/evidence/m27-ab-rollback-1791241326571375000`):
  - three trials printed `reason=signature`, then Recovery ran, then the
    boot rolled back to System A;
  - the healthy B verified and was promoted;
  - the FAT12 malformed B is still refused as `invalid ELF`.
- **M30.** `./nagi m30` passed from a clean worktree of the commit (run
  `m30-release-1791241538355805000`): System A and Recovery printed
  `Nagi slot manifest verified … rollback-index=1 PASS`.
- **Host checks.** 855 workspace host tests and the loader library tests
  pass.

Still open for the M30 update item:

- an in-guest installer that writes a signed update into the inactive slot,
  reads it back, and verifies it;
- a loader-consumed staging request;
- production key provisioning.

**User consent for manifest grants (ADR 0051), 2026-10-03:** A signed
manifest's `grant=` line is now only a request.

- **Effective grants.** A capability is effective only for a live session
  whose manifest requests it *and* for which an authenticated, unlocked user
  recorded `Allow`, or `AllowOnce` for that session. The default is
  `ConsentRequired`, which fails closed. `Deny` overrides the manifest.
- **Who decides.** Developer Mode and the Owner role do not imply consent.
  Launched processes have no route to the decision API.
- **Result.** `./nagi isolated-process` verified fail-closed defaults,
  locked-session refusal, `AllowOnce` scoping across two live sessions and
  its expiry at exit, and `Deny`/`Allow`/`Ask`.

Still open:

- the trusted consent dialog (acceptance decisions come from a fixture
  account);
- persisting decisions;
- foreground/background distinctions.

**Concurrent isolated processes (ADR 0050), 2026-10-03:** The kernel now
runs two isolated processes at once.

- **Per-process resources.** Each process has its own address-space slot
  and CR3. The IPC manager and the exit table track each process with its
  own waiter.
- **Result.** `./nagi isolated-process` ran PIDs 3 and 4 concurrently and
  refused a third spawn. A fault in one left the other running, and a freed
  slot was reused.

**Signed launch packages (ADR 0049), 2026-10-03:** Isolated applications now
launch only from Ed25519-signed M16 `.xapp` packages, built by
`nagi-pkg build-signed`. Their identity and grants come solely from the
signed manifest, which gained `grant=` lines. Package size was raised to
64 KiB.

The Supervisor refuses:
- tampered packages (`UnsignedPackage`);
- packages requested as another application (`WrongApplication`);
- malformed packages (`InvalidPackage`);
- conflicting declarations of the same application.

`./nagi isolated-process` verified this on QEMU.

**Boot image size:** embedding the signed packages made the M22 init too
large for the legacy 1.44 MB FAT12 boot image. Isolated-client and
isolated-process images now use a 4 MiB FAT12 image (`ISOLATED_APPS_IMAGE_SIZE`,
2 KiB clusters). It stays smaller than the User Data disk, because the
kernel selects the largest writable VirtIO Block device as User Data; a
128 MB M17-style image made M7 select the boot disk. After the change,
`./nagi m22` (all three boots), `./nagi m19`, and `./nagi isolated-process`
pass locally.

Still open:
- trust-store provisioning beyond the pinned Developer Preview key;
- user consent for grants (addressed by ADR 0051);
- installing packages through the Package Service store instead of the
  init image.

**Supervisor exit wait/status (ADR 0048), 2026-10-03:**

- **Unique IDs.** Isolated processes now receive unique, never-reused
  Process IDs.
- **Exit records.** The kernel keeps a bounded exit record for each process:
  clean exit with its code, or fault with its vector.
- **Wait syscall.** The init-only `SYS_PROCESS_WAIT` blocks until the
  process exits and then consumes its status. `supervisor::reap` uses it in
  place of yield-probing.
- **Local QEMU result.** `./nagi isolated-process` verified PID 2's exit 0,
  consume-once semantics, and blocked waits woken by the faults of PIDs 3–5
  (vectors 14/6/13, codes 142/134/141).
- **Earlier repeated-boot result.** The 25-boot `./nagi run` stress run on
  the ADR 0047 kernel passed 25/25.

**Ring-3 fault containment and M3 stall fix (ADR 0047), 2026-10-03:**

- **M3 stall root cause.** The intermittent "M3 scheduler workload" stall
  (QMP `shutdown`, RIP=`smp::thread_entry`, RSP=0, CR2=-8) came from an
  18-word initial task frame. `iretq` pops 20 words, so a task could start
  with RSP=0 and interrupts enabled; a timer interrupt arriving before its
  first stack switch then triple-faulted.
- **M3 fix.** The frame now carries an explicit RSP and SS, and a lib test in
  CI pins its layout.
- **TSS and exception IDT.** The BSP now has a TSS (RSP0 fault stack, IST1
  for #DF) and its own exception IDT using the M5 kernel selector.
- **Fault policy.** A CPU exception in an isolated process terminates only
  that process (exit code 128 + vector), and the next thread resumes.
  Kernel and init faults are reported, then halt.
- **QEMU evidence.** `./nagi isolated-process` now launches
  `nagi-faulting-app` three times. #PF, #UD, and #GP were each contained
  (exit codes 142, 134 and 141), the launches were reaped, and init
  continued.
- **Repeated-boot evidence.** Before the fault-containment code was added,
  13 consecutive `./nagi run` boots on the M3 fix passed. A 25-boot run on
  the final kernel is recorded under the Last updated line.

**Remaining in-process callers migrated, 2026-10-03:** These paths now run
with the caller resolved from an isolated client's launch record and
Supervisor manifest grants:

- the M22 `file.copy` action, including its denied-policy and bad-name plans
  (the `m22-files` manifest grants `files.copy`);
- the M21 plan-rejection and partial-execution fixtures;
- the `./nagi m27` Recovery-Undo image and both `./nagi m30` images, now
  built with `m21-action-ipc`.

`GrantSource::InProcessAcceptance` remains only for images built without
`m21-action-ipc`. A local `./nagi m22` passed all three boots with the
`file.copy` isolated-caller marker. See the Last updated line for the M27/M30
local results.

**Supervisor launch registry (ADR 0046), 2026-10-03:** `libnagi::launch`
adds manifest-declared applications and a Supervisor launch registry.

- **Manifests and identity.** Each application's `AppId` is derived from its
  manifest identifier. Grants come from the manifest's `grant=` lines.
- **Session-bound grants.** A grant counts only while a live launched session
  holds it. Exit revokes it. An undeclared application, a duplicate live
  session, or relabeling PID 1 is refused before spawn.
- **Single launch path.** init's `supervisor.rs` loads the embedded manifests
  from `user/nagi-init/manifests/`. It is the only launch and resolve path for
  the isolated-process, Search IPC, and action IPC acceptances.
- **Grant sources.** Search requires a live `search.query` grant. The M19/M22
  action policies take `files.search` and `files.move` from manifests through
  `GrantSource::Supervisor`.
- **Local results.** `./nagi isolated-process`, a fresh-disk `./nagi m19`, and
  `./nagi m22` passed locally. One earlier M22 attempt hit the pre-existing
  intermittent kernel stall in the M3 SMP scheduler workload before user
  space; its log is preserved in `out/evidence/m22-m3-smp-stall-20261003/`,
  and the immediate rerun passed.

Still open:

- manifests are image-embedded acceptance declarations, not signed package
  manifests, and grants have no user consent;
- the in-process acceptance caller remains for `file.copy`, the
  rejection/partial-execution fixtures, and the M27/M30 images.

**M21/M22 action IPC (ADR 0045), 2026-10-03:** M21 `file.search` and M22
`file.move` are now requested by isolated `nagi-action-client` processes over
`action@1` (`crates/nagi-action-ipc`).

- **Caller identity.** The caller passed to Context, Validate, Policy,
  Execute, NH16, and the Activity Ledger is resolved only from the
  kernel-stamped sender PID and the Supervisor launch record. Requests carry
  no identity, capability, Object ID, or plan.
- **Acceptance.** In `./nagi m19` and `./nagi m22`, a client launched as a
  foreign application is denied by policy before any handler is registered.
  The granted application's client then executes the real action.
- **Result.** A local fresh-disk `./nagi m22` run passed all three boots,
  including NH16 grouped Undo and restart verification, on the history
  created by the isolated caller.

Still open:

- `file.copy`, plan-rejection, and partial-execution fixtures stay
  in-process;
- the M27/M30 images keep the in-process caller;
- intents are Supervisor launch arguments, not Nagi Bar / Albert input;
- grants are acceptance-scoped.

M21 and M22 remain `PARTIAL`.

**M19 Search IPC (ADR 0044), 2026-10-03:** M19 Search is now served over a
Channel to isolated client processes.

- **Protocol.** The allocation-free `search@1` codec lives in
  `crates/nagi-search-ipc`. A request carries a query and no identity field.
- **Authorization.** The init-hosted SearchService resolves each caller only
  through the kernel-stamped sender PID and the Supervisor launch record,
  then applies the normal `VisibilityFilter` with that `AccessContext`. A
  sender with no record gets `UnknownCaller` before the index is read.
- **Acceptance.** `./nagi m19` now builds the separate
  `nagi-m19-search-client` ELF and runs it twice with the same query:
  - launched as the M19 app session, it receives exactly the live VFS file's
    stable ObjectId;
  - launched as a foreign app, it receives zero visible matches.
- **Result.** A fresh-disk local QEMU run passed the bootstrap, initial, and
  restart boots, and both guest boots printed the three Search IPC PASS
  markers. Logs are under `out/logs/m19-vfs-objectid-*.log`; the previous
  User Data disk is kept in `out/evidence/pre-m19-search-ipc-20261003/`.

M19 remains `PARTIAL` for three reasons:

- the launch registry is still acceptance-scoped (one isolated slot);
- the Files and Browser producers are not live sources;
- the M21 `file.search` action identity was later moved to the isolated
  caller (ADR 0045).

**Shared service identity (ADR 0043), 2026-10-03:** The M18–M23 identity
blocker now has a kernel primitive.

What was added:

- **`SYS_PROCESS_SPAWN`.** The Supervisor (init, PID 1) can load a real
  second ELF into its own PML4. The child's user half holds only its own image
  pages and a 64 KiB stack; the kernel half is supervisor-only. Only init may
  call spawn.
- **Per-process handle tables.** The user IPC manager keeps one handle table
  per process, and the spawn moves one attenuated Channel endpoint into the
  child. Sending to an endpoint whose peer no process can reach now fails with
  `PeerClosed`.
- **Cross-process scheduling.** Each scheduler thread slot has an owning
  process. Cross-process join and detach are rejected. The kernel switches
  CR3 when the cooperative scheduler crosses a process boundary.
- **Per-process pointer checks.** Every user-pointer check uses the calling
  process's own page tables.
- **Restricted child syscalls.** The child can use console, time,
  yield/sleep, random, Channel, handle close, and exit. A child exit closes
  its handles and scrubs its pages without halting the system.

Acceptance: the new `./nagi isolated-process` acceptance passed locally on
real QEMU/OVMF (Ubuntu 24.04, QEMU 8.2.2). The serial log is
`out/logs/isolated-process.log`. In that run:

- init spawned `nagi-isolated-app` as PID 2;
- the child's request arrived with kernel-stamped sender PID 2, although its
  payload claimed `org.nagi.system`/PID 1;
- the Supervisor resolved the caller through its launch record to
  `org.nagi.acceptance.isolated-app` and denied the system-only operation;
- the child confirmed that init's TLS and mmap windows were unmapped for it;
- the child confirmed that block, mmap, thread-create, spawn, and
  process-info syscalls were rejected;
- the child exited with code 0, and init observed peer closure and continued
  its normal boot.

Verification:

- Kernel host tests: 148 passed, including new scheduler, IPC, and child
  address-space tests.
- Workspace Clippy passed with warnings denied.
- Host workspace tests passed.
- Regressions passed on the same local QEMU: `./nagi run` (M7 boot) and
  `./nagi m19` (Channel ABI, wait/wake, and Search persistence across
  restart).
- The QEMU environment had no Mesa source, because gitlab.freedesktop.org is
  blocked by its network policy. M17/M18 were therefore not rerun locally;
  public CI covers them.

The step is wired into CI after M18. Still open:

- production Files, Browser, and Search callers still run inside init;
- only one isolated slot exists;
- ring-3 scheduling remains cooperative (ADR 0029).

M18–M23 remain `PARTIAL`.
**M18 predecessor evidence:** Browser HTTPS/QEMU Acceptance passed locally and
in authoritative Ubuntu CI on 2026-09-29. On 2026-10-05 a gesture-bound
user-space clipboard service (ADR 0053) passed real QEMU copy/paste
acceptance, and a kana-only user-space Japanese IME (ADR 0054; kanji
conversion deferred past 0.1 by user decision) passed real QEMU composition
acceptance. Earlier M18 HTTPS acceptance accepted pages that painted no text
(empty Nagi font registry); bundled Noto fonts (ADR 0055) fixed rendering and
the acceptance now requires non-background page pixels. M18 remains `PARTIAL` because download/upload destinations and
trusted interactive site-permission decisions still lack Nagi providers; the
user-directed M19 work proceeds because those services are not M19
dependencies. One fresh 2026-10-02 attempt timed out at the TianoCore splash
before any guest marker; its QMP loop diagnostics are preserved under
`out/evidence/m18-timeout-1790866733768047000/`. The 2026-10-03 rerun passed:
QEMU verified TLS chains and hostnames for three HTTPS pages, rendered them
through Nagi Surface, and passed temporary-storage cleanup. Its image, reused
User Data disk, OVMF variables, serial log, invocation log, screenshot, and
checksums are under `out/evidence/m18-completion-sweep-20261003/`; the previous
fixed-path artifacts are preserved under
`out/evidence/m18-pre-sweep-20261003/`. This run used the regenerated pinned
Servo checkout and Homebrew LLVM 19 with matching libc++ headers. QEMU had no
`virtio-sound.in` host audio backend, so it adds no audio acceptance evidence.
**M19 evidence:** The deterministic metadata/search contract is integrated
into the root workspace. Host tests cover metadata search, policy filtering,
producer adapters, and two-slot snapshot recovery. `./nagi m19` now enumerates
one real guest VFS file, maps its metadata through `FilesProducerAdapter`,
persists an independent fixture Object ID, and verifies that ID and file
location after rename, VFS remount, and a fresh QEMU restart. M19 remains
`PARTIAL`: this is a fixed private acceptance file, not synchronization from
the production Files or browser-page services; inode reuse is not addressed,
and Search is not exposed as a production IPC service with authenticated,
capability-bound caller context. A 2026-10-02 bootstrap Channel ABI smoke test
now passes in the M19 guest path, including kernel-stamped sender PID and
attenuated handle transfer. The same guest acceptance now also creates a user
thread that blocks on an empty Channel, verifies it remains blocked until a
peer send, then checks the received payload after wake. The wait requires the
endpoint's `WAIT` right and remains single-process plumbing; it does not provide
authenticated Search service callers.
The guest fixture also now runs a bounded M21 `file.search` plan through
ContextResolver, Validator, Action Registry, and Executor against that real
SearchService, but its caller/capability policy remains fixture-only. When M22
runs on that guest path, the executed Search result is passed as a typed event
to the M22 fixture and recorded in NAL1 without a transaction ID.
The 2026-10-03 QEMU regression passed again: the VFS file search and stable
Object ID survived rename, remount, and restart, and the M21 `file.search`
fixture passed Plan/Validate/Execute. Its pre-run fixed-path image, User Data,
vars, and log are preserved under `out/evidence/m19-pre-regression-20261003/`;
the new run is under `out/evidence/m19-regression-20261003/`.
The 2026-09-30 Completion Sweep rerun of `./nagi m19` passed; its invocation
log is `out/evidence/completion-sweep-regression-20260930/m19-qemu.log` and
pre-run artifacts are SHA-256 preserved under
`out/evidence/completion-sweep-regression-20260930/pre-m19-m22/`.
The post-M20-fixture regression rerun on 2026-10-01 also passed; its guest log
is `out/logs/m19-vfs-objectid-initial.log`.
**M20 evidence:** The provider-neutral `no_std` model manager verifies actual
artifact bytes with streaming SHA-256 before backend load. Its catalog pins the
IBM Granite 4.2 3B Q4_K_M source revision, size, digest, and Apache notice;
an initial streamed digest check matched without retaining a file; on
2026-10-02, the exact artifact was downloaded to the ignored local cache and
independently size/hash verified at
`out/evidence/m20-granite-model-download-20261002/`. A new
`./nagi m20-granite <artifact.gguf>` acceptance command cross-checks the
manifest against `third_party/models.lock`, re-hashes the source in bounded
memory, streams it into a separate disposable reference disk, and verifies the
complete guest-visible artifact digest through the read-only Model Store
capability. The 2026-10-02 QEMU run passed and is preserved under
`out/evidence/m20-granite-artifact-1790892878741511000/`. After generalizing
the artifact runner for the separate Whisper acceptance, the M20 Granite
command was rerun and again verified the full guest-visible artifact at
`out/evidence/m20-granite-artifact-1790895384092435000/`. This artifact exists
only in that dedicated acceptance image; the regular M30 release image stays
empty, and no model has been loaded for inference. Nagi now has a
numbered GGUF patch that bounds parser metadata, returns parse errors without
C++ exceptions, writes tensor data in 8 KiB chunks, and surfaces buffered write
and flush failures. `./nagi fetch` applied it while preserving the clean pinned
llama.cpp checkout. Nagi-target `ggml-base` and the static CPU `ggml` library
compiled under `-fno-exceptions`; the latter includes the new Nagi path-format
adapter for backend registration. Host GGUF tests passed 101/101, and a host
build with Nagi limits enabled passed 103/103. The fresh Nagi-target `llama`
build now passes backend registration and the grammar parser translation unit,
After patch 0009, focused Nagi C++ rebuilds of `llama-model-loader.cpp` and
`llama.cpp` confirm that earlier tensor-range and split-path/count failure
slices are removed, but both objects still fail on other exception-dependent
metadata and model-load paths. This was not a fresh full-target build; previous
full attempts also found RTTI and `PATH_MAX` target gaps. STL allocator OOM
recovery is unsupported in the current no-unwinder ABI. The kernel now
validates a separate Model Store GPT extent and passes it as read-only; a `no_std` FAT32 `ModelArtifactReader`
resolves stable artifact-ID-derived 8.3 names and reads bounded random ranges.
M30 QEMU verified GPT-bound Model Store reads, rejected writes, unchanged boot
sector, and graceful discovery of an absent Granite file. After successful GPT
and User Data initialization, a missing or unreadable Model Store capability
or invalid FAT32 volume logs an M20 acceptance failure without stopping
ordinary OS boot; the dedicated M30 gate still requires its PASS marker.
Structurally invalid GPT metadata remains fail-closed. Forty-eight model
manager tests, two manifest/schema tests, one Store API test, Nagi no-std
target compilation, formatting, lint, repository tests/build, and M30 QEMU
passed. A FAT32 fixture now reaches `ModelRuntime::load`; the runtime hashes
actual bytes, accepts a matching digest, and rejects a wrong digest before
backend load. This uses an orchestration fake and is not inference. No complete
llama.cpp backend, regular-release model installation, active model service,
or real in-guest Granite response exists, so M20 remains
`PARTIAL`; build evidence is in
`out/evidence/m20-backend-reg-noexceptions-20261001/` and reader acceptance is
in `out/evidence/m30-release-1790806831243045000/`. A separate 2026-10-01
guest reader fixture now uses the actual Model Store read-only capability to
read and byte-check a 5,000-byte multi-cluster FAT32 test artifact in bounded
chunks, including a cluster-boundary reread and EOF. It is not a valid model;
the production/reference image still has an empty Model Store. The release
image SHA-256 is unchanged, and the reference, persistence copy, and dedicated
fixture image passed `qemu-img check`. Logs, images, OVMF variables, README,
and hashes are under `out/evidence/m30-release-1790809848636521000/`. This does
not change M20 from `PARTIAL` or claim inference. Both baseline and fixture-
enabled M20 init variants compile for the Nagi target. The first CI run for
the guest-reader fixture found that `nagi-bootstrap` compiles the shared CLI
source from its own manifest, which also needs the direct
`nagi-model-manager` dependency. That manifest and its lockfile now include it;
the locked bootstrap build and all 146 CLI unit tests pass locally. The
2026-10-02 grammar status patch also passes its parser, integration, JSON
Schema-to-grammar, CLI patch-contract, and Nagi-target grammar translation-unit
checks. The full target failure log is
`out/logs/m20-grammar-status-target-build-20261002.log`. A follow-up numbered
patch documents the `llama_sampler_sample()` `LLAMA_TOKEN_NULL` failure result
and guards every direct C++/Swift example caller before EOG checks, token-piece
conversion, or batch submission. The patch applies cleanly to the pinned llama.cpp
source, the CLI contract suite and four C++ example builds pass, and both changed
Swift files typecheck. Top-level `./nagi fetch` currently stops earlier at a
pre-existing mismatched generated Servo checkout, so it did not reach llama.cpp.
M20 remains `PARTIAL`.
Patch `0008-nagi-tensor-weight-status.patch` now replaces the throwing tensor
weight constructor with checked file-range initialization across the main,
split-shard, and file-pointer loader paths. Missing tensor metadata, duplicate
names, truncated ranges, and overflow fail before a weight enters the map; the
model-load status is checked before metadata printing and model creation. Its
host CTest passes the valid, missing-index, beyond-EOF, truncated, and overflow
cases, and the same test translation unit compiles for Nagi with exceptions
disabled. The focused CLI patch-contract test and `./nagi test`, `fmt`, `lint`,
and `build` pass. `./nagi fetch` generated the numbered 0001–0008 llama.cpp
checkout, then stopped at the pre-existing mismatched Servo checkout without
modifying it. Focused target object compilation still fails on other
exception-based loader and model-load paths; see
`out/logs/m20-loader-weight-status-target-build-20261002.log`. No complete
backend or inference is claimed. The patch, test source, host CTest output,
target object and build log have a verified manifest at
`out/evidence/m20-tensor-weight-status-20261002/SHA256SUMS`. M20 remains
`PARTIAL`.
The 2026-10-02 tensor-data follow-up adds numbered patch
`0014-nagi-llama-data-validation-status.patch`. Invalid row data in
`load_all_data()` now sets sticky loader failure and returns a failed status
after backend upload events, staging buffers, and the upload backend are
released; ordinary host builds retain their exception behavior. A minimal
GGUF regression with an infinite F16 value reaches the real loader path: the
Nagi-macro and ordinary host CTests each pass 1/1, and the CLI suite passes
177/177 with `cargo fmt --check`. The CLI patch-contract test also checks that
event synchronization/free and buffer/backend cleanup precede failure return.

Fresh `./nagi fetch` applied patches 0001–0014 with patch fingerprint
`fnv1a64:71b15d0ee0a3a3f4` and checkout fingerprint
`fnv1a64:65aeca191a608371`; reverse `git apply --check` confirms patch 0014 is
present. Fetch then stopped at the pre-existing modified generated Servo
checkout and refused to modify it. The preserved prior generated checkout and
its verified 3,674-entry SHA-256 manifest are under
`out/evidence/m20-loader-status-0014-pretest-patch/`.

The focused Nagi-target loader object still fails on 13 exception-dependent
sites elsewhere in the loader, including tensor lookup/context setup and
`load_data_range()`. The two invalid-row-data throw sites in `load_all_data()`
are gone. See `out/logs/m20-loader-status-0014-final-target-build.log`. No
complete target backend or in-guest inference is claimed; M20 remains
`PARTIAL`.
The next numbered patch, `0015-nagi-llama-data-range-status.patch`, changes
invalid tensor-range validation in `load_data_range()` to sticky loader failure
and a null result on Nagi. Both quantizer range consumers check for null
before reading or dequantizing tensor bytes and propagate failure as a
nonzero quantize result. The synthetic infinite-F16 GGUF regression exercises
both `load_data_range()` and `load_all_data()`: the Nagi-macro CTest returns
failure without throwing, while the ordinary host CTest retains exception
behavior; each passes 1/1. The CLI patch-contract suite passes 178/178 and
`cargo fmt --check` passes.

Fresh `./nagi fetch` applied patches 0001–0015 with patch fingerprint
`fnv1a64:d2c8dafa4e23e117` and checkout fingerprint
`fnv1a64:599ed2820ae18f26`; reverse `git apply --check` confirms patch 0015 is
present. Fetch again stopped at the pre-existing modified generated Servo
checkout and left it untouched. The previous generated working copy and its
verified 3,674-entry SHA-256 manifest are under
`out/evidence/m20-loader-status-0015-generated-working-copy/`.

The focused Nagi-target loader object now reports 12 remaining exception
diagnostics, down from 13; the removed site was `load_data_range()`. See
`out/logs/m20-loader-status-0015-final-target-build.log`. Host CTest,
CLI-contract, and format logs are `out/logs/m20-loader-status-0015-final-nagi-ctest.log`,
`out/logs/m20-loader-status-0015-final-upstream-ctest.log`,
`out/logs/m20-loader-status-0015-prepatch-cli-test.log`, and
`out/logs/m20-loader-status-0015-prepatch-fmt-check.log`. No complete target
backend or in-guest inference is claimed; M20 remains `PARTIAL`.
Patch `0016-nagi-llama-tensor-requirement-status.patch` moves missing required
weight/meta lookup and required tensor shape failures into sticky loader status
and a null result on Nagi. Optional tensor absence still returns null without
invalidating the loader; ordinary host builds keep throwing for required
failures. The synthetic one-tensor GGUF regression checks missing weight,
missing metadata, required missing tensor, wrong shape, and the optional-missing
case. Fresh generated-cache builds and CTests pass 1/1 in both Nagi-macro and
ordinary host configurations; all 179 CLI library tests and `cargo fmt --check`
pass.

Fresh `./nagi fetch` applied patches 0001–0016 with patch fingerprint
`fnv1a64:247231d8ca687200` and checkout fingerprint
`fnv1a64:2dc66e2176707e4b`; reverse `git apply --check` confirms patch 0016 is
present. Fetch stopped at the pre-existing modified Servo checkout without
touching it. The full pre-fetch generated checkout and its verified 3,674-entry
manifest are under
`out/evidence/m20-loader-status-0016-20261002/generated-working-copy/`.

The focused no-exceptions target compile could not reach loader diagnostics in
this environment: the previously used Homebrew LLVM libc++ path is absent, and
the available Command Line Tools libc++ stops in its availability/threading
headers with 20 errors before compiling the loader body. See
`out/logs/m20-loader-status-0016-target-build.log`. No complete target backend
or in-guest inference is claimed; M20 remains `PARTIAL`.
Patch `0017-nagi-llama-tensor-construction-status.patch` adds a nonthrowing
tensor-info lookup for Nagi and routes missing mappings, buffer-selection
failure, unavailable CPU backend, and context-allocation failure into sticky
loader status with null propagation. The existing `llm_tensor_info_for()` host
exception contract is retained. The loader-bounds CTest now verifies unknown
tensor-info lookup returns null while the legacy accessor still throws. Fresh
generated-cache builds and CTests pass 1/1 in Nagi-macro and ordinary host
configurations; all 180 CLI library tests and `cargo fmt --check` pass.

Fresh `./nagi fetch` applied patches 0001–0017 with patch fingerprint
`fnv1a64:ef325ed7869529d9` and checkout fingerprint
`fnv1a64:c33528fd50bdb517`; reverse `git apply --check` confirms patch 0017 is
present. Fetch again stopped at the pre-existing modified Servo checkout and
left it untouched. The pre-fetch generated checkout and all 3,674 entries in
its verified manifest are under
`out/evidence/m20-loader-status-0017-20261002/generated-working-copy/`.

The no-exceptions target check remains unverified because the available
Command Line Tools libc++ fails in availability/threading headers before
compiling the loader body (20 errors); see
`out/logs/m20-loader-status-0017-target-build.log`. No complete target backend
or in-guest inference is claimed; M20 remains `PARTIAL`.
Patch `0018-nagi-llama-model-architecture-status.patch` routes unsupported
model architectures and unsupported tensor-split mode to null/status failures
under `__NAGI__`, while preserving the ordinary host exceptions. The
loader-bounds regression checks both target and host contracts. Fresh generated
cache builds and CTests pass 1/1 in both Nagi-macro and host configurations; all
181 CLI library tests, `./nagi test`, `./nagi fmt`, and `./nagi lint` pass.

Fresh `./nagi fetch` applied patches 0001–0018 with patch fingerprint
`fnv1a64:5ed73ca7fd0df78e` and checkout fingerprint
`fnv1a64:f501486afee21d41`; reverse `git apply --check` confirms patch 0018 is
present. Fetch stopped at the pre-existing modified Servo checkout without
touching it; `third_party/llama.cpp` remains clean. The preserved pre-fetch
generated checkout and its verified 3,674-entry SHA-256 manifest are under
`out/evidence/m20-loader-status-0018-20261002/generated-working-copy/`.

Using the installed LLVM 19 compiler and libc++ explicitly, the Nagi
no-exceptions compile reaches `llama-model.cpp` and reports 15 remaining throw
diagnostics across 13 source sites; the architecture throw paths covered by
patch 0018 are gone. See
`out/logs/m20-loader-status-0018-final-target-model-build.log`. The remaining
model initialization, metadata, buffer, and context failure paths continue in
the next M20 slice. No complete target backend or in-guest inference is
claimed; M20 remains `PARTIAL`.
Patch `0019-nagi-llama-model-initialization-status.patch` converts the
remaining `llama-model.cpp` no-exception paths into metadata/load status
failures and checked returns, including backend selection, model metadata,
expert configuration, buffer allocation, and buffer-probe failure. The
control-vector caller checks the null buffer result. Host exceptions are
preserved. Fresh generated-cache builds and CTests pass 1/1 in both
Nagi-macro and host configurations; 182 CLI library tests plus
`./nagi test`, `./nagi fmt`, and `./nagi lint` pass.

Fresh `./nagi fetch` applied patches 0001–0019 with patch fingerprint
`fnv1a64:6302e19cdb8006d2` and checkout fingerprint
`fnv1a64:e2ddc41301611afa`; reverse `git apply --check` confirms patch 0019 is
present. Fetch stopped at the pre-existing modified Servo checkout without
touching it; `third_party/llama.cpp` remains clean. The final pre-fetch
generated checkout and its verified 3,674-entry SHA-256 manifest are under
`out/evidence/m20-loader-status-0019-20261002/final-before-fetch/`.

Patch `0020-nagi-llama-adapter-status.patch` now propagates LoRA metadata,
tensor, allocation, and shape failures as checked status under `__NAGI__`,
while ordinary host builds retain exception behavior. A malformed-GGUF runtime
regression returns null in both Nagi-macro and host CTests (1/1 each). The
LLVM 19/libc++ no-exceptions object compile for `llama-adapter.cpp` passes.
Fresh fetch applied patches 0001–0020 with patch fingerprint
`fnv1a64:2a9f96a4e4f49b03` and checkout fingerprint
`fnv1a64:5260a9eed933399a`, then stopped at the existing modified Servo
checkout without touching it. The pre-fetch generated checkout and verified
3,674-entry manifest are under
`out/evidence/m20-loader-status-0020-20261002/pre-fetch/`; the fresh generated
checkout manifest is under
`out/evidence/m20-loader-status-0020-20261002/fresh-generated/`.

Patch `0022-nagi-llama-model-load-status.patch` propagates remaining model
metadata and tensor-load failures through checked status under `__NAGI__`,
while ordinary host builds retain exceptions. MoE expert-count checks now run
before tensor construction and fallback divisions; the shared no-active-expert
metadata case returns a checked failure instead of aborting. Fresh `./nagi
fetch` applied patches 0001–0022 with patch fingerprint
`fnv1a64:f54a90214cdfbdf3` and checkout fingerprint
`fnv1a64:ea53c409e97f602c`, then stopped safely at the pre-existing dirty Servo
checkout. The raw pinned `third_party/llama.cpp` checkout remains clean. Verified
3,649-file manifests are under
`out/evidence/m20-loader-status-0022-20261002/{pre-fetch,fresh-generated}/`;
only the generated marker differs between the snapshots.

Fresh generated-cache host and Nagi-macro `llama` plus loader-bounds targets
build, and the focused loader-bounds CTest passes 1/1 in each configuration.
All 185 CLI library tests and `./nagi test`, `./nagi fmt`, `./nagi lint`, and
`./nagi build` pass. The full LLVM 19/libc++ no-exceptions Nagi `llama` target
now compiles past the previous 0021 model failures, then stops in unity units
1–4 with 44 reported throw diagnostics across 19 other model files; unity 1
hits Clang's error limit. See
`out/logs/m20-loader-status-0022-noexceptions-target-build-fresh.log`.
No complete target backend or in-guest inference is established; M20 remains
`PARTIAL`.

**M21 evidence:** Added `NagiPlan@1`, a bounded generative planner adapter,
DecisionProvider/LLM routing, context visibility filtering, deterministic
capability/object/parameter validation, and sequential partial-failure
execution. The registry now has a real `file.search` handler that delegates to
the M19 SearchService, returns at most 64 authorized Object IDs, and relies on
its injected visibility filter plus executor policy checks. Host tests,
warnings-denied Clippy, formatting, and Nagi user-target compilation pass. The
Executor now also passes validated plan intent to action handlers for the M22
ledger bridge. M21 remains `PARTIAL`: the M19 guest fixture now proves
`file.search` through the real target Validator/Executor and SearchService,
including foreign fixture caller denial. The M22 guest fixture also executes
a bounded `file.move` plan
through ContextResolver, Validator, the Action Registry, capability/object
checks, and Executor for three actual VFS files. App launch, general file
copy/move, and volume handlers are absent; a production AI service,
authenticated target policy, and production guest acceptance remain.
The M22 guest fixture now also executes a fixed-destination `file.copy` plan
against that VFS, checks denied `files.copy` and path-injection cases, and
records a recoverable Create in NH16 plus a separate NAL1 Activity record. It
is limited to one 512-byte fixture file and does not add a production Files
handler.
The 2026-10-01 Priority A audit confirmed that libnagi's ServiceRegistry
directly calls in-process handlers and AI caller IDs are logical request data
without a production authenticated provider. The 2026-10-02 bootstrap Channel
syscalls add user ABI plumbing but still expose only the shared-address-space
`nagi-init` Process (PID 1). Adding production policy without process-launch
identity, address-space isolation, and supervisor-authorized endpoints would
weaken the capability boundary; details are recorded in the M21 workstream.
**M22 evidence:** The existing M15 History Service now has a versioned `NH16`
recoverable archive contract, full-width logical caller context, grouped move
transactions, prepared/committed states, reverse-order composite undo, and
restart recovery of pending undo metadata. The initial three-file guest VFS
mutation runs as one M21 `file.move` action: the validated intent reaches the
handler, fixture context and `files.move` capability/object checks pass, NH16
Prepared is persisted before mutation, and Committed is reopened and verified
from guest VFS. A separate bounded `NAL1` AI Activity Ledger archive records
intent, optional model, action/plan summary, context, objects, transaction,
and result transitions in checksummed `NLA1` two-slot guest files. Fresh-disk
`./nagi m22` passed on 2026-09-30: boot 1 persisted and reopened NH16/NAL1
Committed records, boot 2 persisted reverse-order Undo and NAL1
UndoPending/Undone, and boot 3 verified restored files and the complete ledger
after restart. Fresh-disk logs and pre-run images, disks, OVMF vars, and logs
are preserved under `out/evidence/pre-m22-ai-activity-ledger-m28-20260930/`;
the later M28 rerun's current logs remain in `out/logs/`. The outputs replaced
by the fresh-disk run are preserved under
`out/evidence/pre-m22-ai-activity-ledger-20260930/`.
The 2026-09-30 Completion Sweep rerun of `./nagi m22` also passed all three
boots; its invocation log is
`out/evidence/completion-sweep-regression-20260930/m22-qemu.log`. The prior
M19/M22 artifacts and serial logs were copied and hash-verified before rerun.
After the M20 reader-fixture changes, `./nagi m22` passed all three boots again
on 2026-10-01; the serial logs are
`out/logs/m22-history-boot-1.log` through `out/logs/m22-history-boot-3.log`.
The latest fresh-disk M22 run passed grouped `file.move`, fixture `file.copy`,
NH16 Create/Move transactions, separate NAL1 records, Undo of both transactions,
and final restart verification. Its unique image, User Data disk, OVMF vars,
four logs, and seven-file SHA-256 manifest are preserved under
`out/evidence/m22-file-copy-1790854068023718000/` and the associated unique
paths in `out/artifacts/` and `out/logs/`. Seventeen History/Activity Ledger
tests, 24 AI tests, 152 CLI unit tests and 21 CLI integration tests pass, along
with warnings-denied touched-package Clippy, formatting, target check, and the
Nagi build. `NH15` remains the M15 compatibility serializer. M22 remains
`PARTIAL`: the executed plan and policy are deterministic fixture input, not
real AI inference or authenticated production authority, and the ledger
connection is fixture-local rather than a production Activity Ledger
service. The separate M15 regression reached M14 playback but host QEMU had
no capture driver, so its History path did not run.
**M23 evidence:** Added a bounded public Browser Context API boundary,
authorized logical app/object/workspace context, and explicit untrusted-page
provider input. Twenty-four `nagi-ai` tests, warnings-denied Clippy,
formatting, and the Nagi no-std target compile pass. Live Servo extraction,
authenticated guest policy/IPC, Nagi Bar UI, and real inference remain, so the
page-summary acceptance is unmet. See
`docs/workstreams/NagiOS_M23_Nagi_Bar_Context_Albert_AI_Workstream.md`.
**M24 evidence:** Added bounded multilingual UTF-8 chunking, provider-neutral
embedding-space identities, `PersistentVectorIndex` snapshots, and
visibility-filtered `SearchService` semantic indexing/query orchestration.
Twenty-nine `nagi-search` tests, warnings-denied Search and CLI Clippy, changed
package formatting, CLI tests, and the Nagi no-std target compile pass. M19
two-boot and M22 three-boot QEMU regressions restored the guest semantic index
from the User Data VFS; the guest provider is deterministic test data, not
inference. A multilingual embedding model, producer synchronization, hybrid
ranking/explanations, stale-index invalidation, and formal natural-language
QEMU acceptance remain. See
`docs/workstreams/NagiOS_M24_Embedding_Semantic_AI_Workstream.md`.
**M25 evidence:** Added a bounded no-std push-to-talk coordinator with explicit
permission and indicator ordering, PCM framing limits, provider-unavailability
cleanup, empty-transcript rejection/output clearing, and a target AudioService
capture adapter. A replaceable TTS provider and bounded synthesis service now
validate UTF-8 input, stream at most 1 MiB of aligned PCM through a target
AudioService playback sink, and clear buffers on failure. Fourteen `nagi-audio`
tests, warnings-denied host Clippy, Nagi-target audio compile, target `nagi-init`
build, and changed-package formatting pass. The `./nagi m25` QEMU fixture
verifies permission/indicator order, bounded fixture capture, unavailable STT
cleanup, empty-transcript rejection, and fixture TTS playback through the
provider contract; it uses no real microphone, STT model, or TTS engine.
whisper.cpp is pinned at `927cfce34f31707e17f2bff35c349632fb9e2c3a`; a
Nagi-owned no-exception/CPU-backend patch is applied to a generated checkout by
`./nagi fetch`, while the raw upstream checkout remains clean. On 2026-10-01,
`./nagi fetch`, `./nagi test`, `./nagi fmt`, `./nagi lint`, `./nagi build`, the
Nagi-target `whisper` CMake build, the host GGUF metadata/writer regression,
and the `./nagi m25` QEMU fixture passed. CMake and regression evidence is in
`out/evidence/m25-whisper-target-compile-20261001/`; before/after guest images,
disks, OVMF variables, and logs with verified SHA-256 manifests are in
`out/evidence/m25-whisper-noexceptions-pre-final-rerun-20261001/` and
`out/evidence/m25-whisper-noexceptions-final-pass-20261001/`. The Whisper small
multilingual metadata remains pinned to immutable repository revision
`5359861c739e955e79d9a303bcbc70fb988958b1`, 487,601,967 bytes, SHA-256
`1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b`, and MIT
in `third_party/models.lock`; the exact locked bytes were downloaded to the
ignored model cache and independently size/hash verified on 2026-10-02. They
have not been loaded and no inference has run. Download evidence is recorded
at `out/evidence/m25-whisper-model-download-20261002/`. The QEMU host has no
`virtio-sound.in` driver. A fresh run at
`out/logs/m25-voice-1790854850975922000.log` also passed the empty-transcript
rejection marker; image, User Data disk, OVMF variables, and both serial logs
have a verified five-entry manifest at
`out/evidence/m25-empty-transcript-1790854850975922000/manifest.sha256`. M25
remains PARTIAL: authenticated
permission/UI wiring, a real Japanese STT provider and inference, a concrete
local TTS engine, and real guest voice-command acceptance remain. See
`docs/workstreams/NagiOS_M25_Voice_Workstream.md`.
**M26 evidence:** Added deterministic role/capability/resource/provider-health
model routing, strict manual override checks, and unavailable-provider
fallbacks while retaining Granite as the Standard default. Qwen/Gemma pins are
checked against their manifest fixtures. On 2026-10-02, the Qwen artifact at
the locked revision passed host and guest size/SHA-256 verification in a
disposable read-only Model Store image (`out/evidence/m26-qwen-artifact-1790897766859340000/`);
no backend was loaded or inference performed. The Gemma guest feature compiled,
but its weights were not obtained or used because the explicit terms
acknowledgement was not given; the CLI now requires an invoker to pass
`--accept-gemma-terms`. Qwen/Gemma metadata tests, 167 `nagi-cli` unit tests,
21 integration tests, `./nagi fmt`, `./nagi test`, `./nagi lint`, `./nagi build`,
and both M26 Nagi-target feature builds passed. Fresh M19 Search and M22
three-boot grouped-Undo regressions passed, with evidence at
`out/evidence/m19-qwen-regression-pass-20261002/` and
`out/evidence/m22-regression-m26-qwen-20261002/`. The original fixed-name M19
User Data disk was restored byte-for-byte after regression. M26 remains PARTIAL:
model loading/inference, complete packages and Gemma terms/notice review,
guest provider routing, switching UI, and real routing acceptance remain. See
`docs/workstreams/NagiOS_M26_Model_Routing_Workstream.md`.
**M27 evidence:** The bounded A/B boot-control state machine has a
three-attempt trial limit and checksummed two-copy journal backed by two
Nagi-namespaced UEFI non-volatile variables. BootInfo v4 and one-shot,
no-argument `SYS_BOOT_READY` record guest readiness after M6/M7 checks and the
first successful M10 desktop presentation; the next loader confirms only an
exact slot/attempt/generation match. Fresh `./nagi m27` QEMU acceptance rejected
malformed System B on attempts 1–3 and rolled back to persistent A; a healthy B
trial was promoted on the next launch and remained confirmed on a third. The
same run booted Recovery despite invalid A/B kernels, verified its read-only
VFS check and bounded console, and showed Recovery left the journal unchanged.
A real guest M21/M22 three-file `file.move` produced NH16/NAL1 Committed state;
Recovery's explicit NH16 Undo restored all files, and a subsequent M22 guest
restart verified the restored files and NAL1 Undone state. The next automatic
boot still began trial 1/B. The completion-sweep rerun passed after the
Recovery entry-point and image-builder request refactors; evidence, OVMF
variables, and the data disk are at
`out/evidence/m27-ab-rollback-1790740066889678000/`. The follow-up regressions
after adding the GPT loader/kernel path passed at
`out/evidence/m27-ab-rollback-1790744128241974000/` and, after narrowing the
legacy fallback to exclude a separate GPT User Data disk, at
`out/evidence/m27-ab-rollback-1790744869754176000/`. BootInfo (15), ABI (4),
CLI (133 unit, 18 integration), loader (10), feature-enabled loader/kernel/init
builds, formatting, and the complete M27 QEMU acceptance pass. The read-only
checker validates VFS geometry, accounting, and reachable files without write
or format APIs; the acceptance now permits additional valid user files while
still checking the M7 marker bytes. The M30 reference GPT image and System A
boot now use the production M27 journal feature. Fresh `./nagi m27` acceptance
also passed the same reference GPT layout: malformed B failed three trials,
Recovery preserved the journal and passed its read-only VFS check, A rollback
read the GPT User Data volume, and a healthy B trial was promoted after
Recovery. Evidence for the full run is in
`out/evidence/m27-ab-rollback-1790750679606495000/`. A 2026-10-01 QEMU rerun
adds a read-only Recovery `history` command that lists the NH16 sequence,
transaction, operation, state, and Object ID for at most 16 records. The
acceptance displayed the three real `file.move` records as `COMMITTED` before
Undo; the full acceptance evidence is in
`out/evidence/m27-ab-rollback-1790810805162353000/`. M27 remains `PARTIAL`:
the current readiness point is before account login, slot manifests are not
authenticated, and no authenticated GPT updater exists. See
`docs/workstreams/NagiOS_M27_AB_Recovery_Workstream.md`.
During the current-head integrated rerun, a complete M27 gate passed in
repetition 1. Later full attempts timed out before BDS output on healthy-B
promotion or Recovery boots; replaying the saved healthy-B OVMF state reached
confirmed B and M10 desktop READY. The failures are preserved and not counted
as additional passes; their cause remains unconfirmed. A subsequent M28
one-repetition run passed the full GPT A/B and Recovery gate, including
healthy-B promotion; evidence is in
`out/evidence/m27-ab-rollback-1790788040114277000/`. Its preceding GUI Recovery
timeout is preserved at
`out/evidence/m27-ab-rollback-1790787806085590000/`. Headless and GUI timeout
paths now attempt bounded QMP status and CPU-register diagnostics, but the
successful rerun did not exercise this new failure path.
**M28 evidence:** The stress harness uses the current M19 VFS/ObjectId artifact
namespace, validates the live file/ObjectId marker, accepts a previous-boot
marker from either initial or restart logs, and archives generated images,
OVMF variables, logs, and disk snapshots between repetitions. A fresh-disk
M22 run separately passed the M21 `file.move` action, NH16 commit, separate
NAL1 Activity Ledger commit, three-file Undo, and restart verification. After
preserving the existing outputs and disks under
`out/evidence/pre-m22-ai-activity-ledger-m28-20260930/`, a real
`NAGI_M28_REPEAT_COUNT=1 ... --run` passed the M19 guest gate and all three M22
boots with the required NAL1 undo marker. Latest serial logs remain under
`out/logs/`. Shell syntax, self-test, and the real one-repetition run pass.
M28 remains `PARTIAL`:
the combined Desktop/Files/Notes/Albert, Granite, audio, Semantic Search, OOM,
fairness, and leak workload has not been measured. QEMU reported that no host
virtio-sound input driver is available; these Search/History gates did not
exercise audio. The latest two-repetition run passed M19 and M22 in repetition
1, then M27 boot 4 timed out during the M3 SMP transition; no multi-repetition
pass is claimed. Evidence and the unconfirmed diagnosis are recorded below.
See
`docs/workstreams/NagiOS_M28_Integration_Stress_Workstream.md`.
An earlier two-repetition M28 attempt passed all three gates in repetition 1
and M19 in repetition 2. Repetition 2's M22 boot 1 timed out
before guest output; a standalone M22 rerun passed all three boots. Subsequent
M27 retries exposed the same kind of pre-BDS QEMU timeout. The attempt did not
complete as a two-repetition pass; logs, disposable QMP replays, and hashes are
preserved under `out/evidence/m28-repetition-2-m22-timeout-20261001/`.
**M29 evidence:** Added cross-linked root, Developer Preview, SDK, contribution,
and roadmap documentation, with explicit setup, provider, recovery, language,
accessibility, diagnostics, package, licensing, and SDK boundaries. A local
audit resolved 47 relative documentation links and `./nagi --help` printed the
actual supported command list under the pinned rustup toolchain. The host doctor
now detects `python3` without requiring a `python` alias; the regression test
and real macOS `./nagi doctor` run pass (12/12). Three M10 desktop captures and
a three-run persistent-boot timing sample are recorded in the M29 workstream.
The shared `nagi-localization` no-std library now embeds initial UTF-8 `en-US`
and `ja-JP` catalogs with canonical locale parsing, stable-key lookup, English
fallback, and safe unknown-key text. Its five host tests cover translations,
missing-entry fallback, invalid codes, unknown keys, and matched first-party
catalogs. The M10 desktop has a Settings System language selector and
localized title/label preview. It stores the strict `en-US` / `ja-JP` value in
the User Data VFS file `system-language` and loads it before the first Desktop
frame. The 2026-10-01 `./nagi m29` QEMU acceptance selected Japanese, passed all
prior M10 focus/input markers, and verified `ja-JP` after a second guest boot
with the same User Data disk. `./nagi desktop`, `./nagi m19`, three-boot
`./nagi m22`, `./nagi m27`, and `./nagi m30` regressions passed after the
change. The CLI suite passes with 151 unit tests, including the new language
persistence contract checks; the focused
localization/CLI suite and warnings-denied Clippy passed. M29 remains
`PARTIAL`: selected System language persists for the Desktop, but is not
propagated to other services; first-run, full localization and accessibility,
broader product screenshots and clean-install performance
evidence, and end-user recovery/error UI remain incomplete. Binary
redistribution also awaits license review. See
`docs/workstreams/NagiOS_M29_Developer_Preview_Polish_Workstream.md`.
**M30 evidence:** The reference-image path emits a self-contained 64 GiB qcow2
with GPT ESP, System A/B, User Data, Recovery, and Model Store partitions. The
UEFI loader locates System A by GPT partition GUID. The kernel validates
primary/backup GPT metadata and exposes bounded writable User Data plus a
separate read-only Model Store capability. On current clean source
`25e0b5443363f87a4e503a3031cb9804f3e29c07`, `./nagi m30` rebuilt the image at
SHA-256
`47615fce4e0b7442f1d016add408eb84c120b6fb5ad0dcd85b00c517e4de2d41`; its
sidecar binds that digest to the full source revision. QEMU run
`1790904245966571000` passed System A, User Data restart persistence, M19
Search/ObjectId, M21 `file.search` fixture, M22 grouped Move/Copy and Activity
Ledger Undo, Recovery, rejection of unstaged System B, post-Recovery
Search/Undo, and the separate M20 FAT32 Model Store reader fixture. The
12-entry evidence manifest verifies at
`out/evidence/m30-release-1790904245966571000/SHA256SUMS`. `qemu-img check`
passed for the pristine image, writable acceptance copy, fixture, and assembled
bundle image. Clean-source release preflight, assembly to
`out/artifacts/m30-release-bundle-25e0b54/`, and verify passed; all 14
release-tool tests and all 22 bundle checksums passed. The bundle image matches
the pristine reference byte-for-byte. QEMU booted a disposable copy of the
reference image; the assembled bundle remained untouched and verified. The
bundle manifest retains `m30_acceptance=NOT_EVALUATED`. QEMU had no
`virtio-sound.in` host input backend, so audio is untested and no inference is
claimed. M30 remains `PARTIAL` for authenticated updates, remaining M18–M29
acceptance, and human binary redistribution review. See
`docs/workstreams/NagiOS_M30_Release_Workstream.md` for earlier checkpoints.
The release tool's manifest still records guest acceptance as
`NOT_EVALUATED`; the external QEMU evidence is kept separately. On clean source
commit `144cc0d`, release preflight, assembly, and verify passed. Two boots of
`out/evidence/m30-clean-release-144cc0d/release-package-qemu-copy.qcow2`
verified System A and User Data format/write then persistent read. The copy
changed to SHA-256
`7e3266b576f129dabe2848bfc1c76f0a52b6ee19c4ab49aac85bc65867437725`; the
assembled package remained
`461c644d48e4b0d33b937ce6852eb9a6034abe391ea74c4c24e1ac0b99ca2d43`, and
post-boot `release.py verify` plus `qemu-img check` passed. The target CI job
now runs `./nagi m30` after M27 to build or validate the reference qcow2 and
exercise its two-boot System A/User Data persistence acceptance. M30 remains
`PARTIAL`: authenticated update/slot manifests, M18–M29 completion, and binary
license/notice review remain. The production M27 loader boots confirmed GPT
System A, and the M27 QEMU acceptance covers malformed/healthy System B,
Recovery, rollback, and readiness promotion on the same layout. See
`docs/workstreams/NagiOS_M30_Release_Workstream.md` and
`docs/decisions/ADR-0013-m30-reference-disk-layout.md`.
**Host workflow validation — 2026-10-01:** On non-x86_64 hosts, `./nagi
build`, `test`, and `lint` now select the host-compatible packages explicitly,
so x86_64 syscall stubs are not compiled for the host. x86_64 keeps the full
host workspace checks, excluding the kernel. On this ARM64 macOS host,
`./nagi test`, `./nagi build`, and `./nagi lint` passed; the CLI suite passed
with 135 unit and 21 integration tests. The changed CLI package's format check
and `./nagi fmt` both passed; the latter uses the same selected source packages
as CI and leaves vendored Servo formatting untouched.
**Completion Sweep audit — 2026-09-30:** M19 Search and M22 grouped Undo
passed QEMU regression after the M27 readiness and read-only VFS checker
changes. The low-level Channel core now attaches the sending kernel `ProcessId` as receive
metadata, with a regression proving a forged payload ID does not alter it.
This does not add user Channel syscalls or establish application/session
authentication: the bootstrap still has one shared-address-space init process
and no trusted process-to-AppId/session binding. M18–M23 production service
identity remains an open shared prerequisite; see the M19 workstream and
ADR-0002.
The M25 guest orchestration fixture also passed `./nagi m25`: denied permission,
indicator-before-provider ordering, bounded capture, cleanup after an
unavailable STT provider, and the bounded TTS provider/playback contract were
verified on QEMU without a real audio device or speech model. The rerun migrated
the preserved 16 MiB M25 User Data image to the current GPT form; its prior
image, disk, variables, and logs are hash-preserved under
`out/evidence/m25-tts-contract-20260930/pre-run/`.
After the final TTS empty-output guard, `./nagi m19` passed persistence/Search
and `./nagi m22` passed three-boot grouped Undo. Their outputs replaced by that
regression are hash-preserved under
`out/evidence/m25-tts-contract-20260930/pre-m19-m22/`; the M28 harness
self-test and dry-run also pass against the latest logs.
The regressions passed again after M27 readiness changes: `./nagi m19` verified
search/ObjectId across rename and restart, and `./nagi m22` verified grouped
Undo and the separate Activity Ledger across three QEMU boots. Current logs are
`out/logs/m19-vfs-objectid-initial.log` and
`out/logs/m22-history-boot-3.log`.
The M27 GPT run exposed that killing QEMU immediately after a serial marker
could lose pending qcow2 User Data writes. Headless and GUI acceptance runners
now use QMP `quit` after observing the marker; headless QEMU keeps its file
serial backend to preserve UEFI behavior. `./nagi m27` passed twice with the
durable GPT VFS check, and `./nagi m30` passed initial boot and restart with
confirmed System A and persistent reads. A fresh blank-image run then passed
format/write and restart/read using a disposable copy; clean-commit release
preflight/assembly/verify passed, and the assembled package's byte-identical
copy passed two QEMU boots without changing the package checksum. Evidence is
under `out/evidence/m30-clean-release-144cc0d/` and
`out/evidence/m30-release-1790751471624505000/`.
**Next action:** ADR 0044 and ADR 0045 moved M19 Search and M21/M22
`file.search`/`file.move` callers onto the isolated-process boundary. Next:

- ADR 0046 now provides the Supervisor launch registry with manifest-defined
  grants;
- move the remaining in-process M21/M22 fixtures and the M27/M30 images to
  the registry path;
- bind manifests to signed M16 packages;
- ADR 0047 contains ring-3 faults to a child-only exit and fixes the M3
  SMP stall;
- add a Supervisor process-exit wait/status. Earlier note: Continue the highest-priority shared
Service/IPC/Capability boundary audit and implementation for M18–M23, reusing
the existing foundations and preserving fixture-only caller identity. Continue independent
M20, M22–M29 work while authenticated update and provider dependencies remain.

The macOS build failure was a host/target linker mismatch: Mesa's target
configuration probes GNU ELF link flags including `-latomic`, while Darwin's
native linker emits Mach-O. A Darwin-only target-link adapter routes those
ELF links to ELF LLD. Compile-only calls and host build helpers retain their
normal compiler paths. Linux keeps the existing Ubuntu Clang/LLD cross file;
CI run `36533931477` confirms the Mesa, Servo target build, and QEMU paths
still pass there with `-latomic` enabled. Local QEMU verification used
Homebrew LLVM 19 and matching libc++ headers because this Mac's Apple Clang
21 SDK headers do not match the pinned target libc++ flags; that host
compiler issue is separate from the linker adapter.

**Last updated:** 2026-10-02
**Latest continuation CI:** Run
[`36615323040`](https://github.com/RT-NISH/NagiOS/actions/runs/36615323040)
passed Ubuntu host, Windows launcher, and the Nagi target gates on base commit
`db2ffb7bc4a51cb1455efc194ccc843ab08d7203`, including M17 first-web-pixel,
M18-B chrome, and M18 three-site HTTPS/QEMU acceptance. This is the predecessor
baseline; M19 live VFS and M22 regression steps were added to CI in the current
checkpoint and await its new run.
**Last known checkpoint:** The user-directed continuation remains on
`codex/m19-m22-continuation` in
`/Users/tozawa/.codex/worktrees/m19-m22-continuation/NagiOS`. On 2026-09-30,
`./nagi m19` passed on a fresh isolated disk through bootstrap, initial, and
restart boots, indexing a real VFS file and verifying its fixture ObjectId
after rename and restart. A fresh-disk `./nagi m22` run passed M21 guest
`file.move` Plan/Validate/Execute on boot 1, verified the persisted three-file
NH16 Committed transaction, applied composite Undo on boot 2, and verified
restored files plus Undone state on boot 3. One updated M28 Search/History
repetition passed afterward. The fresh M22 action logs and disk snapshot are
preserved under `out/evidence/pre-m28-m21-file-move-20260930/`; the M28
final serial logs remain in `out/logs/`. M19 production
IPC/capability, live Files/page producer integration, real AI inference,
authenticated M21/M22 authority, and production Activity Ledger integration
remain open.
Focused `cargo test --locked --offline -p nagi-cli -p nagi-search -p nagi-ai
-p nagi-history --all-targets` passed 193 tests total (114 CLI unit, 18 CLI
integration, 23 Search, 24 AI, and 14 History/Activity Ledger). The M19
Nagi-target check,
package-only target Clippy, formatting,
M28 shell syntax, self-test, dry-run, and one-repetition run passed. The M22
`nagi-init` Nagi-target check and target package Clippy with warnings denied
also passed, as did the subsequent 114 CLI unit and 18 integration tests. Full target
Clippy also reports a pre-existing `clippy::not_unsafe_ptr_arg_deref` error
in `user/nagi-posix/src/lib.rs:572`; the affected POSIX code was not changed.
`./nagi m18` build/acceptance path, a guest runner that records normal TLS
verifier success and real Servo frames, and the M18-B browser state/chrome
modules from checkpoint `ee49b812fa69c943c34ca076fe795e6ba92e504f`. The guest
routes pointer and key events through bounded chrome hit testing and action
dispatch; QEMU acceptance is set to enter `example.com` through the address bar
before the three-site sweep. Page input is forwarded to Servo. The M18 feature
also includes M18-A's nonblocking POSIX socket/smoltcp path and UEFI realtime
seed from checkpoint `330f322fbfd1c8fc8e696183fc7f13a019644804`; source
integration has now built and run in the target. Host-side acceptance,
browser-state, input-adapter, network, and clock tests pass. Per-tab WebView
ownership is part of the target build; the current QEMU acceptance exercises
the primary tab and address-bar navigation. Bounded session/history/bookmark
persistence is connected through the POSIX VFS, with its ABI enabled only by
the M18 feature. The M18 QEMU acceptance passes locally on macOS after a
Darwin-only ELF-linker adapter was added to the target-link paths. The adapter
is selected only for Darwin target links: Mesa gets a generated Meson cross
file, and `nagi-target-cc.sh` uses the same adapter for link invocations;
compile-only calls and host build helpers keep their normal host compiler.
The original `-latomic` probe remains enabled and succeeds through ELF LLD.
Ubuntu's tracked cross file and Clang/LLD path are unchanged. The fresh
`./nagi m18` rerun on this Mac also passes: the target image boots in QEMU and
renders three TLS-chain- and hostname-verified HTTPS sites through Nagi
Surface. CI run `36512090928` then exposed the Mesa fallback declaration,
host Clippy, and source-contract formatting issues now repaired in the working
tree. Ubuntu CI run `36517686132` passed the initial integrated build and
acceptance; corrected-origin-patch run `36533931477` then passed clean Servo
bootstrap and all Windows, Ubuntu host, and Nagi target gates, including M17
real-QEMU first-web-pixel and M18 three-site HTTPS acceptance.
Clipboard, download/upload, IME text events, and interactive site-permission
decisions still need actual Nagi providers, so overall M18 status remains
`PARTIAL`.

### M17 First Web Pixel completion after Actions run #303 (2026-09-28)

Public CI run #303 ([`36355494134`](https://github.com/RT-NISH/NagiOS/actions/runs/36355494134),
head `31bf815b7230f2658f654643e6d6c898d9881d77`) completed successfully:
the Windows launcher, Ubuntu host, and authoritative Ubuntu `nagi-target`
jobs all passed. The target job built the pinned Servo dependency graph,
Mesa Softpipe, M16 package, kernel, real `nagi-init` link, and UEFI loader,
then ran the unchanged real-QEMU M17 acceptance successfully.

The guest path constructs Servo's `SoftwareRenderingContext`, loads the
bundled local HTML page, paints and reads back the first Servo frame, rejects
a zero checksum, copies the RGBA frame into the capability-checked Nagi
Surface, and requires successful presentation before printing the checksum
and `Nagi M17 first web pixel PASS`. The acceptance script additionally
requires Servo's registered resource reader and verifies ELF constructors
completed before user entry. CI printed both `PASS M17 first web pixel: real
Servo/Mesa Softpipe frame reached Nagi Surface and QEMU` and
`PASS M17 first web pixel acceptance: real Servo guest frame reached Nagi
Surface and QEMU`. The exact numeric checksum and temporary serial log were
not retained as GitHub Actions artifacts; the guest and acceptance checks
require a nonzero checksum before these PASS lines can occur.

This closes M17's formal acceptance, **First Web Pixel on Nagi**. M17 is
`PASS`; M18 remains `NOT STARTED` in this continuation.

### M17 bootstrap mmap capacity after Actions run #302 (2026-09-28)

Run #302 (`36351615434`, head
`fb9ba58f3c0f01b13335a894edd33a1999a92da5`) passed the Windows launcher and
Ubuntu host jobs, target dependency boundary, Mesa Softpipe, M16 package,
kernel, real `nagi-init` link, and UEFI loader. Real QEMU created the Softpipe
GL context, initialized multiple SpiderMonkey GC chunks, and entered
`ScriptThread debugger global creation`. It then rejected a 1 MiB GC mapping
four times with `no contiguous range`, `free_reservation_slots=32`,
`free_pages=4`, and `largest_free_run_pages=4`. The guest asserted because
`JS_NewGlobalObject` received a null context and did not exit before the
unchanged 120-second bound (acceptance exit code 4). No pixel checksum or M17
PASS marker was produced.

The statistics distinguish exhaustion of the 128 MiB mmap/backing capacity
from reservation-table exhaustion. ADR 0036 expands both the finite window and
the statically backed pages to 256 MiB for the official 8 GiB reference QEMU
machine. The 64 reservation identities, page ownership, partial-range VM
semantics, and acceptance gate remain unchanged. The fixed backing store adds
128 MiB to kernel BSS; replacing it with the general physical-frame VM service
is outside this M17 repair.

The kernel boundary tests assert the 256 MiB extent and verify the final page
is accepted while a range crossing `USER_MMAP_LIMIT` is rejected. Local
verification passes all 112 kernel unit tests on x86_64 macOS via Rosetta 2,
`cargo check -p nagi-kernel --lib --tests --target x86_64-unknown-linux-gnu
--locked`, the release `x86_64-unknown-nagi` kernel build, formatting, and diff
checks. The larger BSS and real QEMU behavior await the next public target CI.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### M17 GC mmap rejection diagnostics after Actions run #301 (2026-09-28)

Run #301 (`36347429584`, head
`8818a37382979fc211bf178a1feff5bf1aab7e59`) passed the Windows launcher and
Ubuntu host jobs, target dependency boundary, Mesa Softpipe, M16 package,
kernel, real `nagi-init` link, and UEFI loader. The QEMU acceptance emitted no
first-web-pixel checksum or M17 PASS marker and the guest did not exit before
the 120-second bound (acceptance exit code 4).

The guest trace confirms the partial-unmap fix: the upward prefix `munmap`
completes, the first GC chunk initializes, and a second GC chunk also
initializes. During `ScriptThread debugger global creation`, and again in the
background GC path, a valid 1 MiB chunk allocation reaches `GC base memory
mapping started` and receives `SYS_MEMORY_MAP rejected`. This is not an
expected alignment-hint retry: Nagi's `MapAlignedPages` returns immediately
when this ordinary base mapping fails, and Servo later asserts because
`JS_NewGlobalObject` returned null. The current generic syscall message cannot
distinguish invalid requests, exhausted reservation descriptors, lack of a
contiguous run in the 128 MiB window, PTE mapping failure, or reservation
registration failure. The 64 live reservation identities are a plausible
resource limit, but run #301 does not prove that is the cause.

`kernel/src/user_process.rs` now returns a reason-coded failure with the
requested page count, protection, available reservation slots, total free
pages, and largest free run. `kernel/src/syscall.rs` emits these fields only
when `SYS_MEMORY_MAP` fails; successful mapping behavior is unchanged. The new
kernel regression test distinguishes a full reservation table from an
exhausted address window. All 111 kernel tests pass on x86_64 macOS via Rosetta
2, the x86_64 Linux test configuration checks successfully, and the release
Nagi kernel builds. A new public target run is pending to obtain the guest's
actual resource measurements. No bound has been changed. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### M17 bootstrap partial mmap ranges after Actions run #300 (2026-09-28)

Run #300 (36341631295, head 06392ccf5acc742cb3b2ba70b09e9cea8e5b2e7a) passed the Windows launcher and Ubuntu host jobs, target dependency boundary, Mesa Softpipe, M16 package, kernel, real nagi-init link, and UEFI loader. QEMU timed out with exit code 4; no first-web-pixel checksum or M17 PASS marker was produced.

The GC trace completed the lower-hint mismatch cleanup and the upward-hint mapping, then stopped at SpiderMonkey GC upward prefix unmap started. The POSIX munmap path maps the kernel's exact-whole-region lookup failure to EINVAL, which conflicts with SpiderMonkey's assertion for failed unmaps. This is a partial-range support gap; the trace does not show that the kernel syscall itself blocks.

ADR 0035 preserves the bounded 128 MiB mmap window and 64 live reservation identities while adding a fixed per-page owner map. The kernel now implements page-aligned partial and adjacent-range `munmap`/`mprotect`, keeps `mmap_user_at` restricted to one exact live reservation fragment, and preserves `PROT_NONE` ownership independently of PTE presence. Tests cover partial splits, adjacent reservations, atomic hole rejection, fragment remapping, protection changes, and slot reuse. Local verification passes all 110 kernel tests, x86_64 Linux test compilation, and the release Nagi kernel build. Public Ubuntu CI must confirm the real GC range sequence and continue to first-web-pixel acceptance.

M17 remains BLOCKED; M18 remains NOT STARTED.

### M17 aligned-page allocation diagnostic after Actions run #299 (2026-09-28)

Run #299 (`36337350178`, head
`ee07235c390320a6aa687204dc72319d54284ee9`) passed the Windows launcher and
Ubuntu host jobs, M17 dependency checks, Mesa Softpipe, M16 package, kernel,
the real `nagi-init` link, and UEFI loader. QEMU acceptance exited 4 after the
120-second guest bound, with no first-web-pixel checksum or M17 PASS marker.

The serial trace shows that Nagi selected SpiderMonkey's aligned-page fallback
and completed its first base mapping. It then emitted
`SpiderMonkey GC initial chunk-alignment attempt started` without the matching
completion marker. The available trace cannot distinguish the exact-hint mmap,
its mismatched-address cleanup, a directional partial unmap, or the replacement
mapping inside `TryToAlignChunk`.

Patch `0023-nagi-m17-alignment-traces.patch` adds Nagi-only checkpoints around
those operations. It passes the trace flag only for GC-sized Nagi chunk
allocations and preserves existing mapping calls, order, and success decisions.
Other targets pass the default false trace flag. The source-contract test was
observed failing before patch 0023 existed and passes after it. The fresh
`./nagi fetch` applied the complete pinned MozJS patch series, and reverse-patch
validation passes. Local tests, Clippy, formatting, and diff checks pass; the
public target build and QEMU trace are pending. M17 remains `BLOCKED`; M18
remains `NOT STARTED`.

### M17 bounded GC mapping after Actions run #298 (2026-09-28)

Run #298 (`36332858469`, head
`2e41bb937eda62b1d82ffa079bad4c92990a4231`) passed the Windows launcher and
Ubuntu host jobs, target dependency boundary, Mesa Softpipe, M16 package,
kernel, the real `nagi-init` link, and UEFI loader. QEMU acceptance exited 4
after the 120-second guest bound. The real serial trace advanced through
`Nursery::init`, its configuration and StoreBuffer setup, and the first GC
chunk allocation, then stopped after
`SpiderMonkey GC scattershot mapping started`. No web-pixel checksum or PASS
marker was produced. The trace does not distinguish a stall in random address
selection, mapping, or later scattershot alignment retries.

The pinned source selects scattershot allocation whenever its discovered
address width is at least 43 bits. Nagi's bootstrap ELF starts at 64 TiB, so
that width describes the canonical address base rather than available random
allocation space. Nagi exposes a bounded 128 MiB mmap arena; non-fixed mmap
addresses are hints and the kernel chooses the first free range. The upstream
scattershot path therefore cannot select from the virtual range it assumes.
MozJS patch `0022-nagi-m17-bounded-gc-mapping.patch` makes
`UsingScattershotAllocator()` return false only for Nagi, leaving the existing
aligned-page allocation path and all other target behavior intact. The next
authoritative QEMU run must confirm whether that path completes the first GC
chunk mapping.

The new source-contract test failed before patch 0022 existed and passes with
the patch. `./nagi fetch` regenerated the pinned checkout, the patch passed a
reverse-apply check, and verification passes locally: 90 `nagi-cli` library
tests, 18 CLI integration tests, formatting, Clippy with warnings denied, and
`git diff --check`. Target compilation and QEMU acceptance with patch 0022 are
pending. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### M17 nursery initialization diagnostic after Actions run #297 (2026-09-28)

Run #297 (`36328354764`, head `d39bc0982d1975b6952e89f9a1cc359048b04e55`)
passed the Windows launcher and Ubuntu host jobs, M17 feature-boundary check,
Mesa Softpipe build, M16 package, kernel, `nagi-init` target link, and UEFI
loader. QEMU acceptance exited 4 when the guest did not exit within its
120-second bound. The serial trace reaches `SpiderMonkey GC max-bytes parameter
set completed` and `SpiderMonkey GC nursery initialization started`. This
localizes the stop to entry into `Nursery::init` or its first configuration
read; no nursery-internal marker existed in that run.

Mozjs patch `0021-nagi-m17-nursery-init-traces.patch` adds Nagi-only checkpoints
around nursery configuration, task allocation, StoreBuffer enable, first-chunk
setup, space-vector reservation, arena-chunk acquisition, and GC-sized aligned
page mapping. A focused source-contract test first failed because the patch was
absent, then passed after the patch was added. The next target run must validate
the C++ changes and identify the last completed nursery stage. The complete
tracked patch sequence regenerated from the pinned mozjs source, both patches
pass reverse-apply checks, and generated Nursery/Allocator/Memory files match
the saved patch result. Local validation passes: 89 library tests, 18 CLI
integration tests, formatting, Clippy with warnings denied, and diff checks.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### M17 GC initialization diagnostic after Actions run #296 (2026-09-28)

Run #296 (`36324708375`, head `8cd7103ffa5a4e1ccdf7b78280695912e3194953`)
compiled the updated SpiderMonkey patches into the real `nagi-init` target link
and produced the UEFI loader. The authoritative QEMU acceptance timed out at
120 seconds (exit code 4). Its trace shows helper-thread initialization
returned, then `GCRuntime::init` began without reaching its first inner-stage
checkpoint. This narrows the stall to the GC runtime initializer's entry,
preconditions, or first traced operation.

Mozjs patch `0020-nagi-m17-gc-runtime-init-traces.patch` adds guest-only
checkpoints around GC initialization preconditions, thread-context setup,
helper-thread count update, marker-vector resize, background-allocation locking,
nursery setup, marker and sweep-action setup, atoms-zone setup, zone-vector
reserve, and probe initialization. Its focused source-contract test passed after first failing
against the absent patch. `./nagi fetch` regenerated the pinned mozjs source
using the complete tracked patch order, and `git apply --reverse --check`
confirmed patch 0020 is present. Local validation passes: all 88 `nagi-cli`
library tests, all 18 CLI integration tests, package formatting, Clippy with
warnings denied, and `git diff --check`. Public target compilation and the next
real QEMU trace are pending. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### M17 host Clippy correction and diagnostic run #36309977725 (2026-09-27)

Actions run #36309977725 (`3e9e789594a8826a3adc203953feedccbc4a74aa`) passed
the Windows launcher job. Ubuntu host Clippy stopped before build and tests
because `user/nagi-posix/src/threads.rs` exported `MIN_STACK_SIZE` only for
the target-gated POSIX attribute setter; the host test build excludes that
setter and therefore reported the constant as dead code. The local repair
removes the redundant alias and compares against
`libnagi::BOOTSTRAP_USER_THREAD_STACK_MIN_SIZE` directly. This preserves the
same lower bound and keeps `nagi-abi` as the shared source of truth.

The target job passed Servo bootstrap, the dependency feature boundary, Mesa
Softpipe archive, M16 package, kernel, user-init, and UEFI builds. The real
QEMU acceptance then timed out after 120 seconds (exit code 4). Its diagnostic
excerpt ends at `Servo first event-loop dispatch returned`; no WebView URL or
load-status callback, first frame callback, checksum, or PASS marker appears.
A fresh Mac host Clippy attempt is not a valid substitute:
`libnagi`'s inline x86-64 syscall registers are unavailable to the ARM64 Mac
host target. Local checks after the repair pass for the custom Nagi POSIX
target, the standalone thread-helper harness (3 tests), formatting, and
`git diff --check`. The corrected host lint remains pending public Ubuntu CI.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### M17 Constellation navigation diagnostic after run #36309977725 (2026-09-27)

The #363099 trace moves the stop beyond graphics initialization and Servo
construction: `SoftwareRenderingContext::new`, Mesa context setup, Servo,
WebView, and the first `spin_event_loop` dispatch all return. The guest emits
no URL-change, load-status, or frame-ready callback before the unchanged
120-second QEMU timeout. This does not yet prove whether the Constellation
worker received the `NewWebView` message or where initial pipeline setup
stops.

Nagi-owned Servo patch `0015-nagi-m17-navigation-traces.patch` now places
Nagi-only markers at `NewWebView` receipt, top-level browsing-context setup,
pipeline event-loop setup, and `Pipeline::spawn`. It does not alter navigation
or scheduling behavior. A focused `nagi-cli` source-contract test was first
run against the absent patch and failed as expected; after adding the patch it
passes. The patch applies cleanly to the pinned generated Servo checkout,
and `./nagi fetch` with the pinned nightly regenerated the Servo/MozJS caches
and validated the updated patch set. The full `nagi-cli` library suite passes
(83 tests), including the new contract test; package formatting and
`git diff --check` pass. Public target CI must verify the target compilation
and use the new runtime trace to locate navigation startup. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### M17 Servo ScriptThread stack bound after Actions runs #289 and #363039 (2026-09-27)

The run #289 serial tail shows `Servo::new TLS prewarm completed`, `Servo
constructed`, and `WebView constructed` before the `Constellation` worker
panics at pinned Rust std `library/std/src/sys/pal/unix/thread.rs:80`. That
line asserts the result of the second `pthread_attr_setstacksize` call. The
first call returned `EINVAL`; Rust std rounds the request to a page boundary
and retries. The requested size is already aligned, so the retry reaches the
same rejection.

Pinned Servo source specifies `.stack_size(8 * 1024 * 1024)` for each
`ScriptThread`. Nagi's POSIX stack normalizer accepted at most 2 MiB, and the
kernel independently rejected `SYS_THREAD_CREATE` stacks above 2 MiB. ADR
0034 keeps the 2 MiB default but extends the per-thread maximum to the pinned
Servo requirement of 8 MiB. `nagi-abi` now defines this shared bound for POSIX
normalization and kernel validation. The existing mmap window, stack mapping
checks, thread-pool bound, and first-pixel acceptance remain in force.

The 8 MiB bound advances beyond the previous panic: run #363039 passed target
kernel, user-init link, and UEFI loader stages. Its QEMU trace shows Servo's
worker trampolines running, `Servo constructed`, `WebView constructed`, and
`Servo event loop started`. It shows neither the prior `pthread_attr_setstacksize`
assertion nor a bootstrap thread-slot rejection. The host-side `nagi m17`
command nevertheless timed out after 120 seconds because it did not observe
the unchanged checksum/PASS marker. The captured trace contains no WebView
load-state or frame-ready callback. The next diagnostic adds one-time markers
after the first `spin_event_loop` dispatch and around WebView load, paint,
readback, guest Surface copy, and present.

Local verification passes: `nagi-abi` host tests (2), a standalone harness
that compiles the actual POSIX thread helper (3), `cargo check` for the custom
Nagi kernel target, and `cargo check` for the custom POSIX target. The POSIX
target check reports five existing warnings in unrelated declarations; the
new unused stack-limit warning is gone. The affected-package nightly rustfmt
check, pinned Clippy-driver check for `nagi-abi`, and `git diff --check` pass.
The `manual_is_multiple_of` lint is narrowly allowed on the const ABI
validator because the pinned compiler does not permit that method in const
context. Full POSIX package tests cannot run on this ARM64 Mac because
`libnagi` contains x86-64 syscall-register assembly. Public target
CI must verify that the page load reaches the real first-web-pixel checksum.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### M17 POSIX entropy device after Actions run 36295909293 (2026-09-27)

Pinned-source inspection identifies the exact error emitter as AWS-LC
0.45.0's `aws-lc/crypto/rand_extra/urandom.c`. Servo's TLS prewarm calls
`aws_lc_rs::secure_random.fill()`, which reaches AWS-LC's generic POSIX
provider. The custom Nagi target does not define AWS-LC's Linux raw
`getrandom` path, so AWS-LC opens `/dev/urandom`; Nagi's POSIX adapter sends
that path to the persistent VFS, which correctly reports it absent. The
displayed `Unknown error` comes from relibc's current errno text table and
does not indicate a failed VirtIO RNG request.

The repair exposes `/dev/urandom` as a stateless POSIX descriptor. Reads use
`libnagi::random_fill`, which calls the existing kernel `SYS_RANDOM_GET`
backed by the guest VirtIO RNG. The descriptor does not require a filesystem
mount or persistent file. If the guest entropy request fails, the read returns
`EIO`; it supplies no host or deterministic bytes. M17 remains `BLOCKED` until
the authoritative QEMU run completes TLS prewarm, Servo and WebView startup,
and the real first-web-pixel checksum and PASS marker. M18 remains
`NOT STARTED`.

Local verification: the complete `nagi-cli` suite passes (82 library tests,
18 integration tests), including a source-contract test for the POSIX device
route. The custom-target `cargo check -p nagi-posix --lib
--target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,alloc
--locked --offline` passes with five pre-existing warnings in unrelated POSIX
declarations. `rustfmt --check` on the changed Rust files and `git diff
--check` pass. The full image, UEFI, and QEMU acceptance still require public
target CI.

### M17 storage-thread filesystem repair after Actions run 36284231289 (2026-09-27)

The stopped guest trace in Actions run 36284231289 (run #285, head
`13fde2f8683b00492f589fd4db7f090f48e16e81`) ended at
`Servo::new storage threads started`. The preceding resource thread groups
completed, confirming that the pinned WebPKI-root repair advanced past the
previous blocker. Source inspection found that `ClientStorageThreadFactory`
unwraps `tempfile::tempdir()` and expects `std::fs::create_dir_all()` to
succeed before it calls `thread::Builder::spawn`. M17's custom `_start` had
only run the M7 acceptance's local VFS and never mounted that capability into
`nagi-posix`; the POSIX adapter rejected interior slashes and the VFS resolved
only root entries. This matches the abort point.

The current change initializes `nagi-posix` over the same persistent block
capability after M7 acceptance, ensures `/tmp` exists on that guest volume,
and implements bounded path traversal through actual directory inodes and
their `.` / `..` records. POSIX `stat`, file create/open, `mkdir`, `unlink`,
and `rmdir` now use those paths; the existing root-only directory-stream
boundary remains explicit until directory descriptors are implemented. No
host filesystem or synthetic storage is introduced.

Local verification: `libnagi` storage and the complete library suite pass
(28/28); `nagi-cli` source-contract/library tests pass (80/80), including the
M17 boot ordering check; `rustfmt --check` on the changed runtime files,
`git diff --check`, and an actual custom-target `cargo check -p nagi-posix
--lib --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,alloc
--locked --offline` pass. The target check reports five warnings in existing
POSIX declarations. Running the `nagi-posix` host test binary fails before
test execution because its existing `.data` errno section is not a valid
Mach-O section on this ARM64 macOS host. Full M17 image linking, UEFI build,
and real QEMU acceptance remain for public target CI. M17 remains `BLOCKED`;
M18 remains `NOT STARTED`.

### M17 bootstrap thread-pool capacity after Actions run 36289243570 (2026-09-27)

Run #286 demonstrates that the POSIX storage repair worked: the real guest
prints `persistent storage accepted`, `POSIX filesystem initialized`, and
`temporary directory ready`; Servo then completes its ResourceManager groups
and starts its storage threads. The failing syscall is now specifically the
next user-thread allocation. The kernel's fixed pool has 16 total slots
(initial thread ID 0 plus child IDs 1–15). The guest filled all 15 child slots
with long-lived Servo workers, then a later `pthread_create` returned EAGAIN;
the panic was `Thread spawning failed: Os { code: 11, kind: WouldBlock }` at
`third_party/servo/components/storage/cache_storage.rs:239`.

ADR 0031 supersedes only ADR 0029's 16-slot capacity and sets 32 total slots
(ID 0 plus 31 children), keeping the cooperative scheduler, per-thread TLS,
stack checks, 128 MiB mmap window, and 64-region limit. No host thread pool or
storage/rendering fallback is introduced. The pool's bounded behavior and
reuse must be covered by tests. The latest run's host `Format` job also failed
on rustfmt differences in the changed files; the exact CI formatter commands
pass locally after the format-only correction. Windows build and tests passed.

Verification from run #286: target dependency boundary, Mesa Softpipe, M16
package, kernel, user-init link, and UEFI loader passed. The guest reached the
thread-pool exhaustion described above but produced no first-web-pixel
checksum or M17 PASS marker.

The local implementation now changes the shared ABI count to 32, with the
kernel scheduler, TLS layout, syscall context arrays, and POSIX thread-index
tables deriving their bounds from that constant. Scheduler coverage allocates
all 31 child IDs, checks bounded exhaustion, and verifies slot reuse; a
standalone host harness compiled from the production scheduler source passed
9/9 tests; a standalone harness compiled from the production POSIX thread-index
source passed 2/2 tests using the shared ABI count. The TLS test checks every
control page for the expected address, range, and non-aliasing. The custom Nagi
POSIX and kernel target `cargo check` commands pass, and all CI format checks
plus `git diff --check` pass. The POSIX target check reports five pre-existing
warnings.

The full host workspace test command could not run on this ARM64 macOS
checkout: the environment first selected Homebrew's x86-64 `rustc`, and after
pinning the ARM64 nightly, `libnagi`'s x86-64 syscall-register inline assembly
does not compile for the ARM64 host. The Windows host build/tests and public
Ubuntu checks therefore remain necessary. The next `nagi-target` run must
verify all 32 slots in real QEMU and continue to the first-web-pixel checksum
and M17 PASS marker. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Target evidence from CI Actions run #238 (2026-09-26)

Actions run 36232073962 (#238, head
18ee17af21a2dcf567497832a7e261e0d5f201c1) passed Ubuntu host checks, the
Windows launcher job, Mesa Softpipe, package, kernel, user-init, and UEFI
builds. The real QEMU acceptance completed persistent storage and Mesa/EGL
context creation. Immediately after `Servo construction started`, the kernel
reported `SYS_THREAD_CREATE rejected: child slot occupied`, followed by
Servo's `Thread spawning failed` panic at
third_party/servo/components/profile/mem.rs:48. This rules out child-stack
mmap failure and the thread-create validation branches. No first-web-pixel
checksum or PASS marker was produced. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Local continuation: bounded bootstrap user threads (2026-09-26)

Implemented ADR 0029 in the local branch based on the #238 diagnosis. The
kernel now has a fixed 16-thread cooperative pool, per-thread saved register
and FPU contexts, reusable static TLS pages, detached-thread support, join and
sleep blocking, and bounded round-robin switching at syscall boundaries.
POSIX now maps thread IDs and TLS without aliasing, honors detach and stack
attributes, supports caller-owned mapped stacks, and retains failed stack
unmaps for retry. The guest remains a single process and ring-3 interrupts
remain disabled pending a TSS-backed interrupt path.

Local verification on 2026-09-26:

- The CI package formatting command passed.
- `cargo check -p nagi-kernel --lib --tests --target
  x86_64-unknown-linux-gnu --locked` passed, with an unused-import warning in
  test-only syscall imports.
- The release `x86_64-unknown-nagi` kernel build passed.
- `cargo check -p nagi-posix --tests --target
  x86_64-unknown-linux-gnu --locked` passed.
- The `x86_64-unknown-nagi-user` POSIX target check passed, with existing
  visibility/dead-code warnings.
- Standalone scheduler and thread-helper harnesses passed 9 and 2 tests,
  respectively.
- `git diff --check` passed.

Public CI run 36237832887 (workflow run #252, head
`71fd5c33b53e97251ca4dc0f4569c8944887eb10`) passed Ubuntu formatting,
Clippy, and build, and passed the Windows workspace build. Both host test jobs
then failed on the same stale source-contract assertion in
`tools/nagi-cli/src/mesa.rs:606`, which expected a fixed child TLS address.
The implementation now assigns each thread its own TLS control page. Local
commit `4d79bbe` updates the assertion to require that per-thread mapping;
the focused source-contract test and all 72 `nagi-cli` library tests pass
locally. This repair is not included in run #252. Its target job passed the
Nagi kernel, user-init, and UEFI builds. The real QEMU acceptance timed out
after 120 seconds (exit status 4) during `ServoBuilder::build()`: the final
application trace was `Servo construction started`, with no
`Servo constructed`, pthread-create rejection, kernel thread-create rejection,
panic, first-web-pixel checksum, or PASS marker reported after that point.
Thus the run does not show whether a worker was created or scheduled. M17
remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current diagnostic continuation after CI run #252 (2026-09-26)

Added bounded kernel traces for bootstrap thread creation and each yield,
sleep, join, and exit context switch, including source and destination thread
IDs. If a sleeping thread has no runnable peer, the kernel records the
current tick and deadline before waiting and the observed tick after wake, so
a stalled deadline can be distinguished from a worker that never reaches its
wait. Added POSIX traces for the first 16 pthread trampoline entries and first
16 routine returns. Nagi-owned Servo patch `0011` adds checkpoints inside
`Servo::new` around option/media setup, profiler creation, JavaScript
initialization, paint/resource/storage setup, constellation startup, TLS
prewarming, and final construction. Patch `0012` traces the Servo media and
memory-profiler workers from spawn request through worker entry and
successful profiler construction. The 128-event kernel cap and per-stage
POSIX caps keep the diagnostics finite. Scheduling and acceptance behavior are
unchanged.
This instrumentation distinguishes a synchronous setup stall, a failed
context restore, a child that is never created or selected, a trampoline-entry
failure, and a worker that enters but does not return or yield. It does not
fix a root cause.

Local verification on 2026-09-26:

- The CI-scoped Rust formatting commands passed.
- `cargo check -p nagi-kernel --lib --tests --target
  x86_64-unknown-linux-gnu --locked` passed using the pinned nightly toolchain;
  it reported the pre-existing unused imports in test-only syscall code.
- The release `x86_64-unknown-nagi` kernel build passed.
- `cargo check -p nagi-posix --tests --target
  x86_64-unknown-linux-gnu --locked` passed as a compile check; the x86 test
  binary was not executed on this Apple-Silicon host.
- The `x86_64-unknown-nagi-user` POSIX target check passed, with the existing
  visibility and dead-code warnings.
- Servo diagnostic patches 0011 and 0012 passed `git apply --check` against the
  current generated checkout with patches 0001–0010 applied.
- All 74 `nagi-cli` library tests passed, including source-contract checks for
  Servo patches 0011 and 0012.
- `git diff --check` passed.

The complete `nagi-init --features m17-servo` link and QEMU acceptance still
require public target CI. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

The POSIX test suite was not executed on this Apple-Silicon host: its
`libnagi` syscall assembly uses x86 registers, so a native AArch64 test build
cannot compile it. The x86 test source check passed, and public Ubuntu CI must
run the executable host tests. The complete `nagi-init --features m17-servo`
link was also unavailable locally because `out/rust-src/library` and
`out/m17-mesa/mesa-build` are absent. Public `nagi-target` CI remains required
to verify integration and the real QEMU first pixel. M17 remains `BLOCKED`;
M18 remains `NOT STARTED`.

### Target evidence from CI Actions run #237 (2026-09-26)

Actions run 36228589557 (#237, head
c54af8a046bd510f63aa5f88e2ecbe66dc737101) passed Ubuntu host checks, the
Windows launcher job, Mesa Softpipe, package, kernel, user-init, and UEFI
builds. The real QEMU acceptance created the Mesa/EGL GL context and entered
Servo construction, then panicked in Profiler::create at
third_party/servo/components/profile/mem.rs:48 because thread creation
returned EAGAIN (WouldBlock). It produced no first-web-pixel checksum or
PASS marker. The log does not distinguish child-stack mmap failure from the
single-child bridge's occupied-slot or kernel validation failures. M17 remains
BLOCKED; M18 remains NOT STARTED.

### Local diagnostic continuation after CI run #237 (2026-09-26)

Added failure-only serial traces at the POSIX pthread adapter, the kernel
thread-create validation branches, and the kernel memory-map syscall. The
traces do not change POSIX return values, thread-slot limit, stack size,
mmap-window size, or region-table capacity. The four-region mmap table remains
a candidate cause, not a confirmed one.

Local verification passed the repository CI format command, all 72
`nagi-cli` library tests, `cargo check -p nagi-kernel --tests` for
`x86_64-unknown-linux-gnu`, the release Nagi kernel target build, POSIX test
source checking for `x86_64-unknown-linux-gnu`, the Nagi target POSIX library
check, CI-equivalent workspace Clippy for `x86_64-unknown-linux-gnu`, and
`git diff --check`. Host-side kernel/POSIX test binaries are not run on this
Apple-Silicon Mac because the user syscall code uses x86 registers. The next
public QEMU run confirmed that the single-child bootstrap bridge cannot host
the threads Servo creates. ADR 0029 supersedes the two-slot limit for M17 with
a bounded cooperative user-thread scheduler; implementation and target
verification are pending. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Target evidence from CI run #204 (2026-09-26)

Run `36216371334` (#204, head
`e674668c04061c9abfaf944f609b5f29bf108310`) passed Mesa Softpipe, package,
kernel, Nagi user-init, and UEFI builds. Its real two-boot QEMU acceptance
reported an EGL thread-info cookie mismatch (`inited=80`), reset the TLS state
with `context=0x0`, completed context/thread/surface binding, completed the
state-tracker and DRI make-current calls, and returned from Surfman
make-current. The last Surfman marker announced the start of GL function
loading.
Rust std panicked at `std/src/sys/random/nagi.rs:7:5` because the Nagi random
ABI returned -1. No `SYS_RANDOM_GET` failure reason was logged by this commit,
so the trace does not distinguish user-buffer validation from VirtIO RNG
failure. The 120-second acceptance ended with status 4; no real Servo frame,
pixel checksum, or M17 PASS marker was produced. The separate Ubuntu host job
failed because Clippy rejected `filter().next()` (`filter_next`); the local
source now uses `.any()`. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Target evidence from CI run #203 (2026-09-26)

Run `36212259814` (#203, head
`c1506888655123d819ec75be66891f0cd5477533`) passed both host jobs and all
target builds through the UEFI loader. The real two-boot QEMU acceptance timed
out after 120 seconds, status 4, during EGL thread context binding. The
bounded full-log trace excerpt contains exactly one public `eglMakeCurrent`
path and no `thread-info zero initialization started` or
`thread-info initialized` marker. At that first observed bind,
`_EGLThreadInfo::CurrentContext` was `0x400002b92640`, while reading its
`Binding` yielded `0x8d48080844110f00`; the final marker was
`thread previous-context clear started`. This is consistent with EGL seeing
preexisting invalid TLS state, but does not identify who wrote it or prove that
the attempted clear caused the timeout. The acceptance still produced no
Servo frame, pixel checksum, or RNG runtime evidence. M17 remains `BLOCKED`;
M18 remains `NOT STARTED`.

### Local continuation after CI run #203 (2026-09-26)

Mesa patch `0030` adds a Nagi-only initialization cookie to `_EGLThreadInfo`.
`_eglGetCurrentThread` now clears and initializes the state if either `inited`
is false or the cookie does not match, records a mismatch before the reset,
then writes the cookie before marking the state initialized. Other targets
retain Mesa's original `!inited` condition. This is a targeted recovery
experiment for #203's unexplained preexisting state, not a proven root-cause
fix. Public target CI must show whether the cookie mismatches and whether the
real EGL/Servo path advances. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Local continuation after CI run #204 (2026-09-26)

Added kernel serial diagnostics for every `SYS_RANDOM_GET` rejection class and
each `RandomError` returned by the real VirtIO RNG implementation. The syscall
still returns failure to its caller on error and adds no alternate entropy
source. The existing `nagi-cli` source-contract check now covers the new
diagnostics. Fixed CI #204's Clippy warning by replacing `filter().next()` with
`.any()`. Verification passed: `cargo fmt --all -- --check`, all 71
`nagi-cli` library tests, the full x86_64-target host-workspace Clippy command,
the Nagi kernel release target build, and `git diff --check`. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

Source audit then found the concrete RNG discovery defect: QEMU is configured
with `virtio-rng-pci,disable-modern=on`, but the kernel treated transitional
PCI ID `0x1003` (VirtIO console) as RNG. The VirtIO entropy device uses
transitional ID `0x1005`; the scanner now accepts `0x1005` and modern ID
`0x1044`, with a regression check rejecting the console ID. All 72 tests from
`cargo test -p nagi-cli --lib` pass. The kernel test sources compile under
`cargo check -p nagi-kernel --tests --target x86_64-unknown-linux-gnu --locked`.
The Nagi kernel release target build and x86_64 host-workspace Clippy also
pass. The kernel test binary could not be linked on this Mac because the
installed linker cannot link an x86_64 Linux test harness; the actual Nagi
target build succeeded. CI #205 predates the ID correction and was canceled
before QEMU acceptance. Actions run #232 now builds the corrected revision and
is the first public QEMU verification of the fixed guest RNG path. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### Target evidence from CI Actions run #232 (2026-09-26)

Actions run `36220293827` uses head
`35efaf661fa3bd4c2e7fb207b9e03e11d4cc4b38`. Ubuntu host and Windows launcher
jobs passed. The target job passed Servo/Mesa bootstrap, the M17 dependency
boundary, Mesa Softpipe archive, package, kernel, user-init, and UEFI builds.
The QEMU acceptance passed its persistent-storage check, created the Mesa
Softpipe GL context and swap chain, and entered Servo construction. The guest
then reported `memory allocation of 512 bytes failed`, redirected `abort()`
to `mozalloc_abort`, and did not exit within the 120-second QEMU bound. No
first-web-pixel checksum or PASS marker was produced. The prior random failure
did not recur before this later failure; the fixed RNG path has advanced beyond
the previous stopping point but still needs continued runtime verification.
Source inspection found that all Rust/C++ user allocations share a single
8 MiB POSIX heap, while the bootstrap mmap window is 16 MiB. The local
follow-up under ADR 0027 enlarges this finite guest-owned allocation budget;
public QEMU verification is still required. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Local continuation after CI Actions run #232 (2026-09-26)

ADR 0027 expands the POSIX heap from 8 MiB to 64 MiB and the kernel's bounded
bootstrap mmap window from 16 MiB to 128 MiB. The first-fit mmap search now
checks the four registered regions directly instead of using a per-page stack
bitmap; this keeps the 128 MiB window within the 16 KiB syscall stack. A
failure in `nagi_posix_malloc` now reports whether the heap mapping was
unavailable or the mapped allocator returned no block.

Local checks passed: `cargo fmt --all -- --check`, `git diff --check`,
`cargo check -p nagi-kernel --tests --target x86_64-unknown-linux-gnu --locked`,
the Nagi release kernel build, `cargo check -p nagi-posix --tests --target
x86_64-unknown-linux-gnu --locked`, the Nagi-target `nagi-posix` library check,
and the CI-equivalent workspace Clippy command. The POSIX and kernel unit test
sources compile for x86_64, but their test binaries cannot be run on this
Apple-Silicon host because the kernel and syscall crates use x86-only inline
assembly. The next public target run must verify that the expanded heap carries
Servo through construction to the real frame checksum. M17 remains `BLOCKED`;
M18 remains `NOT STARTED`.

### Target evidence from CI Actions run #233 (2026-09-26)

Actions run `36223836342` (#233, head
`1993a4582952d3c1176ceff4434ac07a11a02881`) passed Ubuntu host, Windows
launcher, Mesa Softpipe, package, kernel, user-init, and UEFI build steps. The
real QEMU acceptance accepted persistent storage, initialized Mesa/EGL, and
created the GL context. Servo construction then reported `memory allocation of
512 bytes failed`, redirected `abort()` to `mozalloc_abort`, and ended without
a first-web-pixel checksum or PASS marker. The serial excerpt contains neither
`POSIX heap mapping unavailable` nor `POSIX allocator returned no block`.

The failure size alone does not include its requested alignment. Source audit
of the pinned Rust Unix allocator shows `System::alloc` uses `posix_memalign`
when a layout requires stronger alignment. Nagi's prior `posix_memalign`
implementation allocated with ordinary malloc and returned `ENOMEM` when its
16-byte-aligned result did not meet that request. This is a source-confirmed
failure path consistent with the log, not runtime proof of the exact
512-byte layout. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Local continuation after CI Actions run #233 (2026-09-26)

ADR 0028 replaces the 16-byte-only `posix_memalign` behavior with a finite,
guest-heap-backed aligned allocation. Over-aligned pointers carry validated
metadata that lets `nagi_posix_free` recover the original allocation and lets
`malloc_usable_size` return the requested payload size; the requested size
remains at the `pointer - 16` ABI location used by `realloc`. C++ aligned
throwing and nothrow `operator new` overloads now pass their requested
`align_val_t` to the same allocator; aligned delete already returns them
through `nagi_posix_free`. The failure case does not use a host allocator or
weaken OOM behavior.

Added allocator tests for POSIX alignment validation, 512-byte and 4096-byte
alignment, freeing/coalescing the underlying blocks, and overflow rejection.
The tests type-check for `x86_64-unknown-linux-gnu`; the package test binary
cannot be linked for the Apple-Silicon host because `libnagi` uses x86 syscall
registers, and the x86_64 macOS test link is rejected by relibc's ELF-style
`.data` section on Mach-O. The focused `nagi-cli` C++ runtime contract test
passes. `nagi-posix` test code type-checks on the Linux target, Clippy passes,
and the Nagi target package check passes with five existing warnings. The
target C++ runtime compiles for `x86_64-unknown-none`, and its object contains
the aligned `new`/`new[]` overloads referencing
`nagi_posix_malloc_aligned`. The repository's CI-format command and
`git diff --check` pass. Public Ubuntu CI remains responsible for executing
the allocator unit tests, and public QEMU acceptance must verify that Servo
reaches the real first-web-pixel checksum. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Target evidence from CI run #202 (2026-09-26)

Run `36208851031` (#202, head
`306d68f6c17f6a700c5f0112cf5a263c58ca9130`) passed both host jobs and every
target build step through the UEFI loader, including Mesa Softpipe and the
Nagi user-init link with the Nagi-specific Rust std random backend. The real
two-boot QEMU acceptance timed out after 120 seconds with status 4. Its final
serial traces show Surfman creating its dummy pbuffer and calling
`eglMakeCurrent`; EGL completed make-current validation and reference updates,
then `_eglBindContextToThread` read `CurrentContext=0x400002b78ca0` while the
new context was `0x400020813710`. Reading the old object's `Binding` returned
`0x8d48080844110f00`; the trace reached `thread previous-context clear started`
but had no completion marker. This is evidence of a suspicious old-context
value, not proof of its source or that the write itself caused the timeout.
The CI report included only the last 64 serial lines, so it could not show
earlier EGL binds or the TLS initialization trace. The M17 failure report now
includes a bounded excerpt of M17 trace markers from the serial log before the
tail. The acceptance did not reach the random request or a Servo frame, so it
does not verify the runtime RNG path. No pixel checksum or PASS marker was
produced. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Target evidence from CI run #201 (2026-09-26)

Run `36205146068` (#201, head
`3ade2f26ca24c3825927e8aa6339e7bc2534c7a7`) passed both host jobs and every
target build step through the UEFI loader, including Mesa Softpipe and the Nagi
user-init link. The real two-boot QEMU acceptance reached EGL with
`inited=1` and `CurrentContext=0x400020813710`, then Rust std panicked at
`library/std/src/sys/random/redox.rs` while opening `/scheme/rand`; the guest
errno was `EINVAL` (22), followed by `mozalloc_abort`. The run ended with the
120-second QEMU timeout. This shows EGL progressed past the prior context
binding diagnostic, but produces no real Servo frame, pixel checksum, or PASS
marker. Source inspection confirmed `libnagi::random_fill` already uses
`SYS_RANDOM_GET`; Rust std had not used it. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Local continuation after CI run #201 (2026-09-26)

Changed the Rust std patch so `target_os = "nagi"` selects a dedicated random
backend instead of Redox's `/scheme/rand` implementation. Its stable C ABI
`__nagi_std_random_fill` in `libnagi` calls the existing bounded
`random_fill`/`SYS_RANDOM_GET` path; failures remain errors and there is no host,
RDRAND, or fixed-byte fallback. Added Mesa patch `0029` to remove the repeated
current-context TLS trace while retaining one-time initialization and owner
checkpoints.

The modified Rust std patch applies to the installed pinned-nightly `rust-src`.
All 29 Mesa patches apply in numeric order from the pinned Mesa source. `cargo
fmt --all -- --check`, `git diff --check`, `cargo clippy -p nagi-cli --lib -- -D
warnings`, and all 68 `nagi-cli` library tests passed. `./nagi std` built the
Nagi Rust std user-init, kernel, UEFI loader, and image with the new backend,
then its local QEMU acceptance timed out after 45 seconds. Its serial log
contains only UEFI screen-control bytes and no guest acceptance markers, so
this does not verify runtime entropy. The existing local persistent user-data
image was reused. `cargo test -p libnagi --lib` is not runnable on this Apple
Silicon host: its x86-64 syscall `asm!` registers are invalid for the host
architecture. Public target CI is required to verify the linked Servo user-init
and runtime path. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Target evidence from CI run #200 (2026-09-26)

Run `36197334175` (#200, head
`8ba8443761873bff33ac6550f697df69479dd23a`) passed both host jobs and every
target build step through UEFI loader, including Mesa Softpipe and the user-init
link. The real two-boot QEMU acceptance completed EGL context creation,
dummy-pbuffer creation, TLS lookups, make-current validation, and resource
reference increments. Inside `_eglBindContextToThread`, its thread-info
`CurrentContext` read returned `0x400002b92640` while the new context was
`0x400020813710`; the log stopped before the old context's `Binding = NULL`
store returned. QEMU timed out after 120 seconds. Source inspection found no earlier
`eglMakeCurrent` marker and no other Mesa writer of `CurrentContext`, but the
CI trace alone does not prove whether the value came from valid earlier EGL
state or incorrect TLS contents. There is no Servo frame, pixel checksum, or
PASS marker. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Local continuation after CI run #200 (2026-09-26)

Added Mesa patch `0028` to report the EGL TLS initialization flag and current
context before binding, then separate the old context owner read from its
clear. Patch 0028 applied cleanly on top of patches 0001–0027 in a clean
worktree based on pinned Mesa revision
`f1f246cfda65eff82fba3be1caf2d23bdeda60cc`. Focused and workspace checks are
complete: the 0028 patch passed `git apply --check --unidiff-zero` after
patches 0001–0027, `git diff --check`, `cargo fmt --all -- --check`, the focused
TLS trace regression test, all 66 `nagi-cli` library tests, and
`cargo clippy -p nagi-cli --lib -- -D warnings`. Public target CI is next; M17
remains `BLOCKED`; M18 remains `NOT STARTED`.

### Target evidence from CI run #199 (2026-09-26)

Run `36193439089` (#199, head
`79941b151610af9db0588f056bc88789bd81b069`) passed both host jobs and every
target build step through UEFI loader, including the patched Mesa Softpipe
archive and the Nagi user-init link. The real two-boot QEMU acceptance reached
the initial dummy-pbuffer `eglMakeCurrent`. Both EGL thread-info lookups,
surface-mode validation, context/surface ownership and config checks, and
resource reference increments returned. The last marker was
`EGL context thread context binding started`; no marker from inside
`_eglBindContextToThread` appeared before the QEMU timeout. No real Servo
frame, pixel checksum, or PASS marker was produced. M17 remains `BLOCKED`;
M18 remains `NOT STARTED`.

### Local continuation after CI run #199 (2026-09-26)

Added Mesa patch `0027` to split `_eglBindContextToThread` into its thread
current-context read, context-owner pointer write, and TLS current-context
write, recording the context and thread pointer values. Patch 0027 applied
cleanly on top of patches 0001–0026; `git diff --check`,
`cargo fmt --all -- --check`, all 65 `nagi-cli` library tests, and
`cargo clippy -p nagi-cli --lib -- -D warnings` passed. Public target CI is
next; M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Target evidence from CI run #198 (2026-09-26)

Run `36186989786` (#198, head
`fc8e6c9e7e5947afe6b82166065858905b1061cd`) passed both host jobs and every
target build step through UEFI loader, including the Mesa Softpipe archive.
The real two-boot QEMU acceptance passed storage persistence, created the
EGL context and dummy pbuffer, then reached `eglMakeCurrent`. Its trace showed
EGL display locking, handle lookup and API validation returning, followed by
`DRI2 make-current entered` and `DRI2 EGL binding started`. No
`DRI2 EGL binding completed` marker appeared before QEMU timed out after 120
seconds. Thus the remaining stop is inside `_eglBindContext` or a call it
makes; it is not yet localized to thread-info lookup, validation, or reference
updates. No real Servo frame, pixel checksum, or PASS marker was produced.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Local continuation after CI run #198 (2026-09-26)

Added Mesa patch `0026` with Nagi-only checkpoints inside `_eglBindContext`
and `_eglCheckMakeCurrent`, including distinct validation rejection markers.
It also brackets the EGL debug-report global mutex to identify an error-path
wait. Patch 0026 applied cleanly atop the prior Mesa patch series; `git diff
--check`, `cargo fmt --all -- --check`, all 64 `nagi-cli` library tests, and
`cargo clippy -p nagi-cli --lib -- -D warnings` passed. Public target CI is the
next verification; M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Target evidence from CI run #197 (2026-09-26)

Run `36179321453` (#197, head
`cb8ab251ec3e085950cdb51369d12a1a1fb32c5b`) passed both host jobs and every
target build step through the UEFI loader. The real two-boot QEMU acceptance
passed storage persistence and completed Mesa Softpipe, GL state-tracker, DRI
context construction, EGL context creation/linking, and dummy-pbuffer setup.
The final marker was `Surfman make-current started`; `eglMakeCurrent` did not
return before QEMU's 120-second timeout. The log does not establish whether
the call reached Mesa's public EGL entrypoint. No real Servo frame, pixel
checksum, or PASS marker was produced. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Local continuation after CI run #197 (2026-09-26)

Added Mesa patch `0025` with Nagi-only checkpoints from the public EGL
`eglMakeCurrent` entry through display locking, DRI2 binding, Gallium DRI,
state-tracker framebuffer validation, and first pbuffer backing allocation.
The patch is diagnostic-only. A clean worktree at pinned Mesa revision
`f1f246cfda65eff82fba3be1caf2d23bdeda60cc` accepted patches `0001`–`0025` in
numeric order with each prechecked, and `git diff --check` passed. All 63
`nagi-cli` library tests, clippy with `-D warnings`, and
`cargo fmt --all -- --check` passed. Target compilation and QEMU verification
of patch `0025` remain pending public CI. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Target evidence from CI run #196 (2026-09-26)

Run `36174195146` (#196, head
`712b343a817bb4aa177f16dbdbbeb27ceee950c8`) passed both host jobs and every
target build step through the UEFI loader. In the real two-boot QEMU
acceptance, all Softpipe context stages completed; Mesa GL state, DRI context
construction, `eglCreateContext`, and `_eglLinkContext` also returned. The
final guest marker was `Nagi M17 trace: EGL context linking completed`.
QEMU timed out after 120 seconds before Surfman's `GL context created` marker,
so the stall is after EGL context creation but before `device.create_context`
returns. There is still no first-web-pixel checksum or PASS marker. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### Local continuation after CI run #196 (2026-09-26)

Added Surfman patch `0002` with Nagi-only trace checkpoints around the EGL
context wrapper, dummy-pbuffer config query/creation, make-current, and GL
function loading. The generated `third_party/surfman` checkout remains
untouched. A clean worktree at pinned Surfman revision
`205778f497327c573929c7b471194390e15f331d` accepted patches `0001`–`0002` in
numeric order with each prechecked, and `git diff --check` passed. All 62
`nagi-cli` library tests, clippy with `-D warnings`, and
`cargo fmt --all -- --check` passed. Target compilation and QEMU verification
remain pending public CI. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Target evidence from CI run #195 (2026-09-26)

Run `36165541043` (#195, head
`2ec9f747d0843ecb32b64daf62fd6a1e609f2ace`) passed both host jobs and every
target build step through the UEFI loader. The real two-boot QEMU acceptance
passed storage persistence, EGL software-driver initialization, and DRI screen
creation. Its trace then showed `Softpipe context creation completed` after
all internal Softpipe stages, but never showed Surfman's `GL context created`.
QEMU timed out after 120 seconds, status 4; there is no first-web-pixel
checksum or PASS marker. The run rules out a stall inside the instrumented
`softpipe_create_context` body. It does not identify which state-tracker, DRI,
or EGL operation after that callback fails to return. M17 remains `BLOCKED`;
M18 remains `NOT STARTED`.

### Local continuation after CI run #195 (2026-09-26)

Added Mesa patch `0024` with Nagi-only checkpoints from the return of
`softpipe_create_context` through Mesa GL-state initialization, state-tracker
context construction, DRI post-processing/thread setup, and EGL context
creation/linking. Mesa patches remain tracked as numbered patches; the
generated `third_party/mesa` checkout was not edited. A clean worktree at the
pinned Mesa revision accepted patches `0001`–`0024` in numeric order with each
patch prechecked, and `git diff --check` passed. All 61 `nagi-cli` library
tests, clippy with `-D warnings`, and `cargo fmt --all -- --check` passed.
Target Mesa compilation and QEMU verification remain pending public CI. M17
remains `BLOCKED`; M18 remains `NOT STARTED`.

### Target evidence from CI run #193 (2026-09-26)

Run `36157913731` (#193, head
`50bcc06b946acb55d0d293a8eec4b46f9a435edc`) passed both host jobs and every
target step through UEFI loader build. The real two-boot QEMU acceptance passed
the persistence checks and reached EGL's static Softpipe driver. EGL completed
driver initialization and DRI screen creation. The next `eglCreateContext`
call did not return within 120 seconds; its serial log ends at
`Nagi M17 trace: GL context creation started`. There is no Servo frame,
checksum, or PASS marker. The target Mesa compile warning at
`sp_context.c:190` reports unused variable `sh`, consistent with the
`__NAGI__` guard skipping the eager cache loop from patch `0022`. The exact
Softpipe context-creation call that stalls is unknown. M17 remains `BLOCKED`;
M18 remains `NOT STARTED`.

### Local continuation after CI run #193 (2026-09-26)

Added Mesa patch `0023` with Nagi-only `_debug_printf` checkpoints around
Softpipe context construction, including TGSI setup, draw-context creation,
vertex-buffer stages, and blitter shader caching. This is diagnostic-only and
does not change rendering behavior. The clean Mesa worktree at pinned revision
`f1f246cfda65eff82fba3be1caf2d23bdeda60cc` accepted patches `0001`–`0023` in
order, and `git diff --check` passed there. The next public target CI remains
necessary to identify the call that stalls; M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Local continuation after CI run #192 (2026-09-26)

CI #192's `ContextCreationFailed(BadAlloc)` is returned after `eglCreateContext`
yields `EGL_NO_CONTEXT`, before pbuffer creation, `eglMakeCurrent`, or surface
setup. Source inspection confirmed that the eager Softpipe texture-cache
matrix needs over 192 MiB, while the Nagi POSIX heap is 8 MiB. Replaced the
initial alignment hypothesis with tracked Mesa patch `0022`, which lazily
allocates caches for bound views and releases them on unbind. It aborts with a
diagnostic if a required cache still cannot be allocated. A clean worktree at
the pinned Mesa revision accepted all 22 numbered patches in order. The 59
`nagi-cli` library tests, clippy, and formatting check pass. The local Mesa
build reached Meson but could not pass its ELF linker probe: Homebrew Clang
selected Mach-O `ld64.lld`, which rejected the ELF-only link arguments, before
Mesa C sources compiled. Target compilation and the runtime effect remain to
be verified by public CI; M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Target evidence from CI run #191 (2026-09-25)

Run `36139834714` (#191, head
`8e12ce4e98616c6146e57707114b04929421889b`) passed host checks, target
dependency validation, Mesa Softpipe, package, kernel, Nagi user-init link,
and UEFI loader build. QEMU passed the two-boot storage gate and reached
device.create_context. The call returned an error, after which the trace
prefix and line ending appeared without the formatted Surfman message. The
existing console syscall validates the user range before checking whether
pages are actually mapped; that range policy allowed image, stack, and TLS but
excluded the mmap region used by the POSIX heap. No GL context or pixel was
produced. M17 remains BLOCKED; M18 remains NOT STARTED.

### Local continuation after CI run #191 (2026-09-25)

The console syscall now permits bounded mmap-region addresses through its
preliminary range check, while the existing mapped-page validation still
rejects unmapped addresses before the kernel copies any bytes. Added coverage
for valid and cross-boundary mmap ranges. The Albert trace callback now checks
each console-write result and prints a static failure marker if a write is
rejected. Formatting passed, the kernel library suite passed (93 tests on the
x86_64-apple-darwin host target), nagi-cli passed all 58 library tests, and the
Nagi-target release kernel build passed. Public target QEMU verification
remains pending; M17 remains BLOCKED; M18 remains NOT STARTED.

### Target evidence from CI run #190 (2026-09-25)

Run `36134006498` (#190, head
`128dd007e039394ee80737e004b5070e7baefedb`) passed Ubuntu host checks,
Windows launcher checks, target Servo bootstrap/feature boundary, Mesa
Softpipe, M16 package, kernel, Nagi user-init linking and UEFI loader build.
The two-boot M17 acceptance timed out after 120 seconds. The guest entered
`SoftwareRenderingContext::new`, initialized EGL's statically linked Softpipe
path, created the Surfman device and GL context descriptor, then called
`device.create_context`. That call returned an error. Servo's existing failure
diagnostic tried `println!` to stdout, which returned Nagi `EIO` and triggered a
Rust panic before the Surfman error could be printed. The callback and larger
stack therefore advanced the path substantially, but the GL context error
remains unknown and no pixel was rendered. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Target evidence from CI run #189 (2026-09-25)

Run `36126812876` (#189, head
`2f62290d64833d0926e0cbfc153f802e154d2823`) passed Ubuntu host checks,
Windows launcher checks, target Servo bootstrap/feature boundary, Mesa
Softpipe, M16 package, kernel, Nagi user-init linking and UEFI loader build.
The M17 QEMU acceptance timed out after 120 seconds, status 4. Its final guest
markers were `GL context creation started` and
`Albert console callback self-test`; the patched
`SoftwareRenderingContext::new entered` marker did not appear. This verifies
the Nagi console callback itself and narrows the stop to the constructor call
or its entry sequence. At this run the bootstrap user stack was 8 pages (32
KiB). Decision 0026 records the bounded 2 MiB stack experiment; stack
exhaustion was not proven by this run. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Local continuation after CI run #190 (2026-09-25)

Implemented the 2 MiB fixed bootstrap stack by mapping one full 512-entry stack
page table, keeping the range below TLS and leaving TLS/mmap virtual addresses
unchanged. Added a regression check for the stack size and boundary. Corrected
the existing bounded-mapping test to inspect its locally constructed TLS page
table instead of the unrelated global bootstrap storage. The focused test and
the complete kernel library suite pass on the Mac host (92/92); formatting,
diff checks, and the Nagi-target release kernel build pass. CI #190 confirms
the larger stack reaches Surfman context creation. Patch `0010` now routes its
failure diagnostic through the callback. It applies to the local generated
Servo checkout, and all 58 `nagi-cli` library tests pass with the new patch
contract. The next public run must expose the Surfman error; M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #188 (2026-09-25)

Run `36120900543` (#188, head
`c41d313d98b3d9dfa3c7f8421453e8f2890149dc`) passed Ubuntu host bootstrap,
format, clippy, build/tests and M0 acceptance; Windows Servo bootstrap,
build/tests and launcher acceptance; and target Servo bootstrap, feature
boundary, Mesa Softpipe, M16 package, kernel, Nagi user-init link and UEFI
loader. The real QEMU acceptance again timed out after
`Nagi M17 trace: GL context creation started`. Neither the Servo stage trace nor
the callback's `libnagi::console_write` output appeared. The next diagnostic
prints a callback self-test before the call and moves a constructor-entry
checkpoint ahead of the size guard. Local checks pass: Servo patch application,
format, `git diff --check`, and all 58 `nagi-cli` tests. M17 remains `BLOCKED`;
M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #187 (2026-09-25)

Run `36115897284` (#187, head
`5b2d23541cab881facb5e9ada9503fd28608644b`) passed pinned Servo bootstrap,
Ubuntu format/clippy/build/tests and M0 acceptance, Windows bootstrap/build/
tests/launcher acceptance, target dependency validation, Mesa Softpipe,
dependencies, package, kernel, Nagi user-init link, and UEFI loader. M17's real
two-boot QEMU test again reached `Nagi M17 trace: GL context creation started`
and timed out after 120 seconds. No `Surfman connection started`, Mesa EGL
trace, checksum, or PASS appeared. Because the direct `libc::write` Servo
checkpoints were also absent, their output path was not reliable evidence of
whether Servo entered the patched constructor. The next diagnostic replaces
that route with an explicit Nagi-only callback to `libnagi::console_write`,
then reruns the authoritative target CI. The Servo patch applies cleanly to
the pinned source, `cargo fmt --all -- --check` passes, and all 58 `nagi-cli`
unit tests pass with the updated callback source contract. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #186 (2026-09-25)

Run `36114799741` (#186) passed `nagi-bootstrap fetch` on Ubuntu and target,
confirming ordered patch `0009` keeps the pinned Servo manifest and its lockfile
consistent. Ubuntu formatting passed. The Ubuntu host Clippy step and target
M17 dependency feature-boundary step then failed with the same root workspace
error: `Cargo.lock needs to be updated but --locked was passed`. The pinned
Servo checkout's own lockfile patch does not update the Nagi root lockfile, so
the root `servo-paint-api` lock entry now also records `libc`; a CLI contract
check guards that edge. The next target run must pass locked dependency checks
before Mesa and init builds. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #185 (2026-09-25)

Run `36113604201` (#185) exposed a source-patch-set consistency issue before
the renderer could be tested: `0008-nagi-m17-rendering-context-traces.patch`
adds a direct `libc` dependency in `components/shared/paint/Cargo.toml`, while
the pinned Servo `Cargo.lock` still lacked the corresponding
`servo-paint-api -> libc` edge. The Ubuntu, Windows, and target bootstraps
invoked Cargo with `--locked` and failed with “the lock file ... needs to be
updated”. The change adds ordered patch
`0009-nagi-m17-rendering-context-traces-lock.patch` for that lock entry and
extends the CLI source-contract test. The next target CI run must first pass
`nagi fetch`, then use the direct descriptor-2 trace to locate the earliest
Surfman stage reached after the application-level marker. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #184 (2026-09-25)

Public CI run `36106455335` (#184, head
`56707103565192957507b177c35373422908588e`) passed Ubuntu host and Windows
launcher acceptance, Mesa Softpipe archive construction, package/kernel builds,
the real target Servo user-init link with no undefined symbols, and UEFI loader
build. The final two-boot M17 QEMU run again passed all M7 persistence checks,
reached surface acquisition, and printed
`Nagi M17 trace: GL context creation started`. QEMU then timed out after 120
seconds. There is no first-web-pixel checksum or PASS marker, so M17 remains
`BLOCKED` and M18 remains `NOT STARTED`.

The Nagi-only EGL policy forced `ForceSoftware` and cleared Zink, but the guest
serial log contained none of the new EGL or Servo/Surfman checkpoints. The
existing checkpoints use Rust `eprintln!` and Mesa's `_eglLog` stderr path;
their absence does not prove which context-creation call stalled. The next
patch changes Servo's diagnostic helper to write directly to the Nagi
descriptor-2 boundary with `libc::write`, avoiding stdio formatting and
locking. The following target CI run will use those direct checkpoints to
locate the first call reached after the application-level context marker.

### Current M17 continuation after CI run #183 (2026-09-25)

Public CI run `36099071216` (#183, head
`4995909db69ea2fa8234662a8d45977b28ecb4fc`) passed Ubuntu host and Windows
launcher acceptance, Mesa Softpipe archive construction, package/kernel builds,
the real target Servo user-init link with no undefined symbols, and UEFI loader
build. The final two-boot M17 QEMU run confirmed that the first boot's
persistent write and the second boot's mount, lookup, read, mmap, and persistent
read all pass. On the second boot, the actual target INIT reached surface
acquisition and entered `SoftwareRenderingContext::new`, then the process
stopped during GL context setup until the 120-second QEMU timeout. This run
does not prove whether EGL device refresh, driver selection, GL context
creation, surface setup, or a later Surfman step is responsible.

The next patch makes Nagi EGL use its software-only renderer policy, clears
the unsupported Zink override, and leaves other platforms' environment-driven
behavior intact. Target-only EGL warning logs bracket device discovery, driver
initialization, surfaceless software and no-DRM probes, and DRI screen creation.
A Servo source patch adds stderr checkpoints around
Surfman connection, GL context and function loading, surface binding, make-
current, and swap-chain setup. `nagi fetch` applies these changes as tracked
patches; the current generated Servo checkout remains untouched. A local M17 run
could not be repeated because the generated checkout fingerprint is already
mismatched. The next public target CI is required to locate the stall and verify
the two-boot regression. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #182 (2026-09-25)

Public CI run `36092134517` (#182, head
`846cb5dc80adcbad01eb5dbd94d127419814639d`) passed Windows launcher, Mesa
Softpipe, package/kernel builds, the real Servo user-init link, and UEFI loader
build. Ubuntu host stopped at the separate loader formatting check. The M17
acceptance command advanced to its final QEMU boot and failed while reading
`INIT.ELF`: `VOLUME_CORRUPTED` at file offset `0xa00000`, requesting 1 MiB from
a 0x79d6fe8-byte file. The EFI diagnostic had already confirmed that opening,
sizing, allocating, and rewinding the file succeeded.

The failure was caused by the two-boot storage gate using the writable block
capability on the largest VirtIO device. The M17 EFI image has 261,415 sectors
(about 128 MiB), while the persistent user-data disk has 32,768 sectors (16
MiB). On the first boot, `Vfs::mount_or_format` writes its ext2 superblock to
LBA 2–3. The FAT12 ESP's first FAT begins at LBA 1, so this write overwrites
FAT12 entries beginning at cluster 341. With INIT beginning at cluster 12,
cluster 341 is reached near file offset `0xa48000`, inside the failing 1 MiB
read. This explains why UEFI can load the bootloader and read the first part
of INIT before the second boot fails.

The repair uses a read-only QEMU attachment for the M17 boot ESP and makes
kernel VirtIO discovery ignore devices offering `VIRTIO_BLK_F_RO` when choosing
the writable user-storage capability. A host regression covers the case where
the read-only boot disk is larger than the writable data disk, and the M17
image-drive argument test checks that only the M17 boot image is opened
read-only. Local QEMU with the full-sized synthetic INIT reaches
`Nagi Kernel started`; public target CI must verify the two-boot persistence
gate and real Servo pixel acceptance. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Current M17 continuation after CI run #181 (2026-09-25)

CI run `36088261144` (#181, head
`25b8a6b4a69e1253977f94415d6937bb34b0d4a1`) passed Ubuntu host and Windows
launcher acceptance, Mesa Softpipe construction, package/kernel builds, the
real Servo user-init link, and UEFI loader build. The M17 acceptance command
ran for 18 minutes and then failed in its final QEMU boot. UEFI reported
`Nagi Loader: init read failed`; the path lookup, ELF size query, page
allocation, and rewind had succeeded, but `RegularFile::read` returned an EFI
error. The loader did not preserve the status or read offset, so the exact
firmware failure is unknown. The log contains no `Nagi Kernel started` marker
or Servo pixel checksum. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

The next repair adds the EFI status, file offset, requested read size, and total
file size to that loader diagnostic. A host-only FAT regression checks every
link in a 3,899-cluster chain matching the 127,747,368-byte init ELF measured in
CI #175, without allocating the complete 128 MiB image. Use the resulting
status and offset to choose a targeted UEFI file-read or image-chain repair;
the current evidence does not justify changing the boot image format.

### Current M17 continuation after CI run #180 (2026-09-25)

CI run `36082853692` (#180, head
`18547213daa966fa37ce5ecde8047ef742091991`) passed Ubuntu host, Windows
launcher, Mesa Softpipe, package, kernel, real Servo user-init link, and UEFI
loader steps. The QEMU first boot printed `Nagi Kernel started`, M2–M4 PASS,
SMP workloads PASS, VirtIO Block/Net/Sound PASS, and M9 display/input setup
PASS, then stopped after `Nagi M5 user process START` until the 120-second
timeout. It did not print a user-address-space error, syscall error, or user
process output. This does not establish whether preparation, ring-3 entry, or
early userspace stalled.

The diagnosis also found that the M17 `_start` branch returned directly to the
Servo pixel path, while `execute_m17` first waits for
`Nagi M7 persistent write PASS` and then boots again to verify persistent
storage. The M17 branch now runs that real M7 write/read check first, exits
after the initial write, and continues to Servo on the verifying boot. New
serial progress reports will identify the stalled kernel preparation phase,
PT_LOAD page counts and BSS sizes, and Servo initialization phase. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI runs #158–#167 (2026-09-24)

Public CI run `35975608809` (#158, head `4ab666897712ff35120fc819cff845f45f5598c6`)
passed target dependency validation, Mesa Softpipe archive construction,
package, and kernel build, then was canceled during `Build Nagi user init`
while Cargo was compiling pinned Servo dependencies. It did not reach target
linking, so it provides no result for the 46-symbol repair. Both host jobs
failed their M0 image acceptance: Ubuntu logs identify the cause as the M17
libc++ ABI/sort shims invoking `nagi-target-cc.sh` when the M0 build has not
generated relibc's pthread headers. The shims only serve Servo/MozJS, so
`user/nagi-init/build.rs` now compiles them only when `m17-servo` is enabled.

Public CI run `35976743137` (#160, head `d68f698a4265afabcf07edb60eb00575bb916112`)
passed `ubuntu-host` and `windows-launcher`, including their M0 launcher
acceptance, and passed target dependency validation, Mesa, package, and kernel
builds. The real `Build Nagi user init` target link found these eight
unresolved symbols:

- libc++: `std::__1::basic_string<char, std::__1::char_traits<char>, std::__1::allocator<char>>::__grow_by(unsigned long, unsigned long, unsigned long, unsigned long, unsigned long, unsigned long)`.
- MozJS: `JS::RestoreMicroTaskQueue`, `JS::InitAsyncTaskCallbacks`,
  `JS::Dispatchable::Run`, and `JS::NewArrayBufferWithContents`.
- Mesa: `glcpp_preprocess`, `spirv_to_nir`, and
  `spirv_verify_gl_specialization_constants`.

The target link line carried the MozJS build directories but did not name
`js_static`, `jsapi`, or `jsglue`; link arguments emitted by that
dependency's build script did not reach the final binary. The current working
tree fixes this at the M17 binary link with a selective static archive group.
The Mesa build now explicitly materializes its `build_by_default=false`
`libglcpp.a` and `libvtn.a` providers, and the target-owned libc++ ABI object
provides the real `__grow_by` implementation.

Public CI run `35984563470` (#161, head `81209f434ad5d60e294139f301740ed51d16a538`)
passed Ubuntu and Windows host acceptance, target dependency validation, Mesa,
package, and kernel builds. The final target link resolved the three Mesa
shader functions and libc++ `__grow_by`, leaving four MozJS entries—
`JS::NewArrayBufferWithContents`, `JS::RestoreMicroTaskQueue`,
`JS::InitAsyncTaskCallbacks`, and `JS::Dispatchable::Run`—plus `strpbrk`.
The UEFI and first-web-pixel jobs were skipped. The compile log showed host
`cc1plus` activity while the workflow set only the target `CC` variable, so
the current repair explicitly routes both target `CC` and `CXX` through the
Nagi wrapper. The CLI now discovers the matching libc++ header root from the
configured Clang C++ include search when `NAGI_CXX_HEADERS` is unset; this
keeps the same C++ build path usable outside CI. `strpbrk` already has a real
relibc implementation, and the final link now seeds that exact provider for
archive extraction. These changes are pending authoritative CI verification.

Public CI run `35989665498` (#162, head `053e5b72ea3df13e100f5206a37a0beb83ba9e72`)
passed Ubuntu and Windows host acceptance, target dependency validation, Mesa,
package, and kernel stages, then failed while compiling
`harfbuzz-sys@0.8.0`. The logged command used `tools/nagi-target-cc.sh` for
`harfbuzz/src/harfbuzz.cc`; Clang stopped because libc++ did not know that
Nagi provides the pthread thread API or the default rune table. These settings
already appear on the M17 libc++ ABI shim, so the wrapper now detects C++
translation units and supplies both definitions. The UEFI loader and
first-web-pixel steps were not reached; the #161 final-link inventory remains
unverified by this run.

Public CI run `35991563209` (#163, head `43df00cc1f60711724925bd7e9931fa70002b252`)
passed Ubuntu and Windows host acceptance, target dependency validation, Mesa,
package, and kernel stages. It compiled HarfBuzz and completed the real target
link with **zero undefined symbols**, then failed on one duplicate
`JS::NewArrayBufferWithContents` definition. rust-lld identifies the genuine
upstream provider at `ArrayBufferObject.cpp:3749` and the duplicate Nagi
wrapper in `jsglue.cpp:1292`, both inside the MozJS Rust archive. The
Nagi-owned patch 0014 added that wrapper when the target C++ provider was not
being compiled with the correct ABI; that condition is now fixed, so the
duplicate patch is removed and the upstream ownership-transfer implementation
is retained. UEFI and first-web-pixel acceptance were not reached.

Public CI run `35995521107` (#164, head `7437d9a2a33aa142ac298ed38caf0e7e85350c23`)
passed Ubuntu and Windows host acceptance, target dependency validation, Mesa,
package, and kernel stages. The target link has no duplicate ArrayBuffer
definition, but rust-lld reported 24 undefined symbols: `ntohs`, `ntohl`,
`htons`, `htonl`, `strpbrk`, libc++ `basic_string` assign/resize/append/replace
entrypoints, and C++ exception/RTTI entrypoints referenced by fontsan's OTS
objects and one MozJS object. The full diagnostic scanned 1,637 target
archives and objects and found no exact provider definitions. The pinned
`fontsan` OTS build script uses cc-rs without exception or RTTI flags; its OTS
sources contain no `throw` or `catch` statements. The repair therefore makes
the common Nagi C++ wrapper enforce the target's no-exception/no-RTTI contract,
instantiates the five real libc++ string entrypoints from target headers, adds
the missing network byte-order and `strpbrk` functions to Nagi relibc, and
roots them before the Rust archive scan. UEFI and first-web-pixel acceptance
were not reached.

Local verification at the #159 checkpoint passed: `./nagi fetch`,
`./nagi doctor` (12/12), `cargo test -p nagi-cli --locked` (48 unit tests
and 18 CLI tests), `./tests/acceptance/m0_launcher.sh`, targeted rustfmt
checks, `bash -n tools/mesa/build.sh`, and a host `clang++` syntax/object
check of the libc++ ABI shim; its object defines the expected `__grow_by`
symbol. The build script also compiles standalone. On this Apple-silicon host,
`cargo check -p nagi-init --locked`
cannot validate the x86-64 guest: it fails on x86-64 inline-assembly registers
in `libnagi` under the host AArch64 target. Workspace-wide rustfmt likewise
reports formatting changes across pinned Servo sources with the local
formatter; the edited Rust files pass targeted checks.

Local verification after the #162 repair passed: `./nagi fetch` regenerated
the MozJS checkout without patch 0014; `cargo test -p nagi-cli --locked` (49
unit tests and 18 CLI tests), targeted Rust formatting, shell syntax, and
`git diff --check` passed. Focused
`cargo clippy -p nagi-cli --all-targets --locked -- -D warnings` passed too.
The regenerated checkout contains the
upstream `ArrayBufferObject.cpp` implementation and no duplicate wrapper in
`jsglue.cpp`. Before removing patch 0014, the Nagi C++ wrapper compiled the
real HarfBuzz `harfbuzz.cc` translation unit and `nagi-libcpp-abi.cpp` with
its libc++ thread/rune-table flags for `x86_64-unknown-elf`; `llvm-nm`
confirmed the expected ABI entry point. The next Ubuntu target CI must verify
the duplicate is gone in the final link. UEFI and real QEMU first-web-pixel
evidence remain pending. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

Local verification after the #164 repair passed: focused M17 tests (11), the
full `nagi-cli` suite (49 unit and 18 CLI tests), focused Clippy,
`bash -n tools/nagi-target-cc.sh`, and `git diff --check`. The Nagi wrapper
compiled `nagi-libcpp-abi.cpp` plus the real fontsan OTS `ots.cc` and `cff.cc`
sources with exceptions and RTTI flags passed on the command line; the wrapper
disabled them, and `llvm-nm` confirmed the required libc++ string methods in
the target-owned ABI object. The OTS `cff.cc` object had no unresolved
exception/RTTI symbols. These local compiles used Homebrew libc++ on macOS;
Ubuntu CI remains the authority for the pinned target ABI and final link. A
focused Nagi relibc target build also passed, and `llvm-nm` found `htonl`,
`htons`, `ntohl`, `ntohs`, and `strpbrk` in the resulting target archive. It
emitted three unrelated existing `private_interfaces` warnings for `NagiTm`
time functions. UEFI and real QEMU first-web-pixel evidence remain pending.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

Public CI run `35999917185` (#165, head
`f5b429c95b2f231d272689fdde57d711665db821`) passed Ubuntu and Windows host
acceptance, target dependency validation, Mesa Softpipe archive construction,
package, and kernel build. `Build Nagi user init` stopped while compiling
MozJS ICU, before final target linking. The compile commands contained both
Mozilla's explicit `-frtti` and an earlier `-fno-rtti`; the common wrapper
appended another `-fno-rtti`, so ICU's `dynamic_cast` in `basictz.cpp` and
`serv.cpp`, and `typeid` in `schriter.cpp`, failed to compile.

The wrapper now tracks the last explicit RTTI option. It retains explicit
`-frtti` for target code supported by Nagi's bounded Itanium RTTI runtime,
continues to disable exceptions, and defaults to `-fno-rtti` if a build script
does not select RTTI. A target-Clang smoke check confirmed the explicit-RTTI
translation unit emits `__dynamic_cast` while an unspecified-RTTI translation
unit remains rejected. `cargo test -p nagi-cli --locked` passed (49 unit and
18 CLI tests), and `bash -n` plus `git diff --check` passed. This repair has
not yet run in authoritative Ubuntu CI. The #164 final-link inventory repair
therefore remains unverified; UEFI and real QEMU first-web-pixel acceptance
were not reached. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

Public CI run `36002926592` (#166, head
`4432a0110df1ba6cf86583205e0b77931e1bc227`) passed Ubuntu and Windows host
acceptance, target dependency validation, Mesa Softpipe archive construction,
package, and kernel build. The target compiled MozJS ICU and proceeded through
Servo/MozJS compilation to the real `nagi-init` link, confirming the RTTI
flag-precedence repair. rust-lld then reported exactly one undefined symbol:
`std::__1::basic_string<char, std::__1::char_traits<char>,
std::__1::allocator<char>>::__grow_by_and_replace(unsigned long, unsigned
long, unsigned long, unsigned long, unsigned long, unsigned long, char const*)`.
The reference inventory identifies the three callers in
`nagi-libcpp-abi.cpp` (`__assign_external`, `append`, and `replace`); scanning
1,637 target archives/objects found no provider.

The current repair explicitly instantiates libc++'s real
`basic_string<char>::__grow_by_and_replace` implementation from the target
headers. A target-Clang compile of the shim succeeds and `llvm-nm` confirms the
exact weak symbol is defined. `cargo test -p nagi-cli --locked` passes (49
unit and 18 CLI tests), along with `cargo clippy -p nagi-cli --all-targets
--locked -- -D warnings`, shell syntax, and `git diff --check`. The new
provider still needs authoritative Ubuntu target CI verification. UEFI and
real QEMU first-web-pixel acceptance were not reached. M17 remains `BLOCKED`;
M18 remains `NOT STARTED`.

Public CI run `36006860116` (#167, head
`daf55d081b96ee5e82045acf9c8f38e32cad3f6d`) passed Ubuntu and Windows host
acceptance, target dependency validation, Mesa Softpipe archive construction,
package, and kernel build. The real target user-init link produced zero
undefined symbols but failed because TLS-bearing objects from Mesa and relibc
were present without a `PT_TLS` program header. UEFI and real QEMU first-web-
pixel acceptance were skipped. ADR 0021 records bounded x86-64 static TLS:
one template no larger than 4 KiB is copied into isolated initial-thread and
child-thread slots, each with a data page and FS-base/control page. Dynamic TLS
modules remain unsupported. The linker emits `PT_TLS`; the parser validates
header uniqueness, alignment, bounds, and load coverage; process setup copies
the initial template, restores the child slot before reuse, initializes each
thread pointer at `FS:0`, and saves/restores FS base during context switches.
The isolated ELF parser suite passed (14 tests), `cargo test -p nagi-cli
--locked` passed (49 unit and 18 CLI tests), the x86-64 Nagi kernel release
build passed, and linker-script ELF probes for initialized TLS, high alignment,
and BSS-only TLS all emitted `PT_TLS` and passed the real ELF parser. The full kernel test crate could
not run on macOS arm64 because existing x86 port-I/O assembly uses unavailable
host registers. These changes still need authoritative Ubuntu target CI, UEFI,
and real QEMU first-web-pixel evidence. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Current M17 continuation after CI run #168 (2026-09-24)

Public CI run `36019101924` (#168, head
`ef69ff4066658eabaed33d1871f432f73bc01d59`) passed `ubuntu-host`, target
dependency validation, Mesa Softpipe archive construction, package, kernel,
the real `nagi-init` target link, and the UEFI loader build. The link completed
with zero undefined symbols after the bounded static TLS repair. The M17
first-web-pixel acceptance then failed with exit code 4, about two seconds
after invoking `./nagi m17`. The acceptance script assigned the combined CLI
output under `set -e`, so the failing assignment exited before the script
printed the captured diagnostic. No guest serial log or first-pixel evidence
was reported; M17 is not PASS.

The same run's Windows host test failed in
`mesa::tests::m17_mesa_link_does_not_force_duplicate_archive_members`: an
assertion compared an LF substring in `kernel/src/syscall.rs`, while the
Windows checkout had CRLF. The local source-inspection test now normalizes
CRLF to LF, and the focused test passes on macOS. The acceptance script now
captures and prints `./nagi m17` output even when the command exits nonzero,
then returns that original status; the first-pixel checks are unchanged. The
next CI run exposed a missing `NAGI_CXX_HEADERS` value on the acceptance step;
the corresponding workflow change and evidence are recorded below. The
Windows suite passed on #169. The real QEMU first-pixel test remains pending.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #169 (2026-09-24)

Public CI run `36023620387` (#169, head
`5b920506a0863fff90805446542a32afd350bf38`) passed `ubuntu-host` and
`windows-launcher`, including the CRLF-normalized source test, then passed
target dependency validation, Mesa Softpipe archive construction, package,
kernel, the real `nagi-init` target link, and UEFI loader build. The target
first-web-pixel acceptance returned exit code 4 before QEMU or guest serial
evidence. With the diagnostic-output repair, the exact CLI error is:
`m17: C++ headers: could not find libc++ headers through clang++; set
NAGI_CXX_HEADERS to a libc++ include directory containing cstddef`.

The successful `Build Nagi user init` step explicitly sets
`NAGI_TARGET_CLANG=clang-19` and `NAGI_CXX_HEADERS=/usr/include/c++/v1`, but
GitHub Actions does not carry step-level environment values into the following
acceptance step. The workflow now supplies those same pinned settings to the
acceptance invocation. This only repairs build configuration; the real QEMU
and guest-pixel acceptance criteria remain unchanged. M17 remains `BLOCKED`;
M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #170 (2026-09-25)

Public CI run `36027813442` (#170, head
`e28602a12d87ba37b52852b42527753753f573fd`) passed both host jobs, target
dependency validation, Mesa Softpipe archive construction, package, kernel,
the real `nagi-init` target link, and UEFI loader build. The target link
completed with zero undefined symbols. The real first-web-pixel acceptance
then stopped before QEMU launch: `./nagi m17` revalidated generated
`freetype-sys` source and found that its checkout no longer matched its marker.

The source fingerprint changed because the pinned crate's build script copied
`libpng/scripts/pnglibconf.h.prebuilt` into the generated source tree at
`libpng/pnglibconf.h`. The new Nagi patch `0002` writes that generated header
to Cargo's `OUT_DIR` and adds that directory to libpng's include path, keeping
the pinned checkout immutable. No guest boot or rendered-pixel evidence was
produced by #170. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #171 (2026-09-25)

Public CI run `36032710471` (#171, head
`798e99c369bd28b661c9417e3a854e6e55f2e056`) passed Ubuntu and Windows host
jobs, Servo bootstrap, target dependency validation, Mesa Softpipe archive
construction, package, and kernel build. `Build Nagi user init` failed while
compiling pinned `freetype-sys`: `freetype2/src/sfnt/pngshim.c` includes
`libpng/png.h`, but the FreeType C builder had not added Cargo's `OUT_DIR` to
its include path and could not resolve `pnglibconf.h`.
The initial patch added `OUT_DIR` only to the later libpng C build; the
FreeType C build also compiles `pngshim.c`. The patch now adds that include
directory to both C builders. No target link, UEFI build, QEMU boot, or pixel
evidence was produced by #171. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Current M17 continuation after CI run #172 (2026-09-25)

Public CI run `36034194228` (#172, head
`40c00607288e3e29e10d1e0ac918d83ca375efc7`) passed both host jobs, target
bootstrap through the real `nagi-init` link, and UEFI loader build. During
the first-web-pixel acceptance invocation, `./nagi m17` passed source
validation and reached its Mesa/Softpipe rebuild, which stopped just after
Meson configuration and before reporting the first selected core target. The
failure output contained no lower-level diagnostic. The likely cause is an
early-exit `awk` in a `ninja -t targets all | awk ... exit` pipeline: with
`pipefail`, Ninja can receive SIGPIPE and terminate the script before its
missing-target diagnostic.

`tools/mesa/build.sh` now captures the complete target graph once and scans it
fully for each required archive, avoiding early pipeline termination. The
real guest-pixel acceptance remains unchanged. No QEMU boot or pixel evidence
was produced by #172. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #173 (2026-09-25)

Public CI run `36039057472` (#173, head
`9dcf843f0b9f35dfcf3c282902e5355446e7f69e`) failed both host workspace
test jobs because `m17_mesa_link_does_not_force_duplicate_archive_members`
still expected the previous escaped-regex strings from `tools/mesa/build.sh`.
The assertion now checks that the full Ninja graph is captured, scans to
completion, selects exact archive suffixes, and does not use the old
early-exit pipeline. The focused test passes locally. In the target job, the
top-level Mesa Softpipe archive build passed and the real user-init link had
started. GitHub canceled that job when #174 replaced the run, before link or
UEFI results were produced. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #174 (2026-09-25)

Public CI run `36041019363` (#174, head
`6fefe6ccc50ed6f689c82115e5d1aac179264033`) passed both host jobs, the pinned
Mesa Softpipe archive build, the real `nagi-init` target link, and the UEFI
loader build. The first-web-pixel acceptance step then failed before starting
QEMU: `./nagi m17` returned exit 4 because the linked init ELF exceeded the
legacy 1.44 MiB FAT12 image's per-file capacity. No guest checksum, surface
present, or QEMU pixel evidence was produced; M17 remains `BLOCKED` and M18
remains `NOT STARTED`.

The host image writer now preserves the legacy 1.44 MiB FAT12 format for
existing milestones and builds a dedicated 8 MiB FAT12 ESP for M17 with 4 KiB
clusters. This provides more than the loader/kernel's existing 4 MiB init
image limit without changing the UEFI file-loading path. The new image test
checks BPB geometry, the nested EFI/NAGI entries, a 2 MiB init file's data, and
its FAT12 cluster chain. All `nagi-cli` tests pass locally (50 unit tests and
18 CLI integration tests). The next authoritative CI run must reach OVMF/QEMU
and confirm the nonzero checksum and successful Nagi Surface present.

### Current M17 continuation after CI run #177 (2026-09-25)

Public CI run `36065949056` was triggered from head
`fdc37631dd1242574be3d8558a52f823c1f3deeb`. Ubuntu host and Windows launcher
jobs passed, including all formatting checks. The target job passed Mesa,
kernel, user-init link, and UEFI loader. QEMU accepted the Linux `none` audio
backend and retained the VirtIO Sound device, then failed to open
`out/artifacts/nagi-0.1-m17-user-data.img`, which the M17 command had not
created. It exited before `Nagi Kernel started`; no ELF guest boot, rendering,
Surface present, or pixel checksum was reached.

The local repair creates the M17 persistent disk using the shared disk helper.
On a fresh disk, it follows the established M14/M16 first-boot path and checks
`NAGI_WRITE_MARKER` before running the dedicated M17 Servo first-pixel boot.
Local verification passes 51 `nagi-cli` unit tests, 18 integration tests,
Clippy with warnings denied, all pinned CI formatting checks, and
`git diff --check`. M17 remains `BLOCKED`; M18 remains `NOT STARTED` pending
public QEMU and guest-pixel evidence.

### Current M17 continuation after CI run #178 (2026-09-25)

Public CI run `36071410328` was triggered from head
`b15cbaa4ef9983529c4fe2065e8f1eeb13d7597f`. Ubuntu host and Windows launcher
passed. The target passed Mesa, kernel, Servo user-init, and UEFI loader builds.
QEMU accepted the Linux `none` audio backend and opened the newly created M17
persistent disk. Its first boot did not exit or emit the expected
`Nagi M7 persistent write PASS` marker within 120 seconds, so the CLI stopped
before the dedicated pixel boot. No guest-pixel acceptance was produced.

The run did not upload `out/logs/m17-first-boot.log`, leaving the exact boot
stage unknown. The local repair appends the last 64 lines of the M17 serial log
when the first or final QEMU boot fails. The target's marker and pixel
acceptance conditions remain unchanged. Local verification passes 52
`nagi-cli` unit tests, 18 integration tests, Clippy with warnings denied, all
pinned CI formatting checks, and `git diff --check`. M17 remains `BLOCKED`;
M18 remains `NOT STARTED` pending real guest-boot and pixel evidence.

### Current M17 continuation after CI run #179 (2026-09-25)

Public CI run `36076724861` was triggered from head
`2821d0156841c9afe7fa11b3755dbfd0f09b9e13`. Both host jobs passed. The target
passed Mesa Softpipe, kernel, the real 127,747,368-byte Servo init link, and
UEFI loader builds, then timed out in its first QEMU boot. The serial tail
showed `Nagi Loader: segment allocation failed` before `Nagi Kernel started`.

Local QEMU/OVMF reproduced the failure. Diagnostic output identified kernel
PT_LOAD segment 2 at `0x219000`, size `0x120d820` (4,622 pages), with UEFI
status `NOT_FOUND`. Its requested range overlapped conventional memory only
through `0x800000`, ACPI non-volatile descriptors around 8–9 MiB, and
Boot-Services data through `0x1780000`. The current writable PT_LOAD includes
static mmap backing storage and M17's added image page tables, so the old 2 MiB
link base made it cross those firmware reservations. ADR 0023 records moving
the fixed kernel base to 64 MiB while keeping exact UEFI allocation, identity
mapping, and the pixel acceptance unchanged.

After the address change, the local QEMU run printed `Nagi Kernel started` and
passed M2, M3, and M4 acceptance. The default 41 KiB non-Servo init then
stopped at M5 ELF validation because its PT_TLS program header has zero file
and memory sizes; this does not exercise the real M17 init with its static
TLS. The kernel release build, UEFI loader release build, loader library tests
(4), loader formatting check, and `git diff --check` pass. The loader binary
test cannot run as a host test on macOS because `uefi` is target-only; a host
kernel test also cannot compile x86 inline-assembly registers on this Apple
Silicon host. The next authoritative step is the public target run using M17's
real Servo image. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #176 (2026-09-25)

Public CI run `36060044054` was triggered from head
`a2400699ce736f843ca122e1da533211c585ca02`. The Windows launcher job passed.
The Ubuntu host job failed at its `Format` step before later host checks ran:
the root workspace formatting passed, but `loader/` is a separate Cargo
workspace and its pinned rustfmt check found a line-wrapping difference in
`loader/src/main.rs`. A local commit fixes that exact formatting issue; the
same pinned rustfmt command now passes for root, package-tool, and loader
workspaces.

`nagi-target` passed target setup, Servo bootstrap, feature-boundary
validation, Mesa Softpipe archive, package and UEFI dependency fetch, M16
package build, kernel build, the real Servo user-init link, and the UEFI loader
build. The M17 acceptance command then failed because QEMU rejected
`-audiodev driver=dsound` on Ubuntu 24.04. It exited before the guest printed
`Nagi Kernel started`; ELF loading, guest rendering, Surface present, and
pixel checksum were not reached. The root cause was that the shared QEMU
command line hardcoded a Windows-only audio backend.

The local repair selects QEMU's host-native audio backend: DirectSound on
Windows, Core Audio on macOS, and the portable dummy backend on Linux/other
hosts. The VirtIO Sound PCI device remains enabled for the guest. In addition,
the separate loader workspace now passes the pinned formatting check. Local
verification passes 51 `nagi-cli` unit tests, 18 integration tests, Clippy with
warnings denied, and every CI formatting command. M17 remains `BLOCKED`; M18
remains `NOT STARTED` pending a public run that boots the guest and proves the
real Servo pixel checksum and Surface present.

### Current M17 continuation after CI run #175 (2026-09-25)

Public CI run `36049471002` (#175, head
`01f6d3f42768ba2ba8d9fa6734474a54026c3055`) passed the Ubuntu host gate,
pinned Servo/dependency setup, Mesa Softpipe archive, actual Nagi user-init
link, and UEFI loader build. The first-web-pixel command stopped before QEMU:
the linked init ELF was 127,747,368 bytes while the 8 MiB FAT12 image could
store only 8,372,224 bytes per file. No UEFI read, kernel ELF mapping, QEMU,
surface-present, or guest-pixel evidence was produced. The Windows launcher job
stopped while fetching pinned Mesa because GitLab reset the connection.

ADR 0022 records the M17 capacity update. Its implementation uses a separate
maximum-capacity FAT12 ESP with 32 KiB clusters; UEFI reads the init file
directly into `LOADER_DATA` pages below 4 GiB in bounded 1 MiB reads. The kernel
checks the entire allocation is identity-mapped, maps fully file-backed ELF
pages from that allocation, and zeroes allocator-backed partial/BSS pages. The
bounded image region is 512 MiB across 256 page tables; stack, static TLS,
Surface, and mmap reservations follow it. W^X and segment permissions remain
enforced, including rejecting overlapping file-backed pages with different
write/execute flags.
Local kernel, loader, and host CLI release builds pass. M17 remains `BLOCKED`
pending the next public run and real QEMU first-web-pixel evidence; M18 remains
`NOT STARTED`.

### Current M17 continuation after CI run #157 (2026-09-24)

Public CI run `35959281238` (#157, head
`8b5c6e451c9afc5142a91264cb0e9b6b527e0f09`) passed Servo bootstrap, target
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. `Build Nagi user init` failed at the real target link. The
diagnostic now reports all 46 distinct undefined symbols, grouped as follows:

- libc/POSIX: `remove`, `madvise`, `getrusage`, `fsync`, `utimes`,
  `ftruncate`, `fchmod`, `fchown`, `__fpclassifyf`, `getc`, `ferror`,
  `clearerr`, `stdin`, `fileno`, `strtok`, `strtok_r`, `llabs`,
  `__program_invocation_short_name`, `log10`, `sigfillset`, `sigdelset`,
  `pthread_sigmask`, `pthread_getcpuclockid`, `pthread_barrier_destroy`,
  `pthread_barrier_wait`, and `fdopen` (26).
- dynamic-loader boundary: `dlopen`, `dlerror`, and `dlclose` (3).
- Mesa: `glcpp_preprocess`, `spirv_to_nir`, and
  `spirv_verify_gl_specialization_constants` (3).
- libc++: `this_thread::sleep_for`, `basic_string::append(size_t, char)`, and
  eight integer `__sort` specializations (10).
- MozJS: `JS::RestoreMicroTaskQueue`, `JS::InitAsyncTaskCallbacks`,
  `JS::Dispatchable::Run`, and `JS::NewArrayBufferWithContents` (4).

The working tree adds target-side providers and link roots for these groups,
including explicit fail-closed dynamic-loader APIs, uses truthful unsupported
behavior for unavailable guest capabilities, adds a pinned-source portability
patch for relibc header generation, and records the VirtIO durable-flush
syscall decision in ADR 0020. Local checks pass: `cargo test -p nagi-cli
--locked` (65 tests), target `cargo check` for the kernel, relibc, and
`nagi-posix`, `cargo clippy -p nagi-cli --all-targets --locked -- -D warnings`,
Rust formatting checks for modified sources, `bash -n` for both changed shell
scripts, and Python diagnostic-script smoke checks. The full host-workspace
Clippy command cannot run on this Apple Silicon host because `libnagi`'s
x86-64-only syscall register assembly does not compile for arm64; Ubuntu CI is
the authoritative host lint. The official `./nagi m17` attempt on this Mac
stops during Mesa Meson configuration: clang 19 sends ELF link probes through
the host `ld64.lld`, which rejects ELF flags and makes the `libatomic` probe
fail. No guest or host runtime fallback was introduced. Run the Ubuntu
`nagi-target` CI after reviewing the grouped changes; it remains the
authoritative target build. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #156 (2026-09-24)

Public CI run `35954492666` (#156, head `fcdd0baf5fa4b37a934736464de4f84c872a6dea`)
passed Servo bootstrap, target dependency validation, Mesa Softpipe archive
construction, package, and kernel compilation. `Build Nagi user init` failed
at the real target link after about twenty-two minutes. rust-lld emitted 20
distinct undefined symbols and then stopped with `too many errors emitted`;
the log explicitly recommends `--error-limit=0`. The visible set includes
`remove`, libc++ `this_thread::sleep_for`, eight libc++ `__sort` instantiations,
`madvise`, `getrusage`, libc++ `basic_string::append(size_t, char)`, four
SpiderMonkey `JS::*` entries, `fsync`, `dlopen`, and `dlerror`. This is a
partial inventory, not a complete list. UEFI and real QEMU first-web-pixel
acceptance were skipped. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.
The same run's non-target jobs also failed: Ubuntu Clippy flagged
`duration.subsec_nanos() / 1_000` in `user/nagi-net/src/smoltcp_stack.rs:660`,
and Windows host tests reported a missing `peer_name` source-contract entry
plus an outdated weak-fallback attribute-order assertion in `tools/nagi-cli`.
Track these for final host-CI cleanup; they do not change the failed target
link result.

### Current M17 continuation after CI run #155 (2026-09-24)

Public CI run `35952148203` (#155, head `c87f349`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. `Build Nagi user init` completed the real target link
after approximately twenty-one minutes and failed with `mktime`, `gmtime_r`,
and `readlink`; UEFI and real QEMU first-web-pixel acceptance were skipped.
The next bounded repair adds UTC-only `mktime`/`gmtime_r` inverse/forward
conversion to the Nagi relibc clock backend and exposes `readlink` as a
real target ABI that returns `ENOSYS` because symlinks are outside the M17
filesystem slice; it never reads a host path or fabricates a target link.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #154 (2026-09-24)

Public CI run `35949658392` (#154, head `e78baac`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. `Build Nagi user init` completed the real target link
after approximately twenty minutes and failed with
`pthread_getattr_np`, `pthread_attr_getstack`, and `nearbyintf`; UEFI and
real QEMU first-web-pixel acceptance were skipped. The next bounded repair
adds `pthread_getattr_np` and `pthread_attr_getstack` to the Nagi-owned POSIX
bridge, reporting the fixed initial guest stack or the actual bounded native
pthread stack, and adds target-owned IEEE `nearbyintf` to relibc with
selective archive seeds. Local Windows `cargo check -p nagi-posix` remains
host-toolchain-limited by missing MSVC `link.exe`; formatting and diff checks
pass. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #153 (2026-09-24)

Public CI run `35947092812` (#153, head `0a390c5`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. `Build Nagi user init` completed the real target link
after roughly nineteen minutes; the #152 `bad_alloc` and `islower` symbols
were resolved. The new link diagnostics exposed
`std::__1::__next_prime(unsigned long)`,
`std::__1::locale::use_facet(std::__1::locale::id&) const`, and `nearbyint`.
The UEFI loader and real QEMU first-web-pixel steps were skipped. The bounded
repair supplies libc++'s exact `_ZNSt3__112__next_primeEm` ABI using a
Nagi-owned prime search, keeps unsupported locale-facet access fail-closed at
the Nagi abort boundary rather than returning a fabricated facet, and adds a
target-owned IEEE round-to-even `nearbyint` with a selective archive seed.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #152 (2026-09-24)

Public CI run `35944501706` (#152, head `12b0e40`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. `Build Nagi user init` ran for roughly twenty-two minutes
and then failed at the real target link with
`std::bad_alloc::bad_alloc()`, `std::bad_alloc::what() const`, and `islower`.
The UEFI loader and real QEMU first-web-pixel steps were skipped. The bounded
repair now defines the unversioned libc++ `std::exception`/`std::bad_alloc`
Itanium ABI in the Nagi-owned freestanding C++ runtime and adds guest-memory
independent ASCII/C-locale `islower` to Nagi relibc, with an explicit static
archive seed. This is target-runtime work; no host C++ runtime, host locale,
or synthetic rendering is introduced. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Current M17 continuation after CI run #145 (2026-09-24)

Public CI run `35927852465` (#145, head `157958d`) passed target bootstrap,
dependency validation, Mesa Softpipe, package, and kernel stages. The target
user-init build then failed after roughly seventeen minutes; the UEFI loader
and real QEMU first-web-pixel steps were skipped. The public job annotation
exposed only the step failure, not the compiler detail, so the next targeted
experiment seeds the exact real relibc pthread symbols used by the new
libc++ mutex/condition-variable bridge before the archive scan. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #146 (2026-09-24)

Public CI run `35930495046` (#146, head `f90d12c`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The target user-init custom build command still failed
after the target-link stage; the public annotation exposed no symbol-level
diagnostic, and UEFI/QEMU were skipped. The pthread provider seeds therefore
did not complete the link. The next targeted repair adds the real Itanium
deleting-destructor (`D0`) entrypoints for libc++ mutex and condition-variable
objects, including real relibc-backed destruction and allocator release. M17
remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #147 (2026-09-24)

Public CI run `35933099876` (#147, head `216d909`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The target user-init custom build command still failed
after the target-link stage; the D0 destructor repair did not complete the
build, and UEFI/QEMU were skipped. The public annotation again contained only
the generic custom-build error. The target diagnostic parser is now extended
to preserve clang/runtime/linker/undefined-symbol details in that annotation
for the next repair. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #148 (2026-09-24)

Public CI run `35934736445` (#148, head `8a98bf8`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The target user-init compile then failed at
`tools/mesa/nagi-cxx-runtime.cpp:851:29` because the newly added real mutex
destructor bridge called `pthread_mutex_destroy` without a forward
declaration. The public annotation exposed the exact compiler error after the
diagnostic-parser repair; UEFI and real QEMU first-web-pixel acceptance were
skipped. The next bounded repair adds that declaration only. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #149 (2026-09-24)

Public CI run `35937071116` (#149, head `f5aebed`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The target user-init compile passed the prior missing
`pthread_mutex_destroy` declaration, then the real link exposed target-owned
providers still required by the pinned graph: `lrint`, `llrint`, and
`std::__1::__call_once(unsigned long volatile&, void*, void (*)(void*))`.
UEFI and real QEMU first-web-pixel acceptance were skipped. The next bounded
repair adds Nagi relibc `lrint/llrint` exports and a libc++ ABI `__call_once`
bridge backed by guest pthread mutex/condition-variable primitives; it does
not import host libm/C++ runtime or weaken M17 acceptance. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #150 (2026-09-24)

Public CI run `35939582983` (#150, head `fb6cdf0`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The target user-init link passed the `lrint`, `llrint`,
and libc++ `__call_once` repairs, then exposed missing target time/locale
providers: `localtime_r`, `tzname`, and `setlocale`. UEFI and real QEMU
first-web-pixel acceptance were skipped. The next bounded repair adds a
guest-clock UTC `struct tm` conversion, C/POSIX locale handling, and guest
UTC timezone globals in Nagi relibc; no host time or locale is imported. M17
remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #151 (2026-09-24)

Public CI run `35942115871` (#151, head `d56c79f`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The target user-init link passed the target time/locale
repair, then exposed `_Unwind_GetCFA`, `_Unwind_FindEnclosingFunction`, and
`strncat`. UEFI and real QEMU first-web-pixel acceptance were skipped. The
next bounded repair keeps the no-unwinder boundary fail-closed and adds a
guest-memory `strncat` implementation; it does not import host libunwind or
host libc. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #140 (2026-09-24)

Public CI run `35911899646` (#140, head `3546db3`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. The target user-init link resolved `__isnormal`, `__isnormalf`,
and `frexp`, then exposed the real MozJS static-archive ordering boundary:
`JS::NewArrayBufferWithContents(...)`,
`JS::RestoreMicroTaskQueue(...)`, and `__gxx_personality_v0`. UEFI and real
QEMU first-web-pixel acceptance were skipped. The next repair adds the
target-only MozJS archive-order patch `0015`, which retains the real jsglue
object and rescans `js_static`, plus a fail-closed Nagi C++ personality ABI.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #142 (2026-09-24)

Public CI run `35919768358` (#142, head `0526f17`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. The ordered raw lld archive state resolved the prior MozJS
ArrayBuffer/microtask provider and personality failures. The target user-init
link then exposed `scalbn`, `__cxa_bad_typeid`, and
`std::__1::mutex::lock()`. UEFI and real QEMU first-web-pixel acceptance were
skipped. The next repair adds target-owned scaling, libc++ mutex ABI routing
to relibc pthreads, and fail-closed typeid handling. M17 remains `BLOCKED`;
M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #143 (2026-09-24)

Public CI run `35923751011` (#143, head `d3564a0`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. The target user-init link resolved `scalbn`,
`__cxa_bad_typeid`, and `std::__1::mutex::lock()`, then exposed the real
libc++ condition-variable and mutex-destruction boundary:
`std::__1::condition_variable::notify_all()`,
`std::__1::condition_variable::wait(unique_lock<mutex>&)`, and
`std::__1::mutex::~mutex()`. UEFI and real QEMU first-web-pixel acceptance
were skipped. The next repair routes these operations to relibc pthreads with
real ownership checks. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

**Reference target:** QEMU x86-64 / q35 / UEFI / 4 vCPU / 8 GB RAM

### Current M17 continuation after CI run #141 (2026-09-24)

Public CI run `35916232106` (#141, head `4b3c5f8`) passed the target bootstrap,
dependency, Mesa Softpipe, package, and kernel stages. `Build Nagi user init`
stopped before link resolution because rustc rejected the duplicate
`static:+whole-archive=jsglue` modifier with `overriding linking modifiers from
command line is not supported`. The next repair keeps the pinned source and
replaces that syntax with ordered raw lld archive state flags. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

## 1B. CI normalization checkpoint (2026-09-19)

CI normalization is now part of the pushed M17 repair stream. The boundary is
explicit: Ubuntu validates format, host-compatible lint/build/test, and POSIX
launcher acceptance; Windows validates host-compatible build/test plus
PowerShell launcher exit propagation; a target job bootstraps the locked Servo
revision, builds the M16 package artifact, and builds the Nagi kernel,
Nagi-user init, and UEFI loader for their intended targets.

Servo is fetch/bootstrap managed: `third_party/sources.lock` is authoritative,
`third_party/servo/` is generated state, and
`third_party/servo-patches/` is the tracked Nagi patch boundary. M17 remains
`BLOCKED` until real target bootstrap, guest integration, and first web pixel
acceptance evidence pass. Host and target CI are being used as the repair loop;
neither host rendering nor a host-only build marker is M17 acceptance evidence.

## 1A. M16-after / M17-before Architecture Alignment Checkpoint

This documentation checkpoint is recorded after M16 PASS and before M17
Servo Bootstrap. It does not advance, reopen, or alter any M0-M16 milestone,
and it does not start M17.

Checkpoint result: documentation/specification alignment complete. The
repository now defines a provider-neutral Decision capability boundary,
separate Deterministic Fast Path, Decision, and Generative/Reasoning lanes,
capability/role-based Model Router and Model Manager rules, a typed
DecisionProvider contract with batch-capable concepts, and a Jev-free
`LlmDecisionAdapter` fallback. IBM Granite 4.2 3B remains Default Standard;
Qwen3 4B remains the alternative Standard; Gemma 3 1B remains Lite.

The checkpoint does not implement Jev, a Decision Provider, an AI runtime,
Model Manager, IDL, Cargo dependency, model package, or cloud service. It
does not change kernel, loader, user-space runtime, Servo, package, History,
third-party, QEMU, or acceptance-test behavior. M17 remains `NOT STARTED`.

The working tree contained an untracked `third_party/servo/` directory before
this checkpoint. It was not modified, removed, or staged. The checkpoint is
therefore clean with respect to its documentation scope, but the overall Git
working tree remains non-clean until that pre-existing out-of-scope state is
handled by an explicitly authorized later task.

---

# 2. Fixed architecture decisions

These are already decided unless the user explicitly changes them.

- Product name: **Nagi OS (陷・ｽｪ)**
- Nagi has its **own kernel** and is not Linux-based.
- Official 0.1 target is the QEMU x86-64 reference VM only.
- Kernel design: capability-based hybrid kernel.
- Core IPC: Channels + transferable handles.
- Large shared data: VMO/shared memory.
- Native process model: spawn-oriented, not fork-oriented.
- Filesystem baseline: VFS + ext2.
- Stable file identity: Object ID separate from path/inode.
- GUI: Nagi Window Server + software compositor.
- Browser: **Albert**
- Browser engine: **Servo only**
- 0.1 rendering baseline: software rendering / Mesa Softpipe path.
- Network: user-space `nagi-net` + smoltcp.
- Package extension: `.xapp` (provisional but current spec).
- POSIX strategy: user-space compatibility, relibc Nagi backend first.
- Default Standard LLM: **IBM Granite 4.2 3B**
- Alternative Standard LLM: Qwen3 4B
- Lite LLM: Gemma 3 1B
- Generative LLM runtime: llama.cpp / GGUF; future Decision Providers are
  not fixed to this runtime.
- STT baseline: whisper.cpp + multilingual Whisper small.
- AI is local/offline-first, user-space and untrusted.
- AI execution: structured Plan -> deterministic Validator/Policy -> Executor.
- Transaction/Undo must exist before AI receives meaningful OS mutation abilities.
- Recovery: Local History + Wayback + A/B System + Recovery Environment.
- Physical hardware support is outside Nagi 0.1 completion criteria.

---

# 3. Milestone table

Use only these statuses:

- `NOT STARTED`
- `PARTIAL`
- `BLOCKED`
- `PASS`

| Milestone | Scope | Status | Evidence / Notes |
|---|---|---|---|
| M0 | Repository / Toolchain / CI | PASS | `5f3b5b8`: configured executable/version probes, OVMF allow-list, clean safety, and launcher acceptance passed. On 2026-10-01, the POSIX launcher was corrected to select Cargo and rustc rustup shims together when system Cargo appears first in PATH; the mocked shim-selection regression and full M0 launcher/image acceptance pass. |
| M1 | UEFI -> Kernel | PASS | PowerShell and Git Bash QEMU acceptance both passed; serial log contained `Nagi Kernel started` from the guest kernel |
| M2 | Memory / Exceptions / Interrupts | PASS | `cec6167`: real QEMU acceptance passed page allocation/free, APIC timer interrupts, vector-14 page fault handling, and invalid-access diagnostics; host memory tests also passed |
| M3 | SMP / Scheduler / Threads | PASS | `2e95c40`: ACPI MADT discovery, real INIT/SIPI AP startup, four online CPUs, timer-frame context switching, wait/wake workload, and PowerShell/Git Bash QEMU acceptance passed |
| M4 | Handles / VMO / IPC | PASS | `9533c63`: final host/cross-build checks and both real QEMU acceptance paths passed |
| M5 | First User Process | PASS | `48882b6..789b92a`: real INIT.ELF booted in a bounded ring-3 address space, used native SYSCALL, preserved/sanitized user FPU state, and passed PowerShell/Git Bash QEMU acceptance |
| M6 | Init / Supervisor / Service Registry | PASS | `9e3e72b..bcd0236`: bounded user-space Supervisor/registry, real `echo@1` call, M6 CLI gate, and both real QEMU acceptance paths passed |
| M7 | Block / Filesystem / Persistent Storage | PASS | `65d9160`: real legacy VirtIO Block, capability-checked sector ABI, bounded user-space VFS/ext2, persistent 16 MiB data disk, file-backed mapping, and both two-boot QEMU acceptance paths passed |
| M8 | CLI Foundation | PASS | `7b1ec49`: bounded user-space `nsh`, real guest VFS commands, process/memory/log diagnostics, QEMU serial transport, and PowerShell/Git Bash acceptance passed |
| M9 | Display / Input / First Window | PASS | `c084f70`: real QEMU VirtIO/VNC scanout, Surface VMO, capability-checked display/input syscalls, first user-space window, QMP-delivered real mouse/keyboard events, and PowerShell/Git Bash acceptance passed |
| M10 | Nagi UI / Desktop | PASS | `87f3b25`: bounded user-space UI toolkit, bitmap Font Service, Japanese text path, four simultaneous app clients, generalized QMP event transport, and PowerShell/Git Bash real-QEMU acceptance passed |
| M11 | Login / Permissions / Security | PASS | `744bc86`: authoritative update below; real QEMU acceptance passed |
| M12 | Networking | PASS | `user/nagi-net` uses pinned smoltcp behind the capability-scoped raw VirtIO boundary; real QEMU DHCP, ICMP, UDP/DNS, ARP, TCP, and HTTP acceptance passed |
| M13 | Rust std / POSIX | PASS | Corrective closure implemented; focused host tests, target builds, formatting checks, and unified PowerShell/Git Bash real-QEMU POSIX/relibc + Rust std acceptance passed |
| M14 | Audio | PASS | Revalidated after review: QEMU `dsound` backend, real VirtIO Sound playback/capture with non-zero capture signal, modern VERSION_1/FEATURES_OK negotiation, bounded AudioService/mixer, volume/mute, session gates, invalid-capability denial, and PowerShell/Git Bash acceptance wrappers passed on 2026-09-19; `out/logs/m14-audio.log`. |
| M15 | History / Transaction / Wayback Foundation | PASS | Real guest create/edit/move/delete/restore/undo flow, persistent version/trash files, bounded History Service ledger with logical app/session/node/object context, and PowerShell/Git Bash acceptance wrappers passed on 2026-09-19; `out/logs/m15-history.log`. |
| M16 | Package / SDK | PASS | Out-of-tree SDK sample emitted a real NAPP artifact; `nagi-pkg` packaged it, the IDL generator reproduced the checked-in Rust/C bindings, Ed25519 signatures were verified with tamper rejection, and QEMU loaded the host `.xapp` through guest VFS for install/list/info/launch/update/atomic replace/remove. Focused host suite, target builds, signed package CLI, PowerShell wrapper, and QEMU acceptance passed on 2026-09-19. Completion Sweep found and fixed an allocator cfg omission for the standalone `m16-package` feature; the target then linked and reached QEMU. The macOS rerun stopped at the prerequisite M14 capture check (`Nagi M14 capture FAIL`, no CoreAudio input); M16 install/update markers were not reached. SDK/package/IDL artifacts and the failed local run are preserved under `out/evidence/completion-sweep-regression-20260930/`; the prior M16 acceptance remains the PASS evidence. |
| M17 | Servo Bootstrap | PASS | Public CI #303 (`36355494134`, head `31bf815`) passed the Windows launcher, Ubuntu host, and authoritative `nagi-target` jobs. Real QEMU passed the Servo/Mesa Softpipe first-web-pixel gate: nonzero guest frame checksum, copy and present through Nagi Surface, registered Servo resources, and ELF constructors before user entry. Local real-QEMU regressions passed on 2026-09-29 and 2026-10-03. The 2026-10-03 rerun uses an explicit `/tmp/nagi-m17-servo` fixture storage root; its nonzero frame checksum and PASS marker are preserved under `out/evidence/m17-first-web-pixel-20261003/`. The immediately preceding failure was caused by a reused User Data VFS with all 64 inodes allocated to prior Servo temporary roots; that failed disk and trace are preserved under `out/evidence/m17-storage-init-abort-20261003/`. |
| M18 | Albert Browser | PARTIAL | **Acceptance PASS locally and in CI on 2026-09-29:** corrected commit `eb22702` passed CI run [`36533931477`](https://github.com/RT-NISH/NagiOS/actions/runs/36533931477) across Windows launcher, Ubuntu host, and `nagi-target`. Clean Servo bootstrap, M17 QEMU first-web-pixel, M18-B chrome, and `./nagi m18` three-site HTTPS/QEMU acceptance all passed. A 2026-10-03 rerun also passed: QEMU verified TLS chains and hostnames for `example.com`, `example.org`, and `example.net`, rendered all three through Nagi Surface, and passed browser temporary-storage cleanup. Current evidence is under `out/evidence/m18-completion-sweep-20261003/`, with the prior fixed-path image, User Data, vars, and logs preserved under `out/evidence/m18-pre-sweep-20261003/`. macOS uses a Darwin-only ELF linker adapter for target links; the Mesa `-latomic` probe remains enabled. Ubuntu's Clang/LLD route is unchanged and verified. On 2026-10-03, the generated Servo cache was archived and regenerated from the pinned checkout and patches. Added an opt-in M18 acceptance delegate with a localized opaque site-permission modal; it holds the real Servo request pending until a fresh Allow/Deny click or Escape Cancel and denies on input/render/timeout failure. Sixty `nagi-albert` acceptance-feature tests pass. The 2026-10-03 `./nagi m18` rerun passed the three HTTPS pages, but none requests a permission; exact image, User Data, OVMF vars, serial log, screenshot, and verified SHA256 manifest are under `out/evidence/m29-browser-1790978167816192000/`. **Clipboard (2026-10-05):** new user-space `nagi-clipboard` service (ADR 0053) with attenuable READ/WRITE endpoints and per-tab one-shot paste gestures recorded only by Albert's trusted input path; Albert now focuses the active WebView and moves keyboard focus to the page after address submission or a page click. Local `./nagi m18` run `1791179851416415000` passed the three HTTPS pages plus QMP-driven Ctrl+C / click / Ctrl+V between page fields (pasted value observed through the page), followed by an ungestured read denied with `no-user-gesture`; serial log, command log, screenshot, and SHA256SUMS are under `out/evidence/m29-browser-1791179851416415000/`. The service is in-process with Albert, not yet behind authenticated Channel IPC. **IME (2026-10-05):** new user-space `nagi-ime` (ADR 0054) composes hiragana from romaji with katakana/hiragana candidates; Nagi 0.1 has no kanji conversion by user decision. Albert routes page keys through it only while Servo reports a focused text field and sends composition start/update/end events. Local `./nagi m18` run `1791180856033701000` passed the HTTPS pages, clipboard, and a QMP-typed Ctrl+Space `nihongo` Enter committed as `にほんご` into the page field (DOM value observed through the page); evidence under `out/evidence/m29-browser-1791180856033701000/`. **Text rendering (2026-10-05):** the M18 HTTPS acceptance had been passing pages that painted no text — Nagi's Servo font registry was empty, so all three sites produced the same background-only frame (`0x5a9955c5`). Bundled Noto Sans/Noto Sans JP (ADR 0055, Servo patch 0026) are published read-only under `/system/fonts/`; file-backed `mmap`/`munmap` now accept non-page-multiple lengths as POSIX requires (the failing step found by diagnostic patch 0027). Each page now reports `ink_pixels` and the validator requires at least 200. Local `./nagi m18` run `1791185005598104000` passed with `ink_pixels=5632` per site and a screenshot showing the pasted token and committed `にほんご`; evidence under `out/evidence/m29-browser-1791185005598104000/`. Production authenticated policy/IPC, interactive QEMU permission acceptance, and download/upload destinations remain. |
| M19 | Semantic Layer / Search | PARTIAL | Integrated `user/nagi-search` into the root workspace and added a bounded two-slot guest snapshot backend plus target VFS adapter. Twenty-nine Search tests, warnings-denied Clippy, format, Nagi target compile, and QEMU persistence/rename acceptance pass. Guest executes bounded M21 `file.search` through ContextResolver, Validator, Action Registry, and Executor; the M22 fixture records its executed result in NAL1 with `transaction_id=None`. Caller policy remains fixture-only. Real Files/page producer synchronization and authenticated production IPC remain. See `docs/workstreams/NagiOS_M19_Semantic_Layer_Search_Workstream.md`. |
| M20 | AI Runtime / Granite | PARTIAL | `third_party/models.lock` pins IBM Granite 4.2 3B and its exact artifact metadata; a separate disposable QEMU disk passed full guest-visible digest verification through the read-only Model Store, but no model has been loaded for inference and the regular M30 image remains empty. The tracked Nagi llama.cpp patch stack runs through 0034, adding checked failures across model/context/state/file/mmap/vocabulary/memory, DSV4, sampler, and quantization paths plus a Nagi-only static backend initialization path while preserving upstream behavior elsewhere. Host and Nagi-macro loader-bounds builds pass; focused CTests pass 1/1 in both, and `nagi-cli` passed 196 unit + 21 integration tests. The no-exceptions syntax sweep passes 32/32 top-level `src/*.cpp` files, and the full LLVM 19/libc++ Nagi target build linked `libllama.a` (42/42 steps, 6.4 MiB; log `out/logs/m20-loader-status-0033-noexceptions-target-build-llvm19.log`). QEMU run `1790962398371656000` links the static llama/ggml CPU archives into `nagi-init` and passes `llama_backend_init()`, CPU registration, and target C++/ctype/math smoke checks; its image, target ELFs, archives, serial log, disks, variables, and SHA-256 manifest are under `out/evidence/m20-llama-link-smoke-1790962398371656000/`. The 2026-10-03 inference integration attempt adds a seekable read-only Model Store callback descriptor, a guest ModelBackend adapter, structured-output grammar, memory sizing, and a reproducible CLI acceptance command. The target archives and provider adapter compile. The first final link reported 142 unresolved symbols; after explicitly linking relibc and adding Nagi stdio/ctype/wchar providers, a fresh official CLI attempt still fails before image creation with 88 unresolved target C++ standard-library symbols (streams, strings, locale, filesystem, regex, random device, shared ownership, exceptions, and thread/future). The exact updated log and attempt record are under `out/evidence/m20-granite-inference-1790985332307332000/`; the initial link log remains under `out/evidence/m20-granite-inference-1790984128856579000/`. No guest inference or QEMU acceptance is claimed. Linking a host libc++ archive is not valid for Nagi. The required target-owned C++ runtime/provider set remains open. On 2026-10-03, the prior ignored generated llama and Servo caches were preserved before `./nagi fetch` regenerated and validated fresh checkouts from their pinned revisions and patches. See `docs/workstreams/NagiOS_M20_AI_Runtime_Granite_Workstream.md`. |
| M21 | Planner / Validator / Executor | PARTIAL | `services/nagi-ai` supplies the authoritative NagiPlan@1 schema to model requests and independently validates provider output at the generic adapter boundary; deterministic parsing and Validator checks remain in force. Bounded guest `file.search`, fixture-scoped `file.move` and `file.copy` run through ContextResolver, Validator, Action Registry, capability/object checks, and Executor against real VFS state; executed search results flow into M22 NAL1. Executor reacquires grants for each step; a new two-step regression revokes the second capability after step one and verifies `Partial/CapabilityDenied` with no second handler call. All 25 AI tests, Clippy, and formatting pass. Policies, handlers, and caller identity remain fixture-only; production IPC/authenticated caller authority, model service integration, and general first-party actions remain. See `docs/workstreams/NagiOS_M21_Planner_Validator_Executor_Workstream.md`.
| M22 | AI Safety / Undo Integration | PARTIAL | Fresh three-boot QEMU runs preserve digest-bearing grouped Move, bounded Copy, NAL1/NH16 persistence, Undo, and restart restoration; standalone run `1790999599873700000` and both repetitions in `out/evidence/m28-run-20261003T035340Z-63277/` passed. The M22 bootstrap and all three numbered guest boots now share one guarded pre-guest retry: it requires the running-CPU/QMP firmware-timeout signature and unchanged writable boot/User Data SHA-256 values, with per-boot evidence sidecars archived by M28. The current M22 runs did not need this retry; unit tests cover restoration, disk-change suppression, and three-boot sidecar isolation. Fixture caller/policy, real inference, authenticated production authority, general production actions, and a production Activity Ledger service remain. See `docs/workstreams/NagiOS_M22_AI_Safety_Undo_Integration_Workstream.md`. |
| M23 | Nagi Bar / Context / Albert AI | PARTIAL | Added the bounded, fail-closed public Browser Context API and trusted visibility checks for selected Object/Workspace context; browser page content is labeled untrusted at the provider boundary. Twenty-four `nagi-ai` tests, warnings-denied Clippy, formatting, and Nagi no-std target compile pass. Live Servo extraction, authenticated guest policy/IPC, Nagi Bar UI, and real inference remain; the formal page-summary acceptance is not met. See `docs/workstreams/NagiOS_M23_Nagi_Bar_Context_Albert_AI_Workstream.md`. |
| M24 | Embedding / Semantic AI | PARTIAL | Added a bounded exact `PersistentVectorIndex` over `SnapshotBackend`, opaque embedding-space identity checks, versioned/checksummed snapshots, atomic object replacement, stable top-k ranking, and visible-ObjectId filtering. Twenty-nine `nagi-search` tests, warnings-denied Search and CLI Clippy, 135 CLI unit + 21 integration tests, formatting, and Nagi no-std target compile pass. M19 two-boot QEMU restored the semantic index after restart; M22 three-boot QEMU revalidated index restore with NH16/NAL1 Undo. Logs, disk images, user-data disks, and OVMF variables are preserved under `out/evidence/m24-persistent-semantic-index-20261001/`. Guest inference uses a deterministic test provider. A multilingual embedding model, content-producer synchronization, hybrid ranking/explanations, stale-index invalidation, reference-scale performance evidence, and the formal natural-language acceptance remain. See `docs/workstreams/NagiOS_M24_Embedding_Semantic_AI_Workstream.md`. |
| M25 | Voice | PARTIAL | Added a bounded no-std push-to-talk coordinator and a replaceable TTS provider/synthesis service with 1 KiB UTF-8 input, 4 KiB PCM chunks, a 1 MiB output cap, frame checks, empty-output rejection, failure cleanup, and a target AudioService playback sink. `SpeechToTextProvider::finish() == Ok(0)` fails closed as `EmptyTranscript`, cancels provider state, clears output, and cleans up the pipeline. Added a fixed-memory, 127-tap Q15 low-pass converter and provider adapter for 48 kHz stereo S16LE to 16 kHz mono S16LE; filter state spans capture chunks, tail flushes at finish, and cancel clears state. Twenty audio tests pass, including chunk-boundary equivalence, channel averaging, and steady-state stop-band rejection. A fresh target fixture verifies authorization/indicator ordering and converted PCM delivery before exercising unavailable/empty/fixed-transcript cleanup; it does not claim inference or execute the fixture transcript. `./nagi m25` passed QEMU run `1790954944032231000` (log `out/logs/m25-voice-1790954944032231000.log`); `./nagi fmt` and warnings-denied `nagi-audio` Clippy pass. The pinned Whisper small multilingual artifact is downloaded, and the opt-in `./nagi m25-whisper <artifact.bin>` acceptance installs it only in a disposable GPT Model Store image; QEMU verifies its exact size, GGML magic, and SHA-256 through the read-only guest capability (`out/evidence/m25-whisper-artifact-1790895162586172000/`). It does not load whisper.cpp or perform inference. On 2026-10-02, CLI tests passed (165 unit, 21 integration), as did `./nagi fmt`, `./nagi test`, `./nagi lint`, `./nagi build`, and QEMU regressions `./nagi m25`, `./nagi m19`, and `./nagi m22`. whisper.cpp is pinned; `./nagi fetch` validates the clean upstream source and generates a separate Nagi-patched checkout. The Nagi-target `whisper` CMake build and host GGUF parser/writer regression passed on 2026-10-01; related evidence is under `out/evidence/m25-whisper-noexceptions-{pre-final-rerun,final-pass}-20261001/` and `out/evidence/m25-whisper-target-compile-20261001/`. On 2026-10-02, numbered patch 0002 hardened standard model reads and tensor-header EOF handling; its host loader regression and Nagi-target `whisper` build passed. A clean-source CI run exposed that the initial zero-context patch could not be applied; contextual hunks now pass forward and reverse `git apply --check`, with blank unified-diff context whitespace scoped in `.gitattributes`. `./nagi fetch` regenerated Whisper with the expected patch and checkout fingerprints, then stopped at the pre-existing modified generated Servo checkout. The exact 487,601,967-byte Whisper artifact matches SHA-256 `1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b`, with source and guest evidence under `out/evidence/m25-whisper-model-download-20261002/` and `out/evidence/m25-whisper-artifact-1790895162586172000/`. QEMU has no host `virtio-sound.in` driver. On 2026-10-03, a Nagi-target Whisper Small provider was connected to the existing speech provider API; it loads the locked model through the read-only guest Model Store capability and enters inference on the Japanese PCM fixture. QEMU run `1790978330938078000` loaded the model through the read-only Model Store capability and emitted `Nagi M25 Whisper Japanese fixture inference PASS` after the real transcript contained the supplied expected phrase. The QEMU image check passed and all eight SHA256 evidence entries verify from the evidence directory. The short 1.48-second fixture took about 37 minutes under TCG; no microphone or transcript execution was involved. Authenticated microphone permission/UI, real capture, local TTS, multi-utterance/latency/resource acceptance, and voice-command acceptance remain. See `docs/workstreams/NagiOS_M25_Voice_Workstream.md`. On 2026-10-03, `./nagi m25` also passed current-source QEMU orchestration run `1790986320984425000`; the six-file evidence manifest is under `out/evidence/m25-voice-current-source-1790986320984425000/`. This fixture covers permission/indicator order and cleanup but no real capture or speech synthesis. |
| M26 | Qwen / Gemma / Automatic | PARTIAL | Added deterministic role/capability/resource/provider-health model routing, strict manual override checks, and safe unavailable fallback while retaining Granite as Standard default. Qwen's locked 2,497,280,256-byte GGUF passed host and read-only guest SHA-256 verification in disposable Model Store image `out/artifacts/nagi-0.1-m26-qwen-1790897766859340000.qcow2`; evidence manifest is under `out/evidence/m26-qwen-artifact-1790897766859340000/`. This verifies integrity and guest readability only, not model loading or inference. The Gemma guest feature compiled; no Gemma weights were obtained or used, and the CLI requires explicit `--accept-gemma-terms`. On 2026-10-02, 167 CLI unit + 21 integration tests, lock/manifest tests, `./nagi fmt/test/lint/build`, both M26 Nagi-target feature builds, and fresh M19/M22 guest regressions passed. Qwen package/install workflow, model loading/inference, Gemma terms/notice review and user acknowledgement, guest provider routing, switching UI, and real routing acceptance remain. See `docs/workstreams/NagiOS_M26_Model_Routing_Workstream.md`. |
| M27 | A/B / Recovery | PARTIAL | Both current-source M27 sub-runs in `out/evidence/m28-run-20261003T035340Z-63277/` passed GPT A/B rollback, healthy-B readiness promotion, Recovery journal preservation, and committed M22 Undo across restart; manifests verify. Repetition 1 reproduced the 90-second pre-guest OVMF loop at Recovery Undo restart verification; unchanged writable-disk SHA-256 permitted one retry, which reached the original marker. The archived 18 MiB User Data images are raw GPT disks with valid primary/backup header and table CRCs; `qemu-img check` does not support raw images. Intermittent firmware stalls remain unexplained. Authenticated update/readiness authority, authenticated slot manifests, full session readiness, and remaining Recovery work remain. See `docs/workstreams/NagiOS_M27_AB_Recovery_Workstream.md`. |
| M28 | Integration / Stress | PARTIAL | The latest current-source run `out/evidence/m28-run-20261003T035340Z-63277/` passed two consecutive repetitions of M19 Search, three-boot M22 Move/Copy + NH16/NAL1 grouped Undo, and M27 GPT A/B/Recovery; the archive and M27 sub-run manifests verify. Repetition 1 exercised M27's guarded Recovery restart retry. The previous current-source run `out/evidence/m28-run-20261003T033818Z-60771/` also passed 2/2; `out/evidence/m28-run-20261003T032603Z-59318/` passed 0/2 after M22 boot 3 stopped pre-guest and remains a failed archived run. Formal Desktop/Files/Notes/Albert combined load, real Granite inference/fairness, audio pressure, OOM, and leak soak remain unmeasured. See `docs/workstreams/NagiOS_M28_Integration_Stress_Workstream.md`. |
| M29 | Developer Preview Polish | PARTIAL | Current `./nagi m29` QEMU run `out/evidence/m29-settings-1790896342100947000/` passed Japanese Settings selection and same-disk restart restoration; READY arrived in 2,444 ms. Its eight-entry SHA256SUMS verifies the screenshot, image, User Data, OVMF vars, three serial logs, and README; the screenshot is byte-identical to the tracked acceptance image. Cross-process language propagation, onboarding, complete localization/accessibility, broader UI/performance evidence, user-facing provider/recovery UX, and human license review remain. See `docs/workstreams/NagiOS_M29_Developer_Preview_Polish_Workstream.md`. |
| M30 | Nagi OS 0.1 Release | PARTIAL | Completion Sweep commit `65f4d6f8773e0b373f237067960738f66e454f3b` produced a source-bound 64 GiB GPT qcow2 with SHA-256 `bda4e15274f52b9005497d05dff0ff732d01aac93d8fec5cd26fe87b50929258`. Fresh QEMU run `1790926329664045000` passed System A, User Data persistence, Recovery, unstaged System B rejection, post-Recovery restart, M19 Search, M22 Ledger/Move/Copy, and the separate M20 Model Store reader fixture. Clean-source release preflight, assembly to `out/artifacts/m30-release-bundle-65f4d6f/`, verification, all 23 bundle checksums, image byte identity, and pristine/bundle `qemu-img check` passed. Evidence manifest: `out/evidence/m30-release-1790926329664045000/SHA256SUMS`. The release manifest retains `m30_acceptance=NOT_EVALUATED`; QEMU has no host audio input driver. A stale image bound to `25e0b54` was preserved with verified checksums under `out/evidence/m30-release-symlink-stale-image-65f4d6f/`. Authenticated updates/System B acceptance, remaining M18–M29 work, and human redistribution review keep M30 `PARTIAL`. See `docs/workstreams/NagiOS_M30_Release_Workstream.md` and `docs/decisions/ADR-0013-m30-reference-disk-layout.md`. On 2026-10-03, clean source commit `4fae6875d64752db8fbe0508a932c28da246e8af` rebuilt the reference qcow2 (SHA-256 `1e81c7a89b4295bcadebfd835d4942ad53849ee1f81be3cb7ff5cc05395f379d`) and passed `./nagi m30` run `1790985901890315000`: System A, User Data restart persistence, Recovery, unstaged System B rejection, post-Recovery restart, M19 Search, M22 grouped undo/ledger, and separate M20 Model Store reader fixture. Clean-source release preflight, assembly to `out/artifacts/m30-release-bundle-4fae687/`, verify, all 23 bundle checksums, byte identity, and bundled qcow2 check also passed. Evidence and a verified manifest are under `out/evidence/m30-release-1790985901890315000/`; the superseded source-65f4d6f image/sidecar are preserved under `out/evidence/m30-stale-image-pre-4fae687-20261003/`. `m30_acceptance=NOT_EVALUATED` remains correct; human redistribution review and authenticated update acceptance remain open. Current-source clean commit `9b16eaae729b8c61403aa929912de5ab5da19d4b` rebuilt the reference image (SHA-256 `e815da59636642c91b06fb6d9f75b038113eabadf7dfd2eb4a251cd61ac2f349`) and passed `./nagi m30` run `1790988888019354000`; preflight, assembly to `out/artifacts/m30-release-bundle-9b16eaa/`, all 23 checksums, image identity, and qcow2 verification passed. The 11-entry run manifest is at `out/evidence/m30-release-1790988888019354000/`. `m30_acceptance=NOT_EVALUATED` remains the correct release-manifest value. |

---

# M19 - Semantic Layer / Search (`PARTIAL`)

The M19-PREP deterministic metadata/search implementation is integrated into
the root workspace. Nineteen host tests cover stable object metadata,
visibility filtering, producer mapping contracts, snapshot recovery, and
search after reopen. The target `m19-search` feature persists a bounded
two-slot snapshot through the guest VFS; `./nagi m19` passed remount and a
second QEMU boot using the same disk. That QEMU path uses a private fixture,
not live Files/page producers or a production Search IPC service. Guest VFS
files remain limited to 1 KiB, so the fixture acceptance caps snapshots at
4 KiB. Authenticated capability-to-object visibility and canonical producer
Object IDs remain M19 blockers.

See `docs/workstreams/NagiOS_M19_Semantic_Layer_Search_Workstream.md` for the
focused evidence and remaining production acceptance criteria.

# M20 - AI Runtime / Granite (`PARTIAL`)

The `nagi-model-manager` package streams artifact bytes through a fixed 8 KiB
buffer and checks their SHA-256 before any backend load. Its Granite profile
identifies IBM's Q4_K_M GGUF snapshot, upstream byte count/digest, and
Apache-2.0 notice. The exact pinned artifact is retained in ignored local
cache and is not bundled. The exact llama.cpp revision is in
`third_party/sources.lock`.

Nagi's numbered llama.cpp patch stack now runs from `0001` through `0022`.
It bounds GGUF metadata and tensor reads, reports parser/loader/model/adapter
failures through checked status under `__NAGI__`, and preserves ordinary host
exceptions. Patches 0021–0022 cover model architecture metadata, model-load
failure propagation, and early expert-count validation; the fresh generated
checkout and verified manifests are under
`out/evidence/m20-loader-status-0022-20261002/`.

The fresh-cache host and Nagi-macro `llama` targets and loader-bounds targets
build, and their focused CTests pass 1/1. All 185 CLI library tests,
`./nagi test`, `./nagi fmt`, `./nagi lint`, and `./nagi build` pass. The full
no-exceptions Nagi `llama` build advances past the 0021–0022 model failures and
currently stops in unity 1–4 with 44 reported throw diagnostics across 19
remaining model files. See
`out/logs/m20-loader-status-0022-noexceptions-target-build-fresh.log`.
Unfiltered CTest requires many unrelated binaries that are not built by these
focused configurations; the explicit loader-bounds CTest passes in both host
and Nagi-macro configurations. Broader CMake `all` attempts also hit unrelated
auxiliary targets lacking `mtmd.h`, `build-info.h`, and `arg.h`. STL allocator
OOM recovery remains unsupported under the no-unwinder ABI.

M20 remains `PARTIAL`: there is no complete Nagi-compatible llama.cpp target
backend, trusted guest installer/catalog, active lazy-loading model service,
or in-guest Granite response/QEMU inference acceptance. The disposable
acceptance image verifies storage and digest only.

The 2026-10-03 Completion Sweep added a read-only seekable callback descriptor
to the Nagi POSIX runtime and a `LlamaCppBackend` path that feeds the pinned
Model Store artifact through `fdopen`/`llama_model_load_from_file_ptr` with
`mmap` disabled. The target adapter constrains one structured JSON response
with llama.cpp grammar and leaves final schema validation to `ModelRuntime`.
`./nagi m20-granite-inference <artifact.gguf>` builds pinned target archives,
assembles the disposable Model Store image, and requests guest inference.
The callback helper's three focused tests and all 200 `nagi-cli` library tests
pass; `./nagi fmt` passes. The first target integration link failed with 142 unresolved symbols. After
explicitly linking relibc and adding Nagi stdio/ctype/wchar providers, a fresh
official CLI attempt still fails before image creation with 88 unresolved
target C++ standard-library symbols. The earlier `fdopen` and other C/POSIX
providers no longer appear in the link failures. The updated link log is
`out/evidence/m20-granite-inference-1790985332307332000/target-link-after-relibc-providers.log`
(SHA-256
`19442fbc322d5fc5168717d5567f81890e4735ad96c774610038b5b8ffb90927`); its
attempt README and target archive build log are in the same directory. The
initial 142-symbol link log and README remain under
`out/evidence/m20-granite-inference-1790984128856579000/`. No guest model load,
inference, unload/restart, or inference QEMU acceptance is claimed. M20 remains
`PARTIAL`; a target-owned C++ runtime with thread, locale, and filesystem ABI
integration is required.

See `docs/workstreams/NagiOS_M20_AI_Runtime_Granite_Workstream.md` for exact
commands, provenance, and the concrete runtime acceptance gap.

# M21 - Planner / Validator / Executor (`PARTIAL`)

`services/nagi-ai` now defines strict complete-document parsing for
`NagiPlan@1`, a bounded prompt adapter over the existing
`GenerativeProvider`, a provider-neutral Decision candidate interface and
`LlmDecisionAdapter`, and confidence-based fallback routing. Context is
filtered to visible stable Object IDs before prompt construction. The Action
Registry is explicit and allow-listed; Validator checks schema/version,
registered action, parameter bounds, capability decisions, and object access
for the entire plan before execution. Executor reacquires capability grants
and object handles for each step, exposes no host path/shell fields, and
reports completed steps plus a partial failure without pretending to roll back.

Sixteen host tests cover the orchestration contract and SearchService action
integration; the `no_std` service compiles for the Nagi user target. The new
`register_file_search_action` binds the existing M19 SearchService to
`file.search`, bounds the query to 128 bytes and results to 64 Object IDs, and
returns only matches allowed by its injected visibility filter. The test runs
the action through plan validation and execution and excludes another app's
private file. Its in-memory snapshot backend and visibility implementation are
test fixtures; this is not guest filesystem acceptance.

The guest init acceptance paths now compose `file.search` with the M19 Search
Service and bounded M22 `file.move` and `file.copy` actions against real guest
VFS files. The copy plan resolves one source Object ID, requires the fixture
`files.copy` capability, accepts only the fixed `m22-copy` basename, and caps
the payload at 512 bytes. It passes ContextResolver, Validator, the Action
Registry, capability/object checks, and Executor before persisting NH16 Create
Prepared and NAL1 Prepared, writing the destination, and committing both
records. The copy action rejects a denied capability and path injection.
These are deterministic fixture plans and policy, not real model inference or
authenticated caller authority. General `app.launch`, `file.copy`, `file.move`,
and `system.volume.set` production handlers, along with the authenticated
target Capability/Permission and Context authorities, remain absent. M21
remains `PARTIAL` until these actions are connected to their real services and
trusted guest policy.

See `docs/workstreams/NagiOS_M21_Planner_Validator_Executor_Workstream.md` for
the exact evidence and remaining integration requirements.

# M22 - AI Safety / Undo Integration (`PARTIAL`)

M22 acceptance requires an authorized M21 file-move action to record one
transaction containing all three moves, preserve caller/app/session/Node/
workspace/Object context in the existing Activity Ledger, and restore all
three files after `undo`, including after a restart. The current M21 branch
has no production Action Registry handlers or authenticated target policy
adapter, so the formal production guest AI mutation cannot be performed
safely. A private M22 fixture now exercises the same bounded M21 Executor
contract against three fixed guest VFS files without claiming production
authority.

The existing `user/nagi-history` `HistoryService` retains the M15
Create/Edit/Move/Delete/Restore API and its legacy `undo_last` behavior. The
new `NH16` recoverable archive API preserves full-width caller context, names,
snapshot bytes, transaction IDs, and state under a bounded checksum. A grouped
move begins `Prepared`, must be persisted before the external moves, and is
made undoable only after the caller commits and persists the group. Undo checks
the originating AppId/AppSessionId, persists `UndoPending` before applying
inverse operations, returns actions in reverse order, and marks the group
`Undone` after completion. Restoring an `UndoPending` archive yields the same
batch for retry. The older `NH15` serializer remains metadata-only and is
still used by the M15 guest acceptance; it is not treated as recoverable.
`user/nagi-history/src/guest.rs` now adds an inactive-slot-first, flushed
two-slot archive store that fits the existing 1 KiB guest VFS limit. The
`m22-history` init feature exercises Prepared, Committed, UndoPending, and
Undone persistence against real guest VFS files, including idempotent replay
of an interrupted reverse batch.
History also supports a recoverable Prepared Create payload for the fixture
`file.copy` action; its Delete inverse participates in caller-scoped Undo.

The separate bounded `NAL1` AI Activity Ledger records user intent, optional
selected model, action and plan summary, logical context, Object IDs,
transaction ID, and result transitions without chain-of-thought. Its
checksummed `NLA1` two-slot store uses separate VFS files. The M21 Executor
passes validated intent to the fixture action handler, which persists the
ledger separately from NH16 and verifies it again after reboot.

The 2026-10-01 Completion Sweep added a fresh-disk acceptance for both fixture
actions: boot 1 commits grouped `file.move` and `file.copy`; boot 2 undoes both;
boot 3 verifies restored sources, absent `m22-copy`, NH16, NAL1, and M24
semantic-index persistence. `cargo test --locked --offline -p nagi-ai
-p nagi-history --all-targets` passes 24 AI and 17 History tests;
`cargo test --locked --offline -p nagi-cli` passes 152 unit and 21 integration
tests. `./nagi fmt`, the Nagi-target M22 init check, touched-package
warnings-denied target Clippy, and `./nagi build` pass. The fresh run's image,
User Data disk, OVMF vars, bootstrap/serial logs, and seven-file SHA-256
manifest are under `out/evidence/m22-file-copy-1790854068023718000/` and its
unique `out/artifacts/` and `out/logs/` paths.

Focused verification on the pinned aarch64 macOS toolchain:

- `cargo test --locked --offline -p nagi-ai -p nagi-history --all-targets` —
  PASS, 24 AI orchestration tests and 14 History/Activity Ledger tests.
  Activity Ledger coverage includes archive round-trip, malformed text and
  transitions, object limits, corruption/version rejection, and two-slot
  fallback.
- `cargo clippy --locked --offline -p nagi-history --all-targets -- -D warnings`
  — PASS.
- `cargo fmt --manifest-path user/nagi-history/Cargo.toml -- --check` — PASS.
- `cargo -Z build-std=core,alloc check --locked --offline -p nagi-history
  --target targets/x86_64-unknown-nagi-user.json` — PASS.
- `cargo -Z build-std=core,alloc check --locked --offline -p nagi-init
  --features m22-history --target targets/x86_64-unknown-nagi-user.json` — PASS.
- Target `cargo clippy --no-deps -Z build-std=core,alloc --locked --offline
  -p nagi-init --features m22-history --target
  targets/x86_64-unknown-nagi-user.json -- -D warnings` — PASS for the touched
  package. Existing dependency warnings remain outside `nagi-init`.
- `cargo test --locked --offline -p nagi-cli` — PASS, 114 unit and 18
  integration tests.
- Fresh-disk `./nagi m22` — PASS: boot 1 ran M21 `file.move`, reopened the
  Committed NH16 transaction and NAL1 record, and verified destinations; boot
  2 persisted grouped Undo plus NAL1 `UndoPending`/`Undone`; boot 3 verified
  restored files and the complete ledger after restart. Current logs are
  `out/logs/m22-history-boot-1.log` through `m22-history-boot-3.log`, with the
  prior image/disk/vars/logs preserved under
  `out/evidence/pre-m22-ai-activity-ledger-20260930/`.
- `./nagi m22` — PASS: durable NH16 History and separate NAL1 Activity Ledger
  recovery verified
  across QEMU boots on the same persistent guest disk. An initial boot exposed
  that M7's root lookup buffer assumed at most eight files; it is now bounded
  by the VFS's 64-inode capacity. Logs: `out/logs/m22-history-boot-1.log`,
  `out/logs/m22-history-boot-2.log`, and `out/logs/m22-history-boot-3.log`.

M22 remains `PARTIAL` for its production AI acceptance. The private QEMU
fixture now passes through a real M21 Executor action and proves guest VFS
durability, separate NAL1 ledger persistence, and restart-restorable grouped
Undo. It does not use real AI inference, authenticated target policy, or a
production Activity Ledger. The
remaining gate must bind the action to authenticated caller capabilities,
record/verify the caller-scoped production Activity Ledger transaction, undo
through that same authority boundary, reboot, and verify both files and ledger.

The local M15 QEMU regression did not reach its History flow. A separate
empty-`PT_TLS` loader defect was corrected and all 15 standalone ELF parser
tests passed. The M13 C POSIX test also exposed a stale eight-byte
`sockaddr_in` fixture; matching Nagi's 16-byte IPv4 socket ABI allowed the
real C socket/DNS/HTTP checks to pass. The guest then passed M14 playback but
failed capture because local QEMU reports `Can not open virtio-sound.in` and
`no host audio driver`. The wrapper timed out without reaching M15 History.
This host-audio limitation does not substitute for, or count as, M22
acceptance.

See `docs/workstreams/NagiOS_M22_AI_Safety_Undo_Integration_Workstream.md` for
the inspected API limits and acceptance blockers.

# M17 - Servo Bootstrap (`PASS`)

M17's formal First Web Pixel acceptance passed in public CI #303, as recorded
at the beginning of this section. The dated blocker entries below are the
historical state at each earlier CI run. The implementation applies sorted
tracked Servo, Surfman, and libc patches, records generated checkout
revisions plus patch/worktree fingerprints, and refuses stale or unsafe
generated state without overwriting it. The M17 QEMU boot image is read-only,
and the writable user-storage capability excludes read-only VirtIO devices
so the first persistent-write gate cannot alter the FAT12 ESP.

The following blocker inventory was reclassified on 2026-09-20 and is retained
as historical context. Present-tense wording inside these dated entries
describes the state at the time; it does not reopen M17 after CI #303 passed.

Internal and actionable during the M17 workstream:

- Servo's target dependency graph needs the pinned local libc 0.2.189 source,
  the Servo workspace boundary, patched `std`, and a complete Nagi user
  runtime/link path;
- CI #81 (`d6edcd1`, run `35501349699`) passed the Tokio adapter in the target
  graph and stopped at `getrandom 0.4.3`'s deliberate unsupported-target
  `compile_error!`. This is an actionable Nagi prerequisite, not a host
  dependency: the reference QEMU command already supplies `virtio-rng-pci`.
  The current repair adds a bounded legacy VirtIO RNG driver, `SYS_RANDOM_GET`
  with user-range validation, and the `getrandom_backend="custom"` hook in
  `libnagi`; it does not use host entropy, RDRAND, a fixed seed, or an
  unsupported-success fallback. Target compilation and guest entropy evidence
  are still required.
- CI #69 reached the Nagi user-init target build after the Mesa archive,
  standalone package/UEFI dependency fetch, M16 package artifact, and kernel
  stages passed. The first Rust dependency then failed in `serde_core 1.0.229`
  because Nagi `std` was still marked `restricted_std`, which made normal
  `std` use unstable for every dependent crate. The tracked Rust std target
  support patch now recognizes `target_os = "nagi"` as a supported std
  environment; this repairs the target contract at its source rather than
  patching `serde_core` or adding a host fallback. CI must re-run the complete
  target build to verify the next boundary.
- CI #70 passed the patched Rust std boundary and compiled `serde_core`, then
  exposed the next libc integration defect: Servo's pinned `libc 0.2.189`
  failed in `src/new/mod.rs` because its Unix-wide `pub use unistd::*` had no
  Nagi platform module. The existing patch only covered legacy
  `src/unix/nagi.rs`. The tracked libc patch now adds the minimal
  `src/new/nagi`/`unistd` adapter and reexports the real POSIX descriptor
  constants; it does not remove the new API or redirect libc to a host OS.
- the pinned Mesa 24.3.0 revision now has a tracked Nagi platform/static
  Softpipe patch boundary and a relibc-header-driven build helper. A local
  pinned-source Meson configure now reports EGL `nagi surfaceless`, Gallium
  `softpipe`, and static `glapi`/`EGL`/`softpipe` targets; CI #36 compiled
  Mesa through `os_time.c` before reaching the next Nagi fcntl open flag
  adapter gap. CI #39 then accepted the fcntl open flags and stopped at
  `src/util/os_memory_fd.c` because the generated target `sys/mman.h` did not
  expose `PROT_READ` or `PROT_WRITE`;
- the Surfman adapter now selects Nagi static EGL/surfaceless code without
  X11/Wayland, and `nagi-albert` has the real Servo `SoftwareRenderingContext`
  handoff, but no guest pixel evidence exists yet;
- the tracked Mesa static-loader patch now makes Nagi's optional dynamic
  loader explicitly unavailable, keeping the first-pixel path on statically
  linked EGL/Softpipe without host library lookup;
- the target build still needs the complete relibc C ABI header generation,
  Mesa archive link, Servo build, and QEMU acceptance sequence. The
  `open_memstream` declaration/runtime slice and Mesa's Nagi monotonic
  clock/sleep adapter are now implemented in the target-only relibc Nagi
  adapter and tracked Mesa include/patch boundaries. The current repair adds
  the Nagi access and open/create flag values required by Mesa's `os_file.c`.
  The current repair adds the Nagi mmap protection/mapping constants required by
  Mesa's file-backed Softpipe utility path.

- CI #43 reached Mesa object 103/946 and stopped at `src/util/u_qsort.cpp`
  because the freestanding target could not resolve its unused `<thread>`
  header. The tracked `0005-nagi-qsort-freestanding.patch` removes only that
  unused standard-library include; it does not add a host C++ runtime or fake
  thread behavior.
- CI #44 reached Mesa object 118/946 and stopped at
  `src/util/texcompress_astc_luts.cpp` because the freestanding C++ target did
  not provide `<cstdint>`. The source review found that this common utility is
  only needed by Mesa's optional ASTC GPU-transcode path. The tracked
  `0006-nagi-disable-astc-cpp-transcode.patch` keeps Nagi on Mesa's existing
  CPU ASTC fallback, removes the optional C++ LUT utility from the Nagi build,
  and makes the optional transcode hook fail truthfully; it does not add a
  host C++ standard library or claim ASTC GPU-transcode support.
- CI #46 applied the ASTC patch successfully and reached the next Mesa
  compile boundary, where the static-loader fallback used `NULL` without a
  Nagi `stddef.h` include. The tracked `0007-nagi-static-loader-null.patch`
  adds only that real standard-header dependency; it does not re-enable a
  dynamic loader or introduce host library lookup.
- CI #47 applied the static-loader header patch and reached `u_debug.c`, where
  Mesa used `strcasecmp` without including the POSIX `strings.h` declaration.
  The tracked `0008-nagi-debug-strings-header.patch` adds that header and uses
  the existing real relibc `strcasecmp` implementation; no compatibility stub
  is introduced.
- CI #48 applied the `strcasecmp` header patch and reached the Mesa loader DRM
  UAPI compile, where the BSD fallback requested missing `sys/ioccom.h` for
  Nagi. The tracked `0009-nagi-drm-uapi-ioctl-header.patch` selects Nagi's
  existing `sys/ioctl.h` ABI while preserving the UAPI type definitions; it
  does not add a host DRM dependency.
- CI #49 applied the DRM UAPI patch and reached `src/compiler/nir/nir_from_ssa.c`,
  where Mesa's `c99_alloca.h` relied on a host libc's transitive `stdlib.h`
  declaration for `alloca`. Nagi relibc intentionally provides the real
  compiler-builtin macro in its separate `alloca.h`; the tracked
  `0010-nagi-alloca-header.patch` includes that header only for `__NAGI__`.
  It adds no allocator implementation or host runtime dependency.
- The bootstrap user address space now reserves eight mmap page tables (16 MiB)
  instead of one (64 KiB), with range validation across table boundaries. The
  fixed 32 KiB POSIX bump allocator is being replaced by a lock-protected
  first-fit free list backed by one 8 MiB Nagi anonymous `SYS_MEMORY_MAP`
  region; the existing relibc allocation-size prefix contract remains intact.

Environment-specific, not product blockers:

- local Windows host `link.exe`/MSVC CRT absence prevents host-side Cargo
  linking for checks that compile target-dependent build scripts; it does not
  justify host rendering or stopping M17, because the target verification path
  is Ubuntu CI/QEMU;
- remote CI/QEMU execution is verification work still outstanding, not an
  external architecture dependency.

Verification checkpoint on 2026-09-20:

- `cargo metadata --format-version 1 --locked --offline --no-deps`, the
  tracked-package format check, `cargo check -p nagi-cli --lib --tests`, and
  `cargo clippy -p nagi-cli --lib --tests -- -D warnings` passed in the M17
  worktree;
- GitHub Actions run #39 (`9cb4e8f`) passed Ubuntu host checks and target
  dependency/bootstrap stages; its target Mesa build accepted the fcntl open
  flag adapter and stopped at `src/util/os_memory_fd.c` because the generated
  target `sys/mman.h` did not expose `PROT_READ` or `PROT_WRITE`.
- GitHub Actions run #36 (`5207b64`) passed Ubuntu host checks and target
  dependency/bootstrap stages; its target Mesa build accepted the real
  `open_memstream` and Nagi `os_time` paths, then stopped at
  `src/util/os_file.c` because relibc's generated target header did not expose
  `O_CREAT`, `O_EXCL`, `O_WRONLY`, or `O_RDONLY`. The tracked `fcntl.h`
  adapter now exposes those values from the Nagi libc ABI for the next target
  run;
- CI #40 accepted the Nagi mmap header boundary and stopped at Mesa
  `src/util/os_misc.c`, where Nagi was not included in the supported
  system-information branches. The tracked `0004-nagi-os-misc.patch` adds the
  real Nagi `unistd.h` path and reports the unavailable physical-page query as
  unsupported; `nagi-posix::sysconf` now exposes the real 4096-byte
  `_SC_PAGE_SIZE` value for the page-size path;
- CI #43 reached the next real Mesa object boundary after the `os_misc.c` fix,
  then stopped at `u_qsort.cpp` because clang could not find `<thread>` for the
  `x86_64-unknown-elf` freestanding C++ compile. The tracked qsort patch is the
  next target-build repair. The same run's Ubuntu host job reported a Rust
  format failure in the previously changed `nagi-posix/src/abi.rs` import order;
  that formatting defect is corrected in the current worktree, while the
  Windows host job passed.
- GitHub Actions run #49 (`1449072`) passed the target dependency/std/Servo
  bootstrap stages and entered the full Mesa build. It reached
  `nir_from_ssa.c` and stopped only at the missing `alloca` declaration; the
  next run verifies the tracked `0010-nagi-alloca-header.patch`.
- GitHub Actions run #50 (`46460d8`) passed the previous `alloca` boundary and
  reached `src/compiler/spirv/spirv_to_nir.c`, where `strcasecmp` was still
  undeclared. The tracked `0011-nagi-strings-header.patch` exposes relibc's
  existing POSIX `strings.h` through Mesa's common `u_string.h` for Nagi, so
  this remains a declaration-boundary repair rather than a compatibility stub.
- GitHub Actions run #51 (`98ce540`) passed the `strcasecmp` boundary and
  reached Mesa GLSL C++ compilation, where the freestanding target had no
  `<new>` header. The Nagi-owned `tools/mesa/nagi-headers/new` now provides
  placement-new/nothrow language declarations without importing host C++
  headers; ordinary allocation operator definitions remain a target-link
  prerequisite and are not claimed complete until the Nagi allocator link
  verifies them.
- GitHub Actions run #52 (`d0092d2`) passed the `<new>` header boundary and
  reached `src/util/enum_operators.h`, where the freestanding target had no
  `<type_traits>`. Source review found that the selected Nagi Softpipe build
  uses only `std::underlying_type_t`; the tracked Nagi header now maps that
  trait to clang's target-language enum builtin. It does not provide a fake
  general-purpose C++ standard library. The next target build is required to
  verify this boundary before addressing any later compile or link failure.
- GitHub Actions run #53 (`b07f659`) passed the Nagi `<type_traits>` header and
  reached the selected GLSL precision pass, where its `std::vector` include
  required an unavailable host C++ STL. The tracked `0012` Mesa patch keeps
  the same stack/child-list behavior but uses Mesa's existing
  allocator-backed `util_dynarray`; it does not add a fake general-purpose
  vector implementation. The next target build must verify the patched
  source and continue to the next concrete boundary.
- GitHub Actions run #54 (`a60ea97`) applied `0012` and compiled the selected
  GLSL precision pass far enough to expose the remaining `std::vector` method
  calls and the out-of-class nested-type spelling in the first patch revision.
  The follow-up keeps every `stack.back()` operation on Mesa's
  `util_dynarray_top` and qualifies `find_lowerable_rvalues_visitor::stack_entry`
  at the free-function assertion. These are target-source corrections, not a
  host STL fallback; the next target run must verify the complete replacement.
- GitHub Actions run #55 (`79011a6`) reached the remaining GLSL precision
  references after the `std::vector` removal. The first follow-up used Mesa's
  `util_dynarray_top` macro with direct member syntax, which expands without
  an object-level parenthesis; the target compiler therefore reported a
  `char *` member access. The correction uses the real pointer form
  `util_dynarray_top_ptr(...)->state` throughout. No host STL or rendering
  substitute is involved.
- GitHub Actions run #56 (`50de662`) confirmed the previous pointer spelling
  still collided with Mesa's unparenthesized `util_dynarray_top_ptr` macro
  expansion. The correction now wraps the dereference before accessing
  `state`, matching the actual macro definition. The target build still has
  not reached the Mesa archive link; the next run is required to verify this
  final GLSL container-access correction.
- GitHub Actions run #57 (`71acdb8`) compiled the patched GLSL precision pass
  and reached Mesa's ASTC CPU decoder, where the freestanding target lacked
  `<cstdlib>`. The decoder uses that include only for two real `abort()` calls;
  `0013` selects relibc's real C `stdlib.h` on Nagi while preserving the
  upstream C++ header on other platforms. The ASTC CPU fallback remains the
  selected first-pixel path.
- GitHub Actions run #58 (`bb9b0c8`) stopped before Mesa compilation because
  the new `0013` patch hunk header counted one extra context line. The patch
  parser rejected it deterministically during the pinned Servo/Mesa bootstrap;
  the header was corrected and the patch now passes local `git apply --check`
  against the generated Mesa inspection checkout. A new target run is needed
  for the actual ASTC compile boundary.
- GitHub Actions run #59 (`6916f57`) passed the corrected ASTC patch and
  compiled the ASTC CPU decoder, then reached Mesa HUD's optional signal-toggle
  code, where `SA_SIGINFO` was not present in the generated Nagi signal header.
  The next patch excludes only that optional signal handler for Nagi; it does
  not disable HUD drawing or add an unimplemented signal constant.
- GitHub Actions run #60 (`67d96c3`) compiled the HUD sources after the Nagi
  signal-toggle exclusion, then reached Mesa's public EGL header, which had no
  `__NAGI__` branch and therefore emitted `Platform not recognized` with
  undefined `EGLNative*` types. The next patch aligns those three types with
  the existing Surfman Nagi surfaceless FFI contract; it does not add a host
  display backend.
- GitHub Actions run #61 (`f93280c`) passed the Nagi EGL native-type boundary
  and reached the Mesa archive/link stage. The remaining failure was the
  unneeded `src/gallium/targets/dri/libgallium-24.3.0.so` shared target, whose
  version script required DRI entry points that the static Nagi EGL/Softpipe
  first-pixel path intentionally does not provide. The tracked
  `0016-nagi-static-egl-without-dri.patch` first isolated the Nagi build graph
  from that target. CI #62 (`02bafba`) then showed that Mesa's EGL configure
  path also uses `with_dri` to compile its real surfaceless DRI2 frontend, so
  the target stopped earlier with `No EGL driver available`. The tracked
  `0017-nagi-static-egl-dri-frontend.patch` now restores `with_dri` for the
  frontend, links Nagi EGL to the static `libdri`, and gates only
  `targets/dri`; this preserves the real static EGL/Softpipe path without
  generating the unusable shared DRI module.
- GitHub Actions run #63 (`796d5b8`) passed the complete pinned Mesa patch
  stack through the Nagi static EGL/DRI frontend and Softpipe archive link;
  the target job emitted `PASS M17 Mesa static Softpipe build`. It then
  stopped in the existing M16 package-artifact command because the root
  workspace resolved `nagi-loader`'s unconditional `uefi = 0.37.0` dependency
  while running offline, but the target cache did not contain that registry
  package. The loader uses UEFI only in its UEFI binary, so the dependency is
  now kept under its existing exact `cfg(target_os = "uefi")` boundary. This
  is a Cargo dependency-scope repair, not a loader stub or host fallback;
  run #64 must verify the package artifact and continue to the kernel/init
  target build.
- GitHub Actions run #64 (`7d4eae7`) passed the Mesa archive and the real
  `hello-nagi` NAPP build, then exposed the next dependency-boundary defect:
  the root workspace package command still resolved `user/nagi-net` and its
  pinned `smoltcp` dependency even though the target runner intentionally uses
  an offline cache without that unrelated target package. The package builder
  is now a standalone locked workspace at `tools/nagi-pkg/Cargo.toml`, with
  its own generated `Cargo.lock`; the UEFI loader is isolated by the same
  boundary so its exact `uefi` graph is not required by host or M16 package
  jobs. This preserves real package/loader builds and removes dependency
  resolution leakage; it does not stub either component.
- Run #65 (`6a23360`) confirmed the prior diagnosis: Mesa and the sample NAPP
  still passed, while the old root `cargo run -p nagi-pkg --offline --locked`
  command failed on missing cached `smoltcp`. Its Ubuntu and Windows host jobs
  also stopped at a lockfile mismatch introduced while testing the old mixed
  workspace boundary. The current repair removes both standalone packages
  from the root lock graph, updates the supported CLI/CI entry points to their
  manifest paths, and retains exact standalone locks; local locked workspace
  checks now reach only the known Windows `link.exe` boundary. The next pushed
  run is the authoritative check of this repair.
- An intermediate local lock experiment added `x11-dl` to the root
  `surfman` graph, but that was not the Nagi rendering path: the tracked Servo
  patch removes Surfman's `sm-x11` feature for Nagi. CI #65 showed that this
  mixed root lock was not valid for the Linux host workspace. The current
  repair removes that stale root edge and keeps target-only UEFI/package
  graphs in their standalone exact locks. The root and standalone locked
  metadata checks now pass locally; the local target build again reaches only
  the Windows `link.exe` boundary, and the next CI run will verify the same
  graph on Ubuntu and the Nagi target.
- CI #67 (`7b13209`) passed Ubuntu format, host lint/build/test, and M0 doctor
  after the root lock repair. Its M0 launcher then exposed the expected
  artifact-layout follow-up: standalone UEFI builds emit
  `loader/target/x86_64-unknown-uefi/release/nagi-loader.efi`, while the CLI
  image step still looked under the old root `target/` directory. The CLI now
  reads the standalone loader output path; this is an integration-path repair,
  not a host or guest stub.
- The same CI #67 target job passed the Mesa archive and real NAPP generation,
  and then failed inside the correctly isolated package workspace only because
  its exact `cfg-if 1.0.5` registry source was not yet present in the runner's
  offline cache. The package command no longer resolved unrelated `smoltcp`.
  CI now fetches the standalone package and UEFI lock graphs with `--locked`
  before entering the intentional offline build steps; no version is floated
  and no package or loader implementation is bypassed.
- the target-only relibc backend now also exports `mmap`, `munmap`, and
  `mprotect` through the existing Nagi POSIX VMO/VFS facade, so the Mesa
  file-backed memory path has a real guest mapping ABI at final link time;
- the new Mesa 0006 source patch passes tracked-source `git apply --check` and
  cleanly applies to the pinned Mesa inspection checkout; remote target build
  verification is pending for the next CI run;
- the local target `nagi-albert` check reached Rust std, compiler-builtins,
  libc, and host build-script compilation, then stopped at the Windows-only
  absence of `link.exe`; no target-source diagnostic was produced before that
  host link failure;
- the local host environment still cannot run the Mesa header/build/QEMU
  sequence because `make`, `cbindgen`, and QEMU are unavailable. Ubuntu CI
  remains the next real target verification environment; no acceptance PASS is
  claimed from this local checkpoint.
- Focused local kernel tests and the host `nagi-posix` check are additionally
  blocked at the Windows `link.exe`/MSVC CRT boundary after source compilation
  begins; this remains environment-specific and is not treated as a target
  implementation result.
- The exact local M16 sample build still reaches the same Windows-only
  `link.exe` absence, so it cannot provide host artifact evidence here. The
  corrected locked/offline workspace metadata succeeds; Ubuntu target CI is
  the authoritative rerun for the package artifact.
- CI #69 (`bc0f027`) passed Ubuntu host checks, Mesa static Softpipe archive
  build, locked standalone package/UEFI dependency fetch, real M16 package
  artifact generation, and the Nagi kernel build. The Nagi user-init target
  build then stopped in `serde_core 1.0.229` with the concrete
  `restricted_std` diagnostics described above. The Rust std patch now adds
  Nagi to `library/std/build.rs`'s supported-target list; this is the next
  target-build experiment and has been checked against the pinned source with
  `git apply --check --ignore-space-change`.
- CI #70 (`13e7239`) passed Rust std preparation, Servo bootstrap, Mesa
  static Softpipe archive, standalone dependency fetch, M16 package artifact,
  and kernel build. It compiled patched `std`, `serde_core`, and `serde`, then
  stopped at `libc 0.2.189` with `unresolved import unistd` in
  `src/new/mod.rs:255`; UEFI and first-web-pixel steps were skipped. The
  tracked libc patch now applies cleanly to the pinned 0.2.189 source and
  exposes the Nagi `new` namespace adapter. The local source-only check reaches
  the known Windows `link.exe` boundary while building target dependencies, so
  CI remains the authoritative target compile verification.
- CI #71 (`3ecb510`) confirmed that the libc API repair was not yet selected by
  the Nagi parent workspace: the target build still compiled the registry path
  `libc-0.2.189/src/new/mod.rs`. The existing `[patch.crates-io]` entry in the
  nested Servo workspace is not inherited when Cargo resolves the path
  dependency from Nagi's root workspace, whose lockfile still recorded the
  registry source. The root workspace now pins `libc` to the generated,
  fingerprint-checked `third_party/libc-servo` checkout and the root lockfile
  records that path source. A local target dependency-tree check with the exact
  patched checkout resolves `libc v0.2.189` from that Nagi-owned path; the next
  CI run must verify the locked target compile and continue to the next real
  boundary.
- CI #72 (`09aaf4d`) verified the root-workspace wiring: Ubuntu and Windows host
  gates passed, and the target passed Servo bootstrap, Mesa static Softpipe,
  standalone dependency fetch, M16 package artifact, and kernel build. The
  target then compiled `libc v0.2.189` from `third_party/libc-servo` but still
  failed at `src/new/mod.rs:255`. Reproduction showed that the generated
  checkout was missing the tracked `src/new/nagi` hunk because the bootstrap
  helper invoked `git -C` on a copied source directory nested inside the Nagi
  repository; Git therefore resolved the parent repository instead of treating
  the copy as an independent patch root. The helper now applies with
  `git --directory=<checkout-relative-to-root>` and has a regression test that
  exercises a patch inside the parent workspace. The next CI run must verify
  the actual patched source compile.
- CI #73 (`752d9ef`) verified that generated libc patch application now reaches
  the target user-init compile: Ubuntu host, Mesa Softpipe, package artifact,
  and kernel stages passed, and the target compiled `libc 0.2.189` from the
  Nagi-owned checkout. It then exposed a real source-compatibility defect in
  the tracked Nagi libc module: its function declarations used the older
  `{const}`/implicit-safe macro syntax, while libc 0.2.189 requires explicit
  `const unsafe` or `const safe` forms. The follow-up `bdf4aa1` updates only
  those declarations and normalizes the Git `--directory` argument to `/` for
  Git-for-Windows. The same run's Windows bootstrap separately failed during
  patch check at the two existing-file hunks (`src/unix/mod.rs` and
  `src/new/mod.rs`); this remains an actionable cross-platform bootstrap issue,
  not a reason to stop target repair.
- CI #76 (`4582b3a`) passed the Ubuntu/Windows host gates and the target
  dependency, std, Mesa, package, and kernel stages, then stopped at the
  pinned `socket2 0.6.5` source because its Nagi cfg surface still referenced
  unsupported libc socket constants and types. The tracked socket2 patch now
  narrows those options for Nagi and keeps the existing real TCP path.
- CI #78 (`ce1c297`) passed target dependency/std/Servo bootstrap, the complete
  Mesa static Softpipe archive, standalone package/UEFI dependency fetch, the
  real M16 package artifact, and the Nagi kernel. It then reached the pinned
  `mio 1.2.3` compile and stopped because the first Nagi mio patch revision
  accidentally excluded the real pipe module while selecting the pipe waker;
  its poll selector also still exposed `Registry`/`Poll` raw-fd methods that
  the Nagi in-memory selector does not implement. The tracked follow-up keeps
  the real `pipe2`/poll waker path enabled and excludes only those unsupported
  raw-fd extension impls for Nagi. The clean-source patch check passes; the
  next target run must verify mio and continue through Servo link/runtime.
- CI #79 (`094ba65`) verified that the mio repair compiled in the target
  graph. The next pinned dependency, `socket2 0.6.5`, then exposed six
  target-source errors: the first patch enabled two `IovLen` definitions,
  excluded the libc `IP_TOS`/`IP_RECVTOS` exports without excluding all of
  their methods, and left the Nagi `msghdr.msg_iovlen` assignment ambiguous.
  The new ordered `0002` socket2 patch removes Nagi from the incompatible
  `c_int` branch, disables only the unsupported TOS APIs, gates the unused
  IPv6 import, and leaves the real Nagi `usize` msghdr ABI selected. Both
  patches now pass clean-source check/apply validation; the next target run
  must verify this corrected socket2 boundary.
- CI #80 (`4c3b239`) verified that the corrected socket2 boundary compiled in
  the target graph and reached Tokio. The target had already passed Servo
  bootstrap, Mesa static Softpipe, package/UEFI, kernel, and mio; Tokio then
  selected Unix-domain `mio` types, Unix credential/signal paths, and socket2
  TOS accessors under `cfg(unix)`, although Nagi intentionally does not expose
  those APIs. The new pinned `tokio 1.53.1` patch excludes only those
  unsupported Nagi features through Tokio's own cfg graph and preserves the
  real TCP/UDP runtime path; it adds no Unix-domain fake or TOS stub. Clean
  source patch check/apply and the bootstrap source-lock check pass locally;
  the next target run must verify Tokio and continue to the Servo link/runtime
  boundary.
- The Tokio source is now part of the same reproducible registry boundary as
  libc, mio, and socket2: exact version/checksum in `sources.lock`, sorted
  Nagi patch application, generated-checkout fingerprinting, and a root Cargo
  path patch. It is not target-verified until CI compiles it and the later
  target/QEMU first-web-pixel gate passes.
- The Nagi POSIX runtime now contains the corresponding bounded guest pipe,
  `poll(-1)`, `fcntl`, readiness, EOF, and broken-pipe path used by the mio
  adapter. It is not treated as target-verified until the target build and
  subsequent QEMU acceptance exercise it.
- The new entropy slice stays below the existing kernel boundary: the kernel
  owns PCI/VirtIO transport and copies bounded RNG output into a validated user
  buffer, while `libnagi` owns only the syscall wrapper and getrandom symbol
  adapter. The QEMU launcher now explicitly selects the legacy VirtIO RNG
  transport used by the driver. This is an implementation checkpoint, not M17
  acceptance evidence.
- CI #82 (`9843136`) verified the real getrandom custom backend through target
  compilation. The next target-only failure is the pinned WebRender 0.70
  `wr_glyph_rasterizer` path selecting `freetype-sys` through its legacy Unix
  condition; that path is independent of Servo's `bundled_freetype` feature
  and therefore invokes host `pkg-config` during the Nagi cross-build. The
  Nagi Albert target dependency now explicitly enables the existing pinned
  `freetype-sys 0.23` `bundled` feature, preserving real FreeType compilation
  without host library lookup. CI must verify this source build and expose
  the next target/runtime boundary before M17 acceptance can be attempted.
- CI #83 (`f11605f`) verified the bundled FreeType C build and reached the
  Servo user-init compile. It then stopped in `ipc-channel 0.23.0`: its
  platform module only selects Unix backends for Linux/BSD/illumos and has no
  Nagi branch, so all backends were cfg'd out. Because M17 does not enable
  Servo multiprocess, the Nagi adapter now selects ipc-channel's existing real
  `force-inprocess` crossbeam transport for same-process/thread IPC. This is
  a target compatibility selection, not a host socket or fake channel; CI
  must verify it and continue to the next boundary.
- CI #84 (`60355b3`) verified the real in-process IPC backend and reached
  Servo's allocator compile. It then exposed the missing Nagi libc symbol
  `malloc_usable_size`, required by `servo-allocator` for its standard-system
  allocator introspection. The Nagi-owned POSIX heap now validates its
  allocation header and reports the recorded payload through a dedicated
  `nagi_posix_malloc_usable_size` ABI, while the pinned Servo libc patch
  exposes the libc declaration and relibc forwards to that runtime boundary.
  This reports real Nagi allocation metadata and does not disable Servo
  allocator accounting or substitute a host allocator; CI must verify the
  target link and continue.
- CI #86 (`f55a7d7`) reached the target user-init compile and exposed a
  malformed patch hunk in the pinned Servo libc adapter: the added
  `src/unix/nagi.rs` file declared 1,412 added lines while its hunk header
  declared 1,411, dropping the closing `cfg_if!` delimiter during patch
  application. The patch hunk count is corrected; this is a tracked source
  boundary repair, not a generated-cache edit. The next CI run must verify
  that the patched libc parses and continue to the target link/runtime gate.
- A pinned-Servo source audit also identified a compile-required target cfg
  gap before final linking: Servo's `gaol` dependency and constellation
  sandbox profile treat x86_64 Nagi as Linux-like, although gaol has no Nagi
  platform backend. `0004-nagi-single-process-no-gaol.patch` excludes Nagi
  from those gaol/profile/spawn branches and selects the existing unsupported
  path. This is consistent with M17's `default-features = false` single-
  process embedder; it does not enable host process spawning, fake sandboxing,
  or a replacement browser security boundary. The patch is tracked under the
  existing Servo ordering/fingerprint boundary and must be target-verified.
- CI #87 (`5ed2486`) reached the target user-init compile after the libc patch
  and exposed a real C cross-build boundary in `aws-lc-sys`: its `cc-rs`
  invocation used the host `cc`, and strict C11 feature visibility hid the
  host rwlock declarations. Enabling host pthread headers would be incorrect,
  because the host `pthread_rwlock_t` layout is not Nagi's four-byte relibc
  ABI. The target path now uses `tools/nagi-target-cc.sh`, which compiles
  target C helpers as freestanding ELF against generated relibc headers and
  Clang resource headers only. `tools/mesa/build.sh` also performs a focused
  generated-header syntax/layout check before Mesa. This is an internal Nagi
  build prerequisite; the next target CI run must verify the real aws-lc
  objects and continue to the next Servo/runtime boundary.
- CI #89 (`5f06fc4`) verified the relibc rwlock preflight, Mesa/Softpipe,
  package/UEFI, M16 package, and kernel, and then verified that the new C
  wrapper carried `aws-lc-sys` past the pthread boundary. The next failure is
  `libz-sys 1.1.29`: its bundled gzip sources need Nagi's real `fcntl.h`
  open-flag constants, which were not in the generated target header search
  path. The wrapper and preflight now layer the existing tracked Nagi header
  overlay before generated relibc headers. This preserves the Nagi ABI and
  keeps host standard headers excluded; target CI must verify zlib and expose
  the next dependency boundary.
- CI #90 (`c0d9629`, run `35507706711`) verified the fcntl overlay through
  bundled zlib and reached the target `freetype-sys 0.23.0` build. Its bundled
  libpng compile then failed because the pinned crate passed the relative
  `libz-sys/src/zlib` include path, which is not a valid path from the Cargo
  build directory under Nagi's freestanding `-nostdinc` wrapper. This is an
  internal reproducibility/build-boundary issue, not a local Windows MSVC
  limitation.
- The exact `freetype-sys 0.23.0` registry source is now materialized through
  the existing source-lock, generated-checkout, patch-fingerprint, and
  bootstrap boundary. Its tracked Nagi patch consumes `DEP_Z_INCLUDE`, the
  include metadata emitted by the pinned `libz-sys` dependency, and retains
  the upstream relative fallback only when that metadata is absent. The next
  target CI run must verify the real FreeType/libpng C build and expose the
  next Servo/runtime boundary.
- CI #91 (`fbe88e4`, run `35508629491`) verified the pinned `freetype-sys`
  checkout, its libz include repair, and reached the real `aws-lc-sys`
  target C build. That build exposed two Nagi relibc header issues: the
  generic `stdatomic.h` macros retained `_Atomic` on temporary values passed
  to Clang's `__atomic_*` builtins, and cbindgen emitted no Nagi
  `struct termios`, leaving aws-lc's console backend incomplete. This is an
  internal Nagi C ABI prerequisite, not a host MSVC limitation.
- The relibc C header boundary now strips the atomic qualifier only from the
  temporary value types while preserving the atomic pointer operations, and
  the target-specific redox-compatible termios structure is selected for
  Nagi header generation. Mesa bootstrap adds a real target syntax/size check
  for both interfaces. Target CI must verify aws-lc and continue to the next
  Servo/runtime boundary.
- CI #92 (`92b164c`, run `35509724031`) showed that cbindgen requires an
  explicit `target_os = "nagi"` define before it emits the Nagi termios
  structure. It also showed that Clang rejects the generic `__atomic_*`
  builtins for C11 `_Atomic` object pointers, so the target header needs
  Clang's native `__c11_atomic_*` builtins. These are now selected for Clang,
  while the generic path remains for GCC-compatible consumers. Target CI must
  re-run the Mesa preflight and then verify the aws-lc build.
- CI #93 (`f349b68`, run `35510072330`) verified the Clang C11 atomic repair;
  its remaining failure showed that cbindgen's generated `__nagi__` guard did
  not match the established `__NAGI__` target-wrapper define. The cbindgen
  mapping now emits the existing uppercase guard. Target CI must verify the
  complete termios header and proceed to aws-lc.
- CI #94 (`dedca2d`, run `35510401081`) passed the generated termios and C11
  atomic preflight, Mesa Softpipe, package/UEFI prerequisites, M16 package,
  kernel, and the real `aws-lc-sys` C build. User-init then reached
  `hyper-util 0.1.20`, whose Unix connector unconditionally implemented
  `Connection for tokio::net::UnixStream` under `cfg(unix)` even though the
  pinned Nagi tokio adapter intentionally excludes Unix-domain sockets. The
  pinned hyper-util source now excludes only that connector implementation
  for `target_os = "nagi"`; target CI must verify the patch and continue.
- CI #95 (`1bcb554`, run `35511522322`) was an Actions startup failure: all
  three jobs ended after roughly three seconds with no executed steps or
  target build output. Rerun attempt 2 reproduced the same infrastructure
  failure. This run provides no evidence about the hyper-util patch and is
  recorded separately from the Nagi target build blockers.
- CI #96 (`274a11f`, run `35511637360`) reproduced the same pre-execution
  failure for all three jobs; rerun attempts 2 and 3 also ended with
  `steps=0`. The repository remains clean with the hyper-util patch applied;
  target verification is pending runner recovery, not a newly observed code
  error.
- Public snapshot CI run #3 (`35520298442`, head `95fcc59`) verified the
  executable-mode repair, Servo bootstrap, Mesa Softpipe archive, M16
  package, kernel, Ubuntu host, and Windows launcher. The target user-init
  compile then exposed a real Servo dependency-feature boundary: Servo's
  workspace enabled the `webdriver` crate's `server` default feature for the
  embedded `script` dependency, which pulled `warp` and its Unix listener
  implementation into Nagi. Nagi intentionally has `target_family = "unix"`
  without Unix-domain sockets, so this is not a reason to add unsupported
  Tokio APIs. The tracked Servo patch boundary now disables WebDriver default
  features and enables `server` only in Servo's standalone
  `webdriver_server` package. The next public target run must verify the
  reduced graph and continue to UEFI and the real M17 pixel gate.

- Public snapshot CI run #4 (`35521317614`, head `d1f8685`) verified Servo
  bootstrap on Ubuntu, Windows, and the target job, then correctly stopped at
  two locked-graph boundaries. The Ubuntu and Windows host jobs rejected the
  feature-boundary change because `Cargo.lock` still retained the WebDriver
  server graph; the target graph preflight independently rejected the same
  stale lock. Cargo regenerated the lock from the patched Servo manifests,
  removing `warp` and its server-only transitive packages while retaining the
  protocol-only `webdriver` dependency. The next public run must verify the
  updated lock, target build, UEFI loader, and real QEMU first-web-pixel gate.

- Public snapshot CI run #5 (`35521923686`, head `0ae187c`) passed Ubuntu and
  Windows host checks, the target WebDriver graph preflight, Mesa Softpipe,
  package/kernel prerequisites, and Servo bootstrap. Nagi user-init then
  reached the real Surfman dependency graph and failed in `libloading 0.8.9`:
  the parent workspace had not bound the generated, patched Surfman checkout,
  so registry Surfman enabled Wayland `dlib`/dynamic-loader dependencies under
  the Nagi Unix target family. The existing pinned Surfman patch already
  excludes those host-display paths for `target_os = "nagi"`; the remediation
  binds `surfman` in the Nagi root `[patch.crates-io]` table, refreshes the lock,
  adds a regression test, and makes CI reject `libloading`, `dlib`, and
  `wayland-sys` in the Nagi target graph. UEFI and real QEMU evidence remain
  outstanding.

- Public snapshot CI run #6 (`35522589749`, head `2c3bcd7`) verified the
  Surfman source binding, target graph boundary, Mesa Softpipe, package/kernel
  prerequisites, and host jobs. User-init then reached the next target-only
  ABI boundary: `tempfile 3.27.0` selected its Unix `rustix` backend because
  Nagi reports the Unix target family, and `rustix` referenced 43 libc APIs
  that are outside the M17 Nagi POSIX contract (`statfs`, `dup3`, fcntl
  locking/fallocate constants, and related operations). A Nagi-owned pinned
  tempfile source now uses real std/VFS file operations on `target_os =
  "nagi"` and keeps rustix for supported Unix targets; the source lock,
  bootstrap, patch fingerprint, root Cargo binding, and target preflight are
  tracked. The next run must verify this backend, then continue to UEFI and
  real QEMU first-web-pixel acceptance.

- Public snapshot CI run #7 (`35523783329`, head `41bdd83`) exposed a fresh
  Public-repository bootstrap defect before the Servo target build: all three
  jobs reached `nagi-bootstrap fetch`, but the hosted runners had no Cargo
  registry source cache (`/home/runner/.cargo/registry/src` and the Windows
  equivalent). The bootstrap implementation only searched that cache and
  stopped before it could materialize the locked registry sources. This is an
  internal reproducibility defect, not an external toolchain or M17 acceptance
  failure. The registry bootstrap now invokes Cargo with a temporary manifest
  containing the exact pinned `=version`, then continues through the existing
  source checksum, ordered patch, and checkout-fingerprint validation. The
  temporary manifest is outside the repository and is removed after fetch; no
  latest dependency or host rendering fallback is introduced. The next run
  must verify fresh-cache bootstrap before retrying the Nagi target build.

- Public snapshot CI run #8 (`35524127951`, head `cb93985`) exercised that
  fresh-cache path on Ubuntu and Windows, but the temporary Cargo manifest
  lacked a target and Cargo stopped with `no targets specified in the
  manifest`. The same failure occurred in the Nagi target job before its
  target build. The bootstrap fix now gives the temporary manifest an empty
  library target solely for Cargo dependency fetching; the fetched pinned
  source is still validated and patched into the real generated checkout.

- Public snapshot CI run #9 (`35524229892`, head `0ddd645`) passed fresh-cache
  bootstrap, target dependency preflight, Mesa Softpipe, the M16 package
  artifact, the kernel build, and the Nagi tempfile filesystem backend. Nagi
  user-init then reached the next real Servo runtime boundary: `mozjs_sys
  v153.0.0-2` passed Cargo compilation but its SpiderMonkey configure script
  rejected `x86_64-unknown-nagi-user` with `OS "user" not recognized`. A
  pinned Nagi `mozjs_sys` source checkout and patch boundary now normalize only
  the Mozilla configure triplet to the recognized freestanding
  `x86_64-unknown-nagi` configure identifier; the Rust target, compiler
  wrapper, relibc headers, and guest link remain Nagi-owned. The next run must
  verify this adapter and continue through the actual SpiderMonkey
  compile/link.

- Public snapshot CI run #10 (`35525315524`, head `219639c`) passed the
  clean-runner bootstrap, dependency feature boundary, Mesa Softpipe, UEFI
  prerequisites, M16 artifact, and kernel build. It then failed in the real
  Nagi user-init build inside `mozjs_sys` because the configure-only fallback
  triplet was set to `x86_64-unknown-elf`, which Mozilla's pinned `config.sub`
  treats as an unknown OS rather than a bare-metal object format. The first
  correction to `x86_64-unknown-none` then passed `config.sub` but was rejected
  by Mozilla's configure `split_triplet()` as an unsupported OS. The adapter
  now adds an explicit Nagi configure OS and uses
  `x86_64-unknown-nagi`. This does not change the Rust target, compiler
  wrapper, relibc headers, or guest link boundary. UEFI and first-web-pixel
  acceptance remain unexecuted.

- Public snapshot CI run #11 (`35526109052`, head `d261c4e`) passed all
  bootstrap, dependency-boundary, Mesa Softpipe, package, UEFI-prerequisite,
  and kernel stages, but `Build Nagi user init` again stopped in the pinned
  `mozjs_sys` configure layer: `split_triplet()` rejected the intermediate
  `x86_64-unknown-none` value with `Unknown OS: none`. The next Nagi-owned
  adapter adds `Nagi` to Mozilla's configure OS/kernel enums and preprocessor
  checks, teaches pinned `config.sub` to accept `nagi`, and avoids adding the
  generic `libm` OS library for Nagi. The target Rust identity remains
  `x86_64-unknown-nagi-user`; no host fallback or synthetic rendering was
  added. UEFI and first-web-pixel acceptance remain unexecuted.

- Public snapshot CI run #12 (`35527150705`, head `aadd411`) passed the new
  native Nagi configure identity, including `config.sub`, `split_triplet()`,
  target compiler detection, and `__NAGI__` preprocessor detection. It then
  failed at Mozilla's real compiler policy check because Ubuntu's unqualified
  `clang` was `18.1.3` while the pinned Servo/mozjs source requires LLVM/Clang
  19 or newer. The next repair installs the Ubuntu 24.04 `clang-19`/`lld-19`
  toolchain explicitly, makes it the CI alternative, routes Nagi C/C++
  preprocessing through the freestanding wrapper, and suppresses mozjs_sys's
  host `stdc++` link request for the Nagi target. This is still target build
  prerequisite work; UEFI and first-web-pixel acceptance remain unexecuted.

- Public snapshot CI run #13 (`35528078834`, head `34832d2`) passed pinned
  source bootstrap, Servo dependency-boundary preflight, Mesa Softpipe archive
  build, UEFI dependency fetch, M16 package artifact, and the Nagi kernel
  build. The real Nagi user-init build then reached Mozilla configure with
  Clang 19 and native Nagi detection, but stopped because the inherited
  `AR` value was `x86_64-unknown-nagi-user-ar`; no such target-prefixed GNU
  archiver exists in the pinned toolchain. The next adapter repair binds only
  this archiver lookup to `llvm-ar`, already required by the pinned Nagi Mesa
  build, and adds a regression assertion for the patch contract. UEFI and
  first-web-pixel acceptance remain unexecuted.

- Public snapshot CI run #14 (`35528990869`, head `98157d1`) passed the
  previous archiver boundary: pinned source bootstrap, Servo dependency
  preflight, Mesa Softpipe archive, UEFI dependencies, M16 package artifact,
  and the Nagi kernel. The real `mozjs_sys` build then stopped in Mozilla's
  `timestamp.mozbuild` because Nagi had no platform source selected and
  reported `No TimeStamp implementation on this platform`. The next
  Nagi-owned adapter patch selects Mozilla's existing POSIX TimeStamp source
  for `OS_TARGET == "Nagi"`; its clock calls use the generated relibc/Nagi
  PAL `clock_gettime(CLOCK_MONOTONIC)` path. No host clock, loop counter, or
  synthetic rendering path is introduced. UEFI and first-web-pixel acceptance
  remain unexecuted.

- Public snapshot CI run #15 (`35529920163`, head `35e6222`) passed the Nagi
  TimeStamp platform selection and reached the next real C++ toolchain
  boundary. Mozilla's configure then failed because the freestanding wrapper
  intentionally used `-nostdinc` but no C++ standard header root was supplied;
  the first missing header was `<cstddef>`. The next repair installs the
  pinned Ubuntu noble `libc++-19-dev` headers, supplies
  `NAGI_CXX_HEADERS=/usr/include/c++/v1`, and makes the wrapper validate and
  pass that path as an explicit system include. This remains a target compile
  header dependency; host C++ runtime linking stays disabled. UEFI and
  first-web-pixel acceptance remain unexecuted.

- Run #16 (`35530635419`, head `310f698`) confirmed the libc++ package and
  explicit `NAGI_CXX_HEADERS` path, so `<cstddef>` was no longer the first
  failure. The next failure was include-order contamination: Mesa's minimal
  `tools/mesa/nagi-headers/type_traits` shadowed libc++'s real header, while
  libc++ `include_next` probes for `stdint.h` and related C headers could not
  reach the Nagi boundary cleanly. The follow-up wrapper repair keeps libc++
  first and places the Nagi Mesa/relibc compatibility headers behind it with
  `-idirafter` only when C++ headers are configured. The target build must be
  rerun; UEFI and first-web-pixel acceptance remain unexecuted.

- Public snapshot CI run #17 (`35531324244`, head `7210f01`) confirmed that
  the include-order repair reached the real MozJS C++ compile: Mesa, package,
  kernel, compiler checks, and Mozilla target configuration all passed. The
  next concrete failure was libc++'s `__config` reporting `No thread API` for
  Nagi's custom target triple. Nagi already provides the real POSIX pthread
  ABI through `nagi-posix`; the ordered `0005` mozjs adapter patch now selects
  libc++'s pthread backend for Nagi during the target build. This is a
  compile-time selection of the existing guest ABI and does not add a host
  pthread/C++ runtime. UEFI and first-web-pixel acceptance remain
  unexecuted.

- Public snapshot CI run #18 (`35532340846`, head `eb37984`) confirmed that
  libc++'s pthread backend selection removed the custom-target `No thread API`
  failure and reached libc++ locale headers. The next concrete failure was
  `unknown rune table for this platform`; Nagi's current target runtime does
  not provide a host locale database. The ordered `0006` mozjs adapter patch
  selects libc++'s portable default rune table for the Nagi target, providing
  the required header-level ctype masks without importing host locale state.
  UEFI and first-web-pixel acceptance remain unexecuted.

- Public snapshot CI run #19 (`35533256072`, head `b1d9ff5`) confirmed that
  the portable rune-table selection moved MozJS past libc++'s platform ctype
  boundary. The next target compile failure was the absence of Nagi `_l`
  locale functions (`strtoll_l`, `strtod_l`, and related APIs) required by
  libc++'s optional localization layer. MozJS already builds its pinned ICU
  path, while Nagi does not provide a host locale database or those optional
  C APIs. The ordered `0007` adapter patch therefore disables libc++
  localization for Nagi; it does not replace ICU, add host locale state, or
   fake rendering. UEFI and first-web-pixel acceptance remain unexecuted.

- Public snapshot CI run #20 (`35534135637`, head `5965075`) confirmed that
  the broad `_LIBCPP_HAS_NO_LOCALIZATION=1` workaround moved past the missing
  `_l` declarations but then removed `streamsize` and `std::ios_base` needed by
  libc++ `streambuf`. That workaround is invalid for M17. The next repair keeps
  localization enabled by applying ordered patch `0008`, adds real Nagi relibc
  `strtod`/`strtof`/`strtoll`/`strtoull` and their C/POSIX `_l` wrappers, and
  records the declarations in the generated `stdlib.h` boundary. No host libc,
  host locale state, or rendering fallback is used. UEFI and first-web-pixel
  acceptance remain unexecuted.

- Public snapshot CI run #21 (`35535949462`, head `cd3ef01`) verified that the
  Nagi numeric locale ABI and restored libc++ localization moved the real
  target compile beyond the earlier `streambuf` failure. The next concrete
  failures were Servo's `servo-fonts-traits` custom-target cfg with no
  `platform::LocalFontIdentifier`, and MozJS's target C++ headers lacking the
  Nagi ABI's existing `PROT_NONE` and `MAP_FIXED` constants. The tracked Servo
  `0006` patch selects the real pinned FreeType backend for Nagi and adds an
 explicit empty Nagi system-font registry rather than importing host
  fontconfig/DirectWrite/CoreText paths. The Mesa Nagi `sys/mman.h` overlay now
  exposes the existing Nagi libc values. UEFI and first-web-pixel acceptance
  remain unexecuted.

- Public snapshot CI run #22 (`35538673589`, head `9da3875`) passed Ubuntu and
  Windows host checks, Servo bootstrap, target feature boundary, Mesa
  Softpipe, standalone package/UEFI dependency fetch, M16 package artifact,
  and the Nagi kernel target build. User init then reached MozJS's POSIX
  thread backend and stopped because the generated Nagi `pthread.h` lacked
  `pthread_setname_np` and `pthread_getname_np`. The current repair stores a
  bounded 16-byte thread name in the guest relibc Pthread object using atomic
  bytes and exports the real APIs through cbindgen. UEFI and first-web-pixel
  acceptance remain unexecuted.

- Public snapshot CI run #23 (`35539581256`, head `fa79566`) verified the
  pthread naming ABI and again passed the target kernel boundary, then stopped
  in MozJS's allocator compile because `mozalloc.cpp` referenced
  `malloc_usable_size` without including Nagi's non-POSIX `malloc.h` header.
  Ordered MozJS patch `0009` makes the pinned Nagi relibc declaration visible
  only under `__NAGI__`; it does not add a host allocator or replace real
  allocator accounting. UEFI and first-web-pixel acceptance remain unexecuted.

- Public snapshot CI run #24 (`35540890122`, head `b76d88d`) verified the
  `malloc_usable_size` header repair and reached the next MozJS synchronization
  boundary. The pinned `ConditionVariable_posix.cpp` selected the
  macOS/Android-only `pthread_cond_timedwait_relative_np`, which Nagi does not
  expose. The next ordered MozJS patch selects the existing standard absolute
  timed-wait path with the real Nagi `CLOCK_REALTIME` condition-variable ABI.
  UEFI and first-web-pixel acceptance remain unexecuted.

- Public snapshot CI run #25 (`35542223324`, head `d56cc42`) verified the
  condition-variable clock repair and reached MozJS's mmap fault-handler
  source. `MmapFaultHandler.cpp` selected Unix signal handling and therefore
  referenced `SA_SIGINFO`, `SA_NODEFER`, and `SA_ONSTACK`, while this M17 Nagi
  vertical slice intentionally has no Unix signal-delivery ABI. Ordered MozJS
  patch `0011` selects the existing no-op mmap fault-handler macros for Nagi
  and excludes only the unsupported `sigaction`/`siglongjmp` implementation;
  it does not add a host signal API or fake memory-fault handling. UEFI and
  first-web-pixel acceptance remain unexecuted.

- Public snapshot CI run #26 (`35543941691`, head `9c01eae`) verified ordered
  MozJS patch `0011` and reached the pinned bindgen phase. Clang rejected the
  Rust-only target spelling `x86_64-unknown-nagi-user` (`version 'user' in
  target triple ... is invalid`) and could not find `<functional>` because
  bindgen did not inherit the target compiler wrapper's libc++/relibc include
  paths. The Nagi-owned MozJS build script now configures bindgen with the
  canonical freestanding `x86_64-unknown-elf` compile target, the pinned
  `NAGI_CXX_HEADERS`, generated relibc headers, Mesa header overlay, and the
  existing libc++ feature boundary. This is compile-time target configuration;
  it does not import host headers or a host runtime. UEFI and first-web-pixel
  acceptance remain unexecuted.

- Public snapshot CI run #30 (`35545417125`, head `21cf313`) passed the pinned
  Servo bootstrap, target feature boundary, Mesa Softpipe archive, package,
  kernel, and all prior target prerequisites. It then failed while compiling
  the Nagi-owned `mozjs_sys` build script with `cannot find function
  configure_nagi_bindgen in this scope`. Reproduction showed that the
  line-number-only hunk in ordered patch `0012` inserted the helper inside
  `link_static_lib_binaries` after earlier patches changed line offsets. The
  patch was corrected to use stable source context and was revalidated against
  the post-`0008` build script; the resulting helper is top-level and the call
  follows the compiler-argument loop. UEFI and first-web-pixel acceptance
  remain unexecuted.

- Public snapshot CI run #31 (`35546742142`, head `e832a26`) verified the
  corrected top-level helper placement but failed with Rust syntax errors
  (`expected one of ... found arg`, followed by `String: From<()>` and an
  extra-argument error). Reproduction showed the remaining line-number-only
  bindgen-call hunk was inserted between `builder.clang_arg(` and its `arg`
  expression. Patch `0012` now replaces the surrounding blank line with a
  stable context hunk covering the completed compiler-argument loop and the
  WASI branch. Ordered application now produces a syntactically correct
  top-level call and helper. UEFI and first-web-pixel acceptance remain
  unexecuted.

- Public snapshot CI run #32 (`35547606552`, head `b3e8bb5`) verified both
  ordered-patch placement repairs and reached the real bindgen invocation
  after the Servo/Mesa/kernel prerequisites. Clang then failed in pinned
  libc++ `<cstddef>`/`<cstdint>` because the bindgen arguments placed the clang
  resource include before `/usr/include/c++/v1`; libc++'s `include_next` could
  not reach builtin `<stddef.h>`/`<stdint.h>`. The Nagi target compiler wrapper
  already defines the correct order, so patch `0012` now matches it:
  libc++ headers, clang resource headers, then relibc/Mesa compatibility
  headers. This remains a freestanding compile-boundary repair; UEFI and
  first-web-pixel acceptance remain unexecuted.

- Public snapshot CI run #33 (`35548995494`, head `42a4597`) verified both
  ordered patch-placement repairs and the corrected libc++/clang header order;
  bindgen no longer failed in `<cstddef>` or `<cstdint>`. It then reached the
  pinned `src/jsglue.cpp` and stopped at its two `unsupported platform`
  branches for Nagi. Ordered patch `0013` now selects the existing real
  relibc `malloc.h` and `malloc_usable_size` ABI under `__NAGI__`, matching the
  allocator bridge already used by patch `0009`. It does not import a host
  allocator or weaken the first-pixel gate. UEFI and first-web-pixel
  acceptance remain unexecuted.

- Public snapshot CI run #34 (`35550551510`, head `c13d463`) passed the
  ordered bindgen header boundary and the real jsglue allocator bridge, then
  reached Servo Rust compilation. `script::dom::navigatorinfo::Platform` had
  branches for Windows, Linux/BSD, macOS, and iOS but none for
  `target_os = "nagi"`, so both the window and worker Navigator
  implementations failed to compile. Ordered Servo patch `0007` adds the
  target-owned `navigator.platform` response `Nagi`. This is a real target Web
  API boundary and does not alter rendering, substitute a host platform, or
  provide synthetic pixel evidence. UEFI and first-web-pixel acceptance remain
  unexecuted.

- Public snapshot CI run #35 (`35552534042`, head `4b585cb`) applied Servo patch
  `0007` and reached the `Build Nagi user init` step, which failed with exit
  code 101. `Build UEFI loader` and the real QEMU first-web-pixel step were not
  reached. The Ubuntu and Windows host test steps also failed because the
  MozJS mmap contract test required the literal `sigaction` string, while the
  tracked `0011` patch intentionally removes the Nagi Unix signal-handler
  implementation and therefore does not contain that string. The test now
  asserts the actual patch boundary (`MmapFaultHandler.cpp` plus the Nagi
  conditional). The target's first compiler diagnostic remains the next
  evidence to retrieve; no speculative target patch is recorded here.

- Public snapshot CI run #36 (`35554276403`, head `9377d07`) passed both host
  jobs and again failed only at `Build Nagi user init` with exit code 101. The
  target job completed the pinned Servo/Mesa bootstrap, dependency boundary,
  package, and kernel build; UEFI and QEMU first-web-pixel steps were skipped.
  The public job page exposed only the terminal annotation, so the next CI
  revision records the complete cargo output and emits its first compiler
  diagnostic as a check annotation without weakening the M17 gate.

- Public snapshot CI run #37 (`35556640788`, head `9519888`) passed Ubuntu and
  Windows host jobs and again failed at `Build Nagi user init` after the pinned
  Servo/Mesa bootstrap and kernel stages. The new target-build diagnostic
  wrapper reported `error: this file contains an unclosed delimiter`; GitHub's
  annotation did not yet include the following `--> path:line` location, so no
  source edit is inferred from this incomplete context. UEFI and real QEMU
  first-web-pixel steps were skipped.

- Public snapshot CI run #38 (`35558265319`, head `c655aad`) passed Ubuntu and
  Windows host jobs and localized the first target compiler diagnostic to
  `third_party/servo/components/script/dom/navigator/navigatorinfo.rs:84:3`.
  Replaying the ordered patch against the pinned clean source showed that
  `0007-nagi-navigator-platform.patch` declared `+60,10` while its hunk
  contained eleven resulting lines. `git apply --check` accepted the malformed
  count, but the applied file omitted the Nagi function's closing `}`. The
  hunk count is corrected to `+60,11`, and the patch-boundary test asserts the
  corrected count and final closing line. Local patch-check, clean-source
  apply-probe, and `git diff --check` pass; target build, UEFI, and real QEMU
  first-web-pixel evidence remain outstanding.

- Public snapshot CI run #39 (`35560131276`, head `5ab6b52`) passed Ubuntu and
  Windows host jobs, Servo bootstrap, Mesa Softpipe, package, kernel, and the
  corrected navigator patch compilation boundary. It then failed in the
  Nagi-owned Albert embedder at `user/nagi-albert/src/lib.rs:124:16` with
  `error[E0600]: cannot apply unary operator ! to type ()`. The pinned Servo
  `Servo::spin_event_loop` API returns `()` and handles shutdown internally;
  the adapter now calls it directly without treating it as a boolean. UEFI and
  real QEMU first-web-pixel steps were skipped and remain required.

- Public snapshot CI run #40 (`35562090985`, head `a19cb94`) passed Ubuntu and
  Windows host jobs, Servo bootstrap, Mesa Softpipe, package, kernel, and the
  corrected Albert `spin_event_loop` API contract. The target job then reached
  the real linker boundary and failed with `error: linking with rust-lld
  failed: exit status: 1`; UEFI and real QEMU first-web-pixel steps were
  skipped. The public log endpoint does not expose the linker body to this
  unauthenticated audit, so the CI wrapper now extracts the first linker symbol
  or `ld.lld` error for the next evidence pass. No linker fallback or fake
  rendering was introduced.

- Public snapshot CI run #41 (`35563574740`, head `7a9f1ba`) passed Ubuntu and
  Windows host jobs, Servo bootstrap, Mesa Softpipe, package, kernel, and the
  corrected Albert adapter. The diagnostic annotation exposed the exact target
  linker cause: `rust-lld: error: unable to find library -lstdc++`. Tracing the
  pinned `mozjs_sys` build showed that its `cc-rs` C++ `Build::compile()` path
  adds the default `stdc++` request even though the existing Nagi patch already
  suppresses MozJS's explicit link branch. Ordered patch `0003` now sets
  `cpp_link_stdlib(None)` for `nagi-user` and makes the explicit branch fail
  closed against any host `CXXSTDLIB` value. Its patch-boundary test and clean
  source apply check pass locally. This is a target-owned link-boundary repair;
   UEFI and real QEMU first-web-pixel evidence remain required.

- Public snapshot CI run #42 (`35566339373`, head `afcccae`) passed Ubuntu and
  Windows host jobs, Servo bootstrap, Mesa Softpipe, package, kernel, and the
  MozJS-specific C++ runtime suppression. The target linker still reported
  `rust-lld: error: unable to find library -lstdc++`. Source tracing found the
  same default in the pinned `fontsan`, `harfbuzz-sys`, and `glslopt` C++ build
  scripts through shared `cc-rs` 1.4.6 behavior. The current repair pins that
  exact `cc` source, applies a target-specific Nagi patch, and bootstraps it
  through the existing source hash, ordered patch, and checkout-fingerprint
  boundary. Local patched-crate compilation and bootstrap CLI check pass;
  local `cargo run ... fetch` is blocked only by the Windows host's missing
  `link.exe`. The next CI run must verify target link, UEFI, and real QEMU
  first-web-pixel evidence.

- Public snapshot CI run #43 (`35568601044`, head `dcce137`) passed Ubuntu and
  Windows host jobs, Servo/bootstrap, Mesa Softpipe, package, and kernel. The
  shared `cc-rs` patch removed the prior `-lstdc++` failure and the target job
  reached final user-init linking. `rust-lld` then reported duplicate
  `softpipe_launch_grid`, `softpipe_draw_vbo`, and `abort` symbols. The source
  audit traced the Softpipe duplicates to forcing every member of the
  aggregated Mesa archive with `+whole-archive`; `abort` is also emitted as a
  strong symbol by both relibc and the Nagi POSIX fallback. The current repair
  switches the target-owned Mesa archive to normal selective extraction and
  makes only the Nagi POSIX fallback weak. UEFI and real QEMU first-web-pixel
  steps were skipped and remain required.

- Public snapshot CI run #44 (`35571409458`, head `8f2a773`) confirmed that the
  duplicate Softpipe and `abort` symbols were gone and reached the final target
  link. The remaining failure was a target-owned ABI gap:
  `__stack_chk_guard`, `__stack_chk_fail`, and `operator delete(void*)` were
  unresolved. The current repair adds the tracked freestanding
  `tools/mesa/nagi-cxx-runtime.cpp` boundary, compiled by `user/nagi-init` for
  `x86_64-unknown-none` without host C++ headers or runtime libraries. Its
  allocation and deallocation operators call the real Nagi POSIX allocator,
  while stack-protector failure calls the target abort boundary. UEFI and real
  QEMU first-web-pixel evidence remain required.

- Public snapshot CI run #45 (`35574033249`, head `1a6ca9e`) confirmed that the
  freestanding C++ runtime shim resolved `__stack_chk_guard`,
  `__stack_chk_fail`, and `operator delete(void*)`. The target then reached the
  next real Nagi POSIX ABI boundary and reported undefined `readv`, `shutdown`,
  and `setsockopt`; UEFI and first-web-pixel steps were therefore skipped.
  Ubuntu Format also caught that the build-script formatting had not been
  included in the commit, and the Windows host job repeated the prior local
  MSVC `link.exe`/CRT boundary. The current repair implements `readv` through
  the Nagi runtime and implements socket shutdown, TCP_NODELAY, and receive/send
  timeouts through `nagi-net` and smoltcp; unsupported options fail closed with
  `ENOPROTOOPT`. The abort weak linkage is target-only so the host COFF build
  keeps its normal fallback. Target link, UEFI, and real QEMU first-web-pixel
  evidence remain required.

- Public snapshot CI run #46 (`35580032533`, head `48f87a3`) compiled the
  repaired network ABI far enough to expose a target-only `u32`/`usize`
  comparison at `user/nagi-posix/src/abi.rs:413`. This was corrected by an
  explicit target ABI-side cast in `0a07fbd`; no socket contract or acceptance
  assertion was weakened.

- Public snapshot CI run #47 (`35582239552`, head `0a07fbd`) passed bootstrap,
  Mesa, package, kernel, and target compilation, then reached final user-init
  linking. The exact linker diagnostics were undefined `pthread_equal`,
  `pthread_setname_np`, and
  `std::__1::this_thread::sleep_for(std::__1::chrono::duration<long long,
  std::__1::ratio<1l, 1000000000l> > const&)`. The current repair provides
  the first two as target-only weak Nagi thread-ABI fallbacks and defines the
  exact libc++ symbol in the Nagi-owned C++ runtime, delegating sleep to the
  real Nagi GuestClock boundary. UEFI and real QEMU first-web-pixel steps were
  skipped and remain required.

- Public snapshot CI run #48 (`35585927884`, head `4b8cb81`) passed bootstrap,
  Mesa, package, kernel, and target compilation, then reached final user-init
  linking. The exact new diagnostics were undefined `strcmp`, `atoi`, and
  `stderr`. Source tracing showed that relibc's upstream `string`, `stdlib`,
  and `stdio` modules are excluded under `target_os = "nagi"`; only the
  Nagi-owned `src/nagi.rs` backend is compiled. The current repair adds
  target-owned `strcmp` and `atoi`, plus a `FILE *stderr` object whose writes
  forward to Nagi descriptor 2 through `nagi_posix_write_fd`. This is a real
  target ABI repair, not a host libc fallback. UEFI and real QEMU first-web-
  pixel steps were skipped and remain required.

- Public snapshot CI run `35589309822` (head `670dbb8`) confirmed that the
  target relibc `strcmp`, `atoi`, and `stderr` repair reached the next final
  link boundary. The exact new diagnostics were undefined
  `pthread_cond_timedwait`, `gai_strerror`, and `ioctl`. The current repair
  adds a sequence-based condition wait using Nagi mutexes and GuestClock,
  target-owned `gai_strerror` diagnostics, and a Nagi POSIX ioctl facade that
  returns `ENOTTY` for unsupported requests without touching host devices.
  UEFI and real QEMU first-web-pixel steps were skipped and remain required.

- Public snapshot CI run `35591875406` (head `697dd08`) passed bootstrap,
  Mesa, package, and kernel, but stopped in target compilation before the
  linker with `error[E0412]: cannot find type AtomicU32` at
  `user/nagi-posix/src/abi.rs:1086`. The condition implementation imported
  `AtomicUsize` but omitted `AtomicU32`; the corrective import is now added.
  This is a source compile repair, not a target ABI or acceptance result.

- Public snapshot CI run `35593932419` (head `085d912`) passed bootstrap,
  Mesa, package, kernel, and target compilation, then reached final linking.
  The exact linker diagnostics were undefined `accept`, `getsockopt`, and
  `lstat`. The current repair adds `accept` as an explicit fail-closed
  boundary because Nagi's current network service is client-only, implements
  `getsockopt` from the real POSIX descriptor state, and routes `lstat`
  through the existing guest VFS stat path. The target-owned relibc backend
  now exports all three symbols without host libc linkage. UEFI and real QEMU
  first-web-pixel steps were skipped and remain required.

- Public snapshot CI run `35597057716` (head `17bcc29`) passed bootstrap,
  Mesa, package, kernel, and target compilation, then reached final linking.
  The exact linker diagnostics were undefined `nagi_posix_lstat`,
  `gettimeofday`, and `pow`. The current repair separates the Nagi VFS lstat
  facade from its weak C wrapper, maps `gettimeofday` to `GuestClock` realtime
  nanoseconds, and adds target-owned freestanding IEEE-aware `pow`/`powf`
  math because the upstream relibc header/math modules are excluded for the
  Nagi target. No host filesystem, host clock, or host math library is used.
  UEFI and real QEMU first-web-pixel steps were skipped and remain required.

- Public snapshot CI run `35600567895` (head `bcc978a`) passed bootstrap, Mesa,
  package, kernel, and target compilation, then reached final target linking.
  The exact next diagnostics were undefined `isatty`, `strncmp`, and
  `snprintf`. Source tracing confirmed that these are still outside the
  target-selected relibc modules. The current repair keeps ownership inside
  the Nagi target boundary: `isatty` checks the real Nagi descriptor facade,
  `strncmp` performs bounded C byte comparison, and `snprintf`/`vsnprintf`
  implement bounded C formatting for strings, characters, integers, pointers,
  floating-point values, width, precision, and variadic argument consumption.
  The formatter reports the full required length and NUL-terminates bounded
  output; it is not a symbol-only stub. Local standalone target-backend
  metadata compilation, format, CLI check, clippy, and whitespace checks pass.
  The focused host test binary remains unable to link locally because this
  Windows environment lacks MSVC `link.exe`. UEFI and real QEMU first-web-
  pixel steps remain unexecuted.

- Public snapshot CI run `35604881762` (head `0f2c05c`) passed bootstrap, Mesa,
  package, kernel, and target compilation, then reached final target linking.
  The exact next diagnostics were undefined `unlink`, `openat`, and
  `unlinkat`. The current repair connects `openat(AT_FDCWD, ...)` and
  `unlink`/`unlinkat` to the existing Nagi root VFS open/remove operations;
  unsupported dirfd-relative resolution returns `ENOTSUP`, and unsupported
  unlink flags return `EINVAL` rather than claiming success. The relibc
  target backend forwards all three symbols without host filesystem access.
  Standalone target-backend metadata compilation, format, CLI check, clippy,
  and whitespace checks pass. The host `nagi-posix` check cannot link on this
  Windows PC because MSVC `link.exe` is unavailable. UEFI and real QEMU
  first-web-pixel steps remain unexecuted.

- Public snapshot CI run `35608468009` (head `04799f3`) passed bootstrap, Mesa,
  package, kernel, and target compilation, then reached final target linking.
  The exact next diagnostics were undefined `cosf`, `sinf`, and `fdopendir`.
  The current repair adds target-owned range-reduced polynomial `sin`/`cos`
  and `sinf`/`cosf` implementations, and routes `fdopendir` through the Nagi
  POSIX directory boundary. The current root-only VFS reports `ENOTDIR` for a
  regular file and preserves the real errno for invalid descriptors rather
  than returning a fabricated directory handle. Standalone target-backend
  metadata compilation, format, CLI check, clippy, and whitespace checks pass.
  UEFI and real QEMU first-web-pixel steps remain unexecuted.

- Public snapshot CI run `35610876131` (head `45ab39c`) passed bootstrap, the
  M17 target dependency boundary, Mesa, package, kernel, and target
  compilation, then reached final target linking. The exact diagnostics were
  undefined `__memcpy_chk`, `vtable for __cxxabiv1::__si_class_type_info`, and
  `dri2_init_drawable`; UEFI and real QEMU were skipped. The next repair adds
  the real bounds-checked `__memcpy_chk` to the Nagi user ABI, passes
  `-fno-exceptions` and `-fno-rtti` to the freestanding Mesa build, and adds
  the pinned `0018-nagi-enable-dri2-frontend.patch` so the real DRI2 source is
  included in Nagi's static `libdri`. These changes remain M17-internal and do
  not claim target-link or guest acceptance until CI reruns.

- Public snapshot CI run `35615722163` (head `768405a`) passed bootstrap, the
  M17 target dependency boundary, Mesa, package, kernel, and target
  compilation, then reached final target linking. The exact diagnostics were
  undefined `__assert_fail`, `getaddrinfo`, and `fork`; UEFI and real QEMU
  were skipped. The next repair adds the target-owned `__assert_fail` abort
  path, a standard `addrinfo` result backed by numeric IPv4 parsing or the
  real Nagi DNS resolver, `freeaddrinfo`, and a fail-closed `fork` returning
  `ENOSYS` because Nagi process creation is spawn-oriented. These changes
  remain M17-internal and do not claim target-link or guest acceptance until
  CI reruns.

- Public snapshot CI run `35619764841` (head `f401b4a`) passed bootstrap, the
  M17 target dependency boundary, Mesa, package, kernel, and target
  compilation, then reached final target linking. The exact diagnostics were
  undefined `strcat`, `bsearch`, and
  `std::__1::__libcpp_verbose_abort(char const*, ...)`; UEFI and real QEMU
  were skipped. The next repair adds target-owned `strcat` and `bsearch`
  implementations and maps libc++'s verbose abort ABI to Nagi's real process
  abort boundary. These changes remain M17-internal and do not claim
  target-link or guest acceptance until CI reruns.

- Public snapshot CI run `35623171268` (head `91c5669`) passed bootstrap, the
  M17 target dependency boundary, Mesa, package, kernel, and target
  compilation, then reached final target linking. The exact diagnostics were
  undefined `std::__1::__libcpp_verbose_abort(char const*, ...)`, `waitpid`,
  and `_exit`; UEFI and real QEMU were skipped. The next repair corrects the
  libc++ ABI mangling and connects target POSIX wait/exit symbols to Nagi's
  real spawn/join and process-exit boundaries. These changes remain
  M17-internal and do not claim target-link or guest acceptance until CI
  reruns.

- Public snapshot CI run `35626833161` (head `d50f224`) passed bootstrap, the
  M17 target dependency boundary, Mesa, package, kernel, and target
  compilation, then reached final target linking. The exact diagnostics were
  undefined `vtable for __cxxabiv1::__si_class_type_info`, `dup2`, and
  `setgid`; UEFI and real QEMU were skipped. The next repair connects regular
  file `dup2` to the Nagi descriptor table and exposes the capability-owned
  `setgid` boundary as an explicit `ENOSYS` result; socket/pipe duplication
  remains fail-closed until shared descriptor ownership exists. These changes
  remain M17-internal and do not claim target-link or guest acceptance until
  CI reruns.

- Public snapshot CI run `35630408478` (head `4127b87`) passed bootstrap, the
  M17 target dependency boundary, Mesa, package, kernel, and target
  compilation, then reached final target linking. The exact diagnostic was a
  duplicate strong `dup2` symbol; UEFI and real QEMU were skipped. The next
  repair leaves the strong POSIX `dup2` definition with relibc and retains
  only the Nagi-owned `nagi_posix_dup2` adapter in `nagi-posix`. This remains
  M17-internal and does not claim target-link or guest acceptance until CI
  reruns.

- Public snapshot CI run `35632734832` (head `f0f4cad`) passed the M17 target
  dependency boundary, Mesa, package, kernel, and target compilation, then
  reached final target linking. The exact diagnostics were undefined
  `__cxa_guard_acquire`, `std::__1::locale::classic()`, and
  `std::__1::ctype<char>::id`; UEFI and QEMU were skipped. The current repair
  adds atomic Itanium guard acquire/release/abort behavior and Nagi-owned
  classic C-locale identity storage to the freestanding C++ boundary. These
  changes remain M17-internal and do not claim target-link or guest acceptance
  until CI reruns.

- Public snapshot CI run `35636554538` (head `9fcd26c`) passed the M17 target
  dependency boundary, Mesa, package, kernel, and target compilation, but
  `Build Nagi user init` failed while compiling the newly added C++ runtime;
  UEFI and QEMU were skipped. The public annotation exposed only the custom
  build-command failure, while local LLVM clang reproduced the concrete
  `alignas` placement error and a C-linkage return warning. The current repair
  fixes both at the source boundary and is verified by a local
  `x86_64-unknown-none` clang compile. These changes remain M17-internal and
  do not claim target-link or guest acceptance until CI reruns.

- Public snapshot CI run `35639577267` (head `6cc0d23`) passed the M17 target
  dependency boundary, Mesa, package, kernel, and target compilation, then
  reached final target linking. The exact diagnostics were undefined `setuid`,
  `chroot`, and `chdir`; UEFI and QEMU were skipped. The current repair adds
  relibc-owned symbols backed by Nagi POSIX adapters: the existing root is a
  valid no-op for `/`, unsupported alternate namespaces fail closed with
  `ENOTSUP`, and mutable POSIX uid changes fail closed with `ENOSYS` because
  Nagi capability identity is not a mutable uid store. These changes remain
  M17-internal and do not claim target-link or guest acceptance until CI
  reruns.

- Public snapshot CI run `35642763129` (head `5bbb66e`) passed the M17 target
  dependency boundary, Mesa, package, kernel, and target compilation, then
  reached final target linking. The exact diagnostics were undefined `setpgid`,
  `setsid`, and `signal`; UEFI and QEMU were skipped. The current repair adds
  relibc-owned symbols backed by Nagi POSIX adapters. Process groups/sessions
  fail closed with `ENOSYS`, and unsupported signal installation returns the
  real `SIG_ERR` pointer with errno because Nagi's current process ABI has no
  Unix signal delivery. These changes remain M17-internal and do not claim
  target-link or guest acceptance until CI reruns.

 - Public snapshot CI run `35646089462` (head `3e49cce`) passed the M17 target
  dependency boundary, Mesa, package, kernel, and target compilation, then
  reached final target linking. The exact diagnostics were undefined `memchr`,
  `qsort`, and `tan`; UEFI and QEMU were skipped. The current repair adds a
  target-owned byte-search loop, deterministic in-place qsort behavior without
  host allocation, and tangent derived from the existing freestanding
  range-reduced sine/cosine implementation. These changes remain M17-internal
  and do not claim target-link or guest acceptance until CI reruns.

 - Public snapshot CI run `35652406841` (head `a188f9a`) passed target
   bootstrap, dependency-boundary validation, Mesa, package, kernel, and
   compilation stages, then reached final target linking. The exact undefined
   symbols were the Itanium ABI vtables for `__cxxabiv1::__class_type_info` and
   `__cxxabiv1::__si_class_type_info`, plus `getpid`; UEFI and QEMU were
   skipped. The current repair adds Nagi-owned C++ type-info vtables and the
   kernel-published root process identity through relibc. This remains
   M17-internal and does not claim target-link or guest acceptance until CI
   reruns.

 - Public snapshot CI run `35655720144` (head `e5fbab8`) passed target
   bootstrap, dependency-boundary validation, Mesa, package, kernel, and
   compilation stages, then reached final target linking. The exact undefined
   symbols were `expf`, `pthread_rwlock_init`, and `pthread_rwlock_rdlock`;
   UEFI and QEMU were skipped. The current repair adds freestanding exp/expf
   and the target-sized atomic reader/writer lock ABI. This remains
   M17-internal and does not claim target-link or guest acceptance until CI
   reruns.

 - Public snapshot CI run `35658269625` (head `18a3442`) passed target
   bootstrap, dependency-boundary validation, Mesa, package, kernel, and
   compilation stages, then reached final target linking. The exact undefined
   symbols were `hypotf`, GNU `std::__throw_length_error(char const*)`, and
   GNU `basic_string::_M_dispose()`; UEFI and QEMU were skipped. The current
   repair adds scaled freestanding hypot/hypotf and allocator-backed GNU C++
   ABI implementations. This remains M17-internal and does not claim
   target-link or guest acceptance until CI reruns.

 - Public snapshot CI run `35661181113` (head `93107b7`) passed target
   bootstrap, dependency-boundary validation, Mesa, package, kernel, and
   compilation stages, then reached final target linking. The exact undefined
   symbols were GNU `std::nothrow`, `environ`, and `execvp`; UEFI and QEMU
   were skipped. The next repair adds the Nagi-owned nothrow object, the
   empty-start environment object, and a fail-closed execvp boundary for the
   spawn-oriented process model. This remains M17-internal and does not claim
   target-link or guest acceptance until CI reruns.

 - Public snapshot CI run `35663893779` (head `1250b41`) passed target
   bootstrap, dependency-boundary validation, Mesa, package, kernel, and
   compilation stages, then reached final target linking. The exact undefined
   symbols were `abs`, `__dynamic_cast`, and GNU
   `_Rb_tree_increment(_Rb_tree_node_base*)`; UEFI and QEMU were skipped. The
   next repair adds target-owned integer abs, bounded single-inheritance RTTI
   casting, and the real GNU tree in-order successor. This remains
   M17-internal and does not claim target-link or guest acceptance until CI
   reruns.

 - Public snapshot CI run `35666372443` (head `b1efeeb`) passed target
   bootstrap, dependency-boundary validation, Mesa, package, kernel, and
   compilation stages, then reached final target linking. The exact undefined
   symbols were `getpeername`, `bind`, and `listen`; UEFI and QEMU were
   skipped. The next repair connects `getpeername` to the real Nagi smoltcp
   client peer endpoint and keeps listener-only `bind`/`listen` fail-closed
   with `ENOSYS`, because the current user-space network service has no
   server-listener primitive. This remains M17-internal and does not claim
   target-link or guest acceptance until CI reruns.

Recent target-link repair history:

- Public CI runs `35674424033` (#78, head `73b20bd`), `35676523724` (#79,
  head `af5533f`), and `35678180409` (#80, head `798e527`) passed the target
  Mesa/package/kernel/compile stages and successively exposed the C++ RTTI and
  Mesa archive roots, the raw rust-lld group-argument boundary, then
  `__cxa_atexit`, `tanf`, and `log2`.
- Public CI run `35680421808` (#82, head `039166e`) again reached final
  linking with `__cxa_atexit`, `tanf`, and `log2`; the next repair added a
  bounded Nagi C++ destructor registry and target-owned math entrypoints.
- Public CI run `35682596273` (#83, head `0588c8b`) then reached final
  linking with `getsockname`, `dirfd`, and `pthread_detach`; the current
  repair adds real smoltcp local-endpoint reporting, root-only directory-fd
  identity, and bounded detached-thread lifecycle state. UEFI and QEMU were
  skipped in all these runs.
- Public CI run `35686392969` (#85, head `de1796d`) passed the target
  bootstrap, dependency boundary, Mesa, package, kernel, and target compile
  stages, then reached final linking with undefined `log`, `tanhf`, and
  `logf`; UEFI and QEMU were skipped. The current repair adds real
  target-owned `log`, `logf`, `tanh`, and `tanhf` implementations using the
  freestanding Nagi math core. M17 remains `BLOCKED` until the complete gate
  passes.
- Public CI run `35688435791` (#86, head `496550a`) passed the target
  bootstrap, dependency boundary, Mesa, package, kernel, and target compile
  stages, then reached final linking with undefined `_Unwind_Resume`,
  `basic_string::_M_append`, and `basic_string::find`; UEFI and QEMU were
  skipped. The current repair adds real Nagi allocator-backed GNU string
  append/find operations and a fail-closed `_Unwind_Resume` boundary for the
  exception-disabled target. M17 remains `BLOCKED` until the complete gate
  passes.
- Public CI run `35690360685` (#87, head `5f90040`) passed the target
  bootstrap, dependency boundary, Mesa, package, kernel, and target compile
  stages, then reached final linking with undefined `exp2f`, `log2f`, and
  `fread`; UEFI and QEMU were skipped. The current repair adds target-owned
  base-2 math entrypoints and forwards descriptor-backed `fread` to the real
  Nagi POSIX read boundary. M17 remains `BLOCKED` until the complete gate
  passes.
- Public CI run `35692225348` (#88, head `5625a18`) passed the target
  bootstrap, dependency boundary, Mesa, package, kernel, and target compile
  stages, then reached final linking with undefined `fprintf`, `strstr`, and
  `fopen`; UEFI and QEMU were skipped. The current repair adds real
  target-owned substring search and Nagi FILE open/format/write paths over the
  existing POSIX/VFS boundaries. M17 remains `BLOCKED` until the complete
  gate passes.
- Public CI run `35694703468` (#89, head `41de72a`) passed the target
  bootstrap, dependency boundary, Mesa, package, kernel, and target compile
  stages, then reached final linking with undefined `fseek`, `ftell`, and
  `strncpy`; UEFI and QEMU were skipped. The current repair adds real
  descriptor-backed `fseek`/`ftell` through `nagi_posix_lseek` and a bounded
  Nagi guest-memory `strncpy`. M17 remains `BLOCKED` until the complete gate
  passes.
- Public CI run `35787930674` (#90, head `04a47e6`) passed the target
  bootstrap, dependency boundary, Mesa, package, and kernel stages, then
  reached final linking with undefined `dlsym`, `pthread_once`, and `perror`;
  UEFI and QEMU were skipped. The current repair adds a fail-closed static
  `dlsym`, a guest atomic `pthread_once`, and descriptor-backed `perror`.
  M17 remains `BLOCKED` until the complete gate passes.
- Public CI run `35791007289` (#91, head `1723c49`) passed the target
  bootstrap, dependency boundary, Mesa, package, and kernel stages, then
  reached final linking with undefined `__errno_location`, `sscanf`, and
  `strdup`; UEFI and QEMU were skipped. The current repair adds the real Nagi
  errno pointer, bounded target scanning for the required Mesa formats, and
  allocator-backed `strdup`. M17 remains `BLOCKED` until the complete gate
  passes.

No host rendering, alternate browser engine, fake GL implementation, or
synthetic web pixel was introduced. See
`docs/decisions/0019-m17-servo-rendering-blocker.md` for the historical block
record and exit criteria. M18 remains `NOT STARTED` because it depends on M17.

---

# 3A. Authoritative M11 / M12 updates

The legacy table rows contain mojibake from the initial status file. The
authoritative status below supersedes those rows.

## M11 - Login / Permissions / Security

M11 is `PASS` at `744bc86`, with design/ADR at `e513d46`. Local login/session
state, role-aware Permission Broker checks, Owner-only Developer Mode, trusted
consent for trusted apps, and fail-closed denial for untrusted file and
microphone requests were implemented in user space. Both real QEMU acceptance
paths passed, including the malicious untrusted-app denial case.

## M12 - Networking (`PASS`)

M12 is `PASS` after the corrective smoltcp migration. The kernel owns only
the bounded legacy VirtIO Net queue and capability-checked raw frame ABI.
`user/nagi-net` owns protocol behavior through the pinned smoltcp 0.12.0
adapter and exposes a bounded `SocketApi` facade; no host socket or Linux
runtime path is used by the guest.

The guest obtains its address, default route, and DNS server through DHCP.
The real QEMU acceptance path performs, in order, DHCP, ICMP echo, UDP/DNS
resolution (`example.com` through the QEMU-provided resolver), ARP, TCP
handshake, and an HTTP request to the host-side fixture. Static
`10.0.2.x` configuration is not used by the guest networking code.

### M12 verification

```text
cargo test -p nagi-net --tests PASS (4 tests)
cargo test -p nagi-kernel --lib PASS (74 tests)
cargo check -p nagi-pal -p nagi-posix -p nagi-init PASS
cargo test -p nagi-cli --locked PASS (14 integration tests)
cargo run --offline -p nagi-cli -- fetch PASS
rustfmt --edition 2021 --check <changed M12 Rust files> PASS
tests/acceptance/m12_networking.ps1 PASS
C:\Program Files\Git\bin\bash.exe ./tests/acceptance/m12_networking.sh PASS
```

The acceptance scripts require ordered markers for `Nagi M12 DHCP PASS`,
`Nagi M12 ICMP PASS`, `Nagi M12 UDP/DNS PASS`, `Nagi M12 ARP PASS`, `Nagi
M12 TCP handshake PASS`, `Nagi M12 HTTP response PASS`, and `Nagi M12
acceptance PASS`. The low-level receive ABI also has a regression test proving
that the returned length excludes the 10-byte VirtIO Net header.

# 3B. Authoritative M13 status (`PASS` after corrective closure)

M13 was `PARTIAL` at `9cbc41c`. The corrective closure was then implemented
in the working tree without weakening or deleting existing tests. M13 now
passes the primary-spec requirements for the Rust PAL/POSIX/relibc and Rust
std paths, including real guest mmap/time/sleep/poll/socket-DNS/thread-TLS/
native-spawn behavior and the unified real-QEMU acceptance gate.

The kernel addition is only the low-level read-only `SYS_TIME_READ` timer
counter. Filesystem, sockets, networking, process creation, POSIX wrappers,
and AI/high-level services remain outside the kernel boundary.

### Existing M13 verification (vertical-slice evidence)

```text
cargo test --workspace --locked PASS
cargo clippy -p nagi-pal -p nagi-posix -p nagi-net --all-targets --locked -- -D warnings PASS
cargo clippy -p nagi-kernel --lib --locked -- -D warnings PASS
cargo clippy -p nagi-cli --all-targets --locked -- -D warnings PASS
cargo check --manifest-path third_party\\relibc\\Cargo.toml --target targets\\x86_64-unknown-nagi-user.json --no-default-features --locked --offline '-Zbuild-std=core,alloc' PASS
rustfmt --edition 2024 --check <changed M13 Rust files> PASS
tests/acceptance/m13_rust_posix.ps1 PASS
C:\\Program Files\\Git\\bin\\bash.exe ./tests/acceptance/m13_rust_posix.sh PASS
tests/acceptance/m13_rust_std.ps1 PASS
C:\\Program Files\\Git\\bin\\bash.exe ./tests/acceptance/m13_rust_std.sh PASS
```

The POSIX and std acceptance images build and run in QEMU; their markers are
checked in order. The std image verifies the target-specific Rust std path,
relibc linkage, allocator, real network, clock/sleep, thread/TLS,
synchronization, and VFS. The POSIX image verifies the PAL, C/POSIX ABI,
relibc symbols, mmap, time/sleep, poll, socket/DNS, thread/TLS, native spawn,
and the OSS compatibility path.

### M13 corrective closure verification

The following evidence was obtained after the corrective implementation:

```text
cargo test -p nagi-abi -p libnagi -p nagi-net -p nagi-posix -p nagi-kernel -p nagi-cli --locked PASS
cargo +nightly-2025-08-01 build -p nagi-init --features m13-posix --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --release --locked PASS
cargo +nightly-2025-08-01 build -p nagi-kernel --target targets/x86_64-unknown-nagi.json -Zbuild-std=core,compiler_builtins --release --locked PASS
rustup run nightly-2025-08-01 rustfmt --edition 2021 --check <changed M13 Rust files> PASS
tests/acceptance/m13_rust_posix.ps1 PASS
C:\Program Files\Git\bin\bash.exe ./tests/acceptance/m13_rust_posix.sh PASS
tests/acceptance/m13_rust_std.ps1 PASS
C:\Program Files\Git\bin\bash.exe ./tests/acceptance/m13_rust_std.sh PASS
tests/acceptance/m13.ps1 PASS
C:\Program Files\Git\bin\bash.exe ./tests/acceptance/m13.sh PASS
```

The authoritative unified gate runs real QEMU for both images and passed with
the repository-owned HTTP fixture. The final guest logs are
`out/logs/m13-posix.log` and `out/logs/m13-std.log`. POSIX markers include
`Nagi M13 mmap PASS`, `time/sleep PASS`, `poll PASS`, `thread/TLS PASS`,
`native spawn PASS`, `relibc C PASS`, and `OSS library PASS`; Rust std markers
include network, clock, thread/TLS, sync, and VFS. `fork()` remains a
deliberate deterministic `ENOSYS` boundary, and the native spawn bridge remains
bounded, capability-attenuating, and user-space initiated.

The host Rust build used LLVM `lld-link.exe` because the installed Windows
toolchain does not provide MSVC `link.exe`. This is a development-host linker
configuration only and is not a Nagi production-runtime dependency.

Unsupported `fork()` remains an intentional deterministic `ENOSYS` boundary.

# 3C. Architecture alignment checkpoint (documentation only)

The unified device/application architecture documentation was recorded at
`c06f4c0` in ADR 0014 and the primary specification update. It remains valid
as a forward architecture alignment. It does not alter the M13 implementation
boundary or start M14.

- one Nagi environment with a stable `NodeId`-based Device Registry;
- one logical `AppId` and continuable `AppSessionId` per application;
- separate `ExecutionInstanceId` and `SurfaceId` identities;
- Presentation Surface as the parent concept for desktop Window;
- distinct `UserId`, `NodeId`, `AppId`, `AppSessionId`,
  `ExecutionInstanceId`, `SurfaceId`, `WorkspaceId`, `ObjectId`, and
  `TransactionId` concepts;
- explicit forward rules for M15, M16, M19, M22, and M23;
- user-space-only future device routing, with explicit attenuated authority;
- no ARM/mobile hardware, cloud sync, remote transport, or distributed
  execution added to the single-node 0.1 target.

Files changed:

- `docs/decisions/0014-unified-device-application-model.md`;
- `docs/architecture/unified-device-application-model.md`;
- `docs/architecture/README.md`;
- `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`;
- this status document.

The existing M0-M13 implementation and acceptance tests were not weakened,
removed, or rewritten for terminology consistency. The passing Window Server,
kernel, IPC, storage, and networking implementations remain the Nagi 0.1
single-Node realization of the new model.

# 3D. Common Language Architecture checkpoint (documentation)

The Language Architecture is now a repository-wide common rule. The primary
specification section `3.2 Common Language Architecture`,
`docs/architecture/language-architecture.md`, ADR 0015, and the short rules
in `AGENTS.md` agree on the following:

- English is the canonical internal language;
- `en-US` and `ja-JP` are equal first-class Nagi 0.1 user languages;
- user-facing strings use shared localization resources and selected-locale
  to `en-US` fallback;
- System Language, Region/Locale, Input Language/Keyboard, and Albert/AI
  Conversation Language are separate settings;
- UTF-8 is the default internal text encoding;
- future language packs must not require an OS-wide code rewrite.

The existing M10 Japanese UTF-8 rendering/input path is compatible, but Nagi
does not yet claim a complete shared resource catalog or language settings
service. Those remain implementation work and require focused lookup,
fallback, invalid-locale, and Unicode tests. This documentation checkpoint
does not change the M13 `PASS` result and does not start M14.

Files added or updated for this checkpoint:

- `docs/architecture/language-architecture.md`;
- `docs/decisions/0015-language-architecture.md`;
- `docs/architecture/README.md`;
- `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`;
- `AGENTS.md`;
- this status document.

# 3E. Decision / Generative AI Architecture checkpoint (documentation only)

This checkpoint is recorded after M16 `PASS` and before M17 Servo Bootstrap.
It does not advance, reopen, or alter M0-M16, and it does not start M17.

The repository now defines:

- capability-centered typed `DecisionProvider` and `GenerativeProvider`
  boundaries;
- Tier 0 Deterministic Fast Path, Tier 1 Decision capability, and Tier 2
  Generative/Reasoning paths;
- bounded `DecisionRequest` / `DecisionResult` and batch-capable concepts;
- capability/role-based Model Router and Model Manager metadata;
- Jev-free local fallback through `LlmDecisionAdapter`;
- shared deterministic Validator / Policy / Permission / Executor and
  Transaction / Undo / Wayback / Activity boundaries for all AI lanes;
- `llama.cpp` / GGUF scoped to the Nagi 0.1 Generative LLM runtime.

IBM Granite 4.2 3B remains Default Standard, Qwen3 4B remains the
alternative Standard, and Gemma 3 1B remains Lite. Jev, a dedicated local
System 1 model, and a cloud DecisionProvider remain optional and are not
Nagi 0.1 dependencies or release blockers. AI-disabled ordinary OS/GUI use
and Offline-first behavior remain required.

Files added or updated for this checkpoint:

- `docs/architecture/decision-and-generative-ai-architecture.md`;
- `docs/decisions/0018-decision-and-generative-model-architecture.md`;
- `docs/architecture/README.md`;
- `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`;
- `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md`;
- `AGENTS.md`;
- this status document.

No kernel, loader, user-space runtime, Servo, package, History, IDL, Cargo,
third-party, model-download, CI-runtime, QEMU, acceptance-test, or
`crates/nagi-model` implementation was changed. A pre-existing untracked
`third_party/servo/` directory remains untouched and is not treated as M17
acceptance evidence.

# 3F. UI Design System parallel workstream

**State:** `PASS` for the independent host-side design foundation. Target UI
attachment remains deferred and blocked before UI startup by the existing M5
ELF loader; this workstream does not revise the historical M10 milestone.

The existing `user/nagi-ui` `no_std` crate now defines semantic visual tokens,
scalable typography, density/control/border/focus treatment, structured
interaction and feedback state, button/toggle/text-field/select/navigation/
dialog contracts, modal focus containment and restoration, localization-aware
min/max sizing, a first-party application-shell contract, and accessibility
roles with validated input/error relations. M10 continues to consume semantic
palette roles through its narrow adapter. The crate remains renderer-neutral
and does not execute app actions or add a system service. See
`docs/architecture/ui-design-system.md` and
`docs/workstreams/ui-design-system.md` for its full contract and evidence.

At UI commit `91465588a13e0f9a66afe9c99b75c2fb1a1c394c`, the focused suite
passed 41 tests; warning-denied Clippy, formatting, the public component
gallery, UEFI compilation, and the Nagi x86-64 user-target compilation passed.
The integration-owned workstream state records these checks. `./nagi desktop`
built the target image, then QEMU serial output stopped at M5
`Nagi M5 user address space FAIL` / `reason: invalid-elf`, before `nagi-init`
or UI startup. The existing empty `PT_TLS` header is rejected by
`kernel/src/user_elf.rs::validate_tls_segment`; kernel/loader ownership was
left unchanged. This is target attachment evidence only, not desktop rendering
or input acceptance. The unrelated M17 `BLOCKED` and M18 `NOT STARTED` states
remain unchanged. CI run `36380321571` was cancelled when the follow-up evidence
commit was pushed. Run `36380820648` is validating the source workstream branch
HEAD `9f0e58df297d02459f89cb8b42a5c228e089afff`; the integration-owned state
tracks its current host and target job status.

# 3G. First-party and shared UI host integration checkpoint (2026-09-26)

**State:** host-side integration is verified locally; target app/runtime
integration remains **NOT RUN**. This checkpoint does not change any official
milestone status or release gate.

The `codex/integration-next-phase` branch now contains the registered
`nagi-ui` shared design-system crate and first-party host adapters for Notes,
Files, Activity/Wayback, and Home/Search. The cross-app preview uses in-memory
providers and labels itself `target NOT RUN`; it does not establish target
storage, capability-service, launch, Action-dispatch, or renderer behavior.

Local verification at commit `94dbf8708af743469678cf5f662bc4a6dd892a45`
passed: `nagi-cli` (102 unit + 26 CLI tests), Activity/Wayback (41), Files
(48), Notes (27), Home/Search (35 + 1 preview), cross-app integration (11),
and `nagi-ui` (27). Root workspace and standalone first-party Clippy checks
passed with warnings denied. Formatting, DF-01 validation (18 registered
workstreams and 11 state files), M0 host acceptance, and the bilingual
memory-only integration preview passed.

First-party source CI run `36230579035` passed both host jobs; its target job
is still running the unchanged M17 first-web-pixel acceptance. Merged-root CI
run `36231902947` passed Ubuntu but failed Windows M0 PowerShell acceptance
during script-root resolution; its target job was skipped. The first repair,
commit `83faf55`, used `$PSCommandPath`, but run `36232534175` showed that the
Windows `\\?\` extended path retained `..` segments and `Test-Path` could not
resolve the launcher. Commit `9fc3f10` now derives the repository root by
walking the script directory's parent directories, removing those segments.
Run `36232966992` confirmed this path resolution but then exposed a Windows
file-lock failure: nested Cargo could not replace the running
`target/debug/nagi.exe`. Commit `3d2001c` gives the nested launcher checks an
isolated temporary `CARGO_TARGET_DIR` and restores the environment afterward.
The local M0 host run passed through the Linux shell wrapper; this macOS host
has no `pwsh`, so that does not verify the Windows script. Root run
`36233551349` passed both Ubuntu and Windows host jobs. Its retained Windows
report records M0-LAUNCHER `PASS` (exit 0); 20 filtered target cases remain
`NOT RUN`. The run's target job is now executing the real M17 first-web-pixel
acceptance. M17 remains `BLOCKED` until the
real guest produces its pixel checksum and acceptance marker; M18 remains
`NOT STARTED`. The UI-specific M10 guest preview remains blocked before UI
startup by the existing M5 `invalid-elf` / empty `PT_TLS` failure. No kernel
or loader changes were made. Nagi 0.2 runtime/product work remains gated on
M30 PASS and an explicit release checkpoint.

# 3H. Unregistered parallel branch review (2026-09-26)

Two clean, local-only branches were reviewed against the registered contracts
and current integration tree. Neither has a remote branch or GitHub Actions
run, and neither was merged or modified:

- `codex/parallel-capability-core` at `59d7e10` adds a second capability
  policy API and line-based package declaration alongside the registered
  `crates/nagi-capability` and JSON App SDK manifest contract. Its scope,
  grant lifecycle, ID vocabulary, and manifest representation are not
  compatible enough for a mechanical merge. Defer it until the canonical
  capability API is explicitly reconciled.
- `codex/parallel-wayback-ledger` at `4943fd8` adds a second activity/Wayback
  schema that overlaps the registered `crates/nagi-wayback` and
  `user/nagi-history`, changes paths forbidden by its workstream ownership,
  and exposes a revert executor without a policy/permission input. Defer the
  branch; any useful access-filtering or idempotency ideas require a later
  authorized-contract review within the registered Wayback workstream.

The active first-party and M17 diagnostic worktrees contain uncommitted
changes and were left untouched.

# 4. Current milestone detail

## M0 遯ｶ繝ｻRepository / Toolchain / CI

### Goal

Create the development foundation before substantive OS implementation.

### Required deliverables

- Nagi monorepo structure
- Cargo workspace
- `tools/nagi-cli`
- root `AGENTS.md`
- architecture/docs skeleton
- tests skeleton
- output/cache conventions
- basic CI
- top-level command skeleton:
  - `./nagi doctor`
  - `./nagi fetch`
  - `./nagi build`
  - `./nagi image`
  - `./nagi run`
  - `./nagi test`

### Acceptance criteria

`./nagi doctor` must successfully inspect/report the supported host dependencies needed at this stage, including where applicable:

- Rust
- LLVM / Clang / LLD
- QEMU
- OVMF
- CMake
- Meson
- Ninja
- Python

The implementation must establish a reproducible repository layout suitable for M1.

### Current work

Repository was initially only the three Source of Truth documents and was not a Git repository. Git was initialized on `main`. M0 is PASS at commit `5f3b5b8`: the Cargo workspace, pinned Rust channel, manifest-driven host requirements, executable identity/version probes, explicit OVMF family pairing, safe output cleanup, strict argument arity, combined Cargo diagnostics, POSIX/PowerShell launchers, acceptance scripts, CI skeleton, documentation skeleton, ADRs, and source-lock schema are verified. No kernel or guest behavior has been claimed.

### Known blockers

No active M0 blocker. Remote Ubuntu CI has not been run from this Windows workspace; the local POSIX-compatible acceptance and the specified `./nagi doctor` command have passed. CI contains the strict Ubuntu job for remote execution.

### Tests / commands last run

`cargo fmt --all -- --check` PASS  
`cargo clippy --workspace --all-targets --locked -- -D warnings` PASS  
`cargo test --workspace --locked` PASS (15 tests + doctests)
`tests/acceptance/m0_doctor.ps1` PASS (12/12 checks)  
`tests/acceptance/m0_doctor.sh` PASS (12/12 checks)  
`tests/acceptance/m0_launcher.ps1` PASS  
`tests/acceptance/m0_launcher.sh` PASS
`nagi.ps1 doctor` PASS (12/12 executed host checks)

### Next concrete action

M1 is PASS. The next milestone is M2, which must add memory management, exception handling, and interrupt foundations with its own build, test, and QEMU acceptance evidence.

## M1 - UEFI -> Kernel

### Current work

M1 guest code is implemented: `nagi-bootinfo` defines and validates the shared ABI, the custom kernel target produces a fixed-address ELF64 kernel, the Rust UEFI loader reads `\\EFI\\NAGI\\KERNEL.ELF`, validates and loads segments, captures the final UEFI memory map/GOP/ACPI data, calls `ExitBootServices`, and transfers control using the UEFI `win64` ABI. The host CLI writes a real FAT12 ESP containing `EFI/BOOT/BOOTX64.EFI` and `EFI/NAGI/KERNEL.ELF`, then starts the configured q35 QEMU reference VM.

### Acceptance criteria

QEMU serial output must contain exactly the kernel line `Nagi Kernel started`. The loader and kernel must reach this line through the real UEFI image; host output is not accepted as evidence.

### Acceptance result

`tests/acceptance/m1_qemu_boot.ps1` PASS and `tests/acceptance/m1_qemu_boot.sh` PASS. Both exercised `nagi run`, which built the real kernel and UEFI loader, booted the FAT12 ESP with OVMF/QEMU, and verified `out/logs/m1-qemu-boot.log` contains `Nagi Kernel started`.

---

## M2 - Memory / Exceptions / Interrupts

### Current work

M2 adds a bounded physical page allocator consuming only conventional UEFI
memory-map entries, a 4KiB-aligned 512-entry page-table representation,
checked page-table entries and kernel-heap primitives, a
runtime IDT with real assembly adapters, masked legacy PIC lines, and a Local
APIC periodic timer. The kernel exercises allocation/free and heap alignment,
waits for real timer interrupts, and then performs a deliberate invalid
canonical-address access. The page-fault handler reports vector 14 and the
non-present access diagnostic over the guest COM1 serial port.

### Acceptance criteria

- page allocation/free;
- expected page fault handling;
- timer interrupts;
- invalid access diagnostic.

### Acceptance result

`tests/acceptance/m2_memory_interrupts.ps1` PASS and
`tests/acceptance/m2_memory_interrupts.sh` PASS. Both exercised `nagi run`,
booted the real UEFI/FAT12/QEMU guest, and verified the serial log contained:

```text
Nagi Kernel started
Nagi M2 page allocation/free PASS
Nagi M2 timer interrupts PASS
Nagi Page fault handled (vector 14)
Nagi invalid access diagnostic PASS
Nagi M2 acceptance PASS
```

The kernel memory library's three host unit tests also pass. No active M2
blocker remains.

### Next concrete action

M2 is PASS. The next milestone is M3, which must bring all four reference CPUs
online and run scheduler test workloads.

---

## M3 - SMP / Scheduler / Threads

### Current work

M3 implements bounded four-CPU ACPI MADT discovery, malformed-table and
x2APIC rejection, MADT Local APIC address override handling, and real guest
AP startup through Nagi-owned INIT/SIPI/SIPI trampoline code. The trampoline
uses an owned low-memory bootstrap stack, a local real-mode GDT, the validated
active BSP GDT, the active CR3, and long-mode entry. AP startup is refused
unless the complete loaded kernel image, trampoline, IDT, APIC, ACPI RSDP,
per-CPU state, and all scheduler stacks are identity mapped.

The kernel now has bounded per-CPU online/work/preemption/wake/context-switch
state, two dedicated-stack kernel thread contexts per CPU, a blocked-to-runnable
wake transition, and timer-driven saved interrupt-frame switching. The timer
stub aligns its temporary Rust call stack, and APs reload the published BSP IDT
without racing to rewrite shared IDT entries.

### Acceptance criteria

All four reference CPUs must report online and run the scheduler test
workloads. The workload must exercise timer preemption, context switching, and
the blocked-to-runnable wake path using guest kernel state.

### Acceptance result

`tests/acceptance/m3_smp_scheduler.ps1` PASS and
`tests/acceptance/m3_smp_scheduler.sh` PASS. Both exercised the real
UEFI/FAT12/QEMU guest and verified all four CPU online/workload markers plus
`Nagi M3 scheduler workloads PASS` and `Nagi M3 acceptance PASS` in the guest
serial log. The guest log also retained the M2 allocation, timer, page-fault,
and invalid-access markers.

Focused ACPI/scheduler/memory tests and the full workspace test suite passed;
the final full suite contained 33 passing unit/integration tests.

### Next concrete action

M3 is PASS. M4 must add the capability handle table, rights attenuation,
generation-based stale-handle resistance, VMO/AddressSpace basics, Channel,
Event, Timer, and wait/wait_many, then prove a cross-process channel round trip
and that a transferred read-only handle cannot be strengthened to write.

---

## M4 Handles / VMO / IPC

### Goal

Establish the bounded capability, VMO, Channel, Event, Timer, and wait
substrate before introducing the first user process.

### Required deliverables

- 64-bit slot/generation handles with independent object generations and
  reference accounting.
- Process-local rights checks with attenuation-only transfer semantics and
  queue-owned escrow.
- Bounded anonymous/shared VMO backing and AddressSpace map/protect/unmap with
  VMO reference retention.
- Fixed-width Channel messages, transactional receive-time handle install,
  Event/Timer state, and source-backed bounded wait registration.
- Real QEMU guest acceptance for Process A -> Process B delivery and rejection
  of receiver WRITE escalation.

### Acceptance criteria

Process A must send a Channel message to Process B through separate process
handle tables. A transferred READ-only VMO capability must resolve in B for
READ and fail closed for WRITE. VMO mapping, Event/Timer mutation, wait-item
creation, queue-full/receiver-full behavior, generation invalidation, and
escrow cleanup must use real bounded kernel state and preserve capability
checks.

### Acceptance result

M4 is PASS at `9533c63`. The final host suite, kernel-library clippy,
UEFI kernel/loader cross-builds, and both real QEMU acceptance paths passed.
The guest serial log
contains the M2 and M3 regression markers plus:

`Nagi M4 handles/VMO/IPC START`
`Nagi M4 VMO basics PASS`
`Nagi M4 channel round-trip PASS`
`Nagi M4 rights attenuation PASS`
`Nagi M4 wait primitives PASS`
`Nagi M4 acceptance PASS`

### Next concrete action

M7 later introduced the block/filesystem/persistent-storage behavior while
preserving the M6 user-space service, M5 user/kernel, syscall, capability, and
FPU boundaries. M7 is PASS; M8 is now current.

## M5 First User Process

### Current implementation

M5 is `PASS`. Commits `48882b6` through `789b92a` add the versioned
`InitImageInfo` BootInfo contract, persistent UEFI `INIT.ELF` loading, a real
static `nagi-init` user ELF, deterministic bounded user page tables, ring-3
entry, native SYSCALL/SYSRET, bounded console/process-exit syscalls, and the
M5 CLI/acceptance harness. The kernel preserves user GPR/RCX/R11 and FPU/SIMD
state across syscall dispatch, initializes x87/MXCSR/XMM state before `IRETQ`,
and presents the user entry stack with the required ABI alignment. The user
process checks that initial state and the state after a real console syscall
round trip.

### Acceptance result

Both required real-QEMU acceptance scripts passed after the final repair:

```text
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tests\acceptance\m5_first_user_process.ps1 PASS
C:\Program Files\Git\bin\bash.exe ./tests/acceptance/m5_first_user_process.sh PASS
```

The final guest serial log contains the ordered sequence through:

```text
Nagi M5 user process START
Nagi M5 FPU state initial PASS
Hello from user space
Nagi M5 FPU state round-trip PASS
Nagi M5 syscall PASS
Nagi M5 acceptance PASS
```

The CLI now waits for `Nagi M6 acceptance PASS`; an M5-only log cannot be
reported as a successful run. The M5 scripts continue to validate the M5
sequence as a regression contract.

## M6 Init / Supervisor / Service Registry

### Current implementation

M6 is `PASS`. Commit `9e3e72b` adds the bounded `libnagi` service protocol,
generation-checked registry handles, explicit service health states, and a
deterministic dependency-ordering Supervisor with finite restart budgets.
Commit `bcd0236` integrates a real `echo@1` handler into the guest
`nagi-init`, resolves and calls it through the registry, verifies the returned
request bytes, and preserves the kernel boundary by using only the existing
console and process-exit syscalls. The terminal M6 acceptance marker is
emitted by the kernel only after the user process reaches successful exit;
there is no host filesystem/socket/process fallback or canned response.

### Acceptance result

Both required real-QEMU acceptance scripts passed:

```text
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tests\acceptance\m6_init_supervisor_registry.ps1 PASS
C:\Program Files\Git\bin\bash.exe ./tests/acceptance/m6_init_supervisor_registry.sh PASS
```

The final guest serial log contains the ordered M6 proof:

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

The M6 scripts also require the complete M0-M5 regression marker sequence.

### Next concrete action

M7 is PASS. M8 is next and must establish the specified CLI foundation while
preserving the real guest, capability, and host-output boundaries.

## M7 Block / Filesystem / Persistent Storage

### Current implementation

M7 is `PASS` at commit `65d9160`. The kernel now discovers the largest
legacy VirtIO Block device through PCI configuration space, initializes a
correct 256-entry legacy queue with the required 4 KiB used-ring alignment,
and exposes only capability-checked fixed 512-byte sector read/write
syscalls. The ring-3 bootstrap passes that capability without allowing
inline-assembly register clearing to replace it, and the bounded user stack
maps four pages for the real storage workload.

High-level storage remains in user space. `libnagi` implements a bounded
single-group ext2 volume over a `BlockDevice` trait, root directory create/open
and listing, generation-checked file handles, 1 KiB file I/O, and an explicit
bounded file-backed mapping load/flush API. The host CLI creates a separate
16 MiB raw data image only when absent, preserves an existing correctly sized
image, attaches it as a second legacy VirtIO Block device, and runs the first
write boot and second read boot against the same image. The Windows clean
command safely skips only its active target executable while still removing
repository-owned `out` outputs.

### Acceptance result

Both required real-QEMU Acceptance scripts passed on the final code:

```text
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tests\acceptance\m7_block_filesystem_persistence.ps1 PASS
C:\Program Files\Git\bin\bash.exe ./tests/acceptance/m7_block_filesystem_persistence.sh PASS
```

The first guest log `out/logs/m7-first-boot.log` proves, in order:

```text
Nagi M7 ext2 format PASS
Nagi M7 file create PASS
Nagi M7 file write PASS
Nagi M7 file-backed mmap PASS
Nagi M7 persistent write PASS
```

The second guest log `out/logs/m1-qemu-boot.log` proves, in order:

```text
Nagi M7 ext2 mount PASS
Nagi M7 directory lookup PASS
Nagi M7 file read PASS
Nagi M7 file-backed mmap PASS
Nagi M7 persistent read PASS
Nagi M5 syscall PASS
Nagi M5 acceptance PASS
Nagi M6 acceptance PASS
Nagi M7 acceptance PASS
```

The M7 scripts also require the complete M0-M6 regression sequence and a
16 MiB persistent data image. M5 and M6 regression Acceptance scripts passed
again after M7 completion.

### Next concrete action

M8 is PASS. M9 is next and must establish the first display/input surface and
first window according to the primary specification, with its own build, test,
and acceptance criteria.

---

## M8 CLI Foundation

### Current implementation

M8 is `PASS` at commit `7b1ec49`. The shared `nagi-abi` crate publishes the
bounded console-read, process-info, memory-info, and log-read ABI. The kernel
keeps the low-level boundary: console input is read from the guest COM1
device, process and memory snapshots describe the actual bootstrap process
and mappings, and log-read copies the bounded serial ring populated by real
guest output. All user buffers are checked against mapped user ranges.

The user-space `nagi-init` shell is feature-selected as `m8-shell`; it keeps
the default one-shot M5-M7 image unchanged for `nagi run`. The shell uses the
real ext2/VFS implementation for `pwd`, `ls`, `cat`, `touch`, `write`, `cp`,
`mv`, `rm`, and `mkdir`, and uses the structured diagnostic ABIs for
`nagi ps`, `nagi mem`, and `nagi log`. The bounded bootstrap image limit is
expanded from eight to sixteen pages and remains explicitly validated by the
kernel. `nagi shell` transports stdin/stdout over QEMU's serial TCP chardev;
the host does not implement or synthesize guest command results.

### Acceptance result

Both required real-QEMU M8 acceptance scripts passed:

```text
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tests\acceptance\m8_cli_foundation.ps1 PASS
C:\Program Files\Git\bin\bash.exe ./tests/acceptance/m8_cli_foundation.sh PASS
```

The guest-produced serial log contains the M0-M7 regression markers, the
real persistent filename and payload, successful `pwd`, `ls`, and `cat`
checks, and actual process, memory, and serial-log diagnostic output before
`Nagi M8 acceptance PASS`. The CLI intentionally stops QEMU after observing
that guest marker; its output records the emulator's termination status while
the acceptance gate is the verified guest marker and log contents.

### Verification result

The final focused and workspace checks passed:

```text
cargo fmt --all -- --check PASS
cargo test -p nagi-cli --locked PASS (18 tests; doc-tests 0)
cargo test --workspace --locked PASS (110 tests; doc-tests 0)
cargo clippy --workspace --all-targets --exclude nagi-kernel --exclude nagi-loader --locked -- -D warnings PASS
cargo clippy -p nagi-kernel --lib --locked -- -D warnings PASS
cargo build --workspace --exclude nagi-kernel --exclude nagi-loader --locked PASS
cargo build -p nagi-init --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --release --locked PASS
cargo build -p nagi-init --features m8-shell --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --release --locked PASS
cargo build -p nagi-kernel --target targets/x86_64-unknown-nagi.json -Zbuild-std=core,compiler_builtins --release --locked PASS
cargo build -p nagi-loader --target x86_64-unknown-uefi --release --locked PASS
target\debug\nagi.exe doctor PASS (12 pass, 0 warn, 0 fail; CMake 4.4.3)
```

### Next concrete action

M10 is PASS. M11 is next and must add local login/session roles, the
Permission Broker, trusted dialogs, and Developer Mode while preserving the
M9/M10 capability and host/guest boundaries.

---

## M9 - Display / Input / First Window

### Current implementation

M9 is `PASS` at commit `c084f70`. The kernel now provisions the real UEFI GOP
scanout supplied by the QEMU VirtIO VGA path, owns a fixed-size RGBA Surface
VMO, exposes bounded display/input capabilities and syscalls, and polls both
legacy and modern VirtIO input PCI layouts. The bootstrap user process maps
the Surface VMO read/write and starts a user-space first Window Server/compositor
that renders a movable window, tracks pointer/focus state, and accepts keyboard
input. The host `nagi gui` command uses QEMU VNC scanout and QMP only as an
input transport; it accepts only guest-produced state and PASS markers.

### Acceptance result

Both required real-QEMU M9 acceptance scripts passed:

```text
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\tests\\acceptance\\m9_display_input_window.ps1 PASS
C:\\Program Files\\Git\\bin\\bash.exe ./tests/acceptance/m9_display_input_window.sh PASS
```

The guest log proves real display setup, modern VirtIO input setup, the first
window READY marker, changed window coordinates/checksum, mouse move PASS,
focus PASS, keyboard PASS, and `Nagi M9 acceptance PASS`. The M8 PowerShell
and Git Bash acceptance scripts were rerun after M9 and remained PASS.

### Verification result

The final focused and workspace checks passed:

```text
cargo fmt --all -- --check PASS
cargo test --workspace --locked PASS (113 tests; doc-tests 0)
cargo clippy --workspace --all-targets --exclude nagi-kernel --exclude nagi-loader --locked -- -D warnings PASS
cargo clippy -p nagi-kernel --lib --locked -- -D warnings PASS
cargo build --workspace --exclude nagi-kernel --exclude nagi-loader --locked PASS
cargo build -p nagi-init --features m9-window --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --release --locked PASS
cargo build -p nagi-kernel --target targets/x86_64-unknown-nagi.json -Zbuild-std=core,compiler_builtins --release --locked PASS
cargo build -p nagi-loader --target x86_64-unknown-uefi --release --locked PASS
```

### Next concrete action

M10 is PASS. M11 is current and must deliver local login/session roles, the
Permission Broker, trusted dialogs, and Developer Mode without weakening
capability checks.

---

# M10 - Nagi UI / Desktop

### Current implementation

M10 is `PASS` at commit `87f3b25`. The user-space M10 desktop extends the M9
Surface VMO and raw input boundary without adding kernel UI syscalls. It
contains a bounded Painter/UI toolkit, a no-std bitmap Font Service, Japanese
UTF-8 glyph rendering, focus and pointer routing, and four simultaneous
application clients: Calculator, Notes, Files, and GUI Terminal. Because the
current developer-preview process model has one bootstrap user process, these
are independent user-space application objects inside the desktop compositor;
the implementation does not claim unsupported kernel process spawning.

### Acceptance result

Both required real-QEMU M10 acceptance scripts passed:

```text
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\tests\\acceptance\\m10_ui_desktop.ps1 PASS
C:\\Program Files\\Git\\bin\\bash.exe ./tests/acceptance/m10_ui_desktop.sh PASS
```

The guest-produced serial log contained the M0-M7 regression markers,
`Nagi M10 desktop READY`, a nonzero surface checksum, Calculator/Notes/Files/
GUI Terminal focus markers, Japanese input PASS, and
`Nagi M10 acceptance PASS`. QMP was used only to send real VirtIO keyboard and
mouse events; the host did not render or synthesize application results.

M8 and M9 PowerShell and Git Bash acceptance scripts were rerun after M10 and
remained PASS.

### Verification result

```text
cargo fmt --all -- --check PASS
cargo test --workspace --locked PASS (113 tests; doc-tests 0)
cargo clippy --workspace --all-targets --exclude nagi-kernel --exclude nagi-loader --locked -- -D warnings PASS
cargo test -p nagi-cli --locked PASS (14 tests; doc-tests 0)
cargo clippy -p nagi-cli --all-targets --locked -- -D warnings PASS
cargo build -p nagi-init --features m10-desktop --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --release --locked PASS
cargo build -p nagi-kernel --target targets/x86_64-unknown-nagi.json -Zbuild-std=core,compiler_builtins --release --locked PASS
cargo build -p nagi-loader --target x86_64-unknown-uefi --release --locked PASS
```

The first M10 image attempt exposed an invalid high-address large-code-model
ELF (read-only `PT_LOAD`); it was discarded. The final implementation retains
the existing small-code-model executable ELF contract and removes generated
jump-table relocations through a bounded user-space glyph table and direct
focus branches.

### Parallel native boot-visual slice (2026-09-18)

Status: `PASS` on feature branch `feature/nagi-boot-sequence`, commits
`531c9da..fdf85fd`. This slice leaves **Current milestone: M13 - Rust std /
POSIX** unchanged and does not advance the M13 status.

The native M10 boot renderer now consumes the monotonic progress bridge,
renders the supplied formal NAGI asset contract, reports the ordered platform,
core-services, storage, graphics, and session stages, performs the lock/
collapse transition, and hands off to the existing desktop only after the
final checksums. M10-only cfg guards exclude the renderer from additive M11,
M12, and M13-posix builds. The linker script now captures the large-code-model
`.ltext*`, `.lrodata*`, `.ldata*`, and `.lbss*` section families required by the
real user ELF.

The guest serial log contains these ordered boot markers:

```text
Nagi boot stage PLATFORM 15
Nagi boot stage CORE_SERVICES 30
Nagi boot stage STORAGE 50
Nagi boot stage GRAPHICS 70
Nagi boot stage SESSION 90
Nagi boot lock READY
Nagi boot collapse COMPLETE
Nagi boot frame checksum=<nonzero>
Nagi boot lock checksum=<nonzero>
Nagi M10 desktop READY
```

Verification:

```text
cargo fmt --all -- --check PASS
cargo test -p nagi-init --test boot_cfg_contract --locked PASS (2 tests)
cargo test -p libnagi --locked PASS (24 unit tests, 2 renderer tests)
cargo test -p nagi-cli --locked PASS (16 tests)
cargo clippy -p libnagi --all-targets --locked -- -D warnings PASS
cargo clippy -p nagi-cli --all-targets --locked -- -D warnings PASS
tests/acceptance/m10_ui_desktop.ps1 PASS (real QEMU)
tests/acceptance/m10_ui_desktop.sh PASS (real QEMU)
```

The two QEMU runs were executed with the duplicate nested-worktree Cargo
config temporarily suppressed and restored afterward; without that local
verification workaround, Cargo reads both the feature worktree and parent
`.cargo/config.toml` and passes the user linker script twice. The repository
files and config are clean after restoration.

### Next concrete action

M11 is current and must deliver local login/session roles, the Permission
Broker, trusted dialogs, and Developer Mode while preserving capability checks
and the rule that AI cannot elevate itself.

---

# 5. Last successful verification

Record the most recent known-good commands here.

```text
`cargo fmt --all -- --check` PASS  
`cargo clippy --workspace --all-targets --exclude nagi-kernel --exclude nagi-loader --locked -- -D warnings` PASS
`cargo clippy -p nagi-kernel --lib --locked -- -D warnings` PASS
`cargo build --workspace --exclude nagi-kernel --exclude nagi-loader --locked` PASS
`cargo test --workspace --locked` PASS (110 tests; doc-tests 0) after final M8 shell changes
`cargo build -p nagi-init --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --release --locked` PASS
`cargo build -p nagi-kernel --target targets/x86_64-unknown-nagi.json -Zbuild-std=core,compiler_builtins --release` PASS after final M5 changes
`cargo build -p nagi-loader --target x86_64-unknown-uefi --release --locked` PASS after final M5 changes
`powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\tests\\acceptance\\m0_doctor.ps1` PASS  
`powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\\tests\\acceptance\\m0_launcher.ps1` PASS
`tests/acceptance/m0_doctor.sh` PASS
`tests/acceptance/m0_launcher.sh` PASS
`cargo build -p nagi-kernel --target targets/x86_64-unknown-nagi.json -Zbuild-std=core,compiler_builtins --release` PASS
`cargo build -p nagi-loader --target x86_64-unknown-uefi --release` PASS
`tests/acceptance/m1_qemu_boot.ps1` PASS
`tests/acceptance/m1_qemu_boot.sh` PASS
`tests/acceptance/m2_memory_interrupts.ps1` PASS
`tests/acceptance/m2_memory_interrupts.sh` PASS
`tests/acceptance/m3_smp_scheduler.ps1` PASS
`tests/acceptance/m3_smp_scheduler.sh` PASS
`tests/acceptance/m4_handles_vmo_ipc.ps1` PASS
`tests/acceptance/m4_handles_vmo_ipc.sh` PASS
`target\debug\nagi.exe doctor` PASS (12 pass, 0 warn, 0 fail; CMake 4.4.3)
`powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tests\acceptance\m5_first_user_process.ps1` PASS
`C:\Program Files\Git\bin\bash.exe ./tests/acceptance/m5_first_user_process.sh` PASS
`powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tests\acceptance\m6_init_supervisor_registry.ps1` PASS
`C:\Program Files\Git\bin\bash.exe ./tests/acceptance/m6_init_supervisor_registry.sh` PASS
`cargo fmt --all -- --check` PASS after M7 final code
`cargo test --workspace --locked` PASS (108 tests; doc-tests 0)
`cargo clippy --workspace --exclude nagi-kernel --exclude nagi-loader --locked -- -D warnings` PASS
`cargo clippy -p nagi-kernel --lib --locked -- -D warnings` PASS
`cargo build --workspace --exclude nagi-kernel --exclude nagi-loader --locked` PASS
`cargo build -p nagi-init --target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,compiler_builtins --release --locked` PASS
`cargo build -p nagi-kernel --target targets/x86_64-unknown-nagi.json -Zbuild-std=core,compiler_builtins --release --locked` PASS
`cargo build -p nagi-loader --target x86_64-unknown-uefi --release --locked` PASS
`target\debug\nagi.exe doctor` PASS (12 pass, 0 warn, 0 fail; CMake 4.4.3)
`powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tests\acceptance\m7_block_filesystem_persistence.ps1` PASS
`C:\Program Files\Git\bin\bash.exe ./tests/acceptance/m7_block_filesystem_persistence.sh` PASS
`powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\tests\acceptance\m8_cli_foundation.ps1` PASS
`C:\Program Files\Git\bin\bash.exe ./tests/acceptance/m8_cli_foundation.sh` PASS
```

When updating, prefer concrete evidence such as:

```text
./nagi doctor        PASS
./nagi build minimal PASS
./nagi test unit     PASS
QEMU smoke boot      PASS
```

---

# 6. Active blockers

No blockers recorded.

When blocked, use this format:

## BLOCKER-XXX 遯ｶ繝ｻShort title

**Milestone:** Mxx  
**Status:** OPEN  
**Observed failure:**  
**Command/test:**  
**Important error:**  
**Suspected root cause:**  
**Attempts already made:**  
1. ...
2. ...
**Next recommended experiment:**  
**Can other non-dependent work continue safely?:** Yes/No

Do not remove blocker history merely because the issue is resolved. Mark it `RESOLVED` and note the fix.

---

# 7. Decisions made during implementation

Record only implementation-level decisions that are not already fixed by the main specification.

Use:

## YYYY-MM-DD 遯ｶ繝ｻDecision title

**Milestone:**  
**Decision:**  
**Reason:**  
**Alternatives considered:**  
**Consequences:**  
**ADR required:** Yes/No

If a decision changes architecture, create/update an ADR under `docs/decisions/`.

---

# 8. Temporary stubs / technical debt

Every temporary stub must be listed here.

Current list:

None.

Use:

| ID | Milestone | Location | Temporary behavior | Removal condition |
|---|---|---|---|---|

A stub must never be used to falsely satisfy the milestone acceptance criteria.

---

# 9. Third-party revisions

Record exact pinned revisions once introduced.

| Component | Revision / Version | Nagi patch state | Notes |
|---|---|---|---|
| Servo | `b820a9679a784877f91b4acc90c2c6e849f18d3b` | Source pin recorded; M17 guest bootstrap/first web pixel not accepted | Browser engine |
| Mesa | `f1f246cfda65eff82fba3be1caf2d23bdeda60cc` | Nagi static Softpipe patch stack `0001`-`0017`; build not yet accepted | Softpipe path |
| Surfman | `205778f497327c573929c7b471194390e15f331d` | Nagi static EGL/surfaceless patch `0001`; guest not yet accepted | Servo rendering adapter |
| libc (Servo) | `0.2.189`, sha256 pinned in `sources.lock` | Nagi target patch `0001`; guest not yet accepted | Servo dependency |
| relibc | `69bb008af1f6d93758631cf0df250500d53a065b` | Nagi backend present; Mesa C headers/archive not yet accepted | Initial POSIX libc candidate |
| cc (cc-rs) | `1.4.6`, sha256 pinned in `sources.lock` | Nagi target patch `0001`; target C++ objects remain target-built without host runtime inference | Shared C/C++ build boundary |
| llama.cpp | `c85b92c69c955961621193cd51da194f3cbcedf3` | Pinned in `sources.lock`; Nagi-owned patches `0001`–`0034` cover bounded GGUF parsing, checked loader/runtime status, and Nagi static backend initialization. LLVM 19/libc++ no-exceptions `llama`/ggml CPU archives pass QEMU backend initialization; they are not yet connected to Model Manager and no model inference is verified. | Generative LLM runtime; Decision and Embedding providers are not fixed to it |
| whisper.cpp | `927cfce34f31707e17f2bff35c349632fb9e2c3a` | Clean raw source pin plus Nagi-owned no-exception patch; generated CPU-only `whisper` target builds, no STT provider | STT |
| smoltcp | Not pinned yet | 遯ｶ繝ｻ| Network stack |

Model artifact hashes/revisions are recorded separately in
`third_party/models.lock`; this metadata does not implement model fetching.

---

# 10. AI model baseline

Nagi 0.1 model roles are currently fixed as:

| Role | Model | Status |
|---|---|---|
| Default Standard | IBM Granite 4.2 3B | Fixed |
| Alternative Standard | Qwen3 4B | Fixed |
| Lite | Gemma 3 1B | Fixed |
| Embedding | multilingual-e5-small candidate | To validate |
| STT | Whisper small multilingual via whisper.cpp | Baseline |
| TTS | Replaceable engine | Porting spike required |

Granite should remain the default Standard LLM unless a documented technical blocker or explicit user decision changes it.

The AI architecture checkpoint adds provider-neutral capability metadata and
keeps Decision capability optional in 0.1. No dedicated local System 1 model,
Jev integration, or cloud DecisionProvider is a 0.1 dependency. Where a Decision
capability is needed before a specialized provider exists, the specified local
`LlmDecisionAdapter` path may reuse a GenerativeProvider; all outputs remain
subject to deterministic validation, policy, permission, execution, and
Activity/Transaction boundaries.

---

# 11. Reference machine

All 0.1 acceptance decisions use:

```text
QEMU
x86-64
UEFI / OVMF
q35
4 vCPU
8 GB RAM
~64 GB virtual disk

VirtIO Block
VirtIO Network
VirtIO GPU
VirtIO Sound
VirtIO RNG
Keyboard / Mouse
```

Do not make physical hardware support a hidden requirement for current milestones.

---

# 12. Resume procedure

Whenever a new Codex session starts, or context appears incomplete:

1. Read `AGENTS.md`.
2. Read the relevant sections of `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`.
3. Read this entire status file.
4. Run `git status`.
5. Inspect `git diff`.
6. Inspect recent commits if available.
7. Inspect the implementation and tests for the current milestone.
8. Re-run the smallest useful last-known verification.
9. Continue from the earliest incomplete acceptance criterion.

Do not ask the user to restate progress that can be reconstructed from the repository.

---

# 13. Status update requirements

After significant work, update at minimum:

- Current milestone
- Milestone status
- Completed work
- Remaining work
- Tests/commands run
- Blockers
- Next concrete action

When a milestone becomes `PASS`:

1. record the exact evidence;
2. update the milestone table;
3. set the next milestone as Current;
4. summarize what the next milestone must accomplish.

---

# 14. Handoff summary

At the end of a work session, leave a short handoff here.

## Latest handoff

M0-M13 are `PASS` at their recorded evidence. M13's corrective closure passed
the focused host tests, target builds, and unified PowerShell/Git Bash real-QEMU
gate for the POSIX/relibc and Rust std images. M14 passed focused host tests,
target builds, and both PowerShell/Git Bash real-QEMU acceptance wrappers for
real VirtIO Sound playback/capture through the Windows `dsound` backend,
non-zero capture validation, mixer, volume/mute, session behavior, and
capability-denial checks.
M15 is PASS: the real guest exercised the complete bounded history flow and
persisted the serialized ledger. M16 is now PASS: the host-built out-of-tree
NAPP artifact was packaged into `.xapp`, staged into the init image, installed
and launched from guest VFS, updated through VFS replace, and removed. The
IDL generator, Rust/C SDK checks, Ed25519 signature tamper rejection, focused
host suite, target build, and both M16 acceptance wrappers passed. M17 is next.
Before resuming M17, the 2026-09-19 Architecture Alignment Checkpoint
completed the documentation-only Decision/Generative provider alignment.
Decision capability is typed and optional; Jev is not a dependency; Granite
remains Default Standard; `llama.cpp` / GGUF is scoped to the Generative LLM
runtime; and M17 remains NOT STARTED. The checkpoint did not modify code,
runtime, Cargo, third_party, tests, or guest behavior.
M10 PASS at `87f3b25`: the
real QEMU guest rendered four bounded user-space GUI clients, routed actual
VirtIO mouse and keyboard events through the M9 capability boundary, rendered
Japanese text, and passed both M10 acceptance paths. M8 and M9 regression
acceptance paths also remained PASS. M17 Servo Bootstrap is the active
milestone. Public CI run `35666372443` at `b1efeeb` passed bootstrap, Mesa,
package, kernel, and target compilation, then final linking exposed
`getpeername`, `bind`, and `listen` after the preceding integer/GNU container
ABI repair. The next target-owned network ABI repair must be pushed and
verified; the required next evidence remains target link, UEFI, real QEMU, and
a real guest-rendered first web pixel. M18 cannot start before formal M17
PASS.

### Current M17 continuation (2026-09-22)

The current pushed implementation head before this handoff update is
`43a3e0c` on `main`. CI run #84 (`35684235079`) passed target bootstrap,
dependency validation, Mesa Softpipe, package, kernel, and target compilation,
then failed final target linking on `getsockname`, `dirfd`, and
`pthread_detach`. The next repair is target-owned and remains within M17:
smoltcp local endpoint reporting, root-only directory descriptor identity, and
bounded detached-thread stack lifecycle. M18 remains `NOT STARTED` and no
M17 PASS is recorded. Local Windows host Cargo linking remains limited by the
missing MSVC `link.exe`/CRT; Ubuntu target CI is the authoritative compile,
UEFI, and QEMU environment.

### Current M17 continuation after CI run #85 (2026-09-22)

The pushed implementation head is `de1796d` on `main`. CI run #85
(`35686392969`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, kernel, and target compilation, then failed final target linking on
`log`, `tanhf`, and `logf`. UEFI and real QEMU first-web-pixel acceptance were
skipped. The next repair is target-owned and remains within M17: freestanding
`log`/`logf` based on the existing Nagi logarithm reduction and stable
`tanh`/`tanhf` based on the existing Nagi exponential implementation. The
source contract now covers these symbols. M18 remains `NOT STARTED`; no M17
PASS is recorded. Local Windows Cargo test linking remains limited by the
missing MSVC `link.exe`/CRT; target CI remains authoritative for target,
UEFI, and QEMU validation.

### Current M17 continuation after CI run #86 (2026-09-22)

The pushed implementation head is `496550a` on `main`. CI run #86
(`35688435791`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, kernel, and target compilation, then failed final target linking on
`_Unwind_Resume`, GNU `basic_string::_M_append`, and GNU `basic_string::find`.
UEFI and real QEMU first-web-pixel acceptance were skipped. The next repair
is target-owned and remains within M17: implement the concrete GNU C++11 string
operations over the existing Nagi allocator/layout, and terminate through the
real Nagi abort boundary if an incompatible exception path attempts to resume.
Local Windows lacks `clang++` as well as MSVC `link.exe`/CRT, so C++ compile
validation remains delegated to the Ubuntu target CI. M18 remains
`NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #87 (2026-09-22)

The pushed implementation head is `5f90040` on `main`. CI run #87
(`35690360685`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, kernel, and target compilation, then failed final target linking on
`exp2f`, `log2f`, and `fread`. UEFI and real QEMU first-web-pixel acceptance
were skipped. The next repair is target-owned and remains within M17:
freestanding `exp2`/`exp2f` and `log2f` over the existing Nagi math core, plus
descriptor-backed `fread` through `nagi_posix_read`; memory output streams
remain explicitly non-readable rather than claiming data. M18 remains
`NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #88 (2026-09-22)

The pushed implementation head is `5625a18` on `main`. CI run #88
(`35692225348`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, kernel, and target compilation, then failed final target linking on
`fprintf`, `strstr`, and `fopen`. UEFI and real QEMU first-web-pixel
acceptance were skipped. The next repair is target-owned and remains within
M17: implement `strstr` over guest memory, map `fopen` modes to Nagi POSIX/VFS
descriptors, and format bounded `fprintf` output through the existing Nagi
formatter and FILE write boundary. M18 remains `NOT STARTED`; no M17 PASS is
recorded.

### Current M17 continuation after CI run #115 (2026-09-23)

The pushed implementation head was `2a6c59e` on `main`. CI run #115
(`35833047798`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, kernel, compilation, and the prior ctype/timezone repairs, then
reached final target linking with the remaining undefined symbols `strcasecmp`,
`isalnum`, and `strcspn`. UEFI and real QEMU first-web-pixel acceptance were
not reached. The next repair adds locale-independent ASCII string/ctype
operations over guest memory. M18 remains `NOT STARTED`; no M17 PASS is
recorded.

### Current M17 continuation after CI run #89 (2026-09-23)

The pushed implementation head was `41de72a` on `main`. CI run #89
(`35694703468`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, kernel, and target compilation, then failed final target linking on
`fseek`, `ftell`, and `strncpy`. UEFI and real QEMU first-web-pixel
acceptance were skipped. The next repair is target-owned and remains within
M17: forward FILE cursor movement and position queries to the real Nagi VFS
descriptor runtime, and implement POSIX bounded string copy in guest memory.
M18 remains `NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #112 (2026-09-23)

The pushed implementation head was `c58ecda` on `main`. CI run #112
(`35825148240`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, kernel, compilation, and the preceding `feof`/`fgets`/`stdout`
repair, then reached final target linking with the remaining undefined symbols
`lround`, `atof`, and `puts`. UEFI and real QEMU first-web-pixel acceptance
were not reached. The next repair adds `lround` over Nagi's target rounding
core, `atof` over target `strtod`, and `puts` over descriptor-1 stdout. M18
remains `NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #113 (2026-09-23)

The pushed implementation head was `240e501` on `main`. CI run #113
(`35827739822`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, kernel, compilation, and the prior stdio/numeric repairs, then
reached final target linking with the remaining undefined symbols `access`,
`setvbuf`, and `shmget`. UEFI and real QEMU first-web-pixel acceptance were
not reached. The next repair adds VFS-backed `access`, explicit Nagi
unbuffered `setvbuf`, and fail-closed `shmget` because SysV IPC is not part of
the M17 surfaceless path. M18 remains `NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #114 (2026-09-23)

The pushed implementation head was `acca2e9` on `main`. CI run #114
(`35830580426`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, kernel, compilation, and the prior VFS/stdio/IPC repairs, then
reached final target linking with the remaining undefined symbols `isdigit`,
`tzset`, and `timezone`. UEFI and real QEMU first-web-pixel acceptance were
not reached. The next repair adds locale-independent `isdigit` and the Nagi
UTC `tzset`/`timezone` ABI. M18 remains `NOT STARTED`; no M17 PASS is
recorded.

### Current M17 continuation after CI run #116 (2026-09-23)

The pushed implementation head was `bea57d7` on `main`. Public CI run #116
(`35835203783`) passed Servo bootstrap, dependency validation, Mesa Softpipe,
package, kernel, and target compilation, then reached final target linking.
The exact remaining undefined symbols were `shmat`, `shmctl`, and `shmdt`;
UEFI and real QEMU first-web-pixel acceptance were skipped. The next repair
adds explicit target-owned fail-closed SysV shared-memory ABI entries that
return `ENOSYS`, because Nagi 0.1 does not expose a guest shared-memory
mapping service for the M17 surfaceless Softpipe path. No host pointer,
synthetic mapping, or fake success is introduced. M18 remains `NOT STARTED`;
no M17 PASS is recorded.

### Current M17 continuation after CI run #118 (2026-09-23)

The pushed implementation head was `4ebf10d` on `main`. Public CI run #118
(`35841735587`) passed Servo bootstrap, dependency validation, Mesa Softpipe,
package, kernel, and target compilation, then reached final target linking.
The exact remaining undefined symbols were `fputs`, `isspace`, and `sync`;
UEFI and real QEMU first-web-pixel acceptance were skipped. The next repair
adds `fputs` over the target-owned FILE boundary, locale-independent ASCII
`isspace`, and a truthful `sync` completion contract because Nagi VFS writes
are committed through the service boundary before returning. No host stdio,
host locale, or host filesystem flush is introduced. M18 remains
`NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #119 (2026-09-23)

The pushed implementation head was `cbb114e` on `main`. Public CI run #119
(`35844150510`) passed Servo bootstrap, dependency validation, Mesa Softpipe,
package, kernel, and target compilation, then reached final target linking.
The exact remaining undefined symbols were `fputc`, `sw_screen_create_vk`, and
`null_sw_create`; UEFI and real QEMU first-web-pixel acceptance were skipped.
The next repair adds target FILE `fputc` and makes the pinned Mesa build
explicitly materialize `libpipe_loader_static.a` and `libws_null.a`, whose
upstream targets are `build_by_default=false`. No Mesa function is replaced
with a stub or fake renderer. M18 remains `NOT STARTED`; no M17 PASS is
recorded.

### Current M17 continuation after CI run #120 (2026-09-23)

The pushed implementation head was `87cc024` on `main`. Public CI run #120
(`35851514326`) passed Servo bootstrap, dependency validation, Mesa Softpipe,
package, kernel, and target compilation, then reached final target linking.
The exact remaining undefined symbols were `sw_screen_create_vk`,
`wrapper_sw_winsys_wrap_pipe_screen`, `null_sw_create`, and `strspn`; UEFI and
real QEMU first-web-pixel acceptance were skipped. The explicit Mesa targets
were built, but the single aggregate archive scan did not extract providers
that occur after their users. The next repair seeds the three real Softpipe
loader/winsys symbols through Cargo's target link arguments and implements
guest-memory `strspn` in the Nagi relibc ABI. M17 remains `BLOCKED`; M18
remains `NOT STARTED`.

### Current M17 continuation after CI run #126 (2026-09-23)

Public CI run `35858894366` (#126, head `4d90f2b`) passed Servo bootstrap and
then failed in the Mesa Softpipe step because the Meson graph had no
`libpipe_loader_nagi_roots.a` output target. The new helper definition was
correct, but `src/gallium/targets/pipe-loader` is normally configured only
for clover/tests, both disabled by the M17 configuration. The next repair
adds `with_platform_nagi` to that existing subdirectory condition, preserving
the pinned Mesa source and patch boundary. Target build, UEFI, and real QEMU
first-web-pixel acceptance were not reached. M17 remains `BLOCKED`; M18
remains `NOT STARTED`.

### Current M17 continuation after CI target Mesa helper compile (2026-09-23)

The latest target CI run for commit `208387f` registered the Nagi helper
archive but failed compiling its real Mesa `sw_helper.h` source. Clang
reported conflicting `pipe_screen_config` types because the new translation
unit did not include Mesa's defining `pipe/p_screen.h` before the helper
header, causing C prototype-scope tags. The next repair adds that standard
Mesa header before `sw_helper.h`; no rendering or ABI stub is introduced.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #122 (2026-09-23)

Public CI run `35857472389` (#122, head `63bb9b8`) failed during the pinned
Servo bootstrap before dependency-boundary, Mesa, target build, UEFI, or real
QEMU acceptance. The public check exposed only exit code 4, so no source or
linker conclusion is drawn from this run. The next repair preserves the
bootstrap failure and writes `out/logs/m17-bootstrap.log`, with a bounded
first-error annotation, so the exact pinned-source, patch-order, fingerprint,
or Servo Cargo-fetch failure can be corrected from evidence. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #138 (2026-09-24)

Public CI run `35904323947` (#138, head `7bc9e6a`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. The real target user-init link still reported MozJS
`JS::NewArrayBufferWithContents(JSContext*, unsigned long,
std::unique_ptr<void, JS::FreePolicy>)`, GNU basic_string
`_M_construct(unsigned long, char)`, and `sincosf`; UEFI and real QEMU
first-web-pixel acceptance were skipped. The next repair adds an exact
selective jsglue extraction anchor for the tracked target-only ownership
wrapper, real allocator-backed GNU string construction, and the Nagi sin/cos
math implementation's `sincosf` ABI. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Current M17 continuation after CI run #139 (2026-09-24)

Public CI run `35909004970` (#139, head `44afc1e`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and kernel
compilation. The target user-init link resolved the MozJS ArrayBuffer wrapper,
GNU basic_string `_M_construct(unsigned long, char)`, and `sincosf`, then
reported `__isnormal`, `__isnormalf`, and `frexp`. UEFI and real QEMU
first-web-pixel acceptance were skipped. The next repair adds Nagi-owned
IEEE-bit-level normal predicates and frexp/frexpf decomposition; M17 remains
`BLOCKED` and M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #123 (2026-09-23)

Public CI run `35858275718` (#123, head `68f07cb`) confirmed the bootstrap
diagnostic: the new Mesa static-helper patch was rejected as a corrupt patch
at line 70 because its added hunk counts did not match the actual additions.
No Mesa, target build, UEFI, or real QEMU acceptance ran. The patch hunk
counts are now corrected and the added hunk was checked against the generated
Mesa source without altering that checkout. M17 remains `BLOCKED`; M18
remains `NOT STARTED`.

### Current M17 continuation after CI run #121 (2026-09-23)

Public CI run `35854076101` (#121, head `3d0286b`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive, package, kernel, and target
compilation. The link-root repair resolved `null_sw_create` and `strspn`, but
the final target link still reported `sw_screen_create_vk`,
`wrapper_sw_winsys_wrap_pipe_screen`, and `strndup`. UEFI and real QEMU
first-web-pixel acceptance were skipped. The next repair adds a pinned Mesa
Nagi static helper target for the real `sw_helper.h` Softpipe implementation,
explicitly materializes the upstream `libwsw.a` wrapper winsys target, and
adds target-owned guest allocator `strndup`. M17 remains `BLOCKED`; M18
remains `NOT STARTED`.

### Current M17 continuation after CI run #117 (2026-09-23)

The pushed implementation head was `32c1313` on `main`. Public CI run #117
(`35838340415`) passed Servo bootstrap, dependency validation, Mesa Softpipe,
package, kernel, and target compilation, then reached final target linking.
The exact remaining undefined symbols were `strerror`, `time`, and `srand`;
UEFI and real QEMU first-web-pixel acceptance were skipped. The next repair
adds Nagi-owned errno text, `time` forwarding to the existing guest realtime
clock ABI, and target-local seeded `rand`/`srand` state. No host time, host
libc error table, or host random source is introduced. M18 remains
`NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #90 (2026-09-23)

The pushed implementation head was `04a47e6` on `main`. CI run #90
(`35787930674`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, and kernel compilation, then failed final target linking on `dlsym`,
`pthread_once`, and `perror`. UEFI and real QEMU first-web-pixel acceptance
were skipped. The next repair is target-owned and remains within M17: keep
dynamic symbol lookup fail-closed under the static Nagi target contract,
implement the four-byte guest atomic once ABI, and write `perror` through the
real Nagi stderr descriptor. M18 remains `NOT STARTED`; no M17 PASS is
recorded.

### Current M17 continuation after CI run #91 (2026-09-23)

The pushed implementation head was `1723c49` on `main`. CI run #91
(`35791007289`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, and kernel compilation, then failed final target linking on
`__errno_location`, `sscanf`, and `strdup`. UEFI and real QEMU first-web-pixel
acceptance were skipped. The next repair is target-owned and remains within
M17: expose the existing Nagi errno slot, parse the bounded integer/string
formats used by the pinned Mesa target, and allocate duplicate strings through
the Nagi allocator. M18 remains `NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #92 (2026-09-23)

The pushed implementation head was `b53d990` on `main`. CI run #92
(`35793927424`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, and kernel compilation, then failed final target linking on
`std::__throw_bad_array_new_length()`, `__cxa_begin_catch`, and
`__cxa_rethrow`. UEFI and real QEMU first-web-pixel acceptance were skipped.
The next repair is target-owned and remains within M17: terminate through the
real Nagi abort boundary if the exceptions-disabled target reaches these
retained C++ exception entrypoints, without importing host libc++abi or an
unwinder. M18 remains `NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #93 (2026-09-23)

The pushed implementation head was `43ee78e` on `main`. CI run #93
(`35795642028`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, and kernel compilation, then failed final target linking on
`__cxa_pure_virtual`, `geteuid`, and `getuid`. UEFI and real QEMU first-web-
pixel acceptance were skipped. The next repair is target-owned and remains
within M17: add the Nagi C++ pure-virtual abort boundary and expose the
capability-scoped root as the explicit POSIX compatibility uid 0 view through
nagi-posix and relibc. Local relibc object compilation and formatting passed;
local cargo tests remain unavailable because the generated `third_party/cc-nagi`
checkout lacks its `Cargo.toml`. M18 remains `NOT STARTED`; no M17 PASS is
recorded.

### Current M17 continuation after CI run #94 (2026-09-23)

The pushed implementation head was `9f14d21` on `main`. CI run #94
(`35797964401`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, and kernel compilation, then failed final target linking on
`std::__throw_bad_alloc()`, `__cxa_end_catch`, and `_mesa_glthread_finish`.
UEFI and real QEMU first-web-pixel acceptance were skipped. The next repair is
target-owned and remains within M17: terminate retained bad-allocation and
exception-end paths through Nagi's real abort boundary, and root the real
pinned Mesa `_mesa_glthread_finish` object through a non-executing Nagi link
anchor rather than substituting a rendering stub. M18 remains `NOT STARTED`;
no M17 PASS is recorded.

### Current M17 continuation after CI run #109 (2026-09-23)

The pushed implementation head was `6296567` on `main`. CI run #109
(`35820755188`) passed the Mesa GLSL archive and math-predicate repairs, then
reached target linking with the remaining undefined symbols `lroundf`,
`llround`, and `sprintf`. The next repair adds target-owned nearest-away-from-
zero rounding and unbounded-format C ABI entrypoints over the existing Nagi
formatter; no host libm or host stdio is introduced. UEFI and real QEMU
first-web-pixel acceptance were not reached. M18 remains `NOT STARTED`; no
M17 PASS is recorded.

### Current M17 continuation after CI run #110 (2026-09-23)

The pushed implementation head was `9073c44` on `main`. CI run #110
(`35823065041`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, kernel, and compilation stages, then reached final target linking
with the remaining undefined symbols `feof`, `fgets`, and `stdout`. UEFI and
real QEMU first-web-pixel acceptance were not reached. The next repair adds a
real descriptor-1 Nagi `stdout` object plus descriptor-backed `fgets` and EOF
state reporting through `feof`; no host stdio or synthetic stream is used.
M18 remains `NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #108 (2026-09-23)

The pushed implementation head was `228551c` on `main`. CI run #108
(`35818125262`) passed Mesa Softpipe and the prior `libgallium.a`/`lrintf`
repair, then reached target linking with the remaining undefined symbols
`link_util_parse_program_resource_name`, `isnan`, and `__isnanf`. The first is
from Mesa's real GLSL linker archive, whose `libglsl.a` target was not yet
explicitly built; the latter two are missing target math predicates. The next
repair explicitly builds `libglsl.a` and adds target-owned `isnan`/`__isnanf`.
UEFI and real QEMU first-web-pixel acceptance were not reached. M18 remains
`NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #107 (2026-09-23)

The pushed implementation head was `54ed6d3` on `main`. CI run #107
(`35815283540`) passed Servo bootstrap and the pinned Mesa Softpipe archive,
then reached final Nagi user-init target linking. The remaining undefined
symbols were `lrintf`, `u_surface_default_template`, and `pp_init`.
`u_surface_default_template` and `pp_init` are real Mesa Gallium auxiliary
objects whose `libgallium.a` target was not part of the default Nagi graph;
`lrintf` was a missing target-owned relibc C ABI export. The next repair
explicitly builds `libgallium.a` and adds target-owned `lrintf`. UEFI and real
QEMU first-web-pixel acceptance were not reached. M18 remains `NOT STARTED`;
no M17 PASS is recorded.

### Current M17 continuation after CI run #130 (2026-09-23)

Public CI run `35871531770` (#130, head `4c726ce`) passed Servo bootstrap,
Mesa Softpipe archive construction, package, kernel compilation, and the
repaired target-owned relibc exit/math ABI. It then reached the real target
link and failed on `std::_Rb_tree_insert_and_rebalance`,
`std::_Rb_tree_decrement`, and `__popcountdi2`. The next repair adds real
Nagi-owned GNU red-black tree insertion/predecessor operations and the target
popcount ABI; it does not import host libstdc++ or compiler-rt. UEFI and real
QEMU first-web-pixel acceptance remain pending. M17 remains `BLOCKED`; M18
remains `NOT STARTED`.

### Current M17 continuation after CI run #131 (2026-09-24)

Public CI run `35876355907` (#131, head `7a698f3`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The real target user-init build reached final linking but
failed on `__fprintf_chk`, `__vfprintf_chk`, and the still-unresolved
`std::_Rb_tree_insert_and_rebalance(...)`; UEFI and real QEMU first-web-pixel
acceptance were skipped. The next repair adds the fortified stdio entrypoints
through the existing bounded Nagi `vfprintf` path and corrects the GNU ABI
mangled length from `_ZSt27` to `_ZSt29`. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Current M17 continuation after CI run #132 (2026-09-24)

Public CI run `35879561669` (#132, head `79f2bf1`) resolved the fortified
stdio symbols and the correctly mangled GNU tree insertion symbol, then
reached the next real target-link set: const `_Rb_tree_increment`,
`_Rb_tree_rebalance_for_erase`, and GNU basic_string `_M_create`. The next
repair implements the real const iterator operations, GNU deletion
rebalancing/header maintenance, and Nagi allocator-backed string capacity
creation. UEFI and real QEMU first-web-pixel evidence remain pending. M17
remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #133 (2026-09-24)

Public CI run `35884059558` (#133, head `659a76a`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The real target user-init build resolved the const GNU
tree iterator, erase/rebalance, and `_M_create` symbols, then failed on
`strnlen`, `div`, and GNU basic_string `_M_replace`; UEFI and real QEMU
first-web-pixel acceptance were skipped. The next repair adds bounded
guest-memory `strnlen`, the C `div_t` ABI, and real allocator-backed string
replacement. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #134 (2026-09-24)

Public CI run `35887795498` (#134, head `4665387`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The real target user-init build resolved `strnlen`,
`div`, and GNU basic_string `_M_replace`, then failed on `syslog`, `openlog`,
and `std::__detail::_Prime_rehash_policy::_M_need_rehash(...)`; UEFI and real
QEMU first-web-pixel acceptance were skipped. The next repair adds
descriptor-backed guest logging and the Nagi-owned GNU rehash ABI. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #135 (2026-09-24)

Public CI run `35892368804` (#135, head `8e039d2`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The real target user-init link resolved `syslog`,
`openlog`, and GNU `_Prime_rehash_policy::_M_need_rehash`, then failed on
fortified `__memset_chk`, `__memmove_chk`, and GNU basic_string
`resize(unsigned long, char)`. UEFI and real QEMU first-web-pixel acceptance
were skipped. The next repair adds bounded guest-memory fortified operations
and allocator-backed GNU string resize. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Current M17 continuation after CI run #136 (2026-09-24)

Public CI run `35896205811` (#136, head `0a31126`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The real target user-init link resolved fortified
`__memset_chk`, `__memmove_chk`, and GNU basic_string `resize(unsigned long,
char)`, then failed on `fabsl`, GNU `__throw_out_of_range_fmt`, and
basic_string `_M_replace_aux(unsigned long, unsigned long, unsigned long,
char)`. UEFI and real QEMU first-web-pixel acceptance were skipped. The next
repair adds an x86-64 long-double ABI implementation, a fail-closed GNU throw
entrypoint, and allocator-backed character replacement. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #137 (2026-09-24)

Public CI run `35899807167` (#137, head `7e1cd53`) passed Servo bootstrap,
dependency validation, Mesa Softpipe archive construction, package, and
kernel compilation. The real target user-init link resolved `fabsl`, GNU
`__throw_out_of_range_fmt`, and basic_string `_M_replace_aux(...)`, then
failed on `JS::NewArrayBufferWithContents(JSContext*, unsigned long,
std::unique_ptr<void, JS::FreePolicy>)`. UEFI and real QEMU first-web-pixel
acceptance were skipped. The next repair adds target-only patch `0014` to
restore the real MozJS UniquePtr ownership wrapper through the existing
four-argument ArrayBuffer API. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

### Current M17 continuation after CI run #106 (2026-09-23)

The pushed implementation head was `61556cd` on `main`. CI run #106
(`35814387308`) produced `libmesa.a`, but neither the archive-level
`llvm-nm` scan nor the generated archive member scan exposed
`_mesa_glthread_finish`. The next repair retains the archive path and adds a
fallback scan of the actual `.o` files emitted by the same Meson target,
placing only the real defining object into the roots archive. Target link,
UEFI, and real QEMU first-web-pixel acceptance were not reached. M18 remains
`NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #104 (2026-09-23)

The pushed implementation head was `3f3bcde` on `main`. CI run #104
(`35813251531`) reached the real `src/mesa/libmesa.a` compile and exposed a
Nagi Mesa dependency-graph defect: `glspirv.c` could not find generated
`compiler/spirv/spirv_info.h`. The next repair adds the generated header as a
source of the existing `idep_vtn` dependency through numbered Mesa patch
`0019`; it does not add a host header or replace SPIR-V compilation. Target
link, UEFI, and real QEMU first-web-pixel acceptance were not reached. M18
remains `NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #105 (2026-09-23)

The pushed implementation head was `9bb8bf0` on `main`. CI run #105
(`35813827384`) passed the generated SPIR-V header repair and produced the
real `libmesa.a` archive, but the archive scan did not find
`_mesa_glthread_finish`; the candidate list now includes `libmesa.a`. The
scan used `llvm-nm -g`, which can exclude Mesa's hidden-visibility symbols.
The next repair scans all defined symbols while preserving exact member
extraction. Target link, UEFI, and real QEMU first-web-pixel acceptance were
not reached. M18 remains `NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #95 (2026-09-23)

The pushed implementation head was `d9db493` on `main`. CI run #95
(`35801744073`) passed target bootstrap, dependency validation, Mesa Softpipe,
package, and kernel compilation, then failed final target linking on
`_mesa_glthread_finish`, `printf`, and `getegid`. UEFI and real QEMU
first-web-pixel acceptance were skipped. The next repair is target-owned and
remains within M17: identify the real Mesa object defining `_mesa_glthread_finish`
with the pinned LLVM toolchain and link it through a dedicated archive before
the aggregate Mesa archive, write printf output through Nagi descriptor 1, and
expose the capability-scoped root's explicit POSIX gid 0 view. M18 remains
`NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #96 (2026-09-23)

The pushed implementation head was `96d830d` on `main`. CI run #96
(`35804263561`) passed Servo bootstrap but failed during the pinned Mesa
Softpipe archive build while discovering the real `_mesa_glthread_finish`
member. Target link, UEFI, and real QEMU first-web-pixel acceptance were not
reached. The next repair keeps the object extraction target-owned and
symbol-aware: locate the defining archive, extract its exact member with the
pinned LLVM archiver, and link that real object before the aggregate Mesa
archive. M18 remains `NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #97 (2026-09-23)

The pushed implementation head was `934f230` on `main`. CI run #97
(`35805304582`) passed Servo bootstrap but failed again during the pinned Mesa
Softpipe archive build while discovering `_mesa_glthread_finish`. Target link,
UEFI, and real QEMU first-web-pixel acceptance were not reached. The next
repair selects the Meson-produced `libmesa.a` directly, skips only malformed
archive members, and emits explicit GitHub annotations if the archive or exact
member cannot be found. M18 remains `NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #98 (2026-09-23)

The pushed implementation head was `f004387` on `main`. CI run #98
(`35805855427`) passed Servo bootstrap but failed during the pinned Mesa
Softpipe archive build because the generated output did not contain the fixed
`libmesa.a` filename. Target link, UEFI, and real QEMU first-web-pixel
acceptance were not reached. The next repair removes that filename assumption,
searches every generated target archive by defined `_mesa_glthread_finish`, and
extracts the exact member with LLVM tools. M18 remains `NOT STARTED`; no M17
PASS is recorded.

### Current M17 continuation after CI run #99 (2026-09-23)

The pushed implementation head was `61f9247` on `main`. CI run #99
(`35806844547`) passed Servo bootstrap but failed during the pinned Mesa
Softpipe archive build because the default generated archive set contained no
definition of `_mesa_glthread_finish` (`libglapi.a`, `libdri.a`, `libEGL.a`,
and related archives were present). Target link, UEFI, and real QEMU
first-web-pixel acceptance were not reached. The next repair explicitly
builds the `libmesa.a` target discovered from Ninja's target graph, then
repeats symbol-aware extraction. M18 remains `NOT STARTED`; no M17 PASS is
recorded.

### Current M17 continuation after CI run #100 (2026-09-23)

The pushed implementation head was `409f15d` on `main`. CI run #100
(`35809154991`) passed Servo bootstrap but failed during the pinned Mesa
Softpipe archive build while selecting the explicit Mesa core target. Target
link, UEFI, and real QEMU first-web-pixel acceptance were not reached. The
next repair accepts all Ninja target names containing `libmesa.a`, including
Meson `.p` output-layout forms, then repeats the real archive extraction.
M18 remains `NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #101 (2026-09-23)

The pushed implementation head was `4c4c92e` on `main`. CI run #101
(`35809747498`) passed Servo bootstrap, then the Mesa Softpipe archive step
completed without finding `_mesa_glthread_finish`; the generated candidates
were `libglapi.a`, `libmesa_sse41.a`, `libdri.a`, `libswdri.a`,
`libsoftpipe.a`, and related archives. Target link, UEFI, and real QEMU
first-web-pixel acceptance were not reached. The preceding target matcher
could select an object under Meson's `libmesa.a.p` directory instead of the
archive output itself. The next repair restricts the Ninja selection to a
target whose final path component is `libmesa.a`, then repeats real target
archive extraction. M18 remains `NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #102 (2026-09-23)

The pushed implementation head was `4694c6c` on `main`. CI run #102
(`35810511071`) passed Servo bootstrap but failed in the Mesa Softpipe step
after the stricter archive-target selection was applied; target link, UEFI,
and real QEMU first-web-pixel acceptance were not reached. The public job
summary exposed only `Process completed with exit code 1`, so the next repair
adds target-name and captured Ninja stderr-tail annotations around the real
`libmesa.a` build. M18 remains `NOT STARTED`; no M17 PASS is recorded.

### Current M17 continuation after CI run #103 (2026-09-23)

The pushed implementation head was `e6cbde1` on `main`. CI run #103
(`35812210395`) selected the real `src/mesa/libmesa.a` target and entered
its 256-object compile, but the Mesa core target failed before archive
creation. The public annotation retained only the warning tail and
`ninja: build stopped`, so the next repair prioritizes compiler `error:` and
`fatal error:` lines in the bounded annotation. Target link, UEFI, and real
QEMU first-web-pixel acceptance were not reached. M18 remains `NOT STARTED`;
no M17 PASS is recorded.

### Current M17 continuation after CI run #255 (2026-09-26)

Public CI run #255 (`36241271287`, head
`d0a7c3c4de713606b673f45b7de044f48f7eed81`) passed the Ubuntu and Windows
host jobs, Mesa Softpipe build, target dependency checks, package build, kernel
build, real `nagi-init` link, and UEFI loader build. The M17 QEMU acceptance
then timed out after 120 seconds and returned exit code 4. The last serial
marker was `Servo::new JS engine setup started`; no guest checksum or first
web-pixel PASS marker was produced.

The complete failure output contained 435 M17 trace lines. Its 256-line
excerpt omitted 179 interior lines while retaining the head and tail; the
separate final-64-line serial tail also ended at the JavaScript-engine marker.
The excerpt cap therefore did not hide later guest activity. The ServoMedia
worker entered and returned, and the MemoryProfiler worker entered,
initialized, and yielded back to the main thread. No later thread event,
panic, or kernel rejection appeared. This narrows the stall to synchronous
Servo `script::init()` but does not establish the exact failing operation.

Source inspection shows `script::init()` proceeds through proxy handlers,
generated binding statics, memory-reporter setup, and `JSEngineSetup::default()`,
which calls SpiderMonkey `JS_Init`. `JS_Init` synchronously initializes the
GC memory subsystem and JIT. GC initialization probes the target address range;
JIT initialization can request random bytes and reserve executable memory.
These are diagnostic hypotheses, not confirmed causes. New reproducible,
Nagi-only trace patches `third_party/servo-patches/0013` and
`third_party/mozjs-sys-nagi-patches/0014` bracket those stages through the
existing guest console callback and preserve all initialization operations
and ordering. Host source-contract checks cover the new markers. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`. The next public target CI run must still
produce the real guest-rendered checksum and PASS marker.

### Current M17 continuation after CI run #256 (2026-09-26)

Public CI run #256 (`36245650368`, head
`e8522ecf95de389e387d178dc0331de39cc159d4`) passed both host jobs, the
target dependency boundary, Mesa Softpipe build, M16 package, kernel, real
`nagi-init` link, and UEFI loader. The real QEMU first-web-pixel acceptance
then timed out after 120 seconds with exit code 4; no checksum or PASS marker
was produced.

The new guest trace narrowed the stall. Servo `script::init()` completed its
JIT choice, proxy handlers, generated bindings, memory reporter, and platform
initialization. SpiderMonkey `JS_Init` completed process, TLS, allocator, and
mutex setup, then entered GC address-limit search and did not return from
`FindAddressLimitInner`. The serial log contains no RNG error diagnostic.

Source inspection identifies the missing target entropy route. In
`mozjs/mfbt/RandomNum.cpp`, SpiderMonkey's Unix provider uses Linux
`getrandom` only when `__linux__` is defined; otherwise it opens
`/dev/urandom`. Nagi is a custom target, and its POSIX `open` routes into the
guest VFS, which has no `/dev/urandom` device. `Memory.cpp`'s
`GetNumberInRange` retries while `RandomUint64()` returns `Nothing`, so this
provider failure explains the observed synchronous loop. Nagi already has a
real guest VirtIO RNG syscall and `libnagi` C ABI for Rust std, but SpiderMonkey
was not using it.

The target adapter adds `__nagi_random_fill` as the shared `libnagi` C ABI,
keeps `__nagi_std_random_fill` as a delegating compatibility entry point, and
adds tracked MozJS patch `0015-nagi-virtio-rng.patch` to route only the Nagi
build through that VirtIO RNG boundary. Other OS providers stay unchanged;
entropy failure is propagated without host or deterministic fallback.
The new `nagi-cli` source-contract test passes. Local validation also passes
all 76 `nagi-cli` library tests, the focused package formatting check, the
patch application check against the pinned MozJS checkout, and C++ signature
syntax checking. Public CI still needs to verify the full target link and real
guest RNG/render path. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #257 (2026-09-26)

Public CI run #257 (`36249263091`, head
`3648b760d7bd4b1623dd9232461bbfe6b07c69dd`) passed host CI, the target
dependency boundary, Mesa Softpipe, M16 package, kernel, real `nagi-init`
link, and UEFI loader. Real QEMU first-web-pixel acceptance timed out after
120 seconds with exit code 4. The trace completed SpiderMonkey GC address
discovery and GC memory setup, then stopped at the
`SpiderMonkey Wasm initialization started` marker. No `SYS_RANDOM_GET` rejection or VirtIO RNG failure
was logged; the #256 entropy loop is no longer the observed stopping point.
No pixel checksum or PASS marker was produced.

Pinned-source inspection shows `JS_Init` next enters `js::wasm::Init()`, which
checks the system page size, configures huge memory, allocates the Wasm
code-block map, initializes static types and built-in module functions,
publishes the map, and creates static tag types. The exact blocked operation is
not yet known. Tracked patch
`third_party/mozjs-sys-nagi-patches/0016-nagi-m17-wasm-init-traces.patch`
adds Nagi-only trace checkpoints around these operations without changing
initialization behavior. The new source-contract test passes locally, as do the
focused package format check and patch reverse-check against the materialized
MozJS checkout. Public target CI must compile the patch and reveal the last
completed Wasm phase. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #258 (2026-09-26)

Public CI run #258 (`36252263959`, head
`aba6b3847c2c4b66842628552af3e8cdf2d3d8ac`) passed host CI, target dependency
checks, Mesa Softpipe, M16 package, kernel, real `nagi-init` link, and UEFI
loader. The M17 QEMU acceptance failed because QEMU did not exit within 120
seconds (exit code 4). The new trace completed Wasm page-size lookup,
huge-memory configuration, and code-block-map allocation, then stopped after
`SpiderMonkey Wasm static type definitions initialization started`. No guest
RNG error was logged; no checksum or pixel PASS marker was produced.

Pinned source shows `StaticTypeDefs::init()` begins with TypeContext allocation,
then creates its first array type and exception tag. Type creation reaches the
canonical recursion-group set and its exclusive lock. The exact blocked
operation is not yet known. Tracked patch
`third_party/mozjs-sys-nagi-patches/0017-nagi-m17-wasm-static-type-traces.patch`
adds Nagi-only checkpoints around these operations without changing their
behavior. The local source-contract test now passes after first failing because
the patch was absent; the focused format check and patch reverse-check pass.
Public target CI must verify compilation and expose the last completed marker.
M17 remains `BLOCKED`; M18 remains `NOT STARTED`.


### Current M17 continuation after CI run #259 (2026-09-26)

Public CI run #259 (`36255926378`, head
`28954b4d0546a77f3b21df5d8066aee51c26288e`) passed both host jobs, target
dependency checks, Mesa Softpipe, the M16 package, kernel, real `nagi-init`
link, and UEFI loader. The M17 QEMU acceptance failed because QEMU did not
exit within 120 seconds (exit code 4). The new static-type trace completed
TypeContext allocation and acquired the canonical type-set lock, then stopped
at `SpiderMonkey Wasm canonical type-set insertion started`. The more precise
trace places the stall inside `TypeIdSet::insert`; no first-web-pixel checksum
or PASS marker was produced.

Pinned source shows that `insert` calls `HashSet::lookupForAdd` before
`HashSet::add`. The first path hashes the one-type mutable-I16 array group; if
the static set is still empty, the add path allocates its initial table through
`SystemAllocPolicy`, which reaches Nagi's malloc/heap-lock path. This is a
candidate only; CI #259 did not distinguish hashing, lookup, and table
allocation. Patch `0017-nagi-m17-wasm-static-type-traces.patch` now brackets
the recursion-group hash, `lookupForAdd`, and `HashSet::add` separately. The
next public target run must identify which operation fails before any runtime
change is selected. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #260 (2026-09-27)

Public CI run #260 (`36259126957`, head
`cc1d9956c5abe76d2b10f9206585978c57a1131b`) passed both host jobs, target
dependency checks, Mesa Softpipe, the M16 package, kernel, real `nagi-init`
link, and UEFI loader. QEMU again did not exit within the 120-second M17
acceptance bound (exit code 4), so no guest checksum or first-web-pixel PASS
marker was produced.

The new markers completed recursion-group hashing and `lookupForAdd`, then
stopped after `HashSet::add` started. This narrows the stall to the add path;
it does not yet prove the allocator is responsible. For an empty set, the
pinned `HashTable` implementation creates its initial table through the
allocation policy's `pod_malloc`. Patch `0017` now uses a TypeIdSet-local
`SystemAllocPolicy` wrapper to trace immediately before and after that call.
This preserves the base allocation implementation and adds no global malloc
tracing. The next target run must establish whether `pod_malloc` is entered
and returns before choosing a runtime repair. M17 remains `BLOCKED`; M18
remains `NOT STARTED`.

### Current M17 continuation after CI run #261 (2026-09-27)

Public CI run #261 (`36262571944`, head
`e14ed151a91cda8597083465c45d94ebbbc1f0a6`) passed the Ubuntu and Windows
host jobs, target dependency checks, Mesa Softpipe, M16 package, and kernel.
`Build Nagi user init` failed while compiling the updated MozJS patch, before
the user-init link, UEFI loader, or QEMU acceptance ran. Clang reported
`expected member name or ';' after declaration specifiers` at the canonical
type-set lock trace in `WasmTypeDef.cpp`.

The allocator wrapper added ten source lines before a zero-context insertion
hunk. Its fixed output line left the lock-start trace at class scope after
`TypeIdSet::clearRecGroup`, which caused the C++ error. Patch `0017` now
replaces the actual lock declaration with a context-anchored hunk and places
the trace immediately before that declaration. The source-contract test also
checks this ordering. The focused tests and a new target CI run must confirm
the patch compiles before QEMU can provide the allocator markers. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #264 (2026-09-27)

Public CI run #264 (`36272680212`, head
`6b128ac29da04badfe60ed0bf0d6452f09f75a45`) passed both host jobs, the M17
target dependency boundary, Mesa Softpipe, M16 package, kernel, real
`nagi-init` link, and UEFI loader build. The M17 QEMU first-web-pixel
acceptance timed out and returned exit code 4; neither boot produced a
checksum or first-web-pixel PASS marker.

Both guest traces completed table allocation, slot initialization,
`createTable`, `changeTableSize`, primary-index calculation, and primary
slot construction. They stopped after `primary liveness read started`, with
no live/free result. The next Nagi-only checkpoint logs the index, capacity,
table base, and key-hash address before the load, then logs the raw hash after
one load and evaluates the existing `Slot::isLiveHash` predicate on that
value. This distinguishes an invalid slot address from an unexpected key hash
without reading memory twice or changing the hash-table branch. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

The local `nagi-cli` library suite passes all 77 tests, including the source
contract for the new trace and a guard for the fixed-size address-message
buffers. Rust formatting and `git diff --check` pass. The ordered MozJS patch
series 0001–0017 applies to the cached registry archive after its SHA-256 is
verified against `sources.lock` (`28adaa4255fd0d42133b993ff81d391df41de1d718777f2e3f0aee5ba8636f10`).
The repository `./nagi fetch` bootstrap could not link locally: its linker
invoked `xcrun` as x86_64 while the installed Command Line Tools provide
arm64/arm64e `libxcrun`. Thus this host has not compiled the updated C++ patch;
the next target CI must verify that compile and emit the address/hash trace.

### Current M17 continuation after CI run #263 (2026-09-27)

Public CI run #263 (`36267970162`, head
`9f00e9dedfa47514f9645d57426b7a3b42257e80`) passed target dependency
checks, Mesa Softpipe, the M16 package, kernel, real `nagi-init` link, and
UEFI loader build. The M17 QEMU first-web-pixel acceptance timed out and
returned exit code 4. Neither acceptance boot produced a checksum or
first-web-pixel PASS marker.

Both serial traces completed TypeIdSet table allocation, all slot
initialization, `createTable`, and `changeTableSize`, then stopped
immediately after `findNonLiveSlot started`. The pinned HashTable source
shows that a freshly initialized table should return from its first
`slot.isLive()` check. Patch `0017` now adds Nagi-only checkpoints after
primary-index computation, after primary-slot construction, around the first
liveness read and result, and through the first collision probe if the primary
slot is unexpectedly live. The result of each `isLive()` read is stored once
and drives the same branch as before; tracing is compiled only for the
TypeIdSet allocation policy. This run does not yet identify whether the stop is
in index calculation, slot construction, liveness access, or the trace
callback itself. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #262 (2026-09-27)

Public CI run #262 (`36264391751`, head
`0f65867429f49ea999455668d19e03be72ec4f96`) passed both host jobs, target
dependency checks, Mesa Softpipe, the M16 package, kernel, real `nagi-init`
link, and UEFI loader. The QEMU M17 acceptance then timed out and returned
exit code 4, with no checksum or first-web-pixel PASS marker.

In both acceptance boots, recursion-group hashing and `lookupForAdd`
completed, `HashSet::add` began, and the TypeIdSet-specific `pod_malloc`
started and completed. The allocator therefore returns; the stall is later in
the add path. Patch `0017` now adds optional HashTable trace hooks that compile
to no calls for policies without the TypeIdSet hooks. For this set, they
bracket slot initialization, `createTable`, `changeTableSize`,
`findNonLiveSlot`, and `setLive`, so the next target run can isolate the first
non-returning stage. Non-Nagi builds retain the original `SystemAllocPolicy`,
and other HashTable instantiations compile the hooks away. No hash-table
behavior is altered. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### Current M17 continuation after CI run #265 (2026-09-27)

Public CI run #265 (`36276918474`, head
`aba9ec447ceb9a6674d3c234b3d8ab620167138c`) passed both host jobs, target
dependencies, Mesa Softpipe, the M16 package, kernel, real `nagi-init` link,
and UEFI loader. The M17 QEMU acceptance timed out after 120 seconds (exit
code 4), without a first-web-pixel checksum or PASS marker.

The guest completed HashSet table creation and primary-slot calculation, then
stopped at the primary slot's key-hash load. The trace reported capacity
`1`, index `0x6fb3b68c`, table base `0x0000400020d53db0`, and key-hash address
`0x00004001dfa417e0`. The address difference is exactly `index * 4`, placing
the read far outside a one-entry table. A startup audit found that the custom
Nagi `_start` bypasses the generic relibc CRT startup, while the user linker
script did not retain or expose constructor arrays. The kernel hands control
directly to the ELF entry and does not run user constructors. This establishes
a missing process-initialization step; the invalid hash-table state makes it a
likely cause, but the next guest run must verify that constructor dispatch
resolves the failure.

The user linker now retains sorted `.preinit_array` and `.init_array` input
sections and exports hidden array bounds. Nagi user `_start` walks preinit
constructors before init constructors, before entering the capability-aware
application body. The M17 guest trace reports constructor completion before
`user entry reached`; the shell acceptance asserts that ordering. The
source-contract regression test was first observed failing before the fix and
passes after it.

Local verification on 2026-09-27:

- All 78 `nagi-cli` library tests passed using the installed stable compiler.
- An LLD `--gc-sections` smoke link retained the constructor arrays and their
  hidden bounds; the output placed preinit before init and sorted init
  priorities `00050`, `00100`, and `00200` before the default-priority entry.
- Focused Rust formatting, `sh -n` for the M17 acceptance script, and
  `git diff --check` passed.
- The full target `nagi-init` link and authoritative QEMU acceptance still
  require public Ubuntu CI. M17 remains `BLOCKED`; M18 remains
  `NOT STARTED`.

### Nagi TLS roots and Servo resource startup after CI run #266 (2026-09-27)

Public CI run #266 (`36281382815`, head
`fdc1b24610fa36ee9651283eaf77f7477820fc9f`) verified the constructor repair.
The guest completed `ELF constructors completed`, `user entry reached`, M7
persistent-storage acceptance, SpiderMonkey TypeIdSet insertion, and
`JS_Init`. `Servo::new` then started resource threads, but the ResourceManager
thread panicked while initializing `rustls-platform-verifier`:
`No CA certificates were loaded from the system`. No real first-web-pixel
checksum or PASS marker was produced.

The target has no host OS certificate store, and the guest must not read one.
ADR 0030 records the bootstrap policy: Nagi uses Servo's existing Rustls
WebPKI verifier with the lock-pinned `webpki-roots` 1.0.9 public root set.
Certificate-chain and hostname verification stay enabled; Servo's explicit
certificate override remains additive. No enterprise or user-root UI is
claimed by this bootstrap choice. Tracked Servo patch 0014 makes only the Nagi
target selection and comment; all other targets retain the existing verifier
selection.

The regression test was run red before patch 0014 existed and now passes. All
79 `nagi-cli` library tests pass, and the patch applies cleanly to the pinned
generated Servo checkout. Formatting and diff checks pass after the final
formatting correction. Public Ubuntu CI must verify resource-thread creation
and the next guest stage. M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

### M17 bundled-resource registration after CI run #287 (2026-09-27)

Public CI run #287 (`36292384786`, head
`63b2cc504e8c0ffb0f27581dd506970ef0577da7`) passed the Ubuntu and Windows host
jobs, the target dependency boundary, Mesa Softpipe, M16 package, kernel,
real user-init link, and UEFI loader. QEMU advanced beyond the 32-slot worker
pool, completed Servo storage startup and constellation creation, then
panicked during TLS prewarm with `No resource reader registered`. No real
first-web-pixel checksum or PASS marker was produced.

The pinned target graph contains `servo-default-resources` and its 11 embedded
resource files. Servo's `DefaultResourceReader` registers through
`inventory::submit!`; the locked `inventory 0.3.24` macro assigned that
constructor to `.init_array` for known ELF operating systems but omitted
Nagi's custom `target_os`. Nagi already retains and executes `.init_array`
before the application entry, so ADR 0032 records the narrow correction:
vendor the exact locked crate, add Nagi to its ELF constructor list, and read
Servo's real embedded domain list before constructing Servo. No resource
bytes, host paths, or rendering behavior are substituted.

Local verification on 2026-09-27:

- The vendored source archive hash matches Cargo.lock and `sources.lock`:
  `a4f0c30c76f2f4ccee3fe55a2435f691ca00c0e4bd87abe4f4a851b1d4dac39b`.
- The tracked patch applies to a pristine 0.3.24 extraction and produces the
  same patched `src/lib.rs` as the vendored tree.
- `cargo tree --locked --offline --package nagi-init --features m17-servo
  --target targets/x86_64-unknown-nagi-user.json --invert inventory` resolves
  inventory from `third_party/inventory-nagi` throughout Servo.
- All 81 `nagi-cli` library tests and 18 CLI integration tests pass using the
  pinned Rust compiler explicitly; the new resource-registration source
  contract is included.
- Focused formatting, `nagi-cli` Clippy with warnings denied, the acceptance
  script syntax, and `git diff --check` pass. This worktree lacks the generated
  Rust std, Mesa, and package inputs for a local Nagi-target image build;
  public Ubuntu CI must validate the new guest code and constructor link
  before QEMU can test the preflight.

M17 remains `BLOCKED` until public target CI reports the real resource-reader
marker, Servo/WebView startup, a nonzero first-web-pixel checksum, and the M17
PASS marker. M18 remains `NOT STARTED`.

### Script-thread and about:blank dispatch tracing after CI run #293 (2026-09-27)

Public CI run #293 (`36314057355`, head
`56ebc809d75dec694dcb95fa528cb312e839c243`) passed the Ubuntu host checks,
Windows launcher, target dependency boundary, Mesa Softpipe, M16 package,
kernel, real user-init link, and UEFI loader. The QEMU M17 acceptance again
timed out after its 120-second guest bound (exit code 4), with no first-web-
pixel checksum or PASS marker.

The new Constellation trace proves that Servo received `NewWebView`, registered
the top-level browsing context, created the script event loop, sent
`SpawnPipeline`, and returned from pipeline creation. The next visible line is
a generic pthread trampoline trace, which does not identify which Servo worker
entered or what it did. In the pinned Servo source, `Pipeline::spawn` only sends
`SpawnPipeline` to the script event-loop channel; successful return does not
mean that the script thread processed it.

Tracked patch `0016-nagi-m17-script-pipeline-traces.patch` adds Nagi-only
checkpoints for script-thread entry, per-thread JavaScript runtime and
debugger-global initialization, script-loop entry, `SpawnPipeline` dispatch,
and the synchronous `about:blank` response through parser metadata, content,
and EOF. It does not alter scheduling or load behavior. The source-contract
test failed before the patch existed, then passed after it was added. All 84
`nagi-cli` library tests and the focused formatting check pass. The patch
passes `git apply --check`, and `./nagi fetch` with the pinned Rust toolchain
regenerated the Servo checkout with the complete ordered patch set. Public
target CI must validate compilation and provide the next guest boundary.

M17 remains `BLOCKED` pending the real first-web-pixel checksum and PASS
marker. M18 remains `NOT STARTED`.

### SpiderMonkey per-thread context creation tracing after CI run #294 (2026-09-27)

Public CI run #294 (`36317441144`, head
`ec0006164ecc234a295359c442864c1c17d5e304`) passed both host jobs, Servo
source bootstrap, target dependency validation, Mesa Softpipe, the M16 package,
kernel, real user-init link, and UEFI loader. Its two QEMU acceptance boots
timed out after the 120-second guest bound (exit code 4); neither produced a
first-web-pixel checksum or PASS marker.

The new script-thread trace confirms that the worker starts and enters
`ScriptThread::new`, but `ScriptThread runtime creation started` has no matching
completion marker. In the pinned Servo source this call enters
`script_runtime::Runtime::new`, whose non-parent path acquires the shared
SpiderMonkey engine handle and constructs a per-thread `RustRuntime`. The run
therefore narrows the stall to that runtime-construction call; it does not yet
identify whether the engine-handle lookup, `JS_NewContext`, or its initialization
is responsible.

Tracked Servo patch `0017-nagi-m17-js-runtime-traces.patch` brackets engine
handle acquisition, `RustRuntime::new`, and JSContext retrieval. Tracked
mozjs-sys patch `0018-nagi-m17-js-context-traces.patch` brackets the native
`JS_NewContext` path through `JSRuntime`/`JSContext` allocation and
initialization. Both patches add Nagi-only diagnostics and preserve the
existing initialization path. Their source-contract tests were added first
and failed because the patch files were absent; both patch files now pass
`git apply --check` against the pinned generated sources. All 86 `nagi-cli`
library tests and 18 CLI integration tests pass, as do the focused format
check, `nagi-cli` Clippy with warnings denied, and `git diff --check`. A fresh
`./nagi fetch` with the pinned Rust toolchain regenerated both Servo and
mozjs-sys from their locked sources and applied the complete ordered patch
sets. Public CI must validate the target build and provide the next guest
trace boundary.

M17 remains `BLOCKED` pending a real first-web-pixel checksum and PASS marker.
M18 remains `NOT STARTED`.

### SpiderMonkey JSRuntime and helper-thread initialization after CI run #295 (2026-09-27)

Public CI run #295 (`36320499660`, head
`d9279f741cd721bd303e87550d664fffc21d45eb`) passed the Windows launcher and
Ubuntu host jobs, target dependency validation, Mesa Softpipe, the M16 package,
kernel, real `nagi-init` link, and UEFI loader. The real QEMU acceptance timed
out after the 120-second guest bound (exit code 4); no first-web-pixel checksum
or PASS marker was produced.

The guest trace reaches the script worker, Servo's per-thread JavaScript
runtime, SpiderMonkey engine-handle acquisition, `RustRuntime::new`, native
`JS_NewContext`, JSRuntime/JSContext allocation, and JSContext initialization.
`JSRuntime::init` is entered but its completion marker is absent. The trace has
not yet identified which operation inside that function is stalled.

Tracked mozjs-sys patch `0019-nagi-m17-js-runtime-init-traces.patch` now
brackets helper-thread policy and initialization, GC and number-state setup,
time-zone reset, and set-prop cache allocation. It also brackets the helper
state lock, internal pool provisioning, worker creation, and worker entry into
the existing wait loop. The diagnostic markers are Nagi-only and preserve the
normal runtime policy and call order. Its source-contract test was added first
and failed while the patch file was absent; it passes with the patch present.
All 87 `nagi-cli` library tests and 18 CLI integration tests pass, as do
formatting, Clippy with warnings denied, and `git diff --check`. `./nagi fetch`
passed using the pinned Rust toolchain and regenerated mozjs with the complete
ordered patches; reverse-apply validation confirms patch 0019 is present in
that clean source checkout. Public CI must now validate target C++ compilation
and provide the next QEMU trace boundary.

M17 remains `BLOCKED` pending a real first-web-pixel checksum and PASS marker.
M18 remains `NOT STARTED`.


# M18 - Albert Browser (`PARTIAL`)

### Main browser composition checkpoint (2026-09-28)

The main M18 worktree is based on the fixed M17 PASS SHA
`94e9a027618182b10c0ac2315e94673543f22423`. It includes the independent
`./nagi m18` target image and QEMU path, a TLS-verifier callback that records
only successful chain and hostname verification, a three-site real-HTTPS guest
runner, and a host serial-log validator. The validator requires one TLS proof,
one browser-chrome presentation, and one nonzero Servo-frame checksum for each
configured host, plus evidence that the first HTTPS request came through the
address bar, before accepting the summary.

The main guest runner now uses M18-B's typed `BrowserState` to issue and
complete per-tab navigations, records same-host redirects and actual Servo page
titles, composes the corresponding browser chrome over each real Servo frame,
and sends the composed image through the capability-checked Nagi Surface.
The runner receives the granted input capability, translates pointer and
primary-button events, edits the address bar from bounded US evdev keys, and
dispatches the typed navigation request to Servo. The QMP path injects the
corresponding click, `example.com` keystrokes, and Enter event.
M18-B's tabs, address/history/bookmark/session models, permission/transfer
state, IME model, and chrome renderer are included from checkpoint
`ee49b812fa69c943c34ca076fe795e6ba92e504f`. M18-A's nonblocking POSIX socket
operations, smoltcp TCP/DNS behavior, and UEFI-to-kernel realtime seed are
integrated from checkpoint `330f322fbfd1c8fc8e696183fc7f13a019644804`.
M18-A CI run `36379279390` passed Ubuntu host and Windows launcher checks but
failed target compilation at a crate-root `pub(super) mod guest` visibility
error on that branch; it did not reach QEMU. That visibility change is not
part of this worktree, and this integration still needs its own target build.

Local verification:

- `cargo test -p nagi-cli --locked --offline`: 106 unit tests and 18 CLI
  integration tests passed after the final acceptance-validator changes.
- `cargo test --manifest-path user/nagi-albert/Cargo.toml --features m18-acceptance --locked --offline`:
  43 browser-state, chrome, persistence, permission, clipboard, transfer, IME,
  and address-input tests passed. The host target excludes the
  `target_os = "nagi"` guest runner.
- `cargo test --manifest-path user/nagi-servo/Cargo.toml --locked --offline`:
  4 surface/input-adapter tests passed.
- `cargo test -p nagi-abi -p nagi-bootinfo --locked --offline`: 3 ABI and 11
  BootInfo/firmware-clock tests passed.
- Clippy with warnings denied passed for `nagi-cli`, `nagi-albert` with
  `m18-acceptance`, and `nagi-servo-adapter`; focused package Rust formatting
  and staged/unstaged diff checks passed.
- The standalone `nagi-servo-adapter` Nagi-target `cargo check` passed with the
  patched `core`/`alloc` source; the full `nagi-init` target check remains
  blocked before Albert's target code by the macOS SpiderMonkey linker probe.
- A focused Nagi-target `cargo check` for `user/nagi-posix` passed with the
  patched `core`/`alloc` source, typechecking the imported nonblocking socket,
  DNS, smoltcp, and POSIX integration path. Native host tests for `nagi-net`
  and `nagi-posix` cannot compile the Nagi x86-64 syscall assembly on this
  aarch64 macOS host; the same M18-A packages passed the Ubuntu host job in CI
  run `36379279390`.
- The Nagi kernel release build and x86-64 UEFI loader release build passed
  with the new BootInfo realtime field and kernel clock handoff.
- Focused Nagi target C++ compilation passes with Homebrew libc++ headers, and
  the target check gets past the C++ dependency compile. Full target checking
  stops in `mozjs-sys-nagi` configuration: macOS Clang routes the ELF link probe
through `ld64.lld`, which rejects GNU ELF linker arguments. With the Python
venv path made absolute, `./nagi m18` passes the Mako check and stops at Mesa's
  `atomic` linker probe, also caused by the Darwin linker. Neither M18 image
  creation nor QEMU acceptance has been reached. The target-only input route and
  QMP injection are source- and host-test-covered but not yet target-verified.
- The Homebrew `ld.lld` executable can directly link a minimal x86-64 ELF
  object, but Homebrew Clang 19 still routes target link commands through the
  macOS GCC/`ld64.lld` path, including with `--ld-path`. This does not provide a
  safe local workaround for the repository's Meson and SpiderMonkey probes;
  no host-specific linker bypass was added.
- M18-A's nonblocking socket, DNS, smoltcp, and firmware realtime code is now
  part of this worktree's M18 build feature, and the POSIX networking graph
  passed a focused Nagi-target check. The runner now owns one Servo WebView per
  `BrowserState` tab, switches visibility with the active tab, drops views when
  tabs close, and routes page input/navigation to the active view. This
  target-only wiring has not yet compiled or been exercised in QEMU. The three
  HTTPS acceptance pages still run sequentially in the selected tab. Session,
  history, and bookmark state now use a pathless Nagi POSIX snapshot service
  with pending-file/replace commits through the guest VFS. Its ABI is enabled
  only by the M18 feature; CI asserts M17 excludes it and M18 includes it. The
  current VFS limits the combined snapshot to 1 KiB; the three-site acceptance
  fixture is covered to fit, while larger collections report capacity without
  stopping navigation. Restored page requests are deferred until user input so
  the first HTTPS request still proves the QMP address-bar route. Clipboard,
  download/upload, IME, site-permission service adapters, and Ubuntu target/QEMU
  evidence remain outstanding. M18 remains `PARTIAL`; no QEMU success or
  milestone PASS is claimed.

### Guest browser snapshot persistence (2026-09-28)

Albert's `BrowserStorage` implementation calls a pathless Nagi POSIX storage
service. The service owns fixed VFS names and accepts neither page-controlled
paths nor raw block capabilities. It writes one checksummed snapshot to a
pending inode, flushes it, atomically replaces the active root entry, and
flushes the directory update. Session validation still rebuilds browser state
and discards permissions, clipboard data, downloads, and upload selections.
The POSIX ABI for this fixed-purpose service is enabled only by
`m18-acceptance`; the target dependency checks require it absent from M17 and
present in M18.

Verification: 48 `nagi-albert` tests pass with `m18-acceptance`, including
snapshot round-trip, corruption/duplicate detection, and the three-site
snapshot capacity. Clippy with warnings denied passes for all Albert targets.
Focused Nagi-target checks for `nagi-posix` pass both without and with the
`browser-storage` feature, with five existing unrelated POSIX warnings. The
M17/M18 dependency graphs confirm the feature is disabled for M17 and enabled
for M18. On 2026-09-28,
`./nagi m18` stopped during Mesa/Softpipe Meson setup before the M18 target
image or QEMU run: the `-latomic` link probe passed GNU ELF options to macOS
`ld64.lld`, which rejected `--entry=0` and `--unresolved-symbols=ignore-all`;
Meson then reported `C shared or static library 'atomic' not found`. Ubuntu
target compilation and real QEMU HTTPS acceptance remain unverified. No M18
PASS is claimed.

### M18 macOS target-link repair, service boundaries, and QEMU acceptance (2026-09-29)

The original macOS failure was a host/target linker mismatch, not a missing
Nagi target library. Mesa's freestanding x86-64 target objects were being
linked through Apple's Clang driver and its Darwin `ld64.lld` route; the
`-latomic` capability probe passed GNU ELF options (`--entry=0` and
`--unresolved-symbols=ignore-all`) to that Mach-O linker, so Meson incorrectly
reported that target `atomic` was unavailable. `tools/mesa/build.sh` now adds
a generated Meson cross-file override only when `uname -s` is Darwin, and
`tools/nagi-target-cc.sh` selects the tracked ELF adapter only for Darwin
linking invocations. `tools/mesa/nagi-ld-adapter.sh` invokes the installed ELF
LLD directly and filters host-only macOS driver arguments. It does not delete
`-latomic`; the Mesa probe and Softpipe build complete. Compile-only target
calls and host-side `HOST_CC`/`HOST_CXX` configure helpers are unaffected.
Ubuntu continues using the existing tracked Meson cross file and normal
Clang/LLD route, while M17 keeps its original kernel features and default
32-slot browser-worker capacity. The M18-only 512 MiB mmap window uses an
additional page-directory table for the final four 2 MiB entries, preserving
the M17 256 MiB layout.

`./nagi m18` passed on this macOS host on 2026-09-29. The guest accepted the
QMP-injected address-bar navigation to `example.com`, verified TLS chain and
hostname for `example.com`, `example.org`, and `example.net`, composed Albert
chrome over real Servo frames, and presented them through the capability-
checked Nagi Surface under QEMU. The final saved serial log is
`out/logs/m18-albert.log.live10-mmap512-service-boundaries-three-https-pass-20260929T104534`;
the matching QMP and image evidence use the same `live10-mmap512-service-boundaries-three-https-pass-20260929T104534`
suffix. The unchanged `./nagi m17` regression then passed on this host:
`PASS M17 first web pixel: real Servo/Mesa Softpipe frame reached Nagi Surface
and QEMU`. The harmless QEMU virtio-sound host-audio warning does not affect
either local acceptance result.

The first pushed Ubuntu/Windows CI run (`36510598517`, head
`7222609a8b0c865aea6a424f64cb6c824c1bd654`) stopped during source bootstrap
on all three OS jobs. The pinned Servo patch `0021` expected the Nagi-only
`#[cfg]` line to already exist, although no earlier numbered patch adds that
line on a fresh checkout; the pre-prepared local source had hidden this
ordering defect. Patch `0021` now adds the cfg-gated atomic import itself.
`cargo test -p nagi-cli --locked --offline` passes all 112 unit tests and 18
CLI tests; the pinned nightly formatting check passes. All seven M18 Servo
patches also pass sequential `git apply --check` and application against the
pinned M17-patched source fixture. The Ubuntu target image/QEMU steps were not
reached in run `36510598517`; the repair commit and fresh CI result are pending.

The guest now attaches Servo clipboard and permission hooks. With no Nagi
clipboard provider, clipboard reads fail and writes/clears report unavailable;
there is no host clipboard fallback. Servo site-permission requests are mapped
with the requesting document's serialized origin (including opaque `null`)
to Albert's typed permission state and denied by default until a trusted prompt
service exists. File-picker requests are dismissed because Servo's picker
returns host paths and Nagi has no capability-safe file/object picker. IME
controls are observed and reported unavailable because no text-composition
service events reach the guest. Albert has download/upload state models, but
the pinned Servo API has no download callback and no Nagi file destination or
selection service; no fake transfer is reported. These are fail-closed hooks,
not completed service integrations.

Focused verification after these changes: 50 `nagi-albert` host tests passed
with `m18-acceptance`; the kernel page-table hierarchy regression passed with
and without `m18-browser-memory`; focused Rust formatting and shell syntax
checks passed. `git diff --check` reports only intentional blank context lines
inside the newly added unified Servo patch files.
Overall M18 remains `PARTIAL` until its remaining required service providers
are implemented and verified. Its browser HTTPS/QEMU Acceptance is `PASS`.

### M18 site-permission requester origin (2026-09-29)

Servo's permission and screen-wake-lock requests now carry the serialized
origin from the requesting `GlobalScope` through `PermissionRequest`. Albert
records that origin instead of substituting the WebView's top-level URL; an
opaque origin remains `null` and is denied without being attributed to another
site. This fixes cross-origin iframe attribution while retaining fail-closed
behavior; it does not grant site permissions or create the missing trusted
prompt service. The change is reproducible in
`third_party/servo-patches/0025-nagi-m18-permission-origin.patch`.

The new opaque-origin regression failed before the fix and passes after it.
The focused Albert suite passes 51 tests, `nagi-cli` passes 113 unit tests and
18 CLI tests, and a fresh local `./nagi m18` passes with three TLS-verified
HTTPS pages rendered through Nagi Surface and QEMU. The final local run is
recorded in `out/logs/m18-origin-renumbered-20260929.log`, with guest evidence
in `out/logs/m18-albert.log`. The M17 target/QEMU regression also passes with
its first-web-pixel marker in `out/logs/m17-servo.log`. Ubuntu CI run
`36533931477` validates the corrected origin patch on a clean checkout and
retains the Linux Clang/LLD path. Download destination,
capability-safe upload/file selection, shared clipboard, IME text/composition
input, and a trusted interactive permission service remain absent from the
repository's user-space service/IPC interfaces, so those paths remain
fail-closed. Overall M18 therefore remains `PARTIAL` even though its
HTTPS/QEMU Acceptance is `PASS`.

### M18 fresh-source patch correction and acceptance rerun (2026-09-29)

The first CI run for Servo patch `0025` (`36530525632`) failed during clean
source bootstrap: the `webview_delegate.rs` accessor hunk did not match the
pinned source around line 84. This was a patch-context defect; compilation and
QEMU acceptance were not reached. The second hunk now uses the stable
`feature()` method signature as context and matches the single blank line in
the pinned source. Sequential application after patch `0024` and reverse-apply
validation succeed on a fresh pinned-source fixture; the resulting five
Servo files match the preserved generated checkout.

After correcting the hunk, a fresh macOS `./nagi m18` completed successfully:
`PASS M18 Albert: three verified HTTPS pages rendered to Nagi Surface and
QEMU`. The command transcript is `out/logs/m18-origin-hunk-fix-20260929.log`
and the guest serial evidence is `out/logs/m18-albert.log`. The unchanged
Darwin-only ELF target-link adapter still passes Mesa's `-latomic` link probe;
it invokes ELF LLD for Nagi target link checks without removing `-latomic` or
altering host build tools. Linux continues to use the existing Clang/LLD
cross-file route; corrected-origin full CI evidence is run `36533931477`.

On the corrected patch, `cargo test -p nagi-cli --locked --offline` passes all
113 unit tests and 18 CLI tests, `cargo test --manifest-path
user/nagi-albert/Cargo.toml --features m18-acceptance --locked --offline`
passes all 51 Albert tests, both affected Clippy commands pass with warnings
denied, and both pinned-nightly format checks pass. A fresh M17 real-QEMU
first-web-pixel regression also passes after the correction; its command log
is `out/logs/m17-post-patch-hunk-fix-20260929.log` and guest serial evidence is
`out/logs/m17-servo.log`. Corrected commit
`eb22702da8e832126c32e420c8fde579b05f8a67` passed CI run `36533931477`:
clean-source Servo bootstrap passed on Windows and Ubuntu, and the target job
passed dependency boundaries, Mesa Softpipe, image/kernel/UEFI builds, M17
QEMU, M18-B chrome, and three-site M18 HTTPS/QEMU acceptance. The job log
records `PASS M18 Albert: three verified HTTPS pages rendered to Nagi Surface
and QEMU`. The five service connections remain fail-closed because the
repository still
has no capability-safe download destination, upload/file picker, shared
clipboard provider, IME text/composition event source, or trusted interactive
site-permission provider. Existing M6 ServiceRegistry calls are in-process
and do not provide the isolated IPC/capability boundary needed to invent
providers inside Albert. M18 stays `PARTIAL`; its formal HTTPS/QEMU Acceptance
is `PASS` locally and in CI.

## Completion sweep — M29 QEMU display evidence (2026-09-30)

The M10 logical surface is now aspect-fitted across the GOP framebuffer and
centered with cleared letterbox bars. At the QEMU reference mode (1280×800),
the 320×200 guest surface fills the scanout, replacing the previous small
upper-left surface and residual UEFI pixels. Pixel-format conversion remains
in the kernel scanout path. The M10 bitmap font now draws the visible ASCII
lowercase letters distinctly, includes the additional punctuation used by its
labels, and uses separate six-bit Latin and seven-bit Japanese glyph widths.
This remains a small fixed glyph table, not a complete localization font.

`nagi desktop` now asks QEMU's QMP `screendump` to save a PNG only after the
guest has printed its M10 acceptance marker. The command refuses to overwrite
an existing screenshot and checks the PNG signature and nonzero dimensions.
The accepted run on 2026-09-30 printed the M10 READY and nonzero surface
checksum, Calculator/Notes/Files/Terminal focus, Japanese input, and final
acceptance markers. `docs/assets/screenshots/nagi-m10-qemu-desktop.png` is the
resulting 1280×800 guest image (SHA-256
`061c02741343026c2b6974ae846fbbf26bde48b8e00d0160abe918da2744932e`); the
full local command output and original capture remain under `out/logs/` and
`out/evidence/` in the worktree.

Verification on the local macOS host: `./nagi desktop` passed; `cargo test
--locked --offline -p nagi-cli --all-targets` passed (135 unit tests and 19
integration tests); `cargo clippy --locked --offline -p nagi-cli --all-targets
-- -D warnings` passed; pinned-nightly formatting for `nagi-cli`,
`nagi-kernel`, and `nagi-init` passed; and `git diff --check` passed. The
scanout geometry unit test was added to Ubuntu-host CI because this AArch64
macOS host cannot execute the kernel's x86-only inline assembly as a native
test. The real x86-64 M10 guest acceptance exercises the 1280×800 path.

M29 remains `PARTIAL`: this is the fixed M10 acceptance surface, not a
finished desktop UX. Settings, first-run setup, accessibility, complete
localization, clean-install/cross-host boot-time benchmarks, and additional
preview screenshots are still outstanding. No milestone was promoted to
`PASS` by this checkpoint.

### M29 QEMU startup timing (2026-09-30)

The M10 GUI QEMU helper now returns the host monotonic duration from the QEMU
child spawn to receipt of the guest's `Nagi M10 desktop READY` serial marker.
Successful `nagi desktop` output reports this value. An early QEMU exit keeps
the measurement absent and retains the ordered-marker failure path.

Three repeated `./nagi desktop` runs on the local macOS aarch64 host all passed
their real guest acceptance. Start-to-READY samples were 2,486 ms, 2,466 ms,
and 2,568 ms; median 2,486 ms, range 2,466–2,568 ms. The runs used the existing
persistent User Data image and the standard QEMU reference-machine settings.
Each run's boot image, variables, serial logs, persistent-disk snapshot,
screenshot, and measurement record is preserved in
`out/evidence/m29-desktop-timing-sample-1/` through `sample-3/`. These are
host-observed repeat-boot timings that include QEMU/UEFI startup and serial
delivery, not clean-install or cross-platform performance claims.

After adding the timing result, `cargo test --locked --offline -p nagi-cli
--all-targets` passed (135 unit tests and 19 integration tests),
`cargo clippy --locked --offline -p nagi-cli --all-targets -- -D warnings`
passed, pinned-nightly formatting and `git diff --check` passed, and all three
QEMU M10 desktop acceptances passed. M29 remains `PARTIAL`; this measurement
does not cover first installation, the M30 release image, or a multi-host
performance matrix.

## Completion sweep — M20 backend-registration target compile (2026-10-01)

Added numbered llama.cpp patch `0002` for the exception-only path conversion
in `ggml-backend-reg.cpp`. Nagi uses its native UTF-8 path bytes; non-Nagi
builds retain the upstream conversion and fallback. The raw pinned checkout
remains clean. `./nagi fetch` passed, the focused target translation unit
changed from its reproduced `try`/`-fno-exceptions` failure to a successful
compile, and a fresh CPU-only static Nagi-target `ggml` CMake build passed
31/31. Full `llama` compilation now passes backend registration but still fails
in 28 object targets with exception diagnostics across 63 source files, two
RTTI uses, and one missing `PATH_MAX` definition. The CMake cache, generated
source marker, patch, and build logs are preserved in
`out/evidence/m20-backend-reg-noexceptions-20261001/`; the prior generated
source checkout is retained under `out/cache/`.

The three llama patch/lock tests and repository `./nagi fmt`, `./nagi lint`,
`./nagi test`, and `./nagi build` passed. No M20 QEMU inference attempt is
claimed because the complete llama target/provider backend is still absent.
M20 remains `PARTIAL`.


## Completion sweep — clean-source M30 release checkpoint (2026-10-01)

On clean source commit `284dbf1`, release preflight, assemble, and verify passed.
The bundle is at
`out/artifacts/m30-release-bundle-284dbf1/`; its qcow2 is byte-identical to
the validated reference input (SHA-256
`461c644d48e4b0d33b937ce6852eb9a6034abe391ea74c4c24e1ac0b99ca2d43`), and
`qemu-img check` reported no errors. The package manifest intentionally
records `m30_acceptance=NOT_EVALUATED`.

`./nagi m30` also passed on this source. It booted a disposable copy through
System A, formatted and wrote User Data on the first boot, then mounted and
read persistent data after restart. Its invocation log is
`out/logs/m30-284dbf1-qemu.log`; serial logs and the acceptance copy are under
`out/evidence/m30-release-1790795417285158000/`. The eight release-tool tests
passed.

This assembly and QEMU rerun used the existing kernel and reference qcow2
inputs; they did not rebuild those payloads. The image is still accepted on
QEMU, and the bundle integrity is verified, but current-head payload rebuild
and authenticated GPT update installation remain open. M30 remains `PARTIAL`.


## Completion sweep — M28 unique evidence namespace and M27 timeout (2026-10-01)

The M28 acceptance runner now uses a per-run UTC timestamp/PID namespace for
intermediate repetition evidence; it no longer reuses the legacy
`out/evidence/m28-repetition-N/` names. Shell syntax, self-test, and
two-repetition dry-run passed.

Before the real run, fixed-name M19/M22 images, OVMF variables, serial logs,
and copies of both persistent disks were hash-preserved under
`out/evidence/pre-m28-repeat-20261001-0a331f6/`. Repetition 1 passed the
M19 Search/ObjectId gate and all three M22 grouped-Undo/Activity-Ledger boots.
The M27 A/B gate then timed out on boot 4 after rolling back to confirmed A.
Its log reached three M3 AP-online messages but no scheduler-workload marker.
QMP reported `status=shutdown`; the CPU#0 instruction pointer maps to
`nagi_kernel::smp::thread_entry`. This points to the M3 SMP workload
transition as the area to isolate, but the timeout does not establish a
specific fault. The attempt is not counted as an M27 or M28 pass.

The M27 images, OVMF state, guest data, serial logs, and QMP diagnostics with a
verified SHA-256 manifest are in
`out/evidence/m27-ab-rollback-1790796067483623000/`. M28 repetition-1
M19/M22 outputs and post-run disk copies are preserved at
`out/evidence/m28-run-failure-20261001-0a331f6/`. The next useful
diagnostic is to capture bounded per-CPU timer/workload progress around M3
startup and compare a replay of the saved rollback state before changing
scheduler behavior. M27 and M28 remain `PARTIAL`.

The subsequent fresh `./nagi m27` acceptance passed its rollback, Recovery,
healthy-B promotion, persistence, and grouped-Undo checks at
`out/evidence/m27-ab-rollback-1790800250176976000/`. Its inputs and firmware
state differ from the prior failed run, so this is a successful sequence replay
but not a reproduction of that exact firmware state. The old failure did not
recur; its root cause remains unconfirmed. The M28 integrated two-repetition
gate still needs a complete pass.

Two new two-repetition M28 attempts each passed M19, M22, and M27 in
repetition 1, then passed M19 in repetition 2 before M22 boot 1 timed out
before the Nagi kernel marker. Both QMP snapshots show the same RIP
`0x7eb84171` in firmware address space while QEMU remains running, with only
the 87-byte UEFI screen-clear sequence in serial. Each failed M22 disk image
matches its successful repetition-1 image, and each data disk matches its
post-acceptance repetition-1 snapshot. The new partial runs and hash manifests
are in `out/evidence/m28-run-20260930T203759Z-48306/` and
`out/evidence/m28-run-20260930T204402Z-48900/`. The failure remains an
unconfirmed firmware-start timeout; no complete M28 two-repetition pass is
claimed.


## Completion sweep — M18 browser screenshot evidence (2026-10-01)

`tools/nagi-cli` now saves the QMP display after the real M18 three-page
HTTPS acceptance marker. The QEMU run kept the ESP read-only, used a unique
`out/evidence/m29-browser-<run-id>/` directory, and refused to overwrite an
existing screenshot. The run passed on the local macOS aarch64 host with
`PASS M18 Albert: three verified HTTPS pages rendered to Nagi Surface and
QEMU` (exit 0). The guest serial log contains verified TLS, presented chrome,
and rendered-page markers for `example.com`, `example.org`, and `example.net`,
followed by `Nagi M18 browser scenario complete pages=3`.

The accepted 1280×800 PNG is tracked as
`docs/assets/screenshots/nagi-m18-qemu-browser.png` (SHA-256
`ede8a7967a1634c393aa53252749af22d8d98aa91e4d4711402de0e860c7e097`). The
original capture is preserved under
`out/evidence/m29-browser-1790798334374076000/`; invocation and serial logs
are `out/logs/m18-screenshot-attempt-1790799000000000000.log` and
`out/logs/m18-albert.log`. The host QEMU build emitted an audio-backend
diagnostic because `virtio-sound.in` could not be opened, but browser
acceptance passed; host audio playback was not tested. Host CLI tests,
warnings-denied Clippy, and formatting checks passed before the target run.
M29 remains `PARTIAL`; the screenshot adds evidence without changing any
milestone status.


## Completion sweep — M20 llama.cpp target boundary repair (2026-10-01)

Added llama.cpp patch `0003-nagi-model-boundaries.patch`. Its Nagi-only
virtual model-base query removes two RTTI compile errors while preserving a
null check; non-Nagi builds keep the upstream casts. `llama_path_max()` now
matches Nagi's 256-byte VFS path limit plus the C-string terminator. A CLI
regression for the Nagi no-RTTI and path-capacity contract failed before the
patch existed and passes after it.

`./nagi fetch` applied and validated patches 0001–0003 without modifying the
raw pinned source. The static CPU llama target was retried with Ninja
keep-going: the RTTI and `PATH_MAX` diagnostics are gone, but 28 object targets
still fail on C++ exceptions. The new run reports 57 distinct source paths,
289 `throw` diagnostics, and 15 `try` diagnostics. Full target build and M20
QEMU inference acceptance remain incomplete; no exception behavior was
replaced with an abort or stub. Build inputs, patch, generated marker, log, and
hash manifest are preserved in
`out/evidence/m20-nagi-boundaries-0003-20261001/`. M20 remains `PARTIAL`.

Verification: CLI tests passed (145 unit and 21 integration tests), as did
warnings-denied Clippy, pinned-nightly formatting, `./nagi fmt`, `./nagi test`,
`./nagi lint`, `./nagi build`, `./nagi fetch`, and `git diff --check`.


## Completion sweep — M30 current-source payload and release rebuild (2026-10-01)

The earlier 64 GiB image was preserved before rebuilding at
`out/evidence/m30-pre-current-rebuild-20261001/original-reference.qcow2`; its
verified SHA-256 remains
`461c644d48e4b0d33b937ce6852eb9a6034abe391ea74c4c24e1ac0b99ca2d43`. With
that fixed-path image moved aside, `./nagi m30` rebuilt the production payloads
and six-partition reference disk from clean source revision
`d2cbff3ed9bde82bf6dde910b3b5bf5e30c6cfb7`. The generated 64 GiB qcow2 has
SHA-256 `f76088e25cea65176035940930dd3b9fd2df796614d0e554fb8d345271558bb9`;
its kernel build ID is `sha256:2c899885d4569d58cb29ab028bc9923e44519c8fa25009f6fce836cc6e982343`.

Both fresh M30 QEMU runs passed GPT System A selection, first-boot User Data
format/write, and a restart with persistent read and `Nagi M7 acceptance PASS`.
After release assembly, the second run exercised a disposable copy from the
same reference image whose SHA-256 exactly matches the bundled qcow2. Its logs,
copy, README, and verified `SHA256SUMS` are at
`out/evidence/m30-release-1790802137948877000/`. The eight release-tool tests,
clean-tree preflight, assembly to `out/artifacts/m30-release-bundle-d2cbff3/`,
and release verification passed. Post-acceptance checksum verification
reported all 15 package entries valid, and `qemu-img check` found no errors on
the bundled 64 GiB image. The release manifest intentionally keeps
`m30_acceptance` at `NOT_EVALUATED`; guest acceptance is separately evidenced
above. M30 remains `PARTIAL` for authenticated update installation, remaining
M18–M29 acceptance, and binary license/notice review. The QEMU host also has
no `virtio-sound.in` audio backend; this run does not establish audio
acceptance.


## Completion sweep — M28 repeated integration gate and QMP diagnostics (2026-10-01)

Added a bounded `x/12i $rip` HMP query to QEMU timeout diagnostics alongside
the existing status and register queries. The entire diagnostic sequence keeps
the existing three-second timeout budget. A focused QMP fixture test passed,
and a live QEMU/QMP smoke confirmed the command returns a 12-instruction
response; the transcript is
`out/evidence/m28-run-20260930T211250Z-51667/qmp-instruction-smoke.log`.

After the two earlier M28 attempts stopped before the Nagi kernel on repetition
2 M22 boot 1, this two-repetition run passed all M19, M22, and M27 gates. Its
21 run files plus README are verified by the 22-entry `SHA256SUMS` in
`out/evidence/m28-run-20260930T211250Z-51667/`. The associated full M27
acceptances at `out/evidence/m27-ab-rollback-1790802782930760000/` and
`out/evidence/m27-ab-rollback-1790802894479757000/` each have a verified
31-entry manifest. The prior firmware-start timeout did not recur; its cause
remains unknown, and the new timeout-only disassembly query was not exercised
by this passing run.

M28 remains `PARTIAL`: the passing two-repetition gate covers the M19/M22/M27
Search, grouped Undo, and Recovery slice, not the full Desktop/Files/Notes/
Albert reference load, real Granite inference, audio pressure, OOM, fairness,
or leak soak. `./nagi test`, `./nagi lint`, `./nagi fmt`, and `./nagi build`
passed, as did the focused QMP test and M28 harness syntax, self-test, and
dry-run.


## Completion sweep — M30 tracked license texts and release verification (2026-10-01)

Commit `16e0cd4` updates the M30 assembler to copy the eight non-empty license
texts currently tracked under `third_party/` into the release bundle and record
their source paths, package paths, and SHA-256 hashes. Twelve release-tool tests
pass, including deterministic selection, tamper detection, symlink rejection,
and compatibility with earlier schema-v1 bundles. The license inventory is
limited to tracked checkout files; fetched/generated and transitive/native
license texts and human redistribution review remain incomplete, so M29 and
M30 remain `PARTIAL`.

Clean-source preflight, assembly, and verification passed for
`out/artifacts/m30-release-bundle-16e0cd4/`. The bundle has 24 files, eight
tracked license texts, and 23 verified `SHA256SUMS` entries. Its qcow2 matches
the source reference image at SHA-256
`e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`; after a
two-boot `./nagi m30` run on a disposable copy of that image, release
verification, every bundle checksum, and `qemu-img check` passed again on the
untouched package. The QEMU acceptance logs and verified evidence checksums are
in `out/evidence/m30-release-1790807691542412000/`. The run passed System A,
M20 Model Store capability checks, User Data format/write/restart-read, and M7
acceptance; the host lacked `virtio-sound.in`. The bundle's
`m30_acceptance=NOT_EVALUATED` is unchanged. See
`docs/workstreams/NagiOS_M30_Release_Workstream.md`.


## Completion sweep — current-commit M30 release bundle QEMU acceptance (2026-10-01)

On clean commit `637f5755a57eaa14adc379fdddfaeebb5dea6387`, release preflight,
assembly, and verification passed; all 12 release-tool tests passed. The
assembled package at `out/artifacts/m30-release-bundle-637f575/` has 23
verified checksum entries and the expected source revision. Its qcow2 SHA-256
is `e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`.

A byte-identical copy of the package qcow2 booted twice with shared OVMF
variables. System A, the Model Store read-only capability, and the M27
confirmed-A journal passed; boot 1 formatted and wrote User Data, and boot 2
read the persisted data and reached `Nagi M7 acceptance PASS`. The disposable
copy's SHA-256 changed to
`0eafa697f6a918486eeb9bd3b6cf673d3792d1698461c76ca3ea88a47b26940a` after
guest writes. The untouched package retained its original hash and passed
post-boot `release.py verify`; `qemu-img check` passed on both images. Evidence
is under `out/evidence/m30-clean-release-637f575/`, including both serial logs,
the QEMU runner, OVMF variables, and before/after hashes. The package manifest
still records `m30_acceptance=NOT_EVALUATED`; guest acceptance is separate.

M30 remains `PARTIAL` for authenticated GPT update installation, remaining
M18–M29 acceptance, and human binary redistribution review.


## Completion sweep — M27 after M29 persistence (2026-10-01)

`./nagi m27` passed the complete GPT A/B and Recovery acceptance after the
M29 Desktop language persistence change. Three malformed System B trials
rolled back to A, healthy B was promoted after guest readiness, Recovery left
the journal unchanged, and Recovery undid the committed three-file M22 group
across restart. Evidence and the verified SHA-256 manifest are in
`out/evidence/m27-ab-rollback-1790816764088451000/`.

The preceding fresh run (`1790816606753646000`) timed out during its first
malformed-B trial before guest output. QMP showed a running VM with CPU#0 in an
OVMF instruction loop. The successful run used new image and firmware state;
the root cause remains unknown, so the failed attempt is not counted as an
acceptance pass. M27 remains `PARTIAL` for authenticated slot/update authority,
full session readiness, and the remaining Recovery functions.


## Completion sweep — M20 chat-template no-exception status (2026-10-01)

Added `third_party/llama-cpp-patches/0004-nagi-chat-template-status.patch`.
Under `__NAGI__`, chat-template lookup returns the existing explicit
`LLM_CHAT_TEMPLATE_UNKNOWN` status for unknown names, and detection does not
compile exception syntax. Other builds retain the upstream `map::at` failure
and detection catch behavior. The caller in `llama.cpp` rejects UNKNOWN with
`-1`; an unsupported format is not converted into a successful template.

`./nagi fetch` applied and fingerprinted patches 0001–0004 into the generated
llama.cpp checkout while leaving the pinned raw `third_party/llama.cpp` clean.
The overall fetch stopped later because the existing pinned Servo checkout is
dirty; its files remain untouched. The prior generated llama checkout is
preserved at `out/cache/llama-cpp-nagi-before-0004-chat/`.

A focused CLI patch-contract test passed, followed by all 148 CLI unit and 21
integration tests, warnings-denied CLI Clippy, `./nagi fmt`, `./nagi lint`,
`./nagi test`, and `./nagi build`. Host and Nagi-branch chat-template smoke
programs passed, including the target branch compiled with `-fno-exceptions`.
The fresh Nagi CMake target compiled `llama-chat.cpp.obj`. A full keep-going
build still failed in 27 Ninja object steps: 56 distinct source paths report
289 `throw` and 14 `try` diagnostics, down from 28 steps, 57 paths, and 15
`try` diagnostics before patch 0004. The full target is not usable yet; M20
QEMU inference acceptance was not run, and no model or inference result is
claimed. Evidence is in `out/evidence/m20-chat-template-0004-20261001/`; M20
remains `PARTIAL`.


## Completion sweep — current M30 reference-image regression (2026-10-01)

`./nagi m30` passed after the M29 System language persistence change. Two
boots of a disposable copy passed System A, the read-only Model Store
capability, User Data format/write/restart-read, and M7 acceptance. The
separate M20 FAT32 fixture passed its 5,000-byte reader check across a cluster
boundary and EOF. The untouched reference qcow2 remains SHA-256
`e215d62fb19f1bb83c5fb8cbdaf68569fb5cb6a7195bb151e520cd7d1e2801a4`; QEMU
image checks passed for the mutable reference copy and fixture. Evidence and
the verified SHA-256 manifest are in
`out/evidence/m30-release-1790817095155131000/`. The host lacked
`virtio-sound.in`; no host audio playback/capture claim is made. The release
manifest remains `m30_acceptance=NOT_EVALUATED`, and M30 remains `PARTIAL`.


## Completion sweep — 805f2bb M30 clean bundle acceptance (2026-10-01)

Clean-source release preflight, assembly, and verification passed for commit
`805f2bb9c043684f88f25618b45f9949da80ab0f`. The bundle at
`out/artifacts/m30-release-bundle-805f2bb/` contains 23 valid checksums and a
64 GiB qcow2 byte-identical to the accepted reference image. A disposable
byte-identical copy booted twice: System A, read-only Model Store, and the M27
confirmed-A journal passed; the first boot wrote User Data, and the second
read it after restart and printed `Nagi M7 acceptance PASS`. The copy changed
to SHA-256
`29ce659593b9dd9335f4408cbcbdca28c13ca66f3724516f75b646414dbef853`; the
untouched package retained its hash. Post-boot verification, all package
checksums, and `qemu-img check` passed. Evidence and a verified nine-file
manifest are in `out/evidence/m30-clean-release-805f2bb/`. The host lacked
`virtio-sound.in`, so audio I/O was not tested. `m30_acceptance` remains
`NOT_EVALUATED`; M30 remains `PARTIAL` for authenticated updates, M18–M29
gaps, and human binary redistribution review.

## Completion sweep — repeated M28/M27 acceptance and Windows locale fix (2026-10-01)

The fresh two-repetition `NAGI_M28_REPEAT_COUNT=2
tests/acceptance/m28_integration_stress.sh --run` gate passed in both
repetitions. Each passed the real M19 VFS/ObjectId/Search restart gate, all
three M22 grouped NH16 Undo/Activity Ledger boots, and the full M27 GPT
A/B/Recovery gate. M28 evidence and its 20-entry SHA-256 manifest are in
`out/evidence/m28-run-20261001T012645Z-75535/`; M27 sub-run evidence and
37-entry manifests are in
`out/evidence/m27-ab-rollback-1790818018301818000/` and
`out/evidence/m27-ab-rollback-1790818130409530000/`. Existing fixed-name
M19/M22 outputs and both starting User Data disks were preserved under
`out/evidence/pre-m28-m29-persistence-20261001T012622Z/` before the run.

The two-repetition pass covers only the existing Search/Undo/Recovery slice.
The combined Desktop/Files/Notes/Albert load, real Granite, audio playback,
OOM, CPU fairness, and memory/handle leak soak remain unmeasured. QEMU did not
have a host `virtio-sound.in` driver; this acceptance does not claim host audio
I/O. M27 and M28 remain `PARTIAL`.

Windows CI run `36800687313` on `d0ff15c` found that a CRLF resource line left
the terminal carriage return in a Japanese localization value. A regression
test reproduced `Some("設定\r")`; `lookup_resource` now removes one trailing
carriage return from each line before parsing. Six `nagi-localization` tests,
151 CLI unit tests, 21 CLI integration tests, formatting, repository tests,
lint, build, and the `nagi-localization` Nagi-target compile pass locally.

A fresh `./nagi m29` run then passed Japanese selection and restoration on the
same User Data disk. Run `1790818615588652000` is documented at
`out/evidence/m29-settings-1790818615588652000/README.md`; all seven screenshot,
disk, OVMF, and serial-log files verify against its `SHA256SUMS`. The screenshot
SHA-256 is
`974ab722fdc40b855ed97d9ab92c2c69f800c373544fbc7825b2146ef5fbd3cc` and
matches the tracked image. CI run `36802487593` on
`e2b9d48ac9333489d392b1a024c6eee155aa6ea1` then passed all three jobs:
Windows launcher, Ubuntu host, and Nagi target. The target job passed Nagi
user-init and UEFI builds plus the M17 first-web-pixel, M18 chrome/HTTPS, M19
Search, M22 grouped Undo, M27 rollback/Recovery, M29 language-persistence, and
M30 release-disk acceptance gates. The preceding run's unfinished target job
was cancelled by the fix push; that cancellation is not treated as a product
failure. M29 remains `PARTIAL` for the remaining UX, propagation,
localization/accessibility, and license-review work.


## Completion sweep — current-source M30 release bundle (2026-10-01)

On clean commit `5ee985f64da4b01837ef37df39693d35bce09135`, a fresh
`./nagi m30` rebuilt the self-contained 64 GiB GPT reference qcow2. Its SHA-256
is `4155639e8866bff430738d996b40eab350a514777a50a16511701476108f3714`; the
prior image was preserved with its original SHA. System A, read-only Model
Store capability, User Data format/write/restart-read, the M20 5,000-byte FAT32
reader fixture, and M7 acceptance passed. `qemu-img check` passed on all three
images. The run README and verified SHA-256 manifest are at
`out/evidence/m30-release-1790848708675783000/`.

Clean-tree release preflight, assembly, and verification passed for
`out/artifacts/m30-release-bundle-5ee985f/`; all 12 release-tool tests passed.
A byte-identical package copy booted twice and passed System A and Model Store
checks, User Data format/write, restart persistence, and `Nagi M7 acceptance
PASS`. Its post-boot digest changed while the untouched package digest remained
unchanged. Post-boot release verification, package checksums, and qcow2 checks
passed. The README and verified ten-file manifest are at
`out/evidence/m30-clean-release-5ee985f/`. The bundle continues to record
`m30_acceptance=NOT_EVALUATED`; M30 remains `PARTIAL` for authenticated update
installation, M18–M29 gaps, and human binary redistribution review. QEMU had no
host `virtio-sound.in` driver, so audio I/O is not covered. See
`docs/workstreams/NagiOS_M30_Release_Workstream.md`.

## Completion sweep — Settings keyboard navigation (2026-10-01)

The M29 Settings language selector now supports Tab focus on the button
while closed, Tab cycling between locale options while open, Up/Down movement,
Enter/Space activation, Escape close, and a visible focus outline.
The source contract regression failed first on the mouse-only implementation
and passes with the keyboard path. All 152 `nagi-cli` unit tests and 21
integration tests pass, as does the Nagi no-std
`m10-desktop,m29-settings-acceptance` compile.

The first QEMU attempt (`1790849842483344000`) sent pointer input after locale
selection and was rejected because the accepted guest had stopped polling. The
input order now moves the pointer before the final keyboard activation. Fresh
`./nagi m29` run `1790850015194700000` passed Japanese selection, User Data
persistence, restart restoration, and the M10 Desktop interaction markers;
READY took 2,425 ms. The accepted screenshot hash is
`8822118a65187b7e29afcba781c3a659e043263f00a7f823ef1c94aae17d323d`; the
nine-entry evidence manifest verifies at
`out/evidence/m29-settings-1790850015194700000/`. This remains a Settings-only
focus path without a system-wide focus model or accessibility tree, so M29
remains `PARTIAL`.

## Completion sweep — expanded Settings keyboard acceptance (2026-10-01)

The M29 keyboard acceptance now exercises Tab, Enter, Escape, Up, Down, and
Space. A broader input run, `1790850731550869000`, exposed an Up-arrow state
transition error: Up retained Japanese focus, then Down moved to English, and
Space persisted `en-US`. The transition was corrected so either arrow moves
between locale options. Fresh QEMU run `1790850851718829000` passed the expanded
sequence, all existing M10 Desktop markers, language persistence, and restart
restoration. Guest READY arrived after 2,407 ms. Its nine-entry SHA-256 manifest
verifies at `out/evidence/m29-settings-1790850851718829000/`; the screenshot
SHA-256 is `8822118a65187b7e29afcba781c3a659e043263f00a7f823ef1c94aae17d323d`.
After the arrow-transition fix, `./nagi fmt`, `./nagi test`, `./nagi lint`,
`./nagi build`, warnings-denied CLI Clippy, all 152 CLI unit tests and 21
integration tests, the M29 Nagi target build, and QEMU acceptance passed. The
M29 feature remains limited to Settings and does not add a system-wide focus
model or accessibility tree; M29 remains `PARTIAL`.

## Completion sweep — bb43b7f M30 current-source release (2026-10-01)

On clean source commit bb43b7f36eb8d4ccdbceabada720e68b71dc113c,
./nagi m30 built a fresh self-contained 64 GiB GPT qcow2. Its SHA-256 is
ca4c04f5c540bf59cef13b93b301a9fe31134080650977b1a57e760567ce4cff. GPT
System A, read-only Model Store capability, User Data write/restart-read, and
M7 acceptance passed. The separate M20 FAT32 fixture passed its 5,000-byte
cross-cluster and EOF read. qemu-img check passed for the pristine image,
mutable acceptance image, and fixture. The nine-entry evidence manifest
verifies under out/evidence/m30-release-1790851367488764000/.

Release preflight, assembly, and verification passed; all 12 standard-library
release-tool tests passed. The bundle at
out/artifacts/m30-release-bundle-bb43b7f/ contains 23 checksum-covered files
and records the full source revision. Its qcow2 is byte-identical to the
accepted reference image. A byte-identical disposable package copy booted twice
with shared OVMF variables: System A, Model Store read-only, confirmed-A, User
Data format/write, restart persistence, and Nagi M7 acceptance PASS. The
copy changed from SHA-256
ca4c04f5c540bf59cef13b93b301a9fe31134080650977b1a57e760567ce4cff to
474ea42ea2f99c95b98488b425ee6e9b0a9fdee78e69e7ec03a5caddaacb84e9 after
guest writes; the untouched package retained its original digest. Post-boot
release verification, all bundle checksums, both package qcow2 checks, and the
20-entry evidence manifest passed under
out/evidence/m30-clean-release-bb43b7f/.

The previous accepted reference image (4155639e8866bff430738d996b40eab350a514777a50a16511701476108f3714) was preserved before
rebuilding from M29 keyboard-navigation sources. The current package still
records m30_acceptance=NOT_EVALUATED: package integrity and guest QEMU
acceptance are separate facts. M30 remains PARTIAL for authenticated GPT
updates, remaining M18–M29 acceptance, and human binary redistribution review.
QEMU lacked host virtio-sound.in; audio I/O is not covered.

## Completion sweep — current-branch M27 regression (2026-10-01)

After pushing `fa9b73a9c20434b52f414a02f969140b5f2e1ac6`, `./nagi m27`
passed the full A/B and Recovery QEMU acceptance. The GPT malformed-B image
rejected all three trials and rolled back to System A; the healthy-B image
persisted readiness, preserved the journal through Recovery, and promoted B on
the next boot. Recovery verified its VFS and undid the committed M22 move group
across restart. All 37 evidence files, including the seven run-scoped boot
images, verify against
`out/evidence/m27-ab-rollback-1790855312225301000/SHA256SUMS`. M27 remains
`PARTIAL`; authenticated update authority and the remaining Recovery work are
not demonstrated. The host audio warning is unrelated to this M27 acceptance.

## Completion sweep — current-source M30 release (2026-10-01)

On clean commit `9db7e0f8d083c7d7ef32d641f7a4c48c45a598bb`, `./nagi m30`
rebuilt a self-contained 64 GiB GPT reference image with SHA-256
`cb63509d4d221320cf2dec1637b5acdcf662cd22e6436717660185389624d5f5`. The
QEMU copy passed System A, Model Store capability checks, first-boot User Data
format/write, and persistent read on restart; the M20 fixture passed its
5,000-byte FAT32 boundary/EOF read. Clean-tree release preflight, assembly,
verification, and all 12 release-tool tests passed. The bundle image is
byte-identical to the QEMU-tested reference image, and the release bundle
checksums verify. All 15 cross-run files verify against
`out/evidence/m30-release-1790855765560052000/SHA256SUMS`. The prior reference
image is preserved and hash-verified under
`out/evidence/m30-current-source-rebuild-pre-9db7e0f/`.

The release manifest remains `m30_acceptance=NOT_EVALUATED`; M30 stays
`PARTIAL` pending authenticated GPT updates, remaining M18–M29 acceptance, and
human binary redistribution review. The host audio warning is outside these
checks.

## Completion sweep — conflict-safe M27 Recovery Undo (2026-10-01)

The Recovery Undo path now preflights the whole inverse batch before writing
`UndoPending`. A conflict leaves files and the persisted NH16 transaction
unchanged. Retry selection gives persisted `UndoPending` work priority over a
later `Committed` transaction, and accepts already-inverted actions so a
restart can finish a partially applied batch. Edit preflight borrows NH16's
recorded forward snapshot by sequence instead of copying it into the undo
batch. MoveBack can only check path occupancy because Move records do not
contain content identity.

On source commit `b70a1ec`, `./nagi m27` passed the complete
A/B, readiness-promotion, GPT, and Recovery acceptance in fresh run
`out/evidence/m27-ab-rollback-1790857169644407000/`. Its guest fixture created
a conflict in the last action of the three-file MoveBack batch and verified
that the earlier actions stayed forward and NH16 remained `Committed`. It
then persisted `UndoPending`, applied one inverse, restarted the Recovery
operation, and verified all actions reached `Undone`; the following M22
restart verified the durable result. All 38 evidence and run-image entries
verify against that run's `SHA256SUMS`.

A preceding fresh attempt, `out/evidence/m27-ab-rollback-1790856979486898000/`,
passed the new Recovery conflict/retry fixture and its restart check but
timed out after 90 seconds during the first A/B trial before Nagi guest code
ran. QMP recorded a running VM looping in the UEFI firmware; the attempt is
preserved, not counted as a full M27 pass. The fresh run with new OVMF state
passed. The failed attempt's 14-entry manifest and successful attempt's
38-entry manifest both verify. QEMU reported the host `virtio-sound.in` driver
unavailable; M27 does not exercise audio I/O.

Focused verification passed: 17 `nagi-history` tests; 152 `nagi-cli` unit and
21 integration tests; and a Nagi target `cargo check` with
`m27-recovery-undo-acceptance`. M27 remains `PARTIAL` for authenticated slot
manifests/update installation, full authenticated session readiness, Move
content identity, and the remaining Recovery repair/log operations. See
`docs/workstreams/NagiOS_M27_AB_Recovery_Workstream.md`.

## Completion sweep — M19/M22 regression after Recovery hardening (2026-10-01)

`./nagi m22` passed after the History and Recovery changes. Boot 1 passed M19
Search persistence and created the real M21/M22 guest Move and Copy records;
boot 2 applied grouped Undo and persisted its result; boot 3 verified restored
files, NH16/NAL1 state, and semantic-index persistence. Its eight-file
SHA-256 manifest verifies at
`out/evidence/m22-history-1790857596655250000/SHA256SUMS`. The repository-wide
`./nagi fmt`, `./nagi lint`, `./nagi test`, and `./nagi build` commands also
passed after the changes.

## Completion Sweep — M20 pin, M27 marker, and M28 evidence (2026-10-01)

`third_party/models.lock` now carries the exact Granite 4.2 3B GGUF source
revision, filename, size, digest, Apache-2.0 notice/acknowledgement, and Model
Store artifact identity. The host regression
`granite_model_artifact_lock_matches_manifest_fixture` compares all these
fields with `granite-4.2-3b.json`. The focused test passes. This does not
download or install the model; the M30 Model Store remains empty, the full
Nagi-target llama.cpp build still fails on exception-dependent code paths,
and no guest inference is claimed. M20 remains `PARTIAL`.

The M27 bootstrap runner now waits for `Nagi M7 reboot required PASS` after the
earlier persistent-write marker, and its host test checks that both are
present. A fresh two-repetition M28 run completed M19 Search/ObjectId, all
three M22 Move/Copy/NH16/NAL1 Undo boots, and full M27 GPT A/B/Recovery in
both repetitions. The run archive at
`out/evidence/m28-run-20261001T130826Z-2523/` has 28 verified SHA256 entries;
M27 runs `1790860121579655000` and `1790860238754593000` each have 31 verified
entries. A further one-repetition QEMU run passed all three gates at
`out/evidence/m28-run-20261001T132819Z-4650/`; its initial evidence-finalizer
exit was traced to a newline/`set -e` interaction, corrected in the harness,
and its manually finalized 15-entry manifest verifies.

The M28 runner now selects M22's run-stamped final serial log, archives the
last repetition as well as earlier ones, writes README/SHA256SUMS manifests,
and preserves the active repetition on a failed gate. `sh -n`, its marker,
run-ID and evidence-manifest self-tests, `--dry-run`, and `git diff --check`
pass. Follow-up QEMU attempts encountered intermittent OVMF startup loops at
RIP `0x7eb84171` during M27 Recovery, M22 bootstrap, and M22 boot 1. Their
partial outputs, QMP diagnostics, and manifests are preserved under
`out/evidence/m28-run-20261001T131752Z-3830/`,
`out/evidence/m28-run-20261001T133248Z-5339/`, and
`out/evidence/m28-run-20261001T133549Z-5668/`; these failures are not counted
as acceptance passes. Their root cause remains unknown. M28 remains `PARTIAL`
because the desktop/browser/model/audio/OOM/fairness/leak stress workload is
still unmeasured. M27 remains `PARTIAL` for authenticated update/readiness and
remaining Recovery work.

Priority A audit findings remain cross-cutting: the kernel has bounded
generational handles, rights attenuation, and channel transfer escrow. The
user ABI now exposes bounded Channel create/send/receive/close and
`SYS_CHANNEL_WAIT_READABLE`, but the bootstrap manager still owns only PID 1
and provides no live authenticated multi-process service boundary. Bootstrap
does not bind application/session identities to per-process handle delivery.
Consequently M18's picker/clipboard/IME/site-permission providers and M23 live
Browser Context cannot yet be safely wired as production providers.
M19's guest Search Service and M21 `file.search` are real guest VFS paths but
use fixture-only callers/policy; M21/M22 Move/Copy and NAL1 ledger restore are
also acceptance-fixture scoped. Do not promote these items to production
service acceptance. The M18–M23 cross-cutting service boundary remains open.

Verification on the current worktree: the focused Granite contract test passed;
the complete CLI suite passed with 154 unit and 21 integration tests;
`./nagi fmt`, `./nagi lint`, `./nagi test`, and `./nagi build` passed. Cargo
continues to emit three pre-existing `target_os = "nagi"` check-cfg warnings
from vendored `libc`.

## Completion Sweep — M19 file.search Activity Ledger record (2026-10-01)

M19 now returns a bounded `M19SearchActivity` only after the real fixture
`file.search` plan succeeds through ContextResolver, Validator, Action Registry,
and Executor. M13 passes that result to M22. On the first M22 boot, the M21
Move handler stores a separate NAL1 `file.search` record with no transaction
ID, `Prepared` then `Committed` results, the actual matched Object ID, and the
M19 fixture App/Session/Node/Workspace context. Boots 2 and 3 verify exactly
one matching record without appending duplicates; the NAL1 archive has room
for the Search, Move, and Copy records within its four-record bound.

The regression test exposed that NAL1 length calculation counted absent
optional IDs as nine bytes. It now uses each optional ID's encoded length, so
records with no Surface ID and no Transaction ID serialize and restore
correctly. `cargo test -p nagi-history` passes all 18 tests. The complete CLI
suite passes 154 unit and 21 integration tests; `nagi-init` M22 Nagi-target
check and warnings-denied package Clippy pass. `./nagi fmt`, `./nagi lint`,
`./nagi test`, and `./nagi build` pass. A fresh `./nagi m22` QEMU run passed all
three boots; its nine-file SHA256 manifest, including the exact source diff
from base `b9e2ee5`, is at
`out/evidence/m22-search-activity-1790863264739984000/`.

`tests/acceptance/m28_integration_stress.sh` now requires the new marker and
its self-test/dry-run pass. Two two-repetition M28 attempts were made after this
slice. In `out/evidence/m28-run-20261001T140924Z-9410/`, M19 passed and M22
reached boot 3 after passing the first two boots, but QEMU remained running at
RIP `0x7eb84171` until the 90-second timeout. The harness initially omitted
that final diagnostic from its archive; the log was added and the archive's
13-entry manifest regenerated and verified. The harness now derives and
archives the run-ID-specific M22 boot-3 path, with the path covered by its
self-test.

The next run, `out/evidence/m28-run-20261001T141428Z-9953/`, completed one full
M19/M22/M27 repetition. M22's three boots each verify Search, while M27
passed malformed System B rollback, healthy System B promotion, Recovery, and
M22 group Undo. Repetition 2 passed M19, then M22 bootstrap timed out after 90
seconds with the same QEMU RIP loop. All archive files and the M27 sub-run
manifest verify. A standalone `./nagi m22` rerun then passed all three boots.
The two-consecutive-repetition M28 acceptance therefore remains unfulfilled;
the failures match the intermittent pre-guest OVMF loop seen in other runs.
M19, M21, M22, and M28 remain `PARTIAL`; the NAL1 path is still an acceptance
fixture and supplies no production caller authentication.

## Completion Sweep — M18 QEMU timeout diagnosis (2026-10-02)

A fresh `./nagi m18` rerun reached its 1200-second timeout with QEMU still
running. The serial log has no Nagi boot or M18 marker; QMP repeatedly sampled
RIP `0x7eb84171` in a jump loop, and the saved VNC frame shows the TianoCore
firmware splash. The cause is undetermined. The serial/QMP logs and PNG/PPM
frames have a verified `SHA256SUMS` under
`out/evidence/m18-timeout-1790866733768047000/`. Earlier local and Ubuntu CI M18
HTTPS acceptance remains the successful evidence; this timeout is not a pass.

## Completion Sweep — M25 fixture transcript handoff (2026-10-02)

The fixture provider can now return an explicitly configured fixed Japanese
transcript through `PushToTalkService::finish_into`. The host regression checks
the bytes, clears the unused output tail and internal PCM buffer, hides the
indicator, and ends the capture lifecycle. The target acceptance checks the
transcript bytes and output tail, indicator state, capture/forwarded-byte
counts, and cleanup state, then emits
`Nagi M25 fixture transcript delivery PASS`. This verifies only orchestration;
the fixture phrase is not recognized speech and is not executed as a command.

Verification passed: the focused host regression, all 15 `nagi-audio` tests,
all 155 `nagi-cli` unit and 21 integration tests, `./nagi fmt`, `./nagi test`,
`./nagi lint`, `./nagi build`, and a fresh `./nagi m25` QEMU run. The run-stamped
image, User Data disk, OVMF variables, bootstrap log, and voice log have a
verified manifest at
`out/evidence/m25-fixture-transcript-1790868293848130000/manifest.sha256`.
QEMU has no host `virtio-sound.in` driver; the fixture does not use host audio.
No microphone, real STT inference, TTS engine, authenticated permission adapter,
or spoken-command execution was exercised. M25 remains `PARTIAL`.

## Completion Sweep — M30 current-source release rebuild (2026-10-02)

The fixed-path release qcow2 was first confirmed to be from old commit `9db7e0f`
by its SHA-256, then copied and verified at
`out/evidence/m30-pre-current-rebuild-39234a6/`. With that prior generated
artifact preserved and the fixed path clear, `./nagi m30` rebuilt the
self-contained 64 GiB GPT image from clean current source `39234a68208fe849461904f5a68cea1daae2f772`.
The pristine qcow2 SHA-256 is
`839eee861a444f2dea447c1f0b5a9dd2a0db4672fa2d8c7ee6290ab5cc3648cf`.

QEMU passed System A, the read-only Model Store capability, User Data format
and write, persistent read after restart, and M7 acceptance. The separate M20
fixture passed a 5,000-byte FAT32 read over a cluster boundary and EOF. The
M30 run image, QEMU copy, logs, variables, fixture, and provenance manifests
have a verified 13-entry `SHA256SUMS` at
`out/evidence/m30-release-1790868798244504000/`.

Release preflight, assembly to `out/artifacts/m30-release-bundle-39234a6/`,
release verification, all 12 release-tool tests, package SHA-256 checks, and
`qemu-img check` passed. The release package image matches the pristine QEMU-
tested image byte-for-byte. Its `m30_acceptance` field remains
`NOT_EVALUATED`; M30 stays `PARTIAL` for authenticated updates, remaining
M18–M29 acceptance, and human binary redistribution review. QEMU has no host
`virtio-sound.in` driver, so audio I/O remains untested.

## Completion Sweep — M27/M28/M29 QEMU regressions (2026-10-02)

Two fresh two-repetition M28 attempts are recorded at
`out/evidence/m28-run-20261001T153942Z-38390/` and
`out/evidence/m28-run-20261001T154452Z-38953/`. M19 passed in both. In the
first, M22 boot 1 passed but boot 2 timed out before guest acceptance; in the
second, M22 bootstrap timed out before guest acceptance. QMP recorded the same
OVMF RIP `0x7eb84171`; the 13- and 11-entry manifests verify. Neither run
completed one repetition, so M28 remains `PARTIAL`.

Fresh standalone `./nagi m22` passed all three boots at
`out/evidence/m22-standalone-1790869348631156000/` (8-entry manifest). Fresh
standalone `./nagi m27` passed rollback, healthy-slot promotion, Recovery, and
cross-restart M22 Move Undo at
`out/evidence/m27-ab-rollback-1790869370028356000/` (38-entry manifest).
`./nagi m29` passed keyboard focus for Settings and all four M10 panels,
`ja-JP` selection and persistence, and same-disk restoration with READY in
2,605 ms; its screenshot, logs, image, and disk are covered by the eight-entry
manifest at `out/evidence/m29-settings-1790869645429869000/`. M27 and M29 remain
`PARTIAL` for their previously recorded production/security and broader
release-quality acceptance gaps.

## Completion Sweep — M20/M21 structured output and M22 regression (2026-10-02)

`ModelRequest` now carries an optional validated `StructuredOutputSchema`.
Structured generation requires the `structured.generate` capability and a
schema; runtime checks backend JSON for duplicate keys, a bounded schema subset,
and a 64 KiB output cap before exposing the response. The schema document is
capped at 16 KiB. A global walk limits output depth, nodes, container sizes, and
string bytes even below unconstrained schema fields. Structured streaming
returns `UnsupportedCapability` because chunks would be observable before
whole-document validation. The schema parser accepts the authoritative
`schemas/NagiPlan@1.json` using fixed recognizers for its three bounded patterns;
unsupported schema keywords fail closed. M21's `ModelManagerPlanAdapter`
supplies and independently rechecks that schema at its generic provider
boundary, using an output-token request budget of 1,024 to match the current
bundled model manifests, while retaining the independent Planner and Validator
checks.

Host verification passed: `nagi-model-manager` 57 unit + 2 manifest/schema + 1
Store API tests; `nagi-ai` 24 tests; warnings-denied Clippy; Nagi no-std target
compilation for both packages; and `./nagi fmt`, `./nagi test`, `./nagi lint`,
and `./nagi build`. A fresh three-boot `./nagi m22` regression passed M21
validation and guest VFS actions, NAL1/NH16 persistence, grouped Undo, and
restart restoration. Its eight-entry manifest verifies at
`out/evidence/m22-structured-output-regression-1790871324761891000/`. The
fixture does not execute a model backend. M20–M22 remain `PARTIAL` for the
previously recorded inference, service-boundary, authenticated production
authority, and general action gaps.

## Completion Sweep — M20 grammar status propagation (2026-10-02)

Numbered llama.cpp patch `0005-nagi-grammar-status.patch` makes grammar parser
errors explicit, checks numeric overflow and repetition bounds, rejects
malformed escapes and undefined rules, and latches a runtime grammar failure
so candidate application fails closed. Sampler failure returns
`LLAMA_TOKEN_NULL`. Existing regex-triggered lazy grammar support is retained;
invalid upstream `std::regex` compilation under Nagi's no-exceptions runtime
remains unverified.

`./nagi fetch`, three focused host CTest targets, the CLI patch-contract test,
and the Nagi-target `llama-grammar.cpp` translation unit compile passed. The
latest full target attempt still fails in 26 object targets across 55 distinct
source files; its log is `out/logs/m20-grammar-status-target-build-20261002.log`.
M20 remains `PARTIAL`, with no complete llama backend or in-guest inference.

## Completion Sweep — Bootstrap Channel user ABI (2026-10-02)

Added `SYS_CHANNEL_CREATE`, `SYS_CHANNEL_SEND`, nonblocking
`SYS_CHANNEL_TRY_RECEIVE`, and `SYS_HANDLE_CLOSE` after the existing syscall
numbers. The fixed-size `repr(C)` ABI caps payloads at 128 bytes, transfers at
four handles, and each directional queue at eight messages. `libnagi` exposes
typed wrappers. Kernel syscall handlers require exact struct sizes and mapped
user ranges, copy bounded data through kernel-owned values, and get sender PID
from the kernel Process rather than a payload field.

The bootstrap manager is bounded to 16 live Channel pairs and 64 handles for
the single `nagi-init` Process (PID 1). Rights bits come from `nagi-abi`,
unknown bits and nonzero reserved transfer fields fail closed, and handle
transfer uses the existing Channel escrow and attenuation checks. Closing
handles traces active handles and queued escrow references, drains unreachable
channels, and reuses pair/object/handle slots, including a tested cross-channel
escrow cycle. The API has no blocking wait and does not authenticate an app or
service; M19 and M21 production authorities, M18 providers, M22 actions, and M23
browser-context services remain incomplete.

Verification on 2026-10-02 passed five `nagi-abi` tests, all 132
`nagi-kernel` tests (including five new user Channel manager tests), and 38
`libnagi` tests on x86_64 macOS. Warnings-denied `libnagi` Clippy passed. The
kernel library Clippy check passed with existing `needless_range_loop`,
`new_without_default`, and `too_many_arguments` lints allowed; the two new
manager findings were fixed. `./nagi fmt`, `./nagi lint`, `./nagi test`, and
`./nagi build` passed. A fresh `./nagi m19` QEMU acceptance passed
the `Nagi bootstrap Channel ABI PASS` marker and the existing guest Search,
ObjectId rename, persistence, and restart checks; the serial log is
`out/logs/m19-vfs-objectid-initial.log`. This run had no host
`virtio-sound.in` backend, unrelated to Channel acceptance. It does not change
M19–M23 from `PARTIAL` or establish authenticated service IPC.

A fresh-disk `./nagi m22` regression passed all three boots after the manager
changes. Each boot printed the bootstrap Channel marker; the run also passed
M19 Search persistence, M21 fixture validation, M22 Move/Copy transactions,
NAL1/NH16 persistence, composite Undo, and restored-file verification. Its
run-stamped image, User Data disk, OVMF variables, and logs are under
`out/artifacts/nagi-0.1-m22-history-1790877967540236000.img`,
`out/artifacts/nagi-0.1-m22-history-user-data-1790877967540236000.img`,
`out/artifacts/nagi-0.1-m22-history-vars-1790877967540236000.fd`, and
`out/logs/m22-history-1790877967540236000-boot-{1,2,3}.log`. M21/M22 remain
`PARTIAL` because these policies and actions are fixture-scoped and use no
model inference or authenticated production service boundary.

## Completion Sweep — M20 sampler failure sentinel consumers (2026-10-02)

Added numbered patch `0006-nagi-sampler-null-consumers.patch` to document the
public `llama_sampler_sample()` failure result (`LLAMA_TOKEN_NULL`) and guard
all six direct C++/Swift example callsites before EOG classification, token
conversion, or later-batch submission. The CLI patch-contract test checks that
each callsite both checks and reports the sentinel.

The patch passes `git apply --check` against the clean pinned llama.cpp
revision. All 157 `nagi-cli` unit tests and 21 integration tests pass. A host
CMake build passed the `llama-simple`, `llama-simple-chat`, `llama-batched`, and
`llama-passkey` targets; both modified Swift files passed `swiftc -typecheck`.
`cargo fmt --all -- --check` passed. `./nagi fetch` did not reach llama.cpp
because the existing generated Servo checkout failed its patch-state
validation; that checkout was left untouched. M20 remains `PARTIAL`, and no
Nagi inference or backend is claimed.

## Completion Sweep — M20 hybrid state-restore rollback (2026-10-02)

Added numbered patch `0007-nagi-hybrid-state-restore-rollback.patch` to extend
llama.cpp's existing state-restore failure suite for generated hybrid models.
The new case truncates the serialized recurrent suffix after attention restore
and checks that the failed restore leaves the sequence empty, returns its
serialized state size to the empty baseline, and preserves another sequence's
logits. The patch applies cleanly to the pinned llama.cpp
revision. `test-save-load-state` and `test-llama-archs` built on the host; a
generated `granitehybrid-dense.gguf` ran all nine state tests, including the
new recurrent-suffix failure case, successfully. Its log is
`out/logs/m20-hybrid-state-restore-20261002.log`. This verifies rollback
behavior in the host test harness only; exception-based runtime error
propagation and Nagi-target inference remain unresolved. M20 remains
`PARTIAL`.

## Completion Sweep — M22/M27 Move content identity (2026-10-02)

NH16 grouped Move records now store SHA-256 of each source file in the existing
`before` snapshot bytes, preserving the archive version and record layout.
Recovery MoveBack preflight hashes the file at the forward or inverse path and
rejects same-name replacement content before persisting `UndoPending`. Empty
digests remain supported for older NH16 Move records with the former path-only
check.

Verification passed: 20 `nagi-history` tests, 158 `nagi-cli` unit tests, 21
CLI integration tests, warnings-denied Clippy for both packages, `cargo fmt`
check, the Nagi-target release build of `nagi-init` with
`m27-recovery-undo-acceptance`, and fresh three-boot `./nagi m22` with restored
files. M27 Recovery printed the same-path content-conflict marker and passed
that subtest. The subsequent full `./nagi m27` rerun timed out before guest
output on the Recovery boot with pending System B; QMP again identified the
OVMF loop at RIP `0x7eb84171`. The run is preserved under
`out/evidence/m27-ab-rollback-1790881641572651000/`; this is not a full M27
acceptance pass. M22 and M27 remain `PARTIAL` for their recorded production
service, authority, update, and broader Recovery gaps.

## Completion Sweep — M30 source-bound image acceptance (2026-10-02)

Completed the current-source rerun after adding image provenance enforcement.
On clean commit `567afa42ed5b3f6f374b176b11f4a6524175eeb2`, `./nagi m30`
rebuilt the 64 GiB GPT qcow2 with SHA-256
`54390507a4e975ad30ee94d7efb7b4c81758854ccbbdcc7f12b39bfb70fc6748`. Its
`.build-info` sidecar binds that digest to the full source revision. QEMU
passed System A, read-only Model Store capability, User Data format/write and
restart-read, and M7 acceptance; the separate M20 fixture passed its 5,000-byte
FAT32 read across a cluster boundary and EOF. `qemu-img check` passed on the
pristine image, mutable acceptance copy, M20 fixture, and release-bundle copy.

All 14 release-tool tests passed. On the clean source, preflight, assembly to
`out/artifacts/m30-release-bundle-567afa4/`, verification, all 21 package
checksums, and byte-identity of the bundled and pristine images passed. The
pre-provenance bundle `m30-release-bundle-5596f43` also still verifies. The
10-entry evidence manifest at
`out/evidence/m30-release-1790883362672919000/SHA256SUMS` verifies and contains
the pristine qcow2 and matching sidecar. QEMU had no host `virtio-sound.in`
input driver, so host audio I/O is not covered. The release manifest keeps
`m30_acceptance=NOT_EVALUATED`; M30 remains `PARTIAL` for authenticated
updates, remaining M18–M29 acceptance, and human binary redistribution review.

## Completion Sweep — M30 GPT Recovery partition acceptance (2026-10-02)

Extended `./nagi m30` to boot Recovery from the Recovery partition in the same
GPT qcow2 used for System A and User Data acceptance. The gate now requires
Recovery VFS and help markers, confirms that manual Recovery selection leaves
the A/B boot journal unchanged, then restarts and verifies confirmed System A
and persistent User Data. It does not stage System B or claim update acceptance.

On clean source commit
`74369f7997bff8b877980841ecd9bcb03ae66f06`, fresh `./nagi m30` passed System A,
User Data format/write/restart-read, Recovery selection and console, the
post-Recovery System A boot, M7, and the separate M20 5,000-byte FAT32 fixture.
The source-bound pristine qcow2 SHA-256 is
`54390507a4e975ad30ee94d7efb7b4c81758854ccbbdcc7f12b39bfb70fc6748`; `qemu-img
check` passed for the pristine, mutable, fixture, and bundled images. All 14
release-tool tests, preflight, assembly, verify, 23 bundle checksums, and
byte-identity passed. The 18-entry evidence manifest verifies at
`out/evidence/m30-release-1790885396024787000/SHA256SUMS`; package output is
`out/artifacts/m30-release-bundle-74369f7/`.

QEMU reported no host `virtio-sound.in` input driver, so host audio I/O is not
covered. The package manifest retains `m30_acceptance=NOT_EVALUATED`; M30
remains `PARTIAL` for authenticated/System B updates, remaining M18–M29
acceptance, and human binary redistribution review.

## Completion Sweep — M30 quiescent restart and full guest acceptance (2026-10-02)

The first fresh M30 attempt, run `1790885889238875000` on source `d814db3`,
failed Recovery's read-only VFS check after its restart runner stopped at
`Nagi M7 acceptance PASS`, before M13/M19/M22 had completed their User Data
writes. Forensics found an allocated but incomplete M13 fixture inode and an
ext2 free-inode count mismatch; Recovery correctly failed closed. The failure
logs, pristine image, and mutable copy are preserved under
`out/evidence/m30-release-1790885889238875000/` with a verified manifest. The
runner now waits for `Nagi M13 acceptance PASS`, serves the M13 HTTP fixture,
and requires M19/M22 markers before proceeding (commit `644d0bb`).

The next run (`1790886615318484000`) passed Recovery and unstaged-System-B
rejection but exposed that the post-Recovery System A boot also needs the M13
HTTP fixture. Its log contains `Nagi M13 acceptance FAIL`. The old check looked
only for earlier M7 markers, so the interrupted investigation produced a
misleading `PASS M30` summary; this run is explicitly not counted. All images,
logs, QEMU variables, M20 fixture, and copied bundle metadata are preserved
under `out/evidence/m30-release-1790886615318484000/` with a verified
17-entry manifest. Commit `b66fabb` starts the fixture for the post-Recovery
boot and requires M13 completion and M19/M22 Search/Undo markers.

On clean source commit `b66fabb1388e67eb4e35fa9cf72d231e61bf097f`, fresh
`./nagi m30` run `1790886957142079000` passed System A, User Data format/write
and restart-read, M19 Search, M22 grouped Move/Copy and Activity Ledger,
Recovery VFS/help with an unchanged A/B journal, explicit rejection of
unstaged System B, post-Recovery System A persistence and M22 Undo, M13
completion, and the separate M20 5,000-byte FAT32 fixture. The Recovery check
reported `files=20 directories=5`. `qemu-img check` passed for pristine,
mutable, fixture, and assembled bundle images. All 14 release-tool tests,
preflight, assembly, verification, 23 bundle checksums, and image
byte-identity passed. The 17-entry evidence manifest verifies at
`out/evidence/m30-release-1790886957142079000/SHA256SUMS`; the bundle is
`out/artifacts/m30-release-bundle-b66fabb/`.

The bundle retains `m30_acceptance=NOT_EVALUATED`. QEMU had no host
`virtio-sound.in` driver, so host audio input is untested. M30 remains
`PARTIAL` for authenticated updates and System B acceptance, remaining
M18–M29 work, and human binary redistribution review.

## Completion Sweep — Priority A Service, IPC, and Capability audit (2026-10-02)

The existing foundations were inspected before considering integration across
M18, M19, and M21–M23. The bootstrap Channel path exposes bounded create, send,
nonblocking receive, close, and `SYS_CHANNEL_WAIT_READABLE` syscalls.
`kernel/src/user_ipc.rs` initializes one `Process` with PID 1 in one address
space; it stamps Channel sends with that process identity and checks
attenuated handle transfers. Channel readability wait is implemented and
covered by the M19 QEMU fixture; generic event/timer `wait_many` is not
published. This demonstrates ABI and capability mechanics, not an
authenticated inter-process service boundary.

`libnagi::ServiceRegistry` resolves manifests to local function pointers and
invokes handlers in the same process. It has bounded capacity and health
states, but no endpoint transport, authenticated client identity, or
supervisor-authorized launch binding. The M19 Search provider and M21/M22
fixture caller contexts therefore remain orchestration inputs, not authority.
M19 runs a real guest VFS Search fixture and M21 executes `file.search` through
its validator/executor; M22 persists the resulting typed Search event and
fixture `file.move`/`file.copy` transactions in NH16/NAL1 across restarts.
These guest results do not establish production Files/page producers,
authenticated Action Registry callers, or a production Activity Ledger
service. General copy/move, rename, metadata-update, and app-launch handlers
remain absent.

Albert's clipboard, IME composition, upload/picker, and site-permission code
has typed state/provider interfaces, but no Nagi service implementation is
connected. M18's embedder explicitly denies site requests when no trusted
prompt service exists and reports clipboard and IME unavailable; upload
selection depends on an injected picker adapter. This preserves fail-closed
behavior and does not provide interactive permission, clipboard, text/IME, or
opaque file-handle providers.

The shared blocker is architectural: isolated process/address-space launch,
supervisor-authorized endpoint delivery bound to kernel process identity and a
launch record, and service adapters that receive capabilities rather than
caller-supplied identity fields are not in place. Channel readability wait
exists, but does not supply those missing process and launch authorities.
Implementing only a policy callback or treating a PID/App ID in a request as
authentication would weaken the stated capability boundary. Keep the existing
M18–M23 guest fixtures as orchestration evidence until that trusted service
path exists. M18, M19, and M21–M23 remain `PARTIAL`; M22's persistent fixture
acceptance remains valid but does not change that status. No milestone was
added or promoted by this audit.

## Completion Sweep — M28 M27 timeout evidence and replay (2026-10-02)

The M28 failure-path parser previously extracted only the `/out/evidence/...`
suffix from an absolute QEMU log path, while its path check expected either the
full repository path or a relative `out/evidence/...` path. Thus a failed M27
sub-run could be omitted from the M28 README and left without its own
README/checksum. It now extracts the run ID and reconstructs the repository-
relative evidence directory. The M28 self-test covers both absolute and
relative diagnostic paths. Shell syntax, self-test, and dry-run pass.

Fresh two-repetition run
`out/evidence/m28-run-20261001T205917Z-97811/` passed M19 Search and the full
three-boot M22 grouped-Undo path in both repetitions. Repetition 1 also passed
M27 rollback, promotion, Recovery, and committed M22 Undo. Repetition 2's M27
Recovery GUI stage timed out after 90 seconds before a guest marker. QMP showed
OVMF looping at RIP `0x7eb84171` (`jmp 0x7eb84150`). The corrected harness
recorded the failed M27 run `1790888490534092000` in the M28 README and
generated its own README/SHA-256 manifest. The archive has 26 hashed files;
the passing M27 sub-run has 31, and the failed one has 8.

The exact failing Recovery boot image, User Data, and OVMF variables were
copied to `out/evidence/m27-replay-recovery-gui-1790888490534092000/`. That
state subsequently reached the Recovery menu; the replay sent the `r` key,
sent the full serial command batch, and passed `Nagi M27 Recovery command help
PASS`. This supports an intermittent pre-guest OVMF startup failure; it does
not convert the original M28 repetition to a pass. The earlier M28 run
`out/evidence/m28-run-20261001T204558Z-96383/` also failed M27 before its M13
fixture marker in repetition 2; replay from its exact boot image and User Data
with a fresh copy of the pinned OVMF variable template reached M13, M21, and
M22 markers. Both original attempts remain failed evidence, and M28 remains
`PARTIAL`.

## Completion Sweep — M25 pinned Whisper model download (2026-10-02)

Downloaded the Whisper small multilingual artifact from the immutable
repository revision recorded in `third_party/models.lock` to
`out/cache/whisper-models/ggml-small-5359861c739e955e79d9a303bcbc70fb988958b1.bin`.
The file size is exactly 487,601,967 bytes and its SHA-256 is
`1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b`, matching
the lock. `out/evidence/m25-whisper-model-download-20261002/SHA256SUMS`
verifies both this cached model file and the evidence README.

This only establishes that the pinned artifact is available in the local
ignored cache. It has not been installed into the guest Model Store, loaded by
the Nagi target, or used for inference. M25 remains `PARTIAL`; authenticated
permission/UI, a connected Japanese STT provider, inference, local TTS, and
spoken-command acceptance remain.

## Completion Sweep — M20 Granite artifact and patch 0007 checkout (2026-10-02)

Downloaded the pinned Granite 4.2 3B Q4_K_M artifact from the immutable
revision in `third_party/models.lock` to
`out/cache/models/granite-4.2-3b-Q4_K_M-c40945d71cd90f249a56985e8155551a9188dc30.gguf`.
The file is exactly 2,244,011,552 bytes with SHA-256
`e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5`, matching
the lock. The GGUF magic is present. The local evidence manifest at
`out/evidence/m20-granite-model-download-20261002/SHA256SUMS` covers the
README and model bytes. The binary is in ignored cache only; it is not present
in a Model Store image and has not been loaded or used for inference.

The previous generated llama.cpp checkout, bound to patches 0001–0006, was
preserved at `out/cache/llama-cpp-nagi-before-0007-20261002/`. A fresh
`./nagi fetch` generated `out/cache/llama-cpp-nagi` with patches 0001–0007 and
its matching revision/patch/tree fingerprints. The command validated this
llama.cpp checkout and proceeded through the other earlier fetch components,
then stopped with exit 4 at the pre-existing mismatched generated Servo state
in `third_party/servo`; Servo was left untouched. Log:
`out/logs/nagi-fetch-m20-0007-20261002.log`.

This advances artifact availability and reproducible patch application only.
The fresh patch-0007 host state test built and passed all nine cases using a
generated hybrid fixture; this verifies rollback behavior on the host only.
The first target-build invocation omitted `NAGI_CXX_HEADERS` and stopped on
missing C++ standard headers. Re-running with the repository's target wrapper,
Homebrew LLVM 19, and its matching libc++ headers exposed the actual current
blocker: 26 object targets fail with no-exception `throw`/`try` diagnostics
across 55 source paths. The complete output is
`out/logs/m20-target-build-patch0007-cxx19-headers-20261002.log`; host test
output is `out/logs/m20-hybrid-state-restore-patch0007-20261002.log`, with its
generated state artifact preserved under
`out/evidence/m20-hybrid-state-restore-patch0007-20261002/`.

The regular M30 release Model Store remains empty, no model service loads
Granite, and no inference has run. M20 remains `PARTIAL`.

## Completion Sweep — M20 Granite guest artifact digest acceptance (2026-10-02)

Added `./nagi m20-granite <artifact.gguf>`. It checks the external artifact's
file type, exact size, and SHA-256 against the checked-in model manifest and
`third_party/models.lock`, then streams the file into a unique disposable
reference disk without buffering the model in host memory. QEMU booted System A
and the guest read all 2,244,011,552 bytes through the read-only Model Store
capability. Serial markers `Nagi M20 Granite artifact digest PASS` and
`Nagi M20 Model Store capability PASS` were present. The digest matched
`e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5`, and
`qemu-img check` reported no errors. Evidence manifest and files are under
`out/evidence/m20-granite-artifact-1790892878741511000/`.

The CLI host suite passed 163 unit and 21 integration tests. Model Manager
passed 58 unit tests, 2 manifest/schema tests, and 1 Store API test. The
`m20-granite-artifact-acceptance` init feature compiled for the Nagi target.
The model appears only in this disposable acceptance image; no model backend
was loaded and no inference was performed. The regular M30 image remains
empty. M20 remains `PARTIAL`.

## Completion Sweep — current-source M30 regression after M20 artifact acceptance (2026-10-02)

On clean source commit `0e7756336cc0f05d28734633df2a9eac12557b5c`, fresh
`./nagi m30` run `1790893780350727000` passed GPT System A, User Data
write/restart-read, M19 Search, M22 Move/Copy and Activity Ledger, Recovery
VFS/help with the A/B journal unchanged, rejection of unstaged System B, and
post-Recovery System A with persisted M22 Undo. The separate M20 fixture passed
its bounded FAT32 cross-cluster/EOF read. Independent `qemu-img check` passed
for the pristine release image, mutable M30 acceptance copy, and separate M20
fixture. The release image SHA-256
`ff944ba9113ae51ad69de31ead2b5b9765444899561adcf664ad38042d6fb58d` matches
its source-bound `.build-info` record.

The 13-entry evidence manifest verifies at
`out/evidence/m30-release-1790893780350727000/SHA256SUMS`; its README records
that the ordinary Model Store capability passed without the Granite digest
marker, and that the separate small fixture is not the Granite artifact. No
inference is claimed. M30 remains `PARTIAL` for authenticated updates and
System B acceptance, remaining M18–M29 acceptance, and release-distribution
review.

## Completion Sweep — M25 Whisper guest artifact digest and regressions (2026-10-02)

Added `./nagi m25-whisper <artifact.bin>` using the pinned model lock and a
separate disposable GPT Model Store image. The guest verified the artifact's
487,601,967-byte length, GGML magic, and full SHA-256
`1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b` through
the read-only Model Store capability. QEMU emitted
`Nagi M25 Whisper artifact digest PASS`; `qemu-img check` found no errors.
Evidence is in `out/evidence/m25-whisper-artifact-1790895162586172000/`, with
the guest image at
`out/artifacts/nagi-0.1-m25-whisper-1790895162586172000.qcow2`.

The shared acceptance runner was regression-tested by
`./nagi m20-granite` with the full 2,244,011,552-byte pinned Granite artifact;
the guest digest and Model Store markers passed and its image passed
`qemu-img check`. Further guest regressions passed for `./nagi m25`,
`./nagi m19`, and `./nagi m22`. The changed CLI passed 165 unit and 21
integration tests, warnings-denied Clippy, `./nagi fmt`, `./nagi test`,
`./nagi lint`, `./nagi build`, and the Nagi-target build with
`m25-whisper-artifact-acceptance`. Evidence for the Granite rerun is at
`out/evidence/m20-granite-artifact-1790895384092435000/`.

This is guest artifact integrity verification only. It does not load
whisper.cpp, perform STT inference, or use audio hardware. The regular M30
Model Store remains unchanged; M25 and M20 remain `PARTIAL`.

## Completion Sweep — current-source M30 image and release bundle (2026-10-02)

The existing fixed-path reference image was from commit `0e77563`; it and its
matching `.build-info` were preserved and checksum-verified at
`out/evidence/m30-pre-m25-whisper-current-source-20261002/` before building
for current source `b4385e1dac8b35e3f86a13f34ae5d806cbd0e40d`. The rebuilt
64 GiB GPT image SHA-256 is
`8260512ffd8dfae98539699163c3bee39e91acd9f2dc88137f0e45cb553dbff7`, and its
sidecar records the same full source revision and digest.

Fresh `./nagi m30` run `1790896634463366000` passed System A, read-only Model
Store access, User Data persistence across restart, M19 Search, M22
Move/Copy/Activity Ledger, Recovery, unchanged A/B journal, rejection of
unstaged System B, post-Recovery M22 Undo, M13 completion, and the separate
M20 FAT32 fixture. The 13-entry evidence manifest verifies at
`out/evidence/m30-release-1790896634463366000/SHA256SUMS`; pristine, mutable,
and fixture images passed `qemu-img check`.

`tools/nagi-release/test_release.py` passed all 14 tests. Clean-tree preflight,
assembly to `out/artifacts/m30-release-bundle-b4385e1/`, release verification,
bundle `qemu-img check`, and byte identity with the pristine reference passed.
The bundle still records `m30_acceptance=NOT_EVALUATED`; authenticated GPT
updates, System B acceptance, remaining M18–M29 work, and human redistribution
review remain. M30 remains `PARTIAL`.

## Completion Sweep — M28 integration and M3 handoff diagnosis (2026-10-02)

A guarded one-repetition M28 run passed M19 Search/ObjectId persistence and
M22 three-boot grouped Undo/Activity Ledger recovery. Its M27 rollback boot 4
timed out after three AP-online markers; QMP reported shutdown and kernel RIP
`0x40060c0` in `smp::thread_entry`. Evidence and verified manifests are under
`out/evidence/m28-run-20261001T235755Z-18930/` and
`out/evidence/m27-ab-rollback-1790899090521806000/`. To make that transition
visible in future traces, the M3 scheduler-start marker now precedes BSP `sti`.
Kernel formatting and a target build passed; guest boots in the follow-up M27
run reached M3 scheduler completion. That M27 command later timed out before
the healthy-B guest started, with QMP at OVMF RIP `0x7eb84171`; its verified
evidence is `out/evidence/m27-ab-rollback-1790899532394169000/`. The firmware
loop root cause is unknown. Neither failed sequence is counted as a M27 or M28
acceptance pass; both milestones remain PARTIAL.

## Completion Sweep — b11858b M30 clean-source release acceptance (2026-10-02)

The previous source-bound reference image at `b4385e1` and its build-info
were hash-preserved in `out/evidence/m30-pre-b11858b-current-source-20261002/`.
From clean source `b11858baad8beea9955f4e52433380c24f302c11`, `./nagi m30`
rebuilt the self-contained 64 GiB GPT image (SHA-256
`95577eb6f46f9cafeeaf4adc7ea48963483f31c0bcf63a281486e8ec3c8b747d`) and
passed System A, User Data restart persistence, M19 Search, M22 grouped Undo,
GPT Recovery, unstaged-System-B rejection, post-Recovery restart, and the
separate M20 FAT32 Model Store reader fixture. The 16-entry evidence manifest
at `out/evidence/m30-release-1790900017117346000/SHA256SUMS` verifies; pristine,
mutable QEMU, fixture, and bundle qcow2 checks passed.

All 14 release-tool tests passed. Current-source preflight, assembly to
`out/artifacts/m30-release-bundle-b11858b/`, bundle verification, and byte
identity between packaged and pristine qcow2 passed. The bundle correctly
retains `m30_acceptance=NOT_EVALUATED`. No model inference or host audio
acceptance is claimed. M30 remains PARTIAL for authenticated updates, remaining
M18–M29 acceptance, and human binary redistribution review.

## Completion Sweep — M28 two-repetition Search/History/Recovery gate (2026-10-02)

At source `f718117009c23cd0b3ecbb0f328277fe7f608200`, the guarded M28 runner
completed two consecutive repetitions. Each passed M19 guest Search/ObjectId,
the three-boot M22 Move/Copy, NH16/NAL1, grouped Undo and restart checks, and
the M27 malformed-System-B rollback, healthy-System-B promotion, and Recovery
Undo/journal checks. The full run archive is
`out/evidence/m28-run-20261002T002354Z-22197/`; its SHA-256 manifest, both M27
sub-run manifests, and the preserved pre-run disk/log snapshot manifest all
verify. Harness self-test, dry-run, and shell syntax checks passed. The host
has no `virtio-sound.in` audio input backend, and the run did not measure audio.

This satisfies the repeated Search/History/Recovery integration slice. It does
not exercise M28's Desktop/Files/Notes/Albert reference workload, real Granite
inference, OOM, CPU fairness, or resource-leak soak. M19, M21, M22, M27, and
M28 remain `PARTIAL` for the production service/security and remaining formal
acceptance work recorded above.

## Completion Sweep — M29 localized selected-state cue (2026-10-02)

The fixed Desktop Settings control now indicates the selected locale with
localized text (`Selected` / `選択中`) as well as its existing teal border.
The target font has dedicated glyphs for the Japanese text; tests confirm both
catalog values and the selected-row render path.

The M29 QEMU acceptance passed on the updated source: keyboard focus and
Japanese selection markers were present, the selected preference was written
to User Data, and a restart with the same disk restored `ja-JP` before the
Desktop's first frame. READY arrived 2,369 ms after QEMU spawn. The screenshot
SHA-256 is
`a6d8e854ccddb53800a8924205072b8cc622abf6f8294361883f77fb007064a9`; the
tracked screenshot matches the run evidence byte-for-byte. The nine-entry
run manifest verifies at
`out/evidence/m29-settings-1790901427735858000/SHA256SUMS`, and the previous
tracked screenshot is preserved with its own verified manifest.

The focused localization suite passed 7 tests and the M29 CLI contract suite
passed 6. Repository fmt, tests, lint, and build passed. This adds a textual
selection cue only; it does not provide assistive-technology support or a
system-wide accessibility tree. M29 remains `PARTIAL` for its wider polish
criteria.

## Completion Sweep — 25e0b54 current-source M30 release acceptance (2026-10-02)

The previous fixed-path qcow2 was bound to source `acffe0b`; its image and
sidecar were preserved with a verified SHA-256 manifest under
`out/evidence/m30-preserved-stale-25e0b54/`. From clean source
`25e0b5443363f87a4e503a3031cb9804f3e29c07`, `./nagi m30` produced a 64 GiB GPT
reference image with SHA-256
`47615fce4e0b7442f1d016add408eb84c120b6fb5ad0dcd85b00c517e4de2d41`; its
`.build-info` binds that digest to the full commit.

QEMU run `1790904245966571000` passed System A, User Data restart persistence,
M19 Search/ObjectId, the M21 Search action fixture, M22 grouped Move/Copy and
Activity Ledger Undo, GPT Recovery, rejection of unstaged System B,
post-Recovery Search/Undo, and the separate M20 FAT32 Model Store reader
fixture. Source, QEMU acceptance copy, fixture, and release-bundle qcow2 all
passed `qemu-img check`. The 12-entry run manifest verifies at
`out/evidence/m30-release-1790904245966571000/SHA256SUMS`.

All 14 release-tool tests passed; current-source preflight, assembly to
`out/artifacts/m30-release-bundle-25e0b54/`, bundle verification, 22 bundle
checksums, and pristine/bundle image byte identity passed. The release manifest
retains `m30_acceptance=NOT_EVALUATED`. No model inference or host audio input
is claimed. M30 remains `PARTIAL` for authenticated updates, remaining
M18–M29 acceptance, and human binary redistribution review.

## Completion Sweep — M20 split path status and bounded shard construction (2026-10-02)

Added llama.cpp patch `0009-nagi-llama-split-path-status.patch` on top of the
pinned upstream revision and patches 0001–0008. `llama_split_path()` and
`llama_split_prefix()` now reject null pointers, empty buffers, invalid split
indices/counts, malformed suffixes, and truncation with a zero return and an
empty output buffer. Split-list construction checks every generated path and
publishes a temporary list only after all expected shard names are built. The
loader checks the split-path status before model metadata printing or model
creation. Host tests cover canonical shard names, exact-fit and short buffers,
invalid indices/suffixes, and empty-on-failure behavior.

`git apply --check` passed against a preserved post-0008 source snapshot. A fresh
`./nagi fetch` generated the patch-0009 checkout and matching marker; the
overall fetch then stopped with exit 4 at the pre-existing mismatched generated
Servo checkout, which was left untouched. The raw pinned
`third_party/llama.cpp` checkout stayed clean. Host `test-model-loader-bounds`
CTest passed 1/1, the CLI patch-contract test and full `./nagi test` suite
passed, and `./nagi fmt`, `./nagi lint`, and `./nagi build` passed. The full
test rerun also fixed a parallel FAT32 fixture temp-path collision with a
test-only atomic sequence. The final
targeted Nagi C++ compile still fails in `llama-model-loader.cpp` and
`llama.cpp` on unrelated remaining `throw`/`try` paths. This patch removes one
loader failure slice; no complete Nagi-target backend or inference is claimed.

The final generated-checkout marker, preserved pre-regeneration snapshots and
checksums, fetch/test logs, and target diagnostics are recorded under
`out/evidence/m20-loader-split-path-0009-20261002/`. M20 remains `PARTIAL`.

## Completion Sweep — bootstrap Channel wait/wake (2026-10-02)

Added syscall 35, `SYS_CHANNEL_WAIT_READABLE`, backed by the existing bounded
`WaitRegistry` and cooperative bootstrap thread scheduler. The Channel
`WaitItem` is constructed through the endpoint handle table and requires
`Rights::WAIT`; the syscall marks the caller blocked while the shared IPC lock
still protects the registration. A successful send drains the reserved wake
records under that lock, releases it, and then marks each matching thread
runnable. The syscall never switches context while holding the IPC lock.
`libnagi::channel_receive` retries nonblocking receive after readiness wakes,
including the case where another receiver consumes the message first. If no
other runnable or sleeping bootstrap thread can produce a message, the wait
returns an error instead of stranding the only thread.

Host tests cover already-readable readiness, denial without `WAIT`, exactly-once
send wakeups, cancelled waiter cleanup, scheduler wake transitions, and the
no-producer abort path. `./nagi test`, `./nagi fmt`, `./nagi lint`, and
`./nagi build` passed. `./nagi m19` passed on QEMU with the new guest-level
blocked-thread/send/wake/payload check; its log, image, vars, and resulting User
Data disk are under `out/evidence/channel-wait-20261002/`. The pre-run M19 User
Data disk is preserved with SHA-256 at
`out/evidence/channel-wait-pre-m19-20261002-de3092c/`.

An attempted `./nagi m17` regression stopped before target build because the
fetch preflight refused the pre-existing modified generated Servo checkout.
That checkout and the pre-run M17/M18 artifacts were left untouched; snapshots
of those artifacts verify under
`out/evidence/channel-wait-pre-m17-m18-20261002-de3092c/`. M19 remains
`PARTIAL`: this bootstrap wait does not add isolated processes, authenticated
endpoint delivery, or production Search/service callers, and it is not a
general user syscall for Event, Timer, process exit, or service readiness.

## Completion Sweep — M25 Whisper standard-loader short reads (2026-10-02)

Added patch 0002 to the Nagi-owned whisper.cpp patch boundary. The standard
model loader now accumulates positive short reads, rejects incomplete or
invalid read counts, and accepts tensor-section EOF only when zero header
bytes were read before EOF. The host regression loads a synthetic valid
no-tensor model, exercises chunked reads, rejects a truncated field, and
rejects partial tensor headers of lengths 1–11. Host CMake build and CTest
passed; the Nagi-target `whisper` library build passed. `./nagi fmt`,
`./nagi test`, and `./nagi lint` passed.

`./nagi fetch` applied patch 0002 and wrote a fresh generated-checkout marker,
then exited 4 because the existing generated Servo checkout does not match
its pinned patch result. The raw pinned `third_party/whisper.cpp` remained
clean. The pre-change generated checkout and verified SHA-256 manifest are at
`out/evidence/m25-whisper-read-count-20261002/`. This verifies loader behavior
with synthetic data only; it does not load the pinned model or run inference.
M25 remains `PARTIAL` pending a real Japanese STT provider, authenticated
permission/UI, local TTS, microphone indicator, and spoken-command acceptance.

## Completion Sweep — M25 clean-checkout patch correction (2026-10-02)

GitHub CI runs for commits `6d15f43` and `1107fba` failed while applying
patch 0002 on a clean pinned whisper.cpp checkout. Replaced its zero-context
hunks with a contextual unified diff and scoped the expected blank context
line whitespace exception to that one patch in `.gitattributes`. The patch
passes forward `git apply --check` against the preserved pre-change generated
checkout and reverse `git apply --check` against the previous generated
checkout. A fresh clone of pinned revision
`927cfce34f31707e17f2bff35c349632fb9e2c3a` also accepted both numbered
patches in order. `./nagi fetch` rebuilt the Whisper checkout with patch fingerprint
`fnv1a64:0752e44c02fe91e8` and the expected checkout fingerprint
`fnv1a64:5a3c628c90d404d3`, then stopped at the pre-existing modified
generated Servo checkout without changing it. The old generated Whisper
checkout was preserved with a 1,988-entry SHA-256 manifest at
`out/evidence/m25-whisper-old-generated-checkout-20261002/`.

The focused `test-whisper-buffer-loader` CTest passed (1/1), and the Nagi-target
CMake `whisper` library build passed. Logs are
`out/logs/m25-whisper-context-host-ctest.log`,
`out/logs/m25-whisper-context-target-build.log`, and
`out/logs/m25-whisper-context-fetch.log`. A local `cargo test` for the CLI
patch-contract test could not link its x86_64 host helper: the installed
Command Line Tools lack an x86_64-compatible `libxcrun`; CI will provide the
clean-host contract-test result. M25 remains `PARTIAL`; the loader test uses
synthetic data and does not run Whisper inference.

## Completion Sweep — M30 release-bundle symlink rejection (2026-10-02)

The release verifier now rejects a symlink at the bundle root or anywhere
under it before reading release metadata or calculating artifact hashes. The
walk does not follow directory symlinks. A regression test first reproduced
the prior gap with an untracked directory symlink to external bytes; after the
fix, the focused case and all 16 release-tool tests pass. `./nagi fmt`,
`./nagi lint`, `./nagi test`, and `./nagi build` also pass. From commit
`65f4d6f8773e0b373f237067960738f66e454f3b`, `./nagi m30` run
`1790926329664045000` passed. Release preflight, assembly to
`out/artifacts/m30-release-bundle-65f4d6f/`, verification, all 23 checksums,
image byte identity, and pristine/bundle `qemu-img check` passed. The 17-entry
evidence manifest verifies at
`out/evidence/m30-release-1790926329664045000/SHA256SUMS`. The previous
fixed-path image bound to `25e0b54` was preserved, rehashed, and checked under
`out/evidence/m30-release-symlink-stale-image-65f4d6f/`. The release manifest
keeps `m30_acceptance=NOT_EVALUATED`. M30 remains `PARTIAL` pending its
existing authenticated-update, remaining M18–M29, and human redistribution-
review criteria.

## Completion Sweep — M20 vocabulary and Unicode checked status (2026-10-02)

Added llama.cpp patches 0023–0029 to the existing Nagi-owned patch stack.
Patches 0023–0025 extend loader failure status through the remaining model
architectures and model-load boundary; 0026 adds explicit context
initialization status; 0027–0028 make state, file, and mmap I/O failures
observable without target exceptions; 0029 validates T5 charsmap structure,
returns tokenizer metadata/load errors through the loader, and adds checked
UTF-8 scalar and byte conversion APIs. Host exception behavior remains enabled.

Host and Nagi-macro CMake builds of `test-model-loader-bounds` passed. The
focused CTest passed 1/1 in both configurations, including invalid tokenizer
metadata, malformed charsmap leaves, and malformed UTF-8 fixtures. `cargo test
-p nagi-cli` passed 192 library tests and 21 CLI integration tests. The
Nagi-configured no-exceptions syntax sweep passed 27 of 32 top-level llama.cpp
translation units; the remaining five are KV cache, DSV4 cache, recurrent
memory, quantization, and sampler. The exact per-unit diagnostics are in
`out/logs/m20-loader-status-0029-noexceptions-tu.log`; build and CTest logs
are in `out/logs/m20-loader-status-0029-{host,nagi}-{build,ctest}.log`.

This is compile and checked-error-propagation progress only. Granite has not
been loaded or run for inference, so M20 remains `PARTIAL`.

## Completion Sweep — M20 memory construction failure status (2026-10-02)

Added llama.cpp patch 0030. KV, recurrent, and DSV4 cache constructors now
retain host exceptions and report Nagi allocation failures through an
initialization status. Composite memory modules inspect their child status;
`llama_model::create_memory()` deletes an incomplete module and returns null so
the existing context initialization path reports failure instead of using
partially allocated cache tensors.

Host and Nagi-macro builds of `test-model-loader-bounds` passed, as did both
focused CTests (1/1). `cargo test -p nagi-cli` passed 193 library tests and 21
CLI integration tests. The Nagi-configured no-exceptions sweep passes 29/32
top-level translation units; DSV4 runtime validation, quantization, and sampler
remain. The allocation-failure branch is not fault-injected yet; this check
establishes compilation and checked propagation wiring, not an induced OOM
acceptance. Logs are under `out/logs/m20-loader-status-0030-`.

No model inference has run; M20 remains `PARTIAL`.

## Completion Sweep — M20 DSV4, sampler, and quantization checked status (2026-10-02)

Added llama.cpp patches 0031–0033. DSV4 batch, stream, compressor-plan, and
rollback metadata now return checked preparation/I/O failures on Nagi, with
compression plans validated before raw cache slots are reserved. Sampler ring
access and backend graph setup now latch and propagate failure; sampling
returns `LLAMA_TOKEN_NULL` after a failed ring operation. Quantizer type
selection, dequantization, row validation, importance-matrix checks, and model
quantization now propagate checked status. `llama_quant_compute_types` returns
`bool`, leaves `GGML_TYPE_COUNT` in the result array on failure, and publishes
types only after the full assignment succeeds. Host exception behavior remains
enabled.

Host and Nagi-macro builds of `test-model-loader-bounds`, `test-sampling`, and
`test-quant-type-selection` passed. Focused CTests passed 1/1 for sampler and
quantization in both configurations. `cargo test -p nagi-cli` passed 196
library tests and 21 CLI integration tests, and `cargo fmt --all -- --check`
passed. A fresh Nagi-configured `-fno-exceptions -fsyntax-only` sweep passed
all 32 top-level llama.cpp `src/*.cpp` translation units; the result is in
`out/logs/m20-loader-status-0033-noexceptions-tu.log`. Build and test logs are
under `out/logs/m20-loader-status-003{1,2,3}-`.

At this checkpoint the full Nagi-target `llama` build had not yet been rerun
after these patches. Its last recorded attempt stopped on exception syntax in
model-specific source files. Direct fault injection for DSV4 malformed
batch/state-I/O paths and sampler ring corruption is not available. No Granite
inference had run, so M20 remained `PARTIAL`.

## Completion Sweep — M20 full Nagi-target llama archive (2026-10-03)

After patches 0031–0033, the complete LLVM 19/libc++ no-exceptions `llama`
target built successfully: Ninja completed all 42 steps and linked
`out/m20-llama-backend-reg-noexceptions-20261001-clang19/src/libllama.a`
(6.4 MiB). The build required the configured target compiler, C++ headers, and
Homebrew Ninja on `PATH`; the complete log is
`out/logs/m20-loader-status-0033-noexceptions-target-build-llvm19.log`.
Warnings were limited to existing unused mmap parameters and unreachable
fallback returns in `llama-context.cpp`.

This is a static library build, not a link into `nagi-init` or the guest Model
Manager. Granite loading, generation, unload/restart, bounded runtime behavior,
and structured-output inference remain unverified. M20 remains `PARTIAL`.

## Completion Sweep — M17 storage recovery and M18 QEMU regression (2026-10-03)

The first 2026-10-03 M17 rerun reached `Servo::new storage threads started`,
then aborted before the first storage thread spawned. A read-only audit of the
preserved User Data image found all 64 VFS inodes allocated and zero free
inodes. Its `/tmp` contained 20 stale Servo temporary trees: ten
`clientstorage/default_v1` and ten `cachestorage/default_v1`; data blocks
remained free. Five earlier Servo constructions had created four such roots
each. With `Servo::Opts::config_dir` unset, the next construction attempted
another `tempfile::tempdir()` and aborted. The exact panic expression is
inferred from the constructor order and trace boundary. The failed image, OVMF
variables, and logs are preserved with a checksum manifest under
`out/evidence/m17-storage-init-abort-20261003/`.

The M17 first-pixel fixture now sets its Servo `config_dir` to
`/tmp/nagi-m17-servo` in `user/nagi-albert/src/lib.rs`, avoiding repeated
random temporary roots on the fixture's persistent VFS. On a fresh User Data
disk, `./nagi m17` passed the real Servo/Mesa Softpipe first-web-pixel
acceptance; the serial log contains the nonzero frame checksum and PASS marker.
The run image, User Data disk, OVMF variables, serial log, and full invocation
output verify under `out/evidence/m17-first-web-pixel-20261003/SHA256SUMS`.
This establishes the recovery on a fresh disk; the preserved inode-full disk
was not reused.

After moving the stale generated Servo checkout aside and regenerating it from
the pinned revision and patches with `./nagi fetch`, the 2026-10-03 `./nagi
m18` rerun passed. QEMU verified TLS chains and hostnames for `example.com`,
`example.org`, and `example.net`, rendered all three pages through Nagi
Surface, and passed browser temporary-storage cleanup. The saved screenshot
was visually inspected. The run reused the current M18 User Data disk. Its
current image, disk, OVMF variables, serial log, invocation output, screenshot,
and hashes are under `out/evidence/m18-completion-sweep-20261003/`; the
pre-rerun fixed-path files are under
`out/evidence/m18-pre-sweep-20261003/`. The run log is
`out/logs/m18-albert.log`, and the full command output is
`out/logs/m18-stdio-20261003.log`. Both QEMU runs used the pinned Mesa Python
dependencies in the ignored `out/m17-mesa-venv` and Homebrew LLVM 19 with its
matching libc++ headers. QEMU did not have a host `virtio-sound.in` audio
backend. M17 remains `PASS`; M18 acceptance remains `PASS` while the milestone
remains `PARTIAL` for its listed missing browser providers and interactions.

## Completion Sweep — M19 persistence and M22 grouped-Undo regression (2026-10-03)

`./nagi m19` passed on QEMU after the M17 storage-root change. Its guest log
passed live VFS file metadata search, stable Object ID after rename/remount and
restart, and the M21 `file.search` Plan/Validate/Execute fixture. The previous
fixed-path M19 image, User Data disk, OVMF vars, and existing initial log were
copied before the rerun to `out/evidence/m19-pre-regression-20261003/`; the
new run image, disk, vars, serial log, invocation output, and manifest verify
under `out/evidence/m19-regression-20261003/`.

Fresh three-boot QEMU run `1790958557043820000` passed the M21/M22 fixture:
VFS `file.move` and `file.copy`, NH16 transactions, Activity Ledger entries,
composite Undo, and restored state survived restarts. Its unique image, User
Data, OVMF vars, bootstrap and three boot logs, invocation output, README, and
SHA-256 manifest are under `out/evidence/m22-regression-20261003/`. This
confirms fixture persistence and undo behavior only; M19/M22 still lack
authenticated production service callers and production Action/Activity
service integration.

## Completion Sweep — package/SDK regression and DF-01 availability (2026-10-03)

`./nagi m16` rebuilt the out-of-tree Hello Nagi sample and its NAPP/.xapp
packages, regenerated and compared the Rust/C IDL bindings, and built the
feature target image. It then timed out in QEMU after printing `Nagi M14
capture FAIL`; the host QEMU reports that it cannot open `virtio-sound.in`
because no host audio input backend is available. The M16 guest install,
launch, atomic update, and remove markers were not reached. This does not
invalidate prior M16 acceptance, but this macOS rerun cannot revalidate those
guest steps. Pre-run fixed-path state is under
`out/evidence/m16-pre-regression-20261003/`; this attempt's image, User Data,
OVMF vars, sample artifacts, generated bindings, and logs are preserved with a
SHA-256 manifest under `out/evidence/m16-regression-20261003/`.

The requested `DF-01 verify` regression has no implementation in this
checkout: there is no `.dev` registry or verify command in the CLI. The M19
workstream records that this baseline predates DF-01 state tooling. No DF-01
result is claimed; rerun it only when that tooling is present in the branch.

## Completion Sweep — current-source M30 release and recovery acceptance (2026-10-03)

On clean source commit `4fae6875d64752db8fbe0508a932c28da246e8af`, the current
reference qcow2 SHA-256 is
`1e81c7a89b4295bcadebfd835d4942ad53849ee1f81be3cb7ff5cc05395f379d`. QEMU run
`1790985901890315000` passed System A boot, first-boot and restart User Data
persistence, M19 Search/ObjectId persistence, M22 Search/Activity Ledger and
grouped Undo, Recovery, rejection of unstaged System B, and restart after
Recovery. The separate M20 reader fixture passed against its disposable Model
Store image. The pristine image, QEMU copy, and M20 fixture all pass
`qemu-img check`.

Clean-source release preflight, assembly to
`out/artifacts/m30-release-bundle-4fae687/`, and verification passed. All 23
bundle checksum entries verify; the packaged image is byte-identical to the
reference image, and the bundle qcow2 passes `qemu-img check`. The release
manifest correctly leaves `m30_acceptance=NOT_EVALUATED`; these checks do not
establish human binary redistribution permission. The pre-run image bound to
`65f4d6f8773e0b373f237067960738f66e454f3b` was moved intact to
`out/evidence/m30-stale-image-pre-4fae687-20261003/`; its digest and qcow2
structure were verified before rebuilding.

The complete run evidence and 15-entry SHA-256 manifest are under
`out/evidence/m30-release-1790985901890315000/`. QEMU reports no host
`virtio-sound.in` driver; guest M14 sound initialization passed, but this run
adds no audio-capture evidence. M30 remains `PARTIAL` for authenticated updates,
remaining formal M18–M29 acceptance, and human redistribution review.

## Completion Sweep — two M28 Search/History/Recovery repetitions (2026-10-03)

The M28 harness passed `sh -n`, `--self-test`, and the two-repetition
`--dry-run`. Its write-collision guard found the existing M19 image, OVMF
variables, and initial log; those three generated files were moved intact to
`out/evidence/pre-m28-repeat-20261002T224528Z-22372/`, whose SHA-256 manifest
verifies. The source revision was
`78b7655efe72e73340e699fae3509cb48a68f8c8`.

`NAGI_M28_REPEAT_COUNT=2
./tests/acceptance/m28_integration_stress.sh --run` passed two consecutive
repetitions. Each passed fresh M19 VFS/ObjectId/Search, all three M22
Move/Copy/NH16/NAL1 grouped-Undo boots, and M27's malformed-System-B rollback,
healthy-System-B readiness and promotion, and Recovery journal/Undo gate.
The archive is `out/evidence/m28-run-20261002T224535Z-22420/`; its manifest,
both M27 sub-run manifests, and the preserved pre-run manifest verify. All four
M27 GPT images pass `qemu-img check`.

This run did not reproduce the earlier OVMF startup loops at RIP `0x7eb84171`,
but their cause remains unknown. QEMU still lacks a host `virtio-sound.in`
driver. Desktop/Files/Notes/Albert concurrent load, real Granite inference,
audio pressure, OOM, CPU fairness, and leak soak remain unmeasured. M27 and M28
remain `PARTIAL`.

## Completion Sweep — M3 scheduler fairness and M28 current-source replay (2026-10-03)

The M3 preemptive self-test now validates bounded dispatch skew between two
busy kernel tasks on each CPU; M19/M22 acceptance requires its fairness PASS
marker. The kernel scheduler regression exercises 131,072 yields with all 64
bootstrap slots runnable. Focused scheduler tests passed 13/13; read-only
callback adapter tests passed 3/3. In the current tree, `./nagi fmt`,
`./nagi test`, `./nagi lint`, and `./nagi build` all pass. The M28 harness
passes `bash -n`, `--self-test`, and `--dry-run`; strict targeted `nagi-posix`
Clippy passes with warnings denied.

Current-source QEMU run `out/evidence/m28-run-20261003T004328Z-37332/` passed
one repetition of M19, M22, and M27. The scheduler fairness marker appeared in
the M19 and M22 logs. The archive and M27 sub-run manifests verify, and the two
M27 GPT images pass `qemu-img check`. Two attempts at a current-source
two-repetition run were incomplete because QEMU/OVMF stopped before guest
acceptance at RIP `0x7eb84171`: `out/evidence/m28-run-20261003T002909Z-34387/`
completed its first repetition and timed out on repetition 2 M19; an immediate
standalone M19 retry passed. `out/evidence/m28-run-20261003T003659Z-36156/`
passed repetition 1 M19/M22 and timed out at the initial M27 System B boot.
Neither incomplete attempt is counted as a pass. M28 remains `PARTIAL`.

GitHub Actions run `37081572976` exposed two strict Clippy diagnostics in
`user/nagi-posix/src/readonly_callback_file.rs`: the test-only `len()` method
was unused and the test used a manually constructed dangling pointer. Both
were fixed locally by asserting the descriptor length and using
`core::ptr::dangling_mut`; the local targeted strict Clippy and callback tests
pass. The Windows launcher job passed on that run; the Ubuntu host job failed
before these fixes, and the Nagi target job had not completed when this entry
was written. A fresh CI result is still required.

The M20 inference target integration remains blocked at final link: after
resolving the first C/POSIX symbols, the official attempt still reports 88
undefined target C++ standard-library symbols. No guest Granite inference is
claimed. The detailed link logs and attempt record remain under
`out/evidence/m20-granite-inference-1790985332307332000/`.

## Completion Sweep — current-source M30 release replay (2026-10-03)

Clean source commit `9b16eaae729b8c61403aa929912de5ab5da19d4b` passed
`./nagi m30` run `1790988888019354000`: System A initialization, User Data
persistence across restart, Recovery with unchanged boot journal, unstaged
System B rejection, post-Recovery System A restart, and a separate M20 Model
Store fixture read. The reference image is a 64 GiB GPT qcow2 with SHA-256
`e815da59636642c91b06fb6d9f75b038113eabadf7dfd2eb4a251cd61ac2f349`.

All 16 release-tool tests passed. Clean-source preflight, assembly to
`out/artifacts/m30-release-bundle-9b16eaa/`, verification, all 23 bundle
checksums, byte identity, and bundled qcow2 check passed. QEMU's source image
and mutable acceptance copy also pass `qemu-img check`. The release manifest
retains `m30_acceptance=NOT_EVALUATED`. The 11-entry QEMU evidence manifest
verifies under `out/evidence/m30-release-1790988888019354000/`. The prior image
bound to `4fae687` and its build-info sidecar were preserved and verified at
`out/evidence/m30-stale-image-pre-9b16eaa-20261003/`.

This host has no `virtio-sound.in` input driver. Authenticated update/System B
installation and human binary redistribution review remain open, so M30 stays
`PARTIAL`.

## Completion Sweep — repeated M27 startup timeout follow-up (2026-10-03)

Current-source M28 run `out/evidence/m28-run-20261003T011836Z-42073/` passed
all three gates in repetition 1. Repetition 2 passed M19 and M22, but M27
boot 5 did not reach the confirmed-System-A marker within its 90-second
timeout. This remains a failed two-repetition attempt. The M27 persisted
journal decision-boot timeout is now 180 seconds without changing guest
acceptance markers. Standalone `./nagi m27` then passed at run
`1790991014827320000`; its SHA-256 evidence manifest verifies, and both M27
GPT images pass `qemu-img check`. M28 still needs a fresh two-repetition pass;
its combined Desktop/Files/Notes/Albert, real Granite, audio, OOM, and leak
soak acceptance remains unmeasured, so M27/M28 remain `PARTIAL`.

The next M28 two-repetition run,
`out/evidence/m28-run-20261003T013706Z-44453/`, stopped during repetition 1
M19 after its 90-second pre-guest timeout. Its serial log contains only the
UEFI screen-clear sequence; QMP reported the guest still running at RIP
`0x7eb84171`. M19 and M22 now use a 180-second QEMU acceptance timeout while
retaining their existing guest markers. The failed archive and SHA-256
manifest are preserved; no M28 pass is claimed from that run.

After those changes, M28 run `out/evidence/m28-run-20261003T014330Z-45609/`
passed M19 and M22 in repetition 1 but timed out at 90 seconds on M27's third
readiness-promotion boot, before confirmed System B and `Nagi M10 desktop
READY`. QMP was still running at the recurring RIP `0x7eb84171`. The failed
attempt's parent and M27 manifests verify. M27's three readiness-promotion
boots now allow 180 seconds with their original markers; no two-repetition
pass is claimed yet.

## Completion Sweep — bounded M22 pre-guest firmware retry (2026-10-03)

Current-source M28 run `out/evidence/m28-run-20261003T020520Z-47383/` passed
repetition 1 across M19, M22's three boots, and M27. Repetition 2 passed M19,
then its M22 bootstrap timed out after 180 seconds before `Nagi Kernel started`;
the serial stream had only the 87-byte UEFI screen-clear prefix before
diagnostics, and QMP reported a running guest at RIP `0x7eb84171`. The run and
M27 sub-run SHA-256 manifests verify. The second repetition did not pass, so
this archive is not a two-repetition acceptance.

M22 bootstrap now retries once only when the timeout has no guest kernel-start
marker and QMP captured a running state plus CPU registers and instruction
window. It preserves the first serial log and OVMF variables, then starts from
a fresh copy of the configured OVMF template. A failure after the kernel
marker is never retried; all M22 guest acceptance markers remain unchanged.
The M28 harness archives both retry artifacts when present. The classifier
regression and `./nagi test`, `./nagi fmt`, `./nagi lint`, `./nagi build`,
`bash -n`, and harness self-test pass. A fresh standalone `./nagi m22` passed
all three QEMU boots at run `1790993965845089000`; that run did not trigger the
new retry path. M28 remains `PARTIAL`; a current-source repeated gate and the
formal Desktop/Files/Notes/Albert, Granite, audio, OOM, and leak-soak workload
remain outstanding.

## Completion Sweep — M27 pre-guest retry exercised (2026-10-03)

Fresh standalone `./nagi m27` run
`out/evidence/m27-ab-rollback-1790994710275400000/` passed the A/B and
Recovery acceptance: three malformed System B trials rolled back to persistent
System A, a healthy System B was promoted only after guest readiness, Recovery
preserved the boot journal, and Recovery undid a committed M22 `file.move`
group across restart. Boot 5's first attempt timed out after 180 seconds before
the guest kernel-start marker; QMP reported a running CPU looping at RIP
`0x7eb84171`. The bounded retry reused the same OVMF variables and passed its
original guest marker. The first serial log, first OVMF variables, and retry
note are preserved as `.pre-guest-timeout-1` and `.pre-guest-retry-1.txt`
sidecars. This validates recovery from the observed startup stall, not its
root cause. M27 remains `PARTIAL` for authenticated update/readiness authority,
authenticated slot manifests, and remaining Recovery requirements; M28's
two-repetition integration gate is also still outstanding.

The retry unit regression confirms the two-attempt bound, same-vars reuse, and
first-attempt evidence; a Unix path test confirms sidecar suffixes preserve
non-UTF-8 path bytes. The run's 41-entry `SHA256SUMS` verifies from its
evidence directory, and both run-stamped GPT images pass `qemu-img check`.
`./nagi fmt`, `./nagi test`, `./nagi lint`, `./nagi build`, M28 harness
`bash -n`, `--self-test`, and `--dry-run` pass. The dry-run confirms the real
gate will rerun M19 before M22 and M27; its prior M19 serial log is absent and
is not counted as evidence.

## Completion Sweep — M27 retry journal-state correction (2026-10-03)

Read-only review of commit `f2aca56` found that retrying with the first
attempt's post-boot OVMF variables could consume a second durable trial count:
the loader can persist `begin_boot()` before emitting the kernel-start marker.
The current M27 retry now snapshots the vars before QEMU starts, refuses
existing retry-sidecar collisions before boot, preserves the first attempt's
post-boot vars, writes the pre-boot snapshot to a separate sidecar, and restores
that snapshot before the single retry. Its regression simulates a first-attempt
journal increment and verifies the retry starts from the original state.

M28 run `out/evidence/m28-run-20261003T024610Z-52844/` passed M19 and M22,
then was interrupted after the harness announced its M27 invocation but
before it recorded an M27 result. The M28 archive manifest verifies and records
zero complete repetitions, so this is not an M28 pass. An unarchived M27
directory from that interrupted invocation is explicitly unverified and is
not counted. Corrected standalone `./nagi m27` run
`1790995870248251000` then passed the A/B and Recovery acceptance. Its 39-entry
manifest verifies and both GPT images pass `qemu-img check`; that run did not
trigger the retry, whose journal restoration is covered by the regression.

## Completion Sweep — M27 writable-disk retry guard and M28 Recovery timeout (2026-10-03)

M28 run `out/evidence/m28-run-20261003T031921Z-57898/` completed repetition 1
across M19, M22, and M27. Repetition 2 passed M19 and all three M22 boots,
then timed out before guest output at M27 Recovery Undo restart verification.
QMP reported `status=running` at the recurring OVMF RIP `0x7eb84171`; the
serial log contains only the 87-byte UEFI screen-clear prefix. The run has one
complete repetition out of two and is not a pass. The parent and both M27
sub-run manifests verify.

An isolated QEMU diagnostic replay used copies of the archived healthy slot
image, User Data image, and pre-promotion OVMF variables and reached
`Nagi M10 desktop READY` in 5.78 seconds. Its manifest verifies at
`out/evidence/m27-replay-promotion-1790996199847179000/`. This supports an
intermittent firmware-start stall; the replay does not establish its cause or
change the M28 result. The failed run did not preserve a pre-attempt User Data
disk snapshot, so byte-identical persistent-disk state at the original boot
boundary is unverified.

The guarded one-time M27 retry now hashes every writable boot and persistent
disk before QEMU. It restores the saved OVMF vars and retries only if all those
disk hashes remain unchanged; a changed disk suppresses retry and preserves its
post-attempt state. The Recovery Undo M13 HTTP fixture boot and restart
verification also use this wrapper. Retry remains limited to one attempt and
the original guest acceptance markers are unchanged. `./nagi fmt`,
`./nagi test`, `./nagi lint`, and `./nagi build` pass, including regressions
for disk-change suppression and same-state journal restoration. A further
current-source two-repetition M28 run followed; its final result is recorded
below. M27 and M28 remain
`PARTIAL` for their recorded authenticated-update/authority and broader
acceptance gaps.

## Completion Sweep — M22 boot retry guard and current M28 result (2026-10-03)

The current-source M28 run `out/evidence/m28-run-20261003T033818Z-60771/`
passed two complete repetitions of M19 Search, M22's three-boot grouped
Move/Copy and Undo, and M27 GPT A/B/Recovery. The parent archive and both M27
sub-run manifests verify. In M27 repetition 2, boot 1 timed out in the known
pre-guest OVMF loop after 90 seconds; writable-disk SHA-256 was unchanged and
the single guarded retry reached the original acceptance marker. The archived
18 MiB User Data raw GPT images passed primary/backup GPT header and partition
table CRC checks. `qemu-img check` is unsupported for these raw images.

The preceding run `out/evidence/m28-run-20261003T032603Z-59318/` passed M19
and M22 boots 1–2, then M22 boot 3 timed out after 180 seconds in the same
pre-guest OVMF loop. Its manifest verifies; zero complete repetitions passed.
M22 bootstrap and all three numbered guest boots now use one bounded retry
only after the running-CPU/QMP timeout signature, no kernel-start marker, and
unchanged SHA-256 for writable boot and User Data images. Every boot has its own retry
sidecars, and the M28 harness archives them. The retry was not needed by M22
in the passing gate; unit regressions cover state restoration, changed-disk
suppression, and evidence isolation across three boots.

Verification on this source state: `./nagi fmt`, `./nagi test`, `./nagi lint`,
`./nagi build`, M28 shell syntax, and harness self-test passed. Standalone
`./nagi m22` run `1790998679703245000` passed all three guest boots. The host's
QEMU reported no `virtio-sound.in` driver; audio remains outside these gates.
M22, M27, and M28 remain `PARTIAL` for their documented production-authority
and wider acceptance gaps.

## Completion Sweep — M22 bootstrap guard and final repeated M28 acceptance (2026-10-03)

The final-source standalone M22 run `1790999599873700000` passed its bootstrap
and all three numbered guest boots. The same M22 acceptance passed in both
repetitions of `out/evidence/m28-run-20261003T035340Z-63277/` after the
bootstrap retry was brought under the shared disk-state guard.

That M28 run passed both complete repetitions of M19 Search, M22 Move/Copy and
grouped Undo, and M27 GPT A/B/Recovery. Its archive and both M27 sub-run
manifests verify. Repetition 1's M27 Recovery Undo restart verification
reproduced the 90-second pre-guest OVMF loop; the two writable image SHA-256
values were unchanged, and the guarded retry reached the original marker.
Both archived raw GPT User Data disks have valid primary/backup header and
partition-table CRCs. `qemu-img check` is unsupported for raw format.

The M22 bootstrap and each numbered guest boot now initialize OVMF from the
configured template, snapshot it, and hash writable boot/User Data before
launch. They retry once only for the diagnosed pre-guest timeout signature and
unchanged disk bytes. The M22 retry was not needed in this final M28 run; unit
regressions cover the guard and sidecar isolation. The earlier
`out/evidence/m28-run-20261003T032603Z-59318/` remains a verified 0/2 failure
after M22 boot 3 stalled pre-guest.

On this final code, `./nagi fmt`, `./nagi test`, `./nagi lint`, `./nagi build`,
M28 shell syntax, and M28 harness self-test passed. M22/M27/M28 remain
`PARTIAL` for their production authority and broader unmeasured acceptance
requirements.

## Completion Sweep — M18 accepted QMP shutdown race (2026-10-03)

After the guest has printed its acceptance marker, QEMU can close the QMP
connection while processing `quit`, before sending the command response. The
CLI now treats that disconnect as an ambiguous shutdown result and waits up to
30 seconds for the QEMU child: a clean exit preserves the accepted serial log
and succeeds, while a nonzero exit, poll error, or timeout remains a failure.
Other QMP errors still fail immediately. The new
`accepted_qemu_exit_survives_qmp_disconnect_during_shutdown` regression passed.

The first M18 retry without an activated Mesa virtual environment stopped at
the pinned Mako requirement. Activating the existing `out/mesa-venv` advanced
the build, but the default macOS 27 SDK libc++ headers were incompatible with
the selected Apple Clang for Nagi's target ABI shim (`__libcpp_thread_yield`
and related declarations were missing). Both C++ ABI probe objects compiled
with the installed Homebrew LLVM 19 compiler and libc++ headers. With that
toolchain selected, local `./nagi m18` run `1791001974237346000` passed target
build and the three-site HTTPS/QEMU acceptance; TLS chain/hostname checks and
real Servo frames for `example.com`, `example.org`, and `example.net` reached
Nagi Surface. The command log is
`out/logs/m18-completion-sweep-20261003-llvm19.log`; the saved browser image and
run evidence are under `out/evidence/m29-browser-1791001974237346000/`. QEMU
reported that the host has no `virtio-sound.in` driver; audio is not part of
this browser gate.

On the same source, the focused QMP test, `./nagi fmt`, `./nagi test`,
`./nagi lint`, and `./nagi build` passed. `./nagi m18` with the default macOS
SDK toolchain failed during C++ compilation; the successful local invocation
activated `out/mesa-venv` and explicitly set
`NAGI_TARGET_CLANG=/opt/homebrew/opt/llvm@19/bin/clang`,
`NAGI_CXX_HEADERS=/opt/homebrew/opt/llvm@19/include/c++/v1`, and
`NAGI_TARGET_LD=/opt/homebrew/opt/lld@19/bin/ld.lld`.
M18 remains `PARTIAL` because authenticated providers for browser permissions,
file selection/transfers, clipboard, and IME are still outstanding.

## Completion Sweep — M20 synchronous loader validation boundary (2026-10-03)

Added tracked llama.cpp patch `0035-nagi-model-tensor-validation-sync.patch`.
Nagi now uses the same `ggml_validate_row_data` check synchronously for mapped
and file-backed tensors, preserving invalid-data reporting and failure status;
host builds retain the upstream asynchronous validation. A source-contract
regression covers the Nagi and host branches.

With LLVM 19 and the patch applied, the pinned Nagi target llama/ggml archives
build successfully. The exact-artifact `./nagi m20-granite-inference` attempt
still fails before guest image creation at final `nagi-init` link. Its unresolved
target C++ ABI diagnostics dropped from 88 to 75: all 13 loader future/thread
symbols disappeared, with no new symbols, while string, stream, locale,
filesystem, regex, and random-device providers remain missing. The run is
`out/logs/m20-granite-inference-sync-loader-20261003.log`; target archive
evidence is `out/evidence/m20-granite-inference-target-1791002538031677000/`.
The focused source-contract regression passed (1/1), and `./nagi fmt`,
`./nagi test`, `./nagi lint`, and `./nagi build` all passed. The generated
llama.cpp checkout confirms patch 0035 is applied. No model load or guest
inference is claimed. M20 remains `PARTIAL`.

## Completion Sweep — M25 failed-indicator cleanup (2026-10-03)

`PushToTalkService::begin` now calls `hide` if the trusted microphone
indicator's `show` returns an error, cleaning partial UI state before returning
`IndicatorUnavailable`. The provider and capture source are not started on
this path. A regression reproduced the stale indicator before the fix and
passes after it. All 21 `nagi-audio` tests and warnings-denied package Clippy
pass, along with `./nagi fmt`, `./nagi test`, `./nagi lint`, and `./nagi build`.
Current-source `./nagi m25` QEMU run `1791003281517596000` passes the existing
fixture acceptance. It uses no real audio input or TTS engine; M25 remains
`PARTIAL` for its production providers and command acceptance.

## Completion Sweep — M3 initial-frame fault reproduced and fix stress-tested (2026-10-03)

Independent confirmation of the ADR 0047 M3 root cause, on an Ubuntu host
with QEMU 8.2 and OVMF (q35, 4 vCPU, 8 GiB, `-no-reboot`). The test image
was a kernel-only ESP containing the release kernel with no features. It
reaches `Nagi M3 acceptance PASS` and then stops at M7 because the image has
no data disk. To create vCPU contention, eight instances ran in parallel on a
4-core host.

- **Before the fix (`956cd88`).** 3 of 24 boots died after
  `Nagi M3 scheduler workload START` or during SIPI. QEMU `-d int` captured
  the full chain for one of them. A timer IRQ (`v=20`) was delivered at
  `smp::thread_entry`'s first instruction with `SP=0000:0000000000000000`.
  That caused `v=0e e=0002 CR2=fffffffffffffff8`, then `v=08`, then
  `check_exception old: 0x8 new 0xd`, then `Triple fault`. The IRQ was
  already pending at `iretq` because the vCPU had stalled for longer than one
  10 ms APIC period inside the timer handler. That explains why the failure
  depends on host load.
- **After the fix (`b96cf54`).** 48 of 48 boots under the same 8-way
  contention reached `Nagi M3 acceptance PASS`. The 152 kernel library tests
  pass.
- **Equivalent alternative.** An alternative patch built the same 20-word
  frame and also removed `thread_entry`'s inline `mov rsp`. It passed 104 of
  104 contended boots. With a valid initial RSP that `mov rsp` is redundant,
  because it reloads the same value, but it is harmless.

Full `./nagi m22`, `./nagi m27` and M28 gates were not rerun here because
this host lacks the fetched Servo/Mesa inputs. M27 and M28 stay `PARTIAL`.

## Completion Sweep — AP #DF handler on IST1 (ADR 0048, 2026-10-03)

Each AP now loads its own kernel-only GDT and TSS with a dedicated IST1 #DF
stack, plus a shared AP exception IDT that the BSP fills before any SIPI.
`exception_entry` reports an AP fault and halts only that AP. The AP
trampoline now enables SSE (CR4.OSFXSR/OSXMMEXCPT, CR0.MP/NE) like the BSP.
Without that, the first compiler-emitted SSE store on an AP raised #UD.

Evidence:

- **Kernel host tests.** 153 pass, including the `ap_gdt` layout and the
  updated ADR 0047 GDT-transition source-order test.
- **Diagnostic probe.** The `m3-ap-double-fault-probe` kernel was run on
  QEMU/OVMF (q35, 4 vCPU). The last AP forces RSP=0 and pushes. The serial
  log shows `Nagi AP exception apic=3 vector=8 ... rsp=0x0
  cr2=0xfffffffffffffff8`, and QEMU logged no triple fault. This is the
  exact M3 failure signature, now caught and diagnosed.
- **Default kernel stress run.** 48 of 48 boots reached
  `Nagi M3 acceptance PASS` under 8-way parallel QEMU contention.

`./nagi m22`/`m27`/M28 were not rerun in this session because the host
lacks the Servo/Mesa inputs.

## Completion Sweep — NMI and #MC IST stacks (ADR 0049, 2026-10-03)

Every CPU with a TSS now gives NMI and #MC their own IST stacks, alongside
#DF. Slot selection is shared through `cpu_tables::exception_ist`: #DF uses
IST1, NMI IST2, and #MC IST3. The AP trampoline now also sets CR4.MCE.
Without it, a machine check shut an AP down instead of raising #MC.

Evidence:

- **Kernel host tests.** 154 pass.
- **IST probe.** The `m3-ap-ist-probe` kernel parks two APs with RSP=0 and
  interrupts disabled. A BSP NMI IPI to AP 3 is reported as `vector=2`. A
  QEMU monitor `mce` injection into CPU 2 is reported as `vector=18`. Both
  arrived with `rsp=0x0`, and QEMU logged no triple fault.
- **#DF probe.** It still reports `vector=8`.
- **Default kernel stress run.** 24 of 24 boots under 8-way contention
  reach `Nagi M3 acceptance PASS`.

Remaining gap: the BSP has no IST coverage before the M5 GDT switch.

## Completion Sweep — BSP tables from kernel entry (ADR 0050, 2026-10-03)

The BSP now installs its GDT, TSS (RSP0 and the #DF/NMI/#MC IST stacks) and
full exception IDT right after `serial_init` in `_start`. Before, it waited
for M5. M2's expected page fault is a temporary vector-14 overlay until M5
rebuilds the table. All M3 task frames now use the kernel selectors, and the
shared firmware-selector IDT is removed.

Evidence:

- **Kernel host tests.** 154 pass.
- **BSP probe.** With `m2-bsp-ist-probe`, the BSP spins with RSP=0 before
  M2. A monitor NMI is reported as `vector=2`, and an injected #MC as
  `vector=18`. QEMU logged no triple fault in either boot.
- **M5 reinstall check.** A scratch reinstall boot passes M3.
- **M2 self-test.** The page-fault markers are unchanged.
- **AP probes.** The #DF, NMI and #MC probes still pass.
- **Default kernel stress run.** 24 of 24 boots under 8-way contention pass
  M3.

Not run: an end-to-end M5 boot, which needs the real init image, and
`./nagi m22`/`m27`/M28. Both need the Servo/Mesa/relibc inputs that are
absent on this host.

## Completion Sweep — link-time entry tables (ADR 0051, 2026-10-03)

`_start` is now assembly. Its first three instructions load link-time GDT,
TSS and IDT tables (`lgdt`, `ltr`, `lidt`). From then on, NMI, #DF and #MC
use their own IST stacks, and any other exception escalates to a reported
#DF. `_start` then jumps to `nagi_kernel_entry`, which installs the full
BSP tables as before (ADR 0050).

Evidence:

- **Kernel host tests.** 154 pass.
- **Linked ELF.** The descriptor and gate fields were checked in the linked
  file.
- **Entry probe.** With `entry-ist-probe`, the BSP spins with RSP=0 on the
  link-time tables. An NMI is reported as `vector=2` and an injected #MC as
  `vector=18`, with no triple fault.
- **Escalation check.** A scratch `ud2` on the link-time tables is reported
  as `vector=8`.
- **Other probes.** The BSP and AP probes and the M2 markers are unchanged.
- **Default kernel stress run.** 24 of 24 boots under 8-way contention pass
  M3.

The only remaining firmware-table window is the two instructions before
`lidt`.

## M18 clipboard provider (2026-10-05)

Added `user/nagi-clipboard` (ADR 0053), a bounded user-space clipboard
service. Init owns it and registers Albert as a READ/WRITE client. Reads
consume a one-shot paste gesture and writes require a recent activation, both
scoped per tab; only Albert's device-input routing holds the gesture source.
This closes the gap left by Servo's unchecked `navigator.clipboard.readText()`
should its async clipboard preference be enabled. Albert's Servo
`ClipboardDelegate` and address-bar Ctrl+C/X/V now use the service.

Real-QEMU bring-up found two Albert focus defects that had never been
exercised because earlier acceptance typed only into the address bar: the
active WebView was never given Servo keyboard focus, and the address bar kept
focus after submission. Both are fixed; address submission and page clicks
now move keyboard focus to the page.

`./nagi m18` gained a staged QMP input stage gated on
`Nagi M18 clipboard page READY`, and its serial validator now requires, in
order before the summary, the clipboard page marker, the ungestured-read
denial, and `Nagi M18 clipboard copy/paste PASS`, rejecting any other
clipboard denial. Local run `1791179851416415000` passed (evidence under
`out/evidence/m29-browser-1791179851416415000/`). That run used
`out/mesa-venv`, `OVMF_HOME=/opt/homebrew/share/qemu`, and Homebrew LLVM 19
(`NAGI_TARGET_CLANG`, `NAGI_CXX_HEADERS`, `NAGI_TARGET_LD`); without the venv
Mesa configuration stops at its Mako check, and without `OVMF_HOME` the QEMU
host probe finds no OVMF pair.

Verification: `nagi-clipboard` 10 tests, `nagi-albert` 67 tests with
`m18-acceptance`, `nagi-cli` 210 unit + 21 integration tests, warning-denied
Clippy and rustfmt for all three. Public CI has not run this change.

Observed but not addressed: the fixture's input-field text is not visibly
painted in the Softpipe frame (the DOM value is confirmed through the page
title), the chrome still shows the last HTTPS URL because the fixture is
loaded directly rather than through browser state, and a faint previous-page
heading remains visible. M18 stays `PARTIAL` for IME, download/upload, and
production permission/IPC providers.

## M18 Japanese IME (2026-10-05)

Added `user/nagi-ime` (ADR 0054), a bounded user-space Japanese input method:
romaji-to-hiragana composition using common IME conventions, Space candidate
cycling, Enter commit, Escape cancel, Backspace edit, F6/F7 hiragana/katakana,
and Ctrl+Space, Zenkaku/Hankaku, Henkan, and Muhenkan mode keys. Candidates
come from a `CandidateSource`; 0.1 ships hiragana/katakana only. Kanji
conversion is deferred beyond Nagi 0.1 by explicit user decision because it
needs a dictionary with licence and size implications.

Albert tracks Servo's `EmbedderControl::InputMethod` request per tab and
offers page keys to the IME only while a text field is focused. It sends
composition start/update/end events to Servo and swallows the press and
release of IME-consumed keys. The address bar does not use the IME in 0.1.

`./nagi m18` adds a third QMP stage gated on `Nagi M18 IME page READY`
(Ctrl+Space, `nihongo`, Enter); the serial validator requires
`Nagi M18 IME page READY` and `Nagi M18 IME commit PASS` after the clipboard
evidence. The fixture page reports `compositionend` data and the field value,
and the guest requires `にほんご` committed after the pasted token with exactly
one composition. Fixture title traces are truncated on UTF-8 boundaries.
Local run `1791180856033701000` passed on the first attempt (same
environment as the clipboard run); evidence and SHA256SUMS are under
`out/evidence/m29-browser-1791180856033701000/`.

Verification: `nagi-ime` 15 tests, `nagi-albert` 68 tests with
`m18-acceptance`, `nagi-cli` 211 unit + 21 integration tests, warning-denied
Clippy and rustfmt. Public CI has not run this change. The Softpipe
screenshot does not show input-field text, so kana rendering inside form
fields is not yet visually verified; the committed value is verified in the
DOM.

## M18 text rendering and bundled fonts (2026-10-05)

Investigating unpainted form-field text showed that Servo on Nagi painted no
text at all. `platform/nagi/font_list.rs` (patch 0006) reported an empty
system-font registry, so pages without web fonts had no usable face. The
three M18 HTTPS frames were the same background-only image (checksum
`0x5a9955c5`); the acceptance accepted them because it checked only TLS
evidence and a nonzero checksum. Earlier M18 "rendered" evidence therefore
did not show page text.

Fix (ADR 0055):

- `third_party/fonts.lock` pins Noto Sans Regular/Bold, Noto Sans JP Regular
  (JP subset OTF), and both OFL 1.1 texts by URL at an immutable revision,
  size, and SHA-256. `./nagi fetch` downloads them to `out/cache/fonts/`,
  rejects mismatches, and writes `manifest.tsv`.
- `nagi-init` embeds the manifest files for Servo builds and publishes them
  through a new bounded read-only static-file table in `nagi-posix` under
  `/system/fonts/` (`open`, `read`, `fstat`, `stat`, file-backed `mmap`).
- Servo patch 0026 registers the families, prefers Noto Sans JP for Japanese
  text, and maps all generic families to Noto Sans. Diagnostic patch 0027
  (bounded to 64 lines) showed every face failing at `Mmap::map`: Nagi's
  file-backed `mmap` and `munmap` rejected lengths that were not a page
  multiple and filled read-only mappings in place. Both now round lengths up
  to whole pages as POSIX requires, fill through a writable mapping, and then
  apply the requested protection.
- Servo-enabled boot images use an `llvm-objcopy --strip-all` copy of the init
  ELF; the fonts had pushed the 133.9 MB ELF past the 133.8 MB FAT12 per-file
  limit. The symbol-bearing ELF stays in `target/`.
- Each HTTPS page now reports `ink_pixels` (non-background pixels in Servo's
  own frame); the host validator requires at least 200, so blank pages fail.

Real-QEMU bring-up also fixed two acceptance-harness defects: Albert now
paints each frame Servo reports ready (Servo waits for that before producing
the next frame, which had left the final screenshot on a stale preedit), and
the clipboard scenario sends Ctrl+C, the destination click, and Ctrl+V as
separate QMP stages gated on `Nagi M18 clipboard copy observed` and
`Nagi M18 clipboard destination focused`, because keyboard and pointer are
separate virtio queues whose relative order is not preserved.

Local `./nagi m18` run `1791185005598104000` passed: three TLS-verified
HTTPS pages with `ink_pixels=5632`, clipboard copy/paste, and IME commit; the
screenshot shows `nagi-clip-7f3a` and `nagi-clip-7f3aにほんご` rendered in the
fields. Evidence and SHA256SUMS are under
`out/evidence/m29-browser-1791185005598104000/`. Verification: `nagi-cli` 217
unit + 21 integration tests, `nagi-posix` 24, `nagi-albert` 72, warning-denied
Clippy and rustfmt. Public CI has not run this change.

After merging `origin/main` (isolated processes, Supervisor, and kernel
exception-table work), the first M18 run read example.com's frame before its
content was painted (`ink_pixels=0`, a different uniform frame): the HTTPS
wait loop also left Servo's first ready frame unconsumed. The HTTPS pages now
use the same bounded frame settling as the clipboard and IME phases before
the evidence frame is read. Post-merge run `1791185927312101000` passed with
`ink_pixels=6283` per site; evidence under
`out/evidence/m29-browser-1791185927312101000/`.

## M18 chrome layout and content navigation (2026-10-05)

- **Layout:** Servo's viewport had covered the whole 320x200 surface, with
  Albert's 48 px toolbar and an unbackgrounded status line drawn over the
  top of the page. The status strip is now part of the chrome (opaque
  background, `PAGE_TOP` = 62 px), Servo renders a 320x138 viewport placed
  below it, page input coordinates are offset to match, and clicks on the
  strip are inert.
- **Content navigation:** Albert's browser state only knew navigations it
  requested, so link, script, or embedder-loaded navigations left the tab
  title, address bar, and history stale. `BrowserState::content_navigated`
  records URLs Servo reports outside a pending Albert navigation: HTTP(S)
  destinations become history entries; other schemes (such as `data:`) update
  the display only and are never persisted. An address the user is editing is
  preserved. The M18 fixture now reaches the chrome this way, and the
  validator requires `Nagi M18 browser content navigation PASS` before the
  clipboard steps.
- **Chrome glyphs:** the address-bar bitmap font gained the remaining URL
  characters (`, ; ! @ ~ ' ( ) [ ] * $`); a test now covers every
  URL-permitted ASCII character.

Local `./nagi m18` run `1791195781053517000` passed (HTTPS `ink_pixels=4051`
per site, content navigation, clipboard, IME); the screenshot shows the
fixture URL and title in the chrome and the page below it. Evidence under
`out/evidence/m29-browser-1791195781053517000/`. `nagi-albert` 79 tests,
`nagi-cli` 255 tests, warning-denied Clippy.

## M18 file upload through a trusted picker (2026-10-05)

Page file inputs had been dismissed because Nagi had no picker. Albert now
keeps Servo's `FilePicker` request and shows a first-party picker modal
(opaque scrim, localized title `Choose a file` / `ファイル選択`) listing
regular files in the user's `/Documents` folder, filtered by the input's
`accept` extensions, sorted and bounded to 12 entries. While it is open,
device input goes to the picker (Up/Down/Enter/Escape and row clicks); only
the file the user chooses is passed to Servo with `select` + `submit`, which
is the selection-as-consent rule in the specification. Requests Servo hides
are dismissed. `nagi-posix` `opendir` now lists any VFS directory, not only
the root, so `std::fs::read_dir` works for the picker.

The M18 scenario writes `/Documents/nagi-upload.txt`, loads a page with a
`.txt` file input, clicks it through QMP, chooses the file with Enter, and
requires the page's `FileReader` to report the name, 14-byte size, and
contents. Local run `1791196426142952000` passed with all earlier phases;
evidence under `out/evidence/m29-browser-1791196426142952000/`.
`nagi-albert` 86 tests and `nagi-cli` 256 tests pass with warning-denied
Clippy.

Still open for M18: download destinations (the pinned Servo has no download
callback), the production authenticated permission policy/IPC provider, and
interactive QEMU permission-prompt acceptance. Later title-only changes (for example a page
updating `document.title`) are not yet reflected in the tab title.

## Nagi 0.2 integration-line checkpoints (merged 2026-10-05)

The sections below were recorded on `codex/integration-next-phase` before it
merged with the 0.1 release line on `main`. Their M17/M18 status lines are
historical: M17 and M18 later passed on the 0.1 line (see the completion
sweep above). The 0.2 workstream checkpoints remain current for their
workstreams.

## Diagnostics workstream checkpoint (2026-09-26)

Workstream `diagnostics` is integrated on `codex/integration-next-phase` from
`codex/ws-diagnostics` (source base `c1506888655123d819ec75be66891f0cd5477533`).
It is `PARTIAL`: its host diagnostics contract is implemented and verified,
while VM smoke remains bounded by an existing guest boot acceptance timeout.
This checkpoint does not change M17's recorded status or take ownership of
Activity, Wayback, Capability, App SDK, or other workstreams.

Implemented a versioned structured event and verification-report contract,
bounded crash/fatal capture with a sink interface, privacy-class redaction,
health-check registration and scoped aggregation, and the `nagi diagnostics`,
`nagi verify`, and `nagi smoke` commands. Added a JSON Schema, CLI and contract
tests, and local-first diagnostics documentation. Reports are emitted to a
file only when `--output` is explicitly supplied. Crash persistence remains a
portable contract until the target diagnostics/VFS boundary is available.

Evidence from the integration checkout:

- `cargo test -p nagi-cli --locked --offline` — PASS, 95 unit tests and 24 CLI integration tests.
- `cargo fmt --all -- --check` — PASS.
- `cargo clippy -p nagi-cli --all-targets --locked --offline -- -D warnings` — PASS; the CLI target compiles cleanly, with three existing `target_os = "nagi"` configuration warnings emitted by the libc dependency.
- `cargo check --manifest-path tools/nagi-bootstrap/Cargo.toml --locked --offline` — PASS after adding its direct `serde` dependency for the shared CLI library.
- `python3 -m json.tool docs/testing/diagnostic-report.schema.json >/dev/null` — PASS for JSON syntax. Full JSON Schema validation was not run because no schema validator is installed; the CLI contract test also checks schema versions and diagnostic event round-trip.
- `nagi diagnostics --scope diagnostics --json`, `nagi verify --scope diagnostics --json`, and `nagi smoke --host-only --json` — PASS after wiring the workstream health check to the existing `nagi dev verify` state validator.
- `./target/debug/nagi smoke --vm --json` — FAIL, classified as `VM` / `ACCEPTANCE`: QEMU did not exit within the existing 30-second M1/M7 acceptance window. Retrying with the pinned nightly toolchain on `PATH` passed the earlier Cargo channel mismatch and reached QEMU, but hit the same timeout. This is an existing guest boot acceptance boundary; no M17 or guest implementation was changed here.
- `git diff --check` — PASS. The report command smoke results above were produced from the host executable and are not target-test evidence.

The `workstreams` health check now invokes the owner-provided DF-01 validator
and passes on this integration registry. A CI run on this integrated head is
still required. The source-branch Actions run `36215571919` failed while
compiling `nagi-bootstrap` because its package dependencies did not include
the shared library's serialization dependencies; this integration adds the
missing direct dependency and verifies the standalone package locally. VM
smoke still times out before guest boot acceptance, so this workstream remains
`PARTIAL`. Mainline M17 remains `BLOCKED`; M18 remains `NOT STARTED`.

## Acceptance-CI workstream checkpoint (2026-09-26)

The host-side acceptance registry and runner from branch
`codex/parallel-acceptance-ci` are integrated on
`codex/integration-next-phase`. The CI host jobs now use the central runner and
publish its reports/logs; the target job routes M17 through the registered
existing `m17_servo_first_web_pixel.sh` wrapper. Its required real guest pixel
checksum and acceptance marker remain unchanged. The current M17 state remains
`BLOCKED`; M18 remains `NOT STARTED`.

The source branch CI run `35975771855` failed the host Clippy gate on
`duration.subsec_nanos() / 1_000` in `user/nagi-net/src/smoltcp_stack.rs`.
The integration branch already has the equivalent `subsec_micros()` conversion;
the focused x86_64 Apple-target `nagi-net` tests (8 total) and Clippy pass.
The integrated `nagi-cli` suite passes 102 unit and 26 CLI tests, the bootstrap
acceptance suite passes 7 tests, and the host acceptance run passes both M0
cases. Public CI for this integrated runner is pending. The runner itself does
not turn filtered or missing evidence into a pass; no milestone status is
inferred from host acceptance.

### First-party integration branch CI (2026-09-26)

Source run `36230579035` at `6cc3255` and documentation/state run
`36231127467` at `efa1c37` completed with both Ubuntu and Windows host jobs
passing. Both target jobs built through UEFI, then the unchanged real M17
QEMU acceptance timed out at `GL context creation started`; neither produced
a real pixel checksum or M17 PASS marker. The later root run `36233551349`
advanced through context creation into Servo construction before observing the
pthread-create `EAGAIN`, so that later trace is the current failure evidence.
The first-party workstream remains `PARTIAL`; its target runtime behavior was
not accepted by these runs.

## Human CLI terminal-control escaping checkpoint (2026-09-26)

Commit 7bd2afab365ccc1c450ec2e370ba15d9cc915ddc hardens human-readable
developer output against terminal-control injection from repository state and
diagnostic metadata. Status/resume lines, the verify summary, diagnostic
console summaries, and diagnostics-bundle host/commit headers now escape
control characters before display. JSON report output remains serialized by
the JSON encoder.

Four regression tests cover hostile workstream state, diagnostics-bundle
headers, and diagnostic stage/output-path summaries. The complete nagi-cli
suite passes (107 unit tests and 26 CLI integration tests), as do formatting,
Clippy with warnings denied, `./nagi dev verify` (18 registered workstreams;
11 state files), and `git diff --check`. The test build still reports three
existing `target_os = "nagi"` configuration warnings from the vendored libc
dependency. No guest, kernel, loader, or third-party source changed.

Actions run `36233551349` was built from parent commit
`3d2001c58f64cd5a3f63751224c2fb21ee325e40`. Ubuntu and Windows host jobs
passed. Its target acceptance completed with failure after QEMU timed out
during Servo construction; it produced no M17 checksum or PASS marker.

Run `36235206493` at `7caf740` exposed a Windows-only regression-test fixture
failure: Windows does not allow the test's newline/ESC filename. The fixture
now uses a valid cross-platform output filename while retaining a separate
Unix-only hostile-filename case. On run `36235660277` at `dd192ca`, both host
jobs passed, including Windows M0 launcher acceptance; the target job has
advanced to the real M17 QEMU acceptance. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

## M17 pthread-create diagnostic checkpoint (2026-09-26)

Target run `36233551349` passed Ubuntu/Windows host jobs and target builds
through UEFI, then QEMU created the Softpipe context and entered Servo
construction. A Servo thread spawn returned `EAGAIN` from `pthread_create`
(`nagi errno`); abort was redirected and QEMU timed out at 120 seconds. There
was no kernel `SYS_THREAD_CREATE rejected` reason and no real first-pixel
checksum or PASS marker. The runner's stage label alone does not establish
the root cause.

Commit `582b5f64585054be354b7d3f9379cfa3054f8828` adds a fixed-buffer,
allocation-free user-space trace for the three POSIX bridge outcomes: an
occupied child slot, failed fixed-stack mmap, and native thread-bridge
rejection. Each failure line reports the attempt, stage, actual 16 KiB bridge
stack size, and returned pthread error. Guest logging is limited to the first
eight failures; return and errno behavior and the single-child bridge remain
unchanged. The formatter is a host-testable module because the guest ABI file
is not part of host test builds.

Verification on that source commit: all 9 `nagi-posix` tests pass, including
two formatter bounds/content tests; package formatting and Clippy with
warnings denied pass; the Nagi user-target library check passes with five
existing visibility/dead-code warnings. Actions run `36236310925` for this
commit has both host jobs passed, and its target job is pending behind the
earlier target run. The real QEMU result is pending. M17 remains `BLOCKED`;
M18 remains `NOT STARTED`.

## M17 pthread bridge capacity audit (2026-09-26)

Source review of integration commit `582b5f64585054be354b7d3f9379cfa3054f8828`
confirms that the current native bridge has one child execution slot. The
POSIX adapter can separately return `EAGAIN` before the syscall for a held,
non-detached child. The kernel bridge also accepts creation only from the main
thread, atomically reserves one child state, and writes the child context to
slot 1.
This made thread capacity a concrete candidate for the observed Servo
`pthread_create` failure. Public owner-branch run `36232073962` at
`18ee17a` confirms the specific rejection: its kernel trace said
`SYS_THREAD_CREATE rejected: child slot occupied`, then recorded the already
mapped 16 KiB child stack. The failing call therefore reached the native
thread bridge with a mapped stack and was rejected because the bootstrap child
slot was occupied.

The confirmed kernel child-slot rejection means supporting additional
simultaneous threads requires a scheduler/kernel thread-context change owned
by the M17 runtime workstream and outside this integration branch's permitted
paths. The root trace may still distinguish the POSIX early-return branch
from a native syscall rejection. No concurrency is emulated in user space and
no thread failure is converted into success.

Root run `36235660277` completed with both host jobs passing and the target
acceptance failing. QEMU's real serial log reached `Servo construction started`,
then recorded the Servo profile thread panic with `pthread_create` error 11
(`EAGAIN`), followed by abort redirection; QEMU did not exit within the
acceptance's 120-second window. The serial log SHA-256 is
`8be7a9676148b1915c146fed25d932bf2ae6158603f65bf468012c821d00830d`. The run
produced no first-pixel checksum or guest PASS marker. Its acceptance report
incorrectly labeled the failure stage `link`: the diagnostic summary's plural
`undefined symbols: 0` matched a broad linker substring, while the QEMU
`did not exit within 120 seconds` wording was not recognized as a timeout.
The host classifier now recognizes that timeout wording and requires the
singular linker diagnostic `undefined symbol:`; regression tests preserve
timeout and linker distinctions. This changes reporting only; the acceptance
verdict remains FAIL. Trace-enabled run `36236310925` has started its target
job; its M17 QEMU result is pending. M17 remains `BLOCKED`; M18 remains
`NOT STARTED`.

Integration commit `4c453969ebd303b7a9ec40803f3b4f61b17d8d75` fixes that
acceptance-stage classifier and adds regression coverage for the observed
timeout plus the plural `undefined symbols: 0` diagnostic summary. The CLI
suite passes (107 unit and 26 integration tests), as do package formatting and
warning-denied Clippy. CI run `36239110612` is validating this commit; its
Ubuntu and Windows host jobs passed; its target job is queued behind the
active target run. Run `36236310925` remains the trace-enabled M17 target run
and is building Nagi user init. Neither run has produced a new guest pixel
result yet.

The M17 owner branch added commit `71fd5c33` for a bounded cooperative
bootstrap scheduler with 16 thread slots. This kernel and syscall change is
outside `codex/integration-next-phase`'s permitted paths and remains with its
owner. Its public run `36237832887` completed with failure: QEMU timed out
after 120 seconds at Servo construction and produced no pixel checksum or
PASS marker. The captured excerpt contains no `SYS_THREAD_CREATE rejected`
line, so this run does not show whether the scheduler dispatched the new
thread before the hang.

Both host jobs in `36237832887` failed the same
`m17_mesa_link_does_not_force_duplicate_archive_members` source assertion,
which still expected the old direct `USER_TLS_CHILD_CONTROL_BASE` assignment.
The local owner branch is one commit ahead at `4d79bbe`, where the assertion
was changed to the new `user_tls_control_base` helper; that commit is not on
the remote branch used by this CI run, so it has no CI verification yet. The
owner worktree also has a dirty status-document change and remains untouched.
The owner-branch scheduler does not change M17's recorded `BLOCKED` state.

## M17 trace acceptance status update (2026-09-26)

Root trace run `36236310925` at `582b5f64585054be354b7d3f9379cfa3054f8828`
passed both host jobs, built Nagi user init and the UEFI loader, and entered
the real M17 first-web-pixel acceptance at 11:56 UTC. At 12:10 UTC the target
job was still in that acceptance step; GitHub does not expose its logs until
the step completes. No new guest checksum or PASS marker is available, so M17
remains `BLOCKED` and M18 remains `NOT STARTED`.

The timeout-classifier run `36239110612` passed both host jobs but its pending
target job was canceled before execution. Newer run `36240025711` at
`faeeb22f9c8e6f6acf0ef8ba237cda59593c8ca1` passed both host jobs and is the
newest target run pending behind `36236310925`. The developer status command
now lists all queued and in-progress CI runs before completed runs.

## M17 trace acceptance result (2026-09-26)

Run `36236310925` completed with both host jobs passing and the target job
failing in the real QEMU first-web-pixel acceptance. Its serial log reached
`Servo construction started`, then recorded
`pthread_create failed attempt=2 stage=native-thread-create-rejected
bridge_stack_bytes=16384 pthread_error=11`. QEMU did not exit within 120
seconds. There is no real pixel checksum or guest PASS marker. The serial log
SHA-256 is
`6b1d0c7449561a180d7b676061d72fba0d5bacfe4b57ace13d5032ff2ea7b98b`.

The trace distinguishes native thread-bridge rejection from the POSIX early
return and stack-allocation failure branches; it does not report the kernel's
specific rejection reason. Owner-branch run `36232073962` separately recorded
`SYS_THREAD_CREATE rejected: child slot occupied` after mapping the same
16 KiB stack. The root run's older `582b5f6` acceptance summary still labeled
the timeout `stage=link`; it predates the classifier correction in `4c45396`.
The newer classifier/status run `36240025711` passed both host jobs and has
started its target build; it is now bootstrapping pinned Servo source. M17
remains `BLOCKED`; M18 remains `NOT STARTED`.

## M17 classifier validation run status (2026-09-26)

Run `36240025711` at `faeeb22f9c8e6f6acf0ef8ba237cda59593c8ca1` passed both
host jobs, built Nagi user init and the UEFI loader, and entered the real M17
first-web-pixel QEMU acceptance at 12:50 UTC. The target result is pending.
This run contains the timeout-classifier correction; its final diagnostic
will verify that the QEMU timeout is no longer mislabeled as a linker failure.
M17 remains `BLOCKED` until a real guest pixel checksum and acceptance marker
are present; M18 remains `NOT STARTED`.

## M17 classifier validation result (2026-09-26)

Run `36240025711` at `faeeb22f9c8e6f6acf0ef8ba237cda59593c8ca1` completed
with both host jobs passing and the target acceptance failing. Its failure
report correctly labeled the QEMU result `stage=timeout`; QEMU did not exit
within 120 seconds. The serial trace again reached Servo construction and
recorded `pthread_create failed attempt=2
stage=native-thread-create-rejected bridge_stack_bytes=16384 pthread_error=11`.
The serial log SHA-256 is
`58a39abf231fa1fc6db060e847735bd994e07c12334e6be73cc23b050581da2e`.
There is no real pixel checksum or guest PASS marker. The classifier fix is
verified; M17 remains `BLOCKED` and M18 remains `NOT STARTED`.

## VFS multi-block regular files (2026-10-05)

The User Data VFS stored each regular file in one 1 KiB block, and the POSIX
layer could only replace a whole file at offset 0. ADR 0056 extends regular
files to the standard ext2 pointer layout (12 direct blocks plus one
single-indirect block, 268 KiB), keeping the on-disk format compatible:
existing single-block files are read unchanged and new single-block files are
written identically. `Vfs::read_at` / `Vfs::write_at` add ranged I/O;
`write`, `truncate`, removal, and replacement grow or release every block and
keep free counts exact; bytes past the end of a file stay zero; a failed
growth releases its blocks and leaves the inode unchanged. The read-only
integrity check validates every direct and indirect pointer. `nagi-posix`
reads and writes at the descriptor offset and takes sizes from metadata.
`MAX_SMALL_FILE_SIZE` (1 KiB) keeps the old bound for whole-file `mmap` and
the Recovery, Desktop, M19, and M22 record buffers.

This also fixed M18 persistence: Albert's session/history/bookmark snapshot
had been capped at 1 KiB and every save reported `SAVE CAPACITY`, so browser
state never persisted in the M18 scenario. The snapshot bound is now 16 KiB
on both sides of the ABI, and the M18 validator requires
`Nagi M18 browser storage SAVE PASS` with no failed save.

Verification: `libnagi` 49 tests (five new multi-block tests, including
rollback on a failed growth and integrity checks after shrink/remove),
`nagi-posix` 24, `nagi-albert` 86, `nagi-cli` 257; warning-denied Clippy on
the x86_64 host target. QEMU regressions on the new VFS all passed: `./nagi
m18` (three saves, restore, upload), `./nagi m19`, `./nagi m22`,
`./nagi m27`, and `./nagi m29`. The first M18 attempt stalled in firmware
before any kernel marker during its second boot; the identical rerun passed.
M18 evidence: `out/evidence/m29-browser-1791204206460013000/`.

## M18 downloads (2026-10-05)

Servo patch 0028 implements the HTML "download the hyperlink" step that the
pinned Servo left as a TODO. A user-activated `<a download>` whose URL is
`data:`, `blob:`, or same-origin is fetched in the document's context
(credentials included, CSP enforced, at most 16 MiB) and handed to the
embedder as `EmbedderMsg::DownloadRequested`; clicks without transient user
activation download nothing, and cross-origin links are followed instead.
Albert saves a download only when the same tab received a trusted page click
or key press within about 5 s, into `/Downloads`, with the page's suggested
name sanitized, kept within the 32-byte VFS name limit (extension
preserved), and de-duplicated with ` (n)`. Multi-block VFS files (ADR 0056)
make room for real downloads.

The M18 scenario loads a page whose script calls `a.click()` on load; the
guest requires that this produced no request and no file, then a QMP click
must save `/Downloads/nagi-download.txt` with the link's exact contents.
Local run passed (`Nagi M18 download saved path=/Downloads/nagi-download.txt
bytes=16`); evidence under `out/evidence/m29-browser-1791205629214489000/`. `nagi-albert` 87 tests and `nagi-cli`
258 tests pass with warning-denied Clippy.

## M18 interactive site-permission acceptance (2026-10-05)

No web page could reach Albert's permission prompt: Servo's Permissions,
StorageManager, and Notification APIs are all preference-disabled by
default. Albert now enables the Notification API
(`dom_notification_enabled`), so `Notification.requestPermission()` on a
secure page sends a real `PermissionRequest` through Albert's broker and
localized modal. Nagi has no notification presenter yet, so granted
notifications are not displayed. `navigator.storage` stays disabled:
`persist()` reached the prompt and was allowed, but never resolved against
Servo's client storage on Nagi (recorded, not fixed).

After the three HTTPS pages, the M18 scenario calls
`Notification.requestPermission()` on `example.net` through Servo's
`evaluate_javascript`, waits for Albert's prompt
(`Nagi M18 permission prompt READY`), and the harness clicks Allow at
(253,145) — a coordinate pinned by a `nagi-albert` unit test — then returns
the pointer. The validator requires the prompt, `ALLOWED_BY_USER`, and the
page receiving `granted`. Local run passed; evidence under `out/evidence/m29-browser-1791206717974462000/`.

With this, every M18 deliverable in the specification has a real-QEMU
evidence path: chrome, tabs, address bar, navigation, history/bookmarks/
session persistence (saves now succeed), downloads, uploads, clipboard, IME
(kana; kanji deferred by ADR 0054), site permissions, and HTTPS. M18 stays
`PARTIAL` until authoritative CI (`nagi-target`) passes this full scenario
on `main`; then it can be recorded as `PASS`, with the known limits above
(in-process browser services, no notification presenter, no kanji
conversion, the StorageManager hang).

## VFS inode generations and M19 file identity (2026-10-05)

ADR 0057 closes M19's "identity across delete/recreate and inode reuse"
blocker. The VFS now stores ext2 `i_generation`: each new file or directory
in a slot gets a generation above any previously issued for it, cleared
inodes keep theirs, and pre-existing inodes (0) read as generation 1 so
their handles and Search records keep working without migration.
`FileHandle` validates the generation, so a handle to a deleted file is
rejected after its slot is reused; `FileMetadata` exposes the generation.
M19 Files indexing keys identity on (inode, generation) via the new
`nagi.files.vfs_generation` attribute and removes a record whose inode was
reused by another file.

`./nagi m19` now deletes and recreates `nagi-m19-reuse.txt`, requires the
same inode with a higher generation, a new `ObjectId`, and the old record
gone (`Nagi M19 trace inode reuse assigned a new ObjectId`), and passes.
`./nagi m22`, `m27`, `m29`, and `m18` also pass on this VFS. `libnagi` 51
tests (two new), `nagi-cli` 259, warning-denied Clippy.

M19 remains `PARTIAL`: continuous synchronization with production
Files/page producers is still open; Search indexes files when its producer
runs.

## M23 live Browser Context from Servo (2026-10-05)

Albert now implements the public `PublicBrowserContextApi` against live Servo
state, closing the first item of the M23 workstream's remaining work (the API
had only a test fixture). `LiveBrowserContext` reads the active WebView's URL
and title and, as requested, `document.body.innerText` and the current
selection through Servo's `evaluate_javascript`, each bounded in time; the
result reaches `nagi-ai` as an untrusted snapshot that `ContextResolver`
wraps as untrusted context. A host-tested `ContextSharingPolicy` denies every
caller until the user enables sharing, and then serves only the Nagi Bar
identity (`AppId::from_identifier(b"org.nagi.bar")`).

The M18 scenario resolves the Nagi Bar's request on `example.net` through
`ContextResolver::resolve_with_browser_api`: it must be denied while sharing
is off (`Nagi M23 browser context DENIED without user sharing`), then return
the real title, URL, and page text once sharing is enabled
(`Nagi M23 live browser context PASS`, 1301 visible bytes). The acceptance
enables sharing on the user's behalf; there is no settings UI for it yet.

M23 stays `PARTIAL`: the Nagi Bar UI, authenticated caller binding for the
context request, and the "Summarize this page" acceptance (which needs M20
guest inference) remain. `nagi-albert` 91 tests, `nagi-cli` 260 tests,
warning-denied Clippy.
