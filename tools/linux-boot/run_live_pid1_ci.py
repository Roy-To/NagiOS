#!/usr/bin/env python3
"""Run a fixed live-test manifest in an isolated TCG guest, fail closed.

Inputs are supplied artifacts, never generated substitutes. --manifest is a JSON
array of exactly 26 unique Rust test names. The prebuilt PID1 supervisor reads
/tests.txt and runs /testbin --exact --test-threads=1 NAME, issuing:
  LIVE_PID1_CI BEGIN NAME
  [Rust output: exact qualified NAME and test result: ok. 1 passed; ...]
  LIVE_PID1_CI PASS NAME
After all children exit successfully it prints LIVE_PID1_CI COMPLETE 26 and
powers off. The supervisor must mount proc/dev/sys and retain PID1 until then.
Markers are a test protocol, not authentication or production READY evidence.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import struct
import subprocess
import sys

import boot_acceptance
from build_dev_service_cpio import _entry
from replace_initramfs_init import RepackError, _validate_static_pie


COUNT = 26
NAME = re.compile(r"[A-Za-z_][A-Za-z_0-9]*(?:::[A-Za-z_][A-Za-z_0-9]*)+")
SUMMARY = re.compile(
    r"test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; "
    r"[0-9]+ filtered out; finished in .+"
)


def read_regular(path: Path, limit: int) -> bytes:
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
    fd = os.open(path, flags)
    with os.fdopen(fd, "rb") as stream:
        metadata = os.fstat(stream.fileno())
        if not stat.S_ISREG(metadata.st_mode) or not 0 < metadata.st_size <= limit:
            raise ValueError(f"invalid regular input or size: {path.name}")
        data = stream.read(limit + 1)
        if len(data) != metadata.st_size:
            raise ValueError(f"input changed or exceeded limit: {path.name}")
        return data


def validate_elf(data: bytes) -> None:
    if len(data) >= 64 and struct.unpack_from("<HH", data, 16) == (3, 62):
        _validate_static_pie(data)
        return
    if (len(data) < 64 or data[:7] != b"\x7fELF\x02\x01\x01"
            or struct.unpack_from("<HH", data, 16) != (2, 62)):
        raise ValueError("expected static x86_64 little-endian ET_EXEC ELF")
    offset = struct.unpack_from("<Q", data, 32)[0]
    size, count = struct.unpack_from("<HH", data, 54)
    if size != 56 or not 0 < count <= 1024 or offset + size * count > len(data):
        raise ValueError("invalid ELF program headers")
    loadable = False
    for index in range(count):
        kind, _, start, _, _, file_size, memory_size, _ = struct.unpack_from(
            "<IIQQQQQQ", data, offset + index * size
        )
        if kind in (2, 3):
            raise ValueError("dynamic ELF/loader dependencies are not allowed")
        if start + file_size > len(data) or file_size > memory_size:
            raise ValueError("invalid ELF segment")
        loadable |= kind == 1
    if not loadable:
        raise ValueError("ELF has no loadable segment")


def read_manifest(path: Path) -> list[str]:
    names = json.loads(read_regular(path, 65536))
    if (not isinstance(names, list) or len(names) != COUNT
            or any(not isinstance(name, str) or len(name) > 512
                   or NAME.fullmatch(name) is None for name in names)
            or len(set(names)) != COUNT):
        raise ValueError(f"manifest must contain exactly {COUNT} unique qualified test names")
    return names


def archive(init: bytes, test: bytes, names: list[str]) -> bytes:
    payload = bytearray()
    expected = {
        "init": init, "testbin": test,
        "tests.txt": ("\n".join(names) + "\n").encode("ascii"),
    }
    inode = 1
    for directory, mode in (("proc", 0o555), ("dev", 0o755),
                            ("sys", 0o555), ("run", 0o755), ("tmp", 0o1777)):
        _entry(payload, directory, inode=inode, mode=stat.S_IFDIR | mode, nlink=2)
        inode += 1
    for name, minor in (("dev/console", 1), ("dev/null", 3)):
        _entry(payload, name, inode=inode, mode=stat.S_IFCHR | 0o600,
               nlink=1, rdevmajor=5 if name == "dev/console" else 1,
               rdevminor=minor)
        inode += 1
    for name, data in expected.items():
        _entry(payload, name, inode=inode, mode=stat.S_IFREG |
               (0o644 if name == "tests.txt" else 0o755), nlink=1, payload=data)
        inode += 1
    _entry(payload, "TRAILER!!!", inode=inode, mode=0, nlink=1)
    result = bytes(payload)
    parsed_init, records = boot_acceptance._parse_newc_records(result)
    found = {name: data for name, _, data in records if name in expected}
    if parsed_init != init or found != expected or len(records) != 11:
        raise ValueError("generated CPIO failed strict round-trip validation")
    return result


def validate_serial(serial: str, names: list[str]) -> None:
    lines = serial.replace("\r\n", "\n").splitlines()
    index = 0
    active: str | None = None
    child_lines: list[str] = []
    complete = False
    powerdown = False
    for line in lines:
        if "Kernel panic" in line or "test result: FAILED" in line:
            raise ValueError("guest kernel/test failure")
        if line.startswith("LIVE_PID1_CI "):
            if line == f"LIVE_PID1_CI COMPLETE {COUNT}":
                if complete or active is not None or index != COUNT:
                    raise ValueError("premature or repeated completion")
                complete = True
            elif line.startswith("LIVE_PID1_CI BEGIN "):
                name = line.removeprefix("LIVE_PID1_CI BEGIN ")
                if complete or active is not None or index >= COUNT or name != names[index]:
                    raise ValueError("unexpected, duplicate, or reordered test start")
                active = name
                child_lines = []
            elif line.startswith("LIVE_PID1_CI PASS "):
                name = line.removeprefix("LIVE_PID1_CI PASS ")
                child_output = "\n".join(child_lines)
                results = re.findall(r"test result:[^\r\n]*", child_output)
                # kmsg may interrupt the libtest name/status line. Require the
                # exact name and successful one-test summary within this child's
                # marker interval, not a pristine `test NAME ... ok` line.
                named = re.search(r"(?<![A-Za-z_0-9:])" + re.escape(name)
                                  + r"(?![A-Za-z_0-9:])", child_output)
                # Some lifecycle tests start their own libtest subprocess.
                # The selected outer test's summary is always last, after
                # those children finish. Earlier summaries cannot replace it.
                if (active != name or named is None or not results
                        or SUMMARY.fullmatch(results[-1]) is None):
                    raise ValueError("PASS without exact child test success")
                active = None
                index += 1
            else:
                raise ValueError("unknown or failing guest protocol marker")
        elif active is not None:
            child_lines.append(line)
        elif "test result:" in line:
            raise ValueError("test result outside a child marker interval")
        if "Power down" in line:
            if not complete:
                raise ValueError("powerdown before complete test results")
            powerdown = True
    if not complete or not powerdown or active is not None or index != COUNT:
        raise ValueError("guest did not complete all tests and power down")


def write_new(path: Path, data: bytes) -> None:
    with path.open("xb") as stream:
        stream.write(data)


def run_helper(command: list[str], log: Path) -> int:
    with log.open("xb") as stream:
        process = subprocess.Popen(command, cwd=log.parent, stdin=subprocess.DEVNULL,
                                   stdout=stream, stderr=subprocess.STDOUT)
        interrupted = False

        def forward(signum: int, _frame: object) -> None:
            nonlocal interrupted
            interrupted = True
            if process.poll() is None:
                process.send_signal(signum)

        previous = {sig: signal.signal(sig, forward)
                    for sig in (signal.SIGINT, signal.SIGTERM)}
        try:
            status = process.wait()
        finally:
            for sig, handler in previous.items():
                signal.signal(sig, handler)
        return 1 if interrupted else status


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("kernel", "test-elf", "init-elf", "manifest", "output-dir"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--timeout-seconds", type=int, default=600)
    args = parser.parse_args()
    output: Path | None = None
    try:
        if not 1 <= args.timeout_seconds <= 600:
            raise ValueError("timeout must be 1..600 seconds")
        names = read_manifest(args.manifest)
        init = read_regular(args.init_elf, 32 * 1024 * 1024)
        test = read_regular(args.test_elf, 80 * 1024 * 1024)
        validate_elf(init)
        validate_elf(test)
        kernel = read_regular(args.kernel, 128 * 1024 * 1024)
        if (len(kernel) < 0x240 or kernel[0x1FE:0x200] != b"\x55\xaa"
                or kernel[0x202:0x206] != b"HdrS"
                or not struct.unpack_from("<H", kernel, 0x236)[0] & 1):
            raise ValueError("kernel is not an x86_64 Linux boot image")
        cpio = archive(init, test, names)
        qemu = shutil.which("qemu-system-x86_64")
        if qemu is None:
            raise ValueError("qemu-system-x86_64 is required; no fallback")
        parent = args.output_dir.parent.resolve(strict=True)
        destination = parent / args.output_dir.name
        # Both helper and QEMU use the fresh output directory as cwd. A short
        # relative socket stays in that directory without Unix path truncation.
        qmp = Path("qmp.sock")
        destination.mkdir(mode=0o700)  # Never reuse, overwrite, or clean a prior run.
        output = destination
        write_new(output / "kernel.bzImage", kernel)
        write_new(output / "initramfs.cpio", cpio)
        write_new(output / "manifest.json", json.dumps(names, indent=2).encode() + b"\n")
        helper = Path(__file__).resolve().with_name("run_qemu_child.py")
        command = [sys.executable, str(helper), "--timeout-seconds", str(args.timeout_seconds),
                   "--qmp-socket", str(qmp), "--qemu-log", str(output / "serial.log"),
                   "--", qemu, "-machine", "q35", "-accel", "tcg", "-cpu", "max", "-m", "2048",
                   "-smp", "4", "-nodefaults", "-no-user-config", "-display", "none",
                   "-monitor", "none", "-serial", "stdio", "-nic", "none", "-no-reboot",
                   "-kernel", str(output / "kernel.bzImage"),
                   "-initrd", str(output / "initramfs.cpio"),
                   "-append", "console=ttyS0 rdinit=/init quiet panic=-1",
                   "-qmp", f"unix:{qmp},server=on,wait=off"]
        status = run_helper(command, output / "supervisor.log")
        if status != 0:
            raise ValueError(f"QEMU helper failed with exit {status}")
        records = [json.loads(line) for line in
                   read_regular(output / "supervisor.log", 1024 * 1024).decode().splitlines()
                   if line.startswith("{")]
        if (len(records) != 1 or records[0].get("qemu_exit") != 0
                or records[0].get("timed_out") is not False
                or records[0].get("cancelled") is not False):
            raise ValueError("missing or failing QEMU termination record")
        validate_serial(read_regular(output / "serial.log", 32 * 1024 * 1024)
                        .decode("utf-8", errors="strict"), names)
        write_new(output / "result.json", json.dumps(
            {"passed": True, "tests": names, "count": COUNT, "qemu": records[0]},
            indent=2).encode() + b"\n")
        print(f"LIVE_PID1_CI_HOST PASS {COUNT}")
        return 0
    except (OSError, ValueError, RepackError, boot_acceptance.AcceptanceError) as error:
        print(f"live PID1 CI failed: {error}", file=sys.stderr)
        if output is not None:
            write_new(output / "failure.json", json.dumps(
                {"passed": False, "error": str(error)}).encode() + b"\n")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
