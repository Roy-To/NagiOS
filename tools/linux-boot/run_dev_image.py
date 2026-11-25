#!/usr/bin/env python3
"""Start a prepared Nagi Linux development image in QEMU for interactive use.

This is the developer entry point behind `./nagi run`. It reads a bundle
directory prepared by the Linux image build and starts one QEMU guest with a
window. The bundle initramfs includes a separately signed development Data
selection entry; the kernel command line carries no Data selector.

Bundle layout (all regular files, no symlinks):
  bundle.json   {"schema": 1, "kernel": "...", "initramfs": "...",
                 "system_image": "...", "data_image": "...",
                 "data_uuid": "<canonical ext4 UUID>",
                 "source_initramfs_sha256": "...",
                 "data_selection_config_sha256": "...",
                 "sha256": {"kernel": "...", "initramfs": "...", "system_image": "..."}}
Paths in bundle.json are file names relative to the bundle directory.

The Data image is writable and keeps user data between runs. The System image
is attached read-only. QEMU runs without network, monitor or QMP.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import re
import shlex
import shutil
import stat
import subprocess
import sys
from pathlib import Path

import data_boot_config

UUID_PATTERN = re.compile(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}")
SHA256_PATTERN = re.compile(r"[0-9a-f]{64}")
NAME_PATTERN = re.compile(r"[A-Za-z0-9._-]{1,128}")
MAX_BUNDLE_JSON_BYTES = 16 * 1024


class BundleError(Exception):
    pass


def _regular_file(path: Path) -> Path:
    try:
        metadata = path.lstat()
    except FileNotFoundError:
        raise BundleError(f"missing bundle file: {path.name}") from None
    if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISREG(metadata.st_mode):
        raise BundleError(f"bundle file must be a regular file: {path.name}")
    return path


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def load_bundle(directory: Path) -> dict:
    if directory.is_symlink():
        raise BundleError("bundle directory must not be a symlink")
    manifest_path = _regular_file(directory / "bundle.json")
    if manifest_path.stat().st_size > MAX_BUNDLE_JSON_BYTES:
        raise BundleError("bundle.json is too large")
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise BundleError("bundle.json is not valid JSON") from None
    if (
        not isinstance(manifest, dict)
        or type(manifest.get("schema")) is not int
        or manifest["schema"] != 1
    ):
        raise BundleError("bundle.json schema must be 1")
    bundle: dict = {}
    for key in ("kernel", "initramfs", "system_image", "data_image"):
        name = manifest.get(key)
        if not isinstance(name, str) or not NAME_PATTERN.fullmatch(name):
            raise BundleError(f"bundle.json {key} must be a plain file name")
        bundle[key] = _regular_file(directory / name)
    for key in ("kernel", "initramfs", "system_image"):
        if bundle["data_image"].samefile(bundle[key]):
            raise BundleError("writable Data must be separate from the read-only inputs")
    data_uuid = manifest.get("data_uuid")
    if not isinstance(data_uuid, str) or not UUID_PATTERN.fullmatch(data_uuid):
        raise BundleError("bundle.json data_uuid must be a canonical lowercase UUID")
    bundle["data_uuid"] = data_uuid
    source_initramfs_sha256 = manifest.get("source_initramfs_sha256")
    if not isinstance(source_initramfs_sha256, str) or not SHA256_PATTERN.fullmatch(source_initramfs_sha256):
        raise BundleError("bundle.json source_initramfs_sha256 must be a lowercase SHA-256")
    data_selection_config_sha256 = manifest.get("data_selection_config_sha256")
    if (
        not isinstance(data_selection_config_sha256, str)
        or not SHA256_PATTERN.fullmatch(data_selection_config_sha256)
    ):
        raise BundleError("bundle.json data_selection_config_sha256 must be a lowercase SHA-256")
    bundle["source_initramfs_sha256"] = source_initramfs_sha256
    bundle["data_selection_config_sha256"] = data_selection_config_sha256
    expected = manifest.get("sha256")
    if not isinstance(expected, dict):
        raise BundleError("bundle.json sha256 must list kernel, initramfs and system_image")
    for key in ("kernel", "initramfs", "system_image"):
        value = expected.get(key)
        if not isinstance(value, str) or not SHA256_PATTERN.fullmatch(value):
            raise BundleError(f"bundle.json sha256.{key} must be a lowercase SHA-256")
        if _sha256(bundle[key]) != value:
            raise BundleError(f"{key} does not match its recorded SHA-256")
    try:
        selection = data_boot_config.extract_selection_config(bundle["initramfs"].read_bytes())
        if hashlib.sha256(selection).hexdigest() != data_selection_config_sha256:
            raise BundleError("Data selection config does not match its recorded SHA-256")
        if data_boot_config.selection_filesystem_uuid(selection) != data_uuid:
            raise BundleError("signed Data selection UUID differs from bundle.json data_uuid")
    except (data_boot_config.DataBootConfigError, ValueError) as error:
        raise BundleError(f"initramfs Data selection config is invalid ({error})") from None
    for key in ("kernel", "initramfs", "system_image", "data_image"):
        if any(character in str(bundle[key]) for character in ",\n\r"):
            raise BundleError("bundle paths cannot contain commas or line breaks")
    return bundle


def default_display() -> str:
    return {"Darwin": "cocoa", "Windows": "sdl"}.get(platform.system(), "gtk")


def qemu_argv(qemu: str, bundle: dict, display: str) -> list[str]:
    data_serial = f"NAGI-DATA-{bundle['data_uuid'][:8]}"
    kernel_cmdline = "console=ttyS0,115200n8 rdinit=/init"
    return [
        qemu,
        "-name", "nagi-linux-dev",
        "-machine", "q35,i8042=off",
        "-accel", "tcg,thread=multi",
        "-cpu", "max",
        "-smp", "4",
        "-m", "8192",
        "-kernel", str(bundle["kernel"]),
        "-initrd", str(bundle["initramfs"]),
        "-append", kernel_cmdline,
        "-drive", f"if=none,file={bundle['system_image']},format=raw,readonly=on,id=systemdisk",
        "-device", "virtio-blk-pci,drive=systemdisk",
        "-drive", f"if=none,file={bundle['data_image']},format=raw,id=datadisk",
        "-device", f"virtio-blk-pci,drive=datadisk,serial={data_serial}",
        "-vga", "none",
        "-device", "virtio-vga,xres=640,yres=480,edid=on",
        "-device", "virtio-keyboard-pci",
        "-display", display,
        "-nic", "none",
        "-serial", "stdio",
        "-monitor", "none",
        "-no-reboot",
    ]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("bundle", type=Path, nargs="?", default=Path("out/linux-dev"),
                        help="prepared bundle directory (default: out/linux-dev)")
    parser.add_argument("--qemu", default=os.environ.get("NAGI_QEMU", "qemu-system-x86_64"))
    parser.add_argument("--display", default=None,
                        help="QEMU display (default: cocoa on macOS, sdl on Windows, gtk elsewhere)")
    parser.add_argument("--print-command", action="store_true", help="print the QEMU command and exit")
    args = parser.parse_args(argv)
    try:
        bundle = load_bundle(args.bundle)
    except (BundleError, OSError) as error:
        print(f"run: {error}", file=sys.stderr)
        print("run: prepare the bundle with tools/linux-boot/make_dev_bundle.py first", file=sys.stderr)
        return 2
    qemu = shutil.which(args.qemu) or args.qemu
    command = qemu_argv(qemu, bundle, args.display or default_display())
    if args.print_command:
        print(subprocess.list2cmdline(command) if os.name == "nt" else shlex.join(command))
        return 0
    if shutil.which(qemu) is None:
        print(f"run: QEMU was not found ({args.qemu})", file=sys.stderr)
        return 2
    try:
        return subprocess.call(command)
    except OSError as error:
        print(f"run: cannot start QEMU: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
