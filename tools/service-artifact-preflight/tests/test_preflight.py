import hashlib
import json
import os
import struct
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import service_artifact_preflight as preflight_module
from service_artifact_preflight import PreflightError, open_verified_artifact, preflight_json


ELF_HEADER_FORMAT = "<16sHHIQQQIHHHHHH"
PROGRAM_HEADER_FORMAT = "<IIQQQQQQ"
SYNTHETIC_STATIC_SHA256 = "b3c62731b6406d81fd95ad5f41caff7620bb4946cea69d0c8ef6a7d3afbecafe"
DEV_FIXTURE_PATH = os.environ.get("NAGI_SERVICE_PREFLIGHT_DEV_FIXTURE")
DEV_FIXTURE_SHA256 = "479e01ea9d8a3d8493ff93b14b946321724be2af3df051669ab70006a99497fb"


def make_elf(
    *,
    machine=62,
    program_headers=None,
    program_header_offset=64,
    payload=b"CODE",
    elf_type=2,
    entry=0x400000,
):
    if program_headers is None:
        program_headers = [
            (1, 5, 120, 0x400000, 0x400000, len(payload), len(payload), 0x1000)
        ]
    ident = b"\x7fELF" + bytes([2, 1, 1, 0]) + bytes(8)
    header = struct.pack(
        ELF_HEADER_FORMAT,
        ident,
        elf_type,
        machine,
        1,
        entry,
        program_header_offset,
        0,
        0,
        64,
        56,
        len(program_headers),
        64,
        0,
        0,
    )
    program_bytes = b"".join(struct.pack(PROGRAM_HEADER_FORMAT, *item) for item in program_headers)
    end = 64
    if program_header_offset <= 1024 * 1024:
        end = max(end, program_header_offset + len(program_bytes))
    for item in program_headers:
        if item[0] != 0 and item[2] <= 1024 * 1024:
            end = max(end, item[2] + item[5])
    data = bytearray(end)
    data[:64] = header
    if program_header_offset <= len(data):
        stop = min(len(data), program_header_offset + len(program_bytes))
        data[program_header_offset:stop] = program_bytes[: stop - program_header_offset]
    load_offsets = [item[2] for item in program_headers if item[0] == 1]
    if payload and load_offsets and load_offsets[0] + len(payload) <= len(data):
        start = load_offsets[0]
        data[start : start + len(payload)] = payload
    return bytes(data)


STATIC_ELF = make_elf()


class ServiceArtifactPreflightTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp_dir.cleanup)
        self.root = Path(self.temp_dir.name).resolve()

    def write_artifact(self, name, data):
        path = self.root / name
        path.write_bytes(data)
        return path

    def request_json(self, path, digest=SYNTHETIC_STATIC_SHA256, extra=""):
        return (
            '{"schema_version":1,'
            f'"artifact_path":{json.dumps(str(path))},'
            f'"expected_sha256":"{digest}",'
            '"target":"x86_64-unknown-linux-musl"'
            f"{extra}"
            "}"
        )

    def assert_code(self, code, raw):
        with self.assertRaises(PreflightError) as context:
            preflight_json(raw)
        self.assertEqual(context.exception.code, code)

    def test_valid_static_elf_passes_and_reports_limited_scope(self):
        self.assertEqual(hashlib.sha256(STATIC_ELF).hexdigest(), SYNTHETIC_STATIC_SHA256)
        path = self.write_artifact("service.elf", STATIC_ELF)
        result = preflight_json(self.request_json(path))
        self.assertEqual(result["status"], "artifact_preflight_passed")
        self.assertEqual(result["artifact_sha256"], SYNTHETIC_STATIC_SHA256)
        self.assertTrue(result["checks"]["no_pt_interp"])
        self.assertEqual(result["claim_scope"], "artifact_preflight_only")
        self.assertIn("runtime_ready", result["not_proven"])
        self.assertNotIn("ready", result)

    def test_assignment_dev_fixture_is_read_only_development_fixture(self):
        if not DEV_FIXTURE_PATH or not Path(DEV_FIXTURE_PATH).is_file():
            self.skipTest("assigned dev fixture is unavailable")
        result = preflight_json(
            self.request_json(Path(DEV_FIXTURE_PATH), digest=DEV_FIXTURE_SHA256)
        )
        self.assertEqual(result["artifact_sha256"], DEV_FIXTURE_SHA256)
        self.assertEqual(result["claim_scope"], "artifact_preflight_only")

    def test_descriptor_api_keeps_the_validated_file_open_and_closes_it(self):
        path = self.write_artifact("service.elf", STATIC_ELF)
        with open_verified_artifact(self.request_json(path)) as verified:
            fd = verified.fileno()
            self.assertTrue(os.fstat(fd).st_size)
            self.assertEqual(os.pread(fd, len(STATIC_ELF), 0), STATIC_ELF)
            self.assertEqual(verified.result["status"], "artifact_preflight_passed")
        with self.assertRaises(ValueError):
            verified.fileno()

    def test_cli_style_preflight_closes_artifact_and_parent_descriptors(self):
        path = self.write_artifact("service.elf", STATIC_ELF)
        real_open_artifact = preflight_module._open_artifact
        real_close = os.close
        opened_fds = []
        closed_fds = []

        def capture_open(path_string):
            opened = real_open_artifact(path_string)
            opened_fds.extend(opened[:2])
            return opened

        def capture_close(fd):
            closed_fds.append(fd)
            return real_close(fd)

        with mock.patch.object(preflight_module, "_open_artifact", side_effect=capture_open):
            with mock.patch.object(preflight_module.os, "close", side_effect=capture_close):
                preflight_json(self.request_json(path))
        self.assertTrue(set(opened_fds).issubset(closed_fds))

    def test_retained_descriptor_survives_path_replacement_without_reopen(self):
        path = self.write_artifact("service.elf", STATIC_ELF)
        replacement = self.write_artifact("replacement.elf", b"replacement")
        with open_verified_artifact(self.request_json(path)) as verified:
            fd = verified.fileno()
            os.replace(replacement, path)
            self.assertEqual(os.pread(fd, len(STATIC_ELF), 0), STATIC_ELF)
            self.assertNotEqual(path.read_bytes(), STATIC_ELF)

    def test_path_replacement_during_preflight_is_rejected(self):
        path = self.write_artifact("service.elf", STATIC_ELF)
        replacement = self.write_artifact("replacement.elf", b"replacement")
        real_pread = os.pread
        swapped = False

        def pread_and_swap(fd, length, offset):
            nonlocal swapped
            data = real_pread(fd, length, offset)
            if not swapped and offset == 0:
                os.replace(replacement, path)
                swapped = True
            return data

        with mock.patch.object(preflight_module.os, "pread", side_effect=pread_and_swap):
            self.assert_code(
                "artifact_changed_during_preflight", self.request_json(path)
            )

    def test_digest_mismatch_is_rejected(self):
        path = self.write_artifact("service.elf", STATIC_ELF)
        self.assert_code("digest_mismatch", self.request_json(path, digest="0" * 64))

    def test_symlink_file_is_rejected(self):
        target = self.write_artifact("target.elf", STATIC_ELF)
        link = self.root / "service.elf"
        link.symlink_to(target)
        self.assert_code("symlink_not_allowed", self.request_json(link))

    def test_symlink_directory_component_is_rejected(self):
        real_dir = self.root / "real"
        real_dir.mkdir()
        (real_dir / "service.elf").write_bytes(STATIC_ELF)
        link_dir = self.root / "linked"
        link_dir.symlink_to(real_dir, target_is_directory=True)
        self.assert_code(
            "symlink_not_allowed", self.request_json(link_dir / "service.elf")
        )

    def test_directory_is_not_a_regular_file(self):
        directory = self.root / "directory"
        directory.mkdir()
        self.assert_code("not_regular_file", self.request_json(directory))

    def test_missing_artifact_is_rejected(self):
        self.assert_code("artifact_not_found", self.request_json(self.root / "missing.elf"))

    def test_missing_json_field_is_rejected(self):
        path = self.write_artifact("service.elf", STATIC_ELF)
        document = json.loads(self.request_json(path))
        del document["target"]
        raw = json.dumps(document)
        self.assert_code("missing_field", raw)

    def test_duplicate_json_field_is_rejected(self):
        path = self.write_artifact("service.elf", STATIC_ELF)
        raw = self.request_json(path).replace(
            '"schema_version":1,', '"schema_version":1,"schema_version":1,'
        )
        self.assert_code("duplicate_json_field", raw)

    def test_unknown_json_field_is_rejected(self):
        path = self.write_artifact("service.elf", STATIC_ELF)
        self.assert_code("unknown_field", self.request_json(path, extra=',"role":"system"'))

    def test_unencodable_json_path_is_rejected(self):
        document = json.loads(self.request_json(self.root / "unused.elf"))
        document["artifact_path"] = "\ud800"
        self.assert_code("invalid_artifact_path", json.dumps(document))

    def test_wrong_target_is_rejected(self):
        path = self.write_artifact("service.elf", STATIC_ELF)
        raw = self.request_json(path).replace("x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl")
        self.assert_code("unsupported_target", raw)

    def test_wrong_architecture_is_rejected(self):
        path = self.write_artifact("wrong-arch.elf", make_elf(machine=183))
        self.assert_code("wrong_architecture", self.request_json(path, digest="0" * 64))

    def test_truncated_elf_header_is_rejected(self):
        path = self.write_artifact("truncated.elf", STATIC_ELF[:32])
        self.assert_code("truncated_elf", self.request_json(path, digest="0" * 64))

    def test_program_header_table_out_of_bounds_is_rejected_without_overflow(self):
        path = self.write_artifact(
            "bad-table.elf", make_elf(program_header_offset=(1 << 64) - 1)
        )
        self.assert_code(
            "program_header_table_out_of_bounds",
            self.request_json(path, digest="0" * 64),
        )

    def test_segment_range_out_of_bounds_is_rejected(self):
        bad_segment = (1, 5, (1 << 64) - 2, 0x400000, 0x400000, 8, 8, 0x1000)
        path = self.write_artifact(
            "bad-segment.elf", make_elf(program_headers=[bad_segment])
        )
        self.assert_code("segment_out_of_bounds", self.request_json(path, digest="0" * 64))

    def test_pt_interp_is_rejected(self):
        interp = (3, 4, 180, 0, 0, 18, 18, 1)
        data = bytearray(
            make_elf(
                program_headers=[
                    (1, 5, 176, 0x400000, 0x400000, 4, 4, 0x1000),
                    interp,
                ]
            )
        )
        data[180:198] = b"/lib/ld-musl.so.1\0"
        path = self.write_artifact("dynamic.elf", bytes(data))
        self.assert_code("dynamic_interpreter_present", self.request_json(path, digest="0" * 64))


if __name__ == "__main__":
    unittest.main()
