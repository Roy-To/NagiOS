#!/usr/bin/env python3
"""Create a `./nagi run` bundle from a prepared Task24 run directory.

The Task24 preparation (tools/linux-boot/task24_run_preflight.py and the
signed image build) already produces the kernel, signed initramfs, read-only
System image and a fresh Data image, and records them in preflight.json. This
tool copies those files into a new bundle directory (default out/linux-dev)
and writes bundle.json for run_dev_image.py.

The source run directory is not modified. Its Data image is copied, so the
acceptance evidence stays intact and the bundle gets its own writable Data.
Nothing else from the run directory (fixture credentials, receipts, serial
logs) is copied.
"""

from __future__ import annotations

import argparse
import json
import shutil
import stat
import sys
from pathlib import Path

from run_dev_image import BundleError, SHA256_PATTERN, UUID_PATTERN, _sha256

MAX_PREFLIGHT_BYTES = 1 << 20


def _regular(path: Path, label: str) -> Path:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        raise BundleError(f"{label} is missing") from None
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise BundleError(f"{label} must be a regular file")
    return path


def read_preflight(run_directory: Path) -> dict:
    preflight = _regular(run_directory / "preflight.json", "preflight.json")
    if preflight.stat().st_size > MAX_PREFLIGHT_BYTES:
        raise BundleError("preflight.json is too large")
    try:
        manifest = json.loads(preflight.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise BundleError("preflight.json is not valid JSON") from None
    try:
        artifacts = manifest["artifacts"]
        data = manifest["data_work_copy"]
        result = {
            key: (Path(artifacts[key]["path"]), artifacts[key]["sha256"])
            for key in ("kernel", "initramfs", "system_image")
        }
        result["data_image"] = Path(data["path"])
        result["data_uuid"] = data["filesystem_uuid"]
    except (KeyError, TypeError):
        raise BundleError("preflight.json does not list kernel, initramfs, system_image and data_work_copy") from None
    if not isinstance(result["data_uuid"], str) or not UUID_PATTERN.fullmatch(result["data_uuid"]):
        raise BundleError("preflight.json Data UUID must be a canonical lowercase UUID")
    return result


def make_bundle(run_directory: Path, output: Path) -> Path:
    source = read_preflight(run_directory)
    for key in ("kernel", "initramfs", "system_image"):
        path, expected = source[key]
        _regular(path, key)
        if (
            not isinstance(expected, str)
            or not SHA256_PATTERN.fullmatch(expected)
            or _sha256(path) != expected
        ):
            raise BundleError(f"{key} does not match the SHA-256 recorded in preflight.json")
    data_image = _regular(source["data_image"], "data image")
    if data_image.resolve() != run_directory.resolve() / "data.raw" or data_image.stat().st_nlink != 1:
        raise BundleError("Data must be the run-local data.raw work copy")
    if any(data_image.samefile(source[key][0]) for key in ("kernel", "initramfs", "system_image")):
        raise BundleError("writable Data must be separate from the read-only inputs")
    names = {
        "kernel": "kernel.bzImage",
        "initramfs": "initramfs.cpio",
        "system_image": "system.raw",
        "data_image": "data.raw",
    }
    bundle = {
        "schema": 1,
        **names,
        "data_uuid": source["data_uuid"],
        "sha256": {key: source[key][1] for key in ("kernel", "initramfs", "system_image")},
        "source_run": run_directory.name,
    }
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    try:
        for key, name in names.items():
            path = source[key][0] if key != "data_image" else data_image
            shutil.copyfile(path, output / name)
            if key == "data_image":
                (output / name).chmod(0o600)
            elif _sha256(output / name) != source[key][1]:
                raise BundleError(f"{key} changed while copying the bundle")
        (output / "bundle.json").write_text(json.dumps(bundle, indent=2) + "\n", encoding="utf-8")
    except (BundleError, OSError):
        shutil.rmtree(output)
        raise
    return output


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("run_directory", type=Path, help="prepared Task24 run directory containing preflight.json")
    parser.add_argument("--output", type=Path, default=Path("out/linux-dev"),
                        help="new bundle directory (must not exist; default out/linux-dev)")
    args = parser.parse_args(argv)
    try:
        output = make_bundle(args.run_directory, args.output)
    except (BundleError, OSError) as error:
        print(f"bundle: {error}", file=sys.stderr)
        return 2
    print(f"bundle: wrote {output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
