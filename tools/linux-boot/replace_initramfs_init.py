#!/usr/bin/env python3
"""Replace only /init in a validated newc initramfs, preserving all other records."""

from __future__ import annotations

import argparse
import hashlib
import os
import stat
import struct
import sys
import tempfile
from pathlib import Path

import boot_acceptance


ROOT = Path(__file__).resolve().parents[2]
MAX_INIT_BYTES = 64 * 1024 * 1024
MAX_ADDED_FILE_BYTES = 64 * 1024


class RepackError(ValueError):
    pass


def _inside_root(value: str, *, missing_leaf: bool) -> Path:
    candidate = Path(value)
    if not candidate.is_absolute():
        candidate = ROOT / candidate
    candidate = Path(os.path.abspath(candidate))
    try:
        relative = candidate.relative_to(ROOT)
    except ValueError:
        raise RepackError("output is outside the assigned worktree") from None
    if not relative.parts:
        raise RepackError("worktree root is not an output file")
    current = ROOT
    for index, part in enumerate(relative.parts):
        current /= part
        try:
            info = os.lstat(current)
        except FileNotFoundError:
            if missing_leaf and index == len(relative.parts) - 1:
                return current
            raise RepackError(f"required output parent is missing: {current.name}") from None
        if stat.S_ISLNK(info.st_mode):
            raise RepackError("symlink output paths are not accepted")
        if index < len(relative.parts) - 1 and not stat.S_ISDIR(info.st_mode):
            raise RepackError("output parent is not a directory")
    return current


def _regular(path: Path, label: str) -> bytes:
    info = os.lstat(path)
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise RepackError(f"{label} must be a regular file")
    data = path.read_bytes()
    if not data:
        raise RepackError(f"{label} is empty")
    return data


def _validate_static_pie(data: bytes) -> None:
    if len(data) > MAX_INIT_BYTES or len(data) < 64:
        raise RepackError("init ELF size is outside the accepted bounds")
    if data[:7] != b"\x7fELF\x02\x01\x01":
        raise RepackError("init must be little-endian ELF64")
    if int.from_bytes(data[16:18], "little") != 3:
        raise RepackError("init must be ET_DYN static PIE")
    if int.from_bytes(data[18:20], "little") != 62 or int.from_bytes(data[20:24], "little") != 1:
        raise RepackError("init must target x86_64")
    entry = int.from_bytes(data[24:32], "little")
    phoff = int.from_bytes(data[32:40], "little")
    ehsize = int.from_bytes(data[52:54], "little")
    phentsize = int.from_bytes(data[54:56], "little")
    phnum = int.from_bytes(data[56:58], "little")
    if ehsize != 64 or phentsize != 56 or phnum == 0 or phoff < ehsize:
        raise RepackError("init ELF program-header table is invalid")
    if phoff + phentsize * phnum > len(data):
        raise RepackError("init ELF program-header table exceeds file size")

    executable_entry = False
    dynamic_ranges: list[tuple[int, int]] = []
    for index in range(phnum):
        offset = phoff + index * phentsize
        header = data[offset:offset + phentsize]
        p_type, p_flags = struct.unpack_from("<II", header, 0)
        p_offset, p_vaddr = struct.unpack_from("<QQ", header, 8)
        p_filesz, p_memsz = struct.unpack_from("<QQ", header, 32)
        if p_offset + p_filesz > len(data):
            raise RepackError(f"init ELF program header {index} exceeds file size")
        if p_type == 3:
            raise RepackError("init must not require a PT_INTERP loader")
        if p_type == 2:
            dynamic_ranges.append((p_offset, p_filesz))
        if p_type == 1:
            if p_filesz > p_memsz:
                raise RepackError("init ELF PT_LOAD has filesz larger than memsz")
            if p_flags & 1 and p_vaddr <= entry < p_vaddr + p_memsz:
                executable_entry = True
    if not executable_entry:
        raise RepackError("init entry is not within an executable PT_LOAD segment")
    for offset, size in dynamic_ranges:
        if size % 16:
            raise RepackError("init ELF dynamic table has an invalid entry size")
        for cursor in range(offset, offset + size, 16):
            tag, _value = struct.unpack_from("<QQ", data, cursor)
            if tag == 1:
                raise RepackError("init ELF has a DT_NEEDED dependency")
            if tag == 0:
                break


def _align4(value: int) -> int:
    return (value + 3) & ~3


def _replace(data: bytes, new_init: bytes) -> tuple[bytes, list, list]:
    old_init, old_entries = boot_acceptance._parse_newc(data)
    if not old_init:
        raise RepackError("input initramfs has no /init")
    output = bytearray()
    cursor = 0
    replaced = 0
    for name, fields in old_entries:
        header = data[cursor:cursor + 110]
        if len(header) != 110 or header[:6] != b"070701":
            raise RepackError("validated CPIO header changed during replacement")
        cursor += 110
        namesize = fields[11]
        raw_name = data[cursor:cursor + namesize]
        cursor = _align4(cursor + namesize)
        size = fields[6]
        payload = data[cursor:cursor + size]
        cursor = _align4(cursor + size)
        updated = list(fields)
        if name == "init":
            replaced += 1
            updated[6] = len(new_init)
            payload = new_init
        output.extend(b"070701")
        output.extend("".join(f"{value:08x}" for value in updated).encode("ascii"))
        output.extend(raw_name)
        output.extend(b"\0" * (_align4(len(output)) - len(output)))
        output.extend(payload)
        output.extend(b"\0" * (_align4(len(output)) - len(output)))
    if replaced != 1 or cursor != len(data):
        raise RepackError("input archive did not contain exactly one /init and one trailer")
    packed = bytes(output)
    packed_init, packed_entries = boot_acceptance._parse_newc(packed)
    if packed_init != new_init or len(packed_entries) != len(old_entries):
        raise RepackError("repacked archive failed its byte-identity check")
    for (old_name, old_fields), (new_name, new_fields) in zip(old_entries, packed_entries):
        if old_name != new_name:
            raise RepackError("repacking changed CPIO record order")
        expected = list(old_fields)
        if old_name == "init":
            expected[6] = len(new_init)
        if list(new_fields) != expected:
            raise RepackError(f"repacking changed metadata for {old_name}")
    return packed, old_entries, packed_entries


def _append_regular_file(data: bytes, name: str, payload: bytes) -> tuple[bytes, list]:
    if not name or name in (".", "..", "TRAILER!!!") or "/" in name:
        raise RepackError("added CPIO path must be a single safe root filename")
    try:
        name_bytes = name.encode("ascii")
    except UnicodeEncodeError:
        raise RepackError("added CPIO path must be ASCII") from None
    if any(byte < 33 or byte > 126 or byte == ord("\\") for byte in name_bytes):
        raise RepackError("added CPIO path contains an invalid character")
    if not payload or len(payload) > MAX_ADDED_FILE_BYTES:
        raise RepackError("added file is empty or exceeds the package bound")

    _init, entries = boot_acceptance._parse_newc(data)
    if any(existing == name for existing, _fields in entries):
        raise RepackError("added CPIO path already exists")
    inode = max(fields[0] for _existing, fields in entries) + 1
    if inode > 0xFFFFFFFF:
        raise RepackError("CPIO inode space is exhausted")
    raw_name = name_bytes + b"\0"
    fields = [inode, stat.S_IFREG | 0o644, 0, 0, 1, 0, len(payload), 0, 0, 0, 0,
              len(raw_name), 0]
    record = bytearray(b"070701")
    record.extend("".join(f"{value:08x}" for value in fields).encode("ascii"))
    record.extend(raw_name)
    record.extend(b"\0" * (_align4(len(record)) - len(record)))
    record.extend(payload)
    record.extend(b"\0" * (_align4(len(record)) - len(record)))

    cursor = 0
    trailer_offset = None
    while cursor < len(data):
        record_start = cursor
        header = data[cursor:cursor + 110]
        if len(header) != 110 or header[:6] != b"070701":
            raise RepackError("validated CPIO header changed during append")
        values = [int(header[index:index + 8], 16) for index in range(6, 110, 8)]
        namesize = values[11]
        cursor = _align4(cursor + 110 + namesize)
        size = values[6]
        cursor = _align4(cursor + size)
        if cursor > len(data):
            raise RepackError("validated CPIO record changed during append")
        name_end = record_start + 110 + namesize
        if data[record_start + 110:name_end] == b"TRAILER!!!\0":
            trailer_offset = record_start
            break
    if trailer_offset is None:
        raise RepackError("CPIO trailer was not found")

    packed = data[:trailer_offset] + bytes(record) + data[trailer_offset:]
    _packed_init, packed_entries = boot_acceptance._parse_newc(packed)
    if (
        len(packed_entries) != len(entries) + 1
        or packed_entries[-2][0] != name
        or packed_entries[-1][0] != "TRAILER!!!"
    ):
        raise RepackError("added CPIO record failed its order/identity check")
    for index, original_record in enumerate(entries[:-1]):
        if packed_entries[index] != original_record:
            raise RepackError("adding the package changed an existing CPIO record")
    if packed_entries[-1] != entries[-1]:
        raise RepackError("adding the package changed CPIO trailer metadata")
    if packed_entries[-2][1] != fields:
        raise RepackError("added CPIO metadata differs from the requested file")
    if _payload_for(packed, name) != payload:
        raise RepackError("added CPIO payload failed its byte-identity check")
    return packed, packed_entries


def _payload_for(data: bytes, wanted_name: str) -> bytes | None:
    cursor = 0
    while cursor < len(data):
        header = data[cursor:cursor + 110]
        if len(header) != 110 or header[:6] != b"070701":
            return None
        fields = [int(header[index:index + 8], 16) for index in range(6, 110, 8)]
        namesize = fields[11]
        name_start = cursor + 110
        name_end = name_start + namesize
        raw_name = data[name_start:name_end]
        data_start = _align4(name_end)
        data_end = data_start + fields[6]
        if len(raw_name) != namesize or data_end > len(data):
            return None
        if raw_name[:-1] == wanted_name.encode("ascii") and raw_name[-1:] == b"\0":
            return data[data_start:data_end]
        cursor = _align4(data_end)
        if raw_name[:-1] == b"TRAILER!!!":
            return None
    return None


def _readonly_regular(path: Path, label: str) -> bytes:
    info = os.lstat(path)
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise RepackError(f"{label} must be a regular file")
    data = path.read_bytes()
    if not data or len(data) > MAX_ADDED_FILE_BYTES:
        raise RepackError(f"{label} is empty or exceeds the package bound")
    return data


def _sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--initramfs", required=True, help="read-only uncompressed newc input")
    parser.add_argument("--init-elf", required=True, help="static x86_64 ET_DYN /init ELF")
    parser.add_argument("--output", required=True, help="new output file inside the assigned worktree")
    parser.add_argument("--add-file", help="optional read-only regular file to add at the CPIO root")
    parser.add_argument("--add-name", help="single safe root filename for --add-file")
    args = parser.parse_args()
    try:
        if (args.add_file is None) != (args.add_name is None):
            raise RepackError("--add-file and --add-name must be supplied together")
        initramfs_path = Path(args.initramfs).resolve(strict=True)
        initramfs = _regular(initramfs_path, "input initramfs")
        init_path = _inside_root(args.init_elf, missing_leaf=False)
        new_init = _regular(init_path, "new init ELF")
        _validate_static_pie(new_init)
        output = _inside_root(args.output, missing_leaf=True)
        if os.path.lexists(output):
            raise RepackError("refusing to overwrite an existing initramfs")
        if not stat.S_ISDIR(os.lstat(output.parent).st_mode):
            raise RepackError("output parent is not a directory")
        packed, old_entries, _new_entries = _replace(initramfs, new_init)
        if args.add_file is not None:
            package_path = Path(args.add_file)
            package = _readonly_regular(package_path, "added package input")
            packed, packed_entries = _append_regular_file(packed, args.add_name, package)
            print(f"added_file={args.add_name} bytes={len(package)} records={len(packed_entries)}")
        temporary_dir = Path(tempfile.mkdtemp(prefix=".initramfs-replace-", dir=output.parent))
        os.chmod(temporary_dir, 0o700)
        temporary_file = temporary_dir / "initramfs.cpio"
        try:
            with temporary_file.open("xb") as stream:
                stream.write(packed)
                stream.flush()
                os.fsync(stream.fileno())
            try:
                os.link(temporary_file, output, follow_symlinks=False)
            except FileExistsError:
                raise RepackError("refusing to overwrite an existing initramfs") from None
        finally:
            temporary_file.unlink(missing_ok=True)
            temporary_dir.rmdir()
        print(f"records={len(old_entries)}")
        print(f"init_bytes={len(new_init)} init_sha256={_sha(new_init)}")
        print(f"initramfs_bytes={len(packed)} initramfs_sha256={_sha(packed)}")
        print(f"output={output.relative_to(ROOT).as_posix()}")
        return 0
    except (OSError, RepackError, boot_acceptance.AcceptanceError) as error:
        message = str(error) if str(error) else type(error).__name__
        print(f"error: {message}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
