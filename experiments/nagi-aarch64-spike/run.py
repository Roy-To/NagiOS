#!/usr/bin/env python3
"""Observe a QEMU run. This runner records a binary; it never attests its source."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

MARKERS = [
    b"SPIKE ARM64 EL1 PL011 PASS",
    b"SPIKE SVC VECTOR ERET PASS",
    b"SPIKE GENERIC COUNTER PASS",
    b"SPIKE COMPLETE PASS",
]
FAIL_MARKER = b"SPIKE FAIL exception-level"


class RunnerError(ValueError):
    pass


def sha256_bytes(data):
    return hashlib.sha256(data).hexdigest()


def sha256_file(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def accepted(returncode, serial, timed_out):
    positions = [serial.find(marker) for marker in MARKERS]
    return (
        returncode == 0 and not timed_out and b"FAIL" not in serial
        and all(position >= 0 for position in positions)
        and positions == sorted(positions)
        and all(serial.count(marker) == 1 for marker in MARKERS)
    )


def expected_el2_rejection(returncode, serial, timed_out):
    return (
        returncode == 1
        and not timed_out
        and FAIL_MARKER in serial
        and b"SPIKE COMPLETE PASS" not in serial
    )


def resolve_executable(value):
    found = shutil.which(value)
    candidate = found or value
    path = Path(candidate).expanduser().resolve(strict=True)
    if not path.is_file() or not os.access(path, os.X_OK):
        raise RunnerError(f"QEMU executable is not an executable file: {path}")
    return path


def valid_elf(data):
    return (
        len(data) >= 20
        and data[:6] == b"\x7fELF\x02\x01"
        and int.from_bytes(data[18:20], "little") == 183
    )


def run_guest(binary, qemu, virtualization, output, timeout=15):
    output = Path(output).expanduser().resolve()
    if output.exists() or output.is_symlink():
        raise RunnerError(f"output already exists: {output}")
    binary = Path(binary).expanduser().resolve(strict=True)
    if not binary.is_file():
        raise RunnerError(f"binary is not a regular file: {binary}")
    qemu_path = resolve_executable(str(qemu))
    data_before = binary.read_bytes()
    if not valid_elf(data_before):
        raise RunnerError("requires a little-endian ELF64 aarch64 executable")
    binary_sha256_before = sha256_bytes(data_before)
    qemu_sha256_before = sha256_file(qemu_path)
    qemu_version = subprocess.check_output(
        [str(qemu_path), "--version"], text=True
    ).splitlines()[0]
    command = [
        str(qemu_path), "-machine",
        f"virt,gic-version=2,virtualization={virtualization}",
        "-cpu", "cortex-a53", "-accel", "tcg", "-smp", "1", "-m", "128M",
        "-display", "none", "-monitor", "none", "-serial", "stdio",
        "-no-reboot", "-nic", "none", "-semihosting-config",
        "enable=on,target=native", "-kernel", str(binary),
    ]
    output.parent.mkdir(parents=True, exist_ok=True)
    output.mkdir()
    timed_out = False
    try:
        result = subprocess.run(command, capture_output=True, timeout=timeout, check=False)
        serial, stderr, returncode = result.stdout, result.stderr, result.returncode
    except subprocess.TimeoutExpired as error:
        serial, stderr, returncode = error.stdout or b"", error.stderr or b"", None
        timed_out = True
    binary_sha256_after = sha256_file(binary)
    qemu_sha256_after = sha256_file(qemu_path)
    passed = (
        accepted(returncode, serial, timed_out)
        and binary_sha256_before == binary_sha256_after
        and qemu_sha256_before == qemu_sha256_after
    )
    (output / "serial.log").write_bytes(serial)
    (output / "stderr.log").write_bytes(stderr)
    report = {
        "scope": "independent feasibility spike; NOT Nagi runtime acceptance",
        "source_attestation": "NOT_CLAIMED_BY_RUNNER",
        "command": command,
        "qemu_path": str(qemu_path),
        "qemu_sha256": qemu_sha256_before,
        "qemu_sha256_after": qemu_sha256_after,
        "virtualization": virtualization,
        "binary_path": str(binary),
        "binary_sha256": binary_sha256_before,
        "binary_sha256_after": binary_sha256_after,
        "binary_bytes": len(data_before),
        "serial_sha256": sha256_bytes(serial),
        "stderr_sha256": sha256_bytes(stderr),
        "returncode": returncode,
        "timed_out": timed_out,
        "outcome": "PASS" if passed else "FAIL",
        "qemu_version": qemu_version,
    }
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(serial.decode("ascii", errors="replace"), end="")
    print(json.dumps(report, indent=2))
    return report


def build_parser():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--qemu", default=os.environ.get("QEMU_AARCH64", "qemu-system-aarch64"))
    parser.add_argument("--binary", type=Path, help="binary to observe; this option does not prove its source")
    parser.add_argument("--virtualization", choices=["off", "on"], default="off")
    parser.add_argument("--output", type=Path, required=True)
    return parser


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)
    root = Path(__file__).resolve().parent
    binary = args.binary or root / "target/aarch64-unknown-none/release/nagi-aarch64-feasibility-spike"
    try:
        report = run_guest(binary, args.qemu, args.virtualization, args.output)
    except (OSError, RunnerError, subprocess.CalledProcessError) as exc:
        parser.error(str(exc))
    return 0 if report["outcome"] == "PASS" else 1


if __name__ == "__main__":
    raise SystemExit(main())
