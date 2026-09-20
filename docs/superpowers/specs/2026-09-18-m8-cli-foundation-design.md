# M8 CLI Foundation Design

## Goal

Provide a small, real Nagi-native serial shell (`nsh`) that can inspect the
guest filesystem and the current process/memory/log state. M8 keeps the
existing `nagi run` M7 acceptance path unchanged and adds a separate
`nagi shell` path for interactive serial acceptance.

## Architecture

- `nagi-init` remains a user-space bootstrap process. The shell parser and
  filesystem policy stay in user space and use the existing user-space VFS.
- The kernel adds only bounded low-level console input and diagnostic snapshot
  syscalls. They validate the caller's writable user mapping before copying
  data and expose no paths, host handles, or arbitrary kernel memory.
- The serial device remains the kernel's low-level COM1 primitive. QEMU's
  serial TCP chardev is only the development transport for real guest serial
  input/output; it does not implement shell behavior on the host.
- M8 builds a feature-selected shell init image. The default M5-M7 init image
  keeps its one-shot process-exit behavior and acceptance markers.
- The bounded bootstrap image window is expanded from the M5 eight-page limit
  to sixteen pages for the M8 shell's real parser and diagnostics; it remains
  fixed, non-demand-paged storage and is still rejected when exceeded.
- The shell uses a bounded command line and fixed-size output buffers. No
  unbounded parser, host filesystem fallback, or generated response is used.

## Command surface

The initial command set is intentionally bounded:

```text
pwd
ls
cd /
cat NAME
echo TEXT
touch NAME
write NAME TEXT
cp SOURCE DEST
mv SOURCE DEST
rm NAME
mkdir NAME
nagi ps
nagi mem
nagi log
help
exit
```

All file commands operate on the mounted guest ext2 root through `libnagi`.
`cd` is root-only in this slice because nested path resolution is not yet an
M8 requirement; it rejects paths instead of pretending to change directory.

`nagi ps`, `nagi mem`, and `nagi log` consume structured, bounded kernel/user
ABIs. The process snapshot describes the real bootstrap process, the memory
snapshot reports the mapped bootstrap ranges, and log output is copied from
the kernel's bounded serial log ring.

## Acceptance boundary

The M8 acceptance script will:

1. clean only repository-owned outputs;
2. run `nagi shell` against a real QEMU guest and the same persistent VirtIO
   Block data disk used by M7;
3. send shell commands over the QEMU serial TCP chardev;
4. require guest-produced output for `pwd`, `ls`, `cat`, `nagi ps`, `nagi mem`,
   and `nagi log`, plus a final `Nagi M8 acceptance PASS` marker;
5. require the M0-M7 boot markers before the shell marker.

The host may transport commands and collect the serial stream, but it may not
interpret or synthesize guest command results.
