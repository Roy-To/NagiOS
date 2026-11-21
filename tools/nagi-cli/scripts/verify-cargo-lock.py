#!/usr/bin/env python3
"""Fail early when the root workspace lock does not match its manifests."""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path


CRATES_IO_SOURCE = b"registry+https://github.com/rust-lang/crates.io-index"


def validate(
    cargo: str, manifest_path: Path, env: dict[str, str] | None = None
) -> subprocess.CompletedProcess[bytes]:
    manifest = manifest_path.resolve(strict=True)
    return subprocess.run(
        [
            cargo,
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--offline",
            "--manifest-path",
            str(manifest),
        ],
        cwd=manifest.parent,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        env=env,
        check=False,
    )


def _lock_registry_sources(
    lockfile: Path, package_name: bytes, version: bytes
) -> list[bytes]:
    try:
        contents = lockfile.read_bytes()
    except OSError:
        return []
    name_line = re.compile(rb"(?m)^name = \"" + re.escape(package_name) + rb"\"$")
    version_line = re.compile(rb"(?m)^version = \"" + re.escape(version) + rb"\"$")
    source_line = re.compile(rb'(?m)^source = "(registry\+[^"\r\n]+)"$')
    sources: list[bytes] = []
    for package in re.split(rb"(?m)^\[\[package\]\]\s*$", contents):
        if name_line.search(package) and version_line.search(package):
            source = source_line.search(package)
            if source:
                sources.append(source.group(1))
    return sources


def classify_result(returncode: int, stderr: bytes, lockfile: Path) -> str:
    if returncode == 0:
        return "success"

    lowered = stderr.lower()
    if (
        b"the lock file" in lowered
        and b"needs to be updated but --locked was passed to prevent this" in lowered
    ):
        return "lock-update-required"

    offline_package = re.search(
        rb"(?im)^error: no matching package named `([^`]+)` found$", stderr
    )
    if (
        offline_package
        and b"location searched: crates.io index" in lowered
        and b"using offline mode (--offline)" in lowered
    ):
        # Cargo reports only a crate name here; version and source identity
        # are absent, so this cannot safely be matched to a lock entry.
        return "unknown"

    offline_download = (
        b"attempting to make an http request, but --offline was specified" in lowered
    )
    if offline_download:
        package_version = re.search(
            rb"(?im)^error: failed to download `([^` ]+) v([^`]+)`$", stderr
        )
        explicit_crates_io_url = re.search(
            rb"(?im)^error: failed to download from `https://static\.crates\.io/crates/([^/]+)/([^/]+)/download`$",
            stderr,
        )
        if package_version:
            name, version = package_version.groups()
        elif explicit_crates_io_url:
            name, version = explicit_crates_io_url.groups()
        else:
            return "unknown"

        sources = _lock_registry_sources(lockfile, name, version)
        if sources == [CRATES_IO_SOURCE]:
            return "offline-cache-insufficient"
        return "unknown"

    if re.search(rb"(?m)^error:\s", stderr):
        return "metadata-failure"
    return "unknown"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo", required=True)
    parser.add_argument("--manifest-path", required=True, type=Path)
    args = parser.parse_args(argv)

    try:
        manifest = args.manifest_path.resolve(strict=True)
        result = validate(args.cargo, manifest)
    except (OSError, subprocess.SubprocessError, ValueError) as error:
        print("METADATA_CLASSIFICATION=unknown", flush=True)
        print(f"FAIL Cargo workspace lock preflight: {error}", file=sys.stderr)
        return 2

    classification = classify_result(
        result.returncode, result.stderr, manifest.parent / "Cargo.lock"
    )
    print(
        f"METADATA_CLASSIFICATION={classification}",
        flush=True,
    )
    if result.stderr:
        sys.stderr.buffer.write(result.stderr)
        sys.stderr.buffer.flush()
    return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
