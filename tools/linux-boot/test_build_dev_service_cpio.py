import stat
import tempfile
import unittest
from pathlib import Path

import boot_acceptance
import build_dev_service_cpio as builder


def rewrite_newc_entry(archive, target, *, field=None, value=None, replacement_name=None):
    data = bytearray(archive)
    cursor = 0
    while cursor + 110 <= len(data):
        header = data[cursor:cursor + 110]
        fields = [int(header[index:index + 8], 16) for index in range(6, 110, 8)]
        namesize, size = fields[11], fields[6]
        name_start = cursor + 110
        name = bytes(data[name_start:name_start + namesize - 1]).decode("ascii")
        if name == target:
            if field is not None:
                start = cursor + 6 + field * 8
                data[start:start + 8] = f"{value:08x}".encode("ascii")
            if replacement_name is not None:
                replacement = replacement_name.encode("ascii") + b"\0"
                if len(replacement) != namesize:
                    raise ValueError("replacement CPIO name must retain namesize")
                data[name_start:name_start + namesize] = replacement
            return bytes(data)
        cursor = (name_start + namesize + 3) & ~3
        cursor = (cursor + size + 3) & ~3
    raise AssertionError(f"CPIO entry not found: {target}")


def duplicate_newc_entry(archive, target):
    record = None
    trailer_offset = None
    cursor = 0
    while cursor + 110 <= len(archive):
        header = archive[cursor:cursor + 110]
        fields = [int(header[index:index + 8], 16) for index in range(6, 110, 8)]
        namesize, size = fields[11], fields[6]
        name_start = cursor + 110
        name = bytes(archive[name_start:name_start + namesize - 1]).decode("ascii")
        data_start = (name_start + namesize + 3) & ~3
        record_end = (data_start + size + 3) & ~3
        if name == target:
            record = archive[cursor:record_end]
        if name == "TRAILER!!!":
            trailer_offset = cursor
            break
        cursor = record_end
    if record is None or trailer_offset is None:
        raise AssertionError(f"could not duplicate CPIO entry: {target}")
    return archive[:trailer_offset] + record + archive[trailer_offset:]


class DevelopmentServiceCpioTests(unittest.TestCase):
    def setUp(self):
        self.inputs = {
            "init": b"static-init-elf",
            "package": b"signed-xapp",
            "service": b"static-service-elf",
            "client": b"static-client-elf",
        }
        self.archive = builder.build_archive(
            self.inputs["init"],
            self.inputs["package"],
            self.inputs["service"],
            self.inputs["client"],
        )

    def test_exact_files_and_private_runtime_directory_validate(self):
        result = builder.validate_archive(
            self.archive,
            self.inputs["init"],
            self.inputs["package"],
            self.inputs["service"],
            self.inputs["client"],
        )
        self.assertEqual(result["cpio_structure"], "PASS")
        self.assertEqual(result["development_input_bytes_match"], "PASS")
        self.assertEqual(result["ready_verified"], False)
        self.assertEqual(
            result["development_character_devices"],
            [{"path": "dev/console", "type": "character", "rdev_major": 5, "rdev_minor": 1}],
        )
        directory = next(
            item for item in result["development_directories"]
            if item["path"] == "run/nagi"
        )
        self.assertEqual(directory["mode"], 0o700)
        self.assertEqual(result["init"]["size_bytes"], len(self.inputs["init"]))

    def test_initial_console_device_is_checked(self):
        cases = (
            (rewrite_newc_entry(self.archive, "dev/console", field=10, value=2),
             "DEV_CPIO_DEVICE_ID"),
            (rewrite_newc_entry(
                self.archive,
                "dev/console",
                field=1,
                value=stat.S_IFREG | 0o600,
            ), "DEV_CPIO_DEVICE_MODE"),
            (rewrite_newc_entry(
                self.archive,
                "dev/console",
                replacement_name="dev/consolx",
            ), "DEV_CPIO_DEVICE_MISSING"),
        )
        for archive, code in cases:
            with self.subTest(code=code):
                with self.assertRaisesRegex(boot_acceptance.AcceptanceError, code):
                    builder.validate_archive(
                        archive,
                        self.inputs["init"],
                        self.inputs["package"],
                        self.inputs["service"],
                        self.inputs["client"],
                    )

    def test_duplicate_and_truncated_archives_fail_closed_at_acceptance_entrypoint(self):
        duplicate = duplicate_newc_entry(self.archive, "dev/console")
        truncated = self.archive[:-16]
        for archive, expected_code in (
            (duplicate, "CPIO_DUPLICATE"),
            (truncated, "CPIO_HEADER"),
        ):
            with self.subTest(expected_code=expected_code):
                with self.assertRaises(boot_acceptance.AcceptanceError) as raised:
                    builder.validate_archive(
                        archive,
                        self.inputs["init"],
                        self.inputs["package"],
                        self.inputs["service"],
                        self.inputs["client"],
                    )
                self.assertEqual(raised.exception.code, expected_code)

    def test_static_dev_null_is_not_required_after_pid1_devtmpfs_mount(self):
        archive = rewrite_newc_entry(
            self.archive,
            "dev/null",
            replacement_name="dev/nulx",
        )
        result = builder.validate_archive(
            archive,
            self.inputs["init"],
            self.inputs["package"],
            self.inputs["service"],
            self.inputs["client"],
        )
        self.assertEqual(result["cpio_structure"], "PASS")
        self.assertEqual(
            [node["path"] for node in result["development_character_devices"]],
            ["dev/console"],
        )

    def test_changed_service_bytes_fail_acceptance(self):
        with self.assertRaises(boot_acceptance.AcceptanceError):
            builder.validate_archive(
                self.archive,
                self.inputs["init"],
                self.inputs["package"],
                b"different-service-elf",
                self.inputs["client"],
            )

    def test_output_is_create_new_and_collision_preserves_existing_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "initramfs.cpio"
            output.write_bytes(b"prior")
            with self.assertRaises(builder.BuildInputError):
                builder.write_new_output(output, self.archive)
            self.assertEqual(output.read_bytes(), b"prior")

    def test_notes_desktop_runtime_and_binary_are_pinned_into_dev_cpio(self):
        notes_desktop = b"static-notes-desktop-elf"
        archive = builder.build_archive(
            self.inputs["init"],
            self.inputs["package"],
            self.inputs["service"],
            self.inputs["client"],
            notes_desktop=notes_desktop,
            notes_phase="save",
        )
        result = builder.validate_archive(
            archive,
            self.inputs["init"],
            self.inputs["package"],
            self.inputs["service"],
            self.inputs["client"],
            notes_desktop=notes_desktop,
            notes_phase="save",
        )
        self.assertEqual(result["cpio_structure"], "PASS")
        self.assertEqual(result["development_input_bytes_match"], "PASS")

    def test_notes_desktop_runtime_requires_matching_binary_and_valid_phase(self):
        with self.assertRaises(builder.BuildInputError):
            builder.build_archive(
                self.inputs["init"],
                self.inputs["package"],
                self.inputs["service"],
                self.inputs["client"],
                notes_desktop=b"static-notes-desktop-elf",
            )
        with self.assertRaises(builder.BuildInputError):
            builder.notes_desktop_input("production")


if __name__ == "__main__":
    unittest.main()
