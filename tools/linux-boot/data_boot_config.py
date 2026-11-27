"""Create a development-only signed Data selection entry for initramfs."""

from __future__ import annotations

import hashlib
import os
import re
import shutil
import stat
import subprocess
import tempfile
from pathlib import Path

import boot_acceptance

HEADER = b"nagi-data-selection 1"
SIGNATURE_DOMAIN = b"nagi-data-selection-v1\0"
CONFIG_ENTRY = "nagi-dev-data-selection.signed"
SIGNATURE_BYTES = 64
MAX_CONFIG_TEXT_BYTES = 512
MAX_CONFIG_BYTES = MAX_CONFIG_TEXT_BYTES + SIGNATURE_BYTES
UUID_PATTERN = re.compile(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}")
MODES = {
    "identity",
    "smoke-write",
    "smoke-read",
    "smoke-reject",
    "smoke-provision-identity-directories",
}

# RFC 8032 public test vector. This private seed is public test material and is
# restricted to the Linux Developer Preview tool path; it is not a product key.
DEVELOPMENT_PREVIEW_PUBLIC_KEY = bytes.fromhex(
    "d75a980182b10ab7d54bfed3c964073a"
    "0ee172f3daa62325af021a68f707511a"
)
_DEVELOPMENT_PREVIEW_TEST_SEED = bytes.fromhex(
    "9d61b19deffd5a60ba844af492ec2cc4"
    "4449c5697b326919703bac031cae7f60"
)
_ED25519_PRIVATE_DER_PREFIX = bytes.fromhex("302e020100300506032b657004220420")
_ED25519_PUBLIC_DER_PREFIX = bytes.fromhex("302a300506032b6570032100")


class DataBootConfigError(ValueError):
    pass


def config_text(filesystem_uuid: str, mode: str = "identity") -> bytes:
    if not isinstance(filesystem_uuid, str) or UUID_PATTERN.fullmatch(filesystem_uuid) is None:
        raise DataBootConfigError("Data filesystem UUID must be canonical lowercase UUID")
    if mode not in MODES:
        raise DataBootConfigError("Data selection mode is unsupported")
    return (
        HEADER
        + b"\nmode=" + mode.encode("ascii")
        + b"\nfilesystem-uuid=" + filesystem_uuid.encode("ascii")
        + b"\naccounts=accounts\nsnapshots=snapshots\n"
    )


def sign_config(filesystem_uuid: str, mode: str = "identity") -> bytes:
    text = config_text(filesystem_uuid, mode)
    return _sign_config_text(text)


def _sign_config_text(text: bytes) -> bytes:
    if type(text) is not bytes or not text or len(text) > MAX_CONFIG_TEXT_BYTES:
        raise DataBootConfigError("Data selection config text is invalid or too large")
    openssl = shutil.which("openssl")
    if openssl is None:
        raise DataBootConfigError("OpenSSL with Ed25519 support is required for dev signing")
    with tempfile.TemporaryDirectory(prefix="nagi-data-selection-") as temporary:
        temporary_path = Path(temporary)
        key_path = temporary_path / "dev-preview-test-key.der"
        message_path = temporary_path / "signed-message.bin"
        signature_path = temporary_path / "signature.bin"
        key_path.write_bytes(_ED25519_PRIVATE_DER_PREFIX + _DEVELOPMENT_PREVIEW_TEST_SEED)
        key_path.chmod(0o600)
        message_path.write_bytes(SIGNATURE_DOMAIN + text)
        try:
            subprocess.run(
                [openssl, "pkeyutl", "-sign", "-inkey", str(key_path), "-keyform", "DER",
                 "-rawin", "-in", str(message_path), "-out", str(signature_path)],
                check=True,
                capture_output=True,
                timeout=20,
            )
        except (OSError, subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
            raise DataBootConfigError("OpenSSL could not sign the dev Data selection config") from error
        signature = signature_path.read_bytes()
    if len(signature) != SIGNATURE_BYTES:
        raise DataBootConfigError("OpenSSL returned an invalid Ed25519 signature size")
    return text + signature


def verify_config_signature(config: bytes) -> bool:
    if type(config) is not bytes or not SIGNATURE_BYTES < len(config) <= MAX_CONFIG_BYTES:
        return False
    text, signature = config[:-SIGNATURE_BYTES], config[-SIGNATURE_BYTES:]
    openssl = shutil.which("openssl")
    if openssl is None:
        raise DataBootConfigError("OpenSSL with Ed25519 support is required to verify dev config")
    with tempfile.TemporaryDirectory(prefix="nagi-data-selection-verify-") as temporary:
        temporary_path = Path(temporary)
        key_path = temporary_path / "dev-preview-public-key.der"
        message_path = temporary_path / "signed-message.bin"
        signature_path = temporary_path / "signature.bin"
        key_path.write_bytes(_ED25519_PUBLIC_DER_PREFIX + DEVELOPMENT_PREVIEW_PUBLIC_KEY)
        message_path.write_bytes(SIGNATURE_DOMAIN + text)
        signature_path.write_bytes(signature)
        try:
            subprocess.run(
                [openssl, "pkeyutl", "-verify", "-pubin", "-inkey", str(key_path), "-keyform", "DER",
                 "-rawin", "-in", str(message_path), "-sigfile", str(signature_path)],
                check=True,
                capture_output=True,
                timeout=20,
            )
            return True
        except subprocess.CalledProcessError:
            return False
        except (OSError, subprocess.TimeoutExpired) as error:
            raise DataBootConfigError("OpenSSL could not verify the dev Data selection config") from error


def extract_selection_config(initramfs: bytes) -> bytes:
    _, records = boot_acceptance._parse_newc_records(
        initramfs,
        retain_payloads={CONFIG_ENTRY},
    )
    matches = [payload for name, _fields, payload in records if name == CONFIG_ENTRY]
    if len(matches) != 1:
        raise DataBootConfigError("initramfs must contain one signed Data selection entry")
    return matches[0]


def _valid_relative_path(value: bytes) -> bool:
    if not value:
        return False
    for component in value.split(b"/"):
        if component in (b"", b".", b".."):
            return False
        if not all(
            0x30 <= byte <= 0x39
            or 0x41 <= byte <= 0x5A
            or 0x61 <= byte <= 0x7A
            or byte in b"._-"
            for byte in component
        ):
            return False
    return True


def selection_mode_and_uuid(config: bytes) -> tuple[str, str]:
    if not verify_config_signature(config):
        raise DataBootConfigError("Data selection config signature is invalid")
    text = config[:-SIGNATURE_BYTES]
    if not text.endswith(b"\n") or b"\r" in text:
        raise DataBootConfigError("Data selection config text is malformed")
    lines = text[:-1].split(b"\n")
    if (
        len(lines) != 5
        or lines[0] != HEADER
        or not lines[1].startswith(b"mode=")
        or not lines[2].startswith(b"filesystem-uuid=")
        or not lines[3].startswith(b"accounts=")
        or not lines[4].startswith(b"snapshots=")
        or not _valid_relative_path(lines[3][len(b"accounts="):])
        or not _valid_relative_path(lines[4][len(b"snapshots="):])
    ):
        raise DataBootConfigError("Data selection config text is malformed")
    try:
        mode = lines[1][len(b"mode="):].decode("ascii")
    except UnicodeDecodeError:
        raise DataBootConfigError("Data selection config mode is malformed") from None
    if mode not in MODES:
        raise DataBootConfigError("Data selection config text is malformed")
    value = lines[2][len(b"filesystem-uuid="):]
    try:
        filesystem_uuid = value.decode("ascii")
    except UnicodeDecodeError:
        raise DataBootConfigError("Data selection config UUID is malformed") from None
    if UUID_PATTERN.fullmatch(filesystem_uuid) is None:
        raise DataBootConfigError("Data selection config UUID is malformed")
    return mode, filesystem_uuid


def selection_filesystem_uuid(config: bytes) -> str:
    return selection_mode_and_uuid(config)[1]


def _encode_newc_entry(name: str, fields: list[int], payload: bytes) -> bytes:
    name_bytes = name.encode("ascii") + b"\0"
    values = list(fields)
    values[6] = len(payload)
    values[11] = len(name_bytes)
    values[12] = 0
    if any(value < 0 or value > 0xFFFFFFFF for value in values):
        raise DataBootConfigError("newc field exceeds its 32-bit limit")
    result = bytearray(b"070701" + b"".join(f"{value:08x}".encode("ascii") for value in values))
    result.extend(name_bytes)
    result.extend(b"\0" * (-len(result) % 4))
    result.extend(payload)
    result.extend(b"\0" * (-len(result) % 4))
    return bytes(result)


def add_signed_selection_entry(initramfs: bytes, config: bytes) -> bytes:
    if not verify_config_signature(config):
        raise DataBootConfigError("Data selection config signature is invalid")
    try:
        _, records = boot_acceptance._parse_newc_records(initramfs)
    except ValueError as error:
        raise DataBootConfigError(f"base initramfs is not a supported newc archive ({error})") from None
    if any(name == CONFIG_ENTRY for name, _fields, _payload in records):
        raise DataBootConfigError("initramfs already contains a Data selection entry")
    max_inode = max(fields[0] for _name, fields, _payload in records)
    if max_inode == 0xFFFFFFFF:
        raise DataBootConfigError("initramfs has no inode number available for the selection entry")
    entry_fields = [
        max_inode + 1,
        stat.S_IFREG | 0o400,
        0,
        0,
        1,
        0,
        len(config),
        0,
        0,
        0,
        0,
        len(CONFIG_ENTRY) + 1,
        0,
    ]
    result = bytearray()
    inserted = False
    for name, fields, payload in records:
        if name == "TRAILER!!!" and not inserted:
            result.extend(_encode_newc_entry(CONFIG_ENTRY, entry_fields, config))
            inserted = True
        result.extend(_encode_newc_entry(name, fields, payload))
    if not inserted:
        raise DataBootConfigError("initramfs trailer is missing")
    return bytes(result)


def write_configured_initramfs(
    source: Path,
    destination: Path,
    filesystem_uuid: str,
    mode: str = "identity",
) -> tuple[str, str]:
    """Write a run-specific initramfs with one authenticated selection entry."""
    source_bytes = source.read_bytes()
    config = sign_config(filesystem_uuid, mode)
    configured = add_signed_selection_entry(source_bytes, config)
    if destination.exists() or destination.is_symlink():
        if destination.is_file() and not destination.is_symlink() and destination.read_bytes() == configured:
            return hashlib.sha256(configured).hexdigest(), hashlib.sha256(config).hexdigest()
        raise DataBootConfigError("configured initramfs output already exists with different bytes")
    try:
        with destination.open("xb") as stream:
            stream.write(configured)
            stream.flush()
            os.fsync(stream.fileno())
            os.fchmod(stream.fileno(), 0o600)
    except FileExistsError:
        raise DataBootConfigError("configured initramfs output appeared during creation") from None
    return hashlib.sha256(configured).hexdigest(), hashlib.sha256(config).hexdigest()
