# AGENTS.md — Nagi OS

This repository implements **Nagi OS 0.1 Developer Preview**.

## Primary specification

The highest-level implementation specification is:

`docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`

The current implementation state is recorded in:

`docs/implementation_status.md`

When instructions conflict, use this priority:

1. Explicit current user instruction
2. `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`
3. This `AGENTS.md`
4. `docs/implementation_status.md`
5. Existing implementation conventions

Do not silently change an architecture decision. If a deviation is required, document the reason in `docs/decisions/` before implementing it.

---

## Required workflow

Before making changes:

1. Read the relevant section of `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`.
2. Read `docs/implementation_status.md`.
3. Inspect the actual repository state.
4. Run `git status`.
5. Inspect relevant `git diff`.
6. Identify the current milestone and its acceptance criteria.

Implement milestones in order:

`M0 -> M1 -> M2 -> ... -> M30`

Do not mark a milestone complete until its acceptance criteria pass.

A milestone may be recorded only as:

- `PASS`
- `PARTIAL`
- `BLOCKED`
- `NOT STARTED`

---

## After each meaningful implementation step

1. Build the affected component.
2. Run focused tests.
3. Fix failures based on evidence.
4. Re-run the focused tests.
5. Run the milestone acceptance test when applicable.
6. Review relevant logs.
7. Update `docs/implementation_status.md`.
8. Leave the repository in a clean, resumable state.

When a milestone passes, continue to the next milestone unless an explicit user decision is required.

---

## Failure continuation rule

When a build or test fails:

1. Identify the concrete failure.
2. Form a specific hypothesis.
3. Make a targeted fix.
4. Re-run the smallest useful test.
5. Re-run the milestone acceptance test if the focused test passes.

Make up to **10 meaningful repair attempts** for a milestone blocker before declaring it blocked.

Do not repeat the same failing command without changing the implementation, configuration, or hypothesis.

If still blocked after meaningful attempts, record in `docs/implementation_status.md`:

- exact blocker;
- failing command/test;
- important error output;
- suspected root cause;
- fixes already attempted;
- next recommended experiment.

Do not hide a persistent failure and continue as if the milestone passed.

---

## Architecture rules

### Nagi is an independent OS

- Nagi is **not Linux-based**.
- Linux/Windows/macOS may be development hosts only.
- Do not introduce a production Linux runtime dependency.
- Do not use the host OS to fake guest functionality.

### Kernel boundary

The kernel owns low-level execution, memory, IPC and authority.

Do **not** add high-level kernel syscalls for:

- files;
- sockets;
- windows;
- audio;
- package management;
- AI.

Keep those in user-space services.

### Capability security

- Do not bypass capability checks.
- Do not grant all apps universal access as a temporary shortcut.
- Rights transferred through handles may be attenuated but never strengthened by the receiver.
- Owner is not a permanently omnipotent root process.
- Developer Mode is not equivalent to disabling security.

### AI security

- AI is user-space and untrusted.
- AI never receives arbitrary OS authority.
- AI must not execute arbitrary generated shell commands.
- Plans must pass deterministic validation/policy before execution.
- AI cannot elevate privileges or invoke Owner override by itself.
- Prefer Object IDs/handles over model-invented paths.

### POSIX compatibility

- POSIX compatibility belongs in user space.
- Do not add Linux/POSIX kernel semantics merely to make a port easier.
- Prefer Nagi PAL + relibc compatibility.
- Native process creation is spawn-oriented; `fork()` is not foundational.

### Language architecture

- English is Nagi's canonical internal language for identifiers, APIs, IPC,
  schemas, configuration keys, localization keys, diagnostics, logs, and
  machine-readable errors.
- `en-US` and `ja-JP` are equal first-class Nagi 0.1 user languages; Japanese
  is not a secondary or post-hoc translation tier.
- User-facing strings use the shared localization architecture and fall back
  from the selected locale to `en-US`.
- System language, region/locale, input language/keyboard, and Albert/AI
  conversation language are separate concepts.
- UTF-8 is the default internal text encoding.
- Read `docs/architecture/language-architecture.md` before changing language,
  locale, localization, first-party UI, or AI conversation behavior.

### Albert / Servo

- Albert uses **Servo** as the browser engine.
- Do not substitute Chromium, WebKit or NetSurf as a fallback for the Nagi 0.1 plan.
- Do not fake X11/Wayland.
- Keep Servo upstream changes minimal and explicit.
- Nagi-specific Servo changes must be tracked as reproducible patches/adapters.
- Browser web content is untrusted and must not directly call Nagi AI, filesystem or system APIs.

### Local AI

The Nagi 0.1 default Standard LLM is:

**IBM Granite 4.2 3B**

Additional bundled models:

- Qwen3 4B — alternative Standard
- Gemma 3 1B — Lite

Do not change the default merely because another model benchmarks slightly better. Change it only for a documented technical blocker or explicit user decision.

### AI provider architecture

- Nagi depends on typed AI capabilities and roles, not specific vendors or model families.
- Deterministic Fast Path, Decision capability, and Generative/Reasoning paths are distinct.
- Specialized Decision Providers are optional accelerators; Nagi 0.1 must work without Jev or any cloud decision service.
- Decision confidence may affect routing or escalation but never grants or strengthens authority.
- All probabilistic outputs still pass deterministic validation, policy, permission, and executor boundaries before side effects.
- Applications must not hardcode model/vendor names when a capability or role can be requested instead.
- `llama.cpp` / GGUF is the Nagi 0.1 Generative LLM runtime, not the universal runtime contract for every provider family.

### Reference machine

Nagi 0.1 official target:

- QEMU
- x86-64
- UEFI / OVMF
- q35
- 4 vCPU
- 8 GB RAM
- VirtIO Block
- VirtIO Network
- VirtIO GPU
- VirtIO Sound
- VirtIO RNG

Physical hardware support is not an Nagi 0.1 completion requirement.

---

## Prohibited shortcuts

Never obtain a milestone PASS by:

- deleting a failing test;
- weakening an assertion without architectural justification;
- hardcoding expected output instead of implementing the feature;
- returning fake network/AI/rendering results;
- disabling the broken feature;
- bypassing security;
- routing guest work through host Linux/Windows/macOS;
- directly editing cached third-party sources without preserving the change as a patch;
- claiming a test passed when it was not run.

Examples of prohibited host escape:

- using the host filesystem as if it were Nagi VFS;
- using host sockets instead of `nagi-net`;
- rendering Servo on the host and copying screenshots into Nagi;
- running Granite on the host and presenting the result as guest inference.

Test-only mocks are allowed only when the test explicitly targets orchestration rather than the real subsystem.

---

## Third-party source rules

External projects such as Servo, Mesa, relibc, llama.cpp and whisper.cpp must use pinned revisions.

Prefer:

- exact source revision;
- source/hash lock file;
- Nagi-owned patch directory;
- reproducible fetch/apply process.

Do not make undocumented edits inside fetched source caches.

---

## Generated files

Treat source IDL/specification files as authoritative.

Generated files should clearly state that they are generated.

Do not manually modify generated bindings unless the generator itself is being fixed.

---

## Build and test entry points

Prefer the repository-level developer interface:

```text
./nagi doctor
./nagi fetch
./nagi build
./nagi image
./nagi run
./nagi test
```

Use focused underlying Cargo/CMake/Meson commands when diagnosing a component, but keep `./nagi` as the supported top-level workflow.

---

## Repository state recovery

Do not depend on chat history to know what to do next.

If context is missing, compressed, stale or uncertain, reconstruct state from:

1. this file;
2. `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`;
3. `docs/implementation_status.md`;
4. `git status`;
5. `git diff`;
6. recent commits;
7. existing code;
8. test/build results.

Then continue from the earliest incomplete milestone.

Ask the user only when a genuine product/architecture decision, unavailable credential, external permission, or destructive choice cannot be resolved from the repository/specification.

---

## Definition of success

Success is not the number of files or lines generated.

Success means that each milestone leaves Nagi as a more complete, testable independent operating system while preserving:

1. correctness;
2. data integrity;
3. security boundaries;
4. diagnosability;
5. architectural cleanliness;
6. reproducibility;
7. responsiveness;
8. performance;
9. visual polish.

When forced to choose, preserve that order.
