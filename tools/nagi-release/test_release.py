import hashlib
import json
import struct
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import release


class ReleaseToolTests(unittest.TestCase):
    def test_tracked_third_party_license_texts_are_selected_stably(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            subprocess.run(["git", "init", "--quiet"], cwd=root, check=True)
            tracked_paths = (
                "third_party/zeta/NOTICE.md",
                "third_party/alpha/LICENSE-MIT",
                "third_party/alpha/COPYING",
                "third_party/alpha/README.md",
                "LICENSE",
            )
            for relative in tracked_paths:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture text\n", encoding="utf-8")
            subprocess.run(["git", "add", "--all"], cwd=root, check=True)

            inputs = release.tracked_license_inputs(root)

            self.assertEqual(
                [
                    (source.relative_to(root.resolve()).as_posix(), package)
                    for source, package in inputs
                ],
                [
                    (
                        "third_party/alpha/COPYING",
                        "licenses/source-tree/third_party/alpha/COPYING",
                    ),
                    (
                        "third_party/alpha/LICENSE-MIT",
                        "licenses/source-tree/third_party/alpha/LICENSE-MIT",
                    ),
                    (
                        "third_party/zeta/NOTICE.md",
                        "licenses/source-tree/third_party/zeta/NOTICE.md",
                    ),
                ],
            )

    def test_tracked_license_inventory_checks_files_and_hashes(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            package = "licenses/source-tree/third_party/example/LICENSE-MIT"
            license_file = directory / package
            license_file.parent.mkdir(parents=True)
            license_file.write_bytes(b"MIT license fixture\n")
            build_manifest = {
                "tracked_license_texts": [
                    {
                        "source_path": "third_party/example/LICENSE-MIT",
                        "package_path": package,
                        "sha256": hashlib.sha256(license_file.read_bytes()).hexdigest(),
                    }
                ]
            }

            release.verify_tracked_license_inventory(directory, build_manifest)

            license_file.write_bytes(b"tampered license fixture\n")
            with self.assertRaisesRegex(release.ReleaseError, "SHA-256 mismatch"):
                release.verify_tracked_license_inventory(directory, build_manifest)

    def test_legacy_tracked_license_inventory_is_optional_without_payloads(self):
        with tempfile.TemporaryDirectory() as temporary:
            release.verify_tracked_license_inventory(Path(temporary), {})

    def test_tracked_license_inventory_rejects_symlink_directories(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            external = directory / "external"
            external.mkdir()
            license_file = external / "LICENSE-MIT"
            license_file.write_bytes(b"MIT license fixture\n")
            package_dir = directory / "licenses/source-tree/third_party/example"
            package_dir.parent.mkdir(parents=True)
            package_dir.symlink_to(external, target_is_directory=True)
            package = "licenses/source-tree/third_party/example/LICENSE-MIT"
            manifest = {
                "tracked_license_texts": [
                    {
                        "source_path": "third_party/example/LICENSE-MIT",
                        "package_path": package,
                        "sha256": hashlib.sha256(license_file.read_bytes()).hexdigest(),
                    }
                ]
            }

            with self.assertRaisesRegex(release.ReleaseError, "symlink"):
                release.verify_tracked_license_inventory(directory, manifest)

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

    def test_image_build_provenance_binds_source_revision_and_image_digest(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            image = root / "out/artifacts/reference.qcow2"
            image.parent.mkdir(parents=True)
            image.write_bytes(b"built release image")
            revision = "a" * 40
            digest = hashlib.sha256(image.read_bytes()).hexdigest()
            info_path = image.with_name(image.name + release.IMAGE_BUILD_INFO_SUFFIX)
            info_path.write_text(
                f"format_version=1\nsource_revision={revision}\nimage_sha256={digest}\n",
                encoding="ascii",
            )

            provenance = release.image_build_provenance(root, image, revision)

            self.assertEqual(provenance["source_revision"], revision)
            self.assertEqual(provenance["image_sha256"], digest)
            image.write_bytes(b"changed image")
            with self.assertRaisesRegex(release.ReleaseError, "disagrees with its build provenance"):
                release.image_build_provenance(root, image, revision)

    def test_image_build_provenance_rejects_missing_and_stale_records(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            image = root / "out/artifacts/reference.qcow2"
            image.parent.mkdir(parents=True)
            image.write_bytes(b"built release image")
            revision = "a" * 40
            digest = hashlib.sha256(image.read_bytes()).hexdigest()
            info_path = image.with_name(image.name + release.IMAGE_BUILD_INFO_SUFFIX)

            with self.assertRaisesRegex(release.ReleaseError, "missing release image build provenance"):
                release.image_build_provenance(root, image, revision)

            info_path.write_text(
                f"format_version=1\nsource_revision={'b' * 40}\nimage_sha256={digest}\n",
                encoding="ascii",
            )
            with self.assertRaisesRegex(release.ReleaseError, "different source revision"):
                release.image_build_provenance(root, image, revision)

    def test_checksum_verification_detects_modified_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            (directory / "payload.bin").write_bytes(b"actual build output")
            (directory / release.SUMS_NAME).write_bytes(release.checksum_lines(directory))
            release.verify_checksum_index(directory)

            (directory / "payload.bin").write_bytes(b"changed after hashing")
            with self.assertRaisesRegex(release.ReleaseError, "SHA-256 mismatch"):
                release.verify_checksum_index(directory)

    def test_checksum_verification_rejects_untracked_symlink_directory(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "bundle"
            directory.mkdir()
            payload = directory / "payload.bin"
            payload.write_bytes(b"actual build output")
            (directory / release.SUMS_NAME).write_bytes(release.checksum_lines(directory))

            external = Path(temporary) / "external"
            external.mkdir()
            external_file = external / "outside.bin"
            external_file.write_bytes(b"outside the release bundle")
            (directory / "untracked-assets").symlink_to(external, target_is_directory=True)

            with patch.object(release, "sha256_file", wraps=release.sha256_file) as hash_file:
                with self.assertRaisesRegex(release.ReleaseError, "symlink"):
                    release.verify_checksum_index(directory)

            hashed_paths = [Path(call.args[0]) for call in hash_file.call_args_list]
            self.assertNotIn(external_file, hashed_paths)

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

    def test_release_verification_rejects_symlink_before_reading_bundle_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            directory = root / "bundle"
            directory.mkdir()
            external = root / "external"
            external.mkdir()
            (external / "outside.bin").write_bytes(b"outside the release bundle")
            (directory / "untracked-assets").symlink_to(external, target_is_directory=True)

            with self.assertRaisesRegex(release.ReleaseError, "symlink"):
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
