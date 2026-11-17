#!/usr/bin/env python3
"""Build a private Linux development-service CPIO from fixed input artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
from pathlib import Path

import boot_acceptance


EXPECTED_PACKAGE_SHA256 = "7f58ad97e255860d83af73095dff8903a57740380a64ca0073619e25654c5e0d"
EXPECTED_SERVICE_SHA256 = "479e01ea9d8a3d8493ff93b14b946321724be2af3df051669ab70006a99497fb"
EXPECTED_CLIENT_SHA256 = "99b4dc7c3379132238355075c83c4c3bc52b83eea4537ee81c33a163c87ad60b"
FIXTURE_PATH = "run/nagi/dev-data/nagi-m19-live-file.txt"
FIXTURE_BYTES = b"nagi development search fixture\n"
NOTES_DESKTOP_PATH = "nagi-notes-desktop"
NOTES_DESKTOP_INPUT_PATH = "run/nagi/notes-desktop.runtime"


def notes_desktop_input(phase: str) -> bytes:
    if phase not in ("save", "read"):
        raise BuildInputError("Notes desktop phase must be save or read")
    return f"mode=development\nphase={phase}\n".encode("ascii")


class BuildInputError(ValueError):
    pass


def _read_fixed(path: Path, expected_sha256: str, label: str) -> bytes:
    try:
        metadata = path.lstat()
    except OSError as error:
        raise BuildInputError(f"{label}: input unavailable ({error.__class__.__name__})") from None
    if not stat.S_ISREG(metadata.st_mode):
        raise BuildInputError(f"{label}: input must be a regular file")
    try:
        payload = path.read_bytes()
    except OSError as error:
        raise BuildInputError(f"{label}: input read failed ({error.__class__.__name__})") from None
    if not payload or hashlib.sha256(payload).hexdigest() != expected_sha256:
        raise BuildInputError(f"{label}: fixed input digest mismatch")
    return payload


def _entry(
    output: bytearray,
    name: str,
    *,
    inode: int,
    mode: int,
    nlink: int,
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
        nlink,
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
        raise BuildInputError("newc entry field overflow")
    output.extend(b"070701" + b"".join(f"{value:08x}".encode("ascii") for value in fields))
    output.extend(encoded_name)
    output.extend(bytes(-len(output) % 4))
    output.extend(payload)
    output.extend(bytes(-len(output) % 4))


def build_archive(
    init: bytes,
    package: bytes,
    service: bytes,
    client: bytes,
    *,
    notes_desktop: bytes | None = None,
    notes_phase: str | None = None,
) -> bytes:
    if not all((init, package, service, client)):
        raise BuildInputError("required CPIO input is empty")
    if (notes_desktop is None) != (notes_phase is None):
        raise BuildInputError("Notes desktop ELF and phase must be supplied together")
    if notes_desktop is not None and not notes_desktop:
        raise BuildInputError("Notes desktop ELF input is empty")
    output = bytearray()
    directories = [
        ("dev", 0o755),
        ("proc", 0o555),
        ("sys", 0o555),
        ("run", 0o755),
        ("run/nagi", 0o700),
        ("run/nagi/dev-data", 0o700),
        ("tmp", 0o1777),
    ]
    inode = 1
    for name, mode in directories:
        _entry(output, name, inode=inode, mode=stat.S_IFDIR | mode, nlink=2)
        inode += 1
    for name, mode, major, minor in (
        ("dev/console", 0o600, 5, 1),
        ("dev/null", 0o666, 1, 3),
        ("dev/ttyS0", 0o600, 4, 64),
    ):
        _entry(
            output,
            name,
            inode=inode,
            mode=stat.S_IFCHR | mode,
            nlink=1,
            rdevmajor=major,
            rdevminor=minor,
        )
        inode += 1

    for name, mode, payload in (
        ("nagi-dev-preview-linux.xapp", 0o644, package),
        ("nagi-dev-action-service", 0o755, service),
        ("nagi-dev-action-client", 0o755, client),
        (FIXTURE_PATH, 0o600, FIXTURE_BYTES),
        ("init", 0o755, init),
    ):
        _entry(output, name, inode=inode, mode=stat.S_IFREG | mode, nlink=1, payload=payload)
        inode += 1
    if notes_desktop is not None and notes_phase is not None:
        _entry(
            output,
            NOTES_DESKTOP_INPUT_PATH,
            inode=inode,
            mode=stat.S_IFREG | 0o600,
            nlink=1,
            payload=notes_desktop_input(notes_phase),
        )
        inode += 1
        _entry(
            output,
            NOTES_DESKTOP_PATH,
            inode=inode,
            mode=stat.S_IFREG | 0o755,
            nlink=1,
            payload=notes_desktop,
        )
        inode += 1
    _entry(output, "TRAILER!!!", inode=inode, mode=0, nlink=1)
    output.extend(bytes(-len(output) % 4))
    return bytes(output)


def validate_archive(
    archive: bytes,
    init: bytes,
    package: bytes,
    service: bytes,
    client: bytes,
    *,
    notes_desktop: bytes | None = None,
    notes_phase: str | None = None,
) -> dict:
    expected_files = {
        "nagi-dev-preview-linux.xapp": (0o644, package),
        "nagi-dev-action-service": (0o755, service),
        "nagi-dev-action-client": (0o755, client),
        FIXTURE_PATH: (0o600, FIXTURE_BYTES),
    }
    if (notes_desktop is None) != (notes_phase is None):
        raise BuildInputError("Notes desktop ELF and phase must be supplied together")
    if notes_desktop is not None and notes_phase is not None:
        expected_files[NOTES_DESKTOP_INPUT_PATH] = (0o600, notes_desktop_input(notes_phase))
        expected_files[NOTES_DESKTOP_PATH] = (0o755, notes_desktop)
    expected_directories = {
        "dev": 0o755,
        "proc": 0o555,
        "sys": 0o555,
        "run": 0o755,
        "run/nagi": 0o700,
        "run/nagi/dev-data": 0o700,
    }
    result = boot_acceptance.validate_development_cpio_inputs(
        archive,
        expected_files,
        expected_directories=expected_directories,
        # The kernel opens /dev/console for PID 1's initial stdio before
        # Nagi mounts devtmpfs over /dev. /dev/null is supplied by that later
        # mount and is not an initramfs-node prerequisite for this path.
        expected_character_devices={"dev/console": (5, 1)},
    )
    _, records = boot_acceptance._parse_newc_records(archive, retain_payloads={"init"})
    init_record = next((record for record in records if record[0] == "init"), None)
    if (
        init_record is None
        or init_record[1][1] != stat.S_IFREG | 0o755
        or init_record[2] != init
    ):
        raise BuildInputError("initramfs init payload mismatch")
    result["init"] = {
        "mode": 0o755,
        "size_bytes": len(init),
        "sha256": hashlib.sha256(init).hexdigest(),
    }
    return result


def build_from_paths(
    init_path: Path,
    package_path: Path,
    service_path: Path,
    client_path: Path,
    *,
    notes_desktop_path: Path | None = None,
    notes_phase: str | None = None,
) -> tuple[bytes, dict]:
    init = init_path.read_bytes()
    package = _read_fixed(package_path, EXPECTED_PACKAGE_SHA256, "signed development package")
    service = _read_fixed(service_path, EXPECTED_SERVICE_SHA256, "development service ELF")
    client = _read_fixed(client_path, EXPECTED_CLIENT_SHA256, "development client ELF")
    if (notes_desktop_path is None) != (notes_phase is None):
        raise BuildInputError("Notes desktop ELF and phase must be supplied together")
    notes_desktop = None
    if notes_desktop_path is not None:
        try:
            metadata = notes_desktop_path.lstat()
        except OSError as error:
            raise BuildInputError(
                f"Notes desktop ELF: input unavailable ({error.__class__.__name__})"
            ) from None
        if not stat.S_ISREG(metadata.st_mode):
            raise BuildInputError("Notes desktop ELF: input must be a regular file")
        try:
            notes_desktop = notes_desktop_path.read_bytes()
        except OSError as error:
            raise BuildInputError(
                f"Notes desktop ELF: input read failed ({error.__class__.__name__})"
            ) from None
    archive = build_archive(
        init,
        package,
        service,
        client,
        notes_desktop=notes_desktop,
        notes_phase=notes_phase,
    )
    return archive, validate_archive(
        archive,
        init,
        package,
        service,
        client,
        notes_desktop=notes_desktop,
        notes_phase=notes_phase,
    )


def write_new_output(path: Path, payload: bytes) -> None:
    if not path.parent.is_dir():
        raise BuildInputError("output parent directory must already exist")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags, 0o600)
    except FileExistsError:
        raise BuildInputError("refusing to overwrite an existing CPIO") from None
    except OSError as error:
        raise BuildInputError(f"CPIO output open failed ({error.__class__.__name__})") from None
    try:
        with os.fdopen(descriptor, "wb", closefd=False) as output:
            output.write(payload)
            output.flush()
            os.fsync(descriptor)
    finally:
        os.close(descriptor)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--init", type=Path, required=True)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--service", type=Path, required=True)
    parser.add_argument("--client", type=Path, required=True)
    parser.add_argument("--notes-desktop", type=Path)
    parser.add_argument("--notes-phase", choices=("save", "read"))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        archive, result = build_from_paths(
            args.init,
            args.package,
            args.service,
            args.client,
            notes_desktop_path=args.notes_desktop,
            notes_phase=args.notes_phase,
        )
        write_new_output(args.output, archive)
        result["cpio_bytes"] = len(archive)
        print(json.dumps(result, sort_keys=True))
        return 0
    except (BuildInputError, OSError) as error:
        parser.error(str(error))
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
