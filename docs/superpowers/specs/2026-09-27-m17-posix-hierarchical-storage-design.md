# M17 POSIX Hierarchical Storage Design

## Problem and intended outcome

CI run `36284231289` reached the real Servo constructor, completed resource
thread startup, and aborted immediately after storage-thread initialization
started. The M17 `_start` path runs the M7 disk acceptance and then enters
Servo without initializing the shared Nagi POSIX filesystem. In addition,
`nagi-posix` currently rejects every pathname containing an interior slash,
and the M7 VFS resolves entries only in its root directory. Servo's storage
threads create a temporary directory and then nested client-storage paths,
so the real guest POSIX filesystem must provide those operations.

The outcome is for Servo's storage threads to start on the existing
capability-backed M7 guest volume and for POSIX path operations to resolve
bounded nested paths through that volume. This keeps storage in the guest and
does not change the browser, the renderer, or the first-web-pixel acceptance.

## Scope and design

- After M7 persistence acceptance, initialize the existing Nagi POSIX VFS
  with the same block capability before entering Servo.
- Ensure the conventional `/tmp` directory exists on that VFS before Servo
  requests a temporary directory.
- Add bounded path traversal to `libnagi::storage::Vfs` using directory
  inodes and their existing `.` / `..` entries. Preserve current root-only
  entry points for M7 callers, and add path-aware create/open/mkdir/remove
  and metadata lookup for POSIX. Directory enumeration remains root-only
  until Nagi has descriptor-backed directory streams.
- Allow POSIX ABI paths up to a fixed 256-byte limit. Resolve components
  beneath the process root, interpret `.` and `..` without escaping the root,
  and reject a component that violates the VFS's existing 32-byte name limit.
- Preserve the bounded single-block directory/file model and truthful errors;
  do not add host filesystem access, symlinks, path escape, or silent success.

## Compatibility and failure behavior

Existing M7 root operations and serialized volume geometry remain unchanged.
Paths are rooted in the process's existing VFS capability. Opening an
uninitialized filesystem remains an error. Non-directory traversal, invalid
components, absent entries, full directories, and out-of-capacity operations
return their existing or specific storage errors.

## Verification

- Add memory-block-device tests for nested create/open/read, directory
  listing, empty-directory removal, persistence after remount, `.` / `..`
  resolution, and resolution that stays inside the process root.
- Add POSIX ABI/runtime coverage for nested paths and initialization behavior.
- Run focused `libnagi` and `nagi-posix` tests, formatting, affected target
  builds when locally available, and `git diff --check`.
- Use public `nagi-target` CI as the authoritative verification that Servo
  storage threads proceed and that real QEMU still produces the first web
  pixel checksum and M17 PASS marker.

## Non-goals

This change does not widen the VFS file-size limit, add multi-block files,
symlinks, arbitrary `openat` directory descriptors, or claim M17 acceptance
before the authoritative guest checksum is observed.
