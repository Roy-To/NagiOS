#!/usr/bin/env python3
"""Assemble and verify a System raw image and an unsigned boot metadata candidate.

The caller must supply a stable staging directory populated from authenticated
release inputs. This tool handles filesystem bytes and verity geometry. The
release signer and trusted boot consumer must authenticate the metadata under
the approved release contract.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import secrets
import subprocess
from pathlib import Path

from system_verity import (
    DATA_BLOCK_SIZE,
    HASH_BLOCK_SIZE,
    HASHES_PER_BLOCK,
    SHA256_PATTERN,
    VerityError,
    _expected_tree_blocks,
    _hash_block,
)


IMAGE_NAME = "system.raw"
METADATA_NAME = "system-boot-metadata.unsigned.json"
MIN_DATA_BLOCKS = 4096  # 16 MiB for the ext4 filesystem.
MAX_DATA_BLOCKS = (1 << 32) - 1  # ext4 filesystem with 32-bit block addressing.


def read_exact(stream, offset: int, size: int) -> bytes:
    stream.seek(offset)
    data = stream.read(size)
    if len(data) != size:
        raise VerityError("truncated System image")
    return data


def check_geometry(blocks: int) -> None:
    if type(blocks) is not int or not MIN_DATA_BLOCKS <= blocks <= MAX_DATA_BLOCKS:
        raise VerityError("System ext4 block count is outside the supported range")


def process_tree(stream, data_blocks: int, salt: bytes, *, generate: bool) -> tuple[int, str]:
    """Process lower levels first at their final offsets in the root-first tree."""
    tree_blocks = _expected_tree_blocks(data_blocks)
    input_start = 0
    input_blocks = data_blocks
    output_end = data_blocks + tree_blocks
    while True:
        output_blocks = (input_blocks + HASHES_PER_BLOCK - 1) // HASHES_PER_BLOCK
        output_start = output_end - output_blocks
        for index in range(output_blocks):
            first = index * HASHES_PER_BLOCK
            count = min(HASHES_PER_BLOCK, input_blocks - first)
            source = read_exact(
                stream, (input_start + first) * DATA_BLOCK_SIZE, count * DATA_BLOCK_SIZE
            )
            hashes = b"".join(
                _hash_block(source[offset : offset + DATA_BLOCK_SIZE], salt)
                for offset in range(0, len(source), DATA_BLOCK_SIZE)
            )
            block = hashes.ljust(HASH_BLOCK_SIZE, b"\0")
            offset = (output_start + index) * HASH_BLOCK_SIZE
            if generate:
                stream.seek(offset)
                stream.write(block)
            elif read_exact(stream, offset, HASH_BLOCK_SIZE) != block:
                raise VerityError(f"System verity tree mismatch at block {output_start + index}")

        if output_blocks == 1:
            return tree_blocks, _hash_block(block, salt).hex()
        input_start, input_blocks, output_end = output_start, output_blocks, output_start


def describe_image(stream, data_blocks: int, salt: bytes, *, generate: bool) -> dict:
    check_geometry(data_blocks)
    data_bytes = data_blocks * DATA_BLOCK_SIZE
    expected_bytes = data_bytes + _expected_tree_blocks(data_blocks) * HASH_BLOCK_SIZE
    if os.fstat(stream.fileno()).st_size != (data_bytes if generate else expected_bytes):
        raise VerityError("System raw length does not match its verity geometry")

    tree_blocks, root_digest = process_tree(stream, data_blocks, salt, generate=generate)
    data_digest = hashlib.sha256()
    for offset in range(0, data_bytes, 1024 * 1024):
        data_digest.update(read_exact(stream, offset, min(data_bytes - offset, 1024 * 1024)))
    image_digest = data_digest.copy()
    stream.seek(data_bytes)
    for chunk in iter(lambda: stream.read(1024 * 1024), b""):
        image_digest.update(chunk)

    return {
        "schema": "nagi-system-verity-unsigned-v1",
        "system": {
            "size_bytes": expected_bytes,
            "sha256": image_digest.hexdigest(),
            "data_sha256": data_digest.hexdigest(),
        },
        "verity": {
            "format_version": 1,
            "algorithm": "sha256",
            "data_block_size": DATA_BLOCK_SIZE,
            "hash_block_size": HASH_BLOCK_SIZE,
            "num_data_blocks": data_blocks,
            "hash_start_block": data_blocks,
            "tree_blocks": tree_blocks,
            "root_digest": root_digest,
            "salt": salt.hex(),
        },
    }


def encode_metadata(record: dict) -> bytes:
    return (json.dumps(record, sort_keys=True, indent=2) + "\n").encode()


def verify_image(image: Path, metadata: bytes) -> None:
    try:
        verity = json.loads(metadata)["verity"]
        blocks = verity["num_data_blocks"]
        salt = verity["salt"]
    except (KeyError, TypeError, ValueError) as exc:
        raise VerityError("invalid unsigned System boot metadata") from exc
    if not isinstance(salt, str) or not SHA256_PATTERN.fullmatch(salt):
        raise VerityError("invalid System verity salt")

    with image.open("rb") as stream:
        actual = describe_image(stream, blocks, bytes.fromhex(salt), generate=False)
    if encode_metadata(actual) != metadata:
        raise VerityError("unsigned boot metadata does not match the verified System image")


def create_image(staging: Path, output: Path, blocks: int, mke2fs: str) -> None:
    check_geometry(blocks)
    staging = staging.resolve(strict=True)
    output = output.parent.resolve(strict=True) / output.name
    if not staging.is_dir() or staging in output.parents or output == staging:
        raise VerityError("staging must be a directory outside the new output directory")

    output.mkdir(mode=0o700)
    image = output / IMAGE_NAME
    metadata_path = output / METADATA_NAME
    try:
        with image.open("xb") as stream:
            stream.truncate(blocks * DATA_BLOCK_SIZE)
        subprocess.run(
            [
                mke2fs, "-q", "-t", "ext4", "-b", str(DATA_BLOCK_SIZE), "-m", "0",
                "-E", "lazy_itable_init=0,lazy_journal_init=0",
                "-d", str(staging), str(image), str(blocks),
            ],
            check=True,
        )
        with image.open("r+b") as stream:
            record = describe_image(stream, blocks, secrets.token_bytes(32), generate=True)
            os.fsync(stream.fileno())
        image.chmod(0o444)
        metadata = encode_metadata(record)
        verify_image(image, metadata)
        fd = os.open(metadata_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o444)
        with os.fdopen(fd, "wb") as stream:
            stream.write(metadata)
            stream.flush()
            os.fsync(stream.fileno())
    except (OSError, VerityError, subprocess.CalledProcessError, KeyboardInterrupt):
        metadata_path.unlink(missing_ok=True)
        image.unlink(missing_ok=True)
        output.rmdir()
        raise


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("create")
    create.add_argument("--verified-staging", type=Path, required=True)
    create.add_argument("--output", type=Path, required=True)
    create.add_argument("--data-blocks", type=int, required=True)
    create.add_argument("--mke2fs", default="mke2fs")
    verify = commands.add_parser("verify")
    verify.add_argument("--output", type=Path, required=True)
    try:
        args = parser.parse_args()
        if args.command == "create":
            create_image(args.verified_staging, args.output, args.data_blocks, args.mke2fs)
        else:
            verify_image(args.output / IMAGE_NAME, (args.output / METADATA_NAME).read_bytes())
    except (OSError, VerityError, subprocess.CalledProcessError) as exc:
        parser.exit(1, f"{exc}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
