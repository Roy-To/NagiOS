# ADR 0020: Keep M17 `fsync` in user space and expose block flush

## Status

Accepted for the M17 runtime repair. This decision does not mark M17 complete;
the real target link, UEFI boot, and first-web-pixel acceptance must still
pass.

## Context

The complete undefined-symbol inventory from target CI run #157 includes
SQLite's `fsync` call. Nagi's file descriptors and VFS are user-space
components, while the kernel owns the VirtIO block queue and the capability
check for block-device access. Returning success without asking the device to
flush would falsely promise durable storage.

## Decision

- Implement `fsync` in the Nagi POSIX/relibc user-space boundary and route it
  through the VFS to the backing block device.
- Add `SYS_BLOCK_FLUSH` as ABI number 28. Like block read/write, it accepts the
  caller's block capability and performs only the low-level device operation;
  it does not add a filesystem syscall.
- Negotiate `VIRTIO_BLK_F_FLUSH` when offered and issue a real VirtIO block
  flush request. If the device does not offer the feature or the request fails,
  the syscall and `fsync` report failure.
- Keep metadata, truncation, permission, ownership, and timestamp operations
  in the user-space EXT2/POSIX implementation.

## Consequences

The published syscall table gains one append-only number. Existing syscall
numbers remain unchanged. The target VFS can now distinguish durable flush
support from ordinary writes, and SQLite cannot mistake an unsupported flush
for durable completion.
