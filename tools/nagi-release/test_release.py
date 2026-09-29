import hashlib
import json
import struct
import tempfile
import unittest
from pathlib import Path

import release


class ReleaseToolTests(unittest.TestCase):
    def test_missing_required_documents_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaisesRegex(release.ReleaseError, "RELEASE_NOTES.md") as error:
                release.required_document_inputs(root)
            self.assertIn("CONTRIBUTING.md", str(error.exception))
            self.assertIn("ROADMAP.md", str(error.exception))
            self.assertIn("docs/architecture/language-architecture.md", str(error.exception))
            self.assertIn("sdk/README.md", str(error.exception))

    def test_missing_qcow2_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaisesRegex(release.ReleaseError, "missing release qcow2 image"):
                release.validate_qcow2(Path(temporary) / "missing.qcow2", Path(temporary), 64)

    def test_checksum_verification_detects_modified_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "payload.bin").write_bytes(b"actual build output")
            (directory / release.SUMS_NAME).write_bytes(release.checksum_lines(directory))
            release.verify_checksum_index(directory)

            (directory / "payload.bin").write_bytes(b"changed after hashing")
            with self.assertRaisesRegex(release.ReleaseError, "SHA-256 mismatch"):
                release.verify_checksum_index(directory)

    def test_kernel_provenance_uses_real_elf_bytes_and_rejects_fixture_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            kernel = root / "target/release/nagi-kernel"
            kernel.parent.mkdir(parents=True)
            kernel.write_bytes(self._minimal_x86_64_elf())
            expected = "sha256:" + hashlib.sha256(kernel.read_bytes()).hexdigest()
            self.assertEqual(release.kernel_build_id(root, kernel), expected)

            fixture = root / "target/tests/fixtures/kernel.elf"
            fixture.parent.mkdir(parents=True)
            fixture.write_bytes(self._minimal_x86_64_elf())
            with self.assertRaisesRegex(release.ReleaseError, "fixture"):
                release.kernel_build_id(root, fixture)

    def test_source_revision_provenance_requires_full_pinned_revision(self):
        locked = {"sources": {"servo": {"revision": "a" * 40}}}
        self.assertEqual(release._pinned_revision(locked, "servo"), "a" * 40)
        with self.assertRaisesRegex(release.ReleaseError, "full pinned revision"):
            release._pinned_revision(
                {"sources": {"servo": {"revision": "moving-branch"}}}, "servo"
            )

    def test_test_fixture_manifest_never_claims_release_readiness(self):
        with tempfile.TemporaryDirectory() as temporary:
            stage = Path(temporary)
            payload = stage / release.IMAGE_NAME
            payload.write_bytes(b"test fixture only")
            manifest = release.release_manifest_for(
                stage, [(Path(release.IMAGE_NAME), "test fixture")]
            )
            self.assertEqual(manifest["assembly_status"], "ASSEMBLED")
            self.assertEqual(manifest["m30_acceptance"], "NOT_EVALUATED")
            self.assertNotIn("release_ready", manifest)
            self.assertNotIn("release-ready", json.dumps(manifest))

    def test_verify_rejects_fixture_that_claims_guest_acceptance(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / release.IMAGE_NAME).write_bytes(b"tiny qcow fixture")
            for _, package_name in release.DOC_INPUTS:
                document = directory / package_name
                document.parent.mkdir(parents=True, exist_ok=True)
                document.write_text("test fixture\n", encoding="utf-8")
            (directory / release.BUILD_MANIFEST_NAME).write_text("{}\n", encoding="utf-8")
            (directory / release.SOURCE_REVISION_NAME).write_text("0" * 40 + "\n", encoding="ascii")
            (directory / release.MANIFEST_NAME).write_text(
                json.dumps(
                    {
                        "schema_version": release.SCHEMA_VERSION,
                        "assembly_status": "ASSEMBLED",
                        "m30_acceptance": "PASS",
                        "release_ready": True,
                    }
                ),
                encoding="utf-8",
            )
            (directory / release.SUMS_NAME).write_bytes(release.checksum_lines(directory))
            with self.assertRaisesRegex(release.ReleaseError, "cannot establish M30 guest acceptance"):
                release.verify_release(directory)

    def test_manifest_and_checksums_are_reproducible(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            outputs = []
            for index, order in enumerate((("b.txt", "a.txt"), ("a.txt", "b.txt"))):
                stage = base / str(index)
                stage.mkdir()
                for name in order:
                    (stage / name).write_bytes(name.encode("ascii"))
                copied = [(Path(name), f"source/{name}") for name in ("a.txt", "b.txt")]
                release._write_json(stage / release.MANIFEST_NAME, release.release_manifest_for(stage, copied))
                (stage / release.SUMS_NAME).write_bytes(release.checksum_lines(stage))
                outputs.append(
                    (
                        (stage / release.MANIFEST_NAME).read_bytes(),
                        (stage / release.SUMS_NAME).read_bytes(),
                    )
                )
            self.assertEqual(outputs[0], outputs[1])

    @staticmethod
    def _minimal_x86_64_elf():
        header = bytearray(64)
        header[:4] = b"\x7fELF"
        header[4] = 2  # ELF64
        header[5] = 1  # little-endian
        struct.pack_into("<H", header, 16, 2)  # ET_EXEC
        struct.pack_into("<H", header, 18, 62)  # EM_X86_64
        return bytes(header)


if __name__ == "__main__":
    unittest.main()
