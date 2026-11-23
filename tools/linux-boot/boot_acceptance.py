"""Pure development-boot evidence checks; no runner, file access or guest PASS.

Design: reuse prepare's ELF contract, independently parse its uncompressed newc
output, then compare /init bytes. Serial observations remain separate from artifact
validity. Caller hashes pin supplied bytes, not provenance or a QEMU invocation.
No authenticated service readiness contract is available, so READY text cannot
establish service success. This module neither accepts nor releases runtime gates.
"""

import hashlib
import re
import stat
from decimal import Decimal

import prepare


class AcceptanceError(ValueError):
    def __init__(self, code: str):
        self.code = code
        super().__init__(code)


def _bytes(data, limit, code):
    if type(data) is not bytes or len(data) > limit:
        raise AcceptanceError(code)


def _sha(data):
    return hashlib.sha256(data).hexdigest()


def _parse_newc_records(
    data: bytes,
    *,
    retain_payloads: set[str] | None = None,
) -> tuple[bytes, list[tuple[str, list[int], bytes]]]:
    """Strict single newc archive, canonical relative paths, no hardlink init.

    Not a general Linux initramfs decoder: gzip/CRC/concatenated archives and
    symlinks are outside prepare's output contract and fail closed.
    """
    _bytes(data, 128 * 1024 * 1024, "CPIO_INPUT_LIMIT")
    cursor = 0
    names = set()
    inodes = set()
    init = None
    entries: list[tuple[str, list[int], bytes]] = []
    while cursor < len(data):
        if len(names) >= 4096:
            raise AcceptanceError("CPIO_ENTRY_LIMIT")
        header = data[cursor:cursor + 110]
        if len(header) != 110 or header[:6] != b"070701":
            raise AcceptanceError("CPIO_HEADER")
        if re.fullmatch(b"[0-9a-fA-F]{104}", header[6:]) is None:
            raise AcceptanceError("CPIO_HEADER")
        fields = [int(header[i:i + 8], 16) for i in range(6, 110, 8)]
        _, mode, _, _, nlink, _, size, _, _, _, _, namesize, check = fields
        if not 1 < namesize <= 4096 or check:
            raise AcceptanceError("CPIO_HEADER")
        cursor += 110
        rawname = data[cursor:cursor + namesize]
        if len(rawname) != namesize or rawname[-1:] != b"\0" or b"\0" in rawname[:-1]:
            raise AcceptanceError("CPIO_NAME")
        try:
            name = rawname[:-1].decode("ascii")
        except UnicodeDecodeError:
            raise AcceptanceError("CPIO_NAME") from None
        if (any(ord(c) < 33 or ord(c) > 126 or c == "\\" for c in name)
                or any(p in ("", ".", "..") for p in name.split("/"))):
            raise AcceptanceError("CPIO_NAME")
        if name in names:
            raise AcceptanceError("CPIO_DUPLICATE")
        names.add(name)
        endname = cursor + namesize
        cursor = (endname + 3) & ~3
        if cursor > len(data) or any(data[endname:cursor]):
            raise AcceptanceError("CPIO_PADDING")
        enddata = cursor + size
        endentry = (enddata + 3) & ~3
        if endentry > len(data):
            raise AcceptanceError("CPIO_TRUNCATED")
        if any(data[enddata:endentry]):
            raise AcceptanceError("CPIO_PADDING")
        payload = (
            data[cursor:enddata]
            if retain_payloads is None or name == "init" or name in retain_payloads
            else b""
        )
        cursor = endentry
        if name == "TRAILER!!!":
            entries.append((name, fields, payload))
            if size or mode or nlink != 1 or any(data[cursor:]):
                raise AcceptanceError("CPIO_TRAILER")
            if init is None:
                raise AcceptanceError("CPIO_INIT_MISSING")
            return init, entries
        # stat.S_IFMT delegates to a platform C integer conversion; newc fields
        # are unsigned 32-bit and must not leak OverflowError on Darwin.
        if mode > 0xFFFF:
            raise AcceptanceError("CPIO_ENTRY_TYPE")
        kind = stat.S_IFMT(mode)
        if kind not in (stat.S_IFDIR, stat.S_IFCHR, stat.S_IFREG):
            raise AcceptanceError("CPIO_ENTRY_TYPE")
        if name != "init" and mode & (stat.S_ISUID | stat.S_ISGID):
            raise AcceptanceError("CPIO_PRIVILEGED_MODE")
        identity = (fields[7], fields[8], fields[0])
        if identity in inodes or nlink != (2 if kind == stat.S_IFDIR else 1):
            raise AcceptanceError("CPIO_LINK")
        inodes.add(identity)
        if kind != stat.S_IFREG and size:
            raise AcceptanceError("CPIO_ENTRY_DATA")
        entries.append((name, fields, payload))
        if name == "init":
            if kind != stat.S_IFREG or nlink != 1 or mode & 0o7777 != 0o755:
                raise AcceptanceError("CPIO_INIT_MODE")
            init = payload
        elif name.startswith("init/"):
            raise AcceptanceError("CPIO_INIT_PATH_CONFLICT")
    raise AcceptanceError("CPIO_TRAILER_MISSING")


def _parse_newc(data: bytes) -> tuple[bytes, list[tuple[str, list[int]]]]:
    """Compatibility view of the strict parser without retaining file payloads."""
    init, entries = _parse_newc_records(data, retain_payloads=set())
    return init, [(name, fields) for name, fields, _ in entries]


def _init_from_newc(data: bytes) -> bytes:
    return _parse_newc(data)[0]


def _expected_x86_prepared_entries(init_size: int) -> list[tuple[str, list[int]]]:
    """The exact ordered CPIO metadata emitted by prepare.py for x86_64."""
    specifications = [
        ("dev", stat.S_IFDIR | 0o755, 2, 0, 0, 0),
        ("dev/console", stat.S_IFCHR | 0o600, 1, 5, 1, 0),
        ("dev/null", stat.S_IFCHR | 0o666, 1, 1, 3, 0),
        ("dev/ttyS0", stat.S_IFCHR | 0o600, 1, 4, 64, 0),
        ("proc", stat.S_IFDIR | 0o555, 2, 0, 0, 0),
        ("sys", stat.S_IFDIR | 0o555, 2, 0, 0, 0),
        ("run", stat.S_IFDIR | 0o755, 2, 0, 0, 0),
        ("tmp", stat.S_IFDIR | 0o1777, 2, 0, 0, 0),
        ("init", stat.S_IFREG | 0o755, 1, 0, 0, init_size),
        ("TRAILER!!!", 0, 1, 0, 0, 0),
    ]
    result = []
    for inode, (name, mode, nlink, rdevmajor, rdevminor, size) in enumerate(specifications, 1):
        fields = [inode, mode, 0, 0, nlink, 0, size, 0, 0,
                  rdevmajor, rdevminor, len(name.encode("ascii")) + 1, 0]
        result.append((name, fields))
    return result


def _validate_x86_prepared_cpio(entries: list[tuple[str, list[int]]], init_size: int) -> None:
    expected = _expected_x86_prepared_entries(init_size)
    if [name for name, _ in entries] != [name for name, _ in expected]:
        raise AcceptanceError("CPIO_PREPARE_ENTRY_SET")
    for (name, actual), (_, wanted) in zip(entries, expected):
        if name == "dev/console" and actual[9:11] != wanted[9:11]:
            raise AcceptanceError("CPIO_PREPARE_CONSOLE_DEVICE")
        if actual != wanted:
            raise AcceptanceError("CPIO_PREPARE_METADATA")


def validate_artifacts(init: bytes, initramfs: bytes) -> dict:
    """Validate a static x86_64 init ELF and prepare.py's exact x86 CPIO."""
    _bytes(init, 64 * 1024 * 1024, "ELF_INPUT_LIMIT")
    try:
        prepare.validate_init_elf_bytes(init, "x86_64")
    except prepare.InputError as error:
        raise AcceptanceError("ELF_CONTRACT") from error
    archived_init, entries = _parse_newc(initramfs)
    if archived_init != init:
        raise AcceptanceError("CPIO_INIT_MISMATCH")
    _validate_x86_prepared_cpio(entries, len(init))
    return {"static_elf_contract": "PASS", "cpio_init_byte_identity": "PASS",
            "x86_prepared_cpio_contract": "PASS",
            "init_sha256": _sha(init), "initramfs_sha256": _sha(initramfs),
            "guest_execution_verified": False}


def validate_development_cpio_inputs(
    initramfs: bytes,
    expected_files: dict[str, tuple[int, bytes]],
    *,
    expected_directories: dict[str, int] | None = None,
    expected_character_devices: dict[str, tuple[int, int]] | None = None,
) -> dict:
    """Check exact caller-supplied development files inside a strict newc CPIO.

    `expected_files` maps canonical archive-relative paths to `(mode, bytes)`;
    `expected_character_devices` maps paths to `(rdev_major, rdev_minor)`.
    This is only an input inclusion check: it does not establish package trust,
    service identity, execution, an authenticated peer, a handshake, or READY.
    """
    if expected_directories is None:
        expected_directories = {}
    if expected_character_devices is None:
        expected_character_devices = {}
    if (type(expected_files) is not dict or type(expected_directories) is not dict
            or type(expected_character_devices) is not dict
            or not expected_files and not expected_directories and not expected_character_devices):
        raise AcceptanceError("DEV_CPIO_EXPECTED_FILES")
    if (set(expected_files) & set(expected_directories)
            or set(expected_files) & set(expected_character_devices)
            or set(expected_directories) & set(expected_character_devices)):
        raise AcceptanceError("DEV_CPIO_EXPECTED_PATH")

    def validate_path(name: str) -> None:
        if (type(name) is not str or not name or name.startswith("/")
                or any(part in ("", ".", "..") for part in name.split("/"))
                or any(ord(char) < 33 or ord(char) > 126 or char == "\\" for char in name)
                or name in ("init", "TRAILER!!!") or name.startswith("init/")):
            raise AcceptanceError("DEV_CPIO_EXPECTED_PATH")

    for name, expected in expected_files.items():
        validate_path(name)
        if type(expected) is not tuple or len(expected) != 2:
            raise AcceptanceError("DEV_CPIO_EXPECTED_FILES")
        mode, payload = expected
        if type(mode) is not int or mode < 0 or mode > 0o777:
            raise AcceptanceError("DEV_CPIO_EXPECTED_MODE")
        if type(payload) is not bytes or not payload or len(payload) > 64 * 1024 * 1024:
            raise AcceptanceError("DEV_CPIO_EXPECTED_PAYLOAD")
    for name, mode in expected_directories.items():
        validate_path(name)
        if type(mode) is not int or mode < 0 or mode > 0o777:
            raise AcceptanceError("DEV_CPIO_EXPECTED_MODE")
    for name, device in expected_character_devices.items():
        validate_path(name)
        if (type(device) is not tuple or len(device) != 2
                or any(type(number) is not int or number < 0 or number > 0xFFFFFFFF
                       for number in device)):
            raise AcceptanceError("DEV_CPIO_EXPECTED_DEVICE")

    _, records = _parse_newc_records(
        initramfs,
        retain_payloads=set(expected_files),
    )
    actual = {name: (fields, payload) for name, fields, payload in records}

    def require_directory(name: str) -> None:
        record = actual.get(name)
        if record is None:
            raise AcceptanceError("DEV_CPIO_PARENT_MISSING")
        if stat.S_IFMT(record[0][1]) != stat.S_IFDIR:
            raise AcceptanceError("DEV_CPIO_PARENT_MODE")
        if name in expected_directories and record[0][1] & 0o7777 != expected_directories[name]:
            raise AcceptanceError("DEV_CPIO_DIRECTORY_MODE")

    for name, expected_mode in expected_directories.items():
        parts = name.split("/")
        for index in range(1, len(parts)):
            require_directory("/".join(parts[:index]))
        record = actual.get(name)
        if record is None:
            raise AcceptanceError("DEV_CPIO_INPUT_MISSING")
        mode = record[0][1]
        if stat.S_IFMT(mode) != stat.S_IFDIR or mode & 0o7777 != expected_mode:
            raise AcceptanceError("DEV_CPIO_DIRECTORY_MODE")

    accepted = []
    for name, (expected_mode, expected_payload) in sorted(expected_files.items()):
        parents = name.split("/")[:-1]
        for index in range(1, len(parents) + 1):
            require_directory("/".join(parents[:index]))
        record = actual.get(name)
        if record is None:
            raise AcceptanceError("DEV_CPIO_INPUT_MISSING")
        fields, payload = record
        mode = fields[1]
        if stat.S_IFMT(mode) != stat.S_IFREG or mode & 0o7777 != expected_mode:
            raise AcceptanceError("DEV_CPIO_INPUT_MODE")
        if payload != expected_payload:
            raise AcceptanceError("DEV_CPIO_INPUT_MISMATCH")
        accepted.append({
            "path": name,
            "mode": expected_mode,
            "size_bytes": len(payload),
            "sha256": _sha(payload),
        })
    accepted_devices = []
    for name, (expected_major, expected_minor) in sorted(expected_character_devices.items()):
        parents = name.split("/")[:-1]
        for index in range(1, len(parents) + 1):
            require_directory("/".join(parents[:index]))
        record = actual.get(name)
        if record is None:
            raise AcceptanceError("DEV_CPIO_DEVICE_MISSING")
        fields = record[0]
        if stat.S_IFMT(fields[1]) != stat.S_IFCHR:
            raise AcceptanceError("DEV_CPIO_DEVICE_MODE")
        if (fields[9], fields[10]) != (expected_major, expected_minor):
            raise AcceptanceError("DEV_CPIO_DEVICE_ID")
        accepted_devices.append({
            "path": name,
            "type": "character",
            "rdev_major": expected_major,
            "rdev_minor": expected_minor,
        })
    return {
        "cpio_structure": "PASS",
        "development_input_bytes_match": "PASS" if expected_files else "NOT_CHECKED",
        "development_directories": [
            {"path": name, "mode": mode}
            for name, mode in sorted(expected_directories.items())
        ],
        "development_inputs": accepted,
        "development_character_devices": accepted_devices,
        "service_execution_verified": False,
        "handshake_verified": False,
        "ready_verified": False,
    }


_TIMESTAMP = re.compile(r"^\[\s*(\d+\.\d+)\]\s*")
_INIT_ATTEMPT = re.compile(r"^Run (.+) as init process$")
_UNAVAILABLE = "nagi-init: startup failed (Unavailable); no service is ready"
_REFUSAL = "nagi-init: refusing Linux init entry outside PID 1"


def observe_serial(serial: bytes) -> dict:
    """Classify complete diagnostic lines only, never authenticate a boot.

    Run /init is printed before kernel_execve (signed Linux 6.18.63 init/main.c).
    Unavailable is after the production PID1 guard in fixed linux_init.rs
    98d3e5ea13e23aaac8f7b38409658908bb775f2575702eef9f35fdcd33621d7d.
    That function is also unit-tested outside PID1: text is conditional evidence,
    never standalone proof of PID, backend identity, or the loaded executable.
    """
    _bytes(serial, 4 * 1024 * 1024, "SERIAL_INPUT_LIMIT")
    try:
        text = serial.decode("utf-8")
    except UnicodeDecodeError:
        raise AcceptanceError("SERIAL_ENCODING") from None
    if any(ord(c) < 32 and c not in "\n\r\t" or 127 <= ord(c) <= 159 for c in text):
        raise AcceptanceError("SERIAL_CONTROL")
    parts = text.replace("\r\n", "\n").split("\n")
    if len(parts) - 1 > 65536 or any(len(line) > 65536 or "\r" in line for line in parts):
        raise AcceptanceError("SERIAL_LINE")
    lines = parts[:-1]
    observations = []
    previous_timestamp = None
    for number, line in enumerate(lines, 1):
        timestamp = _TIMESTAMP.match(line)
        if timestamp:
            current_timestamp = Decimal(timestamp.group(1))
            if previous_timestamp is not None and current_timestamp < previous_timestamp:
                observations.append({"line": number, "event": "TIMESTAMP_REWIND"})
            previous_timestamp = current_timestamp
            line = line[timestamp.end():]
        event = None
        attempt = _INIT_ATTEMPT.fullmatch(line)
        if attempt:
            event = ("INIT_EXEC_ATTEMPT" if attempt.group(1) == "/init"
                     else "OTHER_INIT_EXEC_ATTEMPT")
        elif line == _UNAVAILABLE:
            event = "POST_GUARD_UNAVAILABLE_DIAGNOSTIC"
        elif line == _REFUSAL:
            event = "NON_PID1_REFUSAL"
        elif (failure := re.fullmatch(r"Failed to execute (.+) \(error -?\d+\)", line)):
            event = ("INIT_EXEC_FAILED" if failure.group(1) == "/init"
                     else "OTHER_INIT_EXEC_FAILED")
        elif line.startswith("Kernel panic - not syncing: "):
            event = ("NO_WORKING_INIT" if line.startswith("Kernel panic - not syncing: No working init found.")
                     else "REQUESTED_INIT_FAILED" if re.match(
                         r"Kernel panic - not syncing: Requested init /init failed \(error -?\d+\)\.", line)
                     else "KERNEL_PANIC")
        elif line.startswith("Linux version "):
            event = "KERNEL_BANNER"
        if event:
            observations.append({"line": number, "event": event})
    kinds = [item["event"] for item in observations]
    positions = {event: [i for i, kind in enumerate(kinds) if kind == event]
                 for event in ("KERNEL_BANNER", "INIT_EXEC_ATTEMPT",
                               "POST_GUARD_UNAVAILABLE_DIAGNOSTIC")}
    duplicate_markers = any(len(indices) > 1 for indices in positions.values())
    wrong_order = any(
        len(positions[left]) == len(positions[right]) == 1
        and positions[left][0] >= positions[right][0]
        for left, right in (("KERNEL_BANNER", "INIT_EXEC_ATTEMPT"),
                            ("INIT_EXEC_ATTEMPT", "POST_GUARD_UNAVAILABLE_DIAGNOSTIC"),
                            ("KERNEL_BANNER", "POST_GUARD_UNAVAILABLE_DIAGNOSTIC")))
    marker_evidence = any(positions.values())
    blocking_events = {"NON_PID1_REFUSAL", "INIT_EXEC_FAILED", "NO_WORKING_INIT",
                       "REQUESTED_INIT_FAILED", "OTHER_INIT_EXEC_ATTEMPT",
                       "OTHER_INIT_EXEC_FAILED", "TIMESTAMP_REWIND"}
    has_blocker = any(kind in blocking_events for kind in kinds)
    unavailable_positions = positions["POST_GUARD_UNAVAILABLE_DIAGNOSTIC"]
    panic_before_unavailable = any(
        kind == "KERNEL_PANIC" and (not unavailable_positions or i < unavailable_positions[0])
        for i, kind in enumerate(kinds))
    contradictory = (duplicate_markers or wrong_order
                     or marker_evidence and has_blocker
                     or marker_evidence and panic_before_unavailable
                     or "TIMESTAMP_REWIND" in kinds)
    stage = "UNKNOWN"
    if "KERNEL_BANNER" in kinds:
        stage = "KERNEL_TEXT_OBSERVED"
    if "INIT_EXEC_ATTEMPT" in kinds:
        stage = "INIT_EXEC_ATTEMPT_OBSERVED"
    if "POST_GUARD_UNAVAILABLE_DIAGNOSTIC" in kinds:
        stage = "POST_GUARD_UNAVAILABLE_TEXT_OBSERVED"
    if "NON_PID1_REFUSAL" in kinds:
        stage = "NON_PID1_REFUSAL_OBSERVED"
    if "OTHER_INIT_EXEC_ATTEMPT" in kinds:
        stage = "OTHER_INIT_EXEC_ATTEMPT_OBSERVED"
    if contradictory:
        stage = "AMBIGUOUS_TRANSCRIPT"
    ordered = (not contradictory
               and all(len(positions[event]) == 1 for event in positions)
               and positions["KERNEL_BANNER"][0] < positions["INIT_EXEC_ATTEMPT"][0]
               < positions["POST_GUARD_UNAVAILABLE_DIAGNOSTIC"][0])
    return {"stage": stage, "observations": observations,
            "ordered_post_guard_candidate": ordered,
            "failure": ("AMBIGUOUS_TRANSCRIPT" if contradictory else
                        "NO_WORKING_INIT" if "NO_WORKING_INIT" in kinds else
                        "REQUESTED_INIT_FAILED" if "REQUESTED_INIT_FAILED" in kinds else
                        "KERNEL_PANIC" if "KERNEL_PANIC" in kinds else
                        "INIT_EXEC_FAILED" if "INIT_EXEC_FAILED" in kinds else
                        "OTHER_INIT_EXEC_ATTEMPT" if "OTHER_INIT_EXEC_ATTEMPT" in kinds else
                        "OTHER_INIT_EXEC_FAILED" if "OTHER_INIT_EXEC_FAILED" in kinds else
                        "TIMESTAMP_REWIND" if "TIMESTAMP_REWIND" in kinds else
                        "BACKEND_UNAVAILABLE_DIAGNOSTIC" if "POST_GUARD_UNAVAILABLE_DIAGNOSTIC" in kinds else
                        "NON_PID1_REFUSAL" if "NON_PID1_REFUSAL" in kinds else "UNKNOWN"),
            "service_ready": "NOT_READY_DIAGNOSTIC" if ordered else "UNKNOWN",
            "trailing_partial_line_ignored": bool(text and not text.endswith("\n")),
            "actual_pid1_verified": False, "minimal_service_ready_verified": False}


def evaluate(inputs: dict, *, init: bytes, initramfs: bytes, serial: bytes) -> dict:
    """v1 input schema is closed; all expected hashes are caller references."""
    keys = {"schema", "evidence_kind", "init_sha256", "initramfs_sha256", "serial_sha256"}
    if (type(inputs) is not dict or set(inputs) != keys
            or inputs["schema"] != "nagi-linux-boot-evidence-input-v1"
            or inputs["evidence_kind"] not in ("synthetic_fixture", "unverified_capture")):
        raise AcceptanceError("INPUT_SCHEMA")
    for name, data, limit in (("init", init, 64 * 1024 * 1024),
                              ("initramfs", initramfs, 128 * 1024 * 1024),
                              ("serial", serial, 4 * 1024 * 1024)):
        _bytes(data, limit, "INPUT_BYTES")
        expected = inputs[name + "_sha256"]
        if type(expected) is not str or re.fullmatch("[0-9a-f]{64}", expected) is None:
            raise AcceptanceError("INPUT_HASH_FORMAT")
        if _sha(data) != expected:
            raise AcceptanceError("INPUT_HASH_MISMATCH")
    return {"schema": "nagi-linux-boot-evidence-result-v1",
            "evidence_kind": inputs["evidence_kind"],
            "artifacts": validate_artifacts(init, initramfs),
            "serial": observe_serial(serial), "serial_sha256": _sha(serial),
            "run_binding_verified": False, "guest_acceptance": "NOT_VERIFIED",
            "checks_not_performed": ["kernel_validation", "QEMU_invocation_binding",
                                     "PID1_runtime", "authenticated_service_READY"]}


def verification_receipt(inputs: dict, *, init: bytes, initramfs: bytes, serial: bytes) -> dict:
    """Return a JSON-compatible offline receipt; never execute or persist it.

    returncode is this validation's suggested exit code, not a QEMU/guest exit.
    PASS covers supplied bytes only. Hashes have no source/run authentication.
    A future authorized capture may use unverified_capture, without changing any
    of the runtime gates. Acquisition/authorization is the caller's obligation.
    """
    byte_inputs = {"init": init, "initramfs": initramfs, "serial": serial}
    identities = {
        name: ({"sha256": _sha(data), "size_bytes": len(data)} if type(data) is bytes
               and len(data) <= (128 * 1024 * 1024 if name == "initramfs" else
                                 64 * 1024 * 1024 if name == "init" else 4 * 1024 * 1024)
               else {"sha256": None, "size_bytes": None, "invalid_input": True})
        for name, data in byte_inputs.items()
    }
    references = {}
    for name in byte_inputs:
        expected = inputs.get(name + "_sha256") if type(inputs) is dict else None
        references[name + "_sha256"] = (
            expected if type(expected) is str and re.fullmatch("[0-9a-f]{64}", expected) else None)
    kind = inputs.get("evidence_kind") if type(inputs) is dict else None
    if kind not in ("synthetic_fixture", "unverified_capture"):
        kind = None
    try:
        result = evaluate(inputs, **byte_inputs)
    except AcceptanceError as error:
        status, returncode, result = "FAIL", 2, {"error_code": error.code}
    else:
        status, returncode = "PASS", 0
    return {"format": "nagi-linux-boot-offline-verification", "format_version": 3,
            "receipt_role": "OFFLINE_VALIDATION_ONLY",
            "evidence_kind": kind,
            "status": status, "returncode": returncode,
            "returncode_scope": "offline_validator_only",
            "inputs": identities, "caller_hash_references": references, "result": result,
            "validation_scope": "offline_supplied_bytes_only",
            "checks_completed": (["input_schema", "supplied_byte_hashes", "static_ELF",
                                  "single_x86_prepared_newc_archive", "init_byte_identity",
                                  "serial_syntax"]
                                 if returncode == 0 else []),
            "failure_detail_limit": "Failing receipts do not certify partial completion before the error.",
            "guest_acceptance": "NOT_VERIFIED", "run_binding_verified": False,
            "kernel_built": "NOT_VERIFIED", "QEMU_started": "NOT_VERIFIED",
            "actual_pid1_verified": False, "minimal_service_ready_verified": False,
            "proof_limit": "Caller references and serial text do not authenticate source, a run, PID1 or service readiness."}


def verify_offline_receipt(receipt: dict, *, init: bytes, initramfs: bytes, serial: bytes) -> dict:
    """Recompute a serialized offline receipt from all three supplied inputs.

    This checks internal consistency only. It does not authenticate whoever made
    the receipt, any artifact source, or the capture/run that produced serial.
    """
    if (type(receipt) is not dict
            or receipt.get("format") != "nagi-linux-boot-offline-verification"
            or receipt.get("format_version") != 3
            or receipt.get("receipt_role") != "OFFLINE_VALIDATION_ONLY"):
        raise AcceptanceError("OFFLINE_RECEIPT_SCHEMA")
    result = receipt.get("result")
    references = receipt.get("caller_hash_references")
    if type(result) is not dict or type(references) is not dict:
        raise AcceptanceError("OFFLINE_RECEIPT_INTEGRITY")
    expected = {"schema": "nagi-linux-boot-evidence-input-v1",
                "evidence_kind": receipt.get("evidence_kind"),
                "init_sha256": references.get("init_sha256"),
                "initramfs_sha256": references.get("initramfs_sha256"),
                "serial_sha256": references.get("serial_sha256")}
    recomputed = verification_receipt(expected, init=init, initramfs=initramfs, serial=serial)
    if recomputed != receipt:
        raise AcceptanceError("OFFLINE_RECEIPT_INTEGRITY")
    return {"receipt_valid_offline_only": True, "status": recomputed["status"],
            "guest_acceptance": "NOT_VERIFIED", "run_binding_verified": False}


def require_runtime_acceptance(receipt: dict, *, init: bytes, initramfs: bytes,
                               serial: bytes) -> None:
    """Fail closed: no runtime acceptance receipt contract is defined here.

    In particular, an internally valid offline PASS cannot become guest READY.
    A future runtime record needs a separately approved schema and verifier.
    """
    if type(receipt) is dict and receipt.get("format") == "nagi-linux-boot-offline-verification":
        verified = verify_offline_receipt(receipt, init=init, initramfs=initramfs, serial=serial)
        if verified["status"] != "PASS":
            raise AcceptanceError("OFFLINE_VALIDATION_FAILED")
        raise AcceptanceError("OFFLINE_RECEIPT_NOT_RUNTIME_EVIDENCE")
    raise AcceptanceError("RUNTIME_RECEIPT_SCHEMA_UNDEFINED")
