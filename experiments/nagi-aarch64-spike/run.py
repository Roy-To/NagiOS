#!/usr/bin/env python3
"""Run the real guest and retain stdout/stderr, invocation, digest and exit."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

MARKERS = [b"SPIKE ARM64 EL1 PL011 PASS", b"SPIKE SVC VECTOR ERET PASS",
           b"SPIKE GENERIC COUNTER PASS", b"SPIKE COMPLETE PASS"]


def accepted(returncode, serial, timed_out):
    positions = [serial.find(marker) for marker in MARKERS]
    return (returncode == 0 and not timed_out and b"FAIL" not in serial
            and all(p >= 0 for p in positions) and positions == sorted(positions)
            and all(serial.count(marker) == 1 for marker in MARKERS))


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--qemu", default=os.environ.get("QEMU_AARCH64", "qemu-system-aarch64"))
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--virtualization", choices=["off", "on"], default="off")
    parser.add_argument("--output", type=Path, default=Path("out"))
    args = parser.parse_args()
    root = Path(__file__).resolve().parent
    binary = (args.binary or root / "target/aarch64-unknown-none/release/nagi-aarch64-feasibility-spike").resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    data = binary.read_bytes()
    if data[:6] != b"\x7fELF\x02\x01" or int.from_bytes(data[18:20], "little") != 183:
        parser.error("requires a little-endian ELF64 aarch64 executable")
    command = [args.qemu, "-machine", f"virt,gic-version=2,virtualization={args.virtualization}", "-cpu", "cortex-a53",
               "-accel", "tcg", "-smp", "1", "-m", "128M", "-display", "none", "-monitor", "none",
               "-serial", "stdio", "-no-reboot", "-nic", "none", "-semihosting-config", "enable=on,target=native",
               "-kernel", str(binary)]
    timed_out = False
    try:
        result = subprocess.run(command, capture_output=True, timeout=15, check=False)
        serial, stderr, returncode = result.stdout, result.stderr, result.returncode
    except subprocess.TimeoutExpired as error:
        serial, stderr, returncode = error.stdout or b"", error.stderr or b"", None
        timed_out = True
    (args.output / "serial.log").write_bytes(serial)
    (args.output / "stderr.log").write_bytes(stderr)
    passed = accepted(returncode, serial, timed_out)
    report = {"scope": "independent feasibility spike; NOT Nagi runtime acceptance", "command": command,
              "binary_sha256": hashlib.sha256(data).hexdigest(), "binary_bytes": len(data),
              "serial_sha256": hashlib.sha256(serial).hexdigest(), "returncode": returncode,
              "timed_out": timed_out, "outcome": "PASS" if passed else "FAIL",
              "qemu_version": subprocess.check_output([args.qemu, "--version"], text=True).splitlines()[0]}
    (args.output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(serial.decode("ascii", errors="replace"), end="")
    print(json.dumps(report, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
