# Nagi OS 遯ｶ繝ｻImplementation Status

This file is the persistent implementation handoff for **Nagi OS 0.1 Developer Preview**.

It exists so that Codex can resume work from the repository without relying on previous chat context.

Primary specification:

`docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`

Repository instructions:

`AGENTS.md`

---

# 1. Current status

**Current milestone:** `M17 - Servo Bootstrap`
**Milestone status:** `BLOCKED`
**Next action:** M16 is PASS and M17 remains the active implementation
milestone. Continue the internal blocker-remediation work for the pinned
Servo/Surfman/Mesa/relibc/std vertical slice, then run the target build and
real QEMU first-web-pixel gate. Do not substitute another browser engine or
host rendering. M18 remains forbidden until M17 is formally PASS.

**Last updated:** 2026-09-23
**Last known repair checkpoint:** public CI run `35809154991` (#100) at
`409f15d` passed Servo bootstrap, then failed while building the pinned Mesa
Softpipe archive during explicit core-target selection. Target link, UEFI, and
QEMU were not reached. The current repair accepts Meson/Ninja target names
that contain `libmesa.a`, including `.p` output-layout forms, before
symbol-aware archive aggregation. This run is not target-link acceptance
evidence. No host libc, host filesystem, host rendering, or synthetic output is
used.
Target link, UEFI, and real QEMU first-web-pixel evidence remain required.
**Reference target:** QEMU x86-64 / q35 / UEFI / 4 vCPU / 8 GB RAM

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
| M0 | Repository / Toolchain / CI | PASS | `5f3b5b8`: configured executable/version probes, OVMF allow-list, clean safety, and launcher acceptance passed |
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
| M16 | Package / SDK | PASS | Out-of-tree SDK sample emitted a real NAPP artifact; `nagi-pkg` packaged it, the IDL generator reproduced the checked-in Rust/C bindings, Ed25519 signatures were verified with tamper rejection, and QEMU loaded the host `.xapp` through guest VFS for install/list/info/launch/update/atomic replace/remove. Focused host suite, target builds, signed package CLI, PowerShell wrapper, Git Bash wrapper, and `out/logs/m16-package.log` passed on 2026-09-19. |
| M17 | Servo Bootstrap | BLOCKED | The pinned Servo/Surfman/libc/mio/socket2/tokio patch boundary, Nagi static Mesa/Softpipe build path, relibc-header preparation, and real Servo-to-Nagi Surface adapter are implemented. Public CI run `35851514326` (#120) at `87cc024` passed Servo bootstrap, dependency validation, Mesa Softpipe, package, kernel, and target compilation, then failed at final target linking with `sw_screen_create_vk`, `wrapper_sw_winsys_wrap_pipe_screen`, `null_sw_create`, and `strspn`; UEFI, QEMU, and first-web-pixel acceptance were not reached. The current repair seeds the three real Softpipe loader/winsys symbols through the target link and adds target-owned guest-memory `strspn`. The run is not M17 acceptance evidence. M17 remains BLOCKED until target link, UEFI loader, real QEMU, and real Servo-generated first-web-pixel evidence pass. See ADR 0019. |
| M18 | Albert Browser | NOT STARTED | 遯ｶ繝ｻ|
| M19 | Semantic Layer / Search | NOT STARTED | 遯ｶ繝ｻ|
| M20 | AI Runtime / Granite | NOT STARTED | 遯ｶ繝ｻ|
| M21 | Planner / Validator / Executor | NOT STARTED | 遯ｶ繝ｻ|
| M22 | AI Safety / Undo Integration | NOT STARTED | 遯ｶ繝ｻ|
| M23 | Nagi Bar / Context / Albert AI | NOT STARTED | 遯ｶ繝ｻ|
| M24 | Embedding / Semantic AI | NOT STARTED | 遯ｶ繝ｻ|
| M25 | Voice | NOT STARTED | 遯ｶ繝ｻ|
| M26 | Qwen / Gemma / Automatic | NOT STARTED | 遯ｶ繝ｻ|
| M27 | A/B / Recovery | NOT STARTED | 遯ｶ繝ｻ|
| M28 | Integration / Stress | NOT STARTED | 遯ｶ繝ｻ|
| M29 | Developer Preview Polish | NOT STARTED | 遯ｶ繝ｻ|
| M30 | Nagi OS 0.1 Release | NOT STARTED | 遯ｶ繝ｻ|

---

# M17 - Servo Bootstrap (`BLOCKED`)

M17 remains `BLOCKED` as a truthful acceptance state; this is not a stop
condition and is not a first-web-pixel acceptance. The implementation now
applies sorted tracked Servo, Surfman, and libc patches, records generated
checkout revisions plus patch/worktree fingerprints, and refuses stale or
unsafe generated state without overwriting it.

The blocker inventory was reclassified on 2026-09-20.

Internal and actionable in this workstream:

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
| llama.cpp | Not pinned yet | 遯ｶ繝ｻ| Generative LLM runtime; Decision and Embedding providers are not fixed to it |
| whisper.cpp | Not pinned yet | 遯ｶ繝ｻ| STT |
| smoltcp | Not pinned yet | 遯ｶ繝ｻ| Network stack |

Model hashes/revisions should be added when model fetching is implemented.

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

### Current M17 continuation after CI run #122 (2026-09-23)

Public CI run `35857472389` (#122, head `63bb9b8`) failed during the pinned
Servo bootstrap before dependency-boundary, Mesa, target build, UEFI, or real
QEMU acceptance. The public check exposed only exit code 4, so no source or
linker conclusion is drawn from this run. The next repair preserves the
bootstrap failure and writes `out/logs/m17-bootstrap.log`, with a bounded
first-error annotation, so the exact pinned-source, patch-order, fingerprint,
or Servo Cargo-fetch failure can be corrected from evidence. M17 remains
`BLOCKED`; M18 remains `NOT STARTED`.

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
