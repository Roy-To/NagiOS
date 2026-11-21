#!/usr/bin/env python3
"""Lightweight host-side tests for prepare.py; all scratch stays under an explicit root."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import os
import shutil
import stat
import struct
import sys
import tempfile
import unittest
from pathlib import Path

MODULE_PATH = os.path.join(os.path.dirname(os.path.abspath(__file__)), "prepare.py")
SPEC = importlib.util.spec_from_file_location("nagi_linux_boot_prepare", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load prepare.py")
prepare = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = prepare
SPEC.loader.exec_module(prepare)


def synthetic_kernel(architecture: str = "x86_64", *, big_endian: bool = False) -> bytes:
    if architecture == "x86_64":
        content = bytearray(0x300)
        content[0x1FE:0x200] = bytes((0x55, 0xAA))
        content[0x202:0x206] = b"HdrS"
        content[0x206:0x208] = (0x020C).to_bytes(2, "little")
        content[0x236:0x238] = (0x0001).to_bytes(2, "little")
        return bytes(content)
    content = bytearray(0x100)
    content[0x08:0x10] = (0x80000).to_bytes(8, "little")
    content[0x10:0x18] = (0x200000).to_bytes(8, "little")
    flags = 1 if big_endian else 0
    content[0x18:0x20] = flags.to_bytes(8, "little")
    content[0x38:0x3C] = (0x644D5241).to_bytes(4, "little")
    return bytes(content)


def synthetic_static_elf(
    architecture: str = "x86_64", *, interp: bool = False, dynamic: bool = False
) -> bytes:
    content = bytearray(0x110)
    ident = bytes((0x7F, 0x45, 0x4C, 0x46, 2, 1, 1, 0, 0)) + bytes(7)
    machine = 0x3E if architecture == "x86_64" else 0xB7
    phnum = 1 + int(interp) + int(dynamic)
    header = struct.pack(
        "<16sHHIQQQIHHHHHH",
        ident, 2, machine, 1, 0x401000, 64, 0, 0, 64, 56, phnum, 0, 0, 0,
    )
    content[:64] = header
    headers = [struct.pack("<IIQQQQQQ", 1, 5, 0x100, 0x401000, 0, 4, 4, 0x1000)]
    if interp:
        headers.append(struct.pack("<IIQQQQQQ", 3, 4, 0x108, 0, 0, 1, 1, 1))
    if dynamic:
        headers.append(struct.pack("<IIQQQQQQ", 2, 4, 0x108, 0, 0, 8, 8, 8))
    offset = 64
    for phdr in headers:
        content[offset:offset + len(phdr)] = phdr
        offset += len(phdr)
    content[0x100:0x104] = b"INIT"
    return bytes(content)


CONFIGS = {
    "x86_64": b"""CONFIG_X86_64=y
CONFIG_BLK_DEV_INITRD=y
CONFIG_BINFMT_ELF=y
CONFIG_PRINTK=y
CONFIG_SERIAL_8250=y
CONFIG_SERIAL_8250_CONSOLE=y
# CONFIG_CMDLINE_OVERRIDE is not set
""",
    "aarch64": b"""CONFIG_ARM64=y
CONFIG_BLK_DEV_INITRD=y
CONFIG_BINFMT_ELF=y
CONFIG_PRINTK=y
CONFIG_SERIAL_AMBA_PL011=y
CONFIG_SERIAL_AMBA_PL011_CONSOLE=y
# CONFIG_CMDLINE_OVERRIDE is not set
""",
}


def cpio_names_and_init(archive: bytes) -> tuple[list[str], bytes]:
    names = []
    init_data = b""
    cursor = 0
    while cursor < len(archive):
        if archive[cursor:cursor + 6] != b"070701":
            raise AssertionError(f"invalid newc magic at {cursor}")
        values = [
            int(archive[cursor + 6 + index * 8:cursor + 14 + index * 8], 16)
            for index in range(13)
        ]
        file_size = values[6]
        name_size = values[11]
        header_end = cursor + 110
        name_end = header_end + name_size
        name = archive[header_end:name_end - 1].decode("ascii")
        data_start = (name_end + 3) & ~3
        data_end = data_start + file_size
        names.append(name)
        if name == "init":
            init_data = archive[data_start:data_end]
        cursor = (data_end + 3) & ~3
        if name == "TRAILER!!!":
            break
    return names, init_data


class PrepareTests(unittest.TestCase):
    def setUp(self) -> None:
        scratch = os.environ.get("NAGI_LINUX_BOOT_TEST_ROOT")
        if not scratch:
            raise RuntimeError("NAGI_LINUX_BOOT_TEST_ROOT must name a self-owned temporary directory")
        self.scratch = os.path.abspath(scratch)
        info = os.lstat(self.scratch)
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            raise RuntimeError("NAGI_LINUX_BOOT_TEST_ROOT must be a non-symlink directory")
        self.work = tempfile.mkdtemp(prefix="prepare-test-", dir=self.scratch)
        self.addCleanup(shutil.rmtree, self.work)
        self.inputs = {"x86_64": self._write_inputs("x86_64")}

    def _write_inputs(self, architecture: str) -> dict[str, str]:
        directory = os.path.join(self.work, architecture)
        os.mkdir(directory)
        paths = {
            "kernel": os.path.join(directory, "kernel.input"),
            "config": os.path.join(directory, ".config"),
            "init": os.path.join(directory, "nagi-init.elf"),
        }
        Path(paths["kernel"]).write_bytes(synthetic_kernel(architecture))
        Path(paths["config"]).write_bytes(CONFIGS[architecture])
        Path(paths["init"]).write_bytes(synthetic_static_elf(architecture))
        return paths

    def run_prepare(
        self,
        output: str,
        *,
        architecture: str = "x86_64",
        paths: dict[str, str] | None = None,
        kernel_hash: str | None = None,
        config_hash: str | None = None,
        init_hash: str | None = None,
        revision: str | None = None,
        build_record_path: str | None = None,
    ) -> dict:
        inputs = paths or self.inputs[architecture]
        return prepare.prepare_bundle(
            architecture=architecture,
            kernel_path=inputs["kernel"],
            kernel_sha256=kernel_hash or hashlib.sha256(Path(inputs["kernel"]).read_bytes()).hexdigest(),
            kernel_config_path=inputs["config"],
            kernel_config_sha256=config_hash or hashlib.sha256(Path(inputs["config"]).read_bytes()).hexdigest(),
            kernel_revision=revision or prepare.KERNEL_COMMIT_REFERENCE,
            init_path=inputs["init"],
            init_sha256=init_hash or hashlib.sha256(Path(inputs["init"]).read_bytes()).hexdigest(),
            output_dir=output,
            kernel_build_record_path=build_record_path,
        )

    def _write_build_record(self, path: str, *, wrong_kernel_hash: bool = False) -> bytes:
        inputs = self.inputs["x86_64"]
        kernel_sha = hashlib.sha256(Path(inputs["kernel"]).read_bytes()).hexdigest()
        config_sha = hashlib.sha256(Path(inputs["config"]).read_bytes()).hexdigest()
        if wrong_kernel_hash:
            kernel_sha = "0" * 64
        record = {
            "format": "nagi-linux-kernel-build",
            "format_version": 2,
            "target": "x86_64-linux-qemu-q35",
            "source": {
                "release_tag": prepare.KERNEL_RELEASE_TAG,
                "makefile_release": "6.18.63",
                "archive_url": prepare.KERNEL_ARCHIVE_URL,
                "signature_url": prepare.KERNEL_SIGNATURE_URL,
                "archive_sha256": "1" * 64,
                "archive_size_bytes": 100,
                "signature_sha256": "2" * 64,
                "signature_size_bytes": 90,
                "keyring_sha256": "3" * 64,
                "keyring_size_bytes": 80,
                "keyring_trust": "synthetic fixture only",
                "signature_verified": True,
                "signature_verification": prepare.KERNEL_SIGNATURE_METHOD,
                "commit_reference": prepare.KERNEL_COMMIT_REFERENCE,
                "commit_reference_verified": False,
                "commit_proof_limit": "the signed archive does not authenticate the Git commit mapping",
                "source_tree_sha256": "4" * 64,
            },
            "artifacts": {
                "kernel": {"sha256": kernel_sha},
                "config": {"sha256": config_sha},
            },
            "config_fragment": {"sha256": "5" * 64},
        }
        data = (json.dumps(record, sort_keys=True, indent=2) + "\n").encode("utf-8")
        Path(path).write_bytes(data)
        return data

    def test_x86_64_bundle_is_deterministic(self) -> None:
        first = os.path.join(self.work, "bundle-x86-a")
        second = os.path.join(self.work, "bundle-x86-b")
        manifest_a = self.run_prepare(first)
        manifest_b = self.run_prepare(second)
        self.assertEqual(manifest_a, manifest_b)
        for name in ("kernel.bzImage", "kernel.config", "nagi-init.elf", "initramfs.cpio", "manifest.json"):
            self.assertEqual(Path(first, name).read_bytes(), Path(second, name).read_bytes())
        archive = Path(first, "initramfs.cpio").read_bytes()
        names, init_data = cpio_names_and_init(archive)
        self.assertEqual(names, ["dev", "dev/console", "dev/null", "dev/ttyS0", "proc", "sys", "run", "tmp", "init", "TRAILER!!!"])
        self.assertEqual(init_data, Path(self.inputs["x86_64"]["init"]).read_bytes())
        self.assertEqual(manifest_a["target"]["architecture"], "x86_64")
        self.assertEqual(manifest_a["kernel"]["format"], "linux-x86_64-bzImage")
        self.assertEqual(manifest_a["kernel"]["release_tag_reference"], prepare.KERNEL_RELEASE_TAG)
        self.assertEqual(manifest_a["kernel"]["commit_reference"], prepare.KERNEL_COMMIT_REFERENCE)
        self.assertFalse(manifest_a["kernel"]["commit_reference_verified"])
        self.assertEqual(manifest_a["kernel"]["provenance"]["status"], "unverified")
        self.assertNotIn("revision", manifest_a["kernel"])
        self.assertEqual(manifest_a["initramfs"]["console_nodes"]["/dev/ttyS0"], "char 4:64")
        self.assertEqual(manifest_a["boot"]["kernel_command_line"], ["console=ttyS0", "rdinit=/init"])

    def test_selected_kernel_minimum_config_is_x86_64_fragment(self) -> None:
        path = os.path.join(os.path.dirname(MODULE_PATH), "kernel-minimum.config")
        info = os.lstat(path)
        self.assertFalse(stat.S_ISLNK(info.st_mode))
        self.assertTrue(stat.S_ISREG(info.st_mode))
        fd = os.open(path, os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0))
        with os.fdopen(fd, "rb") as source:
            data = source.read()
        values = prepare._parse_kernel_config(data, "x86_64")
        self.assertEqual(values["CONFIG_X86_64"], "y")
        self.assertNotIn("CONFIG_ARM64", values)
        self.assertNotEqual(values.get("CONFIG_CMDLINE_OVERRIDE"), "y")

    def test_aarch64_bundle_uses_arm64_image_elf_and_console(self) -> None:
        paths = self._write_inputs("aarch64")
        self.inputs["aarch64"] = paths
        output = os.path.join(self.work, "bundle-arm64")
        manifest = self.run_prepare(output, architecture="aarch64", paths=paths)
        self.assertEqual(Path(output, "kernel.Image").read_bytes(), Path(paths["kernel"]).read_bytes())
        self.assertFalse(os.path.exists(os.path.join(output, "kernel.bzImage")))
        self.assertEqual(manifest["target"]["architecture"], "aarch64")
        self.assertEqual(manifest["target"]["machine"], "virt")
        self.assertEqual(manifest["kernel"]["format"], "linux-aarch64-Image")
        self.assertEqual(manifest["kernel"]["header"]["text_offset"], 0x80000)
        self.assertEqual(manifest["init"]["target"], "aarch64-unknown-linux-nagi")
        self.assertEqual(manifest["init"]["format"], "ELF64-LE-ET_EXEC-EM_AARCH64-static")
        self.assertEqual(manifest["initramfs"]["console_nodes"]["/dev/ttyAMA0"], "char 204:64")
        self.assertEqual(manifest["boot"]["kernel_command_line"], ["console=ttyAMA0", "rdinit=/init"])
        self.assertEqual(manifest["boot"]["qemu_argv"][0], "qemu-system-aarch64")
        names, init_data = cpio_names_and_init(Path(output, "initramfs.cpio").read_bytes())
        self.assertIn("dev/ttyAMA0", names)
        self.assertNotIn("dev/ttyS0", names)
        self.assertEqual(init_data, Path(paths["init"]).read_bytes())

    def test_architecture_mismatch_is_rejected(self) -> None:
        output = os.path.join(self.work, "wrong-arch")
        with self.assertRaisesRegex(prepare.InputError, "aarch64 kernel must be an uncompressed Image"):
            self.run_prepare(output, architecture="aarch64", paths=self.inputs["x86_64"])
        self.assertFalse(os.path.exists(output))

    def test_aarch64_big_endian_kernel_image_is_rejected(self) -> None:
        paths = self._write_inputs("aarch64-big-endian")
        Path(paths["kernel"]).write_bytes(synthetic_kernel("aarch64", big_endian=True))
        with self.assertRaisesRegex(prepare.InputError, "little-endian"):
            self.run_prepare(os.path.join(self.work, "big-endian"), architecture="aarch64", paths=paths)

    def test_aarch64_config_rejects_x86_config(self) -> None:
        paths = self._write_inputs("aarch64-wrong-config")
        Path(paths["config"]).write_bytes(CONFIGS["x86_64"])
        with self.assertRaisesRegex(prepare.InputError, "requires CONFIG_ARM64=y"):
            self.run_prepare(os.path.join(self.work, "arm-config"), architecture="aarch64", paths=paths)

    def test_unpinned_kernel_revision_fails_before_output_creation(self) -> None:
        output = os.path.join(self.work, "wrong-kernel-revision")
        with self.assertRaisesRegex(prepare.InputError, "revision reference"):
            self.run_prepare(output, revision=prepare.KERNEL_RELEASE_TAG)
        self.assertFalse(os.path.exists(output))

    def test_build_record_is_matched_and_preserved_with_unverified_commit_mapping(self) -> None:
        output = os.path.join(self.work, "recorded-build")
        record_path = os.path.join(self.work, "build-record.json")
        record_bytes = self._write_build_record(record_path)
        manifest = self.run_prepare(output, build_record_path=record_path)
        provenance = manifest["kernel"]["provenance"]
        self.assertEqual(provenance["source_signature_status"], "reported_verified_in_build_record")
        self.assertFalse(provenance["build_record_authenticated"])
        self.assertFalse(provenance["commit_reference_verified"])
        self.assertEqual(Path(output, "kernel-build-record.json").read_bytes(), record_bytes)

    def test_build_record_kernel_hash_mismatch_fails_before_output_creation(self) -> None:
        output = os.path.join(self.work, "mismatched-build-record")
        record_path = os.path.join(self.work, "mismatched-build-record.json")
        self._write_build_record(record_path, wrong_kernel_hash=True)
        with self.assertRaisesRegex(prepare.InputError, "kernel artifact hash differs from the build record"):
            self.run_prepare(output, build_record_path=record_path)
        self.assertFalse(os.path.exists(output))

    def test_kernel_sha_mismatch_fails_before_output_creation(self) -> None:
        output = os.path.join(self.work, "bad-kernel-hash")
        with self.assertRaisesRegex(prepare.InputError, "kernel SHA-256 mismatch"):
            self.run_prepare(output, kernel_hash="0" * 64)
        self.assertFalse(os.path.exists(output))

    def test_config_sha_mismatch_fails_before_output_creation(self) -> None:
        output = os.path.join(self.work, "bad-config-hash")
        with self.assertRaisesRegex(prepare.InputError, "kernel .config SHA-256 mismatch"):
            self.run_prepare(output, config_hash="0" * 64)
        self.assertFalse(os.path.exists(output))

    def test_invalid_x86_linux_boot_header_is_rejected(self) -> None:
        paths = self.inputs["x86_64"]
        Path(paths["kernel"]).write_bytes(b"not a kernel" + bytes(0x300))
        with self.assertRaisesRegex(prepare.InputError, "boot flag"):
            self.run_prepare(os.path.join(self.work, "bad-x86-kernel"))

    def test_dynamic_init_elf_is_rejected(self) -> None:
        paths = self.inputs["x86_64"]
        Path(paths["init"]).write_bytes(synthetic_static_elf(interp=True))
        with self.assertRaisesRegex(prepare.InputError, "PT_INTERP"):
            self.run_prepare(os.path.join(self.work, "dynamic-init"))

    def test_non_static_dynamic_segment_is_rejected(self) -> None:
        paths = self.inputs["x86_64"]
        Path(paths["init"]).write_bytes(synthetic_static_elf(dynamic=True))
        with self.assertRaisesRegex(prepare.InputError, "PT_DYNAMIC"):
            self.run_prepare(os.path.join(self.work, "dynamic-segment"))

    def test_existing_output_directory_is_never_overwritten(self) -> None:
        output = os.path.join(self.work, "existing")
        os.mkdir(output)
        sentinel = os.path.join(output, "keep")
        Path(sentinel).write_bytes(b"untouched")
        with self.assertRaisesRegex(prepare.InputError, "already exists"):
            self.run_prepare(output)
        self.assertEqual(Path(sentinel).read_bytes(), b"untouched")

    def test_symlink_input_is_rejected(self) -> None:
        paths = self.inputs["x86_64"]
        link = os.path.join(self.work, "kernel-link")
        os.symlink(paths["kernel"], link)
        inputs = dict(paths)
        inputs["kernel"] = link
        with self.assertRaisesRegex(prepare.InputError, "symlink"):
            self.run_prepare(os.path.join(self.work, "symlink-out"), paths=inputs)


if __name__ == "__main__":
    unittest.main(verbosity=2)
