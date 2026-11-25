#!/usr/bin/env python3
"""Build and run a bounded dm-verity System-image acceptance fixture.

The tool keeps the input ext4 image unchanged, appends a version-1 SHA-256
verity tree to a private copy, and creates a second copy with the ext4
superblock block changed while retaining the original tree. QEMU is never
started by the default command; --start also requires an active L11-3 slot
assignment in the internal board.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shlex
import stat
import subprocess
import sys
import time
from pathlib import Path

DATA_BLOCK_SIZE = 4096
HASH_BLOCK_SIZE = 4096
HASHES_PER_BLOCK = HASH_BLOCK_SIZE // hashlib.sha256().digest_size
MAX_SYSTEM_BYTES = 128 * 1024 * 1024
SYSTEM_DM_NAME = "nagi-system-verity"
SYSTEM_DM_UUID = "4e616769-5373-7973-7465-6d2d76303100"
SYSTEM_SERIAL = "NAGI-SYSTEM-L11-3"
TAMPER_OFFSET = 1024 + 56  # ext4 magic inside data block zero
TAMPER_XOR = 0x01
VERITY_MARKER = re.compile(r"^NAGI_SYSTEM_VERITY_(ACCEPTED|REJECTED)(?: .*)?$")
SHA256_PATTERN = re.compile(r"[0-9a-f]{64}")


class VerityError(ValueError):
    pass


def _regular_file(path: Path, label: str) -> os.stat_result:
    try:
        info = path.lstat()
    except FileNotFoundError:
        raise VerityError(f"missing {label}: {path}") from None
    if stat.S_ISLNK(info.st_mode) or not stat.S_ISREG(info.st_mode):
        raise VerityError(f"{label} must be a non-symlink regular file: {path}")
    return info


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _write_exclusive(path: Path, chunks, mode: int = 0o444) -> tuple[str, int]:
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(path, flags, mode)
    digest = hashlib.sha256()
    total = 0
    try:
        for chunk in chunks:
            view = memoryview(chunk)
            while view:
                count = os.write(fd, view)
                if count <= 0:
                    raise OSError("short write")
                digest.update(view[:count])
                total += count
                view = view[count:]
        os.fsync(fd)
    finally:
        os.close(fd)
    os.chmod(path, mode)
    return digest.hexdigest(), total


def _hash_block(block: bytes, salt: bytes) -> bytes:
    if len(block) != HASH_BLOCK_SIZE:
        raise VerityError("verity hash blocks must be exactly 4096 bytes")
    return hashlib.sha256(salt + block).digest()


def build_hash_tree(data: bytes, salt: bytes) -> tuple[bytes, str]:
    """Return root-first dm-verity v1 SHA-256 tree and trusted root digest."""
    if not data or len(data) % DATA_BLOCK_SIZE:
        raise VerityError("System data must be non-empty and 4096-byte aligned")
    if len(data) > MAX_SYSTEM_BYTES:
        raise VerityError("System image exceeds the 128 MiB development fixture limit")
    if len(salt) != 32:
        raise VerityError("the L11-3 SHA-256 salt must be exactly 32 bytes")

    digests = [
        hashlib.sha256(salt + data[offset : offset + DATA_BLOCK_SIZE]).digest()
        for offset in range(0, len(data), DATA_BLOCK_SIZE)
    ]
    levels: list[list[bytes]] = []
    while True:
        blocks: list[bytes] = []
        for start in range(0, len(digests), HASHES_PER_BLOCK):
            block = b"".join(digests[start : start + HASHES_PER_BLOCK])
            blocks.append(block.ljust(HASH_BLOCK_SIZE, b"\0"))
        levels.append(blocks)
        if len(blocks) == 1:
            root = hashlib.sha256(salt + blocks[0]).hexdigest()
            break
        digests = [_hash_block(block, salt) for block in blocks]

    # dm-verity v1 stores the levels depth-first from the root block down.
    tree = b"".join(block for level in reversed(levels) for block in level)
    return tree, root


def deterministic_salt(data_sha256: str) -> bytes:
    if not SHA256_PATTERN.fullmatch(data_sha256):
        raise VerityError("System source SHA-256 is malformed")
    return hashlib.sha256(b"NagiOS L11-3 dm-verity v1\0" + bytes.fromhex(data_sha256)).digest()


def create_image_pair(source: Path, verified: Path, tampered: Path) -> dict:
    source_info = _regular_file(source, "System source")
    if source_info.st_size == 0 or source_info.st_size % DATA_BLOCK_SIZE:
        raise VerityError("System source size must be non-zero and 4096-byte aligned")
    if source_info.st_size > MAX_SYSTEM_BYTES:
        raise VerityError("System source exceeds the 128 MiB development fixture limit")
    if source_info.st_mode & 0o222:
        raise VerityError("System source must be read-only before it is copied")
    if verified.parent != tampered.parent:
        raise VerityError("verified and tampered images must share one output directory")
    if verified.exists() or tampered.exists():
        raise VerityError("System image output already exists")

    source_sha = _sha256_file(source)
    salt = deterministic_salt(source_sha)
    with source.open("rb") as stream:
        data = stream.read()
    if len(data) != source_info.st_size:
        raise VerityError("System source changed while it was read")
    tree, root_digest = build_hash_tree(data, salt)
    blocks = len(data) // DATA_BLOCK_SIZE

    def original_chunks():
        yield data
        yield tree

    verified_sha, verified_size = _write_exclusive(verified, original_chunks())

    changed = bytearray(data)
    changed[TAMPER_OFFSET] ^= TAMPER_XOR
    tampered_sha, tampered_size = _write_exclusive(tampered, (bytes(changed), tree))
    if verified_size != tampered_size:
        raise VerityError("paired System images have different sizes")

    return {
        "schema": 1,
        "source": {"path": source.name, "size_bytes": len(data), "sha256": source_sha},
        "verity": {
            "format_version": 1,
            "algorithm": "sha256",
            "data_block_size": DATA_BLOCK_SIZE,
            "hash_block_size": HASH_BLOCK_SIZE,
            "num_data_blocks": blocks,
            "hash_start_block": blocks,
            "tree_blocks": len(tree) // HASH_BLOCK_SIZE,
            "salt": salt.hex(),
            "root_digest": root_digest,
        },
        "tamper": {"data_block": 0, "byte_offset": TAMPER_OFFSET, "xor": TAMPER_XOR},
        "verified": {"path": verified.name, "size_bytes": verified_size, "sha256": verified_sha},
        "tampered": {"path": tampered.name, "size_bytes": tampered_size, "sha256": tampered_sha},
    }


def dm_mod_create(image: dict) -> str:
    verity = image["verity"]
    sectors = verity["num_data_blocks"] * (DATA_BLOCK_SIZE // 512)
    target = (
        f"0 {sectors} verity 1 /dev/vda /dev/vda "
        f"{DATA_BLOCK_SIZE} {HASH_BLOCK_SIZE} {verity['num_data_blocks']} "
        f"{verity['hash_start_block']} sha256 {verity['root_digest']} {verity['salt']}"
    )
    return f"{SYSTEM_DM_NAME},{SYSTEM_DM_UUID},,ro,{target}"


def kernel_cmdline(image: dict) -> str:
    return (
        "console=ttyS0,115200n8 rdinit=/init panic=-1 "
        "dm-mod.waitfor=/dev/vda "
        f'dm-mod.create="{dm_mod_create(image)}" '
        "nagi.dev_system_verity=probe"
    )


def parse_cmdline(raw: str) -> list[str]:
    """Parse the small quoted-word subset used by the kernel command line."""
    words: list[str] = []
    current: list[str] = []
    quoted = False
    started = False
    for character in raw:
        if character == '"':
            quoted = not quoted
            started = True
        elif character.isspace() and not quoted:
            if started:
                words.append("".join(current))
                current.clear()
                started = False
        elif character == "\\":
            raise VerityError("backslash escapes are not accepted in the fixed cmdline")
        else:
            current.append(character)
            started = True
    if quoted:
        raise VerityError("kernel command line has an unterminated quote")
    if started:
        words.append("".join(current))
    return words


def parse_config(data: bytes) -> dict[str, str]:
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError as error:
        raise VerityError(f"kernel .config is not UTF-8: {error}") from None
    values: dict[str, str] = {}
    for line in text.splitlines():
        match = re.fullmatch(r"(CONFIG_[A-Z0-9_]+)=(.*)", line.strip())
        if match:
            name, value = match.groups()
            if name in values and values[name] != value:
                raise VerityError(f"kernel .config conflicts for {name}")
            values[name] = value
        elif line.startswith("# CONFIG_") and line.endswith(" is not set"):
            name = line[2:-len(" is not set")]
            values.setdefault(name, "n")
    required = {
        "CONFIG_X86_64": "y",
        "CONFIG_BLK_DEV_INITRD": "y",
        "CONFIG_BINFMT_ELF": "y",
        "CONFIG_PRINTK": "y",
        "CONFIG_SERIAL_8250": "y",
        "CONFIG_SERIAL_8250_CONSOLE": "y",
        "CONFIG_BLOCK": "y",
        "CONFIG_PCI": "y",
        "CONFIG_VIRTIO": "y",
        "CONFIG_VIRTIO_PCI": "y",
        "CONFIG_VIRTIO_BLK": "y",
        "CONFIG_BLK_DEV_DM": "y",
        "CONFIG_DM_INIT": "y",
        "CONFIG_DM_VERITY": "y",
        "CONFIG_CRYPTO_SHA256": "y",
        "CONFIG_EXT4_FS": "y",
        "CONFIG_DEVTMPFS": "y",
        "CONFIG_PROC_FS": "y",
        "CONFIG_SYSFS": "y",
    }
    for name, expected in required.items():
        if values.get(name) != expected:
            raise VerityError(f"kernel .config requires {name}={expected}; found {values.get(name, 'missing')}")
    if values.get("CONFIG_CMDLINE_OVERRIDE") == "y":
        raise VerityError("kernel .config must not override the QEMU command line")
    if values.get("CONFIG_INITRAMFS_SOURCE") not in (None, '""'):
        raise VerityError("kernel .config must use the external initramfs")
    return values


def parse_probe_log(serial: str, expected: str, root_digest: str) -> dict:
    markers = [line.strip() for line in serial.splitlines() if VERITY_MARKER.fullmatch(line.strip())]
    if len(markers) != 1:
        raise VerityError(f"expected exactly one System verity result marker, found {len(markers)}")
    marker = markers[0]
    if expected == "accepted":
        if not marker.startswith("NAGI_SYSTEM_VERITY_ACCEPTED "):
            raise VerityError("clean System did not emit the accepted marker")
        required = (
            f"root={root_digest}",
            "filesystem=ext4",
            "readonly=true",
            "fixture=true",
        )
        if not all(field in marker for field in required):
            raise VerityError("clean System accepted marker is missing verified fields")
        if "NAGI_SYSTEM_VERITY_REJECTED" in serial or "state=ready" in serial.lower():
            raise VerityError("clean System log contains a rejection or READY marker")
        if "device-mapper: verity:" in serial:
            raise VerityError("clean System log unexpectedly reports verity corruption")
    elif expected == "rejected":
        if not marker.startswith("NAGI_SYSTEM_VERITY_REJECTED "):
            raise VerityError("tampered System did not emit the rejected marker")
        if "stage=critical_block_read" not in marker or "errno=5" not in marker:
            raise VerityError("tampered System rejection was not the expected verified-block EIO")
        if not re.search(r"device-mapper: verity:.*data block 0 is corrupted", serial):
            raise VerityError("tampered System log lacks the kernel dm-verity corruption report")
        if "NAGI_SYSTEM_VERITY_ACCEPTED" in serial or "state=ready" in serial.lower():
            raise VerityError("tampered System reached an accepted or READY state")
    else:
        raise VerityError("expected result must be accepted or rejected")
    return {"expected": expected, "marker": marker}


def _newc_entry(name: str, mode: int, ino: int, *, data: bytes = b"", rdevmajor: int = 0,
                rdevminor: int = 0) -> bytes:
    encoded_name = name.encode("ascii") + b"\0"
    fields = (ino, mode, 0, 0, 1 if not stat.S_ISDIR(mode) else 2, 0, len(data), 0, 0,
              rdevmajor, rdevminor, len(encoded_name), 0)
    record = b"070701" + b"".join(f"{value:08x}".encode("ascii") for value in fields) + encoded_name
    record += b"\0" * (-len(record) % 4)
    record += data
    record += b"\0" * (-len(record) % 4)
    return record


def make_initramfs(init_elf: Path, output: Path, image: dict) -> dict:
    info = _regular_file(init_elf, "PID 1 ELF")
    if info.st_size > 128 * 1024 * 1024:
        raise VerityError("PID 1 ELF exceeds the 128 MiB acceptance fixture limit")
    if output.exists():
        raise VerityError("initramfs output already exists")
    elf = init_elf.read_bytes()
    if len(elf) != info.st_size or elf[:6] != b"\x7fELF\x02\x01" or elf[18:20] != b"\x3e\0":
        raise VerityError("PID 1 input must be a stable little-endian x86_64 ELF")
    verity = image.get("verity")
    if not isinstance(verity, dict):
        raise VerityError("System image receipt has no verity parameters")
    root_hash = str(verity.get("root_digest", ""))
    blocks = str(verity.get("num_data_blocks", ""))
    salt = str(verity.get("salt", ""))
    if not SHA256_PATTERN.fullmatch(root_hash):
        raise VerityError("PID 1 build root digest does not match the image receipt")
    if not blocks.isascii() or not blocks.isdecimal() or int(blocks) <= 0:
        raise VerityError("PID 1 build data-block count is malformed")
    if not re.fullmatch(r"[0-9a-f]{64}", salt):
        raise VerityError("PID 1 build salt is malformed")
    if root_hash.encode() not in elf or blocks.encode() not in elf or salt.encode() not in elf:
        raise VerityError("PID 1 ELF does not contain the exact dm-verity parameters")

    entries = [
        ("dev", stat.S_IFDIR | 0o755, b"", 0, 0),
        ("dev/console", stat.S_IFCHR | 0o600, b"", 5, 1),
        ("dev/null", stat.S_IFCHR | 0o666, b"", 1, 3),
        ("dev/ttyS0", stat.S_IFCHR | 0o600, b"", 4, 64),
        ("proc", stat.S_IFDIR | 0o555, b"", 0, 0),
        ("sys", stat.S_IFDIR | 0o555, b"", 0, 0),
        ("run", stat.S_IFDIR | 0o755, b"", 0, 0),
        ("run/nagi", stat.S_IFDIR | 0o755, b"", 0, 0),
        ("run/nagi/dev-system", stat.S_IFDIR | 0o755, b"", 0, 0),
        ("tmp", stat.S_IFDIR | 0o1777, b"", 0, 0),
        ("init", stat.S_IFREG | 0o755, elf, 0, 0),
    ]
    archive = bytearray()
    for ino, (name, mode, data, major, minor) in enumerate(entries, start=1):
        archive.extend(_newc_entry(name, mode, ino, data=data, rdevmajor=major, rdevminor=minor))
    archive.extend(_newc_entry("TRAILER!!!", 0, len(entries) + 1))
    archive.extend(b"\0" * (-len(archive) % 4))
    digest, size = _write_exclusive(output, (bytes(archive),), mode=0o444)
    return {"path": output.name, "size_bytes": size, "sha256": digest,
            "init_sha256": hashlib.sha256(elf).hexdigest(), "root_digest": root_hash}


def qemu_argv(qemu: str, kernel: Path, initramfs: Path, system: Path, serial: Path,
              image: dict) -> list[str]:
    return [
        qemu,
        "-name", "nagi-l11-3-system-verity",
        "-machine", "q35,i8042=off",
        "-accel", "tcg,thread=multi",
        "-cpu", "max",
        "-smp", "4",
        "-m", "8192",
        "-kernel", str(kernel),
        "-initrd", str(initramfs),
        "-append", kernel_cmdline(image),
        "-drive", f"if=none,file={system},format=raw,readonly=on,id=systemdisk",
        "-device", f"virtio-blk-pci,drive=systemdisk,serial={SYSTEM_SERIAL}",
        "-display", "none",
        "-vga", "none",
        "-nic", "none",
        "-monitor", "none",
        "-serial", f"file:{serial}",
        "-no-reboot",
    ]


def _hash_named_file(directory: Path, name: str, expected_hash: str) -> Path:
    if not isinstance(name, str) or Path(name).name != name or name in ("", ".", ".."):
        raise VerityError("bundle paths must be plain file names")
    path = directory / name
    _regular_file(path, name)
    if not SHA256_PATTERN.fullmatch(expected_hash) or _sha256_file(path) != expected_hash:
        raise VerityError(f"{name} does not match its recorded SHA-256")
    return path


def load_run_bundle(directory: Path) -> tuple[dict, dict[str, Path]]:
    if directory.is_symlink() or not directory.is_dir():
        raise VerityError("bundle directory must be a non-symlink directory")
    manifest_path = directory / "bundle.json"
    manifest_info = _regular_file(manifest_path, "bundle.json")
    if manifest_info.st_size > 32 * 1024:
        raise VerityError("bundle.json exceeds 32 KiB")
    try:
        bundle = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise VerityError("bundle.json is not valid UTF-8 JSON") from None
    if not isinstance(bundle, dict) or bundle.get("schema") != 1:
        raise VerityError("bundle.json schema must be 1")
    paths: dict[str, Path] = {}
    for key in ("kernel", "config", "initramfs", "source", "verified", "tampered"):
        item = bundle.get(key)
        if not isinstance(item, dict):
            raise VerityError(f"bundle.json {key} record is missing")
        paths[key] = _hash_named_file(directory, item.get("path"), item.get("sha256", ""))
    parse_config(paths["config"].read_bytes())
    image = bundle.get("image")
    if not isinstance(image, dict) or image.get("schema") != 1:
        raise VerityError("bundle.json image record is malformed")
    v = image.get("verity")
    if not isinstance(v, dict) or not SHA256_PATTERN.fullmatch(str(v.get("root_digest", ""))):
        raise VerityError("bundle.json verity record is malformed")
    if not re.fullmatch(r"[0-9a-f]{64}", str(v.get("salt", ""))):
        raise VerityError("bundle.json verity salt is malformed")
    if type(v.get("num_data_blocks")) is not int or v["num_data_blocks"] <= 0:
        raise VerityError("bundle.json data-block count is malformed")
    if v["data_block_size"] != DATA_BLOCK_SIZE or v["hash_block_size"] != HASH_BLOCK_SIZE:
        raise VerityError("bundle.json block sizes do not match this runner")
    if bundle["verified"]["size_bytes"] != bundle["tampered"]["size_bytes"]:
        raise VerityError("paired System images do not have equal lengths")
    if image["source"]["sha256"] != bundle["source"]["sha256"]:
        raise VerityError("private System source hash disagrees with image.json")
    if image["verified"]["sha256"] != bundle["verified"]["sha256"] or image["tampered"]["sha256"] != bundle["tampered"]["sha256"]:
        raise VerityError("image and bundle receipts disagree")
    tamper = image.get("tamper")
    if not isinstance(tamper, dict) or tamper.get("byte_offset") != TAMPER_OFFSET or tamper.get("xor") != TAMPER_XOR:
        raise VerityError("bundle tamper record differs from the fixed ext4 critical byte")
    verify_bundle_image_pair(bundle, paths)
    return bundle, paths


def current_board_assignment(lines: list[str], slot: int) -> str:
    slot_label = f"枠{slot}"
    summary = next((line for line in lines[:30] if line.startswith("QEMU ")), "")
    summary_slot = next((part for part in summary.split("｜") if slot_label in part), None)
    if summary_slot is None or "L11-3" not in summary_slot or "空き" in summary_slot:
        raise VerityError(f"board has no current L11-3 assignment for QEMU slot {slot}")

    action = re.compile(r"(?:割当|割り当て|assigned|返却)", re.IGNORECASE)
    events = [
        line for line in lines[:30]
        if "L11-3" in line and slot_label in line and action.search(line)
    ]
    if not events:
        raise VerityError(f"board has no current L11-3 assignment for QEMU slot {slot}")
    assignment = events[0]
    if any(word in assignment for word in ("返却", "返却済", "空き")):
        raise VerityError("board's L11-3 slot line is already returned or free")
    if not re.search(r"(?:割当|割り当て|assigned)", assignment, re.IGNORECASE):
        raise VerityError(f"board has no current L11-3 assignment for QEMU slot {slot}")
    return assignment


def run_pair(bundle_dir: Path, board: Path, slot: int, qemu_name: str, timeout_seconds: int) -> dict:
    bundle, paths = load_run_bundle(bundle_dir)
    board_info = _regular_file(board, "internal board")
    if board_info.st_mode & 0o222 == 0:
        raise VerityError("internal board is unexpectedly read-only")
    lines = board.read_text(encoding="utf-8").splitlines()[:30]
    assignment = current_board_assignment(lines, slot)
    qemu = shutil_which(qemu_name)
    if qemu is None:
        raise VerityError(f"QEMU executable not found: {qemu_name}")
    run_dir = bundle_dir / "qemu-run"
    try:
        run_dir.mkdir(mode=0o700)
    except FileExistsError:
        raise VerityError("qemu-run already exists; refusing to reuse an old run") from None
    report: dict = {
        "schema": 1,
        "board_assignment": assignment,
        "slot": slot,
        "kernel_sha256": bundle["kernel"]["sha256"],
        "config_sha256": bundle["config"]["sha256"],
        "initramfs_sha256": bundle["initramfs"]["sha256"],
        "root_digest": bundle["image"]["verity"]["root_digest"],
        "runs": [],
    }
    for key, expected in (("verified", "accepted"), ("tampered", "rejected")):
        serial_path = run_dir / f"{key}.serial"
        stderr_path = run_dir / f"{key}.stderr"
        argv = qemu_argv(qemu, paths["kernel"], paths["initramfs"], paths[key], serial_path, bundle["image"])
        started = time.monotonic()
        try:
            completed = subprocess.run(argv, capture_output=True, text=True, timeout=timeout_seconds,
                                       check=False, start_new_session=True)
            timed_out = False
            exit_code = completed.returncode
            stderr_path.write_text(completed.stderr, encoding="utf-8")
        except subprocess.TimeoutExpired as error:
            timed_out = True
            exit_code = None
            stderr_path.write_text((error.stderr or b"").decode("utf-8", "replace")
                                   if isinstance(error.stderr, bytes) else (error.stderr or ""), encoding="utf-8")
        serial = serial_path.read_text(encoding="utf-8", errors="replace") if serial_path.exists() else ""
        row = {
            "case": key,
            "expected": expected,
            "exit_code": exit_code,
            "timed_out": timed_out,
            "duration_seconds": round(time.monotonic() - started, 3),
            "serial_sha256": hashlib.sha256(serial.encode()).hexdigest(),
            "serial_path": serial_path.name,
            "stderr_path": stderr_path.name,
            "argv": argv,
        }
        report["runs"].append(row)
        if timed_out or exit_code != 0:
            row["result"] = "FAIL"
            report["result"] = "FAIL"
            break
        try:
            row["guest"] = parse_probe_log(serial, expected, bundle["image"]["verity"]["root_digest"])
            row["result"] = "PASS"
        except VerityError as error:
            row["result"] = "FAIL"
            row["guest_error"] = str(error)
            report["result"] = "FAIL"
            break
    else:
        report["result"] = "PASS"
    report_path = run_dir / "receipt.json"
    report_path.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    report_path.chmod(0o444)
    return report


def shutil_which(name: str) -> str | None:
    import shutil
    return shutil.which(name)


def prepare_images(source: Path, output_dir: Path) -> dict:
    source_info = _regular_file(source, "System source")
    if source_info.st_mode & 0o222:
        raise VerityError("System source must be read-only before it is copied")
    if output_dir.exists() or output_dir.is_symlink():
        raise VerityError("image output directory already exists")
    output_dir.mkdir(mode=0o700, parents=True)
    source_copy = output_dir / "system.source.raw"
    source_hash, source_size = _write_exclusive(source_copy, _iter_file(source), mode=0o444)
    if source_size != source.stat().st_size or source_hash != _sha256_file(source):
        raise VerityError("private System source copy differs from the read-only input")
    verified = output_dir / "system.verity.raw"
    tampered = output_dir / "system.verity-tampered.raw"
    image = create_image_pair(source_copy, verified, tampered)
    manifest_path = output_dir / "image.json"
    manifest_path.write_text(json.dumps(image, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    manifest_path.chmod(0o444)
    return image


def _iter_file(path: Path):
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            yield chunk


def create_run_bundle(directory: Path, kernel: Path, config: Path, initramfs: Path) -> dict:
    image_path = directory / "image.json"
    _regular_file(image_path, "image.json")
    image = json.loads(image_path.read_text(encoding="utf-8"))
    root_digest = image["verity"]["root_digest"]
    data_blocks = str(image["verity"]["num_data_blocks"])
    salt = image["verity"]["salt"]
    if not SHA256_PATTERN.fullmatch(root_digest) or not re.fullmatch(r"[0-9a-f]{64}", salt):
        raise VerityError("image.json has malformed dm-verity parameters")
    _regular_file(initramfs, "initramfs input")
    cpio_data = initramfs.read_bytes()
    if root_digest.encode() not in cpio_data or data_blocks.encode() not in cpio_data or salt.encode() not in cpio_data:
        raise VerityError("initramfs does not contain the exact image dm-verity identity")
    _regular_file(config, "kernel config")
    parse_config(config.read_bytes())
    records = {}
    for key, source_path, name in (
        ("kernel", kernel, "kernel.bzImage"),
        ("config", config, "kernel.config"),
        ("initramfs", initramfs, "initramfs.cpio"),
    ):
        _regular_file(source_path, f"{key} input")
        destination = directory / name
        sha, size = _write_exclusive(destination, _iter_file(source_path), mode=0o444)
        records[key] = {"path": destination.name, "size_bytes": size, "sha256": sha}
    bundle = {
        "schema": 1,
        **records,
        "source": image["source"],
        "verified": image["verified"],
        "tampered": image["tampered"],
        "image": image,
    }
    payload = (json.dumps(bundle, indent=2, sort_keys=True) + "\n").encode()
    _write_exclusive(directory / "bundle.json", (payload,), mode=0o444)
    return bundle


def verify_bundle_image_pair(bundle: dict, paths: dict[str, Path]) -> None:
    image = bundle["image"]
    source_info = image["source"]
    source = paths["source"].read_bytes()
    if len(source) != source_info["size_bytes"] or hashlib.sha256(source).hexdigest() != source_info["sha256"]:
        raise VerityError("private System source does not match image.json")
    verity = image["verity"]
    salt = bytes.fromhex(verity["salt"])
    tree, root_digest = build_hash_tree(source, salt)
    if root_digest != verity["root_digest"] or len(tree) // HASH_BLOCK_SIZE != verity["tree_blocks"]:
        raise VerityError("image.json root digest or tree size does not match its System source")
    expected_good = source + tree
    changed = bytearray(source)
    changed[image["tamper"]["byte_offset"]] ^= image["tamper"]["xor"]
    expected_bad = bytes(changed) + tree
    if paths["verified"].read_bytes() != expected_good:
        raise VerityError("verified System image differs from source plus its verity tree")
    if paths["tampered"].read_bytes() != expected_bad:
        raise VerityError("tampered System image differs outside its one critical byte or verity tree")
    for key in ("source", "verified", "tampered"):
        if paths[key].stat().st_mode & 0o222:
            raise VerityError(f"{key} System image is writable on the host")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    prepare = commands.add_parser("prepare-images", help="create a private verified/tampered System pair")
    prepare.add_argument("--source", required=True, type=Path)
    prepare.add_argument("--output-dir", required=True, type=Path)
    command = commands.add_parser("print-cmdline", help="print the exact dm-init command line from image.json")
    command.add_argument("image_json", type=Path)
    cpio_command = commands.add_parser("make-initramfs", help="pack the PID 1 ELF with fixed verity parameters")
    cpio_command.add_argument("--elf", required=True, type=Path)
    cpio_command.add_argument("--output", required=True, type=Path)
    cpio_command.add_argument("--image-json", required=True, type=Path)
    bundle_command = commands.add_parser("make-bundle", help="copy fixed inputs and bind their hashes")
    bundle_command.add_argument("--directory", required=True, type=Path)
    bundle_command.add_argument("--kernel", required=True, type=Path)
    bundle_command.add_argument("--config", required=True, type=Path)
    bundle_command.add_argument("--initramfs", required=True, type=Path)
    run = commands.add_parser("run", help="start the two QEMU controls only after board slot authorization")
    run.add_argument("bundle", type=Path)
    run.add_argument("--board", type=Path, required=True)
    run.add_argument("--slot", type=int, choices=(1, 2), required=True)
    run.add_argument("--qemu", default="qemu-system-x86_64")
    run.add_argument("--timeout-seconds", type=int, default=180)
    run.add_argument("--start", action="store_true", help="start QEMU; omission is non-start preflight only")
    args = parser.parse_args(argv)
    try:
        if args.command == "prepare-images":
            image = prepare_images(args.source, args.output_dir)
            print(json.dumps(image, sort_keys=True))
            return 0
        if args.command == "print-cmdline":
            image = json.loads(args.image_json.read_text(encoding="utf-8"))
            print(kernel_cmdline(image))
            return 0
        if args.command == "make-initramfs":
            image = json.loads(args.image_json.read_text(encoding="utf-8"))
            print(json.dumps(make_initramfs(args.elf, args.output, image), sort_keys=True))
            return 0
        if args.command == "make-bundle":
            bundle = create_run_bundle(args.directory, args.kernel, args.config, args.initramfs)
            print(json.dumps(bundle, sort_keys=True))
            return 0
        if args.command == "run":
            bundle, _ = load_run_bundle(args.bundle)
            if not args.start:
                print(json.dumps({"result": "PREPARED_NO_START", "slot": args.slot,
                                  "kernel_sha256": bundle["kernel"]["sha256"],
                                  "initramfs_sha256": bundle["initramfs"]["sha256"],
                                  "root_digest": bundle["image"]["verity"]["root_digest"]}, sort_keys=True))
                return 0
            if args.timeout_seconds < 1 or args.timeout_seconds > 900:
                raise VerityError("QEMU timeout must be between 1 and 900 seconds")
            report = run_pair(args.bundle, args.board, args.slot, args.qemu, args.timeout_seconds)
            print(json.dumps(report, sort_keys=True))
            return 0 if report.get("result") == "PASS" else 1
    except (OSError, VerityError, KeyError, TypeError, json.JSONDecodeError) as error:
        print(f"system-verity: {error}", file=sys.stderr)
        return 2
    return 2


if __name__ == "__main__":
    sys.exit(main())
