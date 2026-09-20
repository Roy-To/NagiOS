# M13 Rust std / POSIX Compatibility Design

## Goal

Provide the first user-space Nagi PAL and POSIX compatibility foundation for
Rust and C guest programs. This milestone must keep filesystem, networking,
process, and synchronization behavior behind Nagi user-space services and the
existing low-level kernel ABI.

## Invariants

- The kernel remains responsible only for low-level execution, memory, IPC,
  capabilities, and device transport.
- No filesystem, socket, process, or POSIX high-level syscall is added to the
  kernel.
- The PAL uses the existing Nagi storage and networking interfaces; host files,
  host sockets, and host process execution are never used as guest behavior.
- Rights remain capability-scoped and handles cannot be strengthened by a
  receiver.
- Native process creation is spawn-oriented. `fork` is not a foundational
  primitive for M13.
- Rust standard-library integration is claimed only when the target-specific
  PAL is actually built and exercised. A facade or host-only test is not a
  substitute for guest `std` support.

## Architecture

`user/nagi-pal` is a `no_std + alloc` crate exposing bounded primitives:

- a generation-checked integer file-descriptor table;
- file and directory adapters over the M7 VFS capability;
- socket adapters over the M12 `nagi-net` path;
- mutex, condition-variable, and TLS state;
- monotonic/realtime clock and sleep helpers;
- bounded poll/select and native spawn result types.

`user/nagi-posix` exposes a narrow C ABI over the PAL for malloc/free, stdio,
file/stat and directory operations, mmap-style memory operations, clocks,
sleep, poll, DNS/socket adapters, environment, pthread/TLS, and posix_spawn.
Unsupported operations return a deterministic errno; they do not escape to the
host.

The M13 guest acceptance image contains:

1. a Rust PAL/allocator contract using real guest VFS and M12 networking;
2. a freestanding C object linked into the guest image through the POSIX ABI;
3. a small no-std-capable crates.io library (`itoa`) exercised in guest code.

## Image and test constraints

The current user image limit is 16 pages. M13 raises this bounded limit to 256
pages (1 MiB) to accommodate the compatibility layer and a linked Rust `std`
image while retaining a fixed upper bound below the fixed 2 MiB user stack
mapping. The Loader and kernel reject images above 4 MiB, and the existing VFS
TLS page and stack remain fixed guest mappings.

Acceptance markers are emitted only after their corresponding guest operation
returns success:

```
Nagi M13 Rust PAL PASS
Nagi M13 C POSIX PASS
Nagi M13 OSS library PASS
Nagi M13 acceptance PASS
```

## Explicit non-goals

- Linux or another host OS as the production runtime;
- a high-level POSIX kernel layer;
- arbitrary `fork`, arbitrary shell execution, or unrestricted process access;
- fake network/filesystem results;
- claiming upstream Rust `std` support without building and running it;
- replacing Servo or changing unrelated browser architecture.
