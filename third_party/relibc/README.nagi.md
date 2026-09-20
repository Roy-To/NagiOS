# Nagi relibc backend

This directory is vendored from relibc revision
`69bb008af1f6d93758631cf0df250500d53a065b`. The Nagi build selects the
Nagi-only backend in `src/nagi.rs`; it does not expose Linux or Redox syscall
ABIs and does not use the host C runtime.

The backend is intentionally an `rlib` dependency of the Nagi Rust `std`
image. Its exported C symbols forward to the user-space `nagi-posix` facade,
which owns the bounded allocator, VFS descriptors, synchronization/TLS, and
low-level Nagi service adapters. The upstream Linux/Redox implementation is
retained for provenance but is not compiled for `x86_64-unknown-nagi-user`.

The target-only backend also owns the bounded
`open_memstream`/`fwrite`/`fflush`/`fclose` slice used by Mesa's
software-rendering utility code. It publishes a real NUL-terminated buffer
through the POSIX output pointers and does not use a host stdio implementation.

The target-only backend also exports `mmap`, `munmap`, and `mprotect`. These
symbols forward to `nagi-posix`'s bounded VMO/VFS mapping facade so Mesa's
file-backed memory utility path uses guest mappings rather than a host memory
API. Mapping failure follows the POSIX `MAP_FAILED` contract.

The exact source revision and the Nagi adapter are recorded in
`third_party/sources.lock`.
