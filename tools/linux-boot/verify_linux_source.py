#!/usr/bin/env python3
"""Verify an extracted Linux 6.18.63 tree against the pinned source archive."""

from __future__ import annotations

import argparse
import hashlib
import json
import lzma
import os
import stat
import sys
import tarfile
from pathlib import Path

PINNED_ARCHIVE_SHA256 = "81764a545072d160748beff4d9de2e7d95aa9f16e8261110288f067a2cc61d71"
ARCHIVE_ROOT = "linux-6.18.63"
CHUNK_SIZE = 1024 * 1024


class SourceVerificationError(ValueError):
    pass


def _regular_file(path: Path, label: str) -> os.stat_result:
    try:
        info = path.lstat()
    except FileNotFoundError:
        raise SourceVerificationError(f"missing {label}: {path}") from None
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise SourceVerificationError(f"{label} must be a non-symlink regular file: {path}")
    return info


def _open_regular_file(path: Path, label: str) -> tuple[int, os.stat_result]:
    before = _regular_file(path, label)
    try:
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    except OSError as error:
        raise SourceVerificationError(f"cannot open {label}: {path}: {error}") from None
    opened = os.fstat(fd)
    if (
        not stat.S_ISREG(opened.st_mode)
        or (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino)
    ):
        os.close(fd)
        raise SourceVerificationError(f"file changed while opening: {path}")
    return fd, opened


class _HashingReader:
    def __init__(self, stream):
        self.stream = stream
        self.digest = hashlib.sha256()

    def read(self, size: int = -1) -> bytes:
        data = self.stream.read(size)
        self.digest.update(data)
        return data


def sha256_file(path: Path) -> str:
    before = _regular_file(path, "source archive or build input")
    digest = hashlib.sha256()
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    try:
        opened = os.fstat(fd)
        if (opened.st_dev, opened.st_ino) != (before.st_dev, before.st_ino):
            raise SourceVerificationError(f"file changed while opening: {path}")
        with os.fdopen(fd, "rb", closefd=False) as stream:
            for chunk in iter(lambda: stream.read(CHUNK_SIZE), b""):
                digest.update(chunk)
        after = os.fstat(fd)
        if (opened.st_dev, opened.st_ino, opened.st_size, opened.st_mtime_ns) != (
            after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns
        ):
            raise SourceVerificationError(f"file changed while hashing: {path}")
    finally:
        os.close(fd)
    return digest.hexdigest()


def _file_sha256(path: Path, expected: os.stat_result) -> str:
    fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
    digest = hashlib.sha256()
    try:
        opened = os.fstat(fd)
        if (opened.st_dev, opened.st_ino, opened.st_size) != (
            expected.st_dev, expected.st_ino, expected.st_size
        ):
            raise SourceVerificationError(f"source file changed while opening: {path}")
        with os.fdopen(fd, "rb", closefd=False) as stream:
            for chunk in iter(lambda: stream.read(CHUNK_SIZE), b""):
                digest.update(chunk)
        after = os.fstat(fd)
        if (opened.st_dev, opened.st_ino, opened.st_size, opened.st_mtime_ns) != (
            after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns
        ):
            raise SourceVerificationError(f"source file changed while hashing: {path}")
    finally:
        os.close(fd)
    return digest.hexdigest()


def _archive_relative_path(name: str) -> str | None:
    while name.startswith("./"):
        name = name[2:]
    name = name.rstrip("/")
    if not name or name.startswith("/"):
        raise SourceVerificationError(f"unsafe source archive path: {name!r}")
    parts = name.split("/")
    if any(part in ("", ".", "..") for part in parts):
        raise SourceVerificationError(f"unsafe source archive path: {name!r}")
    if parts[0] != ARCHIVE_ROOT:
        raise SourceVerificationError(f"unexpected source archive root: {name!r}")
    return "/".join(parts[1:]) or None


def _archive_tree(reader: _HashingReader) -> dict[str, tuple]:
    records: dict[str, tuple] = {}
    hardlinks: dict[str, tuple[int, str]] = {}
    try:
        with tarfile.open(fileobj=reader, mode="r|xz") as archive:
            for member in archive:
                relative = _archive_relative_path(member.name)
                if relative is None:
                    continue
                if relative in records or relative in hardlinks:
                    raise SourceVerificationError(f"duplicate source archive member: {relative}")
                executable_bits = member.mode & 0o111
                if member.isdir():
                    records[relative] = ("directory", executable_bits)
                elif member.issym():
                    records[relative] = ("symlink", member.linkname)
                elif member.islnk():
                    target = _archive_relative_path(member.linkname)
                    if target is None:
                        raise SourceVerificationError(f"invalid source hardlink target: {member.linkname!r}")
                    hardlinks[relative] = (executable_bits, target)
                elif member.isreg():
                    stream = archive.extractfile(member)
                    if stream is None:
                        raise SourceVerificationError(f"cannot read source archive member: {relative}")
                    digest = hashlib.sha256()
                    size = 0
                    with stream:
                        for chunk in iter(lambda: stream.read(CHUNK_SIZE), b""):
                            digest.update(chunk)
                            size += len(chunk)
                    if size != member.size:
                        raise SourceVerificationError(f"short source archive member: {relative}")
                    records[relative] = ("file", executable_bits, size, digest.hexdigest())
                else:
                    raise SourceVerificationError(f"unsupported source archive member: {relative}")
    except (OSError, tarfile.TarError, EOFError, lzma.LZMAError) as error:
        raise SourceVerificationError(f"cannot read pinned Linux source archive: {error}") from None

    for relative, (executable_bits, target) in hardlinks.items():
        target_record = records.get(target)
        if target_record is None or target_record[0] != "file":
            raise SourceVerificationError(f"source hardlink target is missing or not regular: {relative} -> {target}")
        _, _, size, digest = target_record
        records[relative] = ("file", executable_bits, size, digest)
    if not records:
        raise SourceVerificationError("pinned Linux source archive has no source members")
    return records


def _directory_tree(source_dir: Path) -> dict[str, tuple]:
    try:
        root_info = source_dir.lstat()
    except FileNotFoundError:
        raise SourceVerificationError(f"missing extracted Linux source tree: {source_dir}") from None
    if stat.S_ISLNK(root_info.st_mode) or not stat.S_ISDIR(root_info.st_mode):
        raise SourceVerificationError("extracted Linux source tree must be a non-symlink directory")
    records: dict[str, tuple] = {}

    def visit(directory: Path, prefix: str) -> None:
        try:
            entries = sorted(os.scandir(directory), key=lambda entry: entry.name)
        except OSError as error:
            raise SourceVerificationError(f"cannot scan extracted source directory {directory}: {error}") from None
        for entry in entries:
            relative = f"{prefix}/{entry.name}" if prefix else entry.name
            path = Path(entry.path)
            info = entry.stat(follow_symlinks=False)
            if stat.S_ISDIR(info.st_mode):
                records[relative] = ("directory", info.st_mode & 0o111)
                visit(path, relative)
            elif stat.S_ISLNK(info.st_mode):
                records[relative] = ("symlink", os.readlink(path))
            elif stat.S_ISREG(info.st_mode):
                records[relative] = (
                    "file", info.st_mode & 0o111, info.st_size, _file_sha256(path, info)
                )
            else:
                raise SourceVerificationError(f"unsupported file in extracted Linux source tree: {relative}")

    visit(source_dir, "")
    if not records:
        raise SourceVerificationError("extracted Linux source tree is empty")
    return records


def _tree_digest(records: dict[str, tuple]) -> str:
    digest = hashlib.sha256()
    for path in sorted(records):
        row = json.dumps([path, *records[path]], ensure_ascii=True, separators=(",", ":")).encode("ascii")
        digest.update(row + b"\n")
    return digest.hexdigest()


def _compare_trees(archive_tree: dict[str, tuple], source_tree: dict[str, tuple]) -> None:
    if archive_tree == source_tree:
        return
    for path in sorted(archive_tree.keys() | source_tree.keys()):
        if archive_tree.get(path) != source_tree.get(path):
            raise SourceVerificationError(f"extracted Linux source differs from pinned archive at: {path}")


def verify_archive_matches_directory(
    source_dir: Path, archive_path: Path, expected_archive_sha256: str = PINNED_ARCHIVE_SHA256
) -> dict[str, str]:
    archive_fd, opened = _open_regular_file(archive_path, "source archive")
    try:
        with os.fdopen(archive_fd, "rb", closefd=False) as stream:
            reader = _HashingReader(stream)
            archive_tree = _archive_tree(reader)
            while reader.read(CHUNK_SIZE):
                pass
        archive_sha256 = reader.digest.hexdigest()
        after = os.fstat(archive_fd)
        if (opened.st_dev, opened.st_ino, opened.st_size, opened.st_mtime_ns, opened.st_ctime_ns) != (
            after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns
        ):
            raise SourceVerificationError(f"source archive changed while verifying: {archive_path}")
    except (OSError, tarfile.TarError, EOFError, lzma.LZMAError) as error:
        raise SourceVerificationError(f"cannot read pinned Linux source archive: {error}") from None
    finally:
        os.close(archive_fd)
    if archive_sha256 != expected_archive_sha256:
        raise SourceVerificationError("Linux source archive SHA-256 does not match the pinned release")
    source_tree = _directory_tree(source_dir)
    _compare_trees(archive_tree, source_tree)
    return {"archive_sha256": archive_sha256, "tree_sha256": _tree_digest(source_tree)}


def tree_sha256(source_dir: Path) -> str:
    return _tree_digest(_directory_tree(source_dir))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    verify = commands.add_parser("verify")
    verify.add_argument("source_dir", type=Path)
    verify.add_argument("archive", type=Path)
    tree = commands.add_parser("tree")
    tree.add_argument("source_dir", type=Path)
    args = parser.parse_args(argv)
    try:
        if args.command == "verify":
            result = verify_archive_matches_directory(args.source_dir, args.archive)
            print(json.dumps(result, sort_keys=True, separators=(",", ":")))
        else:
            print(tree_sha256(args.source_dir))
    except SourceVerificationError as error:
        print(str(error), file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
