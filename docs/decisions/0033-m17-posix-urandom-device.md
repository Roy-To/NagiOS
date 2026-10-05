# M17 POSIX entropy device for AWS-LC

Status: accepted for the M17 bootstrap
Date: 2026-09-27
Milestone: M17 — Servo Bootstrap

## Context

Actions run 36295909293 (run #288, head `8018045`) passed host checks, the
Nagi target build through UEFI, and the Servo embedded-resource preflight. The
real QEMU guest then stopped at TLS prewarm with:

```text
failed to open /dev/urandom: Unknown error
```

The exact emitter is AWS-LC 0.45.0's
`aws-lc/crypto/rand_extra/urandom.c::init_try_urandom_once()`. Servo's TLS
prewarm calls `aws_lc_rs::secure_random.fill()`. AWS-LC's generic POSIX
provider uses the raw `getrandom` path only for its Linux target; Nagi is a
custom target, so it opens `/dev/urandom`. Nagi's POSIX adapter currently
resolves file paths through the capability-scoped persistent VFS and exposes
no random-device descriptor. The open therefore fails with `ENOENT`; relibc's
errno message table renders that value as `Unknown error`.

Nagi already has a guest entropy boundary: `libnagi::random_fill()` invokes
`SYS_RANDOM_GET`, which obtains bytes from the guest VirtIO RNG. Rust std and
MozJS use this existing boundary directly, but AWS-LC uses its standard POSIX
file interface.

## Decision

- Recognize the exact `/dev/urandom` path in the Nagi POSIX adapter and return
  a stateless random-device file descriptor without consulting or modifying
  the persistent VFS.
- Implement descriptor reads with `libnagi::random_fill()` so every byte
  comes from the existing kernel `SYS_RANDOM_GET` and guest VirtIO RNG path.
- Return `EIO` if entropy acquisition fails. Never return host-derived,
  deterministic, or partial substitute bytes.
- Support the descriptor lifecycle and read operations used by POSIX clients:
  `open`, `fcntl`, `read`, and `close`.

## Verification

Local verification passes: all 82 `nagi-cli` library tests and 18 CLI
integration tests; custom-target `cargo check -p nagi-posix --lib
--target targets/x86_64-unknown-nagi-user.json -Zbuild-std=core,alloc
--locked --offline`; formatting; and `git diff --check`. The target check
reports five existing warnings in unrelated POSIX declarations.

The next public `nagi-target` QEMU acceptance must advance through TLS
prewarm, real Servo and WebView construction, and produce the real first-web-
pixel checksum and M17 PASS marker. Keep M17 `BLOCKED` until that evidence is
present; M18 remains `NOT STARTED`.
