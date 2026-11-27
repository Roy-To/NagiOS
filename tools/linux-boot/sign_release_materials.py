#!/usr/bin/env python3
"""Sign reviewed release preimages with the owner's local Ed25519 key."""

import argparse
import json
import os
from pathlib import Path
import re
import stat

from release_signing import (
    SigningError, derive_public, open_private, passphrase, private_location,
    public_key, run, sha256, sign_material,
)


def verified_bytes(path: Path, expected: str, private: os.stat_result) -> bytes:
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(fd, "rb") as stream:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or os.path.samestat(private, info):
            raise SigningError("Reviewed inputs must be regular files separate from the private key.")
        payload = stream.read()
    if not payload or sha256(payload) != expected:
        raise SigningError("A reviewed input is empty or does not match its SHA-256.")
    return payload


def materials(manifest: Path, expected: str, private: os.stat_result) -> list[tuple[str, bytes]]:
    data = json.loads(verified_bytes(manifest, expected, private))
    if (
        not isinstance(data, dict)
        or data.keys() != {"schema_version", "materials"}
        or type(data["schema_version"]) is not int
        or data["schema_version"] != 1
        or not isinstance(data["materials"], list)
        or not data["materials"]
    ):
        raise SigningError("Invalid release signing manifest.")
    entries = {}
    for entry in data["materials"]:
        if not isinstance(entry, dict) or entry.keys() != {"file", "sha256"}:
            raise SigningError("Invalid signing material entry.")
        name = entry["file"]
        if not isinstance(name, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", name):
            raise SigningError("Each material must have a simple filename.")
        output = f"{name}.sig"
        if output.casefold() in entries:
            raise SigningError("Duplicate signature output.")
        entries[output.casefold()] = (output, verified_bytes(manifest.parent / name, entry["sha256"], private))
    return list(entries.values())


def sign(args) -> None:
    with private_location(args.private_key) as (directory, name):
        fd = open_private(directory, name)
        try:
            private = os.fstat(fd)
            key = public_key(verified_bytes(args.public_key, args.public_key_sha256, private))
            inputs = materials(args.manifest, args.manifest_sha256, private)
            secret = passphrase()
            if derive_public(fd, secret) != key:
                raise SigningError("The private key does not match the approved public key.")
            signatures = [(name, sign_material(fd, secret, key, payload)) for name, payload in inputs]
        finally:
            os.close(fd)
    args.output_directory.mkdir(mode=0o700)
    for name, signature in signatures:
        with (args.output_directory / name).open("xb") as output:
            output.write(signature)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("private_key", type=Path)
    parser.add_argument("public_key", type=Path)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("output_directory", type=Path)
    parser.add_argument("--public-key-sha256", required=True)
    parser.add_argument("--manifest-sha256", required=True)
    sign(parser.parse_args())


if __name__ == "__main__":
    run(main)
