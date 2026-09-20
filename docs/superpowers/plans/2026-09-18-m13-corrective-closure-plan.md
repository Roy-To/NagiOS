# M13 Corrective Closure Implementation Plan

> **For agentic workers:** Execute this plan incrementally. After each task, run the focused build/tests and keep `docs/implementation_status.md` truthful. Do not start M14.

**Goal:** Close the Pre-M14 M13 gaps with real Nagi guest functionality and one authoritative real-QEMU acceptance gate.

**Architecture:** Keep the kernel limited to low-level memory, timer, wait, thread/process primitives and capability validation. Keep files, sockets, DNS, POSIX and Rust compatibility in user space. Extend the existing Nagi ABI and bootstrap address-space implementation only where a low-level primitive is required; do not introduce host filesystem, host sockets, Linux runtime dependencies, or fake acceptance results.

**Tech Stack:** Rust `no_std`, existing x86-64 Nagi kernel, `libnagi`, `nagi-pal`, `nagi-posix`, pinned smoltcp, relibc backend, Cargo locked builds, QEMU acceptance scripts.

## Global constraints

- Preserve the existing M0-M12 acceptance paths and capability checks.
- Keep M13 `PARTIAL` until every required guest operation and the unified gate pass.
- Unsupported `fork()` remains deterministic `ENOSYS`.
- Use up to ten evidence-based repair attempts per blocker; never weaken or delete tests.
- Update `docs/implementation_status.md` after each coherent verified stage.

## Tasks

1. **Add low-level memory and timer ABI primitives.**
   - Inspect and extend `crates/nagi-abi`, `user/libnagi`, `kernel/src/syscall.rs`, and `kernel/src/user_process.rs` with bounded anonymous mapping, unmapping, protection attenuation, realtime, sleep and bounded wait/readiness primitives.
   - Back mappings with Nagi-owned pages/VMO state and enforce user-range, alignment, rights and resource limits.
   - Add unit tests for range validation, rights attenuation, invalid arguments and timer behavior.
   - Run `cargo test -p nagi-kernel --lib --locked`, `cargo test -p libnagi --locked`, and focused cross-checks.

2. **Wire PAL/POSIX memory and time.**
   - Replace `ENOSYS` mmap/munmap/poll placeholders in `user/nagi-posix` with Nagi ABI calls and implement bounded `mprotect`.
   - Implement monotonic/realtime clock conversion and real guest sleep/wake through the timer primitive.
   - Add host unit tests for invalid/unsupported paths and guest-side assertions for mapping, protection, unmapping and elapsed sleep.
   - Run focused PAL/POSIX tests and the existing M13 POSIX build.

3. **Complete the user-space socket/readiness surface.**
   - Extend `user/nagi-net`'s smoltcp-backed `SocketApi` with bounded socket lifecycle, connect, send, receive, DNS and readiness operations.
   - Add POSIX `socket`, `connect`, `send`, `recv`, close and DNS wrappers without adding socket syscalls to the kernel.
   - Keep all traffic behind the capability-scoped raw VirtIO frame device and test the service state machine deterministically.
   - Run nagi-net tests, M12 QEMU acceptance, then the focused POSIX guest build.

4. **Implement the thread bridge and real TLS isolation.**
   - Add the smallest native Nagi thread/wait bridge compatible with the current bootstrap image and expose it to PAL/POSIX.
   - Replace process-global TLS storage with per-thread storage, implement bounded pthread create/join and condition wait/signal, and preserve mutex correctness.
   - Add a guest test in which two real threads set and read distinct TLS values.
   - Run kernel, PAL/POSIX, Rust std and QEMU smoke tests after each ABI change.

5. **Implement native spawn and result waiting.**
   - Define a capability-scoped spawn request using Nagi object/handle identity, bounded argument data, attenuated inherited rights and an exit-result wait path.
   - Reuse existing ELF/process primitives; do not add a path-based high-level kernel spawn syscall or make `fork()` foundational.
   - Add deterministic failure tests and a guest parent/child proof with argument/result validation.

6. **Strengthen compatibility tests and OSS path.**
   - Make the Rust std image exercise allocator, `Arc`/`Mutex`, real thread/TLS, VFS, advancing time/sleep and Nagi socket/DNS.
   - Make the C/POSIX/relibc image exercise the intended backend, mmap, time, socket/DNS, poll and spawn boundaries.
   - Exercise one small pinned OSS library through the compatibility path and document its source lock/provenance.

7. **Add and run one authoritative M13 gate.**
   - Add `nagi m13` plus PowerShell and Git Bash acceptance wrappers with ordered real-QEMU markers for every required dimension and regressions.
   - Ensure the final acceptance marker is emitted only after all operations pass.
   - Run focused builds, full workspace checks, relevant M7-M12 regressions, both M13 QEMU scripts and source-lock verification.

8. **Record the truthful checkpoint.**
   - Update `docs/implementation_status.md` with exact commands, logs, source-lock state, remaining gaps and final M13 status.
   - Commit coherent verified stages. Do not create M14 work or mark M13 `PASS` without the unified gate.

