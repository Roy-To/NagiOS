# M8 CLI Foundation Implementation Plan

**Goal:** Add a real bounded serial `nsh` inside Nagi, basic guest file
commands, bounded process/memory/log inspection, and a QEMU-backed interactive
acceptance path without changing the accepted M7 `nagi run` behavior.

**Architecture:** Kernel low-level console input and structured diagnostic
snapshots; user-space shell and VFS commands; host CLI only builds a dedicated
shell image and transports serial bytes through QEMU TCP.

## Task 1: Define and test the M8 ABI

- Add stable console-read, process-info, memory-info, and log-read constants.
- Add bounded `repr(C)` snapshot structures and no-std wrappers in `libnagi`.
- Add pure tests for command parsing, ABI sizes, output bounds, and invalid
  command handling before the implementation.

## Task 2: Implement kernel low-level console and diagnostics

- Add non-blocking COM1 byte input with the existing serial primitive.
- Add capability-free self-inspection syscalls that accept only mapped writable
  user buffers and return fixed snapshots.
- Maintain a bounded serial log ring populated by actual kernel/user serial
  writes; expose only a bounded copy operation.
- Add kernel tests for syscall numbers, buffer bounds, and snapshot contracts.

## Task 3: Implement the bounded user-space `nsh`

- Add a fixed-size line reader using the real console-read syscall.
- Add parser/dispatcher for the M8 command surface.
- Extend the bounded root VFS only as needed for real `mkdir`, `rm`, and
  `mv`; preserve ext2 metadata checks and generation validation.
- Format output from real guest results and emit the terminal acceptance marker
  only after the scripted command checks pass.

## Task 4: Add a dedicated interactive QEMU CLI path

- Build a feature-selected shell init image while preserving the default M7
  image.
- Add QEMU serial TCP transport with bounded connection and command-script
  handling; retain serial logs and fail on missing guest markers.
- Add `nagi shell` parsing and host unit tests for its marker/transport policy.

## Task 5: Add M8 acceptance and verify

- Add PowerShell and Git Bash acceptance scripts that send commands to the
  guest and assert ordered guest-produced output.
- Run focused tests, workspace tests, fmt, clippy, host/user/kernel/loader
  builds, `nagi doctor`, M5-M7 regressions, and both M8 acceptance scripts.
- Update `docs/implementation_status.md` only after the real M8 acceptance
  passes, then commit the accepted milestone.

