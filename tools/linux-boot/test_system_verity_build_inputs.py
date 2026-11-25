import hashlib
import tempfile
import unittest
from pathlib import Path

from system_verity_build_inputs import BuildInputError, validate_resume_manifest, write_resume_manifest


class SystemVerityBuildInputTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        self.paths = {}
        for name, content in (
            ("base_config", b"base config"),
            ("final_config", b"final config"),
            ("merge_fragment", b"DM_VERITY=y\n"),
            ("builder_script", b"build script"),
            ("source_verifier", b"source verifier"),
            ("build_inputs_helper", b"build input helper"),
            ("config_validator", b"config validator"),
            ("make", b"GNU Make"),
        ):
            path = self.root / name
            path.write_bytes(content)
            self.paths[name] = path
        self.source_dir = self.root / "linux-6.18.63"
        self.source_dir.mkdir()
        (self.source_dir / "Makefile").write_bytes(b"VERSION = 6\n")
        self.archive = self.root / "linux-6.18.63.tar.xz"
        self.archive.write_bytes(b"pinned archive bytes")
        self.host_support = self.root / "host-support-before.json"
        self.host_support.write_bytes(b"original host support")
        self.source_identity = {
            "archive_sha256": hashlib.sha256(self.archive.read_bytes()).hexdigest(),
            "tree_sha256": "a" * 64,
        }
        self.manifest = self.root / "resume-inputs.json"

    def tearDown(self):
        self.temp.cleanup()

    def test_resume_manifest_accepts_identical_build_inputs(self):
        write_resume_manifest(
            self.manifest, self.paths, self.source_dir, self.archive,
            self.source_identity, self.host_support,
        )
        digest = validate_resume_manifest(
            self.manifest, self.paths, self.source_dir, self.archive,
            self.source_identity, self.host_support,
        )
        self.assertEqual(len(digest), 64)

    def test_resume_manifest_rejects_modified_final_config(self):
        write_resume_manifest(
            self.manifest, self.paths, self.source_dir, self.archive,
            self.source_identity, self.host_support,
        )
        self.paths["final_config"].write_bytes(b"changed config")
        with self.assertRaisesRegex(BuildInputError, "final_config"):
            validate_resume_manifest(
                self.manifest, self.paths, self.source_dir, self.archive,
                self.source_identity, self.host_support,
            )

    def test_resume_manifest_rejects_modified_original_host_support(self):
        write_resume_manifest(
            self.manifest, self.paths, self.source_dir, self.archive,
            self.source_identity, self.host_support,
        )
        self.host_support.write_bytes(b"changed host support")
        with self.assertRaisesRegex(BuildInputError, "host-support"):
            validate_resume_manifest(
                self.manifest, self.paths, self.source_dir, self.archive,
                self.source_identity, self.host_support,
            )

    def test_resume_manifest_is_required(self):
        with self.assertRaisesRegex(BuildInputError, "manifest is missing"):
            validate_resume_manifest(
                self.manifest, self.paths, self.source_dir, self.archive,
                self.source_identity, self.host_support,
            )


if __name__ == "__main__":
    unittest.main()
