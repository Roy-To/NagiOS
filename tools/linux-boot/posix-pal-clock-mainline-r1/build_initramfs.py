#!/usr/bin/env python3
"""Build a one-entry newc initramfs for the PAL/POSIX Linux smoke."""

from __future__ import annotations

import argparse
import os
import stat
from pathlib import Path


def append_entry(
    output: bytearray,
    name: str,
    *,
    inode: int,
    mode: int,
    payload: bytes = b"",
    rdevmajor: int = 0,
    rdevminor: int = 0,
) -> None:
    encoded_name = name.encode("ascii") + b"\0"
    fields = (
        inode,
        mode,
        0,
        0,
        1,
        0,
        len(payload),
        0,
        0,
        rdevmajor,
        rdevminor,
        len(encoded_name),
        0,
    )
    if any(value < 0 or value > 0xFFFFFFFF for value in fields):
        raise ValueError("newc field overflow")
    output.extend(b"070701" + b"".join(f"{value:08x}".encode("ascii") for value in fields))
    output.extend(encoded_name)
    output.extend(bytes(-len(output) % 4))
    output.extend(payload)
    output.extend(bytes(-len(output) % 4))


def build(init: bytes) -> bytes:
    if not init:
        raise ValueError("init ELF is empty")
    output = bytearray()
    inode = 1
    for name, permissions in (("dev", 0o755), ("proc", 0o555), ("sys", 0o555), ("tmp", 0o1777)):
        append_entry(output, name, inode=inode, mode=stat.S_IFDIR | permissions)
        inode += 1
    for name, permissions, major, minor in (
        ("dev/console", 0o600, 5, 1),
        ("dev/null", 0o666, 1, 3),
        ("dev/ttyS0", 0o600, 4, 64),
    ):
        append_entry(
            output,
            name,
            inode=inode,
            mode=stat.S_IFCHR | permissions,
            rdevmajor=major,
            rdevminor=minor,
        )
        inode += 1
    append_entry(output, "init", inode=inode, mode=stat.S_IFREG | 0o755, payload=init)
    inode += 1
    append_entry(output, "TRAILER!!!", inode=inode, mode=0)
    output.extend(bytes(-len(output) % 4))
    return bytes(output)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--init", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        metadata = args.init.lstat()
        if not stat.S_ISREG(metadata.st_mode):
            raise ValueError("init input must be a regular file")
        payload = args.init.read_bytes()
        archive = build(payload)
        flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
        descriptor = os.open(args.output, flags, 0o600)
        with os.fdopen(descriptor, "wb") as stream:
            stream.write(archive)
        print(f"cpio={args.output} bytes={len(archive)} entries=9")
    except (OSError, ValueError) as error:
        parser.error(str(error))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
