# ADR 0013: M13 PAL and POSIX in User Space

- Status: Accepted
- Date: 2026-09-18

## Context

M13 requires Rust std/POSIX compatibility, but Nagi's architecture forbids
turning the kernel into a Linux/POSIX compatibility layer. The existing M7
storage and M12 networking implementations already provide the appropriate
user-space boundaries.

## Decision

Implement a Nagi PAL and a narrow POSIX C ABI in user space. Use generation-
checked descriptor ownership, existing capability handles, and native spawn
semantics. Keep `fork` unsupported rather than adding a broad kernel primitive.
Increase the bounded user image limit only as needed for the compatibility
runtime and retain fixed bounds.

Third-party compatibility code is pinned and patched in Nagi-owned metadata;
it is never silently edited in a fetched source cache. Upstream Rust `std`
support is a separate technical claim and may be recorded only after a real
target build and guest acceptance.

## Consequences

The first M13 vertical slice can be tested with real Nagi VFS/network services
without introducing high-level kernel APIs. POSIX applications that require
unsupported Linux semantics receive a deterministic error and must use the
Nagi-native interface instead.
