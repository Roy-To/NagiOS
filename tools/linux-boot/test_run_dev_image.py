import hashlib
import json
import shlex
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import run_dev_image
import data_boot_config


SCRIPTS = Path(__file__).resolve().parent
DATA_UUID = "aabbccdd-1111-2222-3333-444455556666"


def _newc_record(name, payload, *, inode, mode, nlink):
    name_bytes = name.encode("ascii") + b"\0"
    fields = [inode, mode, 0, 0, nlink, 0, len(payload), 0, 0, 0, 0, len(name_bytes), 0]
    result = bytearray(b"070701" + b"".join(f"{value:08x}".encode("ascii") for value in fields))
    result.extend(name_bytes)
    result.extend(b"\0" * (-len(result) % 4))
    result.extend(payload)
    result.extend(b"\0" * (-len(result) % 4))
    return bytes(result)


def _synthetic_initramfs():
    return (
        _newc_record("init", b"synthetic init", inode=1, mode=stat.S_IFREG | 0o755, nlink=1)
        + _newc_record("TRAILER!!!", b"", inode=0, mode=0, nlink=1)
    )


class DevImageTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="nagi-dev-image-")
        self.addCleanup(temporary.cleanup)
        self.source = Path(temporary.name) / "prepared"
        self.source.mkdir()
        self.bundle = self.source.parent / "bundle with spaces"
        artifacts = {}
        for key in ("kernel", "initramfs", "system_image"):
            artifact = self.source / key
            artifact.write_bytes(_synthetic_initramfs() if key == "initramfs" else key.encode())
            artifacts[key] = {
                "path": str(artifact),
                "sha256": hashlib.sha256(artifact.read_bytes()).hexdigest(),
            }
        self.data = self.source / "data.raw"
        self.data.write_bytes(b"original Data")
        self.preflight = {
            "artifacts": artifacts,
            "data_work_copy": {"path": str(self.data), "filesystem_uuid": DATA_UUID},
        }
        self.write_preflight()

    def write_preflight(self):
        (self.source / "preflight.json").write_text(json.dumps(self.preflight), encoding="utf-8")

    def run_script(self, script, *args):
        return subprocess.run(
            [sys.executable, "-B", str(SCRIPTS / script), *map(str, args)],
            capture_output=True,
            text=True,
            timeout=15,
        )

    def make(self):
        return self.run_script("make_dev_bundle.py", self.source, "--output", self.bundle)

    def print_command(self):
        return self.run_script(
            "run_dev_image.py", self.bundle, "--qemu", "qemu-test-command", "--print-command"
        )

    def test_make_and_print_command_keep_data_copy_and_read_only_system(self):
        for name in ("fixture.json", "receipt.json", "serial.log"):
            (self.source / name).write_text("fixture", encoding="utf-8")
        result = self.make()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            {item.name for item in self.bundle.iterdir()},
            {"bundle.json", "kernel.bzImage", "initramfs.cpio", "system.raw", "data.raw"},
        )
        copied_data = self.bundle / "data.raw"
        self.assertEqual(copied_data.read_bytes(), self.data.read_bytes())
        self.assertFalse(copied_data.samefile(self.data))
        copied_data.write_bytes(b"new Data")
        self.assertEqual(self.data.read_bytes(), b"original Data")

        result = self.print_command()
        self.assertEqual(result.returncode, 0, result.stderr)
        command = shlex.split(result.stdout)
        self.assertEqual(command[0], "qemu-test-command")
        options = {
            flag: command[command.index(flag) + 1]
            for flag in ("-machine", "-accel", "-smp", "-m", "-kernel", "-append",
                         "-display", "-nic", "-monitor", "-serial")
        }
        self.assertEqual(options["-machine"], "q35,i8042=off")
        self.assertEqual(options["-accel"], "tcg,thread=multi")
        self.assertEqual((options["-smp"], options["-m"]), ("4", "8192"))
        self.assertEqual(options["-kernel"], str(self.bundle / "kernel.bzImage"))
        self.assertEqual(options["-append"], "console=ttyS0,115200n8 rdinit=/init")
        self.assertNotIn("nagi.dev_data.", options["-append"])
        self.assertIn("virtio-blk-pci,drive=datadisk,serial=NAGI-DATA-aabbccdd", command)
        config = data_boot_config.extract_selection_config((self.bundle / "initramfs.cpio").read_bytes())
        self.assertTrue(data_boot_config.verify_config_signature(config))
        self.assertEqual(data_boot_config.selection_filesystem_uuid(config), DATA_UUID)
        self.assertNotEqual(options["-display"], "none")
        self.assertEqual((options["-nic"], options["-monitor"], options["-serial"]),
                         ("none", "none", "stdio"))
        self.assertNotIn("-qmp", command)
        drives = [command[i + 1] for i, flag in enumerate(command) if flag == "-drive"]
        self.assertIn("readonly=on", drives[0])
        self.assertIn(str(copied_data), drives[1])
        self.assertNotIn("readonly=", drives[1])

    def test_runner_rejects_missing_manifest(self):
        self.bundle.mkdir()
        result = self.print_command()
        self.assertEqual(result.returncode, 2)
        self.assertIn("missing bundle file: bundle.json", result.stderr)

    def test_runner_rejects_tampered_kernel(self):
        self.assertEqual(self.make().returncode, 0)
        (self.bundle / "kernel.bzImage").write_bytes(b"Kernel")
        result = self.print_command()
        self.assertEqual(result.returncode, 2)
        self.assertIn("kernel does not match its recorded SHA-256", result.stderr)

    def test_runner_rejects_signed_data_uuid_different_from_bundle_uuid(self):
        self.assertEqual(self.make().returncode, 0)
        manifest_path = self.bundle / "bundle.json"
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
        manifest["data_uuid"] = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
        manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
        result = self.print_command()
        self.assertEqual(result.returncode, 2)
        self.assertIn("signed Data selection UUID differs", result.stderr)

    def test_dev_selection_signature_rejects_mutation(self):
        config = data_boot_config.sign_config(DATA_UUID)
        self.assertTrue(data_boot_config.verify_config_signature(config))
        tampered = bytearray(config)
        tampered[-1] ^= 1
        self.assertFalse(data_boot_config.verify_config_signature(bytes(tampered)))

    def test_runner_rejects_symlink(self):
        self.assertEqual(self.make().returncode, 0)
        kernel = self.bundle / "kernel.bzImage"
        kernel.unlink()
        kernel.symlink_to(self.source / "kernel")
        result = self.print_command()
        self.assertEqual(result.returncode, 2)
        self.assertIn("bundle file must be a regular file", result.stderr)

    def test_maker_refuses_existing_output(self):
        self.bundle.mkdir()
        sentinel = self.bundle / "original"
        sentinel.write_bytes(b"keep")
        self.assertEqual(self.make().returncode, 2)
        self.assertEqual(sentinel.read_bytes(), b"keep")
        self.assertEqual(list(self.bundle.iterdir()), [sentinel])

    def test_maker_rejects_tampered_initramfs(self):
        (self.source / "initramfs").write_bytes(b"Initramfs")
        result = self.make()
        self.assertEqual(result.returncode, 2)
        self.assertIn("initramfs does not match the SHA-256", result.stderr)
        self.assertFalse(self.bundle.exists())

    def test_maker_rejects_data_outside_the_prepared_run(self):
        other_data = self.source.parent / "data.raw"
        other_data.write_bytes(b"other Data")
        self.preflight["data_work_copy"]["path"] = str(other_data)
        self.write_preflight()
        result = self.make()
        self.assertEqual(result.returncode, 2)
        self.assertIn("run-local data.raw", result.stderr)
        self.assertFalse(self.bundle.exists())

    def test_display_defaults_follow_the_host(self):
        for host, expected in (("Darwin", "cocoa"), ("Windows", "sdl"), ("Linux", "gtk")):
            with self.subTest(host=host), patch.object(run_dev_image.platform, "system", return_value=host):
                self.assertEqual(run_dev_image.default_display(), expected)


if __name__ == "__main__":
    unittest.main()
