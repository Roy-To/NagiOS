#!/usr/bin/env python3
"""Build a deterministic Nagi-initramfs bundle with explicit source-proof status."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import sys

KERNEL_SOURCE = "https://git.kernel.org/pub/scm/linux/kernel/git/stable/linux.git"
KERNEL_RELEASE_TAG = "v6.18.63"
KERNEL_COMMIT_REFERENCE = "1cdda838e8475edf89b73ff0cc952005dd0b1fa7"
KERNEL_ARCHIVE_URL = "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.18.63.tar.xz"
KERNEL_SIGNATURE_URL = "https://cdn.kernel.org/pub/linux/kernel/v6.x/linux-6.18.63.tar.sign"
KERNEL_SIGNATURE_METHOD = "gpgv verified the signature over the xz-decompressed tar stream"
KERNEL_BUILD_TIMESTAMP = "Tue Nov 10 18:07:06 UTC 2026"
KERNEL_SOURCE_DATE_EPOCH = 1794334026
KERNEL_LICENSE = "GPL-2.0-only (kernel overall; preserve per-file SPDX identifiers, notices, and exceptions)"
INIT_TARGETS = {
    "x86_64": "x86_64-unknown-linux-nagi",
    "aarch64": "aarch64-unknown-linux-nagi",
}
CONFIG_REQUIREMENTS = {
    "x86_64": {
        "CONFIG_X86_64": "y",
        "CONFIG_BLK_DEV_INITRD": "y",
        "CONFIG_BINFMT_ELF": "y",
        "CONFIG_PRINTK": "y",
        "CONFIG_SERIAL_8250": "y",
        "CONFIG_SERIAL_8250_CONSOLE": "y",
    },
    "aarch64": {
        "CONFIG_ARM64": "y",
        "CONFIG_BLK_DEV_INITRD": "y",
        "CONFIG_BINFMT_ELF": "y",
        "CONFIG_PRINTK": "y",
        "CONFIG_SERIAL_AMBA_PL011": "y",
        "CONFIG_SERIAL_AMBA_PL011_CONSOLE": "y",
    },
}
BOOT_CONTRACTS = {
    "x86_64": {
        "kernel_name": "kernel.bzImage",
        "kernel_format": "linux-x86_64-bzImage",
        "boot_method": "qemu-linux-x86-boot-protocol",
        "machine": "q35",
        "machine_arg": "q35,accel=tcg",
        "cpu": "qemu64",
        "qemu": "qemu-system-x86_64",
        "console": "ttyS0",
        "console_major": 4,
        "console_minor": 64,
        "elf_machine": 0x3E,
        "elf_machine_name": "EM_X86_64",
    },
    "aarch64": {
        "kernel_name": "kernel.Image",
        "kernel_format": "linux-aarch64-Image",
        "boot_method": "qemu-arm64-image",
        "machine": "virt",
        "machine_arg": "virt,accel=tcg",
        "cpu": "cortex-a57",
        "qemu": "qemu-system-aarch64",
        "console": "ttyAMA0",
        "console_major": 204,
        "console_minor": 64,
        "elf_machine": 0xB7,
        "elf_machine_name": "EM_AARCH64",
    },
}
CHUNK_SIZE = 1024 * 1024
CPIO_MAGIC = b"070701"


class InputError(ValueError):
    """An input or output path does not satisfy the boot-input contract."""


def _absolute_path(value: str) -> str:
    path = os.path.abspath(os.fspath(value))
    if path == "/Volumes" or path.startswith("/Volumes/"):
        raise InputError("paths under /Volumes are not allowed")
    return path


def _lstat_path(path: str, *, must_exist: bool) -> os.stat_result | None:
    """lstat every path component and reject symlinks before opening anything."""
    absolute = _absolute_path(path)
    parts = [part for part in absolute.split(os.sep) if part]
    current = os.sep
    last_index = len(parts) - 1
    if not parts:
        raise InputError("the filesystem root is not a valid file path")
    for index, part in enumerate(parts):
        current = os.path.join(current, part)
        try:
            info = os.lstat(current)
        except FileNotFoundError:
            if index == last_index and not must_exist:
                return None
            raise InputError(f"path component does not exist: {current}") from None
        if stat.S_ISLNK(info.st_mode):
            raise InputError(f"symlink path component is not allowed: {current}")
        if index < last_index and not stat.S_ISDIR(info.st_mode):
            raise InputError(f"path component is not a directory: {current}")
        if index == last_index:
            if must_exist:
                return info
            raise InputError(f"output path already exists: {current}")
    raise InputError(f"invalid path: {absolute}")


def _open_input(path: str, label: str) -> tuple[int, os.stat_result]:
    absolute = _absolute_path(path)
    info = _lstat_path(absolute, must_exist=True)
    if info is None or not stat.S_ISREG(info.st_mode):
        raise InputError(f"{label} must be a regular file: {absolute}")
    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0)
    try:
        fd = os.open(absolute, flags)
    except OSError as error:
        raise InputError(f"cannot open {label} {absolute}: {error}") from None
    opened = os.fstat(fd)
    if not stat.S_ISREG(opened.st_mode):
        os.close(fd)
        raise InputError(f"{label} is not a regular file: {absolute}")
    if (opened.st_dev, opened.st_ino) != (info.st_dev, info.st_ino):
        os.close(fd)
        raise InputError(f"{label} changed while opening: {absolute}")
    return fd, opened


def _read_at(fd: int, size: int, offset: int, label: str) -> bytes:
    if hasattr(os, "pread"):
        data = os.pread(fd, size, offset)
    else:
        saved = os.lseek(fd, 0, os.SEEK_CUR)
        os.lseek(fd, offset, os.SEEK_SET)
        data = os.read(fd, size)
        os.lseek(fd, saved, os.SEEK_SET)
    if len(data) != size:
        raise InputError(f"{label} is truncated at byte offset {offset}")
    return data


def _write_all(fd: int, data: bytes) -> None:
    view = memoryview(data)
    while view:
        written = os.write(fd, view)
        if written <= 0:
            raise InputError("output write made no progress")
        view = view[written:]


class _DigestWriter:
    """Write a stream while keeping the exact output digest and byte count."""

    def __init__(self, fd: int) -> None:
        self.fd = fd
        self.digest = hashlib.sha256()
        self.count = 0

    def write(self, data: bytes) -> None:
        _write_all(self.fd, data)
        self.digest.update(data)
        self.count += len(data)

    def pad4(self) -> None:
        padding = (-self.count) & 3
        if padding:
            self.write(bytes(padding))


def _hash_fd(fd: int, label: str) -> tuple[str, int]:
    before = os.fstat(fd)
    digest = hashlib.sha256()
    total = 0
    os.lseek(fd, 0, os.SEEK_SET)
    while True:
        block = os.read(fd, CHUNK_SIZE)
        if not block:
            break
        digest.update(block)
        total += len(block)
    after = os.fstat(fd)
    if total != before.st_size or (after.st_dev, after.st_ino, after.st_size) != (
        before.st_dev,
        before.st_ino,
        before.st_size,
    ):
        raise InputError(f"{label} changed while hashing")
    os.lseek(fd, 0, os.SEEK_SET)
    return digest.hexdigest(), total


def _validate_expected_sha256(value: str, label: str) -> str:
    normalized = value.strip().lower()
    if not re.fullmatch(r"[0-9a-f]{64}", normalized):
        raise InputError(f"{label} must be exactly 64 hexadecimal SHA-256 characters")
    return normalized


def _validate_kernel(fd: int, size: int, architecture: str) -> dict[str, int]:
    if architecture == "x86_64":
        # Linux x86 boot protocol header: boot flag, HdrS, protocol version,
        # and XLF_KERNEL_64 are required for QEMU direct bzImage boot.
        if size < 0x238:
            raise InputError("x86_64 kernel is too small for the Linux boot header")
        if _read_at(fd, 2, 0x1FE, "kernel artifact") != bytes((0x55, 0xAA)):
            raise InputError("x86_64 kernel is missing the Linux boot flag 0xAA55")
        if _read_at(fd, 4, 0x202, "kernel artifact") != b"HdrS":
            raise InputError("x86_64 kernel is missing the Linux HdrS signature")
        protocol = int.from_bytes(_read_at(fd, 2, 0x206, "kernel artifact"), "little")
        if protocol < 0x020C:
            raise InputError("x86_64 kernel boot protocol must be version 2.12 or newer")
        xloadflags = int.from_bytes(_read_at(fd, 2, 0x236, "kernel artifact"), "little")
        if not xloadflags & 0x0001:
            raise InputError("x86_64 kernel does not declare XLF_KERNEL_64")
        return {"boot_protocol_version": protocol, "xloadflags": xloadflags}

    if architecture == "aarch64":
        # arm64 Image is a flat boot image, not an ELF vmlinux or x86 bzImage.
        if size < 0x40:
            raise InputError("aarch64 kernel Image is too small for its 64-byte header")
        magic = _read_at(fd, 4, 0x38, "aarch64 kernel Image")
        if magic != (0x644D5241).to_bytes(4, "little"):
            raise InputError("aarch64 kernel must be an uncompressed Image with magic 0x644D5241 at offset 0x38")
        text_offset = int.from_bytes(_read_at(fd, 8, 0x08, "aarch64 kernel Image"), "little")
        image_size = int.from_bytes(_read_at(fd, 8, 0x10, "aarch64 kernel Image"), "little")
        flags = int.from_bytes(_read_at(fd, 8, 0x18, "aarch64 kernel Image"), "little")
        if flags & 0x1:
            raise InputError("aarch64 kernel Image must declare little-endian execution")
        if flags & ~0xF:
            raise InputError("aarch64 kernel Image has reserved header flags set")
        return {"text_offset": text_offset, "image_size": image_size, "header_flags": flags}

    raise InputError(f"unsupported architecture {architecture}; choose x86_64 or aarch64")


def _parse_kernel_config(data: bytes, architecture: str) -> dict[str, str]:
    if architecture not in CONFIG_REQUIREMENTS:
        raise InputError(f"unsupported architecture {architecture}; choose x86_64 or aarch64")
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError as error:
        raise InputError(f"kernel .config is not UTF-8 text: {error}") from None
    values: dict[str, str] = {}
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") and " is not set" not in line:
            continue
        unset_prefix = "# "
        unset_suffix = " is not set"
        if line.startswith(unset_prefix) and line.endswith(unset_suffix):
            name = line[len(unset_prefix):-len(unset_suffix)].strip()
            if not re.fullmatch("CONFIG_[A-Z0-9_]+", name):
                continue
            value = "n"
        else:
            match = re.fullmatch(r"(CONFIG_[A-Z0-9_]+)=(.*)", line)
            if not match:
                continue
            name, value = match.groups()
        if name in values and values[name] != value:
            raise InputError(f"kernel .config has conflicting values for {name}")
        values[name] = value
    for name, expected in CONFIG_REQUIREMENTS[architecture].items():
        actual = values.get(name)
        if actual != expected:
            raise InputError(f"{architecture} kernel .config requires {name}={expected}; found {actual or 'missing'}")
    opposite = "CONFIG_ARM64" if architecture == "x86_64" else "CONFIG_X86_64"
    if values.get(opposite) == "y":
        raise InputError(f"kernel .config architecture conflicts with --arch {architecture}: {opposite}=y")
    if values.get("CONFIG_CMDLINE_OVERRIDE") == "y":
        raise InputError("kernel .config must not override the QEMU command line")
    if architecture == "x86_64":
        initramfs_source = values.get("CONFIG_INITRAMFS_SOURCE")
        if initramfs_source not in (None, '""'):
            raise InputError(
                "x86_64 development boot requires CONFIG_INITRAMFS_SOURCE to be empty; "
                "the prepared CPIO is passed externally")
        if values.get("CONFIG_INITRAMFS_FORCE") == "y":
            raise InputError(
                "x86_64 development boot requires CONFIG_INITRAMFS_FORCE to be disabled; "
                "QEMU supplies the prepared CPIO")
    return values


def _validate_init_elf(fd: int, size: int, architecture: str) -> None:
    _validate_init_elf_reader(
        lambda count, offset: _read_at(fd, count, offset, "Nagi init artifact"),
        size, architecture,
    )


def validate_init_elf_bytes(data: bytes, architecture: str = "x86_64") -> None:
    """Apply the existing ELF contract to immutable bytes without filesystem I/O."""
    if type(data) is not bytes:
        raise InputError("Nagi init bytes must be immutable bytes")
    _validate_init_elf_reader(lambda count, offset: data[offset:offset + count],
                              len(data), architecture)


def _validate_init_elf_reader(read_at, size: int, architecture: str) -> None:
    if architecture not in BOOT_CONTRACTS:
        raise InputError(f"unsupported architecture {architecture}; choose x86_64 or aarch64")
    if size < 64:
        raise InputError("Nagi init artifact is too small for an ELF64 header")
    header = read_at(64, 0)
    if header[:4] != bytes((0x7F, 0x45, 0x4C, 0x46)):
        raise InputError("Nagi init artifact is not ELF")
    if header[4] != 2:
        raise InputError("Nagi init artifact must be ELF64")
    if header[5] != 1:
        raise InputError("Nagi init artifact must be little-endian")
    if header[6] != 1:
        raise InputError("Nagi init artifact has an unsupported ELF version")
    elf_type = int.from_bytes(header[16:18], "little")
    machine = int.from_bytes(header[18:20], "little")
    version = int.from_bytes(header[20:24], "little")
    entry = int.from_bytes(header[24:32], "little")
    phoff = int.from_bytes(header[32:40], "little")
    ehsize = int.from_bytes(header[52:54], "little")
    phentsize = int.from_bytes(header[54:56], "little")
    phnum = int.from_bytes(header[56:58], "little")
    if elf_type != 2:
        raise InputError("Nagi init must be a static ELF ET_EXEC executable")
    expected_machine = BOOT_CONTRACTS[architecture]["elf_machine"]
    if machine != expected_machine:
        machine_name = BOOT_CONTRACTS[architecture]["elf_machine_name"]
        raise InputError(f"Nagi init ELF machine must be {machine_name} for --arch {architecture}")
    if version != 1 or ehsize != 64:
        raise InputError("Nagi init ELF header fields are invalid")
    if phentsize != 56 or phnum == 0 or phoff < ehsize:
        raise InputError("Nagi init ELF program-header table is missing or invalid")
    if phoff + phentsize * phnum > size:
        raise InputError("Nagi init ELF program-header table extends past end of file")

    executable_entry = False
    for index in range(phnum):
        raw = read_at(phentsize, phoff + index * phentsize)
        p_type = int.from_bytes(raw[0:4], "little")
        p_flags = int.from_bytes(raw[4:8], "little")
        p_offset = int.from_bytes(raw[8:16], "little")
        p_vaddr = int.from_bytes(raw[16:24], "little")
        p_filesz = int.from_bytes(raw[32:40], "little")
        p_memsz = int.from_bytes(raw[40:48], "little")
        if p_offset + p_filesz > size:
            raise InputError(f"Nagi init ELF program header {index} extends past end of file")
        if p_type == 3:
            raise InputError("Nagi init must not require a PT_INTERP dynamic loader")
        if p_type == 2:
            raise InputError("Nagi init must be statically linked without a PT_DYNAMIC segment")
        if p_type == 1:
            if p_filesz > p_memsz:
                raise InputError(f"Nagi init ELF PT_LOAD segment {index} has filesz larger than memsz")
            if p_flags & 0x1 and p_vaddr <= entry < p_vaddr + p_memsz:
                executable_entry = True
    if not executable_entry:
        raise InputError("Nagi init ELF entry point is not in an executable PT_LOAD segment")


def _cpio_entry(
    writer: _DigestWriter,
    name: str,
    *,
    inode: int,
    mode: int,
    nlink: int,
    size: int = 0,
    rdevmajor: int = 0,
    rdevminor: int = 0,
) -> None:
    encoded_name = name.encode("ascii") + b"\0"
    fields = (
        inode,
        mode,
        0,  # uid
        0,  # gid
        nlink,
        0,  # mtime
        size,
        0,  # devmajor
        0,  # devminor
        rdevmajor,
        rdevminor,
        len(encoded_name),
        0,  # c_check for newc
    )
    if any(value < 0 or value > 0xFFFFFFFF for value in fields):
        raise InputError("initramfs entry exceeds the 32-bit newc CPIO field limit")
    header = CPIO_MAGIC + b"".join(f"{value:08x}".encode("ascii") for value in fields)
    writer.write(header)
    writer.write(encoded_name)
    writer.pad4()


def _write_initramfs(
    fd: int,
    init_fd: int,
    init_size: int,
    expected_sha256: str,
    architecture: str,
) -> tuple[str, int]:
    writer = _DigestWriter(fd)
    contract = BOOT_CONTRACTS[architecture]
    entries = [
        ("dev", stat.S_IFDIR | 0o755, 2, 0, 0),
        ("dev/console", stat.S_IFCHR | 0o600, 1, 5, 1),
        ("dev/null", stat.S_IFCHR | 0o666, 1, 1, 3),
        (
            f"dev/{contract['console']}",
            stat.S_IFCHR | 0o600,
            1,
            contract["console_major"],
            contract["console_minor"],
        ),
        ("proc", stat.S_IFDIR | 0o555, 2, 0, 0),
        ("sys", stat.S_IFDIR | 0o555, 2, 0, 0),
        ("run", stat.S_IFDIR | 0o755, 2, 0, 0),
        ("tmp", stat.S_IFDIR | 0o1777, 2, 0, 0),
    ]
    inode = 1
    for name, mode, nlink, major, minor in entries:
        _cpio_entry(
            writer,
            name,
            inode=inode,
            mode=mode,
            nlink=nlink,
            rdevmajor=major,
            rdevminor=minor,
        )
        inode += 1

    _cpio_entry(writer, "init", inode=inode, mode=stat.S_IFREG | 0o755, nlink=1, size=init_size)
    init_digest = hashlib.sha256()
    copied = 0
    os.lseek(init_fd, 0, os.SEEK_SET)
    while True:
        block = os.read(init_fd, CHUNK_SIZE)
        if not block:
            break
        _write_all(fd, block)
        writer.digest.update(block)
        writer.count += len(block)
        init_digest.update(block)
        copied += len(block)
    if copied != init_size:
        raise InputError("Nagi init artifact changed while writing initramfs")
    writer.pad4()
    if init_digest.hexdigest() != expected_sha256:
        raise InputError("Nagi init artifact changed after SHA-256 verification")
    _cpio_entry(writer, "TRAILER!!!", inode=inode + 1, mode=0, nlink=1)
    writer.pad4()
    return writer.digest.hexdigest(), writer.count


def _open_new_file(directory_fd: int, name: str) -> int:
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    return os.open(name, flags, 0o644, dir_fd=directory_fd)


def _copy_fd_to_new_file(directory_fd: int, name: str, source_fd: int, expected_sha256: str) -> tuple[str, int]:
    output_fd = _open_new_file(directory_fd, name)
    digest = hashlib.sha256()
    total = 0
    try:
        os.lseek(source_fd, 0, os.SEEK_SET)
        while True:
            block = os.read(source_fd, CHUNK_SIZE)
            if not block:
                break
            _write_all(output_fd, block)
            digest.update(block)
            total += len(block)
        os.fsync(output_fd)
    finally:
        os.close(output_fd)
    actual = digest.hexdigest()
    if actual != expected_sha256:
        raise InputError(f"{name} input changed after SHA-256 verification")
    return actual, total


def _write_bytes_to_new_file(directory_fd: int, name: str, content: bytes) -> str:
    output_fd = _open_new_file(directory_fd, name)
    try:
        _write_all(output_fd, content)
        os.fsync(output_fd)
    finally:
        os.close(output_fd)
    return hashlib.sha256(content).hexdigest()


def _validate_revision(value: str) -> str:
    if value != KERNEL_COMMIT_REFERENCE:
        raise InputError(
            f"kernel revision reference must be {KERNEL_COMMIT_REFERENCE}; this string does not prove source provenance"
        )
    return value


def _read_fd_all(fd: int, label: str) -> bytes:
    before = os.fstat(fd)
    if before.st_size > 1024 * 1024:
        raise InputError(f"{label} is unexpectedly large")
    os.lseek(fd, 0, os.SEEK_SET)
    chunks = []
    total = 0
    while True:
        block = os.read(fd, CHUNK_SIZE)
        if not block:
            break
        chunks.append(block)
        total += len(block)
    after = os.fstat(fd)
    if total != before.st_size or (after.st_dev, after.st_ino, after.st_size) != (
        before.st_dev, before.st_ino, before.st_size
    ):
        raise InputError(f"{label} changed while reading")
    return b"".join(chunks)


def _validate_kernel_build_record(
    data: bytes,
    *,
    revision_reference: str,
    kernel_sha256: str,
    config_sha256: str,
) -> dict[str, object]:
    try:
        record = json.loads(data.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise InputError(f"kernel build record is not valid UTF-8 JSON: {error}") from None
    if not isinstance(record, dict):
        raise InputError("kernel build record must be a JSON object")
    if record.get("format") != "nagi-linux-kernel-build" or record.get("format_version") != 2:
        raise InputError("kernel build record format is unsupported")
    if record.get("target") != "x86_64-linux-qemu-q35":
        raise InputError("kernel build record target must be x86_64-linux-qemu-q35")

    source = record.get("source")
    if not isinstance(source, dict):
        raise InputError("kernel build record has no source identity")
    if source.get("release_tag") != KERNEL_RELEASE_TAG or source.get("makefile_release") != KERNEL_RELEASE_TAG.removeprefix("v"):
        raise InputError("kernel build record does not identify the pinned Linux release archive")
    if source.get("archive_url") != KERNEL_ARCHIVE_URL or source.get("signature_url") != KERNEL_SIGNATURE_URL:
        raise InputError("kernel build record source URLs do not match the pinned Linux release")
    if source.get("signature_verified") is not True:
        raise InputError("kernel build record does not report a verified detached source signature")
    if source.get("signature_verification") != KERNEL_SIGNATURE_METHOD:
        raise InputError("kernel build record signature method does not match the expected verification")
    for name in ("archive_size_bytes", "signature_size_bytes", "keyring_size_bytes"):
        value = source.get(name)
        if not isinstance(value, int) or isinstance(value, bool) or value < 1:
            raise InputError(f"kernel build record source {name} is invalid")
    if source.get("commit_reference") != revision_reference:
        raise InputError("kernel build record commit reference does not match --kernel-revision")
    if source.get("commit_reference_verified") is not False:
        raise InputError("kernel build record must state that the Git commit mapping is unverified")
    if not isinstance(source.get("commit_proof_limit"), str) or not source["commit_proof_limit"]:
        raise InputError("kernel build record must state the Git commit proof limit")

    for name in ("archive_sha256", "signature_sha256", "keyring_sha256", "source_tree_sha256"):
        _validate_expected_sha256(str(source.get(name, "")), f"build record source {name}")
    if not isinstance(source.get("keyring_trust"), str) or not source["keyring_trust"]:
        raise InputError("kernel build record must describe who supplied the keyring trust")

    artifacts = record.get("artifacts")
    if not isinstance(artifacts, dict):
        raise InputError("kernel build record has no output artifact hashes")
    kernel = artifacts.get("kernel")
    config = artifacts.get("config")
    if not isinstance(kernel, dict) or not isinstance(config, dict):
        raise InputError("kernel build record is missing kernel or config output hashes")
    if kernel.get("sha256") != kernel_sha256:
        raise InputError("kernel artifact hash differs from the build record")
    if config.get("sha256") != config_sha256:
        raise InputError("kernel config hash differs from the build record")
    config_fragment = record.get("config_fragment")
    if not isinstance(config_fragment, dict):
        raise InputError("kernel build record has no config fragment identity")
    _validate_expected_sha256(str(config_fragment.get("sha256", "")), "build record config fragment SHA-256")
    return record


def _new_output_directory(path: str) -> tuple[int, str, str]:
    absolute = _absolute_path(path)
    parent, leaf = os.path.split(absolute)
    if not leaf or leaf in (".", ".."):
        raise InputError("output directory must name a new child directory")
    parent_info = _lstat_path(parent, must_exist=True)
    if parent_info is None or not stat.S_ISDIR(parent_info.st_mode):
        raise InputError(f"output parent is not a directory: {parent}")
    _lstat_path(absolute, must_exist=False)
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0) | getattr(os, "O_NOFOLLOW", 0)
    parent_fd = os.open(parent, flags)
    try:
        os.mkdir(leaf, 0o700, dir_fd=parent_fd)
        directory_fd = os.open(leaf, flags, dir_fd=parent_fd)
    except OSError as error:
        os.close(parent_fd)
        raise InputError(f"cannot create new output directory {absolute}: {error}") from None
    return parent_fd, leaf, directory_fd


def _discard_new_output(parent_fd: int, leaf: str, directory_fd: int) -> None:
    for name in ("kernel.bzImage", "kernel.config", "nagi-init.elf", "initramfs.cpio", "kernel-build-record.json", "manifest.json"):
        try:
            os.unlink(name, dir_fd=directory_fd)
        except FileNotFoundError:
            pass
    os.close(directory_fd)
    try:
        os.rmdir(leaf, dir_fd=parent_fd)
    finally:
        os.close(parent_fd)


def prepare_bundle(
    *,
    architecture: str,
    kernel_path: str,
    kernel_sha256: str,
    kernel_config_path: str,
    kernel_config_sha256: str,
    kernel_revision: str,
    init_path: str,
    init_sha256: str,
    output_dir: str,
    kernel_build_record_path: str | None = None,
) -> dict[str, object]:
    if architecture not in BOOT_CONTRACTS:
        raise InputError(f"unsupported architecture {architecture}; choose x86_64 or aarch64")
    contract = BOOT_CONTRACTS[architecture]
    kernel_expected = _validate_expected_sha256(kernel_sha256, "kernel --kernel-sha256")
    config_expected = _validate_expected_sha256(kernel_config_sha256, "kernel --kernel-config-sha256")
    init_expected = _validate_expected_sha256(init_sha256, "init --init-sha256")
    revision = _validate_revision(kernel_revision)

    kernel_fd = config_fd = init_fd = build_record_fd = None
    parent_fd = directory_fd = None
    leaf = None
    build_record_data = None
    build_record_sha256 = None
    build_record = None
    try:
        kernel_fd, kernel_info = _open_input(kernel_path, "Linux kernel artifact")
        config_fd, config_info = _open_input(kernel_config_path, "kernel .config")
        init_fd, init_info = _open_input(init_path, "Nagi init ELF")
        if kernel_build_record_path is not None:
            build_record_fd, _ = _open_input(kernel_build_record_path, "kernel build record")
        kernel_header = _validate_kernel(kernel_fd, kernel_info.st_size, architecture)
        _validate_init_elf(init_fd, init_info.st_size, architecture)
        kernel_actual, kernel_size = _hash_fd(kernel_fd, "Linux kernel artifact")
        if kernel_actual != kernel_expected:
            raise InputError(f"Linux kernel SHA-256 mismatch: expected {kernel_expected}, got {kernel_actual}")
        init_actual, init_size = _hash_fd(init_fd, "Nagi init ELF")
        if init_actual != init_expected:
            raise InputError(f"Nagi init SHA-256 mismatch: expected {init_expected}, got {init_actual}")
        config_data = bytearray()
        os.lseek(config_fd, 0, os.SEEK_SET)
        while True:
            block = os.read(config_fd, CHUNK_SIZE)
            if not block:
                break
            config_data.extend(block)
        if len(config_data) != config_info.st_size:
            raise InputError("kernel .config changed while reading")
        _parse_kernel_config(bytes(config_data), architecture)
        config_sha256 = hashlib.sha256(config_data).hexdigest()
        if config_sha256 != config_expected:
            raise InputError(f"kernel .config SHA-256 mismatch: expected {config_expected}, got {config_sha256}")
        if build_record_fd is not None:
            build_record_data = _read_fd_all(build_record_fd, "kernel build record")
            build_record_sha256 = hashlib.sha256(build_record_data).hexdigest()
            build_record = _validate_kernel_build_record(
                build_record_data,
                revision_reference=revision,
                kernel_sha256=kernel_actual,
                config_sha256=config_sha256,
            )

        parent_fd, leaf, directory_fd = _new_output_directory(output_dir)
        kernel_name = contract["kernel_name"]
        copied_kernel_sha256, copied_kernel_size = _copy_fd_to_new_file(
            directory_fd, kernel_name, kernel_fd, kernel_expected
        )
        copied_config_sha256 = _write_bytes_to_new_file(directory_fd, "kernel.config", bytes(config_data))
        if copied_config_sha256 != config_sha256:
            raise InputError("kernel .config changed while preparing bundle")
        copied_init_sha256, copied_init_size = _copy_fd_to_new_file(
            directory_fd, "nagi-init.elf", init_fd, init_expected
        )
        if build_record_data is not None and build_record_sha256 is not None:
            copied_record_sha256 = _write_bytes_to_new_file(
                directory_fd, "kernel-build-record.json", build_record_data
            )
            if copied_record_sha256 != build_record_sha256:
                raise InputError("kernel build record changed while preparing bundle")

        cpio_fd = _open_new_file(directory_fd, "initramfs.cpio")
        try:
            initramfs_sha256, initramfs_size = _write_initramfs(
                cpio_fd, init_fd, init_size, init_expected, architecture
            )
            os.fsync(cpio_fd)
        finally:
            os.close(cpio_fd)

        if copied_kernel_size != kernel_size or copied_init_size != init_size:
            raise InputError("an input artifact changed while preparing the bundle")
        command_line = [f"console={contract['console']}", "rdinit=/init"]
        if build_record is None:
            provenance: dict[str, object] = {
                "status": "unverified",
                "source_signature_status": "not_verified",
                "commit_reference": revision,
                "commit_reference_verified": False,
                "proof_limit": "prepare received no build record; caller-supplied revision and artifact hashes do not prove source provenance",
            }
        else:
            source_record = build_record["source"]
            provenance = {
                "status": "signed_release_reported_by_build_record",
                "source_signature_status": "reported_verified_in_build_record",
                "source_archive_sha256": source_record["archive_sha256"],
                "source_signature_sha256": source_record["signature_sha256"],
                "signature_verification": source_record["signature_verification"],
                "keyring_sha256": source_record["keyring_sha256"],
                "keyring_trust": source_record["keyring_trust"],
                "source_tree_sha256": source_record["source_tree_sha256"],
                "build_record": "kernel-build-record.json",
                "build_record_sha256": build_record_sha256,
                "build_record_authenticated": False,
                "commit_reference": revision,
                "commit_reference_verified": False,
                "proof_limit": source_record["commit_proof_limit"],
            }
        manifest: dict[str, object] = {
            "format": "nagi-linux-boot-inputs",
            "format_version": 2,
            "target": {
                "architecture": architecture,
                "machine": contract["machine"],
                "cpu": contract["cpu"],
                "memory_mib": 512,
                "boot_method": contract["boot_method"],
            },
            "kernel": {
                "artifact": kernel_name,
                "format": contract["kernel_format"],
                "header": kernel_header,
                "sha256": copied_kernel_sha256,
                "size_bytes": copied_kernel_size,
                "source": KERNEL_SOURCE,
                "release_tag_reference": KERNEL_RELEASE_TAG,
                "commit_reference": revision,
                "commit_reference_verified": False,
                "archive_url": KERNEL_ARCHIVE_URL,
                "signature_url": KERNEL_SIGNATURE_URL,
                "provenance": provenance,
                "license": KERNEL_LICENSE,
                "config": "kernel.config",
                "config_sha256": copied_config_sha256,
                "config_size_bytes": len(config_data),
            },
            "init": {
                "artifact": "nagi-init.elf",
                "format": f"ELF64-LE-ET_EXEC-{contract['elf_machine_name']}-static",
                "sha256": copied_init_sha256,
                "size_bytes": copied_init_size,
                "target": INIT_TARGETS[architecture],
                "initramfs_path": "/init",
                "pid": 1,
            },
            "initramfs": {
                "artifact": "initramfs.cpio",
                "format": "cpio-newc-uncompressed",
                "sha256": initramfs_sha256,
                "size_bytes": initramfs_size,
                "console_nodes": {
                    "/dev/console": "char 5:1",
                    "/dev/null": "char 1:3",
                    f"/dev/{contract['console']}": f"char {contract['console_major']}:{contract['console_minor']}",
                },
            },
            "boot": {
                "kernel_command_line": command_line,
                "qemu_argv": [
                    contract["qemu"],
                    "-machine", contract["machine_arg"],
                    "-cpu", contract["cpu"],
                    "-m", "512M",
                    "-smp", "1",
                    "-kernel", kernel_name,
                    "-initrd", "initramfs.cpio",
                    "-append", " ".join(command_line),
                    "-nographic",
                    "-no-reboot",
                ],
            },
        }
        manifest_bytes = (json.dumps(manifest, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
        _write_bytes_to_new_file(directory_fd, "manifest.json", manifest_bytes)
        os.fsync(directory_fd)
        os.close(directory_fd)
        os.close(parent_fd)
        return manifest
    except Exception:
        if directory_fd is not None and parent_fd is not None and leaf is not None:
            _discard_new_output(parent_fd, leaf, directory_fd)
            directory_fd = parent_fd = None
        raise
    finally:
        for descriptor in (kernel_fd, config_fd, init_fd, build_record_fd):
            if descriptor is not None:
                os.close(descriptor)


def _read_plan_document(path: str, expected_sha256: str, label: str) -> dict:
    sys.dont_write_bytecode = True
    from sysroot_plan import MAX_DOCUMENT_BYTES, decode_document, local_absolute_path

    absolute = local_absolute_path(path, label + " path")
    fd, before = _open_input(absolute, label)
    try:
        if before.st_size > MAX_DOCUMENT_BYTES:
            raise InputError(f"{label} exceeds the document size limit")
        raw = bytearray()
        while len(raw) <= MAX_DOCUMENT_BYTES:
            block = os.read(fd, min(CHUNK_SIZE, MAX_DOCUMENT_BYTES + 1 - len(raw)))
            if not block:
                break
            raw.extend(block)
        after = os.fstat(fd)
        if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
            after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns
        ) or len(raw) != before.st_size:
            raise InputError(f"{label} changed while being read")
        return decode_document(bytes(raw), expected_sha256, label)
    finally:
        os.close(fd)


def plan_sysroot(inputs_path: str, inputs_sha256: str, handoff_path: str,
                 handoff_sha256: str) -> dict:
    """Read two pinned documents and inspect output paths; never create or execute."""
    sys.dont_write_bytecode = True
    from sysroot_plan import R23_SHA256, PlanError, local_absolute_path, validate_plan

    # Reject nonlocal paths and a wrong fixed reference before any filesystem access.
    local_absolute_path(inputs_path, "inputs path")
    local_absolute_path(handoff_path, "handoff path")
    if handoff_sha256 != R23_SHA256:
        raise PlanError("the fixed r23 handoff SHA-256 is required")
    plan = _read_plan_document(inputs_path, inputs_sha256, "sysroot inputs")
    handoff = _read_plan_document(handoff_path, handoff_sha256, "r23 handoff")
    output = local_absolute_path(plan.get("proposed_new_output_root"), "proposed_new_output_root")
    # Even an incomplete plan must not hide reuse of an existing/symlink output.
    owner = os.path.dirname(os.path.abspath(__file__))
    if os.path.dirname(output) != owner:
        raise PlanError("output must be a new direct child of the owning linux-boot directory")
    _lstat_path(output, must_exist=False)
    return validate_plan(plan, handoff, handoff_sha256=handoff_sha256,
                         allowed_output_parent=owner)


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Prepare a deterministic direct-boot bundle from an architecture-matched Linux kernel and Nagi init ELF."
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    prepare = subparsers.add_parser("prepare", help="validate and bundle Linux boot inputs with provenance status")
    prepare.add_argument("--arch", required=True, choices=tuple(BOOT_CONTRACTS), help="explicit kernel and init architecture")
    prepare.add_argument("--kernel", required=True, help="x86_64 bzImage or aarch64 uncompressed Image")
    prepare.add_argument("--kernel-sha256", required=True, help="expected SHA-256 of --kernel")
    prepare.add_argument("--kernel-config", required=True, help="exact .config used for the kernel build")
    prepare.add_argument("--kernel-config-sha256", required=True, help="expected SHA-256 of --kernel-config")
    prepare.add_argument("--kernel-revision", required=True, help=f"fixed Git commit reference {KERNEL_COMMIT_REFERENCE}; this value alone does not prove source provenance")
    prepare.add_argument("--kernel-build-record", help="optional build-record.json; validates matching artifact hashes and records source proof limits")
    prepare.add_argument("--init-elf", required=True, help="Nagi Linux init ET_EXEC ELF artifact")
    prepare.add_argument("--init-sha256", required=True, help="expected SHA-256 of --init-elf")
    prepare.add_argument("--output-dir", required=True, help="new, absent directory for the prepared bundle")
    plan = subparsers.add_parser("plan-sysroot", help="read-only sysroot proposal validation; never fetches, writes or executes")
    plan.add_argument("--inputs", required=True, help="absolute path to the fixed proposal JSON")
    plan.add_argument("--inputs-sha256", required=True, help="expected SHA-256 of the proposal JSON")
    plan.add_argument("--handoff", required=True, help="absolute path to the fixed r23 handoff-v2.json")
    plan.add_argument("--handoff-sha256", required=True, help="fixed r23 handoff SHA-256")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    if args.command == "plan-sysroot":
        sys.dont_write_bytecode = True
        from sysroot_plan import PlanError
        try:
            result = plan_sysroot(args.inputs, args.inputs_sha256, args.handoff, args.handoff_sha256)
        except (InputError, PlanError, OSError) as error:
            print(f"FAIL sysroot-plan (NOT EXECUTED): {error}", file=sys.stderr)
            return 2
        print(json.dumps(result, sort_keys=True))
        return 0
    try:
        manifest = prepare_bundle(
            architecture=args.arch,
            kernel_path=args.kernel,
            kernel_sha256=args.kernel_sha256,
            kernel_config_path=args.kernel_config,
            kernel_config_sha256=args.kernel_config_sha256,
            kernel_revision=args.kernel_revision,
            init_path=args.init_elf,
            init_sha256=args.init_sha256,
            output_dir=args.output_dir,
            kernel_build_record_path=args.kernel_build_record,
        )
    except (InputError, OSError) as error:
        print(f"FAIL linux-boot: {error}", file=sys.stderr)
        return 2
    provenance = manifest["kernel"]["provenance"]
    if provenance["source_signature_status"] == "reported_verified_in_build_record":
        print("CREATED linux-boot bundle; source signature status is recorded in the unsigned build record")
        print("Git commit mapping: UNVERIFIED")
    else:
        print("CREATED linux-boot bundle; kernel source provenance NOT VERIFIED")
    print(f"kernel sha256: {manifest['kernel']['sha256']}")
    print(f"init sha256: {manifest['init']['sha256']}")
    print(f"initramfs sha256: {manifest['initramfs']['sha256']}")
    print(f"manifest: {os.path.join(os.path.abspath(args.output_dir), 'manifest.json')}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
