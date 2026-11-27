#!/usr/bin/env python3
"""Generate an encrypted Ed25519 key in the release owner's local terminal."""

import argparse
import os
from pathlib import Path

from release_signing import derive_public, openssl, passphrase, private_location, run, sha256


def generate(private: Path, public: Path, secret: bytes) -> bytes:
    with private_location(private) as (directory, name):
        fd = os.open(
            name, os.O_RDWR | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600, dir_fd=directory,
        )
        try:
            openssl(["genpkey", "-algorithm", "ED25519", "-aes-256-cbc"], secret=secret, stdout=fd)
            raw = derive_public(fd, secret)
            os.fsync(fd)
            with public.open("xb") as output:
                output.write(raw)
        except BaseException:
            try:
                current = os.stat(name, dir_fd=directory, follow_symlinks=False)
            except FileNotFoundError:
                current = None
            if current is not None and os.path.samestat(os.fstat(fd), current):
                os.unlink(name, dir_fd=directory)
            raise
        finally:
            os.close(fd)
    return raw


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("private_key", type=Path)
    parser.add_argument("public_key", type=Path)
    args = parser.parse_args()
    raw = generate(args.private_key, args.public_key, passphrase(confirm=True))
    print(f"Ed25519 public key: {raw.hex()}")
    print(f"SHA-256: {sha256(raw)}")


if __name__ == "__main__":
    run(main)
