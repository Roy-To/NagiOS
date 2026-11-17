"""Strict host-side preflight for catalog-selected static x86-64 ELF artifacts."""

from __future__ import annotations

import argparse
import errno
import hashlib
import json
import os
import re
import stat
import struct
import sys
from dataclasses import dataclass
from typing import Any


SCHEMA_VERSION = 1
TARGET = "x86_64-unknown-linux-musl"
MAX_REQUEST_BYTES = 64 * 1024
HASH_CHUNK_BYTES = 1024 * 1024
ELF_HEADER_FORMAT = "<16sHHIQQQIHHHHHH"
ELF_HEADER_SIZE = struct.calcsize(ELF_HEADER_FORMAT)
PROGRAM_HEADER_FORMAT = "<IIQQQQQQ"
PROGRAM_HEADER_SIZE = struct.calcsize(PROGRAM_HEADER_FORMAT)
EM_X86_64 = 62
ET_EXEC = 2
ET_DYN = 3
PT_LOAD = 1
PT_INTERP = 3
PF_X = 1
PN_XNUM = 0xFFFF
UINT64_MAX = (1 << 64) - 1
REQUEST_FIELDS = frozenset(
    {"schema_version", "artifact_path", "expected_sha256", "target"}
)
SHA256_RE = re.compile(r"[0-9a-f]{64}\Z")


class PreflightError(Exception):
    """A rejected request or artifact with a stable machine-readable code."""

    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code
        self.message = message


@dataclass(frozen=True)
class Request:
    artifact_path: str
    expected_sha256: str
    target: str


@dataclass(frozen=True)
class ElfSummary:
    elf_type: int
    program_header_count: int


class VerifiedArtifact:
    """Context-managed owner of the descriptor whose bytes passed preflight."""

    def __init__(self, fd: int, result: dict[str, Any]):
        self._fd = fd
        self.result = result

    def fileno(self) -> int:
        if self._fd < 0:
            raise ValueError("verified artifact descriptor is closed")
        return self._fd

    def close(self) -> None:
        if self._fd >= 0:
            fd, self._fd = self._fd, -1
            os.close(fd)

    def detach_fd(self) -> int:
        """Transfer descriptor ownership to the caller, who must close it."""
        fd = self.fileno()
        self._fd = -1
        return fd

    def __enter__(self) -> "VerifiedArtifact":
        self.fileno()
        return self

    def __exit__(self, _exc_type: Any, _exc: Any, _traceback: Any) -> None:
        self.close()


def _duplicate_rejecting_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate JSON object field")
        result[key] = value
    return result


def _reject_json_constant(_value: str) -> None:
    raise ValueError("non-standard JSON number")


def parse_request_json(raw: str | bytes) -> Request:
    """Parse and strictly validate the fixed schema-v1 request."""
    if isinstance(raw, bytes):
        if len(raw) > MAX_REQUEST_BYTES:
            raise PreflightError("request_too_large", "JSON request exceeds 64 KiB")
        try:
            text = raw.decode("utf-8", errors="strict")
        except UnicodeDecodeError as exc:
            raise PreflightError("invalid_json", "request is not valid UTF-8") from exc
    elif isinstance(raw, str):
        try:
            encoded = raw.encode("utf-8", errors="strict")
        except UnicodeEncodeError as exc:
            raise PreflightError("invalid_json", "request is not valid UTF-8") from exc
        if len(encoded) > MAX_REQUEST_BYTES:
            raise PreflightError("request_too_large", "JSON request exceeds 64 KiB")
        text = raw
    else:
        raise PreflightError("invalid_json", "request must be UTF-8 JSON text")

    try:
        document = json.loads(
            text,
            object_pairs_hook=_duplicate_rejecting_object,
            parse_constant=_reject_json_constant,
        )
    except (ValueError, RecursionError) as exc:
        message = str(exc)
        code = "duplicate_json_field" if "duplicate JSON" in message else "invalid_json"
        raise PreflightError(code, message) from exc

    if not isinstance(document, dict):
        raise PreflightError("invalid_request", "top-level JSON value must be an object")

    missing = sorted(REQUEST_FIELDS - document.keys())
    if missing:
        raise PreflightError("missing_field", "required JSON field is missing")
    unknown = sorted(document.keys() - REQUEST_FIELDS)
    if unknown:
        raise PreflightError("unknown_field", "JSON contains an unknown field")

    version = document["schema_version"]
    if type(version) is not int or version != SCHEMA_VERSION:
        raise PreflightError("unsupported_schema_version", "schema_version must be integer 1")

    artifact_path = document["artifact_path"]
    if not isinstance(artifact_path, str) or not artifact_path or "\x00" in artifact_path:
        raise PreflightError("invalid_artifact_path", "artifact_path must be a non-empty path")
    try:
        os.fsencode(artifact_path)
    except UnicodeEncodeError as exc:
        raise PreflightError(
            "invalid_artifact_path", "artifact_path cannot be encoded by the host filesystem"
        ) from exc
    if any(component == ".." for component in artifact_path.split(os.sep)):
        raise PreflightError("invalid_artifact_path", "artifact_path must not contain '..'")

    expected_sha256 = document["expected_sha256"]
    if not isinstance(expected_sha256, str) or not SHA256_RE.fullmatch(expected_sha256):
        raise PreflightError(
            "invalid_expected_sha256",
            "expected_sha256 must contain 64 lowercase hexadecimal characters",
        )

    target = document["target"]
    if not isinstance(target, str) or target != TARGET:
        raise PreflightError("unsupported_target", "target must be x86_64-unknown-linux-musl")

    return Request(artifact_path, expected_sha256, target)


def _stat_signature(info: os.stat_result) -> tuple[int, int, int, int, int, int]:
    return (
        info.st_dev,
        info.st_ino,
        stat.S_IFMT(info.st_mode),
        info.st_size,
        info.st_mtime_ns,
        info.st_ctime_ns,
    )


def _map_open_error(exc: OSError, *, component: bool = False) -> PreflightError:
    if exc.errno == errno.ENOENT:
        return PreflightError("artifact_not_found", "artifact path does not exist")
    if exc.errno == errno.EACCES or exc.errno == errno.EPERM:
        return PreflightError("artifact_access_denied", "artifact path cannot be read")
    if exc.errno == errno.ELOOP:
        return PreflightError("symlink_not_allowed", "artifact path contains a symlink")
    if exc.errno == errno.ENOTDIR and component:
        return PreflightError(
            "invalid_artifact_path", "an artifact path component is not a directory"
        )
    return PreflightError("artifact_open_failed", "artifact path could not be opened safely")


def _open_artifact(path: str) -> tuple[int, int, str, os.stat_result]:
    """Open by directory descriptors so symlink path components cannot be followed."""
    nofollow = getattr(os, "O_NOFOLLOW", None)
    directory = getattr(os, "O_DIRECTORY", None)
    nonblock = getattr(os, "O_NONBLOCK", None)
    if nofollow is None or directory is None or nonblock is None or not hasattr(os, "pread"):
        raise PreflightError(
            "unsupported_host",
            "host must provide O_NOFOLLOW, O_DIRECTORY, O_NONBLOCK, and pread",
        )

    absolute = path.startswith(os.sep)
    components = [part for part in path.split(os.sep) if part not in ("", ".")]
    if not components:
        raise PreflightError("invalid_artifact_path", "artifact_path must name a file")

    directory_flags = os.O_RDONLY | directory | nofollow | getattr(os, "O_CLOEXEC", 0)
    file_flags = (
        os.O_RDONLY
        | nofollow
        | nonblock
        | getattr(os, "O_CLOEXEC", 0)
    )
    try:
        parent_fd = os.open(os.sep if absolute else ".", directory_flags)
    except OSError as exc:
        raise _map_open_error(exc, component=True) from exc

    try:
        for component in components[:-1]:
            try:
                next_fd = os.open(component, directory_flags, dir_fd=parent_fd)
            except OSError as exc:
                if exc.errno in (errno.ELOOP, errno.ENOTDIR):
                    try:
                        component_stat = os.stat(
                            component, dir_fd=parent_fd, follow_symlinks=False
                        )
                    except OSError:
                        component_stat = None
                    if component_stat is not None and stat.S_ISLNK(component_stat.st_mode):
                        raise PreflightError(
                            "symlink_not_allowed", "artifact path contains a symlink"
                        ) from exc
                raise _map_open_error(exc, component=True) from exc
            os.close(parent_fd)
            parent_fd = next_fd

        name = components[-1]
        try:
            path_stat = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
        except OSError as exc:
            raise _map_open_error(exc) from exc
        if stat.S_ISLNK(path_stat.st_mode):
            raise PreflightError("symlink_not_allowed", "artifact path names a symlink")
        if not stat.S_ISREG(path_stat.st_mode):
            raise PreflightError("not_regular_file", "artifact must be a regular file")

        try:
            artifact_fd = os.open(name, file_flags, dir_fd=parent_fd)
        except OSError as exc:
            raise _map_open_error(exc) from exc

        try:
            opened_stat = os.fstat(artifact_fd)
            if not stat.S_ISREG(opened_stat.st_mode):
                raise PreflightError("not_regular_file", "artifact must be a regular file")
            if _stat_signature(path_stat) != _stat_signature(opened_stat):
                raise PreflightError(
                    "artifact_changed_during_preflight",
                    "artifact changed while it was being opened",
                )
            return artifact_fd, parent_fd, name, opened_stat
        except BaseException:
            os.close(artifact_fd)
            raise
    except BaseException:
        os.close(parent_fd)
        raise


def _pread_exact(fd: int, length: int, offset: int, *, what: str) -> bytes:
    chunks: list[bytes] = []
    remaining = length
    current_offset = offset
    while remaining:
        try:
            chunk = os.pread(fd, remaining, current_offset)
        except OSError as exc:
            raise PreflightError("artifact_read_failed", f"could not read {what}") from exc
        if not chunk:
            raise PreflightError("truncated_elf", f"ELF {what} is truncated")
        chunks.append(chunk)
        remaining -= len(chunk)
        current_offset += len(chunk)
    return b"".join(chunks)


def _validate_elf(fd: int, file_size: int) -> ElfSummary:
    if file_size < ELF_HEADER_SIZE:
        raise PreflightError("truncated_elf", "ELF header is incomplete")

    header = _pread_exact(fd, ELF_HEADER_SIZE, 0, what="header")
    (
        ident,
        elf_type,
        machine,
        version,
        entry,
        program_header_offset,
        _section_header_offset,
        _flags,
        header_size,
        program_header_entry_size,
        program_header_count,
        _section_header_entry_size,
        _section_header_count,
        _section_name_index,
    ) = struct.unpack(ELF_HEADER_FORMAT, header)

    if ident[:4] != b"\x7fELF":
        raise PreflightError("invalid_elf_magic", "artifact does not have ELF magic")
    if ident[4] != 2:
        raise PreflightError("wrong_elf_class", "artifact must use ELF64 class")
    if ident[5] != 1:
        raise PreflightError("wrong_elf_endianness", "artifact must be little-endian ELF")
    if ident[6] != 1 or version != 1:
        raise PreflightError("invalid_elf_version", "ELF version must be current")
    if machine != EM_X86_64:
        raise PreflightError("wrong_architecture", "ELF machine must be x86-64")
    if elf_type not in (ET_EXEC, ET_DYN):
        raise PreflightError("not_executable_elf", "ELF type must be ET_EXEC or ET_DYN")
    if header_size != ELF_HEADER_SIZE:
        raise PreflightError("invalid_elf_header_size", "ELF64 header size must be 64 bytes")
    if program_header_entry_size != PROGRAM_HEADER_SIZE:
        raise PreflightError(
            "invalid_program_header_size", "ELF64 program-header entries must be 56 bytes"
        )
    if program_header_count == 0:
        raise PreflightError("missing_program_headers", "executable ELF has no program headers")
    if program_header_count == PN_XNUM:
        raise PreflightError(
            "unsupported_program_header_count",
            "extended ELF program-header counts are unsupported",
        )
    if program_header_offset < header_size or program_header_offset > file_size:
        raise PreflightError(
            "program_header_table_out_of_bounds", "ELF program-header table starts out of bounds"
        )
    available = file_size - program_header_offset
    if program_header_count > available // program_header_entry_size:
        raise PreflightError(
            "program_header_table_out_of_bounds", "ELF program-header table exceeds the file"
        )

    table_size = program_header_count * program_header_entry_size
    table = _pread_exact(
        fd, table_size, program_header_offset, what="program-header table"
    )
    has_load_segment = False
    entry_is_executable = False

    for index in range(program_header_count):
        start = index * program_header_entry_size
        item = struct.unpack_from(PROGRAM_HEADER_FORMAT, table, start)
        segment_type, segment_flags, file_offset, virtual_address, _physical_address, file_bytes, memory_bytes, _alignment = item

        if segment_type == PT_INTERP:
            raise PreflightError(
                "dynamic_interpreter_present",
                "ELF PT_INTERP is present; static-loader condition failed",
            )
        if segment_type == 0:  # PT_NULL entries do not describe file ranges.
            continue
        if file_offset > file_size or file_bytes > file_size - file_offset:
            raise PreflightError(
                "segment_out_of_bounds", "ELF program segment exceeds the artifact"
            )

        if segment_type == PT_LOAD:
            has_load_segment = True
            if memory_bytes < file_bytes:
                raise PreflightError(
                    "invalid_load_segment", "PT_LOAD memory size is smaller than file size"
                )
            if virtual_address > UINT64_MAX - memory_bytes:
                raise PreflightError(
                    "invalid_load_segment", "PT_LOAD virtual address range overflows ELF64"
                )
            if segment_flags & PF_X:
                end = virtual_address + memory_bytes
                if virtual_address <= entry < end:
                    entry_is_executable = True

    if not has_load_segment:
        raise PreflightError("missing_load_segment", "executable ELF has no PT_LOAD segment")
    if not entry_is_executable:
        raise PreflightError(
            "entry_not_executable", "ELF entry point is outside executable PT_LOAD segments"
        )

    return ElfSummary(elf_type, program_header_count)


def _hash_descriptor(fd: int, file_size: int) -> str:
    digest = hashlib.sha256()
    offset = 0
    while offset < file_size:
        length = min(HASH_CHUNK_BYTES, file_size - offset)
        try:
            chunk = os.pread(fd, length, offset)
        except OSError as exc:
            raise PreflightError("artifact_read_failed", "could not hash artifact bytes") from exc
        if not chunk:
            raise PreflightError(
                "artifact_changed_during_preflight", "artifact became shorter while being read"
            )
        digest.update(chunk)
        offset += len(chunk)
    return digest.hexdigest()


def _verify_artifact_unchanged(
    fd: int,
    parent_fd: int,
    name: str,
    initial_stat: os.stat_result,
) -> None:
    try:
        descriptor_stat = os.fstat(fd)
        path_stat = os.stat(name, dir_fd=parent_fd, follow_symlinks=False)
    except OSError as exc:
        raise PreflightError(
            "artifact_changed_during_preflight", "artifact path changed while being checked"
        ) from exc
    if stat.S_ISLNK(path_stat.st_mode):
        raise PreflightError(
            "artifact_changed_during_preflight", "artifact path became a symlink while checked"
        )
    if (
        _stat_signature(initial_stat) != _stat_signature(descriptor_stat)
        or _stat_signature(initial_stat) != _stat_signature(path_stat)
    ):
        raise PreflightError(
            "artifact_changed_during_preflight", "artifact changed while it was being checked"
        )


def _verify_request(request: Request) -> tuple[dict[str, Any], int]:
    """Verify a request and return its report plus the still-open verified FD."""
    artifact_fd, parent_fd, name, initial_stat = _open_artifact(request.artifact_path)
    try:
        elf = _validate_elf(artifact_fd, initial_stat.st_size)
        actual_sha256 = _hash_descriptor(artifact_fd, initial_stat.st_size)
        _verify_artifact_unchanged(artifact_fd, parent_fd, name, initial_stat)
        if actual_sha256 != request.expected_sha256:
            raise PreflightError(
                "digest_mismatch", "artifact SHA-256 does not match the trusted expected value"
            )

        result = {
            "status": "artifact_preflight_passed",
            "schema_version": SCHEMA_VERSION,
            "target": request.target,
            "artifact_sha256": actual_sha256,
            "elf_type": "ET_EXEC" if elf.elf_type == ET_EXEC else "ET_DYN",
            "program_header_count": elf.program_header_count,
            "checks": {
                "regular_file": True,
                "no_symlink_path_components": True,
                "sha256_matches_expected": True,
                "elf64_little_endian": True,
                "x86_64": True,
                "executable_elf": True,
                "no_pt_interp": True,
            },
            "claim_scope": "artifact_preflight_only",
            "not_proven": [
                "signature",
                "canonical_package_identity",
                "service_role",
                "user_authentication",
                "policy_authorization",
                "installation",
                "runtime_ready",
            ],
        }
    except BaseException:
        os.close(artifact_fd)
        raise
    finally:
        os.close(parent_fd)
    return result, artifact_fd


def preflight_request(request: Request) -> dict[str, Any]:
    """Verify one request and close the checked artifact descriptor."""
    result, artifact_fd = _verify_request(request)
    os.close(artifact_fd)
    return result


def preflight_json(raw: str | bytes) -> dict[str, Any]:
    """Parse schema-v1 JSON and perform artifact preflight."""
    return preflight_request(parse_request_json(raw))


def open_verified_artifact(raw: str | bytes) -> VerifiedArtifact:
    """Return the verified descriptor for caller use without reopening its path.

    The returned context manager owns the descriptor. Use ``fileno()`` while
    it is open and let the context manager close it, or call ``detach_fd()``
    and close the transferred descriptor yourself.
    """
    result, artifact_fd = _verify_request(parse_request_json(raw))
    try:
        return VerifiedArtifact(artifact_fd, result)
    except BaseException:
        os.close(artifact_fd)
        raise


def _read_request_input(input_path: str | None) -> bytes:
    try:
        if input_path is None or input_path == "-":
            data = sys.stdin.buffer.read(MAX_REQUEST_BYTES + 1)
        else:
            with open(input_path, "rb") as request_file:
                data = request_file.read(MAX_REQUEST_BYTES + 1)
    except OSError as exc:
        raise PreflightError("request_read_failed", "could not read JSON request") from exc
    if len(data) > MAX_REQUEST_BYTES:
        raise PreflightError("request_too_large", "JSON request exceeds 64 KiB")
    return data


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Preflight a catalog-selected service ELF")
    parser.add_argument(
        "--input",
        metavar="JSON_FILE",
        help="read the JSON request from this file (default: standard input)",
    )
    args = parser.parse_args(argv)

    try:
        result = preflight_json(_read_request_input(args.input))
    except PreflightError as exc:
        print(json.dumps({"status": "artifact_preflight_failed", "error": {"code": exc.code, "message": exc.message}}), file=sys.stderr)
        return 2

    print(json.dumps(result, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
