#!/usr/bin/env python3
"""Check development Data persistence or prepare identity directories on a new copy."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import time
import uuid
from pathlib import Path

RUNNER = Path(__file__).with_name("run_qemu_child.py")
DISK_BYTES = 128 * 1024 * 1024
PROVISION_PHASE = "provision-identity-directories"
FAILURE_MARKERS = (
    "NAGI_DEV_DATA_OWNER_FAIL",
    "NAGI_DEV_DATA_OWNER_UNAVAILABLE",
    "NAGI_DEV_IDENTITY_DIRECTORIES_PROVISION_FAILED",
    "Kernel panic",
    "Attempted to kill init",
)


def run_phase(
    args: argparse.Namespace, phase: str, serial: str, filesystem_uuid: str
) -> None:
    phase_dir = Path(phase)
    (args.output / phase_dir).mkdir()
    socket = phase_dir / "qmp.sock"
    guest_log = phase_dir / "serial.log"
    cmdline = (
        f"console=ttyS0,115200n8 rdinit=/init panic=-1 loglevel=5 "
        f"nagi.dev_data.smoke={phase} nagi.dev_data.serial={serial} "
        f"nagi.dev_data.fs_uuid={filesystem_uuid}"
    )
    expected = re.compile(
        rf"^NAGI_DEV_DATA_OWNER_PASS phase={phase} serial={serial} uuid={filesystem_uuid} "
        r"mount_id=[0-9]+ retained_fd=true same_volume_notes_input=true "
        r"notes_pending=true owner_ended=true$",
        re.MULTILINE,
    )
    if phase == PROVISION_PHASE:
        expected = re.compile(
            r"^NAGI_DEV_IDENTITY_DIRECTORIES_PROVISIONED "
            r"accounts=accounts snapshots=snapshots owner_ended=true$",
            re.MULTILINE,
        )
    command = [
        sys.executable, "-B", str(RUNNER), "--timeout-seconds", "60",
        "--qmp-socket", str(socket), "--qemu-log", str(phase_dir / "qemu.log"),
        "--", args.qemu, "-machine", "q35,accel=tcg", "-cpu", "qemu64",
        "-m", "8192", "-smp", "4", "-kernel", str(args.kernel), "-initrd",
        str(args.initramfs), "-append", cmdline, "-nodefaults", "-no-user-config",
        "-display", "none", "-serial", f"file:{guest_log}", "-monitor", "none",
        "-no-reboot", "-drive", "if=none,file=data.raw,format=raw,id=nagi-data",
        "-device", f"virtio-blk-pci,drive=nagi-data,serial={serial}",
        "-qmp", f"unix:{socket},server=on,wait=off",
    ]
    # QEMU resolves its short Unix socket path relative to this run directory.
    with subprocess.Popen(
        command, cwd=args.output, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.PIPE, text=True,
    ) as process:
        log_path = args.output / guest_log
        deadline = time.monotonic() + 50
        while time.monotonic() < deadline and process.poll() is None:
            if log_path.exists():
                log = log_path.read_text(errors="replace")
                if expected.search(log) or any(marker in log for marker in FAILURE_MARKERS):
                    break
            time.sleep(0.1)
        try:
            stdout, stderr = process.communicate(input="QUIT\n", timeout=15)
        except subprocess.TimeoutExpired:
            process.terminate()
            process.communicate()
            raise
    print(stdout, end="", flush=True)
    if stderr:
        print(stderr, file=sys.stderr, end="", flush=True)
    lines = stdout.splitlines()
    result = json.loads(lines[-1]) if lines else {}
    log = log_path.read_text(errors="replace") if log_path.exists() else ""
    if (
        process.returncode != 0
        or result.get("qemu_exit") != 0
        or result.get("timed_out") is not False
        or result.get("cancelled") is not False
        or any(marker in log for marker in FAILURE_MARKERS)
        or not (match := expected.search(log))
    ):
        raise SystemExit(f"{phase} boot failed: {log_path}")
    print(match.group(), flush=True)


def copy_development_disk(source: Path, destination: Path) -> str:
    # Only the exclusive new regular file is passed to QEMU as a drive.
    with os.fdopen(os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK), "rb") as image:
        metadata = os.fstat(image.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size != DISK_BYTES:
            raise ValueError("expected a regular image from the development Data runner")
        image.seek(1024)
        superblock = image.read(120)
        if len(superblock) != 120 or superblock[56:58] != b"\x53\xef":
            raise ValueError("expected an ext4 development Data image")
        filesystem_uuid = str(uuid.UUID(bytes=superblock[104:120]))
        image.seek(0)
        fd = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "wb") as copy:
            shutil.copyfileobj(image, copy)
            if copy.tell() != metadata.st_size:
                raise ValueError("development Data image size changed during copy")
            copy.flush()
            os.fsync(copy.fileno())
        return filesystem_uuid


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kernel", required=True, type=Path)
    parser.add_argument("--initramfs", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path, help="new run directory")
    parser.add_argument("--qemu", default="qemu-system-x86_64")
    parser.add_argument("--mkfs", default="mkfs.ext4")
    parser.add_argument(
        "--provision-identity-from", type=Path, metavar="DEV_DATA_IMAGE",
        help="copy a stopped development Data image and prepare its private identity directories",
    )
    args = parser.parse_args()
    args.kernel = args.kernel.resolve(strict=True)
    args.initramfs = args.initramfs.resolve(strict=True)
    args.output.mkdir()
    args.output = args.output.resolve()

    disk = args.output / "data.raw"
    if args.provision_identity_from is not None:
        filesystem_uuid = copy_development_disk(args.provision_identity_from, disk)
        serial = f"NAGI-DATA-{filesystem_uuid[:8]}"
        run_phase(args, PROVISION_PHASE, serial, filesystem_uuid)
        return

    filesystem_uuid = str(uuid.uuid4())
    serial = f"NAGI-DATA-{filesystem_uuid[:8]}"
    with disk.open("xb") as image:
        image.truncate(DISK_BYTES)
    subprocess.run(
        [args.mkfs, "-q", "-F", "-U", filesystem_uuid, str(disk)], check=True
    )
    for phase in ("write", "read"):
        run_phase(args, phase, serial, filesystem_uuid)


if __name__ == "__main__":
    main()
