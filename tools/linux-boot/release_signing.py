#!/usr/bin/env python3
"""Shared local-only Ed25519 signing mechanics."""

from __future__ import annotations

from contextlib import contextmanager
import getpass
import hashlib
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import tempfile
import warnings


SPKI_PREFIX = bytes.fromhex("302a300506032b6570032100")
DEV_PUBLIC_KEY = bytes.fromhex(
    "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
)


class SigningError(Exception):
    pass


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def public_key(raw: bytes) -> bytes:
    if len(raw) != 32 or raw in (bytes(32), DEV_PUBLIC_KEY):
        raise SigningError("Expected a 32-byte release public key, distinct from the development key.")
    return raw


def check_private_location(parent: Path) -> None:
    codex_home = Path(os.environ.get("CODEX_HOME", Path.home() / ".codex")).resolve()
    if parent == codex_home or codex_home in parent.parents:
        raise SigningError("The private key must stay outside Codex storage.")
    for candidate in (parent, *parent.parents):
        if candidate.name.casefold() in (".git", ".codex") or os.path.lexists(candidate / ".git"):
            raise SigningError("The private key must stay outside repositories and worktrees.")
        git_storage = (candidate / "HEAD").exists() and (
            (candidate / "objects").is_dir() or (candidate / "commondir").is_file()
        )
        if git_storage:
            raise SigningError("The private key must stay outside Git administrative directories.")


@contextmanager
def private_location(path: Path):
    if not path.is_absolute() or ".." in path.parts or path.name in ("", ".", ".."):
        raise SigningError("Use an absolute private key path with an existing parent directory.")
    check_private_location(path.parent)
    parent = path.parent.resolve(strict=True)
    check_private_location(parent)
    flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
    fd = os.open(parent.anchor, flags)
    try:
        for part in parent.parts[1:]:
            child = os.open(part, flags, dir_fd=fd)
            os.close(fd)
            fd = child
        info = os.fstat(fd)
        if info.st_uid != os.geteuid() or info.st_mode & 0o022:
            raise SigningError("The private key directory must be owner-controlled.")
        yield fd, path.name
    finally:
        os.close(fd)


def open_private(directory: int, name: str) -> int:
    fd = os.open(name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK, dir_fd=directory)
    info = os.fstat(fd)
    if (
        not stat.S_ISREG(info.st_mode)
        or info.st_uid != os.geteuid()
        or info.st_mode & 0o077
        or info.st_nlink != 1
    ):
        os.close(fd)
        raise SigningError("The private key must be an owner-only regular file with one link.")
    return fd


def passphrase(confirm: bool = False) -> bytes:
    if not sys.stdin.isatty():
        raise SigningError("Run this command in the key owner's local terminal.")
    with warnings.catch_warnings():
        warnings.simplefilter("error", getpass.GetPassWarning)
        value = getpass.getpass("Release key passphrase: ")
        if not value or any(char in value for char in ("\r", "\n", "\0")):
            raise SigningError("Enter a nonempty passphrase.")
        if confirm and value != getpass.getpass("Confirm passphrase: "):
            raise SigningError("Passphrases do not match.")
    return value.encode("utf-8")


def openssl(args: list[str], *, secret=None, key_fd=None, stdout=subprocess.PIPE) -> bytes:
    executable = shutil.which("openssl")
    if executable is None:
        raise SigningError("OpenSSL 3 with Ed25519 support is required.")
    fds = ()
    if key_fd is not None:
        os.lseek(key_fd, 0, os.SEEK_SET)
        fds = (key_fd,)
    # OpenSSL also uses stdin solely for the passphrase when a material file
    # is supplied. The passphrase is never a process argument or environment value.
    if secret is not None:
        option = "-pass" if args[0] == "genpkey" else "-passin"
        args = [*args, option, "stdin"]
    result = subprocess.run(
        [executable, *args],
        input=secret + b"\n" if secret is not None else b"",
        stdout=stdout,
        stderr=subprocess.DEVNULL,
        pass_fds=fds,
        timeout=60,
        check=False,
    )
    if result.returncode:
        raise SigningError("OpenSSL rejected the input or operation.")
    return result.stdout


def derive_public(key_fd: int, secret: bytes) -> bytes:
    der = openssl(
        ["pkey", "-in", f"/dev/fd/{key_fd}", "-pubout", "-outform", "DER"],
        secret=secret, key_fd=key_fd,
    )
    if not der.startswith(SPKI_PREFIX):
        raise SigningError("The private key is not Ed25519.")
    return public_key(der[len(SPKI_PREFIX):])


def sign_material(key_fd: int, secret: bytes, raw_key: bytes, material: bytes) -> bytes:
    with tempfile.TemporaryDirectory(prefix="nagi-release-sign-") as scratch:
        directory = Path(scratch)
        payload = directory / "material"
        payload.write_bytes(material)
        signature = openssl(
            ["pkeyutl", "-sign", "-rawin", "-inkey", f"/dev/fd/{key_fd}", "-in", str(payload)],
            secret=secret, key_fd=key_fd,
        )
        if len(signature) != 64:
            raise SigningError("OpenSSL did not return a raw 64-byte Ed25519 signature.")
        verify_key = directory / "public.der"
        verify_key.write_bytes(SPKI_PREFIX + raw_key)
        detached = directory / "signature"
        detached.write_bytes(signature)
        openssl([
            "pkeyutl", "-verify", "-rawin", "-pubin", "-keyform", "DER",
            "-inkey", str(verify_key), "-in", str(payload), "-sigfile", str(detached),
        ])
        return signature


def run(main) -> None:
    try:
        main()
    except SigningError as error:
        sys.exit(str(error))
    except (OSError, ValueError, subprocess.TimeoutExpired, EOFError, KeyboardInterrupt, getpass.GetPassWarning):
        sys.exit("Release signing did not complete. Check local files and OpenSSL.")
