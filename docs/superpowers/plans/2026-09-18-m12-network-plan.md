# M12 Networking Implementation Plan

## 1. ABI and kernel device boundary

- Add stable raw network syscall numbers and a bounded frame size.
- Add a legacy VirtIO Net driver with RX/TX queues, PCI discovery, device
  status, queue validation, and a derived network capability.
- Pass the network capability through the bootstrap register contract and
  validate user frame ranges in the syscall layer.
- Add kernel unit tests for capability matching, frame bounds, checksums of
  queue state, and malformed device responses.

## 2. User-space driver and protocol library

- Add `user/nagi-net` as a no_std workspace crate.
- Implement packet readers/writers, checksums, ARP, IPv4, ICMP, UDP, DHCP,
  DNS, TCP state, and bounded HTTP helpers with deterministic tests.
- Add raw device wrappers to `libnagi` and an M12 guest acceptance app in
  `nagi-init`.

## 3. QEMU integration and acceptance

- Configure the existing QEMU command line with explicit legacy VirtIO Net.
- Add `nagi network` image/run orchestration and ordered guest-marker checks.
- Add PowerShell and Git Bash acceptance scripts with cleanup for the external
  HTTP fixture.
- Verify the response bytes are produced by the guest TCP/HTTP path and are
  not hard-coded.

## 4. Verification gate

- Run focused protocol and kernel tests after each subsystem.
- Run workspace fmt/test/clippy and all M12 builds.
- Run both real-QEMU acceptance paths and inspect serial logs.
- Update `docs/implementation_status.md` only after both acceptance paths pass.
